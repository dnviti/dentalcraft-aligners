//! A simulated control visit on a golden demo arch (`dca_algo::visits`).
//!
//! The scan of the case is taken to stage 3 of a made-up plan (every tooth
//! moved by a known pose, the gingiva following), then every tooth is moved
//! again by a known deviation (the "achieved" position), noise is added to
//! every vertex and the whole scan is placed anywhere in space. The analysis
//! must put the scan back on the plan and find the deviation of every tooth
//! within 0.02 mm and 0.1 degrees.

mod golden_case;

use std::collections::BTreeMap;

use dc_case::{Fdi, Pose};
use dca_algo::anatomy::Arch;
use dca_algo::demo::{Draws, SeededDraws};
use dca_algo::frame;
use dca_algo::gingiva;
use dca_algo::visits::{self, MeshRef, ToothInput};
use dca_model::{Decision, Stage};
use glam::{DMat3, DMat4, DQuat, DVec3};
use golden_case::Case;

const WORN: u32 = 3;
/// Registration of the whole scan, in mm.
const TOL_SCAN_MM: f64 = 0.02;

fn rot_about(c: DVec3, axis: DVec3, deg: f64, shift: DVec3) -> DMat4 {
    let q = DQuat::from_axis_angle(axis.normalize(), deg.to_radians());
    DMat4::from_translation(c + shift) * DMat4::from_quat(q) * DMat4::from_translation(-c)
}

/// Crown points and normals of a tooth in its rest frame: the labelled
/// vertices with the triangles that have the three of them.
fn crown(case: &Case, f: Fdi, rest_inv: &DMat4) -> (Vec<DVec3>, Vec<DVec3>) {
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
    let r = DMat3::from_mat4(*rest_inv);
    (
        idx.iter()
            .map(|&i| rest_inv.transform_point3(case.vertices[i]))
            .collect(),
        idx.iter().map(|&i| r * normals[i]).collect(),
    )
}

/// How far the recovered world pose is from the true one: (mm at the crown
/// centre, degrees).
fn pose_error(got: &DMat4, truth: &DMat4, c: DVec3) -> (f64, f64) {
    let e = *got * frame::rigid_inverse(truth);
    let mm = e.transform_point3(c).distance(c);
    let q = DQuat::from_mat3(&DMat3::from_mat4(e)).normalize();
    (mm, q.to_scaled_axis().length().to_degrees())
}

