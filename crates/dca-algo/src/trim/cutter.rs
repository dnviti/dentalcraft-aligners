//! Heights of the trim surface on a plan grid: interpolated between control
//! points (`aligner.trim_cutter`) or the EQUATOR field
//! (`case_print._equator_field`). The aligner keeps what is above them.

use std::cmp::Ordering;

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use rayon::prelude::*;

use super::grid::{self, PlanGrid, np_max, np_min};
use super::line::{EQUATOR_GUM_GAP_MM, ToothLoop};
use super::survey::{bridge_contacts, loop_nearest};

/// The aligner reaches at most trim extension (EQUATOR: wall) + this (mm)
/// from the crowns.
pub const NEAR_TEETH_EXTRA: f64 = 1.5;
/// EQUATOR: the equator of a tooth holds within wall + this (mm, plan) of
/// its crown.
pub const EQUATOR_REACH_EXTRA: f64 = 0.5;
/// EQUATOR: grid of the trim surface (mm) ...
pub const EQUATOR_CELL_MM: f64 = 0.3;
/// ... at most this x the wall ...
pub const EQUATOR_CELL_WALL: f64 = 0.3;
/// ... and at least this.
pub const EQUATOR_CELL_MIN_MM: f64 = 0.1;
/// EQUATOR: the lower limits of the trim are smoothed over this many cells
/// ...
pub const EQUATOR_SMOOTH_CELLS: usize = 2;
/// ... raising a node of the wall held at an equator at most this above it.
pub const EQUATOR_SMOOTH_RISE_MM: f64 = 0.2;
/// EQUATOR: beyond the reach of the aligner the trim rises this much per mm.
pub const EQUATOR_RISE: f64 = 10.0;
/// EQUATOR: a crown column under the gingiva this near (plan) is no crown
/// column.
pub const EQUATOR_GUM_NEAR_MM: f64 = 0.3;
/// EQUATOR: the gap over a sloping gingiva is taken along its normal up to
/// 60 degrees of slope.
pub const EQUATOR_GUM_COS_MIN: f64 = 0.5;
/// EQUATOR: the trim is smoothed at last by this many passes of a 3x3
/// filter ...
pub const EQUATOR_RIM_SMOOTH_PASSES: usize = 8;
/// ... within its limits, raised by at most this (mm) ...
pub const EQUATOR_RIM_RAISE_MM: f64 = 0.1;
/// ... or, inside an outline within this (mm), up to the border beside it
/// ...
pub const EQUATOR_RIM_BAND_MM: f64 = 0.6;
/// ... or beside a node held higher, up to a slope of this (mm per mm).
pub const EQUATOR_RIM_SLOPE: f64 = 2.0;
/// EQUATOR: a border moved towards occlusal holds this far inside the
/// outline too.
pub const EQUATOR_NEG_BAND_MM: f64 = 1.0;
/// EQUATOR: beside two crowns the equator of both holds where they are this
/// close to as near ...
pub const EQUATOR_SHARE_MM: f64 = 0.3;
/// ... and farther, lower by this per mm (plan) beyond it.
pub const EQUATOR_SHARE_SLOPE: f64 = 8.0;
/// EQUATOR: an equator point within this (plan) of a neighbour's equator is
/// at the contact.
pub const EQUATOR_CONTACT_MM: f64 = 1.0;
/// EQUATOR: over a crown with no undercut under it the trim stays this below
/// its top.
pub const CROWN_CAP_MM: f64 = 0.5;
/// EQUATOR: empty space under a crown thicker than this is an undercut.
pub const CROWN_GAP_MM: f64 = 0.05;
/// EQUATOR: at the foot of a crown the trim may rise to this below its
/// surface.
pub const CROWN_SKIN_MM: f64 = 0.05;
/// EQUATOR: a node of the trim this far below most of its neighbours is a
/// spike.
pub const CROWN_SPIKE_MM: f64 = 0.2;
/// EQUATOR: a node within this of two outlines is at a contact.
pub const CROWN_CAP_CLEAR_MM: f64 = 1.0;

