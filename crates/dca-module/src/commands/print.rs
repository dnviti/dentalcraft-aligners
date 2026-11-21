//! The print tracking of the aligners (print plan §7.4, §8).
//!
//! * `aligners.production.print` is the leaf "Print": it lists the aligner
//!   shells to print (`print_parts`), with the folder of the last export,
//!   and changes nothing. In the app its output opens the print window with
//!   those parts; an agent hands their files to `print.prepare`.
//! * `aligners.print.parts` lists every aligner shell of the series: its id
//!   (`SUP-03`, `INF-12-P`), arch, number, whether it is passive, its file
//!   when the last export wrote it, and its print status.
//! * `aligners.print.status` counts them: printed, still to print, by status.
//! * `aligners.print.mark` marks aligners (`queued`, `sent`, `printed`,
//!   `failed`, `reprint`), all or none, as one undoable change. `printed`
//!   needs the user's confirmation or a printer's report
//!   ([`dca_model::print_transition`]).
//!
//! The series is the one the export writes: aligners 1 to N of each arch
//! with aligners (N: its last aligner that moves a tooth), then passive
//! aligners up to the longer series of the two
//! (`dca_algo::export::package`). An aligner with no record is still to
//! print (`missing`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::export::package::aligner_name;
use dca_algo::schedule::{self, Date};
use dca_algo::teeth::Which;
use dca_model::{
    AlignersState, MODULE_ID, MarkSource, PrintKey, PrintMark, PrintRecord, PrintStatus,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::messages::{invalid, tr};
use super::occlusion::aligners_data;

/// Id of the leaf "Print".
pub(crate) const PRINT: &str = "aligners.production.print";
/// Id of the command that lists the aligner shells.
pub(crate) const PARTS: &str = "aligners.print.parts";
/// Id of the command that counts them.
pub(crate) const STATUS: &str = "aligners.print.status";
/// Id of the command that marks them.
pub(crate) const MARK: &str = "aligners.print.mark";

/// The registered commands of the print tracking.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: PRINT,
            label: "aligners-menu-production-print",
            doc: "aligners-cmd-production-print-doc",
            params: PartsParams,
            run: run_print,
        },
        stage_command! {
            id: PARTS,
            label: "aligners-cmd-print-parts",
            doc: "aligners-cmd-print-parts-doc",
            params: PartsParams,
            run: run_parts,
        },
        stage_command! {
            id: STATUS,
            label: "aligners-cmd-print-status",
            doc: "aligners-cmd-print-status-doc",
            params: PartsParams,
            run: run_status,
        },
        stage_command! {
            id: MARK,
            label: "aligners-cmd-print-mark",
            doc: "aligners-cmd-print-mark-doc",
            params: MarkParams,
            run: run_mark,
        },
    ]
}

/// Parameters of the commands that list or count the aligners.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PartsParams {
    /// The arches (`all`, `upper`, `lower`).
    arch: Which,
}

/// Parameters of `aligners.print.mark`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkParams {
    /// The aligners, by id (`SUP-03`, `INF-12-P`), as `aligners.print.parts`
    /// lists them.
    ids: Vec<String>,
    /// The new status: `queued`, `sent`, `printed`, `failed`, `reprint`.
    status: PrintStatus,
    /// Who marks: `user` (default), `printer`, `app`. `printed` is refused
    /// from `app`.
    #[serde(default)]
    by: MarkSource,
    /// The plate or job the aligners went with (the plate file, the
    /// printer's task).
    #[serde(default)]
    job: Option<String>,
    /// Why, for `failed` and `reprint`.
    #[serde(default)]
    reason: Option<String>,
}

impl Default for MarkParams {
    fn default() -> Self {
        Self {
            ids: Vec::new(),
            status: PrintStatus::Printed,
            by: MarkSource::User,
            job: None,
            reason: None,
        }
    }
}

/// Every aligner shell of the series of `data`, arch by arch (upper first)
/// and in order: the active aligners, then the passive ones.
pub(crate) fn series(data: &AlignersState, which: Which) -> Vec<PrintKey> {
    let active: Vec<(Arch, u32)> = [Arch::Upper, Arch::Lower]
        .into_iter()
        .map(|a| (a, schedule::active_stages(a, &data.stages)))
        .filter(|(_, n)| *n > 0)
        .collect();
    let total = active.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let wanted = which.arches();
    active
        .into_iter()
        .filter(|(a, _)| wanted.contains(a))
        .flat_map(|(a, n)| (1..=total).filter_map(move |k| PrintKey::new(a.role(), k, k > n)))
        .collect()
}

