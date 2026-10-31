//! Per-tooth geometry of a segmented scan: local frames, mesio-distal and
//! bucco-lingual widths, crown height, cusp tips, interproximal contacts and
//! the anatomical estimates (CEJ, alveolar crest, apex, centre of
//! resistance).
//!
//! Reference: `ortho_aligner/teeth.py` (`compute_teeth`, `wall_axis`,
//! `symmetry_md_axis`); `reconstruct.py` also calls `wall_axis`.
//!
//! Tooth frame (right-handed, in arch coordinates): `e_ax` is the crown long
//! axis (occlusal), `e_bl` the buccal / labial direction and
//! `e_md = e_bl × e_ax` the mesio-distal axis; `md_sign` is `+1` when `e_md`
//! points mesially.
//!
//! * Long axis: the normal of the best-fit plane of the gingival margin
//!   (cervical line), tilt-limited and averaged with the axis of the side
//!   walls of the crown.
//! * Mesio-distal axis: the line through the two interproximal contacts, the
//!   incisal edge for incisors, otherwise the mirror-symmetry plane of the
//!   crown, otherwise the arch tangent.
//! * CEJ = cusp tip − anatomical crown length; apex = CEJ − root length
//!   (Wheeler / Nelson 2019). Alveolar crest = gingival margin − 2.73 mm
//!   (Gargiulo et al. 1961). Centre of resistance at 42% of the alveolar
//!   height from the crest (Poppe et al. 2002).
//!
//! Surface scans contain no roots: the CEJ, apex, crest and centre of
//! resistance are population estimates, not measurements.
//!
//! The computation works in arch coordinates (origin between the last molars
//! on the occlusal plane, `+Y` towards the incisors, `+Z` occlusal; see
//! [`frame::ArchFrame`]). [`ToothGeom::transformed`] moves a result to case
//! coordinates, and [`tooth_frame`] gives the [`ToothFrame`] stored in the
//! Aligners data.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{Fdi, Pose};
use dc_geom::knn::KnnIndex;
use dca_model::ToothFrame;
use glam::{DMat3, DMat4, DQuat, DVec2, DVec3, Vec3};
use nalgebra::{Matrix3, SymmetricEigen};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::{archform, evidence, frame};

/// Errors of the teeth stage.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TeethError {
    /// The labels do not match the mesh.
    #[error("{0}")]
    Invalid(String),
    /// The scan has no labelled tooth.
    #[error("the scan has no labelled tooth")]
    NoTeeth,
}

/// Which arches the command works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Which {
    /// Every arch that has a scan and labels.
    #[default]
    All,
    /// The upper arch only.
    Upper,
    /// The lower arch only.
    Lower,
}

impl Which {
    /// The arches selected, lower first (the reference arch).
    pub fn arches(self) -> Vec<Arch> {
        match self {
            Which::All => vec![Arch::Lower, Arch::Upper],
            Which::Upper => vec![Arch::Upper],
            Which::Lower => vec![Arch::Lower],
        }
    }
}

/// Parameters of the teeth stage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to work on.
    pub arch: Which,
    /// Distance from the gingival margin to the alveolar crest (mm,
    /// Gargiulo 1961).
    pub margin_to_crest_mm: f64,
    /// Centre of resistance as a fraction of the alveolar height measured
    /// from the crest (Poppe 2002).
    pub cres_ratio: f64,
    /// Largest tilt of the long axis from the vertical (degrees); the
    /// effective limit is smaller for canines and posterior teeth.
    pub max_tilt_deg: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: Which::All,
            margin_to_crest_mm: evidence::param_f64("margin_to_crest").unwrap_or(2.73),
            cres_ratio: evidence::param_f64("cres_ratio").unwrap_or(0.42),
            max_tilt_deg: 35.0,
        }
    }
}

/// Input of the teeth stage: a labelled arch scan in arch coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Arch of the scan.
    pub arch: Arch,
    /// Vertices in arch coordinates (mm, `+Z` occlusal).
    pub vertices: &'a [DVec3],
    /// Triangles (indices into `vertices`).
    pub tris: &'a [[u32; 3]],
    /// FDI number of each vertex, `0` for the gingiva.
    pub labels: &'a [u16],
}

/// Geometry of one tooth, in the coordinates of the input.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToothGeom {
    /// Tooth number.
    pub fdi: Fdi,
    /// Number of vertices labelled with this tooth.
    pub vertex_count: usize,
    /// Number of triangles whose three vertices carry this label.
    pub face_count: usize,
    /// Area-weighted centroid of the crown.
    pub centroid: DVec3,
    /// Mesio-distal axis.
    pub e_md: DVec3,
    /// Bucco-lingual axis (outward).
    pub e_bl: DVec3,
    /// Long axis (occlusal).
    pub e_ax: DVec3,
    /// `+1` when `e_md` points mesially, `-1` when distally.
    pub md_sign: i8,
    /// Reference point of the crown: the centroid moved to the middle of the
    /// mesio-distal extent. Origin of the stored [`ToothFrame`].
    pub reference: DVec3,
    /// Mesio-distal width (mm).
    pub width_md: f64,
    /// Bucco-lingual width (mm).
    pub width_bl: f64,
    /// Crown height above the mean of the margin (mm).
    pub height: f64,
    /// Occlusal height reference on the curve of Spee: the incisal edge or
    /// cusp tip (buccal cusps for posterior teeth), `z` coordinate.
    pub occ_z: f64,
    /// Highest point of the crown along the long axis (cusp tip).
    pub tip: DVec3,
    /// Vertices of the gingival margin (shared with the gingiva).
    #[serde(skip)]
    pub margin: Vec<DVec3>,
    /// Mean of the margin vertices.
    pub margin_mean: DVec3,
    /// Cemento-enamel junction (estimate), on the axis through the margin.
    pub cej: DVec3,
    /// Alveolar crest (estimate).
    pub crest: DVec3,
    /// Apex of the root (estimate).
    pub apex: DVec3,
    /// Centre of resistance (estimate).
    pub cres: DVec3,
    /// Mesial interproximal contact point.
    pub contact_m: DVec3,
    /// Distal interproximal contact point.
    pub contact_d: DVec3,
    /// Rotation about the long axis relative to the arch tangent (degrees).
    pub rot_dev_deg: f64,
    /// Lowest point of the margin on the buccal side (`z`).
    pub zen_b: f64,
    /// Lowest point of the margin on the lingual side (`z`).
    pub zen_l: f64,
    /// Mesial direction of the arch at the tooth (horizontal).
    pub mesial_dir: DVec3,
    /// Outward direction of the arch at the tooth (horizontal).
    pub out_dir: DVec3,
    /// Tilt of the long axis from the vertical (degrees).
    pub axis_tilt_deg: f64,
    /// Elongation of the most occlusal 20% of the crown (an incisal edge is
    /// high).
    pub top_elong: f64,
    /// Spread (mm) of the most occlusal 10% of the crown (a multi-cusp tooth
    /// is high).
    pub top_spread: f64,
}

