//! Print geometry of a stage: the printable model, the blockout model and
//! the aligner shell before trimming.
//!
//! Reference: `ortho_aligner/aligner.py` (`gum_cast`, `gum_prisms`,
//! `_sdf_group`, `sdf_mesh`, `reduce_to_tolerance`) and
//! `ortho_aligner/case_print.py` (`stage_parts`, `model_solid`,
//! `_aligner_solid`); `gpu_sdf.py` is the step-by-step reference of the
//! distance-field operations.
//!
//! * **Model** of a stage: the union of the closed teeth at their stage pose
//!   and of the gingiva closed down to a flat base like a trimmed plaster
//!   cast ([`cast::gum_cast`]: a height field seen from the occlusal side;
//!   [`cast::gum_prisms`]: the exact scanned gingiva near the teeth), meshed
//!   from a signed distance field with the narrow crevices filled: the
//!   blockout of the plugin, a morphological closing by [`BLOCKOUT_MM`]
//!   (crevices narrower than twice it, which the sheet bridges anyway).
//! * **Shell** (aligner solid before trimming): the material between the
//!   offsets at `clearance` and `clearance + thickness` of that closed
//!   model, `max(S − outer, inner − S)` on the same field, so the shell is
//!   the shape of a sheet of uniform thickness laid on the model. Without
//!   the trim solid (B5) it is a closed hollow solid: an outer surface and
//!   the inner (fitting) surface around the whole model.
//! * **Blockout model**: the closed model with every undercut relative to
//!   the insertion axis filled, i.e. its sweep along the axis away from the
//!   occlusal side `{p : p + t·a ∈ M for some t ≥ 0}`, built exactly as the
//!   union of the model with a vertical prism under every face of it that
//!   faces the insertion direction (a vertical line meets it in one
//!   interval). The plugin leaves undercuts to the trim line (equator of
//!   every crown); this solid is the one a thermoforming workflow or a
//!   undercut-free check needs.
//!
//! Every output is one closed, manifold surface decimated within the mesh
//! tolerance of the plugin ([`REDUCE_TOL_MM`], the rule of
//! `reduce_to_tolerance`: 99.9 % of the vertices of the full-resolution
//! solid within it of the reduced one and back, none beyond
//! [`REDUCE_MAX_FACTOR`] times it; [`dc_geom::qem::decimate`] in parallel
//! slabs, [`reduce`]); a reduction that fails the rule or would not stay
//! watertight keeps the full resolution.
//!
//! Coordinates: the inputs and outputs are in case coordinates; the work is
//! done in arch coordinates (`+Z` occlusal, [`Input::to_arch`]) so that the
//! base of the cast is flat on the voxel grid.
//!
//! Deviations from the plugin (documented, not yet ported): the contact
//! bridges and embrasure fills of `stage_parts` (`contact_bridges`,
//! `embrasure_fills`) and `_tuck_below_margin` (the closed tooth drawn under
//! the gingiva below its margin); the closing still fills every contact
//! closer than twice [`BLOCKOUT_MM`]. The occlusal shaping and the ramps of
//! `aligner_solid` belong to the bite stages, the trim to B5.
//!
//! Outputs are generated on demand by [`run`] (nothing is cached in the
//! case: one stage of one arch is a few megabytes per solid, a full series
//! hundreds); the export calls [`run`] per stage and arch with the stage
//! inputs that [`stage_teeth`] places.

pub mod cast;

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use dc_case::Fdi;
use dc_geom::qem::{self, DecimateParams};
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use dca_model::Settings;
use glam::{DMat3, DMat4, DVec3, Vec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::frame;
use crate::teeth::Which;

/// Crevices narrower than twice this are filled (`aligner.BLOCKOUT_MM`).
pub const BLOCKOUT_MM: f64 = 0.3;
/// Default mesh tolerance of the print files (`aligner.REDUCE_TOL_MM`, the
/// Standard quality `mesh_tolerance`).
pub const REDUCE_TOL_MM: f64 = 0.01;
/// Voxel of the models (`export_voxel`, Standard quality).
pub const MODEL_VOXEL_MM: f64 = 0.1;
/// Voxel of the shells (`shell_voxel`, Standard quality).
pub const SHELL_VOXEL_MM: f64 = 0.1;
/// Height of the flat base below the lowest gingiva (`base_height`).
pub const BASE_HEIGHT_MM: f64 = 3.0;
/// Voxels of the band beyond the levels read (`SDF_BAND_EXTRA`).
pub const SDF_BAND_EXTRA: f64 = 3.0;
/// Crown points that give the plan distance of the cast are sampled this
/// coarse (`crown_points(obj, deltas, 0.8)`).
pub const CROWN_STEP_MM: f64 = 0.8;
/// No vertex of a reduced solid farther than this many tolerances from the
/// full resolution (`REDUCE_MAX_FACTOR`).
pub const REDUCE_MAX_FACTOR: f64 = 5.0;
/// Smallest reduced mesh worth the reduction (`REDUCE_MIN_TRIS`).
pub const REDUCE_MIN_TRIS: usize = 5000;
/// [`reduce`] works in slabs of about this many triangles.
pub const REDUCE_SLAB_TRIS: usize = 30_000;
/// At most this many slabs (`REDUCE_MAX_SLABS`).
pub const REDUCE_MAX_SLABS: usize = 64;
/// Rounds of [`reduce`] that hold the full resolution around the triangles
/// the reduction made cross each other, before the full resolution is kept.
pub const REDUCE_FIX_ROUNDS: usize = 4;
/// ... within this of the crossing triangles (mm).
pub const REDUCE_HOLD_MM: f64 = 0.5;
/// No vertex of a reduced solid gets more neighbours than this
/// ([`DecimateParams::max_valence`]): without it the flat base ends in fans of
/// dozens of thin triangles whose tolerance check takes most of the
/// reduction (about 1 % more triangles, a few times faster).
pub const REDUCE_MAX_VALENCE: usize = 16;

/// The sweep of the blockout is grown across the axis by the mesh tolerance
/// plus this many voxels ([`blockout_grid`]).
pub const SWEEP_GROW_VOXELS: f64 = 0.1;

/// Errors of the shell stage.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ShellError {
    /// The inputs or parameters are not usable.
    #[error("{0}")]
    Invalid(String),
    /// A distance-field or mesh operation failed.
    #[error("geometry: {0}")]
    Geometry(String),
}

fn geom(e: impl std::fmt::Display) -> ShellError {
    ShellError::Geometry(e.to_string())
}

