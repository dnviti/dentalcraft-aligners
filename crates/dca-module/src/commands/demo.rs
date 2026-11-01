//! Commands of the `demo` stage (`dca_algo::demo`).
//!
//! `aligners.demo` generates the synthetic dental arch and puts it in the
//! case as the scan of its arch, together with its ground truth in the
//! Aligners data: the seed of every tooth and the per-vertex FDI labels
//! (a mesh blob in `labels`). A scan of the same arch already in the case is
//! replaced; the other arch is kept. The world frame is reset: it belonged to
//! the previous scans.

use dc_case::{CaseState, ModuleState, Scan};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_model::{AlignersState, MODULE_ID};
use serde_json::json;

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.demo",
        label: "aligners-cmd-demo-generate",
        doc: "aligners-cmd-demo-generate-doc",
        params: dca_algo::demo::Params,
        run: run_demo,
    }]
}

fn run_demo(
    state: &CaseState,
    params: dca_algo::demo::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    // the data of the module as it is, or new ones; unknown data is not overwritten
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data)?,
    };
    let demo = dca_algo::demo::run(&params);
    ctx.check_cancelled()?;
    let role = demo.arch.role();

    let mut next = state.clone();
    // blobs of the scan and labels this one replaces
    let mut stale: Vec<_> = next.case.scan(role).map(|s| s.mesh).into_iter().collect();
    stale.extend(data.labels.remove(&role));
    next.case.scans.retain(|s| s.role != role);
    data.seeds
        .retain(|s| dca_algo::anatomy::Arch::of(s.fdi) != demo.arch);

    // the scan carries no labels (they are the ground truth, kept apart)
    let mut scan_mesh = demo.mesh.clone();
    scan_mesh.vertex_labels = None;
    let scan_ref = next.blobs.insert(scan_mesh);
    let labels_ref = next.blobs.insert(demo.mesh);
    let vertices = next.blobs.require(&scan_ref)?.vertex_count();
    let triangles = next.blobs.require(&scan_ref)?.face_count();

    // deterministic id: the same parameters always give the same case content
    let id = (u64::from(params.arch.is_upper()) | (params.seed << 1)) & 0xffff_ffff_ffff;
    let scan: Scan = serde_json::from_value(json!({
        "id": format!("de300000-0000-4000-8000-{id:012x}"),
        "role": role,
        "source_name": format!(
            "demo-{}-{:?}-seed{}",
            if params.arch.is_upper() { "upper" } else { "lower" },
            params.severity,
            params.seed
        )
        .to_lowercase(),
        "source_units": "mm",
        "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
        "mesh": scan_ref,
    }))
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    next.case.scans.push(scan);
    next.case.world_frame = Default::default();
    if next.case.patient_code.is_empty() {
        next.case.patient_code = "DEMO".to_owned();
    }

    let teeth: Vec<u8> = demo.seeds.iter().map(|s| s.fdi.get()).collect();
    data.seeds.extend(demo.seeds);
    data.labels.insert(role, labels_ref);
    next.set_module(MODULE_ID, ModuleState::new(data));
    // drop the replaced scan and labels, unless something else still uses them
    let used = next.referenced_blobs();
    let keep = next
        .blobs
        .iter()
        .map(|(r, _)| *r)
        .filter(|r| used.contains(r) || !stale.contains(r))
        .collect();
    next.blobs.retain_only(&keep);

    let out = json!({
        "arch": params.arch,
        "severity": params.severity,
        "seed": params.seed,
        "vertices": vertices,
        "triangles": triangles,
        "teeth": teeth,
        "scan_blob": scan_ref.to_hex(),
        "labels_blob": labels_ref.to_hex(),
    });
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;
    use serde_json::{Value, json};

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_defaults_make_a_lower_arch() {
        let (state, out) = run(&CaseState::default(), Value::Null).unwrap();
        assert_eq!(out["arch"], "lower");
        assert_eq!(out["severity"], "moderate");
        assert_eq!(out["seed"], 7);
        assert_eq!(out["teeth"].as_array().unwrap().len(), 14);
        assert_eq!(state.case.patient_code, "DEMO");
        assert_eq!(state.case.scans.len(), 1);
        let scan = state.case.scan(ScanRole::Lower).unwrap();
        let mesh = state.mesh(&scan.mesh).unwrap();
        assert_eq!(
            mesh.vertex_count() as u64,
            out["vertices"].as_u64().unwrap()
        );
        assert!(mesh.vertex_labels.is_none());
        mesh.check().unwrap();

        // ground truth in the Aligners data
        let data = state.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.seeds.len(), 14);
        let labels = state.mesh(&data.labels[&ScanRole::Lower]).unwrap();
        assert_eq!(labels.positions, mesh.positions);
        assert_eq!(labels.tris, mesh.tris);
        assert_eq!(
            labels.vertex_labels.as_ref().unwrap().len(),
            mesh.vertex_count()
        );
        // both meshes are in the blob store and nothing else
        assert_eq!(state.blobs.len(), 2);
    }

    #[test]
    fn it_is_deterministic() {
        let p = json!({ "arch": "upper", "severity": "severe", "seed": 42 });
        let (a, oa) = run(&CaseState::default(), p.clone()).unwrap();
        let (b, ob) = run(&CaseState::default(), p).unwrap();
        assert_eq!(oa, ob);
        assert_eq!(a.case.scans, b.case.scans);
        assert_eq!(
            a.module_data::<AlignersState>(MODULE_ID),
            b.module_data::<AlignersState>(MODULE_ID)
        );
        let (_, other) = run(
            &CaseState::default(),
            json!({ "arch": "upper", "seed": 43 }),
        )
        .unwrap();
        assert_ne!(oa["scan_blob"], other["scan_blob"]);
    }

    #[test]
    fn the_other_arch_is_kept_and_the_same_arch_replaced() {
        let (lower, _) = run(&CaseState::default(), json!({ "arch": "lower" })).unwrap();
        let (both, _) = run(&lower, json!({ "arch": "upper", "seed": 1 })).unwrap();
        assert_eq!(both.case.scans.len(), 2);
        let data = both.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.seeds.len(), 28);
        assert_eq!(data.labels.len(), 2);
        assert_eq!(both.blobs.len(), 4);
        // the lower scan is unchanged
        assert_eq!(
            both.case.scan(ScanRole::Lower),
            lower.case.scan(ScanRole::Lower)
        );

        // a new lower arch replaces the old one, with its seeds and labels
        let (again, out) = run(
            &both,
            json!({ "arch": "lower", "severity": "mild", "seed": 5 }),
        )
        .unwrap();
        assert_eq!(again.case.scans.len(), 2);
        assert_eq!(
            again
                .module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .seeds
                .len(),
            28
        );
        assert_eq!(again.blobs.len(), 4);
        assert_eq!(
            again.case.scan(ScanRole::Lower).unwrap().mesh.to_hex(),
            out["scan_blob"]
        );
        assert_ne!(
            again.case.scan(ScanRole::Lower),
            both.case.scan(ScanRole::Lower)
        );
    }

    #[test]
    fn the_world_frame_is_reset_and_unknown_data_is_kept() {
        let mut state = run(&CaseState::default(), Value::Null).unwrap().0;
        state.case.world_frame.from_case.t.x = 3.0;
        state.case.patient_code = "PZ-1".to_owned();
        let (next, _) = run(&state, json!({ "arch": "lower", "seed": 2 })).unwrap();
        assert_eq!(next.case.world_frame.from_case, dc_case::Pose::IDENTITY);
        assert_eq!(next.case.patient_code, "PZ-1");

        // module data of a newer schema is not overwritten
        let mut opaque = CaseState::default();
        opaque.set_module(
            MODULE_ID,
            ModuleState::new(dc_case::OpaqueModuleData {
                schema_version: 99,
                value: json!({ "x": 1 }),
            }),
        );
        assert!(matches!(
            run(&opaque, Value::Null),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn bad_parameters_are_rejected() {
        for p in [
            json!({ "arch": "both" }),
            json!({ "severity": "huge" }),
            json!({ "seed": -1 }),
            json!({ "nope": 1 }),
        ] {
            assert!(matches!(
                run(&CaseState::default(), p),
                Err(CommandError::Invalid(_))
            ));
        }
    }
}
