//! Numerical comparison of `dca_algo::setup` with the Python plugin
//! (`ortho_aligner/setup_conservativo.py`, through the golden oracle).
//!
//! `golden/fixtures/<case>/setup/setup.json` is the oracle's output for the
//! six demo cases (`golden/README.md`: no locked or excluded tooth, no drawn
//! line, default scene settings); `setup_params.json` is the same with every
//! setup parameter of the plugin's page away from its default
//! (`inputs.settings`, the scene settings of `props.py`). The Rust stage is
//! fed the oracle's own inputs: the full scan moved to arch coordinates with
//! the worldframe matrix, the refined segmentation labels and the per-tooth
//! geometry of `teeth/teeth.json`, and the parameters the case settings
//! give (`Params::from_settings`, the settings the scene ones map to). The
//! configuration it runs with must be the plugin's
//! (`case_plan.conservative_config`, `inputs.config`). Bounds: target poses
//! within 0.01 mm and 0.05 degrees, the same IPR plan and therapy, the arch
//! line within 0.01 mm.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::setup::{self, Config, Input, Params, ToothInput};
use dca_model::SetupSettings;
use glam::{DMat3, DMat4, DVec3};
use serde_json::{Map, Value};

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

/// The case settings for the plugin's scene settings `scene` (the names of
/// `props.py`; an enum value in capitals).
fn settings_of(scene: &Value) -> SetupSettings {
    let mut out = Map::new();
    for (k, v) in scene.as_object().into_iter().flatten() {
        let key = match k.as_str() {
            "cons_align_scope" => "align_scope",
            "cons_line_sigma" => "line_sigma_mm",
            "cons_normal_tol" => "normal_tol_mm",
            "cons_rot_tol" => "rot_tol_deg",
            "cons_post_normal_tol" => "post_normal_tol_mm",
            "cons_post_rot_tol" => "post_rot_tol_deg",
            "cons_pos_tol" => "pos_tol_mm",
            "cons_contact_tol" => "contact_tol_mm",
            "cons_incisor_weight" => "incisor_weight",
            "cons_fix_molars" => "fix_molars",
            "cons_align_fixed" => "align_fixed",
            "cons_fix_third" => "fix_third",
            "cons_level_tol" => "level_tol_mm",
            "cons_level_canines" => "level_canines",
            "expansion_upper" => "expansion_upper_mm",
            "expansion_lower_posterior" => "expansion_lower_posterior_mm",
            "expansion_lower_canine" => "expansion_lower_canine_mm",
            "advance_max" => "advance_max_mm",
            "retract_max" => "retract_max_mm",
            "ipr_surface_cap" => "ipr_surface_cap_mm",
            "vertical_max" => "vertical_max_mm",
            "midline" | "priority" | "ipr_zone" | "leveling" | "enamel_fraction"
            | "close_spaces" => k.as_str(),
            other => panic!("scene setting {other} has no case setting"),
        };
        let v = match v.as_str() {
            Some(s) => Value::from(s.to_lowercase()),
            None => v.clone(),
        };
        out.insert(key.to_owned(), v);
    }
    let s: SetupSettings = serde_json::from_value(Value::Object(out)).unwrap();
    assert!(s.out_of_range().is_none(), "{s:?}");
    s
}

/// An enum of the configuration as the plugin writes it.
fn upper_name(v: impl serde::Serialize) -> Value {
    match serde_json::to_value(v).unwrap() {
        Value::String(s) => Value::from(s.to_uppercase()),
        Value::Array(a) => Value::Array(
            a.into_iter()
                .map(|x| Value::from(x.as_str().unwrap().to_uppercase()))
                .collect(),
        ),
        other => other,
    }
}

