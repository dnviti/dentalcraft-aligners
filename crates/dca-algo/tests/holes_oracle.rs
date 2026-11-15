//! Numerical comparison of the holes through the wall of a trimmed shell
//! (`dca_algo::trim::holes`) and of its rigid removal (`dca_algo::trim::removal`) with
//! the Python plugin (`ortho_aligner/case_print.py`: `_handle_loops`, `_short_loops`,
//! `_cap_loop`, `_plug_holes`, `_window_cuts`, `_removal_lock` glued as `rigid_lock`;
//! `aligner.trim_cutter` with slots).
//!
//! `tests/fixtures/holes.json` is produced by `gen_holes.py` from the plugin's own
//! functions (python3 + numpy, no Blender): slabs of voxels with tunnels through their
//! 0.5 mm wall on a box (the model), their vertices exact in single precision. The loops
//! (vertex indices) and the faces of the capped and plugged surfaces (an FNV-1a hash of
//! the vertex indices) are equal; lengths, new vertices and slots within 1e-6 mm (the
//! slots are rounded to 1e-4 mm by both). The removal: a tooth with an undercut and its
//! shell, the locked areas, deep areas and depths equal (to the 0.01 the plugin rounds
//! them to), the path chosen within 1e-12.

use std::path::PathBuf;

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use dca_algo::trim::grid::PlanGrid;
use dca_algo::trim::holes::{self, CapFill, Design, Model, Slot};
use dca_algo::trim::{cutter, removal};
use glam::{DVec2, DVec3, Vec3};
use serde_json::Value;

const TOL_MM: f64 = 1e-6;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/holes.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn ints(v: &Value) -> Vec<u32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u32)
        .collect()
}

fn vec3(v: &Value) -> DVec3 {
    let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    DVec3::new(n[0], n[1], n[2])
}

fn mesh(v: &Value) -> TriMesh {
    let pos: Vec<Vec3> = v["V"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let q = vec3(p);
            let f = q.as_vec3();
            assert_eq!(
                f.as_dvec3(),
                q,
                "the fixture's vertices are single precision"
            );
            f
        })
        .collect();
    let tris: Vec<[u32; 3]> = v["T"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let t = ints(t);
            [t[0], t[1], t[2]]
        })
        .collect();
    TriMesh::new(pos, tris)
}

fn fnv(tris: &[[u32; 3]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in tris.iter().flatten() {
        h = (h ^ u64::from(v)).wrapping_mul(0x0100_0000_01b3);
    }
    h.to_string()
}

/// A surface made from `input` against the plugin's record of it.
fn same_surface(input: &TriMesh, pos: &[DVec3], tris: &[[u32; 3]], want: &Value, what: &str) {
    let dropped = ints(&want["dropped"]);
    let new: Vec<DVec3> = want["new"].as_array().unwrap().iter().map(vec3).collect();
    let kept: Vec<DVec3> = (0..input.vertex_count() as u32)
        .filter(|v| dropped.binary_search(v).is_err())
        .map(|v| input.positions[v as usize].as_dvec3())
        .collect();
    assert_eq!(pos.len(), kept.len() + new.len(), "{what}: vertices");
    for (k, (p, q)) in pos.iter().zip(kept.iter().chain(&new)).enumerate() {
        assert!(
            (*p - *q).abs().max_element() < TOL_MM,
            "{what} vertex {k}: {p} vs {q}"
        );
    }
    assert_eq!(
        tris.len() as u64,
        want["faces"].as_u64().unwrap(),
        "{what}: faces"
    );
    assert_eq!(
        fnv(tris),
        want["hash"].as_str().unwrap(),
        "{what}: faces differ"
    );
}

fn slots(v: &Value) -> Vec<Slot> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let p = |x: &Value| {
                let n: Vec<f64> = x.as_array().unwrap().iter().map(num).collect();
                DVec2::new(n[0], n[1])
            };
            Slot {
                p0: p(&s[0]),
                p1: p(&s[1]),
                z: num(&s[2]),
                half: num(&s[3]),
            }
        })
        .collect()
}

