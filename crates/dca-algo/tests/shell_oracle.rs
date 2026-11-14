//! Print geometry of the demo cases (golden fixtures): the model, the
//! blockout model and the untrimmed shell of a stage must be closed,
//! manifold surfaces, and the shell wall must have the thickness asked.
//!
//! Inputs from the oracle: the closed teeth of `staging/closed_<fdi>.dcm`
//! (tooth-local coordinates) placed by their rest frame (`teeth.json`) and
//! the staging movement `q` of the stage (`staging.json`), and the gingiva of
//! `scan.dcm` (the faces whose three vertices are labelled 0). Case
//! coordinates are the world coordinates of the scan; the arch frame is
//! `worldframe.json`.
//!
//! A coarse run (0.25 mm voxels) runs by default; the full resolution of the
//! plugin (0.1 mm, tolerance 0.01 mm) on two cases, with timings, runs with
//! `cargo test --release -p dca-algo --test shell_oracle -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dc_case::Fdi;
use dc_mesh::TriMesh;
use dca_algo::frame::rigid_inverse;
use dca_algo::rotation::rotvec_to_matrix;
use dca_algo::shell::{self, Design, Input, Outputs, Params};
use glam::{DMat4, DVec3};
use serde_json::Value;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../golden/fixtures");

fn case(name: &str) -> PathBuf {
    Path::new(GOLDEN).join(name)
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn mat4(v: &Value) -> DMat4 {
    let r: Vec<Vec<f64>> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect();
    DMat4::from_cols_array_2d(&[
        [r[0][0], r[1][0], r[2][0], r[3][0]],
        [r[0][1], r[1][1], r[2][1], r[3][1]],
        [r[0][2], r[1][2], r[2][2], r[3][2]],
        [r[0][3], r[1][3], r[2][3], r[3][3]],
    ])
}

/// The rigid transform of a movement `q = [tx, ty, tz, rx, ry, rz]` in the
/// rest frame (`staging.local_pose`).
fn local_pose(q: &[f64]) -> DMat4 {
    let r = rotvec_to_matrix(DVec3::new(q[3], q[4], q[5]));
    DMat4::from_cols(
        r.x_axis.extend(0.0),
        r.y_axis.extend(0.0),
        r.z_axis.extend(0.0),
        DVec3::new(q[0], q[1], q[2]).extend(1.0),
    )
}

fn moved(mesh: &TriMesh, m: &DMat4) -> TriMesh {
    TriMesh::new(
        mesh.positions
            .iter()
            .map(|p| m.transform_point3(p.as_dvec3()).as_vec3())
            .collect(),
        mesh.tris.clone(),
    )
}

struct Stage {
    to_arch: DMat4,
    teeth: Vec<TriMesh>,
    gingiva: TriMesh,
    n_stages: usize,
}

/// The closed teeth at stage `k` (`None`: the last one) and the gingiva, in
/// world coordinates.
fn stage(name: &str, k: Option<usize>) -> Stage {
    let dir = case(name);
    let arch_frame = mat4(&json(&dir.join("worldframe/worldframe.json"))["arch_frame"]["M"]);
    let teeth_json = json(&dir.join("teeth/teeth.json"));
    let staging = json(&dir.join("staging/staging.json"));
    let q = staging["plan"]["q"].as_object().unwrap();
    let n_stages = staging["plan"]["n_stages"].as_u64().unwrap() as usize;
    let k = k.unwrap_or(n_stages);
    let mut teeth = Vec::new();
    for (fdi, qs) in q {
        let f: u8 = fdi.parse().unwrap();
        assert!(Fdi::new(f).is_ok());
        let rest = mat4(&teeth_json["teeth"][fdi]["rest_frame"]);
        let qs = qs.as_array().unwrap();
        let qk: Vec<f64> = qs[k.min(qs.len() - 1)]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        let local = dc_mesh::dcm::decode(
            &std::fs::read(dir.join(format!("staging/closed_{fdi}.dcm"))).unwrap(),
        )
        .unwrap();
        teeth.push(moved(&local, &(arch_frame * rest * local_pose(&qk))));
    }
    let scan = dc_mesh::dcm::decode(&std::fs::read(dir.join("scan.dcm")).unwrap()).unwrap();
    let labels = scan.vertex_labels.clone().unwrap();
    let gum: Vec<[u32; 3]> = scan
        .tris
        .iter()
        .filter(|t| t.iter().all(|&i| labels[i as usize] == 0))
        .copied()
        .collect();
    let mut remap = BTreeMap::new();
    let mut positions = Vec::new();
    let tris = gum
        .iter()
        .map(|t| {
            t.map(|i| {
                *remap.entry(i).or_insert_with(|| {
                    positions.push(scan.positions[i as usize]);
                    (positions.len() - 1) as u32
                })
            })
        })
        .collect();
    Stage {
        to_arch: rigid_inverse(&arch_frame),
        teeth,
        gingiva: TriMesh::new(positions, tris),
        n_stages,
    }
}

/// Lines along the insertion axis (a grid across the arch) that cross a
/// gap of air between two parts of `mesh`: along a wall parallel to the axis
/// the reduced surface wavers by the mesh tolerance, so a gap counts when
/// its middle is farther than `depth` from the surface. Returns (lines with
/// a gap, lines that meet the mesh).
fn undercut_lines(mesh: &TriMesh, to_arch: DMat4, params: &Params, depth: f32) -> (usize, usize) {
    let from_arch = rigid_inverse(&to_arch);
    let bvh = dc_geom::bvh::Bvh::build(mesh).unwrap();
    let up = from_arch
        .transform_vector3(DVec3::from(params.insertion_axis).normalize())
        .as_vec3();
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in &mesh.positions {
        let q = to_arch.transform_point3(p.as_dvec3());
        lo = lo.min(q);
        hi = hi.max(q);
    }
    let (mut lines, mut undercut) = (0, 0);
    let step = 0.37;
    let mut x = lo.x + 0.013;
    while x < hi.x {
        let mut y = lo.y + 0.029;
        while y < hi.y {
            let o = from_arch
                .transform_point3(DVec3::new(x, y, lo.z - 5.0))
                .as_vec3();
            let hits = bvh.ray_hits(o, up, (hi.z - lo.z + 10.0) as f32, 64, 1e-5);
            if !hits.is_empty() {
                lines += 1;
                // entry or exit by the side of the face hit; a gap of air is an exit
                // followed by an entry, deeper than `depth` at its middle
                let mut exit: Option<f32> = None;
                let mut gap = false;
                for h in &hits {
                    let n = mesh.face_normal(h.face as usize);
                    if n.dot(up) > 0.0 {
                        exit = Some(h.t);
                    } else if let Some(e) = exit.take() {
                        let mid = o + up * (0.5 * (e + h.t));
                        gap |= bvh
                            .closest_point(mid, f32::INFINITY)
                            .is_some_and(|c| c.distance > depth);
                    }
                }
                if gap {
                    undercut += 1;
                }
            }
            y += step;
        }
        x += step;
    }
    (undercut, lines)
}

/// Builds the solids of a stage and checks them; returns the seconds taken.
fn check(name: &str, k: Option<usize>, params: &Params, thickness_tol: f64) -> f64 {
    let st = stage(name, k);
    let design = Design {
        thickness_mm: 1.0,
        clearance_mm: 0.0,
    };
    let input = Input {
        to_arch: st.to_arch,
        teeth: &st.teeth,
        gingiva: &st.gingiva,
        cover: &[],
        fdis: &[],
    };
    let t = Instant::now();
    let out = shell::run(&input, params, &design, Outputs::ALL).unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "{name} stage {:?}/{}: {secs:.2} s, {:?}",
        k,
        st.n_stages,
        serde_json::to_string(&out.report).unwrap()
    );
    let model = out.model.unwrap();
    let blockout = out.blockout.unwrap();
    let shell_mesh = out.shell.unwrap();
    for (what, m) in [
        ("model", &model),
        ("blockout", &blockout),
        ("shell", &shell_mesh),
    ] {
        let rep = m.validate();
        assert!(rep.is_watertight(), "{name} {what}: {rep:?}");

        println!(
            "  {what}: {} faces, {} components",
            rep.faces, rep.components
        );
    }
    assert_eq!(
        model.validate().components,
        1,
        "{name}: the model is one piece"
    );
    assert_eq!(
        blockout.validate().components,
        1,
        "{name}: the blockout is one piece"
    );
    // no two triangles of the reduced solids cross (the print check refuses it)
    for (what, m) in [
        ("model", &model),
        ("blockout", &blockout),
        ("shell", &shell_mesh),
    ] {
        let pairs = shell::crossing_pairs(m);
        assert!(
            pairs.is_empty(),
            "{name} {what}: {} crossing pairs",
            pairs.len()
        );
    }
    // wall thickness: every vertex of the shell lies at the clearance (fitting side)
    // or at clearance + thickness (outer side) from the model surface. Both meshes are
    // reduced within the tolerance, so the 99.9th percentile must be within
    // `thickness_tol` and the largest within it plus the two tolerances
    let d = shell::wall_distances(&shell_mesh, &model).unwrap();
    let split = design.clearance_mm + 0.5 * design.thickness_mm;
    let mut inner: Vec<f64> = d
        .iter()
        .filter(|x| **x < split)
        .map(|x| (x - design.clearance_mm).abs())
        .collect();
    // signed: negative where the wall is thinner than asked
    let outer: Vec<f64> = d
        .iter()
        .filter(|x| **x >= split)
        .map(|x| x - design.clearance_mm - design.thickness_mm)
        .collect();
    assert!(!inner.is_empty() && !outer.is_empty());
    let stats = |v: &mut Vec<f64>, q: f64| {
        v.sort_unstable_by(f64::total_cmp);
        (v[((v.len() - 1) as f64 * q) as usize], v[v.len() - 1])
    };
    let (in999, in_max) = stats(&mut inner, 0.999);
    let thinnest = outer.iter().copied().fold(f64::INFINITY, f64::min);
    let mut outer_abs: Vec<f64> = outer.iter().map(|x| x.abs()).collect();
    let (out99, out_max) = stats(&mut outer_abs, 0.99);
    println!(
        "  wall error: fitting side p99.9 {in999:.4} max {in_max:.4} mm, outer side p99 {out99:.4} max {out_max:.4} thinnest {thinnest:.4} mm ({} outer vertices)",
        outer.len()
    );
    let slack = 2.0 * params.tolerance_mm;
    assert!(
        in999 <= thickness_tol,
        "{name}: fitting side p99.9 off by {in999} mm"
    );
    assert!(
        in_max <= thickness_tol + slack,
        "{name}: fitting side off by {in_max} mm"
    );
    // the outer side is the level of the closing field (`_sdf_group`), the exact
    // offset of the solid the closing samples: never closer than the thickness, a
    // little farther from the meshed model where the closing rounded a crease (the
    // OpenVDB shell of the plugin, same soup, matches it within 0.05 mm at p99.9)
    assert!(
        thinnest >= -(thickness_tol + slack),
        "{name}: outer side thinner by {thinnest} mm"
    );
    assert!(
        out99 <= thickness_tol + slack,
        "{name}: outer side p99 off by {out99} mm"
    );
    assert!(
        out_max <= 4.0 * thickness_tol + slack,
        "{name}: outer side off by {out_max} mm"
    );
    // no undercut: a line along the insertion axis meets the blockout in one interval
    // (the model, for comparison, has undercuts)
    let depth = (2.0 * params.tolerance_mm + 0.5 * params.model_voxel_mm) as f32;
    let (undercut, lines) = undercut_lines(&blockout, st.to_arch, params, depth);
    let (model_undercut, _) = undercut_lines(&model, st.to_arch, params, depth);
    println!(
        "  undercuts deeper than {depth} mm: blockout {undercut} of {lines} lines along the axis, model {model_undercut}"
    );
    assert!(lines > 1000);
    assert!(
        model_undercut > 0,
        "{name}: the model should have undercuts"
    );
    // a few lines at the heel of the cast, a voxel deep where its vertical wall meets
    // the flat base, are left (none on the crowns): at most one line in a thousand
    assert!(
        undercut * 1000 <= lines,
        "{name}: the blockout has undercuts on {undercut} of {lines} lines"
    );
    secs
}

