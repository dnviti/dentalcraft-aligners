//! Tests of the bite and ramps commands on the demo arches.

use super::*;
use crate::commands::occlusion::tests::{exec, try_exec, two_demo_arches};

fn closed() -> CaseState {
    exec(
        "aligners.occlusion.virtual-bite",
        &two_demo_arches(),
        Value::Null,
    )
    .0
}

/// The demo arches in occlusion with the upper arch `mm` lower: a deep bite.
fn deep(mm: f64) -> CaseState {
    let state = closed();
    let d = data(&state);
    let lower = arch_model(&state, &d, Arch::Lower).unwrap();
    let up = glam::DMat3::from_mat4(lower.to_case).z_axis;
    super::super::occlusion::move_arch(
        &state,
        d,
        Arch::Upper,
        &DMat4::from_translation(-up * mm),
        &[],
    )
}

fn data(s: &CaseState) -> AlignersState {
    s.module_data::<AlignersState>(MODULE_ID).unwrap().clone()
}

#[test]
fn the_commands_are_registered_with_schemas() {
    for id in [
        "aligners.occlusion.bite-with-aligners",
        "aligners.occlusion.bite-ramps",
    ] {
        let spec = crate::commands::find(id).unwrap();
        assert_eq!(spec.schema()["type"], "object");
        assert!(crate::tree::find(id).is_none(), "{id}");
    }
    let schema = crate::commands::find("aligners.occlusion.bite-ramps")
        .unwrap()
        .schema();
    assert!(schema["properties"]["action"].is_object(), "{schema}");
}

#[test]
fn one_arch_has_no_bite_and_its_aligners_no_ramps() {
    use crate::commands::occlusion::tests::assert_not_applicable;
    use dc_case::ScanRole;
    let lower = exec("aligners.demo", &CaseState::default(), Value::Null).0;
    for id in [
        "aligners.occlusion.bite-with-aligners",
        "aligners.occlusion.bite-ramps",
    ] {
        assert_not_applicable(id, &lower, Value::Null);
    }
    assert!(matches!(
        try_exec(
            "aligners.occlusion.bite-with-aligners",
            &lower,
            json!({ "nope": 1 })
        ),
        Err(CommandError::Invalid(_))
    ));

    // the ramps of a deep bite on the upper incisors, in the aligners of both arches
    let (with, _) = exec(
        "aligners.occlusion.bite-ramps",
        &deep(2.5),
        json!({ "overbite_min_mm": 1.0 }),
    );
    let d = data(&with);
    assert!(d.aligner_bite.ramps.enabled && !d.aligner_bite.ramps.ramps.is_empty());
    let both = StageBite::new(&with).unwrap();
    assert!(!both.ramps(Arch::Upper, 0).is_empty());
    assert!(both.shapes_shells() && both.to_arch(Arch::Upper).is_some());
    // the same project with the lower arch gone, its bite still stored (an
    // old file): nothing of it enters the aligners of the upper arch
    let mut upper = with.clone();
    upper.case.scans.retain(|s| s.role != ScanRole::Lower);
    let one = StageBite::new(&upper).unwrap();
    assert!(
        !one.shapes_shells(),
        "no occlusal shaping, and no note about it"
    );
    assert!(one.ramps(Arch::Upper, 0).is_empty());
    assert!(one.to_arch(Arch::Upper).is_none() && one.bite(0, 0).is_none());
    assert!(one.covered(Arch::Upper).is_empty());
    assert!(ramp_solids(&upper, Arch::Upper, 0).unwrap().is_empty());
    for (id, params) in [
        (
            "aligners.occlusion.bite-with-aligners",
            json!({ "mode": "manual" }),
        ),
        (
            "aligners.occlusion.bite-ramps",
            json!({ "action": "propose" }),
        ),
        (
            "aligners.occlusion.bite-ramps",
            json!({ "action": "remove" }),
        ),
        (
            "aligners.occlusion.bite-ramps",
            json!({ "action": "set", "enabled": true }),
        ),
    ] {
        assert_not_applicable(id, &upper, params);
    }
}

