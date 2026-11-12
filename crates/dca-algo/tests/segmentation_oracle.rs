//! Golden test of the segmentation against the Blender plugin (oracle
//! fixtures in `golden/fixtures/<case>/segmentation/segmentation.json`).
//!
//! Fed the oracle's simplified mesh (`decimated.dcm`, Blender's Decimate) and
//! its seeds, the face labels, the transferred labels and the final vertex
//! labels must agree with the oracle on at least 99 % of the faces / vertices
//! (PLAN section 5). All six cases, and the full pipeline with our own
//! simplification against the ground truth, run by default; use
//! `cargo test --release -p dca-algo --test segmentation_oracle -- --nocapture`
//! to see the per-case numbers.

use std::path::{Path, PathBuf};
use std::time::Instant;

use dc_case::Fdi;
use dc_geom::metrics::label_agreement;
use dca_algo::anatomy::Arch;
use dca_algo::frame::rigid_inverse;
use dca_algo::segmentation::{self, Input, Params};
use glam::{DMat4, DVec3};
use serde_json::Value;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../golden/fixtures");
const CASES: [&str; 6] = [
    "demo_lower_mild",
    "demo_lower_moderate",
    "demo_lower_severe",
    "demo_upper_mild",
    "demo_upper_moderate",
    "demo_upper_severe",
];

