//! "Find the teeth" in the inspector: the parameters of the leaf, which the
//! shell shows in its section "Parameters", and what has been found
//! (`page_numeri` of the plugin).
//!
//! * the box "Search also the third molars" (`include_third_molars`);
//! * the field "Missing teeth" (`missing_teeth`): FDI numbers separated by
//!   commas, validated while typing and applied when the field is left;
//! * with the advanced options on, a grid with a cell for each tooth, 8
//!   columns per arch (4 when the inspector is too narrow for 8): filled when
//!   the tooth has a mark, crossed when it is missing, empty otherwise; with
//!   them off, one line with how many teeth were found and are missing.
//!
//! Both options are stored in the case (`Settings::find_teeth`, through the
//! command `aligners.settings.find-teeth`, one undo step each) and are the
//! parameters `aligners.teeth.find` gets when the leaf is run from its
//! inspector ([`leaf_params`]).

use dc_case::{CaseState, Fdi, ScanRole};
use dc_i18n::{tr, tr_args};
use dc_module_api::ModuleCtx;
use dca_algo::anatomy::{Arch, arch_order, index};
use dca_model::{AlignersState, FindTeethSettings, MODULE_ID};
use egui::{Align2, FontId, Grid, RichText, Role, Sense, TextEdit, Ui, WidgetInfo, vec2};
use serde_json::{Value, json};

use crate::commands::autoseed::{FIND_SETTINGS, permanent};

/// What the user has typed in the field of the missing teeth.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FindEdit {
    /// The list of the case the text started from.
    base: Vec<u8>,
    /// The text of the field.
    text: String,
}

/// The settings of the case (the defaults when it has none yet).
pub fn settings_of(state: &CaseState) -> FindTeethSettings {
    state
        .module_data::<AlignersState>(MODULE_ID)
        .map(|d| d.settings.find_teeth.clone())
        .unwrap_or_default()
}

/// The parameters of `aligners.teeth.find` when its leaf is run: the settings
/// of the case.
pub fn leaf_params(state: &CaseState) -> Value {
    settings_of(state).leaf_params()
}

/// The FDI numbers of a list typed by the user: separated by commas,
/// semicolons or spaces, sorted, without repetitions. `Err` holds the entries
/// that are not the number of a permanent tooth, as typed.
pub fn parse_missing(text: &str) -> Result<Vec<u8>, Vec<String>> {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for tok in text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        match tok.parse::<u8>() {
            Ok(n) if permanent(n) => ok.push(n),
            _ => bad.push(tok.to_owned()),
        }
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    ok.sort_unstable();
    ok.dedup();
    Ok(ok)
}

/// The list as it is written in the field: `35, 45`.
pub fn format_missing(list: &[u8]) -> String {
    list.iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a cell of the grid says about a tooth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// The tooth has a mark.
    Marked,
    /// The tooth is known to be missing.
    Missing,
    /// Nothing yet.
    Empty,
}

/// The cells of the grid of `arch`, in rows of 8 (the right side, then the
/// left side of the patient). `None` is the place of a third molar that is
/// not searched for.
pub fn grid(
    arch: Arch,
    third_molars: bool,
    marked: &[Fdi],
    missing: &[u8],
) -> Vec<Option<(Fdi, Cell)>> {
    arch_order(arch)
        .into_iter()
        .map(|f| {
            if index(f) == 8 && !third_molars {
                return None;
            }
            let cell = if missing.contains(&f.get()) {
                Cell::Missing
            } else if marked.contains(&f) {
                Cell::Marked
            } else {
                Cell::Empty
            };
            Some((f, cell))
        })
        .collect()
}

fn queue_settings(ctx: &mut ModuleCtx<'_>, patch: Value) {
    if let Some(spec) = crate::commands::find(FIND_SETTINGS) {
        ctx.queue(Box::new(spec.command(patch)));
    }
}

fn cell_text(f: Fdi, cell: Cell) -> String {
    let key = match cell {
        Cell::Marked => "aligners-find-cell-marked",
        Cell::Missing => "aligners-find-cell-missing",
        Cell::Empty => "aligners-find-cell-empty",
    };
    tr_args(key, &[("fdi", &f.get().to_string())])
}

