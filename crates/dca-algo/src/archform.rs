//! Dental arch form models and arc-length utilities.
//!
//! Reference: `ortho_aligner/archform.py`.
//!
//! * Beta function (Braun et al. 1998): `y = 3.0314 D [(1/2 + x/W)(1/2 - x/W)]^0.8`
//! * Symmetric 4th-order polynomial `y = a + b x^2 + c x^4` (AlHarbi et al.
//!   2008: best description of the smooth arch curvature).
//! * Symmetric even polynomial through given points (Noroozi et al. 2001
//!   for 4 points).
//!
//! Arch coordinates: origin between the terminal molars on the occlusal
//! plane, +Y anterior, +Z occlusal, X = Y x Z. The curves here are planar
//! (XY).

use glam::DVec2;
use nalgebra::{DMatrix, DVector};

/// `1 / 0.25^0.8` (Braun 1998).
pub const BETA_K: f64 = 3.0314;

/// Beta function arch: `y(x)` for arch width `w` and depth `d`.
pub fn beta_y(x: f64, w: f64, d: f64) -> f64 {
    let u = (0.25 - (x / w).powi(2)).max(0.0);
    BETA_K * d * u.powf(0.8)
}

/// Symmetric quartic `a + b x^2 + c x^4`.
pub fn quartic_y(x: f64, abc: [f64; 3]) -> f64 {
    let x2 = x * x;
    abc[0] + abc[1] * x2 + abc[2] * x2 * x2
}

/// Minimum-norm least-squares solution of `a x = b` (the `numpy.linalg.lstsq`
/// default cut-off `eps * max(m, n)` relative to the largest singular value).
pub(crate) fn lstsq(a: &DMatrix<f64>, b: &DVector<f64>) -> DVector<f64> {
    let svd = a.clone().svd(true, true);
    let smax = svd.singular_values.iter().cloned().fold(0.0, f64::max);
    let eps = f64::EPSILON * a.nrows().max(a.ncols()) as f64 * smax;
    svd.solve(b, eps)
        .unwrap_or_else(|_| DVector::zeros(a.ncols()))
}

/// Least-squares fit of the symmetric quartic to points `(x, y)`, with
/// optional weights. Fewer than 3 points give a quadratic (`c = 0`).
pub fn fit_quartic(xy: &[DVec2], weights: Option<&[f64]>) -> [f64; 3] {
    let n = xy.len();
    let w = |i: usize| weights.map_or(1.0, |w| w[i]);
    let cols = if n < 3 { 2 } else { 3 };
    let a = DMatrix::from_fn(n, cols, |i, j| {
        let x = xy[i].x;
        w(i) * match j {
            0 => 1.0,
            1 => x * x,
            _ => x.powi(4),
        }
    });
    let y = DVector::from_fn(n, |i, _| xy[i].y * w(i));
    let sol = lstsq(&a, &y);
    if cols == 2 {
        [sol[0], sol[1], 0.0]
    } else {
        [sol[0], sol[1], sol[2]]
    }
}

/// Symmetric quartic through `(0, y0)`, `(±xc, yc)` and `(±xm, ym)`.
pub fn quartic_through(y0: f64, xc: f64, yc: f64, xm: f64, ym: f64) -> [f64; 3] {
    let m = nalgebra::Matrix2::new(xc.powi(2), xc.powi(4), xm.powi(2), xm.powi(4));
    let rhs = nalgebra::Vector2::new(yc - y0, ym - y0);
    match m.lu().solve(&rhs) {
        Some(s) if s.iter().all(|v| v.is_finite()) => [y0, s[0], s[1]],
        _ => [y0, (ym - y0) / (xm * xm).max(1e-9), 0.0],
    }
}

/// Symmetric even polynomial `y = sum_k a_k x^(2k)` through points
/// `(x_i, y_i)`, `i = 0..n-1` with `x_0 = 0` (degree `2(n-1)`). With 4
/// points this is the 6th-order form discussed by Noroozi et al. 2001.
pub fn poly_even_through(points: &[DVec2]) -> Vec<f64> {
    let n = points.len();
    let a = DMatrix::from_fn(n, n, |i, k| points[i].x.powi(2 * k as i32));
    let b = DVector::from_fn(n, |i, _| points[i].y);
    match a.clone().lu().solve(&b) {
        Some(s) if s.iter().all(|v| v.is_finite()) => s.iter().copied().collect(),
        _ => lstsq(&a, &b).iter().copied().collect(),
    }
}

/// Value of the even polynomial of [`poly_even_through`] at `x`.
pub fn poly_even_y(x: f64, coef: &[f64]) -> f64 {
    coef.iter()
        .enumerate()
        .map(|(k, c)| c * x.powi(2 * k as i32))
        .sum()
}

