//! Control visits: what is stored of the comparison of a progress scan with
//! the plan (`dca_algo::visits`).

use std::collections::BTreeMap;

use dc_case::{BlobRef, Fdi, Pose, ScanRole};
use serde::{Deserialize, Serialize};

/// Clinical components of the difference between the achieved and the planned
/// position of a tooth: linear at the crown centre, angular about the axes of
/// the tooth. Mesial, buccal and extrusion are positive in the direction they
/// name; torque is positive crown-buccal, tip positive crown-mesial and
/// rotation positive mesio-buccal.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToothDeviation {
    /// Mesial (+) or distal (-) shift (mm).
    pub mesial_mm: f64,
    /// Buccal (+) or lingual (-) shift (mm).
    pub buccal_mm: f64,
    /// Extrusion (+) or intrusion (-) (mm).
    pub extrusion_mm: f64,
    /// Crown torque (degrees).
    pub torque_deg: f64,
    /// Crown tip (degrees).
    pub tip_deg: f64,
    /// Rotation about the long axis (degrees).
    pub rotation_deg: f64,
}

impl ToothDeviation {
    /// Largest linear component, in absolute value (mm).
    pub fn linear_mm(&self) -> f64 {
        self.mesial_mm
            .abs()
            .max(self.buccal_mm.abs())
            .max(self.extrusion_mm.abs())
    }

    /// Largest angular component, in absolute value (degrees).
    pub fn angular_deg(&self) -> f64 {
        self.torque_deg
            .abs()
            .max(self.tip_deg.abs())
            .max(self.rotation_deg.abs())
    }
}

/// Where one tooth is at a visit, against the plan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToothTracking {
    /// Every component of the deviation is within the thresholds.
    pub on_track: bool,
    /// Largest linear component of the deviation (mm).
    pub linear_mm: f64,
    /// Largest angular component of the deviation (degrees).
    pub angular_deg: f64,
    /// Achieved / planned movement (per cent, as in the accuracy studies);
    /// `None` when the planned movement is below 0.2 mm.
    pub efficacy_pct: Option<f64>,
    /// Mean displacement of the crown points planned from the rest position
    /// (mm).
    pub planned_mm: f64,
    /// RMS of the last iteration of the tooth's registration (mm).
    pub rms_mm: f64,
    /// Scan vertices around the expected crown that the registration used.
    pub scan_points: u32,
    /// The deviation by component.
    pub deviation: ToothDeviation,
    /// Achieved position, relative to the rest pose (the convention of
    /// `Stage::poses`).
    pub achieved: Pose,
}

/// What the visit suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Every tooth found is on track: carry on with the plan.
    #[default]
    OnTrack,
    /// Some teeth are behind: a refinement (new plan from the real position,
    /// the lagging teeth moved more slowly) is suggested.
    Refinement,
}

/// One control visit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    /// Position in the treatment (1 is the first control visit).
    pub number: u32,
    /// Date of the visit, `YYYY-MM-DD`.
    pub date: String,
    /// The arch of the progress scan.
    pub arch: ScanRole,
    /// The aligner the patient wore (the stage of the plan the scan is
    /// compared with; 0 is the initial situation).
    pub stage: u32,
    /// Name of the scan file.
    pub source_name: String,
    /// RMS residual of the registration of the whole scan (mm).
    pub registration_rms_mm: f64,
    /// Linear and angular thresholds of the tracking (mm, degrees).
    pub thresholds: [f64; 2],
    /// Tracking of each tooth that was found on the scan.
    pub teeth: BTreeMap<Fdi, ToothTracking>,
    /// Teeth of the plan that were not found on the scan.
    pub untracked: Vec<Fdi>,
    /// Number of teeth on track.
    pub on_track: u32,
    /// What the visit suggests.
    pub decision: Decision,
    /// Velocity factor of the teeth that are behind, for the new aligners
    /// (the evidence `lag_speed`).
    pub speed: BTreeMap<Fdi, f64>,
    /// The progress scan moved onto the plan (a mesh blob in case
    /// coordinates). When `segmented`, the blob's vertex labels hold the
    /// tooth of each vertex (`0` = gingiva), as the labels blobs of
    /// `AlignersState::labels`.
    pub scan: Option<BlobRef>,
    /// The scan was segmented (its blob carries the tooth labels).
    pub segmented: bool,
}

