//! The module's state stored in the case.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{BlobRef, Fdi, Pose, ScanRole};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{
    AlignerBite, AlignerEnd, ComponentEdits, RampKind, ReviewKind, ReviewRecord, SCHEMA_VERSION,
    Settings, StagingPattern, TrimParams, Visit,
};

/// A seed point: the centre of the occlusal face of a tooth, with its
/// number. Seeds tell the segmentation which teeth exist and where.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    /// Tooth number.
    pub fdi: Fdi,
    /// Position in case coordinates (mm).
    pub position: DVec3,
}

/// What the built teeth of an arch were built from: its tooth colours (the
/// labels blob), its tooth marks (sorted by number), its missing teeth and
/// the side of its occlusal face. When the arch holds something else now,
/// its built teeth are out of date ([`AlignersState::teeth_out_of_date`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TeethBasis {
    /// The labels blob of the arch.
    pub labels: Option<BlobRef>,
    /// The tooth marks of the arch, by tooth number.
    pub seeds: Vec<Seed>,
    /// The teeth of the arch marked missing.
    pub missing: Vec<Fdi>,
    /// The occlusal side of the arch was turned over by hand
    /// ([`AlignersState::occlusal_flipped`]).
    pub occlusal_flipped: bool,
}

/// Why the built teeth of an arch are out of date
/// ([`AlignersState::teeth_changes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeethChange {
    /// Teeth corrected by hand wait to be rebuilt.
    HandCorrected,
    /// The tooth colours (the labels blob) changed.
    Colours,
    /// The tooth marks changed (placed, moved, deleted or renumbered).
    Marks,
    /// Teeth were marked missing or present.
    Missing,
    /// The occlusal side of the arch was turned over by hand: the arch frame
    /// the teeth were measured in is upside down now.
    Orientation,
}

/// Rest frame of a 3D tooth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToothFrame {
    /// Tooth frame in case coordinates (x mesio-distal, y bucco-lingual,
    /// z along the long axis).
    pub pose: Pose,
    /// Estimated centre of resistance, in the tooth frame (mm).
    pub center_of_resistance: DVec3,
}

/// Measures of one tooth that the diagnosis (and the later planning stages)
/// read, kept by the teeth stage. Positions are in case coordinates (mm), like
/// [`ToothFrame`]; distances and angles do not depend on the frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToothMeasures {
    /// Area-weighted centroid of the crown.
    pub centroid: DVec3,
    /// Reference point of the crown (origin of the [`ToothFrame`]).
    pub reference: DVec3,
    /// Mesio-distal width (mm).
    pub width_md: f64,
    /// Bucco-lingual width (mm).
    pub width_bl: f64,
    /// Crown height above the mean of the gingival margin (mm).
    pub height: f64,
    /// Cusp tip (the highest point of the crown along the long axis).
    pub tip: DVec3,
    /// Cemento-enamel junction (estimate).
    pub cej: DVec3,
    /// Alveolar crest (estimate).
    pub crest: DVec3,
    /// Root apex (estimate).
    pub apex: DVec3,
    /// Mesial interproximal contact point.
    pub contact_m: DVec3,
    /// Distal interproximal contact point.
    pub contact_d: DVec3,
    /// Rotation about the long axis relative to the arch tangent (degrees).
    pub rot_dev_deg: f64,
    /// Tilt of the long axis from the vertical of the arch (degrees).
    pub axis_tilt_deg: f64,
    /// `+1` when the tooth frame's x axis points mesially, `-1` distally.
    pub md_sign: i8,
}

/// Further results of the space analysis of one arch.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchDetails {
    /// Largest space between two adjacent crowns (mm).
    pub largest_space_mm: f64,
    /// Offset of the dental midline from the middle of the arch (mm).
    pub midline_offset_mm: f64,
    /// CAT-CAT model-analysis score (points).
    pub catcat_score: u32,
}

/// Space analysis of one arch.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ArchSpace {
    /// Space available along the arch (mm).
    pub available_mm: f64,
    /// Space required by the teeth (mm).
    pub required_mm: f64,
    /// Little irregularity index (mm).
    pub little_index_mm: f64,
}

impl ArchSpace {
    /// Crowding (positive) or spacing (negative) in mm.
    pub fn crowding_mm(&self) -> f64 {
        self.required_mm - self.available_mm
    }
}

/// Values the clinician measured by hand for one arch, in place of the
/// computed ones (the plugin's `oa.diagnosis_edit`). `None` is a value left
/// to the computation. They are kept apart from the computed values
/// ([`ArchSpace`], [`ArchDetails`]), which a reset brings back.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchEdits {
    /// Space available minus space required (mm): negative is crowding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discrepancy_mm: Option<f64>,
    /// Little irregularity index (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub little_index_mm: Option<f64>,
    /// Offset of the dental midline from the middle of the arch (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub midline_offset_mm: Option<f64>,
    /// Largest space between two adjacent crowns (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub largest_space_mm: Option<f64>,
}

impl ArchEdits {
    /// No value was written by hand.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Results of the diagnosis; `None` until computed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnosis {
    /// Space analysis per arch ([`ScanRole::Upper`] and [`ScanRole::Lower`]).
    pub arches: BTreeMap<ScanRole, ArchSpace>,
    /// Bolton anterior ratio (lower / upper, per cent).
    pub bolton_anterior: Option<f64>,
    /// Bolton overall ratio (lower / upper, per cent).
    pub bolton_overall: Option<f64>,
    /// Complexity score of the case: the largest CAT-CAT score of the arches
    /// with the values the clinician wrote by hand ([`Diagnosis::edits`])
    /// taken in place of the computed ones.
    pub complexity_score: Option<f64>,
    /// Further results per arch.
    pub details: BTreeMap<ScanRole, ArchDetails>,
    /// Values written by hand per arch. A new analysis of the arch clears
    /// them, as in the plugin, where `analyze` writes the diagnosis anew.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub edits: BTreeMap<ScanRole, ArchEdits>,
}

impl Diagnosis {
    /// The values written by hand for the arch (none when it has none).
    pub fn edits_of(&self, role: ScanRole) -> ArchEdits {
        self.edits.get(&role).copied().unwrap_or_default()
    }

    /// Space available minus required of the arch (mm, negative is
    /// crowding): the value written by hand, else the computed one. `None`
    /// until the arch is analysed.
    pub fn discrepancy_mm(&self, role: ScanRole) -> Option<f64> {
        let space = self.arches.get(&role)?;
        Some(
            self.edits_of(role)
                .discrepancy_mm
                .unwrap_or(-space.crowding_mm()),
        )
    }

    /// Crowding (positive) or spacing (negative) of the arch, mm, with the
    /// value written by hand when there is one.
    pub fn crowding_mm(&self, role: ScanRole) -> Option<f64> {
        self.discrepancy_mm(role).map(|d| -d)
    }

    /// Little irregularity index of the arch (mm), the value written by
    /// hand or the computed one.
    pub fn little_index_mm(&self, role: ScanRole) -> Option<f64> {
        let space = self.arches.get(&role)?;
        Some(
            self.edits_of(role)
                .little_index_mm
                .unwrap_or(space.little_index_mm),
        )
    }

    /// Dental midline offset of the arch (mm; 0 when not analysed), the
    /// value written by hand or the computed one.
    pub fn midline_offset_mm(&self, role: ScanRole) -> f64 {
        self.edits_of(role)
            .midline_offset_mm
            .unwrap_or_else(|| self.details.get(&role).map_or(0.0, |d| d.midline_offset_mm))
    }

    /// Largest space between crowns of the arch (mm; 0 when not analysed),
    /// the value written by hand or the computed one.
    pub fn largest_space_mm(&self, role: ScanRole) -> f64 {
        self.edits_of(role)
            .largest_space_mm
            .unwrap_or_else(|| self.details.get(&role).map_or(0.0, |d| d.largest_space_mm))
    }
}

/// Final position of the teeth (the setup).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// Final pose of each tooth, relative to its rest pose.
    pub poses: BTreeMap<Fdi, Pose>,
    /// Teeth the clinician locked: automatic recomputation leaves them alone.
    pub locked: BTreeSet<Fdi>,
    /// Interproximal reduction planned by the setup, one entry per contact
    /// (`amount_mm` in total over both surfaces). The staging decides at
    /// which aligner each one is done.
    pub ipr: Vec<IprStep>,
    /// The arch line the setup aligned each arch on (case coordinates, mm,
    /// evenly spaced points).
    pub lines: BTreeMap<ScanRole, Vec<DVec3>>,
    /// The arch line drawn by the clinician for each arch (case
    /// coordinates, mm): the setup aligns the cusps on it instead of the
    /// automatic line until it is taken back (the plugin's
    /// `oa_arch_line`). Kept when the final position is cleared.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub drawn: BTreeMap<ScanRole, Vec<DVec3>>,
    /// What the last automatic final position gave for each arch (the
    /// plugin's `setup_json`, shown by the inspector of "Final position").
    /// It goes with the final position it describes.
    pub results: BTreeMap<ScanRole, SetupSummary>,
}

