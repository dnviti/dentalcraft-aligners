//! The segmentation behind the last molar of a printed model. Behind the
//! last tooth there is never a crown: past the distal surface of the last
//! tooth the scan is gingiva (`segmentation::faces`), and that constraint
//! must leave the crown of the last molar whole.
//!
//! Two models of the upper right posterior teeth. On the first, a heel as
//! high as the crowns reaches the distal slope of the molar with a rim, and
//! the constraint must not cut the crown. The second is the base of the
//! printed test piece: behind the molar the scan falls 1-3 mm to a valley
//! and rises again into a heel with no rim, one surface with the molar,
//! with a sharp or a rounded valley. The region of the last molar ends at
//! that valley (where the crown ends), at most 1 mm past it.
//!
//! On these synthetic surfaces the walk alone already stops at the valley
//! or at the rim (both make a fold it sees). On the real scans of the test
//! piece it did not: with the seeds of the automatic chain and no gingiva
//! behind the last tooth, 16 and 26 ran 5-10 mm past the valley, up to the
//! rear edge of the model; with it they end at the valley (on 1 mm strips
//! across the crown at most +0.4 mm on 16, +1.2 mm on 26 but one strip low
//! on the palatal wall of Provino-demo 26). Those scans are not in the
//! repository: this file pins the criterion, the scans measured it.

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

/// The crowns of 16-13: FDI, centre `y`, mesio-distal and bucco-lingual
/// diameters; 16 at `y = 0`, the others towards `+y`, touching.
fn crowns() -> Vec<(Fdi, f64, f64, f64)> {
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
    crowns
}

/// How far (in its own diameters) a point is from the centre of a crown:
/// under 1 on the crown, whose top falls 2 mm from its centre to its edge.
fn crown_e(x: f64, y: f64, c: f64, md: f64, bl: f64) -> f64 {
    ((y - c) / (md / 2.0)).powi(4) + (x / (bl / 2.0)).powi(4)
}

/// A printed model of the upper right posterior teeth seen from above.
struct Model {
    /// Vertices (arch coordinates), a grid of `ny` per column of `x`.
    v: Vec<DVec3>,
    /// Triangles.
    t: Vec<[u32; 3]>,
    /// Vertices per column of the grid.
    ny: usize,
    /// The seed of every tooth.
    seeds: Vec<(Fdi, DVec3)>,
}

/// The height field `height` over `x` in [-14, 14] and `y` in `[y0, y1]`
/// as one surface, with a seed on the top of every crown.
fn model(y0: f64, y1: f64, height: impl Fn(f64, f64) -> f64) -> Model {
    let (x0, x1) = (-14.0, 14.0);
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
    let seeds = crowns()
        .iter()
        .map(|&(f, c, _, _)| (f, DVec3::new(0.0, c, TOP)))
        .collect();
    Model {
        v,
        t,
        ny: ny as usize,
        seeds,
    }
}