/// Parameters of the print geometry. Thickness and clearance default to the
/// case settings ([`Params::design`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to build.
    pub arch: Which,
    /// Stages to build (aligner numbers, `0` the initial situation); empty:
    /// every stage of the series.
    pub stages: Vec<u32>,
    /// Wall thickness of the shell (mm); default the case setting.
    pub thickness_mm: Option<f64>,
    /// Internal clearance between model and shell (mm); default the case
    /// setting.
    pub clearance_mm: Option<f64>,
    /// Voxel of the model and blockout fields (mm).
    pub model_voxel_mm: f64,
    /// Voxel of the shell field (mm).
    pub shell_voxel_mm: f64,
    /// Crevices narrower than twice this are filled (mm).
    pub blockout_mm: f64,
    /// Height of the flat base below the lowest gingiva (mm).
    pub base_height_mm: f64,
    /// Mesh tolerance of the outputs (mm); `0` keeps the full resolution.
    pub tolerance_mm: f64,
    /// Insertion direction of the aligner in arch coordinates (the blockout
    /// fills what is hidden from it); default occlusal `+Z`.
    pub insertion_axis: [f64; 3],
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: Which::All,
            stages: Vec::new(),
            thickness_mm: None,
            clearance_mm: None,
            model_voxel_mm: MODEL_VOXEL_MM,
            shell_voxel_mm: SHELL_VOXEL_MM,
            blockout_mm: BLOCKOUT_MM,
            base_height_mm: BASE_HEIGHT_MM,
            tolerance_mm: REDUCE_TOL_MM,
            insertion_axis: [0.0, 0.0, 1.0],
        }
    }
}

/// Thickness and clearance of the shell actually used.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Design {
    /// Wall thickness (mm).
    pub thickness_mm: f64,
    /// Internal clearance (mm).
    pub clearance_mm: f64,
}

impl Params {
    /// The wall thickness and clearance: the parameters, else the settings.
    pub fn design(&self, settings: &Settings) -> Design {
        Design {
            thickness_mm: self.thickness_mm.unwrap_or(settings.shell_thickness_mm),
            clearance_mm: self.clearance_mm.unwrap_or(settings.clearance_mm),
        }
    }

    fn check(&self, design: &Design) -> Result<(), ShellError> {
        let bad = |what: &str, v: f64| {
            ShellError::Invalid(format!("{what} must be positive and finite, got {v}"))
        };
        for (what, v) in [
            ("model_voxel_mm", self.model_voxel_mm),
            ("shell_voxel_mm", self.shell_voxel_mm),
            ("thickness_mm", design.thickness_mm),
            ("base_height_mm", self.base_height_mm),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return Err(bad(what, v));
            }
        }
        for (what, v) in [
            ("blockout_mm", self.blockout_mm),
            ("tolerance_mm", self.tolerance_mm),
            ("clearance_mm", design.clearance_mm),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(ShellError::Invalid(format!(
                    "{what} must be zero or positive, got {v}"
                )));
            }
        }
        let a = DVec3::from(self.insertion_axis);
        if !(a.is_finite() && a.length() > 1e-9) {
            return Err(ShellError::Invalid(format!(
                "insertion_axis must be a non-zero vector, got {:?}",
                self.insertion_axis
            )));
        }
        Ok(())
    }
}

/// Which solids [`run`] builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outputs {
    /// The printable model.
    pub model: bool,
    /// The blockout model.
    pub blockout: bool,
    /// The aligner shell before trimming.
    pub shell: bool,
}

impl Outputs {
    /// Every solid.
    pub const ALL: Self = Self {
        model: true,
        blockout: true,
        shell: true,
    };
}

/// The geometry of one stage of one arch (case coordinates).
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Case → arch coordinates (origin on the occlusal plane, `+Z`
    /// occlusal for both arches).
    pub to_arch: DMat4,
    /// The closed teeth at their stage pose (watertight, outward).
    pub teeth: &'a [TriMesh],
    /// The gingiva at the stage (the scan gingiva or the gingiva without
    /// holes; open, possibly not clean).
    pub gingiva: &'a TriMesh,
    /// Per face of `gingiva`: under the crowns (the membrane, no prism);
    /// empty when none.
    pub cover: &'a [bool],
}

/// What [`run`] measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// Triangles of the model, the blockout and the shell (0: not built).
    pub faces: BTreeMap<String, usize>,
    /// Triangles before the reduction.
    pub full_faces: BTreeMap<String, usize>,
    /// Seconds per step.
    pub seconds: BTreeMap<String, f64>,
    /// Largest distance between the full-resolution and the reduced solid
    /// (mm, both ways).
    pub deviation_mm: BTreeMap<String, f64>,
    /// Wall thickness used (mm).
    pub thickness_mm: f64,
    /// Clearance used (mm).
    pub clearance_mm: f64,
}

/// The solids of a stage (case coordinates).
#[derive(Debug, Clone, Default)]
pub struct StageSolids {
    /// The printable model (teeth, gingiva cast, flat base).
    pub model: Option<TriMesh>,
    /// The model without undercuts relative to the insertion axis.
    pub blockout: Option<TriMesh>,
    /// The aligner shell before trimming.
    pub shell: Option<TriMesh>,
    /// Sizes and timings.
    pub report: Report,
}

fn transformed(mesh: &TriMesh, m: &DMat4) -> TriMesh {
    TriMesh::new(
        mesh.positions
            .iter()
            .map(|p| m.transform_point3(p.as_dvec3()).as_vec3())
            .collect(),
        mesh.tris.clone(),
    )
}

/// The closed teeth placed at a stage: every rest solid (case coordinates)
/// moved by its stage transform (`x' = D x`, case coordinates; a tooth
/// without one stays at rest).
pub fn stage_teeth(rest: &BTreeMap<Fdi, TriMesh>, deltas: &BTreeMap<Fdi, DMat4>) -> Vec<TriMesh> {
    rest.iter()
        .map(|(f, m)| match deltas.get(f) {
            Some(d) => transformed(m, d),
            None => m.clone(),
        })
        .collect()
}

/// Plan positions of the crowns, one per [`CROWN_STEP_MM`] cell of the
/// teeth (arch coordinates).
fn crown_xy(teeth: &[Vec<DVec3>]) -> Vec<[f64; 2]> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in teeth {
        for p in t {
            let key = (p / CROWN_STEP_MM).floor().as_i64vec3();
            if seen.insert(key) {
                out.push([p.x, p.y]);
            }
        }
    }
    out
}

