//! Numerical comparison of the foundations of `dca-algo` (anatomy, archform,
//! evidence, frame, demo) with the Python plugin.
//!
//! `tests/fixtures/foundations.json` is produced by `gen_foundations.py` from
//! the plugin's own modules (python3 + numpy, no Blender). The demo generator
//! is replayed with the same random draws as the plugin, so the meshes can be
//! compared vertex by vertex.

use std::path::PathBuf;

use dc_case::{Fdi, Pose};
use dca_algo::anatomy::{self, Arch, Surface};
use dca_algo::archform::{self, ArchCurve, ArchModel};
use dca_algo::demo::{self, Draws, Severity};
use dca_algo::{evidence, frame, rotation};
use dca_model::Seed;
use glam::{DMat3, DMat4, DVec2, DVec3};
use serde_json::Value;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foundations.json");
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

fn vec2(v: &Value) -> DVec2 {
    let n = nums(v);
    DVec2::new(n[0], n[1])
}

fn fdi(n: u64) -> Fdi {
    Fdi::new(n as u8).unwrap()
}

#[track_caller]
fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!(
        (a - b).abs() <= tol * (1.0 + b.abs()),
        "{what}: {a} against {b} (tolerance {tol})"
    );
}

#[track_caller]
fn close_all(a: &[f64], b: &[f64], tol: f64, what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: length");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        close(*x, *y, tol, &format!("{what}[{i}]"));
    }
}

/// Row-major 4x4 matrix of the fixture (numpy convention) as a `DMat4`.
fn mat4(v: &Value) -> DMat4 {
    let rows: Vec<Vec<f64>> = v.as_array().unwrap().iter().map(nums).collect();
    DMat4::from_cols_array_2d(&std::array::from_fn(|c| {
        std::array::from_fn(|r| rows[r][c])
    }))
}

fn mat3(v: &Value) -> DMat3 {
    let rows: Vec<Vec<f64>> = v.as_array().unwrap().iter().map(nums).collect();
    DMat3::from_cols_array_2d(&std::array::from_fn(|c| {
        std::array::from_fn(|r| rows[r][c])
    }))
}

fn fdi_list(v: &Value) -> Vec<Fdi> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| fdi(x.as_u64().unwrap()))
        .collect()
}

#[test]
fn anatomy_matches_the_plugin() {
    let fx = fixture();
    let a = &fx["anatomy"];
    for (name, arch) in [("UPPER", Arch::Upper), ("LOWER", Arch::Lower)] {
        assert_eq!(
            anatomy::arch_order(arch).to_vec(),
            fdi_list(&a["order"][name])
        );
    }
    for (key, t) in a["teeth"].as_object().unwrap() {
        let f = Fdi::new(key.parse().unwrap()).unwrap();
        let what = |s: &str| format!("{key} {s}");
        let arch = if t["arch"] == "UPPER" {
            Arch::Upper
        } else {
            Arch::Lower
        };
        assert_eq!(anatomy::arch_of(f), arch, "{key}");
        assert_eq!(anatomy::is_right(f), t["right"].as_bool().unwrap(), "{key}");
        assert_eq!(
            anatomy::is_anterior(f),
            t["anterior"].as_bool().unwrap(),
            "{key}"
        );
        assert_eq!(
            anatomy::is_incisor(f),
            t["incisor"].as_bool().unwrap(),
            "{key}"
        );
        assert_eq!(
            anatomy::is_round_crown(f),
            t["round"].as_bool().unwrap(),
            "{key}"
        );
        assert_eq!(anatomy::is_molar(f), t["molar"].as_bool().unwrap(), "{key}");
        assert_eq!(anatomy::tooth_name(f), t["name"].as_str().unwrap(), "{key}");
        let (c, r) = anatomy::crown_root_length(f);
        close_all(&[c, r], &nums(&t["crown_root"]), 0.0, &what("crown_root"));
        let (md, bl) = anatomy::typical_diameters(f);
        close_all(&[md, bl], &nums(&t["diameters"]), 0.0, &what("diameters"));
        let enamel = [
            anatomy::enamel_thickness(f, Surface::Mesial),
            anatomy::enamel_thickness(f, Surface::Distal),
        ];
        close_all(&enamel, &nums(&t["enamel"]), 0.0, &what("enamel"));
        let ipr = [
            anatomy::max_ipr_surface_default(f, Surface::Mesial),
            anatomy::max_ipr_surface_default(f, Surface::Distal),
        ];
        close_all(&ipr, &nums(&t["ipr"]), 1e-15, &what("ipr"));
        close(
            anatomy::max_ipr_surface(f, Surface::Distal, 0.25, 0.2),
            num(&t["ipr_tight"]),
            1e-15,
            &what("ipr_tight"),
        );
    }
    for s in a["sequences"].as_array().unwrap() {
        let arch = if s["arch"] == "UPPER" {
            Arch::Upper
        } else {
            Arch::Lower
        };
        let seq = anatomy::arch_sequence(&fdi_list(&s["teeth"]), arch);
        assert_eq!(seq, fdi_list(&s["seq"]));
        for f in &seq {
            let key = f.get().to_string();
            let want = |v: &Value| v.as_u64().map(fdi);
            assert_eq!(
                anatomy::mesial_neighbor(&seq, *f),
                want(&s["mesial"][&key]),
                "mesial {key}"
            );
            assert_eq!(
                anatomy::distal_neighbor(&seq, *f),
                want(&s["distal"][&key]),
                "distal {key}"
            );
        }
    }
}

