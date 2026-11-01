//! Automatic seeds: finds the teeth of an arch and puts a numbered seed point
//! (FDI) at the centre of the occlusal face of each, without clicks. The
//! proposal must be checked by the user.
//!
//! Reference: `ortho_aligner/autoseed.py` (with `case_auto.auto_seeds` and
//! `api.auto_seeds`, which call [`propose`] on the scan of the case).
//!
//! 1. Occlusal direction ([`occlusal_axis`]): smallest principal axis of
//!    the scan; its sign from the open border (intraoral scans) or from the
//!    flat base (plaster models).
//! 2. Arch orientation ([`arch_frame`]): the crown tops form a U; the
//!    rotation that best fits a symmetric 4th-order polynomial (AlHarbi
//!    2008) gives the anterior direction.
//! 3. Cusp profile along the arch ([`arch_profiles`]): highest crown point
//!    for every 0.25 mm of arc, crown score (enamel texture at the occlusal
//!    level) to find the arch ends, edentulous ridges and the spaces of
//!    missing teeth.
//! 4. Tooth boundaries by dynamic programming ([`dp`]): segment lengths
//!    follow typical mesio-distal widths (Wheeler) scaled to the arch,
//!    boundaries lie in the interproximal valleys of the cusp profile; a
//!    tooth may be missing inside the arch (its space, when not closed, is
//!    gingiva) and the arch may end at any molar ([`propose`]).
//! 5. Upper / lower arch without cues in the file ([`detect_arch`]): file
//!    name, scanner frame, palate, side of the highest cusps of the last
//!    molars (curve of Wilson), tooth-width pattern.
//!
//! [`orient`] puts the patient on the world axes like [`crate::frame`], with
//! the arch frame of step 2 when the arch has no seeds yet.
//!
//! Port notes: the computation follows the plugin step by step in `f64`,
//! including numpy's rounding (half to even), ranges and percentile
//! interpolation. The nearest arch-curve sample is exact (ties to the
//! smallest index), as the plugin's `gpu_query.knn` returns it. The arch
//! sequence candidates and their scales are evaluated in parallel and
//! reduced in the plugin's order, so the result does not depend on the
//! thread count. Diagnostic texts (Italian in the plugin) are enums here,
//! and costs are not rounded.

mod dp;
mod nearest;
mod numpy;

use std::collections::BTreeMap;

use dc_case::{Fdi, Pose};
use dca_model::Seed;
use glam::{DMat4, DVec2, DVec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use dp::{DpProblem, dp};

use crate::anatomy::{self, Arch};
use crate::archform::ArchCurve;
use crate::frame::{self, ArchFrame};
use numpy::{
    arange, fill_nan, linspace, lstsq3, mean, median, moving_average, moving_sum_same, percentile,
    percentile_sorted, subsample,
};

// ----------------------------------------------------------------------------
// parameters (arc length and heights in mm; costs in -2 ln likelihood units)
// ----------------------------------------------------------------------------

/// Normal dispersion below which the surface is smooth (gingiva).
pub const TEXTURE_GUM: f64 = 0.17;
/// Texture ramp from smooth gingiva (0) to enamel-like crown (1).
pub const CROWN_TEXTURE: (f64, f64) = (0.10, 0.17);
/// mm below the occlusal plane of the crowns: edentulous ridge (0) .. crown (1).
pub const CROWN_LOW: (f64, f64) = (-5.5, -4.0);
/// mm of margin beyond the crown-detected arch ends.
pub const END_ZONE: f64 = 2.0;
/// Relative spread of the crown widths along the arch around the typical
/// (Wheeler) ones after the global scale, per tooth kind: SD 0.05-0.12 on
/// the calibration patients; the biological CV of mesio-distal diameters is
/// 6-9% (Moyers 1976). Incisors get more room (rotations and crowding
/// shorten them along the arch).
pub fn width_sigma(fdi: Fdi) -> f64 {
    match anatomy::index(fdi) {
        1 | 2 => 0.14,
        _ => 0.10,
    }
}
/// Weak midline anchor (mm): the dental midline often deviates 2-4 mm from
/// the middle of the anterior crowns, a whole incisor (5-8.5 mm) rarely.
pub const MIDLINE_SIGMA: f64 = 4.0;
/// Interproximal valley depth (mm) of the boundary cost table.
pub const VALLEY_MM: [f64; 6] = [0.0, 0.3, 0.7, 1.0, 1.5, 2.2];
/// -2 ln of the likelihood ratio of the valley depth at a contact between
/// two teeth vs inside a crown (calibration patients: 101 contacts, 1829
/// crown bins). A boundary where the crown top is flat costs, a deep valley
/// pays.
pub const VALLEY_COST: [f64; 6] = [6.5, 2.0, 1.0, -2.5, -6.5, -8.0];
/// Per mm of smooth or low (gingiva-like) surface inside a tooth.
pub const NONCROWN_COST: f64 = 3.0;
/// Per mm of crown-like surface inside a space left by a missing tooth or a
/// diastema.
pub const GAP_COST: f64 = 3.0;
/// Prior cost of a missing tooth inside the arch (about a 5% probability).
pub const MISSING_COST: f64 = 6.0;
/// The space of a missing tooth is at most this fraction of its width.
pub const MISSING_GAP: f64 = 1.3;
/// Interdental space allowed before a tooth (diastema, spacing), mm.
pub const DIASTEMA_MM: f64 = 6.0;
/// Cost of a diastema.
pub const DIASTEMA_COST: f64 = 1.0;
/// Upper / lower arch from the segmented crowns: central incisor / first
/// molar width (about 0.85 upper, 0.45 lower); measured as lower arch,
/// `>= RATIO_UPPER` means upper.
pub const RATIO_UPPER: f64 = 0.62;
/// Distal length of each side of the arch examined for the curve of Wilson.
pub const WILSON_MM: f64 = 12.0;
/// mm: below this the side of the highest molar cusps is not decisive.
pub const WILSON_MIN: f64 = 0.3;
/// mm: buccal molar cusps this much higher overrule an apparent palate (the
/// closed base of a lower plaster model).
pub const WILSON_OVER_PALATE: f64 = 0.6;
/// Depth of the band of crown tops below the highest point (mm).
pub const BAND_DEPTH: f64 = 6.0;
/// Bin size of the profiles along the arch (mm).
pub const BIN_MM: f64 = 0.25;
/// At least this many seeds, or the proposal is rejected (`case_auto`).
pub const MIN_SEEDS: usize = 4;

/// Errors of the automatic seeds.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AutoseedError {
    /// The mesh has no triangles or is flat in every direction.
    #[error("the scan is empty or damaged")]
    DegenerateMesh,
    /// The crowns do not form an arch.
    #[error("arch not found")]
    ArchNotFound,
    /// Fewer than [`MIN_SEEDS`] teeth found.
    #[error("the teeth could not be found ({0} found): mark them by hand")]
    TooFewSeeds(usize),
    /// The forced arch ends are not molars or second premolars (5..=8).
    #[error("the distal tooth of each side must be 5, 6, 7 or 8")]
    BadEnds,
    /// A missing tooth is not a valid FDI number.
    #[error("not a valid FDI tooth number: {0}")]
    BadTooth(u8),
}

// ----------------------------------------------------------------------------
// mesh helpers (meshutil)
// ----------------------------------------------------------------------------

/// Vertices on the open border: edges not shared by exactly two triangles
/// (`meshutil.boundary_vertex_mask`).
fn boundary_vertex_mask(t: &[[u32; 3]], n_verts: usize) -> Vec<bool> {
    let mut edges: Vec<(u32, u32)> = t
        .iter()
        .flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])].map(|(a, b)| (a.min(b), a.max(b))))
        .collect();
    edges.sort_unstable();
    let mut mask = vec![false; n_verts];
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j] == edges[i] {
            j += 1;
        }
        if j - i != 2 {
            mask[edges[i].0 as usize] = true;
            mask[edges[i].1 as usize] = true;
        }
        i = j;
    }
    mask
}

/// Area-weighted vertex normals, unit length (zero for isolated vertices).
fn vertex_normals(v: &[DVec3], t: &[[u32; 3]]) -> Vec<DVec3> {
    let mut vn = vec![DVec3::ZERO; v.len()];
    for f in t {
        let [a, b, c] = f.map(|i| v[i as usize]);
        let cr = (b - a).cross(c - a);
        for i in f {
            vn[*i as usize] += cr;
        }
    }
    for n in &mut vn {
        let l = n.length();
        if l != 0.0 {
            *n /= l;
        }
    }
    vn
}

