//! The gizmo of the viewport on the 3D components:
//! `aligners.components.transform` moves, turns or scales one component of
//! the case, as one undo step.
//!
//! | Component | Move and turn | Scale |
//! | --- | --- | --- |
//! | tooth (`fdi`) | its pose in the plan: the final position (setup) or the aligner `stage`, stored as `aligners.planning.movements` stores it | its closed solid, about its centre ([`dca_model::ToothScale`]) |
//! | gingiva (`arch`) | the whole arch, as the bite registration moves it (`occlusion::move_arch`) | refused |
//! | scan (`arch`, or `scan_id`) | the same for the scan of an arch; a bite scan moves alone | refused |
//!
//! The move is a rigid move in case coordinates. It is applied after what
//! the component already has, or, with `absolute`, it is where the component
//! ends up from where it started (the rest position of the tooth, the scan as
//! imported). The total move of each scan and the size of each tooth are
//! kept in [`dca_model::ComponentEdits`], so the inspector shows them and
//! `reset` takes them back. A tooth is moved in the plan: its pose relative
//! to its rest frame ([`teeth::rest_pose`]), the convention of the setup, the
//! staging and the manual adjustment.

use dc_case::{CaseState, Pose, Scan, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::{frame, teeth};
use dca_model::{TOOTH_SCALE_MAX, TOOTH_SCALE_MIN, ToothScale};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::messages;
use super::occlusion::{move_arch, moved_mesh};
use crate::manual::adjust::{AdjustParams, PoseDelta, run_adjust};
use crate::manual::common::{data_of, fdi_of, finish, invalid, pose_in};

/// Id of the command.
pub(crate) const TRANSFORM: &str = "aligners.components.transform";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: TRANSFORM,
        label: "aligners-cmd-components-transform",
        doc: "aligners-cmd-components-transform-doc",
        params: TransformParams,
        run: run_transform,
    }]
}

/// The kind of component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// A tooth (give `fdi`).
    Tooth,
    /// The gingiva of an arch (give `arch`).
    Gingiva,
    /// A scan (give `arch`, or `scan_id`).
    Scan,
}

/// Parameters of the move, turn or scale of a component.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TransformParams {
    /// What to change (required): `tooth`, `gingiva` or `scan`.
    pub component: Option<Kind>,
    /// Tooth number (FDI), for a tooth.
    pub fdi: Option<u8>,
    /// The arch (`upper` or `lower`), for the gingiva or the scan of an arch.
    pub arch: Option<Arch>,
    /// Id of a scan as the case lists it (any scan, a bite scan included),
    /// instead of `arch`.
    pub scan_id: Option<String>,
    /// For a tooth: the aligner (1 or more) whose pose changes; absent: the
    /// final position.
    pub stage: Option<u32>,
    /// Rigid move in case coordinates: `t` in mm and `q` a unit quaternion
    /// `[x, y, z, w]`, applied after what the component already has.
    #[serde(rename = "move")]
    pub movement: Option<PoseDelta>,
    /// For a tooth: size factor about its centre, on the size it has now;
    /// the total stays between 0.5 and 2 times the size it was built with.
    pub scale: Option<f64>,
    /// `move` and `scale` say where the component ends up from where it
    /// started (the rest position of a tooth, a scan as imported, the built
    /// size), not a change of where it is.
    pub absolute: bool,
    /// Back to where the component started first (no move, the built size).
    pub reset: bool,
}

/// What the move of a component starts from and gives.
struct Plan {
    /// The total move now.
    current: Pose,
    /// The total move asked for.
    target: Pose,
}

impl Plan {
    fn new(current: Pose, delta: Option<Pose>, p: &TransformParams) -> Self {
        let start = if p.reset || p.absolute {
            Pose::IDENTITY
        } else {
            current
        };
        let target = delta.map_or(start, |d| d.compose(&start));
        Self { current, target }
    }