/// What one automatic final position of an arch gave (the plugin's
/// `setup_json`). The IPR of the arch is in [`Setup::ipr`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SetupSummary {
    /// Expansion per side at the molars (mm).
    pub expansion_posterior_mm: f64,
    /// Expansion per side at the canines (mm).
    pub expansion_canine_mm: f64,
    /// Incisor advancement (mm; negative: retracted).
    pub advance_mm: f64,
    /// Crowding not resolved (mm).
    pub residual_crowding_mm: f64,
    /// Space left next to the fixed teeth (mm).
    pub spacing_mm: f64,
    /// The log of the run, each entry as `dca_algo::setup::Note` writes it
    /// (data, worded by the interface).
    pub notes: Vec<serde_json::Value>,
    /// Where the aligner ended when this final position was computed (the
    /// plugin's `oa_setup_end`): another end now is to compute again
    /// ([`AlignersState::setup_end_changed`]).
    pub aligner_end: AlignerEnd,
}

/// Interproximal reduction on the contact between two neighbouring teeth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IprStep {
    /// The two teeth in contact.
    pub between: [Fdi; 2],
    /// Enamel to remove at this stage, in total over both surfaces (mm).
    pub amount_mm: f64,
}

/// One aligner of the treatment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    /// Position in the series (0 is the initial situation).
    pub index: u32,
    /// Pose of each tooth at this stage, relative to its rest pose.
    pub poses: BTreeMap<Fdi, Pose>,
    /// IPR to do before this aligner.
    #[serde(default)]
    pub ipr: Vec<IprStep>,
}

impl Stage {
    /// Stage with no tooth moved and no IPR.
    pub fn new(index: u32) -> Self {
        Self {
            index,
            poses: BTreeMap::new(),
            ipr: Vec::new(),
        }
    }
}

/// What the last automatic staging of an arch gave (the parts of the
/// plugin's `plan_json` its page "Allineatori" shows). It goes with the
/// aligners of the arch it describes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StagingSummary {
    /// Aligners of the arch.
    pub aligners: u32,
    /// The pattern used (never `auto`: the one the automatic choice took).
    pub pattern: StagingPattern,
    /// Complexity level of the case the pattern was chosen from (the name
    /// `dca_algo::staging::Level` serialises to: `easy`, `moderate`,
    /// `difficult`, `very_complex`).
    pub level: String,
    /// The slowed teeth and their speed (the teeth at the normal speed are
    /// not listed).
    pub slowed: BTreeMap<Fdi, f64>,
    /// Collisions between neighbouring crowns left in the aligners.
    pub collisions_left: u32,
    /// The final movement was increased by the overcorrection fractions.
    pub overcorrection: bool,
}

/// What one automatic separation of an arch gave.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SegmentationRun {
    /// Teeth it separated.
    pub teeth: u32,
    /// How long it took (s).
    pub seconds: f64,
}

/// Everything the Aligners module keeps in a case.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignersState {
    /// Seed points of the teeth.
    pub seeds: Vec<Seed>,
    /// Teeth marked as missing by hand (agenesis, extraction): they have no
    /// seed, labels, frame, solid or movement, and stay out of the later
    /// stages until they are marked present again.
    pub missing: BTreeSet<Fdi>,
    /// Teeth whose labels were corrected by hand after the teeth were built:
    /// their solid has to be rebuilt.
    pub dirty: BTreeSet<Fdi>,
    /// Per-vertex tooth labels of each scan (a mesh blob whose vertex
    /// attribute holds the label), keyed by arch.
    pub labels: BTreeMap<ScanRole, BlobRef>,
    /// Rest frame of each reconstructed tooth.
    pub tooth_frames: BTreeMap<Fdi, ToothFrame>,
    /// Diagnosis results.
    pub diagnosis: Diagnosis,
    /// Final position of the teeth.
    pub setup: Setup,
    /// Measures of each tooth (widths, contacts, landmarks), from the teeth
    /// stage.
    pub tooth_geometry: BTreeMap<Fdi, ToothMeasures>,
    /// The aligners, in order.
    pub stages: Vec<Stage>,
    /// What the last automatic staging of each arch gave (the result line
    /// of the inspector of "Aligners"). It goes with the aligners of the
    /// arch.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub staging_results: BTreeMap<ScanRole, StagingSummary>,
    /// The teeth slowed by hand in the staging and their relative speed
    /// ([`crate::TOOTH_SPEED`], below 1; the plugin's `speed_json`, set by
    /// `oa.tooth_speed`): their limits per aligner are multiplied by it.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tooth_speed: BTreeMap<Fdi, f64>,
    /// Trim line parameters.
    pub trim: TrimParams,
    /// Settings.
    pub settings: Settings,
    /// Bite with the aligners worn: occlusal settings and ramps.
    pub aligner_bite: AlignerBite,
    /// Closed solid of each reconstructed tooth (a mesh blob in case
    /// coordinates: watertight, outward normals).
    pub crowns: BTreeMap<Fdi, BlobRef>,
    /// How many of the first vertices of each closed tooth of `crowns` are
    /// scanned, by the blob of the tooth (the plugin's `crown_vertices`: the
    /// vertices from that index on were added to close the crown, its
    /// `oa_rebuilt`). Written when a tooth is closed
    /// ([`AlignersState::record_crown`]); a tooth closed by a version that
    /// did not keep it has none, and its closed solid is not drawn under the
    /// gingiva of the model below its margin (`_tuck_below_margin`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub crown_scanned: BTreeMap<BlobRef, u32>,
    /// Gingiva without holes of each arch (a mesh blob in case coordinates
    /// whose vertex labels hold the tooth a vertex moves with and the
    /// margin, membrane and col flags of `dca_algo::gingiva`).
    pub gingiva: BTreeMap<ScanRole, BlobRef>,
    /// Control visits, in order (`dca_algo::visits`).
    pub visits: Vec<Visit>,
    /// A bite registration (the registration commands and the virtual bite)
    /// moved the upper arch and no arch was replaced since. The move is baked
    /// into the mesh: nothing undoes it when an arch is replaced, so
    /// [`AlignersState::clear_arch`] reports it once, as `bite_registered`.
    pub bite_registered: bool,
    /// The last bite of the case: how the relation was found and the report
    /// of a registration on the bite scans (the plugin's `bite_json`). It
    /// goes with [`AlignersState::bite_registered`], reported with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bite_report: Option<crate::BiteReport>,
    /// What the check of the separated teeth
    /// (`aligners.preparation.segmentation.check`) left implausible in each
    /// arch, in words (its problems, then its notes): the plugin's
    /// `segmentation_doubts`. The export and the report warn about them
    /// (the aligners must not be delivered without checking the tooth marks
    /// and their numbers). They go when the arch is separated again, its
    /// colours are corrected by hand, its teeth are rebuilt by hand, or what
    /// depends on its labels is cleared.
    pub segmentation_doubts: BTreeMap<ScanRole, Vec<String>>,
    /// The last automatic separation of each arch
    /// (`aligners.preparation.segmentation.automatic`): how many teeth it
    /// gave and how long it took, for the line the inspector shows. It goes
    /// with the labels it describes ([`AlignersState::forget_separation`]):
    /// when the arch is separated again, its colours are corrected by hand,
    /// its teeth are rebuilt by hand, or the labels are cleared or replaced.
    pub segmentation_runs: BTreeMap<ScanRole, SegmentationRun>,
    /// What the built teeth of each arch were built from
    /// ([`AlignersState::record_teeth_basis`]). An arch built by an older
    /// version has none: its teeth are out of date only while teeth
    /// corrected by hand (`dirty`) wait to be rebuilt.
    pub teeth_basis: BTreeMap<ScanRole, TeethBasis>,
    /// What was changed by hand on the 3D components with the gizmo: the
    /// moves of the scans (with their arch) and the sizes of built teeth.
    pub components: ComponentEdits,
    /// The arches whose occlusal side was turned over by hand
    /// (`aligners.case.set-arch` with `flip_occlusal`, the plugin's
    /// `oa_occlusal_flip`): the side of the teeth the seeds give was the
    /// wrong one, so every arch frame of the arch is turned over (`x` and the
    /// occlusal normal reversed, `y` kept). It goes with the scan: it moves
    /// with an arch that changes place and goes when the scan is replaced.
    pub occlusal_flipped: BTreeSet<ScanRole>,
    /// The clinical review: the classifications of the program and the
    /// clinician's decision on each (`dca_algo::review`, the plugin's
    /// `classes`). The records of an arch go with the teeth they were
    /// computed from ([`AlignersState::clear_label_dependents`], id
    /// `review`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub review: Vec<ReviewRecord>,
    /// Where the aligner of each arch ends (the plugin's
    /// `aligner_end_right` / `_left`), set by
    /// `aligners.production.aligner-end`; an arch without an entry is
    /// covered whole ([`AlignersState::aligner_teeth`]). It is a choice of
    /// the clinician: it stays when the scan is replaced and moves with an
    /// arch that changes place.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub aligner_end: BTreeMap<ScanRole, AlignerEnd>,
    /// The study model of each arch (`aligners.output.study-models`, B6):
    /// a mesh blob in case coordinates, the scan of the arch on its trimmed
    /// base, as the last build made it. It moves with its arch (the bite,
    /// a move by hand), goes when the scan is replaced or changes jaw, and
    /// is made again by the next build.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub study_models: BTreeMap<ScanRole, BlobRef>,
    /// The aligner shown in the 3D view and the aligners built for it
    /// (`aligners.view.aligner`, `aligners.view.aligner-exact`; the plugin's
    /// `show_aligner` and its preview). A built aligner is kept with the key
    /// of what it was built from and is drawn only while the case gives the
    /// same key.
    #[serde(skip_serializing_if = "crate::AlignerView::is_empty")]
    pub aligner_view: crate::AlignerView,
    /// The scans of the case closed with a base (by the import, or by
    /// "Close with a base"), by the blob of the closed scan: where the scan
    /// as it was read ends in it ([`crate::ScanBase`]). The steps that read
    /// the open border read that part. It holds on a scan moved rigidly
    /// ([`AlignersState::scan_moved`]); a blob with no record is a scan as
    /// it was read (open, or closed by the scanner).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scan_bases: BTreeMap<BlobRef, crate::ScanBase>,
}

