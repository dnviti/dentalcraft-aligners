//! Tests of the ramps: shape, placement on the teeth and the union with a
//! shell. The placement against the plugin is `tests/aligner_bite_oracle.rs`.

use super::*;
use dc_geom::sdf::SdfGrid;
use glam::{DQuat, Vec3};

fn fdi(n: u8) -> Fdi {
    Fdi::new(n).unwrap()
}

fn box_mesh(lo: Vec3, hi: Vec3) -> TriMesh {
    let p: Vec<Vec3> = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let t = quads
        .iter()
        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
        .collect();
    TriMesh::new(p, t)
}

fn volume(m: &TriMesh) -> f64 {
    m.tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| m.positions[i as usize].as_dvec3());
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn bbox(v: &[DVec3]) -> (DVec3, DVec3) {
    v.iter().fold(
        (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

#[test]
fn the_unit_ramp_is_a_closed_outward_solid_with_the_plugin_dimensions() {
    let m = unit_mesh();
    let report = m.validate();
    assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
    assert!(volume(&m) > 0.0, "outward faces");
    let (v, t) = ramp_mesh(N_AROUND, N_UP);
    assert_eq!(v.len(), N_AROUND * (N_UP + 1) + 2);
    assert_eq!(t.len(), 2 * N_AROUND * N_UP + 2 * N_AROUND);
    let (lo, hi) = bbox(&v);
    // half axes 0.5 x 0.5 across, the base at -EMBED, the top at 1
    assert!((hi.x - 0.5).abs() < 1e-12 && (lo.x + 0.5).abs() < 1e-12);
    assert!((hi.y - 0.5).abs() < 1e-12 && (lo.y + 0.5).abs() < 1e-12);
    assert!((lo.z + EMBED).abs() < 1e-12 && (hi.z - 1.0).abs() < 1e-12);
}

#[test]
fn a_ramp_has_the_size_of_its_settings_and_follows_its_tooth() {
    let s = RampSettings::default();
    let size = [s.width_mm, s.depth_mm, s.height_mm];
    let r = new_ramp(
        fdi(11),
        &frame(DVec3::new(1.0, 2.0, 3.0), DVec3::Z, None),
        size,
        RampKind::Auto,
    );
    // the pose round-trips the 4x4
    assert!((r.pose.t - DVec3::new(1.0, 2.0, 3.0)).length() < 1e-12);
    assert!(r.pose.q.angle_between(DQuat::IDENTITY) < 1e-9);
    let at_rest = BTreeMap::from([(fdi(11), DMat4::IDENTITY)]);
    let s0 = &solids(&[r], &at_rest)[0];
    let (lo, hi) = bbox(&s0.vertices);
    // within 0.05 mm of the plugin's: width x depth x height, base EMBED * height deep
    assert!(((hi.x - lo.x) - 3.0).abs() < 0.05, "{lo} {hi}");
    assert!(((hi.y - lo.y) - 2.0).abs() < 0.05);
    assert!(((hi.z - 3.0) - 1.5).abs() < 0.05);
    assert!(((3.0 - lo.z) - EMBED * 1.5).abs() < 0.05);
    let mesh = s0.mesh();
    assert!(mesh.validate().is_watertight());
    assert!((volume(&mesh) - 3.0 * 2.0 * 1.5 * volume(&unit_mesh())).abs() < 1e-6);
    // the tooth moves 0.3 mm and turns 90 degrees about Z: the ramp goes with it
    let d = DMat4::from_rotation_translation(
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
        DVec3::new(0.3, 0.0, 0.0),
    );
    let s1 = &solids(&[r], &BTreeMap::from([(fdi(11), d)]))[0];
    for (a, b) in s0.vertices.iter().zip(&s1.vertices) {
        assert!((d.transform_point3(*a) - *b).length() < 1e-9);
    }
    // a ramp of a tooth that is not placed is left out
    assert!(solids(&[r], &BTreeMap::new()).is_empty());
    let (p, l) = points(std::slice::from_ref(s0));
    assert_eq!(p.len(), l.len());
    assert!(l.iter().all(|&f| f == 11));
}

#[test]
fn the_frame_puts_z_along_the_normal_and_x_towards_the_hint() {
    let m = frame(
        DVec3::new(1.0, 2.0, 3.0),
        DVec3::new(0.0, 2.0, 0.0),
        Some(DVec3::new(1.0, 1.0, 1.0)),
    );
    assert!((m.z_axis.truncate() - DVec3::Y).length() < 1e-12);
    assert!(m.x_axis.truncate().dot(DVec3::Y).abs() < 1e-12);
    assert!(m.x_axis.truncate().dot(DVec3::new(1.0, 1.0, 1.0)) > 0.0);
    // right handed
    assert!(
        (m.x_axis.truncate().cross(m.y_axis.truncate()) - m.z_axis.truncate()).length() < 1e-12
    );
    // a hint along the normal falls back to a perpendicular
    let m = frame(DVec3::ZERO, DVec3::Z, Some(DVec3::Z));
    assert!(m.x_axis.truncate().dot(DVec3::Z).abs() < 1e-12);
    assert!((m.x_axis.truncate().length() - 1.0).abs() < 1e-12);
}

#[test]
fn a_manual_ramp_stands_on_the_outer_surface_of_the_aligner() {
    let r = manual_ramp(
        fdi(21),
        DVec3::new(0.0, 0.0, 5.0),
        DVec3::new(0.0, 0.0, 2.0),
        [3.0, 2.0, 1.5],
        0.1,
        0.9,
    );
    assert_eq!(r.kind, RampKind::Manual);
    assert!((r.pose.t - DVec3::new(0.0, 0.0, 6.0)).length() < 1e-12);
}

#[test]
fn the_ramps_are_united_with_the_shell_and_the_tooth_stays_free() {
    // a slab of shell 1 mm thick over a tooth block that reaches into the base of the ramp
    let shell = box_mesh(Vec3::new(-8.0, -8.0, 0.0), Vec3::new(8.0, 8.0, 1.0));
    let tooth = box_mesh(Vec3::new(-8.0, -8.0, -4.0), Vec3::new(8.0, 8.0, 0.8));
    let teeth = BTreeMap::from([(11, tooth)]);
    let on = new_ramp(
        fdi(11),
        &frame(DVec3::new(0.0, 0.0, 1.0), DVec3::Z, None),
        [3.0, 2.0, 1.5],
        RampKind::Auto,
    );
    // far above the shell: not on it
    let off = new_ramp(
        fdi(11),
        &frame(DVec3::new(5.0, 5.0, 6.0), DVec3::Z, None),
        [1.0, 1.0, 1.0],
        RampKind::Auto,
    );
    let to_arch = BTreeMap::from([(fdi(11), DMat4::IDENTITY)]);
    let all = solids(&[on, off], &to_arch);
    let (out, notes) = apply_to_mesh(&shell, &all, &teeth, 0.1).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("11"));
    let report = out.validate();
    assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
    let v: Vec<DVec3> = out.positions.iter().map(|p| p.as_dvec3()).collect();
    let (lo, hi) = bbox(&v);
    assert!((hi.z - 2.5).abs() < 0.1, "top of the ramp {hi}");
    // the tooth block takes the shell below z = 0.8: only 0.2 mm of slab is left, and the ramp on it
    assert!(lo.z >= 0.7, "the cavity of the tooth stays free: {lo}");
    let unit = volume(&unit_mesh());
    let slab = 16.0 * 16.0 * 0.2;
    let grown = volume(&out) - slab;
    assert!(
        grown > 0.0 && grown < 3.0 * 2.0 * 1.5 * unit + 1.0,
        "{grown}"
    );
    // no ramps: the shell as it is
    let (same, none) = apply_to_mesh(&shell, &[], &teeth, 0.1).unwrap();
    assert!(none.is_empty() && same == shell);
}

#[test]
fn the_cavity_of_the_tooth_is_cut_again() {
    // the ramp base goes down to z = 0.475; the tooth reaches z = 0.8: that part is not material
    let shell = box_mesh(Vec3::new(-8.0, -8.0, 0.0), Vec3::new(8.0, 8.0, 1.0));
    let tooth = box_mesh(Vec3::new(-2.0, -2.0, -4.0), Vec3::new(2.0, 2.0, 0.8));
    let params = dc_geom::sdf::SdfParams {
        voxel_size: 0.1,
        band: 0.5,
    };
    let mut grid = SdfGrid::from_mesh(&shell, &params).unwrap();
    let ramp = new_ramp(
        fdi(11),
        &frame(DVec3::new(0.0, 0.0, 1.0), DVec3::Z, None),
        [3.0, 2.0, 1.5],
        RampKind::Auto,
    );
    let all = solids(&[ramp], &BTreeMap::from([(fdi(11), DMat4::IDENTITY)]));
    let notes = apply_to_grid(&mut grid, &all, &BTreeMap::from([(11, tooth)])).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert!(
        grid.sample(Vec3::new(0.0, 0.0, 0.5)) > 0.0,
        "inside the tooth: free"
    );
    assert!(grid.sample(Vec3::new(0.0, 0.0, 1.4)) < 0.0, "in the ramp");
    assert!(
        grid.sample(Vec3::new(6.0, 0.0, 0.5)) < 0.0,
        "in the shell away from the tooth"
    );
}
