use super::*;
use crate::demo;

/// The demo arch (scan, labels, arch frame from its seeds).
fn demo_arch(arch: Arch) -> (TriMesh, Vec<u16>, ArchFrame) {
    let d = demo::run(&demo::Params {
        arch,
        ..demo::Params::default()
    });
    let labels = d.mesh.vertex_labels.clone().unwrap();
    let v = to_vec(&d.mesh);
    let f = frame::run(
        &frame::Input {
            arch,
            vertices: &v,
            seeds: &d.seeds,
            occlusal_flipped: false,
        },
        &frame::Params::default(),
    )
    .unwrap();
    (d.mesh, labels, f.arch_frame)
}

fn coarse(style: Style) -> Params {
    Params {
        style,
        voxel_mm: 0.5,
        tolerance_mm: 0.0,
        ..Params::default()
    }
}

fn single(arch: Arch, params: &Params) -> Output {
    let (mesh, labels, af) = demo_arch(arch);
    let a = ArchInput {
        scan: &mesh,
        labels: &labels,
        arch_frame: af,
    };
    let input = if arch.is_upper() {
        Input {
            upper: Some(a),
            lower: None,
        }
    } else {
        Input {
            upper: None,
            lower: Some(a),
        }
    };
    run(&input, params).unwrap()
}

#[test]
fn the_abo_upper_base_is_a_closed_heptagon_with_the_angles_of_the_params() {
    let params = coarse(Style::Abo);
    let out = single(Arch::Upper, &params);
    let r = &out.report.arches["upper"];
    let mesh = out.upper_base.as_ref().unwrap();
    assert!(mesh.validate().is_watertight(), "{r:?}");
    assert_eq!(r.components, 1);
    // seven walls: back, two heels, two sides, two front cuts
    let names: Vec<&str> = r
        .outline
        .edge_wall
        .iter()
        .map(|&k| r.outline.walls[k].name.as_str())
        .collect();
    assert_eq!(names.len(), 7, "{names:?}");
    let d = &r.design_angles_deg;
    assert!((d["back_heel"] - 115.0).abs() < 1e-6, "{d:?}");
    assert!((d["side_to_back_line"] - 65.0).abs() < 1e-6, "{d:?}");
    assert!((d["front_to_back_line"] - 25.0).abs() < 1e-6, "{d:?}");
    assert!((d["apex"] - 130.0).abs() < 1e-6, "{d:?}");
    // measured on the mesh
    let m = &r.measured_angles_deg;
    for (k, want) in [
        ("back_heel", 115.0),
        ("side_to_back_line", 65.0),
        ("apex", 130.0),
    ] {
        assert!((m[k] - want).abs() < 0.5, "{k}: {m:?}");
    }
    assert!(r.base_tilt_deg < 0.5, "{}", r.base_tilt_deg);
    assert!(
        (r.height_mm - 35.0).abs() < 1e-9
            || out
                .report
                .warnings
                .iter()
                .any(|w| w.code == "height_raised")
    );
    assert!((r.measured_height_mm - r.height_mm).abs() < 0.5, "{r:?}");
    assert!((r.art_mm - r.height_mm / 3.0).abs() < 1e-9);
}

#[test]
fn the_european_lower_base_has_vertical_walls_and_a_10_mm_base() {
    let params = coarse(Style::European);
    let out = single(Arch::Lower, &params);
    let r = &out.report.arches["lower"];
    assert!(out.lower_base.as_ref().unwrap().validate().is_watertight());
    assert!((r.art_mm - 10.0).abs() < 1e-9);
    assert!((r.anatomic_mm - (r.depth_mm + 3.0)).abs() < 1e-9);
    assert!(!r.walls.is_empty());
    for w in &r.walls {
        assert!((w.to_base_deg - 90.0).abs() < 0.5, "{w:?}");
    }
    assert!(r.base_tilt_deg < 0.5);
}

#[test]
fn the_result_is_deterministic_and_keeps_the_teeth() {
    let params = coarse(Style::Abo);
    let a = single(Arch::Lower, &params);
    let b = single(Arch::Lower, &params);
    let (ma, mb) = (a.lower_base.unwrap(), b.lower_base.unwrap());
    assert_eq!(ma.positions, mb.positions);
    assert_eq!(ma.tris, mb.tris);
    // every tooth vertex is inside or on the solid (within a voxel)
    let (mesh, labels, _) = demo_arch(Arch::Lower);
    let grid = SdfGrid::from_mesh(
        &ma,
        &SdfParams {
            voxel_size: 0.25,
            band: 1.0,
        },
    )
    .unwrap();
    let worst = mesh
        .positions
        .iter()
        .zip(&labels)
        .filter(|(_, l)| **l > 0)
        .map(|(p, _)| grid.sample(*p))
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(worst <= 0.5, "a tooth vertex {worst} mm outside the base");
}

