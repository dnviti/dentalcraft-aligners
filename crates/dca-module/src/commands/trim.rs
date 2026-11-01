//! Commands of the `trim` stage (`dca_algo::trim`).
//!
//! `aligners.production.trim-line` computes the trim line of the aligner of
//! an arch and cuts the shell along it: the line of the mode (equator of
//! every crown, straight beyond the gingival zenith, scalloped along the
//! margin, smooth along the margin of the arch), the trim
//! surface through it, the shell intersected with the solid above that
//! surface and with the reach from the crowns, with a rounded rim. Until the
//! model of a stage comes from the shell stage, the model is the segmented
//! scan itself, which has to be a closed solid (the parts of the demo arch
//! are closed and merged); the shell is the sheet of the case's wall
//! thickness and clearance laid on it. The trim settings of the case are
//! the defaults of the parameters. The case is not modified: the line of
//! every tooth and the trimmed aligner are described (case coordinates)
//! and, with `dir`, the aligner is written there as binary STL
//! (`<arch>_trimmed.stl`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_mesh::io::MeshFormat;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::teeth::{self, Which};
use dca_algo::trim::warp::InsertionAxis;
use dca_algo::trim::{self, Mode, model};
use dca_model::TrimMode;
use glam::DVec3;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::staging::{aligners_data, arch_name, labelled_scan};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.production.trim-line",
        label: "aligners-cmd-trim-line",
        doc: "aligners-cmd-trim-line-doc",
        params: TrimLineParams,
        run: run_trim_line,
    }]
}

/// Parameters of `aligners.production.trim-line`; the trim values default to
/// the trim settings of the case, the wall and the clearance to its
/// settings.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
pub(crate) struct TrimLineParams {
    /// Arches to trim.
    arch: Which,
    /// Directory where the trimmed aligners are written as binary STL
    /// (`upper_trimmed.stl`, `lower_trimmed.stl`, case coordinates); not
    /// written when missing.
    dir: Option<String>,
    /// Shape of the edge (equator, straight, scalloped, smooth).
    mode: Option<Mode>,
    /// Smooth: distance of the border from the gingival margin (mm).
    margin_gap_mm: Option<f64>,
    /// Smooth: height of the scallops (mm).
    scallop_mm: Option<f64>,
    /// Equator: border beyond the equator, towards the gingiva (mm).
    equator_offset_mm: Option<f64>,
    /// Straight, scalloped: extension below the gingival margin (mm).
    extension_mm: Option<f64>,
    /// Radius of the rounded rim (mm; default 0.3 x wall, at most 0.3).
    round_mm: Option<f64>,
    /// Equator: the insertion axis (vertical, common or per_tooth).
    insertion_axis: InsertionAxis,
    /// Wall thickness of the shell (mm).
    thickness_mm: Option<f64>,
    /// Internal clearance (mm).
    clearance_mm: Option<f64>,
    /// Detail of the shell (mm).
    voxel_mm: f64,
    /// Crevices narrower than twice this are filled before the shell (mm).
    blockout_mm: f64,
}

impl Default for TrimLineParams {
    fn default() -> Self {
        Self {
            arch: Which::All,
            dir: None,
            mode: None,
            margin_gap_mm: None,
            scallop_mm: None,
            equator_offset_mm: None,
            extension_mm: None,
            round_mm: None,
            insertion_axis: InsertionAxis::PerTooth,
            thickness_mm: None,
            clearance_mm: None,
            voxel_mm: 0.1,
            blockout_mm: 0.3,
        }
    }
}

fn mode_of(m: TrimMode) -> Mode {
    match m {
        TrimMode::Equator => Mode::Equator,
        TrimMode::Straight => Mode::Straight,
        TrimMode::Scalloped => Mode::Scalloped,
        TrimMode::Smooth => Mode::Smooth,
    }
}

