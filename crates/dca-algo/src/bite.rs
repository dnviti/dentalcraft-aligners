//! Bite registration: the intermaxillary relation from buccal (vestibular)
//! bite scans.
//!
//! Reference: `ortho_aligner/bite.py` (registration). The bite with the
//! aligners worn (`aligner_bite.py`, `case_occlusal.py`) and the bite ramps
//! are later stages (WP33).
//!
//! Intraoral scanners record the relation of the arches with buccal scans
//! taken with the patient biting in maximum intercuspation, one per side.
//! Each arch is registered to the bite scans by best fit of the overlapping
//! buccal surfaces:
//! 1. arch models in arch coordinates ([`crate::occlusion::ArchModel`]):
//!    crowns plus the gingiva within 2 mm of them for the coarse search,
//!    crowns only for the fine alignment; closest points are exact points of
//!    the scan surface (BVH);
//! 2. coarse alignment: the frame of the bite scan (buccal direction = mean
//!    outward normal, mesio-distal direction = longest extent) is placed on
//!    the buccal surface of the arch every 2.5 mm from the incisors to the
//!    last molar on both sides, in the 4 orientations (vertical flip, normals
//!    wound outwards or inwards); every start is refined by a few ICP
//!    iterations and the one with the largest overlap wins. A scan round the
//!    whole arch (models in occlusion on a desktop scanner) gets its own
//!    starts on the incisors. Manual fallback: 3 point pairs (Kabsch 1976);
//! 3. fine alignment: trimmed point-to-plane ICP with normal compatibility
//!    and automatic overlap (Chetverikov 2005), [`dc_geom::icp`];
//! 4. the lower arch stays in place: the bite scans are moved onto it, their
//!    points on the lower arch are removed and the upper arch is registered
//!    to the rest (both bite scans together when there are two);
//! 5. checks: overlap, the teeth in common, facing arches, interpenetration;
//!    reasons to review: RMS, coherence, few teeth, what the scans cover,
//!    agreement of the two bite scans, arches not in contact. Nothing is
//!    moved when the registration is rejected.
//!
//! Where this departs from the plugin, for the bite scans of real life
//! (partial, holed, noisy, distorted, with soft tissue): when the plugin's
//! search gives no acceptable fit, a wide search turns and shifts its
//! starts ([`fit`]); a fit is refused only when its teeth do not match
//! the crowns, whatever the noise and distortion of the scan
//! ([`check_fit`]); what the scans cover comes from the teeth they fit,
//! not from their extent; of two bite scans, one that does not fit is
//! left out ([`register`]). On the plugin's own cases the result is the
//! plugin's (`tests/bite_oracle.rs`); `tests/bite_robustness.rs` holds the
//! spoiled scans.
//!
//! Transforms follow the plugin: a fit `T` maps the bite scan (case
//! coordinates) into the arch coordinates of an arch.

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_geom::icp::{self, IcpMetric, IcpParams, IcpTarget};
use dc_mesh::TriMesh;
use glam::{DAffine3, DMat3, DMat4, DVec3};
use nalgebra::{Matrix3, SymmetricEigen};
use serde::Serialize;

use crate::anatomy;
use crate::evidence;
use crate::frame;
use crate::occlusion::{self, ArchModel, S_FLIP, sub_idx};

/// Bite point on the arch surface (mm).
pub fn inlier() -> f64 {
    evidence::param_f64("bite_inlier").unwrap_or(0.10)
}

fn p(key: &str, default: f64) -> f64 {
    evidence::param_f64(key).unwrap_or(default)
}

/// Bite points this close to the crowns face them (RMS, coherence, mm).
pub const NEAR: f64 = 0.5;
/// Bite points this close to the lower scan belong to the lower arch (mm).
pub const OWN: f64 = 0.5;
/// Spacing of the coarse starts along the arch (mm).
pub const STEP: f64 = 2.5;
/// A bite scan this wide goes round the front of the arch (mm): its coarse
/// search starts on the incisors.
pub const FULL_ARCH_MM: f64 = 45.0;
/// Normal gate of the coarse search (degrees).
pub const COARSE_DEG: f64 = 60.0;
/// Normal gate of the fine alignment (degrees).
pub const FINE_DEG: f64 = 45.0;

/// Errors of the bite registration (nothing is moved).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BiteError {
    /// The input cannot be registered.
    #[error("{0}")]
    Invalid(String),
    /// No alignment was found: use the 3-point alignment.
    #[error("no alignment found between the bite scan '{bite}' and the {arch} arch")]
    NoAlignment {
        /// Bite scan.
        bite: String,
        /// Arch name.
        arch: String,
    },
    /// Too little overlap between a bite scan and an arch.
    #[error(
        "too little overlap between '{bite}' and the {arch} arch: {teeth} teeth and {area:.0} mm2 \
         in common (at least {min_teeth} teeth and {min_area:.0} mm2 needed): the scan must show \
         the outer faces of at least two teeth of both arches in contact"
    )]
    Overlap {
        /// Bite scan.
        bite: String,
        /// Arch name.
        arch: String,
        /// Teeth in common.
        teeth: usize,
        /// Area in common (mm²).
        area: f64,
        /// Fewest teeth accepted.
        min_teeth: usize,
        /// Smallest area accepted (mm²).
        min_area: f64,
    },
    /// The bite scan does not match the arch.
    #[error(
        "'{bite}' does not match the {arch} arch: at its best place its teeth lie \
         {tooth_fit:.2} mm from the crowns (at most {tooth_fit_max:.2} mm) and change by \
         {tooth_jump:.2} mm from one tooth to the next (at most {tooth_jump_max:.2} mm): check \
         that the bite scan is of this patient and not mirrored"
    )]
    Mismatch {
        /// Bite scan.
        bite: String,
        /// Arch name.
        arch: String,
        /// Median distance of the typical tooth (mm).
        tooth_fit: f64,
        /// Largest accepted (mm).
        tooth_fit_max: f64,
        /// Change of the offset from one tooth to the next (mm).
        tooth_jump: f64,
        /// Largest accepted (mm).
        tooth_jump_max: f64,
    },
    /// Two different places of the arch fit the bite scan about as well.
    #[error(
        "'{bite}' fits two places of the {arch} arch about equally well ({apart_mm:.1} mm \
         apart): use a bite scan with more teeth, or the 3-point alignment"
    )]
    Ambiguous {
        /// Bite scan.
        bite: String,
        /// Arch name.
        arch: String,
        /// How far apart the two places are (mm, on the bite scan).
        apart_mm: f64,
    },
    /// The registered relation is not plausible.
    #[error("bite registration rejected: {0}")]
    Rejected(String),
}

// ----------------------------------------------------------------------------
// data
// ----------------------------------------------------------------------------

/// Points, normals and areas of a bite scan (case coordinates).
#[derive(Debug, Clone)]
pub struct BiteScan {
    /// Name (for the report).
    pub name: String,
    /// Vertices with a positive area.
    pub p: Vec<DVec3>,
    /// Area-weighted vertex normals (file winding).
    pub n: Vec<DVec3>,
    /// Area of each vertex (a third of its triangles).
    pub a: Vec<f64>,
    /// Total area (mm²).
    pub area: f64,
}

impl BiteScan {
    /// Bite scan of a mesh in case coordinates.
    ///
    /// # Errors
    /// [`BiteError::Invalid`] without triangles or for a very small scan
    /// (units are not millimetres).
    pub fn new(name: &str, vertices: &[DVec3], tris: &[[u32; 3]]) -> Result<Self, BiteError> {
        if tris.is_empty() || vertices.is_empty() {
            return Err(BiteError::Invalid(format!(
                "the bite scan '{name}' must be a mesh with faces"
            )));
        }
        let (lo, hi) = vertices.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        if (hi - lo).max_element() < 5.0 {
            return Err(BiteError::Invalid(format!(
                "'{name}' is very small: check that its units are millimetres"
            )));
        }
        let mut area = vec![0.0; vertices.len()];
        let mut nrm = vec![DVec3::ZERO; vertices.len()];
        for t in tris {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            let cr = (b - a).cross(c - a);
            let ar = 0.5 * cr.length();
            for &i in t {
                area[i as usize] += ar / 3.0;
                nrm[i as usize] += cr;
            }
        }
        let mut out = Self {
            name: name.to_owned(),
            p: Vec::new(),
            n: Vec::new(),
            a: Vec::new(),
            area: 0.0,
        };
        for i in 0..vertices.len() {
            if area[i] > 0.0 {
                out.p.push(vertices[i]);
                out.n.push(nrm[i].normalize_or_zero());
                out.a.push(area[i]);
            }
        }
        out.area = out.a.iter().sum();
        Ok(out)
    }

    fn from_parts(name: &str, p: Vec<DVec3>, n: Vec<DVec3>, a: Vec<f64>) -> Self {
        let area = a.iter().sum();
        Self {
            name: name.to_owned(),
            p,
            n,
            a,
            area,
        }
    }
}

/// Triangles of part of an arch scan (arch coordinates) with outward face
/// normals and a BVH.
#[derive(Debug, Clone)]
pub struct Surface {
    /// Triangles (outward winding), indices into the arch vertices.
    pub tris: Vec<[u32; 3]>,
    target: IcpTarget,
}

impl Surface {
    fn new(v: &[DVec3], tris: &[[u32; 3]], sign: f64) -> Option<Self> {
        if tris.is_empty() {
            return None;
        }
        let tris: Vec<[u32; 3]> = if sign >= 0.0 {
            tris.to_vec()
        } else {
            tris.iter().map(|t| [t[0], t[2], t[1]]).collect()
        };
        let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), tris.clone());
        let target = IcpTarget::new(&mesh).ok()?;
        Some(Self { tris, target })
    }

    /// Outward unit normal of a face.
    pub fn normal(&self, face: u32) -> DVec3 {
        self.target.face_normal(face)
    }

    /// The hierarchy over the triangles.
    pub fn bvh(&self) -> &Bvh {
        self.target.bvh()
    }

    /// Closest surface point within `gate` mm: (point, face, distance).
    pub fn nearest(&self, p: DVec3, gate: f64) -> Option<(DVec3, u32, f64)> {
        let g = if gate.is_finite() {
            gate as f32
        } else {
            f32::INFINITY
        };
        self.bvh().closest_point(p.as_vec3(), g).map(|h| {
            let q = h.point.as_dvec3();
            (q, h.face, p.distance(q))
        })
    }
}