/// The parts of the model as one triangle soup in arch coordinates
/// (`stage_parts`): the gingiva cast, the prisms of the exact gingiva and
/// the closed teeth. Their union is the model (overlapping closed parts).
pub fn model_parts(input: &Input<'_>, base_height: f64) -> Result<TriMesh, ShellError> {
    if input.gingiva.tris.is_empty() {
        return Err(ShellError::Invalid("the gingiva is empty".into()));
    }
    if input.teeth.iter().all(|t| t.tris.is_empty()) {
        return Err(ShellError::Invalid("no closed tooth".into()));
    }
    if !input.cover.is_empty() && input.cover.len() != input.gingiva.tris.len() {
        return Err(ShellError::Invalid(format!(
            "cover has {} entries for {} gingiva faces",
            input.cover.len(),
            input.gingiva.tris.len()
        )));
    }
    let a = input.to_arch;
    let vg: Vec<DVec3> = input
        .gingiva
        .positions
        .iter()
        .map(|p| a.transform_point3(p.as_dvec3()))
        .collect();
    let teeth: Vec<Vec<DVec3>> = input
        .teeth
        .iter()
        .map(|t| {
            t.positions
                .iter()
                .map(|p| a.transform_point3(p.as_dvec3()))
                .collect()
        })
        .collect();
    let (mut prisms, mut cover) = (Vec::new(), Vec::new());
    for (f, t) in input.gingiva.tris.iter().enumerate() {
        if input.cover.get(f).copied().unwrap_or(false) {
            cover.push(*t);
        } else {
            prisms.push(*t);
        }
    }
    let params = cast::CastParams {
        base_height,
        ..cast::CastParams::default()
    };
    let c = cast::gum_cast(
        &vg,
        &input.gingiva.tris,
        &crown_xy(&teeth),
        &params,
        &prisms,
        &cover,
    );
    let (pv, pt) = cast::gum_prisms(&vg, &c.info);
    let mut positions: Vec<Vec3> = c.positions.iter().map(|p| p.as_vec3()).collect();
    let mut tris = c.tris.clone();
    let mut append = |v: &mut dyn Iterator<Item = Vec3>, t: &[[u32; 3]]| {
        let off = positions.len() as u32;
        positions.extend(v);
        tris.extend(t.iter().map(|f| f.map(|i| i + off)));
    };
    append(&mut pv.iter().map(|p| p.as_vec3()), &pt);
    for (pts, mesh) in teeth.iter().zip(input.teeth) {
        append(&mut pts.iter().map(|p| p.as_vec3()), &mesh.tris);
    }
    Ok(TriMesh::new(positions, tris))
}

fn band_for(voxel: f64, reach: f64) -> f32 {
    (((reach / voxel).ceil() + SDF_BAND_EXTRA) * voxel) as f32
}

/// The closed field of the model (`_sdf_group`: the closing by `blockout`
/// of the union of the parts, [`SdfGrid::from_mesh_closed`]: the dilated
/// solid sampled again from its surface, so the pockets the parts enclose
/// with a mouth narrower than `2 * blockout` are filled). Its band reaches
/// the levels up to `blockout`; [`closed_grid_for`] widens it for a shell.
pub fn closed_grid(parts: &TriMesh, voxel: f64, blockout: f64) -> Result<SdfGrid, ShellError> {
    closed_grid_for(parts, voxel, blockout, 0.0)
}

/// [`closed_grid`] with a band that also reaches the level `reach` (mm), so
/// that [`shell_grid`] reads a shell up to it (`clearance + thickness`)
/// from the same field without sampling it again.
pub fn closed_grid_for(
    parts: &TriMesh,
    voxel: f64,
    blockout: f64,
    reach: f64,
) -> Result<SdfGrid, ShellError> {
    let params = SdfParams {
        voxel_size: voxel as f32,
        band: band_for(voxel, blockout.max(reach)),
    };
    SdfGrid::from_mesh_closed(parts, &params, blockout as f32).map_err(geom)
}

/// The shell field `max(S − outer, inner − S)` of a closed field: the
/// material between its levels at `clearance` and `clearance + thickness`
/// (`_sdf_group` with a shell: plain arithmetic on the field, its outward
/// levels exact offsets of the closed surface). A band that does not reach
/// the outer level is widened first: the field is sampled again from its
/// highest exact level set.
pub fn shell_grid(closed: &SdfGrid, design: &Design) -> Result<SdfGrid, ShellError> {
    let inner = design.clearance_mm as f32;
    let outer = (design.clearance_mm + design.thickness_mm) as f32;
    let vs = closed.voxel_size;
    let margin = 1.5 * vs;
    if outer + margin <= closed.band() && margin - inner <= closed.band() {
        return closed.shell(inner, outer).map_err(geom);
    }
    // the outer side from the field of a level set as high as the band allows
    let level = (closed.band() - 2.0 * vs).max(0.0);
    let surface = closed.to_mesh_at(level).map_err(geom)?;
    if surface.tris.is_empty() {
        return SdfGrid::empty(&SdfParams {
            voxel_size: vs,
            band: closed.band(),
        })
        .map_err(geom);
    }
    let params = SdfParams {
        voxel_size: vs,
        band: band_for(f64::from(vs), f64::from(outer - level)),
    };
    let mut out = SdfGrid::from_mesh(&surface, &params).map_err(geom)?;
    drop(surface);
    out.shift(outer - level).map_err(geom)?;
    let mut fitting = closed.clone();
    fitting.shift(inner).map_err(geom)?;
    out.subtract(&fitting).map_err(geom)?;
    Ok(out)
}

/// A rotation taking `axis` to `+Z`.
fn rotation_to_z(axis: DVec3) -> DMat3 {
    let a = axis.normalize();
    let q = glam::DQuat::from_rotation_arc(a, DVec3::Z);
    DMat3::from_quat(q)
}

/// The sweep of the closed surface `model` along `−axis` down to its lowest
/// point: a closed vertical prism under every face that faces `axis`
/// (arch coordinates). Its union with the model has no undercut.
///
/// `shift` moves the prisms across the axis (mm; `0` for none: the walls
/// between two prisms lie inside the sweep, and [`SdfGrid::from_mesh`] keeps
/// a wall through voxel centres inside).
///
/// `grow` moves every vertex outwards across the axis by that much times
/// the part of its normal across the axis (mm): `model` may lie a little
/// inside the solid it stands for (a reduced mesh, the iso-surface of a
/// grid), and the sliver of a wall leaning over by less than that would be
/// left as an undercut as tall as the wall. Faces turned towards the axis
/// barely move.
pub fn sweep_prisms(model: &TriMesh, axis: DVec3, shift: f64, grow: f64) -> TriMesh {
    let r = rotation_to_z(axis);
    let rt = r.transpose();
    let jitter = DVec3::new(0.37 * shift, 0.61 * shift, 0.0);
    let normals = model.compute_vertex_normals();
    let v: Vec<DVec3> = model
        .positions
        .iter()
        .zip(&normals)
        .map(|(p, n)| {
            let n = r * n.as_dvec3();
            r * p.as_dvec3() + jitter + DVec3::new(n.x, n.y, 0.0) * grow
        })
        .collect();
    let floor = v.iter().map(|p| p.z).fold(f64::INFINITY, f64::min) - 0.05;
    let up: Vec<[u32; 3]> = model
        .tris
        .iter()
        .filter(|t| {
            let [a, b, c] = t.map(|i| v[i as usize]);
            (b - a).cross(c - a).z > 1e-12
        })
        .copied()
        .collect();
    let mut positions = Vec::with_capacity(6 * up.len());
    let mut tris = Vec::with_capacity(8 * up.len());
    for t in &up {
        let off = positions.len() as u32;
        let top = t.map(|i| v[i as usize]);
        for p in top {
            positions.push((rt * p).as_vec3());
        }
        for p in top {
            positions.push((rt * DVec3::new(p.x, p.y, floor)).as_vec3());
        }
        let (a, b, c) = (off, off + 1, off + 2);
        let (ab, bb, cb) = (off + 3, off + 4, off + 5);
        tris.extend_from_slice(&[
            [a, b, c],
            [ab, cb, bb],
            [a, ab, bb],
            [a, bb, b],
            [b, bb, cb],
            [b, cb, c],
            [c, cb, ab],
            [c, ab, a],
        ]);
    }
    TriMesh::new(positions, tris)
}

