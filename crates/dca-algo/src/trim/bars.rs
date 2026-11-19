//! Bars joining the caps of two consecutive teeth over the space between
//! them (`case_print._weak_pairs`, `_cap_pieces`, `_nearest_pair`, `_bar`,
//! `_grid_prism`; `aligner.section_area`, `aligner.strip_ceiling`).
//!
//! The trimmed shell is one piece. Two consecutive teeth the aligner covers
//! (arch order, a missing tooth skipped) whose caps are in different pieces of
//! the meshed shell, or joined through less than [`CONNECT_MIN_WIDTH_MM`] x the
//! wall of section, are a weak pair ([`weak_pairs`]): a neck that thin breaks
//! when the aligner is flexed and does not print reliably. A bar joins them
//! ([`bar`]): a straight box [`CONNECT_HALF_MM`] x 2 wide from
//! [`CONNECT_INTO_MM`] inside one tooth to as much inside the other, its top
//! [`CONNECT_DROP_MM`] under the equators of the two teeth where they face each
//! other (at most the wall less [`CONNECT_OVER_MM`]: the shell over it merges
//! with the walls beside the crowns) and at least [`EQUATOR_GUM_GAP_MM`] + the
//! clearance above the gingiva under it, down into the gingiva. Under a crown
//! that overhangs the bar (a tooth tipped over the space) the top is raised to
//! the underside of the crown, on a thin wall only. The box is added to the
//! model of the shell only (a blockout of the space; the printed model is
//! unchanged): the aligner lies on its top, the trim is lowered to it along
//! the bar ([`ceiling`]) and the reach of the aligner takes it in
//! ([`Bar::path`]).
//!
//! Deviation: the plugin bars the caps of the EQUATOR border only; with its
//! other borders a shell split over a space (an edentulous span wider than the
//! reach of the crowns) is left to its largest piece, the other teeth without
//! aligner. Here every border is barred the same way, its top under the
//! equators of the two teeth along the vertical path: the aligner covers the
//! whole arch in one piece.

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_geom::knn::KnnIndex;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use serde::Serialize;

use super::Tooth;
use super::cutter::CROWN_GAP_MM;
use super::line::{EQUATOR_GUM_GAP_MM, ToothLoop};
use super::survey::loop_nearest;
use super::warp::PathWarp;
use crate::anatomy::{self, Arch};

/// A piece with at least this share of the faces is a cap, not a fragment
/// (`CONNECT_MIN_SHARE`).
pub const CONNECT_MIN_SHARE: f64 = 0.01;
/// Two caps are joined through at least this x the wall thickness (mm2) ...
/// (`CONNECT_MIN_WIDTH_MM`) ...
pub const CONNECT_MIN_WIDTH_MM: f64 = 3.0;
/// ... of section within this (mm) across the plane between the teeth
/// (`CONNECT_SECTION_HALF_MM`).
pub const CONNECT_SECTION_HALF_MM: f64 = 5.0;
/// Rows of the section this far apart (mm; `section_area`'s step).
pub const SECTION_STEP_MM: f64 = 0.1;
/// Half width of a bar joining two caps (`CONNECT_HALF_MM`).
pub const CONNECT_HALF_MM: f64 = 1.75;
/// Spacing of the rays measuring the gingiva under a bar (`BAR_GUM_STEP_MM`).
pub const BAR_GUM_STEP_MM: f64 = 0.1;
/// The bar (its box) reaches this far (mm, plan) into each of the two teeth
/// (`CONNECT_INTO_MM`).
pub const CONNECT_INTO_MM: f64 = 1.0;
/// The top of the bar is this far under the equators of the two teeth
/// (`CONNECT_DROP_MM`) ...
pub const CONNECT_DROP_MM: f64 = 0.7;
/// ... at most the wall less this (`CONNECT_OVER_MM`).
pub const CONNECT_OVER_MM: f64 = 0.3;
/// Points across the top of a bar in its path (`np.linspace(-h, h, 5)`).
pub const PATH_COLUMNS: usize = 5;

/// Two consecutive teeth whose caps are not joined well enough.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Weak {
    /// The first tooth in arch order.
    pub a: Fdi,
    /// The second one.
    pub b: Fdi,
    /// Section of the join (mm2, rounded to 0.01): 0 for caps in different
    /// pieces.
    pub mm2: f64,
    /// The caps are in different pieces of the shell.
    pub split: bool,
}

