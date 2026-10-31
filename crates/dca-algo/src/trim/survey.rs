//! Closed loops around a crown seen along the insertion path: the survey
//! line (height of contour) and the operations on loop heights that the
//! equator border applies (`aligner.survey_loop`, `aligner.loop_z`,
//! `aligner.loop_nearest`, `case_print._circular_smooth`, `_slope_floor`,
//! `_loop_smooth_plan`, `_round_tabs`, `_bridge_contacts`).

use std::f64::consts::{PI, TAU};

use glam::{DVec2, DVec3};

/// Survey line of the crown points `p` seen along `+Z`.
#[derive(Debug, Clone, PartialEq)]
pub struct Survey {
    /// One point per direction: the plan position of the extreme point
    /// along it and the mean height of the points near it.
    pub loop_points: Vec<DVec3>,
    /// Extent of the crown along every direction from the centre.
    pub extent: Vec<f64>,
    /// The horizontal unit directions (`n`, counter-clockwise from `+X`).
    pub dirs: Vec<DVec2>,
    /// Plan centre.
    pub centre: DVec2,
}

/// Survey line (height of contour) of the crown points `p` seen along `+Z`
/// (`aligner.survey_loop`): for `n` horizontal directions around the plan
/// centre, the plan position of the extreme point along it and the mean
/// height of the points within `band` mm of the extreme along it and
/// `side` mm of it across (robust to the noise of a scan). `centre`: the
/// plan centre (default: the mean of the points).
pub fn survey_loop(p: &[DVec3], n: usize, band: f64, side: f64, centre: Option<DVec2>) -> Survey {
    let c = centre.unwrap_or_else(|| {
        let s: DVec2 = p.iter().map(|q| q.truncate()).sum();
        s / p.len().max(1) as f64
    });
    let mut loop_points = Vec::with_capacity(n);
    let mut extent = Vec::with_capacity(n);
    let mut dirs = Vec::with_capacity(n);
    for k in 0..n {
        let a = TAU * k as f64 / n as f64;
        let d = DVec2::new(a.cos(), a.sin());
        let w = DVec2::new(-d.y, d.x);
        // the first point of largest extent (numpy argmax)
        let mut top = 0;
        let mut best = f64::NEG_INFINITY;
        for (i, q) in p.iter().enumerate() {
            let s = (q.truncate() - c).dot(d);
            if s > best {
                best = s;
                top = i;
            }
        }
        let wt = (p[top].truncate() - c).dot(w);
        let (mut sum, mut cnt) = (0.0, 0usize);
        for q in p {
            let r = q.truncate() - c;
            if r.dot(d) >= best - band && (r.dot(w) - wt).abs() <= side {
                sum += q.z;
                cnt += 1;
            }
        }
        loop_points.push(DVec3::new(p[top].x, p[top].y, sum / cnt.max(1) as f64));
        extent.push(best);
        dirs.push(d);
    }
    Survey {
        loop_points,
        extent,
        dirs,
        centre: c,
    }
}

/// `np.interp(x, xp, fp, period)` for sorted-or-not abscissae `xp`.
pub(crate) fn interp_periodic(x: &[f64], xp: &[f64], fp: &[f64], period: f64) -> Vec<f64> {
    let mut pts: Vec<(f64, f64)> = xp
        .iter()
        .zip(fp)
        .map(|(&a, &f)| (a.rem_euclid(period), f))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let m = pts.len();
    if m == 0 {
        return vec![f64::NAN; x.len()];
    }
    let mut ext = Vec::with_capacity(m + 2);
    ext.push((pts[m - 1].0 - period, pts[m - 1].1));
    ext.extend_from_slice(&pts);
    ext.push((pts[0].0 + period, pts[0].1));
    x.iter()
        .map(|&q| {
            let q = q.rem_euclid(period);
            // the first abscissa above q
            let j = ext.partition_point(|e| e.0 <= q).clamp(1, ext.len() - 1);
            let (x0, f0) = ext[j - 1];
            let (x1, f1) = ext[j];
            if x1 - x0 <= 0.0 {
                f0
            } else {
                f0 + (q - x0) * (f1 - f0) / (x1 - x0)
            }
        })
        .collect()
}

