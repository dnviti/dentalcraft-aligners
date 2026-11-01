//! The full export: models and aligners of every stage in parallel, their
//! print check, the tables, the calendars, the patient programmes, the
//! report and the manifest (`case_export.py`, `export_parallel.py`).
//!
//! Layout of the export folder (the plugin's names, `SUP`/`INF` for the
//! upper and lower arch, `NN` the number of the aligner):
//!
//! ```text
//! modelli_termoformatura/SUP_modello_00_iniziale.stl   initial model
//! modelli_termoformatura/SUP_modello_NN.stl            model of aligner NN
//! stampa_resina/SUP_allineatore_NN.stl                 shell of aligner NN
//! stampa_resina/SUP_allineatore_NN_passivo.stl         passive aligner
//! SUP_manifest_stampa.csv                              files, volume, dates
//! SUP_programma_paziente.html                          programme of the patient
//! stages.csv, visits.ics                               movements, calendar
//! report.html, print_check.json, manifest.json
//! ```
//!
//! The work that the plugin spread over background Blender processes
//! (`export_parallel.py`) is a parallel loop here: one task per stage, arch
//! and kind of file, taken in turn by a few workers (`parallel`, or what
//! the provider says one build can afford: a shell at 0.1 mm is a few
//! gigabytes of distance fields), each task using the `rayon` pool inside.
//! Every task is a pure function of its inputs and writes its own file, and
//! the lists are sorted by name before they are used, so the files and their
//! hashes do not depend on the number of threads (`threads` in the
//! parameters builds a pool of that size) or of workers.
//!
//! The meshes come from a [`MeshProvider`](super::provider::MeshProvider);
//! what it could not do as asked (a stamp without room, a ramp left out)
//! comes back as notes, listed in the report.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dca_model::AlignersState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::manifest::{self, Entry, Generator, MANIFEST_NAME, Manifest, PrintSummary};
use super::patient::{self, PatientOptions};
use super::printcheck::{self, CheckOptions};
use super::provider::{MeshProvider, Note, Quality};
use super::report::{self, FileCheck, Renders, ReportInput};
use super::{
    ExportError, Input, Params, Workflow, arch_label, arch_name, check_input, plan_calendars,
    stl_bytes, write_atomic,
};
use crate::anatomy::Arch;
use crate::schedule::{self, Calendar, Date, IcsOptions, Lang, Policy};

/// Parameters of the full export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FullParams {
    /// Target directory (created if it does not exist).
    pub dir: String,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    pub start_date: Option<String>,
    /// Days of wear of each aligner (default: the one of the case, 14).
    pub interval_days: Option<u32>,
    /// Interval policy of the calendar.
    pub policy: Policy,
    /// Language of the report, the patient programmes and the calendar.
    pub lang: Lang,
    /// Name of the case in the calendar events and the report.
    pub title: String,
    /// Which print files to make: models for thermoforming, aligners to
    /// print directly, or both.
    pub workflow: Workflow,
    /// Run the print check on every file.
    pub print_check: bool,
    /// Count the self-intersections in the print check (the slow part).
    pub self_intersections: bool,
    /// Thinnest wall allowed in an aligner (mm); `null` skips the thickness.
    pub min_thickness_mm: Option<f64>,
    /// Build volume of the printer `[x, y, z]` (mm); `null` skips the fit.
    pub build_plate_mm: Option<[f64; 3]>,
    /// Write the report.
    pub report: bool,
    /// Embed the renders in the report (the caller supplies them).
    pub embed_renders: bool,
    /// Threads of the export (default: all the cores).
    pub threads: Option<usize>,
    /// Date written in the files, `YYYY-MM-DD` (default today). Fixing it
    /// makes two exports of the same case byte for byte equal.
    pub generated: Option<String>,
    /// Print files from the shell stage (the printable model and the
    /// trimmed aligner): `null` when the case has the closed teeth and the
    /// gingiva, else the deformed scans as the models and no aligner;
    /// `true` requires them, `false` uses the scans.
    pub shells: Option<bool>,
    /// Quality of the shell-stage files (detail and surface tolerance).
    pub quality: Quality,
    /// Detail of the models and shells (mm), instead of the quality's.
    pub voxel_mm: Option<f64>,
    /// Identification stamp: embossed on the models, engraved in the
    /// aligners (arch, number, date of the export, `stamp_code`).
    pub stamp: bool,
    /// Case code in the stamp (letters, digits, `-`; at most 8; never the
    /// name of the patient).
    pub stamp_code: String,
    /// Files built at the same time (default: what one build can afford, at
    /// most the threads).
    pub parallel: Option<usize>,
}

impl Default for FullParams {
    fn default() -> Self {
        let base = Params::default();
        Self {
            dir: String::new(),
            start_date: None,
            interval_days: None,
            policy: base.policy,
            lang: base.lang,
            title: base.title,
            workflow: Workflow::Both,
            print_check: true,
            self_intersections: true,
            min_thickness_mm: Some(0.3),
            build_plate_mm: Some(printcheck::DEFAULT_PLATE_MM),
            report: true,
            embed_renders: true,
            threads: None,
            generated: None,
            shells: None,
            quality: Quality::default(),
            voxel_mm: None,
            stamp: true,
            stamp_code: String::new(),
            parallel: None,
        }
    }
}

