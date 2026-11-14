//! Commands of the `staging` stage (`dca_algo::staging`).
//!
//! `aligners.planning.staging.automatic` divides the movement of every tooth
//! from its rest position to the setup into aligners, within the velocity
//! limits of the evidence (or the case's settings, or the overrides of the
//! command), with collision-free delays and the timing of the IPR. The
//! aligners are stored in `AlignersState::stages`; the result of the
//! planning (pattern, aligners per tooth, collisions, notes) is returned.
//!
//! Inputs from the case: the tooth frames (`aligners.teeth.frames`), the
//! setup (`Setup::poses`, the final local pose of each tooth in its rest
//! frame; a tooth without a pose stays at rest; `Setup::ipr`), the labelled
//! scan (crown points), and the diagnosis (crowding, Little index, score) for
//! the automatic pattern.
//!
//! The crown points are the closed crowns of the reconstruction
//! (`AlignersState::crowns`) when the tooth has one, else the labelled
//! vertices of the scan.
//!
//! The two arches have their own series; `stages` holds the longer of the
//! two and the arch that finishes first repeats its last pose (the passive
//! aligners of the calendar).

use std::collections::BTreeMap;

use super::messages;
use dc_case::{CaseState, Fdi, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::frame;
use dca_algo::staging::{self, Complexity, IprPlan, Level, Pattern, Perio, ToothMove};
use dca_algo::teeth::{self, Which};
use dca_model::{AlignersState, IprStep, MODULE_ID, Stage};
use glam::{DMat4, DVec3};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.planning.staging.automatic",
        label: "aligners-cmd-staging-automatic",
        doc: "aligners-cmd-staging-automatic-doc",
        params: Params,
        run: run_automatic,
    }]
}

/// Parameters of the command: which arches, and overrides of the staging
/// parameters (what is not given comes from the settings of the case).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Params {
    /// Arches to stage (all that have a setup).
    arch: Which,
    /// Distribution pattern.
    pattern: Option<Pattern>,
    /// Periodontal condition.
    perio: Option<Perio>,
    /// Delay teeth that would collide with a neighbour.
    collision: Option<bool>,
    /// Interpenetration of adjacent crowns tolerated at a stage (mm).
    collision_tol_mm: Option<f64>,
    /// Increase the final movement by the overcorrection fractions.
    overcorrection: Option<bool>,
    /// Largest translation per aligner (mm).
    step_linear_mm: Option<f64>,
    /// Largest intrusion or extrusion per aligner (mm).
    step_vertical_mm: Option<f64>,
    /// Largest rotation per aligner (deg).
    step_rotation_deg: Option<f64>,
    /// Largest rotation per aligner for canines and premolars (deg).
    step_rotation_round_deg: Option<f64>,
    /// Largest torque and tip per aligner (deg).
    step_angular_deg: Option<f64>,
    /// Largest move of any crown point per aligner (mm).
    max_crown_step_mm: Option<f64>,
}

impl Params {
    fn staging(&self, data: &AlignersState) -> staging::Params {
        let mut p = staging::Params::from_settings(&data.settings.staging);
        macro_rules! set {
            ($($field:ident),*) => {$(
                if let Some(v) = self.$field { p.$field = v; }
            )*};
        }
        set!(
            pattern,
            perio,
            collision,
            collision_tol_mm,
            overcorrection,
            step_linear_mm,
            step_vertical_mm,
            step_rotation_deg,
            step_rotation_round_deg,
            step_angular_deg,
            max_crown_step_mm
        );
        p
    }
}

pub(crate) fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The Aligners data of the case (a copy), or new ones.
pub(crate) fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

/// The scan of an arch with its per-vertex tooth labels.
pub(crate) struct LabelledScan {
    pub mesh: std::sync::Arc<dc_mesh::TriMesh>,
    pub labels: Vec<u16>,
}

/// The scan and labels of an arch; `Ok(None)` when it has none.
pub(crate) fn labelled_scan(
    state: &CaseState,
    data: &AlignersState,
    role: ScanRole,
) -> Result<Option<LabelledScan>, CommandError> {
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
    };
    let mesh = state.blobs.require(&scan.mesh)?.clone();
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .clone()
        .ok_or_else(crate::commands::messages::tooth_labels_missing)?;
    if labels.len() != mesh.vertex_count() {
        return Err(crate::commands::messages::tooth_labels_mismatch(
            labels.len(),
            mesh.vertex_count(),
        ));
    }
    Ok(Some(LabelledScan { mesh, labels }))
}