/// `meshutil.normalize` of one vector (a zero vector stays zero).
fn normalize(v: DVec3) -> DVec3 {
    let l = v.length();
    if l == 0.0 { v } else { v / l }
}

fn transform(m: &DMat4, p: DVec3) -> DVec3 {
    m.transform_point3(p)
}

/// Indices sorted by `(x, y, z)` (`numpy.lexsort((z, y, x))`, stable).
fn lexsort_xyz(v: &[DVec3]) -> Vec<usize> {
    let mut o: Vec<usize> = (0..v.len()).collect();
    o.sort_by(|&a, &b| {
        let (p, q) = (v[a], v[b]);
        p.x.total_cmp(&q.x)
            .then(p.y.total_cmp(&q.y))
            .then(p.z.total_cmp(&q.z))
    });
    o
}

/// `round()` of Python and numpy: half to even.
fn round_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

// ----------------------------------------------------------------------------
// arch frame
// ----------------------------------------------------------------------------

/// Occlusal direction and centroid of a scan: smallest principal axis,
/// pointing away from the open border (intraoral scans, more than 20
/// border vertices) or, for a closed model, away from its flat base.
pub fn occlusal_axis(v: &[DVec3], t: &[[u32; 3]]) -> Result<(DVec3, DVec3), AutoseedError> {
    if v.len() < 3 || t.is_empty() {
        return Err(AutoseedError::DegenerateMesh);
    }
    let pca = frame::pca(v).ok_or(AutoseedError::DegenerateMesh)?;
    let (mut n, c) = (pca.axes[2], pca.mean);
    let bmask = boundary_vertex_mask(t, v.len());
    let nbound = bmask.iter().filter(|b| **b).count();
    if nbound > 20 {
        let mb = v
            .iter()
            .zip(&bmask)
            .filter(|(_, b)| **b)
            .map(|(p, _)| *p)
            .sum::<DVec3>()
            / nbound as f64;
        if (mb - c).dot(n) > 0.0 {
            n = -n;
        }
        return Ok((n, c));
    }
    let (mut a_plus, mut a_minus) = (0.0, 0.0);
    for f in t {
        let [a, b, cc] = f.map(|i| v[i as usize]);
        let cr = (b - a).cross(cc - a);
        let area2 = cr.length();
        let nf = cr / if area2 > 0.0 { area2 } else { 1.0 };
        let d = nf.dot(n);
        if d > 0.9 {
            a_plus += 0.5 * area2;
        } else if d < -0.9 {
            a_minus += 0.5 * area2;
        }
    }
    Ok((if a_minus > a_plus { n } else { -n }, c))
}

/// RMS residual and coefficients of the least-squares fit
/// `y = c0 + c1 x² + c2 x⁴`.
fn fit_resid(q: &[DVec2]) -> (f64, [f64; 3]) {
    let rows: Vec<[f64; 3]> = q.iter().map(|p| [1.0, p.x * p.x, p.x.powi(4)]).collect();
    let rhs: Vec<f64> = q.iter().map(|p| p.y).collect();
    let sol = lstsq3(&rows, &rhs);
    let ss: f64 = rows
        .iter()
        .zip(&rhs)
        .map(|(r, y)| {
            let e = r[0] * sol[0] + r[1] * sol[1] + r[2] * sol[2] - y;
            e * e
        })
        .sum();
    ((ss / q.len() as f64).sqrt(), sol)
}

/// The arch frame of a scan (4x4, columns X Y Z origin, in the scan's
/// coordinates): Z occlusal ([`occlusal_axis`]), Y towards the incisors (the
/// rotation of the crown band that best fits an open-backwards symmetric
/// quartic), X = Y × Z; the origin on the midline of the most anterior
/// crowns, at the level of the most posterior crowns, on the plane of the
/// highest crown tops.
pub fn arch_frame(v: &[DVec3], t: &[[u32; 3]], band_depth: f64) -> Result<DMat4, AutoseedError> {
    let (n, c) = occlusal_axis(v, t)?;
    let h: Vec<f64> = v.iter().map(|p| p.dot(n)).collect();
    let top = percentile(&h, 99.5);
    let mut band: Vec<DVec3> = v
        .iter()
        .zip(&h)
        .filter(|(_, hh)| **hh >= top - band_depth)
        .map(|(p, _)| *p)
        .collect();
    if band.len() > 20000 {
        // canonical (spatial) order, not the file's vertex order
        let sorted: Vec<DVec3> = lexsort_xyz(&band).into_iter().map(|i| band[i]).collect();
        band = subsample(&sorted, 20000);
    }
    let a = normalize(if n.x.abs() < 0.9 {
        n.cross(DVec3::X)
    } else {
        n.cross(DVec3::Y)
    });
    let b = n.cross(a);
    let mut p: Vec<DVec2> = band
        .iter()
        .map(|q| DVec2::new((*q - c).dot(a), (*q - c).dot(b)))
        .collect();
    let pm = p.iter().copied().sum::<DVec2>() / p.len() as f64;
    for q in &mut p {
        *q -= pm;
    }
    let fit_at = |deg: f64| {
        let (s, co) = deg.to_radians().sin_cos();
        let q: Vec<DVec2> = p
            .iter()
            .map(|v| DVec2::new(co * v.x - s * v.y, s * v.x + co * v.y))
            .collect();
        fit_resid(&q)
    };
    let coarse: Vec<(f64, [f64; 3])> = (0..72)
        .into_par_iter()
        .map(|k| fit_at((5 * k) as f64))
        .collect();
    let mut best: Option<(f64, f64)> = None;
    for (k, (res, sol)) in coarse.iter().enumerate() {
        if sol[1] < 0.0 && best.is_none_or(|b| *res < b.0) {
            best = Some((*res, (5 * k) as f64));
        }
    }
    let deg0 = best.map_or(0.0, |b| b.1);
    let fine_degs = arange(deg0 - 5.0, deg0 + 5.01, 0.5);
    let fine: Vec<(f64, [f64; 3])> = fine_degs.par_iter().map(|d| fit_at(*d)).collect();
    for (d, (res, sol)) in fine_degs.iter().zip(&fine) {
        if sol[1] < 0.0 && best.is_none_or(|b| *res < b.0) {
            best = Some((*res, *d));
        }
    }
    let (_, deg) = best.ok_or(AutoseedError::ArchNotFound)?;
    let (s, co) = deg.to_radians().sin_cos();
    // rotated 2D axes: x' = cos x - sin y, y' = sin x + cos y; anterior = +y'
    let y = normalize(s * a + co * b);
    let z = n;
    let x = y.cross(z);
    let ctr = band.iter().copied().sum::<DVec3>() / band.len() as f64;
    let yb: Vec<f64> = band.iter().map(|q| (*q - ctr).dot(y)).collect();
    let xb: Vec<f64> = band.iter().map(|q| (*q - ctr).dot(x)).collect();
    // midline from the most anterior crowns (the posterior extent of a scan
    // is often asymmetric), origin at the level of the most posterior crowns
    let ymax = yb.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let front: Vec<f64> = xb
        .iter()
        .zip(&yb)
        .filter(|(_, yy)| **yy > ymax - 5.0)
        .map(|(xx, _)| *xx)
        .collect();
    let x_mid = mean(&front);
    let mut origin = ctr + x * x_mid + y * percentile(&yb, 1.0);
    origin = origin - (origin - ctr).dot(z) * z + z * (top - ctr.dot(z));
    Ok(frame::make_frame(x, y, z, origin))
}

// ----------------------------------------------------------------------------
// arch curve and profiles
// ----------------------------------------------------------------------------

/// Arc length of the nearest curve point, the curve sampled every `step` mm
/// (`autoseed._project_s`).
pub fn project_s(curve: &ArchCurve, p2: &[DVec2], step: f64) -> Vec<f64> {
    let ss = arange(curve.s[0], *curve.s.last().unwrap_or(&0.0), step);
    if ss.is_empty() {
        return vec![curve.s[0]; p2.len()];
    }
    let q: Vec<DVec2> = ss.iter().map(|s| curve.point(*s)).collect();
    let index = nearest::Nearest::new(q);
    index.nearest_all(p2).into_iter().map(|j| ss[j]).collect()
}

/// The teeth of an arch in the order of increasing arch-frame x (the
/// patient's right in the lower arch, left in the upper).
pub fn expected_sequence(arch: Arch, include_third: bool, missing: &[Fdi]) -> Vec<Fdi> {
    let order: Vec<Fdi> = anatomy::arch_order(arch)
        .into_iter()
        .filter(|f| (include_third || anatomy::index(*f) != 8) && !missing.contains(f))
        .collect();
    if arch == Arch::Upper {
        order
    } else {
        order.into_iter().rev().collect()
    }
}

