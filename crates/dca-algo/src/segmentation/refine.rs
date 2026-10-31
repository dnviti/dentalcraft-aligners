//! Boundary refinement at full resolution (`segmentation.refine_boundaries_iter`,
//! `_pair_walk`): for every pair of adjacent labels the faces near their
//! common boundary are labelled again by a two-label random walker with
//! sharper fold weights, seeded by the rest of the two labels.

use std::collections::{BTreeSet, HashMap};

use glam::DVec3;

use super::clean::face_labels;
use super::graph::{FaceGraph, face_graph};
use super::mesh::NONE;
use super::solve::{Ell, repro_cg};
use super::{GUM, RefineParams};

/// Two-label random walker on the faces of `region` (probability of X),
/// X and Y seeds outside it, other neighbours ignored.
fn pair_walk(
    g: &FaceGraph,
    region: &[bool],
    seed_x: &[bool],
    seed_y: &[bool],
    tol: f64,
    maxiter: usize,
) -> (Vec<usize>, Vec<f64>) {
    let idx: Vec<usize> = (0..region.len()).filter(|&i| region[i]).collect();
    let mut pos = vec![0u32; region.len()];
    for (k, &i) in idx.iter().enumerate() {
        pos[i] = k as u32;
    }
    let n = idx.len();
    let mut ell = Ell {
        nb: vec![[0; 3]; n],
        w: vec![[0.0; 3]; n],
        d: vec![0.0; n],
    };
    let mut b = vec![0.0; n];
    for (r, &i) in idx.iter().enumerate() {
        let mut dk = [0.0f64; 3];
        let mut bk = [0.0f64; 3];
        for k in 0..3 {
            let nb = g.nbr[i][k];
            if nb == NONE {
                continue;
            }
            let nb = nb as usize;
            let wr = g.w[i][k];
            let inside = region[nb];
            let to_x = seed_x[nb] && !region[nb];
            let to_y = seed_y[nb] && !region[nb];
            if inside || to_x || to_y {
                dk[k] = wr;
            }
            if inside {
                ell.nb[r][k] = pos[nb];
                ell.w[r][k] = wr;
            }
            if to_x {
                bk[k] = wr;
            }
        }
        ell.d[r] = (dk[0] + dk[1]) + dk[2];
        b[r] = (bk[0] + bk[1]) + bk[2];
    }
    let diag: Vec<f64> = ell.d.iter().map(|&d| d.max(1e-12)).collect();
    let sol = repro_cg(&ell, &[b], &diag, Some(vec![vec![0.5; n]]), tol, maxiter);
    let x = sol.x.into_iter().next().unwrap_or_default();
    (idx, x)
}

/// Distance from every point of `p` (where `want`) to the nearest point of
/// `s`, exact when below `reach`, infinite beyond.
fn min_distances_within(p: &[DVec3], want: &[usize], s: &[DVec3], reach: f64) -> Vec<(usize, f64)> {
    let cell = reach.max(1e-6);
    let key = |q: DVec3| {
        (
            (q.x / cell).floor() as i64,
            (q.y / cell).floor() as i64,
            (q.z / cell).floor() as i64,
        )
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (j, &q) in s.iter().enumerate() {
        grid.entry(key(q)).or_default().push(j);
    }
    want.iter()
        .map(|&i| {
            let q = p[i];
            let (kx, ky, kz) = key(q);
            let mut best = f64::INFINITY;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(js) = grid.get(&(kx + dx, ky + dy, kz + dz)) {
                            for &j in js {
                                let e = q - s[j];
                                let d = (e.x * e.x + e.y * e.y + e.z * e.z).sqrt();
                                best = best.min(d);
                            }
                        }
                    }
                }
            }
            (i, if best < reach { best } else { f64::INFINITY })
        })
        .collect()
}

