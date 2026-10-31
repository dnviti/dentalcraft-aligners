//! The arch line of the setup and the positions along it: the prominent
//! cusps smoothed by a robust local quadratic regression (`cusp_line`), the
//! projections on the polyline, and the weighted isotonic regression that
//! keeps the teeth in order without overlaps (`isotonic`, `solve_chain`).
//!
//! Reference: the first two sections of `ortho_aligner/setup_conservativo.py`.
//! Lines are polylines `(k, 3)` in arch coordinates; the arc length is
//! measured in plan (x, y).

use std::collections::BTreeMap;

use dc_case::Fdi;
use glam::{DVec2, DVec3};

/// Spacing of the samples of the arch line (mm).
const STEP: f64 = 0.25;
/// Straight extension of the line beyond the first and the last tooth (mm).
const EXT: f64 = 4.0;
/// Passes of robust re-weighting.
const ROBUST: usize = 5;
/// Smallest scale of the biweight of the residuals (mm).
const MIN_SCALE: f64 = 0.6;
/// Weight of an anchor tooth (locked by the clinician).
const ANCHOR_WEIGHT: f64 = 10.0;

// ---- numpy semantics ----------------------------------------------------------------------------------

/// `numpy.interp(x, xp, fp)` for non-decreasing `xp`, clamped at the ends.
pub(crate) fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if x < xp[0] {
        return fp[0];
    }
    if x > xp[n - 1] {
        return fp[n - 1];
    }
    let j = xp.partition_point(|&v| v <= x).saturating_sub(1);
    if j == n - 1 || x == xp[j] {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    let r = slope * (x - xp[j]) + fp[j];
    if r.is_nan() {
        let r2 = slope * (x - xp[j + 1]) + fp[j + 1];
        if r2.is_nan() && fp[j] == fp[j + 1] {
            return fp[j];
        }
        return r2;
    }
    r
}

/// `numpy.median`.
pub(crate) fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// `numpy.arange(start, stop, step)` for floats.
pub(crate) fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    (0..n).map(|i| start + i as f64 * step).collect()
}

