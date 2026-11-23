//! The print files of the export from the shell stage: the printable model
//! and the trimmed aligner of every stage, with the ramps, the occlusal
//! shaping and the identification stamp.
//!
//! Order of the operations (the plugin's `case_export.export_iter`,
//! `case_print.model_solid`, `aligner_solid` / `_aligner_solid`):
//!
//! * **model** of aligner `k`: the closed teeth at their stage pose and the
//!   gingiva moved with them, closed into a cast with a flat base
//!   ([`dca_algo::shell::run`], model only, reduced within the mesh
//!   tolerance without crossings in case coordinates), then the stamp
//!   embossed on it (`k >= 1`; the initial model has none);
//! * **aligner** `k`: the closed field of the same parts (the blockout of the
//!   trim mode: 0.8 mm for the smooth border, `SMOOTH_BLOCKOUT_MM`; its band
//!   reaching `clearance + wall`, [`shell::closed_grid_for`]), the shell
//!   field between its levels at `clearance` and `clearance + wall`, the
//!   ramps united with it on the field and the cavity of their tooth cut
//!   again ([`dca_algo::ramps::apply_to_grid`]), the trim (cutter, reach of
//!   the crowns, rounded rim: [`dca_algo::trim::run`]), and in it, after the
//!   rim is rounded, the occlusal thickness and the grinding of the bite with
//!   the aligners worn cut from the field, its crease blended
//!   ([`occlusal::shape_cutter`], when the case asks for it and both arches
//!   are there), the reduction within the mesh tolerance
//!   ([`shell::reduce_to`]), then the stamp engraved inside the wall.
//!
//! The reduction and the stamp are checked as the print check would, in
//! case coordinates (self-intersections, holes, thinnest wall): one that
//! makes the file worse to print is left out and the report says so.
//!
//! Deviation: the plugin adds the ramps after the occlusal shaping, with mesh
//! booleans, and lowers the outer vertices of the shell mesh onto the
//! occlusal limit. Here both are done on the shell field before the trim
//! (one meshing, a closed surface by construction): the ramps are united
//! with it, the solid above the occlusal limit is subtracted from it except
//! within the minimum thickness of the model, and nothing is cut over the
//! ramps, so they keep their height as in the plugin.
//!
//! Nothing is kept between two calls but the bite of a stage (small, used by
//! the aligners of both arches): every call builds its fields and drops
//! them, so the export holds only the stages in progress.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use dc_case::{CaseState, Fdi};
use dc_command::CommandError;
use dc_mesh::TriMesh;
use dca_algo::aligner_bite::occlusal;
use dca_algo::anatomy::Arch;
use dca_algo::export::printcheck;
use dca_algo::export::provider::{MeshProvider, Note};
use dca_algo::export::{self, ExportError};
use dca_algo::schedule::{self, Date};
use dca_algo::shell::{self, Design, Outputs};
use dca_algo::stamp::{self, Outcome, ToothSite};
use dca_algo::trim;
use dca_model::{AlignersState, TrimMode};
use glam::{DMat4, DVec3};

use super::super::ramps::StageBite;
use super::super::shell::{ArchPrint, StageGeometry};

/// Thinnest wall of an aligner the reduction may not go below (the default
/// minimum of the print check).
const MIN_WALL_MM: f64 = 0.3;

/// How the print files are made (from the export parameters).
#[derive(Debug, Clone)]
pub(crate) struct Options {
    /// Voxel of the model and shell fields (mm).
    pub voxel_mm: f64,
    /// Mesh tolerance of the files (mm).
    pub tolerance_mm: f64,
    /// Put the identification stamp on the files.
    pub stamp: bool,
    /// Case code of the stamp.
    pub stamp_code: String,
    /// Date of the stamp (the day of the export).
    pub date: Date,
    /// The first day of every aligner when the stamp carries it instead
    /// (`stamp_date` START; an aligner without one carries `date`).
    pub stamp_dates: BTreeMap<(Arch, u32), Date>,
    /// Stops the build of a mesh at the end of its current step.
    pub cancel: dc_command::CancelToken,
    /// The files are checked as print files: a reduction is kept only when
    /// it is no worse to print than the full resolution, and the reach of an
    /// EQUATOR border is measured for the print check. `false` (the preview
    /// of the 3D view, `aligners.view.aligner`): reduced within the
    /// tolerance as it comes, the plugin's `_reduce` (`check=False`).
    pub checked: bool,
}

