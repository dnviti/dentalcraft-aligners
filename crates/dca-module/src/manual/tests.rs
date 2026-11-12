//! Tests of the manual tools on the synthetic demo arch: every command is
//! driven by its JSON parameters, as an agent (or the viewport) drives it.

use std::sync::Arc;

use dc_case::{CaseState, Fdi, Pose, ScanRole};
use dc_command::{CommandError, JobCtx, UndoStack};
use dca_model::{AlignersState, MODULE_ID, Stage};
use glam::DVec3;
use serde_json::{Value, json};

const BRUSH: &str = "aligners.preparation.segmentation.manual-brush";
const CLEAN: &str = "aligners.preparation.segmentation.clean-borders";
const RENUMBER: &str = "aligners.preparation.numbering.renumber";
const MISSING: &str = "aligners.preparation.numbering.mark-missing";
const SEEDS: &str = "aligners.preparation.numbering.place-seeds";
const SEQUENCE: &str = "aligners.preparation.numbering.sequence";
const CLEAR: &str = "aligners.preparation.numbering.clear-seeds";
const REBUILD: &str = "aligners.manual.rebuild-tooth";
const ADJUST: &str = "aligners.planning.movements";

fn exec(id: &str, state: &CaseState, params: Value) -> Result<(CaseState, Value), CommandError> {
    crate::commands::find(id)
        .unwrap_or_else(|| panic!("{id} is not registered"))
        .execute(state, &params, &JobCtx::detached())
}

