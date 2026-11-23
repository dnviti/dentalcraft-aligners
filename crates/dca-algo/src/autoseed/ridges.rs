//! Stretches of the arch with no crown on them, beyond the plugin
//! ([`super::propose_checked`]).
//!
//! The plugin tells a crown from the ridge of a missing tooth by its crown
//! score: surface texture, and a height above the occlusal plane where an
//! edentulous ridge is 5-8 mm under the crowns ([`super::CROWN_LOW`]). On a
//! model the ridge of a missing tooth can stand much higher (the empty
//! ridge of 21 on the printed test piece: 3.3-5.4 mm under the crowns on
//! either side) and be as rough as a crown: then, without the tooth marked
//! missing, the plugin takes the ridge for a tooth (a seed in the gap) or
//! spaces the teeth over it (every number on that side shifted). The
//! plugin (`autoseed.py`) has the same defect.
//!
//! A stretch is the ridge of a missing tooth (or a space between two teeth)
//! when three things hold together ([`low_stretches`]):
//!
//! - along the arch it sinks between the crowns beside it: at least
//!   [`RIDGE_MIN_MM`] long [`RIDGE_DROP_MM`] under the crown tops on both
//!   sides (looked for within [`RIDGE_REACH_MM`]);
//! - nothing stands on it: no point rises [`RIDGE_RISE_MM`] above the
//!   lowest points on both sides of it, as the top of a crown does above
//!   the valleys beside it;
//! - it has no walls: across the arch the scan falls from its top at most
//!   [`RIDGE_GUM_SLOPE`] mm per mm, as gum does, where a crown, even a short
//!   or partly erupted one, stands in the gum with walls (a crown sunk under
//!   its neighbours can be as flat along the arch as a ridge).
//!
//! Its crown score is then set to that of the gum, so that the teeth are
//! found around it: a missing tooth or a space ([`Doubt::EmptyRidge`]),
//! unless that loses a crown the plugin's proposal had elsewhere
//! ([`keeps_crowns`]). A stretch only [`RIDGE_DOUBT_MM`] under the crowns,
//! with a rise under [`RIDGE_TOP_MM`] or with walls under
//! [`RIDGE_CROWN_SLOPE`], could be a low tooth or a ridge: it is left as it
//! is ([`Doubt::LowCrown`]), and so is an empty stretch in that case.
//!
//! [`RIDGE_MIN_MM`]: super::RIDGE_MIN_MM
//! [`RIDGE_DROP_MM`]: super::RIDGE_DROP_MM
//! [`RIDGE_REACH_MM`]: super::RIDGE_REACH_MM
//! [`RIDGE_RISE_MM`]: super::RIDGE_RISE_MM
//! [`RIDGE_GUM_SLOPE`]: super::RIDGE_GUM_SLOPE
//! [`RIDGE_DOUBT_MM`]: super::RIDGE_DOUBT_MM
//! [`RIDGE_TOP_MM`]: super::RIDGE_TOP_MM
//! [`RIDGE_CROWN_SLOPE`]: super::RIDGE_CROWN_SLOPE
//! [`Doubt::EmptyRidge`]: super::Doubt::EmptyRidge
//! [`Doubt::LowCrown`]: super::Doubt::LowCrown

use glam::DVec2;

use super::ends::{self, HeightMap};
use super::{
    BIN_MM, Profiles, RIDGE_CROWN_SLOPE, RIDGE_DOUBT_MM, RIDGE_DROP_MM, RIDGE_GUM_SLOPE,
    RIDGE_MIN_MM, RIDGE_REACH_MM, RIDGE_RISE_MM, RIDGE_TOP_MM, RIDGE_WALL_REACH_MM, ToothBins,
};

/// The crowns beside a stretch are taken from this length (mm) of the arch
/// next to it ([`steepest_fall`]).
const BESIDE_MM: f64 = 2.0;
/// Step (mm) across the arch of the fall of the scan ([`steepest_fall`]).
const FALL_STEP_MM: f64 = 0.25;
/// The fall of the scan is measured over this distance (mm) across the arch.
const FALL_RUN_MM: f64 = 1.0;