/// The same tooth in the other arch, on the same side (the plugin's
/// `manual.other_arch_fdi`): 13 ↔ 43, 24 ↔ 34. The deciduous quadrants
/// follow the same rule (55 ↔ 85, 64 ↔ 74), which the plugin never meets.
pub fn other_arch_fdi(f: Fdi) -> Fdi {
    let q = match f.quadrant() {
        1 => 4,
        4 => 1,
        2 => 3,
        3 => 2,
        5 => 8,
        8 => 5,
        6 => 7,
        _ => 6,
    };
    Fdi::new(q * 10 + f.position()).expect("the same position in the opposite quadrant")
}

/// A tooth label (`0` = gingiva) in the other arch: every tooth number goes
/// to [`other_arch_fdi`], the gingiva stays `0`, and a value that is no tooth
/// becomes gingiva (the plugin's conversion table maps it to `0`).
pub fn other_arch_label(label: u16) -> u16 {
    u8::try_from(label)
        .ok()
        .and_then(|n| Fdi::new(n).ok())
        .map_or(0, |f| u16::from(other_arch_fdi(f).get()))
}

/// The arch `role` in the other jaw (a bite role stays a bite).
fn opposite(role: ScanRole) -> ScanRole {
    match role {
        ScanRole::Upper => ScanRole::Lower,
        ScanRole::Lower => ScanRole::Upper,
        ScanRole::Bite => ScanRole::Bite,
    }
}

/// Ids [`AlignersState::clear_arch`] (and so the `scan_replaced` hook) can
/// return, in the order it returns them. Each names a group of data and is
/// returned only when the group held something for the replaced arch.
///
/// `bite_registered` is not data that goes: it says that the bite
/// registration (or the virtual bite) was applied and is now stale, because
/// the other arch stays where that registration moved it. The flag is reset,
/// the move is NOT undone, and the registration has to be done again with the
/// new arch. The workspace shows the ids by the key
/// `aligners-cleared-<id>` (`_` as `-`).
pub const CLEARED_IDS: [&str; 17] = [
    "bite_registered",
    "seeds",
    "missing",
    "labels",
    "tooth_frames",
    "tooth_geometry",
    "crowns",
    "gingiva",
    "diagnosis",
    "bolton",
    "complexity",
    "setup",
    "stages",
    "review",
    "ramps",
    "visits",
    "study_models",
];

/// `fdi` belongs to the arch of `role` (`false` for a bite role).
pub(crate) fn in_arch(fdi: Fdi, role: ScanRole) -> bool {
    match role {
        ScanRole::Upper => fdi.is_upper(),
        ScanRole::Lower => !fdi.is_upper(),
        ScanRole::Bite => false,
    }
}

/// Removes the entries of `map` whose tooth is in the arch; `true` when some
/// were there.
fn drop_teeth<V>(map: &mut BTreeMap<Fdi, V>, role: ScanRole) -> bool {
    let n = map.len();
    map.retain(|f, _| !in_arch(*f, role));
    map.len() != n
}

/// The last aligner of `stages` in which a tooth moves (its pose differs
/// from the one before it, the rest pose before its first) or an IPR is
/// done; 0 when there is none. The staging keeps the aligners up to there
/// (`dca_algo::schedule::active_stages` with the IPR, in `merge_arch`).
fn last_active_stage(stages: &[Stage]) -> u32 {
    let mut ordered: Vec<&Stage> = stages.iter().collect();
    ordered.sort_by_key(|s| s.index);
    let mut before: BTreeMap<Fdi, Pose> = BTreeMap::new();
    let mut last = 0;
    for s in ordered {
        if !s.ipr.is_empty() {
            last = last.max(s.index);
        }
        for (f, p) in &s.poses {
            let prev = before.insert(*f, *p).unwrap_or(Pose::IDENTITY);
            if !p.approx_eq(&prev, 1e-9, 1e-9) {
                last = last.max(s.index);
            }
        }
    }
    last
}

impl AlignersState {
    /// Forgets the bite of the case: the `bite_registered` flag and the
    /// report of the last bite ([`AlignersState::bite_report`]). `true` when
    /// either was there (the id `bite_registered` reports both).
    pub fn forget_bite(&mut self) -> bool {
        let flag = std::mem::take(&mut self.bite_registered);
        let report = self.bite_report.take().is_some();
        flag || report
    }

    /// The value in force of the review record `kind` of the arch `role`
    /// (the tooth `fdi`, `None` for the whole arch): the clinician's
    /// decision or, while it is to review, the proposal (the plugin's
    /// `review.decided`). `None` when the case has no such record.
    pub fn review_value(&self, role: ScanRole, kind: ReviewKind, fdi: Option<Fdi>) -> Option<&str> {
        self.review
            .iter()
            .find(|r| r.arch == role && r.kind == kind && r.fdi == fdi)
            .map(ReviewRecord::in_force)
            .filter(|v| !v.is_empty())
    }

    /// The teeth of the arch `role` that get an attachment, in the order of
    /// the review: those whose attachment record has a value in force other
    /// than `none` (the plugin's `schedule.build`, bonded before aligner 1).
    pub fn attachment_teeth(&self, role: ScanRole) -> Vec<String> {
        self.review
            .iter()
            .filter(|r| r.arch == role && r.kind == ReviewKind::Attachment)
            .filter(|r| !matches!(r.in_force(), "" | "none"))
            .filter_map(|r| r.fdi.map(|f| f.to_string()))
            .collect()
    }