/// Heights interpolated between the control points `ctrl` on the nodes
/// (`trim_cutter` without a field): inverse squared distance of the `k`
/// nearest in plan; farther than `reach` mm from every control point the
/// surface rises by `slope` per mm; at most `z_top - 1`.
pub fn interpolated(
    grid: &PlanGrid,
    ctrl: &[DVec3],
    k: usize,
    reach: f64,
    slope: f64,
    z_top: f64,
) -> Vec<f64> {
    let kk = k.min(ctrl.len()).max(1);
    (0..grid.len())
        .into_par_iter()
        .map(|n| {
            if ctrl.is_empty() {
                return z_top - 1.0;
            }
            let p = grid.node(n);
            let mut d: Vec<(f64, usize)> = ctrl
                .iter()
                .enumerate()
                .map(|(i, c)| ((p - c.truncate()).length(), i))
                .collect();
            d.select_nth_unstable_by(kk - 1, |a, b| a.0.total_cmp(&b.0));
            let near = &d[..kk];
            let (mut ws, mut zs) = (0.0, 0.0);
            let mut dmin = f64::INFINITY;
            for &(di, i) in near {
                let w = 1.0 / (di * di + 0.25);
                ws += w;
                zs += w * ctrl[i].z;
                dmin = dmin.min(di);
            }
            let z = zs / ws + slope * (dmin - reach).max(0.0);
            z.min(z_top - 1.0)
        })
        .collect()
}

/// Cell of the grid of the EQUATOR trim surface for the wall (`_trim_cell`).
pub fn equator_cell(wall: f64) -> f64 {
    let w = (EQUATOR_CELL_WALL * wall * 1e6).round() / 1e6;
    EQUATOR_CELL_MM.min(w).max(EQUATOR_CELL_MIN_MM)
}

/// What the EQUATOR field needs of the model of the stage.
pub struct EquatorModel<'a> {
    /// Closed teeth (the rays find the crown tops and their undercuts).
    pub teeth: &'a TriMesh,
    /// Gingiva of the model (the support under the crowns, the floor of the
    /// wall beside them).
    pub gingiva: &'a TriMesh,
    /// Height above the model where the rays start (the top of the model +
    /// 5 mm).
    pub z0: f64,
}

/// The design values of the EQUATOR field.
#[derive(Debug, Clone, Copy)]
pub struct EquatorDesign {
    /// Outer offset of the shell (clearance + wall).
    pub outer: f64,
    /// Clearance.
    pub clearance: f64,
    /// Offset of the border beyond the equator.
    pub offset: f64,
}

/// One downward ray of a column.
struct Column {
    /// Gingiva hit (with the slope correction), `-inf` when missed.
    g_top: f64,
    /// Crown top, where it leaves the crown, the next surface below.
    teeth: [f64; 3],
}

fn columns(
    m: &EquatorModel<'_>,
    tb: Option<&Bvh>,
    gb: Option<&Bvh>,
    gnorm: &[DVec3],
    nodes: &[DVec2],
    want: &[bool],
    inside: &[bool],
) -> Vec<Column> {
    let down = Vec3::NEG_Z;
    let max_t = f32::INFINITY;
    nodes
        .par_iter()
        .enumerate()
        .map(|(k, p)| {
            let mut c = Column {
                g_top: f64::NEG_INFINITY,
                teeth: [f64::NAN; 3],
            };
            if !want[k] {
                return c;
            }
            let o = DVec3::new(p.x, p.y, m.z0).as_vec3();
            if let Some(g) = gb
                && let Some(h) = g.ray_cast(o, down, max_t)
            {
                let nz = gnorm[h.face as usize].z.abs().max(EQUATOR_GUM_COS_MIN);
                c.g_top = m.z0 - f64::from(h.t) + EQUATOR_GUM_GAP_MM * (1.0 / nz - 1.0);
            }
            if inside[k]
                && let Some(t) = tb
            {
                for (i, h) in t.ray_hits(o, down, max_t, 3, 1e-4).iter().enumerate() {
                    c.teeth[i] = m.z0 - f64::from(h.t);
                }
            }
            c
        })
        .collect()
}

fn face_normals(mesh: &TriMesh) -> Vec<DVec3> {
    mesh.tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3());
            (b - a).cross(c - a).normalize_or_zero()
        })
        .collect()
}

/// EQUATOR: the wall beside a crown is measured from the crown the scan
/// shows less this many cells (`EQUATOR_VISIBLE_SLACK_CELLS`: the nearest
/// crown node is up to about 1.4 cells beyond the outline).
pub const EQUATOR_VISIBLE_SLACK_CELLS: f64 = 1.5;
/// EQUATOR: a grid finer than [`EQUATOR_CELL_MM`] is followed within this
/// inside an outline, blending over as much again into the field sampled
/// every [`EQUATOR_CELL_MM`] deeper (`EQUATOR_FINE_BAND_MM`).
pub const EQUATOR_FINE_BAND_MM: f64 = 0.6;