fn ok(id: &str, state: &CaseState, params: Value) -> (CaseState, Value) {
    exec(id, state, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn demo() -> CaseState {
    ok(
        "aligners.demo",
        &CaseState::default(),
        json!({ "arch": "lower", "severity": "mild" }),
    )
    .0
}

fn al(state: &CaseState) -> &AlignersState {
    state.module_data::<AlignersState>(MODULE_ID).unwrap()
}

fn fdi(n: u8) -> Fdi {
    Fdi::new(n).unwrap()
}

fn labels(state: &CaseState) -> Vec<u16> {
    let r = al(state).labels[&ScanRole::Lower];
    state.mesh(&r).unwrap().vertex_labels.clone().unwrap()
}

fn vertices(state: &CaseState) -> Vec<DVec3> {
    let scan = state.case.scan(ScanRole::Lower).unwrap();
    state
        .mesh(&scan.mesh)
        .unwrap()
        .positions
        .iter()
        .map(|p| p.as_dvec3())
        .collect()
}

fn seed(state: &CaseState, n: u8) -> DVec3 {
    al(state)
        .seeds
        .iter()
        .find(|s| s.fdi == fdi(n))
        .unwrap_or_else(|| panic!("no seed {n}"))
        .position
}

fn region(labels: &[u16], n: u8) -> Vec<u32> {
    (0..labels.len() as u32)
        .filter(|&i| labels[i as usize] == u16::from(n))
        .collect()
}

/// A vertex in the middle of the gingiva.
fn gum_vertex(state: &CaseState) -> usize {
    let gum = region(&labels(state), 0);
    gum[gum.len() / 2] as usize
}

fn invalid(r: Result<(CaseState, Value), CommandError>) -> String {
    match r {
        Err(CommandError::Invalid(m)) => m,
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("the command should have been refused"),
    }
}

/// Runs `id` with `params` on an undo stack and checks that undo gives back
/// the previous state exactly.
fn undoable(state: &CaseState, id: &str, params: Value) -> CaseState {
    let spec = crate::commands::find(id).unwrap();
    let mut stack = UndoStack::new(Arc::new(state.clone()), UndoStack::DEFAULT_BUDGET);
    let after = stack
        .apply(&spec.command(params), &JobCtx::detached())
        .unwrap()
        .clone();
    let back = stack.undo().unwrap();
    assert!(!Arc::ptr_eq(&back, &after));
    assert_eq!(
        back.module_data::<AlignersState>(MODULE_ID),
        state.module_data::<AlignersState>(MODULE_ID),
        "undo restores the Aligners data"
    );
    assert_eq!(back.blobs.len(), state.blobs.len());
    assert_ne!(
        after.module_data::<AlignersState>(MODULE_ID),
        state.module_data::<AlignersState>(MODULE_ID),
        "the command changed the data"
    );
    stack.redo().unwrap();
    (*after).clone()
}

#[test]
fn the_manual_commands_are_the_menu_leaves_and_have_schemas() {
    for id in [BRUSH, CLEAN, RENUMBER, MISSING, SEEDS, ADJUST] {
        assert!(crate::tree::find(id).is_some(), "{id} is a menu leaf");
        assert!(crate::commands::is_registered(id), "{id}");
    }
    let rebuild = crate::commands::find(REBUILD).unwrap();
    assert!(rebuild.schema()["properties"]["fdi"].is_object());
    let brush = crate::commands::find(BRUSH).unwrap().schema();
    for k in ["label", "mode", "vertices", "centre", "radius_mm", "stroke"] {
        assert!(brush["properties"][k].is_object(), "brush parameter {k}");
    }
    let adjust = crate::commands::find(ADJUST).unwrap().schema();
    for k in ["fdi", "stage", "delta", "mesial_mm", "rotation_deg", "lock"] {
        assert!(adjust["properties"][k].is_object(), "adjust parameter {k}");
    }
    // null parameters are refused with a reason, not a panic
    for id in [BRUSH, RENUMBER, MISSING, SEEDS, REBUILD, ADJUST] {
        assert!(matches!(
            exec(id, &demo(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
    }
}

// ---- the label brush -----------------------------------------------------------------------------------------

#[test]
fn painting_vertices_changes_exactly_those_labels_and_undo_restores_them() {
    let state = demo();
    let before = labels(&state);
    let tooth = region(&before, 36);
    let pick: Vec<u32> = tooth.iter().copied().take(5).collect();
    let after = undoable(&state, BRUSH, json!({ "label": 35, "vertices": pick }));
    let now = labels(&after);
    for (i, (a, b)) in before.iter().zip(&now).enumerate() {
        if pick.contains(&(i as u32)) {
            assert_eq!(*b, 35, "vertex {i}");
        } else {
            assert_eq!(a, b, "vertex {i} unchanged");
        }
    }
    // no tooth was built yet: nothing to rebuild; the old blob is gone
    assert!(al(&after).dirty.is_empty());
    assert_eq!(after.blobs.len(), state.blobs.len());
    // the labels blob is the scan with the new labels
    let (_, out) = ok(BRUSH, &state, json!({ "label": 35, "vertices": pick }));
    assert_eq!(out["changed_vertices"], 5);
    assert_eq!(out["changed_teeth"], json!([35, 36]));
}

#[test]
fn painting_by_hand_drops_the_result_of_the_separation_of_the_arch() {
    let mut state = demo();
    let mut d = al(&state).clone();
    for role in [ScanRole::Lower, ScanRole::Upper] {
        d.segmentation_runs.insert(
            role,
            dca_model::SegmentationRun {
                teeth: 14,
                seconds: 2.0,
            },
        );
    }
    state.set_module(MODULE_ID, dc_case::ModuleState::new(d));
    let pick: Vec<u32> = region(&labels(&state), 36).into_iter().take(5).collect();
    let (after, _) = ok(BRUSH, &state, json!({ "label": 35, "vertices": pick }));
    // the painted arch no longer has the colours the run gave; the other keeps its line
    assert!(!al(&after).segmentation_runs.contains_key(&ScanRole::Lower));
    assert!(al(&after).segmentation_runs.contains_key(&ScanRole::Upper));
}

#[test]
fn a_dab_paints_the_ball_and_erase_sends_a_tooth_back_to_the_gingiva() {
    let state = demo();
    let before = labels(&state);
    let v = vertices(&state);
    let c = seed(&state, 36);
    // erase tooth 36 under a 2 mm dab at its seed
    let after = undoable(
        &state,
        BRUSH,
        json!({ "mode": "erase", "label": 36, "centre": c.to_array(), "radius_mm": 2.0 }),
    );
    let now = labels(&after);
    let changed: Vec<usize> = (0..now.len()).filter(|&i| now[i] != before[i]).collect();
    assert!(!changed.is_empty());
    for &i in &changed {
        assert_eq!(before[i], 36, "only the tooth is erased");
        assert_eq!(now[i], 0);
        assert!(v[i].distance(c) <= 2.0 + 1e-3, "inside the dab");
    }
    assert!(region(&now, 36).len() > 10, "the rest of the tooth stays");
    // painting the gum label 0 erases whatever is under the dab
    let (g, _) = ok(
        BRUSH,
        &state,
        json!({ "label": 0, "centre": c.to_array(), "radius_mm": 1.5 }),
    );
    assert!(labels(&g).iter().zip(&before).any(|(a, b)| a != b));
    // a stroke covers the line between its points
    let a = seed(&state, 31);
    let b = seed(&state, 32);
    let (s, out) = ok(
        BRUSH,
        &state,
        json!({ "label": 33, "centre": a.to_array(), "stroke": [b.to_array()], "radius_mm": 1.0 }),
    );
    let mid = (a + b) / 2.0;
    let nearest_mid = (0..v.len())
        .min_by(|&i, &j| v[i].distance(mid).total_cmp(&v[j].distance(mid)))
        .unwrap();
    assert_eq!(labels(&s)[nearest_mid], 33, "{out}");
}

#[test]
fn the_brush_reports_bad_requests() {
    let state = demo();
    let c = seed(&state, 36).to_array();
    assert!(invalid(exec(BRUSH, &state, json!({ "label": 99, "centre": c }))).contains("99"));
    // tooth 16 is not in the lower arch
    assert!(
        invalid(exec(
            BRUSH,
            &state,
            json!({ "label": 16, "arch": "lower", "centre": c })
        ))
        .contains("lower")
    );
    assert!(invalid(exec(BRUSH, &state, json!({ "label": 36 }))).contains("nothing to paint"));
    assert!(
        invalid(exec(
            BRUSH,
            &state,
            json!({ "label": 36, "vertices": [4_000_000] })
        ))
        .contains("out of range")
    );
    assert!(
        invalid(exec(
            BRUSH,
            &state,
            json!({ "label": 36, "centre": c, "radius_mm": -1.0 })
        ))
        .contains("radius")
    );
    assert!(invalid(exec(BRUSH, &state, json!({ "mode": "fill", "label": 36 }))).contains("fill"));
    assert!(exec(BRUSH, &state, json!({ "label": 36, "bogus": 1 })).is_err());
}

#[test]
fn only_over_restricts_the_paint_to_a_label() {
    let state = demo();
    let before = labels(&state);
    let v = vertices(&state);
    // a dab on the gingiva, away from the teeth
    let c = v[gum_vertex(&state)].to_array();
    // only the vertices that are tooth 31 now: there is none under the dab
    let (same, out) = ok(
        BRUSH,
        &state,
        json!({ "label": 33, "centre": c, "radius_mm": 2.0, "only_over": 31 }),
    );
    assert_eq!(out["changed_vertices"], 0);
    assert_eq!(
        al(&same).labels,
        al(&state).labels,
        "nothing changed, same blob"
    );
    // only the gingiva: they all change
    let (s, out) = ok(
        BRUSH,
        &state,
        json!({ "label": 33, "centre": c, "radius_mm": 2.0, "only_over": 0 }),
    );
    let n = out["changed_vertices"].as_u64().unwrap();
    assert!(n > 3, "{out}");
    assert_eq!(
        labels(&s).iter().filter(|&&l| l == 33).count() as u64,
        n + region(&before, 33).len() as u64
    );
}

#[test]
fn the_fill_paints_a_crown_from_a_click_and_never_takes_another_tooth() {
    // the demo teeth are separate pieces: the fill takes part of the piece it starts on
    // (the algorithm itself is checked on a crown with a gingiva around it in dca-algo)
    let state = demo();
    let truth = labels(&state);
    let tooth = region(&truth, 36);
    let (erased, _) = ok(
        BRUSH,
        &state,
        json!({ "label": 36, "mode": "erase", "vertices": tooth }),
    );
    assert!(region(&labels(&erased), 36).is_empty());
    let c = seed(&state, 36);
    let (filled, out) = ok(
        BRUSH,
        &erased,
        json!({ "mode": "fill", "label": 36, "centre": c.to_array() }),
    );
    let now = labels(&filled);
    let got = region(&now, 36);
    assert!(got.len() > 100, "{out}");
    assert!(
        got.iter().all(|&i| truth[i as usize] == 36),
        "inside the tooth"
    );
    assert_eq!(out["targeted_vertices"], got.len());
    // the same fill by vertex index
    let v = vertices(&state);
    let nearest = (0..v.len())
        .min_by(|&i, &j| v[i].distance(c).total_cmp(&v[j].distance(c)))
        .unwrap();
    let (again, _) = ok(
        BRUSH,
        &erased,
        json!({ "mode": "fill", "label": 36, "vertex": nearest }),
    );
    assert_eq!(labels(&again), now);
    // other teeth are not taken: fill 35 over the region of 36
    let (over, _) = ok(
        BRUSH,
        &state,
        json!({ "mode": "fill", "label": 35, "centre": c.to_array() }),
    );
    // the clicked tooth takes the new number; no other tooth changes
    let l = labels(&over);
    for (a, b) in truth.iter().zip(&l) {
        if a != b {
            assert_eq!(*a, 36);
            assert_eq!(*b, 35);
        }
    }
    assert!(l.contains(&35));
}

#[test]
fn filling_with_the_gingiva_label_reports_what_it_took_back() {
    // a click on a vertex that is gingiva finds nothing to take back
    let state = demo();
    let truth = labels(&state);
    let gum = truth.iter().position(|&l| l == 0).unwrap();
    let (same, out) = ok(
        BRUSH,
        &state,
        json!({ "mode": "fill", "label": 0, "vertex": gum }),
    );
    assert_eq!(out["changed_vertices"], 0);
    assert_eq!(labels(&same), truth);
}

#[test]
fn the_labels_of_a_scan_without_segmentation_start_as_gingiva() {
    let mut state = demo();
    let mut data = al(&state).clone();
    data.labels.clear();
    state.set_module(MODULE_ID, dc_case::ModuleState::new(data));
    let v = vertices(&state);
    let (s, _) = ok(BRUSH, &state, json!({ "label": 31, "vertices": [0, 1, 2] }));
    let l = labels(&s);
    assert_eq!(l.len(), v.len());
    assert_eq!(l.iter().filter(|&&x| x == 31).count(), 3);
    assert_eq!(l.iter().filter(|&&x| x == 0).count(), v.len() - 3);
}

#[test]
fn cleaning_removes_islands_and_the_hole_in_a_crown() {
    let state = demo();
    let truth = labels(&state);
    let v = vertices(&state);
    // an island of tooth 31 on the gingiva
    let g = gum_vertex(&state);
    let island: Vec<u32> = (0..v.len() as u32)
        .filter(|&i| truth[i as usize] == 0 && v[i as usize].distance(v[g]) < 2.5)
        .collect();
    assert!(island.len() > 3);
    let (dirty, _) = ok(BRUSH, &state, json!({ "label": 31, "vertices": island }));
    assert!(labels(&dirty) != truth);
    let cleaned = undoable(&dirty, CLEAN, json!({ "fdi": 31, "rings": 0 }));
    let now = labels(&cleaned);
    assert!(
        island.iter().all(|&i| now[i as usize] == 0),
        "island removed"
    );
    assert_eq!(now, truth);
    // a hole of gingiva inside tooth 36
    let hole: Vec<u32> = region(&truth, 36)
        .into_iter()
        .filter(|&i| v[i as usize].distance(seed(&state, 36)) < 0.8)
        .collect();
    assert!(!hole.is_empty());
    let (holed, _) = ok(BRUSH, &state, json!({ "label": 0, "vertices": hole }));
    let (healed, out) = ok(CLEAN, &holed, json!({ "rings": 0 }));
    assert!(out["lower"]["islands"].as_u64().unwrap() >= 1, "{out}");
    assert!(hole.iter().all(|&i| labels(&healed)[i as usize] == 36));
    // a clean scan stays as it is
    let (same, out) = ok(CLEAN, &state, json!({ "rings": 0 }));
    assert_eq!(out["lower"]["changed_vertices"], 0);
    assert_eq!(al(&same).labels, al(&state).labels);
    assert!(invalid(exec(CLEAN, &CaseState::default(), Value::Null)).contains("tooth colours"));
}

// ---- tooth numbers ------------------------------------------------------------------------------------------

/// Every seed lies on the region of its own number.
fn seeds_match_labels(state: &CaseState) {
    let v = vertices(state);
    let l = labels(state);
    for s in &al(state).seeds {
        let nearest = (0..v.len())
            .min_by(|&i, &j| {
                v[i].distance(s.position)
                    .total_cmp(&v[j].distance(s.position))
            })
            .unwrap();
        assert_eq!(l[nearest], u16::from(s.fdi.get()), "seed {}", s.fdi);
    }
}

#[test]
fn swapping_two_teeth_keeps_seeds_labels_and_frames_consistent() {
    let (state, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    seeds_match_labels(&state);
    let truth = labels(&state);
    let (a, b) = (seed(&state, 31), seed(&state, 41));
    let after = undoable(&state, RENUMBER, json!({ "src": 31, "dst": 41 }));
    let (_, out) = ok(RENUMBER, &state, json!({ "src": 31, "dst": 41 }));
    assert_eq!(out["swap"], true);
    let now = labels(&after);
    for (x, y) in truth.iter().zip(&now) {
        let want = match *x {
            31 => 41,
            41 => 31,
            o => o,
        };
        assert_eq!(*y, want);
    }
    // the seeds exchanged their numbers: seed 41 sits where seed 31 was
    assert_eq!(seed(&after, 41), a);
    assert_eq!(seed(&after, 31), b);
    seeds_match_labels(&after);
    // the frames were measured again: each tooth's frame is on its own region
    let v = vertices(&after);
    let st = al(&after);
    assert_eq!(st.tooth_frames.len(), al(&state).tooth_frames.len());
    for (f, fr) in &st.tooth_frames {
        let pts: Vec<DVec3> = region(&now, f.get())
            .iter()
            .map(|&i| v[i as usize])
            .collect();
        let centroid = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
        assert!(fr.pose.t.distance(centroid) < 8.0, "frame of {f}");
    }
    assert!(st.tooth_geometry.contains_key(&fdi(31)) && st.tooth_geometry.contains_key(&fdi(41)));
}

#[test]
fn renumbering_to_a_free_number_moves_the_tooth_and_missing_is_respected() {
    let state = demo();
    // 38 does not exist in the demo (no third molars)
    assert!(!labels(&state).contains(&38));
    let (s, out) = ok(RENUMBER, &state, json!({ "src": 37, "dst": 38 }));
    assert_eq!(out["swap"], false);
    let l = labels(&s);
    assert!(!l.contains(&37) && l.contains(&38));
    assert_eq!(seed(&s, 38), seed(&state, 37));
    assert!(al(&s).seeds.iter().all(|x| x.fdi != fdi(37)));
    seeds_match_labels(&s);
    // a free number that was marked missing is a tooth again
    let (m, _) = ok(MISSING, &state, json!({ "fdi": 38 }));
    assert!(al(&m).missing.contains(&fdi(38)));
    let (s, _) = ok(RENUMBER, &m, json!({ "src": 37, "dst": 38 }));
    assert!(al(&s).missing.is_empty());
    // errors
    assert!(invalid(exec(RENUMBER, &state, json!({ "src": 37, "dst": 37 }))).contains("same"));
    assert!(invalid(exec(RENUMBER, &state, json!({ "src": 37, "dst": 17 }))).contains("arches"));
    assert!(invalid(exec(RENUMBER, &state, json!({ "src": 38, "dst": 37 }))).contains("38"));
    assert!(invalid(exec(RENUMBER, &state, json!({ "src": 37, "dst": 99 }))).contains("99"));
}

#[test]
fn a_renumbered_tooth_is_rebuilt_with_its_new_number() {
    // measure and close the teeth, then swap two of them: their frames and solids follow the number
    let (s, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    let (s, _) = ok(
        "aligners.preparation.reconstruction.automatic",
        &s,
        Value::Null,
    );
    assert!(al(&s).crowns.contains_key(&fdi(36)));
    let old36 = al(&s).crowns[&fdi(36)];
    let old46 = al(&s).crowns[&fdi(46)];
    let (after, out) = ok(RENUMBER, &s, json!({ "src": 36, "dst": 46 }));
    assert_eq!(out["rebuilt"], json!([36, 46]));
    let st = al(&after);
    assert!(st.crowns.contains_key(&fdi(36)) && st.crowns.contains_key(&fdi(46)));
    // the solid of 46 is now where the old 36 was (a mirror of the arch): not the same blob
    assert_ne!(st.crowns[&fdi(46)], old46);
    assert_ne!(st.crowns[&fdi(36)], old36);
}

#[test]
fn a_missing_tooth_is_gone_and_stays_out_of_the_later_stages() {
    let (s, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    assert!(al(&s).tooth_frames.contains_key(&fdi(41)));
    let before = labels(&s);
    let n41 = region(&before, 41).len();
    assert!(n41 > 0);
    let after = undoable(&s, MISSING, json!({ "fdi": 41 }));
    let st = al(&after);
    assert!(st.missing.contains(&fdi(41)));
    assert!(st.seeds.iter().all(|x| x.fdi != fdi(41)));
    assert!(!st.tooth_frames.contains_key(&fdi(41)) && !st.tooth_geometry.contains_key(&fdi(41)));
    let now = labels(&after);
    assert!(!now.contains(&41));
    assert_eq!(
        now.iter().filter(|&&x| x == 0).count(),
        before.iter().filter(|&&x| x == 0).count() + n41
    );
    // its space stays empty: the other teeth are untouched
    for (a, b) in before.iter().zip(&now) {
        if *a != 41 {
            assert_eq!(a, b);
        }
    }
    // the later stages do not find it: segment again from the seeds, measure, set up
    let (seg, _) = ok(
        "aligners.preparation.segmentation.automatic",
        &after,
        Value::Null,
    );
    assert!(!labels(&seg).contains(&41));
    let (teeth, _) = ok("aligners.teeth.frames", &seg, Value::Null);
    assert!(!al(&teeth).tooth_frames.contains_key(&fdi(41)));
    assert!(al(&teeth).tooth_frames.contains_key(&fdi(31)));
    let (setup, _) = ok("aligners.planning.setup.automatic", &teeth, Value::Null);
    assert!(!al(&setup).setup.poses.contains_key(&fdi(41)));
    // a missing tooth cannot be painted, seeded or adjusted until it is present again
    assert!(
        invalid(exec(BRUSH, &after, json!({ "label": 41, "vertices": [0] }))).contains("missing")
    );
    assert!(
        invalid(exec(
            SEEDS,
            &after,
            json!({ "fdi": 41, "position": [0.0, 0.0, 0.0] })
        ))
        .contains("missing")
    );
    // the automatic numbering does not propose it again
    let (numbered, _) = ok(
        "aligners.preparation.numbering.automatic",
        &after,
        Value::Null,
    );
    assert!(al(&numbered).seeds.iter().all(|x| x.fdi != fdi(41)));
    assert!(al(&numbered).seeds.iter().any(|x| x.fdi == fdi(31)));
    // present again
    let (back, out) = ok(MISSING, &after, json!({ "fdi": 41, "missing": false }));
    assert_eq!(out["was_missing"], true);
    assert!(al(&back).missing.is_empty());
    ok(
        SEEDS,
        &back,
        json!({ "fdi": 41, "position": seed(&s, 41).to_array() }),
    );
}

#[test]
fn marking_missing_drops_the_movements_of_the_tooth() {
    let (s, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    let (mut s, _) = ok("aligners.planning.setup.automatic", &s, Value::Null);
    let mut data = al(&s).clone();
    data.setup
        .poses
        .insert(fdi(41), Pose::from_translation(DVec3::X));
    data.setup.locked.insert(fdi(41));
    let mut stage = Stage::new(1);
    stage
        .poses
        .insert(fdi(41), Pose::from_translation(DVec3::X));
    data.stages.push(stage);
    s.set_module(MODULE_ID, dc_case::ModuleState::new(data));
    let (m, out) = ok(MISSING, &s, json!({ "fdi": 41 }));
    assert_eq!(out["setup_outdated"], true);
    let st = al(&m);
    assert!(!st.setup.poses.contains_key(&fdi(41)) && !st.setup.locked.contains(&fdi(41)));
    assert!(st.stages.iter().all(|x| !x.poses.contains_key(&fdi(41))));
}

// ---- seeds ----------------------------------------------------------------------------------------------------

#[test]
fn seeds_are_refused_on_a_scan_whose_arch_is_a_guess() {
    let mut state = demo();
    state.case.scans[0].role_source = dc_case::RoleSource::Provisional;
    for op in [
        json!({ "op": "delete", "fdi": 36 }),
        json!({ "op": "add", "fdi": 36, "position": [0.0, 0.0, 0.0] }),
        json!({ "op": "move", "fdi": 36, "position": [0.0, 0.0, 0.0] }),
    ] {
        assert!(
            invalid(exec(SEEDS, &state, op)).contains("Import scans panel"),
            "a provisional scan took a seed"
        );
    }
    // the other sources of a role let it pass, the guess made by the geometry too
    for source in [
        dc_case::RoleSource::Explicit,
        dc_case::RoleSource::Detected,
        dc_case::RoleSource::DetectedUnsure,
    ] {
        state.case.scans[0].role_source = source;
        ok(SEEDS, &state, json!({ "op": "delete", "fdi": 36 }));
    }
}

#[test]
fn seeds_are_added_moved_and_deleted() {
    let state = demo();
    let n = al(&state).seeds.len();
    let p = seed(&state, 36);
    // delete
    let deleted = undoable(&state, SEEDS, json!({ "op": "delete", "fdi": 36 }));
    assert_eq!(al(&deleted).seeds.len(), n - 1);
    let (_, out) = ok(SEEDS, &state, json!({ "op": "delete", "fdi": 36 }));
    assert_eq!(out["segmentation_outdated"], true);
    // add it back somewhere else, snapped onto the scan
    let q = p + DVec3::new(0.3, 0.2, 5.0);
    let (added, out) = ok(
        SEEDS,
        &deleted,
        json!({ "op": "add", "fdi": 36, "position": q.to_array(), "snap_to_scan": true }),
    );
    assert_eq!(al(&added).seeds.len(), n);
    let snapped = seed(&added, 36);
    assert!(vertices(&added).iter().any(|v| v.distance(snapped) < 1e-6));
    assert_eq!(out["position"], json!(snapped.to_array()));
    // move
    let moved = undoable(
        &added,
        SEEDS,
        json!({ "op": "move", "fdi": 36, "position": p.to_array() }),
    );
    assert_eq!(seed(&moved, 36), p);
    assert_eq!(al(&moved).seeds.len(), n);
    // errors
    assert!(
        invalid(exec(
            SEEDS,
            &state,
            json!({ "op": "add", "fdi": 36, "position": [0.0, 0.0, 0.0] })
        ))
        .contains("already")
    );
    assert!(
        invalid(exec(
            SEEDS,
            &deleted,
            json!({ "op": "move", "fdi": 36, "position": [0.0, 0.0, 0.0] })
        ))
        .contains("no tooth mark")
    );
    assert!(
        invalid(exec(SEEDS, &deleted, json!({ "op": "delete", "fdi": 36 })))
            .contains("no tooth mark")
    );
    assert!(invalid(exec(SEEDS, &state, json!({ "op": "move", "fdi": 36 }))).contains("position"));
    assert!(
        exec(
            SEEDS,
            &state,
            json!({ "op": "move", "fdi": 36, "position": [null, 0.0, 0.0] })
        )
        .is_err()
    );
}

// ---- numbering in order, clearing the marks ---------------------------------------------------------

fn at(x: f64) -> Value {
    json!({ "kind": "point", "position": [x, 0.0, 0.0] })
}

#[test]
fn clearing_the_marks_keeps_the_missing_list_unless_asked_and_can_be_undone() {
    let (state, _) = ok(MISSING, &demo(), json!({ "fdi": 38 }));
    assert!(!al(&state).seeds.is_empty() && al(&state).missing.contains(&fdi(38)));
    // as in the plugin, the list of missing teeth stays by default
    let cleared = undoable(&state, CLEAR, json!({}));
    assert!(al(&cleared).seeds.is_empty() && al(&cleared).missing.contains(&fdi(38)));
    let (_, out) = ok(CLEAR, &state, json!({}));
    assert_eq!(out["missing_cleared"], json!([]));
    // keep_missing false clears it too
    let (all, out) = ok(CLEAR, &state, json!({ "keep_missing": false }));
    assert!(al(&all).seeds.is_empty() && al(&all).missing.is_empty());
    assert_eq!(out["missing_cleared"], json!([38]));
    // what was computed from the marks stays, out of date (as in the plugin)
    assert_eq!(
        al(&cleared).labels.get(&ScanRole::Lower),
        al(&state).labels.get(&ScanRole::Lower)
    );
    assert_eq!(al(&cleared).tooth_frames, al(&state).tooth_frames);
    let (_, out) = ok(CLEAR, &state, json!({}));
    assert_eq!(out["segmentation_outdated"], true);
    assert!(
        out["invalidated"]
            .as_array()
            .unwrap()
            .contains(&json!("labels"))
    );
    assert_eq!(out["seeds_removed"], al(&state).seeds.len());
    // only one arch: the lower marks are not touched by clearing the upper arch
    let (other, out) = ok(CLEAR, &state, json!({ "arch": "upper" }));
    assert_eq!(al(&other).seeds, al(&state).seeds);
    assert_eq!(out["seeds_removed"], 0);
    assert_eq!(out["segmentation_outdated"], false);
    assert!(al(&other).labels.contains_key(&ScanRole::Lower));
    // nothing to clear: nothing is out of date
    let (_, out) = ok(CLEAR, &cleared, json!({}));
    assert_eq!(out["invalidated"], json!([]));
    assert!(exec(CLEAR, &state, json!({ "arch": "sideways" })).is_err());
    assert!(exec(CLEAR, &state, json!({ "nope": 1 })).is_err());
}

#[test]
fn the_sequence_numbers_the_teeth_from_the_right_distal_to_the_left() {
    let (state, _) = ok(CLEAR, &demo(), json!({}));
    let steps = json!([at(1.0), at(2.0), { "kind": "skip" }, at(4.0)]);
    let done = undoable(&state, SEQUENCE, json!({ "steps": steps }));
    // 47, 46 get a point, 45 is skipped, 44 gets the next point (no third molars)
    let nums: Vec<u8> = al(&done).seeds.iter().map(|s| s.fdi.get()).collect();
    assert_eq!(nums, [47, 46, 44]);
    assert_eq!(seed(&done, 46), DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(seed(&done, 44), DVec3::new(4.0, 0.0, 0.0));
    // S skips the tooth only: it is not added to the missing teeth
    assert!(al(&done).missing.is_empty());
    let (_, out) = ok(SEQUENCE, &state, json!({ "steps": steps }));
    assert_eq!(out["arch"], "lower");
    assert_eq!(out["next_fdi"], 43);
    assert_eq!(out["seeds_in_arch"], 3);
    assert_eq!(out["assigned"][2], json!({ "fdi": 45, "kind": "skip" }));
    assert_eq!(out["remaining"].as_array().unwrap().len(), 10);
}

#[test]
fn the_sequence_continues_where_it_stopped_and_skips_the_taken_teeth() {
    let (state, _) = ok(CLEAR, &demo(), json!({}));
    let (first, _) = ok(SEQUENCE, &state, json!({ "steps": [at(1.0), at(2.0)] }));
    let (second, out) = ok(
        SEQUENCE,
        &first,
        json!({ "steps": [{ "kind": "skip" }, at(3.0)] }),
    );
    assert_eq!(out["assigned"][0]["fdi"], 45);
    assert_eq!(out["assigned"][1]["fdi"], 44);
    assert_eq!(al(&second).seeds.len(), 3);
    // a skipped tooth is asked for again in the next numbering (as in the plugin)
    let (_, out) = ok(SEQUENCE, &second, json!({ "steps": [at(5.0)] }));
    assert_eq!(out["assigned"][0]["fdi"], 45);
    // a tooth marked missing is not asked for
    let (marked, _) = ok(MISSING, &second, json!({ "fdi": 45 }));
    let (_, out) = ok(SEQUENCE, &marked, json!({ "steps": [at(5.0)] }));
    assert_eq!(out["assigned"][0]["fdi"], 43);
    // third molars on request: the sequence starts with 48 and ends with 38
    let (third, out) = ok(
        SEQUENCE,
        &state,
        json!({ "third_molars": true, "steps": [at(1.0)] }),
    );
    assert_eq!(seed(&third, 48), DVec3::new(1.0, 0.0, 0.0));
    assert_eq!(out["remaining"].as_array().unwrap().last().unwrap(), 38);
}

#[test]
fn the_sequence_refuses_what_does_not_fit_and_changes_nothing() {
    let (state, _) = ok(CLEAR, &demo(), json!({}));
    for params in [
        json!({}),
        json!({ "steps": [] }),
        json!({ "steps": [{ "kind": "teleport" }] }),
        json!({ "steps": [{ "kind": "point" }] }),
        json!({ "steps": [{ "kind": "point", "position": [null, 0.0, 0.0] }] }),
        json!({ "steps": [at(1.0)], "arch": "upper" }),
        json!({ "steps": vec![at(1.0); 15] }),
    ] {
        assert!(exec(SEQUENCE, &state, params.clone()).is_err(), "{params}");
    }
    // a scan whose arch is a guess is refused
    let mut guess = state.clone();
    guess.case.scans[0].role_source = dc_case::RoleSource::Provisional;
    assert!(
        invalid(exec(SEQUENCE, &guess, json!({ "steps": [at(1.0)] }))).contains("Import scans")
    );
    // the whole arch fits exactly
    let full = vec![at(1.0); 14];
    let (done, out) = ok(SEQUENCE, &state, json!({ "steps": full }));
    assert_eq!(al(&done).seeds.len(), 14);
    assert_eq!(out["next_fdi"], Value::Null);
}

#[test]
fn a_skip_step_leaves_the_tooth_alone_and_a_stop_keeps_the_marks_given() {
    // the arch has its colours but no marks (so nothing is dropped by a skip)
    let state = without_marks(&demo());
    assert!(labels(&state).contains(&47));
    let (done, out) = ok(SEQUENCE, &state, json!({ "steps": [{ "kind": "skip" }] }));
    assert_eq!(out["segmentation_outdated"], false);
    assert!(al(&done).missing.is_empty() && al(&done).seeds.is_empty());
    assert!(labels(&done).contains(&47));
    assert_eq!(out["next_fdi"], 46);
    // stopping early (Esc or Enter) keeps the marks given
    let (stopped, out) = ok(SEQUENCE, &state, json!({ "steps": [at(1.0), at(2.0)] }));
    assert_eq!(al(&stopped).seeds.len(), 2);
    assert_eq!(out["next_fdi"], 45);
}

/// The state with no tooth marks but everything else it had (the marks
/// removed by hand, not by a command).
fn without_marks(state: &CaseState) -> CaseState {
    let mut data = al(state).clone();
    data.seeds.clear();
    let mut out = state.clone();
    out.set_module(MODULE_ID, dc_case::ModuleState::new(data));
    out
}

#[test]
fn a_change_of_the_marks_keeps_what_was_computed_from_them_out_of_date() {
    let (state, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    let p = seed(&state, 36);
    // moving a mark by 0.1 mm keeps the colours and the teeth of the arch,
    // as the plugin does (`add_seed`, then the later steps are to redo)
    let (moved, out) = ok(
        SEEDS,
        &state,
        json!({ "op": "move", "fdi": 36, "position": (p + DVec3::new(0.1, 0.0, 0.0)).to_array() }),
    );
    assert_eq!(out["segmentation_outdated"], true);
    let outdated = out["invalidated"].as_array().unwrap();
    for id in ["labels", "tooth_frames"] {
        assert!(outdated.contains(&json!(id)), "{id} in {outdated:?}");
    }
    assert!(out.get("dropped").is_none());
    let mut expect = al(&state).clone();
    expect.seeds = al(&moved).seeds.clone();
    assert_eq!(al(&moved), &expect, "only the mark changed");
    assert_eq!(labels(&moved), labels(&state));
    assert_eq!(moved.blobs.len(), state.blobs.len());
    // the same position again is no change
    let (same, out) = ok(
        SEEDS,
        &state,
        json!({ "op": "move", "fdi": 36, "position": p.to_array() }),
    );
    assert_eq!(out["segmentation_outdated"], false);
    assert_eq!(out["invalidated"], json!([]));
    assert!(al(&same).labels.contains_key(&ScanRole::Lower));
    // a sequence that adds marks keeps them too
    let (done, out) = ok(
        SEQUENCE,
        &without_marks(&state),
        json!({ "steps": [at(1.0)] }),
    );
    assert_eq!(out["segmentation_outdated"], true);
    assert!(al(&done).labels.contains_key(&ScanRole::Lower));
    assert_eq!(al(&done).tooth_frames, al(&state).tooth_frames);
}

#[test]
fn a_step_for_another_tooth_refuses_the_numbering_in_words_of_the_app() {
    let (state, _) = ok(CLEAR, &demo(), json!({}));
    // the steps name the teeth they are for: 47 then 46
    let named = json!({ "steps": [
        { "kind": "point", "position": [1.0, 0.0, 0.0], "fdi": 47 },
        { "kind": "skip", "fdi": 46 },
    ] });
    let (done, out) = ok(SEQUENCE, &state, named.clone());
    assert_eq!(al(&done).seeds.len(), 1);
    assert_eq!(out["next_fdi"], 45);
    // once 47 has its mark, the same steps are for other teeth: refused
    let m = invalid(exec(SEQUENCE, &done, named));
    assert!(
        m.contains("47") && m.contains("46") && m.contains("start it again"),
        "{m}"
    );
    let wrong = json!({ "steps": [{ "kind": "skip", "fdi": 31 }] });
    assert!(invalid(exec(SEQUENCE, &state, wrong)).contains("next tooth to number is 47"));
    // the other refusals are sentences of the app too
    assert_eq!(
        invalid(exec(SEQUENCE, &state, json!({ "steps": [] }))),
        "give at least one step"
    );
    let m = invalid(exec(
        SEQUENCE,
        &state,
        json!({ "steps": vec![at(1.0); 15] }),
    ));
    assert!(m.contains("15 steps for 14 teeth"), "{m}");
    let m = invalid(exec(
        SEQUENCE,
        &state,
        json!({ "steps": [at(1.0)], "arch": "upper" }),
    ));
    assert!(m.contains("no") && m.contains("upper"), "{m}");
}

// ---- rebuild one tooth ------------------------------------------------------------------------------------

#[test]
fn rebuilding_a_tooth_checks_the_region_and_replaces_only_its_solid() {
    let (s, _) = ok(
        "aligners.preparation.reconstruction.automatic",
        &demo(),
        Value::Null,
    );
    let st = al(&s);
    assert!(st.crowns.contains_key(&fdi(36)));
    let truth = labels(&s);
    let (old36, old35) = (st.crowns[&fdi(36)], st.crowns[&fdi(35)]);
    // damage the region of tooth 36 with the brush: it is listed to rebuild
    let tooth = region(&truth, 36);
    let cut: Vec<u32> = tooth.iter().copied().step_by(9).take(40).collect();
    let (hurt, out) = ok(
        BRUSH,
        &s,
        json!({ "label": 36, "mode": "erase", "vertices": cut }),
    );
    assert_eq!(out["to_rebuild"], json!([36]));
    assert!(al(&hurt).dirty.contains(&fdi(36)));
    // rebuild: the region is found again from the seed, the solid replaced, the others kept
    let rebuilt = undoable(&hurt, REBUILD, json!({ "fdi": 36, "resegment": true }));
    let st = al(&rebuilt);
    assert!(st.dirty.is_empty());
    assert_ne!(st.crowns[&fdi(36)], old36);
    assert_eq!(st.crowns[&fdi(35)], old35);
    let now = labels(&rebuilt);
    let got = region(&now, 36);
    let inter = got.iter().filter(|&&i| truth[i as usize] == 36).count();
    assert_eq!(inter, got.len(), "the region is inside the tooth");
    assert!(
        got.len() > 300,
        "the crown fill found a region again: {}",
        got.len()
    );
    // a selection becomes the tooth, the rest of its vertices go to the gingiva
    let (sel, out) = ok(REBUILD, &s, json!({ "fdi": 36, "vertices": tooth }));
    assert_eq!(out["labels_changed"], false);
    assert!(out["message"].as_str().unwrap().contains("36"));
    assert!(al(&sel).crowns.contains_key(&fdi(36)));
    // a region that cannot make a crown is refused and nothing changes
    let tiny: Vec<u32> = tooth.iter().copied().take(6).collect();
    let msg = invalid(exec(REBUILD, &s, json!({ "fdi": 36, "vertices": tiny })));
    assert!(msg.contains("too small"), "{msg}");
    let (erased, _) = ok(
        BRUSH,
        &s,
        json!({ "label": 36, "mode": "erase", "vertices": tooth }),
    );
    assert!(invalid(exec(REBUILD, &erased, json!({ "fdi": 36 }))).contains("no region"));
    assert!(invalid(exec(REBUILD, &s, json!({ "fdi": 16 }))).contains("no upper scan"));
}

// ---- adjust a tooth by hand -----------------------------------------------------------------------------

fn framed() -> CaseState {
    let (s, _) = ok("aligners.teeth.frames", &demo(), Value::Null);
    s
}

fn clinical(out: &Value, which: &str, key: &str) -> f64 {
    out[which]["clinical"][key].as_f64().unwrap()
}

#[test]
fn adjusting_a_tooth_updates_its_pose_in_the_clinical_frame() {
    let state = framed();
    let after = undoable(
        &state,
        ADJUST,
        json!({ "fdi": 33, "mesial_mm": 0.4, "buccal_mm": -0.3, "extrusion_mm": 0.2, "rotation_deg": 5.0, "tip_deg": 2.0 }),
    );
    let pose = al(&after).setup.poses[&fdi(33)];
    let (_, out) = ok(
        ADJUST,
        &state,
        json!({ "fdi": 33, "mesial_mm": 0.4, "buccal_mm": -0.3, "extrusion_mm": 0.2, "rotation_deg": 5.0, "tip_deg": 2.0 }),
    );
    // the clinical description reads back what was asked
    for (k, want) in [
        ("mesial_mm", 0.4),
        ("buccal_mm", -0.3),
        ("extrusion_mm", 0.2),
    ] {
        assert!(
            (clinical(&out, "after", k) - want).abs() < 1e-9,
            "{k} {out}"
        );
    }
    // the rotations compose, so only the dominant one reads back exactly
    assert!((clinical(&out, "after", "rotation_deg") - 5.0).abs() < 0.2);
    assert!((clinical(&out, "after", "tip_deg") - 2.0).abs() < 0.2);
    assert!(pose.t.length() > 0.4);
    // another adjustment composes with the first
    let (more, _) = ok(ADJUST, &after, json!({ "fdi": 33, "mesial_mm": 0.1 }));
    let (_, out) = ok(
        ADJUST,
        &more,
        json!({ "fdi": 33, "mesial_mm": 0.0, "extrusion_mm": 0.0, "reset": false, "delta": {} }),
    );
    assert!((clinical(&out, "after", "mesial_mm") - 0.5).abs() < 0.05);
    // the mesial direction follows the side of the arch: the same word moves 31 and 41 toward their own midline
    let (a, _) = ok(ADJUST, &state, json!({ "fdi": 31, "mesial_mm": 1.0 }));
    let (b, _) = ok(ADJUST, &state, json!({ "fdi": 41, "mesial_mm": 1.0 }));
    let ta = al(&a).setup.poses[&fdi(31)].t;
    let tb = al(&b).setup.poses[&fdi(41)].t;
    assert!(ta.length() > 0.9 && tb.length() > 0.9);
    // a pose delta replaces the pose when absolute, resets go back to rest, lock keeps it
    let (abs, _) = ok(
        ADJUST,
        &after,
        json!({ "fdi": 33, "delta": { "t": [0.0, 0.0, 1.0] }, "absolute": true, "lock": true }),
    );
    assert_eq!(al(&abs).setup.poses[&fdi(33)].t, DVec3::Z);
    assert!(al(&abs).setup.locked.contains(&fdi(33)));
    let (reset, _) = ok(ADJUST, &after, json!({ "fdi": 33, "reset": true }));
    assert!(!al(&reset).setup.poses.contains_key(&fdi(33)));
    // errors
    assert!(invalid(exec(ADJUST, &state, json!({ "fdi": 33 }))).contains("no movement"));
    assert!(
        invalid(exec(
            ADJUST,
            &demo(),
            json!({ "fdi": 33, "mesial_mm": 1.0 })
        ))
        .contains("is not measured")
    );
    assert!(
        invalid(exec(
            ADJUST,
            &state,
            json!({ "fdi": 33, "stage": 4, "mesial_mm": 1.0 })
        ))
        .contains("stage 4")
    );
    assert!(
        invalid(exec(
            ADJUST,
            &state,
            json!({ "fdi": 33, "lock": true, "stage": 0, "mesial_mm": 1.0 })
        ))
        .contains("lock")
    );
}

fn penetration(out: &Value, which: &str, with: u64) -> f64 {
    out[which]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["with"] == with)
        .unwrap()["penetration_mm"]
        .as_f64()
        .unwrap()
}

#[test]
fn a_move_into_a_neighbour_is_reported_as_a_collision() {
    let state = framed();
    // 32 moved mesially towards 31 by 3 mm (they are a few mm apart): the crowns overlap
    let (moved, out) = ok(ADJUST, &state, json!({ "fdi": 32, "mesial_mm": 3.0 }));
    assert_eq!(out["collides"], true, "{out}");
    let hit = out["collisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["with"] == 31)
        .unwrap();
    assert_eq!(hit["collides"], true);
    assert!(hit["penetration_mm"].as_f64().unwrap() > 2.5, "{hit}");
    assert!(penetration(&out, "collisions_before", 31) < 0.5);
    // the other neighbour (33) is away from the move
    assert_eq!(
        out["collisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["with"] == 33)
            .unwrap()["collides"],
        false
    );
    // the same move is refused on request, and nothing changes
    let msg = invalid(exec(
        ADJUST,
        &state,
        json!({ "fdi": 32, "mesial_mm": 3.0, "refuse_on_collision": true }),
    ));
    assert!(msg.contains("collide") && msg.contains("31"), "{msg}");
    // a move away from both neighbours is not worse than before
    let (_, small) = ok(
        ADJUST,
        &state,
        json!({ "fdi": 32, "mesial_mm": 0.0, "extrusion_mm": 0.5 }),
    );
    assert!(
        penetration(&small, "collisions", 31) <= penetration(&small, "collisions_before", 31) + 0.2,
        "{small}"
    );
    // moving it back clears the collision with 31
    let (_, back) = ok(ADJUST, &moved, json!({ "fdi": 32, "mesial_mm": -3.0 }));
    assert!(penetration(&back, "collisions", 31) < 0.5, "{back}");
    // the report reads the setup or the stage the move is made in
    assert_eq!(out["stage"], Value::Null);
}

#[test]
fn a_stage_can_be_adjusted_on_its_own() {
    let mut state = framed();
    let mut data = al(&state).clone();
    data.stages = (0..3).map(Stage::new).collect();
    state.set_module(MODULE_ID, dc_case::ModuleState::new(data));
    let after = undoable(
        &state,
        ADJUST,
        json!({ "fdi": 33, "stage": 2, "buccal_mm": 0.5 }),
    );
    let st = al(&after);
    assert!(st.setup.poses.is_empty(), "the setup is untouched");
    assert!(st.stages[2].poses.contains_key(&fdi(33)));
    assert!(st.stages[1].poses.is_empty());
    let (_, out) = ok(
        ADJUST,
        &state,
        json!({ "fdi": 33, "stage": 2, "buccal_mm": 0.5 }),
    );
    assert_eq!(out["stage"], 2);
    assert_eq!(out["staging_outdated"], false);
    // a setup move leaves the staging out of date
    let (_, out) = ok(ADJUST, &state, json!({ "fdi": 33, "buccal_mm": 0.5 }));
    assert_eq!(out["staging_outdated"], true);
}
