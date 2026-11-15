//! Scan import with recognition of the arch, and the arch step of the
//! pipeline.
//!
//! * `aligners.case.import-scans` is `case.import` with a resolver: a file
//!   with neither an explicit role nor a name hint gets its arch from the
//!   geometry ([`autoseed::detect_arch`]). Same parameters and same output as
//!   `case.import`, plus `detections` and `bases`, because MCP and the CLI
//!   prefer it over `case.import` ([`dc_module_api::IMPORT_SCANS_SUFFIX`]).
//!   The app calls [`import_scans`] directly.
//! * `aligners.case.detect-arch` is the plugin's `arcata` step: it refuses an
//!   arch scan with too few faces (`case_auto.auto_steps`) and resolves the
//!   scans still [`RoleSource::Provisional`]. `aligners.auto` runs it first.
//!   It is not a menu leaf.
//!
//! The roles are never decided here for a scan with a surer source: explicit,
//! file name, detected and detected-unsure roles stay as they are. A
//! detection from a cue the plugin marks "da verificare"
//! ([`ArchSource::to_be_checked`]) is stored as
//! [`RoleSource::DetectedUnsure`]: settled, with a warning; `case.set-role`
//! with the same role on both sides confirms it.
//!
//! Detection runs on the thread of the job and needs no decimation: about
//! 0.3 s in a release build on a real scan of 855k faces.
//!
//! Every imported arch scan that is open under the gum is then closed with a
//! flat base in the same change ([`close_imported`]): the closed solid is the
//! scan of the case from then on, the scan as it was read stays its start
//! (`AlignersState::scan_bases`, `crate::open_scan`). A deviation from the
//! plugin, which closes a scan only when asked and needs the tooth marks for
//! it: the import has none, so the base is perpendicular to the occlusal
//! direction of the scan itself (`dca_algo::scan_base::occlusal_direction`).
//! A bite scan stays as it is, and so does a scan that is closed already.

use std::collections::BTreeSet;
use std::path::PathBuf;

use dc_case::import::{
    Detection, ImportFile, ImportReport, ImportRequest, ImportWarning, LOW_POLYGON_FACES,
    PreparedScan, ScanMove, WarningCode, import_files, roles_changed,
};
use dc_case::{CaseState, ModuleState, RoleSource, Scan, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::autoseed::{self, ArchDetection, ArchSource};
use dca_algo::scan_base::{self, HEIGHT_DEFAULT_MM};
use dca_model::{AlignersState, MODULE_ID};
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Id of the import command.
pub const IMPORT_SCANS: &str = "aligners.case.import-scans";

/// Id of the arch step.
pub const DETECT_ARCH: &str = "aligners.case.detect-arch";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: IMPORT_SCANS,
            label: "aligners-menu-case-import-scans",
            doc: "aligners-cmd-import-scans-doc",
            params: ImportParams,
            run: run_import,
        },
        stage_command! {
            id: DETECT_ARCH,
            label: "aligners-cmd-detect-arch",
            doc: "aligners-cmd-detect-arch-doc",
            params: DetectArchParams,
            run: run_detect_arch,
        },
    ]
}

/// Role of an imported scan, or `auto` to let the import decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RoleChoice {
    /// The import decides: file name, then the geometry, then a free arch
    /// (provisional).
    Auto,
    /// Upper (maxillary) arch.
    Upper,
    /// Lower (mandibular) arch.
    Lower,
    /// Bite registration.
    Bite,
}

impl RoleChoice {
    /// The role chosen, `None` for `auto`.
    fn role(self) -> Option<ScanRole> {
        match self {
            Self::Auto => None,
            Self::Upper => Some(ScanRole::Upper),
            Self::Lower => Some(ScanRole::Lower),
            Self::Bite => Some(ScanRole::Bite),
        }
    }
}

/// One file of a batch.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportFileParam {
    /// Path of the STL, OBJ or PLY file.
    path: String,
    /// Arch or bite of this file; `auto` or missing lets the import decide.
    #[serde(default)]
    role: Option<RoleChoice>,
}

/// Parameters of `aligners.case.import-scans`, the ones of `case.import`:
/// exactly one of `path` and `files`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("oneOf" = [{ "required": ["path"] }, { "required": ["files"] }]))]
struct ImportParams {
    /// Path of one STL, OBJ or PLY file. Give this or `files`.
    #[serde(default)]
    path: Option<String>,
    /// Several files, each with its own optional role. Give this or `path`.
    #[serde(default)]
    files: Option<Vec<ImportFileParam>>,
    /// Arch or bite of `path`; the default for the entries of `files` that
    /// have no role of their own. `auto` or missing lets the import decide
    /// (file name, then the geometry, then a free arch, marked provisional).
    #[serde(default)]
    role: Option<RoleChoice>,
}

/// Parameters of `aligners.case.detect-arch` (none).
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct DetectArchParams {}

/// A file whose arch was recognised from the geometry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanDetection {
    /// The file.
    pub path: PathBuf,
    /// File name without directory.
    pub source_name: String,
    /// What the geometry said. The role in the case may differ: see
    /// [`ImportReport::imported`] and its arch conflict warnings.
    #[serde(flatten)]
    pub detection: ArchDetection,
}

/// Result of [`import_scans`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScanImport {
    /// The report of the import. Its `warnings` also hold the warnings of
    /// this module, in the same form: failed recognition, arch to check after
    /// segmentation. A bite registration that an arch replacement leaves
    /// applied is reported in `module_cleared` (`bite_registered`). An arch
    /// scan closed with a base is reported as the case holds it: the blob
    /// and the counts of the closed solid.
    pub report: ImportReport,
    /// The recognised files, in request order.
    pub detections: Vec<ScanDetection>,
    /// The base of every imported arch scan, in the order of
    /// `report.imported` ([`close_imported`]).
    pub bases: Vec<ScanClosure>,
}

/// What the import did to close one imported arch scan with a base.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanClosure {
    /// Id of the scan (as in the `imported` list of the report).
    pub scan_id: String,
    /// Its role.
    pub role: ScanRole,
    /// It had no open edge: it stays as it was read.
    pub already_closed: bool,
    /// Open edges closed (`0` when it was not closed).
    pub closed_edges: usize,
    /// Points the base added.
    pub vertices_added: usize,
    /// Triangles the base added.
    pub faces_added: usize,
    /// Thickness of the base under the lowest point of the scan (mm).
    pub height_mm: f64,
    /// The scan of the case is a closed solid.
    pub watertight: bool,
    /// Why an open scan was not closed (it stays open; "Close with a base"
    /// closes it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_closed: Option<String>,
}

/// The step of the import that closes the scans, for the status bar.
pub const CLOSING_KEY: &str = "aligners-import-closing";

fn tr(key: &str, args: &[(&str, &str)]) -> String {
    crate::loader();
    dc_i18n::tr_args(key, args)
}

fn invalid(key: &str, args: &[(&str, &str)]) -> CommandError {
    CommandError::Invalid(tr(key, args))
}

fn warning(code: WarningCode, key: &'static str, args: &[(&str, &str)]) -> ImportWarning {
    crate::loader(); // its `message` looks the key up when it is serialized
    let args = args
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    ImportWarning { code, key, args }
}

fn role_id(role: ScanRole) -> &'static str {
    match role {
        ScanRole::Upper => "upper",
        ScanRole::Lower => "lower",
        ScanRole::Bite => "bite",
    }
}

fn opposite(role: ScanRole) -> ScanRole {
    match role {
        ScanRole::Upper => ScanRole::Lower,
        _ => ScanRole::Upper,
    }
}

/// The ids of `module_cleared` as a translated list, for a toast. An id this
/// module does not know (another module cleared it) stays as it is.
pub fn cleared_list(ids: &[&str]) -> String {
    crate::loader();
    let names: Vec<String> = ids
        .iter()
        .map(|id| {
            let key = format!("aligners-cleared-{}", id.replace('_', "-"));
            if dc_i18n::has(&key) {
                dc_i18n::tr(&key)
            } else {
                (*id).to_owned()
            }
        })
        .collect();
    names.join(", ")
}

