//! Scan import on four real arch scans (two lower, two upper, 100k to 430k
//! vertices each), through `aligners.case.import-scans`.
//!
//! The scans are patient data: they are not in the repository, and neither
//! their file names nor any coordinate may appear in committed code or in
//! test output. The folder comes from `DC_REAL_SCANS`. The tests are skipped,
//! with a line on stderr, when it is unset or has no four STL files.
//!
//! The files are found by glob and sorted by name, and called `scan_a` to
//! `scan_d` here. The sorted order is lower, upper, lower, upper. The
//! neutral runs go through symlinks in a temporary folder named `scan_a.stl`
//! ... so that no hint is left in the file name; the original names (which
//! name the arch) are used only in the role-from-name test, and assertion
//! messages carry the letter only.
//!
//! The counts are the ones of the Blender add-on (the oracle) on the same
//! files, rounded so that the repository does not fingerprint a scan, and the
//! tests accept 5 % around them; the detection numbers are what `detect_arch`
//! gives on the neutral copies.

use std::path::{Path, PathBuf};
use std::time::Instant;

use dc_case::CaseState;
use dc_command::JobCtx;
use dca_module::commands;
use serde_json::{Value, json};

const IMPORT_SCANS: &str = "aligners.case.import-scans";

/// One real scan: its letter, the path in the dataset, and what it must be.
struct Real {
    letter: char,
    path: PathBuf,
    /// Faces of the file as the plugin counts them, rounded.
    faces: usize,
    /// Vertices, rounded.
    vertices: usize,
    arch: &'static str,
    /// Cue and confidence of the detection on a file with a neutral name.
    source: &'static str,
    confidence: f64,
}

/// True when `count` is within 5 % of the rounded figure `about`.
fn near(count: usize, about: usize) -> bool {
    count.abs_diff(about) * 20 <= about
}

/// The four scans, sorted by file name, or `None` (and a line on stderr)
/// when `DC_REAL_SCANS` is unset or the folder has no four STL files.
fn scans() -> Option<[Real; 4]> {
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
    let spec = [
        ('a', 860_000, 430_000, "lower", "curve_of_wilson", 0.80),
        ('b', 800_000, 400_000, "upper", "palate", 1.00),
        ('c', 220_000, 110_000, "lower", "curve_of_wilson", 0.90),
        ('d', 200_000, 100_000, "upper", "scanner_orientation", 0.80),
    ];
    let mut it = files.into_iter();
    Some(
        spec.map(|(letter, faces, vertices, arch, source, confidence)| Real {
            letter,
            path: it.next().unwrap(),
            faces,
            vertices,
            arch,
            source,
            confidence,
        }),
    )
}

/// Symlinks `scan_a.stl` ... in a fresh temporary folder, removed on drop.
struct Neutral {
    dir: PathBuf,
}

impl Neutral {
    fn new(tag: &str, real: &[Real; 4]) -> Self {
        let dir = std::env::temp_dir().join(format!("dca-real-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for r in real {
            std::os::unix::fs::symlink(&r.path, dir.join(format!("scan_{}.stl", r.letter)))
                .unwrap();
        }
        Self { dir }
    }

    fn path(&self, letter: char) -> PathBuf {
        self.dir.join(format!("scan_{letter}.stl"))
    }
}

impl Drop for Neutral {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs the import command on `paths` into an empty case; the JSON output.
/// The error text is not returned (it would name the file): it is replaced
/// by a fixed sentence.
fn import(paths: &[&Path]) -> Value {
    let files: Vec<Value> = paths.iter().map(|p| json!({ "path": p })).collect();
    commands::find(IMPORT_SCANS)
        .unwrap()
        .execute(
            &CaseState::default(),
            &json!({ "files": files }),
            &JobCtx::detached(),
        )
        .map(|(_, out)| out)
        .unwrap_or_else(|_| panic!("the import of {} file(s) failed", paths.len()))
}

/// The entry of `imported` for `path`.
fn imported<'a>(out: &'a Value, path: &Path) -> &'a Value {
    let want = path.display().to_string();
    out["imported"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["path"] == want)
        .expect("the file is not in the imported list")
}

fn detection<'a>(out: &'a Value, path: &Path) -> &'a Value {
    let want = path.display().to_string();
    out["detections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["path"] == want)
        .expect("the file has no detection")
}

fn warning_codes(out: &Value) -> Vec<&str> {
    out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect()
}

