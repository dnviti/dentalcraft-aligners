//! Numerical comparison of `dca_algo::bite` with the Python plugin
//! (`ortho_aligner/bite.py`: `register`, `kabsch_init`,
//! `correct_penetration`; operators `oa.register_bite`, `oa.bite_points`,
//! `oa.bite_correct`).
//!
//! `tests/fixtures/bite.json` is produced by `gen_bite.py` (Blender, the
//! plugin's own code read-only) on the plugin's demo arches (16 points per
//! ring of a tooth, gum step 1 mm, occlusal plane at z = 0), the lower one in
//! a case frame, the upper one displaced from the true bite (0.3 mm under
//! load), and two buccal bite scans cut from the arches in the true bite by
//! the plugin's own simulator (`tests/bitesim.py`, canine to first molar,
//! noise 0.02 mm, the left one wound inwards, in the frame of a scanner).
//! The plugin's own registration lands 0.02-0.03 mm / 0.03 degrees from the
//! true bite on it. Scenarios: two scans, one scan, 3 points on the lower
//! arch, 3 points on both arches (the plugin's scenario D, 1 mm picking
//! error), and the correction of the interpenetration after the first. The
//! port is fed the same single-precision vertices.
//!
//! The manual pairing from 3 points (`kabsch_init`) is a closed form and
//! matches the plugin's within 1e-9 mm (measured 2.6e-13). The correction
//! started from the plugin's registered pose opens the bite within 1e-5 mm
//! of the plugin's (measured 1.9e-6: the port's rays are single precision).
//! The registrations do not match to 1e-6, and cannot:
//! the plugin's own result is not stable to 1e-6 mm. Its closest points are
//! exact float64 queries; the port's run on single-precision trees
//! (`dc_geom::bvh`: the query point is rounded to float32, about 2e-6 mm
//! here), so at a convex edge of the arch, where two faces are at the same
//! distance, rounding picks the face, and with it the normal of the
//! point-to-plane ICP and of the facing test of the metrics. The plugin
//! itself, fed the bite points moved by 1e-6 mm (30 seeds for the 3-point
//! cases, 5 for the others), moves the upper arch by up to 5.9e-3 mm /
//! 6.8e-3 degrees with one scan (3.6e-4 mm with two), its areas by up to
//! 0.5 mm², its ICP overlap by 0.006, its agreement by 0.004 mm, its
//! interpenetration and opening by 0.001 mm and its contacts by 1 (one seed
//! of the one-scan case even lands 0.065 mm away: its coarse search keeps
//! another start). Its fine ICP, started from the port's coarse pose, ends
//! within 2e-6 mm of the port's. The bounds sit just above that measured
//! spread, not tuned to the port: the moves within [`TOL_MM`] / [`TOL_DEG`]
//! ([`TOL_TWO_MM`] / [`TOL_TWO_DEG`] with two scans and after the
//! correction; largest displacement of the upper crown points, or of the
//! bite scan points), the areas within [`TOL_AREA`], the other figures
//! within the bounds of [`same_fit`] and [`same_report`]; the teeth, the
//! counts of teeth, the verdict and the reasons equal.

use std::path::PathBuf;

use dca_algo::anatomy::Arch;
use dca_algo::bite::{self, BiteArch, BiteScan, FitMetrics, ManualInit, Registration};
use dca_algo::frame;
use dca_algo::occlusion::ArchModel;
use glam::{DMat3, DMat4, DVec3};
use serde_json::Value;

/// Largest displacement between the port's and the plugin's moves with one
/// bite scan, automatic or from 3 points (mm). The plugin's own spread: up
/// to 5.9e-3 mm.
const TOL_MM: f64 = 6e-3;
/// Largest rotation between those moves (degrees; plugin: up to 6.8e-3).
const TOL_DEG: f64 = 7e-3;
/// Largest displacement with two bite scans, fitted together at the end, and
/// after the correction (mm; plugin: up to 4.2e-4).
const TOL_TWO_MM: f64 = 1e-3;
/// Largest rotation with two bite scans (degrees; plugin: up to 1.3e-3).
const TOL_TWO_DEG: f64 = 2e-3;
/// Largest difference of an area of the metrics (mm², rounded to 0.1;
/// plugin: up to 0.7).
const TOL_AREA: f64 = 0.7;

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

fn f32s(s: &str) -> Vec<DVec3> {
    base64(s)
        .as_chunks::<12>()
        .0
        .iter()
        .map(|b| {
            let f =
                |k: usize| f64::from(f32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap()));
            DVec3::new(f(0), f(1), f(2))
        })
        .collect()
}

