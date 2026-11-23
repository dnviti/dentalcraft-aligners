//! Orthodontic study-model bases ("squadratura dello zoccolo"), European or
//! American (ABO) style (B6).
//!
//! Specification: `docs/plan/B6-STUDY-MODEL-TRIMMING.md` of the
//! superproject (a synthesis of the ABO Dental Cast Guidelines, the NEBDN
//! handout, the Leipzig course and the vendors' virtual bases). The old
//! plugin has no trimmed base (only the flat `base_height` of the printable
//! model). Every angle and distance is a [`Params`] field.
//!
//! # Frame and landmarks
//! Common occlusion frame (the arches are in occlusion in case coordinates,
//! after a bite registration or the virtual bite):
//! * `z`: normal of the occlusal plane fitted through the cusp tips of both
//!   arches (the highest vertex of every tooth label towards the
//!   antagonist), pointing from the lower to the upper arch; origin on it;
//! * `y`: the midline `m`, anterior. The midpalatal raphe is estimated as the
//!   symmetry axis of the upper arch: perpendicular to the mean transverse
//!   vector of the contralateral tooth pairs (centroids of `1k` and `2k`)
//!   and through their midpoints. Fallbacks: the same on the lower arch
//!   (the dental midline), then the `y` of the arch frame from the seeds;
//! * `x = y × z` (the patient's right).
//!
//! Each arch is worked in its own frame with `+Z` occlusal (the upper arch
//! turned 180° about `y`), so its base plane is `z = −H`.
//!
//! # Protected anatomy
//! The tooth vertices (labels) and the scan within `sulcus_reach_mm` (plan)
//! of them (the attached gingiva down to the vestibular sulcus, the tuber
//! and the retromolar pad); for the upper arch also the palate (the scan
//! inside the convex hull of the teeth). The base keeps all of it: the base
//! plane is lowered and the walls are pushed out instead of cutting it, and
//! a warning says so. The scan outside it (lips, cheeks, floor of the mouth)
//! is left out of the model.
//!
//! # Base
//! * Base planes parallel to the occlusal plane at `z = ±H` (upper `+`),
//!   `H` = anatomic height `A` + art portion. ABO: `H = total_height / 2`,
//!   art `= H · art_fraction` (or `art_thickness_mm`); European:
//!   `A = depth + m_base`, art `= base_thickness_mm`. `H` is always at least
//!   the deepest protected point (vault, sulcus) plus `m_base`.
//! * Back plane perpendicular to the raphe and to the base, shared by both
//!   arches, `back_margin_mm` behind the most distal protected point.
//! * Outline (vertical walls), symmetric about the raphe:
//!   - ABO upper: heptagon. Heels at `heel_angle_deg` (interior, to the
//!     back line) `heel_length_mm` long, sides at `side_angle_deg` to the
//!     back line `side_clear_mm` outside the anatomy, two front cuts at
//!     `front_angle_deg` to the back line through the canine tips plus
//!     `canine_clear_mm`, meeting on the raphe;
//!   - ABO lower: the same back and heels (flush), sides at
//!     `lower_side_angle_deg`, and an arc from canine to canine tangent to
//!     the sides, as round as the labial vestibule plus `labial_clear_mm`
//!     allows;
//!   - European: walls at 90° to the base; sides along the buccal segments
//!     (or `eu_side_angle_deg`), optional heels; upper front tip on the
//!     raphe with the lateral corners at the canines, lower front flat with
//!     the corners at the canines; clearance `vestibule_clear_mm`.
//! * Solid: the scan of the protected anatomy as it is, undercuts included
//!   (nothing of it is filled or moved): its holes capped, its open border
//!   walled straight down into the slab of the outline, from the base plane
//!   up to the land (the art portion `z = −A`, or [`LAND_GAP_MM`] under the
//!   deepest protected point when that is higher, so that the walls rise to
//!   the land without burying the sulcus), a piece with no open border
//!   standing on the land on its lowest [`LAND_GAP_MM`]; thickened into a
//!   shell, filled where it encloses and brought back onto the scan, cut by
//!   the outline prism from the base plane; the narrow-band SDF of `dc_geom`
//!   meshes it closed and manifold. Optional hollowing and drain holes are
//!   not built.
//!
//! The report gives the frame, the planes, the outline, the design angles
//! and the angles and heights measured back on the meshes.

mod outline;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};

use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DMat4, DVec2, DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use outline::{HalfPlane, Outline};

use crate::anatomy::Arch;
use crate::frame::{self, ArchFrame};

/// Style of the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// European (Leipzig / Italian textbooks): walls at 90°, 10 mm base.
    #[default]
    European,
    /// American Board of Orthodontics: 70 mm pair, 65°/115° heptagon.
    Abo,
}

/// Parameters of the bases (defaults from the specification).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Style of the base.
    pub style: Style,
    /// ABO: height of the articulated pair, base to base (mm).
    pub total_height_mm: f64,
    /// ABO: fraction of each model taken by the art portion.
    pub art_fraction: f64,
    /// ABO: thickness of the art portion (mm), overriding `art_fraction`.
    pub art_thickness_mm: Option<f64>,
    /// European: thickness of the base under the anatomy (mm).
    pub base_thickness_mm: f64,
    /// Clearance of the base plane under the vault and the deepest sulcus (mm).
    pub m_base_mm: f64,
    /// Distance of the back plane behind the most distal anatomy (mm).
    pub back_margin_mm: f64,
    /// ABO: angle of the upper side cuts to the back line (degrees).
    pub side_angle_deg: f64,
    /// ABO: angle of the lower side cuts to the back line (degrees).
    pub lower_side_angle_deg: f64,
    /// Interior angle of the heels with the back plane (degrees).
    pub heel_angle_deg: f64,
    /// Length of the heel cuts (mm).
    pub heel_length_mm: f64,
    /// ABO: angle of the upper front cuts to the back line (degrees).
    pub front_angle_deg: f64,
    /// ABO: side cuts outside the deepest sulcus (mm).
    pub side_clear_mm: f64,
    /// ABO: upper front cuts outside the canine tips (mm).
    pub canine_clear_mm: f64,
    /// ABO: lower front arc outside the labial vestibule (mm).
    pub labial_clear_mm: f64,
    /// European: walls outside the vestibule (mm).
    pub vestibule_clear_mm: f64,
    /// European: angle of the sides to the back line (degrees); default
    /// along the buccal segments of the upper arch.
    pub eu_side_angle_deg: Option<f64>,
    /// European: cut the heels (at `heel_angle_deg`, `heel_length_mm`).
    pub eu_heels: bool,
    /// Reach of the protected gingiva around the teeth, plan (mm).
    pub sulcus_reach_mm: f64,
    /// Detail of the solid (mm).
    pub voxel_mm: f64,
    /// Surface tolerance of the output (mm); `0` keeps the full resolution.
    pub tolerance_mm: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            style: Style::European,
            total_height_mm: 70.0,
            art_fraction: 1.0 / 3.0,
            art_thickness_mm: None,
            base_thickness_mm: 10.0,
            m_base_mm: 3.0,
            back_margin_mm: 2.0,
            side_angle_deg: 65.0,
            lower_side_angle_deg: 65.0,
            heel_angle_deg: 115.0,
            heel_length_mm: 10.0,
            front_angle_deg: 25.0,
            side_clear_mm: 2.5,
            canine_clear_mm: 2.0,
            labial_clear_mm: 2.0,
            vestibule_clear_mm: 3.0,
            eu_side_angle_deg: None,
            eu_heels: false,
            sulcus_reach_mm: 8.0,
            voxel_mm: 0.25,
            tolerance_mm: 0.02,
        }
    }
}

