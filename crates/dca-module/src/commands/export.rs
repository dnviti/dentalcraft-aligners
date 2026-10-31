//! Commands of the `export` stage (`dca_algo::export`).
//!
//! * `aligners.output.export-stl` writes the aligners into a target
//!   directory: the scan of each arch with the teeth at every stage as binary
//!   STL (`upper_stage_00.stl` is the initial situation), `stages.csv` with
//!   the movement of every tooth at every stage and the dates, and
//!   `visits.ics` with the aligner changes of both arches. Parameters:
//!   `dir`, `start_date`, `interval_days`, `policy`, `lang`.
//! * `aligners.output.report` is the full export package: the models (and
//!   the shells, once `shell.rs` is wired in), the print check of every
//!   mesh, the tables and calendars, the programme of the patient for each
//!   arch, the HTML report with two renders, `print_check.json` and
//!   `manifest.json`, in parallel. See [`dca_algo::export::package`].
//! * `aligners.output.print-check` runs the print check on the models of
//!   some stages without writing anything.
//! * `aligners.output.manifest` verifies a folder against its
//!   `manifest.json`, or writes a manifest for a folder that has none.
//!
//! The case is not modified. The gingiva stays where it is in the models
//! until the gingiva deformation lands.
//!
//! # Wiring the shells
//!
//! The meshes come from [`mesh_provider`]: today the deformed scan as the
//! model and no shell. When `shell.rs` is ready, return a provider built on
//! it there (the trait is `dca_algo::export::provider::MeshProvider`).

use std::collections::BTreeMap;
use std::path::Path;

use dc_case::{CaseState, Fdi, Pose, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::export::manifest;
use dca_algo::export::package::{self, CaseInfo, FullParams};
use dca_algo::export::printcheck::{self, CheckOptions};
use dca_algo::export::provider::{MeshProvider, ScanProvider};
use dca_algo::export::report::Renders;
use dca_algo::export::{self, ArchSeries};
use dca_algo::schedule;
use dca_algo::teeth;
use dca_model::AlignersState;
use schemars::JsonSchema;
use serde::Deserialize;

use super::staging::{LabelledScan, aligners_data, labelled_scan, md_sign};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.output.export-stl",
            label: "aligners-menu-output-export-stl",
            doc: "aligners-cmd-export-doc",
            params: dca_algo::export::Params,
            run: run_export,
        },
        stage_command! {
            id: "aligners.output.print-check",
            label: "aligners-menu-output-print-check",
            doc: "aligners-cmd-print-check-doc",
            params: PrintCheckParams,
            run: run_print_check,
        },
        stage_command! {
            id: "aligners.output.manifest",
            label: "aligners-menu-output-manifest",
            doc: "aligners-cmd-manifest-doc",
            params: ManifestParams,
            run: run_manifest,
        },
        stage_command! {
            id: "aligners.output.report",
            label: "aligners-menu-output-report",
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
}

/// The arches that have aligners and a segmented scan; an error when there
/// are none.
fn prepare(state: &CaseState, data: &AlignersState) -> Result<Vec<ArchData>, CommandError> {
    if data.stages.is_empty() {
        return Err(CommandError::Invalid(
            "there are no aligners yet: run the staging first".to_owned(),
        ));
    }
    let mut out = Vec::new();
    for arch in [Arch::Upper, Arch::Lower] {
        if schedule::active_stages(arch, &data.stages) == 0 {
            continue;
        }
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
            out.push(ArchData {
                arch,
                scan,
                rest,
                md,
            });
        }
    }
    if out.is_empty() {
        return Err(CommandError::Invalid(
            "no arch with aligners has a segmented scan".to_owned(),
        ));
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
            })
            .collect(),
        stages: &data.stages,
    }
}

/// Where the printable meshes of every stage come from. Wiring point of
/// the shell code: return the provider built on `shell.rs` here.
fn mesh_provider<'a>(input: &'a export::Input<'a>) -> Box<dyn MeshProvider + 'a> {
    Box::new(ScanProvider::new(input))
}

fn export_error(e: export::ExportError) -> CommandError {
    match e {
        export::ExportError::Io { .. }
        | export::ExportError::Mesh(_)
        | export::ExportError::Package(_) => CommandError::Failed(e.to_string()),
        other => CommandError::Invalid(other.to_string()),
    }
}

fn run_export(
    state: &CaseState,
    mut params: export::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if data.stages.is_empty() {
        return Err(CommandError::Invalid(
            "there are no aligners yet: run the staging first".to_owned(),
        ));
    }
    if params.dir.trim().is_empty() {
        return Err(CommandError::Invalid(
            "dir: the target directory is required".to_owned(),
        ));
    }
    params
        .interval_days
        .get_or_insert(data.settings.interval_days);
    let arches = prepare(state, &data)?;
    let input = input(&arches, &data);
    ctx.check_cancelled()?;
    let report = export::run(&input, &params).map_err(export_error)?;
    let out = serde_json::to_value(&report).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((state.clone(), out))
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
    params
        .interval_days
        .get_or_insert(data.settings.interval_days);
    let input = input(&arches, &data);
    let renders = if params.report && params.embed_renders {
        render_views(state, &data)
    } else {
        Renders::default()
    };
    ctx.check_cancelled()?;
    let case = CaseInfo {
        patient_code: &state.case.patient_code,
        state: &data,
        // the attachments are not planned yet (WP: attachments)
        attachments: BTreeMap::new(),
        renders,
    };
    let provider = mesh_provider(&input);
    let report = package::run(&input, &case, provider.as_ref(), &params).map_err(export_error)?;
    let out = serde_json::to_value(&report).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((state.clone(), out))
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
}

impl Default for PrintCheckParams {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            self_intersections: true,
            build_plate_mm: Some(printcheck::DEFAULT_PLATE_MM),
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
    let provider = mesh_provider(&input);
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
            ctx.check_cancelled()?;
            let Some(model) = provider.model(a.arch, k).map_err(export_error)? else {
                continue;
            };
            let check = printcheck::check(&model, &options);
            results.push(serde_json::json!({
                "arch": if a.arch.is_upper() { "upper" } else { "lower" },
                "stage": k,
                "kind": "model",
                "ok": check.ok,
                "problems": check.problem_texts(dca_algo::schedule::Lang::En),
                "check": check,
            }));
        }
    }
    let passed = results.iter().filter(|r| r["ok"] == true).count();
    let out = serde_json::json!({
        "checked": results.len(),
        "passed": passed,
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
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_command_is_registered_as_the_menu_leaf() {
        let spec = crate::commands::find("aligners.output.export-stl").unwrap();
        assert!(spec.schema()["properties"]["dir"].is_object());
        assert!(crate::tree::find("aligners.output.export-stl").is_some());
    }

    #[test]
    fn it_needs_aligners_and_a_directory() {
        let err = exec(
            "aligners.output.export-stl",
            &CaseState::default(),
            json!({"dir": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("staging")));
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
    fn the_export_leaves_are_registered() {
        for id in [
            "aligners.output.report",
            "aligners.output.print-check",
            "aligners.output.manifest",
        ] {
            assert!(crate::commands::find(id).is_some(), "{id}");
            assert!(crate::tree::find(id).is_some(), "{id}");
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
    fn the_report_needs_aligners_and_a_directory() {
        let err = exec(
            "aligners.output.report",
            &CaseState::default(),
            json!({"dir": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("staging")));
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