fn check_curve(name: &str, c: &ArchCurve, o: &Value) {
    assert_eq!(
        c.points.len() as u64,
        o["n"].as_u64().unwrap(),
        "{name}: points"
    );
    let at = |i: usize| c.points[i];
    close_all(
        &at(0).to_array(),
        &nums(&o["first"]),
        1e-12,
        &format!("{name} first"),
    );
    close_all(
        &at(c.points.len() - 1).to_array(),
        &nums(&o["last"]),
        1e-12,
        &format!("{name} last"),
    );
    close_all(
        &at(c.points.len() / 2).to_array(),
        &nums(&o["mid"]),
        1e-12,
        &format!("{name} mid"),
    );
    close(c.s[0], num(&o["s_first"]), 1e-9, &format!("{name} s_first"));
    close(
        *c.s.last().unwrap(),
        num(&o["s_last"]),
        1e-9,
        &format!("{name} s_last"),
    );
    let ss = nums(&o["s"]);
    let (pts, tan, nor) = (&o["points"], &o["tangent"], &o["normal"]);
    for (i, s) in ss.iter().enumerate() {
        let what = |k: &str| format!("{name} {k} s={s}");
        close_all(
            &c.point(*s).to_array(),
            &nums(&pts[i]),
            1e-9,
            &what("point"),
        );
        close_all(
            &c.tangent(*s, 0.05).to_array(),
            &nums(&tan[i]),
            1e-7,
            &what("tangent"),
        );
        close_all(
            &c.normal_out(*s).to_array(),
            &nums(&nor[i]),
            1e-7,
            &what("normal"),
        );
    }
    for (p, want) in o["probe"]
        .as_array()
        .unwrap()
        .iter()
        .zip(nums(&o["project"]))
    {
        close(
            c.project(vec2(p)),
            want,
            1e-9,
            &format!("{name} project {p}"),
        );
    }
    for ch in o["chords"].as_array().unwrap() {
        let [s0, chord, dir, want] = nums(ch)[..] else {
            panic!()
        };
        close(
            c.s_at_chord(s0, chord, dir),
            want,
            1e-9,
            &format!("{name} chord {s0} {chord}"),
        );
    }
}

