//! `aligners.case.make-base`: closes an open arch scan with a flat base, the
//! plugin's `oa.make_base` ("Chiudi con base", `case_core.make_base` and
//! `aligner.make_base`; algorithm in [`dca_algo::scan_base`]).
//!
//! As in the plugin the base is perpendicular to the occlusal direction of
//! the arch frame the seeds of the arch define (`compute_frame`: the arch
//! needs its tooth marks on both sides and on the incisors), `height_mm`
//! (0.5 to 20, default 3: the plugin's `base_height`) below the lowest point
//! of the scan. One deviation: the frame is turned over when the occlusal
//! side of the arch was turned over by hand (`aligners.case.set-arch` with
//! `flip_occlusal`), where the plugin's `compute_frame` here forgets it and
//! would put the base on the teeth.
//!
//! The scan gets a new mesh in one undoable change; its points keep their
//! index and the points of the base follow. The plugin keeps the label of
//! every point and gives each new point the label of the point it was
//! extruded from (bmesh copies the attributes): so does the labels blob of
//! the arch here, and nothing computed from the labels goes or goes out of
//! date (the tooth colours are the same; the plugin records no step). A scan
//! that is already closed is left as it is: not an error, the plugin's "Il
//! modello era gia chiuso" (`already_closed`).

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::scan_base::{self, BaseError, HEIGHT_DEFAULT_MM};
use dca_model::{ClosedBy, MODULE_ID, ScanBase};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::messages;
use crate::manual::common::{arch_name, data_of, finish_blobs, pick_arch};

/// Id of the command.
pub const MAKE_BASE: &str = "aligners.case.make-base";

/// Parameters of `aligners.case.make-base`.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MakeBaseParams {
    /// The arch scan to close: upper or lower (default: the only arch scan
    /// of the case).
    pub arch: Option<Arch>,
    /// Thickness of the base under the lowest point of the scan, mm (0.5 to
    /// 20, default 3).
    pub height_mm: f64,
}

impl Default for MakeBaseParams {
    fn default() -> Self {
        Self {
            arch: None,
            height_mm: HEIGHT_DEFAULT_MM,
        }
    }
}

