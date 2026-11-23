//! Commands of the `occlusion` stage (`dca_algo::occlusion`).
//!
//! * `aligners.occlusion.virtual-bite` puts the upper arch in occlusion on
//!   the lower one when no bite scan is available (estimated occlusion:
//!   midlines, overjet and Class I, maximum intercuspation without
//!   interpenetration). The upper scan, its labels, seeds and tooth frames
//!   move rigidly in the case; the lower arch stays in place.
//! * `aligners.occlusion.contact-map` measures the occlusal contacts of the
//!   relation in the case: the signed distance of every crown vertex to the
//!   antagonist crowns, the teeth in contact and the deepest
//!   interpenetration. The case is not modified (the app colours the map).
//!
//! Both need the two arches segmented (labels) with their seeds. A case with
//! one arch has no bite ([`dca_model::bite_use`]): every bite command answers
//! that it does not apply ([`one_arch`]) and leaves the case as it is.

use super::messages;
use dc_case::{BlobRef, CaseState, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::occlusion::{self, ArchModel};
use dca_model::{AlignersState, BiteUse, MODULE_ID};
use glam::{DMat3, DMat4};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.occlusion.virtual-bite",
            label: "aligners-cmd-occlusion-virtual-bite",
            doc: "aligners-cmd-occlusion-virtual-bite-doc",
            params: occlusion::Params,
            run: run_virtual_bite,
        },
        stage_command! {
            id: "aligners.occlusion.contact-map",
            label: "aligners-cmd-occlusion-contact-map",
            doc: "aligners-cmd-occlusion-contact-map-doc",
            params: ContactMapParams,
            run: run_contact_map,
        },
    ]
}

/// Parameters of the contact map (none yet).
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ContactMapParams {}

/// The Aligners data of the case (new data when there is none).
pub(crate) fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The answer of a bite command on a case with one arch: there the bite
/// does not apply ([`dca_model::bite_use`]), so nothing is registered,
/// estimated or measured and the case is left as it is. The output says it:
/// `applies` false, `reason` `one_arch`, the `arch` of the case and the
/// sentence (`message`). `None`: the case has both arches (or none), the
/// command runs.
pub(crate) fn one_arch(state: &CaseState) -> Option<CommandOutput> {
    if !BiteUse::of(state).one_arch() {
        return None;
    }
    let arch = if state.case.scan(ScanRole::Upper).is_some() {
        Arch::Upper
    } else {
        Arch::Lower
    };
    Some((
        state.clone(),
        json!({
            "applies": false,
            "reason": "one_arch",
            "arch": arch_name(arch),
            "message": messages::tr(
                "aligners-bite-one-arch",
                &[("arch", &messages::arch_word(arch))],
            ),
        }),
    ))
}

/// The labelled scan of an arch as an [`ArchModel`].
pub(crate) fn arch_model(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<ArchModel, CommandError> {
    let role = arch.role();
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Err(messages::invalid(
            "aligners-err-both-arches",
            &[("arch", &messages::arch_word(arch))],
        ));
    };
    let mesh = state.blobs.require(&scan.mesh)?;
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .as_ref()
        .filter(|l| l.len() == mesh.vertex_count())
        .ok_or_else(|| {
            messages::invalid(
                "aligners-err-labels-stale",
                &[("arch", &messages::arch_word(arch))],
            )
        })?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let flipped = data.occlusal_flipped.contains(&arch.role());
    ArchModel::new(arch, &vertices, &mesh.tris, labels, &data.seeds, flipped)
        .map_err(|e| CommandError::Invalid(format!("{} arch: {e}", arch_name(arch))))
}

/// `mesh` moved rigidly by `delta` (normals turned with it).
pub(crate) fn moved_mesh(mesh: &TriMesh, delta: &DMat4) -> TriMesh {
    let mut out = mesh.clone();
    out.positions = occlusion::transform_positions(&mesh.positions, delta);
    let r = DMat3::from_mat4(*delta);
    if let Some(n) = &mut out.normals {
        for v in n.iter_mut() {
            *v = (r * v.as_dvec3()).normalize_or_zero().as_vec3();
        }
    }
    out
}