/// The configuration the case settings give is the plugin's
/// (`conservative_config` of the same scene settings), field by field.
fn check_config(what: &str, cfg: &Config, want: &Value) {
    let numbers = [
        ("sigma", cfg.sigma),
        ("pos_tol", cfg.pos_tol),
        ("normal_tol", cfg.normal_tol),
        ("rot_tol_deg", cfg.rot_tol_deg),
        ("max_derotation", cfg.max_derotation),
        ("vertical_max", cfg.vertical_max),
        ("post_normal_tol", cfg.post_normal_tol),
        ("post_rot_tol_deg", cfg.post_rot_tol_deg),
        ("level_tol", cfg.level_tol),
        ("contact_tol", cfg.contact_tol),
        ("incisor_weight", cfg.incisor_weight),
        ("expansion_posterior", cfg.expansion_posterior),
        ("expansion_canine", cfg.expansion_canine),
        ("advance_max", cfg.advance_max),
        ("retract_max", cfg.retract_max),
        ("ipr_surface_cap", cfg.ipr_surface_cap),
        ("enamel_fraction", cfg.enamel_fraction),
    ];
    for (k, v) in numbers {
        assert!(
            (v - num(&want[k])).abs() < 1e-12,
            "{what}: {k} {v} against {}",
            want[k]
        );
    }
    for (k, v) in [
        ("level_canines", cfg.level_canines),
        ("fix_molars", cfg.fix_molars),
        ("fix_third", cfg.fix_third),
        ("fix_canines", cfg.fix_canines),
        ("align_fixed", cfg.align_fixed),
        ("close_spaces", cfg.close_spaces),
    ] {
        assert_eq!(Value::from(v), want[k], "{what}: {k}");
    }
    for (k, v) in [
        ("align_scope", upper_name(cfg.align_scope)),
        ("leveling", upper_name(cfg.leveling)),
        ("midline", upper_name(cfg.midline)),
        ("priority", upper_name(&cfg.priority)),
        ("ipr_zone", upper_name(cfg.ipr_zone)),
    ] {
        assert_eq!(v, want[k], "{what}: {k}");
    }
}

fn compare(case: &str, file: &str, worst: &mut Worst) {
    let dir = fixtures().join(case);
    let teeth_json = json(&dir.join("teeth/teeth.json"));
    let oracle = json(&dir.join("setup").join(file));
    let arch = if teeth_json["arch"] == "UPPER" {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let params = Params::from_settings(&settings_of(&oracle["inputs"]["settings"]));
    check_config(
        &format!("{case} {file}"),
        &params.config(arch),
        &oracle["inputs"]["config"],
    );
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
    // the plugin's derotation and line scale (the port's own are stated in
    // `derotating_only_the_excess_changes_the_demo_setups_as_stated` and
    // `the_scale_of_the_arch_line_changes_the_demo_setups_as_stated`)
    let cfg = Config {
        plugin_derotation: true,
        plugin_line_scale: true,
        ..params.config(arch)
    };
    let out = setup::run_with(&input, &cfg).unwrap();
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
    assert_eq!(setup::run_with(&input, &cfg).unwrap(), out, "{case}: rerun");
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
        compare(case, "setup.json", &mut worst);
        println!("{case}: {worst:?}");
        let mut worst = Worst::default();
        compare(case, "setup_params.json", &mut worst);
        println!("{case} with parameters: {worst:?}");
    }
}

/// What derotating only the excess over the tolerance (the port's default,
/// [`Config::plugin_derotation`] off, departure of 2026-11-23) changes on
/// the demo setups against the plugin's (`setup.json`, default settings):
/// a tooth the plugin derotates keeps up to the tolerance of its zone, so
/// its final turn differs from the plugin's by about that tolerance (1
/// degree for the incisors and canines; up to 2 degrees where the tooth
/// also ends elsewhere on the line or its axis is tipped), and its centre
/// of resistance ends up to 0.15 mm away. The arch line, the moved teeth
/// and the IPR plan are the same.
const EXCESS_ONLY: [Departure; 6] = [
    ("demo_lower_mild", 1.64, 0.10, 0.20, &[], 0.0),
    ("demo_lower_moderate", 1.04, 0.02, 0.35, &[], 0.0),
    ("demo_lower_severe", 1.12, 0.12, 0.50, &[], 0.0),
    ("demo_upper_mild", 1.18, 0.13, 0.10, &[], 0.0),
    ("demo_upper_moderate", 1.98, 0.15, 0.15, &[], 0.0),
    ("demo_upper_severe", 1.20, 0.08, 0.50, &[], 0.0),
];

/// What the port's scale of the robust arch line (taken once, from the
/// canines and premolars, [`Config::plugin_line_scale`] off, departure of
/// 2026-11-23) changes on the demo setups against the plugin's
/// (`setup.json`, default settings, the plugin's derotation): the line
/// moves by up to 1.12 mm (`demo_lower_mild`, elsewhere at most 0.22 mm),
/// a moved tooth by at most 1.63 degrees and 0.16 mm, the moved teeth are
/// the same and the IPR of `demo_upper_severe` is one step less (0.45 mm
/// instead of 0.50 at its seven contacts).
const LINE_SCALE: [Departure; 6] = [
    ("demo_lower_mild", 1.63, 0.15, 0.20, &[], 1.12),
    ("demo_lower_moderate", 0.44, 0.04, 0.35, &[], 0.03),
    ("demo_lower_severe", 0.06, 0.01, 0.50, &[], 0.01),
    ("demo_upper_mild", 0.01, 0.00, 0.10, &[], 0.00),
    ("demo_upper_moderate", 0.01, 0.00, 0.15, &[], 0.00),
    ("demo_upper_severe", 0.79, 0.16, 0.45, &[], 0.22),
];

