//! The planning (`dca_algo::setup`, `dca_algo::staging`) on the four real
//! arch scans, compared with the Python plugin's `conservative_setup` and
//! `staging.plan` on the same inputs, with the target poses as the case
//! stores them.
//!
//! The scans are patient data: they are not in the repository, and neither
//! their file names nor any coordinate may appear in committed code or in
//! test output (see `dca-module/tests/import_real.rs`). The folder comes from
//! `DC_REAL_SCANS`; the files are sorted by name and called `a` to `d`
//! (lower, upper, lower, upper).
//!
//! What the plugin finds is read from the folder `DC_PLANNING_REAL_ORACLE`
//! names, kept outside the repository: first `fixtures/gen_segmentation_real.py`
//! writes the plugin's segmentation there (`<letter>.json`: arch frame, final
//! vertex labels), then `fixtures/gen_planning_real.py` runs the plugin's
//! teeth, diagnosis, setup and staging from those labels and writes the
//! folder `<letter>/` (the files of a golden case: `teeth.json`,
//! `setup.json`, `staging.json`, the closed crowns `closed_<fdi>.dcm` in
//! their rest frames, and `overlaps.json`). Regenerate it, run, delete it:
//!
//! ```sh
//! out=$(mktemp -d /tmp/dc-planreal-XXXX)
//! for l in a b c d; do        # one process per scan, Blender on PATH
//!   DC_REAL_SCANS=<scans> python3 -B crates/dca-algo/tests/fixtures/gen_segmentation_real.py "$out" --only $l &&
//!   DC_REAL_SCANS=<scans> python3 -B crates/dca-algo/tests/fixtures/gen_planning_real.py "$out" --only $l &
//! done; wait                  # minutes per scan, mostly the plugin's closed teeth
//! DC_REAL_SCANS=<scans> DC_PLANNING_REAL_ORACLE="$out" cargo test --release -p dca-algo \
//!     --test planning_real -- --nocapture --test-threads=1
//! rm -r "$out"
//! ```
//!
//! Without `DC_REAL_SCANS` or `DC_PLANNING_REAL_ORACLE`, or without the
//! folder of a scan, the tests are skipped with a line on stderr. The
//! messages carry tooth numbers, counts, distances, angles and aligner
//! counts, never positions. Every check of a scan runs and the failures are
//! reported together. Checked per scan:
//!
//! * the setup, fed what the setup command feeds it (the port's own teeth,
//!   `teeth::run` on the plugin's labels and frame): the same moved teeth,
//!   every target move within [`TOL_MM`] and [`TOL_DEG`], the same IPR plan,
//!   expansion and advancement within 1e-6 mm, the arch line within
//!   [`TOL_MM`];
//! * the staging of the plugin's targets as the case stores them
//!   (`staging::pose_from_q` of `q_from_delta`, what the setup command
//!   stores), with the quaternion sign as stored and flipped (projects saved
//!   before the fix of 2026-11-14 hold `w < 0`), read back as the staging
//!   command reads them (`staging::q_from_pose`), with the plugin's closed
//!   crowns, IPR plan and complexity: the same number of aligners, every
//!   pose within the bounds, the same aligners per tooth, IPR stages and
//!   unresolved collisions, the final overlaps within [`TOL_OVERLAP_MM`];
//! * the chained plan, from the port's own setup targets the same way: the
//!   same number of aligners as the plugin;
//! * the overlap of every adjacent pair of crowns at rest and in the final
//!   setup (the staging's collision metric) against the plugin's.
//!
//! Printed per scan: aligners (port and plugin), the largest turn of a
//! tooth from one aligner to the next, and every pair of crowns that
//! overlaps beyond the collision tolerance at rest or in the final setup.

mod golden_case;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::frame::rigid_inverse;
use dca_algo::rotation;
use dca_algo::setup::{self, ToothInput};
use dca_algo::staging::{self, Complexity, IprPlan, Level, Params, Q6, ToothMove};
use dca_algo::teeth::{self, Which};
use glam::{DMat3, DMat4, DVec3};
use golden_case::{mat4, transform, unpack_labels, vec3};
use serde_json::Value;

