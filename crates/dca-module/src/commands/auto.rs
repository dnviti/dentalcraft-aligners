//! The whole pipeline in one command (`aligners.auto`, the plugin's
//! `api.auto`).
//!
//! `aligners.auto` runs the automatic stages in pipeline order on the scans
//! of the case, with their default parameters:
//!
//! 1. orientation (`aligners.preparation.orientation.automatic`);
//! 2. numbering (`aligners.preparation.numbering.automatic`): the seeds of
//!    the arches that have none (parameter `seeds`);
//! 3. segmentation (`aligners.preparation.segmentation.automatic`);
//! 4. reconstruction (`aligners.preparation.reconstruction.automatic`, which
//!    measures the teeth first: `aligners.teeth.frames`);
//! 5. gingiva (`aligners.preparation.gingiva`);
//! 6. diagnosis (`aligners.diagnosis.space-analysis`: the whole analysis);
//! 7. setup (`aligners.planning.setup.automatic`);
//! 8. staging (`aligners.planning.staging.automatic`);
//! 9. schedule (`aligners.planning.schedule`);
//! 10. with both arches, the virtual bite and the contact map
//!     (`aligners.occlusion.virtual-bite`, `aligners.occlusion.contact-map`;
//!     parameter `occlusion`);
//! 11. with `export_dir`, the export (`aligners.output.export-stl`, or the
//!     full package `aligners.output.report` with `full_report`).
//!
//! The first step that fails stops the pipeline: the error names the step
//! and the case is left as it was (the command is one undoable change). The
//! output lists every step with its seconds and its output.
//!
//! It is not a menu leaf: the app runs the stages one by one.

use std::time::Instant;

use dc_case::{CaseState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_model::{AlignersState, MODULE_ID};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.auto",
        label: "aligners-cmd-auto",
        doc: "aligners-cmd-auto-doc",
        params: Params,
        run: run_auto,
    }]
}

/// Which arches get automatic seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Seeds {
    /// The arches without seeds (default): seeds placed by hand are kept.
    #[default]
    Missing,
    /// Every arch: the seeds already there are replaced.
    All,
    /// None: every arch must have its seeds.
    Keep,
}

/// Parameters of `aligners.auto`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Which arches get automatic seeds.
    pub seeds: Seeds,
    /// Third molars may be present (numbering).
    pub third_molars: bool,
    /// FDI numbers of the teeth known to be missing (numbering).
    pub missing: Vec<u8>,
    /// With both arches, put them in occlusion and measure the contacts.
    pub occlusion: bool,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    pub start_date: Option<String>,
    /// Days of wear of each aligner (default: the case settings).
    pub interval_days: Option<u32>,
    /// Export the aligners into this directory at the end.
    pub export_dir: Option<String>,
    /// Export the full package (`aligners.output.report`) instead of the
    /// models, the table and the calendar (`aligners.output.export-stl`).
    pub full_report: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            seeds: Seeds::Missing,
            third_molars: false,
            missing: Vec::new(),
            occlusion: true,
            start_date: None,
            interval_days: None,
            export_dir: None,
            full_report: false,
        }
    }
}

/// One step of the pipeline: a command and its parameters.
struct Step {
    id: &'static str,
    params: Value,
}

