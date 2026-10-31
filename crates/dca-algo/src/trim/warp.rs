//! Insertion paths of the aligner and the space warped so that they are
//! vertical (`aligner.PathWarp`; `case_print.insertion_paths`,
//! `_common_path`, `_chain_paths`, `insertion_warp`, `_survey_cover`,
//! `_labial_share`).
//!
//! VERTICAL: the axis of the arch (`+Z`) for every tooth, no warp. COMMON:
//! one rigid path for the whole arch, tilted as on a dental surveyor.
//! PER_TOOTH (the plugin's default): the path of every tooth that leaves
//! the least of its crown below the survey line, within the tilt limits and
//! at most a few degrees from its neighbours'.

use std::collections::BTreeMap;

use dc_case::Fdi;
use glam::{DVec2, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::line::{EQUATOR_BAND_MM, EQUATOR_SIDE_MM, EQUATOR_TOP_MM};
use super::survey::{loop_z, median, survey_loop};
use crate::evidence;

/// PER_TOOTH: tilts sampled in steps of this (degrees) ...
pub const EQUATOR_TILT_STEP_DEG: f64 = 5.0;
/// ... and this many directions ...
pub const EQUATOR_TILT_DIRS: usize = 24;
/// ... and a path leans only where every degree uncovers this share of the
/// crown less.
pub const EQUATOR_TILT_COST: f64 = 0.004;
/// An incisor or canine whose path leaves less than this share of its
/// labial face above the survey line ...
pub const EQUATOR_LABIAL_FLOOR: f64 = 0.6;
/// ... costs this per unit missing (PER_TOOTH).
pub const EQUATOR_LABIAL_COST: f64 = 1.0;
/// The paths of neighbouring teeth blend over this (mm, plan).
pub const EQUATOR_PATH_SIGMA_MM: f64 = 3.0;
/// The paths and the height range of the warp are surveyed on crown points
/// this coarse (mm).
pub const EQUATOR_SURVEY_STEP_MM: f64 = 0.8;
/// COMMON: tilts sampled in steps of this (degrees) ...
pub const EQUATOR_COMMON_STEP_DEG: f64 = 2.5;
/// ... and this many directions ...
pub const EQUATOR_COMMON_DIRS: usize = 16;
/// ... leaning only where every degree uncovers this share of the crowns
/// less ...
pub const EQUATOR_COMMON_COST: f64 = 0.001;
/// ... the least covered labial face under the floor costs this per unit
/// ...
pub const EQUATOR_COMMON_LABIAL_COST: f64 = 0.25;
/// ... and the difference of the right and left teeth of a kind this.
pub const EQUATOR_COMMON_PAIR_COST: f64 = 0.25;

/// The insertion axis of the aligners (`insertion_axis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsertionAxis {
    /// One axis for the whole arch, perpendicular to the occlusal plane.
    Vertical,
    /// One rigid axis for the whole arch, tilted as on a surveyor.
    Common,
    /// Every tooth its own axis (the aligner flexes between them).
    #[default]
    PerTooth,
}

/// Shear of space in which the insertion path of every tooth is vertical
/// (`aligner.PathWarp`): a warped point `q` stands for the point
/// `(q.xy + h(q.z) s(q.xy), q.z)`, `s` the shear of the paths blended
/// between the teeth with Gaussian weights of the plan distance from their
/// centres (fading to 0 far from every tooth), `h(z) = clip(z, zlo, zhi) -
/// z0`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathWarp {
    /// Plan centres of the teeth.
    pub centres: Vec<DVec2>,
    /// Shear of the path of every tooth (direction `(s, 1)`).
    pub shears: Vec<DVec2>,
    /// Height of no shift.
    pub z0: f64,
    /// Heights the shift is clipped to.
    pub zlo: f64,
    /// See `zlo`.
    pub zhi: f64,
    /// Blend (mm).
    pub sigma: f64,
    eps: f64,
}

