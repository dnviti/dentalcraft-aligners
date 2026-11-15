//! Control visits: compares a progress scan with the plan, tooth by tooth.
//!
//! Port of the add-on's `ortho_aligner/visit.py` (analysis of a control
//! visit). At a visit the patient has worn the aligners up to stage `k`:
//!
//! 1. **Registration of the scan** ([`register_scan`]): the progress scan is
//!    placed on the model expected after `k` aligners (the scan of the case
//!    with its teeth, and the gingiva, moved to stage `k`). Arch-frame
//!    pre-alignment (four half-turn candidates), trimmed point-to-point ICP
//!    on subsamples (Besl & McKay 1992; Chetverikov 2002; Kabsch 1976), then
//!    point-to-plane ICP with normal compatibility on the full density
//!    (Chen & Medioni 1992; Rusinkiewicz & Levoy 2001). The sign of the
//!    scan's normals is detected (some STL files are wound inwards).
//! 2. **Achieved position of every tooth** ([`track`]): every crown of the
//!    plan, at its expected position, is registered to the scan points around
//!    it (point-to-plane ICP with normal compatibility so the facing surfaces
//!    of the neighbours are not used); a second pass repeats it on the scan
//!    points that are closer to that crown than to its neighbours.
//! 3. **Tracking**: the deviation achieved - planned is split in the clinical
//!    components of [`ToothDeviation`]; a tooth is on track when every
//!    component is within the thresholds (0.5 mm and 2 degrees,
//!    Grunheid 2017, Al-Nadawi 2021); the efficacy is the achieved over the
//!    planned movement (Kravitz 2009; Haouili 2020).
//! 4. **Decision** ([`Decision`]): all on track, or a refinement is suggested
//!    with a lower velocity (`lag_speed`, Simon 2014) for the lagging teeth.
//!
//! All the work is in one coordinate system (the case's): the arrays of the
//! expected model, the scan and the crowns are in it, and so are the results.
//! The pairs of the ICPs are the nearest *vertices* of the target, as in
//! `visit.icp` (the targets are point sets: the scan vertices around a crown
//! have no connectivity), with the vertex normals of the scan.
//!
//! The re-plan of the add-on (the visit scan segmented with seeds on the
//! achieved crowns becomes a new case, new staging) is not part of this
//! module: the command stores the labels and the speed factors, and the
//! staging is run again from them.
//!
//! Reference: `ortho_aligner/visit.py` and `ortho_aligner/ops_visit.py`.

use std::collections::BTreeMap;

use dc_case::{Fdi, Pose};
use dc_geom::icp::{kabsch, point_to_plane_step};
use dc_geom::knn::KnnIndex;
use dca_model::{Decision, Stage, ToothDeviation, ToothTracking};
use glam::{DMat3, DMat4, DVec3, Vec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::{autoseed, evidence, frame, rotation};

/// Fewest pairs an ICP iteration needs (the add-on's `12`).
const MIN_PAIRS: usize = 12;
/// Fewest scan points around a crown to register it (the add-on's `50`).
const MIN_NEAR: usize = 50;
/// Margin (mm) of the box around an expected crown that selects scan points.
const NEAR_MARGIN_MM: f64 = 2.0;
/// Crown points used in the first registration of a tooth.
const CROWN_SAMPLES: usize = 1500;
/// Crown points of the neighbour test of the second pass.
const OWNER_SAMPLES: usize = 2500;
/// A scan point is a tooth's own only within this distance (mm) of its crown.
const OWN_DISTANCE_MM: f64 = 1.5;
/// Planned movement (mm) below which the efficacy is not defined.
const MIN_PLANNED_MM: f64 = 0.2;

/// Errors of the analysis of a visit.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VisitError {
    /// A mesh is unusable (empty, or a triangle index out of range).
    #[error("{0}")]
    Invalid(String),
    /// The arch frame of a model could not be estimated.
    #[error("the arch frame of the {0} cannot be estimated: {1}")]
    Frame(&'static str, String),
    /// The registration of the scan failed.
    #[error("the scan cannot be registered to the plan: {0}")]
    Registration(String),
    /// No tooth of the plan was recognised on the scan.
    #[error("no tooth was recognised on the control scan: check the arch")]
    NoTeeth,
}

/// Parameters of the analysis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Linear deviation (mm) beyond which a tooth is not on track (evidence
    /// `tracking_linear`).
    pub linear_mm: f64,
    /// Angular deviation (degrees) beyond which a tooth is not on track
    /// (evidence `tracking_angular`).
    pub angular_deg: f64,
    /// Velocity factor of the teeth that are behind, in the new aligners
    /// (evidence `lag_speed`).
    pub lag_speed: f64,
    /// Repeat the registration of every tooth on the scan points that are
    /// closer to it than to its neighbours.
    pub two_pass: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            linear_mm: evidence::param_f64("tracking_linear").unwrap_or(0.5),
            angular_deg: evidence::param_f64("tracking_angular").unwrap_or(2.0),
            lag_speed: evidence::param_f64("lag_speed").unwrap_or(0.75),
            two_pass: true,
        }
    }
}

