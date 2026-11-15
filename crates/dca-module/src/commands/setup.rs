//! Commands of the `setup` stage (`dca_algo::setup`).
//!
//! `aligners.planning.setup.automatic` computes the final position of the
//! teeth of every segmented arch (or of the arch given): each tooth to align
//! with its prominent cusps on the patient's arch line (or on the line drawn
//! by the clinician, `line`), the space made by IPR, expansion and incisor
//! advancement in the order of `priority`. The tooth geometry is measured
//! again from the labelled scan (the teeth stage, deterministic); the locked
//! teeth (`Setup::locked`) keep their pose and are the reference; the teeth
//! in `excluded` (out of the aligner) stay at rest. A parameter not given is
//! the one of the case settings (`Settings::setup`), which
//! `aligners.settings.setup` changes inside the ranges of the plugin's
//! sliders; the menu leaf and `aligners.auto` give none.
//!
//! `aligners.planning.setup.reset` takes the final position of every arch
//! (or of the arch given) back to the initial one, as the plugin's
//! `oa.reset_setup`: poses, IPR, arch line, result and the aligners made
//! from it go; the locks, the drawn line and the settings stay
//! ([`AlignersState::reset_setup`]).
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
//! `ipr` (one entry per contact, total over both surfaces), `lines` (the
//! arch line each arch was aligned on, case coordinates) and `results` (the
//! figures and the log of each arch, the plugin's `setup_json`). The summary
//! of each arch is returned. The poses go to and from the arch-space moves of
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
use dca_model::{
    AlignersState, IprStep, MODULE_ID, SetupIprZone, SetupLeveling, SetupMidline, SetupScope,
    SetupSummary, SpacePriority,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Id of the automatic final position.
pub(crate) const SETUP: &str = "aligners.planning.setup.automatic";
/// Id of the command that changes the final position parameters of the case.
pub(crate) const SETUP_SETTINGS: &str = "aligners.settings.setup";
/// Id of the command that takes the final position back to the initial one.
pub(crate) const SETUP_RESET: &str = "aligners.planning.setup.reset";

/// The registered commands of this stage.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: SETUP,
            label_key: "aligners-menu-planning-setup-automatic",
            doc_key: "aligners-cmd-setup-automatic-doc",
            params_schema: || schemars::schema_for!(setup::Params).to_value(),
            run: |state, params, ctx| run_resolved(state, params.clone(), ctx),
        },
        stage_command! {
            id: SETUP_SETTINGS,
            label: "aligners-cmd-setup-settings",
            doc: "aligners-cmd-setup-settings-doc",
            params: SettingsPatch,
            run: run_settings,
        },
        stage_command! {
            id: SETUP_RESET,
            label: "aligners-cmd-setup-reset",
            doc: "aligners-cmd-setup-reset-doc",
            params: ResetParams,
            run: run_reset,
        },
    ]
}

