//! What replacing a scan clears from the Aligners state
//! (`ModuleData::scan_replaced`), and what it keeps.

use std::collections::BTreeMap;

use dc_case::import::{Placement, PreparedScan, commit};
use dc_case::{
    BlobRef, Case, CaseState, Fdi, ModuleData, ModuleState, Pose, RoleSource, Scan, ScanRole,
};
use dc_mesh::{BadFaceReport, MeshFormat, TriMesh, Units};
use dca_model::{
    AlignersState, ArchDetails, ArchSpace, CLEARED_IDS, Decision, IprStep, MODULE_ID, Ramp,
    RampKind, Seed, Stage, ToothFrame, ToothMeasures, Visit,
};
use glam::{DQuat, DVec3, Vec3};
use serde_json::Value;

use ScanRole::{Bite, Lower, Upper};

fn fdi(n: u8) -> Fdi {
    Fdi::new(n).unwrap()
}

/// Three teeth of each arch: the numbers of the arch, and one more used for
/// the contact between the first two.
fn teeth(role: ScanRole) -> [Fdi; 3] {
    match role {
        Upper => [fdi(11), fdi(12), fdi(13)],
        _ => [fdi(31), fdi(32), fdi(33)],
    }
}

fn other(role: ScanRole) -> ScanRole {
    if role == Upper { Lower } else { Upper }
}

/// A one-triangle mesh, different for every `tag`.
fn tri(tag: f32) -> TriMesh {
    TriMesh::new(
        vec![Vec3::ZERO, Vec3::X, Vec3::new(0.0, 1.0, tag)],
        vec![[0, 1, 2]],
    )
}

fn pose(x: f64) -> Pose {
    Pose::new(DVec3::new(x, 0.0, 0.0), DQuat::from_rotation_z(0.1 * x))
}

fn measures(x: f64) -> ToothMeasures {
    ToothMeasures {
        centroid: DVec3::new(x, 1.0, 2.0),
        reference: DVec3::new(x, 0.0, 0.0),
        width_md: 8.0,
        width_bl: 7.0,
        height: 9.0,
        tip: DVec3::Z,
        cej: DVec3::X,
        crest: DVec3::Y,
        apex: DVec3::NEG_Z,
        contact_m: DVec3::ONE,
        contact_d: DVec3::NEG_ONE,
        rot_dev_deg: 1.0,
        axis_tilt_deg: 2.0,
        md_sign: 1,
    }
}

fn visit(number: u32, arch: ScanRole, scan: BlobRef) -> Visit {
    Visit {
        number,
        date: "2026-03-02".to_owned(),
        arch,
        stage: 2,
        source_name: format!("visit{number}.stl"),
        registration_rms_mm: 0.05,
        thresholds: [0.5, 2.0],
        teeth: BTreeMap::new(),
        untracked: Vec::new(),
        on_track: 0,
        decision: Decision::OnTrack,
        speed: BTreeMap::new(),
        scan: Some(scan),
        segmented: true,
    }
}

/// Everything one arch holds. Every blob is new in `case`.
fn fill_arch(case: &mut CaseState, st: &mut AlignersState, role: ScanRole, tag: f32) {
    let [a, b, c] = teeth(role);
    let mut blob = |n: f32| case.blobs.insert(tri(tag + n));

    st.seeds.push(Seed {
        fdi: a,
        position: DVec3::new(tag as f64, 0.0, 1.0),
    });
    st.missing.insert(c);
    st.dirty.insert(b);
    st.labels.insert(role, blob(0.1));
    for (i, f) in [a, b].into_iter().enumerate() {
        let x = f64::from(i as u8) + f64::from(tag);
        st.tooth_frames.insert(
            f,
            ToothFrame {
                pose: pose(x),
                center_of_resistance: DVec3::new(0.0, 0.0, 6.0),
            },
        );
        st.tooth_geometry.insert(f, measures(x));
        st.crowns.insert(f, blob(0.2 + i as f32 * 0.01));
        st.setup.poses.insert(f, pose(0.5 + x));
    }
    st.gingiva.insert(role, blob(0.3));
    st.diagnosis.arches.insert(
        role,
        ArchSpace {
            available_mm: 70.0 + f64::from(tag),
            required_mm: 73.5,
            little_index_mm: 4.2,
        },
    );
    st.diagnosis.details.insert(
        role,
        ArchDetails {
            largest_space_mm: 1.0,
            midline_offset_mm: 0.5,
            catcat_score: if role == Upper { 12 } else { 9 },
        },
    );
    st.setup.locked.insert(b);
    st.setup.ipr.push(IprStep {
        between: [a, b],
        amount_mm: 0.3,
    });
    st.setup
        .lines
        .insert(role, vec![DVec3::ZERO, DVec3::new(tag as f64, 1.0, 0.0)]);
    // three aligners; the ipr of this arch is at the second
    for k in 1..=3u32 {
        if st.stages.len() < k as usize {
            st.stages.push(Stage::new(k));
        }
        let s = &mut st.stages[k as usize - 1];
        for f in [a, b] {
            s.poses
                .insert(f, pose(f64::from(k) * 0.25 + f64::from(tag)));
        }
        if k == 2 {
            s.ipr.push(IprStep {
                between: [a, b],
                amount_mm: 0.1,
            });
        }
    }
    st.aligner_bite.ramps.ramps.push(Ramp {
        fdi: a,
        pose: pose(1.0),
        size_mm: [3.0, 2.0, 1.5],
        kind: RampKind::Manual,
    });
    let scan = blob(0.4);
    st.visits
        .push(visit(if role == Upper { 1 } else { 2 }, role, scan));
}

