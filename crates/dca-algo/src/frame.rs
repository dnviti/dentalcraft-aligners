//! World frame of a case: the rigid transform that puts the patient on the
//! world axes, plus the PCA and rigid-frame helpers it needs.
//!
//! Reference: `ortho_aligner/worldframe.py` (`delta`, `arch_centre`,
//! `is_identity`), `case_core.compute_frame` (the arch frame from the seeds)
//! and the PCA / rotation helpers of `ortho_aligner/meshutil.py`.
//!
//! Intraoral scanners export each arch in its own coordinates: the teeth can
//! stand far from the origin, tilted or upside down. The world frame moves
//! the whole patient rigidly so that
//!
//! * the world origin is the centre of the dental arch on the occlusal plane
//!   (halfway between the last molars and the central incisors, on the
//!   midline);
//! * the occlusal plane is the floor (XY) and the midline runs along Y, with
//!   the incisors towards −Y and the patient's right towards −X;
//! * the lower arch has the teeth up (+Z) and the upper arch stands above it
//!   with the teeth down.
//!
//! The reference is the lower arch when both exist, otherwise the only arch.
//! The transform is stored in the case ([`dc_case::WorldFrame`]); scan
//! meshes are not modified.
//!
//! The arch frame ([`ArchFrame`]) has its origin between the last molars on
//! the occlusal plane, `y` towards the incisors, `n` along the occlusal
//! direction and `x = y × n`. It is computed from the seeds
//! ([`arch_frame_from_seeds`], like the plugin). Without usable seeds,
//! [`arch_frame_from_mesh`] estimates it from the mesh alone with a PCA and
//! two shape heuristics; it is a fallback until the automatic seeds exist.

use dc_case::Pose;
use dca_model::Seed;
use glam::{DMat3, DMat4, DQuat, DVec3};
use nalgebra::{Matrix3, SymmetricEigen};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};

/// Translation below which the patient is already on the axes (mm).
pub const TOL_MM: f64 = 0.01;
/// Rotation below which the patient is already on the axes (degrees).
pub const TOL_DEG: f64 = 0.05;
/// Direction of the incisors in the world frame.
pub const FRONT: DVec3 = DVec3::NEG_Y;

/// Errors of the frame computation.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FrameError {
    /// Fewer than 4 seeds.
    #[error("at least 4 tooth marks (incisors and molars) are needed to place the arch")]
    TooFewSeeds,
    /// Seeds do not cover the incisors and both sides.
    #[error("tooth marks are needed on both sides and on the incisors")]
    SeedsOnOneSide,
    /// The mesh has no vertices or is flat in every direction.
    #[error("the scan is empty or damaged")]
    DegenerateMesh,
    /// The case has no upper or lower scan.
    #[error("the case has no arch scan")]
    NoArchScan,
}

/// Principal component analysis of a point cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct Pca {
    /// Centroid.
    pub mean: DVec3,
    /// Variances along the axes, largest first (sample variance, `n - 1`).
    pub eigenvalues: [f64; 3],
    /// Unit axes, largest variance first; right-handed. The sign of each
    /// axis is fixed (largest component positive) so that the result is
    /// reproducible.
    pub axes: [DVec3; 3],
}

/// PCA of `points` (`None` for an empty slice).
pub fn pca(points: &[DVec3]) -> Option<Pca> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f64;
    let mean = points.iter().copied().sum::<DVec3>() / n;
    let mut c = Matrix3::<f64>::zeros();
    for p in points {
        let d = *p - mean;
        let v = [d.x, d.y, d.z];
        for i in 0..3 {
            for j in 0..3 {
                c[(i, j)] += v[i] * v[j];
            }
        }
    }
    c /= (n - 1.0).max(1.0);
    let eig = SymmetricEigen::new(c);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| eig.eigenvalues[b].total_cmp(&eig.eigenvalues[a]));
    let axis = |k: usize| {
        let col = eig.eigenvectors.column(order[k]);
        let v = DVec3::new(col[0], col[1], col[2]).normalize_or_zero();
        let big = if v.x.abs() >= v.y.abs() && v.x.abs() >= v.z.abs() {
            v.x
        } else if v.y.abs() >= v.z.abs() {
            v.y
        } else {
            v.z
        };
        if big < 0.0 { -v } else { v }
    };
    let mut axes = [axis(0), axis(1), axis(2)];
    // keep the frame right-handed
    if axes[0].cross(axes[1]).dot(axes[2]) < 0.0 {
        axes[2] = -axes[2];
    }
    Some(Pca {
        mean,
        eigenvalues: [
            eig.eigenvalues[order[0]].max(0.0),
            eig.eigenvalues[order[1]].max(0.0),
            eig.eigenvalues[order[2]].max(0.0),
        ],
        axes,
    })
}