impl ToothGeom {
    /// Rest frame: the axes as columns, origin at the centre of resistance.
    pub fn rest_frame(&self) -> DMat4 {
        frame::make_frame(self.e_md, self.e_bl, self.e_ax, self.cres)
    }

    /// The same geometry moved by a rigid transform (arch to case
    /// coordinates, for instance).
    pub fn transformed(&self, m: &DMat4) -> Self {
        let p = |v: DVec3| m.transform_point3(v);
        let d = |v: DVec3| m.transform_vector3(v);
        Self {
            centroid: p(self.centroid),
            e_md: d(self.e_md),
            e_bl: d(self.e_bl),
            e_ax: d(self.e_ax),
            reference: p(self.reference),
            tip: p(self.tip),
            margin: self.margin.iter().map(|&v| p(v)).collect(),
            margin_mean: p(self.margin_mean),
            cej: p(self.cej),
            crest: p(self.crest),
            apex: p(self.apex),
            cres: p(self.cres),
            contact_m: p(self.contact_m),
            contact_d: p(self.contact_d),
            mesial_dir: d(self.mesial_dir),
            out_dir: d(self.out_dir),
            ..self.clone()
        }
    }
}

/// Result of the teeth stage for one arch.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The teeth found, along the arch from the patient's right distal.
    pub sequence: Vec<Fdi>,
    /// Geometry of each tooth, in the coordinates of the input.
    pub teeth: BTreeMap<Fdi, ToothGeom>,
}

/// The [`ToothFrame`] stored in the Aligners data, from a tooth in case
/// coordinates: the pose has the tooth axes (x mesio-distal, y bucco-lingual,
/// z long axis) and its origin at the crown reference point; the centre of
/// resistance is given in that frame.
pub fn tooth_frame(tooth: &ToothGeom) -> ToothFrame {
    let q = DQuat::from_mat3(&DMat3::from_cols(tooth.e_md, tooth.e_bl, tooth.e_ax)).normalize();
    let pose = Pose::new(tooth.reference, q);
    ToothFrame {
        pose,
        center_of_resistance: pose.inverse().apply_point(tooth.cres),
    }
}

/// Rest pose of the plugin (`ToothGeom.frame()`: origin at the centre of
/// resistance) from a stored [`ToothFrame`].
pub fn rest_pose(frame: &ToothFrame) -> Pose {
    Pose::new(
        frame.pose.apply_point(frame.center_of_resistance),
        frame.pose.q,
    )
}

// ---- numeric helpers ------------------------------------------------------------------------------------

fn normalize(v: DVec3) -> DVec3 {
    let n = v.length();
    if n == 0.0 { v } else { v / n }
}

fn mean(p: &[DVec3]) -> DVec3 {
    p.iter().copied().sum::<DVec3>() / p.len() as f64
}

/// `x` rounded away from zero to `±1`; `1` for zero (`np.sign(x) or 1.0`).
fn sign_or_one(x: f64) -> f64 {
    if x == 0.0 { 1.0 } else { x.signum() }
}

fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn angle_deg(c: f64) -> f64 {
    c.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Percentile `q` (0-100) of `values` with linear interpolation, as
/// `numpy.percentile`.
pub fn percentile(values: &[f64], q: f64) -> f64 {
    let mut s = values.to_vec();
    s.sort_by(f64::total_cmp);
    percentile_sorted(&s, q)
}

fn percentile_sorted(s: &[f64], q: f64) -> f64 {
    let n = s.len();
    let virt = (n - 1) as f64 * (q / 100.0);
    let lo = virt.floor();
    let t = virt - lo;
    let i = (lo as usize).min(n - 1);
    let j = (i + 1).min(n - 1);
    let (a, b) = (s[i], s[j]);
    let diff = b - a;
    if t >= 0.5 {
        b - diff * (1.0 - t)
    } else {
        a + diff * t
    }
}

/// At most `n` points, evenly spread in their order (`mu.subsample`: the points
/// at `numpy.linspace(0, len - 1, n).astype(int)`).
pub fn subsample(p: &[DVec3], n: usize) -> Vec<DVec3> {
    if p.len() <= n {
        return p.to_vec();
    }
    // numpy.linspace(0, len - 1, n).astype(int)
    let step = (p.len() - 1) as f64 / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let x = if i == n - 1 {
                (p.len() - 1) as f64
            } else {
                i as f64 * step
            };
            p[x as usize]
        })
        .collect()
}

/// Eigenvector of the smallest eigenvalue of a symmetric 3x3 matrix.
fn smallest_eigenvector(m: Matrix3<f64>) -> DVec3 {
    let eig = SymmetricEigen::new(m);
    let k = (0..3)
        .min_by(|&a, &b| eig.eigenvalues[a].total_cmp(&eig.eigenvalues[b]))
        .unwrap_or(0);
    let c = eig.eigenvectors.column(k);
    DVec3::new(c[0], c[1], c[2])
}

fn z_up(v: DVec3) -> DVec3 {
    if v.z >= 0.0 { v } else { -v }
}

