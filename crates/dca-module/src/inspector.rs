//! The inspector of the Aligners module: title and description of the
//! selected menu entry, a "not available yet" notice for placeholders and,
//! for the staging, the layout of the future stage table and limits; for the
//! study models, their base settings (stored in the case through the
//! `aligners.settings.study-models` command).

use dc_i18n::tr;
use dc_module_api::{MenuStatus, ModuleCtx};
use dca_model::{AlignersState, MODULE_ID, StagingSettings, StudyBaseSettings, StudyBaseStyle};
use egui::{Button, DragValue, Grid, RichText, Slider, Ui};

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

/// The study base settings being edited: the case values they started from
/// and the edited values.
pub type StudyEdit = Option<(StudyBaseSettings, StudyBaseSettings)>;

/// Settings of the study-model bases (B6): style and main parameters,
/// applied to the case as one undoable command.
pub fn study_settings(ui: &mut Ui, ctx: &mut ModuleCtx<'_>, edit: &mut StudyEdit) {
    let Some(current) = ctx
        .state()
        .module_data::<AlignersState>(MODULE_ID)
        .map(|d| d.settings.study_base.clone())
    else {
        ui.add_space(8.0);
        ui.weak(tr("aligners-study-no-case"));
        return;
    };
    let (base, s) = edit.get_or_insert_with(|| (current.clone(), current.clone()));
    if *base != current {
        // the case changed (apply, undo, another case): follow it unless edited
        if s == base {
            *s = current.clone();
        }
        *base = current.clone();
    }
    ui.add_space(8.0);
    ui.separator();
    ui.strong(tr("aligners-study-settings"));
    Grid::new("aligners-study-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label(tr("aligners-study-style"));
            let style_key = |st: StudyBaseStyle| match st {
                StudyBaseStyle::European => "aligners-study-style-european",
                StudyBaseStyle::Abo => "aligners-study-style-abo",
            };
            egui::ComboBox::from_id_salt("aligners-study-style")
                .selected_text(tr(style_key(s.style)))
                .show_ui(ui, |ui| {
                    for st in [StudyBaseStyle::European, StudyBaseStyle::Abo] {
                        ui.selectable_value(&mut s.style, st, tr(style_key(st)));
                    }
                });
            ui.end_row();
            let abo = s.style == StudyBaseStyle::Abo;
            let rows: [(&str, &mut f64, std::ops::RangeInclusive<f64>, bool); 9] = [
                (
                    "aligners-study-total-height",
                    &mut s.total_height_mm,
                    40.0..=120.0,
                    abo,
                ),
                (
                    "aligners-study-art-fraction",
                    &mut s.art_fraction,
                    0.1..=0.6,
                    abo,
                ),
                (
                    "aligners-study-base-thickness",
                    &mut s.base_thickness_mm,
                    3.0..=30.0,
                    !abo,
                ),
                ("aligners-study-m-base", &mut s.m_base_mm, 0.0..=10.0, true),
                (
                    "aligners-study-back-margin",
                    &mut s.back_margin_mm,
                    0.0..=10.0,
                    true,
                ),
                (
                    "aligners-study-side-angle",
                    &mut s.side_angle_deg,
                    30.0..=90.0,
                    abo,
                ),
                (
                    "aligners-study-heel-angle",
                    &mut s.heel_angle_deg,
                    91.0..=170.0,
                    true,
                ),
                (
                    "aligners-study-side-clear",
                    &mut s.side_clear_mm,
                    0.0..=10.0,
                    abo,
                ),
                (
                    "aligners-study-vestibule-clear",
                    &mut s.vestibule_clear_mm,
                    0.0..=10.0,
                    !abo,
                ),
            ];
            for (key, value, range, enabled) in rows {
                ui.add_enabled(enabled, egui::Label::new(tr(key)));
                ui.add_enabled(enabled, DragValue::new(value).range(range).speed(0.1));
                ui.end_row();
            }
        });
    let changed = *s != current;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(changed, Button::new(tr("aligners-study-apply")))
            .clicked()
            && let (Some(spec), Ok(params)) = (
                crate::commands::find(crate::commands::study_base::SETTINGS_ID),
                serde_json::to_value(&*s),
            )
        {
            ctx.queue(Box::new(spec.command(params)));
        }
        if ui
            .add_enabled(changed, Button::new(tr("aligners-study-reset")))
            .clicked()
        {
            *s = current.clone();
        }
    });
}

/// `true` for the study-models entry.
pub fn is_study_models(selected: Option<&str>) -> bool {
    selected == Some(crate::commands::study_base::STUDY_MODELS_ID)
}
