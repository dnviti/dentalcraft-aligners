//! The aligner of the 3D view moved to a stage it was not built at
//! (`dca_algo::aligner_view`) against the Python plugin (`animation._bind`,
//! `_sparse`, `_relative`, `_deform`, `animation.py:540-641`).
//!
//! `tests/fixtures/aligner_view.json` is produced by `gen_aligner_view.py`:
//! a synthetic case (demo crowns of 31-34, a band of gingiva with weights on
//! 31-35, a patch of it on an exact grid where the nearest points tie), a
//! built aligner at the stage of a base, and two stages shown. The plugin's
//! `_bind` runs unchanged on a stand-in case module; the points it measured
//! from are stored as their bits, so both sides bind to the same model.
//!
//! Compared:
//! * the points of the gingiva moved to the stage of the base
//!   (`gingiva::deform` against `reconstruct.deform`, the first part of
//!   `_bind`'s model);
//! * the weights of every vertex on every tooth and the vertices each tooth
//!   moves (`_bind`, `_sparse`);
//! * the motions from the base to each stage (`_relative`);
//! * the moved vertices (`_deform`), from the Rust motions and from the
//!   plugin's.
//!
//! Bound: [`TOL_MM`] on positions, [`TOL_W`] on weights. Measured: every
//! weight and every moved vertex is bit for bit the plugin's, the gingiva
//! within 3.6e-15 mm. The plugin's single-precision `Q @ A` is BLAS sgemm
//! (fused multiply-adds, numpy of system python and of Blender 5.2.2 alike):
//! with plain products instead, 75 of 1231 vertices were one single-precision
//! step (1.9e-6 mm) off.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::aligner_view::{self as av, Model};
use dca_algo::gingiva::{self, GumWeights};
use glam::{DMat4, DVec3, Vec3};
use serde_json::Value;

const TOL_MM: f64 = 1e-6;
const TOL_W: f64 = 1e-6;
/// The motions: the plugin's `D @ rigid_inverse(B)` against glam's, entry
/// by entry (rounding of the products only).
const TOL_REL: f64 = 1e-12;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aligner_view.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn double(v: &Value) -> f64 {
    f64::from_bits(v.as_u64().unwrap())
}

fn dvec3s(v: &Value) -> Vec<DVec3> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_array().unwrap();
            DVec3::new(double(&p[0]), double(&p[1]), double(&p[2]))
        })
        .collect()
}

fn single(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn rows(v: &Value) -> Vec<Vec<f32>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().iter().map(single).collect())
        .collect()
}

fn vec3s(v: &Value) -> Vec<Vec3> {
    rows(v)
        .iter()
        .map(|r| Vec3::new(r[0], r[1], r[2]))
        .collect()
}

fn fdi(s: &str) -> Fdi {
    Fdi::new(s.parse().unwrap()).unwrap()
}

/// A row-major 4x4 matrix of bits.
fn mat(v: &Value) -> DMat4 {
    let a: Vec<f64> = v
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().unwrap().iter().map(double))
        .collect();
    DMat4::from_cols_array(&a.try_into().unwrap()).transpose()
}

fn mats(v: &Value) -> BTreeMap<Fdi, DMat4> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(f, m)| (fdi(f), mat(m)))
        .collect()
}

fn max_dist(a: &[Vec3], b: &[Vec3]) -> (f64, usize) {
    assert_eq!(a.len(), b.len());
    let worst = a
        .iter()
        .zip(b)
        .map(|(p, q)| (p.as_dvec3() - q.as_dvec3()).length())
        .fold(0.0, f64::max);
    let differ = a.iter().zip(b).filter(|(p, q)| p != q).count();
    (worst, differ)
}