/// An arch scan prepared for the registration (`bite.Arch`).
#[derive(Debug, Clone)]
pub struct BiteArch {
    /// The arch model (arch coordinates, case frame).
    pub model: ArchModel,
    /// Outward sign of the scan winding.
    pub sign: f64,
    /// Crown vertices.
    pub ci: Vec<u32>,
    /// Crown triangles: fine alignment and overlap.
    pub crowns: Surface,
    /// Crown faces on the margin (a bite point facing them may lie on the
    /// gingiva).
    pub border: Vec<bool>,
    /// Crowns plus the gingiva within 2 mm: coarse search.
    pub coarse: Surface,
    /// The whole scan.
    pub whole: Surface,
}

impl BiteArch {
    /// Prepares a labelled arch.
    ///
    /// # Errors
    /// [`BiteError::Invalid`] for an arch without crown triangles.
    pub fn new(model: ArchModel) -> Result<Self, BiteError> {
        let v = &model.vertices;
        let lab = &model.labels;
        let crown: Vec<bool> = lab.iter().map(|&l| l != 0).collect();
        let mut cz: Vec<f64> = model
            .crown_indices()
            .iter()
            .map(|&i| v[i as usize].z)
            .collect();
        let cut = occlusion::percentile(&mut cz, 85.0);
        // sum of the unit normals' z of the top crown vertices
        let mut nrm = vec![DVec3::ZERO; v.len()];
        for t in &model.tris {
            let [a, b, c] = t.map(|i| v[i as usize]);
            let cr = (b - a).cross(c - a);
            for &i in t {
                nrm[i as usize] += cr;
            }
        }
        let s: f64 = (0..v.len())
            .filter(|&i| crown[i] && v[i].z >= cut)
            .map(|i| nrm[i].normalize_or_zero().z)
            .sum();
        let sign = if s >= 0.0 { 1.0 } else { -1.0 };
        let ci = model.crown_indices();
        let tc: Vec<[u32; 3]> = model
            .tris
            .iter()
            .filter(|t| t.iter().all(|&i| crown[i as usize]))
            .copied()
            .collect();
        let crowns = Surface::new(v, &tc, sign)
            .ok_or_else(|| BiteError::Invalid("an arch has no segmented tooth".to_owned()))?;
        // crown vertices on an edge to the gingiva
        let mut margin = vec![false; v.len()];
        for t in &model.tris {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                if crown[a] != crown[b] {
                    margin[a] = true;
                    margin[b] = true;
                }
            }
        }
        let border = crowns
            .tris
            .iter()
            .map(|t| t.iter().any(|&i| margin[i as usize]))
            .collect();
        let mut near = crown.clone();
        for i in 0..v.len() {
            if !crown[i] && crowns.nearest(v[i], 2.0).is_some() {
                near[i] = true;
            }
        }
        let tn: Vec<[u32; 3]> = model
            .tris
            .iter()
            .filter(|t| t.iter().all(|&i| near[i as usize]))
            .copied()
            .collect();
        let coarse = Surface::new(v, &tn, sign)
            .ok_or_else(|| BiteError::Invalid("an arch has no segmented tooth".to_owned()))?;
        let whole = Surface::new(v, &model.tris, sign)
            .ok_or_else(|| BiteError::Invalid("an arch has no triangles".to_owned()))?;
        Ok(Self {
            model,
            sign,
            ci,
            crowns,
            border,
            coarse,
            whole,
        })
    }

    fn label_of_face(&self, face: u32) -> u16 {
        self.model.labels[self.crowns.tris[face as usize][0] as usize]
    }

    /// Coarse starts on the buccal surface: (point, mesio-distal tangent,
    /// buccal direction, right side) every [`STEP`] mm from the central
    /// incisor to the last molar (arch coordinates).
    pub fn anchors(&self, right_side: Option<bool>) -> Vec<(DVec3, DVec3, DVec3, bool)> {
        let v = &self.model.vertices;
        let cents: BTreeMap<u16, DVec3> = self
            .model
            .crown_idx
            .iter()
            .filter(|(_, idx)| idx.len() > 30)
            .map(|(&f, idx)| {
                let c = idx.iter().map(|&i| v[i as usize]).sum::<DVec3>() / idx.len() as f64;
                (f, c)
            })
            .collect();
        if cents.is_empty() {
            return Vec::new();
        }
        let allc = cents.values().copied().sum::<DVec3>() / cents.len() as f64;
        let mut out = Vec::new();
        for right in [true, false] {
            if right_side.is_some_and(|r| r != right) {
                continue;
            }
            let mut fs: Vec<u16> = cents
                .keys()
                .copied()
                .filter(|&f| is_right(f) == right)
                .collect();
            fs.sort_by_key(|&f| index(f));
            if fs.is_empty() {
                continue;
            }
            let mut pts = Vec::new();
            let mut tans = Vec::new();
            let mut outs = Vec::new();
            for (k, &f) in fs.iter().enumerate() {
                let c = cents[&f];
                let a = if k > 0 { cents[&fs[k - 1]] } else { c };
                let b = if k + 1 < fs.len() {
                    cents[&fs[k + 1]]
                } else {
                    c
                };
                let mut t = b - a;
                t.z = 0.0;
                if t.length() < 1e-6 {
                    t = DVec3::Y;
                }
                let t = t.normalize();
                let mut o = t.cross(DVec3::Z).normalize_or_zero();
                let mut h = c - allc;
                h.z = 0.0;
                if o.dot(h) < 0.0 {
                    o = -o;
                }
                let reach = self.model.crown_idx[&f]
                    .iter()
                    .map(|&i| (v[i as usize] - c).dot(o))
                    .fold(f64::NEG_INFINITY, f64::max);
                pts.push(c + o * reach);
                tans.push(t);
                outs.push(o);
            }
            let mut cum = vec![0.0];
            for k in 1..pts.len() {
                cum.push(cum[k - 1] + pts[k].distance(pts[k - 1]));
            }
            let total = *cum.last().unwrap_or(&0.0);
            let stations: Vec<f64> = if pts.len() > 1 {
                let n = ((total + 1e-6) / STEP).ceil() as usize;
                (0..n).map(|k| k as f64 * STEP).collect()
            } else {
                vec![0.0]
            };
            for s in stations {
                let j = if pts.len() > 1 {
                    let j = cum.partition_point(|&c| c <= s).saturating_sub(1);
                    j.min(pts.len() - 2)
                } else {
                    0
                };
                let j1 = (j + 1).min(pts.len() - 1);
                let w = if j1 == j {
                    0.0
                } else {
                    (s - cum[j]) / (cum[j1] - cum[j]).max(1e-9)
                };
                out.push((
                    (1.0 - w) * pts[j] + w * pts[j1],
                    ((1.0 - w) * tans[j] + w * tans[j1]).normalize_or_zero(),
                    ((1.0 - w) * outs[j] + w * outs[j1]).normalize_or_zero(),
                    right,
                ));
            }
        }
        out
    }

    /// The most labial point of the incisor crowns (arch coordinates).
    pub fn front(&self) -> DVec3 {
        let v = &self.model.vertices;
        let idx: Vec<u32> = self
            .model
            .crown_idx
            .iter()
            .filter(|(f, _)| index(**f) <= 2)
            .flat_map(|(_, i)| i.iter().copied())
            .collect();
        let idx = if idx.is_empty() { self.ci.clone() } else { idx };
        let pts: Vec<DVec3> = idx.iter().map(|&i| v[i as usize]).collect();
        let mut xs: Vec<f64> = pts.iter().map(|p| p.x).collect();
        let med = occlusion::percentile(&mut xs, 50.0);
        let mid: Vec<DVec3> = pts
            .iter()
            .copied()
            .filter(|p| (p.x - med).abs() < 6.0)
            .collect();
        let pts = if mid.len() >= 20 { mid } else { pts };
        let ymax = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        let top: Vec<DVec3> = pts.into_iter().filter(|p| p.y >= ymax - 0.7).collect();
        top.iter().copied().sum::<DVec3>() / top.len().max(1) as f64
    }
}

fn is_right(fdi: u16) -> bool {
    Fdi::new(fdi as u8).is_ok_and(anatomy::is_right)
}

fn index(fdi: u16) -> u8 {
    Fdi::new(fdi as u8).map_or(9, anatomy::index)
}

// ----------------------------------------------------------------------------
// ICP
// ----------------------------------------------------------------------------

fn to_affine(m: &DMat4) -> DAffine3 {
    DAffine3::from_mat4(*m)
}

fn to_mat4(a: &DAffine3) -> DMat4 {
    DMat4::from(*a)
}

/// Trimmed ICP of `src` (with normals) onto a surface, starting at `t`
/// (`bite._icp`). Returns the transform and the overlap, `None` when the
/// surface was lost.
#[allow(clippy::too_many_arguments)]
fn run_icp(
    src: &[DVec3],
    src_n: &[DVec3],
    surf: &Surface,
    t: &DMat4,
    iters: usize,
    gate: f64,
    angle_deg: f64,
    plane: bool,
    xi_min: f64,
) -> (DMat4, Option<f64>) {
    let metric = if plane {
        IcpMetric::PointToPlane
    } else {
        IcpMetric::PointToPoint
    };
    let params = IcpParams {
        initial: to_affine(t),
        ..IcpParams::bite(iters, gate, angle_deg, metric, xi_min)
    };
    match icp::icp(src, Some(src_n), &surf.target, &params) {
        Ok(r) if !r.lost => (to_mat4(&r.transform), Some(r.overlap)),
        Ok(r) => (to_mat4(&r.transform), None),
        Err(_) => (*t, None),
    }
}

/// Share of the source points within `tol` of the surface with compatible
/// normals (`bite._score`).
fn overlap_score(surf: &Surface, src: &[DVec3], src_n: &[DVec3], t: &DMat4, tol: f64) -> f64 {
    let c = COARSE_DEG.to_radians().cos();
    let r = DMat3::from_mat4(*t);
    let good = src
        .iter()
        .zip(src_n)
        .filter(|(p, n)| {
            surf.nearest(t.transform_point3(**p), tol)
                .is_some_and(|(_, f, _)| (r * **n).dot(surf.normal(f)) >= c)
        })
        .count();
    good as f64 / src.len().max(1) as f64
}