/// How the caps of the covered teeth are joined on a meshed shell
/// ([`weak_pairs`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Joins {
    /// The weak pairs.
    pub weak: Vec<Weak>,
    /// The covered teeth whose cap is not on the largest piece of the shell
    /// (the one the trim keeps), in arch order.
    pub lost: Vec<Fdi>,
}

/// A bar joining the caps of two teeth over the space between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    /// The two teeth, in arch order.
    pub teeth: (Fdi, Fdi),
    /// Plan end of the bar inside the first tooth (the warped space of the
    /// insertion paths when there is one) ...
    pub p0: DVec2,
    /// ... and inside the second one.
    pub p1: DVec2,
    /// Height of the top at `p0` ...
    pub z0: f64,
    /// ... and at `p1` (warped space).
    pub z1: f64,
    /// The closed box under the top, down into the gingiva (arch
    /// coordinates, outward faces).
    pub solid: TriMesh,
    /// Points along the top, [`PATH_COLUMNS`] across every row (arch
    /// coordinates): the reach of the aligner takes them in.
    pub path: Vec<DVec3>,
}

impl Bar {
    /// The highest trim allowed at the plan point `p` (warped space): within
    /// [`CONNECT_HALF_MM`] of the segment `p0 p1` the height of the top there
    /// (`z0` at `p0` to `z1` at `p1`), `+inf` elsewhere (`strip_ceiling`).
    pub fn ceiling(&self, p: DVec2) -> f64 {
        let ab = self.p1 - self.p0;
        let t = ((p - self.p0).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        if (p - (self.p0 + t * ab)).length() <= CONNECT_HALF_MM {
            self.z0 + t * (self.z1 - self.z0)
        } else {
            f64::INFINITY
        }
    }

    /// The top of the bar as a strip of triangles through [`Bar::path`]
    /// (arch coordinates): the reach of the aligner as a surface.
    pub fn top_strip(&self) -> TriMesh {
        let w = PATH_COLUMNS as u32;
        let rows = (self.path.len() / PATH_COLUMNS) as u32;
        let mut tris = Vec::new();
        for i in 0..rows.saturating_sub(1) {
            for j in 0..w - 1 {
                let (a, b) = (i * w + j, (i + 1) * w + j);
                tris.push([a, b, b + 1]);
                tris.push([a, b + 1, a + 1]);
            }
        }
        TriMesh::new(self.path.iter().map(|p| p.as_vec3()).collect(), tris)
    }
}

/// The highest trim allowed at the plan point `p` by any of `bars`
/// ([`Bar::ceiling`]).
pub fn ceiling(bars: &[Bar], p: DVec2) -> f64 {
    bars.iter()
        .map(|b| b.ceiling(p))
        .fold(f64::INFINITY, f64::min)
}

/// The points that stand for a tooth: its closed tooth, or its scanned
/// crown when there is no closed one (`_stage_tooth_points`).
pub fn tooth_points(t: &Tooth) -> &[DVec3] {
    if t.closed.is_empty() {
        &t.crown
    } else {
        &t.closed
    }
}

/// Indices of the `n` highest of `p` (`np.argsort(p[:, 2])[-n:]`, stable).
fn highest(p: &[DVec3], n: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..p.len()).collect();
    idx.sort_by(|&a, &b| p[a].z.total_cmp(&p[b].z));
    idx.split_off(idx.len().saturating_sub(n))
}

/// The piece of `mesh` (index in [`TriMesh::connected_components`], the
/// largest first) that holds the cap of every tooth: the piece of most of the
/// nearest vertices to its 30 highest points (`_cap_pieces`). Pieces with
/// less than [`CONNECT_MIN_SHARE`] of the faces are fragments, not caps; no
/// entry for a tooth without points or when no piece is a cap.
pub fn cap_pieces(mesh: &TriMesh, teeth: &[(Fdi, &[DVec3])]) -> BTreeMap<Fdi, usize> {
    let comps = mesh.connected_components();
    let min = CONNECT_MIN_SHARE * mesh.tris.len() as f64;
    let mut vlab = vec![usize::MAX; mesh.positions.len()];
    for (r, faces) in comps.iter().enumerate() {
        if faces.len() as f64 >= min {
            for &f in faces {
                for v in mesh.tris[f as usize] {
                    vlab[v as usize] = r;
                }
            }
        }
    }
    let idx: Vec<usize> = (0..vlab.len())
        .filter(|&v| vlab[v] != usize::MAX)
        .step_by(3)
        .collect();
    let pts: Vec<Vec3> = idx.iter().map(|&v| mesh.positions[v]).collect();
    let Ok(index) = KnnIndex::build(&pts) else {
        return BTreeMap::new();
    };
    if index.is_empty() {
        return BTreeMap::new();
    }
    let mut out = BTreeMap::new();
    for &(f, p) in teeth {
        if p.is_empty() {
            continue;
        }
        let mut count: BTreeMap<usize, usize> = BTreeMap::new();
        for i in highest(p, 30) {
            if let Some(nb) = index.nearest(p[i].as_vec3()) {
                *count.entry(vlab[idx[nb.index as usize]]).or_default() += 1;
            }
        }
        // the most frequent piece (a tie: the larger piece)
        if let Some((&r, _)) = count.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))) {
            out.insert(f, r);
        }
    }
    out
}

