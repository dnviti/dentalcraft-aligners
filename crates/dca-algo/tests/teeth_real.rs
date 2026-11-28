//! The 3D teeth step (`dca_algo::teeth`, `reconstruct`, `gingiva`) on the
//! four real arch scans, compared with the Python plugin's
//! `teeth.compute_teeth`, `reconstruct.closed_tooth`, `gum_surface`,
//! `gum_weights` and `deform` on the same inputs.
//!
//! The scans are patient data: they are not in the repository, and neither
//! their file names nor any coordinate may appear in committed code or in
//! test output (see `dca-module/tests/import_real.rs`). The folder comes from
//! `DC_REAL_SCANS`; the files are sorted by name and called `a` to `d`
//! (lower, upper, lower, upper).
//!
//! What the plugin finds is read from the folder `DC_TEETH_REAL_ORACLE`
//! names, kept outside the repository: first `fixtures/gen_segmentation_real.py`
//! writes the plugin's segmentation there (`<letter>.json`: arch frame, final
//! vertex labels), then `fixtures/gen_teeth_real.py` runs the plugin's 3D
//! teeth from those labels and adds `<letter>_teeth.json`, the closed teeth
//! and the gingiva (read its header). Regenerate it, run, delete it:
//!
//! ```sh
//! out=$(mktemp -d /tmp/dc-teethreal-XXXX)
//! for l in a b c d; do        # one process per scan, Blender on PATH
//!   DC_REAL_SCANS=<scans> python3 -B crates/dca-algo/tests/fixtures/gen_segmentation_real.py "$out" --only $l &&
//!   DC_REAL_SCANS=<scans> python3 -B crates/dca-algo/tests/fixtures/gen_teeth_real.py "$out" --only $l &
//! done; wait                  # minutes per full-size scan
//! DC_REAL_SCANS=<scans> DC_TEETH_REAL_ORACLE="$out" cargo test --release -p dca-algo \
//!     --test teeth_real -- --nocapture --test-threads=1
//! rm -r "$out"
//! ```
//!
//! Without `DC_REAL_SCANS` or `DC_TEETH_REAL_ORACLE` the tests are skipped
//! with a line on stderr. Run them in release and one at a time for
//! meaningful timings. The messages carry tooth numbers, counts, distances
//! and seconds, never positions.
//!
//! The step is isolated: both sides start from the plugin's labels and arch
//! frame (the scan moved to arch coordinates by its inverse), the closed
//! teeth from the plugin's long axes, the gingiva weights from the plugin's
//! sequence, the deformation from the same two test motions: `mild`
//! (`gen_gingiva.py`'s: the two central teeth of the sequence moved and
//! turned 6 degrees, the first tooth moved 0.5 mm) and `strong`
//! (`gen_teeth_real.py`'s: 15 degrees and 2 mm, every third tooth 2 mm
//! inwards). Every check of a scan runs and the failures are reported
//! together. Checked per scan:
//!
//! * the teeth: the same sequence; per tooth `md_sign`, margin, vertex and
//!   face counts equal, axes within [`TOL_DEG`], widths and points within
//!   [`TOL_MM`], the rest frame within [`TOL_FRAME`] (the bounds of
//!   `teeth_oracle.rs`);
//! * every closed tooth: all vertices finite, watertight and manifold, one
//!   piece without a tunnel through it, no closed tooth missing; a tooth
//!   within [`HAUSDORFF_MM`] (symmetric surface Hausdorff distance to the
//!   plugin's solid) has every number of the plugin's info dict equal; a
//!   tooth above it is a rounding divergence, printed with its info
//!   differences and counted per scan, and fails only above
//!   [`HAUSDORFF_DIVERGED_MM`]; a tooth whose plugin solid has a tunnel
//!   through it is a departure (its handle cut here), printed, with no more
//!   crossing faces than the plugin's;
//! * the gingiva: every vertex finite, the vertex and face counts, the tooth
//!   and flags of every vertex equal, every rest and deformed position within
//!   [`TOL_POS_MM`], every weight within [`TOL_W`] (column sums printed);
//! * the deformation quality on real data, measured with the plugin's
//!   formulas on both gingivas (`tests/archive/test_gingiva_real.py`: flipped
//!   triangles of the whole and of the shown gingiva, longest edge over its
//!   rest length; `tests/archive/dev/dev_gum_quality.py`: folded triangles and
//!   edge stretch of the visible and hidden parts), per motion. The Rust
//!   measure of the plugin's gingiva equals the numbers the plugin script
//!   measured (within [`METRIC_TOL`]); the port's equals the plugin's
//!   (counts equal, stretch within [`STRETCH_TOL`]) and stays within
//!   `test_gingiva_real.py`'s bounds ([`FLIP_SHARE`], [`FLIP_SHARE_SHOWN`],
//!   [`STRETCH_MAX`]) wherever the plugin's own gingiva does (the bounds are
//!   the plugin's for its real staging, not for an arbitrary test motion);
//! * the closed teeth against the gingiva (`test_gingiva_real.py`'s "seen":
//!   closed-tooth surface more than [`DEEP_MM`] below its cervical line seen
//!   from outside, from above and 12 azimuths 30 degrees up, at rest and
//!   under each motion), measured on both sides with the same code: the
//!   port within [`SEEN_TOL_MM2`] of the plugin, and within [`SEEN_RATIO`]
//!   of rest with no tooth more than [`SEEN_TOOTH_MM2`] above its rest value
//!   (the plugin's bounds) wherever the plugin is. The shell metrics of
//!   `test_gingiva_shell.py` (`inner_mm2`, minimum distance) need an aligner
//!   shell, which this step does not make.
//!
//! Measured on 2026-11-12 (plugin 42e6d87; thresholds and reasons at the
//! constants). Every closed tooth is finite on both sides and watertight;
//! no tooth gave non-finite vertices.
//!
//! * scan b (14 teeth, 821278 gingiva faces): **fails**. Teeth within
//!   1.2e-6 deg; 12 closed teeth with the plugin's numbers and within
//!   6.4e-6 mm, but tooth 24 differs (contact-filled 2013 vertices against
//!   2026, despiked 1709 against 1703, sharp left 53 against 48, Hausdorff
//!   3.4e-2 mm) and tooth 26 lies 7.0e-4 mm from the plugin's with the same
//!   numbers. Known divergence, found 2026-11-12: it predates that day's
//!   edits (the same on the committed 68bb16b), not a non-finite case, and
//!   the bound stays: these are real failures until the port is fixed.
//!   Gingiva positions within 4.4e-5 mm, weights within 8.2e-6; mild motion
//!   14 flipped (2 shown), 1.57 x; strong 639 flipped (527 shown), 2.71 x;
//!   seen 76.4 -> 76.6 (mild) and 92.5 mm2 (strong; tooth 26 +7.4 mm2).
//! * scan c (12 teeth): passes. Hausdorff at most 9.1e-7 mm (tooth 38: 116
//!   crossing faces on both sides); gingiva 112322 vertices, positions within
//!   1e-5 mm, weights within 6e-6; mild motion 0 flipped, 1.29 x; strong 114
//!   of 220800 flipped (108 shown), 2.46 x; seen 58.1 -> 59.2 (mild) and
//!   78.5 mm2 (strong; tooth 45 +10.6 mm2).
//! * scan d (15 teeth): passes. Teeth within 1.2e-6 deg and 2e-11 mm;
//!   Hausdorff at most 2.8e-6 mm; gingiva 103703 vertices, positions within
//!   1e-5 mm, weights within 1e-6; mild motion 0 flipped, 1.28 x; strong 77 of
//!   204972 flipped (76 shown), 1.95 x; seen 48.5 -> 48.3 (mild) and
//!   80.6 mm2 (strong; tooth 12 +8.5 mm2).
//!
//! Port and plugin agree on every quality number; under the strong motion
//! the seen area grows past [`SEEN_RATIO`] on both, so there only the
//! per-tooth bound applies. Port, loaded machine: 1.4 s (d) to 21 s (b) for the
//! closed teeth, 0.5-8 s for the surface and the weights. The plugin's
//! oracle (Python, exact float64 `mathutils` shim) takes minutes per tooth:
//! 53-312 s on c and d, 263-2069 s on b, 2629 s for scan c tooth 38 (four
//! closing axes, each with its crossing checks).

