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
//! Stored in `AlignersState::setup`: `poses` (the final local pose of each
//! tooth that moves, in its rest frame: the rest pose of the stored tooth
//! frame, centre of resistance origin; a tooth without a pose stays at rest),
//! `ipr` (one entry per contact, total over both surfaces) and `lines` (the
//! arch line each arch was aligned on, case coordinates). The summary of each
//! arch is returned.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, ModuleState, Pose};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::setup::{self, ToothInput};
use dca_algo::teeth::{self, Which};
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

/// The local pose (in the rest frame `rest`) of the rigid move `delta` of a
/// tooth: `rest^-1 * delta * rest`.
fn local_pose(rest: &Pose, delta: &Pose) -> Pose {
    rest.inverse().compose(delta).compose(rest)
}

/// The rigid move of a tooth for its local pose: `rest * local * rest^-1`.
fn world_delta(rest: &Pose, local: &Pose) -> Pose {
    rest.compose(local).compose(&rest.inverse())
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
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(|| invalid("the Aligners data cannot be read"))?,
    };
    let explicit = params.arch != Which::All;
    let mut next = state.clone();
    let mut report = Map::new();

    for arch in params.arch.arches() {
        let role = arch.role();
        let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
            if explicit {
                return Err(invalid(format!(
                    "the {} arch has no segmented scan: run the segmentation first",
                    arch_name(arch)
                )));
            }
            continue;
        };
        let mesh = state.blobs.require(&scan.mesh)?;
        let labelled = state.blobs.require(labels_ref)?;
        let labels = labelled
            .vertex_labels
            .as_ref()
            .ok_or_else(|| invalid(format!("the {} labels are missing", arch_name(arch))))?;
        if labels.len() != mesh.vertex_count() {
            return Err(invalid(format!(
                "the {} labels do not match the scan ({} labels, {} vertices)",
                arch_name(arch),
                labels.len(),
                mesh.vertex_count()
            )));
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
        let to_case_pose = Pose::from_mat4(&to_case);
        let to_arch_pose = Pose::from_mat4(&to_arch);
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
        // the rest pose of each tooth (case coordinates): the stored tooth frame, else the one
        // the teeth stage would store
        let rest: BTreeMap<Fdi, Pose> = geo
            .teeth
            .iter()
            .map(|(f, t)| {
                let fr = data
                    .tooth_frames
                    .get(f)
                    .copied()
                    .unwrap_or_else(|| teeth::tooth_frame(&t.transformed(&to_case)));
                (*f, teeth::rest_pose(&fr))
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
                let local = data.setup.poses.get(f).copied().unwrap_or(Pose::IDENTITY);
                let delta = to_arch_pose
                    .compose(&world_delta(&rest[f], &local))
                    .compose(&to_case_pose);
                (*f, delta.to_mat4())
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
                    let delta = to_case_pose
                        .compose(&Pose::from_mat4(d))
                        .compose(&to_arch_pose);
                    let local = local_pose(&rest[f], &delta);
                    teeth_report.insert(
                        f.to_string(),
                        json!({
                            "translation_mm": local.t.to_array(),
                            "rotation_deg": local.q.normalize().to_axis_angle().1.to_degrees(),
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
        return Err(invalid(
            "no arch has a segmented scan: run the segmentation first",
        ));
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;

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
            Err(CommandError::Invalid(m)) if m.contains("segmentation")
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

    #[test]
    fn a_drawn_line_is_followed_and_both_arches_are_set_up() {
        let both = demo(
            &demo(&CaseState::default(), "lower", "mild"),
            "upper",
            "mild",
        );
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
