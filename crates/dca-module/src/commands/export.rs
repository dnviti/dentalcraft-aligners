//! Commands of the `export` stage (`dca_algo::export`).
//!
//! * `aligners.output.export-stl` writes the aligners into a target
//!   directory: the scan of each arch with the teeth at every stage as binary
//!   STL (`upper_stage_00.stl` is the initial situation), `stages.csv` with
//!   the movement of every tooth at every stage and the dates, and
//!   `visits.ics` with the aligner changes of both arches. Parameters:
//!   `dir`, `start_date`, `interval_days`, `policy`, `lang`; the calendar
//!   values not given are those of the case settings.
//! * `aligners.output.report` is the full export package: the models and
//!   the aligners of every stage, the print check of every mesh, the tables
//!   and calendars, the programme of the patient for each arch, the HTML
//!   report with two renders, `print_check.json` and `manifest.json`, in
//!   parallel. See [`dca_algo::export::package`].
//! * `aligners.output.print-check` runs the print check on the models (and
//!   the aligners, from the shell stage) of some stages without writing
//!   anything.
//! * `aligners.output.manifest` verifies a folder against its
//!   `manifest.json`, or writes a manifest for a folder that has none.
//!
//! The case is not modified. In the models the gingiva follows the teeth
//! when the arch has a stored gingiva (`aligners.preparation.gingiva`,
//! [`dca_algo::export::GumDeform`]); otherwise it stays where it is.
//!
//! The outputs of `export-stl` and `report` carry the doubts the check of
//! the separated teeth left in the case (`segmentation_doubts`, by arch) and
//! a warning for each arch that has some (`warnings`); the report lists them
//! first among its notes, as the plugin's export and report warn.
//!
//! # Where the meshes come from
//!
//! [`mesh_provider`]: when every arch with aligners has its closed teeth and
//! its gingiva, the shell stage ([`shells::ShellProvider`]: the printable
//! model with the stamp embossed, the trimmed aligner with the ramps, the
//! occlusal shaping and the stamp engraved, at the voxel of the quality);
//! otherwise the deformed scans as the models and no aligner
//! ([`ScanProvider`]), with a note in the report saying why. `shells: false`
//! asks for the scans, `shells: true` refuses to fall back.

mod shells;

use std::collections::BTreeMap;
use std::path::Path;

use dc_case::{CaseState, Fdi, Pose, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::export::manifest;
use dca_algo::export::package::{self, CaseInfo, FullParams};
use dca_algo::export::printcheck::{self, CheckOptions};
use dca_algo::export::provider::{MeshProvider, Note, Quality, ScanProvider};
use dca_algo::export::report::Renders;
use dca_algo::export::{self, ArchSeries, GumDeform};
use dca_algo::schedule;
use dca_algo::teeth;
use dca_model::{AlignersState, Settings};
use schemars::JsonSchema;
use serde::Deserialize;

use super::messages;
use super::staging::{LabelledScan, aligners_data, labelled_scan, md_sign};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.output.export-stl",
            label: "aligners-cmd-export",
            doc: "aligners-cmd-export-doc",
            params: dca_algo::export::Params,
            run: run_export,
        },
        stage_command! {
            id: "aligners.output.print-check",
            label: "aligners-cmd-print-check",
            doc: "aligners-cmd-print-check-doc",
            params: PrintCheckParams,
            run: run_print_check,
        },
        stage_command! {
            id: "aligners.output.manifest",
            label: "aligners-cmd-manifest",
            doc: "aligners-cmd-manifest-doc",
            params: ManifestParams,
            run: run_manifest,
        },
        stage_command! {
            id: "aligners.output.report",
            label: "aligners-cmd-report",
            doc: "aligners-cmd-report-doc",
            params: FullParams,
            run: run_report,
        },
    ]
}

/// The scan, rest poses and mesial-distal signs of an arch with aligners.
struct ArchData {
    arch: Arch,
    scan: LabelledScan,
    rest: BTreeMap<Fdi, Pose>,
    md: BTreeMap<Fdi, i8>,
    gum: Option<GumDeform>,
}

/// The arches that have aligners and a segmented scan; an error when there
/// are none.
fn prepare(state: &CaseState, data: &AlignersState) -> Result<Vec<ArchData>, CommandError> {
    if data.stages.is_empty() {
        return Err(messages::invalid("aligners-err-no-aligners", &[]));
    }
    let mut out = Vec::new();
    for arch in [Arch::Upper, Arch::Lower] {
        if schedule::active_stages(arch, &data.stages) == 0 {
            continue;
        }
        // the scan of the case: a scan closed with a base is printed with it
        if let Some(scan) = labelled_scan(state, data, arch.role())? {
            let fdis: Vec<Fdi> = data
                .tooth_frames
                .keys()
                .copied()
                .filter(|&f| Arch::of(f) == arch)
                .collect();
            let seq = anatomy::arch_sequence(&fdis, arch);
            let rest: BTreeMap<_, _> = seq
                .iter()
                .map(|&f| (f, teeth::rest_pose(&data.tooth_frames[&f])))
                .collect();
            let md: BTreeMap<_, _> = seq
                .iter()
                .map(|&f| (f, md_sign(data, &seq, &rest, f)))
                .collect();
            // the gingiva was built on the scan as it was read
            let gum = match data.gingiva.get(&arch.role()) {
                Some(r) => Some(
                    GumDeform::new(
                        state.blobs.require(r)?,
                        scan.scanned,
                        &dca_algo::gingiva::Params::default(),
                    )
                    .map_err(|e| CommandError::Invalid(format!("the stored gingiva: {e}")))?,
                ),
                None => None,
            };
            out.push(ArchData {
                arch,
                scan,
                rest,
                md,
                gum,
            });
        }
    }
    if out.is_empty() {
        return Err(messages::none_segmented());
    }
    Ok(out)
}