/// Row-major 4x4 (numpy `ravel`) to a glam matrix.
fn mat4(v: &Value) -> DMat4 {
    let a: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    DMat4::from_cols_array(&a.try_into().unwrap()).transpose()
}

fn points(v: &Value) -> Vec<DVec3> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_array().unwrap();
            DVec3::new(num(&p[0]), num(&p[1]), num(&p[2]))
        })
        .collect()
}

fn arch(c: &Value, arch: Arch, to_case: DMat4) -> ArchModel {
    let vertices = f32s(c["vertices"].as_str().unwrap());
    let tris: Vec<[u32; 3]> = base64(c["tris"].as_str().unwrap())
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
    ArchModel::from_arch(arch, to_case, vertices, tris, labels, Vec::new()).unwrap()
}

fn bite_scan(name: &str, rec: &Value) -> BiteScan {
    let v = f32s(rec["vertices"].as_str().unwrap());
    let tris: Vec<[u32; 3]> = base64(rec["tris"].as_str().unwrap())
        .as_chunks::<12>()
        .0
        .iter()
        .map(|b| {
            let i = |k: usize| u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap());
            [i(0), i(1), i(2)]
        })
        .collect();
    assert_eq!(v.len() as u64, rec["n_vertices"].as_u64().unwrap());
    assert_eq!(tris.len() as u64, rec["n_tris"].as_u64().unwrap());
    BiteScan::new(name, &v, &tris).unwrap()
}

struct Case {
    f: Value,
    lower: ArchModel,
    upper: ArchModel,
}

fn case() -> Case {
    let f = read("bite.json");
    let lower = arch(&f["lower"], Arch::Lower, mat4(&f["lower_frame"]));
    let upper = arch(&f["upper"], Arch::Upper, mat4(&f["upper_frame"]));
    Case { f, lower, upper }
}

/// Rotation between two moves (degrees; `atan2` of the axis and the trace,
/// accurate at small angles where `acos` of the trace is not).
fn rot_deg(a: &DMat4, b: &DMat4) -> f64 {
    let r = DMat3::from_mat4(*a) * DMat3::from_mat4(*b).transpose();
    let s = DVec3::new(
        r.y_axis.z - r.z_axis.y,
        r.z_axis.x - r.x_axis.z,
        r.x_axis.y - r.y_axis.x,
    )
    .length()
        / 2.0;
    let c = (r.x_axis.x + r.y_axis.y + r.z_axis.z - 1.0) / 2.0;
    s.atan2(c).to_degrees()
}

/// Largest displacement of `pts` between two moves, and their rotation apart.
fn apart(a: &DMat4, b: &DMat4, pts: &[DVec3]) -> (f64, f64) {
    let d = pts
        .iter()
        .map(|&p| a.transform_point3(p).distance(b.transform_point3(p)))
        .fold(0.0, f64::max);
    (d, rot_deg(a, b))
}

fn close(name: &str, got: f64, want: &Value, bound: f64) {
    let w = num(want);
    if got != w {
        eprintln!("bite oracle {name}: port {got}, plugin {w}");
    }
    assert!(
        (got - w).abs() <= bound + 1e-9,
        "{name}: port {got}, plugin {w}"
    );
}

/// The metrics of a fit against the plugin's: the areas within
/// [`TOL_AREA`], the shares and distances within their rounding and the
/// plugin's own spread (overlap 0.006, bite fraction and p95 0.001).
fn same_fit(name: &str, m: &FitMetrics, o: &Value) {
    close(
        &format!("{name} area"),
        m.area_mm2,
        &o["area_mm2"],
        TOL_AREA,
    );
    close(&format!("{name} rms"), m.rms_mm, &o["rms_mm"], 1e-3);
    close(
        &format!("{name} coherence"),
        m.coherence,
        &o["coherence"],
        1e-3,
    );
    close(
        &format!("{name} bite fraction"),
        m.bite_fraction,
        &o["bite_fraction"],
        2e-3,
    );
    close(
        &format!("{name} overlap"),
        m.icp_overlap,
        &o["icp_overlap"],
        1e-2,
    );
    close(
        &format!("{name} p95"),
        m.p95_mm.unwrap(),
        &o["p95_mm"],
        2e-3,
    );
    assert_eq!(m.n_teeth as u64, o["n_teeth"].as_u64().unwrap(), "{name}");
    let teeth: Vec<u16> = o["teeth"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.parse().unwrap())
        .collect();
    assert_eq!(m.teeth.keys().copied().collect::<Vec<_>>(), teeth, "{name}");
    for (f, a) in &m.teeth {
        close(
            &format!("{name} tooth {f}"),
            *a,
            &o["teeth"][f.to_string()],
            TOL_AREA,
        );
    }
}

