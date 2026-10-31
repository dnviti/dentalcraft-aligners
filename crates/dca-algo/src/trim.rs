//! Trim line: the edge of the aligner (equator, straight, scalloped or
//! smooth along the gingival margin) and the cut of the shell along it.
//!
//! Reference: `ortho_aligner/case_print.py` (`trim_points`,
//! `tooth_equators`, `equator_trim`, `_equator_field`, `_aligner_solid`)
//! and `ortho_aligner/aligner.py` (`survey_loop`, `trim_cutter`,
//! `_sdf_group`, `rim_round`); parameters in `ortho_aligner/props.py`.
//!
//! The work is done in arch coordinates (`+Z` occlusal). The line of every
//! mode becomes a *trim surface*, a height field `z = f(x, y)` on a plan
//! grid; the cutter is the closed solid above it (`trim_cutter`), and the
//! trimmed aligner is the untrimmed shell field intersected with the cutter
//! and with the region within reach of the crowns (`points_solid`), then
//! opened by the rim radius (the rounded rim of `_sdf_group`:
//! `min(0.3 x wall, 0.3 mm)`), meshed and reduced to its largest piece: one
//! closed, manifold surface.
//!
//! Modes ([`Mode`]):
//!
//! * **Equator** ([`line::tooth_equator`], [`cutter::equator_field`]): the
//!   border on the equator (survey line, height of contour) of every crown
//!   along its insertion path ([`warp`]: vertical, one common tilted path or
//!   one path per tooth, the plugin's default; the work is done in the space
//!   warped so that the paths are vertical), moved by `equator_offset_mm`, its tabs
//!   rounded, never under the gingival margin nor within 1 mm of the top;
//!   the trim surface keeps the caps over the crowns and the wall beside
//!   them above the equator and above the gingiva.
//! * **Straight** ([`line::margin_points`]): the gingival zenith of the
//!   buccal and of the lingual side of every tooth lowered by
//!   `extension_mm` (Elshazly 2022/2023), interpolated between the teeth
//!   (2 nearest).
//! * **Scalloped**: the gingival margin lowered by `extension_mm` (4
//!   nearest).
//! * **Smooth** (the plugin's default, [`smooth::smooth_border`]): the
//!   margin of the covered teeth taken along the line of the arch, side by
//!   side, smoothed into a line without steps with gentle scallops
//!   (`scallop_mm`), `margin_gap_mm` above it, round the distal face of the
//!   last covered tooth of each side, nothing kept behind it or over the
//!   teeth left out (3 nearest). With fewer than two covered teeth with a
//!   margin it falls back to Scalloped, as the plugin does, and
//!   [`Output::fallback`] says so.
//!
//! Not ported yet (documented deviations): the bars of the equator border over a space between two caps, the fills
//! and slots that close a hole the rounded rim opens (`shell_fills`,
//! `_plug_holes`, `_window_cuts`: the genus of the result is reported
//! instead) and the checks of the panel (labial cover, removal). The rim
//! radius is limited so that the opening leaves at least
//! [`RIM_MIN_WALL_VOXELS`] voxels of wall (the plugin rebuilds a shell with
//! a hole at its print voxel instead). The shell
//! is the caller's: the plugin blocks out 0.8 mm for the smooth border
//! (`SMOOTH_BLOCKOUT_MM`) and limits its reach to the covered crowns; here
//! the reach is measured from [`Input::crowns`].

pub mod cutter;
pub mod grid;
pub mod line;
pub mod model;
pub mod smooth;
pub mod survey;
pub mod warp;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::evidence;
use cutter::{EquatorDesign, EquatorModel, NEAR_TEETH_EXTRA};
use grid::PlanGrid;
use line::MarginMode;

