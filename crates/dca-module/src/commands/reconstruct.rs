//! Commands of the `reconstruct` stage (`dca_algo::reconstruct`).
//!
//! `aligners.preparation.reconstruction.automatic` closes every segmented
//! tooth of every arch (or the one arch asked for): it measures the teeth
//! first (`aligners.teeth.frames`, stored as the tooth frames), moves the
//! scan to the arch frame its seeds define and closes each crown along its
//! long axis into a watertight solid. The solids are stored as blobs in
//! `AlignersState::crowns` (case coordinates), replacing the previous ones of
//! that arch; the numbers of every tooth are returned.
//!
//! `aligners.preparation.reconstruction.rebuild-tooth` closes one tooth again
//! (parameter `fdi`), with its stored frame (measured first when missing).

use std::collections::BTreeMap;

use super::messages;
use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::reconstruct::{self, ClosedTooth, Input};
use dca_algo::teeth::Which;
use dca_model::{AlignersState, MODULE_ID};
use glam::{DMat4, DVec3};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.preparation.reconstruction.automatic",
            label: "aligners-cmd-reconstruction-automatic",
            doc: "aligners-cmd-reconstruction-automatic-doc",
            params: dca_algo::reconstruct::Params,
            run: run_automatic,
        },
        stage_command! {
            id: "aligners.preparation.reconstruction.rebuild-tooth",
            label: "aligners-cmd-reconstruction-rebuild-tooth",
            doc: "aligners-cmd-reconstruction-rebuild-tooth-doc",
            params: dca_algo::reconstruct::ToothParams,
            run: run_rebuild_tooth,
        },
    ]
}

pub(crate) fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

pub(crate) fn which_name(which: Which) -> &'static str {
    match which {
        Which::All => "all",
        Which::Upper => "upper",
        Which::Lower => "lower",
    }
}

/// The Aligners data of the case (the defaults when there is none).
pub(crate) fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

/// A segmented arch scan in arch coordinates.
pub(crate) struct ArchScan {
    /// Arch frame (arch to case coordinates).
    pub to_case: DMat4,
    /// Inverse of `to_case`.
    pub to_arch: DMat4,
    /// Vertices in arch coordinates.
    pub vertices: Vec<DVec3>,
    /// Triangles.
    pub tris: Vec<[u32; 3]>,
    /// FDI label of every vertex.
    pub labels: Vec<u16>,
}

