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
    assert!(
        worst <= 0.5,
        "a tooth vertex {worst} mm outside the base: {:?}",
        a.report.arches["lower"]
    );
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
    arch_sheet(arch, true)
}

/// The sheet of [`undercut_arch`], its crowns bulging over the gum when
/// `undercut`, else the gum going straight out from under them.
fn arch_sheet(arch: Arch, undercut: bool) -> (TriMesh, Vec<u16>, ArchFrame) {
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
    ]);
    if undercut {
        ctrl.extend([
            (27.2, -3.6, true),
            (26.3, -4.6, true), // gingival margin, under the bulge
            (25.6, -7.0, false),
            (25.4, -9.5, false),
            (26.5, -12.0, false),
        ]);
    } else {
        ctrl.extend([
            (27.8, -4.6, true),
            (28.0, -7.0, false),
            (28.2, -9.5, false),
            (28.6, -12.0, false),
        ]);
    }
    ctrl.push((29.0, -14.0, false)); // the border, in the vestibule
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
    assert!(out.report.paired, "{:?}", out.report.warnings);
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

/// `scan` and its arch frame as an upper scan is read: turned 180° about
/// `y` to face the scanner as a lower scan does, and moved by `shift`.
fn as_read(scan: &TriMesh, frame: ArchFrame, shift: DVec3) -> (TriMesh, ArchFrame) {
    let m = DMat4::from_translation(shift) * DMat4::from_rotation_y(std::f64::consts::PI);
    let mut moved = scan.clone();
    for p in &mut moved.positions {
        *p = m.transform_point3(p.as_dvec3()).as_vec3();
    }
    let frame = ArchFrame {
        origin: m.transform_point3(frame.origin),
        x: m.transform_vector3(frame.x),
        y: m.transform_vector3(frame.y),
        n: m.transform_vector3(frame.n),
    };
    (moved, frame)
}

#[test]
fn two_arches_not_in_occlusion_are_each_based_as_when_alone() {
    // the scans as read, before the bite: the upper facing the scanner as
    // the lower does, beside it (a pair's frame would take the lower's
    // occlusal side for the upper's, turn the upper upside down and put its
    // base on its crowns, with a false mirrored-labels warning)
    let params = Params {
        voxel_mm: 0.4,
        tolerance_mm: 0.0,
        ..Params::default()
    };
    let (upper, upper_labels, upper_frame) = arch_sheet(Arch::Upper, false);
    let (lower, lower_labels, lower_frame) = arch_sheet(Arch::Lower, false);
    let (upper, upper_frame) = as_read(&upper, upper_frame, DVec3::new(0.0, 100.0, 0.0));
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
    let both = run(&input, &params).unwrap();
    assert!(!both.report.paired);
    let codes: Vec<&str> = both
        .report
        .warnings
        .iter()
        .map(|w| w.code.as_str())
        .collect();
    assert!(
        !codes.contains(&"mirrored_labels") && !codes.contains(&"single_arch"),
        "{:?}",
        both.report.warnings
    );
    for (arch, scan, labels, frame) in [
        (Arch::Upper, &upper, &upper_labels, upper_frame),
        (Arch::Lower, &lower, &lower_labels, lower_frame),
    ] {
        let key = arch_key(arch);
        assert_one_clean_solid(&both, key);
        let one = alone(arch, scan, labels, frame, &params);
        let (a, b) = if arch.is_upper() {
            (&both.upper_base, &one.upper_base)
        } else {
            (&both.lower_base, &one.lower_base)
        };
        let (a, b) = (a.as_ref().unwrap(), b.as_ref().unwrap());
        assert_eq!(a.positions, b.positions, "{key}");
        assert_eq!(a.tris, b.tris, "{key}");
        let (ra, rb) = (&both.report.arches[key], &one.report.arches[key]);
        assert_eq!(ra.depth_mm, rb.depth_mm, "{key}");
        assert_eq!(ra.frame, rb.frame, "{key}");
    }
}