/// Parameters of `aligners.settings.setup`: the settings to change (the
/// others stay), each inside the range of the plugin's slider.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsPatch {
    /// Teeth to align (`anterior`: incisors and canines, `premolars`, `all`).
    align_scope: Option<SetupScope>,
    /// Smoothing of the arch line from the cusps (1-20 mm).
    line_sigma_mm: Option<f64>,
    /// Incisors and canines: distance from the line tolerated (0-2 mm).
    normal_tol_mm: Option<f64>,
    /// Incisors and canines: rotation tolerated (0-30 degrees).
    rot_tol_deg: Option<f64>,
    /// Premolars and molars: distance from the line tolerated (0-2 mm).
    post_normal_tol_mm: Option<f64>,
    /// Premolars and molars: rotation tolerated (0-30 degrees).
    post_rot_tol_deg: Option<f64>,
    /// Move along the arch tolerated (0-1 mm).
    pos_tol_mm: Option<f64>,
    /// Tight contact tolerated (0-0.5 mm).
    contact_tol_mm: Option<f64>,
    /// First and second molars held along the arch.
    fix_molars: Option<bool>,
    /// The teeth held along the arch are still derotated and put on the line.
    align_fixed: Option<bool>,
    /// Third molars held.
    fix_third: Option<bool>,
    /// Midline (`keep`, `center`, `balance`).
    midline: Option<SetupMidline>,
    /// Weight of the midline (1-20).
    incisor_weight: Option<f64>,
    /// Order in which space is made (`exp_ipr_pro`, `ipr_exp_pro`, `exp_pro_ipr`).
    priority: Option<SpacePriority>,
    /// Largest expansion per side of the upper arch (0-4 mm).
    expansion_upper_mm: Option<f64>,
    /// Largest expansion per side of the lower posterior teeth (0-4 mm).
    expansion_lower_posterior_mm: Option<f64>,
    /// Largest expansion per side of the lower canines (0-3 mm).
    expansion_lower_canine_mm: Option<f64>,
    /// Largest incisor advancement (0-4 mm).
    advance_max_mm: Option<f64>,
    /// Largest incisor retraction (0-4 mm).
    retract_max_mm: Option<f64>,
    /// Close the spaces between the teeth.
    close_spaces: Option<bool>,
    /// Contacts where IPR is allowed (`none`, `anterior`, `anterior_premolars`, `all`).
    ipr_zone: Option<SetupIprZone>,
    /// Largest IPR per surface (0-0.5 mm).
    ipr_surface_cap_mm: Option<f64>,
    /// Largest fraction of the enamel removed (0.1-0.5).
    enamel_fraction: Option<f64>,
    /// Vertical levelling (`smooth`, `flat`, `none`).
    leveling: Option<SetupLeveling>,
    /// Largest vertical correction (0-3 mm).
    vertical_max_mm: Option<f64>,
    /// Vertical step tolerated (0-2 mm).
    level_tol_mm: Option<f64>,
    /// Level the canines too.
    level_canines: Option<bool>,
    /// Step of the − and + buttons of "Move a tooth" for translations (0.01-1 mm).
    adjust_step_mm: Option<f64>,
    /// Step of the − and + buttons of "Move a tooth" for rotations (0.1-10 degrees).
    adjust_step_deg: Option<f64>,
}

/// Parameters of `aligners.planning.setup.reset`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ResetParams {
    /// Arch to reset (`upper` or `lower`); every arch when absent.
    arch: Option<Arch>,
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

/// The automatic final position with the parameters `params` (an object of
/// [`setup::Params`] fields, or null): each parameter not given is the one
/// of the case settings (`Settings::setup`). The command runs this; so does
/// whatever computes the final position again for the user.
pub(crate) fn run_resolved(
    state: &CaseState,
    params: Value,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let settings = aligners_data(state)?.settings.setup;
    let mut merged = serde_json::to_value(setup::Params::from_settings(&settings))
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    match params {
        Value::Null => {}
        Value::Object(given) => {
            if let Some(m) = merged.as_object_mut() {
                m.extend(given);
            }
        }
        other => {
            return Err(invalid(format!(
                "parameters: expected an object, got {other}"
            )));
        }
    }
    run_automatic(state, super::parse_params(&merged)?, ctx)
}

