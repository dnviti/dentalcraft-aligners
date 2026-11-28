//! Export: STL files per stage, the table of stages and the change calendar
//! (part 1, [`run`]), and the full package with the print check, the
//! manifest, the report and the patient programmes ([`package::run`]).
//!
//! Part 1 (port plan B3.4) writes into the target directory:
//!
//! * `<arch>_stage_<NN>.stl` (`upper_stage_00.stl`, ...): the scan of the arch
//!   with the teeth at stage `NN` (0 is the initial situation), binary STL
//!   in the coordinates of the case;
//! * `stages.csv`: the movement of every tooth at every stage (the plugin's
//!   `movimenti_per_stadio.csv`: mesial, buccal, extrusion in mm, crown
//!   torque, crown tip and rotation in degrees, cumulative from the rest
//!   position) with the dates of the aligner;
//! * `visits.ics`: the aligner changes and the final check as an iCalendar
//!   file for both arches ([`crate::schedule::to_ics_many`]).
//!
//! The deformed scan moves the vertices labelled with a tooth by the rigid
//! transform of that tooth at the stage ([`staging::world_delta`]); the
//! gingiva vertices follow the teeth with the harmonic weights of the stored
//! gingiva ([`GumDeform`], `AlignersState::gingiva`), or stay where they are
//! when the arch has none.
//!
//! The full package (port plan B4, `ortho_aligner/case_export.py`,
//! `export_parallel.py`, `printcheck.py`, `report.py`) lives in the
//! submodules and takes its meshes from a [`provider::MeshProvider`]:
//!
//! * [`printcheck`]: watertight, manifold, self-intersections, wall, volume,
//!   build plate;
//! * [`package`]: the parallel export (`rayon` instead of the plugin's
//!   background processes) with the plugin's file names;
//! * [`manifest`]: files with BLAKE3 hashes and sizes, and their
//!   verification;
//! * [`report`], [`patient`]: the HTML pages, in English and Italian
//!   ([`text`]).
//!
//! The files are written atomically (a `.tmp` file renamed over the final
//! name), so a cancelled or failed export never leaves a partial file under
//! a final name.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use dc_case::{Fdi, Pose};
use dc_mesh::TriMesh;
use dc_mesh::io::{self, MeshFormat};
use dca_model::Stage;
use glam::{DMat4, DVec3, Vec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::gingiva::{self, GingivaError};
use crate::schedule::{self, Calendar, IcsOptions, Lang, Policy};
use crate::staging;

pub mod html;
pub mod manifest;
pub mod package;
pub mod patient;
pub mod printcheck;
pub mod provider;
pub mod report;
pub mod text;

/// Which print files the full export makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    /// Models for thermoforming and aligners to print directly.
    #[default]
    Both,
    /// Models for thermoforming only.
    Thermo,
    /// Aligners to print directly only.
    Direct,
}

/// Parameters of the export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Target directory (created if it does not exist).
    pub dir: String,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    pub start_date: Option<String>,
    /// Days of wear of each aligner (default: the evidence value, 14).
    pub interval_days: Option<u32>,
    /// Interval policy of the calendar (default fixed).
    pub policy: Option<Policy>,
    /// Language of the text of the calendar.
    pub lang: Lang,
    /// Name of the case in the calendar events.
    pub title: String,
}

impl Default for Params {
    fn default() -> Self {
        let cal = schedule::Params::default();
        Self {
            dir: String::new(),
            start_date: None,
            interval_days: None,
            policy: None,
            lang: cal.lang,
            title: "aligners".to_owned(),
        }
    }
}

/// One arch to export.
#[derive(Debug, Clone, Copy)]
pub struct ArchSeries<'a> {
    /// The arch.
    pub arch: Arch,
    /// The scan, in the coordinates of the case.
    pub mesh: &'a TriMesh,
    /// Tooth number of each vertex of the scan, `0` for the gingiva.
    pub labels: &'a [u16],
    /// Rest pose of each tooth in the coordinates of the case (origin at the
    /// centre of resistance, [`crate::teeth::rest_pose`]).
    pub rest: &'a BTreeMap<Fdi, Pose>,
    /// `+1` when the x axis of the tooth frame points mesially.
    pub md_sign: &'a BTreeMap<Fdi, i8>,
    /// The gingiva of the arch with its blend weights; `None`: the gingiva
    /// of the scan does not move.
    pub gingiva: Option<&'a GumDeform>,
}

