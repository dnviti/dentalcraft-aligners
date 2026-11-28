//! Numerical comparison of `dca_algo::staging` and `dca_algo::schedule` with
//! the Python plugin (`ortho_aligner/staging.py`, `schedule.py`) on the six
//! demo cases of `golden/fixtures`.
//!
//! The staging is fed what the oracle fed it (`golden/oracle/stages.py`,
//! `stage_staging`): the closed teeth in their rest frames
//! (`staging/closed_<fdi>.dcm`), the rest frames and `md_sign` of
//! `teeth/teeth.json`, the target deltas and the IPR plan of
//! `setup/setup.json`, the complexity of the oracle's diagnosis and the
//! default parameters, or, for `staging/staging_params.json`, every staging
//! parameter of the plugin's page away from its default (through the case
//! settings), each explicit pattern and one slowed tooth. Bounds (PLAN
//! section 5): poses within 0.01 mm and 0.05 degrees; the number of stages,
//! the phases, the IPR timing and the calendar dates equal. The same plan
//! comes out of the target poses as the case stores them (`Setup::poses`),
//! whatever the sign of their quaternions. The comparison keeps the plugin's
//! sample of 2500 points per crown in the collision check, its delays
//! against a neighbour that never moves and its delays that do not lower the
//! worst overlap of the pair over the plan; what the port's own defaults,
//! every crown point and neither kind of delay, change is stated in
//! `every_crown_point_changes_the_demo_plans_as_stated`,
//! `no_delay_against_a_still_neighbour_leaves_the_demo_plans_as_they_are` and
//! `only_delays_that_lower_the_worst_overlap_leave_the_demo_plans_as_they_are`.

use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::schedule::{self, Date, Policy};
use dca_algo::staging::{self, Complexity, IprPlan, Level, Params, Pattern, ToothMove};
use dca_model::{IntervalPolicy, Settings};
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

/// Crown points the plugin's collision check keeps (`sample_points`).
const PLUGIN_SAMPLE: usize = 2500;

/// The staging parameters `p` with the plugin's sample of the crowns (2500
/// points, every k-th of the list), its delays against a neighbour that
/// never moves and its delays that do not lower the worst overlap of the
/// pair over the plan: the plugin's plans are reproduced with them. The
/// port's own defaults use every crown point
/// ([`staging::Params::sample_points`]) and leave such collisions listed
/// instead ([`staging::Params::delay_against_still`],
/// [`staging::Params::delay_without_gain`]); what they change on the demo
/// cases is stated in [`every_crown_point_changes_the_demo_plans_as_stated`],
/// [`no_delay_against_a_still_neighbour_leaves_the_demo_plans_as_they_are`]
/// and [`only_delays_that_lower_the_worst_overlap_leave_the_demo_plans_as_they_are`].
fn plugin(p: Params) -> Params {
    Params {
        sample_points: Some(PLUGIN_SAMPLE),
        delay_against_still: true,
        delay_without_gain: true,
        ..p
    }
}

fn plan_with(c: &Case, p: &Params) -> staging::Plan {
    staging::run(
        &staging::Input {
            moves: &c.moves,
            ipr: &c.ipr,
            complexity: c.complexity,
        },
        p,
    )
    .unwrap()
}

fn plan_of(c: &Case) -> staging::Plan {
    plan_with(c, &plugin(Params::default()))
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

/// The largest rotation (degrees) of any tooth from one aligner to the next.
fn max_turn_per_stage<'a>(series: impl Iterator<Item = &'a [staging::Q6]>) -> f64 {
    series
        .flat_map(|qs| qs.windows(2).map(|w| pose_error(&w[0], &w[1]).1))
        .fold(0.0, f64::max)
}