/// The models and the aligners of every stage from the shell stage.
pub(crate) struct ShellProvider<'a> {
    data: &'a AlignersState,
    input: &'a export::Input<'a>,
    prints: BTreeMap<Arch, ArchPrint>,
    bite: StageBite,
    shape: bool,
    params: shell::Params,
    design: Design,
    trim: trim::Params,
    shell_blockout: f64,
    options: Options,
    bites: Vec<OnceLock<Option<Arc<occlusal::Bite>>>>,
    n_lower: u32,
    n_upper: u32,
    notes: Mutex<Vec<Note>>,
    /// The reach of every aligner made (EQUATOR border), for its print check.
    reaches: Mutex<BTreeMap<(Arch, u32), printcheck::Reach>>,
    /// The shell of the last stage of an arch without its stamp, for the
    /// passive aligners (stamped on it one by one).
    passive_base: Mutex<BTreeMap<(Arch, u32), PassiveBase>>,
}

/// The last shell of an arch, before its stamp (arch coordinates).
struct PassiveBase {
    mesh: TriMesh,
    /// The defects of its file, when known.
    base: Option<Defects>,
    /// The teeth where the stamp can go.
    sites: Vec<ToothSite>,
}

fn rigid(m: &DMat4, mesh: &TriMesh) -> TriMesh {
    TriMesh::new(
        mesh.positions
            .iter()
            .map(|p| m.transform_point3(p.as_dvec3()).as_vec3())
            .collect(),
        mesh.tris.clone(),
    )
}

fn merged(meshes: &[TriMesh]) -> TriMesh {
    let mut out = TriMesh::new(Vec::new(), Vec::new());
    for m in meshes {
        let base = out.positions.len() as u32;
        out.positions.extend_from_slice(&m.positions);
        out.tris.extend(m.tris.iter().map(|t| t.map(|i| i + base)));
    }
    out
}

fn package(what: &str, arch: Arch, k: u32, e: impl std::fmt::Display) -> ExportError {
    let a = if arch.is_upper() { "upper" } else { "lower" };
    ExportError::Package(format!("{what} of the {a} aligner {k}: {e}"))
}

impl<'a> ShellProvider<'a> {
    /// The provider over the arches of `input`; `Err(reason)` when an arch
    /// with aligners has no closed teeth or no gingiva.
    pub(crate) fn new(
        state: &CaseState,
        data: &'a AlignersState,
        input: &'a export::Input<'a>,
        options: Options,
    ) -> Result<Result<Self, String>, CommandError> {
        let mut prints = BTreeMap::new();
        for s in &input.arches {
            match ArchPrint::new(state, data, s.arch)? {
                Some(p) => {
                    prints.insert(s.arch, p);
                }
                None => {
                    return Ok(Err(super::messages::tr(
                        "aligners-err-no-3d-teeth",
                        &[("arch", &super::messages::arch_word(s.arch))],
                    )));
                }
            }
        }
        let bite = StageBite::new(state)?;
        let n_lower = schedule::active_stages(Arch::Lower, &data.stages);
        let n_upper = schedule::active_stages(Arch::Upper, &data.stages);
        let mut notes = Vec::new();
        let shape = bite.shapes_shells();
        if shape && let Some(why) = bite.missing() {
            notes.push(Note {
                arch: None,
                stage: None,
                aligner: true,
                passive: false,
                key: "rpt-note-bite",
                detail: why.to_owned(),
            });
        }
        let t = &data.trim;
        let trim = super::super::trim::case_trim_params(t);
        let shell_blockout = if t.mode == TrimMode::Smooth {
            shell::SMOOTH_BLOCKOUT_MM
        } else {
            shell::BLOCKOUT_MM
        };
        let params = shell::Params {
            model_voxel_mm: options.voxel_mm,
            shell_voxel_mm: options.voxel_mm,
            tolerance_mm: options.tolerance_mm,
            ..shell::Params::default()
        };
        let design = params.design(&data.settings);
        Ok(Ok(Self {
            data,
            input,
            prints,
            bite,
            shape,
            params,
            design,
            trim,
            shell_blockout,
            options,
            bites: (0..=n_lower.max(n_upper))
                .map(|_| OnceLock::new())
                .collect(),
            n_lower,
            n_upper,
            notes: Mutex::new(notes),
            reaches: Mutex::new(BTreeMap::new()),
            passive_base: Mutex::new(BTreeMap::new()),
        }))
    }

