//! Occlusion: the upper and the lower arch in occlusion (closed mouth), the
//! virtual bite (estimated occlusion of two unrelated scans) and the
//! occlusal contact map.
//!
//! Reference: `ortho_aligner/occlusion.py` (`align`, `_TopSurface`,
//! `_place`, `contact_offset`, `penetration`, `signed_distances`,
//! `contact_map`, `in_occlusion`).
//!
//! Every arch is handled in its own arch coordinates ([`ArchModel`]): origin
//! between the last molars on the occlusal plane, `+Y` towards the incisors,
//! `+Z` occlusal, `+X = Y × Z` (the patient's right for the lower arch, the
//! left for the upper arch). The upper arch faces the lower one after the
//! 180 degree turn about the antero-posterior axis [`S_FLIP`].
//!
//! Virtual bite ([`align`]), as the plugin:
//! 1. the upper arch frame faces the lower one;
//! 2. starting pose: midlines aligned, the upper incisal edges
//!    [`OVERJET`] mm in front of the lower ones and the upper first molars
//!    [`CLASS_I`] mm distal to the lower ones, averaged;
//! 3. maximum intercuspation: small turns about the midline and shifts around
//!    the starting pose; for each the upper arch is lowered onto a 0.5 mm
//!    height grid of the lower crowns until the first contact, and the pose
//!    with the most contacts spread on both sides wins;
//! 4. settling: small tilts let the upper arch rest on both sides;
//! 5. the first contact is then measured exactly on the crown triangles.
//!
//! The result is a virtual occlusion for display: the clinical relation is
//! not known from two separate scans (see [`crate::bite`] for the
//! registration from bite scans).
//!
//! Contact map ([`contact_map`]): the signed distance of every crown vertex
//! of each arch to the crowns of the antagonist (negative inside a crown).
//! The plugin measures the closed reconstructed teeth; here the crowns of
//! the scans are used (the reconstruction is a later stage), with their
//! normals turned outwards.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, ScanRole};
use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use dca_model::{AlignersState, MODULE_ID, Seed};
use glam::{DMat3, DMat4, DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::{evidence, frame};

/// Normal overjet of the starting pose (mm).
pub const OVERJET: f64 = 2.5;
/// Upper first molar distal to the lower first molar in the starting pose
/// (mm, Class I).
pub const CLASS_I: f64 = 2.5;
/// Gap counted as an occlusal contact by the intercuspation search (mm).
pub const CONTACT: f64 = 0.3;
/// Distance counted as a contact by the contact map (mm).
pub const CONTACT_MM: f64 = 0.10;
/// Reach of the contact map: farther vertices get no distance (mm).
pub const MAP_REACH: f64 = 0.5;

/// 180 degrees about the antero-posterior axis: upper arch coordinates to
/// lower arch coordinates of two facing arches.
pub const S_FLIP: DMat3 = DMat3::from_cols(
    DVec3::new(-1.0, 0.0, 0.0),
    DVec3::new(0.0, 1.0, 0.0),
    DVec3::new(0.0, 0.0, -1.0),
);

/// Colour scale of the signed distance to the antagonist (mm; negative =
/// interpenetration): 50 µm bins of interpenetration and distance (Dentsply
/// Sirona 2019), contact up to 0.10 mm (Lee 2018; Beck 2023), near contact
/// up to 0.50 mm. Each entry: upper bound (exclusive) and RGBA in sRGB (the
/// plugin writes them as sRGB vertex colours; convert to linear to shade).
pub const CONTACT_SCALE: [(f64, [f32; 4]); 7] = [
    (-0.10, [0.80, 0.00, 0.60, 1.0]),
    (-0.05, [0.90, 0.05, 0.05, 1.0]),
    (0.00, [1.00, 0.45, 0.00, 1.0]),
    (0.05, [1.00, 0.85, 0.00, 1.0]),
    (0.10, [0.15, 0.75, 0.20, 1.0]),
    (0.30, [0.25, 0.70, 0.95, 1.0]),
    (0.50, [0.15, 0.30, 0.85, 1.0]),
];
/// Tooth colour beyond the last bin of [`CONTACT_SCALE`].
pub const NO_CONTACT: [f32; 4] = [0.93, 0.91, 0.84, 1.0];

/// Colour of a signed distance on the contact map ([`NO_CONTACT`] beyond
/// 0.50 mm, for no distance or a non-finite one).
pub fn contact_color(d: f64) -> [f32; 4] {
    if d.is_nan() {
        return NO_CONTACT;
    }
    CONTACT_SCALE
        .iter()
        .find(|(hi, _)| d < *hi)
        .map_or(NO_CONTACT, |(_, c)| *c)
}

/// Errors of the occlusion stage.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OcclusionError {
    /// The input does not describe a usable arch.
    #[error("{0}")]
    Invalid(String),
    /// The arches never overlap: they cannot be put in occlusion.
    #[error("the arches do not overlap: they cannot be put in occlusion")]
    NoOverlap,
}

/// Parameters of the virtual bite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Search the maximum intercuspation and settle on both sides (`false`:
    /// the starting pose lowered to the first contact only).
    pub search: bool,
    /// Overjet of the starting pose (mm).
    pub overjet_mm: f64,
    /// Upper first molar distal to the lower one in the starting pose (mm).
    pub class_i_mm: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            search: true,
            overjet_mm: evidence::param_f64("occlusion_overjet").unwrap_or(OVERJET),
            class_i_mm: evidence::param_f64("occlusion_class_i").unwrap_or(CLASS_I),
        }
    }
}

// ----------------------------------------------------------------------------
// arch model
// ----------------------------------------------------------------------------

