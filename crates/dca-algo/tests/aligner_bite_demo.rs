//! The bite with the aligners worn on the plugin's demo arches in occlusion
//! (`fixtures/occlusion.json`, upper arch 0.6 mm into the lower one): the
//! automatic occlusal thickness, the ramps on the surfaces, the shaping of a
//! shell and the closed envelope solid.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use dca_algo::aligner_bite::occlusal::{
    ArchStage, Bite, Design, bite, contact_points, crown_cells, envelope_solid, model_points,
    shape_shell,
};
use dca_model::{BiteClosing, OcclusalMode, OcclusalSettings, Settings, TrimParams};
use glam::{DMat4, DVec3, Vec3};
use serde_json::Value;

fn base64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let bytes = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= u32::from(val(c)) << (18 - 6 * i);
        }
        let n = chunk.len() - 1;
        out.extend_from_slice(&acc.to_be_bytes()[1..=n]);
    }
    out
}

struct Demo {
    rel: DMat4,
    lower: ArchStage,
    upper: ArchStage,
    lower_teeth: Vec<TriMesh>,
}

fn arch_stage(rec: &Value) -> (ArchStage, Vec<TriMesh>) {
    let n = rec["n_vertices"].as_u64().unwrap() as usize;
    let m = rec["n_tris"].as_u64().unwrap() as usize;
    let v: Vec<Vec3> = base64(rec["vertices"].as_str().unwrap())
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            let f = |k: usize| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap());
            Vec3::new(f(0), f(1), f(2))
        })
        .collect();
    let t: Vec<[u32; 3]> = base64(rec["tris"].as_str().unwrap())
        .as_chunks::<6>()
        .0
        .iter()
        .map(|c| {
            let f = |k: usize| u32::from(u16::from_le_bytes([c[2 * k], c[2 * k + 1]]));
            [f(0), f(1), f(2)]
        })
        .collect();
    let l = base64(rec["labels"].as_str().unwrap());
    assert_eq!((v.len(), t.len(), l.len()), (n, m, n));
    let mut ids: Vec<u8> = l.iter().copied().filter(|&x| x != 0).collect();
    ids.sort_unstable();
    ids.dedup();
    let (mut crown, mut crown_labels) = (Vec::new(), Vec::new());
    let mut teeth = Vec::new();
    for f in ids {
        let own: Vec<DVec3> = v
            .iter()
            .zip(&l)
            .filter(|(_, x)| **x == f)
            .map(|(p, _)| p.as_dvec3())
            .collect();
        let cells = crown_cells(&own, 0.2);
        crown_labels.extend(std::iter::repeat_n(i64::from(f), cells.len()));
        crown.extend(cells);
        // the closed shell of the tooth
        let tris: Vec<[u32; 3]> = t
            .iter()
            .copied()
            .filter(|tr| tr.iter().all(|&i| l[i as usize] == f))
            .collect();
        let mut mesh = TriMesh::new(v.clone(), tris);
        if mesh.validate().is_watertight() {
            let vol: f64 = mesh
                .tris
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3());
                    a.dot(b.cross(c)) / 6.0
                })
                .sum();
            if vol < 0.0 {
                for tr in &mut mesh.tris {
                    tr.swap(1, 2);
                }
            }
        }
        teeth.push(compact(&mesh));
    }
    let (model, model_labels) = model_points(&teeth, &crown, &crown_labels);
    let covered: Vec<i64> = {
        let mut c = crown_labels.clone();
        c.sort_unstable();
        c.dedup();
        c
    };
    (
        ArchStage {
            crown_points: crown,
            crown_labels,
            model_points: model,
            model_labels,
            covered,
            ramp_points: vec![],
            ramp_labels: vec![],
        },
        teeth,
    )
}

/// The mesh with only the vertices its triangles use.
fn compact(m: &TriMesh) -> TriMesh {
    let mut map = vec![u32::MAX; m.positions.len()];
    let mut pos = Vec::new();
    let tris = m
        .tris
        .iter()
        .map(|t| {
            t.map(|i| {
                if map[i as usize] == u32::MAX {
                    map[i as usize] = pos.len() as u32;
                    pos.push(m.positions[i as usize]);
                }
                map[i as usize]
            })
        })
        .collect();
    TriMesh::new(pos, tris)
}

