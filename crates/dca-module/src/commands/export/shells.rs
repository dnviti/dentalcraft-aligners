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
//!   tolerance), then the stamp embossed on it (`k >= 1`; the initial model
//!   has none);
//! * **aligner** `k`: the closed field of the same parts (the blockout of the
//!   trim mode: 0.8 mm for the smooth border, `SMOOTH_BLOCKOUT_MM`), the
//!   shell field between the offsets at `clearance` and `clearance + wall`,
//!   the ramps united with it on the field and the cavity of their tooth cut
//!   again ([`dca_algo::ramps::apply_to_grid`]), the trim (cutter, reach of
//!   the crowns, rounded rim: [`dca_algo::trim::run`]), the reduction within
//!   the mesh tolerance, the occlusal thickness and the grinding of the bite
//!   with the aligners worn ([`occlusal::shape_shell`], when the case asks
//!   for it and both arches are there), then the stamp engraved inside the
//!   wall.
//!
//! Every step after the trim is checked as the print check would (self-
//! intersections, holes, thinnest wall): a reduction, an occlusal shaping or
//! a stamp that makes the file worse to print is left out and the report
//! says so (the reduction of the shell stage accepts a fold within the
//! tolerance; the local stamp and the occlusal shaping can fold a coarse
//! surface).
//!
//! Deviation: the plugin adds the ramps after the occlusal shaping, with mesh
//! booleans. Here they are united on the shell field before the trim (one
//! meshing, a closed surface by construction), and the vertices on the
//! ramps that the occlusal shaping would lower are put back where they were,
//! so the ramps keep their height as in the plugin.
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

/// Blockout of the shell under a smooth border (`SMOOTH_BLOCKOUT_MM`).
const SMOOTH_BLOCKOUT_MM: f64 = 0.8;
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