/// `+1` when the x axis of the tooth frame points mesially: from the
/// measures of the teeth stage, otherwise from the position of the mesial
/// neighbour.
pub(crate) fn md_sign(
    data: &AlignersState,
    seq: &[Fdi],
    rest: &BTreeMap<Fdi, dc_case::Pose>,
    f: Fdi,
) -> i8 {
    if let Some(g) = data.tooth_geometry.get(&f) {
        return g.md_sign;
    }
    let x = rest[&f].apply_vector(DVec3::X);
    let from = |other: Fdi, towards_mesial: bool| {
        let d = rest.get(&other)?.t - rest[&f].t;
        let s = if x.dot(d) >= 0.0 { 1 } else { -1 };
        Some(if towards_mesial { s } else { -s })
    };
    anatomy::mesial_neighbor(seq, f)
        .and_then(|n| from(n, true))
        .or_else(|| anatomy::distal_neighbor(seq, f).and_then(|n| from(n, false)))
        .unwrap_or(1)
}

fn run_automatic(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let sp = params.staging(&data);
    let explicit = params.arch != Which::All;
    let mut report = serde_json::Map::new();

    for arch in params.arch.arches() {
        let role = arch.role();
        // `why` is the i18n key of the reason
        let skip = |why: &str| -> Result<bool, CommandError> {
            if explicit {
                Err(messages::invalid(
                    "aligners-err-arch-why",
                    &[
                        ("arch", &messages::arch_word(arch)),
                        ("why", &messages::tr(why, &[])),
                    ],
                ))
            } else {
                Ok(true)
            }
        };
        let Some(scan) = labelled_scan(state, &data, role)? else {
            skip("aligners-err-why-not-separated")?;
            continue;
        };
        let fdis: Vec<Fdi> = data
            .tooth_frames
            .keys()
            .copied()
            .filter(|&f| Arch::of(f) == arch)
            .collect();
        if fdis.is_empty() {
            skip("aligners-err-why-not-measured")?;
            continue;
        }
        if !fdis.iter().any(|f| data.setup.poses.contains_key(f)) {
            skip("aligners-err-why-no-setup")?;
            continue;
        }
        let seq = anatomy::arch_sequence(&fdis, arch);
        let rest: BTreeMap<Fdi, dc_case::Pose> = seq
            .iter()
            .map(|&f| (f, teeth::rest_pose(&data.tooth_frames[&f])))
            .collect();

        // the arch frame, as the teeth stage works in it (its z is occlusal)
        let case_vertices: Vec<DVec3> = scan.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: &case_vertices,
                seeds: &data.seeds,
            },
            &frame::Params::default(),
        )
        .map_err(|e| CommandError::Failed(e.to_string()))?;
        let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
        ctx.check_cancelled()?;

        // crown points of each tooth in its rest frame (the labelled vertices)
        let mut crowns: BTreeMap<Fdi, Vec<DVec3>> = BTreeMap::new();
        let inverse: BTreeMap<Fdi, DMat4> = rest
            .iter()
            .map(|(&f, r)| (f, frame::rigid_inverse(&r.to_mat4())))
            .collect();
        // the closed crowns of the reconstruction when they exist, else the
        // labelled vertices of the scan
        for (&f, r) in &data.crowns {
            if let Some(inv) = inverse.get(&f) {
                let mesh = state.blobs.require(r)?;
                crowns.insert(
                    f,
                    mesh.positions
                        .iter()
                        .map(|p| inv.transform_point3(p.as_dvec3()))
                        .collect(),
                );
            }
        }
        for (p, &l) in case_vertices.iter().zip(&scan.labels) {
            if l == 0 {
                continue;
            }
            if let Some(f) = u8::try_from(l).ok().and_then(|n| Fdi::new(n).ok())
                && !data.crowns.contains_key(&f)
                && let Some(inv) = inverse.get(&f)
            {
                crowns.entry(f).or_default().push(inv.transform_point3(*p));
            }
        }

        let mut moves = Vec::with_capacity(seq.len());
        for &f in &seq {
            let q = data
                .setup
                .poses
                .get(&f)
                .map_or([0.0; 6], staging::q_from_pose);
            let frame_arch = to_arch * rest[&f].to_mat4();
            moves.push(ToothMove::new(
                f,
                frame_arch,
                q,
                crowns.remove(&f).unwrap_or_default(),
                md_sign(&data, &seq, &rest, f),
            ));
        }
        let ipr: Vec<IprPlan> = data
            .setup
            .ipr
            .iter()
            .filter(|r| Arch::of(r.between[0]) == arch)
            .map(|r| IprPlan {
                a: r.between[0],
                b: r.between[1],
                mm: r.amount_mm,
            })
            .collect();
        let complexity = complexity_of(&data, role, &moves);

        let mut progress = |fraction: f64| {
            ctx.set_fraction(fraction as f32);
            !ctx.is_cancelled()
        };
        let plan = staging::run_with_progress(
            &staging::Input {
                moves: &moves,
                ipr: &ipr,
                complexity,
            },
            &sp,
            &mut progress,
        )
        .map_err(|_| CommandError::Cancelled)?;

        let new_stages = stages_of_plan(&plan, &ipr);
        merge_arch(&mut data.stages, arch, &new_stages, &seq);

        let mut teeth_out = serde_json::Map::new();
        for (f, st) in &plan.stats {
            teeth_out.insert(
                f.to_string(),
                json!({
                    "aligners": plan.need[f],
                    "moving_stages": st.moving_stages,
                    "max_crown_step_mm": st.max_crown_step,
                }),
            );
        }
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "pattern": plan.pattern,
                "aligners": plan.n_stages,
                "teeth": teeth_out,
                "ipr_stage": plan.ipr_stage.iter().map(|r| json!({"between": [r.a, r.b], "stage": r.stage})).collect::<Vec<_>>(),
                "unresolved": plan.unresolved,
                "final_overlap": plan.final_overlap,
                "notes": plan.log,
            }),
        );
    }
    if report.is_empty() {
        return Err(messages::invalid("aligners-err-nothing-to-stage", &[]));
    }
    let mut next = state.clone();
    let total = data.stages.len();
    next.set_module(MODULE_ID, ModuleState::new(data));
    report.insert("total_aligners".to_owned(), json!(total));
    Ok((next, Value::Object(report)))
}

