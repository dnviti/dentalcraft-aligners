//! Numerical comparison of `dca_algo::setup` with the Python plugin
//! (`ortho_aligner/setup_conservativo.py`, through the golden oracle).
//!
//! `golden/fixtures/<case>/setup/setup.json` is the oracle's output for the
//! six demo cases (`golden/README.md`: no locked or excluded tooth, no drawn
//! line, default scene settings). The Rust stage is fed the oracle's own
//! inputs: the full scan moved to arch coordinates with the worldframe
//! matrix, the refined segmentation labels and the per-tooth geometry of
//! `teeth/teeth.json`. Bounds: target poses within 0.01 mm and 0.05 degrees,
//! the same IPR plan and therapy, the arch line within 0.01 mm.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::setup::{self, Input, Params, ToothInput};
use glam::{DMat3, DMat4, DVec3};
use serde_json::Value;

const TOL_MM: f64 = 0.01;
const TOL_DEG: f64 = 0.05;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../golden/fixtures")
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> DVec3 {
    DVec3::new(num(&v[0]), num(&v[1]), num(&v[2]))
}

/// A 4x4 row-major nested list.
fn mat4(v: &Value) -> DMat4 {
    let mut rows = [[0.0; 4]; 4];
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, x) in row.iter_mut().enumerate() {
            *x = num(&v[r][c]);
        }
    }
    DMat4::from_cols_array_2d(&rows).transpose()
}

fn fdi(key: &str) -> Fdi {
    Fdi::new(key.parse().unwrap()).unwrap()
}

/// Run-length encoded labels `{"n": N, "rle": [label, count, ...]}`.
fn labels(v: &Value) -> Vec<u16> {
    let rle = v["rle"].as_array().unwrap();
    let mut out = Vec::new();
    for pair in rle.chunks(2) {
        let (l, c) = (pair[0].as_u64().unwrap(), pair[1].as_u64().unwrap());
        out.extend(std::iter::repeat_n(l as u16, c as usize));
    }
    assert_eq!(out.len() as u64, v["n"].as_u64().unwrap());
    out
}

fn tooth(f: Fdi, t: &Value) -> ToothInput {
    ToothInput {
        fdi: f,
        centroid: vec3(&t["centroid"]),
        e_md: vec3(&t["e_md"]),
        e_bl: vec3(&t["e_bl"]),
        e_ax: vec3(&t["e_ax"]),
        md_sign: num(&t["md_sign"]),
        reference: vec3(&t["ref"]),
        occ_z: num(&t["occ_z"]),
        margin: t["margin"].as_array().unwrap().iter().map(vec3).collect(),
        contact_m: vec3(&t["contact_m"]),
        contact_d: vec3(&t["contact_d"]),
        width_md: num(&t["width_md"]),
    }
}

