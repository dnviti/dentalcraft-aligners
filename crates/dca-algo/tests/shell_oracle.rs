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
use dca_algo::anatomy::{Arch, arch_order};
use dca_algo::frame::rigid_inverse;
use dca_algo::rotation::rotvec_to_matrix;
use dca_algo::shell::{self, Design, Input, Outputs, Params};
use glam::{DMat4, DVec2, DVec3};
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
    /// The tooth number of every tooth of `teeth`.
    fdis: Vec<Fdi>,
    /// Every tooth of `teeth` in its own frame, its placement (tooth →
    /// world) and the count of its first vertices that are scanned
    /// (`closed_tooth_info`).
    local: Vec<(TriMesh, DMat4, usize)>,
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
    let mut fdis = Vec::new();
    let mut locals = Vec::new();
    for (fdi, qs) in q {
        let f: u8 = fdi.parse().unwrap();
        fdis.push(Fdi::new(f).unwrap());
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
        let m = arch_frame * rest * local_pose(&qk);
        teeth.push(moved(&local, &m));
        let scanned = staging["closed_tooth_info"][fdi]["crown_vertices"]
            .as_u64()
            .unwrap() as usize;
        locals.push((local, m, scanned));
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
        fdis,
        local: locals,
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
        // the plugin's shell lies on the gingiva
        gum_clearance_mm: 0.0,
    };
    let input = Input {
        to_arch: st.to_arch,
        teeth: &st.teeth,
        gingiva: &st.gingiva,
        cover: &[],
        fdis: &[],
        covered: None,
        stumps: &[],
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

/// The recess of the interproximal block-out under the smooth-border shell
/// (the default of `TrimParams::recess_mm`).
const RECESS_MM: f64 = 0.6;

/// Points where the edges of a mesh cross the height `z` (plan).
fn section(p: &[DVec3], tris: &[[u32; 3]], z: f64) -> Vec<DVec2> {
    let mut out = Vec::new();
    for t in tris {
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (p[t[i] as usize], p[t[j] as usize]);
            if (a.z < z) != (b.z < z) {
                out.push((a + (b - a) * ((z - a.z) / (b.z - a.z))).truncate());
            }
        }
    }
    out
}