/// The middle band in height of a bite scan (its points between the 20th
/// and the 80th percentile across its mesio-distal and buccal axes): the
/// teeth without the lips and cheeks a scan catches above and below them.
/// `None` when too little or all of the scan is left.
fn core_band(b: &BiteScan) -> Option<BiteScan> {
    let (g, a1, nb) = bite_frame(b);
    let up = nb.cross(a1).normalize_or_zero();
    let mut hs: Vec<f64> = b.p.iter().map(|p| (*p - g).dot(up)).collect();
    let lo = occlusion::percentile(&mut hs, 20.0);
    let hi = occlusion::percentile(&mut hs, 80.0);
    let keep: Vec<usize> = (0..b.p.len())
        .filter(|&i| (lo..=hi).contains(&(b.p[i] - g).dot(up)))
        .collect();
    (keep.len() >= 100 && keep.len() < b.p.len()).then(|| {
        BiteScan::from_parts(
            &b.name,
            pick(&b.p, &keep),
            pick(&b.n, &keep),
            pick(&b.a, &keep),
        )
    })
}

fn pick<T: Copy>(v: &[T], idx: &[usize]) -> Vec<T> {
    idx.iter().map(|&i| v[i]).collect()
}

fn signed(n: &[DVec3], sg: f64) -> Vec<DVec3> {
    n.iter().map(|&v| v * sg).collect()
}

/// Eigen decomposition of a symmetric 3x3 matrix: (values, vectors as
/// columns), ascending.
fn eigh(m: &Matrix3<f64>) -> ([f64; 3], [DVec3; 3]) {
    let e = SymmetricEigen::new(*m);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| e.eigenvalues[a].total_cmp(&e.eigenvalues[b]));
    let vals = order.map(|k| e.eigenvalues[k]);
    let vecs = order.map(|k| {
        let c = e.eigenvectors.column(k);
        DVec3::new(c[0], c[1], c[2])
    });
    (vals, vecs)
}

fn weighted_cov(p: &[DVec3], w: &[f64], g: DVec3) -> Matrix3<f64> {
    let mut m = Matrix3::zeros();
    for (q, &wi) in p.iter().zip(w) {
        let d = *q - g;
        let d = [d.x, d.y, d.z];
        for r in 0..3 {
            for c in 0..3 {
                m[(r, c)] += wi * d[r] * d[c];
            }
        }
    }
    m
}

/// Centroid, mesio-distal axis and buccal axis of a bite scan (`bite._frame`).
fn bite_frame(b: &BiteScan) -> (DVec3, DVec3, DVec3) {
    let w: Vec<f64> = b.a.iter().map(|a| a / b.area).collect();
    let g = b.p.iter().zip(&w).map(|(p, w)| *p * *w).sum::<DVec3>();
    let nb =
        b.n.iter()
            .zip(&w)
            .map(|(n, w)| *n * *w)
            .sum::<DVec3>()
            .normalize_or_zero();
    let q: Vec<DVec3> =
        b.p.iter()
            .map(|p| {
                let d = *p - g;
                d - d.dot(nb) * nb
            })
            .collect();
    let (_, vecs) = eigh(&weighted_cov(&q, &w, DVec3::ZERO));
    let a1 = vecs[2];
    (g, (a1 - a1.dot(nb) * nb).normalize_or_zero(), nb)
}

/// Turns tried about every start of the free search (axis: 0 the
/// mesio-distal tangent of the arch, 1 its occlusal axis, 2 the buccal
/// direction; degrees), the unturned start first. The frame of a bite scan
/// (mean normal, longest extent) is only roughly the frame of its buccal
/// surface once it carries gingiva, both arches, holes, a curve of the arch
/// or soft tissue: measured on bite scans cut from real arches, no start of
/// the plugin's search came within 25 degrees of the right pose for a right
/// side or a front scan, and the scan landed on the wrong teeth.
const TURNS: [(usize, f64); 9] = [
    (2, 0.0),
    (2, -30.0),
    (2, -15.0),
    (2, 15.0),
    (2, 30.0),
    (1, -20.0),
    (1, 20.0),
    (0, -20.0),
    (0, 20.0),
];

/// Where the centre of the bite scan goes about a buccal start (mm, along
/// the occlusal axis and outwards): the plugin's two places first, then
/// higher and more buccal, where the centre of a scan carrying the upper
/// teeth and gingiva lies (the upper teeth overlap the lower ones
/// buccally). [`hypotheses_free`] adds the places where the most buccal
/// part of the scan touches the start: the centre of a scan round the
/// curve of the front teeth lies several millimetres inside the arch.
const PLACES: [(f64, f64); 6] = [
    (-1.5, 0.5),
    (2.5, 0.5),
    (5.5, 0.5),
    (-1.5, 2.0),
    (2.5, 2.0),
    (5.5, 2.0),
];

/// Every orientation of the bite frame on every buccal start
/// (`bite._hypotheses_free`): (start world -> arch, normal sign). `wide`:
/// each also at the [`PLACES`] and turned by [`TURNS`] (the plugin's starts
/// otherwise).
fn hypotheses_free(arch: &BiteArch, b: &BiteScan, wide: bool) -> Vec<(DMat4, f64)> {
    let w = frame::rigid_inverse(&arch.model.to_case);
    let (g, a1, nb) = bite_frame(b);
    // how far the most buccal part of the scan lies from its centre, for
    // either winding of its normals
    let mut ahead: Vec<f64> = b.p.iter().map(|p| (*p - g).dot(nb)).collect();
    let front = occlusion::percentile(&mut ahead, 95.0);
    let back = -occlusion::percentile(&mut ahead, 5.0);
    let (g, a1, nb) = (
        w.transform_point3(g),
        w.transform_vector3(a1),
        w.transform_vector3(nb),
    );
    let turns: &[(usize, f64)] = if wide { &TURNS } else { &TURNS[..1] };
    let mut out = Vec::new();
    for (p, t, o, _) in arch.anchors(None) {
        let u = DMat3::from_cols(t, o.cross(t), o);
        let axes = [t, DVec3::Z, o];
        for s3 in [1.0, -1.0] {
            let mut places = PLACES[..2].to_vec();
            if wide {
                places = PLACES.to_vec();
                let reach = if s3 > 0.0 { front } else { back };
                if reach > 1.0 {
                    places.extend([-1.5, 2.5, 5.5].map(|h| (h, 0.5 - reach)));
                }
            }
            for s1 in [1.0, -1.0] {
                let (a1s, a3s) = (a1 * s1, nb * s3);
                let r = u * DMat3::from_cols(a1s, a3s.cross(a1s), a3s).transpose();
                for &(h, d) in &places {
                    let c = p + DVec3::Z * h + d * o;
                    for &(axis, deg) in turns {
                        let r = DMat3::from_axis_angle(axes[axis], deg.to_radians()) * r;
                        let mut tm = DMat4::from_mat3(r);
                        tm.w_axis = (c - r * g).extend(1.0);
                        out.push((tm * w, s3));
                    }
                }
            }
        }
    }
    out
}

/// `true` for a bite scan round the whole arch (wider than
/// [`FULL_ARCH_MM`] along its longest axis).
pub fn is_full_arch(b: &BiteScan) -> bool {
    let w: Vec<f64> = b.a.iter().map(|a| a / b.area).collect();
    let g = b.p.iter().zip(&w).map(|(p, w)| *p * *w).sum::<DVec3>();
    let (_, vecs) = eigh(&weighted_cov(&b.p, &w, g));
    let e = vecs[2];
    let proj: Vec<f64> = b.p.iter().map(|p| (*p - g).dot(e)).collect();
    ptp(&proj) >= FULL_ARCH_MM
}

fn ptp(v: &[f64]) -> f64 {
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    hi - lo
}

fn median(v: &[f64]) -> f64 {
    let mut c = v.to_vec();
    occlusion::percentile(&mut c, 50.0)
}

/// Starts of a bite scan round the whole arch (`bite._hypotheses_full`);
/// empty for a scan of one side.
fn hypotheses_full(arch: &BiteArch, b: &BiteScan) -> Vec<(DMat4, f64)> {
    const HEIGHTS: [f64; 9] = [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0];
    const YAWS: [f64; 9] = [-30.0, -22.5, -15.0, -7.5, 0.0, 7.5, 15.0, 22.5, 30.0];
    const DEPTHS: [f64; 3] = [-3.0, 0.0, 2.0];
    let w = frame::rigid_inverse(&arch.model.to_case);
    let pa: Vec<DVec3> = b.p.iter().map(|&p| w.transform_point3(p)).collect();
    let wa: Vec<f64> = b.a.iter().map(|a| a / b.area).collect();
    let ga = pa.iter().zip(&wa).map(|(p, w)| *p * *w).sum::<DVec3>();
    let (_, vecs) = eigh(&weighted_cov(&pa, &wa, ga));
    let (e1, e2, e3) = (vecs[2], vecs[1], vecs[0]);
    let lat: Vec<f64> = pa.iter().map(|p| (*p - ga).dot(e1)).collect();
    if ptp(&lat) < FULL_ARCH_MM {
        return Vec::new();
    }
    let ap: Vec<f64> = pa.iter().map(|p| (*p - ga).dot(e2)).collect();
    let med = median(&lat);
    let mid: Vec<bool> = lat.iter().map(|l| (l - med).abs() < 6.0).collect();
    let ap_mean = ap.iter().sum::<f64>() / ap.len() as f64;
    let n_mid = mid.iter().filter(|m| **m).count();
    let ap_mid = if n_mid > 0 {
        ap.iter()
            .zip(&mid)
            .filter(|(_, m)| **m)
            .map(|(a, _)| a)
            .sum::<f64>()
            / n_mid as f64
    } else {
        0.0
    };
    let yd = if n_mid == 0 || ap_mid >= ap_mean {
        e2
    } else {
        -e2
    };
    // the normals of the file: outwards from the middle of the horseshoe
    let s: f64 =
        b.n.iter()
            .zip(&pa)
            .zip(&wa)
            .map(|((n, p), wi)| {
                let na = w.transform_vector3(*n);
                let d = *p - ga;
                let rad = d - d.dot(e3) * e3;
                wi * na.dot(rad)
            })
            .sum();
    let sg = if s >= 0.0 { 1.0 } else { -1.0 };
    let pf_front = arch.front();
    let mut out = Vec::new();
    for zd in [e3, -e3] {
        let r0 = DMat3::from_cols(yd.cross(zd), yd, zd).transpose();
        for yaw in YAWS {
            let r = DMat3::from_axis_angle(DVec3::Z, yaw.to_radians()) * r0;
            let q: Vec<DVec3> = pa.iter().map(|p| r * (*p - ga)).collect();
            let qx: Vec<f64> = q.iter().map(|p| p.x).collect();
            let medq = median(&qx);
            let midq: Vec<bool> = qx.iter().map(|x| (x - medq).abs() < 6.0).collect();
            if !midq.iter().any(|m| *m) {
                continue;
            }
            let ymax = q
                .iter()
                .zip(&midq)
                .filter(|(_, m)| **m)
                .map(|(p, _)| p.y)
                .fold(f64::NEG_INFINITY, f64::max);
            let sel: Vec<DVec3> = pa
                .iter()
                .zip(&q)
                .zip(&midq)
                .filter(|((_, qq), m)| **m && qq.y >= ymax - 0.7)
                .map(|((p, _), _)| *p)
                .collect();
            let pf = sel.iter().copied().sum::<DVec3>() / sel.len().max(1) as f64;
            for h in HEIGHTS {
                for d in DEPTHS {
                    let mut tm = DMat4::from_mat3(r);
                    tm.w_axis = (pf_front + DVec3::Z * h + DVec3::Y * d - r * pf).extend(1.0);
                    out.push((tm * w, sg));
                }
            }
        }
    }
    out
}