/// The blockout field: the closed field united with its sweep along `−axis`
/// down to the lowest point of its surface `model` (no undercut relative to
/// `axis`). Along the grid axis `+Z` (the occlusal direction of the arch
/// frame) the sweep is taken on the field itself, column by column
/// ([`SdfGrid::sweep_down`]); along another axis it is the field of the
/// prisms under `model` ([`sweep_prisms`]), which lies within `tol` of the
/// zero level of `closed` (a reduced mesh of it): the prisms are grown
/// across the axis by that plus [`SWEEP_GROW_VOXELS`].
pub fn blockout_grid(
    closed: &SdfGrid,
    model: &TriMesh,
    axis: DVec3,
    tol: f64,
) -> Result<SdfGrid, ShellError> {
    let voxel = f64::from(closed.voxel_size);
    let a = axis.normalize();
    if a.z >= 1.0 - 1e-12 {
        let mut g = closed.clone();
        let floor = model
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f32::INFINITY, f32::min);
        if floor.is_finite() {
            g.sweep_down(floor).map_err(geom)?;
        }
        return Ok(g);
    }
    let grow = tol + SWEEP_GROW_VOXELS * voxel;
    let sweep = sweep_prisms(model, axis, 0.0, grow);
    let mut g = closed.clone();
    if sweep.tris.is_empty() {
        return Ok(g);
    }
    let params = SdfParams {
        voxel_size: closed.voxel_size,
        band: band_for(voxel, 0.0),
    };
    let s = SdfGrid::from_mesh(&sweep, &params).map_err(geom)?;
    g.union(&s).map_err(geom)?;
    Ok(g)
}

/// The fewest triangles within `tol` of `mesh` (both ways), still
/// watertight and crossing itself nowhere (the full resolution is held
/// around any crossing the reduction makes, [`REDUCE_FIX_ROUNDS`]); the full
/// mesh when the reduction is not (or `tol` is 0).
///
/// The exact reduction ([`dc_geom::qem::decimate`] with a tolerance) costs
/// more than linearly in the triangles, so a large mesh is reduced in slabs
/// of about [`REDUCE_SLAB_TRIS`] triangles at the same time
/// (`REDUCE_CHUNK_TRIS` of the plugin), the seams between them held at full
/// resolution ([`REDUCE_MAX_SLABS`] at most).
pub fn reduce(mesh: &TriMesh, tol: f64) -> Result<TriMesh, ShellError> {
    reduce_to(mesh, tol, None)
}

/// [`reduce`] of a mesh that is then moved by `to_out` (a rigid transform):
/// the crossings are looked for where the vertices land, since rounding
/// them may make two triangles that nearly touch cross.
pub fn reduce_to(mesh: &TriMesh, tol: f64, to_out: Option<&DMat4>) -> Result<TriMesh, ShellError> {
    if tol <= 0.0 || mesh.tris.len() < REDUCE_MIN_TRIS {
        return Ok(mesh.clone());
    }
    let n = mesh
        .tris
        .len()
        .div_ceil(REDUCE_SLAB_TRIS)
        .clamp(1, REDUCE_MAX_SLABS);
    // the reduction may fold a triangle through another one at a sharp crease
    // (the full resolution never crosses itself): the faces of the full
    // resolution around every crossing are held and the reduction is done again
    let mut held = vec![false; mesh.tris.len()];
    let mut bvh = None;
    for _ in 0..REDUCE_FIX_ROUNDS {
        let out = reduce_slabs(mesh, tol, n, &held)?;
        if !out.validate().is_watertight() {
            break;
        }
        let pairs = match to_out {
            Some(m) => crossing_pairs(&transformed(&out, m)),
            None => crossing_pairs(&out),
        };
        if pairs.is_empty() {
            return Ok(out);
        }
        let bvh = match &bvh {
            Some(b) => b,
            None => bvh.insert(dc_geom::bvh::Bvh::build(mesh).map_err(geom)?),
        };
        let r = Vec3::splat(REDUCE_HOLD_MM as f32);
        for f in pairs.iter().flat_map(|&(a, b)| [a, b]) {
            let t = out.tris[f as usize].map(|i| out.positions[i as usize]);
            let lo = t[0].min(t[1]).min(t[2]) - r;
            let hi = t[0].max(t[1]).max(t[2]) + r;
            for g in bvh.overlap_aabb(lo, hi) {
                held[g as usize] = true;
            }
        }
    }
    Ok(mesh.clone())
}

/// Pairs `(i, j)`, `i < j`, of triangles of `mesh` that share no vertex and
/// cross each other (an edge of one meets the other), sorted.
pub fn crossing_pairs(mesh: &TriMesh) -> Vec<(u32, u32)> {
    let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let tris = &mesh.tris;
    let corners = |t: [u32; 3]| t.map(|i| v[i as usize]);
    let boxes: Vec<(DVec3, DVec3)> = tris
        .iter()
        .map(|&t| {
            let [a, b, c] = corners(t);
            (a.min(b).min(c), a.max(b).max(c))
        })
        .collect();
    let mut order: Vec<u32> = (0..tris.len() as u32).collect();
    order.sort_unstable_by(|&a, &b| boxes[a as usize].0.x.total_cmp(&boxes[b as usize].0.x));
    let mut pairs: Vec<(u32, u32)> = (0..order.len())
        .into_par_iter()
        .with_min_len(512)
        .flat_map_iter(|k| {
            let i = order[k] as usize;
            let (lo, hi) = boxes[i];
            let mut out = Vec::new();
            for &j in &order[k + 1..] {
                let j = j as usize;
                let (lo2, hi2) = boxes[j];
                if lo2.x > hi.x {
                    break;
                }
                if lo2.y > hi.y || hi2.y < lo.y || lo2.z > hi.z || hi2.z < lo.z {
                    continue;
                }
                let (ta, tb) = (tris[i], tris[j]);
                if ta.iter().any(|x| tb.contains(x)) {
                    continue;
                }
                let (a, b) = (corners(ta), corners(tb));
                let crosses = |x: &[DVec3; 3], y: &[DVec3; 3]| {
                    (0..3).any(|k| segment_crosses_triangle(x[k], x[(k + 1) % 3], y))
                };
                if crosses(&a, &b) || crosses(&b, &a) {
                    out.push((i.min(j) as u32, i.max(j) as u32));
                }
            }
            out
        })
        .collect();
    pairs.sort_unstable();
    pairs
}

