//! Thin-plate fairing, untangling and despiking of a closed tooth
//! (`reconstruct.fair`, `untangle`, `edge_angles`, `despike`).

use dc_geom::sparse::PcgParams;
use glam::DVec3;

use super::geom::{self, unique_edges};

/// Uniform Laplacian `L = I - D^-1 A` of the edges (`Umbrella`): the
/// neighbours of every vertex and its degree (at least 1).
struct Umbrella {
    start: Vec<usize>,
    nb: Vec<u32>,
    deg: Vec<f64>,
}

impl Umbrella {
    fn new(edges: &[[u32; 2]], n: usize) -> Self {
        let mut count = vec![0usize; n];
        for e in edges {
            count[e[0] as usize] += 1;
            count[e[1] as usize] += 1;
        }
        let mut start = vec![0usize; n + 1];
        for i in 0..n {
            start[i + 1] = start[i] + count[i];
        }
        let mut fill = start.clone();
        let mut nb = vec![0u32; start[n]];
        // directed edges i -> j in the order of `i = [E0, E1]`, `j = [E1, E0]`
        for e in edges {
            nb[fill[e[0] as usize]] = e[1];
            fill[e[0] as usize] += 1;
        }
        for e in edges {
            nb[fill[e[1] as usize]] = e[0];
            fill[e[1] as usize] += 1;
        }
        let deg = count.iter().map(|&c| c.max(1) as f64).collect();
        Self { start, nb, deg }
    }

    fn neighbours(&self, i: usize) -> &[u32] {
        &self.nb[self.start[i]..self.start[i + 1]]
    }

    /// `L x` of one coordinate column.
    fn apply(&self, x: &[f64]) -> Vec<f64> {
        (0..x.len())
            .map(|i| {
                let s: f64 = self.neighbours(i).iter().map(|&j| x[j as usize]).sum();
                x[i] - s / self.deg[i]
            })
            .collect()
    }

    /// `L^T y` of one column.
    fn apply_t(&self, y: &[f64]) -> Vec<f64> {
        let yd: Vec<f64> = y.iter().zip(&self.deg).map(|(a, d)| a / d).collect();
        (0..y.len())
            .map(|i| {
                let s: f64 = self.neighbours(i).iter().map(|&j| yd[j as usize]).sum();
                y[i] - s
            })
            .collect()
    }
}

/// Per-vertex weight: one value for all, or one per vertex.
#[derive(Debug, Clone, Copy)]
pub enum Weight<'a> {
    /// The same weight everywhere.
    All(f64),
    /// One weight per vertex.
    Each(&'a [f64]),
}

impl Weight<'_> {
    fn at(&self, i: usize) -> f64 {
        match self {
            Weight::All(w) => *w,
            Weight::Each(w) => w[i],
        }
    }
}