/// The stored gingiva of an arch ready to follow the teeth
/// ([`gingiva::gum_weights`], [`gingiva::deform`]).
///
/// The first vertices of a gingiva mesh are the scan vertices, in the same
/// order (the membrane ones moved under the crowns); the vertices of the
/// margin cut follow. A gingiva vertex of the scan moves by the displacement
/// of its gingiva vertex. The gingiva is built on the scan as it was read: a
/// scan closed with a base has the points of its base after those, and they
/// stay where they are.
#[derive(Debug, Clone)]
pub struct GumDeform {
    rest: Vec<DVec3>,
    prep: gingiva::DeformPrep,
    /// The points of the scan the gingiva was built on.
    scan: usize,
}

impl GumDeform {
    /// The deformation of the gingiva mesh `gum` (`AlignersState::gingiva`,
    /// case coordinates) of a scan with `scan_vertices` vertices as it was
    /// read (without the base of a scan closed with one).
    pub fn new(
        gum: &TriMesh,
        scan_vertices: usize,
        params: &gingiva::Params,
    ) -> Result<Self, GingivaError> {
        let g = gingiva::GumSurface::from_mesh(gum)?;
        if g.vertices.len() < scan_vertices {
            return Err(GingivaError::Invalid(format!(
                "the gingiva has {} vertices, the scan {scan_vertices}: it belongs to another scan",
                g.vertices.len()
            )));
        }
        let mut fdis: Vec<Fdi> = g
            .tooth
            .iter()
            .filter_map(|&t| u8::try_from(t).ok().and_then(|x| Fdi::new(x).ok()))
            .collect();
        fdis.sort_unstable();
        fdis.dedup();
        let w = gingiva::gum_weights(&g, &fdis, params);
        Ok(Self {
            prep: gingiva::deform_prep(&w),
            rest: g.vertices,
            scan: scan_vertices,
        })
    }

    /// Displacement of every gingiva vertex for the tooth motions `deltas`.
    fn displacements(&self, deltas: &BTreeMap<Fdi, DMat4>) -> Vec<DVec3> {
        gingiva::deform(&self.rest, &self.prep, deltas)
            .into_iter()
            .zip(&self.rest)
            .map(|(m, r)| m - r)
            .collect()
    }
}

/// Input of the export.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    /// The arches to export.
    pub arches: Vec<ArchSeries<'a>>,
    /// The aligners of the case (`AlignersState::stages`).
    pub stages: &'a [Stage],
}

/// Errors of the export.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// No target directory.
    #[error("no target directory")]
    NoDir,
    /// No stage to export.
    #[error("there are no aligners yet")]
    NoStages,
    /// The labels do not belong to the scan.
    #[error("the {arch} tooth colours do not match the scan ({labels} colours, {vertices} points)")]
    Labels {
        /// Arch.
        arch: &'static str,
        /// Number of labels.
        labels: usize,
        /// Number of vertices.
        vertices: usize,
    },
    /// The calendar could not be built.
    #[error(transparent)]
    Schedule(#[from] schedule::ScheduleError),
    /// A mesh could not be written.
    #[error("{0}")]
    Mesh(String),
    /// The full export failed for another reason (thread pool, JSON, date).
    #[error("{0}")]
    Package(String),
    /// The export was stopped by the user.
    #[error("the export was cancelled")]
    Cancelled,
    /// A file could not be written.
    #[error("{path}: {message}")]
    Io {
        /// File.
        path: String,
        /// What happened.
        message: String,
    },
}

/// A file that was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRecord {
    /// Name relative to the target directory.
    pub name: String,
    /// What it is: `stl`, `csv` or `ics`.
    pub kind: &'static str,
    /// Size in bytes.
    pub bytes: u64,
}