#[test]
fn coarse_stage_is_closed_and_has_its_thickness() {
    let params = Params {
        model_voxel_mm: 0.25,
        shell_voxel_mm: 0.25,
        tolerance_mm: 0.02,
        ..Params::default()
    };
    check("demo_lower_mild", None, &params, 0.15);
}

#[test]
#[ignore = "plugin resolution (0.1 mm): a few seconds per stage in release, minutes in debug: run with --release --ignored"]
fn full_resolution_on_two_cases() {
    let params = Params::default();
    let mut total = 0.0;
    for (name, k) in [
        ("demo_lower_mild", None),
        ("demo_upper_moderate", None),
        ("demo_upper_moderate", Some(0)),
    ] {
        total += check(name, k, &params, 0.05);
    }
    println!("total {total:.1} s");
}

/// Writes a soup as the raw arrays of `golden/vdb_model.py`.
fn write_raw(mesh: &TriMesh, stem: &Path) {
    let v: Vec<u8> = mesh
        .positions
        .iter()
        .flat_map(|p| p.to_array())
        .flat_map(f32::to_le_bytes)
        .collect();
    let t: Vec<u8> = mesh
        .tris
        .iter()
        .flat_map(|t| *t)
        .flat_map(u32::to_le_bytes)
        .collect();
    std::fs::write(stem.with_extension("v.bin"), v).unwrap();
    std::fs::write(stem.with_extension("t.bin"), t).unwrap();
}