/// Height of the closed loop `e` at the azimuth of the plan points `p`
/// around the plan point `c` (`aligner.loop_z`: periodic linear
/// interpolation).
pub fn loop_z(e: &[DVec3], c: DVec2, p: &[DVec2]) -> Vec<f64> {
    let a: Vec<f64> = e.iter().map(|q| (q.y - c.y).atan2(q.x - c.x)).collect();
    let z: Vec<f64> = e.iter().map(|q| q.z).collect();
    let q: Vec<f64> = p.iter().map(|q| (q.y - c.y).atan2(q.x - c.x)).collect();
    interp_periodic(&q, &a, &z, TAU)
}

/// Plan point `p` against the closed loop `e` (`aligner.loop_nearest`):
/// signed distance (depth inside the loop, minus the distance outside) and
/// the height of the loop at its nearest point.
pub fn loop_nearest(e: &[DVec3], p: DVec2) -> (f64, f64) {
    let n = e.len();
    let (mut best, mut zbest) = (f64::INFINITY, f64::NAN);
    let mut crossings = 0u32;
    for i in 0..n {
        let a = e[i];
        let b = e[(i + 1) % n];
        let ab = b.truncate() - a.truncate();
        let l2 = ab.length_squared().max(1e-12);
        let r = p - a.truncate();
        let t = (r.dot(ab) / l2).clamp(0.0, 1.0);
        let d = (r - t * ab).length();
        if d < best {
            best = d;
            zbest = a.z + t * (b.z - a.z);
        }
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * ab.x / ab.y {
            crossings += 1;
        }
    }
    let depth = if crossings % 2 == 1 { best } else { -best };
    (depth, zbest)
}

/// Plan points of a closed loop around `c` smoothed along the loop by
/// `passes` passes of the (1, 2, 1) / 4 filter (`_loop_smooth_plan`).
pub fn loop_smooth_plan(q: &[DVec2], c: DVec2, passes: usize) -> Vec<DVec2> {
    let n = q.len();
    let mut cur: Vec<DVec2> = q.iter().map(|&p| p - c).collect();
    for _ in 0..passes {
        cur = (0..n)
            .map(|i| 0.25 * (cur[(i + n - 1) % n] + 2.0 * cur[i] + cur[(i + 1) % n]))
            .collect();
    }
    cur.into_iter().map(|p| p + c).collect()
}

