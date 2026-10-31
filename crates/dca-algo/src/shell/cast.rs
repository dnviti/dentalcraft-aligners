//! The gingiva of the model as a trimmed plaster cast on a flat base.
//!
//! Reference: `ortho_aligner/aligner.py` (`gum_cast`, `prism_faces`,
//! `gum_prisms` and the grid helpers `_grid_slope_limit`, `_grid_morph`,
//! `_grid_holes`, `_grid_cover`, `_box_min`).
//!
//! Arch coordinates (occlusal = `+Z`). The cast is a height field seen from
//! the occlusal side, sampled every `cell` mm, whose heights never rise
//! going away from the teeth (folds of lip and cheek and the border of the
//! scan never make walls in front of the gingiva), closed down to a flat
//! base. Near the teeth the exact scanned gingiva is given by vertical
//! prisms, one closed prism under every chosen face, reaching down into the
//! cast; their union with the cast and the teeth is the model.

use std::collections::VecDeque;

use dc_geom::bvh::Bvh;
use dc_geom::knn::KnnIndex;
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};

/// The cast sunk under the prisms rises back to the scanned height over this
/// many cells from their border (`GUM_RAMP_CELLS`).
pub const GUM_RAMP_CELLS: usize = 4;
/// Gaps of the top view up to about twice this many cells wide are filled
/// by the cast (`GUM_GAP_CELLS`).
pub const GUM_GAP_CELLS: usize = 2;
/// Under the prisms the cast rises at most this (mm per mm of plan) above a
/// node beside it (`GUM_CLIFF_SLOPE`).
pub const GUM_CLIFF_SLOPE: f64 = 1.0;
/// Faces within this (plan, mm) of the crowns become prisms (`near_mm`).
pub const PRISM_NEAR_MM: f64 = 4.5;
/// A face more than this above the highest cast around is a fold (`tol`).
pub const PRISM_TOL_MM: f64 = 0.5;
/// ... unless no scan lies within this under it (`below_mm`).
pub const PRISM_BELOW_MM: f64 = 3.0;
/// A prism reaches at least this far below its face (`depth`).
pub const PRISM_DEPTH_MM: f64 = 1.0;

/// Grid of nodes `lo + cell * (i, j)`, `i < nx`, `j < ny`, row-major in `i`.
#[derive(Debug, Clone)]
pub struct CastGrid {
    /// Plan position of node `(0, 0)` (x, y) and the lowest z of the input.
    pub lo: DVec3,
    /// Node spacing (mm).
    pub cell: f64,
    /// Nodes along x.
    pub nx: usize,
    /// Nodes along y.
    pub ny: usize,
}

impl CastGrid {
    fn idx(&self, i: usize, j: usize) -> usize {
        i * self.ny + j
    }

    fn len(&self) -> usize {
        self.nx * self.ny
    }

    fn x(&self, i: usize) -> f64 {
        self.lo.x + self.cell * i as f64
    }

    fn y(&self, j: usize) -> f64 {
        self.lo.y + self.cell * j as f64
    }

    /// Nearest node of a plan position (clamped to the grid).
    fn node(&self, x: f64, y: f64) -> (usize, usize) {
        let i = ((x - self.lo.x) / self.cell).round();
        let j = ((y - self.lo.y) / self.cell).round();
        (
            (i.max(0.0) as usize).min(self.nx - 1),
            (j.max(0.0) as usize).min(self.ny - 1),
        )
    }

