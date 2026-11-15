//! Checks of an EQUATOR border for the panel and the report (the checks of
//! `case_print._aligner_solid`): how much of the labial face of every
//! incisor and canine the border covers (`labial_cover`, on the border of
//! the shell built: `border_heights`), the ones whose border the wall held
//! over the gingiva raised above their trim line, how far beyond its
//! equator the border goes on every tooth (`border_offsets`), the tilt of a
//! common insertion axis and the rigid removal of the shell
//! ([`super::removal`]).

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use dc_case::Fdi;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3};
use serde::Serialize;

use super::line::ToothLoop;
use super::warp::PathWarp;

/// A labial face of an incisor or canine covered less than this high ...
/// (mm, `LABIAL_WARN_MM`)
pub const LABIAL_WARN_MM: f64 = 3.0;
/// ... or less than this share of its crown (top to margin) is reported
/// (`LABIAL_WARN_SHARE`).
pub const LABIAL_WARN_SHARE: f64 = 0.4;
/// The border of the shell built: its lowest vertex per azimuth of a tooth
/// within the wall + this outside the loop of the tooth ... (mm,
/// `BORDER_BAND_MM`)
pub const BORDER_BAND_MM: f64 = 0.3;
/// ... and this inside it (deeper: the occlusal table; `BORDER_INSIDE_MM`)
/// ...
pub const BORDER_INSIDE_MM: f64 = 1.0;
/// ... at most this above the highest point of its trim line, and under the
/// crown top (`BORDER_ABOVE_MM`).
pub const BORDER_ABOVE_MM: f64 = 3.0;
/// An incisor or canine whose built border is this much above its trim line
/// (median of its labial azimuths: the wall held over the gingiva) is listed
/// (mm, `BORDER_RAISED_MM`).
pub const BORDER_RAISED_MM: f64 = 0.3;
/// A tooth whose border goes beyond its equator less than this share of the
/// offset asked (median of its buccal and lingual azimuths) is listed
/// (`OFFSET_LIMIT_SHARE`).
pub const OFFSET_LIMIT_SHARE: f64 = 0.5;
/// An azimuth faces a direction within this cosine (53 degrees).
const FACING_COS: f64 = 0.6;

/// The checks of an EQUATOR border.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Checks {
    /// Labial face of every incisor and canine covered: (mm from the crown
    /// top, share of the crown down to the margin), on the border built.
    pub labial: BTreeMap<Fdi, (f64, f64)>,
    /// Incisors and canines covered less than [`LABIAL_WARN_MM`] or
    /// [`LABIAL_WARN_SHARE`]: (fdi, mm, share), by FDI number.
    pub labial_low: Vec<(Fdi, f64, f64)>,
    /// Incisors and canines whose border is more than [`BORDER_RAISED_MM`]
    /// above their trim line on the labial side: (fdi, mm).
    pub labial_raised: Vec<(Fdi, f64)>,
    /// How far beyond its equator the border goes on every tooth (mm).
    pub offsets: BTreeMap<Fdi, f64>,
    /// Teeth (arch order) where the border goes beyond the equator less than
    /// [`OFFSET_LIMIT_SHARE`] of an offset of at least 0.1 mm: (fdi, mm).
    pub offset_limited: Vec<(Fdi, f64)>,
    /// COMMON insertion axis: its tilt (degrees).
    pub common_tilt: Option<f64>,
    /// The shell taken off in one piece: PER_TOOTH along the least locked
    /// rigid path found, VERTICAL and COMMON along their one path.
    pub removal: Option<super::removal::Removal>,
}

/// Python's `round(x, 2)` for the values the panel shows.
fn round2(x: f64) -> f64 {
    (x * 100.0).round_ties_even() / 100.0
}

/// `np.median`.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// The azimuths of a loop of `n` points facing `u` (`|cos| > 0.6` with
/// `both`).
fn facing(n: usize, u: DVec2, both: bool) -> Vec<bool> {
    (0..n)
        .map(|k| {
            let a = TAU * k as f64 / n as f64;
            let c = a.cos() * u.x + a.sin() * u.y;
            if both {
                c.abs() > FACING_COS
            } else {
                c > FACING_COS
            }
        })
        .collect()
}

