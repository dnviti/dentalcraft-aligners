//! The aligner shown in the 3D view (the plugin's `show_aligner`,
//! `props.py:715`, and the meshes of its preview, `animation.py`
//! `set_aligner_preview` / `aligner_exact`).
//!
//! The aligners are built for the view by `aligners.view.aligner` (the
//! preview: the construction of the printed aligner at the coarser detail of
//! the view) and `aligners.view.aligner-exact` (the aligner the export
//! writes, at the quality of the print files). Each built aligner is a base:
//! a stage shown without one of its own shows the nearest base of its arch,
//! moved with the teeth and the gingiva. A base is kept with the key of what
//! it was built from: once the design or the plan changes, it is out of date
//! and is no longer drawn.

use dc_case::{BlobRef, CaseState, ScanRole};
use serde::{Deserialize, Serialize};

use crate::AlignersState;

/// What the aligners of the view are built from, as a key: the data of the
/// module but what never changes an aligner (the calendar and the staging
/// settings, the clinical review, the case analysis, the visits, the reports
/// of the steps, the study models and the view itself), and the scans of the
/// case. Another key: the aligners built before are out of date.
pub fn view_key(state: &CaseState, data: &AlignersState) -> String {
    let mut d = data.clone();
    d.aligner_view = AlignerView::default();
    d.diagnosis = Default::default();
    d.setup = Default::default();
    d.staging_results.clear();
    d.tooth_speed.clear();
    d.visits.clear();
    d.bite_report = None;
    d.segmentation_doubts.clear();
    d.segmentation_runs.clear();
    d.teeth_basis.clear();
    d.review.clear();
    d.study_models.clear();
    let s = &mut d.settings;
    s.staging = Default::default();
    s.interval_days = 0;
    s.interval_policy = Default::default();
    s.start_date = None;
    s.wear_hours = 0.0;
    s.study_base = Default::default();
    s.find_teeth = Default::default();
    s.segmentation = Default::default();
    s.setup = Default::default();
    let scans: Vec<_> = state
        .case
        .scans
        .iter()
        .map(|s| (s.role, s.mesh, s.import_xform))
        .collect();
    let bytes = serde_json::to_vec(&(&d, scans)).unwrap_or_default();
    BlobRef::of_bytes(&bytes).to_hex()
}

/// The aligner in the 3D view: shown or not, and the aligners built for it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignerView {
    /// The aligner is drawn over the teeth, at the stage shown.
    pub shown: bool,
    /// The aligners built for the view, by arch and stage.
    pub bases: Vec<AlignerBase>,
}

/// One aligner built for the view: an arch at a stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignerBase {
    /// Its arch.
    pub arch: ScanRole,
    /// The stage it was built at (`0` is the initial position).
    pub stage: u32,
    /// The solid, in case coordinates.
    pub mesh: BlobRef,
    /// Built as the export builds the print file, at the quality of the
    /// print files; `false`: the preview, at the detail of the view.
    pub exact: bool,
    /// Detail of the fields it was built with (mm).
    pub voxel_mm: f64,
    /// Tolerance of its surface (mm).
    pub tolerance_mm: f64,
    /// What it was built from (the design, the teeth and the plan): out of
    /// date when the case gives another key.
    pub key: String,
    /// How long it took (s).
    pub seconds: f64,
}

impl AlignerView {
    /// Nothing shown and nothing built.
    pub fn is_empty(&self) -> bool {
        !self.shown && self.bases.is_empty()
    }

    /// The bases of `arch` built from `key` (up to date), in the order of
    /// their stages.
    pub fn current(&self, arch: ScanRole, key: &str) -> Vec<&AlignerBase> {
        let mut v: Vec<&AlignerBase> = self
            .bases
            .iter()
            .filter(|b| b.arch == arch && b.key == key)
            .collect();
        v.sort_by_key(|b| (b.stage, !b.exact));
        v
    }

    /// The base of `arch` the stage `stage` shows: the up-to-date base
    /// nearest to it, an exact one before a preview at the same stage, the
    /// earlier one of two as near (the plugin's `min(have, key=|x - t|)`).
    pub fn nearest(&self, arch: ScanRole, key: &str, stage: u32) -> Option<&AlignerBase> {
        self.current(arch, key)
            .into_iter()
            .min_by_key(|b| (b.stage.abs_diff(stage), b.stage, !b.exact))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(arch: ScanRole, stage: u32, exact: bool, key: &str) -> AlignerBase {
        AlignerBase {
            arch,
            stage,
            mesh: BlobRef::of_bytes(format!("{arch:?}{stage}{exact}").as_bytes()),
            exact,
            voxel_mm: 0.3,
            tolerance_mm: 0.04,
            key: key.to_owned(),
            seconds: 1.0,
        }
    }

    #[test]
    fn the_key_follows_the_design_and_the_plan_not_the_calendar_or_the_view() {
        let s = CaseState::default();
        let d = AlignersState::default();
        let k = view_key(&s, &d);
        let mut calendar = d.clone();
        calendar.settings.start_date = Some("2026-12-01".into());
        calendar.settings.interval_days = 7;
        calendar.aligner_view.shown = true;
        calendar
            .aligner_view
            .bases
            .push(base(ScanRole::Upper, 1, false, "x"));
        assert_eq!(view_key(&s, &calendar), k);
        let mut wall = d.clone();
        wall.settings.shell_thickness_mm = 0.8;
        assert_ne!(view_key(&s, &wall), k);
        let mut plan = d.clone();
        plan.stages.push(crate::Stage::new(1));
        assert_ne!(view_key(&s, &plan), k);
        let mut quality = d;
        quality.settings.quality = crate::Quality::Draft;
        assert_ne!(view_key(&s, &quality), k);
    }

    #[test]
    fn a_stage_shows_the_nearest_base_of_its_arch_built_from_the_case_as_it_is() {
        let v = AlignerView {
            shown: true,
            bases: vec![
                base(ScanRole::Upper, 2, false, "k"),
                base(ScanRole::Upper, 8, false, "k"),
                base(ScanRole::Upper, 8, true, "k"),
                base(ScanRole::Upper, 5, false, "old"),
                base(ScanRole::Lower, 6, false, "k"),
            ],
        };
        let at = |stage| {
            v.nearest(ScanRole::Upper, "k", stage)
                .map(|b| (b.stage, b.exact))
        };
        assert_eq!(at(0), Some((2, false)));
        // as near as 2 and 8: the earlier one
        assert_eq!(at(5), Some((2, false)));
        assert_eq!(at(6), Some((8, true)));
        assert_eq!(at(8), Some((8, true)));
        // the base of an earlier design is never shown
        assert_eq!(v.current(ScanRole::Upper, "old").len(), 1);
        assert!(v.nearest(ScanRole::Upper, "new", 5).is_none());
        assert_eq!(v.nearest(ScanRole::Lower, "k", 1).map(|b| b.stage), Some(6));
        assert!(!v.is_empty() && AlignerView::default().is_empty());
    }

    #[test]
    fn a_view_saved_without_its_fields_loads_as_nothing_shown() {
        let v: AlignerView = serde_json::from_str("{}").unwrap();
        assert_eq!(v, AlignerView::default());
        let full = AlignerView {
            shown: true,
            bases: vec![base(ScanRole::Lower, 3, true, "k")],
        };
        let back: AlignerView =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back, full);
    }
}