/// Refuses a scan whose arch is still a guess: everything computed on it would
/// have to be redone when the guess changes.
pub(crate) fn require_settled(scan: &Scan) -> Result<(), CommandError> {
    if scan.role_source != RoleSource::Provisional {
        return Ok(());
    }
    Err(invalid(
        "aligners-arch-error-provisional",
        &[("name", &scan.source_name), ("role", role_id(scan.role))],
    ))
}

/// The arch of a mesh, or the technical cause of the failure.
///
/// [`autoseed::detect_arch`] keeps the plugin's arithmetic and always answers
/// once the geometry could be analysed. Two answers are no recognition, so
/// they count as a failure and the scan stays provisional:
/// * the proposal failed for both arches (both costs infinite): the
///   comparison of the widths is NaN and the plugin's answer, an upper arch
///   at 0.6, rests on nothing ([`ArchDetection::widths_failed`]);
/// * no confidence at all: the two width hypotheses cost the same.
pub(crate) fn detect(mesh: &TriMesh, name: &str) -> Result<ArchDetection, String> {
    let vertices: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let detection =
        autoseed::detect_arch(&vertices, &mesh.tris, Some(name)).map_err(|e| e.to_string())?;
    recognised(detection)
}

/// `detection` as a recognition, or the reason it is none (see [`detect`]).
fn recognised(detection: ArchDetection) -> Result<ArchDetection, String> {
    if detection.widths_failed() {
        return Err("the tooth widths could not be compared for either arch".to_owned());
    }
    if detection.confidence > 0.0 {
        Ok(detection)
    } else {
        Err("no cue: the upper and lower arch hypotheses are equal".to_owned())
    }
}

fn detection_failed(name: &str, detail: &str) -> ImportWarning {
    // `detail` is the technical cause, for the JSON; the message does not show it
    warning(
        WarningCode::Provisional,
        "aligners-arch-warn-not-recognised",
        &[("name", name), ("detail", detail)],
    )
}

/// The role source of a scan placed on the arch `detection` recognised:
/// [`RoleSource::DetectedUnsure`] when the cue is one the plugin marks "da
/// verificare" (`autoseed.py`, `detect_arch`: the palate that disagrees with
/// the molar cusps, the Wilson cusps over a closed model base, and the shape
/// and widths of the teeth alone), otherwise [`RoleSource::Detected`].
///
/// The plugin does not trust the first and the last path
/// (`case_auto.auto_steps`, the sources starting with "forma"): it treats the
/// scan as a lower arch, then after the reconstruction compares the crown
/// widths and restarts as an upper arch when they fit better. That
/// verification loop is ported in `aligners.auto` (`commands::auto`, output
/// `arch_check`), which runs the pipeline; the import itself only has the
/// guess of [`autoseed::detect_arch`], which it keeps, settled, telling the
/// user to check it.
fn detected_source(source: ArchSource) -> RoleSource {
    if source.to_be_checked() {
        RoleSource::DetectedUnsure
    } else {
        RoleSource::Detected
    }
}

fn width_check(name: &str, role: ScanRole) -> ImportWarning {
    warning(
        WarningCode::Provisional,
        "aligners-arch-warn-to-be-checked",
        &[("name", name), ("role", role_id(role))],
    )
}

/// Turns into [`RoleSource::DetectedUnsure`], in `state` and in the report,
/// the imported scans that the import placed on the arch the geometry said
/// (`Detected`) from a cue to be checked, and warns about each. A scan that
/// went to the other arch (arch conflict) is provisional and not touched.
fn mark_unsure(state: &mut CaseState, report: &mut ImportReport, detections: &[ScanDetection]) {
    for d in detections {
        if detected_source(d.detection.source) != RoleSource::DetectedUnsure {
            continue;
        }
        let settled = report
            .imported
            .iter_mut()
            .find(|s| s.path == d.path && s.role_source == RoleSource::Detected);
        if let Some(scan) = settled {
            scan.role_source = RoleSource::DetectedUnsure;
            if let Some(held) = state.case.scans.iter_mut().find(|s| s.id == scan.scan_id) {
                held.role_source = RoleSource::DetectedUnsure;
            }
            report
                .warnings
                .push(width_check(&scan.source_name, scan.role));
        }
    }
}

/// Imports a batch into `state`, recognising the arch of the files with
/// neither an explicit role nor a name hint, and closes the open arch scans
/// with a base ([`close_imported`]): the Aligners form of
/// [`dc_case::import::import_files`], for `create_job` and for the command.
///
/// Runs on the caller's thread, so a job can call it. A scan under
/// [`LOW_POLYGON_FACES`] faces is not analysed, and a scan that cannot be
/// recognised is not refused: both take a free arch as provisional. When
/// nothing is imported or the job is cancelled, `state` stays as it was.
/// Callers holding a shared state clone it first and keep the old one for
/// undo: the whole batch is one change.
pub fn import_scans(state: &mut CaseState, request: &ImportRequest, ctx: &JobCtx) -> ScanImport {
    let before = state.clone();
    let mut detections = Vec::new();
    let mut failures = Vec::new();
    let mut resolve = |scan: &PreparedScan| -> Option<Detection> {
        // the low_polygons warning of the import covers this one
        if scan.mesh.face_count() < LOW_POLYGON_FACES {
            return None;
        }
        match detect(&scan.mesh, &scan.source_name) {
            Ok(detection) => {
                detections.push(ScanDetection {
                    path: scan.path.clone(),
                    source_name: scan.source_name.clone(),
                    detection,
                });
                Some(Detection {
                    arch: detection.arch.role(),
                    confidence: detection.confidence as f32,
                })
            }
            Err(e) => {
                failures.push(detection_failed(&scan.source_name, &e));
                None
            }
        }
    };
    let mut report = import_files(state, request, Some(&mut resolve), ctx);
    if report.cancelled {
        return ScanImport {
            report,
            ..ScanImport::default()
        };
    }
    report.warnings.extend(failures);
    mark_unsure(state, &mut report, &detections);
    let Some(bases) = close_imported(state, &mut report, ctx) else {
        *state = before;
        return ScanImport {
            report: ImportReport {
                cancelled: true,
                ..ImportReport::default()
            },
            ..ScanImport::default()
        };
    };
    ScanImport {
        report,
        detections,
        bases,
    }
}

/// Closes with a base every arch scan of `report.imported` that is open
/// under the gum ([`scan_base::make_base`], [`HEIGHT_DEFAULT_MM`] under its
/// lowest point, perpendicular to [`scan_base::occlusal_direction`]: the
/// case has no tooth marks yet). The closed solid becomes the scan of the
/// case, and `report` describes it (its blob and counts); where the scan as
/// it was read ends in it is kept (`AlignersState::scan_bases`). A bite
/// scan and a closed scan stay as they are, and so does a scan whose
/// direction cannot be found (said in [`ScanClosure::not_closed`]).
/// `None` when the job was cancelled: `state` is then half done.
fn close_imported(
    state: &mut CaseState,
    report: &mut ImportReport,
    ctx: &JobCtx,
) -> Option<Vec<ScanClosure>> {
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => match m.downcast_ref::<AlignersState>() {
            Some(d) => d.clone(),
            // data of a newer version, which this one cannot write: the
            // scans stay as they were read
            None => return Some(Vec::new()),
        },
    };
    let mut out = Vec::new();
    let mut stale = Vec::new();
    for imported in report
        .imported
        .iter_mut()
        // a bite stays as it is; a mesh too small to be a dental scan too
        // (the arch step refuses it)
        .filter(|s| s.role != ScanRole::Bite && s.faces >= LOW_POLYGON_FACES)
    {
        if ctx.is_cancelled() {
            return None;
        }
        ctx.set_progress(1.0, Some(CLOSING_KEY));
        let Some(mesh) = state.blobs.get(&imported.mesh).cloned() else {
            continue;
        };
        let mut closure = ScanClosure {
            scan_id: imported.scan_id.to_string(),
            role: imported.role,
            already_closed: false,
            closed_edges: 0,
            vertices_added: 0,
            faces_added: 0,
            height_mm: HEIGHT_DEFAULT_MM,
            watertight: false,
            not_closed: None,
        };
        let based = scan_base::occlusal_direction(&mesh)
            .and_then(|n| scan_base::make_base(&mesh, n, HEIGHT_DEFAULT_MM));
        match based {
            Ok(None) => {
                closure.already_closed = true;
                closure.watertight = mesh.validate().is_watertight();
            }
            Err(e) => closure.not_closed = Some(e.to_string()),
            Ok(Some(based)) => {
                closure.closed_edges = based.closed_edges;
                closure.vertices_added = based.sources.len();
                closure.faces_added = based.mesh.face_count() - mesh.face_count();
                closure.watertight = based.mesh.validate().is_watertight();
                let (vertices, faces) = (based.mesh.vertex_count(), based.mesh.face_count());
                let r = state.blobs.insert(based.mesh);
                for s in state
                    .case
                    .scans
                    .iter_mut()
                    .filter(|s| s.id == imported.scan_id)
                {
                    s.mesh = r;
                }
                data.scan_bases.insert(r, based.scan);
                stale.push(imported.mesh);
                imported.mesh = r;
                imported.vertices = vertices;
                imported.faces = faces;
            }
        }
        out.push(closure);
    }
    if !stale.is_empty() {
        data.prune_scan_bases(state.case.scans.iter().map(|s| &s.mesh));
        state.set_module(MODULE_ID, ModuleState::new(data));
        crate::manual::common::finish_blobs(state, &stale);
    }
    ctx.set_progress(1.0, None);
    Some(out)
}