/// A labelled arch scan in its arch coordinates.
#[derive(Debug, Clone)]
pub struct ArchModel {
    /// Arch of the scan.
    pub arch: Arch,
    /// Arch coordinates to case coordinates.
    pub to_case: DMat4,
    /// Vertices in arch coordinates (mm).
    pub vertices: Vec<DVec3>,
    /// Triangles.
    pub tris: Vec<[u32; 3]>,
    /// FDI number of each vertex, `0` for the gingiva.
    pub labels: Vec<u16>,
    /// Vertices of every tooth, by FDI number (ascending).
    pub crown_idx: BTreeMap<u16, Vec<u32>>,
    /// Seeds of the arch in arch coordinates.
    pub seeds: Vec<(Fdi, DVec3)>,
}

impl ArchModel {
    /// Model of a labelled scan in case coordinates; the arch frame comes
    /// from the seeds of the arch ([`frame::run`]).
    ///
    /// # Errors
    /// [`OcclusionError::Invalid`] for labels that do not match the scan, a
    /// scan without teeth or no usable frame.
    pub fn new(
        arch: Arch,
        case_vertices: &[DVec3],
        tris: &[[u32; 3]],
        labels: &[u16],
        seeds: &[Seed],
    ) -> Result<Self, OcclusionError> {
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: case_vertices,
                seeds,
            },
            &frame::Params::default(),
        )
        .map_err(|e| OcclusionError::Invalid(e.to_string()))?;
        let to_case = framed.arch_frame.matrix();
        let arch_seeds: Vec<(Fdi, DVec3)> = seeds
            .iter()
            .filter(|s| Arch::of(s.fdi) == arch)
            .map(|s| (s.fdi, s.position))
            .collect();
        Self::from_case(arch, to_case, case_vertices, tris, labels, &arch_seeds)
    }

    /// Model of a labelled scan in case coordinates with a known arch frame
    /// (`to_case`: arch coordinates to case coordinates); seeds in case
    /// coordinates.
    ///
    /// # Errors
    /// See [`ArchModel::new`].
    pub fn from_case(
        arch: Arch,
        to_case: DMat4,
        case_vertices: &[DVec3],
        tris: &[[u32; 3]],
        labels: &[u16],
        seeds: &[(Fdi, DVec3)],
    ) -> Result<Self, OcclusionError> {
        let to_arch = frame::rigid_inverse(&to_case);
        let vertices = case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect();
        let seeds = seeds
            .iter()
            .map(|&(f, p)| (f, to_arch.transform_point3(p)))
            .collect();
        Self::from_arch(
            arch,
            to_case,
            vertices,
            tris.to_vec(),
            labels.to_vec(),
            seeds,
        )
    }

    /// Model from vertices and seeds already in arch coordinates.
    ///
    /// # Errors
    /// See [`ArchModel::new`].
    pub fn from_arch(
        arch: Arch,
        to_case: DMat4,
        vertices: Vec<DVec3>,
        tris: Vec<[u32; 3]>,
        labels: Vec<u16>,
        seeds: Vec<(Fdi, DVec3)>,
    ) -> Result<Self, OcclusionError> {
        if labels.len() != vertices.len() {
            return Err(OcclusionError::Invalid(format!(
                "{} labels for {} vertices",
                labels.len(),
                vertices.len()
            )));
        }
        let n = vertices.len() as u32;
        if tris.iter().flatten().any(|&i| i >= n) {
            return Err(OcclusionError::Invalid(
                "triangle index out of range".to_owned(),
            ));
        }
        let mut crown_idx: BTreeMap<u16, Vec<u32>> = BTreeMap::new();
        for (i, &l) in labels.iter().enumerate() {
            if l != 0 {
                crown_idx.entry(l).or_default().push(i as u32);
            }
        }
        if crown_idx.is_empty() {
            return Err(OcclusionError::Invalid(
                "the scan has no labelled tooth".to_owned(),
            ));
        }
        Ok(Self {
            arch,
            to_case,
            vertices,
            tris,
            labels,
            crown_idx,
            seeds,
        })
    }

    /// Indices of every crown vertex (tooth by tooth, ascending FDI).
    pub fn crown_indices(&self) -> Vec<u32> {
        self.crown_idx.values().flatten().copied().collect()
    }

    /// Crown vertices in arch coordinates, at most `n` evenly spread in
    /// their order (all of them for `None`), with their FDI numbers.
    pub fn crown_points(&self, n: Option<usize>) -> (Vec<DVec3>, Vec<u16>) {
        let idx = self.crown_indices();
        let idx = match n {
            Some(n) => subsample(&idx, n),
            None => idx,
        };
        let p = idx.iter().map(|&i| self.vertices[i as usize]).collect();
        let l = idx.iter().map(|&i| self.labels[i as usize]).collect();
        (p, l)
    }

    /// Landmarks in arch coordinates: the midline point between the central
    /// incisor seeds and the mean of the first molar seeds.
    pub fn landmarks(&self) -> (Option<DVec3>, Option<DVec3>) {
        let mean = |k: u8| {
            let v: Vec<DVec3> = self
                .seeds
                .iter()
                .filter(|(f, _)| anatomy::index(*f) == k)
                .map(|(_, p)| *p)
                .collect();
            (!v.is_empty()).then(|| v.iter().copied().sum::<DVec3>() / v.len() as f64)
        };
        (mean(1), mean(6))
    }

    /// Case coordinates of the arch vertices.
    pub fn case_vertices(&self) -> Vec<DVec3> {
        self.vertices
            .iter()
            .map(|&p| self.to_case.transform_point3(p))
            .collect()
    }

    /// The crown surface of the arch (arch coordinates).
    pub fn crown_surface(&self) -> Option<CrownSurface> {
        CrownSurface::new(&self.vertices, &self.tris, &self.labels)
    }
}

/// At most `n` rows of `x`, evenly spread in their order (`mu.subsample`,
/// `visit._sub_idx`: `linspace(0, len - 1, n)` truncated).
pub fn subsample<T: Copy>(x: &[T], n: usize) -> Vec<T> {
    sub_idx(x.len(), n).into_iter().map(|i| x[i]).collect()
}