#[test]
fn archform_matches_the_plugin() {
    let fx = fixture();
    let a = &fx["archform"];
    let xs = nums(&a["beta_x"]);
    let beta: Vec<f64> = xs
        .iter()
        .map(|x| archform::beta_y(*x, 50.0, 35.0))
        .collect();
    close_all(&beta, &nums(&a["beta_y"]), 1e-12, "beta_y");
    let quartic: Vec<f64> = xs
        .iter()
        .map(|x| archform::quartic_y(*x, [30.0, -0.04, 1.5e-6]))
        .collect();
    close_all(&quartic, &nums(&a["quartic_y"]), 1e-12, "quartic_y");

    let xy: Vec<DVec2> = a["fit_xy"].as_array().unwrap().iter().map(vec2).collect();
    let w = nums(&a["fit_w"]);
    close_all(
        &archform::fit_quartic(&xy, None),
        &nums(&a["fit"]),
        1e-6,
        "fit",
    );
    close_all(
        &archform::fit_quartic(&xy, Some(&w)),
        &nums(&a["fit_weighted"]),
        1e-6,
        "fit weighted",
    );
    close_all(
        &archform::fit_quartic(&[DVec2::new(1.0, 2.0), DVec2::new(2.0, 5.0)], None),
        &nums(&a["fit_two"]),
        1e-9,
        "fit two points",
    );
    close_all(
        &archform::fit_quartic(&[DVec2::new(2.0, 4.0)], None),
        &nums(&a["fit_one"]),
        1e-9,
        "fit one point",
    );
    close_all(
        &archform::quartic_through(40.0, 8.0, 36.0, 25.0, 5.0),
        &nums(&a["through"]),
        1e-9,
        "through",
    );
    close_all(
        &archform::quartic_through(10.0, 0.0, 5.0, 0.0, 4.0),
        &nums(&a["through_singular"]),
        1e-9,
        "through singular",
    );
    let pts: Vec<DVec2> = a["even_points"]
        .as_array()
        .unwrap()
        .iter()
        .map(vec2)
        .collect();
    let coef = archform::poly_even_through(&pts);
    close_all(&coef, &nums(&a["even_coef"]), 1e-9, "even coef");
    let ys: Vec<f64> = xs
        .iter()
        .map(|x| archform::poly_even_y(*x, &coef))
        .collect();
    close_all(&ys, &nums(&a["even_y"]), 1e-9, "even y");

    let c = &a["curves"];
    let model = |name: &str, m: ArchModel, half: f64, post: bool| {
        check_curve(name, &archform::curve_from_model(&m, half, post), &c[name]);
    };
    model(
        "beta",
        ArchModel::Beta {
            w: 50.0,
            d: 33.0,
            y0: 0.0,
        },
        25.0,
        false,
    );
    model(
        "quartic",
        ArchModel::Quartic([31.0, -0.032, 1.1e-6]),
        26.0,
        false,
    );
    model(
        "quartic_posterior",
        ArchModel::Quartic([31.0, -0.032, 1.1e-6]),
        26.0,
        true,
    );
    model("even_posterior", ArchModel::Even(coef), 28.0, true);
}

