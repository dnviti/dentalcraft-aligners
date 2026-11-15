//! Contact bridges of the print model (`case_print.contact_bridges`,
//! `pair_bridge`): closed solids filling the interproximal space around the
//! contact of every two neighbouring teeth.
//!
//! The closed teeth touch at the contact point and the gingival embrasure
//! under it is open (the papilla fills it in the mouth); on the model the
//! sheet would enter it and form a bead, and a narrow slot between two teeth
//! the closing joins only in places leaves tunnels through the model. As on
//! the lab model it is blocked out, and nothing else: the space between two
//! touching teeth is sampled with lines along the arch (the mesiodistal
//! direction of the pair, horizontal, one every [`STEP_MM`] in the plane of
//! the contact), and a line is filled from the surface of one tooth to the
//! surface of the other where they face each other along it and are closer
//! than the width allowed at its height: [`EMBRASURE_MM`] up to the top of
//! the contact, then shrinking by [`SLOPE`] per mm down to the reach
//! ([`crate::reconstruct::BRIDGE_MM`]), kept at any height (a narrow slot is
//! bridged, as the scanner does).
//!
//! Under the contact every column of lines is filled on down to the
//! gingiva whatever its width (an open black triangle would be a tunnel
//! through the model); a line is as narrow as the distance along it or,
//! where the faces are oblique to the arch, from where it leaves one tooth
//! to the other; a window left in a column between two filled parts is
//! filled, and pieces of fill apart along the arch are joined
//! ([`connect_cells`]). Teeth that do not touch are not bridged.
//!
//! The plugin works in the local frame of every tooth (one tree per tooth
//! for every stage); here the teeth are placed at the stage already, which
//! is the same geometry (the teeth move rigidly).

use std::collections::VecDeque;

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_geom::knn::KnnIndex;
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};
use rayon::prelude::*;

use super::embrasure::{Solid, TOUCH_MM, Tooth};
use crate::anatomy::{Arch, arch_order};
use crate::segmentation::transfer::{closest_tri, d2};

/// Below the contact the embrasure is blocked out where narrower than this
/// (mm, `EMBRASURE_MM`).
pub const EMBRASURE_MM: f64 = 1.5;
/// Above the top of the contact the blocked width shrinks by this much per
/// mm (`BRIDGE_SLOPE`).
pub const SLOPE: f64 = 0.5;
/// Spacing of the lines sampling an embrasure (mm, `BRIDGE_STEP_MM`, the
/// voxel of the model).
pub const STEP_MM: f64 = 0.1;
/// A cell of the bridge joins four lines whose ends (in each tooth) are at
/// most this many steps apart along the arch (`BRIDGE_JUMP_STEPS`).
pub const JUMP_STEPS: f64 = 6.0;
/// ... and a window between two filled parts of a column is filled where
/// its ends jump at most this many (`BRIDGE_WINDOW_JUMP_STEPS`).
pub const WINDOW_JUMP_STEPS: f64 = 10.0;
/// A line is measured in 3D too where at most this long (mm,
/// `BRIDGE_SLAB_MM`).
pub const SLAB_MM: f64 = 3.0;
/// A way joining two pieces of a bridge is widened by this many cells on
/// each side (`BRIDGE_LINK_GROW`).
pub const LINK_GROW: usize = 2;
/// The printed model also bridges the pairs up to `MODEL_TOUCH_MM +
/// MODEL_TOUCH_VOXELS x voxel` apart ([`model_touch_mm`]).
pub const MODEL_TOUCH_MM: f64 = 0.65;
/// See [`MODEL_TOUCH_MM`].
pub const MODEL_TOUCH_VOXELS: f64 = 2.7;
/// Rays along the lines reach this far (mm).
const RAY_MM: f64 = 15.0;
/// A hit nearer the origin of a ray than this times (scale of the mesh +
/// largest coordinate of the origin, centred) is none: the rounding of an
/// origin on the surface (`gpu_query.RAY_EPS`).
const RAY_EPS: f64 = 1.0 / (1u64 << 30) as f64;
/// A triangle with `|e1 x e2|^2 <= AREA_EPS |e1|^2 |e2|^2` has no area for a
/// ray (`gpu_query.AREA_EPS`).
const AREA_EPS: f64 = 1e-20;
/// A float32 hit this close to an edge (barycentric) brings the faces around
/// it as candidates (`gpu_query.EDGE_BARY`).
const EDGE_BARY: f64 = 1e-3;
/// Margin of the float32 candidate searches (mm): far beyond their rounding.
const CANDIDATE_MM: f64 = 0.01;

/// The surface gap up to which two neighbouring teeth are bridged in a
/// printed model built at `voxel` (at least [`TOUCH_MM`]): the largest gap
/// its blockout can join, with a margin (`model_touch_mm`).
pub fn model_touch_mm(voxel: f64) -> f64 {
    TOUCH_MM.max(MODEL_TOUCH_MM + MODEL_TOUCH_VOXELS * voxel)
}

/// Sizes of the bridges (the arguments of `contact_bridges`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeParams {
    /// Width kept at any height (mm, `reach`).
    pub reach: f64,
    /// Width blocked out under the top of the contact (mm, `gingival`).
    pub gingival: f64,
    /// Surface gap up to which two teeth touch (mm, `touch`).
    pub touch: f64,
    /// Only the pairs farther apart than this are bridged (`above`; the
    /// model bridges the pairs its coarser voxel joins).
    pub above: Option<f64>,
    /// The labial and lingual faces of every bridge this far inside the
    /// outermost lines meeting both teeth (mm, `inset`: the smooth border).
    pub inset: f64,
}

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            reach: crate::reconstruct::BRIDGE_MM,
            gingival: EMBRASURE_MM,
            touch: TOUCH_MM,
            above: None,
            inset: 0.0,
        }
    }
}

