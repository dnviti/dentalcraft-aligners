//! Tooth reconstruction: every tooth a closed solid.
//!
//! Reference: `ortho_aligner/reconstruct.py` (`closed_tooth`, `crown_patch`,
//! `close_crown` and their helpers). The gingiva without holes and its
//! deformation are in [`crate::gingiva`].
//!
//! An intraoral scan records only what the light reaches: neighbouring
//! crowns are joined by a bridge of surface at the contact, every crown is
//! an open shell ending at the gingival margin, and the margin follows the
//! zig-zag of the triangles. Here, per tooth:
//!
//! 1. the crown is cut along a smooth margin line: the 0.5 iso-line of the
//!    tooth label smoothed on the surface (marching triangles);
//! 2. the crown surface within [`BRIDGE_MM`] of the neighbouring crown (the
//!    scanner bridge), thin sheets and the strip next to the margin are
//!    rebuilt;
//! 3. the opening is closed by rings narrowing to a neck [`Params::neck_depth_mm`]
//!    below the lowest margin point and a rounded stump below it, faired as
//!    the fairest surface tangent to the crown (minimum of the discrete
//!    thin-plate energy, Kobbelt et al. 1998);
//! 4. the rebuilt surface is held on the convex outline of every horizontal
//!    section of the crown, narrowed under the margin, brought to the
//!    contact surface where the scan joined two crowns, and despiked.
//!
//! Everything is in arch coordinates (occlusal = `+Z`). The result is a
//! watertight, consistently wound solid per tooth.

mod fair;
pub mod geom;
mod outline;
pub(crate) mod solve;

use std::collections::BTreeMap;

use dc_case::Fdi;
use glam::DVec3;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::evidence;
use fair::Weight;
use geom::{normalize, unique_edges};

/// Crown surface rebuilt next to the neighbouring crown (scanner bridge), mm.
pub const BRIDGE_MM: f64 = 0.8;
/// Crown surface rebuilt next to the margin (the scan bends towards the
/// gingiva), mm.
pub const RIM_MM: f64 = 0.5;
/// Thinner than this near a neighbour is a sheet closed by the scanner, mm.
pub const THIN_MM: f64 = 0.35;
/// Distance from the neighbouring crown where thin sheets are looked for, mm.
pub const THIN_REACH: f64 = 2.0;
/// Occlusal / incisal band kept as scanned next to the neighbour, mm.
pub const KEEP_TOP_MM: f64 = 1.0;
/// Top of the contact line held where the scanner saw it, mm.
pub const CONTACT_MM: f64 = 0.5;
/// Section of the neck over the section of the crown.
pub const NECK_SCALE: f64 = 0.85;
/// The closed tooth ends this far below the lowest margin point, mm.
pub const APEX_MM: f64 = 3.0;
/// Smoothing iterations of the label indicator before cutting.
pub const SMOOTH_ITER: usize = 6;
/// Thickness of the horizontal slabs of the convex outlines, mm.
pub const CONVEX_SLAB_MM: f64 = 0.4;
/// Fewest crown points of a slab for its outline.
pub const CONVEX_MIN_PTS: usize = 12;
/// Weight holding the rebuilt surface on the convex outline.
pub const CONVEX_W: f64 = 0.6;
/// Clearance kept from the hidden proximal wall to the contact surface, mm.
pub const CONTACT_FILL_MM: f64 = 0.04;
/// Weight holding a wall on the contact surface.
pub const CONTACT_W: f64 = 2.0;
/// Rounds of the contact fill.
pub const CONTACT_ROUNDS: usize = 4;
/// Height of the slices of the contact surface, mm.
pub const CONTACT_SLAB_MM: f64 = 0.5;
/// A wall is moved to the contact surface from at most this far, mm.
pub const CONTACT_REACH_MM: f64 = 2.0;
/// Rounds of the narrowing under the margin.
pub const TAPER_ROUNDS: usize = 3;
/// The narrowing applies where the margin is at most this above its lowest
/// point, mm.
pub const TAPER_RIM_MM: f64 = 2.5;
/// Band either side of the margin cut faired as one surface, mm.
pub const RIM_BAND_MM: f64 = 1.0;
/// Weight holding that band where it is.
pub const RIM_BAND_W: f64 = 0.05;
/// A closed tooth has no edge sharper than this (degrees).
pub const SPIKE_DEG: f64 = 60.0;
/// Rings around a sharp vertex faired with it.
pub const SPIKE_RINGS: usize = 3;
/// Weight holding the despiked vertices where they are.
pub const SPIKE_W: f64 = 0.25;
/// Rounds of the despiking.
pub const SPIKE_ROUNDS: usize = 10;
/// The convex outline never brings the surface closer than this to a
/// neighbouring crown, mm.
pub const CONVEX_CLEAR_MM: f64 = 0.1;
/// Weight of the last fairing after the convex outlines.
pub const CONVEX_SMOOTH_W: f64 = 0.08;
/// Rounds of the convex outlines.
pub const CONVEX_ROUNDS: usize = 3;
/// Cut points are kept this far (edge parameter) from mesh vertices.
pub const T_MIN: f64 = 0.12;
/// Target edge length of the rebuilt rings, mm.
pub const TARGET_EDGE_MM: f64 = 0.35;
/// Weight of the elastic anchors of the first fairing.
const ANCHOR_W: f64 = 0.3;

