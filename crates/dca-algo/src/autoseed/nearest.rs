//! Exact nearest sample of a dense planar polyline, like the plugin's
//! `gpu_query.knn` with one neighbour: squared distances in `f64`, ties to
//! the smallest index.
//!
//! The samples come in order along a curve, so runs of consecutive samples
//! are spatially compact: each run of [`BLOCK`] samples gets a bounding box,
//! and only the boxes that can hold a point at least as close as the best
//! one found so far are searched.

use glam::DVec2;
use rayon::prelude::*;

/// Samples per block.
const BLOCK: usize = 32;

/// Dense planar samples with block bounding boxes.
pub(crate) struct Nearest {
    pts: Vec<DVec2>,
    boxes: Vec<(DVec2, DVec2)>,
}

impl Nearest {
    /// Index of the samples `pts` (not empty).
    pub(crate) fn new(pts: Vec<DVec2>) -> Self {
        let boxes = pts
            .chunks(BLOCK)
            .map(|c| {
                c.iter()
                    .fold((c[0], c[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
            })
            .collect();
        Self { pts, boxes }
    }

    /// Index of the sample nearest to `q`.
    pub(crate) fn nearest(&self, q: DVec2) -> usize {
        let lb: Vec<f64> = self
            .boxes
            .iter()
            .map(|(lo, hi)| {
                let d = (*lo - q).max(q - *hi).max(DVec2::ZERO);
                d.x * d.x + d.y * d.y
            })
            .collect();
        let first = (0..lb.len())
            .min_by(|&a, &b| lb[a].total_cmp(&lb[b]))
            .unwrap_or(0);
        let mut best = (f64::INFINITY, usize::MAX);
        let scan = |b: usize, best: &mut (f64, usize)| {
            let end = ((b + 1) * BLOCK).min(self.pts.len());
            for i in b * BLOCK..end {
                let d = self.pts[i] - q;
                let d2 = d.x * d.x + d.y * d.y;
                if d2 < best.0 || (d2 == best.0 && i < best.1) {
                    *best = (d2, i);
                }
            }
        };
        scan(first, &mut best);
        // a small margin: the box bound must never exclude an exact tie
        let limit = best.0 + 1e-9 * (1.0 + best.0);
        for (b, l) in lb.iter().enumerate() {
            if b != first && *l <= limit {
                scan(b, &mut best);
            }
        }
        best.1
    }

    /// [`Nearest::nearest`] of every query (in parallel, same order).
    pub(crate) fn nearest_all(&self, queries: &[DVec2]) -> Vec<usize> {
        queries.par_iter().map(|q| self.nearest(*q)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_brute_force_with_ties_to_the_first() {
        let pts: Vec<DVec2> = (0..500)
            .map(|i| {
                let t = i as f64 * 0.05;
                DVec2::new(10.0 * t.cos(), 7.0 * t.sin())
            })
            .chain([DVec2::new(1.0, 1.0), DVec2::new(1.0, 1.0)])
            .collect();
        let idx = Nearest::new(pts.clone());
        for k in 0..400 {
            let q = DVec2::new(
                ((k * 37) % 41) as f64 - 20.0,
                ((k * 53) % 29) as f64 * 0.7 - 10.0,
            );
            let brute = (0..pts.len())
                .min_by(|&a, &b| {
                    let da = (pts[a] - q).length_squared();
                    let db = (pts[b] - q).length_squared();
                    da.total_cmp(&db).then(a.cmp(&b))
                })
                .unwrap();
            assert_eq!(idx.nearest(q), brute, "{q:?}");
        }
        assert_eq!(idx.nearest(DVec2::new(1.0, 1.0)), 500);
    }
}
