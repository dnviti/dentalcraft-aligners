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
//!   last covered tooth of each side at its gum (not on a flange of the scan
//!   behind it at the height of the crown), nothing kept behind it or over
//!   the teeth left out (3 nearest), the reach of the aligner from that
//!   tooth only [`smooth::SMOOTH_END_NEAR_MM`] past its wall. With fewer than
//!   two covered teeth with a margin it falls back to Scalloped, as the
//!   plugin does, and [`Output::fallback`] says so.
//!
//! Rim: the shell is opened by the rim radius (the rounded rim of
//! `_sdf_group`); when that opens a hole through a thin web of the wall the
//! shell is built again with the fills of `shell_fills` (a closing of the
//! rounded shell inside the sharp cut grown in the wall, then also beyond
//! its outer side, then wider), kept when they leave fewer holes. A hole
//! still left is plugged where it is when it is small, or opened to the
//! border by a slot of the trim surface and the shell built again
//! ([`holes`]: `_open_holes`, `_plug_holes`, `_window_cuts`).
//!
//! One piece ([`bars`], `_weak_pairs`, `_bar`): two consecutive caps (arch
//! order, a missing tooth skipped) in different pieces of the meshed shell,
//! or joined through a neck thinner than the section wanted, are joined by a
//! bar over the space between them. Its box is added to the model of the
//! shell ([`Input::rebuild`] builds the shell field again with it), the trim
//! is lowered to its top along it and the reach of the aligner takes it in;
//! a cap that still ends in another piece than the aligner kept is an error
//! ([`TrimError::Split`]), never a segment of the aligner silently dropped.
//!
//! EQUATOR: the checks of the border built (labial cover, offsets reached,
//! the rigid removal of the shell) are in [`Output::checks`] ([`checks`],
//! [`removal`]).
//!
//! Not ported yet (documented deviations): the ceilings of the embrasure
//! fills in the equator field, and the border measured off the bars in the
//! checks. The crowns are thinned for the insertion paths on cells of the
//! arch coordinates (the plugin: of the tooth frame). The rim radius is
//! limited so that the opening leaves at least [`RIM_MIN_WALL_VOXELS`]
//! voxels of wall (the plugin rebuilds a shell with a hole at its print
//! voxel instead). The shell is the caller's: the plugin blocks out 0.8 mm
//! for the smooth border (`SMOOTH_BLOCKOUT_MM`) and limits its reach to the
//! covered crowns; here the reach is measured from [`Input::crowns`]. The
//! plugin bars the EQUATOR border only; here every border is barred
//! ([`bars`]). SMOOTH: the margin of the distal half of the last tooth of
//! each side counts only at the gum, the rows of high points behind it go on
//! past the end of the line of the arch (the plugin skips them) and the reach
//! from its crown is [`smooth::SMOOTH_END_NEAR_MM`], not `SMOOTH_NEAR_MM`
//! ([`smooth_near`]): on a scan with a flange behind the last molar the
//! plugin's rules cut a notch into its cap and leave ragged tongues.

pub mod bars;
pub mod checks;
pub mod cutter;
pub mod grid;
pub mod holes;
pub mod line;
pub mod model;
pub mod removal;
pub mod smooth;
pub mod survey;
pub mod warp;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use dc_geom::knn::KnnIndex;
use dc_geom::sdf::{CsgOp, SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
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
/// A shell whose rounded rim leaves a hole through the wall is built again
/// with this fill radius ...
pub const SHELL_FILL_MM: f64 = 0.5;
/// ... the sharp cut first grown by this inside the wall ...
pub const SHELL_FILL_GROW_MM: f64 = 0.15;
/// ... and, if a hole is still left, with this radius ...
pub const SHELL_FILL_WIDE_MM: f64 = 0.75;
/// ... only where the wall is less than this thicker than the parts the
/// rounding takes.
pub const SHELL_FILL_MARGIN_MM: f64 = 0.4;
/// The rounded rim never erodes the wall below this many voxels.
pub const RIM_MIN_WALL_VOXELS: f64 = 1.5;
/// A shell is built again with bars over the weak joins it shows at most
/// this many times (the plugin: its bars from the last build, then once
/// more for the joins they missed).
pub const BAR_ROUNDS: usize = 2;
/// [`cut_away`] blends its crease over this many voxels.
pub const CUT_BLEND_VOXELS: f32 = 2.0;
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
    /// The aligner comes apart: the caps of these teeth are on another piece
    /// than the one kept, and no bar joins them.
    #[error("the aligner comes apart: the caps of {0:?} are not joined to the rest")]
    Split(Vec<Fdi>),
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
    /// Material taken away after the rim is rounded (the occlusal shaping of
    /// the bite, [`crate::aligner_bite::occlusal::shape_cutter`]): the
    /// opening of the rim, sized for the full wall, would cut through a
    /// thinner occlusal wall. Taken away by [`cut_away`]. `None`: nothing.
    pub cut: Option<&'a SdfGrid>,
    /// Builds the untrimmed shell field again, as `shell` was built, with
    /// these closed solids added to the model (the boxes of the bars over a
    /// space between two caps, arch coordinates: [`bars`]). `None`: no bar can
    /// be added, and caps left in pieces apart are an error.
    pub rebuild: Option<Rebuild<'a>>,
}

