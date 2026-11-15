//! The analysis of a control visit of `dca_algo::visits` against the Python
//! plugin (`visit._register_compute`, `visit._track_compute`).
//!
//! `tests/fixtures/visits.json` is produced by `gen_visits.py` from the
//! plugin's own functions on a golden demo case: the scan of the case with
//! every tooth moved by a planned motion is the expected model, the same with
//! every tooth moved again by a deviation (two teeth well behind), placed
//! anywhere, is the control scan. The motions are stored in the fixture and
//! the scans are rebuilt here, so both sides run on the same arrays.
//!
//! Compared: the registration of the scan (transform on probe points, RMS,
//! normal sign) and, for every tooth, the achieved transform (displacement of
//! the crown centre and rotation angle against the plugin's), the tracking
//! figures and the deviation components.

mod golden_case;

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::visits::{self, MeshRef, ToothInput};
use dca_algo::{frame, rotation};
use glam::{DMat3, DMat4, DVec3};
use golden_case::Case;
use serde_json::Value;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visits.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A flat row-major 4x4 matrix of the fixture.
fn mat(v: &Value) -> DMat4 {
    let a: Vec<f64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    let rows: [f64; 16] = a.try_into().unwrap();
    DMat4::from_cols_array(&rows).transpose()
}

fn fdi(n: u64) -> Fdi {
    Fdi::new(n as u8).unwrap()
}

/// Displacement (mm) of `c` and rotation (degrees) between two transforms.
fn difference(a: &DMat4, b: &DMat4, c: DVec3) -> (f64, f64) {
    let e = *a * frame::rigid_inverse(b);
    (
        e.transform_point3(c).distance(c),
        rotation::matrix_to_rotvec(&DMat3::from_mat4(e))
            .length()
            .to_degrees(),
    )
}

