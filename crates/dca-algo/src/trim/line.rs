//! The trim line of every mode: the equator loops of the crowns (EQUATOR)
//! and the control points along the gingival margin (STRAIGHT, SCALLOPED),
//! `case_print.tooth_equators`, `equator_trim`, `trim_points` and
//! `_border_floor`.

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use dc_case::Fdi;
use glam::{DVec2, DVec3};

use super::Tooth;
use super::survey::{
    angle_diff, circular_smooth, interp_periodic, loop_smooth_plan, round_tabs, slope_floor,
    survey_loop,
};

/// Directions around every crown sampled for its equator.
pub const EQUATOR_AZIMUTHS: usize = 48;
/// Support of a direction: points within this of the extreme along it ...
pub const EQUATOR_BAND_MM: f64 = 0.15;
/// ... and within this of the extreme across it (the local height of contour).
pub const EQUATOR_SIDE_MM: f64 = 1.0;
/// The equator (also moved by the offset) stays at least this below the top
/// of the crown.
pub const EQUATOR_TOP_MM: f64 = 1.0;
/// The tab rule measures the loop smoothed in plan by this many (1, 2, 1)
/// passes.
pub const EQUATOR_PLAN_PASSES: usize = 1;
/// The gingival margin, the floor of the equator, rises at most this between
/// two azimuths.
pub const EQUATOR_FLOOR_SLOPE_MM: f64 = 0.5;
/// The border leaves no tab of crown covered narrower than 2 x this x the
/// wall.
pub const EQUATOR_TAB_WALL: f64 = 1.0;
/// ... the disks resting on the loop this many times more finely than its
/// points.
pub const EQUATOR_TAB_FINE: usize = 4;
/// A direction along which the scanned crown ends this far inside the
/// closed tooth takes the equator of the closed tooth ...
pub const EQUATOR_SCAN_GAP_MM: f64 = 1.0;
/// ... its points at most this below the gingival margin at their azimuth.
pub const EQUATOR_SCAN_GUM_MM: f64 = 0.2;
/// Beside the crowns the underside of the wall stays this far above the
/// gingiva; the equator never goes below the margin plus this.
pub const EQUATOR_GUM_GAP_MM: f64 = 0.1;

/// Equator of one tooth with what the trim surface needs of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToothLoop {
    /// The loop, one point per azimuth around the plan centre.
    pub points: Vec<DVec3>,
    /// Height of the equator before the offset (EQUATOR border only).
    pub equator: Vec<f64>,
    /// Gingival margin height at every azimuth, when the tooth has a margin.
    pub margin: Option<Vec<f64>>,
    /// Top of the scanned crown.
    pub top: f64,
    /// Plan centre of the crown.
    pub centre: DVec2,
}

/// Height of the gingival margin `m` at the `n` azimuths around `c`
/// (`tooth_equators`): the highest margin point within a sector, the
/// nearest one where a sector is empty.
fn margin_heights(m: &[DVec3], c: DVec2, n: usize) -> Vec<f64> {
    let a: Vec<f64> = m.iter().map(|p| (p.y - c.y).atan2(p.x - c.x)).collect();
    (0..n)
        .map(|k| {
            let ang = TAU * k as f64 / n as f64;
            let mut best = f64::NEG_INFINITY;
            let mut near = (f64::INFINITY, 0usize);
            for (i, &ai) in a.iter().enumerate() {
                let d = angle_diff(ai, ang);
                if d <= TAU / n as f64 {
                    best = best.max(m[i].z);
                }
                if d < near.0 {
                    near = (d, i);
                }
            }
            if best.is_finite() { best } else { m[near.1].z }
        })
        .collect()
}

