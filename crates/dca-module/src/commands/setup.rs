//! Commands of the `setup` stage (`dca_algo::setup`).
//!
//! `aligners.planning.setup.automatic` computes the final position of the
//! teeth of every segmented arch (or of the arch given): each tooth to align
//! with its prominent cusps on the patient's arch line (or on the line drawn
//! by the clinician, `line`), the space made by IPR, expansion and incisor
//! advancement in the order of `priority`. The tooth geometry is measured
//! again from the labelled scan (the teeth stage, deterministic); the locked
//! teeth (`Setup::locked`) keep their pose and are the reference; the teeth
//! in `excluded` (out of the aligner) stay at rest.
//!
//! With both arches segmented and not related (no bite registration or
//! virtual bite, and the scans not exported in occlusion:
//! [`dca_algo::occlusion::bite_needed`]) the setup refuses and changes nothing,
//! as the plugin's `case_plan.run_setup`: with both arches the bite comes
//! first.
//!
//! Stored in `AlignersState::setup`: `poses` (the final local pose of each
//! tooth that moves, in its rest frame: the rest pose of the stored tooth
//! frame, centre of resistance origin; a tooth without a pose stays at rest),
//! `ipr` (one entry per contact, total over both surfaces) and `lines` (the
//! arch line each arch was aligned on, case coordinates). The summary of each
//! arch is returned. The poses go to and from the arch-space moves of
//! `setup::run` as the plugin's staging reads them
//! (`staging::q_from_delta`, `staging::delta_from_q`): the shortest rotation,
//! a quaternion with `w >= 0` ([`dca_algo::rotation`]).

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::setup::{self, ToothInput};
use dca_algo::teeth::{self, Which};
use dca_algo::{frame, rotation, staging};
use dca_model::{AlignersState, IprStep, MODULE_ID};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.planning.setup.automatic",
        label: "aligners-menu-planning-setup-automatic",
        doc: "aligners-cmd-setup-automatic-doc",
        params: dca_algo::setup::Params,
        run: run_automatic,
    }]
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn which(arch: Arch) -> Which {
    if arch.is_upper() {
        Which::Upper
    } else {
        Which::Lower
    }
}

fn invalid(msg: impl Into<String>) -> CommandError {
    CommandError::Invalid(msg.into())
}

fn check(params: &setup::Params) -> Result<Vec<Fdi>, CommandError> {
    let numbers = [
        params.line_sigma_mm,
        params.pos_tol_mm,
        params.normal_tol_mm,
        params.rot_tol_deg,
        params.post_normal_tol_mm,
        params.post_rot_tol_deg,
        params.max_derotation_deg,
        params.vertical_max_mm,
        params.level_tol_mm,
        params.contact_tol_mm,
        params.incisor_weight,
        params.expansion_upper_mm,
        params.expansion_lower_posterior_mm,
        params.expansion_lower_canine_mm,
        params.advance_max_mm,
        params.retract_max_mm,
        params.ipr_surface_cap_mm,
        params.enamel_fraction,
    ];
    if numbers.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(invalid(
            "the tolerances and limits must be finite and not negative",
        ));
    }
    if params.line_sigma_mm <= 0.0 || params.incisor_weight <= 0.0 {
        return Err(invalid("line_sigma_mm and incisor_weight must be positive"));
    }
    if !params.line.is_empty() {
        if params.arch == Which::All {
            return Err(invalid(
                "a drawn arch line belongs to one arch: set arch to upper or lower",
            ));
        }
        if params.line.len() < 2 || params.line.iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid("the arch line needs at least two finite points"));
        }
    }
    params
        .excluded
        .iter()
        .map(|&n| Fdi::new(n).map_err(|_| invalid(format!("{n} is not a tooth number"))))
        .collect()
}

