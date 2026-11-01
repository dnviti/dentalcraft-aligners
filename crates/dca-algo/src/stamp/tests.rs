use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};

use super::*;
use crate::demo;

fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Closed box `w` x `d` x `h` centred on the origin, outward normals.
fn block(w: f32, d: f32, h: f32) -> TriMesh {
    let (x, y, z) = (w / 2.0, d / 2.0, h / 2.0);
    let positions = vec![
        Vec3::new(-x, -y, -z),
        Vec3::new(x, -y, -z),
        Vec3::new(x, y, -z),
        Vec3::new(-x, y, -z),
        Vec3::new(-x, -y, z),
        Vec3::new(x, -y, z),
        Vec3::new(x, y, z),
        Vec3::new(-x, y, z),
    ];
    let quads = [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let tris = quads
        .iter()
        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
        .collect();
    TriMesh::new(positions, tris)
}

fn manual(point: [f64; 3]) -> Placement {
    Placement::Manual {
        point,
        normal: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
    }
}

fn params(kind: Kind) -> Params {
    Params {
        date: Some("2026-10-13".to_owned()),
        number: 3,
        kind,
        placement: manual([0.0, 0.0, 2.5]),
        ..Params::default()
    }
}

fn height_at(bvh: &Bvh, x: f64, y: f64) -> Option<f64> {
    bvh.ray_cast(
        Vec3::new(x as f32, y as f32, 10.0),
        Vec3::NEG_Z,
        f32::INFINITY,
    )
    .map(|h| 10.0 - f64::from(h.t))
}

fn stamped(outcome: Outcome) -> Stamped {
    match outcome {
        Outcome::Stamped(s) => *s,
        Outcome::Skipped { reason } => panic!("skipped: {reason}"),
    }
}

#[test]
fn the_label_follows_the_plugin() {
    let l = |tpl: &str, arch, n, passive, code: &str| {
        label_lines(tpl, arch, n, passive, "131026", code).unwrap()
    };
    let tpl = Params::default().text;
    assert_eq!(
        l(&tpl, Arch::Upper, 3, false, ""),
        lines(&["SUP 03", "131026"])
    );
    assert_eq!(
        l(&tpl, Arch::Lower, 12, false, ""),
        lines(&["INF 12", "131026"])
    );
    assert_eq!(
        l(&tpl, Arch::Upper, 21, true, ""),
        lines(&["SUP 21 P", "131026"])
    );
    assert_eq!(
        l(&tpl, Arch::Upper, 3, false, "ab-12 x_9345678"),
        lines(&["SUP 03", "131026", "AB-12X93"])
    );
    assert_eq!(clean_code("é-1"), "-1");
    assert_eq!(
        label_lines("case {code}\n\n {arch}", Arch::Upper, 1, false, "d", "k7").unwrap(),
        lines(&["CASE K7", "SUP"])
    );
    // errors
    assert!(label_lines("{nope}", Arch::Upper, 1, false, "d", "").is_err());
    assert!(label_lines("ä", Arch::Upper, 1, false, "d", "").is_err());
    assert!(label_lines("  \n ", Arch::Upper, 1, false, "d", "").is_err());
}

#[test]
fn the_date_has_three_formats() {
    let d = Date::parse("2026-10-13").unwrap();
    assert_eq!(date_text(d, DateFormat::DdMmYy), "131026");
    assert_eq!(date_text(d, DateFormat::DotDdMmYy), "13.10.26");
    assert_eq!(date_text(d, DateFormat::YyMmDd), "261013");
    let d = Date::parse("2005-01-09").unwrap();
    assert_eq!(date_text(d, DateFormat::DdMmYy), "090105");
}

#[test]
fn layouts_drop_the_code_then_join_the_lines() {
    let full = lines(&["SUP 03", "131026", "CODE"]);
    assert_eq!(
        layouts(&full),
        vec![
            full.clone(),
            lines(&["SUP 03", "131026"]),
            lines(&["SUP 03 131026"])
        ]
    );
    let two = lines(&["SUP 03", "131026"]);
    assert_eq!(layouts(&two), vec![two.clone(), lines(&["SUP 03 131026"])]);
    assert_eq!(layouts(&lines(&["X"])), vec![lines(&["X"])]);
    assert_eq!(text_of(&two), "SUP 03 / 131026");
}

#[test]
fn the_glyph_bounds_match_the_requested_height() {
    for h in [0.5, 0.7, 1.0, 2.0] {
        // the digit 0 is the reference of the size
        let zero = layout(&lines(&["0"]), h).unwrap();
        assert!(
            (zero.height - h).abs() < 2e-3 * h.max(1.0),
            "{h}: {}",
            zero.height
        );
        // two lines: one pitch (1.45 heights) between the baselines
        let two = layout(&lines(&["00", "00"]), h).unwrap();
        assert!(
            (two.height - 2.45 * h).abs() < 5e-3 * h.max(1.0),
            "{h}: {}",
            two.height
        );
        // the ink grows with the characters
        let one = layout(&lines(&["0"]), h).unwrap();
        let four = layout(&lines(&["0000"]), h).unwrap();
        assert!(four.width > 3.0 * one.width);
    }
    // the text is centred on the ink
    let t = layout(&lines(&["SUP 03", "131026"]), 0.7).unwrap();
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for tri in &t.tris {
        for &i in tri {
            lo = lo.min(t.points[i as usize]);
            hi = hi.max(t.points[i as usize]);
        }
    }
    assert!((lo + hi).length() < 1e-9);
    assert!(((hi - lo).x - t.width).abs() < 1e-9 && ((hi - lo).y - t.height).abs() < 1e-9);
    // counter-clockwise triangles with a plausible ink area
    let area: f64 = t
        .tris
        .iter()
        .map(|tri| {
            let [a, b, c] = tri.map(|i| t.points[i as usize]);
            let a2 = (b - a).perp_dot(c - a);
            assert!(a2 > 0.0);
            0.5 * a2
        })
        .sum();
    assert!(
        area > 0.1 * t.width * t.height && area < 0.6 * t.width * t.height,
        "{area}"
    );
}

/// The room of a real face, the lingual of 16 of `demo_upper_mild` at stage 1
/// with the embrasure fills: a stretch of 7.2 mm between the fills and the
/// tooth edge above it. The plugin's label fits it (6.16 mm wide, measured in
/// Blender); the advances of the monospaced font left 7.61 mm, too wide for
/// every face of the upper molars and premolars.
#[test]
fn a_one_line_label_fits_the_face_of_a_molar_like_the_plugin() {
    let xs: Vec<f64> = (0..61).map(|k| -6.0 + 0.2 * f64::from(k)).collect();
    let (nx, ny) = (xs.len(), xs.len());
    let valid: Vec<bool> = (0..nx * ny)
        .map(|k| xs[k % nx].abs() <= 3.6 + 1e-9 && xs[k / nx].abs() <= 1.0 + 1e-9)
        .collect();
    let fm = place::FaceMap {
        ys: xs.clone(),
        xs,
        valid,
        center: DVec3::ZERO,
        frame: Frame {
            n: DVec3::X,
            u: DVec3::Y,
            r: DVec3::Z,
        },
        reason: None,
    };
    // the digits are not all as wide: the widest ones and the plugin's example
    for text in ["SUP 01 010126", "INF 13 311226", "SUP 08 280826"] {
        let t = layout(&lines(&[text]), 0.7).unwrap();
        assert!(t.width <= 6.4, "{text}: {} mm", t.width);
        assert!(
            place::place(&fm, t.width, t.height, 0.2).is_some(),
            "{text}: {} mm",
            t.width
        );
    }
    assert!(layout(&lines(&["SUP 01 010126"]), 0.7).unwrap().width <= 6.16);
}

#[test]
fn a_character_the_font_does_not_have_is_an_error() {
    assert!(matches!(
        layout(&lines(&["a"]), 0.7),
        Err(StampError::Invalid(_))
    ));
    assert!(matches!(
        layout(&lines(&["Ü"]), 0.7),
        Err(StampError::Invalid(_))
    ));
    assert!(layout(&lines(&["AZ 09 -./:_#"]), 0.7).is_ok());
    // a space is not ink
    assert!(layout(&lines(&[" "]), 0.7).is_err());
}

#[test]
fn the_tooth_frame_is_orthonormal_and_faces_the_chosen_side() {
    let centre = DVec3::new(-25.0, 5.0, 3.0);
    let arch_c = DVec3::new(0.0, 0.0, 3.0);
    // mesio-distal axis along y for a molar on the left of the arch (x < 0)
    let md = DVec3::new(0.0, 1.0, 0.2);
    let f = tooth_frame(md, centre, arch_c, Face::Lingual, Read::Inside);
    for v in [f.n, f.u, f.r] {
        assert!((v.length() - 1.0).abs() < 1e-12);
    }
    assert!(f.n.dot(f.u).abs() < 1e-12 && f.n.dot(f.r).abs() < 1e-12);
    assert!(f.n.z.abs() < 1e-12, "n is horizontal");
    assert!(f.n.x > 0.9, "lingual points to the centre of the arch");
    assert!(f.u.z < -0.99, "u goes to the gingiva");
    let b = tooth_frame(md, centre, arch_c, Face::Buccal, Read::Inside);
    assert!((b.n + f.n).length() < 1e-12);
    // read from outside mirrors the text: r flips
    let o = tooth_frame(md, centre, arch_c, Face::Lingual, Read::Outside);
    assert!((o.r + f.r).length() < 1e-12 && (o.n - f.n).length() < 1e-12);
    // the tilt turns n and u about r
    let t = place::tilt_frame(&f, 20.0);
    assert!((t.n.dot(f.n) - 20f64.to_radians().cos()).abs() < 1e-12);
    assert!(t.n.z > 0.0, "n rises towards the occlusal side");
    assert!((t.r - f.r).length() < 1e-12);
}

#[test]
fn the_spot_search_finds_the_widest_valid_stretch() {
    let nx = 61;
    let xs: Vec<f64> = (0..nx).map(|k| -6.0 + 0.2 * k as f64).collect();
    let mut valid = vec![false; nx * nx];
    // a valid rectangle x in [-1.6, 3.0], y in [0.4, 3.0]
    for iy in 0..nx {
        for ix in 0..nx {
            let (x, y) = (xs[ix], xs[iy]);
            valid[iy * nx + ix] = (-1.6..=3.0001).contains(&x) && (0.3999..=3.0001).contains(&y);
        }
    }
    let fm = place::FaceMap {
        xs: xs.clone(),
        ys: xs,
        valid,
        center: DVec3::ZERO,
        frame: place::Frame {
            n: DVec3::X,
            u: DVec3::Y,
            r: DVec3::Z,
        },
        reason: None,
    };
    // a 2 x 1.2 box with a 0.2 border fits; x is the closest to 0 (the column is valid), y the middle
    let (x, y) = place::place(&fm, 2.0, 1.2, 0.2).unwrap();
    assert!(x.abs() < 1e-9, "{x}");
    assert!((y - 1.6).abs() < 1e-9, "{y}");
    // a box wider than the rectangle does not fit
    assert!(place::place(&fm, 6.0, 1.0, 0.2).is_none());
    assert!(place::place(&fm, 1.0, 4.0, 0.2).is_none());
}

#[test]
fn the_text_solid_is_a_set_of_closed_prisms_along_the_surface() {
    let shape = layout(&lines(&["10"]), 1.0).unwrap();
    let h: Vec<DVec3> = shape
        .points
        .iter()
        .map(|p| DVec3::new(p.x, p.y, 2.5))
        .collect();
    let n = vec![DVec3::Z; h.len()];
    for engrave in [true, false] {
        let solid = text_solid(&shape, &h, &n, 0.1, engrave);
        assert_eq!(solid.face_count(), 8 * shape.tris.len());
        // volume = (depth + under) * ink area, positive (outward faces)
        let volume: f64 = solid
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| solid.positions[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum();
        let area: f64 = shape
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| shape.points[i as usize]);
                0.5 * (b - a).perp_dot(c - a)
            })
            .sum();
        assert!(
            (volume - 0.4 * area).abs() < 1e-4,
            "{volume} {}",
            0.4 * area
        );
        // engraving goes below the surface, relief above
        let (zmin, zmax) = solid
            .positions
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                (a.min(p.z), b.max(p.z))
            });
        if engrave {
            assert!((f64::from(zmin) - 2.4).abs() < 1e-6 && (f64::from(zmax) - 2.8).abs() < 1e-6);
        } else {
            assert!((f64::from(zmin) - 2.2).abs() < 1e-6 && (f64::from(zmax) - 2.6).abs() < 1e-6);
        }
    }
}