fn input<'a>(arches: &'a [ArchData], data: &'a AlignersState) -> export::Input<'a> {
    export::Input {
        arches: arches
            .iter()
            .map(|a| ArchSeries {
                arch: a.arch,
                mesh: &a.scan.mesh,
                labels: &a.scan.labels,
                rest: &a.rest,
                md_sign: &a.md,
                gingiva: a.gum.as_ref(),
            })
            .collect(),
        stages: &data.stages,
    }
}

/// What decides where the meshes come from.
struct MeshChoice {
    /// `None`: the shells when the case has what they need.
    shells: Option<bool>,
    quality: Quality,
    voxel_mm: Option<f64>,
    stamp: bool,
    stamp_code: String,
    date: schedule::Date,
    /// The first day of every aligner when the stamp carries it (empty: the
    /// stamp carries `date`).
    stamp_dates: BTreeMap<(Arch, u32), schedule::Date>,
    /// Stops the building of the meshes between their steps.
    cancel: dc_command::CancelToken,
}

/// The source of the printable meshes of every stage, its name for the
/// manifest (`shells`, `scans`) and the note that says why when it is the
/// scans.
type Provider<'a> = (Box<dyn MeshProvider + 'a>, &'static str, Vec<Note>);

/// Where the printable meshes of every stage come from (see the module
/// docs).
fn mesh_provider<'a>(
    state: &CaseState,
    data: &'a AlignersState,
    input: &'a export::Input<'a>,
    choice: &MeshChoice,
) -> Result<Provider<'a>, CommandError> {
    let scans = |why: String| -> Provider<'a> {
        let note = Note {
            arch: None,
            stage: None,
            aligner: false,
            key: "rpt-note-scans",
            detail: why,
        };
        (Box::new(ScanProvider::new(input)), "scans", vec![note])
    };
    if choice.shells == Some(false) {
        return Ok(scans("shells: false".to_owned()));
    }
    let voxel = choice.voxel_mm.unwrap_or(choice.quality.voxel_mm());
    if !(voxel.is_finite() && (0.04..=0.5).contains(&voxel)) {
        return Err(CommandError::Invalid(format!(
            "voxel_mm must be between 0.04 and 0.5, got {voxel}"
        )));
    }
    let options = shells::Options {
        voxel_mm: voxel,
        tolerance_mm: choice.quality.tolerance_mm(),
        stamp: choice.stamp,
        stamp_code: choice.stamp_code.clone(),
        date: choice.date,
        stamp_dates: choice.stamp_dates.clone(),
        cancel: choice.cancel.clone(),
        checked: true,
    };
    match shells::ShellProvider::new(state, data, input, options)? {
        Ok(p) => {
            let note = Note {
                arch: None,
                stage: None,
                aligner: false,
                key: "rpt-note-shells",
                detail: format!(
                    "resolution {voxel} mm, tolerance {} mm",
                    choice.quality.tolerance_mm()
                ),
            };
            Ok((Box::new(p), "shells", vec![note]))
        }
        Err(why) if choice.shells == Some(true) => Err(CommandError::Invalid(why)),
        Err(why) => Ok(scans(why)),
    }
}

/// The date of the files: `generated`, else today.
fn export_date(generated: Option<&str>) -> Result<schedule::Date, CommandError> {
    match generated {
        Some(g) => schedule::Date::parse(g)
            .ok_or_else(|| CommandError::Invalid(format!("generated: invalid date {g:?}"))),
        None => Ok(schedule::Date::today()),
    }
}

fn export_error(e: export::ExportError) -> CommandError {
    match e {
        export::ExportError::Cancelled => CommandError::Cancelled,
        export::ExportError::Io { .. }
        | export::ExportError::Mesh(_)
        | export::ExportError::Package(_) => CommandError::Failed(e.to_string()),
        other => CommandError::Invalid(other.to_string()),
    }
}

/// The start date, days and policy of the calendar not given: those of the
/// case settings (no start date in the case either: today, in the export).
fn calendar_defaults(
    start_date: &mut Option<String>,
    interval_days: &mut Option<u32>,
    policy: &mut Option<schedule::Policy>,
    settings: &Settings,
) {
    if start_date.is_none() {
        start_date.clone_from(&settings.start_date);
    }
    interval_days.get_or_insert(settings.interval_days);
    policy.get_or_insert(settings.interval_policy.into());
}

fn run_export(
    state: &CaseState,
    mut params: export::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if data.stages.is_empty() {
        return Err(messages::invalid("aligners-err-no-aligners", &[]));
    }
    if params.dir.trim().is_empty() {
        return Err(CommandError::Invalid(
            "dir: the target directory is required".to_owned(),
        ));
    }
    calendar_defaults(
        &mut params.start_date,
        &mut params.interval_days,
        &mut params.policy,
        &data.settings,
    );
    let arches = prepare(state, &data)?;
    let input = input(&arches, &data);
    ctx.check_cancelled()?;
    let report = export::run(&input, &params, &attachments(&data)).map_err(export_error)?;
    let out = serde_json::to_value(&report).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((state.clone(), with_doubts(out, &data)))
}

/// The teeth with an attachment in the clinical review of each arch, bonded
/// before its first aligner (the arches without any are left out).
fn attachments(data: &AlignersState) -> BTreeMap<Arch, Vec<String>> {
    [Arch::Lower, Arch::Upper]
        .into_iter()
        .map(|a| (a, data.attachment_teeth(a.role())))
        .filter(|(_, teeth)| !teeth.is_empty())
        .collect()
}

/// `out` (an object) with the doubts the check of the separated teeth left
/// in the case (`segmentation_doubts`, by arch) and a warning for each arch
/// that has some (`warnings`), as the plugin's export warns: the aligners
/// must not be delivered before the tooth marks and their numbers are
/// checked.
fn with_doubts(mut out: serde_json::Value, data: &AlignersState) -> serde_json::Value {
    let (doubts, warnings) = messages::segmentation_doubts(data);
    if let serde_json::Value::Object(m) = &mut out {
        m.insert("segmentation_doubts".to_owned(), doubts);
        m.insert("warnings".to_owned(), serde_json::json!(warnings));
    }
    out
}