/// Coarse search of a scan round the whole arch (`bite._coarse_full`).
fn coarse_full(arch: &BiteArch, b: &BiteScan, hyps: &[(DMat4, f64)]) -> Option<(DMat4, f64, f64)> {
    let sub = sub_idx(b.p.len(), 1000);
    let src = pick(&b.p, &sub);
    let nrm = pick(&b.n, &sub);
    let mut best: Option<(DMat4, f64, f64)> = None;
    for (t0, sg) in hyps {
        let sn = signed(&nrm, *sg);
        let (t, xi) = run_icp(&src, &sn, &arch.coarse, t0, 25, 5.0, COARSE_DEG, true, 0.2);
        if xi.is_none() {
            continue;
        }
        let sc = overlap_score(&arch.coarse, &src, &sn, &t, 0.3);
        if best.is_none_or(|b| sc > b.2) {
            best = Some((t, *sg, sc));
        }
    }
    best
}

/// Starts with a known orientation (the upper arch faces the lower one):
/// only the position along the buccal surface of one side is searched
/// (`bite._hypotheses_prior`).
fn hypotheses_prior(
    arch: &BiteArch,
    b: &BiteScan,
    prior: &DMat4,
    right_side: Option<bool>,
) -> Vec<(DMat4, f64)> {
    let w: Vec<f64> = b.a.iter().map(|a| a / b.area).collect();
    let g = prior.transform_point3(b.p.iter().zip(&w).map(|(p, w)| *p * *w).sum::<DVec3>());
    let q: Vec<DVec3> = b.p.iter().map(|&p| prior.transform_point3(p)).collect();
    let mut out = Vec::new();
    let qx: Vec<f64> = q.iter().map(|p| p.x).collect();
    if ptp(&qx) >= FULL_ARCH_MM {
        let p = arch.front();
        let med = median(&qx);
        let mid: Vec<DVec3> = q
            .iter()
            .copied()
            .filter(|p| (p.x - med).abs() < 6.0)
            .collect();
        let qm = if mid.len() >= 20 { mid } else { q.clone() };
        let ymax = qm.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        let top: Vec<DVec3> = qm.into_iter().filter(|p| p.y >= ymax - 0.7).collect();
        let gf = top.iter().copied().sum::<DVec3>() / top.len().max(1) as f64;
        for h in [-3.0, -1.5, 0.0, 1.5, 3.0] {
            let mut t = *prior;
            t.w_axis += (p + DVec3::Z * h - gf).extend(0.0);
            out.push((t, 1.0));
        }
    }
    for (p, _, o, _) in arch.anchors(right_side) {
        for h in [-3.0, -0.5] {
            let mut t = *prior;
            t.w_axis += (p + DVec3::Z * h + 0.5 * o - g).extend(0.0);
            out.push((t, 1.0));
        }
    }
    out
}

/// Multi-start trimmed ICP (`bite._coarse`): the refined starts (start world
/// -> arch, normal sign, overlap score), best first; the plugin keeps the
/// first one.
fn coarse(
    arch: &BiteArch,
    b: &BiteScan,
    hyps: &[(DMat4, f64)],
    wide: bool,
) -> Vec<(DMat4, f64, f64)> {
    // the plugin: 8 point-to-point iterations from every start, the best 8
    // refined; the wide search, its starts turned up to 30 degrees from
    // the truth, converges with point-to-plane iterations
    let (iters, plane, refined) = if wide {
        (15, true, REFINED_WIDE)
    } else {
        (8, false, REFINED)
    };
    let s1 = sub_idx(b.p.len(), 200);
    let s2 = sub_idx(b.p.len(), 1500);
    let (p1, n1) = (pick(&b.p, &s1), pick(&b.n, &s1));
    let (p2, n2) = (pick(&b.p, &s2), pick(&b.n, &s2));
    use rayon::prelude::*;
    let mut res: Vec<(f64, DMat4, f64)> = hyps
        .par_iter()
        .filter_map(|(t0, sg)| {
            let sn = signed(&n1, *sg);
            let (t, xi) = run_icp(
                &p1,
                &sn,
                &arch.coarse,
                t0,
                iters,
                4.0,
                COARSE_DEG,
                plane,
                0.2,
            );
            xi.map(|_| (overlap_score(&arch.coarse, &p1, &sn, &t, 0.5), t, *sg))
        })
        .collect();
    // stable: equal scores keep the order of the starts
    res.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<(DMat4, f64, f64)> = Vec::new();
    let mut kept: Vec<DMat4> = Vec::new();
    for (_, t, sg) in res {
        let distinct = kept.iter().all(|k| {
            let dt = (t.w_axis - k.w_axis).truncate().length();
            let r = DMat3::from_mat4(t) * DMat3::from_mat4(*k).transpose();
            dt >= 1.0 || rot_angle(&r) >= 5f64.to_radians()
        });
        if !distinct {
            continue;
        }
        kept.push(t);
        let sn = signed(&n2, sg);
        let (t2, xi) = run_icp(&p2, &sn, &arch.coarse, &t, 20, 1.5, COARSE_DEG, true, 0.2);
        if xi.is_some() {
            let sc2 = overlap_score(&arch.coarse, &p2, &sn, &t2, 0.25);
            out.push((t2, sg, sc2));
        }
        if kept.len() >= refined {
            break;
        }
    }
    // stable: equal scores keep the order of the starts, the plugin's best
    // (the first highest) comes first
    out.sort_by(|a, b| b.2.total_cmp(&a.2));
    out
}

/// Starts of the multi-start search refined at the second level: 8 in the
/// plugin, 16 in the wide free search, whose starts are turned by
/// [`TURNS`].
const REFINED: usize = 8;
/// See [`REFINED`].
const REFINED_WIDE: usize = 16;

fn rot_angle(r: &DMat3) -> f64 {
    let tr = r.x_axis.x + r.y_axis.y + r.z_axis.z;
    ((tr - 1.0) * 0.5).clamp(-1.0, 1.0).acos()
}

/// Point-to-plane trimmed ICP on the crowns only (`bite._fine`).
fn fine(arch: &BiteArch, b: &BiteScan, t: &DMat4, sg: f64, n: usize) -> (DMat4, Option<f64>) {
    let s = sub_idx(b.p.len(), n);
    let src = pick(&b.p, &s);
    let sn = signed(&pick(&b.n, &s), sg);
    let (t1, xi) = run_icp(&src, &sn, &arch.crowns, t, 40, 1.0, FINE_DEG, true, 0.1);
    if xi.is_none() {
        return (*t, None);
    }
    let (t2, xi2) = run_icp(&src, &sn, &arch.crowns, &t1, 30, 0.5, FINE_DEG, true, 0.1);
    if xi2.is_some() { (t2, xi2) } else { (t1, xi) }
}

/// Overlap of a registered bite scan with the crowns of an arch
/// (`bite.metrics`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FitMetrics {
    /// Bite area within the inlier distance of the crowns (mm²).
    pub area_mm2: f64,
    /// Teeth with at least the tooth area in common, with their area (mm²).
    pub teeth: BTreeMap<u16, f64>,
    /// Number of those teeth.
    pub n_teeth: usize,
    /// RMS of the facing surfaces away from the margins (mm).
    pub rms_mm: f64,
    /// 95th percentile of the facing distances (mm).
    pub p95_mm: Option<f64>,
    /// Share of the facing surfaces within the inlier distance.
    pub coherence: f64,
    /// Share of the bite scan in common.
    pub bite_fraction: f64,
    /// Overlap of the last ICP iteration.
    pub icp_overlap: f64,
    /// Area of the bite scan facing the crowns (within [`NEAR`] mm, normals
    /// compatible; mm²).
    pub facing_mm2: f64,
    /// Median distance of the bite scan to each facing tooth away from the
    /// margins (mm), for the teeth with at least [`TOOTH_SAMPLES`] points.
    pub tooth_median_mm: BTreeMap<u16, f64>,
    /// Median signed distance (outside the crown: positive) of the bite
    /// scan to each of those teeth (mm).
    pub tooth_offset_mm: BTreeMap<u16, f64>,
    /// Median of [`FitMetrics::tooth_median_mm`] (mm, `INFINITY` without a
    /// facing tooth): how closely the teeth themselves fit, whatever the
    /// holes, the soft tissue or the extent of the scan around them.
    pub tooth_fit_mm: f64,
    /// RMS of the change of [`FitMetrics::tooth_offset_mm`] from a tooth to
    /// the next one along the arch (mm, `None` without two neighbours): the
    /// distortion of a scan changes it smoothly, a scan of other teeth from
    /// tooth to tooth.
    pub tooth_jump_mm: Option<f64>,
}

/// Fewest sampled bite points facing a tooth for its own median distance.
pub const TOOTH_SAMPLES: usize = 20;

fn round_to(x: f64, d: i32) -> f64 {
    let f = 10f64.powi(d);
    (x * f).round() / f
}

