//! Numerical comparison of `dca_algo::aligner_bite` and `dca_algo::ramps`
//! with the Python plugin (`aligner_bite.py`, `ramp_shape.py`, `ramps.py`).
//!
//! `tests/fixtures/aligner_bite.json` is produced by `gen_aligner_bite.py`
//! from the plugin's own code (python3 + numpy, no Blender) on the two demo
//! arches of `occlusion.json`: envelopes, heights, surfaces, the closings
//! (vertical and hinge, with and without the front teeth), the
//! equilibration, the ramp mesh and frames and `propose` on four cases. The
//! Rust port is fed the same single-precision vertices. Bounds: height maps
//! and closings within 1e-9 (the closings that use the front teeth within
//! 1e-4: the nearest points run on single-precision trees); ramp meshes and
//! placement within 1e-9 mm.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_mesh::TriMesh;
use dca_algo::aligner_bite::{
    Closing, Front, Grid, Raster, Surface, ball_closing, bare_surface, base_from_heights, closing,
    envelope, equilibrate, first_contact, heights, hinge_axis, is_front, limit_heights,
    occlusal_surface, opening_at, overlap_mask, pose, solve_thickness, with_occlusal,
};
use dca_algo::ramps::{self, ProposeInput, ProposeTooth};
use dca_model::{BiteClosing, RampSettings};
use glam::{DMat4, DVec3};
use serde_json::Value;

const CELL: f64 = 0.5;
const TOL: f64 = 1e-9;
const TOL_FRONT: f64 = 1e-4;

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
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

fn mat4(v: &Value) -> DMat4 {
    let flat = |v: &Value| -> Vec<f64> {
        match v.as_array().unwrap().first() {
            Some(Value::Array(_)) => v
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| r.as_array().unwrap().iter().map(num))
                .collect(),
            _ => v.as_array().unwrap().iter().map(num).collect(),
        }
    };
    let a = flat(v);
    DMat4::from_cols_array(&a.try_into().unwrap()).transpose()
}

fn vec3(v: &Value) -> DVec3 {
    let a = v.as_array().unwrap();
    DVec3::new(num(&a[0]), num(&a[1]), num(&a[2]))
}

/// Crown points (labelled vertices) of an arch of `occlusion.json`, with all
/// its vertices, triangles and labels.
struct Arch {
    crown: Vec<DVec3>,
    crown_labels: Vec<i64>,
}

fn arch(rec: &Value) -> Arch {
    let n = rec["n_vertices"].as_u64().unwrap() as usize;
    let v = base64(rec["vertices"].as_str().unwrap());
    let l = base64(rec["labels"].as_str().unwrap());
    assert_eq!(l.len(), n);
    let verts: Vec<DVec3> = v
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            let f =
                |k: usize| f64::from(f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap()));
            DVec3::new(f(0), f(1), f(2))
        })
        .collect();
    let mut crown = Vec::new();
    let mut crown_labels = Vec::new();
    for (p, &lab) in verts.iter().zip(&l) {
        if lab != 0 {
            crown.push(*p);
            crown_labels.push(i64::from(lab));
        }
    }
    Arch {
        crown,
        crown_labels,
    }
}

fn grid(p: &Value, key: &str) -> Grid {
    let g = &p[key];
    let two = |v: &Value| [num(&v[0]), num(&v[1])];
    Grid::new(two(&g["lo"]), two(&g["hi"]), CELL)
}

fn close(name: &str, got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol * want.abs().max(1.0),
        "{name}: got {got}, want {want}"
    );
}

/// Totals and probes of a raster against the fixture's `stats`.
fn stats(name: &str, probes: &[usize], got: &Raster<f64>, want: &Value) {
    let fin: Vec<f64> = got.data.iter().copied().filter(|v| v.is_finite()).collect();
    assert_eq!(
        fin.len() as u64,
        want["n"].as_u64().unwrap(),
        "{name}: finite nodes"
    );
    close(
        &format!("{name} sum"),
        fin.iter().sum(),
        num(&want["sum"]),
        1e-9,
    );
    let wp = want["probes"].as_array().unwrap();
    let used: Vec<&usize> = probes.iter().filter(|&&i| i < got.data.len()).collect();
    assert_eq!(used.len(), wp.len());
    for (&&i, w) in used.iter().zip(wp) {
        let g = got.data[i];
        match w {
            Value::Null => assert!(!g.is_finite(), "{name}[{i}]: {g}"),
            w => close(&format!("{name}[{i}]"), g, num(w), TOL),
        }
    }
}

