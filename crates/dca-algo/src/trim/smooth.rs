//! The SMOOTH border, the border of a finished aligner (`aligner.smooth_border`,
//! `_arch_line`, `_gauss_rows`; `case_print.smooth_trim_points`): the gingival
//! margin taken along the line of the arch, side by side, smoothed into a
//! line without steps with gentle scallops, `gap` mm above the margin; round
//! the distal face of the last tooth covered of each side; nothing kept
//! behind it and over the teeth left out.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use glam::{DVec2, DVec3};

use super::survey::median;
use crate::reconstruct::TAPER_RIM_MM;

/// The control points along the arch every this many mm ...
pub const SMOOTH_STEP_MM: f64 = 0.5;
/// ... the line under the scallops: the margin smoothed along the arch
/// (gaussian, mm) ...
pub const SMOOTH_BASE_SIGMA_MM: f64 = 2.5;
/// ... the scallops (held within +-scallop / 2) smoothed ...
pub const SMOOTH_WAVE_SIGMA_MM: f64 = 0.8;
/// ... the distance of the margin from the arch line smoothed (plan) ...
pub const SMOOTH_DEPTH_SIGMA_MM: f64 = 1.5;
/// ... a second ring of points this much farther from the arch ...
pub const SMOOTH_RING_MM: f64 = 1.5;
/// ... and behind the last tooth covered, past the shell (outer + this),
/// nothing kept ...
pub const SMOOTH_END_MM: f64 = 0.6;
/// ... the border goes round the distal face of that tooth: its points fill
/// the last this many mm of the crown and the wall behind it ...
pub const SMOOTH_CAP_IN_MM: f64 = 3.0;
/// ... at the height of the margin of the distal face (its points within
/// this of the end).
pub const SMOOTH_CAP_MARGIN_MM: f64 = 1.5;
/// The shell reaches this (+ wall, + the gap below the margin) from the
/// crowns covered (`case_print.SMOOTH_NEAR_MM`).
pub const SMOOTH_NEAR_MM: f64 = 3.5;
/// ... from the crown of the last tooth covered of each side only this (no
/// papilla under a contact past it; `trim::smooth_near`).
pub const SMOOTH_END_NEAR_MM: f64 = 1.0;

/// `y` smoothed by a gaussian of `sigma` samples, the weights renormalised
/// at the ends (`_gauss_rows`).
pub fn gauss_rows(y: &[f64], sigma: f64) -> Vec<f64> {
    let s = sigma.max(1e-6);
    (0..y.len())
        .map(|i| {
            let (mut num, mut den) = (0.0, 0.0);
            for (j, &v) in y.iter().enumerate() {
                let w = (-0.5 * ((i as f64 - j as f64) / s).powi(2)).exp();
                num += w * v;
                den += w;
            }
            num / den
        })
        .collect()
}

/// Height and depth of the border at one end of a side.
type SideEnd = (f64, f64);

/// `np.arange(start, stop, step)` for floats.
fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil();
    if n.is_nan() || n <= 0.0 {
        return Vec::new();
    }
    (0..n as usize).map(|i| start + i as f64 * step).collect()
}

/// `np.interp` (non-periodic, clamped), `xp` increasing.
fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if x <= xp[0] {
        return fp[0];
    }
    if x >= xp[n - 1] {
        return fp[n - 1];
    }
    let j = xp.partition_point(|&v| v <= x);
    let (x0, x1, f0, f1) = (xp[j - 1], xp[j], fp[j - 1], fp[j]);
    if x1 - x0 <= 0.0 {
        f0
    } else {
        f0 + (x - x0) * (f1 - f0) / (x1 - x0)
    }
}

/// `np.percentile(v, q)` (linear).
fn percentile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = q / 100.0 * (s.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
}

/// `np.searchsorted(u, x)` (left).
fn searchsorted(u: &[f64], x: f64) -> usize {
    u.partition_point(|&v| v < x)
}