impl Params {
    /// Checks the ranges of the parameters.
    ///
    /// # Errors
    /// [`StudyBaseError::Invalid`] naming the first parameter out of range.
    pub fn check(&self) -> Result<(), StudyBaseError> {
        let positive = [
            ("total_height_mm", self.total_height_mm),
            ("base_thickness_mm", self.base_thickness_mm),
            ("heel_length_mm", self.heel_length_mm),
            ("sulcus_reach_mm", self.sulcus_reach_mm),
            ("voxel_mm", self.voxel_mm),
        ];
        for (what, v) in positive {
            if !(v.is_finite() && v > 0.0) {
                return Err(StudyBaseError::Invalid(format!(
                    "{what} must be positive, got {v}"
                )));
            }
        }
        let non_negative = [
            ("m_base_mm", self.m_base_mm),
            ("back_margin_mm", self.back_margin_mm),
            ("side_clear_mm", self.side_clear_mm),
            ("canine_clear_mm", self.canine_clear_mm),
            ("labial_clear_mm", self.labial_clear_mm),
            ("vestibule_clear_mm", self.vestibule_clear_mm),
            ("tolerance_mm", self.tolerance_mm),
        ];
        for (what, v) in non_negative {
            if !(v.is_finite() && v >= 0.0) {
                return Err(StudyBaseError::Invalid(format!(
                    "{what} must be zero or positive, got {v}"
                )));
            }
        }
        if !(self.art_fraction > 0.0 && self.art_fraction < 1.0) {
            return Err(StudyBaseError::Invalid(format!(
                "art_fraction must be between 0 and 1, got {}",
                self.art_fraction
            )));
        }
        if let Some(t) = self.art_thickness_mm
            && !(t.is_finite() && t > 0.0)
        {
            return Err(StudyBaseError::Invalid(format!(
                "art_thickness_mm must be positive, got {t}"
            )));
        }
        let angles = [
            ("side_angle_deg", self.side_angle_deg, 30.0, 90.0),
            (
                "lower_side_angle_deg",
                self.lower_side_angle_deg,
                30.0,
                90.0,
            ),
            ("heel_angle_deg", self.heel_angle_deg, 91.0, 170.0),
            ("front_angle_deg", self.front_angle_deg, 0.0, 60.0),
            (
                "eu_side_angle_deg",
                self.eu_side_angle_deg.unwrap_or(80.0),
                30.0,
                90.0,
            ),
        ];
        for (what, v, lo, hi) in angles {
            if !(v.is_finite() && (lo..=hi).contains(&v)) {
                return Err(StudyBaseError::Invalid(format!(
                    "{what} must be within {lo}..{hi} degrees, got {v}"
                )));
            }
        }
        Ok(())
    }
}

/// Errors of the study-base stage.
#[derive(Debug, thiserror::Error)]
pub enum StudyBaseError {
    /// Invalid input or parameters.
    #[error("{0}")]
    Invalid(String),
    /// The solid could not be built.
    #[error("geometry: {0}")]
    Geometry(#[from] dc_geom::GeomError),
}

/// One arch: its scan in occlusion (case coordinates) and its tooth labels.
#[derive(Debug, Clone, Copy)]
pub struct ArchInput<'a> {
    /// The scan (open surface, teeth and gingiva).
    pub scan: &'a TriMesh,
    /// FDI number per scan vertex, `0` for the gingiva.
    pub labels: &'a [u16],
    /// The arch frame (seeds or mesh), for the fallbacks of the frame.
    pub arch_frame: ArchFrame,
}

/// The arches to base (at least one), in occlusion in case coordinates.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input<'a> {
    /// The upper arch.
    pub upper: Option<ArchInput<'a>>,
    /// The lower arch.
    pub lower: Option<ArchInput<'a>>,
}

/// A plane (case coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Plane {
    /// A point of the plane.
    pub point: [f64; 3],
    /// Unit normal.
    pub normal: [f64; 3],
}

impl Plane {
    fn new(point: DVec3, normal: DVec3) -> Self {
        Self {
            point: point.to_array(),
            normal: normal.normalize().to_array(),
        }
    }
}

/// The common occlusion frame (case coordinates).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommonFrame {
    /// Origin, on the occlusal plane and the raphe.
    pub origin: [f64; 3],
    /// Lateral axis (the patient's right).
    pub x: [f64; 3],
    /// Raphe, anterior.
    pub y: [f64; 3],
    /// Occlusal-plane normal, lower to upper.
    pub z: [f64; 3],
    /// Where the raphe came from: `upper_symmetry`, `lower_symmetry` or
    /// `arch_frame`.
    pub raphe_source: String,
    /// Where the occlusal plane came from: `cusp_tips` or `arch_frame`.
    pub plane_source: String,
}

/// A warning of the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    /// Stable code (`height_raised`, `art_portion_reached`, `wall_pushed`,
    /// `no_canine`, `heel_missing`, `mirrored_labels`, `single_arch`).
    pub code: String,
    /// Arch (`upper`, `lower`) or empty.
    pub arch: String,
    /// Details for the reader.
    pub detail: String,
}

/// A wall measured on the mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WallMeasure {
    /// Name of the wall.
    pub name: String,
    /// Area of the faces found on it (mm²).
    pub area_mm2: f64,
    /// Angle of the wall to the base plane (degrees, 90 = vertical).
    pub to_base_deg: f64,
    /// Angle between the measured and the designed wall normal (degrees).
    pub deviation_deg: f64,
    /// Measured outward normal (common frame).
    pub normal: [f64; 3],
    /// Mean signed offset along the normal (common frame, mm).
    pub offset_mm: f64,
}

/// What [`run`] built and measured for one arch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchReport {
    /// Distance of the base plane from the occlusal plane (mm).
    pub height_mm: f64,
    /// Anatomic height: occlusal plane to the top of the art portion (mm).
    pub anatomic_mm: f64,
    /// Thickness of the art portion (mm).
    pub art_mm: f64,
    /// Deepest protected anatomy below the occlusal plane (mm).
    pub depth_mm: f64,
    /// Depth of the land: the outline is solid from the base plane up to
    /// here (the art portion, or just under the deepest anatomy when that is
    /// higher) (mm).
    pub land_mm: f64,
    /// The base plane.
    pub base_plane: Plane,
    /// The outline (common frame, plan).
    pub outline: Outline,
    /// Interior angles of the outline as designed (degrees).
    pub design_angles_deg: BTreeMap<String, f64>,
    /// Interior angles measured between the walls of the mesh (degrees).
    pub measured_angles_deg: BTreeMap<String, f64>,
    /// The walls measured on the mesh.
    pub walls: Vec<WallMeasure>,
    /// Tilt of the measured base plane from the occlusal plane (degrees).
    pub base_tilt_deg: f64,
    /// Height measured on the mesh: occlusal plane to its base (mm).
    pub measured_height_mm: f64,
    /// Triangles of the base.
    pub faces: usize,
    /// Closed and manifold.
    pub watertight: bool,
    /// Connected pieces.
    pub components: usize,
    /// Seconds spent.
    pub seconds: f64,
}