fn segment_crosses_triangle(p: DVec3, q: DVec3, t: &[DVec3; 3]) -> bool {
    let d = q - p;
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det.abs() <= 1e-18 {
        return false;
    }
    let inv = 1.0 / det;
    let s = p - t[0];
    let u = s.dot(pv) * inv;
    let qv = s.cross(e1);
    let w = d.dot(qv) * inv;
    let h = e2.dot(qv) * inv;
    u >= 0.0 && w >= 0.0 && u + w <= 1.0 && (0.0..=1.0).contains(&h)
}

/// The exact reduction within `tol` in `n` slabs along the longest side of
/// the box, at the same time: each slab is decimated with a ring of the
/// faces of its neighbours under another label and its own faces on the
/// seam under labels of their own, so the vertices there neither move nor
/// disappear, and the slabs are welded back on them.
fn reduce_slabs(mesh: &TriMesh, tol: f64, n: usize, held: &[bool]) -> Result<TriMesh, ShellError> {
    if n <= 1 && !held.contains(&true) {
        let params = DecimateParams {
            preserve_labels: false,
            max_valence: REDUCE_MAX_VALENCE,
            ..DecimateParams::to_tolerance(tol)
        };
        let plain = TriMesh::new(mesh.positions.clone(), mesh.tris.clone());
        return Ok(qem::decimate(&plain, &params).map_err(geom)?.mesh);
    }
    const OWN: u16 = 1;
    const RING: u16 = 2;
    const BAND0: u16 = 2;
    let size = mesh.aabb().map_or(Vec3::X, |b| b.size());
    let axis = if size.x >= size.y && size.x >= size.z {
        0
    } else if size.y >= size.z {
        1
    } else {
        2
    };
    let key = |t: &[u32; 3]| -> f32 { t.iter().map(|&i| mesh.positions[i as usize][axis]).sum() };
    let mut order: Vec<usize> = (0..mesh.tris.len()).collect();
    order.sort_by(|&a, &b| {
        key(&mesh.tris[a])
            .total_cmp(&key(&mesh.tris[b]))
            .then(a.cmp(&b))
    });
    let n = n.max(1);
    let mut slab = vec![0usize; mesh.tris.len()];
    for (rank, &f) in order.iter().enumerate() {
        slab[f] = rank * n / order.len();
    }
    // seam vertices: used by the faces of more than one slab
    const NONE: usize = usize::MAX;
    const MANY: usize = usize::MAX - 1;
    let mut owner = vec![NONE; mesh.positions.len()];
    for (t, &k) in mesh.tris.iter().zip(&slab) {
        for &i in t {
            let o = &mut owner[i as usize];
            *o = if *o == NONE || *o == k { k } else { MANY };
        }
    }
    let parts: Vec<Result<TriMesh, ShellError>> = (0..n)
        .into_par_iter()
        .map(|s| {
            let mut used = vec![false; mesh.positions.len()];
            for (t, _) in mesh.tris.iter().zip(&slab).filter(|(_, k)| **k == s) {
                for &i in t {
                    used[i as usize] = true;
                }
            }
            let mut local = vec![u32::MAX; mesh.positions.len()];
            let mut positions = Vec::new();
            let mut tris = Vec::new();
            let mut labels = Vec::new();
            let mut band = 0u16;
            for (f, (t, &k)) in mesh.tris.iter().zip(&slab).enumerate() {
                let own = k == s;
                if !own && !t.iter().any(|&i| used[i as usize]) {
                    continue;
                }
                tris.push(t.map(|i| {
                    let l = &mut local[i as usize];
                    if *l == u32::MAX {
                        *l = positions.len() as u32;
                        positions.push(mesh.positions[i as usize]);
                    }
                    *l
                }));
                // own faces on the seam (and the held ones): each its own label, so
                // that every vertex of the band is held and no collapse makes a new
                // edge between two seam vertices (the neighbour slab could make the
                // same one)
                let label = if !own {
                    RING
                } else if held[f] || t.iter().any(|&i| owner[i as usize] == MANY) {
                    band = band % (u16::MAX - BAND0) + 1;
                    BAND0 + band
                } else {
                    OWN
                };
                labels.push(label);
            }
            let mut sub = TriMesh::new(positions, tris);
            sub.face_labels = Some(labels);
            let params = DecimateParams {
                preserve_labels: true,
                max_valence: REDUCE_MAX_VALENCE,
                ..DecimateParams::to_tolerance(tol)
            };
            let out = qem::decimate(&sub, &params).map_err(geom)?.mesh;
            let keep = out.face_labels.clone().unwrap_or_default();
            let tris = out
                .tris
                .iter()
                .zip(&keep)
                .filter(|(_, l)| **l != RING)
                .map(|(t, _)| *t)
                .collect();
            Ok(TriMesh::new(out.positions, tris))
        })
        .collect();
    // weld the slabs on their seams (the same positions, bit for bit)
    let mut index = std::collections::HashMap::new();
    let mut positions = Vec::new();
    let mut tris = Vec::new();
    for part in parts {
        let part = part?;
        let map: Vec<u32> = part
            .positions
            .iter()
            .map(|p| {
                *index
                    .entry(p.to_array().map(f32::to_bits))
                    .or_insert_with(|| {
                        positions.push(*p);
                        (positions.len() - 1) as u32
                    })
            })
            .collect();
        tris.extend(part.tris.iter().map(|t| t.map(|i| map[i as usize])));
    }
    // vertices of dropped ring faces are not referenced: compact
    let mut used = vec![u32::MAX; positions.len()];
    let mut compact = Vec::new();
    for t in &mut tris {
        for i in t.iter_mut() {
            if used[*i as usize] == u32::MAX {
                used[*i as usize] = compact.len() as u32;
                compact.push(positions[*i as usize]);
            }
            *i = used[*i as usize];
        }
    }
    Ok(TriMesh::new(compact, tris))
}

/// A reduced solid with its sizes and its measured deviation.
struct Finished {
    mesh: TriMesh,
    full_faces: usize,
    deviation: f64,
}

impl Finished {
    fn record(&self, name: &str, secs: f64, report: &mut Report) {
        report.faces.insert(name.into(), self.mesh.tris.len());
        report.full_faces.insert(name.into(), self.full_faces);
        report.deviation_mm.insert(name.into(), self.deviation);
        report.seconds.insert(name.into(), secs);
    }
}

