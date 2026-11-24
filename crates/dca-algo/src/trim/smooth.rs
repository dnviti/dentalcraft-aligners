//! The SMOOTH border, the border of a finished aligner (`aligner.smooth_border`,
//! `_arch_line`, `_gauss_rows`; `case_print.smooth_trim_points`): the gingival
//! margin taken along the line of the arch, side by side, smoothed into a
//! line without steps with gentle scallops, `gap` mm above the margin; round
//! the distal face of the last tooth covered of each side; nothing kept
//! behind it and over the teeth left out.
//!
//! Papillae (not in the plugin): the scallops are held within
//! `±scallop / 2` of the smoothed margin, but a papilla rises 1-4 mm above
//! it, so the border of the plugin comes down between the teeth onto the
//! tip of every papilla (a real case: 0.4-0.9 mm above it, or up to 0.6 mm
//! on it, against 2 mm above the margin in the middle of the teeth). With a
//! `relief` the border is raised wherever it is less than that above the
//! margin, and above the tip of every papilla within [`PAPILLA_TOP_MM`] of
//! it along the arch ([`papilla_tops`]), in a rounded notch round the
//! papilla ([`papilla_lift`]), never within [`PAPILLA_NECK_MM`] of the top
//! of the contact of the two crowns ([`contacts`]).

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use dc_geom::knn::KnnIndex;
use glam::{DVec2, DVec3, Vec3};

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
/// Default of [`super::Params::papilla_relief_mm`]: the edge stays this far
/// above the gingival margin (mm) all along the arch, round every papilla.
pub const PAPILLA_RELIEF_MM: f64 = 1.0;
/// The tip of the papilla under a contact is the highest margin point of a
/// side within this distance (mm) along the arch of the contact, and the
/// border stands the relief above it as far on either side of it
/// ([`papilla_tops`]): the edge crosses the gap between the crowns in the
/// plane of their contact, which meets the line of the arch at a slant, and
/// the trim surface between the two rings of its points takes the height
/// of the nearest of them, so the edge passes over the papilla up to this
/// far along the arch from the tip (three real arches: 0.5 to 1.5 mm, the
/// edge 0.3 mm above the tip there against 1 mm at the tip's own place).
pub const PAPILLA_TOP_MM: f64 = 1.5;
/// The border raised round a papilla falls back to its own height this far
/// (mm, along the arch) from where the lift is needed, as the bump
/// [`notch`]: a notch with a rounded top and no step at its feet.
pub const PAPILLA_NOTCH_HALF_MM: f64 = 2.5;
/// The raised border stays this far (mm) under the top of the contact of
/// the two crowns, so that their caps stay joined round it ...
pub const PAPILLA_NECK_MM: f64 = 0.5;
/// ... the limit rising away from it along the arch as a rounded V
/// ([`soft_abs`] with this radius, mm; one mm per mm far from it) ...
pub const PAPILLA_CAP_ROUND_MM: f64 = 1.0;
/// ... and the raised border meeting that limit in a round of this width
/// (mm, [`smooth_min`]), never a sharp corner the rim could not round.
pub const PAPILLA_ROUND_MM: f64 = 0.5;
/// Two neighbouring crowns whose scanned points come closer than this (mm)
/// have a contact; farther apart, nothing holds the border down.
pub const PAPILLA_CONTACT_MM: f64 = 2.0;
/// The scanned points of two crowns at most this much (mm) farther apart
/// than their closest two touch as well: the contact is a zone, not a
/// point (where the scanner cannot see between two crowns, the boundary of
/// their labels runs from the contact down to each papilla, every point of
/// it as close as the contact itself).
pub const PAPILLA_CONTACT_TOL_MM: f64 = 0.1;
/// A papilla under a contact whose crowns touch (within
/// [`crate::shell::embrasure::TOUCH_MM`]) lies under the interproximal
/// block-out of the shell when the shell has it ([`smooth_border`]
/// `blocked_out`): the border is not raised round it within this distance
/// (mm, along the arch) of the contact, and crosses it straight from the
/// zenith of one tooth to the next ([`zenith_line`]), the wall standing on
/// the block-out over the papilla.
pub const PAPILLA_BLOCKED_MM: f64 = 2.5;
/// The zenith of a tooth on a side is its lowest margin point within this
/// (mm, along the arch) of the middle of its crown ([`zenith_line`]).
pub const ZENITH_MM: f64 = 2.0;

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