/// `numpy.clip(x, lo, hi)` (`min(max(x, lo), hi)`).
pub(crate) fn clip(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

/// Plan cross product.
pub(crate) fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

// ---- polylines ----------------------------------------------------------------------------------------

/// Cumulative arc length in plan of the vertices of `l`.
pub(crate) fn arc(l: &[DVec3]) -> Vec<f64> {
    let mut out = Vec::with_capacity(l.len());
    let mut acc = 0.0;
    for (i, p) in l.iter().enumerate() {
        if i > 0 {
            acc += (p.truncate() - l[i - 1].truncate()).length();
        }
        out.push(acc);
    }
    out
}

/// Arc length and point (plan) of the point of the line nearest to `p` in
/// plan (`_project`).
pub(crate) fn project(l: &[DVec3], sl: &[f64], p: DVec3) -> (f64, DVec2) {
    let p2 = p.truncate();
    let mut best = (f64::INFINITY, 0.0, DVec2::ZERO);
    for j in 0..l.len().saturating_sub(1) {
        let a = l[j].truncate();
        let ab = l[j + 1].truncate() - a;
        let l2 = ab.length_squared().max(1e-12);
        let t = ((p2 - a).dot(ab) / l2).clamp(0.0, 1.0);
        let q = a + t * ab;
        let d = (q - p2).length_squared();
        if d < best.0 {
            best = (d, sl[j] + t * l2.sqrt(), q);
        }
    }
    (best.1, best.2)
}

/// Point of the line at arc length `s`, extended straight beyond the ends.
pub(crate) fn point_at(l: &[DVec3], sl: &[f64], s: f64) -> DVec3 {
    let n = l.len();
    if s <= sl[0] {
        let d = l[1] - l[0];
        return l[0] + d * (s - sl[0]) / d.truncate().length().max(1e-12);
    }
    if s >= sl[n - 1] {
        let d = l[n - 1] - l[n - 2];
        return l[n - 1] + d * (s - sl[n - 1]) / d.truncate().length().max(1e-12);
    }
    let col = |k: usize| -> f64 {
        let fp: Vec<f64> = l.iter().map(|p| p[k]).collect();
        interp(s, sl, &fp)
    };
    DVec3::new(col(0), col(1), col(2))
}

/// Unit plan tangent of the line at arc length `s` (central difference over
/// 2 x 0.5 mm).
pub(crate) fn tangent_at(l: &[DVec3], sl: &[f64], s: f64) -> DVec2 {
    let d = (point_at(l, sl, s + 0.5) - point_at(l, sl, s - 0.5)).truncate();
    d / d.length().max(1e-12)
}

/// The plan normal of the tangent `tg` that points away from `interior`.
pub(crate) fn outward(tg: DVec2, interior: DVec2, p: DVec2) -> DVec2 {
    let n = DVec2::new(-tg.y, tg.x);
    if n.dot(p - interior) >= 0.0 { n } else { -n }
}

/// Plan distance (mm) of each point from the line.
pub fn line_distance(l: &[DVec3], pts: &[DVec3]) -> Vec<f64> {
    let sl = arc(l);
    pts.iter()
        .map(|&p| (project(l, &sl, p).1 - p.truncate()).length())
        .collect()
}

/// Distance of each point from the line in plan and in height (the height
/// of the line at the nearest point in plan).
fn offsets_3d(l: &[DVec3], pts: &[DVec3]) -> Vec<f64> {
    let sl = arc(l);
    let zs: Vec<f64> = l.iter().map(|p| p.z).collect();
    pts.iter()
        .map(|&p| {
            let (s, q) = project(l, &sl, p);
            let z = interp(s, &sl, &zs);
            (q - p.truncate()).length().hypot(p.z - z)
        })
        .collect()
}

/// Outward plan normals at the vertices of the line; `interior` is the mean
/// plan position of the cusps.
pub(crate) fn line_normals(l: &[DVec3], interior: DVec2) -> Vec<DVec2> {
    let sl = arc(l);
    sl.iter()
        .zip(l)
        .map(|(&s, p)| outward(tangent_at(l, &sl, s), interior, p.truncate()))
        .collect()
}

/// Mean plan position of all the cusps of the teeth of `seq`.
pub(crate) fn interior(cusps: &BTreeMap<Fdi, Vec<DVec3>>, seq: &[Fdi]) -> DVec2 {
    let pts: Vec<DVec2> = seq
        .iter()
        .filter_map(|f| cusps.get(f))
        .flatten()
        .map(|p| p.truncate())
        .collect();
    pts.iter().copied().sum::<DVec2>() / pts.len().max(1) as f64
}

/// The line reversed when its last point is nearer to `start` than its first.
pub(crate) fn oriented(l: &[DVec3], start: DVec3) -> Vec<DVec3> {
    let mut out = l.to_vec();
    if let (Some(first), Some(last)) = (l.first(), l.last())
        && (last.truncate() - start.truncate()).length()
            < (first.truncate() - start.truncate()).length()
    {
        out.reverse();
    }
    out
}

// ---- the line of the cusps ----------------------------------------------------------------------------

/// The cusps in arch order: teeth in the order of `seq`, the cusps of one
/// tooth along the direction from the previous tooth to the next. With the
/// tooth of each point.
pub fn order_points(cusps: &BTreeMap<Fdi, Vec<DVec3>>, seq: &[Fdi]) -> (Vec<DVec3>, Vec<Fdi>) {
    let seq: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| cusps.get(f).is_some_and(|c| !c.is_empty()))
        .collect();
    let mean = |c: &[DVec3]| c.iter().copied().sum::<DVec3>() / c.len() as f64;
    let centers: Vec<DVec3> = seq.iter().map(|f| mean(&cusps[f])).collect();
    let (mut out, mut lab) = (Vec::new(), Vec::new());
    for (i, f) in seq.iter().enumerate() {
        let mut c = cusps[f].clone();
        if c.len() > 1 {
            let a = if i > 0 { centers[i - 1] } else { centers[i] };
            let b = if i + 1 < seq.len() {
                centers[i + 1]
            } else {
                centers[i]
            };
            let mut d = (b - a).truncate();
            if d.length() < 1e-9 {
                d = DVec2::X;
            }
            c.sort_by(|p, q| p.truncate().dot(d).total_cmp(&q.truncate().dot(d)));
        }
        lab.extend(std::iter::repeat_n(*f, c.len()));
        out.extend(c);
    }
    (out, lab)
}

/// Weighted local quadratic regression of `y` (one row per sample `t`) at
/// the points `tg`, Gaussian weights of width `sigma` times `w`.
pub(crate) fn local_quadratic<const D: usize>(
    t: &[f64],
    y: &[[f64; D]],
    tg: &[f64],
    sigma: f64,
    w: &[f64],
) -> Vec<[f64; D]> {
    tg.iter()
        .map(|&g| {
            let mut s = [0.0; 5];
            let mut b = [[0.0; 3]; D];
            for (k, &ti) in t.iter().enumerate() {
                let dt = ti - g;
                let wk = (-0.5 * (dt / sigma).powi(2)).exp() * w[k];
                let mut pw = 1.0;
                for (e, se) in s.iter_mut().enumerate() {
                    *se += wk * pw;
                    if e < 3 {
                        for (j, bj) in b.iter_mut().enumerate() {
                            bj[e] += wk * y[k][j] * pw;
                        }
                    }
                    pw *= dt;
                }
            }
            let a = [
                [s[0] + 1e-9, s[1], s[2]],
                [s[1], s[2] + 1e-9, s[3]],
                [s[2], s[3], s[4] + 1e-9],
            ];
            let mut out = [0.0; D];
            for (j, o) in out.iter_mut().enumerate() {
                *o = solve3(a, b[j])[0];
            }
            out
        })
        .collect()
}

