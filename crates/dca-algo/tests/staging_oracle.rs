//! Numerical comparison of `dca_algo::staging` and `dca_algo::schedule` with
//! the Python plugin (`ortho_aligner/staging.py`, `schedule.py`) on the six
//! demo cases of `golden/fixtures`.
//!
//! The staging is fed what the oracle fed it (`golden/oracle/stages.py`,
//! `stage_staging`): the closed teeth in their rest frames
//! (`staging/closed_<fdi>.dcm`), the rest frames and `md_sign` of
//! `teeth/teeth.json`, the target deltas and the IPR plan of
//! `setup/setup.json`, the complexity of the oracle's diagnosis and the
//! default parameters. Bounds (PLAN section 5): poses within 0.01 mm and
//! 0.05 degrees; the number of stages, the phases and the IPR timing equal.

use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::schedule::{self, Date, Policy};
use dca_algo::staging::{self, Complexity, IprPlan, Level, Params, Pattern, ToothMove};
use glam::{DMat4, DVec3};
use serde_json::Value;

const CASES: [&str; 6] = [
    "demo_lower_mild",
    "demo_lower_moderate",
    "demo_lower_severe",
    "demo_upper_mild",
    "demo_upper_moderate",
    "demo_upper_severe",
];

const TOL_MM: f64 = 0.01;
const TOL_DEG: f64 = 0.05;

fn dir(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/fixtures")
        .join(case)
}