/// Target moves and staged poses, mm (PLAN section 5).
const TOL_MM: f64 = 0.01;
/// Target moves and staged poses, degrees (PLAN section 5).
const TOL_DEG: f64 = 0.05;
/// Overlap of two crowns, mm (`staging_oracle.rs`).
const TOL_OVERLAP_MM: f64 = 5e-3;

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
    let Some(dir) = std::env::var_os("DC_PLANNING_REAL_ORACLE").map(PathBuf::from) else {
        eprintln!("skipped: DC_PLANNING_REAL_ORACLE is not set");
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

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn fdi(f: u64) -> Fdi {
    Fdi::new(f as u8).unwrap()
}

fn key(k: &str) -> Fdi {
    fdi(k.parse().unwrap())
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

/// Angle (degrees) of the rotation from `a` to `b`.
fn rotation_deg(a: &DMat4, b: &DMat4) -> f64 {
    let r = DMat3::from_mat4(*a).transpose() * DMat3::from_mat4(*b);
    rotation::matrix_to_rotvec(&r).length().to_degrees()
}

/// Translation (mm) and rotation (degrees) between two movements.
fn pose_error(a: &Q6, b: &Q6) -> (f64, f64) {
    let t = DVec3::new(a[0] - b[0], a[1] - b[1], a[2] - b[2]).length();
    let (pa, pb) = (staging::pose_from_q(a), staging::pose_from_q(b));
    (t, rotation::degrees(&pb.compose(&pa.inverse())))
}

/// The largest turn (degrees) of any tooth from one aligner to the next.
fn max_turn<'a>(series: impl Iterator<Item = &'a Vec<Q6>>) -> f64 {
    series
        .flat_map(|qs| qs.windows(2).map(|w| pose_error(&w[0], &w[1]).1))
        .fold(0.0, f64::max)
}

fn q6(v: &Value) -> Q6 {
    std::array::from_fn(|i| num(&v[i]))
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
/// planning.
struct Scan {
    letter: char,
    arch: Arch,
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    labels: Vec<u16>,
    teeth: Value,
    setup: Value,
    plan: Value,
    overlaps: Value,
    dir: PathBuf,
}

fn load(letter: char, path: &Path, dir: &Path) -> Scan {
    let seg = read_json(&dir.join(format!("{letter}.json")));
    let dir = dir.join(letter.to_string());
    let teeth = read_json(&dir.join("teeth.json"));
    let arch = match teeth["arch"].as_str() {
        Some("UPPER") => Arch::Upper,
        Some("LOWER") => Arch::Lower,
        other => panic!("scan {letter}: the oracle has no arch ({other:?})"),
    };
    let mesh = dc_mesh::io::read(path).unwrap_or_else(|_| panic!("scan {letter}: unreadable"));
    let world: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
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
        setup: read_json(&dir.join("setup.json")),
        plan: read_json(&dir.join("staging.json"))["plan"].clone(),
        overlaps: read_json(&dir.join("overlaps.json")),
        teeth,
        dir,
    }
}

/// The plugin's sequence.
fn sequence(s: &Scan) -> Vec<Fdi> {
    s.teeth["sequence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| fdi(f.as_u64().unwrap()))
        .collect()
}

/// The setup from the port's own teeth against `conservative_setup`;
/// returns the port's targets.
fn check_setup(s: &Scan, r: &mut Report) -> BTreeMap<Fdi, DMat4> {
    let geo = teeth::run(
        &teeth::Input {
            arch: s.arch,
            vertices: &s.vertices,
            tris: &s.tris,
            labels: &s.labels,
        },
        &teeth::Params {
            arch: if s.arch.is_upper() {
                Which::Upper
            } else {
                Which::Lower
            },
            ..teeth::Params::default()
        },
    )
    .unwrap_or_else(|e| panic!("scan {}: teeth: {e}", s.letter));
    r.check(geo.sequence == sequence(s), || {
        format!("sequence {:?} against {:?}", geo.sequence, sequence(s))
    });
    let inputs: BTreeMap<Fdi, ToothInput> = geo
        .teeth
        .iter()
        .map(|(f, t)| (*f, ToothInput::from(t)))
        .collect();
    let out = setup::run(
        &setup::Input {
            arch: s.arch,
            vertices: &s.vertices,
            labels: &s.labels,
            sequence: &geo.sequence,
            teeth: &inputs,
            line: None,
            fixed: &[],
            poses: &BTreeMap::new(),
        },
        &setup::Params::default(),
    )
    .unwrap_or_else(|e| panic!("scan {}: setup: {e}", s.letter));
    let res = &out.result;
    let deltas = s.setup["targets_delta"].as_object().unwrap();
    let got: Vec<Fdi> = res.targets.keys().copied().collect();
    let want: Vec<Fdi> = deltas.keys().map(|k| key(k)).collect();
    r.check(got == want, || {
        format!("moved teeth {got:?} against {want:?}")
    });
    let (mut worst_mm, mut worst_deg) = (0.0_f64, 0.0_f64);
    for (k, d) in deltas {
        let Some(a) = res.targets.get(&key(k)) else {
            continue;
        };
        let b = mat4(d);
        let dt = (a.w_axis - b.w_axis).truncate().length();
        let deg = rotation_deg(a, &b);
        worst_mm = worst_mm.max(dt);
        worst_deg = worst_deg.max(deg);
        r.check(dt <= TOL_MM && deg <= TOL_DEG, || {
            format!("target of {k}: {dt:.2e} mm, {deg:.2e} deg")
        });
    }
    let su = &s.setup["setup"];
    let ipr: Vec<(u8, u8, f64)> = res
        .ipr
        .iter()
        .map(|((a, b), mm)| (a.get(), b.get(), *mm))
        .collect();
    let want_ipr: Vec<(u8, u8, f64)> = su["ipr"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["a"].as_u64().unwrap() as u8,
                x["b"].as_u64().unwrap() as u8,
                num(&x["mm"]),
            )
        })
        .collect();
    r.check(
        ipr.len() == want_ipr.len()
            && ipr
                .iter()
                .zip(&want_ipr)
                .all(|(g, w)| (g.0, g.1) == (w.0, w.1) && (g.2 - w.2).abs() < 1e-9),
        || format!("IPR {ipr:?} against {want_ipr:?}"),
    );
    for (got, name) in [
        (res.expansion, "expansion_posterior"),
        (res.expansion_canine, "expansion_canine"),
        (res.advance, "advance"),
    ] {
        r.check((got - num(&su[name])).abs() < 1e-6, || {
            format!("{name} {got} against {}", su[name])
        });
    }
    let line = su["line"].as_array().unwrap();
    let line_mm = out
        .line
        .iter()
        .zip(line)
        .map(|(p, q)| (*p - vec3(q)).length())
        .fold(0.0, f64::max);
    r.check(out.line.len() == line.len() && line_mm <= TOL_MM, || {
        format!("arch line: {} samples, {line_mm:.2e} mm", out.line.len())
    });
    eprintln!(
        "scan {}: setup moves {} teeth, targets within {worst_mm:.1e} mm and {worst_deg:.1e} deg, \
         line within {line_mm:.1e} mm, setup overlap_max {:.3} mm (plugin {:.3} mm)",
        s.letter,
        res.targets.len(),
        res.overlap_max,
        num(&s.overlaps["setup_overlap_max"]),
    );
    res.targets.clone()
}