/// Faces removed by the cleanup of the import (degenerate plus duplicate).
fn removed(out: &Value) -> usize {
    out["cleanup"]["degenerate"].as_u64().unwrap() as usize
        + out["cleanup"]["duplicate"].as_u64().unwrap() as usize
}

#[test]
fn counts_match_the_plugin_and_the_unit_factor_is_one() {
    let Some(real) = scans() else { return };
    for r in &real {
        let n = Neutral::new(&format!("counts-{}", r.letter), &real);
        let path = n.path(r.letter);
        let t = Instant::now();
        let out = import(&[&path]);
        eprintln!(
            "scan_{}: imported in {:.2} s",
            r.letter,
            t.elapsed().as_secs_f64()
        );
        let s = imported(&out, &path);
        let gone = removed(&out);
        let l = r.letter;
        // the messages name the letter only, never the count
        let vertices = s["vertices"].as_u64().unwrap() as usize;
        assert!(near(vertices, r.vertices), "scan_{l}: vertices");
        // the plugin count is the file's: what stays plus what the cleanup removed
        let faces = s["faces"].as_u64().unwrap() as usize;
        assert!(near(faces + gone, r.faces), "scan_{l}: faces plus removed");
        assert_eq!(s["source_units"], "mm", "scan_{l}: unit factor 1 (mm)");
        // a clean scanner export loses no face; anything else is a finding
        assert_eq!(gone, 0, "scan_{l}: the cleanup removed {gone} faces");
    }
}

#[test]
fn the_file_name_decides_the_role_of_the_original_names() {
    let Some(real) = scans() else { return };
    for r in &real {
        // the original name, not a symlink: the name carries the hint
        let out = import(&[r.path.as_path()]);
        let s = imported(&out, &r.path);
        let l = r.letter;
        assert_eq!(s["role"], r.arch, "scan_{l}: role from the file name");
        assert_eq!(s["role_source"], "file_name", "scan_{l}");
        // a name that decides does not look at the geometry
        assert_eq!(out["detections"], json!([]), "scan_{l}: no detection");
        assert!(
            warning_codes(&out).is_empty(),
            "scan_{l}: {:?}",
            warning_codes(&out)
        );
    }
}

#[test]
fn detection_on_neutral_names_finds_each_arch() {
    let Some(real) = scans() else { return };
    for r in &real {
        let n = Neutral::new(&format!("detect-{}", r.letter), &real);
        let path = n.path(r.letter);
        let out = import(&[&path]);
        let l = r.letter;
        let d = detection(&out, &path);
        assert_eq!(d["arch"], r.arch, "scan_{l}: detected arch");
        assert_eq!(d["source"], r.source, "scan_{l}: deciding cue");
        let c = d["confidence"].as_f64().unwrap();
        assert!(
            (c - r.confidence).abs() <= 0.01,
            "scan_{l}: confidence {c:.3}, expected {:.2}",
            r.confidence
        );
        let s = imported(&out, &path);
        assert_eq!(s["role"], r.arch, "scan_{l}: role");
        assert_eq!(s["role_source"], "detected", "scan_{l}");
        // a cue the plugin trusts: no `detected_unsure`, no warning to check it
        let to_check = out["warnings"].as_array().unwrap().iter();
        let to_check = to_check.filter(|w| w["key"] == "aligners-arch-warn-to-be-checked");
        assert_eq!(to_check.count(), 0, "scan_{l}");
    }
}

/// An opposite-arch pair (lower `a`, upper `b`): both detected, in either
/// order, without a conflict. The two big scans: the slowest import here.
#[test]
fn a_neutral_pair_of_opposite_arches_is_settled_in_both_orders() {
    let Some(real) = scans() else { return };
    let n = Neutral::new("pair-ab", &real);
    let (a, b) = (n.path('a'), n.path('b'));
    for (first, second) in [(&a, &b), (&b, &a)] {
        let t = Instant::now();
        let out = import(&[first, second]);
        eprintln!("pair a/b: {:.2} s", t.elapsed().as_secs_f64());
        assert_eq!(imported(&out, &a)["role"], "lower", "scan_a");
        assert_eq!(imported(&out, &b)["role"], "upper", "scan_b");
        assert_eq!(imported(&out, &a)["role_source"], "detected", "scan_a");
        assert_eq!(imported(&out, &b)["role_source"], "detected", "scan_b");
        assert!(warning_codes(&out).is_empty(), "{:?}", warning_codes(&out));
    }
}