#[test]
fn evidence_matches_the_plugin() {
    let fx = fixture();
    let e = &fx["evidence"];
    assert_eq!(
        evidence::REFERENCES.len() as u64,
        e["n_refs"].as_u64().unwrap()
    );
    assert_eq!(
        evidence::PARAMS.len() as u64,
        e["n_params"].as_u64().unwrap()
    );
    let keys = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_owned())
            .collect()
    };
    let ref_keys: Vec<String> = evidence::REFERENCES
        .iter()
        .map(|r| r.key.to_owned())
        .collect();
    assert_eq!(ref_keys, keys(&e["ref_keys"]));
    let param_keys: Vec<String> = evidence::PARAMS.iter().map(|p| p.key.to_owned()).collect();
    assert_eq!(param_keys, keys(&e["param_keys"]));

    for r in evidence::REFERENCES {
        assert_eq!(
            r.short(),
            e["short"][r.key].as_str().unwrap(),
            "short {}",
            r.key
        );
        assert_eq!(
            r.full(),
            e["full"][r.key].as_str().unwrap(),
            "full {}",
            r.key
        );
    }
    for p in evidence::PARAMS {
        assert_eq!(
            p.tooltip(),
            e["tooltips"][p.key].as_str().unwrap(),
            "tooltip {}",
            p.key
        );
        assert_eq!(
            p.unit,
            e["units"][p.key].as_str().unwrap(),
            "unit {}",
            p.key
        );
        assert_eq!(keys(&e["refs"][p.key]), p.refs, "refs {}", p.key);
        let want = &e["values"][p.key];
        match p.value {
            evidence::ParamValue::Float(v) => close(v, num(want), 0.0, p.key),
            evidence::ParamValue::Int(v) => assert_eq!(v, want.as_i64().unwrap(), "{}", p.key),
            evidence::ParamValue::Map(m) => {
                let o = want.as_object().unwrap();
                assert_eq!(m.len(), o.len(), "{}", p.key);
                for (k, v) in m {
                    close(*v, num(&o[*k]), 0.0, p.key);
                }
            }
        }
    }
    let confirm: Vec<String> = evidence::PARAMS
        .iter()
        .filter(|p| p.to_confirm)
        .map(|p| p.key.to_owned())
        .collect();
    assert_eq!(confirm, keys(&e["to_confirm"]));
    assert_eq!(
        evidence::cite(&["simon2014", "castroflorio2023", "wang2025consensus"]),
        e["cite"].as_str().unwrap()
    );

    let text = evidence::references_text();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len() as u64, e["text_lines"].as_u64().unwrap());
    assert_eq!(
        text.chars().count() as u64,
        e["text_chars"].as_u64().unwrap()
    );
    let want: Vec<&str> = e["text_head"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(lines[..want.len()], want[..]);
    let want: Vec<&str> = e["text_tail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(lines[lines.len() - want.len()..], want[..]);
}

#[test]
fn frame_helpers_match_the_plugin() {
    let fx = fixture();
    let f = &fx["frame"];

    let cloud: Vec<DVec3> = f["pca"]["cloud"]
        .as_array()
        .unwrap()
        .iter()
        .map(vec3)
        .collect();
    let p = frame::pca(&cloud).unwrap();
    close_all(
        &p.mean.to_array(),
        &nums(&f["pca"]["mean"]),
        1e-10,
        "pca mean",
    );
    close_all(
        &p.eigenvalues,
        &nums(&f["pca"]["eigenvalues"]),
        1e-9,
        "pca eigenvalues",
    );
    for (i, axis) in f["pca"]["axes"].as_array().unwrap().iter().enumerate() {
        // numpy's SVD leaves the sign of each axis arbitrary
        let d = p.axes[i].dot(vec3(axis)).abs();
        assert!((d - 1.0).abs() < 1e-9, "pca axis {i}: |dot| {d}");
    }

    let r = &f["rot_axis_angle"];
    let m = frame::rot_axis_angle(vec3(&r["axis"]), num(&r["angle"]));
    assert!(m.abs_diff_eq(mat3(&r["matrix"]), 1e-12));
    let rv = &f["rotvec"];
    close_all(
        &rotation::matrix_to_rotvec(&mat3(&rv["matrix"])).to_array(),
        &nums(&rv["vec"]),
        1e-9,
        "rotvec",
    );
    close_all(
        &rotation::matrix_to_rotvec(&frame::rot_axis_angle(DVec3::Z, std::f64::consts::PI))
            .to_array(),
        &nums(&rv["vec_pi"]),
        1e-9,
        "rotvec pi",
    );
    let ri = &f["rigid_inverse"];
    let frame_m = frame::make_frame(DVec3::Y, DVec3::Z, DVec3::X, DVec3::new(1.0, 2.0, 3.0));
    assert!(frame_m.abs_diff_eq(mat4(&ri["m"]), 1e-12));
    assert!(frame::rigid_inverse(&frame_m).abs_diff_eq(mat4(&ri["inv"]), 1e-12));

    for case in f["cases"].as_array().unwrap() {
        let arch = if case["arch"] == "UPPER" {
            Arch::Upper
        } else {
            Arch::Lower
        };
        let seeds: Vec<Seed> = case["seeds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Seed {
                fdi: fdi(s[0].as_u64().unwrap()),
                position: vec3(&s[1]),
            })
            .collect();
        let fr = frame::arch_frame_from_seeds(&seeds, vec3(&case["vertices_mean"])).unwrap();
        let want = mat4(&case["frame"]);
        assert!(
            fr.matrix().abs_diff_eq(want, 1e-9),
            "{arch:?} frame\n{:?}\n{want:?}",
            fr.matrix()
        );
        let centre = frame::arch_centre(&fr, &seeds);
        close_all(&centre.to_array(), &nums(&case["centre"]), 1e-9, "centre");
        let delta = frame::world_delta(&fr, arch, centre);
        let want = mat4(&case["delta"]);
        assert!(
            delta.to_mat4().abs_diff_eq(want, 1e-9),
            "{arch:?} delta\n{:?}\n{want:?}",
            delta.to_mat4()
        );
        assert_eq!(
            frame::is_identity(&delta),
            case["identity"].as_bool().unwrap()
        );
    }

    for item in f["is_identity"].as_array().unwrap() {
        let pose = Pose::from_mat4(&mat4(&item[0]));
        assert_eq!(
            frame::is_identity(&pose),
            item[1].as_bool().unwrap(),
            "{pose:?}"
        );
    }
}

/// Replays the draws recorded by the fixture: `U` in `[0, 1)` for uniform
/// draws, `Z` standard normal for the normal ones.
struct Replay {
    u: Vec<f64>,
    z: Vec<f64>,
    iu: usize,
    iz: usize,
}

impl Draws for Replay {
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        self.iu += 1;
        lo + (hi - lo) * self.u[self.iu - 1]
    }

    fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        self.iz += 1;
        mean + sd * self.z[self.iz - 1]
    }
}