    /// The 8 neighbours of a node inside the grid, with their offsets.
    fn neighbours(&self, i: usize, j: usize) -> impl Iterator<Item = (usize, usize, bool)> + '_ {
        const OFF: [(i64, i64); 8] = [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ];
        OFF.iter().filter_map(move |&(a, b)| {
            let (p, q) = (i as i64 + a, j as i64 + b);
            (p >= 0 && q >= 0 && (p as usize) < self.nx && (q as usize) < self.ny).then_some((
                p as usize,
                q as usize,
                a != 0 && b != 0,
            ))
        })
    }

    /// Max (or min) of `v` over the 3 x 3 box of every node; beyond the grid
    /// counts as `fill`.
    fn box3(&self, v: &[f64], max: bool) -> Vec<f64> {
        let mut out = vec![0.0; self.len()];
        for i in 0..self.nx {
            for j in 0..self.ny {
                let mut m = v[self.idx(i, j)];
                for (p, q, _) in self.neighbours(i, j) {
                    let w = v[self.idx(p, q)];
                    m = if max { m.max(w) } else { m.min(w) };
                }
                out[self.idx(i, j)] = m;
            }
        }
        out
    }

    /// Dilation (erosion: `erode`) of a boolean grid by a `(2r + 1)` square;
    /// beyond the grid counts as `erode` (`_grid_morph`).
    fn morph(&self, m: &[bool], r: usize, erode: bool) -> Vec<bool> {
        let (n, k) = (self.nx as i64, self.ny as i64);
        let r = r as i64;
        let mut out = vec![erode; self.len()];
        for i in 0..n {
            for j in 0..k {
                let mut acc = erode;
                'win: for a in -r..=r {
                    for b in -r..=r {
                        let (p, q) = (i + a, j + b);
                        let v = if p < 0 || q < 0 || p >= n || q >= k {
                            erode
                        } else {
                            m[(p * k + q) as usize]
                        };
                        if erode && !v {
                            acc = false;
                            break 'win;
                        }
                        if !erode && v {
                            acc = true;
                            break 'win;
                        }
                    }
                }
                out[(i * k + j) as usize] = acc;
            }
        }
        out
    }

    /// Cells `false` in `m` not 4-connected to the border of the grid
    /// (`_grid_holes`).
    fn holes(&self, m: &[bool]) -> Vec<bool> {
        let mut reached = vec![false; self.len()];
        let mut queue = VecDeque::new();
        for i in 0..self.nx {
            for j in 0..self.ny {
                if (i == 0 || j == 0 || i + 1 == self.nx || j + 1 == self.ny) && !m[self.idx(i, j)]
                {
                    reached[self.idx(i, j)] = true;
                    queue.push_back((i, j));
                }
            }
        }
        while let Some((i, j)) = queue.pop_front() {
            let cand = [
                (i.wrapping_sub(1), j),
                (i + 1, j),
                (i, j.wrapping_sub(1)),
                (i, j + 1),
            ];
            for (p, q) in cand {
                if p < self.nx && q < self.ny {
                    let k = self.idx(p, q);
                    if !m[k] && !reached[k] {
                        reached[k] = true;
                        queue.push_back((p, q));
                    }
                }
            }
        }
        (0..self.len()).map(|k| !m[k] && !reached[k]).collect()
    }

    /// Top view of triangles sampled about cell / 3 apart: (nearest nodes,
    /// cells containing a sample) (`_grid_cover`).
    fn cover(&self, tris: &[[DVec3; 3]]) -> (Vec<bool>, Vec<bool>) {
        let (nx, ny) = (self.nx, self.ny);
        let mut nodes = vec![false; nx * ny];
        let mut cells = vec![false; (nx - 1) * (ny - 1)];
        for t in tris {
            let q: [[f64; 2]; 3] =
                t.map(|p| [(p.x - self.lo.x) / self.cell, (p.y - self.lo.y) / self.cell]);
            let edge = |a: usize, b: usize| (q[a][0] - q[b][0]).hypot(q[a][1] - q[b][1]);
            let longest = edge(0, 1).max(edge(1, 2)).max(edge(2, 0));
            let kk = (3.0 * longest).ceil().max(1.0) as usize;
            for ai in 0..=kk {
                for bi in 0..=(kk - ai) {
                    let (a, b) = (ai as f64 / kk as f64, bi as f64 / kk as f64);
                    let w = 1.0 - a - b;
                    let s0 = q[0][0] * w + q[1][0] * a + q[2][0] * b;
                    let s1 = q[0][1] * w + q[1][1] * a + q[2][1] * b;
                    let ni = (s0.round().max(0.0) as usize).min(nx - 1);
                    let nj = (s1.round().max(0.0) as usize).min(ny - 1);
                    nodes[ni * ny + nj] = true;
                    let ci = (s0.floor().max(0.0) as usize).min(nx - 2);
                    let cj = (s1.floor().max(0.0) as usize).min(ny - 2);
                    cells[ci * (ny - 1) + cj] = true;
                }
            }
        }
        (nodes, cells)
    }
}