/// What the export did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The target directory.
    pub dir: String,
    /// The files written, STL files first.
    pub files: Vec<FileRecord>,
    /// Active aligners of each arch (`upper`, `lower`).
    pub aligners: BTreeMap<String, u32>,
    /// Aligners of the longest series (the other arch gets passive ones).
    pub total_aligners: u32,
    /// Delivery of the first aligner.
    pub start: String,
    /// The day after the last aligner.
    pub end: String,
    /// Days of the series.
    pub total_days: u32,
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// Pose of every tooth at aligner `k` (`0` is the rest position): the local
/// pose stored in the stage, the last one defined up to `k` when a stage
/// does not mention the tooth.
fn local_poses(stages: &[Stage], arch: Arch, k: u32) -> BTreeMap<Fdi, Pose> {
    let mut ordered: Vec<&Stage> = stages.iter().filter(|s| s.index <= k).collect();
    ordered.sort_by_key(|s| s.index);
    let mut out = BTreeMap::new();
    for s in ordered {
        for (&f, &p) in &s.poses {
            if Arch::of(f) == arch {
                out.insert(f, p);
            }
        }
    }
    out
}

/// The rigid transform of every tooth of the series at aligner `k`, in the
/// coordinates of the case.
pub fn stage_deltas(series: &ArchSeries<'_>, stages: &[Stage], k: u32) -> BTreeMap<Fdi, DMat4> {
    local_poses(stages, series.arch, k)
        .into_iter()
        .filter_map(|(f, local)| {
            let rest = series.rest.get(&f)?;
            Some((f, staging::world_delta(rest, &local).to_mat4()))
        })
        .collect()
}

/// The gingiva follows the teeth: every vertex of the scan labelled `0`
/// moves by the displacement of its vertex in `gum` (the base of a scan
/// closed with one stays); `positions` already carry the teeth.
fn deform_gingiva(
    positions: &mut [Vec3],
    labels: &[u16],
    gum: &GumDeform,
    deltas: &BTreeMap<Fdi, DMat4>,
) {
    if deltas.values().all(|d| *d == DMat4::IDENTITY) {
        return;
    }
    let disp = gum.displacements(deltas);
    let scanned = positions.len().min(gum.scan);
    for ((p, &l), d) in positions[..scanned].iter_mut().zip(labels).zip(&disp) {
        if l == 0 && *d != DVec3::ZERO {
            *p = (p.as_dvec3() + *d).as_vec3();
        }
    }
}

/// The model of a series at a stage: [`deform_scan`] with the gingiva of the
/// series following the teeth when it has one.
pub fn deform_series(series: &ArchSeries<'_>, deltas: &BTreeMap<Fdi, DMat4>) -> TriMesh {
    let mut out = deform_scan(series.mesh, series.labels, deltas);
    if let Some(gum) = series.gingiva {
        deform_gingiva(&mut out.positions, series.labels, gum, deltas);
    }
    out
}

/// The scan with every labelled tooth moved by its transform in `deltas`
/// (case coordinates); the other vertices are not moved by the teeth
/// ([`deform_series`] moves the gingiva too).
pub fn deform_scan(mesh: &TriMesh, labels: &[u16], deltas: &BTreeMap<Fdi, DMat4>) -> TriMesh {
    let mut out = TriMesh::new(mesh.positions.clone(), mesh.tris.clone());
    let by_label: BTreeMap<u16, &DMat4> = deltas
        .iter()
        .map(|(f, d)| (u16::from(f.get()), d))
        .collect();
    for (p, &l) in out.positions.iter_mut().zip(labels) {
        if let Some(d) = by_label.get(&l) {
            *p = d.transform_point3(p.as_dvec3()).as_vec3();
        }
    }
    out
}

fn io_err(path: &Path, e: impl std::fmt::Display) -> ExportError {
    ExportError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

/// Writes a file atomically: to `<name>.tmp`, synced to the disk, then
/// renamed, and the folder synced (best effort), so a power loss never
/// leaves a short file under the final name.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    use std::io::Write as _;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(&tmp, e)
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(path, e)
    })?;
    // the rename itself (best effort: a folder cannot be synced everywhere)
    let _ = sync_dir(path);
    Ok(())
}

/// Syncs the folder of `path` (its entries: a rename into it).
#[cfg(unix)]
fn sync_dir(path: &Path) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::File::open(dir)?.sync_all()
}