/// Overlap metrics of a registered bite scan (`T`: bite -> arch).
pub fn metrics(arch: &BiteArch, b: &BiteScan, t: &DMat4, sg: f64) -> FitMetrics {
    let cos_fine = FINE_DEG.to_radians().cos();
    let inl = inlier();
    let tooth_area = p("bite_tooth_area", 8.0);
    let s = sub_idx(b.p.len(), 40000);
    let sub_area: f64 = s.iter().map(|&i| b.a[i]).sum();
    let scale = b.area / sub_area.max(1e-12);
    let r = DMat3::from_mat4(*t);
    let mut area_in = 0.0;
    let mut area_inner = 0.0;
    let mut area_inner_in = 0.0;
    let mut teeth: BTreeMap<u16, f64> = BTreeMap::new();
    let mut inner_d: Vec<f64> = Vec::new();
    let mut facing = 0.0;
    let mut per_tooth: BTreeMap<u16, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for &i in &s {
        let ab = b.a[i] * scale;
        let pp = t.transform_point3(b.p[i]);
        let nn = r * (b.n[i] * sg);
        let Some((q, f, d)) = arch.crowns.nearest(pp, NEAR) else {
            continue;
        };
        let fn_ = arch.crowns.normal(f);
        if nn.dot(fn_) < cos_fine {
            continue;
        }
        facing += ab;
        if d <= inl {
            area_in += ab;
            let fdi = arch.label_of_face(f);
            if fdi != 0 {
                *teeth.entry(fdi).or_default() += ab;
            }
        }
        if !arch.border[f as usize] {
            area_inner += ab;
            inner_d.push(d);
            if d <= inl {
                area_inner_in += ab;
            }
            let fdi = arch.label_of_face(f);
            if fdi != 0 {
                let e = per_tooth.entry(fdi).or_default();
                e.0.push(d);
                e.1.push((pp - q).dot(fn_));
            }
        }
    }
    teeth.retain(|_, a| *a >= tooth_area);
    for a in teeth.values_mut() {
        *a = round_to(*a, 1);
    }
    let rms = if inner_d.is_empty() {
        f64::INFINITY
    } else {
        (inner_d.iter().map(|d| d * d).sum::<f64>() / inner_d.len() as f64).sqrt()
    };
    let p95 = (!inner_d.is_empty()).then(|| round_to(occlusion::percentile(&mut inner_d, 95.0), 3));
    let mut tooth_median_mm = BTreeMap::new();
    let mut tooth_offset_mm = BTreeMap::new();
    for (fdi, (mut d, mut s)) in per_tooth {
        if d.len() >= TOOTH_SAMPLES {
            tooth_median_mm.insert(fdi, round_to(occlusion::percentile(&mut d, 50.0), 3));
            tooth_offset_mm.insert(fdi, round_to(occlusion::percentile(&mut s, 50.0), 3));
        }
    }
    let tooth_fit_mm = if tooth_median_mm.is_empty() {
        f64::INFINITY
    } else {
        median(&tooth_median_mm.values().copied().collect::<Vec<_>>())
    };
    // neighbours along the arch: the next tooth of the quadrant, and the
    // central incisors across the midline
    let mut jumps = Vec::new();
    for (&f, &o) in &tooth_offset_mm {
        let (quadrant, i) = (f / 10, f % 10);
        let mut next = vec![quadrant * 10 + i + 1];
        if i == 1 && quadrant % 2 == 1 {
            next.push((quadrant + 1) * 10 + 1);
        }
        jumps.extend(
            next.iter()
                .filter_map(|n| tooth_offset_mm.get(n))
                .map(|&o2| o2 - o),
        );
    }
    let tooth_jump_mm = (!jumps.is_empty()).then(|| {
        round_to(
            (jumps.iter().map(|j| j * j).sum::<f64>() / jumps.len() as f64).sqrt(),
            3,
        )
    });
    FitMetrics {
        area_mm2: round_to(area_in, 1),
        n_teeth: teeth.len(),
        teeth,
        rms_mm: round_to(rms, 3),
        p95_mm: p95,
        coherence: if area_inner > 0.0 {
            round_to(area_inner_in / area_inner, 3)
        } else {
            0.0
        },
        bite_fraction: round_to(area_in / b.area.max(1e-9), 3),
        icp_overlap: 0.0,
        facing_mm2: round_to(facing, 1),
        tooth_median_mm,
        tooth_offset_mm,
        tooth_fit_mm,
        tooth_jump_mm,
    }
}

/// Largest median distance of the typical tooth of a fit (mm,
/// [`FitMetrics::tooth_fit_mm`]): beyond it the teeth of the bite scan are
/// not the teeth of the arch.
pub const TOOTH_FIT_MAX: f64 = 0.20;
/// Largest [`FitMetrics::tooth_jump_mm`] of a fit (mm): beyond it the
/// teeth of the bite scan sit on the crowns in a way no distortion of a
/// scan explains (real scan of models: up to 0.065 mm; the same scans
/// mirrored: 0.10-0.12 mm).
pub const TOOTH_JUMP_MAX: f64 = 0.08;

/// Refusal of a fit (`bite._check_fit`, with other limits): too little in
/// common to place the arch, or teeth that are not the teeth of the arch.
///
/// The plugin also refuses an RMS above `bite_rms_max` (0.25 mm round the
/// arch) or coherent surfaces below `bite_coherence` (0.55 round the arch)
/// and asks 8 teeth of a scan round the arch. A real bite scan of models
/// on a desktop scanner (MINI 2) fits the incisors within 0.04 mm and
/// drifts to 0.2-0.3 mm on the molars (distortion between two scans of the
/// same models): RMS 0.18, coherence 0.43, refused although the place is
/// right. Those figures only say how good the scan is; they become reasons
/// to review the registration ([`register`]). What refuses is how the teeth
/// in common sit on the crowns: [`FitMetrics::tooth_fit_mm`] (that scan:
/// 0.08-0.14 mm; its bites on another patient's real arches: 0.21-0.24 mm)
/// and [`FitMetrics::tooth_jump_mm`] (that scan: 0.04-0.07 mm; the bite of
/// another demo patient: 0.22 mm), whatever the quality of the scan. A
/// mirrored bite of a symmetric dentition can fit its teeth: the
/// interpenetration of the registered arches refuses it ([`register`]).
///
/// # Errors
/// [`BiteError::Overlap`] or [`BiteError::Mismatch`].
pub fn check_fit(m: &FitMetrics, arch: &str, bite: &str) -> Result<(), BiteError> {
    let min_teeth = p("bite_min_teeth", 2.0) as usize;
    let min_area = p("bite_min_area", 90.0);
    if m.n_teeth < min_teeth || m.area_mm2 < min_area {
        return Err(BiteError::Overlap {
            bite: bite.to_owned(),
            arch: arch.to_owned(),
            teeth: m.n_teeth,
            area: m.area_mm2,
            min_teeth,
            min_area,
        });
    }
    let jump = m.tooth_jump_mm.unwrap_or(0.0);
    if m.tooth_fit_mm > TOOTH_FIT_MAX || jump > TOOTH_JUMP_MAX {
        return Err(BiteError::Mismatch {
            bite: bite.to_owned(),
            arch: arch.to_owned(),
            tooth_fit: m.tooth_fit_mm,
            tooth_fit_max: TOOTH_FIT_MAX,
            tooth_jump: jump,
            tooth_jump_max: TOOTH_JUMP_MAX,
        });
    }
    Ok(())
}

/// `Some(true)` / `Some(false)`: the bite scan lies on the right / left
/// side; `None`: on both or unknown (`bite._side`).
pub fn side(m: &FitMetrics) -> Option<bool> {
    if m.teeth.is_empty() {
        return None;
    }
    let right: f64 = m
        .teeth
        .iter()
        .filter(|(f, _)| is_right(**f))
        .map(|(_, a)| a)
        .sum();
    let total: f64 = m.teeth.values().sum();
    let share = right / total.max(1e-9);
    if (0.2..=0.8).contains(&share) {
        return None;
    }
    Some(share >= 0.5)
}

/// Manual pairing: transform bite scan -> arch coordinates from 3 or more
/// corresponding points picked on the bite scan and on the arch (case
/// coordinates; Kabsch 1976), `bite.kabsch_init`.
///
/// # Errors
/// [`BiteError::Invalid`] for fewer than 3 pairs, sets of different
/// lengths, or collinear or too close points.
pub fn kabsch_init(
    arch_to_case: &DMat4,
    arch_pts: &[DVec3],
    bite_pts: &[DVec3],
) -> Result<DMat4, BiteError> {
    if bite_pts.len() < 3 || bite_pts.len() != arch_pts.len() {
        return Err(BiteError::Invalid(
            "3 pairs of corresponding points are needed (arch and bite)".to_owned(),
        ));
    }
    for x in [bite_pts, arch_pts] {
        let c = x.iter().copied().sum::<DVec3>() / x.len() as f64;
        let w = vec![1.0; x.len()];
        let (vals, _) = eigh(&weighted_cov(x, &w, c));
        // second singular value of the centred points
        if vals[1].max(0.0).sqrt() < 2.0 {
            return Err(BiteError::Invalid(
                "the 3 points are aligned or too close: pick distant points (for example \
                 canine, premolar and molar)"
                    .to_owned(),
            ));
        }
    }
    let w = frame::rigid_inverse(arch_to_case);
    let dst: Vec<DVec3> = arch_pts.iter().map(|&p| w.transform_point3(p)).collect();
    let a =
        icp::kabsch(bite_pts, &dst, None, false).map_err(|e| BiteError::Invalid(e.to_string()))?;
    Ok(to_mat4(&a))
}

/// Winding of the bite scan normals once its pose is known
/// (`bite._normal_sign_from_init`).
fn normal_sign_from_init(arch: &BiteArch, b: &BiteScan, t: &DMat4) -> f64 {
    let s = sub_idx(b.p.len(), 2000);
    let r = DMat3::from_mat4(*t);
    let mut pairs: Vec<(f64, f64)> = s
        .iter()
        .filter_map(|&i| {
            let (_, f, d) = arch.coarse.nearest(t.transform_point3(b.p[i]), 3.0)?;
            Some((d, (r * b.n[i]).dot(arch.coarse.normal(f))))
        })
        .collect();
    if pairs.is_empty() {
        return 1.0;
    }
    let mut ds: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let q = occlusion::percentile(&mut ds, 50.0);
    pairs.retain(|p| p.0 <= q);
    let dots: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    if median(&dots) >= 0.0 { 1.0 } else { -1.0 }
}

/// A registered bite scan: (bite -> arch coordinates, normal sign, metrics).
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// Bite scan (case coordinates) to arch coordinates.
    pub transform: DMat4,
    /// Sign that turns the bite normals outwards.
    pub normal_sign: f64,
    /// Overlap with the arch.
    pub metrics: FitMetrics,
}

/// Coarse poses taken to the fine alignment (the plugin takes the best one
/// only): the best and the next distinct ones scoring at least half of it.
const ALTERNATIVES: usize = 3;
/// A fit this much larger (common area) replaces an earlier one.
const CLEARLY_BETTER: f64 = 1.2;
/// Two fits farther apart than this (mm, on the bite scan) are different
/// places on the arch.
const DISTINCT_MM: f64 = 1.0;
/// A different place with this share of the common area of the best fit
/// makes the fit ambiguous.
const AMBIGUOUS_SHARE: f64 = 0.85;