/// Rotation by `angle` (radians) about `axis` (normalised first; a zero axis
/// gives the identity).
pub fn rot_axis_angle(axis: DVec3, angle: f64) -> DMat3 {
    let a = axis.normalize_or_zero();
    if a == DVec3::ZERO {
        return DMat3::IDENTITY;
    }
    DMat3::from_axis_angle(a, angle)
}

/// Rotation matrix of the rotation vector `r` (axis times angle).
pub fn rotvec_to_matrix(r: DVec3) -> DMat3 {
    DMat3::from_quat(DQuat::from_scaled_axis(r))
}

/// Rotation vector (axis times angle, angle in `[0, π]`) of a rotation matrix.
pub fn matrix_to_rotvec(m: &DMat3) -> DVec3 {
    DQuat::from_mat3(m).normalize().to_scaled_axis()
}

/// Frame matrix with columns `x`, `y`, `z` and translation `origin`.
pub fn make_frame(x: DVec3, y: DVec3, z: DVec3, origin: DVec3) -> DMat4 {
    DMat4::from_cols(
        x.extend(0.0),
        y.extend(0.0),
        z.extend(0.0),
        origin.extend(1.0),
    )
}

/// Inverse of a rigid 4x4 matrix (transpose of the rotation).
pub fn rigid_inverse(m: &DMat4) -> DMat4 {
    let r = DMat3::from_mat4(*m).transpose();
    let t = -(r * m.w_axis.truncate());
    DMat4::from_cols(
        r.x_axis.extend(0.0),
        r.y_axis.extend(0.0),
        r.z_axis.extend(0.0),
        t.extend(1.0),
    )
}

/// Reference frame of an arch: origin between the last molars on the
/// occlusal plane, `y` towards the incisors, `n` occlusal, `x = y × n`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArchFrame {
    /// Origin (case coordinates, mm).
    #[schemars(with = "[f64; 3]")]
    pub origin: DVec3,
    /// Lateral axis.
    #[schemars(with = "[f64; 3]")]
    pub x: DVec3,
    /// Antero-posterior axis, towards the incisors.
    #[schemars(with = "[f64; 3]")]
    pub y: DVec3,
    /// Occlusal normal.
    #[schemars(with = "[f64; 3]")]
    pub n: DVec3,
}

impl ArchFrame {
    /// 4x4 matrix with the axes as columns.
    pub fn matrix(&self) -> DMat4 {
        make_frame(self.x, self.y, self.n, self.origin)
    }

    /// Frame from a 4x4 matrix with the axes as columns.
    pub fn from_matrix(m: &DMat4) -> Self {
        Self {
            origin: m.w_axis.truncate(),
            x: m.x_axis.truncate(),
            y: m.y_axis.truncate(),
            n: m.z_axis.truncate(),
        }
    }

    /// `+1` for a right-handed frame, `-1` for a mirrored one.
    pub fn handedness(&self) -> f64 {
        if self.x.dot(self.y.cross(self.n)) >= 0.0 {
            1.0
        } else {
            -1.0
        }
    }
}

/// Arch frame from the seeds of one arch (occlusal plane by least squares),
/// like `case_core.compute_frame`.
///
/// `mesh_centroid` is the centroid of the scan vertices: it sits on the gum
/// side of the plane of the crown tops and gives the occlusal sign. The
/// anterior axis points from the middle of the most distal seeds to the mean
/// of the incisor seeds (canines when there is no incisor).
pub fn arch_frame_from_seeds(
    seeds: &[Seed],
    mesh_centroid: DVec3,
) -> Result<ArchFrame, FrameError> {
    if seeds.len() < 4 {
        return Err(FrameError::TooFewSeeds);
    }
    let pts: Vec<DVec3> = seeds.iter().map(|s| s.position).collect();
    let pca = pca(&pts).ok_or(FrameError::TooFewSeeds)?;
    let mut n = pca.axes[2];
    if (mesh_centroid - pca.mean).dot(n) > 0.0 {
        n = -n;
    }
    let mean_of = |sel: &dyn Fn(&Seed) -> bool| -> Option<DVec3> {
        let v: Vec<DVec3> = seeds
            .iter()
            .filter(|s| sel(s))
            .map(|s| s.position)
            .collect();
        (!v.is_empty()).then(|| v.iter().copied().sum::<DVec3>() / v.len() as f64)
    };
    let ant = mean_of(&|s| anatomy::index(s.fdi) <= 2)
        .or_else(|| mean_of(&|s| anatomy::index(s.fdi) <= 3));
    // most distal seed of a side (the first one in the input order wins ties)
    let most_distal = |right: bool| {
        seeds
            .iter()
            .filter(|s| anatomy::is_right(s.fdi) == right)
            .fold(None::<&Seed>, |best, s| match best {
                Some(b) if anatomy::index(b.fdi) >= anatomy::index(s.fdi) => Some(b),
                _ => Some(s),
            })
            .map(|s| s.position)
    };
    let (Some(ant), Some(pr), Some(pl)) = (ant, most_distal(true), most_distal(false)) else {
        return Err(FrameError::SeedsOnOneSide);
    };
    let post = 0.5 * (pr + pl);
    let y = ant - post;
    let y = (y - y.dot(n) * n).normalize_or_zero();
    let x = y.cross(n).normalize_or_zero();
    let origin = post - (post - pca.mean).dot(n) * n;
    Ok(ArchFrame { origin, x, y, n })
}