#[test]
fn invalid_parameters_are_refused() {
    let mesh = block(14.0, 14.0, 5.0);
    let input = Input {
        mesh: &mesh,
        arch: Arch::Upper,
        teeth: Vec::new(),
    };
    for p in [
        Params {
            height: 0.1,
            ..params(Kind::Model)
        },
        Params {
            depth: 0.9,
            ..params(Kind::Model)
        },
        Params {
            date: Some("2026-13-40".to_owned()),
            ..params(Kind::Model)
        },
        Params {
            resolution_mm: Some(0.0),
            ..params(Kind::Model)
        },
        Params {
            text: "{x}".to_owned(),
            ..params(Kind::Model)
        },
    ] {
        assert!(
            matches!(run(&input, &p), Err(StampError::Invalid(_))),
            "{p:?}"
        );
    }
    // a placement on a tooth needs teeth: skipped, not an error
    let p = Params {
        placement: Placement::default(),
        ..params(Kind::Model)
    };
    assert!(matches!(run(&input, &p), Ok(Outcome::Skipped { .. })));
}

/// Engraves / embosses the text on a block and checks it: watertight and
/// manifold, depth within 0.05 mm, ink bounds those of the glyphs.
fn check_block(kind: Kind, method: Method) {
    let mesh = block(14.0, 14.0, 5.0);
    let input = Input {
        mesh: &mesh,
        arch: Arch::Upper,
        teeth: Vec::new(),
    };
    let p = Params {
        method,
        ..params(kind)
    };
    let out = stamped(run(&input, &p).unwrap());
    let report = out.mesh.validate();
    assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
    let placed = &out.placed;
    assert_eq!(placed.text, "SUP 03 / 131026");
    assert_eq!(placed.engraved, kind == Kind::Shell);
    assert!(placed.normal[2] > 0.999 && (placed.center[2] - 2.5).abs() < 1e-3);

    let bvh = Bvh::build(&out.mesh).unwrap();
    let sign = if kind == Kind::Shell { -1.0 } else { 1.0 };
    // depth under the centre of every glyph triangle
    let shape = layout(&placed.lines, p.height).unwrap();
    let mut errors: Vec<f64> = Vec::new();
    for (ids, w) in deep_points(&shape, 64) {
        let c: DVec2 = (0..3).map(|k| w[k] * shape.points[ids[k]]).sum();
        // r = x, u = y on the top face of the block (the reader looks along -z)
        let z = height_at(&bvh, c.x, c.y).expect("ray hits the block");
        errors.push(((z - 2.5) * sign - p.depth).abs());
    }
    errors.sort_by(f64::total_cmp);
    let p90 = errors[(errors.len() * 9) / 10];
    let median = errors[errors.len() / 2];
    assert!(median < 0.01, "median depth error {median}");
    assert!(p90 <= 0.05, "p90 depth error {p90}");

    // the ink: points of the top face moved by more than half a depth
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    let step = 0.02;
    let mut y = -1.2;
    while y <= 1.2 {
        let mut x = -2.5;
        while x <= 2.5 {
            let z = height_at(&bvh, x, y).unwrap();
            if (z - 2.5) * sign > 0.5 * p.depth {
                lo = lo.min(DVec2::new(x, y));
                hi = hi.max(DVec2::new(x, y));
            }
            x += step;
        }
        y += step;
    }
    let size = hi - lo;
    let tol = 0.08;
    assert!(
        (size.x - shape.width).abs() < tol,
        "width {} vs {}",
        size.x,
        shape.width
    );
    assert!(
        (size.y - shape.height).abs() < tol,
        "height {} vs {}",
        size.y,
        shape.height
    );
    assert!(((lo + hi) * 0.5).length() < tol);
    // nothing outside the text moved
    for (x, y) in [(-6.0, 0.0), (6.0, 5.0), (0.0, -5.0), (3.5, 0.0)] {
        let z = height_at(&bvh, x, y).unwrap();
        assert!((z - 2.5).abs() < 1e-4, "{x},{y}: {z}");
    }
}