/// Indices `0..n_all`, or `n` of them evenly spread (`visit._sub_idx`).
pub fn sub_idx(n_all: usize, n: usize) -> Vec<usize> {
    if n_all <= n {
        return (0..n_all).collect();
    }
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }
    let last = (n_all - 1) as f64;
    let step = last / (n - 1) as f64;
    (0..n)
        .map(|k| {
            // numpy linspace: start + k * step, the last one exactly the end
            let v = if k == n - 1 { last } else { k as f64 * step };
            v as usize
        })
        .collect()
}

// ----------------------------------------------------------------------------
// crown surface
// ----------------------------------------------------------------------------

/// The crown triangles of an arch (all three corners labelled) with outward
/// normals and a BVH, in the coordinates of the vertices given.
#[derive(Debug, Clone)]
pub struct CrownSurface {
    mesh: TriMesh,
    bvh: Bvh,
    normals: Vec<DVec3>,
    face_fdi: Vec<u16>,
}

impl CrownSurface {
    /// Crown surface of a labelled arch in arch coordinates (`+Z`
    /// occlusal: the normals are turned so that the top 15% of the faces
    /// face `+Z` on average, as `occlusion.penetration`). `None` without
    /// crown triangles.
    pub fn new(vertices: &[DVec3], tris: &[[u32; 3]], labels: &[u16]) -> Option<Self> {
        let crown: Vec<[u32; 3]> = tris
            .iter()
            .filter(|t| t.iter().all(|&i| labels[i as usize] != 0))
            .copied()
            .collect();
        if crown.is_empty() {
            return None;
        }
        let raw: Vec<(DVec3, f64, f64)> = crown
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| vertices[i as usize]);
                let cr = (b - a).cross(c - a);
                let len = cr.length();
                let n = if len > 0.0 { cr / len } else { cr };
                (n, 0.5 * len, (a.z + b.z + c.z) / 3.0)
            })
            .collect();
        let mut zs: Vec<f64> = raw.iter().map(|r| r.2).collect();
        let cut = percentile(&mut zs, 85.0);
        let s: f64 = raw.iter().filter(|r| r.2 >= cut).map(|r| r.0.z * r.1).sum();
        let sign = if s >= 0.0 { 1.0 } else { -1.0 };
        let tris: Vec<[u32; 3]> = if sign > 0.0 {
            crown
        } else {
            crown.iter().map(|t| [t[0], t[2], t[1]]).collect()
        };
        let normals = raw.iter().map(|r| r.0 * sign).collect();
        let face_fdi = tris.iter().map(|t| labels[t[0] as usize]).collect();
        let mesh = TriMesh::new(vertices.iter().map(|p| p.as_vec3()).collect(), tris);
        let bvh = Bvh::build(&mesh).ok()?;
        Some(Self {
            mesh,
            bvh,
            normals,
            face_fdi,
        })
    }

    /// Number of crown triangles.
    pub fn len(&self) -> usize {
        self.normals.len()
    }

    /// `true` without triangles (never for a built surface).
    pub fn is_empty(&self) -> bool {
        self.normals.is_empty()
    }

    /// Outward unit normal of a face.
    pub fn normal(&self, face: u32) -> DVec3 {
        self.normals[face as usize]
    }

    /// FDI number of a face (label of its first corner).
    pub fn fdi(&self, face: u32) -> u16 {
        self.face_fdi[face as usize]
    }

    /// The crown triangles (outward winding).
    pub fn mesh(&self) -> &TriMesh {
        &self.mesh
    }

    /// The hierarchy over the crown triangles.
    pub fn bvh(&self) -> &Bvh {
        &self.bvh
    }

    /// Distance to the nearest crown point within `reach` (mm).
    pub fn nearest_distance(&self, p: DVec3, reach: f64) -> Option<f64> {
        let reach = if reach.is_finite() {
            reach as f32
        } else {
            f32::INFINITY
        };
        self.bvh
            .closest_point(p.as_vec3(), reach)
            .map(|h| p.distance(h.point.as_dvec3()))
    }

    /// First face hit by the ray `p + t dir`, `t` in `[0, max_t]`.
    pub fn first_hit(&self, p: DVec3, dir: DVec3, max_t: f64) -> Option<(u32, f64)> {
        self.bvh
            .ray_cast(p.as_vec3(), dir.as_vec3(), max_t as f32)
            .map(|h| (h.face, f64::from(h.t)))
    }

    /// Height of the crowns on the vertical of `p` (the highest crown point
    /// below `p + 30 mm`) and the FDI number of the crown there.
    pub fn height_below(&self, p: DVec3) -> Option<(f64, u16)> {
        let top = p + DVec3::new(0.0, 0.0, 30.0);
        self.first_hit(top, DVec3::NEG_Z, 60.0)
            .map(|(f, t)| (top.z - t, self.fdi(f)))
    }

    /// Signed distance of `p` to the crowns within `reach`: negative inside a
    /// crown (the ray along `+Z`, towards the antagonist, leaves a crown
    /// through an outward face), `+inf` beyond `reach`
    /// (`occlusion.signed_distances`).
    pub fn signed_distance(&self, p: DVec3, reach: f64) -> f64 {
        let Some(d) = self.nearest_distance(p, reach) else {
            return f64::INFINITY;
        };
        let inside = self
            .first_hit(p, DVec3::Z, 12.0)
            .is_some_and(|(f, _)| self.normal(f).z > 0.0);
        if inside { -d } else { d }
    }
}

/// The `q`-th percentile of `v` with linear interpolation (numpy's
/// default); `v` is sorted in place. NaN for an empty slice.
pub fn percentile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let pos = (q / 100.0) * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(v.len() - 1);
    let frac = pos - lo as f64;
    v[lo] + (v[hi] - v[lo]) * frac
}

// ----------------------------------------------------------------------------
// height grid and first contact
// ----------------------------------------------------------------------------