/// `n` tilted at most `cap_deg` from the vertical, in its own direction.
fn capped(n: DVec3, cap_deg: f64) -> DVec3 {
    let n = normalize(n);
    if n.z.clamp(-1.0, 1.0).acos().to_degrees() <= cap_deg {
        return n;
    }
    let h = DVec3::new(n.x, n.y, 0.0);
    if h.length() < 1e-9 {
        return DVec3::Z;
    }
    let a = cap_deg.to_radians();
    normalize(a.cos() * DVec3::Z + a.sin() * normalize(h))
}

/// Long axis from the side walls of the crown (faces `fidx`): the direction
/// most perpendicular to the normals of the faces that are neither on the
/// occlusal / incisal top nor at the very margin (buccal, lingual, mesial and
/// distal walls run along the axis). It does not depend on where the margin
/// line was drawn, so labels spilled on the gingiva barely move it. `start`
/// is the first estimate (the vertical by default).
pub fn wall_axis(
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    fidx: &[usize],
    start: Option<DVec3>,
) -> DVec3 {
    wall_axis_with(vertices, tris, fidx, start, 5.0, 85.0, 0.5, 4)
}

#[allow(clippy::too_many_arguments)]
fn wall_axis_with(
    v: &[DVec3],
    tris: &[[u32; 3]],
    fidx: &[usize],
    start: Option<DVec3>,
    low: f64,
    high: f64,
    wall: f64,
    iters: usize,
) -> DVec3 {
    let mut ax = start.map_or(DVec3::Z, normalize);
    if fidx.len() < 30 {
        return ax;
    }
    let mut nrm = Vec::with_capacity(fidx.len());
    let mut area = Vec::with_capacity(fidx.len());
    let mut cen = Vec::with_capacity(fidx.len());
    for &f in fidx {
        let [a, b, c] = tris[f].map(|i| v[i as usize]);
        let cr = (b - a).cross(c - a);
        let area2 = cr.length();
        nrm.push(if area2 > 0.0 { cr / area2 } else { cr });
        area.push(0.5 * area2);
        cen.push((a + b + c) / 3.0);
    }
    for _ in 0..iters {
        let h: Vec<f64> = cen.iter().map(|c| c.dot(ax)).collect();
        let mut sorted = h.clone();
        sorted.sort_by(f64::total_cmp);
        let lo = percentile_sorted(&sorted, low);
        let hi = percentile_sorted(&sorted, high);
        let mut m = Matrix3::<f64>::zeros();
        let mut count = 0usize;
        for i in 0..fidx.len() {
            if nrm[i].dot(ax).abs() < wall && h[i] > lo && h[i] < hi {
                let w = nrm[i] * area[i].sqrt();
                let w = [w.x, w.y, w.z];
                for r in 0..3 {
                    for c in 0..3 {
                        m[(r, c)] += w[r] * w[c];
                    }
                }
                count += 1;
            }
        }
        if count < 30 {
            break;
        }
        ax = z_up(smallest_eigenvector(m));
    }
    normalize(ax)
}

/// Mesio-distal axis as the normal of the crown's best mirror-symmetry plane
/// that contains the long axis (crowns are approximately symmetric about
/// their bucco-lingual plane). Searches `±max_angle` degrees around `guess`
/// with a small penalty per degree of deviation (prior: tooth aligned with
/// the arch). If the best plane improves the guess by less than
/// `keep_ratio`, the guess is kept. Returns the axis and the score (mm).
///
/// The nearest-point distances come from a single-precision k-d tree, like
/// the plugin's `mathutils.kdtree`.
pub fn symmetry_md_axis(
    p: &[DVec3],
    e_ax: DVec3,
    guess: DVec3,
    max_angle: f64,
    n_samples: usize,
    penalty: f64,
    keep_ratio: f64,
) -> (DVec3, f64) {
    if p.len() < 30 {
        return (guess, f64::INFINITY);
    }
    let pts: Vec<Vec3> = p.iter().map(|q| q.as_vec3()).collect();
    let Ok(kd) = KnnIndex::build(&pts) else {
        return (guess, f64::INFINITY);
    };
    let s = subsample(p, n_samples);
    let c = mean(p);
    let b = normalize(e_ax.cross(guess));
    let lim = max_angle.to_radians();

    let raw = |theta: f64, off: f64| -> f64 {
        let n = theta.cos() * guess + theta.sin() * b;
        let mut tot = 0.0;
        for &q in &s {
            let d = (q - c).dot(n) - off;
            let r = q - 2.0 * d * n;
            tot += kd
                .nearest(r.as_vec3())
                .map_or(f64::INFINITY, |nb| f64::from(nb.distance));
        }
        tot / s.len() as f64
    };
    let score = |theta: f64, off: f64| -> f64 {
        if theta.abs() > lim {
            return f64::INFINITY;
        }
        raw(theta, off) + penalty * theta.to_degrees().abs()
    };

    let mut best = (f64::INFINITY, 0.0, 0.0);
    // numpy.arange(-max_angle, max_angle + 0.1, 3.0)
    let count = ((2.0 * max_angle + 0.1) / 3.0).ceil() as usize;
    for k in 0..count {
        let th = (-max_angle + 3.0 * k as f64).to_radians();
        for off in [-0.4, 0.0, 0.4] {
            let sc = score(th, off);
            if sc < best.0 {
                best = (sc, th, off);
            }
        }
    }
    let (_, mut th0, mut o0) = best;
    for (step_t, step_o) in [(1.0f64.to_radians(), 0.1), (0.3f64.to_radians(), 0.03)] {
        for i in -3..=3 {
            let th = th0 + step_t * f64::from(i);
            for j in -3..=3 {
                let off = o0 + step_o * f64::from(j);
                let sc = score(th, off);
                if sc < best.0 {
                    best = (sc, th, off);
                }
            }
        }
        (_, th0, o0) = best;
    }
    if keep_ratio > 0.0 {
        let base = [-0.4, -0.2, 0.0, 0.2, 0.4]
            .into_iter()
            .map(|o| raw(0.0, o))
            .fold(f64::INFINITY, f64::min);
        if raw(th0, o0) > (1.0 - keep_ratio) * base {
            return (guess, base);
        }
    }
    let n = th0.cos() * guess + th0.sin() * b;
    (normalize(n), best.0)
}

