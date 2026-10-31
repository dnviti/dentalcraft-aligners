//! Height grids of the trim surface and their filters (`aligner.trim_cutter`
//! grid, `_grid_filter`, `smooth_above`, `smooth_within`, `cone_above`,
//! `despike`, `_slab_solid`).

use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};

use super::survey::median;

/// A regular plan grid, node `(i, j)` at `lo + (i dx, j dy)`, stored at
/// `i * ny + j` (the layout of the plugin's `meshgrid(..., indexing="ij")`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanGrid {
    /// First node.
    pub lo: DVec2,
    /// Last node.
    pub hi: DVec2,
    /// Nodes along x.
    pub nx: usize,
    /// Nodes along y.
    pub ny: usize,
}

impl PlanGrid {
    /// The grid of `trim_cutter`: the plan bounds grown by 2 mm, at most
    /// `cell` between nodes (`linspace` with `ceil(extent / cell) + 1`
    /// nodes).
    pub fn around(lo: DVec3, hi: DVec3, cell: f64) -> Self {
        let lo = lo.truncate() - DVec2::splat(2.0);
        let hi = hi.truncate() + DVec2::splat(2.0);
        let nx = ((hi.x - lo.x) / cell).ceil() as usize + 1;
        let ny = ((hi.y - lo.y) / cell).ceil() as usize + 1;
        Self { lo, hi, nx, ny }
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nx * self.ny
    }

    /// `true` for a grid without nodes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn coord(lo: f64, hi: f64, n: usize, i: usize) -> f64 {
        if n <= 1 {
            lo
        } else {
            // numpy linspace: lo + i * step, the last one exactly hi
            let step = (hi - lo) / (n - 1) as f64;
            if i == n - 1 { hi } else { lo + i as f64 * step }
        }
    }

    /// Plan position of node `k`.
    pub fn node(&self, k: usize) -> DVec2 {
        let (i, j) = (k / self.ny, k % self.ny);
        DVec2::new(
            Self::coord(self.lo.x, self.hi.x, self.nx, i),
            Self::coord(self.lo.y, self.hi.y, self.ny, j),
        )
    }

    /// Spacing along x.
    pub fn dx(&self) -> f64 {
        if self.nx > 1 {
            (self.hi.x - self.lo.x) / (self.nx - 1) as f64
        } else {
            0.0
        }
    }

    /// Plan positions of every node.
    pub fn nodes(&self) -> Vec<DVec2> {
        (0..self.len()).map(|k| self.node(k)).collect()
    }
}

/// Largest value (`max`) or sum over the `(2r+1)^2` nodes around every
/// node, the border repeated (`_grid_filter`).
pub fn filter(z: &[f64], nx: usize, ny: usize, r: usize, sum: bool) -> Vec<f64> {
    let r = r as isize;
    let at = |i: isize, j: isize| {
        let i = i.clamp(0, nx as isize - 1) as usize;
        let j = j.clamp(0, ny as isize - 1) as usize;
        z[i * ny + j]
    };
    let mut out = vec![0.0; z.len()];
    for i in 0..nx as isize {
        for j in 0..ny as isize {
            let mut acc = if sum { 0.0 } else { f64::NEG_INFINITY };
            let mut nan = false;
            for di in -r..=r {
                for dj in -r..=r {
                    let v = at(i + di, j + dj);
                    nan |= v.is_nan();
                    acc = if sum { acc + v } else { acc.max(v) };
                }
            }
            out[i as usize * ny + j as usize] = if nan { f64::NAN } else { acc };
        }
    }
    out
}

/// Smooth version of a grid of heights that is nowhere below it
/// (`smooth_above`): the largest value within `r` cells, averaged over the
/// same window.
pub fn smooth_above(z: &[f64], nx: usize, ny: usize, r: usize) -> Vec<f64> {
    let m = filter(z, nx, ny, r, false);
    let w = ((2 * r + 1) * (2 * r + 1)) as f64;
    filter(&m, nx, ny, r, true)
        .into_iter()
        .map(|v| v / w)
        .collect()
}

/// Grid of heights smoothed by `iters` passes of the 3x3 binomial filter,
/// every pass kept within `lo <= z <= hi` (where `lo > hi`, `lo`); nodes not
/// finite stay as they are and are left out of the means (`smooth_within`).
pub fn smooth_within(
    z: &[f64],
    lo: &[f64],
    hi: &[f64],
    nx: usize,
    ny: usize,
    iters: usize,
) -> Vec<f64> {
    let hi: Vec<f64> = hi.iter().zip(lo).map(|(&h, &l)| np_max(h, l)).collect();
    let ok: Vec<bool> = z.iter().map(|v| v.is_finite()).collect();
    if iters == 0 || !ok.iter().any(|&o| o) {
        return z.to_vec();
    }
    let w = [1.0, 2.0, 1.0];
    let at = |v: &[f64], i: isize, j: isize| {
        let i = i.clamp(0, nx as isize - 1) as usize;
        let j = j.clamp(0, ny as isize - 1) as usize;
        v[i * ny + j]
    };
    let okf: Vec<f64> = ok.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect();
    let mut wsum = vec![0.0; z.len()];
    for i in 0..nx as isize {
        for j in 0..ny as isize {
            let mut s = 0.0;
            for a in 0..3 {
                for b in 0..3 {
                    s += w[a] * w[b] * at(&okf, i + a as isize - 1, j + b as isize - 1);
                }
            }
            wsum[i as usize * ny + j as usize] = s;
        }
    }
    let clip = |v: f64, k: usize| np_min(np_max(v, lo[k]), hi[k]);
    let mut out: Vec<f64> = (0..z.len())
        .map(|k| if ok[k] { clip(z[k], k) } else { z[k] })
        .collect();
    for _ in 0..iters {
        let src: Vec<f64> = (0..z.len())
            .map(|k| if ok[k] { out[k] } else { 0.0 })
            .collect();
        let mut next = out.clone();
        for i in 0..nx as isize {
            for j in 0..ny as isize {
                let k = i as usize * ny + j as usize;
                if !ok[k] {
                    continue;
                }
                let mut s = 0.0;
                for a in 0..3 {
                    for b in 0..3 {
                        s += w[a] * w[b] * at(&src, i + a as isize - 1, j + b as isize - 1);
                    }
                }
                next[k] = clip(s / wsum[k], k);
            }
        }
        out = next;
    }
    out
}

