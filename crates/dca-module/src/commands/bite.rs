//! Commands of the `bite` stage (`dca_algo::bite`): bite registration and
//! its correction.
//!
//! * `aligners.occlusion.bite-registration.icp` registers the arches to the
//!   buccal bite scans of the case (scans with role `bite`, one or two):
//!   automatic coarse search and trimmed point-to-plane ICP.
//! * `aligners.occlusion.bite-registration.three-points` does the same from
//!   three (or more) pairs of points picked on the first bite scan and on
//!   the arches (manual pairing), then the same fine alignment.
//! * `aligners.occlusion.bite-correct` removes the interpenetration of the
//!   arches (`oa.bite_correct`): the upper arch moves along the occlusal
//!   axis of the lower one to the first contact.
//!
//! The lower arch stays in place: the bite scans move onto it and the upper
//! scan (with its labels, seeds and tooth frames) moves onto the bite scans.
//! Nothing is moved when the registration is rejected (the command fails
//! with the reason). Both arches must be segmented with their seeds. The
//! report of a registration is kept in the case
//! ([`AlignersState::bite_report`]), as the plugin keeps it on both cases.

use dc_case::{CaseState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::bite::{self, BiteArch, BiteScan, FitMetrics, ManualInit, Registration};
use dca_model::{
    AlignersState, BiteFit, BiteMethod, BiteRegistration, BiteReport, BiteScanFit, BiteVerdict,
};
use glam::DVec3;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::messages;
use super::occlusion::{aligners_data, arch_model, move_arch, move_json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.occlusion.bite-registration.icp",
            label: "aligners-cmd-bite-icp",
            doc: "aligners-cmd-bite-icp-doc",
            params: IcpParams,
            run: run_icp,
        },
        stage_command! {
            id: "aligners.occlusion.bite-registration.three-points",
            label: "aligners-menu-occlusion-bite-registration-three-points",
            doc: "aligners-cmd-bite-three-points-doc",
            params: ThreePointsParams,
            run: run_three_points,
        },
        stage_command! {
            id: "aligners.occlusion.bite-correct",
            label: "aligners-cmd-bite-correct",
            doc: "aligners-cmd-bite-correct-doc",
            params: CorrectParams,
            run: run_correct,
        },
    ]
}

/// Parameters of the automatic registration (none yet).
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IcpParams {}

/// Parameters of the 3-point registration: matching points on the first bite
/// scan and on the arches, in case coordinates (mm), as the scans are now.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ThreePointsParams {
    /// Points picked on the lower arch (at least 3, far apart: e.g. canine,
    /// premolar and molar).
    pub lower_points: Vec<[f64; 3]>,
    /// The matching points on the bite scan, in the same order.
    pub bite_points: Vec<[f64; 3]>,
    /// Optional: points picked on the upper arch.
    pub upper_points: Vec<[f64; 3]>,
    /// The matching points on the bite scan for the upper arch.
    pub bite_points_upper: Vec<[f64; 3]>,
}

/// Parameters of the correction of the bite (none).
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CorrectParams {}

fn pts(v: &[[f64; 3]]) -> Vec<DVec3> {
    v.iter().map(|p| DVec3::from_array(*p)).collect()
}

fn fit_of(m: &FitMetrics) -> BiteFit {
    BiteFit {
        area_mm2: m.area_mm2,
        teeth: m.teeth.clone(),
        n_teeth: m.n_teeth,
        rms_mm: m.rms_mm,
        p95_mm: m.p95_mm,
        coherence: m.coherence,
        bite_fraction: m.bite_fraction,
        icp_overlap: m.icp_overlap,
    }
}

/// The report a registration leaves in the case.
fn report_of(reg: &Registration, method: BiteMethod) -> BiteReport {
    BiteReport {
        method,
        registration: Some(BiteRegistration {
            scans: reg
                .scans
                .iter()
                .map(|s| BiteScanFit {
                    scan: s.scan.clone(),
                    side: s.side.map(str::to_owned),
                    lower: fit_of(&s.lower),
                    upper: fit_of(&s.upper),
                })
                .collect(),
            agreement_mm: reg.agreement_mm,
            penetration_mm: reg.penetration_mm,
            registered_penetration_mm: None,
            contact_points: reg.contact_points,
            opening_at_contact_mm: reg.opening_at_contact_mm,
            facing: reg.facing,
            verdict: if reg.verdict == "good" {
                BiteVerdict::Good
            } else {
                BiteVerdict::ToReview
            },
            reasons: reg.reasons.clone(),
        }),
        correction_mm: 0.0,
    }
}

