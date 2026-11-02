//! The commands behind the menu leaves that join several stages: one click,
//! one undoable change. Each runs the registered commands of its stages
//! through the same steps as `aligners.auto`, with no logic of its own.
//!
//! * `aligners.teeth.find`: the numbering of the teeth, then the orientation
//!   from the new marks (the first two stages of `aligners.auto`); each arch
//!   is numbered on its own, an arch that fails is reported in the output of
//!   the numbering step (`failed`) and keeps what it had;
//! * `aligners.teeth.build`: the closed teeth (which measures the teeth
//!   first) and the gingiva;
//! * `aligners.occlusion.align`: the registration on the bite scans when the
//!   case has one, otherwise the virtual bite; then the contact map;
//! * `aligners.planning.aligners`: the aligners and their calendar;
//! * `aligners.output.export-all`: the full package from the shell stage,
//!   refusing to fall back on the scans (see [`run_export_all`]).

use std::time::Instant;

use dc_case::{CaseState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_i18n::Lang as UiLang;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::export::provider::Quality;
use dca_algo::schedule::Lang;
use dca_algo::teeth::Which;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::auto::{Step, run_step};
use super::autoseed::NumberingParams;

/// Id of the "Find the teeth" leaf.
pub(crate) const FIND_TEETH: &str = "aligners.teeth.find";
/// The steps of [`FIND_TEETH`], also run one at a time.
pub(super) const ORIENTATION: &str = "aligners.preparation.orientation.automatic";
pub(super) const NUMBERING: &str = "aligners.preparation.numbering.automatic";
/// Id of the bite leaf.
pub(crate) const BITE: &str = "aligners.occlusion.align";
pub(super) const REGISTER: &str = "aligners.occlusion.bite-registration.icp";
pub(super) const VIRTUAL_BITE: &str = "aligners.occlusion.virtual-bite";
pub(super) const CONTACT_MAP: &str = "aligners.occlusion.contact-map";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: FIND_TEETH,
            label: "aligners-menu-teeth-find",
            doc: "aligners-cmd-teeth-find-doc",
            params: NumberingParams,
            run: run_find_teeth,
        },
        stage_command! {
            id: "aligners.teeth.build",
            label: "aligners-menu-teeth-build",
            doc: "aligners-cmd-teeth-build-doc",
            params: TeethParams,
            run: run_teeth,
        },
        stage_command! {
            id: BITE,
            label: "aligners-menu-occlusion-align",
            doc: "aligners-cmd-occlusion-align-doc",
            params: BiteParams,
            run: run_bite,
        },
        stage_command! {
            id: "aligners.planning.aligners",
            label: "aligners-menu-planning-aligners",
            doc: "aligners-cmd-planning-aligners-doc",
            params: AlignersParams,
            run: run_aligners,
        },
        stage_command! {
            id: "aligners.output.export-all",
            label: "aligners-menu-output-export-all",
            doc: "aligners-cmd-export-all-doc",
            params: ExportParams,
            run: run_export_all,
        },
    ]
}

/// Runs `steps` in order on `state`: the first failure stops them and leaves
/// the case as it was. Returns the new state and the output of every step.
fn run_steps(
    state: &CaseState,
    steps: &[Step],
    ctx: &JobCtx,
) -> Result<(CaseState, Vec<Value>), CommandError> {
    let mut current = state.clone();
    let mut records = Vec::with_capacity(steps.len());
    for (k, step) in steps.iter().enumerate() {
        ctx.set_fraction(k as f32 / steps.len() as f32);
        let (next, record) = run_step(&current, step, k, ctx)?;
        current = next;
        records.push(record);
    }
    ctx.set_fraction(1.0);
    Ok((current, records))
}

/// The language of the app, for what the files say.
fn app_lang() -> Lang {
    match dc_i18n::current() {
        UiLang::En => Lang::En,
        UiLang::It => Lang::It,
    }
}

/// The numbering of the teeth, then the orientation: the steps
/// `aligners.auto` starts with, in its order, so the frame comes from the new
/// marks as in the plugin (`case_auto.auto_seeds_iter` sets the arch frame of
/// the proposal), here with the numbering parameters of the click. The
/// numbering always runs on the arches asked for, replacing their seeds;
/// with every arch, one that fails is reported and the others keep their new
/// seeds (the numbering does not read the world frame, so the order changes
/// nothing for it).
fn run_find_teeth(
    state: &CaseState,
    params: NumberingParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let numbering =
        serde_json::to_value(&params).map_err(|e| CommandError::Failed(e.to_string()))?;
    let steps = [
        Step::new(NUMBERING, numbering),
        Step::new(ORIENTATION, Value::Null),
    ];
    let (next, records) = run_steps(state, &steps, ctx)?;
    Ok((next, json!({ "steps": records })))
}