/// `numpy.interp` for increasing `xp` (clamped at the ends).
fn interp(x: f64, xp: &[f64], fp: impl Fn(usize) -> f64) -> f64 {
    let n = xp.len();
    if x <= xp[0] {
        return fp(0);
    }
    if x >= xp[n - 1] {
        return fp(n - 1);
    }
    let hi = xp.partition_point(|&v| v <= x).min(n - 1);
    let lo = hi - 1;
    let dx = xp[hi] - xp[lo];
    if dx <= 0.0 {
        return fp(lo);
    }
    let t = (x - xp[lo]) / dx;
    fp(lo) + t * (fp(hi) - fp(lo))
}

/// Planar polyline (dense) with signed arc length `s` measured from the
/// point where `x = 0`; `s` grows with `x`.
#[derive(Debug, Clone)]
pub struct ArchCurve {
    /// Polyline vertices.
    pub points: Vec<DVec2>,
    /// Signed arc length of every vertex.
    pub s: Vec<f64>,
}

impl ArchCurve {
    /// Curve through the polyline `points` (at least two, `x` increasing
    /// across the midline).
    pub fn new(points: Vec<DVec2>) -> Self {
        let n = points.len();
        let mut s = Vec::with_capacity(n);
        let mut acc = 0.0;
        s.push(0.0);
        for w in points.windows(2) {
            acc += (w[1] - w[0]).length();
            s.push(acc);
        }
        // arc-length origin at the x = 0 crossing (linear interpolation)
        let crossings: Vec<usize> = (0..n - 1)
            .filter(|&i| points[i].x <= 0.0 && points[i + 1].x > 0.0)
            .collect();
        let s0 = if crossings.is_empty() {
            let j = (0..n)
                .min_by(|&a, &b| points[a].x.abs().total_cmp(&points[b].x.abs()))
                .unwrap_or(0);
            s[j]
        } else {
            let j = crossings[crossings.len() / 2];
            let t = -points[j].x / (points[j + 1].x - points[j].x);
            s[j] + t * (s[j + 1] - s[j])
        };
        for v in &mut s {
            *v -= s0;
        }
        Self { points, s }
    }

    /// Point at arc length `s` (clamped to the ends of the polyline).
    pub fn point(&self, s: f64) -> DVec2 {
        DVec2::new(
            interp(s, &self.s, |i| self.points[i].x),
            interp(s, &self.s, |i| self.points[i].y),
        )
    }

    /// Unit tangent at arc length `s` (central difference over `±h`).
    pub fn tangent(&self, s: f64, h: f64) -> DVec2 {
        let t = self.point(s + h) - self.point(s - h);
        let n = t.length();
        if n > 0.0 { t / n } else { t }
    }

    /// Outward (buccal / labial) normal: the tangent rotated by +90 degrees.
    pub fn normal_out(&self, s: f64) -> DVec2 {
        let t = self.tangent(s, 0.05);
        DVec2::new(-t.y, t.x)
    }

    /// Arc length of the point of the curve closest to `p`.
    pub fn project(&self, p: DVec2) -> f64 {
        let j = (0..self.points.len())
            .min_by(|&a, &b| {
                (self.points[a] - p)
                    .length_squared()
                    .total_cmp(&(self.points[b] - p).length_squared())
            })
            .unwrap_or(0);
        let lo = j.saturating_sub(2);
        let hi = (j + 2).min(self.points.len() - 1);
        let (s_lo, s_hi) = (self.s[lo], self.s[hi]);
        let mut best = (f64::INFINITY, s_lo);
        for k in 0..41 {
            let ss = s_lo + (s_hi - s_lo) * k as f64 / 40.0;
            let d = (self.point(ss) - p).length_squared();
            if d < best.0 {
                best = (d, ss);
            }
        }
        best.1
    }