/// The plugin reads each target as a matrix (`q_from_delta`); the case
/// stores it as a pose (`staging::pose_from_q`, what the setup command
/// stores) and the staging command reads it back (`staging::q_from_pose`).
/// Projects saved before the fix hold the same rotations with `w < 0`: both
/// signs give the plugin's aligners, never a tooth spun the long way round.
#[test]
fn stored_target_poses_of_either_quaternion_sign_give_the_plugins_aligners() {
    for case in CASES {
        let c = load(case);
        let want = &c.plan;
        let want_q: Vec<Vec<staging::Q6>> = want["q"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_array().unwrap().iter().map(q6).collect())
            .collect();
        let plugin_turn = max_turn_per_stage(want_q.iter().map(Vec::as_slice));
        for flip in [false, true] {
            let moves: Vec<ToothMove> = c
                .moves
                .iter()
                .map(|m| {
                    let mut stored = staging::pose_from_q(&m.q);
                    assert!(stored.q.w >= 0.0, "{case} {}", m.fdi);
                    if flip {
                        stored.q = -stored.q;
                    }
                    let q = staging::q_from_pose(&stored);
                    for (i, (a, b)) in q.iter().zip(&m.q).enumerate() {
                        assert!((a - b).abs() < 1e-12, "{case} {} {i}", m.fdi);
                    }
                    ToothMove::new(m.fdi, m.frame, q, m.pts.clone(), m.md_sign)
                })
                .collect();
            let plan = staging::run(
                &staging::Input {
                    moves: &moves,
                    ipr: &c.ipr,
                    complexity: c.complexity,
                },
                &plugin(Params::default()),
            )
            .unwrap();
            let n = want["n_stages"].as_u64().unwrap() as u32;
            assert_eq!(plan.n_stages, n, "{case} flip {flip}: number of aligners");
            for (f, w) in want["q"].as_object().unwrap() {
                let got = &plan.q[&fdi(f.parse().unwrap())];
                for (k, (g, w)) in got.iter().zip(w.as_array().unwrap()).enumerate() {
                    let (mm, deg) = pose_error(g, &q6(w));
                    assert!(
                        mm < TOL_MM && deg < TOL_DEG,
                        "{case} {f} stage {k}: {mm} mm {deg} deg"
                    );
                }
            }
            let turn = max_turn_per_stage(plan.q.values().map(Vec::as_slice));
            assert!(
                (turn - plugin_turn).abs() < TOL_DEG,
                "{case}: {turn} vs {plugin_turn}"
            );
            if flip {
                eprintln!(
                    "{case}: {n} aligners (plugin {n}), largest turn of a tooth per aligner {turn:.3} deg (plugin {plugin_turn:.3} deg)"
                );
            }
        }
    }
}

/// The plan of the port against the plugin's `plan_json` `want`: the
/// pattern, the number of aligners, the poses of every stage within the
/// bounds, the aligners each tooth needs, the phases, the IPR timing, the
/// collisions left, the final overlaps and the statistics per tooth. Returns
/// the worst pose error (mm, degrees).
fn assert_plan_matches(case: &str, plan: &staging::Plan, want: &Value) -> (f64, f64) {
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
    // the final movement (after the overcorrection, when it is on)
    for (f, w) in want["q_final"].as_object().unwrap() {
        let (mm, deg) = pose_error(&plan.q_final[&fdi(f.parse().unwrap())], &q6(w));
        assert!(
            mm < TOL_MM && deg < TOL_DEG,
            "{case} {f}: final {mm} mm {deg} deg"
        );
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
        assert!((st.cap - num(&s["cap"])).abs() < 1e-12, "{case} {f}: cap");
        assert!(
            (st.max_crown_step - num(&s["max_crown_step"])).abs() < 1e-3,
            "{case} {f}"
        );
        for i in 0..6 {
            assert!(
                (st.limits[i] - num(&s["limits"][i])).abs() < 1e-12,
                "{case} {f}: limit {i}"
            );
        }
    }
    (worst_mm, worst_deg)
}

#[test]
fn the_staging_matches_the_plugin_on_the_demo_cases() {
    for case in CASES {
        let c = load(case);
        let plan = plan_of(&c);
        let (worst_mm, worst_deg) = assert_plan_matches(case, &plan, &c.plan);
        eprintln!(
            "{case}: {} aligners, worst pose error {worst_mm:.2e} mm / {worst_deg:.2e} deg",
            plan.n_stages
        );
    }
}