fn run_trim_line(
    state: &CaseState,
    params: TrimLineParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let case = &data.trim;
    let tp = trim::Params {
        mode: params.mode.unwrap_or(mode_of(case.mode)),
        margin_gap_mm: params.margin_gap_mm.unwrap_or(case.margin_gap_mm),
        scallop_mm: params.scallop_mm.unwrap_or(case.scallop_mm),
        equator_offset_mm: params.equator_offset_mm.unwrap_or(case.equator_offset_mm),
        extension_mm: params.extension_mm.unwrap_or(case.extension_mm),
        round_mm: params.round_mm,
        insertion_axis: params.insertion_axis,
    };
    let wall = params
        .thickness_mm
        .unwrap_or(data.settings.shell_thickness_mm);
    let clearance = params.clearance_mm.unwrap_or(data.settings.clearance_mm);
    let ok = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
    if !ok(wall, 0.3, 2.0)
        || !ok(clearance, 0.0, 0.3)
        || !ok(params.voxel_mm, 0.04, 0.5)
        || !ok(params.blockout_mm, 0.0, 1.0)
    {
        return Err(CommandError::Invalid(
            "thickness_mm 0.3..2, clearance_mm 0..0.3, voxel_mm 0.04..0.5 and blockout_mm 0..1"
                .to_owned(),
        ));
    }
    let explicit = params.arch != Which::All;
    let dir = params.dir.as_deref().map(Path::new);
    if let Some(d) = dir {
        std::fs::create_dir_all(d)
            .map_err(|e| CommandError::Failed(format!("{}: {e}", d.display())))?;
    }
    let mut out = Map::new();
    for arch in params.arch.arches() {
        ctx.check_cancelled()?;
        let Some(scan) = labelled_scan(state, &data, arch.role())? else {
            if explicit {
                return Err(crate::commands::messages::not_segmented(arch));
            }
            continue;
        };
        let report = trim_arch(
            &data,
            arch,
            &scan.mesh,
            &scan.labels,
            &tp,
            wall,
            clearance,
            &params,
            dir,
            ctx,
        )?;
        out.insert(arch_name(arch).to_owned(), report);
    }
    if out.is_empty() {
        return Err(crate::commands::messages::none_segmented());
    }
    Ok((state.clone(), Value::Object(out)))
}