/// Straight extensions along the end tangents (`archform._extend`).
fn extend(p: &[DVec2], length: f64, n: usize) -> Vec<DVec2> {
    let len = p.len();
    let mut t0 = p[0] - p[5.min(len - 1)];
    let mut t1 = p[len - 1] - p[len.saturating_sub(6)];
    t0 /= t0.length().max(1e-12);
    t1 /= t1.length().max(1e-12);
    let ks: Vec<f64> = linspace(0.0, length, n)[1..].to_vec();
    let mut out = Vec::with_capacity(len + 2 * ks.len());
    out.extend(ks.iter().rev().map(|&k| p[0] + k * t0));
    out.extend_from_slice(p);
    out.extend(ks.iter().map(|&k| p[len - 1] + k * t1));
    out
}

/// Arch curve through the crown band (arch coordinates), also for
/// asymmetric arches: median of the crown points in 2-degree sectors around
/// a point inside the U (the sector branch cut lies in the posterior
/// opening of the arch), smoothed, extended by 8 mm at both ends.
pub fn arch_polyline(b2: &[DVec2]) -> Result<ArchCurve, AutoseedError> {
    const BIN_DEG: f64 = 2.0;
    const SMOOTH: usize = 3;
    const EXTEND: f64 = 8.0;
    if b2.is_empty() {
        return Err(AutoseedError::ArchNotFound);
    }
    let mut xs: Vec<f64> = b2.iter().map(|p| p.x).collect();
    let mut ys: Vec<f64> = b2.iter().map(|p| p.y).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let x0 = 0.5 * (percentile_sorted(&xs, 2.0) + percentile_sorted(&xs, 98.0));
    let (y_lo, y_hi) = (percentile_sorted(&ys, 2.0), percentile_sorted(&ys, 98.0));
    let yc = y_lo + 0.35 * (y_hi - y_lo);
    let mut groups: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, p) in b2.iter().enumerate() {
        let key = round_even((p.x - x0).atan2(p.y - yc).to_degrees() / BIN_DEG);
        groups.entry(key).or_default().push(i);
    }
    let mut pts: Vec<DVec2> = groups
        .values()
        .filter(|g| g.len() >= 8)
        .map(|g| {
            let gx: Vec<f64> = g.iter().map(|&i| b2[i].x).collect();
            let gy: Vec<f64> = g.iter().map(|&i| b2[i].y).collect();
            DVec2::new(median(&gx), median(&gy))
        })
        .collect();
    if pts.len() < 10 {
        return Err(AutoseedError::ArchNotFound);
    }
    let sx = moving_average(&pts.iter().map(|p| p.x).collect::<Vec<_>>(), SMOOTH);
    let sy = moving_average(&pts.iter().map(|p| p.y).collect::<Vec<_>>(), SMOOTH);
    for (i, p) in pts.iter_mut().enumerate() {
        *p = DVec2::new(sx[i], sy[i]);
    }
    Ok(ArchCurve::new(extend(
        &pts,
        EXTEND,
        (EXTEND / 0.2) as usize + 1,
    )))
}

/// Scan points near the arch curve (`autoseed.arch_points`).
#[derive(Debug, Clone, Default)]
pub struct ArchPoints {
    /// Points (arch coordinates).
    pub points: Vec<DVec3>,
    /// Their unit normals.
    pub normals: Vec<DVec3>,
    /// Arc length of the nearest curve point.
    pub s: Vec<f64>,
    /// Bucco-lingual offset from the curve (+ buccal).
    pub off: Vec<f64>,
    /// Vertex index in the scan.
    pub vertex: Vec<usize>,
}

/// Scan points within `max_off` mm of the arch curve (at most about
/// `max_points`, subsampled in a canonical order: the same points whatever
/// the vertex order of the file).
pub fn arch_points(
    va: &[DVec3],
    na: &[DVec3],
    curve: &ArchCurve,
    max_off: f64,
    max_points: usize,
) -> ArchPoints {
    let step = (va.len() / max_points).max(1);
    let o: Vec<usize> = if step > 1 {
        lexsort_xyz(va).into_iter().step_by(step).collect()
    } else {
        (0..va.len()).collect()
    };
    let xy: Vec<DVec2> = o.iter().map(|&i| va[i].truncate()).collect();
    let s = project_s(curve, &xy, 0.05);
    let mut out = ArchPoints::default();
    for (k, &i) in o.iter().enumerate() {
        let off = (xy[k] - curve.point(s[k])).dot(curve.normal_out(s[k]));
        if off.abs() < max_off {
            out.points.push(va[i]);
            out.normals.push(na[i]);
            out.s.push(s[k]);
            out.off.push(off);
            out.vertex.push(i);
        }
    }
    out
}

/// Crown score 0-1: enamel-like texture at the level of the occlusal plane
/// (an edentulous ridge can be rough in the scan, but it is 5-8 mm lower).
pub fn crown_score(tex: f64, rh: f64) -> f64 {
    ((tex - CROWN_TEXTURE.0) / (CROWN_TEXTURE.1 - CROWN_TEXTURE.0)).clamp(0.0, 1.0)
        * ((rh - CROWN_LOW.0) / (CROWN_LOW.1 - CROWN_LOW.0)).clamp(0.0, 1.0)
}

/// Last crown bin walking from bin `c` in direction `step` (+1/-1). The arch
/// ends where the crown score stays low for 3 mm and the next 10 mm contain
/// almost no crown (a missing tooth or a diastema is a gap followed by
/// crowns, the end of the arch is not).
fn arch_end(crown: &[f64], c: usize, step: isize, bin_mm: f64) -> isize {
    const THR: f64 = 0.5;
    const CROWN_SHARE: f64 = 0.3;
    let n = crown.len() as isize;
    let run = (3.0 / bin_mm) as isize;
    let look = (10.0 / bin_mm) as isize;
    let (mut k, mut low) = (c as isize, 0isize);
    while 0 <= k + step && k + step < n {
        k += step;
        if crown[k as usize] < THR {
            low += 1;
            if low >= run {
                let start = k - step * (run - 1);
                let seg = if step > 0 {
                    &crown[start as usize..(start + look).min(n) as usize]
                } else {
                    &crown[(start - look + 1).max(0) as usize..(start + 1) as usize]
                };
                let share =
                    seg.iter().filter(|v| **v >= THR).count() as f64 / seg.len().max(1) as f64;
                if seg.is_empty() || share < CROWN_SHARE {
                    return start - step;
                }
                low = 0;
            }
        } else {
            low = 0;
        }
    }
    k
}

/// Height of the crown-top profile above the occlusal plane of the crowns:
/// least-squares plane through the textured crown tops, refitted on the
/// tops within 1.5 mm below it (robust to gingiva, ridges, tipped teeth).
/// Bins without points get -99.
fn occlusal_residual(h: &[f64], xy: &[DVec2], crownish: &[bool]) -> Vec<f64> {
    const KEEP: f64 = -1.5;
    let ok: Vec<bool> = h.iter().map(|v| v.is_finite()).collect();
    let mut w: Vec<bool> = (0..h.len()).map(|i| ok[i] && crownish[i]).collect();
    if w.iter().filter(|b| **b).count() < 10 {
        w = ok.clone();
    }
    let mut r = vec![0.0; h.len()];
    for _ in 0..8 {
        let (rows, rhs): (Vec<[f64; 3]>, Vec<f64>) = (0..h.len())
            .filter(|&i| w[i])
            .map(|i| ([1.0, xy[i].x, xy[i].y], h[i]))
            .unzip();
        let sol = lstsq3(&rows, &rhs);
        for i in 0..h.len() {
            r[i] = h[i] - (sol[0] + xy[i].x * sol[1] + xy[i].y * sol[2]);
        }
        let w2: Vec<bool> = (0..h.len())
            .map(|i| ok[i] && crownish[i] && r[i] > KEEP)
            .collect();
        if w2.iter().filter(|b| **b).count() < 10 {
            break;
        }
        w = w2;
    }
    (0..h.len())
        .map(|i| if ok[i] { r[i] } else { -99.0 })
        .collect()
}

