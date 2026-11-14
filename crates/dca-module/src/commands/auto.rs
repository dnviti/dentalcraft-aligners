//! The whole pipeline in one command (`aligners.auto`, the plugin's
//! `api.auto`).
//!
//! `aligners.auto` runs the automatic stages in pipeline order on the scans
//! of the case, with their default parameters, in the order of the plugin's
//! `case_auto.auto_steps` (arch, seeds, segmentation, teeth, diagnosis, the
//! bite when the arches are not related, setup, staging):
//!
//! 0. the arch (`aligners.case.detect-arch`): it may move scans and clear
//!    their results, so the rest of the list is built from its result;
//! 1. numbering (`aligners.preparation.numbering.automatic`): the seeds of
//!    the arches that have none (parameter `seeds`);
//! 2. orientation (`aligners.preparation.orientation.automatic`), from the
//!    seeds the numbering left, as the plugin takes the arch frame of the
//!    proposal (`case_auto.auto_seeds_iter`);
//! 3. segmentation (`aligners.preparation.segmentation.automatic`), then
//!    the check of the segmented teeth of each arch
//!    (`aligners.preparation.segmentation.check`): the seeds this run
//!    proposed are corrected and the arch segmented again, the seeds placed
//!    by hand only checked;
//! 4. reconstruction (`aligners.preparation.reconstruction.automatic`, which
//!    measures the teeth first: `aligners.teeth.frames`);
//! 5. gingiva (`aligners.preparation.gingiva`);
//! 6. diagnosis (`aligners.diagnosis.space-analysis`: the whole analysis);
//! 7. with both arches, when they are not related at that point
//!    ([`dca_algo::occlusion::bite_needed`]; `case_auto.auto_steps` before the
//!    setup: "with both arches the bite comes first"), the bite as the menu
//!    leaf "Bite" takes it: the registration on the bite scan when the case
//!    has one (`aligners.occlusion.bite-registration.icp`, a refusal stops
//!    the run, the estimate does not take its place), otherwise the virtual
//!    bite (`aligners.occlusion.virtual-bite`), which adds the plugin's
//!    warning to `warnings`. Arches already related (registered, or
//!    exported in occlusion) are not moved and the step is left out;
//! 8. setup (`aligners.planning.setup.automatic`);
//! 9. staging (`aligners.planning.staging.automatic`);
//! 10. schedule (`aligners.planning.schedule`);
//! 11. with both arches, the contact map (`aligners.occlusion.contact-map`,
//!     the summary `occlusion.auto_pair` refreshes after the analysis);
//!     steps 7 and 11 follow parameter `occlusion`: without them two arches
//!     not related stop the run at the setup, which refuses them;
//! 12. with `export_dir`, the export (`aligners.output.export-stl`, or the
//!     full package `aligners.output.report` with `full_report`). With
//!     `shells` the full package has the print files of the shell stage
//!     (printable models, trimmed aligners, at `quality`, draft by default);
//!     without it the models are the deformed scans.
//!
//! When the arch step decided the only arch scan of the case by the shape
//! and the widths of its teeth alone, the arch is verified on the built
//! crowns as in the plugin (`dca_algo::arch_check`): the scan is analysed as
//! a lower arch first; after the reconstruction the incisor / molar ratio
//! and the width fit may move it to the upper arch (and back), and the
//! pipeline starts again from the numbering on the moved scan, at most three
//! builds in all (output `arch_check`). With any other cue an unusual ratio
//! is only reported (`warnings`).
//!
//! The first step that fails stops the pipeline: the error names the step
//! and the case is left as it was (the command is one undoable change). The
//! output lists every step with its seconds and its output. What the check
//! of the separated teeth left implausible on each arch is in
//! `segmentation_doubts` (by arch) and one warning per arch in `warnings`.
//!
//! The menu leaf "Run everything automatically" runs it with no parameters.
//! The other leaves run the stages one at a time.

use std::collections::BTreeMap;
use std::time::Instant;