/// Thin-plate fairing (`fair`): positions of the `free` vertices minimising
/// the sum over the `rows` vertices of `|L x|^2` (umbrella Laplacian) plus
/// `anchor_w |x - x0|^2` over the `anchor` vertices, all other vertices
/// fixed. Normal equations solved by Jacobi-preconditioned conjugate
/// gradients from the current positions (single-threaded, see
/// [`super::solve`]: the teeth are closed in parallel).
pub fn fair(
    v: &[DVec3],
    tris: &[[u32; 3]],
    free: &[bool],
    rows: &[bool],
    anchor: Option<&[bool]>,
    anchor_w: Weight<'_>,
) -> Vec<DVec3> {
    let n = v.len();
    let edges = unique_edges(tris);
    let lap = Umbrella::new(&edges, n);
    let fidx: Vec<usize> = (0..n).filter(|&i| free[i]).collect();
    if fidx.is_empty() {
        return v.to_vec();
    }
    let mut fpos = vec![usize::MAX; n];
    for (k, &i) in fidx.iter().enumerate() {
        fpos[i] = k;
    }
    let nf = fidx.len();
    let lam: Vec<f64> = fidx
        .iter()
        .map(|&i| match anchor {
            Some(a) if a[i] => anchor_w.at(i),
            _ => 0.0,
        })
        .collect();
    // matrix L_RF^T L_RF + diag(lam) (gpu_solve.normal_matrix)
    let mut trip: Vec<(usize, usize, f64)> = Vec::new();
    let mut row_entries: Vec<(usize, f64)> = Vec::new();
    for r in 0..n {
        if !rows[r] {
            continue;
        }
        row_entries.clear();
        if fpos[r] != usize::MAX {
            row_entries.push((fpos[r], 1.0));
        }
        for &j in lap.neighbours(r) {
            let c = fpos[j as usize];
            if c != usize::MAX {
                row_entries.push((c, -1.0 / lap.deg[r]));
            }
        }
        for &(c1, v1) in &row_entries {
            for &(c2, v2) in &row_entries {
                trip.push((c1, c2, v1 * v2));
            }
        }
    }
    for (k, &l) in lam.iter().enumerate() {
        trip.push((k, k, l));
    }
    let a = super::solve::csr_from_triplets(nf, &trip);
    // right-hand side: -L_RF^T L_R,fixed x_fixed + lam x0
    let mut bs = Vec::with_capacity(3);
    let mut x0 = Vec::with_capacity(3);
    #[allow(clippy::needless_range_loop)] // d is the coordinate of every vertex
    for d in 0..3 {
        let xb: Vec<f64> = (0..n)
            .map(|i| if free[i] { 0.0 } else { v[i][d] })
            .collect();
        let mut y = lap.apply(&xb);
        for (i, yi) in y.iter_mut().enumerate() {
            if !rows[i] {
                *yi = 0.0;
            }
        }
        let nt = lap.apply_t(&y);
        bs.push(
            fidx.iter()
                .enumerate()
                .map(|(k, &i)| -nt[i] + lam[k] * v[i][d])
                .collect::<Vec<f64>>(),
        );
        x0.push(fidx.iter().map(|&i| v[i][d]).collect::<Vec<f64>>());
    }
    let params = PcgParams {
        max_iterations: 4000,
        tolerance: 1e-6,
    };
    let mut out = v.to_vec();
    let sol = super::solve::pcg_solve_many(&a, &bs, &x0, &params);
    for (k, &i) in fidx.iter().enumerate() {
        out[i] = DVec3::new(sol[0][k], sol[1][k], sol[2][k]);
    }
    out
}

/// Local umbrella smoothing around crossing triangles (`untangle`): free
/// vertices first; tiny folds of the scan itself (crossings between fixed
/// vertices) are smoothed too when nothing else is left. Returns the
/// positions with the fewest crossings seen.
pub fn untangle(
    v: &[DVec3],
    tris: &[[u32; 3]],
    free: &[bool],
    edges: &[[u32; 2]],
    rounds: usize,
    iterations: usize,
) -> Vec<DVec3> {
    let n = v.len();
    let mut v = v.to_vec();
    let deg: Vec<f64> = geom::edge_degree(edges, n)
        .into_iter()
        .map(|d| d.max(1) as f64)
        .collect();
    let mut last: Option<usize> = None;
    let mut best: Option<(usize, Vec<DVec3>)> = None;
    for rnd in 0..=rounds {
        let pairs = geom::self_intersecting_pairs(&v, tris);
        if best.as_ref().is_none_or(|b| pairs.len() < b.0) {
            best = Some((pairs.len(), v.clone()));
        }
        if pairs.is_empty() || rnd == rounds {
            break;
        }
        let mut touched = vec![false; n];
        for &(a, b) in &pairs {
            for f in [a, b] {
                for &i in &tris[f as usize] {
                    touched[i as usize] = true;
                }
            }
        }
        let stuck = last.is_some_and(|l| pairs.len() >= l);
        last = Some(pairs.len());
        let any_free = (0..n).any(|i| touched[i] && free[i]);
        let movable: Vec<bool> = if any_free && !stuck {
            free.to_vec()
        } else {
            vec![true; n]
        };
        let mut sel: Vec<bool> = (0..n).map(|i| touched[i] && movable[i]).collect();
        for _ in 0..3 {
            let mut grow = sel.clone();
            for e in edges {
                let (a, b) = (e[0] as usize, e[1] as usize);
                if sel[a] {
                    grow[b] = true;
                }
                if sel[b] {
                    grow[a] = true;
                }
            }
            sel = (0..n).map(|i| grow[i] && movable[i]).collect();
        }
        let idx: Vec<usize> = (0..n).filter(|&i| sel[i]).collect();
        for _ in 0..iterations {
            let mut acc = vec![DVec3::ZERO; n];
            for e in edges {
                acc[e[0] as usize] += v[e[1] as usize];
            }
            for e in edges {
                acc[e[1] as usize] += v[e[0] as usize];
            }
            for &i in &idx {
                v[i] = 0.5 * v[i] + 0.5 * (acc[i] / deg[i]);
            }
        }
    }
    best.map_or(v, |b| b.1)
}