/// The report of [`run`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Style built.
    pub style: Style,
    /// The common occlusion frame.
    pub frame: CommonFrame,
    /// The occlusal plane.
    pub occlusal_plane: Plane,
    /// The back plane, shared by both arches.
    pub back_plane: Plane,
    /// Per arch (`upper`, `lower`).
    pub arches: BTreeMap<String, ArchReport>,
    /// Upper base to lower base, designed (mm; both arches only).
    pub total_height_mm: Option<f64>,
    /// Upper base to lower base measured on the meshes (mm).
    pub measured_total_height_mm: Option<f64>,
    /// Angle between the back walls of the two meshes (degrees).
    pub back_coplanarity_deg: Option<f64>,
    /// Distance between the back walls of the two meshes (mm).
    pub back_offset_mm: Option<f64>,
    /// What was changed to keep the anatomy, and other notes.
    pub warnings: Vec<Warning>,
}

/// The trimmed models (case coordinates) and the report.
#[derive(Debug, Clone)]
pub struct Output {
    /// The upper model on its base.
    pub upper_base: Option<TriMesh>,
    /// The lower model on its base.
    pub lower_base: Option<TriMesh>,
    /// Planes, angles, heights, warnings.
    pub report: Report,
}

fn arch_key(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn warn(warnings: &mut Vec<Warning>, code: &str, arch: &str, detail: String) {
    warnings.push(Warning {
        code: code.to_owned(),
        arch: arch.to_owned(),
        detail,
    });
}

/// One arch prepared in the common frame.
struct Prepared<'a> {
    arch: Arch,
    input: ArchInput<'a>,
    /// Case → working frame (`+Z` occlusal).
    to_work: DMat4,
    /// Vertices in the working frame.
    v: Vec<DVec3>,
    /// Protected vertices.
    protected: Vec<bool>,
    /// Protected points in the common plan (mirrored copies included).
    plan: Vec<DVec2>,
    /// Tooth centroids, working frame.
    centroids: BTreeMap<u8, DVec3>,
    depth: f64,
}

impl Prepared<'_> {
    /// Working-frame point → common plan.
    fn to_plan(&self, p: DVec3) -> DVec2 {
        if self.arch.is_upper() {
            DVec2::new(-p.x, p.y)
        } else {
            DVec2::new(p.x, p.y)
        }
    }

    /// The canine tip in the common plan, averaged with the mirror of the
    /// other side (`|x|`, `y`); fallbacks: first premolar, lateral incisor.
    fn canine(&self) -> Option<DVec2> {
        let q = if self.arch.is_upper() {
            [1u8, 2]
        } else {
            [4, 3]
        };
        for pos in [3u8, 4, 2] {
            let tips: Vec<DVec2> = q
                .iter()
                .filter_map(|&qq| self.tip(qq * 10 + pos))
                .map(|p| DVec2::new(p.x.abs(), p.y))
                .collect();
            if !tips.is_empty() {
                return Some(tips.iter().copied().sum::<DVec2>() / tips.len() as f64);
            }
        }
        None
    }

    /// Highest vertex (working `+Z`) of the tooth `fdi`, common plan.
    fn tip(&self, fdi: u8) -> Option<DVec2> {
        let mut best: Option<DVec3> = None;
        for (p, &l) in self.v.iter().zip(self.input.labels) {
            if l == u16::from(fdi) && best.is_none_or(|b| p.z > b.z) {
                best = Some(*p);
            }
        }
        best.map(|p| self.to_plan(p))
    }
}

fn to_vec(m: &TriMesh) -> Vec<DVec3> {
    m.positions.iter().map(|p| p.as_dvec3()).collect()
}

/// Centroid of every labelled tooth (case coordinates).
fn tooth_centroids(v: &[DVec3], labels: &[u16]) -> BTreeMap<u8, DVec3> {
    let mut acc: BTreeMap<u8, (DVec3, f64)> = BTreeMap::new();
    for (p, &l) in v.iter().zip(labels) {
        if l > 0 && l < 100 {
            let e = acc.entry(l as u8).or_insert((DVec3::ZERO, 0.0));
            e.0 += *p;
            e.1 += 1.0;
        }
    }
    acc.into_iter().map(|(k, (s, n))| (k, s / n)).collect()
}

/// Cusp tip of every tooth: its farthest vertex along `dir`.
fn cusp_tips(v: &[DVec3], labels: &[u16], dir: DVec3) -> Vec<DVec3> {
    let mut best: BTreeMap<u16, DVec3> = BTreeMap::new();
    for (p, &l) in v.iter().zip(labels) {
        if l > 0 {
            let e = best.entry(l).or_insert(*p);
            if p.dot(dir) > e.dot(dir) {
                *e = *p;
            }
        }
    }
    best.into_values().collect()
}

/// The symmetry axis of one arch from its contralateral tooth pairs:
/// (unit transverse vector towards the patient's right, a point of the
/// midline), in case coordinates.
fn symmetry(centroids: &BTreeMap<u8, DVec3>, arch: Arch) -> Option<(DVec3, DVec3)> {
    let (right, left) = if arch.is_upper() { (1u8, 2u8) } else { (4, 3) };
    let mut t = DVec3::ZERO;
    let mut mid = DVec3::ZERO;
    let mut n = 0.0;
    for pos in 1..=8u8 {
        if let (Some(r), Some(l)) = (
            centroids.get(&(right * 10 + pos)),
            centroids.get(&(left * 10 + pos)),
        ) {
            let d = *r - *l;
            if d.length() > 1e-6 {
                t += d.normalize();
                mid += (*r + *l) * 0.5;
                n += 1.0;
            }
        }
    }
    (n > 0.0 && t.length() > 1e-9).then(|| (t.normalize(), mid / n))
}

