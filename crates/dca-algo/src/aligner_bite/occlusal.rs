//! Bite with the aligners worn, in a case: the occlusal surfaces of the two
//! aligners of a stage, the closing of the mouth on them, the automatic
//! occlusal thickness, the equilibration, and the shaping of the shell.
//!
//! Reference: `ortho_aligner/case_occlusal.py`. The Blender objects of the
//! plugin are replaced by plain inputs ([`ArchStage`], in arch coordinates,
//! `+Z` occlusal): the caller builds them for a stage from the scanned
//! crowns, the closed teeth and the ramps ([`crate::ramps`]). Nothing is
//! cached here.
//!
//! The two arches go together as in the synchronised series: aligner `k` of
//! one arch with aligner `min(k, n)` of the other ([`stage_pair`]). The
//! relation of the arches is the one of the scans (registered bite, scans in
//! occlusion or virtual occlusion); the closing starts there.
//!
//! Occlusal thickness ([`OcclusalMode`]): `Same` the whole shell of the wall
//! thickness (no shaping); `Manual` the thickness of the settings; `Auto`
//! the thickness, the same for both aligners of the stage, that brings the
//! opening of the bite caused by the aligners at the first molars to the
//! target ([`solve_thickness`]), never under the minimum thickness nor over
//! the wall.
//!
//! Deviations from the plugin: the report keys are English (the text is the
//! plugin's Italian); the nearest-point queries run on single-precision
//! k-d trees ([`dc_geom::knn`], [`dc_geom::bvh`]) instead of the GPU
//! queries; the bite is not cached.

use dc_geom::bvh::Bvh;
use dc_geom::knn::KnnIndex;
use dc_mesh::TriMesh;
use dca_model::{BiteClosing, OcclusalMode, OcclusalSettings, Settings, TrimMode, TrimParams};
use glam::{DMat4, DVec3, Vec3};
use serde::Serialize;

use super::{
    Base, Closing, Equilibration, Front, Grid, Raster, Surface, bare_surface, base_from_heights,
    closing, envelope, equilibrate, heights, hinge_axis, is_front, opening_at, overlap_mask,
    solve_thickness, with_occlusal,
};
use crate::evidence;
use crate::occlusion::percentile;

/// Grid of the occlusal surfaces (mm); the voxel of the shell is 0.1-0.15.
pub const CELL: f64 = 0.25;
/// Spacing of the crown points (mm), finer than the grid.
pub const STEP: f64 = 0.2;
/// Margin around the crowns, beyond clearance and wall (mm).
pub const MARGIN: f64 = 3.0;
/// The bottom of the limit solid under the lowest crown point (mm).
pub const CAP_BELOW: f64 = 30.0;
/// An upper point this far under a lower tooth in the relation of the scans
/// is under it (overbite), not in it: left out of the vertical contacts (mm).
pub const OVERLAP_MM: f64 = 0.25;
/// Added to the radius of every front point under an aligner (the points are
/// 0.2 mm apart: two surfaces in contact have points about 0.1 mm apart);
/// none on bare teeth (mm).
pub const TOUCH_MM: f64 = 0.05;
/// Spacing of the occlusal thicknesses tried with the equilibration (mm).
pub const AUTO_STEP: f64 = 0.1;
/// Outer vertices whose normal has at least this `Z` are lowered
/// ([`shape_shell`]).
pub const UP_NZ: f64 = 0.3;
/// Blockout of the shell with the smooth border (`SMOOTH_BLOCKOUT_MM`).
pub const SMOOTH_BLOCKOUT_MM: f64 = 0.8;
/// Blockout of the shell otherwise (`aligner.BLOCKOUT_MM`).
pub const BLOCKOUT_MM: f64 = 0.3;

/// The design of the aligners the bite depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Design {
    /// Clearance between tooth and shell (mm).
    pub clearance: f64,
    /// Wall thickness of the shell (mm).
    pub wall: f64,
    /// Blockout ball of the shell (mm).
    pub blockout: f64,
    /// Occlusal thickness, closing and equilibration.
    pub occlusal: OcclusalSettings,
}

impl Design {
    /// The design of the case settings.
    pub fn new(settings: &Settings, occlusal: &OcclusalSettings, trim: &TrimParams) -> Self {
        Self {
            clearance: settings.clearance_mm,
            wall: settings.shell_thickness_mm,
            blockout: if trim.mode == TrimMode::Smooth {
                SMOOTH_BLOCKOUT_MM
            } else {
                BLOCKOUT_MM
            },
            occlusal: *occlusal,
        }
    }
}

