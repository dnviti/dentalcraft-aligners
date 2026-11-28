//! The ends of the arch checked on the shape of the scan, beyond the plugin
//! ([`super::propose_checked`]).
//!
//! The plugin ends the arch where its crown score (surface texture and
//! height, [`super::crown_score`]) stays low, and its tooth sequence must
//! cover the arch up to those ends. On a model the score can be wrong both
//! ways: printed or polished molars are smooth, and the base behind them
//! (a heel or a back wall as high as the crowns, with sharp edges) is
//! rough. Then the arch runs onto the base, which gets a tooth of its own
//! (every number after it is shifted), or it stops inside the last molar.
//! The plugin (`autoseed.py`) has the same defect; the rule that no tooth is
//! found behind the last molars wins over parity here.
//!
//! A crown stands above what is next to it: across the arch the scan falls
//! by [`WALL_DROP_MM`] within [`WALL_REACH_MM`] of the centre of its top, on
//! the buccal and on the lingual side (the walls of the crown down to the
//! gum). The base of a model, a flat heel or a ridge running across the
//! arch, stays as high on one side at least. When an end tooth of the
//! checked proposal fails this test ([`end_crowns`]), the arch is profiled
//! over the whole scan along the arch curve and cut back, an end tooth that
//! is no crown at a time, until both end teeth are crowns ([`refit`]). An
//! end tooth that passes the test but reaches onto the slope of what rises
//! behind the last crown, higher than it (the heel of a model), is cut back
//! too, to the valley in front of that slope ([`on_slope`]): otherwise the
//! sequence stretches the last crown over the slope, and every other tooth
//! with it. The new proposal replaces the checked one only when it is no
//! worse ([`no_worse`]): its end teeth are crowns, and every contact of the
//! checked proposal in an interproximal valley is still a contact.
//!
//! [`WALL_DROP_MM`]: super::WALL_DROP_MM
//! [`WALL_REACH_MM`]: super::WALL_REACH_MM

use glam::{DMat4, DVec2, DVec3};

use super::dp::SLACK_MM;
use super::numpy::interp;
use super::ridges;
use super::{
    AutoseedError, BIN_MM, CROWN_WIDTH_MM, END_ZONE, MIN_SEEDS, Options, Profiles, Proposal,
    RawProfile, TAIL_DROP_MM, ToothBins, VALLEY_COST, VALLEY_MM, WALL_DROP_MM, WALL_REACH_MM,
    propose_in,
};
use crate::anatomy::Arch;

/// Cell of the height map (mm).
const CELL_MM: f64 = 0.5;
/// Step (mm) across the arch of the walls test.
const WALL_STEP_MM: f64 = 0.25;
/// Half length (mm) along the arch of the strip of the walls test.
const WALL_STRIP_MM: f64 = 1.0;
/// The top of a crown: its points less than this (mm) under its highest.
const TOP_BAND_MM: f64 = 2.0;
/// How many teeth inside the arch the direction of an end tooth is taken
/// from ([`is_crown`]).
const DIRECTION_TEETH: usize = 3;
/// Profiles of the arch cut back at most this many times.
const ROUNDS: usize = 12;
/// Shortest arch (mm) a cut may leave.
const MIN_ARCH_MM: f64 = 20.0;
/// A contact is kept by a boundary at most this far (mm) along the arch
/// ([`no_worse`]).
const CONTACT_MM: f64 = 1.0;
/// The crown tops on each side of a valley are looked for within this
/// distance (mm) along the arch ([`valley`]), as for the plugin's valley
/// depth.
const VALLEY_REACH_MM: f64 = 2.0;

/// The scan seen from its occlusal side: the highest point of every cell
/// of [`CELL_MM`] (arch coordinates).
#[derive(Debug, Clone, Default)]
pub(crate) struct HeightMap {
    x0: f64,
    y0: f64,
    nx: usize,
    ny: usize,
    z: Vec<f64>,
}