fn read_raw(stem: &Path) -> TriMesh {
    let v = std::fs::read(stem.with_extension("v.bin")).unwrap();
    let t = std::fs::read(stem.with_extension("t.bin")).unwrap();
    let f = |b: &[u8]| f32::from_le_bytes(b.try_into().unwrap());
    let u = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
    TriMesh::new(
        v.as_chunks::<12>()
            .0
            .iter()
            .map(|c| glam::Vec3::new(f(&c[0..4]), f(&c[4..8]), f(&c[8..12])))
            .collect(),
        t.as_chunks::<12>()
            .0
            .iter()
            .map(|c| [u(&c[0..4]), u(&c[4..8]), u(&c[8..12])])
            .collect(),
    )
}

/// The closed model of the soup of `model_parts` against OpenVDB through the
/// plugin (`golden/vdb_model.py`, Blender headless) on the same soup: the
/// union of overlapping closed parts closed by the blockout, with the pockets
/// the parts enclose behind a narrow mouth filled (on lower_mild the field
/// used to keep a surface about 7 mm deep in the lingual area).
#[test]
#[ignore = "needs Blender (blender -b) and the plugin repository: run with --release --ignored --nocapture"]
fn closed_model_matches_openvdb() {
    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    if std::process::Command::new(&blender)
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: {blender} not found");
        return;
    }
    let (voxel, blockout) = (shell::MODEL_VOXEL_MM, shell::BLOCKOUT_MM);
    let st = stage("demo_lower_mild", None);
    let input = Input {
        to_arch: st.to_arch,
        teeth: &st.teeth,
        gingiva: &st.gingiva,
        cover: &[],
        fdis: &[],
    };
    let parts = shell::model_parts(&input, shell::BASE_HEIGHT_MM).unwrap();
    let dir = std::env::temp_dir().join(format!("dc_vdb_model_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (soup, out) = (dir.join("soup"), dir.join("vdb"));
    write_raw(&parts, &soup);
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/vdb_model.py");
    let status = std::process::Command::new(&blender)
        .args(["-b", "--factory-startup", "-P"])
        .arg(&script)
        .arg("--")
        .arg(&soup)
        .arg(voxel.to_string())
        .arg(blockout.to_string())
        .arg(&out)
        .status()
        .unwrap();
    let vdb = status.success().then(|| read_raw(&out));
    std::fs::remove_dir_all(&dir).unwrap();
    let vdb = vdb.expect("blender run failed");
    let t = Instant::now();
    let model = shell::closed_grid(&parts, voxel, blockout)
        .unwrap()
        .to_mesh()
        .unwrap();
    println!(
        "closed model: {:.2} s, {} faces (OpenVDB {})",
        t.elapsed().as_secs_f64(),
        model.tris.len(),
        vdb.tris.len()
    );
    assert_eq!(model.validate().components, 1, "a surface left inside");
    let (max, p999) = shell::deviation_stats(&model, &vdb).unwrap();
    println!("deviation from OpenVDB: p99.9 {p999:.4} mm, max {max:.4} mm");
    assert!(p999 < 0.1, "p99.9 {p999} mm");
    assert!(max < 0.5, "max {max} mm");
}