/// One arch at one stage, in its arch coordinates.
#[derive(Debug, Clone, Default)]
pub struct ArchStage {
    /// Points of the scanned crowns, about one every [`STEP`] mm
    /// ([`crown_cells`]), tooth by tooth.
    pub crown_points: Vec<DVec3>,
    /// FDI of each crown point.
    pub crown_labels: Vec<i64>,
    /// Points of the model the shell is the offset of ([`model_points`]).
    pub model_points: Vec<DVec3>,
    /// FDI of each model point (0 farther than 1.5 mm from a crown point).
    pub model_labels: Vec<i64>,
    /// Teeth under the aligner (the others take part bare).
    pub covered: Vec<i64>,
    /// Points of the surfaces of the ramps ([`crate::ramps::points`]).
    pub ramp_points: Vec<DVec3>,
    /// FDI of each ramp point.
    pub ramp_labels: Vec<i64>,
}

/// Stages of the lower and of the upper arch that go with stage `k` of an
/// arch (`_stage_pair`): the arches with fewer aligners stay at their last.
pub fn stage_pair(k: u32, n_lower: u32, n_upper: u32) -> (u32, u32) {
    (k.min(n_lower), k.min(n_upper))
}

/// Points of `p` thinned out to the first one of every `step` cell, in their
/// order (`_crown_cells`).
pub fn crown_cells(p: &[DVec3], step: f64) -> Vec<DVec3> {
    let mut seen = std::collections::HashSet::new();
    p.iter()
        .filter(|q| {
            seen.insert((
                (q.x / step).floor() as i64,
                (q.y / step).floor() as i64,
                (q.z / step).floor() as i64,
            ))
        })
        .copied()
        .collect()
}

/// Points of the model of a stage: the used vertices and the centre of every
/// triangle of the closed teeth (arch coordinates), labelled with the FDI of
/// the nearest crown point (0 farther than 1.5 mm) (`_model_points`).
pub fn model_points(
    teeth: &[TriMesh],
    crown: &[DVec3],
    crown_labels: &[i64],
) -> (Vec<DVec3>, Vec<i64>) {
    let mut verts = Vec::new();
    let mut cents = Vec::new();
    for m in teeth {
        let mut used = vec![false; m.positions.len()];
        for t in &m.tris {
            let v: [DVec3; 3] = t.map(|i| m.positions[i as usize].as_dvec3());
            cents.push((v[0] + v[1] + v[2]) / 3.0);
            for &i in t {
                used[i as usize] = true;
            }
        }
        verts.extend(
            used.iter()
                .enumerate()
                .filter(|&(_, &u)| u)
                .map(|(i, _)| m.positions[i].as_dvec3()),
        );
    }
    verts.extend(cents);
    let labels = label_nearest(&verts, crown, crown_labels, 1.5);
    (verts, labels)
}

fn label_nearest(p: &[DVec3], crown: &[DVec3], crown_labels: &[i64], reach: f64) -> Vec<i64> {
    let pts: Vec<Vec3> = crown.iter().map(|q| q.as_vec3()).collect();
    let Some(index) = KnnIndex::build(&pts).ok().filter(|_| !crown.is_empty()) else {
        return vec![0; p.len()];
    };
    let q: Vec<Vec3> = p.iter().map(|q| q.as_vec3()).collect();
    index
        .nearest_batch(&q)
        .into_iter()
        .map(|h| match h {
            Some(h) if f64::from(h.distance) <= reach => crown_labels[h.index as usize],
            _ => 0,
        })
        .collect()
}

fn grid_of(model: &[DVec3], d: &Design) -> Grid {
    let m = MARGIN + d.clearance + d.wall;
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in model {
        lo[0] = lo[0].min(p.x);
        lo[1] = lo[1].min(p.y);
        hi[0] = hi[0].max(p.x);
        hi[1] = hi[1].max(p.y);
    }
    // one lattice for every grid (limit solids line up)
    let lo = [
        ((lo[0] - m) / CELL).floor() * CELL,
        ((lo[1] - m) / CELL).floor() * CELL,
    ];
    Grid::new(lo, [hi[0] + m, hi[1] + m], CELL)
}