fn run_automatic(
    state: &CaseState,
    params: setup::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let excluded = check(&params)?;
    // `case_plan.run_setup`: with both arches the bite comes first, whichever
    // arch is asked for; nothing changes in the case
    if dca_algo::occlusion::bite_needed(state) {
        return Err(crate::commands::messages::invalid(
            "aligners-err-setup-bite-first",
            &[],
        ));
    }
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data)?,
    };
    let explicit = params.arch != Which::All;
    let mut next = state.clone();
    let mut report = Map::new();

    for arch in params.arch.arches() {
        let role = arch.role();
        let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
            if explicit {
                return Err(crate::commands::messages::not_segmented(arch));
            }
            continue;
        };
        let mesh = state.blobs.require(&scan.mesh)?;
        let labelled = state.blobs.require(labels_ref)?;
        let labels = labelled
            .vertex_labels
            .as_ref()
            .ok_or_else(|| crate::commands::messages::labels_missing(arch))?;
        if labels.len() != mesh.vertex_count() {
            return Err(crate::commands::messages::labels_mismatch(
                arch,
                labels.len(),
                mesh.vertex_count(),
            ));
        }
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: &case_vertices,
                seeds: &data.seeds,
            },
            &frame::Params::default(),
        )
        .map_err(|e| CommandError::Failed(e.to_string()))?;
        let to_case = framed.arch_frame.matrix();
        let to_arch = frame::rigid_inverse(&to_case);
        let vertices: Vec<_> = case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect();
        ctx.check_cancelled()?;

        let geo = teeth::run(
            &teeth::Input {
                arch,
                vertices: &vertices,
                tris: &mesh.tris,
                labels,
            },
            &teeth::Params {
                arch: which(arch),
                ..teeth::Params::default()
            },
        )
        .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
        ctx.check_cancelled()?;
        let inputs: BTreeMap<Fdi, ToothInput> = geo
            .teeth
            .iter()
            .map(|(f, t)| (*f, ToothInput::from(t)))
            .collect();
        // the rest frame of each tooth in arch coordinates (the staging's frame): the rest pose
        // of the stored tooth frame, else of the one the teeth stage would store
        let frames: BTreeMap<Fdi, _> = geo
            .teeth
            .iter()
            .map(|(f, t)| {
                let fr = data
                    .tooth_frames
                    .get(f)
                    .copied()
                    .unwrap_or_else(|| teeth::tooth_frame(&t.transformed(&to_case)));
                (*f, to_arch * teeth::rest_pose(&fr).to_mat4())
            })
            .collect();

        // the locked teeth: their final move in arch coordinates
        let locked: Vec<Fdi> = data
            .setup
            .locked
            .iter()
            .copied()
            .filter(|f| inputs.contains_key(f))
            .collect();
        let poses: BTreeMap<_, _> = locked
            .iter()
            .map(|f| {
                let q = data
                    .setup
                    .poses
                    .get(f)
                    .map_or([0.0; 6], staging::q_from_pose);
                (*f, staging::delta_from_q(&frames[f], &q))
            })
            .collect();
        let line: Vec<_> = params
            .line_points()
            .into_iter()
            .map(|p| to_arch.transform_point3(p))
            .collect();
        let fixed: Vec<Fdi> = excluded
            .iter()
            .copied()
            .filter(|f| Arch::of(*f) == arch)
            .collect();
        let out = setup::run(
            &setup::Input {
                arch,
                vertices: &vertices,
                labels,
                sequence: &geo.sequence,
                teeth: &inputs,
                line: (!line.is_empty()).then_some(&line[..]),
                fixed: &fixed,
                poses: &poses,
            },
            &params,
        )
        .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
        ctx.check_cancelled()?;
        let r = &out.result;

        // store: a locked tooth keeps its pose, a tooth out of the aligner stays at rest
        data.setup.poses.retain(|f, _| {
            Arch::of(*f) != arch || inputs.contains_key(f) || data.setup.locked.contains(f)
        });
        let mut teeth_report = Map::new();
        for f in &geo.sequence {
            if data.setup.locked.contains(f) && !fixed.contains(f) {
                continue;
            }
            match r.targets.get(f) {
                Some(d) => {
                    let local = staging::pose_from_q(&staging::q_from_delta(&frames[f], d));
                    teeth_report.insert(
                        f.to_string(),
                        json!({
                            "translation_mm": local.t.to_array(),
                            "rotation_deg": rotation::degrees(&local),
                        }),
                    );
                    data.setup.poses.insert(*f, local);
                }
                None => {
                    data.setup.poses.remove(f);
                }
            }
        }
        data.setup.ipr.retain(|s| Arch::of(s.between[0]) != arch);
        data.setup
            .ipr
            .extend(r.ipr.iter().map(|((a, b), mm)| IprStep {
                between: [*a, *b],
                amount_mm: *mm,
            }));
        data.setup.lines.insert(
            role,
            out.line
                .iter()
                .map(|&p| to_case.transform_point3(p))
                .collect(),
        );
        let ipr: Vec<Value> = r
            .ipr
            .iter()
            .map(|((a, b), mm)| {
                json!({
                    "a": a,
                    "b": b,
                    "mm": mm,
                    "a_surface": setup::facing_surface(*a, *b),
                    "b_surface": setup::facing_surface(*b, *a),
                })
            })
            .collect();
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "frame_source": framed.source,
                "sequence": geo.sequence,
                "moved": r.moved,
                "locked": locked,
                "excluded": fixed,
                "teeth": teeth_report,
                "ipr": ipr,
                "expansion_posterior_mm": r.expansion,
                "expansion_canine_mm": r.expansion_canine,
                "advance_mm": r.advance,
                "residual_crowding_mm": r.residual,
                "spacing_mm": r.spacing,
                "overlap_max_mm": r.overlap_max,
                "midline_shift_mm": r.midline_shift,
                "notes": r.log,
            }),
        );
    }
    if report.is_empty() {
        return Err(crate::commands::messages::none_segmented());
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::{Pose, ScanRole};

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find("aligners.planning.setup.automatic")
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo(state: &CaseState, arch: &str, severity: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                state,
                &json!({ "arch": arch, "severity": severity }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0
    }

    fn data(state: &CaseState) -> AlignersState {
        state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone()
    }

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn the_command_is_registered_as_the_menu_leaf() {
        let spec = crate::commands::find("aligners.planning.setup.automatic").unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        for key in [
            "arch",
            "align_scope",
            "priority",
            "excluded",
            "line",
            "ipr_zone",
        ] {
            assert!(schema["properties"][key].is_object(), "{key}");
        }
        assert!(crate::tree::find("aligners.planning.setup.automatic").is_some());
    }

    #[test]
    fn it_needs_a_segmented_arch_and_valid_parameters() {
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("tooth colours")
        ));
        let lower = demo(&CaseState::default(), "lower", "mild");
        assert!(matches!(
            run(&lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper")
        ));
        for bad in [
            json!({ "line": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] }),
            json!({ "arch": "lower", "line": [[0.0, 0.0, 0.0]] }),
            json!({ "excluded": [99] }),
            json!({ "line_sigma_mm": 0.0 }),
            json!({ "advance_max_mm": -1.0 }),
            json!({ "priority": ["sideways"] }),
            json!({ "unknown": 1 }),
        ] {
            assert!(
                matches!(run(&lower, bad.clone()), Err(CommandError::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_demo_arch_gets_a_setup_stored_in_the_case() {
        let state = demo(&CaseState::default(), "lower", "moderate");
        let (next, out) = run(&state, Value::Null).unwrap();
        let report = &out["lower"];
        assert!(out.get("upper").is_none());
        let d = data(&next);
        let moved: Vec<Fdi> = report["moved"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| fdi(v.as_u64().unwrap() as u8))
            .collect();
        assert!(!moved.is_empty());
        // only incisors and canines are aligned by default
        for f in &moved {
            assert!(dca_algo::anatomy::index(*f) <= 3, "{f}");
        }
        let posed: Vec<Fdi> = d.setup.poses.keys().copied().collect();
        assert_eq!(posed, moved);
        for (f, p) in &d.setup.poses {
            let t = &report["teeth"][f.to_string()];
            assert!((t["translation_mm"][0].as_f64().unwrap() - p.t.x).abs() < 1e-12);
            assert!(p.t.length() < 5.0, "{f}");
        }
        // the IPR plan and the line are stored
        assert_eq!(d.setup.ipr.len(), report["ipr"].as_array().unwrap().len());
        for s in &d.setup.ipr {
            assert!(s.amount_mm > 0.0 && s.amount_mm <= 0.5 + 1e-9);
        }
        assert_eq!(d.setup.lines[&ScanRole::Lower].len(), 96);
        assert!(
            report["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["kind"] == "evaluation")
        );
        // nothing else changed
        let before = data(&state);
        assert_eq!((&before.seeds, &before.labels), (&d.seeds, &d.labels));
        assert_eq!(next.case.scans, state.case.scans);
        // a second run gives the same setup
        let (again, again_out) = run(&next, Value::Null).unwrap();
        assert_eq!(data(&again).setup, d.setup);
        assert_eq!(again_out, out);
    }

    #[test]
    fn the_stored_pose_moves_the_tooth_to_the_target() {
        // the local pose in the rest frame of the stored tooth frame gives back the target
        let state = demo(&CaseState::default(), "upper", "moderate");
        let framed = crate::commands::find("aligners.teeth.frames")
            .unwrap()
            .execute(&state, &Value::Null, &JobCtx::detached())
            .unwrap()
            .0;
        let (a, _) = run(&state, Value::Null).unwrap();
        let (b, _) = run(&framed, Value::Null).unwrap();
        let (pa, pb) = (data(&a).setup.poses, data(&b).setup.poses);
        assert_eq!(pa.keys().collect::<Vec<_>>(), pb.keys().collect::<Vec<_>>());
        for (f, p) in &pa {
            assert!(p.approx_eq(&pb[f], 1e-9, 1e-9), "{f}");
        }
    }

    /// The 719-stage anomaly: where the arch lies in the case changed the sign of
    /// the stored quaternions (`w < 0`), read as turns of 350 degrees and more.
    /// The setup is the same move of each tooth wherever the arch lies, stored as
    /// the shortest rotation, and the staging plans the same aligners.
    #[test]
    fn the_setup_and_its_aligners_do_not_depend_on_where_the_arch_lies() {
        // the staging needs the measured teeth (`aligners.teeth.frames`)
        let state = crate::commands::find("aligners.teeth.frames")
            .unwrap()
            .execute(
                &demo(&CaseState::default(), "lower", "moderate"),
                &Value::Null,
                &JobCtx::detached(),
            )
            .unwrap()
            .0;
        let (base, base_out) = run(&state, Value::Null).unwrap();
        let stage = |s: &CaseState| {
            crate::commands::find("aligners.planning.staging.automatic")
                .unwrap()
                .execute(s, &Value::Null, &JobCtx::detached())
                .unwrap()
        };
        let (_, base_plan) = stage(&base);
        let n = base_plan["lower"]["aligners"].as_u64().unwrap();
        assert!((1..=40).contains(&n), "{n}");
        for (axis, deg) in [
            (glam::DVec3::Z, 170.0),
            (glam::DVec3::new(1.0, 0.3, -0.2), 135.0),
            (glam::DVec3::new(-0.4, 1.0, 0.7), 250.0),
            (glam::DVec3::X, 180.0),
        ] {
            let r = glam::DQuat::from_axis_angle(axis.normalize(), f64::to_radians(deg));
            let delta =
                glam::DMat4::from_rotation_translation(r, glam::DVec3::new(12.0, -30.0, 7.0));
            let moved = crate::commands::occlusion::move_arch(
                &state,
                data(&state),
                Arch::Lower,
                &delta,
                &[],
            );
            let (next, out) = run(&moved, Value::Null).unwrap();
            let (a, b) = (data(&base).setup.poses, data(&next).setup.poses);
            assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
            for (f, p) in &b {
                assert!(p.q.w >= 0.0, "{axis} {deg}: {f} {p:?}");
                // the moved scan is stored in f32 again: the same move within the
                // bounds of the setup oracle (0.01 mm, 0.05 degrees)
                let (dt, turn) = (
                    p.t.distance(a[f].t),
                    rotation::degrees(&p.compose(&a[f].inverse())),
                );
                assert!(
                    dt < 0.01 && turn < 0.05,
                    "{axis} {deg}: {f} {dt} mm {turn} deg"
                );
                let reported = out["lower"]["teeth"][f.to_string()]["rotation_deg"]
                    .as_f64()
                    .unwrap();
                let want = base_out["lower"]["teeth"][f.to_string()]["rotation_deg"]
                    .as_f64()
                    .unwrap();
                assert!(
                    reported <= 30.0 && (reported - want).abs() < 0.05,
                    "{f}: {reported} against {want}"
                );
            }
            let (_, plan) = stage(&next);
            assert_eq!(plan["lower"]["aligners"], n, "{axis} {deg}");
        }
    }

    #[test]
    fn locked_teeth_keep_their_pose_and_excluded_teeth_stay_at_rest() {
        let state = demo(&CaseState::default(), "lower", "moderate");
        let (first, _) = run(&state, Value::Null).unwrap();
        let mut d = data(&first);
        let moved: Vec<Fdi> = d.setup.poses.keys().copied().collect();
        assert!(moved.len() >= 2);
        let (locked, excluded) = (moved[0], moved[1]);
        let hand = Pose::from_translation(d.setup.poses[&locked].t * 0.5);
        d.setup.poses.insert(locked, hand);
        d.setup.locked.insert(locked);
        let mut s = first.clone();
        s.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = run(&s, json!({ "excluded": [excluded.get()] })).unwrap();
        let d = data(&next);
        assert_eq!(d.setup.poses[&locked], hand);
        assert!(!d.setup.poses.contains_key(&excluded));
        let report = &out["lower"];
        assert_eq!(report["locked"], json!([locked.get()]));
        assert_eq!(report["excluded"], json!([excluded.get()]));
        let notes = report["notes"].as_array().unwrap();
        assert_eq!(notes[0]["kind"], "out_of_aligner");
        assert!(notes.iter().any(|n| n["kind"] == "locked"));
    }

    /// `state` after the virtual bite.
    fn bitten(state: &CaseState) -> CaseState {
        crate::commands::find("aligners.occlusion.virtual-bite")
            .unwrap()
            .execute(state, &Value::Null, &JobCtx::detached())
            .unwrap()
            .0
    }

    #[test]
    fn with_both_arches_the_bite_comes_first() {
        // `case_plan.run_setup`: both arches with their teeth, not related
        let both = super::super::occlusion::tests::two_demo_arches();
        assert!(dca_algo::occlusion::bite_needed(&both));
        let before = data(&both);
        for params in [
            Value::Null,
            json!({ "arch": "lower" }),
            json!({ "arch": "upper" }),
        ] {
            let err = run(&both, params.clone()).unwrap_err();
            let CommandError::Invalid(m) = err else {
                panic!("not a refusal: {err:?}");
            };
            assert!(
                m.starts_with("with both arches the bite comes first") && m.contains("Bite"),
                "{params}: {m}"
            );
        }
        // the refusal changes nothing in the case
        assert_eq!(data(&both), before);
        assert!(before.setup.poses.is_empty() && !before.bite_registered);

        // allowed after the virtual bite
        let virtual_bite = bitten(&both);
        assert!(!dca_algo::occlusion::bite_needed(&virtual_bite));
        let (_, out) = run(&virtual_bite, Value::Null).unwrap();
        assert!(out["lower"].is_object() && out["upper"].is_object());

        // allowed after a registration (the flag the registration commands set)
        let mut registered = both.clone();
        let mut d = data(&both);
        d.bite_registered = true;
        registered.set_module(MODULE_ID, ModuleState::new(d));
        assert!(run(&registered, Value::Null).is_ok());

        // allowed for scans already in occlusion (exported in occlusion): the
        // arch frames say so without any registration
        let mut exported = virtual_bite.clone();
        let mut d = data(&virtual_bite);
        d.bite_registered = false;
        exported.set_module(MODULE_ID, ModuleState::new(d));
        assert!(!dca_algo::occlusion::bite_needed(&exported));
        assert!(run(&exported, Value::Null).is_ok());

        // allowed with a single arch
        let lower = demo(&CaseState::default(), "lower", "mild");
        assert!(!dca_algo::occlusion::bite_needed(&lower));
        assert!(run(&lower, Value::Null).is_ok());
        // and with two scans when one arch has no tooth colours yet
        let mut d = data(&both);
        d.labels.remove(&ScanRole::Upper);
        let mut one = both.clone();
        one.set_module(MODULE_ID, ModuleState::new(d));
        assert!(!dca_algo::occlusion::bite_needed(&one));
        let (_, out) = run(&one, Value::Null).unwrap();
        assert!(out["lower"].is_object() && out.get("upper").is_none());
    }

    #[test]
    fn a_drawn_line_is_followed_and_both_arches_are_set_up() {
        let both = bitten(&demo(
            &demo(&CaseState::default(), "lower", "mild"),
            "upper",
            "mild",
        ));
        let (all, out) = run(&both, Value::Null).unwrap();
        assert!(out["lower"].is_object() && out["upper"].is_object());
        let d = data(&all);
        assert_eq!(d.setup.lines.len(), 2);
        // the stored line given back as the drawn line of the upper arch
        let line: Vec<[f64; 3]> = d.setup.lines[&ScanRole::Upper]
            .iter()
            .map(|p| p.to_array())
            .collect();
        let (drawn, out2) = run(&all, json!({ "arch": "upper", "line": line })).unwrap();
        assert!(out2.get("lower").is_none());
        let d2 = data(&drawn);
        assert!(
            out2["upper"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["kind"] == "arch_line" && n["drawn"] == true)
        );
        // the lower arch is untouched
        for (f, p) in &d.setup.poses {
            if Arch::of(*f) == Arch::Lower {
                assert_eq!(d2.setup.poses[f], *p);
            }
        }
    }
}