/// Registers one bite scan to an arch (`bite.fit`). `init`: manual pairing
/// (bite -> arch); `prior`: known orientation for the upper arch.
///
/// The plugin takes the best coarse pose of one search to the fine
/// alignment. Here the next distinct poses of that search go too; when
/// none of them gives an acceptable fit ([`check_fit`]) the wide free
/// search over the whole arch adds its own, then the same search on the
/// middle band of the scan alone ([`core_band`]): a scan of the front teeth
/// only, with holes, with soft tissue above and below the teeth, or round
/// the arch without its lingual side does not always fit at the first pose.
/// The acceptable fit
/// with the most surface in common wins (an earlier one is kept unless a
/// later one has [`CLEARLY_BETTER`] times its area); another acceptable fit
/// at a different place with nearly as much ([`AMBIGUOUS_SHARE`]) makes the
/// registration ambiguous.
///
/// # Errors
/// [`BiteError::NoAlignment`] when no start converges,
/// [`BiteError::Ambiguous`] for two equally good places.
pub fn fit(
    arch: &BiteArch,
    b: &BiteScan,
    init: Option<&DMat4>,
    prior: Option<&DMat4>,
    right_side: Option<bool>,
    arch_name: &str,
) -> Result<Fit, BiteError> {
    let finish = |t: &DMat4, sg: f64| {
        let (t, xi) = fine(arch, b, t, sg, 20000);
        let mut m = metrics(arch, b, &t, sg);
        m.icp_overlap = round_to(xi.unwrap_or(0.0), 3);
        Fit {
            transform: t,
            normal_sign: sg,
            metrics: m,
        }
    };
    if let Some(init) = init {
        let sg = normal_sign_from_init(arch, b, init);
        let s2 = sub_idx(b.p.len(), 1500);
        let src = pick(&b.p, &s2);
        let sn = signed(&pick(&b.n, &s2), sg);
        let (t, xi) = run_icp(
            &src,
            &sn,
            &arch.coarse,
            init,
            30,
            3.0,
            COARSE_DEG,
            true,
            0.2,
        );
        return Ok(finish(if xi.is_some() { &t } else { init }, sg));
    }
    let mut starts: Vec<(DMat4, f64, f64)> = Vec::new();
    let full = hypotheses_full(arch, b);
    if !full.is_empty() {
        let mut cands = vec![coarse_full(arch, b, &full)];
        if let Some(prior) = prior {
            let h: Vec<_> = hypotheses_prior(arch, b, prior, right_side)
                .into_iter()
                .take(5)
                .collect();
            cands.push(coarse_full(arch, b, &h));
        }
        starts.extend(
            cands
                .into_iter()
                .flatten()
                .max_by(|a, b| a.2.total_cmp(&b.2)),
        );
    } else if let Some(prior) = prior {
        let h = hypotheses_prior(arch, b, prior, right_side);
        starts = coarse(arch, b, &h, false);
    }
    if starts.is_empty() {
        starts = coarse(arch, b, &hypotheses_free(arch, b, false), false);
    }
    let pts: Vec<DVec3> = occlusion::subsample(&b.p, 500);
    let mut fits: Vec<Fit> = Vec::new();
    let evaluate = |starts: &[(DMat4, f64, f64)], fits: &mut Vec<Fit>| {
        let Some(top) = starts.first().map(|s| s.2) else {
            return;
        };
        let mut taken: Vec<DMat4> = Vec::new();
        for (t, sg, sc) in starts {
            if taken.len() >= ALTERNATIVES || *sc < 0.5 * top {
                break;
            }
            if taken.iter().any(|k| disagreement(k, t, &pts) < DISTINCT_MM) {
                continue;
            }
            taken.push(*t);
            fits.push(finish(t, *sg));
        }
    };
    evaluate(&starts, &mut fits);
    let ok = |f: &Fit| check_fit(&f.metrics, arch_name, &b.name).is_ok();
    if !fits.iter().any(ok) {
        let h = hypotheses_free(arch, b, true);
        evaluate(&coarse(arch, b, &h, true), &mut fits);
    }
    // the soft tissue a scan catches above and below the teeth can pull its
    // frame away from every start: the middle band of the scan alone
    if !fits.iter().any(ok)
        && let Some(core) = core_band(b)
    {
        let h = hypotheses_free(arch, &core, true);
        evaluate(&coarse(arch, &core, &h, true), &mut fits);
    }
    // the first acceptable fit, unless a later one is clearly better; then
    // the first fit (refused by the caller with its reasons)
    let mut best: Option<usize> = None;
    for (k, f) in fits.iter().enumerate() {
        if !ok(f) {
            continue;
        }
        if best.is_none_or(|j| f.metrics.area_mm2 > CLEARLY_BETTER * fits[j].metrics.area_mm2) {
            best = Some(k);
        }
    }
    if let Some(j) = best {
        let rival = fits.iter().enumerate().find(|(k, f)| {
            *k != j
                && ok(f)
                && f.metrics.area_mm2 >= AMBIGUOUS_SHARE * fits[j].metrics.area_mm2
                && disagreement(&f.transform, &fits[j].transform, &pts) >= DISTINCT_MM
        });
        if let Some((_, r)) = rival {
            return Err(BiteError::Ambiguous {
                bite: b.name.clone(),
                arch: arch_name.to_owned(),
                apart_mm: round_to(disagreement(&r.transform, &fits[j].transform, &pts), 1),
            });
        }
        return Ok(fits.swap_remove(j));
    }
    if fits.is_empty() {
        return Err(BiteError::NoAlignment {
            bite: b.name.clone(),
            arch: arch_name.to_owned(),
        });
    }
    Ok(fits.swap_remove(0))
}

// ----------------------------------------------------------------------------
// workflow
// ----------------------------------------------------------------------------

/// Upper arch frame in lower arch coordinates when the upper arch is moved
/// so that `t_upper` maps the case to its arch coordinates (`bite._relation`).
fn relation_of(ml: &DMat4, t_upper: &DMat4) -> DMat4 {
    frame::rigid_inverse(ml) * frame::rigid_inverse(t_upper)
}

/// Largest displacement (mm) of `pts` between two relations
/// (`bite.disagreement`).
pub fn disagreement(a: &DMat4, b: &DMat4, pts: &[DVec3]) -> f64 {
    pts.iter()
        .map(|&p| a.transform_point3(p).distance(b.transform_point3(p)))
        .fold(0.0, f64::max)
}

/// Manual pairing of the first bite scan, in case coordinates.
#[derive(Debug, Clone, Default)]
pub struct ManualInit {
    /// Bite -> lower arch coordinates.
    pub lower: Option<DMat4>,
    /// Bite -> upper arch coordinates (the bite scan before the
    /// registration).
    pub upper: Option<DMat4>,
}

/// One bite scan in the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanReport {
    /// Bite scan.
    pub scan: String,
    /// Side: `right`, `left`, `both` or `None` (unknown).
    pub side: Option<&'static str>,
    /// Fit on the lower arch.
    pub lower: FitMetrics,
    /// Fit on the upper arch.
    pub upper: FitMetrics,
}

/// Outcome of a registration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Registration {
    /// Rigid move of the upper scan in case coordinates.
    #[serde(skip)]
    pub upper_delta: DMat4,
    /// Rigid move of each bite scan in case coordinates (onto the lower
    /// arch).
    #[serde(skip)]
    pub bite_deltas: Vec<DMat4>,
    /// Upper arch coordinates to lower arch coordinates after the move.
    #[serde(skip)]
    pub relation: DMat4,
    /// Per bite scan.
    pub scans: Vec<ScanReport>,
    /// Agreement of the two bite scans (mm), `None` with one scan.
    pub agreement_mm: Option<f64>,
    /// Deepest interpenetration of the registered arches (mm).
    pub penetration_mm: f64,
    /// Upper crown points in contact.
    pub contact_points: usize,
    /// Vertical move to the first contact (mm; positive = interpenetration,
    /// negative = open).
    pub opening_at_contact_mm: f64,
    /// Dot product of the occlusal axes (−1 = facing).
    pub facing: f64,
    /// `good` or `to_review`.
    pub verdict: &'static str,
    /// Reasons to review the registration (codes).
    pub reasons: Vec<String>,
    /// Bite scans left out because they do not fit (the other one does).
    pub ignored: Vec<IgnoredScan>,
}

/// A bite scan the registration left out, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IgnoredScan {
    /// Bite scan.
    pub scan: String,
    /// Why it was left out.
    pub why: String,
}

/// One bite scan registered to both arches.
struct Registered {
    lower: Fit,
    upper: Fit,
    /// Its points off the lower arch, on the lower arch (case coordinates).
    rest: BiteScan,
}

/// A bite scan onto the lower arch, then the rest of it onto the upper one
/// (`k`: its order, the manual pairing is for the first). The error comes
/// with the move of the scan onto the lower arch when that fit holds.
fn register_one(
    lower: &BiteArch,
    upper: &BiteArch,
    b: &BiteScan,
    k: usize,
    init: &ManualInit,
) -> Result<Registered, (BiteError, Option<Box<DMat4>>)> {
    let ml = lower.model.to_case;
    let i = if k == 0 { init.lower.as_ref() } else { None };
    let f = fit(lower, b, i, None, None, "lower").map_err(|e| (e, None))?;
    check_fit(&f.metrics, "lower", &b.name).map_err(|e| (e, None))?;
    // the bite points of the lower arch are removed; the rest goes to the
    // upper arch, which faces the lower one
    let t = f.transform;
    let g = ml * t; // bite scan on the lower arch (case)
    let rg = DMat3::from_mat4(g);
    let (mut rp, mut rn, mut ra) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..b.p.len() {
        if lower
            .whole
            .nearest(t.transform_point3(b.p[i]), OWN)
            .is_none()
        {
            rp.push(g.transform_point3(b.p[i]));
            rn.push(rg * (b.n[i] * f.normal_sign));
            ra.push(b.a[i]);
        }
    }
    let rest = BiteScan::from_parts(&b.name, rp, rn, ra);
    if rest.area < p("bite_min_area", 90.0) {
        return Err((
            BiteError::Invalid(format!(
                "'{}' does not contain the upper teeth: the bite scan must include both arches \
                 in contact",
                b.name
            )),
            Some(Box::new(g)),
        ));
    }
    let i = if k == 0 {
        init.upper.map(|u| u * frame::rigid_inverse(&g))
    } else {
        None
    };
    let prior = DMat4::from_mat3(S_FLIP) * frame::rigid_inverse(&ml);
    let fu = fit(
        upper,
        &rest,
        i.as_ref(),
        Some(&prior),
        side(&f.metrics),
        "upper",
    )
    .map_err(|e| (e, Some(Box::new(g))))?;
    check_fit(&fu.metrics, "upper", &b.name).map_err(|e| (e, Some(Box::new(g))))?;
    Ok(Registered {
        lower: f,
        upper: fu,
        rest,
    })
}