/// Plan direction across the arch (unsigned) at every tooth of `cs` (crown
/// centres; `_arch_normals`): square to the line between its neighbours
/// present in arch order `order` (teeth not in it after the others).
pub fn arch_normals(cs: &[(Fdi, DVec2)], order: &[Fdi]) -> BTreeMap<Fdi, DVec2> {
    let has = |f: &Fdi| cs.iter().any(|(g, _)| g == f);
    let mut seq: Vec<Fdi> = order.iter().copied().filter(has).collect();
    seq.extend(cs.iter().map(|(f, _)| *f).filter(|f| !order.contains(f)));
    let at = |f: Fdi| cs.iter().find(|(g, _)| *g == f).map(|(_, c)| *c);
    let mut out = BTreeMap::new();
    for (i, &f) in seq.iter().enumerate() {
        let (Some(a), Some(b)) = (
            at(seq[i.saturating_sub(1)]),
            at(seq[(i + 1).min(seq.len() - 1)]),
        ) else {
            continue;
        };
        let t = b - a;
        let len = (t.x * t.x + t.y * t.y).sqrt();
        if len < 1e-9 {
            continue;
        }
        let t = t / len;
        out.insert(f, DVec2::new(-t.y, t.x));
    }
    out
}

/// Labial direction of every tooth in plan (`labial_dirs`): across the arch
/// ([`arch_normals`]), outward (away from the middle of the arch); the
/// direction from the middle of the arch where a tooth has no neighbour.
pub fn labial_dirs(loops: &[(Fdi, &ToothLoop)], order: &[Fdi]) -> BTreeMap<Fdi, DVec2> {
    let cs: Vec<(Fdi, DVec2)> = loops.iter().map(|(f, l)| (*f, l.centre)).collect();
    if cs.len() < 2 {
        return BTreeMap::new();
    }
    let mid = cs.iter().fold(DVec2::ZERO, |s, (_, c)| s + *c) / cs.len() as f64;
    let nrm = arch_normals(&cs, order);
    let mut out = BTreeMap::new();
    for (f, c) in &cs {
        let out_d = *c - mid;
        let len = (out_d.x * out_d.x + out_d.y * out_d.y).sqrt();
        let u = match nrm.get(f) {
            Some(u) => *u,
            None if len < 1e-6 => continue,
            None => out_d / len,
        };
        out.insert(*f, if u.dot(out_d) >= 0.0 { u } else { -u });
    }
    out
}

/// Labial face of every incisor and canine covered by the trim lines
/// (`labial_cover`): the crown top less the line at the azimuths facing the
/// labial direction (median), and its share of the crown from the top to
/// the gingival margin there. `border` (from [`border_heights`]): the border
/// of the shell built at those azimuths instead of the line, where it has a
/// point.
pub fn labial_cover(
    loops: &[(Fdi, &ToothLoop)],
    border: Option<&BTreeMap<Fdi, Vec<f64>>>,
    order: &[Fdi],
) -> BTreeMap<Fdi, (f64, f64)> {
    let dirs = labial_dirs(loops, order);
    let mut out = BTreeMap::new();
    for (f, l) in loops {
        let Some(&u) = dirs.get(f).filter(|_| f.get() % 10 <= 3) else {
            continue;
        };
        let lab = facing(l.points.len(), u, false);
        if !lab.iter().any(|x| *x) {
            continue;
        }
        let mut z: Vec<f64> = (0..lab.len())
            .filter(|&k| lab[k])
            .map(|k| l.points[k].z)
            .collect();
        if let Some(b) = border
            .and_then(|b| b.get(f))
            .filter(|b| b.len() == l.points.len())
        {
            let bl: Vec<f64> = (0..lab.len()).filter(|&k| lab[k]).map(|k| b[k]).collect();
            if bl.iter().any(|v| v.is_finite()) {
                for (zi, bi) in z.iter_mut().zip(&bl) {
                    if bi.is_finite() {
                        *zi = *bi;
                    }
                }
            }
        }
        let cov = l.top - median(&mut z);
        let full = l.top
            - match &l.margin {
                Some(zm) => {
                    let mut m: Vec<f64> =
                        (0..lab.len()).filter(|&k| lab[k]).map(|k| zm[k]).collect();
                    median(&mut m)
                }
                None => l.points.iter().map(|p| p.z).fold(f64::INFINITY, f64::min),
            };
        out.insert(*f, (round2(cov), round2(cov / full.max(1e-6))));
    }
    out
}