impl PathWarp {
    /// A warp (`fade`: in sigmas, where the shear fades to the arch axis).
    pub fn new(
        centres: Vec<DVec2>,
        shears: Vec<DVec2>,
        z0: f64,
        zlo: f64,
        zhi: f64,
        sigma: f64,
        fade: f64,
    ) -> Self {
        Self {
            centres,
            shears,
            z0,
            zlo,
            zhi,
            sigma,
            eps: (-0.5 * fade * fade).exp(),
        }
    }

    /// Shear at the warped plan point `q`.
    pub fn shear(&self, q: DVec2) -> DVec2 {
        let (mut num, mut den) = (DVec2::ZERO, 0.0);
        for (c, s) in self.centres.iter().zip(&self.shears) {
            let w = (-0.5 * (q - *c).length_squared() / (self.sigma * self.sigma)).exp();
            num += w * *s;
            den += w;
        }
        num / (den + self.eps)
    }

    fn h(&self, z: f64) -> f64 {
        z.clamp(self.zlo, self.zhi) - self.z0
    }

    /// The arch point of the warped point `q`.
    pub fn from_warp(&self, q: DVec3) -> DVec3 {
        let s = self.shear(q.truncate());
        (q.truncate() + self.h(q.z) * s).extend(q.z)
    }

    /// The warped point of the arch point `p` (fixed point of
    /// [`PathWarp::from_warp`], 8 iterations).
    pub fn to_warp(&self, p: DVec3) -> DVec3 {
        let h = self.h(p.z);
        let mut q = p.truncate();
        for _ in 0..8 {
            q = p.truncate() - h * self.shear(q);
        }
        q.extend(p.z)
    }
}

/// The points thinned out to the first one of every `step` mm cell
/// (`_crown_cells`, here on the cells of the arch coordinates).
pub fn crown_cells(p: &[DVec3], step: f64) -> Vec<DVec3> {
    let mut seen = std::collections::HashSet::new();
    p.iter()
        .copied()
        .filter(|q| {
            let k = (q / step).floor().as_i64vec3();
            seen.insert((k.x, k.y, k.z))
        })
        .collect()
}

fn sheared(p: &[DVec3], shear: DVec2) -> Vec<DVec3> {
    let mut z: Vec<f64> = p.iter().map(|q| q.z).collect();
    let m = median(&mut z);
    p.iter()
        .map(|q| (q.truncate() - (q.z - m) * shear).extend(q.z))
        .collect()
}

/// Share of the crown points above their survey line along the path
/// `(shear, 1)` (`_survey_cover`).
pub fn survey_cover(p: &[DVec3], shear: DVec2, n: usize) -> f64 {
    let q = sheared(p, shear);
    let s = survey_loop(&q, n, EQUATOR_BAND_MM, EQUATOR_SIDE_MM, None);
    let plan: Vec<DVec2> = q.iter().map(|v| v.truncate()).collect();
    let lz = loop_z(&s.loop_points, s.centre, &plan);
    q.iter().zip(&lz).filter(|(v, z)| v.z >= *z - 0.05).count() as f64 / q.len() as f64
}

/// Share of the labial face of the crown above its survey line along the
/// path (`_labial_share`): the median over the directions within 53 degrees
/// of the labial direction `u` of (top - survey line) / crown height.
pub fn labial_share(p: &[DVec3], shear: DVec2, u: DVec2, n: usize) -> f64 {
    let q = sheared(p, shear);
    let s = survey_loop(&q, n, EQUATOR_BAND_MM, EQUATOR_SIDE_MM, None);
    let top = q.iter().map(|v| v.z).fold(f64::NEG_INFINITY, f64::max);
    let low = q.iter().map(|v| v.z).fold(f64::INFINITY, f64::min);
    let mut vals: Vec<f64> = s
        .dirs
        .iter()
        .zip(&s.loop_points)
        .filter(|(d, _)| d.dot(u) > 0.6)
        .map(|(_, e)| {
            let z = e.z.min(top - EQUATOR_TOP_MM).max(low);
            (top - z) / (top - low)
        })
        .collect();
    if vals.is_empty() || top - low < 1e-6 {
        return 1.0;
    }
    median(&mut vals)
}