/// Vertex labels after the boundary refinement of `labels` on the mesh
/// `(v, t)` in arch coordinates.
pub(crate) fn refine_boundaries(
    v: &[DVec3],
    t: &[[u32; 3]],
    labels: &[u16],
    params: &RefineParams,
) -> Vec<u16> {
    let mut fl = face_labels(t, labels);
    let g = face_graph(v, t, params.eta_convex, params.sigma, params.normal_scale);
    let m = t.len();
    let mut pairs: BTreeSet<(u16, u16)> = BTreeSet::new();
    for i in 0..m {
        for &nb in &g.nbr[i] {
            if nb != NONE && fl[nb as usize] != fl[i] {
                let (a, b) = (fl[i], fl[nb as usize]);
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    // tooth-tooth boundaries first, then tooth-gingiva
    let mut order: Vec<(u16, u16)> = pairs.into_iter().collect();
    order.sort_by_key(|&(x, y)| (i64::from(x) == GUM, x, y));
    let band = params.band;
    for (x, y) in order {
        let in_xy: Vec<bool> = fl.iter().map(|&l| l == x || l == y).collect();
        let bnd: Vec<usize> = (0..m)
            .filter(|&i| {
                in_xy[i]
                    && g.nbr[i].iter().any(|&nb| {
                        nb != NONE
                            && ((fl[i] == x && fl[nb as usize] == y)
                                || (fl[i] == y && fl[nb as usize] == x))
                    })
            })
            .collect();
        if bnd.len() < 3 {
            continue;
        }
        let cb: Vec<DVec3> = bnd.iter().map(|&i| g.c[i]).collect();
        let (mut lo, mut hi) = (cb[0], cb[0]);
        for &q in &cb {
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let (lo, hi) = (lo - band, hi + band);
        let cand: Vec<usize> = (0..m)
            .filter(|&i| in_xy[i] && g.c[i].cmpge(lo).all() && g.c[i].cmple(hi).all())
            .collect();
        let mut dist = vec![f64::INFINITY; m];
        for (i, d) in min_distances_within(&g.c, &cand, &cb, band) {
            dist[i] = d;
        }
        let count = |l: u16| fl.iter().filter(|&&f| f == l).count();
        let (nx, ny) = (count(x), count(y));
        let mut b = band;
        let mut region;
        loop {
            region = (0..m)
                .map(|i| in_xy[i] && dist[i] < b)
                .collect::<Vec<bool>>();
            let ok = [(x, nx), (y, ny)].iter().all(|&(f, nf)| {
                let inside = (0..m).filter(|&i| region[i] && fl[i] == f).count();
                inside as f64 <= (1.0 - params.min_core) * nf.max(1) as f64
            });
            if ok || b < 0.3 {
                break;
            }
            b *= 0.7;
        }
        if region.iter().filter(|&&r| r).count() < 3 {
            continue;
        }
        let seed_x: Vec<bool> = (0..m).map(|i| fl[i] == x && !region[i]).collect();
        let seed_y: Vec<bool> = (0..m).map(|i| fl[i] == y && !region[i]).collect();
        let (idx, px) = pair_walk(&g, &region, &seed_x, &seed_y, params.tol, params.maxiter);
        let new: Vec<u16> = idx
            .iter()
            .zip(&px)
            .map(|(&i, &p)| {
                let l = if p >= 0.5 { x } else { y };
                // a tooth takes gingiva only within gain_band of its first boundary
                match params.gain_band_mm {
                    Some(gb) if i64::from(x) == GUM && fl[i] == x && dist[i] > gb => x,
                    _ => l,
                }
            })
            .collect();
        for (&i, l) in idx.iter().zip(new) {
            fl[i] = l;
        }
    }
    // vertices: the label of most of their faces (ties to the smallest label)
    let mut ids: Vec<u16> = fl.clone();
    ids.sort_unstable();
    ids.dedup();
    let k = ids.len();
    let mut stack = vec![0u32; v.len() * k];
    for (f, &l) in t.iter().zip(&fl) {
        let j = ids.binary_search(&l).unwrap_or(0);
        for &i in f {
            stack[i as usize * k + j] += 1;
        }
    }
    let mut out = labels.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        let row = &stack[i * k..(i + 1) * k];
        if row.iter().any(|&c| c > 0) {
            let mut best = 0;
            for j in 1..k {
                if row[j] > row[best] {
                    best = j;
                }
            }
            *o = ids[best];
        }
    }
    out
}