/// The options and the grid, in the section "Parameters" of the leaf.
pub fn show(ui: &mut Ui, ctx: &mut ModuleCtx<'_>, edit: &mut FindEdit) {
    let advanced = dc_module_api::advanced_ui(ui.ctx());
    let state = ctx.state().clone();
    let current = settings_of(&state);
    // the case changed (undo, another case): follow it unless the text was edited
    if edit.base != current.missing {
        if edit.text == format_missing(&edit.base) {
            edit.text = format_missing(&current.missing);
        }
        edit.base.clone_from(&current.missing);
    }

    let mut third = current.third_molars;
    if ui.checkbox(&mut third, tr("aligners-find-third")).changed() {
        queue_settings(ctx, json!({ "third_molars": third }));
    }

    ui.add_space(4.0);
    let label = ui.label(tr("aligners-find-missing"));
    let parsed = parse_missing(&edit.text);
    // the label names the field for screen readers (and the tests)
    let field = ui
        .add(
            TextEdit::singleline(&mut edit.text)
                .id_salt("aligners-find-missing")
                .hint_text(tr("aligners-find-missing-hint"))
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
    match &parsed {
        Err(bad) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                tr_args("aligners-find-missing-error", &[("list", &bad.join(", "))]),
            );
        }
        Ok(_) if advanced => {
            ui.weak(tr("aligners-find-missing-detail"));
        }
        Ok(_) => {}
    }
    if field.lost_focus()
        && let Ok(list) = parsed
    {
        edit.text = format_missing(&list);
        if list != current.missing {
            queue_settings(ctx, json!({ "missing": list }));
        }
    }

    // what was found, per arch
    let (marked, missing_case): (Vec<Fdi>, Vec<u8>) = state
        .module_data::<AlignersState>(MODULE_ID)
        .map(|d| {
            (
                d.seeds.iter().map(|s| s.fdi).collect(),
                d.missing.iter().map(|f| f.get()).collect(),
            )
        })
        .unwrap_or_default();
    let mut missing = current.missing.clone();
    missing.extend(missing_case);
    missing.sort_unstable();
    missing.dedup();
    ui.add_space(8.0);
    if advanced {
        // with the advanced options off the summary line names it
        ui.strong(tr("aligners-find-found"));
    }
    let arches: Vec<Arch> = [
        (ScanRole::Upper, Arch::Upper),
        (ScanRole::Lower, Arch::Lower),
    ]
    .into_iter()
    .filter(|(role, _)| state.case.scan(*role).is_some())
    .map(|(_, arch)| arch)
    .collect();
    if arches.is_empty() {
        ui.weak(tr("aligners-find-no-scan"));
        return;
    }
    if !advanced {
        ui.label(tr_args(
            "aligners-find-summary",
            &[
                ("n", &marked.len().to_string()),
                ("missing", &missing.len().to_string()),
            ],
        ));
        return;
    }
    let (cols, cell_w) = grid_layout(ui.available_width());
    for arch in arches {
        let cells = grid(arch, third, &marked, &missing);
        ui.add_space(4.0);
        ui.label(RichText::new(tr(if arch.is_upper() {
            "aligners-find-upper"
        } else {
            "aligners-find-lower"
        })));
        Grid::new(("aligners-find-grid", arch.is_upper()))
            .num_columns(cols)
            .min_col_width(cell_w)
            .spacing([CELL_GAP, 4.0])
            .show(ui, |ui| {
                for (i, c) in cells.iter().enumerate() {
                    match c {
                        Some((f, cell)) => draw_cell(ui, *f, *cell, cell_w),
                        None => {
                            ui.allocate_space(vec2(cell_w, CELL_H));
                        }
                    }
                    if i % cols == cols - 1 {
                        ui.end_row();
                    }
                }
            });
    }
    let n = marked.len();
    ui.add_space(4.0);
    ui.weak(tr_args("aligners-find-count", &[("n", &n.to_string())]));
    ui.weak(tr("aligners-find-legend"));
}