/// The `n` largest connected pieces of a mesh (by faces), the vertices
/// compacted (`largest_piece` of the plugin): the specks the mesher leaves
/// outside and the surfaces of the cavities the closing encloses inside the
/// solid are dropped.
pub fn largest_pieces(mesh: &TriMesh, n: usize) -> TriMesh {
    let comps = mesh.connected_components();
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    let mut tris = Vec::new();
    for c in comps.iter().take(n) {
        for &f in c {
            tris.push(mesh.tris[f as usize].map(|i| {
                let r = &mut remap[i as usize];
                if *r == u32::MAX {
                    *r = positions.len() as u32;
                    positions.push(mesh.positions[i as usize]);
                }
                *r
            }));
        }
    }
    TriMesh::new(positions, tris)
}

/// Reduces a full-resolution solid (its `pieces` largest pieces) and
/// measures the deviation. The reduction is accepted as the plugin's
/// `reduce_to_tolerance` does: 99.9 % of the vertices (both ways) within the
/// tolerance and none beyond [`REDUCE_MAX_FACTOR`] times it (the exact
/// reduction keeps all of them within it); otherwise it is done again
/// within half the tolerance, and then the full resolution kept.
fn finish(full: TriMesh, pieces: usize, tol: f64, to_out: &DMat4) -> Result<Finished, ShellError> {
    let full = largest_pieces(&full, pieces);
    let mut try_tol = tol;
    for _ in 0..2 {
        let mesh = reduce_to(&full, try_tol, Some(to_out))?;
        if mesh.tris.len() == full.tris.len() {
            break;
        }
        let (max, p999) = deviation_stats(&full, &mesh)?;
        if p999 <= tol && max <= REDUCE_MAX_FACTOR * tol {
            return Ok(Finished {
                mesh,
                full_faces: full.tris.len(),
                deviation: max,
            });
        }
        try_tol *= 0.5;
    }
    Ok(Finished {
        full_faces: full.tris.len(),
        mesh: full,
        deviation: 0.0,
    })
}

/// Builds the solids of one stage of one arch (see the module docs).
///
/// # Errors
/// [`ShellError::Invalid`] for empty inputs or bad parameters,
/// [`ShellError::Geometry`] when a field cannot be built.
pub fn run(
    input: &Input<'_>,
    params: &Params,
    design: &Design,
    outputs: Outputs,
) -> Result<StageSolids, ShellError> {
    params.check(design)?;
    let mut report = Report {
        thickness_mm: design.thickness_mm,
        clearance_mm: design.clearance_mm,
        ..Report::default()
    };
    let t = Instant::now();
    let parts = model_parts(input, params.base_height_mm)?;
    report
        .seconds
        .insert("parts".into(), t.elapsed().as_secs_f64());
    let from_arch = frame::rigid_inverse(&input.to_arch);
    let tol = params.tolerance_mm;
    let mut out = StageSolids::default();

    let need_model_grid = outputs.model || outputs.blockout;
    let same_voxel = (params.model_voxel_mm - params.shell_voxel_mm).abs() < 1e-12;
    let reach = design.clearance_mm + design.thickness_mm;
    let t = Instant::now();
    let model_grid = if need_model_grid || (outputs.shell && same_voxel) {
        // one field for the three solids: its band reaches the outer level of the shell
        let reach = if outputs.shell && same_voxel {
            reach
        } else {
            0.0
        };
        Some(closed_grid_for(
            &parts,
            params.model_voxel_mm,
            params.blockout_mm,
            reach,
        )?)
    } else {
        None
    };
    report
        .seconds
        .insert("closing".into(), t.elapsed().as_secs_f64());
    let model_full = match model_grid.as_ref().filter(|_| need_model_grid) {
        Some(g) => Some(largest_pieces(&g.to_mesh().map_err(geom)?, 1)),
        None => None,
    };
    let axis = DVec3::from(params.insertion_axis);
    // the sweep along the grid axis needs only the lowest point of the model
    let sweep_on_grid = axis.normalize().z >= 1.0 - 1e-12;

    let model_job = || -> Result<Option<(Finished, f64)>, ShellError> {
        let Some(full) = &model_full else {
            return Ok(None);
        };
        let t = Instant::now();
        let solid = finish(full.clone(), 1, tol, &from_arch)?;
        Ok(Some((solid, t.elapsed().as_secs_f64())))
    };
    let shell_job = || -> Result<Option<(Finished, f64)>, ShellError> {
        if !outputs.shell {
            return Ok(None);
        }
        let t = Instant::now();
        let own;
        let closed = match (&model_grid, same_voxel) {
            (Some(g), true) => g,
            _ => {
                own = closed_grid_for(&parts, params.shell_voxel_mm, params.blockout_mm, reach)?;
                &own
            }
        };
        // the untrimmed shell: the outer surface and the fitting surface
        let solid = finish(
            shell_grid(closed, design)?.to_mesh().map_err(geom)?,
            2,
            tol,
            &from_arch,
        )?;
        Ok(Some((solid, t.elapsed().as_secs_f64())))
    };
    let blockout_job = |model: Option<&TriMesh>| -> Result<Option<(Finished, f64)>, ShellError> {
        let (true, Some(g), Some(model)) = (outputs.blockout, &model_grid, model) else {
            return Ok(None);
        };
        let t = Instant::now();
        let solid = finish(
            blockout_grid(g, model, axis, tol)?
                .to_mesh()
                .map_err(geom)?,
            1,
            tol,
            &from_arch,
        )?;
        Ok(Some((solid, t.elapsed().as_secs_f64())))
    };
    // the three solids are independent (the prisms of a tilted sweep stand on the
    // reduced model): built at the same time
    let ((model, blockout), shell) = rayon::join(
        || {
            if sweep_on_grid {
                rayon::join(model_job, || blockout_job(model_full.as_ref()))
            } else {
                let model = model_job();
                let reduced = model
                    .as_ref()
                    .ok()
                    .and_then(|m| m.as_ref().map(|m| &m.0.mesh));
                let blockout = blockout_job(reduced);
                (model, blockout)
            }
        },
        shell_job,
    );
    let model = model?;
    if let Some((solid, secs)) = &model {
        solid.record("model", *secs, &mut report);
    }
    if let Some((solid, secs)) = shell? {
        solid.record("shell", secs, &mut report);
        out.shell = Some(transformed(&solid.mesh, &from_arch));
    }
    if let Some((solid, secs)) = blockout? {
        solid.record("blockout", secs, &mut report);
        out.blockout = Some(transformed(&solid.mesh, &from_arch));
    }
    if outputs.model {
        out.model = model.map(|(m, _)| transformed(&m.mesh, &from_arch));
    }
    out.report = report;
    Ok(out)
}