/// Errors of the reconstruction.
#[derive(Debug, thiserror::Error)]
pub enum ReconstructError {
    /// The input is not usable.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The tooth has no crown surface to close.
    #[error("tooth {0} has no crown surface")]
    NoCrown(u16),
}

/// Parameters of the reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to work on (the command uses it; the algorithm works on one
    /// arch at a time).
    pub arch: crate::teeth::Which,
    /// Depth of the neck (cemento-enamel junction) below the lowest margin
    /// point, mm (sulcus 0.69 + junctional epithelium 0.97, Gargiulo 1961).
    pub neck_depth_mm: f64,
    /// Depth of the end of the closed tooth below the lowest margin point, mm.
    pub apex_mm: f64,
    /// Section of the neck over the section of the crown.
    pub neck_scale: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: crate::teeth::Which::All,
            neck_depth_mm: evidence::param_f64("neck_depth").unwrap_or(1.66),
            apex_mm: APEX_MM,
            neck_scale: NECK_SCALE,
        }
    }
}

/// Parameters of the reconstruction of a single tooth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ToothParams {
    /// FDI number of the tooth to rebuild (required).
    pub fdi: u8,
    /// Depth of the neck below the lowest margin point, mm.
    pub neck_depth_mm: f64,
    /// Depth of the end of the closed tooth below the lowest margin point, mm.
    pub apex_mm: f64,
    /// Section of the neck over the section of the crown.
    pub neck_scale: f64,
}

impl Default for ToothParams {
    fn default() -> Self {
        let p = Params::default();
        Self {
            fdi: 0,
            neck_depth_mm: p.neck_depth_mm,
            apex_mm: p.apex_mm,
            neck_scale: p.neck_scale,
        }
    }
}

impl ToothParams {
    /// The reconstruction parameters of these.
    pub fn params(&self) -> Params {
        Params {
            neck_depth_mm: self.neck_depth_mm,
            apex_mm: self.apex_mm,
            neck_scale: self.neck_scale,
            ..Params::default()
        }
    }
}

/// Input of the reconstruction: a labelled arch scan in arch coordinates and
/// the long axis of every tooth to close (from the teeth stage).
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Vertices in arch coordinates (mm, `+Z` occlusal).
    pub vertices: &'a [DVec3],
    /// Triangles.
    pub tris: &'a [[u32; 3]],
    /// FDI label of every vertex (`0` = gingiva).
    pub labels: &'a [u16],
    /// Long axis of every tooth to close (arch coordinates).
    pub axes: &'a BTreeMap<Fdi, DVec3>,
}

/// Numbers of one closed tooth (`closed_tooth`'s info).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClosedInfo {
    /// Vertices of the open crown.
    pub crown_vertices: usize,
    /// Vertices added to close it.
    pub fill_vertices: usize,
    /// Crown vertices of the scanner bridge.
    pub bridge_vertices: usize,
    /// Crown vertices of thin sheets.
    pub thin_vertices: usize,
    /// Index of the closing axis kept (0: the long axis).
    pub closing_axis: usize,
    /// Faces crossing another face.
    pub crossing_faces: usize,
    /// Rebuilt vertices held on the convex outline.
    pub convex_held: usize,
    /// Vertices moved by the despiking.
    pub despiked: usize,
    /// Sharp vertices left.
    pub sharp_left: usize,
    /// Vertices brought to a contact surface.
    pub contact_filled: usize,
    /// Rebuilt vertices narrowed under the margin.
    pub tapered: usize,
}

/// One closed tooth, arch coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedTooth {
    /// Vertices.
    pub vertices: Vec<DVec3>,
    /// Triangles, wound with outward normals.
    pub tris: Vec<[u32; 3]>,
    /// Numbers of the reconstruction.
    pub info: ClosedInfo,
}

/// Result of the reconstruction of an arch.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The closed teeth.
    pub teeth: BTreeMap<Fdi, ClosedTooth>,
}

/// Closes every tooth of `input.axes` (in parallel).
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, ReconstructError> {
    run_with_progress(input, params, &|_, _| {})
}