/// Profiles along the arch, one entry per bin (`autoseed.arch_profiles`).
#[derive(Debug, Clone)]
pub struct Profiles {
    /// Arc length of the start of bin 0.
    pub s_min: f64,
    /// Arc length of the end of the profiled arch.
    pub s_max: f64,
    /// Number of bins.
    pub nb: usize,
    /// Bin of every point.
    pub bins: Vec<usize>,
    /// The points of the profiled arch (arch coordinates).
    pub points: Vec<DVec3>,
    /// Crown top height, smoothed.
    pub h: Vec<f64>,
    /// Depth of the interproximal valleys (>= 0).
    pub dip: Vec<f64>,
    /// Bucco-lingual width of the crowns, smoothed.
    pub bl: Vec<f64>,
    /// Crown score 0-1 (1 = enamel-like surface at the occlusal level).
    pub crown: Vec<f64>,
    /// Bucco-lingual offset of the highest point of the bin (NaN: none).
    pub off_top: Vec<f64>,
    /// Highest point of the buccal half minus the lingual half (NaN: none).
    pub dh_bl: Vec<f64>,
    /// Scan vertex of every point.
    pub vertex: Vec<usize>,
}

/// Profiles along the arch (bins of `bin_mm`): crown top height, depth of
/// the interproximal valleys, bucco-lingual width of the crowns (spread of
/// the points within 4.5 mm of the local crown top), crown score 0-1 from
/// the surface texture (dispersion of the normals within 3 mm of the top,
/// 1 mm windows) and the height above the occlusal plane, used to find
/// where the crowns end and where a tooth is missing; side of the highest
/// cusp. `pts`: [`arch_points`] result (computed if `None`).
pub fn arch_profiles(
    va: &[DVec3],
    na: &[DVec3],
    curve: &ArchCurve,
    bin_mm: f64,
    pts: Option<ArchPoints>,
) -> Result<Profiles, AutoseedError> {
    const CROWN_DEPTH: f64 = 4.5;
    const MAX_OFF: f64 = 7.0;
    const TEX_DEPTH: f64 = 3.0;
    const TEX_MM: f64 = 1.0;
    let ap = pts.unwrap_or_else(|| arch_points(va, na, curve, MAX_OFF, 150_000));
    if ap.points.is_empty() {
        return Err(AutoseedError::ArchNotFound);
    }
    let s0 = ap.s.iter().copied().fold(f64::INFINITY, f64::min);
    let s_hi = ap.s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let nb0 = ((s_hi - s0) / bin_mm).ceil() as usize + 1;
    let b0: Vec<usize> =
        ap.s.iter()
            .map(|s| (((s - s0) / bin_mm) as usize).min(nb0 - 1))
            .collect();
    let mut h0 = vec![f64::NEG_INFINITY; nb0];
    for (p, &b) in ap.points.iter().zip(&b0) {
        h0[b] = h0[b].max(p.z);
    }
    // texture: mean of the unit normals of the points near the top, 1 mm windows
    let mut sn = [vec![0.0; nb0], vec![0.0; nb0], vec![0.0; nb0]];
    let mut cnt = vec![0.0; nb0];
    for (i, &b) in b0.iter().enumerate() {
        if ap.points[i].z >= h0[b] - TEX_DEPTH {
            let nn = ap.normals[i];
            sn[0][b] += nn.x;
            sn[1][b] += nn.y;
            sn[2][b] += nn.z;
            cnt[b] += 1.0;
        }
    }
    let w = (round_even(TEX_MM / bin_mm)).max(1) as usize;
    let snw: Vec<Vec<f64>> = sn.iter().map(|v| moving_sum_same(v, w)).collect();
    let cw = moving_sum_same(&cnt, w);
    // sparse bins (edge of the scan) are unreliable: treated as smooth.
    // Relative to the scan density only, so that coarse scans keep their crowns
    let pos: Vec<f64> = cw.iter().copied().filter(|c| *c > 0.0).collect();
    let dense_min = 0.15 * median(&pos);
    let tex: Vec<f64> = (0..nb0)
        .map(|k| {
            if cw[k] >= dense_min {
                let l =
                    (snw[0][k] * snw[0][k] + snw[1][k] * snw[1][k] + snw[2][k] * snw[2][k]).sqrt();
                1.0 - l / cw[k].max(1.0)
            } else {
                0.0
            }
        })
        .collect();
    // crown score from the texture and the height above the occlusal plane
    let xy0: Vec<DVec2> = (0..nb0)
        .map(|k| curve.point(s0 + (k as f64 + 0.5) * bin_mm))
        .collect();
    let crownish: Vec<bool> = tex.iter().map(|t| *t >= TEXTURE_GUM).collect();
    let rh = occlusal_residual(&h0, &xy0, &crownish);
    let crown0: Vec<f64> = (0..nb0).map(|k| crown_score(tex[k], rh[k])).collect();
    let c0 = round_even((0.0 - s0) / bin_mm).clamp(0, nb0 as i64 - 1) as usize;
    let lo_end = arch_end(&crown0, c0, -1, bin_mm);
    let hi_end = arch_end(&crown0, c0, 1, bin_mm);
    // the exact end is left to the dynamic programming: END_ZONE mm of
    // margin, where leaving smooth (gingival) surface unassigned costs nothing
    let s_min = s0.max(s0 + lo_end as f64 * bin_mm - END_ZONE);
    let s_max = (s0 + (nb0 - 1) as f64 * bin_mm).min(s0 + (hi_end + 1) as f64 * bin_mm + END_ZONE);
    let inside: Vec<usize> = (0..ap.s.len())
        .filter(|&i| ap.s[i] >= s_min && ap.s[i] <= s_max)
        .collect();
    if inside.is_empty() {
        return Err(AutoseedError::ArchNotFound);
    }
    let points: Vec<DVec3> = inside.iter().map(|&i| ap.points[i]).collect();
    let s: Vec<f64> = inside.iter().map(|&i| ap.s[i]).collect();
    let off: Vec<f64> = inside.iter().map(|&i| ap.off[i]).collect();
    let vertex: Vec<usize> = inside.iter().map(|&i| ap.vertex[i]).collect();
    let nb = ((s_max - s_min) / bin_mm).ceil() as usize + 1;
    let bins: Vec<usize> = s
        .iter()
        .map(|v| (((v - s_min) / bin_mm) as usize).min(nb - 1))
        .collect();
    let mut hraw = vec![f64::NAN; nb];
    for (p, &b) in points.iter().zip(&bins) {
        // numpy.fmax: NaN loses
        hraw[b] = if hraw[b].is_nan() {
            p.z
        } else {
            hraw[b].max(p.z)
        };
    }
    let hfill = fill_nan(&hraw).ok_or(AutoseedError::ArchNotFound)?;
    let hs = moving_average(&hfill, 2);
    let r = round_even(2.0 / bin_mm) as isize;
    let dip: Vec<f64> = (0..nb as isize)
        .map(|i| {
            let m = (i - r..=i + r)
                .map(|k| hs[k.clamp(0, nb as isize - 1) as usize])
                .fold(f64::NEG_INFINITY, f64::max);
            m - hs[i as usize]
        })
        .collect();
    let mut bl = vec![f64::NAN; nb];
    let mut dh_bl = vec![f64::NAN; nb];
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (i, &b) in bins.iter().enumerate() {
        if points[i].z >= hs[b] - CROWN_DEPTH {
            groups[b].push(i);
        }
    }
    for (k, g) in groups.iter().enumerate() {
        if g.len() >= 6 {
            let og: Vec<f64> = g.iter().map(|&i| off[i]).collect();
            let mut sorted = og.clone();
            sorted.sort_by(f64::total_cmp);
            bl[k] = percentile_sorted(&sorted, 97.0) - percentile_sorted(&sorted, 3.0);
            let med = median(&og);
            let top_of = |buccal: bool| {
                g.iter()
                    .zip(&og)
                    .filter(|(_, o)| (**o > med) == buccal)
                    .map(|(&i, _)| points[i].z)
                    .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))
            };
            if let (Some(bu), Some(li)) = (top_of(true), top_of(false)) {
                dh_bl[k] = bu - li;
            }
        }
    }
    let bl = moving_average(&fill_nan(&bl).unwrap_or_else(|| vec![0.0; nb]), 2);
    // bucco-lingual offset of the highest point of every bin (ties: the last)
    let mut off_top = vec![f64::NAN; nb];
    let mut top_z = vec![f64::NEG_INFINITY; nb];
    for (i, &b) in bins.iter().enumerate() {
        if points[i].z >= top_z[b] {
            top_z[b] = points[i].z;
            off_top[b] = off[i];
        }
    }
    let crown: Vec<f64> = (0..nb)
        .map(|i| {
            let k = round_even((s_min + i as f64 * bin_mm - s0) / bin_mm);
            crown0[k.clamp(0, nb0 as i64 - 1) as usize]
        })
        .collect();
    Ok(Profiles {
        s_min,
        s_max,
        nb,
        bins,
        points,
        h: hs,
        dip,
        bl,
        crown,
        off_top,
        dh_bl,
        vertex,
    })
}

