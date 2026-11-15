//! The aligner shown in the 3D view at a stage it was not built at (the
//! plugin's `animation._bind`, `_sparse`, `_deform` and `_relative`,
//! `animation.py:540-641`).
//!
//! An aligner built for the view (a base) belongs to one stage. The other
//! stages show the nearest base moved with the model: every vertex of the
//! base blends the motions of the nearest points of the model at the stage of
//! the base, the teeth moving rigidly and the gingiva with its own blend
//! weights, so a vertex between two teeth follows both ([`bind`]); the
//! motion of each tooth is the one from the stage of the base to the stage
//! shown ([`relative`]), applied to the vertices with their weights
//! ([`deform`]).
//!
//! Precision is the plugin's: the neighbours and their distances in double
//! precision, the weights and the moved vertices in single precision (the
//! precision of the mesh), in the plugin's order of operations.

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::knn::KnnIndex;
use glam::{DMat4, DVec3, Vec3};
use rayon::prelude::*;

use crate::frame::rigid_inverse;

/// Points of the model whose motions a vertex blends (`_bind`: `k = 8`).
pub const NEIGHBOURS: usize = 8;
/// Added to the squared distance of a neighbour before its weight is taken
/// (mm²; `_bind`: `1 / (d² + 0.04)`).
pub const SOFTEN_MM2: f64 = 0.04;
/// A weight at or below this moves nothing (`_sparse`: `W > 1e-5`).
pub const WEIGHT_EPS: f32 = 1e-5;
/// A motion this close to the identity moves nothing (`_deform`).
pub const STILL: f64 = 1e-12;
/// Candidates asked of the single-precision tree before the exact distances
/// pick the nearest [`NEIGHBOURS`]: an order the rounding to single
/// precision changes stays among them.
const CANDIDATES: usize = 2 * NEIGHBOURS;
/// Vertices per task of the parallel search.
const CHUNK: usize = 256;

/// The model at the stage of a base: its points and the share of every tooth
/// in the motion of each (`_bind`: the gingiva with its own weights, every
/// point of a tooth all to that tooth).
#[derive(Debug, Clone, Copy)]
pub struct Model<'a> {
    /// The teeth, in the order of the columns of `weights`.
    pub fdis: &'a [Fdi],
    /// The points (case coordinates).
    pub points: &'a [DVec3],
    /// `points.len() × fdis.len()`, row-major: the share of each tooth in
    /// the motion of each point (the rows sum to at most 1: the rest stays).
    pub weights: &'a [f32],
}

/// What [`bind`] could not do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindError {
    /// The model has no point, or its weights are not one row per point.
    #[error("the model has no points or its weights do not fit them")]
    Model,
    /// A point of the model or a vertex is not finite.
    #[error("{0}")]
    NotFinite(String),
}

/// One column of a [`Binding`] (`_sparse`): the tooth, the vertices it
/// moves, their weights and their rest positions.
type Column = (Fdi, Vec<u32>, Vec<f32>, Vec<Vec3>);

/// A base bound to the model it was built on (`_binding`): its vertices and,
/// for every tooth that moves some of them, the vertices it moves and their
/// weights (`_sparse`).
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    rest: Vec<Vec3>,
    cols: Vec<Column>,
}

impl Binding {
    /// The vertices of the base.
    pub fn rest(&self) -> &[Vec3] {
        &self.rest
    }

    /// The weight of tooth `fdi` on vertex `i` (`0` when it does not move it).
    pub fn weight(&self, fdi: Fdi, i: u32) -> f32 {
        self.cols
            .iter()
            .find(|(f, ..)| *f == fdi)
            .and_then(|(_, rows, w, _)| rows.binary_search(&i).ok().map(|k| w[k]))
            .unwrap_or(0.0)
    }
}

/// The squared distance and its order key, in double precision.
fn dist2(p: DVec3, q: DVec3) -> f64 {
    let d = p - q;
    d.x * d.x + d.y * d.y + d.z * d.z
}