/// `g_top` (the gingiva hit by the ray of every node) raised by the most a
/// vertex of the gingiva in a cell of the node stands above the bilinear
/// interpolation of `top` there (the top of the model at every node, its
/// crown or its gingiva; default `g_top`), over `top` at the node
/// (`_gum_cell_max`): the rays sample the gingiva at the nodes only, and a
/// ridge between them (the rim of the gingiva over the distal of a last
/// molar) would stand above the trim surface between the nodes. A cell with
/// a node without gingiva raises nothing.
pub fn gum_cell_max(
    gum: &TriMesh,
    grid: &PlanGrid,
    g_top: &[f64],
    top: Option<&[f64]>,
) -> Vec<f64> {
    let out = g_top.to_vec();
    let base: Vec<f64> = match top {
        None => out.clone(),
        Some(t) => out
            .iter()
            .zip(t)
            .map(|(&g, &t)| if g.is_finite() { t } else { g })
            .collect(),
    };
    let (nx, ny) = (grid.nx, grid.ny);
    if gum.tris.is_empty() || nx < 2 || ny < 2 {
        return out;
    }
    let mut used: Vec<u32> = gum.tris.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let o = grid.node(0);
    let (hx, hy) = (grid.node(ny).x - o.x, grid.node(1).y - o.y);
    let mut lift = vec![0.0f64; nx * ny];
    for &v in &used {
        let q = gum.positions[v as usize].as_dvec3();
        let (fx, fy) = ((q.x - o.x) / hx, (q.y - o.y) / hy);
        let (fi, fj) = (fx.floor(), fy.floor());
        if !((0.0..(nx - 1) as f64).contains(&fi) && (0.0..(ny - 1) as f64).contains(&fj)) {
            continue;
        }
        let (i, j) = (fi as usize, fj as usize);
        let (u, w) = (fx - fi, fy - fj);
        let c = [
            base[i * ny + j],
            base[(i + 1) * ny + j],
            base[i * ny + j + 1],
            base[(i + 1) * ny + j + 1],
        ];
        if !c.iter().all(|x| x.is_finite()) {
            continue;
        }
        let e = q.z
            - (c[0] * (1.0 - u) * (1.0 - w)
                + c[1] * u * (1.0 - w)
                + c[2] * (1.0 - u) * w
                + c[3] * u * w);
        if e > 0.0 {
            for (di, dj) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let k = (i + di) * ny + j + dj;
                lift[k] = lift[k].max(e);
            }
        }
    }
    out.iter()
        .zip(&base)
        .zip(&lift)
        .map(|((&g, &b), &l)| {
            if g.is_finite() && l > 0.0 {
                g.max(b + l)
            } else {
                g
            }
        })
        .collect()
}

/// Plan distance from the nodes `want` of a grid (nodes `p`, `nx` x `ny`) to
/// the nearest node of `mask` (`_grid_distance`): to the nodes of the mask on
/// its edge (a node with a neighbour off the mask or off the grid), 0 on the
/// mask, in single precision as the plugin's k-d tree; infinite with no
/// node in the mask.
pub fn grid_distance(p: &[DVec2], mask: &[bool], want: &[usize], nx: usize, ny: usize) -> Vec<f64> {
    let at = |i: isize, j: isize| {
        i >= 0
            && j >= 0
            && (i as usize) < nx
            && (j as usize) < ny
            && mask[i as usize * ny + j as usize]
    };
    let edge: Vec<[f32; 2]> = (0..nx * ny)
        .filter(|&k| {
            let (i, j) = ((k / ny) as isize, (k % ny) as isize);
            mask[k] && !(at(i - 1, j) && at(i + 1, j) && at(i, j - 1) && at(i, j + 1))
        })
        .map(|k| [p[k].x as f32, p[k].y as f32])
        .collect();
    if edge.is_empty() {
        return vec![f64::INFINITY; want.len()];
    }
    want.par_iter()
        .map(|&k| {
            if mask[k] {
                return 0.0;
            }
            let (x, y) = (p[k].x as f32, p[k].y as f32);
            let d2 = edge
                .iter()
                .map(|e| {
                    let (dx, dy, dz) = (e[0] - x, e[1] - y, 0.0f32);
                    dx * dx + dy * dy + dz * dz
                })
                .fold(f32::INFINITY, f32::min);
            f64::from(d2.sqrt())
        })
        .collect()
}