/// Results that need both arches.
fn fill_shared(st: &mut AlignersState) {
    st.diagnosis.bolton_anterior = Some(77.5);
    st.diagnosis.bolton_overall = Some(91.0);
    st.diagnosis.complexity_score = Some(12.0);
    st.bite_registered = true;
}

/// A case with both scans and the state of the module filled for the arches
/// in `roles` (and the shared results when both are there).
fn case_with(roles: &[ScanRole]) -> (CaseState, AlignersState) {
    let mut case = CaseState::new(Case::new("T"));
    let mut st = AlignersState::default();
    for &role in roles {
        let tag = if role == Upper { 10.0 } else { 20.0 };
        fill_arch(&mut case, &mut st, role, tag);
    }
    if roles.len() == 2 {
        fill_shared(&mut st);
    }
    (case, st)
}

fn json(st: &AlignersState) -> Value {
    st.to_json().unwrap()
}

#[test]
fn every_group_of_the_arch_is_cleared_and_reported_in_order() {
    for role in [Upper, Lower] {
        let (_, mut st) = case_with(&[Upper, Lower]);
        let ids = st.scan_replaced(role);
        assert_eq!(ids, CLEARED_IDS, "{role:?}");

        let arch = |f: &Fdi| (f.is_upper()) == (role == Upper);
        assert!(!st.seeds.iter().any(|s| arch(&s.fdi)));
        assert!(!st.missing.iter().any(arch));
        assert!(!st.dirty.iter().any(arch));
        assert!(!st.labels.contains_key(&role));
        assert!(!st.tooth_frames.keys().any(arch));
        assert!(!st.tooth_geometry.keys().any(arch));
        assert!(!st.crowns.keys().any(arch));
        assert!(!st.gingiva.contains_key(&role));
        assert!(!st.diagnosis.arches.contains_key(&role));
        assert!(!st.diagnosis.details.contains_key(&role));
        assert!(st.diagnosis.bolton_anterior.is_none());
        assert!(st.diagnosis.bolton_overall.is_none());
        assert!(st.diagnosis.complexity_score.is_none());
        assert!(!st.setup.poses.keys().any(arch));
        assert!(!st.setup.locked.iter().any(arch));
        assert!(!st.setup.ipr.iter().any(|r| arch(&r.between[0])));
        assert!(!st.setup.lines.contains_key(&role));
        assert!(st.stages.iter().all(|s| !s.poses.keys().any(arch)));
        assert!(
            st.stages
                .iter()
                .all(|s| !s.ipr.iter().any(|r| arch(&r.between[0])))
        );
        assert!(!st.aligner_bite.ramps.ramps.iter().any(|r| arch(&r.fdi)));
        assert!(!st.visits.iter().any(|v| v.arch == role));
        // the blobs of the cleared records are no longer referenced; the
        // other arch keeps exactly its own (blobs are content addressed)
        let (_, rest) = case_with(&[other(role)]);
        let mut live = st.referenced_blobs();
        let mut want = rest.referenced_blobs();
        live.sort();
        want.sort();
        assert_eq!(live, want, "{role:?}");
        assert_eq!(live.len(), 5);
    }
}

