//! Verification of the arch on the crown widths after the teeth are built
//! (`case_auto.auto_steps`, the loop over `attempt`).
//!
//! When the arch of a scan was decided only by the shape and the widths of
//! its teeth ([`crate::autoseed::ArchSource::ShapeAndWidths`]), the plugin
//! analyses it as a lower arch first and checks the hypothesis on the
//! segmented crowns: the central incisors are about half the first molar
//! width (5 / 11 mm) in a lower arch and about 0.64-0.9 in an upper arch
//! (8.5 / 10 mm, Wheeler). The upper hypothesis is tried when the ratio says
//! upper, or when the crowns do not follow the lower sequence at all (width
//! fit above [`FIT_MAX`]); it is kept only when its fit is better (clearly
//! better, less than half, without the incisor evidence). With any other
//! cue the arch is not questioned: an unusual ratio is only reported.

use std::collections::BTreeMap;

use dc_case::Fdi;
use serde::Serialize;

use crate::anatomy::Arch;
use crate::autoseed::{RATIO_UPPER, incisor_molar_ratio, width_fit};

/// Width fit above which the segmented crowns do not follow the tooth
/// sequence (`case_auto.FIT_MAX`).
pub const FIT_MAX: f64 = 1.0;
/// Attempts of the loop (`range(3)`).
pub const ATTEMPTS: usize = 3;

/// What the crowns of one hypothesis measured.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Trial {
    /// [`width_fit`] cost of the mesio-distal widths.
    pub fit: f64,
    /// [`incisor_molar_ratio`].
    pub ratio: Option<f64>,
}

impl Trial {
    /// The trial of the mesio-distal widths of the built teeth.
    pub fn of(widths: &BTreeMap<Fdi, f64>) -> Self {
        Self {
            fit: width_fit(widths).0,
            ratio: incisor_molar_ratio(widths),
        }
    }
}

/// The arch to try next after `current` was built and measured, with every
/// hypothesis tried so far (`current` included); `None` keeps `current`.
pub fn next_arch(current: Arch, tried: &BTreeMap<Arch, Trial>) -> Option<Arch> {
    let lower = tried.get(&Arch::Lower);
    let by_ratio = lower
        .and_then(|t| t.ratio)
        .is_some_and(|r| r >= RATIO_UPPER);
    match current {
        Arch::Lower if tried.len() == 1 => {
            let lower = lower?;
            (by_ratio || lower.fit > FIT_MAX).then_some(Arch::Upper)
        }
        Arch::Upper => {
            let (upper, lower) = (tried.get(&Arch::Upper)?, lower?);
            let k = if by_ratio { 1.0 } else { 0.5 };
            (upper.fit > k * lower.fit).then_some(Arch::Lower)
        }
        Arch::Lower => None,
    }
}

/// An incisor / molar ratio unusual for `arch` (reported when another cue
/// decided the arch).
pub fn ratio_unusual(arch: Arch, ratio: Option<f64>) -> bool {
    ratio.is_some_and(|r| match arch {
        Arch::Upper => r < 0.55,
        Arch::Lower => r > 0.72,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(fit: f64, ratio: Option<f64>) -> Trial {
        Trial { fit, ratio }
    }

    #[test]
    fn a_lower_arch_whose_incisors_are_upper_ones_tries_the_upper_arch() {
        let mut tried = BTreeMap::from([(Arch::Lower, t(0.4, Some(0.85)))]);
        assert_eq!(next_arch(Arch::Lower, &tried), Some(Arch::Upper));
        // the upper fit is not worse than the lower one: kept
        tried.insert(Arch::Upper, t(0.3, Some(0.85)));
        assert_eq!(next_arch(Arch::Upper, &tried), None);
        // worse: back to lower, and then it stays
        tried.insert(Arch::Upper, t(0.5, Some(0.85)));
        assert_eq!(next_arch(Arch::Upper, &tried), Some(Arch::Lower));
        assert_eq!(next_arch(Arch::Lower, &tried), None);
    }

    #[test]
    fn without_the_incisors_only_a_bad_fit_tries_upper_and_upper_must_halve_it() {
        let ok = BTreeMap::from([(Arch::Lower, t(0.4, None))]);
        assert_eq!(next_arch(Arch::Lower, &ok), None);
        let mut bad = BTreeMap::from([(Arch::Lower, t(2.0, Some(0.45)))]);
        assert_eq!(next_arch(Arch::Lower, &bad), Some(Arch::Upper));
        bad.insert(Arch::Upper, t(1.2, Some(0.6)));
        assert_eq!(next_arch(Arch::Upper, &bad), Some(Arch::Lower));
        bad.insert(Arch::Upper, t(0.9, Some(0.6)));
        assert_eq!(next_arch(Arch::Upper, &bad), None);
    }

    #[test]
    fn the_ratio_is_unusual_outside_the_range_of_the_arch() {
        assert!(ratio_unusual(Arch::Lower, Some(0.8)));
        assert!(!ratio_unusual(Arch::Lower, Some(0.5)));
        assert!(ratio_unusual(Arch::Upper, Some(0.5)));
        assert!(!ratio_unusual(Arch::Upper, None));
    }
}