/// A point on the top of the crowns of the first group of FDIs present (mean
/// of their highest points, `_point_of`).
fn point_of(p: &[DVec3], lab: &[i64], groups: &[&[i64]]) -> Option<DVec3> {
    for g in groups {
        let q: Vec<DVec3> = p
            .iter()
            .zip(lab)
            .filter(|(_, l)| g.contains(l))
            .map(|(&q, _)| q)
            .collect();
        if q.len() >= 10 {
            let mut z: Vec<f64> = q.iter().map(|v| v.z).collect();
            let z0 = percentile(&mut z, 80.0);
            let top: Vec<DVec3> = q.into_iter().filter(|v| v.z >= z0).collect();
            return Some(top.iter().copied().sum::<DVec3>() / top.len() as f64);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn front_of(
    lower: (&[DVec3], &[i64]),
    upper: (&[DVec3], &[i64]),
    rel: &DMat4,
    covered_l: &[i64],
    covered_u: &[i64],
    r: f64,
    k: f64,
    bare: (usize, usize),
) -> Front {
    let pick = |p: &[DVec3], l: &[i64], up: bool, cov: &[i64], bare_n: usize| {
        let mut pts = Vec::new();
        let mut lab = Vec::new();
        let mut rad = Vec::new();
        for (a, (&q, &f)) in p.iter().zip(l).enumerate() {
            if is_front(f, up) {
                pts.push(q);
                lab.push(f);
                rad.push(if cov.contains(&f) && a < bare_n {
                    r + TOUCH_MM
                } else {
                    0.0
                });
            }
        }
        (pts, lab, rad)
    };
    let (pl, ll, rl) = pick(lower.0, lower.1, false, covered_l, bare.0);
    let (pu, lu, ru) = pick(upper.0, upper.1, true, covered_u, bare.1);
    Front::new(pl, ll, rl, &pu, lu, ru, rel, k)
}

/// The occlusal surface of an aligner with its ramps on it (the higher of
/// the two; a ramp is never ground).
fn with_ramps(surf: Surface, p: &[DVec3], l: &[i64]) -> Surface {
    if p.is_empty() {
        return surf;
    }
    let (zr, lr) = envelope(p, 0.0, &surf.grid, Some(l), None);
    let up = zr.zip(&surf.z, |r, z| r.is_finite() && (!z.is_finite() || r > z));
    if !up.data.iter().any(|&u| u) {
        return surf;
    }
    let z = up
        .zip(&zr, |u, r| (u, r))
        .zip(&surf.z, |(u, r), z| if u { r } else { z });
    let l = up
        .zip(&lr, |u, r| (u, r))
        .zip(&surf.l, |(u, r), l| if u { r } else { l });
    let cov = surf.cov.zip(&up, |c, u| c && !u);
    Surface { z, l, cov, ..surf }
}

/// The bite with the aligners worn.
#[derive(Debug, Clone)]
pub struct Bite {
    /// The occlusal surfaces (lower, upper) at the occlusal thickness.
    pub surfaces: (Surface, Surface),
    /// Occlusal thickness (mm).
    pub occlusal: f64,
    /// The closing, after the equilibration when it is on.
    pub closing: Option<Closing>,
    /// The equilibration.
    pub equilibration: Option<Equilibration>,
    /// How the mouth closes.
    pub model: BiteClosing,
    /// Point of the hinge axis (lower coordinates).
    pub axis: Option<DVec3>,
    /// Lower incisal point.
    pub incisal: DVec3,
    /// Lower first molar point.
    pub molar: DVec3,
    /// Upper to lower arch coordinates.
    pub rel: DMat4,
    /// The closing of the bare teeth.
    pub teeth: Option<Closing>,
    /// Opening caused by the aligners at the incisors and at the molars,
    /// beyond the teeth closing (mm).
    pub openings: (Option<f64>, Option<f64>),
    /// Upper nodes left out of the vertical contacts (overbite).
    pub excluded: usize,
    /// The bite as numbers and a sentence.
    pub report: Report,
}

/// Grinding of the equilibration, in the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EquilibrationReport {
    /// Both sides touch.
    pub done: bool,
    /// Why it stopped short (`"spessore"`, `"anteriori"`, `"passi"`).
    pub stop: String,
    /// Largest grinding of the lower and of the upper aligner (mm).
    pub max_grind_mm: [f64; 2],
    /// Teeth in contact before the grinding, right and left.
    pub before_right: Vec<i64>,
    /// Teeth in contact before the grinding, left.
    pub before_left: Vec<i64>,
}

/// The bite of a stage as numbers and a sentence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Stages of the lower and of the upper arch.
    pub stages: [u32; 2],
    /// Occlusal thickness (mm).
    pub occlusal_thickness_mm: f64,
    /// Smallest gap between the aligners on the right and left sides (mm).
    pub side_gap_mm: [Option<f64>; 2],
    /// How the thickness was chosen.
    pub thickness_mode: String,
    /// How the mouth closes.
    pub closing: String,
    /// Opening caused by the aligners at the molars (mm).
    pub molar_opening_mm: Option<f64>,
    /// Opening caused by the aligners at the incisors (mm).
    pub incisor_opening_mm: Option<f64>,
    /// Teeth in contact, right side.
    pub contacts_right: Vec<i64>,
    /// Teeth in contact, left side.
    pub contacts_left: Vec<i64>,
    /// Contacts on both sides.
    pub bilateral: bool,
    /// Anterior teeth touching between the aligners.
    pub anterior_contacts: Vec<i64>,
    /// The equilibration, when on.
    pub equilibration: Option<EquilibrationReport>,
    /// The sentence (Italian, as the plugin's report).
    pub text: String,
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round_ties_even() / 100.0
}

/// The bite with the aligners of stages `stages` (lower, upper) worn.
/// `rel` maps the upper to the lower arch coordinates. None without crowns.
pub fn bite(
    lower: &ArchStage,
    upper: &ArchStage,
    rel: &DMat4,
    d: &Design,
    stages: [u32; 2],
) -> Option<Bite> {
    if lower.crown_points.is_empty() || upper.crown_points.is_empty() {
        return None;
    }
    let (c, wall) = (d.clearance, d.wall);
    let o = &d.occlusal;
    let tmin = o.min_thickness_mm.min(wall);
    let (pl, ll) = (&lower.crown_points[..], &lower.crown_labels[..]);
    let (pu, lu) = (&upper.crown_points[..], &upper.crown_labels[..]);
    let gl = grid_of(&lower.model_points, d);
    let gu = grid_of(&upper.model_points, d);
    let (sl, lln) = heights(&lower.model_points, &lower.model_labels, &gl, d.blockout);
    let (su, lun) = heights(&upper.model_points, &upper.model_labels, &gu, d.blockout);
    let bl: Base = base_from_heights(&sl, &lln, &lower.covered, c, wall, tmin, &gl);
    let bu: Base = base_from_heights(&su, &lun, &upper.covered, c, wall, tmin, &gu);
    let model = o.closing;
    let incisal = point_of(pl, ll, &[&[31, 41], &[32, 42], &[33, 43]]).unwrap_or_else(|| {
        pl.iter()
            .copied()
            .fold(pl[0], |m, p| if p.y > m.y { p } else { m })
    });
    let axis = (model == BiteClosing::Hinge)
        .then(|| hinge_axis(incisal, o.hinge_posterior_mm, o.hinge_height_mm));
    let molar = point_of(pl, ll, &[&[36, 46], &[37, 47], &[35, 45]]).unwrap_or_else(|| {
        pl.iter()
            .copied()
            .fold(pl[0], |m, p| if p.y < m.y { p } else { m })
    });
    // the incisors and canines in 3D (Front), the other teeth on the height maps (their upper nodes only)
    let bare_l = bare_surface(&bl);
    let bare_u = bare_surface(&bu);
    let mut mask = overlap_mask(&bare_l, &bare_u, rel, OVERLAP_MM, c + wall + 0.5);
    for (m, (a, b)) in mask
        .data
        .iter_mut()
        .zip(bu.lall.data.iter().zip(&bu.lw.data))
    {
        *m &= !is_front(*a, true) && !is_front(*b, true);
    }
    let front0 = front_of(
        (pl, ll),
        (pu, lu),
        rel,
        &[],
        &[],
        0.0,
        1.0,
        (pl.len(), pu.len()),
    );
    let teeth = closing(
        &bare_l,
        &bare_u,
        rel,
        model,
        axis,
        0.1,
        Some(&mask),
        Some(&front0),
    );
    let opened = |cl: &Option<Closing>| -> (Option<f64>, Option<f64>) {
        match (cl, &teeth) {
            (Some(cl), Some(t)) => (
                Some(
                    opening_at(incisal, model, cl.value, axis)
                        - opening_at(incisal, model, t.value, axis),
                ),
                Some(
                    opening_at(molar, model, cl.value, axis)
                        - opening_at(molar, model, t.value, axis),
                ),
            ),
            _ => (None, None),
        }
    };
    let contact = evidence::param_f64("contact_map_contact").unwrap_or(0.1);
    let target = o.opening_target_mm;
    let (cov_l, cov_u) = (&lower.covered[..], &upper.covered[..]);
    // the surfaces, the equilibration and the closing at the occlusal thickness t
    let at = |t: f64| -> (Surface, Surface, Option<Equilibration>, Option<Closing>) {
        let s_l = with_ramps(
            with_occlusal(&bl, t),
            &lower.ramp_points,
            &lower.ramp_labels,
        );
        let s_u = with_ramps(
            with_occlusal(&bu, t),
            &upper.ramp_points,
            &upper.ramp_labels,
        );
        let cat = |a: &[DVec3], b: &[DVec3]| [a, b].concat();
        let (rp_l, rp_u) = (cat(pl, &lower.ramp_points), cat(pu, &upper.ramp_points));
        let (rl_l, rl_u) = (
            [ll, &lower.ramp_labels[..]].concat(),
            [lu, &upper.ramp_labels[..]].concat(),
        );
        let fr = front_of(
            (&rp_l, &rl_l),
            (&rp_u, &rl_u),
            rel,
            cov_l,
            cov_u,
            c + wall,
            (c + wall) / (c + t.min(wall)).max(1e-3),
            (pl.len(), pu.len()),
        );
        if o.equilibrate {
            let e = equilibrate(
                &s_l,
                &s_u,
                rel,
                model,
                axis,
                contact,
                o.equilibrate_teeth_per_side as usize,
                o.equilibrate_max_mm,
                Some(&mask),
                Some(&fr),
            );
            let cl = e.closing.clone();
            (s_l, s_u, Some(e), cl)
        } else {
            let cl = closing(
                &s_l,
                &s_u,
                rel,
                model,
                axis,
                contact,
                Some(&mask),
                Some(&fr),
            );
            (s_l, s_u, None, cl)
        }
    };
    let occ = match o.mode {
        OcclusalMode::Same => wall,
        OcclusalMode::Manual => o.thickness_mm.min(wall),
        OcclusalMode::Auto if !o.equilibrate => {
            let o_wall = opened(&at(wall).3).1;
            let o_min = opened(&at(tmin).3).1;
            match (o_wall, o_min) {
                (Some(o_wall), Some(o_min)) => {
                    let mut occ = solve_thickness(o_wall, o_min, wall, tmin, target);
                    if tmin < occ && occ < wall {
                        // the opening is about linear in the thickness: one secant step on the measure at the solution
                        if let Some(o_mid) = opened(&at(occ).3).1
                            && (o_mid - target).abs() > 0.05
                        {
                            let ((lo_t, lo_o), (hi_t, hi_o)) = if o_mid < target {
                                ((occ, o_mid), (wall, o_wall))
                            } else {
                                ((tmin, o_min), (occ, o_mid))
                            };
                            occ = solve_thickness(hi_o, lo_o, hi_t, lo_t, target);
                        }
                    }
                    occ
                }
                _ => wall,
            }
        }
        OcclusalMode::Auto => {
            // with the equilibration, among the thicknesses from the wall to the minimum (AUTO_STEP apart): first
            // the contacts on both sides, then the opening within the target, then the thickest (the strongest);
            // the grinding needs room above the minimum, so the thickness is not brought down to it first
            let mut ts: Vec<f64> = Vec::new();
            let mut t = wall;
            while t >= tmin - 1e-9 {
                ts.push(t);
                t -= AUTO_STEP;
            }
            ts.push(tmin);
            let mut ts: Vec<f64> = ts
                .iter()
                .map(|t| (t * 1e4).round_ties_even() / 1e4)
                .collect();
            ts.sort_by(f64::total_cmp);
            ts.dedup();
            ts.reverse();
            let cand: Vec<(f64, bool, f64)> = ts
                .iter()
                .map(|&t| {
                    let r = at(t);
                    let op = opened(&r.3).1;
                    (
                        t,
                        r.2.as_ref().is_some_and(|e| e.done),
                        op.unwrap_or(f64::INFINITY),
                    )
                })
                .collect();
            let ok: Vec<&(f64, bool, f64)> = cand.iter().filter(|x| x.1 && x.2 <= target).collect();
            if let Some(b) = ok.iter().max_by(|a, b| a.0.total_cmp(&b.0)) {
                b.0
            } else {
                let pool: Vec<&(f64, bool, f64)> = {
                    let done: Vec<_> = cand.iter().filter(|x| x.1).collect();
                    if done.is_empty() {
                        cand.iter().collect()
                    } else {
                        done
                    }
                };
                pool.iter()
                    .min_by(|a, b| {
                        ((a.2 * 1000.0).round_ties_even())
                            .total_cmp(&(b.2 * 1000.0).round_ties_even())
                            .then(b.0.total_cmp(&a.0))
                    })
                    .map_or(wall, |b| b.0)
            }
        }
    };
    let (s_l, s_u, eq, cl) = at(occ);
    let (o_inc, o_mol) = opened(&cl);
    let excluded = mask
        .data
        .iter()
        .zip(&bu.zall.data)
        .filter(|(m, z)| !**m && z.is_finite())
        .count();
    let mut out = Bite {
        surfaces: (s_l, s_u),
        occlusal: occ,
        closing: cl,
        equilibration: eq,
        model,
        axis,
        incisal,
        molar,
        rel: *rel,
        teeth,
        openings: (o_inc, o_mol),
        excluded,
        report: Report {
            stages,
            occlusal_thickness_mm: 0.0,
            side_gap_mm: [None, None],
            thickness_mode: String::new(),
            closing: String::new(),
            molar_opening_mm: None,
            incisor_opening_mm: None,
            contacts_right: vec![],
            contacts_left: vec![],
            bilateral: false,
            anterior_contacts: vec![],
            equilibration: None,
            text: String::new(),
        },
    };
    out.report = report(&out, o, stages);
    Some(out)
}

fn mode_name(m: OcclusalMode) -> &'static str {
    match m {
        OcclusalMode::Auto => "AUTO",
        OcclusalMode::Manual => "MANUAL",
        OcclusalMode::Same => "SAME",
    }
}