/// The line of the arch (`_arch_line`): through the centres of the crowns
/// in arch order, rounded (Chaikin, the ends kept), prolonged by `extend`
/// mm past both ends and resampled every `step` mm. Returns the points, the
/// unit tangents and the arc length.
pub fn arch_line(c: &[DVec2], step: f64, extend: f64) -> (Vec<DVec2>, Vec<DVec2>, Vec<f64>) {
    let mut p = c.to_vec();
    for _ in 0..3 {
        if p.len() < 3 {
            break;
        }
        let mut q = vec![p[0]];
        for w in p.windows(2) {
            q.push(0.75 * w[0] + 0.25 * w[1]);
            q.push(0.25 * w[0] + 0.75 * w[1]);
        }
        q.push(p[p.len() - 1]);
        p = q;
    }
    let n = p.len();
    let t0 = p[0] - p[1];
    let t1 = p[n - 1] - p[n - 2];
    let mut pts = vec![p[0] + extend * t0 / t0.length().max(1e-9)];
    pts.extend_from_slice(&p);
    pts.push(p[n - 1] + extend * t1 / t1.length().max(1e-9));
    let mut s = vec![0.0];
    for w in pts.windows(2) {
        s.push(s[s.len() - 1] + (w[1] - w[0]).length());
    }
    let u = arange(0.0, s[s.len() - 1] + 1e-9, step);
    let xs: Vec<f64> = pts.iter().map(|q| q.x).collect();
    let ys: Vec<f64> = pts.iter().map(|q| q.y).collect();
    let l: Vec<DVec2> = u
        .iter()
        .map(|&v| DVec2::new(interp(v, &s, &xs), interp(v, &s, &ys)))
        .collect();
    let m = l.len();
    let tn: Vec<DVec2> = (0..m)
        .map(|i| {
            let g = if m < 2 {
                DVec2::ZERO
            } else if i == 0 {
                l[1] - l[0]
            } else if i == m - 1 {
                l[m - 1] - l[m - 2]
            } else {
                0.5 * (l[i + 1] - l[i - 1])
            };
            g / g.length().max(1e-9)
        })
        .collect();
    (l, tn, u)
}

fn nearest(l: &[DVec2], p: DVec2) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (j, q) in l.iter().enumerate() {
        let d = (p - *q).length_squared();
        if d < best.0 {
            best = (d, j);
        }
    }
    best.1
}

/// The point of the line of the arch `l` (normals `nn`, arc length `u`) at
/// arc length `s` and its normal: the sample at or after `s` (`searchsorted`);
/// past the ends of the line, on its straight prolongation.
fn along(l: &[DVec2], nn: &[DVec2], u: &[f64], s: f64) -> (DVec2, DVec2) {
    let n = l.len();
    if n >= 2 && s > u[n - 1] {
        let t = (l[n - 1] - l[n - 2]).normalize_or_zero();
        return (l[n - 1] + (s - u[n - 1]) * t, nn[n - 1]);
    }
    if n >= 2 && s < u[0] {
        let t = (l[0] - l[1]).normalize_or_zero();
        return (l[0] + (u[0] - s) * t, nn[0]);
    }
    let j = searchsorted(u, s).min(n - 1);
    (l[j], nn[j])
}

/// The margin `marg` (arch order) with the distal half of the last tooth of
/// each side kept only at the gum: within [`TAPER_RIM_MM`] of the lowest
/// point of its margin (`reconstruct.TAPER_RIM_MM`: the gingival margin, not
/// the cut up the side of the crown). No tooth covered follows there, so a
/// margin higher up is the cut along what the scan has behind the crown at
/// its height (a flange of gingiva or of the base, a tooth left out): the
/// border taken from it would lie over the distal face, cut a notch into the
/// cap and graze the shell over the flange into ragged tongues. `l`, `u`:
/// the line of the arch and its arc length; `cen`: the centres of the crowns
/// of `marg`.
fn gum_ends(
    marg: &[(Fdi, Vec<DVec3>)],
    l: &[DVec2],
    u: &[f64],
    cen: &[DVec2],
) -> Vec<(Fdi, Vec<DVec3>)> {
    let mut out = marg.to_vec();
    let last = out.len() - 1;
    for (i, dirn) in [(0, -1.0), (last, 1.0)] {
        let sc = u[nearest(l, cen[i])];
        let p = &out[i].1;
        let floor = p.iter().map(|q| q.z).fold(f64::INFINITY, f64::min) + TAPER_RIM_MM;
        let kept: Vec<DVec3> = p
            .iter()
            .copied()
            .filter(|q| q.z <= floor || (u[nearest(l, q.truncate())] - sc) * dirn <= 0.0)
            .collect();
        out[i].1 = kept;
    }
    out
}