/// The common occlusion frame (common → case) and its report.
fn common_frame(
    upper: Option<(&[DVec3], &ArchInput<'_>)>,
    lower: Option<(&[DVec3], &ArchInput<'_>)>,
    warnings: &mut Vec<Warning>,
) -> Result<(DMat4, CommonFrame), StudyBaseError> {
    // up: lower occlusal direction, else the opposite of the upper one
    let n0 = match (lower, upper) {
        (Some((_, l)), _) => l.arch_frame.n,
        (None, Some((_, u))) => -u.arch_frame.n,
        _ => return Err(StudyBaseError::Invalid("no arch to base".into())),
    }
    .normalize();
    let mut tips = Vec::new();
    if let Some((v, a)) = lower {
        tips.extend(cusp_tips(v, a.labels, n0));
    }
    if let Some((v, a)) = upper {
        tips.extend(cusp_tips(v, a.labels, -n0));
    }
    let (mut z, mut plane_source) = (n0, "arch_frame");
    let mut origin = if let Some(p) = frame::pca(&tips).filter(|_| tips.len() >= 3) {
        let mut nz = p.axes[2];
        if nz.dot(n0) < 0.0 {
            nz = -nz;
        }
        if nz.dot(n0) > 30f64.to_radians().cos() {
            z = nz;
            plane_source = "cusp_tips";
        }
        p.mean
    } else {
        lower.or(upper).map(|(_, a)| a.arch_frame.origin).unwrap()
    };
    // raphe: symmetry of the upper arch, else of the lower, else arch frame
    let sym = |arch: Arch, side: Option<(&[DVec3], &ArchInput<'_>)>| {
        side.and_then(|(v, a)| symmetry(&tooth_centroids(v, a.labels), arch))
    };
    let (x, mid, raphe_source) = if let Some((t, m)) = sym(Arch::Upper, upper) {
        (t, Some(m), "upper_symmetry")
    } else if let Some((t, m)) = sym(Arch::Lower, lower) {
        (t, Some(m), "lower_symmetry")
    } else {
        let a = lower.or(upper).map(|(_, a)| a.arch_frame).unwrap();
        (a.y.cross(z), None, "arch_frame")
    };
    let mut x = (x - z * x.dot(z)).normalize();
    let mut y = z.cross(x);
    // the seeds' frame tells the incisors' side; disagreeing labels are mirrored
    let hint = lower.or(upper).map(|(_, a)| a.arch_frame.y).unwrap();
    if y.dot(hint) < 0.0 {
        warn(
            warnings,
            "mirrored_labels",
            "",
            "the tooth labels put the incisors behind the molars: the left/right labels look swapped".into(),
        );
        x = -x;
        y = -y;
    }
    if let Some(m) = mid {
        origin += x * x.dot(m - origin);
    }
    let m = frame::make_frame(x, y, z, origin);
    Ok((
        m,
        CommonFrame {
            origin: origin.to_array(),
            x: x.to_array(),
            y: y.to_array(),
            z: z.to_array(),
            raphe_source: raphe_source.to_owned(),
            plane_source: plane_source.to_owned(),
        },
    ))
}

/// The land stays at least this far under the deepest protected point (mm).
pub const LAND_GAP_MM: f64 = 1.0;

/// Plan cell of the protection masks (mm).
const MASK_CELL: f64 = 0.5;

/// Marks the vertices within `reach` (plan) of a tooth vertex; for the upper
/// arch also the palate inside the convex hull of the teeth.
fn protect(v: &[DVec3], labels: &[u16], reach: f64, palate: bool) -> Vec<bool> {
    let teeth: Vec<DVec2> = v
        .iter()
        .zip(labels)
        .filter(|(_, l)| **l > 0)
        .map(|(p, _)| p.truncate())
        .collect();
    let key = |p: DVec2| {
        (
            (p.x / MASK_CELL).floor() as i64,
            (p.y / MASK_CELL).floor() as i64,
        )
    };
    let mut seeds: Vec<(i64, i64)> = teeth.iter().map(|p| key(*p)).collect();
    seeds.sort_unstable();
    seeds.dedup();
    let r = (reach / MASK_CELL).ceil() as i64;
    let r2 = (reach / MASK_CELL).powi(2);
    let mut mask: HashMap<(i64, i64), ()> = HashMap::with_capacity(seeds.len() * 8);
    for &(i, j) in &seeds {
        for di in -r..=r {
            for dj in -r..=r {
                if ((di * di + dj * dj) as f64) <= r2 {
                    mask.insert((i + di, j + dj), ());
                }
            }
        }
    }
    let hull = if palate {
        convex_hull(&teeth)
    } else {
        Vec::new()
    };
    v.iter()
        .zip(labels)
        .map(|(p, l)| {
            *l > 0
                || mask.contains_key(&key(p.truncate()))
                || (hull.len() >= 3 && in_convex(&hull, p.truncate()))
        })
        .collect()
}

/// Convex hull, counter-clockwise (monotone chain).
fn convex_hull(pts: &[DVec2]) -> Vec<DVec2> {
    let mut p: Vec<DVec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<DVec2> = Vec::with_capacity(2 * p.len());
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for q in it {
            while h.len() >= start + 2 {
                let (a, b) = (h[h.len() - 2], h[h.len() - 1]);
                if (b - a).perp_dot(*q - a) <= 0.0 {
                    h.pop();
                } else {
                    break;
                }
            }
            h.push(*q);
        }
        h.pop();
    }
    h
}

fn in_convex(hull: &[DVec2], p: DVec2) -> bool {
    (0..hull.len()).all(|i| {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        (b - a).perp_dot(p - a) >= 0.0
    })
}

/// Angle (degrees, 0..90] of a plan direction to the back line (`x`).
fn line_angle_to_back(d: DVec2) -> f64 {
    d.y.abs().atan2(d.x.abs()).to_degrees()
}

/// Side angle of the European style: along the buccal segments (centroids
/// of the premolars and molars) of the arch, both sides averaged.
fn buccal_side_angle(p: &Prepared<'_>) -> Option<f64> {
    let q = if p.arch.is_upper() { [1u8, 2] } else { [4, 3] };
    let mut angles = Vec::new();
    for qq in q {
        let pts: Vec<DVec2> = (4..=7u8)
            .filter_map(|pos| p.centroids.get(&(qq * 10 + pos)))
            .map(|c| p.to_plan(*c))
            .collect();
        if pts.len() >= 2 {
            let m = pts.iter().copied().sum::<DVec2>() / pts.len() as f64;
            let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
            for q in &pts {
                let d = *q - m;
                sxx += d.x * d.x;
                sxy += d.x * d.y;
                syy += d.y * d.y;
            }
            let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
            angles.push(line_angle_to_back(DVec2::new(theta.cos(), theta.sin())));
        }
    }
    (!angles.is_empty())
        .then(|| (angles.iter().sum::<f64>() / angles.len() as f64).clamp(60.0, 90.0))
}

/// A pair of symmetric walls at `psi` (normal angle from `−y`), each pushed
/// out by `pts` + `clear`; both share the larger offset.
fn wall_pair(name: &str, psi: f64, pts: &[DVec2], clear: f64) -> [HalfPlane; 2] {
    let mut r = HalfPlane::at_angle(format!("{name}_r"), psi, 1.0);
    let mut l = HalfPlane::at_angle(format!("{name}_l"), psi, -1.0);
    r.clear_points(pts, clear);
    l.clear_points(pts, clear);
    let c = r.c.max(l.c);
    r.c = c;
    l.c = c;
    [r, l]
}

/// Heels of length `len` cutting the corners of `back` and the sides, but
/// never into `pts` + `clear`.
fn heels(
    back: &HalfPlane,
    sides: &[HalfPlane; 2],
    psi: f64,
    len: f64,
    pts: &[DVec2],
    clear: f64,
) -> [HalfPlane; 2] {
    let mut out = wall_pair("heel", psi, pts, clear);
    for (h, s) in out.iter_mut().zip(sides) {
        if let Some(k) = back.meet(s) {
            let a1 = back.n.angle_to(h.n).abs();
            let a2 = h.n.angle_to(s.n).abs();
            let depth = len / (1.0 / a1.tan() + 1.0 / a2.tan());
            h.c = h.c.max(h.n.dot(k) - depth);
        }
    }
    let c = out[0].c.max(out[1].c);
    out[0].c = c;
    out[1].c = c;
    out
}

/// The lower ABO front: arc tangent to the sides, the largest radius that
/// keeps `pts` + `clear` inside; as tangent walls every 2.5°.
fn lower_arc(sides: &[HalfPlane; 2], back_y: f64, pts: &[DVec2], clear: f64) -> Vec<HalfPlane> {
    let s = &sides[0];
    if s.n.y <= 1e-6 {
        return Vec::new(); // parallel sides: no apex, a flat front instead
    }
    let apex_y = s.c / s.n.y;
    let fits = |d: f64| {
        let o = DVec2::new(0.0, apex_y - d);
        let r = d * s.n.y;
        let ty = o.y + r * s.n.y;
        pts.iter()
            .all(|p| p.y <= ty || p.distance(o) + clear <= r + 1e-9)
    };
    let (mut lo, mut hi) = (0.0, (apex_y - back_y).max(0.0));
    if fits(hi) {
        lo = hi;
    } else {
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
    }
    let o = DVec2::new(0.0, apex_y - lo);
    let r = lo * s.n.y;
    let alpha_s = s.n.x.atan2(s.n.y).to_degrees();
    let mut out = Vec::new();
    let steps = (2.0 * alpha_s / 2.5).ceil().max(1.0) as usize;
    for k in 1..steps {
        let a = (-alpha_s + 2.0 * alpha_s * k as f64 / steps as f64).to_radians();
        let n = DVec2::new(a.sin(), a.cos());
        out.push(HalfPlane {
            name: format!("arc_{k}"),
            n,
            c: n.dot(o) + r,
        });
    }
    out
}

/// A closed prism of `poly` (plan, any orientation) from `z0` to `z1`.
fn prism(poly: &[DVec2], z0: f64, z1: f64) -> TriMesh {
    let n = poly.len();
    let ccw = {
        let a: f64 = (0..n).map(|i| poly[i].perp_dot(poly[(i + 1) % n])).sum();
        a > 0.0
    };
    let pts: Vec<DVec2> = if ccw {
        poly.to_vec()
    } else {
        poly.iter().rev().copied().collect()
    };
    let mut positions = Vec::with_capacity(2 * n);
    for z in [z0, z1] {
        positions.extend(
            pts.iter()
                .map(|p| Vec3::new(p.x as f32, p.y as f32, z as f32)),
        );
    }
    let n32 = n as u32;
    let mut tris = Vec::new();
    for i in 1..n32 - 1 {
        tris.push([0, i + 1, i]); // bottom, facing −Z
        tris.push([n32, n32 + i, n32 + i + 1]); // top, facing +Z
    }
    for i in 0..n32 {
        let j = (i + 1) % n32;
        tris.push([i, j, n32 + j]);
        tris.push([i, n32 + j, n32 + i]);
    }
    TriMesh::new(positions, tris)
}

/// A loop of the border of the protected scan shorter than this fraction of
/// the longest one of its piece is a hole of the scan (closed with a cap);
/// a longer one is an open border (walled down), such as the lingual
/// border of a lower arch whose protected scan goes round the tongue.
const HOLE_FRACTION: f64 = 0.25;

/// The protected scan, the triangles `tris` on the points `v` (working
/// frame, `+Z` occlusal), with what closes it as it is: nothing of it moves
/// and its undercuts stay. The points that share a position are joined, so
/// that only the real border is open. Each hole of a piece (a closed loop
/// of its border shorter than [`HOLE_FRACTION`] of its longest one) gets a
/// cap, a fan from its middle point; every other border edge (the open
/// border, and any chain that does not close) gets a wall straight down to
/// `floor`, the quad between the edge and its copy, as the plugin's
/// `make_base` walls a scan. An open surface, without a side: the caller
/// thickens it into a shell, ends the walls in the slab of the land and
/// fills what they enclose.
///
/// Also gives the feet of the pieces with no open border (a closed model,
/// e.g. the demo typodont, which nothing walls down): the triangles of each
/// within `band` of its lowest point, for the caller to stand it on the
/// land.
fn walled_anatomy(
    v: &[DVec3],
    tris: &[[u32; 3]],
    floor: f64,
    band: f64,
) -> (TriMesh, Vec<[u32; 3]>) {
    let key = |i: u32| {
        let p = v[i as usize];
        (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())
    };
    let mut order: Vec<u32> = (0..v.len() as u32).collect();
    order.sort_unstable_by_key(|&i| (key(i), i));
    let mut canon: Vec<u32> = (0..v.len() as u32).collect();
    for w in order.windows(2) {
        if key(w[0]) == key(w[1]) {
            canon[w[1] as usize] = canon[w[0] as usize];
        }
    }
    let sheet: Vec<[u32; 3]> = tris
        .iter()
        .map(|t| t.map(|i| canon[i as usize]))
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .collect();
    // the pieces: the root point of each point
    let mut parent: Vec<u32> = (0..v.len() as u32).collect();
    let find = |parent: &mut Vec<u32>, mut x: u32| {
        while parent[x as usize] != x {
            let up = parent[parent[x as usize] as usize];
            parent[x as usize] = up;
            x = up;
        }
        x
    };
    for t in &sheet {
        let r0 = find(&mut parent, t[0]);
        for &w in &t[1..] {
            let r = find(&mut parent, w);
            if r != r0 {
                parent[r as usize] = r0;
            }
        }
    }
    let piece: Vec<u32> = (0..v.len() as u32).map(|w| find(&mut parent, w)).collect();

    let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), sheet);
    let adjacency = mesh.edge_adjacency();
    let border: Vec<(u32, u32)> = mesh
        .tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .filter(|&(a, b)| adjacency.faces_of(a, b).len() == 1)
        .collect();
    let half: HashSet<(u32, u32)> = border.iter().copied().collect();
    let loops: Vec<Vec<u32>> = adjacency
        .boundary_loops()
        .into_iter()
        .filter(|lp| lp.len() >= 3 && half.contains(&(lp[lp.len() - 1], lp[0])))
        .collect();
    // the longest closed loop of each piece is its open border
    let lengths: Vec<f64> = loops
        .iter()
        .map(|lp| {
            (0..lp.len())
                .map(|i| v[lp[i] as usize].distance(v[lp[(i + 1) % lp.len()] as usize]))
                .sum()
        })
        .collect();
    let mut longest: HashMap<u32, (f64, usize)> = HashMap::new();
    for (k, lp) in loops.iter().enumerate() {
        let e = longest
            .entry(piece[lp[0] as usize])
            .or_insert((lengths[k], k));
        if lengths[k] > e.0 {
            *e = (lengths[k], k);
        }
    }
    let open: HashSet<u32> = border.iter().map(|&(a, _)| piece[a as usize]).collect();
    let mut lowest: HashMap<u32, f64> = HashMap::new();
    for t in &mesh.tris {
        let r = piece[t[0] as usize];
        if !open.contains(&r) {
            let z = t
                .iter()
                .map(|&w| v[w as usize].z)
                .fold(f64::INFINITY, f64::min);
            let e = lowest.entry(r).or_insert(z);
            *e = e.min(z);
        }
    }
    let feet: Vec<[u32; 3]> = mesh
        .tris
        .iter()
        .filter(|t| {
            lowest
                .get(&piece[t[0] as usize])
                .is_some_and(|&z0| t.iter().all(|&w| v[w as usize].z <= z0 + band))
        })
        .copied()
        .collect();

    let TriMesh {
        mut positions,
        mut tris,
        ..
    } = mesh;
    let mut capped: HashSet<(u32, u32)> = HashSet::new();
    for (k, lp) in loops.iter().enumerate() {
        let (top, border_loop) = longest[&piece[lp[0] as usize]];
        if border_loop == k || lengths[k] >= HOLE_FRACTION * top {
            continue;
        }
        let c = positions.len() as u32;
        let mid = lp.iter().map(|&w| v[w as usize]).sum::<DVec3>() / lp.len() as f64;
        positions.push(mid.as_vec3());
        for i in 0..lp.len() {
            let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
            tris.push([b, a, c]);
            capped.insert((a, b));
        }
    }
    let mut copy: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &border {
        if capped.contains(&(a, b)) {
            continue;
        }
        let [a2, b2] = [a, b].map(|w| {
            *copy.entry(w).or_insert_with(|| {
                let p = v[w as usize];
                positions.push(DVec3::new(p.x, p.y, floor).as_vec3());
                (positions.len() - 1) as u32
            })
        });
        tris.push([b, a, a2]);
        tris.push([b, a2, b2]);
    }
    (TriMesh::new(positions, tris), feet)
}