/// Sizes of the cast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CastParams {
    /// The flat base lies this far below the lowest gingiva (mm).
    pub base_height: f64,
    /// Node spacing of the height field (mm).
    pub cell: f64,
    /// The cast is lowered this much under the prisms and the crowns (mm).
    pub sink: f64,
}

impl Default for CastParams {
    fn default() -> Self {
        Self {
            base_height: 3.0,
            cell: 0.25,
            sink: 0.3,
        }
    }
}

/// What [`gum_cast`] leaves for [`gum_prisms`] (the `info` of the plugin).
#[derive(Debug, Clone)]
pub struct CastInfo {
    /// The node grid.
    pub grid: CastGrid,
    /// Highest allowed height around every node (3 x 3), `-inf` where none.
    pub hmax: Vec<f64>,
    /// Plan distance of every node from the crowns.
    pub dist: Vec<f64>,
    /// Height of the flat base.
    pub z_base: f64,
    /// Lowest top of the cast around every node (3 x 3), `inf` where none.
    pub hmin: Vec<f64>,
    /// The faces chosen for the prisms (`info['faces']`).
    pub faces: Vec<[u32; 3]>,
}

/// The cast: a closed mesh (arch coordinates) and what the prisms need.
#[derive(Debug, Clone)]
pub struct Cast {
    /// Positions of the closed cast.
    pub positions: Vec<DVec3>,
    /// Triangles of the closed cast (outward).
    pub tris: Vec<[u32; 3]>,
    /// The grid data for [`gum_prisms`].
    pub info: CastInfo,
}

fn tri_points(v: &[DVec3], t: &[u32; 3]) -> [DVec3; 3] {
    [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]]
}

/// Faces of `tris` that become prisms of the exact gingiva (`prism_faces`):
/// within [`PRISM_NEAR_MM`] (plan) of the crowns, not folds above the cast
/// (more scan within [`PRISM_BELOW_MM`] under them); a face reaching below
/// the flat base is left out.
fn prism_faces(
    v: &[DVec3],
    tris: &[[u32; 3]],
    bvh: Option<&Bvh>,
    info: &CastInfo,
) -> Vec<[u32; 3]> {
    let g = &info.grid;
    let mut keep = vec![false; tris.len()];
    let mut cand = Vec::new();
    for (f, t) in tris.iter().enumerate() {
        let p = tri_points(v, t);
        let c = (p[0] + p[1] + p[2]) / 3.0;
        let (i, j) = g.node(c.x, c.y);
        let k = g.idx(i, j);
        if info.dist[k] > PRISM_NEAR_MM {
            continue;
        }
        if c.z <= info.hmax[k] + PRISM_TOL_MM {
            keep[f] = true;
            continue;
        }
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n.z.abs() > 0.3 {
            cand.push((f, c));
        }
    }
    if let Some(bvh) = bvh {
        let origins: Vec<Vec3> = cand
            .iter()
            .map(|(_, c)| (*c - DVec3::new(0.0, 0.0, 0.02)).as_vec3())
            .collect();
        let dirs = vec![Vec3::NEG_Z; origins.len()];
        for ((f, _), hit) in cand
            .iter()
            .zip(bvh.ray_casts(&origins, &dirs, PRISM_BELOW_MM as f32))
        {
            if hit.is_none() {
                keep[*f] = true;
            }
        }
    }
    tris.iter()
        .zip(&keep)
        .filter(|(t, k)| {
            **k && tri_points(v, t)
                .iter()
                .map(|p| p.z)
                .fold(f64::INFINITY, f64::min)
                > info.z_base
        })
        .map(|(t, _)| *t)
        .collect()
}