/// A placed tooth ready for the queries of [`pair_bridge`]. Its hits and
/// distances are those of the plugin's float64 queries (`gpu_query.Mesh`):
/// the faces come from the [`Bvh`] (float32), every candidate is measured
/// again in float64 on coordinates centred on the box of the mesh, the
/// smallest parameter or distance winning, ties to the smallest face.
struct Prepared<'a> {
    points: &'a [DVec3],
    knn: KnnIndex,
    bvh: Bvh,
    /// Centre of the box of the vertices of the faces (`_centre`).
    c: DVec3,
    /// Corners of every face, centred on `c`.
    w: Vec<[DVec3; 3]>,
    /// Largest coordinate of `w` (`Mesh.scale`).
    scale: f64,
    /// Unit normal of every face (`Mesh.normals`).
    normals: Vec<DVec3>,
    lo: DVec3,
    hi: DVec3,
    mean: DVec3,
}

impl<'a> Prepared<'a> {
    fn new(t: &Tooth<'a>) -> Option<Self> {
        if t.points.is_empty() || t.tris.is_empty() {
            return None;
        }
        let pts: Vec<Vec3> = t.points.iter().map(|p| p.as_vec3()).collect();
        let knn = KnnIndex::build(&pts).ok()?;
        let bvh = Bvh::build(&TriMesh::new(pts, t.tris.to_vec())).ok()?;
        let (blo, bhi) = t.tris.iter().flatten().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), &i| (lo.min(t.points[i as usize]), hi.max(t.points[i as usize])),
        );
        let c = 0.5 * (blo + bhi);
        let w: Vec<[DVec3; 3]> = t
            .tris
            .iter()
            .map(|f| f.map(|i| t.points[i as usize] - c))
            .collect();
        let scale = w
            .iter()
            .flatten()
            .fold(0.0f64, |m, p| m.max(p.abs().max_element()));
        let normals = w
            .iter()
            .map(|[a, b, c]| {
                let n = (*b - *a).cross(*c - *a);
                n / n.length().max(1e-300)
            })
            .collect();
        let (lo, hi) = t.points.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        let mean = t.points.iter().copied().sum::<DVec3>() / t.points.len() as f64;
        Some(Self {
            points: t.points,
            knn,
            bvh,
            c,
            w,
            scale,
            normals,
            lo,
            hi,
            mean,
        })
    }

    /// First hit of the ray `o + t dir` (`dir` normalised here) with `t` from
    /// `RAY_EPS (scale + max |o - c|)` (a ray starting on the surface never
    /// hits it there) to [`RAY_MM`] (`Mesh.ray_cast`): its parameter and the
    /// normal of the face hit along `u`.
    fn hit(&self, o: DVec3, dir: DVec3, u: DVec3) -> Option<(f64, f64)> {
        let d = dir / dir.length().max(1e-300);
        let oc = o - self.c;
        let start = RAY_EPS * (self.scale + oc.abs().max_element());
        // every face the float32 ray meets from a little behind the origin,
        // and near an edge the faces around the hit, measured in float64:
        // the smallest parameter wins, ties to the smallest face
        let back = o - CANDIDATE_MM * d;
        let mut best: Option<(f64, u32)> = None;
        let mut take = |f: u32, t: f64| {
            if t >= start
                && t <= RAY_MM
                && best.is_none_or(|(bt, bf)| t < bt || (t == bt && f < bf))
            {
                best = Some((t, f));
            }
        };
        for h in self.bvh.ray_all(
            back.as_vec3(),
            d.as_vec3(),
            (RAY_MM + 2.0 * CANDIDATE_MM) as f32,
        ) {
            let (t, b) = ray_tri(oc, d, &self.w[h.face as usize]);
            take(h.face, t);
            if b.is_nan() || b < EDGE_BARY {
                let at = back + f64::from(h.t) * d;
                for f in self.bvh.overlap_sphere(at.as_vec3(), CANDIDATE_MM as f32) {
                    take(f, ray_tri(oc, d, &self.w[f as usize]).0);
                }
            }
        }
        best.map(|(t, f)| (t, self.normals[f as usize].dot(u)))
    }

    /// Distance from `p` to the surface, `inf` beyond `max` (`Mesh.nearest`).
    fn surface(&self, p: DVec3, max: f64) -> f64 {
        let Some(h) = self
            .bvh
            .closest_point(p.as_vec3(), (max + CANDIDATE_MM) as f32)
        else {
            return f64::INFINITY;
        };
        let q = p - self.c;
        let dist = |f: u32| {
            let [a, b, c] = self.w[f as usize];
            d2(closest_tri(q, a, b, c), q).sqrt()
        };
        let d0 = dist(h.face);
        let best = self
            .bvh
            .overlap_sphere(p.as_vec3(), (d0 * (1.0 + 1e-6) + CANDIDATE_MM) as f32)
            .into_iter()
            .map(dist)
            .fold(d0, f64::min);
        if best <= max { best } else { f64::INFINITY }
    }
}