pub(super) fn run_make_base(
    state: &CaseState,
    params: MakeBaseParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let height = params.height_mm;
    if !(scan_base::HEIGHT_MIN_MM..=scan_base::HEIGHT_MAX_MM).contains(&height) {
        return Err(height_error(height));
    }
    let arch = pick_arch(state, params.arch, None)?;
    let role = arch.role();
    let scan = state
        .case
        .scan(role)
        .ok_or_else(|| messages::no_scan(arch))?;
    let mut data = data_of(state)?;
    let closed = state.blobs.require(&scan.mesh)?;
    // the scan as it was read: a scan closed with a base gets it again,
    // never a second one
    let rebuilt = data.scan_base(&scan.mesh).is_some();
    let mesh = crate::open_scan::open_part(&data, &scan.mesh, closed)?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let flipped = data.occlusal_flipped.contains(&role);
    // the frame of the tooth marks, as the plugin; without them the occlusal
    // direction of the scan itself, as the import
    let framed = frame::run_from_seeds(
        &frame::Input {
            arch,
            vertices: &vertices,
            seeds: &data.seeds,
            occlusal_flipped: flipped,
        },
        &frame::Params::default(),
    );
    let (n, from_marks) = match framed {
        Ok(f) => (f.arch_frame.n, true),
        Err(frame::FrameError::TooFewSeeds | frame::FrameError::SeedsOnOneSide) => {
            let n = scan_base::occlusal_direction(&mesh)
                .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
            (if flipped { -n } else { n }, false)
        }
        Err(e) => return Err(messages::frame_error(arch, &e)),
    };
    ctx.check_cancelled()?;
    let based = scan_base::make_base(&mesh, n, height).map_err(|e| match e {
        BaseError::Height(h) => height_error(h),
        BaseError::Direction | BaseError::Mesh => {
            CommandError::Failed(format!("{} arch: {e}", arch_name(arch)))
        }
    })?;
    let scan_id = scan.id;
    let Some(based) = based else {
        return Ok((
            state.clone(),
            json!({
                "arch": arch_name(arch),
                "scan_id": scan_id,
                "already_closed": true,
                "rebuilt": false,
                "frame": if from_marks { "tooth_marks" } else { "scan" },
                "closed_edges": 0,
                "vertices_added": 0,
                "faces_added": 0,
                "height_mm": height,
                "watertight": closed.validate().is_watertight(),
                "labels_extended": false,
            }),
        ));
    };
    ctx.check_cancelled()?;

    let mut next = state.clone();
    let mut stale = vec![scan.mesh];
    let vertices_added = based.sources.len();
    let faces_added = based.mesh.tris.len() - mesh.tris.len();
    let watertight = based.mesh.validate().is_watertight();
    // the labels of the arch follow the points of the scan; the base is
    // gingiva
    let mut labels_extended = false;
    if let Some(old) = data.labels.get(&role).copied() {
        let labelled = state.blobs.require(&old)?;
        let labels = labelled
            .vertex_labels
            .as_ref()
            .ok_or_else(|| messages::labels_missing(arch))?;
        if labels.len() != closed.vertex_count() {
            return Err(messages::labels_mismatch(
                arch,
                labels.len(),
                closed.vertex_count(),
            ));
        }
        let mut extended = crate::open_scan::open_labels(labels, &mesh).to_vec();
        extended.resize(based.mesh.vertex_count(), 0);
        let mut new = based.mesh.clone();
        new.vertex_labels = Some(extended);
        new.face_labels = None;
        let r = next.blobs.insert(new);
        data.labels.insert(role, r);
        // the teeth were built from these colours: they still match
        if let Some(basis) = data.teeth_basis.get_mut(&role)
            && basis.labels == Some(old)
        {
            basis.labels = Some(r);
        }
        stale.push(old);
        labels_extended = true;
    }
    let record = ScanBase {
        closed_by: if from_marks {
            ClosedBy::ToothMarks
        } else {
            ClosedBy::Hand
        },
        ..based.scan
    };
    let mesh_ref = next.blobs.insert(based.mesh);
    if let Some(s) = next.case.scans.iter_mut().find(|s| s.id == scan_id) {
        s.mesh = mesh_ref;
    }
    data.scan_bases.insert(mesh_ref, record);
    data.prune_scan_bases(next.case.scans.iter().map(|s| &s.mesh));
    next.set_module(MODULE_ID, ModuleState::new(data));
    finish_blobs(&mut next, &stale);
    Ok((
        next,
        json!({
            "arch": arch_name(arch),
            "scan_id": scan_id,
            "already_closed": false,
            "rebuilt": rebuilt,
            "frame": if from_marks { "tooth_marks" } else { "scan" },
            "closed_edges": based.closed_edges,
            "vertices_added": vertices_added,
            "faces_added": faces_added,
            "height_mm": height,
            "watertight": watertight,
            "labels_extended": labels_extended,
        }),
    ))
}