/// The rim of a trimmed shell is rounded with this x the wall ...
pub const RIM_ROUND_WALL: f64 = 0.3;
/// ... at most this (`aligner.BLOCKOUT_MM`: the fitting surface, blocked out
/// as much, is not touched).
pub const RIM_ROUND_MAX_MM: f64 = 0.3;
/// The rounded rim never erodes the wall below this many voxels.
pub const RIM_MIN_WALL_VOXELS: f64 = 1.5;
/// STRAIGHT, SCALLOPED: grid of the trim surface (mm, `trim_cutter`).
pub const MARGIN_CELL_MM: f64 = 0.4;
/// STRAIGHT, SCALLOPED: control points farther than this (plan) from a node
/// raise the surface by [`MARGIN_SLOPE`] per mm.
pub const MARGIN_REACH_MM: f64 = 40.0;
/// See [`MARGIN_REACH_MM`].
pub const MARGIN_SLOPE: f64 = 3.0;

/// Errors of the trim.
#[derive(Debug, thiserror::Error)]
pub enum TrimError {
    /// Inputs that cannot be trimmed.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// A distance field operation failed.
    #[error("distance field: {0}")]
    Geom(String),
}

fn geom(e: impl std::fmt::Display) -> TrimError {
    TrimError::Geom(e.to_string())
}

/// Shape of the aligner's edge (`trim_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// On the equator of every crown along its insertion path: no undercut
    /// and no gingiva covered.
    Equator,
    /// A straight line beyond the gingival zenith (Elshazly 2022-2024;
    /// Traversa 2025).
    Straight,
    /// Following the gingival margin.
    Scalloped,
    /// The gingival margin smoothed along the arch with gentle scallops.
    #[default]
    Smooth,
}

/// Parameters of the trim (the plugin's `trim_*` settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Params {
    /// Shape of the edge.
    pub mode: Mode,
    /// SMOOTH: distance of the border from the gingival margin, positive on
    /// the crown (mm).
    pub margin_gap_mm: f64,
    /// SMOOTH: height of the scallops (mm).
    pub scallop_mm: f64,
    /// EQUATOR: border beyond the equator, towards the gingiva (mm; negative
    /// towards the occlusal surface).
    pub equator_offset_mm: f64,
    /// STRAIGHT, SCALLOPED: extension of the border below the gingival
    /// margin (mm).
    pub extension_mm: f64,
    /// Radius of the rounded rim (mm); default `min(0.3 x wall, 0.3)`, 0 for
    /// a sharp cut.
    pub round_mm: Option<f64>,
    /// EQUATOR: the insertion axis the equators are surveyed along.
    pub insertion_axis: warp::InsertionAxis,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            mode: Mode::Smooth,
            margin_gap_mm: 1.5,
            scallop_mm: 1.5,
            equator_offset_mm: evidence::param_f64("trim_equator_offset").unwrap_or(0.0),
            extension_mm: evidence::param_f64("trim_extension").unwrap_or(2.0),
            round_mm: None,
            insertion_axis: warp::InsertionAxis::PerTooth,
        }
    }
}

impl Params {
    fn check(&self) -> Result<(), TrimError> {
        let ok = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !ok(self.margin_gap_mm, -2.0, 5.0)
            || !ok(self.scallop_mm, 0.0, 4.0)
            || !ok(self.equator_offset_mm, -2.0, 1.5)
            || !ok(self.extension_mm, 0.0, 4.0)
            || self.round_mm.is_some_and(|r| !ok(r, 0.0, 1.0))
        {
            return Err(TrimError::Invalid(format!(
                "parameters out of range (margin_gap_mm -2..5, scallop_mm 0..4, \
                 equator_offset_mm -2..1.5, extension_mm 0..4, round_mm 0..1): {self:?}"
            )));
        }
        Ok(())
    }
}

/// One tooth of the stage, in arch coordinates at its stage pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Tooth {
    /// FDI number.
    pub fdi: Fdi,
    /// Scanned crown points (the equator is surveyed on them).
    pub crown: Vec<DVec3>,
    /// Vertices of the closed tooth (the equator where the crown was not
    /// scanned); may be empty.
    pub closed: Vec<DVec3>,
    /// Gingival margin points of the tooth.
    pub margin: Vec<DVec3>,
    /// Origin of the tooth frame (buccal and lingual sides of STRAIGHT).
    pub origin: DVec3,
    /// Buccal / labial direction of the tooth frame.
    pub e_bl: DVec3,
    /// The aligner covers this tooth (EQUATOR: no shell over the others).
    pub covered: bool,
}