/// Parameter of the hit of the ray `(o, d)` on the triangle `w` (centred
/// coordinates), `-1` for none, and the smallest barycentric coordinate of
/// the hit point on its plane (`gpu_query._ray_tri_b`: Moller-Trumbore, both
/// sides, a triangle of no area never hit).
fn ray_tri(o: DVec3, d: DVec3, w: &[DVec3; 3]) -> (f64, f64) {
    let t0 = w[0];
    let (e1x, e1y, e1z) = (w[1].x - t0.x, w[1].y - t0.y, w[1].z - t0.z);
    let (e2x, e2y, e2z) = (w[2].x - t0.x, w[2].y - t0.y, w[2].z - t0.z);
    let (px, py, pz) = (
        d.y * e2z - d.z * e2y,
        d.z * e2x - d.x * e2z,
        d.x * e2y - d.y * e2x,
    );
    let det = e1x * px + e1y * py + e1z * pz;
    let inv = 1.0 / det;
    let (sx, sy, sz) = (o.x - t0.x, o.y - t0.y, o.z - t0.z);
    let u = (sx * px + sy * py + sz * pz) * inv;
    let (qx, qy, qz) = (
        sy * e1z - sz * e1y,
        sz * e1x - sx * e1z,
        sx * e1y - sy * e1x,
    );
    let v = (d.x * qx + d.y * qy + d.z * qz) * inv;
    let t = (e2x * qx + e2y * qy + e2z * qz) * inv;
    let (nx, ny, nz) = (
        e1y * e2z - e1z * e2y,
        e1z * e2x - e1x * e2z,
        e1x * e2y - e1y * e2x,
    );
    let n2 = nx * nx + ny * ny + nz * nz;
    let l2 = (e1x * e1x + e1y * e1y + e1z * e1z) * (e2x * e2x + e2y * e2y + e2z * e2z);
    let ok =
        det != 0.0 && (0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && n2 > AREA_EPS * l2;
    (if ok { t } else { -1.0 }, u.min(v).min(1.0 - u - v))
}

/// Rows of a grid of `n x m` booleans, row-major.
struct Grid {
    n: usize,
    m: usize,
}

impl Grid {
    fn at(&self, i: usize, j: usize) -> usize {
        i * self.m + j
    }

    /// The 4 neighbours of cell `k` inside the grid.
    fn neighbours(&self, k: usize) -> impl Iterator<Item = usize> + '_ {
        let (i, j) = (k / self.m, k % self.m);
        [
            (i.wrapping_sub(1), j),
            (i + 1, j),
            (i, j.wrapping_sub(1)),
            (i, j + 1),
        ]
        .into_iter()
        .filter(|&(a, b)| a < self.n && b < self.m)
        .map(|(a, b)| a * self.m + b)
    }

    /// Morphological opening with a 3 x 3 square (`_opening`, r = 1): parts
    /// thinner than that are dropped. Outside the grid counts as empty.
    fn opening(&self, f: &[bool]) -> Vec<bool> {
        let pass = |g: &[bool], erode: bool| -> Vec<bool> {
            (0..self.n * self.m)
                .map(|k| {
                    let (i, j) = ((k / self.m) as isize, (k % self.m) as isize);
                    let mut acc = erode;
                    for di in -1..=1isize {
                        for dj in -1..=1isize {
                            let (a, b) = (i + di, j + dj);
                            let v = a >= 0
                                && b >= 0
                                && (a as usize) < self.n
                                && (b as usize) < self.m
                                && g[a as usize * self.m + b as usize];
                            acc = if erode { acc && v } else { acc || v };
                        }
                    }
                    acc
                })
                .collect()
        };
        pass(&pass(f, true), false)
    }

    /// Breadth-first steps (4 neighbours) from the cells `src` through the
    /// cells `f` (`_grid_steps`): the step of every cell (`-1` not reached)
    /// and the cell it was reached from (`usize::MAX` at the sources).
    fn steps(&self, f: &[bool], src: &[bool]) -> (Vec<i64>, Vec<usize>) {
        let size = self.n * self.m;
        let mut step = vec![-1i64; size];
        let mut par = vec![usize::MAX; size];
        let mut queue = VecDeque::new();
        for k in (0..size).filter(|&k| src[k]) {
            step[k] = 0;
            queue.push_back(k);
        }
        while let Some(k) = queue.pop_front() {
            for kk in self.neighbours(k).collect::<Vec<_>>() {
                if step[kk] < 0 && f[kk] {
                    step[kk] = step[k] + 1;
                    par[kk] = k;
                    queue.push_back(kk);
                }
            }
        }
        (step, par)
    }
}