fn demo() -> Demo {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/occlusion.json");
    let occ: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let a: Vec<f64> = occ["relation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    let rel = DMat4::from_cols_array(&a.try_into().unwrap()).transpose();
    let (lower, lower_teeth) = arch_stage(&occ["lower"]);
    let (upper, _) = arch_stage(&occ["upper"]);
    Demo {
        rel,
        lower,
        upper,
        lower_teeth,
    }
}

fn design(occlusal: OcclusalSettings) -> Design {
    Design::new(&Settings::default(), &occlusal, &TrimParams::default())
}

fn run(d: &Demo, o: OcclusalSettings) -> Bite {
    bite(&d.lower, &d.upper, &d.rel, &design(o), [0, 0]).expect("the arches have crowns")
}

#[test]
fn the_automatic_thickness_stays_between_the_minimum_and_the_wall() {
    let d = demo();
    let auto = run(&d, OcclusalSettings::default());
    let o = OcclusalSettings::default();
    assert!(
        auto.occlusal >= o.min_thickness_mm - 1e-9 && auto.occlusal <= 1.0 + 1e-9,
        "{}",
        auto.occlusal
    );
    assert!(auto.closing.is_some() && auto.teeth.is_some());
    assert!(!auto.report.text.is_empty());
    assert_eq!(auto.report.thickness_mode, "AUTO");
    // the opening caused by the aligners at the molars is within the target, or the thickness is the minimum
    let mol = auto.openings.1.expect("the molar opening is measured");
    assert!(
        mol <= o.opening_target_mm + 0.15 || (auto.occlusal - o.min_thickness_mm).abs() < 1e-6,
        "{mol} at {}",
        auto.occlusal
    );
    // the mode of the wall keeps the whole wall and opens the bite more
    let same = run(
        &d,
        OcclusalSettings {
            mode: OcclusalMode::Same,
            ..o
        },
    );
    assert!((same.occlusal - 1.0).abs() < 1e-12);
    assert!(same.openings.1.unwrap() >= mol - 1e-9);
    // a manual thickness is used as it is
    let manual = run(
        &d,
        OcclusalSettings {
            mode: OcclusalMode::Manual,
            thickness_mm: 0.6,
            ..o
        },
    );
    assert!((manual.occlusal - 0.6).abs() < 1e-12);
    // the vertical closing is also a closing
    let vertical = run(
        &d,
        OcclusalSettings {
            closing: BiteClosing::Vertical,
            ..o
        },
    );
    assert!(vertical.closing.is_some() && vertical.axis.is_none());
    assert_eq!(vertical.report.closing, "VERTICAL");
    // the contacts are points of the closing
    let (p, g) = contact_points(&same);
    assert_eq!(p.len(), g.len());
    assert!(g.iter().all(|&x| x <= 0.1));
}

#[test]
fn the_equilibration_grinds_until_both_sides_touch_or_reports_why_not() {
    let d = demo();
    let o = OcclusalSettings {
        mode: OcclusalMode::Manual,
        thickness_mm: 0.8,
        equilibrate: true,
        ..Default::default()
    };
    let b = run(&d, o);
    let eq = b.equilibration.as_ref().expect("equilibrated");
    let rep = b.report.equilibration.as_ref().unwrap();
    assert_eq!(rep.done, eq.done);
    assert!(
        rep.max_grind_mm
            .iter()
            .all(|&g| g <= o.equilibrate_max_mm + 1e-9)
    );
    assert!(b.report.text.contains("equilibratura"));
}

#[test]
fn a_ramp_is_part_of_the_aligner_the_antagonist_touches() {
    let d = demo();
    let o = OcclusalSettings {
        mode: OcclusalMode::Same,
        ..Default::default()
    };
    let plain = run(&d, o);
    // a ramp point 2 mm above the lowest-lying upper incisor point opens the bite at the front
    let mut with = Demo {
        rel: d.rel,
        lower: d.lower.clone(),
        upper: d.upper.clone(),
        lower_teeth: vec![],
    };
    let top = with
        .lower
        .crown_points
        .iter()
        .cloned()
        .fold(DVec3::splat(f64::MIN), |m, p| if p.z > m.z { p } else { m });
    with.lower.ramp_points = vec![top + DVec3::Z * 2.0];
    with.lower.ramp_labels = vec![31];
    let ramped = run(&with, o);
    let z = |b: &Bite| {
        let s = &b.surfaces.0;
        s.z.data.iter().cloned().fold(f64::MIN, f64::max)
    };
    assert!(
        z(&ramped) >= top.z + 2.0 - 0.25,
        "the ramp is on the surface (within a fraction of a cell)"
    );
    assert!(z(&ramped) > z(&plain));
    assert!(ramped.closing.unwrap().value >= plain.closing.unwrap().value - 1e-9);
}

#[test]
fn the_envelope_of_an_aligner_is_a_valid_closed_solid() {
    let d = demo();
    let b = run(&d, OcclusalSettings::default());
    for surf in [&b.surfaces.0, &b.surfaces.1] {
        let solid = envelope_solid(surf, 5.0).expect("a solid");
        let r = solid.validate();
        assert!(r.is_watertight() && r.is_manifold(), "{r:?}");
        let vol: f64 = solid
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| solid.positions[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum();
        assert!(vol > 1000.0, "outward faces and a real volume: {vol}");
    }
}

#[test]
fn the_shell_is_lowered_onto_the_occlusal_limit_and_never_touches_the_inner_surface() {
    let d = demo();
    let mut o = OcclusalSettings {
        mode: OcclusalMode::Manual,
        thickness_mm: 0.6,
        ..Default::default()
    };
    o.closing = BiteClosing::Hinge;
    let des = design(o);
    let b = bite(&d.lower, &d.upper, &d.rel, &des, [0, 0]).unwrap();
    // the shell of the lower arch: the offset of its teeth by the wall, minus the teeth
    let teeth_all = {
        let mut pos = Vec::new();
        let mut tris = Vec::new();
        for t in &d.lower_teeth {
            let base = pos.len() as u32;
            pos.extend_from_slice(&t.positions);
            tris.extend(t.tris.iter().map(|tr| tr.map(|i| i + base)));
        }
        TriMesh::new(pos, tris)
    };
    let params = SdfParams {
        voxel_size: 0.2,
        band: 2.0,
    };
    let model = SdfGrid::from_mesh(&teeth_all, &params).unwrap();
    let mut outer = model.clone();
    outer.offset(1.0).unwrap();
    outer.subtract(&model).unwrap();
    let shell = outer.to_mesh().unwrap();
    let covered = d.lower.covered.clone();
    let (shaped, rep) = shape_shell(&shell, &teeth_all, &b, true, &covered, &des);
    assert_eq!(shaped.tris, shell.tris, "the faces stay");
    assert!(rep.vertices_lowered > 0, "{rep:?}");
    let mut lowered = 0;
    for (a, s) in shell.positions.iter().zip(&shaped.positions) {
        assert_eq!((a.x, a.y), (s.x, s.y));
        lowered += usize::from(s.z < a.z - 1e-6);
    }
    assert!(lowered > 0);
    let top = |m: &TriMesh| m.positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!(top(&shaped) < top(&shell));
    let [p5, med] = rep.measured_thickness_mm.expect("measured");
    assert!(p5 > 0.0 && med >= p5, "{p5} {med}");
    // wall thickness as the manual occlusal thickness is not exceeded by much on the lowered vertices
    assert!(med < 1.0, "{med}");
    // the mode of the wall leaves the shell as it is
    let same = design(OcclusalSettings {
        mode: OcclusalMode::Same,
        ..Default::default()
    });
    let bs = bite(&d.lower, &d.upper, &d.rel, &same, [0, 0]).unwrap();
    let (unchanged, rep) = shape_shell(&shell, &teeth_all, &bs, true, &covered, &same);
    assert_eq!(unchanged, shell);
    assert_eq!(rep.vertices_lowered, 0);
    let _ = BTreeMap::<i64, i64>::new();
}