/// Arch frame, arch curve and profiles of a scan, shared by the arch
/// recognition and the seed proposal (`autoseed.arch_data`).
#[derive(Debug, Clone)]
pub struct ArchData {
    /// Arch frame in scan coordinates ([`arch_frame`]).
    pub frame: DMat4,
    /// Arch curve in arch coordinates.
    pub curve: ArchCurve,
    /// Profiles along the curve.
    pub profiles: Profiles,
}

/// [`ArchData`] of a scan (vertices in scan coordinates).
pub fn arch_data(v: &[DVec3], t: &[[u32; 3]]) -> Result<ArchData, AutoseedError> {
    let m = arch_frame(v, t, BAND_DEPTH)?;
    let mi = frame::rigid_inverse(&m);
    let va: Vec<DVec3> = v.iter().map(|p| transform(&mi, *p)).collect();
    let mut na = vertex_normals(&va, t);
    let z: Vec<f64> = va.iter().map(|p| p.z).collect();
    let z95 = percentile(&z, 95.0);
    let up: Vec<f64> = va
        .iter()
        .zip(&na)
        .filter(|(p, _)| p.z >= z95)
        .map(|(_, n)| n.z)
        .collect();
    if median(&up) < 0.0 {
        // inward-wound scan: crown tops must face occlusally
        for n in &mut na {
            *n = -*n;
        }
    }
    let top = percentile(&z, 99.5);
    let b: Vec<DVec2> = va
        .iter()
        .filter(|p| p.z >= top - BAND_DEPTH)
        .map(|p| p.truncate())
        .collect();
    let curve = arch_polyline(&b)?;
    let profiles = arch_profiles(&va, &na, &curve, BIN_MM, None)?;
    Ok(ArchData {
        frame: m,
        curve,
        profiles,
    })
}

// ----------------------------------------------------------------------------
// seed proposal
// ----------------------------------------------------------------------------

/// A tooth sequence along the arch: the teeth, their prior cost and the
/// distal tooth of each side (right, left).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The teeth in the order of increasing arch-frame x.
    pub teeth: Vec<Fdi>,
    /// Prior cost.
    pub prior: f64,
    /// Distal tooth position of the right and left side (5..=8).
    pub ends: [u8; 2],
}

/// Tooth sequences along the arch with the distal end of each side at the
/// second premolar (first and second molar missing), first, second or third
/// molar, with a prior cost (second molars present in most patients, third
/// molars in fewer, a missing first molar costs like any missing tooth).
/// `only`: (right end, left end).
pub fn candidate_sequences(
    arch: Arch,
    include_third: bool,
    missing: &[Fdi],
    only: Option<[u8; 2]>,
) -> Vec<Candidate> {
    let prior = |e: u8| match e {
        5 => 0.5 + MISSING_COST,
        6 => 0.5,
        7 => 0.0,
        _ if include_third => 0.0,
        _ => 3.0,
    };
    let base = expected_sequence(arch, true, missing);
    let mut out = Vec::new();
    for er in 5..=8u8 {
        for el in 5..=8u8 {
            if only.is_some_and(|o| o != [er, el]) {
                continue;
            }
            let teeth = base
                .iter()
                .copied()
                .filter(|f| anatomy::index(*f) <= if anatomy::is_right(*f) { er } else { el })
                .collect();
            out.push(Candidate {
                teeth,
                prior: prior(er) + prior(el),
                ends: [er, el],
            });
        }
    }
    out
}

/// Distal tooth of each side of the chosen sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Molars {
    /// Position of the right distal tooth (5..=8).
    pub right: u8,
    /// Position of the left distal tooth (5..=8).
    pub left: u8,
}

/// Cost of one candidate sequence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateCost {
    /// Distal tooth of the right and left side.
    pub ends: [u8; 2],
    /// Best matching cost over the scales.
    pub cost: f64,
    /// `cost` plus the prior of the sequence.
    pub total: f64,
}

/// Diagnostics of a proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostics {
    /// Scale of the typical tooth widths that fits the arch.
    pub scale: f64,
    /// Length of the profiled arch (mm).
    pub arch_length_mm: f64,
    /// Number of seeds.
    pub teeth: usize,
    /// Total cost of the chosen sequence (matching plus prior).
    pub cost: f64,
    /// Matching cost of the chosen sequence.
    pub cost_raw: f64,
    /// Distal tooth of each side.
    pub molars: Molars,
    /// Every candidate sequence tried, in order.
    pub candidates: Vec<CandidateCost>,
    /// Teeth of the sequence found missing inside the arch.
    pub absent_inside: Vec<u8>,
}

/// One seed per tooth found.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The seeds (scan coordinates), in the order of [`anatomy::arch_order`].
    pub seeds: Vec<Seed>,
    /// Arch frame (scan coordinates).
    pub frame: DMat4,
    /// Diagnostics.
    pub diagnostics: Diagnostics,
}

/// Options of the proposal (the plugin's settings).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    /// Third molars may be present.
    pub include_third: bool,
    /// Teeth known to be missing.
    pub missing: Vec<Fdi>,
    /// Forced distal tooth of the right and left side (5..=8).
    pub force_ends: Option<[u8; 2]>,
}

/// One seed per tooth. The arch curve follows the crowns; along it the
/// crown top profile, its interproximal valleys, the crown score and the
/// bucco-lingual width of the crowns are matched by dynamic programming to
/// the typical mesio-distal and bucco-lingual diameters of the tooth
/// sequence (Wheeler), for every plausible number of molars on each side; a
/// tooth inside the arch may be missing (its space, if not closed, must
/// look like gingiva). `data`: [`arch_data`] result (computed if `None`).
pub fn propose(
    v: &[DVec3],
    t: &[[u32; 3]],
    arch: Arch,
    opts: &Options,
    data: Option<&ArchData>,
) -> Result<Proposal, AutoseedError> {
    if opts
        .force_ends
        .is_some_and(|e| e.iter().any(|x| !(5..=8).contains(x)))
    {
        return Err(AutoseedError::BadEnds);
    }
    let owned;
    let data = match data {
        Some(d) => d,
        None => {
            owned = arch_data(v, t)?;
            &owned
        }
    };
    let pr = &data.profiles;
    let nb = pr.nb;
    // the teeth are counted from the midline, like a clinician: the contact
    // between the central incisors lies near the middle of the most anterior
    // crowns, the arc-length origin of the arch frame
    let mid_bin = (0.0 - pr.s_min) / BIN_MM;
    // bucco-lingual widths compared as relative patterns (molars wider than
    // premolars...): the absolute measure depends on crown shape and inclination
    let med_bl = median(&pr.bl).max(1e-6);
    let mut c = Vec::with_capacity(nb + 1);
    c.push(0.0);
    let mut acc = 0.0;
    for b in &pr.bl {
        acc += b / med_bl;
        c.push(acc);
    }
    let kscales = linspace(0.82, 1.18, 10);
    let cands = candidate_sequences(arch, opts.include_third, &opts.missing, opts.force_ends);
    // per candidate: (cost, segments, scale) of the best scale (first minimum)
    type Best = (f64, Vec<Option<(usize, usize)>>, f64);
    let results: Vec<Best> = cands
        .par_iter()
        .map(|cand| {
            let seq = &cand.teeth;
            let md: Vec<f64> = seq
                .iter()
                .map(|f| anatomy::typical_diameters(*f).0)
                .collect();
            let bl_raw: Vec<f64> = seq
                .iter()
                .map(|f| anatomy::typical_diameters(*f).1)
                .collect();
            let bl_med = median(&bl_raw);
            let bl: Vec<f64> = bl_raw.iter().map(|b| b / bl_med).collect();
            // tolerance on the mesio-distal width: anterior teeth are often
            // crowded, molars are rarely crowded and vary less
            let sig: Vec<f64> = seq.iter().map(|f| width_sigma(*f)).collect();
            let centrals: Vec<usize> = (0..seq.len())
                .filter(|&j| anatomy::index(seq[j]) == 1)
                .collect();
            let mid = (centrals.len() == 2).then(|| (centrals[0], mid_bin, MIDLINE_SIGMA / BIN_MM));
            // any tooth between the two ends of the sequence may be missing
            let skippable: Vec<bool> = (0..seq.len()).map(|j| 0 < j && j + 1 < seq.len()).collect();
            let runs: Vec<Best> = kscales
                .par_iter()
                .map(|k| {
                    let w: Vec<f64> = md.iter().map(|m| m * k / BIN_MM).collect();
                    let (cost, segs) = dp(&DpProblem {
                        w_bins: &w,
                        bl_exp: &bl,
                        dip: &pr.dip,
                        bl_cum: &c,
                        sigma: &sig,
                        mid,
                        crown: &pr.crown,
                        skippable: &skippable,
                        bin_mm: BIN_MM,
                    });
                    (cost, segs, *k)
                })
                .collect();
            runs.into_iter()
                .reduce(|best, r| if r.0 < best.0 { r } else { best })
                .unwrap_or((f64::INFINITY, Vec::new(), 1.0))
        })
        .collect();
    let mut tried = Vec::with_capacity(cands.len());
    let mut best: Option<(f64, usize)> = None;
    for (i, (cand, r)) in cands.iter().zip(&results).enumerate() {
        let total = r.0 + cand.prior;
        tried.push(CandidateCost {
            ends: cand.ends,
            cost: r.0,
            total,
        });
        if best.is_none_or(|b| total < b.0) {
            best = Some((total, i));
        }
    }
    let (total, bi) = best.ok_or(AutoseedError::BadEnds)?;
    let (raw, segs, kscale) = &results[bi];
    let seq = &cands[bi].teeth;
    let mut found: BTreeMap<Fdi, DVec3> = BTreeMap::new();
    let mut absent = Vec::new();
    for (j, f) in seq.iter().enumerate() {
        let Some((a0, a1)) = segs[j] else {
            absent.push(f.get());
            continue;
        };
        let sel: Vec<DVec3> = pr
            .bins
            .iter()
            .zip(&pr.points)
            .filter(|(b, _)| (a0..a1).contains(*b))
            .map(|(_, p)| *p)
            .collect();
        if sel.len() < 5 {
            continue;
        }
        let zmax = sel.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        let hi: Vec<DVec2> = sel
            .iter()
            .filter(|p| p.z >= zmax - 2.5)
            .map(|p| p.truncate())
            .collect();
        let cxy = hi.iter().copied().sum::<DVec2>() / hi.len() as f64;
        let score = |p: &DVec3| (p.truncate() - cxy).length() - 0.2 * (p.z - zmax);
        let mut k = 0;
        for (i, p) in sel.iter().enumerate() {
            if score(p) < score(&sel[k]) {
                k = i;
            }
        }
        found.insert(*f, transform(&data.frame, sel[k]));
    }
    let seeds: Vec<Seed> = anatomy::arch_order(arch)
        .into_iter()
        .filter_map(|f| {
            found.get(&f).map(|p| Seed {
                fdi: f,
                position: *p,
            })
        })
        .collect();
    let ends = cands[bi].ends;
    Ok(Proposal {
        diagnostics: Diagnostics {
            scale: *kscale,
            arch_length_mm: pr.s_max - pr.s_min,
            teeth: seeds.len(),
            cost: total,
            cost_raw: *raw,
            molars: Molars {
                right: ends[0],
                left: ends[1],
            },
            candidates: tried,
            absent_inside: absent,
        },
        seeds,
        frame: data.frame,
    })
}