/// Grid raised to the cones going down from every node: over the nodes
/// within `reach` cells, their height less `drop` per cell (`cone_above`).
pub fn cone_above(z: &[f64], nx: usize, ny: usize, drop: f64, reach: usize) -> Vec<f64> {
    let mut out = z.to_vec();
    let diag = 2f64.sqrt() * drop;
    for _ in 0..reach {
        let prev = out.clone();
        for i in 0..nx as isize {
            for j in 0..ny as isize {
                let k = i as usize * ny + j as usize;
                let mut m = f64::NEG_INFINITY;
                for a in -1..=1isize {
                    for b in -1..=1isize {
                        if a == 0 && b == 0 {
                            continue;
                        }
                        let (ii, jj) = (i + a, j + b);
                        if ii < 0 || jj < 0 || ii >= nx as isize || jj >= ny as isize {
                            continue;
                        }
                        let d = if a != 0 && b != 0 { diag } else { drop };
                        m = np_max(m, prev[ii as usize * ny + jj as usize] - d);
                    }
                }
                out[k] = np_max(prev[k], m);
            }
        }
    }
    out
}

/// Grid with its downward spikes removed (`despike`): a node lower than at
/// least `low` of its 8 neighbours by more than `t` is raised to their
/// median (twice); `where_` limits it to some nodes.
pub fn despike(z: &[f64], nx: usize, ny: usize, t: f64, low: usize, where_: &[bool]) -> Vec<f64> {
    let mut z = z.to_vec();
    for _ in 0..2 {
        let prev = z.clone();
        for i in 0..nx as isize {
            for j in 0..ny as isize {
                let k = i as usize * ny + j as usize;
                if !prev[k].is_finite() || !where_[k] {
                    continue;
                }
                let mut nb = Vec::with_capacity(8);
                for a in -1..=1isize {
                    for b in -1..=1isize {
                        if a == 0 && b == 0 {
                            continue;
                        }
                        let ii = (i + a).clamp(0, nx as isize - 1) as usize;
                        let jj = (j + b).clamp(0, ny as isize - 1) as usize;
                        nb.push(prev[ii * ny + jj]);
                    }
                }
                let above = nb.iter().filter(|&&v| v > prev[k] + t).count();
                if above >= low {
                    let med = if nb.iter().any(|v| v.is_nan()) {
                        f64::NAN
                    } else {
                        median(&mut nb)
                    };
                    if med.is_finite() {
                        z[k] = prev[k].max(med);
                    }
                }
            }
        }
    }
    z
}

/// `np.maximum`: NaN when either is NaN.
pub(crate) fn np_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `np.minimum`: NaN when either is NaN.
pub(crate) fn np_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// Closed solid between the heights `zb` (bottom) and `zt` (top) of the
/// grid nodes (`_slab_solid`), `zb < zt` everywhere; outward faces.
pub fn slab_solid(grid: &PlanGrid, zb: &[f64], zt: f64) -> TriMesh {
    let (nx, ny) = (grid.nx, grid.ny);
    let off = (nx * ny) as u32;
    let mut pos: Vec<Vec3> = Vec::with_capacity(2 * nx * ny);
    for (k, &z) in zb.iter().enumerate() {
        pos.push(grid.node(k).extend(z).as_vec3());
    }
    for k in 0..nx * ny {
        pos.push(grid.node(k).extend(zt).as_vec3());
    }
    let mut tris = Vec::with_capacity(4 * (nx - 1) * (ny - 1) + 4 * (nx + ny));
    let id = |i: usize, j: usize| (i * ny + j) as u32;
    for i in 0..nx - 1 {
        for j in 0..ny - 1 {
            let (a, b, c, d) = (id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1));
            tris.push([a, d, c]);
            tris.push([a, c, b]);
            tris.push([a + off, b + off, c + off]);
            tris.push([a + off, c + off, d + off]);
        }
    }
    let mut ring: Vec<u32> = (0..nx).map(|i| id(i, 0)).collect();
    ring.extend((1..ny).map(|j| id(nx - 1, j)));
    ring.extend((0..nx - 1).rev().map(|i| id(i, ny - 1)));
    ring.extend((1..ny - 1).rev().map(|j| id(0, j)));
    for k in 0..ring.len() {
        let (r, r2) = (ring[k], ring[(k + 1) % ring.len()]);
        tris.push([r, r2, r2 + off]);
        tris.push([r, r2 + off, r + off]);
    }
    let vol: f64 = tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| pos[i as usize].as_dvec3());
            a.dot(b.cross(c))
        })
        .sum();
    if vol < 0.0 {
        for t in &mut tris {
            t.swap(1, 2);
        }
    }
    TriMesh::new(pos, tris)
}