#[test]
fn bad_parameters_and_inputs_are_refused() {
    assert!(matches!(
        run(&Input::default(), &Params::default()),
        Err(StudyBaseError::Invalid(_))
    ));
    let (mesh, labels, af) = demo_arch(Arch::Lower);
    let a = ArchInput {
        scan: &mesh,
        labels: &labels[1..],
        arch_frame: af,
    };
    let input = Input {
        upper: None,
        lower: Some(a),
    };
    assert!(matches!(
        run(&input, &Params::default()),
        Err(StudyBaseError::Invalid(_))
    ));
    let p = Params {
        side_angle_deg: 120.0,
        ..Params::default()
    };
    assert!(matches!(p.check(), Err(StudyBaseError::Invalid(_))));
}

/// A synthetic open arch scan, a sheet as an intraoral scan gives it, in
/// occlusion (case coordinates: incisors towards `+Y`, the patient's right
/// towards `+X`, the upper arch above the lower one, its teeth down). Across
/// the arch the crowns bulge out over a gum that recedes under them (an
/// undercut seen from the occlusal side, as under proclined upper incisors
/// and buccally flared crowns), then the gum flares out to the vestibule;
/// the upper sheet also holds the palate vault, deeper than the vestibule.
/// The crown points carry the FDI number of their tooth.
fn undercut_arch(arch: Arch) -> (TriMesh, Vec<u16>, ArchFrame) {
    // the profile across the arch, occlusal side up: (radius, height, crown)
    let mut ctrl: Vec<(f64, f64, bool)> = Vec::new();
    if arch.is_upper() {
        ctrl.extend((0..=8).map(|k| {
            let r = 2.0 * f64::from(k);
            (r, -16.0 + 9.0 * (r / 16.0).powi(2), false)
        }));
    } else {
        ctrl.extend([(14.5, -12.0, false), (16.5, -8.0, false)]);
    }
    ctrl.extend([
        (18.5, -4.0, false),
        (20.0, -1.5, true),
        (21.0, 0.0, true),
        (23.5, 0.6, true),
        (26.0, 0.0, true),
        (27.0, -1.0, true),
        (27.6, -2.4, true), // height of contour
        (27.2, -3.6, true),
        (26.3, -4.6, true), // gingival margin, under the bulge
        (25.6, -7.0, false),
        (25.4, -9.5, false),
        (26.5, -12.0, false),
        (29.0, -14.0, false), // the border, in the vestibule
    ]);
    let mut profile: Vec<(f64, f64, bool)> = vec![ctrl[0]];
    for w in ctrl.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((b.0 - a.0).hypot(b.1 - a.1) / 0.4).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            let crown = if k == n { b.2 } else { a.2 && b.2 };
            profile.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, crown));
        }
    }
    // around the arch: the angle from `+Y`, the teeth up to 100° each side
    let widths = [8.5, 6.5, 7.5, 7.0, 6.5, 10.0, 9.0];
    let total: f64 = widths.iter().sum();
    let tooth = |deg: f64| -> u16 {
        let mut edge = 0.0;
        for (k, w) in widths.iter().enumerate() {
            edge += 100.0 * w / total;
            if deg.abs() <= edge {
                let quadrant = match (arch.is_upper(), deg >= 0.0) {
                    (true, true) => 1,
                    (true, false) => 2,
                    (false, true) => 4,
                    (false, false) => 3,
                };
                return quadrant * 10 + k as u16 + 1;
            }
        }
        0
    };
    let n_theta = 181usize;
    let angles: Vec<f64> = (0..n_theta)
        .map(|i| -105.0 + 210.0 * i as f64 / (n_theta - 1) as f64)
        .collect();
    let place = |r: f64, z: f64, deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        let z = if arch.is_upper() { 1.4 - z } else { z };
        Vec3::new((r * s) as f32, (r * c) as f32, z as f32)
    };
    // the palate vault has one point in the middle
    let apex = profile[0].0 == 0.0;
    let first = usize::from(apex);
    let mut positions = Vec::new();
    let mut labels = Vec::new();
    if apex {
        positions.push(place(0.0, profile[0].1, 0.0));
        labels.push(0);
    }
    let row = |j: usize| -> u32 { (usize::from(apex) + (j - first) * n_theta) as u32 };
    for &(r, z, crown) in &profile[first..] {
        for &deg in &angles {
            positions.push(place(r, z, deg));
            labels.push(if crown { tooth(deg) } else { 0 });
        }
    }
    let mut tris: Vec<[u32; 3]> = Vec::new();
    if apex {
        for i in 0..n_theta as u32 - 1 {
            tris.push([0, row(1) + i, row(1) + i + 1]);
        }
    }
    for j in first..profile.len() - 1 {
        let (r0, r1) = (row(j), row(j + 1));
        for i in 0..n_theta as u32 - 1 {
            tris.push([r0 + i, r1 + i, r1 + i + 1]);
            tris.push([r0 + i, r1 + i + 1, r0 + i + 1]);
        }
    }
    let occlusal = if arch.is_upper() {
        DVec3::NEG_Z
    } else {
        DVec3::Z
    };
    let mut mesh = TriMesh::new(positions, tris);
    // outward: the crowns face the occlusal side
    let facing: f64 = (0..mesh.tris.len())
        .filter(|&f| mesh.tris[f].iter().all(|&i| labels[i as usize] > 0))
        .map(|f| mesh.face_normal_raw(f).as_dvec3().dot(occlusal))
        .sum();
    if facing < 0.0 {
        for t in &mut mesh.tris {
            t.swap(1, 2);
        }
    }
    let y = DVec3::Y;
    let frame = ArchFrame {
        origin: DVec3::new(0.0, 25.0 * 100f64.to_radians().cos(), 0.0),
        x: y.cross(occlusal),
        y,
        n: occlusal,
    };
    (mesh, labels, frame)
}

