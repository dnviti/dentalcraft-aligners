//! Commands of the `gingiva` stage (`dca_algo::gingiva`).
//!
//! `aligners.preparation.gingiva` builds the gingiva without holes of every
//! segmented arch (or the one arch asked for): the scan, in the arch frame
//! its seeds define, cut along the smoothed margin of all the teeth, with a
//! harmonic membrane under the crowns, then closed with the flat base the
//! import gives an open scan (`dca_algo::scan_base::make_base`, 3 mm under
//! its lowest point along the occlusal axis of the arch frame), so that the
//! model shown with the 3D teeth is a closed solid. It is stored as a blob in
//! `AlignersState::gingiva` (case coordinates; the vertex labels hold the
//! tooth each vertex moves with and the margin, membrane, col and base flags,
//! from which `dca_algo::gingiva::gum_weights` computes the blend weights
//! that move it with the teeth; the base stays still). The scans are not
//! modified.

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::gingiva;
use dca_algo::scan_base::HEIGHT_DEFAULT_MM;
use dca_model::MODULE_ID;
use serde_json::{Map, Value, json};

use super::reconstruct::{aligners_data, arch_name, arch_scan};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.preparation.gingiva",
        label: "aligners-cmd-gingiva",
        doc: "aligners-cmd-gingiva-doc",
        params: dca_algo::gingiva::Params,
        run: run_gingiva,
    }]
}

fn run_gingiva(
    state: &CaseState,
    params: gingiva::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let explicit = params.arch != dca_algo::teeth::Which::All;
    let mut next = state.clone();
    let mut out = Map::new();
    let fraction = super::auto::step_fraction(ctx);
    let arches = params.arch.arches();
    for (k, &arch) in arches.iter().enumerate() {
        let Some(scan) = arch_scan(state, &data, arch)? else {
            if explicit {
                return Err(crate::commands::messages::not_segmented(arch));
            }
            continue;
        };
        let t0 = std::time::Instant::now();
        // the bar moves as the stages of the gum end, inside the part of this arch
        let (k, n) = (k as f64, arches.len() as f64);
        let progress = |x: f32| ctx.set_fraction(fraction((k + f64::from(x)) / n));
        let mut g = gingiva::gum_surface_with_progress(
            &scan.vertices,
            &scan.tris,
            &scan.labels,
            &params,
            &progress,
            &|| ctx.is_cancelled(),
        )
        .map_err(|e| match e {
            gingiva::GingivaError::Cancelled => CommandError::Cancelled,
            e => CommandError::Failed(format!("{} arch: {e}", arch_name(arch))),
        })?;
        ctx.check_cancelled()?;
        // closed with the flat base, in the arch frame (occlusal +Z)
        let base_edges = g
            .close_with_base(HEIGHT_DEFAULT_MM)
            .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
        for p in &mut g.vertices {
            *p = scan.to_case.transform_point3(*p);
        }
        let count = |m: &[bool]| m.iter().filter(|&&x| x).count();
        let report = json!({
            "vertices": g.vertices.len(),
            "faces": g.tris.len(),
            "margin_vertices": count(&g.margin),
            "membrane_vertices": count(&g.membrane),
            "col_vertices": count(&g.col),
            "base_vertices": count(&g.base),
            "base_edges": base_edges,
            "seconds": t0.elapsed().as_secs_f64(),
        });
        let blob = next.blobs.insert(g.to_mesh());
        data.gingiva.insert(arch.role(), blob);
        out.insert(arch_name(arch).to_owned(), report);
    }
    if out.is_empty() {
        return Err(crate::commands::messages::none_segmented());
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::{Fdi, ScanRole};
    use dca_model::AlignersState;
    use glam::{DMat4, DVec3};
    use std::collections::BTreeMap;

    const ID: &str = "aligners.preparation.gingiva";

    fn run(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_command_is_for_the_cli_and_needs_teeth_and_gum_separated() {
        assert!(crate::tree::find(ID).is_none());
        assert_eq!(
            crate::commands::find(ID).unwrap().schema()["type"],
            "object"
        );
        assert!(matches!(
            run(ID, &CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
        let lower = run(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0;
        assert!(matches!(
            run(ID, &lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper")
        ));
    }

    #[test]
    fn the_gingiva_of_a_scan_is_stored_and_follows_a_moved_tooth() {
        let state = super::super::reconstruct::golden_state("demo_lower_mild");
        let (next, out) = run(ID, &state, Value::Null).unwrap();
        assert!(out.get("upper").is_none());
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let blob = data.gingiva[&ScanRole::Lower];
        assert!(next.referenced_blobs().contains(&blob));
        let mesh = next.mesh(&blob).unwrap();
        let scan = next
            .mesh(&next.case.scan(ScanRole::Lower).unwrap().mesh)
            .unwrap();
        // the plugin's numbers on this scan (dca-algo tests/fixtures/gingiva.json)
        assert_eq!(mesh.vertex_count(), 80245);
        assert_eq!(mesh.face_count(), 160530);
        assert_eq!(out["lower"]["membrane_vertices"], 27512);
        assert_eq!(out["lower"]["col_vertices"], 37);
        let g = gingiva::GumSurface::from_mesh(mesh).unwrap();
        let margin = g.margin.iter().filter(|&&m| m).count();
        assert_eq!(margin, 2245);
        assert_eq!(margin, mesh.vertex_count() - scan.vertex_count());
        // the scan vertices off the membrane are where they were (case coordinates)
        for i in (0..scan.vertex_count()).step_by(97) {
            if !g.membrane[i] {
                assert!(
                    (mesh.positions[i] - scan.positions[i]).length() < 1e-4,
                    "{i}"
                );
            }
        }
        // weights and a rigid motion of tooth 41: its margin follows it exactly
        let f41 = Fdi::new(41).unwrap();
        let mut fdis: Vec<Fdi> = g
            .tooth
            .iter()
            .filter_map(|&t| u8::try_from(t).ok().and_then(|x| Fdi::new(x).ok()))
            .collect();
        fdis.sort_unstable();
        fdis.dedup();
        assert_eq!(fdis.len(), 14);
        let w = gingiva::gum_weights(&g, &fdis, &gingiva::Params::default());
        let shift = DVec3::new(0.5, 0.0, 0.0);
        let deltas = BTreeMap::from([(f41, DMat4::from_translation(shift))]);
        let moved = gingiva::deform(&g.vertices, &gingiva::deform_prep(&w), &deltas);
        let j = fdis.iter().position(|&f| f == f41).unwrap();
        let mut rigid = 0;
        for (i, (m, v)) in moved.iter().zip(&g.vertices).enumerate() {
            let d = *m - *v;
            assert!(d.length() <= shift.length() + 1e-9);
            if g.margin[i] && g.tooth[i] == 41 && w.get(i, j) == 1.0 {
                assert!((d - shift).length() < 1e-9);
                rigid += 1;
            }
        }
        assert!(rigid > 10, "{rigid}");
        // far from the tooth nothing moves
        let still = (0..g.vertices.len())
            .filter(|&i| w.get(i, j) == 0.0 && moved[i] == g.vertices[i])
            .count();
        assert!(still > g.vertices.len() / 2, "{still}");
    }

    #[test]
    fn cancelling_the_gingiva_returns_cancelled_and_stores_nothing() {
        let state = super::super::reconstruct::golden_state("demo_lower_mild");
        let before = state.module_data::<AlignersState>(MODULE_ID).cloned();
        let r = super::super::reconstruct::run_cancelled_at_first_progress(ID, &state, Value::Null);
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