/// Height of the lower occlusal surface on a grid (the highest `z` per
/// cell), `occlusion._TopSurface`.
#[derive(Debug, Clone)]
pub struct TopSurface {
    cell: f64,
    x0: f64,
    y0: f64,
    nx: i64,
    ny: i64,
    h: Vec<f64>,
}

impl TopSurface {
    /// Grid of the points `p` (arch coordinates) with `cell` mm cells.
    pub fn new(p: &[DVec3], cell: f64) -> Self {
        let x0 = p.iter().map(|q| q.x).fold(f64::INFINITY, f64::min) - 1.0;
        let y0 = p.iter().map(|q| q.y).fold(f64::INFINITY, f64::min) - 1.0;
        let ix: Vec<i64> = p.iter().map(|q| ((q.x - x0) / cell) as i64).collect();
        let iy: Vec<i64> = p.iter().map(|q| ((q.y - y0) / cell) as i64).collect();
        let nx = ix.iter().copied().max().unwrap_or(0) + 2;
        let ny = iy.iter().copied().max().unwrap_or(0) + 2;
        let mut h = vec![f64::NEG_INFINITY; (nx * ny) as usize];
        for k in 0..p.len() {
            let c = &mut h[(ix[k] * ny + iy[k]) as usize];
            *c = c.max(p[k].z);
        }
        Self {
            cell,
            x0,
            y0,
            nx,
            ny,
            h,
        }
    }

    /// Grid height on the vertical of `q` (`-inf` outside the grid). The
    /// cell index is truncated towards zero, as numpy's `astype(int)`.
    pub fn at(&self, q: DVec3) -> f64 {
        let ix = ((q.x - self.x0) / self.cell) as i64;
        let iy = ((q.y - self.y0) / self.cell) as i64;
        if ix >= 0 && ix < self.nx && iy >= 0 && iy < self.ny {
            self.h[(ix * self.ny + iy) as usize]
        } else {
            f64::NEG_INFINITY
        }
    }
}

/// A pose of the upper arch lowered to the first contact.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Placement {
    /// Vertical move that brings the upper points to the first contact (mm).
    pub tz: f64,
    /// Upper points within [`CONTACT`] mm of the lower surface.
    pub contacts: usize,
    /// Of them, on the right side (`x` below the midline).
    pub right: usize,
    /// Of them, on the left side.
    pub left: usize,
}

/// Upper crown points `pu` posed by `g` (lower arch coordinates) and
/// lowered onto `top` until the first contact (`occlusion._place`). `None`
/// with fewer than 50 points over the grid.
pub fn place(pu: &[DVec3], top: &TopSurface, g: &DMat4, midline_x: f64) -> Option<Placement> {
    let pts: Vec<(DVec3, f64)> = pu
        .iter()
        .map(|&p| {
            let q = g.transform_point3(p);
            (q, top.at(q))
        })
        .filter(|(_, h)| h.is_finite())
        .collect();
    if pts.len() < 50 {
        return None;
    }
    let tz = pts
        .iter()
        .map(|(q, h)| h - q.z)
        .fold(f64::NEG_INFINITY, f64::max)
        + 0.02;
    let mut contacts = 0;
    let mut right = 0;
    for (q, h) in &pts {
        if q.z + tz - h <= CONTACT {
            contacts += 1;
            if q.x < midline_x {
                right += 1;
            }
        }
    }
    Some(Placement {
        tz,
        contacts,
        right,
        left: contacts - right,
    })
}

/// Most contacts, preferably on both sides, close to the anatomical pose
/// (`occlusion._score`).
pub fn score(r: &Placement, penalty: f64) -> f64 {
    r.contacts as f64 + 2.0 * r.right.min(r.left) as f64 - penalty
}

/// The first contact of the upper arch on the lower one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FirstContact {
    /// Vertical move of the upper arch (along the lower occlusal axis, mm)
    /// that brings it to the first contact; positive = the bite opens.
    pub dz: f64,
    /// Upper tooth that touches first (`0` when unknown).
    pub upper_fdi: u16,
    /// Lower tooth that touches first (`0` when unknown).
    pub lower_fdi: u16,
}

/// Vertical move that brings the upper crown points `q` (lower arch
/// coordinates, with their FDI numbers) to the first contact with the lower
/// crowns (`occlusion.contact_offset` at stage 0). A height grid gives a safe
/// estimate; the points that can touch first are then measured exactly on
/// the crown triangles.
pub fn contact_offset(
    lower_points: &[DVec3],
    lower: &CrownSurface,
    q: &[DVec3],
    q_fdi: &[u16],
) -> FirstContact {
    const N_EXACT: usize = 400;
    const BAND: f64 = 0.8;
    let none = FirstContact {
        dz: 0.0,
        upper_fdi: 0,
        lower_fdi: 0,
    };
    let top = TopSurface::new(lower_points, 0.5);
    let ok: Vec<(usize, f64)> = q
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let h = top.at(*p);
            h.is_finite().then_some((i, p.z - h))
        })
        .collect();
    if ok.len() < 50 {
        return none;
    }
    let mut order = ok.clone();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));
    let gmin = order[0].1;
    let dz_grid = -gmin + 0.02;
    let mut best = FirstContact {
        dz: dz_grid,
        upper_fdi: q_fdi[order[0].0],
        lower_fdi: 0,
    };
    let mut exact: Option<(f64, usize, u16)> = None;
    for &(i, g) in order.iter().take(N_EXACT) {
        if g > gmin + BAND {
            continue;
        }
        if let Some((h, f)) = lower.height_below(q[i]) {
            let gap = q[i].z - h;
            if exact.is_none_or(|e| gap < e.0) {
                exact = Some((gap, i, f));
            }
        }
    }
    if let Some((gap, i, f)) = exact {
        best.dz = (-gap + 0.02).max(dz_grid - BAND);
        best.upper_fdi = q_fdi[i];
        best.lower_fdi = f;
    }
    best
}