/// Equator (survey line, height of contour) of a tooth along `+Z`
/// (`tooth_equators`, the vertical insertion axis): for every direction
/// across the path the scanned crown points farthest along it are where the
/// surface turns from facing up to facing down; where the crown was not
/// scanned the closed tooth gives it. Never below the gingival margin at
/// that azimuth (raised where it climbs more than
/// [`EQUATOR_FLOOR_SLOPE_MM`] between two azimuths) nor within
/// [`EQUATOR_TOP_MM`] of the top of the crown; smoothed along the loop.
/// `None` for a crown with fewer than 20 points.
pub fn tooth_equator(tooth: &Tooth, n: usize) -> Option<ToothLoop> {
    let p = &tooth.crown;
    if p.len() < 20 {
        return None;
    }
    let s = survey_loop(p, n, EQUATOR_BAND_MM, EQUATOR_SIDE_MM, None);
    let mut e = s.loop_points;
    let c = s.centre;
    let top = p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
    let low = p.iter().map(|q| q.z).fold(f64::INFINITY, f64::min);
    let zm = (tooth.margin.len() >= 3).then(|| margin_heights(&tooth.margin, c, n));
    // where the crown was not scanned the closed tooth gives the equator (its crown part only)
    let ang: Vec<f64> = (0..n).map(|k| TAU * k as f64 / n as f64).collect();
    let mut pc: Vec<DVec3> = tooth
        .closed
        .iter()
        .copied()
        .filter(|q| q.z >= low - 0.5)
        .collect();
    if let Some(zm) = &zm
        && !pc.is_empty()
    {
        let az: Vec<f64> = pc.iter().map(|q| (q.y - c.y).atan2(q.x - c.x)).collect();
        let floor = interp_periodic(&az, &ang, zm, TAU);
        pc = pc
            .into_iter()
            .zip(floor)
            .filter(|(q, f)| q.z >= f - EQUATOR_SCAN_GUM_MM)
            .map(|(q, _)| q)
            .collect();
    }
    if pc.len() >= 20 {
        let sc = survey_loop(&pc, n, EQUATOR_BAND_MM, EQUATOR_SIDE_MM, Some(c));
        for (k, q) in e.iter_mut().enumerate() {
            if sc.extent[k] - s.extent[k] > EQUATOR_SCAN_GAP_MM {
                *q = sc.loop_points[k];
            }
        }
    }
    let zf = zm.as_ref().map(|z| slope_floor(z, EQUATOR_FLOOR_SLOPE_MM));
    let clamp = |z: &[f64]| -> Vec<f64> {
        z.iter()
            .enumerate()
            .map(|(k, &v)| {
                zf.as_ref()
                    .map_or(v, |f| v.max(f[k]))
                    .min(top - EQUATOR_TOP_MM)
            })
            .collect()
    };
    let z0: Vec<f64> = e.iter().map(|q| q.z).collect();
    let z = clamp(&circular_smooth(&clamp(&z0), 5));
    for (q, z) in e.iter_mut().zip(z) {
        q.z = z;
    }
    Some(ToothLoop {
        equator: e.iter().map(|q| q.z).collect(),
        points: e,
        margin: zm,
        top,
        centre: c,
    })
}

/// Trim line of the EQUATOR border of one tooth (`equator_trim`): the
/// equator moved along the insertion path by `offset` (positive = beyond
/// the equator towards the gingiva, into the undercut), its tabs rounded
/// (none narrower than `2 x EQUATOR_TAB_WALL x wall`, along the loop
/// smoothed in plan), then again at least [`EQUATOR_TOP_MM`] below the top
/// of the crown but never below the gingival margin +
/// [`EQUATOR_GUM_GAP_MM`].
pub fn equator_trim(eq: &ToothLoop, offset: f64, wall: f64) -> ToothLoop {
    let r = EQUATOR_TAB_WALL * wall;
    let mut out = eq.clone();
    out.equator = eq.points.iter().map(|q| q.z).collect();
    let plan: Vec<DVec2> = eq.points.iter().map(|q| q.truncate()).collect();
    let smooth = loop_smooth_plan(&plan, eq.centre, EQUATOR_PLAN_PASSES);
    let f: Vec<DVec3> = smooth
        .iter()
        .zip(&eq.points)
        .map(|(p, q)| p.extend(q.z - offset))
        .collect();
    let z = round_tabs(&f, r, EQUATOR_TAB_FINE);
    for (k, q) in out.points.iter_mut().enumerate() {
        let mut v = z[k].min(eq.top - EQUATOR_TOP_MM);
        if let Some(zm) = &eq.margin {
            v = v.max((zm[k] + EQUATOR_GUM_GAP_MM).min(eq.top - EQUATOR_GUM_GAP_MM));
        }
        q.z = v;
    }
    out
}

