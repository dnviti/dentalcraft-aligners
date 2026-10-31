//! The few numpy semantics the port depends on, reproduced exactly where the
//! result can change a decision (bin indices, rounding, ranges) and to the
//! last bits where that is cheap (percentile interpolation, `interp`).

use nalgebra::{DMatrix, DVector};

/// `numpy.percentile(v, q)` with the default linear method (`q` in
/// percent). NaN values sort last, like numpy's sort. `v` must not be empty.
pub(crate) fn percentile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    percentile_sorted(&s, q)
}

/// [`percentile`] of values already sorted.
pub(crate) fn percentile_sorted(s: &[f64], q: f64) -> f64 {
    let n = s.len();
    let virt = (n - 1) as f64 * (q / 100.0);
    let lo = virt.floor();
    let i = (lo as usize).min(n - 1);
    let j = (i + 1).min(n - 1);
    let t = virt - lo;
    let (a, b) = (s[i], s[j]);
    // numpy `_lerp`: from the nearer end
    let d = b - a;
    if t >= 0.5 {
        b - d * (1.0 - t)
    } else {
        a + d * t
    }
}

/// `numpy.median`: the middle value, or the mean of the two middle ones.
pub(crate) fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// `numpy.interp(x, xp, fp)` for increasing `xp` (clamped at the ends).
pub(crate) fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if x < xp[0] {
        return fp[0];
    }
    if x > xp[n - 1] {
        return fp[n - 1];
    }
    // last j with xp[j] <= x
    let j = xp.partition_point(|&v| v <= x).saturating_sub(1);
    if j == n - 1 || x == xp[j] {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    let r = slope * (x - xp[j]) + fp[j];
    if r.is_nan() {
        let r2 = slope * (x - xp[j + 1]) + fp[j + 1];
        if r2.is_nan() && fp[j] == fp[j + 1] {
            return fp[j];
        }
        return r2;
    }
    r
}

/// `numpy.interp(arange(n), idx[good], v[good])`: fills the NaN entries of
/// `v` by linear interpolation between the finite ones (constant beyond the
/// first and the last). `None` when no entry is finite.
pub(crate) fn fill_nan(v: &[f64]) -> Option<Vec<f64>> {
    let (xp, fp): (Vec<f64>, Vec<f64>) = v
        .iter()
        .enumerate()
        .filter(|(_, x)| !x.is_nan())
        .map(|(i, x)| (i as f64, *x))
        .unzip();
    if xp.is_empty() {
        return None;
    }
    Some((0..v.len()).map(|i| interp(i as f64, &xp, &fp)).collect())
}

/// `numpy.convolve(numpy.pad(v, r, mode="edge"), ones(2r+1)/(2r+1), "valid")`:
/// centred moving average with the ends repeated.
pub(crate) fn moving_average(v: &[f64], r: usize) -> Vec<f64> {
    let n = v.len();
    let k = 1.0 / (2 * r + 1) as f64;
    let at = |i: isize| v[i.clamp(0, n as isize - 1) as usize];
    (0..n as isize)
        .map(|i| {
            (-(r as isize)..=r as isize)
                .map(|m| at(i + m) * k)
                .sum::<f64>()
        })
        .collect()
}

/// `numpy.convolve(v, ones(w), "same")`: moving sum over `w` entries, with
/// numpy's alignment for even `w` (entries `i - w/2 .. i + w/2 - 1 + w%2`).
pub(crate) fn moving_sum_same(v: &[f64], w: usize) -> Vec<f64> {
    let n = v.len() as isize;
    let off = ((w - 1) / 2) as isize;
    (0..n)
        .map(|i| {
            (0..w as isize)
                .map(|m| i + off - m)
                .filter(|&k| (0..n).contains(&k))
                .map(|k| v[k as usize])
                .sum()
        })
        .collect()
}

/// `numpy.arange(start, stop, step)` for floats: `ceil((stop - start) / step)`
/// values `start + i * delta` with `delta = (start + step) - start`.
pub(crate) fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let len = ((stop - start) / step).ceil();
    if len.is_nan() || len <= 0.0 {
        return Vec::new();
    }
    let delta = (start + step) - start;
    (0..len as usize)
        .map(|i| start + i as f64 * delta)
        .collect()
}