/// Independent check of a relation on the lower crown triangles: how deep
/// upper crown points `q` (lower arch coordinates) are inside a lower crown
/// and how many touch it (`occlusion.penetration`). Returns (largest depth
/// in mm, points inside or within [`CONTACT`] mm).
pub fn penetration(lower: &CrownSurface, q: &[DVec3]) -> (f64, usize) {
    let mut depth = 0.0_f64;
    let mut contacts = 0;
    for &p in q {
        let inside = lower
            .first_hit(p, DVec3::Z, f64::INFINITY)
            .is_some_and(|(f, _)| lower.normal(f).z > 0.0);
        let dist = lower
            .nearest_distance(p, f64::INFINITY)
            .unwrap_or(f64::INFINITY);
        if inside {
            depth = depth.max(dist);
        }
        if inside || dist <= CONTACT {
            contacts += 1;
        }
    }
    (depth, contacts)
}

/// `true` when two arch frames are already related (`occlusion.in_occlusion`
/// without the stored bite source): facing occlusal planes, the upper arch
/// between 2 mm into and 10 mm above the lower one, within 15 mm sideways.
pub fn in_occlusion(lower_to_case: &DMat4, upper_to_case: &DMat4) -> bool {
    let zl = lower_to_case.z_axis.truncate();
    let zu = upper_to_case.z_axis.truncate();
    if zl.dot(zu) > -0.7 {
        return false;
    }
    let rel = frame::rigid_inverse(lower_to_case) * *upper_to_case;
    let t = rel.w_axis.truncate();
    let horiz = (t.x * t.x + t.y * t.y).sqrt();
    (-2.0..=10.0).contains(&t.z) && horiz <= 15.0
}

/// The arch frame of `arch` in `state` (arch coordinates to case
/// coordinates) as the virtual bite takes it ([`ArchModel::new`]): from the
/// seeds of the arch, else from its scan. `None` without a scan or a frame.
fn arch_to_case(state: &CaseState, data: &AlignersState, arch: Arch) -> Option<DMat4> {
    let mesh = state.mesh(&state.case.scan(arch.role())?.mesh)?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let input = frame::Input {
        arch,
        vertices: &vertices,
        seeds: &data.seeds,
    };
    let framed = frame::run(&input, &frame::Params::default()).ok()?;
    Some(framed.arch_frame.matrix())
}

/// `true` when the relation of the two arches is known
/// (`case_plan.arches_related`): a bite registration or the virtual bite was
/// applied and no arch was replaced since (`bite_registered`, the plugin's
/// stored bite source `bite.info(lower)["fonte"]`), or the two arch frames
/// are already in occlusion ([`in_occlusion`]: scans exported in
/// occlusion). Arches without a frame are not related.
pub fn arches_related(state: &CaseState, data: &AlignersState) -> bool {
    data.bite_registered
        || match (
            arch_to_case(state, data, Arch::Lower),
            arch_to_case(state, data, Arch::Upper),
        ) {
            (Some(lower), Some(upper)) => in_occlusion(&lower, &upper),
            _ => false,
        }
}

/// `true` when both arches have their teeth and are not related: the
/// plugin's `case_plan.opposing_pair` (both arches with their 3D teeth; here
/// a scan with tooth colours each, the teeth the setup measures) and not
/// [`arches_related`]. With both arches the bite comes first: the setup
/// refuses such a case (`case_plan.run_setup`), and the whole automatic run
/// puts the arches in occlusion before the setup (`case_auto.auto_steps`).
pub fn bite_needed(state: &CaseState) -> bool {
    let Some(data) = state.module_data::<AlignersState>(MODULE_ID) else {
        return false;
    };
    let teeth = |role: ScanRole| state.case.scan(role).is_some() && data.labels.contains_key(&role);
    teeth(ScanRole::Lower) && teeth(ScanRole::Upper) && !arches_related(state, data)
}

// ----------------------------------------------------------------------------
// virtual bite
// ----------------------------------------------------------------------------

fn shift(v: DVec3) -> DMat4 {
    DMat4::from_translation(v)
}

fn rz(a: f64) -> DMat3 {
    DMat3::from_rotation_z(a)
}

/// Rotation about the transverse (`x`) then the antero-posterior (`y`)
/// axis (`occlusion._tilt`: `ry @ rx`).
fn tilt(pitch: f64, roll: f64) -> DMat3 {
    DMat3::from_rotation_y(roll) * DMat3::from_rotation_x(pitch)
}

/// Rotation `r` about the point `c`.
fn about(r: DMat3, c: DVec3) -> DMat4 {
    let mut g = DMat4::from_mat3(r);
    g.w_axis = (c - r * c).extend(1.0);
    g
}

/// The virtual occlusion of two arches.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VirtualBite {
    /// Upper arch coordinates to lower arch coordinates in occlusion.
    #[serde(skip)]
    pub relation: DMat4,
    /// Rigid move of the upper scan in case coordinates (apply it to the
    /// upper scan, its labels, seeds and tooth frames).
    #[serde(skip)]
    pub delta: DMat4,
    /// Upper crown points in contact (within [`CONTACT`] mm on the grid).
    pub contacts: usize,
    /// Of them, on the right side.
    pub contacts_right: usize,
    /// Of them, on the left side.
    pub contacts_left: usize,
    /// Turn about the midline of the chosen pose (degrees).
    pub rotation_deg: f64,
    /// Settling tilts (pitch about the transverse axis, roll about the
    /// antero-posterior axis, degrees).
    pub tilt_deg: [f64; 2],
    /// Shift of the chosen pose from the starting pose (mm, x and y).
    pub shift_mm: [f64; 2],
    /// Exact correction of the first contact on the crown triangles (mm).
    pub exact_correction_mm: f64,
    /// Teeth that touch first (upper, lower).
    pub first_contact: [u16; 2],
}