/// What the module holds when only group `id` is filled for `role`.
fn only_group(id: &str, role: ScanRole) -> AlignersState {
    let (_, full) = case_with(&[role]);
    let mut st = AlignersState::default();
    match id {
        "bite_registered" => st.bite_registered = true,
        "seeds" => st.seeds = full.seeds,
        "missing" => st.missing = full.missing,
        "labels" => st.labels = full.labels,
        "tooth_frames" => st.tooth_frames = full.tooth_frames,
        "tooth_geometry" => st.tooth_geometry = full.tooth_geometry,
        "crowns" => st.crowns = full.crowns,
        "gingiva" => st.gingiva = full.gingiva,
        "diagnosis" => {
            st.diagnosis.arches = full.diagnosis.arches;
            st.diagnosis.details = full.diagnosis.details;
        }
        "bolton" => {
            st.diagnosis.bolton_anterior = Some(77.5);
            st.diagnosis.bolton_overall = Some(91.0);
        }
        "complexity" => st.diagnosis.complexity_score = Some(12.0),
        "setup" => st.setup = full.setup,
        "stages" => st.stages = full.stages,
        "ramps" => st.aligner_bite.ramps.ramps = full.aligner_bite.ramps.ramps,
        "visits" => st.visits = full.visits,
        other => panic!("unknown id {other}"),
    }
    st
}

#[test]
fn only_the_groups_that_held_data_are_returned() {
    for role in [Upper, Lower] {
        for id in CLEARED_IDS {
            let mut st = only_group(id, role);
            assert_ne!(st, AlignersState::default(), "{id}");
            // the other arch has nothing in this group: nothing happens,
            // except for the results that need both arches
            let mut untouched = st.clone();
            let ids = untouched.scan_replaced(other(role));
            if matches!(id, "bolton" | "complexity" | "bite_registered") {
                assert_eq!(ids, [id], "{id}");
            } else {
                assert!(ids.is_empty(), "{id} {role:?}: {ids:?}");
                assert_eq!(untouched, st, "{id}");
            }
            // the arch itself: exactly that id, and nothing is left
            assert_eq!(st.scan_replaced(role), [id], "{id} {role:?}");
            assert_eq!(st, AlignersState::default(), "{id} {role:?}");
        }
    }
}

#[test]
fn the_other_arch_is_left_byte_for_byte() {
    for role in [Upper, Lower] {
        let (_, mut st) = case_with(&[Upper, Lower]);
        st.scan_replaced(role);
        // what is left is what an arch alone holds, minus the shared results
        let (_, expected) = case_with(&[other(role)]);
        assert_eq!(json(&st), json(&expected), "{role:?}");
        assert_eq!(
            serde_json::to_string(&json(&st)).unwrap(),
            serde_json::to_string(&json(&expected)).unwrap()
        );
        // and so the aligner count of the stages
        assert_eq!(st.stages.len(), 3);
    }
}

#[test]
fn an_empty_state_gives_nothing() {
    for role in [Upper, Lower, Bite] {
        let mut st = AlignersState::default();
        assert!(st.scan_replaced(role).is_empty());
        assert_eq!(st, AlignersState::default());
    }
}

#[test]
fn a_bite_scan_clears_nothing() {
    let (_, mut st) = case_with(&[Upper, Lower]);
    let before = st.clone();
    // the arches keep the registered position: the registration stays valid
    assert!(before.bite_registered);
    assert!(st.scan_replaced(Bite).is_empty());
    assert_eq!(st, before);
}

#[test]
fn settings_are_not_results_and_stay() {
    let (_, mut st) = case_with(&[Upper, Lower]);
    st.settings.interval_days = 9;
    st.trim.margin_gap_mm = 0.7;
    st.aligner_bite.ramps.enabled = true;
    st.aligner_bite.ramps.width_mm = 4.0;
    st.aligner_bite.occlusal.thickness_mm = 0.9;
    let (settings, trim, bite) = (
        st.settings.clone(),
        st.trim.clone(),
        (
            st.aligner_bite.ramps.enabled,
            st.aligner_bite.ramps.width_mm,
            st.aligner_bite.occlusal,
        ),
    );
    st.scan_replaced(Upper);
    assert_eq!(st.settings, settings);
    assert_eq!(st.trim, trim);
    assert_eq!(
        (
            st.aligner_bite.ramps.enabled,
            st.aligner_bite.ramps.width_mm,
            st.aligner_bite.occlusal
        ),
        bite
    );
}