    /// The move still to do: from where the component is to the target.
    fn step(&self) -> Pose {
        self.target.compose(&self.current.inverse())
    }
}

/// A rigid move as JSON: translation (mm) and rotation (degrees), and the
/// pose itself.
fn pose_json(p: &Pose) -> Value {
    json!({
        "translation_mm": p.t.to_array(),
        "rotation_deg": frame::rotation_degrees(p),
        "pose": p,
    })
}

fn run_transform(
    state: &CaseState,
    p: TransformParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let kind = p
        .component
        .ok_or_else(|| invalid("give the component: tooth, gingiva or scan"))?;
    let delta = p.movement.as_ref().map(PoseDelta::pose).transpose()?;
    if p.scale.is_some() && kind != Kind::Tooth {
        return Err(messages::invalid("aligners-err-scale-tooth-only", &[]));
    }
    if p.stage.is_some() && kind != Kind::Tooth {
        return Err(invalid("stage applies to a tooth only"));
    }
    if delta.is_none() && p.scale.is_none() && !p.reset {
        return Err(invalid("no change given: set move, scale or reset"));
    }
    match kind {
        Kind::Tooth => transform_tooth(state, &p, delta, ctx),
        Kind::Gingiva => {
            let arch = p
                .arch
                .ok_or_else(|| invalid("give the arch of the gingiva: upper or lower"))?;
            let data = data_of(state)?;
            if !data.gingiva.contains_key(&arch.role()) {
                return Err(invalid(format!(
                    "the {} arch has no gingiva: build the 3D teeth first",
                    messages::arch_word(arch)
                )));
            }
            move_scan(state, arch_scan(state, arch)?, "gingiva", delta, &p)
        }
        Kind::Scan => {
            let scan = match (&p.scan_id, p.arch) {
                (Some(id), _) => state
                    .case
                    .scans
                    .iter()
                    .find(|s| s.id.to_string() == *id)
                    .ok_or_else(|| invalid(format!("the case has no scan {id}")))?,
                (None, Some(arch)) => arch_scan(state, arch)?,
                (None, None) => return Err(invalid("give the arch or the scan_id of the scan")),
            };
            move_scan(state, scan, "scan", delta, &p)
        }
    }
}

/// The scan of `arch`.
fn arch_scan(state: &CaseState, arch: Arch) -> Result<&Scan, CommandError> {
    state.case.scan(arch.role()).ok_or_else(|| {
        messages::invalid(
            "aligners-err-no-scan",
            &[("arch", &messages::arch_word(arch))],
        )
    })
}

/// Moves `scan` rigidly: the scan of an arch with everything of its arch,
/// a bite scan alone.
fn move_scan(
    state: &CaseState,
    scan: &Scan,
    what: &str,
    delta: Option<Pose>,
    p: &TransformParams,
) -> Result<CommandOutput, CommandError> {
    let id = scan.id.to_string();
    let mut data = data_of(state)?;
    let plan = Plan::new(data.components.scan_move(&id), delta, p);
    let step = plan.step();
    data.components.set_scan_move(&id, plan.target);
    let arch = match scan.role {
        ScanRole::Upper => Some(Arch::Upper),
        ScanRole::Lower => Some(Arch::Lower),
        ScanRole::Bite => None,
    };
    let mut next = match arch {
        Some(a) => move_arch(state, data, a, &step.to_mat4(), &[]),
        None => {
            let mut next = state.clone();
            let i = next
                .case
                .scans
                .iter()
                .position(|s| s.id == scan.id)
                .ok_or_else(|| invalid(format!("the case has no scan {id}")))?;
            let old = next.case.scans[i].mesh;
            let mesh = next.blobs.require(&old)?.clone();
            next.case.scans[i].mesh = next.blobs.insert(moved_mesh(&mesh, &step.to_mat4()));
            finish(&mut next, data, &[old]);
            next
        }
    };
    prune(&mut next)?;
    Ok((
        next,
        json!({
            "component": what,
            "arch": arch,
            "scan_id": id,
            "role": scan.role,
            "moved": pose_json(&step),
            "total": pose_json(&plan.target),
        }),
    ))
}