impl Visit {
    /// The teeth that are behind the plan, in order.
    pub fn lagging(&self) -> Vec<Fdi> {
        self.teeth
            .iter()
            .filter(|(_, t)| !t.on_track)
            .map(|(&f, _)| f)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AlignersState;
    use dc_case::{CaseState, ModuleState};
    use dc_mesh::TriMesh;
    use glam::{DQuat, DVec3};

    fn tracking(on_track: bool) -> ToothTracking {
        ToothTracking {
            on_track,
            linear_mm: 0.31,
            angular_deg: 1.2,
            efficacy_pct: Some(82.0),
            planned_mm: 0.75,
            rms_mm: 0.04,
            scan_points: 1200,
            deviation: ToothDeviation {
                mesial_mm: 0.1,
                buccal_mm: -0.31,
                extrusion_mm: 0.02,
                torque_deg: 0.5,
                tip_deg: -1.2,
                rotation_deg: 0.3,
            },
            achieved: Pose::new(DVec3::new(0.4, 0.1, 0.0), DQuat::from_rotation_z(0.05)),
        }
    }

    fn visit(scan: Option<BlobRef>) -> Visit {
        let f = |n| Fdi::new(n).unwrap();
        Visit {
            number: 1,
            date: "2026-03-02".to_owned(),
            arch: ScanRole::Lower,
            stage: 3,
            source_name: "visit1.stl".to_owned(),
            registration_rms_mm: 0.05,
            thresholds: [0.5, 2.0],
            teeth: BTreeMap::from([(f(31), tracking(true)), (f(32), tracking(false))]),
            untracked: vec![f(33)],
            on_track: 1,
            decision: Decision::Refinement,
            speed: BTreeMap::from([(f(32), 0.75)]),
            scan,
            segmented: true,
        }
    }

    #[test]
    fn the_deviation_is_the_largest_component() {
        let d = tracking(true).deviation;
        assert_eq!(d.linear_mm(), 0.31);
        assert_eq!(d.angular_deg(), 1.2);
        assert_eq!(ToothDeviation::default().linear_mm(), 0.0);
    }

    #[test]
    fn a_visit_round_trips_through_json_and_lists_the_lagging_teeth() {
        let v = visit(Some(BlobRef::of_bytes(b"scan")));
        assert_eq!(v.lagging(), vec![Fdi::new(32).unwrap()]);
        let st = AlignersState {
            visits: vec![v],
            ..AlignersState::default()
        };
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["visits"][0]["decision"], "refinement");
        assert!(json["visits"][0]["teeth"]["32"]["on_track"] == false);
        assert!(json["visits"][0]["speed"]["32"].is_number());
        let back: AlignersState = serde_json::from_value(json).unwrap();
        assert_eq!(back, st);
        // a state written before the visits existed still reads
        let old: AlignersState = serde_json::from_str("{}").unwrap();
        assert!(old.visits.is_empty());
    }

    #[test]
    fn the_scan_of_a_visit_survives_pruning() {
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
        let scan = case.blobs.insert(tri(1.0));
        let garbage = case.blobs.insert(tri(2.0));
        let st = AlignersState {
            visits: vec![visit(Some(scan))],
            ..AlignersState::default()
        };
        case.set_module("aligners", ModuleState::new(st));
        let used = case.referenced_blobs();
        assert!(used.contains(&scan) && !used.contains(&garbage));
        let keep = used.iter().copied().collect();
        case.blobs.retain_only(&keep);
        assert!(case.blobs.contains(&scan) && !case.blobs.contains(&garbage));
    }
}