// ---- numeric helpers ------------------------------------------------------------------------------------

/// Indices of `n` elements evenly spread over `n_all` (`visit._sub_idx`:
/// `np.linspace(0, n_all - 1, n).astype(int)`), all of them when `n_all <= n`.
pub fn sub_idx(n_all: usize, n: usize) -> Vec<usize> {
    if n_all <= n {
        return (0..n_all).collect();
    }
    if n < 2 {
        return vec![0; n];
    }
    let step = (n_all - 1) as f64 / (n - 1) as f64;
    let mut out: Vec<usize> = (0..n).map(|i| (i as f64 * step) as usize).collect();
    out[n - 1] = n_all - 1;
    out
}

fn pick(p: &[DVec3], idx: &[usize]) -> Vec<DVec3> {
    idx.iter().map(|&i| p[i]).collect()
}

/// Area-weighted vertex normals, normalised (zero for isolated vertices),
/// as `meshutil.vertex_normals`.
pub fn vertex_normals(v: &[DVec3], tris: &[[u32; 3]]) -> Vec<DVec3> {
    let mut acc = vec![DVec3::ZERO; v.len()];
    for t in tris {
        let [a, b, c] = t.map(|i| v[i as usize]);
        let n = (b - a).cross(c - a);
        for &i in t {
            acc[i as usize] += n;
        }
    }
    acc.into_iter().map(|n| n.normalize_or_zero()).collect()
}

/// `numpy.quantile` (linear interpolation) of unsorted values.
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut s = values.to_vec();
    s.sort_unstable_by(f64::total_cmp);
    let h = (s.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

/// `numpy.median`.
fn median(values: &[f64]) -> f64 {
    let mut s = values.to_vec();
    s.sort_unstable_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    }
}

fn rms(values: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut n) = (0.0, 0usize);
    for v in values {
        s += v * v;
        n += 1;
    }
    (s / n as f64).sqrt()
}

fn xform(m: &DMat4, p: &[DVec3]) -> Vec<DVec3> {
    p.iter().map(|&q| m.transform_point3(q)).collect()
}

fn rot(m: &DMat4) -> DMat3 {
    DMat3::from_mat4(*m)
}

// ---- ICP on point sets ------------------------------------------------------------------------------------

/// Target of an ICP: a point set (with optional normals) and its nearest
/// neighbour index.
struct Target {
    pts: Vec<DVec3>,
    normals: Option<Vec<DVec3>>,
    index: Option<KnnIndex>,
}

impl Target {
    fn new(pts: Vec<DVec3>, normals: Option<Vec<DVec3>>) -> Self {
        let q: Vec<Vec3> = pts.iter().map(|p| p.as_vec3()).collect();
        let index = KnnIndex::build(&q).ok();
        Self {
            pts,
            normals,
            index,
        }
    }

    /// Nearest target point of every query: (index, distance in f64).
    fn nearest(&self, queries: &[DVec3]) -> Vec<(usize, f64)> {
        let Some(index) = &self.index else {
            return vec![(0, f64::INFINITY); queries.len()];
        };
        let q: Vec<Vec3> = queries.par_iter().map(|p| p.as_vec3()).collect();
        index
            .nearest_batch(&q)
            .into_par_iter()
            .zip(queries.par_iter())
            .map(|(nb, p)| match nb {
                Some(n) => (n.index as usize, p.distance(self.pts[n.index as usize])),
                None => (0, f64::INFINITY),
            })
            .collect()
    }
}