/// Moves, turns or scales the tooth `p.fdi`.
fn transform_tooth(
    state: &CaseState,
    p: &TransformParams,
    delta: Option<Pose>,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let f = fdi_of(p.fdi.ok_or_else(|| invalid("give the tooth number: fdi"))?)?;
    if p.stage == Some(0) {
        return Err(invalid(
            "stage 0 is the initial position: give an aligner (1 or more), or no stage for the final position",
        ));
    }
    let data = data_of(state)?;
    // check the size first: nothing changes when it is refused
    let scale = match (p.scale, p.reset) {
        (None, false) => None,
        (factor, reset) => {
            let current = data.tooth_scale(f);
            let start = if reset || p.absolute { 1.0 } else { current };
            let target = factor.map_or(start, |k| start * k);
            if !data.crowns.contains_key(&f) {
                if factor.is_some() {
                    return Err(messages::invalid(
                        "aligners-err-tooth-not-built",
                        &[("fdi", &f.to_string())],
                    ));
                }
                None
            } else if !target.is_finite() || !(TOOTH_SCALE_MIN..=TOOTH_SCALE_MAX).contains(&target)
            {
                return Err(messages::invalid(
                    "aligners-err-tooth-scale-range",
                    &[
                        ("fdi", &f.to_string()),
                        ("min", &TOOTH_SCALE_MIN.to_string()),
                        ("max", &TOOTH_SCALE_MAX.to_string()),
                    ],
                ));
            } else {
                Some((current, target))
            }
        }
    };
    let mut out = json!({ "component": "tooth", "fdi": f.get(), "stage": p.stage });
    let mut next = state.clone();
    // the pose in the plan, as the manual adjustment stores it
    if delta.is_some() || p.reset {
        let fr = data.tooth_frames.get(&f).ok_or_else(|| {
            messages::invalid(
                "aligners-err-tooth-not-measured",
                &[("fdi", &f.to_string())],
            )
        })?;
        let rest = teeth::rest_pose(fr);
        let local = pose_in(&data, p.stage, f);
        let plan = Plan::new(rest.compose(&local).compose(&rest.inverse()), delta, p);
        let target = rest.inverse().compose(&plan.target).compose(&rest);
        let (s, report) = run_adjust(
            &next,
            AdjustParams {
                fdi: f.get(),
                stage: p.stage,
                delta: Some(PoseDelta {
                    t: target.t.to_array(),
                    q: target.q.to_array(),
                }),
                absolute: true,
                ..AdjustParams::default()
            },
            ctx,
        )?;
        next = s;
        out["total"] = pose_json(&plan.target);
        out["plan"] = report;
    }
    if let Some((current, target)) = scale {
        let mut data = data_of(&next)?;
        let mut stale = Vec::new();
        let ratio = target / current;
        if (ratio - 1.0).abs() > 1e-12
            && let Some(old) = data.crowns.get(&f).copied()
        {
            let mesh = next.blobs.require(&old)?;
            let r = next.blobs.insert(scaled_about_centre(mesh, ratio));
            data.crowns.insert(f, r);
            stale.push(old);
        }
        if (target - 1.0).abs() <= 1e-9 {
            data.components.tooth_scales.remove(&f);
        } else if let Some(crown) = data.crowns.get(&f).copied() {
            data.components.tooth_scales.insert(
                f,
                ToothScale {
                    factor: target,
                    crown,
                },
            );
        }
        let moved = data.setup.poses.contains_key(&f)
            || data.stages.iter().any(|s| s.poses.contains_key(&f));
        finish(&mut next, data, &stale);
        out["scale"] = json!(target);
        out["setup_outdated"] = json!(moved);
    }
    prune(&mut next)?;
    Ok((next, out))
}