/// The base of the model about 5 mm under the crown tops with the crowns of
/// 16-13 on it, and behind 16 the base rising into a heel 6 mm long and
/// 16 mm wide that reaches the distal slope of 16, its rough top `rise` mm
/// over the crowns. Returns the model and the `y` of the distal surface of
/// 16.
fn rim_heel(rise: f64) -> (Model, f64) {
    let crowns = crowns();
    let distal = -crowns[0].2 / 2.0;
    // the heel reaches the distal slope of the molar: one surface with it
    let (long, wide) = (6.0, 8.0);
    let heel = (distal - long, distal + 0.5);
    let height = |x: f64, y: f64| -> f64 {
        let mut z = -4.0 - 0.01 * x * x;
        for &(_, c, md, bl) in &crowns {
            let e = crown_e(x, y, c, md, bl);
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
    let end = crowns.last().map_or(0.0, |c| c.1 + c.2 / 2.0);
    (model(heel.0 - 4.0, end + 4.0, height), distal)
}

/// The base of the printed test piece behind 16: the distal wall of 16
/// falls to a valley `drop` mm under the crown tops, and from it the base
/// rises distally by `slope` mm per mm into a heel 9 mm long with no rim (it
/// falls away to the sides), one surface with the molar; the valley is
/// sharp (`round` 0) or rounded over about `round` mm. The walls of the
/// crowns go on falling under their edge until they meet the base.
fn valley_heel(drop: f64, slope: f64, round: f64) -> Model {
    let crowns = crowns();
    let (_, _, md, _) = crowns[0];
    // where the distal wall of 16 reaches the valley, on its axis
    let valley = -(md / 2.0) * (drop / 2.0).powf(0.25);
    let height = |x: f64, y: f64| -> f64 {
        let mut z = -4.0 - 0.01 * x * x;
        for &(_, c, md, bl) in &crowns {
            z = z.max(TOP - 2.0 * crown_e(x, y, c, md, bl));
        }
        // the valley under the molar, the heel rising behind it
        let along = if y < valley {
            slope * (valley - y)
        } else {
            -0.5 * (y - valley)
        };
        let heel = TOP - drop + along - 0.06 * x * x;
        // the larger of the two, the valley rounded over about `round` mm
        (z + heel + ((z - heel).powi(2) + round * round).sqrt()) / 2.0
    };
    let end = crowns.last().map_or(0.0, |c| c.1 + c.2 / 2.0);
    model(valley - 9.0, end + 4.0, height)
}

fn segment(m: &Model) -> Vec<u16> {
    segmentation::run(
        &Input {
            arch: Arch::Upper,
            vertices: &m.v,
            tris: &m.t,
            seeds: &m.seeds,
            decimated: None,
        },
        &Params::default(),
    )
    .unwrap()
    .labels
}

/// How many of the vertices of the core of the crown of 16 (the points
/// within `core` of its diameters from its centre) are 16, and how many
/// vertices the core has.
fn core_of_16(m: &Model, labels: &[u16], core: f64) -> (usize, usize) {
    let (_, c, md, bl) = crowns()[0];
    let crown: Vec<usize> = (0..m.v.len())
        .filter(|&i| crown_e(m.v[i].x, m.v[i].y, c, md, bl) <= core)
        .collect();
    let of_16 = crown.iter().filter(|&&i| labels[i] == 16).count();
    (of_16, crown.len())
}

/// The heel as high as the crowns, lower or higher: past 3 mm behind the
/// distal surface of 16 (2 mm and a margin for the boundary refinement)
/// nothing is a tooth, while the core of the crown of 16 stays 16 (the
/// gingiva behind the last tooth does not cut it).
#[test]
fn the_base_behind_the_last_molar_is_gum() {
    for rise in [-0.5, 0.0, 1.5] {
        let (m, distal) = rim_heel(rise);
        let labels = segment(&m);
        let behind: Vec<usize> = (0..m.v.len())
            .filter(|&i| m.v[i].y < distal - 3.0 && labels[i] != 0)
            .collect();
        assert!(
            behind.is_empty(),
            "heel {rise} mm: {} vertices behind 16 in a tooth, the first at {:?} in {}",
            behind.len(),
            behind.first().map(|&i| m.v[i]),
            behind.first().map_or(0, |&i| labels[i])
        );
        let (of_16, all) = core_of_16(&m, &labels, 0.3);
        assert!(
            of_16 * 100 >= all * 95,
            "heel {rise} mm: only {of_16} of {all} vertices of the crown of 16 are 16"
        );
    }
}

/// The base of the test piece: the molar falls 1-3 mm to a valley, sharp or
/// rounded, and the heel rises behind it with no rim. Across the crown
/// (every column of the grid within 4 mm of its axis) the region of 16 ends
/// at the valley, at most 1 mm past it, and its crown stays 16.
#[test]
fn the_last_molar_ends_at_the_valley_before_the_heel() {
    let cases = [
        (1.0, 0.3, 0.0),
        (2.0, 0.3, 0.0),
        (3.0, 0.3, 0.0),
        (2.0, 0.6, 0.0),
        (1.0, 0.5, 1.5),
        (2.0, 0.5, 1.5),
        (3.0, 0.5, 1.5),
    ];
    for (drop, slope, round) in cases {
        let m = valley_heel(drop, slope, round);
        let labels = segment(&m);
        let what = format!("valley {drop} mm (rounded {round} mm), heel {slope} mm/mm");
        for (vs, ls) in m.v.chunks(m.ny).zip(labels.chunks(m.ny)) {
            let x = vs[0].x;
            if x.abs() > 4.0 {
                continue;
            }
            // the valley of this column: its lowest point behind the centre
            // of 16, off the open border of the model
            let Some(valley) = vs
                .iter()
                .filter(|p| p.y < 0.0 && p.y > vs[0].y + 1.0)
                .min_by(|a, b| a.z.total_cmp(&b.z))
            else {
                continue;
            };
            let last = vs
                .iter()
                .zip(ls)
                .filter(|&(_, &l)| l == 16)
                .map(|(p, _)| p.y)
                .fold(f64::INFINITY, f64::min);
            assert!(
                last >= valley.y - 1.0,
                "{what}, x {x}: 16 reaches y {last}, {:.2} mm past the valley at y {}",
                valley.y - last,
                valley.y
            );
        }
        let (of_16, all) = core_of_16(&m, &labels, 0.3);
        assert!(
            of_16 * 100 >= all * 95,
            "{what}: only {of_16} of {all} vertices of the crown of 16 are 16"
        );
    }
}