/// Settings of [`icp`] that vary between the calls of the add-on.
#[derive(Clone, Copy)]
struct IcpOpts {
    iters: usize,
    trim: f64,
}

/// Tolerance on the change of the RMS between two iterations (`visit.icp`).
const ICP_TOL: f64 = 1e-7;
/// Largest angle between the normals of a pair (degrees, `visit.icp`).
const ICP_MAX_ANGLE: f64 = 60.0;

/// Trimmed ICP of the points `src` onto the target (`visit.icp`): the worst
/// `1 - trim` pairs are discarded at every iteration; with normals on both
/// sides the pairs whose normals differ by more than 60 degrees are
/// discarded and the point-to-plane distance is minimised, else the distance
/// between the points is, with the Kabsch rotation. Returns the transform
/// `src -> target` and the RMS of the kept pairs before the last update.
fn icp(
    src: &[DVec3],
    src_n: Option<&[DVec3]>,
    dst: &Target,
    init: DMat4,
    opts: IcpOpts,
) -> (DMat4, f64) {
    let dst_n = dst.normals.as_deref();
    let use_n = src_n.is_some() && dst_n.is_some();
    let cos_max = ICP_MAX_ANGLE.to_radians().cos();
    let mut t = init;
    let (mut last, mut err) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..opts.iters {
        let moved = xform(&t, src);
        let hits = dst.nearest(&moved);
        let r = rot(&t);
        let mut keep: Vec<usize> = (0..moved.len())
            .filter(|&i| match (src_n, dst_n) {
                (Some(sn), Some(dn)) => (r * sn[i]).dot(dn[hits[i].0]) >= cos_max,
                _ => true,
            })
            .collect();
        if keep.len() < MIN_PAIRS {
            break;
        }
        let dists: Vec<f64> = keep.iter().map(|&i| hits[i].1).collect();
        let q = quantile(&dists, opts.trim);
        keep.retain(|&i| hits[i].1 <= q);
        if keep.is_empty() {
            break;
        }
        let pk: Vec<DVec3> = keep.iter().map(|&i| moved[i]).collect();
        let qk: Vec<DVec3> = keep.iter().map(|&i| dst.pts[hits[i].0]).collect();
        let step = if let (true, Some(dn)) = (use_n, dst_n) {
            let nk: Vec<DVec3> = keep.iter().map(|&i| dn[hits[i].0]).collect();
            err = rms(pk
                .iter()
                .zip(&qk)
                .zip(&nk)
                .map(|((p, q), n)| (*q - *p).dot(*n)));
            point_to_plane_step(&pk, &qk, &nk)
        } else {
            err = rms(keep.iter().map(|&i| hits[i].1));
            kabsch(&pk, &qk, None, false)
        };
        let Ok(step) = step else {
            break;
        };
        t = DMat4::from(step) * t;
        if (last - err).abs() < ICP_TOL {
            break;
        }
        last = err;
    }
    (t, err)
}

// ---- registration of the scan -------------------------------------------------------------------------------

/// A triangle mesh as arrays, in the coordinates of the case.
#[derive(Debug, Clone, Copy)]
pub struct MeshRef<'a> {
    /// Vertices (mm).
    pub vertices: &'a [DVec3],
    /// Triangles.
    pub tris: &'a [[u32; 3]],
}

impl MeshRef<'_> {
    fn check(&self, what: &str) -> Result<(), VisitError> {
        if self.vertices.len() < 3 || self.tris.is_empty() {
            return Err(VisitError::Invalid(format!("the {what} is empty")));
        }
        let n = self.vertices.len() as u32;
        if self.tris.iter().any(|t| t.iter().any(|&i| i >= n)) {
            return Err(VisitError::Invalid(format!(
                "the {what} has a triangle with a vertex out of range"
            )));
        }
        if self.vertices.iter().any(|p| !p.is_finite()) {
            return Err(VisitError::Invalid(format!(
                "the {what} has a vertex that is not finite"
            )));
        }
        Ok(())
    }
}