#[test]
fn bolton_goes_when_either_arch_is_replaced() {
    for role in [Upper, Lower] {
        let mut st = AlignersState::default();
        st.diagnosis.bolton_anterior = Some(77.5);
        st.diagnosis.bolton_overall = Some(91.0);
        assert_eq!(st.scan_replaced(role), ["bolton"]);
        assert!(st.diagnosis.bolton_anterior.is_none() && st.diagnosis.bolton_overall.is_none());
    }
    // one ratio alone counts as well
    let mut st = AlignersState::default();
    st.diagnosis.bolton_overall = Some(91.0);
    assert_eq!(st.scan_replaced(Lower), ["bolton"]);
}

#[test]
fn the_complexity_score_survives_only_when_the_other_arch_explains_it() {
    let details = |score| ArchDetails {
        catcat_score: score,
        ..ArchDetails::default()
    };
    // the replaced arch contributed (or nothing says who did): gone
    let mut st = AlignersState::default();
    st.diagnosis.complexity_score = Some(12.0);
    st.diagnosis.details.insert(Upper, details(12));
    st.diagnosis.details.insert(Lower, details(9));
    assert_eq!(st.scan_replaced(Upper), ["diagnosis", "complexity"]);
    assert_eq!(st.diagnosis.details.len(), 1);
    let mut st = AlignersState::default();
    st.diagnosis.complexity_score = Some(12.0);
    assert_eq!(st.scan_replaced(Lower), ["complexity"]);
    // only the other arch has details: the score is its own and stays
    let mut st = AlignersState::default();
    st.diagnosis.complexity_score = Some(12.0);
    st.diagnosis.details.insert(Lower, details(12));
    assert!(st.scan_replaced(Upper).is_empty());
    assert_eq!(st.diagnosis.complexity_score, Some(12.0));
}

#[test]
fn automatic_ramps_of_the_other_arch_go_but_manual_ones_stay() {
    let ramp = |n, kind| Ramp {
        fdi: fdi(n),
        pose: Pose::IDENTITY,
        size_mm: [3.0, 2.0, 1.5],
        kind,
    };
    let mut st = AlignersState::default();
    st.aligner_bite.ramps.ramps = vec![
        ramp(11, RampKind::Auto),
        ramp(12, RampKind::Manual),
        ramp(31, RampKind::Manual),
    ];
    // the lower scan changes: the upper ramps proposed from the overbite go
    assert_eq!(st.scan_replaced(Lower), ["ramps"]);
    let left: Vec<u8> = st
        .aligner_bite
        .ramps
        .ramps
        .iter()
        .map(|r| r.fdi.get())
        .collect();
    assert_eq!(left, [12]);
    // the upper scan changes: its ramps go, of any kind
    assert_eq!(st.scan_replaced(Upper), ["ramps"]);
    assert!(st.aligner_bite.ramps.ramps.is_empty());
}

#[test]
fn stages_follow_merge_arch_with_an_empty_series() {
    let ipr = |a, b| IprStep {
        between: [fdi(a), fdi(b)],
        amount_mm: 0.2,
    };
    let stage = |k, poses: &[u8], iprs: Vec<IprStep>| {
        let mut s = Stage::new(k);
        for &n in poses {
            s.poses.insert(fdi(n), pose(f64::from(k)));
        }
        s.ipr = iprs;
        s
    };
    let mut st = AlignersState {
        stages: vec![
            stage(1, &[11, 31], vec![ipr(31, 32)]),
            stage(2, &[11, 31], vec![]),
            // only the upper arch is planned this far
            stage(3, &[11], vec![ipr(11, 12)]),
            stage(4, &[12], vec![]),
        ],
        ..AlignersState::default()
    };
    assert_eq!(st.scan_replaced(Upper), ["stages"]);
    // the stages the upper arch alone filled are dropped; the rest keep the
    // lower poses and IPR as they were
    let indexes: Vec<u32> = st.stages.iter().map(|s| s.index).collect();
    assert_eq!(indexes, [1, 2]);
    assert_eq!(
        st.stages[0].poses.keys().copied().collect::<Vec<_>>(),
        [fdi(31)]
    );
    assert_eq!(st.stages[0].ipr, vec![ipr(31, 32)]);
    // a ramp with no pose and an IPR of the lower arch alone still counts
    let mut st = AlignersState {
        stages: vec![stage(1, &[], vec![ipr(31, 32)])],
        ..AlignersState::default()
    };
    assert!(st.scan_replaced(Upper).is_empty());
    assert_eq!(st.stages.len(), 1);
    assert_eq!(st.scan_replaced(Lower), ["stages"]);
    assert!(st.stages.is_empty());
}