/// What the automatic pattern needs: the diagnosis of the arch and the
/// distal movement of the first molars.
fn complexity_of(data: &AlignersState, role: ScanRole, moves: &[ToothMove]) -> Complexity {
    let space = data.diagnosis.arches.get(&role);
    let distal = moves
        .iter()
        .filter(|m| anatomy::index(m.fdi) == 6)
        .map(|m| -(m.delta()[0] * f64::from(m.md_sign)))
        .fold(0.0_f64, f64::max);
    Complexity {
        level: data
            .diagnosis
            .complexity_score
            .map_or(Level::Easy, Level::from_score),
        crowding_mm: space.map_or(0.0, |s| s.crowding_mm().max(0.0)),
        little_mm: space.map_or(0.0, |s| s.little_index_mm),
        distalization_mm: distal,
        pattern_override: None,
    }
}

/// The aligners 1..=N of a plan as stages of the case.
fn stages_of_plan(plan: &staging::Plan, ipr: &[IprPlan]) -> Vec<Stage> {
    (1..=plan.n_stages)
        .map(|k| {
            let mut s = Stage::new(k);
            for (&f, qs) in &plan.q {
                s.poses.insert(f, staging::pose_from_q(&qs[k as usize]));
            }
            for r in plan.ipr_stage.iter().filter(|r| r.stage == k) {
                let mm = ipr
                    .iter()
                    .find(|p| (p.a, p.b) == (r.a, r.b) || (p.a, p.b) == (r.b, r.a))
                    .map_or(0.0, |p| p.mm);
                s.ipr.push(IprStep {
                    between: [r.a, r.b],
                    amount_mm: mm,
                });
            }
            s
        })
        .collect()
}