/// Moves the scans with role `role`, in order, by `deltas` (one move per
/// scan; scans beyond the moves stay) in `next`; returns the replaced blobs.
/// The base a moved scan was closed with holds on its new blob.
fn move_scans(
    next: &mut CaseState,
    data: &mut AlignersState,
    role: ScanRole,
    deltas: &[DMat4],
) -> Vec<BlobRef> {
    let mut stale = Vec::new();
    let mut k = 0;
    for i in 0..next.case.scans.len() {
        if next.case.scans[i].role != role || k >= deltas.len() {
            continue;
        }
        let old = next.case.scans[i].mesh;
        if let Some(mesh) = next.blobs.get(&old).cloned() {
            let r = next.blobs.insert(moved_mesh(&mesh, &deltas[k]));
            next.case.scans[i].mesh = r;
            data.scan_moved(old, r);
            stale.push(old);
        }
        k += 1;
    }
    stale
}

/// Moves the scan of `arch` with its labels, seeds, tooth frames and
/// measures, closed crowns (a size corrected by hand stays with its moved
/// solid), gingiva, study model, setup arch line and the record of what its
/// 3D teeth were built from (so a move alone never leaves them out of date) by the
/// case-coordinate rigid move `delta`, and moves the scans of the bite (role
/// `Bite`, in order) by `bite_deltas`. A moved scan keeps the base it was
/// closed with. Unused blobs are dropped.
pub(crate) fn move_arch(
    state: &CaseState,
    mut data: AlignersState,
    arch: Arch,
    delta: &DMat4,
    bite_deltas: &[DMat4],
) -> CaseState {
    let mut next = state.clone();
    let mut stale = move_scans(
        &mut next,
        &mut data,
        arch.role(),
        std::slice::from_ref(delta),
    );
    stale.extend(move_scans(
        &mut next,
        &mut data,
        ScanRole::Bite,
        bite_deltas,
    ));
    let old_labels = data.labels.get(&arch.role()).copied();
    if let Some(old) = old_labels
        && let Some(mesh) = next.blobs.get(&old).cloned()
    {
        let r = next.blobs.insert(moved_mesh(&mesh, delta));
        data.labels.insert(arch.role(), r);
        stale.push(old);
    }
    // the 3D teeth move with the arch: what they were built from moves too
    let new_labels = data.labels.get(&arch.role()).copied();
    if let Some(basis) = data.teeth_basis.get_mut(&arch.role()) {
        if basis.labels == old_labels {
            basis.labels = new_labels;
        }
        for s in &mut basis.seeds {
            s.position = delta.transform_point3(s.position);
        }
    }
    // the closed crowns, the gingiva and the study model are case-coordinate
    // blobs too
    let blobs: Vec<BlobRef> = data
        .crowns
        .iter()
        .filter(|(f, _)| Arch::of(**f) == arch)
        .map(|(_, r)| *r)
        .chain(data.gingiva.get(&arch.role()).copied())
        .chain(data.study_models.get(&arch.role()).copied())
        .collect();
    for old in blobs {
        if let Some(mesh) = next.blobs.get(&old).cloned() {
            let r = next.blobs.insert(moved_mesh(&mesh, delta));
            for v in data
                .crowns
                .values_mut()
                .chain(data.gingiva.values_mut())
                .chain(data.study_models.values_mut())
            {
                if *v == old {
                    *v = r;
                }
            }
            // a size corrected by hand and the scanned part hold on the moved solid
            data.components.crown_moved(old, r);
            data.crown_replaced(old, r);
            stale.push(old);
        }
    }
    for s in data.seeds.iter_mut().filter(|s| Arch::of(s.fdi) == arch) {
        s.position = delta.transform_point3(s.position);
    }
    // the line the setup used and the one drawn by the clinician
    let role = arch.role();
    for line in [
        data.setup.lines.get_mut(&role),
        data.setup.drawn.get_mut(&role),
    ]
    .into_iter()
    .flatten()
    {
        for p in line {
            *p = delta.transform_point3(*p);
        }
    }
    for (_, m) in data
        .tooth_geometry
        .iter_mut()
        .filter(|(f, _)| Arch::of(**f) == arch)
    {
        for p in [
            &mut m.centroid,
            &mut m.reference,
            &mut m.tip,
            &mut m.cej,
            &mut m.crest,
            &mut m.apex,
            &mut m.contact_m,
            &mut m.contact_d,
        ] {
            *p = delta.transform_point3(*p);
        }
    }
    let pose = dca_algo::rotation::pose_from_mat4(delta);
    for (_, f) in data
        .tooth_frames
        .iter_mut()
        .filter(|(f, _)| Arch::of(**f) == arch)
    {
        f.pose = pose.compose(&f.pose);
    }
    data.prune_scan_bases(next.case.scans.iter().map(|s| &s.mesh));
    next.set_module(MODULE_ID, ModuleState::new(data));
    let used = next.referenced_blobs();
    let keep = next
        .blobs
        .iter()
        .map(|(r, _)| *r)
        .filter(|r| used.contains(r) || !stale.contains(r))
        .collect();
    next.blobs.retain_only(&keep);
    next
}