/// Vertices on the border between two labels: for each `(own, other)` pair
/// the sorted vertices of `own` that have an edge to a vertex of `other`.
fn boundary_vertices(tris: &[[u32; 3]], labels: &[u16]) -> BTreeMap<(u16, u16), Vec<u32>> {
    let mut sets: BTreeMap<(u16, u16), BTreeSet<u32>> = BTreeMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let (la, lb) = (labels[a as usize], labels[b as usize]);
            if la != lb {
                sets.entry((la, lb)).or_default().insert(a);
                sets.entry((lb, la)).or_default().insert(b);
            }
        }
    }
    sets.into_iter()
        .map(|(k, s)| (k, s.into_iter().collect()))
        .collect()
}

/// Geometry of every labelled tooth of an arch scan: frames, widths,
/// landmarks and anatomical estimates. `compute_teeth` of the plugin.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, TeethError> {
    let (v, t, labels) = (input.vertices, input.tris, input.labels);
    if labels.len() != v.len() {
        return Err(TeethError::Invalid(format!(
            "{} labels for {} vertices",
            labels.len(),
            v.len()
        )));
    }
    if t.iter().flatten().any(|&i| i as usize >= v.len()) {
        return Err(TeethError::Invalid(
            "a triangle refers to a missing vertex".to_owned(),
        ));
    }
    let boundary = boundary_vertices(t, labels);

    let present: BTreeSet<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
    let fdis: Vec<Fdi> = present
        .iter()
        .filter_map(|&l| u8::try_from(l).ok().and_then(|n| Fdi::new(n).ok()))
        .collect();
    let seq = anatomy::arch_sequence(&fdis, input.arch);
    if seq.is_empty() {
        return Err(TeethError::NoTeeth);
    }

    // full faces of each tooth, and its area-weighted centroid
    let mut faces: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for (i, tri) in t.iter().enumerate() {
        let l = labels[tri[0] as usize];
        if l != 0 && l == labels[tri[1] as usize] && l == labels[tri[2] as usize] {
            faces.entry(l).or_default().push(i);
        }
    }
    let mut vert_of: BTreeMap<u16, Vec<u32>> = BTreeMap::new();
    for (i, &l) in labels.iter().enumerate() {
        if l != 0 {
            vert_of.entry(l).or_default().push(i as u32);
        }
    }
    let label_of = |f: Fdi| u16::from(f.get());
    let no_faces: Vec<usize> = Vec::new();
    let mut cents: BTreeMap<Fdi, DVec3> = BTreeMap::new();
    for &f in &seq {
        let fidx = faces.get(&label_of(f)).unwrap_or(&no_faces);
        if fidx.is_empty() {
            let pts: Vec<DVec3> = vert_of[&label_of(f)]
                .iter()
                .map(|&i| v[i as usize])
                .collect();
            cents.insert(f, mean(&pts));
            continue;
        }
        let (mut sum, mut total) = (DVec3::ZERO, 0.0);
        for &fi in fidx {
            let [a, b, c] = t[fi].map(|i| v[i as usize]);
            let area = 0.5 * (b - a).cross(c - a).length();
            sum += (a + b + c) / 3.0 * area;
            total += area;
        }
        cents.insert(f, sum / total.max(1e-12));
    }

    let xy: Vec<DVec2> = seq.iter().map(|f| cents[f].truncate()).collect();
    let abc = if seq.len() >= 3 {
        archform::fit_quartic(&xy, None)
    } else {
        [0.0, -0.02, 0.0]
    };
    let right: Vec<f64> = seq
        .iter()
        .filter(|f| anatomy::is_right(**f))
        .map(|f| cents[f].x)
        .collect();
    let s_r = if right.is_empty() {
        1.0
    } else {
        sign_or_one(right.iter().sum::<f64>() / right.len() as f64)
    };

    let mut teeth = BTreeMap::new();
    for &f in &seq {
        let fidx = faces.get(&label_of(f)).unwrap_or(&no_faces);
        let vidx = &vert_of[&label_of(f)];
        let tooth = tooth_geometry(
            v,
            t,
            f,
            vidx,
            fidx,
            cents[&f],
            &Context {
                seq: &seq,
                boundary: &boundary,
                abc,
                s_r,
                params,
            },
        );
        teeth.insert(f, tooth);
    }
    Ok(Output {
        sequence: seq,
        teeth,
    })
}

/// What `tooth_geometry` needs to know about the arch.
struct Context<'a> {
    seq: &'a [Fdi],
    boundary: &'a BTreeMap<(u16, u16), Vec<u32>>,
    abc: [f64; 3],
    s_r: f64,
    params: &'a Params,
}

