//! The module's state stored in the case.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{BlobRef, Fdi, Pose, ScanRole};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{SCHEMA_VERSION, Settings, TrimParams};

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
}

/// Final position of the teeth (the setup).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// Final pose of each tooth, relative to its rest pose.
    pub poses: BTreeMap<Fdi, Pose>,
    /// Teeth the clinician locked: automatic recomputation leaves them alone.
    pub locked: BTreeSet<Fdi>,
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
    /// Per-vertex tooth labels of each scan (a mesh blob whose vertex
    /// attribute holds the label), keyed by arch.
    pub labels: BTreeMap<ScanRole, BlobRef>,
    /// Rest frame of each reconstructed tooth.
    pub tooth_frames: BTreeMap<Fdi, ToothFrame>,
    /// Diagnosis results.
    pub diagnosis: Diagnosis,
    /// Final position of the teeth.
    pub setup: Setup,
    /// The aligners, in order.
    pub stages: Vec<Stage>,
    /// Trim line parameters.
    pub trim: TrimParams,
    /// Settings.
    pub settings: Settings,
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
    /// The label blobs are the only blob references in the state (the other
    /// fields are plain numbers and poses).
    fn referenced_blobs(&self) -> Vec<BlobRef> {
        self.labels.values().copied().collect()
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