impl HeightMap {
    /// The height map of the points `va` (arch coordinates).
    pub(crate) fn new(va: &[DVec3]) -> Self {
        let pts: Vec<DVec3> = va.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.is_empty() {
            return Self::default();
        }
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for p in &pts {
            lo = lo.min(p.truncate());
            hi = hi.max(p.truncate());
        }
        let nx = ((hi.x - lo.x) / CELL_MM) as usize + 1;
        let ny = ((hi.y - lo.y) / CELL_MM) as usize + 1;
        let mut z = vec![f64::NEG_INFINITY; nx * ny];
        for p in &pts {
            let i = (((p.x - lo.x) / CELL_MM) as usize).min(nx - 1);
            let j = (((p.y - lo.y) / CELL_MM) as usize).min(ny - 1);
            z[i * ny + j] = z[i * ny + j].max(p.z);
        }
        Self {
            x0: lo.x,
            y0: lo.y,
            nx,
            ny,
            z,
        }
    }

    /// Height of the scan at `p`; `None` where it has no point.
    pub(crate) fn at(&self, p: DVec2) -> Option<f64> {
        let (fx, fy) = ((p.x - self.x0) / CELL_MM, (p.y - self.y0) / CELL_MM);
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (i, j) = (fx as usize, fy as usize);
        if i >= self.nx || j >= self.ny {
            return None;
        }
        let z = self.z[i * self.ny + j];
        z.is_finite().then_some(z)
    }
}

/// The centre (occlusal plane) and the height of the top of the points
/// `pts`: those less than [`TOP_BAND_MM`] under the highest one. `None`
/// with fewer than 5 points.
fn top_of(pts: &[DVec3]) -> Option<(DVec2, f64)> {
    if pts.len() < 5 {
        return None;
    }
    let top = pts.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
    let band: Vec<DVec2> = pts
        .iter()
        .filter(|p| p.z >= top - TOP_BAND_MM)
        .map(|p| p.truncate())
        .collect();
    let c = band.iter().copied().sum::<DVec2>() / band.len() as f64;
    Some((c, top))
}

/// [`top_of`] the crown on the bins `[a, b)` of the profiles.
pub(crate) fn crown_top(pr: &Profiles, (a, b): (usize, usize)) -> Option<(DVec2, f64)> {
    let pts: Vec<DVec3> = pr
        .points
        .iter()
        .zip(&pr.bins)
        .filter(|(_, k)| (a..b).contains(*k))
        .map(|(p, _)| *p)
        .collect();
    top_of(&pts)
}

/// The highest point of the scan on a strip [`WALL_STRIP_MM`] long along
/// `u` centred on `p`; `None` where the scan has no point.
pub(crate) fn strip(hm: &HeightMap, p: DVec2, u: DVec2) -> Option<f64> {
    let z = [-1.0, -0.5, 0.0, 0.5, 1.0]
        .map(|f| hm.at(p + u * (f * WALL_STRIP_MM)))
        .into_iter()
        .flatten()
        .fold(f64::NEG_INFINITY, f64::max);
    z.is_finite().then_some(z)
}

/// How far from `c` towards `w` the scan falls [`WALL_DROP_MM`] under
/// `top` (a strip along `u`, [`strip`]), up to [`WALL_REACH_MM`]. Where the
/// scan has no point the search goes on: past its border, nothing falls.
fn wall(hm: &HeightMap, c: DVec2, top: f64, u: DVec2, w: DVec2) -> Option<f64> {
    let steps = (WALL_REACH_MM / WALL_STEP_MM).round() as usize;
    (1..=steps)
        .map(|k| k as f64 * WALL_STEP_MM)
        .find(|&t| strip(hm, c + w * t, u).is_some_and(|z| z < top - WALL_DROP_MM))
}

/// The top `top` centred on `c` stands above the scan on both sides across
/// the direction `u` of the arch: the walls of a crown, at most
/// [`CROWN_WIDTH_MM`] apart.
fn walls(hm: &HeightMap, c: DVec2, top: f64, u: DVec2) -> bool {
    let w = u.perp();
    wall(hm, c, top, u, w)
        .zip(wall(hm, c, top, u, -w))
        .is_some_and(|(a, b)| a + b <= CROWN_WIDTH_MM)
}

/// The tooth on the bins `tooth` of the profiles stands above the scan on
/// both sides across the arch, from the centre of the top of its bins. The
/// arch runs from the tooth on the bins `inner`, [`DIRECTION_TEETH`] teeth
/// inside the arch, to it: not from the tooth next to it, which on the base
/// of a model may lie on the base too, both along its rim. `true` when the
/// bins have too few points to tell.
fn is_crown(pr: &Profiles, hm: &HeightMap, tooth: (usize, usize), inner: (usize, usize)) -> bool {
    let (Some((c, top)), Some((ci, _))) = (crown_top(pr, tooth), crown_top(pr, inner)) else {
        return true;
    };
    let u = (c - ci).normalize_or_zero();
    u == DVec2::ZERO || walls(hm, c, top, u)
}