fn tooth_geometry(
    v: &[DVec3],
    t: &[[u32; 3]],
    f: Fdi,
    vidx: &[u32],
    fidx: &[usize],
    cent: DVec3,
    ctx: &Context<'_>,
) -> ToothGeom {
    let p: Vec<DVec3> = vidx.iter().map(|&i| v[i as usize]).collect();
    let lf = u16::from(f.get());
    let x = cent.x;
    let slope = 2.0 * ctx.abc[1] * x + 4.0 * ctx.abc[2] * x.powi(3);
    let t2 = DVec2::new(1.0, slope).normalize();
    let m2 = if anatomy::is_right(f) {
        -ctx.s_r * t2
    } else {
        ctx.s_r * t2
    };
    let out2 = DVec2::new(-t2.y, t2.x);
    let mesial3 = DVec3::new(m2.x, m2.y, 0.0);
    let out3 = DVec3::new(out2.x, out2.y, 0.0);

    // --- long axis: cervical line and side walls of the crown --------------
    // The plane of the cervical line alone is noisy (scalloped margin, margin
    // made of two arcs on crowded teeth, labels spilled on the gingiva): its
    // tilt is limited to typical values and averaged with the axis of the
    // side walls of the crown, which does not depend on the margin.
    let margin: Vec<DVec3> = match ctx.boundary.get(&(lf, 0)) {
        Some(idx) if !idx.is_empty() => idx.iter().map(|&i| v[i as usize]).collect(),
        _ => {
            let zs: Vec<f64> = p.iter().map(|q| q.z).collect();
            let lim = percentile(&zs, 10.0);
            p.iter().copied().filter(|q| q.z < lim).collect()
        }
    };
    let cap = ctx.params.max_tilt_deg.min(if anatomy::is_incisor(f) {
        28.0
    } else if anatomy::index(f) == 3 {
        20.0
    } else {
        15.0
    });
    let mut n = DVec3::Z;
    if margin.len() >= 8
        && let Some(pc) = frame::pca(&margin)
    {
        n = capped(z_up(pc.axes[2]), cap);
    }
    let e_ax = capped(n + wall_axis(v, t, fidx, None), cap);
    let tilt = e_ax.z.clamp(-1.0, 1.0).acos().to_degrees();

    // --- own mesio-distal direction -----------------------------------------
    let perp = |w: DVec3| normalize(w - w.dot(e_ax) * e_ax);
    let m_perp = perp(mesial3);
    let h: Vec<f64> = p.iter().map(|q| (*q - cent).dot(e_ax)).collect();
    let margin_h0 = if margin.is_empty() {
        h.iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        margin.iter().map(|q| (*q - cent).dot(e_ax)).sum::<f64>() / margin.len() as f64
    };
    // interproximal contact areas with the neighbours (from the segmentation):
    // the line through the two contacts is the clinical reference of the
    // alignment (contact point displacement, Little 1975)
    let contact = |nb: Option<Fdi>| -> Option<DVec3> {
        let sel = ctx.boundary.get(&(lf, u16::from(nb?.get())))?;
        (sel.len() >= 3).then(|| mean(&sel.iter().map(|&i| v[i as usize]).collect::<Vec<_>>()))
    };
    let cm = contact(anatomy::mesial_neighbor(ctx.seq, f));
    let cd = contact(anatomy::distal_neighbor(ctx.seq, f));
    let md_typ = anatomy::typical_diameters(f).0;
    let mut edge = None;
    if anatomy::is_incisor(f) && p.len() > 30 {
        let lim = percentile(&h, 80.0);
        let tp: Vec<DVec3> = p
            .iter()
            .zip(&h)
            .filter(|(_, hh)| **hh >= lim)
            .map(|(q, _)| *q - q.dot(e_ax) * e_ax)
            .collect();
        if let Some(pc) = frame::pca(&tp)
            && pc.eigenvalues[0] > 2.0 * pc.eigenvalues[1].max(1e-9)
        {
            edge = Some(pc.axes[0]);
        }
    }
    let mut cand: Option<DVec3> = None;
    if let (Some(cm), Some(cd)) = (cm, cd)
        && (cm - cd).length() > 0.45 * md_typ
    {
        let mut c = cm - cd;
        if let Some(edge) = edge {
            // incisors: the incisal edge is a good MD reference as well; the
            // contacts win when the two disagree (worn / notched edges)
            let ce = perp(edge) * sign_or_one(perp(edge).dot(perp(c)));
            if angle_deg(ce.dot(perp(c))) < 15.0 {
                c = perp(c) + ce;
            }
        }
        cand = Some(c);
    } else if let Some(edge) = edge {
        cand = Some(edge);
    }
    if cand.is_none() {
        let upper_pts: Vec<DVec3> = p
            .iter()
            .zip(&h)
            .filter(|(_, hh)| **hh > margin_h0 + 1.0)
            .map(|(q, _)| *q)
            .collect();
        let mut guess = m_perp;
        if let Some(cm) = cm {
            let g = perp(cm - cent);
            if g.dot(m_perp) > 0.0 {
                guess = g;
            }
        } else if let Some(cd) = cd {
            let g = perp(cent - cd);
            if g.dot(m_perp) > 0.0 {
                guess = g;
            }
        }
        if upper_pts.len() >= 60 {
            // molars / teeth without both contacts: symmetry of the crown with
            // a prior towards the contact or arch direction
            cand = Some(symmetry_md_axis(&upper_pts, e_ax, guess, 30.0, 250, 0.001, 0.05).0);
        }
    }
    let mut cand = perp(cand.unwrap_or(m_perp));
    if cand.dot(m_perp) < 0.0 {
        cand = -cand;
    }
    if angle_deg(cand.dot(m_perp)) > 60.0 {
        cand = m_perp;
    }
    let mut e_bl = normalize(e_ax.cross(cand));
    let (e_md, md_sign) = if e_bl.dot(out3) >= 0.0 {
        (cand, 1i8)
    } else {
        let e_md = -cand;
        e_bl = normalize(e_ax.cross(e_md));
        (e_md, -1i8)
    };

    // --- measurements --------------------------------------------------------
    let pmd: Vec<f64> = p.iter().map(|q| (*q - cent).dot(e_md)).collect();
    let pbl: Vec<f64> = p.iter().map(|q| (*q - cent).dot(e_bl)).collect();
    let pax: Vec<f64> = p.iter().map(|q| (*q - cent).dot(e_ax)).collect();
    let min_max = |a: &[f64]| {
        a.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| {
                (lo.min(x), hi.max(x))
            })
    };
    let (md_lo, md_hi) = min_max(&pmd);
    let (bl_lo, bl_hi) = min_max(&pbl);
    let (ax_lo, ax_hi) = min_max(&pax);
    let width_md = md_hi - md_lo;
    let width_bl = bl_hi - bl_lo;
    let reference = cent + e_md * 0.5 * (md_hi + md_lo);
    let top_i = pax
        .iter()
        .enumerate()
        .fold(0, |best, (i, &x)| if x > pax[best] { i } else { best });
    let tip = p[top_i];
    let margin_h = if margin.is_empty() {
        ax_lo
    } else {
        margin.iter().map(|q| (*q - cent).dot(e_ax)).sum::<f64>() / margin.len() as f64
    };
    let height = ax_hi - margin_h;
    // occlusal reference on the curve of Spee: incisal edge / cusp tip for
    // anterior teeth, buccal cusp tips for posterior teeth
    let zmax = |sel: &dyn Fn(usize) -> bool| {
        p.iter()
            .enumerate()
            .filter(|(i, _)| sel(*i))
            .map(|(_, q)| q.z)
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let occ_z = if anatomy::index(f) <= 3 || !pbl.iter().any(|&b| b > 0.0) {
        zmax(&|_| true)
    } else {
        zmax(&|i| pbl[i] > 0.0)
    };
    let margin_mean = if margin.is_empty() {
        cent - e_ax * (height * 0.5)
    } else {
        mean(&margin)
    };

    let pm: Vec<f64> = pmd.iter().map(|&x| x * f64::from(md_sign)).collect();
    let upper: Vec<bool> = pax.iter().map(|&a| a >= margin_h + 0.3 * height).collect();
    let n_upper = upper.iter().filter(|&&u| u).count();
    let pick: Vec<usize> = if n_upper > 10 {
        (0..p.len()).filter(|&i| upper[i]).collect()
    } else {
        (0..p.len()).collect()
    };
    let (pm_lo, pm_hi) = min_max(&pick.iter().map(|&i| pm[i]).collect::<Vec<_>>());
    let contact_m = mean(
        &pick
            .iter()
            .filter(|&&i| pm[i] >= pm_hi - 0.3)
            .map(|&i| p[i])
            .collect::<Vec<_>>(),
    );
    let contact_d = mean(
        &pick
            .iter()
            .filter(|&&i| pm[i] <= pm_lo + 0.3)
            .map(|&i| p[i])
            .collect::<Vec<_>>(),
    );

    let (crown_len, root_len) = anatomy::crown_root_length(f);
    let tip_h = ax_hi;
    let cej_h = tip_h - crown_len;
    let crest_h = (margin_h - ctx.params.margin_to_crest_mm).min(cej_h - 0.5);
    let apex_h = cej_h - root_len;
    let alv = (crest_h - apex_h).max(0.3 * root_len);
    let cres_h = crest_h - ctx.params.cres_ratio * alv;
    let on_axis = |hh: f64| margin_mean + (hh - margin_h) * e_ax;

    let own_m2 = {
        let w = (e_md * f64::from(md_sign)).truncate();
        let n = w.length();
        if n == 0.0 { w } else { w / n }
    };
    let phi = cross2(m2, own_m2).atan2(m2.dot(own_m2));
    let s_out = sign_or_one(cross2(m2, out2));
    let rot_dev_deg = phi.to_degrees() * s_out;

    // occlusal shape descriptors (used for the independent tooth-type check)
    let (mut top_elong, mut top_spread) = (0.0, 0.0);
    if p.len() > 30 {
        let top_of = |q: f64| -> Vec<DVec3> {
            let lim = percentile(&pax, q);
            p.iter()
                .zip(&pax)
                .filter(|(_, a)| **a >= lim)
                .map(|(pt, _)| *pt - pt.dot(e_ax) * e_ax)
                .collect()
        };
        if let Some(pc) = frame::pca(&top_of(80.0)) {
            top_elong = (pc.eigenvalues[0] / pc.eigenvalues[1].max(1e-9)).sqrt();
        }
        let t10 = top_of(90.0);
        let c10 = mean(&t10);
        let var: DVec3 =
            t10.iter().map(|q| (*q - c10) * (*q - c10)).sum::<DVec3>() / t10.len() as f64;
        top_spread = (var.x + var.y + var.z).sqrt();
    }

    let (zen_b, zen_l) = if margin.is_empty() {
        let z = p.iter().map(|q| q.z).fold(f64::INFINITY, f64::min);
        (z, z)
    } else {
        let all = margin.iter().map(|q| q.z).fold(f64::INFINITY, f64::min);
        let side = |buccal: bool| {
            margin
                .iter()
                .filter(|q| ((**q - cent).dot(e_bl) > 0.0) == buccal)
                .map(|q| q.z)
                .fold(f64::INFINITY, f64::min)
        };
        let (b, l) = (side(true), side(false));
        (
            if b.is_finite() { b } else { all },
            if l.is_finite() { l } else { all },
        )
    };

    ToothGeom {
        fdi: f,
        vertex_count: p.len(),
        face_count: fidx.len(),
        centroid: cent,
        e_md,
        e_bl,
        e_ax,
        md_sign,
        reference,
        width_md,
        width_bl,
        height,
        occ_z,
        tip,
        margin,
        margin_mean,
        cej: on_axis(cej_h),
        crest: on_axis(crest_h),
        apex: on_axis(apex_h),
        cres: on_axis(cres_h),
        contact_m,
        contact_d,
        rot_dev_deg,
        zen_b,
        zen_l,
        mesial_dir: mesial3,
        out_dir: out3,
        axis_tilt_deg: tilt,
        top_elong,
        top_spread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{self, Severity};

    const PER_TOOTH: usize = 23 * 48 + 2;

    fn model(arch: Arch, severity: Severity, seed: u64) -> demo::Demo {
        demo::run(&demo::Params {
            arch,
            severity,
            seed,
            third_molars: false,
        })
    }

    /// The demo as a connected labelled surface: the two lowest rings of every
    /// crown (the root stub) count as gingiva, so the cervical ring is a real
    /// margin; the crown vertices that touch a neighbour's crown (within
    /// 0.4 mm) are joined to it by a degenerate triangle, which makes the
    /// interproximal contacts. Returns vertices, triangles, labels.
    fn connected(m: &demo::Demo) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<u16>) {
        let v: Vec<DVec3> = m.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let mut labels = m.mesh.vertex_labels.clone().unwrap();
        let mut tris = m.mesh.tris.clone();
        let teeth: Vec<Fdi> = m.teeth.keys().copied().collect();
        // parts are consecutive runs of PER_TOOTH vertices
        for k in 0..teeth.len() {
            let s = k * PER_TOOTH;
            labels[s..s + 2 * 48].fill(0);
            labels[s + PER_TOOTH - 1] = 0;
        }
        let seq = anatomy::arch_sequence(&teeth, m.arch);
        for &f in &seq {
            let Some(nb) = anatomy::mesial_neighbor(&seq, f) else {
                continue;
            };
            let of = |g: Fdi| -> Vec<usize> {
                (0..v.len())
                    .filter(|&i| labels[i] == u16::from(g.get()))
                    .collect()
            };
            let (a, b) = (of(f), of(nb));
            for &i in &a {
                let near = b
                    .iter()
                    .map(|&j| ((v[i] - v[j]).length(), j))
                    .min_by(|x, y| x.0.total_cmp(&y.0))
                    .unwrap();
                if near.0 < 0.4 {
                    tris.push([i as u32, i as u32, near.1 as u32]);
                }
            }
        }
        (v, tris, labels)
    }

    fn compute(arch: Arch, v: &[DVec3], t: &[[u32; 3]], l: &[u16]) -> Output {
        run(
            &Input {
                arch,
                vertices: v,
                tris: t,
                labels: l,
            },
            &Params::default(),
        )
        .unwrap()
    }

    /// Local frame of the demo tooth in the arch (columns: mesio-distal,
    /// bucco-lingual, long axis) before the placement rotation `rot`, `tip`,
    /// `torque`: only the long axis is returned.
    fn truth_axis(tr: &demo::ToothTruth) -> DVec3 {
        DMat3::from_rotation_z(tr.rotation_deg.to_radians())
            * DMat3::from_rotation_y(tr.tip_deg.to_radians())
            * DMat3::from_rotation_x(tr.torque_deg.to_radians())
            * DVec3::Z
    }

    /// Extent of the demo tooth in its own mesio-distal and bucco-lingual
    /// directions: the true widths.
    fn true_widths(f: Fdi) -> (f64, f64) {
        let (vt, _, _) = demo::tooth_mesh(f, 48);
        let ptp = |g: &dyn Fn(&DVec3) -> f64| {
            let (lo, hi) = vt.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| {
                (a.0.min(g(p)), a.1.max(g(p)))
            });
            hi - lo
        };
        (ptp(&|p| p.x), ptp(&|p| p.y))
    }

    fn check_frame(g: &ToothGeom) {
        let what = g.fdi.to_string();
        for e in [g.e_md, g.e_bl, g.e_ax] {
            assert!((e.length() - 1.0).abs() < 1e-9, "{what}: {e:?}");
        }
        assert!(g.e_md.dot(g.e_bl).abs() < 1e-9, "{what}: md . bl");
        assert!(g.e_md.dot(g.e_ax).abs() < 1e-9, "{what}: md . ax");
        assert!(g.e_bl.dot(g.e_ax).abs() < 1e-9, "{what}: bl . ax");
        // right-handed: md x bl = ax
        assert!(
            (g.e_md.cross(g.e_bl) - g.e_ax).length() < 1e-9,
            "{what}: handedness"
        );
        // occlusal side up, within the tilt limit; buccal side outwards
        assert!(
            g.e_ax.z > 35f64.to_radians().cos() - 1e-9,
            "{what}: axis {:?}",
            g.e_ax
        );
        assert!(g.e_bl.dot(g.out_dir) >= 0.0, "{what}: bl is not outward");
        // md_sign says whether e_md points mesially
        assert_eq!(
            g.e_md.dot(g.mesial_dir) > 0.0,
            g.md_sign == 1,
            "{what}: md_sign"
        );
        assert!(
            g.width_md > 3.0 && g.width_bl > 3.0 && g.height > 3.0,
            "{what}"
        );
        assert!(
            g.tip.z >= g.margin_mean.z,
            "{what}: the tip is above the margin"
        );
        // landmarks along the long axis: apex < crest < margin < CEJ < tip
        let h = |p: DVec3| (p - g.margin_mean).dot(g.e_ax);
        assert!(h(g.apex) < h(g.cres) && h(g.cres) < h(g.crest), "{what}");
        assert!(h(g.crest) < 0.0 && h(g.cej) < h(g.tip), "{what}");
    }

    #[test]
    fn demo_arches_give_orthonormal_oriented_frames_and_true_widths() {
        for (arch, severity, md_tol) in [
            (Arch::Lower, Severity::Mild, 0.5),
            (Arch::Lower, Severity::Moderate, 0.6),
            (Arch::Upper, Severity::Moderate, 1.2),
            (Arch::Upper, Severity::Severe, 1.6),
        ] {
            let m = model(arch, severity, 7);
            let (v, t, l) = connected(&m);
            let out = compute(arch, &v, &t, &l);
            assert_eq!(out.sequence.len(), 14);
            let mut axis_err = Vec::new();
            for (f, g) in &out.teeth {
                check_frame(g);
                let (md, bl) = true_widths(*f);
                assert!(
                    (g.width_md - md).abs() <= md_tol,
                    "{arch:?} {severity:?} {f}: md {} against {md}",
                    g.width_md
                );
                assert!(
                    (g.width_bl - bl).abs() <= 0.8,
                    "{arch:?} {severity:?} {f}: bl {} against {bl}",
                    g.width_bl
                );
                axis_err.push(angle_deg(g.e_ax.dot(truth_axis(&m.teeth[f]))));
                // the margin is the cervical ring
                assert_eq!(g.margin.len(), 48, "{f}");
            }
            axis_err.sort_by(f64::total_cmp);
            assert!(axis_err[13] < 20.0, "{arch:?} {severity:?}: {axis_err:?}");
            assert!(
                axis_err[7] < 8.0,
                "{arch:?} {severity:?}: median {axis_err:?}"
            );
        }
    }

    #[test]
    fn separate_parts_use_the_lowest_crown_as_margin() {
        // the demo as it is: crowns and gum are disconnected, so there is no border
        for arch in [Arch::Lower, Arch::Upper] {
            let m = model(arch, Severity::Mild, 7);
            let v: Vec<DVec3> = m.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
            let l = m.mesh.vertex_labels.clone().unwrap();
            let out = compute(arch, &v, &m.mesh.tris, &l);
            assert_eq!(out.teeth.len(), 14);
            for (f, g) in &out.teeth {
                check_frame(g);
                // the 10% of the crown below the 10th percentile of its height
                assert!(
                    (100..=130).contains(&g.margin.len()),
                    "{f}: {}",
                    g.margin.len()
                );
                // widths cannot be smaller than the truth by more than the tilt allows
                assert!(g.width_md > 0.9 * true_widths(*f).0, "{f}");
                // no neighbour contact on a disconnected mesh: the extreme points are
                // taken from the crown itself
                assert!(g.contact_m.distance(g.contact_d) > 1.0, "{f}");
            }
        }
    }

    #[test]
    fn the_result_is_deterministic_and_stored_as_case_frames() {
        let m = model(Arch::Lower, Severity::Moderate, 3);
        let (v, t, l) = connected(&m);
        let a = compute(Arch::Lower, &v, &t, &l);
        let b = compute(Arch::Lower, &v, &t, &l);
        assert_eq!(a, b);

        // moved to another coordinate system and back: the same geometry
        let q = DQuat::from_axis_angle(DVec3::new(0.3, -1.0, 0.5).normalize(), 1.1);
        let m4 = DMat4::from_rotation_translation(q, DVec3::new(31.0, -12.0, 200.0));
        let moved = a.teeth.values().next().unwrap().transformed(&m4);
        let back = moved.transformed(&m4.inverse());
        let first = a.teeth.values().next().unwrap();
        assert!((back.cres - first.cres).length() < 1e-9);
        assert!((back.e_md - first.e_md).length() < 1e-12);
        assert!(moved.e_md.dot(moved.e_bl).abs() < 1e-12);
        assert!((moved.e_md.cross(moved.e_bl) - moved.e_ax).length() < 1e-12);

        for g in a.teeth.values() {
            let f = tooth_frame(g);
            // pose: axes of the tooth, origin at the crown reference point
            assert!((f.pose.apply_vector(DVec3::X) - g.e_md).length() < 1e-9);
            assert!((f.pose.apply_vector(DVec3::Y) - g.e_bl).length() < 1e-9);
            assert!((f.pose.apply_vector(DVec3::Z) - g.e_ax).length() < 1e-9);
            assert!((f.pose.t - g.reference).length() < 1e-12);
            // the centre of resistance is below the crown, along the axis
            assert!((f.pose.apply_point(f.center_of_resistance) - g.cres).length() < 1e-9);
            assert!(f.center_of_resistance.z < 0.0);
            // the plugin's rest frame
            let rest = rest_pose(&f);
            assert!((rest.t - g.cres).length() < 1e-9);
            assert!(rest.to_mat4().abs_diff_eq(g.rest_frame(), 1e-9));
        }
    }

    #[test]
    fn bad_input_is_reported() {
        let m = model(Arch::Lower, Severity::Mild, 7);
        let v: Vec<DVec3> = m.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let l = m.mesh.vertex_labels.clone().unwrap();
        let short = &l[..l.len() - 1];
        let err = run(
            &Input {
                arch: Arch::Lower,
                vertices: &v,
                tris: &m.mesh.tris,
                labels: short,
            },
            &Params::default(),
        )
        .unwrap_err();
        assert!(matches!(err, TeethError::Invalid(_)));
        let gum = vec![0u16; v.len()];
        let err = run(
            &Input {
                arch: Arch::Lower,
                vertices: &v,
                tris: &m.mesh.tris,
                labels: &gum,
            },
            &Params::default(),
        )
        .unwrap_err();
        assert_eq!(err, TeethError::NoTeeth);
        // a label of the other arch is not a tooth of this one
        let upper = vec![11u16; v.len()];
        assert_eq!(
            run(
                &Input {
                    arch: Arch::Lower,
                    vertices: &v,
                    tris: &m.mesh.tris,
                    labels: &upper
                },
                &Params::default()
            )
            .unwrap_err(),
            TeethError::NoTeeth
        );
        let bad = [[0u32, 1, v.len() as u32]];
        assert!(matches!(
            run(
                &Input {
                    arch: Arch::Lower,
                    vertices: &v,
                    tris: &bad,
                    labels: &l
                },
                &Params::default()
            ),
            Err(TeethError::Invalid(_))
        ));
    }

    #[test]
    fn helpers_behave_like_numpy() {
        // np.percentile([1, 2, 3, 4], 50) = 2.5, 90 -> 3.7
        assert!((percentile(&[4.0, 1.0, 3.0, 2.0], 50.0) - 2.5).abs() < 1e-12);
        assert!((percentile(&[1.0, 2.0, 3.0, 4.0], 90.0) - 3.7).abs() < 1e-12);
        assert_eq!(percentile(&[5.0], 80.0), 5.0);
        // a short list of faces gives the start axis, normalised
        let v = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let w = wall_axis(&v, &[[0, 1, 2]], &[0], Some(DVec3::new(0.0, 0.0, 3.0)));
        assert_eq!(w, DVec3::Z);
        assert_eq!(wall_axis(&v, &[[0, 1, 2]], &[0], None), DVec3::Z);
        assert_eq!(subsample(&[DVec3::X; 5], 250).len(), 5);
        // a tilt larger than the limit is brought back in its own direction
        let n = capped(DVec3::new(1.0, 0.0, 0.2), 15.0);
        assert!((angle_deg(n.z) - 15.0).abs() < 1e-9 && n.y == 0.0 && n.x > 0.0);
        assert_eq!(sign_or_one(0.0), 1.0);
        assert_eq!(sign_or_one(-0.0), 1.0);
        assert_eq!(sign_or_one(-3.0), -1.0);
    }
}