/// The low stretches of a profile ([`low_stretches`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LowStretches {
    /// No crown: the bins `[a, b)` of the ridge of a missing tooth or of a
    /// space between two teeth.
    pub empty: Vec<(usize, usize)>,
    /// Low between two crowns, but possibly a low tooth: left as they are.
    pub doubtful: Vec<(usize, usize)>,
}

impl LowStretches {
    /// Every stretch left as it was found, a doubt: when making the empty
    /// ones gum would lose a crown ([`keeps_crowns`]).
    pub(crate) fn left_as_found(mut self) -> Self {
        self.doubtful.append(&mut self.empty);
        self.doubtful.sort_unstable();
        self
    }
}

/// The teeth on the bins `after` (the proposal with the stretches `empty`
/// made gum) keep every crown of the teeth on the bins `before` (the
/// proposal of the same profile as found): the middle of each tooth of
/// `before` outside the stretches lies on a tooth of `after`. Making a
/// stretch gum must not lose a crown elsewhere (at an end of the arch).
pub(crate) fn keeps_crowns(
    before: &ToothBins,
    after: &ToothBins,
    empty: &[(usize, usize)],
) -> bool {
    let on = |m: usize, (a, b): (usize, usize)| (a..b).contains(&m);
    before.iter().all(|(_, (a, b))| {
        let m = (a + b) / 2;
        empty.iter().any(|&s| on(m, s)) || after.iter().any(|&(_, s)| on(m, s))
    })
}