/// Binds the vertices of a base (`vertices`, single precision as the mesh)
/// to the model at the stage the base was built at (`_bind`, then
/// `_sparse`). Each vertex takes the [`NEIGHBOURS`] nearest points of the
/// model (nearest first, ties to the smallest index), weighs them by
/// `1 / (d² + 0.04)` normalised, and blends their rows.
///
/// # Errors
/// [`BindError`] when the model is empty or its rows do not fit, or a point
/// is not finite.
pub fn bind(vertices: &[Vec3], model: &Model<'_>) -> Result<Binding, BindError> {
    let nf = model.fdis.len();
    if model.points.is_empty() || model.weights.len() != model.points.len() * nf {
        return Err(BindError::Model);
    }
    let rounded: Vec<Vec3> = model.points.iter().map(|p| p.as_vec3()).collect();
    let tree = KnnIndex::build(&rounded).map_err(|e| BindError::NotFinite(e.to_string()))?;
    if let Some(i) = vertices.iter().position(|v| !v.is_finite()) {
        return Err(BindError::NotFinite(format!("vertex {i} is not finite")));
    }
    let k = NEIGHBOURS.min(model.points.len());
    // the dense weights of every vertex (the plugin's `W`, single precision)
    let mut w = vec![0.0f32; vertices.len() * nf];
    w.par_chunks_mut(nf.max(1) * CHUNK)
        .enumerate()
        .for_each(|(c, out)| {
            let first = c * CHUNK;
            for (r, row) in out.chunks_mut(nf.max(1)).enumerate() {
                let q = vertices[first + r].as_dvec3();
                // the candidates of the rounded tree, ordered by their exact
                // distance (ties to the smallest index)
                let mut near: Vec<(f64, u32)> = tree
                    .nearest_k(vertices[first + r], CANDIDATES)
                    .into_iter()
                    .map(|n| (dist2(model.points[n.index as usize], q), n.index))
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                near.truncate(k);
                let mut ws = [0.0f64; NEIGHBOURS];
                for (j, (d2, _)) in near.iter().enumerate() {
                    let d = d2.sqrt();
                    ws[j] = 1.0 / (d * d + SOFTEN_MM2);
                }
                // numpy's sum of a row of eight: pairwise
                let total = if k == NEIGHBOURS {
                    ((ws[0] + ws[1]) + (ws[2] + ws[3])) + ((ws[4] + ws[5]) + (ws[6] + ws[7]))
                } else {
                    ws[..k].iter().sum()
                };
                for (j, (_, idx)) in near.iter().enumerate() {
                    let wj = (ws[j] / total) as f32;
                    let src = &model.weights[*idx as usize * nf..(*idx as usize + 1) * nf];
                    for (o, s) in row.iter_mut().zip(src) {
                        *o += wj * *s;
                    }
                }
            }
        });
    let rest = vertices.to_vec();
    let cols = model
        .fdis
        .iter()
        .enumerate()
        .filter_map(|(j, &f)| {
            let rows: Vec<u32> = (0..vertices.len())
                .filter(|&i| w[i * nf + j] > WEIGHT_EPS)
                .map(|i| i as u32)
                .collect();
            if rows.is_empty() {
                return None;
            }
            let ws = rows.iter().map(|&i| w[i as usize * nf + j]).collect();
            let at = rows.iter().map(|&i| rest[i as usize]).collect();
            Some((f, rows, ws, at))
        })
        .collect();
    Ok(Binding { rest, cols })
}

/// The motion of every tooth from the stage of the base to the stage shown
/// (`_relative`: `D · base⁻¹`, the identity for a tooth the base does not
/// move), for the teeth of `stage`.
pub fn relative(stage: &BTreeMap<Fdi, DMat4>, base: &BTreeMap<Fdi, DMat4>) -> BTreeMap<Fdi, DMat4> {
    stage
        .iter()
        .map(|(f, d)| {
            let b = base.get(f).map_or(DMat4::IDENTITY, rigid_inverse);
            (*f, *d * b)
        })
        .collect()
}