fn rotation_deg(a: &DMat4, b: &DMat4) -> f64 {
    let r = DMat3::from_mat4(*a).transpose() * DMat3::from_mat4(*b);
    let tr = r.x_axis.x + r.y_axis.y + r.z_axis.z;
    ((tr - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Largest deviations found, reported with `--nocapture`.
#[derive(Default, Debug)]
struct Worst {
    delta_mm: f64,
    pose_mm: f64,
    deg: f64,
    line_mm: f64,
}

fn compare(case: &str, worst: &mut Worst) {
    let dir = fixtures().join(case);
    let teeth_json = json(&dir.join("teeth/teeth.json"));
    let oracle = json(&dir.join("setup/setup.json"));
    let arch = if teeth_json["arch"] == "UPPER" {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let m_inv = mat4(&json(&dir.join("worldframe/worldframe.json"))["arch_frame"]["M_inv"]);
    let mesh = dc_mesh::dcm::decode(&std::fs::read(dir.join("scan.dcm")).unwrap()).unwrap();
    let vertices: Vec<DVec3> = mesh
        .positions
        .iter()
        .map(|p| m_inv.transform_point3(p.as_dvec3()))
        .collect();
    let labels = labels(&json(&dir.join("segmentation/segmentation.json"))["vertex_labels"]);
    let sequence: Vec<Fdi> = teeth_json["sequence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Fdi::new(v.as_u64().unwrap() as u8).unwrap())
        .collect();
    let teeth: BTreeMap<Fdi, ToothInput> = teeth_json["teeth"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, t)| (fdi(k), tooth(fdi(k), t)))
        .collect();
    let input = Input {
        arch,
        vertices: &vertices,
        labels: &labels,
        sequence: &sequence,
        teeth: &teeth,
        line: None,
        fixed: &[],
        poses: &BTreeMap::new(),
    };
    let out = setup::run(&input, &Params::default()).unwrap();
    let r = &out.result;

    // the same teeth move, to the same poses
    let deltas = oracle["targets_delta"].as_object().unwrap();
    let got: Vec<Fdi> = r.targets.keys().copied().collect();
    let want: Vec<Fdi> = deltas.keys().map(|k| fdi(k)).collect();
    assert_eq!(got, want, "{case}: moved teeth");
    for (k, d) in deltas {
        let f = fdi(k);
        let (a, b) = (r.targets[&f], mat4(d));
        let dt = (a.w_axis - b.w_axis).truncate().length();
        let deg = rotation_deg(&a, &b);
        let rest = mat4(&oracle["rest_frames"][k]);
        let pa = a * rest;
        let pb = mat4(&oracle["targets_pose"][k]);
        let dp = (pa.w_axis - pb.w_axis).truncate().length();
        worst.delta_mm = worst.delta_mm.max(dt);
        worst.pose_mm = worst.pose_mm.max(dp);
        worst.deg = worst.deg.max(deg);
        assert!(dt <= TOL_MM, "{case} {f}: delta translation off by {dt} mm");
        assert!(dp <= TOL_MM, "{case} {f}: pose origin off by {dp} mm");
        assert!(deg <= TOL_DEG, "{case} {f}: rotation off by {deg} deg");
    }

    // the same IPR plan and therapy
    let s = &oracle["setup"];
    let ipr: Vec<(u8, u8, f64)> = r
        .ipr
        .iter()
        .map(|((a, b), mm)| (a.get(), b.get(), *mm))
        .collect();
    let want_ipr: Vec<(u8, u8, f64)> = s["ipr"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["a"].as_u64().unwrap() as u8,
                x["b"].as_u64().unwrap() as u8,
                num(&x["mm"]),
            )
        })
        .collect();
    assert_eq!(ipr.len(), want_ipr.len(), "{case}: IPR contacts");
    for (g, w) in ipr.iter().zip(&want_ipr) {
        assert_eq!((g.0, g.1), (w.0, w.1), "{case}: IPR contact");
        assert!((g.2 - w.2).abs() < 1e-9, "{case}: IPR {g:?} against {w:?}");
    }
    for (got, key) in [
        (r.expansion, "expansion_posterior"),
        (r.expansion_canine, "expansion_canine"),
        (r.advance, "advance"),
    ] {
        assert!((got - num(&s[key])).abs() < 1e-6, "{case}: {key} {got}");
    }
    assert_eq!(
        r.residual > 1e-6,
        s["residual"].get("affollamento").is_some(),
        "{case}: crowding"
    );

    // the line the teeth were aligned on
    let line = s["line"].as_array().unwrap();
    assert_eq!(out.line.len(), line.len(), "{case}: line samples");
    for (p, q) in out.line.iter().zip(line) {
        let d = (*p - vec3(q)).length();
        worst.line_mm = worst.line_mm.max(d);
        assert!(d <= TOL_MM, "{case}: line point off by {d} mm");
    }

    // determinism
    assert_eq!(
        setup::run(&input, &Params::default()).unwrap(),
        out,
        "{case}: rerun"
    );
}

#[test]
fn setup_matches_the_oracle_on_the_demo_cases() {
    let mut cases: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            e.path()
                .join("setup/setup.json")
                .exists()
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 6, "{cases:?}");
    for case in &cases {
        let mut worst = Worst::default();
        compare(case, &mut worst);
        println!("{case}: {worst:?}");
    }
}