fn json(case: &str, rel: &str) -> Value {
    let text = std::fs::read_to_string(dir(case).join(rel)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn fdi(n: u64) -> Fdi {
    Fdi::new(n as u8).unwrap()
}

fn mat4(v: &Value) -> DMat4 {
    let rows = v.as_array().unwrap();
    let mut cols = [0.0; 16];
    for (r, row) in rows.iter().enumerate() {
        for (c, x) in row.as_array().unwrap().iter().enumerate() {
            cols[c * 4 + r] = num(x);
        }
    }
    DMat4::from_cols_array(&cols)
}

fn q6(v: &Value) -> staging::Q6 {
    let a = v.as_array().unwrap();
    std::array::from_fn(|i| num(&a[i]))
}

struct Case {
    moves: Vec<ToothMove>,
    ipr: Vec<IprPlan>,
    complexity: Complexity,
    plan: Value,
}

fn load(case: &str) -> Case {
    let teeth = json(case, "teeth/teeth.json");
    let setup = json(case, "setup/setup.json");
    let staging = json(case, "staging/staging.json");
    let plan = staging["plan"].clone();
    let mut moves = Vec::new();
    for f in teeth["sequence"].as_array().unwrap() {
        let f = f.as_u64().unwrap();
        let t = &teeth["teeth"][f.to_string()];
        let frame = mat4(&t["rest_frame"]);
        let delta = setup["targets_delta"]
            .get(f.to_string())
            .map_or(DMat4::IDENTITY, mat4);
        let bytes = std::fs::read(dir(case).join(format!("staging/closed_{f}.dcm"))).unwrap();
        let mesh = dc_mesh::dcm::decode(&bytes).unwrap();
        let pts: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        moves.push(ToothMove::from_delta(
            fdi(f),
            frame,
            &delta,
            pts,
            t["md_sign"].as_i64().unwrap() as i8,
        ));
    }
    let ipr = setup["setup"]["ipr"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| IprPlan {
            a: fdi(r["a"].as_u64().unwrap()),
            b: fdi(r["b"].as_u64().unwrap()),
            mm: num(&r["mm"]),
        })
        .collect();
    let cx = &plan["complexity"];
    let complexity = Complexity {
        level: match cx["level"].as_str().unwrap() {
            "FACILE" => Level::Easy,
            "MODERATO" => Level::Moderate,
            "DIFFICILE" => Level::Difficult,
            _ => Level::VeryComplex,
        },
        crowding_mm: num(&cx["crowding"]),
        little_mm: num(&cx["little"]),
        distalization_mm: num(&cx["distalization"]),
        pattern_override: None,
    };
    Case {
        moves,
        ipr,
        complexity,
        plan,
    }
}

fn plan_of(c: &Case) -> staging::Plan {
    staging::run(
        &staging::Input {
            moves: &c.moves,
            ipr: &c.ipr,
            complexity: c.complexity,
        },
        &Params::default(),
    )
    .unwrap()
}

/// Largest pose difference between two movements: millimetres of the
/// translation and degrees of the relative rotation.
fn pose_error(a: &staging::Q6, b: &staging::Q6) -> (f64, f64) {
    let t = DVec3::new(a[0] - b[0], a[1] - b[1], a[2] - b[2]).length();
    let ra = staging::pose_from_q(a).q;
    let rb = staging::pose_from_q(b).q;
    let deg = ra.angle_between(rb).to_degrees();
    (t, deg)
}

#[test]
fn the_staging_matches_the_plugin_on_the_demo_cases() {
    for case in CASES {
        let c = load(case);
        let plan = plan_of(&c);
        let want = &c.plan;
        let n = want["n_stages"].as_u64().unwrap() as u32;
        assert_eq!(plan.n_stages, n, "{case}: number of aligners");
        assert_eq!(
            format!("{:?}", plan.pattern).to_uppercase(),
            want["pattern"].as_str().unwrap().replace('_', ""),
            "{case}: pattern"
        );
        let (mut worst_mm, mut worst_deg) = (0.0_f64, 0.0_f64);
        for (f, want_q) in want["q"].as_object().unwrap() {
            let f = fdi(f.parse().unwrap());
            let got = &plan.q[&f];
            let want_q = want_q.as_array().unwrap();
            assert_eq!(got.len(), want_q.len(), "{case} {f}");
            for (k, (g, w)) in got.iter().zip(want_q).enumerate() {
                let (mm, deg) = pose_error(g, &q6(w));
                worst_mm = worst_mm.max(mm);
                worst_deg = worst_deg.max(deg);
                assert!(mm < TOL_MM, "{case} {f} stage {k}: {mm} mm");
                assert!(deg < TOL_DEG, "{case} {f} stage {k}: {deg} deg");
            }
        }
        // the aligners each tooth needs and its phases
        for (f, n) in want["need"].as_object().unwrap() {
            let f = fdi(f.parse().unwrap());
            assert_eq!(
                u64::from(plan.need[&f]),
                n.as_u64().unwrap(),
                "{case} {f}: need"
            );
        }
        for (f, phases) in want["phases"].as_object().unwrap() {
            let f = fdi(f.parse().unwrap());
            let got = &plan.phases[&f];
            let phases = phases.as_array().unwrap();
            assert_eq!(got.len(), phases.len(), "{case} {f}");
            for (g, w) in got.iter().zip(phases) {
                let mask: Vec<u64> = w[0]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap())
                    .collect();
                let got_mask: Vec<u64> = (0..6).filter(|&i| g.mask[i]).map(|i| i as u64).collect();
                assert_eq!(got_mask, mask, "{case} {f}: phase mask");
                assert_eq!(g.start, w[1].as_i64().unwrap(), "{case} {f}: phase start");
                assert_eq!(g.n, w[2].as_i64().unwrap(), "{case} {f}: phase length");
            }
        }
        // IPR timing
        let want_ipr: Vec<(u64, u64, u64)> = want["ipr_stage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[0].as_u64().unwrap(),
                    r[1].as_u64().unwrap(),
                    r[2].as_u64().unwrap(),
                )
            })
            .collect();
        let got_ipr: Vec<(u64, u64, u64)> = plan
            .ipr_stage
            .iter()
            .map(|r| {
                (
                    u64::from(r.a.get()),
                    u64::from(r.b.get()),
                    u64::from(r.stage),
                )
            })
            .collect();
        assert_eq!(got_ipr, want_ipr, "{case}: IPR timing");
        assert_eq!(
            plan.unresolved.len(),
            want["unresolved"].as_array().unwrap().len(),
            "{case}"
        );
        for (g, w) in plan
            .final_overlap
            .iter()
            .zip(want["final_overlap"].as_array().unwrap())
        {
            assert_eq!(u64::from(g.a.get()), w[0].as_u64().unwrap());
            assert!(
                (g.mm - num(&w[2])).abs() < 5e-3,
                "{case} {}|{}: {} vs {}",
                g.a,
                g.b,
                g.mm,
                w[2]
            );
        }
        // the per-tooth statistics
        for (f, s) in want["stats"].as_object().unwrap() {
            let f = fdi(f.parse().unwrap());
            let st = &plan.stats[&f];
            assert_eq!(
                u64::from(st.moving_stages),
                s["moving_stages"].as_u64().unwrap(),
                "{case} {f}"
            );
            assert!((st.cap - num(&s["cap"])).abs() < 1e-12);
            assert!(
                (st.max_crown_step - num(&s["max_crown_step"])).abs() < 1e-3,
                "{case} {f}"
            );
            for i in 0..6 {
                assert!((st.limits[i] - num(&s["limits"][i])).abs() < 1e-12);
            }
        }
        eprintln!(
            "{case}: {} aligners, worst pose error {worst_mm:.2e} mm / {worst_deg:.2e} deg",
            plan.n_stages
        );
    }
}

