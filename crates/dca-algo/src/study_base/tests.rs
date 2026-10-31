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