/// Windows and the rest: the folder cannot be opened as a file; NTFS logs
/// the rename in its journal.
#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn stl_bytes(mesh: &TriMesh) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::with_capacity(84 + 50 * mesh.tris.len());
    io::write_to(&mut buf, mesh, MeshFormat::Stl).map_err(|e| ExportError::Mesh(e.to_string()))?;
    Ok(buf)
}

/// File name of the model of aligner `k` of an arch.
pub fn stl_name(arch: Arch, k: u32, width: usize) -> String {
    format!("{}_stage_{k:0width$}.stl", arch_name(arch))
}

/// The table of stages: one row per tooth and stage, comma separated, `.` as
/// the decimal mark. Columns: `arch,stage,aligner,tooth,mesial_mm,
/// buccal_mm,extrusion_mm,torque_deg,tip_deg,rotation_deg,start,end` (the
/// dates of the aligner, empty at stage 0).
pub fn stages_csv(input: &Input<'_>, calendars: &BTreeMap<Arch, Calendar>) -> String {
    let mut out = String::from(
        "arch,stage,aligner,tooth,mesial_mm,buccal_mm,extrusion_mm,torque_deg,tip_deg,rotation_deg,start,end\n",
    );
    for series in &input.arches {
        let Some(cal) = calendars.get(&series.arch) else {
            continue;
        };
        let teeth: Vec<Fdi> = {
            let present: Vec<Fdi> = input
                .stages
                .iter()
                .flat_map(|s| s.poses.keys().copied())
                .filter(|&f| Arch::of(f) == series.arch)
                .collect();
            anatomy::arch_sequence(&present, series.arch)
        };
        for f in teeth {
            let md = series.md_sign.get(&f).copied().unwrap_or(1);
            for k in 0..=cal.n_active {
                let q = local_poses(input.stages, series.arch, k)
                    .get(&f)
                    .map_or([0.0; 6], staging::q_from_pose);
                let d = staging::describe_q(&q, md);
                let (start, end) = match k.checked_sub(1).and_then(|i| cal.rows.get(i as usize)) {
                    Some(r) => (r.start.to_iso(), r.end.to_iso()),
                    None => (String::new(), String::new()),
                };
                let _ = writeln!(
                    out,
                    "{},{k},{},{f},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{start},{end}",
                    arch_name(series.arch),
                    cal.offset + k,
                    d[0],
                    d[1],
                    d[2],
                    d[3],
                    d[4],
                    d[5],
                );
            }
        }
    }
    out
}

/// The first day of wear of every aligner (`(arch, aligner)`, passive ones
/// included) in the calendars of `params`: the date of the stamp when it
/// carries the start of the aligner (`case_stamp.day_of`). Empty when there
/// is no calendar.
pub fn start_dates(input: &Input<'_>, params: &Params) -> BTreeMap<(Arch, u32), schedule::Date> {
    plan_calendars(input, params, &BTreeMap::new())
        .map(|plan| {
            plan.calendars
                .iter()
                .flat_map(|(a, c)| c.rows.iter().map(move |r| ((*a, r.aligner), r.start)))
                .collect()
        })
        .unwrap_or_default()
}

/// The series of each arch and their calendars.
struct Plan {
    /// Active aligners of each arch with any.
    active: BTreeMap<Arch, u32>,
    /// Aligners of the longest series.
    total: u32,
    /// The calendar of each arch, passive aligners included.
    calendars: BTreeMap<Arch, Calendar>,
}

/// Counts the aligners of every arch and builds their calendars from one
/// start date (today when none is given).
fn plan_calendars(
    input: &Input<'_>,
    params: &Params,
    attachments: &BTreeMap<Arch, Vec<String>>,
) -> Result<Plan, ExportError> {
    // the series of each arch and the total including the passive aligners
    let active: BTreeMap<Arch, u32> = input
        .arches
        .iter()
        .map(|s| (s.arch, schedule::active_stages(s.arch, input.stages)))
        .filter(|(_, n)| *n > 0)
        .collect();
    let total = active
        .values()
        .copied()
        .max()
        .ok_or(ExportError::NoStages)?;

    // one start date for both arches (today when none is given)
    let cal_params = schedule::Params {
        start_date: Some(
            params
                .start_date
                .clone()
                .unwrap_or_else(|| schedule::Date::today().to_iso()),
        ),
        interval_days: params
            .interval_days
            .unwrap_or_else(|| schedule::Params::default().interval_days),
        policy: params.policy.unwrap_or_default(),
        lang: params.lang,
        ..schedule::Params::default()
    };
    let mut calendars: BTreeMap<Arch, Calendar> = BTreeMap::new();
    for s in &input.arches {
        if !active.contains_key(&s.arch) {
            continue;
        }
        let mut cal_in = schedule::input_from_stages(
            s.arch,
            input.stages,
            attachments.get(&s.arch).cloned().unwrap_or_default(),
        );
        cal_in.n_total = Some(total);
        calendars.insert(s.arch, schedule::run(&cal_in, &cal_params)?);
    }
    Ok(Plan {
        active,
        total,
        calendars,
    })
}

