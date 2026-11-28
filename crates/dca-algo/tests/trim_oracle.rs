//! Numerical comparison of the trim lines of `dca_algo::trim` with the Python
//! plugin (`ortho_aligner/case_print.py`, `aligner.py`).
//!
//! `tests/fixtures/trim.json` is produced by `gen_trim.py` from the plugin's own
//! functions (python3 + numpy, no Blender): four synthetic teeth (the plugin's
//! demo crowns, the incisors proclined, the cervical ring scalloped) with the
//! equator loops of `tooth_equators` along the vertical axis, the EQUATOR trim
//! loops of `equator_trim` for three offsets, the loops bridged at the
//! contacts (`_bridge_contacts`), the STRAIGHT and SCALLOPED control points of
//! `trim_points`, the trim surfaces of `trim_cutter` and the SMOOTH border of
//! `smooth_border` (all covered, one tooth left out), the insertion paths
//! (COMMON and PER_TOOTH, `insertion_paths`, `insertion_warp`) and the
//! equator trims surveyed along them. Bound: 1e-6 mm.
//!
//! Also from the plugin: the refinements of the EQUATOR field (`_gum_cell_max`,
//! `_grid_distance` in single precision as its k-d tree, `_coarse_inside`) and the checks
//! of an EQUATOR border (`border_heights`, `labial_cover`, `border_offsets` and the lists
//! of `_aligner_solid`) on a border made around the trim lines; the holes through the
//! wall are in `holes_oracle.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::anatomy::{self, Arch};
use dca_algo::trim::Tooth;
use dca_algo::trim::grid::PlanGrid;
use dca_algo::trim::line::{self, MarginMode};
use dca_algo::trim::warp::{self, InsertionAxis};
use dca_algo::trim::{checks, cutter};
use dca_algo::trim::{smooth, survey};
use glam::DVec3;
use serde_json::Value;

const TOL_MM: f64 = 1e-6;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trim.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> DVec3 {
    let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    DVec3::new(n[0], n[1], n[2])
}

fn points(v: &Value) -> Vec<DVec3> {
    v.as_array().unwrap().iter().map(vec3).collect()
}

fn tooth(t: &Value) -> Tooth {
    Tooth {
        fdi: Fdi::new(num(&t["fdi"]) as u8).unwrap(),
        crown: points(&t["crown"]),
        closed: points(&t["closed"]),
        margin: points(&t["margin"]),
        origin: vec3(&t["origin"]),
        e_bl: vec3(&t["e_bl"]),
        covered: true,
    }
}

fn close(a: &[DVec3], b: &[DVec3], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: lengths");
    for (k, (p, q)) in a.iter().zip(b).enumerate() {
        assert!(
            (*p - *q).abs().max_element() < TOL_MM,
            "{what} [{k}]: {p} vs {q}"
        );
    }
}

#[test]
fn equator_loops_match_the_plugin() {
    let f = fixture();
    let wall = num(&f["wall"]);
    let mut trimmed = Vec::new();
    for t in f["teeth"].as_array().unwrap() {
        let tooth = tooth(t);
        let eq = line::tooth_equator(&tooth, line::EQUATOR_AZIMUTHS).unwrap();
        let what = format!("{} equator", tooth.fdi);
        close(&eq.points, &points(&t["equator"]), &what);
        assert!((eq.top - num(&t["top"])).abs() < TOL_MM);
        let zm: Vec<f64> = t["margin_z"].as_array().unwrap().iter().map(num).collect();
        for (a, b) in eq.margin.as_ref().unwrap().iter().zip(&zm) {
            assert!((a - b).abs() < TOL_MM, "{what} margin");
        }
        for (off, want) in t["trim"].as_object().unwrap() {
            let off: f64 = off.parse().unwrap();
            let tr = line::equator_trim(&eq, off, wall);
            close(
                &tr.points,
                &points(want),
                &format!("{} trim {off}", tooth.fdi),
            );
            if off == 0.0 {
                trimmed.push(tr.points);
            }
        }
    }
    let bridged = survey::bridge_contacts(&trimmed, cutter::EQUATOR_CONTACT_MM);
    let want: Vec<Vec<DVec3>> = f["bridged"]
        .as_array()
        .unwrap()
        .iter()
        .map(points)
        .collect();
    for (k, (a, b)) in bridged.iter().zip(&want).enumerate() {
        close(a, b, &format!("bridged {k}"));
    }
    // the contact of 31 and 32 is bridged: some heights changed
    assert!(
        bridged[0]
            .iter()
            .zip(&trimmed[0])
            .any(|(a, b)| (a.z - b.z).abs() > 1e-3)
    );
}