use dc_case::{CaseState, Fdi, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::arch_check::{self, ATTEMPTS, Trial};
use dca_algo::export::provider::Quality;
use dca_model::{AlignersState, MODULE_ID};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::import::DETECT_ARCH;
use super::messages;
use super::seed_check::CHECK;
use super::teeth_build::RECONSTRUCT;
use super::wrappers::{VIRTUAL_BITE, bite_steps};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.auto",
        label: "aligners-menu-auto",
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
    /// With both arches, put them in occlusion before the final position
    /// when they are not related, and measure the contacts at the end.
    /// Without it, two arches not related stop the run at the setup.
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
    /// Put the print files of the shell stage in the export (printable
    /// models and trimmed aligners; implies `full_report`).
    pub shells: bool,
    /// Quality of those files (detail and surface tolerance).
    pub quality: Quality,
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
            shells: false,
            quality: Quality::Draft,
        }
    }
}

/// One step of the pipeline: a command and its parameters.
pub(super) struct Step {
    pub(super) id: &'static str,
    params: Value,
    /// The step runs only when the two arches are not related when it comes
    /// ([`dca_algo::occlusion::bite_needed`]); otherwise it is left out.
    unless_related: bool,
}

impl Step {
    pub(super) fn new(id: &'static str, params: Value) -> Self {
        Self {
            id,
            params,
            unless_related: false,
        }
    }

