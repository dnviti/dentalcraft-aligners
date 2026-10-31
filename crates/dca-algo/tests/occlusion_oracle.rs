//! Numerical comparison of `dca_algo::occlusion` with the Python plugin
//! (`ortho_aligner/occlusion.py`).
//!
//! `tests/fixtures/occlusion.json` is produced by `gen_occlusion.py` from the plugin's own
//! code (python3 + numpy, no Blender): two demo arches in occlusion (with interpenetration),
//! the height grid and the first-contact placement of `_place` on a few poses, and the
//! signed distances of `signed_distances` both ways (the contact map). The Rust port is fed
//! the same single-precision vertices. Bounds: distances within 1e-5 mm, placements within
//! 1e-9 mm with the same contact counts.

use std::path::PathBuf;

use dca_algo::anatomy::Arch;
use dca_algo::occlusion::{self, ArchModel, TopSurface};
use glam::{DMat4, DVec3};
use serde_json::Value;

const TOL_MM: f64 = 1e-5;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/occlusion.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
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

/// Row-major 4x4 (numpy `ravel`) to a glam matrix.
fn mat4(v: &Value) -> DMat4 {
    let a: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    DMat4::from_cols_array(&a.try_into().unwrap()).transpose()
}

fn arch(c: &Value, arch: Arch, to_case: DMat4) -> ArchModel {
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
    assert_eq!(vertices.len() as u64, c["n_vertices"].as_u64().unwrap());
    assert_eq!(tris.len() as u64, c["n_tris"].as_u64().unwrap());
    ArchModel::from_arch(arch, to_case, vertices, tris, labels, Vec::new()).unwrap()
}

fn models(f: &Value) -> (ArchModel, ArchModel) {
    let lower = arch(&f["lower"], Arch::Lower, DMat4::IDENTITY);
    let upper = arch(&f["upper"], Arch::Upper, mat4(&f["relation"]));
    (lower, upper)
}

#[test]
fn the_height_grid_and_the_first_contact_match_the_plugin() {
    let f = fixture();
    let (lower, upper) = models(&f);
    assert_eq!(num(&f["contact"]), occlusion::CONTACT);
    let (pl, _) = lower.crown_points(None);
    let (pu, _) = upper.crown_points(None);
    let top = TopSurface::new(&pl, 0.5);
    let probes = &f["probes"];
    for (p, h) in probes["points"]
        .as_array()
        .unwrap()
        .iter()
        .zip(probes["heights"].as_array().unwrap())
    {
        let p = p.as_array().unwrap();
        let q = DVec3::new(num(&p[0]), num(&p[1]), num(&p[2]));
        let got = top.at(q);
        match h.as_f64() {
            Some(h) => assert!((got - h).abs() < 1e-12, "{q:?}: {got} vs {h}"),
            None => assert!(got.is_infinite(), "{q:?}: {got}"),
        }
    }
    for pose in f["poses"].as_array().unwrap() {
        let g = mat4(&pose["g"]);
        let r = occlusion::place(&pu, &top, &g, 0.0).unwrap();
        let tz = num(&pose["tz"]);
        assert!((r.tz - tz).abs() < 1e-9, "{} vs {tz}", r.tz);
        assert_eq!(r.contacts as u64, pose["contacts"].as_u64().unwrap());
        assert_eq!(r.right as u64, pose["right"].as_u64().unwrap());
        assert_eq!(r.left as u64, pose["left"].as_u64().unwrap());
    }
}

fn compare(name: &str, got: &[f64], oracle: &Value, measured: usize) -> (usize, f64) {
    let idx: Vec<usize> = oracle["index"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let dist: Vec<f64> = oracle["distance"]
        .as_array()
        .unwrap()
        .iter()
        .map(num)
        .collect();
    assert_eq!(oracle["n_measured"].as_u64().unwrap() as usize, measured);
    let mut worst: f64 = 0.0;
    let mut negative = 0;
    for (&i, &d) in idx.iter().zip(&dist) {
        let g = got[i];
        assert!(g.is_finite(), "{name} vertex {i}: oracle {d}, port {g}");
        assert_eq!(g < 0.0, d < 0.0, "{name} vertex {i}: oracle {d}, port {g}");
        worst = worst.max((g - d).abs());
        negative += usize::from(d < 0.0);
    }
    assert!(worst < TOL_MM, "{name}: worst {worst}");
    // nothing else is within reach (except at the very edge of the reach)
    let n_finite = got.iter().filter(|d| d.is_finite()).count();
    let extra: Vec<f64> = got
        .iter()
        .enumerate()
        .filter(|(i, d)| d.is_finite() && !idx.contains(i))
        .map(|(_, d)| *d)
        .collect();
    assert!(
        extra
            .iter()
            .all(|d| (d.abs() - occlusion::MAP_REACH).abs() < 1e-4),
        "{name}: {extra:?}"
    );
    assert_eq!(n_finite, idx.len() + extra.len());
    (negative, worst)
}

#[test]
fn contact_distances_match_the_plugin() {
    let f = fixture();
    let (lower, upper) = models(&f);
    let map = occlusion::contact_map(&lower, &upper).unwrap();
    let n_l = lower.labels.iter().filter(|&&l| l != 0).count();
    let n_u = upper.labels.iter().filter(|&&l| l != 0).count();
    let (neg_u, worst_u) = compare("upper", &map.upper.distances, &f["upper_to_lower"], n_u);
    let (neg_l, worst_l) = compare("lower", &map.lower.distances, &f["lower_to_upper"], n_l);
    // the case exercises interpenetration and contacts on both arches
    assert!(neg_u > 10 && neg_l > 10, "{neg_u} {neg_l}");
    assert!(map.penetration_mm > 0.2);
    assert!(!map.lower.teeth_in_contact.is_empty() && !map.upper.teeth_in_contact.is_empty());
    eprintln!(
        "occlusion oracle: upper {} / lower {} vertices within reach, {neg_u} / {neg_l} inside; \
         worst difference {:.2e} / {:.2e} mm; deepest {:.3} mm",
        f["upper_to_lower"]["index"].as_array().unwrap().len(),
        f["lower_to_upper"]["index"].as_array().unwrap().len(),
        worst_u,
        worst_l,
        map.penetration_mm
    );
}
