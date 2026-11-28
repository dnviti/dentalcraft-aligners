//! Closed teeth of `dca_algo::reconstruct` against the Python plugin
//! (`reconstruct.closed_tooth`) on the six golden demo cases.
//!
//! The Rust stage is fed the oracle's inputs: the scan in arch coordinates
//! (ground-truth arch frame), the oracle's segmentation labels and the long
//! axes of its teeth stage. The oracle's closed teeth are
//! `staging/closed_<fdi>.dcm` (tooth-local coordinates, moved back with the
//! tooth's `rest_frame`). Checked: every closed tooth is watertight,
//! manifold and one piece without a tunnel through it, its numbers (crown,
//! bridge, thin, filled, convex-held, contact, despiked vertices, closing
//! axis, crossings) are the oracle's, and its symmetric surface Hausdorff
//! distance to the oracle's solid is below [`TOL_MM`] (measured: at most
//! 6.1e-6 mm, the single-precision rounding of the fixtures).
//!
//! The default test runs one case (about 1 s in release, 30-60 s in debug);
//! all six: `cargo test --release -p dca-algo --test reconstruct_oracle --
//! --ignored --nocapture` (1-5 s per case in release). `DC_CASE=a,b`
//! selects cases for the second.

mod golden_case;

use dc_mesh::{TriMesh, ValidationReport};
use dca_algo::reconstruct::{self, Input, Params};
use glam::DVec3;
use golden_case::{CASES, Case, json, transform};

const TOL_MM: f64 = 1e-4;

fn mesh(v: &[DVec3], t: &[[u32; 3]]) -> TriMesh {
    TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t.to_vec())
}

struct Row {
    fdi: String,
    hausdorff: f64,
    /// The plugin's solid has a tunnel through it, ours not: a departure,
    /// not compared with the plugin's (see [`compare_case`]).
    departed: bool,
}

fn compare_case(name: &str) -> Vec<Row> {
    let case = Case::load(name);
    let axes = case.axes();
    let input = Input {
        vertices: &case.vertices,
        tris: &case.tris,
        labels: &case.labels,
        axes: &axes,
    };
    let t0 = std::time::Instant::now();
    let out = reconstruct::run(&input, &Params::default()).unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let staging = json(&case.dir.join("staging/staging.json"));
    let mut rows = Vec::new();
    for (f, tooth) in &out.teeth {
        let m = mesh(&tooth.vertices, &tooth.tris);
        let rep = ValidationReport::of(&m);
        assert!(
            rep.is_watertight() && rep.is_manifold(),
            "{name} {f}: not a closed solid: {rep:?}"
        );
        let s = dc_mesh::solid_topology(&m);
        assert_eq!(
            (s.pieces, s.genus),
            (1, Some(0)),
            "{name} {f}: not one solid without a tunnel: {s:?}"
        );
        let oracle = dc_mesh::dcm::decode(
            &std::fs::read(case.dir.join(format!("staging/closed_{f}.dcm"))).unwrap(),
        )
        .unwrap();
        let rf = case.rest_frame(*f);
        let ov: Vec<DVec3> = oracle
            .positions
            .iter()
            .map(|p| transform(&rf, p.as_dvec3()))
            .collect();
        let om = mesh(&ov, &oracle.tris);
        let hd = dc_geom::metrics::hausdorff_surface(&m, &om).unwrap();
        let info = &staging["closed_tooth_info"][f.to_string()];
        let rust = serde_json::to_value(tooth.info).unwrap();
        // a plugin's solid with a tunnel through it (a handle of the scanned
        // crown, which the plugin keeps) is a departure: ours has the handle
        // cut away (`reconstruct/handles.rs`), so neither its numbers nor its
        // surface are the plugin's (on `demo_lower_severe` 35 the plugin's
        // closing along the long axis folded at the tunnel and it took
        // another axis, with 16 crossing faces; ours closes along the long
        // axis without one): a handle cut, and no more crossing faces
        let departed = dc_mesh::solid_topology(&om).genus.is_some_and(|g| g > 0);
        if departed {
            let theirs = info["crossing_faces"].as_u64().unwrap() as usize;
            assert!(
                tooth.info.handles_cut > 0 && tooth.info.crossing_faces <= theirs,
                "{name} {f}: {:?}",
                tooth.info
            );
        } else {
            for (k, v) in info.as_object().unwrap() {
                assert_eq!(&rust[k], v, "{name} {f}: {k}");
            }
        }
        eprintln!(
            "{name} {f}: {}hausdorff {hd:.2e} mm | verts {} vs {} | rust {:?} | oracle {}",
            if departed {
                "DEPARTED, the plugin's solid has a tunnel through it: "
            } else {
                ""
            },
            tooth.vertices.len(),
            oracle.positions.len(),
            tooth.info,
            info
        );
        rows.push(Row {
            fdi: f.to_string(),
            hausdorff: hd,
            departed,
        });
    }
    let mut h: Vec<f64> = rows
        .iter()
        .filter(|r| !r.departed)
        .map(|r| r.hausdorff)
        .collect();
    h.sort_by(f64::total_cmp);
    let (median, worst) = (h[h.len() / 2], h[h.len() - 1]);
    eprintln!(
        "{name}: {} teeth in {secs:.1} s, Hausdorff median {median:.2e} mm, worst {worst:.2e} mm",
        rows.len()
    );
    rows
}

fn selected() -> Vec<String> {
    match std::env::var("DC_CASE") {
        Ok(c) => c.split(',').map(str::to_owned).collect(),
        Err(_) => CASES.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn check(names: &[String]) {
    for name in names {
        for r in compare_case(name).into_iter().filter(|r| !r.departed) {
            assert!(r.hausdorff < TOL_MM, "{name} {}: {} mm", r.fdi, r.hausdorff);
        }
    }
}

#[test]
fn closed_teeth_match_the_plugin() {
    check(&["demo_upper_mild".to_owned()]);
}

#[test]
#[ignore = "six cases, 1-5 s each in release, about a minute each in debug: run with --release --ignored"]
fn closed_teeth_match_the_plugin_on_every_case() {
    check(&selected());
}