/// A mesh moved by `m`.
fn moved(mesh: &TriMesh, m: &DMat4) -> TriMesh {
    TriMesh::new(
        mesh.positions
            .iter()
            .map(|p| m.transform_point3(p.as_dvec3()).as_vec3())
            .collect(),
        mesh.tris.clone(),
    )
}

/// The faces of `mesh` (common frame) whose normal is within `max_deg` of
/// `n` and whose centroid lies within `near` of the plane `n · p = c`:
/// (area, area-weighted normal, mean offset).
fn faces_on(mesh: &TriMesh, n: DVec3, c: f64, near: f64, max_deg: f64) -> (f64, DVec3, f64) {
    let cos = max_deg.to_radians().cos();
    let (mut area, mut nsum, mut off) = (0.0, DVec3::ZERO, 0.0);
    for f in 0..mesh.tris.len() {
        let [a, b, cc] = mesh.triangle(f).map(|p| p.as_dvec3());
        let raw = (b - a).cross(cc - a);
        let ar = 0.5 * raw.length();
        if ar < 1e-12 {
            continue;
        }
        let fnorm = raw / (2.0 * ar);
        let cen = (a + b + cc) / 3.0;
        if fnorm.dot(n) >= cos && (n.dot(cen) - c).abs() <= near {
            area += ar;
            nsum += fnorm * ar;
            off += n.dot(cen) * ar;
        }
    }
    if area > 0.0 {
        (area, nsum.normalize(), off / area)
    } else {
        (0.0, n, c)
    }
}