/// [`run`], calling `progress(done, total)` each time a tooth is closed
/// (from the worker threads, in completion order; `total` is the number of
/// teeth of `input.axes`).
pub fn run_with_progress(
    input: &Input<'_>,
    params: &Params,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Output, ReconstructError> {
    check(input)?;
    let edges = unique_edges(input.tris);
    let fdis: Vec<Fdi> = input.axes.keys().copied().collect();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let teeth: Result<Vec<(Fdi, ClosedTooth)>, ReconstructError> = fdis
        .par_iter()
        .map(|&f| {
            let axis = input.axes[&f];
            let t = closed_tooth_with(input, &edges, f, axis, params).map(|t| (f, t));
            let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            progress(k, fdis.len());
            t
        })
        .collect();
    Ok(Output {
        teeth: teeth?.into_iter().collect(),
    })
}

fn check(input: &Input<'_>) -> Result<(), ReconstructError> {
    if input.labels.len() != input.vertices.len() {
        return Err(ReconstructError::Invalid(format!(
            "{} labels for {} vertices",
            input.labels.len(),
            input.vertices.len()
        )));
    }
    if input
        .tris
        .iter()
        .flatten()
        .any(|&i| i as usize >= input.vertices.len())
    {
        return Err(ReconstructError::Invalid(
            "a triangle refers to a missing vertex".to_owned(),
        ));
    }
    Ok(())
}

/// Closed solid of one tooth (`closed_tooth`). The crown is closed along its
/// long axis; when that solid folds through itself, the other closing axes
/// are tried and the solid with the fewest crossing faces, then the least
/// rebuilt surface, is kept.
pub fn closed_tooth(
    input: &Input<'_>,
    fdi: Fdi,
    axis: DVec3,
    params: &Params,
) -> Result<ClosedTooth, ReconstructError> {
    check(input)?;
    closed_tooth_with(input, &unique_edges(input.tris), fdi, axis, params)
}

fn closed_tooth_with(
    input: &Input<'_>,
    edges: &[[u32; 2]],
    fdi: Fdi,
    axis: DVec3,
    params: &Params,
) -> Result<ClosedTooth, ReconstructError> {
    let f = u16::from(fdi.get());
    let patch = crown_patch(input.vertices, input.tris, input.labels, edges, f);
    if patch.tris.is_empty() {
        return Err(ReconstructError::NoCrown(f));
    }
    let others = neighbour_points(input.vertices, input.labels, f, &patch.vertices, 3.0);
    /// ((crossings, filled), vertices, triangles, axis index) of the best solid.
    type Best = ((usize, usize), Vec<DVec3>, Vec<[u32; 3]>, usize);
    let mut best: Option<Best> = None;
    let mut last = Stats::default();
    let axes = closing_axes(&patch.vertices, &patch.tris, axis);
    let attempt = |k: usize| {
        let (vt, tt, n_fill, stats) = close_crown(&patch, axes[k], params, &others);
        let folds = geom::self_intersections(&vt, &tt);
        (vt, tt, n_fill, stats, folds)
    };
    // the long axis first; when that solid folds, the other axes together
    // (each attempt is independent, the choice below is the sequential one)
    let first = attempt(0);
    let rest: Vec<_> = if first.4 == 0 {
        Vec::new()
    } else {
        (1..axes.len()).into_par_iter().map(attempt).collect()
    };
    for (k, (vt, tt, n_fill, stats, folds)) in std::iter::once(first).chain(rest).enumerate() {
        last = stats;
        if best.as_ref().is_none_or(|b| (folds, n_fill) < b.0) {
            best = Some(((folds, n_fill), vt, tt, k));
        }
        if folds == 0 {
            break;
        }
    }
    let Some(((folds, n_fill), vt, tt, k)) = best else {
        return Err(ReconstructError::NoCrown(f));
    };
    Ok(ClosedTooth {
        vertices: vt,
        tris: tt,
        info: ClosedInfo {
            crown_vertices: patch.vertices.len(),
            fill_vertices: n_fill,
            bridge_vertices: patch.bridge.iter().filter(|&&b| b).count(),
            thin_vertices: patch.thin.iter().filter(|&&b| b).count(),
            closing_axis: k,
            crossing_faces: folds,
            convex_held: last.convex,
            despiked: last.despiked,
            sharp_left: last.sharp_left,
            contact_filled: last.contact_filled,
            tapered: last.tapered,
        },
    })
}

/// Scanned vertices of the other crowns within `reach` of the box of the
/// crown `vc` (`neighbour_points`).
fn neighbour_points(v: &[DVec3], labels: &[u16], fdi: u16, vc: &[DVec3], reach: f64) -> Vec<DVec3> {
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in vc {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let (lo, hi) = (lo - reach, hi + reach);
    v.iter()
        .zip(labels)
        .filter(|(p, l)| **l != 0 && **l != fdi && p.cmpge(lo).all() && p.cmple(hi).all())
        .map(|(p, _)| *p)
        .collect()
}

/// The open crown of a tooth (`crown_patch`).
#[derive(Debug, Clone)]
struct Patch {
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    /// Crown vertices of the scanner bridge (within [`BRIDGE_MM`] of a
    /// neighbouring crown).
    bridge: Vec<bool>,
    /// Crown vertices of thin sheets near a neighbour.
    thin: Vec<bool>,
    /// Contact surfaces with the neighbours.
    contacts: Vec<outline::ContactSurface>,
}

/// Open crown of tooth `fdi` cut along the smooth margin line.
fn crown_patch(
    v: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    edges: &[[u32; 2]],
    fdi: u16,
) -> Patch {
    let n = v.len();
    let ind: Vec<f64> = labels
        .iter()
        .map(|&l| f64::from(u8::from(l == fdi)))
        .collect();
    let phi_f = geom::smooth_scalar(&ind, edges, SMOOTH_ITER, 0.5);
    let near: Vec<bool> = phi_f.iter().map(|&x| x > 0.02).collect();
    let sel: Vec<[u32; 3]> = tris
        .iter()
        .filter(|t| t.iter().any(|&i| near[i as usize]))
        .copied()
        .collect();
    let (vl, tl, used) = geom::compact(v, &sel);
    let phi_l: Vec<f64> = used.iter().map(|&i| phi_f[i] - 0.5).collect();
    // distance to the neighbouring crowns (their vertices touching this crown)
    let other = |i: usize| labels[i] != 0 && labels[i] != fdi;
    let mut touch: Vec<u32> = Vec::new();
    for e in edges {
        let (a, b) = (e[0] as usize, e[1] as usize);
        if labels[a] == fdi && other(b) {
            touch.push(e[1]);
        }
        if labels[b] == fdi && other(a) {
            touch.push(e[0]);
        }
    }
    touch.sort_unstable();
    touch.dedup();
    let dist = if touch.is_empty() {
        vec![f64::INFINITY; vl.len()]
    } else {
        let tp: Vec<DVec3> = touch.iter().map(|&i| v[i as usize]).collect();
        geom::min_distances(&vl, &tp)
    };
    let contacts = outline::contact_surfaces(v, labels, edges, fdi, CONTACT_SLAB_MM, 0.15);
    let cut = geom::cut_mesh(&vl, &tl, &phi_l, T_MIN);
    let mut d2 = dist.clone();
    for i in 0..cut.p.len() {
        let (dp, dq) = (dist[cut.p[i] as usize], dist[cut.q[i] as usize]);
        d2.push(if dp.is_finite() && dq.is_finite() {
            dp + cut.t[i] * (dq - dp)
        } else {
            dp.min(dq)
        });
    }
    let tc: Vec<[u32; 3]> = cut
        .tris
        .iter()
        .zip(&cut.side)
        .filter(|(_, s)| **s)
        .map(|(t, _)| *t)
        .collect();
    let keep = geom::largest_face_component(&tc);
    let tc: Vec<[u32; 3]> = tc
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(t, _)| *t)
        .collect();
    let (vc, tc, u2) = geom::compact(&cut.vertices, &tc);
    let dn: Vec<f64> = u2.iter().map(|&i| d2[i]).collect();
    let bridge: Vec<bool> = dn.iter().map(|&d| d < BRIDGE_MM).collect();
    let mut thin = vec![false; vc.len()];
    let near_nb: Vec<usize> = (0..vc.len())
        .filter(|&i| dn[i].is_finite() && dn[i] < THIN_REACH)
        .collect();
    if !near_nb.is_empty() {
        let th = outline::thickness(&vc, &tc, &near_nb);
        for (k, &i) in near_nb.iter().enumerate() {
            if th[k] < THIN_MM {
                thin[i] = true;
            }
        }
    }
    let _ = n;
    Patch {
        vertices: vc,
        tris: tc,
        bridge,
        thin,
        contacts,
    }
}

/// Directions to close an open crown along (`closing_axes`): the long axis,
/// then the side walls of the scanned crown, the vertical of the arch and
/// the normal of the margin loop (when they differ from the ones before by
/// more than 4 degrees and point occlusally).
fn closing_axes(vc: &[DVec3], tc: &[[u32; 3]], axis: DVec3) -> Vec<DVec3> {
    let mut out = vec![normalize(axis)];
    let all: Vec<usize> = (0..tc.len()).collect();
    let mut cands = vec![
        crate::teeth::wall_axis(vc, tc, &all, Some(out[0])),
        DVec3::Z,
    ];
    let loops = geom::boundary_loops(tc);
    if !loops.is_empty() {
        // max(loops, key=len): the first of the longest
        let mut li = 0;
        for (i, l) in loops.iter().enumerate() {
            if l.len() > loops[li].len() {
                li = i;
            }
        }
        let r: Vec<DVec3> = loops[li].iter().map(|&i| vc[i as usize]).collect();
        let c = geom::mean(&r);
        let nrm = geom::plane_normal(&r, c);
        cands.push(if nrm.dot(out[0]) >= 0.0 { nrm } else { -nrm });
    }
    let cos4 = 4.0f64.to_radians().cos();
    for c in cands {
        let c = normalize(c);
        if c.z > 0.5 && out.iter().all(|o| c.dot(*o) < cos4) {
            out.push(c);
        }
    }
    out
}

/// Faces between ring `p` (border already made, arc parameters `u_p` in
/// `[0, 1)`) and the next ring `q` (`n_q` vertices at parameters `j / n_q`),
/// wound opposite to the border edges `p[i] -> p[i+1]` (`_ring_band`).
fn ring_band(p: &[u32], q: &[u32], u_p: &[f64]) -> Vec<[u32; 3]> {
    let (n_p, n_q) = (p.len(), q.len());
    let mut mp: Vec<usize> = u_p
        .iter()
        .map(|&u| ((u * n_q as f64 + 1e-9).floor().max(0.0) as usize).min(n_q - 1))
        .collect();
    for i in 1..n_p {
        mp[i] = mp[i].max(mp[i - 1]);
    }
    let mut faces = Vec::new();
    for i in 0..n_p {
        let j0 = mp[i];
        let j1 = if i + 1 < n_p { mp[i + 1] } else { n_q };
        let (a, b) = (p[i], p[(i + 1) % n_p]);
        faces.push([b, a, q[j0 % n_q]]);
        for j in j0..j1 {
            faces.push([b, q[j % n_q], q[(j + 1) % n_q]]);
        }
    }
    faces
}

/// Counters of one `close_crown` run.
#[derive(Debug, Clone, Copy, Default)]
struct Stats {
    convex: usize,
    tapered: usize,
    contact_filled: usize,
    despiked: usize,
    sharp_left: usize,
}

fn neighbours_of(mask: &[bool], edges: &[[u32; 2]]) -> Vec<bool> {
    let mut near = mask.to_vec();
    for e in edges {
        if mask[e[0] as usize] {
            near[e[1] as usize] = true;
        }
        if mask[e[1] as usize] {
            near[e[0] as usize] = true;
        }
    }
    near
}

/// Closed solid from an open crown (`close_crown`): `(vertices, triangles,
/// vertices added, counters)`.
fn close_crown(
    patch: &Patch,
    axis: DVec3,
    params: &Params,
    others: &[DVec3],
) -> (Vec<DVec3>, Vec<[u32; 3]>, usize, Stats) {
    let mut stats = Stats::default();
    let (vc, tc) = (&patch.vertices, &patch.tris);
    let n0 = vc.len();
    let mut loops = geom::boundary_loops(tc);
    if loops.is_empty() {
        return (vc.clone(), tc.clone(), 0, stats);
    }
    loops.sort_by_key(|l| std::cmp::Reverse(l.len()));
    let neck_mm = params.neck_depth_mm;
    let neck_scale = params.neck_scale;
    let main = loops[0].clone();
    let r: Vec<DVec3> = main.iter().map(|&i| vc[i as usize]).collect();
    let n = main.len();
    let c = geom::mean(&r);
    let mut axis = normalize(axis);
    let mut nrm = geom::plane_normal(&r, c);
    if (geom::mean(vc) - c).dot(nrm) < 0.0 {
        nrm = -nrm;
    }
    if nrm.dot(axis) > 0.87 {
        // within 30 degrees: a well-defined margin plane
        axis = nrm;
    }
    let h: Vec<f64> = r.iter().map(|p| (*p - c).dot(axis)).collect();
    let hmin = h.iter().copied().fold(f64::INFINITY, f64::min);
    let per: f64 = (0..n).map(|i| (r[(i + 1) % n] - r[i]).length()).sum();
    // np.clip(x, 12, n): the upper bound wins when n < 12
    let m1 = ((per / TARGET_EDGE_MM).round_ties_even() as usize)
        .max(12)
        .min(n);
    // rings parametrised by the angle around the axis
    let e1 = normalize(axis.cross(if axis.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    }));
    let e2 = axis.cross(e1);
    let rx: Vec<f64> = r.iter().map(|p| (*p - c).dot(e1)).collect();
    let ry: Vec<f64> = r.iter().map(|p| (*p - c).dot(e2)).collect();
    let th_raw: Vec<f64> = rx.iter().zip(&ry).map(|(x, y)| y.atan2(*x)).collect();
    let th = geom::unwrap(&th_raw);
    let two_pi = 2.0 * std::f64::consts::PI;
    let sgn = if th[n - 1] >= th[0] { 1.0 } else { -1.0 };
    let mut u_rim: Vec<f64> = th
        .iter()
        .map(|&t| (sgn * (t - th[0]) / two_pi).max(0.0))
        .collect();
    for i in 1..n {
        u_rim[i] = u_rim[i].max(u_rim[i - 1]);
    }
    for u in &mut u_rim {
        *u = u.min(1.0 - 1e-9);
    }
    let r_rim: Vec<f64> = rx.iter().zip(&ry).map(|(x, y)| x.hypot(*y)).collect();
    let u1: Vec<f64> = (0..m1).map(|i| i as f64 / m1 as f64).collect();
    let mut rad = vec![-1.0f64; m1];
    for i in 0..n {
        let b = (((u_rim[i] * m1 as f64 + 0.5) as usize) % m1).min(m1 - 1);
        rad[b] = rad[b].max(r_rim[i]);
    }
    let (hx, hf): (Vec<f64>, Vec<f64>) = (0..m1)
        .filter(|&i| rad[i] > 0.0)
        .map(|i| (u1[i], rad[i]))
        .unzip();
    let mut rad = geom::interp_periodic(&u1, &hx, &hf, 1.0);
    for _ in 0..2usize.max(m1 / 20) {
        let prev = rad.clone();
        for i in 0..m1 {
            rad[i] = 0.5 * prev[i] + 0.25 * (prev[(i + m1 - 1) % m1] + prev[(i + 1) % m1]);
        }
    }
    let dirs: Vec<DVec3> = u1
        .iter()
        .map(|&u| {
            let ang = th[0] + sgn * two_pi * u;
            ang.cos() * e1 + ang.sin() * e2
        })
        .collect();
    let hu = geom::interp_periodic(&u1, &u_rim, &h, 1.0);
    let top: Vec<DVec3> = (0..m1)
        .map(|i| c + dirs[i] * rad[i] + hu[i] * axis)
        .collect();
    let base = c + axis * (hmin - neck_mm);
    let neck: Vec<DVec3> = (0..m1)
        .map(|i| base + dirs[i] * (neck_scale * rad[i]))
        .collect();
    let apex = c + axis * (hmin - params.apex_mm);

    let mut verts: Vec<DVec3> = vc.clone();
    let mut faces: Vec<[u32; 3]> = tc.clone();
    let mut fixed: Vec<u32> = Vec::new();
    let mut prev: Vec<u32> = main.clone();
    let mut u_prev: Vec<f64> = u_rim.clone();
    let mut ring = |pts: &[DVec3], verts: &mut Vec<DVec3>, faces: &mut Vec<[u32; 3]>| -> Vec<u32> {
        let m = pts.len();
        let base = verts.len() as u32;
        let ids: Vec<u32> = (0..m as u32).map(|k| base + k).collect();
        verts.extend_from_slice(pts);
        faces.extend(ring_band(&prev, &ids, &u_prev));
        prev = ids.clone();
        u_prev = (0..m).map(|k| k as f64 / m as f64).collect();
        ids
    };
    let mean_len = |a: &[DVec3], b: &[DVec3]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (*x - *y).length())
            .sum::<f64>()
            / a.len() as f64
    };
    let d1 = mean_len(&neck, &top);
    let k1 = ((d1 / TARGET_EDGE_MM).round_ties_even() as i64).clamp(2, 30) as usize;
    for k in 1..=k1 {
        let s = k as f64 / (k1 as f64 + 1.0);
        let p: Vec<DVec3> = (0..m1).map(|i| (1.0 - s) * top[i] + s * neck[i]).collect();
        ring(&p, &mut verts, &mut faces);
    }
    fixed.extend(ring(&neck, &mut verts, &mut faces));
    let apex_v = vec![apex; m1];
    let d2 = mean_len(&neck, &apex_v);
    let k2 = ((d2 / TARGET_EDGE_MM).round_ties_even() as i64).clamp(2, 30) as usize;
    let neck_c: [Vec<f64>; 3] = [
        neck.iter().map(|p| p.x).collect(),
        neck.iter().map(|p| p.y).collect(),
        neck.iter().map(|p| p.z).collect(),
    ];
    for k in 1..=k2 {
        let s = k as f64 / (k2 as f64 + 1.0);
        let m = 6usize.max((m1 as f64 * (1.0 - s).sqrt()).round_ties_even() as usize);
        let u: Vec<f64> = (0..m).map(|i| i as f64 / m as f64).collect();
        let px = geom::interp_periodic(&u, &u1, &neck_c[0], 1.0);
        let py = geom::interp_periodic(&u, &u1, &neck_c[1], 1.0);
        let pz = geom::interp_periodic(&u, &u1, &neck_c[2], 1.0);
        let p: Vec<DVec3> = (0..m)
            .map(|i| (1.0 - s) * DVec3::new(px[i], py[i], pz[i]) + s * apex)
            .collect();
        ring(&p, &mut verts, &mut faces);
    }
    drop(ring);
    let bi = verts.len() as u32;
    verts.push(apex);
    let np_ = prev.len();
    for i in 0..np_ {
        faces.push([prev[(i + 1) % np_], prev[i], bi]);
    }
    fixed.push(bi);
    // small holes of the scan inside the crown: fan to a free centre
    for lp in &loops[1..] {
        let ci = verts.len() as u32;
        let pts: Vec<DVec3> = lp.iter().map(|&i| vc[i as usize]).collect();
        verts.push(geom::mean(&pts));
        let m = lp.len();
        for i in 0..m {
            faces.push([lp[(i + 1) % m], lp[i], ci]);
        }
    }
    let mut v = verts;
    let t = faces;
    let nv = v.len();
    let mut is_fixed = vec![false; nv];
    for &i in &fixed {
        is_fixed[i as usize] = true;
    }
    let mut free = vec![false; nv];
    for (i, f) in free.iter_mut().enumerate().skip(n0) {
        *f = !is_fixed[i];
    }
    let mut anchor = vec![false; nv];
    // occlusal / incisal band: stays as scanned
    let h_all: Vec<f64> = vc.iter().map(|p| (*p - c).dot(axis)).collect();
    let hmax = h_all.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let top_band: Vec<bool> = h_all.iter().map(|&x| x >= hmax - KEEP_TOP_MM).collect();
    let soft: Vec<bool> = (0..n0).map(|i| patch.bridge[i] && !top_band[i]).collect();
    for i in 0..n0 {
        free[i] |= soft[i];
    }
    // the contact point (top of every run of the margin loop along a neighbour) held elastically
    let on: Vec<usize> = (0..n).filter(|&k| soft[main[k] as usize]).collect();
    if !on.is_empty() {
        let mut runs: Vec<Vec<usize>> = Vec::new();
        for &k in &on {
            match runs.last_mut() {
                Some(r) if k - *r.last().unwrap_or(&k) <= 1 => r.push(k),
                _ => runs.push(vec![k]),
            }
        }
        if runs.len() > 1 && runs[0][0] == 0 && *runs[runs.len() - 1].last().unwrap_or(&0) == n - 1
        {
            let last = runs.pop().unwrap_or_default();
            let first = runs.remove(0);
            let mut merged = last;
            merged.extend(first);
            runs.insert(0, merged);
        }
        for run in &runs {
            let hm: Vec<f64> = run.iter().map(|&k| h_all[main[k] as usize]).collect();
            let mx = hm.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            for (k, &x) in run.iter().zip(&hm) {
                if x >= mx - CONTACT_MM {
                    anchor[main[*k] as usize] = true;
                }
            }
        }
    }
    let mut on_main = vec![false; n0];
    for &i in &main {
        on_main[i as usize] = true;
    }
    for i in 0..n0 {
        if patch.thin[i] && !on_main[i] {
            free[i] = true;
        }
    }
    // the last strip of scan before the margin: rebuilt
    let strip = geom::min_distances(vc, &r);
    for i in 0..n0 {
        if strip[i] < RIM_MM && !on_main[i] && !top_band[i] {
            free[i] = true;
        }
    }
    // energy on the free vertices and their neighbours (tangent continuity)
    let edges = unique_edges(&t);
    let mut rows = neighbours_of(&free, &edges);
    for &i in &fixed {
        rows[i as usize] = true;
    }
    v = fair::fair(&v, &t, &free, &rows, Some(&anchor), Weight::All(ANCHOR_W));
    v = fair::untangle(&v, &t, &free, &edges, 10, 10);
    // no hollow in the crown: the convex outline of every horizontal section
    let keep: Vec<bool> = (0..n0).map(|i| !free[i]).collect();
    let mut held = 0usize;
    for rnd in 0..CONVEX_ROUNDS {
        let ids: Vec<usize> = (0..nv).filter(|&i| free[i]).collect();
        let q: Vec<DVec3> = ids.iter().map(|&i| v[i]).collect();
        let tg = outline::convex_targets(
            vc,
            &keep,
            axis,
            &q,
            0.05,
            if rnd == 0 { None } else { Some(&v) },
        );
        let mut ok: Vec<bool> = tg.iter().map(|x| x.is_some()).collect();
        if ok.iter().any(|&x| x) && !others.is_empty() {
            let j: Vec<usize> = (0..ids.len()).filter(|&k| ok[k]).collect();
            let tj: Vec<DVec3> = j.iter().filter_map(|&k| tg[k]).collect();
            let vj: Vec<DVec3> = j.iter().map(|&k| v[ids[k]]).collect();
            let d_new = geom::min_distances(&tj, others);
            let d_old = geom::min_distances(&vj, others);
            for (m, &k) in j.iter().enumerate() {
                if d_new[m] < CONVEX_CLEAR_MM || (d_new[m] < d_old[m] - 1e-6 && d_new[m] < 1.0) {
                    ok[k] = false;
                }
            }
        }
        let cnt = ok.iter().filter(|&&x| x).count();
        if cnt == 0 {
            break;
        }
        held = held.max(cnt);
        let mut anc = anchor.clone();
        for (k, &i) in ids.iter().enumerate() {
            if ok[k] {
                if let Some(p) = tg[k] {
                    v[i] = p;
                }
                anc[i] = true;
            }
        }
        v = fair::fair(&v, &t, &free, &rows, Some(&anc), Weight::All(CONVEX_W));
        v = fair::untangle(&v, &t, &free, &edges, 10, 10);
    }
    if held > 0 {
        let anc: Vec<bool> = (0..nv).map(|i| anchor[i] || free[i]).collect();
        v = fair::fair(
            &v,
            &t,
            &free,
            &rows,
            Some(&anc),
            Weight::All(CONVEX_SMOOTH_W),
        );
        v = fair::untangle(&v, &t, &free, &edges, 10, 10);
    }
    stats.convex = held;
    // vertices brought to a contact (below): held by the later fairings
    let mut held_c = vec![false; nv];
    // under the margin the tooth goes into the gingiva narrowing: no collar
    let nbin = 72usize;
    let cen: Vec<f64> = (0..nbin)
        .map(|i| (i as f64 + 0.5) / nbin as f64 * two_pi - std::f64::consts::PI)
        .collect();
    let mut rim_h = vec![f64::INFINITY; nbin];
    let mut rim_r = vec![0.0f64; nbin];
    for i in 0..n {
        let b = ((th[i].rem_euclid(two_pi) / two_pi * nbin as f64).floor() as i64)
            .rem_euclid(nbin as i64) as usize;
        rim_h[b] = rim_h[b].min(h[i]);
        rim_r[b] = rim_r[b].max(r_rim[i]);
    }
    let okb: Vec<usize> = (0..nbin).filter(|&i| rim_h[i].is_finite()).collect();
    if okb.len() >= 3 {
        let cx: Vec<f64> = okb.iter().map(|&i| cen[i]).collect();
        let rim_h = geom::interp_periodic(
            &cen,
            &cx,
            &okb.iter().map(|&i| rim_h[i]).collect::<Vec<_>>(),
            two_pi,
        );
        let rim_r = geom::interp_periodic(
            &cen,
            &cx,
            &okb.iter().map(|&i| rim_r[i]).collect::<Vec<_>>(),
            two_pi,
        );
        let h_neck = hmin - neck_mm;
        let mut tapered = 0usize;
        for _ in 0..TAPER_ROUNDS {
            let loc: Vec<(f64, f64, f64)> = v
                .iter()
                .map(|p| {
                    let d = *p - c;
                    (d.dot(axis), d.dot(e1), d.dot(e2))
                })
                .collect();
            let bth: Vec<f64> = loc.iter().map(|x| x.2.atan2(x.1)).collect();
            let hr = geom::interp_periodic(&bth, &cen, &rim_h, two_pi);
            let rr = geom::interp_periodic(&bth, &cen, &rim_r, two_pi);
            let hrmax = hr.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let span = (hrmax - h_neck).max(1e-6);
            let mut over = vec![false; nv];
            let mut caps = vec![0.0; nv];
            for i in 0..nv {
                let (bh, bx, by) = loc[i];
                let br = bx.hypot(by);
                let s = ((hr[i] - bh) / span).clamp(0.0, 1.0);
                let cap = rr[i] * (1.0 - (1.0 - neck_scale) * s);
                let at_gum = hr[i] <= hmin + TAPER_RIM_MM;
                over[i] = free[i] && at_gum && !held_c[i] && bh < hr[i] && br > cap && bh > h_neck;
                caps[i] = cap;
            }
            let cnt = over.iter().filter(|&&x| x).count();
            if cnt == 0 {
                break;
            }
            tapered = tapered.max(cnt);
            let mut anc = anchor.clone();
            for i in 0..nv {
                if over[i] {
                    let (bh, bx, by) = loc[i];
                    let k = caps[i] / bx.hypot(by).max(1e-9);
                    v[i] = c + (bx * k) * e1 + (by * k) * e2 + bh * axis;
                    anc[i] = true;
                }
            }
            v = fair::fair(&v, &t, &free, &rows, Some(&anc), Weight::All(CONVEX_W));
            v = fair::untangle(&v, &t, &free, &edges, 10, 10);
        }
        stats.tapered = tapered;
    }
    // where the scan joined this tooth to a neighbour they touch: the hidden wall up to the contact surface
    let mut filled = 0usize;
    if !patch.contacts.is_empty() {
        for _ in 0..CONTACT_ROUNDS {
            let ids: Vec<usize> = (0..nv).filter(|&i| free[i]).collect();
            let mut nvn = geom::vertex_normals(&v, &t);
            if geom::signed_volume(&v, &t) < 0.0 {
                for x in &mut nvn {
                    *x = -*x;
                }
            }
            let q: Vec<DVec3> = ids.iter().map(|&i| v[i]).collect();
            let nq: Vec<DVec3> = ids.iter().map(|&i| nvn[i]).collect();
            let tg = outline::contact_targets(&q, &nq, &patch.contacts);
            let cnt = tg.iter().filter(|x| x.is_some()).count();
            if cnt == 0 {
                break;
            }
            filled = filled.max(cnt);
            for (k, &i) in ids.iter().enumerate() {
                if let Some(p) = tg[k] {
                    v[i] = p;
                    held_c[i] = true;
                }
            }
            let anc: Vec<bool> = (0..nv).map(|i| anchor[i] || held_c[i]).collect();
            let w: Vec<f64> = held_c
                .iter()
                .map(|&h| if h { CONTACT_W } else { CONVEX_W })
                .collect();
            v = fair::fair(&v, &t, &free, &rows, Some(&anc), Weight::Each(&w));
            v = fair::untangle(&v, &t, &free, &edges, 10, 10);
        }
    }
    stats.contact_filled = filled;
    // a band either side of the margin cut faired as one surface, held lightly where it is
    let rim_pts: Vec<DVec3> = main.iter().map(|&i| v[i as usize]).collect();
    let dband = geom::min_distances(&v, &rim_pts);
    let band: Vec<bool> = (0..nv)
        .map(|i| dband[i] < RIM_BAND_MM && !is_fixed[i])
        .collect();
    if band.iter().any(|&b| b) {
        let nb = neighbours_of(&band, &edges);
        let w: Vec<f64> = held_c
            .iter()
            .map(|&h| if h { CONTACT_W } else { RIM_BAND_W })
            .collect();
        v = fair::fair(&v, &t, &band, &nb, Some(&band), Weight::Each(&w));
        v = fair::untangle(&v, &t, &band, &edges, 10, 10);
    }
    // no spike, fold or crease; the neck ring and the end of the stump stay
    let w: Vec<f64> = held_c
        .iter()
        .map(|&h| if h { CONTACT_W } else { SPIKE_W })
        .collect();
    let (vd, moved, left) = fair::despike(
        &v,
        &t,
        &is_fixed,
        SPIKE_DEG,
        SPIKE_RINGS,
        Weight::Each(&w),
        SPIKE_ROUNDS,
    );
    v = vd;
    stats.despiked = moved;
    stats.sharp_left = left;
    let mut t = t;
    if geom::signed_volume(&v, &t) < 0.0 {
        for f in &mut t {
            f.swap(1, 2);
        }
    }
    let added = v.len() - n0;
    (v, t, added, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_band_closes_two_rings() {
        let p: Vec<u32> = (0..6).collect();
        let q: Vec<u32> = (6..10).collect();
        let u: Vec<f64> = (0..6).map(|i| i as f64 / 6.0).collect();
        let f = ring_band(&p, &q, &u);
        assert_eq!(f.len(), 6 + 4);
        // every edge of the band between the rings is used twice
        let mut count = std::collections::BTreeMap::new();
        for t in &f {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        let inner = count
            .iter()
            .filter(|((a, b), _)| (*a < 6) != (*b < 6))
            .all(|(_, c)| *c == 2);
        assert!(inner);
    }

    #[test]
    fn params_default_from_the_evidence_table() {
        let p = Params::default();
        assert!((p.neck_depth_mm - 1.66).abs() < 1e-12);
        assert_eq!(p.apex_mm, 3.0);
    }
}