/// Rigid move as JSON: translation (mm) and rotation (degrees).
pub(crate) fn move_json(m: &DMat4) -> Value {
    json!({
        "translation_mm": [m.w_axis.x, m.w_axis.y, m.w_axis.z],
        "rotation_deg": dca_algo::rotation::degrees(&dca_algo::rotation::pose_from_mat4(m)),
    })
}

fn run_virtual_bite(
    state: &CaseState,
    params: occlusion::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if let Some(out) = one_arch(state) {
        return Ok(out);
    }
    let mut data = aligners_data(state)?;
    let lower = arch_model(state, &data, Arch::Lower)?;
    let upper = arch_model(state, &data, Arch::Upper)?;
    ctx.check_cancelled()?;
    let was_in_occlusion = occlusion::in_occlusion(&lower.to_case, &upper.to_case);
    let vb = occlusion::align(&lower, &upper, &params)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    ctx.check_cancelled()?;
    // like the registration: the move is baked into the upper mesh, so replacing
    // an arch later has to say that the registration is stale
    data.bite_registered = true;
    // the source of the relation, clearly an estimate (`bite.mark(VIRTUALE)`)
    data.bite_report = Some(dca_model::BiteReport::virtual_bite());
    let next = move_arch(state, data, Arch::Upper, &vb.delta, &[]);
    let mut out = serde_json::to_value(&vb).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["source"] = json!("virtual");
    out["was_in_occlusion"] = json!(was_in_occlusion);
    out["upper_move"] = move_json(&vb.delta);
    Ok((next, out))
}