/// The teeth to stage: the rest frames, closed crowns and `md_sign` of the
/// plugin, the targets `delta` stored as the case stores them and read
/// back as the staging command reads them (`flip`: with the sign of every
/// stored quaternion flipped).
fn moves(s: &Scan, delta: &dyn Fn(Fdi) -> DMat4, flip: bool) -> Vec<ToothMove> {
    sequence(s)
        .into_iter()
        .map(|f| {
            let t = &s.teeth["teeth"][f.to_string()];
            let frame = mat4(&t["rest_frame"]);
            let mut stored = staging::pose_from_q(&staging::q_from_delta(&frame, &delta(f)));
            assert!(stored.q.w >= 0.0, "scan {}: {f}", s.letter);
            if flip {
                stored.q = -stored.q;
            }
            let bytes = std::fs::read(s.dir.join(format!("closed_{f}.dcm"))).unwrap();
            let mesh = dc_mesh::dcm::decode(&bytes).unwrap();
            ToothMove::new(
                f,
                frame,
                staging::q_from_pose(&stored),
                mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
                t["md_sign"].as_i64().unwrap() as i8,
            )
        })
        .collect()
}

fn plan(s: &Scan, moves: &[ToothMove]) -> staging::Plan {
    let ipr: Vec<IprPlan> = s.setup["setup"]["ipr"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| IprPlan {
            a: fdi(x["a"].as_u64().unwrap()),
            b: fdi(x["b"].as_u64().unwrap()),
            mm: num(&x["mm"]),
        })
        .collect();
    let cx = &s.plan["complexity"];
    let complexity = Complexity {
        level: match cx["level"].as_str().unwrap() {
            "FACILE" => Level::Easy,
            "MODERATO" => Level::Moderate,
            "DIFFICILE" => Level::Difficult,
            _ => Level::VeryComplex,
        },
        crowding_mm: num(&cx["crowding"]),
        little_mm: num(&cx["little"]),
        distalization_mm: num(&cx["distalization"]),
        pattern_override: None,
    };
    staging::run(
        &staging::Input {
            moves,
            ipr: &ipr,
            complexity,
        },
        // the plugin's sample of the crowns (the port's default uses every point)
        &Params {
            sample_points: Some(2500),
            ..Params::default()
        },
    )
    .unwrap()
}