    /// `s` such that `|P(s) - P(s0)| = chord`, moving along `direction`
    /// (`+1` or `-1`).
    pub fn s_at_chord(&self, s0: f64, chord: f64, direction: f64) -> f64 {
        let p0 = self.point(s0);
        let mut lo = s0;
        let mut hi = s0 + direction * (chord * 2.0 + 1.0);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if (self.point(mid) - p0).length() < chord {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }
}

/// Appends straight extensions along the end tangents, or parallel to the
/// mid-sagittal plane (`-Y`) when `posterior` (like the end of Braun's beta
/// function, whose tangent at the molar line is antero-posterior).
fn extend(p: &[DVec2], length: f64, n: usize, posterior: bool) -> Vec<DVec2> {
    let len = p.len();
    let mut t0 = p[0] - p[5.min(len - 1)];
    let mut t1 = p[len - 1] - p[len.saturating_sub(6)];
    if posterior {
        t0 = DVec2::new(0.0, -1.0);
        t1 = DVec2::new(0.0, -1.0);
    }
    t0 /= t0.length().max(1e-12);
    t1 /= t1.length().max(1e-12);
    let ks: Vec<f64> = (1..n).map(|i| length * i as f64 / (n - 1) as f64).collect();
    let mut out = Vec::with_capacity(len + 2 * ks.len());
    out.extend(ks.iter().rev().map(|&k| p[0] + k * t0));
    out.extend_from_slice(p);
    out.extend(ks.iter().map(|&k| p[len - 1] + k * t1));
    out
}

/// Abscissae of a symmetric arch curve, denser towards the ends
/// (`x = half sin(phi)`).
fn xs(half_extent: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let phi =
                -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / (n - 1) as f64;
            half_extent * phi.sin()
        })
        .collect()
}

/// Number of abscissae of the dense curve.
const SAMPLES: usize = 6001;

/// Parametric model of an arch form.
#[derive(Debug, Clone, PartialEq)]
pub enum ArchModel {
    /// Symmetric quartic `(a, b, c)`.
    Quartic([f64; 3]),
    /// Even polynomial with these coefficients.
    Even(Vec<f64>),
    /// Beta function with width `w`, depth `d` and offset `y0`.
    Beta {
        /// Width.
        w: f64,
        /// Depth.
        d: f64,
        /// Offset along Y.
        y0: f64,
    },
}