/// The builder of [`Input::rebuild`]: the shell field of the model with the
/// solids given added to it, or why it could not be built.
#[derive(Clone, Copy)]
pub struct Rebuild<'a>(pub &'a (dyn Fn(&[TriMesh]) -> Result<SdfGrid, String> + Sync));

impl std::fmt::Debug for Rebuild<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Rebuild")
    }
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
    /// The fill the shell was built again with to close the holes the
    /// rounded rim opened (`None`: not needed or not better).
    pub fill: Option<Fill>,
    /// Holes through the wall plugged where they were ([`holes::plug_holes`]).
    pub plugs: usize,
    /// Slots that raise [`Output::surface`] where they opened holes through
    /// the wall to the border ([`holes::window_cuts`], [`holes::apply_slots`]).
    pub slots: Vec<holes::Slot>,
    /// Pieces of the trimmed shell dropped (only the largest is kept; none
    /// holds a cap: [`TrimError::Split`] otherwise).
    pub dropped_pieces: usize,
    /// Holes through the wall left (genus of the surface; 0 expected).
    pub genus: i64,
    /// The bars joining two caps over a space ([`bars`]), in arch order.
    pub bars: Vec<bars::Bar>,
    /// Joins of two caps still thinner than the section wanted
    /// ([`bars::weak_pairs`]; their caps are on the aligner).
    pub weak: Vec<bars::Weak>,
    /// EQUATOR: the checks of the border built (labial cover, offsets
    /// reached; [`checks::checks`]).
    pub checks: Option<checks::Checks>,
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

/// A fill tried on a trimmed shell whose rounded rim opened a hole through
/// its wall (`shell['fill']`, `shell['fill_out']`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Fill {
    /// Radius of the closing of the rounded shell (mm).
    pub rho: f64,
    /// The sharp cut, grown by [`SHELL_FILL_GROW_MM`], also kept this far
    /// beyond the outer side of the wall (mm; 0: inside the wall only).
    pub out: f64,
}

/// The fills tried in turn on a trimmed shell whose rounded rim opened a
/// hole through its wall (`aligner.shell_fills`): [`SHELL_FILL_MM`], then
/// also beyond the outer side of the wall (a tunnel of a voxel where the
/// outer surfaces of two crowns barely join over a contact), then
/// [`SHELL_FILL_WIDE_MM`]; none when the wall keeps [`SHELL_FILL_MARGIN_MM`]
/// beyond the parts the rounding takes.
pub fn shell_fills(wall: f64, r: f64) -> Vec<Fill> {
    if r <= 0.0 || ((wall - 2.0 * r) * 1e6).round() / 1e6 >= SHELL_FILL_MARGIN_MM {
        return Vec::new();
    }
    let first = Fill {
        rho: SHELL_FILL_MM,
        out: 0.0,
    };
    let out = Fill {
        out: SHELL_FILL_GROW_MM,
        ..first
    };
    vec![
        first,
        out,
        Fill {
            rho: SHELL_FILL_WIDE_MM,
            ..out
        },
    ]
}

/// The untrimmed shell `wall` with its outer side grown by `by`
/// (`shell['fill_out']`: `max(S − outer − by, inner − S)`), the fitting
/// surface where it was: the solid inside the outer side (the shell with
/// its cavity filled) grown by `by`, less the cavity.
fn outer_grown(wall: &SdfGrid, by: f64) -> Result<SdfGrid, TrimError> {
    let mut solid = wall.clone();
    solid.fill_interior().map_err(geom)?;
    let mut cavity = solid.clone();
    cavity.subtract(wall).map_err(geom)?;
    solid.offset(by as f32).map_err(geom)?;
    solid.subtract(&cavity).map_err(geom)?;
    Ok(solid)
}

