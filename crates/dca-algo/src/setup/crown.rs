//! What the setup reads from the scanned crowns: the prominent cusps
//! (`planning.cusp_points`), the crown points above the gingival margin
//! (`case_plan.rest_crowns`), the incisal-edge direction of incisors and
//! canines (`front_axes`), the extent of a crown along a direction and the
//! clearance between two crowns (`meshutil.facing_clearance`).

use std::collections::{BTreeMap, HashMap};

use dc_case::Fdi;
use glam::{DVec2, DVec3};

use super::{Tooth, ToothInput};
use crate::anatomy;
use crate::teeth::percentile;

/// Depth of the occlusal band (mm) whose plan direction is the incisal edge
/// (incisors) or the ridges of the canine, by tooth index.
fn edge_depth(i: u8) -> Option<f64> {
    match i {
        1 | 2 => Some(1.0),
        3 => Some(2.0),
        _ => None,
    }
}
/// Smallest elongation of the band (square root of the eigenvalue ratio).
const EDGE_MIN_ELONG: f64 = 1.4;
/// Degrees: beyond this turn from the segmentation axis the band is not
/// believed.
const EDGE_MAX_TURN: f64 = 60.0;

fn mean(p: &[DVec3]) -> DVec3 {
    p.iter().copied().sum::<DVec3>() / p.len() as f64
}

fn unit(v: DVec3) -> DVec3 {
    v / v.length().max(1e-12)
}

/// Index of the first largest value (`numpy.argmax`).
fn argmax(v: impl Iterator<Item = f64>) -> usize {
    let (mut best, mut bv) = (0, f64::NEG_INFINITY);
    for (i, x) in v.enumerate() {
        if i == 0 || x > bv {
            (best, bv) = (i, x);
        }
    }
    best
}

/// Prominent cusps of a tooth (arch coordinates, at rest) from its crown
/// points `p`: incisors the two ends of the incisal edge (mesial, distal),
/// canine its tip, premolars the buccal cusp, molars the mesiobuccal and
/// distobuccal cusps (mesial, distal).
pub fn cusp_points(t: &ToothInput, p: &[DVec3]) -> Vec<DVec3> {
    if p.len() < 20 {
        return vec![DVec3::new(t.reference.x, t.reference.y, t.occ_z)];
    }
    let c = t.centroid;
    let pax: Vec<f64> = p.iter().map(|q| (*q - c).dot(t.e_ax)).collect();
    let pbl: Vec<f64> = p.iter().map(|q| (*q - c).dot(t.e_bl)).collect();
    let pm: Vec<f64> = p.iter().map(|q| (*q - c).dot(t.e_md) * t.md_sign).collect();
    let i = anatomy::index(t.fdi);
    let highest = || p[argmax(pax.iter().copied())];
    if i <= 2 {
        let top_h = pax.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 1.0;
        let top: Vec<usize> = (0..p.len()).filter(|&k| pax[k] >= top_h).collect();
        if top.len() < 6 {
            return vec![highest()];
        }
        let mq: Vec<f64> = top.iter().map(|&k| pm[k]).collect();
        let (lo, hi) = (percentile(&mq, 20.0), percentile(&mq, 80.0));
        let mut out = Vec::new();
        for mesial in [true, false] {
            let sel: Vec<usize> = top
                .iter()
                .zip(&mq)
                .filter(|(_, m)| if mesial { **m >= hi } else { **m <= lo })
                .map(|(k, _)| *k)
                .collect();
            let hs: Vec<f64> = sel.iter().map(|&k| pax[k]).collect();
            let h50 = percentile(&hs, 50.0);
            let pts: Vec<DVec3> = sel
                .iter()
                .filter(|&&k| pax[k] >= h50)
                .map(|&k| p[k])
                .collect();
            out.push(mean(&pts));
        }
        return out;
    }
    if i == 3 {
        return vec![highest()];
    }
    let mut buccal: Vec<bool> = pbl.iter().map(|&b| b > 0.0).collect();
    if buccal.iter().filter(|&&b| b).count() < 10 {
        buccal = vec![true; p.len()];
    }
    let pick = |idx: &[usize]| p[idx[argmax(idx.iter().map(|&k| pax[k]))]];
    let bidx: Vec<usize> = (0..p.len()).filter(|&k| buccal[k]).collect();
    if i <= 5 {
        return vec![pick(&bidx)];
    }
    // the highest point of the mesial and of the distal third of the buccal half
    let bm: Vec<f64> = bidx.iter().map(|&k| pm[k]).collect();
    let (lo, hi) = (percentile(&bm, 33.0), percentile(&bm, 67.0));
    [true, false]
        .iter()
        .map(|&mesial| {
            let idx: Vec<usize> = bidx
                .iter()
                .copied()
                .filter(|&k| if mesial { pm[k] >= hi } else { pm[k] <= lo })
                .collect();
            if idx.is_empty() {
                pick(&bidx)
            } else {
                pick(&idx)
            }
        })
        .collect()
}

