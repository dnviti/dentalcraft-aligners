//! Scan import with recognition of the arch, and the arch step of the
//! pipeline.
//!
//! * `aligners.case.import-scans` is `case.import` with a resolver: a file
//!   with neither an explicit role nor a name hint gets its arch from the
//!   geometry ([`autoseed::detect_arch`]). Same parameters and same output as
//!   `case.import`, plus `detections`, because MCP and the CLI prefer it over
//!   `case.import` ([`dc_module_api::IMPORT_SCANS_SUFFIX`]). The app calls
//!   [`import_scans`] directly.
//! * `aligners.case.detect-arch` is the plugin's `arcata` step: it refuses an
//!   arch scan with too few faces (`case_auto.auto_steps`) and resolves the
//!   scans still [`RoleSource::Provisional`]. `aligners.auto` runs it first.
//!   It is not a menu leaf.
//!
//! The roles are never decided here for a scan with a surer source: explicit,
//! file name and detected roles stay as they are.
//!
//! Detection runs on the thread of the job and needs no decimation: about
//! 0.3 s in a release build on a real scan of 855k faces.

use std::collections::BTreeSet;
use std::path::PathBuf;

use dc_case::import::{
    Detection, ImportFile, ImportReport, ImportRequest, ImportWarning, LOW_POLYGON_FACES,
    PreparedScan, ScanMove, WarningCode, import_files, roles_changed,
};
use dc_case::{CaseState, RoleSource, Scan, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::autoseed::{self, ArchDetection, ArchSource};
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
    /// segmentation, bite registration not reset.
    pub report: ImportReport,
    /// The recognised files, in request order.
    pub detections: Vec<ScanDetection>,
}

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
/// With no confidence at all the two width hypotheses cost the same and
/// [`autoseed::detect_arch`] still has to answer something: that is no
/// recognition, so it counts as a failure and the scan stays provisional.
fn detect(mesh: &TriMesh, name: &str) -> Result<ArchDetection, String> {
    let vertices: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let detection =
        autoseed::detect_arch(&vertices, &mesh.tris, Some(name)).map_err(|e| e.to_string())?;
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

/// `true` when the arch rests only on the shape and the tooth widths.
///
/// The plugin does not trust this path (`case_auto.auto_steps`, the sources
/// starting with "forma"): it treats the scan as a lower arch, then after the
/// segmentation compares the crown widths and restarts as an upper arch when
/// they fit better. That verification loop is NOT ported: it needs the
/// segmentation and the teeth stage. For now the guess of
/// [`autoseed::detect_arch`] is kept and the user is told to check it. When
/// the loop lands, this warning goes.
fn needs_width_check(source: ArchSource) -> bool {
    source == ArchSource::ShapeAndWidths
}

fn width_check(name: &str, role: ScanRole) -> ImportWarning {
    warning(
        WarningCode::Provisional,
        "aligners-arch-warn-to-be-checked",
        &[("name", name), ("role", role_id(role))],
    )
}

fn bite_registered(state: &CaseState) -> bool {
    state
        .module_data::<AlignersState>(MODULE_ID)
        .is_some_and(|d| d.bite_registered)
}

fn bite_not_reset() -> ImportWarning {
    warning(
        WarningCode::Replaced,
        "aligners-arch-warn-bite-not-reset",
        &[],
    )
}

/// Imports a batch into `state`, recognising the arch of the files with
/// neither an explicit role nor a name hint: the Aligners form of
/// [`dc_case::import::import_files`], for `create_job` and for the command.
///
/// Runs on the caller's thread, so a job can call it. A scan under
/// [`LOW_POLYGON_FACES`] faces is not analysed, and a scan that cannot be
/// recognised is not refused: both take a free arch as provisional. When
/// nothing is imported or the job is cancelled, `state` stays as it was.
/// Callers holding a shared state clone it first and keep the old one for
/// undo: the whole batch is one change.
pub fn import_scans(state: &mut CaseState, request: &ImportRequest, ctx: &JobCtx) -> ScanImport {
    let registered = bite_registered(state);
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
            detections: Vec::new(),
        };
    }
    report.warnings.extend(failures);
    for d in detections.iter() {
        let settled = report
            .imported
            .iter()
            .find(|s| s.path == d.path && s.role_source == RoleSource::Detected);
        if let Some(scan) = settled
            && needs_width_check(d.detection.source)
        {
            report
                .warnings
                .push(width_check(&scan.source_name, scan.role));
        }
    }
    if registered && report.replaced_roles.iter().any(|r| *r != ScanRole::Bite) {
        report.warnings.push(bite_not_reset());
    }
    ScanImport { report, detections }
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
    let reasons: Vec<String> = report
        .failed
        .iter()
        .map(|f| match &f.detail {
            Some(d) => format!("{} ({d})", f.reason),
            None => f.reason.clone(),
        })
        .collect();
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
    let ScanImport { report, detections } = import_scans(&mut next, &request, ctx);
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
    Ok((next, Value::Object(out)))
}