/// Distances from the vertices of either mesh to the surface of the other
/// (both ways; vertices used by no triangle are left out): the largest and
/// the 99.9th percentile.
pub fn deviation_stats(a: &TriMesh, b: &TriMesh) -> Result<(f64, f64), ShellError> {
    let one_way = |from: &TriMesh, to: &TriMesh| -> Result<Vec<f32>, ShellError> {
        let bvh = dc_geom::bvh::Bvh::build(to).map_err(geom)?;
        let mut used = vec![false; from.positions.len()];
        for t in &from.tris {
            for &i in t {
                used[i as usize] = true;
            }
        }
        let pts: Vec<Vec3> = from
            .positions
            .iter()
            .zip(&used)
            .filter(|(_, u)| **u)
            .map(|(p, _)| *p)
            .collect();
        Ok(bvh
            .closest_points(&pts, f32::INFINITY)
            .iter()
            .map(|h| h.map_or(f32::INFINITY, |h| h.distance))
            .collect())
    };
    let mut d = one_way(a, b)?;
    d.extend(one_way(b, a)?);
    if d.is_empty() {
        return Ok((0.0, 0.0));
    }
    d.sort_unstable_by(f32::total_cmp);
    let p999 = d[((d.len() - 1) as f64 * 0.999).round() as usize];
    Ok((f64::from(d[d.len() - 1]), f64::from(p999)))
}

/// Largest distance from a vertex of either mesh to the surface of the
/// other ([`deviation_stats`]).
pub fn deviation(a: &TriMesh, b: &TriMesh) -> Result<f64, ShellError> {
    Ok(deviation_stats(a, b)?.0)
}

