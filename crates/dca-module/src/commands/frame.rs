//! Commands of the `frame` stage (`dca_algo::frame`).
//!
//! `aligners.frame` computes the world frame of the case (the rigid
//! transform that puts the patient on the world axes) and stores it in
//! `case.world_frame`. The scans are not modified.

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_model::{AlignersState, MODULE_ID};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.frame",
        label: "aligners-cmd-frame-world",
        doc: "aligners-cmd-frame-world-doc",
        params: dca_algo::frame::Params,
        run: run_frame,
    }]
}

fn run_frame(
    state: &CaseState,
    params: dca_algo::frame::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let has = |role| state.case.scan(role).is_some();
    let arch = params
        .reference
        .choose(has(dc_case::ScanRole::Upper), has(dc_case::ScanRole::Lower))
        .ok_or_else(|| {
            use dca_algo::frame::Reference;
            CommandError::Invalid(
                match params.reference {
                    Reference::Auto => "the case has no upper or lower scan",
                    Reference::Upper => "the case has no upper scan",
                    Reference::Lower => "the case has no lower scan",
                }
                .to_owned(),
            )
        })?;
    let scan = state
        .case
        .scan(arch.role())
        .ok_or_else(|| CommandError::Invalid("the reference scan is missing".to_owned()))?;
    let mesh = state.blobs.require(&scan.mesh)?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let seeds = match state.module(MODULE_ID) {
        None => Vec::new(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .ok_or_else(|| CommandError::Invalid("the Aligners data cannot be read".to_owned()))?
            .seeds
            .clone(),
    };
    let input = dca_algo::frame::Input {
        arch,
        vertices: &vertices,
        seeds: &seeds,
    };
    let out =
        dca_algo::frame::run(&input, &params).map_err(|e| CommandError::Failed(e.to_string()))?;
    ctx.check_cancelled()?;
    let mut next = state.clone();
    next.case.world_frame.from_case = out.from_case;
    let json = serde_json::to_value(out).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((next, json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;
    use serde_json::{Value, json};

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find("aligners.frame").unwrap().execute(
            state,
            &params,
            &JobCtx::detached(),
        )
    }

    fn demo_state(arch: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                &CaseState::default(),
                &json!({ "arch": arch }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn an_empty_case_has_no_frame() {
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn a_requested_arch_must_exist() {
        let lower = demo_state("lower");
        let err = run(&lower, json!({ "reference": "upper" })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("upper")));
        assert!(matches!(
            run(&lower, json!({ "reference": "sideways" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_demo_gets_a_world_frame_from_its_seeds() {
        for arch in ["lower", "upper"] {
            let state = demo_state(arch);
            assert!(dca_algo::frame::is_identity(
                &state.case.world_frame.from_case
            ));
            let (next, out) = run(&state, Value::Null).unwrap();
            assert_eq!(out["source"], "seeds");
            assert_eq!(out["arch"], arch);
            let pose = next.case.world_frame.from_case;
            assert!(!dca_algo::frame::is_identity(&pose));
            assert_eq!(out["from_case"], serde_json::to_value(pose).unwrap());
            // the scans and the Aligners data are untouched
            assert_eq!(next.case.scans, state.case.scans);
            assert_eq!(
                next.module_data::<AlignersState>(MODULE_ID),
                state.module_data::<AlignersState>(MODULE_ID)
            );
            // the arch centre is the world origin; incisors in front, right at -X
            let seeds = &next.module_data::<AlignersState>(MODULE_ID).unwrap().seeds;
            let world: Vec<_> = seeds
                .iter()
                .map(|s| (s.fdi, pose.apply_point(s.position)))
                .collect();
            let n = world.len() as f64;
            let mean = world.iter().map(|(_, p)| *p).reduce(|a, b| a + b).unwrap() / n;
            assert!(mean.x.abs() < 1.0 && mean.y.abs() < 8.0, "{mean:?}");
            for (f, p) in &world {
                if f.position() >= 3 {
                    assert_eq!(p.x < 0.0, f.is_right(), "{f}");
                }
                if f.position() == 1 {
                    assert!(p.y < 0.0, "{f}");
                }
            }
            // the crowns sit on the floor: the mesh is below it for the lower arch
            // (teeth up) and above it for the upper arch (teeth down)
            let scan = next.case.scan(if arch == "lower" {
                ScanRole::Lower
            } else {
                ScanRole::Upper
            });
            let mesh = next.mesh(&scan.unwrap().mesh).unwrap();
            let mean_z = mesh
                .positions
                .iter()
                .map(|p| pose.apply_point(p.as_dvec3()).z)
                .sum::<f64>()
                / mesh.vertex_count() as f64;
            assert!(
                if arch == "lower" {
                    mean_z < -2.0
                } else {
                    mean_z > 2.0
                },
                "{mean_z}"
            );
            // a second run gives the same frame
            let (again, _) = run(&next, Value::Null).unwrap();
            assert_eq!(again.case.world_frame.from_case, pose);
        }
    }

    #[test]
    fn without_seeds_the_frame_is_estimated_from_the_mesh() {
        let mut state = demo_state("lower");
        let (with_seeds, _) = run(&state, Value::Null).unwrap();
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.seeds.clear();
        state.set_module(MODULE_ID, dc_case::ModuleState::new(data));
        let (next, out) = run(&state, Value::Null).unwrap();
        assert_eq!(out["source"], "mesh");
        // the same orientation as the seeds-based frame, the centre within a few mm
        let a = with_seeds.case.world_frame.from_case;
        let b = next.case.world_frame.from_case;
        assert!(a.q.angle_between(b.q).to_degrees() < 10.0);
        assert!((a.t - b.t).length() < 8.0, "{:?} {:?}", a.t, b.t);
    }

    #[test]
    fn the_lower_arch_is_the_reference_unless_asked_otherwise() {
        let both = crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                &demo_state("lower"),
                &json!({ "arch": "upper" }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0;
        assert!(
            both.case.scan(ScanRole::Upper).is_some() && both.case.scan(ScanRole::Lower).is_some()
        );
        let (_, out) = run(&both, Value::Null).unwrap();
        assert_eq!(out["arch"], "lower");
        let (_, out) = run(&both, json!({ "reference": "upper" })).unwrap();
        assert_eq!(out["arch"], "upper");
    }
}