#[test]
fn a_block_is_engraved_to_the_requested_depth() {
    check_block(Kind::Shell, Method::Local);
}

#[test]
fn a_block_is_embossed_to_the_requested_height() {
    check_block(Kind::Model, Method::Local);
}

#[test]
fn the_sdf_method_engraves_and_embosses_a_block_too() {
    check_block(Kind::Shell, Method::Sdf);
    check_block(Kind::Model, Method::Sdf);
}

#[test]
fn the_local_method_keeps_the_topology_and_the_labels() {
    let mut mesh = block(14.0, 14.0, 5.0);
    mesh.vertex_labels = Some((0..mesh.vertex_count() as u16).collect());
    let input = Input {
        mesh: &mesh,
        arch: Arch::Upper,
        teeth: Vec::new(),
    };
    let before = mesh.validate();
    let out = stamped(run(&input, &params(Kind::Shell)).unwrap());
    let after = out.mesh.validate();
    assert!(
        before.is_watertight() && after.is_watertight() && after.is_manifold(),
        "{after:?}"
    );
    assert!(out.triangles_added > 5000 && out.mesh.face_count() == 12 + out.triangles_added);
    // the original corners are still there, with their labels
    let labels = out.mesh.vertex_labels.as_ref().unwrap();
    assert_eq!(labels.len(), out.mesh.vertex_count());
    for (i, p) in mesh.positions.iter().enumerate() {
        assert_eq!(out.mesh.positions[i], *p);
        assert_eq!(labels[i], i as u16);
    }
    // the volume changed by about the ink area times the depth
    let volume = |m: &TriMesh| -> f64 {
        m.tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.positions[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    };
    let shape = layout(&out.placed.lines, 0.7).unwrap();
    let ink: f64 = shape
        .tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| shape.points[i as usize]);
            0.5 * (b - a).perp_dot(c - a)
        })
        .sum();
    let lost = volume(&mesh) - volume(&out.mesh);
    assert!(
        (lost - ink * 0.08).abs() < 0.1 * ink * 0.08,
        "{lost} vs {}",
        ink * 0.08
    );
}