/// `scan` closed as an older program closes a scan: the triangle `hole`
/// taken out of it, its border and the loop of that hole walled along `down`
/// onto a plane 3 mm beyond its farthest point, each ring on the plane
/// capped by a fan from its middle point. The border gives the model a base
/// of its own, tilted when `down` is; the hole a chimney from the crown down
/// to that base, inside the model. The new points are labelled `0`; also
/// the three corners of the chimney on the crown and on the base.
fn closed_by_an_old_base(
    scan: &TriMesh,
    labels: &[u16],
    down: DVec3,
    hole: usize,
) -> (TriMesh, Vec<u16>, [(DVec3, DVec3); 3]) {
    let mut tris = scan.tris.clone();
    let chimney = tris.remove(hole);
    let mut positions = scan.positions.clone();
    let reach = positions
        .iter()
        .map(|p| down.dot(p.as_dvec3()))
        .fold(f64::NEG_INFINITY, f64::max)
        + 3.0;
    let mut corners = [(DVec3::ZERO, DVec3::ZERO); 3];
    for ring in holes::boundary_loops(&tris, positions.len()) {
        let copy: Vec<u32> = ring
            .iter()
            .map(|&w| {
                let p = positions[w as usize].as_dvec3();
                let q = p + down * (reach - down.dot(p));
                if let Some(k) = chimney.iter().position(|&c| c == w) {
                    corners[k] = (p, q);
                }
                positions.push(q.as_vec3());
                (positions.len() - 1) as u32
            })
            .collect();
        let mid = copy
            .iter()
            .map(|&w| positions[w as usize].as_dvec3())
            .sum::<DVec3>()
            / copy.len() as f64;
        let center = positions.len() as u32;
        positions.push(mid.as_vec3());
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            let (a2, b2) = (copy[k], copy[(k + 1) % ring.len()]);
            tris.extend([[b, a, a2], [b, a2, b2], [b2, a2, center]]);
        }
    }
    let mut labels = labels.to_vec();
    labels.resize(positions.len(), 0);
    (TriMesh::new(positions, tris), labels, corners)
}

/// One arch alone through [`run`].
fn alone(arch: Arch, scan: &TriMesh, labels: &[u16], frame: ArchFrame, params: &Params) -> Output {
    let a = Some(ArchInput {
        scan,
        labels,
        arch_frame: frame,
    });
    let input = if arch.is_upper() {
        Input {
            upper: a,
            lower: None,
        }
    } else {
        Input {
            upper: None,
            lower: a,
        }
    };
    run(&input, params).unwrap()
}

/// One closed piece without tunnels, no `not_solid` warning (else where
/// the tunnels are: the length and the middle of the shortest loop round
/// each).
fn assert_one_clean_solid(out: &Output, key: &str) {
    let r = &out.report.arches[key];
    assert!(r.watertight, "{key}: {r:?}");
    let model = if key == "upper" {
        out.upper_base.as_ref()
    } else {
        out.lower_base.as_ref()
    }
    .unwrap();
    let pos: Vec<DVec3> = to_vec(model);
    let loops: Vec<(f64, DVec3)> = holes::handle_loops(&pos, &model.tris)
        .iter()
        .filter_map(|l| {
            holes::short_loops(&pos, &model.tris, l, holes::CROP_MM, holes::ROOTS, 1).pop()
        })
        .map(|(len, lp)| {
            let at = lp.iter().map(|&w| pos[w as usize]).sum::<DVec3>() / lp.len() as f64;
            (len, at)
        })
        .collect();
    assert_eq!(
        (r.components, r.genus, r.open_edges, r.nonmanifold_edges),
        (1, Some(0), 0, 0),
        "{key}: tunnels {loops:?}"
    );
    assert!(
        !out.report.warnings.iter().any(|w| w.code == "not_solid"),
        "{:?}",
        out.report.warnings
    );
}