#[test]
fn the_bite_with_the_aligners_is_measured_and_its_settings_kept() {
    let state = closed();
    let (next, out) = exec("aligners.occlusion.bite-with-aligners", &state, Value::Null);
    let occ = out["occlusal_thickness_mm"].as_f64().unwrap();
    assert!((0.5..=1.0).contains(&occ), "{out}");
    assert_eq!(out["thickness_mode"], "AUTO");
    assert_eq!(out["closing"], "HINGE");
    assert!(out["contacts"].as_u64().unwrap() > 0, "{out}");
    assert!(
        out["text"]
            .as_str()
            .unwrap()
            .starts_with("Spessore occlusale")
    );
    // the default settings are the case's: nothing to keep
    assert_eq!(data(&next), data(&state));
    // a change of the settings is kept in the case, and used again
    let (kept, out) = exec(
        "aligners.occlusion.bite-with-aligners",
        &state,
        json!({ "mode": "manual", "thickness_mm": 0.7, "closing": "vertical" }),
    );
    assert!((out["occlusal_thickness_mm"].as_f64().unwrap() - 0.7).abs() < 1e-9);
    assert_eq!(out["closing"], "VERTICAL");
    let o = data(&kept).aligner_bite.occlusal;
    assert_eq!(
        (o.mode, o.closing, o.thickness_mm),
        (OcclusalMode::Manual, BiteClosing::Vertical, 0.7)
    );
    let (_, again) = exec("aligners.occlusion.bite-with-aligners", &kept, Value::Null);
    assert_eq!(again["thickness_mode"], "MANUAL");
    // not kept with save false
    let (unsaved, _) = exec(
        "aligners.occlusion.bite-with-aligners",
        &state,
        json!({ "mode": "same", "save": false }),
    );
    assert_eq!(
        data(&unsaved).aligner_bite.occlusal,
        OcclusalSettings::default()
    );
    // out of range
    assert!(matches!(
        try_exec("aligners.occlusion.bite-with-aligners", &state, json!({ "thickness_mm": 9.0 })),
        Err(CommandError::Invalid(m)) if m.contains("thickness_mm")
    ));
}