impl FullParams {
    /// The parameters of the first slice of the export.
    pub fn base(&self) -> Params {
        Params {
            dir: self.dir.clone(),
            start_date: self.start_date.clone(),
            interval_days: self.interval_days,
            policy: self.policy,
            lang: self.lang,
            title: self.title.clone(),
        }
    }
}

/// What the full export is made from, besides the series.
#[derive(Debug, Clone)]
pub struct CaseInfo<'a> {
    /// Pseudonymised patient code.
    pub patient_code: &'a str,
    /// The Aligners data of the case (diagnosis, set-up, settings).
    pub state: &'a AlignersState,
    /// Teeth with an attachment, by arch.
    pub attachments: BTreeMap<Arch, Vec<String>>,
    /// Images for the report (used when `embed_renders` is set).
    pub renders: Renders,
    /// Where the meshes come from (`shells`, `scans`), for the manifest.
    pub meshes: &'a str,
    /// Notes of the caller about the whole export (the source of the meshes
    /// chosen and why), before the notes of the provider.
    pub notes: Vec<Note>,
}

/// What the full export did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FullReport {
    /// The target directory.
    pub dir: String,
    /// Every file written, by name; the manifest is not in the list.
    pub files: Vec<Entry>,
    /// Active aligners of each arch.
    pub aligners: BTreeMap<String, u32>,
    /// Aligners of the longest series.
    pub total_aligners: u32,
    /// Delivery of the first aligner.
    pub start: String,
    /// The day after the last aligner.
    pub end: String,
    /// Days of the series.
    pub total_days: u32,
    /// The print check (`None` when it was not run).
    pub print_check: Option<PrintSummary>,
    /// Where the meshes came from (`shells`, `scans`).
    pub meshes: String,
    /// What could not be done as asked, in the language of the export.
    pub notes: Vec<String>,
    /// Milliseconds the export took.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Model,
    Aligner,
}

struct Made {
    entry: Entry,
    check: Option<FileCheck>,
    /// The bytes of the last aligner of an arch, kept for the passive ones.
    keep: Option<Vec<u8>>,
}

/// Prefix of the files of an arch.
pub fn prefix(arch: Arch) -> &'static str {
    if arch.is_upper() { "SUP" } else { "INF" }
}

/// Name of the model of aligner `k` (`0`: the initial model), with its folder.
pub fn model_name(arch: Arch, k: u32) -> String {
    format!(
        "modelli_termoformatura/{}_modello_{k:02}{}.stl",
        prefix(arch),
        if k == 0 { "_iniziale" } else { "" }
    )
}

/// Name of the shell of aligner `k`, with its folder.
pub fn aligner_name(arch: Arch, k: u32, passive: bool) -> String {
    format!(
        "stampa_resina/{}_allineatore_{k:02}{}.stl",
        prefix(arch),
        if passive { "_passivo" } else { "" }
    )
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// Writes a file of the export and records it.
fn put(
    dir: &Path,
    entries: &mut Vec<Entry>,
    name: &str,
    kind: &str,
    bytes: Vec<u8>,
) -> Result<(), ExportError> {
    write_in(dir, name, &bytes)?;
    entries.push(Entry::of(name, kind, &bytes));
    Ok(())
}

fn io_err(path: &Path, e: impl std::fmt::Display) -> ExportError {
    ExportError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

fn write_in(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), ExportError> {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    write_atomic(&path, bytes)
}

/// Runs the full export of `input` with the meshes of `provider`.
pub fn run(
    input: &Input<'_>,
    case: &CaseInfo<'_>,
    provider: &dyn MeshProvider,
    params: &FullParams,
) -> Result<FullReport, ExportError> {
    match params.threads {
        Some(n) if n > 0 => {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .map_err(|e| ExportError::Package(e.to_string()))?;
            // not inside the pool: the workers of the meshes wait for it
            run_inner(input, case, provider, params, Some(&pool))
        }
        _ => run_inner(input, case, provider, params, None),
    }
}

/// `f` of every item, `workers` at a time on threads of their own (each
/// one using the `rayon` pool `pool`, or the global one, inside), the
/// results in the order of the items. The first error stops the workers
/// at their next item.
fn bounded_map<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    pool: Option<&rayon::ThreadPool>,
    f: impl Fn(&T) -> Result<R, ExportError> + Sync,
) -> Result<Vec<R>, ExportError> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let slots: Mutex<Vec<Option<Result<R, ExportError>>>> =
        Mutex::new((0..items.len()).map(|_| None).collect());
    let work = || {
        loop {
            if failed.load(Ordering::Relaxed) {
                break;
            }
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { break };
            let r = match pool {
                Some(p) => p.install(|| f(item)),
                None => f(item),
            };
            if r.is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            slots.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(r);
        }
    };
    let workers = workers.clamp(1, items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(work);
        }
    });
    let slots = slots.into_inner().unwrap_or_else(|e| e.into_inner());
    let mut out = Vec::with_capacity(items.len());
    for r in slots {
        match r {
            Some(Ok(v)) => out.push(v),
            Some(Err(e)) => return Err(e),
            // not started: an error stopped the workers (found above)
            None => {}
        }
    }
    if out.len() < items.len() {
        return Err(ExportError::Package("the export stopped".to_owned()));
    }
    Ok(out)
}

