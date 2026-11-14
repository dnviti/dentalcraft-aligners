//! The scene entries of the built teeth and gingiva.

use dc_case::{Case, CaseState, Fdi, ModuleState, Pose, ScanRole};
use dc_mesh::TriMesh;
use dc_module_api::{Module, ModuleRegistry};
use dc_scene::{MeshColoring, SceneItem};
use dca_model::{AlignersState, Stage, ToothFrame};
use dca_module::{AlignersModule, GINGIVA_COLOR, MODULE_ID};
use glam::{DVec3, Vec3};

fn tri() -> TriMesh {
    TriMesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![[0, 1, 2]])
}

fn built() -> CaseState {
    let mut s = CaseState::new(Case::new("T"));
    let mut data = AlignersState::default();
    let f11 = Fdi::new(11).unwrap();
    let f31 = Fdi::new(31).unwrap();
    for f in [f11, f31] {
        data.crowns.insert(f, s.blobs.insert(tri()));
    }
    data.gingiva.insert(ScanRole::Upper, s.blobs.insert(tri()));
    data.tooth_frames.insert(
        f11,
        ToothFrame {
            pose: Pose::from_translation(DVec3::new(10.0, 0.0, 0.0)),
            center_of_resistance: DVec3::ZERO,
        },
    );
    let mut st = Stage::new(2);
    st.poses
        .insert(f11, Pose::from_translation(DVec3::new(0.0, 1.5, 0.0)));
    data.stages.push(st);
    s.set_module(MODULE_ID, ModuleState::new(data));
    s
}

fn xform(e: &dc_scene::SceneEntry) -> Pose {
    match &e.item {
        SceneItem::Mesh { xform, .. } => *xform,
        i => panic!("{i:?}"),
    }
}

#[test]
fn no_data_no_scene() {
    let m = AlignersModule::new();
    let s = CaseState::new(Case::new("T"));
    assert!(m.scene_at(&s, None).is_empty());
}

#[test]
fn built_pose_has_teeth_and_gingiva() {
    let s = built();
    let entries = AlignersModule::new().scene_at(&s, None);
    assert_eq!(entries.len(), 3);
    let t11 = entries.iter().find(|e| e.tooth == Some(11)).unwrap();
    assert_eq!(t11.arch, Some(ScanRole::Upper));
    assert!(t11.hides_scan);
    assert_eq!(xform(t11), Pose::IDENTITY);
    match &t11.item {
        SceneItem::Mesh { coloring, .. } => assert_eq!(
            *coloring,
            MeshColoring::Uniform(dc_scene::LabelPalette::default().color(11))
        ),
        i => panic!("{i:?}"),
    }
    let t31 = entries.iter().find(|e| e.tooth == Some(31)).unwrap();
    assert_eq!(t31.arch, Some(ScanRole::Lower));
    assert!(t31.hides_scan);
    let gum = entries.iter().find(|e| e.tooth.is_none()).unwrap();
    assert_eq!(gum.arch, Some(ScanRole::Upper));
    assert!(!gum.hides_scan);
    match &gum.item {
        SceneItem::Mesh { coloring, .. } => {
            assert_eq!(*coloring, MeshColoring::Uniform(GINGIVA_COLOR));
        }
        i => panic!("{i:?}"),
    }
    // stage 0 and a stage without data are the built pose
    assert_eq!(AlignersModule::new().scene_at(&s, Some(0)), entries);
    assert_eq!(AlignersModule::new().scene_at(&s, Some(9)), entries);
}

#[test]
fn a_stage_moves_the_teeth_with_a_pose() {
    let s = built();
    let entries = AlignersModule::new().scene_at(&s, Some(2));
    let t11 = entries.iter().find(|e| e.tooth == Some(11)).unwrap();
    let p = xform(t11).apply_point(DVec3::new(1.0, 2.0, 3.0));
    assert!(p.abs_diff_eq(DVec3::new(1.0, 3.5, 3.0), 1e-9), "{p:?}");
    // a tooth with no pose stays
    let t31 = entries.iter().find(|e| e.tooth == Some(31)).unwrap();
    assert_eq!(xform(t31), Pose::IDENTITY);
}

#[test]
fn a_stage_turns_a_tooth_about_its_centre_of_resistance_as_the_plan_does() {
    let mut s = built();
    let f11 = Fdi::new(11).unwrap();
    let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
    let frame = ToothFrame {
        pose: Pose::new(
            DVec3::new(10.0, 0.0, 0.0),
            glam::DQuat::from_rotation_z(0.4),
        ),
        center_of_resistance: DVec3::new(0.0, 0.0, -8.0),
    };
    data.tooth_frames.insert(f11, frame);
    let turn = Pose::from_rotation(glam::DQuat::from_rotation_x(0.2));
    data.stages[0].poses.insert(f11, turn);
    s.set_module(MODULE_ID, ModuleState::new(data));
    let entries = AlignersModule::new().scene_at(&s, Some(2));
    let t11 = entries.iter().find(|e| e.tooth == Some(11)).unwrap();
    // the move the setup, the staging and the export read from the pose
    let rest = dca_algo::teeth::rest_pose(&frame);
    let planned = rest.compose(&turn).compose(&rest.inverse());
    assert!(xform(t11).approx_eq(&planned, 1e-9, 1e-9), "{t11:?}");
    // the centre of resistance stays where it is, the crown turns about it
    let cor = frame.pose.apply_point(frame.center_of_resistance);
    assert!(xform(t11).apply_point(cor).abs_diff_eq(cor, 1e-9));
    let tip = frame.pose.apply_point(DVec3::ZERO);
    assert!(xform(t11).apply_point(tip).distance(tip) > 1.0);
}