/// Puts the upper arch in occlusion on the lower one (`occlusion.align`).
///
/// # Errors
/// [`OcclusionError::NoOverlap`] when no pose overlaps the lower arch,
/// [`OcclusionError::Invalid`] when the lower arch has no crown triangle.
pub fn align(
    lower: &ArchModel,
    upper: &ArchModel,
    params: &Params,
) -> Result<VirtualBite, OcclusionError> {
    let (pl, _) = lower.crown_points(None);
    let (pu, _) = upper.crown_points(Some(25000));
    let (mid_l, m1_l) = lower.landmarks();
    let (mid_u, m1_u) = upper.landmarks();
    // starting pose in lower arch coordinates (after the 180 degree flip)
    let mut base = DVec3::ZERO;
    if let (Some(ml), Some(mu)) = (mid_l, mid_u) {
        let mu_f = S_FLIP * mu;
        base.x = ml.x - mu_f.x;
        let dy_inc = (ml.y + params.overjet_mm) - mu_f.y;
        let mut dy = dy_inc;
        if let (Some(l1), Some(u1)) = (m1_l, m1_u) {
            let m1_f = S_FLIP * u1;
            let dy_mol = (l1.y - params.class_i_mm) - m1_f.y;
            dy = 0.5 * (dy_inc + dy_mol);
        }
        base.y = dy;
    }
    let g0 = shift(base) * DMat4::from_mat3(S_FLIP);
    // turns about the vertical through the upper midline: the midline stays
    let pivot = match mid_u {
        Some(m) => g0.transform_point3(m),
        None => pu.iter().map(|&p| S_FLIP * p + base).sum::<DVec3>() / pu.len().max(1) as f64,
    };
    let mx = mid_l.map_or(0.0, |m| m.x);
    let top = TopSurface::new(&pl, 0.5);
    let angles: Vec<f64> = if params.search {
        [-3.0_f64, -1.5, 0.0, 1.5, 3.0]
            .iter()
            .map(|a| a.to_radians())
            .collect()
    } else {
        vec![0.0]
    };
    let shifts: Vec<f64> = if params.search {
        vec![-1.0, -0.5, 0.0, 0.5, 1.0]
    } else {
        vec![0.0]
    };
    struct Best {
        score: f64,
        g: DMat4,
        r: Placement,
        dx: f64,
        dy: f64,
        ang: f64,
        pitch: f64,
        roll: f64,
    }
    let mut best: Option<Best> = None;
    for &ang in &angles {
        for &dx in &shifts {
            for &dy in &shifts {
                let g = shift(DVec3::new(dx, dy, 0.0)) * about(rz(ang), pivot) * g0;
                let Some(r) = place(&pu, &top, &g, mx) else {
                    continue;
                };
                let sc = score(&r, 40.0 * (dx.abs() + dy.abs()) + 400.0 * ang.abs());
                if best.as_ref().is_none_or(|b| sc > b.score) {
                    best = Some(Best {
                        score: sc,
                        g,
                        r,
                        dx,
                        dy,
                        ang,
                        pitch: 0.0,
                        roll: 0.0,
                    });
                }
            }
        }
    }
    let mut best = best.ok_or(OcclusionError::NoOverlap)?;
    // settling on both sides: small tilts about the centre of the upper crowns
    if params.search {
        let g1 = best.g;
        let centre = pu.iter().map(|&p| g1.transform_point3(p)).sum::<DVec3>() / pu.len() as f64;
        best.score = score(&best.r, 0.0);
        let tilts: Vec<f64> = [-2.0_f64, -1.0, 0.0, 1.0, 2.0]
            .iter()
            .map(|a| a.to_radians())
            .collect();
        for &pitch in &tilts {
            for &roll in &tilts {
                if pitch == 0.0 && roll == 0.0 {
                    continue;
                }
                let g = about(tilt(pitch, roll), centre) * g1;
                let Some(r) = place(&pu, &top, &g, mx) else {
                    continue;
                };
                let sc = score(&r, 400.0 * (pitch.abs() + roll.abs()));
                if sc > best.score {
                    best = Best {
                        score: sc,
                        g,
                        r,
                        pitch,
                        roll,
                        ..best
                    };
                }
            }
        }
    }
    // final height with every upper crown vertex (the search used a subset)
    let (pu_all, lab_u) = upper.crown_points(None);
    let r_all = place(&pu_all, &top, &best.g, mx).ok_or(OcclusionError::NoOverlap)?;
    let mut g = shift(DVec3::new(0.0, 0.0, r_all.tz)) * best.g;
    // exact first contact on the crown triangles (the search used the grid)
    let surface = lower
        .crown_surface()
        .ok_or_else(|| OcclusionError::Invalid("the lower arch has no crown".to_owned()))?;
    let q: Vec<DVec3> = pu_all.iter().map(|&p| g.transform_point3(p)).collect();
    let first = contact_offset(&pl, &surface, &q, &lab_u);
    let corr = if first.dz.abs() > 1e-3 { first.dz } else { 0.0 };
    g = shift(DVec3::new(0.0, 0.0, corr)) * g;
    let delta = lower.to_case * g * frame::rigid_inverse(&upper.to_case);
    let round1 = |x: f64| (x * 10.0).round() / 10.0;
    Ok(VirtualBite {
        relation: g,
        delta,
        contacts: r_all.contacts,
        contacts_right: r_all.right,
        contacts_left: r_all.left,
        rotation_deg: round1(best.ang.to_degrees()),
        tilt_deg: [
            round1(best.pitch.to_degrees()),
            round1(best.roll.to_degrees()),
        ],
        shift_mm: [best.dx, best.dy],
        exact_correction_mm: corr,
        first_contact: [first.upper_fdi, first.lower_fdi],
    })
}

/// Relation (upper arch coordinates to lower arch coordinates) of two arch
/// models as they are in the case.
pub fn relation(lower: &ArchModel, upper: &ArchModel) -> DMat4 {
    frame::rigid_inverse(&lower.to_case) * upper.to_case
}

