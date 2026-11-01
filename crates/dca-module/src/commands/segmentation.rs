//! Commands of the `segmentation` stage (`dca_algo::segmentation`).
//!
//! `aligners.preparation.segmentation.automatic` segments the scan of every
//! arch that has tooth seeds (or the one arch asked for): the scan is moved
//! to the arch frame its seeds define, simplified to about 30 000 faces,
//! partitioned by a random walk from the seeds, the labels are transferred
//! back to the full scan, cleaned and their boundaries refined at full
//! resolution. The per-vertex FDI labels (`0` = gingiva) are stored as the
//! arch's labels blob in the Aligners data (the scan mesh with vertex
//! labels, as `aligners.demo` stores its ground truth), replacing the
//! previous labels of that arch. The scans and the seeds are not modified.

use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::segmentation::{self, SegmentationError};
use dca_model::{AlignersState, MODULE_ID};
use serde_json::{Map, Value, json};

use super::import::require_settled;

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.preparation.segmentation.automatic",
        label: "aligners-cmd-segmentation-automatic",
        doc: "aligners-cmd-segmentation-automatic-doc",
        params: dca_algo::segmentation::Params,
        run: run_automatic,
    }]
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn run_automatic(
    state: &CaseState,
    params: segmentation::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(|| CommandError::Invalid("the Aligners data cannot be read".to_owned()))?,
    };
    let has_seeds = |arch: Arch| data.seeds.iter().any(|s| Arch::of(s.fdi) == arch);
    let arches: Vec<Arch> = match params.arch {
        Some(arch) => {
            if state.case.scan(arch.role()).is_none() {
                return Err(CommandError::Invalid(format!(
                    "the case has no {} scan",
                    arch_name(arch)
                )));
            }
            if !has_seeds(arch) {
                return Err(CommandError::Invalid(format!(
                    "the {} arch has no tooth seeds: place or propose the seeds first",
                    arch_name(arch)
                )));
            }
            vec![arch]
        }
        None => [Arch::Upper, Arch::Lower]
            .into_iter()
            .filter(|&a| state.case.scan(a.role()).is_some() && has_seeds(a))
            .collect(),
    };
    if arches.is_empty() {
        return Err(CommandError::Invalid(
            "no arch has a scan with tooth seeds: place or propose the seeds first".to_owned(),
        ));
    }

    // the gate of every later stage: nothing is computed on a scan whose arch
    // is still a guess
    for scan in arches.iter().filter_map(|a| state.case.scan(a.role())) {
        require_settled(scan)?;
    }

    let mut next = state.clone();
    let mut stale = Vec::new();
    let mut report = Map::new();
    for arch in arches {
        ctx.check_cancelled()?;
        let role = arch.role();
        let scan = state.case.scan(role).ok_or_else(|| {
            CommandError::Invalid(format!("the {} scan is missing", arch_name(arch)))
        })?;
        let mesh = state.blobs.require(&scan.mesh)?;
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        // the arch frame its seeds define (case_teeth.segment_iter works in arch coordinates)
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
        let vertices: Vec<_> = case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect();
        let mut seeds: Vec<(Fdi, _)> = data
            .seeds
            .iter()
            .filter(|s| Arch::of(s.fdi) == arch)
            .map(|s| (s.fdi, to_arch.transform_point3(s.position)))
            .collect();
        seeds.sort_by_key(|s| s.0);
        let out = segmentation::run(
            &segmentation::Input {
                arch,
                vertices: &vertices,
                tris: &mesh.tris,
                seeds: &seeds,
                decimated: None,
            },
            &params,
        )
        .map_err(|e| match e {
            SegmentationError::Invalid(_) | SegmentationError::NoSeeds => {
                CommandError::Invalid(format!("{} arch: {e}", arch_name(arch)))
            }
            SegmentationError::Decimate(_) => {
                CommandError::Failed(format!("{} arch: {e}", arch_name(arch)))
            }
        })?;
        ctx.check_cancelled()?;

        // the labels blob: the scan mesh with one FDI per vertex
        let mut labelled = (**mesh).clone();
        labelled.vertex_labels = Some(out.labels.clone());
        labelled.face_labels = None;
        let labels_ref = next.blobs.insert(labelled);
        if let Some(old) = data.labels.insert(role, labels_ref)
            && old != labels_ref
        {
            stale.push(old);
        }
        let mut per_tooth = Map::new();
        for &l in &out.labels {
            let key = l.to_string();
            let n = per_tooth.get(&key).and_then(Value::as_u64).unwrap_or(0);
            per_tooth.insert(key, json!(n + 1));
        }
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "frame_source": framed.source,
                "vertices": out.labels.len(),
                "simplified_faces": out.decimated_tris.len(),
                "teeth": out.teeth,
                "vertices_per_label": per_tooth,
                "labels_blob": labels_ref.to_hex(),
                "diagnostics": out.diagnostics,
            }),
        );
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    // drop the replaced labels, unless something else still uses them
    let used = next.referenced_blobs();
    let keep = next
        .blobs
        .iter()
        .map(|(r, _)| *r)
        .filter(|r| used.contains(r) || !stale.contains(r))
        .collect();
    next.blobs.retain_only(&keep);
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;

    const ID: &str = "aligners.preparation.segmentation.automatic";

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(ID)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo_state(arch: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                &CaseState::default(),
                &json!({ "arch": arch, "severity": "mild" }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn the_command_is_the_automatic_segmentation_leaf() {
        let spec = crate::commands::find(ID).unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["arch"].is_object());
        assert!(crate::tree::find(ID).is_some());
    }

    #[test]
    fn missing_scans_or_seeds_are_reported() {
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("seeds")
        ));
        let lower = demo_state("lower");
        assert!(matches!(
            run(&lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper scan")
        ));
        let mut no_seeds = lower.clone();
        let mut data = no_seeds
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.seeds.clear();
        no_seeds.set_module(MODULE_ID, ModuleState::new(data));
        assert!(matches!(
            run(&no_seeds, json!({ "arch": "lower" })),
            Err(CommandError::Invalid(m)) if m.contains("seeds")
        ));
        assert!(matches!(
            run(&lower, json!({ "arch": "sideways" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn a_provisional_scan_is_refused_whatever_arch_is_asked() {
        let mut state = demo_state("lower");
        state.case.scans[0].role_source = dc_case::RoleSource::Provisional;
        for params in [json!({ "arch": "lower" }), Value::Null] {
            let Err(CommandError::Invalid(m)) = run(&state, params) else {
                panic!("a provisional scan was segmented");
            };
            assert!(m.contains("Import scans panel"), "{m}");
        }
    }

    #[test]
    fn the_demo_arch_is_segmented_like_its_ground_truth() {
        let demo = demo_state("lower");
        let data = demo.module_data::<AlignersState>(MODULE_ID).unwrap();
        let truth_ref = data.labels[&ScanRole::Lower];
        let truth = demo
            .mesh(&truth_ref)
            .unwrap()
            .vertex_labels
            .clone()
            .unwrap();
        // previous labels of the arch (all gingiva): replaced and dropped
        let mut state = demo.clone();
        let mut data = data.clone();
        let mut old = (**demo.mesh(&truth_ref).unwrap()).clone();
        old.vertex_labels = Some(vec![0; truth.len()]);
        let old_ref = state.blobs.insert(old);
        data.labels.insert(ScanRole::Lower, old_ref);
        state.set_module(MODULE_ID, ModuleState::new(data));
        state.blobs.retain_only(&state.referenced_blobs());

        let (next, out) = run(&state, Value::Null).unwrap();
        let report = &out["lower"];
        assert_eq!(report["teeth"], 14);
        assert!(out.get("upper").is_none());
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let labels_ref = data.labels[&ScanRole::Lower];
        assert_eq!(report["labels_blob"], labels_ref.to_hex());
        let labelled = next.mesh(&labels_ref).unwrap();
        let scan = next.case.scan(ScanRole::Lower).unwrap();
        let mesh = next.mesh(&scan.mesh).unwrap();
        assert_eq!(labelled.positions, mesh.positions);
        assert_eq!(labelled.tris, mesh.tris);
        let labels = labelled.vertex_labels.as_ref().unwrap();
        let same = labels.iter().zip(&truth).filter(|(a, b)| a == b).count();
        let agreement = same as f64 / truth.len() as f64;
        assert!(
            agreement > 0.98,
            "agreement with the ground truth {agreement}"
        );
        // the scan and the seeds are untouched, the old labels are gone
        assert_eq!(next.case.scans, state.case.scans);
        assert_eq!(
            data.seeds,
            state.module_data::<AlignersState>(MODULE_ID).unwrap().seeds
        );
        assert!(next.blobs.get(&old_ref).is_none());
        assert_eq!(next.blobs.len(), 2);

        // deterministic: a second run stores the same labels
        let (again, again_out) = run(&next, json!({ "arch": "lower" })).unwrap();
        assert_eq!(again_out, out);
        assert_eq!(
            again
                .module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .labels,
            data.labels
        );
        assert_eq!(again.blobs.len(), 2);
    }
}