/// The Aligners data of the case (a copy), or new ones.
fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
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
    let mut data = aligners_data(state)?;
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
        let closed = state.blobs.require(&scan.mesh)?;
        let labelled = state.blobs.require(labels_ref)?;
        let labels = labelled
            .vertex_labels
            .as_ref()
            .ok_or_else(|| crate::commands::messages::labels_missing(arch))?;
        if labels.len() != closed.vertex_count() {
            return Err(crate::commands::messages::labels_mismatch(
                arch,
                labels.len(),
                closed.vertex_count(),
            ));
        }
        // the scan as it was read, where the teeth stage measures
        let mesh = crate::open_scan::open_part(&data, &scan.mesh, closed)?;
        let labels = crate::open_scan::open_labels(labels, &mesh);
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: &case_vertices,
                seeds: &data.seeds,
                occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
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
        // the line given, else the one the clinician drew for this arch
        // (`case_plan.run_setup`: `line=arch_line_stored(obj)`)
        let given = params.line_points();
        let line: Vec<_> = if given.is_empty() {
            data.setup.drawn.get(&role).cloned().unwrap_or_default()
        } else {
            given
        }
        .into_iter()
        .map(|p| to_arch.transform_point3(p))
        .collect();
        // the teeth given and those left out of the aligner stay at rest
        // (`case_plan.out_of_aligner_teeth`)
        let mut fixed: Vec<Fdi> = excluded
            .iter()
            .copied()
            .filter(|f| Arch::of(*f) == arch)
            .collect();
        for f in data.out_of_aligner(role) {
            if !fixed.contains(&f) {
                fixed.push(f);
            }
        }
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
        let notes = r
            .log
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CommandError::Failed(e.to_string()))?;
        let end = data.aligner_end_of(role);
        data.setup.results.insert(
            role,
            SetupSummary {
                expansion_posterior_mm: r.expansion,
                expansion_canine_mm: r.expansion_canine,
                advance_mm: r.advance,
                residual_crowding_mm: r.residual,
                spacing_mm: r.spacing,
                notes,
                aligner_end: end,
            },
        );
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

fn run_settings(
    state: &CaseState,
    patch: SettingsPatch,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let mut new = data.settings.setup;
    macro_rules! set {
        ($($field:ident),* $(,)?) => {$(
            if let Some(v) = patch.$field { new.$field = v; }
        )*};
    }
    set!(
        align_scope,
        line_sigma_mm,
        normal_tol_mm,
        rot_tol_deg,
        post_normal_tol_mm,
        post_rot_tol_deg,
        pos_tol_mm,
        contact_tol_mm,
        fix_molars,
        align_fixed,
        fix_third,
        midline,
        incisor_weight,
        priority,
        expansion_upper_mm,
        expansion_lower_posterior_mm,
        expansion_lower_canine_mm,
        advance_max_mm,
        retract_max_mm,
        close_spaces,
        ipr_zone,
        ipr_surface_cap_mm,
        enamel_fraction,
        leveling,
        vertical_max_mm,
        level_tol_mm,
        level_canines,
        adjust_step_mm,
        adjust_step_deg,
    );
    // the sliders' ranges: a value outside them changes nothing
    if let Some((name, value, lo, hi)) = new.out_of_range() {
        return Err(crate::commands::messages::invalid(
            "aligners-err-setup-settings-range",
            &[
                ("name", name),
                ("value", &value.to_string()),
                ("min", &lo.to_string()),
                ("max", &hi.to_string()),
            ],
        ));
    }
    data.settings.setup = new;
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    let out = serde_json::to_value(new).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((next, out))
}