#[test]
fn an_open_surface_is_stamped_too() {
    // a flat 10 x 10 mm sheet: two triangles, boundary kept
    let mesh = TriMesh::new(
        vec![
            Vec3::new(-5.0, -5.0, 0.0),
            Vec3::new(5.0, -5.0, 0.0),
            Vec3::new(5.0, 5.0, 0.0),
            Vec3::new(-5.0, 5.0, 0.0),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    );
    let input = Input {
        mesh: &mesh,
        arch: Arch::Lower,
        teeth: Vec::new(),
    };
    let p = Params {
        placement: manual([0.0, 0.0, 0.0]),
        ..params(Kind::Model)
    };
    let out = stamped(run(&input, &p).unwrap());
    let r = out.mesh.validate();
    assert!(r.is_manifold() && !r.is_watertight());
    assert_eq!(out.mesh.boundary_loops().len(), 1);
    let (zmin, zmax) = out
        .mesh
        .positions
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.z), b.max(p.z)));
    assert!(
        zmin == 0.0 && (f64::from(zmax) - 0.08).abs() < 1e-5,
        "{zmin} {zmax}"
    );
}

#[test]
fn the_text_plane_follows_the_reading_frame() {
    // read from outside the top face with up = +y, the reader's right is +x
    let mesh = block(14.0, 14.0, 5.0);
    let input = Input {
        mesh: &mesh,
        arch: Arch::Upper,
        teeth: Vec::new(),
    };
    let located = locate(&input, &params(Kind::Shell)).unwrap();
    let v = text_plane(&located.placed, DVec3::new(1.0, 0.5, 2.5));
    assert!(
        (v.x - 1.0).abs() < 1e-3 && (v.y - 0.5).abs() < 1e-3,
        "{v:?}"
    );
    assert!(located.solid.face_count() > 100);
}

