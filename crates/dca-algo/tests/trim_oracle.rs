//! Numerical comparison of the trim lines of `dca_algo::trim` with the Python
//! plugin (`ortho_aligner/case_print.py`, `aligner.py`).
//!
//! `tests/fixtures/trim.json` is produced by `gen_trim.py` from the plugin's own
//! functions (python3 + numpy, no Blender): three synthetic teeth (the plugin's
//! demo crowns, the incisors proclined, the cervical ring scalloped) with the
//! equator loops of `tooth_equators` along the vertical axis, the EQUATOR trim
//! loops of `equator_trim` for three offsets, the loops bridged at the
//! contacts (`_bridge_contacts`), the STRAIGHT and SCALLOPED control points of
//! `trim_points` and the trim surfaces of `trim_cutter`. Bound: 1e-6 mm.

use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::trim::Tooth;
use dca_algo::trim::cutter;
use dca_algo::trim::grid::PlanGrid;
use dca_algo::trim::line::{self, MarginMode};
use dca_algo::trim::survey;
use glam::DVec3;
use serde_json::Value;

const TOL_MM: f64 = 1e-6;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trim.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> DVec3 {
    let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    DVec3::new(n[0], n[1], n[2])
}

fn points(v: &Value) -> Vec<DVec3> {
    v.as_array().unwrap().iter().map(vec3).collect()
}

fn tooth(t: &Value) -> Tooth {
    Tooth {
        fdi: Fdi::new(num(&t["fdi"]) as u8).unwrap(),
        crown: points(&t["crown"]),
        closed: points(&t["closed"]),
        margin: points(&t["margin"]),
        origin: vec3(&t["origin"]),
        e_bl: vec3(&t["e_bl"]),
        covered: true,
    }
}

fn close(a: &[DVec3], b: &[DVec3], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: lengths");
    for (k, (p, q)) in a.iter().zip(b).enumerate() {
        assert!(
            (*p - *q).abs().max_element() < TOL_MM,
            "{what} [{k}]: {p} vs {q}"
        );
    }
}

#[test]
fn equator_loops_match_the_plugin() {
    let f = fixture();
    let wall = num(&f["wall"]);
    let mut trimmed = Vec::new();
    for t in f["teeth"].as_array().unwrap() {
        let tooth = tooth(t);
        let eq = line::tooth_equator(&tooth, line::EQUATOR_AZIMUTHS).unwrap();
        let what = format!("{} equator", tooth.fdi);
        close(&eq.points, &points(&t["equator"]), &what);
        assert!((eq.top - num(&t["top"])).abs() < TOL_MM);
        let zm: Vec<f64> = t["margin_z"].as_array().unwrap().iter().map(num).collect();
        for (a, b) in eq.margin.as_ref().unwrap().iter().zip(&zm) {
            assert!((a - b).abs() < TOL_MM, "{what} margin");
        }
        for (off, want) in t["trim"].as_object().unwrap() {
            let off: f64 = off.parse().unwrap();
            let tr = line::equator_trim(&eq, off, wall);
            close(
                &tr.points,
                &points(want),
                &format!("{} trim {off}", tooth.fdi),
            );
            if off == 0.0 {
                trimmed.push(tr.points);
            }
        }
    }
    let bridged = survey::bridge_contacts(&trimmed, cutter::EQUATOR_CONTACT_MM);
    let want: Vec<Vec<DVec3>> = f["bridged"]
        .as_array()
        .unwrap()
        .iter()
        .map(points)
        .collect();
    for (k, (a, b)) in bridged.iter().zip(&want).enumerate() {
        close(a, b, &format!("bridged {k}"));
    }
    // the contact of 31 and 32 is bridged: some heights changed
    assert!(
        bridged[0]
            .iter()
            .zip(&trimmed[0])
            .any(|(a, b)| (a.z - b.z).abs() > 1e-3)
    );
}

#[test]
fn margin_lines_and_trim_surfaces_match_the_plugin() {
    let f = fixture();
    let ext = num(&f["extension"]);
    let teeth: Vec<Tooth> = f["teeth"].as_array().unwrap().iter().map(tooth).collect();
    let c = &f["cutter"];
    let (lo, hi) = (vec3(&c["lo"]), vec3(&c["hi"]));
    let grid = PlanGrid::around(lo, hi, num(&c["cell"]));
    for (mode, key, k) in [
        (MarginMode::Straight, "straight", 2),
        (MarginMode::Scalloped, "scalloped", 4),
    ] {
        let per = line::margin_points(&teeth, mode, ext);
        for ((fdi, pts), t) in per.iter().zip(f["teeth"].as_array().unwrap()) {
            close(pts, &points(&t[key]), &format!("{fdi} {key}"));
        }
        let ctrl: Vec<DVec3> = per.iter().flat_map(|(_, p)| p.iter().copied()).collect();
        let z = cutter::interpolated(&grid, &ctrl, k, 40.0, 3.0, hi.z + 5.0);
        let want: Vec<f64> = c[key].as_array().unwrap().iter().map(num).collect();
        assert_eq!(z.len(), want.len(), "{key} grid");
        for (i, (a, b)) in z.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < TOL_MM, "{key} surface [{i}]: {a} vs {b}");
        }
    }
}
