//! Segmentation: assigns every vertex of a scan to a tooth (by FDI) or to the
//! gingiva; the border between the labels becomes the crown margin.
//!
//! Reference: `ortho_aligner/segmentation.py` (`segment_faces`,
//! `refine_boundaries`, `clean_vertex_labels`) and `ortho_aligner/case_teeth.py`
//! (`segment_iter`, `_decimate_here`, `transfer_labels`, `_labels_full`);
//! manual labelling (`ortho_aligner/manual.py`) is not part of this module.
//!
//! Method, as `case_teeth.segment_iter`:
//!
//! 1. The scan (arch coordinates) is simplified to about 30 000 faces
//!    ([`dc_geom::qem`], the stand-in for Blender's Decimate modifier).
//! 2. Random-walk segmentation on its dual (face) graph (Lai et al. 2009:
//!    `d = eta (1 - cos(dihedral))`, eta 1 on concave and 0.2 on convex
//!    edges, normalised by its mean, `w = |e| exp(-d / sigma)`; Grady 2006),
//!    seeded by a disk around every tooth seed and by a-priori gingival
//!    constraints (far from the arch line, well below the crowns, the open
//!    border of the scan; Zou 2015, Liao 2015). Repeated twice with every
//!    seed moved to the centre of its crown as just segmented.
//! 3. The face labels are transferred to the full-resolution vertices (label
//!    of the exact nearest face, near-ties by one rule), cleaned (majority
//!    filter, one connected component per label).
//! 4. Every boundary is refined at full resolution by a two-label random walk
//!    on sharper fold weights, then the labels are cleaned again.
//!
//! The linear solves are the plugin's reproducible PCG (exact dot products,
//! the operation order of its kernel), so the labels match the oracle except
//! where the transcendental functions of the weights round differently.
//!
//! Everything works in arch coordinates (origin between the last molars on
//! the occlusal plane, `+Y` towards the incisors, `+Z` occlusal; see
//! [`crate::frame::ArchFrame`]). Labels are FDI numbers, `0` for the gingiva.

mod clean;
mod faces;
mod graph;
mod mesh;
mod refine;
mod solve;
mod transfer;

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::qem::{self, DecimateParams};
use dc_mesh::TriMesh;
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::Arch;

/// Label of the gingiva (internal label space and FDI labels alike).
const GUM: i64 = 0;

/// Errors of the segmentation.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SegmentationError {
    /// The input is not usable (empty mesh, indices out of range...).
    #[error("{0}")]
    Invalid(String),
    /// No seed of the arch: nothing to segment.
    #[error("no tooth seed on this arch: place or propose the seeds first")]
    NoSeeds,
    /// The simplification failed.
    #[error("simplification of the scan failed: {0}")]
    Decimate(String),
}

/// Parameters of the segmentation command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arch to segment (`upper` or `lower`); all the arches with a scan and
    /// seeds when absent.
    pub arch: Option<Arch>,
    /// Faces of the simplified mesh of the first partition (plugin
    /// `rw_faces`, Lai et al. 2009).
    pub target_faces: usize,
    /// Weight of the convex edges in the fold measure (plugin
    /// `rw_eta_convex`, Lai et al. 2009).
    pub eta_convex: f64,
    /// Sharpness of the fold weights, `exp(-d / sigma)` (plugin `rw_sigma`).
    pub sigma: f64,
    /// Refine the boundaries at full resolution.
    pub refine: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: None,
            target_faces: 30_000,
            eta_convex: 0.2,
            sigma: 1.0,
            refine: true,
        }
    }
}

/// Parameters of the first partition on the simplified mesh
/// (`segmentation.segment_faces`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceParams {
    /// Weight of the convex edges.
    pub eta_convex: f64,
    /// Fold sharpness.
    pub sigma: f64,
    /// Relative residual of the random-walk solve.
    pub tol: f64,
    /// Iteration limit of every random-walk solve.
    pub maxiter: usize,
    /// Segmentations with the seeds moved to the centre of their crowns.
    pub recentre_passes: usize,
}

impl Default for FaceParams {
    fn default() -> Self {
        Self {
            eta_convex: 0.2,
            sigma: 1.0,
            tol: 1e-4,
            maxiter: 1500,
            recentre_passes: 2,
        }
    }
}

/// Parameters of the boundary refinement (`segmentation.refine_boundaries`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefineParams {
    /// Faces within this distance (mm) of a boundary are labelled again.
    pub band: f64,
    /// Weight of the convex edges.
    pub eta_convex: f64,
    /// Fold sharpness (sharper than on the simplified mesh).
    pub sigma: f64,
    /// Relative residual of the two-label solves.
    pub tol: f64,
    /// Iteration limit of the two-label solves.
    pub maxiter: usize,
    /// Fraction of its faces every label keeps as seeds.
    pub min_core: f64,
    /// Normals averaged over this radius (mm) before measuring the folds.
    pub normal_scale: f64,
    /// A tooth takes gingiva only this near (mm) its first boundary.
    pub gain_band_mm: Option<f64>,
}