/// The bump of the rounded notch `t` half-widths from its top: `(1 − t²)²`,
/// 1 at the top, 0 with a flat tangent at its feet (`|t| ≥ 1`).
pub fn notch(t: f64) -> f64 {
    let u = 1.0 - t * t;
    if u > 0.0 { u * u } else { 0.0 }
}

/// The weights of the notch ([`notch`], half-width
/// [`PAPILLA_NOTCH_HALF_MM`]) at 0, 1, 2 ... samples `step` mm apart.
pub fn notch_weights(step: f64) -> Vec<f64> {
    let n = (PAPILLA_NOTCH_HALF_MM / step.max(1e-3)).ceil() as usize;
    (0..=n)
        .map(|k| notch(k as f64 * step / PAPILLA_NOTCH_HALF_MM))
        .collect()
}

/// `|x|` rounded at 0 with the radius `r`: `√(x² + r²) − r`.
pub fn soft_abs(x: f64, r: f64) -> f64 {
    (x * x + r * r).sqrt() - r
}

/// The smaller of `a` and `b`, their corner rounded over `k` (the
/// polynomial smooth minimum): `min(a, b)` when they are `k` or more apart,
/// never above it and never more than `k / 4` under it.
pub fn smooth_min(a: f64, b: f64, k: f64) -> f64 {
    let m = if a < b { a } else { b };
    if k <= 0.0 {
        return m;
    }
    let d = k - (a - b).abs();
    let h = if d > 0.0 { d / k } else { 0.0 };
    m - h * h * k * 0.25
}

/// The border `z` (heights of samples along the arch) raised where it is
/// under `need` (the margin plus the relief), at most to `cap` (under the
/// contact of two crowns): every sample short of its target `min(need,
/// cap)` lifts the samples round it by that much times the notch `weights`
/// ([`notch_weights`], 1 at its own sample), the highest lift is kept, the
/// sample's own target reached, then the border is held under `cap` (unless
/// it was already above it) by a [`smooth_min`] over [`PAPILLA_ROUND_MM`]:
/// where the notch meets the limit the line turns round, never in a corner.
///
/// Never lowers the border; reaches `min(need, cap)` at every sample, less
/// at most a quarter of the round; never passes `max(z, cap)`; a border at
/// or above `need` everywhere is left as it is (the Kani laws below). `need`
/// may be `−∞` (no margin there) and `cap` `+∞` (no contact near).
pub fn papilla_lift(z: &[f64], need: &[f64], cap: &[f64], weights: &[f64]) -> Vec<f64> {
    let n = z.len();
    if weights.is_empty() || need.len() != n || cap.len() != n {
        return z.to_vec();
    }
    // the height every sample must reach (never under the border)
    let target: Vec<f64> = (0..n)
        .map(|i| {
            let t = if need[i] < cap[i] { need[i] } else { cap[i] };
            if t > z[i] { t } else { z[i] }
        })
        .collect();
    let lift: Vec<f64> = (0..n).map(|i| target[i] - z[i]).collect();
    let k = weights.len();
    (0..n)
        .map(|i| {
            let mut l = 0.0;
            let lo = i.saturating_sub(k - 1);
            let hi = (i + k).min(n);
            for (j, &lj) in lift.iter().enumerate().take(hi).skip(lo) {
                let v = lj * weights[i.abs_diff(j)];
                if v > l {
                    l = v;
                }
            }
            let raised = z[i] + l;
            let raised = if target[i] > raised {
                target[i]
            } else {
                raised
            };
            let ceiling = if cap[i] > z[i] { cap[i] } else { z[i] };
            let held = smooth_min(raised, ceiling, PAPILLA_ROUND_MM);
            if held > z[i] { held } else { z[i] }
        })
        .collect()
}

