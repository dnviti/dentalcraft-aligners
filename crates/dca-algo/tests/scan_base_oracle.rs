//! The base closing of an open scan (`dca_algo::scan_base`) against the
//! plugin's `aligner.make_base`, run in Blender on golden demo scans opened at
//! the bottom (`tests/fixtures/make_base.json`, from `gen_make_base.py`).
//!
//! The demo scans are closed; both sides open them the same way (the faces
//! within `cut_mm` of the lowest point along the occlusal axis go). Compared:
//! the boundary edges closed, the counts of points and faces, the plane under
//! the scan, the points of the base (their sum and extremes: bmesh numbers
//! them in its own order), the enclosed volume and the area (the flat cap is
//! triangulated differently, over the same area), and the old points, which
//! never move. Blender computes in float32: the points agree within 1e-4 mm.

mod golden_case;

use std::path::PathBuf;

use dc_mesh::TriMesh;
use dca_algo::scan_base::{self, signed_volume};
use glam::DVec3;
use serde_json::Value;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/make_base.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn v3(v: &Value) -> DVec3 {
    DVec3::new(f(&v[0]), f(&v[1]), f(&v[2]))
}

/// The scan without its faces lower than `cut` over its lowest point along
/// `z`, the points no face uses dropped (the others keep their order); the
/// number of faces removed.
fn opened(scan: &TriMesh, z: DVec3, cut: f64) -> (TriMesh, usize) {
    let h: Vec<f64> = scan.positions.iter().map(|p| p.as_dvec3().dot(z)).collect();
    let low = h.iter().copied().fold(f64::INFINITY, f64::min) + cut;
    let kept: Vec<[u32; 3]> = scan
        .tris
        .iter()
        .filter(|t| !t.iter().all(|&i| h[i as usize] < low))
        .copied()
        .collect();
    let removed = scan.tris.len() - kept.len();
    let mut index = vec![u32::MAX; scan.positions.len()];
    for t in &kept {
        for &i in t {
            index[i as usize] = 0;
        }
    }
    let mut positions = Vec::new();
    for (i, slot) in index.iter_mut().enumerate() {
        if *slot == 0 {
            *slot = positions.len() as u32;
            positions.push(scan.positions[i]);
        }
    }
    let tris = kept.iter().map(|t| t.map(|i| index[i as usize])).collect();
    (TriMesh::new(positions, tris), removed)
}

#[track_caller]
fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!(
        (a - b).abs() <= tol,
        "{what}: {a} against {b} (tolerance {tol})"
    );
}

#[test]
fn the_base_matches_the_plugin_on_opened_demo_scans() {
    let fx = fixture();
    let cut = f(&fx["cut_mm"]);
    let cases = fx["cases"].as_object().unwrap();
    assert!(!cases.is_empty());
    for (name, case) in cases {
        let bytes = std::fs::read(golden_case::dir(name).join("scan.dcm")).unwrap();
        let scan = dc_mesh::dcm::decode(&bytes).unwrap();
        let z = v3(&case["occlusal"]);
        let (open, removed) = opened(&scan, z, cut);
        assert_eq!(
            removed as u64,
            case["removed_faces"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            open.positions.len() as u64,
            case["vertices"].as_u64().unwrap()
        );
        assert_eq!(open.tris.len() as u64, case["faces"].as_u64().unwrap());
        for (height, want) in case["runs"].as_object().unwrap() {
            let what = format!("{name} {height} mm");
            let height: f64 = height.parse().unwrap();
            let got = scan_base::make_base(&open, z, height).unwrap().unwrap();
            let n = open.positions.len();
            assert_eq!(
                got.closed_edges as u64,
                want["closed_edges"].as_u64().unwrap(),
                "{what}"
            );
            assert_eq!(
                got.mesh.positions.len() as u64,
                want["vertices"].as_u64().unwrap(),
                "{what}"
            );
            assert_eq!(
                got.mesh.tris.len() as u64,
                want["faces"].as_u64().unwrap(),
                "{what}"
            );
            assert!(got.mesh.validate().is_watertight(), "{what}");
            close(
                got.plane,
                f(&want["plane"]),
                1e-9,
                &format!("{what}: plane"),
            );
            // the old points do not move (the plugin's do not either)
            assert_eq!(&got.mesh.positions[..n], &open.positions[..], "{what}");
            assert_eq!(
                f(&want["old_moved"]),
                0.0,
                "{what}: the plugin moved a point"
            );
            // the new points: on the plane, the same set
            let new: Vec<DVec3> = got.mesh.positions[n..]
                .iter()
                .map(|p| p.as_dvec3())
                .collect();
            for p in &new {
                close(p.dot(z), got.plane, 1e-4, &format!("{what}: height"));
            }
            let [lo, hi] = [&want["new_heights"][0], &want["new_heights"][1]].map(f);
            close(
                lo,
                got.plane,
                1e-4,
                &format!("{what}: plugin's lowest new point"),
            );
            close(
                hi,
                got.plane,
                1e-4,
                &format!("{what}: plugin's highest new point"),
            );
            let sum: DVec3 = new.iter().copied().sum();
            let tol = 1e-4 * new.len() as f64;
            for k in 0..3 {
                close(
                    sum[k],
                    v3(&want["new_sum"])[k],
                    tol,
                    &format!("{what}: sum {k}"),
                );
                let min = new.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
                let max = new.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
                close(
                    min,
                    v3(&want["new_min"])[k],
                    1e-4,
                    &format!("{what}: min {k}"),
                );
                close(
                    max,
                    v3(&want["new_max"])[k],
                    1e-4,
                    &format!("{what}: max {k}"),
                );
            }
            // the closed solid: same volume and area, outward normals
            let volume = signed_volume(&got.mesh);
            let want_volume = f(&want["volume"]);
            assert!(want_volume > 0.0, "{what}");
            close(
                volume,
                want_volume,
                1e-5 * want_volume,
                &format!("{what}: volume"),
            );
            let area: f64 = (0..got.mesh.tris.len())
                .map(|t| f64::from(got.mesh.face_area(t)))
                .sum();
            let want_area = f(&want["area"]);
            close(area, want_area, 1e-5 * want_area, &format!("{what}: area"));
        }
    }
}