/// Nearest points (plan) of the point sets `pa`, `pb` and the unit direction
/// from the first to the second, across the space (`_nearest_pair`): every
/// other point of `pa` against all of `pb`; touching (closer than 0.2 mm),
/// the direction between their centres.
pub fn nearest_pair(pa: &[DVec3], pb: &[DVec3]) -> (DVec2, DVec2, DVec2) {
    let fb: Vec<Vec3> = pb.iter().map(|p| p.as_vec3()).collect();
    let mut best = (f64::INFINITY, DVec2::ZERO, DVec2::ZERO);
    if let Ok(index) = KnnIndex::build(&fb) {
        for q in pa.iter().step_by(2) {
            if let Some(nb) = index.nearest(q.as_vec3()) {
                let d = (pb[nb.index as usize] - *q).length();
                if d < best.0 {
                    best = (d, q.truncate(), pb[nb.index as usize].truncate());
                }
            }
        }
    }
    let (_, a, b) = best;
    let mut u = b - a;
    if u.length() < 0.2 {
        let mean =
            |p: &[DVec3]| p.iter().map(|q| q.truncate()).sum::<DVec2>() / p.len().max(1) as f64;
        u = mean(pb) - mean(pa);
    }
    (a, b, u / u.length().max(1e-9))
}

/// Area (mm2) of the section of the closed surface of `bvh` (`normals`: the
/// outward normal of every face) by the vertical plane through the plan point
/// `m` with the horizontal normal `u`, within `half` mm of `m` across and
/// between `zlo` and `zhi`: rows of lines along the plane every `step`, each
/// split where it enters and leaves the solid; a line starting or ending
/// inside it counts from or up to the window (`aligner.section_area`).
#[allow(clippy::too_many_arguments)]
pub fn section_area(
    bvh: &Bvh,
    normals: &[DVec3],
    m: DVec2,
    u: DVec2,
    zlo: f64,
    zhi: f64,
    half: f64,
    step: f64,
) -> f64 {
    let ev = DVec2::new(-u.y, u.x) / u.length().max(1e-12);
    let dv = ev.extend(0.0);
    let mut area = 0.0;
    let rows = ((zhi - zlo) / step).ceil().max(0.0) as usize;
    for r in 0..rows {
        let z = zlo + r as f64 * step;
        let o = (m - half * ev).extend(z);
        let (mut t, mut inside, mut entered, mut first) = (0.0f64, false, 0.0f64, true);
        for _ in 0..200 {
            let from = o + dv * (t + 1e-5);
            let Some(h) = bvh.ray_cast(from.as_vec3(), dv.as_vec3(), (2.0 * half - t) as f32)
            else {
                break;
            };
            t += f64::from(h.t) + 1e-5;
            if normals[h.face as usize].dot(dv) < 0.0 {
                // entering
                inside = true;
                entered = t;
            } else if inside || first {
                // leaving (from the start of the window if first)
                area += (t - if inside { entered } else { 0.0 }) * step;
                inside = false;
            }
            first = false;
            if t >= 2.0 * half {
                break;
            }
        }
        if inside {
            area += (2.0 * half - entered) * step;
        }
    }
    area
}

/// A vertical window of [`section_area`]: the plan point, the normal of the
/// plane and the heights.
type Window = (DVec2, DVec2, f64, f64);