/// The plugin's targets through the stored poses against `staging.plan`.
fn check_staging(s: &Scan, r: &mut Report) {
    let want = &s.plan;
    let n = want["n_stages"].as_u64().unwrap() as u32;
    let targets = s.setup["targets_delta"].as_object().unwrap();
    let delta = |f: Fdi| targets.get(&f.to_string()).map_or(DMat4::IDENTITY, mat4);
    let want_q: BTreeMap<Fdi, Vec<Q6>> = want["q"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (key(k), v.as_array().unwrap().iter().map(q6).collect()))
        .collect();
    let plugin_turn = max_turn(want_q.values());
    for flip in [false, true] {
        let p = plan(s, &moves(s, &delta, flip));
        r.check(p.n_stages == n, || {
            format!("flip {flip}: {} aligners against {n}", p.n_stages)
        });
        let (mut worst_mm, mut worst_deg) = (0.0_f64, 0.0_f64);
        for (f, w) in &want_q {
            let got = &p.q[f];
            r.check(got.len() == w.len(), || {
                format!(
                    "flip {flip}: {f} has {} poses against {}",
                    got.len(),
                    w.len()
                )
            });
            for (g, w) in got.iter().zip(w) {
                let (mm, deg) = pose_error(g, w);
                worst_mm = worst_mm.max(mm);
                worst_deg = worst_deg.max(deg);
            }
        }
        r.check(worst_mm < TOL_MM && worst_deg < TOL_DEG, || {
            format!("flip {flip}: poses off by {worst_mm:.2e} mm / {worst_deg:.2e} deg")
        });
        for (k, need) in want["need"].as_object().unwrap() {
            let got = p.need[&key(k)];
            r.check(got as u64 == need.as_u64().unwrap(), || {
                format!("flip {flip}: {k} needs {got} aligners against {need}")
            });
        }
        let ipr_stage: Vec<(u64, u64, u64)> = p
            .ipr_stage
            .iter()
            .map(|x| (x.a.get().into(), x.b.get().into(), x.stage.into()))
            .collect();
        let want_ipr: Vec<(u64, u64, u64)> = want["ipr_stage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x[0].as_u64().unwrap(),
                    x[1].as_u64().unwrap(),
                    x[2].as_u64().unwrap(),
                )
            })
            .collect();
        r.check(ipr_stage == want_ipr, || {
            format!("flip {flip}: IPR stages {ipr_stage:?} against {want_ipr:?}")
        });
        let unresolved = want["unresolved"].as_array().unwrap().len();
        r.check(p.unresolved.len() == unresolved, || {
            format!(
                "flip {flip}: {} unresolved collisions against {unresolved}",
                p.unresolved.len()
            )
        });
        let turn = max_turn(p.q.values());
        r.check((turn - plugin_turn).abs() < TOL_DEG, || {
            format!("flip {flip}: largest turn per aligner {turn:.3} deg against {plugin_turn:.3}")
        });
        if flip {
            eprintln!(
                "scan {}: plugin targets, stored poses with w < 0: {} aligners (plugin {n}), poses \
                 within {worst_mm:.1e} mm / {worst_deg:.1e} deg, largest turn of a tooth per \
                 aligner {turn:.3} deg (plugin {plugin_turn:.3} deg)",
                s.letter, p.n_stages
            );
        }
    }
}