    /// The last active stage of an arch.
    fn last(&self, arch: Arch) -> u32 {
        if arch.is_upper() {
            self.n_upper
        } else {
            self.n_lower
        }
    }

    /// A note about the file of model or aligner `k`; an aligner after the
    /// last active stage of its arch is a passive one (its own file).
    fn note(&self, arch: Arch, k: u32, aligner: bool, key: &'static str, detail: String) {
        let passive = aligner && k > self.last(arch);
        self.notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Note {
                arch: Some(arch),
                stage: Some(k),
                aligner,
                passive,
                key,
                detail,
            });
    }

    /// The bite of the aligners of stage `k` (shared by both arches).
    fn bite_at(&self, k: u32) -> Option<Arc<occlusal::Bite>> {
        let (kl, ku) = occlusal::stage_pair(k, self.n_lower, self.n_upper);
        self.bites
            .get(k as usize)?
            .get_or_init(|| self.bite.bite(kl, ku).map(Arc::new))
            .clone()
    }

    /// The trim inputs of a stage in the arch coordinates of `to_arch`: the
    /// labelled scan moved to the stage, the crowns, margins and border of
    /// the teeth the aligner covers (`covered`; `None`: every tooth); the
    /// teeth are the closed ones of the shell stage.
    fn trim_parts(
        &self,
        arch: Arch,
        g: &StageGeometry,
        to_arch: &DMat4,
        closed: &[TriMesh],
        covered: Option<&std::collections::BTreeSet<Fdi>>,
    ) -> Option<trim::model::ModelParts> {
        let series = self.input.arches.iter().find(|s| s.arch == arch)?;
        let scan = export::deform_series(series, &g.deltas);
        let vertices: Vec<DVec3> = scan
            .positions
            .iter()
            .map(|p| to_arch.transform_point3(p.as_dvec3()))
            .collect();
        let frames: BTreeMap<Fdi, (DVec3, DVec3)> = g
            .fdis
            .iter()
            .filter_map(|f| {
                let fr = self.data.tooth_frames.get(f)?;
                let d = g.deltas.get(f).copied().unwrap_or(DMat4::IDENTITY);
                let m = *to_arch * d;
                let origin = m.transform_point3(fr.pose.t);
                let e_bl = m
                    .transform_vector3(fr.pose.q * DVec3::Y)
                    .normalize_or_zero();
                Some((*f, (origin, e_bl)))
            })
            .collect();
        let mut parts = trim::model::parts(&vertices, &scan.tris, series.labels, &frames, covered);
        let by_fdi: BTreeMap<Fdi, &TriMesh> = g.fdis.iter().copied().zip(closed).collect();
        for t in &mut parts.teeth {
            if let Some(m) = by_fdi.get(&t.fdi) {
                t.closed = m.positions.iter().map(|p| p.as_dvec3()).collect();
            }
        }
        parts.teeth_mesh = merged(closed);
        Some(parts)
    }

    /// The teeth where the stamp can go (arch coordinates).
    fn stamp_sites(
        &self,
        parts: &trim::model::ModelParts,
        g: &StageGeometry,
        to_arch: &DMat4,
    ) -> Vec<ToothSite> {
        parts
            .teeth
            .iter()
            .map(|t| {
                let e_md = self.data.tooth_frames.get(&t.fdi).map(|fr| {
                    let d = g.deltas.get(&t.fdi).copied().unwrap_or(DMat4::IDENTITY);
                    (*to_arch * d).transform_vector3(fr.pose.q * DVec3::X)
                });
                ToothSite {
                    fdi: t.fdi,
                    crown: t.crown.clone(),
                    e_md,
                }
            })
            .collect()
    }

    /// The reduction of `full` (arch coordinates) within the tolerance (or
    /// half of it, [`shell::reduce_to`]: no crossing where the vertices land
    /// in case coordinates, `to_case`) when it is no worse to print than
    /// `full`: no more self-intersections and triangles of no area, the same
    /// holes, no wall thinner than `min_wall` that `full` does not have.
    /// Otherwise the full resolution, with a note. Also the defects of the
    /// mesh returned, in case coordinates.
    fn reduce_checked(
        &self,
        full: TriMesh,
        to_case: &DMat4,
        (arch, k): (Arch, u32),
        aligner: bool,
        min_wall: Option<f64>,
    ) -> Result<(TriMesh, Option<Defects>), ExportError> {
        let tol = self.params.tolerance_mm;
        if tol <= 0.0 {
            return Ok((full, None));
        }
        if !self.options.checked {
            let r = shell::reduce_to(&full, tol, Some(to_case))
                .map_err(|e| package("reduction", arch, k, e))?;
            return Ok((r, None));
        }
        let base = check(&rigid(to_case, &full), min_wall);
        let wall = |c: &printcheck::PrintCheck| c.min_thickness_mm.unwrap_or(f64::INFINITY);
        let floor = min_wall.map_or(0.0, |m| wall(&base).min(m));
        for t in [tol, 0.5 * tol] {
            let r = shell::reduce_to(&full, t, Some(to_case))
                .map_err(|e| package("reduction", arch, k, e))?;
            if r.tris.len() == full.tris.len() {
                break;
            }
            let c = check(&rigid(to_case, &r), min_wall);
            if Defects::of(&c).no_worse_than(&Defects::of(&base))
                && c.genus == base.genus
                && c.boundary_edges == 0
                && (min_wall.is_none() || wall(&c) >= floor - 1e-9)
            {
                return Ok((r, Some(Defects::of(&c))));
            }
        }
        self.note(arch, k, aligner, "rpt-note-reduce", format!("{tol} mm"));
        Ok((full, Some(Defects::of(&base))))
    }

    /// The stamp on `mesh` (arch coordinates), on one of the `covered`
    /// teeth of `sites`, for aligner `k` (`passive`: a passive aligner, its
    /// own number with the `P`): the mesh stamped and the file it makes
    /// (`land`: the mesh in case coordinates), or `None` with a note (no
    /// room). A text that would fold the surface (more self-intersections or
    /// triangles of no area in the file than `base`, those of the file of
    /// `mesh` when known) is not kept: the next tooth is tried.
    fn stamped(
        &self,
        mesh: &TriMesh,
        base_known: Option<Defects>,
        land: &dyn Fn(&TriMesh) -> TriMesh,
        (arch, k, passive): (Arch, u32, bool),
        kind: stamp::Kind,
        (sites, covered): (Vec<ToothSite>, &std::collections::BTreeSet<Fdi>),
    ) -> Option<(TriMesh, TriMesh)> {
        let aligner = kind == stamp::Kind::Shell;
        // the text, the tooth, the face and the sizes of the case settings
        let case = match super::super::stamp::case_stamp_params(
            &self.data.settings.stamp,
            &serde_json::Map::new(),
        ) {
            Ok(p) => p,
            Err(e) => {
                self.note(arch, k, aligner, "rpt-note-stamp", e.to_string());
                return None;
            }
        };
        let clearance = if aligner {
            self.design.clearance_mm
        } else {
            0.0
        };
        let params = stamp_params(case, &self.options, (arch, k, passive), kind, clearance);
        let input = stamp::Input {
            mesh,
            arch,
            teeth: sites,
            covered: Some(covered),
        };
        let base = std::cell::OnceCell::new();
        let flat = |stamped: &TriMesh| -> Result<(), String> {
            let base = base.get_or_init(|| {
                base_known.unwrap_or_else(|| Defects::of(&check(&land(mesh), None)))
            });
            let after = Defects::of(&check(&land(stamped), None));
            if after.no_worse_than(base) {
                return Ok(());
            }
            Err(format!(
                "the text would fold the surface ({} self-intersections, {} triangles of no area)",
                after
                    .self_intersections
                    .unwrap_or(0)
                    .saturating_sub(base.self_intersections.unwrap_or(0)),
                after.degenerate.saturating_sub(base.degenerate)
            ))
        };
        let why = match stamp::run_checked(&input, &params, &flat) {
            Ok(Outcome::Stamped(done)) => {
                let file = land(&done.mesh);
                return Some((done.mesh, file));
            }
            Ok(Outcome::Skipped { reason }) => reason,
            Err(e) => e.to_string(),
        };
        self.note(arch, k, aligner, "rpt-note-stamp", why);
        None
    }
}