/// What every crown point (the port's default, [`staging::Params::sample_points`]
/// `None`, departure of 2026-11-23) changes on the demo cases against the
/// plugin's plans, made with its sample of 2500 points per crown:
/// * the same number of aligners, the same collisions left and the same
///   poses of every aligner;
/// * the IPR of a few contacts one or two aligners earlier or later (the
///   aligner where the crowns first touch beyond the tolerance);
/// * final overlaps up to 0.12 mm deeper (0.07 to 0.11 mm on the six
///   cases): the sample missed the deepest points of the crowns.
#[test]
fn every_crown_point_changes_the_demo_plans_as_stated() {
    for case in CASES {
        let c = load(case);
        let plan = plan_with(
            &c,
            &Params {
                delay_against_still: true,
                delay_without_gain: true,
                ..Params::default()
            },
        );
        let want = &c.plan;
        let n = want["n_stages"].as_u64().unwrap() as u32;
        assert_eq!(plan.n_stages, n, "{case}: number of aligners");
        assert_eq!(
            plan.unresolved.len(),
            want["unresolved"].as_array().unwrap().len(),
            "{case}: collisions left"
        );
        for (f, want_q) in want["q"].as_object().unwrap() {
            let got = &plan.q[&fdi(f.parse().unwrap())];
            for (k, (g, w)) in got.iter().zip(want_q.as_array().unwrap()).enumerate() {
                let (mm, deg) = pose_error(g, &q6(w));
                assert!(
                    mm < TOL_MM && deg < TOL_DEG,
                    "{case} {f} stage {k}: {mm} mm {deg} deg"
                );
            }
        }
        let want_ipr = want["ipr_stage"].as_array().unwrap();
        assert_eq!(plan.ipr_stage.len(), want_ipr.len(), "{case}");
        for (g, w) in plan.ipr_stage.iter().zip(want_ipr) {
            assert_eq!(u64::from(g.a.get()), w[0].as_u64().unwrap(), "{case}");
            assert_eq!(u64::from(g.b.get()), w[1].as_u64().unwrap(), "{case}");
            assert!(
                u64::from(g.stage).abs_diff(w[2].as_u64().unwrap()) <= 2,
                "{case} IPR {}|{}: aligner {} (plugin {})",
                g.a,
                g.b,
                g.stage,
                w[2]
            );
        }
        for (g, w) in plan
            .final_overlap
            .iter()
            .zip(want["final_overlap"].as_array().unwrap())
        {
            assert!(
                (g.mm - num(&w[2])).abs() < 0.12,
                "{case} {}|{}: {} vs {}",
                g.a,
                g.b,
                g.mm,
                w[2]
            );
        }
    }
}

/// What leaving a collision against a neighbour that never moves (the port's
/// default, [`staging::Params::delay_against_still`] off, departure of
/// 2026-11-23) changes on the demo cases against the plugin's plans: nothing.
/// On the plugin's setups of these cases no tooth runs into a still
/// neighbour, so the plugin delays none and the plans stay the same (on the
/// port's own setups of the demo arches, as the commands chain them, it is
/// what shortens `demo_lower_severe` from 30 to 18 aligners and
/// `demo_upper_moderate` from 25 to 9, each still listing its collision).
#[test]
fn no_delay_against_a_still_neighbour_leaves_the_demo_plans_as_they_are() {
    for case in CASES {
        let c = load(case);
        let want = plan_of(&c);
        let plan = plan_with(
            &c,
            &Params {
                sample_points: Some(PLUGIN_SAMPLE),
                delay_without_gain: true,
                ..Params::default()
            },
        );
        assert_eq!(plan.n_stages, want.n_stages, "{case}: number of aligners");
        assert_eq!(plan.q, want.q, "{case}: poses");
        assert_eq!(plan.unresolved, want.unresolved, "{case}: collisions left");
        assert_eq!(plan.log, want.log, "{case}: notes");
    }
}