/// The figures of a registration against the plugin's report.
fn same_report(name: &str, reg: &Registration, o: &Value) {
    let scans = o["scans"].as_array().unwrap();
    assert_eq!(reg.scans.len(), scans.len(), "{name}");
    for (s, os) in reg.scans.iter().zip(scans) {
        assert_eq!(s.scan, os["scan"].as_str().unwrap(), "{name}");
        assert_eq!(s.side, os["side"].as_str(), "{name} {}", s.scan);
        same_fit(&format!("{name} {} lower", s.scan), &s.lower, &os["lower"]);
        same_fit(&format!("{name} {} upper", s.scan), &s.upper, &os["upper"]);
    }
    match o["agreement_mm"].as_f64() {
        Some(a) => close(
            &format!("{name} agreement"),
            reg.agreement_mm.unwrap(),
            &a.into(),
            5e-3,
        ),
        None => assert!(reg.agreement_mm.is_none(), "{name}"),
    }
    close(
        &format!("{name} penetration"),
        reg.penetration_mm,
        &o["penetration_mm"],
        2e-3,
    );
    close(
        &format!("{name} opening"),
        reg.opening_at_contact_mm,
        &o["opening_at_contact_mm"],
        2e-3,
    );
    close(&format!("{name} facing"), reg.facing, &o["facing"], 1e-3);
    close(
        &format!("{name} contacts"),
        reg.contact_points as f64,
        &o["contact_points"],
        1.0,
    );
    assert_eq!(reg.verdict, o["verdict"].as_str().unwrap(), "{name}");
    let reasons: Vec<&str> = o["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(reg.reasons, reasons, "{name}");
}

/// Runs scenario `name` and compares it; returns the registration.
fn scenario(c: &Case, name: &str) -> Registration {
    let s = &c.f["scenarios"][name];
    let bites: Vec<BiteScan> = s["scans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let n = n.as_str().unwrap();
            bite_scan(n, &c.f["bite_scans"][n])
        })
        .collect();
    let mut init = ManualInit::default();
    if let Some(p) = s.get("picks") {
        // the manual pairing itself (`oa.bite_points`): a closed form, exact
        let l = bite::kabsch_init(
            &c.lower.to_case,
            &points(&p["lower_points"]),
            &points(&p["bite_points"]),
        )
        .unwrap();
        same_init(name, "lower", &l, &p["init_lower"], &bites[0].p);
        init.lower = Some(l);
        if let Some(u) = p.get("upper_points") {
            let u = bite::kabsch_init(
                &c.upper.to_case,
                &points(u),
                &points(&p["bite_points_upper"]),
            )
            .unwrap();
            same_init(name, "upper", &u, &p["init_upper"], &bites[0].p);
            init.upper = Some(u);
        }
    }
    let la = BiteArch::new(c.lower.clone()).unwrap();
    let ua = BiteArch::new(c.upper.clone()).unwrap();
    let reg = bite::register(&la, &ua, &bites, &init).unwrap();
    let (tol_mm, tol_deg) = if bites.len() == 2 {
        (TOL_TWO_MM, TOL_TWO_DEG)
    } else {
        (TOL_MM, TOL_DEG)
    };
    // the upper arch: its crown points in the case before the move
    let (pu, _) = c.upper.crown_points(None);
    let pu: Vec<DVec3> = pu
        .iter()
        .map(|&p| c.upper.to_case.transform_point3(p))
        .collect();
    let (d, deg) = apart(&reg.upper_delta, &mat4(&s["upper_delta"]), &pu);
    eprintln!("bite oracle {name}: upper arch {d:.2e} mm, {deg:.2e} deg");
    assert!(d < tol_mm && deg < tol_deg, "{name}: {d} mm, {deg} deg");
    for ((delta, want), b) in reg
        .bite_deltas
        .iter()
        .zip(s["bite_deltas"].as_array().unwrap())
        .zip(&bites)
    {
        let (d, deg) = apart(delta, &mat4(want), &b.p);
        eprintln!(
            "bite oracle {name}: bite scan {} {d:.2e} mm, {deg:.2e} deg",
            b.name
        );
        assert!(
            d < tol_mm && deg < tol_deg,
            "{name} {}: {d} mm, {deg} deg",
            b.name
        );
    }
    same_report(name, &reg, &s["report"]);
    reg
}