fn stats_l(name: &str, probes: &[usize], got: &Raster<i64>, want: &Value) {
    stats(name, probes, &got.map(|v| v as f64), want);
}

fn surf_stats(name: &str, probes: &[usize], s: &Surface, want: &Value) {
    stats(&format!("{name} z"), probes, &s.z, &want["z"]);
    stats_l(&format!("{name} l"), probes, &s.l, &want["l"]);
    stats(
        &format!("{name} z_wall"),
        probes,
        &s.z_wall,
        &want["z_wall"],
    );
    stats(&format!("{name} z_min"), probes, &s.z_min, &want["z_min"]);
    assert_eq!(
        s.cov.data.iter().filter(|&&c| c).count() as u64,
        want["cov"].as_u64().unwrap(),
        "{name} cov"
    );
}

fn closing_check(name: &str, c: &Option<Closing>, want: &Value, tol: f64) {
    if want.is_null() {
        assert!(c.is_none(), "{name}");
        return;
    }
    let c = c.as_ref().unwrap_or_else(|| panic!("{name}: no closing"));
    close(&format!("{name} value"), c.value, num(&want["value"]), tol);
    let ids = |v: &Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    };
    assert_eq!(c.sides.right, ids(&want["sides"]["R"]), "{name} right");
    assert_eq!(c.sides.left, ids(&want["sides"]["L"]), "{name} left");
    for (map, key) in [(&c.upper, "upper"), (&c.lower, "lower")] {
        let w = want[key].as_object().unwrap();
        assert_eq!(map.len(), w.len(), "{name} {key}");
        for (f, n) in map {
            assert_eq!(
                *n as u64,
                w[&f.to_string()].as_u64().unwrap(),
                "{name} {key} {f}"
            );
        }
    }
    assert_eq!(
        c.front_g.len() as u64,
        want["n_front"].as_u64().unwrap(),
        "{name} n_front"
    );
    let fin: Vec<f64> = c.g.iter().copied().filter(|v| v.is_finite()).collect();
    assert_eq!(
        fin.len() as u64,
        want["g_n"].as_u64().unwrap(),
        "{name} g_n"
    );
    close(
        &format!("{name} g_sum"),
        fin.iter().sum(),
        num(&want["g_sum"]),
        tol.max(1e-8),
    );
    for (k, key) in ["R", "L"].iter().enumerate() {
        match &want["side_gap"][key] {
            Value::Null => assert!(c.side_gap[k].is_none(), "{name} gap {key}"),
            w => close(
                &format!("{name} gap {key}"),
                c.side_gap[k].unwrap(),
                num(w),
                tol.max(1e-8),
            ),
        }
    }
}