/// Builds the bases of the arches of `input`.
///
/// # Errors
/// [`StudyBaseError::Invalid`] for bad parameters, no arch, labels that do
/// not match the scan or an arch without teeth; [`StudyBaseError::Geometry`]
/// when the solid cannot be built.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, StudyBaseError> {
    params.check()?;
    let mut warnings = Vec::new();
    let arches: Vec<(Arch, ArchInput<'_>)> =
        [(Arch::Upper, input.upper), (Arch::Lower, input.lower)]
            .into_iter()
            .filter_map(|(a, i)| i.map(|i| (a, i)))
            .collect();
    if arches.is_empty() {
        return Err(StudyBaseError::Invalid("no arch to base".into()));
    }
    let mut case_v = Vec::new();
    for (arch, a) in &arches {
        if a.labels.len() != a.scan.positions.len() {
            return Err(StudyBaseError::Invalid(format!(
                "the {} labels have {} entries for {} vertices",
                arch_key(*arch),
                a.labels.len(),
                a.scan.positions.len()
            )));
        }
        if !a.labels.iter().any(|l| *l > 0) {
            return Err(StudyBaseError::Invalid(format!(
                "the {} scan has no labelled tooth: run the segmentation first",
                arch_key(*arch)
            )));
        }
        case_v.push(to_vec(a.scan));
    }
    if arches.len() == 1 {
        warn(
            &mut warnings,
            "single_arch",
            arch_key(arches[0].0),
            "only one arch: the back plane and the outline come from it alone".into(),
        );
    }
    let side = |want: Arch| {
        arches
            .iter()
            .zip(&case_v)
            .find(|((a, _), _)| *a == want)
            .map(|((_, i), v)| (v.as_slice(), i))
    };
    let (to_case, frame_report) =
        common_frame(side(Arch::Upper), side(Arch::Lower), &mut warnings)?;
    let to_common = frame::rigid_inverse(&to_case);
    let flip = DMat4::from_cols_array(&[
        -1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, -1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    // landmarks and protected anatomy per arch
    let mut prepared: Vec<Prepared<'_>> = Vec::new();
    for ((arch, a), cv) in arches.iter().zip(&case_v) {
        let to_work = if arch.is_upper() {
            flip * to_common
        } else {
            to_common
        };
        let v: Vec<DVec3> = cv.iter().map(|p| to_work.transform_point3(*p)).collect();
        let protected = protect(&v, a.labels, params.sulcus_reach_mm, arch.is_upper());
        let depth = v
            .iter()
            .zip(&protected)
            .filter(|(_, k)| **k)
            .map(|(p, _)| -p.z)
            .fold(0.0f64, f64::max);
        let centroids = tooth_centroids(&v, a.labels);
        let mut p = Prepared {
            arch: *arch,
            input: *a,
            to_work,
            v,
            protected,
            plan: Vec::new(),
            centroids,
            depth,
        };
        let mut plan: Vec<DVec2> =
            p.v.iter()
                .zip(&p.protected)
                .filter(|(_, k)| **k)
                .map(|(q, _)| p.to_plan(*q))
                .collect();
        // a plan cloud thinned to 0.25 mm cells, mirrored about the raphe
        plan.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        let mut seen = std::collections::HashSet::new();
        plan.retain(|q| seen.insert(((q.x * 4.0).floor() as i64, (q.y * 4.0).floor() as i64)));
        let mirrored: Vec<DVec2> = plan.iter().map(|q| DVec2::new(-q.x, q.y)).collect();
        plan.extend(mirrored);
        p.plan = plan;
        prepared.push(p);
    }
    let all_plan: Vec<DVec2> = prepared
        .iter()
        .flat_map(|p| p.plan.iter().copied())
        .collect();
    let find = |want: Arch| prepared.iter().find(|p| p.arch == want);

    // heights
    let mut heights: BTreeMap<&'static str, (f64, f64)> = BTreeMap::new(); // (H, art)
    for p in &prepared {
        let key = arch_key(p.arch);
        let need = p.depth + params.m_base_mm;
        let (mut h, art) = match params.style {
            Style::Abo => {
                let h = 0.5 * params.total_height_mm;
                (
                    h,
                    params.art_thickness_mm.unwrap_or(h * params.art_fraction),
                )
            }
            Style::European => (need + params.base_thickness_mm, params.base_thickness_mm),
        };
        if h < need {
            warn(
                &mut warnings,
                "height_raised",
                key,
                format!(
                    "the {key} base plane moved from {h:.2} to {need:.2} mm below the occlusal plane to clear the anatomy ({:.2} mm deep) by {:.2} mm",
                    p.depth, params.m_base_mm
                ),
            );
            h = need;
        }
        if params.style == Style::Abo && p.depth > h - art {
            warn(
                &mut warnings,
                "art_portion_reached",
                key,
                format!(
                    "the {key} anatomy ({:.2} mm deep) reaches into the art portion (from {:.2} mm)",
                    p.depth,
                    h - art
                ),
            );
        }
        heights.insert(key, (h, art.min(h)));
    }

    // back wall, shared
    let mut back = HalfPlane::at_angle("back", 0.0, 1.0);
    back.clear_points(&all_plan, params.back_margin_mm);
    let back_y = -back.c;

    // outlines
    let upper_ref = find(Arch::Upper).or_else(|| find(Arch::Lower)).unwrap();
    let mut outlines: BTreeMap<&'static str, Outline> = BTreeMap::new();
    let mut design: BTreeMap<&'static str, BTreeMap<String, f64>> = BTreeMap::new();
    for p in &prepared {
        let key = arch_key(p.arch);
        let own = &p.plan;
        let canine = p.canine();
        if canine.is_none() {
            warn(
                &mut warnings,
                "no_canine",
                key,
                format!(
                    "the {key} arch has no canine, premolar or lateral incisor labelled: the front follows the anatomy only"
                ),
            );
        }
        let mut walls = vec![back.clone()];
        let mut names: Vec<(&str, &str, &str)> = Vec::new(); // (angle name, wall a, wall b)
        match params.style {
            Style::Abo => {
                let side_deg = if p.arch.is_upper() {
                    params.side_angle_deg
                } else {
                    params.lower_side_angle_deg
                };
                // the sides of both arches clear the anatomy of both (flush pair)
                let sides = wall_pair("side", 180.0 - side_deg, &all_plan, params.side_clear_mm);
                // heels from the upper sides, so that they are flush
                let up_sides = wall_pair(
                    "side",
                    180.0 - params.side_angle_deg,
                    &all_plan,
                    params.side_clear_mm,
                );
                let hs = heels(
                    &back,
                    &up_sides,
                    180.0 - params.heel_angle_deg,
                    params.heel_length_mm,
                    &all_plan,
                    params.side_clear_mm,
                );
                walls.extend(hs);
                walls.extend(sides.clone());
                names.push(("back_heel", "back", "heel_r"));
                names.push(("heel_side", "heel_r", "side_r"));
                if p.arch.is_upper() {
                    let mut fronts = wall_pair(
                        "front",
                        180.0 - params.front_angle_deg,
                        own,
                        params.canine_clear_mm,
                    );
                    if let Some(c) = canine {
                        for f in &mut fronts {
                            let cs = DVec2::new(f.n.x.signum() * c.x, c.y);
                            f.through(cs + f.n * params.canine_clear_mm);
                        }
                        let cc = fronts[0].c.max(fronts[1].c);
                        fronts[0].c = cc;
                        fronts[1].c = cc;
                    }
                    walls.extend(fronts);
                    names.push(("side_front", "side_r", "front_r"));
                    names.push(("apex", "front_r", "front_l"));
                } else {
                    let arc = lower_arc(&sides, back_y, own, params.labial_clear_mm);
                    if arc.is_empty() {
                        let mut f = HalfPlane::at_angle("front", 180.0, 1.0);
                        f.clear_points(own, params.labial_clear_mm);
                        walls.push(f);
                    } else {
                        walls.extend(arc);
                    }
                }
            }
            Style::European => {
                let side_deg = params
                    .eu_side_angle_deg
                    .or_else(|| buccal_side_angle(upper_ref))
                    .unwrap_or(90.0);
                let sides = wall_pair(
                    "side",
                    180.0 - side_deg,
                    &all_plan,
                    params.vestibule_clear_mm,
                );
                if params.eu_heels {
                    let hs = heels(
                        &back,
                        &sides,
                        180.0 - params.heel_angle_deg,
                        params.heel_length_mm,
                        &all_plan,
                        params.vestibule_clear_mm,
                    );
                    walls.extend(hs);
                    names.push(("back_heel", "back", "heel_r"));
                    names.push(("heel_side", "heel_r", "side_r"));
                } else {
                    names.push(("back_side", "back", "side_r"));
                }
                walls.extend(sides);
                let clear = params.vestibule_clear_mm;
                let front_y = own.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max) + clear;
                if p.arch.is_upper() {
                    // tip on the raphe in front of the incisors, corners at the canines
                    let apex = DVec2::new(0.0, front_y);
                    let corner = canine.map_or(DVec2::new(20.0, front_y - 15.0), |c| {
                        DVec2::new(c.x + clear, c.y)
                    });
                    let d = (apex - corner).normalize();
                    let psi = 180.0 - line_angle_to_back(d);
                    let fronts = wall_pair("front", psi, own, clear);
                    let mut fronts = fronts;
                    for f in &mut fronts {
                        f.through(apex);
                    }
                    walls.extend(fronts);
                    names.push(("side_front", "side_r", "front_r"));
                    names.push(("apex", "front_r", "front_l"));
                } else {
                    // flat front, corners cut at the canines (bisecting side and front)
                    let mut f = HalfPlane::at_angle("front", 180.0, 1.0);
                    f.clear_points(own, clear);
                    let psi = 0.5 * (180.0 + (180.0 - side_deg));
                    let mut corners = wall_pair("corner", psi, own, clear);
                    if let Some(c) = canine {
                        for k in &mut corners {
                            k.through(DVec2::new(k.n.x.signum() * (c.x + clear), c.y));
                        }
                        let cc = corners[0].c.max(corners[1].c);
                        corners[0].c = cc;
                        corners[1].c = cc;
                    }
                    walls.extend(corners);
                    walls.push(f);
                    names.push(("side_corner", "side_r", "corner_r"));
                    names.push(("corner_front", "corner_r", "front"));
                }
            }
        }
        let o = Outline::new(walls);
        for q in own {
            if !o.contains(*q, 0.0) {
                warn(
                    &mut warnings,
                    "wall_pushed",
                    key,
                    format!(
                        "protected anatomy at ({:.1}, {:.1}) outside the {key} outline",
                        q.x, q.y
                    ),
                );
                break;
            }
        }
        for heel in ["heel_r", "heel_l"] {
            if o.walls.iter().any(|w| w.name == heel) && o.edge(heel).is_none() {
                warn(
                    &mut warnings,
                    "heel_missing",
                    key,
                    format!(
                        "the {key} {heel} cut falls outside the outline (the anatomy reaches the corner)"
                    ),
                );
            }
        }
        let mut angles = BTreeMap::new();
        for (name, a, b) in names {
            if let Some(v) = o.interior_deg(a, b) {
                angles.insert(name.to_owned(), v);
            }
        }
        if let (Some(s), Some(b)) = (o.edge("side_r"), o.edge("back")) {
            angles.insert(
                "side_to_back_line".to_owned(),
                180.0 - s.n.angle_to(b.n).abs().to_degrees(),
            );
        }
        if let Some(f) = o.edge("front_r") {
            angles.insert(
                "front_to_back_line".to_owned(),
                f.n.angle_to(DVec2::Y).abs().to_degrees(),
            );
        }
        design.insert(key, angles);
        outlines.insert(key, o);
    }

    // solids
    let vs = params.voxel_mm as f32;
    let sdf = SdfParams {
        voxel_size: vs,
        band: 4.0 * vs,
    };
    let mut out_meshes: BTreeMap<&'static str, TriMesh> = BTreeMap::new();
    let mut reports: BTreeMap<String, ArchReport> = BTreeMap::new();
    let z_axis = DVec3::from(frame_report.z);
    for p in &prepared {
        let t0 = std::time::Instant::now();
        let key = arch_key(p.arch);
        let (h, art) = heights[key];
        let a = h - art;
        let o = &outlines[key];
        let work_poly: Vec<DVec2> = o
            .points
            .iter()
            .map(|q| {
                if p.arch.is_upper() {
                    DVec2::new(-q.x, q.y)
                } else {
                    *q
                }
            })
            .collect();
        // the land: the outline is solid up to just under the deepest
        // protected point (or the art portion when that is higher), so no
        // anatomy is buried and no surface grazes it
        let land = a.min(p.depth + (4.0 * params.voxel_mm).max(LAND_GAP_MM));
        // the protected scan as it is, its holes capped and its open border
        // walled straight down into the slab of the land, which spans the
        // anatomy and the outline from under the base plane: as a shell of
        // 1.5 voxels (whatever the winding of the scan, its walls crossing
        // each other or not), filled where they enclose, then brought back
        // onto the scan; the outline prism cuts it
        let keep: Vec<[u32; 3]> = p
            .input
            .scan
            .tris
            .iter()
            .filter(|t| t.iter().all(|&i| p.protected[i as usize]))
            .copied()
            .collect();
        let top = p.v.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max) + 2.0;
        let (walled, feet) = walled_anatomy(&p.v, &keep, -0.5 * (land + h), LAND_GAP_MM);
        drop(keep);
        let mut solid = SdfGrid::from_mesh(&walled, &sdf)?;
        drop(walled);
        solid.shift(0.75 * vs)?;
        // a piece with no open border stands on the land on its lowest part
        if !feet.is_empty() {
            let feet = TriMesh::new(p.v.iter().map(|q| q.as_vec3()).collect(), feet);
            let mut stand = SdfGrid::from_mesh(&feet, &sdf)?;
            drop(feet);
            stand.shift(0.75 * vs)?;
            stand.sweep_down((-h) as f32 - 2.0 * vs)?;
            solid.union(&stand)?;
        }
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for q in work_poly.iter().copied().chain(
            p.v.iter()
                .zip(&p.protected)
                .filter(|(_, k)| **k)
                .map(|(q, _)| q.truncate()),
        ) {
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let (lo, hi) = (lo - DVec2::ONE, hi + DVec2::ONE);
        let rect = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
        let slab = SdfGrid::from_mesh(&prism(&rect, -h - 2.0 * params.voxel_mm, -land), &sdf)?;
        solid.union(&slab)?;
        drop(slab);
        solid.fill_interior()?;
        solid.shift(-0.75 * vs)?;
        let walls = SdfGrid::from_mesh(&prism(&work_poly, -h, top), &sdf)?;
        solid.intersect(&walls)?;
        drop(walls);
        let full = solid.to_mesh()?;
        drop(solid);
        let to_case = frame::rigid_inverse(&p.to_work);
        let reduced = crate::shell::reduce_to(&full, params.tolerance_mm, Some(&to_case))
            .map_err(|e| StudyBaseError::Invalid(e.to_string()))?;
        drop(full);
        let case_mesh = moved(&reduced, &to_case);
        drop(reduced);

        // measures, in the common frame
        let common = moved(&case_mesh, &to_common);
        let sgn = if p.arch.is_upper() { 1.0 } else { -1.0 };
        let near = 0.5 + params.voxel_mm;
        let base_n = DVec3::Z * sgn;
        let (_, bn, _) = faces_on(&common, base_n, h, near, 10.0);
        let base_tilt = bn.angle_between(base_n).to_degrees();
        let mut walls_m = Vec::new();
        let mut measured_n: HashMap<String, DVec2> = HashMap::new();
        for i in 0..o.points.len() {
            let k = o.edge_wall[i];
            if k == usize::MAX {
                continue;
            }
            let w = &o.walls[k];
            if w.name.starts_with("arc_") || o.edge_length(&w.name) < 3.0 {
                continue;
            }
            let n3 = DVec3::new(w.n.x, w.n.y, 0.0);
            let (area, mn, off) = faces_on(&common, n3, w.c, near, 10.0);
            if area <= 0.0 {
                continue;
            }
            measured_n.insert(w.name.clone(), DVec2::new(mn.x, mn.y).normalize());
            walls_m.push(WallMeasure {
                name: w.name.clone(),
                area_mm2: area,
                to_base_deg: 90.0 - mn.z.abs().asin().to_degrees(),
                deviation_deg: mn.angle_between(n3).to_degrees(),
                normal: mn.to_array(),
                offset_mm: off,
            });
        }
        let mut measured_angles = BTreeMap::new();
        for name in design[key].keys() {
            let pair = match name.as_str() {
                "back_heel" => Some(("back", "heel_r")),
                "heel_side" => Some(("heel_r", "side_r")),
                "back_side" => Some(("back", "side_r")),
                "side_front" => Some(("side_r", "front_r")),
                "apex" => Some(("front_r", "front_l")),
                "side_corner" => Some(("side_r", "corner_r")),
                "corner_front" => Some(("corner_r", "front")),
                "side_to_back_line" => None,
                _ => None,
            };
            if let Some((a, b)) = pair
                && let (Some(na), Some(nb)) = (measured_n.get(a), measured_n.get(b))
            {
                measured_angles.insert(name.clone(), 180.0 - na.angle_to(*nb).abs().to_degrees());
            }
        }
        if let (Some(s), Some(b)) = (measured_n.get("side_r"), measured_n.get("back")) {
            measured_angles.insert(
                "side_to_back_line".to_owned(),
                180.0 - s.angle_to(*b).abs().to_degrees(),
            );
        }
        if let Some(f) = measured_n.get("front_r") {
            measured_angles.insert(
                "front_to_back_line".to_owned(),
                f.angle_to(DVec2::Y).abs().to_degrees(),
            );
        }
        let zs = common.positions.iter().map(|q| f64::from(q.z) * sgn);
        let measured_height = zs.fold(f64::NEG_INFINITY, f64::max);
        let v = case_mesh.validate();
        let origin = DVec3::from(frame_report.origin);
        reports.insert(
            key.to_owned(),
            ArchReport {
                height_mm: h,
                anatomic_mm: a,
                art_mm: art,
                depth_mm: p.depth,
                land_mm: land,
                base_plane: Plane::new(origin + z_axis * (sgn * h), z_axis * sgn),
                outline: o.clone(),
                design_angles_deg: design[key].clone(),
                measured_angles_deg: measured_angles,
                walls: walls_m,
                base_tilt_deg: base_tilt,
                measured_height_mm: measured_height,
                faces: v.faces,
                watertight: v.is_watertight(),
                components: v.components,
                seconds: t0.elapsed().as_secs_f64(),
            },
        );
        out_meshes.insert(key, case_mesh);
    }

    let (mut total, mut measured_total, mut coplanar, mut back_off) = (None, None, None, None);
    if let (Some(u), Some(l)) = (reports.get("upper"), reports.get("lower")) {
        total = Some(u.height_mm + l.height_mm);
        measured_total = Some(u.measured_height_mm + l.measured_height_mm);
        let bu = u.walls.iter().find(|w| w.name == "back");
        let bl = l.walls.iter().find(|w| w.name == "back");
        if let (Some(bu), Some(bl)) = (bu, bl) {
            coplanar = Some(
                DVec3::from(bu.normal)
                    .angle_between(DVec3::from(bl.normal))
                    .to_degrees(),
            );
            back_off = Some((bu.offset_mm - bl.offset_mm).abs());
        }
    }
    let y = DVec3::from(frame_report.y);
    let origin = DVec3::from(frame_report.origin);
    let report = Report {
        style: params.style,
        occlusal_plane: Plane::new(origin, z_axis),
        back_plane: Plane::new(origin + y * back_y, -y),
        frame: frame_report,
        arches: reports,
        total_height_mm: total,
        measured_total_height_mm: measured_total,
        back_coplanarity_deg: coplanar,
        back_offset_mm: back_off,
        warnings,
    };
    Ok(Output {
        upper_base: out_meshes.remove("upper"),
        lower_base: out_meshes.remove("lower"),
        report,
    })
}