fn run_reset(
    state: &CaseState,
    p: ResetParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let arches = match p.arch {
        Some(arch) => vec![arch],
        None => vec![Arch::Upper, Arch::Lower],
    };
    let mut cleared = Vec::new();
    for arch in arches {
        cleared.extend(data.reset_setup(arch.role()));
    }
    let cleared = super::segmentation::in_cleared_order(cleared);
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, json!({ "cleared": cleared })))
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

    #[test]
    fn the_teeth_behind_the_end_of_the_aligner_stay_at_rest() {
        let state = demo(&CaseState::default(), "lower", "moderate");
        let exec = |s: &CaseState, id: &str, p: Value| {
            crate::commands::find(id)
                .unwrap()
                .execute(s, &p, &JobCtx::detached())
                .unwrap()
                .0
        };
        let ended = exec(
            &state,
            "aligners.production.aligner-end",
            json!({ "action": "set", "fdi": 46 }),
        );
        assert_eq!(data(&ended).out_of_aligner(ScanRole::Lower), [fdi(47)]);
        let (next, out) = run(&ended, Value::Null).unwrap();
        let d = data(&next);
        assert!(!d.setup.poses.contains_key(&fdi(47)));
        assert!(
            out["lower"]["excluded"]
                .as_array()
                .unwrap()
                .contains(&json!(47))
        );
        // the end it was computed with is recorded: another end is reported
        assert_eq!(
            d.setup.results[&ScanRole::Lower].aligner_end.right,
            Some(fdi(46))
        );
        assert!(!d.setup_end_changed(ScanRole::Lower));
        let cleared = exec(
            &next,
            "aligners.production.aligner-end",
            json!({ "action": "clear" }),
        );
        assert!(data(&cleared).setup_end_changed(ScanRole::Lower));
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

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_settings_are_changed_inside_their_ranges_and_saved() {
        let state = demo(&CaseState::default(), "lower", "mild");
        let (next, out) = exec(
            SETUP_SETTINGS,
            &state,
            json!({ "ipr_zone": "none", "advance_max_mm": 2.5, "priority": "exp_pro_ipr",
                    "adjust_step_mm": 0.25 }),
        )
        .unwrap();
        let s = data(&next).settings.setup;
        assert_eq!(s.ipr_zone, SetupIprZone::None);
        assert_eq!(s.priority, SpacePriority::ExpProIpr);
        assert_eq!((s.advance_max_mm, s.adjust_step_mm), (2.5, 0.25));
        // the others stay
        assert_eq!(
            s.retract_max_mm,
            dca_model::SetupSettings::default().retract_max_mm
        );
        assert_eq!(out["ipr_zone"], "none");
        assert_eq!(out["advance_max_mm"], 2.5);
        // they go through the case file as they are
        let v = serde_json::to_value(data(&next)).unwrap();
        let back: AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(back.settings.setup, s);
        // out of range, unknown or of the wrong kind: refused, nothing changes
        for bad in [
            json!({ "advance_max_mm": 4.5 }),
            json!({ "enamel_fraction": 0.05 }),
            json!({ "line_sigma_mm": 0.5 }),
            json!({ "adjust_step_deg": 0.0 }),
            json!({ "ipr_zone": "everywhere" }),
            json!({ "cons_line_sigma": 5.0 }),
        ] {
            assert!(
                matches!(
                    exec(SETUP_SETTINGS, &next, bad.clone()),
                    Err(CommandError::Invalid(_))
                ),
                "{bad}"
            );
        }
        let err = exec(
            SETUP_SETTINGS,
            &next,
            json!({ "advance_max_mm": 4.5, "ipr_zone": "all" }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("advance_max_mm") && err.contains('4'), "{err}");
    }

    #[test]
    fn the_setup_reads_the_stored_settings_and_the_given_ones_win() {
        let state = demo(&CaseState::default(), "lower", "moderate");
        let (plain, _) = run(&state, Value::Null).unwrap();
        assert!(!data(&plain).setup.ipr.is_empty(), "the demo needs IPR");
        // stored: no IPR anywhere, all the teeth aligned
        let (set, _) = exec(
            SETUP_SETTINGS,
            &state,
            json!({ "ipr_zone": "none", "align_scope": "all" }),
        )
        .unwrap();
        let (stored, out) = run(&set, Value::Null).unwrap();
        assert!(data(&stored).setup.ipr.is_empty());
        // the same as giving them to the command
        let (given, out_given) =
            run(&state, json!({ "ipr_zone": "none", "align_scope": "all" })).unwrap();
        assert_eq!(data(&given).setup.poses, data(&stored).setup.poses);
        assert_eq!(out["lower"]["moved"], out_given["lower"]["moved"]);
        // a parameter given wins over the stored one
        let (over, _) = run(
            &set,
            json!({ "ipr_zone": "anterior_premolars", "align_scope": "anterior" }),
        )
        .unwrap();
        assert_eq!(data(&over).setup.poses, data(&plain).setup.poses);
        assert_eq!(data(&over).setup.ipr, data(&plain).setup.ipr);
        // not an object: refused
        assert!(matches!(
            run(&state, json!([1])),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_result_is_stored_and_survives_the_case_file() {
        let state = demo(&CaseState::default(), "lower", "moderate");
        let (next, out) = run(&state, Value::Null).unwrap();
        let d = data(&next);
        let r = &d.setup.results[&ScanRole::Lower];
        let o = &out["lower"];
        assert_eq!(
            r.expansion_posterior_mm,
            o["expansion_posterior_mm"].as_f64().unwrap()
        );
        assert_eq!(
            r.expansion_canine_mm,
            o["expansion_canine_mm"].as_f64().unwrap()
        );
        assert_eq!(r.advance_mm, o["advance_mm"].as_f64().unwrap());
        assert_eq!(
            r.residual_crowding_mm,
            o["residual_crowding_mm"].as_f64().unwrap()
        );
        assert_eq!(r.spacing_mm, o["spacing_mm"].as_f64().unwrap());
        assert_eq!(&Value::Array(r.notes.clone()), &o["notes"]);
        // each note reads back as the setup wrote it
        for n in &r.notes {
            serde_json::from_value::<setup::Note>(n.clone()).unwrap();
        }
        let v = serde_json::to_value(&d).unwrap();
        let back: AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(back.setup.results, d.setup.results);
    }

    #[test]
    fn reset_takes_the_final_position_back_like_the_plugin() {
        let both = bitten(&demo(
            &demo(&CaseState::default(), "lower", "moderate"),
            "upper",
            "moderate",
        ));
        let (set, _) = run(&both, Value::Null).unwrap();
        // a locked tooth and an aligner made from the final position (the
        // staging is not what is tested here)
        let mut d = data(&set);
        let locked = *d
            .setup
            .poses
            .keys()
            .find(|f| Arch::of(**f) == Arch::Lower)
            .unwrap();
        d.setup.locked.insert(locked);
        let mut stage = dca_model::Stage::new(1);
        stage.poses = d.setup.poses.clone();
        stage.ipr = d.setup.ipr.clone();
        d.stages.push(stage);
        let mut staged = set.clone();
        staged.set_module(MODULE_ID, ModuleState::new(d));

        // the lower arch only
        let (lower, out) = exec(SETUP_RESET, &staged, json!({ "arch": "lower" })).unwrap();
        assert_eq!(out["cleared"], json!(["setup", "stages"]));
        let d = data(&lower);
        assert!(d.setup.poses.keys().all(|f| Arch::of(*f) == Arch::Upper));
        assert!(
            d.setup
                .ipr
                .iter()
                .all(|s| Arch::of(s.between[0]) == Arch::Upper)
        );
        assert!(!d.setup.lines.contains_key(&ScanRole::Lower));
        assert!(!d.setup.results.contains_key(&ScanRole::Lower));
        assert!(d.setup.results.contains_key(&ScanRole::Upper));
        assert!(
            d.stages
                .iter()
                .all(|s| s.poses.keys().all(|f| Arch::of(*f) == Arch::Upper))
        );
        // the lock and the settings stay, as in the plugin
        assert!(d.setup.locked.contains(&locked));
        assert_eq!(d.settings, data(&staged).settings);
        assert!(!crate::done::leaf_done(&lower, SETUP));

        // every arch: nothing of the final position is left
        let (all, out) = exec(SETUP_RESET, &staged, Value::Null).unwrap();
        assert_eq!(out["cleared"], json!(["setup", "stages"]));
        let d = data(&all);
        assert!(d.setup.poses.is_empty() && d.setup.ipr.is_empty());
        assert!(d.setup.lines.is_empty() && d.setup.results.is_empty());
        assert!(d.stages.is_empty());
        // again: nothing to clear, still a valid command
        let (_, out) = exec(SETUP_RESET, &all, Value::Null).unwrap();
        assert_eq!(out["cleared"], json!([]));
        assert!(matches!(
            exec(SETUP_RESET, &all, json!({ "arch": "bite" })),
            Err(CommandError::Invalid(_))
        ));
    }
}