/// First contact of the arches as they are in the case: the vertical move
/// of the upper arch that closes the bite exactly (positive = it opens).
pub fn first_contact(lower: &ArchModel, upper: &ArchModel) -> Option<FirstContact> {
    let surface = lower.crown_surface()?;
    let (pl, _) = lower.crown_points(None);
    let rel = relation(lower, upper);
    let (pu, lab) = upper.crown_points(None);
    let q: Vec<DVec3> = pu.iter().map(|&p| rel.transform_point3(p)).collect();
    Some(contact_offset(&pl, &surface, &q, &lab))
}

// ----------------------------------------------------------------------------
// contact map
// ----------------------------------------------------------------------------

/// Contacts of one arch on the contact map.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchContacts {
    /// Signed distance of every vertex of the scan to the antagonist crowns
    /// (mm; negative = interpenetration; `+inf` for the gingiva and beyond
    /// [`MAP_REACH`]).
    #[serde(skip)]
    pub distances: Vec<f64>,
    /// Teeth in contact (smallest distance at most [`CONTACT_MM`]) with their
    /// smallest distance (mm, rounded to µm).
    pub teeth_in_contact: BTreeMap<u16, f64>,
    /// Deepest interpenetration (mm, `0` without).
    pub penetration_mm: f64,
    /// Smallest signed distance (mm), `None` when nothing is within reach.
    pub min_distance_mm: Option<f64>,
    /// Teeth in contact on the right side.
    pub teeth_right: usize,
    /// Teeth in contact on the left side.
    pub teeth_left: usize,
}

/// Occlusal contact map of the relation in the case (`occlusion.contact_map`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContactMap {
    /// The lower arch measured against the upper crowns.
    pub lower: ArchContacts,
    /// The upper arch measured against the lower crowns.
    pub upper: ArchContacts,
    /// Deepest interpenetration of either arch (mm).
    pub penetration_mm: f64,
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Signed distances of the crown vertices of `a` to the crowns of `b`.
fn arch_contacts(a: &ArchModel, b: &ArchModel, b_surface: &CrownSurface) -> ArchContacts {
    use rayon::prelude::*;
    // a's arch coordinates to b's arch coordinates
    let to_b = frame::rigid_inverse(&b.to_case) * a.to_case;
    let distances: Vec<f64> = a
        .vertices
        .par_iter()
        .zip(a.labels.par_iter())
        .map(|(&p, &l)| {
            if l == 0 {
                f64::INFINITY
            } else {
                b_surface.signed_distance(to_b.transform_point3(p), MAP_REACH)
            }
        })
        .collect();
    let mut teeth = BTreeMap::new();
    let mut deepest = f64::INFINITY;
    let (mut right, mut left) = (0, 0);
    for (&fdi, idx) in &a.crown_idx {
        let dmin = idx
            .iter()
            .map(|&i| distances[i as usize])
            .fold(f64::INFINITY, f64::min);
        deepest = deepest.min(dmin);
        if dmin <= CONTACT_MM {
            teeth.insert(fdi, round3(dmin));
            let is_right = Fdi::new(fdi as u8).is_ok_and(anatomy::is_right);
            if is_right {
                right += 1;
            } else {
                left += 1;
            }
        }
    }
    ArchContacts {
        distances,
        teeth_in_contact: teeth,
        penetration_mm: round3((-deepest).max(0.0)),
        min_distance_mm: deepest.is_finite().then(|| round3(deepest)),
        teeth_right: right,
        teeth_left: left,
    }
}

/// Contact map of two arches as they are in the case.
///
/// # Errors
/// [`OcclusionError::Invalid`] when an arch has no crown triangle.
pub fn contact_map(lower: &ArchModel, upper: &ArchModel) -> Result<ContactMap, OcclusionError> {
    let no_crown = || OcclusionError::Invalid("an arch has no crown".to_owned());
    let sl = lower.crown_surface().ok_or_else(no_crown)?;
    let su = upper.crown_surface().ok_or_else(no_crown)?;
    let lo = arch_contacts(lower, upper, &su);
    let up = arch_contacts(upper, lower, &sl);
    let penetration_mm = lo.penetration_mm.max(up.penetration_mm);
    Ok(ContactMap {
        lower: lo,
        upper: up,
        penetration_mm,
    })
}

