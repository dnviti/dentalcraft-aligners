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

/// The prisms of the blockout sweep are moved this many voxels across the
/// axis ([`sweep_prisms`]).
pub const SWEEP_SHIFT_VOXELS: f64 = 0.003;
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
/// of the union of the parts). Cavities of a voxel or two may be left: the
/// meshes keep their largest pieces only ([`largest_pieces`]).
pub fn closed_grid(parts: &TriMesh, voxel: f64, blockout: f64) -> Result<SdfGrid, ShellError> {
    let params = SdfParams {
        voxel_size: voxel as f32,
        band: band_for(voxel, blockout),
    };
    let mut g = SdfGrid::from_mesh(parts, &params).map_err(geom)?;
    if blockout > 0.0 {
        g.close(blockout as f32).map_err(geom)?;
    }
    Ok(g)
}

/// The shell field `max(S − outer, inner − S)` of a closed field: the
/// material between its offsets at `clearance` and `clearance + thickness`.
pub fn shell_grid(closed: &SdfGrid, design: &Design) -> Result<SdfGrid, ShellError> {
    let mut outer = closed.clone();
    outer
        .offset((design.clearance_mm + design.thickness_mm) as f32)
        .map_err(geom)?;
    if design.clearance_mm > 0.0 {
        let mut inner = closed.clone();
        inner.offset(design.clearance_mm as f32).map_err(geom)?;
        outer.subtract(&inner).map_err(geom)?;
    } else {
        outer.subtract(closed).map_err(geom)?;
    }
    Ok(outer)
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
/// `shift` moves the prisms across the axis (mm, a tiny fraction of a
/// voxel): the walls between two prisms lie inside the sweep, and a wall
/// through voxel centres (marching-cubes vertices share grid coordinates)
/// would read a distance of exactly zero there, which the sign of
/// [`SdfGrid::from_mesh`] cannot tell from the outside.
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

/// The blockout field: the closed field united with the sweep of its
/// surface `model` along `−axis` (no undercut relative to `axis`). `model`
/// lies within `tol` of the zero level of `closed` (a reduced mesh of it):
/// the sweep is grown across the axis by that plus [`SWEEP_GROW_VOXELS`]
/// ([`sweep_prisms`]).
pub fn blockout_grid(
    closed: &SdfGrid,
    model: &TriMesh,
    axis: DVec3,
    tol: f64,
) -> Result<SdfGrid, ShellError> {
    let voxel = f64::from(closed.voxel_size);
    let grow = tol + SWEEP_GROW_VOXELS * voxel;
    let sweep = sweep_prisms(model, axis, SWEEP_SHIFT_VOXELS * voxel, grow);
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
/// watertight; the full mesh when the reduction is not (or `tol` is 0).
///
/// The exact reduction ([`dc_geom::qem::decimate`] with a tolerance) costs
/// more than linearly in the triangles, so a large mesh is reduced in slabs
/// of about [`REDUCE_SLAB_TRIS`] triangles at the same time
/// (`REDUCE_CHUNK_TRIS` of the plugin), the seams between them held at full
/// resolution ([`REDUCE_MAX_SLABS`] at most).
pub fn reduce(mesh: &TriMesh, tol: f64) -> Result<TriMesh, ShellError> {
    if tol <= 0.0 || mesh.tris.len() < REDUCE_MIN_TRIS {
        return Ok(mesh.clone());
    }
    let n = mesh
        .tris
        .len()
        .div_ceil(REDUCE_SLAB_TRIS)
        .clamp(1, REDUCE_MAX_SLABS);
    let out = reduce_slabs(mesh, tol, n)?;
    if out.validate().is_watertight() {
        Ok(out)
    } else {
        Ok(mesh.clone())
    }
}

/// The exact reduction within `tol` in `n` slabs along the longest side of
/// the box, at the same time: each slab is decimated with a ring of the
/// faces of its neighbours under another label and its own faces on the
/// seam under labels of their own, so the vertices there neither move nor
/// disappear, and the slabs are welded back on them.
fn reduce_slabs(mesh: &TriMesh, tol: f64, n: usize) -> Result<TriMesh, ShellError> {
    if n <= 1 {
        let params = DecimateParams {
            preserve_labels: false,
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
            for (t, &k) in mesh.tris.iter().zip(&slab) {
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
                // own faces on the seam: each its own label, so that every vertex of
                // the band is held and no collapse makes a new edge between two seam
                // vertices (the neighbour slab could make the same one)
                let label = if !own {
                    RING
                } else if t.iter().any(|&i| owner[i as usize] == MANY) {
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
fn finish(full: TriMesh, pieces: usize, tol: f64) -> Result<Finished, ShellError> {
    let full = largest_pieces(&full, pieces);
    let mut try_tol = tol;
    for _ in 0..2 {
        let mesh = reduce(&full, try_tol)?;
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
    let t = Instant::now();
    let model_grid = if need_model_grid || (outputs.shell && same_voxel) {
        Some(closed_grid(
            &parts,
            params.model_voxel_mm,
            params.blockout_mm,
        )?)
    } else {
        None
    };
    report
        .seconds
        .insert("closing".into(), t.elapsed().as_secs_f64());

    let mut model_reduced = None;
    if let Some(g) = model_grid.as_ref().filter(|_| need_model_grid) {
        let t = Instant::now();
        let solid = finish(g.to_mesh().map_err(geom)?, 1, tol)?;
        solid.record("model", t.elapsed().as_secs_f64(), &mut report);
        model_reduced = Some(solid.mesh);
    }

    // the shell and the blockout are independent: built at the same time
    let shell_job = || -> Result<Option<(Finished, f64)>, ShellError> {
        if !outputs.shell {
            return Ok(None);
        }
        let t = Instant::now();
        let own;
        let closed = match (&model_grid, same_voxel) {
            (Some(g), true) => g,
            _ => {
                own = closed_grid(&parts, params.shell_voxel_mm, params.blockout_mm)?;
                &own
            }
        };
        // the untrimmed shell: the outer surface and the fitting surface
        let solid = finish(shell_grid(closed, design)?.to_mesh().map_err(geom)?, 2, tol)?;
        Ok(Some((solid, t.elapsed().as_secs_f64())))
    };
    let blockout_job = || -> Result<Option<(Finished, f64)>, ShellError> {
        let (true, Some(g), Some(model)) = (outputs.blockout, &model_grid, &model_reduced) else {
            return Ok(None);
        };
        let t = Instant::now();
        let axis = DVec3::from(params.insertion_axis);
        let solid = finish(
            blockout_grid(g, model, axis, tol)?
                .to_mesh()
                .map_err(geom)?,
            1,
            tol,
        )?;
        Ok(Some((solid, t.elapsed().as_secs_f64())))
    };
    let (shell, blockout) = rayon::join(shell_job, blockout_job);
    if let Some((solid, secs)) = shell? {
        solid.record("shell", secs, &mut report);
        out.shell = Some(transformed(&solid.mesh, &from_arch));
    }
    if let Some((solid, secs)) = blockout? {
        solid.record("blockout", secs, &mut report);
        out.blockout = Some(transformed(&solid.mesh, &from_arch));
    }
    if outputs.model {
        out.model = model_reduced.as_ref().map(|m| transformed(m, &from_arch));
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
        let d = wall_distances(&shell, &model).unwrap();
        let (mut inner, mut outer) = (0, 0);
        for x in d {
            if (x - 0.9).abs() < 0.05 {
                outer += 1;
            } else if (x - 0.1).abs() < 0.05 {
                inner += 1;
            } else {
                panic!("vertex {x} mm from the model");
            }
        }
        assert!(inner > 0 && outer > 0);
    }

    #[test]
    fn slabs_reduce_within_the_tolerance_and_stay_closed() {
        let g = closed_grid(&mushroom(), 0.1, 0.3).unwrap();
        let full = g.to_mesh().unwrap();
        let reduced = reduce_slabs(&full, 0.01, 4).unwrap();
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