/// Dense [`ArchCurve`] of a model over `x in [-half_extent, half_extent]`,
/// extended past both ends (`posterior`: parallel to the sagittal plane).
/// The beta model spans its own width (`half_extent` is ignored).
pub fn curve_from_model(model: &ArchModel, half_extent: f64, posterior: bool) -> ArchCurve {
    let (pts, length, posterior) = match model {
        ArchModel::Even(coef) => (
            xs(half_extent, SAMPLES)
                .into_iter()
                .map(|x| DVec2::new(x, poly_even_y(x, coef)))
                .collect::<Vec<_>>(),
            30.0,
            posterior,
        ),
        ArchModel::Beta { w, d, y0 } => (
            xs(w / 2.0, SAMPLES)
                .into_iter()
                .map(|x| DVec2::new(x, y0 + beta_y(x, *w, *d)))
                .collect(),
            25.0,
            false,
        ),
        ArchModel::Quartic(abc) => (
            xs(half_extent, SAMPLES)
                .into_iter()
                .map(|x| DVec2::new(x, quartic_y(x, *abc)))
                .collect(),
            30.0,
            posterior,
        ),
    };
    ArchCurve::new(extend(&pts, length, 200, posterior))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_function_matches_braun() {
        // at the midline y = 3.0314 D 0.25^0.8 = D, at the ends 0
        assert!((beta_y(0.0, 50.0, 35.0) - 35.0).abs() < 1e-3);
        assert_eq!(beta_y(25.0, 50.0, 35.0), 0.0);
        assert_eq!(beta_y(40.0, 50.0, 35.0), 0.0);
        // symmetric and decreasing
        assert_eq!(beta_y(10.0, 50.0, 35.0), beta_y(-10.0, 50.0, 35.0));
        assert!(beta_y(10.0, 50.0, 35.0) > beta_y(20.0, 50.0, 35.0));
    }

    #[test]
    fn quartic_fit_recovers_the_coefficients() {
        let abc = [30.0, -0.04, 1.5e-6];
        let pts: Vec<DVec2> = (-10..=10)
            .map(|i| {
                let x = i as f64 * 2.5;
                DVec2::new(x, quartic_y(x, abc))
            })
            .collect();
        let fit = fit_quartic(&pts, None);
        for (a, b) in fit.iter().zip(abc) {
            assert!((a - b).abs() < 1e-9 * b.abs().max(1.0), "{fit:?}");
        }
        // weights do not change an exact fit
        let w: Vec<f64> = (0..pts.len()).map(|i| 1.0 + i as f64).collect();
        let fit_w = fit_quartic(&pts, Some(&w));
        for (a, b) in fit_w.iter().zip(abc) {
            assert!((a - b).abs() < 1e-9 * b.abs().max(1.0));
        }
    }

    #[test]
    fn quartic_fit_with_few_points_is_a_quadratic() {
        let two = fit_quartic(&[DVec2::new(1.0, 2.0), DVec2::new(2.0, 5.0)], None);
        assert_eq!(two[2], 0.0);
        // y = a + b x^2 through (1,2) and (2,5): b = 1, a = 1
        assert!((two[0] - 1.0).abs() < 1e-12 && (two[1] - 1.0).abs() < 1e-12);
        // one point: minimum-norm solution, still finite
        let one = fit_quartic(&[DVec2::new(2.0, 4.0)], None);
        assert!(one.iter().all(|v| v.is_finite()));
        assert!((one[0] + one[1] * 4.0 - 4.0).abs() < 1e-12);
    }

    #[test]
    fn quartic_through_passes_through_the_points() {
        let q = quartic_through(40.0, 8.0, 36.0, 25.0, 5.0);
        assert_eq!(quartic_y(0.0, q), 40.0);
        assert!((quartic_y(8.0, q) - 36.0).abs() < 1e-9);
        assert!((quartic_y(-25.0, q) - 5.0).abs() < 1e-9);
        // singular system falls back to a parabola
        let deg = quartic_through(10.0, 0.0, 5.0, 0.0, 4.0);
        assert_eq!(deg[2], 0.0);
        assert!((deg[1] - (4.0 - 10.0) / 1e-9).abs() / deg[1].abs() < 1e-9);
    }

    #[test]
    fn even_polynomial_through_four_points() {
        let pts = [
            DVec2::new(0.0, 38.0),
            DVec2::new(10.0, 34.0),
            DVec2::new(20.0, 22.0),
            DVec2::new(28.0, 3.0),
        ];
        let coef = poly_even_through(&pts);
        assert_eq!(coef.len(), 4);
        for p in pts {
            assert!((poly_even_y(p.x, &coef) - p.y).abs() < 1e-8);
            assert!((poly_even_y(-p.x, &coef) - p.y).abs() < 1e-8);
        }
    }

    #[test]
    fn curve_arc_length_origin_is_at_the_midline() {
        let c = curve_from_model(
            &ArchModel::Beta {
                w: 50.0,
                d: 30.0,
                y0: 0.0,
            },
            25.0,
            false,
        );
        assert!(c.point(0.0).x.abs() < 1e-3);
        assert!((c.point(0.0).y - 30.0).abs() < 0.1);
        // s grows with x
        assert!(c.point(10.0).x > 0.0 && c.point(-10.0).x < 0.0);
        // the curve is symmetric in arc length
        let (a, b) = (c.point(17.3), c.point(-17.3));
        assert!((a.x + b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3);
        // tangent is unit, normal points outwards (+Y at the midline)
        assert!((c.tangent(5.0, 0.05).length() - 1.0).abs() < 1e-9);
        let n = c.normal_out(0.0);
        assert!(n.y > 0.99, "{n:?}");
    }

    #[test]
    fn project_and_chord_are_inverse_of_point() {
        let c = curve_from_model(&ArchModel::Quartic([30.0, -0.03, 0.0]), 25.0, false);
        for s in [-30.0, -7.5, 0.0, 12.0, 33.3] {
            let p = c.point(s);
            let back = c.project(p);
            assert!((back - s).abs() < 0.05, "s={s} back={back}");
        }
        for dir in [1.0, -1.0] {
            let s2 = c.s_at_chord(3.0, 7.0, dir);
            assert!(((c.point(s2) - c.point(3.0)).length() - 7.0).abs() < 1e-9);
            assert!((s2 - 3.0) * dir > 0.0);
        }
    }

    #[test]
    fn posterior_extension_runs_parallel_to_the_sagittal_plane() {
        let m = ArchModel::Quartic([30.0, -0.03, 0.0]);
        let plain = curve_from_model(&m, 25.0, false);
        let post = curve_from_model(&m, 25.0, true);
        // the 199 extension points at each end lie on the vertical line through the end
        // point of the curve (x = ±25), going backwards (−Y)
        let n = post.points.len();
        for p in &post.points[..199] {
            assert!((p.x + 25.0).abs() < 1e-9);
        }
        for p in &post.points[n - 199..] {
            assert!((p.x - 25.0).abs() < 1e-9);
        }
        assert!((post.points[0].y - (post.points[199].y - 30.0)).abs() < 1e-9);
        let first = post.points[0];
        let last = *post.points.last().unwrap();
        assert!(first.y < post.points[199].y && last.y < post.points[post.points.len() - 200].y);
        assert!((first.x + 25.0).abs() < 1e-9 && (last.x - 25.0).abs() < 1e-9);
        // the plain extension follows the (here sloping) end tangent
        assert!((plain.points[0].x + 25.0).abs() > 1.0);
    }
}
