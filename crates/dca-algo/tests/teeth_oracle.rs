//! Numerical comparison of `dca_algo::teeth` with the Python plugin
//! (`ortho_aligner/teeth.py`).
//!
//! `tests/fixtures/teeth.json` is produced by `gen_teeth.py` from the plugin's own
//! modules (python3 + numpy, no Blender): two labelled demo arches (one connected
//! surface, with a gingival margin and interproximal contacts) and what
//! `compute_teeth` gives for them. The Rust port is fed the same single-precision
//! vertices. Bounds: frames within 0.05 degrees, widths and points within 0.01 mm.

use std::path::PathBuf;

use dca_algo::anatomy::Arch;
use dca_algo::teeth::{self, Input, Params};
use glam::DVec3;
use serde_json::Value;

const TOL_DEG: f64 = 0.05;
const TOL_MM: f64 = 0.01;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/teeth.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(num).collect()
}

fn vec3(v: &Value) -> DVec3 {
    let n = nums(v);
    DVec3::new(n[0], n[1], n[2])
}

fn base64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let bytes = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= u32::from(val(c)) << (18 - 6 * i);
        }
        let n = chunk.len() - 1;
        out.extend_from_slice(&acc.to_be_bytes()[1..=n]);
    }
    out
}

struct Case {
    arch: Arch,
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    labels: Vec<u16>,
}

fn case(c: &Value) -> Case {
    let v = base64(c["vertices"].as_str().unwrap());
    let vertices: Vec<DVec3> = v
        .as_chunks::<12>()
        .0
        .iter()
        .map(|b| {
            let f =
                |k: usize| f64::from(f32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap()));
            DVec3::new(f(0), f(1), f(2))
        })
        .collect();
    let t = base64(c["tris"].as_str().unwrap());
    let tris: Vec<[u32; 3]> = t
        .as_chunks::<6>()
        .0
        .iter()
        .map(|b| {
            let i = |k: usize| u32::from(u16::from_le_bytes([b[2 * k], b[2 * k + 1]]));
            [i(0), i(1), i(2)]
        })
        .collect();
    let labels: Vec<u16> = base64(c["labels"].as_str().unwrap())
        .into_iter()
        .map(u16::from)
        .collect();
    assert_eq!(vertices.len(), c["n_vertices"].as_u64().unwrap() as usize);
    assert_eq!(tris.len(), c["n_tris"].as_u64().unwrap() as usize);
    assert_eq!(labels.len(), vertices.len());
    Case {
        arch: if c["arch"] == "UPPER" {
            Arch::Upper
        } else {
            Arch::Lower
        },
        vertices,
        tris,
        labels,
    }
}

/// Largest deviations found, reported with `--nocapture`.
#[derive(Default)]
struct Worst {
    deg: f64,
    mm: f64,
}

#[track_caller]
fn point(a: DVec3, b: DVec3, what: &str, w: &mut Worst) {
    let d = (a - b).length();
    w.mm = w.mm.max(d);
    assert!(d <= TOL_MM, "{what}: {a:?} against {b:?} ({d} mm)");
}

#[track_caller]
fn scalar_mm(a: f64, b: f64, what: &str, w: &mut Worst) {
    let d = (a - b).abs();
    w.mm = w.mm.max(d);
    assert!(d <= TOL_MM, "{what}: {a} against {b}");
}

#[track_caller]
fn axis(a: DVec3, b: DVec3, what: &str, w: &mut Worst) {
    let d = a.dot(b).clamp(-1.0, 1.0).acos().to_degrees();
    w.deg = w.deg.max(d);
    assert!(d <= TOL_DEG, "{what}: {a:?} against {b:?} ({d} deg)");
}

#[track_caller]
fn scalar_deg(a: f64, b: f64, what: &str, w: &mut Worst) {
    let d = (a - b).abs();
    w.deg = w.deg.max(d);
    assert!(d <= TOL_DEG, "{what}: {a} against {b}");
}

#[test]
fn percentile_and_subsample_match_numpy() {
    let fx = fixture();
    for p in fx["percentile"].as_array().unwrap() {
        let values = nums(&p["values"]);
        for (q, want) in p["q"].as_object().unwrap() {
            let got = teeth::percentile(&values, q.parse().unwrap());
            assert!(
                (got - num(want)).abs() <= 1e-12 * (1.0 + got.abs()),
                "n={} q={q}: {got} against {want}",
                values.len()
            );
        }
    }
    for s in fx["subsample"].as_array().unwrap() {
        let n = s["n"].as_u64().unwrap() as usize;
        let k = s["k"].as_u64().unwrap() as usize;
        let pts: Vec<DVec3> = (0..n)
            .map(|i| DVec3::new(3.0 * i as f64, 3.0 * i as f64 + 1.0, 3.0 * i as f64 + 2.0))
            .collect();
        let got: Vec<f64> = teeth::subsample(&pts, k).iter().map(|p| p.x).collect();
        assert_eq!(got, nums(&s["first_col"]), "subsample {n} -> {k}");
    }
}