#[test]
fn margin_lines_and_trim_surfaces_match_the_plugin() {
    let f = fixture();
    let ext = num(&f["extension"]);
    let teeth: Vec<Tooth> = f["teeth"].as_array().unwrap().iter().map(tooth).collect();
    let c = &f["cutter"];
    let (lo, hi) = (vec3(&c["lo"]), vec3(&c["hi"]));
    let grid = PlanGrid::around(lo, hi, num(&c["cell"]));
    for (mode, key, k) in [
        (MarginMode::Straight, "straight", 2),
        (MarginMode::Scalloped, "scalloped", 4),
    ] {
        let per = line::margin_points(&teeth, mode, ext);
        for ((fdi, pts), t) in per.iter().zip(f["teeth"].as_array().unwrap()) {
            close(pts, &points(&t[key]), &format!("{fdi} {key}"));
        }
        let ctrl: Vec<DVec3> = per.iter().flat_map(|(_, p)| p.iter().copied()).collect();
        let z = cutter::interpolated(&grid, &ctrl, k, 40.0, 3.0, hi.z + 5.0);
        let want: Vec<f64> = c[key].as_array().unwrap().iter().map(num).collect();
        assert_eq!(z.len(), want.len(), "{key} grid");
        for (i, (a, b)) in z.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < TOL_MM, "{key} surface [{i}]: {a} vs {b}");
        }
    }
}