/// Edges shared by two faces and the angle (degrees) between the normals
/// of those faces (`edge_angles`).
pub fn edge_angles(v: &[DVec3], tris: &[[u32; 3]]) -> Vec<([u32; 2], f64)> {
    let (fn_, _) = geom::tri_normals(v, tris);
    let mut e: Vec<([u32; 2], u32)> = Vec::with_capacity(tris.len() * 3);
    for k in 0..3 {
        for (f, t) in tris.iter().enumerate() {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            e.push((if a < b { [a, b] } else { [b, a] }, f as u32));
        }
    }
    e.sort_by_key(|x| x.0);
    let mut out = Vec::new();
    for w in e.windows(2) {
        if w[0].0 == w[1].0 {
            let d = fn_[w[0].1 as usize]
                .dot(fn_[w[1].1 as usize])
                .clamp(-1.0, 1.0);
            out.push((w[0].0, d.acos().to_degrees()));
        }
    }
    out
}

/// Sharp vertices: those of an edge sharper than `deg`, not `fixed`.
fn sharp_vertices(v: &[DVec3], tris: &[[u32; 3]], deg: f64, fixed: &[bool]) -> Vec<u32> {
    let mut bad: Vec<u32> = edge_angles(v, tris)
        .into_iter()
        .filter(|(_, a)| *a > deg)
        .flat_map(|(e, _)| e)
        .collect();
    bad.sort_unstable();
    bad.dedup();
    bad.retain(|&i| !fixed[i as usize]);
    bad
}

/// Despiking (`despike`): every vertex of an edge sharper than `deg` and
/// its `rings` rings are faired held `w` where they are, then untangled,
/// until none is left or `rounds` times; the `fixed` vertices never move.
/// Returns `(positions, vertices moved, sharp vertices left)`.
pub fn despike(
    v: &[DVec3],
    tris: &[[u32; 3]],
    fixed: &[bool],
    deg: f64,
    rings: usize,
    w: Weight<'_>,
    rounds: usize,
) -> (Vec<DVec3>, usize, usize) {
    let n = v.len();
    let edges = unique_edges(tris);
    let mut v = v.to_vec();
    let mut moved = vec![false; n];
    let mut left = 0usize;
    let mut completed = true;
    for _ in 0..rounds {
        let bad = sharp_vertices(&v, tris, deg, fixed);
        left = bad.len();
        if left == 0 {
            completed = false;
            break;
        }
        let mut free = vec![false; n];
        for &i in &bad {
            free[i as usize] = true;
        }
        for _ in 0..rings {
            let mut grow = free.clone();
            for e in &edges {
                if free[e[0] as usize] || free[e[1] as usize] {
                    grow[e[0] as usize] = true;
                    grow[e[1] as usize] = true;
                }
            }
            free = grow;
        }
        for i in 0..n {
            free[i] &= !fixed[i];
        }
        let mut near = free.clone();
        for e in &edges {
            if free[e[0] as usize] {
                near[e[1] as usize] = true;
            }
            if free[e[1] as usize] {
                near[e[0] as usize] = true;
            }
        }
        v = fair(&v, tris, &free, &near, Some(&free), w);
        v = untangle(&v, tris, &free, &edges, 10, 10);
        for i in 0..n {
            moved[i] |= free[i];
        }
    }
    if completed {
        left = sharp_vertices(&v, tris, deg, fixed).len();
    }
    (v, moved.iter().filter(|&&m| m).count(), left)
}