fn run_inner(
    input: &Input<'_>,
    case: &CaseInfo<'_>,
    provider: &dyn MeshProvider,
    params: &FullParams,
    pool: Option<&rayon::ThreadPool>,
) -> Result<FullReport, ExportError> {
    let t0 = Instant::now();
    check_input(input, &params.dir)?;
    let base = params.base();
    let plan = plan_calendars(input, &base, &case.attachments)?;
    let generated = match &params.generated {
        Some(g) => {
            Date::parse(g).ok_or_else(|| ExportError::Package(format!("invalid date {g:?}")))?
        }
        None => Date::today(),
    };
    let lang = params.lang;
    let thermo = matches!(params.workflow, Workflow::Both | Workflow::Thermo);
    let direct = matches!(params.workflow, Workflow::Both | Workflow::Direct);
    let dir = PathBuf::from(&params.dir);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;

    // ---- the meshes: one task per arch, stage and kind ----
    let mut tasks: Vec<(Arch, u32, Kind)> = Vec::new();
    for (&arch, &n) in &plan.active {
        if thermo {
            tasks.extend((0..=n).map(|k| (arch, k, Kind::Model)));
        }
        if direct {
            tasks.extend((1..=n).map(|k| (arch, k, Kind::Aligner)));
        }
    }
    let make = |&(arch, k, kind): &(Arch, u32, Kind)| -> Result<Option<Made>, ExportError> {
        let mesh = match kind {
            Kind::Model => provider.model(arch, k)?,
            Kind::Aligner => provider.aligner(arch, k)?,
        };
        let Some(mesh) = mesh else {
            return Ok(None);
        };
        let bytes = stl_bytes(&mesh)?;
        let name = match kind {
            Kind::Model => model_name(arch, k),
            Kind::Aligner => aligner_name(arch, k, false),
        };
        write_in(&dir, &name, &bytes)?;
        let mut entry = Entry::of(&name, "stl", &bytes);
        entry.arch = Some(arch_name(arch).to_owned());
        entry.stage = Some(k);
        let check = params.print_check.then(|| {
            let options = CheckOptions {
                self_intersections: params.self_intersections,
                min_thickness_mm: if kind == Kind::Aligner {
                    params.min_thickness_mm
                } else {
                    None
                },
                build_plate_mm: params.build_plate_mm,
                ..CheckOptions::default()
            };
            FileCheck {
                name: name.clone(),
                arch,
                stage: k,
                kind: if kind == Kind::Model {
                    "model"
                } else {
                    "aligner"
                },
                check: printcheck::check(&mesh, &options),
            }
        });
        entry.printable = check.as_ref().map(|c| c.check.ok);
        let keep = (kind == Kind::Aligner && k == plan.active[&arch]).then_some(bytes);
        Ok(Some(Made { entry, check, keep }))
    };
    let threads = pool.map_or_else(rayon::current_num_threads, |p| p.current_num_threads());
    let workers = params
        .parallel
        .filter(|&n| n > 0)
        .or_else(|| provider.parallel_hint())
        .unwrap_or(threads)
        .min(threads);
    let mut made: Vec<Made> = bounded_map(&tasks, workers, pool, make)?
        .into_iter()
        .flatten()
        .collect();
    let notes: Vec<String> = case
        .notes
        .iter()
        .chain(&provider.notes())
        .map(|n| super::text::note(n, lang))
        .collect();

    // the passive aligners repeat the shell of the last stage of the arch
    if direct {
        let mut passives = Vec::new();
        for (&arch, &n) in &plan.active {
            let total = plan.calendars[&arch].n_total;
            let last = made.iter().find(|m| {
                m.entry.arch.as_deref() == Some(arch_name(arch))
                    && m.keep.is_some()
                    && m.entry.stage == Some(n)
            });
            if let Some(last) = last {
                for k in n + 1..=total {
                    let name = aligner_name(arch, k, true);
                    let bytes = last.keep.as_deref().unwrap_or_default();
                    write_in(&dir, &name, bytes)?;
                    let mut entry = Entry::of(&name, "stl", bytes);
                    entry.arch = Some(arch_name(arch).to_owned());
                    entry.stage = Some(k);
                    entry.printable = last.entry.printable;
                    passives.push(Made {
                        entry,
                        check: last.check.as_ref().map(|c| FileCheck {
                            name: name.clone(),
                            stage: k,
                            ..c.clone()
                        }),
                        keep: None,
                    });
                }
            }
        }
        made.extend(passives);
    }
    for m in &mut made {
        m.keep = None;
    }
    made.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
    let mut entries: Vec<Entry> = made.iter().map(|m| m.entry.clone()).collect();
    let checks: Vec<FileCheck> = made.iter().filter_map(|m| m.check.clone()).collect();

    // ---- tables, calendars, programmes ----
    put(
        &dir,
        &mut entries,
        "stages.csv",
        "csv",
        super::stages_csv(input, &plan.calendars).into_bytes(),
    )?;
    let stamp = format!("{}T000000Z", generated.compact());
    let options: Vec<(Arch, IcsOptions)> = input
        .arches
        .iter()
        .filter(|s| plan.calendars.contains_key(&s.arch))
        .map(|s| {
            (
                s.arch,
                IcsOptions {
                    title: params.title.clone(),
                    arch_label: arch_label(s.arch, lang).to_owned(),
                    lang,
                    stamp: stamp.clone(),
                    ..IcsOptions::default()
                },
            )
        })
        .collect();
    let ics_entries: Vec<(&Calendar, &IcsOptions)> = options
        .iter()
        .map(|(a, o)| (&plan.calendars[a], o))
        .collect();
    put(
        &dir,
        &mut entries,
        "visits.ics",
        "ics",
        schedule::to_ics_many(&ics_entries).into_bytes(),
    )?;
    for (&arch, cal) in &plan.calendars {
        let page = patient::patient_html(
            cal,
            &PatientOptions {
                lang,
                arch_label: arch_label(arch, lang).to_owned(),
                first: None,
                last: None,
                visit: None,
            },
        );
        put(
            &dir,
            &mut entries,
            &format!("{}_programma_paziente.html", prefix(arch)),
            "html",
            page.into_bytes(),
        )?;
        let csv = print_manifest_csv(arch, cal, &entries, &checks);
        put(
            &dir,
            &mut entries,
            &format!("{}_manifest_stampa.csv", prefix(arch)),
            "csv",
            csv.into_bytes(),
        )?;
    }

    // ---- print check summary ----
    let summary = params.print_check.then(|| PrintSummary {
        checked: checks.len(),
        passed: checks.iter().filter(|c| c.check.ok).count(),
        failed: checks
            .iter()
            .filter(|c| !c.check.ok)
            .map(|c| c.name.clone())
            .collect(),
    });
    if params.print_check {
        let value = serde_json::json!({
            "summary": summary,
            "build_plate_mm": params.build_plate_mm,
            "min_thickness_mm": params.min_thickness_mm,
            "files": checks.iter().map(|c| serde_json::json!({
                "name": c.name, "arch": arch_name(c.arch), "stage": c.stage, "kind": c.kind,
                "ok": c.check.ok,
                "problems_text": c.check.problem_texts(lang),
                "check": c.check,
            })).collect::<Vec<_>>(),
        });
        let bytes =
            serde_json::to_vec_pretty(&value).map_err(|e| ExportError::Package(e.to_string()))?;
        put(&dir, &mut entries, "print_check.json", "json", bytes)?;
    }

    // ---- report ----
    let generated_iso = generated.to_iso();
    if params.report {
        let md = md_signs(input);
        let renders = if params.embed_renders {
            case.renders.clone()
        } else {
            Renders::default()
        };
        let page = report::report_html(&ReportInput {
            patient_code: case.patient_code,
            title: &params.title,
            generated: &generated_iso,
            version: env!("CARGO_PKG_VERSION"),
            lang,
            state: case.state,
            md_sign: &md,
            calendars: &plan.calendars,
            attachments: &case.attachments,
            files: &entries,
            checks: &checks,
            renders: &renders,
            workflow: params.workflow,
            notes: &notes,
        });
        put(&dir, &mut entries, "report.html", "html", page.into_bytes())?;
    }

    // ---- manifest, last ----
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let first = plan
        .calendars
        .values()
        .next()
        .ok_or(ExportError::NoStages)?;
    let aligners: BTreeMap<String, u32> = plan
        .active
        .iter()
        .map(|(a, n)| (arch_name(*a).to_owned(), *n))
        .collect();
    let man = Manifest {
        format_version: manifest::FORMAT_VERSION,
        generator: Generator {
            name: "dentalcraft-aligners".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        patient_code: case.patient_code.to_owned(),
        generated: generated_iso,
        settings: serde_json::json!({
            "title": params.title,
            "start": first.start.to_iso(),
            "interval_days": params.interval_days.unwrap_or(case.state.settings.interval_days),
            "policy": params.policy,
            "lang": params.lang,
            "workflow": params.workflow,
            "wear_hours": first.wear_hours,
            "build_plate_mm": params.build_plate_mm,
            "min_thickness_mm": params.min_thickness_mm,
            "shell_thickness_mm": case.state.settings.shell_thickness_mm,
            "clearance_mm": case.state.settings.clearance_mm,
            "meshes": case.meshes,
            "quality": params.quality,
            "voxel_mm": params.voxel_mm.unwrap_or(params.quality.voxel_mm()),
            "stamp": params.stamp,
        }),
        aligners: aligners.clone(),
        total_aligners: plan.total,
        files: entries.clone(),
        print_check: summary.clone().unwrap_or_default(),
    };
    let bytes = serde_json::to_vec_pretty(&man).map_err(|e| ExportError::Package(e.to_string()))?;
    write_in(&dir, MANIFEST_NAME, &bytes)?;

    Ok(FullReport {
        dir: params.dir.clone(),
        files: entries,
        aligners,
        total_aligners: plan.total,
        start: first.start.to_iso(),
        end: first.end.to_iso(),
        total_days: first.total_days,
        print_check: summary,
        meshes: case.meshes.to_owned(),
        notes,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn md_signs(input: &Input<'_>) -> BTreeMap<dc_case::Fdi, i8> {
    input
        .arches
        .iter()
        .flat_map(|s| s.md_sign.iter().map(|(&f, &m)| (f, m)))
        .collect()
}

/// `<P>_manifest_stampa.csv`: one row per aligner of the arch with its
/// files, volume, size, whether it prints, dates and what to do first.
fn print_manifest_csv(
    arch: Arch,
    cal: &Calendar,
    entries: &[Entry],
    checks: &[FileCheck],
) -> String {
    let has = |name: &str| entries.iter().any(|e| e.name == name);
    let check = |name: &str| checks.iter().find(|c| c.name == name);
    let mut out = String::from(
        "aligner,plan_stage,passive,model,shell,volume_ml,size_mm,printable,start,end,before\n",
    );
    for r in &cal.rows {
        let k = r.aligner;
        let n = cal.n_active;
        let model = model_name(arch, k.min(n));
        let model = if !r.passive && has(&model) {
            model
        } else {
            String::new()
        };
        let shell = aligner_name(arch, k, r.passive);
        let shell = if has(&shell) { shell } else { String::new() };
        // the volume and size are the ones of the shell, else of the model
        let chk = check(&shell).or_else(|| check(&model));
        let (vol, size, printable) =
            chk.map_or((String::new(), String::new(), String::new()), |c| {
                (
                    format!("{:.2}", c.check.volume_ml()),
                    format!(
                        "{:.1}x{:.1}x{:.1}",
                        c.check.size_mm[0], c.check.size_mm[1], c.check.size_mm[2]
                    ),
                    if c.check.ok {
                        "yes".to_owned()
                    } else {
                        "no".to_owned()
                    },
                )
            });
        let _ = writeln!(
            out,
            "{},{},{},{},{},{vol},{size},{printable},{},{},{}",
            r.number,
            k.min(n),
            if r.passive { "yes" } else { "no" },
            csv_field(&model),
            csv_field(&shell),
            r.start.to_iso(),
            r.end.to_iso(),
            csv_field(&r.todo.join("; ")),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ArchSeries;
    use crate::export::html::check_wellformed;
    use crate::export::printcheck::tests::{cube, hollow};
    use crate::export::provider::ScanProvider;
    use crate::staging;
    use dc_case::{Fdi, Pose, ScanRole};
    use dc_mesh::TriMesh;
    use dca_model::{ArchDetails, ArchSpace, IprStep, Stage};
    use glam::{DVec3, Vec3};

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// Upper arch: tooth 11 moves in aligners 1-3; lower arch: tooth 31 in 1-5.
    fn stages() -> Vec<Stage> {
        (1..=5)
            .map(|k| {
                let mut s = Stage::new(k);
                s.poses.insert(
                    fdi(11),
                    staging::pose_from_q(&[0.25 * f64::from(k.min(3)), 0.0, 0.0, 0.0, 0.0, 0.0]),
                );
                s.poses.insert(
                    fdi(31),
                    staging::pose_from_q(&[0.2 * f64::from(k), 0.0, 0.0, 0.0, 0.0, 0.0]),
                );
                if k == 2 {
                    s.ipr.push(IprStep {
                        between: [fdi(11), fdi(21)],
                        amount_mm: 0.3,
                    });
                }
                s
            })
            .collect()
    }

    /// A scan of two cubes labelled with the teeth of the arch.
    fn scan(a: u16, b: u16) -> (TriMesh, Vec<u16>) {
        let (c1, c2) = (
            cube(Vec3::new(-6.0, 0.0, 0.0), 4.0),
            cube(Vec3::new(2.0, 0.0, 0.0), 4.0),
        );
        let off = c1.positions.len() as u32;
        let mut pos = c1.positions;
        pos.extend(c2.positions);
        let mut tris = c1.tris;
        tris.extend(c2.tris.iter().map(|t| t.map(|i| i + off)));
        let labels = (0..16).map(|i| if i < 8 { a } else { b }).collect();
        (TriMesh::new(pos, tris), labels)
    }

    struct Fixture {
        up: (TriMesh, Vec<u16>),
        lo: (TriMesh, Vec<u16>),
        rest_up: BTreeMap<Fdi, Pose>,
        rest_lo: BTreeMap<Fdi, Pose>,
        md: BTreeMap<Fdi, i8>,
        stages: Vec<Stage>,
        state: AlignersState,
    }

    fn fixture() -> Fixture {
        let mut state = AlignersState::default();
        state.diagnosis.arches.insert(
            ScanRole::Upper,
            ArchSpace {
                available_mm: 70.0,
                required_mm: 80.5,
                little_index_mm: 4.2,
            },
        );
        state.diagnosis.details.insert(
            ScanRole::Upper,
            ArchDetails {
                largest_space_mm: 1.2,
                midline_offset_mm: 0.4,
                catcat_score: 12,
            },
        );
        state.diagnosis.bolton_overall = Some(91.0);
        state.diagnosis.bolton_anterior = Some(77.5);
        state.diagnosis.complexity_score = Some(14.0);
        state.setup.ipr.push(IprStep {
            between: [fdi(11), fdi(21)],
            amount_mm: 0.3,
        });
        state.stages = stages();
        Fixture {
            up: scan(11, 21),
            lo: scan(31, 41),
            rest_up: BTreeMap::from([(
                fdi(11),
                Pose::from_translation(DVec3::new(-4.0, 2.0, 2.0)),
            )]),
            rest_lo: BTreeMap::from([(
                fdi(31),
                Pose::from_translation(DVec3::new(-4.0, 2.0, 2.0)),
            )]),
            md: BTreeMap::from([(fdi(11), 1), (fdi(31), 1)]),
            stages: stages(),
            state,
        }
    }

    impl Fixture {
        fn input(&self) -> Input<'_> {
            Input {
                arches: vec![
                    ArchSeries {
                        arch: Arch::Upper,
                        mesh: &self.up.0,
                        labels: &self.up.1,
                        rest: &self.rest_up,
                        md_sign: &self.md,
                        gingiva: None,
                    },
                    ArchSeries {
                        arch: Arch::Lower,
                        mesh: &self.lo.0,
                        labels: &self.lo.1,
                        rest: &self.rest_lo,
                        md_sign: &self.md,
                        gingiva: None,
                    },
                ],
                stages: &self.stages,
            }
        }

        fn case(&self) -> CaseInfo<'_> {
            CaseInfo {
                patient_code: "P-0042",
                state: &self.state,
                attachments: BTreeMap::from([(
                    Arch::Upper,
                    vec!["13".to_owned(), "23".to_owned()],
                )]),
                renders: Renders {
                    occlusal: Some(vec![0x89, b'P', b'N', b'G']),
                    frontal: None,
                },
                meshes: "test",
                notes: Vec::new(),
            }
        }
    }

    /// Closed cubes whose position follows the stage; the shell of aligner
    /// `broken` (when given) has a hole.
    struct Cubes {
        broken: Option<(Arch, u32)>,
    }

    impl MeshProvider for Cubes {
        fn model(&self, arch: Arch, stage: u32) -> Result<Option<TriMesh>, ExportError> {
            let y = if arch.is_upper() { 0.0 } else { 30.0 };
            Ok(Some(cube(Vec3::new(0.25 * stage as f32, y, 0.0), 10.0)))
        }

        fn aligner(&self, arch: Arch, stage: u32) -> Result<Option<TriMesh>, ExportError> {
            let y = if arch.is_upper() { 0.0 } else { 30.0 };
            let mut m = hollow(12.0, 1.0);
            // keep the cavity attached to the outside so that it is one piece? no: a shell is
            // one outer surface plus one inner surface
            for p in &mut m.positions {
                p.x += 0.25 * stage as f32;
                p.y += y;
            }
            if self.broken == Some((arch, stage)) {
                m.tris.pop();
            }
            Ok(Some(m))
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dca-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn params(dir: &Path) -> FullParams {
        FullParams {
            dir: dir.display().to_string(),
            start_date: Some("2026-03-02".into()),
            interval_days: Some(7),
            generated: Some("2026-03-01".into()),
            title: "Case <A>".into(),
            // the hollow cube is two pieces of surface: a cavity
            ..FullParams::default()
        }
    }

    #[test]
    fn a_full_export_writes_every_file_and_the_manifest_verifies() {
        let f = fixture();
        let dir = temp("all");
        let report = run(
            &f.input(),
            &f.case(),
            &Cubes { broken: None },
            &params(&dir),
        )
        .unwrap();
        assert_eq!(report.aligners["upper"], 3);
        assert_eq!(report.aligners["lower"], 5);
        assert_eq!(report.total_aligners, 5);
        assert_eq!(report.start, "2026-03-02");
        assert_eq!(report.total_days, 35);

        let names: Vec<&str> = report.files.iter().map(|e| e.name.as_str()).collect();
        // models 0..=n of both arches, shells 1..=n, passive upper 4 and 5
        for k in 0..=3 {
            assert!(names.contains(&model_name(Arch::Upper, k).as_str()), "{k}");
        }
        assert!(names.contains(&"modelli_termoformatura/SUP_modello_00_iniziale.stl"));
        assert!(names.contains(&"modelli_termoformatura/INF_modello_05.stl"));
        assert!(names.contains(&"stampa_resina/SUP_allineatore_03.stl"));
        assert!(names.contains(&"stampa_resina/SUP_allineatore_04_passivo.stl"));
        assert!(names.contains(&"stampa_resina/SUP_allineatore_05_passivo.stl"));
        assert!(names.contains(&"stampa_resina/INF_allineatore_05.stl"));
        assert_eq!(
            names.iter().filter(|n| n.ends_with(".stl")).count(),
            4 + 6 + 3 + 2 + 5
        );
        for n in [
            "stages.csv",
            "visits.ics",
            "report.html",
            "print_check.json",
            "SUP_programma_paziente.html",
            "INF_programma_paziente.html",
            "SUP_manifest_stampa.csv",
            "INF_manifest_stampa.csv",
        ] {
            assert!(names.contains(&n), "missing {n}");
            assert!(dir.join(n).is_file(), "{n} not on disk");
        }
        assert!(dir.join(MANIFEST_NAME).is_file());
        // the passive aligner is a copy of the last one
        assert_eq!(
            std::fs::read(dir.join("stampa_resina/SUP_allineatore_05_passivo.stl")).unwrap(),
            std::fs::read(dir.join("stampa_resina/SUP_allineatore_03.stl")).unwrap()
        );

        // the manifest lists every file except itself and verifies
        let v = manifest::verify(&dir).unwrap();
        assert!(v.ok(), "{v:?}");
        assert_eq!(v.listed, report.files.len());
        assert_eq!(v.verified, v.listed);
        let m = manifest::read(&dir).unwrap();
        assert_eq!(m.patient_code, "P-0042");
        assert_eq!(m.generated, "2026-03-01");
        assert_eq!(m.files, report.files);
        assert_eq!(m.settings["interval_days"], 7);
        // and notices a change
        std::fs::write(dir.join("stages.csv"), b"tampered").unwrap();
        assert_eq!(manifest::verify(&dir).unwrap().mismatched, ["stages.csv"]);

        // no temporary files are left
        assert!(
            manifest::list_files(&dir)
                .unwrap()
                .iter()
                .all(|n| !n.ends_with(".tmp"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_print_check_flags_the_broken_aligner_only() {
        let f = fixture();
        let dir = temp("check");
        let mut p = params(&dir);
        // the hollow cube is two surfaces, so ask only what the test is about
        p.min_thickness_mm = Some(0.5);
        let r = run(
            &f.input(),
            &f.case(),
            &Cubes {
                broken: Some((Arch::Lower, 2)),
            },
            &p,
        )
        .unwrap();
        let s = r.print_check.unwrap();
        // models are single cubes, shells have a cavity (two pieces of surface)
        assert!(
            s.failed
                .contains(&"stampa_resina/INF_allineatore_02.stl".to_owned())
        );
        assert!(
            !s.failed
                .contains(&"modelli_termoformatura/INF_modello_02.stl".to_owned())
        );
        assert!(
            !s.failed
                .contains(&"modelli_termoformatura/SUP_modello_00_iniziale.stl".to_owned())
        );
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("print_check.json")).unwrap()).unwrap();
        let bad = json["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "stampa_resina/INF_allineatore_02.stl")
            .unwrap();
        assert_eq!(bad["ok"], false);
        assert!(
            bad["problems_text"][0]
                .as_str()
                .unwrap()
                .contains("open edges")
        );
        let good = json["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "modelli_termoformatura/INF_modello_02.stl")
            .unwrap();
        assert_eq!(good["ok"], true);
        // the manifest says it too
        let m = manifest::read(&dir).unwrap();
        let e = m
            .files
            .iter()
            .find(|e| e.name == "stampa_resina/INF_allineatore_02.stl")
            .unwrap();
        assert_eq!(e.printable, Some(false));
        assert!(!m.print_check.failed.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_export_does_not_depend_on_the_number_of_threads() {
        let f = fixture();
        let mut hashes = Vec::new();
        for threads in [1, 2, 7] {
            let dir = temp(&format!("threads{threads}"));
            let mut p = params(&dir);
            p.threads = Some(threads);
            run(
                &f.input(),
                &f.case(),
                &Cubes {
                    broken: Some((Arch::Upper, 1)),
                },
                &p,
            )
            .unwrap();
            let files = manifest::scan(&dir).unwrap();
            hashes.push(
                files
                    .iter()
                    .map(|e| (e.name.clone(), e.blake3.clone()))
                    .collect::<Vec<_>>(),
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
        assert_eq!(hashes[0].len(), 28);
        assert_eq!(hashes[0], hashes[1]);
        assert_eq!(hashes[0], hashes[2]);
    }

    #[test]
    fn the_pages_are_well_formed_in_both_languages() {
        let f = fixture();
        let mut out = Vec::new();
        for lang in [Lang::En, Lang::It] {
            let dir = temp(&format!("html-{lang:?}"));
            let mut p = params(&dir);
            p.lang = lang;
            run(
                &f.input(),
                &f.case(),
                &Cubes {
                    broken: Some((Arch::Lower, 3)),
                },
                &p,
            )
            .unwrap();
            let report = std::fs::read_to_string(dir.join("report.html")).unwrap();
            let patient = std::fs::read_to_string(dir.join("SUP_programma_paziente.html")).unwrap();
            for page in [&report, &patient] {
                let tags = check_wellformed(page).unwrap();
                assert!(tags > 20);
                for key in ["rpt-", "pat-", "chk-"] {
                    assert!(!page.contains(key), "{lang:?}: untranslated {key}");
                }
                // the title is user text: escaped
                assert!(!page.contains("<A>"));
            }
            assert!(report.contains("Case &lt;A&gt;"));
            assert!(report.contains("P-0042"));
            // diagnosis, IPR, attachments and the print check are there
            assert!(report.contains("11|21"));
            assert!(report.contains("13, 23"));
            assert!(report.contains("data:image/png;base64,iVBORw=="));
            out.push((report, patient));
            std::fs::remove_dir_all(&dir).unwrap();
        }
        let (en, it) = (&out[0], &out[1]);
        assert!(en.0.contains("lang=\"en\"") && it.0.contains("lang=\"it\""));
        assert!(en.0.contains("Treatment calendar") && it.0.contains("Calendario del trattamento"));
        assert!(en.0.contains("Diagnosis from the model") && it.0.contains("Diagnosi dal modello"));
        assert!(en.0.contains("open edges") && it.0.contains("spigoli aperti"));
        assert!(en.1.contains("Your programme") && it.1.contains("Il tuo programma"));
        assert!(en.1.contains("2026-03-02") && it.1.contains("02/03/2026"));
        assert!(en.1.contains("(passive)") && it.1.contains("(passivo)"));
        assert!(en.0.contains("7.0") || en.0.contains("0.30"));
        assert!(it.0.contains("0,30"));
    }

    #[test]
    fn the_default_provider_gives_the_deformed_scans_as_models() {
        let f = fixture();
        let input = f.input();
        let dir = temp("scan");
        let mut p = params(&dir);
        p.self_intersections = false;
        p.embed_renders = false;
        let r = run(&input, &f.case(), &ScanProvider::new(&input), &p).unwrap();
        let stl: Vec<&str> = r
            .files
            .iter()
            .filter(|e| e.kind == "stl")
            .map(|e| e.name.as_str())
            .collect();
        // models only: no shell yet
        assert_eq!(stl.len(), 4 + 6);
        assert!(stl.iter().all(|n| n.starts_with("modelli_termoformatura/")));
        // the scan is two disjoint cubes: not one piece
        let s = r.print_check.unwrap();
        assert_eq!(s.checked, 10);
        let report = std::fs::read_to_string(dir.join("report.html")).unwrap();
        assert!(!report.contains("data:image"));
        assert!(report.contains("2 separate pieces"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_workflow_and_the_check_can_be_switched_off() {
        let f = fixture();
        let dir = temp("workflow");
        let mut p = params(&dir);
        p.workflow = Workflow::Direct;
        p.print_check = false;
        p.report = false;
        let r = run(&f.input(), &f.case(), &Cubes { broken: None }, &p).unwrap();
        assert!(r.print_check.is_none());
        assert!(
            r.files
                .iter()
                .all(|e| !e.name.starts_with("modelli_termoformatura/"))
        );
        assert!(
            r.files
                .iter()
                .all(|e| e.name != "report.html" && e.name != "print_check.json")
        );
        assert!(
            r.files
                .iter()
                .any(|e| e.name == "stampa_resina/INF_allineatore_05.stl")
        );
        let csv = std::fs::read_to_string(dir.join("SUP_manifest_stampa.csv")).unwrap();
        assert_eq!(csv.lines().count(), 1 + 5);
        assert!(csv.lines().nth(4).unwrap().contains("yes,"), "{csv}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_input_is_reported_before_anything_is_written() {
        let f = fixture();
        let dir = temp("bad");
        let mut p = params(&dir);
        p.generated = Some("yesterday".into());
        assert!(matches!(
            run(&f.input(), &f.case(), &Cubes { broken: None }, &p),
            Err(ExportError::Package(_))
        ));
        p.generated = None;
        p.start_date = Some("soon".into());
        assert!(matches!(
            run(&f.input(), &f.case(), &Cubes { broken: None }, &p),
            Err(ExportError::Schedule(_))
        ));
        assert!(!dir.exists());

        // no directory, labels that do not belong to the scan
        let mut p = params(&dir);
        p.dir = " ".into();
        assert!(matches!(
            run(&f.input(), &f.case(), &Cubes { broken: None }, &p),
            Err(ExportError::NoDir)
        ));
        let mut input = f.input();
        input.arches[0].labels = &f.up.1[..3];
        assert!(matches!(
            run(&input, &f.case(), &Cubes { broken: None }, &params(&dir)),
            Err(ExportError::Labels { .. })
        ));
        assert!(!dir.exists());
    }
}