/// Result of [`register_scan`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Registration {
    /// Rigid transform of the scan onto the expected model.
    pub transform: DMat4,
    /// RMS of the point-to-plane distance of the kept pairs (mm).
    pub rms_mm: f64,
    /// `+1` when the normals of the scan point outwards, `-1` when the scan
    /// is wound inwards.
    pub normal_sign: f64,
}

/// Places the progress `scan` on the `expected` model (the scan of the case
/// at the stage the patient is at): arch frames, then the four half-turn
/// candidates through a coarse point-to-point ICP, then a point-to-plane ICP
/// on the full density (`visit._register_compute`).
///
/// # Errors
/// [`VisitError::Invalid`] for an unusable mesh, [`VisitError::Frame`] when
/// an arch frame cannot be estimated.
pub fn register_scan(expected: MeshRef<'_>, scan: MeshRef<'_>) -> Result<Registration, VisitError> {
    expected.check("expected model")?;
    scan.check("control scan")?;
    let n_exp = vertex_normals(expected.vertices, expected.tris);
    let n_scan = vertex_normals(scan.vertices, scan.tris);
    let m_e = autoseed::arch_frame(expected.vertices, expected.tris, autoseed::BAND_DEPTH)
        .map_err(|e| VisitError::Frame("plan", e.to_string()))?;
    let m_n = autoseed::arch_frame(scan.vertices, scan.tris, autoseed::BAND_DEPTH)
        .map_err(|e| VisitError::Frame("control scan", e.to_string()))?;
    let coarse = sub_idx(expected.vertices.len(), 40_000);
    let coarse_target = Target::new(pick(expected.vertices, &coarse), None);
    let src = pick(scan.vertices, &sub_idx(scan.vertices.len(), 4000));
    let inv_n = frame::rigid_inverse(&m_n);
    let mut best: Option<(DMat4, f64)> = None;
    // the arch frames fix the pose up to a half turn: try the four symmetric candidates
    for flip in [
        [1.0, 1.0, 1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
    ] {
        let a0 = m_e * DMat4::from_scale(DVec3::from_array(flip)) * inv_n;
        let (t, err) = icp(
            &src,
            None,
            &coarse_target,
            a0,
            IcpOpts {
                iters: 40,
                trim: 0.7,
            },
        );
        if best.is_none_or(|b| err < b.1) {
            best = Some((t, err));
        }
    }
    let (t, _) = best.ok_or_else(|| VisitError::Registration("no candidate".to_owned()))?;
    // orientation of the scan normals (some STL files are wound inwards)
    let fine = sub_idx(scan.vertices.len(), 15_000);
    let full = Target::new(expected.vertices.to_vec(), Some(n_exp.clone()));
    let pts = pick(scan.vertices, &fine);
    let hits = full.nearest(&xform(&t, &pts));
    let r = rot(&t);
    let dots: Vec<f64> = fine
        .iter()
        .zip(&hits)
        .map(|(&i, h)| (r * n_scan[i]).dot(n_exp[h.0]))
        .collect();
    let dist: Vec<f64> = hits.iter().map(|h| h.1).collect();
    let cut = quantile(&dist, 0.5);
    let near_dots: Vec<f64> = dots
        .iter()
        .zip(&dist)
        .filter(|(_, d)| **d <= cut)
        .map(|(x, _)| *x)
        .collect();
    let sign = if near_dots.is_empty() || median(&near_dots) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let src_n: Vec<DVec3> = fine.iter().map(|&i| n_scan[i] * sign).collect();
    let (t, err) = icp(
        &pts,
        Some(&src_n),
        &full,
        t,
        IcpOpts {
            iters: 40,
            trim: 0.7,
        },
    );
    if !err.is_finite() {
        return Err(VisitError::Registration(
            "the scan does not overlap the plan".to_owned(),
        ));
    }
    Ok(Registration {
        transform: t,
        rms_mm: err,
        normal_sign: sign,
    })
}

// ---- achieved position of the teeth -------------------------------------------------------------------------

/// A crown of the plan, with where it is expected.
#[derive(Debug, Clone)]
pub struct ToothInput {
    /// Tooth number.
    pub fdi: Fdi,
    /// Rest frame (origin at the centre of resistance), in case coordinates.
    pub rest: DMat4,
    /// Expected frame at the stage the scan is compared with: the rest frame
    /// moved by the planned pose of that stage.
    pub expected: DMat4,
    /// Crown points in the rest frame.
    pub points: Vec<DVec3>,
    /// Unit normals of those points (outward).
    pub normals: Vec<DVec3>,
    /// `+1` when the x axis of the tooth frame points mesially, `-1`
    /// distally.
    pub md_sign: i8,
}

/// Components of the difference achieved - planned (`visit._tooth_deviation`):
/// `t_f` moves the expected crown (centre `centre`, frame `expected`) onto the
/// scan; linear at the crown centre, angular about the tooth axes.
pub fn tooth_deviation(
    expected: &DMat4,
    t_f: &DMat4,
    centre: DVec3,
    md_sign: i8,
) -> ToothDeviation {
    let r_e = rot(expected);
    let lin = r_e.transpose() * (t_f.transform_point3(centre) - centre);
    let rv =
        (r_e.transpose() * rotation::matrix_to_rotvec(&rot(t_f))) * (180.0 / std::f64::consts::PI);
    let md = f64::from(md_sign);
    ToothDeviation {
        mesial_mm: lin.x * md,
        buccal_mm: lin.y,
        extrusion_mm: lin.z,
        torque_deg: -rv.x,
        tip_deg: rv.y * md,
        rotation_deg: rv.z * md,
    }
}

/// State of one tooth between the two passes of [`track`].
struct Pass {
    input_idx: usize,
    p_exp: Vec<DVec3>,
    near: Vec<usize>,
    src: Vec<DVec3>,
    src_n: Vec<DVec3>,
    t: DMat4,
    err: f64,
}

/// The achieved position of every tooth on the registered scan (`visit.track`):
/// `scan` and `scan_normals` (outward) in the coordinates of the plan. Teeth
/// with fewer than 50 scan points around their expected crown, or whose
/// registration found no pairs, are left out. `order` is the arch order of
/// the teeth, for the neighbours of the second pass.
pub fn track(
    scan: &[DVec3],
    scan_normals: &[DVec3],
    teeth: &[ToothInput],
    order: &[Fdi],
    params: &Params,
) -> BTreeMap<Fdi, ToothTracking> {
    // pass 1: each expected crown on the scan around it
    let mut passes: Vec<Pass> = teeth
        .par_iter()
        .enumerate()
        .filter_map(|(k, tt)| {
            if tt.points.is_empty() {
                return None;
            }
            let p_exp = xform(&tt.expected, &tt.points);
            // scan points around the expected crown (its box in the tooth frame + 2 mm)
            let inv = frame::rigid_inverse(&tt.expected);
            let lo = tt
                .points
                .iter()
                .fold(DVec3::splat(f64::INFINITY), |m, p| m.min(*p))
                - DVec3::splat(NEAR_MARGIN_MM);
            let hi = tt
                .points
                .iter()
                .fold(DVec3::splat(f64::NEG_INFINITY), |m, p| m.max(*p))
                + DVec3::splat(NEAR_MARGIN_MM);
            let near: Vec<usize> = (0..scan.len())
                .filter(|&i| {
                    let l = inv.transform_point3(scan[i]);
                    l.cmpge(lo).all() && l.cmple(hi).all()
                })
                .collect();
            if near.len() < MIN_NEAR {
                return None;
            }
            let si = sub_idx(tt.points.len(), CROWN_SAMPLES);
            let src = pick(&p_exp, &si);
            let r = rot(&tt.expected);
            let src_n: Vec<DVec3> = si.iter().map(|&i| r * tt.normals[i]).collect();
            let target = Target::new(pick(scan, &near), Some(pick(scan_normals, &near)));
            let (t, err) = icp(
                &src,
                Some(&src_n),
                &target,
                DMat4::IDENTITY,
                IcpOpts {
                    iters: 60,
                    trim: 0.8,
                },
            );
            err.is_finite().then_some(Pass {
                input_idx: k,
                p_exp,
                near,
                src,
                src_n,
                t,
                err,
            })
        })
        .collect();
    passes.sort_by_key(|p| {
        let f = teeth[p.input_idx].fdi;
        order.iter().position(|&o| o == f).unwrap_or(usize::MAX)
    });

    // pass 2: the scan points closer to the crown than to its neighbours
    if params.two_pass && passes.len() > 1 {
        let trees: Vec<Target> = passes
            .par_iter()
            .map(|p| {
                let sample = pick(&p.p_exp, &sub_idx(p.p_exp.len(), OWNER_SAMPLES));
                Target::new(xform(&p.t, &sample), None)
            })
            .collect();
        let refined: Vec<Option<(DMat4, f64)>> = (0..passes.len())
            .into_par_iter()
            .map(|i| {
                let x = &passes[i];
                let pts = pick(scan, &x.near);
                let d_own = trees[i].nearest(&pts);
                let mut d_other = vec![f64::INFINITY; pts.len()];
                for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                    if j < passes.len() {
                        for (d, h) in d_other.iter_mut().zip(trees[j].nearest(&pts)) {
                            *d = d.min(h.1);
                        }
                    }
                }
                let own: Vec<usize> = x
                    .near
                    .iter()
                    .zip(d_own.iter().zip(&d_other))
                    .filter(|(_, (o, other))| o.1 <= **other && o.1 < OWN_DISTANCE_MM)
                    .map(|(&i, _)| i)
                    .collect();
                if own.len() < MIN_NEAR {
                    return None;
                }
                let target = Target::new(pick(scan, &own), Some(pick(scan_normals, &own)));
                Some(icp(
                    &x.src,
                    Some(&x.src_n),
                    &target,
                    x.t,
                    IcpOpts {
                        iters: 60,
                        trim: 0.85,
                    },
                ))
            })
            .collect();
        for (p, r) in passes.iter_mut().zip(refined) {
            if let Some((t, err)) = r {
                p.t = t;
                p.err = err;
            }
        }
    }

    passes
        .iter()
        .map(|p| {
            let tt = &teeth[p.input_idx];
            (tt.fdi, tooth_tracking(tt, p, params))
        })
        .collect()
}