/// Distances from `points` to the surface of `mesh` (`5` mm at most).
fn distances(points: &[Vec3], mesh: &TriMesh) -> Vec<f64> {
    let bvh = dc_geom::bvh::Bvh::build(mesh).unwrap();
    bvh.closest_points(points, 5.0)
        .into_iter()
        .map(|h| h.map_or(5.0, |h| f64::from(h.distance)))
        .collect()
}

#[test]
fn the_study_models_keep_the_scanned_anatomy_undercuts_and_palate_included() {
    let params = Params {
        voxel_mm: 0.4,
        tolerance_mm: 0.0,
        ..Params::default()
    };
    let (upper, upper_labels, upper_frame) = undercut_arch(Arch::Upper);
    let (lower, lower_labels, lower_frame) = undercut_arch(Arch::Lower);
    let input = Input {
        upper: Some(ArchInput {
            scan: &upper,
            labels: &upper_labels,
            arch_frame: upper_frame,
        }),
        lower: Some(ArchInput {
            scan: &lower,
            labels: &lower_labels,
            arch_frame: lower_frame,
        }),
    };
    let out = run(&input, &params).unwrap();
    for (key, scan, model) in [
        ("upper", &upper, out.upper_base.as_ref().unwrap()),
        ("lower", &lower, out.lower_base.as_ref().unwrap()),
    ] {
        let r = &out.report.arches[key];
        // one closed piece without handles
        let v = model.validate();
        assert!(v.is_watertight(), "{key}: {v:?}");
        assert_eq!(v.components, 1, "{key}: {v:?}");
        let edges = 3 * model.tris.len() / 2;
        let euler = model.positions.len() as i64 - edges as i64 + model.tris.len() as i64;
        assert_eq!(euler, 2, "{key}: genus {}", (2 - euler) / 2);
        // the whole sheet is protected anatomy: every point of it is on the
        // surface of the model, within the meshing of the voxel (a concave
        // corner where a wall meets the border is rounded by under a voxel)
        let d = distances(&scan.positions, model);
        let mut sorted = d.clone();
        sorted.sort_by(f64::total_cmp);
        let max = sorted[sorted.len() - 1];
        let p95 = sorted[sorted.len() * 95 / 100];
        let worst = d
            .iter()
            .zip(&scan.positions)
            .max_by(|a, b| a.0.total_cmp(b.0))
            .map(|(_, p)| *p);
        assert!(
            max <= params.voxel_mm && p95 <= 0.1,
            "{key}: the model departs from the scan by up to {max:.2} mm (95%: {p95:.3} mm) at {worst:?}"
        );
        // the base keeps its design
        assert!(r.base_tilt_deg < 0.5, "{key}: {r:?}");
        assert!(
            (r.measured_height_mm - r.height_mm).abs() < 0.5,
            "{key}: {r:?}"
        );
    }
}