/// Cells joining every piece of the filled `cells` of a bridge (4
/// neighbours) to the largest one through the cells `free` (lines meeting
/// both teeth), `_connect_cells`: two pieces of fill each joining the two
/// teeth make a ring with them, a tunnel through the model. The shortest
/// way between them (in cells), widened by [`LINK_GROW`] cells within
/// `free`, is filled; a piece no way reaches is left as it is.
fn connect_cells(g: &Grid, cells: &[bool], free: &[bool]) -> Vec<bool> {
    let size = g.n * g.m;
    let mut add = vec![false; size];
    let mut lab = vec![usize::MAX; size];
    let mut sizes: Vec<usize> = Vec::new();
    for k in (0..size).filter(|&k| cells[k]) {
        if lab[k] == usize::MAX {
            let mut src = vec![false; size];
            src[k] = true;
            let (st, _) = g.steps(cells, &src);
            let mut n = 0;
            for (c, s) in st.iter().enumerate() {
                if *s >= 0 {
                    lab[c] = sizes.len();
                    n += 1;
                }
            }
            sizes.push(n);
        }
    }
    if sizes.len() < 2 {
        return add;
    }
    // the largest piece (the first of equals, `argmax`), then the others by
    // size, largest first, as `np.argsort(sizes)[::-1][1:]` orders them
    let largest = (0..sizes.len()).fold(0, |b, c| if sizes[c] > sizes[b] { c } else { b });
    let mut main: Vec<bool> = lab.iter().map(|&l| l == largest).collect();
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&c| sizes[c]);
    order.reverse();
    for &c in &order[1..] {
        let piece: Vec<bool> = lab.iter().map(|&l| l == c).collect();
        if (0..size).any(|k| main[k] && piece[k]) {
            continue;
        }
        let through: Vec<bool> = (0..size).map(|k| free[k] || main[k] || piece[k]).collect();
        let (st, par) = g.steps(&through, &main);
        let Some(mut k) = (0..size)
            .filter(|&k| piece[k] && st[k] >= 0)
            .reduce(|a, b| if st[b] < st[a] { b } else { a })
        else {
            continue;
        };
        let mut way = vec![false; size];
        loop {
            way[k] = true;
            if par[k] == usize::MAX {
                break;
            }
            k = par[k];
        }
        for _ in 0..LINK_GROW {
            let mut grown = way.clone();
            for (k, w) in way.iter().enumerate() {
                if *w {
                    for kk in g.neighbours(k) {
                        grown[kk] = true;
                    }
                }
            }
            way = (0..size).map(|k| grown[k] && through[k]).collect();
        }
        for k in 0..size {
            add[k] |= way[k] && !cells[k];
            main[k] |= piece[k] || way[k];
        }
    }
    add
}

/// The closed solid swept along `u` over the filled cells `f` (`nv - 1` by
/// `nz - 1`) of a grid of lines (`_gap_solid`): node `(i, j)` starts at
/// `x[k][..3]` and spans `x[k][3]..x[k][4]` along `u`. Two sheets (one in
/// each tooth) joined by walls along the border of the cells, the border
/// smoothed along itself.
fn gap_solid(x: &[[f64; 5]], nv: usize, nz: usize, u: DVec3, cells: &[bool]) -> Option<Solid> {
    let (ni, nj) = (nv - 1, nz - 1);
    let mut f = cells.to_vec();
    // cells touching only at a corner would share a wall edge four times
    loop {
        let (mut p1, mut p2) = (Vec::new(), Vec::new());
        for i in 0..ni.saturating_sub(1) {
            for j in 0..nj.saturating_sub(1) {
                let (a, b, c, e) = (
                    f[i * nj + j],
                    f[(i + 1) * nj + j],
                    f[i * nj + j + 1],
                    f[(i + 1) * nj + j + 1],
                );
                if a && e && !b && !c {
                    p1.push(i * nj + j);
                }
                if b && c && !a && !e {
                    p2.push((i + 1) * nj + j);
                }
            }
        }
        if p1.is_empty() && p2.is_empty() {
            break;
        }
        for k in p1.into_iter().chain(p2) {
            f[k] = false;
        }
    }
    if !f.iter().any(|c| *c) {
        return None;
    }
    let mut used = vec![false; nv * nz];
    let filled: Vec<(usize, usize)> = (0..ni * nj)
        .filter(|&k| f[k])
        .map(|k| (k / nj, k % nj))
        .collect();
    for &(i, j) in &filled {
        for (a, b) in [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)] {
            used[a * nz + b] = true;
        }
    }
    let mut nid = vec![u32::MAX; nv * nz];
    let mut nodes = Vec::new();
    for k in (0..nv * nz).filter(|&k| used[k]) {
        nid[k] = nodes.len() as u32;
        nodes.push(k);
    }
    let m = nodes.len() as u32;
    let id = |i: usize, j: usize| nid[i * nz + j];
    let corners: Vec<[u32; 4]> = filled
        .iter()
        .map(|&(i, j)| [id(i, j), id(i + 1, j), id(i, j + 1), id(i + 1, j + 1)])
        .collect();
    // (v, z) counterclockwise is +u: the sheet `hi` faces +u, `lo` faces -u
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(4 * corners.len());
    tris.extend(corners.iter().map(|c| [c[0] + m, c[1] + m, c[3] + m]));
    tris.extend(corners.iter().map(|c| [c[0] + m, c[3] + m, c[2] + m]));
    tris.extend(corners.iter().map(|c| [c[0], c[2], c[3]]));
    tris.extend(corners.iter().map(|c| [c[0], c[3], c[1]]));
    let filled_at = |i: isize, j: isize| -> bool {
        i >= 0
            && j >= 0
            && (i as usize) < ni
            && (j as usize) < nj
            && f[i as usize * nj + j as usize]
    };
    // walls on the border edges p -> q (counterclockwise around the cell)
    let mut border: Vec<(u32, u32)> = Vec::new();
    for ((pi, qi), (di, dj)) in [
        ((0, 1), (0isize, -1isize)),
        ((1, 3), (1, 0)),
        ((3, 2), (0, 1)),
        ((2, 0), (-1, 0)),
    ] {
        let edges: Vec<(u32, u32)> = filled
            .iter()
            .zip(&corners)
            .filter(|((i, j), _)| !filled_at(*i as isize + di, *j as isize + dj))
            .map(|(_, c)| (c[pi], c[qi]))
            .collect();
        tris.extend(edges.iter().map(|&(p, q)| [p, q, q + m]));
        tris.extend(edges.iter().map(|&(p, q)| [p, q + m, p + m]));
        border.extend(edges);
    }
    // the border follows the grid in steps: smoothed along itself
    let mut xs: Vec<[f64; 5]> = nodes.iter().map(|&k| x[k]).collect();
    let mut on = vec![false; nodes.len()];
    for &(p, q) in &border {
        on[p as usize] = true;
        on[q as usize] = true;
    }
    for _ in 0..10 {
        let mut acc = vec![[0.0f64; 5]; nodes.len()];
        for &(p, q) in &border {
            for c in 0..5 {
                acc[p as usize][c] += xs[q as usize][c];
                acc[q as usize][c] += xs[p as usize][c];
            }
        }
        for (k, x) in xs.iter_mut().enumerate() {
            if on[k] {
                for c in 0..5 {
                    x[c] += 0.5 * (acc[k][c] / 2.0 - x[c]);
                }
            }
        }
    }
    let p = |x: &[f64; 5]| DVec3::new(x[0], x[1], x[2]);
    let mut v: Vec<DVec3> = xs.iter().map(|x| p(x) + x[3] * u).collect();
    v.extend(xs.iter().map(|x| p(x) + x[4] * u));
    Some((v, tris))
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