mod golden_case;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_mesh::{TriMesh, ValidationReport};
use dca_algo::anatomy::Arch;
use dca_algo::frame::rigid_inverse;
use dca_algo::gingiva::{self, FLAG_COL, FLAG_MARGIN, FLAG_MEMBRANE, GumSurface, TOOTH_MASK};
use dca_algo::reconstruct;
use dca_algo::teeth::{self, ToothGeom, percentile};
use glam::{DMat4, DVec3, Vec3};
use golden_case::{mat4, transform, unpack_labels, vec3};
use rayon::prelude::*;
use serde_json::Value;

/// Tooth axes, degrees (`teeth_oracle.rs`).
const TOL_DEG: f64 = 0.05;
/// Tooth widths and points, mm (`teeth_oracle.rs`).
const TOL_MM: f64 = 0.01;
/// Entries of the rest frame (`teeth_oracle.rs`).
const TOL_FRAME: f64 = 1e-3;
/// Closed tooth against the plugin's, symmetric surface Hausdorff, mm. The
/// demo cases give 6e-6 (`reconstruct_oracle.rs`, the f32 rounding of the
/// fixtures); the real scans are larger (coordinates up to about 40 mm, f32
/// steps of 4e-6): the same bound as `reconstruct_oracle.rs`.
const HAUSDORFF_MM: f64 = 1e-4;
/// Largest Hausdorff a diverged tooth may reach, mm: clinically negligible,
/// below the order of scanner accuracy. On real scans `close_crown` is
/// chaotic in rounding (ring vertices differ by 1e-14 mm between the
/// eigen and SVD plane normals, another Jacobi preconditioner, CG at 1e-6,
/// then one crossing pair flips in the untangle), so a tooth can end up
/// above [`HAUSDORFF_MM`] with other info counts, though every stage
/// reproduces the other side exactly when fed the same rings.
const HAUSDORFF_DIVERGED_MM: f64 = 0.1;
/// Gingiva rest and deformed positions, mm (`gingiva_oracle.rs`).
const TOL_POS_MM: f64 = 1e-4;
/// Gingiva weights (`gingiva_oracle.rs`).
const TOL_W: f64 = 1e-4;
/// `test_gingiva_real.py`: flipped triangles of the whole gingiva, share.
const FLIP_SHARE: f64 = 5e-4;
/// ... and of the gingiva shown and printed.
const FLIP_SHARE_SHOWN: f64 = 1.5e-3;
/// ... longest edge over its rest length.
const STRETCH_MAX: f64 = 3.0;
/// The Rust measure of the plugin's gingiva against the plugin script's
/// numbers (the same positions and formulas: rounding only), relative.
const METRIC_TOL: f64 = 1e-9;
/// The port's edge stretch against the plugin's, relative: positions agree
/// within [`TOL_POS_MM`] (measured 1e-5 mm), which moves the ratio of a
/// 0.1 mm edge by up to 2e-3 at the bound (measured 5e-7). Triangle counts
/// (flipped, folded, above 3x) must be equal.
const STRETCH_TOL: f64 = 2e-3;
/// `test_gingiva_real.py`: closed tooth this far below its cervical line.
const DEEP_MM: f64 = 1.0;
/// ... seen under the motion at most this times at rest ...
const SEEN_RATIO: f64 = 1.25;
/// ... and no tooth more than this above its rest value, mm2.
const SEEN_TOOTH_MM2: f64 = 12.0;
/// Seen area of the port against the plugin's, mm2 per scan (the same code
/// on both: what differs is the geometry).
const SEEN_TOL_MM2: f64 = 0.05;

/// The test motions of `gen_teeth_real.py`.
const MOTIONS: [&str; 2] = ["mild", "strong"];

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
    let Some(dir) = std::env::var_os("DC_TEETH_REAL_ORACLE").map(PathBuf::from) else {
        eprintln!("skipped: DC_TEETH_REAL_ORACLE is not set");
        return None;
    };
    Some(dir)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &std::fs::read(path).unwrap_or_else(|_| panic!("missing {}", path.display())),
    )
    .unwrap()
}

fn read_raw(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|_| panic!("missing {}", path.display()))
}

fn raw_points(path: &Path) -> Vec<DVec3> {
    read_raw(path)
        .as_chunks::<24>()
        .0
        .iter()
        .map(|b| {
            let f = |k: usize| f64::from_le_bytes(b[8 * k..8 * k + 8].try_into().unwrap());
            DVec3::new(f(0), f(1), f(2))
        })
        .collect()
}