fn tooth_tracking(tt: &ToothInput, p: &Pass, params: &Params) -> ToothTracking {
    let n = p.p_exp.len() as f64;
    let centre = p.p_exp.iter().copied().sum::<DVec3>() / n;
    let p_rest = xform(&tt.rest, &tt.points);
    let (mut planned_norm, mut dot, mut pp) = (0.0, 0.0, 0.0);
    for (pe, pr) in p.p_exp.iter().zip(&p_rest) {
        let planned = *pe - *pr;
        let achieved = p.t.transform_point3(*pe) - *pr;
        planned_norm += planned.length();
        dot += achieved.dot(planned);
        pp += planned.dot(planned);
    }
    let planned_mm = planned_norm / n;
    let efficacy_pct = (planned_mm >= MIN_PLANNED_MM).then(|| 100.0 * dot / pp);
    let deviation = tooth_deviation(&tt.expected, &p.t, centre, tt.md_sign);
    let (linear_mm, angular_deg) = (deviation.linear_mm(), deviation.angular_deg());
    // the achieved position relative to the rest pose, the convention of `Stage::poses`
    let achieved = rotation::pose_from_mat4(&(frame::rigid_inverse(&tt.rest) * p.t * tt.expected));
    ToothTracking {
        on_track: linear_mm <= params.linear_mm && angular_deg <= params.angular_deg,
        linear_mm,
        angular_deg,
        efficacy_pct,
        planned_mm,
        rms_mm: p.err,
        scan_points: p.near.len() as u32,
        deviation,
        achieved,
    }
}