/// Widest cell ...
const CELL_W: f32 = 36.0;
/// ... narrowest one (the box and two digits) ...
const CELL_MIN_W: f32 = 26.0;
/// ... and the space between two.
const CELL_GAP: f32 = 6.0;
const CELL_H: f32 = 18.0;

/// Columns of the grid and width of a cell in `width`: 8 columns (a side of
/// the arch a row) when their cells are at least [`CELL_MIN_W`] wide, else 4
/// (a side on two rows).
fn grid_layout(width: f32) -> (usize, f32) {
    let cell = |cols: usize| (width - CELL_GAP * (cols - 1) as f32) / cols as f32;
    let cols = if cell(8) >= CELL_MIN_W { 8 } else { 4 };
    (cols, cell(cols).clamp(CELL_MIN_W, CELL_W))
}

/// One cell `w` wide: a box (filled, crossed or empty) and the number of the
/// tooth.
fn draw_cell(ui: &mut Ui, f: Fdi, cell: Cell, w: f32) {
    let text = cell_text(f, cell);
    let (rect, response) = ui.allocate_exact_size(vec2(w, CELL_H), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text.clone()));
    let v = ui.visuals();
    let box_rect = egui::Rect::from_min_size(rect.left_center() - vec2(0.0, 5.0), vec2(10.0, 10.0));
    let (fill, line) = match cell {
        Cell::Marked => (v.selection.bg_fill, v.selection.stroke.color),
        Cell::Missing => (v.faint_bg_color, v.warn_fg_color),
        Cell::Empty => (v.extreme_bg_color, v.widgets.noninteractive.fg_stroke.color),
    };
    let painter = ui.painter();
    painter.rect(
        box_rect,
        2.0,
        fill,
        egui::Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    if cell == Cell::Missing {
        let s = egui::Stroke::new(1.5, line);
        painter.line_segment([box_rect.left_top(), box_rect.right_bottom()], s);
        painter.line_segment([box_rect.left_bottom(), box_rect.right_top()], s);
    }
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        f.get().to_string(),
        FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
    response.on_hover_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_takes_fdi_numbers_in_any_separator() {
        assert_eq!(parse_missing(""), Ok(vec![]));
        assert_eq!(parse_missing(" 45 ,35;  35 18"), Ok(vec![18, 35, 45]));
        assert_eq!(format_missing(&[18, 35, 45]), "18, 35, 45");
        assert_eq!(format_missing(&[]), "");
    }

    #[test]
    fn invalid_entries_are_named_not_dropped() {
        for text in ["9", "49", "19", "0", "300", "x", "35, 4a", "-1", "51"] {
            assert!(parse_missing(text).is_err(), "{text}");
        }
        assert_eq!(
            parse_missing("35, 99, abc, 45"),
            Err(vec!["99".to_owned(), "abc".to_owned()])
        );
    }

    #[test]
    fn the_grid_is_two_rows_of_eight_with_the_right_distal_first() {
        let f = |n| Fdi::new(n).unwrap();
        let cells = grid(Arch::Lower, true, &[f(46), f(31)], &[45]);
        assert_eq!(cells.len(), 16);
        let nums: Vec<u8> = cells.iter().flatten().map(|(f, _)| f.get()).collect();
        assert_eq!(
            nums,
            [
                48, 47, 46, 45, 44, 43, 42, 41, 31, 32, 33, 34, 35, 36, 37, 38
            ]
        );
        let at = |n: u8| {
            cells
                .iter()
                .flatten()
                .find(|(f, _)| f.get() == n)
                .unwrap()
                .1
        };
        assert_eq!(at(46), Cell::Marked);
        assert_eq!(at(45), Cell::Missing);
        assert_eq!(at(44), Cell::Empty);
        // missing wins over a mark; the third molars are left out unless searched for
        let cells = grid(Arch::Upper, false, &[f(18), f(11)], &[11]);
        assert!(cells[0].is_none() && cells[15].is_none());
        assert_eq!(cells.iter().flatten().count(), 14);
        assert_eq!(cells[7], Some((f(11), Cell::Missing)));
    }
}