/// Runs a simulated visit and checks the decisions; returns the worst error
/// of the recovered teeth (mm at the crown centre, degrees).
fn simulate(seed: u64, noise_mm: f64) -> (f64, f64) {
    let case = Case::load("demo_lower_mild");
    let fdis = case.fdis();
    let mut rng = SeededDraws::new(seed);

    let rest: BTreeMap<Fdi, Pose> = fdis
        .iter()
        .map(|&f| (f, Pose::from_mat4(&case.rest_frame(f))))
        .collect();
    // the plan: the pose of every tooth at stage 3, in its rest frame
    let mut stage = Stage::new(WORN);
    for &f in &fdis {
        let t = DVec3::new(
            rng.uniform(-0.6, 0.6),
            rng.uniform(-0.6, 0.6),
            rng.uniform(-0.3, 0.3),
        );
        let q = DQuat::from_rotation_z(rng.uniform(-4.0, 4.0).to_radians());
        stage.poses.insert(f, Pose::new(t, q));
    }
    let stages = [Stage::new(1), Stage::new(2), stage];

    // expected model and teeth
    let blend = gingiva::scan_blend(&case.vertices, &case.tris, &case.labels).unwrap();
    let planned: BTreeMap<Fdi, DMat4> = fdis
        .iter()
        .map(|&f| (f, visits::delta_at(&rest[&f], &stages, f, WORN)))
        .collect();
    let expected = gingiva::deform_scan(&case.vertices, &blend, &planned);

    let mut teeth = Vec::new();
    for &f in &fdis {
        let rest_m = rest[&f].to_mat4();
        let (points, normals) = crown(&case, f, &frame::rigid_inverse(&rest_m));
        let md = case.teeth["teeth"][f.to_string()]["md_sign"]
            .as_i64()
            .unwrap() as i8;
        teeth.push(ToothInput {
            fdi: f,
            rest: rest_m,
            expected: visits::expected_frame(&rest[&f], &stages, f, WORN),
            points,
            normals,
            md_sign: md,
        });
    }

    // what the patient's teeth really did: a deviation on top of the plan.
    // Two teeth are well behind (a shift, a turn), the others within 0.3 mm / 1 degree.
    let behind_shift = fdis[3];
    let behind_turn = fdis[9];
    let mut deviation: BTreeMap<Fdi, DMat4> = BTreeMap::new();
    for t in &teeth {
        let c = t
            .expected
            .transform_point3(t.points.iter().copied().sum::<DVec3>() / t.points.len() as f64);
        let (shift, deg) = if t.fdi == behind_shift {
            (DVec3::new(0.9, -0.5, 0.2), 0.5)
        } else if t.fdi == behind_turn {
            (DVec3::new(0.1, 0.1, 0.0), 4.0)
        } else {
            (
                DVec3::new(
                    rng.uniform(-0.2, 0.2),
                    rng.uniform(-0.2, 0.2),
                    rng.uniform(-0.2, 0.2),
                ),
                rng.uniform(-0.8, 0.8),
            )
        };
        let axis = DVec3::new(rng.uniform(-1.0, 1.0), rng.uniform(-1.0, 1.0), 1.0);
        deviation.insert(t.fdi, rot_about(c, axis, deg, shift));
    }
    let achieved: BTreeMap<Fdi, DMat4> = planned
        .iter()
        .map(|(&f, d)| (f, deviation[&f] * *d))
        .collect();
    let mut scan = gingiva::deform_scan(&case.vertices, &blend, &achieved);
    for p in &mut scan {
        *p += DVec3::new(
            rng.normal(0.0, noise_mm),
            rng.normal(0.0, noise_mm),
            rng.normal(0.0, noise_mm),
        );
    }
    // the scanner puts it anywhere
    let placed = DMat4::from_translation(DVec3::new(14.0, -9.0, 6.0))
        * DMat4::from_quat(DQuat::from_axis_angle(
            DVec3::new(0.3, 0.2, 1.0).normalize(),
            25f64.to_radians(),
        ));
    let scan: Vec<DVec3> = scan.iter().map(|&p| placed.transform_point3(p)).collect();

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

    // the registration undoes the placement
    let back = out.registration.transform * placed;
    let probe = DVec3::new(10.0, 5.0, 0.0);
    assert!(back.transform_point3(probe).distance(probe) < TOL_SCAN_MM);
    assert!(out.registration.normal_sign > 0.0);
    println!("registration rms {:.4} mm", out.registration.rms_mm);

    assert!(out.untracked.is_empty(), "{:?}", out.untracked);
    assert_eq!(out.teeth.len(), fdis.len());
    let (mut worst_mm, mut worst_deg) = (0.0_f64, 0.0_f64);
    for t in &teeth {
        let got = &out.teeth[&t.fdi];
        let w_got = t.rest * got.achieved.to_mat4();
        let w_true = deviation[&t.fdi] * t.expected;
        let c = t
            .expected
            .transform_point3(t.points.iter().copied().sum::<DVec3>() / t.points.len() as f64);
        let (mm, deg) = pose_error(&w_got, &w_true, c);
        println!(
            "{}: error {mm:.4} mm {deg:.3} deg, deviation {:.2} mm {:.1} deg, efficacy {:?}",
            t.fdi, got.linear_mm, got.angular_deg, got.efficacy_pct
        );
        worst_mm = worst_mm.max(mm);
        worst_deg = worst_deg.max(deg);
        let behind = t.fdi == behind_shift || t.fdi == behind_turn;
        assert_eq!(got.on_track, !behind, "{}", t.fdi);
    }
    println!("worst: {worst_mm:.4} mm, {worst_deg:.3} deg");
    assert_eq!(out.decision, Decision::Refinement);
    assert_eq!(out.on_track, fdis.len() - 2);
    assert_eq!(out.speed.keys().copied().collect::<Vec<_>>(), {
        let mut v = vec![behind_shift, behind_turn];
        v.sort();
        v
    });
    (worst_mm, worst_deg)
}

/// Scanner noise of 10 micrometres per coordinate: every tooth within
/// 0.02 mm and 0.1 degrees.
#[test]
fn a_simulated_visit_is_recovered_within_0_02_mm_and_0_1_degrees() {
    for seed in [2026, 11] {
        let (mm, deg) = simulate(seed, 0.01);
        assert!(mm < 0.02, "seed {seed}: {mm} mm");
        assert!(deg < 0.1, "seed {seed}: {deg} deg");
    }
}

/// Twice the noise: the teeth are still found within 0.03 mm and 0.2 degrees.
#[test]
fn a_noisier_scan_is_still_recovered() {
    let (mm, deg) = simulate(2026, 0.02);
    assert!(mm < 0.03, "{mm} mm");
    assert!(deg < 0.2, "{deg} deg");
}