/// Convex hull of plan points (monotone chain).
fn hull(pts: &[DVec2]) -> Vec<DVec2> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut h: Vec<DVec2> = Vec::new();
    for pass in [p.clone(), p.iter().rev().copied().collect()] {
        let start = h.len();
        for q in pass {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// The stretch of the line `c + s w` inside the convex polygon `poly`.
fn line_range(poly: &[DVec2], c: DVec2, w: DVec2) -> Option<(f64, f64)> {
    let mut ss: Vec<f64> = Vec::new();
    for i in 0..poly.len() {
        let (a, e) = (poly[i], poly[(i + 1) % poly.len()] - poly[i]);
        let den = w.perp_dot(e);
        if den.abs() < 1e-12 {
            continue;
        }
        let d = a - c;
        let t = d.perp_dot(w) / den;
        if (-1e-9..=1.0 + 1e-9).contains(&t) {
            ss.push(d.perp_dot(e) / den);
        }
    }
    let lo = ss.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ss.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (ss.len() >= 2).then_some((lo, hi))
}

/// The fitting surface of the smooth-border shell (the zero level of the
/// closed field relieved over the gingiva) on the model with the
/// interproximal block-out: under the contact of every two touching
/// neighbours, at every height down to 3 mm under it, it stays within the
/// recess (+ a voxel and a tenth) of the lines tangent to the two crowns: the
/// wall bridges the embrasure and never goes down into it onto the papilla
/// (without the block-out, the shell the export laid before, it did: the
/// test checks that too). At the stage pose of the teeth, first and last
/// stage. Away from the block-out the field, so the fit of the shell on the
/// crowns, is the same to a tenth of a voxel (the closing resampled).
#[test]
fn the_smooth_shell_bridges_every_embrasure_and_keeps_the_crowns() {
    let voxel = 0.25;
    let tol = voxel + 0.1;
    let design = Design {
        thickness_mm: 1.0,
        clearance_mm: 0.0,
        gum_clearance_mm: 0.3,
    };
    for k in [Some(0), None] {
        let st = stage("demo_lower_mild", k);
        let input = Input {
            to_arch: st.to_arch,
            teeth: &st.teeth,
            gingiva: &st.gingiva,
            cover: &[],
            fdis: &st.fdis,
            covered: None,
            stumps: &[],
        };
        let bare = Input { fdis: &[], ..input };
        let fitting = |parts: &TriMesh| {
            let closed =
                shell::closed_grid_for(parts, voxel, shell::SMOOTH_BLOCKOUT_MM, 1.0).unwrap();
            shell::relieved_shell(closed, &shell::gum_surface(&input), &design)
                .unwrap()
                .0
        };
        let old_parts = shell::model_parts(&bare, shell::BASE_HEIGHT_MM).unwrap();
        let new_parts = shell::model_parts_inset(&input, shell::BASE_HEIGHT_MM, RECESS_MM).unwrap();
        let (old, new) = (fitting(&old_parts), fitting(&new_parts));
        // the block-out: what the soup with it adds
        let n0 = old_parts.positions.len() as u32;
        let solids = TriMesh::new(
            new_parts.positions.clone(),
            new_parts
                .tris
                .iter()
                .filter(|t| t[0] >= n0)
                .copied()
                .collect(),
        );
        assert!(!solids.tris.is_empty(), "stage {k:?}: no block-out");
        let teeth: BTreeMap<Fdi, (Vec<DVec3>, &TriMesh)> = st
            .fdis
            .iter()
            .zip(&st.teeth)
            .map(|(f, t)| {
                let p = t
                    .positions
                    .iter()
                    .map(|p| st.to_arch.transform_point3(p.as_dvec3()))
                    .collect();
                (*f, (p, t))
            })
            .collect();
        let order: Vec<Fdi> = arch_order(Arch::of(st.fdis[0]))
            .into_iter()
            .filter(|f| teeth.contains_key(f))
            .collect();
        let cen = |p: &[DVec3]| p.iter().map(|v| v.truncate()).sum::<DVec2>() / p.len() as f64;
        let (mut pairs, mut points, mut bad_new, mut bad_old) = (0, 0, 0, 0);
        for w2 in order.windows(2) {
            let ((pa, ma), (pb, mb)) = (&teeth[&w2[0]], &teeth[&w2[1]]);
            let a32: Vec<glam::Vec3> = pa.iter().map(|p| p.as_vec3()).collect();
            let knn = dc_geom::knn::KnnIndex::build(&a32).unwrap();
            let close: Vec<(f64, DVec3)> = pb
                .iter()
                .filter_map(|p| {
                    let q = pa[knn.nearest(p.as_vec3())?.index as usize];
                    Some((q.distance(*p), (q + *p) * 0.5))
                })
                .collect();
            let dmin = close.iter().map(|x| x.0).fold(f64::INFINITY, f64::min);
            if dmin > shell::embrasure::TOUCH_MM {
                continue;
            }
            pairs += 1;
            let c = close
                .iter()
                .filter(|x| x.0 <= dmin + 0.1)
                .map(|x| x.1)
                .max_by(|a, b| a.z.total_cmp(&b.z))
                .unwrap();
            let u = (cen(pb) - cen(pa)).normalize();
            let w = DVec2::new(-u.y, u.x);
            let mut z = c.z - 0.3;
            while z > c.z - 3.0 {
                let mut sec = section(pa, &ma.tris, z);
                let sb = section(pb, &mb.tris, z);
                if sec.len() >= 3 && sb.len() >= 3 {
                    sec.extend(sb);
                    if let Some((lo, hi)) = line_range(&hull(&sec), c.truncate(), w) {
                        let mut s = lo + RECESS_MM + tol;
                        while s <= hi - RECESS_MM - tol {
                            let p = (c.truncate() + s * w).extend(z).as_vec3();
                            points += 1;
                            bad_new += usize::from(f64::from(new.sample(p)) > 0.5 * voxel);
                            bad_old += usize::from(f64::from(old.sample(p)) > 0.5 * voxel);
                            s += 0.1;
                        }
                    }
                }
                z -= 0.25;
            }
        }
        assert!(
            pairs >= 8 && points > 1000,
            "stage {k:?}: {pairs} pairs, {points} points"
        );
        assert_eq!(
            bad_new, 0,
            "stage {k:?}: the shell enters {bad_new} of {points} points"
        );
        assert!(
            bad_old > points / 20,
            "stage {k:?}: without the block-out the shell entered {bad_old} of {points} points"
        );
        // the crowns away from the block-out: the same fit
        let bvh = dc_geom::bvh::Bvh::build(&solids).unwrap();
        let (mut kept, mut worst) = (0, 0.0f64);
        for (p, _) in teeth.values() {
            for v in p {
                let q = v.as_vec3();
                let o = f64::from(old.sample(q));
                if o.abs() > 0.5 || bvh.closest_point(q, 2.0).is_some() {
                    continue;
                }
                kept += 1;
                worst = worst.max((f64::from(new.sample(q)) - o).abs());
            }
        }
        eprintln!(
            "stage {k:?}: {pairs} pairs, {points} embrasure points ({bad_old} entered \
             without the block-out), {kept} crown points, crown fit moved at most {worst} mm"
        );
        assert!(kept > 5_000, "stage {k:?}: {kept} crown points");
        // the closing of the model resampled on its 0.25 mm voxels: a
        // tenth of the voxel at most
        assert!(
            worst < 0.1 * voxel,
            "stage {k:?}: the crown fit moved by {worst} mm"
        );
    }
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
        covered: None,
        stumps: &[],
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

/// The contact bridges of the port against the plugin's `pair_bridge`
/// (`golden/bridge_oracle.py`, Blender headless) on the same closed teeth
/// placed at a stage: the same pairs of teeth are bridged, and every bridge
/// is the same solid, vertex for vertex.
///
/// The plugin runs its float64 queries (`gpu_query` on the GPU): on the CPU
/// its float32 BVH (mathutils) misses a hit closer to the origin of a ray
/// than its rounding, which the float64 path finds (demo_upper_mild stage 0,
/// 23-24: one line starting 4.5e-7 mm inside 23, the bridge 1.3 % larger).
/// Without the GPU the test is skipped.
#[test]
#[ignore = "needs Blender (blender -b), the plugin repository and its GPU queries: run with --release --ignored --nocapture"]
fn contact_bridges_match_the_plugin() {
    use dca_algo::anatomy::{Arch, arch_order};
    use dca_algo::shell::bridge::{self, BridgeParams};
    use dca_algo::shell::embrasure::Tooth;
    const VERTEX_TOL_MM: f64 = 1e-9;

    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    if std::process::Command::new(&blender)
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: {blender} not found");
        return;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/bridge_oracle.py");
    let mut bridged = 0;
    for (name, k) in [
        ("demo_lower_mild", Some(0)),
        ("demo_lower_mild", None),
        ("demo_upper_mild", Some(0)),
        ("demo_upper_moderate", None),
    ] {
        let st = stage(name, k);
        // the plugin reads the points as float32: the port gets the same
        let points: Vec<Vec<DVec3>> = st
            .teeth
            .iter()
            .map(|t| {
                t.positions
                    .iter()
                    .map(|p| {
                        st.to_arch
                            .transform_point3(p.as_dvec3())
                            .as_vec3()
                            .as_dvec3()
                    })
                    .collect()
            })
            .collect();
        let placed: Vec<Tooth<'_>> = st
            .fdis
            .iter()
            .zip(&points)
            .zip(&st.teeth)
            .map(|((&fdi, points), mesh)| Tooth {
                fdi,
                points,
                tris: &mesh.tris,
            })
            .collect();
        let params = BridgeParams::default();
        let t = Instant::now();
        let port = bridge::bridges_by_pair(&placed, &params);
        let secs = t.elapsed().as_secs_f64();

        // the same teeth, in arch order, through the plugin
        let dir = std::env::temp_dir().join(format!("dc_bridges_{}_{name}", std::process::id()));
        let (src, out) = (dir.join("in"), dir.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        let order: Vec<Fdi> = arch_order(Arch::of(st.fdis[0]))
            .into_iter()
            .filter(|f| st.fdis.contains(f))
            .collect();
        for t in &placed {
            let mesh = TriMesh::new(
                t.points.iter().map(|p| p.as_vec3()).collect(),
                t.tris.to_vec(),
            );
            write_raw(&mesh, &src.join(format!("tooth_{}", t.fdi.get())));
        }
        let lines: Vec<String> = order.iter().map(|f| f.get().to_string()).collect();
        std::fs::write(src.join("order.txt"), lines.join("\n")).unwrap();
        let p = serde_json::json!({
            "reach": params.reach,
            "gingival": params.gingival,
            "touch": params.touch,
            "above": params.above,
            "inset": params.inset,
        });
        std::fs::write(src.join("params.json"), p.to_string()).unwrap();
        let status = std::process::Command::new(&blender)
            .args(["-b", "--factory-startup", "-P"])
            .arg(&script)
            .arg("--")
            .arg(&src)
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success(), "blender run failed");
        let report = json(&out.join("bridges.json"));
        if report["gpu"].as_bool() != Some(true) {
            std::fs::remove_dir_all(&dir).unwrap();
            println!("skipped: the plugin's float64 (GPU) queries are not available");
            return;
        }
        let pairs = report["pairs"].as_array().unwrap();
        println!(
            "{name} stage {k:?}: port {secs:.2} s, plugin {:.2} s",
            report["seconds"].as_f64().unwrap()
        );
        let mut failures = Vec::new();
        for pr in pairs {
            let (a, b) = (pr["a"].as_u64().unwrap(), pr["b"].as_u64().unwrap());
            let mine = port
                .iter()
                .find(|(x, y, _)| u64::from(x.get()) == a && u64::from(y.get()) == b);
            let theirs = pr["bridged"].as_bool().unwrap();
            match (mine, theirs) {
                (None, false) => {}
                (Some((_, _, s)), true) => {
                    bridged += 1;
                    let stem = out.join(format!("bridge_{a}_{b}"));
                    let ours = TriMesh::new(s.0.iter().map(|p| p.as_vec3()).collect(), s.1.clone());
                    assert!(ours.validate().is_watertight(), "{a}-{b}: not closed");
                    let plugin = read_raw(&stem);
                    let v64: Vec<DVec3> = std::fs::read(stem.with_extension("v64.bin"))
                        .unwrap()
                        .as_chunks::<24>()
                        .0
                        .iter()
                        .map(|c| {
                            let f = |k: usize| f64::from_le_bytes(c[k..k + 8].try_into().unwrap());
                            DVec3::new(f(0), f(8), f(16))
                        })
                        .collect();
                    // the same nodes of the same cells: vertex for vertex
                    if s.1 != plugin.tris || s.0.len() != v64.len() {
                        let (max, p999) = shell::deviation_stats(&ours, &plugin).unwrap();
                        println!(
                            "  {a}-{b}: other cells, surfaces p99.9 {p999:.4} mm, max {max:.4} mm"
                        );
                        failures.push(format!("{a}-{b}: other cells"));
                        continue;
                    }
                    let dv =
                        s.0.iter()
                            .zip(&v64)
                            .map(|(p, q)| p.distance(*q))
                            .fold(0.0f64, f64::max);
                    println!("  {a}-{b}: {} vertices, max {dv:.2e} mm", v64.len());
                    if dv > VERTEX_TOL_MM {
                        failures.push(format!("{a}-{b}: {dv:.2e} mm"));
                    }
                }
                (m, t) => {
                    failures.push(format!("{a}-{b}: port bridged {}, plugin {t}", m.is_some()))
                }
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(failures.is_empty(), "{name} stage {k:?}: {failures:#?}");
    }
    assert!(
        bridged > 0,
        "no contact was bridged: the oracle compared nothing"
    );
}

/// The stumps of the closed teeth drawn under the gingiva of the cast
/// ([`shell::tuck_below_margin`]) against the plugin's `_tuck_below_margin`
/// (`golden/tuck_oracle.py`, Blender headless) on the same closed teeth,
/// with their scanned part (`closed_tooth_info`), and the same cast (the
/// port's `gum_cast` of the stage): the same vertices below the margin,
/// every vertex where the plugin puts it. The cast is also lowered by
/// `lower` mm (a gingiva thinner over the roots) so that stumps stand out.
#[test]
#[ignore = "needs Blender (blender -b) and the plugin repository: run with --release --ignored --nocapture"]
fn stumps_are_tucked_as_the_plugin_does() {
    use std::collections::HashSet;

    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    if std::process::Command::new(&blender)
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: {blender} not found");
        return;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/tuck_oracle.py");
    let f64s = |v: &[f64]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut moved = 0usize;
    for (name, k) in [
        ("demo_lower_mild", Some(0)),
        ("demo_lower_mild", None),
        ("demo_upper_mild", Some(0)),
        ("demo_upper_moderate", None),
    ] {
        let st = stage(name, k);
        let a = st.to_arch;
        let vg: Vec<DVec3> = st
            .gingiva
            .positions
            .iter()
            .map(|p| a.transform_point3(p.as_dvec3()))
            .collect();
        // the crowns one per CROWN_STEP_MM cell, as `model_parts` gives them
        let mut seen = HashSet::new();
        let mut crown_xy = Vec::new();
        for t in &st.teeth {
            for p in &t.positions {
                let q = a.transform_point3(p.as_dvec3());
                if seen.insert((q / shell::CROWN_STEP_MM).floor().as_i64vec3()) {
                    crown_xy.push([q.x, q.y]);
                }
            }
        }
        let c = shell::cast::gum_cast(
            &vg,
            &st.gingiva.tris,
            &crown_xy,
            &shell::cast::CastParams::default(),
            &st.gingiva.tris,
            &[],
        );
        for lower in [0.0, 1.0] {
            let mut gum = c.info.clone();
            for h in &mut gum.h {
                *h -= lower;
            }
            let g = &gum.grid;
            let dir =
                std::env::temp_dir().join(format!("dc_tuck_{}_{name}_{lower}", std::process::id()));
            let (src, out) = (dir.join("in"), dir.join("out"));
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(src.join("h.bin"), f64s(&gum.h)).unwrap();
            let mut teeth = Vec::new();
            let mut ours = Vec::new();
            for (fdi, (local, frame, scanned)) in st.fdis.iter().zip(&st.local) {
                let p: Vec<DVec3> = local.positions.iter().map(|p| p.as_dvec3()).collect();
                let m = a * *frame;
                let rows: Vec<f64> = (0..16).map(|k| m.col(k % 4)[k / 4]).collect();
                let stem = src.join(format!("tooth_{}", fdi.get()));
                let flat: Vec<f64> = p.iter().flat_map(|q| q.to_array()).collect();
                std::fs::write(stem.with_extension("p.bin"), f64s(&flat)).unwrap();
                std::fs::write(stem.with_extension("m.bin"), f64s(&rows)).unwrap();
                teeth.push(serde_json::json!({ "fdi": fdi.get(), "crown_vertices": scanned }));
                let below = shell::below_margin(&p, *scanned);
                ours.push((
                    *fdi,
                    below.clone(),
                    shell::tuck_below_margin(&p, &m, &below, &gum),
                    p.iter().map(|q| m.transform_point3(*q)).collect::<Vec<_>>(),
                ));
            }
            let spec = serde_json::json!({
                "nx": g.nx, "ny": g.ny, "lo": [g.lo.x, g.lo.y], "cell": g.cell, "teeth": teeth,
            });
            std::fs::write(src.join("tuck.json"), spec.to_string()).unwrap();
            let status = std::process::Command::new(&blender)
                .args(["-b", "--factory-startup", "-P"])
                .arg(&script)
                .arg("--")
                .arg(&src)
                .arg(&out)
                .status()
                .unwrap();
            assert!(status.success(), "blender run failed");
            let (mut worst, mut drawn, mut stumps) = (0.0f64, 0usize, 0usize);
            for (fdi, below, tucked, placed) in &ours {
                let stem = out.join(format!("tooth_{}", fdi.get()));
                let mask = std::fs::read(stem.with_extension("below.bin")).unwrap();
                let theirs: Vec<u32> = (0..mask.len() as u32)
                    .filter(|&i| mask[i as usize] != 0)
                    .collect();
                assert_eq!(
                    *below, theirs,
                    "{name} {k:?} {fdi}: other vertices below the margin"
                );
                let w = std::fs::read(stem.with_extension("w.bin")).unwrap();
                let w: Vec<DVec3> = w
                    .as_chunks::<24>()
                    .0
                    .iter()
                    .map(|c| {
                        let f = |k: usize| f64::from_le_bytes(c[k..k + 8].try_into().unwrap());
                        DVec3::new(f(0), f(8), f(16))
                    })
                    .collect();
                assert_eq!(w.len(), tucked.len());
                for ((q, t), p) in tucked.iter().zip(&w).zip(placed) {
                    worst = worst.max(q.distance(*t));
                    drawn += usize::from(q.distance(*p) > 1e-9);
                }
                stumps += below.len();
            }
            std::fs::remove_dir_all(&dir).unwrap();
            println!(
                "{name} stage {k:?}, cast lowered {lower} mm: {stumps} stump vertices, {drawn} drawn in, max {worst:.2e} mm from the plugin"
            );
            assert!(worst < 1e-9, "{name} {k:?} {lower}: {worst} mm");
            moved += drawn;
        }
    }
    assert!(
        moved > 0,
        "no stump stood out of the gum: the oracle compared nothing"
    );
}