/// The grid `z` lowered on `mask` until no node stands more than `step` per
/// cell of plan distance above a neighbour (`_grid_slope_limit`).
fn slope_limit(g: &CastGrid, z: &[f64], mask: &[bool], step: f64) -> Vec<f64> {
    let mut z = z.to_vec();
    if !mask.iter().any(|m| *m) {
        return z;
    }
    let diag = step * 2f64.sqrt();
    for _ in 0..(g.nx + g.ny) {
        let mut nb = vec![f64::INFINITY; g.len()];
        for i in 0..g.nx {
            for j in 0..g.ny {
                let mut m = f64::INFINITY;
                for (p, q, d) in g.neighbours(i, j) {
                    m = m.min(z[g.idx(p, q)] + if d { diag } else { step });
                }
                nb[g.idx(i, j)] = m;
            }
        }
        let mut changed = false;
        for k in 0..g.len() {
            if mask[k] && nb[k] < z[k] {
                z[k] = nb[k];
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    z
}

/// Closed solid of the gingiva like a trimmed plaster cast (`gum_cast`): the
/// surface of `(v, tris)` seen from `+Z`, sampled every `cell` mm, heights
/// non-increasing away from the crowns (`crown_xy`), down to a flat base
/// `base_height` below its lowest point, lowered by `sink` under the prisms
/// and the crowns. `prisms`: the faces offered to the prisms (the chosen
/// ones end in [`CastInfo::faces`]); `cover`: faces under the teeth.
pub fn gum_cast(
    v: &[DVec3],
    tris: &[[u32; 3]],
    crown_xy: &[[f64; 2]],
    params: &CastParams,
    prisms: &[[u32; 3]],
    cover: &[[u32; 3]],
) -> Cast {
    let CastParams {
        base_height,
        cell,
        sink,
    } = *params;
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in v {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    lo -= DVec3::ONE;
    hi += DVec3::ONE;
    let nx = ((hi.x - lo.x) / cell).ceil() as usize + 1;
    let ny = ((hi.y - lo.y) / cell).ceil() as usize + 1;
    let g = CastGrid { lo, cell, nx, ny };
    let z0 = hi.z + 5.0;
    let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), tris.to_vec());
    let bvh = Bvh::build(&mesh).ok();

    // the surface seen from above
    let mut h = vec![f64::NAN; g.len()];
    if let Some(bvh) = &bvh {
        let mut origins = Vec::with_capacity(g.len());
        for i in 0..nx {
            for j in 0..ny {
                origins.push(Vec3::new(g.x(i) as f32, g.y(j) as f32, z0 as f32));
            }
        }
        let dirs = vec![Vec3::NEG_Z; origins.len()];
        for (k, hit) in bvh
            .ray_casts(&origins, &dirs, f32::INFINITY)
            .into_iter()
            .enumerate()
        {
            if let Some(hit) = hit {
                h[k] = z0 - f64::from(hit.t);
            }
        }
    }
    // plan distance from the crowns, then heights non-increasing away from them
    let crown_pts: Vec<Vec3> = crown_xy
        .iter()
        .map(|c| Vec3::new(c[0] as f32, c[1] as f32, 0.0))
        .collect();
    let mut dist = vec![f64::INFINITY; g.len()];
    if let Ok(index) = KnnIndex::build(&crown_pts) {
        let queries: Vec<Vec3> = (0..g.len())
            .map(|k| Vec3::new(g.x(k / ny) as f32, g.y(k % ny) as f32, 0.0))
            .collect();
        for (k, n) in index.nearest_batch(&queries).into_iter().enumerate() {
            if let Some(n) = n {
                dist[k] = f64::from(n.distance);
            }
        }
    }
    let mut valid: Vec<bool> = h.iter().map(|z| !z.is_nan()).collect();
    let mut order: Vec<usize> = (0..g.len()).collect();
    order.sort_by(|&a, &b| dist[a].total_cmp(&dist[b]).then(a.cmp(&b)));
    let mut hm = h.clone();
    for &k in &order {
        let (i, j) = (k / ny, k % ny);
        if !valid[k] || dist[k] <= 2.0 * cell {
            continue;
        }
        let mut best = f64::NEG_INFINITY;
        for (p, q, _) in g.neighbours(i, j) {
            let n = g.idx(p, q);
            if valid[n] && dist[n] < dist[k] {
                best = best.max(hm[n]);
            }
        }
        if best > f64::NEG_INFINITY {
            hm[k] = hm[k].min(best);
        }
    }
    let lowest = hm
        .iter()
        .filter(|z| !z.is_nan())
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let z_base = lowest - base_height;
    let hn: Vec<f64> = (0..g.len())
        .map(|k| if valid[k] { hm[k] } else { f64::NEG_INFINITY })
        .collect();
    let hmax = g.box3(&hn, true);
    let mut info = CastInfo {
        grid: g.clone(),
        hmax,
        dist: dist.clone(),
        z_base,
        hmin: Vec::new(),
        faces: Vec::new(),
    };
    let near: Vec<bool> = dist.iter().map(|d| *d <= 2.0 * cell).collect();
    let mut sunk = vec![sink; g.len()];
    if !prisms.is_empty() {
        let faces = prism_faces(v, prisms, bvh.as_ref(), &info);
        let pts: Vec<[DVec3; 3]> = faces.iter().map(|t| tri_points(v, t)).collect();
        info.faces = faces;
        let (nodes, cells) = g.cover(&pts);
        let mut need = valid.clone();
        for ci in 0..nx - 1 {
            for cj in 0..ny - 1 {
                if cells[ci * (ny - 1) + cj] {
                    for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        need[g.idx(ci + di, cj + dj)] = true;
                    }
                }
            }
        }
        let mut dom = g.morph(&g.morph(&need, GUM_GAP_CELLS, false), GUM_GAP_CELLS, true);
        let holes = g.holes(&dom);
        for (d, hole) in dom.iter_mut().zip(holes) {
            *d |= hole;
        }
        // no face seen from above: the lowest height around, spread from the scanned nodes
        let mut hf: Vec<f64> = (0..g.len())
            .map(|k| if valid[k] { hm[k] } else { f64::INFINITY })
            .collect();
        let mut todo: Vec<bool> = (0..g.len()).map(|k| dom[k] && !valid[k]).collect();
        while todo.iter().any(|t| *t) {
            let nb = g.box3(&hf, false);
            let mut got = false;
            for k in 0..g.len() {
                if todo[k] && nb[k].is_finite() {
                    hf[k] = nb[k];
                    todo[k] = false;
                    got = true;
                }
            }
            if !got {
                break;
            }
        }
        valid = hf.iter().map(|z| z.is_finite()).collect();
        hm = (0..g.len())
            .map(|k| if valid[k] { hf[k] } else { f64::NAN })
            .collect();
        // sunk under the prisms and the teeth only, rising to 0 over GUM_RAMP_CELLS cells
        let mut inner: Vec<bool> = (0..g.len()).map(|k| nodes[k] || near[k]).collect();
        if !cover.is_empty() {
            let cpts: Vec<[DVec3; 3]> = cover.iter().map(|t| tri_points(v, t)).collect();
            for (a, b) in inner.iter_mut().zip(g.cover(&cpts).0) {
                *a |= b;
            }
        }
        let mut layers = vec![0.0f64; g.len()];
        for _ in 0..=GUM_RAMP_CELLS {
            for (l, i) in layers.iter_mut().zip(&inner) {
                *l += f64::from(u8::from(*i));
            }
            inner = g.morph(&inner, 1, true);
        }
        sunk = layers
            .iter()
            .map(|l| sink * ((l - 1.0) / GUM_RAMP_CELLS as f64).clamp(0.0, 1.0))
            .collect();
        // where the prisms cover it the cast rises at most GUM_CLIFF_SLOPE above a node beside it
        let hide: Vec<bool> = (0..g.len()).map(|k| nodes[k] && valid[k]).collect();
        if hide.iter().any(|x| *x) {
            let top: Vec<f64> = (0..g.len())
                .map(|k| {
                    if valid[k] {
                        hm[k] - sunk[k]
                    } else {
                        f64::INFINITY
                    }
                })
                .collect();
            let low = slope_limit(&g, &top, &hide, GUM_CLIFF_SLOPE * cell);
            for k in 0..g.len() {
                if hide[k] {
                    sunk[k] = hm[k] - low[k];
                }
            }
        }
    }
    // quads of four valid nodes; top, bottom and the walls along their border
    let mut idx = vec![u32::MAX; g.len()];
    let mut quads = Vec::new();
    for i in 0..nx - 1 {
        for j in 0..ny - 1 {
            let c = [
                g.idx(i, j),
                g.idx(i + 1, j),
                g.idx(i + 1, j + 1),
                g.idx(i, j + 1),
            ];
            if c.iter().all(|&k| valid[k]) {
                quads.push(c);
            }
        }
    }
    let mut top = Vec::new();
    for q in &quads {
        for &k in q {
            if idx[k] == u32::MAX {
                idx[k] = top.len() as u32;
                top.push(k);
            }
        }
    }
    let n = top.len() as u32;
    let mut positions: Vec<DVec3> = top
        .iter()
        .map(|&k| DVec3::new(g.x(k / ny), g.y(k % ny), hm[k] - sunk[k]))
        .collect();
    positions.extend(
        top.iter()
            .map(|&k| DVec3::new(g.x(k / ny), g.y(k % ny), z_base)),
    );
    let mut out_tris = Vec::with_capacity(quads.len() * 4);
    let mut edges: std::collections::HashMap<(u32, u32), (u32, u32, u32)> =
        std::collections::HashMap::new();
    for q in &quads {
        let [a, b, c, d] = q.map(|k| idx[k]);
        out_tris.push([a, b, c]);
        out_tris.push([a, c, d]);
        out_tris.push([a + n, c + n, b + n]);
        out_tris.push([a + n, d + n, c + n]);
        for (s, e) in [(a, b), (b, c), (c, d), (d, a)] {
            let key = (s.min(e), s.max(e));
            edges.entry(key).or_insert((s, e, 0)).2 += 1;
        }
    }
    let mut border: Vec<(u32, u32)> = edges
        .values()
        .filter(|(_, _, cnt)| *cnt == 1)
        .map(|(s, e, _)| (*s, *e))
        .collect();
    border.sort_unstable();
    for (s, e) in border {
        out_tris.push([e, s, s + n]);
        out_tris.push([e, s + n, e + n]);
    }
    if signed_volume(&positions, &out_tris) < 0.0 {
        for t in &mut out_tris {
            t.swap(1, 2);
        }
    }
    // lowest top of the cast around every node (where prisms must reach down to)
    let topz: Vec<f64> = (0..g.len())
        .map(|k| {
            if valid[k] {
                hm[k] - sunk[k]
            } else {
                f64::INFINITY
            }
        })
        .collect();
    info.hmin = g.box3(&topz, false);
    Cast {
        positions,
        tris: out_tris,
        info,
    }
}

/// Signed volume of a triangle mesh (positive for outward faces).
pub fn signed_volume(v: &[DVec3], tris: &[[u32; 3]]) -> f64 {
    tris.iter()
        .map(|t| {
            let [a, b, c] = tri_points(v, t);
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        / 6.0
}

/// Exact gingiva near the teeth (`gum_prisms`): every face chosen by
/// [`gum_cast`] becomes a closed vertical prism reaching down into the cast
/// under the whole face (at least [`PRISM_DEPTH_MM`]), or to the flat base
/// where no cast is under it.
pub fn gum_prisms(v: &[DVec3], info: &CastInfo) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    let g = &info.grid;
    let m = info.faces.len();
    let mut top = Vec::with_capacity(3 * m);
    let mut floor = Vec::with_capacity(3 * m);
    for t in &info.faces {
        let [a, mut b, mut c] = tri_points(v, t);
        if (b - a).cross(c - a).z < 0.0 {
            std::mem::swap(&mut b, &mut c);
        }
        let nodes = [a, b, c].map(|p| g.node(p.x, p.y));
        let (i0, i1) = (
            nodes.iter().map(|n| n.0).min().unwrap_or(0),
            nodes.iter().map(|n| n.0).max().unwrap_or(0),
        );
        let (j0, j1) = (
            nodes.iter().map(|n| n.1).min().unwrap_or(0),
            nodes.iter().map(|n| n.1).max().unwrap_or(0),
        );
        let mut hc = f64::INFINITY;
        for i in i0..=i1 {
            for j in j0..=j1 {
                hc = hc.min(info.hmin[g.idx(i, j)]);
            }
        }
        for p in [a, b, c] {
            let f = p.z - PRISM_DEPTH_MM;
            let f = if hc.is_finite() {
                f.min(hc - 0.1)
            } else {
                info.z_base
            };
            top.push(p);
            floor.push(f.max(info.z_base));
        }
    }
    let mut positions = top.clone();
    positions.extend(
        top.iter()
            .zip(&floor)
            .map(|(p, f)| DVec3::new(p.x, p.y, *f)),
    );
    let off = (3 * m) as u32;
    let mut tris = Vec::with_capacity(8 * m);
    for k in 0..m as u32 {
        let (a, b, c) = (3 * k, 3 * k + 1, 3 * k + 2);
        let (ab, bb, cb) = (a + off, b + off, c + off);
        tris.extend_from_slice(&[
            [a, b, c],
            [ab, cb, bb],
            [a, ab, bb],
            [a, bb, b],
            [b, bb, cb],
            [b, cb, c],
            [c, cb, ab],
            [c, ab, a],
        ]);
    }
    (positions, tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An open, wavy sheet of gingiva (z = f(x, y)) around a gap.
    fn sheet() -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let n = 40;
        let mut v = Vec::new();
        for i in 0..=n {
            for j in 0..=n {
                let (x, y) = (i as f64 * 0.5 - 10.0, j as f64 * 0.5 - 10.0);
                v.push(DVec3::new(
                    x,
                    y,
                    2.0 - 0.02 * (x * x + y * y) + 0.3 * (x * 0.7).sin(),
                ));
            }
        }
        let mut t = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let a = (i * (n + 1) + j) as u32;
                let (b, c, d) = (a + (n + 1) as u32, a + (n + 2) as u32, a + 1);
                t.push([a, b, c]);
                t.push([a, c, d]);
            }
        }
        (v, t)
    }

    #[test]
    fn cast_is_closed_and_flat_below() {
        let (v, t) = sheet();
        let crowns = [[0.0, 0.0], [1.0, 0.5], [-1.0, 0.5]];
        let cast = gum_cast(&v, &t, &crowns, &CastParams::default(), &t, &[]);
        let mesh = TriMesh::new(
            cast.positions.iter().map(|p| p.as_vec3()).collect(),
            cast.tris.clone(),
        );
        let rep = mesh.validate();
        assert!(rep.is_watertight(), "{rep:?}");
        assert!(signed_volume(&cast.positions, &cast.tris) > 0.0);
        let zmin = cast
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f64::INFINITY, f64::min);
        assert!((zmin - cast.info.z_base).abs() < 1e-9);
        assert!(!cast.info.faces.is_empty());
        let (pv, pt) = gum_prisms(&v, &cast.info);
        assert_eq!(pt.len(), 8 * cast.info.faces.len());
        let prisms = TriMesh::new(pv.iter().map(|p| p.as_vec3()).collect(), pt);
        assert_eq!(prisms.validate().boundary_edges, 0);
    }

    #[test]
    fn heights_never_rise_away_from_the_teeth() {
        // a wall of "cheek" rising far from the crowns is flattened
        let (mut v, t) = sheet();
        for p in &mut v {
            if p.x > 7.0 {
                p.z += 5.0;
            }
        }
        let cast = gum_cast(&v, &t, &[[0.0, 0.0]], &CastParams::default(), &[], &[]);
        let ztop = cast
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(ztop < 2.5, "{ztop}");
    }
}