    /// This step, run only when the two arches are not related when it
    /// comes.
    fn unless_related(self) -> Self {
        Self {
            unless_related: true,
            ..self
        }
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
            .ok_or_else(crate::commands::messages::no_data)?,
    };
    let mut steps = Vec::new();
    for arch in arches(state) {
        let has = seeds.iter().any(|s| Arch::of(s.fdi) == arch);
        let run = match params.seeds {
            Seeds::All => true,
            Seeds::Missing => !has,
            Seeds::Keep if !has => {
                return Err(messages::invalid(
                    "aligners-err-auto-keep-seeds",
                    &[("arch", &messages::arch_word(arch))],
                ));
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

/// The check of the segmented teeth of each arch
/// (`case_auto.check_segmentation_iter`): the seeds the numbering proposed
/// in this run are corrected, the seeds placed by hand only checked.
fn check_steps(state: &CaseState, params: &Params, numbered: &[Arch]) -> Vec<Step> {
    arches(state)
        .into_iter()
        .map(|arch| {
            Step::new(
                CHECK,
                json!({
                    "arch": arch_name(arch),
                    "correct": numbered.contains(&arch),
                    "third_molars": params.third_molars,
                    "missing": params.missing,
                }),
            )
        })
        .collect()
}

/// The steps after the numbering, in pipeline order.
fn later_steps(state: &CaseState, params: &Params, numbered: &[Arch]) -> Vec<Step> {
    let mut steps = vec![Step::new(
        "aligners.preparation.segmentation.automatic",
        Value::Null,
    )];
    steps.extend(check_steps(state, params, numbered));
    steps.extend([
        Step::new("aligners.preparation.reconstruction.automatic", Value::Null),
        Step::new("aligners.preparation.gingiva", Value::Null),
        Step::new("aligners.diagnosis.space-analysis", Value::Null),
    ]);
    let both =
        state.case.scan(ScanRole::Upper).is_some() && state.case.scan(ScanRole::Lower).is_some();
    let [bite, contacts] = bite_steps(state).1;
    let occlusion = params.occlusion && both;
    // `case_auto.auto_steps`: with both arches the bite comes before the
    // final position, when they are not related yet
    if occlusion {
        steps.push(bite.unless_related());
    }
    steps.extend([
        Step::new("aligners.planning.setup.automatic", Value::Null),
        Step::new("aligners.planning.staging.automatic", Value::Null),
        Step::new(
            "aligners.planning.schedule",
            Value::Object(calendar(params)),
        ),
    ]);
    if occlusion {
        steps.push(contacts);
    }
    if let Some(dir) = params.export_dir.as_ref().filter(|d| !d.trim().is_empty()) {
        let mut p = calendar(params);
        p.insert("dir".to_owned(), json!(dir));
        let id = if params.full_report || params.shells {
            p.insert("shells".to_owned(), json!(params.shells));
            if params.shells {
                p.insert("quality".to_owned(), json!(params.quality));
            }
            "aligners.output.report"
        } else {
            "aligners.output.export-stl"
        };
        steps.push(Step::new(id, Value::Object(p)));
    }
    steps
}

/// The steps after the arch step, for the state that step left.
pub(super) fn plan(state: &CaseState, params: &Params) -> Result<Vec<Step>, CommandError> {
    let mut steps = numbering(state, params)?;
    let numbered: Vec<Arch> = steps
        .iter()
        .filter_map(|s| s.params["arch"].as_str())
        .map(|a| {
            if a == "upper" {
                Arch::Upper
            } else {
                Arch::Lower
            }
        })
        .collect();
    steps.push(Step::new(
        "aligners.preparation.orientation.automatic",
        Value::Null,
    ));
    steps.extend(later_steps(state, params, &numbered));
    Ok(steps)
}

thread_local! {
    /// The part of the progress bar the step running on this thread owns.
    static SPAN: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with `[from, to]` as the part of the progress bar of the step
/// it runs, so that a stage that reports its own progress
/// ([`step_fraction`]) moves the bar inside it.
pub(super) fn with_span<T>(from: f32, to: f32, f: impl FnOnce() -> T) -> T {
    let old = SPAN.replace(Some((from, to)));
    let out = f();
    SPAN.set(old);
    out
}

/// The fraction of the whole job when the running stage is at `x` (0-1) of
/// its work: inside the span of its step, or from where the bar is to the
/// end when it runs on its own.
pub(super) fn step_fraction(ctx: &JobCtx) -> impl Fn(f64) -> f32 + Sync + use<> {
    let (a, b) = SPAN.get().unwrap_or((ctx.progress().fraction, 1.0));
    move |x| a + (b - a) * x.clamp(0.0, 1.0) as f32
}

/// Runs one step on `state`; the error names it by its label, in the words
/// the user sees.
pub(super) fn run_step(
    state: &CaseState,
    step: &Step,
    k: usize,
    ctx: &JobCtx,
) -> Result<(CaseState, Value), CommandError> {
    let spec = crate::commands::find(step.id)
        .ok_or_else(|| CommandError::Failed(format!("the step {} is not registered", step.id)))?;
    ctx.check_cancelled()?;
    // the status bar names the step that runs, keeping the fraction done
    ctx.set_progress(ctx.progress().fraction, Some(spec.label_key));
    let t0 = Instant::now();
    let (next, out) = spec.execute(state, &step.params, ctx).map_err(|e| {
        let at = |m: String| {
            messages::tr(
                "aligners-err-stopped-at",
                &[("step", &messages::tr(spec.label_key, &[])), ("reason", &m)],
            )
        };
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

/// The arch scan the arch step decided only by the shape and the widths of
/// its teeth, when it is the only arch scan of the case: the plugin checks
/// that arch on the built crowns (`case_auto.auto_steps`, `anatomical`).
fn anatomical(state: &CaseState, detect: &Value) -> Option<Arch> {
    let only = match arches(state).as_slice() {
        [a] => *a,
        _ => return None,
    };
    let decided = detect["output"]["scans"]
        .as_array()?
        .iter()
        .any(|s| s["detection"]["source"] == "shape_and_widths" && s["to"] == arch_name(only));
    decided.then_some(only)
}

/// The scan of `from` moved to the opposite arch, its results cleared (as
/// the arch step does when it moves a scan).
fn move_arch(state: &CaseState, from: Arch, to: Arch) -> Result<CaseState, CommandError> {
    let mut next = state.clone();
    let before = next.referenced_blobs();
    for scan in &mut next.case.scans {
        if scan.role == from.role() {
            scan.role = to.role();
        }
    }
    next.case.scans.sort_by_key(|s| s.role);
    let touched = [from.role(), to.role()].into_iter().collect();
    dc_case::import::roles_changed(&mut next, &touched, &before)
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    Ok(next)
}

/// The mesio-distal widths of the built teeth of `arch`.
fn widths(state: &CaseState, arch: Arch) -> Result<BTreeMap<Fdi, f64>, CommandError> {
    let data = super::reconstruct::aligners_data(state)?;
    Ok(data
        .tooth_geometry
        .iter()
        .filter(|(f, _)| Arch::of(**f) == arch)
        .map(|(f, m)| (*f, m.width_md))
        .collect())
}

fn run_auto(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if arches(state).is_empty() {
        return Err(messages::invalid("aligners-arch-error-no-scan", &[]));
    }
    let t0 = Instant::now();
    let arch_step = Step::new(DETECT_ARCH, Value::Null);
    let (mut current, record) = run_step(state, &arch_step, 0, ctx)?;
    // an arch decided by the widths alone is analysed as a lower arch first
    // and verified on the crowns (not when the seeds must be kept)
    let verify = anatomical(&current, &record).filter(|_| params.seeds != Seeds::Keep);
    let mut arch_check = Map::new();
    let mut warnings = Vec::new();
    if let Some(a) = verify {
        arch_check.insert("detected".to_owned(), json!(arch_name(a)));
        if a == Arch::Upper {
            current = move_arch(&current, a, Arch::Lower)?;
        }
    }
    let mut verified = verify.map(|_| Arch::Lower);
    let mut tried: BTreeMap<Arch, Trial> = BTreeMap::new();
    let mut attempts = 0;
    let mut steps = plan(&current, &params)?;

    let mut records = vec![record];
    let mut i = 0;
    while i < steps.len() {
        // the bite before the final position: only for arches not related yet
        if steps[i].unless_related && !dca_algo::occlusion::bite_needed(&current) {
            i += 1;
            continue;
        }
        let n = (steps.len() + 1) as f32;
        let (from, to) = ((i + 1) as f32 / n, (i + 2) as f32 / n);
        ctx.set_fraction(from);
        let k = records.len();
        let (next, record) = with_span(from, to, || run_step(&current, &steps[i], k, ctx))?;
        current = next;
        records.push(record);
        if steps[i].unless_related && steps[i].id == VIRTUAL_BITE {
            // `case_auto.auto_steps`: the relation is estimated, not the patient's
            warnings.push(messages::tr("aligners-auto-warn-virtual-bite", &[]));
        }
        let mut restart = false;
        if steps[i].id == RECONSTRUCT {
            for arch in arches(&current) {
                let trial = Trial::of(&widths(&current, arch)?);
                if verified == Some(arch) {
                    attempts += 1;
                    tried.insert(arch, trial);
                    if attempts >= ATTEMPTS {
                        continue;
                    }
                    if let Some(other) = arch_check::next_arch(arch, &tried) {
                        // the crowns contradict the arch: the other one, from the seeds
                        current = move_arch(&current, arch, other)?;
                        verified = Some(other);
                        steps = plan(&current, &params)?;
                        restart = true;
                        break;
                    }
                } else if verify.is_none() && arch_check::ratio_unusual(arch, trial.ratio) {
                    warnings.push(messages::tr(
                        "aligners-auto-warn-ratio",
                        &[
                            ("ratio", &format!("{:.2}", trial.ratio.unwrap_or_default())),
                            ("arch", &messages::arch_word(arch)),
                        ],
                    ));
                }
            }
        }
        i = if restart { 0 } else { i + 1 };
    }
    if let Some(a) = verified {
        arch_check.insert("arch".to_owned(), json!(arch_name(a)));
        arch_check.insert(
            "tried".to_owned(),
            Value::Object(
                tried
                    .iter()
                    .map(|(a, t)| (arch_name(*a).to_owned(), json!(t)))
                    .collect(),
            ),
        );
    }
    // what the check of the separated teeth left implausible: the aligners
    // must not be delivered before the tooth marks are checked
    let (doubts, doubt_warnings) = match current.module_data::<AlignersState>(MODULE_ID) {
        Some(d) => messages::segmentation_doubts(d),
        None => (json!({}), Vec::new()),
    };
    warnings.extend(doubt_warnings);
    ctx.set_fraction(1.0);
    let out = json!({
        "steps": records,
        "arch_check": arch_check,
        "warnings": warnings,
        "segmentation_doubts": doubts,
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
    fn it_is_registered_and_a_menu_leaf_that_is_not_a_step() {
        let spec = crate::commands::find("aligners.auto").unwrap();
        let schema = spec.schema();
        assert!(schema["properties"]["export_dir"].is_object(), "{schema}");
        let leaf = crate::tree::find("aligners.auto").unwrap();
        assert!(leaf.is_leaf() && !leaf.counts_as_step);
    }

    #[test]
    fn a_click_on_a_guessed_arch_stops_with_the_advice_and_no_command_id() {
        // a scan the geometry cannot place: it stays a guess, and from a click
        // (no parameters) the run stops at it and says what to do
        let mut s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        s.case.scans[0].role_source = dc_case::RoleSource::Provisional;
        s.case.scans[0].source_name = "scan.stl".to_owned();
        let tiny = dc_mesh::TriMesh::new(
            vec![
                glam::Vec3::ZERO,
                glam::Vec3::X,
                glam::Vec3::Y,
                glam::Vec3::Z,
            ],
            vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        );
        let blob = s.blobs.insert(tiny);
        s.case.scans[0].mesh = blob;
        let m = exec("aligners.auto", &s, Value::Null)
            .unwrap_err()
            .to_string();
        assert!(m.contains("Stopped at Arch recognition"), "{m}");
        assert!(m.contains("scan.stl"), "{m}");
        assert!(!m.contains("aligners."), "{m}");
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
        assert!(
            matches!(err, CommandError::Invalid(m) if m.contains("lower arch has no tooth marks"))
        );
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
    fn kept_marks_that_cannot_place_the_arch_stop_the_run_at_the_separation() {
        let mut s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.seeds.truncate(3);
        s.set_module(MODULE_ID, dc_case::ModuleState::new(data));
        let err = exec("aligners.auto", &s, json!({"seeds": "keep"})).unwrap_err();
        let CommandError::Invalid(m) = err else {
            panic!("not a refusal: {err:?}");
        };
        let step = messages::tr("aligners-menu-preparation-segmentation-automatic", &[]);
        assert!(m.starts_with(&format!("Stopped at {step}")), "{m}");
        assert!(
            m.contains("lower arch") && m.contains("fewer than 4"),
            "{m}"
        );
    }

    #[test]
    fn the_separation_of_the_run_uses_the_settings_of_the_case() {
        let mut s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        // a value outside the sliders (a project edited by hand): the run
        // stops at the separation, which took it from the settings
        data.settings.segmentation.target_faces = 4321;
        s.set_module(MODULE_ID, dc_case::ModuleState::new(data));
        let err = exec("aligners.auto", &s, json!({"seeds": "keep"})).unwrap_err();
        let CommandError::Invalid(m) = err else {
            panic!("not a refusal: {err:?}");
        };
        let step = messages::tr("aligners-menu-preparation-segmentation-automatic", &[]);
        assert!(m.starts_with(&format!("Stopped at {step}")), "{m}");
        assert!(m.contains("target_faces") && m.contains("4321"), "{m}");
    }

    #[test]
    fn it_plans_after_the_arch_step() {
        // the two demo arches on the wrong sides, as a guess
        let mut s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({"arch": "lower"}),
        )
        .unwrap()
        .0;
        s = exec("aligners.demo", &s, json!({"arch": "upper"}))
            .unwrap()
            .0;
        for scan in &mut s.case.scans {
            scan.role = if scan.role == ScanRole::Upper {
                ScanRole::Lower
            } else {
                ScanRole::Upper
            };
            scan.role_source = dc_case::RoleSource::Provisional;
        }
        s.case.scans.sort_by_key(|scan| scan.role);
        // from this state: nothing to number, the seeds are there
        let keep = Params {
            seeds: Seeds::Keep,
            ..Params::default()
        };
        assert!(numbering(&s, &keep).unwrap().is_empty());
        // the arch step swaps the scans and clears the seeds: the plan sees it
        let after = exec(DETECT_ARCH, &s, Value::Null).unwrap().0;
        let ids =
            |p: &Params| -> Vec<&str> { plan(&after, p).unwrap().iter().map(|s| s.id).collect() };
        let numberings = ids(&Params::default())
            .iter()
            .filter(|id| **id == "aligners.preparation.numbering.automatic")
            .count();
        assert_eq!(numberings, 2);
        // the orientation comes right after the numbering, from its seeds
        assert_eq!(
            ids(&Params::default())[2],
            "aligners.preparation.orientation.automatic"
        );
        // and so does the command, which stops at the plan and not at the start
        let err = exec("aligners.auto", &s, json!({"seeds": "keep"})).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("arch has no tooth marks")));
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
        let ids: Vec<&str> = later_steps(&s, &p, &[]).iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "aligners.preparation.segmentation.automatic",
                CHECK,
                CHECK,
                "aligners.preparation.reconstruction.automatic",
                "aligners.preparation.gingiva",
                "aligners.diagnosis.space-analysis",
                "aligners.occlusion.virtual-bite",
                "aligners.planning.setup.automatic",
                "aligners.planning.staging.automatic",
                "aligners.planning.schedule",
                "aligners.occlusion.contact-map",
                "aligners.output.export-stl",
            ]
        );
        for id in ids {
            assert!(crate::commands::is_registered(id), "{id}");
        }
        let steps = later_steps(&s, &p, &[]);
        // only the bite before the setup depends on the relation of the arches
        let when: Vec<bool> = steps.iter().map(|s| s.unless_related).collect();
        assert_eq!(when.iter().filter(|w| **w).count(), 1);
        assert!(when[6]);
        assert_eq!(steps[9].params["start_date"], "2026-01-05");
        assert_eq!(steps[11].params["dir"], "x");
        // the seeds of the demo were placed before the run: only checked
        assert_eq!(steps[1].params["correct"], false);
        let numbered = later_steps(&s, &p, &[Arch::Lower]);
        assert_eq!(numbered[1].params["arch"], "lower");
        assert_eq!(numbered[1].params["correct"], true);
        assert_eq!(numbered[2].params["correct"], false);
    }

    /// The plugin's `steps.ORDER`, with the bite `case_auto.auto_steps` runs
    /// in the setup step before the final position (`morso`); the
    /// classifications are not ported.
    const PLUGIN_ORDER: [&str; 8] = [
        "arcata",
        "semi",
        "segmentazione",
        "denti",
        "diagnosi",
        "morso",
        "setup",
        "staging",
    ];

    /// The step of the plugin a step of the run belongs to; `None` for the
    /// steps the plugin runs after the analysis (the contact map of
    /// `occlusion.auto_pair`) or not at all (the export).
    fn plugin_step(id: &str) -> Option<&'static str> {
        Some(match id {
            DETECT_ARCH => "arcata",
            "aligners.preparation.numbering.automatic"
            | "aligners.preparation.orientation.automatic" => "semi",
            "aligners.preparation.segmentation.automatic" | CHECK => "segmentazione",
            RECONSTRUCT | "aligners.preparation.gingiva" => "denti",
            "aligners.diagnosis.space-analysis" => "diagnosi",
            super::super::wrappers::REGISTER | VIRTUAL_BITE => "morso",
            "aligners.planning.setup.automatic" => "setup",
            "aligners.planning.staging.automatic" | "aligners.planning.schedule" => "staging",
            _ => return None,
        })
    }

    #[test]
    fn the_steps_follow_the_order_of_the_plugin() {
        use super::super::occlusion::tests::two_demo_arches;
        let mut s = two_demo_arches();
        let p = Params {
            seeds: Seeds::All,
            export_dir: Some("x".to_owned()),
            ..Params::default()
        };
        for bite_scan in [false, true] {
            if bite_scan {
                let mut bite = s.case.scan(ScanRole::Lower).unwrap().clone();
                bite.role = ScanRole::Bite;
                s.case.scans.push(bite);
            }
            let ids: Vec<&str> = std::iter::once(DETECT_ARCH)
                .chain(plan(&s, &p).unwrap().iter().map(|s| s.id))
                .collect();
            let mut order: Vec<&str> = ids.iter().filter_map(|id| plugin_step(id)).collect();
            order.dedup();
            assert_eq!(order, PLUGIN_ORDER, "{ids:?}");
            // what the plugin does after the analysis comes after it
            let last = ids
                .iter()
                .rposition(|id| plugin_step(id).is_some())
                .unwrap();
            assert_eq!(
                ids[last + 1..],
                [
                    "aligners.occlusion.contact-map",
                    "aligners.output.export-stl"
                ]
            );
        }
    }

    #[test]
    fn the_bite_is_the_one_of_the_menu_leaf_and_a_refusal_is_not_replaced() {
        use super::super::occlusion::tests::two_demo_arches;
        let mut s = two_demo_arches();
        let ids = |s: &CaseState| -> Vec<&'static str> {
            later_steps(s, &Params::default(), &[])
                .iter()
                .map(|s| s.id)
                .filter(|id| id.starts_with("aligners.occlusion."))
                .collect()
        };
        assert_eq!(
            ids(&s),
            [
                "aligners.occlusion.virtual-bite",
                "aligners.occlusion.contact-map"
            ],
            "no bite scan: the estimate"
        );
        let mut bite = s.case.scan(ScanRole::Lower).unwrap().clone();
        bite.role = ScanRole::Bite;
        s.case.scans.push(bite);
        assert_eq!(
            ids(&s),
            [
                "aligners.occlusion.bite-registration.icp",
                "aligners.occlusion.contact-map"
            ],
            "a bite scan: the registration"
        );
        let no_bite = Params {
            occlusion: false,
            ..Params::default()
        };
        assert!(
            later_steps(&s, &no_bite, &[])
                .iter()
                .all(|s| !s.id.starts_with("aligners.occlusion."))
        );
    }

    #[test]
    fn arches_not_related_get_the_virtual_bite_before_the_setup() {
        use super::super::occlusion::tests::two_demo_arches;
        let s = two_demo_arches();
        let run = |s: &CaseState, params: Value| {
            let (next, out) = exec("aligners.auto", s, params).unwrap();
            let ids: Vec<String> = out["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].as_str().unwrap().to_owned())
                .collect();
            (next, out, ids)
        };
        let at = |ids: &[String], id: &str| ids.iter().position(|i| i == id);
        let warning = messages::tr("aligners-auto-warn-virtual-bite", &[]);
        let setup = "aligners.planning.setup.automatic";

        let (next, out, ids) = run(&s, Value::Null);
        let bite = at(&ids, VIRTUAL_BITE).expect("the virtual bite ran");
        assert_eq!(at(&ids, setup), Some(bite + 1), "{ids:?}");
        assert_eq!(ids.iter().filter(|i| *i == VIRTUAL_BITE).count(), 1);
        assert_eq!(ids.last().unwrap(), "aligners.occlusion.contact-map");
        assert_eq!(out["steps"][ids.len() - 1]["output"]["in_occlusion"], true);
        let warnings = out["warnings"].as_array().unwrap();
        assert!(warnings.iter().any(|w| *w == warning), "{warnings:?}");
        let d = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(d.bite_registered && !d.setup.poses.is_empty() && !d.stages.is_empty());

        // arches already related are not moved again: no bite, no warning
        let (again, out, ids) = run(&next, Value::Null);
        assert_eq!(at(&ids, VIRTUAL_BITE), None, "{ids:?}");
        assert!(at(&ids, setup).is_some());
        assert!(
            out["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|w| *w != warning)
        );
        let upper = |s: &CaseState| s.case.scan(ScanRole::Upper).unwrap().mesh;
        assert_eq!(upper(&again), upper(&next));

        // without the occlusion the setup refuses the arches not related
        let err = exec("aligners.auto", &s, json!({ "occlusion": false })).unwrap_err();
        let CommandError::Invalid(m) = err else {
            panic!("not a refusal: {err:?}");
        };
        let step = messages::tr("aligners-menu-planning-setup-automatic", &[]);
        assert!(m.starts_with(&format!("Stopped at {step}")), "{m}");
        assert!(m.contains("the bite comes first"), "{m}");
    }

    #[test]
    fn an_arch_decided_by_the_widths_alone_is_verified_and_can_be_moved() {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0;
        let record = |source: &str| {
            json!({ "output": { "scans": [
                { "to": "lower", "detection": { "source": source } }
            ] } })
        };
        assert_eq!(
            anatomical(&s, &record("shape_and_widths")),
            Some(Arch::Lower)
        );
        assert_eq!(anatomical(&s, &record("palate")), None);
        // the widths of the demo teeth say lower: kept
        let built = exec("aligners.teeth.frames", &s, Value::Null).unwrap().0;
        let trial = Trial::of(&widths(&built, Arch::Lower).unwrap());
        let tried = BTreeMap::from([(Arch::Lower, trial)]);
        assert_eq!(
            arch_check::next_arch(Arch::Lower, &tried),
            None,
            "{trial:?}"
        );
        // moved: the scan is upper and what was computed on it is gone
        let moved = move_arch(&built, Arch::Lower, Arch::Upper).unwrap();
        assert!(moved.case.scan(ScanRole::Upper).is_some());
        assert!(moved.case.scan(ScanRole::Lower).is_none());
        let d = super::super::reconstruct::aligners_data(&moved).unwrap();
        assert!(d.seeds.is_empty() && d.tooth_frames.is_empty(), "cleared");
        // with both arches nothing is questioned
        let two = super::super::occlusion::tests::two_demo_arches();
        assert_eq!(anatomical(&two, &record("shape_and_widths")), None);
    }
}