/// The notes of the report for the doubts of the case (first in its notes):
/// one per arch, with the doubts the plugin's report shows (at most six).
fn doubt_notes(data: &AlignersState) -> Vec<Note> {
    [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter_map(|arch| {
            let doubts = data
                .segmentation_doubts
                .get(&arch.role())
                .filter(|d| !d.is_empty())?;
            Some(Note {
                arch: Some(arch),
                stage: None,
                aligner: false,
                key: "rpt-note-doubts",
                detail: doubts
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; "),
            })
        })
        .collect()
}

// ---- report: the full export ---------------------------------------------------------------------------------

/// A coloured view of the case for the report: the upper arch seen from the
/// occlusal side (the lower one when there is no upper) and the front.
fn render_views(state: &CaseState, data: &AlignersState) -> Renders {
    use dc_raster::{RenderOptions, View, case_items, render};
    let png = |roles: &[ScanRole], view: View| -> Option<Vec<u8>> {
        let items = case_items(state, &data.labels, Some(roles));
        if items.is_empty() {
            return None;
        }
        render(&items, &RenderOptions::new(640, 480, view))
            .ok()?
            .to_png()
            .ok()
    };
    let has = |r: ScanRole| state.case.scan(r).is_some();
    let occlusal = if has(ScanRole::Upper) {
        png(&[ScanRole::Upper], View::OcclusalUpper)
    } else {
        png(&[ScanRole::Lower], View::OcclusalLower)
    };
    Renders {
        occlusal,
        frontal: png(&[ScanRole::Upper, ScanRole::Lower], View::Frontal),
    }
}

fn run_report(
    state: &CaseState,
    mut params: FullParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if params.dir.trim().is_empty() {
        return Err(CommandError::Invalid(
            "dir: the target directory is required".to_owned(),
        ));
    }
    let arches = prepare(state, &data)?;
    calendar_defaults(
        &mut params.start_date,
        &mut params.interval_days,
        &mut params.policy,
        &data.settings,
    );
    let input = input(&arches, &data);
    let choice = report_choice(&data, &input, &mut params, ctx.cancel_token().clone())?;
    let renders = if params.report && params.embed_renders {
        render_views(state, &data)
    } else {
        Renders::default()
    };
    ctx.check_cancelled()?;
    let case = CaseInfo {
        patient_code: &state.case.patient_code,
        state: &data,
        attachments: attachments(&data),
        renders,
        meshes: "",
        notes: Vec::new(),
    };
    let (provider, meshes, notes) = mesh_provider(state, &data, &input, &choice)?;
    let case = CaseInfo {
        meshes,
        notes: doubt_notes(&data).into_iter().chain(notes).collect(),
        ..case
    };
    let control = package::Control {
        cancelled: &|| ctx.is_cancelled(),
        progress: &|f| ctx.set_fraction(f),
    };
    let report = package::run_cancellable(&input, &case, provider.as_ref(), &params, &control)
        .map_err(export_error)?;
    let out = serde_json::to_value(&report).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((state.clone(), with_doubts(out, &data)))
}

/// How `aligners.output.report` chooses its print files from `params`: the
/// quality, the stamp and its code the call does not give are those of the
/// case settings (written back into `params`, which the package reads), the
/// stamp carries the day of the export or the first day of every aligner.
/// The aligner of the 3D view built as the export writes it
/// (`aligners.view.aligner-exact`) takes the same choice.
fn report_choice(
    data: &AlignersState,
    input: &export::Input<'_>,
    params: &mut FullParams,
    cancel: dc_command::CancelToken,
) -> Result<MeshChoice, CommandError> {
    let stamp_settings = &data.settings.stamp;
    let quality = *params.quality.get_or_insert(data.settings.quality);
    let stamp = *params.stamp.get_or_insert(stamp_settings.enabled);
    if params.stamp_code.trim().is_empty() {
        params.stamp_code = stamp_settings.code.clone();
    }
    // the stamp carries the day of the export or the first day of the aligner
    let stamp_dates = if stamp_settings.date == dca_model::StampDate::Start {
        export::start_dates(input, &params.base())
    } else {
        BTreeMap::new()
    };
    Ok(MeshChoice {
        shells: params.shells,
        quality,
        voxel_mm: params.voxel_mm,
        stamp,
        stamp_code: params.stamp_code.clone(),
        date: export_date(params.generated.as_deref())?,
        stamp_dates,
        cancel,
    })
}

// ---- the aligner of the 3D view ----------------------------------------------------------------------------------

/// How the aligner of the 3D view is built (`aligners.view.*`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ViewBuild {
    /// The preview (the plugin's `aligner_solid` at `preview_params`): the
    /// construction of the print file at `voxel_mm`, reduced within
    /// `tolerance_mm` as it comes, without the stamp and the checks of the
    /// print files.
    Preview {
        /// Detail of the fields (mm).
        voxel_mm: f64,
        /// Tolerance of the surface (mm).
        tolerance_mm: f64,
    },
    /// The aligner `aligners.output.export-all` writes, at `quality`.
    Exact {
        /// Quality of the print files.
        quality: Quality,
    },
}

/// An aligner built for the 3D view (case coordinates).
pub(crate) struct ViewAligner {
    pub arch: Arch,
    pub stage: u32,
    pub mesh: dc_mesh::TriMesh,
    pub seconds: f64,
}