/// The port's own setup targets through the stored poses: the plugin's
/// number of aligners.
fn check_chained(s: &Scan, targets: &BTreeMap<Fdi, DMat4>, r: &mut Report) {
    let n = s.plan["n_stages"].as_u64().unwrap() as u32;
    let delta = |f: Fdi| targets.get(&f).copied().unwrap_or(DMat4::IDENTITY);
    let p = plan(s, &moves(s, &delta, false));
    r.check(p.n_stages == n, || {
        format!("chained: {} aligners against {n}", p.n_stages)
    });
    eprintln!(
        "scan {}: port setup -> stored poses -> staging: {} aligners (plugin {n}), largest turn \
         of a tooth per aligner {:.3} deg",
        s.letter,
        p.n_stages,
        max_turn(p.q.values())
    );
}

/// The overlaps of adjacent crowns at rest and in the final setup.
fn check_overlaps(s: &Scan, r: &mut Report) {
    let targets = s.setup["targets_delta"].as_object().unwrap();
    let delta = |f: Fdi| targets.get(&f.to_string()).map_or(DMat4::IDENTITY, mat4);
    let at_rest = |_: Fdi| DMat4::IDENTITY;
    let rest = plan(s, &moves(s, &at_rest, false)).final_overlap;
    let fin = plan(s, &moves(s, &delta, false)).final_overlap;
    let tol = Params::default().collision_tol_mm;
    for row in s.overlaps["pairs"].as_array().unwrap() {
        let (a, b) = (
            fdi(row["a"].as_u64().unwrap()),
            fdi(row["b"].as_u64().unwrap()),
        );
        let find = |v: &[staging::Overlap]| v.iter().find(|o| (o.a, o.b) == (a, b)).map(|o| o.mm);
        let (Some(r0), Some(rf)) = (find(&rest), find(&fin)) else {
            r.check(false, || format!("no overlap measured for {a}|{b}"));
            continue;
        };
        let (w0, wf) = (num(&row["rest"]), num(&row["final"]));
        r.check(
            (r0 - w0).abs() < TOL_OVERLAP_MM && (rf - wf).abs() < TOL_OVERLAP_MM,
            || format!("overlap {a}|{b}: rest {r0:.3} final {rf:.3} against {w0:.3} {wf:.3} mm"),
        );
        if r0.max(rf).max(w0).max(wf) > tol {
            eprintln!(
                "scan {}: crowns {a}|{b} overlap {r0:.2} mm at rest, {rf:.2} mm in the final setup \
                 (plugin {w0:.2} / {wf:.2} mm)",
                s.letter
            );
        }
    }
}

fn with_scan(letter: char) {
    let (Some(files), Some(dir)) = (scans(), oracle_dir()) else {
        return;
    };
    if !dir.join(letter.to_string()).join("staging.json").exists() {
        eprintln!("skipped: no planning oracle for scan {letter}");
        return;
    }
    let path = &files["abcd".find(letter).unwrap()];
    let s = load(letter, path, &dir);
    let mut r = Report {
        letter,
        fails: Vec::new(),
    };
    let targets = check_setup(&s, &mut r);
    check_staging(&s, &mut r);
    check_chained(&s, &targets, &mut r);
    check_overlaps(&s, &mut r);
    assert!(
        r.fails.is_empty(),
        "{} failures:\n{}",
        r.fails.len(),
        r.fails.join("\n")
    );
}

#[test]
fn planning_of_real_scan_a_matches_the_plugin() {
    with_scan('a');
}

#[test]
fn planning_of_real_scan_b_matches_the_plugin() {
    with_scan('b');
}

#[test]
fn planning_of_real_scan_c_matches_the_plugin() {
    with_scan('c');
}

#[test]
fn planning_of_real_scan_d_matches_the_plugin() {
    with_scan('d');
}