/// The contacts of the neighbouring crowns of `marg` (arch order), for
/// every two consecutive teeth whose scanned points come closer than
/// [`PAPILLA_CONTACT_MM`]: the top of the zone where they touch, the
/// highest middle of two points at most [`PAPILLA_CONTACT_TOL_MM`] farther
/// apart than the closest two. Under it the crowns touch, or the scan
/// bridges the gap between them down to the papillae: the closest two alone
/// may lie anywhere on that bridge, as low as a papilla (a real case: 3 mm
/// under the contact, 2.4 mm under the tip of the papilla it held down).
/// Each with the distance of the closest two.
fn contacts(marg: &[(Fdi, Vec<DVec3>)], crowns: &BTreeMap<Fdi, Vec<DVec3>>) -> Vec<(f64, DVec3)> {
    marg.windows(2)
        .filter_map(|w| {
            let (a, b) = (crowns.get(&w[0].0)?, crowns.get(&w[1].0)?);
            let pb: Vec<Vec3> = b.iter().map(|p| p.as_vec3()).collect();
            let index = KnnIndex::build(&pb).ok()?;
            let pairs: Vec<(f64, DVec3)> = a
                .iter()
                .filter_map(|p| {
                    let q = b[index.nearest(p.as_vec3())?.index as usize];
                    Some((p.distance(q), (*p + q) * 0.5))
                })
                .collect();
            let closest = pairs.iter().map(|(d, _)| *d).fold(f64::INFINITY, f64::min);
            if closest > PAPILLA_CONTACT_MM {
                return None;
            }
            pairs
                .into_iter()
                .filter(|(d, _)| *d <= closest + PAPILLA_CONTACT_TOL_MM)
                .map(|(_, c)| c)
                .max_by(|x, y| x.z.total_cmp(&y.z))
                .map(|c| (closest, c))
        })
        .collect()
}

/// `tip` (the highest margin point of every sample `g` of a side, `−∞`
/// without one) with the tip of the papilla under every contact of `touch`
/// (its place along the arch first: the highest of `tip` within
/// [`PAPILLA_TOP_MM`] of it) held as far on either side of that tip: every
/// sample there at least as high.
fn papilla_tops(g: &[f64], touch: &[(f64, f64)], tip: &[f64]) -> Vec<f64> {
    let mut out = tip.to_vec();
    let n = g.len().min(tip.len());
    for &(s, _) in touch {
        let Some(kt) = (0..n)
            .filter(|&k| (g[k] - s).abs() <= PAPILLA_TOP_MM && tip[k].is_finite())
            .max_by(|&a, &b| tip[a].total_cmp(&tip[b]))
        else {
            continue;
        };
        for k in (0..n).filter(|&k| (g[k] - g[kt]).abs() <= PAPILLA_TOP_MM) {
            out[k] = out[k].max(tip[kt]);
        }
    }
    out
}