/// The faces of `mesh` that a window can meet (vertices within `edge` of
/// it, `_window_tree`), as a mesh of their own, and their outward normals.
fn window_mesh(mesh: &TriMesh, windows: &[Window], half: f64) -> (TriMesh, Vec<DVec3>) {
    let edge = 1.0;
    let mut near = vec![false; mesh.positions.len()];
    for &(m, v, zlo, zhi) in windows {
        let nv = v / v.length().max(1e-12);
        let ev = DVec2::new(-nv.y, nv.x);
        for (k, p) in mesh.positions.iter().enumerate() {
            let p = p.as_dvec3();
            let r = p.truncate() - m;
            if r.dot(nv).abs() <= edge
                && r.dot(ev).abs() <= half + edge
                && p.z >= zlo - edge
                && p.z <= zhi + 0.1 + edge
            {
                near[k] = true;
            }
        }
    }
    let tris: Vec<[u32; 3]> = mesh
        .tris
        .iter()
        .copied()
        .filter(|t| t.iter().any(|&v| near[v as usize]))
        .collect();
    let normals = tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| mesh.positions[v as usize].as_dvec3());
            (b - a).cross(c - a).normalize_or_zero()
        })
        .collect();
    (TriMesh::new(mesh.positions.clone(), tris), normals)
}

/// Consecutive covered teeth (arch order, a missing tooth skipped) whose caps
/// are in different pieces of the shell `mesh` (every piece of the meshed
/// field, before any is dropped), or joined through less than
/// [`CONNECT_MIN_WIDTH_MM`] x `wall` of section: the section by the vertical
/// plane through the middle of their nearest points, across the line between
/// their centres or between those points, the thinner, within
/// [`CONNECT_SECTION_HALF_MM`] and from 12 mm under the lower top to 1 mm
/// above `outer` over the higher one (`_weak_pairs`). Also the teeth whose
/// cap is not on the largest piece.
pub fn weak_pairs(mesh: &TriMesh, teeth: &[Tooth], wall: f64, outer: f64) -> Joins {
    let by: BTreeMap<Fdi, &Tooth> = teeth
        .iter()
        .filter(|t| t.covered && !tooth_points(t).is_empty())
        .map(|t| (t.fdi, t))
        .collect();
    let fdis: Vec<Fdi> = by.keys().copied().collect();
    let Some(&first) = fdis.first() else {
        return Joins::default();
    };
    let order = anatomy::arch_sequence(&fdis, Arch::of(first));
    if order.len() < 2 || mesh.tris.is_empty() {
        return Joins::default();
    }
    let need = CONNECT_MIN_WIDTH_MM * wall;
    let pts: Vec<(Fdi, &[DVec3])> = order.iter().map(|f| (*f, tooth_points(by[f]))).collect();
    let piece = cap_pieces(mesh, &pts);
    let lost: Vec<Fdi> = order
        .iter()
        .copied()
        .filter(|f| piece.get(f) != Some(&0))
        .collect();
    let top = |p: &[DVec3]| p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
    let centre = |p: &[DVec3]| p.iter().map(|q| q.truncate()).sum::<DVec2>() / p.len() as f64;
    let mut planes: Vec<(Fdi, Fdi, Option<[Window; 2]>)> = Vec::new();
    for w in order.windows(2) {
        let (fa, fb) = (w[0], w[1]);
        if piece.get(&fa) != piece.get(&fb) {
            planes.push((fa, fb, None));
            continue;
        }
        let (pa, pb) = (tooth_points(by[&fa]), tooth_points(by[&fb]));
        let (na, nb, u) = nearest_pair(pa, pb);
        let (ta, tb) = (top(pa), top(pb));
        let zlo = ta.min(tb) - 12.0;
        let zhi = ta.max(tb) + outer + 1.0;
        let mid = 0.5 * (na + nb);
        planes.push((
            fa,
            fb,
            Some([(mid, centre(pb) - centre(pa), zlo, zhi), (mid, u, zlo, zhi)]),
        ));
    }
    let windows: Vec<Window> = planes.iter().filter_map(|p| p.2).flatten().collect();
    let tree = (!windows.is_empty())
        .then(|| {
            let (m, n) = window_mesh(mesh, &windows, CONNECT_SECTION_HALF_MM);
            Bvh::build(&m).ok().map(|b| (b, n))
        })
        .flatten();
    let mut weak = Vec::new();
    for (fa, fb, ws) in planes {
        let area = match (ws, &tree) {
            (None, _) => 0.0,
            (Some(ws), Some((bvh, normals))) => ws
                .iter()
                .map(|&(m, v, zlo, zhi)| {
                    section_area(
                        bvh,
                        normals,
                        m,
                        v,
                        zlo,
                        zhi,
                        CONNECT_SECTION_HALF_MM,
                        SECTION_STEP_MM,
                    )
                })
                .fold(f64::INFINITY, f64::min),
            (Some(_), None) => 0.0,
        };
        if area < need {
            weak.push(Weak {
                a: fa,
                b: fb,
                mm2: (area * 100.0).round() / 100.0,
                split: ws.is_none(),
            });
        }
    }
    Joins { weak, lost }
}