// ----------------------------------------------------------------------------
// upper / lower arch
// ----------------------------------------------------------------------------

/// Fraction of the region inside the dental arch covered by the scan: ~1
/// when the palate is present (upper arch), ~0 for the tongue space (lower
/// arch). `m`: arch frame (computed if `None`).
pub fn palate_coverage(
    v: &[DVec3],
    t: &[[u32; 3]],
    m: Option<DMat4>,
) -> Result<f64, AutoseedError> {
    let m = match m {
        Some(m) => m,
        None => arch_frame(v, t, BAND_DEPTH)?,
    };
    let mi = frame::rigid_inverse(&m);
    let va: Vec<DVec3> = v.iter().map(|p| transform(&mi, *p)).collect();
    let z: Vec<f64> = va.iter().map(|p| p.z).collect();
    let top = percentile(&z, 99.5);
    let b: Vec<DVec3> = va
        .iter()
        .copied()
        .filter(|p| p.z >= top - BAND_DEPTH)
        .collect();
    let mut bx: Vec<f64> = b.iter().map(|p| p.x).collect();
    let mut by: Vec<f64> = b.iter().map(|p| p.y).collect();
    bx.sort_by(f64::total_cmp);
    by.sort_by(f64::total_cmp);
    let (x0, x1) = (percentile_sorted(&bx, 1.0), percentile_sorted(&bx, 99.0));
    let (y0, y1) = (percentile_sorted(&by, 1.0), percentile_sorted(&by, 99.0));
    let (wd, dp) = (x1 - x0, y1 - y0);
    let xc = 0.5 * (x0 + x1);
    // central part of the region inside the U: palate vault or tongue space
    let gx = arange(xc - 0.15 * wd, xc + 0.15 * wd, 1.0);
    let gy = arange(y0 + 0.2 * dp, y1 - 0.5 * dp, 1.0);
    if gx.len() < 3 || gy.len() < 3 {
        return Ok(0.0);
    }
    let mut occ = vec![false; gx.len() * gy.len()];
    for p in &va {
        let ix = (p.x - gx[0]).floor();
        let iy = (p.y - gy[0]).floor();
        if ix >= 0.0 && iy >= 0.0 && (ix as usize) < gx.len() && (iy as usize) < gy.len() {
            occ[ix as usize * gy.len() + iy as usize] = true;
        }
    }
    Ok(occ.iter().filter(|o| **o).count() as f64 / occ.len() as f64)
}

/// Arch written in the file name (`..._UpperJaw.stl`, `mandibular`,
/// `arcata_superiore`...), as exported by the intraoral scanners.
pub fn arch_from_name(name: &str) -> Option<Arch> {
    match dc_case::arch_hint(name) {
        Some(dc_case::ScanRole::Upper) => Some(Arch::Upper),
        Some(dc_case::ScanRole::Lower) => Some(Arch::Lower),
        _ => None,
    }
}

/// How well the measured crown widths follow the typical mesio-distal
/// diameters of their FDI numbers (Wheeler) after a global scale: lower =
/// better. Returns (cost, scale).
pub fn width_fit(widths: &BTreeMap<Fdi, f64>) -> (f64, f64) {
    let f: Vec<(Fdi, f64)> = widths
        .iter()
        .filter(|(_, w)| **w > 0.0)
        .map(|(f, w)| (*f, *w))
        .collect();
    if f.is_empty() {
        return (f64::INFINITY, 1.0);
    }
    let exp: Vec<f64> = f
        .iter()
        .map(|(k, _)| anatomy::typical_diameters(*k).0)
        .collect();
    let ratio: Vec<f64> = f.iter().zip(&exp).map(|((_, w), e)| w / e).collect();
    let k = median(&ratio);
    let terms: Vec<f64> = f
        .iter()
        .zip(&exp)
        .map(|((_, w), e)| {
            let d = (w - k * e) / (0.15 * k * e);
            d * d
        })
        .collect();
    (mean(&terms), k)
}

/// Teeth whose measured crown width differs from the typical one of their
/// number (after the global scale of [`width_fit`]) by more than `rel`: a
/// molar counted as a premolar, a premolar split in two, a gum flap taken
/// as a crown... Returns (fdi, measured mm, expected mm), largest
/// deviations first, at most `top`.
pub fn width_outliers(widths: &BTreeMap<Fdi, f64>, rel: f64, top: usize) -> Vec<(Fdi, f64, f64)> {
    let (_, k) = width_fit(widths);
    let mut out: Vec<(f64, Fdi, f64, f64)> = widths
        .iter()
        .filter(|(_, w)| **w > 0.0)
        .filter_map(|(f, w)| {
            let e = k * anatomy::typical_diameters(*f).0;
            ((w - e).abs() > rel * e).then(|| ((w - e).abs() / e, *f, *w, e))
        })
        .collect();
    out.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(b.2.total_cmp(&a.2))
            .then(b.3.total_cmp(&a.3))
    });
    out.into_iter()
        .take(top)
        .map(|(_, f, w, e)| (f, w, e))
        .collect()
}

/// Mean width of the central incisors / first molars: about 0.85 in the
/// upper arch (8.5 / 10 mm), 0.45 in the lower (5 / 11 mm).
pub fn incisor_molar_ratio(widths: &BTreeMap<Fdi, f64>) -> Option<f64> {
    let of = |i: u8| -> Vec<f64> {
        widths
            .iter()
            .filter(|(f, w)| anatomy::index(**f) == i && **w > 0.0)
            .map(|(_, w)| *w)
            .collect()
    };
    let (c, m) = (of(1), of(6));
    if c.is_empty() || m.is_empty() {
        return None;
    }
    Some(mean(&c) / mean(&m))
}

