//! The segmentation (`dca_algo::segmentation`) on the four real arch scans,
//! compared with the Python plugin's `case_teeth.segment_iter` on the same
//! files and the same seeds.
//!
//! The scans are patient data: they are not in the repository, and neither
//! their file names nor any coordinate may appear in committed code or in
//! test output (see `dca-module/tests/import_real.rs`). The folder comes from
//! `DC_REAL_SCANS`; the files are sorted by name and called `a` to `d`
//! (lower, upper, lower, upper).
//!
//! What the plugin finds is read from the folder `DC_SEGMENTATION_REAL_ORACLE`
//! names, written by `fixtures/gen_segmentation_real.py` and kept outside the
//! repository (read its header): per scan `<letter>.json` (the plugin's
//! automatic seeds, its arch frame, the face labels of the simplified mesh,
//! the vertex labels after the transfer and the final ones) and
//! `<letter>_decimated.dcm` (the scan simplified by Blender's Decimate
//! modifier, arch coordinates). Regenerate it, run, delete it:
//!
//! ```sh
//! out=$(mktemp -d /tmp/dc-segreal-XXXX)
//! for l in a b c d; do
//!   DC_REAL_SCANS=<scans> python3 -B crates/dca-algo/tests/fixtures/gen_segmentation_real.py "$out" --only $l &
//! done; wait          # Blender on PATH; several minutes per full-size scan
//! DC_REAL_SCANS=<scans> DC_SEGMENTATION_REAL_ORACLE="$out" cargo test --release -p dca-algo \
//!     --test segmentation_real -- --nocapture --test-threads=1
//! rm -r "$out"
//! ```
//!
//! Without `DC_REAL_SCANS` or `DC_SEGMENTATION_REAL_ORACLE` the tests are
//! skipped with a line on stderr. Run them in release (debug takes minutes
//! per scan) and one at a time for meaningful timings. The messages carry
//! tooth numbers, fractions and seconds, never positions.
//!
//! Both sides start from the plugin's seeds; the arch frame is the port's
//! (`frame::arch_frame_from_seeds`), checked against the plugin's. Two
//! comparisons per scan, measured on the four scans on 2026-11-08 (plugin
//! 42e6d87, Blender 5.2.2):
//!
//! * Blender's simplified mesh (`<letter>_decimated.dcm`) fed to the port:
//!   face labels, transferred and final vertex labels agree with the plugin
//!   on at least 99 % (PLAN section 5, as `segmentation_oracle.rs`), and the
//!   teeth are the same. Measured: face and transferred labels 1.00000 on
//!   all four, final labels 1.00000 / 1.00000 / 1.00000 / 0.99997, the same
//!   CG iteration counts as the plugin.
//! * The full pipeline with the port's own simplification ([`dc_geom::qem`]
//!   instead of Blender's Decimate): the same teeth as the plugin, and the
//!   final vertex labels agree on at least [`FULL_MIN_AGREEMENT`]. The
//!   simplified meshes differ, so the first partition differs along the
//!   tooth boundaries (transferred labels 0.981 to 0.994); the boundary
//!   refinement at full resolution redraws most of it. Measured final
//!   agreement 0.99797 / 0.99479 / 0.99688 / 0.99660: 58 % to 96 % of the
//!   vertices that differ lie within 3 edges of a boundary of the plugin's
//!   labels, the rest is a patch (up to 16 edges deep) of a last molar
//!   that goes to its neighbouring molar or to the gingiva, where the first
//!   partition put that boundary farther than the refinement band reaches
//!   (worst tooth IoU 0.83). The threshold is the plan's 99 %: every scan
//!   clears it by at least 0.0047, and both simplifications are
//!   deterministic, so it fails only when the result changes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dc_case::Fdi;
use dc_geom::metrics::label_agreement;
use dca_algo::anatomy::Arch;
use dca_algo::frame::{arch_frame_from_seeds, rigid_inverse};
use dca_algo::segmentation::{self, Input, Output, Params};
use dca_model::Seed;
use glam::{DMat4, DVec3};
use serde_json::Value;

/// Agreement of the final vertex labels with Blender's simplified mesh
/// (PLAN section 5).
const TIER_A_MIN_AGREEMENT: f64 = 0.99;
/// Agreement of the final vertex labels with the port's own simplification,
/// see the module documentation.
const FULL_MIN_AGREEMENT: f64 = 0.99;
/// The port's arch frame against the plugin's, from the same seeds: origin
/// (mm) and axes (degrees).
const FRAME_TOL_MM: f64 = 1e-6;
const FRAME_TOL_DEG: f64 = 1e-6;