    /// Drops everything computed from, or entered about, the scan of the
    /// arch `role`, and returns the ids of the groups that held data, in the
    /// order of [`CLEARED_IDS`]. A bite role clears nothing.
    ///
    /// The `bite_registered` flag is reset and reported when it was set (the
    /// registration, or the virtual bite, is stale; the move it applied to
    /// the other arch is kept). Cleared for the arch: the seeds and the teeth
    /// marked missing; the
    /// labels (and the teeth flagged for a rebuild), tooth frames, measures,
    /// closed crowns and gingiva; its space analysis and details; the setup
    /// (poses, locks, IPR, arch line); its poses and IPR in every stage; its
    /// ramps; its control visits, with their scan blobs; its study model.
    ///
    /// Results that need both arches go when either one is replaced: the
    /// Bolton ratios, the complexity score (unless it can only come from the
    /// other arch's details, which stay), and the automatic ramps of the
    /// other arch, which the deep-bite rule derives from the overbite
    /// between the two. The manual ramps, the other arch's own data and the
    /// settings (staging, trim, occlusal, ramp sizes) are kept as they are.
    ///
    /// Stages: as `merge_arch` of the staging command does with an empty
    /// series, the arch's poses and IPR leave every stage, and the aligners
    /// after the end of the other arch's series (its last movement or IPR)
    /// are dropped, with any stage left with neither poses nor IPR. The
    /// other arch keeps its own series as it was: the repeats of its last
    /// pose that only kept up with the longer series of the cleared arch go
    /// with it, as in the plugin, where each arch has its own aligners
    /// (`n_stages`). The stage `index` values are not renumbered, as in
    /// `merge_arch`. Control visits are not renumbered either; the
    /// remaining ones keep their `number`.
    pub fn clear_arch(&mut self, role: ScanRole) -> Vec<&'static str> {
        if role == ScanRole::Bite {
            return Vec::new();
        }
        let mut cleared = Vec::new();
        if self.forget_bite() {
            cleared.push("bite_registered");
        }
        let n = self.seeds.len();
        self.seeds.retain(|s| !in_arch(s.fdi, role));
        if self.seeds.len() != n {
            cleared.push("seeds");
        }
        let n = self.missing.len();
        self.missing.retain(|f| !in_arch(*f, role));
        if self.missing.len() != n {
            cleared.push("missing");
        }
        cleared.extend(self.clear_seed_dependents(role));
        let n = self.visits.len();
        self.visits.retain(|v| v.arch != role);
        if self.visits.len() != n {
            cleared.push("visits");
        }
        if self.study_models.remove(&role).is_some() {
            cleared.push("study_models");
        }
        // a correction of the scan that was replaced
        self.occlusal_flipped.remove(&role);
        cleared
    }

    /// Drops what is computed from the seeds of the arch `role`, keeping the
    /// seeds, the teeth marked missing, the bite flag, the settings and the
    /// control visits (scans of their own), and returns the ids of the groups
    /// that held data, in the order of [`CLEARED_IDS`]. A bite role clears
    /// nothing.
    ///
    /// Call it whenever the seeds of an arch change (automatic numbering,
    /// manual marks): the tooth labels (and the teeth flagged for a rebuild),
    /// and everything [`AlignersState::clear_label_dependents`] drops, no
    /// longer describe the teeth the seeds name. The results that need both
    /// arches (Bolton, complexity score, automatic ramps) go with them, as in
    /// [`AlignersState::clear_arch`], which is this plus the arch's own entries.
    pub fn clear_seed_dependents(&mut self, role: ScanRole) -> Vec<&'static str> {
        if role == ScanRole::Bite {
            return Vec::new();
        }
        let n = self.dirty.len();
        let labels = self.labels.remove(&role).is_some();
        let mut cleared = self.clear_label_dependents(role);
        if labels || self.dirty.len() != n {
            cleared.insert(0, "labels");
        }
        cleared
    }

    /// Forgets what the separation and the check of the arch `role` left in
    /// the case: the doubts and the result of the last run (teeth and
    /// seconds). Call it whenever the colours of the arch are replaced or
    /// corrected by other means than a separation, so the case never says
    /// more than the labels it holds.
    pub fn forget_separation(&mut self, role: ScanRole) {
        self.segmentation_doubts.remove(&role);
        self.segmentation_runs.remove(&role);
    }

    /// The tooth colours, tooth marks, missing teeth and occlusal side the
    /// arch `role` holds now.
    pub fn teeth_basis_now(&self, role: ScanRole) -> TeethBasis {
        let mut seeds: Vec<Seed> = self
            .seeds
            .iter()
            .filter(|s| in_arch(s.fdi, role))
            .copied()
            .collect();
        seeds.sort_by_key(|s| s.fdi);
        TeethBasis {
            labels: self.labels.get(&role).copied(),
            seeds,
            missing: self
                .missing
                .iter()
                .copied()
                .filter(|f| in_arch(*f, role))
                .collect(),
            occlusal_flipped: self.occlusal_flipped.contains(&role),
        }
    }

    /// Notes that the teeth of the arch `role` were just built from what it
    /// holds now.
    pub fn record_teeth_basis(&mut self, role: ScanRole) {
        let basis = self.teeth_basis_now(role);
        self.teeth_basis.insert(role, basis);
    }

    /// The closed tooth `fdi` is the blob `crown`, whose first `scanned`
    /// vertices are scanned ([`AlignersState::crown_scanned`]: a tooth
    /// closed again); the counts of the blobs no tooth uses any more go.
    pub fn record_crown(&mut self, fdi: Fdi, crown: BlobRef, scanned: usize) {
        self.crowns.insert(fdi, crown);
        self.crown_scanned.insert(crown, scanned as u32);
        self.prune_crown_scanned();
    }

    /// The closed solid `old` became `new` with the same vertices in the
    /// same order (moved rigidly or scaled; call it once `crowns` holds
    /// `new`): its scanned count holds on `new`.
    pub fn crown_replaced(&mut self, old: BlobRef, new: BlobRef) {
        if let Some(&n) = self.crown_scanned.get(&old) {
            self.crown_scanned.insert(new, n);
        }
        self.prune_crown_scanned();
    }

    /// Drops the scanned counts of the blobs no tooth uses.
    fn prune_crown_scanned(&mut self) {
        let crowns = &self.crowns;
        self.crown_scanned
            .retain(|b, _| crowns.values().any(|c| c == b));
    }

    /// A tooth of the arch `role` was built again by hand: once no tooth
    /// corrected by hand waits to be rebuilt and the tooth marks, missing
    /// teeth and occlusal side are those the teeth were built from, the
    /// built teeth match the arch again (its colours were only changed tooth
    /// by tooth).
    pub fn rebuilt_by_hand(&mut self, role: ScanRole) {
        let now = self.teeth_basis_now(role);
        let marks_same = self.teeth_basis.get(&role).is_some_and(|b| {
            b.seeds == now.seeds
                && b.missing == now.missing
                && b.occlusal_flipped == now.occlusal_flipped
        });
        if marks_same && !self.dirty.iter().any(|f| in_arch(*f, role)) {
            self.teeth_basis.insert(role, now);
        }
    }

    /// `true` when the arch `role` has built teeth that are out of date: a
    /// tooth corrected by hand waits to be rebuilt, or its tooth colours,
    /// tooth marks or missing teeth changed since the teeth were built (the
    /// brush, the clean-up, a mark placed, moved or deleted, a number
    /// changed, a tooth marked missing or present, the numbering in order,
    /// the marks cleared, Find the teeth or the separation run again).
    /// Moving the arch rigidly (the bite) moves its record along, so it is
    /// no change.
    pub fn teeth_out_of_date(&self, role: ScanRole) -> bool {
        !self.teeth_changes(role).is_empty()
    }

    /// What changed in the arch `role` since its teeth were built, in the
    /// order of [`TeethChange`]; empty when the arch has no built teeth or
    /// they are up to date ([`AlignersState::teeth_out_of_date`]).
    pub fn teeth_changes(&self, role: ScanRole) -> Vec<TeethChange> {
        let mut out = Vec::new();
        if !self.crowns.keys().any(|f| in_arch(*f, role)) {
            return out;
        }
        if self.dirty.iter().any(|f| in_arch(*f, role)) {
            out.push(TeethChange::HandCorrected);
        }
        if let Some(b) = self.teeth_basis.get(&role) {
            let now = self.teeth_basis_now(role);
            if b.labels != now.labels {
                out.push(TeethChange::Colours);
            }
            if b.seeds != now.seeds {
                out.push(TeethChange::Marks);
            }
            if b.missing != now.missing {
                out.push(TeethChange::Missing);
            }
            if b.occlusal_flipped != now.occlusal_flipped {
                out.push(TeethChange::Orientation);
            }
        }
        out
    }

    /// The arches `moved` change jaw (the plugin's `manual.set_arch_steps`
    /// with a new arch): their scans swap, or one scan takes the free arch.
    /// What the case keeps of them is renumbered to the other jaw
    /// ([`other_arch_fdi`], same side, same tooth) and moved to the other
    /// role: the tooth marks, the teeth marked missing (both the marks of
    /// the arch and the "missing teeth" setting of Find the teeth), the
    /// teeth waiting for a rebuild, the key of the labels blob, the occlusal
    /// side turned over by hand, and the control visits (their arch and
    /// their teeth; their scan blobs keep the old numbers and are relabelled
    /// by the caller, as the labels blobs).
    ///
    /// Call it after [`AlignersState::clear_label_dependents`] on every
    /// moved arch: the built teeth and what follows them are made again from
    /// the renumbered labels, never renumbered. The record of what the teeth
    /// were built from goes with them, and so does the study model of each
    /// moved arch and of the arch it lands on (built for the other jaw).
    ///
    /// Entries of the arch a single scan moves to (it had no scan) are kept
    /// for the sets and replaced for the rest, as in the plugin, where the
    /// "missing teeth" list keeps the numbers already in the new arch.
    pub fn renumber_arches(&mut self, moved: &BTreeSet<ScanRole>) {
        let moved: BTreeSet<ScanRole> = moved
            .iter()
            .copied()
            .filter(|r| *r != ScanRole::Bite)
            .collect();
        if moved.is_empty() {
            return;
        }
        let moves = |f: Fdi| moved.iter().any(|r| in_arch(f, *r));
        let map = |f: Fdi| if moves(f) { other_arch_fdi(f) } else { f };
        let role_map = |r: ScanRole| {
            if moved.contains(&r) { opposite(r) } else { r }
        };
        // the arches the moved ones go to and that did not move themselves
        let landing: BTreeSet<ScanRole> = moved
            .iter()
            .map(|r| opposite(*r))
            .filter(|r| !moved.contains(r))
            .collect();
        let lands = |f: Fdi| landing.iter().any(|r| in_arch(f, *r));

        self.seeds.retain(|s| !lands(s.fdi));
        for s in &mut self.seeds {
            s.fdi = map(s.fdi);
        }
        self.missing = self.missing.iter().map(|f| map(*f)).collect();
        let find = &mut self.settings.find_teeth.missing;
        let mut renumbered: Vec<u8> = find
            .iter()
            .map(|&n| Fdi::new(n).map_or(n, |f| map(f).get()))
            .collect();
        renumbered.sort_unstable();
        renumbered.dedup();
        *find = renumbered;
        self.dirty = self.dirty.iter().map(|f| map(*f)).collect();

        let labels = std::mem::take(&mut self.labels);
        self.labels = labels
            .into_iter()
            .filter(|(r, _)| !landing.contains(r))
            .map(|(r, v)| (role_map(r), v))
            .collect();
        self.teeth_basis
            .retain(|r, _| !moved.contains(r) && !landing.contains(r));
        self.study_models
            .retain(|r, _| !moved.contains(r) && !landing.contains(r));
        self.occlusal_flipped = self
            .occlusal_flipped
            .iter()
            .filter(|r| !landing.contains(r))
            .map(|r| role_map(*r))
            .collect();
        let ends = std::mem::take(&mut self.aligner_end);
        self.aligner_end = ends
            .into_iter()
            .filter(|(r, _)| !landing.contains(r))
            .map(|(r, e)| {
                if moved.contains(&r) {
                    let other = |f: Option<Fdi>| f.map(other_arch_fdi);
                    let e = AlignerEnd {
                        right: other(e.right),
                        left: other(e.left),
                    };
                    (opposite(r), e)
                } else {
                    (r, e)
                }
            })
            .collect();

        for v in &mut self.visits {
            if !moved.contains(&v.arch) {
                continue;
            }
            v.arch = opposite(v.arch);
            v.teeth = std::mem::take(&mut v.teeth)
                .into_iter()
                .map(|(f, t)| (other_arch_fdi(f), t))
                .collect();
            v.untracked = v.untracked.iter().map(|f| other_arch_fdi(*f)).collect();
            v.speed = std::mem::take(&mut v.speed)
                .into_iter()
                .map(|(f, s)| (other_arch_fdi(f), s))
                .collect();
        }
    }

    /// Drops what is computed from the labels of the arch `role`, keeping
    /// the labels themselves, and returns the ids of the groups that held
    /// data, in the order of [`CLEARED_IDS`] (never `labels`). A bite role
    /// clears nothing.
    ///
    /// Call it when the labels of an arch are made again from the seeds (the
    /// automatic separation): the teeth flagged for a rebuild, the tooth
    /// frames, measures, closed crowns and gingiva of the arch, its space
    /// analysis, the setup, its poses in every stage (with the result of its
    /// last staging and its teeth slowed by hand, under `stages`), its
    /// clinical review (`review`) and its ramps describe the old teeth, and
    /// the results that need both arches
    /// (Bolton, complexity score, automatic ramps) go with them. The doubts of the
    /// check of the separated teeth and the result of the last separation of
    /// the arch go too, without an id (they were about the old labels).
    pub fn clear_label_dependents(&mut self, role: ScanRole) -> Vec<&'static str> {
        let other = match role {
            ScanRole::Upper => ScanRole::Lower,
            ScanRole::Lower => ScanRole::Upper,
            ScanRole::Bite => return Vec::new(),
        };
        let mut cleared = Vec::new();
        let mut note = |id: &'static str, held: bool| {
            if held {
                cleared.push(id);
            }
        };

        self.dirty.retain(|f| !in_arch(*f, role));
        self.forget_separation(role);

        note("tooth_frames", drop_teeth(&mut self.tooth_frames, role));
        note("tooth_geometry", drop_teeth(&mut self.tooth_geometry, role));
        note("crowns", drop_teeth(&mut self.crowns, role));
        note("gingiva", self.gingiva.remove(&role).is_some());

        // the score is the largest CAT-CAT score of the arches: it survives
        // only when the other arch's details can account for it
        let score_uses_arch = self.diagnosis.details.contains_key(&role)
            || !self.diagnosis.details.contains_key(&other);
        let space = self.diagnosis.arches.remove(&role).is_some();
        let details = self.diagnosis.details.remove(&role).is_some();
        // the values written by hand were about the teeth that go
        let edits = self.diagnosis.edits.remove(&role).is_some();
        note("diagnosis", space || details || edits);
        let bolton = self.diagnosis.bolton_anterior.take().is_some()
            | self.diagnosis.bolton_overall.take().is_some();
        note("bolton", bolton);
        let score = score_uses_arch && self.diagnosis.complexity_score.take().is_some();
        note("complexity", score);

        let setup = &mut self.setup;
        let n = (setup.locked.len(), setup.ipr.len());
        setup.locked.retain(|f| !in_arch(*f, role));
        setup
            .ipr
            .retain(|r| !r.between.iter().any(|f| in_arch(*f, role)));
        let held = drop_teeth(&mut setup.poses, role)
            | (n != (setup.locked.len(), setup.ipr.len()))
            | setup.lines.remove(&role).is_some()
            | setup.drawn.remove(&role).is_some()
            | setup.results.remove(&role).is_some();
        note("setup", held);

        // the teeth slowed by hand were teeth of the old labels
        let slowed = drop_teeth(&mut self.tooth_speed, role);
        note("stages", self.drop_stages_of(role) | slowed);

        // the classifications were made on the teeth that go
        let n = self.review.len();
        self.review.retain(|r| r.arch != role);
        note("review", self.review.len() != n);

        let ramps = &mut self.aligner_bite.ramps.ramps;
        let n = ramps.len();
        ramps.retain(|r| !in_arch(r.fdi, role) && r.kind != RampKind::Auto);
        note("ramps", ramps.len() != n);

        cleared
    }

    /// Takes the final position of the arch `role` back to the initial
    /// position, as the plugin's `case_plan.reset_setup` (`oa.reset_setup`):
    /// every tooth of the arch back at rest, its IPR, its arch line and the
    /// result of its last final position gone, and the aligners computed
    /// from it (the arch's poses and IPR leave every stage, as
    /// [`AlignersState::clear_label_dependents`] does). The locks, the line
    /// drawn by the clinician and the settings stay. Returns the ids of the
    /// groups that held data, in the order of [`CLEARED_IDS`] (`setup`,
    /// `stages`). A bite role clears nothing.
    pub fn reset_setup(&mut self, role: ScanRole) -> Vec<&'static str> {
        if role == ScanRole::Bite {
            return Vec::new();
        }
        let mut cleared = Vec::new();
        let setup = &mut self.setup;
        let n = setup.ipr.len();
        setup
            .ipr
            .retain(|r| !r.between.iter().any(|f| in_arch(*f, role)));
        let held = drop_teeth(&mut setup.poses, role)
            | (n != setup.ipr.len())
            | setup.lines.remove(&role).is_some()
            | setup.results.remove(&role).is_some();
        if held {
            cleared.push("setup");
        }
        if self.drop_stages_of(role) {
            cleared.push("stages");
        }
        cleared
    }

    /// The poses and IPR of the arch `role` leave every stage, with the
    /// result of its last staging; `true` when some were there. The other
    /// arch keeps its own series, not the repeats of its last pose that kept
    /// up with this arch (the staging's `merge_arch`), and the stages left
    /// empty go.
    fn drop_stages_of(&mut self, role: ScanRole) -> bool {
        let summary = self.staging_results.remove(&role).is_some();
        let mut held = false;
        for s in &mut self.stages {
            let n = s.ipr.len();
            held |= drop_teeth(&mut s.poses, role);
            s.ipr
                .retain(|r| !r.between.iter().any(|f| in_arch(*f, role)));
            held |= n != s.ipr.len();
        }
        if held {
            let end = last_active_stage(&self.stages);
            self.stages.retain(|s| s.index <= end);
        }
        self.stages
            .retain(|s| !s.poses.is_empty() || !s.ipr.is_empty());
        held || summary
    }

    /// Replaces the seeds of the arch `role` with `seeds` (the other arch
    /// keeps its own). When the arch's seeds really change, what depends on
    /// them goes ([`AlignersState::clear_seed_dependents`]) and its ids are
    /// returned; the same seeds again clear nothing. This is for the
    /// automatic numbering, which proposes a new set of marks.
    pub fn replace_arch_seeds(&mut self, role: ScanRole, seeds: &[Seed]) -> Vec<&'static str> {
        if self.set_arch_seeds(role, seeds) {
            self.clear_seed_dependents(role)
        } else {
            Vec::new()
        }
    }

    /// Sets the seeds of the arch `role` to `seeds` as edited by hand (a
    /// mark added, moved or deleted, the numbering in order, the marks
    /// cleared), keeping everything computed from the old ones: as in the
    /// plugin (`case_core.add_seed`, `steps.mark_manual(obj, "semi")`), the
    /// colours of the teeth and what follows them stay and are only out of
    /// date until the user runs the segmentation again. Returns the ids of
    /// the groups that are now out of date ([`AlignersState::seed_dependents`]),
    /// empty when the seeds did not change.
    pub fn edit_arch_seeds(&mut self, role: ScanRole, seeds: &[Seed]) -> Vec<&'static str> {
        if self.set_arch_seeds(role, seeds) {
            self.seed_dependents(role)
        } else {
            Vec::new()
        }
    }

    /// The ids of the groups [`AlignersState::clear_seed_dependents`] would
    /// drop for the arch `role`, in the order of [`CLEARED_IDS`], without
    /// dropping anything.
    pub fn seed_dependents(&self, role: ScanRole) -> Vec<&'static str> {
        self.clone().clear_seed_dependents(role)
    }

    /// Puts `seeds` in the place of the seeds of the arch `role`; `true` when
    /// they differ (in any order) from the ones it had.
    fn set_arch_seeds(&mut self, role: ScanRole, seeds: &[Seed]) -> bool {
        let key = |s: &Seed| s.fdi.get();
        let mut old: Vec<Seed> = self
            .seeds
            .iter()
            .filter(|s| in_arch(s.fdi, role))
            .copied()
            .collect();
        let mut new = seeds.to_vec();
        old.sort_by_key(key);
        new.sort_by_key(key);
        self.seeds.retain(|s| !in_arch(s.fdi, role));
        self.seeds.extend_from_slice(seeds);
        old != new
    }
}

