//! Reproducible Jacobi PCG of the random walks (`gpu_solve.repro_cg_cpu`,
//! `_repro_pcg`, `_repro_cg1`): the operator is given in ELL form, three
//! neighbours per row, and applied in the order of the plugin's kernel,
//! `d x - ((w0 x[n0] + w1 x[n1]) + w2 x[n2])`; the dot products are
//! [`dc_geom::sparse::exact_dot`]. All right-hand sides iterate together
//! until every residual is below the tolerance, as in the plugin, so the
//! result has the same bits as the oracle (up to the transcendental
//! functions that build the weights).

use dc_geom::sparse::exact_dot;
use rayon::prelude::*;

/// Rows per parallel work item.
const CHUNK: usize = 4096;

/// Operator `d x - Σ w x[nb]` with three neighbours per row (padding: weight
/// 0 on row 0).
pub(crate) struct Ell {
    pub nb: Vec<[u32; 3]>,
    pub w: Vec<[f64; 3]>,
    pub d: Vec<f64>,
}

impl Ell {
    fn apply(&self, x: &[f64], out: &mut [f64]) {
        out.par_chunks_mut(CHUNK)
            .enumerate()
            .for_each(|(chunk, o)| {
                let first = chunk * CHUNK;
                for (k, o) in o.iter_mut().enumerate() {
                    let i = first + k;
                    let (nb, w) = (self.nb[i], self.w[i]);
                    let mut s = x[nb[0] as usize] * w[0];
                    s += x[nb[1] as usize] * w[1];
                    s += x[nb[2] as usize] * w[2];
                    *o = self.d[i] * x[i] - s;
                }
            });
    }
}

/// Solution of [`repro_cg`].
pub(crate) struct CgSolution {
    /// One solution per right-hand side.
    pub x: Vec<Vec<f64>>,
    /// Iterations, as the plugin counts them (one less at the limit).
    pub iterations: usize,
    /// Final relative residual of every right-hand side.
    pub residuals: Vec<f64>,
}

fn update(out: &mut [f64], f: impl Fn(usize, f64) -> f64 + Sync) {
    out.par_chunks_mut(CHUNK)
        .enumerate()
        .for_each(|(chunk, xs)| {
            let first = chunk * CHUNK;
            for (k, x) in xs.iter_mut().enumerate() {
                *x = f(first + k, *x);
            }
        });
}

/// Jacobi PCG of `a x = b` for every column of `bs`, preconditioner
/// `1 / diag` (1 where not positive), warm start `x0`.
pub(crate) fn repro_cg(
    a: &Ell,
    bs: &[Vec<f64>],
    diag: &[f64],
    x0: Option<Vec<Vec<f64>>>,
    tol: f64,
    maxiter: usize,
) -> CgSolution {
    let n = a.d.len();
    let l = bs.len();
    let mut xs = x0.unwrap_or_else(|| vec![vec![0.0; n]; l]);
    let minv: Vec<f64> = diag
        .iter()
        .map(|&d| 1.0 / if d > 0.0 { d } else { 1.0 })
        .collect();
    let mut rs = Vec::with_capacity(l);
    let mut zs = Vec::with_capacity(l);
    let (mut rz, mut bn, mut res) = (vec![0.0; l], vec![0.0; l], vec![0.0; l]);
    for c in 0..l {
        let mut r = vec![0.0; n];
        a.apply(&xs[c], &mut r);
        let b = &bs[c];
        update(&mut r, |i, ax| b[i] - ax);
        let mut z = vec![0.0; n];
        update(&mut z, |i, _| minv[i] * r[i]);
        rz[c] = exact_dot(&r, &z);
        let nb = exact_dot(b, b).sqrt();
        bn[c] = if nb == 0.0 { 1.0 } else { nb };
        res[c] = exact_dot(&r, &r).sqrt() / bn[c];
        rs.push(r);
        zs.push(z);
    }
    let mut ps = zs.clone();
    let mut aps = vec![vec![0.0; n]; l];
    let mut it = 0usize;
    let mut at_limit = false;
    loop {
        if res.iter().all(|&r| r < tol) {
            break;
        }
        if it >= maxiter {
            at_limit = true;
            break;
        }
        let cols: Vec<(f64, f64)> = xs
            .par_iter_mut()
            .zip(rs.par_iter_mut())
            .zip(zs.par_iter_mut())
            .zip(ps.par_iter_mut())
            .zip(aps.par_iter_mut())
            .enumerate()
            .map(|(c, ((((x, r), z), p), ap))| {
                a.apply(p, ap);
                let pap = exact_dot(p, ap);
                let alpha = rz[c] / if pap.abs() > 1e-300 { pap } else { 1.0 };
                update(x, |i, x| x + alpha * p[i]);
                update(r, |i, r| r - alpha * ap[i]);
                {
                    let r: &[f64] = r;
                    update(z, |i, _| minv[i] * r[i]);
                }
                let rr = exact_dot(r, r);
                let rzn = exact_dot(r, z);
                let beta = rzn / if rz[c].abs() > 1e-300 { rz[c] } else { 1.0 };
                let z: &[f64] = z;
                update(p, |i, p| p * beta + z[i]);
                (rr, rzn)
            })
            .collect();
        for (c, (rr, rzn)) in cols.into_iter().enumerate() {
            res[c] = rr.sqrt() / bn[c];
            rz[c] = rzn;
        }
        it += 1;
    }
    CgSolution {
        x: xs,
        iterations: it - usize::from(at_limit),
        residuals: res,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_path_laplacian_with_dirichlet_ends() {
        // 3 free nodes on a path 0 - [1 2 3] - 4, x0 = 1, x4 = 0
        let a = Ell {
            nb: vec![[1, 0, 0], [0, 2, 0], [1, 0, 0]],
            w: vec![[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            d: vec![2.0, 2.0, 2.0],
        };
        let b = vec![vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0]];
        let s = repro_cg(&a, &b, &a.d.clone(), None, 1e-12, 100);
        for (got, want) in s.x[0].iter().zip([0.75, 0.5, 0.25]) {
            assert!((got - want).abs() < 1e-12);
        }
        for (got, want) in s.x[1].iter().zip([0.25, 0.5, 0.75]) {
            assert!((got - want).abs() < 1e-12);
        }
        assert!(s.residuals.iter().all(|&r| r < 1e-12));
    }
}
