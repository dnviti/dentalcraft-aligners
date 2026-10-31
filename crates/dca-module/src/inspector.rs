//! The inspector of the Aligners module: title and description of the
//! selected menu entry, a "not available yet" notice for placeholders and,
//! for the staging, the layout of the future stage table and limits.

use dc_i18n::tr;
use dc_module_api::MenuStatus;
use dca_model::StagingSettings;
use egui::{Grid, RichText, Slider, Ui};

use crate::tree::{self, Node};

/// Id of the staging entry; the staging layout is shown for it and for all
/// the entries below it.
pub const STAGING_ID: &str = "aligners.planning.staging";

/// Draws the inspector for the entry `selected` (the overview when `None`).
/// `staging` holds the values of the limit sliders.
pub fn show(ui: &mut Ui, selected: Option<&str>, staging: &mut StagingSettings) {
    let Some(node) = selected.and_then(tree::find) else {
        ui.heading(tr("aligners-name"));
        ui.label(tr("aligners-select-hint"));
        return;
    };
    ui.heading(tr(node.label_key));
    ui.label(tr(node.desc_key));
    if node.status() == MenuStatus::Stub {
        ui.add_space(6.0);
        ui.label(RichText::new(tr("aligners-status-stub")).strong());
        ui.weak(tr("aligners-status-stub-detail"));
    }
    if is_staging(node) {
        ui.add_space(8.0);
        ui.separator();
        staging_layout(ui, staging);
    }
}

fn is_staging(node: &Node) -> bool {
    node.id == STAGING_ID || node.id.starts_with("aligners.planning.staging.")
}

/// Placeholder of the staging page: the (empty) stage table and the limits
/// per aligner, with the defaults of the old plugin's `staging.py` and
/// `evidence.py`. Disabled until the staging is implemented.
fn staging_layout(ui: &mut Ui, s: &mut StagingSettings) {
    ui.add_enabled_ui(false, |ui| {
        ui.strong(tr("aligners-staging-stages"));
        Grid::new("aligners-staging-stages-grid")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for key in [
                    "aligners-staging-col-stage",
                    "aligners-staging-col-teeth",
                    "aligners-staging-col-max-translation",
                    "aligners-staging-col-max-rotation",
                ] {
                    ui.strong(tr(key));
                }
                ui.end_row();
                ui.label("-");
                ui.label("-");
                ui.label("- mm");
                ui.label("- °");
                ui.end_row();
            });
        ui.label(tr("aligners-staging-no-stages"));

        ui.add_space(8.0);
        ui.strong(tr("aligners-staging-limits"));
        limit(
            ui,
            "aligners-staging-step-linear",
            &mut s.step_linear_mm,
            0.05..=0.5,
        );
        limit(
            ui,
            "aligners-staging-step-vertical",
            &mut s.step_vertical_mm,
            0.05..=0.5,
        );
        limit(
            ui,
            "aligners-staging-step-rotation",
            &mut s.step_rotation_deg,
            0.5..=4.0,
        );
        limit(
            ui,
            "aligners-staging-step-rotation-round",
            &mut s.step_rotation_round_deg,
            0.5..=4.0,
        );
        limit(
            ui,
            "aligners-staging-step-angular",
            &mut s.step_angular_deg,
            0.25..=2.0,
        );
        limit(
            ui,
            "aligners-staging-max-crown-step",
            &mut s.max_crown_step_mm,
            0.1..=0.5,
        );
    });
}

fn limit(ui: &mut Ui, key: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>) {
    ui.add(Slider::new(value, range).text(tr(key)));
}