fn design(f: &Value) -> Design {
    let outer = num(&f["outer"]);
    Design {
        outer,
        clearance: num(&f["clearance"]),
        rim: dca_algo::trim::rim_round(outer - num(&f["clearance"]), None),
        voxel: num(&f["voxel"]),
    }
}

#[test]
fn handle_and_short_loops_match_the_plugin() {
    let f = fixture();
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let m = mesh(c);
        let pos: Vec<DVec3> = m.positions.iter().map(|p| p.as_dvec3()).collect();
        assert_eq!(
            holes::genus(pos.len(), &m.tris),
            c["genus"].as_i64().unwrap(),
            "{name} genus"
        );
        let loops = holes::handle_loops(&pos, &m.tris);
        let want: Vec<Vec<u32>> = c["handle_loops"]
            .as_array()
            .unwrap()
            .iter()
            .map(ints)
            .collect();
        assert_eq!(loops, want, "{name} handle loops");
        for (k, (l, w)) in loops
            .iter()
            .zip(c["short_loops"].as_array().unwrap())
            .enumerate()
        {
            let got =
                holes::short_loops(&pos, &m.tris, l, holes::CROP_MM, holes::ROOTS, holes::KEEP);
            let w = w.as_array().unwrap();
            assert_eq!(got.len(), w.len(), "{name} short loops {k}");
            for ((len, s), x) in got.iter().zip(w) {
                assert!(
                    (len - num(&x[0])).abs() < TOL_MM,
                    "{name} short loop {k}: {len} vs {}",
                    x[0]
                );
                assert_eq!(*s, ints(&x[1]), "{name} short loop {k}");
            }
        }
    }
}

#[test]
fn caps_and_plugs_match_the_plugin() {
    let f = fixture();
    let model_mesh = mesh(&f["model"]);
    let model = Model::new(&[&model_mesh]).unwrap();
    let d = design(&f);
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let m = mesh(c);
        let pos: Vec<DVec3> = m.positions.iter().map(|p| p.as_dvec3()).collect();
        let lp = ints(&c["capped_loop"]);
        for (key, fill) in [("fan", CapFill::Fan), ("area", CapFill::Area)] {
            let want = &c["caps"][key];
            let got = holes::cap_loop(&pos, &m.tris, &lp, fill, None);
            let (Some(got), false) = (got, want.is_null()) else {
                panic!("{name} {key}: one of the caps is missing");
            };
            let what = format!("{name} {key} cap");
            same_surface(&m, &got.pos, &got.tris, want, &what);
            let cap: Vec<u32> = (0..got.cap.len() as u32)
                .filter(|&i| got.cap[i as usize])
                .collect();
            assert_eq!(cap, ints(&want["cap"]), "{what}: cap faces");
            assert_eq!(got.genus, want["genus"].as_i64().unwrap(), "{what}: genus");
        }
        let want = &c["plugged"];
        let (plugged, g, plugs) = holes::plug_holes(m.clone(), &model, d, None);
        let ppos: Vec<DVec3> = plugged.positions.iter().map(|p| p.as_dvec3()).collect();
        // the new vertices are the plugin's within single precision (the mesh is f32)
        same_surface(&m, &ppos, &plugged.tris, want, &format!("{name} plugged"));
        assert_eq!(g, want["genus"].as_i64().unwrap(), "{name} plugged genus");
        assert_eq!(
            plugs as u64,
            want["plugs"].as_u64().unwrap(),
            "{name} plugs"
        );
        let r = plugged.validate();
        assert!(r.is_watertight() && r.is_manifold(), "{name}: {r:?}");
    }
}