/// The segmented scan of `arch` in its arch frame (the frame its seeds
/// define, as the teeth stage does); `None` when the arch has no scan or no
/// labels.
pub(crate) fn arch_scan(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<Option<ArchScan>, CommandError> {
    let role = arch.role();
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
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
    let case_vertices: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
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
    Ok(Some(ArchScan {
        to_case,
        to_arch,
        vertices: case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect(),
        tris: mesh.tris.clone(),
        labels: labels.clone(),
    }))
}

/// Long axis of every labelled tooth of the arch with a stored frame, in
/// arch coordinates.
fn axes(data: &AlignersState, scan: &ArchScan, arch: Arch) -> BTreeMap<Fdi, DVec3> {
    let present: std::collections::BTreeSet<u16> = scan.labels.iter().copied().collect();
    data.tooth_frames
        .iter()
        .filter(|(f, _)| Arch::of(**f) == arch && present.contains(&u16::from(f.get())))
        .map(|(f, fr)| {
            let ax = fr.pose.apply_vector(DVec3::Z);
            (*f, scan.to_arch.transform_vector3(ax))
        })
        .collect()
}

/// The closed tooth as a mesh in case coordinates.
fn crown_mesh(tooth: &ClosedTooth, to_case: &DMat4) -> TriMesh {
    TriMesh::new(
        tooth
            .vertices
            .iter()
            .map(|&p| to_case.transform_point3(p).as_vec3())
            .collect(),
        tooth.tris.clone(),
    )
}

fn report(tooth: &ClosedTooth) -> Result<Value, CommandError> {
    let mut v =
        serde_json::to_value(tooth.info).map_err(|e| CommandError::Failed(e.to_string()))?;
    if let Value::Object(m) = &mut v {
        m.insert("vertices".to_owned(), json!(tooth.vertices.len()));
        m.insert("faces".to_owned(), json!(tooth.tris.len()));
    }
    Ok(v)
}

/// Measures the teeth of the selected arches (`aligners.teeth.frames`).
fn measured(state: &CaseState, which: Which, ctx: &JobCtx) -> Result<CaseState, CommandError> {
    let spec = crate::commands::find("aligners.teeth.frames")
        .ok_or_else(|| CommandError::Failed("the teeth stage is not available".to_owned()))?;
    Ok(spec
        .execute(state, &json!({ "arch": which_name(which) }), ctx)?
        .0)
}

fn run_automatic(
    state: &CaseState,
    params: reconstruct::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut next = measured(state, params.arch, ctx)?;
    let mut data = aligners_data(&next)?;
    let mut out = Map::new();
    let fraction = super::auto::step_fraction(ctx);
    let arches = params.arch.arches();
    for (k, &arch) in arches.iter().enumerate() {
        let Some(scan) = arch_scan(&next, &data, arch)? else {
            continue;
        };
        let axes = axes(&data, &scan, arch);
        let t0 = std::time::Instant::now();
        // the bar moves with every tooth closed, inside the part of this arch
        let (k, n) = (k as f64, arches.len() as f64);
        let progress = |done: usize, total: usize| {
            let x = (k + done as f64 / total.max(1) as f64) / n;
            ctx.set_fraction(fraction(x));
        };
        let closed = reconstruct::run_with_progress(
            &Input {
                vertices: &scan.vertices,
                tris: &scan.tris,
                labels: &scan.labels,
                axes: &axes,
            },
            &params,
            &progress,
            &|| ctx.is_cancelled(),
        )
        .map_err(|e| match e {
            reconstruct::ReconstructError::Cancelled => CommandError::Cancelled,
            e => CommandError::Failed(format!("{} arch: {e}", arch_name(arch))),
        })?;
        ctx.check_cancelled()?;
        data.crowns
            .retain(|f, _| Arch::of(*f) != arch || closed.teeth.contains_key(f));
        let mut teeth = Map::new();
        for (f, tooth) in &closed.teeth {
            let blob = next.blobs.insert(crown_mesh(tooth, &scan.to_case));
            data.record_crown(*f, blob, tooth.info.crown_vertices);
            teeth.insert(f.to_string(), report(tooth)?);
        }
        data.record_teeth_basis(arch.role());
        out.insert(
            arch_name(arch).to_owned(),
            json!({ "teeth": teeth, "seconds": t0.elapsed().as_secs_f64() }),
        );
    }
    if out.is_empty() {
        return Err(crate::commands::messages::none_segmented());
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(out)))
}

fn run_rebuild_tooth(
    state: &CaseState,
    params: reconstruct::ToothParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let fdi = Fdi::new(params.fdi)
        .map_err(|_| CommandError::Invalid(format!("{} is not a tooth number", params.fdi)))?;
    let arch = Arch::of(fdi);
    let which = if arch.is_upper() {
        Which::Upper
    } else {
        Which::Lower
    };
    let data = aligners_data(state)?;
    let mut next = if data.tooth_frames.contains_key(&fdi) {
        state.clone()
    } else {
        measured(state, which, ctx)?
    };
    let mut data = aligners_data(&next)?;
    let scan = arch_scan(&next, &data, arch)?
        .ok_or_else(|| crate::commands::messages::not_segmented(arch))?;
    let axis = axes(&data, &scan, arch).get(&fdi).copied().ok_or_else(|| {
        messages::invalid(
            "aligners-err-tooth-not-separated",
            &[("fdi", &fdi.to_string())],
        )
    })?;
    let one = BTreeMap::from([(fdi, axis)]);
    let input = Input {
        vertices: &scan.vertices,
        tris: &scan.tris,
        labels: &scan.labels,
        axes: &one,
    };
    let tooth = reconstruct::closed_tooth(&input, fdi, axis, &params.params())
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    ctx.check_cancelled()?;
    let blob = next.blobs.insert(crown_mesh(&tooth, &scan.to_case));
    data.record_crown(fdi, blob, tooth.info.crown_vertices);
    next.set_module(MODULE_ID, ModuleState::new(data));
    let out = json!({ "fdi": fdi.get(), "tooth": report(&tooth)? });
    Ok((next, out))
}