/// Control points of the smooth border (`aligner.smooth_border`). `marg`:
/// the gingival margin of every tooth covered with one, in arch order;
/// `crowns`: the crown points of every tooth of the case; `covered`: the
/// teeth the aligner covers; `outer`: clearance + wall; `gap`: the border
/// above the margin (towards the crown; negative on the gingiva);
/// `scallop`: the height of the scallops. The distal half of the last tooth
/// of each side counts only its margin at the gum ([`gum_ends`]). Returns
/// the border points and the high points (nothing kept under them), or
/// `None` with fewer than two teeth with a margin.
#[allow(clippy::too_many_lines)]
pub fn smooth_border(
    marg: &[(Fdi, Vec<DVec3>)],
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
    covered: &BTreeSet<Fdi>,
    outer: f64,
    gap: f64,
    scallop: f64,
) -> Option<(Vec<DVec3>, Vec<DVec3>)> {
    if marg.len() < 2 {
        return None;
    }
    let cen: Vec<DVec2> = marg
        .iter()
        .map(|(f, _)| {
            let c = &crowns[f];
            c.iter().map(|p| p.truncate()).sum::<DVec2>() / c.len() as f64
        })
        .collect();
    let (l, tn, u) = arch_line(&cen, 0.25, 8.0);
    let mut nn: Vec<DVec2> = tn.iter().map(|t| DVec2::new(t.y, -t.x)).collect();
    let lm = l.iter().sum::<DVec2>() / l.len() as f64;
    let side: f64 = l
        .iter()
        .zip(&nn)
        .map(|(p, n)| (*p - lm).dot(*n))
        .sum::<f64>()
        / l.len() as f64;
    if side < 0.0 {
        for n in &mut nn {
            *n = -*n; // +N: labial (out of the arch)
        }
    }
    let marg = &gum_ends(marg, &l, &u, &cen);
    let m: Vec<DVec3> = marg.iter().flat_map(|(_, p)| p.iter().copied()).collect();
    let near_m: Vec<usize> = m.iter().map(|p| nearest(&l, p.truncate())).collect();
    let sm: Vec<f64> = near_m.iter().map(|&j| u[j]).collect();
    let dm: Vec<f64> = m
        .iter()
        .zip(&near_m)
        .map(|(p, &j)| (p.truncate() - l[j]).dot(nn[j]))
        .collect();
    let proj = |p: &DVec3| u[nearest(&l, p.truncate())];
    let ca: Vec<f64> = marg
        .iter()
        .flat_map(|(f, _)| crowns[f].iter().map(proj))
        .collect();
    let s0 = ca.iter().copied().fold(f64::INFINITY, f64::min);
    let s1 = ca.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let amp = 0.5 * scallop;
    let step = SMOOTH_STEP_MM;

    // where the shell ends behind the last tooth covered of each side
    let behind: Vec<Vec<f64>> = crowns
        .iter()
        .filter(|(g, p)| !covered.contains(g) && !p.is_empty())
        .map(|(_, p)| p.iter().map(proj).collect())
        .collect();
    let mut ends = Vec::new();
    for (f, se, dirn) in [(marg[0].0, s0, -1.0), (marg[marg.len() - 1].0, s1, 1.0)] {
        let mut room = outer;
        for sp in &behind {
            let sp: Vec<f64> = sp.iter().map(|s| s * dirn - se * dirn).collect();
            let share = sp.iter().filter(|&&v| v > -1.0).count() as f64 / sp.len() as f64;
            if share > 0.5 {
                let m0 = sp.iter().map(|v| v.max(0.0)).fold(f64::INFINITY, f64::min);
                room = room.min((m0 * 0.5).max(0.0));
            }
        }
        ends.push((f, se, dirn, se + dirn * room));
    }
    let smin = sm.iter().copied().fold(f64::INFINITY, f64::min);
    let smax = sm.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let grid = arange((smin / step).floor() * step, smax + step, step);
    let mut out: Vec<DVec3> = Vec::new();
    let mut side_ends: BTreeMap<i8, (SideEnd, SideEnd)> = BTreeMap::new();
    for sign in [1.0f64, -1.0] {
        let sel: Vec<usize> = (0..m.len()).filter(|&i| dm[i] * sign > 0.0).collect();
        if sel.len() < 6 {
            continue;
        }
        let ng = grid.len();
        let b: Vec<usize> = sel
            .iter()
            .map(|&i| {
                let v = ((sm[i] - grid[0]) / step).round_ties_even();
                (v.max(0.0) as usize).min(ng - 1)
            })
            .collect();
        let mut z = vec![f64::NAN; ng];
        let mut d = vec![f64::NAN; ng];
        let mut bins: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (k, &bi) in b.iter().enumerate() {
            bins.entry(bi).or_default().push(sel[k]);
        }
        for (&bi, ix) in &bins {
            let mut zs: Vec<f64> = ix.iter().map(|&i| m[i].z).collect();
            z[bi] = median(&mut zs);
            let ds: Vec<f64> = ix.iter().map(|&i| dm[i].abs()).collect();
            d[bi] = percentile(&ds, 75.0);
        }
        let ok: Vec<bool> = z.iter().map(|v| v.is_finite()).collect();
        let lo_i = ok.iter().position(|&o| o).unwrap_or(0);
        let hi_i = ng - ok.iter().rev().position(|&o| o).unwrap_or(0);
        let gx: Vec<f64> = (0..ng).filter(|&i| ok[i]).map(|i| grid[i]).collect();
        let gz: Vec<f64> = (0..ng).filter(|&i| ok[i]).map(|i| z[i]).collect();
        let gd: Vec<f64> = (0..ng).filter(|&i| ok[i]).map(|i| d[i]).collect();
        let g: Vec<f64> = grid[lo_i..hi_i].to_vec();
        let z: Vec<f64> = g.iter().map(|&x| interp(x, &gx, &gz)).collect();
        let d: Vec<f64> = g.iter().map(|&x| interp(x, &gx, &gd)).collect();
        let base = gauss_rows(&z, SMOOTH_BASE_SIGMA_MM / step);
        let dev: Vec<f64> = z
            .iter()
            .zip(&base)
            .map(|(a, b)| (a - b).clamp(-amp, amp))
            .collect();
        let wave = gauss_rows(&dev, SMOOTH_WAVE_SIGMA_MM / step);
        let zb: Vec<f64> = base.iter().zip(&wave).map(|(a, w)| a + w + gap).collect();
        let db = gauss_rows(&d, SMOOTH_DEPTH_SIGMA_MM / step);
        let nb = zb.len();
        side_ends.insert(sign as i8, ((zb[0], db[0]), (zb[nb - 1], db[nb - 1])));
        // prolonged at the ends to the end of the shell
        let mut ext_lo = arange(g[0] - step, ends[0].3 - step, -step);
        ext_lo.reverse();
        let ext_hi = arange(g[g.len() - 1] + step, ends[1].3 + step, step);
        let mut gg = ext_lo.clone();
        gg.extend_from_slice(&g);
        gg.extend_from_slice(&ext_hi);
        let mut zz = vec![zb[0]; ext_lo.len()];
        zz.extend_from_slice(&zb);
        zz.extend(std::iter::repeat_n(zb[nb - 1], ext_hi.len()));
        let mut dd = vec![db[0]; ext_lo.len()];
        dd.extend_from_slice(&db);
        dd.extend(std::iter::repeat_n(db[nb - 1], ext_hi.len()));
        for off in [0.0, SMOOTH_RING_MM] {
            for k in 0..gg.len() {
                let (p, n) = along(&l, &nn, &u, gg[k]);
                let xy = p + sign * (dd[k] + off) * n;
                out.push(xy.extend(zz[k]));
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    // round the distal face of the last tooth covered of each side
    for (e, &(f, se, dirn, sc)) in ends.iter().enumerate() {
        let (lab, lin) = (side_ends.get(&1), side_ends.get(&-1));
        let pick = |s: &(SideEnd, SideEnd)| if e == 0 { s.0 } else { s.1 };
        let (zr, dr) = pick(lab.or(lin).unwrap());
        let (zl, dl) = pick(lin.or(lab).unwrap());
        let p = &marg.iter().find(|(g, _)| *g == f).unwrap().1;
        let j: Vec<usize> = p.iter().map(|q| nearest(&l, q.truncate())).collect();
        let back: Vec<f64> = j.iter().map(|&jj| (u[jj] - se) * dirn).collect();
        let bmax = back.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let on: Vec<usize> = (0..p.len())
            .filter(|&i| back[i] >= bmax - SMOOTH_CAP_MARGIN_MM)
            .collect();
        let pd: Vec<f64> = on
            .iter()
            .map(|&i| (p[i].truncate() - l[j[i]]).dot(nn[j[i]]))
            .collect();
        let cols = arange(-dl, dr + 1e-9, step);
        let line: Vec<f64> = if dr + dl > 1e-9 {
            cols.iter()
                .map(|&c| interp(c, &[-dl, dr], &[zl, zr]))
                .collect()
        } else {
            vec![zl; cols.len()]
        };
        let rb: Vec<i64> = pd
            .iter()
            .map(|v| (v / step).round_ties_even() as i64)
            .collect();
        let mut uniq = rb.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let (mut kd, mut kz) = (vec![-dl], vec![zl]);
        for &bb in &uniq {
            let x = bb as f64 * step;
            if -dl < x && x < dr {
                let mut zs: Vec<f64> = on
                    .iter()
                    .zip(&rb)
                    .filter(|&(_, &r)| r == bb)
                    .map(|(&i, _)| p[i].z)
                    .collect();
                kd.push(x);
                kz.push(median(&mut zs) + gap);
            }
        }
        kd.push(dr);
        kz.push(zr);
        let zc: Vec<f64> = if kd.len() > 2 {
            cols.iter().map(|&c| interp(c, &kd, &kz)).collect()
        } else {
            line.clone()
        };
        let dev: Vec<f64> = zc
            .iter()
            .zip(&line)
            .map(|(a, b)| (a - b).clamp(-amp, amp))
            .collect();
        let wave = gauss_rows(&dev, SMOOTH_WAVE_SIGMA_MM / step);
        let zc: Vec<f64> = line.iter().zip(&wave).map(|(a, w)| a + w).collect();
        let mut rows = arange(-SMOOTH_CAP_IN_MM, (sc - se) * dirn, step);
        rows.push((sc - se) * dirn);
        for r in rows {
            let (p, n) = along(&l, &nn, &u, se + dirn * r);
            for (c, z) in cols.iter().zip(&zc) {
                out.push((p + *c * n).extend(*z));
            }
        }
    }
    // behind the last tooth covered and over the teeth left out: high points
    // (past the end of the line on its prolongation: the plugin skips those
    // rows, and behind a long last molar none was left, the shell running on
    // over what the scan has behind it to the reach of the crowns)
    let ctop = marg
        .iter()
        .flat_map(|(f, _)| crowns[f].iter().map(|p| p.z))
        .fold(f64::NEG_INFINITY, f64::max);
    let top = ctop + 10.0;
    let mut high = Vec::new();
    for (sa, dirn) in [
        (ends[0].3 - SMOOTH_END_MM, -1.0),
        (ends[1].3 + SMOOTH_END_MM, 1.0),
    ] {
        for k in 0..8 {
            let (p, n) = along(&l, &nn, &u, sa + dirn * f64::from(k));
            for dd in arange(-12.0, 12.01, 1.0) {
                high.push((p + dd * n).extend(top));
            }
        }
    }
    for (f, p) in crowns {
        if !covered.contains(f) {
            high.extend(p.iter().map(|q| q.truncate().extend(top)));
        }
    }
    Some((out, high))
}