#[test]
fn slots_match_the_plugin() {
    let f = fixture();
    let model_mesh = mesh(&f["model"]);
    let model = Model::new(&[&model_mesh]).unwrap();
    let d = design(&f);
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let m = mesh(c);
        for (half, want) in c["slots"].as_object().unwrap() {
            let got = holes::window_cuts(&m, &model, d, None, half.parse().unwrap());
            let want = slots(want);
            assert_eq!(got.len(), want.len(), "{name} slots {half}");
            for (a, b) in got.iter().zip(&want) {
                assert!(
                    (a.p0 - b.p0).abs().max_element() < TOL_MM
                        && (a.p1 - b.p1).abs().max_element() < TOL_MM
                        && (a.z - b.z).abs() < TOL_MM
                        && a.half == b.half,
                    "{name} slot {half}: {a:?} vs {b:?}"
                );
            }
        }
    }
    // the slots raise the trim surface of trim_cutter
    let cu = &f["cutter"];
    let ctrl: Vec<DVec3> = cu["ctrl"].as_array().unwrap().iter().map(vec3).collect();
    let (lo, hi) = (vec3(&cu["lo"]), vec3(&cu["hi"]));
    let grid = PlanGrid::around(lo, hi, num(&cu["cell"]));
    let z_top = hi.z + 5.0;
    let mut z = cutter::interpolated(&grid, &ctrl, 4, 40.0, 3.0, z_top);
    holes::apply_slots(&grid, &mut z, &slots(&cu["cuts"]), z_top);
    let want: Vec<f64> = cu["z"].as_array().unwrap().iter().map(num).collect();
    assert_eq!(z.len(), want.len());
    for (k, (a, b)) in z.iter().zip(&want).enumerate() {
        assert!((a - b).abs() < TOL_MM, "cutter [{k}]: {a} vs {b}");
    }
}

fn mesh4096(v: &Value) -> TriMesh {
    let pos: Vec<Vec3> = v["V4096"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let n: Vec<f32> = p
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as f32 / 4096.0)
                .collect();
            Vec3::new(n[0], n[1], n[2])
        })
        .collect();
    let tris: Vec<[u32; 3]> = v["T"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let t = ints(t);
            [t[0], t[1], t[2]]
        })
        .collect();
    TriMesh::new(pos, tris)
}

fn lock_eq(a: &removal::Lock, w: &Value, what: &str) {
    let w: Vec<f64> = w.as_array().unwrap().iter().map(num).collect();
    assert!(
        (a.area_mm2 - w[0]).abs() < 1e-9
            && (a.deep_mm2 - w[1]).abs() < 1e-9
            && (a.deepest_mm - w[2]).abs() < 1e-9,
        "{what}: {a:?} vs {w:?}"
    );
}

#[test]
fn rigid_removal_matches_the_plugin() {
    let f = fixture();
    let r = &f["removal"];
    let (tooth, shell) = (mesh4096(&r["tooth"]), mesh4096(&r["shell"]));
    let (tb, sb) = (Bvh::build(&tooth).unwrap(), Bvh::build(&shell).unwrap());
    let pair = |v: &Value| {
        let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
        DVec2::new(n[0], n[1])
    };
    // removal_lock along VERTICAL and a leaning path
    let paths: Vec<DVec2> = r["paths"].as_array().unwrap().iter().map(pair).collect();
    let locks = removal::removal_lock(&shell, &sb, Some(&tb), &paths, removal::CHECK_GRID_MM);
    for (k, (a, w)) in locks.iter().zip(r["locks"].as_array().unwrap()).enumerate() {
        lock_eq(a, w, &format!("path {k}"));
    }
    assert!(
        locks[0].area_mm2 > 1.0,
        "the undercut locks the vertical path"
    );
    // rigid_lock: the mean path of the teeth and the paths leaning towards it
    let shears: Vec<DVec2> = r["rigid"]["shears"]
        .as_array()
        .unwrap()
        .iter()
        .map(pair)
        .collect();
    let got = removal::rigid_lock(&shell, &sb, Some(&tb), &shears);
    lock_eq(&got.lock, &r["rigid"]["lock"], "rigid");
    let want = pair(&r["rigid"]["shear"]);
    assert!(
        (DVec2::from(got.shear) - want).abs().max_element() < 1e-12,
        "{:?} vs {want}",
        got.shear
    );
}