/// Crown points at rest above the gingival margin (0.5 mm above the mean
/// margin height along the long axis).
pub fn rest_crown(t: &ToothInput, p: &[DVec3]) -> Vec<DVec3> {
    let h: Vec<f64> = p.iter().map(|q| (*q - t.centroid).dot(t.e_ax)).collect();
    let mh = if t.margin.is_empty() {
        h.iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        t.margin
            .iter()
            .map(|m| (*m - t.centroid).dot(t.e_ax))
            .sum::<f64>()
            / t.margin.len() as f64
    };
    p.iter()
        .zip(&h)
        .filter(|(_, hh)| **hh > mh + 0.5)
        .map(|(q, _)| *q)
        .collect()
}

/// Main plan direction (3D, orthogonal to `e_ax`) of the occlusal band
/// `depth` mm high of the crown points, and the points of the band; `None`
/// when the band is small or not elongated.
pub(crate) fn edge_direction(p: &[DVec3], e_ax: DVec3, depth: f64) -> Option<(DVec3, Vec<DVec3>)> {
    let e_ax = unit(e_ax);
    let h: Vec<f64> = p.iter().map(|q| q.dot(e_ax)).collect();
    let hmax = h.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let top: Vec<DVec3> = p
        .iter()
        .zip(&h)
        .filter(|(_, hh)| **hh >= hmax - depth)
        .map(|(q, _)| *q)
        .collect();
    if top.len() < 12 {
        return None;
    }
    let m = top.iter().map(|q| q.truncate()).sum::<DVec2>() / top.len() as f64;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for q in &top {
        let d = q.truncate() - m;
        sxx += d.x * d.x;
        sxy += d.x * d.y;
        syy += d.y * d.y;
    }
    let (w0, w1, v) = eig2(sxx, sxy, syy);
    if (w1 / w0.max(1e-12)).sqrt() < EDGE_MIN_ELONG {
        return None;
    }
    let d = DVec3::new(v.x, v.y, 0.0);
    let d = d - d.dot(e_ax) * e_ax;
    Some((unit(d), top))
}

/// Eigenvalues (ascending) of the symmetric 2x2 matrix `[[a, b], [b, c]]` and
/// the unit eigenvector of the larger one.
fn eig2(a: f64, b: f64, c: f64) -> (f64, f64, DVec2) {
    let m = 0.5 * (a + c);
    let r = (0.25 * (a - c) * (a - c) + b * b).sqrt();
    let (w0, w1) = (m - r, m + r);
    let v = if b == 0.0 {
        if a >= c { DVec2::X } else { DVec2::Y }
    } else {
        let v1 = DVec2::new(w1 - c, b);
        let v2 = DVec2::new(b, w1 - a);
        if v1.length_squared() >= v2.length_squared() {
            v1
        } else {
            v2
        }
    };
    (w0, w1, v.normalize())
}

