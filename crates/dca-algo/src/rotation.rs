//! Rotations: the one conversion of the port between unit quaternions,
//! rotation matrices, rotation vectors and rotation angles.
//!
//! Reference: `ortho_aligner/meshutil.py` (`rotvec_to_matrix`,
//! `matrix_to_rotvec`).
//!
//! A rotation has two unit quaternions, `q` and `-q`. Composing poses
//! ([`Pose::compose`]) or reading a matrix (`DQuat::from_mat3`) gives either
//! of them, and glam reads a quaternion with `w < 0` the long way round:
//! `to_axis_angle` and `to_scaled_axis` give an angle in `(π, 2π)`. A tooth
//! turned by 4 degrees then reads as turned by 356 degrees the other way, and
//! a staging that divides that rotation vector into aligners spins the tooth
//! a whole turn over hundreds of them.
//!
//! Every conversion here takes the shortest rotation, as the plugin does
//! (its `matrix_to_rotvec` takes the angle from the trace): the canonical
//! quaternion has `w >= 0` ([`canonical`]) and every angle is in `[0, π]`.
//! At exactly 180 degrees both axes are as short; the canonical one has its
//! largest component positive, the plugin's choice there.

use dc_case::Pose;
use glam::{DMat3, DMat4, DQuat, DVec3};

/// The component of `v` with the largest magnitude (the first one on a tie).
fn largest(v: DVec3) -> f64 {
    if v.x.abs() >= v.y.abs() && v.x.abs() >= v.z.abs() {
        v.x
    } else if v.y.abs() >= v.z.abs() {
        v.y
    } else {
        v.z
    }
}

/// The unit quaternion of the rotation `q` that takes the shortest way to
/// it: `w >= 0`, and at exactly 180 degrees (`w = 0`) the largest component
/// of the axis positive.
pub fn canonical(q: DQuat) -> DQuat {
    let q = q.normalize();
    let flip = if q.w == 0.0 {
        largest(q.xyz()) < 0.0
    } else {
        q.w < 0.0
    };
    if flip { -q } else { q }
}

/// The pose `p` with its rotation [`canonical`].
pub fn canonical_pose(p: &Pose) -> Pose {
    Pose {
        t: p.t,
        q: canonical(p.q),
    }
}

/// The rigid part of a 4x4 matrix as a pose with a [`canonical`] rotation.
pub fn pose_from_mat4(m: &DMat4) -> Pose {
    canonical_pose(&Pose::from_mat4(m))
}

/// Angle of the rotation `q` (radians, in `[0, π]`).
pub fn angle(q: DQuat) -> f64 {
    2.0 * q.xyz().length().atan2(q.w.abs())
}

/// Angle of the rotation of the pose `p` (degrees, in `[0, 180]`).
pub fn degrees(p: &Pose) -> f64 {
    angle(p.q).to_degrees()
}

/// Rotation vector of `q`: its axis times its angle, the angle in `[0, π]`.
pub fn to_rotvec(q: DQuat) -> DVec3 {
    let q = canonical(q);
    let v = q.xyz();
    let s = v.length();
    if s == 0.0 {
        return DVec3::ZERO;
    }
    v * (2.0 * s.atan2(q.w) / s)
}

/// The [`canonical`] unit quaternion of the rotation vector `r` (axis times
/// angle in radians).
pub fn from_rotvec(r: DVec3) -> DQuat {
    canonical(DQuat::from_scaled_axis(r))
}

/// Rotation matrix of the rotation vector `r` (`meshutil.rotvec_to_matrix`).
pub fn rotvec_to_matrix(r: DVec3) -> DMat3 {
    DMat3::from_quat(from_rotvec(r))
}