#[test]
fn a_model_closed_on_a_tilted_base_of_its_own_with_a_chimney_is_one_solid() {
    let params = Params {
        voxel_mm: 0.4,
        tolerance_mm: 0.0,
        ..Params::default()
    };
    // the crowns without an undercut: the old base walls the ends of the
    // arch down too, and walls dropped from a crown that bulges over its gum
    // would pass through one another
    let (sheet, labels, frame) = arch_sheet(Arch::Upper, false);
    // a triangle of an incisor crown, its chimney walled down with the base
    let hole = (0..sheet.tris.len())
        .filter(|&f| sheet.tris[f].iter().all(|&w| labels[w as usize] == 11))
        .max_by(|&a, &b| {
            let depth = |f: usize| -sheet.triangle(f)[0].z;
            depth(a).total_cmp(&depth(b))
        })
        .unwrap();
    // the base of the upper arch is towards +Z, tilted 8° across the arch
    let tilt = 8f64.to_radians();
    let down = DVec3::new(tilt.sin(), 0.0, tilt.cos());
    let (model, labels, chimney) = closed_by_an_old_base(&sheet, &labels, down, hole);
    assert_eq!(dc_mesh::solid_topology(&model).boundary_edges, 0);
    let out = alone(Arch::Upper, &model, &labels, frame, &params);
    assert_one_clean_solid(&out, "upper");
    let built = out.upper_base.as_ref().unwrap();
    // the anatomy is on the surface of the model, within the meshing
    let d = distances(&sheet.positions, built);
    let mut sorted = d.clone();
    sorted.sort_by(f64::total_cmp);
    let (max, p95) = (sorted[sorted.len() - 1], sorted[sorted.len() * 95 / 100]);
    assert!(
        max <= params.voxel_mm && p95 <= 0.1,
        "the model departs from the anatomy by up to {max:.2} mm (95%: {p95:.3} mm): {:?}",
        out.report
    );
    // the chimney is inside the model: none of its walls is surface
    let inside: Vec<Vec3> = chimney
        .iter()
        .flat_map(|(top, bottom)| [0.3, 0.5, 0.7].map(|t| top.lerp(*bottom, t).as_vec3()))
        .collect();
    let deep = distances(&inside, built);
    assert!(
        deep.iter().all(|&x| x > 0.5),
        "the walls of the chimney are on the surface: {deep:?}"
    );
}

#[test]
fn an_open_scan_with_a_folded_face_and_a_pinched_border_gets_no_fin() {
    let params = Params {
        voxel_mm: 0.4,
        tolerance_mm: 0.0,
        ..Params::default()
    };
    let (mut sheet, mut labels, frame) = undercut_arch(Arch::Lower);
    // a face folded back 1 mm over a crown tip, on an edge of two crown faces
    let crown = (0..sheet.tris.len())
        .filter(|&f| sheet.tris[f].iter().all(|&w| labels[w as usize] == 41))
        .max_by(|&a, &b| sheet.triangle(a)[0].z.total_cmp(&sheet.triangle(b)[0].z))
        .unwrap();
    let [a, b, _] = sheet.tris[crown];
    let apex = (sheet.positions[a as usize] + sheet.positions[b as usize]) * 0.5 + Vec3::Z;
    sheet.positions.push(apex);
    labels.push(41);
    let fold = sheet.positions.len() as u32 - 1;
    sheet.tris.push([a, b, fold]);
    // a hole of three by three cells in the gum, touching at a point the
    // notch of a face taken out of the border: the border passes it twice
    let n_theta = 181u32;
    let rows = (sheet.positions.len() as u32 - 1) / n_theta;
    let at = |j: u32, i: u32| j * n_theta + i;
    let (last, i) = (rows - 1, 60);
    let gone: Vec<[u32; 3]> = std::iter::once([at(last - 1, i), at(last, i), at(last, i + 1)])
        .chain((last - 4..last - 1).flat_map(|j| {
            (i..i + 3).flat_map(move |c| {
                [
                    [at(j, c), at(j + 1, c), at(j + 1, c + 1)],
                    [at(j, c), at(j + 1, c + 1), at(j, c + 1)],
                ]
            })
        }))
        .collect();
    let sorted = |t: &[u32; 3]| {
        let mut s = *t;
        s.sort_unstable();
        s
    };
    let gone: Vec<[u32; 3]> = gone.iter().map(sorted).collect();
    let before = sheet.tris.len();
    sheet.tris.retain(|t| !gone.contains(&sorted(t)));
    assert_eq!(sheet.tris.len(), before - gone.len());
    let out = alone(Arch::Lower, &sheet, &labels, frame, &params);
    assert_one_clean_solid(&out, "lower");
    // the fold is gone: nothing of the model reaches up to its tip
    let built = out.lower_base.as_ref().unwrap();
    let tip = distances(&[apex], built)[0];
    assert!(
        tip > 0.5,
        "a fin up to {tip:.2} mm from the tip of the fold"
    );
    // the rest of the anatomy is on the surface of the model
    let used: Vec<Vec3> = sheet.positions[..sheet.positions.len() - 1]
        .iter()
        .enumerate()
        .filter(|(w, _)| sheet.tris.iter().any(|t| t.contains(&(*w as u32))))
        .map(|(_, p)| *p)
        .collect();
    let d = distances(&used, built);
    let max = d.iter().copied().fold(0.0, f64::max);
    assert!(
        max <= params.voxel_mm,
        "the model departs from the scan by {max:.2} mm"
    );
}

