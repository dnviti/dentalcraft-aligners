//! The gingiva of `dca_algo::gingiva` against the Python plugin
//! (`reconstruct.gum_surface`, `gum_weights`, `deform`; `gingiva.deform`).
//!
//! `tests/fixtures/gingiva.json` is produced by `gen_gingiva.py` from the
//! plugin's own modules on two golden demo cases (the scan in arch coordinates,
//! the oracle's segmentation labels and teeth) with a test motion of three
//! teeth. Compared: the counts and flags of the gum surface, rest and deformed
//! positions (within 1e-4 mm), weights (within 1e-4), on every vertex through
//! checksums and on an evenly spaced sample one by one.

mod golden_case;

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::gingiva::{self, Params};
use glam::{DMat4, DVec3};
use golden_case::{Case, mat4};
use serde_json::Value;

const TOL_MM: f64 = 1e-4;
const TOL_W: f64 = 1e-4;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gingiva.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn points(v: &Value) -> Vec<DVec3> {
    nums(v)
        .chunks(3)
        .map(|c| DVec3::new(c[0], c[1], c[2]))
        .collect()
}

fn usizes(v: &Value) -> Vec<usize> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as usize)
        .collect()
}

fn fdi(v: &Value) -> Fdi {
    Fdi::new(v.as_u64().unwrap() as u8).unwrap()
}

fn sum(p: &[DVec3]) -> DVec3 {
    p.iter().copied().sum()
}

fn close(a: DVec3, b: DVec3, tol: f64, what: &str) {
    assert!((a - b).abs().max_element() < tol, "{what}: {a:?} vs {b:?}");
}

fn weights_row(w: &Value, nf: usize) -> Vec<f64> {
    let mut row = vec![0.0; nf];
    for (k, x) in w.as_object().unwrap() {
        row[k.parse::<usize>().unwrap()] = x.as_f64().unwrap();
    }
    row
}