#[test]
fn the_ramps_are_proposed_on_the_upper_incisors_of_a_deep_bite() {
    let state = closed();
    // the demo bite is not deep: nothing at the default threshold
    let (same, out) = exec("aligners.occlusion.bite-ramps", &state, Value::Null);
    assert_eq!(out["ramps"].as_array().unwrap().len(), 0, "{out}");
    assert!(
        out["reason"]
            .as_str()
            .unwrap()
            .contains("morso non profondo"),
        "{out}"
    );
    assert!(!data(&same).aligner_bite.ramps.enabled);
    // with a lower threshold they go on the upper central and lateral incisors
    let state = deep(2.5);
    let (next, out) = exec(
        "aligners.occlusion.bite-ramps",
        &state,
        json!({ "overbite_min_mm": 1.0 }),
    );
    let teeth: Vec<u64> = out["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_u64().unwrap())
        .collect();
    assert!(!teeth.is_empty(), "{out}");
    assert!(
        teeth.iter().all(|t| [11, 12, 21, 22].contains(t)),
        "{teeth:?}"
    );
    assert_eq!(out["enabled"], true);
    let d = data(&next);
    assert!(
        d.aligner_bite
            .ramps
            .ramps
            .iter()
            .all(|r| r.kind == RampKind::Auto && r.size_mm == [3.0, 2.0, 1.5])
    );
    // a second proposal replaces the first, a manual ramp stays
    let c = arch_case(&next, &d, Arch::Upper).unwrap();
    let f = Fdi::new(14).unwrap();
    let m = &c.crowns[&f];
    let top = m
        .positions
        .iter()
        .map(|p| p.as_dvec3())
        .fold(DVec3::splat(f64::MIN), |a, p| if p.z > a.z { p } else { a });
    let (hand, out) = exec(
        "aligners.occlusion.bite-ramps",
        &next,
        json!({ "action": "add", "tooth": 14, "point": top.to_array(), "normal": [0.0, 0.0, 1.0], "height_mm": 1.0 }),
    );
    assert_eq!(out["added"]["kind"], "manual");
    assert_eq!(out["added"]["size_mm"], json!([3.0, 2.0, 1.0]));
    let (again, out) = exec(
        "aligners.occlusion.bite-ramps",
        &hand,
        json!({ "overbite_min_mm": 1.0 }),
    );
    let rs = data(&again).aligner_bite.ramps.ramps;
    assert_eq!(
        rs.iter().filter(|r| r.kind == RampKind::Manual).count(),
        1,
        "{out}"
    );
    assert_eq!(rs.len(), teeth.len() + 1);
    // remove one tooth, then all
    let (less, out) = exec(
        "aligners.occlusion.bite-ramps",
        &again,
        json!({ "action": "remove", "tooth": 14 }),
    );
    assert_eq!(out["removed"], 1);
    assert_eq!(data(&less).aligner_bite.ramps.ramps.len(), teeth.len());
    let (none, out) = exec(
        "aligners.occlusion.bite-ramps",
        &less,
        json!({ "action": "remove" }),
    );
    assert_eq!(out["removed"], teeth.len());
    assert!(data(&none).aligner_bite.ramps.ramps.is_empty());
    // settings only
    let (set, out) = exec(
        "aligners.occlusion.bite-ramps",
        &none,
        json!({ "action": "set", "enabled": false, "width_mm": 4.0 }),
    );
    assert_eq!(out["enabled"], false);
    assert_eq!(data(&set).aligner_bite.ramps.width_mm, 4.0);
    // bad input
    for p in [
        json!({ "action": "add", "tooth": 14 }),
        json!({ "action": "add", "tooth": 14, "point": [0, 0, 0], "normal": [0, 0, 0] }),
        json!({ "action": "add", "tooth": 99, "point": [0, 0, 0], "normal": [0, 0, 1] }),
        json!({ "width_mm": 50.0 }),
    ] {
        assert!(
            matches!(
                try_exec("aligners.occlusion.bite-ramps", &state, p.clone()),
                Err(CommandError::Invalid(_))
            ),
            "{p}"
        );
    }
}

#[test]
fn the_ramps_are_part_of_the_aligners_the_bite_is_measured_on() {
    let state = deep(2.5);
    let (with, _) = exec(
        "aligners.occlusion.bite-ramps",
        &state,
        json!({ "overbite_min_mm": 1.0 }),
    );
    let off = {
        let (s, _) = exec(
            "aligners.occlusion.bite-ramps",
            &with,
            json!({ "action": "set", "enabled": false }),
        );
        exec(
            "aligners.occlusion.bite-with-aligners",
            &s,
            json!({ "mode": "same", "save": false }),
        )
        .1
    };
    let on = exec(
        "aligners.occlusion.bite-with-aligners",
        &with,
        json!({ "mode": "same", "save": false }),
    )
    .1;
    assert_eq!(off["ramps"], 0);
    assert!(on["ramps"].as_u64().unwrap() > 0);
    // the ramps open the bite where they stand more than the plain aligners
    let o = |v: &Value| v["incisor_opening_mm"].as_f64().unwrap();
    assert!(o(&on) >= o(&off) - 1e-9, "{on} {off}");
}

#[test]
fn the_ramps_follow_their_tooth_at_every_stage() {
    use dca_model::{Stage, ToothFrame};
    let state = deep(2.5);
    let (with, _) = exec(
        "aligners.occlusion.bite-ramps",
        &state,
        json!({ "overbite_min_mm": 1.0 }),
    );
    let mut d = data(&with);
    let f = d.aligner_bite.ramps.ramps[0].fdi;
    d.tooth_frames.insert(
        f,
        ToothFrame {
            pose: Pose::IDENTITY,
            center_of_resistance: DVec3::ZERO,
        },
    );
    let mut s1 = Stage::new(1);
    s1.poses
        .insert(f, Pose::from_translation(DVec3::new(0.5, 0.0, 0.0)));
    d.stages.push(s1);
    let mut moved = with.clone();
    moved.set_module(MODULE_ID, ModuleState::new(d));
    let at = |k: u32| {
        ramp_solids(&moved, Arch::Upper, k)
            .unwrap()
            .into_iter()
            .find(|s| s.fdi == i64::from(f.get()))
            .unwrap()
    };
    let (a, b) = (at(0), at(1));
    // the move is 0.5 mm along the rest frame of the tooth (the case axes), seen in the arch
    let to_arch = frame::rigid_inverse(
        &arch_model(&moved, &data(&moved), Arch::Upper)
            .unwrap()
            .to_case,
    );
    let step = to_arch.transform_vector3(DVec3::new(0.5, 0.0, 0.0));
    for (p, q) in a.vertices.iter().zip(&b.vertices) {
        assert!((*q - *p - step).length() < 1e-9);
    }
    // the ramps are off: no solids
    let (off, _) = exec(
        "aligners.occlusion.bite-ramps",
        &moved,
        json!({ "action": "set", "enabled": false }),
    );
    assert!(ramp_solids(&off, Arch::Upper, 0).unwrap().is_empty());
}

#[test]
fn the_teeth_behind_the_end_of_the_aligner_are_not_under_it() {
    let state = closed();
    let all = StageBite::new(&state).unwrap();
    assert!(all.covered(Arch::Lower).contains(&47));
    let (ended, _) = exec(
        "aligners.production.aligner-end",
        &state,
        json!({ "action": "set", "fdi": 46 }),
    );
    let bite = StageBite::new(&ended).unwrap();
    let covered = bite.covered(Arch::Lower);
    assert!(!covered.contains(&47) && covered.contains(&46) && covered.contains(&37));
    assert_eq!(covered.len(), all.covered(Arch::Lower).len() - 1);
    // the other arch is covered whole
    assert_eq!(bite.covered(Arch::Upper), all.covered(Arch::Upper));
    // the bite is measured with the aligner that ends there
    exec("aligners.occlusion.bite-with-aligners", &ended, Value::Null);
}