/// `numpy.linspace(start, stop, num)` (`num >= 2`).
pub(crate) fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    let step = (stop - start) / (num - 1) as f64;
    let mut out: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
    out[num - 1] = stop;
    out
}

/// Rows of `x` evenly spread in their order, at most `n` (`meshutil.subsample`).
pub(crate) fn subsample<T: Copy>(x: &[T], n: usize) -> Vec<T> {
    if x.len() <= n {
        return x.to_vec();
    }
    linspace(0.0, (x.len() - 1) as f64, n)
        .into_iter()
        .map(|i| x[i as usize])
        .collect()
}

/// Least squares `rows · sol ≈ rhs` for rows of 3 columns
/// (`numpy.linalg.lstsq`, minimum norm). No rows give zeros.
pub(crate) fn lstsq3(rows: &[[f64; 3]], rhs: &[f64]) -> [f64; 3] {
    if rows.is_empty() {
        return [0.0; 3];
    }
    let a = DMatrix::from_fn(rows.len(), 3, |i, j| rows[i][j]);
    let b = DVector::from_column_slice(rhs);
    let x = crate::archform::lstsq(&a, &b);
    [x[0], x[1], x[2]]
}

/// Mean of a non-empty slice.
pub(crate) fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_follow_numpy() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        // numpy.percentile(x, [2, 98, 99.5]) = [1.14, 7.86, 7.965]
        assert!((percentile(&x, 2.0) - 1.14).abs() < 1e-12);
        assert!((percentile(&x, 98.0) - 7.86).abs() < 1e-12);
        assert!((percentile(&x, 99.5) - 7.965).abs() < 1e-12);
        assert_eq!(percentile(&[3.0], 50.0), 3.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[4.0, 1.0, 3.0]), 3.0);
    }

    #[test]
    fn interp_and_fill() {
        let xp = [0.0, 1.0, 3.0];
        let fp = [1.0, 3.0, -1.0];
        assert_eq!(interp(-1.0, &xp, &fp), 1.0);
        assert_eq!(interp(0.5, &xp, &fp), 2.0);
        assert_eq!(interp(2.0, &xp, &fp), 1.0);
        assert_eq!(interp(3.0, &xp, &fp), -1.0);
        assert_eq!(interp(9.0, &xp, &fp), -1.0);
        let f = fill_nan(&[f64::NAN, 2.0, f64::NAN, 4.0, f64::NAN]).unwrap();
        assert_eq!(f, vec![2.0, 2.0, 3.0, 4.0, 4.0]);
        assert!(fill_nan(&[f64::NAN]).is_none());
    }

    #[test]
    fn convolutions_follow_numpy() {
        // numpy.convolve([1,2,3,4,5,6], ones(4), "same") = [3, 6, 10, 14, 18, 15]
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(
            moving_sum_same(&v, 4),
            vec![3.0, 6.0, 10.0, 14.0, 18.0, 15.0]
        );
        assert_eq!(moving_sum_same(&v, 1), v.to_vec());
        let m = moving_average(&v, 1);
        assert!((m[0] - 4.0 / 3.0).abs() < 1e-12 && (m[5] - 17.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn ranges_follow_numpy() {
        let a = arange(-75.31234567, 80.0, 0.05);
        assert_eq!(a.len(), 3107);
        let d = (-75.31234567 + 0.05) - -75.31234567;
        assert_eq!(a[1000], -75.31234567 + 1000.0 * d);
        assert!(arange(1.0, 1.0, 0.5).is_empty());
        let l = linspace(0.82, 1.18, 10);
        assert_eq!(l[2], 0.8999999999999999);
        assert_eq!(l[9], 1.18);
        assert_eq!(subsample(&[0, 1, 2, 3, 4, 5, 6], 3), vec![0, 3, 6]);
        assert_eq!(subsample(&[0, 1], 3), vec![0, 1]);
    }
}