/// How many teeth from each end (first, last) of a proposal whose teeth lie
/// on the bins `segs` of `pr` are no crown ([`is_crown`]), up to the first
/// crown: the teeth on the base of a model behind the last molar.
fn non_crowns(pr: &Profiles, hm: &HeightMap, segs: &ToothBins) -> [usize; 2] {
    let n = segs.len();
    if n < 2 {
        return [0; 2];
    }
    let k = DIRECTION_TEETH.min(n - 1);
    let first = (0..n - k)
        .take_while(|&j| !is_crown(pr, hm, segs[j].1, segs[j + k].1))
        .count();
    let last = (k..n)
        .rev()
        .take_while(|&j| !is_crown(pr, hm, segs[j].1, segs[j - k].1))
        .count();
    [first, last]
}

/// Which end teeth (first, last) of a proposal whose teeth lie on the bins
/// `segs` of `pr` are crowns ([`is_crown`]).
pub(crate) fn end_crowns(pr: &Profiles, hm: &HeightMap, segs: &ToothBins) -> [bool; 2] {
    non_crowns(pr, hm, segs).map(|r| r == 0)
}

/// The teeth from each end of a proposal whose teeth lie on the bins `segs`
/// of `pr` that are no crown ([`non_crowns`]), first end first.
pub(crate) fn non_crown_teeth(pr: &Profiles, hm: &HeightMap, segs: &ToothBins) -> Vec<u8> {
    let [first, last] = non_crowns(pr, hm, segs);
    let n = segs.len();
    let mut out: Vec<u8> = segs[..first.min(n)].iter().map(|s| s.0.get()).collect();
    let tail = n.saturating_sub(last).max(first.min(n));
    out.extend(segs[tail..].iter().map(|s| s.0.get()));
    out
}

/// The plugin's proposal on profiles whose ends are found again from the
/// crowns: the profiles run over every point along the arch curve, then the
/// arch is cut at the middle of an end tooth that is no crown (with
/// [`END_ZONE`] mm of margin past it), until both end teeth are crowns (at
/// most [`ROUNDS`] times, never below [`MIN_SEEDS`] teeth or [`MIN_ARCH_MM`]
/// of arch). At its middle and not at its start: a crown stretched over the
/// base behind it by the plugin's sequence keeps its own part. An end tooth
/// that is a crown but reaches onto the slope of what rises behind it
/// ([`on_slope`]) is cut back to the valley in front of that slope, less
/// the slack the sequence has at an end of the profile ([`SLACK_MM`]): the
/// sequence ends in that valley, wherever the cut before left it, and not
/// up the slope with every tooth stretched. The crown score of these
/// profiles is its height part alone ([`RawProfile::between`]): the texture
/// is what put the plugin's ends in the wrong place. Returns the profiles,
/// the proposal and the bins of its teeth.
pub(crate) fn refit(
    arch: Arch,
    opts: &Options,
    frame: &DMat4,
    raw: &RawProfile,
    hm: &HeightMap,
) -> Result<(Profiles, Proposal, ToothBins), AutoseedError> {
    let (mut lo, mut hi) = raw.extent();
    // a stretch with no crown between two crowns is gum here too
    let mut pr = ridges::gum_stretches(raw.between(lo, hi, true)?, hm);
    let (mut p, mut segs) = propose_in(arch, opts, frame, &pr, false)?;
    for _ in 0..ROUNDS {
        let n = segs.len();
        if n == 0 {
            break;
        }
        let [first, last] = end_crowns(&pr, hm, &segs);
        let up = [
            first && on_slope(&pr, segs[0].1, true),
            last && on_slope(&pr, segs[n - 1].1, false),
        ];
        if first && last && up == [false; 2] {
            break;
        }
        let at = |k: usize| pr.s_min + k as f64 * BIN_MM;
        let half = |(a, b): (usize, usize)| pr.s_min + 0.5 * (a + b) as f64 * BIN_MM;
        let lo2 = if up[0] {
            (at(slope_valley(&pr, segs[0].1, true)) - SLACK_MM).max(lo)
        } else if first {
            lo
        } else {
            let h = half(segs[0].1);
            if h - END_ZONE > lo { h - END_ZONE } else { h }
        };
        let hi2 = if up[1] {
            (at(slope_valley(&pr, segs[n - 1].1, false)) + SLACK_MM).min(hi)
        } else if last {
            hi
        } else {
            let h = half(segs[n - 1].1);
            if h + END_ZONE < hi { h + END_ZONE } else { h }
        };
        // nothing cut (a slope reaching the end of the profile), or too little left
        if hi2 - lo2 < MIN_ARCH_MM || (lo2 <= lo && hi2 >= hi) {
            break;
        }
        let pr2 = ridges::gum_stretches(raw.between(lo2, hi2, true)?, hm);
        let (p2, segs2) = propose_in(arch, opts, frame, &pr2, false)?;
        if p2.seeds.len() < MIN_SEEDS {
            break;
        }
        (lo, hi, pr, p, segs) = (lo2, hi2, pr2, p2, segs2);
    }
    Ok((pr, p, segs))
}