fn case(name: &str) -> PathBuf {
    Path::new(GOLDEN).join(name)
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Run-length encoded labels `{"n": N, "rle": [label, count, ...]}`.
fn unpack(v: &Value) -> Vec<u16> {
    let rle = v["rle"].as_array().unwrap();
    let mut out = Vec::with_capacity(v["n"].as_u64().unwrap() as usize);
    for pair in rle.chunks(2) {
        let (l, n) = (pair[0].as_u64().unwrap(), pair[1].as_u64().unwrap());
        out.extend(std::iter::repeat_n(l as u16, n as usize));
    }
    assert_eq!(out.len() as u64, v["n"].as_u64().unwrap());
    out
}

struct Case {
    arch: Arch,
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    truth: Vec<u16>,
    dec_vertices: Vec<DVec3>,
    dec_tris: Vec<[u32; 3]>,
    seeds: Vec<(Fdi, DVec3)>,
    oracle: Value,
}

fn load(name: &str) -> Case {
    let dir = case(name);
    let meta = json(&dir.join("meta.json"));
    let oracle = json(&dir.join("segmentation/segmentation.json"));
    let scan = dc_mesh::dcm::decode(&std::fs::read(dir.join("scan.dcm")).unwrap()).unwrap();
    let dec = dc_mesh::dcm::decode(&std::fs::read(dir.join("decimated.dcm")).unwrap()).unwrap();
    // arch frame M (rows in the JSON): world -> arch with its rigid inverse
    let rows: Vec<Vec<f64>> = serde_json::from_value(meta["frame_gt"].clone()).unwrap();
    let mut cols = [[0.0; 4]; 4];
    for (r, row) in rows.iter().enumerate() {
        for (c, &x) in row.iter().enumerate() {
            cols[c][r] = x;
        }
    }
    let to_arch = rigid_inverse(&DMat4::from_cols_array_2d(&cols));
    let vertices = scan
        .positions
        .iter()
        .map(|p| to_arch.transform_point3(p.as_dvec3()))
        .collect();
    let mut seeds: Vec<(Fdi, DVec3)> = oracle["inputs"]["seeds_arch"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(f, p)| {
            let p: [f64; 3] = serde_json::from_value(p.clone()).unwrap();
            (Fdi::new(f.parse().unwrap()).unwrap(), DVec3::from(p))
        })
        .collect();
    seeds.sort_by_key(|s| s.0);
    let arch = if meta["arch"] == "UPPER" {
        Arch::Upper
    } else {
        Arch::Lower
    };
    Case {
        arch,
        vertices,
        tris: scan.tris.clone(),
        truth: scan.vertex_labels.clone().unwrap(),
        dec_vertices: dec.positions.iter().map(|p| p.as_dvec3()).collect(),
        dec_tris: dec.tris.clone(),
        seeds,
        oracle,
    }
}

/// Agreement with the oracle of the face labels, the transferred labels and
/// the final labels.
fn check_against_oracle(name: &str) -> [f64; 3] {
    let c = load(name);
    let t0 = Instant::now();
    let out = segmentation::run(
        &Input {
            arch: c.arch,
            vertices: &c.vertices,
            tris: &c.tris,
            seeds: &c.seeds,
            decimated: Some((&c.dec_vertices, &c.dec_tris)),
        },
        &Params::default(),
    )
    .unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let faces = label_agreement(
        &out.face_labels_decimated,
        &unpack(&c.oracle["face_labels_decimated"]),
    )
    .unwrap();
    let transfer = label_agreement(
        &out.labels_transfer,
        &unpack(&c.oracle["vertex_labels_transfer"]),
    )
    .unwrap();
    let fin = label_agreement(&out.labels, &unpack(&c.oracle["vertex_labels"])).unwrap();
    let gt = label_agreement(&out.labels, &c.truth).unwrap();
    println!(
        "{name}: vs oracle faces {:.4} transfer {:.4} final {:.4}; vs truth {:.4} \
         (oracle {:.4}); {} CG iterations (oracle {}); {secs:.2} s",
        faces,
        transfer,
        fin,
        gt,
        c.oracle["label_agreement_vs_ground_truth"]
            .as_f64()
            .unwrap(),
        out.diagnostics.cg_iterations,
        c.oracle["diagnostics"]["cg_iterations"],
    );
    [faces, transfer, fin]
}

fn assert_agrees(name: &str) {
    let a = check_against_oracle(name);
    for (what, x) in ["face labels", "transferred labels", "final labels"]
        .iter()
        .zip(a)
    {
        assert!(x >= 0.99, "{name}: {what} agree with the oracle at {x:.4}");
    }
}

#[test]
fn segmentation_matches_the_oracle_on_one_case() {
    assert_agrees("demo_lower_mild");
}

// CI runs `cargo test --workspace --release`. Measured in release: 1.3-2.7 s per case,
// about 10 s for the six cases (about 20 s in debug).
#[test]
fn segmentation_matches_the_oracle_on_every_case() {
    for name in CASES {
        assert_agrees(name);
    }
}

// Full pipeline with our own simplification. Measured in release: 1.9-2.6 s per case,
// about 12 s for the six cases; run in parallel with the test above, about 27 s together.
#[test]
fn full_pipeline_agrees_with_the_ground_truth() {
    for name in CASES {
        let c = load(name);
        let input = Input {
            arch: c.arch,
            vertices: &c.vertices,
            tris: &c.tris,
            seeds: &c.seeds,
            decimated: None,
        };
        let t0 = Instant::now();
        let out = segmentation::run(&input, &Params::default()).unwrap();
        let secs = t0.elapsed().as_secs_f64();
        let gt = label_agreement(&out.labels, &c.truth).unwrap();
        let oracle_gt = c.oracle["label_agreement_vs_ground_truth"]
            .as_f64()
            .unwrap();
        let vs_oracle = label_agreement(&out.labels, &unpack(&c.oracle["vertex_labels"])).unwrap();
        println!(
            "{name}: own QEM ({} faces) vs truth {gt:.4} (oracle {oracle_gt:.4}), vs oracle {vs_oracle:.4}; \
             {} teeth; {secs:.2} s",
            out.decimated_tris.len(),
            out.teeth
        );
        assert!(gt >= oracle_gt - 0.005, "{name}: {gt:.4} vs truth");
        // deterministic: the same labels on a second run
        let again = segmentation::run(&input, &Params::default()).unwrap();
        assert_eq!(again.labels, out.labels, "{name}: not deterministic");
    }
}