/// Centre of the dental arch on the occlusal plane, on the midline: halfway
/// between the frame origin (between the last molars) and the central
/// incisors. With no anterior seed it is the frame origin.
pub fn arch_centre(frame: &ArchFrame, seeds: &[Seed]) -> DVec3 {
    let mean_of = |sel: &dyn Fn(&Seed) -> bool| -> Option<DVec3> {
        let v: Vec<DVec3> = seeds
            .iter()
            .filter(|s| sel(s))
            .map(|s| s.position)
            .collect();
        (!v.is_empty()).then(|| v.iter().copied().sum::<DVec3>() / v.len() as f64)
    };
    let ant = mean_of(&|s| anatomy::index(s.fdi) == 1)
        .or_else(|| mean_of(&|s| anatomy::index(s.fdi) <= 3));
    match ant {
        Some(a) => {
            let depth = (a - frame.origin).dot(frame.y);
            frame.origin + 0.5 * depth * frame.y
        }
        None => frame.origin,
    }
}

/// Rigid transform (case coordinates to world) that puts the arch with this
/// frame on the world axes, with `centre` as the world origin.
///
/// The occlusal direction becomes +Z for the lower arch and −Z for the upper
/// arch, the incisors go to −Y.
pub fn world_delta(frame: &ArchFrame, arch: Arch, centre: DVec3) -> Pose {
    let up = if arch == Arch::Upper {
        DVec3::NEG_Z
    } else {
        DVec3::Z
    };
    let hand = frame.handedness();
    let world = DMat3::from_cols(hand * FRONT.cross(up), FRONT, up);
    let local = DMat3::from_cols(frame.x, frame.y, frame.n);
    let r = world * local.transpose();
    Pose {
        t: -(r * centre),
        q: DQuat::from_mat3(&r).normalize(),
    }
}

/// Rotation angle of the pose in degrees.
pub fn rotation_degrees(pose: &Pose) -> f64 {
    let m = DMat3::from_quat(pose.q);
    let tr = m.x_axis.x + m.y_axis.y + m.z_axis.z;
    ((tr - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees()
}

/// `true` when the pose is within [`TOL_MM`] / [`TOL_DEG`] of the identity.
pub fn is_identity(pose: &Pose) -> bool {
    rotation_degrees(pose) < TOL_DEG && pose.t.length() < TOL_MM
}

/// Fraction of the occlusal height used for the occlusal plane of the
/// mesh-only estimate (90th percentile along the occlusal normal).
const OCCLUSAL_QUANTILE: f64 = 0.90;

/// Arch frame and arch centre estimated from the scan alone (no seeds):
/// PCA of the vertices, then
///
/// * `n` is the smallest-variance axis, signed towards the tail of the
///   distribution (the gum is a deep mass below the crowns, which stick
///   out: the occlusal side has the negative skew);
/// * `y` is the in-plane axis with the larger skewness (the midline axis of
///   a U-shaped arch is asymmetric, the lateral one is symmetric), signed
///   towards its tail (the incisors, for a U with the legs backwards);
/// * the origin is on the midline at the posterior end of the scan, at the
///   90th percentile of the height; the centre is halfway to the anterior
///   end.
///
/// A heuristic for scans in any pose; the seeds-based frame replaces it as
/// soon as the seeds exist.
pub fn arch_frame_from_mesh(vertices: &[DVec3]) -> Result<(ArchFrame, DVec3), FrameError> {
    let pca = pca(vertices).ok_or(FrameError::DegenerateMesh)?;
    if pca.eigenvalues[0] <= 0.0 || pca.eigenvalues[2] <= 1e-12 * pca.eigenvalues[0] {
        return Err(FrameError::DegenerateMesh);
    }
    let skew = |axis: DVec3| {
        let p: Vec<f64> = vertices.iter().map(|v| (*v - pca.mean).dot(axis)).collect();
        let m2 = p.iter().map(|v| v * v).sum::<f64>() / p.len() as f64;
        let m3 = p.iter().map(|v| v * v * v).sum::<f64>() / p.len() as f64;
        if m2 > 0.0 { m3 / m2.powf(1.5) } else { 0.0 }
    };
    // the gum is a deep mass below the crowns: the tail of the distribution
    // (negative skew) points to the occlusal side
    let mut n = pca.axes[2];
    if skew(n) > 0.0 {
        n = -n;
    }
    let (a, b) = (pca.axes[0], pca.axes[1]);
    let mut y = if skew(a).abs() > skew(b).abs() { a } else { b };
    if skew(y) > 0.0 {
        y = -y;
    }
    let y = (y - y.dot(n) * n).normalize_or_zero();
    let x = y.cross(n).normalize_or_zero();

    let coords = |v: &DVec3| {
        (
            *v - pca.mean,
            (*v - pca.mean).dot(y),
            (*v - pca.mean).dot(n),
        )
    };
    let ys: Vec<f64> = vertices.iter().map(|v| coords(v).1).collect();
    let mut ns: Vec<f64> = vertices.iter().map(|v| coords(v).2).collect();
    ns.sort_by(f64::total_cmp);
    let y_min = ys.iter().cloned().fold(f64::INFINITY, f64::min);
    let y_max = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let n_occ = ns[((ns.len() - 1) as f64 * OCCLUSAL_QUANTILE) as usize];
    let origin = pca.mean + y_min * y + n_occ * n;
    let centre = origin + 0.5 * (y_max - y_min) * y;
    Ok((ArchFrame { origin, x, y, n }, centre))
}

/// Which scan gives the position of the patient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reference {
    /// The lower arch when the case has one, otherwise the upper.
    #[default]
    Auto,
    /// The upper arch.
    Upper,
    /// The lower arch.
    Lower,
}

impl Reference {
    /// The arch to use among those available, `None` when the requested one
    /// is missing (or none is available).
    pub fn choose(self, has_upper: bool, has_lower: bool) -> Option<Arch> {
        match self {
            Reference::Auto if has_lower => Some(Arch::Lower),
            Reference::Auto if has_upper => Some(Arch::Upper),
            Reference::Lower if has_lower => Some(Arch::Lower),
            Reference::Upper if has_upper => Some(Arch::Upper),
            _ => None,
        }
    }
}

/// Parameters of the frame stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Which arch scan the world axes are taken from (default: the lower
    /// arch, else the upper).
    pub reference: Reference,
}

