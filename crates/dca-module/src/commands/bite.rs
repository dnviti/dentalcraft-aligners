//! Commands of the `bite` stage (`dca_algo::bite`): bite registration.
//!
//! * `aligners.occlusion.bite-registration.icp` registers the arches to the
//!   buccal bite scans of the case (scans with role `bite`, one or two):
//!   automatic coarse search and trimmed point-to-plane ICP.
//! * `aligners.occlusion.bite-registration.three-points` does the same from
//!   three (or more) pairs of points picked on the first bite scan and on
//!   the arches (manual pairing), then the same fine alignment.
//!
//! The lower arch stays in place: the bite scans move onto it and the upper
//! scan (with its labels, seeds and tooth frames) moves onto the bite scans.
//! Nothing is moved when the registration is rejected (the command fails
//! with the reason). Both arches must be segmented with their seeds.

use dc_case::{CaseState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::bite::{self, BiteArch, BiteScan, ManualInit};
use glam::DVec3;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::occlusion::{aligners_data, arch_model, move_arch, move_json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.occlusion.bite-registration.icp",
            label: "aligners-menu-occlusion-bite-registration-icp",
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

fn pts(v: &[[f64; 3]]) -> Vec<DVec3> {
    v.iter().map(|p| DVec3::from_array(*p)).collect()
}

fn run_registration(
    state: &CaseState,
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
    let next = move_arch(state, data, Arch::Upper, &reg.upper_delta, &reg.bite_deltas);
    let mut out = serde_json::to_value(&reg).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["source"] = json!("bite");
    out["upper_move"] = move_json(&reg.upper_delta);
    out["bite_moves"] = reg.bite_deltas.iter().map(move_json).collect();
    Ok((next, out))
}

fn run_icp(
    state: &CaseState,
    _params: IcpParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    run_registration(state, |_, _| Ok(ManualInit::default()), ctx)
}

fn run_three_points(
    state: &CaseState,
    params: ThreePointsParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    run_registration(
        state,
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
    fn the_commands_are_menu_leaves_and_documented() {
        for id in [
            "aligners.occlusion.bite-registration.icp",
            "aligners.occlusion.bite-registration.three-points",
        ] {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object");
            assert!(crate::tree::find(id).is_some(), "{id}");
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
        // too few points
        let err = try_exec(
            "aligners.occlusion.bite-registration.three-points",
            &moved,
            Value::Null,
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("3 pairs")));
    }
}