fn arch_of(key: PrintKey) -> Arch {
    Arch::from_role(key.arch).unwrap_or(Arch::Lower)
}

/// The shell file of `key` in the export folder `dir`, when it is there.
pub(crate) fn shell_file(dir: &str, key: PrintKey) -> Option<PathBuf> {
    let name = aligner_name(arch_of(key), key.stage, key.passive);
    let path = name
        .split('/')
        .fold(PathBuf::from(dir), |p, part| p.join(part));
    path.is_file().then_some(path)
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The name of a status in the outputs; `missing` for no record.
fn status_name(status: Option<PrintStatus>) -> &'static str {
    status.map_or("missing", PrintStatus::as_str)
}

/// i18n key of the name of a status as the user reads it (`None`: still to
/// print).
pub(crate) fn status_key(status: Option<PrintStatus>) -> &'static str {
    match status {
        None => "aligners-print-status-missing",
        Some(PrintStatus::Queued) => "aligners-print-status-queued",
        Some(PrintStatus::Sent) => "aligners-print-status-sent",
        Some(PrintStatus::Printed) => "aligners-print-status-printed",
        Some(PrintStatus::Failed) => "aligners-print-status-failed",
        Some(PrintStatus::Reprint) => "aligners-print-status-reprint",
    }
}