/// Side of the highest cusps at the distal ends of the arch, in mm: > 0
/// buccal, < 0 lingual (median bucco-lingual offset of the crown top +
/// median height difference between the buccal and the lingual half of the
/// crown). Curve of Wilson: the maxillary molars are inclined buccally, so
/// their palatal cusps are the most occlusal; the mandibular molars are
/// inclined lingually and their buccal cusps stand highest. `None` if the
/// crowns are not found.
pub fn posterior_cusp_side(pr: &Profiles, bin_mm: f64, length_mm: f64) -> Option<f64> {
    let cr: Vec<usize> = (0..pr.nb).filter(|&k| pr.crown[k] >= 0.5).collect();
    if cr.len() < 40 {
        return None;
    }
    let n = (length_mm / bin_mm) as i64;
    let (first, last) = (cr[0] as i64, *cr.last()? as i64);
    let ks: Vec<usize> = (first..first + n)
        .chain(last - n + 1..last + 1)
        .filter(|&k| k >= 0 && (k as usize) < pr.nb)
        .map(|k| k as usize)
        .filter(|&k| pr.crown[k] >= 0.5)
        .collect();
    let off: Vec<f64> = ks
        .iter()
        .map(|&k| pr.off_top[k])
        .filter(|v| v.is_finite())
        .collect();
    let dh: Vec<f64> = ks
        .iter()
        .map(|&k| pr.dh_bl[k])
        .filter(|v| v.is_finite())
        .collect();
    if off.len() < 10 || dh.len() < 10 {
        return None;
    }
    Some(median(&off) + median(&dh))
}

/// Which cue decided the arch in [`detect_arch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchSource {
    /// The file name written by the scanner software.
    FileName,
    /// Scanner frame: the teeth point down (upper arch).
    ScannerOrientation,
    /// The region inside the arch is closed, but the buccal molar cusps are
    /// clearly highest: a lower arch with a closed model base, to be checked.
    WilsonOverPalate,
    /// The palate is present (upper arch).
    Palate,
    /// The palate is present, but the molar cusps point to a lower arch: to
    /// be checked.
    PalateDisagreesWithMolars,
    /// Curve of Wilson: side of the highest cusps of the last molars.
    CurveOfWilson,
    /// Two weak cues that agree: molar cusps and tooth widths.
    WilsonAndWidths,
    /// Shape and widths of the teeth only: to be checked after segmentation.
    ShapeAndWidths,
}

impl ArchSource {
    /// `true` for the cues the plugin marks "da verificare" in `detect_arch`
    /// (`autoseed.py`: the sources of [`ArchSource::WilsonOverPalate`],
    /// [`ArchSource::PalateDisagreesWithMolars`] and
    /// [`ArchSource::ShapeAndWidths`]): the arch is chosen, but a person should
    /// look at it. The others (file name, scanner orientation, palate, curve
    /// of Wilson, two weak cues that agree) are not flagged.
    pub fn to_be_checked(self) -> bool {
        matches!(
            self,
            Self::WilsonOverPalate | Self::PalateDisagreesWithMolars | Self::ShapeAndWidths
        )
    }
}

/// Result of [`detect_arch`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArchDetection {
    /// The arch.
    pub arch: Arch,
    /// Confidence 0-1.
    pub confidence: f64,
    /// The deciding cue.
    pub source: ArchSource,
    /// [`palate_coverage`] (`None` when the name decided).
    pub palate_coverage: Option<f64>,
    /// World Z component of the occlusal direction.
    pub occlusal_z: Option<f64>,
    /// [`posterior_cusp_side`] in mm.
    pub molar_cusps_mm: Option<f64>,
    /// Proposal cost as an upper arch (only when the widths decided).
    pub cost_upper: Option<f64>,
    /// Proposal cost as a lower arch (only when the widths decided).
    pub cost_lower: Option<f64>,
}

impl ArchDetection {
    /// `true` when the tooth-width comparison itself failed: the proposal
    /// could not be built for either arch, so both costs are infinite (or
    /// not a number). [`detect_arch`] keeps the plugin's arithmetic and still
    /// answers (an upper arch at confidence 0.6, as the plugin does, because
    /// the difference of the costs is NaN), but that answer rests on nothing:
    /// a caller should not take it as a recognition.
    pub fn widths_failed(&self) -> bool {
        matches!((self.cost_upper, self.cost_lower), (Some(u), Some(l))
            if !u.is_finite() && !l.is_finite())
    }
}

/// Upper or lower arch.
///
/// 1. File name written by the scanner software (UpperJaw / LowerJaw ...).
/// 2. Scanner frame: intraoral scans exported in occlusion have the upper
///    teeth pointing down.
/// 3. Palate: the region inside the arch is covered by the scan only for
///    the upper arch, or by the base of a model: buccal molar cusps clearly
///    highest ([`WILSON_OVER_PALATE`]) make it a lower arch with a closed
///    base, to be checked.
/// 4. Curve of Wilson ([`posterior_cusp_side`]).
/// 5. Tooth-width pattern along the arch: the proposal fits better with the
///    widths of the right arch; accepted at full weak confidence when it
///    agrees with a weak molar-cusp cue.
pub fn detect_arch(
    v: &[DVec3],
    t: &[[u32; 3]],
    name: Option<&str>,
) -> Result<ArchDetection, AutoseedError> {
    let mut det = ArchDetection {
        arch: Arch::Lower,
        confidence: 1.0,
        source: ArchSource::FileName,
        palate_coverage: None,
        occlusal_z: None,
        molar_cusps_mm: None,
        cost_upper: None,
        cost_lower: None,
    };
    if let Some(arch) = name.and_then(arch_from_name) {
        det.arch = arch;
        return Ok(det);
    }
    let (n, _) = occlusal_axis(v, t)?;
    let data = arch_data(v, t)?;
    let cov = palate_coverage(v, t, Some(data.frame))?;
    let side = posterior_cusp_side(&data.profiles, BIN_MM, WILSON_MM);
    det.palate_coverage = Some(cov);
    det.occlusal_z = Some(n.z);
    det.molar_cusps_mm = side;
    let finish = |mut det: ArchDetection, arch, confidence, source| {
        det.arch = arch;
        det.confidence = confidence;
        det.source = source;
        Ok(det)
    };
    if n.z < -0.8 {
        return finish(det, Arch::Upper, 0.8, ArchSource::ScannerOrientation);
    }
    if cov >= 0.5 {
        if let Some(sd) = side
            && sd >= WILSON_OVER_PALATE
        {
            // closed region inside the arch, but the molars of a lower arch:
            // the base of a model, not a palate
            return finish(
                det,
                Arch::Lower,
                0.65f64.min(0.4 + 0.2 * sd),
                ArchSource::WilsonOverPalate,
            );
        }
        if side.is_some_and(|sd| sd >= WILSON_MIN) {
            // weak disagreement of the molar cusps: kept, to be checked
            return finish(det, Arch::Upper, 0.6, ArchSource::PalateDisagreesWithMolars);
        }
        return finish(
            det,
            Arch::Upper,
            1.0f64.min(0.6 + cov * 0.4),
            ArchSource::Palate,
        );
    }
    if let Some(sd) = side
        && sd.abs() >= WILSON_MIN
    {
        let arch = if sd < 0.0 { Arch::Upper } else { Arch::Lower };
        return finish(
            det,
            arch,
            0.9f64.min(0.5 + 0.4 * sd.abs()),
            ArchSource::CurveOfWilson,
        );
    }
    let cost = |arch| {
        propose(v, t, arch, &Options::default(), Some(&data))
            .map_or(f64::INFINITY, |p| p.diagnostics.cost)
    };
    let (cu, cl) = (cost(Arch::Upper), cost(Arch::Lower));
    det.cost_upper = Some(cu);
    det.cost_lower = Some(cl);
    let diff = cl - cu;
    let scale = cl.abs().max(cu.abs()).max(1.0);
    let arch = if diff < 0.0 { Arch::Lower } else { Arch::Upper };
    if let Some(sd) = side
        && sd != 0.0
        && (sd < 0.0) == (arch == Arch::Upper)
    {
        return finish(det, arch, 0.6, ArchSource::WilsonAndWidths);
    }
    finish(
        det,
        arch,
        0.6f64.min(diff.abs() / (0.2 * scale)),
        ArchSource::ShapeAndWidths,
    )
}

// ----------------------------------------------------------------------------
// stage entry points
// ----------------------------------------------------------------------------

/// Parameters of the automatic seeds (the plugin's settings).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Third molars may be present (default false).
    pub third_molars: bool,
    /// FDI numbers of the teeth known to be missing.
    pub missing: Vec<u8>,
    /// Forced distal tooth of the right and left side, as tooth positions
    /// 5..=8 (e.g. `[7, 7]`); default: chosen by the data.
    pub ends: Option<[u8; 2]>,
}

