//! Commands of the `teeth` stage (`dca_algo::teeth`).
//!
//! `aligners.teeth.frames` measures every labelled tooth of the segmented
//! arches: the local frame (mesio-distal, bucco-lingual and long axes), the
//! mesio-distal and bucco-lingual widths, the cusp tip, the interproximal
//! contacts and the estimated CEJ, alveolar crest, apex and centre of
//! resistance. The frames are stored in `AlignersState::tooth_frames`
//! (case coordinates; the pose has its origin at the crown reference point
//! and the centre of resistance is given in the tooth frame); the measures
//! (widths, contacts, landmarks) are also kept in
//! `AlignersState::tooth_geometry` for the diagnosis, and returned. The scans
//! are not modified. It is not a menu leaf: the
//! reconstruction leaf also builds the closed crowns, which is a later stage.

use dc_case::{CaseState, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::teeth::{self, Which};
use dca_model::{AlignersState, MODULE_ID};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.teeth.frames",
        label: "aligners-cmd-teeth-frames",
        doc: "aligners-cmd-teeth-frames-doc",
        params: dca_algo::teeth::Params,
        run: run_frames,
    }]
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn run_frames(
    state: &CaseState,
    params: teeth::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data)?,
    };
    let explicit = params.arch != Which::All;
    let mut next = state.clone();
    let mut report = Map::new();

    for arch in params.arch.arches() {
        let role: ScanRole = arch.role();
        let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
            if explicit {
                return Err(crate::commands::messages::not_segmented(arch));
            }
            continue;
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
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();

        // the arch frame of this arch (the one the seeds define): its z is occlusal
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
        let vertices: Vec<_> = case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect();
        ctx.check_cancelled()?;

        let out = teeth::run(
            &teeth::Input {
                arch,
                vertices: &vertices,
                tris: &mesh.tris,
                labels,
            },
            &params,
        )
        .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
        ctx.check_cancelled()?;

        data.tooth_frames
            .retain(|f, _| Arch::of(*f) != arch || out.teeth.contains_key(f));
        data.tooth_geometry
            .retain(|f, _| Arch::of(*f) != arch || out.teeth.contains_key(f));
        let mut measures = Map::new();
        for (fdi, tooth) in &out.teeth {
            let tooth = tooth.transformed(&to_case);
            data.tooth_frames.insert(*fdi, teeth::tooth_frame(&tooth));
            data.tooth_geometry
                .insert(*fdi, dca_algo::diagnosis::tooth_measures(&tooth));
            measures.insert(
                fdi.to_string(),
                serde_json::to_value(&tooth).map_err(|e| CommandError::Failed(e.to_string()))?,
            );
        }
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "frame_source": framed.source,
                "sequence": out.sequence,
                "teeth": measures,
            }),
        );
    }
    if report.is_empty() {
        return Err(crate::commands::messages::none_segmented());
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::Fdi;
    use serde_json::json;

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find("aligners.teeth.frames")
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo_state(state: &CaseState, arch: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(state, &json!({ "arch": arch }), &JobCtx::detached())
            .unwrap()
            .0
    }

    fn frames(state: &CaseState) -> std::collections::BTreeMap<Fdi, dca_model::ToothFrame> {
        state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .tooth_frames
            .clone()
    }

    #[test]
    fn the_command_is_registered_documented_and_not_a_menu_leaf() {
        let spec = crate::commands::find("aligners.teeth.frames").unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["arch"].is_object());
        assert!(crate::tree::find("aligners.teeth.frames").is_none());
    }

    #[test]
    fn an_empty_or_unsegmented_case_has_no_frames() {
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("tooth colours")
        ));
        let lower = demo_state(&CaseState::default(), "lower");
        let err = run(&lower, json!({ "arch": "upper" })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("upper")));
        // labels that do not belong to the scan
        let mut broken = lower.clone();
        let mut data = broken
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        let mut short = (**broken.mesh(&data.labels[&ScanRole::Lower]).unwrap()).clone();
        short.vertex_labels.as_mut().unwrap().pop();
        data.labels
            .insert(ScanRole::Lower, broken.blobs.insert(short));
        broken.set_module(MODULE_ID, ModuleState::new(data));
        assert!(matches!(
            run(&broken, Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("do not match")
        ));
        assert!(matches!(
            run(&lower, json!({ "arch": "sideways" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_demo_arches_get_a_frame_and_widths_per_tooth() {
        for arch in ["lower", "upper"] {
            let state = demo_state(&CaseState::default(), arch);
            let before = frames(&state);
            assert!(before.is_empty());
            let (next, out) = run(&state, Value::Null).unwrap();
            let stored = frames(&next);
            let report = &out[arch];
            assert_eq!(report["frame_source"], "seeds");
            assert_eq!(report["sequence"].as_array().unwrap().len(), 14);
            assert_eq!(stored.len(), 14);
            assert!(
                out.get(if arch == "lower" { "upper" } else { "lower" })
                    .is_none()
            );
            for (fdi, frame) in &stored {
                let t = &report["teeth"][fdi.to_string()];
                let md = t["width_md"].as_f64().unwrap();
                let bl = t["width_bl"].as_f64().unwrap();
                assert!(
                    (3.0..14.0).contains(&md) && (3.0..14.0).contains(&bl),
                    "{fdi}"
                );
                // the stored pose is a rotation with the tooth axes, in case coordinates
                let q = frame.pose.q;
                assert!((q.length() - 1.0).abs() < 1e-9);
                let mut ex = frame.pose.t * 0.0;
                ex.x = 1.0;
                let mut ez = frame.pose.t * 0.0;
                ez.z = 1.0;
                let x = frame.pose.apply_vector(ex);
                let axes = &t["e_md"];
                assert!(
                    (axes[0].as_f64().unwrap() - x.x).abs() < 1e-9
                        && (axes[2].as_f64().unwrap() - x.z).abs() < 1e-9,
                    "{fdi}"
                );
                // the long axis points to the occlusal side: up for both demo arches
                let z = frame.pose.apply_vector(ez);
                assert!(z.z > 0.5, "{fdi}: {z:?}");
                // the centre of resistance lies below the crown reference, in the root
                assert!(frame.center_of_resistance.z < -3.0, "{fdi}");
                // the cusp tip is above it
                let tip_z = t["tip"][2].as_f64().unwrap();
                let cres = frame.pose.apply_point(frame.center_of_resistance);
                assert!(tip_z > cres.z + 5.0, "{fdi}");
            }
            // nothing else changed
            assert_eq!(next.case.scans, state.case.scans);
            let (a, b) = (
                state.module_data::<AlignersState>(MODULE_ID).unwrap(),
                next.module_data::<AlignersState>(MODULE_ID).unwrap(),
            );
            assert_eq!((&a.seeds, &a.labels), (&b.seeds, &b.labels));
            assert_eq!(a.setup, b.setup);
            // a second run gives the same frames
            let (again, again_out) = run(&next, Value::Null).unwrap();
            assert_eq!(frames(&again), stored);
            assert_eq!(again_out, out);
        }
    }

    #[test]
    fn both_arches_are_measured_and_one_can_be_redone_alone() {
        let both = demo_state(&demo_state(&CaseState::default(), "lower"), "upper");
        let (all, out) = run(&both, Value::Null).unwrap();
        assert!(out["lower"].is_object() && out["upper"].is_object());
        let stored = frames(&all);
        assert_eq!(stored.len(), 28);
        // only the upper arch again: the lower frames stay as they are
        let mut stale = all.clone();
        let mut data = stale
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        let f11 = Fdi::new(11).unwrap();
        let f41 = Fdi::new(41).unwrap();
        for f in [f11, f41] {
            data.tooth_frames.get_mut(&f).unwrap().pose.t.x += 9.0;
        }
        stale.set_module(MODULE_ID, ModuleState::new(data));
        let (redone, out) = run(&stale, json!({ "arch": "upper" })).unwrap();
        assert!(out.get("lower").is_none());
        let now = frames(&redone);
        assert_eq!(now[&f11], stored[&f11]);
        assert_ne!(now[&f41], stored[&f41]);
        assert_eq!(now.len(), 28);
    }
}