/// The files of an import, from exactly one of `path` and `files`.
fn import_request(p: &ImportParams) -> Result<ImportRequest, CommandError> {
    let default = p.role.and_then(RoleChoice::role);
    let files = match (&p.path, &p.files) {
        (Some(path), None) => vec![ImportFile {
            path: path.into(),
            role: default,
        }],
        (None, Some(files)) if !files.is_empty() => files
            .iter()
            .map(|f| ImportFile {
                path: (&f.path).into(),
                role: f.role.map_or(default, RoleChoice::role),
            })
            .collect(),
        (None, Some(_)) => return Err(invalid("aligners-import-error-no-files", &[])),
        _ => return Err(invalid("aligners-import-error-path-or-files", &[])),
    };
    Ok(ImportRequest { files })
}

/// The reasons of the failed files, one sentence each.
fn failure_text(report: &ImportReport) -> String {
    let mut reasons: Vec<String> = report
        .failed
        .iter()
        .map(|f| match &f.detail {
            Some(d) => format!("{} ({d})", f.reason),
            None => f.reason.clone(),
        })
        .collect();
    reasons.dedup(); // a reason about the project is the same for every file
    tr(
        "aligners-import-error-nothing",
        &[("reasons", &reasons.join(" "))],
    )
}

fn to_value(v: impl Serialize) -> Result<Value, CommandError> {
    serde_json::to_value(v).map_err(|e| CommandError::Failed(e.to_string()))
}

fn run_import(
    state: &CaseState,
    params: ImportParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let request = import_request(&params)?;
    ctx.check_cancelled()?;
    let mut next = state.clone();
    let ScanImport {
        report,
        detections,
        bases,
    } = import_scans(&mut next, &request, ctx);
    if report.cancelled {
        return Err(CommandError::Cancelled);
    }
    if report.imported.is_empty() {
        return Err(CommandError::Failed(failure_text(&report)));
    }
    let replaced = state
        .case
        .scans
        .iter()
        .any(|s| next.case.scan_by_id(s.id).is_none());
    let imported = to_value(&report.imported)?;
    // the single-file fields of case.import describe the first imported scan
    let mut out = Map::new();
    if let Some(Value::Object(first)) = imported.get(0) {
        for key in [
            "scan_id",
            "role",
            "role_source",
            "source_name",
            "source_units",
            "mesh",
            "vertices",
            "faces",
        ] {
            if let Some(v) = first.get(key) {
                out.insert(key.to_owned(), v.clone());
            }
        }
    }
    out.insert("replaced".into(), json!(replaced));
    out.insert("imported".into(), imported);
    out.insert("warnings".into(), to_value(&report.warnings)?);
    out.insert("failed".into(), to_value(&report.failed)?);
    out.insert("moved".into(), to_value(&report.moved)?);
    out.insert("cleanup".into(), to_value(report.cleanup)?);
    out.insert("module_cleared".into(), json!(report.module_cleared));
    out.insert("detections".into(), to_value(&detections)?);
    out.insert("bases".into(), to_value(&bases)?);
    Ok((next, Value::Object(out)))
}

/// What the arch step did to a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// The scan was not provisional: left alone.
    Unchanged,
    /// The arch was recognised: the scan is on it, marked detected (or
    /// detected unsure, with a warning, for a cue the plugin marks to be
    /// checked).
    Resolved,
    /// The arch recognised is taken: the scan stays provisional on the other.
    Conflict,
    /// The arch was not recognised: the scan stays provisional.
    NotRecognised,
}

/// Where the arch step puts a provisional scan.
struct Placement<'a> {
    scan: &'a Scan,
    to: ScanRole,
    source: RoleSource,
    outcome: Outcome,
    detection: Option<ArchDetection>,
}

/// Places the provisional scans: the rules of
/// [`allocate`](dc_case::import::allocate) for scans already in the case.
/// Recognised ones go surest first, each to its arch, or to the other one with
/// an arch conflict warning when a surer scan holds it. `taken` holds the
/// arches of the scans that are not provisional.
fn place_provisional<'a>(
    mut detected: Vec<(&'a Scan, ArchDetection)>,
    not_recognised: Vec<&'a Scan>,
    mut taken: BTreeSet<ScanRole>,
    warnings: &mut Vec<ImportWarning>,
) -> Vec<Placement<'a>> {
    detected.sort_by(|a, b| b.1.confidence.total_cmp(&a.1.confidence));
    let mut out = Vec::with_capacity(detected.len() + not_recognised.len());
    for (scan, detection) in detected {
        let want = detection.arch.role();
        let (to, source, outcome) = if taken.contains(&want) {
            let other = opposite(want);
            let to = if taken.contains(&other) {
                scan.role
            } else {
                other
            };
            warnings.push(warning(
                WarningCode::ArchConflict,
                "case-import-warn-arch-conflict",
                &[("name", &scan.source_name), ("role", role_id(to))],
            ));
            (to, RoleSource::Provisional, Outcome::Conflict)
        } else {
            let source = detected_source(detection.source);
            if source == RoleSource::DetectedUnsure {
                warnings.push(width_check(&scan.source_name, want));
            }
            (want, source, Outcome::Resolved)
        };
        taken.insert(to);
        out.push(Placement {
            scan,
            to,
            source,
            outcome,
            detection: Some(detection),
        });
    }
    for scan in not_recognised {
        let to = [scan.role, opposite(scan.role)]
            .into_iter()
            .find(|r| !taken.contains(r))
            .unwrap_or(scan.role);
        taken.insert(to);
        out.push(Placement {
            scan,
            to,
            source: RoleSource::Provisional,
            outcome: Outcome::NotRecognised,
            detection: None,
        });
    }
    out
}