/// Rotation vector of a rotation matrix, angle in `[0, π]`
/// (`meshutil.matrix_to_rotvec`).
pub fn matrix_to_rotvec(m: &DMat3) -> DVec3 {
    to_rotvec(DQuat::from_mat3(m))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::frame::rot_axis_angle;

    /// The plugin's `matrix_to_rotvec` (angle from the trace).
    fn plugin_rotvec(r: &DMat3) -> DVec3 {
        let tr = r.x_axis.x + r.y_axis.y + r.z_axis.z;
        let th = ((tr - 1.0) / 2.0).clamp(-1.0, 1.0).acos();
        if th < 1e-9 {
            return DVec3::ZERO;
        }
        let w = DVec3::new(
            r.y_axis.z - r.z_axis.y,
            r.z_axis.x - r.x_axis.z,
            r.x_axis.y - r.y_axis.x,
        ) / (2.0 * th.sin());
        w * th
    }

    #[test]
    fn a_quaternion_with_negative_w_is_the_short_rotation() {
        // a tooth turned 4 degrees, as the setup stored it with w < 0
        let q = DQuat::from_axis_angle(DVec3::new(0.2, -0.3, 1.0).normalize(), 4f64.to_radians());
        let neg = -q;
        assert!(neg.w < 0.0);
        // glam reads it the long way round
        assert!((neg.to_axis_angle().1.to_degrees() - 356.0).abs() < 1e-9);
        assert_eq!(canonical(neg), q);
        assert!((angle(neg).to_degrees() - 4.0).abs() < 1e-9);
        assert!((degrees(&Pose::from_rotation(neg)) - 4.0).abs() < 1e-9);
        let (r, rn) = (to_rotvec(q), to_rotvec(neg));
        assert!((r - rn).length() < 1e-15, "{r} {rn}");
        assert!((r.length().to_degrees() - 4.0).abs() < 1e-9);
        assert!(r.dot(DVec3::new(0.2, -0.3, 1.0)) > 0.0);
        assert!(from_rotvec(rn).abs_diff_eq(q, 1e-15));
        let p = canonical_pose(&Pose::new(DVec3::X, neg));
        assert_eq!((p.t, p.q), (DVec3::X, q));
    }

    #[test]
    fn rotations_next_to_180_degrees_take_the_short_way() {
        for deg in [179.0, 179.999, 180.001, 181.0, 270.0, 359.0] {
            let q = DQuat::from_axis_angle(DVec3::Z, f64::to_radians(deg));
            // the same rotation, the short way: about -z beyond 180 degrees
            let short = if deg > 180.0 { 360.0 - deg } else { deg };
            let r = to_rotvec(q);
            assert!((r.length().to_degrees() - short).abs() < 1e-9, "{deg}: {r}");
            assert!(r.length() <= PI, "{deg}");
            assert_eq!(r.z.signum(), if deg > 180.0 { -1.0 } else { 1.0 }, "{deg}");
            assert!((angle(q).to_degrees() - short).abs() < 1e-9, "{deg}");
            assert!(canonical(q).w >= 0.0 && canonical(-q).w >= 0.0);
            assert!(rotvec_to_matrix(r).abs_diff_eq(DMat3::from_quat(q), 1e-12));
        }
        // exactly 180 degrees: both signs give the plugin's axis (largest component positive)
        for axis in [DVec3::Z, DVec3::NEG_Z, DVec3::new(-0.6, 0.0, 0.8)] {
            let q = DQuat::from_xyzw(axis.x, axis.y, axis.z, 0.0);
            for v in [to_rotvec(q), to_rotvec(-q)] {
                assert!((v.length() - PI).abs() < 1e-15);
                assert!(largest(v) > 0.0, "{axis}: {v}");
                assert!(rotvec_to_matrix(v).abs_diff_eq(DMat3::from_quat(q), 1e-12));
            }
        }
    }

    #[test]
    fn matrices_give_the_plugins_rotation_vector() {
        // glam's quaternion of a matrix can have w < 0 beyond 90 degrees: the old
        // conversion gave angles above 180 degrees there
        let mut worst = 0.0_f64;
        let mut flipped = 0;
        for i in 0..400 {
            let t = f64::from(i);
            let axis = DVec3::new((t * 0.71).sin(), (t * 1.37).cos(), (t * 0.29).sin() - 0.3);
            let ang = (t * 0.0157).rem_euclid(PI - 1e-3);
            let m = rot_axis_angle(axis, ang);
            if DQuat::from_mat3(&m).w < 0.0 {
                flipped += 1;
            }
            let v = matrix_to_rotvec(&m);
            assert!(v.length() <= PI + 1e-12, "{i}");
            worst = worst.max((v - plugin_rotvec(&m)).length());
            assert!(rotvec_to_matrix(v).abs_diff_eq(m, 1e-12), "{i}");
        }
        assert!(flipped > 10, "{flipped}");
        assert!(worst < 1e-9, "{worst}");
    }

    #[test]
    fn conversions_round_trip() {
        let r = rot_axis_angle(DVec3::new(1.0, 2.0, -0.5), 0.8);
        let v = matrix_to_rotvec(&r);
        assert!((v.length() - 0.8).abs() < 1e-12);
        assert!(rotvec_to_matrix(v).abs_diff_eq(r, 1e-12));
        assert_eq!(rotvec_to_matrix(DVec3::ZERO), DMat3::IDENTITY);
        assert_eq!(matrix_to_rotvec(&DMat3::IDENTITY), DVec3::ZERO);
        assert_eq!(to_rotvec(-DQuat::IDENTITY), DVec3::ZERO);
        assert_eq!(from_rotvec(DVec3::ZERO), DQuat::IDENTITY);
        // 180 degrees
        let half = rot_axis_angle(DVec3::Z, PI);
        assert!((matrix_to_rotvec(&half).length() - PI).abs() < 1e-9);
        // a tiny rotation is kept, not rounded to zero
        let tiny = DVec3::new(3e-10, -1e-10, 2e-10);
        assert!((to_rotvec(from_rotvec(tiny)) - tiny).length() < 1e-24);
        // the pose of a matrix
        let m = DMat4::from_rotation_translation(
            DQuat::from_axis_angle(DVec3::new(1.0, -2.0, 0.5).normalize(), 2.5),
            DVec3::new(1.0, 2.0, 3.0),
        );
        let p = pose_from_mat4(&m);
        assert!(p.q.w >= 0.0);
        assert!(p.to_mat4().abs_diff_eq(m, 1e-12));
        assert!((degrees(&p) - 2.5f64.to_degrees()).abs() < 1e-9);
        assert!(
            (degrees(&Pose::from_rotation(DQuat::from_rotation_x(FRAC_PI_2))) - 90.0).abs() < 1e-12
        );
    }
}