/// Thickness check of a shell: for every vertex of `shell`, its distance to
/// the closed `model` surface (the solid it was built from). The outer side
/// sits at `clearance + thickness`, the inner side at `clearance`.
pub fn wall_distances(shell: &TriMesh, model: &TriMesh) -> Result<Vec<f64>, ShellError> {
    let bvh = dc_geom::bvh::Bvh::build(model).map_err(geom)?;
    Ok(bvh
        .closest_points(&shell.positions, f32::INFINITY)
        .par_iter()
        .map(|h| h.map_or(f64::INFINITY, |h| f64::from(h.distance)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed axis-aligned box.
    fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> TriMesh {
        let v = (0..8)
            .map(|k| {
                Vec3::new(
                    if k & 1 == 0 { lo[0] } else { hi[0] },
                    if k & 2 == 0 { lo[1] } else { hi[1] },
                    if k & 4 == 0 { lo[2] } else { hi[2] },
                )
            })
            .collect();
        let t = vec![
            [0, 2, 3],
            [0, 3, 1],
            [4, 5, 7],
            [4, 7, 6],
            [0, 1, 5],
            [0, 5, 4],
            [2, 6, 7],
            [2, 7, 3],
            [0, 4, 6],
            [0, 6, 2],
            [1, 3, 7],
            [1, 7, 5],
        ];
        TriMesh::new(v, t)
    }

    fn soup(parts: &[TriMesh]) -> TriMesh {
        let mut v = Vec::new();
        let mut t = Vec::new();
        for p in parts {
            let off = v.len() as u32;
            v.extend_from_slice(&p.positions);
            t.extend(p.tris.iter().map(|f| f.map(|i| i + off)));
        }
        TriMesh::new(v, t)
    }

    /// A mushroom on a base: the cap overhangs the stem (an undercut
    /// relative to `+Z`).
    fn mushroom() -> TriMesh {
        soup(&[
            cuboid([-5.0, -5.0, 0.0], [5.0, 5.0, 1.0]),
            cuboid([-1.0, -1.0, 0.5], [1.0, 1.0, 4.5]),
            cuboid([-3.0, -3.0, 4.0], [3.0, 3.0, 5.0]),
        ])
    }

    /// Intervals of the vertical line through `(x, y)` inside the mesh.
    fn crossings(bvh: &dc_geom::bvh::Bvh, x: f32, y: f32) -> usize {
        bvh.ray_hits(Vec3::new(x, y, -10.0), Vec3::Z, 100.0, 16, 1e-4)
            .len()
    }

    #[test]
    fn blockout_fills_the_undercut_of_the_axis() {
        let g = closed_grid(&mushroom(), 0.1, 0.0).unwrap();
        let model = g.to_mesh().unwrap();
        let b = blockout_grid(&g, &model, DVec3::Z, 0.0).unwrap();
        let mesh = b.to_mesh().unwrap();
        assert!(mesh.validate().is_watertight());
        // under the cap, beside the stem: empty in the model, filled in the blockout
        let p = Vec3::new(2.0, 0.3, 2.5);
        assert!(!g.contains(p) && b.contains(p));
        // outside the footprint of the cap and above it: still empty
        assert!(!b.contains(Vec3::new(4.0, 0.3, 2.5)));
        assert!(!b.contains(Vec3::new(0.3, 0.3, 5.3)));
        // every vertical line meets the blockout in one interval (2 crossings), the
        // model under the cap in two
        let mb = dc_geom::bvh::Bvh::build(&model).unwrap();
        let bb = dc_geom::bvh::Bvh::build(&mesh).unwrap();
        assert_eq!(crossings(&mb, 2.03, 0.31), 4);
        for i in -9..=9 {
            for j in -9..=9 {
                let (x, y) = (i as f32 * 0.53 + 0.011, j as f32 * 0.51 + 0.017);
                let n = crossings(&bb, x, y);
                assert!(n == 2 || n == 0, "{n} crossings at ({x}, {y})");
            }
        }
        // the top of the model is not raised
        let top = mesh
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((top - 5.0).abs() < 0.05, "{top}");
    }

    #[test]
    fn blockout_follows_a_tilted_axis() {
        let g = closed_grid(&mushroom(), 0.1, 0.0).unwrap();
        let model = g.to_mesh().unwrap();
        // inserted from +X: what the stem hides towards -X is filled, nothing towards +X
        let b = blockout_grid(&g, &model, DVec3::X, 0.0).unwrap();
        assert!(b.contains(Vec3::new(-2.0, 0.3, 2.5)));
        assert!(!b.contains(Vec3::new(2.0, 0.3, 2.5)));
    }

    /// Closed boxes and a prism enclosing a pocket whose only mouth is a slit
    /// of `slit` mm in the lid (overlapping parts, as the gingiva cast, the
    /// prisms and the teeth of a model).
    fn pocket(slit: f32) -> TriMesh {
        let s = 0.5 * (4.0 - slit);
        let prism = TriMesh::new(
            vec![
                Vec3::new(0.3, 0.3, 0.3),
                Vec3::new(1.4, 0.3, 0.3),
                Vec3::new(0.3, 1.4, 0.3),
                Vec3::new(0.3, 0.3, 2.7),
                Vec3::new(1.4, 0.3, 2.7),
                Vec3::new(0.3, 1.4, 2.7),
            ],
            vec![
                [0, 2, 1],
                [3, 4, 5],
                [0, 1, 4],
                [0, 4, 3],
                [1, 2, 5],
                [1, 5, 4],
                [2, 0, 3],
                [2, 3, 5],
            ],
        );
        soup(&[
            cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 0.6]),
            cuboid([0.0, 0.0, 0.0], [0.6, 4.0, 3.0]),
            cuboid([3.4, 0.0, 0.0], [4.0, 4.0, 3.0]),
            cuboid([0.0, 0.0, 0.0], [4.0, 0.6, 3.0]),
            cuboid([0.0, 3.4, 0.0], [4.0, 4.0, 3.0]),
            cuboid([0.0, 0.0, 2.4], [s, 4.0, 3.0]),
            cuboid([4.0 - s, 0.0, 2.4], [4.0, 4.0, 3.0]),
            prism,
        ])
    }

    #[test]
    fn closing_fills_a_pocket_of_overlapping_parts() {
        // the slit (0.4 mm) is narrower than twice the blockout: sealed, and the
        // pocket behind it filled as OpenVDB's closing in the add-on does
        let g = closed_grid(&pocket(0.4), 0.1, BLOCKOUT_MM).unwrap();
        assert!(g.contains(Vec3::new(2.0, 2.0, 1.5)));
        assert!(g.contains(Vec3::new(2.5, 1.0, 1.0)));
        let m = g.to_mesh().unwrap();
        let rep = m.validate();
        assert!(rep.is_watertight(), "{rep:?}");
        assert_eq!(rep.components, 1, "a surface left in the pocket");
        // no surface inside the box (the slit dips by less than a voxel)
        for p in &m.positions {
            let inner =
                p.x > 0.15 && p.x < 3.85 && p.y > 0.15 && p.y < 3.85 && p.z > 0.15 && p.z < 2.85;
            assert!(!inner, "vertex {p} inside the model");
        }
        // the same field, widened for a shell, gives the same model and a shell
        let wide = closed_grid_for(&pocket(0.4), 0.1, BLOCKOUT_MM, 0.9).unwrap();
        assert!(wide.band() > g.band());
        assert_eq!(wide.to_mesh().unwrap().tris.len(), m.tris.len());
        let design = Design {
            thickness_mm: 0.8,
            clearance_mm: 0.1,
        };
        let a = shell_grid(&wide, &design).unwrap().to_mesh().unwrap();
        let b = shell_grid(&g, &design).unwrap().to_mesh().unwrap();
        assert_eq!(a.validate().components, 2);
        assert!(deviation(&a, &b).unwrap() < 0.05);
        // a slit wider than twice the blockout stays open
        let open = closed_grid(&pocket(0.9), 0.1, BLOCKOUT_MM).unwrap();
        assert!(!open.contains(Vec3::new(2.0, 2.0, 1.5)));
    }

    #[test]
    fn reductions_never_cross_themselves() {
        let g = closed_grid(&soup(&[mushroom(), pocket(0.4)]), 0.1, 0.3).unwrap();
        let full = g.to_mesh().unwrap();
        let reduced = reduce(&full, 0.01).unwrap();
        assert!(reduced.tris.len() < full.tris.len());
        assert!(crossing_pairs(&full).is_empty());
        assert!(crossing_pairs(&reduced).is_empty());
        let m = DMat4::from_rotation_translation(
            glam::DQuat::from_rotation_z(0.3) * glam::DQuat::from_rotation_x(0.2),
            DVec3::new(12.3, -40.1, 7.7),
        );
        let moved = reduce_to(&full, 0.01, Some(&m)).unwrap();
        assert!(crossing_pairs(&transformed(&moved, &m)).is_empty());
    }

    #[test]
    fn shell_wall_has_the_thickness() {
        let g = closed_grid(&mushroom(), 0.1, 0.3).unwrap();
        let model = g.to_mesh().unwrap();
        let design = Design {
            thickness_mm: 0.8,
            clearance_mm: 0.1,
        };
        let shell = shell_grid(&g, &design).unwrap().to_mesh().unwrap();
        let rep = shell.validate();
        assert!(rep.is_watertight(), "{rep:?}");
        assert_eq!(rep.components, 2, "outer and fitting surfaces");
        assert!(!model.tris.is_empty());
        // measured from the boxes themselves (the meshed model cuts their edges,
        // which lie on grid lines, by up to a voxel): the outer side is the exact
        // offset of the closed solid, i.e. of the boxes by 0.9; the fitting side
        // is the level of the closing field (`_sdf_group`), the boxes grown by the
        // clearance except where the closing rounded a concave corner
        let boxes = [
            ([-5.0, -5.0, 0.0], [5.0, 5.0, 1.0]),
            ([-1.0, -1.0, 0.5], [1.0, 1.0, 4.5]),
            ([-3.0, -3.0, 4.0], [3.0, 3.0, 5.0]),
        ];
        let dist = |p: Vec3| {
            boxes
                .iter()
                .map(|(lo, hi)| {
                    let (lo, hi) = (Vec3::from(*lo), Vec3::from(*hi));
                    let q = (p - (lo + hi) * 0.5).abs() - (hi - lo) * 0.5;
                    q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
                })
                .fold(f32::INFINITY, f32::min)
        };
        let (mut inner, mut outer, mut at_clearance) = (0, 0, 0);
        for p in &shell.positions {
            let x = dist(*p);
            if x > 0.5 {
                assert!((x - 0.9).abs() < 0.03, "outer vertex {x} mm from the boxes");
                outer += 1;
            } else {
                assert!(
                    (0.07..0.25).contains(&x),
                    "fitting vertex {x} mm from the boxes"
                );
                inner += 1;
                if (x - 0.1).abs() < 0.03 {
                    at_clearance += 1;
                }
            }
        }
        assert!(inner > 0 && outer > 0);
        assert!(at_clearance * 10 > inner * 8, "{at_clearance} of {inner}");
    }

    #[test]
    fn slabs_reduce_within_the_tolerance_and_stay_closed() {
        let g = closed_grid(&mushroom(), 0.1, 0.3).unwrap();
        let full = g.to_mesh().unwrap();
        let reduced = reduce_slabs(&full, 0.01, 4, &vec![false; full.tris.len()]).unwrap();
        let rep = reduced.validate();
        assert!(rep.is_watertight(), "{rep:?}");
        assert_eq!(rep.components, 1);
        assert!(
            reduced.tris.len() * 3 < full.tris.len(),
            "{} of {}",
            reduced.tris.len(),
            full.tris.len()
        );
        let h = deviation(&full, &reduced).unwrap();
        assert!(h <= 0.0101, "{h}");
    }

    #[test]
    fn params_are_checked() {
        let design = Params::default().design(&Settings::default());
        assert_eq!(design.thickness_mm, 1.0);
        assert_eq!(design.clearance_mm, 0.0);
        let p = Params {
            model_voxel_mm: 0.0,
            ..Params::default()
        };
        assert!(p.check(&design).is_err());
        let p = Params {
            insertion_axis: [0.0; 3],
            ..Params::default()
        };
        assert!(p.check(&design).is_err());
        assert!(Params::default().check(&design).is_ok());
        let p: Params = serde_json::from_str(r#"{"thickness_mm": 0.75}"#).unwrap();
        assert_eq!(p.design(&Settings::default()).thickness_mm, 0.75);
    }
}