/// The equator trim loops of the covered teeth and the equators of the
/// teeth left out (no shell over them), by FDI number.
pub fn equator_loops(
    teeth: &[Tooth],
    offset: f64,
    wall: f64,
) -> (BTreeMap<Fdi, ToothLoop>, BTreeMap<Fdi, ToothLoop>) {
    let mut covered = BTreeMap::new();
    let mut out = BTreeMap::new();
    for t in teeth {
        let Some(eq) = tooth_equator(t, EQUATOR_AZIMUTHS) else {
            continue;
        };
        if t.covered {
            covered.insert(t.fdi, equator_trim(&eq, offset, wall));
        } else {
            out.insert(t.fdi, eq);
        }
    }
    (covered, out)
}

/// Margin-based modes of the control points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginMode {
    /// Every side of a tooth at the height of its gingival zenith.
    Straight,
    /// Following the margin.
    Scalloped,
}

/// Control points of the STRAIGHT and SCALLOPED trim lines (`trim_points`):
/// the gingival margin of every tooth lowered by `extension`; STRAIGHT, the
/// lowest margin point of the buccal and of the lingual side of every tooth
/// (its gingival zenith, Elshazly 2022/2023), SCALLOPED, the margin itself
/// (at most about 60 points a tooth). Returns the points of every tooth.
pub fn margin_points(teeth: &[Tooth], mode: MarginMode, extension: f64) -> Vec<(Fdi, Vec<DVec3>)> {
    let mut out = Vec::new();
    for t in teeth {
        if t.margin.len() < 3 {
            continue;
        }
        let step = (t.margin.len() / 60).max(1);
        let p: Vec<DVec3> = t.margin.iter().step_by(step).copied().collect();
        match mode {
            MarginMode::Scalloped => {
                out.push((t.fdi, p.iter().map(|q| q - DVec3::Z * extension).collect()));
            }
            MarginMode::Straight => {
                let buccal: Vec<bool> = p
                    .iter()
                    .map(|q| (*q - t.origin).dot(t.e_bl) > 0.0)
                    .collect();
                let mut pts = Vec::new();
                for want in [true, false] {
                    let zen = p
                        .iter()
                        .zip(&buccal)
                        .filter(|&(_, &b)| b == want)
                        .map(|(q, _)| *q)
                        .reduce(|a, b| if b.z < a.z { b } else { a });
                    if let Some(z) = zen {
                        pts.push(z - DVec3::Z * extension);
                    }
                }
                out.push((t.fdi, pts));
            }
        }
    }
    out
}

/// Trim points `c` kept at least 1 mm above the border of the scan where it
/// ends in the gingiva just under them, less than `ext` mm below them
/// (`_border_floor`): the line never lies on the base added to close the
/// model.
pub fn border_floor(c: &mut [DVec3], border: &[DVec3], ext: f64) {
    if border.is_empty() {
        return;
    }
    for p in c.iter_mut() {
        let floor = border
            .iter()
            .filter(|b| (b.truncate() - p.truncate()).length() < 4.0 && b.z < p.z + ext)
            .map(|b| b.z)
            .fold(f64::NEG_INFINITY, f64::max);
        p.z = p.z.max(floor + 1.0);
    }
}