/// Drops the corrections by hand that no longer describe the case.
fn prune(next: &mut CaseState) -> Result<(), CommandError> {
    let mut data = data_of(next)?;
    let before = data.components.clone();
    let ids: Vec<String> = next.case.scans.iter().map(|s| s.id.to_string()).collect();
    data.prune_component_edits(ids.iter().map(String::as_str));
    if data.components != before {
        finish(next, data, &[]);
    }
    Ok(())
}

/// `mesh` scaled by `ratio` about the centre of the tooth
/// ([`dca_model::tooth_centre`]; a uniform scale keeps the normals).
fn scaled_about_centre(mesh: &TriMesh, ratio: f64) -> TriMesh {
    let c = dca_model::tooth_centre(&mesh.positions);
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = (c + (p.as_dvec3() - c) * ratio).as_vec3();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::occlusion::tests::{exec, try_exec, two_demo_arches};
    use dc_case::{Fdi, ScanRole};
    use dca_model::{AlignersState, MODULE_ID};
    use glam::{DQuat, DVec3};

    fn data(s: &CaseState) -> AlignersState {
        s.module_data::<AlignersState>(MODULE_ID).unwrap().clone()
    }

    /// The two demo arches with their 3D teeth, built once for every test.
    fn built() -> CaseState {
        static BUILT: std::sync::LazyLock<CaseState> = std::sync::LazyLock::new(|| {
            exec("aligners.teeth.build", &two_demo_arches(), Value::Null).0
        });
        BUILT.clone()
    }

    fn a_lower_tooth(s: &CaseState) -> Fdi {
        *data(s).crowns.keys().find(|f| !f.is_upper()).unwrap()
    }

    fn mv(t: [f64; 3], q: DQuat) -> Value {
        json!({ "t": t, "q": q.to_array() })
    }

    fn crown(s: &CaseState, f: Fdi) -> std::sync::Arc<TriMesh> {
        s.mesh(&data(s).crowns[&f]).unwrap().clone()
    }

    fn tooth_scale(s: &CaseState, f: Fdi) -> f64 {
        data(s).tooth_scale(f)
    }

    #[test]
    fn the_command_is_registered_and_documented() {
        let spec = crate::commands::find(TRANSFORM).unwrap();
        assert_eq!(spec.schema()["type"], "object");
        assert!(spec.schema()["properties"]["move"].is_object());
    }

    #[test]
    fn a_tooth_moves_in_the_final_position_as_the_manual_adjustment_stores_it() {
        let s = built();
        let f = a_lower_tooth(&s);
        let d = Pose::new(DVec3::new(0.5, -0.25, 0.1), DQuat::from_rotation_z(0.05));
        let (next, out) = exec(
            TRANSFORM,
            &s,
            json!({ "component": "tooth", "fdi": f.get(), "move": mv(d.t.to_array(), d.q) }),
        );
        let al = data(&next);
        let rest = teeth::rest_pose(&al.tooth_frames[&f]);
        let local = al.setup.poses[&f];
        let world = rest.compose(&local).compose(&rest.inverse());
        assert!(world.approx_eq(&d, 1e-9, 1e-9), "{world:?}");
        assert!(out["plan"]["collisions"].is_array(), "{out}");
        // the same move again adds to it; absolute sets it
        let (twice, _) = exec(
            TRANSFORM,
            &next,
            json!({ "component": "tooth", "fdi": f.get(), "move": mv(d.t.to_array(), d.q) }),
        );
        let local = data(&twice).setup.poses[&f];
        let world = rest.compose(&local).compose(&rest.inverse());
        assert!(world.approx_eq(&d.compose(&d), 1e-9, 1e-9));
        let (abs, _) = exec(
            TRANSFORM,
            &twice,
            json!({ "component": "tooth", "fdi": f.get(), "absolute": true, "move": mv(d.t.to_array(), d.q) }),
        );
        let local = data(&abs).setup.poses[&f];
        assert!(
            rest.compose(&local)
                .compose(&rest.inverse())
                .approx_eq(&d, 1e-9, 1e-9)
        );
        // reset: back to rest
        let (back, _) = exec(
            TRANSFORM,
            &abs,
            json!({ "component": "tooth", "fdi": f.get(), "reset": true }),
        );
        assert!(!data(&back).setup.poses.contains_key(&f));
    }

    #[test]
    fn a_tooth_moves_at_an_aligner_and_stage_0_is_refused() {
        let mut s = built();
        let f = a_lower_tooth(&s);
        let mut al = data(&s);
        al.stages = (0..3).map(dca_model::Stage::new).collect();
        s.set_module(MODULE_ID, dc_case::ModuleState::new(al));
        let d = Pose::from_translation(DVec3::new(0.0, 0.0, 0.4));
        let (next, _) = exec(
            TRANSFORM,
            &s,
            json!({ "component": "tooth", "fdi": f.get(), "stage": 2, "move": mv(d.t.to_array(), DQuat::IDENTITY) }),
        );
        let al = data(&next);
        assert!(al.stages[2].poses.contains_key(&f));
        assert!(!al.setup.poses.contains_key(&f));
        for stage in [0, 7] {
            assert!(
                try_exec(
                    TRANSFORM,
                    &s,
                    json!({ "component": "tooth", "fdi": f.get(), "stage": stage, "move": mv([0.1, 0.0, 0.0], DQuat::IDENTITY) }),
                )
                .is_err(),
                "stage {stage}"
            );
        }
    }

    #[test]
    fn a_tooth_is_scaled_about_its_centre_and_back() {
        let s = built();
        let f = a_lower_tooth(&s);
        let before = crown(&s, f);
        let c0 = dca_model::tooth_centre(&before.positions);
        let (next, out) = exec(
            TRANSFORM,
            &s,
            json!({ "component": "tooth", "fdi": f.get(), "scale": 1.2 }),
        );
        assert_eq!(out["scale"], 1.2);
        let after = crown(&next, f);
        assert!((dca_model::tooth_centre(&after.positions) - c0).length() < 1e-4);
        let size = |m: &TriMesh| {
            let b = m.aabb().unwrap();
            (b.max - b.min).length()
        };
        assert!((size(&after) / size(&before) - 1.2).abs() < 1e-4);
        assert_eq!(tooth_scale(&next, f), 1.2);
        // the old solid is dropped
        assert!(next.blobs.get(&data(&s).crowns[&f]).is_none());
        // relative, then absolute, then reset
        let (twice, _) = exec(
            TRANSFORM,
            &next,
            json!({ "component": "tooth", "fdi": f.get(), "scale": 1.25 }),
        );
        assert!((tooth_scale(&twice, f) - 1.5).abs() < 1e-12);
        let (abs, _) = exec(
            TRANSFORM,
            &twice,
            json!({ "component": "tooth", "fdi": f.get(), "scale": 0.9, "absolute": true }),
        );
        assert!((tooth_scale(&abs, f) - 0.9).abs() < 1e-12);
        let (back, _) = exec(
            TRANSFORM,
            &abs,
            json!({ "component": "tooth", "fdi": f.get(), "reset": true }),
        );
        assert_eq!(tooth_scale(&back, f), 1.0);
        assert!(data(&back).components.tooth_scales.is_empty());
        assert!((size(&crown(&back, f)) / size(&before) - 1.0).abs() < 1e-4);
        // out of range: refused, nothing changes
        let err = try_exec(
            TRANSFORM,
            &s,
            json!({ "component": "tooth", "fdi": f.get(), "scale": 3.0 }),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
    }

    #[test]
    fn the_gingiva_and_the_scans_cannot_be_scaled() {
        let s = built();
        for params in [
            json!({ "component": "gingiva", "arch": "lower", "scale": 1.1 }),
            json!({ "component": "scan", "arch": "upper", "scale": 1.1 }),
            json!({ "component": "scan", "arch": "upper", "stage": 2, "move": mv([1.0, 0.0, 0.0], DQuat::IDENTITY) }),
        ] {
            let err = try_exec(TRANSFORM, &s, params.clone()).unwrap_err();
            assert!(matches!(err, CommandError::Invalid(_)), "{params}: {err}");
        }
        // and a command with no change is refused
        assert!(try_exec(TRANSFORM, &s, json!({ "component": "tooth", "fdi": 31 })).is_err());
        assert!(
            try_exec(
                TRANSFORM,
                &s,
                json!({ "move": mv([1.0, 0.0, 0.0], DQuat::IDENTITY) })
            )
            .is_err()
        );
    }

    #[test]
    fn the_gingiva_moves_the_whole_arch_and_reset_brings_it_back() {
        let s = built();
        let d = Pose::new(DVec3::new(0.0, 1.0, -0.5), DQuat::from_rotation_x(0.03));
        let (next, out) = exec(
            TRANSFORM,
            &s,
            json!({ "component": "gingiva", "arch": "upper", "move": mv(d.t.to_array(), d.q) }),
        );
        assert_eq!(out["component"], "gingiva");
        let (a, b) = (data(&s), data(&next));
        let scan = next.case.scan(ScanRole::Upper).unwrap();
        assert_eq!(b.components.scan_move(&scan.id.to_string()), d);
        // the scan, its labels, its 3D teeth and gingiva moved; the lower arch did not
        let p0 = s
            .mesh(&s.case.scan(ScanRole::Upper).unwrap().mesh)
            .unwrap()
            .positions[0]
            .as_dvec3();
        let p1 = next.mesh(&scan.mesh).unwrap().positions[0].as_dvec3();
        assert!((d.apply_point(p0) - p1).length() < 1e-3);
        assert_ne!(a.gingiva[&ScanRole::Upper], b.gingiva[&ScanRole::Upper]);
        assert_eq!(a.gingiva[&ScanRole::Lower], b.gingiva[&ScanRole::Lower]);
        let up = |al: &AlignersState| *al.crowns.keys().find(|f| f.is_upper()).unwrap();
        assert_ne!(a.crowns[&up(&a)], b.crowns[&up(&b)]);
        // moving an arch does not make its 3D teeth out of date
        assert!(!b.teeth_out_of_date(ScanRole::Upper));
        // the scan of the arch gives the same total; reset brings it back
        let (back, _) = exec(
            TRANSFORM,
            &next,
            json!({ "component": "scan", "arch": "upper", "reset": true }),
        );
        let p2 = back
            .mesh(&back.case.scan(ScanRole::Upper).unwrap().mesh)
            .unwrap()
            .positions[0]
            .as_dvec3();
        assert!((p2 - p0).length() < 1e-3);
        assert!(data(&back).components.scan_moves.is_empty());
    }

    #[test]
    fn a_scaled_tooth_keeps_its_size_when_its_arch_moves() {
        let s = built();
        let f = *data(&s).crowns.keys().find(|f| f.is_upper()).unwrap();
        let (scaled, _) = exec(
            TRANSFORM,
            &s,
            json!({ "component": "tooth", "fdi": f.get(), "scale": 1.1 }),
        );
        let (moved, _) = exec(
            TRANSFORM,
            &scaled,
            json!({ "component": "gingiva", "arch": "upper", "move": mv([0.0, 0.0, 1.0], DQuat::IDENTITY) }),
        );
        assert!((tooth_scale(&moved, f) - 1.1).abs() < 1e-12);
        // built again: the built size
        let (rebuilt, _) = exec(
            "aligners.teeth.build",
            &moved,
            json!({ "arch": "upper", "teeth": [f.get()] }),
        );
        assert_eq!(tooth_scale(&rebuilt, f), 1.0);
    }
}