/// What the print check finds wrong in a file that a later step (the
/// reduction, the stamp) must not make worse.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Defects {
    /// Pairs of triangles that cross.
    self_intersections: Option<usize>,
    /// Triangles of no area.
    degenerate: usize,
}

impl Defects {
    fn of(c: &printcheck::PrintCheck) -> Self {
        Self {
            self_intersections: c.self_intersections,
            degenerate: c.degenerate,
        }
    }

    fn no_worse_than(&self, base: &Self) -> bool {
        self.self_intersections <= base.self_intersections && self.degenerate <= base.degenerate
    }
}

/// The print check of the export (self-intersections, holes, the wall when
/// `min_wall` is given; no build plate).
fn check(mesh: &TriMesh, min_wall: Option<f64>) -> printcheck::PrintCheck {
    printcheck::check(
        mesh,
        &printcheck::CheckOptions {
            self_intersections: true,
            min_thickness_mm: min_wall,
            build_plate_mm: None,
            ..printcheck::CheckOptions::default()
        },
    )
}

/// The stamp of aligner `k` from the stamp of the case settings `case`: its
/// own number, the passive mark (`P`) of a passive aligner, its own date (the
/// first day of the aligner when the case asks for it, else the day of the
/// export), the case code (the plugin's `case_stamp.stamp_mesh`).
fn stamp_params(
    case: stamp::Params,
    options: &Options,
    (arch, k, passive): (Arch, u32, bool),
    kind: stamp::Kind,
    clearance: f64,
) -> stamp::Params {
    let date = options.stamp_dates.get(&(arch, k)).unwrap_or(&options.date);
    stamp::Params {
        number: k,
        passive,
        code: stamp::clean_code(&options.stamp_code),
        date: Some(date.to_iso()),
        clearance,
        kind,
        ..case
    }
}