/// A departure from the plugin's setup of a demo case: (case, largest
/// rotation difference of a moved tooth in degrees, largest distance
/// between the final centres of resistance in mm, the IPR at every contact
/// the plugin strips in mm, teeth the plugin moves and the port does not,
/// largest distance between the arch lines in mm).
type Departure = (&'static str, f64, f64, f64, &'static [u8], f64);

#[test]
fn derotating_only_the_excess_changes_the_demo_setups_as_stated() {
    stated(&EXCESS_ONLY, (false, true));
}

#[test]
fn the_scale_of_the_arch_line_changes_the_demo_setups_as_stated() {
    stated(&LINE_SCALE, (true, false));
}

/// Checks `rows` against the setups with the port's `(plugin_derotation,
/// plugin_line_scale)` and the plugin's, on every demo case.
fn stated(rows: &[Departure], port_cfg: (bool, bool)) {
    let dir = fixtures();
    let mut cases: Vec<String> = std::fs::read_dir(&dir)
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
    let mut unstated = Vec::new();
    for case in &cases {
        let (port, plugin) = (run_case(case, port_cfg), run_case(case, (true, true)));
        let rest_frames = &json(&dir.join(case).join("setup/setup.json"))["rest_frames"];
        let (mut deg, mut mm) = (0.0f64, 0.0f64);
        for (f, d) in &plugin.result.targets {
            let a = port
                .result
                .targets
                .get(f)
                .copied()
                .unwrap_or(DMat4::IDENTITY);
            let rest = mat4(&rest_frames[f.to_string()]);
            deg = deg.max(rotation_deg(&a, d));
            mm = mm.max(((a * rest).w_axis - (*d * rest).w_axis).truncate().length());
        }
        let gone: Vec<u8> = plugin
            .result
            .targets
            .keys()
            .filter(|f| !port.result.targets.contains_key(f))
            .map(|f| f.get())
            .collect();
        let ipr: Vec<f64> = plugin
            .result
            .ipr
            .keys()
            .map(|c| port.result.ipr.get(c).copied().unwrap_or(0.0))
            .collect();
        let line = port
            .line
            .iter()
            .zip(&plugin.line)
            .map(|(p, q)| p.distance(*q))
            .fold(0.0, f64::max);
        eprintln!(
            "{case}: rotation {deg:.2} deg, translation {mm:.2} mm, IPR {ipr:?} (plugin {:?}), not moved {gone:?}, line {line:.3} mm",
            plugin.result.ipr.values().collect::<Vec<_>>()
        );
        let want = rows.iter().find(|r| r.0 == case);
        let Some(&(_, w_deg, w_mm, w_ipr, w_gone, w_line)) = want else {
            unstated.push(case.clone());
            continue;
        };
        assert!((line - w_line).abs() < 0.01, "{case}: line {line}");
        assert!((deg - w_deg).abs() < 0.05, "{case}: rotation {deg}");
        assert!((mm - w_mm).abs() < 0.05, "{case}: translation {mm}");
        assert!(
            ipr.iter().all(|&v| (v - w_ipr).abs() < 1e-9),
            "{case}: {ipr:?}"
        );
        assert_eq!(gone, w_gone, "{case}: teeth not moved");
    }
    assert!(unstated.is_empty(), "not stated: {unstated:?}");
}

/// The setup of `case` (`setup.json`, the oracle's inputs) with the
/// plugin's derotation and line scale or the port's.
fn run_case(case: &str, (plugin_derotation, plugin_line_scale): (bool, bool)) -> setup::Output {
    let dir = fixtures().join(case);
    let teeth_json = json(&dir.join("teeth/teeth.json"));
    let oracle = json(&dir.join("setup").join("setup.json"));
    let arch = if teeth_json["arch"] == "UPPER" {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let params = Params::from_settings(&settings_of(&oracle["inputs"]["settings"]));
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
    let cfg = Config {
        plugin_derotation,
        plugin_line_scale,
        ..params.config(arch)
    };
    setup::run_with(&input, &cfg).unwrap()
}

#[test]
fn the_default_settings_are_the_default_parameters() {
    assert_eq!(
        Params::from_settings(&SetupSettings::default()),
        Params::default()
    );
}