/// Parameters of `aligners.teeth.build`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TeethParams {
    /// Arches (default all the arches with teeth and gum separated).
    arch: Which,
}

fn run_teeth(
    state: &CaseState,
    params: TeethParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let arch = json!({ "arch": params.arch });
    let steps = [
        Step::new(
            "aligners.preparation.reconstruction.automatic",
            arch.clone(),
        ),
        Step::new("aligners.preparation.gingiva", arch),
    ];
    let (next, records) = run_steps(state, &steps, ctx)?;
    Ok((next, json!({ "steps": records })))
}

/// Parameters of `aligners.occlusion.align`: none.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BiteParams {}

fn has_bite_scan(state: &CaseState) -> bool {
    state.case.scan(ScanRole::Bite).is_some()
}

/// The steps that put the arches in occlusion, and the name of the method
/// (`bite_scan` or `virtual_bite`): the registration on the bite scans when
/// the case has one, otherwise the virtual bite; then the contact map. A
/// refused registration is an error, the estimate never takes its place.
/// `aligners.auto` takes the same steps.
pub(super) fn bite_steps(state: &CaseState) -> (&'static str, [Step; 2]) {
    let (method, id) = if has_bite_scan(state) {
        ("bite_scan", REGISTER)
    } else {
        ("virtual_bite", VIRTUAL_BITE)
    };
    (
        method,
        [
            Step::new(id, Value::Null),
            Step::new(CONTACT_MAP, Value::Null),
        ],
    )
}

fn run_bite(
    state: &CaseState,
    _params: BiteParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let (method, steps) = bite_steps(state);
    let (next, records) = run_steps(state, &steps, ctx)?;
    Ok((next, json!({ "method": method, "steps": records })))
}

/// Parameters of `aligners.planning.aligners`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AlignersParams {
    /// Arches (default all the arches with a final position).
    arch: Which,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    start_date: Option<String>,
    /// Days of wear of each aligner (default: the case settings).
    interval_days: Option<u32>,
}

fn run_aligners(
    state: &CaseState,
    params: AlignersParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let mut calendar = Map::new();
    calendar.insert("arch".to_owned(), json!(params.arch));
    calendar.insert("lang".to_owned(), json!(app_lang()));
    if let Some(d) = &params.start_date {
        calendar.insert("start_date".to_owned(), json!(d));
    }
    if let Some(n) = params.interval_days {
        calendar.insert("interval_days".to_owned(), json!(n));
    }
    let steps = [
        Step::new(
            "aligners.planning.staging.automatic",
            json!({ "arch": params.arch }),
        ),
        Step::new("aligners.planning.schedule", Value::Object(calendar)),
    ];
    let (next, records) = run_steps(state, &steps, ctx)?;
    Ok((next, json!({ "steps": records })))
}

/// Parameters of `aligners.output.export-all`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ExportParams {
    /// Target folder (created if it does not exist).
    dir: String,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    start_date: Option<String>,
    /// Days of wear of each aligner (default: the case settings).
    interval_days: Option<u32>,
    /// Quality of the print files (default the production quality).
    quality: Quality,
    /// Language of the report, the programmes and the calendar (default the
    /// language of the app).
    lang: Option<Lang>,
    /// Case code in the stamp (letters, digits, `-`; at most 8; never the
    /// name of the patient).
    stamp_code: String,
}