#[test]
fn the_registry_collects_it_and_the_scan_is_hidden() {
    let mut reg = ModuleRegistry::new();
    reg.register(Box::new(AlignersModule::new())).unwrap();
    let s = built();
    let entries = reg.scene_at(&s, None);
    assert_eq!(entries.len(), 3);
    let hidden: Vec<_> = entries
        .iter()
        .filter(|e| e.hides_scan)
        .filter_map(|e| e.arch)
        .collect();
    assert_eq!(hidden, [ScanRole::Upper, ScanRole::Lower]);
}

/// A flat strip of gingiva along +X (1 mm grid, 3 rows) whose first column
/// is the margin of tooth 11; the far end is beyond the reach of the tooth.
fn strip() -> TriMesh {
    let (nx, ny) = (25u32, 3u32);
    let mut vertices = Vec::new();
    for y in 0..ny {
        for x in 0..nx {
            vertices.push(DVec3::new(f64::from(x), f64::from(y), 0.0));
        }
    }
    let at = |x: u32, y: u32| y * nx + x;
    let mut tris = Vec::new();
    for y in 0..ny - 1 {
        for x in 0..nx - 1 {
            tris.push([at(x, y), at(x + 1, y), at(x + 1, y + 1)]);
            tris.push([at(x, y), at(x + 1, y + 1), at(x, y + 1)]);
        }
    }
    let n = vertices.len();
    let margin: Vec<bool> = (0..n).map(|i| (i as u32).is_multiple_of(nx)).collect();
    dca_algo::gingiva::GumSurface {
        tooth: margin.iter().map(|&m| if m { 11 } else { 0 }).collect(),
        margin,
        membrane: vec![false; n],
        col: vec![false; n],
        vertices,
        tris,
    }
    .to_mesh()
}

fn deformed(e: &dc_scene::SceneEntry) -> Option<&TriMesh> {
    e.mesh.as_deref()
}

#[test]
fn the_gingiva_follows_the_moved_teeth_at_a_stage() {
    let mut s = built();
    let rest = strip();
    let gum = s.blobs.insert(rest.clone());
    let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
    data.gingiva.insert(ScanRole::Upper, gum);
    s.set_module(MODULE_ID, ModuleState::new(data));
    let m = AlignersModule::new();
    let gum_entry = |entries: &[dc_scene::SceneEntry]| {
        entries
            .iter()
            .find(|e| e.tooth.is_none() && e.arch == Some(ScanRole::Upper))
            .cloned()
            .unwrap()
    };
    // at rest (and at a stage that moves nothing) the blob is drawn as it is
    assert!(deformed(&gum_entry(&m.scene_at(&s, None))).is_none());
    assert!(deformed(&gum_entry(&m.scene_at(&s, Some(9)))).is_none());
    // stage 2 moves tooth 11 by 1.5 mm along +Y: its margin follows, the far end stays
    let e = gum_entry(&m.scene_at(&s, Some(2)));
    let moved = deformed(&e).expect("deformed gingiva");
    assert_eq!(moved.tris, rest.tris);
    let d = |i: usize| moved.positions[i] - rest.positions[i];
    assert!(
        (d(0) - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-4,
        "{:?}",
        d(0)
    );
    assert!(d(24).length() < 1e-6, "{:?}", d(24));
    // in between it blends: it moves less than the margin
    assert!(d(5).y > 0.0 && d(5).y < 1.5, "{:?}", d(5));
    // the cached weights give the same result again
    let again = gum_entry(&m.scene_at(&s, Some(2)));
    assert_eq!(deformed(&again).unwrap().positions, moved.positions);
}

#[test]
fn the_nowait_scene_draws_the_gum_at_rest_until_its_weights_are_ready() {
    let mut s = built();
    let gum = s.blobs.insert(strip());
    let mut data = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
    data.gingiva.insert(ScanRole::Upper, gum);
    s.set_module(MODULE_ID, ModuleState::new(data));
    let m = AlignersModule::new();
    let upper_gum = |entries: &[dc_scene::SceneEntry]| {
        entries
            .iter()
            .find(|e| e.tooth.is_none() && e.arch == Some(ScanRole::Upper))
            .cloned()
            .unwrap()
    };
    // a stage that moves nothing needs no weights
    let (_, complete) = m.scene_at_nowait(&s, Some(9));
    assert!(complete);
    // the first ask starts the computation and does not wait for it
    let (entries, complete) = m.scene_at_nowait(&s, Some(2));
    assert!(!complete);
    assert!(deformed(&upper_gum(&entries)).is_none());
    // the teeth are posed meanwhile
    assert_ne!(
        xform(entries.iter().find(|e| e.tooth == Some(11)).unwrap()),
        Pose::IDENTITY
    );
    let t0 = std::time::Instant::now();
    let done = loop {
        let (entries, complete) = m.scene_at_nowait(&s, Some(2));
        if complete {
            break entries;
        }
        assert!(t0.elapsed().as_secs() < 30, "weights never ready");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let e = upper_gum(&done);
    assert_eq!(
        deformed(&e).unwrap().positions,
        deformed(&upper_gum(&m.scene_at(&s, Some(2))))
            .unwrap()
            .positions
    );
}