/// The target directory is given and the labels belong to the scans.
fn check_input(input: &Input<'_>, dir: &str) -> Result<(), ExportError> {
    if dir.trim().is_empty() {
        return Err(ExportError::NoDir);
    }
    for s in &input.arches {
        if s.labels.len() != s.mesh.vertex_count() {
            return Err(ExportError::Labels {
                arch: arch_name(s.arch),
                labels: s.labels.len(),
                vertices: s.mesh.vertex_count(),
            });
        }
    }
    Ok(())
}

/// Exports the stages: STL files, `stages.csv` and `visits.ics`, the
/// attachments of each arch (teeth) bonded before its first aligner.
pub fn run(
    input: &Input<'_>,
    params: &Params,
    attachments: &BTreeMap<Arch, Vec<String>>,
) -> Result<Report, ExportError> {
    check_input(input, &params.dir)?;
    let dir = PathBuf::from(&params.dir);
    let Plan {
        active,
        total,
        calendars,
    } = plan_calendars(input, params, attachments)?;

    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    // the models: (arch index, stage)
    let width = total.to_string().len().max(2);
    let tasks: Vec<(usize, u32)> = input
        .arches
        .iter()
        .enumerate()
        .filter(|(_, s)| active.contains_key(&s.arch))
        .flat_map(|(i, s)| (0..=active[&s.arch]).map(move |k| (i, k)))
        .collect();
    let mut files: Vec<FileRecord> = tasks
        .par_iter()
        .map(|&(i, k)| -> Result<FileRecord, ExportError> {
            let s = &input.arches[i];
            let deltas = stage_deltas(s, input.stages, k);
            let mesh = deform_series(s, &deltas);
            let bytes = stl_bytes(&mesh)?;
            let name = stl_name(s.arch, k, width);
            write_atomic(&dir.join(&name), &bytes)?;
            Ok(FileRecord {
                name,
                kind: "stl",
                bytes: bytes.len() as u64,
            })
        })
        .collect::<Result<_, _>>()?;
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let csv = stages_csv(input, &calendars);
    write_atomic(&dir.join("stages.csv"), csv.as_bytes())?;
    files.push(FileRecord {
        name: "stages.csv".to_owned(),
        kind: "csv",
        bytes: csv.len() as u64,
    });

    let stamp = String::new();
    let options: Vec<IcsOptions> = input
        .arches
        .iter()
        .filter(|s| calendars.contains_key(&s.arch))
        .map(|s| IcsOptions {
            title: params.title.clone(),
            arch_label: arch_label(s.arch, params.lang).to_owned(),
            lang: params.lang,
            stamp: stamp.clone(),
            ..IcsOptions::default()
        })
        .collect();
    let entries: Vec<(&Calendar, &IcsOptions)> = input
        .arches
        .iter()
        .filter_map(|s| calendars.get(&s.arch))
        .zip(&options)
        .collect();
    let ics = schedule::to_ics_many(&entries);
    write_atomic(&dir.join("visits.ics"), ics.as_bytes())?;
    files.push(FileRecord {
        name: "visits.ics".to_owned(),
        kind: "ics",
        bytes: ics.len() as u64,
    });

    let first = calendars.values().next().ok_or(ExportError::NoStages)?;
    Ok(Report {
        dir: params.dir.clone(),
        files,
        aligners: active
            .iter()
            .map(|(a, n)| (arch_name(*a).to_owned(), *n))
            .collect(),
        total_aligners: total,
        start: first.start.to_iso(),
        end: first.end.to_iso(),
        total_days: first.total_days,
    })
}