/// Runs `id` on `state` and cancels it as soon as it reports progress (a
/// tooth closed, a stage of the gingiva done).
#[cfg(test)]
pub(crate) fn run_cancelled_at_first_progress(
    id: &str,
    state: &CaseState,
    params: Value,
) -> Result<CommandOutput, CommandError> {
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    let ctx = JobCtx::detached();
    let finished = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !finished.load(Relaxed) {
                if ctx.progress().fraction > 0.0 {
                    ctx.cancel_token().cancel();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        });
        let r = crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &ctx);
        finished.store(true, Relaxed);
        r
    })
}

/// A case built from a golden fixture (`golden/fixtures/<name>/`): the
/// demo case of the same arch with its scan replaced by the fixture's scan
/// (a connected, voxel-remeshed surface), the oracle's segmentation labels
/// and the ground-truth seeds. For the tests of the stages that need real
/// gingival margins.
#[cfg(test)]
pub(crate) fn golden_state(name: &str) -> CaseState {
    use dc_case::ScanRole;
    use dca_model::Seed;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/fixtures")
        .join(name);
    let read = |p: &str| std::fs::read(dir.join(p)).unwrap();
    let meta: Value = serde_json::from_slice(&read("meta.json")).unwrap();
    let seg: Value = serde_json::from_slice(&read("segmentation/segmentation.json")).unwrap();
    let upper = meta["arch"] == "UPPER";
    let arch = if upper { "upper" } else { "lower" };
    let role = if upper {
        ScanRole::Upper
    } else {
        ScanRole::Lower
    };
    let mut state = crate::commands::find("aligners.demo")
        .unwrap()
        .execute(
            &CaseState::default(),
            &json!({ "arch": arch }),
            &JobCtx::detached(),
        )
        .unwrap()
        .0;
    let mut scan = dc_mesh::dcm::decode(&read("scan.dcm")).unwrap();
    let rle = seg["vertex_labels"]["rle"].as_array().unwrap();
    let labels: Vec<u16> = rle
        .chunks(2)
        .flat_map(|p| {
            std::iter::repeat_n(
                p[0].as_u64().unwrap() as u16,
                p[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(labels.len(), scan.vertex_count());
    scan.vertex_labels = None;
    let mut labelled = scan.clone();
    labelled.vertex_labels = Some(labels);
    let scan_ref = state.blobs.insert(scan);
    let labels_ref = state.blobs.insert(labelled);
    for s in &mut state.case.scans {
        if s.role == role {
            s.mesh = scan_ref;
        }
    }
    let mut data = aligners_data(&state).unwrap();
    data.labels.insert(role, labels_ref);
    data.seeds = meta["seeds_gt_world"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(f, p)| {
            let p: Vec<f64> = p
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            Seed {
                fdi: Fdi::new(f.parse().unwrap()).unwrap(),
                position: DVec3::new(p[0], p[1], p[2]),
            }
        })
        .collect();
    state.set_module(MODULE_ID, ModuleState::new(data));
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_mesh::ValidationReport;

    fn run(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    const AUTO: &str = "aligners.preparation.reconstruction.automatic";
    const ONE: &str = "aligners.preparation.reconstruction.rebuild-tooth";

    fn demo_state(arch: &str) -> CaseState {
        run(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": arch }),
        )
        .unwrap()
        .0
    }

    #[test]
    fn the_commands_are_registered_with_schemas() {
        for id in [AUTO, ONE] {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object");
            assert!(crate::commands::is_registered(id), "{id}");
        }
        assert!(crate::commands::find(ONE).unwrap().schema()["properties"]["fdi"].is_object());
    }

    #[test]
    fn an_unsegmented_case_or_a_bad_tooth_is_refused() {
        assert!(matches!(
            run(AUTO, &CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
        let lower = demo_state("lower");
        assert!(matches!(
            run(ONE, &lower, Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("tooth number")
        ));
        assert!(matches!(
            run(ONE, &lower, json!({ "fdi": 11 })),
            Err(CommandError::Invalid(m)) if m.contains("upper")
        ));
        assert!(matches!(
            run(AUTO, &lower, json!({ "neck_depth_mm": "deep" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_demo_teeth_are_closed_solids_stored_in_the_case() {
        // the demo arch is made of closed shells: every tooth is closed already
        let state = demo_state("lower");
        let (next, out) = run(AUTO, &state, Value::Null).unwrap();
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.crowns.len(), 14);
        assert_eq!(data.tooth_frames.len(), 14);
        for (f, blob) in &data.crowns {
            let m = next.mesh(blob).unwrap();
            let rep = ValidationReport::of(m);
            assert!(rep.is_watertight() && rep.is_manifold(), "{f}: {rep:?}");
            let t = &out["lower"]["teeth"][f.to_string()];
            assert_eq!(t["vertices"].as_u64().unwrap() as usize, m.vertex_count());
        }
        // the crowns are kept on save
        let used = next.referenced_blobs();
        assert!(data.crowns.values().all(|b| used.contains(b)));
    }

    /// Symmetric largest distance between the vertices of two meshes.
    fn vertex_hausdorff(a: &TriMesh, b: &TriMesh) -> f64 {
        let pa: Vec<DVec3> = a.positions.iter().map(|p| p.as_dvec3()).collect();
        let pb: Vec<DVec3> = b.positions.iter().map(|p| p.as_dvec3()).collect();
        let d = |x: &[DVec3], y: &[DVec3]| {
            reconstruct::geom::min_distances(x, y)
                .into_iter()
                .fold(0.0, f64::max)
        };
        d(&pa, &pb).max(d(&pb, &pa))
    }

    #[test]
    fn a_scanned_tooth_is_closed_like_the_plugin_does() {
        let state = golden_state("demo_upper_mild");
        let f = Fdi::new(21).unwrap();
        let (next, out) = run(ONE, &state, json!({ "fdi": 21 })).unwrap();
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(data.crowns.len(), 1);
        assert_eq!(data.tooth_frames.len(), 14, "the frames are measured first");
        let m = next.mesh(&data.crowns[&f]).unwrap();
        let rep = ValidationReport::of(m);
        assert!(rep.is_watertight() && rep.is_manifold(), "{rep:?}");
        let t = &out["tooth"];
        assert!(t["fill_vertices"].as_u64().unwrap() > 100, "{t}");
        assert_eq!(t["crossing_faces"], 0);
        // the oracle's solid (tooth-local coordinates, moved by its rest frame and the arch frame)
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../golden/fixtures/demo_upper_mild");
        let oracle =
            dc_mesh::dcm::decode(&std::fs::read(dir.join("staging/closed_21.dcm")).unwrap())
                .unwrap();
        let teeth: Value =
            serde_json::from_slice(&std::fs::read(dir.join("teeth/teeth.json")).unwrap()).unwrap();
        let meta: Value =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
        let mat = |v: &Value| {
            let r: Vec<f64> = v
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|row| row.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()))
                .collect();
            let a: [f64; 16] = r.try_into().unwrap();
            DMat4::from_cols_array(&a).transpose()
        };
        let to_world = mat(&meta["frame_gt"]) * mat(&teeth["teeth"]["21"]["rest_frame"]);
        let mut want = oracle.clone();
        for p in &mut want.positions {
            *p = to_world.transform_point3(p.as_dvec3()).as_vec3();
        }
        let hd = vertex_hausdorff(m, &want);
        assert!(hd < 0.05, "{hd} mm from the plugin's solid");
        // the same tooth again gives the same solid; a deeper neck another one
        let (again, _) = run(ONE, &next, json!({ "fdi": 21 })).unwrap();
        let d2 = again.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(d2.crowns[&f], data.crowns[&f]);
        let (deeper, _) = run(ONE, &next, json!({ "fdi": 21, "neck_depth_mm": 2.5 })).unwrap();
        let d3 = deeper.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_ne!(d3.crowns[&f], data.crowns[&f]);
    }

    #[test]
    fn cancelling_after_the_first_tooth_stores_nothing() {
        let state = demo_state("lower");
        let before = state.module_data::<AlignersState>(MODULE_ID).cloned();
        let r = run_cancelled_at_first_progress(AUTO, &state, Value::Null);
        assert!(
            matches!(r, Err(CommandError::Cancelled)),
            "{:?}",
            r.map(|o| o.1)
        );
        assert_eq!(
            state.module_data::<AlignersState>(MODULE_ID).cloned(),
            before
        );
    }
}