#[test]
fn the_height_maps_and_surfaces_match_the_plugin() {
    let fx = read("aligner_bite.json");
    let occ = read("occlusion.json");
    let p = &fx["params"];
    let probes: Vec<usize> = p["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let covered: Vec<i64> = p["covered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let (clear, wall, min) = (num(&p["clearance"]), num(&p["wall"]), num(&p["minimum"]));
    let lower = arch(&occ["lower"]);
    let gl = grid(p, "grid_lower");

    let (z0, l0) = envelope(&lower.crown, 0.0, &gl, Some(&lower.crown_labels), None);
    stats("bare z", &probes, &z0, &fx["envelope"]["bare"]["z"]);
    stats_l("bare l", &probes, &l0, &fx["envelope"]["bare"]["l"]);
    let (zb, lb) = envelope(&lower.crown, 1.0, &gl, Some(&lower.crown_labels), None);
    stats("ball z", &probes, &zb, &fx["envelope"]["ball"]["z"]);
    stats_l("ball l", &probes, &lb, &fx["envelope"]["ball"]["l"]);
    let ze = envelope(&lower.crown, 1.0, &gl, None, Some(0.75)).0;
    stats("ellipsoid", &probes, &ze, &fx["envelope"]["ellipsoid"]);

    let (s, sl) = heights(&lower.crown, &lower.crown_labels, &gl, 0.3);
    stats("heights z", &probes, &s, &fx["heights"]["b03"]["z"]);
    stats_l("heights l", &probes, &sl, &fx["heights"]["b03"]["l"]);
    let (s8, sl8) = heights(&lower.crown, &lower.crown_labels, &gl, 0.8);
    stats("heights08 z", &probes, &s8, &fx["heights"]["b08"]["z"]);
    stats_l("heights08 l", &probes, &sl8, &fx["heights"]["b08"]["l"]);
    stats(
        "ball_closing",
        &probes,
        &ball_closing(&z0, CELL, 0.8),
        &fx["ball_closing"],
    );

    let cov_l: Vec<i64> = fx["surface"]["covered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let base = base_from_heights(&s, &sl, &cov_l, clear, wall, min, &gl);
    let sf = with_occlusal(&base, 0.75);
    surf_stats(
        "occlusal 0.75",
        &probes,
        &sf,
        &fx["surface"]["occlusal_075"],
    );
    surf_stats(
        "wall",
        &probes,
        &with_occlusal(&base, wall),
        &fx["surface"]["wall"],
    );
    surf_stats(
        "occlusal 0.40",
        &probes,
        &with_occlusal(&base, 0.4),
        &fx["surface"]["occlusal_040"],
    );
    surf_stats(
        "bare",
        &probes,
        &bare_surface(&base),
        &fx["surface"]["bare"],
    );
    let direct = occlusal_surface(
        &lower.crown,
        &lower.crown_labels,
        &cov_l,
        clear,
        wall,
        0.75,
        min,
        &gl,
    );
    surf_stats("direct", &probes, &direct, &fx["occlusal_surface_direct"]);
    let lim = limit_heights(&sf, None);
    let fin: Vec<f64> = lim.data.iter().copied().filter(|v| v.is_finite()).collect();
    assert_eq!(fin.len() as u64, fx["limit_heights"]["n"].as_u64().unwrap());
    close(
        "limit sum",
        fin.iter().sum(),
        num(&fx["limit_heights"]["sum"]),
        1e-9,
    );
    assert!(covered.len() > 20);
}

struct Case {
    sl: Surface,
    su: Surface,
    bare_l: Surface,
    bare_u: Surface,
    rel: DMat4,
    mask: Raster<bool>,
    axis: DVec3,
    lower: Arch,
    upper: Arch,
}

fn case() -> (Case, Value) {
    let fx = read("aligner_bite.json");
    let occ = read("occlusion.json");
    let p = &fx["params"];
    let covered: Vec<i64> = p["covered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let (clear, wall, min) = (num(&p["clearance"]), num(&p["wall"]), num(&p["minimum"]));
    let (lower, upper) = (arch(&occ["lower"]), arch(&occ["upper"]));
    let (gl, gu) = (grid(p, "grid_lower"), grid(p, "grid_upper"));
    let rel = mat4(&occ["relation"]);
    let (s, sl) = heights(&lower.crown, &lower.crown_labels, &gl, 0.3);
    let (su_h, lu_h) = heights(&upper.crown, &upper.crown_labels, &gu, 0.3);
    let bl = base_from_heights(&s, &sl, &covered, clear, wall, min, &gl);
    let bu = base_from_heights(&su_h, &lu_h, &covered, clear, wall, min, &gu);
    let (bare_l, bare_u) = (bare_surface(&bl), bare_surface(&bu));
    let mut mask = overlap_mask(&bare_l, &bare_u, &rel, 0.25, wall + 0.5);
    for (m, (a, b)) in mask
        .data
        .iter_mut()
        .zip(bu.lall.data.iter().zip(&bu.lw.data))
    {
        *m &= !is_front(*a, true) && !is_front(*b, true);
    }
    assert_eq!(
        mask.data.iter().filter(|&&m| m).count() as u64,
        fx["mask"]["n_true"].as_u64().unwrap()
    );
    let axis = hinge_axis(vec3(&fx["hinge"]["incisal"]), 78.0, 38.0);
    let w = vec3(&fx["hinge"]["axis"]);
    assert!((axis - w).length() < 1e-9, "{axis} {w}");
    let c = Case {
        sl: with_occlusal(&bl, 0.75),
        su: with_occlusal(&bu, 0.75),
        bare_l,
        bare_u,
        rel,
        mask,
        axis,
        lower,
        upper,
    };
    (c, fx)
}

fn front_of(c: &Case, r: f64, k: f64) -> Front {
    let pick = |a: &Arch, up: bool| {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        for (q, &f) in a.crown.iter().zip(&a.crown_labels) {
            if is_front(f, up) {
                p.push(*q);
                l.push(f);
            }
        }
        (p, l)
    };
    let (pl, ll) = pick(&c.lower, false);
    let (pu, lu) = pick(&c.upper, true);
    let (nl, nu) = (pl.len(), pu.len());
    Front::new(pl, ll, vec![r; nl], &pu, lu, vec![r; nu], &c.rel, k)
}

#[test]
fn the_closings_match_the_plugin() {
    let (c, fx) = case();
    let m = Some(&c.mask);
    let vert = closing(
        &c.sl,
        &c.su,
        &c.rel,
        BiteClosing::Vertical,
        None,
        0.1,
        m,
        None,
    );
    closing_check("vertical", &vert, &fx["closing"]["vertical"], TOL);
    let hinge = closing(
        &c.sl,
        &c.su,
        &c.rel,
        BiteClosing::Hinge,
        Some(c.axis),
        0.1,
        m,
        None,
    );
    closing_check("hinge", &hinge, &fx["closing"]["hinge"], TOL);
    let teeth = closing(
        &c.bare_l,
        &c.bare_u,
        &c.rel,
        BiteClosing::Hinge,
        Some(c.axis),
        0.1,
        m,
        None,
    );
    closing_check("teeth", &teeth, &fx["teeth_closing"], TOL);

    // the front teeth in 3D
    let front = front_of(&c, 0.3, 1.0);
    for (v, w) in [-0.05, 0.0, 0.05]
        .iter()
        .zip(fx["front"]["worst"].as_array().unwrap())
    {
        close(
            "front worst",
            front.worst(BiteClosing::Hinge, *v, Some(c.axis)).unwrap(),
            num(w),
            TOL_FRONT,
        );
    }
    let cf = closing(
        &c.sl,
        &c.su,
        &c.rel,
        BiteClosing::Hinge,
        Some(c.axis),
        0.1,
        m,
        Some(&front),
    );
    closing_check("front", &cf, &fx["closing_front"], TOL_FRONT);

    // first contact and the poses
    let q: Vec<DVec3> = c
        .upper
        .crown
        .iter()
        .map(|p| c.rel.transform_point3(*p))
        .step_by(7)
        .collect();
    let fc = &fx["first_contact"];
    close(
        "first_contact vertical",
        first_contact(
            &c.sl.grid,
            &c.sl.z,
            &q,
            BiteClosing::Vertical,
            None,
            None,
            50,
            None,
        )
        .unwrap(),
        num(&fc["vertical"]),
        TOL,
    );
    close(
        "first_contact hinge",
        first_contact(
            &c.sl.grid,
            &c.sl.z,
            &q,
            BiteClosing::Hinge,
            Some(c.axis),
            None,
            50,
            None,
        )
        .unwrap(),
        num(&fc["hinge"]),
        TOL,
    );
    let pts: Vec<DVec3> = fx["pose"]["q"]
        .as_array()
        .unwrap()
        .iter()
        .map(vec3)
        .collect();
    for (key, model, v, ax) in [
        ("vertical", BiteClosing::Vertical, 0.3, None),
        ("hinge", BiteClosing::Hinge, 0.05, Some(c.axis)),
    ] {
        for (got, want) in pose(&pts, model, v, ax)
            .iter()
            .zip(fx["pose"][key].as_array().unwrap())
        {
            assert!((*got - vec3(want)).length() < 1e-9, "{key}");
        }
    }
    let inc = vec3(&fx["hinge"]["incisal"]);
    for (v, w) in [0.0, 0.02, -0.03]
        .iter()
        .zip(fx["opening_at"].as_array().unwrap())
    {
        close(
            "opening_at",
            opening_at(inc, BiteClosing::Hinge, *v, Some(c.axis)),
            num(w),
            TOL,
        );
    }
    for row in fx["solve_thickness"].as_array().unwrap() {
        let r: Vec<f64> = row.as_array().unwrap().iter().map(num).collect();
        close(
            "solve_thickness",
            solve_thickness(r[0], r[1], r[2], r[3], r[4]),
            r[5],
            TOL,
        );
    }
}

#[test]
fn the_equilibration_matches_the_plugin() {
    let (c, fx) = case();
    let probes: Vec<usize> = fx["params"]["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let m = Some(&c.mask);
    for (key, model, ax) in [
        ("equilibrate_vertical", BiteClosing::Vertical, None),
        ("equilibrate_hinge", BiteClosing::Hinge, Some(c.axis)),
    ] {
        let e = equilibrate(&c.sl, &c.su, &c.rel, model, ax, 0.1, 2, 0.3, m, None);
        let w = &fx[key];
        assert_eq!(e.done, w["done"].as_bool().unwrap(), "{key}");
        assert_eq!(e.steps as u64, w["steps"].as_u64().unwrap(), "{key} steps");
        assert_eq!(e.stop, w["stop"].as_str().unwrap(), "{key} stop");
        close(
            &format!("{key} value"),
            e.value.unwrap(),
            num(&w["value"]),
            TOL,
        );
        stats(
            &format!("{key} grind_l"),
            &probes,
            &e.grind_l,
            &w["grind_l"],
        );
        stats(
            &format!("{key} grind_u"),
            &probes,
            &e.grind_u,
            &w["grind_u"],
        );
        closing_check(&format!("{key} closing"), &e.closing, &w["closing"], TOL);
        closing_check(&format!("{key} before"), &e.before, &w["before"], TOL);
    }
    // with the front teeth: the front stops it as in the plugin
    let front = front_of(&c, 0.3, 1.0);
    let e = equilibrate(
        &c.sl,
        &c.su,
        &c.rel,
        BiteClosing::Hinge,
        Some(c.axis),
        0.1,
        2,
        0.3,
        m,
        Some(&front),
    );
    let w = &fx["equilibrate"];
    assert_eq!(e.done, w["done"].as_bool().unwrap());
    assert_eq!(e.steps as u64, w["steps"].as_u64().unwrap());
    assert_eq!(e.stop, w["stop"].as_str().unwrap());
}

// ---------------------------------------------------------------------------------------------
// ramps
// ---------------------------------------------------------------------------------------------

#[test]
fn the_ramp_shape_matches_the_plugin() {
    let fx = read("aligner_bite.json");
    let w = &fx["ramp_shape"];
    let (v, t) = ramps::ramp_mesh(ramps::N_AROUND, ramps::N_UP);
    assert_eq!(v.len() as u64, w["n_vertices"].as_u64().unwrap());
    assert_eq!(t.len() as u64, w["n_tris"].as_u64().unwrap());
    for (g, want) in v.iter().zip(w["vertices"].as_array().unwrap()) {
        assert!((*g - vec3(want)).length() < 1e-9);
    }
    for (g, want) in t.iter().zip(w["tris"].as_array().unwrap()) {
        let a: Vec<u64> = want
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        assert_eq!(g.map(u64::from).to_vec(), a);
    }
    for f in w["frames"].as_array().unwrap() {
        let toward = (!f["toward"].is_null()).then(|| vec3(&f["toward"]));
        let m = ramps::frame(vec3(&f["origin"]), vec3(&f["normal"]), toward);
        let want = mat4(&f["m"]);
        for (a, b) in m.to_cols_array().iter().zip(want.to_cols_array()) {
            assert!((a - b).abs() < 1e-9, "{m:?} {want:?}");
        }
    }
    let occ = read("occlusion.json");
    let (lower, upper) = (arch(&occ["lower"]), arch(&occ["upper"]));
    let rel = mat4(&occ["relation"]);
    let lo: Vec<DVec3> = pick(&lower, &[31, 41]);
    let up: Vec<DVec3> = pick(&upper, &[11, 21])
        .iter()
        .map(|p| rel.transform_point3(*p))
        .collect();
    close(
        "overbite",
        ramps::overbite(&lo, &up).unwrap(),
        num(&w["overbite"][0]),
        TOL,
    );
    assert!(ramps::overbite(&[], &[DVec3::ZERO]).is_none());
}

fn pick(a: &Arch, ids: &[i64]) -> Vec<DVec3> {
    a.crown
        .iter()
        .zip(&a.crown_labels)
        .filter(|(_, l)| ids.contains(l))
        .map(|(p, _)| *p)
        .collect()
}

/// Crown points of every tooth thinned to one per 0.2 mm cell.
fn crowns(a: &Arch) -> BTreeMap<i64, Vec<DVec3>> {
    let mut ids: Vec<i64> = a.crown_labels.clone();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|f| {
            (
                f,
                dca_algo::aligner_bite::occlusal::crown_cells(&pick(a, &[f]), 0.2),
            )
        })
        .collect()
}

#[test]
fn the_proposed_ramps_are_where_the_plugin_puts_them() {
    let fx = read("aligner_bite.json");
    let occ = read("occlusion.json");
    let (lower, upper) = (arch(&occ["lower"]), arch(&occ["upper"]));
    let rel0 = mat4(&occ["relation"]);
    let (cl, cu) = (crowns(&lower), crowns(&upper));
    let teeth: BTreeMap<i64, TriMesh> = fx["propose"]["incisors"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(f, rec)| {
            let v = rec["vertices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| vec3(p).as_vec3())
                .collect();
            let t = rec["tris"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    let a: Vec<u32> = t
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_u64().unwrap() as u32)
                        .collect();
                    [a[0], a[1], a[2]]
                })
                .collect();
            (f.parse().unwrap(), TriMesh::new(v, t))
        })
        .collect();
    let proposals: BTreeMap<i64, ProposeTooth<'_>> = teeth
        .iter()
        .map(|(f, m)| {
            (
                *f,
                ProposeTooth {
                    mesh: m,
                    to_arch: DMat4::IDENTITY,
                },
            )
        })
        .collect();
    let mut chosen = Vec::new();
    for case in fx["propose"]["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let covered: Vec<i64> = case["covered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let mut rel = rel0;
        rel.w_axis.z += num(&case["shift"]);
        let input = ProposeInput {
            lower_crowns: &cl,
            upper_crowns: &cu,
            rel,
            upper_teeth: &proposals,
            covered: &covered,
            clearance: 0.0,
            wall: 1.0,
        };
        let settings = RampSettings {
            overbite_min_mm: num(&case["overbite_min"]),
            ..Default::default()
        };
        let got = ramps::propose(&input, &settings);
        match &case["overbite"] {
            Value::Null => assert!(got.overbite_mm.is_none(), "{name}"),
            w => close(
                &format!("{name} overbite"),
                got.overbite_mm.unwrap(),
                num(w),
                TOL,
            ),
        }
        assert_eq!(got.reason, case["why"].as_str().unwrap(), "{name}");
        let want = case["ramps"].as_array().unwrap();
        assert_eq!(got.ramps.len(), want.len(), "{name}");
        for (g, w) in got.ramps.iter().zip(want) {
            assert_eq!(i64::from(g.fdi.get()), w["fdi"].as_i64().unwrap(), "{name}");
            let m = ramps::matrix(g);
            let size = [num(&w["size"][0]), num(&w["size"][1]), num(&w["size"][2])];
            assert_eq!(g.size_mm, size);
            // the plugin's M_local is the rigid 4x4 (row-major); the ramp holds it as a pose
            let wm = mat4(&w["m"]);
            let rigid = m * glam::DMat4::from_scale(DVec3::new(
                1.0 / size[0],
                1.0 / size[1],
                1.0 / size[2],
            ));
            for (a, b) in rigid.to_cols_array().iter().zip(wm.to_cols_array()) {
                assert!((a - b).abs() < 1e-9, "{name}: {a} {b}");
            }
        }
        chosen.push((
            name.to_owned(),
            got.ramps.iter().map(|r| r.fdi.get()).collect::<Vec<_>>(),
        ));
    }
    assert_eq!(chosen[0].1, vec![11, 21, 12, 22]);
    assert_eq!(chosen[1].1, vec![21, 12, 22]);
    assert!(chosen[2].1.is_empty());
}