/// The end tooth on the bins `(a, b)` of `pr`, at the first end of the
/// arch (`first`) or at the last, reaches onto the slope of something that
/// rises behind it: its outer half stands higher than its inner half, the
/// profile does not fall from that top to the outer end of the tooth by
/// [`TAIL_DROP_MM`] (it goes on rising past it, no distal surface there),
/// and the profile behind the tooth stands higher than the whole tooth.
/// The heel of a model rising behind the last molar, higher than its crown:
/// the crown passes the walls test, since the top band of the stretched
/// tooth covers it. A last crown falls at its distal surface, or stands at
/// least as high as its distal part, or nothing higher stands behind it.
fn on_slope(pr: &Profiles, (a, b): (usize, usize), first: bool) -> bool {
    if b <= a {
        return false;
    }
    let top = |r: std::ops::Range<usize>| pr.h[r].iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mid = (a + b) / 2;
    let (outer, inner, behind, end) = if first {
        (top(a..mid), top(mid..b), top(0..a), pr.h[a])
    } else {
        (top(mid..b), top(a..mid), top(b..pr.nb), pr.h[b - 1])
    };
    outer > inner && end >= outer - TAIL_DROP_MM && behind > outer
}

/// The valley of the end tooth on the bins `(a, b)` that reaches onto a
/// slope ([`on_slope`]): its lowest bin between the highest bin of its
/// outer half (on the slope) and that of its inner half (on the crown).
fn slope_valley(pr: &Profiles, (a, b): (usize, usize), first: bool) -> usize {
    let highest = |r: std::ops::Range<usize>| {
        let start = r.start;
        r.max_by(|&i, &j| pr.h[i].total_cmp(&pr.h[j]))
            .unwrap_or(start)
    };
    let mid = (a + b) / 2;
    let (o, i) = if first {
        (highest(a..mid), highest(mid..b))
    } else {
        (highest(mid..b), highest(a..mid))
    };
    (o.min(i)..=o.max(i))
        .min_by(|&x, &y| pr.h[x].total_cmp(&pr.h[y]))
        .unwrap_or(o)
}

/// Arc length of the contacts of the teeth on the bins `segs` of `pr`: the
/// boundaries where two teeth touch (not across a space) in a valley of the
/// crown tops, under the tops on both sides of it ([`valley`]) by a depth
/// the plugin's boundary cost does not penalise (a contact at least as
/// likely as the inside of a crown, [`VALLEY_COST`]), but those of the
/// teeth that are no crown at either end ([`non_crowns`]). A boundary on
/// the slope of a crown is no contact: a proposal stretched over the end
/// of a crown puts boundaries there.
fn contacts(pr: &Profiles, hm: &HeightMap, segs: &ToothBins) -> Vec<f64> {
    let n = segs.len();
    let [first, last] = non_crowns(pr, hm, segs);
    (0..n.saturating_sub(1))
        .filter(|&j| j >= first && j + 1 + last < n)
        .filter(|&j| segs[j].1.1 == segs[j + 1].1.0)
        .map(|j| segs[j].1.1)
        .filter(|&k| interp(valley(pr, k), &VALLEY_MM, &VALLEY_COST) <= 0.0)
        .map(|k| pr.s_min + k as f64 * BIN_MM)
        .collect()
}

