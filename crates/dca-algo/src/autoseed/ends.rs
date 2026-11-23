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
//! is no crown at a time, until both end teeth are crowns ([`refit`]). The
//! new proposal replaces the checked one only when it is no worse
//! ([`no_worse`]): its end teeth are crowns, and every contact of the
//! checked proposal in an interproximal valley is still a contact.
//!
//! [`WALL_DROP_MM`]: super::WALL_DROP_MM
//! [`WALL_REACH_MM`]: super::WALL_REACH_MM

use glam::{DMat4, DVec2, DVec3};

use super::numpy::interp;
use super::ridges;
use super::{
    AutoseedError, BIN_MM, CROWN_WIDTH_MM, END_ZONE, MIN_SEEDS, Options, Profiles, Proposal,
    RawProfile, ToothBins, VALLEY_COST, VALLEY_MM, WALL_DROP_MM, WALL_REACH_MM, propose_in,
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

/// The plugin's proposal on profiles whose ends are found again from the
/// crowns: the profiles run over every point along the arch curve, then the
/// arch is cut at the middle of an end tooth that is no crown (with
/// [`END_ZONE`] mm of margin past it), until both end teeth are crowns (at
/// most [`ROUNDS`] times, never below [`MIN_SEEDS`] teeth or [`MIN_ARCH_MM`]
/// of arch). At its middle and not at its start: a crown stretched over the
/// base behind it by the plugin's sequence keeps its own part. The crown
/// score of these profiles is its height part alone
/// ([`RawProfile::between`]): the texture is what put the plugin's ends in
/// the wrong place. Returns the profiles, the proposal and the bins of its
/// teeth.
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
        let [first, last] = end_crowns(&pr, hm, &segs);
        if first && last {
            break;
        }
        let n = segs.len();
        let half = |(a, b): (usize, usize)| pr.s_min + 0.5 * (a + b) as f64 * BIN_MM;
        let lo2 = if first {
            lo
        } else {
            let h = half(segs[0].1);
            if h - END_ZONE > lo { h - END_ZONE } else { h }
        };
        let hi2 = if last {
            hi
        } else {
            let h = half(segs[n - 1].1);
            if h + END_ZONE < hi { h + END_ZONE } else { h }
        };
        if hi2 - lo2 < MIN_ARCH_MM {
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