/// Transforms arch-model vertices (or any case points) by a case-coordinate
/// rigid move, in single precision for a mesh.
pub fn transform_positions(positions: &[Vec3], delta: &DMat4) -> Vec<Vec3> {
    positions
        .iter()
        .map(|p| delta.transform_point3(p.as_dvec3()).as_vec3())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    pub(crate) fn demo_model(arch: Arch, seed: u64) -> ArchModel {
        let d = demo::run(&demo::Params {
            arch,
            seed,
            ..demo::Params::default()
        });
        let v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let labels = d.mesh.vertex_labels.clone().unwrap();
        ArchModel::new(arch, &v, &d.mesh.tris, &labels, &d.seeds).unwrap()
    }

    #[test]
    fn subsampling_matches_numpy_linspace() {
        assert_eq!(sub_idx(5, 10), vec![0, 1, 2, 3, 4]);
        assert_eq!(sub_idx(10, 4), vec![0, 3, 6, 9]);
        assert_eq!(sub_idx(7, 3), vec![0, 3, 6]);
        assert_eq!(subsample(&[1, 2, 3, 4, 5, 6, 7, 8], 3), vec![1, 4, 8]);
    }

    #[test]
    fn percentile_is_linear() {
        let mut v = vec![4.0, 1.0, 3.0, 2.0];
        assert!((percentile(&mut v, 50.0) - 2.5).abs() < 1e-12);
        assert!((percentile(&mut v, 85.0) - 3.55).abs() < 1e-12);
        assert!(percentile(&mut [], 50.0).is_nan());
    }

    #[test]
    fn contact_colours_follow_the_scale() {
        assert_eq!(contact_color(-1.0), CONTACT_SCALE[0].1);
        assert_eq!(contact_color(-0.07), CONTACT_SCALE[1].1);
        assert_eq!(contact_color(0.0), CONTACT_SCALE[3].1);
        assert_eq!(contact_color(0.07), CONTACT_SCALE[4].1);
        assert_eq!(contact_color(0.4), CONTACT_SCALE[6].1);
        assert_eq!(contact_color(0.5), NO_CONTACT);
        assert_eq!(contact_color(f64::INFINITY), NO_CONTACT);
        assert_eq!(contact_color(f64::NAN), NO_CONTACT);
    }

    #[test]
    fn the_flip_turns_the_upper_arch_onto_the_lower() {
        let r = S_FLIP;
        assert_eq!(r * DVec3::Y, DVec3::Y);
        assert_eq!(r * DVec3::Z, DVec3::NEG_Z);
        assert!((r.determinant() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_height_grid_keeps_the_top_of_each_cell() {
        let p = [
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(0.1, 0.1, 3.0),
            DVec3::new(2.0, 2.0, -1.0),
        ];
        let top = TopSurface::new(&p, 0.5);
        assert_eq!(top.at(DVec3::new(0.05, 0.05, 9.0)), 3.0);
        assert_eq!(top.at(DVec3::new(2.1, 2.1, 0.0)), -1.0);
        assert_eq!(top.at(DVec3::new(1.0, 1.0, 0.0)), f64::NEG_INFINITY);
        assert_eq!(top.at(DVec3::new(50.0, 0.0, 0.0)), f64::NEG_INFINITY);
    }

    #[test]
    fn in_occlusion_needs_facing_close_frames() {
        let lower = DMat4::IDENTITY;
        let upper = shift(DVec3::new(0.0, 1.0, 2.0)) * DMat4::from_mat3(S_FLIP);
        assert!(in_occlusion(&lower, &upper));
        assert!(!in_occlusion(&lower, &shift(DVec3::new(0.0, 0.0, 2.0))));
        let far = shift(DVec3::new(0.0, 0.0, 30.0)) * DMat4::from_mat3(S_FLIP);
        assert!(!in_occlusion(&lower, &far));
    }

    #[test]
    fn the_virtual_bite_closes_the_demo_arches_without_interpenetration() {
        let lower = demo_model(Arch::Lower, 7);
        let upper = demo_model(Arch::Upper, 11);
        // the demo arches are both in plaster-cast orientation: not related
        assert!(!in_occlusion(&lower.to_case, &upper.to_case));
        let vb = align(&lower, &upper, &Params::default()).unwrap();
        assert!(vb.contacts > 50, "{vb:?}");
        assert!(vb.contacts_right > 0 && vb.contacts_left > 0, "{vb:?}");
        // the moved upper frame faces the lower one, in occlusion
        let moved = vb.delta * upper.to_case;
        assert!(in_occlusion(&lower.to_case, &moved));
        let upper2 = ArchModel {
            to_case: moved,
            ..upper.clone()
        };
        assert!((relation(&lower, &upper2) - vb.relation).abs_diff_eq(DMat4::ZERO, 1e-9));
        // the first contact is closed exactly (within 0.05 mm)
        let fc = first_contact(&lower, &upper2).unwrap();
        assert!(fc.dz.abs() < 0.05, "{fc:?}");
        // and nothing interpenetrates beyond 0.05 mm
        let (pu, _) = upper2.crown_points(Some(15000));
        let rel = relation(&lower, &upper2);
        let q: Vec<DVec3> = pu.iter().map(|&p| rel.transform_point3(p)).collect();
        let (depth, contacts) = penetration(&lower.crown_surface().unwrap(), &q);
        assert!(depth < 0.05, "{depth}");
        assert!(contacts > 0);
        // the contact map sees the contacts on both arches
        let map = contact_map(&lower, &upper2).unwrap();
        assert!(!map.lower.teeth_in_contact.is_empty());
        assert!(!map.upper.teeth_in_contact.is_empty());
        assert!(map.penetration_mm < 0.05, "{map:?}");
        assert_eq!(map.lower.distances.len(), lower.vertices.len());
        // gum vertices get no distance
        let gum = lower.labels.iter().position(|&l| l == 0).unwrap();
        assert!(map.lower.distances[gum].is_infinite());
    }

    #[test]
    fn without_search_the_starting_pose_is_kept() {
        let lower = demo_model(Arch::Lower, 7);
        let upper = demo_model(Arch::Upper, 7);
        let vb = align(
            &lower,
            &upper,
            &Params {
                search: false,
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(vb.rotation_deg, 0.0);
        assert_eq!(vb.shift_mm, [0.0, 0.0]);
        assert_eq!(vb.tilt_deg, [0.0, 0.0]);
        // midlines aligned, upper incisors in front of the lower ones
        let (mid_l, _) = lower.landmarks();
        let (mid_u, _) = upper.landmarks();
        let mu = vb.relation.transform_point3(mid_u.unwrap());
        let ml = mid_l.unwrap();
        assert!((mu.x - ml.x).abs() < 1e-6);
        assert!(mu.y > ml.y, "{mu:?} {ml:?}");
    }

    #[test]
    fn arch_models_reject_bad_input() {
        let v = vec![DVec3::ZERO; 3];
        let err = ArchModel::from_arch(
            Arch::Lower,
            DMat4::IDENTITY,
            v.clone(),
            vec![[0, 1, 2]],
            vec![0; 2],
            vec![],
        );
        assert!(matches!(err, Err(OcclusionError::Invalid(_))));
        let err = ArchModel::from_arch(
            Arch::Lower,
            DMat4::IDENTITY,
            v.clone(),
            vec![[0, 1, 2]],
            vec![0; 3],
            vec![],
        );
        assert!(matches!(err, Err(OcclusionError::Invalid(m)) if m.contains("no labelled")));
        let err = ArchModel::from_arch(
            Arch::Lower,
            DMat4::IDENTITY,
            v,
            vec![[0, 1, 5]],
            vec![31; 3],
            vec![],
        );
        assert!(matches!(err, Err(OcclusionError::Invalid(m)) if m.contains("range")));
    }
}