/// Solution of a 3x3 linear system (Gaussian elimination with partial
/// pivoting, as LAPACK `gesv`).
fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> [f64; 3] {
    for c in 0..3 {
        // the first largest pivot, as LAPACK `idamax`
        let mut p = c;
        for r in c + 1..3 {
            if a[r][c].abs() > a[p][c].abs() {
                p = r;
            }
        }
        a.swap(c, p);
        b.swap(c, p);
        let pivot = a[c];
        for r in c + 1..3 {
            let f = a[r][c] / pivot[c];
            for (x, pv) in a[r].iter_mut().zip(pivot).skip(c) {
                *x -= f * pv;
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; 3];
    for c in (0..3).rev() {
        let mut v = b[c];
        for k in c + 1..3 {
            v -= a[c][k] * x[k];
        }
        x[c] = v / a[c][c];
    }
    x
}

/// The arch line `(n, 3)` through the real cusps `cusps` in the order `seq`,
/// from the first to the last tooth, extended straight by 4 mm at the ends.
/// Robust weights per tooth (biweight of the mean residual of its cusps,
/// scale 6 x the median residual, at least 0.6 mm): a tooth out of the arch
/// does not drag the line. `anchors`: reference teeth (locked by the
/// clinician) that weigh 10 and are never down-weighted.
pub fn cusp_line(
    cusps: &BTreeMap<Fdi, Vec<DVec3>>,
    seq: &[Fdi],
    sigma: f64,
    anchors: &[Fdi],
) -> Vec<DVec3> {
    let (p, lab) = order_points(cusps, seq);
    if p.len() < 3 {
        return p;
    }
    let t = arc(&p);
    let tg = arange(t[0], t[t.len() - 1] + 1e-9, STEP);
    let anchor: Vec<bool> = lab.iter().map(|f| anchors.contains(f)).collect();
    let y: Vec<[f64; 3]> = p.iter().map(|q| q.to_array()).collect();
    let mut w: Vec<f64> = anchor
        .iter()
        .map(|&a| if a { ANCHOR_WEIGHT } else { 1.0 })
        .collect();
    let to_line =
        |rows: Vec<[f64; 3]>| -> Vec<DVec3> { rows.into_iter().map(DVec3::from).collect() };
    let mut l = to_line(local_quadratic(&t, &y, &tg, sigma, &w));
    let mut teeth: Vec<Fdi> = lab.clone();
    teeth.sort();
    teeth.dedup();
    for _ in 0..ROBUST {
        let r = offsets_3d(&l, &p);
        let rt: Vec<f64> = teeth
            .iter()
            .map(|f| {
                let v: Vec<f64> = r
                    .iter()
                    .zip(&lab)
                    .filter(|(_, g)| *g == f)
                    .map(|(x, _)| *x)
                    .collect();
                v.iter().sum::<f64>() / v.len() as f64
            })
            .collect();
        let scale = (6.0 * median(&rt)).max(MIN_SCALE);
        let wt: Vec<f64> = rt
            .iter()
            .map(|&x| {
                let u = clip(x / scale, 0.0, 1.0);
                (1.0 - u * u).powi(2) + 1e-3
            })
            .collect();
        w = lab
            .iter()
            .zip(&anchor)
            .map(|(f, &a)| {
                if a {
                    ANCHOR_WEIGHT
                } else {
                    wt[teeth.binary_search(f).unwrap_or(0)]
                }
            })
            .collect();
        l = to_line(local_quadratic(&t, &y, &tg, sigma, &w));
    }
    if EXT > 0.0 && l.len() >= 5 {
        let k = 2usize.max((1.0 / STEP).round() as usize);
        let n = l.len();
        let d0 = l[0] - l[k];
        let d1 = l[n - 1] - l[n - 1 - k];
        let d0 = d0 / d0.truncate().length().max(1e-9);
        let d1 = d1 / d1.truncate().length().max(1e-9);
        let h = arange(STEP, EXT + 1e-9, STEP);
        let mut out: Vec<DVec3> = h.iter().rev().map(|&hh| l[0] + hh * d0).collect();
        out.extend_from_slice(&l);
        out.extend(h.iter().map(|&hh| l[n - 1] + hh * d1));
        l = out;
    }
    l
}

// ---- positions along the arch -------------------------------------------------------------------------

/// Weighted isotonic regression (non-decreasing, pool adjacent violators),
/// then clipped to `[lo, hi]`.
pub fn isotonic(y: &[f64], w: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let (mut vals, mut wts, mut cnt): (Vec<f64>, Vec<f64>, Vec<usize>) =
        (Vec::new(), Vec::new(), Vec::new());
    for (&yi, &wi) in y.iter().zip(w) {
        let (mut v, mut ww, mut n) = (yi, wi, 1);
        while let Some(&last) = vals.last() {
            if last <= v {
                break;
            }
            let (pv, pw, pn) = (
                vals.pop().unwrap_or(last),
                wts.pop().unwrap_or(0.0),
                cnt.pop().unwrap_or(0),
            );
            v = (pv * pw + v * ww) / (pw + ww);
            ww += pw;
            n += pn;
        }
        vals.push(v);
        wts.push(ww);
        cnt.push(n);
    }
    vals.iter()
        .zip(&cnt)
        .flat_map(|(&v, &n)| std::iter::repeat_n(clip(v, lo, hi), n))
        .collect()
}

/// A stretch between two fixed teeth (indices, `-1` / `n` when open) whose
/// fixed teeth are out of order by `deficit` mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crowded {
    /// Fixed tooth on the left (`-1`: open).
    pub left: i64,
    /// Fixed tooth on the right (`n`: open).
    pub right: i64,
    /// Missing space (mm).
    pub deficit: f64,
}

/// Result of [`solve_chain`].
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    /// Positions, non-decreasing within each stretch.
    pub u: Vec<f64>,
    /// Total missing space (mm).
    pub deficit: f64,
    /// The crowded stretches.
    pub segs: Vec<Crowded>,
    /// Free space of the closed stretches between two fixed teeth (mm), with
    /// `pack`.
    pub spacing: f64,
}