/// The bite of a stage as numbers and a sentence (`_report`).
fn report(b: &Bite, o: &OcclusalSettings, stages: [u32; 2]) -> Report {
    let cl = b.closing.as_ref();
    let empty = super::Sides::default();
    let sides = cl.map_or(&empty, |c| &c.sides);
    let gap = cl.map_or([None, None], |c| c.side_gap);
    let front: Vec<i64> = {
        let mut f: Vec<i64> = sides
            .right
            .iter()
            .chain(&sides.left)
            .copied()
            .filter(|&f| is_front(f, (1..=2).contains(&(f / 10))))
            .collect();
        f.sort_unstable();
        f
    };
    let bilateral = !sides.right.is_empty() && !sides.left.is_empty();
    let occ = r2(b.occlusal);
    let eq = b.equilibration.as_ref().map(|e| {
        let max = |g: &Raster<f64>| g.data.iter().cloned().fold(0.0, f64::max);
        EquilibrationReport {
            done: e.done,
            stop: e.stop.to_owned(),
            max_grind_mm: [r2(max(&e.grind_l)), r2(max(&e.grind_u))],
            before_right: e.before.as_ref().map_or(vec![], |c| c.sides.right.clone()),
            before_left: e.before.as_ref().map_or(vec![], |c| c.sides.left.clone()),
        }
    });
    let mut parts = vec![format!("Spessore occlusale {occ:.2} mm")];
    if let (Some(m), Some(i)) = (b.openings.1, b.openings.0) {
        parts.push(format!(
            "morso aperto dagli allineatori di {m:.1} mm ai molari e {i:.1} mm agli incisivi"
        ));
    }
    if !bilateral {
        let (lato, k) = if sides.right.is_empty() {
            ("destro", 0)
        } else {
            ("sinistro", 1)
        };
        let extra = gap[k].map_or(String::new(), |d| format!(" (a {d:.2} mm)"));
        parts.push(format!("nessun contatto sul lato {lato}{extra}"));
    }
    if !front.is_empty() {
        let list: Vec<String> = front.iter().map(i64::to_string).collect();
        parts.push(format!(
            "contatto tra gli allineatori anteriori ({})",
            list.join(", ")
        ));
    }
    if let Some(e) = &eq {
        let why = match e.stop.as_str() {
            "spessore" => "spessore minimo o molatura massima raggiunti",
            "anteriori" => "toccano gli anteriori",
            "passi" => "chiusura massima raggiunta",
            _ => "",
        };
        parts.push(if e.done {
            "equilibratura riuscita".to_owned()
        } else {
            format!("equilibratura non completa ({why})")
        });
    }
    Report {
        stages,
        occlusal_thickness_mm: occ,
        side_gap_mm: [gap[0].map(r2), gap[1].map(r2)],
        thickness_mode: mode_name(o.mode).to_owned(),
        closing: match b.model {
            BiteClosing::Hinge => "HINGE".to_owned(),
            BiteClosing::Vertical => "VERTICAL".to_owned(),
        },
        molar_opening_mm: b.openings.1.map(r2),
        incisor_opening_mm: b.openings.0.map(r2),
        contacts_right: sides.right.clone(),
        contacts_left: sides.left.clone(),
        bilateral,
        anterior_contacts: front,
        equilibration: eq,
        text: parts.join("; "),
    }
}