#[test]
fn the_aligner_follows_the_model_as_the_plugins() {
    let fx = fixture();
    let fdis: Vec<Fdi> = fx["fdis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| fdi(f.as_str().unwrap()))
        .collect();
    let nf = fdis.len();
    let points = dvec3s(&fx["points"]);
    let weights: Vec<f32> = rows(&fx["point_weights"]).concat();
    let vertices = vec3s(&fx["vertices"]);
    let base = mats(&fx["base"]);

    // the gingiva at the stage of the base, the first points of the model
    let n_gum = fx["gum_points"].as_u64().unwrap() as usize;
    let gum = GumWeights {
        fdis: fdis.clone(),
        w: rows(&fx["gum_weights"]).concat(),
    };
    let moved_gum = gingiva::deform(&dvec3s(&fx["gum_rest"]), &gingiva::deform_prep(&gum), &base);
    let gum_err = moved_gum
        .iter()
        .zip(&points[..n_gum])
        .map(|(a, b)| (*a - *b).length())
        .fold(0.0, f64::max);
    eprintln!("gingiva at the base: max {gum_err:.3e} mm over {n_gum} points");
    assert!(gum_err <= TOL_MM, "gingiva at the base: {gum_err:e} mm");

    // _bind and _sparse
    let model = Model {
        fdis: &fdis,
        points: &points,
        weights: &weights,
    };
    let b = av::bind(&vertices, &model).unwrap();
    assert_eq!(b.rest(), &vertices[..]);
    let expect = rows(&fx["weights"]);
    let sparse = fx["sparse_rows"].as_object().unwrap();
    let mut w_err = 0.0f64;
    let mut w_differ = 0usize;
    for (j, f) in fdis.iter().enumerate() {
        let moved: Vec<u64> = sparse
            .get(&f.to_string())
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i.as_u64().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        let got: Vec<u64> = (0..vertices.len() as u32)
            .filter(|&i| b.weight(*f, i) != 0.0)
            .map(u64::from)
            .collect();
        assert_eq!(got, moved, "{f}: the vertices it moves");
        for (i, row) in expect.iter().enumerate() {
            assert_eq!(row.len(), nf);
            let want = if row[j] > av::WEIGHT_EPS { row[j] } else { 0.0 };
            let have = b.weight(*f, i as u32);
            w_err = w_err.max(f64::from((have - want).abs()));
            w_differ += usize::from(have != want);
        }
    }
    let (t0, t1) = (
        fx["tie_vertices"][0].as_u64().unwrap() as u32,
        fx["tie_vertices"][1].as_u64().unwrap() as u32,
    );
    assert!(
        (t0..t1).all(|i| b.weight(fdis[0], i) > 0.1),
        "the tie vertices bind to 31"
    );
    eprintln!(
        "weights: max {w_err:.3e}, {w_differ} of {} differ",
        vertices.len() * nf
    );
    assert!(w_err <= TOL_W, "weights: {w_err:e}");

    for st in fx["stages"].as_array().unwrap() {
        let name = st["name"].as_str().unwrap();
        // _relative
        let rel = av::relative(&mats(&st["deltas"]), &base);
        let want_rel = mats(&st["relative"]);
        assert_eq!(
            rel.keys().collect::<Vec<_>>(),
            want_rel.keys().collect::<Vec<_>>(),
            "{name}: the teeth of the motions"
        );
        let rel_err = rel
            .iter()
            .flat_map(|(f, m)| {
                let w = want_rel[f].to_cols_array();
                m.to_cols_array()
                    .into_iter()
                    .zip(w)
                    .map(|(a, b)| (a - b).abs())
            })
            .fold(0.0, f64::max);
        assert!(rel_err <= TOL_REL, "{name}: motions {rel_err:e}");
        // _deform, from the Rust motions and from the plugin's
        let want = vec3s(&st["vertices"]);
        let (err, differ) = max_dist(&av::deform(&b, &rel), &want);
        let (err_p, differ_p) = max_dist(&av::deform(&b, &want_rel), &want);
        eprintln!(
            "{name}: motions max {rel_err:.3e}; vertices max {err:.3e} mm ({differ} of {} differ), \
             from the plugin's motions {err_p:.3e} mm ({differ_p} differ)",
            want.len()
        );
        assert!(err <= TOL_MM, "{name}: vertices {err:e} mm");
        assert!(
            err_p <= TOL_MM,
            "{name}: vertices (plugin's motions) {err_p:e} mm"
        );
    }
}