/// `loop_nearest` of the plan point `p` against the closed loop `e`
/// (signed distance, positive inside) and where its nearest point is along
/// the loop: the index of the point before it plus the fraction of the
/// segment (`_loop_place`).
pub fn loop_place(e: &[DVec3], p: DVec2) -> (f64, f64) {
    let n = e.len();
    let (mut best, mut at) = (f64::INFINITY, 0.0);
    let mut crossings = 0u32;
    for i in 0..n {
        let (a, b) = (e[i].truncate(), e[(i + 1) % n].truncate());
        let ab = b - a;
        let l2 = (ab.x * ab.x + ab.y * ab.y).max(1e-12);
        let (rx, ry) = (p.x - a.x, p.y - a.y);
        let t = ((rx * ab.x + ry * ab.y) / l2).clamp(0.0, 1.0);
        let (dx, dy) = (rx - t * ab.x, ry - t * ab.y);
        let d2 = dx * dx + dy * dy;
        if d2 < best {
            (best, at) = (d2, i as f64 + t);
        }
        if (a.y > p.y) != (b.y > p.y) && rx < ry * (ab.x / ab.y) {
            crossings += 1;
        }
    }
    let d = best.sqrt();
    (if crossings % 2 == 1 { d } else { -d }, at)
}

/// The border of the shell built around every tooth (`border_heights`): at
/// every azimuth of its loop the lowest vertex of the shell (`vertices`,
/// arch coordinates, taken into the warped space of `warp`) whose nearest
/// loop (plan) is that one, within the outer offset + [`BORDER_BAND_MM`]
/// outside it and [`BORDER_INSIDE_MM`] inside it, at most
/// [`BORDER_ABOVE_MM`] above its trim line and under the crown top; NaN at
/// an azimuth without one.
pub fn border_heights(
    loops: &[(Fdi, &ToothLoop)],
    mesh: &TriMesh,
    warp: Option<&PathWarp>,
    outer: f64,
) -> BTreeMap<Fdi, Vec<f64>> {
    let fs: Vec<&(Fdi, &ToothLoop)> = loops.iter().filter(|(_, l)| l.points.len() >= 3).collect();
    if fs.is_empty() || mesh.positions.is_empty() {
        return BTreeMap::new();
    }
    let ztop = |l: &ToothLoop| {
        l.top.min(
            l.points
                .iter()
                .map(|p| p.z)
                .fold(f64::NEG_INFINITY, f64::max)
                + BORDER_ABOVE_MM,
        )
    };
    let zmax = fs
        .iter()
        .map(|(_, l)| ztop(l))
        .fold(f64::NEG_INFINITY, f64::max);
    let q: Vec<DVec3> = mesh
        .positions
        .iter()
        .map(|p| p.as_dvec3())
        .filter(|p| p.z <= zmax)
        .map(|p| warp.map_or(p, |w| w.to_warp(p)))
        .collect();
    let pad = outer + BORDER_BAND_MM;
    let mut best = vec![f64::INFINITY; q.len()];
    let mut owner = vec![usize::MAX; q.len()];
    let mut pos = vec![0.0; q.len()];
    for (k, (_, l)) in fs.iter().enumerate() {
        let e = &l.points;
        let lo = e
            .iter()
            .fold(DVec2::splat(f64::INFINITY), |a, p| a.min(p.truncate()))
            - pad;
        let hi = e
            .iter()
            .fold(DVec2::splat(f64::NEG_INFINITY), |a, p| a.max(p.truncate()))
            + pad;
        let zt = ztop(l);
        for (j, p) in q.iter().enumerate() {
            let pl = p.truncate();
            if !(pl.cmpge(lo).all() && pl.cmple(hi).all() && p.z <= zt) {
                continue;
            }
            let (dep, at) = loop_place(e, pl);
            if -dep <= pad && dep <= BORDER_INSIDE_MM && -dep < best[j] {
                (best[j], owner[j], pos[j]) = (-dep, k, at);
            }
        }
    }
    fs.iter()
        .enumerate()
        .map(|(k, (f, l))| {
            let n = l.points.len();
            let mut z = vec![f64::INFINITY; n];
            for j in (0..q.len()).filter(|&j| owner[j] == k) {
                let a = pos[j].round_ties_even() as usize % n;
                z[a] = z[a].min(q[j].z);
            }
            (
                *f,
                z.into_iter()
                    .map(|v| if v.is_finite() { v } else { f64::NAN })
                    .collect(),
            )
        })
        .collect()
}