/// `at` (seconds since 1970-01-01 UTC) as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn utc_text(at: u64) -> String {
    let day = Date::from_days((at / 86_400) as i64).to_iso();
    let s = at % 86_400;
    format!("{day}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The record of `key` as the outputs give it.
fn record_json(r: &PrintRecord) -> Value {
    let mut v = json!({
        "at": r.at,
        "at_utc": utc_text(r.at),
        "by": r.by,
    });
    if let Some(j) = &r.job {
        v["job"] = json!(j);
    }
    if let Some(why) = &r.reason {
        v["reason"] = json!(why);
    }
    v
}

/// One aligner shell as the commands list it.
fn part_json(data: &AlignersState, key: PrintKey) -> Value {
    let record = data.prints.get(&key);
    let path = data
        .export_dir
        .as_deref()
        .and_then(|d| shell_file(d, key))
        .map(|p| path_text(&p));
    let label = if key.passive {
        format!("{} {:02} P", key.prefix(), key.stage)
    } else {
        format!("{} {:02}", key.prefix(), key.stage)
    };
    let mut v = json!({
        "id": key.to_string(),
        "label": label,
        "arch": if arch_of(key).is_upper() { "upper" } else { "lower" },
        "stage": key.stage,
        "passive": key.passive,
        "path": path,
        "status": status_name(record.map(|r| r.status)),
    });
    if let Some(r) = record {
        v["record"] = record_json(r);
    }
    v
}

fn parts_json(data: &AlignersState, keys: &[PrintKey]) -> Vec<Value> {
    keys.iter().map(|k| part_json(data, *k)).collect()
}

/// The aligners of the series of `data`, or the error that says there are
/// none.
fn series_of(data: &AlignersState, which: Which) -> Result<Vec<PrintKey>, CommandError> {
    let keys = series(data, which);
    if keys.is_empty() {
        return Err(invalid("aligners-err-no-aligners", &[]));
    }
    Ok(keys)
}

/// How many aligners of `keys` are printed and how many are still to print,
/// by status.
pub(crate) fn summary(data: &AlignersState, keys: &[PrintKey]) -> Value {
    let mut by_status = Map::new();
    by_status.insert("missing".to_owned(), json!(0));
    for s in PrintStatus::ALL {
        by_status.insert(s.as_str().to_owned(), json!(0));
    }
    let mut printed = Vec::new();
    let mut to_print = Vec::new();
    for k in keys {
        let status = data.print_status(*k);
        let name = status_name(status);
        by_status[name] = json!(by_status[name].as_u64().unwrap_or(0) + 1);
        if status == Some(PrintStatus::Printed) {
            printed.push(k.to_string());
        } else {
            to_print.push(k.to_string());
        }
    }
    json!({
        "total": keys.len(),
        "printed": printed.len(),
        "missing": to_print.len(),
        "by_status": by_status,
        "printed_ids": printed,
        "missing_ids": to_print,
    })
}

fn run_print(
    state: &CaseState,
    p: PartsParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let keys = series_of(&data, p.arch)?;
    let counts = summary(&data, &keys);
    Ok((
        state.clone(),
        json!({
            "print_parts": parts_json(&data, &keys),
            "export_dir": data.export_dir,
            "printed": counts["printed"],
            "missing": counts["missing"],
        }),
    ))
}

fn run_parts(
    state: &CaseState,
    p: PartsParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let keys = series_of(&data, p.arch)?;
    Ok((
        state.clone(),
        json!({
            "parts": parts_json(&data, &keys),
            "export_dir": data.export_dir,
        }),
    ))
}

fn run_status(
    state: &CaseState,
    p: PartsParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let keys = series(&data, p.arch);
    Ok((state.clone(), summary(&data, &keys)))
}

fn run_mark(state: &CaseState, p: MarkParams, ctx: &JobCtx) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    if p.ids.is_empty() {
        return Err(invalid("aligners-err-print-no-ids", &[]));
    }
    let known = series(&data, Which::All);
    let mut keys: Vec<PrintKey> = Vec::with_capacity(p.ids.len());
    for id in &p.ids {
        let key: PrintKey = id
            .trim()
            .parse()
            .ok()
            .filter(|k| known.contains(k))
            .ok_or_else(|| invalid("aligners-err-print-unknown-id", &[("id", id)]))?;
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mark = PrintMark {
        status: p.status,
        by: p.by,
        at: now(),
        job: p.job.filter(|j| !j.trim().is_empty()),
        reason: p.reason.filter(|r| !r.trim().is_empty()),
    };
    if let Err(k) = data.mark_prints(&keys, &mark) {
        let id = k.to_string();
        let from = tr(status_key(data.print_status(k)), &[]);
        let to = tr(status_key(Some(p.status)), &[]);
        return Err(
            if p.status == PrintStatus::Printed && p.by == MarkSource::App {
                invalid("aligners-err-print-printed-by-app", &[("id", &id)])
            } else {
                invalid(
                    "aligners-err-print-transition",
                    &[("id", &id), ("from", &from), ("to", &to)],
                )
            },
        );
    }
    let out = json!({
        "marked": parts_json(&data, &keys),
        "summary": summary(&data, &known),
    });
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::{Fdi, Pose};
    use dca_model::Stage;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    /// A case whose upper arch moves in 3 aligners and the lower in 5: the
    /// upper gets passive aligners 4 and 5.
    fn staged() -> CaseState {
        let (f11, f41) = (Fdi::new(11).unwrap(), Fdi::new(41).unwrap());
        let stages = (0..=5u32)
            .map(|k| {
                // a tooth moving in `n` aligners: 0.1 mm along x each
                let at = |n: u32| Pose {
                    t: glam::DVec3::new(f64::from(k.min(n)) * 0.1, 0.0, 0.0),
                    ..Pose::IDENTITY
                };
                let mut s = Stage::new(k);
                s.poses.insert(f11, at(3));
                s.poses.insert(f41, at(5));
                s
            })
            .collect();
        let mut state = CaseState::default();
        state.set_module(
            MODULE_ID,
            ModuleState::new(AlignersState {
                stages,
                ..AlignersState::default()
            }),
        );
        state
    }

    fn ids(v: &Value, key: &str) -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn the_parts_are_the_series_the_export_writes() {
        let state = staged();
        let (same, out) = exec(PARTS, &state, Value::Null).unwrap();
        assert_eq!(data(&same), data(&state));
        assert_eq!(
            ids(&out, "parts"),
            [
                "SUP-01", "SUP-02", "SUP-03", "SUP-04-P", "SUP-05-P", "INF-01", "INF-02", "INF-03",
                "INF-04", "INF-05"
            ]
        );
        let p = &out["parts"][3];
        assert_eq!(p["arch"], "upper");
        assert_eq!(
            (p["stage"].as_u64(), p["passive"].as_bool()),
            (Some(4), Some(true))
        );
        assert_eq!(p["label"], "SUP 04 P");
        assert_eq!(p["status"], "missing");
        assert!(p["path"].is_null() && out["export_dir"].is_null());
        let (_, out) = exec(PARTS, &state, json!({ "arch": "lower" })).unwrap();
        assert_eq!(ids(&out, "parts").len(), 5);
        // no aligners: nothing to print
        assert!(matches!(
            exec(PARTS, &CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_files_of_the_last_export_are_found() {
        let dir = std::env::temp_dir().join(format!("dca-print-parts-{}", std::process::id()));
        let shells = dir.join("stampa_resina");
        std::fs::create_dir_all(&shells).unwrap();
        std::fs::write(shells.join("SUP_allineatore_02.stl"), b"solid").unwrap();
        std::fs::write(shells.join("SUP_allineatore_05_passivo.stl"), b"solid").unwrap();
        let mut state = staged();
        let mut d = data(&state).clone();
        d.export_dir = Some(dir.display().to_string());
        state.set_module(MODULE_ID, ModuleState::new(d));
        let (_, out) = exec(PRINT, &state, Value::Null).unwrap();
        let parts = out["print_parts"].as_array().unwrap();
        let path = |id: &str| {
            parts.iter().find(|p| p["id"] == id).unwrap()["path"]
                .as_str()
                .map(PathBuf::from)
        };
        assert_eq!(path("SUP-02"), Some(shells.join("SUP_allineatore_02.stl")));
        assert_eq!(
            path("SUP-05-P"),
            Some(shells.join("SUP_allineatore_05_passivo.stl"))
        );
        assert_eq!(path("SUP-01"), None);
        assert_eq!(out["export_dir"], dir.display().to_string());
        assert_eq!(
            (out["printed"].as_u64(), out["missing"].as_u64()),
            (Some(0), Some(10))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_mark_is_one_change_and_printed_needs_the_user_or_the_printer() {
        let state = staged();
        let (s1, out) = exec(
            MARK,
            &state,
            json!({ "ids": ["SUP-01", "INF-01", "SUP-04-P"], "status": "sent",
                    "job": "plate_01.goo", "by": "app" }),
        )
        .unwrap();
        assert_eq!(ids(&out, "marked"), ["SUP-01", "INF-01", "SUP-04-P"]);
        assert_eq!(out["marked"][0]["status"], "sent");
        assert_eq!(out["marked"][0]["record"]["job"], "plate_01.goo");
        assert!(data(&state).prints.is_empty(), "the old state is untouched");
        // the app never marks printed
        let err = exec(
            MARK,
            &s1,
            json!({ "ids": ["SUP-01"], "status": "printed", "by": "app" }),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
        // the user confirms; then asks a reprint
        let (s2, _) = exec(MARK, &s1, json!({ "ids": ["SUP-01"], "status": "printed" })).unwrap();
        let (s3, _) = exec(
            MARK,
            &s2,
            json!({ "ids": ["SUP-01"], "status": "reprint", "reason": "lost" }),
        )
        .unwrap();
        assert_eq!(
            data(&s3).prints[&"SUP-01".parse().unwrap()]
                .reason
                .as_deref(),
            Some("lost")
        );
        // one refused aligner refuses the whole mark
        let err = exec(
            MARK,
            &s2,
            json!({ "ids": ["INF-01", "SUP-01"], "status": "printed" }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("SUP-01"), "{err}");
        // unknown, malformed or no ids
        for bad in [
            json!({ "ids": ["SUP-09"], "status": "sent" }),
            json!({ "ids": ["INF-03-P"], "status": "sent" }),
            json!({ "ids": ["nope"], "status": "sent" }),
            json!({ "ids": [], "status": "sent" }),
            json!({ "ids": ["SUP-01"], "status": "lost" }),
            json!({ "ids": ["SUP-01"] }),
        ] {
            assert!(
                matches!(exec(MARK, &s2, bad.clone()), Err(CommandError::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_status_counts_printed_and_missing() {
        let state = staged();
        let (s, _) = exec(
            MARK,
            &state,
            json!({ "ids": ["SUP-01", "SUP-02", "INF-01"], "status": "sent", "by": "app" }),
        )
        .unwrap();
        let (s, _) = exec(
            MARK,
            &s,
            json!({ "ids": ["SUP-01", "INF-01"], "status": "printed", "by": "printer" }),
        )
        .unwrap();
        let (s, _) = exec(MARK, &s, json!({ "ids": ["SUP-02"], "status": "failed" })).unwrap();
        let (same, out) = exec(STATUS, &s, Value::Null).unwrap();
        assert_eq!(data(&same), data(&s));
        assert_eq!(out["total"], 10);
        assert_eq!(out["printed"], 2);
        assert_eq!(out["missing"], 8);
        assert_eq!(out["printed_ids"], json!(["SUP-01", "INF-01"]));
        assert_eq!(out["by_status"]["failed"], 1);
        assert_eq!(out["by_status"]["missing"], 7);
        let (_, out) = exec(STATUS, &s, json!({ "arch": "upper" })).unwrap();
        assert_eq!(
            (out["total"].as_u64(), out["printed"].as_u64()),
            (Some(5), Some(1))
        );
        // no aligners: nothing counted, no error
        let (_, out) = exec(STATUS, &CaseState::default(), Value::Null).unwrap();
        assert_eq!(out["total"], 0);
    }

    #[test]
    fn the_time_reads_as_utc() {
        assert_eq!(utc_text(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_text(1_790_000_000), "2026-09-21T14:13:20Z");
    }
}