/// Input of the frame stage: the reference scan.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Arch of the reference scan.
    pub arch: Arch,
    /// Vertices of the scan in case coordinates (mm).
    pub vertices: &'a [DVec3],
    /// Seeds of the case (any arch; other arches are ignored).
    pub seeds: &'a [Seed],
}

/// Where the arch frame came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameSource {
    /// Computed from the seeds of the arch (the plugin's method).
    Seeds,
    /// Estimated from the mesh with a PCA (no usable seeds).
    Mesh,
}

/// Result of the frame stage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Output {
    /// Arch the frame was taken from.
    pub arch: Arch,
    /// How the arch frame was obtained.
    pub source: FrameSource,
    /// Arch frame in case coordinates.
    pub arch_frame: ArchFrame,
    /// Centre of the arch (becomes the world origin).
    #[schemars(with = "[f64; 3]")]
    pub centre: DVec3,
    /// Case coordinates to world.
    #[schemars(with = "serde_json::Value")]
    pub from_case: Pose,
    /// `true` when the scan already sits on the world axes.
    pub already_aligned: bool,
}

/// Computes the world frame of the reference scan. Without usable seeds the
/// arch frame is estimated from the mesh ([`FrameSource::Mesh`]); use
/// [`run_from_seeds`] where that guess is not acceptable.
pub fn run(input: &Input<'_>, _params: &Params) -> Result<Output, FrameError> {
    if input.vertices.is_empty() {
        return Err(FrameError::DegenerateMesh);
    }
    let arch_seeds = arch_seeds(input);
    let centroid = centroid(input.vertices);
    let (source, arch_frame, centre) = match arch_frame_from_seeds(&arch_seeds, centroid) {
        Ok(f) => (FrameSource::Seeds, f, arch_centre(&f, &arch_seeds)),
        Err(_) => {
            let (f, c) = arch_frame_from_mesh(input.vertices)?;
            (FrameSource::Mesh, f, c)
        }
    };
    Ok(output(input.arch, source, arch_frame, centre))
}

/// Like [`run`], but the arch frame must come from the seeds of the arch, as
/// in the plugin's `case_core.compute_frame`: with fewer than 4 seeds, or
/// none on a side or on the incisors, it is
/// [`FrameError::TooFewSeeds`] / [`FrameError::SeedsOnOneSide`] and nothing
/// is guessed from the mesh.
pub fn run_from_seeds(input: &Input<'_>, _params: &Params) -> Result<Output, FrameError> {
    if input.vertices.is_empty() {
        return Err(FrameError::DegenerateMesh);
    }
    let arch_seeds = arch_seeds(input);
    let arch_frame = arch_frame_from_seeds(&arch_seeds, centroid(input.vertices))?;
    let centre = arch_centre(&arch_frame, &arch_seeds);
    Ok(output(input.arch, FrameSource::Seeds, arch_frame, centre))
}