impl Step {
    fn new(id: &'static str, params: Value) -> Self {
        Self { id, params }
    }
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The arches with a scan in the case, lower first.
fn arches(state: &CaseState) -> Vec<Arch> {
    [Arch::Lower, Arch::Upper]
        .into_iter()
        .filter(|a| state.case.scan(a.role()).is_some())
        .collect()
}

/// The numbering steps the `seeds` policy asks for on this case.
fn numbering(state: &CaseState, params: &Params) -> Result<Vec<Step>, CommandError> {
    let seeds = match state.module(MODULE_ID) {
        None => Vec::new(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .map(|d| d.seeds.clone())
            .ok_or_else(|| CommandError::Invalid("the Aligners data cannot be read".to_owned()))?,
    };
    let mut steps = Vec::new();
    for arch in arches(state) {
        let has = seeds.iter().any(|s| Arch::of(s.fdi) == arch);
        let run = match params.seeds {
            Seeds::All => true,
            Seeds::Missing => !has,
            Seeds::Keep if !has => {
                return Err(CommandError::Invalid(format!(
                    "aligners.auto: the {} arch has no seeds (seeds: keep): place them or use seeds: missing",
                    arch_name(arch)
                )));
            }
            Seeds::Keep => false,
        };
        if run {
            steps.push(Step::new(
                "aligners.preparation.numbering.automatic",
                json!({
                    "arch": arch_name(arch),
                    "third_molars": params.third_molars,
                    "missing": params.missing,
                }),
            ));
        }
    }
    Ok(steps)
}

/// The calendar parameters shared by the schedule and the export.
fn calendar(params: &Params) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(d) = &params.start_date {
        m.insert("start_date".to_owned(), json!(d));
    }
    if let Some(n) = params.interval_days {
        m.insert("interval_days".to_owned(), json!(n));
    }
    m
}

/// The steps after the numbering, in pipeline order.
fn later_steps(state: &CaseState, params: &Params) -> Vec<Step> {
    let mut steps = vec![
        Step::new("aligners.preparation.segmentation.automatic", Value::Null),
        Step::new("aligners.preparation.reconstruction.automatic", Value::Null),
        Step::new("aligners.preparation.gingiva", Value::Null),
        Step::new("aligners.diagnosis.space-analysis", Value::Null),
        Step::new("aligners.planning.setup.automatic", Value::Null),
        Step::new("aligners.planning.staging.automatic", Value::Null),
        Step::new(
            "aligners.planning.schedule",
            Value::Object(calendar(params)),
        ),
    ];
    let both =
        state.case.scan(ScanRole::Upper).is_some() && state.case.scan(ScanRole::Lower).is_some();
    if params.occlusion && both {
        steps.push(Step::new("aligners.occlusion.virtual-bite", Value::Null));
        steps.push(Step::new("aligners.occlusion.contact-map", Value::Null));
    }
    if let Some(dir) = params.export_dir.as_ref().filter(|d| !d.trim().is_empty()) {
        let mut p = calendar(params);
        p.insert("dir".to_owned(), json!(dir));
        let id = if params.full_report {
            "aligners.output.report"
        } else {
            "aligners.output.export-stl"
        };
        steps.push(Step::new(id, Value::Object(p)));
    }
    steps
}

/// Runs one step on `state`; the error names it.
fn run_step(
    state: &CaseState,
    step: &Step,
    k: usize,
    ctx: &JobCtx,
) -> Result<(CaseState, Value), CommandError> {
    let spec = crate::commands::find(step.id).ok_or_else(|| {
        CommandError::Failed(format!(
            "aligners.auto: the step {} is not registered",
            step.id
        ))
    })?;
    ctx.check_cancelled()?;
    let t0 = Instant::now();
    let (next, out) = spec.execute(state, &step.params, ctx).map_err(|e| {
        let at = |m: String| format!("aligners.auto stopped at step {k} ({}): {m}", step.id);
        match e {
            CommandError::Cancelled => CommandError::Cancelled,
            CommandError::Invalid(m) => CommandError::Invalid(at(m)),
            CommandError::Failed(m) => CommandError::Failed(at(m)),
            CommandError::NotAvailable(m) => CommandError::NotAvailable(at(m)),
            other => CommandError::Failed(at(other.to_string())),
        }
    })?;
    let record = json!({
        "step": k,
        "id": step.id,
        "seconds": t0.elapsed().as_secs_f64(),
        "output": out,
    });
    Ok((next, record))
}

fn run_auto(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if arches(state).is_empty() {
        return Err(CommandError::Invalid(
            "aligners.auto: the case has no upper or lower scan: import one (case.import) or run aligners.demo".to_owned(),
        ));
    }
    let t0 = Instant::now();
    let mut steps = vec![Step::new(
        "aligners.preparation.orientation.automatic",
        Value::Null,
    )];
    steps.extend(numbering(state, &params)?);
    steps.extend(later_steps(state, &params));

    let n = steps.len();
    let mut current = state.clone();
    let mut records = Vec::with_capacity(n);
    for (i, step) in steps.iter().enumerate() {
        ctx.set_fraction(i as f32 / n as f32);
        let (next, record) = run_step(&current, step, i + 1, ctx)?;
        current = next;
        records.push(record);
    }
    ctx.set_fraction(1.0);
    let out = json!({
        "steps": records,
        "seconds": t0.elapsed().as_secs_f64(),
    });
    Ok((current, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn it_is_registered_and_not_a_menu_leaf() {
        let spec = crate::commands::find("aligners.auto").unwrap();
        let schema = spec.schema();
        assert!(schema["properties"]["export_dir"].is_object(), "{schema}");
        assert!(crate::tree::find("aligners.auto").is_none());
    }

    #[test]
    fn it_needs_a_scan_and_names_the_failing_step() {
        let err = exec("aligners.auto", &CaseState::default(), Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("no upper or lower scan")));
        // the demo arch with its seeds removed and seeds: keep stops before any work
        let mut s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.seeds.clear();
        s.set_module(MODULE_ID, dc_case::ModuleState::new(data));
        let err = exec("aligners.auto", &s, json!({"seeds": "keep"})).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("lower arch has no seeds")));
        // unknown parameter values are refused
        let err = exec(
            "aligners.auto",
            &CaseState::default(),
            json!({"seeds": "nope"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("parameters")));
    }

    #[test]
    fn the_steps_follow_the_pipeline_order() {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        let s = exec("aligners.demo", &s, json!({"arch": "upper"}))
            .unwrap()
            .0;
        let p = Params {
            export_dir: Some("x".to_owned()),
            start_date: Some("2026-01-05".to_owned()),
            ..Params::default()
        };
        // the demo has its seeds: no numbering
        assert!(numbering(&s, &p).unwrap().is_empty());
        let all = numbering(
            &s,
            &Params {
                seeds: Seeds::All,
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<&str> = later_steps(&s, &p).iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "aligners.preparation.segmentation.automatic",
                "aligners.preparation.reconstruction.automatic",
                "aligners.preparation.gingiva",
                "aligners.diagnosis.space-analysis",
                "aligners.planning.setup.automatic",
                "aligners.planning.staging.automatic",
                "aligners.planning.schedule",
                "aligners.occlusion.virtual-bite",
                "aligners.occlusion.contact-map",
                "aligners.output.export-stl",
            ]
        );
        for id in ids {
            assert!(crate::commands::is_registered(id), "{id}");
        }
        let steps = later_steps(&s, &p);
        assert_eq!(steps[6].params["start_date"], "2026-01-05");
        assert_eq!(steps[9].params["dir"], "x");
    }
}