/// Puts the series of one arch into the stages of the case, replacing what
/// the arch had. `seq` are the teeth of the arch (all of them get a pose in
/// every stage). The arch that finishes first repeats its last pose. The
/// aligners after the end of both series are dropped: an older, longer plan
/// of this arch leaves nothing behind.
fn merge_arch(stages: &mut Vec<Stage>, arch: Arch, new: &[Stage], seq: &[Fdi]) {
    for s in stages.iter_mut() {
        s.poses.retain(|f, _| Arch::of(*f) != arch);
        s.ipr.retain(|r| Arch::of(r.between[0]) != arch);
    }
    stages.sort_by_key(|s| s.index);
    // where the series of the other arch ends: its last movement or IPR
    let other_end = [Arch::Lower, Arch::Upper]
        .into_iter()
        .filter(|a| *a != arch)
        .map(|a| dca_algo::schedule::active_stages(a, stages))
        .chain(stages.iter().filter(|s| !s.ipr.is_empty()).map(|s| s.index))
        .max()
        .unwrap_or(0);
    stages.retain(|s| s.index as usize <= new.len().max(other_end as usize));
    let total = stages.len().max(new.len());
    for k in stages.len() + 1..=total {
        stages.push(Stage::new(k as u32));
    }
    for (i, s) in stages.iter_mut().enumerate() {
        if new.is_empty() {
            break;
        }
        let src = &new[i.min(new.len() - 1)];
        for &f in seq {
            if let Some(p) = src.poses.get(&f) {
                s.poses.insert(f, *p);
            }
        }
        if let Some(own) = new.get(i) {
            s.ipr.extend(own.ipr.iter().copied());
        }
    }
    // the other arch repeats its last pose up to the end
    for i in 1..stages.len() {
        let (before, rest) = stages.split_at_mut(i);
        for (f, p) in &before[i - 1].poses {
            rest[0].poses.entry(*f).or_insert(*p);
        }
    }
    stages.retain(|s| !s.poses.is_empty() || !s.ipr.is_empty());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dc_case::Pose;
    use dca_algo::schedule;
    use glam::DQuat;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    /// A demo arch with its tooth frames and a setup that moves some teeth.
    pub(crate) fn staged_input(arch: &str, state: &CaseState) -> CaseState {
        let s = exec(
            "aligners.demo",
            state,
            json!({ "arch": arch, "severity": "mild" }),
        )
        .unwrap()
        .0;
        let s = exec("aligners.teeth.frames", &s, Value::Null).unwrap().0;
        let mut data = aligners_data(&s).unwrap();
        let fdis: Vec<Fdi> = data.tooth_frames.keys().copied().collect();
        for (i, f) in fdis.iter().enumerate() {
            // 0.2 mm, 0.6 mm, 1.0 mm along the tooth x axis, some rotation and tip
            let mm = 0.2 * ((i % 3) as f64 + 1.0);
            let rot = if i % 4 == 0 {
                3.0_f64.to_radians()
            } else {
                0.0
            };
            data.setup.poses.insert(
                *f,
                Pose::new(DVec3::new(mm, 0.0, 0.0), DQuat::from_rotation_z(rot)),
            );
        }
        let mut s = s;
        s.set_module(MODULE_ID, ModuleState::new(data));
        s
    }

    #[test]
    fn the_command_is_registered_for_the_cli_and_not_in_the_menu() {
        let spec = crate::commands::find("aligners.planning.staging.automatic").unwrap();
        let schema = spec.schema();
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["arch"].is_object());
        assert!(crate::tree::find("aligners.planning.staging.automatic").is_none());
    }

    #[test]
    fn it_needs_a_setup() {
        let err = exec(
            "aligners.planning.staging.automatic",
            &CaseState::default(),
            Value::Null,
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0;
        let s = exec("aligners.teeth.frames", &s, Value::Null).unwrap().0;
        let err = exec(
            "aligners.planning.staging.automatic",
            &s,
            json!({ "arch": "lower" }),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("final position")));
        let err = exec(
            "aligners.planning.staging.automatic",
            &s,
            json!({ "arch": "upper" }),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("upper")));
        assert!(
            exec(
                "aligners.planning.staging.automatic",
                &s,
                json!({ "speed": 3 })
            )
            .is_err()
        );
    }

    #[test]
    fn it_stages_the_demo_arch_and_stores_the_aligners() {
        let s = staged_input("lower", &CaseState::default());
        let (next, out) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let lower = &out["lower"];
        let n = lower["aligners"].as_u64().unwrap() as usize;
        // 0.6 mm at 0.25 mm per aligner: 3 aligners
        assert!((2..=24).contains(&n), "{n}");
        assert_eq!(out["total_aligners"], n);
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.stages.len(), n);
        assert_eq!(data.stages[0].index, 1);
        assert_eq!(data.stages[n - 1].index as usize, n);
        assert_eq!(data.stages[0].poses.len(), 14);
        // the last aligner is the setup
        for (f, want) in &data.setup.poses {
            let got = &data.stages[n - 1].poses[f];
            assert!(got.approx_eq(want, 1e-9, 1e-9), "{f}");
        }
        // no step is larger than the cap
        let q = |k: usize, f: &Fdi| staging::q_from_pose(&data.stages[k].poses[f]);
        for f in data.setup.poses.keys() {
            for k in 1..n {
                let (a, b) = (q(k - 1, f), q(k, f));
                let step = DVec3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]).length();
                assert!(step <= 0.25 + 1e-9, "{f} stage {k}: {step}");
            }
        }
        // a second run gives the same aligners
        let (again, _) = exec("aligners.planning.staging.automatic", &next, Value::Null).unwrap();
        assert_eq!(
            again
                .module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .stages,
            data.stages
        );
        // a different pattern and limits change the series
        let (slow, out) = exec(
            "aligners.planning.staging.automatic",
            &s,
            json!({ "pattern": "max_velocity", "step_linear_mm": 0.1, "collision": false }),
        )
        .unwrap();
        assert_eq!(out["lower"]["pattern"], "max_velocity");
        assert!(
            slow.module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .stages
                .len()
                > n
        );
    }

    #[test]
    fn staging_again_drops_the_aligners_of_a_longer_old_plan() {
        // the long-way turns left plans of 700 aligners: staged again, only the new ones stay
        let id = "aligners.planning.staging.automatic";
        let s = staged_input("lower", &CaseState::default());
        let (planned, out) = exec(id, &s, Value::Null).unwrap();
        let n = out["lower"]["aligners"].as_u64().unwrap() as usize;
        let mut d = aligners_data(&planned).unwrap();
        let last = d.stages.last().unwrap().clone();
        for k in d.stages.len() as u32 + 1..=700 {
            let mut st = last.clone();
            st.index = k;
            st.ipr.clear();
            d.stages.push(st);
        }
        let mut old = planned.clone();
        old.set_module(MODULE_ID, ModuleState::new(d));
        let (again, out) = exec(id, &old, Value::Null).unwrap();
        assert_eq!(out["total_aligners"], n);
        let stages = &aligners_data(&again).unwrap().stages;
        assert_eq!(stages.len(), n);
        assert_eq!(*stages, aligners_data(&planned).unwrap().stages);
    }

    #[test]
    fn setup_poses_saved_with_negative_w_are_staged_the_short_way() {
        // projects saved before the fix hold the setup turns with w < 0: read the
        // long way round, a 3 degree turn was 357 degrees, hundreds of aligners
        let s = staged_input("lower", &CaseState::default());
        let mut d = aligners_data(&s).unwrap();
        for p in d.setup.poses.values_mut() {
            p.q = -p.q;
        }
        let mut saved = s.clone();
        saved.set_module(MODULE_ID, ModuleState::new(d));
        let id = "aligners.planning.staging.automatic";
        let (a, out_a) = exec(id, &s, Value::Null).unwrap();
        let (b, out_b) = exec(id, &saved, Value::Null).unwrap();
        assert_eq!(out_b["lower"]["aligners"], out_a["lower"]["aligners"]);
        let (da, db) = (aligners_data(&a).unwrap(), aligners_data(&b).unwrap());
        assert_eq!(db.stages.len(), da.stages.len());
        for (sa, sb) in da.stages.iter().zip(&db.stages) {
            for (f, p) in &sb.poses {
                // the same rotation vectors to the bit: the same poses
                assert_eq!(*p, sa.poses[f], "{f}");
                // never more than the 3 degrees of the setup, w >= 0 as stored now
                assert!(dca_algo::rotation::degrees(p) <= 3.0 + 1e-9, "{f}");
                assert!(p.q.w >= 0.0, "{f}");
            }
        }
    }

    #[test]
    fn the_two_arches_keep_their_own_series() {
        let s = staged_input("lower", &CaseState::default());
        let s = staged_input("upper", &s);
        // the two helper calls rebuilt the lower arch's setup too: stage both
        let (next, out) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let (nl, nu) = (
            out["lower"]["aligners"].as_u64().unwrap() as u32,
            out["upper"]["aligners"].as_u64().unwrap() as u32,
        );
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.stages.len() as u32, nl.max(nu));
        assert_eq!(schedule::active_stages(Arch::Lower, &data.stages), nl);
        assert_eq!(schedule::active_stages(Arch::Upper, &data.stages), nu);
        assert!(data.stages.iter().all(|s| s.poses.len() == 28));
        // staging one arch again leaves the other as it was
        let (only, _) = exec(
            "aligners.planning.staging.automatic",
            &next,
            json!({ "arch": "upper", "step_linear_mm": 0.1 }),
        )
        .unwrap();
        let d2 = only.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(schedule::active_stages(Arch::Lower, &d2.stages), nl);
        assert!(schedule::active_stages(Arch::Upper, &d2.stages) > nu);
    }

    #[test]
    fn cancelling_stops_the_command() {
        let s = staged_input("lower", &CaseState::default());
        let ctx = JobCtx::detached();
        ctx.cancel_token().cancel();
        let r = crate::commands::find("aligners.planning.staging.automatic")
            .unwrap()
            .execute(&s, &Value::Null, &ctx);
        assert!(matches!(r, Err(CommandError::Cancelled)), "{r:?}");
    }
}