/// The four scans sorted by name, or `None` (and a line on stderr).
fn scans() -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os("DC_REAL_SCANS").map(PathBuf::from) else {
        eprintln!("skipped: DC_REAL_SCANS is not set");
        return None;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("stl"))
        })
        .collect();
    files.sort();
    if files.len() != 4 {
        eprintln!(
            "skipped: {} STL files in the real scan folder (set DC_REAL_SCANS), the test needs 4",
            files.len()
        );
        return None;
    }
    Some(files)
}

/// The plugin's output folder, or `None` (and a line on stderr).
fn oracle_dir() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("DC_SEGMENTATION_REAL_ORACLE").map(PathBuf::from) else {
        eprintln!("skipped: DC_SEGMENTATION_REAL_ORACLE is not set");
        return None;
    };
    Some(dir)
}

/// Run-length encoded labels `{"n": N, "rle": [label, count, ...]}`.
fn unpack(v: &Value) -> Vec<u16> {
    let rle = v["rle"].as_array().unwrap();
    let mut out = Vec::with_capacity(v["n"].as_u64().unwrap() as usize);
    for pair in rle.chunks(2) {
        let (l, n) = (pair[0].as_u64().unwrap(), pair[1].as_u64().unwrap());
        out.extend(std::iter::repeat_n(l as u16, n as usize));
    }
    assert_eq!(out.len() as u64, v["n"].as_u64().unwrap());
    out
}

fn vec3(v: &Value) -> DVec3 {
    let n: Vec<f64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    DVec3::new(n[0], n[1], n[2])
}

/// Row-major 4x4 of the plugin.
fn mat4(v: &Value) -> DMat4 {
    let rows: Vec<Vec<f64>> = serde_json::from_value(v.clone()).unwrap();
    DMat4::from_cols_array_2d(&std::array::from_fn(|j| {
        std::array::from_fn(|i| rows[i][j])
    }))
}

/// Order-sensitive checksum of the vertices and its scale, as
/// `gen_segmentation_real.py` computes it.
fn fingerprint(v: &[DVec3]) -> (f64, f64) {
    v.iter().enumerate().fold((0.0, 0.0), |(a, b), (i, p)| {
        let w = (i % 997 + 1) as f64;
        let s = p.x + 2.0 * p.y + 3.0 * p.z;
        (a + w * s, b + w * s.abs())
    })
}

/// One scan in the port's arch frame, with the plugin's seeds and output.
struct Scan {
    letter: char,
    arch: Arch,
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    seeds: Vec<(Fdi, DVec3)>,
    oracle: Value,
    dir: PathBuf,
}

fn load(letter: char, path: &Path, dir: &Path) -> Scan {
    let json = dir.join(format!("{letter}.json"));
    let oracle: Value = serde_json::from_slice(
        &std::fs::read(&json).unwrap_or_else(|_| panic!("scan {letter}: no {letter}.json")),
    )
    .unwrap_or_else(|_| panic!("scan {letter}: {letter}.json is not JSON"));
    let arch = match oracle["arch"].as_str() {
        Some("UPPER") => Arch::Upper,
        Some("LOWER") => Arch::Lower,
        other => panic!("scan {letter}: the oracle has no arch ({other:?})"),
    };
    let mesh = dc_mesh::io::read(path).unwrap_or_else(|_| panic!("scan {letter}: unreadable"));
    let world: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    assert_eq!(
        (oracle["vertices"].as_u64(), oracle["faces"].as_u64()),
        (Some(world.len() as u64), Some(mesh.tris.len() as u64)),
        "scan {letter}: the plugin saw another mesh"
    );
    // the labels compare vertex by vertex: the same vertex numbering
    let (fp, scale) = fingerprint(&world);
    let want = &oracle["fingerprint"];
    assert!(
        (fp - want[0].as_f64().unwrap()).abs() <= 1e-9 * scale.max(want[1].as_f64().unwrap()),
        "scan {letter}: the plugin numbered the vertices differently"
    );

    let seeds_world: Vec<Seed> = oracle["seeds_world"]
        .as_object()
        .unwrap_or_else(|| panic!("scan {letter}: the oracle has no seeds"))
        .iter()
        .map(|(f, p)| Seed {
            fdi: Fdi::new(f.parse().unwrap()).unwrap(),
            position: vec3(p),
        })
        .collect();
    let centroid = world.iter().copied().sum::<DVec3>() / world.len() as f64;
    let frame = arch_frame_from_seeds(&seeds_world, centroid)
        .unwrap_or_else(|e| panic!("scan {letter}: {e}"))
        .matrix();
    let m = mat4(&oracle["frame"]);
    let d_origin = (frame.w_axis - m.w_axis).truncate().length();
    let d_axes = (0..3)
        .map(|i| {
            let (a, b) = (frame.col(i).truncate(), m.col(i).truncate());
            a.angle_between(b).to_degrees()
        })
        .fold(0.0, f64::max);
    assert!(
        d_origin <= FRAME_TOL_MM && d_axes <= FRAME_TOL_DEG,
        "scan {letter}: arch frame differs from the plugin's by {d_origin:.2e} mm, {d_axes:.2e} deg"
    );

    let to_arch = rigid_inverse(&frame);
    let mut seeds: Vec<(Fdi, DVec3)> = seeds_world
        .iter()
        .map(|s| (s.fdi, to_arch.transform_point3(s.position)))
        .collect();
    seeds.sort_by_key(|s| s.0);
    Scan {
        letter,
        arch,
        vertices: world.iter().map(|&p| to_arch.transform_point3(p)).collect(),
        tris: mesh.tris,
        seeds,
        oracle,
        dir: dir.to_path_buf(),
    }
}