#[allow(clippy::too_many_arguments)]
fn trim_arch(
    data: &dca_model::AlignersState,
    arch: Arch,
    mesh: &dc_mesh::TriMesh,
    labels: &[u16],
    tp: &trim::Params,
    wall: f64,
    clearance: f64,
    params: &TrimLineParams,
    dir: Option<&Path>,
    ctx: &JobCtx,
) -> Result<Value, CommandError> {
    let t0 = Instant::now();
    let name = arch_name(arch);
    let report = mesh.validate();
    if !(report.is_watertight() && report.is_manifold()) {
        return Err(CommandError::Invalid(format!(
            "the {name} scan is not a closed solid: the trim needs the closed model of the stage"
        )));
    }
    // arch coordinates: the frame of the arch (its z is occlusal)
    let case_vertices: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: &case_vertices,
            seeds: &data.seeds,
        },
        &frame::Params::default(),
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    let to_case = framed.arch_frame.matrix();
    let to_arch = frame::rigid_inverse(&to_case);
    let vertices: Vec<DVec3> = case_vertices
        .iter()
        .map(|&p| to_arch.transform_point3(p))
        .collect();
    // the frames of the teeth (buccal axis) from the teeth stage
    let geo = teeth::run(
        &teeth::Input {
            arch,
            vertices: &vertices,
            tris: &mesh.tris,
            labels,
        },
        &teeth::Params::default(),
    )
    .map_err(|e| CommandError::Failed(format!("{name} arch: {e}")))?;
    let frames: BTreeMap<_, _> = geo
        .teeth
        .iter()
        .map(|(f, t)| (*f, (t.centroid, t.e_bl)))
        .collect();
    ctx.check_cancelled()?;
    let parts = model::parts(&vertices, &mesh.tris, labels, &frames, None);
    let arch_mesh = dc_mesh::TriMesh::new(
        vertices.iter().map(|p| p.as_vec3()).collect(),
        mesh.tris.clone(),
    );
    let shell = model::untrimmed_shell(
        &arch_mesh,
        params.voxel_mm,
        params.blockout_mm,
        clearance,
        wall,
    )
    .map_err(|e| CommandError::Failed(format!("{name} arch: {e}")))?;
    ctx.check_cancelled()?;
    let input = trim::Input {
        shell: &shell,
        teeth_mesh: &parts.teeth_mesh,
        gingiva: &parts.gingiva,
        crowns: &parts.crowns,
        teeth: &parts.teeth,
        scan_border: &parts.scan_border,
        clearance_mm: clearance,
        thickness_mm: wall,
        cut: None,
    };
    let done = trim::run(&input, tp).map_err(|e| match e {
        trim::TrimError::Invalid(m) => CommandError::Invalid(format!("{name} arch: {m}")),
        e => CommandError::Failed(format!("{name} arch: {e}")),
    })?;
    let mut aligner = done.mesh;
    for p in &mut aligner.positions {
        *p = to_case.transform_point3(p.as_dvec3()).as_vec3();
    }
    let check = aligner.validate();
    let lines: Map<String, Value> = done
        .lines
        .iter()
        .map(|l| {
            let pts: Vec<[f64; 3]> = l
                .points
                .iter()
                .map(|&p| to_case.transform_point3(p).to_array())
                .collect();
            let key = l.fdi.map_or_else(|| "arch".to_owned(), |f| f.to_string());
            (key, json!(pts))
        })
        .collect();
    let written = match dir {
        Some(d) => {
            let file = d.join(format!("{name}_trimmed.stl"));
            dc_mesh::io::write(&file, &aligner, MeshFormat::Stl)
                .map_err(|e| CommandError::Failed(format!("{}: {e}", file.display())))?;
            Some(file.display().to_string())
        }
        None => None,
    };
    Ok(json!({
        "mode": done.mode,
        "fallback": done.fallback,
        "round_mm": done.round_mm,
        "vertices": aligner.vertex_count(),
        "faces": aligner.face_count(),
        "watertight": check.is_watertight(),
        "manifold": check.is_manifold(),
        "genus": done.genus,
        "dropped_pieces": done.dropped_pieces,
        "lines": lines,
        "seconds": t0.elapsed().as_secs_f64(),
        "file": written,
    }))
}

#[cfg(test)]
mod tests {
    use dc_case::CaseState;
    use dc_command::{CommandError, JobCtx};
    use dc_module_api::CommandOutput;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_trim_line_is_registered_for_the_cli() {
        let spec = crate::commands::find("aligners.production.trim-line").unwrap();
        assert!(spec.schema()["properties"]["mode"].is_object());
        assert!(crate::tree::find("aligners.production.trim-line").is_none());
        assert!(matches!(
            exec("aligners.production.trim-line", &CaseState::default(), Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("tooth colours")
        ));
    }

    #[test]
    fn the_demo_arch_is_trimmed_on_its_equators() {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0;
        let dir = std::env::temp_dir().join(format!("dca-trim-test-{}", std::process::id()));
        let (_, out) = exec(
            "aligners.production.trim-line",
            &s,
            json!({ "mode": "equator", "voxel_mm": 0.3, "thickness_mm": 0.6, "dir": dir }),
        )
        .unwrap();
        let lower = &out["lower"];
        assert_eq!(lower["mode"], "equator");
        assert_eq!(lower["watertight"], true);
        assert_eq!(lower["manifold"], true);
        assert!(lower["lines"].as_object().unwrap().len() >= 12, "{lower}");
        let file = std::path::PathBuf::from(lower["file"].as_str().unwrap());
        assert!(file.metadata().unwrap().len() > 1000);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