/// What a bar is measured against (the warped space of the insertion paths
/// when there is one): the closed teeth, the gingiva and the height above
/// the model where the rays start.
pub struct BarModel<'a> {
    /// The closed teeth.
    pub teeth: Option<&'a Bvh>,
    /// The gingiva and its mesh (its vertices under the bar count too).
    pub gingiva: Option<(&'a Bvh, &'a TriMesh)>,
    /// Height of the start of the rays (the top of the model + 5 mm).
    pub z_ray: f64,
}

/// First hit (height) of the ray cast down from `z_top` at the plan point
/// `q` (`_column_z`); NaN without one.
fn column_z(bvh: Option<&Bvh>, q: DVec2, z_top: f64) -> f64 {
    bvh.and_then(|b| b.ray_cast(q.extend(z_top).as_vec3(), Vec3::NEG_Z, f32::INFINITY))
        .map_or(f64::NAN, |h| z_top - f64::from(h.t))
}

/// `np.linspace(a, b, n)`.
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![a];
    }
    (0..n)
        .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
        .collect()
}

fn signed_volume(v: &[DVec3], t: &[[u32; 3]]) -> f64 {
    t.iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| v[i as usize]);
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        / 6.0
}

/// Closed prism over the plan grid `q` (`nt` x `nw` nodes, row-major), its
/// top at the heights `top` of the nodes, its bottom flat at `bottom`,
/// outward faces (`_grid_prism`).
fn grid_prism(
    q: &[DVec2],
    top: &[f64],
    bottom: f64,
    nt: usize,
    nw: usize,
) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    let n = (nt * nw) as u32;
    let mut v: Vec<DVec3> = q.iter().map(|p| p.extend(bottom)).collect();
    v.extend(q.iter().zip(top).map(|(p, z)| p.extend(*z)));
    let id = |i: usize, j: usize| (i * nw + j) as u32;
    let mut f = Vec::new();
    for i in 0..nt - 1 {
        for j in 0..nw - 1 {
            let (a, b, c, d) = (id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1));
            f.push([a, d, c]);
            f.push([a, c, b]);
            f.push([a + n, b + n, c + n]);
            f.push([a + n, c + n, d + n]);
        }
    }
    let mut ring: Vec<u32> = (0..nt).map(|i| id(i, 0)).collect();
    ring.extend((1..nw).map(|j| id(nt - 1, j)));
    ring.extend((0..nt - 1).rev().map(|i| id(i, nw - 1)));
    ring.extend((1..nw - 1).rev().map(|j| id(0, j)));
    for k in 0..ring.len() {
        let (r, r2) = (ring[k], ring[(k + 1) % ring.len()]);
        f.push([r, r2, r2 + n]);
        f.push([r, r2 + n, r + n]);
    }
    if signed_volume(&v, &f) < 0.0 {
        for t in &mut f {
            t.swap(1, 2);
        }
    }
    (v, f)
}