/// The teeth (non-zero labels) present in `labels`.
fn teeth(labels: &[u16]) -> BTreeSet<u16> {
    labels.iter().copied().filter(|&l| l != 0).collect()
}

/// Intersection over union of every label of `a` or `b`.
fn iou(a: &[u16], b: &[u16]) -> Vec<(u16, f64)> {
    let mut n = std::collections::BTreeMap::<u16, [usize; 3]>::new();
    for (&x, &y) in a.iter().zip(b) {
        n.entry(x).or_default()[0] += 1;
        n.entry(y).or_default()[1] += 1;
        if x == y {
            n.entry(x).or_default()[2] += 1;
        }
    }
    n.into_iter()
        .map(|(l, [na, nb, both])| (l, both as f64 / (na + nb - both) as f64))
        .collect()
}

/// Edge rings from the nearest boundary of `labels` (a vertex with a
/// neighbour of another label is ring 0); `u32::MAX` where no boundary is
/// reachable.
fn rings_from_boundary(n: usize, tris: &[[u32; 3]], labels: &[u16]) -> Vec<u32> {
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in tris {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            adj[a as usize].push(b);
            adj[b as usize].push(a);
        }
    }
    let mut ring = vec![u32::MAX; n];
    let mut front: Vec<u32> = (0..n as u32)
        .filter(|&v| {
            adj[v as usize]
                .iter()
                .any(|&w| labels[w as usize] != labels[v as usize])
        })
        .collect();
    for &v in &front {
        ring[v as usize] = 0;
    }
    let mut d = 0;
    while !front.is_empty() {
        d += 1;
        let mut next = Vec::new();
        for v in front {
            for &w in &adj[v as usize] {
                if ring[w as usize] == u32::MAX {
                    ring[w as usize] = d;
                    next.push(w);
                }
            }
        }
        front = next;
    }
    ring
}

/// Runs the port on a scan (with Blender's simplified mesh, or its own) and
/// returns the output and the seconds it took.
fn segment(s: &Scan, decimated: Option<(&[DVec3], &[[u32; 3]])>) -> (Output, f64) {
    let t0 = Instant::now();
    let out = segmentation::run(
        &Input {
            arch: s.arch,
            vertices: &s.vertices,
            tris: &s.tris,
            seeds: &s.seeds,
            decimated,
        },
        &Params::default(),
    )
    .unwrap_or_else(|e| panic!("scan {}: {e}", s.letter));
    (out, t0.elapsed().as_secs_f64())
}