/// The seeds of the arch of the input.
fn arch_seeds(input: &Input<'_>) -> Vec<Seed> {
    input
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == input.arch)
        .copied()
        .collect()
}

fn centroid(vertices: &[DVec3]) -> DVec3 {
    vertices.iter().copied().sum::<DVec3>() / vertices.len() as f64
}

fn output(arch: Arch, source: FrameSource, arch_frame: ArchFrame, centre: DVec3) -> Output {
    let from_case = world_delta(&arch_frame, arch, centre);
    Output {
        arch,
        source,
        arch_frame,
        centre,
        already_aligned: is_identity(&from_case),
        from_case,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::Fdi;
    use glam::DQuat;

    fn seed(n: u8, p: DVec3) -> Seed {
        Seed {
            fdi: Fdi::new(n).unwrap(),
            position: p,
        }
    }

    /// Seeds of a lower arch in arch coordinates (origin between the molars,
    /// +Y anterior, +Z occlusal, +X the patient's right).
    fn lower_seeds() -> Vec<Seed> {
        let mut out = Vec::new();
        let pos = [
            (1u8, 3.0, 36.0),
            (2, 8.0, 34.0),
            (3, 12.0, 29.0),
            (4, 16.0, 23.0),
            (5, 19.0, 17.0),
            (6, 22.0, 9.0),
            (7, 23.0, 1.0),
        ];
        for (i, x, y) in pos {
            out.push(seed(40 + i, DVec3::new(x, y, 8.0)));
            out.push(seed(30 + i, DVec3::new(-x, y, 8.0)));
        }
        out
    }

    fn pose() -> Pose {
        Pose::new(
            DVec3::new(31.0, -12.0, 200.0),
            DQuat::from_rotation_z(2.1)
                * DQuat::from_rotation_x(-0.7)
                * DQuat::from_rotation_y(3.0),
        )
    }

    /// Gum mass below the crown tops, so the occlusal sign is known.
    fn gum_centroid() -> DVec3 {
        DVec3::new(0.0, 20.0, -4.0)
    }

    #[test]
    fn pca_of_an_axis_aligned_box() {
        let mut pts = Vec::new();
        for i in 0..11 {
            for j in 0..5 {
                for k in 0..3 {
                    pts.push(
                        DVec3::new(i as f64 * 2.0, j as f64, k as f64 * 0.5) + DVec3::splat(7.0),
                    );
                }
            }
        }
        let p = pca(&pts).unwrap();
        assert!((p.mean - DVec3::new(17.0, 9.0, 7.5)).length() < 1e-12);
        assert!(p.eigenvalues[0] > p.eigenvalues[1] && p.eigenvalues[1] > p.eigenvalues[2]);
        assert!(p.axes[0].abs_diff_eq(DVec3::X, 1e-9));
        assert!(p.axes[1].abs_diff_eq(DVec3::Y, 1e-9));
        assert!(p.axes[2].abs_diff_eq(DVec3::Z, 1e-9));
        // variance of 0..20 in 11 steps of 2, sample variance
        let var_x =
            pts.iter().map(|v| (v.x - 17.0).powi(2)).sum::<f64>() / (pts.len() as f64 - 1.0);
        assert!((p.eigenvalues[0] - var_x).abs() < 1e-9);
        assert!(pca(&[]).is_none());
    }

    #[test]
    fn pca_axes_are_orthonormal_and_right_handed() {
        let q = DQuat::from_rotation_z(0.4) * DQuat::from_rotation_y(1.1);
        let pts: Vec<DVec3> = (0..200)
            .map(|i| {
                let t = i as f64;
                q * DVec3::new(
                    (t * 0.37).sin() * 9.0,
                    (t * 0.11).cos() * 4.0,
                    (t * 0.7).sin() * 1.0,
                )
            })
            .collect();
        let p = pca(&pts).unwrap();
        for a in p.axes {
            assert!((a.length() - 1.0).abs() < 1e-12);
        }
        assert!(p.axes[0].dot(p.axes[1]).abs() < 1e-9);
        assert!(p.axes[0].cross(p.axes[1]).dot(p.axes[2]) > 0.999);
    }

    #[test]
    fn rotation_helpers_round_trip() {
        let r = rot_axis_angle(DVec3::new(1.0, 2.0, -0.5), 0.8);
        let v = matrix_to_rotvec(&r);
        assert!((v.length() - 0.8).abs() < 1e-12);
        assert!(rotvec_to_matrix(v).abs_diff_eq(r, 1e-12));
        assert_eq!(rotvec_to_matrix(DVec3::ZERO), DMat3::IDENTITY);
        assert_eq!(matrix_to_rotvec(&DMat3::IDENTITY), DVec3::ZERO);
        // 180 degrees
        let half = rot_axis_angle(DVec3::Z, std::f64::consts::PI);
        assert!((matrix_to_rotvec(&half).length() - std::f64::consts::PI).abs() < 1e-9);
        assert_eq!(rot_axis_angle(DVec3::ZERO, 1.0), DMat3::IDENTITY);
        // 90 degrees about Z takes X to Y
        let q = rot_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
        assert!((q * DVec3::X - DVec3::Y).length() < 1e-12);
    }

    #[test]
    fn frames_and_rigid_inverse() {
        let m = make_frame(DVec3::Y, DVec3::Z, DVec3::X, DVec3::new(1.0, 2.0, 3.0));
        let inv = rigid_inverse(&m);
        assert!((m * inv).abs_diff_eq(DMat4::IDENTITY, 1e-12));
        let f = ArchFrame::from_matrix(&m);
        assert_eq!(f.matrix(), m);
        assert_eq!(f.handedness(), 1.0);
        let mirrored = ArchFrame { x: -f.x, ..f };
        assert_eq!(mirrored.handedness(), -1.0);
    }

    #[test]
    fn frame_from_seeds_recovers_the_arch_coordinates() {
        let pose = pose();
        let seeds: Vec<Seed> = lower_seeds()
            .into_iter()
            .map(|s| Seed {
                position: pose.apply_point(s.position),
                ..s
            })
            .collect();
        let f = arch_frame_from_seeds(&seeds, pose.apply_point(gum_centroid())).unwrap();
        // n is the occlusal direction, y points to the incisors, x = y x n
        assert!(f.n.dot(pose.apply_vector(DVec3::Z)) > 0.999, "{f:?}");
        assert!(f.y.dot(pose.apply_vector(DVec3::Y)) > 0.999);
        assert!(f.x.dot(pose.apply_vector(DVec3::X)) > 0.999);
        assert!(f.handedness() > 0.0);
        // the origin lies between the last molars on the plane of the seeds
        let local = pose.inverse().apply_point(f.origin);
        assert!(
            local.x.abs() < 1e-9 && (local.y - 1.0).abs() < 1e-9 && (local.z - 8.0).abs() < 1e-9
        );
        // centre: halfway to the central incisors (mean of 41 and 31)
        let c = pose.inverse().apply_point(arch_centre(&f, &seeds));
        assert!(c.x.abs() < 1e-9 && (c.y - 0.5 * (1.0 + 36.0)).abs() < 1e-9);
    }

    #[test]
    fn frame_from_seeds_rejects_incomplete_input() {
        let seeds = lower_seeds();
        assert_eq!(
            arch_frame_from_seeds(&seeds[..3], gum_centroid()),
            Err(FrameError::TooFewSeeds)
        );
        let right: Vec<Seed> = seeds.iter().filter(|s| s.fdi.is_right()).copied().collect();
        assert_eq!(
            arch_frame_from_seeds(&right, gum_centroid()),
            Err(FrameError::SeedsOnOneSide)
        );
        let no_ant: Vec<Seed> = seeds
            .iter()
            .filter(|s| anatomy::index(s.fdi) > 3)
            .copied()
            .collect();
        assert_eq!(
            arch_frame_from_seeds(&no_ant, gum_centroid()),
            Err(FrameError::SeedsOnOneSide)
        );
    }

    #[test]
    fn arch_centre_without_anterior_seeds_is_the_origin() {
        let f = ArchFrame::from_matrix(&DMat4::IDENTITY);
        assert_eq!(arch_centre(&f, &[]), DVec3::ZERO);
        let canine = [seed(13, DVec3::new(0.0, 20.0, 0.0))];
        assert_eq!(arch_centre(&f, &canine), DVec3::new(0.0, 10.0, 0.0));
    }

    #[test]
    fn world_delta_puts_the_arch_on_the_axes() {
        let pose = pose();
        for arch in [Arch::Lower, Arch::Upper] {
            let seeds: Vec<Seed> = lower_seeds()
                .into_iter()
                .map(|s| Seed {
                    position: pose.apply_point(s.position),
                    ..s
                })
                .collect();
            let f = arch_frame_from_seeds(&seeds, pose.apply_point(gum_centroid())).unwrap();
            let c = arch_centre(&f, &seeds);
            let d = world_delta(&f, arch, c);
            assert!(d.apply_point(c).length() < 1e-9);
            let up = if arch == Arch::Lower {
                DVec3::Z
            } else {
                DVec3::NEG_Z
            };
            assert!(d.apply_vector(f.n).abs_diff_eq(up, 1e-9));
            assert!(d.apply_vector(f.y).abs_diff_eq(DVec3::NEG_Y, 1e-9));
            // arch x (the patient's right here) goes to −X for the lower arch
            let x_world = d.apply_vector(f.x);
            let expect = if arch == Arch::Lower {
                DVec3::NEG_X
            } else {
                DVec3::X
            };
            assert!(x_world.abs_diff_eq(expect, 1e-9), "{arch:?} {x_world:?}");
            // proper rotation
            let r = DMat3::from_quat(d.q);
            assert!((r.determinant() - 1.0).abs() < 1e-12);
            // incisors in front of the centre, right side at −X (lower)
            for s in &seeds {
                let w = d.apply_point(s.position);
                if anatomy::index(s.fdi) == 1 {
                    assert!(w.y < 0.0);
                }
                if arch == Arch::Lower && anatomy::index(s.fdi) >= 6 {
                    assert_eq!(w.x < 0.0, s.fdi.is_right(), "{}", s.fdi);
                }
            }
        }
    }

    #[test]
    fn a_patient_already_on_the_axes_gets_the_identity() {
        // lower arch built directly in world axes: incisors −Y, right −X, up +Z
        let seeds: Vec<Seed> = lower_seeds()
            .into_iter()
            .map(|s| {
                let p = s.position;
                Seed {
                    position: DVec3::new(-p.x, -(p.y - 18.5), p.z - 8.0),
                    ..s
                }
            })
            .collect();
        let centroid = DVec3::new(0.0, 0.0, -12.0);
        let f = arch_frame_from_seeds(&seeds, centroid).unwrap();
        let c = arch_centre(&f, &seeds);
        let d = world_delta(&f, Arch::Lower, c);
        assert!(is_identity(&d), "{d:?}");
        assert!(rotation_degrees(&d) < 1e-6);
    }

    #[test]
    fn is_identity_uses_the_tolerances() {
        assert!(is_identity(&Pose::IDENTITY));
        assert!(is_identity(&Pose::from_translation(DVec3::new(
            0.005, 0.0, 0.0
        ))));
        assert!(!is_identity(&Pose::from_translation(DVec3::new(
            0.02, 0.0, 0.0
        ))));
        assert!(is_identity(&Pose::from_rotation(DQuat::from_rotation_x(
            0.0004
        ))));
        assert!(!is_identity(&Pose::from_rotation(DQuat::from_rotation_x(
            0.002
        ))));
        assert!(
            (rotation_degrees(&Pose::from_rotation(DQuat::from_rotation_z(1.0)))
                - 57.29577951308232)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn reference_choice() {
        assert_eq!(Reference::Auto.choose(true, true), Some(Arch::Lower));
        assert_eq!(Reference::Auto.choose(true, false), Some(Arch::Upper));
        assert_eq!(Reference::Auto.choose(false, false), None);
        assert_eq!(Reference::Upper.choose(true, true), Some(Arch::Upper));
        assert_eq!(Reference::Upper.choose(false, true), None);
        assert_eq!(Reference::Lower.choose(true, false), None);
        let p: Params = serde_json::from_str("{}").unwrap();
        assert_eq!(p, Params::default());
        assert!(serde_json::from_str::<Params>(r#"{"reference":"x"}"#).is_err());
    }

    fn demo_input(arch: Arch, with_seeds: bool) -> (Vec<DVec3>, Vec<Seed>) {
        demo_input_seed(arch, 7, with_seeds)
    }

    fn demo_input_seed(arch: Arch, seed: u64, with_seeds: bool) -> (Vec<DVec3>, Vec<Seed>) {
        let d = crate::demo::run(&crate::demo::Params {
            arch,
            seed,
            severity: crate::demo::Severity::Severe,
            ..Default::default()
        });
        let v = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        (v, if with_seeds { d.seeds } else { Vec::new() })
    }

    #[test]
    fn the_demo_arch_is_put_on_the_world_axes() {
        for arch in [Arch::Lower, Arch::Upper] {
            let (v, seeds) = demo_input(arch, true);
            let input = Input {
                arch,
                vertices: &v,
                seeds: &seeds,
            };
            let out = run(&input, &Params::default()).unwrap();
            assert_eq!(out.source, FrameSource::Seeds);
            assert!(!out.already_aligned);
            // plaster orientation: occlusal +Z, incisors +Y
            let f = out.arch_frame;
            assert!(
                f.n.dot(DVec3::Z) > 0.97 && f.y.dot(DVec3::Y) > 0.97,
                "{f:?}"
            );

            // moving the patient there and asking again gives the identity
            let v2: Vec<DVec3> = v.iter().map(|p| out.from_case.apply_point(*p)).collect();
            let s2: Vec<Seed> = seeds
                .iter()
                .map(|s| Seed {
                    position: out.from_case.apply_point(s.position),
                    ..*s
                })
                .collect();
            let again = run(
                &Input {
                    arch,
                    vertices: &v2,
                    seeds: &s2,
                },
                &Params::default(),
            )
            .unwrap();
            assert!(again.already_aligned, "{:?}", again.from_case);
            assert!(again.centre.length() < 1e-6);
        }
    }

    #[test]
    fn the_mesh_only_estimate_agrees_with_the_seeds_on_the_demo() {
        for (arch, seed) in [Arch::Lower, Arch::Upper]
            .into_iter()
            .flat_map(|a| (1..=6).map(move |s| (a, s)))
        {
            let (v, seeds) = demo_input_seed(arch, seed, true);
            let with = run(
                &Input {
                    arch,
                    vertices: &v,
                    seeds: &seeds,
                },
                &Params::default(),
            )
            .unwrap();
            let without = run(
                &Input {
                    arch,
                    vertices: &v,
                    seeds: &[],
                },
                &Params::default(),
            )
            .unwrap();
            assert_eq!(without.source, FrameSource::Mesh);
            let (a, b) = (with.arch_frame, without.arch_frame);
            assert!(a.n.dot(b.n) > 0.97, "{arch:?} n {a:?} {b:?}");
            assert!(a.y.dot(b.y) > 0.97, "{arch:?} y");
            assert!(a.x.dot(b.x) > 0.97, "{arch:?} x");
            assert!((with.centre - without.centre).length() < 8.0, "{arch:?}");
            assert!(
                without
                    .from_case
                    .q
                    .angle_between(with.from_case.q)
                    .to_degrees()
                    < 12.0
            );
            // seeds of the other arch are ignored
            let other = demo_input(arch.opposite(), true).1;
            let ignored = run(
                &Input {
                    arch,
                    vertices: &v,
                    seeds: &other,
                },
                &Params::default(),
            )
            .unwrap();
            assert_eq!(ignored.source, FrameSource::Mesh);
        }
    }

    #[test]
    fn run_from_seeds_never_falls_back_to_the_mesh() {
        let (v, seeds) = demo_input(Arch::Lower, true);
        let run_with = |seeds: &[Seed]| {
            run_from_seeds(
                &Input {
                    arch: Arch::Lower,
                    vertices: &v,
                    seeds,
                },
                &Params::default(),
            )
        };
        // with usable seeds it is `run`
        let strict = run_with(&seeds).unwrap();
        let lenient = run(
            &Input {
                arch: Arch::Lower,
                vertices: &v,
                seeds: &seeds,
            },
            &Params::default(),
        )
        .unwrap();
        assert_eq!(strict.source, FrameSource::Seeds);
        assert_eq!(strict, lenient);
        // fewer than 4 seeds, other-arch seeds only, one side, no incisors
        assert_eq!(run_with(&seeds[..3]), Err(FrameError::TooFewSeeds));
        assert_eq!(run_with(&[]), Err(FrameError::TooFewSeeds));
        let other = demo_input(Arch::Upper, true).1;
        assert_eq!(run_with(&other), Err(FrameError::TooFewSeeds));
        let right: Vec<Seed> = seeds.iter().filter(|s| s.fdi.is_right()).copied().collect();
        assert_eq!(run_with(&right), Err(FrameError::SeedsOnOneSide));
        let no_ant: Vec<Seed> = seeds
            .iter()
            .filter(|s| anatomy::index(s.fdi) > 3)
            .copied()
            .collect();
        assert_eq!(run_with(&no_ant), Err(FrameError::SeedsOnOneSide));
        // the lenient stage still answers for the same seeds
        let guessed = run(
            &Input {
                arch: Arch::Lower,
                vertices: &v,
                seeds: &seeds[..3],
            },
            &Params::default(),
        )
        .unwrap();
        assert_eq!(guessed.source, FrameSource::Mesh);
        assert_eq!(
            run_from_seeds(
                &Input {
                    arch: Arch::Lower,
                    vertices: &[],
                    seeds: &seeds,
                },
                &Params::default()
            ),
            Err(FrameError::DegenerateMesh)
        );
    }

    #[test]
    fn a_flat_or_tiny_scan_has_no_estimate() {
        let line: Vec<DVec3> = (0..10).map(|i| DVec3::new(i as f64, 0.0, 0.0)).collect();
        assert_eq!(arch_frame_from_mesh(&line), Err(FrameError::DegenerateMesh));
        let plane: Vec<DVec3> = (0..100)
            .map(|i| DVec3::new((i % 10) as f64, (i / 10) as f64, 0.0))
            .collect();
        assert_eq!(
            arch_frame_from_mesh(&plane),
            Err(FrameError::DegenerateMesh)
        );
        assert_eq!(arch_frame_from_mesh(&[]), Err(FrameError::DegenerateMesh));
    }

    #[test]
    fn run_rejects_an_empty_scan() {
        let input = Input {
            arch: Arch::Lower,
            vertices: &[],
            seeds: &[],
        };
        assert_eq!(
            run(&input, &Params::default()),
            Err(FrameError::DegenerateMesh)
        );
    }
}