/// The EQUATOR trim heights `z` of a grid finer than [`EQUATOR_CELL_MM`]
/// followed only near the border (`_coarse_inside`): deeper inside an
/// outline (`loops`) than [`EQUATOR_FINE_BAND_MM`] they blend, over as much
/// again, into the heights `field` gives on the grid of [`EQUATOR_CELL_MM`]
/// over the same bounds (the triangles of its cells, diagonal from node
/// `(i, j)` to `(i + 1, j + 1)`). The rim follows the fine cells, the cap
/// over the crown does not: a finer grid there finds thin overhangs inside
/// the outline whose columns are undercut, and the wall under them a
/// window.
pub fn coarse_inside(
    grid: &PlanGrid,
    z: &mut [f64],
    loops: &[&[DVec3]],
    field: impl FnOnce(&PlanGrid) -> Vec<f64>,
) {
    let (nx, ny) = (grid.nx, grid.ny);
    if nx < 2 || ny < 2 || loops.is_empty() {
        return;
    }
    let (lo, hi) = (grid.lo, grid.hi);
    let coarse = PlanGrid {
        lo,
        hi,
        nx: ((hi.x - lo.x) / EQUATOR_CELL_MM).ceil() as usize + 1,
        ny: ((hi.y - lo.y) / EQUATOR_CELL_MM).ceil() as usize + 1,
    };
    let p = grid.nodes();
    let mut depth = vec![0.0f64; p.len()];
    for e in loops {
        let blo = e
            .iter()
            .fold(DVec2::splat(f64::INFINITY), |a, q| a.min(q.truncate()));
        let bhi = e
            .iter()
            .fold(DVec2::splat(f64::NEG_INFINITY), |a, q| a.max(q.truncate()));
        for (k, q) in p.iter().enumerate() {
            if q.cmpge(blo).all() && q.cmple(bhi).all() {
                depth[k] = np_max(depth[k], loop_nearest(e, *q).0);
            }
        }
    }
    let w: Vec<f64> = depth
        .iter()
        .map(|d| ((d - EQUATOR_FINE_BAND_MM) / EQUATOR_FINE_BAND_MM).clamp(0.0, 1.0))
        .collect();
    if !w.iter().any(|&x| x > 0.0) {
        return;
    }
    let zc = field(&coarse);
    let (mx, my) = (coarse.nx, coarse.ny);
    let (hx, hy) = (
        (hi.x - lo.x) / (mx - 1) as f64,
        (hi.y - lo.y) / (my - 1) as f64,
    );
    for (k, q) in p.iter().enumerate() {
        if w[k].partial_cmp(&0.0) != Some(Ordering::Greater) {
            continue;
        }
        let (fx, fy) = ((q.x - lo.x) / hx, (q.y - lo.y) / hy);
        let i = (fx.floor().max(0.0) as usize).min(mx - 2);
        let j = (fy.floor().max(0.0) as usize).min(my - 2);
        let (u, v) = (fx - i as f64, fy - j as f64);
        let (za, zb) = (zc[i * my + j], zc[(i + 1) * my + j]);
        let (zcc, zd) = (zc[(i + 1) * my + j + 1], zc[i * my + j + 1]);
        let zi = if u >= v {
            za + u * (zb - za) + v * (zcc - zb)
        } else {
            za + v * (zd - za) + u * (zcc - zd)
        };
        if zi.is_finite() && z[k].is_finite() {
            z[k] += w[k] * (zi - z[k]);
        }
    }
}