/// The plan boxes of the ramps (their coordinates), grown by `reach`: the
/// occlusal shaping cuts nothing over them, so the ramps keep their height.
fn ramp_boxes(ramps: &[TriMesh], reach: f64) -> Vec<(DVec3, DVec3)> {
    ramps
        .iter()
        .filter(|r| !r.positions.is_empty())
        .map(|r| {
            let (lo, hi) = r.positions.iter().fold(
                (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                |(lo, hi), p| (lo.min(p.as_dvec3()), hi.max(p.as_dvec3())),
            );
            (lo - DVec3::splat(reach), hi + DVec3::splat(reach))
        })
        .collect()
}

impl ShellProvider<'_> {
    /// `Err(Cancelled)` once the export was stopped: asked between the steps
    /// of a mesh, so a stop does not wait for the whole mesh.
    fn stop(&self) -> Result<(), ExportError> {
        if self.options.cancel.is_cancelled() {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

impl MeshProvider for ShellProvider<'_> {
    fn model(&self, arch: Arch, k: u32) -> Result<Option<TriMesh>, ExportError> {
        let Some(print) = self.prints.get(&arch) else {
            return Ok(None);
        };
        self.stop()?;
        let g = print.stage(&self.data.stages, k);
        let outputs = Outputs {
            model: true,
            blockout: false,
            shell: false,
        };
        // reduced within the tolerance by the shell stage (no crossing in case
        // coordinates, the deviation rule of the plugin)
        let out = shell::run(&print.input(&g), &self.params, &self.design, outputs)
            .map_err(|e| package("model", arch, k, e))?;
        self.stop()?;
        let Some(model) = out.model else {
            return Ok(None);
        };
        if !self.options.stamp || k == 0 {
            return Ok(Some(model));
        }
        let to_arch = print.to_arch();
        let from_arch = dca_algo::frame::rigid_inverse(&to_arch);
        let Some(parts) = self.trim_parts(arch, &g, &to_arch, &[], None) else {
            return Ok(Some(model));
        };
        let sites = self.stamp_sites(&parts, &g, &to_arch);
        let local = rigid(&to_arch, &model);
        // back in case coordinates, every vertex the stamp left where it was
        // keeps its own position: going to the arch and back rounds it twice,
        // which can lay the three corners of a sliver in line or make two
        // triangles that nearly touch cross (SUP_modello_08 of a real case)
        let land = |s: &TriMesh| {
            let n = local.positions.len();
            let positions = s
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i < n && *p == local.positions[i] {
                        model.positions[i]
                    } else {
                        from_arch.transform_point3(p.as_dvec3()).as_vec3()
                    }
                })
                .collect();
            TriMesh::new(positions, s.tris.clone())
        };
        let base = Defects::of(&check(&model, None));
        let stamped = self.stamped(
            &local,
            Some(base),
            &land,
            (arch, k, false),
            stamp::Kind::Model,
            (sites, &print.covered),
        );
        Ok(Some(stamped.map_or(model, |(_, file)| file)))
    }

    fn aligner(&self, arch: Arch, k: u32) -> Result<Option<TriMesh>, ExportError> {
        let Some(print) = self.prints.get(&arch) else {
            return Ok(None);
        };
        self.stop()?;
        let g = print.stage(&self.data.stages, k);
        let to_arch = print.to_arch();
        let from_arch = dca_algo::frame::rigid_inverse(&to_arch);
        let err = |what: &str, e: &dyn std::fmt::Display| package(what, arch, k, e);
        let teeth: Vec<TriMesh> = g.teeth.iter().map(|t| rigid(&to_arch, t)).collect();

        // the ramps, in the arch coordinates of the shell
        let mut ramps_bite = Vec::new();
        let mut ramps = None;
        if let Some(bite_to_arch) = self.bite.to_arch(arch) {
            let solids = self.bite.ramps(arch, k);
            if !solids.is_empty() {
                let m = to_arch * dca_algo::frame::rigid_inverse(&bite_to_arch);
                ramps_bite = solids.iter().map(|r| r.mesh()).collect();
                let moved: Vec<_> = solids
                    .into_iter()
                    .map(|mut r| {
                        for v in &mut r.vertices {
                            *v = m.transform_point3(*v);
                        }
                        r
                    })
                    .collect();
                let by_fdi: BTreeMap<i64, TriMesh> = g
                    .fdis
                    .iter()
                    .zip(&teeth)
                    .map(|(f, t)| (i64::from(f.get()), t.clone()))
                    .collect();
                ramps = Some((moved, by_fdi));
            }
        }
        // the closed field of the model with the solids `extra` added (the
        // boxes of the bars over a space) and the untrimmed shell field read
        // from it by arithmetic, without the embrasure fills of the model (the
        // trim of a shell over them left holes through its wall at the
        // margin), the ramps united with it
        let fields = |extra: &[TriMesh]| -> Result<(_, _), ExportError> {
            let input = shell::Input {
                fdis: &[],
                ..print.input(&g)
            };
            let mut parts = shell::model_parts(&input, self.params.base_height_mm)
                .map_err(|e| err("model", &e))?;
            for e in extra {
                let off = parts.positions.len() as u32;
                parts.positions.extend_from_slice(&e.positions);
                parts.tris.extend(e.tris.iter().map(|t| t.map(|v| v + off)));
            }
            let closed = shell::closed_grid_for(
                &parts,
                self.params.shell_voxel_mm,
                self.shell_blockout,
                self.design.clearance_mm + self.design.thickness_mm,
            )
            .map_err(|e| err("shell", &e))?;
            drop(parts);
            self.stop()?;
            let mut field =
                shell::shell_grid(&closed, &self.design).map_err(|e| err("shell", &e))?;
            if let Some((moved, by_fdi)) = &ramps {
                let notes = dca_algo::ramps::apply_to_grid(&mut field, moved, by_fdi)
                    .map_err(|e| err("ramps", &e))?;
                for n in notes {
                    self.note(arch, k, true, "rpt-note-ramps", n);
                }
            }
            Ok((closed, field))
        };
        let (mut closed, field) = fields(&[])?;
        self.stop()?;

        // the occlusal thickness and the grinding of the bite: the solid above
        // the allowed top, cut from the field after the rim is rounded (nothing
        // over the ramps)
        let mut cut = None;
        if self.shape
            && let (Some(bite_to_arch), Some(b)) = (self.bite.to_arch(arch), self.bite_at(k))
        {
            let keep = ramp_boxes(&ramps_bite, occlusal::CELL + self.params.shell_voxel_mm);
            cut = occlusal::shape_cutter(
                &mut closed,
                self.design.clearance_mm,
                &b,
                !arch.is_upper(),
                &self.bite.covered(arch),
                self.bite.design(),
                &(bite_to_arch * from_arch),
                &keep,
                &field,
            )
            .map_err(|e| err("occlusal shaping", &e))?
            .map(|(c, _)| c);
        }
        drop(closed);
        self.stop()?;

        // the trim, in one piece: a space between two caps gets a bar, its box
        // added to the model and the shell field built again with it
        let Some(tp) = self.trim_parts(arch, &g, &to_arch, &teeth, Some(&print.covered)) else {
            return Ok(None);
        };
        let rebuild = |extra: &[TriMesh]| -> Result<_, String> {
            fields(extra).map(|(_, f)| f).map_err(|e| e.to_string())
        };
        let input = trim::Input {
            shell: &field,
            teeth_mesh: &tp.teeth_mesh,
            gingiva: &tp.gingiva,
            crowns: &tp.crowns,
            teeth: &tp.teeth,
            scan_border: &tp.scan_border,
            clearance_mm: self.design.clearance_mm,
            thickness_mm: self.design.thickness_mm,
            cut: cut.as_ref(),
            rebuild: Some(trim::Rebuild(&rebuild)),
        };
        let done = trim::run(&input, &self.trim).map_err(|e| {
            if self.options.cancel.is_cancelled() {
                ExportError::Cancelled
            } else if let trim::TrimError::Split(lost) = &e {
                ExportError::Package(super::messages::aligner_split(arch, k, lost))
            } else {
                err("trim", &e)
            }
        })?;
        drop(field);
        self.stop()?;
        if done.fallback {
            self.note(
                arch,
                k,
                true,
                "rpt-note-trim",
                format!("{:?} border used instead", done.mode).to_lowercase(),
            );
        }
        if done.genus > 0 {
            self.note(
                arch,
                k,
                true,
                "rpt-note-trim",
                format!("{} holes through the wall left", done.genus),
            );
        }
        if !done.bars.is_empty() {
            let pairs: Vec<String> = done
                .bars
                .iter()
                .map(|b| format!("{}-{}", b.teeth.0, b.teeth.1))
                .collect();
            self.note(arch, k, true, "rpt-note-bars", pairs.join(", "));
        }
        if !done.weak.is_empty() {
            let pairs: Vec<String> = done
                .weak
                .iter()
                .map(|w| format!("{}-{} ({} mm²)", w.a, w.b, w.mm2))
                .collect();
            self.note(arch, k, true, "rpt-note-weak", pairs.join(", "));
        }
        let (mut mesh, base) =
            self.reduce_checked(done.mesh, &from_arch, (arch, k), true, Some(MIN_WALL_MM))?;
        let land = |s: &TriMesh| rigid(&from_arch, s);
        let mut file = None;

        // the stamp, engraved inside the wall
        if self.options.stamp {
            // the centre of the arch from every crown, the tooth among those
            // the aligner covers (`case_stamp`)
            let sites = if g.fdis.iter().all(|f| print.covered.contains(f)) {
                self.stamp_sites(&tp, &g, &to_arch)
            } else {
                self.trim_parts(arch, &g, &to_arch, &[], None)
                    .map(|all| self.stamp_sites(&all, &g, &to_arch))
                    .unwrap_or_default()
            };
            // the last stage without its stamp: the passive aligners are
            // stamped on it, each with its own number and date
            if k == self.last(arch) {
                self.passive_base
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        (arch, k),
                        PassiveBase {
                            mesh: mesh.clone(),
                            base,
                            sites: sites.clone(),
                        },
                    );
            }
            if let Some((done, landed)) = self.stamped(
                &mesh,
                base,
                &land,
                (arch, k, false),
                stamp::Kind::Shell,
                (sites, &print.covered),
            ) {
                mesh = done;
                file = Some(landed);
            }
        }

        // no fin, no flap far from the teeth (EQUATOR: a straight or
        // scalloped border covers the gingiva on purpose)
        if self.options.checked && self.trim.mode == trim::Mode::Equator {
            let (teeth_mesh, face_fdi) = shell::reach_teeth(&print.input(&g));
            let outer = self.design.clearance_mm + self.design.thickness_mm;
            let reach =
                printcheck::shell_reach(&mesh.positions, &teeth_mesh, &face_fdi, outer, &[])
                    .map_err(|e| err("reach", &e))?;
            self.reaches
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((arch, k), reach);
        }
        Ok(Some(file.unwrap_or_else(|| land(&mesh))))
    }

    fn passive(&self, arch: Arch, last: u32, number: u32) -> Result<Option<TriMesh>, ExportError> {
        // no stamp: the file of the last stage, copied (the plugin)
        if !self.options.stamp {
            return Ok(None);
        }
        let Some(print) = self.prints.get(&arch) else {
            return Ok(None);
        };
        self.stop()?;
        let cached = |s: &Self| {
            s.passive_base
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&(arch, last))
        };
        // the last shell was not built by this provider: built once now
        if !cached(self) {
            self.aligner(arch, last)?;
            if !cached(self) {
                return Ok(None);
            }
        }
        let bases = self.passive_base.lock().unwrap_or_else(|e| e.into_inner());
        let b = &bases[&(arch, last)];
        let from_arch = dca_algo::frame::rigid_inverse(&print.to_arch());
        let land = |s: &TriMesh| rigid(&from_arch, s);
        let file = self
            .stamped(
                &b.mesh,
                b.base,
                &land,
                (arch, number, true),
                stamp::Kind::Shell,
                (b.sites.clone(), &print.covered),
            )
            .map_or_else(|| land(&b.mesh), |(_, file)| file);
        Ok(Some(file))
    }

    fn reach(&self, arch: Arch, k: u32) -> Option<printcheck::Reach> {
        self.reaches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(arch, k))
            .cloned()
    }

    fn notes(&self) -> Vec<Note> {
        let mut v = self.notes.lock().unwrap_or_else(|e| e.into_inner()).clone();
        v.sort();
        v.dedup();
        v
    }

    fn parallel_hint(&self) -> Option<usize> {
        // the fields of one build grow with the inverse square of the voxel
        // (a narrow band around the surfaces): about 3 GB at 0.1 mm
        let v = self.params.shell_voxel_mm;
        Some(((v / 0.1).powi(2) * 4.0).floor().clamp(2.0, 64.0) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passive_aligner_is_stamped_with_its_own_number_mark_and_date() {
        let day = |s: &str| Date::parse(s).unwrap();
        let options = Options {
            voxel_mm: 0.2,
            tolerance_mm: 0.02,
            stamp: true,
            stamp_code: "ab-12".to_owned(),
            date: day("2026-03-01"),
            stamp_dates: [
                ((Arch::Upper, 5), day("2026-03-29")),
                ((Arch::Upper, 6), day("2026-04-05")),
                ((Arch::Upper, 7), day("2026-04-12")),
            ]
            .into_iter()
            .collect(),
            cancel: dc_command::CancelToken::new(),
            checked: true,
        };
        let case = stamp::Params::default();
        let at = |k, passive| {
            stamp_params(
                case.clone(),
                &options,
                (Arch::Upper, k, passive),
                stamp::Kind::Shell,
                0.1,
            )
        };
        let last = at(5, false);
        let p6 = at(6, true);
        let p7 = at(7, true);
        assert_eq!((last.number, last.passive), (5, false));
        assert_eq!((p6.number, p6.passive), (6, true));
        assert_eq!((p7.number, p7.passive), (7, true));
        assert_eq!(last.date.as_deref(), Some("2026-03-29"));
        assert_eq!(p6.date.as_deref(), Some("2026-04-05"));
        assert_eq!(p7.date.as_deref(), Some("2026-04-12"));
        assert_eq!(p6.code, "AB-12");
        // the first line of the text: the plugin's `label_lines` ('SUP 21 P')
        let first = |p: &stamp::Params| {
            stamp::label_lines(&p.text, Arch::Upper, p.number, p.passive, "", &p.code).unwrap()[0]
                .clone()
        };
        assert_eq!(first(&last), "SUP 05");
        assert_eq!(first(&p6), "SUP 06 P");
        assert_eq!(first(&p7), "SUP 07 P");
        // no date of its own: the day of the export
        assert_eq!(at(9, true).date.as_deref(), Some("2026-03-01"));
    }
}