/// The bridge between two consecutive teeth `a`, `b` (`pair_bridge`);
/// `None` when they do not touch (farther apart than `touch`, or not
/// farther than `above`).
fn pair_bridge(a: &Prepared<'_>, b: &Prepared<'_>, p: &BridgeParams) -> Option<Solid> {
    let touch = p.touch;
    // the contact itself (up to `touch` wide) is filled at its height
    let wmax = p.gingival.max(p.reach).max(touch);
    let h = STEP_MM;
    // vertices of each tooth near the other one, with their distance from it
    let near_of = |pp: &Prepared<'_>, qq: &Prepared<'_>| -> (Vec<DVec3>, Vec<f64>) {
        let lo = qq.lo - DVec3::splat(wmax + 1.0);
        let hi = qq.hi + DVec3::splat(wmax + 1.0);
        let pts: Vec<DVec3> = pp
            .points
            .iter()
            .copied()
            .filter(|x| x.cmpge(lo).all() && x.cmple(hi).all())
            .collect();
        let d = pts
            .par_iter()
            .map(|x| {
                qq.knn
                    .nearest(x.as_vec3())
                    .map_or(f64::INFINITY, |n| f64::from(n.distance))
            })
            .collect();
        (pts, d)
    };
    let sides = [near_of(a, b), near_of(b, a)];
    if sides.iter().any(|(pts, _)| pts.is_empty()) {
        return None;
    }
    let dmin = sides
        .iter()
        .flat_map(|(_, d)| d.iter().copied())
        .fold(f64::INFINITY, f64::min);
    if dmin > touch + 0.5 {
        return None;
    }
    // the gap from the surface of the other tooth (the nearest vertex is up
    // to its spacing farther)
    let mut gap = dmin;
    for ((pts, d), other) in sides.iter().zip([b, a]) {
        let near: Vec<DVec3> = pts
            .iter()
            .zip(d)
            .filter(|(_, d)| **d <= touch + 0.5)
            .map(|(x, _)| *x)
            .collect();
        let m = near
            .par_iter()
            .map(|x| other.surface(*x, touch + 0.5))
            .reduce(|| f64::INFINITY, f64::min);
        gap = gap.min(m);
    }
    if gap > touch || p.above.is_some_and(|above| gap <= above) {
        return None;
    }
    // top of the contact: the highest points where the two teeth touch
    let tops: Vec<Vec<DVec3>> = sides
        .iter()
        .map(|(pts, d)| {
            pts.iter()
                .zip(d)
                .filter(|(_, d)| **d <= dmin + 0.1)
                .map(|(x, _)| *x)
                .collect()
        })
        .collect();
    if tops.iter().any(Vec::is_empty) {
        return None;
    }
    let z_c = tops
        .iter()
        .map(|t| t.iter().map(|x| x.z).fold(f64::NEG_INFINITY, f64::max))
        .fold(f64::INFINITY, f64::min);
    let all_tops: Vec<DVec3> = tops.concat();
    let c = all_tops.iter().copied().sum::<DVec3>() / all_tops.len() as f64;
    let mut u = b.mean - a.mean;
    u.z = 0.0;
    u /= u.length().max(1e-9);
    let ev = DVec3::Z.cross(u);
    // lines in the plane of the contact, over the parts of the teeth near
    // each other
    let near: Vec<DVec3> = sides
        .iter()
        .flat_map(|(pts, d)| {
            pts.iter()
                .zip(d)
                .filter(|(_, d)| **d <= wmax + 0.5)
                .map(|(x, _)| *x)
        })
        .collect();
    let (vmin, vmax) = near
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
            let v = (*x - c).dot(ev);
            (lo.min(v), hi.max(v))
        });
    let (v0, v1) = (vmin - h, vmax + 2.0 * h);
    let nv = ((v1 - v0) / h).ceil().max(0.0) as usize;
    let vs: Vec<f64> = (0..nv).map(|k| v0 + k as f64 * h).collect();
    let reach_top = sides
        .iter()
        .flat_map(|(pts, d)| {
            pts.iter()
                .zip(d)
                .filter(|(_, d)| **d <= p.reach + 0.2)
                .map(|(x, _)| x.z)
        })
        .fold(-1e9, f64::max);
    let z_top = (z_c + (wmax - p.reach) / SLOPE).max(reach_top) + h;
    // down to the lower of the two teeth (the column fill under the contact
    // reaches the gingiva)
    let low = near
        .iter()
        .chain(a.points)
        .chain(b.points)
        .map(|x| x.z)
        .fold(f64::INFINITY, f64::min);
    let z_lo = (low - h).max(z_c - 12.0);
    let steps = ((z_top - z_lo) / h).ceil().max(0.0) as usize;
    let zs: Vec<f64> = (0..=steps)
        .map(|j| z_top - h * (steps - j) as f64)
        .collect();
    if vs.len() < 2 || zs.len() < 2 {
        return None;
    }
    let nz = zs.len();
    let n = nv * nz;
    let base = DVec3::new(c.x, c.y, 0.0);
    let p0 = |k: usize| base + vs[k / nz] * ev + zs[k % nz] * DVec3::Z;
    // below the top of the contact the gingival embrasure is filled where
    // narrower than `gingival`; above it the width shrinks back to `reach`
    let width: Vec<f64> = zs
        .iter()
        .map(|z| (wmax - SLOPE * (z - z_c)).clamp(p.reach, wmax))
        .collect();
    // where each line leaves A towards B (sa) and enters B (sb), along u
    let ends: Vec<(f64, f64)> = (0..n)
        .into_par_iter()
        .map(|k| {
            let o = p0(k);
            let sa = match a.hit(o, -u, u) {
                // outside A: its surface facing B
                Some((t, nu)) if nu > 0.0 => -t,
                // inside A: where the line leaves it
                Some(_) => a.hit(o, u, u).map_or(f64::NAN, |(t, _)| t),
                None => f64::NAN,
            };
            if !sa.is_finite() {
                return (f64::NAN, f64::NAN);
            }
            let sb = match b.hit(o, u, u) {
                // outside B: its surface facing A
                Some((t, nu)) if nu < 0.0 => t,
                Some(_) => b.hit(o, -u, u).map_or(f64::NAN, |(t, _)| -t),
                None => f64::NAN,
            };
            if sb.is_finite() {
                (sa, sb)
            } else {
                (f64::NAN, f64::NAN)
            }
        })
        .collect();
    let sa: Vec<f64> = ends.iter().map(|e| e.0).collect();
    let sb: Vec<f64> = ends.iter().map(|e| e.1).collect();
    let fin: Vec<bool> = (0..n).map(|k| sa[k].is_finite()).collect();
    // how narrow the space is at a line: the distance along it, or less
    // where the faces are oblique to the arch, the distance from where it
    // leaves a tooth to the other tooth (only on lines at most SLAB_MM long)
    let narrow: Vec<f64> = (0..n)
        .into_par_iter()
        .map(|k| {
            let d = sb[k] - sa[k];
            if d.is_finite() && d > width[k % nz] && d <= SLAB_MM {
                let o = p0(k);
                let da = b.surface(o + sa[k] * u, wmax);
                let db = a.surface(o + sb[k] * u, wmax);
                d.min(da.min(db))
            } else {
                d
            }
        })
        .collect();
    let mut ok: Vec<bool> = (0..n).map(|k| narrow[k] <= width[k % nz]).collect();
    // under the contact a window wider than `gingival` left between the
    // fill and the gingiva (an open black triangle) is blocked out: every
    // column of lines is filled on down from its highest filled line not
    // above the top of the contact, while its lines meet both teeth
    let mut ext = vec![false; n];
    for i in 0..nv {
        let row = i * nz;
        let Some(j1) = (0..nz).rev().find(|&j| ok[row + j] && zs[j] <= z_c) else {
            continue;
        };
        let mut run = true;
        for j in (0..nz).rev() {
            run = run && (j >= j1 || fin[row + j]);
            if j < j1 && run && !ok[row + j] {
                ext[row + j] = true;
            }
        }
    }
    for k in 0..n {
        ok[k] |= ext[k];
    }
    if p.inset > 0.0 && fin.iter().any(|f| *f) {
        // the labial and lingual faces `inset` inside the outermost lines
        // meeting both teeth at every height
        for j in 0..nz {
            let (lo, hi) = (0..nv)
                .filter(|&i| fin[i * nz + j])
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), i| {
                    (lo.min(vs[i]), hi.max(vs[i]))
                });
            for (i, v) in vs.iter().enumerate() {
                ok[i * nz + j] &= *v >= lo + p.inset && *v <= hi - p.inset;
            }
        }
    }
    if !ok.iter().any(|o| *o) {
        return None;
    }
    // a filled line ends one step inside each tooth; where the teeth overlap
    // a short segment around the middle keeps the solid whole
    let mid: Vec<f64> = (0..n).map(|k| 0.5 * (sa[k] + sb[k])).collect();
    let mut lo = vec![0.0f64; n];
    let mut hi = vec![0.0f64; n];
    for k in (0..n).filter(|&k| ok[k]) {
        if sb[k] - sa[k] > 0.0 {
            (lo[k], hi[k]) = (sa[k] - h, sb[k] + h);
        } else {
            (lo[k], hi[k]) = (mid[k] - h, mid[k] + h);
        }
    }
    let cg = Grid {
        n: nv - 1,
        m: nz - 1,
    };
    let nc = cg.n * cg.m;
    let corner = |k: usize| -> [usize; 4] {
        let (i, j) = (k / cg.m, k % cg.m);
        [
            i * nz + j,
            (i + 1) * nz + j,
            i * nz + j + 1,
            (i + 1) * nz + j + 1,
        ]
    };
    let ptp = |s: &dyn Fn(usize) -> f64, q: [usize; 4]| -> f64 {
        let v = q.map(s);
        let (mn, mx) = v
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                (a.min(*x), b.max(*x))
            });
        mx - mn
    };
    // both sheets continuous over a cell, no fins along the border
    let cells: Vec<bool> = (0..nc)
        .map(|k| {
            let q = corner(k);
            q.iter().all(|&c| ok[c])
                && ptp(&|c| lo[c], q) <= JUMP_STEPS * h
                && ptp(&|c| hi[c], q) <= JUMP_STEPS * h
        })
        .collect();
    let mut cells = cg.opening(&cells);
    // a window left in a column between two filled parts is a tunnel through
    // the model: filled at any width while its lines meet both teeth
    let fsa = |c: usize| if fin[c] { sa[c] } else { 0.0 };
    let fsb = |c: usize| if fin[c] { sb[c] } else { 0.0 };
    let fc: Vec<bool> = (0..nc)
        .map(|k| {
            let q = corner(k);
            q.iter().all(|&c| fin[c])
                && ptp(&fsa, q) <= WINDOW_JUMP_STEPS * h
                && ptp(&fsb, q) <= WINDOW_JUMP_STEPS * h
        })
        .collect();
    let mut win = vec![false; nc];
    for i in 0..cg.n {
        let js: Vec<usize> = (0..cg.m).filter(|&j| cells[cg.at(i, j)]).collect();
        for w in js.windows(2) {
            let (j0, j1) = (w[0], w[1]);
            if j1 - j0 > 1 && (j0 + 1..j1).all(|j| fc[cg.at(i, j)]) {
                for j in j0 + 1..j1 {
                    win[cg.at(i, j)] = true;
                }
            }
        }
    }
    // ... and pieces of fill apart along the arch are joined
    let both: Vec<bool> = (0..nc).map(|k| cells[k] || win[k]).collect();
    for (k, add) in connect_cells(&cg, &both, &fc).into_iter().enumerate() {
        win[k] |= add;
    }
    if win.iter().any(|w| *w) {
        let mut lw = vec![false; n];
        for k in (0..nc).filter(|&k| win[k]) {
            for c in corner(k) {
                lw[c] = true;
            }
        }
        for k in (0..n).filter(|&k| lw[k] && !ok[k]) {
            if sb[k] > sa[k] {
                (lo[k], hi[k]) = (sa[k] - h, sb[k] + h);
            } else {
                (lo[k], hi[k]) = (mid[k] - h, mid[k] + h);
            }
            ext[k] = true;
        }
        for k in 0..n {
            ok[k] |= lw[k];
        }
        for k in 0..nc {
            cells[k] |= win[k];
        }
    }
    // the border where the gap reaches the allowed width lies between two
    // lines: its nodes are moved onto it (linear interpolation)
    let exc: Vec<f64> = (0..n)
        .map(|k| {
            if ext[k] {
                -h
            } else {
                narrow[k] - width[k % nz]
            }
        })
        .collect();
    let x: Vec<[f64; 5]> = (0..n)
        .map(|k| {
            let q = p0(k);
            [q.x, q.y, q.z, lo[k], hi[k]]
        })
        .collect();
    let mut moved = x.clone();
    for k in 0..n {
        let (i, j) = (k / nz, k % nz);
        let mut acc = [0.0f64; 5];
        let mut cnt = 0usize;
        for (di, dj) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let (a2, b2) = (i as isize + di, j as isize + dj);
            if a2 < 0 || b2 < 0 || a2 as usize >= nv || b2 as usize >= nz {
                continue;
            }
            let m = a2 as usize * nz + b2 as usize;
            let cross = ok[k]
                && !ext[k]
                && exc[m] > 0.0
                && (sa[m] - sa[k]).abs() <= 10.0 * h
                && (sb[m] - sb[k]).abs() <= 10.0 * h;
            if !cross {
                continue;
            }
            let t = exc[k] / (exc[k] - exc[m]);
            let q = p0(m);
            let xm = [q.x, q.y, q.z, sa[m] - h, sb[m] + h];
            for c in 0..5 {
                acc[c] += t * (xm[c] - x[k][c]);
            }
            cnt += 1;
        }
        if cnt > 0 {
            for c in 0..5 {
                moved[k][c] += acc[c] / cnt as f64;
            }
        }
    }
    let solid = gap_solid(&moved, nv, nz, u, &cells)?;
    (signed_volume(&solid.0, &solid.1) >= 1e-3).then_some(solid)
}

