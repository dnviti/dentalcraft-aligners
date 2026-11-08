//! The module's state stored in the case.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{BlobRef, Fdi, Pose, ScanRole};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{AlignerBite, RampKind, SCHEMA_VERSION, Settings, TrimParams, Visit};

/// A seed point: the centre of the occlusal face of a tooth, with its
/// number. Seeds tell the segmentation which teeth exist and where.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    /// Tooth number.
    pub fdi: Fdi,
    /// Position in case coordinates (mm).
    pub position: DVec3,
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
    /// Complexity score of the case.
    pub complexity_score: Option<f64>,
    /// Further results per arch.
    pub details: BTreeMap<ScanRole, ArchDetails>,
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
    /// Trim line parameters.
    pub trim: TrimParams,
    /// Settings.
    pub settings: Settings,
    /// Bite with the aligners worn: occlusal settings and ramps.
    pub aligner_bite: AlignerBite,
    /// Closed solid of each reconstructed tooth (a mesh blob in case
    /// coordinates: watertight, outward normals).
    pub crowns: BTreeMap<Fdi, BlobRef>,
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
    /// What the check of the separated teeth
    /// (`aligners.preparation.segmentation.check`) left implausible in each
    /// arch, in words (its problems, then its notes): the plugin's
    /// `segmentation_doubts`. The export and the report warn about them
    /// (the aligners must not be delivered without checking the tooth marks
    /// and their numbers). They go when the arch is separated again, its
    /// colours are corrected by hand, its teeth are rebuilt by hand, or what
    /// depends on its labels is cleared.
    pub segmentation_doubts: BTreeMap<ScanRole, Vec<String>>,
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
pub const CLEARED_IDS: [&str; 15] = [
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
    "ramps",
    "visits",
];

/// `fdi` belongs to the arch of `role` (`false` for a bite role).
fn in_arch(fdi: Fdi, role: ScanRole) -> bool {
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

impl AlignersState {
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
    /// ramps; its control visits, with their scan blobs.
    ///
    /// Results that need both arches go when either one is replaced: the
    /// Bolton ratios, the complexity score (unless it can only come from the
    /// other arch's details, which stay), and the automatic ramps of the
    /// other arch, which the deep-bite rule derives from the overbite
    /// between the two. The manual ramps, the other arch's own data and the
    /// settings (staging, trim, occlusal, ramp sizes) are kept as they are.
    ///
    /// Stages: as `merge_arch` of the staging command does with an empty
    /// series, the arch's poses and IPR leave every stage and a stage left
    /// with neither is dropped. The other arch is not touched: it keeps the
    /// poses it has in every stage, so a case staged with `merge_arch` keeps
    /// its aligner count (the arch that finished first repeats its last
    /// pose), while stages that only the cleared arch filled disappear. The
    /// stage `index` values are not renumbered, as in `merge_arch`. Control
    /// visits are not renumbered either; the remaining ones keep their
    /// `number`.
    pub fn clear_arch(&mut self, role: ScanRole) -> Vec<&'static str> {
        if role == ScanRole::Bite {
            return Vec::new();
        }
        let mut cleared = Vec::new();
        if std::mem::take(&mut self.bite_registered) {
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

    /// Drops what is computed from the labels of the arch `role`, keeping
    /// the labels themselves, and returns the ids of the groups that held
    /// data, in the order of [`CLEARED_IDS`] (never `labels`). A bite role
    /// clears nothing.
    ///
    /// Call it when the labels of an arch are made again from the seeds (the
    /// automatic separation): the teeth flagged for a rebuild, the tooth
    /// frames, measures, closed crowns and gingiva of the arch, its space
    /// analysis, the setup, its poses in every stage and its ramps describe
    /// the old teeth, and the results that need both arches (Bolton,
    /// complexity score, automatic ramps) go with them. The doubts of the
    /// check of the separated teeth on the arch go too, without an id (they
    /// were about the old labels).
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
        self.segmentation_doubts.remove(&role);

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
        note("diagnosis", space || details);
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
            | setup.lines.remove(&role).is_some();
        note("setup", held);

        let mut held = false;
        for s in &mut self.stages {
            let n = s.ipr.len();
            held |= drop_teeth(&mut s.poses, role);
            s.ipr
                .retain(|r| !r.between.iter().any(|f| in_arch(*f, role)));
            held |= n != s.ipr.len();
        }
        self.stages
            .retain(|s| !s.poses.is_empty() || !s.ipr.is_empty());
        note("stages", held);

        let ramps = &mut self.aligner_bite.ramps.ramps;
        let n = ramps.len();
        ramps.retain(|r| !in_arch(r.fdi, role) && r.kind != RampKind::Auto);
        note("ramps", ramps.len() != n);

        cleared
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
    /// The label, closed crown, gingiva and control-visit scan blobs are the
    /// blob references in the state (the other fields are plain numbers and
    /// poses).
    fn referenced_blobs(&self) -> Vec<BlobRef> {
        self.labels
            .values()
            .chain(self.crowns.values())
            .chain(self.gingiva.values())
            .chain(self.visits.iter().filter_map(|v| v.scan.as_ref()))
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
        st.crowns.insert(Fdi::new(11).unwrap(), crown);
        st.gingiva.insert(ScanRole::Upper, gum);
        case.set_module("aligners", ModuleState::new(st.clone()));
        let used = case.referenced_blobs();
        assert!(used.contains(&crown) && used.contains(&gum) && !used.contains(&garbage));
        let keep = used.iter().copied().collect();
        case.blobs.retain_only(&keep);
        assert!(case.blobs.contains(&crown) && case.blobs.contains(&gum));
        assert!(!case.blobs.contains(&garbage));
        // and the JSON keys are the tooth numbers and the arches
        let json = serde_json::to_value(&st).unwrap();
        assert!(json["crowns"]["11"].is_string() && json["gingiva"]["upper"].is_string());
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
}
