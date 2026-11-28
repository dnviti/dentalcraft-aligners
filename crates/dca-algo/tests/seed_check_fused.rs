//! The check of the segmented teeth (`dca_algo::seed_check`) on a synthetic
//! upper arch whose teeth touch in shallow, fused contacts, as on the printed
//! test piece: the crowns run into each other with only a slight valley, so
//! the boundary between two neighbours has little fold and the plugin's
//! check reads it as weak. The plugin then merges real neighbours
//! (`plan_corrections`); the app never does: two teeth are merged only with
//! positive evidence that they are two parts of one crown
//! (`seed_check::vetted`). A crown that really carries two labels is still
//! merged.

use std::collections::BTreeMap;

use dc_case::Fdi;
use dca_algo::anatomy::{self, Arch};
use dca_algo::autoseed;
use dca_algo::demo;
use dca_algo::seed_check::{self, Action, OVER_CROWN_FOLD, Report};
use glam::{DVec2, DVec3};

/// Grid step of the mesh (mm).
const STEP: f64 = 0.3;
/// Half width of the band of the mesh across the arch (mm).
const BAND: f64 = 11.0;
/// Height of the crowns above the gum (mm).
const CROWN: f64 = 6.0;
/// Slope of the top of a crown at its contacts: the fused contacts have
/// folds of 0.46-1.98 (six of the eleven under `seed_check::WEAK_FOLD`).
const FUSED_SLOPE: f64 = 0.25;

/// One tooth of the model: its number, the middle and the half width of its
/// crown along the arch, the half width across.
struct Tooth {
    fdi: u16,
    mid: f64,
    half: f64,
    across: f64,
}

/// The upper teeth 16-26 side by side along the arch with their typical
/// widths, 11 and 21 meeting at the midline (arc length 0).
fn teeth() -> Vec<Tooth> {
    let mut out = Vec::new();
    for (q, sign) in [(1u8, -1.0), (2u8, 1.0)] {
        let mut s = 0.0;
        for i in 1..=6u8 {
            let f = Fdi::new(q * 10 + i).unwrap();
            let (md, bl) = anatomy::typical_diameters(f);
            out.push(Tooth {
                fdi: u16::from(f.get()),
                mid: sign * (s + 0.5 * md),
                half: 0.5 * md,
                across: 0.5 * bl,
            });
            s += md;
        }
    }
    out
}

/// A height-field model of an upper arch along the curve of the demo arch:
/// a ridge of gum, and on it the crowns of `teeth`, each with a top that
/// falls from its middle to its contacts with the slope `slope` there.
/// Neighbouring crowns meet at full height: the contact is fused, a
/// shallow valley. Returns the vertices, the triangles and the label of
/// every vertex (the tooth whose crown it lies on, 0 on the gum).
fn model(teeth: &[Tooth], slope: f64) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<u16>) {
    let fdis: Vec<Fdi> = teeth
        .iter()
        .map(|t| Fdi::new(t.fdi as u8).unwrap())
        .collect();
    let (curve, _) = demo::ideal_arch(Arch::Upper, &fdis);
    let (lo, hi) = curve.s.iter().map(|&s| curve.point(s)).fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(a, b), p| (a.min(p), b.max(p)),
    );
    let (lo, hi) = (lo - DVec2::splat(BAND), hi + DVec2::splat(BAND));
    let nx = ((hi.x - lo.x) / STEP) as usize + 1;
    let ny = ((hi.y - lo.y) / STEP) as usize + 1;
    let s_end = teeth
        .iter()
        .map(|t| t.mid.abs() + t.half)
        .fold(0.0, f64::max)
        + 4.0;
    let mut index = vec![u32::MAX; nx * ny];
    let (mut v, mut labels) = (Vec::new(), Vec::new());
    for i in 0..nx {
        for j in 0..ny {
            let p = lo + DVec2::new(i as f64, j as f64) * STEP;
            let s = curve.project(p);
            let o = (p - curve.point(s)).dot(curve.normal_out(s));
            if o.abs() > BAND || s.abs() > s_end {
                continue;
            }
            // a ridge of gum, and the crowns on it
            let gum = 2.0 - 0.08 * o * o;
            let mut z = gum;
            let mut label = 0;
            for t in teeth {
                let (u, w) = ((s - t.mid) / t.half, o / t.across);
                if u.abs() <= 1.0 && w.abs() <= 1.0 {
                    let top = CROWN - 0.5 * slope * t.half * u * u;
                    let h = gum + top * (1.0 - w.powi(6));
                    if h > z {
                        z = h;
                        label = t.fdi;
                    }
                }
            }
            index[i * ny + j] = v.len() as u32;
            v.push(p.extend(z));
            labels.push(label);
        }
    }
    let mut t = Vec::new();
    for i in 0..nx - 1 {
        for j in 0..ny - 1 {
            let [a, b, c, d] =
                [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)].map(|(x, y)| index[x * ny + y]);
            if [a, b, c, d].contains(&u32::MAX) {
                continue;
            }
            t.push([a, b, c]);
            t.push([a, c, d]);
        }
    }
    (v, t, labels)
}