/// The vertices of the bound base moved by `rel` ([`relative`]), in single
/// precision (`_deform`): every tooth adds `w · ((R − I) q + t)` to the
/// vertices it moves; a tooth without a motion, or whose motion is the
/// identity within [`STILL`], moves nothing.
pub fn deform(b: &Binding, rel: &BTreeMap<Fdi, DMat4>) -> Vec<Vec3> {
    let mut v = b.rest.clone();
    for (f, rows, ws, at) in &b.cols {
        let Some(d) = rel.get(f) else {
            continue;
        };
        let still = (*d - DMat4::IDENTITY)
            .to_cols_array()
            .iter()
            .all(|x| x.abs() < STILL);
        if still {
            continue;
        }
        // A = (R − I)ᵀ and t in single precision: q · A is (R − I) q, summed
        // as numpy's single-precision `Q @ A` does (BLAS sgemm, in Blender
        // 5.2.2 too): the first product, then two fused multiply-adds
        let a: [[f32; 3]; 3] = std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                let m = d.col(r)[c] - if r == c { 1.0 } else { 0.0 };
                m as f32
            })
        });
        let t = d.w_axis.truncate().as_vec3();
        for ((&i, &w), q) in rows.iter().zip(ws).zip(at) {
            let m = |c: usize| q.z.mul_add(a[2][c], q.y.mul_add(a[1][c], q.x * a[0][c]));
            let step = Vec3::new(w * (m(0) + t.x), w * (m(1) + t.y), w * (m(2) + t.z));
            v[i as usize] += step;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// Two teeth side by side along x and a strip of gingiva under them,
    /// half on each.
    fn model() -> (Vec<Fdi>, Vec<DVec3>, Vec<f32>) {
        let fdis = vec![fdi(11), fdi(21)];
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for (j, x0) in [(0usize, -6.0), (1, 2.0)] {
            for i in 0..5 {
                for k in 0..5 {
                    points.push(DVec3::new(x0 + i as f64, k as f64, 4.0));
                    let mut row = [0.0f32; 2];
                    row[j] = 1.0;
                    weights.extend(row);
                }
            }
        }
        for i in 0..17 {
            points.push(DVec3::new(-8.0 + i as f64, 2.0, 0.0));
            let s = (i as f32 / 16.0).clamp(0.0, 1.0);
            weights.extend([0.5 * (1.0 - s), 0.5 * s]);
        }
        (fdis, points, weights)
    }

    #[test]
    fn a_vertex_over_a_tooth_moves_with_it_and_one_between_two_follows_both() {
        let (fdis, points, weights) = model();
        let m = Model {
            fdis: &fdis,
            points: &points,
            weights: &weights,
        };
        let vertices = [
            Vec3::new(-4.0, 2.0, 5.0), // over 11
            Vec3::new(4.0, 2.0, 5.0),  // over 21
            Vec3::new(0.0, 2.0, 5.0),  // between
        ];
        let b = bind(&vertices, &m).unwrap();
        assert!(b.weight(fdi(11), 0) > 0.999 && b.weight(fdi(21), 0) < 1e-3);
        assert!(b.weight(fdi(21), 1) > 0.999);
        // the eight nearest are four points of each tooth and two more at the
        // same distance of each: the tie goes to the smaller index (11)
        let (w11, w21) = (b.weight(fdi(11), 2), b.weight(fdi(21), 2));
        assert!((w11 + w21 - 1.0).abs() < 1e-6, "{w11} {w21}");
        assert!(w11 > w21 && w21 > 0.35, "{w11} {w21}");
        // 21 moves 1 mm along x from the base to the stage shown
        let base: BTreeMap<Fdi, DMat4> = [
            (fdi(11), DMat4::IDENTITY),
            (fdi(21), DMat4::from_translation(DVec3::new(0.5, 0.0, 0.0))),
        ]
        .into();
        let stage: BTreeMap<Fdi, DMat4> = [
            (fdi(11), DMat4::IDENTITY),
            (fdi(21), DMat4::from_translation(DVec3::new(1.5, 0.0, 0.0))),
        ]
        .into();
        let rel = relative(&stage, &base);
        assert!((rel[&fdi(21)].w_axis.x - 1.0).abs() < 1e-12);
        let v = deform(&b, &rel);
        assert!((v[0] - vertices[0]).length() < 1e-5);
        assert!((v[1] - vertices[1] - Vec3::X).length() < 1e-3);
        assert!(((v[2] - vertices[2]).x - w21).abs() < 1e-5);
        // the stage of the base gives the base back
        assert_eq!(deform(&b, &relative(&base, &base)), vertices.to_vec());
    }

    #[test]
    fn a_turn_moves_the_vertices_as_the_tooth() {
        let (fdis, points, weights) = model();
        let m = Model {
            fdis: &fdis,
            points: &points,
            weights: &weights,
        };
        let vertices = [Vec3::new(-4.0, 2.0, 5.0)];
        let b = bind(&vertices, &m).unwrap();
        let turn = DMat4::from_rotation_translation(
            DQuat::from_rotation_z(0.1),
            DVec3::new(0.2, -0.1, 0.0),
        );
        let stage: BTreeMap<Fdi, DMat4> = [(fdi(11), turn), (fdi(21), DMat4::IDENTITY)].into();
        let v = deform(&b, &relative(&stage, &BTreeMap::new()));
        let expect = turn.transform_point3(vertices[0].as_dvec3()).as_vec3();
        let w = b.weight(fdi(11), 0);
        let blended = vertices[0] + w * (expect - vertices[0]);
        assert!((v[0] - blended).length() < 1e-5, "{:?} {:?}", v[0], blended);
    }

    #[test]
    fn a_model_without_points_or_with_rows_that_do_not_fit_is_refused() {
        let fdis = [fdi(11)];
        let m = Model {
            fdis: &fdis,
            points: &[],
            weights: &[],
        };
        assert_eq!(bind(&[Vec3::ZERO], &m), Err(BindError::Model));
        let points = [DVec3::ZERO, DVec3::X];
        let m = Model {
            fdis: &fdis,
            points: &points,
            weights: &[1.0],
        };
        assert_eq!(bind(&[Vec3::ZERO], &m), Err(BindError::Model));
        let m = Model {
            fdis: &fdis,
            points: &points,
            weights: &[1.0, 1.0],
        };
        assert!(matches!(
            bind(&[Vec3::NAN], &m),
            Err(BindError::NotFinite(_))
        ));
    }
}