#[test]
fn a_model_that_is_not_one_clean_solid_is_reported() {
    // a closed box: nothing to say
    let cube = |o: Vec3| {
        let p: Vec<Vec3> = (0..8)
            .map(|k| o + Vec3::new((k & 1) as f32, ((k >> 1) & 1) as f32, ((k >> 2) & 1) as f32))
            .collect();
        let t = vec![
            [0, 2, 1],
            [1, 2, 3],
            [4, 5, 6],
            [5, 7, 6],
            [0, 1, 4],
            [1, 5, 4],
            [2, 6, 3],
            [3, 6, 7],
            [0, 4, 2],
            [2, 4, 6],
            [1, 3, 5],
            [3, 7, 5],
        ];
        TriMesh::new(p, t)
    };
    let mut warnings = Vec::new();
    let solid = solid_check(&cube(Vec3::ZERO), 0, "upper", &mut warnings);
    assert_eq!((solid.pieces, solid.genus), (1, Some(0)));
    assert!(warnings.is_empty(), "{warnings:?}");
    // two pieces
    let mut two = cube(Vec3::ZERO);
    let other = cube(Vec3::splat(3.0));
    two.tris.extend(other.tris.iter().map(|t| t.map(|w| w + 8)));
    two.positions.extend(other.positions);
    solid_check(&two, 0, "lower", &mut warnings);
    // open: a face taken out
    let mut open = cube(Vec3::ZERO);
    open.tris.pop();
    solid_check(&open, 0, "upper", &mut warnings);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().all(|w| w.code == "not_solid"));
    assert_eq!(warnings[0].arch, "lower");
    assert!(
        warnings[0].detail.contains("2 pieces"),
        "{}",
        warnings[0].detail
    );
    assert!(
        warnings[1].detail.contains("3 open edges"),
        "{}",
        warnings[1].detail
    );
    // a tunnel through it: one the scan does not have is reported, one of
    // the scan kept is not
    let (nu, nv) = (24u32, 12u32);
    let torus = TriMesh::new(
        (0..nu)
            .flat_map(|i| {
                (0..nv).map(move |j| {
                    let (u, v) = (
                        std::f32::consts::TAU * i as f32 / nu as f32,
                        std::f32::consts::TAU * j as f32 / nv as f32,
                    );
                    let r = 10.0 + 3.0 * v.cos();
                    Vec3::new(r * u.cos(), r * u.sin(), 3.0 * v.sin())
                })
            })
            .collect(),
        (0..nu)
            .flat_map(|i| {
                (0..nv).flat_map(move |j| {
                    let w = |a: u32, b: u32| (a % nu) * nv + b % nv;
                    [
                        [w(i, j), w(i + 1, j), w(i + 1, j + 1)],
                        [w(i, j), w(i + 1, j + 1), w(i, j + 1)],
                    ]
                })
            })
            .collect(),
    );
    let mut more = Vec::new();
    let solid = solid_check(&torus, 0, "upper", &mut more);
    assert_eq!((solid.pieces, solid.genus), (1, Some(1)));
    assert_eq!(more.len(), 1);
    assert!(more[0].detail.contains("1 tunnels"), "{}", more[0].detail);
    more.clear();
    solid_check(&torus, 1, "upper", &mut more);
    assert!(more.is_empty(), "{more:?}");
}