/// Regions of the arches the registered bite scans hold, both arches
/// together (teeth in common, [`FitMetrics::teeth`]): (right posterior,
/// left posterior). Premolars and molars count; the canines and incisors
/// are the front.
fn posterior_sides(used: &[Registered]) -> (bool, bool) {
    let held = |right: bool| {
        [true, false].iter().all(|&lower_arch| {
            used.iter().any(|u| {
                let m = if lower_arch {
                    &u.lower.metrics
                } else {
                    &u.upper.metrics
                };
                m.teeth
                    .keys()
                    .any(|&f| index(f) >= 4 && index(f) <= 8 && is_right(f) == right)
            })
        })
    };
    (held(true), held(false))
}

fn side_name(m: &FitMetrics) -> Option<&'static str> {
    match side(m) {
        None if !m.teeth.is_empty() => Some("both"),
        None => None,
        Some(true) => Some("right"),
        Some(false) => Some("left"),
    }
}

/// Registers the arches to one or two buccal bite scans (`bite.register`):
/// the lower arch stays in place, the bite scans move onto it and the upper
/// arch moves onto the bite scans.
///
/// Two bite scans are the two sides or two takes of the same teeth: a scan
/// that does not fit while the other one does is left out
/// ([`Registration::ignored`], reason `scan_ignored:<scan>`) and moved onto
/// the lower arch when that fit holds. A scan of the front teeth only, of
/// one side, with holes, noise or soft tissue registers when its teeth fit
/// ([`check_fit`]); how good the scan is and what it covers become reasons
/// to review (`rms_above_good`, `few_teeth`, `low_coherence`,
/// `one_side_only`, `front_only`).
///
/// # Errors
/// [`BiteError`] (nothing is to be moved) when the registration is not
/// acceptable: no bite scan fits, or the arches do not face each other or
/// interpenetrate beyond `bite_penetration_max`.
pub fn register(
    lower: &BiteArch,
    upper: &BiteArch,
    bites: &[BiteScan],
    init: &ManualInit,
) -> Result<Registration, BiteError> {
    if bites.is_empty() {
        return Err(BiteError::Invalid(
            "import at least one buccal bite scan".to_owned(),
        ));
    }
    if bites.len() > 2 {
        return Err(BiteError::Invalid(
            "at most two bite scans (the two sides, or two takes of the same teeth)".to_owned(),
        ));
    }
    let ml = lower.model.to_case;
    let mu = upper.model.to_case;
    // 1-2. every bite scan onto the lower arch, its rest onto the upper arch
    let mut used: Vec<Registered> = Vec::new();
    let mut moves: Vec<Option<DMat4>> = Vec::new();
    let mut failed: Vec<(usize, BiteError, bool)> = Vec::new();
    for (k, b) in bites.iter().enumerate() {
        match register_one(lower, upper, b, k, init) {
            Ok(r) => {
                moves.push(Some(ml * r.lower.transform));
                used.push(r);
            }
            Err((e, on_lower)) => {
                failed.push((k, e, on_lower.is_some()));
                moves.push(on_lower.map(|m| *m));
            }
        }
    }
    if used.is_empty() {
        // the scan that went furthest says best why
        let j = failed.iter().position(|f| f.2).unwrap_or(0);
        return Err(failed.swap_remove(j).1);
    }
    // 3. both bite scans together; agreement of the relations of each one
    let ci_sub = subsample_idx(&upper.ci, 4000);
    let pts: Vec<DVec3> = ci_sub
        .iter()
        .map(|&i| upper.model.vertices[i as usize])
        .collect();
    let rels: Vec<DMat4> = used
        .iter()
        .map(|u| relation_of(&ml, &u.upper.transform))
        .collect();
    let agree = (rels.len() == 2).then(|| disagreement(&rels[0], &rels[1], &pts));
    let tu = if used.len() == 2 {
        let j = if used[1].upper.metrics.area_mm2 > used[0].upper.metrics.area_mm2 {
            1
        } else {
            0
        };
        let mut bp = Vec::new();
        let mut bn = Vec::new();
        let mut ba = Vec::new();
        for u in &used {
            bp.extend(&u.rest.p);
            bn.extend(u.rest.n.iter().map(|n| *n * u.upper.normal_sign));
            ba.extend(&u.rest.a);
        }
        let both = BiteScan::from_parts(&bites[0].name, bp, bn, ba);
        let (t, xi) = fine(upper, &both, &used[j].upper.transform, 1.0, 30000);
        if xi.is_some() {
            t
        } else {
            used[j].upper.transform
        }
    } else {
        used[0].upper.transform
    };
    // 4. the upper case onto the bite scans
    let upper_delta = frame::rigid_inverse(&tu) * frame::rigid_inverse(&mu);
    let mu2 = upper_delta * mu;
    let relation = frame::rigid_inverse(&ml) * mu2;
    // 5. plausibility: facing arches, interpenetration
    let facing = ml.z_axis.truncate().dot(mu2.z_axis.truncate());
    let lower_surface = lower
        .model
        .crown_surface()
        .ok_or_else(|| BiteError::Invalid("the lower arch has no crown".to_owned()))?;
    let (pu, lab_u) = upper.model.crown_points(None);
    let q: Vec<DVec3> = pu.iter().map(|&p| relation.transform_point3(p)).collect();
    let q15 = occlusion::subsample(&q, 15000);
    let (depth, contacts) = occlusion::penetration(&lower_surface, &q15);
    let pen_max = p("bite_penetration_max", 1.0);
    if facing > -0.9 {
        return Err(BiteError::Rejected(format!(
            "the registered arches do not face each other (occlusal planes {:.0} degrees apart)",
            (-facing).clamp(-1.0, 1.0).acos().to_degrees()
        )));
    }
    if depth > pen_max {
        return Err(BiteError::Rejected(format!(
            "interpenetration of {depth:.2} mm between the arches (at most {pen_max:.1} mm is \
             plausible)"
        )));
    }
    let (pl, _) = lower.model.crown_points(None);
    let gap = occlusion::contact_offset(&pl, &lower_surface, &q, &lab_u).dz;
    // report
    let rms_good = p("bite_rms_good", 0.10);
    let coherent = p("bite_coherence", 0.70);
    let good_teeth = p("bite_teeth", 4.0) as usize;
    let mut reasons = Vec::new();
    let mut scans = Vec::new();
    for u in &used {
        let name = &u.rest.name;
        for (m, arch) in [(&u.lower.metrics, "lower"), (&u.upper.metrics, "upper")] {
            if m.rms_mm > rms_good {
                reasons.push(format!("rms_above_good:{name}:{arch}"));
            }
            if m.n_teeth < good_teeth {
                reasons.push(format!("few_teeth:{name}:{arch}"));
            }
            if m.coherence < coherent {
                reasons.push(format!("low_coherence:{name}:{arch}"));
            }
        }
        scans.push(ScanReport {
            scan: name.clone(),
            side: side_name(&u.lower.metrics),
            lower: u.lower.metrics.clone(),
            upper: u.upper.metrics.clone(),
        });
    }
    let mut ignored = Vec::new();
    for (k, e, _) in &failed {
        reasons.push(format!("scan_ignored:{}", bites[*k].name));
        ignored.push(IgnoredScan {
            scan: bites[*k].name.clone(),
            why: e.to_string(),
        });
    }
    match posterior_sides(&used) {
        (true, true) => {}
        (false, false) => reasons.push("front_only".to_owned()),
        _ => reasons.push("one_side_only".to_owned()),
    }
    if agree.is_some_and(|a| a > p("bite_agreement", 0.10)) {
        reasons.push("scans_disagree".to_owned());
    }
    if depth > p("bite_penetration_warn", 0.10) {
        reasons.push("interpenetration".to_owned());
    }
    if gap < -p("bite_gap_warn", 0.10) {
        reasons.push("arches_not_in_contact".to_owned());
    }
    let bite_deltas = moves
        .into_iter()
        .map(|m| m.unwrap_or(DMat4::IDENTITY))
        .collect();
    Ok(Registration {
        upper_delta,
        bite_deltas,
        relation,
        scans,
        agreement_mm: agree.map(|a| round_to(a, 3)),
        penetration_mm: round_to(depth, 3),
        contact_points: contacts,
        opening_at_contact_mm: round_to(gap, 3),
        facing: round_to(facing, 3),
        verdict: if reasons.is_empty() {
            "good"
        } else {
            "to_review"
        },
        reasons,
        ignored,
    })
}

/// Smallest interpenetration the correction of the bite removes (mm,
/// `bite.correct_penetration`).
pub const CORRECT_MIN_MM: f64 = 0.021;

/// Opening of a bite to the first contact (`bite.correct_penetration`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Correction {
    /// Rigid move of the upper arch in case coordinates: a translation along
    /// the occlusal axis of the lower arch.
    #[serde(skip)]
    pub upper_delta: DMat4,
    /// How far the bite opens (mm).
    pub opening_mm: f64,
    /// Deepest interpenetration left after the opening (mm).
    pub penetration_mm: f64,
    /// Upper crown points in contact after the opening.
    pub contact_points: usize,
    /// Upper and lower tooth that touch first (`0` when unknown).
    pub first_contact: [u16; 2],
}

/// The correction of the interpenetration of a bite, as the bite adjustment
/// of intraoral scanner software (`bite.correct_penetration`): the upper arch
/// moves away along the occlusal axis of the lower arch until the first
/// contact, as with the teeth unloaded. `None` when the arches do not
/// interpenetrate (at most [`CORRECT_MIN_MM`] to the first contact). The
/// interpenetration and the contacts after the move are measured as the
/// registration measures them.
///
/// # Errors
/// [`BiteError::Invalid`] when the lower arch has no crown.
pub fn correct_penetration(
    lower: &ArchModel,
    upper: &ArchModel,
) -> Result<Option<Correction>, BiteError> {
    let no_crown = || BiteError::Invalid("the lower arch has no crown".to_owned());
    let first = occlusion::first_contact(lower, upper).ok_or_else(no_crown)?;
    let dz = first.dz;
    if dz <= CORRECT_MIN_MM {
        return Ok(None);
    }
    let upper_delta = DMat4::from_translation(lower.to_case.z_axis.truncate() * dz);
    let relation = frame::rigid_inverse(&lower.to_case) * upper_delta * upper.to_case;
    let surface = lower.crown_surface().ok_or_else(no_crown)?;
    let (pu, _) = upper.crown_points(None);
    let q: Vec<DVec3> = pu.iter().map(|&p| relation.transform_point3(p)).collect();
    let (depth, contacts) = occlusion::penetration(&surface, &occlusion::subsample(&q, 15000));
    Ok(Some(Correction {
        upper_delta,
        opening_mm: dz,
        penetration_mm: round_to(depth, 3),
        contact_points: contacts,
        first_contact: [first.upper_fdi, first.lower_fdi],
    }))
}