/// The bridges of every two neighbouring teeth along the arch
/// (`contact_bridges`; `teeth` in any order, the arch is the one of the
/// first tooth), placed teeth in arch coordinates (`+Z` occlusal).
pub fn contact_bridges(teeth: &[Tooth<'_>], params: &BridgeParams) -> Vec<Solid> {
    bridges_by_pair(teeth, params)
        .into_iter()
        .map(|(_, _, s)| s)
        .collect()
}

/// [`contact_bridges`] with the two teeth of every bridge, in arch order.
pub fn bridges_by_pair(teeth: &[Tooth<'_>], params: &BridgeParams) -> Vec<(Fdi, Fdi, Solid)> {
    let Some(first) = teeth.first() else {
        return Vec::new();
    };
    let order: Vec<&Tooth<'_>> = arch_order(Arch::of(first.fdi))
        .iter()
        .filter_map(|f| teeth.iter().find(|t| t.fdi == *f))
        .collect();
    let prepared: Vec<(Fdi, Option<Prepared<'_>>)> = order
        .par_iter()
        .map(|t| (t.fdi, Prepared::new(t)))
        .collect();
    prepared
        .par_windows(2)
        .filter_map(|w| match (&w[0], &w[1]) {
            ((fa, Some(a)), (fb, Some(b))) => pair_bridge(a, b, params).map(|s| (*fa, *fb, s)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed box `[lo, hi]`, outward faces.
    fn cuboid(lo: DVec3, hi: DVec3) -> Solid {
        let v: Vec<DVec3> = (0..8)
            .map(|k| {
                DVec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                )
            })
            .collect();
        let t = vec![
            [0, 2, 1],
            [1, 2, 3],
            [4, 5, 6],
            [5, 7, 6],
            [0, 1, 4],
            [1, 5, 4],
            [2, 6, 3],
            [3, 6, 7],
            [0, 4, 2],
            [2, 4, 6],
            [1, 3, 5],
            [3, 7, 5],
        ];
        (v, t)
    }

    /// A box meshed from its distance field, so that its vertices are about
    /// `step` apart (the KD tree measures vertex to vertex, as in the
    /// plugin).
    fn fine_box(lo: DVec3, hi: DVec3, step: f64) -> Solid {
        let (v, t) = cuboid(lo, hi);
        let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t);
        let params = dc_geom::sdf::SdfParams {
            voxel_size: step as f32,
            band: (3.0 * step) as f32,
        };
        let mesh = dc_geom::sdf::SdfGrid::from_mesh(&mesh, &params)
            .unwrap()
            .to_mesh()
            .unwrap();
        (
            mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
            mesh.tris,
        )
    }

    fn volume(s: &Solid) -> f64 {
        signed_volume(&s.0, &s.1)
    }

    /// Two crowns 0.4 mm apart along the arch over their whole height: the
    /// slot between them is bridged where it is narrower than the allowed
    /// width, the bridge closed and inside the slot; 1.5 mm apart they do
    /// not touch.
    #[test]
    fn a_narrow_slot_between_two_crowns_is_bridged() {
        let f = |n: u8| Fdi::new(n).unwrap();
        let a = fine_box(
            DVec3::new(-5.0, -3.0, -6.0),
            DVec3::new(-0.2, 3.0, 0.0),
            0.25,
        );
        let b = fine_box(DVec3::new(0.2, -3.0, -6.0), DVec3::new(5.0, 3.0, 0.0), 0.25);
        let ta = Tooth {
            fdi: f(41),
            points: &a.0,
            tris: &a.1,
        };
        let tb = Tooth {
            fdi: f(42),
            points: &b.0,
            tris: &b.1,
        };
        let out = contact_bridges(&[tb, ta], &BridgeParams::default());
        assert_eq!(out.len(), 1);
        let s = &out[0];
        let mesh = TriMesh::new(s.0.iter().map(|p| p.as_vec3()).collect(), s.1.clone());
        let check = mesh.validate();
        assert!(check.is_watertight(), "{check:?}");
        assert!(volume(s) > 1.0, "{}", volume(s));
        let (lo, hi) = s.0.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        // one step into each crown across the slot, within the crowns
        assert!(lo.x <= -0.2 && hi.x >= 0.2, "{lo} {hi}");
        assert!(
            lo.x >= -0.2 - 2.0 * STEP_MM && hi.x <= 0.2 + 2.0 * STEP_MM,
            "{lo} {hi}"
        );
        assert!(lo.y >= -3.0 - 1e-6 && hi.y <= 3.0 + 1e-6, "{lo} {hi}");
        assert!(hi.z <= 0.0 + 1e-6, "{hi}");
        // apart: nothing
        let far = fine_box(DVec3::new(1.3, -3.0, -6.0), DVec3::new(6.0, 3.0, 0.0), 0.25);
        let tf = Tooth {
            fdi: f(42),
            points: &far.0,
            tris: &far.1,
        };
        assert!(contact_bridges(&[ta, tf], &BridgeParams::default()).is_empty());
    }

    #[test]
    fn the_model_touch_grows_with_the_voxel() {
        assert!((model_touch_mm(0.0) - TOUCH_MM).abs() < 1e-12);
        assert!((model_touch_mm(0.1) - 0.92).abs() < 1e-12);
        assert!(model_touch_mm(0.25) > model_touch_mm(0.1));
    }

    #[test]
    fn the_opening_drops_thin_parts() {
        let g = Grid { n: 5, m: 5 };
        let mut f = vec![false; 25];
        // a 3 x 3 block and a lone cell
        for i in 0..3 {
            for j in 0..3 {
                f[g.at(i, j)] = true;
            }
        }
        f[g.at(4, 4)] = true;
        let o = g.opening(&f);
        assert!(o[g.at(1, 1)] && o[g.at(0, 0)] && o[g.at(2, 2)]);
        assert!(!o[g.at(4, 4)]);
    }

    #[test]
    fn two_pieces_are_joined_through_the_free_cells() {
        let g = Grid { n: 3, m: 9 };
        let mut cells = vec![false; 27];
        for i in 0..3 {
            cells[g.at(i, 0)] = true;
            cells[g.at(i, 1)] = true;
            cells[g.at(i, 7)] = true;
        }
        let free = vec![true; 27];
        let add = connect_cells(&g, &cells, &free);
        // the way between the pieces, widened within the free cells
        for j in 2..7 {
            assert!((0..3).any(|i| add[g.at(i, j)]), "{j}");
        }
        assert!(!add.iter().zip(&cells).any(|(a, c)| *a && *c));
        // nothing to join without free cells between them
        let none = connect_cells(&g, &cells, &[false; 27]);
        assert!(!none.iter().any(|a| *a));
    }
}
