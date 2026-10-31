//! Unit tests of the visit analysis; the oracle and the synthetic visit are
//! in `tests/visits_oracle.rs` and `tests/visits_synthetic.rs`.

use super::*;
use glam::DQuat;

#[test]
fn sub_idx_is_linspace_truncated() {
    assert_eq!(sub_idx(5, 10), vec![0, 1, 2, 3, 4]);
    // np.linspace(0, 9, 4).astype(int) == [0, 3, 6, 9]
    assert_eq!(sub_idx(10, 4), vec![0, 3, 6, 9]);
    // np.linspace(0, 99, 7).astype(int) == [0, 16, 33, 49, 66, 82, 99]
    assert_eq!(sub_idx(100, 7), vec![0, 16, 33, 49, 66, 82, 99]);
}

#[test]
fn quantile_and_median_match_numpy() {
    let v = [4.0, 1.0, 3.0, 2.0];
    assert!((quantile(&v, 0.5) - 2.5).abs() < 1e-15);
    assert!((quantile(&v, 0.8) - 3.4).abs() < 1e-12);
    assert_eq!(quantile(&v, 1.0), 4.0);
    assert_eq!(median(&v), 2.5);
    assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
}

#[test]
fn deviation_components_follow_the_tooth_axes() {
    // expected frame turned 90 degrees about z: its x axis is the world y axis
    let e = DMat4::from_quat(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2));
    // a world shift of +y is +x (mesial when md_sign = 1) in the tooth frame
    let t = DMat4::from_translation(DVec3::new(0.0, 0.3, 0.0));
    let d = tooth_deviation(&e, &t, DVec3::ZERO, 1);
    assert!((d.mesial_mm - 0.3).abs() < 1e-12 && d.buccal_mm.abs() < 1e-12);
    let d = tooth_deviation(&e, &t, DVec3::ZERO, -1);
    assert!((d.mesial_mm + 0.3).abs() < 1e-12);
    // a turn of 2 degrees about the tooth's long axis is the rotation
    let r = DMat4::from_quat(DQuat::from_axis_angle(DVec3::Z, 2f64.to_radians()));
    let d = tooth_deviation(&e, &r, DVec3::ZERO, 1);
    assert!((d.rotation_deg - 2.0).abs() < 1e-9 && d.tip_deg.abs() < 1e-9);
    assert!((d.angular_deg() - 2.0).abs() < 1e-9);
}

#[test]
fn icp_recovers_a_rigid_motion_of_a_surface() {
    // a bumpy patch z = f(x, y) with its analytic normals
    let f = |x: f64, y: f64| 2.0 * (0.4 * x).sin() * (0.3 * y).cos() + 0.05 * x * y / 10.0;
    let grad = |x: f64, y: f64| {
        (
            0.8 * (0.4 * x).cos() * (0.3 * y).cos() + 0.005 * y,
            -0.6 * (0.4 * x).sin() * (0.3 * y).sin() + 0.005 * x,
        )
    };
    let mut pts = Vec::new();
    let mut nrm = Vec::new();
    for i in 0..80 {
        for j in 0..80 {
            let (x, y) = (-12.0 + 0.3 * i as f64, -12.0 + 0.3 * j as f64);
            pts.push(DVec3::new(x, y, f(x, y)));
            let (gx, gy) = grad(x, y);
            nrm.push(DVec3::new(-gx, -gy, 1.0).normalize());
        }
    }
    let truth = DMat4::from_rotation_translation(
        DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 0.5).normalize(), 0.02),
        DVec3::new(0.15, -0.1, 0.12),
    );
    let moved = xform(&truth, &pts);
    let r = rot(&truth);
    let moved_n: Vec<DVec3> = nrm.iter().map(|n| r * *n).collect();
    let target = Target::new(moved, Some(moved_n));
    let (t, err) = icp(
        &pts,
        Some(&nrm),
        &target,
        DMat4::IDENTITY,
        IcpOpts {
            iters: 60,
            trim: 0.8,
        },
    );
    assert!(err < 0.05, "rms {err}");
    let got = t.transform_point3(DVec3::new(5.0, 3.0, 2.0));
    let want = truth.transform_point3(DVec3::new(5.0, 3.0, 2.0));
    assert!(got.distance(want) < 0.05, "{got:?} vs {want:?}");
}

#[test]
fn pose_at_uses_the_latest_stage_with_the_tooth() {
    let f = Fdi::new(11).unwrap();
    let mut s1 = Stage::new(1);
    s1.poses
        .insert(f, Pose::from_translation(DVec3::new(0.25, 0.0, 0.0)));
    let s2 = Stage::new(2);
    let stages = [s1, s2];
    assert_eq!(pose_at(&stages, f, 0), Pose::default());
    assert_eq!(pose_at(&stages, f, 1).t.x, 0.25);
    assert_eq!(pose_at(&stages, f, 2).t.x, 0.25);
    assert_eq!(pose_at(&stages, f, 9).t.x, 0.25);
}

#[test]
fn the_decision_follows_the_teeth() {
    let reg = Registration {
        transform: DMat4::IDENTITY,
        rms_mm: 0.05,
        normal_sign: 1.0,
    };
    let tracking = |on_track| ToothTracking {
        on_track,
        linear_mm: 0.0,
        angular_deg: 0.0,
        efficacy_pct: None,
        planned_mm: 0.0,
        rms_mm: 0.0,
        scan_points: 100,
        deviation: ToothDeviation::default(),
        achieved: Pose::default(),
    };
    let f = |n| Fdi::new(n).unwrap();
    let inputs: Vec<ToothInput> = [11, 12, 13]
        .iter()
        .map(|&n| ToothInput {
            fdi: f(n),
            rest: DMat4::IDENTITY,
            expected: DMat4::IDENTITY,
            points: vec![],
            normals: vec![],
            md_sign: 1,
        })
        .collect();
    let p = Params::default();
    let ok = BTreeMap::from([(f(11), tracking(true)), (f(12), tracking(true))]);
    let out = summarize(reg, ok, &inputs, &p);
    assert_eq!(out.decision, Decision::OnTrack);
    assert_eq!(out.untracked, vec![f(13)]);
    assert!(out.speed.is_empty() && out.on_track == 2);
    let bad = BTreeMap::from([(f(11), tracking(true)), (f(12), tracking(false))]);
    let out = summarize(reg, bad, &inputs, &p);
    assert_eq!(out.decision, Decision::Refinement);
    assert_eq!(out.speed, BTreeMap::from([(f(12), p.lag_speed)]));
}