fn report(v: &[DVec3], t: &[[u32; 3]], labels: &[u16]) -> Report {
    let data = autoseed::arch_data(v, t).expect("an arch");
    seed_check::check_teeth(v, t, labels, Arch::Upper, &data)
}

fn merges(acts: &[Action]) -> Vec<(u16, u16)> {
    acts.iter()
        .filter_map(|a| match *a {
            Action::Merge { keep, drop } => Some((keep, drop)),
            _ => None,
        })
        .collect()
}

#[test]
fn neighbours_with_fused_contacts_are_never_merged() {
    let teeth = teeth();
    let (v, t, labels) = model(&teeth, FUSED_SLOPE);
    let rep = report(&v, &t, &labels);
    let folds: BTreeMap<(u16, u16), f64> = rep.weak.iter().map(|w| ((w.a, w.b), w.fold)).collect();
    // the plugin's check reads the fused contacts as boundaries with no
    // groove, some of them with as little fold as a crown split in two
    assert!(rep.weak.len() >= 4, "{folds:?}");
    assert!(
        rep.weak.iter().any(|w| w.fold < OVER_CROWN_FOLD),
        "{folds:?}"
    );
    assert!(rep.slivers.is_empty() && rep.wide.is_empty(), "{rep:?}");
    // and its plan merges real neighbours (the defect on the test piece)
    let plugin = merges(&seed_check::plan_corrections(&rep));
    assert!(plugin.len() >= 2, "{plugin:?} {folds:?}");
    // the app keeps every tooth: no weak boundary is over one crown
    for w in &rep.weak {
        assert!(!seed_check::one_crown(&rep, w, Arch::Upper), "{w:?}");
    }
    let vetted = seed_check::vetted(&rep, Arch::Upper);
    assert!(vetted.weak.is_empty(), "{:?}", vetted.weak);
    assert!(
        seed_check::plan_corrections(&vetted).is_empty(),
        "{:?}",
        seed_check::plan_corrections(&vetted)
    );
}

#[test]
fn a_crown_carrying_two_labels_is_still_merged() {
    let teeth = teeth();
    let (v, t, mut labels) = model(&teeth, FUSED_SLOPE);
    // 16 carries a second label (17) on its distal half
    let s16 = teeth.iter().find(|x| x.fdi == 16).unwrap();
    let fdis: Vec<Fdi> = teeth
        .iter()
        .map(|t| Fdi::new(t.fdi as u8).unwrap())
        .collect();
    let (curve, _) = demo::ideal_arch(Arch::Upper, &fdis);
    for (p, l) in v.iter().zip(labels.iter_mut()) {
        if *l == 16 && curve.project(p.truncate()) < s16.mid {
            *l = 17;
        }
    }
    let rep = report(&v, &t, &labels);
    let split = rep
        .weak
        .iter()
        .find(|w| (w.a, w.b) == (16, 17))
        .expect("16|17 weak");
    assert!(seed_check::one_crown(&rep, split, Arch::Upper), "{split:?}");
    // only the two parts of 16 are merged, under the number nearer the midline
    let plan = seed_check::plan_corrections(&seed_check::vetted(&rep, Arch::Upper));
    assert_eq!(merges(&plan), vec![(16, 17)], "{plan:?}");
}