/// Name of an arch in the calendar.
pub fn arch_label(arch: Arch, lang: Lang) -> &'static str {
    match (arch.is_upper(), lang) {
        (true, Lang::En) => "upper arch",
        (false, Lang::En) => "lower arch",
        (true, Lang::It) => "arcata superiore",
        (false, Lang::It) => "arcata inferiore",
    }
}

/// The rest poses of `frames` as the plugin's rest frames (origin at the
/// centre of resistance), in the coordinates of the case.
pub fn rest_poses(frames: &BTreeMap<Fdi, dca_model::ToothFrame>) -> BTreeMap<Fdi, Pose> {
    frames
        .iter()
        .map(|(&f, fr)| (f, crate::teeth::rest_pose(fr)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// Two unit cubes, labelled 11 and 21, on a base labelled 0.
    fn scan() -> (TriMesh, Vec<u16>) {
        let mut pos = Vec::new();
        let mut tris = Vec::new();
        let mut labels = Vec::new();
        for (cx, label) in [(-2.0_f32, 11_u16), (2.0, 21), (0.0, 0)] {
            let o = pos.len() as u32;
            for &(x, y, z) in &[
                (0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (1.0, 1.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.0, 0.0, 1.0),
                (1.0, 0.0, 1.0),
                (1.0, 1.0, 1.0),
                (0.0, 1.0, 1.0),
            ] {
                pos.push(Vec3::new(cx + x, y, z - if label == 0 { 3.0 } else { 0.0 }));
                labels.push(label);
            }
            for f in [
                [0, 2, 1],
                [0, 3, 2],
                [4, 5, 6],
                [4, 6, 7],
                [0, 1, 5],
                [0, 5, 4],
                [1, 2, 6],
                [1, 6, 5],
                [2, 3, 7],
                [2, 7, 6],
                [3, 0, 4],
                [3, 4, 7],
            ] {
                tris.push([o + f[0], o + f[1], o + f[2]]);
            }
        }
        (TriMesh::new(pos, tris), labels)
    }

    fn stages_of(n: u32) -> Vec<Stage> {
        (1..=n)
            .map(|k| {
                let mut s = Stage::new(k);
                // 11 moves 0.25 mm per aligner along x of its frame
                s.poses.insert(
                    fdi(11),
                    staging::pose_from_q(&[0.25 * f64::from(k), 0.0, 0.0, 0.0, 0.0, 0.0]),
                );
                s
            })
            .collect()
    }

    fn series<'a>(
        mesh: &'a TriMesh,
        labels: &'a [u16],
        rest: &'a BTreeMap<Fdi, Pose>,
        md: &'a BTreeMap<Fdi, i8>,
    ) -> ArchSeries<'a> {
        ArchSeries {
            arch: Arch::Upper,
            mesh,
            labels,
            rest,
            md_sign: md,
            gingiva: None,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dca-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn write_atomic_replaces_the_file_and_leaves_no_temporary() {
        let dir = temp_dir("atomic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.stl");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second, longer");
        assert!(!dir.join("a.stl.tmp").exists());
        // a folder that does not exist: an error naming the temporary file
        let missing = dir.join("no/such.stl");
        assert!(matches!(
            write_atomic(&missing, b"x"),
            Err(ExportError::Io { path, .. }) if path.ends_with("such.stl.tmp")
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_teeth_move_by_their_delta_and_the_rest_stays() {
        let (mesh, labels) = scan();
        // the rest frame of 11 is rotated 90 degrees about z: its x axis is the case y axis
        let rest = BTreeMap::from([(
            fdi(11),
            Pose::new(
                DVec3::new(-1.5, 0.5, 0.5),
                glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
            ),
        )]);
        let md = BTreeMap::from([(fdi(11), 1)]);
        let s = series(&mesh, &labels, &rest, &md);
        let stages = stages_of(4);
        let d0 = stage_deltas(&s, &stages, 0);
        let m0 = deform_scan(&mesh, &labels, &d0);
        assert_eq!(m0.positions, mesh.positions);
        let d = stage_deltas(&s, &stages, 4);
        let m = deform_scan(&mesh, &labels, &d);
        for (i, (a, b)) in mesh.positions.iter().zip(&m.positions).enumerate() {
            if labels[i] == 11 {
                assert!((*b - *a - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5, "{i}");
            } else {
                assert_eq!(a, b, "{i}");
            }
        }
        assert_eq!(m.tris, mesh.tris);
    }

    #[test]
    fn the_export_writes_the_stl_the_csv_and_the_calendar() {
        let (mesh, labels) = scan();
        let rest = BTreeMap::from([(fdi(11), Pose::from_translation(DVec3::new(-1.5, 0.5, 0.5)))]);
        let md = BTreeMap::from([(fdi(11), 1)]);
        let stages = stages_of(3);
        let input = Input {
            arches: vec![series(&mesh, &labels, &rest, &md)],
            stages: &stages,
        };
        let dir = temp_dir("all");
        let params = Params {
            dir: dir.display().to_string(),
            start_date: Some("2026-02-02".into()),
            interval_days: Some(10),
            ..Params::default()
        };
        let report = run(&input, &params, &BTreeMap::new()).unwrap();
        assert_eq!(report.total_aligners, 3);
        assert_eq!(report.start, "2026-02-02");
        assert_eq!(report.end, "2026-03-04");
        assert_eq!(report.files.iter().filter(|f| f.kind == "stl").count(), 4);

        // the STL files load back and have the right geometry
        for k in 0..=3 {
            let m = io::read(dir.join(format!("upper_stage_{k:02}.stl"))).unwrap();
            assert_eq!(m.face_count(), mesh.face_count());
            // the base is static, 11 moves 0.25 mm per aligner along x, 21 stays
            let min = m
                .positions
                .iter()
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min);
            let max = m.positions.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            assert!(
                (min - (-2.0 + 0.25 * k as f32)).abs() < 1e-5,
                "stage {k}: {min}"
            );
            assert!((max - 3.0).abs() < 1e-5);
        }

        let csv = std::fs::read_to_string(dir.join("stages.csv")).unwrap();
        let rows: Vec<Vec<&str>> = csv.lines().map(|l| l.split(',').collect()).collect();
        assert_eq!(rows[0].len(), 12);
        assert!(rows.iter().all(|r| r.len() == 12));
        assert_eq!(rows.len(), 1 + 4);
        assert_eq!(rows[4][1], "3");
        assert_eq!(rows[4][3], "11");
        assert_eq!(rows[4][4], "0.750");
        assert_eq!(rows[4][10], "2026-02-22");
        assert_eq!(rows[1][10], "");

        let ics = std::fs::read_to_string(dir.join("visits.ics")).unwrap();
        crate::schedule::validate_ics(&ics).unwrap();
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 4);

        // no temporary files are left
        assert!(
            std::fs::read_dir(&dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_errors_are_reported() {
        let (mesh, labels) = scan();
        let rest = BTreeMap::new();
        let md = BTreeMap::new();
        let stages = stages_of(1);
        let mut input = Input {
            arches: vec![series(&mesh, &labels, &rest, &md)],
            stages: &stages,
        };
        assert!(matches!(
            run(&input, &Params::default(), &BTreeMap::new()),
            Err(ExportError::NoDir)
        ));
        let dir = temp_dir("err");
        let params = Params {
            dir: dir.display().to_string(),
            ..Params::default()
        };
        let none: Vec<Stage> = Vec::new();
        let empty = Input {
            arches: input.arches.clone(),
            stages: &none,
        };
        assert!(matches!(
            run(&empty, &params, &BTreeMap::new()),
            Err(ExportError::NoStages)
        ));
        let bad = Params {
            start_date: Some("soon".into()),
            ..params.clone()
        };
        assert!(matches!(
            run(&input, &bad, &BTreeMap::new()),
            Err(ExportError::Schedule(_))
        ));
        let short = &labels[..labels.len() - 1];
        input.arches[0].labels = short;
        assert!(matches!(
            run(&input, &params, &BTreeMap::new()),
            Err(ExportError::Labels { .. })
        ));
        assert!(!dir.exists(), "nothing is created when the input is wrong");
    }
}