/// Shear of the path leaning `th` (radians) towards the azimuth `ph`.
pub fn tilted(th: f64, ph: f64) -> DVec2 {
    th.tan() * DVec2::new(ph.cos(), ph.sin())
}

fn plan_mean(p: &[DVec3]) -> DVec2 {
    p.iter().map(|q| q.truncate()).sum::<DVec2>() / p.len() as f64
}

/// Shear of the one rigid insertion path of the arch (COMMON,
/// `_common_path`): the path leaning at most `insertion_tilt_common` that
/// leaves the least crown below the survey lines (mean over the teeth), the
/// labial faces of the incisors and canines balanced.
pub fn common_path(crowns: &BTreeMap<Fdi, Vec<DVec3>>) -> DVec2 {
    let max_deg = evidence::param_f64("insertion_tilt_common").unwrap_or(20.0);
    let crowns: BTreeMap<Fdi, &Vec<DVec3>> = crowns
        .iter()
        .filter(|(_, p)| p.len() >= 20)
        .map(|(f, p)| (*f, p))
        .collect();
    if crowns.is_empty() || max_deg <= 0.0 {
        return DVec2::ZERO;
    }
    let mid = crowns.values().map(|p| plan_mean(p)).sum::<DVec2>() / crowns.len() as f64;
    let mut front: BTreeMap<Fdi, (&Vec<DVec3>, DVec2)> = BTreeMap::new();
    for (f, p) in &crowns {
        let u = plan_mean(p) - mid;
        if f.get() % 10 <= 3 && u.length() > 1e-6 {
            front.insert(*f, (p, u / u.length()));
        }
    }
    let pairs: Vec<(Fdi, Fdi)> = front
        .keys()
        .filter(|f| matches!(f.get() / 10, 1 | 4))
        .filter_map(|f| {
            let b = Fdi::new(if f.get() / 10 == 1 { 20 } else { 30 } + f.get() % 10).ok()?;
            front.contains_key(&b).then_some((*f, b))
        })
        .collect();
    let score = |th: f64, ph: f64| {
        let sh = tilted(th, ph);
        let mut sc = crowns
            .values()
            .map(|p| survey_cover(p, sh, 24))
            .sum::<f64>()
            / crowns.len() as f64
            - EQUATOR_COMMON_COST * th.to_degrees();
        if !front.is_empty() {
            let lab: BTreeMap<Fdi, f64> = front
                .iter()
                .map(|(f, (p, u))| (*f, labial_share(p, sh, *u, 24)))
                .collect();
            let lmin = lab.values().copied().fold(f64::INFINITY, f64::min);
            sc -= EQUATOR_COMMON_LABIAL_COST * (EQUATOR_LABIAL_FLOOR - lmin).max(0.0);
            if !pairs.is_empty() {
                sc -= EQUATOR_COMMON_PAIR_COST
                    * pairs
                        .iter()
                        .map(|(a, b)| (lab[a] - lab[b]).abs())
                        .sum::<f64>()
                    / pairs.len() as f64;
            }
        }
        sc
    };
    let step = EQUATOR_COMMON_STEP_DEG.to_radians();
    let top = max_deg.to_radians();
    let dph = std::f64::consts::TAU / EQUATOR_COMMON_DIRS as f64;
    let (mut best, mut th0, mut ph0) = (score(0.0, 0.0), 0.0, 0.0);
    let mut th = step;
    while th <= top + 1e-9 {
        for k in 0..EQUATOR_COMMON_DIRS {
            let ph = dph * k as f64;
            let sc = score(th, ph);
            if sc > best + 1e-9 {
                (best, th0, ph0) = (sc, th, ph);
            }
        }
        th += step;
    }
    if th0 > 0.0 {
        let (t0, p0) = (th0, ph0);
        for th in [t0 - 0.5 * step, t0, (t0 + 0.5 * step).min(top)] {
            for ph in [p0 - 0.5 * dph, p0, p0 + 0.5 * dph] {
                if th > 0.0 && (th, ph) != (t0, p0) {
                    let sc = score(th, ph);
                    if sc > best + 1e-9 {
                        (best, th0, ph0) = (sc, th, ph);
                    }
                }
            }
        }
    }
    tilted(th0, ph0)
}