/// Depth of the valley of the crown tops of `pr` at the bin `k`: how far it
/// lies under the highest bin on each side of it within [`VALLEY_REACH_MM`],
/// the lower of the two (the plugin's depth, [`Profiles::dip`], takes the
/// highest bin on either side, and is as deep on a slope).
fn valley(pr: &Profiles, k: usize) -> f64 {
    let k = k.min(pr.nb - 1);
    let r = (VALLEY_REACH_MM / BIN_MM).round() as usize;
    let top = |s: &[f64]| s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let left = top(&pr.h[k.saturating_sub(r)..=k]);
    let right = top(&pr.h[k..(k + r + 1).min(pr.nb)]);
    left.min(right) - pr.h[k]
}

/// The proposal on the bins `r_segs` of `r_pr` ([`refit`]) is no worse than
/// the checked one on the bins `c_segs` of `c_pr`: its own end teeth are
/// crowns, and every contact of the checked proposal ([`contacts`]) is
/// still a boundary of its teeth, within [`CONTACT_MM`]. Its numbers may
/// differ (the teeth on the base of the model gone, every number after them
/// shifted back) and it may have a tooth more at an end, but no crown of
/// the checked proposal is lost, nor any of its teeth moved off its crown.
pub(crate) fn no_worse(
    c_pr: &Profiles,
    c_segs: &ToothBins,
    r_pr: &Profiles,
    r_segs: &ToothBins,
    hm: &HeightMap,
) -> bool {
    if end_crowns(r_pr, hm, r_segs) != [true; 2] {
        return false;
    }
    let bounds: Vec<f64> = r_segs
        .iter()
        .flat_map(|(_, (a, b))| [*a, *b])
        .map(|k| r_pr.s_min + k as f64 * BIN_MM)
        .collect();
    contacts(c_pr, hm, c_segs)
        .iter()
        .all(|s| bounds.iter().any(|b| (b - s).abs() <= CONTACT_MM))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Profiles with the crown-top heights `h` only.
    fn profile(h: &[f64]) -> Profiles {
        let nb = h.len();
        Profiles {
            s_min: 0.0,
            s_max: (nb - 1) as f64 * BIN_MM,
            nb,
            bins: Vec::new(),
            points: Vec::new(),
            h: h.to_vec(),
            dip: vec![0.0; nb],
            bl: vec![0.0; nb],
            crown: vec![1.0; nb],
            off_top: vec![f64::NAN; nb],
            dh_bl: vec![f64::NAN; nb],
            vertex: Vec::new(),
        }
    }

    /// The crown tops (mm) of the right end of the printed test piece as the
    /// re-fit profiled it after its first cut, the scan closed by the base of
    /// an earlier import: from the cut, the heel of the model falling to a
    /// valley, then the first molar and the second premolar. The re-fit's
    /// first molar ran on bins 12..66, up the heel.
    const CLOSED: [f64; 98] = [
        -5.46, -5.56, -5.67, -5.79, -5.88, -5.96, -6.02, -6.09, -6.18, -6.33, -6.54, -6.68, -6.76,
        -6.75, -6.70, -6.59, -6.55, -6.55, -6.61, -6.65, -6.66, -6.65, -6.67, -6.72, -6.82, -6.96,
        -7.14, -7.33, -7.47, -7.59, -7.68, -7.74, -7.79, -7.86, -7.92, -8.00, -8.10, -8.19, -8.28,
        -8.34, -8.36, -8.33, -8.28, -8.19, -8.10, -8.00, -7.92, -7.88, -7.82, -7.76, -7.72, -7.66,
        -7.55, -7.46, -7.37, -7.27, -7.17, -7.09, -7.02, -6.96, -6.97, -6.96, -6.94, -6.93, -6.92,
        -6.85, -6.81, -6.79, -6.76, -6.75, -6.75, -6.75, -6.73, -6.66, -6.54, -6.40, -6.21, -6.00,
        -5.83, -5.67, -5.53, -5.42, -5.34, -5.28, -5.25, -5.23, -5.22, -5.23, -5.30, -5.43, -5.50,
        -5.50, -5.50, -5.51, -5.47, -5.46, -5.50, -5.48,
    ];

    /// The same end of the open scan, after the last cut of the re-fit: its
    /// first molar on bins 20..65, from the slope of the heel to the
    /// premolar.
    const OPEN: [f64; 97] = [
        -5.94, -5.99, -6.07, -6.17, -6.27, -6.43, -6.51, -6.56, -6.60, -6.61, -6.54, -6.53, -6.52,
        -6.52, -6.53, -6.56, -6.62, -6.70, -6.88, -7.05, -7.22, -7.38, -7.54, -7.59, -7.64, -7.70,
        -7.77, -7.83, -7.91, -8.00, -8.08, -8.17, -8.26, -8.31, -8.33, -8.30, -8.23, -8.13, -8.03,
        -7.94, -7.86, -7.84, -7.78, -7.74, -7.68, -7.61, -7.49, -7.40, -7.30, -7.20, -7.11, -7.02,
        -6.95, -6.89, -6.87, -6.84, -6.82, -6.81, -6.79, -6.75, -6.72, -6.70, -6.68, -6.67, -6.68,
        -6.68, -6.66, -6.57, -6.45, -6.27, -6.06, -5.84, -5.67, -5.51, -5.39, -5.29, -5.23, -5.19,
        -5.16, -5.15, -5.18, -5.29, -5.34, -5.34, -5.35, -5.37, -5.34, -5.37, -5.45, -5.50, -5.47,
        -5.39, -5.25, -5.08, -4.89, -4.71, -4.52,
    ];

    /// The same profile turned end for end: the last end of an arch.
    fn reversed(h: &[f64]) -> Vec<f64> {
        h.iter().rev().copied().collect()
    }

    /// The first molar stretched up the heel is on a slope, and its valley
    /// is the low point between the heel and its crown (bin 40, 10 mm from
    /// the cut); the same molar ending on the slope of the heel, its crown
    /// higher than the rest of it, is not. Both ends of an arch alike.
    #[test]
    fn a_last_crown_stretched_up_the_heel_is_on_a_slope() {
        let closed = profile(&CLOSED);
        assert!(on_slope(&closed, (12, 66), true));
        assert_eq!(slope_valley(&closed, (12, 66), true), 40);
        let open = profile(&OPEN);
        assert!(!on_slope(&open, (20, 65), true));
        let n = CLOSED.len();
        let last = profile(&reversed(&CLOSED));
        assert!(on_slope(&last, (n - 66, n - 12), false));
        assert_eq!(slope_valley(&last, (n - 66, n - 12), false), n - 1 - 40);
        let m = OPEN.len();
        assert!(!on_slope(
            &profile(&reversed(&OPEN)),
            (m - 65, m - 20),
            false
        ));
    }

    /// A last crown higher at its distal part (a molar tipped on the curve
    /// of Spee) is no slope when nothing higher stands behind it, nor when
    /// it falls at its distal surface into a gap before a heel higher than
    /// it; a crown with something higher behind it is none either when its
    /// own top is its inner half.
    #[test]
    fn a_tipped_last_crown_is_not_on_a_slope() {
        // gum behind (-8), the molar rising distally to -5, the premolar -6
        let mut h = vec![-8.0; 8];
        h.extend((0..40).map(|k| -6.5 + 1.5 * f64::from(40 - k) / 40.0));
        h.extend(vec![-6.0; 30]);
        assert!(!on_slope(&profile(&h), (8, 48), true));
        // a heel behind (-0.5), the molar falling 4 mm into the gap before
        // it from its distal cusp (-0.8), its mesial cusps at -1.2
        let mut h = vec![-0.5; 8];
        h.extend((0..8).map(|k| -5.0 + 4.2 * f64::from(k) / 8.0));
        h.extend(vec![-0.8; 8]);
        h.extend(vec![-1.2; 24]);
        h.extend(vec![-1.0; 30]);
        assert!(!on_slope(&profile(&h), (8, 48), true));
        // the heel behind higher (-4), the molar highest at its inner half
        let mut h = vec![-4.0; 8];
        h.extend((0..40).map(|k| -6.5 + 1.5 * f64::from(k) / 40.0));
        h.extend(vec![-6.0; 30]);
        assert!(!on_slope(&profile(&h), (8, 48), true));
    }
}