#[test]
fn the_gingiva_and_its_deformation_match_the_plugin() {
    let fx = fixture();
    for c in fx["cases"].as_array().unwrap() {
        let name = c["case"].as_str().unwrap();
        let case = Case::load(name);
        let fdis: Vec<Fdi> = c["fdis"].as_array().unwrap().iter().map(fdi).collect();
        let deltas: BTreeMap<Fdi, DMat4> = c["motion"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, m)| (Fdi::new(k.parse().unwrap()).unwrap(), mat4(m)))
            .collect();

        // the gum surface
        let t0 = std::time::Instant::now();
        let g = gingiva::gum_surface(&case.vertices, &case.tris, &case.labels, &Params::default())
            .unwrap();
        let t1 = t0.elapsed().as_secs_f64();
        let want = &c["gum"];
        let count = |m: &[bool]| m.iter().filter(|&&x| x).count() as u64;
        assert_eq!(
            g.vertices.len() as u64,
            want["n_vertices"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            g.tris.len() as u64,
            want["n_faces"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            count(&g.margin),
            want["n_margin"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            count(&g.membrane),
            want["n_membrane"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(count(&g.col), want["n_col"].as_u64().unwrap(), "{name}");
        let mut hist: BTreeMap<String, u64> = BTreeMap::new();
        for &t in &g.tooth {
            *hist.entry(t.to_string()).or_default() += 1;
        }
        for (k, v) in want["tooth_hist"].as_object().unwrap() {
            assert_eq!(hist.get(k).copied(), v.as_u64(), "{name} tooth {k}");
        }
        let n = g.vertices.len() as f64;
        close(
            sum(&g.vertices) / n,
            DVec3::from_slice(&nums(&want["rest_sum"])) / n,
            TOL_MM,
            "rest mean",
        );
        let idx = usizes(&want["sample_idx"]);
        let rest = points(&want["sample_rest"]);
        let tooth = usizes(&want["sample_tooth"]);
        let flags = usizes(&want["sample_flags"]);
        for (k, &i) in idx.iter().enumerate() {
            close(g.vertices[i], rest[k], TOL_MM, &format!("{name} rest {i}"));
            assert_eq!(g.tooth[i] as usize, tooth[k], "{name} tooth {i}");
            let f = usize::from(g.margin[i])
                + 2 * usize::from(g.membrane[i])
                + 4 * usize::from(g.col[i]);
            assert_eq!(f, flags[k], "{name} flags {i}");
        }

        // the weights
        let t0 = std::time::Instant::now();
        let w = gingiva::gum_weights(&g, &fdis, &Params::default());
        let t2 = t0.elapsed().as_secs_f64();
        let nf = fdis.len();
        let col_sum: Vec<f64> = (0..nf)
            .map(|j| (0..g.vertices.len()).map(|i| f64::from(w.get(i, j))).sum())
            .collect();
        for (j, (a, b)) in col_sum
            .iter()
            .zip(nums(&want["weights_col_sum"]))
            .enumerate()
        {
            assert!(
                (a - b).abs() < 1e-3 * b.max(1.0),
                "{name} weight sum of column {j}: {a} vs {b}"
            );
        }
        let mut worst_w = 0.0f64;
        for (k, &i) in idx.iter().enumerate() {
            let row = weights_row(&want["sample_weights"][k], nf);
            for (j, x) in row.iter().enumerate() {
                worst_w = worst_w.max((f64::from(w.get(i, j)) - x).abs());
            }
        }
        assert!(worst_w < TOL_W, "{name}: weights differ by {worst_w}");

        // the deformation of the test motion
        let prep = gingiva::deform_prep(&w);
        let vd = gingiva::deform(&g.vertices, &prep, &deltas);
        close(
            sum(&vd) / n,
            DVec3::from_slice(&nums(&want["deformed_sum"])) / n,
            TOL_MM,
            "deformed mean",
        );
        let dwant = points(&want["sample_deformed"]);
        let mut worst_d = 0.0f64;
        for (k, &i) in idx.iter().enumerate() {
            worst_d = worst_d.max((vd[i] - dwant[k]).abs().max_element());
        }
        assert!(
            worst_d < TOL_MM,
            "{name}: deformed gingiva differs by {worst_d} mm"
        );

        // the scan deformation of gingiva.py
        let blend = gingiva::scan_blend(&case.vertices, &case.tris, &case.labels).unwrap();
        let s = &c["scan"];
        let sf: Vec<Fdi> = s["fdis"].as_array().unwrap().iter().map(fdi).collect();
        assert_eq!(blend.fdis, sf, "{name}");
        assert_eq!(blend.gum_idx.len() as u64, s["n_gum"].as_u64().unwrap());
        let snf = sf.len();
        let gi = usizes(&s["sample_gum"]);
        let mut worst_a = 0.0f64;
        for (k, &i) in gi.iter().enumerate() {
            let row = weights_row(&s["sample_weights"][k], snf);
            for (j, x) in row.iter().enumerate() {
                worst_a = worst_a.max((f64::from(blend.a[i * snf + j]) - x).abs());
            }
        }
        assert!(worst_a < TOL_W, "{name}: scan weights differ by {worst_a}");
        let vs = gingiva::deform_scan(&case.vertices, &blend, &deltas);
        let ns = vs.len() as f64;
        close(
            sum(&vs) / ns,
            DVec3::from_slice(&nums(&s["deformed_sum"])) / ns,
            TOL_MM,
            "scan mean",
        );
        let swant = points(&s["sample_deformed"]);
        let mut worst_s = 0.0f64;
        for (k, &i) in gi.iter().enumerate() {
            worst_s = worst_s.max((vs[blend.gum_idx[i]] - swant[k]).abs().max_element());
        }
        assert!(
            worst_s < TOL_MM,
            "{name}: deformed scan differs by {worst_s} mm"
        );
        // the deformation moves what moves: a moved tooth's margin follows it
        let moved = (0..g.vertices.len())
            .filter(|&i| (vd[i] - g.vertices[i]).length() > 1e-3)
            .count();
        assert!(moved > 1000, "{name}: {moved}");
        eprintln!(
            "{name}: gum surface {t1:.2} s, weights {t2:.2} s; worst weight {worst_w:.1e}, \
             deformed {worst_d:.1e} mm, scan weight {worst_a:.1e}, scan {worst_s:.1e} mm"
        );
    }
}
