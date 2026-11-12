//! Sequential sparse helpers for the small systems of one tooth.
//!
//! The fairing of a closed tooth solves a few thousand unknowns some thirty
//! times per tooth, and the teeth already run in parallel: the parallel
//! kernels of [`dc_geom::sparse`] spend more on scheduling than on the work
//! there. These are the same computations, single-threaded, giving the
//! same bits: [`csr_from_triplets`] is [`CsrMatrix::from_triplets`] (the
//! stable `(row, col)` order, duplicates added in input order from `0.0`)
//! with a counting sort, and [`pcg_solve_many`] is
//! [`dc_geom::sparse::pcg_solve_many`] with the same operation order
//! (`exact_dot` is exact, so its value does not depend on how it is split).

use dc_geom::sparse::{CsrMatrix, PcgParams};

/// [`CsrMatrix::from_triplets`] for in-range entries, bit for bit.
pub fn csr_from_triplets(n: usize, trip: &[(usize, usize, f64)]) -> CsrMatrix {
    // stable counting sort by row, then a stable sort by column in each row:
    // the order of a stable sort by (row, col)
    let mut start = vec![0usize; n + 1];
    for &(r, _, _) in trip {
        start[r + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut sorted: Vec<(usize, f64)> = vec![(0, 0.0); trip.len()];
    for &(r, c, v) in trip {
        sorted[fill[r]] = (c, v);
        fill[r] += 1;
    }
    let mut row_ptr = vec![0usize; n + 1];
    let mut cols = Vec::with_capacity(trip.len());
    let mut vals: Vec<f64> = Vec::with_capacity(trip.len());
    for r in 0..n {
        let row = &mut sorted[start[r]..start[r + 1]];
        row.sort_by_key(|e| e.0);
        let mut last = None;
        for &(c, v) in row.iter() {
            if last != Some(c) {
                cols.push(c);
                vals.push(0.0);
                last = Some(c);
            }
            *vals.last_mut().expect("pushed above") += v;
        }
        row_ptr[r + 1] = cols.len();
    }
    CsrMatrix {
        n,
        row_ptr,
        cols,
        vals,
    }
}

/// `frexp` exponent (as `dc_geom::sparse`).
fn frexp_exp(x: f64) -> i32 {
    if x == 0.0 || !x.is_finite() {
        return 0;
    }
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    if e == 0 {
        return frexp_exp(x * f64::powi(2.0, 64)) - 64;
    }
    e - 1022
}

/// `x 2^k` with a single rounding (as `dc_geom::sparse`).
fn ldexp(mut x: f64, mut k: i32) -> f64 {
    while k > 1000 {
        x *= f64::powi(2.0, 1000);
        k -= 1000;
    }
    while k < -1000 {
        x *= f64::powi(2.0, -1000);
        k += 1000;
    }
    x * f64::from_bits(((k + 1023) as u64) << 52)
}

/// `dc_geom::sparse::exact_dot`, single-threaded: the parts of every
/// product lie on the grids `c1`, `c2`, so their sums are exact in any
/// order and the result has the same bits.
pub fn exact_dot(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len();
    if n == 0 {
        return 0.0;
    }
    let m = a
        .iter()
        .zip(b)
        .fold(0.0f64, |m, (&u, &v)| m.max((u * v).abs()));
    let nf = n as f64;
    let e1 = frexp_exp(nf * m);
    let c1 = ldexp(1.5, e1 + 1);
    let c2 = ldexp(1.5, frexp_exp(nf * ldexp(1.0, e1 - 52)) + 1);
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (&u, &v) in a.iter().zip(b) {
        let p = u * v;
        let q1 = (p + c1) - c1;
        let q2 = ((p - q1) + c2) - c2;
        s1 += q1;
        s2 += q2;
    }
    s1 + s2
}

fn mul_vec_into(a: &CsrMatrix, x: &[f64], out: &mut [f64]) {
    for (i, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for e in a.row_ptr[i]..a.row_ptr[i + 1] {
            s += x[a.cols[e]] * a.vals[e];
        }
        *o = s;
    }
}

/// [`mul_vec_into`] of three vectors at once (same order per row).
fn mul_vec3(a: &CsrMatrix, x: &[Vec<f64>], out: &mut [Vec<f64>]) {
    let (x0, x1, x2) = (&x[0], &x[1], &x[2]);
    let [o0, o1, o2] = out else {
        return;
    };
    for i in 0..a.n {
        let (mut s0, mut s1, mut s2) = (0.0, 0.0, 0.0);
        for e in a.row_ptr[i]..a.row_ptr[i + 1] {
            let (c, v) = (a.cols[e], a.vals[e]);
            s0 += x0[c] * v;
            s1 += x1[c] * v;
            s2 += x2[c] * v;
        }
        o0[i] = s0;
        o1[i] = s1;
        o2[i] = s2;
    }
}

/// `dc_geom::sparse::pcg_solve_many(a, bs, Some(x0), params)`,
/// single-threaded, for right-hand sides and warm starts of `a.n` entries:
/// the solutions.
pub fn pcg_solve_many(
    a: &CsrMatrix,
    bs: &[Vec<f64>],
    x0: &[Vec<f64>],
    params: &PcgParams,
) -> Vec<Vec<f64>> {
    let n = a.n;
    let mut xs: Vec<Vec<f64>> = x0.to_vec();
    let minv: Vec<f64> = (0..n)
        .map(|i| {
            let (s, e) = (a.row_ptr[i], a.row_ptr[i + 1]);
            let d = a.cols[s..e]
                .iter()
                .zip(&a.vals[s..e])
                .filter(|&(&c, _)| c == i)
                .fold(0.0, |s, (_, &v)| s + v);
            1.0 / if d > 0.0 { d } else { 1.0 }
        })
        .collect();
    let l = bs.len();
    let mut rs: Vec<Vec<f64>> = Vec::with_capacity(l);
    let mut zs: Vec<Vec<f64>> = Vec::with_capacity(l);
    let mut rz = Vec::with_capacity(l);
    let mut bn = Vec::with_capacity(l);
    let mut res = Vec::with_capacity(l);
    let mut aps = vec![vec![0.0; n]; l];
    for (b, x) in bs.iter().zip(&xs) {
        let mut r = vec![0.0; n];
        mul_vec_into(a, x, &mut r);
        for (i, ri) in r.iter_mut().enumerate() {
            *ri = b[i] - *ri;
        }
        let z: Vec<f64> = r.iter().zip(&minv).map(|(r, m)| m * r).collect();
        rz.push(exact_dot(&r, &z));
        let nb = exact_dot(b, b).sqrt();
        let nb = if nb == 0.0 { 1.0 } else { nb };
        bn.push(nb);
        res.push(exact_dot(&r, &r).sqrt() / nb);
        rs.push(r);
        zs.push(z);
    }
    let mut ps = zs.clone();
    let mut it = 0usize;
    loop {
        if res.iter().all(|&r| r < params.tolerance) || it >= params.max_iterations {
            break;
        }
        // every A p first, the matrix read once (each right-hand side only
        // reads its own p before updating it, so the order is unchanged)
        if l == 3 {
            mul_vec3(a, &ps, &mut aps);
        } else {
            for (p, ap) in ps.iter().zip(aps.iter_mut()) {
                mul_vec_into(a, p, ap);
            }
        }
        for c in 0..l {
            let (p, r, z, x) = (&mut ps[c], &mut rs[c], &mut zs[c], &mut xs[c]);
            let ap = &aps[c];
            let pap = exact_dot(p, ap);
            let alpha = rz[c] / if pap.abs() > 1e-300 { pap } else { 1.0 };
            for (xi, pi) in x.iter_mut().zip(p.iter()) {
                *xi += alpha * pi;
            }
            for (ri, ai) in r.iter_mut().zip(ap) {
                *ri -= alpha * ai;
            }
            for ((zi, ri), m) in z.iter_mut().zip(r.iter()).zip(&minv) {
                *zi = m * ri;
            }
            res[c] = exact_dot(r, r).sqrt() / bn[c];
            let rzn = exact_dot(r, z);
            let beta = rzn / if rz[c].abs() > 1e-300 { rz[c] } else { 1.0 };
            rz[c] = rzn;
            for (pi, zi) in p.iter_mut().zip(z.iter()) {
                *pi = *pi * beta + zi;
            }
        }
        it += 1;
    }
    xs
}

/// Grids of `exact_dot` for `n` products of largest magnitude `m`.
fn grids(n: usize, m: f64) -> (f64, f64) {
    let nf = n as f64;
    let e1 = frexp_exp(nf * m);
    (
        ldexp(1.5, e1 + 1),
        ldexp(1.5, frexp_exp(nf * ldexp(1.0, e1 - 52)) + 1),
    )
}

/// Rows per parallel chunk: [`dc_geom::sparse::CHUNK`], so the partial
/// maxima and sums of every dot product are those of `exact_dot`, folded
/// in the same order.
const CH: usize = dc_geom::sparse::CHUNK;

/// `L` interleaved dot products `a[i * L + c] b[i * L + c]` of `n` rows,
/// `exact_dot` per column, in parallel.
fn dots<const L: usize>(a: &[f64], b: &[f64], n: usize) -> [f64; L] {
    use rayon::prelude::*;
    if n == 0 {
        return [0.0; L];
    }
    let maxes: Vec<[f64; L]> = a
        .par_chunks(CH * L)
        .zip(b.par_chunks(CH * L))
        .map(|(x, y)| {
            let mut m = [0.0f64; L];
            for (xr, yr) in x.as_chunks::<L>().0.iter().zip(y.as_chunks::<L>().0) {
                for c in 0..L {
                    m[c] = m[c].max((xr[c] * yr[c]).abs());
                }
            }
            m
        })
        .collect();
    let mut m = [0.0f64; L];
    for x in &maxes {
        for c in 0..L {
            m[c] = m[c].max(x[c]);
        }
    }
    let g: [(f64, f64); L] = std::array::from_fn(|c| grids(n, m[c]));
    let parts: Vec<[(f64, f64); L]> = a
        .par_chunks(CH * L)
        .zip(b.par_chunks(CH * L))
        .map(|(x, y)| {
            let mut s = [(0.0f64, 0.0f64); L];
            for (xr, yr) in x.as_chunks::<L>().0.iter().zip(y.as_chunks::<L>().0) {
                for c in 0..L {
                    let (c1, c2) = g[c];
                    let p = xr[c] * yr[c];
                    let q1 = (p + c1) - c1;
                    let q2 = ((p - q1) + c2) - c2;
                    s[c].0 += q1;
                    s[c].1 += q2;
                }
            }
            s
        })
        .collect();
    std::array::from_fn(|c| {
        let (s1, s2) = parts
            .iter()
            .fold((0.0, 0.0), |(a1, a2), b| (a1 + b[c].0, a2 + b[c].1));
        s1 + s2
    })
}

/// How many iterations of [`pcg_solve_many_par`] pass between two asks of
/// its `cancelled` (an iteration reads the matrix once: milliseconds).
const CANCEL_EVERY: usize = 8;

/// `dc_geom::sparse::pcg_solve_many(a, bs, Some(x0), params)` for `L`
/// right-hand sides iterated together in parallel passes over the rows
/// (the matrix read once per iteration for all of them), with the same
/// operations per right-hand side and the same bits. `cancelled` is asked
/// every few iterations; once `true` the solve stops and returns `None`.
pub fn pcg_solve_many_par<const L: usize>(
    a: &CsrMatrix,
    bs: &[Vec<f64>; L],
    x0: &[Vec<f64>; L],
    params: &PcgParams,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<[Vec<f64>; L]> {
    use dc_geom::sparse::exact_dot as edot;
    use rayon::prelude::*;
    let n = a.n;
    let minv: Vec<f64> = a
        .diagonal()
        .into_iter()
        .map(|d| 1.0 / if d > 0.0 { d } else { 1.0 })
        .collect();
    let mut rz = [0.0f64; L];
    let mut bn = [0.0f64; L];
    let mut res = [0.0f64; L];
    let (mut x, mut r, mut z) = (vec![0.0; n * L], vec![0.0; n * L], vec![0.0; n * L]);
    for c in 0..L {
        let mut rc = a.mul_vec(&x0[c]);
        for (i, ri) in rc.iter_mut().enumerate() {
            *ri = bs[c][i] - *ri;
        }
        let zc: Vec<f64> = rc.iter().zip(&minv).map(|(r, m)| m * r).collect();
        rz[c] = edot(&rc, &zc);
        let nb = edot(&bs[c], &bs[c]).sqrt();
        bn[c] = if nb == 0.0 { 1.0 } else { nb };
        res[c] = edot(&rc, &rc).sqrt() / bn[c];
        for i in 0..n {
            x[i * L + c] = x0[c][i];
            r[i * L + c] = rc[i];
            z[i * L + c] = zc[i];
        }
    }
    let mut p = z.clone();
    let mut ap = vec![0.0; n * L];
    let mut it = 0usize;
    loop {
        if res.iter().all(|&r| r < params.tolerance) || it >= params.max_iterations {
            break;
        }
        if it.is_multiple_of(CANCEL_EVERY) && cancelled() {
            return None;
        }
        ap.par_chunks_mut(CH * L).enumerate().for_each(|(k, out)| {
            for (j, o) in out.as_chunks_mut::<L>().0.iter_mut().enumerate() {
                let i = k * CH + j;
                let mut s = [0.0f64; L];
                for e in a.row_ptr[i]..a.row_ptr[i + 1] {
                    let (col, v) = (a.cols[e], a.vals[e]);
                    for c in 0..L {
                        s[c] += p[col * L + c] * v;
                    }
                }
                o.copy_from_slice(&s);
            }
        });
        let pap = dots::<L>(&p, &ap, n);
        let alpha: [f64; L] =
            std::array::from_fn(|c| rz[c] / if pap[c].abs() > 1e-300 { pap[c] } else { 1.0 });
        x.par_chunks_mut(CH * L)
            .zip(r.par_chunks_mut(CH * L))
            .zip(z.par_chunks_mut(CH * L))
            .enumerate()
            .for_each(|(k, ((xs, rs), zs))| {
                let base = k * CH * L;
                for j in 0..xs.len() {
                    let c = j % L;
                    xs[j] += alpha[c] * p[base + j];
                    rs[j] -= alpha[c] * ap[base + j];
                    zs[j] = minv[(base + j) / L] * rs[j];
                }
            });
        let rr = dots::<L>(&r, &r, n);
        let rzn = dots::<L>(&r, &z, n);
        let mut beta = [0.0f64; L];
        for c in 0..L {
            res[c] = rr[c].sqrt() / bn[c];
            beta[c] = rzn[c] / if rz[c].abs() > 1e-300 { rz[c] } else { 1.0 };
            rz[c] = rzn[c];
        }
        p.par_chunks_mut(CH * L)
            .zip(z.par_chunks(CH * L))
            .for_each(|(ps, zs)| {
                for j in 0..ps.len() {
                    ps[j] = ps[j] * beta[j % L] + zs[j];
                }
            });
        it += 1;
    }
    Some(std::array::from_fn(|c| {
        (0..n).map(|i| x[i * L + c]).collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(n: usize) -> Vec<(usize, usize, f64)> {
        let mut t = Vec::new();
        for i in 0..n {
            t.push((i, i, 4.0 + (i % 3) as f64 * 0.1));
            if i + 1 < n {
                t.push((i + 1, i, -1.0 - (i % 5) as f64 * 0.01));
                t.push((i, i + 1, -1.0 - (i % 5) as f64 * 0.01));
            }
            // duplicates, in input order
            t.push((i, i, 0.3));
            t.push(((i * 7) % n, (i * 7) % n, 0.1));
        }
        t
    }

    #[test]
    fn csr_matches_from_triplets() {
        let t = system(5000);
        assert_eq!(
            csr_from_triplets(5000, &t),
            CsrMatrix::from_triplets(5000, &t).unwrap()
        );
    }

    #[test]
    fn pcg_and_dot_match_the_parallel_ones_bit_for_bit() {
        let n = 9000;
        let a = csr_from_triplets(n, &system(n));
        let bs: Vec<Vec<f64>> = (0..3)
            .map(|c| (0..n).map(|i| ((i * (c + 3)) % 17) as f64 - 8.1).collect())
            .collect();
        let x0: Vec<Vec<f64>> = (0..3)
            .map(|c| (0..n).map(|i| ((i + c) % 5) as f64 * 0.01).collect())
            .collect();
        assert_eq!(
            exact_dot(&bs[0], &bs[1]).to_bits(),
            dc_geom::sparse::exact_dot(&bs[0], &bs[1]).to_bits()
        );
        let p = PcgParams {
            max_iterations: 400,
            tolerance: 1e-10,
        };
        let ours = pcg_solve_many(&a, &bs, &x0, &p);
        let theirs = dc_geom::sparse::pcg_solve_many(&a, &bs, Some(&x0), &p).unwrap();
        let par = pcg_solve_many_par::<3>(
            &a,
            &[bs[0].clone(), bs[1].clone(), bs[2].clone()],
            &[x0[0].clone(), x0[1].clone(), x0[2].clone()],
            &p,
            &|| false,
        )
        .unwrap();
        let bits =
            |x: &[Vec<f64>]| -> Vec<u64> { x.iter().flatten().map(|v| v.to_bits()).collect() };
        assert_eq!(bits(&ours), bits(&theirs.x));
        assert_eq!(bits(&par), bits(&theirs.x));
    }
}