/// The aligner of every `(arch, stage)` of `wanted` built for the 3D view by
/// the provider of the export, as `build` says, the arches at the same time;
/// also the notes of the provider. The arches of the input are those with
/// aligners, a separated scan, closed teeth and a gingiva: with both, the
/// input of the export, so an exact aligner is the solid the export writes.
pub(crate) fn view_aligners(
    state: &CaseState,
    data: &AlignersState,
    wanted: &[(Arch, u32)],
    build: ViewBuild,
    ctx: &JobCtx,
) -> Result<(Vec<ViewAligner>, Vec<Note>), CommandError> {
    let built = |a: Arch| {
        data.gingiva.contains_key(&a.role()) && data.crowns.keys().any(|f| Arch::of(*f) == a)
    };
    let arches: Vec<ArchData> = prepare(state, data)?
        .into_iter()
        .filter(|a| built(a.arch))
        .collect();
    if let Some((a, _)) = wanted
        .iter()
        .find(|(a, _)| !arches.iter().any(|d| d.arch == *a))
    {
        return Err(messages::invalid(
            "aligners-err-no-3d-teeth",
            &[("arch", &messages::arch_word(*a))],
        ));
    }
    let input = input(&arches, data);
    let provider: Box<dyn MeshProvider + '_> = match build {
        ViewBuild::Exact { quality } => {
            // `aligners.output.export-all`: the shells required, the quality
            // asked, the rest from the case settings
            let mut params = FullParams {
                shells: Some(true),
                quality: Some(quality),
                ..FullParams::default()
            };
            calendar_defaults(
                &mut params.start_date,
                &mut params.interval_days,
                &mut params.policy,
                &data.settings,
            );
            let choice = report_choice(data, &input, &mut params, ctx.cancel_token().clone())?;
            mesh_provider(state, data, &input, &choice)?.0
        }
        ViewBuild::Preview {
            voxel_mm,
            tolerance_mm,
        } => {
            let options = shells::Options {
                voxel_mm,
                tolerance_mm,
                stamp: false,
                stamp_code: String::new(),
                date: schedule::Date::today(),
                stamp_dates: BTreeMap::new(),
                cancel: ctx.cancel_token().clone(),
                checked: false,
            };
            match shells::ShellProvider::new(state, data, &input, options)? {
                Ok(p) => Box::new(p),
                Err(why) => return Err(CommandError::Invalid(why)),
            }
        }
    };
    let done = std::sync::atomic::AtomicUsize::new(0);
    let provider = provider.as_ref();
    let results: Vec<Result<Option<ViewAligner>, export::ExportError>> = std::thread::scope(|s| {
        let jobs: Vec<_> = wanted
            .iter()
            .map(|&(arch, stage)| {
                let done = &done;
                s.spawn(move || {
                    let t = std::time::Instant::now();
                    let mesh = provider.aligner(arch, stage)?;
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    ctx.set_fraction(n as f32 / wanted.len() as f32);
                    Ok(mesh.map(|mesh| ViewAligner {
                        arch,
                        stage,
                        mesh,
                        seconds: t.elapsed().as_secs_f64(),
                    }))
                })
            })
            .collect();
        jobs.into_iter()
            .map(|j| {
                j.join()
                    .unwrap_or_else(|_| Err(export::ExportError::Package("panicked".into())))
            })
            .collect()
    });
    let mut out = Vec::new();
    for r in results {
        if let Some(a) = r.map_err(export_error)? {
            out.push(a);
        }
    }
    Ok((out, provider.notes()))
}

// ---- print check -------------------------------------------------------------------------------------------------

/// Parameters of `aligners.output.print-check`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PrintCheckParams {
    /// Stages to check (0 is the initial situation); default: the first and
    /// the last.
    pub stages: Vec<u32>,
    /// Count the self-intersections (the slow part).
    pub self_intersections: bool,
    /// Build volume of the printer `[x, y, z]` (mm); `null` skips the fit.
    pub build_plate_mm: Option<[f64; 3]>,
    /// Files from the shell stage (models and aligners): `null` when the
    /// case has the closed teeth and the gingiva, `false` the scans.
    pub shells: Option<bool>,
    /// Quality of the shell-stage files; default the quality stored in the
    /// case.
    pub quality: Option<Quality>,
    /// Detail of the models and shells (mm), instead of the quality's.
    pub voxel_mm: Option<f64>,
    /// Thinnest wall allowed in an aligner (mm); `null` skips the thickness.
    pub min_thickness_mm: Option<f64>,
}

impl Default for PrintCheckParams {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            self_intersections: true,
            build_plate_mm: Some(printcheck::DEFAULT_PLATE_MM),
            shells: None,
            quality: None,
            voxel_mm: None,
            min_thickness_mm: Some(0.3),
        }
    }
}

fn run_print_check(
    state: &CaseState,
    params: PrintCheckParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let arches = prepare(state, &data)?;
    let input = input(&arches, &data);
    let choice = MeshChoice {
        shells: params.shells,
        quality: params.quality.unwrap_or(data.settings.quality),
        voxel_mm: params.voxel_mm,
        stamp: false,
        stamp_code: String::new(),
        date: schedule::Date::today(),
        stamp_dates: BTreeMap::new(),
        cancel: ctx.cancel_token().clone(),
    };
    let (provider, meshes, notes) = mesh_provider(state, &data, &input, &choice)?;
    let options = CheckOptions {
        self_intersections: params.self_intersections,
        build_plate_mm: params.build_plate_mm,
        ..CheckOptions::default()
    };
    let mut results = Vec::new();
    for a in &arches {
        let n = schedule::active_stages(a.arch, &data.stages);
        let stages = if params.stages.is_empty() {
            vec![0, n]
        } else {
            params.stages.clone()
        };
        for k in stages.into_iter().filter(|&k| k <= n) {
            for aligner in [false, true] {
                ctx.check_cancelled()?;
                let mesh = if aligner {
                    if k == 0 {
                        continue;
                    }
                    provider.aligner(a.arch, k)
                } else {
                    provider.model(a.arch, k)
                };
                let Some(mesh) = mesh.map_err(export_error)? else {
                    continue;
                };
                let mut check = printcheck::check(
                    &mesh,
                    &CheckOptions {
                        min_thickness_mm: if aligner {
                            params.min_thickness_mm
                        } else {
                            None
                        },
                        ..options.clone()
                    },
                );
                // no fin, no flap far from the teeth (`shell_reach`)
                if aligner && let Some(reach) = provider.reach(a.arch, k) {
                    printcheck::add_reach(&mut check, reach);
                }
                results.push(serde_json::json!({
                    "arch": if a.arch.is_upper() { "upper" } else { "lower" },
                    "stage": k,
                    "kind": if aligner { "aligner" } else { "model" },
                    "ok": check.ok,
                    "problems": check.problem_texts(dca_algo::schedule::Lang::En),
                    "check": check,
                }));
            }
        }
    }
    let passed = results.iter().filter(|r| r["ok"] == true).count();
    let notes: Vec<String> = notes
        .iter()
        .chain(&provider.notes())
        .map(|n| dca_algo::export::text::note(n, dca_algo::schedule::Lang::En))
        .collect();
    let out = serde_json::json!({
        "checked": results.len(),
        "passed": passed,
        "meshes": meshes,
        "notes": notes,
        "files": results,
    });
    Ok((state.clone(), out))
}

