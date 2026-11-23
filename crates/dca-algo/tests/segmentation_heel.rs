//! The segmentation behind the last molar of a printed model: the base of
//! the model rises behind the molar into a heel as high as the crowns, one
//! surface with it, as on the printed test piece of the project. Behind the
//! last tooth there is never a crown: past the distal surface of the last
//! tooth the scan is gingiva (`segmentation::faces`), and that constraint
//! must leave the crown of the last molar whole. On this synthetic heel the
//! walk alone already keeps the heel out of the molar (its rim makes a
//! fold); the spill it stops on the real test piece, where the base meets
//! the molar without one, is measured on the real scans.

use dc_case::Fdi;
use dca_algo::anatomy::{self, Arch};
use dca_algo::segmentation::{self, Input, Params};
use glam::DVec3;

/// Step of the height field (mm).
const STEP: f64 = 0.25;
/// Height of the crown tops (arch coordinates).
const TOP: f64 = 1.5;

/// The upper right posterior teeth of the model, from the last molar.
fn teeth() -> [Fdi; 4] {
    [16, 15, 14, 13].map(|n| Fdi::new(n).unwrap())
}

/// A printed model of the upper right posterior teeth seen from above.
struct Model {
    /// Vertices (arch coordinates).
    v: Vec<DVec3>,
    /// Triangles.
    t: Vec<[u32; 3]>,
    /// The seed of every tooth.
    seeds: Vec<(Fdi, DVec3)>,
    /// `y` of the distal surface of 16.
    distal: f64,
}

/// The [`Model`], one surface: the base of the model about 5 mm under the
/// crown tops with the crowns of 16-13 on it (16 at `y = 0`, the others
/// towards `+y`, touching), and behind 16 the base rising into a heel 6 mm
/// long and 16 mm wide that reaches the distal slope of 16, its rough top
/// `rise` mm over the crowns.
fn model(rise: f64) -> Model {
    let mut y = 0.0;
    let mut crowns = Vec::new();
    for (k, f) in teeth().into_iter().enumerate() {
        let (md, bl) = anatomy::typical_diameters(f);
        if k > 0 {
            y += md / 2.0;
        }
        crowns.push((f, y, md, bl));
        y += md / 2.0;
    }
    let distal = -crowns[0].2 / 2.0;
    // the heel reaches the distal slope of the molar: one surface with it
    let (long, wide) = (6.0, 8.0);
    let heel = (distal - long, distal + 0.5);
    let height = |x: f64, y: f64| -> f64 {
        let mut z = -4.0 - 0.01 * x * x;
        for &(_, c, md, bl) in &crowns {
            let e = ((y - c) / (md / 2.0)).powi(4) + (x / (bl / 2.0)).powi(4);
            if e <= 1.0 {
                z = z.max(TOP - 2.0 * e);
            }
        }
        if x.abs() <= wide && (heel.0..=heel.1).contains(&y) {
            let rough = 0.2 * (y * 7.6).sin() * (x * 9.2).cos();
            z = z.max(TOP + rise + rough);
        }
        z
    };
    let (x0, x1) = (-14.0, 14.0);
    let (y0, y1) = (heel.0 - 4.0, y + 4.0);
    let nx = ((x1 - x0) / STEP) as u32 + 1;
    let ny = ((y1 - y0) / STEP) as u32 + 1;
    let mut v = Vec::with_capacity((nx * ny) as usize);
    for i in 0..nx {
        for j in 0..ny {
            let (x, y) = (x0 + f64::from(i) * STEP, y0 + f64::from(j) * STEP);
            v.push(DVec3::new(x, y, height(x, y)));
        }
    }
    let id = |i: u32, j: u32| i * ny + j;
    let mut t = Vec::new();
    for i in 0..nx - 1 {
        for j in 0..ny - 1 {
            t.push([id(i, j), id(i + 1, j), id(i + 1, j + 1)]);
            t.push([id(i, j), id(i + 1, j + 1), id(i, j + 1)]);
        }
    }
    let seeds = crowns
        .iter()
        .map(|&(f, c, _, _)| (f, DVec3::new(0.0, c, TOP)))
        .collect();
    Model {
        v,
        t,
        seeds,
        distal,
    }
}

/// The heel as high as the crowns, lower or higher: past 3 mm behind the
/// distal surface of 16 (2 mm and a margin for the boundary refinement)
/// nothing is a tooth, while the core of the crown of 16 stays 16 (the
/// gingiva behind the last tooth does not cut it).
#[test]
fn the_base_behind_the_last_molar_is_gum() {
    for rise in [-0.5, 0.0, 1.5] {
        let Model {
            v,
            t,
            seeds,
            distal,
        } = model(rise);
        let out = segmentation::run(
            &Input {
                arch: Arch::Upper,
                vertices: &v,
                tris: &t,
                seeds: &seeds,
                decimated: None,
            },
            &Params::default(),
        )
        .unwrap();
        let behind: Vec<usize> = (0..v.len())
            .filter(|&i| v[i].y < distal - 3.0 && out.labels[i] != 0)
            .collect();
        assert!(
            behind.is_empty(),
            "heel {rise} mm: {} vertices behind 16 in a tooth, the first at {:?} in {}",
            behind.len(),
            behind.first().map(|&i| v[i]),
            behind.first().map_or(0, |&i| out.labels[i])
        );
        let (md, bl) = anatomy::typical_diameters(teeth()[0]);
        let crown: Vec<usize> = (0..v.len())
            .filter(|&i| {
                let e = (v[i].y / (md / 2.0)).powi(4) + (v[i].x / (bl / 2.0)).powi(4);
                e <= 0.3
            })
            .collect();
        let of_16 = crown.iter().filter(|&&i| out.labels[i] == 16).count();
        assert!(
            of_16 * 100 >= crown.len() * 95,
            "heel {rise} mm: only {of_16} of {} vertices of the crown of 16 are 16",
            crown.len()
        );
    }
}