#[test]
fn the_analysis_of_a_visit_matches_the_plugin() {
    let fx = fixture();
    let case = Case::load(fx["case"].as_str().unwrap());
    let fdis: Vec<Fdi> = fx["fdis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| fdi(f.as_u64().unwrap()))
        .collect();
    assert_eq!(fdis, case.fdis());
    let planned: BTreeMap<Fdi, DMat4> = fdis
        .iter()
        .map(|f| (*f, mat(&fx["planned"][f.to_string()])))
        .collect();
    let deviation: BTreeMap<Fdi, DMat4> = fdis
        .iter()
        .map(|f| (*f, mat(&fx["deviation"][f.to_string()])))
        .collect();
    let placement = mat(&fx["placement"]);

    // expected model and control scan: the teeth moved, the gingiva where it was
    let mut expected = case.vertices.clone();
    let mut scan = case.vertices.clone();
    for (i, &l) in case.labels.iter().enumerate() {
        if let Some(f) = u8::try_from(l)
            .ok()
            .and_then(|n| Fdi::new(n).ok())
            .filter(|f| planned.contains_key(f))
        {
            expected[i] = planned[&f].transform_point3(case.vertices[i]);
            scan[i] = (deviation[&f] * planned[&f]).transform_point3(case.vertices[i]);
        }
    }
    let scan: Vec<DVec3> = scan
        .iter()
        .map(|&p| placement.transform_point3(p))
        .collect();

    let mut teeth = Vec::new();
    for &f in &fdis {
        let rest = case.rest_frame(f);
        let inv = frame::rigid_inverse(&rest);
        let rot = DMat3::from_mat4(inv);
        let tris: Vec<[u32; 3]> = case
            .tris
            .iter()
            .copied()
            .filter(|t| {
                t.iter()
                    .all(|&i| case.labels[i as usize] == u16::from(f.get()))
            })
            .collect();
        let normals = visits::vertex_normals(&case.vertices, &tris);
        let idx: Vec<usize> = (0..case.vertices.len())
            .filter(|&i| case.labels[i] == u16::from(f.get()))
            .collect();
        teeth.push(ToothInput {
            fdi: f,
            rest,
            expected: planned[&f] * rest,
            points: idx
                .iter()
                .map(|&i| inv.transform_point3(case.vertices[i]))
                .collect(),
            normals: idx.iter().map(|&i| rot * normals[i]).collect(),
            md_sign: case.teeth["teeth"][f.to_string()]["md_sign"]
                .as_i64()
                .unwrap() as i8,
        });
    }

    let out = visits::run(
        &visits::Input {
            arch: Arch::Lower,
            expected: MeshRef {
                vertices: &expected,
                tris: &case.tris,
            },
            scan: MeshRef {
                vertices: &scan,
                tris: &case.tris,
            },
            teeth: &teeth,
        },
        &visits::Params::default(),
    )
    .unwrap();

    // the registration of the scan
    let reg = &fx["registration"];
    let want = mat(&reg["T"]);
    let mut worst = 0.0_f64;
    for p in [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(20.0, 10.0, -3.0),
        DVec3::new(-25.0, 15.0, 4.0),
    ] {
        worst = worst.max(
            out.registration
                .transform
                .transform_point3(p)
                .distance(want.transform_point3(p)),
        );
    }
    println!(
        "registration: probes within {worst:.5} mm, rms {:.5} vs {:.5}",
        out.registration.rms_mm,
        reg["rms"].as_f64().unwrap()
    );
    assert!(worst < 1e-3, "registration differs by {worst} mm");
    assert_eq!(out.registration.normal_sign, reg["sign"].as_f64().unwrap());
    assert!((out.registration.rms_mm - reg["rms"].as_f64().unwrap()).abs() < 2e-3);

    // every tooth
    assert_eq!(out.teeth.len(), fx["teeth"].as_object().unwrap().len());
    let (mut w_mm, mut w_deg, mut w_dev) = (0.0_f64, 0.0_f64, 0.0_f64);
    for t in &teeth {
        let key = t.fdi.to_string();
        let py = &fx["teeth"][&key];
        let got = &out.teeth[&t.fdi];
        let t_got = t.rest * got.achieved.to_mat4() * frame::rigid_inverse(&t.expected);
        let t_py = mat(&py["T"]);
        let centre = t
            .expected
            .transform_point3(t.points.iter().copied().sum::<DVec3>() / t.points.len() as f64);
        let (mm, deg) = difference(&t_got, &t_py, centre);
        w_mm = w_mm.max(mm);
        w_deg = w_deg.max(deg);
        let dev = &got.deviation;
        let py_dev = &py["deviazione"];
        let pairs = [
            (dev.mesial_mm, "mesiale mm"),
            (dev.buccal_mm, "vestibolare mm"),
            (dev.extrusion_mm, "estrusione mm"),
            (dev.torque_deg, "torque gradi"),
            (dev.tip_deg, "tip gradi"),
            (dev.rotation_deg, "rotazione gradi"),
        ];
        for (v, name) in pairs {
            let d = (v - py_dev[name].as_f64().unwrap()).abs();
            w_dev = w_dev.max(d);
            assert!(d < 0.03, "{key} {name}: {v} vs {}", py_dev[name]);
        }
        assert_eq!(got.on_track, py["in_pari"].as_bool().unwrap(), "{key}");
        match (got.efficacy_pct, py["efficacia_pct"].as_f64()) {
            (Some(g), Some(p)) => assert!((g - p).abs() < 2.0, "{key} efficacy {g} vs {p}"),
            (None, None) => {}
            (g, p) => panic!("{key} efficacy {g:?} vs {p:?}"),
        }
        assert!(
            (got.planned_mm - py["movimento_pianificato_mm"].as_f64().unwrap()).abs() < 0.006,
            "{key}"
        );
        println!(
            "{key}: T within {mm:.5} mm {deg:.4} deg, rms {:.4} vs {:.4}",
            got.rms_mm, py["rms_icp_mm"]
        );
    }
    println!("worst tooth: {w_mm:.5} mm, {w_deg:.4} deg, deviation components {w_dev:.4}");
    assert!(w_mm < 1e-3, "teeth differ by {w_mm} mm");
    assert!(w_deg < 0.01, "teeth differ by {w_deg} deg");
}