fn run_registration(
    state: &CaseState,
    method: BiteMethod,
    init: impl FnOnce(&BiteArch, &BiteArch) -> Result<ManualInit, CommandError>,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let bites: Vec<BiteScan> = state
        .case
        .scans
        .iter()
        .filter(|s| s.role == ScanRole::Bite)
        .map(|s| {
            let mesh = state.blobs.require(&s.mesh)?;
            let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
            BiteScan::new(&s.source_name, &v, &mesh.tris)
                .map_err(|e| CommandError::Invalid(e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    if bites.is_empty() {
        return Err(CommandError::Invalid(
            "the case has no bite scan: import one or two buccal bite scans".to_owned(),
        ));
    }
    let lower = BiteArch::new(arch_model(state, &data, Arch::Lower)?)
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    let upper = BiteArch::new(arch_model(state, &data, Arch::Upper)?)
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    let init = init(&lower, &upper)?;
    ctx.check_cancelled()?;
    let reg = bite::register(&lower, &upper, &bites, &init)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    ctx.check_cancelled()?;
    data.bite_registered = true;
    data.bite_report = Some(report_of(&reg, method));
    let next = move_arch(state, data, Arch::Upper, &reg.upper_delta, &reg.bite_deltas);
    let mut out = serde_json::to_value(&reg).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["source"] = json!("bite");
    out["method"] = json!(method);
    out["upper_move"] = move_json(&reg.upper_delta);
    out["bite_moves"] = reg.bite_deltas.iter().map(move_json).collect();
    Ok((next, out))
}

fn run_icp(
    state: &CaseState,
    _params: IcpParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    run_registration(
        state,
        BiteMethod::Registration,
        |_, _| Ok(ManualInit::default()),
        ctx,
    )
}

fn run_three_points(
    state: &CaseState,
    params: ThreePointsParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    run_registration(
        state,
        BiteMethod::ThreePoints,
        |lower, upper| {
            let invalid = |e: bite::BiteError| CommandError::Invalid(e.to_string());
            let l = bite::kabsch_init(
                &lower.model.to_case,
                &pts(&params.lower_points),
                &pts(&params.bite_points),
            )
            .map_err(invalid)?;
            let u = if params.upper_points.is_empty() && params.bite_points_upper.is_empty() {
                None
            } else {
                Some(
                    bite::kabsch_init(
                        &upper.model.to_case,
                        &pts(&params.upper_points),
                        &pts(&params.bite_points_upper),
                    )
                    .map_err(invalid)?,
                )
            };
            Ok(ManualInit {
                lower: Some(l),
                upper: u,
            })
        },
        ctx,
    )
}

/// The report after a correction (`bite.correct_penetration`): the opening
/// is added up, and a registration keeps the interpenetration it gave, takes
/// the one left and drops the reason about it.
fn corrected(report: &mut BiteReport, c: &bite::Correction) {
    report.correction_mm = round3(report.correction_mm + c.opening_mm);
    if let Some(r) = &mut report.registration {
        r.registered_penetration_mm.get_or_insert(r.penetration_mm);
        r.penetration_mm = c.penetration_mm;
        r.contact_points = c.contact_points;
        r.reasons.retain(|w| w != "interpenetration");
        r.update_verdict();
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn run_correct(
    state: &CaseState,
    _params: CorrectParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data: AlignersState = aligners_data(state)?;
    let lower = arch_model(state, &data, Arch::Lower)?;
    let upper = arch_model(state, &data, Arch::Upper)?;
    ctx.check_cancelled()?;
    let c = bite::correct_penetration(&lower, &upper)
        .map_err(|e| CommandError::Invalid(e.to_string()))?
        .ok_or_else(|| messages::invalid("aligners-err-bite-no-penetration", &[]))?;
    ctx.check_cancelled()?;
    if let Some(report) = &mut data.bite_report {
        corrected(report, &c);
    }
    let next = move_arch(state, data, Arch::Upper, &c.upper_delta, &[]);
    let mut out = serde_json::to_value(&c).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["upper_move"] = move_json(&c.upper_delta);
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::occlusion::tests::{exec, try_exec, two_demo_arches};
    use dc_case::{Scan, ScanRole};
    use dc_mesh::TriMesh;
    use dca_model::{AlignersState, MODULE_ID};
    use glam::DMat4;
    use serde_json::Value;

    /// The demo arches in virtual occlusion, a synthetic buccal bite scan of
    /// each side imported in the scanner's coordinates (`x`), then the upper
    /// arch displaced by `y`.
    fn bite_case(x: DMat4, y: DMat4) -> (CaseState, CaseState) {
        let state = two_demo_arches();
        let closed = exec("aligners.occlusion.virtual-bite", &state, Value::Null).0;
        let data = closed
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        let lower = arch_model(&closed, &data, Arch::Lower).unwrap();
        let upper = arch_model(&closed, &data, Arch::Upper).unwrap();
        let mut with_bites = closed.clone();
        for (k, right) in [true, false].into_iter().enumerate() {
            let (v, t) = bite::demo_bite_scan(&lower, &upper, right);
            let mesh = TriMesh::new(
                v.iter().map(|&p| x.transform_point3(p).as_vec3()).collect(),
                t,
            );
            let blob = with_bites.blobs.insert(mesh);
            let scan: Scan = serde_json::from_value(json!({
                "id": format!("b1700000-0000-4000-8000-00000000000{k}"),
                "role": "bite",
                "source_name": if right { "bite-right.stl" } else { "bite-left.stl" },
                "source_units": "mm",
                "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
                "mesh": blob,
            }))
            .unwrap();
            with_bites.case.scans.push(scan);
        }
        let moved = move_arch(&with_bites, data, Arch::Upper, &y, &[]);
        (closed, moved)
    }

    fn upper_positions(s: &CaseState) -> Vec<glam::Vec3> {
        s.mesh(&s.case.scan(ScanRole::Upper).unwrap().mesh)
            .unwrap()
            .positions
            .clone()
    }

    fn max_gap(a: &[glam::Vec3], b: &[glam::Vec3]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(p, q)| p.distance(*q))
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_commands_are_registered_and_documented() {
        for id in [
            "aligners.occlusion.bite-registration.icp",
            "aligners.occlusion.bite-registration.three-points",
            "aligners.occlusion.bite-correct",
        ] {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object");
            assert!(crate::commands::is_registered(id), "{id}");
        }
    }

    #[test]
    fn a_case_without_bite_scans_cannot_be_registered() {
        let state = two_demo_arches();
        let err = try_exec(
            "aligners.occlusion.bite-registration.icp",
            &state,
            Value::Null,
        )
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("bite scan")),
            "{err}"
        );
    }

    #[test]
    fn the_bite_leaf_uses_the_bite_scans_when_the_case_has_them() {
        let x = DMat4::from_translation(DVec3::new(10.0, -4.0, 18.0))
            * DMat4::from_axis_angle(DVec3::new(0.2, 1.0, -0.3).normalize(), 0.7);
        let y = DMat4::from_translation(DVec3::new(2.0, -3.0, 5.0))
            * DMat4::from_axis_angle(DVec3::new(1.0, 0.0, 0.3).normalize(), 0.2);
        let (closed, moved) = bite_case(x, y);
        let (next, out) = exec("aligners.occlusion.align", &moved, Value::Null);
        assert_eq!(out["method"], "bite_scan");
        let ids: Vec<_> = out["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "aligners.occlusion.bite-registration.icp",
                "aligners.occlusion.contact-map"
            ]
        );
        let gap = max_gap(&upper_positions(&closed), &upper_positions(&next));
        assert!(gap < 0.05, "{gap}");
    }

    #[test]
    fn the_registration_puts_the_upper_arch_back_on_the_bite() {
        let x = DMat4::from_translation(DVec3::new(10.0, -4.0, 18.0))
            * DMat4::from_axis_angle(DVec3::new(0.2, 1.0, -0.3).normalize(), 0.7);
        let y = DMat4::from_translation(DVec3::new(2.0, -3.0, 5.0))
            * DMat4::from_axis_angle(DVec3::new(1.0, 0.0, 0.3).normalize(), 0.2);
        let (closed, moved) = bite_case(x, y);
        assert!(max_gap(&upper_positions(&closed), &upper_positions(&moved)) > 3.0);
        let (next, out) = exec(
            "aligners.occlusion.bite-registration.icp",
            &moved,
            Value::Null,
        );
        assert_eq!(out["source"], "bite");
        assert_eq!(out["scans"].as_array().unwrap().len(), 2);
        assert!(out["agreement_mm"].as_f64().unwrap() < 0.05, "{out}");
        // the upper arch is back in the relation of the bite (within 0.05 mm)
        let gap = max_gap(&upper_positions(&closed), &upper_positions(&next));
        assert!(gap < 0.05, "{gap}");
        // the labels and seeds followed
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let labels = next.mesh(&data.labels[&ScanRole::Upper]).unwrap();
        assert_eq!(labels.positions, upper_positions(&next));
        // and the case remembers it: replacing an arch will say so
        assert!(data.bite_registered);
        // the bite scans now lie on the arches
        let bites: Vec<_> = next
            .case
            .scans
            .iter()
            .filter(|s| s.role == ScanRole::Bite)
            .collect();
        assert_eq!(bites.len(), 2);
        let first = next.mesh(&bites[0].mesh).unwrap();
        let original = closed
            .mesh(&closed.case.scan(ScanRole::Lower).unwrap().mesh)
            .unwrap();
        let lower_box = original.aabb().unwrap();
        assert!(
            first
                .positions
                .iter()
                .all(|p| p.distance(lower_box.center()) < 60.0)
        );

        // the 3-point registration from picked points gives the same result
        let data = moved.module_data::<AlignersState>(MODULE_ID).unwrap();
        let lower_seeds: Vec<DVec3> = data
            .seeds
            .iter()
            .filter(|s| Arch::of(s.fdi) == Arch::Lower && s.fdi.is_right())
            .map(|s| s.position)
            .collect();
        let picks: Vec<[f64; 3]> = [0, 3, 6]
            .iter()
            .map(|&i| lower_seeds[i].to_array())
            .collect();
        let on_bite: Vec<[f64; 3]> = [0, 3, 6]
            .iter()
            .map(|&i| x.transform_point3(lower_seeds[i]).to_array())
            .collect();
        let (next3, _) = exec(
            "aligners.occlusion.bite-registration.three-points",
            &moved,
            json!({ "lower_points": picks, "bite_points": on_bite }),
        );
        let gap3 = max_gap(&upper_positions(&closed), &upper_positions(&next3));
        assert!(gap3 < 0.05, "{gap3}");
        let report = next3
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .bite_report
            .clone()
            .unwrap();
        assert_eq!(report.method, BiteMethod::ThreePoints);
        // too few points
        let err = try_exec(
            "aligners.occlusion.bite-registration.three-points",
            &moved,
            Value::Null,
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("3 pairs")));
    }

    fn report(s: &CaseState) -> Option<BiteReport> {
        s.module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .bite_report
            .clone()
    }

    #[test]
    fn the_report_of_a_registration_is_kept_in_the_case() {
        let x = DMat4::from_translation(DVec3::new(10.0, -4.0, 18.0))
            * DMat4::from_axis_angle(DVec3::new(0.2, 1.0, -0.3).normalize(), 0.7);
        let y = DMat4::from_translation(DVec3::new(2.0, -3.0, 5.0))
            * DMat4::from_axis_angle(DVec3::new(1.0, 0.0, 0.3).normalize(), 0.2);
        let (closed, moved) = bite_case(x, y);
        // the virtual bite of `bite_case` said it is an estimate
        assert_eq!(report(&closed), Some(BiteReport::virtual_bite()));
        let (next, out) = exec(
            "aligners.occlusion.bite-registration.icp",
            &moved,
            Value::Null,
        );
        assert_eq!(out["method"], "registration");
        let rep = report(&next).unwrap();
        assert_eq!(rep.method, BiteMethod::Registration);
        assert_eq!(rep.correction_mm, 0.0);
        let r = rep.registration.as_ref().unwrap();
        // the plugin's fields, as the command reported them
        assert_eq!(r.scans.len(), 2);
        assert_eq!(r.scans[0].scan, "bite-right.stl");
        assert_eq!(r.scans[0].side.as_deref(), Some("right"));
        assert_eq!(r.scans[1].side.as_deref(), Some("left"));
        assert_eq!(
            r.scans[0].lower.rms_mm,
            out["scans"][0]["lower"]["rms_mm"].as_f64().unwrap()
        );
        assert_eq!(
            r.scans[1].upper.n_teeth as u64,
            out["scans"][1]["upper"]["n_teeth"].as_u64().unwrap()
        );
        assert_eq!(r.agreement_mm, out["agreement_mm"].as_f64());
        assert_eq!(r.penetration_mm, out["penetration_mm"].as_f64().unwrap());
        assert_eq!(
            r.contact_points as u64,
            out["contact_points"].as_u64().unwrap()
        );
        assert_eq!(
            r.opening_at_contact_mm,
            out["opening_at_contact_mm"].as_f64().unwrap()
        );
        assert_eq!(r.facing, out["facing"].as_f64().unwrap());
        let verdict = serde_json::to_value(r.verdict).unwrap();
        assert_eq!(verdict, out["verdict"]);
        assert_eq!(json!(r.reasons), out["reasons"]);
        assert!(r.max_rms_mm() < 0.02, "{r:?}");
        // a reopened project has it (the module state goes through JSON)
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let back: AlignersState =
            serde_json::from_value(serde_json::to_value(data).unwrap()).unwrap();
        assert_eq!(back.bite_report, Some(rep));
        // replacing an arch forgets it with the registration
        let mut d = data.clone();
        assert_eq!(d.clear_arch(ScanRole::Upper)[0], "bite_registered");
        assert!(d.bite_report.is_none() && !d.bite_registered);
    }

    /// The demo arches in virtual occlusion with the upper arch pushed
    /// `depth` mm into the lower one along the occlusal axis, and a stored
    /// registration report that says so.
    fn pushed(depth: f64) -> CaseState {
        let closed = exec(
            "aligners.occlusion.virtual-bite",
            &two_demo_arches(),
            Value::Null,
        )
        .0;
        let mut data = closed
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        let lower = arch_model(&closed, &data, Arch::Lower).unwrap();
        let down = DMat4::from_translation(-lower.to_case.z_axis.truncate() * depth);
        data.bite_report = Some(BiteReport {
            method: BiteMethod::Registration,
            registration: Some(BiteRegistration {
                penetration_mm: depth,
                reasons: vec!["interpenetration".into()],
                verdict: BiteVerdict::ToReview,
                ..BiteRegistration::default()
            }),
            correction_mm: 0.0,
        });
        move_arch(&closed, data, Arch::Upper, &down, &[])
    }

    fn first_contact(s: &CaseState) -> f64 {
        let data = s.module_data::<AlignersState>(MODULE_ID).unwrap();
        let lower = arch_model(s, data, Arch::Lower).unwrap();
        let upper = arch_model(s, data, Arch::Upper).unwrap();
        dca_algo::occlusion::first_contact(&lower, &upper)
            .unwrap()
            .dz
    }

    #[test]
    fn the_correction_lifts_the_upper_arch_to_the_first_contact() {
        let state = pushed(0.4);
        let before = first_contact(&state);
        assert!(before > 0.3, "{before}");
        let (next, out) = exec("aligners.occlusion.bite-correct", &state, Value::Null);
        let opening = out["opening_mm"].as_f64().unwrap();
        assert!((opening - before).abs() < 1e-9, "{out}");
        // the arches touch: the plugin keeps 0.02 mm to the first contact
        let after = first_contact(&next);
        assert!(after.abs() <= bite::CORRECT_MIN_MM, "{after}");
        assert!(out["penetration_mm"].as_f64().unwrap() < 0.05, "{out}");
        assert!(out["contact_points"].as_u64().unwrap() > 0, "{out}");
        // a pure lift along the occlusal axis of the lower arch
        let rot = out["upper_move"]["rotation_deg"].as_f64().unwrap();
        assert!(rot.abs() < 1e-9, "{out}");
        // the lower arch stays
        assert_eq!(
            state.case.scan(ScanRole::Lower).unwrap().mesh,
            next.case.scan(ScanRole::Lower).unwrap().mesh
        );
        // the report: the opening, the interpenetration it had, the one left
        let rep = report(&next).unwrap();
        assert_eq!(rep.correction_mm, round3(opening));
        let r = rep.registration.unwrap();
        assert_eq!(r.registered_penetration_mm, Some(0.4));
        assert_eq!(r.penetration_mm, out["penetration_mm"].as_f64().unwrap());
        assert!(r.reasons.is_empty());
        assert_eq!(r.verdict, BiteVerdict::Good);
        // nothing left to correct: refused, with the reason
        let err = try_exec("aligners.occlusion.bite-correct", &next, Value::Null).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("interpenetrate")),
            "{err}"
        );
        // the virtual bite of the demo arches does not interpenetrate either
        let closed = exec(
            "aligners.occlusion.virtual-bite",
            &two_demo_arches(),
            Value::Null,
        )
        .0;
        assert!(try_exec("aligners.occlusion.bite-correct", &closed, Value::Null).is_err());
    }

    #[test]
    fn the_correction_needs_both_arches() {
        let lower = exec("aligners.demo", &CaseState::default(), Value::Null).0;
        let err = try_exec("aligners.occlusion.bite-correct", &lower, Value::Null).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("upper")),
            "{err}"
        );
        assert!(matches!(
            try_exec(
                "aligners.occlusion.bite-correct",
                &lower,
                json!({ "nope": 1 })
            ),
            Err(CommandError::Invalid(_))
        ));
    }
}