#[test]
fn visits_of_the_other_arch_keep_their_numbers() {
    let mut st = AlignersState {
        visits: vec![
            visit(1, Upper, BlobRef::of_bytes(b"u1")),
            visit(2, Lower, BlobRef::of_bytes(b"l2")),
            visit(3, Upper, BlobRef::of_bytes(b"u3")),
        ],
        ..AlignersState::default()
    };
    assert_eq!(st.scan_replaced(Upper), ["visits"]);
    assert_eq!(st.visits.len(), 1);
    assert_eq!(st.visits[0].number, 2);
    assert_eq!(st.referenced_blobs(), [BlobRef::of_bytes(b"l2")]);
}

/// A new scan of `role` that takes the place of `replaces`.
fn placement(mesh: TriMesh, role: ScanRole, replaces: Option<&Scan>) -> Placement {
    Placement {
        scan: PreparedScan {
            path: "scan.stl".into(),
            source_name: "scan.stl".to_owned(),
            mesh: mesh.into(),
            format: MeshFormat::Stl,
            source_units: Units::default(),
            cleanup: BadFaceReport::default(),
            warnings: Vec::new(),
        },
        role,
        source: RoleSource::Explicit,
        replaces: replaces.map(|s| vec![s.id]).unwrap_or_default(),
    }
}

#[test]
fn replacing_a_scan_through_the_import_clears_results_and_undo_restores_them() {
    let (mut state, st) = case_with(&[Upper, Lower]);
    let st_json = json(&st);
    state.set_module(MODULE_ID, ModuleState::new(st.clone()));
    commit(
        &mut state,
        &[
            placement(tri(1.0), Upper, None),
            placement(tri(2.0), Lower, None),
        ],
        &[],
    )
    .unwrap();
    // the upper and lower scans are in; everything the module holds is in too
    let held = st.referenced_blobs();
    assert!(held.iter().all(|b| state.blobs.contains(b)));
    let snapshot = state.clone(); // what the undo stack keeps

    let old_upper = state.case.scan(Upper).unwrap().clone();
    let report = commit(
        &mut state,
        &[placement(tri(3.0), Upper, Some(&old_upper))],
        &[],
    )
    .unwrap();
    assert_eq!(report.replaced_roles, [Upper]);
    let mut want: Vec<&str> = CLEARED_IDS.to_vec();
    want.sort_unstable();
    assert_eq!(report.module_cleared, want);

    // the Lower arch is exactly what it was
    let after: &AlignersState = state.module_data(MODULE_ID).unwrap();
    let (_, lower_only) = case_with(&[Lower]);
    assert_eq!(json(after), json(&lower_only));
    // the blobs of the cleared records and of the old scan are gone, those of
    // the Lower arch stay
    let (_, upper_only) = case_with(&[Upper]);
    for b in upper_only.referenced_blobs() {
        assert!(!state.blobs.contains(&b));
    }
    assert!(!state.blobs.contains(&old_upper.mesh));
    for b in lower_only.referenced_blobs() {
        assert!(state.blobs.contains(&b));
    }
    assert!(state.blobs.contains(&state.case.scan(Lower).unwrap().mesh));
    assert!(state.blobs.contains(&state.case.scan(Upper).unwrap().mesh));

    // undo: the snapshot has the results and the blobs back
    let undone = snapshot;
    let back: &AlignersState = undone.module_data(MODULE_ID).unwrap();
    assert_eq!(json(back), st_json);
    assert_eq!(back, &st);
    assert!(held.iter().all(|b| undone.blobs.contains(b)));
    assert_eq!(undone.case.scan(Upper).unwrap().id, old_upper.id);
}

#[test]
fn replacing_a_bite_scan_through_the_import_keeps_the_results() {
    let (mut state, st) = case_with(&[Upper, Lower]);
    state.set_module(MODULE_ID, ModuleState::new(st.clone()));
    commit(&mut state, &[placement(tri(1.0), Bite, None)], &[]).unwrap();
    let report = commit(&mut state, &[placement(tri(2.0), Bite, None)], &[]).unwrap();
    assert!(report.module_cleared.is_empty());
    assert_eq!(state.module_data::<AlignersState>(MODULE_ID), Some(&st));
}