/// Input of the trim of one aligner, in arch coordinates (`+Z` occlusal).
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Untrimmed shell field (the material between the offsets of the
    /// closed model), on a grid with the world origin.
    pub shell: &'a SdfGrid,
    /// The closed teeth at the stage pose (the rays of the equator border).
    pub teeth_mesh: &'a TriMesh,
    /// The gingiva of the model (the rays of the equator border).
    pub gingiva: &'a TriMesh,
    /// The scanned crowns (the aligner reaches only so far from them; an
    /// open surface is fine).
    pub crowns: &'a TriMesh,
    /// The teeth.
    pub teeth: &'a [Tooth],
    /// Points of the open border of the scan (the margin lines stay 1 mm
    /// above it where it is just under them); empty for a closed model.
    pub scan_border: &'a [DVec3],
    /// Internal clearance between the model and the shell (mm).
    pub clearance_mm: f64,
    /// Wall thickness of the shell (mm).
    pub thickness_mm: f64,
}

/// The trim line of one tooth, or of the whole arch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToothLine {
    /// FDI number; `None` for the line of the whole arch (SMOOTH).
    pub fdi: Option<Fdi>,
    /// EQUATOR: the closed loop around the crown (one point per azimuth);
    /// SMOOTH: the border points along the arch (both sides and round the
    /// distal faces); otherwise the control points of the tooth.
    pub points: Vec<DVec3>,
}

/// Result of the trim.
#[derive(Debug, Clone)]
pub struct Output {
    /// The mode used (SMOOTH falls back to SCALLOPED).
    pub mode: Mode,
    /// `true` when the requested mode was replaced by another one.
    pub fallback: bool,
    /// The trimmed aligner: one closed, manifold surface (arch
    /// coordinates).
    pub mesh: TriMesh,
    /// The trim line of every tooth.
    pub lines: Vec<ToothLine>,
    /// The trim surface: its plan grid and the height of every node (in the
    /// warped space of [`Output::warp`] when there is one).
    pub surface: (PlanGrid, Vec<f64>),
    /// EQUATOR: the warp of the insertion paths (`None`: vertical).
    pub warp: Option<warp::PathWarp>,
    /// Radius of the rounded rim used (mm; at most half the wall less
    /// [`RIM_MIN_WALL_VOXELS`] voxels).
    pub round_mm: f64,
    /// Pieces of the trimmed shell dropped (only the largest is kept).
    pub dropped_pieces: usize,
    /// Holes through the wall left (genus of the surface; 0 expected).
    pub genus: i64,
}

impl Output {
    /// Height of the trim surface at a plan point (bilinear between the
    /// nodes).
    pub fn surface_at(&self, x: f64, y: f64) -> f64 {
        let (g, z) = &self.surface;
        let fx = ((x - g.lo.x) / g.dx()).clamp(0.0, (g.nx - 1) as f64);
        let dy = (g.hi.y - g.lo.y) / (g.ny - 1) as f64;
        let fy = ((y - g.lo.y) / dy).clamp(0.0, (g.ny - 1) as f64);
        let (i, j) = (
            (fx.floor() as usize).min(g.nx - 2),
            (fy.floor() as usize).min(g.ny - 2),
        );
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let at = |a: usize, b: usize| z[a * g.ny + b];
        (1.0 - tx) * ((1.0 - ty) * at(i, j) + ty * at(i, j + 1))
            + tx * ((1.0 - ty) * at(i + 1, j) + ty * at(i + 1, j + 1))
    }
}

/// Radius of the rounded rim for the wall (`aligner.rim_round`).
pub fn rim_round(wall: f64, round: Option<f64>) -> f64 {
    round
        .unwrap_or_else(|| (RIM_ROUND_WALL * wall).min(RIM_ROUND_MAX_MM))
        .max(0.0)
}