/// What keeping only the delays that lower the worst overlap of their pair
/// over the whole plan (the port's default,
/// [`staging::Params::delay_without_gain`] off, departure of 2026-11-23)
/// changes on the demo cases against the plugin's plans: nothing. The one
/// delay the plugin makes on its setups of these cases, 41 by 7 aligners in
/// `demo_lower_mild`, lowers the worst overlap of 41 and its neighbour over
/// the plan, so it stays. A delay that would only move a collision to a
/// later aligner is not made and the collision is listed instead (the unit
/// test `a_delay_that_only_moves_a_collision_later_is_not_kept`); on the
/// port's own setups of the demo arches under segmentation noise it shortens
/// plans that the plugin's delays stretched to 50-54 aligners to 23-36
/// (measured 2026-11-23).
#[test]
fn only_delays_that_lower_the_worst_overlap_leave_the_demo_plans_as_they_are() {
    for case in CASES {
        let c = load(case);
        let want = plan_of(&c);
        let plan = plan_with(
            &c,
            &Params {
                sample_points: Some(PLUGIN_SAMPLE),
                delay_against_still: true,
                ..Params::default()
            },
        );
        assert_eq!(plan.n_stages, want.n_stages, "{case}: number of aligners");
        assert_eq!(plan.q, want.q, "{case}: poses");
        assert_eq!(plan.unresolved, want.unresolved, "{case}: collisions left");
        assert_eq!(plan.log, want.log, "{case}: notes");
        let delays: Vec<&String> = plan
            .log
            .iter()
            .filter(|l| l.starts_with("Anti-collision delays"))
            .collect();
        if case == "demo_lower_mild" {
            assert_eq!(
                delays,
                ["Anti-collision delays (aligners): 41: 7"],
                "{case}"
            );
        } else {
            assert!(delays.is_empty(), "{case}: {delays:?}");
        }
    }
}

/// The staging settings of the case with the scene settings of the oracle
/// (`STAGING_PARAMS` of `golden/oracle/stages.py`, the plugin's names) and
/// the pattern `pattern`.
fn settings_of(scene: &Value, pattern: &str) -> Settings {
    let mut s = Settings::default();
    for (name, v) in scene.as_object().unwrap() {
        let st = &mut s.staging;
        match name.as_str() {
            "perio" => {
                st.perio =
                    serde_json::from_value(Value::String(v.as_str().unwrap().to_lowercase()))
                        .unwrap();
            }
            "collision" => st.collision_check = v.as_bool().unwrap(),
            "overcorrection" => st.overcorrection = v.as_bool().unwrap(),
            "interval_days" => s.interval_days = v.as_u64().unwrap() as u32,
            other => {
                let field = match other {
                    "step_rotation" | "step_rotation_round" | "step_angular" => {
                        format!("{other}_deg")
                    }
                    _ => format!("{other}_mm"),
                };
                assert!(st.set_number(&field, num(v)), "{other}");
            }
        }
    }
    s.staging.pattern = serde_json::from_value(Value::String(pattern.to_lowercase())).unwrap();
    assert!(s.staging.out_of_range().is_none());
    s
}

/// The calendar of a plan of the port (`schedule::Input` as the case
/// stores it: the movements of each aligner, the IPR with its amount).
fn calendar_input(plan: &staging::Plan, ipr: &[IprPlan]) -> schedule::Input {
    schedule::Input {
        n_stages: plan.n_stages,
        n_total: None,
        q: plan.q.clone(),
        ipr: plan
            .ipr_stage
            .iter()
            .map(|r| schedule::IprAt {
                a: r.a,
                b: r.b,
                stage: r.stage,
                mm: ipr
                    .iter()
                    .find(|p| p.a == r.a && p.b == r.b)
                    .map_or(0.0, |p| p.mm),
            })
            .collect(),
        attachments: Vec::new(),
        offset: 0,
    }
}