/// Index of the path of every tooth of a chain maximising the sum of the
/// scores with consecutive paths at most `max_deg` apart (`_chain_paths`,
/// dynamic programming; ties to the earlier candidate).
pub fn chain_paths(cands: &[(Vec<DVec2>, Vec<f64>)], max_deg: f64) -> Vec<usize> {
    if cands.is_empty() {
        return Vec::new();
    }
    let cos_lim = if (0.0..180.0).contains(&max_deg) {
        max_deg.to_radians().cos() - 1e-12
    } else {
        -2.0
    };
    let unit = |s: &DVec2| s.extend(1.0).normalize();
    let mut val = cands[0].1.clone();
    let mut back: Vec<Vec<usize>> = Vec::new();
    for w in cands.windows(2) {
        let (sp, (sc_s, sc)) = (&w[0].0, (&w[1].0, &w[1].1));
        let mut next = vec![0.0; sc.len()];
        let mut bj = vec![0usize; sc.len()];
        for (i, s) in sc_s.iter().enumerate() {
            let us = unit(s);
            let (mut best, mut j) = (f64::NEG_INFINITY, 0);
            for (k, p) in sp.iter().enumerate() {
                let v = if us.dot(unit(p)) >= cos_lim {
                    val[k]
                } else {
                    f64::NEG_INFINITY
                };
                if v > best {
                    (best, j) = (v, k);
                }
            }
            next[i] = sc[i] + best;
            bj[i] = j;
        }
        val = next;
        back.push(bj);
    }
    let mut first = 0;
    for (i, v) in val.iter().enumerate() {
        if *v > val[first] {
            first = i;
        }
    }
    let mut pick = vec![first];
    for b in back.iter().rev() {
        pick.push(b[*pick.last().unwrap()]);
    }
    pick.reverse();
    pick
}

