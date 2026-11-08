//! The automatic seeds (`dca_algo::autoseed`) on the four real arch scans,
//! compared with the Python plugin on the same files.
//!
//! The scans are patient data: they are not in the repository, and neither
//! their file names nor any coordinate may appear in committed code or in
//! test output (see `dca-module/tests/import_real.rs`). The folder comes from
//! `DC_REAL_SCANS`; the files are sorted by name and called `a` to `d`
//! (lower, upper, lower, upper).
//!
//! What the plugin finds on each scan, with its default settings (third
//! molars off, no missing teeth, ends chosen by the data), is read from the
//! JSON file `DC_AUTOSEED_REAL_ORACLE` names, written by
//! `fixtures/gen_autoseed_real.py` and kept outside the repository: the arch
//! and tooth list of each scan, its seed points and its arch frame. Nothing
//! about the patients is committed, not even the tooth lists. Without
//! `DC_REAL_SCANS` or `DC_AUTOSEED_REAL_ORACLE` the tests are skipped with a
//! line on stderr. The messages carry tooth numbers, distances and angles,
//! never positions.
//!
//! Tolerance on the seeds: 0.5 mm. A seed is a scan vertex picked inside the
//! bins of its tooth (`BIN_MM` = 0.25 mm along the arch), so the port either
//! picks the very same vertex (difference ~1e-9 mm) or, if a tooth boundary
//! moves by a bin, a neighbouring one; 0.5 mm allows two bins and stays
//! an order of magnitude below the narrowest tooth (~5 mm), so a seed on the
//! wrong tooth always fails.

use std::path::PathBuf;
use std::time::Instant;

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::autoseed::{self, Options};
use glam::{DMat4, DVec3};
use serde_json::Value;

/// Seed tolerance (mm), see the module documentation.
const SEED_TOL_MM: f64 = 0.5;
/// Arch frame tolerance: origin (mm) and axes (degrees).
const FRAME_TOL_MM: f64 = 0.05;
const FRAME_TOL_DEG: f64 = 0.05;

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

/// The plugin's output, or `None` (and a line on stderr).
fn oracle() -> Option<Value> {
    let Some(path) = std::env::var_os("DC_AUTOSEED_REAL_ORACLE") else {
        eprintln!("skipped: DC_AUTOSEED_REAL_ORACLE is not set");
        return None;
    };
    let text = std::fs::read_to_string(path).expect("the oracle file cannot be read");
    Some(serde_json::from_str(&text).expect("the oracle file is not JSON"))
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
    let rows: Vec<Vec<f64>> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect();
    DMat4::from_cols_array_2d(&std::array::from_fn(|j| {
        std::array::from_fn(|i| rows[i][j])
    }))
}

/// The arch and the sorted tooth list the plugin found on a scan, checked to
/// be valid tooth numbers of that arch.
fn expected(letter: char, o: &Value) -> (Arch, Vec<u8>) {
    let arch = match o["arch"].as_str() {
        Some("UPPER") => Arch::Upper,
        Some("LOWER") => Arch::Lower,
        other => panic!("scan {letter}: the oracle has no arch ({other:?})"),
    };
    let want: Vec<u8> = o["fdi"]
        .as_array()
        .unwrap_or_else(|| panic!("scan {letter}: the oracle has no tooth list"))
        .iter()
        .map(|n| {
            n.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or_else(|| panic!("scan {letter}: {n} is not a tooth number"))
        })
        .collect();
    assert!(
        want.windows(2).all(|w| w[0] < w[1]),
        "scan {letter}: the oracle's teeth are not sorted"
    );
    for &n in &want {
        let f = Fdi::new(n).unwrap_or_else(|_| panic!("scan {letter}: {n}"));
        assert_eq!(f.is_upper(), arch == Arch::Upper, "scan {letter}: {n}");
    }
    (arch, want)
}

/// One scan: the Rust proposal against the plugin's tooth list, seed points
/// and frame.
fn check(letter: char, path: &PathBuf, oracle: &Value) {
    let o = &oracle[letter.to_string()];
    let (arch, want) = expected(letter, o);
    let mesh = dc_mesh::io::read(path).unwrap_or_else(|_| panic!("scan {letter}: unreadable"));
    let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let t0 = Instant::now();
    let p = autoseed::propose(&v, &mesh.tris, arch, &Options::default(), None)
        .unwrap_or_else(|e| panic!("scan {letter}: {e}"));
    eprintln!(
        "scan {letter}: {} seeds in {:.1} s",
        p.seeds.len(),
        t0.elapsed().as_secs_f64()
    );
    let mut got: Vec<u8> = p.seeds.iter().map(|s| s.fdi.get()).collect();
    got.sort_unstable();
    assert_eq!(got, want, "scan {letter}: teeth");

    assert_eq!(
        o["vertices"].as_u64(),
        Some(v.len() as u64),
        "scan {letter}: the plugin saw another mesh"
    );
    let mut worst: (f64, u8) = (0.0, 0);
    for s in &p.seeds {
        let w = vec3(&o["seeds"][s.fdi.get().to_string()]);
        let d = (s.position - w).length();
        if d > worst.0 {
            worst = (d, s.fdi.get());
        }
    }
    eprintln!(
        "scan {letter}: largest seed difference {:.2e} mm (tooth {})",
        worst.0, worst.1
    );
    assert!(
        worst.0 <= SEED_TOL_MM,
        "scan {letter}: seed of tooth {} is {:.3} mm from the plugin's",
        worst.1,
        worst.0
    );
    let m = mat4(&o["frame"]);
    let d_origin = (p.frame.w_axis - m.w_axis).truncate().length();
    let d_axes = [0, 1, 2]
        .map(|i| {
            let (a, b) = (p.frame.col(i).truncate(), m.col(i).truncate());
            a.angle_between(b).to_degrees()
        })
        .into_iter()
        .fold(0.0, f64::max);
    eprintln!("scan {letter}: frame differs by {d_origin:.2e} mm, {d_axes:.2e} deg");
    assert!(
        d_origin <= FRAME_TOL_MM && d_axes <= FRAME_TOL_DEG,
        "scan {letter}: arch frame differs by {d_origin:.3} mm, {d_axes:.3} deg"
    );
}

fn run(letter: char) {
    let (Some(files), Some(oracle)) = (scans(), oracle()) else {
        return;
    };
    let i = (letter as u8 - b'a') as usize;
    check(letter, &files[i], &oracle);
}

#[test]
fn seeds_on_real_scan_a_match_the_plugin() {
    run('a');
}

#[test]
fn seeds_on_real_scan_b_match_the_plugin() {
    run('b');
}

#[test]
fn seeds_on_real_scan_c_match_the_plugin() {
    run('c');
}

#[test]
fn seeds_on_real_scan_d_match_the_plugin() {
    run('d');
}