/// Every staging parameter of the plugin's page away from its default (a
/// periodontal stage, every limit, the contact tolerance, the
/// overcorrection), each explicit pattern, the tooth that needs the most
/// aligners slowed to half its speed, and the FIXED and ADAPTIVE calendars
/// of 10 days (`staging/staging_params.json`). The settings go through the
/// case settings as the command reads them.
#[test]
fn non_default_settings_and_a_slowed_tooth_match_the_plugin() {
    for case in CASES {
        let c = load(case);
        let want = json(case, "staging/staging_params.json");
        let inputs = &want["inputs"];
        let slowed: Vec<(Fdi, f64)> = inputs["speed"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(f, s)| (fdi(f.parse().unwrap()), num(s)))
            .collect();
        assert_eq!(slowed.len(), 1, "{case}");
        let mut moves = c.moves.clone();
        for m in &mut moves {
            if let Some((_, s)) = slowed.iter().find(|(f, _)| *f == m.fdi) {
                m.speed = *s;
            }
        }
        for (pattern, entry) in want["plans"].as_object().unwrap() {
            let label = format!("{case} {pattern}");
            let settings = settings_of(&inputs["settings"], pattern);
            let plan = staging::run(
                &staging::Input {
                    moves: &moves,
                    ipr: &c.ipr,
                    complexity: c.complexity,
                },
                &plugin(Params::from_settings(&settings.staging)),
            )
            .unwrap();
            let (worst_mm, worst_deg) = assert_plan_matches(&label, &plan, &entry["plan"]);
            assert_eq!(
                entry["plan"]["speed"], inputs["speed"],
                "{label}: the plugin's plan lists the slowed tooth"
            );
            let input = calendar_input(&plan, &c.ipr);
            for (policy, name) in [
                (IntervalPolicy::Fixed, "FIXED"),
                (IntervalPolicy::Adaptive, "ADAPTIVE"),
            ] {
                let mut s = settings.clone();
                s.interval_policy = policy;
                s.start_date = inputs["start_date"].as_str().map(str::to_owned);
                let cal = schedule::run(
                    &input,
                    &schedule::Params {
                        start_date: s.start_date.clone(),
                        interval_days: s.interval_days,
                        policy: s.interval_policy.into(),
                        ..schedule::Params::default()
                    },
                )
                .unwrap();
                assert_calendar_matches(&format!("{label} {name}"), &cal, &entry["calendar"][name]);
            }
            eprintln!(
                "{label}: {} aligners, worst pose error {worst_mm:.2e} mm / {worst_deg:.2e} deg",
                plan.n_stages
            );
        }
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

/// The calendar of the port against the plugin's (`{intervals, calendar}`
/// of the oracle): the days of wear of every aligner, the start, the end,
/// the total days, and the number, dates, treatment day and to-do items of
/// every row.
fn assert_calendar_matches(label: &str, cal: &schedule::Calendar, want: &Value) {
    let want_iv: Vec<u32> = want["intervals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u32)
        .collect();
    let got_iv: Vec<u32> = cal.rows.iter().map(|r| r.days).collect();
    assert_eq!(got_iv, want_iv, "{label}: intervals");
    let wc = &want["calendar"];
    assert_eq!(cal.start.to_iso(), wc["start"].as_str().unwrap(), "{label}");
    assert_eq!(cal.end.to_iso(), wc["end"].as_str().unwrap(), "{label}");
    assert_eq!(
        u64::from(cal.total_days),
        wc["total_days"].as_u64().unwrap(),
        "{label}"
    );
    assert_eq!(
        cal.rows.len(),
        wc["rows"].as_array().unwrap().len(),
        "{label}"
    );
    for (r, w) in cal.rows.iter().zip(wc["rows"].as_array().unwrap()) {
        assert_eq!(u64::from(r.number), w["number"].as_u64().unwrap());
        assert_eq!(r.passive, w["passive"].as_bool().unwrap(), "{label}");
        assert_eq!(r.start.to_iso(), w["start"].as_str().unwrap(), "{label}");
        assert_eq!(r.end.to_iso(), w["end"].as_str().unwrap(), "{label}");
        assert_eq!(u64::from(r.first_day), w["first_day"].as_u64().unwrap());
        let todo: Vec<&str> = w["todo"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(r.todo, todo, "{label} aligner {}", r.number);
    }
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
            assert_calendar_matches(&format!("{case} {name}"), &cal, &expected["policies"][name]);
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
