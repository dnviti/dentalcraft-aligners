//! The last bite of the case: how the relation of the two arches was found
//! and, for a registration on bite scans, its report.
//!
//! The plugin stores the same report as JSON on both cases (`bite_json`,
//! `bite.store` / `bite.info`): its source (`MORSO`, `VIRTUALE`), the fit of
//! every bite scan, the agreement of two scans, the interpenetration, the
//! contacts, the opening at the first contact, the facing of the occlusal
//! planes, the correction and the verdict with its reasons. The date of the
//! plugin is left out: a command gives the same case whenever it runs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How the relation of the arches was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiteMethod {
    /// Automatic registration on the bite scans (`MORSO`).
    Registration,
    /// Registration on the bite scans started from points picked by hand
    /// (`oa.bite_points`, then `MORSO`).
    ThreePoints,
    /// Estimated closing without a bite scan (`VIRTUALE`).
    Virtual,
}

/// Verdict of a registration (`verdetto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiteVerdict {
    /// No reason to review it (`BUONA`).
    Good,
    /// Some reason to review it (`DA VERIFICARE`).
    ToReview,
}

/// Overlap of a registered bite scan with the crowns of one arch
/// (`bite.metrics`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BiteFit {
    /// Bite area within the inlier distance of the crowns (mm²).
    pub area_mm2: f64,
    /// Teeth with at least the tooth area in common, with their area (mm²).
    pub teeth: BTreeMap<u16, f64>,
    /// Number of those teeth.
    pub n_teeth: usize,
    /// RMS of the facing surfaces away from the margins (mm).
    pub rms_mm: f64,
    /// 95th percentile of the facing distances (mm).
    pub p95_mm: Option<f64>,
    /// Share of the facing surfaces within the inlier distance.
    pub coherence: f64,
    /// Share of the bite scan in common.
    pub bite_fraction: f64,
    /// Overlap of the last fitting iteration.
    pub icp_overlap: f64,
}

/// One bite scan of a registration (`scansioni`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BiteScanFit {
    /// Name of the bite scan (its file).
    pub scan: String,
    /// Side it lies on: `right`, `left`, `both`, or `None` when unknown.
    pub side: Option<String>,
    /// Fit on the lower arch.
    pub lower: BiteFit,
    /// Fit on the upper arch.
    pub upper: BiteFit,
}

/// Report of a registration on the bite scans (`bite._report`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BiteRegistration {
    /// Every bite scan.
    pub scans: Vec<BiteScanFit>,
    /// Largest displacement of the upper crowns between the relations of
    /// two bite scans (mm), `None` with one scan.
    pub agreement_mm: Option<f64>,
    /// Deepest interpenetration of the arches now (mm).
    pub penetration_mm: f64,
    /// The interpenetration the registration gave, kept once the bite is
    /// corrected (`compenetrazione_registrata_mm`).
    pub registered_penetration_mm: Option<f64>,
    /// Upper crown points inside or within 0.3 mm of the lower crowns.
    pub contact_points: usize,
    /// Vertical move of the upper arch to the first contact after the
    /// registration (mm; positive = interpenetration, negative = open).
    pub opening_at_contact_mm: f64,
    /// Dot product of the occlusal axes (−1 = facing).
    pub facing: f64,
    /// Verdict.
    pub verdict: BiteVerdict,
    /// Reasons to review it, as codes: `rms_above_good:<scan>:<arch>`,
    /// `few_teeth:<scan>:<arch>`, `full_arch_scan`, `one_side_only`,
    /// `scans_disagree`, `interpenetration`, `arches_not_in_contact`.
    pub reasons: Vec<String>,
}

impl Default for BiteRegistration {
    fn default() -> Self {
        Self {
            scans: Vec::new(),
            agreement_mm: None,
            penetration_mm: 0.0,
            registered_penetration_mm: None,
            contact_points: 0,
            opening_at_contact_mm: 0.0,
            facing: 0.0,
            verdict: BiteVerdict::Good,
            reasons: Vec::new(),
        }
    }
}

impl BiteRegistration {
    /// The verdict the reasons give (`bite._verdict`).
    pub fn update_verdict(&mut self) {
        self.verdict = if self.reasons.is_empty() {
            BiteVerdict::Good
        } else {
            BiteVerdict::ToReview
        };
    }

    /// Largest RMS of every bite scan on either arch (mm).
    pub fn max_rms_mm(&self) -> f64 {
        self.scans
            .iter()
            .map(|s| s.lower.rms_mm.max(s.upper.rms_mm))
            .fold(0.0, f64::max)
    }
}

/// The last bite of the case (`bite_json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BiteReport {
    /// How the relation was found.
    pub method: BiteMethod,
    /// The report of a registration on the bite scans; `None` for the
    /// virtual bite.
    #[serde(default)]
    pub registration: Option<BiteRegistration>,
    /// Opening of the bite to remove the interpenetration, summed over the
    /// corrections since (mm, `0` when not corrected; `correzione_mm`).
    #[serde(default)]
    pub correction_mm: f64,
}

impl BiteReport {
    /// The report of a virtual bite.
    pub fn virtual_bite() -> Self {
        Self {
            method: BiteMethod::Virtual,
            registration: None,
            correction_mm: 0.0,
        }
    }

    /// The bite was opened to remove the interpenetration.
    pub fn corrected(&self) -> bool {
        self.correction_mm > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_follows_the_reasons() {
        let mut r = BiteRegistration {
            reasons: vec!["one_side_only".into()],
            ..BiteRegistration::default()
        };
        r.update_verdict();
        assert_eq!(r.verdict, BiteVerdict::ToReview);
        r.reasons.clear();
        r.update_verdict();
        assert_eq!(r.verdict, BiteVerdict::Good);
    }

    #[test]
    fn a_report_survives_json() {
        let rep = BiteReport {
            method: BiteMethod::ThreePoints,
            registration: Some(BiteRegistration {
                scans: vec![BiteScanFit {
                    scan: "right.stl".into(),
                    side: Some("right".into()),
                    lower: BiteFit {
                        rms_mm: 0.021,
                        teeth: [(44, 31.5)].into(),
                        n_teeth: 1,
                        ..BiteFit::default()
                    },
                    upper: BiteFit::default(),
                }],
                agreement_mm: Some(0.03),
                penetration_mm: 0.2,
                registered_penetration_mm: Some(0.6),
                verdict: BiteVerdict::ToReview,
                reasons: vec!["one_side_only".into()],
                ..BiteRegistration::default()
            }),
            correction_mm: 0.4,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["method"], "three_points");
        assert_eq!(v["registration"]["verdict"], "to_review");
        let back: BiteReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, rep);
        assert!(back.corrected());
        assert!(!BiteReport::virtual_bite().corrected());
    }
}