/// Points where the two aligners touch (lower arch coordinates) with their
/// gap (mm); empty when the mouth does not close (`contact_points_world`
/// before the move to the world).
pub fn contact_points(b: &Bite) -> (Vec<DVec3>, Vec<f64>) {
    let Some(cl) = &b.closing else {
        return (vec![], vec![]);
    };
    let contact = evidence::param_f64("contact_map_contact").unwrap_or(0.1);
    let mut p = Vec::new();
    let mut g = Vec::new();
    for (q, &gv) in cl.q.iter().zip(&cl.g) {
        if gv.is_finite() && gv <= contact {
            p.push(*q);
            g.push(gv);
        }
    }
    (p, g)
}

/// A grinding map without steps: grown by one cell (its contacts kept) and
/// averaged over 3 x 3 cells twice (`_smooth_grind`).
fn smooth_grind(g: &Raster<f64>) -> Raster<f64> {
    if !g.data.iter().any(|&v| v > 0.0) {
        return g.clone();
    }
    let mut out = g.clone();
    for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let s = g.shifted(di, dj, 0.0);
        for (a, b) in out.data.iter_mut().zip(&s.data) {
            *a = a.max(*b);
        }
    }
    for _ in 0..2 {
        let mut acc = Raster::filled([g.nx, g.ny], 0.0);
        for di in -1..=1 {
            for dj in -1..=1 {
                let s = out.shifted(di, dj, 0.0);
                for (a, b) in acc.data.iter_mut().zip(&s.data) {
                    *a += *b;
                }
            }
        }
        out = acc.map(|v| v / 9.0);
    }
    out
}