/// Compares a mesh given as double-precision points with the fixture summary.
fn check_summary(name: &str, v: &[DVec3], t: &[[u32; 3]], o: &Value, tol: f64) {
    assert_eq!(
        v.len() as u64,
        o["n_vertices"].as_u64().unwrap(),
        "{name}: vertices"
    );
    assert_eq!(
        t.len() as u64,
        o["n_tris"].as_u64().unwrap(),
        "{name}: triangles"
    );
    let tri_sum: u64 = t.iter().flatten().map(|i| u64::from(*i)).sum();
    assert_eq!(tri_sum, o["tri_sum"].as_u64().unwrap(), "{name}: tri_sum");
    let tri = |v: &Value| -> [u32; 3] {
        let n = v.as_array().unwrap();
        std::array::from_fn(|i| num(&n[i]) as u32)
    };
    assert_eq!(t[0], tri(&o["tri_first"]), "{name}");
    assert_eq!(*t.last().unwrap(), tri(&o["tri_last"]), "{name}");
    let sum: DVec3 = v.iter().copied().sum();
    close_all(
        &sum.to_array(),
        &nums(&o["vertex_sum"]),
        tol.max(1e-9) * 50.0,
        &format!("{name} sum"),
    );
    let min = v.iter().copied().reduce(DVec3::min).unwrap();
    let max = v.iter().copied().reduce(DVec3::max).unwrap();
    close_all(
        &min.to_array(),
        &nums(&o["min"]),
        tol,
        &format!("{name} min"),
    );
    close_all(
        &max.to_array(),
        &nums(&o["max"]),
        tol,
        &format!("{name} max"),
    );
    let step = o["step"].as_u64().unwrap() as usize;
    for (k, want) in o["samples"].as_array().unwrap().iter().enumerate() {
        close_all(
            &v[k * step].to_array(),
            &nums(want),
            tol,
            &format!("{name} vertex {}", k * step),
        );
    }
}

#[test]
fn demo_matches_the_plugin() {
    let fx = fixture();
    let d = &fx["demo"];

    // single shells, in double precision
    for (key, o) in d["teeth"].as_object().unwrap() {
        let f = Fdi::new(key.parse().unwrap()).unwrap();
        let (v, t, top) = demo::tooth_mesh(f, 48);
        check_summary(&format!("tooth {key}"), &v, &t, o, 1e-9);
        close_all(
            &top.to_array(),
            &nums(&o["top"]),
            1e-9,
            &format!("tooth {key} top"),
        );
    }
    let lower: Vec<Fdi> = anatomy::arch_order(Arch::Lower)
        .into_iter()
        .filter(|f| anatomy::index(*f) != 8)
        .collect();
    let (curve, w) = demo::ideal_arch(Arch::Lower, &lower);
    assert_eq!(w, num(&d["ideal_lower"]["w"]));
    let s = [-30.0, -10.0, 0.0, 17.0, 40.0];
    for (s, want) in s.iter().zip(d["ideal_lower"]["points"].as_array().unwrap()) {
        close_all(
            &curve.point(*s).to_array(),
            &nums(want),
            1e-9,
            "ideal lower",
        );
    }
    assert_eq!(
        curve.points.len() as u64,
        d["ideal_lower"]["n"].as_u64().unwrap()
    );
    let (cu, wu) = demo::ideal_arch(Arch::Upper, &anatomy::arch_order(Arch::Upper));
    assert_eq!(wu, num(&d["ideal_upper_all"]["w"]));
    for (s, want) in s
        .iter()
        .zip(d["ideal_upper_all"]["points"].as_array().unwrap())
    {
        close_all(&cu.point(*s).to_array(), &nums(want), 1e-9, "ideal upper");
    }
    let (vg, tg) = demo::gum_mesh(&curve, -33.0, 31.0, 0.5, 6.8, 12.0);
    check_summary("gum", &vg, &tg, &d["gum"], 1e-9);

    // the whole model with the plugin's draws
    for run in d["runs"].as_array().unwrap() {
        let arch = if run["arch"] == "UPPER" {
            Arch::Upper
        } else {
            Arch::Lower
        };
        let severity = match run["severity"].as_str().unwrap() {
            "MILD" => Severity::Mild,
            "MODERATE" => Severity::Moderate,
            _ => Severity::Severe,
        };
        let third = run["third_molars"].as_bool().unwrap();
        let name = format!("{arch:?} {severity:?} third={third}");
        let mut replay = Replay {
            u: nums(&d["U"]),
            z: nums(&d["Z"]),
            iu: 0,
            iz: 0,
        };
        let model = demo::generate_with(arch, severity, third, &mut replay);
        let mesh = &model.mesh;
        let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        // positions are stored in single precision
        check_summary(&name, &v, &mesh.tris, run, 2e-4);

        // shells in the plugin's order (right side then left), then the gum
        let labels = mesh.vertex_labels.as_ref().unwrap();
        let mut order: Vec<u16> = Vec::new();
        for l in labels {
            if order.last() != Some(l) {
                order.push(*l);
            }
        }
        let mut want: Vec<u16> = run["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_u64().unwrap() as u16)
            .collect();
        want.push(0);
        assert_eq!(order, want, "{name}: shells");
    }
}