/// The port's manual pairing (bite -> arch) against the plugin's.
fn same_init(name: &str, arch: &str, got: &DMat4, want: &Value, pts: &[DVec3]) {
    let (d, deg) = apart(got, &mat4(want), pts);
    eprintln!("bite oracle {name}: 3 points on the {arch} arch {d:.2e} mm, {deg:.2e} deg");
    assert!(d < 1e-9 && deg < 1e-6, "{name} {arch}: {d} mm, {deg} deg");
}

#[test]
fn two_bite_scans_register_as_in_the_plugin() {
    let c = case();
    let reg = scenario(&c, "two_scans");
    // the case exercises the agreement and the interpenetration of a bite
    // under load
    assert!(reg.agreement_mm.is_some());
    assert!(
        reg.reasons.iter().any(|r| r == "interpenetration"),
        "{reg:?}"
    );
}

#[test]
fn one_bite_scan_registers_as_in_the_plugin() {
    let c = case();
    let reg = scenario(&c, "one_scan");
    assert!(reg.reasons.iter().any(|r| r == "one_side_only"), "{reg:?}");
}

#[test]
fn three_points_register_as_in_the_plugin() {
    let c = case();
    scenario(&c, "three_points_lower");
    scenario(&c, "three_points_both");
}

#[test]
fn the_correction_opens_the_bite_as_in_the_plugin() {
    let c = case();
    let reg = scenario(&c, "two_scans");
    let o = &c.f["scenarios"]["correct"];
    let upper = ArchModel {
        to_case: reg.upper_delta * c.upper.to_case,
        ..c.upper.clone()
    };
    let cor = bite::correct_penetration(&c.lower, &upper)
        .unwrap()
        .unwrap();
    let want = num(&o["opening_mm"]);
    eprintln!(
        "bite oracle correct: opening {:.6} mm (plugin {want:.6}), difference {:.2e} mm",
        cor.opening_mm,
        (cor.opening_mm - want).abs()
    );
    assert!(
        (cor.opening_mm - want).abs() < TOL_TWO_MM,
        "{cor:?} vs {want}"
    );
    // the correction alone, from the plugin's registered pose: the same
    // first contact but for the port's single-precision rays (the height
    // below a point is cast from 30 mm above it and its parameter comes back
    // as an f32: about 2e-6 mm at these coordinates), hence 1e-5 mm
    let at_plugin = ArchModel {
        to_case: mat4(&c.f["scenarios"]["two_scans"]["upper_after"]),
        ..c.upper.clone()
    };
    let alone = bite::correct_penetration(&c.lower, &at_plugin)
        .unwrap()
        .unwrap();
    eprintln!(
        "bite oracle correct from the plugin's pose: opening {:.9} mm, difference {:.2e} mm",
        alone.opening_mm,
        (alone.opening_mm - want).abs()
    );
    assert!(
        (alone.opening_mm - want).abs() < 1e-5,
        "{alone:?} vs {want}"
    );
    // the lift is along the occlusal axis of the lower arch
    let axis = c.lower.to_case.z_axis.truncate();
    let t = cor.upper_delta.w_axis.truncate();
    assert!((t - axis * cor.opening_mm).length() < 1e-12);
    // the report after it
    let r = &o["report"];
    close(
        "correct penetration",
        cor.penetration_mm,
        &r["penetration_mm"],
        2e-3,
    );
    close(
        "correct contacts",
        cor.contact_points as f64,
        &r["contact_points"],
        1.0,
    );
    close("correct total", cor.opening_mm, &r["correction_mm"], 2e-3);
    close(
        "correct registered",
        reg.penetration_mm,
        &r["registered_penetration_mm"],
        2e-3,
    );
    let reasons: Vec<&str> = r["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    let mut left = reg.reasons.clone();
    left.retain(|w| w != "interpenetration");
    assert_eq!(left, reasons);
    // the upper arch where the plugin put it
    let (pu, _) = c.upper.crown_points(None);
    let after = cor.upper_delta * upper.to_case;
    let (d, deg) = apart(&after, &mat4(&o["upper_after"]), &pu);
    eprintln!("bite oracle correct: upper arch {d:.2e} mm, {deg:.2e} deg");
    assert!(d < TOL_TWO_MM && deg < TOL_TWO_DEG, "{d} mm, {deg} deg");
    // the plugin's frames: the true bite and the displacement it starts from
    let truth = mat4(&c.f["lower_frame"]) * mat4(&c.f["true_relation"]);
    assert!(
        frame::rigid_inverse(&mat4(&c.f["upper_displacement"]))
            .abs_diff_eq(truth * frame::rigid_inverse(&c.upper.to_case), 1e-9)
    );
}