/// Insertion path of every tooth: `{fdi: (shear, plan centre)}`
/// (`insertion_paths`). `crowns`: the crown points of every tooth (thinned
/// to [`EQUATOR_SURVEY_STEP_MM`]); `order`: the arch order.
pub fn insertion_paths(
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
    axis: InsertionAxis,
    order: &[Fdi],
) -> BTreeMap<Fdi, (DVec2, DVec2)> {
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = crowns
        .iter()
        .filter(|(_, p)| p.len() >= 20)
        .map(|(f, p)| (*f, p.clone()))
        .collect();
    if axis != InsertionAxis::PerTooth {
        let sh = if axis == InsertionAxis::Common {
            common_path(&crowns)
        } else {
            DVec2::ZERO
        };
        return crowns
            .iter()
            .map(|(f, p)| (*f, (sh, plan_mean(p))))
            .collect();
    }
    let ant = evidence::param_f64("insertion_tilt_anterior").unwrap_or(30.0);
    let post = evidence::param_f64("insertion_tilt_posterior").unwrap_or(10.0);
    let neigh = evidence::param_f64("insertion_tilt_neighbour").unwrap_or(10.0);
    let phis: Vec<f64> = (0..EQUATOR_TILT_DIRS)
        .map(|k| std::f64::consts::TAU * k as f64 / EQUATOR_TILT_DIRS as f64)
        .collect();
    let mid = if crowns.is_empty() {
        DVec2::ZERO
    } else {
        crowns.values().map(|p| plan_mean(p)).sum::<DVec2>() / crowns.len() as f64
    };
    let mut cands: BTreeMap<Fdi, (Vec<DVec2>, Vec<f64>)> = BTreeMap::new();
    for (f, p) in &crowns {
        let top = if f.get() % 10 <= 3 { ant } else { post };
        let mut tilts = Vec::new();
        if top > 0.0 {
            let n = ((top + 1e-6 - EQUATOR_TILT_STEP_DEG) / EQUATOR_TILT_STEP_DEG)
                .ceil()
                .max(0.0) as usize;
            tilts = (0..n)
                .map(|i| (EQUATOR_TILT_STEP_DEG + i as f64 * EQUATOR_TILT_STEP_DEG).to_radians())
                .collect();
        }
        let tp: Vec<(f64, f64)> = tilts
            .iter()
            .flat_map(|&th| phis.iter().map(move |&ph| (th, ph)))
            .collect();
        let mut s = vec![DVec2::ZERO];
        s.extend(tp.iter().map(|&(th, ph)| tilted(th, ph)));
        let mut sc = vec![survey_cover(p, s[0], 24)];
        sc.extend(
            s[1..].iter().zip(&tp).map(|(sh, (th, _))| {
                survey_cover(p, *sh, 24) - EQUATOR_TILT_COST * th.to_degrees()
            }),
        );
        let u = plan_mean(p) - mid;
        if f.get() % 10 <= 3 && EQUATOR_LABIAL_COST > 0.0 && u.length() > 1e-6 {
            let u = u / u.length();
            for (k, sh) in s.iter().enumerate() {
                let lab = labial_share(p, *sh, u, 24);
                sc[k] += EQUATOR_LABIAL_COST * (lab - EQUATOR_LABIAL_FLOOR).min(0.0);
            }
        }
        cands.insert(*f, (s, sc));
    }
    let mut seq: Vec<Fdi> = order
        .iter()
        .copied()
        .filter(|f| cands.contains_key(f))
        .collect();
    seq.extend(cands.keys().copied().filter(|f| !order.contains(f)));
    let chain: Vec<(Vec<DVec2>, Vec<f64>)> = seq.iter().map(|f| cands[f].clone()).collect();
    let pick = chain_paths(&chain, neigh);
    seq.iter()
        .zip(pick)
        .map(|(f, j)| (*f, (cands[f].0[j], plan_mean(&crowns[f]))))
        .collect()
}

/// The warp of the insertion paths (`insertion_warp`), `None` when every
/// path is the axis of the arch (always with VERTICAL). COMMON: one shear
/// everywhere.
pub fn insertion_warp(
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
    axis: InsertionAxis,
    order: &[Fdi],
) -> Option<PathWarp> {
    if axis == InsertionAxis::Vertical {
        return None;
    }
    let thin: BTreeMap<Fdi, Vec<DVec3>> = crowns
        .iter()
        .map(|(f, p)| (*f, crown_cells(p, EQUATOR_SURVEY_STEP_MM)))
        .collect();
    let paths = insertion_paths(&thin, axis, order);
    if paths.is_empty() || paths.values().all(|(s, _)| s.abs().max_element() < 1e-6) {
        return None;
    }
    let mut z: Vec<f64> = thin.values().flatten().map(|q| q.z).collect();
    let zlo = z.iter().copied().fold(f64::INFINITY, f64::min) - 1.0;
    let zhi = z.iter().copied().fold(f64::NEG_INFINITY, f64::max) + 1.0;
    let z0 = median(&mut z);
    if axis == InsertionAxis::Common {
        let sh = paths.values().next().unwrap().0;
        return Some(PathWarp::new(
            vec![DVec2::ZERO],
            vec![sh],
            z0,
            zlo,
            zhi,
            1e9,
            50.0,
        ));
    }
    let (c, s): (Vec<DVec2>, Vec<DVec2>) = paths.values().map(|(s, c)| (*c, *s)).unzip();
    Some(PathWarp::new(
        c,
        s,
        z0,
        zlo,
        zhi,
        EQUATOR_PATH_SIGMA_MM,
        3.0,
    ))
}