/// The same pair on the smaller scans (lower `c`, upper `d`).
#[test]
fn a_neutral_pair_of_the_small_scans_is_settled_in_both_orders() {
    let Some(real) = scans() else { return };
    let n = Neutral::new("pair-cd", &real);
    let (c, d) = (n.path('c'), n.path('d'));
    for (first, second) in [(&c, &d), (&d, &c)] {
        let t = Instant::now();
        let out = import(&[first, second]);
        eprintln!("pair c/d: {:.2} s", t.elapsed().as_secs_f64());
        assert_eq!(imported(&out, &c)["role"], "lower", "scan_c");
        assert_eq!(imported(&out, &d)["role"], "upper", "scan_d");
        assert_eq!(imported(&out, &c)["role_source"], "detected", "scan_c");
        assert_eq!(imported(&out, &d)["role_source"], "detected", "scan_d");
        assert!(warning_codes(&out).is_empty(), "{:?}", warning_codes(&out));
    }
}

/// Two lower arches, `a` (confidence 0.80) and `c` (0.90): the surer one
/// keeps Lower, the other takes the free Upper as a provisional scan, with
/// the arch conflict warning, whatever the order of the files.
#[test]
fn two_scans_of_one_arch_give_the_surer_the_arch_and_a_conflict() {
    let Some(real) = scans() else { return };
    let n = Neutral::new("same-arch", &real);
    let (a, c) = (n.path('a'), n.path('c'));
    for (first, second) in [(&a, &c), (&c, &a)] {
        let out = import(&[first, second]);
        let (sa, sc) = (imported(&out, &a), imported(&out, &c));
        assert_eq!(sc["role"], "lower", "scan_c wins Lower");
        assert_eq!(sc["role_source"], "detected", "scan_c");
        assert_eq!(sa["role"], "upper", "scan_a goes to the free arch");
        assert_eq!(sa["role_source"], "provisional", "scan_a");
        assert_eq!(warning_codes(&out), ["arch_conflict"]);
        // both detections say Lower: the report keeps what the geometry said
        assert_eq!(detection(&out, &a)["arch"], "lower", "scan_a detection");
        assert_eq!(detection(&out, &c)["arch"], "lower", "scan_c detection");
    }
}

/// The import closes each scan with a base, and the steps that read the
/// open border (finding the teeth, the separation, the frame, the 3D teeth)
/// read the start of the closed scan back: it must be exactly the file as
/// `dc_mesh::io::read` gives it, which is what the plugin oracles on the
/// real scans (`dca-algo/tests/autoseed_real.rs`, `segmentation_real.rs`)
/// feed the algorithms. So those oracles hold for the module path too.
#[test]
fn the_closed_scan_starts_with_the_file_as_the_real_scan_oracles_read_it() {
    let Some(real) = scans() else { return };
    for r in &real {
        let l = r.letter;
        let n = Neutral::new(&format!("closed-{l}"), &real);
        let path = n.path(l);
        let t = Instant::now();
        let (state, out) = commands::find(IMPORT_SCANS)
            .unwrap()
            .execute(
                &CaseState::default(),
                &json!({ "files": [{ "path": path }] }),
                &JobCtx::detached(),
            )
            .unwrap_or_else(|_| panic!("scan_{l}: the import failed"));
        let secs = t.elapsed().as_secs_f64();
        let base = &out["bases"][0];
        assert_eq!(base["already_closed"], false, "scan_{l}: open as read");
        assert_eq!(base["watertight"], true, "scan_{l}: closed solid");
        let scan = &state.case.scans[0];
        let closed = state.blobs.require(&scan.mesh).unwrap();
        assert!(closed.validate().is_watertight(), "scan_{l}: watertight");
        let data = state
            .module_data::<dca_model::AlignersState>(dca_model::MODULE_ID)
            .unwrap();
        let record = data.scan_base(&scan.mesh).expect("a base record");
        assert_eq!(record.closed_by, dca_model::ClosedBy::Import, "scan_{l}");
        let open = dca_algo::scan_base::open_part(closed, &record).expect("the scan as read");
        let file = dc_mesh::io::read(&path).unwrap_or_else(|_| panic!("scan_{l}: unreadable"));
        assert!(open.positions == file.positions, "scan_{l}: points");
        assert!(open.tris == file.tris, "scan_{l}: triangles");
        // counts and time, never positions
        eprintln!(
            "scan_{l}: imported and closed in {secs:.2} s; base: {} border edges, {} points and {} triangles added",
            base["closed_edges"], base["vertices_added"], base["faces_added"]
        );
    }
}