// ---- manifest ----------------------------------------------------------------------------------------------------

/// Parameters of `aligners.output.manifest`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ManifestParams {
    /// Export folder.
    pub dir: String,
    /// Verify the folder against its `manifest.json` (default). When false,
    /// write a manifest of the files found, replacing the existing one.
    pub verify: bool,
}

impl Default for ManifestParams {
    fn default() -> Self {
        Self {
            dir: String::new(),
            verify: true,
        }
    }
}

fn run_manifest(
    state: &CaseState,
    params: ManifestParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if params.dir.trim().is_empty() {
        return Err(CommandError::Invalid(
            "dir: the export folder is required".to_owned(),
        ));
    }
    let dir = Path::new(&params.dir);
    let out = if params.verify {
        let v = manifest::verify(dir).map_err(|e| CommandError::Invalid(e.to_string()))?;
        serde_json::json!({ "ok": v.ok(), "verification": v })
    } else {
        let files = manifest::scan(dir).map_err(|e| CommandError::Invalid(e.to_string()))?;
        let m = manifest::Manifest {
            format_version: manifest::FORMAT_VERSION,
            generator: manifest::Generator {
                name: "dentalcraft-aligners".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            patient_code: state.case.patient_code.clone(),
            generated: schedule::Date::today().to_iso(),
            settings: serde_json::json!({}),
            aligners: BTreeMap::new(),
            total_aligners: 0,
            files,
            print_check: manifest::PrintSummary::default(),
        };
        let bytes =
            serde_json::to_vec_pretty(&m).map_err(|e| CommandError::Failed(e.to_string()))?;
        let path = dir.join(manifest::MANIFEST_NAME);
        std::fs::write(&path, bytes)
            .map_err(|e| CommandError::Failed(format!("{}: {e}", path.display())))?;
        serde_json::json!({ "ok": true, "written": path.display().to_string(), "files": m.files.len() })
    };
    Ok((state.clone(), out))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_command_is_registered_for_the_cli_and_not_in_the_menu() {
        let spec = crate::commands::find("aligners.output.export-stl").unwrap();
        assert!(spec.schema()["properties"]["dir"].is_object());
        assert!(crate::tree::find("aligners.output.export-stl").is_none());
    }

    #[test]
    fn it_needs_aligners_and_a_directory() {
        let err = exec(
            "aligners.output.export-stl",
            &CaseState::default(),
            json!({"dir": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("no aligners")));
        let s = super::super::staging::tests::staged_input("lower", &CaseState::default());
        let (s, _) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let err = exec("aligners.output.export-stl", &s, Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("dir")));
    }

    #[test]
    fn it_exports_the_stl_the_csv_and_the_calendar_of_both_arches() {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let s = staged_input("upper", &s);
        let (s, staged) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let nl = staged["lower"]["aligners"].as_u64().unwrap();
        let nu = staged["upper"]["aligners"].as_u64().unwrap();
        let dir = std::env::temp_dir().join(format!("dca-export-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (same, out) = exec(
            "aligners.output.export-stl",
            &s,
            json!({ "dir": dir.display().to_string(), "start_date": "2026-01-05", "interval_days": 7 }),
        )
        .unwrap();
        assert_eq!(same.case.scans, s.case.scans);
        assert_eq!(out["aligners"]["lower"], nl);
        assert_eq!(out["aligners"]["upper"], nu);
        assert_eq!(out["total_aligners"], nl.max(nu));
        assert_eq!(out["total_days"].as_u64().unwrap(), 7 * nl.max(nu));
        let files = out["files"].as_array().unwrap();
        assert_eq!(
            files.iter().filter(|f| f["kind"] == "stl").count() as u64,
            nl + nu + 2
        );

        // every STL loads back with dc-mesh; the initial model is the scan
        let scan = s
            .mesh(&s.case.scan(dc_case::ScanRole::Lower).unwrap().mesh)
            .unwrap()
            .clone();
        let w = (nl.max(nu)).to_string().len().max(2);
        let first = dc_mesh::io::read(dir.join(format!("lower_stage_{:0w$}.stl", 0))).unwrap();
        assert_eq!(first.face_count(), scan.face_count());
        let last = dc_mesh::io::read(dir.join(format!("lower_stage_{nl:0w$}.stl"))).unwrap();
        assert_eq!(last.face_count(), scan.face_count());
        // the final model differs from the initial one: the teeth moved
        // (the vertex order of an STL is the order of its triangles, so compare sorted sets)
        let key = |m: &dc_mesh::TriMesh| {
            let mut v: Vec<[u32; 3]> = m
                .positions
                .iter()
                .map(|p| p.to_array().map(f32::to_bits))
                .collect();
            v.sort_unstable();
            v
        };
        let moved = |a: &dc_mesh::TriMesh, b: &dc_mesh::TriMesh| key(a) != key(b);
        assert!(moved(&first, &last));
        for f in files.iter().filter(|f| f["kind"] == "stl") {
            let m = dc_mesh::io::read(dir.join(f["name"].as_str().unwrap())).unwrap();
            assert!(m.check().is_ok());
        }

        // the CSV is parseable, the ICS has an event per aligner of both arches
        let csv = std::fs::read_to_string(dir.join("stages.csv")).unwrap();
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        assert_eq!(header[0], "arch");
        for line in csv.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), header.len(), "{line}");
            for c in &cols[4..10] {
                c.parse::<f64>().unwrap();
            }
        }
        assert_eq!(
            csv.lines().count() as u64,
            1 + 14 * (nl + 1) + 14 * (nu + 1)
        );
        let ics = std::fs::read_to_string(dir.join("visits.ics")).unwrap();
        assert_eq!(
            dca_algo::schedule::validate_ics(&ics).unwrap() as u64,
            nl.max(nu) * 2 + 2
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn staged_both() -> CaseState {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let s = staged_input("upper", &s);
        exec("aligners.planning.staging.automatic", &s, Value::Null)
            .unwrap()
            .0
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dca-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_doubts_of_the_check_are_a_warning_of_the_export_and_of_the_report() {
        let mut s = staged_both();
        let (_, out) = exec(
            "aligners.output.export-stl",
            &s,
            json!({ "dir": temp("no-doubts").display().to_string() }),
        )
        .unwrap();
        assert_eq!(out["warnings"], json!([]));
        assert_eq!(out["segmentation_doubts"], json!({}));
        let mut d = aligners_data(&s).unwrap();
        d.segmentation_doubts
            .insert(ScanRole::Lower, vec!["46 is only 2.1 mm wide".to_owned()]);
        s.set_module(dca_model::MODULE_ID, dc_case::ModuleState::new(d));

        let dir = temp("doubts");
        let (_, out) = exec(
            "aligners.output.export-stl",
            &s,
            json!({ "dir": dir.display().to_string() }),
        )
        .unwrap();
        assert_eq!(
            out["segmentation_doubts"],
            json!({ "lower": ["46 is only 2.1 mm wide"] })
        );
        let w = out["warnings"][0].as_str().unwrap();
        assert!(
            w.contains("lower arch") && w.contains("46 is only 2.1 mm wide"),
            "{w}"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // the report says it first among its notes, in its language
        let (_, out) = exec(
            "aligners.output.report",
            &s,
            json!({ "dir": dir.display().to_string(), "lang": "en" }),
        )
        .unwrap();
        assert_eq!(out["warnings"].as_array().unwrap().len(), 1);
        let note = out["notes"][0].as_str().unwrap();
        assert!(
            note.starts_with("to check before delivery") && note.contains("46 is only 2.1 mm wide"),
            "{note}"
        );
        let report = std::fs::read_to_string(dir.join("report.html")).unwrap();
        assert!(report.contains("to check before delivery"));
        std::fs::remove_dir_all(&dir).unwrap();
        let _ = std::fs::remove_dir_all(temp("no-doubts"));
    }

    #[test]
    fn the_export_commands_are_registered() {
        for id in [
            "aligners.output.report",
            "aligners.output.print-check",
            "aligners.output.manifest",
        ] {
            assert!(crate::commands::find(id).is_some(), "{id}");
        }
        let schema = crate::commands::find("aligners.output.report")
            .unwrap()
            .schema();
        assert!(schema["properties"]["workflow"].is_object());
    }

    #[test]
    fn the_report_command_writes_the_whole_package_and_the_manifest_verifies() {
        let s = staged_both();
        let dir = temp("report");
        let started = std::time::Instant::now();
        let (same, out) = exec(
            "aligners.output.report",
            &s,
            json!({
                "dir": dir.display().to_string(),
                "start_date": "2026-01-05",
                "interval_days": 7,
                "lang": "it",
                "generated": "2026-01-01",
            }),
        )
        .unwrap();
        eprintln!("full export of the demo case: {:?}", started.elapsed());
        assert_eq!(same.case.scans, s.case.scans);
        let files = out["files"].as_array().unwrap();
        let has = |n: &str| files.iter().any(|f| f["name"] == n);
        for n in [
            "stages.csv",
            "visits.ics",
            "report.html",
            "print_check.json",
            "SUP_programma_paziente.html",
            "INF_programma_paziente.html",
            "SUP_manifest_stampa.csv",
            "INF_manifest_stampa.csv",
            "modelli_termoformatura/SUP_modello_00_iniziale.stl",
            "modelli_termoformatura/INF_modello_00_iniziale.stl",
        ] {
            assert!(has(n), "missing {n}");
        }
        let nl = out["aligners"]["lower"].as_u64().unwrap();
        let nu = out["aligners"]["upper"].as_u64().unwrap();
        assert_eq!(
            files.iter().filter(|f| f["kind"] == "stl").count() as u64,
            nl + nu + 2
        );

        // every hash verifies, through the command too
        let (_, v) = exec(
            "aligners.output.manifest",
            &s,
            json!({ "dir": dir.display().to_string() }),
        )
        .unwrap();
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(
            v["verification"]["verified"].as_u64().unwrap() as usize,
            files.len()
        );

        // the report is in Italian, well formed, with the two renders
        let report = std::fs::read_to_string(dir.join("report.html")).unwrap();
        dca_algo::export::html::check_wellformed(&report).unwrap();
        assert!(report.contains("Diagnosi dal modello"));
        assert_eq!(report.matches("data:image/png;base64,").count(), 2);
        let patient = std::fs::read_to_string(dir.join("SUP_programma_paziente.html")).unwrap();
        dca_algo::export::html::check_wellformed(&patient).unwrap();
        assert!(patient.contains("05/01/2026"));

        // the scans are open surfaces: the print check says so
        assert!(!out["print_check"]["failed"].as_array().unwrap().is_empty());
        // no closed teeth and no gingiva: the scans, and the report says why
        assert_eq!(out["meshes"], "scans");
        assert!(
            out["notes"][0]
                .as_str()
                .unwrap()
                .contains("i modelli sono le scansioni deformate"),
            "{}",
            out["notes"]
        );
        assert!(report.contains("Note sui file di stampa"));

        // a changed file is found by the manifest command
        std::fs::write(dir.join("stages.csv"), b"x").unwrap();
        let (_, v) = exec(
            "aligners.output.manifest",
            &s,
            json!({ "dir": dir.display().to_string() }),
        )
        .unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["verification"]["mismatched"][0], "stages.csv");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_shells_are_refused_without_the_closed_teeth() {
        let s = staged_both();
        let dir = temp("no-shells");
        let err = exec(
            "aligners.output.report",
            &s,
            json!({ "dir": dir.display().to_string(), "shells": true }),
        )
        .unwrap_err();
        assert!(
            matches!(err, CommandError::Invalid(ref m) if m.contains("3D teeth")),
            "{err:?}"
        );
        let err = exec(
            "aligners.output.report",
            &s,
            json!({ "dir": dir.display().to_string(), "voxel_mm": 2.0 }),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(ref m) if m.contains("voxel_mm")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The full export of a golden arch from the shell stage at the draft
    /// quality: a printable model and a trimmed aligner per stage, every one
    /// closed, in one piece and passing the print check.
    #[test]
    #[ignore = "reconstruction, gingiva and the shells of every stage: run with --release --ignored"]
    fn the_shells_of_a_golden_arch_pass_the_print_check() {
        golden_arch_export("demo_lower_mild", "lower");
    }

    /// The upper arch has the narrowest faces: with the embrasure fills the
    /// models lost their stamp when the label was wider than the plugin's.
    #[test]
    #[ignore = "reconstruction, gingiva and the shells of every stage: run with --release --ignored"]
    fn the_models_of_an_upper_golden_arch_keep_their_stamps() {
        golden_arch_export("demo_upper_mild", "upper");
    }

    fn golden_arch_export(case: &str, arch: &str) {
        let mut s = super::super::reconstruct::golden_state(case);
        for id in [
            "aligners.teeth.frames",
            "aligners.preparation.reconstruction.automatic",
            "aligners.preparation.gingiva",
            "aligners.planning.setup.automatic",
            "aligners.planning.staging.automatic",
        ] {
            let t = std::time::Instant::now();
            s = exec(id, &s, Value::Null).unwrap().0;
            eprintln!("{id}: {:.1} s", t.elapsed().as_secs_f64());
        }
        let dir = temp(&format!("shells-{case}"));
        let quality = std::env::var("DCA_QUALITY").unwrap_or_else(|_| "draft".to_owned());
        let t = std::time::Instant::now();
        let (_, out) = exec(
            "aligners.output.report",
            &s,
            json!({
                "dir": dir.display().to_string(),
                "quality": quality,
                "self_intersections": true,
                "embed_renders": false,
                "generated": "2026-01-01",
                "stamp_code": "GOLD-1",
            }),
        )
        .unwrap();
        eprintln!("export ({quality}): {:.1} s", t.elapsed().as_secs_f64());
        eprintln!("notes: {:#}", out["notes"]);
        assert_eq!(out["meshes"], "shells");
        let dropped: Vec<&Value> = out["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n.as_str().unwrap().contains("no stamp"))
            .collect();
        assert!(dropped.is_empty(), "{dropped:#?}");
        let n = out["aligners"][arch].as_u64().unwrap();
        let files = out["files"].as_array().unwrap();
        let stl = |p: &str| {
            files
                .iter()
                .filter(|f| f["name"].as_str().unwrap().starts_with(p))
                .count() as u64
        };
        assert_eq!(stl("modelli_termoformatura/"), n + 1);
        assert_eq!(stl("stampa_resina/"), n);
        let check: Value =
            serde_json::from_slice(&std::fs::read(dir.join("print_check.json")).unwrap()).unwrap();
        for f in check["files"].as_array().unwrap() {
            eprintln!(
                "{:<48} ok {:<5} {}",
                f["name"].as_str().unwrap(),
                f["ok"],
                f["problems_text"]
            );
        }
        assert_eq!(
            out["print_check"]["failed"],
            json!([]),
            "{:#}",
            out["print_check"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The golden scans of both arches as the app imports them (STL files in
    /// a new case), through the automatic pipeline up to the bite.
    pub(crate) fn golden_pipeline() -> CaseState {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/fixtures");
        let dir = temp("pipeline");
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = CaseState::default();
        for (fixture, name, role, id) in [
            ("demo_lower_mild", "lower.stl", "lower", 1),
            ("demo_upper_mild", "upper.stl", "upper", 2),
        ] {
            let bytes = std::fs::read(root.join(fixture).join("scan.dcm")).unwrap();
            let mesh = dc_mesh::dcm::decode(&bytes).unwrap();
            let path = dir.join(name);
            dc_mesh::io::write(&path, &mesh, dc_mesh::io::MeshFormat::Stl).unwrap();
            let mut mesh = dc_mesh::io::read(&path).unwrap();
            dc_mesh::io::normalize_units(&mut mesh).unwrap();
            let blob = s.blobs.insert(mesh);
            let scan: dc_case::Scan = serde_json::from_value(json!({
                "id": format!("de300000-0000-4000-8000-{id:012x}"),
                "role": role,
                "source_name": name,
                "source_units": "mm",
                "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
                "mesh": blob,
            }))
            .unwrap();
            s.case.scans.push(scan);
        }
        std::fs::remove_dir_all(&dir).unwrap();
        let t = std::time::Instant::now();
        let s = exec("aligners.auto", &s, json!({ "start_date": "2026-11-02" }))
            .unwrap()
            .0;
        eprintln!("pipeline: {:.1} s", t.elapsed().as_secs_f64());
        s
    }

    /// One aligner of each arch through the provider of the full export, at
    /// the Standard quality (`DCA_QUALITY` for another one): the upper one
    /// (`DCA_STAGE`, 1 by default) and the lower 01. The hard cut of the
    /// occlusal shaping left 3 handles in the first and a crossing in the
    /// second, on the outer surface of the wall.
    #[test]
    #[ignore = "the golden pipeline and two aligners at 0.1 mm: run with --release --ignored"]
    fn standard_aligners_of_the_golden_arches_print() {
        let s = golden_pipeline();
        let data = aligners_data(&s).unwrap();
        // every closed tooth of the pipeline knows its scanned part: the
        // models draw their stumps under the gingiva
        assert_eq!(data.crown_scanned.len(), data.crowns.len());
        for arch in [Arch::Upper, Arch::Lower] {
            let print = crate::commands::shell::ArchPrint::new(&s, &data, arch)
                .unwrap()
                .unwrap();
            let g = print.stage(&data.stages, 1);
            assert!(g.stumps.iter().any(|t| !t.below.is_empty()), "{arch:?}");
        }
        let arches = prepare(&s, &data).unwrap();
        let input = input(&arches, &data);
        let quality = std::env::var("DCA_QUALITY").unwrap_or_else(|_| "standard".to_owned());
        let quality: Quality = serde_json::from_value(json!(quality)).unwrap();
        let choice = MeshChoice {
            shells: Some(true),
            quality,
            voxel_mm: None,
            stamp: true,
            stamp_code: String::new(),
            date: schedule::Date::parse("2026-11-01").unwrap(),
            stamp_dates: BTreeMap::new(),
            cancel: dc_command::CancelToken::new(),
        };
        let (provider, _, _) = mesh_provider(&s, &data, &input, &choice).unwrap();
        let upper: u32 = std::env::var("DCA_STAGE").map_or(1, |k| k.parse().unwrap());
        let options = CheckOptions {
            min_thickness_mm: Some(0.3),
            ..CheckOptions::default()
        };
        let mut failed = Vec::new();
        for (arch, k) in [(Arch::Upper, upper), (Arch::Lower, 1)] {
            let t = std::time::Instant::now();
            let mesh = provider.aligner(arch, k).unwrap().unwrap();
            let c = printcheck::check(&mesh, &options);
            eprintln!(
                "{arch:?} {k}: {:.1} s, {} faces, genus {:?}, {:?} crossings, wall {:?}, {:?}",
                t.elapsed().as_secs_f64(),
                c.triangles,
                c.genus,
                c.self_intersections,
                c.min_thickness_mm,
                c.problem_texts(schedule::Lang::En)
            );
            if !c.ok {
                failed.push((arch, k));
            }
        }
        for n in provider.notes() {
            eprintln!("note: {n:?}");
        }
        assert!(failed.is_empty(), "not printable: {failed:?}");
    }

    #[test]
    fn the_report_needs_aligners_and_a_directory() {
        let err = exec(
            "aligners.output.report",
            &CaseState::default(),
            json!({"dir": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("no aligners")));
        let s = staged_both();
        let err = exec("aligners.output.report", &s, Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("dir")));
    }

    #[test]
    fn the_print_check_command_flags_the_open_scans() {
        let s = staged_both();
        let (_, out) = exec(
            "aligners.output.print-check",
            &s,
            json!({ "self_intersections": false }),
        )
        .unwrap();
        assert_eq!(out["checked"], 4);
        // a scan is a set of crowns and gingiva pieces, not one printable solid
        assert_eq!(out["passed"], 0);
        let first = &out["files"][0];
        assert!(first["check"]["pieces"].as_u64().unwrap() > 1);
        assert!(
            first["problems"][0]
                .as_str()
                .unwrap()
                .contains("separate pieces")
        );
    }

    #[test]
    fn the_gingiva_follows_the_teeth_in_the_models_when_it_is_stored() {
        let s = super::super::reconstruct::golden_state("demo_lower_mild");
        let mut s = s;
        for id in [
            "aligners.teeth.frames",
            "aligners.planning.setup.automatic",
            "aligners.planning.staging.automatic",
        ] {
            s = exec(id, &s, Value::Null).unwrap().0;
        }
        let (with_gum, _) = exec("aligners.preparation.gingiva", &s, Value::Null).unwrap();
        let models = |state: &CaseState| {
            let data = aligners_data(state).unwrap();
            let arches = prepare(state, &data).unwrap();
            let input = input(&arches, &data);
            let provider = ScanProvider::new(&input);
            let n = schedule::active_stages(Arch::Lower, &data.stages);
            assert!(n > 0);
            let first = provider.model(Arch::Lower, 0).unwrap().unwrap();
            let last = provider.model(Arch::Lower, n).unwrap().unwrap();
            (arches[0].scan.labels.clone(), first, last)
        };
        let moved_gum = |labels: &[u16], a: &dc_mesh::TriMesh, b: &dc_mesh::TriMesh| {
            labels
                .iter()
                .zip(a.positions.iter().zip(&b.positions))
                .filter(|(l, (p, q))| **l == 0 && (**p - **q).length() > 1e-4)
                .count()
        };
        // without a stored gingiva the gum stays where it is
        let (labels, first, last) = models(&s);
        let scan = s.mesh(&s.case.scan(ScanRole::Lower).unwrap().mesh).unwrap();
        assert_eq!(first.positions, scan.positions);
        assert_eq!(moved_gum(&labels, &first, &last), 0);
        // with it, the gum near the moved teeth follows them, the base stays
        let (labels, first, last) = models(&with_gum);
        let scan = with_gum
            .mesh(&with_gum.case.scan(ScanRole::Lower).unwrap().mesh)
            .unwrap();
        assert_eq!(first.positions, scan.positions, "stage 0 is the scan");
        let moved = moved_gum(&labels, &first, &last);
        let gum = labels.iter().filter(|&&l| l == 0).count();
        assert!(moved > 100 && moved < gum, "{moved} of {gum}");
        let max = labels
            .iter()
            .zip(first.positions.iter().zip(&last.positions))
            .filter(|(l, _)| **l == 0)
            .map(|(_, (p, q))| (*p - *q).length())
            .fold(0.0f32, f32::max);
        assert!(max < 5.0, "{max}");
    }

    #[test]
    fn the_manifest_command_writes_a_manifest_for_a_folder_without_one() {
        let dir = temp("manifest");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.csv"), b"1,2\n").unwrap();
        let params = json!({ "dir": dir.display().to_string() });
        let err = exec(
            "aligners.output.manifest",
            &CaseState::default(),
            params.clone(),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)));
        let (_, out) = exec(
            "aligners.output.manifest",
            &CaseState::default(),
            json!({ "dir": dir.display().to_string(), "verify": false }),
        )
        .unwrap();
        assert_eq!(out["files"], 1);
        let (_, v) = exec("aligners.output.manifest", &CaseState::default(), params).unwrap();
        assert_eq!(v["ok"], true);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