impl Params {
    /// The proposal options (an error when a missing tooth is not a valid
    /// FDI number).
    pub fn options(&self) -> Result<Options, AutoseedError> {
        let missing = self
            .missing
            .iter()
            .map(|n| Fdi::new(*n).map_err(|_| AutoseedError::BadTooth(*n)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Options {
            include_third: self.third_molars,
            missing,
            force_ends: self.ends,
        })
    }
}

/// Input of the automatic seeds: one arch scan.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Arch of the scan.
    pub arch: Arch,
    /// Vertices (case coordinates, mm).
    pub vertices: &'a [DVec3],
    /// Triangles.
    pub triangles: &'a [[u32; 3]],
}

/// Result of the automatic seeds of one arch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Output {
    /// The arch.
    pub arch: Arch,
    /// The seeds (case coordinates), in the order of [`anatomy::arch_order`].
    pub seeds: Vec<Seed>,
    /// Arch frame of the scan (case coordinates).
    pub arch_frame: ArchFrame,
    /// Diagnostics of the proposal.
    pub diagnostics: Diagnostics,
}

/// Proposes the seeds of one arch scan (`case_auto.auto_seeds`): fails with
/// [`AutoseedError::TooFewSeeds`] below [`MIN_SEEDS`] teeth.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, AutoseedError> {
    let opts = params.options()?;
    let p = propose(input.vertices, input.triangles, input.arch, &opts, None)?;
    if p.seeds.len() < MIN_SEEDS {
        return Err(AutoseedError::TooFewSeeds(p.seeds.len()));
    }
    Ok(Output {
        arch: input.arch,
        arch_frame: ArchFrame::from_matrix(&p.frame),
        seeds: p.seeds,
        diagnostics: p.diagnostics,
    })
}

/// Where the arch frame of [`orient`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrientSource {
    /// The seeds of the arch ([`frame::arch_frame_from_seeds`]).
    Seeds,
    /// The crowns of the scan ([`arch_frame`]).
    Crowns,
}

/// Result of [`orient`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Orientation {
    /// Arch the frame was taken from.
    pub arch: Arch,
    /// How the arch frame was obtained.
    pub source: OrientSource,
    /// Arch frame in case coordinates.
    pub arch_frame: ArchFrame,
    /// Centre of the arch (becomes the world origin).
    pub centre: DVec3,
    /// Case coordinates to world.
    pub from_case: Pose,
    /// `true` when the scan already sits on the world axes.
    pub already_aligned: bool,
}

/// The world frame of the reference scan (see [`crate::frame`]): from the
/// seeds of the arch when there are enough of them, otherwise from the arch
/// frame of the crowns ([`arch_frame`]), with the arch centre halfway
/// between its origin (the most posterior crowns) and the most anterior
/// crown tops.
pub fn orient(
    input: &frame::Input<'_>,
    triangles: &[[u32; 3]],
) -> Result<Orientation, AutoseedError> {
    if input.vertices.is_empty() {
        return Err(AutoseedError::DegenerateMesh);
    }
    let arch_seeds: Vec<Seed> = input
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == input.arch)
        .copied()
        .collect();
    let centroid = input.vertices.iter().copied().sum::<DVec3>() / input.vertices.len() as f64;
    let (source, arch_frame, centre) = match frame::arch_frame_from_seeds(&arch_seeds, centroid) {
        Ok(f) => (OrientSource::Seeds, f, frame::arch_centre(&f, &arch_seeds)),
        Err(_) => {
            let m = arch_frame(input.vertices, triangles, BAND_DEPTH)?;
            let mi = frame::rigid_inverse(&m);
            let va: Vec<DVec3> = input.vertices.iter().map(|p| transform(&mi, *p)).collect();
            let z: Vec<f64> = va.iter().map(|p| p.z).collect();
            let top = percentile(&z, 99.5);
            let front = va
                .iter()
                .filter(|p| p.z >= top - BAND_DEPTH)
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max);
            let f = ArchFrame::from_matrix(&m);
            (
                OrientSource::Crowns,
                f,
                transform(&m, DVec3::new(0.0, 0.5 * front, 0.0)),
            )
        }
    };
    let from_case = frame::world_delta(&arch_frame, input.arch, centre);
    Ok(Orientation {
        arch: input.arch,
        source,
        arch_frame,
        centre,
        already_aligned: frame::is_identity(&from_case),
        from_case,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn the_cues_the_plugin_marks_to_be_checked() {
        use ArchSource::*;
        let flagged: Vec<ArchSource> = [
            FileName,
            ScannerOrientation,
            WilsonOverPalate,
            Palate,
            PalateDisagreesWithMolars,
            CurveOfWilson,
            WilsonAndWidths,
            ShapeAndWidths,
        ]
        .into_iter()
        .filter(|s| s.to_be_checked())
        .collect();
        assert_eq!(
            flagged,
            [WilsonOverPalate, PalateDisagreesWithMolars, ShapeAndWidths]
        );
    }

    #[test]
    fn widths_failed_needs_both_costs_to_be_unusable() {
        let det = |cu, cl| ArchDetection {
            arch: Arch::Upper,
            confidence: 0.6,
            source: ArchSource::ShapeAndWidths,
            palate_coverage: Some(0.0),
            occlusal_z: Some(1.0),
            molar_cusps_mm: None,
            cost_upper: cu,
            cost_lower: cl,
        };
        let inf = Some(f64::INFINITY);
        assert!(det(inf, inf).widths_failed());
        assert!(det(Some(f64::NAN), inf).widths_failed());
        assert!(!det(Some(3.0), inf).widths_failed());
        assert!(!det(inf, Some(3.0)).widths_failed());
        assert!(!det(Some(3.0), Some(4.0)).widths_failed());
        assert!(!det(None, None).widths_failed());
    }

    #[test]
    fn names_give_the_arch() {
        assert_eq!(arch_from_name("Patient_UpperJaw.stl"), Some(Arch::Upper));
        assert_eq!(arch_from_name("scan-LowerJaw"), Some(Arch::Lower));
        assert_eq!(arch_from_name("arcata_superiore"), Some(Arch::Upper));
        assert_eq!(arch_from_name("MaxillaScan"), Some(Arch::Upper));
        assert_eq!(arch_from_name("upper and lower"), None);
        assert_eq!(arch_from_name("scansione"), None);
        assert_eq!(arch_from_name(""), None);
    }

    #[test]
    fn sequences_run_along_increasing_x() {
        let up = expected_sequence(Arch::Upper, false, &[]);
        assert_eq!(up.first().map(|f| f.get()), Some(17));
        assert_eq!(up.last().map(|f| f.get()), Some(27));
        assert_eq!(up.len(), 14);
        let lo = expected_sequence(Arch::Lower, true, &[fdi(36)]);
        assert_eq!(lo.first().map(|f| f.get()), Some(38));
        assert_eq!(lo.last().map(|f| f.get()), Some(48));
        assert!(!lo.contains(&fdi(36)));
        let c = candidate_sequences(Arch::Lower, false, &[], None);
        assert_eq!(c.len(), 16);
        let c77 = c.iter().find(|c| c.ends == [7, 7]).unwrap();
        assert_eq!(c77.teeth.len(), 14);
        assert_eq!(c77.prior, 0.0);
        let only = candidate_sequences(Arch::Lower, false, &[], Some([6, 8]));
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].prior, 3.5);
    }

    #[test]
    fn widths() {
        let w: BTreeMap<Fdi, f64> = [(11, 8.6), (21, 8.4), (16, 10.1), (26, 10.3), (12, 0.0)]
            .into_iter()
            .map(|(f, w)| (fdi(f), w))
            .collect();
        let (cost, k) = width_fit(&w);
        assert!(cost.is_finite() && (k - 1.0).abs() < 0.1, "{cost} {k}");
        assert!(width_outliers(&w, 0.3, 3).is_empty());
        let r = incisor_molar_ratio(&w).unwrap();
        assert!((r - 8.5 / 10.2).abs() < 1e-12);
        assert_eq!(width_fit(&BTreeMap::new()), (f64::INFINITY, 1.0));
    }

    #[test]
    fn a_degenerate_mesh_is_an_error() {
        assert_eq!(
            arch_frame(&[], &[], BAND_DEPTH),
            Err(AutoseedError::DegenerateMesh)
        );
        let opts = Options {
            force_ends: Some([4, 7]),
            ..Default::default()
        };
        assert_eq!(
            propose(&[], &[], Arch::Lower, &opts, None).unwrap_err(),
            AutoseedError::BadEnds
        );
    }
}
