//! The parameters the module draws in the inspector fit its narrowest width
//! (the shell's inspector is resizable from 300 pt and clips what overflows).

use std::sync::Arc;

use dc_case::{Case, CaseState, ModuleState, Scan, ScanRole};
use dc_i18n::Lang;
use dc_jobs::JobSystem;
use dc_mesh::TriMesh;
use dc_module_api::{Module, ModuleCtx};
use dca_model::AlignersState;
use dca_module::{AlignersModule, MODULE_ID, loader};
use egui::{Rect, pos2, vec2};
use glam::Vec3;
use serde_json::json;

/// Width of the content of the inspector at its narrowest (300 pt less its
/// margins), with some room to spare.
const NARROW: f32 = 260.0;

/// A case of the module with an upper scan: "Find the teeth" shows its grid
/// of the teeth, the study models their settings.
fn case_with_upper_scan() -> CaseState {
    let mut s = CaseState::new(Case::new("T"));
    let mesh = TriMesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![[0, 1, 2]]);
    let blob = s.blobs.insert(mesh);
    let scan: Scan = serde_json::from_value(json!({
        "id": "de300000-0000-4000-8000-000000000001",
        "role": ScanRole::Upper,
        "source_name": "upper.stl",
        "source_units": "mm",
        "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
        "mesh": blob,
    }))
    .unwrap();
    s.case.scans.push(scan);
    s.set_module(MODULE_ID, ModuleState::new(AlignersState::default()));
    s
}

/// The right edge of everything the inspector of `leaf` draws in a column
/// `width` wide (a few frames: a grid sizes its columns from the one before).
fn right_edge(leaf: &str, width: f32) -> f32 {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(case_with_upper_scan());
    let mut module = AlignersModule::new();
    assert!(module.select_entry(leaf), "unknown item {leaf}");
    let egui_ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 4000.0))),
        ..Default::default()
    };
    let mut right = 0.0f32;
    for _ in 0..3 {
        let mut out = egui_ctx.run_ui(input.clone(), |ui| {
            let mut ctx = ModuleCtx::new(&state, &jobs);
            module.inspector(ui, &mut ctx);
        });
        out.textures_delta.clear();
        right = out
            .shapes
            .iter()
            .map(|s| s.shape.visual_bounding_rect())
            .filter(|r| r.is_positive())
            .fold(0.0, |m, r| m.max(r.right()));
    }
    right
}

#[test]
fn the_parameters_fit_the_narrowest_inspector_in_both_languages() {
    loader();
    for lang in [Lang::It, Lang::En] {
        dc_i18n::set_language(lang).unwrap();
        for leaf in ["aligners.teeth.find", "aligners.output.study-models"] {
            let right = right_edge(leaf, NARROW);
            assert!(right > 0.0, "{lang:?} {leaf}: nothing drawn");
            assert!(
                right <= NARROW + 0.5,
                "{lang:?} {leaf}: drawn up to {right} in {NARROW}"
            );
        }
    }
}