fn bounds(meshes: &[&TriMesh]) -> Option<(DVec3, DVec3)> {
    let mut it = meshes
        .iter()
        .flat_map(|m| m.positions.iter())
        .map(|p| p.as_dvec3());
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// The largest connected piece of a mesh and the number of pieces dropped.
fn largest_piece(mesh: TriMesh) -> (TriMesh, usize) {
    let comps = mesh.connected_components();
    if comps.len() <= 1 {
        return (mesh, 0);
    }
    let keep = &comps[0];
    let mut map = vec![u32::MAX; mesh.vertex_count()];
    let mut pos = Vec::new();
    let mut tris = Vec::with_capacity(keep.len());
    for &f in keep {
        let t = mesh.tris[f as usize].map(|v| {
            let m = &mut map[v as usize];
            if *m == u32::MAX {
                *m = pos.len() as u32;
                pos.push(mesh.positions[v as usize]);
            }
            *m
        });
        tris.push(t);
    }
    (TriMesh::new(pos, tris), comps.len() - 1)
}

/// Genus of a closed connected surface (`2 - 2g = V - E + F`).
pub fn genus(mesh: &TriMesh) -> i64 {
    let e = mesh.edge_adjacency().edge_count() as i64;
    let chi = mesh.vertex_count() as i64 - e + mesh.face_count() as i64;
    (2 - chi) / 2
}

/// The closed solid above the trim surface as a field on the voxels of
/// `like` (`trim_cutter`), taken back from the warped space of the
/// insertion paths.
fn cutter_grid(
    grid: &PlanGrid,
    z: &[f64],
    z_top: f64,
    warp: Option<&warp::PathWarp>,
    like: &SdfGrid,
) -> Result<SdfGrid, TrimError> {
    let mut slab = grid::slab_solid(grid, z, z_top);
    if let Some(w) = warp {
        for p in &mut slab.positions {
            *p = w.from_warp(p.as_dvec3()).as_vec3();
        }
    }
    SdfGrid::from_mesh(
        &slab,
        &SdfParams {
            voxel_size: like.voxel_size,
            band: like.band(),
        },
    )
    .map_err(geom)
}

/// The region within `radius` of the crowns (`aligner.points_solid` on the
/// crown points) as a field on the voxels of `like`: the distance field of
/// the crown surface (an open surface has no inside) moved out by the
/// radius.
fn near_grid(crowns: &TriMesh, radius: f64, like: &SdfGrid) -> Result<SdfGrid, TrimError> {
    let vs = like.voxel_size;
    let mut g = SdfGrid::from_mesh(
        crowns,
        &SdfParams {
            voxel_size: vs,
            band: radius as f32 + 3.0 * vs,
        },
    )
    .map_err(geom)?;
    g.offset(radius as f32).map_err(geom)?;
    Ok(g)
}

/// The control points of the SMOOTH border (`smooth_trim_points`): the
/// margin of the covered teeth in arch order made into the smooth border,
/// kept 1 mm above the border of the scan, then the high points; with the
/// border points alone (the line). `None` with fewer than two covered teeth
/// with a margin.
fn smooth_points(
    input: &Input<'_>,
    params: &Params,
    outer: f64,
) -> Option<(Vec<DVec3>, Vec<DVec3>)> {
    let covered: BTreeSet<Fdi> = input
        .teeth
        .iter()
        .filter(|t| t.covered)
        .map(|t| t.fdi)
        .collect();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = input
        .teeth
        .iter()
        .filter(|t| !t.crown.is_empty())
        .map(|t| (t.fdi, t.crown.clone()))
        .collect();
    let with_margin: Vec<Fdi> = input
        .teeth
        .iter()
        .filter(|t| t.covered && t.margin.len() >= 3 && crowns.contains_key(&t.fdi))
        .map(|t| t.fdi)
        .collect();
    let arch = Arch::of(*with_margin.first()?);
    let marg: Vec<(Fdi, Vec<DVec3>)> = anatomy::arch_sequence(&with_margin, arch)
        .into_iter()
        .filter_map(|f| Some((f, input.teeth.iter().find(|t| t.fdi == f)?.margin.clone())))
        .collect();
    let gap = params.margin_gap_mm;
    let (mut c, high) =
        smooth::smooth_border(&marg, &crowns, &covered, outer, gap, params.scallop_mm)?;
    line::border_floor(&mut c, input.scan_border, (-gap).max(0.0) + 0.5);
    let mut ctrl = c.clone();
    ctrl.extend(high);
    Some((ctrl, c))
}

/// Trims the shell of one aligner: the trim line of the mode, the trim
/// surface, the cut of the shell along it with a rounded rim.
///
/// # Errors
/// [`TrimError::Invalid`] for parameters out of range, a shell grid not on
/// the world origin, empty meshes or no tooth with a line;
/// [`TrimError::Geom`] when a distance field operation fails.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, TrimError> {
    params.check()?;
    let (cl, wall) = (input.clearance_mm, input.thickness_mm);
    if !(cl.is_finite() && cl >= 0.0 && wall.is_finite() && wall > 0.0) {
        return Err(TrimError::Invalid(format!(
            "clearance {cl} and thickness {wall} must be finite, the thickness positive"
        )));
    }
    if input.shell.origin != Vec3::ZERO {
        return Err(TrimError::Invalid(
            "the shell field must be sampled on the world origin".into(),
        ));
    }
    if input.crowns.tris.is_empty() {
        return Err(TrimError::Invalid("no crown surface".into()));
    }
    let outer = cl + wall;
    let (lo, hi) = bounds(&[input.teeth_mesh, input.gingiva])
        .ok_or_else(|| TrimError::Invalid("the model is empty".into()))?;
    let z_top = hi.z + 5.0;
    // SMOOTH: the smooth border, or SCALLOPED when it cannot be built (fewer
    // than two covered teeth with a margin), as the plugin falls back
    let smooth = if params.mode == Mode::Smooth {
        smooth_points(input, params, outer)
    } else {
        None
    };
    let (mode, fallback) = match params.mode {
        Mode::Smooth if smooth.is_none() => (Mode::Scalloped, true),
        m => (m, false),
    };

    let mut warp: Option<warp::PathWarp> = None;
    let (grid, mut z, lines, near_r) = match mode {
        Mode::Equator => {
            // the insertion paths: the work is done in the warped space where they are vertical
            let crowns: BTreeMap<Fdi, Vec<DVec3>> = input
                .teeth
                .iter()
                .map(|t| (t.fdi, t.crown.clone()))
                .collect();
            let fdis: Vec<Fdi> = crowns.keys().copied().collect();
            let order = fdis
                .first()
                .map(|f| anatomy::arch_sequence(&fdis, Arch::of(*f)))
                .unwrap_or_default();
            warp = warp::insertion_warp(&crowns, params.insertion_axis, &order);
            let to_w = |p: &[DVec3]| -> Vec<DVec3> {
                warp.as_ref()
                    .map_or_else(|| p.to_vec(), |w| p.iter().map(|&q| w.to_warp(q)).collect())
            };
            let warped: Vec<Tooth> = input
                .teeth
                .iter()
                .map(|t| Tooth {
                    crown: to_w(&t.crown),
                    closed: to_w(&t.closed),
                    margin: to_w(&t.margin),
                    ..t.clone()
                })
                .collect();
            let mesh_w = |m: &TriMesh| -> TriMesh {
                match &warp {
                    None => m.clone(),
                    Some(w) => TriMesh::new(
                        m.positions
                            .iter()
                            .map(|p| w.to_warp(p.as_dvec3()).as_vec3())
                            .collect(),
                        m.tris.clone(),
                    ),
                }
            };
            let (teeth_w, gum_w) = (mesh_w(input.teeth_mesh), mesh_w(input.gingiva));
            let (wlo, whi) = bounds(&[&teeth_w, &gum_w]).unwrap_or((lo, hi));
            let (loops, cut) = line::equator_loops(&warped, params.equator_offset_mm, wall);
            if loops.is_empty() {
                return Err(TrimError::Invalid(
                    "no covered tooth has a crown with at least 20 points".into(),
                ));
            }
            let grid = PlanGrid::around(wlo, whi, cutter::equator_cell(wall));
            let model = EquatorModel {
                teeth: &teeth_w,
                gingiva: &gum_w,
                z0: whi.z + 5.0,
            };
            let design = EquatorDesign {
                outer,
                clearance: cl,
                offset: params.equator_offset_mm,
            };
            let z = cutter::equator_field(
                &grid,
                &loops.values().collect::<Vec<_>>(),
                &cut.values().collect::<Vec<_>>(),
                &model,
                design,
            );
            let lines = loops
                .iter()
                .map(|(f, l)| ToothLine {
                    fdi: Some(*f),
                    points: warp.as_ref().map_or_else(
                        || l.points.clone(),
                        |w| l.points.iter().map(|&q| w.from_warp(q)).collect(),
                    ),
                })
                .collect();
            (grid, z, lines, outer + NEAR_TEETH_EXTRA)
        }
        Mode::Smooth => {
            let (ctrl, line) = smooth.unwrap_or_default();
            let grid = PlanGrid::around(lo, hi, MARGIN_CELL_MM);
            let z = cutter::interpolated(&grid, &ctrl, 3, MARGIN_REACH_MM, MARGIN_SLOPE, z_top);
            let lines = vec![ToothLine {
                fdi: None,
                points: line,
            }];
            let reach = outer + (-params.margin_gap_mm).max(0.0) + smooth::SMOOTH_NEAR_MM;
            (grid, z, lines, reach)
        }
        _ => {
            let (mm, k) = if mode == Mode::Straight {
                (MarginMode::Straight, 2)
            } else {
                (MarginMode::Scalloped, 4)
            };
            let ext = params.extension_mm;
            let mut per = line::margin_points(input.teeth, mm, ext);
            for (_, pts) in &mut per {
                line::border_floor(pts, input.scan_border, ext);
            }
            let ctrl: Vec<DVec3> = per.iter().flat_map(|(_, p)| p.iter().copied()).collect();
            if ctrl.is_empty() {
                return Err(TrimError::Invalid(
                    "no tooth has a gingival margin of at least 3 points".into(),
                ));
            }
            let grid = PlanGrid::around(lo, hi, MARGIN_CELL_MM);
            let z = cutter::interpolated(&grid, &ctrl, k, MARGIN_REACH_MM, MARGIN_SLOPE, z_top);
            let lines = per
                .into_iter()
                .map(|(fdi, points)| ToothLine {
                    fdi: Some(fdi),
                    points,
                })
                .collect();
            (grid, z, lines, ext + NEAR_TEETH_EXTRA)
        }
    };
    for v in &mut z {
        *v = if v.is_finite() {
            v.min(z_top - 1.0)
        } else {
            z_top - 1.0
        };
    }

    let mut f = input.shell.clone();
    f.intersect(&cutter_grid(&grid, &z, z_top, warp.as_ref(), input.shell)?)
        .map_err(geom)?;
    f.intersect(&near_grid(input.crowns, near_r, input.shell)?)
        .map_err(geom)?;
    // the opening never erodes the wall below 1.5 voxels: at a voxel coarse
    // for the wall it would cut holes through it (the plugin builds such a
    // shell again at its print voxel)
    let r = rim_round(wall, params.round_mm)
        .min(((wall - RIM_MIN_WALL_VOXELS * f64::from(input.shell.voxel_size)) / 2.0).max(0.0));
    if r > 0.0 {
        f.open(r as f32).map_err(geom)?;
    } else {
        f.redistance().map_err(geom)?;
    }
    let mesh = f.to_mesh().map_err(geom)?;
    if mesh.is_empty() {
        return Err(TrimError::Invalid(
            "nothing of the shell is left above the trim line".into(),
        ));
    }
    let (mesh, dropped_pieces) = largest_piece(mesh);
    let genus = genus(&mesh);
    Ok(Output {
        mode,
        fallback,
        mesh,
        lines,
        surface: (grid, z),
        warp,
        round_mm: r,
        dropped_pieces,
        genus,
    })
}