/// `g` without the solid `c`, the crease blended over [`CUT_BLEND_VOXELS`].
///
/// On the steep sides of a crown the occlusal cut fades out at a grazing
/// angle (the ellipsoids of the allowed top are as wide as the wall) and ends
/// on the steps of its grid of cells, so what it leaves beyond them is a skin
/// thinner than a voxel. A plain difference samples that skin as lone voxels
/// and strands one voxel wide on the outer surface: loose specks, handles of
/// the surface (about a millimetre around, the wall whole under them) and
/// sheets a ten-thousandth of a millimetre apart, which rounding makes cross.
/// The plugin lowers the vertices of a mesh, which cannot change its topology
/// (`case_occlusal.shape_shell`). The blend moves the level by at most a
/// quarter of its width (half a voxel) and only on the crease: the skin goes
/// and the wall keeps its thickness.
fn cut_away(g: &mut SdfGrid, c: &SdfGrid) -> Result<(), TrimError> {
    let k = CUT_BLEND_VOXELS * g.voxel_size;
    g.csg(c, CsgOp::SmoothDifference(k)).map_err(geom)
}

/// The rounded shell `rounded` with its holes and necks closed again
/// (`_sdf_group`, `shell['fill']`): a closing of it by `rho` intersected
/// with the sharp cut grown by [`SHELL_FILL_GROW_MM`] inside `wall` (the
/// untrimmed shell, or the one of [`outer_grown`]), never into the fitting
/// surface. The mesh and the pieces dropped.
fn filled(
    rounded: &SdfGrid,
    sharp: &SdfGrid,
    wall: &SdfGrid,
    cut: Option<&SdfGrid>,
    rho: f64,
) -> Result<(TriMesh, usize), TrimError> {
    let mut g = rounded.clone();
    g.close(rho as f32).map_err(geom)?;
    let mut grown = sharp.clone();
    grown.offset(SHELL_FILL_GROW_MM as f32).map_err(geom)?;
    grown.intersect(wall).map_err(geom)?;
    grown.intersect(&g).map_err(geom)?;
    if let Some(c) = cut {
        cut_away(&mut grown, c)?;
    }
    grown.redistance().map_err(geom)?;
    let mesh = grown.to_mesh().map_err(geom)?;
    Ok(largest_piece(mesh))
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

/// SMOOTH: the reach of the aligner ([`near_grid`]): `radius` from the
/// crowns, only `end` from the crown of the last tooth of each side of
/// `order`. The plugin's reach (`SMOOTH_NEAR_MM`) holds the papilla under the
/// contact of two crowns; past the last tooth there is none, and where the
/// scan stands at the height of the crown beside or behind it (a flange of
/// gingiva) the shell over it was kept as far as that reach, in tongues
/// ragged where the trim surface grazes them. Every triangle of the crowns
/// goes to the tooth of the crown point nearest its first corner. The
/// plugin's reach everywhere when the ends are the only teeth.
fn smooth_near(
    input: &Input<'_>,
    order: &[Fdi],
    radius: f64,
    end: f64,
    like: &SdfGrid,
) -> Result<SdfGrid, TrimError> {
    let (Some(first), Some(last)) = (order.first(), order.last()) else {
        return near_grid(input.crowns, radius, like);
    };
    if order.len() <= 2 {
        return near_grid(input.crowns, radius, like);
    }
    let (mut pts, mut at_end) = (Vec::new(), Vec::new());
    for t in input.teeth {
        let e = t.fdi == *first || t.fdi == *last;
        pts.extend(t.crown.iter().map(|p| p.as_vec3()));
        at_end.extend(std::iter::repeat_n(e, t.crown.len()));
    }
    let index = KnnIndex::build(&pts).map_err(geom)?;
    let crowns = input.crowns;
    let owner: Vec<bool> = index
        .nearest_batch(&crowns.positions)
        .into_iter()
        .map(|n| n.is_some_and(|n| at_end[n.index as usize]))
        .collect();
    let (ends, rest): (Vec<[u32; 3]>, Vec<[u32; 3]>) =
        crowns.tris.iter().partition(|t| owner[t[0] as usize]);
    if ends.is_empty() || rest.is_empty() {
        return near_grid(input.crowns, radius, like);
    }
    let mut g = near_grid(&faces_of(crowns, &rest), radius, like)?;
    g.union(&near_grid(&faces_of(crowns, &ends), end, like)?)
        .map_err(geom)?;
    Ok(g)
}

/// The triangles `tris` of `mesh` as a mesh of their own corners.
fn faces_of(mesh: &TriMesh, tris: &[[u32; 3]]) -> TriMesh {
    let mut map = vec![u32::MAX; mesh.positions.len()];
    let mut pos = Vec::new();
    let tris = tris
        .iter()
        .map(|t| {
            t.map(|i| {
                let m = &mut map[i as usize];
                if *m == u32::MAX {
                    *m = pos.len() as u32;
                    pos.push(mesh.positions[i as usize]);
                }
                *m
            })
        })
        .collect();
    TriMesh::new(pos, tris)
}

/// SMOOTH: the covered teeth with a crown and a margin of at least 3 points,
/// in arch order: the teeth the border is made of.
fn smooth_order(input: &Input<'_>) -> Vec<Fdi> {
    let with_margin: Vec<Fdi> = input
        .teeth
        .iter()
        .filter(|t| t.covered && t.margin.len() >= 3 && !t.crown.is_empty())
        .map(|t| t.fdi)
        .collect();
    with_margin
        .first()
        .map(|f| anatomy::arch_sequence(&with_margin, Arch::of(*f)))
        .unwrap_or_default()
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
    let marg: Vec<(Fdi, Vec<DVec3>)> = smooth_order(input)
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

/// What the EQUATOR border is built from, in the warped space of the
/// insertion paths when there is a warp (`_equator_context`).
struct EquatorParts {
    /// The warp of the insertion paths (`None`: vertical).
    warp: Option<warp::PathWarp>,
    /// The trim loops of the covered teeth ...
    loops: BTreeMap<Fdi, line::ToothLoop>,
    /// ... and the equators of the teeth left out (no shell over them).
    cut: BTreeMap<Fdi, line::ToothLoop>,
    /// The teeth in arch order.
    order: Vec<Fdi>,
    /// The teeth, their points warped.
    teeth: Vec<Tooth>,
    /// The closed teeth, warped.
    teeth_mesh: TriMesh,
    /// The gingiva, warped.
    gingiva: TriMesh,
    /// The bounds of the warped model.
    lo: DVec3,
    /// See `lo`.
    hi: DVec3,
}

/// `m` in the warped space of `w` (itself without a warp).
fn warped_mesh(w: Option<&warp::PathWarp>, m: &TriMesh) -> TriMesh {
    match w {
        None => m.clone(),
        Some(w) => TriMesh::new(
            m.positions
                .iter()
                .map(|p| w.to_warp(p.as_dvec3()).as_vec3())
                .collect(),
            m.tris.clone(),
        ),
    }
}

impl EquatorParts {
    /// The insertion paths (the work is done in the warped space where they
    /// are vertical) and the trim loops of the teeth.
    fn new(
        input: &Input<'_>,
        params: &Params,
        wall: f64,
        bounds_: (DVec3, DVec3),
    ) -> Result<Self, TrimError> {
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
        let warp = warp::insertion_warp(&crowns, params.insertion_axis, &order);
        let to_w = |p: &[DVec3]| -> Vec<DVec3> {
            warp.as_ref()
                .map_or_else(|| p.to_vec(), |w| p.iter().map(|&q| w.to_warp(q)).collect())
        };
        let teeth: Vec<Tooth> = input
            .teeth
            .iter()
            .map(|t| Tooth {
                crown: to_w(&t.crown),
                closed: to_w(&t.closed),
                margin: to_w(&t.margin),
                ..t.clone()
            })
            .collect();
        let teeth_mesh = warped_mesh(warp.as_ref(), input.teeth_mesh);
        let gingiva = warped_mesh(warp.as_ref(), input.gingiva);
        let (lo, hi) = bounds(&[&teeth_mesh, &gingiva]).unwrap_or(bounds_);
        let (loops, cut) = line::equator_loops(&teeth, params.equator_offset_mm, wall);
        if loops.is_empty() {
            return Err(TrimError::Invalid(
                "no covered tooth has a crown with at least 20 points".into(),
            ));
        }
        Ok(Self {
            warp,
            loops,
            cut,
            order,
            teeth,
            teeth_mesh,
            gingiva,
            lo,
            hi,
        })
    }

    /// The heights of the trim surface on `grid` with the `bars`
    /// (`_equator_field(ctx, bars)`).
    fn surface(
        &self,
        grid: &PlanGrid,
        design: EquatorDesign,
        wall: f64,
        bars: &[bars::Bar],
    ) -> Vec<f64> {
        let boxes: Vec<TriMesh> = bars
            .iter()
            .map(|b| warped_mesh(self.warp.as_ref(), &b.solid))
            .collect();
        let model = EquatorModel {
            teeth: &self.teeth_mesh,
            gingiva: &self.gingiva,
            z0: self.hi.z + 5.0,
            bars,
            bar_boxes: &boxes,
        };
        let (lv, cv) = (
            self.loops.values().collect::<Vec<_>>(),
            self.cut.values().collect::<Vec<_>>(),
        );
        let field = |g: &PlanGrid| cutter::equator_field(g, &lv, &cv, &model, design);
        let mut z = field(grid);
        if cutter::equator_cell(wall) < cutter::EQUATOR_CELL_MM {
            // the cap over the crown as on the grid of EQUATOR_CELL_MM deep inside the outlines
            let outlines: Vec<&[DVec3]> = self
                .loops
                .values()
                .filter(|l| l.points.len() >= 3)
                .map(|l| l.points.as_slice())
                .collect();
            cutter::coarse_inside(grid, &mut z, &outlines, field);
        }
        z
    }
}

/// What the bars of a trim are measured against ([`bars::bar`]): the teeth
/// and the model in the space of the trim surface (warped along the
/// insertion paths for EQUATOR), and the loop under which the top of a bar
/// is held (EQUATOR: the trim loops; else the equator along the vertical
/// path).
struct BarSource<'a> {
    teeth: &'a [Tooth],
    loops: Option<&'a BTreeMap<Fdi, line::ToothLoop>>,
    teeth_tree: Option<dc_geom::bvh::Bvh>,
    gum_tree: Option<dc_geom::bvh::Bvh>,
    gum: &'a TriMesh,
    z_ray: f64,
    warp: Option<&'a warp::PathWarp>,
}

impl<'a> BarSource<'a> {
    fn new(input: &Input<'a>, eq: Option<&'a EquatorParts>, z_top: f64) -> Result<Self, TrimError> {
        let tree = |m: &TriMesh| {
            (!m.tris.is_empty())
                .then(|| dc_geom::bvh::Bvh::build(m))
                .transpose()
                .map_err(geom)
        };
        Ok(match eq {
            Some(e) => Self {
                teeth: &e.teeth,
                loops: Some(&e.loops),
                teeth_tree: tree(&e.teeth_mesh)?,
                gum_tree: tree(&e.gingiva)?,
                gum: &e.gingiva,
                z_ray: e.hi.z + 5.0,
                warp: e.warp.as_ref(),
            },
            None => Self {
                teeth: input.teeth,
                loops: None,
                teeth_tree: tree(input.teeth_mesh)?,
                gum_tree: tree(input.gingiva)?,
                gum: input.gingiva,
                z_ray: z_top,
                warp: None,
            },
        })
    }

    /// The bar joining the caps of `a` and `b`; `None` when a tooth is not
    /// there.
    fn bar(&self, (a, b): (Fdi, Fdi), clearance: f64, outer: f64, thin: bool) -> Option<bars::Bar> {
        let find = |f: Fdi| self.teeth.iter().find(|t| t.fdi == f);
        let (ta, tb) = (find(a)?, find(b)?);
        let lp = |t: &Tooth| match self.loops {
            Some(l) => l.get(&t.fdi).cloned(),
            None => line::tooth_equator(t, line::EQUATOR_AZIMUTHS),
        };
        let (la, lb) = (lp(ta), lp(tb));
        let model = bars::BarModel {
            teeth: self.teeth_tree.as_ref(),
            gingiva: self.gum_tree.as_ref().map(|g| (g, self.gum)),
            z_ray: self.z_ray,
        };
        Some(bars::bar(
            (a, b),
            (bars::tooth_points(ta), bars::tooth_points(tb)),
            (la.as_ref(), lb.as_ref()),
            &model,
            clearance,
            outer,
            thin,
            self.warp,
        ))
    }
}

/// The tops of the `bars`: the reach of the aligner takes them in (the
/// points along the top of every bar, `points_solid`).
fn strips(bars: &[bars::Bar]) -> TriMesh {
    let (mut pos, mut tris) = (Vec::new(), Vec::new());
    for b in bars {
        let s = b.top_strip();
        let off = pos.len() as u32;
        pos.extend(s.positions);
        tris.extend(s.tris.iter().map(|t| t.map(|v| v + off)));
    }
    TriMesh::new(pos, tris)
}

/// The reach `near` of the aligner (from [`near_grid`]) grown over the tops
/// of the `bars`: the distance to the crowns and the strips is the smaller
/// of the two, so only the strips are measured, then united with it.
fn near_with_strips(
    mut near: SdfGrid,
    bars: &[bars::Bar],
    radius: f64,
    like: &SdfGrid,
) -> Result<SdfGrid, TrimError> {
    let s = strips(bars);
    if !s.tris.is_empty() {
        near.union(&near_grid(&s, radius, like)?).map_err(geom)?;
    }
    Ok(near)
}

/// Trims the shell of one aligner: the trim line of the mode, the trim
/// surface, the cut of the shell along it with a rounded rim, in one piece
/// (two caps in pieces apart, or joined too thinly, are joined by a bar over
/// the space between them and the shell built again: [`bars`]).
///
/// # Errors
/// [`TrimError::Invalid`] for parameters out of range, a shell grid not on
/// the world origin, empty meshes or no tooth with a line;
/// [`TrimError::Geom`] when a distance field operation fails or the shell
/// cannot be built again with the bars; [`TrimError::Split`] when the cap of
/// a covered tooth is left on another piece than the aligner kept.
#[allow(clippy::too_many_lines)]
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
    let design = EquatorDesign {
        outer,
        clearance: cl,
        offset: params.equator_offset_mm,
    };
    let eq = if mode == Mode::Equator {
        Some(EquatorParts::new(input, params, wall, (lo, hi))?)
    } else {
        None
    };
    let warp = eq.as_ref().and_then(|e| e.warp.clone());

    // the grid of the trim surface, its heights without bars (EQUATOR: the
    // field, made with the bars), the lines and the reach of the aligner
    let (grid, plain, lines, near_r) = match &eq {
        Some(e) => {
            let grid = PlanGrid::around(e.lo, e.hi, cutter::equator_cell(wall));
            let lines = e
                .loops
                .iter()
                .map(|(f, l)| ToothLine {
                    fdi: Some(*f),
                    points: warp.as_ref().map_or_else(
                        || l.points.clone(),
                        |w| l.points.iter().map(|&q| w.from_warp(q)).collect(),
                    ),
                })
                .collect();
            (grid, Vec::new(), lines, outer + NEAR_TEETH_EXTRA)
        }
        None if mode == Mode::Smooth => {
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
        None => {
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
    // the heights with the bars: lowered to their top along them, at most
    // 1 mm under the top of the solid of the cutter
    let surface = |bars: &[bars::Bar]| -> Vec<f64> {
        let mut z = match &eq {
            Some(e) => e.surface(&grid, design, wall, bars),
            None => {
                let mut z = plain.clone();
                if !bars.is_empty() {
                    for (k, v) in z.iter_mut().enumerate() {
                        *v = v.min(bars::ceiling(bars, grid.node(k)));
                    }
                }
                z
            }
        };
        for v in &mut z {
            *v = if v.is_finite() {
                v.min(z_top - 1.0)
            } else {
                z_top - 1.0
            };
        }
        z
    };

    // the opening never erodes the wall below 1.5 voxels: at a voxel coarse
    // for the wall it would cut holes through it (the plugin builds such a
    // shell again at its print voxel)
    let voxel = f64::from(input.shell.voxel_size);
    let r =
        rim_round(wall, params.round_mm).min(((wall - RIM_MIN_WALL_VOXELS * voxel) / 2.0).max(0.0));
    let mut z = surface(&[]);
    let near = if mode == Mode::Smooth {
        let end = near_r - smooth::SMOOTH_NEAR_MM + smooth::SMOOTH_END_NEAR_MM;
        smooth_near(input, &smooth_order(input), near_r, end, input.shell)?
    } else {
        near_grid(input.crowns, near_r, input.shell)?
    };
    let mut b = Build {
        input,
        shell: Cow::Borrowed(input.shell),
        grid: &grid,
        z_top,
        warp: warp.as_ref(),
        near,
        r,
    };
    let mut cut = b.cut(&z)?;
    let mut full = b.full(&cut)?;
    let mut joins = bars::weak_pairs(&full, input.teeth, wall, outer);

    // one piece: the caps of two teeth in pieces apart, or joined too thinly,
    // joined by a bar over the space between them, the shell built again with
    // its box; the joins the bars missed are barred at the next round
    let mut made: Vec<bars::Bar> = Vec::new();
    if let Some(rebuild) = input.rebuild {
        let thin = !shell_fills(wall, rim_round(wall, None)).is_empty();
        let order = {
            let fdis: Vec<Fdi> = input.teeth.iter().map(|t| t.fdi).collect();
            fdis.first()
                .map(|f| anatomy::arch_sequence(&fdis, Arch::of(*f)))
                .unwrap_or_default()
        };
        let rank = |f: Fdi| order.iter().position(|x| *x == f).unwrap_or(usize::MAX);
        let mut source = None;
        for _ in 0..BAR_ROUNDS {
            let new: Vec<(Fdi, Fdi)> = joins
                .weak
                .iter()
                .map(|w| (w.a, w.b))
                .filter(|p| !made.iter().any(|m| m.teeth == *p))
                .collect();
            if new.is_empty() {
                break;
            }
            if source.is_none() {
                source = Some(BarSource::new(input, eq.as_ref(), z_top)?);
            }
            let Some(src) = source.as_ref() else {
                break;
            };
            made.extend(new.into_iter().filter_map(|p| src.bar(p, cl, outer, thin)));
            made.sort_by_key(|m| (rank(m.teeth.0), rank(m.teeth.1)));
            let boxes: Vec<TriMesh> = made.iter().map(|m| m.solid.clone()).collect();
            drop(cut);
            let shell = (rebuild.0)(&boxes).map_err(TrimError::Geom)?;
            if shell.origin != Vec3::ZERO || shell.voxel_size != input.shell.voxel_size {
                return Err(TrimError::Invalid(
                    "the shell built again with the bars is not on the voxels of the shell".into(),
                ));
            }
            z = surface(&made);
            b = Build {
                shell: Cow::Owned(shell),
                near: near_with_strips(b.near, &made, near_r, input.shell)?,
                ..b
            };
            cut = b.cut(&z)?;
            full = b.full(&cut)?;
            joins = bars::weak_pairs(&full, input.teeth, wall, outer);
        }
    }
    // a cap on another piece than the one kept: the aligner would lose those
    // teeth (never dropped silently)
    if !joins.lost.is_empty() {
        return Err(TrimError::Split(joins.lost));
    }
    let (mut mesh, mut dropped_pieces) = largest_piece(full);
    if mesh.is_empty() {
        return Err(TrimError::Invalid(
            "nothing of the shell is left above the trim line".into(),
        ));
    }
    let mut grown_out = None;
    let mut genus = genus(&mesh);
    let mut fill = None;
    if genus > 0 {
        // holes the rounded rim opened through a thin web: built again with the fills
        for fl in shell_fills(wall, r) {
            let (m2, d2) = b.mesh(&cut, Some(fl), &mut grown_out)?;
            let g2 = self::genus(&m2);
            if g2 < genus || (fill.is_some() && g2 == genus) {
                (mesh, dropped_pieces, genus, fill) = (m2, d2, g2, Some(fl));
            }
            if genus == 0 {
                break;
            }
        }
    }
    drop(cut);

    // the holes left (`_open_holes`): plugged where they are, or opened to the
    // border by a slot, wider at every round, the shell built again; a round
    // is kept when the holes are fewer, no tooth loses its cap and at most
    // VOLUME_SHARE of the volume goes
    let (mut plugs, mut slots) = (0, Vec::new());
    if genus > 0 {
        // the model of the shell: the boxes of the bars too (`with_boxes`)
        let mut parts = vec![input.teeth_mesh, input.gingiva];
        parts.extend(made.iter().map(|m| &m.solid));
        let model = holes::Model::new(&parts).map_err(geom)?;
        let design = holes::Design {
            outer,
            clearance: cl,
            rim: r,
            voxel,
        };
        (mesh, genus, plugs) = holes::plug_holes(mesh, &model, design, Some(genus));
        let reach = outer + 2.0 * voxel + 0.3;
        let mut uncovered = None;
        for k in 0..holes::ROUNDS {
            if genus <= 0 {
                break;
            }
            let half = (k + 1) as f64 * holes::SLOT_MM.max(1.5 * voxel);
            let new = holes::window_cuts(&mesh, &model, design, warp.as_ref(), half);
            if new.is_empty() {
                break;
            }
            let mut all = slots.clone();
            all.extend(new);
            let mut z2 = z.clone();
            holes::apply_slots(&grid, &mut z2, &all, z_top);
            let cut = b.cut(&z2)?;
            let (m2, d2) = b.mesh(&cut, fill, &mut grown_out)?;
            drop(cut);
            let (m2, g2, p2) = holes::plug_holes(m2, &model, design, None);
            let before = uncovered
                .get_or_insert_with(|| holes::uncovered_teeth(&mesh, input.teeth, reach))
                .clone();
            let u2 = holes::uncovered_teeth(&m2, input.teeth, reach);
            if g2 >= genus
                || u2.iter().any(|f| !before.contains(f))
                || holes::volume(&m2) < (1.0 - holes::VOLUME_SHARE) * holes::volume(&mesh)
            {
                break;
            }
            (slots, mesh, dropped_pieces, genus, plugs) = (all, m2, d2, g2, p2);
            uncovered = Some(u2);
        }
    }
    drop(b);
    // EQUATOR: the checks of the border built, for the panel and the report;
    // the shell taken off in one piece (PER_TOOTH: the least locked rigid path
    // around the mean path of the teeth, else the one path of the arch)
    let checks = match &eq {
        None => None,
        Some(e) => {
            let list: Vec<(Fdi, &line::ToothLoop)> = e.loops.iter().map(|(f, l)| (*f, l)).collect();
            let axis = params.insertion_axis;
            let mut c = checks::checks(
                &list,
                &mesh,
                warp.as_ref(),
                &e.order,
                params.equator_offset_mm,
                outer,
                axis == warp::InsertionAxis::Common,
            );
            let shell = dc_geom::bvh::Bvh::build(&mesh).map_err(geom)?;
            let teeth = (!input.teeth_mesh.tris.is_empty())
                .then(|| dc_geom::bvh::Bvh::build(input.teeth_mesh))
                .transpose()
                .map_err(geom)?;
            let shears = warp.as_ref().map_or_else(Vec::new, |w| w.shears.clone());
            c.removal = Some(if axis == warp::InsertionAxis::PerTooth {
                removal::rigid_lock(&mesh, &shell, teeth.as_ref(), &shears)
            } else {
                let sh = shears.first().copied().unwrap_or(DVec2::ZERO);
                removal::path_lock(&mesh, &shell, teeth.as_ref(), sh)
            });
            Some(c)
        }
    };
    Ok(Output {
        mode,
        fallback,
        mesh,
        lines,
        surface: (grid, z),
        warp,
        round_mm: r,
        fill,
        plugs,
        slots,
        dropped_pieces,
        genus,
        bars: made,
        weak: joins.weak,
        checks,
    })
}

/// What builds the trimmed shell from a trim surface: the shell field (of
/// the input, or built again with the boxes of the bars), the plan grid, the
/// warp of the insertion paths, the reach of the aligner and the rim radius.
struct Build<'a> {
    input: &'a Input<'a>,
    shell: Cow<'a, SdfGrid>,
    grid: &'a PlanGrid,
    z_top: f64,
    warp: Option<&'a warp::PathWarp>,
    near: SdfGrid,
    r: f64,
}

/// The shell cut by a trim surface, its rim rounded, and the sharp cut (for
/// the fills; `None` without rounding).
struct Cut {
    rounded: SdfGrid,
    sharp: Option<SdfGrid>,
}

impl Build<'_> {
    /// The shell field intersected with the solid above the trim surface `z`
    /// and with the reach of the aligner, then opened by the rim radius;
    /// the occlusal cut taken away from both.
    fn cut(&self, z: &[f64]) -> Result<Cut, TrimError> {
        let shell = self.shell.as_ref();
        let mut f = shell.clone();
        f.intersect(&cutter_grid(self.grid, z, self.z_top, self.warp, shell)?)
            .map_err(geom)?;
        f.intersect(&self.near).map_err(geom)?;
        let mut sharp = (self.r > 0.0).then(|| f.clone());
        if self.r > 0.0 {
            f.open(self.r as f32).map_err(geom)?;
        } else {
            f.redistance().map_err(geom)?;
        }
        if let Some(c) = self.input.cut {
            cut_away(&mut f, c)?;
            if let Some(s) = &mut sharp {
                cut_away(s, c)?;
            }
        }
        Ok(Cut { rounded: f, sharp })
    }

    /// The mesh of `cut`, every piece of it.
    fn full(&self, cut: &Cut) -> Result<TriMesh, TrimError> {
        cut.rounded.to_mesh().map_err(geom)
    }

    /// The mesh of `cut` (its largest piece and the pieces dropped), built
    /// again with `fill` when there is one; `grown_out` keeps the shell of
    /// [`outer_grown`] for the fills that need it.
    fn mesh(
        &self,
        cut: &Cut,
        fill: Option<Fill>,
        grown_out: &mut Option<(f64, SdfGrid)>,
    ) -> Result<(TriMesh, usize), TrimError> {
        let shell = self.shell.as_ref();
        match (fill, &cut.sharp) {
            (Some(fl), Some(sharp)) => {
                let w = if fl.out > 0.0 {
                    if grown_out.as_ref().is_none_or(|(by, _)| *by != fl.out) {
                        *grown_out = Some((fl.out, outer_grown(shell, fl.out)?));
                    }
                    grown_out.as_ref().map_or(shell, |(_, g)| g)
                } else {
                    shell
                };
                filled(&cut.rounded, sharp, w, self.input.cut, fl.rho)
            }
            _ => Ok(largest_piece(self.full(cut)?)),
        }
    }
}