impl Default for RefineParams {
    fn default() -> Self {
        Self {
            band: 2.5,
            eta_convex: 0.2,
            sigma: 0.25,
            tol: 1e-6,
            maxiter: 6000,
            min_core: 0.4,
            normal_scale: 0.3,
            gain_band_mm: Some(0.8),
        }
    }
}

/// Diagnostics of the first partition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FaceDiagnostics {
    /// Faces of the simplified mesh.
    pub faces: usize,
    /// CG iterations of all the random walks.
    pub cg_iterations: usize,
    /// Largest final residual of the last walk.
    pub cg_residual: f64,
    /// Largest move of a seed by the re-centring (mm, 2 decimals).
    pub seed_shift_mm: Option<f64>,
    /// Faces constrained to the gingiva in the last walk.
    pub gum_seed_faces: usize,
    /// Faces constrained to a tooth in the last walk.
    pub tooth_seed_faces: usize,
}

/// Input of [`run`]: one arch in arch coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The arch of the scan.
    pub arch: Arch,
    /// Vertices of the full scan (arch coordinates, mm).
    pub vertices: &'a [DVec3],
    /// Triangles of the full scan.
    pub tris: &'a [[u32; 3]],
    /// Tooth seeds (arch coordinates): the occlusal centre or incisal edge
    /// of every tooth. Seeds of the other arch are ignored.
    pub seeds: &'a [(Fdi, DVec3)],
    /// The simplified mesh to segment instead of simplifying the scan here
    /// (arch coordinates): e.g. the oracle's Blender-decimated mesh.
    pub decimated: Option<(&'a [DVec3], &'a [[u32; 3]])>,
}

/// Result of [`run`].
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// FDI label of every vertex of the scan (`0` = gingiva).
    pub labels: Vec<u16>,
    /// Vertex labels after the transfer and the first clean-up (before the
    /// boundary refinement).
    pub labels_transfer: Vec<u16>,
    /// FDI label of every face of the simplified mesh.
    pub face_labels_decimated: Vec<u16>,
    /// Vertices of the simplified mesh (arch coordinates).
    pub decimated_vertices: Vec<DVec3>,
    /// Triangles of the simplified mesh.
    pub decimated_tris: Vec<[u32; 3]>,
    /// Diagnostics of the first partition.
    pub diagnostics: FaceDiagnostics,
    /// Number of teeth in the final labels.
    pub teeth: usize,
}

/// First partition: FDI label of every face of a (simplified) mesh in arch
/// coordinates, from the seeds (`segmentation.segment_faces`).
pub fn segment_faces(
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    seeds: &[(Fdi, DVec3)],
    arch: Arch,
    params: &FaceParams,
) -> (Vec<u16>, FaceDiagnostics) {
    faces::segment_faces(vertices, tris, seeds, arch, params)
}

/// Undirected edges of the triangles, each once, sorted.
pub fn unique_edges(tris: &[[u32; 3]]) -> Vec<[u32; 2]> {
    mesh::unique_edges(tris)
}

/// Vertex labels of the full scan from the face labels of the simplified
/// mesh: the label of the exact nearest face, near-ties (within 1e-5 mm) to
/// the gingiva, else to the tooth of most neighbours, else to the smallest
/// face (`case_teeth.transfer_labels`).
pub fn transfer_labels(
    vertices: &[DVec3],
    edges: &[[u32; 2]],
    dec_vertices: &[DVec3],
    dec_tris: &[[u32; 3]],
    face_labels: &[u16],
) -> Result<Vec<u16>, SegmentationError> {
    transfer::transfer_labels(
        vertices,
        edges,
        dec_vertices,
        dec_tris,
        face_labels,
        transfer::TIE_MM,
    )
    .map_err(SegmentationError::Invalid)
}

/// The vertex nearest to every seed (first one on ties).
pub fn seed_vertices(vertices: &[DVec3], seeds: &[(Fdi, DVec3)]) -> BTreeMap<u16, usize> {
    seeds
        .iter()
        .map(|&(f, p)| {
            let mut best = (f64::INFINITY, 0usize);
            for (i, &v) in vertices.iter().enumerate() {
                let d = (v - p).length();
                if d < best.0 {
                    best = (d, i);
                }
            }
            (u16::from(f.get()), best.1)
        })
        .collect()
}

/// Majority filter (`smooth_iterations` times), one connected component per
/// label (a tooth keeps the one of its seed vertex), the rest refilled from
/// the neighbours (`segmentation.clean_vertex_labels`).
pub fn clean_vertex_labels(
    labels: &[u16],
    edges: &[[u32; 2]],
    seed_vertex: &BTreeMap<u16, usize>,
    smooth_iterations: usize,
) -> Vec<u16> {
    clean::clean_vertex_labels(labels, edges, seed_vertex, smooth_iterations)
}