#[test]
fn smooth_border_matches_the_plugin() {
    let f = fixture();
    let teeth: Vec<Tooth> = f["teeth"].as_array().unwrap().iter().map(tooth).collect();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> =
        teeth.iter().map(|t| (t.fdi, t.crown.clone())).collect();
    for case in f["smooth"].as_array().unwrap() {
        let covered: BTreeSet<Fdi> = case["covered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Fdi::new(num(v) as u8).unwrap())
            .collect();
        let marg: Vec<(Fdi, Vec<DVec3>)> = teeth
            .iter()
            .filter(|t| covered.contains(&t.fdi))
            .map(|t| (t.fdi, t.margin.clone()))
            .collect();
        let (border, high) = smooth::smooth_border(
            &marg,
            &crowns,
            &covered,
            num(&case["outer"]),
            num(&case["gap"]),
            num(&case["scallop"]),
            // the plugin's border: no relief round the papillae
            0.0,
            false,
        )
        .unwrap();
        let what = format!("smooth {covered:?}");
        close(&border, &points(&case["border"]), &format!("{what} border"));
        // the plugin's high points, in order; ours also go on past the end
        // of the line of the arch, where the plugin skips the rows (a
        // documented deviation: `trim` module docs)
        let want = points(&case["high"]);
        let mut k = 0;
        for p in &high {
            if k < want.len() && (*p - want[k]).abs().max_element() < TOL_MM {
                k += 1;
            }
        }
        assert_eq!(k, want.len(), "{what} high: the plugin's points in order");
        assert_eq!((high.len() - want.len()) % 25, 0, "{what} high: whole rows");
    }
}

/// Over the interproximal block-out (`blocked_out`) the smooth border is not
/// raised round the papillae under the touching contacts: it runs across
/// them as the plugin's border does (no relief), and is raised only where no
/// block-out holds the shell off the papilla. Without it the notches round
/// the papillae lift it by up to the relief over the tip.
#[test]
fn over_the_block_out_the_smooth_border_runs_straight_across_the_papillae() {
    let f = fixture();
    let teeth: Vec<Tooth> = f["teeth"].as_array().unwrap().iter().map(tooth).collect();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> =
        teeth.iter().map(|t| (t.fdi, t.crown.clone())).collect();
    for case in f["smooth"].as_array().unwrap() {
        let covered: BTreeSet<Fdi> = case["covered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Fdi::new(num(v) as u8).unwrap())
            .collect();
        let marg: Vec<(Fdi, Vec<DVec3>)> = teeth
            .iter()
            .filter(|t| covered.contains(&t.fdi))
            .map(|t| (t.fdi, t.margin.clone()))
            .collect();
        let border = |relief: f64, blocked: bool| {
            smooth::smooth_border(
                &marg,
                &crowns,
                &covered,
                num(&case["outer"]),
                num(&case["gap"]),
                num(&case["scallop"]),
                relief,
                blocked,
            )
            .unwrap()
            .0
        };
        let plain = border(0.0, false);
        let lift = |b: &[DVec3]| -> (f64, f64) {
            assert_eq!(b.len(), plain.len());
            b.iter().zip(&plain).fold((0.0, 0.0), |(s, m), (p, q)| {
                let l = (p.z - q.z).max(0.0);
                (s + l, f64::max(m, l))
            })
        };
        let (notched, notched_max) = lift(&border(1.0, false));
        let (straight, straight_max) = lift(&border(1.0, true));
        eprintln!(
            "{covered:?}: lift over the plugin's border, notched {notched:.1} (max \
             {notched_max:.2} mm), over the block-out {straight:.1} (max {straight_max:.2} mm)"
        );
        assert!(notched_max > 0.5, "{covered:?}: no notch to compare");
        assert!(
            straight < 0.5 * notched,
            "{covered:?}: {straight} of {notched} lifted"
        );
    }
}

#[test]
fn insertion_paths_and_equators_along_them_match_the_plugin() {
    let f = fixture();
    let wall = num(&f["wall"]);
    let teeth: Vec<Tooth> = f["teeth"].as_array().unwrap().iter().map(tooth).collect();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> =
        teeth.iter().map(|t| (t.fdi, t.crown.clone())).collect();
    let fdis: Vec<Fdi> = crowns.keys().copied().collect();
    let order = anatomy::arch_sequence(&fdis, Arch::Lower);
    for (key, axis) in [
        ("COMMON", InsertionAxis::Common),
        ("PER_TOOTH", InsertionAxis::PerTooth),
    ] {
        let rec = &f["axes"][key];
        let w = warp::insertion_warp(&crowns, axis, &order).expect(key);
        let want = &rec["warp"];
        assert!(
            (w.z0 - num(&want["z0"])).abs() < TOL_MM && (w.zlo - num(&want["zlo"])).abs() < TOL_MM
        );
        assert!(
            (w.zhi - num(&want["zhi"])).abs() < TOL_MM
                && (w.sigma - num(&want["sigma"])).abs() < TOL_MM
        );
        let shears: Vec<DVec3> = w.shears.iter().map(|s| s.extend(0.0)).collect();
        let want_s: Vec<DVec3> = want["shears"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
                DVec3::new(n[0], n[1], 0.0)
            })
            .collect();
        close(&shears, &want_s, &format!("{key} shears"));
        for t in &teeth {
            let warped = Tooth {
                crown: t.crown.iter().map(|&q| w.to_warp(q)).collect(),
                closed: t.closed.iter().map(|&q| w.to_warp(q)).collect(),
                margin: t.margin.iter().map(|&q| w.to_warp(q)).collect(),
                ..t.clone()
            };
            let eq = line::tooth_equator(&warped, line::EQUATOR_AZIMUTHS).unwrap();
            let tr = line::equator_trim(&eq, 0.0, wall);
            let back: Vec<DVec3> = tr.points.iter().map(|&q| w.from_warp(q)).collect();
            close(
                &back,
                &points(&rec["trim"][t.fdi.to_string()]),
                &format!("{key} {} trim", t.fdi),
            );
        }
    }
}

/// numpy's float `arange(start, stop, step)`: the first two values, then
/// `start + i * (a[1] - a[0])`.
fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    let delta = (start + step) - start;
    (0..n)
        .map(|i| match i {
            0 => start,
            1 => start + step,
            _ => start + i as f64 * delta,
        })
        .collect()
}

fn analytic(p: glam::DVec2) -> f64 {
    0.3 * p.x.sin() + 0.2 * (1.3 * p.y).cos() - 0.01 * p.x * p.y
}