impl dc_case::ModuleData for AlignersState {
    fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_box(&self) -> Box<dyn dc_case::ModuleData> {
        Box::new(self.clone())
    }
    fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
    /// The label, closed crown, gingiva, control-visit scan, study model and
    /// view aligner blobs are the blob references in the state (the other
    /// fields are plain numbers and poses).
    fn referenced_blobs(&self) -> Vec<BlobRef> {
        self.labels
            .values()
            .chain(self.crowns.values())
            .chain(self.gingiva.values())
            .chain(self.visits.iter().filter_map(|v| v.scan.as_ref()))
            .chain(self.study_models.values())
            .chain(self.aligner_view.bases.iter().map(|b| &b.mesh))
            .copied()
            .collect()
    }

    /// Clears what was computed from the replaced scan, see
    /// [`AlignersState::clear_arch`] for the ids and what is kept.
    ///
    /// A replaced bite scan clears nothing and returns no id. Bite
    /// registration moves an arch and bakes the move into that arch's scan
    /// mesh (`move_arch`), so replacing the bite or an arch does not undo the
    /// move already applied to the other arch: the other arch stays exactly
    /// as it is, and the ids returned are only the groups that were really
    /// cleared. This is not a reset to the imported situation.
    fn scan_replaced(&mut self, role: ScanRole) -> Vec<&'static str> {
        self.clear_arch(role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ModuleState;
    use glam::DQuat;

    fn sample() -> AlignersState {
        let f11 = Fdi::new(11).unwrap();
        let f21 = Fdi::new(21).unwrap();
        let mut st = AlignersState::default();
        st.seeds.push(Seed {
            fdi: f11,
            position: DVec3::new(1.0, -30.0, 2.0),
        });
        st.labels
            .insert(ScanRole::Upper, BlobRef::of_bytes(b"labels"));
        st.tooth_frames.insert(
            f11,
            ToothFrame {
                pose: Pose::new(DVec3::X, DQuat::from_rotation_z(0.2)),
                center_of_resistance: DVec3::new(0.0, 0.0, 6.0),
            },
        );
        st.diagnosis.arches.insert(
            ScanRole::Upper,
            ArchSpace {
                available_mm: 70.0,
                required_mm: 73.5,
                little_index_mm: 4.2,
            },
        );
        st.setup
            .poses
            .insert(f11, Pose::from_translation(DVec3::new(0.5, 0.0, 0.0)));
        st.setup.locked.insert(f21);
        let mut stage = Stage::new(1);
        stage
            .poses
            .insert(f11, Pose::from_translation(DVec3::new(0.25, 0.0, 0.0)));
        stage.ipr.push(IprStep {
            between: [f11, f21],
            amount_mm: 0.2,
        });
        st.stages.push(stage);
        st
    }

    #[test]
    fn seed_dependents_go_when_the_seeds_of_the_arch_change() {
        let mut st = sample();
        let f11 = Fdi::new(11).unwrap();
        let same = st.seeds.clone();
        // the same seeds again: nothing goes
        assert!(st.replace_arch_seeds(ScanRole::Upper, &same).is_empty());
        assert!(st.labels.contains_key(&ScanRole::Upper) && !st.setup.poses.is_empty());
        // another arch: the upper results stay
        assert!(st.replace_arch_seeds(ScanRole::Lower, &[]).is_empty());
        assert!(st.labels.contains_key(&ScanRole::Upper));
        // other seeds: the results go, the seeds, the teeth marked missing and
        // the settings stay
        st.missing.insert(Fdi::new(18).unwrap());
        let moved = [Seed {
            fdi: f11,
            position: DVec3::new(2.0, -30.0, 2.0),
        }];
        let cleared = st.replace_arch_seeds(ScanRole::Upper, &moved);
        assert_eq!(
            cleared,
            ["labels", "tooth_frames", "diagnosis", "setup", "stages"]
        );
        assert_eq!(st.seeds, moved);
        assert!(st.missing.contains(&Fdi::new(18).unwrap()));
        assert!(st.labels.is_empty() && st.tooth_frames.is_empty() && st.stages.is_empty());
        assert!(st.diagnosis.arches.is_empty() && st.setup.poses.is_empty());
        // the cleared ids are a subset of the ones the workspace words
        assert!(cleared.iter().all(|c| CLEARED_IDS.contains(c)));
    }

    #[test]
    fn seeds_edited_by_hand_keep_the_results_and_say_what_is_out_of_date() {
        let mut st = sample();
        let before = st.clone();
        let same = st.seeds.clone();
        assert!(st.edit_arch_seeds(ScanRole::Upper, &same).is_empty());
        assert_eq!(st, before);
        let moved = [Seed {
            fdi: Fdi::new(11).unwrap(),
            position: DVec3::new(2.0, -30.0, 2.0),
        }];
        let outdated = st.edit_arch_seeds(ScanRole::Upper, &moved);
        // the same ids the automatic numbering would clear
        assert_eq!(outdated, before.seed_dependents(ScanRole::Upper));
        assert_eq!(
            outdated,
            ["labels", "tooth_frames", "diagnosis", "setup", "stages"]
        );
        // but only the seeds changed
        assert_eq!(st.seeds, moved);
        let mut expect = before.clone();
        expect.seeds = moved.to_vec();
        assert_eq!(st, expect);
        // asking does not change anything
        assert_eq!(st.seed_dependents(ScanRole::Upper), outdated);
        assert_eq!(st, expect);
    }

    #[test]
    fn label_dependents_go_and_the_labels_and_seeds_stay() {
        let mut st = sample();
        let f11 = Fdi::new(11).unwrap();
        st.dirty.insert(f11);
        st.crowns.insert(f11, BlobRef::of_bytes(b"crown"));
        st.segmentation_doubts
            .insert(ScanRole::Upper, vec!["11 is narrow".to_owned()]);
        st.segmentation_doubts
            .insert(ScanRole::Lower, vec!["41 is wide".to_owned()]);
        let before = st.clone();
        // the other arch: nothing of the upper one goes
        assert!(st.clear_label_dependents(ScanRole::Lower).is_empty());
        assert!(!st.segmentation_doubts.contains_key(&ScanRole::Lower));
        assert!(st.crowns.contains_key(&f11) && st.dirty.contains(&f11));
        let cleared = st.clear_label_dependents(ScanRole::Upper);
        assert_eq!(
            cleared,
            ["tooth_frames", "crowns", "diagnosis", "setup", "stages"]
        );
        assert_eq!(st.labels, before.labels);
        assert_eq!(st.seeds, before.seeds);
        assert!(st.crowns.is_empty() && st.dirty.is_empty() && st.stages.is_empty());
        assert!(st.segmentation_doubts.is_empty());
        // the seeds' dependents are the labels plus these
        let mut all = vec!["labels"];
        all.extend(&cleared);
        assert_eq!(before.seed_dependents(ScanRole::Upper), all);
        assert!(st.clear_label_dependents(ScanRole::Bite).is_empty());
    }

    #[test]
    fn a_case_saved_without_doubts_loads_with_none() {
        let mut json = serde_json::to_value(sample()).unwrap();
        json.as_object_mut().unwrap().remove("segmentation_doubts");
        let back: AlignersState = serde_json::from_value(json).unwrap();
        assert!(back.segmentation_doubts.is_empty());
    }

    #[test]
    fn the_separation_result_round_trips_and_an_old_case_loads_without_it() {
        let mut st = sample();
        st.segmentation_runs.insert(
            ScanRole::Upper,
            SegmentationRun {
                teeth: 14,
                seconds: 9.6,
            },
        );
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["segmentation_runs"]["upper"]["teeth"], 14);
        let back: AlignersState = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, st);
        // a case saved before the field existed
        let mut old = json;
        old.as_object_mut().unwrap().remove("segmentation_runs");
        let back: AlignersState = serde_json::from_value(old).unwrap();
        assert!(back.segmentation_runs.is_empty());
    }

    #[test]
    fn what_clears_the_separation_result_of_an_arch_leaves_the_other() {
        let run = SegmentationRun {
            teeth: 14,
            seconds: 3.0,
        };
        let both = || {
            let mut st = AlignersState::default();
            st.segmentation_runs.insert(ScanRole::Upper, run);
            st.segmentation_runs.insert(ScanRole::Lower, run);
            st
        };
        let mut st = both();
        st.forget_separation(ScanRole::Upper);
        assert!(!st.segmentation_runs.contains_key(&ScanRole::Upper));
        assert!(st.segmentation_runs.contains_key(&ScanRole::Lower));
        let mut st = both();
        st.clear_arch(ScanRole::Lower);
        assert!(!st.segmentation_runs.contains_key(&ScanRole::Lower));
        assert!(st.segmentation_runs.contains_key(&ScanRole::Upper));
        let mut st = both();
        st.clear_seed_dependents(ScanRole::Upper);
        assert!(!st.segmentation_runs.contains_key(&ScanRole::Upper));
        let mut st = both();
        st.clear_label_dependents(ScanRole::Lower);
        assert!(!st.segmentation_runs.contains_key(&ScanRole::Lower));
        assert!(st.segmentation_runs.contains_key(&ScanRole::Upper));
        // a bite role clears nothing
        let mut st = both();
        st.clear_label_dependents(ScanRole::Bite);
        assert_eq!(st, both());
    }

    #[test]
    fn values_written_by_hand_are_kept_apart_and_win() {
        let mut st = sample();
        let up = ScanRole::Upper;
        // nothing written: the computed values, and the bytes of an old project
        assert!(st.diagnosis.edits_of(up).is_empty());
        assert_eq!(st.diagnosis.crowding_mm(up), Some(3.5));
        assert_eq!(st.diagnosis.little_index_mm(up), Some(4.2));
        assert_eq!(st.diagnosis.crowding_mm(ScanRole::Lower), None);
        assert!(serde_json::to_value(&st).unwrap()["diagnosis"]["edits"].is_null());
        // written: they win in the accessors, the computed ones stay
        st.diagnosis.edits.insert(
            up,
            ArchEdits {
                discrepancy_mm: Some(-6.0),
                midline_offset_mm: Some(2.5),
                ..ArchEdits::default()
            },
        );
        assert_eq!(st.diagnosis.crowding_mm(up), Some(6.0));
        assert_eq!(st.diagnosis.little_index_mm(up), Some(4.2));
        assert_eq!(st.diagnosis.midline_offset_mm(up), 2.5);
        assert_eq!(st.diagnosis.largest_space_mm(up), 0.0);
        assert_eq!(st.diagnosis.arches[&up].crowding_mm(), 3.5);
        // saved and read back; a project without the field reads as none
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["diagnosis"]["edits"]["upper"]["discrepancy_mm"], -6.0);
        assert!(json["diagnosis"]["edits"]["upper"]["little_index_mm"].is_null());
        let back: AlignersState = serde_json::from_value(json).unwrap();
        assert_eq!(back, st);
        let mut old = serde_json::to_value(sample()).unwrap();
        old["diagnosis"].as_object_mut().unwrap().remove("edits");
        let old: AlignersState = serde_json::from_value(old).unwrap();
        assert!(old.diagnosis.edits.is_empty());
        // a new separation of the arch clears them with its diagnosis
        let cleared = st.clear_label_dependents(up);
        assert!(cleared.contains(&"diagnosis"));
        assert!(st.diagnosis.edits.is_empty());
    }

    #[test]
    fn json_round_trip() {
        let st = sample();
        let json = serde_json::to_value(&st).unwrap();
        // Fdi keys and ScanRole keys are written as strings
        assert!(json["tooth_frames"]["11"].is_object());
        assert!(json["labels"]["upper"].is_string());
        let back: AlignersState = serde_json::from_value(json).unwrap();
        assert_eq!(back, st);
        assert_eq!(st.diagnosis.arches[&ScanRole::Upper].crowding_mm(), 3.5);
    }

    #[test]
    fn label_blobs_survive_pruning() {
        use dc_case::CaseState;
        use dc_mesh::TriMesh;
        let mut case = CaseState::default();
        let labels = case.blobs.insert(TriMesh::new(
            vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
            vec![[0, 1, 2]],
        ));
        let garbage = case.blobs.insert(TriMesh::new(
            vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z],
            vec![[0, 1, 2]],
        ));
        let mut st = AlignersState::default();
        st.labels.insert(ScanRole::Upper, labels);
        case.set_module("aligners", ModuleState::new(st));
        let used = case.referenced_blobs();
        assert!(used.contains(&labels) && !used.contains(&garbage));
        // this is what a .dcprj save does
        let keep = used.iter().copied().collect();
        case.blobs.retain_only(&keep);
        assert!(case.blobs.contains(&labels));
        assert!(!case.blobs.contains(&garbage));
    }

    #[test]
    fn crown_and_gingiva_blobs_survive_pruning() {
        use dc_case::CaseState;
        use dc_mesh::TriMesh;
        let tri = |z: f32| {
            TriMesh::new(
                vec![
                    glam::Vec3::ZERO,
                    glam::Vec3::X,
                    glam::Vec3::new(0.0, 1.0, z),
                ],
                vec![[0, 1, 2]],
            )
        };
        let mut case = CaseState::default();
        let crown = case.blobs.insert(tri(1.0));
        let gum = case.blobs.insert(tri(2.0));
        let garbage = case.blobs.insert(tri(3.0));
        let mut st = AlignersState::default();
        st.record_crown(Fdi::new(11).unwrap(), crown, 2);
        st.gingiva.insert(ScanRole::Upper, gum);
        case.set_module("aligners", ModuleState::new(st.clone()));
        let used = case.referenced_blobs();
        assert!(used.contains(&crown) && used.contains(&gum) && !used.contains(&garbage));
        let keep = used.iter().copied().collect();
        case.blobs.retain_only(&keep);
        assert!(case.blobs.contains(&crown) && case.blobs.contains(&gum));
        assert!(!case.blobs.contains(&garbage));
        // and the JSON keys are the tooth numbers, the arches and the blobs;
        // the scanned count of a tooth is saved and read back
        let json = serde_json::to_value(&st).unwrap();
        assert!(json["crowns"]["11"].is_string() && json["gingiva"]["upper"].is_string());
        assert_eq!(json["crown_scanned"][crown.to_hex()], 2);
        let back: AlignersState = serde_json::from_value(json).unwrap();
        assert_eq!(back, st);
    }

    #[test]
    fn empty_json_gives_defaults() {
        let st: AlignersState = serde_json::from_str("{}").unwrap();
        assert_eq!(st, AlignersState::default());
    }

    #[test]
    fn works_as_module_data() {
        let state = ModuleState::new(sample());
        assert_eq!(state.data().schema_version(), SCHEMA_VERSION);
        assert_eq!(state.downcast_ref::<AlignersState>(), Some(&sample()));
        assert_eq!(
            state.to_json().unwrap(),
            serde_json::to_value(sample()).unwrap()
        );
    }

    #[test]
    fn built_teeth_are_out_of_date_when_the_colours_or_marks_of_their_arch_change() {
        let (f31, f32) = (Fdi::new(31).unwrap(), Fdi::new(32).unwrap());
        let mut st = AlignersState::default();
        st.seeds.push(Seed {
            fdi: f31,
            position: DVec3::new(1.0, 2.0, 3.0),
        });
        st.seeds.push(Seed {
            fdi: f32,
            position: DVec3::new(4.0, 5.0, 6.0),
        });
        st.labels
            .insert(ScanRole::Lower, BlobRef::of_bytes(b"labels"));
        // nothing built: never out of date
        assert!(!st.teeth_out_of_date(ScanRole::Lower));
        st.crowns.insert(f31, BlobRef::of_bytes(b"crown"));
        // built by an older version (no basis): only the hand corrections count
        assert!(!st.teeth_out_of_date(ScanRole::Lower));
        st.record_teeth_basis(ScanRole::Lower);
        let built = st.clone();
        assert!(!built.teeth_out_of_date(ScanRole::Lower));
        type Change = fn(&mut AlignersState);
        use TeethChange::{Colours, Marks, Missing, Orientation};
        let changes: [(&str, TeethChange, Change); 7] = [
            ("colours", Colours, |s| {
                s.labels
                    .insert(ScanRole::Lower, BlobRef::of_bytes(b"painted"));
            }),
            ("mark moved", Marks, |s| s.seeds[1].position.x += 0.25),
            ("mark deleted", Marks, |s| {
                s.seeds.remove(1);
            }),
            ("mark added", Marks, |s| {
                s.seeds.push(Seed {
                    fdi: Fdi::new(33).unwrap(),
                    position: DVec3::ZERO,
                });
            }),
            ("number changed", Marks, |s| {
                s.seeds[1].fdi = Fdi::new(34).unwrap()
            }),
            ("missing", Missing, |s| {
                s.missing.insert(Fdi::new(35).unwrap());
            }),
            ("occlusal side turned over", Orientation, |s| {
                s.occlusal_flipped.insert(ScanRole::Lower);
            }),
        ];
        for (what, why, change) in changes {
            let mut s = built.clone();
            change(&mut s);
            assert!(s.teeth_out_of_date(ScanRole::Lower), "{what}");
            assert_eq!(s.teeth_changes(ScanRole::Lower), [why], "{what}");
            assert!(!s.teeth_out_of_date(ScanRole::Upper), "{what}: upper");
            // it comes back from the project file
            let back: AlignersState =
                serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
            assert!(back.teeth_out_of_date(ScanRole::Lower), "{what}: reopened");
            // built again: up to date
            s.record_teeth_basis(ScanRole::Lower);
            assert!(!s.teeth_out_of_date(ScanRole::Lower), "{what}: rebuilt");
        }
        // a tooth painted by hand waits to be rebuilt; rebuilt by hand, the
        // teeth match again
        let mut s = built.clone();
        s.labels
            .insert(ScanRole::Lower, BlobRef::of_bytes(b"brushed"));
        s.dirty.insert(f31);
        assert_eq!(
            s.teeth_changes(ScanRole::Lower),
            [TeethChange::HandCorrected, TeethChange::Colours]
        );
        s.dirty.remove(&f31);
        s.rebuilt_by_hand(ScanRole::Lower);
        assert!(!s.teeth_out_of_date(ScanRole::Lower));
        // ...but not while a mark changed too
        let mut s = built;
        s.seeds[0].position.y += 1.0;
        s.rebuilt_by_hand(ScanRole::Lower);
        assert!(s.teeth_out_of_date(ScanRole::Lower));
    }

    #[test]
    fn the_other_arch_follows_the_plugins_quadrant_table() {
        // manual._QUAD_SWAP = {1: 4, 4: 1, 2: 3, 3: 2}: same side, same tooth
        for (a, b) in [(11, 41), (18, 48), (21, 31), (27, 37), (33, 23), (46, 16)] {
            let f = Fdi::new(a).unwrap();
            assert_eq!(other_arch_fdi(f).get(), b, "{a}");
            assert_eq!(other_arch_fdi(other_arch_fdi(f)), f, "{a} and back");
            assert_eq!(other_arch_fdi(f).is_right(), f.is_right(), "{a}: side");
        }
        for f in Fdi::PERMANENT {
            assert_ne!(other_arch_fdi(f).is_upper(), f.is_upper(), "{f}");
        }
        // the labels: the gingiva stays gingiva, a value that is no tooth goes to it
        assert_eq!(other_arch_label(0), 0);
        assert_eq!(other_arch_label(36), 26);
        assert_eq!(other_arch_label(19), 0);
        assert_eq!(other_arch_label(300), 0);
    }

    #[test]
    fn an_arch_that_changes_jaw_takes_the_other_jaws_numbers() {
        let f = |n: u8| Fdi::new(n).unwrap();
        let (lower, upper) = (ScanRole::Lower, ScanRole::Upper);
        let mut st = AlignersState::default();
        for n in [31, 36, 41, 46] {
            st.seeds.push(Seed {
                fdi: f(n),
                position: DVec3::splat(f64::from(n)),
            });
        }
        // a stray mark of the arch the scan moves to (it has no scan)
        st.seeds.push(Seed {
            fdi: f(11),
            position: DVec3::ZERO,
        });
        st.missing.extend([f(35), f(17)]);
        st.settings.find_teeth.missing = vec![17, 35, 45];
        st.dirty.insert(f(36));
        st.labels.insert(lower, BlobRef::of_bytes(b"lower labels"));
        st.teeth_basis.insert(lower, TeethBasis::default());
        st.occlusal_flipped.insert(lower);
        let visit: Visit = serde_json::from_value(serde_json::json!({
            "number": 1, "date": "2026-11-14", "arch": "lower", "stage": 1,
            "source_name": "v.stl", "registration_rms_mm": 0.1, "thresholds": [0.5, 2.0],
            "teeth": {}, "untracked": [36], "on_track": 0, "decision": "on_track",
            "speed": { "41": 0.5 }, "scan": null, "segmented": false,
        }))
        .unwrap();
        st.visits.push(visit);

        st.renumber_arches(&BTreeSet::from([lower]));
        let mut seeds: Vec<(u8, f64)> = st
            .seeds
            .iter()
            .map(|s| (s.fdi.get(), s.position.x))
            .collect();
        seeds.sort_by_key(|s| s.0);
        // each mark keeps its place and takes the number of the same tooth above;
        // the stray mark of the free arch goes
        assert_eq!(seeds, [(11, 41.0), (16, 46.0), (21, 31.0), (26, 36.0)]);
        // the plugin: `other_arch_fdi(f) if arch_of(f) != arch else f`
        assert_eq!(
            st.missing.iter().map(|f| f.get()).collect::<Vec<_>>(),
            [17, 25]
        );
        assert_eq!(st.settings.find_teeth.missing, [15, 17, 25]);
        assert_eq!(st.dirty, BTreeSet::from([f(26)]));
        assert_eq!(
            st.labels.keys().copied().collect::<Vec<_>>(),
            [upper],
            "the labels blob is the upper arch's now"
        );
        assert!(st.teeth_basis.is_empty());
        assert_eq!(st.occlusal_flipped, BTreeSet::from([upper]));
        let v = &st.visits[0];
        assert_eq!(v.arch, upper);
        assert_eq!(v.untracked, [f(26)]);
        assert_eq!(v.speed.keys().copied().collect::<Vec<_>>(), [f(11)]);

        // two scans swap: both arches renumber, nothing merges
        let mut st = AlignersState::default();
        for n in [11, 21, 31, 41] {
            st.seeds.push(Seed {
                fdi: f(n),
                position: DVec3::splat(f64::from(n)),
            });
        }
        st.labels.insert(lower, BlobRef::of_bytes(b"L"));
        st.labels.insert(upper, BlobRef::of_bytes(b"U"));
        st.missing.extend([f(18), f(38)]);
        st.renumber_arches(&BTreeSet::from([lower, upper]));
        let mut seeds: Vec<(u8, f64)> = st
            .seeds
            .iter()
            .map(|s| (s.fdi.get(), s.position.x))
            .collect();
        seeds.sort_by_key(|s| s.0);
        assert_eq!(seeds, [(11, 41.0), (21, 31.0), (31, 21.0), (41, 11.0)]);
        assert_eq!(st.labels[&upper], BlobRef::of_bytes(b"L"));
        assert_eq!(st.labels[&lower], BlobRef::of_bytes(b"U"));
        assert_eq!(st.missing, BTreeSet::from([f(28), f(48)]));
        // a bite role moves nothing
        let before = st.clone();
        st.renumber_arches(&BTreeSet::from([ScanRole::Bite]));
        assert_eq!(st, before);
    }

    #[test]
    fn a_replaced_scan_loses_its_occlusal_flip() {
        let mut st = AlignersState::default();
        st.occlusal_flipped
            .extend([ScanRole::Lower, ScanRole::Upper]);
        st.clear_arch(ScanRole::Lower);
        assert_eq!(st.occlusal_flipped, BTreeSet::from([ScanRole::Upper]));
    }
}