// ---- the whole analysis -------------------------------------------------------------------------------------

/// Input of [`run`].
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Arch of the scan.
    pub arch: Arch,
    /// The model expected at the stage the patient is at.
    pub expected: MeshRef<'a>,
    /// The progress scan, anywhere in space.
    pub scan: MeshRef<'a>,
    /// The crowns of the plan.
    pub teeth: &'a [ToothInput],
}

/// Result of [`run`].
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Placement of the scan on the plan.
    pub registration: Registration,
    /// The tracking of every tooth found.
    pub teeth: BTreeMap<Fdi, ToothTracking>,
    /// Teeth of the plan that were not found on the scan.
    pub untracked: Vec<Fdi>,
    /// Number of teeth on track.
    pub on_track: usize,
    /// What the visit suggests.
    pub decision: Decision,
    /// Velocity factor of the teeth that are behind.
    pub speed: BTreeMap<Fdi, f64>,
}

/// Analyses a control visit: registers the scan to the expected model and
/// measures every tooth against the plan (`visit.new_visit` without the
/// re-plan).
///
/// # Errors
/// As [`register_scan`]; [`VisitError::NoTeeth`] when no tooth was found.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, VisitError> {
    let registration = register_scan(input.expected, input.scan)?;
    let (moved, normals) = registered_scan(&registration, input.scan);
    let order = anatomy::arch_order(input.arch);
    let teeth = track(&moved, &normals, input.teeth, &order, params);
    if teeth.is_empty() {
        return Err(VisitError::NoTeeth);
    }
    Ok(summarize(registration, teeth, input.teeth, params))
}