fn trim_mode(m: TrimMode) -> trim::Mode {
    match m {
        TrimMode::Equator => trim::Mode::Equator,
        TrimMode::Straight => trim::Mode::Straight,
        TrimMode::Scalloped => trim::Mode::Scalloped,
        TrimMode::Smooth => trim::Mode::Smooth,
    }
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
                    let a = if s.arch.is_upper() { "upper" } else { "lower" };
                    return Ok(Err(format!(
                        "the {a} arch has no closed teeth or no gingiva (run the reconstruction and the gingiva)"
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
                key: "rpt-note-bite",
                detail: why.to_owned(),
            });
        }
        let t = &data.trim;
        let trim = trim::Params {
            mode: trim_mode(t.mode),
            margin_gap_mm: t.margin_gap_mm,
            scallop_mm: t.scallop_mm,
            equator_offset_mm: t.equator_offset_mm,
            extension_mm: t.extension_mm,
            ..trim::Params::default()
        };
        let shell_blockout = if t.mode == TrimMode::Smooth {
            SMOOTH_BLOCKOUT_MM
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
        }))
    }

    fn note(&self, arch: Arch, k: u32, aligner: bool, key: &'static str, detail: String) {
        self.notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Note {
                arch: Some(arch),
                stage: Some(k),
                aligner,
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
    /// labelled scan moved to the stage, its crowns, margins and border; the
    /// teeth are the closed ones of the shell stage.
    fn trim_parts(
        &self,
        arch: Arch,
        g: &StageGeometry,
        to_arch: &DMat4,
        closed: &[TriMesh],
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
        let mut parts = trim::model::parts(&vertices, &scan.tris, series.labels, &frames, None);
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

    /// The reduction of `full` within the tolerance (or half of it) when it
    /// is no worse to print than `full`: no more self-intersections, the
    /// same holes, no wall thinner than `min_wall` that `full` does not have
    /// (the shell stage accepts a reduction on its distance alone: a fold
    /// within the tolerance passes). Otherwise the full resolution, with a
    /// note. Also the self-intersections of the mesh returned.
    fn reduce_checked(
        &self,
        full: TriMesh,
        arch: Arch,
        k: u32,
        aligner: bool,
        min_wall: Option<f64>,
    ) -> Result<(TriMesh, Option<usize>), ExportError> {
        let tol = self.params.tolerance_mm;
        if tol <= 0.0 {
            return Ok((full, None));
        }
        let base = check(&full, min_wall);
        let wall = |c: &printcheck::PrintCheck| c.min_thickness_mm.unwrap_or(f64::INFINITY);
        let floor = min_wall.map_or(0.0, |m| wall(&base).min(m));
        for t in [tol, 0.5 * tol] {
            let r = shell::reduce(&full, t).map_err(|e| package("reduction", arch, k, e))?;
            if r.tris.len() == full.tris.len() {
                break;
            }
            let c = check(&r, min_wall);
            if c.self_intersections <= base.self_intersections
                && c.genus == base.genus
                && c.boundary_edges == 0
                && (min_wall.is_none() || wall(&c) >= floor - 1e-9)
            {
                return Ok((r, c.self_intersections));
            }
        }
        self.note(arch, k, aligner, "rpt-note-reduce", format!("{tol} mm"));
        Ok((full, base.self_intersections))
    }

    /// The stamp on `mesh` (arch coordinates): the mesh stamped, or as it was
    /// with a note (no room, or the text would fold the surface: more
    /// self-intersections than `before`, the ones of `mesh` when known).
    fn stamped(
        &self,
        mesh: TriMesh,
        before: Option<usize>,
        arch: Arch,
        k: u32,
        kind: stamp::Kind,
        sites: Vec<ToothSite>,
    ) -> Result<TriMesh, ExportError> {
        let aligner = kind == stamp::Kind::Shell;
        let params = stamp::Params {
            number: k,
            code: stamp::clean_code(&self.options.stamp_code),
            date: Some(self.options.date.to_iso()),
            clearance: if aligner {
                self.design.clearance_mm
            } else {
                0.0
            },
            kind,
            ..stamp::Params::default()
        };
        let input = stamp::Input {
            mesh: &mesh,
            arch,
            teeth: sites,
        };
        match stamp::run(&input, &params) {
            Ok(Outcome::Stamped(done)) => {
                let before = before.or_else(|| check(&mesh, None).self_intersections);
                let after = check(&done.mesh, None).self_intersections;
                if after > before {
                    self.note(
                        arch,
                        k,
                        aligner,
                        "rpt-note-stamp",
                        format!(
                            "the text would fold the surface ({} self-intersections)",
                            after.unwrap_or(0) - before.unwrap_or(0)
                        ),
                    );
                    return Ok(mesh);
                }
                Ok(done.mesh)
            }
            Ok(Outcome::Skipped { reason }) => {
                self.note(arch, k, aligner, "rpt-note-stamp", reason);
                Ok(mesh)
            }
            Err(e) => {
                self.note(arch, k, aligner, "rpt-note-stamp", e.to_string());
                Ok(mesh)
            }
        }
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

/// Vertices of `shaped` that the occlusal shaping moved within the box of a
/// ramp (grown by `reach`) put back where they were in `before` (same
/// topology): the ramp and the wall it stands on keep their height.
fn keep_ramps(before: &TriMesh, shaped: &mut TriMesh, ramps: &[TriMesh], reach: f32) {
    let boxes: Vec<(glam::Vec3, glam::Vec3)> = ramps
        .iter()
        .filter(|r| !r.positions.is_empty())
        .map(|r| {
            let lo = r
                .positions
                .iter()
                .copied()
                .reduce(glam::Vec3::min)
                .unwrap_or_default();
            let hi = r
                .positions
                .iter()
                .copied()
                .reduce(glam::Vec3::max)
                .unwrap_or_default();
            (lo - glam::Vec3::splat(reach), hi + glam::Vec3::splat(reach))
        })
        .collect();
    for (p, q) in shaped.positions.iter_mut().zip(&before.positions) {
        if p != q
            && boxes
                .iter()
                .any(|(lo, hi)| q.cmpge(*lo).all() && q.cmple(*hi).all())
        {
            *p = *q;
        }
    }
}

impl MeshProvider for ShellProvider<'_> {
    fn model(&self, arch: Arch, k: u32) -> Result<Option<TriMesh>, ExportError> {
        let Some(print) = self.prints.get(&arch) else {
            return Ok(None);
        };
        let g = print.stage(&self.data.stages, k);
        let outputs = Outputs {
            model: true,
            blockout: false,
            shell: false,
        };
        let full_res = shell::Params {
            tolerance_mm: 0.0,
            ..self.params.clone()
        };
        let out = shell::run(&print.input(&g), &full_res, &self.design, outputs)
            .map_err(|e| package("model", arch, k, e))?;
        let Some(model) = out.model else {
            return Ok(None);
        };
        let (model, si) = self.reduce_checked(model, arch, k, false, None)?;
        if !self.options.stamp || k == 0 {
            return Ok(Some(model));
        }
        let to_arch = print.to_arch();
        let Some(parts) = self.trim_parts(arch, &g, &to_arch, &[]) else {
            return Ok(Some(model));
        };
        let sites = self.stamp_sites(&parts, &g, &to_arch);
        let local = rigid(&to_arch, &model);
        drop(model);
        let done = self.stamped(local, si, arch, k, stamp::Kind::Model, sites)?;
        Ok(Some(rigid(
            &dca_algo::frame::rigid_inverse(&to_arch),
            &done,
        )))
    }

    fn aligner(&self, arch: Arch, k: u32) -> Result<Option<TriMesh>, ExportError> {
        let Some(print) = self.prints.get(&arch) else {
            return Ok(None);
        };
        let g = print.stage(&self.data.stages, k);
        let to_arch = print.to_arch();
        let from_arch = dca_algo::frame::rigid_inverse(&to_arch);
        let err = |what: &str, e: &dyn std::fmt::Display| package(what, arch, k, e);
        // the untrimmed shell field
        let parts = shell::model_parts(&print.input(&g), self.params.base_height_mm)
            .map_err(|e| err("model", &e))?;
        let closed = shell::closed_grid(&parts, self.params.shell_voxel_mm, self.shell_blockout)
            .map_err(|e| err("shell", &e))?;
        let mut field = shell::shell_grid(&closed, &self.design).map_err(|e| err("shell", &e))?;
        drop(closed);
        let teeth: Vec<TriMesh> = g.teeth.iter().map(|t| rigid(&to_arch, t)).collect();

        // the ramps, on the field (arch coordinates of the bite → of the shell)
        let mut ramps_bite = Vec::new();
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
                let notes = dca_algo::ramps::apply_to_grid(&mut field, &moved, &by_fdi)
                    .map_err(|e| err("ramps", &e))?;
                for n in notes {
                    self.note(arch, k, true, "rpt-note-ramps", n);
                }
            }
        }

        // the trim
        let Some(tp) = self.trim_parts(arch, &g, &to_arch, &teeth) else {
            return Ok(None);
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
        };
        let done = trim::run(&input, &self.trim).map_err(|e| err("trim", &e))?;
        drop(field);
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
        let (mut mesh, mut si) =
            self.reduce_checked(done.mesh, arch, k, true, Some(MIN_WALL_MM))?;

        // the occlusal thickness and the grinding of the bite
        if self.shape
            && let (Some(bite_to_arch), Some(b)) = (self.bite.to_arch(arch), self.bite_at(k))
        {
            let to_bite = bite_to_arch * from_arch;
            let shell_b = rigid(&to_bite, &mesh);
            let model_b = rigid(&to_bite, &parts);
            let (mut shaped, _report) = occlusal::shape_shell(
                &shell_b,
                &model_b,
                &b,
                !arch.is_upper(),
                &self.bite.covered(arch),
                self.bite.design(),
            );
            if !ramps_bite.is_empty() {
                keep_ramps(
                    &shell_b,
                    &mut shaped,
                    &ramps_bite,
                    self.params.shell_voxel_mm as f32,
                );
            }
            let shaped = rigid(
                &(to_arch * dca_algo::frame::rigid_inverse(&bite_to_arch)),
                &shaped,
            );
            // the shaping may lower the outer surface through the fitting one
            // where the blockout keeps the fitting surface off the model: the
            // wall of the shell stays then, as with the occlusal mode `same`
            let before = si.or_else(|| check(&mesh, None).self_intersections);
            let c = check(&shaped, Some(MIN_WALL_MM));
            if c.self_intersections <= before && c.min_thickness_mm.is_none_or(|w| w >= MIN_WALL_MM)
            {
                si = c.self_intersections;
                mesh = shaped;
            } else {
                self.note(
                    arch,
                    k,
                    true,
                    "rpt-note-bite",
                    format!(
                        "the shaping would fold the wall ({} self-intersections, wall {:.2} mm)",
                        c.self_intersections.unwrap_or(0),
                        c.min_thickness_mm.unwrap_or(0.0)
                    ),
                );
            }
        }
        drop(parts);

        // the stamp, engraved inside the wall
        if self.options.stamp {
            let sites = self.stamp_sites(&tp, &g, &to_arch);
            mesh = self.stamped(mesh, si, arch, k, stamp::Kind::Shell, sites)?;
        }
        Ok(Some(rigid(&from_arch, &mesh)))
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