/// Boundary refinement at full resolution (`segmentation.refine_boundaries`):
/// vertex labels.
pub fn refine_boundaries(
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    params: &RefineParams,
) -> Vec<u16> {
    refine::refine_boundaries(vertices, tris, labels, params)
}

/// The scan simplified to about `target_faces` triangles, as Blender's
/// Decimate modifier with `ratio = target / faces` (unchanged when it has
/// no more faces than the target).
pub fn decimate(
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    target_faces: usize,
) -> Result<(Vec<DVec3>, Vec<[u32; 3]>), SegmentationError> {
    if tris.len() <= target_faces {
        return Ok((vertices.to_vec(), tris.to_vec()));
    }
    let mesh = TriMesh::new(
        vertices.iter().map(|p| p.as_vec3()).collect(),
        tris.to_vec(),
    );
    let ratio = target_faces as f64 / tris.len() as f64;
    let out = qem::decimate(&mesh, &DecimateParams::to_ratio(ratio))
        .map_err(|e| SegmentationError::Decimate(e.to_string()))?;
    Ok((
        out.mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
        out.mesh.tris,
    ))
}

fn check_mesh(vertices: &[DVec3], tris: &[[u32; 3]], what: &str) -> Result<(), SegmentationError> {
    if tris.is_empty() {
        return Err(SegmentationError::Invalid(format!(
            "the {what} has no triangles"
        )));
    }
    if tris.iter().flatten().any(|&i| i as usize >= vertices.len()) {
        return Err(SegmentationError::Invalid(format!(
            "the {what} has a triangle index out of range"
        )));
    }
    if vertices.iter().any(|p| !p.is_finite()) {
        return Err(SegmentationError::Invalid(format!(
            "the {what} has a vertex that is not finite"
        )));
    }
    Ok(())
}

/// Segments one arch: simplification (unless `input.decimated` is given),
/// first partition, transfer to the full scan, clean-up, boundary
/// refinement (when `params.refine`) and the last clean-up, as
/// `case_teeth.segment_iter`.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, SegmentationError> {
    check_mesh(input.vertices, input.tris, "scan")?;
    let seeds: Vec<(Fdi, DVec3)> = input
        .seeds
        .iter()
        .copied()
        .filter(|s| Arch::of(s.0) == input.arch)
        .collect();
    if seeds.is_empty() {
        return Err(SegmentationError::NoSeeds);
    }
    if seeds.iter().any(|s| !s.1.is_finite()) {
        return Err(SegmentationError::Invalid(
            "a seed is not finite".to_owned(),
        ));
    }
    let (vd, td) = match input.decimated {
        Some((v, t)) => (v.to_vec(), t.to_vec()),
        None => decimate(input.vertices, input.tris, params.target_faces)?,
    };
    check_mesh(&vd, &td, "simplified scan")?;
    let face_params = FaceParams {
        eta_convex: params.eta_convex,
        sigma: params.sigma,
        ..FaceParams::default()
    };
    let (flab, diagnostics) = segment_faces(&vd, &td, &seeds, input.arch, &face_params);
    let edges = unique_edges(input.tris);
    let vl = transfer_labels(input.vertices, &edges, &vd, &td, &flab)?;
    let seed_v = seed_vertices(input.vertices, &seeds);
    let labels_transfer = clean_vertex_labels(&vl, &edges, &seed_v, 2);
    let labels = if params.refine {
        let refined = refine_boundaries(
            input.vertices,
            input.tris,
            &labels_transfer,
            &RefineParams::default(),
        );
        clean_vertex_labels(&refined, &edges, &seed_v, 2)
    } else {
        labels_transfer.clone()
    };
    let mut present: Vec<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
    present.sort_unstable();
    present.dedup();
    Ok(Output {
        labels,
        labels_transfer,
        face_labels_decimated: flab,
        decimated_vertices: vd,
        decimated_tris: td,
        diagnostics,
        teeth: present.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_default_and_parse() {
        let p: Params = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(p, Params::default());
        let p: Params = serde_json::from_value(serde_json::json!({ "arch": "upper" })).unwrap();
        assert_eq!(p.arch, Some(Arch::Upper));
        assert!(serde_json::from_value::<Params>(serde_json::json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn run_rejects_bad_input() {
        let v = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let t = [[0u32, 1, 2]];
        let seeds = [(Fdi::new(11).unwrap(), DVec3::ZERO)];
        let input = Input {
            arch: Arch::Lower,
            vertices: &v,
            tris: &t,
            seeds: &seeds,
            decimated: None,
        };
        assert_eq!(
            run(&input, &Params::default()),
            Err(SegmentationError::NoSeeds)
        );
        let bad = [[0u32, 1, 7]];
        let input = Input {
            tris: &bad,
            arch: Arch::Upper,
            ..input
        };
        assert!(matches!(
            run(&input, &Params::default()),
            Err(SegmentationError::Invalid(_))
        ));
    }
}