/// Heights of a closed loop raised, never lowered, so that two neighbours
/// differ at most `slope` (`_slope_floor`: the upper envelope of cones of
/// that slope on every height; `slope <= 0`: unchanged).
pub fn slope_floor(z: &[f64], slope: f64) -> Vec<f64> {
    let n = z.len();
    if slope <= 0.0 || n < 3 {
        return z.to_vec();
    }
    (0..n)
        .map(|k| {
            (0..n)
                .map(|j| {
                    let d = k.abs_diff(j);
                    z[j] - slope * d.min(n - d) as f64
                })
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect()
}

/// Loop heights smoothed along the loop: running median of `k`, then mean
/// of 3 (`_circular_smooth`).
pub fn circular_smooth(z: &[f64], k: usize) -> Vec<f64> {
    let n = z.len();
    if n < k {
        return z.to_vec();
    }
    let h = (k / 2) as isize;
    let m: Vec<f64> = (0..n as isize)
        .map(|i| {
            let mut w: Vec<f64> = (-h..=h)
                .map(|o| z[(i + o).rem_euclid(n as isize) as usize])
                .collect();
            median(&mut w)
        })
        .collect();
    (0..n)
        .map(|i| (m[(i + n - 1) % n] + m[i] + m[(i + 1) % n]) / 3.0)
        .collect()
}

/// Median as numpy computes it (mean of the two middle values).
pub(crate) fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Heights of the closed loop `e` raised so that the crown it leaves
/// covered (above it) has no tab narrower than `2 r` (`_round_tabs`): the
/// opening, by a disk of radius `r`, of the region above the loop developed
/// along its length and its height; the disks rest on the loop `fine` times
/// more finely than its points (linear between them).
pub fn round_tabs(e: &[DVec3], r: f64, fine: usize) -> Vec<f64> {
    let n = e.len();
    if r <= 0.0 || n < 3 {
        return e.iter().map(|p| p.z).collect();
    }
    let k = fine.max(1);
    if k > 1 {
        let f: Vec<DVec3> = (0..n)
            .flat_map(|i| {
                let a = e[i];
                let b = e[(i + 1) % n];
                (0..k).map(move |j| a + (b - a) * (j as f64 / k as f64))
            })
            .collect();
        let zf = round_tabs(&f, r, 1);
        return (0..n).map(|i| e[i].z.max(zf[i * k])).collect();
    }
    let mut t = Vec::with_capacity(n);
    let mut acc = 0.0;
    for i in 0..n {
        t.push(acc);
        acc += (e[(i + 1) % n].truncate() - e[i].truncate()).length();
    }
    let perim = acc;
    let dist = |i: usize, j: usize| {
        let d = (t[i] - t[j]).abs();
        d.min(perim - d)
    };
    // lowest centre of a disk above the loop at every point
    let g: Vec<f64> = (0..n)
        .map(|i| {
            (0..n)
                .filter_map(|j| {
                    let d = dist(i, j);
                    (d <= r).then(|| e[j].z + (r * r - d * d).max(0.0).sqrt())
                })
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();
    (0..n)
        .map(|i| {
            let z = (0..n)
                .filter_map(|j| {
                    let d = dist(i, j);
                    (d <= r).then(|| g[j] - (r * r - d * d).max(0.0).sqrt())
                })
                .fold(f64::INFINITY, f64::min);
            e[i].z.max(z)
        })
        .collect()
}

/// The equator loops for the trim with every stretch at a contact (within
/// `near`, plan, of another loop) at the heights interpolated along it
/// between its two ends (`_bridge_contacts`): the trim crosses the
/// embrasure under the contact instead of rising to it.
pub fn bridge_contacts(loops: &[Vec<DVec3>], near: f64) -> Vec<Vec<DVec3>> {
    let mut out = Vec::with_capacity(loops.len());
    for (i, e) in loops.iter().enumerate() {
        let mut e = e.clone();
        let n = e.len();
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for p in &e {
            lo = lo.min(p.truncate());
            hi = hi.max(p.truncate());
        }
        lo -= DVec2::splat(near);
        hi += DVec2::splat(near);
        let others: Vec<DVec2> = loops
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .flat_map(|(_, f)| f.iter().map(|p| p.truncate()))
            .filter(|o| o.cmpge(lo).all() && o.cmple(hi).all())
            .collect();
        if others.is_empty() {
            out.push(e);
            continue;
        }
        // single precision distances, as the KD-tree of the plugin
        let at: Vec<bool> = e
            .iter()
            .map(|p| {
                let q = p.truncate().as_vec2();
                others
                    .iter()
                    .map(|o| (o.as_vec2() - q).length())
                    .fold(f32::INFINITY, f32::min) as f64
                    <= near
            })
            .collect();
        if at.iter().all(|&a| a) || !at.iter().any(|&a| a) {
            out.push(e);
            continue;
        }
        let seg: Vec<f64> = (0..n)
            .map(|k| (e[(k + 1) % n].truncate() - e[k].truncate()).length())
            .collect();
        let start = at.iter().position(|&a| !a).unwrap_or(0);
        let mut k = 0;
        while k < n {
            if !at[(start + k) % n] {
                k += 1;
                continue;
            }
            let mut j = k;
            while j < n && at[(start + j) % n] {
                j += 1;
            }
            let a = (start + k + n - 1) % n;
            let b = (start + j) % n;
            let mut s_run = Vec::with_capacity(j - k + 1);
            let mut acc = 0.0;
            for m in 0..=(j - k) {
                acc += seg[(a + m) % n];
                s_run.push(acc);
            }
            let total = acc;
            let (za, zb) = (e[a].z, e[b].z);
            for (m, &s) in s_run.iter().take(j - k).enumerate() {
                let ii = (start + k + m) % n;
                let t = if total > 0.0 { s / total } else { 0.5 };
                e[ii].z = (1.0 - t) * za + t * zb;
            }
            k = j;
        }
        out.push(e);
    }
    out
}

/// Smallest angular difference of two azimuths (radians, in `[0, π]`).
pub(crate) fn angle_diff(a: f64, b: f64) -> f64 {
    ((a - b + PI).rem_euclid(TAU) - PI).abs()
}