/// The bar joining the caps of the consecutive teeth `a` and `b` (their
/// points `pa`, `pb` and their trim loops, warped when `warp` is given) over
/// the space between them (`_bar`; see the module docs). `thin`: the wall is
/// one the fills are for ([`super::shell_fills`]), where a crown overhanging
/// the bar raises its top to the underside of the crown.
#[allow(clippy::too_many_arguments)]
pub fn bar(
    (a, b): (Fdi, Fdi),
    (pa, pb): (&[DVec3], &[DVec3]),
    (la, lb): (Option<&ToothLoop>, Option<&ToothLoop>),
    model: &BarModel<'_>,
    clearance: f64,
    outer: f64,
    thin: bool,
    warp: Option<&PathWarp>,
) -> Bar {
    let (na, nb, u) = nearest_pair(pa, pb);
    let ev = DVec2::new(-u.y, u.x);
    let (p0, p1) = (na - CONNECT_INTO_MM * u, nb + CONNECT_INTO_MM * u);
    let top = |p: &[DVec3]| p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
    let za = la
        .filter(|l| l.points.len() >= 3)
        .map_or(top(pa) - 1.0, |l| loop_nearest(&l.points, nb).1);
    let zb = lb
        .filter(|l| l.points.len() >= 3)
        .map_or(top(pb) - 1.0, |l| loop_nearest(&l.points, na).1);
    // the gingiva under the bar (rays cast from above), where no crown is over it
    let z_ray = model.z_ray;
    let len = (p1 - p0).length();
    let nt = ((len / BAR_GUM_STEP_MM).ceil() as usize + 1).max(2);
    let nw = ((2.0 * CONNECT_HALF_MM / BAR_GUM_STEP_MM).ceil() as usize + 1).max(2);
    let (ts, ws) = (
        linspace(0.0, 1.0, nt),
        linspace(-CONNECT_HALF_MM, CONNECT_HALF_MM, nw),
    );
    let mut q: Vec<DVec2> = Vec::with_capacity(nt * nw);
    for &t in &ts {
        for &w in &ws {
            q.push(p0 + t * (p1 - p0) + w * ev);
        }
    }
    let n = q.len();
    // ... and at every vertex of the gingiva under the bar: its highest
    // points between the rays, so the top stays over all of it
    if let Some((_, gum)) = model.gingiva
        && len > 0.0
    {
        for p in &gum.positions {
            let r = p.as_dvec3().truncate() - p0;
            let (s, w) = (r.dot(p1 - p0) / (len * len), r.dot(ev));
            if (0.0..=1.0).contains(&s) && w.abs() <= CONNECT_HALF_MM {
                q.push(p.as_dvec3().truncate());
            }
        }
    }
    let gbvh = model.gingiva.map(|(b, _)| b);
    let gz: Vec<f64> = q.iter().map(|&p| column_z(gbvh, p, z_ray)).collect();
    let tz: Vec<f64> = q.iter().map(|&p| column_z(model.teeth, p, z_ray)).collect();
    let on: Vec<f64> = gz.iter().copied().filter(|z| z.is_finite()).collect();
    // gingiva with no crown over it (a NaN crown counts as none)
    let g_max = gz
        .iter()
        .zip(&tz)
        .filter(|(g, t)| g.is_finite() && t.partial_cmp(g) != Some(std::cmp::Ordering::Greater))
        .map(|(g, _)| *g)
        .fold(f64::NEG_INFINITY, f64::max);
    let g_lo = if on.is_empty() {
        za.min(zb) - 3.0
    } else {
        on.iter().copied().fold(f64::INFINITY, f64::min)
    };
    let lift = if g_max.is_finite() {
        g_max + clearance + EQUATOR_GUM_GAP_MM
    } else {
        f64::NEG_INFINITY
    };
    // under the equators by CONNECT_DROP_MM, at most the wall less
    // CONNECT_OVER_MM: the walls beside the two crowns join along the bar too
    let drop =
        CONNECT_DROP_MM.min((((outer - clearance - CONNECT_OVER_MM) * 1e6).round() / 1e6).max(0.0));
    let (z0, z1) = ((za - drop).max(lift), (zb - drop).max(lift));
    let bottom = g_lo - 1.0;
    // a crown overhanging the bar (a tooth tipped over the space): the top
    // raised to the underside of the crown, on a thin wall only
    let zlin: Vec<f64> = ts
        .iter()
        .flat_map(|&t| std::iter::repeat_n(z0 + t * (z1 - z0), nw))
        .collect();
    let mut up = vec![false; n];
    let mut under = vec![f64::NAN; n];
    if thin && let Some(tb) = model.teeth {
        for k in 0..n {
            let hits = tb.ray_hits(
                q[k].extend(z_ray).as_vec3(),
                Vec3::NEG_Z,
                f32::INFINITY,
                2,
                1e-4,
            );
            let (ctop, und) = (
                hits.first().map_or(f64::NAN, |h| z_ray - f64::from(h.t)),
                hits.get(1).map_or(f64::NAN, |h| z_ray - f64::from(h.t)),
            );
            let g = if gz[k].is_nan() {
                f64::NEG_INFINITY
            } else {
                gz[k]
            };
            up[k] = und.is_finite()
                && ctop.is_finite()
                && ctop > g + EQUATOR_GUM_GAP_MM
                && und > zlin[k];
            under[k] = und;
        }
    }
    let (mut v, tris, mut path) = if up.iter().any(|&x| x) {
        let tops: Vec<f64> = (0..n)
            .map(|k| {
                if up[k] {
                    under[k] + 0.5 * CROWN_GAP_MM
                } else {
                    zlin[k]
                }
            })
            .collect();
        let (v, t) = grid_prism(&q[..n], &tops, bottom, nt, nw);
        let cols: Vec<usize> = linspace(0.0, (nw - 1) as f64, PATH_COLUMNS)
            .iter()
            .map(|c| *c as usize)
            .collect();
        let path: Vec<DVec3> = (0..nt)
            .flat_map(|i| cols.iter().map(move |&j| (i, j)))
            .map(|(i, j)| q[i * nw + j].extend(tops[i * nw + j]))
            .collect();
        (v, t, path)
    } else {
        // the box under the top, down into the gingiva
        let corners = [
            p0 - CONNECT_HALF_MM * ev,
            p1 - CONNECT_HALF_MM * ev,
            p1 + CONNECT_HALF_MM * ev,
            p0 + CONNECT_HALF_MM * ev,
        ];
        let tops = [z0, z1, z1, z0];
        let mut v: Vec<DVec3> = corners.iter().map(|c| c.extend(bottom)).collect();
        v.extend(corners.iter().zip(tops).map(|(c, z)| c.extend(z)));
        let mut t: Vec<[u32; 3]> = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        if signed_volume(&v, &t) < 0.0 {
            for f in &mut t {
                f.swap(1, 2);
            }
        }
        let across = linspace(-CONNECT_HALF_MM, CONNECT_HALF_MM, PATH_COLUMNS);
        let path: Vec<DVec3> = ts
            .iter()
            .flat_map(|&t| {
                across
                    .iter()
                    .map(move |&w| (p0 + t * (p1 - p0) + w * ev).extend(z0 + t * (z1 - z0)))
            })
            .collect();
        (v, t, path)
    };
    if let Some(w) = warp {
        for p in v.iter_mut().chain(path.iter_mut()) {
            *p = w.from_warp(*p);
        }
    }
    Bar {
        teeth: (a, b),
        p0,
        p1,
        z0,
        z1,
        solid: TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), tris),
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// A closed box `[lo, hi]` with outward faces.
    fn cube(lo: DVec3, hi: DVec3) -> TriMesh {
        let c = |i: usize| {
            DVec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
            .as_vec3()
        };
        let tris = vec![
            [0, 2, 3],
            [0, 3, 1],
            [4, 5, 7],
            [4, 7, 6],
            [0, 1, 5],
            [0, 5, 4],
            [2, 6, 7],
            [2, 7, 3],
            [0, 4, 6],
            [0, 6, 2],
            [1, 3, 7],
            [1, 7, 5],
        ];
        TriMesh::new((0..8).map(c).collect(), tris)
    }

    #[test]
    fn the_section_of_a_box_is_its_cross_section() {
        let m = cube(DVec3::new(-1.0, -2.0, 0.0), DVec3::new(1.0, 2.0, 3.0));
        let normals: Vec<DVec3> = m
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| m.positions[v as usize].as_dvec3());
                (b - a).cross(c - a).normalize()
            })
            .collect();
        let bvh = Bvh::build(&m).unwrap();
        // the plane x = 0 cuts 4 mm x 3 mm; the window holds all of it
        let a = section_area(&bvh, &normals, DVec2::ZERO, DVec2::X, -1.0, 4.0, 5.0, 0.1);
        assert!((a - 12.0).abs() < 0.25, "{a}");
        // a line starting inside the solid counts from the window: from
        // y = 0.5 to the face at y = 2 on 10 rows (a line all inside, never
        // crossing the surface, counts nothing, as in the plugin)
        let a = section_area(
            &bvh,
            &normals,
            DVec2::new(0.0, 1.5),
            DVec2::X,
            0.05,
            1.0,
            1.0,
            0.1,
        );
        assert!((a - 1.5).abs() < 1e-4, "{a}");
        let a = section_area(&bvh, &normals, DVec2::ZERO, DVec2::X, 0.05, 1.0, 1.0, 0.1);
        assert_eq!(a, 0.0);
        // a plane beside it: nothing
        let a = section_area(
            &bvh,
            &normals,
            DVec2::new(3.0, 0.0),
            DVec2::X,
            -1.0,
            4.0,
            5.0,
            0.1,
        );
        assert_eq!(a, 0.0);
    }

    #[test]
    fn the_ceiling_follows_the_top_within_the_half_width() {
        let b = Bar {
            teeth: (fdi(11), fdi(21)),
            p0: DVec2::ZERO,
            p1: DVec2::new(10.0, 0.0),
            z0: 1.0,
            z1: 3.0,
            solid: TriMesh::new(Vec::new(), Vec::new()),
            path: Vec::new(),
        };
        assert!((b.ceiling(DVec2::new(5.0, 1.0)) - 2.0).abs() < 1e-12);
        assert!((b.ceiling(DVec2::new(-1.0, 0.0)) - 1.0).abs() < 1e-12);
        assert_eq!(
            b.ceiling(DVec2::new(5.0, CONNECT_HALF_MM + 0.01)),
            f64::INFINITY
        );
        assert_eq!(ceiling(&[], DVec2::ZERO), f64::INFINITY);
    }

    #[test]
    fn the_cap_of_every_tooth_is_on_its_own_piece() {
        // two boxes far apart: two pieces of the same size
        let mut m = cube(DVec3::new(0.0, 0.0, 0.0), DVec3::new(2.0, 2.0, 2.0));
        let other = cube(DVec3::new(10.0, 0.0, 0.0), DVec3::new(12.0, 2.0, 2.0));
        let off = m.positions.len() as u32;
        m.positions.extend(other.positions);
        m.tris.extend(other.tris.iter().map(|t| t.map(|v| v + off)));
        let a = [DVec3::new(1.0, 1.0, 1.5)];
        let b = [DVec3::new(11.0, 1.0, 1.5)];
        let p = cap_pieces(&m, &[(fdi(11), &a), (fdi(21), &b)]);
        assert_eq!(p.len(), 2);
        assert_ne!(p[&fdi(11)], p[&fdi(21)]);
    }

    /// Points on the surface of the box `[lo, hi]`, about every `step`.
    fn box_points(lo: DVec3, hi: DVec3, step: f64) -> Vec<DVec3> {
        let n = ((hi - lo) / step).ceil().as_uvec3();
        let mut out = Vec::new();
        for i in 0..=n.x {
            for j in 0..=n.y {
                for k in 0..=n.z {
                    let on = i == 0 || j == 0 || k == 0 || i == n.x || j == n.y || k == n.z;
                    if on {
                        let f = DVec3::new(i as f64, j as f64, k as f64) / n.as_dvec3();
                        out.push(lo + (hi - lo) * f);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_box_bar_spans_the_space_into_both_teeth() {
        // two teeth 8 mm apart along x, the gingiva a plane at z = 0
        let (la, ha) = (DVec3::new(-5.0, -3.0, 0.0), DVec3::new(0.0, 3.0, 8.0));
        let (lb, hb) = (DVec3::new(8.0, -3.0, 0.0), DVec3::new(13.0, 3.0, 8.0));
        let (ta, tb) = (cube(la, ha), cube(lb, hb));
        let (pa, pb) = (box_points(la, ha, 0.25), box_points(lb, hb, 0.25));
        let gum = cube(DVec3::new(-10.0, -10.0, -4.0), DVec3::new(20.0, 10.0, 0.5));
        let mut teeth = ta.clone();
        let off = teeth.positions.len() as u32;
        teeth.positions.extend(tb.positions.clone());
        teeth
            .tris
            .extend(tb.tris.iter().map(|t| t.map(|v| v + off)));
        let (tbvh, gbvh) = (Bvh::build(&teeth).unwrap(), Bvh::build(&gum).unwrap());
        let model = BarModel {
            teeth: Some(&tbvh),
            gingiva: Some((&gbvh, &gum)),
            z_ray: 20.0,
        };
        let b = bar(
            (fdi(11), fdi(21)),
            (&pa, &pb),
            (None, None),
            &model,
            0.0,
            1.0,
            false,
            None,
        );
        // from 1 mm inside one tooth to 1 mm inside the other
        assert!(
            (b.p0.x + 1.0).abs() < 1e-9 && (b.p1.x - 9.0).abs() < 1e-9,
            "{b:?}"
        );
        // the top under the tops by 1 + 0.7 mm, over the gingiva
        assert!((b.z0 - (8.0 - 1.7)).abs() < 1e-9 && (b.z1 - (8.0 - 1.7)).abs() < 1e-9);
        let report = b.solid.validate();
        assert!(report.is_watertight() && report.is_manifold());
        assert!(
            signed_volume(
                &b.solid
                    .positions
                    .iter()
                    .map(|p| p.as_dvec3())
                    .collect::<Vec<_>>(),
                &b.solid.tris
            ) > 0.0
        );
        // down into the gingiva
        let low = b
            .solid
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f32::INFINITY, f32::min);
        assert!(f64::from(low) < 0.5 - 1.0 + 1e-6);
        assert_eq!(b.path.len() % PATH_COLUMNS, 0);
        assert!(!b.top_strip().tris.is_empty());
    }
}