#[test]
fn the_pattern_comes_from_the_complexity_when_it_is_auto() {
    let c = load("demo_lower_mild");
    let plan = plan_of(&c);
    assert_eq!(plan.pattern, Pattern::Structured);
    // an override from the clinician wins
    let mut cx = c.complexity;
    cx.pattern_override = Some(Pattern::Proportional);
    let plan = staging::run(
        &staging::Input {
            moves: &c.moves,
            ipr: &c.ipr,
            complexity: cx,
        },
        &Params::default(),
    )
    .unwrap();
    assert_eq!(plan.pattern, Pattern::Proportional);
}

/// The wear calendar of the plugin, FIXED and ADAPTIVE, on the oracle's plan.
#[test]
fn the_calendar_matches_the_plugin() {
    for case in CASES {
        let staging = json(case, "staging/staging.json");
        let setup = json(case, "setup/setup.json");
        let expected = json(case, "schedule/schedule.json");
        let plan = &staging["plan"];
        let q = plan["q"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(f, v)| {
                (
                    fdi(f.parse().unwrap()),
                    v.as_array().unwrap().iter().map(q6).collect(),
                )
            })
            .collect();
        let ipr = plan["ipr_stage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let (a, b) = (fdi(r[0].as_u64().unwrap()), fdi(r[1].as_u64().unwrap()));
                let mm = setup["setup"]["ipr"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|x| x["a"] == r[0] && x["b"] == r[1])
                    .map_or(0.0, |x| num(&x["mm"]));
                schedule::IprAt {
                    a,
                    b,
                    stage: r[2].as_u64().unwrap() as u32,
                    mm,
                }
            })
            .collect();
        let input = schedule::Input {
            n_stages: plan["n_stages"].as_u64().unwrap() as u32,
            n_total: None,
            q,
            ipr,
            attachments: Vec::new(),
            offset: 0,
        };
        for (policy, name) in [(Policy::Fixed, "FIXED"), (Policy::Adaptive, "ADAPTIVE")] {
            let params = schedule::Params {
                start_date: Some("2026-01-05".into()),
                interval_days: plan["interval_days"].as_u64().unwrap() as u32,
                policy,
                ..schedule::Params::default()
            };
            let cal = schedule::run(&input, &params).unwrap();
            let want = &expected["policies"][name];
            let want_iv: Vec<u32> = want["intervals"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u32)
                .collect();
            let got_iv: Vec<u32> = cal.rows.iter().map(|r| r.days).collect();
            assert_eq!(got_iv, want_iv, "{case} {name}: intervals");
            let wc = &want["calendar"];
            assert_eq!(cal.start.to_iso(), wc["start"].as_str().unwrap());
            assert_eq!(
                cal.end.to_iso(),
                wc["end"].as_str().unwrap(),
                "{case} {name}"
            );
            assert_eq!(
                u64::from(cal.total_days),
                wc["total_days"].as_u64().unwrap()
            );
            assert_eq!(cal.rows.len(), wc["rows"].as_array().unwrap().len());
            for (r, w) in cal.rows.iter().zip(wc["rows"].as_array().unwrap()) {
                assert_eq!(u64::from(r.number), w["number"].as_u64().unwrap());
                assert_eq!(
                    r.start.to_iso(),
                    w["start"].as_str().unwrap(),
                    "{case} {name}"
                );
                assert_eq!(r.end.to_iso(), w["end"].as_str().unwrap());
                assert_eq!(u64::from(r.first_day), w["first_day"].as_u64().unwrap());
                let todo: Vec<&str> = w["todo"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap())
                    .collect();
                assert_eq!(r.todo, todo, "{case} {name} aligner {}", r.number);
            }
            // the .ics, unfolded, is the plugin's apart from the product lines and the stamp
            let ics = schedule::to_ics(
                &cal,
                &schedule::IcsOptions {
                    title: "Golden".into(),
                    arch_label: if case.contains("lower") {
                        "LOWER"
                    } else {
                        "UPPER"
                    }
                    .into(),
                    lang: schedule::Lang::It,
                    stamp: "20261031T161746Z".into(),
                    ..schedule::IcsOptions::default()
                },
            );
            let want_ics = std::fs::read_to_string(
                dir(case).join(format!("schedule/schedule_{}.ics", name.to_lowercase())),
            )
            .unwrap();
            let norm = |s: &str| -> Vec<String> {
                schedule::unfold(s)
                    .lines()
                    .filter(|l| !l.starts_with("PRODID") && !l.starts_with("DTSTAMP"))
                    .map(|l| {
                        let l = l
                            .replace("@dentalcraft", "@X")
                            .replace("@ortho-aligner", "@X")
                            .replace("-controllo@", "-check@");
                        l.replacen("UID:dc-", "UID:oa-", 1)
                    })
                    .collect()
            };
            assert_eq!(norm(&ics), norm(&want_ics), "{case} {name}: ics");
        }
        let _ = Date::parse("2026-01-05").unwrap();
    }
}