fn height_error(h: f64) -> CommandError {
    messages::invalid("aligners-err-base-height", &[("value", &h.to_string())])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dc_case::ScanRole;
    use dc_command::UndoStack;
    use dc_mesh::TriMesh;
    use dca_model::AlignersState;
    use serde_json::Value;
    use std::sync::Arc;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    fn scan_mesh(state: &CaseState, role: ScanRole) -> Arc<TriMesh> {
        Arc::clone(
            state
                .blobs
                .require(&state.case.scan(role).unwrap().mesh)
                .unwrap(),
        )
    }

    /// The demo arch opened under the gum (see [`opened_under_gum`]).
    fn open_demo(arch: &str) -> CaseState {
        let state = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": arch }),
        )
        .unwrap()
        .0;
        let role = if arch == "upper" {
            ScanRole::Upper
        } else {
            ScanRole::Lower
        };
        opened_under_gum(state, role)
    }

    /// The scan of `role` opened under the gum: the faces within 4 mm of its
    /// lowest point along the occlusal axis removed (the demo scan is
    /// closed).
    pub(crate) fn opened_under_gum(mut state: CaseState, role: ScanRole) -> CaseState {
        let arch = Arch::from_role(role).unwrap();
        let mesh = scan_mesh(&state, role);
        let v: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let n = frame::run_from_seeds(
            &frame::Input {
                arch,
                vertices: &v,
                seeds: &data(&state).seeds,
                occlusal_flipped: false,
            },
            &frame::Params::default(),
        )
        .unwrap()
        .arch_frame
        .n;
        let low = v.iter().map(|p| p.dot(n)).fold(f64::INFINITY, f64::min) + 4.0;
        // the faces go, the points stay (the labels keep matching the scan)
        let mut open = mesh.as_ref().clone();
        open.tris
            .retain(|t| !t.iter().all(|&i| v[i as usize].dot(n) < low));
        assert!(open.tris.len() < mesh.tris.len());
        let r = state.blobs.insert(open);
        let s = state
            .case
            .scans
            .iter_mut()
            .find(|s| s.role == role)
            .unwrap();
        s.mesh = r;
        state
    }

    #[test]
    fn it_is_registered_for_the_cli_with_the_plugins_default_height() {
        let spec = crate::commands::find(MAKE_BASE).unwrap();
        let schema = spec.schema();
        assert!(schema["properties"]["height_mm"].is_object(), "{schema}");
        assert!(schema["properties"]["arch"].is_object(), "{schema}");
        assert!(crate::tree::find(MAKE_BASE).is_none());
        assert_eq!(MakeBaseParams::default().height_mm, 3.0);
    }

    #[test]
    fn an_open_scan_gets_a_watertight_base_and_keeps_its_colours() {
        let state = open_demo("lower");
        let before = data(&state).clone();
        let old = scan_mesh(&state, ScanRole::Lower);
        assert!(!old.validate().is_watertight());
        let (next, out) = exec(
            MAKE_BASE,
            &state,
            json!({ "arch": "lower", "height_mm": 2.5 }),
        )
        .unwrap();
        assert_eq!(out["arch"], "lower", "{out}");
        assert_eq!(out["already_closed"], false, "{out}");
        assert_eq!(out["watertight"], true, "{out}");
        assert_eq!(out["labels_extended"], true, "{out}");
        let new = scan_mesh(&next, ScanRole::Lower);
        assert!(new.validate().is_watertight());
        let added = out["vertices_added"].as_u64().unwrap() as usize;
        assert_eq!(new.positions.len(), old.positions.len() + added);
        assert_eq!(&new.positions[..old.positions.len()], &old.positions[..]);
        // the base plane: 2.5 mm under the lowest point of the scan, along the occlusal axis
        let v: Vec<_> = old.positions.iter().map(|p| p.as_dvec3()).collect();
        let n = crate::manual::common::arch_to_case(data(&state), Arch::Lower, &v)
            .unwrap()
            .z_axis
            .truncate();
        let lowest = v.iter().map(|p| p.dot(n)).fold(f64::INFINITY, f64::min);
        for p in &new.positions[old.positions.len()..] {
            assert!((p.as_dvec3().dot(n) - (lowest - 2.5)).abs() < 1e-4);
        }
        // the colours follow the points; nothing computed goes
        let d = data(&next);
        let labels = next.blobs.require(&d.labels[&ScanRole::Lower]).unwrap();
        let l = labels.vertex_labels.as_ref().unwrap();
        assert_eq!(l.len(), new.positions.len());
        let old_labels = state
            .blobs
            .require(&before.labels[&ScanRole::Lower])
            .unwrap();
        assert_eq!(
            &l[..old.positions.len()],
            &old_labels.vertex_labels.as_ref().unwrap()[..]
        );
        assert_eq!(labels.positions, new.positions);
        assert_eq!(d.seeds, before.seeds);
        // the old scan and labels blobs are gone, nothing dangles
        assert!(
            next.blobs
                .get(&state.case.scan(ScanRole::Lower).unwrap().mesh)
                .is_none()
        );
        for r in next.referenced_blobs() {
            assert!(next.blobs.get(&r).is_some());
        }
        // the scan as it was read is kept as the start of the closed one
        let base = d
            .scan_base(&next.case.scan(ScanRole::Lower).unwrap().mesh)
            .unwrap();
        assert_eq!(base.closed_by, ClosedBy::ToothMarks);
        let open = dca_algo::scan_base::open_part(&new, &base).unwrap();
        assert_eq!(open, *old);
        // a second time: the base is made again from the scan as it was
        // read, never a second one; the same height gives the same solid
        let (again, out) = exec(
            MAKE_BASE,
            &next,
            json!({ "arch": "lower", "height_mm": 2.5 }),
        )
        .unwrap();
        assert_eq!(out["already_closed"], false, "{out}");
        assert_eq!(out["rebuilt"], true, "{out}");
        assert_eq!(out["frame"], "tooth_marks", "{out}");
        assert_eq!(again.case, next.case);
        assert_eq!(data(&again).scan_bases, data(&next).scan_bases);
        // one undo step brings the open scan back
        let mut undo = UndoStack::new(Arc::new(state.clone()), UndoStack::DEFAULT_BUDGET);
        undo.push(Arc::new(next), MAKE_BASE);
        undo.undo().unwrap();
        assert_eq!(undo.current().case, state.case);
    }

    #[test]
    fn it_needs_the_tooth_marks_and_a_height_in_the_plugins_range() {
        let state = open_demo("upper");
        for h in [0.2, 25.0] {
            let e = exec(MAKE_BASE, &state, json!({ "height_mm": h })).unwrap_err();
            assert!(matches!(e, CommandError::Invalid(_)), "{h}: {e:?}");
        }
        // the only arch scan is the default
        assert!(exec(MAKE_BASE, &state, json!({})).is_ok());
        let e = exec(MAKE_BASE, &state, json!({ "arch": "lower" })).unwrap_err();
        assert!(matches!(e, CommandError::Invalid(_)), "{e:?}");
        // without tooth marks the base is perpendicular to the occlusal
        // direction of the scan itself, as the import makes it
        let mut bare = state.clone();
        let mut d = data(&bare).clone();
        d.seeds.clear();
        bare.set_module(MODULE_ID, ModuleState::new(d));
        let (closed, out) = exec(MAKE_BASE, &bare, json!({})).unwrap();
        assert_eq!(out["frame"], "scan", "{out}");
        assert_eq!(out["watertight"], true, "{out}");
        let records = &data(&closed).scan_bases;
        assert_eq!(records.len(), 1);
        assert!(records.values().all(|b| b.closed_by == ClosedBy::Hand));
    }

    #[test]
    fn the_base_goes_under_the_occlusal_side_turned_over_by_hand() {
        let mut state = open_demo("lower");
        let mut d = data(&state).clone();
        d.occlusal_flipped.insert(ScanRole::Lower);
        state.set_module(MODULE_ID, ModuleState::new(d));
        let old = scan_mesh(&state, ScanRole::Lower);
        let (next, _) = exec(MAKE_BASE, &state, json!({})).unwrap();
        let new = scan_mesh(&next, ScanRole::Lower);
        // the occlusal axis is turned: the base is above the teeth now
        let top = |m: &TriMesh| {
            m.positions
                .iter()
                .map(|p| p.z)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert!(top(&new) > top(&old) + 1.0);
    }
}