/// What the arch step did to a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// The scan was not provisional: left alone.
    Unchanged,
    /// The arch was recognised: the scan is on it, marked detected.
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
            if needs_width_check(detection.source) {
                warnings.push(width_check(&scan.source_name, want));
            }
            (want, RoleSource::Detected, Outcome::Resolved)
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
    // case_auto.py: `len(obj.data.polygons) < 1000`, whatever the source of the arch
    for scan in arch_scans() {
        let faces = state.blobs.require(&scan.mesh)?.face_count();
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
        match detect(state.blobs.require(&scan.mesh)?, &scan.source_name) {
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
    if bite_registered(state) && !touched.is_empty() {
        warnings.push(bite_not_reset());
    }

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
    use dca_model::CLEARED_IDS;
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
        assert!(m.contains("step 0") && m.contains(DETECT_ARCH), "{m}");

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

    #[test]
    fn replacing_an_arch_after_a_bite_registration_says_it_is_not_reset() {
        let mut state = exec("aligners.demo", &CaseState::default(), json!({}))
            .unwrap()
            .0;
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.bite_registered = true;
        state.set_module(MODULE_ID, ModuleState::new(data));
        let files = Files::new("registered");
        let bite = files.stl("scan_a.stl", &demo_mesh(LOWER, 1));
        let key = "aligners-arch-warn-bite-not-reset";

        // a new bite scan leaves the arches as they are
        let params = json!({ "path": bite, "role": "bite" });
        let (with_bite, out) = import(&state, params).unwrap();
        assert!(!keys(&out).iter().any(|k| k == key), "{out}");

        let params = json!({ "path": bite, "role": "lower" });
        let (next, out) = import(&with_bite, params).unwrap();
        assert!(keys(&out).iter().any(|k| k == key), "{out}");
        let warning = out["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["key"] == key)
            .unwrap();
        assert_ne!(warning["message"], key, "the text is translated");
        // said once: the next replacement has nothing new to say
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!data.bite_registered);
        let (_, out) = import(&next, json!({ "path": bite, "role": "lower" })).unwrap();
        assert!(!keys(&out).iter().any(|k| k == key), "{out}");

        // nothing replaced, no warning, with or without a registration
        let mut state = CaseState::default();
        state.set_module(
            MODULE_ID,
            ModuleState::new(AlignersState {
                bite_registered: true,
                ..AlignersState::default()
            }),
        );
        let (_, out) = import(&state, json!({ "path": bite, "role": "upper" })).unwrap();
        assert!(!keys(&out).iter().any(|k| k == key), "{out}");

        // the arch step warns the same way when it moves a scan
        add(&mut state, demo_mesh(LOWER, 1), "upper", "provisional", 1);
        let (_, out) = detect_step(&state).unwrap();
        assert!(keys(&out).iter().any(|k| k == key), "{out}");
    }

    #[test]
    fn only_the_shape_and_widths_path_is_to_be_checked() {
        for source in [
            ArchSource::FileName,
            ArchSource::ScannerOrientation,
            ArchSource::WilsonOverPalate,
            ArchSource::Palate,
            ArchSource::PalateDisagreesWithMolars,
            ArchSource::CurveOfWilson,
            ArchSource::WilsonAndWidths,
        ] {
            assert!(!needs_width_check(source), "{source:?}");
        }
        assert!(needs_width_check(ArchSource::ShapeAndWidths));
        let w = width_check("scan_a.stl", ScanRole::Lower);
        assert_eq!(w.code, WarningCode::Provisional);
        assert!(w.message().contains("scan_a.stl"));
        assert!(!w.message().contains("aligners-"));

        // the arch step gives the warning when the widths place the scan
        let mut state = CaseState::default();
        let scan = add(&mut state, strip(1000), "lower", "provisional", 1);
        let detection = ArchDetection {
            arch: Arch::Upper,
            confidence: 0.3,
            source: ArchSource::ShapeAndWidths,
            palate_coverage: None,
            occlusal_z: None,
            molar_cusps_mm: None,
            cost_upper: None,
            cost_lower: None,
        };
        let mut warnings = Vec::new();
        let placed = place_provisional(
            vec![(&state.case.scans[0], detection)],
            Vec::new(),
            BTreeSet::new(),
            &mut warnings,
        );
        assert_eq!(placed[0].scan.id, scan.id);
        assert_eq!(placed[0].to, ScanRole::Upper);
        assert_eq!(placed[0].source, RoleSource::Detected);
        assert_eq!(warnings[0].key, "aligners-arch-warn-to-be-checked");
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