/// The non-decreasing `u` nearest to `u0` (weights `w`) with the `fixed`
/// teeth held. With `pack` the free teeth of a stretch stay together (one
/// `u` for the whole stretch: the weighted mean, within the fixed teeth;
/// the spaces close).
pub fn solve_chain(u0: &[f64], w: &[f64], fixed: &[bool], pack: bool) -> Chain {
    let n = u0.len();
    let mut u = u0.to_vec();
    let mut bounds: Vec<i64> = vec![-1];
    bounds.extend((0..n).filter(|&i| fixed[i]).map(|i| i as i64));
    bounds.push(n as i64);
    let (mut total, mut segs, mut spacing) = (0.0, Vec::new(), 0.0);
    for ab in bounds.windows(2) {
        let (a, b) = (ab[0], ab[1]);
        let lo = if a >= 0 {
            u0[a as usize]
        } else {
            f64::NEG_INFINITY
        };
        let hi = if b < n as i64 {
            u0[b as usize]
        } else {
            f64::INFINITY
        };
        let idx: Vec<usize> = ((a + 1)..b).map(|i| i as usize).collect();
        // two fixed teeth side by side (no free tooth between them) do not move: their overlap
        // at rest is not crowding to resolve
        let deficit = if lo.is_finite() && hi.is_finite() && !idx.is_empty() {
            (lo - hi).max(0.0)
        } else {
            0.0
        };
        if deficit > 1e-9 {
            total += deficit;
            segs.push(Crowded {
                left: a,
                right: b,
                deficit,
            });
        }
        if idx.is_empty() {
            continue;
        }
        let y: Vec<f64> = idx.iter().map(|&i| u0[i]).collect();
        let ww: Vec<f64> = idx.iter().map(|&i| w[i]).collect();
        if deficit > 1e-9 {
            let v = isotonic(&y, &ww, f64::NEG_INFINITY, f64::INFINITY);
            for (k, &i) in idx.iter().enumerate() {
                u[i] = clip(v[k], hi, lo);
            }
        } else if pack {
            let num: f64 = y.iter().zip(&ww).map(|(a, b)| a * b).sum();
            let den: f64 = ww.iter().sum();
            let m = clip(num / den, lo, hi);
            for &i in &idx {
                u[i] = m;
            }
            if lo.is_finite() && hi.is_finite() {
                spacing += hi - lo;
            }
        } else {
            let v = isotonic(&y, &ww, lo, hi);
            for (k, &i) in idx.iter().enumerate() {
                u[i] = v[k];
            }
        }
    }
    Chain {
        u,
        deficit: total,
        segs,
        spacing,
    }
}