#[test]
fn equator_refinements_match_the_plugin() {
    let f = fixture();
    let rf = &f["refine"];
    let (lo, hi) = (vec3(&f["cutter"]["lo"]), vec3(&f["cutter"]["hi"]));
    let origins: Vec<DVec3> = f["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| vec3(&t["origin"]))
        .collect();
    // the inputs, made as gen_trim.refine_inputs makes them
    let grid = PlanGrid::around(lo, hi, 0.6);
    let nodes = grid.nodes();
    let (gx, gy) = (
        arange(lo.x - 1.0, hi.x + 1.0, 0.57),
        arange(lo.y - 1.0, hi.y + 1.0, 0.61),
    );
    let (m, n) = (gx.len(), gy.len());
    let mut pos = Vec::new();
    for &x in &gx {
        for &y in &gy {
            let z = 0.6 * (1.7 * x).sin() * (1.1 * y).cos() - 2.0;
            pos.push(glam::Vec3::new(x as f32, y as f32, z as f32));
        }
    }
    let mut tris = Vec::new();
    for i in 0..m - 1 {
        for j in 0..n - 1 {
            tris.push([i * n + j, (i + 1) * n + j, (i + 1) * n + j + 1].map(|v| v as u32));
        }
    }
    for i in 0..m - 1 {
        for j in 0..n - 1 {
            tris.push([i * n + j, (i + 1) * n + j + 1, i * n + j + 1].map(|v| v as u32));
        }
    }
    let gum = dc_mesh::TriMesh::new(pos, tris);
    let g_top: Vec<f64> = nodes
        .iter()
        .map(|p| {
            if p.x < lo.x && p.y < lo.y + 3.0 {
                f64::NEG_INFINITY
            } else {
                0.6 * (1.7 * p.x).sin() * (1.1 * p.y).cos() - 2.05
            }
        })
        .collect();
    let top_z: Vec<f64> = nodes
        .iter()
        .map(|p| {
            origins
                .iter()
                .rev()
                .find(|c| (p.x - c.x) * (p.x - c.x) + (p.y - c.y) * (p.y - c.y) < 2.5 * 2.5)
                .map_or(f64::NEG_INFINITY, |_| 1.0 + 0.1 * p.x)
        })
        .collect();
    let top: Vec<f64> = g_top.iter().zip(&top_z).map(|(g, t)| g.max(*t)).collect();

    // the ridge of the gingiva between the nodes (_gum_cell_max)
    let lifted = cutter::gum_cell_max(&gum, &grid, &g_top, Some(&top));
    let mut want = g_top.clone();
    for kv in rf["gum"].as_array().unwrap() {
        want[kv[0].as_u64().unwrap() as usize] = num(&kv[1]);
    }
    assert!(rf["gum"].as_array().unwrap().len() > 100);
    for (k, (a, b)) in lifted.iter().zip(&want).enumerate() {
        assert!(a == b || (a - b).abs() < TOL_MM, "gum [{k}]: {a} vs {b}");
    }

    // the distance from the crown columns (_grid_distance), single precision as the plugin's k-d tree
    let mask: Vec<bool> = top_z.iter().map(|t| t.is_finite()).collect();
    let asked: Vec<usize> = (0..nodes.len()).step_by(3).collect();
    let d = cutter::grid_distance(&nodes, &mask, &asked, grid.nx, grid.ny);
    let want: Vec<f64> = rf["distance"].as_array().unwrap().iter().map(num).collect();
    assert_eq!(d.len(), want.len());
    for (k, (a, b)) in d.iter().zip(&want).enumerate() {
        assert!((a - b).abs() < TOL_MM, "distance [{k}]: {a} vs {b}");
    }

    // the fine grid of a thin wall blended into the coarse one inside the outlines (_coarse_inside)
    let fine = PlanGrid::around(lo, hi, num(&rf["coarse"]["cell"]));
    let mut z: Vec<f64> = fine.nodes().into_iter().map(analytic).collect();
    let z0 = z.clone();
    let outlines: Vec<Vec<DVec3>> = f["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| points(&t["trim"]["0.0"]))
        .collect();
    let refs: Vec<&[DVec3]> = outlines.iter().map(Vec::as_slice).collect();
    cutter::coarse_inside(&fine, &mut z, &refs, |g| {
        g.nodes().into_iter().map(analytic).collect()
    });
    let mut want = z0;
    let changed = rf["coarse"]["changed"].as_array().unwrap();
    assert!(changed.len() > 100);
    for kv in changed {
        want[kv[0].as_u64().unwrap() as usize] = num(&kv[1]);
    }
    for (k, (a, b)) in z.iter().zip(&want).enumerate() {
        assert!((a - b).abs() < TOL_MM, "coarse [{k}]: {a} vs {b}");
    }
}

fn fdi(v: &Value) -> Fdi {
    Fdi::new(v.as_u64().unwrap() as u8).unwrap()
}

#[test]
fn border_checks_match_the_plugin() {
    let f = fixture();
    let c = &f["checks"];
    let loops: Vec<(Fdi, line::ToothLoop)> = f["teeth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let ctr = points2(&t["centre"]);
            (
                fdi(&t["fdi"]),
                line::ToothLoop {
                    points: points(&t["trim"]["0.5"]),
                    equator: points(&t["equator"]).iter().map(|p| p.z).collect(),
                    margin: Some(t["margin_z"].as_array().unwrap().iter().map(num).collect()),
                    top: num(&t["top"]),
                    centre: ctr,
                },
            )
        })
        .collect();
    let list: Vec<(Fdi, &line::ToothLoop)> = loops.iter().map(|(f, l)| (*f, l)).collect();
    let order: Vec<Fdi> = c["order"].as_array().unwrap().iter().map(fdi).collect();
    let pos: Vec<glam::Vec3> = c["V"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| vec3(p).as_vec3())
        .collect();
    let shell = dc_mesh::TriMesh::new(pos, Vec::new());
    let outer = num(&c["outer"]);

    // the border of the shell built at every azimuth (border_heights)
    let border = checks::border_heights(&list, &shell, None, outer);
    let want = c["border"].as_object().unwrap();
    assert_eq!(border.len(), want.len());
    for (f, b) in &border {
        let w = want[&f.to_string()].as_array().unwrap();
        assert_eq!(b.len(), w.len());
        for (k, (x, y)) in b.iter().zip(w).enumerate() {
            match y.as_f64() {
                None => assert!(x.is_nan(), "{f} border [{k}]: {x} vs none"),
                Some(y) => assert!((x - y).abs() < TOL_MM, "{f} border [{k}]: {x} vs {y}"),
            }
        }
    }

    // the checks on it (labial cover, raised borders, offsets reached)
    let got = checks::checks(&list, &shell, None, &order, num(&c["offset"]), outer, false);
    let near = |a: f64, b: &Value| (a - num(b)).abs() < 1e-9;
    let labial = c["labial"].as_object().unwrap();
    assert_eq!(got.labial.len(), labial.len());
    for (f, (mm, sh)) in &got.labial {
        let w = &labial[&f.to_string()];
        assert!(
            near(*mm, &w[0]) && near(*sh, &w[1]),
            "{f} labial {mm} {sh} vs {w}"
        );
    }
    let raised = c["raised"].as_array().unwrap();
    assert_eq!(
        got.labial_raised.len(),
        raised.len(),
        "{:?}",
        got.labial_raised
    );
    for ((f, mm), w) in got.labial_raised.iter().zip(raised) {
        assert!(
            *f == fdi(&w[0]) && near(*mm, &w[1]),
            "raised {f} {mm} vs {w}"
        );
    }
    let offsets = c["offsets"].as_object().unwrap();
    assert_eq!(got.offsets.len(), offsets.len());
    for (f, mm) in &got.offsets {
        assert!(near(*mm, &offsets[&f.to_string()]), "{f} offset {mm}");
    }
    let limited = c["limited"].as_array().unwrap();
    assert_eq!(got.offset_limited.len(), limited.len());
    for ((f, mm), w) in got.offset_limited.iter().zip(limited) {
        assert!(
            *f == fdi(&w[0]) && near(*mm, &w[1]),
            "limited {f} {mm} vs {w}"
        );
    }
    let low = c["low"].as_array().unwrap();
    assert_eq!(got.labial_low.len(), low.len());
    for ((f, mm, sh), w) in got.labial_low.iter().zip(low) {
        assert!(
            *f == fdi(&w[0]) && near(*mm, &w[1]) && near(*sh, &w[2]),
            "low {f} vs {w}"
        );
    }
    assert!(!got.labial.is_empty() && !got.offsets.is_empty());
}

fn points2(v: &Value) -> glam::DVec2 {
    let n: Vec<f64> = v.as_array().unwrap().iter().map(num).collect();
    glam::DVec2::new(n[0], n[1])
}