fn run_contact_map(
    state: &CaseState,
    _params: ContactMapParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if let Some(out) = one_arch(state) {
        return Ok(out);
    }
    let data = aligners_data(state)?;
    let lower = arch_model(state, &data, Arch::Lower)?;
    let upper = arch_model(state, &data, Arch::Upper)?;
    ctx.check_cancelled()?;
    let map =
        occlusion::contact_map(&lower, &upper).map_err(|e| CommandError::Failed(e.to_string()))?;
    let mut out = serde_json::to_value(&map).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["in_occlusion"] = json!(occlusion::in_occlusion(&lower.to_case, &upper.to_case));
    out["contact_mm"] = json!(occlusion::CONTACT_MM);
    Ok((state.clone(), out))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dc_case::ScanRole;
    use glam::DQuat;

    pub(crate) fn exec(id: &str, state: &CaseState, params: Value) -> CommandOutput {
        try_exec(id, state, params).unwrap()
    }

    pub(crate) fn try_exec(
        id: &str,
        state: &CaseState,
        params: Value,
    ) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap_or_else(|| panic!("{id} is not registered"))
            .execute(state, &params, &JobCtx::detached())
    }

    /// Lower demo arch (seed 7) and upper demo arch (seed 11), not related.
    pub(crate) fn two_demo_arches() -> CaseState {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .0;
        exec("aligners.demo", &s, json!({ "arch": "upper", "seed": 11 })).0
    }

    #[test]
    fn the_commands_are_registered_and_documented() {
        for id in [
            "aligners.occlusion.virtual-bite",
            "aligners.occlusion.contact-map",
        ] {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object");
            assert!(crate::commands::is_registered(id), "{id}");
        }
    }

    /// The bite command `id` on `state`, a case with one arch: it does not
    /// apply, says so in the words of the app and leaves the case as it is.
    pub(crate) fn assert_not_applicable(id: &str, state: &CaseState, params: Value) {
        let (next, out) = try_exec(id, state, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(out["applies"], false, "{id}: {out}");
        assert_eq!(out["reason"], "one_arch", "{id}: {out}");
        let arch = if out["arch"] == "upper" {
            Arch::Upper
        } else {
            assert_eq!(out["arch"], "lower", "{id}: {out}");
            Arch::Lower
        };
        assert!(state.case.scan(arch.role()).is_some(), "{id}: {out}");
        let word = messages::arch_word(arch);
        assert_eq!(
            out["message"],
            messages::tr("aligners-bite-one-arch", &[("arch", &word)]),
            "{id}"
        );
        assert_eq!(next.case, state.case, "{id}");
        assert_eq!(
            next.module_data::<AlignersState>(MODULE_ID),
            state.module_data::<AlignersState>(MODULE_ID),
            "{id}"
        );
        assert_eq!(next.referenced_blobs(), state.referenced_blobs(), "{id}");
    }

    #[test]
    fn one_arch_has_no_bite_to_estimate_or_measure() {
        for arch in ["lower", "upper"] {
            let one = exec(
                "aligners.demo",
                &CaseState::default(),
                json!({ "arch": arch }),
            )
            .0;
            for id in [
                "aligners.occlusion.virtual-bite",
                "aligners.occlusion.contact-map",
            ] {
                assert_not_applicable(id, &one, Value::Null);
            }
            // the parameters are still checked
            assert!(matches!(
                try_exec(
                    "aligners.occlusion.virtual-bite",
                    &one,
                    json!({ "nope": 1 })
                ),
                Err(CommandError::Invalid(_))
            ));
        }
        // both arches, one of them without its tooth colours: the step needs it
        let mut two = two_demo_arches();
        let mut d = two.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        d.labels.remove(&ScanRole::Upper);
        two.set_module(MODULE_ID, ModuleState::new(d));
        let err = try_exec("aligners.occlusion.virtual-bite", &two, Value::Null).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("upper")),
            "{err}"
        );
    }

    #[test]
    fn the_virtual_bite_moves_the_upper_arch_into_occlusion() {
        let state = two_demo_arches();
        let (next, out) = exec("aligners.occlusion.virtual-bite", &state, Value::Null);
        assert_eq!(out["source"], "virtual");
        assert_eq!(out["was_in_occlusion"], false);
        // the move is baked into the upper mesh: replacing an arch has to report
        // the stale registration, as for the registration commands
        let before = state.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!before.bite_registered);
        let after = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(after.bite_registered);
        assert!(out["contacts"].as_u64().unwrap() > 50, "{out}");
        // the lower scan is untouched, the upper scan, labels and seeds moved together
        let data0 = state.module_data::<AlignersState>(MODULE_ID).unwrap();
        let data1 = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(
            state.case.scan(ScanRole::Lower).unwrap().mesh,
            next.case.scan(ScanRole::Lower).unwrap().mesh
        );
        assert_eq!(
            data0.labels[&ScanRole::Lower],
            data1.labels[&ScanRole::Lower]
        );
        let up = next
            .mesh(&next.case.scan(ScanRole::Upper).unwrap().mesh)
            .unwrap();
        let up_labels = next.mesh(&data1.labels[&ScanRole::Upper]).unwrap();
        assert_eq!(up.positions, up_labels.positions);
        assert_ne!(
            up.positions,
            state
                .mesh(&state.case.scan(ScanRole::Upper).unwrap().mesh)
                .unwrap()
                .positions
        );
        // no stale blob left: two scans and two label meshes
        assert_eq!(next.blobs.len(), 4);
        let lower = arch_model(&next, data1, Arch::Lower).unwrap();
        let upper = arch_model(&next, data1, Arch::Upper).unwrap();
        assert!(occlusion::in_occlusion(&lower.to_case, &upper.to_case));
        // the contact map of the new relation sees contacts and no deep interpenetration
        let (same, map) = exec("aligners.occlusion.contact-map", &next, Value::Null);
        // the contact map does not modify the case
        assert_eq!(same.case, next.case);
        assert_eq!(same.blobs.len(), next.blobs.len());
        assert_eq!(map["in_occlusion"], true);
        assert!(map["penetration_mm"].as_f64().unwrap() < 0.1, "{map}");
        assert!(
            !map["lower"]["teeth_in_contact"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(
            !map["upper"]["teeth_in_contact"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        // a second virtual bite starts from the occlusion and stays close to it
        let (_, out2) = exec("aligners.occlusion.virtual-bite", &next, Value::Null);
        assert_eq!(out2["was_in_occlusion"], true);
        let d = &out2["upper_move"];
        assert!(d["rotation_deg"].as_f64().unwrap() < 0.5, "{out2}");
    }

    #[test]
    fn the_crowns_gingiva_measures_and_arch_line_move_with_the_upper_arch() {
        let mut state = two_demo_arches();
        // with both arches the setup needs their relation first: taken as
        // registered, so that the virtual bite then moves an upper arch with
        // its setup from where the demo put it
        let mut data = aligners_data(&state).unwrap();
        data.bite_registered = true;
        state.set_module(MODULE_ID, ModuleState::new(data));
        for id in [
            "aligners.preparation.reconstruction.automatic",
            "aligners.preparation.gingiva",
            "aligners.planning.setup.automatic",
        ] {
            state = exec(id, &state, Value::Null).0;
        }
        // stand-ins for the study models: the scans raised by 1 mm
        let mut data = aligners_data(&state).unwrap();
        for role in [ScanRole::Upper, ScanRole::Lower] {
            let mut m = (**state.mesh(&state.case.scan(role).unwrap().mesh).unwrap()).clone();
            for p in &mut m.positions {
                p.z += 1.0;
            }
            data.study_models.insert(role, state.blobs.insert(m));
        }
        state.set_module(MODULE_ID, ModuleState::new(data));
        let (next, _) = exec("aligners.occlusion.virtual-bite", &state, Value::Null);
        let d0 = state.module_data::<AlignersState>(MODULE_ID).unwrap();
        let d1 = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        // the rigid move of the upper arch, from its scan
        let scan = |s: &CaseState| {
            s.mesh(&s.case.scan(ScanRole::Upper).unwrap().mesh)
                .unwrap()
                .positions
                .clone()
        };
        let (a, b) = (scan(&state), scan(&next));
        let f11 = dc_case::Fdi::new(11).unwrap();
        let f31 = dc_case::Fdi::new(31).unwrap();
        // the frame of 11 moved like the scan: so must its crown, measures and the gingiva
        let delta =
            d1.tooth_frames[&f11].pose.to_mat4() * d0.tooth_frames[&f11].pose.to_mat4().inverse();
        let close = |p: glam::DVec3, q: glam::DVec3| (p - q).length() < 1e-3;
        assert!(close(
            delta.transform_point3(a[0].as_dvec3()),
            b[0].as_dvec3()
        ));
        let c0 = state.mesh(&d0.crowns[&f11]).unwrap();
        let c1 = next.mesh(&d1.crowns[&f11]).unwrap();
        assert!(close(
            delta.transform_point3(c0.positions[0].as_dvec3()),
            c1.positions[0].as_dvec3()
        ));
        let g0 = state.mesh(&d0.gingiva[&ScanRole::Upper]).unwrap();
        let g1 = next.mesh(&d1.gingiva[&ScanRole::Upper]).unwrap();
        assert_eq!(g0.vertex_labels, g1.vertex_labels);
        assert!(close(
            delta.transform_point3(g0.positions[7].as_dvec3()),
            g1.positions[7].as_dvec3()
        ));
        let (m0, m1) = (&d0.tooth_geometry[&f11], &d1.tooth_geometry[&f11]);
        assert!(close(delta.transform_point3(m0.tip), m1.tip));
        let (l0, l1) = (
            &d0.setup.lines[&ScanRole::Upper],
            &d1.setup.lines[&ScanRole::Upper],
        );
        assert!(close(delta.transform_point3(l0[0]), l1[0]));
        // the lower arch stays
        assert_eq!(d0.crowns[&f31], d1.crowns[&f31]);
        assert_eq!(d0.gingiva[&ScanRole::Lower], d1.gingiva[&ScanRole::Lower]);
        // the study model of the upper arch moved with it, the lower one stays
        let s0 = state.mesh(&d0.study_models[&ScanRole::Upper]).unwrap();
        let s1 = next.mesh(&d1.study_models[&ScanRole::Upper]).unwrap();
        assert!(close(
            delta.transform_point3(s0.positions[3].as_dvec3()),
            s1.positions[3].as_dvec3()
        ));
        assert_eq!(
            d0.study_models[&ScanRole::Lower],
            d1.study_models[&ScanRole::Lower]
        );
        // no stale blob left
        let used = next.referenced_blobs();
        assert!(next.blobs.iter().all(|(r, _)| used.contains(r)));
    }

    #[test]
    fn moving_an_arch_keeps_its_3d_teeth_up_to_date() {
        let built = exec("aligners.teeth.build", &two_demo_arches(), Value::Null).0;
        let data = |s: &CaseState| s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        let stale =
            |d: &AlignersState| [ScanRole::Upper, ScanRole::Lower].map(|r| d.teeth_out_of_date(r));
        assert_eq!(stale(&data(&built)), [false, false]);
        // the virtual bite moves the upper arch with its 3D teeth: its colours
        // and marks are moved, not changed, so both arches stay up to date
        let (next, _) = exec("aligners.occlusion.virtual-bite", &built, Value::Null);
        let d = data(&next);
        assert_ne!(
            d.labels[&ScanRole::Upper],
            data(&built).labels[&ScanRole::Upper]
        );
        assert_eq!(stale(&d), [false, false]);
        // also once the project is reopened
        let back: AlignersState =
            serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(stale(&back), [false, false]);
        // a mark moved after the build: still out of date once the arch is moved
        let mut d = data(&built);
        let mark = d.seeds.iter_mut().find(|s| s.fdi.is_upper()).unwrap();
        mark.position.x += 0.5;
        assert_eq!(stale(&d), [true, false]);
        let delta = DMat4::from_rotation_translation(
            DQuat::from_rotation_z(0.1),
            glam::DVec3::new(1.0, 2.0, 3.0),
        );
        let moved = move_arch(&built, d, Arch::Upper, &delta, &[]);
        assert_eq!(stale(&data(&moved)), [true, false]);
    }
}