fn subsample_idx(v: &[u32], n: usize) -> Vec<u32> {
    occlusion::subsample(v, n)
}

/// A synthetic buccal bite scan of two arches in occlusion, for tests and
/// demos: the buccal faces of both arches from the canine to the first molar
/// of one side (`right`: the patient's right), within 7 mm of the occlusal
/// plane, in case coordinates. NOT A PATIENT SCAN.
pub fn demo_bite_scan(
    lower: &ArchModel,
    upper: &ArchModel,
    right: bool,
) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    let rel = occlusion::relation(lower, upper);
    let (cl, _) = lower.crown_points(None);
    let centre = cl.iter().copied().sum::<DVec3>() / cl.len() as f64;
    // from the canine to the first molar of the lower arch (about 4 teeth)
    let ys: Vec<f64> = lower
        .crown_idx
        .iter()
        .filter(|(f, _)| (3..=6).contains(&index(**f)) && is_right(**f) == right)
        .flat_map(|(_, idx)| idx.iter().map(|&i| lower.vertices[i as usize].y))
        .collect();
    let (y0, y1) = (
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for (m, to_lower) in [(lower, DMat4::IDENTITY), (upper, rel)] {
        let v: Vec<DVec3> = m
            .vertices
            .iter()
            .map(|&p| to_lower.transform_point3(p))
            .collect();
        let mut map = std::collections::HashMap::new();
        for t in &m.tris {
            let [a, b, c] = t.map(|i| v[i as usize]);
            let g = (a + b + c) / 3.0;
            let n = (b - a).cross(c - a).normalize_or_zero();
            let radial = DVec3::new(g.x - centre.x, g.y - centre.y, 0.0).normalize_or_zero();
            let side = (if right { g.x > 8.0 } else { g.x < -8.0 }) && (y0..=y1).contains(&g.y);
            if side && g.z.abs() < 7.0 && n.dot(radial) > 0.2 {
                let mut tri = [0u32; 3];
                for k in 0..3 {
                    let i = t[k];
                    tri[k] = *map.entry(i).or_insert_with(|| {
                        verts.push(lower.to_case.transform_point3(v[i as usize]));
                        (verts.len() - 1) as u32
                    });
                }
                tris.push(tri);
            }
        }
    }
    (verts, tris)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anatomy::Arch;
    use crate::demo;

    fn model(arch: Arch, seed: u64) -> ArchModel {
        let d = demo::run(&demo::Params {
            arch,
            seed,
            ..demo::Params::default()
        });
        let v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let labels = d.mesh.vertex_labels.clone().unwrap();
        ArchModel::new(arch, &v, &d.mesh.tris, &labels, &d.seeds, false).unwrap()
    }

    #[test]
    fn small_or_empty_bite_scans_are_rejected() {
        let v = [DVec3::ZERO, DVec3::X, DVec3::Y];
        assert!(matches!(
            BiteScan::new("b", &v, &[]),
            Err(BiteError::Invalid(m)) if m.contains("faces")
        ));
        assert!(matches!(
            BiteScan::new("b", &v, &[[0, 1, 2]]),
            Err(BiteError::Invalid(m)) if m.contains("millimetres")
        ));
        let v = [DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 10.0];
        let b = BiteScan::new("b", &v, &[[0, 1, 2]]).unwrap();
        assert!((b.area - 50.0).abs() < 1e-9);
        assert!((b.n[0] - DVec3::Z).length() < 1e-12);
    }

    #[test]
    fn kabsch_init_recovers_three_points() {
        let to_case = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
        let arch = [
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(0.0, 12.0, 0.0),
            DVec3::new(-8.0, 3.0, 1.0),
        ];
        let x = DMat4::from_rotation_z(0.3) * DMat4::from_translation(DVec3::new(5.0, -1.0, 2.0));
        let bite: Vec<DVec3> = arch.iter().map(|&p| x.transform_point3(p)).collect();
        let t = kabsch_init(&to_case, &arch, &bite).unwrap();
        for (a, b) in arch.iter().zip(&bite) {
            let got = t.transform_point3(*b);
            let want = frame::rigid_inverse(&to_case).transform_point3(*a);
            assert!(got.distance(want) < 1e-9);
        }
        let line = [DVec3::ZERO, DVec3::X * 5.0, DVec3::X * 10.0];
        assert!(kabsch_init(&to_case, &line, &line).is_err());
        assert!(kabsch_init(&to_case, &arch[..2], &bite[..2]).is_err());
    }

    fn rot_err_deg(a: &DMat4, b: &DMat4) -> f64 {
        rot_angle(&(DMat3::from_mat4(*a) * DMat3::from_mat4(*b).transpose())).to_degrees()
    }

    #[test]
    fn a_known_transform_is_recovered_on_the_demo_arches() {
        let lower = model(Arch::Lower, 7);
        let upper = model(Arch::Upper, 11);
        // the true relation: the virtual bite of the two demo arches
        let vb = occlusion::align(&lower, &upper, &occlusion::Params::default()).unwrap();
        let upper_true = ArchModel {
            to_case: vb.delta * upper.to_case,
            ..upper.clone()
        };
        // a bite scan of each side, in the coordinates of the scanner
        let x = DMat4::from_translation(DVec3::new(14.0, -9.0, 22.0))
            * DMat4::from_axis_angle(DVec3::new(0.3, -1.0, 0.5).normalize(), 0.9);
        let mut bites = Vec::new();
        for (right, name) in [(true, "bite-right"), (false, "bite-left")] {
            let (v, t) = demo_bite_scan(&lower, &upper_true, right);
            let v: Vec<DVec3> = v.iter().map(|&p| x.transform_point3(p)).collect();
            bites.push(BiteScan::new(name, &v, &t).unwrap());
        }
        // the upper arch is displaced by a known rigid move
        let y = DMat4::from_translation(DVec3::new(-3.0, 4.0, 6.0))
            * DMat4::from_axis_angle(DVec3::new(1.0, 0.2, -0.4).normalize(), 0.25);
        let upper_moved = ArchModel {
            to_case: y * upper_true.to_case,
            ..upper.clone()
        };
        let la = BiteArch::new(lower.clone()).unwrap();
        let ua = BiteArch::new(upper_moved.clone()).unwrap();
        for scans in [&bites[..1], &bites[..]] {
            let reg = register(&la, &ua, scans, &ManualInit::default()).unwrap();
            // the upper arch goes back exactly: the move is the inverse of y
            let back = reg.upper_delta * y;
            assert!(rot_err_deg(&back, &DMat4::IDENTITY) < 0.05, "{back:?}");
            assert!(back.w_axis.truncate().length() < 0.05, "{back:?}");
            assert!(rot_err_deg(&reg.relation, &vb.relation) < 0.05);
            // each bite scan goes back from the scanner onto the arches
            for d in &reg.bite_deltas {
                let b = *d * x;
                assert!(rot_err_deg(&b, &DMat4::IDENTITY) < 0.05, "{b:?}");
                assert!(b.w_axis.truncate().length() < 0.05, "{b:?}");
            }
            for s in &reg.scans {
                assert!(s.lower.rms_mm < 0.02, "{s:?}");
                assert!(s.upper.rms_mm < 0.02, "{s:?}");
                assert!(s.lower.n_teeth >= 3 && s.upper.n_teeth >= 3, "{s:?}");
            }
            assert!(reg.facing < -0.99);
            assert!(reg.penetration_mm < 0.05, "{reg:?}");
            if scans.len() == 2 {
                assert!(reg.agreement_mm.unwrap() < 0.05);
                assert_eq!(reg.scans[0].side, Some("right"));
                assert_eq!(reg.scans[1].side, Some("left"));
            } else {
                assert!(reg.reasons.iter().any(|r| r == "one_side_only"));
            }
        }
    }

    #[test]
    fn a_bite_scan_without_upper_teeth_is_rejected() {
        let lower = model(Arch::Lower, 7);
        let upper = model(Arch::Upper, 11);
        let vb = occlusion::align(&lower, &upper, &occlusion::Params::default()).unwrap();
        let upper_true = ArchModel {
            to_case: vb.delta * upper.to_case,
            ..upper.clone()
        };
        let (v, t) = demo_bite_scan(&lower, &upper_true, true);
        // keep the lower half only (below the occlusal plane of the lower arch)
        let to_arch = frame::rigid_inverse(&lower.to_case);
        let t: Vec<[u32; 3]> = t
            .into_iter()
            .filter(|tri| {
                tri.iter()
                    .all(|&i| to_arch.transform_point3(v[i as usize]).z < -0.5)
            })
            .collect();
        let b = BiteScan::new("lower-only", &v, &t).unwrap();
        let la = BiteArch::new(lower).unwrap();
        let ua = BiteArch::new(upper_true).unwrap();
        let err = register(&la, &ua, &[b], &ManualInit::default()).unwrap_err();
        assert!(
            matches!(&err, BiteError::Invalid(m) if m.contains("upper teeth"))
                || matches!(&err, BiteError::Overlap { .. }),
            "{err:?}"
        );
        assert!(matches!(
            register(&la, &ua, &[], &ManualInit::default()),
            Err(BiteError::Invalid(_))
        ));
    }

    #[test]
    fn anchors_lie_on_both_buccal_sides() {
        let arch = BiteArch::new(model(Arch::Lower, 7)).unwrap();
        let a = arch.anchors(None);
        assert!(a.len() > 20, "{}", a.len());
        assert!(a.iter().any(|x| x.3) && a.iter().any(|x| !x.3));
        // right side: +x in lower arch coordinates
        for (p, _, o, right) in &a {
            if p.x.abs() > 8.0 {
                assert_eq!(p.x > 0.0, *right, "{p:?}");
                assert!(o.dot(DVec3::new(p.x, 0.0, 0.0)) > 0.0);
            }
        }
        assert_eq!(arch.anchors(Some(true)).iter().filter(|x| !x.3).count(), 0);
        let f = arch.front();
        assert!(f.x.abs() < 5.0 && f.y > 20.0, "{f:?}");
    }
}