/// Runs the port on `input` and compares every tooth with the plugin's record.
fn compare(name: &str, input: &Case, want_seq: &Value, want_teeth: &Value, w: &mut Worst) {
    let out = teeth::run(
        &Input {
            arch: input.arch,
            vertices: &input.vertices,
            tris: &input.tris,
            labels: &input.labels,
        },
        &Params::default(),
    )
    .unwrap();
    let want_seq: Vec<u64> = want_seq
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_u64().unwrap())
        .collect();
    let got_seq: Vec<u64> = out.sequence.iter().map(|f| u64::from(f.get())).collect();
    assert_eq!(got_seq, want_seq, "{name} sequence");

    for want in want_teeth.as_array().unwrap() {
        let fdi = want["fdi"].as_u64().unwrap();
        let t = &out.teeth[&dc_case::Fdi::new(fdi as u8).unwrap()];
        let at = |what: &str| format!("{name} tooth {fdi} {what}");
        assert_eq!(
            i64::from(t.md_sign),
            want["md_sign"].as_i64().unwrap(),
            "{}",
            at("md_sign")
        );
        assert_eq!(
            t.margin.len(),
            want["n_margin"].as_u64().unwrap() as usize,
            "{}",
            at("margin")
        );
        assert_eq!(
            t.vertex_count,
            want["n_vertices"].as_u64().unwrap() as usize
        );
        assert_eq!(t.face_count, want["n_faces"].as_u64().unwrap() as usize);

        axis(t.e_md, vec3(&want["e_md"]), &at("e_md"), w);
        axis(t.e_bl, vec3(&want["e_bl"]), &at("e_bl"), w);
        axis(t.e_ax, vec3(&want["e_ax"]), &at("e_ax"), w);
        axis(t.mesial_dir, vec3(&want["mesial_dir"]), &at("mesial"), w);
        axis(t.out_dir, vec3(&want["out_dir"]), &at("out"), w);
        scalar_deg(t.axis_tilt_deg, num(&want["axis_tilt"]), &at("tilt"), w);
        scalar_deg(t.rot_dev_deg, num(&want["rot_dev"]), &at("rot_dev"), w);

        scalar_mm(t.width_md, num(&want["width_md"]), &at("width_md"), w);
        scalar_mm(t.width_bl, num(&want["width_bl"]), &at("width_bl"), w);
        scalar_mm(t.height, num(&want["height"]), &at("height"), w);
        scalar_mm(t.occ_z, num(&want["occ_z"]), &at("occ_z"), w);
        scalar_mm(t.zen_b, num(&want["zen_b"]), &at("zen_b"), w);
        scalar_mm(t.zen_l, num(&want["zen_l"]), &at("zen_l"), w);
        scalar_mm(t.top_elong, num(&want["top_elong"]), &at("top_elong"), w);
        scalar_mm(t.top_spread, num(&want["top_spread"]), &at("top_spread"), w);
        for (got, key) in [
            (t.centroid, "centroid"),
            (t.reference, "ref"),
            (t.tip, "tip"),
            (t.margin_mean, "margin_mean"),
            (t.cej, "cej"),
            (t.crest, "crest"),
            (t.apex, "apex"),
            (t.cres, "cres"),
            (t.contact_m, "contact_m"),
            (t.contact_d, "contact_d"),
        ] {
            point(got, vec3(&want[key]), &at(key), w);
        }

        // the rest frame of the plugin: axes as columns, origin at the centre of resistance
        let frame = t.rest_frame();
        let rows: Vec<Vec<f64>> = want["frame"].as_array().unwrap().iter().map(nums).collect();
        for (r, row) in rows.iter().enumerate() {
            for (col, want) in row.iter().enumerate() {
                let got = frame.col(col)[r];
                assert!((got - want).abs() <= 1e-3, "{}[{r}][{col}]", at("frame"));
            }
        }
    }
}

#[test]
fn the_teeth_match_the_plugin() {
    let fx = fixture();
    let mut w = Worst::default();
    for c in fx["cases"].as_array().unwrap() {
        let input = case(c);
        let name = format!("{} connected", c["arch"]);
        compare(&name, &input, &c["sequence"], &c["teeth"], &mut w);
    }
    eprintln!(
        "teeth oracle (margin and contacts): worst angle {:.2e} deg, worst length {:.2e} mm",
        w.deg, w.mm
    );
}

/// Separate parts: no border between a tooth and the gum or between teeth, so the
/// margin is the lowest 10% of the crown and no tooth has contacts.
#[test]
fn the_teeth_without_margin_or_contacts_match_the_plugin() {
    let fx = fixture();
    let mut w = Worst::default();
    for c in fx["cases"].as_array().unwrap() {
        let mut input = case(c);
        let plain = &c["plain"];
        input
            .tris
            .truncate(plain["n_tris"].as_u64().unwrap() as usize);
        input.labels = base64(plain["labels"].as_str().unwrap())
            .into_iter()
            .map(u16::from)
            .collect();
        let name = format!("{} separate", c["arch"]);
        compare(&name, &input, &plain["sequence"], &plain["teeth"], &mut w);
    }
    eprintln!(
        "teeth oracle (fallback margin): worst angle {:.2e} deg, worst length {:.2e} mm",
        w.deg, w.mm
    );
}

#[test]
fn the_wall_axis_matches_the_plugin() {
    let fx = fixture();
    let mut w = Worst::default();
    for c in fx["cases"].as_array().unwrap() {
        let input = case(c);
        for (f, want) in c["wall_axis"].as_object().unwrap() {
            let label: u16 = f.parse().unwrap();
            let fidx: Vec<usize> = input
                .tris
                .iter()
                .enumerate()
                .filter(|(_, t)| t.iter().all(|&i| input.labels[i as usize] == label))
                .map(|(i, _)| i)
                .collect();
            let got = teeth::wall_axis(&input.vertices, &input.tris, &fidx, None);
            axis(got, vec3(want), &format!("wall axis of {f}"), &mut w);
        }
    }
    eprintln!("wall axis oracle: worst angle {:.2e} deg", w.deg);
}