/// The whole package: `aligners.output.report` with the print files of the
/// shell stage required. Left to itself that command falls back on the moved
/// scans, with no aligner, when a tooth is not closed; a print job must not
/// end that way, so here it is refused with the reason.
fn run_export_all(
    state: &CaseState,
    params: ExportParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let mut report = Map::new();
    report.insert("dir".to_owned(), json!(params.dir));
    report.insert("shells".to_owned(), json!(true));
    report.insert("quality".to_owned(), json!(params.quality));
    report.insert("stamp_code".to_owned(), json!(params.stamp_code));
    report.insert(
        "lang".to_owned(),
        json!(params.lang.unwrap_or_else(app_lang)),
    );
    if let Some(d) = &params.start_date {
        report.insert("start_date".to_owned(), json!(d));
    }
    if let Some(n) = params.interval_days {
        report.insert("interval_days".to_owned(), json!(n));
    }
    let t0 = Instant::now();
    let step = Step::new("aligners.output.report", Value::Object(report));
    let (next, record) = run_step(state, &step, 0, ctx)?;
    let mut out = record["output"].clone();
    out["seconds"] = json!(t0.elapsed().as_secs_f64());
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use super::super::occlusion::tests::two_demo_arches;
    use super::*;
    use dca_algo::anatomy::Arch;
    use dca_model::{AlignersState, MODULE_ID};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    fn step_ids(out: &Value) -> Vec<&str> {
        out["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn every_wrapper_is_a_menu_leaf_with_a_schema() {
        for spec in specs() {
            assert!(crate::tree::find(spec.id).is_some(), "{}", spec.id);
            assert_eq!(spec.schema()["type"], "object", "{}", spec.id);
        }
    }

    fn lower_demo() -> CaseState {
        exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0
    }

    #[test]
    fn find_teeth_is_the_numbering_then_the_orientation_like_the_pipeline() {
        use super::super::auto::{Params as AutoParams, Seeds, plan};
        let s = lower_demo();
        let (next, out) = exec(FIND_TEETH, &s, Value::Null).unwrap();
        assert_eq!(step_ids(&out), [NUMBERING, ORIENTATION]);
        // the steps `aligners.auto` takes first, one after the other
        let auto_params = AutoParams {
            seeds: Seeds::All,
            ..AutoParams::default()
        };
        let steps = plan(&s, &auto_params).unwrap();
        let mut by_pipeline = s.clone();
        for (k, step) in steps.iter().take(2).enumerate() {
            assert_eq!(step.id, [NUMBERING, ORIENTATION][k]);
            by_pipeline = run_step(&by_pipeline, step, k, &JobCtx::detached())
                .unwrap()
                .0;
        }
        assert_eq!(
            next.case.world_frame.from_case,
            by_pipeline.case.world_frame.from_case
        );
        assert_eq!(data(&next).seeds, data(&by_pipeline).seeds);
        // the frame comes from the new seeds: the orientation run on the
        // numbered case gives the same, not the identity the demo starts with
        let numbered = exec(NUMBERING, &s, Value::Null).unwrap().0;
        let oriented = exec(ORIENTATION, &numbered, Value::Null).unwrap().0;
        assert_eq!(
            next.case.world_frame.from_case,
            oriented.case.world_frame.from_case
        );
        assert_ne!(
            next.case.world_frame.from_case,
            s.case.world_frame.from_case
        );
    }

    #[test]
    fn find_teeth_keeps_the_arch_that_worked_and_reports_the_other_in_one_undo() {
        use super::super::autoseed::tests::with_broken_scan;
        let s = two_demo_arches();
        let broken = with_broken_scan(&s, ScanRole::Upper);
        let (next, out) = exec(FIND_TEETH, &broken, json!({ "ends": [6, 6] })).unwrap();
        assert_eq!(step_ids(&out), [NUMBERING, ORIENTATION]);
        let numbered = &out["steps"][0]["output"];
        assert_eq!(numbered["arches"][0]["arch"], "lower");
        assert_eq!(numbered["failed"][0]["arch"], "upper");
        assert!(
            numbered["failed"][0]["error"]
                .as_str()
                .unwrap()
                .contains("upper")
        );
        // the lower marks are new, the upper ones are the ones it had
        let up = |st: &CaseState| -> Vec<_> {
            data(st)
                .seeds
                .iter()
                .filter(|x| x.fdi.is_upper())
                .copied()
                .collect()
        };
        assert_eq!(up(&next), up(&broken));
        assert_ne!(data(&next).seeds, data(&broken).seeds);
    }

    #[test]
    fn find_teeth_passes_its_parameters_through_and_is_one_undo() {
        use dc_command::UndoStack;
        use std::sync::Arc;
        let spec = crate::commands::find(FIND_TEETH).unwrap();
        let schema = spec.schema();
        for p in ["arch", "third_molars", "missing", "ends"] {
            assert!(schema["properties"][p].is_object(), "{p}: {schema}");
        }
        let s = lower_demo();
        // a bad parameter is refused by the numbering step
        let err = exec(FIND_TEETH, &s, json!({ "ends": [4, 7] })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
        // a tooth known to be missing stays without a mark
        let (next, _) = exec(FIND_TEETH, &s, json!({ "missing": [41] })).unwrap();
        assert!(data(&next).seeds.iter().all(|x| x.fdi.get() != 41));
        // one undoable step: undo restores the frame and the seeds, redo the new ones
        let before = Arc::new(s);
        let mut stack = UndoStack::new(before.clone(), 1 << 30);
        stack.push(Arc::new(next.clone()), "aligners-menu-teeth-find");
        assert_eq!(stack.undo_depth(), 1);
        let undone = stack.undo().unwrap();
        assert!(Arc::ptr_eq(&undone, &before));
        assert_eq!(undone.case.world_frame, before.case.world_frame);
        assert_eq!(data(&undone).seeds, data(&before).seeds);
        let redone = stack.redo().unwrap();
        assert_eq!(redone.case.world_frame, next.case.world_frame);
        assert_eq!(data(&redone).seeds, data(&next).seeds);
    }

    #[test]
    fn find_teeth_stops_at_the_failing_step_and_changes_nothing() {
        let err = exec(FIND_TEETH, &CaseState::default(), Value::Null).unwrap_err();
        let m = err.to_string();
        assert!(m.contains("Stopped at Number the teeth"), "{m}");
        assert!(!m.contains("aligners."), "{m}");
    }

    #[test]
    fn the_bite_without_a_scan_is_the_virtual_bite_then_the_contact_map() {
        let s = two_demo_arches();
        let (next, out) = exec(BITE, &s, Value::Null).unwrap();
        assert_eq!(out["method"], "virtual_bite");
        assert_eq!(step_ids(&out), [VIRTUAL_BITE, CONTACT_MAP]);
        assert!(data(&next).bite_registered);
        assert!(!data(&s).bite_registered, "the first state is untouched");
    }

    #[test]
    fn the_bite_with_a_scan_is_the_registration_and_a_refusal_is_not_replaced() {
        let mut s = two_demo_arches();
        let lower = s.case.scan(ScanRole::Lower).unwrap().clone();
        let mut bite = lower;
        bite.role = ScanRole::Bite;
        s.case.scans.push(bite);
        // a bite scan that is a whole arch does not fit: the error is the
        // registration's, the virtual bite does not take over
        let m = exec(BITE, &s, Value::Null).unwrap_err().to_string();
        assert!(m.contains("Stopped at Bite from the bite scans"), "{m}");
    }

    #[test]
    fn the_aligners_are_the_sequence_and_the_calendar_in_one_change() {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        assert!(data(&s).stages.is_empty());
        let (next, out) = exec(
            "aligners.planning.aligners",
            &s,
            json!({ "start_date": "2026-01-05", "interval_days": 7 }),
        )
        .unwrap();
        assert_eq!(
            step_ids(&out),
            [
                "aligners.planning.staging.automatic",
                "aligners.planning.schedule"
            ]
        );
        assert!(dca_algo::schedule::active_stages(Arch::Lower, &data(&next).stages) > 0);
        assert!(data(&s).stages.is_empty());
    }

    #[test]
    fn a_failed_step_is_named_in_words_and_changes_nothing() {
        let err = exec("aligners.teeth.build", &CaseState::default(), Value::Null).unwrap_err();
        let m = err.to_string();
        assert!(m.contains("Close all the 3D teeth"), "{m}");
        assert!(!m.contains("aligners."), "{m}");
    }

    #[test]
    fn the_export_refuses_teeth_that_are_not_closed_instead_of_exporting_scans() {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let (s, _) = exec("aligners.planning.aligners", &s, Value::Null).unwrap();
        let dir = std::env::temp_dir().join(format!("dca-export-all-{}", std::process::id()));
        let err = exec(
            "aligners.output.export-all",
            &s,
            json!({ "dir": dir.display().to_string() }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("3D teeth"), "{err}");
        assert!(!dir.exists(), "nothing was written");
    }

    /// The clicks of a case from the demo arches to the print files, at the
    /// draft quality: what "Export for printing" leaves in the folder.
    #[test]
    #[ignore = "the whole pipeline and the shells of every stage: run with --release --ignored"]
    fn a_click_through_case_ends_in_a_complete_export() {
        let s = two_demo_arches();
        let (s, out) = exec("aligners.auto", &s, Value::Null).unwrap();
        eprintln!("auto: {:.1} s", out["seconds"].as_f64().unwrap());
        let dir = std::env::temp_dir().join(format!("dca-export-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let quality = std::env::var("DCA_QUALITY").unwrap_or_else(|_| "draft".to_owned());
        let (same, out) = exec(
            "aligners.output.export-all",
            &s,
            json!({ "dir": dir.display().to_string(), "quality": quality }),
        )
        .unwrap();
        assert_eq!(same.case.scans, s.case.scans, "the case does not change");
        assert_eq!(out["meshes"], "shells");
        eprintln!("export: {:.1} s", out["seconds"].as_f64().unwrap());
        let files = out["files"].as_array().unwrap();
        let mut names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
        names.sort_unstable();
        eprintln!("{} files: {names:#?}", names.len());
        eprintln!("notes: {:#}", out["notes"]);
        eprintln!("print check: {:#}", out["print_check"]);
        for n in [
            "report.html",
            "print_check.json",
            "stages.csv",
            "visits.ics",
        ] {
            assert!(names.contains(&n), "missing {n}");
        }
        assert!(dir.join("manifest.json").is_file());
        assert!(names.iter().any(|n| n.starts_with("stampa_resina/")));
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("modelli_termoformatura/"))
        );
        let (_, v) = exec(
            "aligners.output.manifest",
            &s,
            json!({ "dir": dir.display().to_string() }),
        )
        .unwrap();
        assert_eq!(v["ok"], true, "{v}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