/// The SMOOTH border of one side under the block-out: `gap` over the zenith
/// of every tooth (`zen`: arc length and height, arch order) and straight
/// from one zenith to the next where a contact with the block-out under it
/// (`blocked`, arc lengths) lies between them, smoothed as the scallops;
/// `+∞` over the samples `g` (step `step`) of no such stretch (a space or a
/// missing tooth keeps the border of the plugin).
pub fn zenith_line(
    zen: &[(f64, f64)],
    g: &[f64],
    blocked: &[f64],
    gap: f64,
    step: f64,
) -> Vec<f64> {
    let mut out = vec![f64::INFINITY; g.len()];
    for w in zen.windows(2) {
        let ((s0, z0), (s1, z1)) = (w[0], w[1]);
        if s1 <= s0 || !blocked.iter().any(|&b| b > s0 && b < s1) {
            continue;
        }
        for (k, &s) in g.iter().enumerate() {
            if (s0..=s1).contains(&s) {
                out[k] = gap + z0 + (s - s0) / (s1 - s0) * (z1 - z0);
            }
        }
    }
    // every stretch smoothed on its own
    let mut k = 0;
    while k < out.len() {
        if !out[k].is_finite() {
            k += 1;
            continue;
        }
        let e = (k..out.len())
            .find(|&j| !out[j].is_finite())
            .unwrap_or(out.len());
        let smooth = gauss_rows(&out[k..e], SMOOTH_WAVE_SIGMA_MM / step);
        out[k..e].copy_from_slice(&smooth);
        k = e;
    }
    out
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
/// `scallop`: the height of the scallops; `relief`: the border raised to at
/// least this above the margin all along the arch, in a rounded notch round
/// every papilla and never within [`PAPILLA_NECK_MM`] of a contact
/// ([`papilla_lift`]; `0`: the plugin's border); `blocked_out`: the shell
/// stands on the interproximal block-out under every contact whose crowns
/// touch, so the border is not raised round the papillae under those
/// contacts ([`PAPILLA_BLOCKED_MM`]) and runs straight across them from
/// the zenith of one tooth to the next, `gap` over them ([`zenith_line`],
/// never above the border of the plugin). The distal half of the
/// last tooth of each side counts only its margin at the gum
/// ([`gum_ends`]). Returns the border points and the high points (nothing
/// kept under them), or `None` with fewer than two teeth with a margin.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn smooth_border(
    marg: &[(Fdi, Vec<DVec3>)],
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
    covered: &BTreeSet<Fdi>,
    outer: f64,
    gap: f64,
    scallop: f64,
    relief: f64,
    blocked_out: bool,
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
    // the contacts of the neighbours (arc length, height): the border raised
    // round a papilla stays under them
    let found = if relief > 0.0 {
        contacts(marg, crowns)
    } else {
        Vec::new()
    };
    let touch: Vec<(f64, f64)> = found
        .iter()
        .map(|(_, c)| (u[nearest(&l, c.truncate())], c.z))
        .collect();
    // the contacts with the block-out under them (arc length)
    let blocked: Vec<f64> = found
        .iter()
        .zip(&touch)
        .filter(|((d, _), _)| blocked_out && *d <= crate::shell::embrasure::TOUCH_MM)
        .map(|(_, &(s, _))| s)
        .collect();
    let weights = notch_weights(SMOOTH_STEP_MM);
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
        // under the block-out the border stands `gap` over the zenith of
        // every tooth and runs straight from one zenith to the next across
        // every blocked papilla ([`zenith_line`]), never above the line of
        // the plugin: the smoothing of the margin left 0.1 to 1.2 mm more of
        // the crown out, and its scallops dipped round every papilla
        let zb: Vec<f64> = if blocked.is_empty() {
            zb
        } else {
            let mut zen = Vec::new();
            let mut at = 0;
            for (i, (_, p)) in marg.iter().enumerate() {
                let sc = u[nearest(&l, cen[i])];
                let low = (at..at + p.len())
                    .filter(|&k| dm[k] * sign > 0.0 && (sm[k] - sc).abs() <= ZENITH_MM)
                    .min_by(|&a, &b| m[a].z.total_cmp(&m[b].z));
                if let Some(k) = low {
                    zen.push((sm[k], m[k].z));
                }
                at += p.len();
            }
            let zl = zenith_line(&zen, &g, &blocked, gap, step);
            zb.iter().zip(&zl).map(|(a, b)| a.min(*b)).collect()
        };
        // round every papilla: the border at least `relief` above the highest
        // margin point of every bin (the tip of a papilla, held round it),
        // under the contacts
        let zb = if relief > 0.0 {
            let mut tip = vec![f64::NEG_INFINITY; g.len()];
            for (&bi, ix) in &bins {
                if let Some(v) = bi.checked_sub(lo_i).and_then(|k| tip.get_mut(k)) {
                    *v = ix.iter().map(|&i| m[i].z).fold(f64::NEG_INFINITY, f64::max);
                }
            }
            // over a papilla under the block-out the wall stands on the
            // block-out: nothing to clear there
            let need: Vec<f64> = papilla_tops(&g, &touch, &tip)
                .into_iter()
                .zip(&g)
                .map(|(t, &s)| {
                    if blocked.iter().any(|&b| (s - b).abs() <= PAPILLA_BLOCKED_MM) {
                        f64::NEG_INFINITY
                    } else {
                        t + relief
                    }
                })
                .collect();
            let cap: Vec<f64> = g
                .iter()
                .map(|&s| {
                    touch.iter().fold(f64::INFINITY, |c, &(sc, zc)| {
                        c.min(zc - PAPILLA_NECK_MM + soft_abs(s - sc, PAPILLA_CAP_ROUND_MM))
                    })
                })
                .collect();
            papilla_lift(&zb, &need, &cap, &weights)
        } else {
            zb
        };
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dc_case::Fdi;
    use glam::DVec3;

    use super::{PAPILLA_CONTACT_TOL_MM, contacts, papilla_tops, zenith_line};

    /// Between two zeniths with a blocked contact between them the border
    /// runs straight at the gap over them (smoothed); a stretch without one
    /// (a space, a missing tooth) is left to the border of the plugin.
    #[test]
    fn the_zenith_line_is_straight_across_a_blocked_papilla() {
        let g: Vec<f64> = (0..=60).map(|k| f64::from(k) * 0.5).collect();
        let zen = [(2.0, -4.0), (10.0, -5.0), (20.0, -4.0)];
        let line = zenith_line(&zen, &g, &[6.0], 1.5, 0.5);
        for (s, z) in g.iter().zip(&line) {
            if *s < 2.0 || *s > 10.0 {
                assert!(z.is_infinite(), "{s}: {z}");
            } else {
                let straight = 1.5 - 4.0 - (s - 2.0) / 8.0;
                // the smoothing renormalised at the ends of the stretch
                assert!((z - straight).abs() < 0.15, "{s}: {z} {straight}");
            }
        }
        // no blocked contact: nothing
        assert!(
            zenith_line(&zen, &g, &[], 1.5, 0.5)
                .iter()
                .all(|z| z.is_infinite())
        );
    }

    #[test]
    fn the_contact_is_the_top_of_the_zone_where_two_crowns_touch() {
        // two crowns bridged from their contact (z 0) down to the papilla
        // (z -3): the closest two points lie at the bottom of the bridge
        let (a, b) = (Fdi::new(31).unwrap(), Fdi::new(32).unwrap());
        let bridge = |x: f64, k: usize| DVec3::new(x, 0.0, -3.0 + 0.1 * k as f64);
        let crowns: BTreeMap<Fdi, Vec<DVec3>> = [
            (a, (0..=30).map(|k| bridge(-0.02, k)).collect::<Vec<_>>()),
            (
                b,
                (0..=30)
                    .map(|k| bridge(if k == 0 { 0.0 } else { 0.03 }, k))
                    .collect(),
            ),
        ]
        .into_iter()
        .collect();
        let marg = vec![(a, vec![DVec3::ZERO]), (b, vec![DVec3::ZERO])];
        let c = contacts(&marg, &crowns);
        assert_eq!(c.len(), 1);
        assert!(c[0].1.z.abs() < 1e-9, "the contact at {}", c[0].1);
        // apart by more than the tolerance than the closest two, the bridge
        // is no contact
        let mut far = crowns.clone();
        for p in far.get_mut(&b).unwrap().iter_mut().skip(1) {
            p.x = 0.05 + PAPILLA_CONTACT_TOL_MM;
        }
        let c = contacts(&marg, &far);
        assert!(c[0].1.z < -2.9, "the contact at {}", c[0].1);
    }

    #[test]
    fn the_tip_of_a_papilla_is_held_round_it_along_the_arch() {
        // samples every 0.5 mm, a papilla at s 5 under the contact at s 5.7
        // and a higher bump past the reach of that contact
        let g: Vec<f64> = (0..=20).map(|k| 0.5 * f64::from(k)).collect();
        let mut tip = vec![0.0; g.len()];
        tip[10] = 2.0;
        tip[18] = 3.0;
        tip[0] = f64::NEG_INFINITY;
        let out = papilla_tops(&g, &[(5.7, 1.0)], &tip);
        for (k, (&o, &t)) in out.iter().zip(&tip).enumerate() {
            let want = if (7..=13).contains(&k) { 2.0 } else { t };
            assert!(o == want, "sample {k} (s {}): {o}, want {want}", g[k]);
        }
        // no contact: the margin as it is
        assert_eq!(papilla_tops(&g, &[], &tip), tip);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::{PAPILLA_NOTCH_HALF_MM, PAPILLA_ROUND_MM, SMOOTH_STEP_MM, notch, papilla_lift};

    /// Three heights in tenths of a millimetre within ±4 mm (not exact in
    /// binary: the rounding of the arithmetic is part of what is proved).
    fn heights() -> [f64; 3] {
        let a: [i8; 3] = kani::any();
        a.map(|v| {
            kani::assume((-40..=40).contains(&v));
            f64::from(v) / 10.0
        })
    }

    /// The margin plus the relief: a height, or none (`−∞`, no margin there).
    fn needs() -> [f64; 3] {
        let (h, none): ([f64; 3], [bool; 3]) = (heights(), kani::any());
        [0, 1, 2].map(|i| if none[i] { f64::NEG_INFINITY } else { h[i] })
    }

    /// The limit under a contact: a height, or none (`+∞`, no contact near).
    fn caps() -> [f64; 3] {
        let (h, none): ([f64; 3], [bool; 3]) = (heights(), kani::any());
        [0, 1, 2].map(|i| if none[i] { f64::INFINITY } else { h[i] })
    }

    /// The weights of the notch at 0, 1 and 2 samples, as the border uses them.
    fn weights() -> [f64; 3] {
        [0.0, 1.0, 2.0].map(|k| notch(k * SMOOTH_STEP_MM / PAPILLA_NOTCH_HALF_MM))
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn papilla_lift_never_lowers_the_border() {
        // The border raised round the papillae is nowhere lower than the
        // border it starts from.
        let (z, need, cap) = (heights(), needs(), caps());
        let out = papilla_lift(&z, &need, &cap, &weights());
        for i in 0..3 {
            assert!(out[i] >= z[i]);
        }
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn papilla_lift_reaches_the_relief_or_the_cap() {
        // At every sample the raised border is at least the margin plus the
        // relief, or the limit under the contact where that is lower, less at
        // most a quarter of the round where the notch meets that limit.
        let (z, need, cap) = (heights(), needs(), caps());
        let out = papilla_lift(&z, &need, &cap, &weights());
        for i in 0..3 {
            assert!(out[i] >= need[i].min(cap[i]) - 0.25 * PAPILLA_ROUND_MM);
        }
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn papilla_lift_never_passes_the_cap() {
        // The raised border never goes above the limit under a contact,
        // unless the border was already above it (it is never lowered).
        let (z, need, cap) = (heights(), needs(), caps());
        let out = papilla_lift(&z, &need, &cap, &weights());
        for i in 0..3 {
            assert!(out[i] <= z[i].max(cap[i]));
        }
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn papilla_lift_leaves_a_clear_border_alone() {
        // A border already at least the relief above the margin everywhere
        // is left exactly as it is.
        let (z, need, cap) = (heights(), needs(), caps());
        for i in 0..3 {
            kani::assume(need[i] <= z[i]);
        }
        let out = papilla_lift(&z, &need, &cap, &weights());
        for i in 0..3 {
            assert!(out[i] == z[i]);
        }
    }
}