/// The scan placed on the plan by `registration`: its vertices and its outward
/// vertex normals (the sign of the registration applied).
pub fn registered_scan(registration: &Registration, scan: MeshRef<'_>) -> (Vec<DVec3>, Vec<DVec3>) {
    let moved = xform(&registration.transform, scan.vertices);
    let normals = vertex_normals(&moved, scan.tris)
        .into_iter()
        .map(|n| n * registration.normal_sign)
        .collect();
    (moved, normals)
}

/// The decision of a visit from the tracking of its teeth.
pub fn summarize(
    registration: Registration,
    teeth: BTreeMap<Fdi, ToothTracking>,
    inputs: &[ToothInput],
    params: &Params,
) -> Output {
    let untracked: Vec<Fdi> = inputs
        .iter()
        .map(|t| t.fdi)
        .filter(|f| !teeth.contains_key(f))
        .collect();
    let on_track = teeth.values().filter(|t| t.on_track).count();
    let speed: BTreeMap<Fdi, f64> = teeth
        .iter()
        .filter(|(_, t)| !t.on_track)
        .map(|(&f, _)| (f, params.lag_speed))
        .collect();
    let decision = if speed.is_empty() {
        Decision::OnTrack
    } else {
        Decision::Refinement
    };
    Output {
        registration,
        teeth,
        untracked,
        on_track,
        decision,
        speed,
    }
}

// ---- the plan at a stage -------------------------------------------------------------------------------------

/// Pose of a tooth at stage `stage` of the plan, relative to its rest pose
/// (`stage` 0 is the initial situation; a tooth without a pose at that stage
/// has the pose of the latest stage before it that has one).
pub fn pose_at(stages: &[Stage], fdi: Fdi, stage: u32) -> Pose {
    let mut sorted: Vec<&Stage> = stages.iter().filter(|s| s.index <= stage).collect();
    sorted.sort_by_key(|s| s.index);
    sorted
        .iter()
        .rev()
        .find_map(|s| s.poses.get(&fdi).copied())
        .unwrap_or_default()
}

/// Expected frame of a tooth at `stage`: its rest pose moved by the planned
/// pose of that stage (`visit.expected_crown`).
pub fn expected_frame(rest: &Pose, stages: &[Stage], fdi: Fdi, stage: u32) -> DMat4 {
    rest.compose(&pose_at(stages, fdi, stage)).to_mat4()
}

/// Rigid motion (`x' = D x`, case coordinates) of a tooth from its rest
/// position to stage `stage`.
pub fn delta_at(rest: &Pose, stages: &[Stage], fdi: Fdi, stage: u32) -> DMat4 {
    expected_frame(rest, stages, fdi, stage) * frame::rigid_inverse(&rest.to_mat4())
}

#[cfg(test)]
mod tests;
