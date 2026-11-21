//! The inspector of "Print" (`aligners.production.print`): how many aligners
//! are printed and how many are still to print, and the list of the aligners
//! with their status and the marks the user may give (printed, failed, to
//! print again), each one undoable command (`aligners.print.mark`). The
//! shell draws the rest: the button that opens the print window runs the
//! leaf, whose output lists the aligners to print.

use dc_i18n::{tr, tr_args};
use dc_module_api::ModuleCtx;
use dca_algo::teeth::Which;
use dca_model::{AlignersState, MODULE_ID, MarkSource, PrintStatus, print_transition};
use egui::{RichText, ScrollArea, Ui};
use serde_json::json;

use crate::commands::print::{MARK, series, status_key, utc_text};

/// The marks the user may give, with the i18n key of their action.
const MARKS: [(PrintStatus, &str); 3] = [
    (PrintStatus::Printed, "aligners-print-mark-printed"),
    (PrintStatus::Failed, "aligners-print-mark-failed"),
    (PrintStatus::Reprint, "aligners-print-mark-reprint"),
];

/// Draws the tracking of the aligners of the case.
pub fn show(ui: &mut Ui, ctx: &mut ModuleCtx<'_>) {
    let Some(data) = ctx.state().module_data::<AlignersState>(MODULE_ID) else {
        ui.weak(tr("aligners-print-no-aligners"));
        return;
    };
    let keys = series(data, Which::All);
    if keys.is_empty() {
        ui.weak(tr("aligners-print-no-aligners"));
        return;
    }
    let printed = keys
        .iter()
        .filter(|k| data.print_status(**k) == Some(PrintStatus::Printed))
        .count();
    ui.label(
        RichText::new(tr_args(
            "aligners-print-count",
            &[
                ("printed", &printed.to_string()),
                ("total", &keys.len().to_string()),
            ],
        ))
        .strong(),
    );
    ui.label(tr_args(
        "aligners-print-missing",
        &[("count", &(keys.len() - printed).to_string())],
    ));
    if data.export_dir.is_none() {
        ui.weak(tr("aligners-print-no-export"));
    }
    ui.add_space(6.0);
    let details = dc_module_api::advanced_ui(ui.ctx());
    let mut marks: Vec<(String, PrintStatus)> = Vec::new();
    ScrollArea::vertical()
        .id_salt("aligners-print-list")
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for key in &keys {
                let status = data.print_status(*key);
                let id = key.to_string();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&id).monospace());
                    let name = RichText::new(tr(status_key(status)));
                    let name = match status {
                        Some(PrintStatus::Printed) => name.strong(),
                        Some(PrintStatus::Failed | PrintStatus::Reprint) => {
                            name.color(ui.visuals().warn_fg_color)
                        }
                        None => name.weak(),
                        _ => name,
                    };
                    ui.label(name);
                    let allowed: Vec<_> = MARKS
                        .iter()
                        .filter(|(to, _)| print_transition(status, *to, MarkSource::User))
                        .collect();
                    if !allowed.is_empty() {
                        ui.menu_button(tr("aligners-print-mark"), |ui| {
                            for (to, label) in allowed {
                                if ui.button(tr(label)).clicked() {
                                    marks.push((id.clone(), *to));
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                if details && let Some(r) = data.prints.get(key) {
                    let job = r.job.as_deref().unwrap_or("–");
                    ui.weak(format!("{}  {job}", utc_text(r.at)));
                }
            }
        });
    for (id, status) in marks {
        if let Some(spec) = crate::commands::find(MARK) {
            let params = json!({ "ids": [id], "status": status, "by": MarkSource::User });
            ctx.queue(Box::new(spec.command(params)));
        }
    }
}