fn run_detect_arch(
    state: &CaseState,
    _params: DetectArchParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let arch_scans = || state.case.scans.iter().filter(|s| s.role != ScanRole::Bite);
    if arch_scans().next().is_none() {
        return Err(invalid("aligners-arch-error-no-scan", &[]));
    }
    // case_auto.py: `len(obj.data.polygons) < 1000`, whatever the source of
    // the arch; the scan as it was read (without the base it was closed with)
    let data = super::staging::aligners_data(state)?;
    for scan in arch_scans() {
        let mesh = state.blobs.require(&scan.mesh)?;
        let faces = data
            .scan_base(&scan.mesh)
            .map_or(mesh.face_count(), |b| b.faces as usize);
        if faces < LOW_POLYGON_FACES {
            return Err(invalid(
                "aligners-arch-error-not-dental",
                &[("name", &scan.source_name), ("faces", &faces.to_string())],
            ));
        }
    }

    let is_provisional = |s: &&Scan| s.role_source == RoleSource::Provisional;
    let taken = arch_scans()
        .filter(|s| s.role_source != RoleSource::Provisional)
        .map(|s| s.role)
        .collect();
    let mut warnings = Vec::new();
    let mut detected = Vec::new();
    let mut not_recognised = Vec::new();
    for scan in arch_scans().filter(is_provisional) {
        let mesh = crate::open_scan::open_scan(state, &data, &scan.mesh)?;
        match detect(&mesh, &scan.source_name) {
            Ok(d) => detected.push((scan, d)),
            Err(e) => {
                warnings.push(detection_failed(&scan.source_name, &e));
                not_recognised.push(scan);
            }
        }
        ctx.check_cancelled()?;
    }
    let placements = place_provisional(detected, not_recognised, taken, &mut warnings);

    let mut next = state.clone();
    let before = next.referenced_blobs();
    let mut touched = BTreeSet::new();
    let mut moved = Vec::new();
    for p in &placements {
        if p.to != p.scan.role {
            touched.extend([p.scan.role, p.to]);
            moved.push(ScanMove {
                scan: p.scan.id,
                from: p.scan.role,
                to: p.to,
            });
        }
        if let Some(scan) = next.case.scans.iter_mut().find(|s| s.id == p.scan.id) {
            scan.role = p.to;
            scan.role_source = p.source;
        }
    }
    next.case.scans.sort_by_key(|s| s.role);
    let module_cleared = if touched.is_empty() {
        Vec::new()
    } else {
        roles_changed(&mut next, &touched, &before)
            .map_err(|e| CommandError::Invalid(e.to_string()))?
    };

    let scans: Vec<Value> = state
        .case
        .scans
        .iter()
        .map(|scan| {
            let placed = placements.iter().find(|p| p.scan.id == scan.id);
            json!({
                "scan_id": scan.id,
                "source_name": scan.source_name,
                "outcome": placed.map_or(Outcome::Unchanged, |p| p.outcome),
                "from": scan.role,
                "to": placed.map_or(scan.role, |p| p.to),
                "role_source": placed.map_or(scan.role_source, |p| p.source),
                "detection": placed.and_then(|p| p.detection),
            })
        })
        .collect();
    let out = json!({
        "scans": scans,
        "moved": moved,
        "warnings": warnings,
        "world_frame_reset": !touched.is_empty(),
        "module_cleared": module_cleared,
    });
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use dc_case::ModuleState;
    use dc_command::UndoStack;
    use dca_algo::anatomy::Arch;
    use dca_algo::demo;
    use dca_model::{AlignersState, CLEARED_IDS, MODULE_ID};
    use glam::Vec3;

    const LOWER: Arch = Arch::Lower;
    const UPPER: Arch = Arch::Upper;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn import(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        exec(IMPORT_SCANS, state, params)
    }

    fn detect_step(state: &CaseState) -> Result<CommandOutput, CommandError> {
        exec(DETECT_ARCH, state, Value::Null)
    }

    /// A directory of test files, removed at the end of the test.
    struct Files(PathBuf);

    impl Files {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("dca-import-{}-{test}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        /// Writes `mesh` as the STL `name`; returns its path.
        fn stl(&self, name: &str, mesh: &TriMesh) -> String {
            let path = self.0.join(name);
            dc_mesh::io::write(&path, mesh, dc_mesh::MeshFormat::Stl).unwrap();
            path.display().to_string()
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn demo_mesh(arch: Arch, seed: u64) -> TriMesh {
        let params = demo::Params {
            arch,
            seed,
            ..demo::Params::default()
        };
        let mut mesh = demo::run(&params).mesh;
        mesh.vertex_labels = None;
        mesh
    }

    /// A flat strip with `faces` triangles.
    fn strip(faces: usize) -> TriMesh {
        let v = (0..faces + 2)
            .map(|i| Vec3::new((i / 2) as f32 * 30.0 / faces as f32, (i % 2) as f32, 0.0))
            .collect();
        let t = (0..faces as u32).map(|i| [i, i + 1, i + 2]).collect();
        TriMesh::new(v, t)
    }

    /// Adds a scan to the case; its id ends with `k`.
    fn add(state: &mut CaseState, mesh: TriMesh, role: &str, source: &str, k: u32) -> Scan {
        let blob = state.blobs.insert(mesh);
        let scan: Scan = serde_json::from_value(json!({
            "id": format!("a1000000-0000-4000-8000-{k:012}"),
            "role": role,
            "role_source": source,
            "source_name": format!("scan-{k}.stl"),
            "source_units": "mm",
            "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
            "mesh": blob,
        }))
        .unwrap();
        state.case.scans.push(scan.clone());
        state.case.scans.sort_by_key(|s| s.role);
        scan
    }

    fn role_of(state: &CaseState, scan: &Scan) -> (ScanRole, RoleSource) {
        let s = state.case.scan_by_id(scan.id).unwrap();
        (s.role, s.role_source)
    }

    fn key_list(out: &Value, field: &str) -> Vec<String> {
        out["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w[field].as_str().unwrap().to_owned())
            .collect()
    }

    fn codes(out: &Value) -> Vec<String> {
        key_list(out, "code")
    }

    fn keys(out: &Value) -> Vec<String> {
        key_list(out, "key")
    }

    fn message(e: CommandError) -> String {
        match e {
            CommandError::Invalid(m) | CommandError::Failed(m) => m,
            other => panic!("{other}"),
        }
    }

    #[test]
    fn the_import_leaf_is_ready_and_the_arch_step_is_not_a_leaf() {
        assert!(crate::commands::is_registered(IMPORT_SCANS));
        let leaf = crate::tree::find(IMPORT_SCANS).unwrap();
        assert_eq!(leaf.status(), dc_module_api::MenuStatus::Ready);
        assert!(dc_module_api::is_import_scans_command(IMPORT_SCANS));
        assert!(crate::commands::is_registered(DETECT_ARCH));
        assert!(crate::tree::find(DETECT_ARCH).is_none());
    }

    #[test]
    fn the_parameters_are_the_ones_of_case_import() {
        let spec = crate::commands::find(IMPORT_SCANS).unwrap();
        let schema = spec.schema();
        for p in ["path", "files", "role"] {
            assert!(schema["properties"][p].is_object(), "{schema}");
        }
        assert_eq!(schema["oneOf"][0]["required"][0], "path", "{schema}");
        assert_eq!(schema["oneOf"][1]["required"][0], "files", "{schema}");
        let state = CaseState::default();
        for p in [
            Value::Null,
            json!({}),
            json!({ "files": [] }),
            json!({ "path": "a.stl", "files": [{ "path": "b.stl" }] }),
            json!({ "path": "a.stl", "nope": 1 }),
            json!({ "path": "a.stl", "role": "palate" }),
        ] {
            assert!(
                matches!(import(&state, p.clone()), Err(CommandError::Invalid(_))),
                "{p}"
            );
        }
        // nothing imported: an error with the reason, and no state
        let e = import(&state, json!({ "path": "/nonexistent/a.stl" })).unwrap_err();
        assert!(matches!(e, CommandError::Failed(_)), "{e}");
    }

    #[test]
    fn the_output_is_a_superset_of_case_import() {
        let files = Files::new("output");
        let path = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        for key in [
            "scan_id",
            "role",
            "role_source",
            "source_name",
            "source_units",
            "mesh",
            "vertices",
            "faces",
            "replaced",
            "imported",
            "warnings",
            "failed",
            "moved",
            "cleanup",
            "module_cleared",
        ] {
            assert!(out.get(key).is_some(), "{key} missing: {out}");
        }
        assert_eq!(out["role"], "lower", "{out}");
        assert_eq!(out["role_source"], "detected", "{out}");
        assert_eq!(out["replaced"], false);
        assert_eq!(out["imported"].as_array().unwrap().len(), 1);
        assert_eq!(next.case.scans[0].role_source, RoleSource::Detected);
        // what this command adds
        assert_eq!(out["detections"][0]["arch"], "lower", "{out}");
        assert_eq!(out["detections"][0]["source"], "curve_of_wilson", "{out}");
        assert_eq!(out["detections"][0]["source_name"], "scan_a.stl", "{out}");
    }

    #[test]
    fn two_files_of_one_arch_give_the_winner_and_the_opposite_with_a_conflict() {
        let files = Files::new("conflict");
        // turned over, the second one is recognised less surely (0.8, not 0.9)
        let mut weak = demo_mesh(UPPER, 2);
        for p in &mut weak.positions {
            *p = Vec3::new(p.x, -p.y, -p.z);
        }
        let weak = files.stl("scan_a.stl", &weak);
        let sure = files.stl("scan_b.stl", &demo_mesh(UPPER, 1));
        let params = json!({ "files": [{ "path": weak }, { "path": sure }] });
        let (next, out) = import(&CaseState::default(), params).unwrap();
        let imported = out["imported"].as_array().unwrap();
        let of = |name: &str| imported.iter().find(|s| s["source_name"] == name).unwrap();
        assert_eq!(of("scan_b.stl")["role"], "upper", "{out}");
        assert_eq!(of("scan_b.stl")["role_source"], "detected", "{out}");
        assert_eq!(of("scan_a.stl")["role"], "lower", "{out}");
        assert_eq!(of("scan_a.stl")["role_source"], "provisional", "{out}");
        assert_eq!(codes(&out), ["arch_conflict"], "{out}");
        // both detections are in the output, with what the geometry really said
        assert_eq!(out["detections"][0]["arch"], "upper", "{out}");
        assert_eq!(out["detections"][1]["arch"], "upper", "{out}");

        // the arch step does not settle it: the slot is taken by a surer scan
        let (again, out) = detect_step(&next).unwrap();
        assert_eq!(codes(&out), ["arch_conflict"], "{out}");
        assert_eq!(again.case.scans, next.case.scans);
    }

    #[test]
    fn an_explicit_role_overrules_the_detection() {
        let files = Files::new("explicit");
        let path = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let params = json!({ "path": path, "role": "upper" });
        let (next, out) = import(&CaseState::default(), params).unwrap();
        assert_eq!(out["role"], "upper", "{out}");
        assert_eq!(out["role_source"], "explicit", "{out}");
        assert_eq!(out["detections"], json!([]), "{out}");
        assert_eq!(next.case.scans[0].role_source, RoleSource::Explicit);
        // a role of the file overrules the default, and the name
        let named = files.stl("upper.stl", &demo_mesh(LOWER, 1));
        let params = json!({ "role": "upper", "files": [{ "path": named, "role": "lower" }] });
        let (_, out) = import(&CaseState::default(), params).unwrap();
        assert_eq!(out["role"], "lower", "{out}");
        // auto goes to the name, without looking at the geometry
        let params = json!({ "path": named, "role": "auto" });
        let (_, out) = import(&CaseState::default(), params).unwrap();
        assert_eq!(out["role"], "upper", "{out}");
        assert_eq!(out["role_source"], "file_name", "{out}");
        assert_eq!(out["detections"], json!([]), "{out}");
    }

    #[test]
    fn a_bite_scan_is_not_touched_by_the_detection() {
        let files = Files::new("bite");
        let chosen = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let named = files.stl("bite.stl", &demo_mesh(UPPER, 1));
        let params = json!({ "files": [{ "path": chosen, "role": "bite" }, { "path": named }] });
        let (next, out) = import(&CaseState::default(), params).unwrap();
        assert_eq!(out["detections"], json!([]), "{out}");
        assert!(next.case.scans.iter().all(|s| s.role == ScanRole::Bite));

        // the arch step leaves it alone, and it does not count as an arch scan
        let e = detect_step(&next).unwrap_err();
        assert!(matches!(e, CommandError::Invalid(_)), "{e}");
        let mut state = next.clone();
        let arch = add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        let (done, out) = detect_step(&state).unwrap();
        assert_eq!(role_of(&done, &arch).0, ScanRole::Lower);
        let bites = done.case.scans_of(ScanRole::Bite).count();
        assert_eq!(bites, 2);
        let unchanged = |s: &&Value| s["outcome"] == "unchanged";
        let reports = out["scans"].as_array().unwrap();
        assert_eq!(reports.iter().filter(unchanged).count(), 2, "{out}");
    }

    #[test]
    fn a_small_mesh_stays_provisional_and_fails_at_the_arch_step() {
        let files = Files::new("small");
        let path = files.stl("scan_a.stl", &strip(999));
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(out["role_source"], "provisional", "{out}");
        assert_eq!(out["detections"], json!([]), "{out}");
        let codes = codes(&out);
        assert!(codes.iter().any(|c| c == "low_polygons"), "{out}");
        assert!(
            keys(&out).iter().all(|k| !k.starts_with("aligners-")),
            "{out}"
        );

        let m = message(detect_step(&next).unwrap_err());
        assert!(m.contains("scan_a.stl") && m.contains("999"), "{m}");
        assert!(!m.contains("aligners-"), "{m}");
        // the first step of the pipeline, by name
        let m = message(exec("aligners.auto", &next, Value::Null).unwrap_err());
        assert!(m.contains("Stopped at Arch recognition"), "{m}");
        assert!(!m.contains("aligners."), "{m}");

        // the rule holds whatever the source of the role, as in the plugin
        let mut state = CaseState::default();
        add(&mut state, strip(10), "lower", "explicit", 1);
        assert!(detect_step(&state).is_err());
        // a thousand faces are enough to pass it
        let mut state = CaseState::default();
        add(&mut state, strip(1000), "lower", "explicit", 1);
        assert!(detect_step(&state).is_ok());
    }

    #[test]
    fn an_open_arch_scan_is_closed_with_a_base_and_keeps_the_scan_as_read() {
        let files = Files::new("open");
        // the demo arch opened under the gum: the faces of its lowest 4 mm
        // (its occlusal axis is +Z) go
        let closed = demo_mesh(LOWER, 1);
        let low = closed
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f32::INFINITY, f32::min)
            + 4.0;
        let mut open = closed.clone();
        open.tris
            .retain(|t| !t.iter().all(|&i| closed.positions[i as usize].z < low));
        let path = files.stl("scan_a.stl", &open);
        let read = dc_case::import::prepare(std::path::Path::new(&path), &JobCtx::detached())
            .unwrap()
            .mesh;
        assert!(!read.validate().is_watertight());
        let params = json!({ "path": path, "role": "lower" });
        let (next, out) = import(&CaseState::default(), params).unwrap();
        let base = &out["bases"][0];
        assert_eq!(base["already_closed"], false, "{out}");
        assert_eq!(base["watertight"], true, "{out}");
        // the closed solid is the scan of the case, and the report says so
        let scan = next.case.scan(ScanRole::Lower).unwrap();
        assert_eq!(out["mesh"], json!(scan.mesh), "{out}");
        let mesh = next.blobs.require(&scan.mesh).unwrap();
        assert!(mesh.validate().is_watertight());
        assert_eq!(out["vertices"], mesh.vertex_count(), "{out}");
        // the scan as it was read is its start, exactly
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let record = data.scan_base(&scan.mesh).unwrap();
        assert_eq!(record.closed_by, dca_model::ClosedBy::Import);
        let back = dca_algo::scan_base::open_part(mesh, &record).unwrap();
        assert_eq!(back, *read);
        assert_eq!(next.blobs.len(), 1, "the scan as read is not kept twice");

        // a closed scan stays as it is, with no record
        let path = files.stl("scan_b.stl", &demo_mesh(LOWER, 2));
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(out["bases"][0]["already_closed"], true, "{out}");
        let data = next.module_data::<AlignersState>(MODULE_ID);
        assert!(data.is_none_or(|d| d.scan_bases.is_empty()));
    }

    #[test]
    fn a_failed_detection_stays_provisional_with_a_warning() {
        let files = Files::new("failed");
        // enough faces, but flat: there is no arch to find
        let path = files.stl("scan_a.stl", &strip(1200));
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(out["role_source"], "provisional", "{out}");
        assert_eq!(out["role"], "lower", "{out}");
        let found = keys(&out);
        assert!(
            found.iter().any(|k| k == "case-import-warn-provisional"),
            "{out}"
        );
        assert!(
            found
                .iter()
                .any(|k| k == "aligners-arch-warn-not-recognised"),
            "{out}"
        );

        let (done, out) = detect_step(&next).unwrap();
        assert_eq!(done.case.scans, next.case.scans);
        assert_eq!(out["scans"][0]["outcome"], "not_recognised", "{out}");
        assert_eq!(keys(&out), ["aligners-arch-warn-not-recognised"], "{out}");
    }

    #[test]
    fn the_arch_step_swaps_two_provisional_scans_and_clears_their_results() {
        let mut state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        state.case.scans.clear();
        let lower = add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        let upper = add(&mut state, demo_mesh(UPPER, 1), "lower", "provisional", 2);
        state.case.world_frame.from_case.t = glam::DVec3::X;
        let (next, out) = detect_step(&state).unwrap();
        assert_eq!(
            role_of(&next, &lower),
            (ScanRole::Lower, RoleSource::Detected)
        );
        assert_eq!(
            role_of(&next, &upper),
            (ScanRole::Upper, RoleSource::Detected)
        );
        assert_eq!(out["moved"].as_array().unwrap().len(), 2, "{out}");
        assert_eq!(out["world_frame_reset"], true, "{out}");
        assert_eq!(next.case.world_frame, Default::default());
        let cleared = out["module_cleared"].as_array().unwrap();
        assert!(cleared.iter().any(|c| c == "seeds"), "{out}");
        assert!(
            next.module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .seeds
                .is_empty()
        );
        assert_eq!(out["scans"][0]["outcome"], "resolved", "{out}");
        assert_eq!(
            out["scans"][0]["detection"]["source"], "curve_of_wilson",
            "{out}"
        );
        // a second run has nothing to do
        let (again, out) = detect_step(&next).unwrap();
        assert_eq!(again.case.scans, next.case.scans);
        assert_eq!(out["moved"], json!([]), "{out}");
        assert_eq!(out["scans"][1]["outcome"], "unchanged", "{out}");
    }

    #[test]
    fn the_arch_step_confirms_or_moves_a_provisional_scan() {
        // already on its arch: confirmed, and the results stay
        let mut state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        state.case.scans[0].role_source = RoleSource::Provisional;
        state.case.world_frame.from_case.t = glam::DVec3::X;
        let (next, out) = detect_step(&state).unwrap();
        assert_eq!(next.case.scans[0].role, ScanRole::Lower);
        assert_eq!(next.case.scans[0].role_source, RoleSource::Detected);
        assert_eq!(out["moved"], json!([]), "{out}");
        assert_eq!(out["module_cleared"], json!([]), "{out}");
        assert_eq!(out["world_frame_reset"], false, "{out}");
        assert_eq!(next.case.world_frame, state.case.world_frame);

        // on the wrong arch with the other one free: moved
        let mut state = CaseState::default();
        let scan = add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        let (next, out) = detect_step(&state).unwrap();
        assert_eq!(
            role_of(&next, &scan),
            (ScanRole::Lower, RoleSource::Detected)
        );
        assert_eq!(out["scans"][0]["from"], "upper", "{out}");
        assert_eq!(out["scans"][0]["to"], "lower", "{out}");
    }

    #[test]
    fn the_arch_step_leaves_the_surer_roles_alone() {
        for source in ["explicit", "file_name", "detected"] {
            // the mesh does not fit the role: nobody asked
            let mut state = CaseState::default();
            add(&mut state, demo_mesh(LOWER, 1), "upper", source, 1);
            let (next, out) = detect_step(&state).unwrap();
            assert_eq!(next.case.scans, state.case.scans, "{source}");
            assert_eq!(out["scans"][0]["outcome"], "unchanged", "{out}");
            assert_eq!(out["scans"][0]["role_source"], source, "{out}");
            assert_eq!(out["scans"][0]["detection"], Value::Null, "{out}");

            // and they keep the arch against a provisional scan of the same one
            let provisional = add(&mut state, demo_mesh(UPPER, 1), "lower", "provisional", 2);
            let (next, out) = detect_step(&state).unwrap();
            assert_eq!(
                role_of(&next, &provisional),
                (ScanRole::Lower, RoleSource::Provisional)
            );
            assert_eq!(codes(&out), ["arch_conflict"], "{out}");
            assert_eq!(out["scans"][1]["outcome"], "conflict", "{out}");
        }
    }

    #[test]
    fn replacing_an_arch_clears_it_through_the_command_and_undo_restores() {
        let state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        let files = Files::new("replace");
        let path = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let spec = crate::commands::find(IMPORT_SCANS).unwrap();
        let params = json!({ "path": path, "role": "lower" });
        let (next, out) = spec.execute(&state, &params, &JobCtx::detached()).unwrap();
        assert_eq!(out["replaced"], true, "{out}");
        assert!(codes(&out).iter().any(|c| c == "replaced"), "{out}");
        let cleared = out["module_cleared"].as_array().unwrap();
        for id in ["seeds", "labels"] {
            assert!(cleared.iter().any(|c| c == id), "{id}: {out}");
        }
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(data.seeds.is_empty() && data.labels.is_empty());

        // one change on the undo stack, and it brings everything back
        let mut undo = UndoStack::new(Arc::new(state.clone()), UndoStack::DEFAULT_BUDGET);
        undo.push(Arc::new(next), spec.label_key);
        assert_eq!(undo.undo_depth(), 1);
        undo.undo().unwrap();
        let back = undo.current();
        assert_eq!(back.case, state.case);
        assert_eq!(
            back.module_data::<AlignersState>(MODULE_ID),
            state.module_data::<AlignersState>(MODULE_ID)
        );
    }

    #[test]
    fn the_arch_step_is_one_change_undo_restores_the_roles() {
        let mut state = CaseState::default();
        add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        add(&mut state, demo_mesh(UPPER, 1), "lower", "provisional", 2);
        let spec = crate::commands::find(DETECT_ARCH).unwrap();
        let (next, _) = spec
            .execute(&state, &Value::Null, &JobCtx::detached())
            .unwrap();
        assert_ne!(next.case.scans, state.case.scans);
        let mut undo = UndoStack::new(Arc::new(state.clone()), UndoStack::DEFAULT_BUDGET);
        undo.push(Arc::new(next), spec.label_key);
        undo.undo().unwrap();
        assert_eq!(undo.current().case.scans, state.case.scans);
    }

    /// The demo arches with the bite registration flag set.
    fn registered_state() -> CaseState {
        let mut state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.bite_registered = true;
        state.set_module(MODULE_ID, ModuleState::new(data));
        state
    }

    fn flag(state: &CaseState) -> bool {
        state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .bite_registered
    }

    fn cleared_ids(out: &Value) -> Vec<String> {
        let ids = out["module_cleared"].as_array().unwrap();
        ids.iter().map(|v| v.as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn replacing_an_arch_after_a_bite_registration_reports_it_as_cleared() {
        let state = registered_state();
        let files = Files::new("registered");
        let bite = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let id = "bite_registered";

        // a new bite scan leaves the arches as they are: the registration holds
        let params = json!({ "path": bite, "role": "bite" });
        let (with_bite, out) = import(&state, params).unwrap();
        assert!(!cleared_ids(&out).iter().any(|c| c == id), "{out}");
        assert!(flag(&with_bite));

        // an arch replaced: reported as a cleared id, no warning of its own
        let params = json!({ "path": bite, "role": "lower" });
        let (next, out) = import(&with_bite, params).unwrap();
        assert!(cleared_ids(&out).iter().any(|c| c == id), "{out}");
        assert!(
            keys(&out).iter().all(|k| !k.contains("bite-not-reset")),
            "{out}"
        );
        assert!(!flag(&next));
        // the text of the id is translated, and says what to do
        let text = cleared_list(&[id]);
        assert!(!text.contains('_') && text.contains("redone"), "{text}");
        // said once: the next replacement has nothing new to say
        let (_, out) = import(&next, json!({ "path": bite, "role": "lower" })).unwrap();
        assert!(!cleared_ids(&out).iter().any(|c| c == id), "{out}");

        // nothing replaced, nothing reported, with or without a registration
        let mut empty = CaseState::default();
        empty.set_module(
            MODULE_ID,
            ModuleState::new(AlignersState {
                bite_registered: true,
                ..AlignersState::default()
            }),
        );
        let (_, out) = import(&empty, json!({ "path": bite, "role": "upper" })).unwrap();
        assert!(!cleared_ids(&out).iter().any(|c| c == id), "{out}");

        // the arch step reports it when it moves a scan
        let mut moved = empty.clone();
        add(&mut moved, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        let (done, out) = detect_step(&moved).unwrap();
        assert!(cleared_ids(&out).iter().any(|c| c == id), "{out}");
        assert!(!flag(&done));
    }

    #[test]
    fn case_import_and_set_role_report_the_registration_too() {
        use dc_case::import::{ImportFile, ImportRequest, import_files, roles_changed};

        // the shared pipeline behind case.import: no module command involved
        let mut state = registered_state();
        let files = Files::new("shared");
        let path = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let request = ImportRequest {
            files: vec![ImportFile {
                path: path.into(),
                role: Some(ScanRole::Lower),
            }],
        };
        let report = import_files(&mut state, &request, None, &JobCtx::detached());
        assert!(report.module_cleared.contains(&"bite_registered"));
        assert!(!flag(&state));

        // what case.set-role ends with: the arches swapped
        let mut state = registered_state();
        let before = state.referenced_blobs();
        for scan in &mut state.case.scans {
            scan.role = opposite(scan.role);
        }
        state.case.scans.sort_by_key(|s| s.role);
        let touched = [ScanRole::Lower, ScanRole::Upper].into();
        let cleared = roles_changed(&mut state, &touched, &before).unwrap();
        assert!(cleared.contains(&"bite_registered"), "{cleared:?}");
        assert!(!flag(&state));
        // a bite scan moving does not touch the registration
        let mut state = registered_state();
        let before = state.referenced_blobs();
        let touched = [ScanRole::Bite].into();
        let cleared = roles_changed(&mut state, &touched, &before).unwrap();
        assert!(cleared.is_empty(), "{cleared:?}");
        assert!(flag(&state));
    }

    fn detection(arch: Arch, confidence: f64, source: ArchSource) -> ArchDetection {
        ArchDetection {
            arch,
            confidence,
            source,
            palate_coverage: None,
            occlusal_z: None,
            molar_cusps_mm: None,
            cost_upper: None,
            cost_lower: None,
        }
    }

    const CHECKED: [ArchSource; 3] = [
        ArchSource::WilsonOverPalate,
        ArchSource::PalateDisagreesWithMolars,
        ArchSource::ShapeAndWidths,
    ];
    const SURE: [ArchSource; 5] = [
        ArchSource::FileName,
        ArchSource::ScannerOrientation,
        ArchSource::Palate,
        ArchSource::CurveOfWilson,
        ArchSource::WilsonAndWidths,
    ];

    #[test]
    fn the_cues_the_plugin_marks_to_be_checked_give_detected_unsure() {
        for source in CHECKED {
            assert_eq!(detected_source(source), RoleSource::DetectedUnsure);
        }
        for source in SURE {
            assert_eq!(detected_source(source), RoleSource::Detected);
        }
        let w = width_check("scan_a.stl", ScanRole::Lower);
        assert_eq!(w.code, WarningCode::Provisional);
        assert!(w.message().contains("scan_a.stl"));
        assert!(!w.message().contains("aligners-"));
    }

    #[test]
    fn the_arch_step_stores_detected_unsure_and_warns_for_each_cue_to_check() {
        let mut state = CaseState::default();
        add(&mut state, strip(1000), "lower", "provisional", 1);
        for source in CHECKED.into_iter().chain(SURE) {
            let mut warnings = Vec::new();
            let placed = place_provisional(
                vec![(&state.case.scans[0], detection(Arch::Upper, 0.3, source))],
                Vec::new(),
                BTreeSet::new(),
                &mut warnings,
            );
            assert_eq!(placed[0].to, ScanRole::Upper);
            assert_eq!(placed[0].outcome, Outcome::Resolved);
            if CHECKED.contains(&source) {
                assert_eq!(placed[0].source, RoleSource::DetectedUnsure, "{source:?}");
                assert_eq!(warnings.len(), 1, "{source:?}");
                assert_eq!(warnings[0].key, "aligners-arch-warn-to-be-checked");
            } else {
                assert_eq!(placed[0].source, RoleSource::Detected, "{source:?}");
                assert!(warnings.is_empty(), "{source:?}");
            }
        }
    }

    #[test]
    fn the_arch_command_keeps_a_detected_unsure_scan_settled() {
        let mut state = CaseState::default();
        let scan = add(
            &mut state,
            demo_mesh(UPPER, 1),
            "lower",
            "detected_unsure",
            1,
        );
        let other = add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 2);
        let (next, out) = detect_step(&state).unwrap();
        // not an arch to settle: it is reported unchanged, and the provisional
        // scan, which also looks like a lower arch, cannot take its place
        assert_eq!(
            role_of(&next, &scan),
            (ScanRole::Lower, RoleSource::DetectedUnsure)
        );
        let report = |name: &str| {
            let scans = out["scans"].as_array().unwrap();
            scans.iter().find(|s| s["source_name"] == name).unwrap()
        };
        assert_eq!(report("scan-1.stl")["outcome"], "unchanged", "{out}");
        assert_eq!(
            report("scan-1.stl")["role_source"],
            "detected_unsure",
            "{out}"
        );
        assert_eq!(report("scan-2.stl")["outcome"], "conflict", "{out}");
        assert_eq!(
            role_of(&next, &other),
            (ScanRole::Upper, RoleSource::Provisional)
        );
        // the guards let it pass: only a provisional scan is refused
        let held = next.case.scan_by_id(scan.id).unwrap();
        assert!(require_settled(held).is_ok());
        // while the provisional scan next to it is refused
        assert!(require_settled(next.case.scan_by_id(other.id).unwrap()).is_err());
    }

    /// The lower demo arch with a closed base inside the arch, as a model
    /// has: the buccal molar cusps are highest over a closed region, which
    /// the plugin flags as "to be checked" ([`ArchSource::WilsonOverPalate`]).
    fn lower_on_a_model_base() -> TriMesh {
        let mut mesh = demo_mesh(LOWER, 1);
        let bb = mesh.aabb().unwrap();
        let (lo, hi) = (bb.min, bb.max);
        let n = 40u32;
        let first = mesh.positions.len() as u32;
        let z = lo.z + 0.2 * (hi.z - lo.z);
        for i in 0..n {
            for j in 0..n {
                let x = lo.x + (hi.x - lo.x) * (0.15 + 0.7 * i as f32 / (n - 1) as f32);
                let y = lo.y + (hi.y - lo.y) * (0.05 + 0.9 * j as f32 / (n - 1) as f32);
                mesh.positions.push(Vec3::new(x, y, z));
            }
        }
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                let a = first + i * n + j;
                mesh.tris.push([a, a + 1, a + n]);
                mesh.tris.push([a + 1, a + n + 1, a + n]);
            }
        }
        mesh
    }

    #[test]
    fn a_cue_to_check_is_detected_unsure_with_a_warning_and_stays_settled() {
        let files = Files::new("unsure-real");
        let path = files.stl("scan_a.stl", &lower_on_a_model_base());
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(
            out["detections"][0]["source"], "wilson_over_palate",
            "{out}"
        );
        assert_eq!(out["role"], "lower", "{out}");
        assert_eq!(out["role_source"], "detected_unsure", "{out}");
        assert_eq!(
            out["imported"][0]["role_source"], "detected_unsure",
            "{out}"
        );
        assert_eq!(next.case.scans[0].role_source, RoleSource::DetectedUnsure);
        assert_eq!(keys(&out), ["aligners-arch-warn-to-be-checked"], "{out}");
        assert_eq!(codes(&out), ["provisional"], "{out}");
        // the guards let it pass: the plugin warns and does not stop
        assert!(require_settled(&next.case.scans[0]).is_ok());

        // a later unhinted file that wants the same arch does not displace it
        let other = files.stl("scan_b.stl", &demo_mesh(LOWER, 2));
        let (after, out) = import(&next, json!({ "path": other })).unwrap();
        assert_eq!(
            role_of(&after, &next.case.scans[0]),
            (ScanRole::Lower, RoleSource::DetectedUnsure),
            "{out}"
        );
        assert_eq!(out["role"], "upper", "{out}");
        assert_eq!(out["role_source"], "provisional", "{out}");
        assert_eq!(out["moved"], json!([]), "{out}");
        assert_eq!(codes(&out), ["arch_conflict"], "{out}");

        // the arch step is the second call site: same source, same warning
        let mut state = CaseState::default();
        let mesh = lower_on_a_model_base();
        let scan = add(&mut state, mesh, "upper", "provisional", 1);
        let (done, out) = detect_step(&state).unwrap();
        assert_eq!(
            role_of(&done, &scan),
            (ScanRole::Lower, RoleSource::DetectedUnsure)
        );
        assert_eq!(out["scans"][0]["role_source"], "detected_unsure", "{out}");
        assert_eq!(out["scans"][0]["outcome"], "resolved", "{out}");
        assert_eq!(keys(&out), ["aligners-arch-warn-to-be-checked"], "{out}");
    }

    #[test]
    fn the_sure_cues_of_the_real_scans_stay_detected_without_a_warning() {
        let files = Files::new("sure");
        let path = files.stl("scan_a.stl", &demo_mesh(UPPER, 1));
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(out["detections"][0]["source"], "curve_of_wilson", "{out}");
        assert_eq!(out["role_source"], "detected", "{out}");
        assert_eq!(next.case.scans[0].role_source, RoleSource::Detected);
        assert!(keys(&out).is_empty(), "{out}");
        // the scanner orientation (teeth pointing down) is a sure cue too
        let mut turned = demo_mesh(UPPER, 2);
        for p in &mut turned.positions {
            *p = Vec3::new(p.x, -p.y, -p.z);
        }
        let path = files.stl("scan_b.stl", &turned);
        let (_, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(
            out["detections"][0]["source"], "scanner_orientation",
            "{out}"
        );
        assert_eq!(out["role_source"], "detected", "{out}");
        assert!(keys(&out).is_empty(), "{out}");
    }

    #[test]
    fn a_failed_width_comparison_is_no_recognition() {
        let inf = f64::INFINITY;
        let widths = |cu, cl, confidence| ArchDetection {
            cost_upper: Some(cu),
            cost_lower: Some(cl),
            ..detection(Arch::Upper, confidence, ArchSource::ShapeAndWidths)
        };
        // the proposal failed for both arches: NaN difference, an upper arch at
        // 0.6 in the plugin's arithmetic, but nothing was recognised
        for d in [widths(inf, inf, 0.6), widths(f64::NAN, inf, 0.6)] {
            let reason = recognised(d).unwrap_err();
            assert!(reason.contains("widths"), "{reason}");
        }
        // no confidence at all: the same
        assert!(recognised(widths(1e18, 1e18, 0.0)).is_err());
        // one hypothesis failed, the other has a cost: that is an answer
        assert!(recognised(widths(3.0, inf, 0.6)).is_ok());
        assert!(recognised(widths(3.0, 5.0, 0.4)).is_ok());
        // not the widths that decided: costs are not looked at
        let by_palate = detection(Arch::Upper, 0.9, ArchSource::Palate);
        assert!(recognised(by_palate).is_ok());
    }

    #[test]
    fn a_mesh_with_no_cue_at_all_stays_provisional() {
        // flat, and turned so that no other cue answers: both hypotheses cost
        // the same (not reachable with infinite costs on a real mesh: once the
        // arch data exist, `propose` does not fail)
        let mut mesh = demo_mesh(UPPER, 1);
        for p in &mut mesh.positions {
            p.z *= -0.7;
        }
        let files = Files::new("nocue");
        let path = files.stl("scan_a.stl", &mesh);
        let (next, out) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(out["role_source"], "provisional", "{out}");
        assert_eq!(out["detections"], json!([]), "{out}");
        assert!(
            keys(&out)
                .iter()
                .any(|k| k == "aligners-arch-warn-not-recognised"),
            "{out}"
        );
        assert_eq!(next.case.scans[0].role_source, RoleSource::Provisional);
    }

    #[test]
    fn the_pipeline_stops_on_a_provisional_scan_with_a_clear_message() {
        let files = Files::new("stops");
        let path = files.stl("scan_a.stl", &strip(1200));
        let (next, _) = import(&CaseState::default(), json!({ "path": path })).unwrap();
        assert_eq!(next.case.scans[0].role_source, RoleSource::Provisional);
        let m = message(exec("aligners.auto", &next, Value::Null).unwrap_err());
        assert!(m.contains("Stopped at Number the teeth"), "{m}");
        assert!(m.contains("scan_a.stl"), "{m}");
        assert!(m.contains("Import scans panel"), "{m}");
        assert!(m.contains("press Confirm"), "{m}");
        assert!(!m.contains("aligners") && !m.contains("case."), "{m}");
        // the other gates of the pipeline say the same
        for id in [
            "aligners.preparation.orientation.automatic",
            "aligners.preparation.numbering.automatic",
            "aligners.frame",
        ] {
            let m = message(exec(id, &next, json!({})).unwrap_err());
            assert!(m.contains("Import scans panel"), "{id}: {m}");
        }
    }

    #[test]
    fn repeated_reasons_of_the_failed_files_are_said_once() {
        use dc_case::OpaqueModuleData;
        let mut state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        state = exec("aligners.demo", &state, json!({"arch": "upper"}))
            .unwrap()
            .0;
        state.set_module(
            "future",
            ModuleState::new(OpaqueModuleData {
                schema_version: 9,
                value: json!({}),
            }),
        );
        let files = Files::new("dedup");
        let a = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let b = files.stl("scan_b.stl", &demo_mesh(UPPER, 1));
        let params = json!({"files": [{"path": a, "role": "lower"}, {"path": b, "role": "upper"}]});
        let m = message(import(&state, params).unwrap_err());
        // two files fail with the same reason, which a module of the project
        // caused: one sentence, not two
        let sentence = m.trim_start_matches("No scan was imported. ");
        let first = sentence.split(". ").next().unwrap();
        assert_eq!(m.matches(first).count(), 1, "{m}");
    }

    #[test]
    fn a_scan_not_recognised_makes_way_for_a_recognised_one() {
        let mut state = CaseState::default();
        let lost = add(&mut state, strip(1000), "lower", "provisional", 1);
        let found = add(&mut state, strip(1000), "upper", "provisional", 2);
        let detection = ArchDetection {
            arch: Arch::Lower,
            confidence: 0.9,
            source: ArchSource::CurveOfWilson,
            palate_coverage: None,
            occlusal_z: None,
            molar_cusps_mm: None,
            cost_upper: None,
            cost_lower: None,
        };
        let by_id = |id| state.case.scan_by_id(id).unwrap();
        let mut warnings = Vec::new();
        let placed = place_provisional(
            vec![(by_id(found.id), detection)],
            vec![by_id(lost.id)],
            BTreeSet::new(),
            &mut warnings,
        );
        assert_eq!(placed[0].to, ScanRole::Lower);
        assert_eq!(placed[1].to, ScanRole::Upper);
        assert_eq!(placed[1].source, RoleSource::Provisional);
        assert_eq!(placed[1].outcome, Outcome::NotRecognised);
        assert!(warnings.is_empty());
    }

    #[test]
    fn every_cleared_id_has_a_translated_name() {
        for id in CLEARED_IDS {
            let name = cleared_list(&[id]);
            let key = format!("aligners-cleared-{}", id.replace('_', "-"));
            assert!(dc_i18n::has(&key), "{key}");
            assert!(!name.contains('_'), "{id}");
        }
        let list = cleared_list(&["seeds", "tooth_frames"]);
        assert!(
            list.contains(", ") && !list.contains("tooth_frames"),
            "{list}"
        );
        // an id of another module stays readable
        assert_eq!(cleared_list(&["other_thing"]), "other_thing");
        assert_eq!(cleared_list(&[]), "");
    }
}