/// Incisors and canines with the mesio-distal direction from the crown
/// (`edge_direction`), the contact points at the ends of the crown along it
/// (above 30% of the height) and, for the incisors, the ends of the incisal
/// edge as cusps. Teeth whose band is not reliable stay as they are.
pub(crate) fn front_axes(
    teeth: &mut BTreeMap<Fdi, Tooth>,
    cusps: &mut BTreeMap<Fdi, Vec<DVec3>>,
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
) {
    for (&f, p) in crowns {
        let i = anatomy::index(f);
        let (Some(g), Some(depth)) = (teeth.get(&f), edge_depth(i)) else {
            continue;
        };
        let e_ax = unit(g.e_ax);
        let sign = g.md_sign;
        let old = g.e_md * sign;
        let Some((mut d, top)) = edge_direction(p, e_ax, depth) else {
            continue;
        };
        if d.truncate().dot(old.truncate()) < 0.0 {
            d = -d;
        }
        let cos = d.truncate().dot(old.truncate()) / old.truncate().length().max(1e-12);
        if cos.clamp(-1.0, 1.0).acos().to_degrees() > EDGE_MAX_TURN {
            continue;
        }
        let h: Vec<f64> = p.iter().map(|q| q.dot(e_ax)).collect();
        let (hmin, hmax) = h
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                (a.min(x), b.max(x))
            });
        let up: Vec<DVec3> = p
            .iter()
            .zip(&h)
            .filter(|(_, hh)| **hh >= hmin + 0.3 * (hmax - hmin))
            .map(|(q, _)| *q)
            .collect();
        let pm: Vec<f64> = up.iter().map(|q| q.dot(d)).collect();
        // the ends of the crown (at 1% and 99%: a few segmentation points beyond the contact
        // do not count)
        let end = |e: f64| -> DVec3 {
            let sel: Vec<DVec3> = up
                .iter()
                .zip(&pm)
                .filter(|(_, x)| **x >= e - 0.15 && **x <= e + 0.15)
                .map(|(q, _)| *q)
                .collect();
            let c = mean(&sel);
            c + d * (e - c.dot(d))
        };
        let (cm, cd) = (end(percentile(&pm, 99.0)), end(percentile(&pm, 1.0)));
        let tooth = Tooth {
            e_md: d * sign,
            contact_m: cm,
            contact_d: cd,
            width_md: (cm - cd).truncate().length(),
            edge_md: true,
            ..g.clone()
        };
        teeth.insert(f, tooth);
        if i <= 2 {
            let q: Vec<f64> = top.iter().map(|x| x.dot(d)).collect();
            let (lo, hi) = (percentile(&q, 20.0), percentile(&q, 80.0));
            let mut ends = Vec::new();
            for mesial in [true, false] {
                let sel: Vec<DVec3> = top
                    .iter()
                    .zip(&q)
                    .filter(|(_, x)| if mesial { **x >= hi } else { **x <= lo })
                    .map(|(x, _)| *x)
                    .collect();
                let hs: Vec<f64> = sel.iter().map(|x| x.dot(e_ax)).collect();
                let h50 = percentile(&hs, 50.0);
                let pts: Vec<DVec3> = sel
                    .iter()
                    .zip(&hs)
                    .filter(|(_, hh)| **hh >= h50)
                    .map(|(x, _)| *x)
                    .collect();
                ends.push(mean(&pts));
            }
            cusps.insert(f, ends);
        }
    }
}

/// Extent (mm) of the plan points along the plan direction `u`, between the
/// 1st and the 99th percentile.
pub(crate) fn extent(pq: &[DVec2], u: DVec2) -> f64 {
    let q: Vec<f64> = pq.iter().map(|p| p.dot(u)).collect();
    percentile(&q, 99.0) - percentile(&q, 1.0)
}

/// Signed clearance between two point-sampled crowns along the unit
/// direction `u` (a to b): the minimum over the cells of a 0.35 mm grid on
/// the plane orthogonal to `u` of (lowest b - highest a) along `u`.
/// Negative: interpenetration. Infinite when they do not face each other.
pub(crate) fn contact_separation(pa: &[DVec3], pb: &[DVec3], u: DVec3) -> f64 {
    const CELL: f64 = 0.35;
    if pa.is_empty() || pb.is_empty() {
        return f64::INFINITY;
    }
    let up = DVec3::Z;
    let v = up - up.dot(u) * u;
    let vn = v.length();
    let v = if vn == 0.0 { v } else { v / vn };
    let w = u.cross(v);
    let key = |p: &DVec3| -> i64 {
        ((p.dot(v) / CELL).floor() as i64)
            .wrapping_mul(1_000_003)
            .wrapping_add((p.dot(w) / CELL).floor() as i64)
    };
    let mut top_a: HashMap<i64, f64> = HashMap::with_capacity(pa.len());
    for p in pa {
        let e = top_a.entry(key(p)).or_insert(f64::NEG_INFINITY);
        *e = e.max(p.dot(u));
    }
    let mut low_b: HashMap<i64, f64> = HashMap::with_capacity(pb.len());
    for p in pb {
        let e = low_b.entry(key(p)).or_insert(f64::INFINITY);
        *e = e.min(p.dot(u));
    }
    let mut best = f64::INFINITY;
    for (k, a) in &top_a {
        if let Some(b) = low_b.get(k) {
            best = best.min(b - a);
        }
    }
    best
}

/// Clearance between two crowns (arch coordinates) along the horizontal
/// direction joining their centroids, with the facing halves only.
pub(crate) fn facing_clearance(pa: &[DVec3], pb: &[DVec3]) -> f64 {
    if pa.is_empty() || pb.is_empty() {
        return f64::INFINITY;
    }
    let (ca, cb) = (mean(pa), mean(pb));
    let mut u = cb - ca;
    u.z = 0.0;
    if u.length() < 1e-9 {
        return f64::INFINITY;
    }
    let u = u / u.length();
    let fa: Vec<DVec3> = pa
        .iter()
        .copied()
        .filter(|p| (*p - ca).dot(u) > 0.0)
        .collect();
    let fb: Vec<DVec3> = pb
        .iter()
        .copied()
        .filter(|p| (*p - cb).dot(u) < 0.0)
        .collect();
    contact_separation(&fa, &fb, u)
}