/// How far beyond its equator the border of the shell goes on every tooth
/// (`border_offsets`): the equator less the border, median of the azimuths
/// facing buccal or lingual (not the proximal sides, held for the join with
/// a neighbour).
pub fn border_offsets(
    loops: &[(Fdi, &ToothLoop)],
    border: &BTreeMap<Fdi, Vec<f64>>,
    order: &[Fdi],
) -> BTreeMap<Fdi, f64> {
    let cs: Vec<(Fdi, DVec2)> = loops
        .iter()
        .filter(|(_, l)| !l.equator.is_empty())
        .map(|(f, l)| (*f, l.centre))
        .collect();
    let mut out = BTreeMap::new();
    for (f, u) in arch_normals(&cs, order) {
        let (Some(b), Some((_, l))) = (border.get(&f), loops.iter().find(|(g, _)| *g == f)) else {
            continue;
        };
        if b.len() != l.equator.len() {
            continue;
        }
        let sel = facing(b.len(), u, true);
        let mut d: Vec<f64> = (0..b.len())
            .filter(|&k| sel[k] && b[k].is_finite())
            .map(|k| l.equator[k] - b[k])
            .collect();
        if !d.is_empty() {
            out.insert(f, round2(median(&mut d)));
        }
    }
    out
}

/// The checks of the EQUATOR border of the shell `mesh` (arch coordinates):
/// `loops` the trim lines of the covered teeth (warped space of `warp`),
/// `order` the arch order, `offset` the equator offset asked, `outer` the
/// outer offset of the shell; `common`: the axis is COMMON.
pub fn checks(
    loops: &[(Fdi, &ToothLoop)],
    mesh: &TriMesh,
    warp: Option<&PathWarp>,
    order: &[Fdi],
    offset: f64,
    outer: f64,
    common: bool,
) -> Checks {
    let mut c = Checks {
        labial: labial_cover(loops, None, order),
        ..Checks::default()
    };
    let border = border_heights(loops, mesh, warp, outer);
    if !border.is_empty() {
        let on_border = labial_cover(loops, Some(&border), order);
        if !on_border.is_empty() {
            c.labial = on_border;
        }
        let dirs = labial_dirs(loops, order);
        for f in c.labial.keys() {
            let (Some(b), Some(u), Some((_, l))) = (
                border.get(f),
                dirs.get(f),
                loops.iter().find(|(g, _)| g == f),
            ) else {
                continue;
            };
            let lab = facing(l.points.len(), *u, false);
            let mut rise: Vec<f64> = (0..lab.len())
                .filter(|&k| lab[k] && b[k].is_finite())
                .map(|k| b[k] - l.points[k].z)
                .collect();
            let rise = if rise.is_empty() {
                0.0
            } else {
                median(&mut rise)
            };
            if rise > BORDER_RAISED_MM {
                c.labial_raised.push((*f, round2(rise)));
            }
        }
        c.offsets = border_offsets(loops, &border, order);
        if offset >= 0.1 {
            let rank = |f: &Fdi| order.iter().position(|g| g == f).unwrap_or(99);
            let mut lim: Vec<(Fdi, f64)> = c
                .offsets
                .iter()
                .filter(|(_, mm)| **mm < OFFSET_LIMIT_SHARE * offset)
                .map(|(f, mm)| (*f, *mm))
                .collect();
            lim.sort_by_key(|(f, _)| rank(f));
            c.offset_limited = lim;
        }
    }
    if common {
        let s = warp
            .and_then(|w| w.shears.first().copied())
            .unwrap_or(DVec2::ZERO);
        c.common_tilt = Some(round2((s.x * s.x + s.y * s.y).sqrt().atan().to_degrees()));
    }
    c.labial_low = c
        .labial
        .iter()
        .filter(|(_, (mm, sh))| *mm < LABIAL_WARN_MM || *sh < LABIAL_WARN_SHARE)
        .map(|(f, (mm, sh))| (*f, *mm, *sh))
        .collect();
    c
}