fn raw_tris(path: &Path) -> Vec<[u32; 3]> {
    read_raw(path)
        .as_chunks::<12>()
        .0
        .iter()
        .map(|b| {
            std::array::from_fn(|k| u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
        })
        .collect()
}

fn raw_u16(path: &Path) -> Vec<u16> {
    read_raw(path)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect()
}

fn raw_f32(path: &Path) -> Vec<f32> {
    read_raw(path)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn count(v: &Value) -> usize {
    v.as_u64().unwrap_or_else(|| panic!("not a count: {v}")) as usize
}

fn fdi(f: u64) -> Fdi {
    Fdi::new(f as u8).unwrap()
}

fn trimesh(v: &[DVec3], t: &[[u32; 3]]) -> TriMesh {
    TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t.to_vec())
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

/// The failures of a scan, reported together at the end.
struct Report {
    letter: char,
    fails: Vec<String>,
}

impl Report {
    fn check(&mut self, ok: bool, msg: impl FnOnce() -> String) {
        if !ok {
            let m = format!("scan {}: {}", self.letter, msg());
            eprintln!("FAIL {m}");
            self.fails.push(m);
        }
    }
}

/// One scan in the plugin's arch frame, with its labels and the plugin's
/// output of the step.
struct Scan {
    letter: char,
    arch: Arch,
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    labels: Vec<u16>,
    oracle: Value,
    dir: PathBuf,
}

fn load(letter: char, path: &Path, dir: &Path) -> Scan {
    let seg = read_json(&dir.join(format!("{letter}.json")));
    let oracle = read_json(&dir.join(format!("{letter}_teeth.json")));
    let arch = match oracle["arch"].as_str() {
        Some("UPPER") => Arch::Upper,
        Some("LOWER") => Arch::Lower,
        other => panic!("scan {letter}: the oracle has no arch ({other:?})"),
    };
    let mesh = dc_mesh::io::read(path).unwrap_or_else(|_| panic!("scan {letter}: unreadable"));
    let world: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    assert_eq!(
        (seg["vertices"].as_u64(), seg["faces"].as_u64()),
        (Some(world.len() as u64), Some(mesh.tris.len() as u64)),
        "scan {letter}: the plugin saw another mesh"
    );
    let (fp, scale) = fingerprint(&world);
    let want = &seg["fingerprint"];
    assert!(
        (fp - num(&want[0])).abs() <= 1e-9 * scale.max(num(&want[1])),
        "scan {letter}: the plugin numbered the vertices differently"
    );
    let to_arch = rigid_inverse(&mat4(&seg["frame"]));
    let labels = unpack_labels(&seg["vertex_labels"]);
    assert_eq!(labels.len(), world.len(), "scan {letter}: labels");
    Scan {
        letter,
        arch,
        vertices: world.iter().map(|&p| transform(&to_arch, p)).collect(),
        tris: mesh.tris,
        labels,
        oracle,
        dir: dir.to_path_buf(),
    }
}

/// The plugin's sequence.
fn sequence(s: &Scan) -> Vec<Fdi> {
    s.oracle["sequence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| fdi(f.as_u64().unwrap()))
        .collect()
}

/// The teeth against `compute_teeth`; returns the port's teeth.
fn check_teeth(s: &Scan, r: &mut Report) -> BTreeMap<Fdi, ToothGeom> {
    let t0 = Instant::now();
    let out = teeth::run(
        &teeth::Input {
            arch: s.arch,
            vertices: &s.vertices,
            tris: &s.tris,
            labels: &s.labels,
        },
        &teeth::Params::default(),
    )
    .unwrap_or_else(|e| panic!("scan {}: teeth: {e}", s.letter));
    let secs = t0.elapsed().as_secs_f64();
    let seq = sequence(s);
    r.check(out.sequence == seq, || {
        format!("sequence {:?}, plugin {seq:?}", out.sequence)
    });
    let (mut worst_deg, mut worst_mm, mut worst_frame) = (0.0f64, 0.0f64, 0.0f64);
    for want in s.oracle["teeth"].as_array().unwrap() {
        let f = fdi(want["fdi"].as_u64().unwrap());
        let Some(t) = out.teeth.get(&f) else {
            r.check(false, || format!("tooth {f}: not found by the port"));
            continue;
        };
        for (got, key) in [
            (i64::from(t.md_sign), "md_sign"),
            (t.margin.len() as i64, "n_margin"),
            (t.vertex_count as i64, "n_vertices"),
            (t.face_count as i64, "n_faces"),
        ] {
            let w = want[key].as_i64().unwrap();
            r.check(got == w, || format!("tooth {f} {key}: {got}, plugin {w}"));
        }
        for (got, key) in [
            (t.e_md, "e_md"),
            (t.e_bl, "e_bl"),
            (t.e_ax, "e_ax"),
            (t.mesial_dir, "mesial_dir"),
            (t.out_dir, "out_dir"),
        ] {
            let d = got
                .dot(vec3(&want[key]))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            worst_deg = worst_deg.max(d);
            r.check(d <= TOL_DEG, || format!("tooth {f} {key}: {d:.4} deg off"));
        }
        for (got, key) in [(t.axis_tilt_deg, "axis_tilt"), (t.rot_dev_deg, "rot_dev")] {
            let d = (got - num(&want[key])).abs();
            worst_deg = worst_deg.max(d);
            r.check(d <= TOL_DEG, || format!("tooth {f} {key}: {d:.4} deg off"));
        }
        for (got, key) in [
            (t.width_md, "width_md"),
            (t.width_bl, "width_bl"),
            (t.height, "height"),
            (t.occ_z, "occ_z"),
            (t.zen_b, "zen_b"),
            (t.zen_l, "zen_l"),
            (t.top_elong, "top_elong"),
            (t.top_spread, "top_spread"),
        ] {
            let d = (got - num(&want[key])).abs();
            worst_mm = worst_mm.max(d);
            r.check(d <= TOL_MM, || format!("tooth {f} {key}: {d:.4} mm off"));
        }
        for (got, key) in [
            (t.centroid, "centroid"),
            (t.reference, "ref"),
            (t.tip, "tip"),
            (t.margin_mean, "margin_mean"),
            (t.cej, "cej"),
            (t.crest, "crest"),
            (t.apex, "apex"),
            (t.cres, "cres"),
            (t.contact_m, "contact_m"),
            (t.contact_d, "contact_d"),
        ] {
            let d = (got - vec3(&want[key])).length();
            worst_mm = worst_mm.max(d);
            r.check(d <= TOL_MM, || format!("tooth {f} {key}: {d:.4} mm off"));
        }
        let (got, m) = (t.rest_frame(), mat4(&want["frame"]));
        let d = (0..4)
            .flat_map(|c| (0..4).map(move |k| (c, k)))
            .map(|(c, k)| (got.col(c)[k] - m.col(c)[k]).abs())
            .fold(0.0, f64::max);
        worst_frame = worst_frame.max(d);
        r.check(d <= TOL_FRAME, || {
            format!("tooth {f} rest frame: {d:.2e} off")
        });
    }
    eprintln!(
        "scan {}: teeth: {} teeth, worst {worst_deg:.2e} deg, {worst_mm:.2e} mm, frame {worst_frame:.2e}; \
         port {secs:.2} s, plugin {} s",
        s.letter,
        out.sequence.len(),
        s.oracle["seconds"]["compute_teeth"],
    );
    out.teeth
}

/// The closed teeth against `closed_tooth`, from the plugin's long axes;
/// returns the port's solids.
fn check_closed(s: &Scan, r: &mut Report) -> BTreeMap<Fdi, reconstruct::ClosedTooth> {
    let axes: BTreeMap<Fdi, DVec3> = s.oracle["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (fdi(t["fdi"].as_u64().unwrap()), vec3(&t["e_ax"])))
        .collect();
    let t0 = Instant::now();
    let out = reconstruct::run(
        &reconstruct::Input {
            vertices: &s.vertices,
            tris: &s.tris,
            labels: &s.labels,
            axes: &axes,
        },
        &reconstruct::Params::default(),
    )
    .unwrap_or_else(|e| panic!("scan {}: closed teeth: {e}", s.letter));
    let secs = t0.elapsed().as_secs_f64();
    let mut hd_all = Vec::new();
    for (f, tooth) in &out.teeth {
        let want = &s.oracle["closed"][f.to_string()];
        let plugin_finite = want["finite"].as_bool().unwrap();
        let bad = tooth.vertices.iter().filter(|p| !p.is_finite()).count();
        r.check(bad == 0, || {
            format!(
                "closed tooth {f}: {bad} of {} vertices not finite (plugin's {})",
                tooth.vertices.len(),
                if plugin_finite {
                    "all finite"
                } else {
                    "not finite either"
                }
            )
        });
        let m = trimesh(&tooth.vertices, &tooth.tris);
        let rep = ValidationReport::of(&m);
        r.check(rep.is_watertight() && rep.is_manifold(), || {
            format!("closed tooth {f}: not a closed solid: {rep:?}")
        });
        let solid = dc_mesh::solid_topology(&m);
        r.check(solid.pieces == 1 && solid.genus == Some(0), || {
            format!("closed tooth {f}: not one solid without a tunnel: {solid:?}")
        });
        let oracle = dc_mesh::dcm::decode(&read_raw(
            &s.dir.join(format!("{}_closed_{f}.dcm", s.letter)),
        ))
        .unwrap();
        let hd = if bad == 0 && plugin_finite {
            let o = TriMesh::new(oracle.positions.clone(), oracle.tris.clone());
            dc_geom::metrics::hausdorff_surface(&m, &o).unwrap_or(f64::INFINITY)
        } else {
            f64::NAN
        };
        let departed = dc_mesh::solid_topology(&oracle)
            .genus
            .is_some_and(|g| g > 0);
        let rust = serde_json::to_value(tooth.info).unwrap();
        let info_diffs: Vec<String> = want["info"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(k, v)| &rust[k.as_str()] != *v)
            .map(|(k, v)| format!("{k} {} (plugin {v})", rust[k.as_str()]))
            .collect();
        if departed {
            // a plugin's solid with a tunnel through it (a handle of the
            // scanned crown, which the plugin keeps): ours has the handle cut
            // away (`reconstruct/handles.rs`), so neither its numbers nor its
            // surface are the plugin's (`reconstruct_oracle.rs`); a handle
            // cut, and no more crossing faces
            eprintln!(
                "scan {} tooth {f}: DEPARTED, the plugin's solid has a tunnel through it; Hausdorff \
                 {hd:.2e} mm; info differences: {}",
                s.letter,
                info_diffs.join(", ")
            );
            let theirs = want["info"]["crossing_faces"].as_u64().unwrap() as usize;
            r.check(tooth.info.handles_cut > 0 && tooth.info.crossing_faces <= theirs, || {
                format!(
                    "closed tooth {f}: the plugin's tunnel is not cut or more faces cross: {:?}",
                    tooth.info
                )
            });
        } else if hd > HAUSDORFF_MM {
            eprintln!(
                "scan {} tooth {f}: DIVERGED by rounding, Hausdorff {hd:.2e} mm; info differences: {}",
                s.letter,
                if info_diffs.is_empty() {
                    "none".to_string()
                } else {
                    info_diffs.join(", ")
                }
            );
        } else {
            r.check(info_diffs.is_empty(), || {
                format!("closed tooth {f}: info differs: {}", info_diffs.join(", "))
            });
        }
        if !hd.is_nan() && !departed {
            hd_all.push(hd);
            r.check(hd <= HAUSDORFF_DIVERGED_MM, || {
                format!("closed tooth {f}: Hausdorff {hd:.2e} mm to the plugin's (limit {HAUSDORFF_DIVERGED_MM} mm)")
            });
        }
        eprintln!(
            "scan {} tooth {f}: hausdorff {hd:.2e} mm, vertices {} (plugin {}), crossings {} (plugin {}), \
             axis {} (plugin {}), plugin {:.1} s",
            s.letter,
            tooth.vertices.len(),
            want["n_vertices"],
            tooth.info.crossing_faces,
            want["info"]["crossing_faces"],
            tooth.info.closing_axis,
            want["info"]["closing_axis"],
            num(&want["seconds"]),
        );
    }
    let missing: Vec<_> = axes.keys().filter(|f| !out.teeth.contains_key(f)).collect();
    r.check(missing.is_empty(), || {
        format!("closed teeth missing: {missing:?}")
    });
    hd_all.sort_by(f64::total_cmp);
    let diverged = hd_all.iter().filter(|h| **h > HAUSDORFF_MM).count();
    eprintln!(
        "scan {}: closed teeth: {} teeth, {diverged} diverged (> {HAUSDORFF_MM:.0e} mm), Hausdorff median \
         {:.2e} mm, worst {:.2e} mm; port {secs:.1} s (parallel), plugin {} s ({} s summed over the teeth)",
        s.letter,
        out.teeth.len(),
        hd_all.get(hd_all.len() / 2).copied().unwrap_or(f64::NAN),
        hd_all.last().copied().unwrap_or(f64::NAN),
        s.oracle["seconds"]["closed_teeth_wall"],
        s.oracle["closed"]
            .as_object()
            .unwrap()
            .values()
            .map(|c| num(&c["seconds"]))
            .sum::<f64>()
            .round(),
    );
    out.teeth
}

/// A gingiva, rest and deformed, with its flags.
struct Gum {
    rest: Vec<DVec3>,
    moved: BTreeMap<&'static str, Vec<DVec3>>,
    tris: Vec<[u32; 3]>,
    tooth: Vec<u16>,
    margin: Vec<bool>,
    membrane: Vec<bool>,
    col: Vec<bool>,
}

impl Gum {
    fn of(g: &GumSurface, moved: BTreeMap<&'static str, Vec<DVec3>>) -> Self {
        Self {
            rest: g.vertices.clone(),
            moved,
            tris: g.tris.clone(),
            tooth: g.tooth.clone(),
            margin: g.margin.clone(),
            membrane: g.membrane.clone(),
            col: g.col.clone(),
        }
    }

    fn flags(&self, i: usize) -> u16 {
        (self.tooth[i] & TOOTH_MASK)
            | if self.margin[i] { FLAG_MARGIN } else { 0 }
            | if self.membrane[i] { FLAG_MEMBRANE } else { 0 }
            | if self.col[i] { FLAG_COL } else { 0 }
    }

    fn plugin(s: &Scan) -> Self {
        let p = |what: &str| s.dir.join(format!("{}_gum_{what}", s.letter));
        let flags = raw_u16(&p("flags.u16"));
        Self {
            rest: raw_points(&p("rest.f64")),
            moved: MOTIONS
                .iter()
                .map(|&m| (m, raw_points(&p(&format!("{m}.f64")))))
                .collect(),
            tris: raw_tris(&p("tris.u32")),
            tooth: flags.iter().map(|&x| x & TOOTH_MASK).collect(),
            margin: flags.iter().map(|&x| x & FLAG_MARGIN != 0).collect(),
            membrane: flags.iter().map(|&x| x & FLAG_MEMBRANE != 0).collect(),
            col: flags.iter().map(|&x| x & FLAG_COL != 0).collect(),
        }
    }

    /// Triangles shown and printed: not all corners on the membrane under a
    /// crown (the col aside), as `test_gingiva_real.py`.
    fn shown(&self) -> Vec<bool> {
        self.tris
            .iter()
            .map(|t| {
                !t.iter()
                    .all(|&i| self.membrane[i as usize] && !self.col[i as usize])
            })
            .collect()
    }
}

/// `test_gingiva_real.py` `tri_stats` and its row.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quality {
    flipped: usize,
    shown: usize,
    flipped_shown: usize,
    stretch_max: f64,
    stretch_gt3: usize,
}

/// `dev_gum_quality.py` `quality` of one part.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DevQuality {
    triangles: usize,
    folded: usize,
    p999: f64,
    max: f64,
    gt3: usize,
}

/// Closed teeth of one side: rest frame, vertices (arch coordinates) and
/// triangles per tooth.
type Solids = BTreeMap<Fdi, (DMat4, Vec<DVec3>, Vec<[u32; 3]>)>;

fn cross(v: &[DVec3], t: &[u32; 3]) -> DVec3 {
    let a = v[t[0] as usize];
    (v[t[1] as usize] - a).cross(v[t[2] as usize] - a)
}

fn edges(v: &[DVec3], t: &[u32; 3]) -> [f64; 3] {
    std::array::from_fn(|k| (v[t[k] as usize] - v[t[(k + 1) % 3] as usize]).length())
}

fn quality(g: &Gum, moved: &[DVec3]) -> Quality {
    let shown = g.shown();
    let mut q = Quality {
        flipped: 0,
        shown: shown.iter().filter(|&&x| x).count(),
        flipped_shown: 0,
        stretch_max: 0.0,
        stretch_gt3: 0,
    };
    for (t, &sh) in g.tris.iter().zip(&shown) {
        let (n1, n0) = (cross(moved, t), cross(&g.rest, t));
        let (a1, a0) = (n1.length(), n0.length());
        let cos = n1.dot(n0) / (a1 * a0).max(1e-12);
        if cos < 0.0 && a0 > 1e-8 {
            q.flipped += 1;
            q.flipped_shown += usize::from(sh);
        }
        let longest = |e: [f64; 3]| e[0].max(e[1]).max(e[2]);
        let st = longest(edges(moved, t)) / longest(edges(&g.rest, t)).max(1e-6);
        q.stretch_max = q.stretch_max.max(st);
        q.stretch_gt3 += usize::from(st > 3.0);
    }
    q
}

/// `dev_gum_quality.py`: (visible, hidden) where visible means not all
/// corners on the membrane.
fn quality_dev(g: &Gum, moved: &[DVec3]) -> [DevQuality; 2] {
    let vis: Vec<bool> = g
        .tris
        .iter()
        .map(|t| !t.iter().all(|&i| g.membrane[i as usize]))
        .collect();
    [true, false].map(|part| {
        let mut st = Vec::new();
        let mut folded = 0;
        for (t, _) in g.tris.iter().zip(&vis).filter(|(_, v)| **v == part) {
            let (n0, n1) = (cross(&g.rest, t), cross(moved, t));
            let a0 = 0.5 * n0.length();
            let unit = |n: DVec3| {
                let l = n.length();
                n / if l > 0.0 { l } else { 1.0 }
            };
            if unit(n0).dot(unit(n1)) < 0.0 && a0 > 1e-8 {
                folded += 1;
            }
            let (e0, e1) = (edges(&g.rest, t), edges(moved, t));
            st.push(
                (0..3)
                    .map(|k| e1[k] / e0[k].max(1e-6))
                    .fold(f64::MIN, f64::max),
            );
        }
        DevQuality {
            triangles: st.len(),
            folded,
            p999: if st.is_empty() {
                0.0
            } else {
                percentile(&st, 99.9)
            },
            max: st.iter().copied().fold(0.0, f64::max),
            gt3: st.iter().filter(|&&x| x > 3.0).count(),
        }
    })
}

/// View directions of `test_gingiva_real.py`: from above and 12 azimuths 30
/// degrees above the occlusal plane.
fn views() -> Vec<DVec3> {
    let el = 30f64.to_radians();
    std::iter::once(DVec3::Z)
        .chain((0..12).map(|k| {
            let a = 2.0 * std::f64::consts::PI * f64::from(k) / 12.0;
            DVec3::new(a.cos() * el.cos(), a.sin() * el.cos(), el.sin())
        }))
        .collect()
}

fn sector(p: DVec3) -> usize {
    (((p.y.atan2(p.x) + std::f64::consts::PI) / (2.0 * std::f64::consts::PI) * 36.0) as usize) % 36
}

/// `test_gingiva_real.py` `deep_band`: per tooth the closed-tooth vertices
/// more than [`DEEP_MM`] below the lowest margin point of their side (36
/// sectors about the long axis, tooth frame) and the vertex areas.
fn deep_band(g: &Gum, teeth: &Solids) -> BTreeMap<Fdi, (Vec<usize>, Vec<f64>)> {
    let mut out = BTreeMap::new();
    for (&f, (rest, v, t)) in teeth {
        let inv = rigid_inverse(rest);
        let fi = u16::from(f.get());
        let m: Vec<DVec3> = (0..g.rest.len())
            .filter(|&i| g.margin[i] && g.tooth[i] == fi)
            .map(|i| transform(&inv, g.rest[i]))
            .collect();
        if m.len() < 3 {
            continue;
        }
        let mut zmin = [f64::INFINITY; 36];
        for p in &m {
            let s = sector(*p);
            zmin[s] = zmin[s].min(p.z);
        }
        // np.interp(arange(36), ok, zmin[ok], period=36)
        let ok: Vec<usize> = (0..36).filter(|&i| zmin[i].is_finite()).collect();
        let z: Vec<f64> = (0..36)
            .map(|i| {
                if zmin[i].is_finite() {
                    return zmin[i];
                }
                let prev = ok
                    .iter()
                    .rev()
                    .find(|&&k| k < i)
                    .or(ok.last())
                    .copied()
                    .unwrap();
                let next = ok.iter().find(|&&k| k > i).or(ok.first()).copied().unwrap();
                let dp = (i + 36 - prev) % 36;
                let dn = (next + 36 - i) % 36;
                zmin[prev] + (zmin[next] - zmin[prev]) * dp as f64 / (dp + dn) as f64
            })
            .collect();
        let local: Vec<DVec3> = v.iter().map(|&p| transform(&inv, p)).collect();
        let mut area = vec![0.0; local.len()];
        for tri in t {
            let a = 0.5 * cross(&local, tri).length() / 3.0;
            for &i in tri {
                area[i as usize] += a;
            }
        }
        let idx = (0..local.len())
            .filter(|&i| local[i].z < z[sector(local[i])] - DEEP_MM)
            .collect();
        out.insert(f, (idx, area));
    }
    out
}

/// `test_gingiva_real.py` `seen_mm2`: per tooth the area of its deep band
/// seen from outside (a ray from the vertex along a view direction facing its
/// normal leaves the scene: the gingiva shown and every closed tooth) with
/// the teeth at `deltas` and the gingiva at `gum`.
fn seen_mm2(
    gum: &[DVec3],
    shown_tris: &[[u32; 3]],
    teeth: &Solids,
    deltas: &BTreeMap<Fdi, DMat4>,
    band: &BTreeMap<Fdi, (Vec<usize>, Vec<f64>)>,
) -> BTreeMap<Fdi, f64> {
    let mut pos: Vec<Vec3> = gum.iter().map(|p| p.as_vec3()).collect();
    let mut tris = shown_tris.to_vec();
    let mut posed = BTreeMap::new();
    for (&f, (_, v, t)) in teeth {
        let d = deltas.get(&f).copied().unwrap_or(DMat4::IDENTITY);
        let p: Vec<DVec3> = v.iter().map(|&x| transform(&d, x)).collect();
        let mut n = vec![DVec3::ZERO; p.len()];
        for tri in t {
            let c = cross(&p, tri);
            for &i in tri {
                n[i as usize] += c;
            }
        }
        let n: Vec<DVec3> = n.iter().map(|x| x.normalize_or_zero()).collect();
        let off = pos.len() as u32;
        pos.extend(p.iter().map(|x| x.as_vec3()));
        tris.extend(t.iter().map(|tri| tri.map(|i| i + off)));
        posed.insert(f, (p, n));
    }
    let bvh = Bvh::build(&TriMesh::new(pos, tris)).unwrap();
    let views = views();
    band.iter()
        .map(|(f, (idx, area))| {
            let (p, n) = &posed[f];
            let seen: f64 = idx
                .par_iter()
                .filter(|&&i| {
                    views.iter().any(|&v| {
                        n[i].dot(v) > 0.1
                            && bvh
                                .ray_cast((p[i] + 1e-3 * v).as_vec3(), v.as_vec3(), 40.0)
                                .is_none()
                    })
                })
                .map(|&i| area[i])
                .sum();
            (*f, seen)
        })
        .collect()
}

/// The shown triangles of a gingiva.
fn shown_tris(g: &Gum) -> Vec<[u32; 3]> {
    g.tris
        .iter()
        .zip(g.shown())
        .filter(|(_, s)| *s)
        .map(|(t, _)| *t)
        .collect()
}

/// The closed teeth of the plugin, with its rest frames (non-finite ones
/// left out).
fn plugin_solids(s: &Scan) -> Solids {
    s.oracle["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let f = fdi(t["fdi"].as_u64().unwrap());
            let m = dc_mesh::dcm::decode(&read_raw(
                &s.dir.join(format!("{}_closed_{f}.dcm", s.letter)),
            ))
            .unwrap();
            let v: Vec<DVec3> = m.positions.iter().map(|p| p.as_dvec3()).collect();
            (f, (mat4(&t["frame"]), v, m.tris))
        })
        .filter(|(_, (_, v, _))| v.iter().all(|p| p.is_finite()))
        .collect()
}

/// The gingiva, its weights, its deformation and the quality metrics.
fn check_gingiva(
    s: &Scan,
    port_teeth: &BTreeMap<Fdi, ToothGeom>,
    closed: &BTreeMap<Fdi, reconstruct::ClosedTooth>,
    r: &mut Report,
) {
    let l = s.letter;
    let want = &s.oracle["gum"];
    let seq = sequence(s);
    let params = gingiva::Params::default();
    let t0 = Instant::now();
    let g = gingiva::gum_surface(&s.vertices, &s.tris, &s.labels, &params)
        .unwrap_or_else(|e| panic!("scan {l}: gum surface: {e}"));
    let t_surface = t0.elapsed().as_secs_f64();
    let t0 = Instant::now();
    let w = gingiva::gum_weights(&g, &seq, &params);
    let t_weights = t0.elapsed().as_secs_f64();
    let motions: Vec<(&'static str, BTreeMap<Fdi, DMat4>)> = MOTIONS
        .iter()
        .map(|&m| {
            let d = s.oracle["motions"][m]["deltas"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, x)| (fdi(k.parse().unwrap()), mat4(x)))
                .collect();
            (m, d)
        })
        .collect();
    let t0 = Instant::now();
    let prep = gingiva::deform_prep(&w);
    let moved = motions
        .iter()
        .map(|(m, d)| (*m, gingiva::deform(&g.vertices, &prep, d)))
        .collect();
    let t_deform = t0.elapsed().as_secs_f64() / motions.len() as f64;
    eprintln!(
        "scan {l}: gingiva: port surface {t_surface:.1} s, weights {t_weights:.1} s, deform {t_deform:.2} s; \
         plugin {} / {} / {} s",
        s.oracle["seconds"]["gum_surface"],
        s.oracle["seconds"]["gum_weights"],
        s.oracle["seconds"]["deform"],
    );
    let port = Gum::of(&g, moved);
    let plugin = Gum::plugin(s);

    let bad = port
        .rest
        .iter()
        .chain(port.moved.values().flatten())
        .filter(|p| !p.is_finite())
        .count();
    r.check(bad == 0, || format!("gingiva: {bad} positions not finite"));
    for (got, key) in [
        (port.rest.len(), "n_vertices"),
        (port.tris.len(), "n_faces"),
        (port.margin.iter().filter(|&&x| x).count(), "n_margin"),
        (port.membrane.iter().filter(|&&x| x).count(), "n_membrane"),
        (port.col.iter().filter(|&&x| x).count(), "n_col"),
    ] {
        let w = count(&want[key]);
        r.check(got == w, || format!("gingiva {key}: {got}, plugin {w}"));
    }
    let same_mesh = port.rest.len() == plugin.rest.len() && port.tris == plugin.tris;
    r.check(same_mesh, || {
        "gingiva: other triangles than the plugin's".to_owned()
    });
    if same_mesh {
        let flags = (0..port.rest.len())
            .filter(|&i| port.flags(i) != plugin.flags(i))
            .count();
        r.check(flags == 0, || {
            format!("gingiva: {flags} vertices with other tooth or flags")
        });
        let worst = |a: &[DVec3], b: &[DVec3]| {
            a.iter()
                .zip(b)
                .map(|(x, y)| (*x - *y).abs().max_element())
                .fold(0.0, f64::max)
        };
        let d_rest = worst(&port.rest, &plugin.rest);
        r.check(d_rest < TOL_POS_MM, || {
            format!("gingiva rest positions {d_rest:.2e} mm off")
        });
        let mut d_moved = 0.0f64;
        for m in MOTIONS {
            let d = worst(&port.moved[m], &plugin.moved[m]);
            d_moved = d_moved.max(d);
            r.check(d < TOL_POS_MM, || {
                format!("gingiva deformed ({m}) positions {d:.2e} mm off")
            });
        }
        let pw = raw_f32(&s.dir.join(format!("{l}_gum_weights.f32")));
        r.check(pw.len() == w.w.len(), || {
            format!("weights: {} entries, plugin {}", w.w.len(), pw.len())
        });
        let d_w =
            w.w.iter()
                .zip(&pw)
                .map(|(a, b)| f64::from((a - b).abs()))
                .fold(0.0, f64::max);
        r.check(d_w < TOL_W, || format!("weights {d_w:.2e} off"));
        let nf = seq.len();
        let col = |v: &[f32], j: usize| {
            (0..port.rest.len())
                .map(|i| f64::from(v[i * nf + j]))
                .sum::<f64>()
        };
        let sums: Vec<String> = (0..nf)
            .map(|j| format!("{}:{:.3}/{:.3}", seq[j], col(&w.w, j), col(&pw, j)))
            .collect();
        eprintln!(
            "scan {l}: gingiva: {} vertices, {} faces, rest {d_rest:.1e} mm, deformed {d_moved:.1e} mm, \
             weights {d_w:.1e}; weight column sums port/plugin {}",
            port.rest.len(),
            port.tris.len(),
            sums.join(" ")
        );
    }

    // the closed teeth against the gingiva at rest, the same measure on both
    // sides
    let port_solids: Solids = closed
        .iter()
        .filter(|(f, t)| port_teeth.contains_key(f) && t.vertices.iter().all(|p| p.is_finite()))
        .map(|(f, t)| {
            (
                *f,
                (
                    port_teeth[f].rest_frame(),
                    t.vertices.clone(),
                    t.tris.clone(),
                ),
            )
        })
        .collect();
    let plugin_solids = plugin_solids(s);
    let (port_band, plugin_band) = (
        deep_band(&port, &port_solids),
        deep_band(&plugin, &plugin_solids),
    );
    let (port_shown, plugin_shown) = (shown_tris(&port), shown_tris(&plugin));
    let rest = BTreeMap::new();
    let p0 = seen_mm2(&port.rest, &port_shown, &port_solids, &rest, &port_band);
    let o0 = seen_mm2(
        &plugin.rest,
        &plugin_shown,
        &plugin_solids,
        &rest,
        &plugin_band,
    );
    let (p0_sum, o0_sum) = (p0.values().sum::<f64>(), o0.values().sum::<f64>());
    r.check((p0_sum - o0_sum).abs() <= SEEN_TOL_MM2, || {
        format!("seen at rest {p0_sum:.3} mm2, plugin {o0_sum:.3}")
    });

    for (m, deltas) in &motions {
        // deformation quality: the Rust measure of the plugin's gingiva is the
        // plugin script's; the port's equals the plugin's and stays within the
        // plugin's bounds wherever the plugin's own gingiva does
        let (q_port, q_plugin) = (
            quality(&port, &port.moved[m]),
            quality(&plugin, &plugin.moved[m]),
        );
        let wm = &s.oracle["motions"][*m];
        let wq = &wm["quality"];
        let q_script = Quality {
            flipped: count(&wq["flipped"]),
            shown: count(&wq["shown"]),
            flipped_shown: count(&wq["flipped_shown"]),
            stretch_max: num(&wq["stretch_max"]),
            stretch_gt3: count(&wq["stretch_gt3"]),
        };
        let close_q = |a: &Quality, b: &Quality, tol: f64| {
            Quality {
                stretch_max: 0.0,
                ..*a
            } == Quality {
                stretch_max: 0.0,
                ..*b
            } && (a.stretch_max - b.stretch_max).abs() <= tol * b.stretch_max
        };
        r.check(close_q(&q_plugin, &q_script, METRIC_TOL), || {
            format!("metric ({m}): Rust {q_plugin:?} on the plugin's gingiva, plugin script {q_script:?}")
        });
        r.check(close_q(&q_port, &q_plugin, STRETCH_TOL), || {
            format!("quality ({m}) {q_port:?}, plugin {q_plugin:?}")
        });
        let tri = port.tris.len() as f64;
        let folds = |q: &Quality| {
            (q.flipped as f64) < FLIP_SHARE * tri
                && (q.flipped_shown as f64) < FLIP_SHARE_SHOWN * q.shown as f64
        };
        if folds(&q_plugin) {
            r.check(folds(&q_port), || {
                format!("gingiva folds ({m}): {q_port:?}")
            });
        }
        if q_plugin.stretch_max < STRETCH_MAX {
            r.check(q_port.stretch_max < STRETCH_MAX, || {
                format!(
                    "gingiva spikes ({m}): longest edge {:.2} x",
                    q_port.stretch_max
                )
            });
        }
        let (d_port, d_plugin) = (
            quality_dev(&port, &port.moved[m]),
            quality_dev(&plugin, &plugin.moved[m]),
        );
        for (k, part) in ["visible", "hidden"].into_iter().enumerate() {
            let ws = &wm["quality_dev"][part];
            let script = DevQuality {
                triangles: count(&ws["triangles"]),
                folded: count(&ws["folded"]),
                p999: num(&ws["stretch_p999"]),
                max: num(&ws["stretch_max"]),
                gt3: count(&ws["stretch_gt3"]),
            };
            let close_d = |a: &DevQuality, b: &DevQuality, tol: f64| {
                (a.triangles, a.folded, a.gt3) == (b.triangles, b.folded, b.gt3)
                    && (a.p999 - b.p999).abs() <= tol * b.p999.max(1.0)
                    && (a.max - b.max).abs() <= tol * b.max.max(1.0)
            };
            r.check(close_d(&d_plugin[k], &script, METRIC_TOL), || {
                format!(
                    "metric ({m}) {part}: Rust {:?}, plugin script {script:?}",
                    d_plugin[k]
                )
            });
            r.check(close_d(&d_port[k], &d_plugin[k], STRETCH_TOL), || {
                format!(
                    "quality ({m}) {part}: {:?}, plugin {:?}",
                    d_port[k], d_plugin[k]
                )
            });
        }
        eprintln!(
            "scan {l} ({m} motion, teeth moved up to {:.2} mm): gingiva flipped {} of {} (shown {} of {}), \
             longest edge {:.2} x, >3x {}; plugin flipped {} (shown {}), {:.2} x; dev_gum_quality visible \
             folded {} stretch p99.9 {:.2} max {:.2}, hidden folded {} max {:.2}",
            num(&wq["moved_max_mm"]),
            q_port.flipped,
            port.tris.len(),
            q_port.flipped_shown,
            q_port.shown,
            q_port.stretch_max,
            q_port.stretch_gt3,
            q_plugin.flipped,
            q_plugin.flipped_shown,
            q_plugin.stretch_max,
            d_port[0].folded,
            d_port[0].p999,
            d_port[0].max,
            d_port[1].folded,
            d_port[1].max,
        );

        // the closed teeth against the deformed gingiva
        let t0 = Instant::now();
        let p1 = seen_mm2(
            &port.moved[m],
            &port_shown,
            &port_solids,
            deltas,
            &port_band,
        );
        let o1 = seen_mm2(
            &plugin.moved[m],
            &plugin_shown,
            &plugin_solids,
            deltas,
            &plugin_band,
        );
        let grow = |s1: &BTreeMap<Fdi, f64>, s0: &BTreeMap<Fdi, f64>| {
            s1.iter()
                .map(|(f, x)| (*f, x - s0[f]))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((seq[0], 0.0))
        };
        let (p1_sum, o1_sum) = (p1.values().sum::<f64>(), o1.values().sum::<f64>());
        let (pg, og) = (grow(&p1, &p0), grow(&o1, &o0));
        eprintln!(
            "scan {l} ({m} motion): closed teeth seen out of the gingiva: port {p0_sum:.2} -> {p1_sum:.2} mm2 \
             (largest increase tooth {} {:.2}), plugin {o0_sum:.2} -> {o1_sum:.2} mm2 (tooth {} {:.2}); \
             measured in {:.1} s",
            pg.0,
            pg.1,
            og.0,
            og.1,
            t0.elapsed().as_secs_f64()
        );
        r.check((p1_sum - o1_sum).abs() <= SEEN_TOL_MM2, || {
            format!("seen ({m}) {p1_sum:.3} mm2, plugin {o1_sum:.3}")
        });
        if o1_sum <= SEEN_RATIO * o0_sum {
            r.check(p1_sum <= SEEN_RATIO * p0_sum, || {
                format!(
                    "closed teeth come out of the gingiva ({m}): seen {p1_sum:.2} mm2, rest {p0_sum:.2}"
                )
            });
        }
        if og.1 <= SEEN_TOOTH_MM2 {
            r.check(pg.1 <= SEEN_TOOTH_MM2, || {
                format!(
                    "closed tooth {} comes out of the gingiva by {:.2} mm2 ({m})",
                    pg.0, pg.1
                )
            });
        }
    }
}

fn check_scan(s: &Scan) {
    let mut r = Report {
        letter: s.letter,
        fails: Vec::new(),
    };
    let t0 = Instant::now();
    let port_teeth = check_teeth(s, &mut r);
    let closed = check_closed(s, &mut r);
    check_gingiva(s, &port_teeth, &closed, &mut r);
    eprintln!(
        "scan {}: {} failures, {:.1} s in all",
        s.letter,
        r.fails.len(),
        t0.elapsed().as_secs_f64()
    );
    assert!(r.fails.is_empty(), "{}", r.fails.join("\n"));
}

fn with_scan(letter: char) {
    let (Some(files), Some(dir)) = (scans(), oracle_dir()) else {
        return;
    };
    let i = (letter as u8 - b'a') as usize;
    check_scan(&load(letter, &files[i], &dir));
}

#[test]
fn teeth_of_real_scan_a_match_the_plugin() {
    with_scan('a');
}

#[test]
fn teeth_of_real_scan_b_match_the_plugin() {
    with_scan('b');
}

#[test]
fn teeth_of_real_scan_c_match_the_plugin() {
    with_scan('c');
}

#[test]
fn teeth_of_real_scan_d_match_the_plugin() {
    with_scan('d');
}