/// The runs `[a, b)` of the bins of `h` that lie `drop` mm under the
/// highest bins within `reach` bins on both sides, not touching an end of
/// the profile.
fn runs_under(h: &[f64], reach: usize, drop: f64) -> Vec<(usize, usize)> {
    let nb = h.len();
    let low: Vec<bool> = (0..nb)
        .map(|i| {
            let left = h[i.saturating_sub(reach)..i]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let right = h[(i + 1).min(nb)..(i + 1 + reach).min(nb)]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            h[i] <= left.min(right) - drop
        })
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < nb {
        if !low[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < nb && low[i] {
            i += 1;
        }
        if a > 0 && i < nb {
            out.push((a, i));
        }
    }
    out
}

/// How much the highest point of `h` rises above the lowest points on both
/// sides of it: 0 on a profile that only falls, or only rises, or sinks.
fn rise(h: &[f64]) -> f64 {
    let n = h.len();
    let mut left = vec![f64::INFINITY; n];
    let mut right = vec![f64::INFINITY; n];
    let mut m = f64::INFINITY;
    for i in 0..n {
        m = m.min(h[i]);
        left[i] = m;
    }
    m = f64::INFINITY;
    for i in (0..n).rev() {
        m = m.min(h[i]);
        right[i] = m;
    }
    (0..n)
        .map(|i| h[i] - left[i].max(right[i]))
        .fold(0.0, f64::max)
}

/// The steepest fall of the scan (mm per mm, over [`FALL_RUN_MM`]) across
/// the arch from the centre of the top of the stretch `[a, b)` of `pr`,
/// within [`RIDGE_WALL_REACH_MM`] of it on either side: the walls of a
/// crown, or the slopes of the gum. The arch runs from the crowns before
/// the stretch to those after it. `None` when that cannot be measured.
fn steepest_fall(pr: &Profiles, hm: &HeightMap, (a, b): (usize, usize)) -> Option<f64> {
    let beside = (BESIDE_MM / BIN_MM).round() as usize;
    let (before, _) = ends::crown_top(pr, (a.saturating_sub(beside), a))?;
    let (after, _) = ends::crown_top(pr, (b, (b + beside).min(pr.nb)))?;
    let (c, _) = ends::crown_top(pr, (a, b))?;
    let u = (after - before).normalize_or_zero();
    if u == DVec2::ZERO {
        return None;
    }
    let n = (RIDGE_WALL_REACH_MM / FALL_STEP_MM).round() as usize;
    let run = (FALL_RUN_MM / FALL_STEP_MM).round() as usize;
    let mut steepest = None::<f64>;
    for w in [u.perp(), -u.perp()] {
        let z: Vec<Option<f64>> = (0..=n + run)
            .map(|i| ends::strip(hm, c + w * (i as f64 * FALL_STEP_MM), u))
            .collect();
        for i in 0..=n {
            if let (Some(z0), Some(z1)) = (z[i], z[i + run]) {
                let fall = (z0 - z1) / FALL_RUN_MM;
                steepest = Some(steepest.map_or(fall, |s| s.max(fall)));
            }
        }
    }
    steepest
}

/// The stretches of the arch with no crown on them, and those that could
/// be a low tooth, on the crown-top profile of `pr` and the scan seen from
/// its occlusal side `hm` (module docs).
pub(crate) fn low_stretches(pr: &Profiles, hm: &HeightMap) -> LowStretches {
    let reach = (RIDGE_REACH_MM / BIN_MM).round() as usize;
    let long = |(a, b): &(usize, usize)| (b - a) as f64 * BIN_MM >= RIDGE_MIN_MM;
    let deep = runs_under(&pr.h, reach, RIDGE_DROP_MM);
    let mut out = LowStretches::default();
    for (a, b) in runs_under(&pr.h, reach, RIDGE_DOUBT_MM)
        .into_iter()
        .filter(long)
    {
        let r = rise(&pr.h[a..b]);
        let sinks = deep.iter().any(|&(c, d)| a <= c && d <= b && long(&(c, d)));
        let fall = steepest_fall(pr, hm, (a, b));
        let gum = fall.is_some_and(|f| f <= RIDGE_GUM_SLOPE);
        let crown_walls = fall.is_some_and(|f| f > RIDGE_CROWN_SLOPE);
        if sinks && r < RIDGE_RISE_MM && gum {
            out.empty.push((a, b));
        } else if r < RIDGE_TOP_MM && !crown_walls {
            out.doubtful.push((a, b));
        }
    }
    out
}

/// `pr` with the crown score of the bins `stretches` set to 0, that of the
/// gum.
pub(crate) fn as_gum(mut pr: Profiles, stretches: &[(usize, usize)]) -> Profiles {
    for &(a, b) in stretches {
        for c in &mut pr.crown[a..b] {
            *c = 0.0;
        }
    }
    pr
}

/// `pr` with its stretches with no crown ([`low_stretches`]) made gum.
pub(crate) fn gum_stretches(pr: Profiles, hm: &HeightMap) -> Profiles {
    let empty = low_stretches(&pr, hm).empty;
    as_gum(pr, &empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hollow_with_nothing_on_it_is_found_and_a_low_crown_is_not() {
        // two crowns, 8 mm of ridge 4 mm under them, then two crowns
        let mut h = vec![0.0; 40];
        h.extend(vec![-4.0; 32]);
        h.extend(vec![0.0; 40]);
        assert_eq!(runs_under(&h, 32, 2.5), vec![(40, 72)]);
        assert!(rise(&h[40..72]) < 1e-12);
        // a low tooth in the hollow: its top 1 mm above the valleys
        let mut t = h.clone();
        for (k, z) in t[44..68].iter_mut().enumerate() {
            let u = (k as f64 - 11.5) / 12.0;
            *z = -4.0 + 1.0 * (1.0 - u * u);
        }
        assert!(
            (rise(&t[40..72]) - 1.0).abs() < 0.01,
            "{}",
            rise(&t[40..72])
        );
        // the ends of the profile are no hollow
        let end = [vec![0.0; 40], vec![-4.0; 32]].concat();
        assert!(runs_under(&end, 32, 2.5).is_empty());
    }
}