#[test]
fn a_hand_placed_stamp_on_a_surface_with_no_wall_under_it_is_refused() {
    // the point is above the block: the rays miss the surface
    let mesh = block(4.0, 4.0, 5.0);
    let input = Input {
        mesh: &mesh,
        arch: Arch::Upper,
        teeth: Vec::new(),
    };
    let p = Params {
        placement: manual([4.0, 0.0, 2.5]),
        ..params(Kind::Shell)
    };
    assert!(matches!(run(&input, &p).unwrap(), Outcome::Skipped { .. }));
    assert!(locate(&input, &p).is_err());
}

fn demo_input(demo: &demo::Demo) -> Vec<ToothSite> {
    let labels = demo.mesh.vertex_labels.as_ref().unwrap();
    demo.seeds
        .iter()
        .map(|s| ToothSite {
            fdi: s.fdi,
            crown: demo
                .mesh
                .positions
                .iter()
                .zip(labels)
                .filter(|(_, l)| **l == u16::from(s.fdi.get()))
                .map(|(p, _)| p.as_dvec3())
                .collect(),
            e_md: None,
        })
        .collect()
}

#[test]
fn the_demo_arch_gets_its_stamp_on_a_molar() {
    for (arch, kind) in [(Arch::Lower, Kind::Model), (Arch::Upper, Kind::Model)] {
        let demo = demo::run(&demo::Params {
            arch,
            ..demo::Params::default()
        });
        let mesh = demo.mesh.clone();
        let input = Input {
            mesh: &mesh,
            arch,
            teeth: demo_input(&demo),
        };
        let p = Params {
            date: Some("2026-10-13".to_owned()),
            number: 5,
            code: "DEMO".to_owned(),
            kind,
            ..Params::default()
        };
        let t0 = std::time::Instant::now();
        let out = stamped(run(&input, &p).unwrap());
        // a loose bound: the run takes a few seconds, more under load
        assert!(t0.elapsed().as_secs() < 120, "{arch:?}: {:?}", t0.elapsed());
        let report = out.mesh.validate();
        assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
        // the text folds the surface nowhere: no triangle with a vertex moved
        // off the input surface crosses another one (the demo surface crosses
        // itself where its parts overlap, away from the text)
        let bvh = Bvh::build(&mesh).unwrap();
        let off: Vec<bool> = bvh
            .closest_points(&out.mesh.positions, f32::INFINITY)
            .iter()
            .map(|h| h.is_some_and(|h| h.distance > 1e-3))
            .collect();
        let moved = |f: u32| out.mesh.tris[f as usize].iter().any(|&i| off[i as usize]);
        assert!(off.iter().any(|&o| o), "{arch:?}: nothing moved");
        let folds = crate::shell::crossing_pairs(&out.mesh)
            .into_iter()
            .filter(|&(a, b)| moved(a) || moved(b))
            .count();
        assert_eq!(folds, 0, "{arch:?} {kind:?}: the text folds the surface");
        assert!(out.placed.tooth.is_some());
        assert!(out.placed.height_mm > 2.0 * p.height);
        // the stamp is on a first molar when it fits there
        let fdi = out.placed.tooth.unwrap();
        assert!(fdi % 10 >= 5 && fdi % 10 <= 7, "{fdi}");
    }
}