/// Area-weighted vertex normals (`meshutil.vertex_normals`).
pub fn vertex_normals(v: &[DVec3], t: &[[u32; 3]]) -> Vec<DVec3> {
    let mut n = vec![DVec3::ZERO; v.len()];
    for f in t {
        let [a, b, c] = f.map(|i| v[i as usize]);
        let cr = (b - a).cross(c - a);
        for &i in f {
            n[i as usize] += cr;
        }
    }
    n.into_iter().map(|x| x.normalize_or_zero()).collect()
}

/// What [`shape_shell`] did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShapeReport {
    /// The report of the bite of the stage.
    pub bite: Report,
    /// Outer vertices lowered onto the occlusal limit.
    pub vertices_lowered: usize,
    /// 5th percentile and median of the occlusal thickness measured on the
    /// lowered vertices (mm).
    pub measured_thickness_mm: Option<[f64; 2]>,
}

/// The shell of the wall thickness (`shell`, arch coordinates) with the
/// occlusal thickness and the grinding of its stage: its outer vertices above
/// the allowed top are lowered onto it (the faces stay: no hole, the inner
/// surface never moves). The allowed top comes from the shell itself: the
/// envelope of its inner surface (the vertices whose normal faces the model,
/// or on it) by ellipsoids of the wall across and the occlusal thickness
/// along `Z`, minus the grinding (smoothed), never under the minimum
/// thickness; only over the teeth under the aligner (`covered`). `model` is
/// the model the shell is the offset of. Returns the shaped shell and the
/// report of the stage with the thickness measured.
pub fn shape_shell(
    shell: &TriMesh,
    model: &TriMesh,
    b: &Bite,
    is_lower: bool,
    covered: &[i64],
    d: &Design,
) -> (TriMesh, ShapeReport) {
    let surf = if is_lower {
        &b.surfaces.0
    } else {
        &b.surfaces.1
    };
    let g = &surf.grid;
    let grind = b
        .equilibration
        .as_ref()
        .map(|e| smooth_grind(if is_lower { &e.grind_l } else { &e.grind_u }));
    let (wall, occ) = (d.wall, b.occlusal);
    let tmin = d.occlusal.min_thickness_mm.min(occ);
    let mut report = ShapeReport {
        bite: b.report.clone(),
        vertices_lowered: 0,
        measured_thickness_mm: None,
    };
    if occ >= wall - 1e-6
        && grind
            .as_ref()
            .is_none_or(|g| !g.data.iter().any(|&v| v > 1e-4))
    {
        return (shell.clone(), report);
    }
    let vs: Vec<DVec3> = shell.positions.iter().map(|p| p.as_dvec3()).collect();
    let Ok(bvh) = Bvh::build(model) else {
        return (shell.clone(), report);
    };
    let nearest: Vec<Option<dc_geom::bvh::ClosestHit>> =
        bvh.closest_points(&shell.positions, f32::INFINITY);
    let n = vertex_normals(&vs, &shell.tris);
    let toward: Vec<f64> = vs
        .iter()
        .zip(&nearest)
        .zip(&n)
        .map(|((v, h), n)| h.map_or(0.0, |h| (h.point.as_dvec3() - *v).dot(*n)))
        .collect();
    let inner: Vec<bool> = nearest
        .iter()
        .zip(&toward)
        .map(|(h, &t)| h.is_some_and(|h| f64::from(h.distance) <= 0.05) || t > 0.0)
        .collect();
    let pin: Vec<DVec3> = vs
        .iter()
        .zip(&inner)
        .filter(|(_, i)| **i)
        .map(|(&v, _)| v)
        .collect();
    let zo = if occ < wall - 1e-6 {
        envelope(&pin, wall, g, None, Some(occ)).0
    } else {
        envelope(&pin, wall, g, None, None).0
    };
    let zm = envelope(&pin, wall, g, None, Some(tmin)).0;
    let mut h = zo.clone();
    for k in 0..h.data.len() {
        let mut v = zo.data[k] - grind.as_ref().map_or(0.0, |g| g.data[k]);
        if zm.data[k].is_finite() {
            v = v.max(zm.data[k]);
        }
        let on = surf.cov.data[k] && covered.contains(&surf.l.data[k]);
        h.data[k] = if on && v.is_finite() {
            v
        } else {
            f64::INFINITY
        };
    }
    let h_big = h.map(|v| if v.is_finite() { v } else { 1e9 });
    let out_idx: Vec<usize> = (0..vs.len())
        .filter(|&i| !inner[i] && n[i].z > UP_NZ)
        .collect();
    let out_pts: Vec<DVec3> = out_idx.iter().map(|&i| vs[i]).collect();
    let hs = g.sample(&h_big, &out_pts, f64::INFINITY, None);
    let mut moved: Vec<usize> = out_idx
        .iter()
        .zip(&hs)
        .filter(|(i, hh)| vs[**i].z > **hh + 1e-6 && **hh < 1e8)
        .map(|(&i, _)| i)
        .collect();
    moved.sort_unstable();
    let mut vs2 = vs.clone();
    if !moved.is_empty() {
        let mp: Vec<DVec3> = moved.iter().map(|&i| vs[i]).collect();
        let nz = g.sample(&h_big, &mp, super::NO, None);
        for (&i, &z) in moved.iter().zip(&nz) {
            vs2[i].z = z;
        }
        // the minimum thickness measured from the model (along any direction, not only along Z)
        let lim = d.clearance + tmin;
        let q: Vec<Vec3> = moved.iter().map(|&i| vs2[i].as_vec3()).collect();
        let d2 = bvh.closest_points(&q, f32::INFINITY);
        for (&i, hit) in moved.iter().zip(&d2) {
            if let Some(hit) = hit {
                let dist = f64::from(hit.distance);
                if dist < lim {
                    vs2[i].z += (lim - dist) / n[i].z.max(0.5);
                }
            }
        }
        let top = envelope(&pin, 0.0, g, None, None).0;
        let tops = g.sample(&top, &mp, super::NO, None);
        let mut th: Vec<f64> = moved
            .iter()
            .zip(&tops)
            .map(|(&i, &t)| vs2[i].z - t)
            .filter(|v| v.is_finite())
            .collect();
        if !th.is_empty() {
            let p5 = r2(percentile(&mut th, 5.0));
            let med = r2(percentile(&mut th, 50.0));
            report.measured_thickness_mm = Some([p5, med]);
        }
    }
    report.vertices_lowered = moved.len();
    let mut out = TriMesh::new(
        vs2.iter().map(|v| v.as_vec3()).collect(),
        shell.tris.clone(),
    );
    out.vertex_labels.clone_from(&shell.vertex_labels);
    (out, report)
}