/// Final labels against the plugin's: agreement, the same teeth, the worst
/// tooth IoU (printed).
fn compare_final(s: &Scan, out: &Output, what: &str, secs: f64) -> f64 {
    let l = s.letter;
    let want = unpack(&s.oracle["vertex_labels"]);
    let agree = label_agreement(&out.labels, &want).unwrap();
    let per = iou(&out.labels, &want);
    let gum = per.iter().find(|p| p.0 == 0).map_or(1.0, |p| p.1);
    let worst = per
        .iter()
        .filter(|p| p.0 != 0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .copied()
        .unwrap_or((0, 1.0));
    eprintln!(
        "scan {l} ({what}): {} faces simplified, final labels agree {agree:.5}, gingiva IoU {gum:.4}, \
         worst tooth {} IoU {:.4}; {} teeth; port {secs:.1} s, plugin {} s",
        out.decimated_tris.len(),
        worst.0,
        worst.1,
        out.teeth,
        s.oracle["seconds"]["segmentation"],
    );
    if worst.1 < 1.0 {
        // which way the worst tooth differs, and against what
        let w = worst.0;
        let (mut extra, mut extra_gum, mut missing, mut missing_gum) = (0, 0, 0, 0);
        for (&g, &e) in out.labels.iter().zip(&want) {
            if g == w && e != w {
                extra += 1;
                extra_gum += usize::from(e == 0);
            } else if e == w && g != w {
                missing += 1;
                missing_gum += usize::from(g == 0);
            }
        }
        eprintln!(
            "scan {l} ({what}): tooth {w}: {extra} more vertices than the plugin ({extra_gum} of its \
             gingiva), {missing} fewer ({missing_gum} given to the gingiva)"
        );
    }
    let (got, exp) = (teeth(&out.labels), teeth(&want));
    assert_eq!(got, exp, "scan {l} ({what}): teeth");
    let seeded: BTreeSet<u16> = s.seeds.iter().map(|f| u16::from(f.0.get())).collect();
    if let Some(f) = seeded.difference(&exp).next() {
        eprintln!(
            "scan {l} ({what}): seeded tooth {f} has no vertex in the plugin's labels either"
        );
    }
    agree
}

/// Blender's simplified mesh fed to the port: every stage within 99 %.
fn check_blender_decimated(s: &Scan) {
    let l = s.letter;
    let dec = dc_mesh::dcm::decode(
        &std::fs::read(s.dir.join(format!("{l}_decimated.dcm")))
            .unwrap_or_else(|_| panic!("scan {l}: no {l}_decimated.dcm")),
    )
    .unwrap();
    let dv: Vec<DVec3> = dec.positions.iter().map(|p| p.as_dvec3()).collect();
    let (out, secs) = segment(s, Some((&dv, &dec.tris)));
    let faces = label_agreement(
        &out.face_labels_decimated,
        &unpack(&s.oracle["face_labels_decimated"]),
    )
    .unwrap();
    let transfer = label_agreement(
        &out.labels_transfer,
        &unpack(&s.oracle["vertex_labels_transfer"]),
    )
    .unwrap();
    eprintln!(
        "scan {l} (Blender's simplified mesh): face labels agree {faces:.5}, transferred {transfer:.5}; \
         {} CG iterations (plugin {})",
        out.diagnostics.cg_iterations, s.oracle["diagnostics"]["cg_iterations"],
    );
    let fin = compare_final(s, &out, "Blender's simplified mesh", secs);
    for (what, x) in [
        ("face labels", faces),
        ("transferred labels", transfer),
        ("final labels", fin),
    ] {
        assert!(
            x >= TIER_A_MIN_AGREEMENT,
            "scan {l}: {what} agree with the plugin at {x:.4}"
        );
    }
}

/// The port's own simplification: the same teeth, final labels within
/// [`FULL_MIN_AGREEMENT`].
fn check_full_pipeline(s: &Scan) {
    let (out, secs) = segment(s, None);
    let transfer = label_agreement(
        &out.labels_transfer,
        &unpack(&s.oracle["vertex_labels_transfer"]),
    )
    .unwrap();
    eprintln!(
        "scan {} (own simplification): transferred labels agree {transfer:.5} (before the refinement)",
        s.letter
    );
    // where the labels differ: how far from the plugin's boundaries
    let want = unpack(&s.oracle["vertex_labels"]);
    let ring = rings_from_boundary(s.vertices.len(), &s.tris, &want);
    let off: Vec<u32> = (0..want.len())
        .filter(|&i| out.labels[i] != want[i])
        .map(|i| ring[i])
        .collect();
    let near = off.iter().filter(|&&r| r <= 3).count();
    eprintln!(
        "scan {} (own simplification): {} vertices differ, {:.3} of them within 3 edges of a plugin \
         boundary, the farthest {} edges",
        s.letter,
        off.len(),
        near as f64 / off.len().max(1) as f64,
        off.iter().max().copied().unwrap_or(0),
    );
    let fin = compare_final(s, &out, "own simplification", secs);
    assert!(
        fin >= FULL_MIN_AGREEMENT,
        "scan {}: final labels agree with the plugin at {fin:.4}",
        s.letter
    );
}

fn with_scan(letter: char, check: fn(&Scan)) {
    let (Some(files), Some(dir)) = (scans(), oracle_dir()) else {
        return;
    };
    let i = (letter as u8 - b'a') as usize;
    check(&load(letter, &files[i], &dir));
}

#[test]
fn blender_simplified_real_scan_a_matches_the_plugin() {
    with_scan('a', check_blender_decimated);
}

#[test]
fn blender_simplified_real_scan_b_matches_the_plugin() {
    with_scan('b', check_blender_decimated);
}

#[test]
fn blender_simplified_real_scan_c_matches_the_plugin() {
    with_scan('c', check_blender_decimated);
}

#[test]
fn blender_simplified_real_scan_d_matches_the_plugin() {
    with_scan('d', check_blender_decimated);
}

#[test]
fn segmentation_of_real_scan_a_matches_the_plugin() {
    with_scan('a', check_full_pipeline);
}

#[test]
fn segmentation_of_real_scan_b_matches_the_plugin() {
    with_scan('b', check_full_pipeline);
}

#[test]
fn segmentation_of_real_scan_c_matches_the_plugin() {
    with_scan('c', check_full_pipeline);
}

#[test]
fn segmentation_of_real_scan_d_matches_the_plugin() {
    with_scan('d', check_full_pipeline);
}