/// Heights of the EQUATOR trim surface on the nodes (`_equator_field`).
///
/// Inside the outline of a crown (its equator loop) the column of the model
/// decides (ray cast along the path): the trim stays [`CROWN_CAP_MM`] below
/// the top of the crown, except where the crown overhangs an empty space
/// (an undercut), where it stays at the underside of the overhang (lowered
/// by a positive offset). Beside a crown, within `outer` +
/// [`EQUATOR_REACH_EXTRA`] of its loop, the wall is kept above the equator
/// of that crown (and of a neighbour about as near) and above the gingiva:
/// its underside rises from [`EQUATOR_GUM_GAP_MM`] over the gingiva at the
/// crown to the whole wall farther out, measured from the crown the scan
/// shows ([`grid_distance`]); over a node inside an outline that is no crown
/// column the gingiva is raised by its ridges between the nodes
/// ([`gum_cell_max`]). Beyond the reach of the aligner the surface rises
/// steeply. The lower limits are smoothed without going below them, spikes
/// are removed and the steps between the nodes smoothed within the limits.
/// Over a tooth left out nothing is kept.
///
/// Not ported (no input for them yet): the bars over a space between two
/// caps and the ceilings of the embrasure fills.
pub fn equator_field(
    grid: &PlanGrid,
    loops: &[&ToothLoop],
    cut: &[&ToothLoop],
    model: &EquatorModel<'_>,
    d: EquatorDesign,
) -> Vec<f64> {
    let raw: Vec<Vec<DVec3>> = loops
        .iter()
        .filter(|l| l.points.len() >= 3)
        .map(|l| l.points.clone())
        .collect();
    let loops = bridge_contacts(&raw, EQUATOR_CONTACT_MM);
    let (outer, cl, off) = (d.outer, d.clearance, d.offset);
    let near = outer + EQUATOR_REACH_EXTRA;
    let reach = outer + NEAR_TEETH_EXTRA;
    let z0 = model.z0;
    let p = grid.nodes();
    let n = p.len();
    let (nx, ny) = (grid.nx, grid.ny);

    // the loops near every node
    struct NodeLoops {
        inside: bool,
        n_close: usize,
        dmin: f64,
        z_out: f64,
        depth: f64,
        z_own: f64,
    }
    let boxes: Vec<(DVec2, DVec2)> = loops
        .iter()
        .map(|e| {
            let lo = e
                .iter()
                .fold(DVec2::splat(f64::INFINITY), |a, q| a.min(q.truncate()));
            let hi = e
                .iter()
                .fold(DVec2::splat(f64::NEG_INFINITY), |a, q| a.max(q.truncate()));
            (lo - DVec2::splat(reach), hi + DVec2::splat(reach))
        })
        .collect();
    let all: Vec<DVec3> = loops.iter().flatten().copied().collect();
    let nl: Vec<NodeLoops> = p
        .par_iter()
        .map(|&q| {
            let mut r = NodeLoops {
                inside: false,
                n_close: 0,
                dmin: f64::INFINITY,
                z_out: f64::NEG_INFINITY,
                depth: 0.0,
                z_own: f64::NAN,
            };
            let mut seen: Vec<(f64, f64)> = Vec::new();
            for (e, (lo, hi)) in loops.iter().zip(&boxes) {
                if !(q.cmpge(*lo).all() && q.cmple(*hi).all()) {
                    continue;
                }
                let (dep, zn) = loop_nearest(e, q);
                let ins = dep > 0.0;
                r.inside |= ins;
                r.n_close += usize::from(dep > -CROWN_CAP_CLEAR_MM);
                let dist = (-dep).max(0.0);
                r.dmin = r.dmin.min(dist);
                seen.push((dist, zn));
                if ins && dep > r.depth {
                    r.depth = dep;
                    r.z_own = zn;
                }
            }
            for &(dist, zn) in &seen {
                let zs = zn - EQUATOR_SHARE_SLOPE * (dist - r.dmin - EQUATOR_SHARE_MM).max(0.0);
                if dist <= near {
                    r.z_out = r.z_out.max(zs);
                }
            }
            // away from every tooth: the nearest equator point, and how far it is
            if !r.z_out.is_finite() && !r.inside && !all.is_empty() {
                let (mut bd, mut bz) = (f64::INFINITY, f64::NAN);
                for a in &all {
                    let d2 = (q - a.truncate()).length_squared();
                    if d2 < bd {
                        bd = d2;
                        bz = a.z;
                    }
                }
                r.z_out = bz;
                r.dmin = r.dmin.min(bd.sqrt());
            }
            r
        })
        .collect();

    // the column of every node inside an outline or beside a crown
    let inside: Vec<bool> = nl.iter().map(|r| r.inside).collect();
    let want: Vec<bool> = nl.iter().map(|r| r.inside || r.dmin <= reach).collect();
    let tb = (!model.teeth.tris.is_empty())
        .then(|| Bvh::build(model.teeth).ok())
        .flatten();
    let gb = (!model.gingiva.tris.is_empty())
        .then(|| Bvh::build(model.gingiva).ok())
        .flatten();
    let gnorm = face_normals(model.gingiva);
    let cols = columns(model, tb.as_ref(), gb.as_ref(), &gnorm, &p, &want, &inside);
    let mut l = vec![f64::NEG_INFINITY; n];
    let mut u = vec![f64::NEG_INFINITY; n];
    let mut hit = vec![false; n];
    let mut top_z = vec![f64::NEG_INFINITY; n];
    let g_top: Vec<f64> = cols.iter().map(|c| c.g_top).collect();
    for k in 0..n {
        if !inside[k] {
            continue;
        }
        let [top, under, below] = cols[k].teeth;
        if !(top.is_finite() && top > g_top[k] + EQUATOR_GUM_GAP_MM) {
            continue;
        }
        hit[k] = true;
        top_z[k] = top;
        u[k] = top - CROWN_CAP_MM;
        if under.is_finite() {
            // what supports the crown from below: the gingiva or another tooth; empty space = undercut
            let mut sup = f64::NEG_INFINITY;
            if g_top[k].is_finite() {
                sup = sup.max(g_top[k]);
            }
            if below.is_finite() {
                sup = sup.max(below);
            }
            if !sup.is_finite() || under - sup > CROWN_GAP_MM {
                l[k] = under - off.max(0.0);
            }
        }
    }
    // the spacing of the nodes as the plugin reads it off the grid (X[1, 0] - X[0, 0])
    let h = if nx > 1 {
        grid.node(ny).x - grid.node(0).x
    } else {
        EQUATOR_CELL_MM
    };
    let k_near = ((EQUATOR_GUM_NEAR_MM / h).round() as usize).max(1);
    let g_ray = grid::filter(&g_top, nx, ny, k_near, false);
    // the crown columns the scan shows (the foot of a crown too)
    let vis = hit.clone();
    for k in 0..n {
        let z_own = if nl[k].z_own.is_nan() {
            f64::INFINITY
        } else {
            nl[k].z_own
        };
        // not above the gingiva beside it (NaN counts as not above)
        let above =
            top_z[k].partial_cmp(&(g_ray[k] + EQUATOR_GUM_GAP_MM)) == Some(Ordering::Greater);
        let foot = hit[k] && !above && top_z[k] < z_own;
        if foot {
            hit[k] = false;
            u[k] = f64::NEG_INFINITY;
            l[k] = f64::NEG_INFINITY;
        }
    }
    // the gingiva of a node inside an outline and no crown column (the gingiva
    // covering the closed tooth, distal of a last molar): raised by the ridge
    // of the gingiva between it and the nodes beside it
    let tops: Vec<f64> = g_top.iter().zip(&top_z).map(|(g, t)| g.max(*t)).collect();
    let ridge = gum_cell_max(model.gingiva, grid, &g_top, Some(&tops));
    let g_cell: Vec<f64> = (0..n)
        .map(|k| {
            if inside[k] && !hit[k] {
                ridge[k]
            } else {
                g_top[k]
            }
        })
        .collect();
    let g_near = grid::filter(&g_cell, nx, ny, k_near, false);
    // the wall beside a crown is measured from the crown the scan shows: where
    // the gingiva covers the closed tooth inside its outline the nodes are
    // gingiva, as far from the crown as its visible edge
    let mut dwall: Vec<f64> = nl.iter().map(|r| r.dmin).collect();
    let want: Vec<usize> = (0..n).filter(|&k| !hit[k] && nl[k].dmin <= reach).collect();
    if vis.iter().any(|v| *v) && !want.is_empty() {
        let dvis = grid_distance(&p, &vis, &want, nx, ny);
        for (&k, dv) in want.iter().zip(dvis) {
            dwall[k] = np_max(dwall[k], dv - EQUATOR_VISIBLE_SLACK_CELLS * h);
        }
    }
    let floor: Vec<f64> = (0..n)
        .map(|k| {
            let g = if inside[k] { g_near[k] } else { g_cell[k] };
            g + cl + EQUATOR_GUM_GAP_MM + dwall[k].min(outer)
        })
        .collect();
    for k in 0..n {
        if !hit[k] {
            l[k] = np_max(nl[k].z_out, floor[k]) + EQUATOR_RISE * (dwall[k] - reach).max(0.0);
        }
    }
    if off < 0.0 {
        for k in 0..n {
            if hit[k] && nl[k].depth <= EQUATOR_NEG_BAND_MM && nl[k].n_close == 1 {
                l[k] = np_max(l[k], nl[k].z_own);
            }
        }
    }
    let mut z = grid::smooth_above(&l, nx, ny, EQUATOR_SMOOTH_CELLS);
    for k in 0..n {
        let lim = !hit[k] && nl[k].dmin <= outer && l[k].is_finite() && nl[k].z_out >= floor[k];
        if lim {
            z[k] = np_min(z[k], l[k] + EQUATOR_SMOOTH_RISE_MM);
        }
    }
    let zs = z.clone();
    let hard: Vec<f64> = (0..n)
        .map(|k| {
            if hit[k] {
                np_max(l[k], g_near[k] + cl + EQUATOR_GUM_GAP_MM)
            } else {
                f64::NEG_INFINITY
            }
        })
        .collect();
    for k in 0..n {
        if hit[k] {
            let capped = if z[k].is_finite() {
                np_min(z[k], u[k])
            } else {
                u[k]
            };
            z[k] = np_max(capped, hard[k]);
        }
    }
    // a lone node far below its neighbours along the border is raised to them
    let ring: Vec<bool> = (0..n)
        .map(|k| {
            let (i, j) = ((k / ny) as isize, (k % ny) as isize);
            let at = |a: isize, b: isize| {
                hit[a.clamp(0, nx as isize - 1) as usize * ny
                    + b.clamp(0, ny as isize - 1) as usize]
            };
            !hit[k] || !(at(i - 1, j) && at(i + 1, j) && at(i, j - 1) && at(i, j + 1))
        })
        .collect();
    let zd = grid::despike(&z, nx, ny, CROWN_SPIKE_MM, 6, &ring);
    for k in 0..n {
        let cap = if hit[k] { nl[k].z_own } else { f64::INFINITY };
        if zd[k] > z[k] {
            z[k] = np_max(z[k], np_min(zd[k], cap));
        }
    }
    // the steps between the nodes smoothed out within the limits
    if EQUATOR_RIM_SMOOTH_PASSES > 0 {
        let lo: Vec<f64> = (0..n)
            .map(|k| if hit[k] { hard[k] } else { l[k] })
            .collect();
        let mut hi: Vec<f64> = z.iter().map(|v| v + EQUATOR_RIM_RAISE_MM).collect();
        for k in 0..n {
            let t = (nl[k].depth / EQUATOR_RIM_BAND_MM).clamp(0.0, 1.0);
            if hit[k] && zs[k].is_finite() && zs[k] > z[k] {
                hi[k] = hi[k].max(zs[k] - t * (zs[k] - z[k]));
            }
        }
        if EQUATOR_RIM_SLOPE > 0.0 {
            let cells = (EQUATOR_RIM_BAND_MM / h).ceil() as usize;
            let cone = grid::cone_above(&lo, nx, ny, EQUATOR_RIM_SLOPE * h, cells);
            for k in 0..n {
                hi[k] = np_max(hi[k], cone[k]);
            }
        }
        for k in 0..n {
            if hit[k] {
                let own = if nl[k].z_own.is_nan() {
                    f64::NEG_INFINITY
                } else {
                    nl[k].z_own
                };
                let keep = np_max(top_z[k] - CROWN_SKIN_MM, own);
                hi[k] = np_min(hi[k], np_max(z[k], keep));
            }
        }
        z = grid::smooth_within(&z, &lo, &hi, nx, ny, EQUATOR_RIM_SMOOTH_PASSES);
    }
    // over a tooth the aligner leaves out: nothing is kept
    for e in cut.iter().filter(|e| e.points.len() >= 3) {
        let lo = e
            .points
            .iter()
            .fold(DVec2::splat(f64::INFINITY), |a, q| a.min(q.truncate()));
        let hi = e
            .points
            .iter()
            .fold(DVec2::splat(f64::NEG_INFINITY), |a, q| a.max(q.truncate()));
        for k in 0..n {
            if !inside[k]
                && p[k].cmpge(lo).all()
                && p[k].cmple(hi).all()
                && loop_nearest(&e.points, p[k]).0 > 0.0
            {
                z[k] = z0 + 10.0;
            }
        }
    }
    z
}