/// The closed solid under the occlusal surface of an aligner (the envelope
/// of its outer surface): the nodes with a surface and the cells between
/// them, from the surface down to `depth` mm under the lowest of them (a
/// flat bottom), side walls on the boundary. Closed and outward oriented.
/// The limit solid of the shell is its intersection with the shell.
pub fn envelope_solid(surf: &Surface, depth: f64) -> Option<TriMesh> {
    height_solid(&surf.z, &surf.grid, depth)
}

/// The closed solid under a height map (see [`envelope_solid`]); `NaN` and
/// infinite nodes have no surface.
pub fn height_solid(z: &Raster<f64>, grid: &Grid, depth: f64) -> Option<TriMesh> {
    let (nx, ny) = (z.nx, z.ny);
    let ok = |i: usize, j: usize| z.at(i, j).is_finite();
    let cell = |i: usize, j: usize| {
        i + 1 < nx && j + 1 < ny && ok(i, j) && ok(i + 1, j) && ok(i, j + 1) && ok(i + 1, j + 1)
    };
    let zmin = z
        .data
        .iter()
        .cloned()
        .filter(|v| v.is_finite())
        .fold(f64::INFINITY, f64::min);
    if !zmin.is_finite() {
        return None;
    }
    let bottom = zmin - depth;
    // vertices: top nodes of the cells, bottom nodes under the boundary
    let mut top_id = vec![u32::MAX; nx * ny];
    let mut bot_id = vec![u32::MAX; nx * ny];
    let mut pos: Vec<Vec3> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut any = false;
    let mut top = |i: usize, j: usize, pos: &mut Vec<Vec3>| -> u32 {
        let id = &mut top_id[i * ny + j];
        if *id == u32::MAX {
            let [x, y] = grid.node(i, j);
            *id = pos.len() as u32;
            pos.push(Vec3::new(x as f32, y as f32, z.at(i, j) as f32));
        }
        *id
    };
    let mut bot = |i: usize, j: usize, pos: &mut Vec<Vec3>| -> u32 {
        let id = &mut bot_id[i * ny + j];
        if *id == u32::MAX {
            let [x, y] = grid.node(i, j);
            *id = pos.len() as u32;
            pos.push(Vec3::new(x as f32, y as f32, bottom as f32));
        }
        *id
    };
    for i in 0..nx.saturating_sub(1) {
        for j in 0..ny.saturating_sub(1) {
            if !cell(i, j) {
                continue;
            }
            any = true;
            // top (normal +Z): counter-clockwise seen from above
            let (a, b, c, d) = (
                top(i, j, &mut pos),
                top(i + 1, j, &mut pos),
                top(i + 1, j + 1, &mut pos),
                top(i, j + 1, &mut pos),
            );
            if (z.at(i, j) - z.at(i + 1, j + 1)).abs() <= (z.at(i + 1, j) - z.at(i, j + 1)).abs() {
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            } else {
                tris.push([a, b, d]);
                tris.push([b, c, d]);
            }
            // bottom (normal -Z), flat
            let (e, f, g2, h) = (
                bot(i, j, &mut pos),
                bot(i + 1, j, &mut pos),
                bot(i + 1, j + 1, &mut pos),
                bot(i, j + 1, &mut pos),
            );
            tris.push([e, g2, f]);
            tris.push([e, h, g2]);
            // side walls where the neighbouring cell is missing
            let has = |di: i64, dj: i64| {
                let (ci, cj) = (i as i64 + di, j as i64 + dj);
                ci >= 0 && cj >= 0 && cell(ci as usize, cj as usize)
            };
            if !has(0, -1) {
                // edge (i,j)-(i+1,j), outward -Y
                tris.push([a, e, f]);
                tris.push([a, f, b]);
            }
            if !has(1, 0) {
                // edge (i+1,j)-(i+1,j+1), outward +X
                tris.push([b, f, g2]);
                tris.push([b, g2, c]);
            }
            if !has(0, 1) {
                // edge (i+1,j+1)-(i,j+1), outward +Y
                tris.push([c, g2, h]);
                tris.push([c, h, d]);
            }
            if !has(-1, 0) {
                // edge (i,j+1)-(i,j), outward -X
                tris.push([d, h, e]);
                tris.push([d, e, a]);
            }
        }
    }
    any.then(|| TriMesh::new(pos, tris))
}
