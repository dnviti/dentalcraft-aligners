//! The margin line of a tooth: where the crown ends on the scan
//! (`manual.margin_line`).
//!
//! It is the line the closed tooth is cut along (`reconstruct::crown_patch`):
//! the 0.5 iso-line of the tooth label, 1 on the vertices of the tooth and 0
//! elsewhere, smoothed on the surface ([`SMOOTH_ITER`] umbrella steps), cut
//! by marching triangles with the cut points kept [`T_MIN`] away from the
//! vertices. The line is not stored in the case: it is worked out here from
//! the labels, so the view of the region and the command that reports it
//! always show the line the next rebuild of the tooth would use.

use std::collections::HashMap;

use glam::DVec3;

use crate::reconstruct::geom::{smooth_scalar, unique_edges};
use crate::reconstruct::{SMOOTH_ITER, T_MIN};

/// The margin line of one tooth: the points where it crosses the edges of
/// the scan and the segments between them (one per crossed triangle).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarginLine {
    /// Points of the line (in the coordinates of the scan vertices).
    pub points: Vec<DVec3>,
    /// Segments, as indices of `points`.
    pub segments: Vec<[u32; 2]>,
}

/// A chain of margin points, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Polyline {
    /// Indices of [`MarginLine::points`] along the line.
    pub points: Vec<u32>,
    /// The last point joins the first (the usual crown outline); an open
    /// chain ends on the open border of the scan.
    pub closed: bool,
}

impl MarginLine {
    /// `true` when the tooth has no margin line (no region, or the whole
    /// scan is the tooth).
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Total length of the line (mm, in the units of the scan).
    pub fn length(&self) -> f64 {
        if self.segments.is_empty() {
            // not `sum()`, which gives -0.0 for nothing
            return 0.0;
        }
        self.segments
            .iter()
            .map(|s| (self.points[s[1] as usize] - self.points[s[0] as usize]).length())
            .sum()
    }

    /// The line as chains of points: closed loops for the outline of a
    /// crown, open chains where it reaches the open border of the scan. A
    /// point where more than two segments meet (a non-manifold scan) ends a
    /// chain and starts the next one.
    pub fn polylines(&self) -> Vec<Polyline> {
        let n = self.points.len();
        let mut next: Vec<Vec<(u32, usize)>> = vec![Vec::new(); n];
        for (k, s) in self.segments.iter().enumerate() {
            next[s[0] as usize].push((s[1], k));
            next[s[1] as usize].push((s[0], k));
        }
        let mut used = vec![false; self.segments.len()];
        let mut out = Vec::new();
        // open chains first (they start at a point with one segment, or
        // with more than two), then what is left are closed loops
        let starts = (0..n as u32)
            .filter(|&p| next[p as usize].len() != 2)
            .chain(0..n as u32);
        for start in starts {
            while let Some(&(_, first)) = next[start as usize].iter().find(|(_, k)| !used[*k]) {
                let mut points = vec![start];
                let mut at = start;
                let mut k = first;
                loop {
                    used[k] = true;
                    let s = self.segments[k];
                    at = if s[0] == at { s[1] } else { s[0] };
                    points.push(at);
                    match next[at as usize].iter().find(|(_, j)| !used[*j]) {
                        Some(&(_, j)) if at != start => k = j,
                        _ => break,
                    }
                }
                let closed = at == start && points.len() > 2;
                if closed {
                    points.pop();
                }
                out.push(Polyline { points, closed });
            }
        }
        out
    }
}

/// The margin line of the tooth `fdi` on the scan (`vertices`, `tris`) with
/// the tooth labels `labels` (`0` = gingiva): empty when the tooth has no
/// vertex, or when every vertex is the tooth. Pure: nothing is stored.
pub fn margin_line(vertices: &[DVec3], tris: &[[u32; 3]], labels: &[u16], fdi: u16) -> MarginLine {
    if labels.len() != vertices.len() || tris.is_empty() {
        return MarginLine::default();
    }
    let indicator: Vec<f64> = labels
        .iter()
        .map(|&l| f64::from(u8::from(l == fdi)))
        .collect();
    let edges = unique_edges(tris);
    // a value of exactly 0 counts as positive, as in the cut of the crown
    let phi: Vec<f64> = smooth_scalar(&indicator, &edges, SMOOTH_ITER, 0.5)
        .into_iter()
        .map(|x| {
            let p = x - 0.5;
            if p.abs() < 1e-9 { 1e-9 } else { p }
        })
        .collect();
    let mut index: HashMap<(u32, u32), u32> = HashMap::new();
    let mut line = MarginLine::default();
    let mut point = |line: &mut MarginLine, a: u32, b: u32| -> u32 {
        let (p, q) = if a < b { (a, b) } else { (b, a) };
        *index.entry((p, q)).or_insert_with(|| {
            let (fp, fq) = (phi[p as usize], phi[q as usize]);
            let t = (fp / (fp - fq)).clamp(T_MIN, 1.0 - T_MIN);
            let (vp, vq) = (vertices[p as usize], vertices[q as usize]);
            line.points.push(vp + t * (vq - vp));
            (line.points.len() - 1) as u32
        })
    };
    for t in tris {
        let pos = t.map(|i| phi[i as usize] > 0.0);
        let npos = pos.iter().filter(|&&p| p).count();
        if npos == 0 || npos == 3 {
            continue;
        }
        // the lone vertex: the only positive, or the only negative one
        let lone = pos
            .iter()
            .position(|&p| p == (npos == 1))
            .unwrap_or_default();
        let (a, b, c) = (t[lone], t[(lone + 1) % 3], t[(lone + 2) % 3]);
        let pab = point(&mut line, a, b);
        let pac = point(&mut line, a, c);
        line.segments.push([pab, pac]);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manual::tests::bump;

    fn labels_of(r: &[f64], limit: f64) -> Vec<u16> {
        r.iter().map(|&d| if d < limit { 36 } else { 0 }).collect()
    }

    /// Triangles with vertices of both kinds: the edges the line crosses.
    fn crossed(t: &[[u32; 3]], labels: &[u16]) -> usize {
        t.iter()
            .filter(|t| {
                let k = t.iter().filter(|&&i| labels[i as usize] == 36).count();
                k == 1 || k == 2
            })
            .count()
    }

    #[test]
    fn the_margin_of_a_crown_is_one_closed_loop_round_its_labels() {
        let (v, t, r) = bump(5.5, 7.5);
        let labels = labels_of(&r, 5.6);
        let m = margin_line(&v, &t, &labels, 36);
        assert!(!m.is_empty());
        let lines = m.polylines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].closed);
        // the loop uses every point and every segment once
        assert_eq!(lines[0].points.len(), m.points.len());
        assert_eq!(lines[0].points.len(), m.segments.len());
        // one mesh step is 0.4 mm: seen from above, a point of the line is within
        // a step of both sides (the wall is steep, so the height is left out)
        let flat = |p: DVec3| DVec3::new(p.x, p.y, 0.0);
        let tooth: Vec<DVec3> = (0..v.len())
            .filter(|&i| labels[i] == 36)
            .map(|i| flat(v[i]))
            .collect();
        let gum: Vec<DVec3> = (0..v.len())
            .filter(|&i| labels[i] == 0)
            .map(|i| flat(v[i]))
            .collect();
        let near = |set: &[DVec3], p: DVec3| {
            set.iter()
                .map(|q| (*q - flat(p)).length())
                .fold(f64::MAX, f64::min)
        };
        for p in &m.points {
            assert!(near(&tooth, *p) < 0.9 && near(&gum, *p) < 0.9, "{p:?}");
        }
        // round the tooth: about 2 pi r long, one segment per crossed triangle
        let radial: Vec<f64> = m
            .points
            .iter()
            .map(|p| ((p.x - 20.0).powi(2) + (p.y - 20.0).powi(2)).sqrt())
            .collect();
        assert!(
            radial.iter().all(|&d| (5.0..6.2).contains(&d)),
            "{radial:?}"
        );
        // seen from above it is a circle of about the radius of the labels (the wall is
        // steep, so in 3D the line is longer)
        let ring = &lines[0].points;
        let planar: f64 = (0..ring.len())
            .map(|k| {
                let (a, b) = (
                    m.points[ring[k] as usize],
                    m.points[ring[(k + 1) % ring.len()] as usize],
                );
                (DVec3::new(a.x, a.y, 0.0) - DVec3::new(b.x, b.y, 0.0)).length()
            })
            .sum();
        let (lo, hi) = (
            2.0 * std::f64::consts::PI * 5.0,
            2.0 * std::f64::consts::PI * 6.2,
        );
        assert!((lo..hi).contains(&planar), "{planar}");
        assert!(m.length() >= planar);
        assert_eq!(m.segments.len(), crossed(&t, &labels));
    }

    #[test]
    fn two_separate_pieces_give_two_loops_and_nothing_gives_no_line() {
        let (v, t, r) = bump(5.5, 7.5);
        let split: Vec<u16> = (0..v.len())
            .map(|i| {
                let far = (v[i].x - 4.0).powi(2) + (v[i].y - 4.0).powi(2) < 4.0;
                if r[i] < 5.6 || far { 36 } else { 0 }
            })
            .collect();
        let m = margin_line(&v, &t, &split, 36);
        let lines = m.polylines();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.closed));
        // no vertex of the tooth, another tooth's label, all of it the tooth
        let none = vec![0u16; v.len()];
        let nothing = margin_line(&v, &t, &none, 36);
        assert!(nothing.is_empty() && nothing.length().is_sign_positive());
        assert!(margin_line(&v, &t, &labels_of(&r, 5.6), 37).is_empty());
        let all = vec![36u16; v.len()];
        assert!(margin_line(&v, &t, &all, 36).is_empty());
        assert!(margin_line(&v, &t, &none[1..], 36).is_empty());
    }

    #[test]
    fn a_region_on_the_border_of_the_scan_gives_an_open_chain() {
        let (v, t, _) = bump(5.5, 7.5);
        let corner: Vec<u16> = v
            .iter()
            .map(|p| if p.x < 7.0 && p.y < 7.0 { 36 } else { 0 })
            .collect();
        let lines = margin_line(&v, &t, &corner, 36).polylines();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].closed);
    }

    #[test]
    fn the_line_is_the_cut_of_the_crown() {
        // the points are the ones the crown is cut at: the same smoothing, the
        // same marching triangles, the same distance from the vertices
        let (v, t, r) = bump(5.5, 7.5);
        let labels = labels_of(&r, 5.6);
        let m = margin_line(&v, &t, &labels, 36);
        let edges = unique_edges(&t);
        let ind: Vec<f64> = labels
            .iter()
            .map(|&l| f64::from(u8::from(l == 36)))
            .collect();
        let phi: Vec<f64> = smooth_scalar(&ind, &edges, SMOOTH_ITER, 0.5)
            .into_iter()
            .map(|x| x - 0.5)
            .collect();
        let cut = crate::reconstruct::geom::cut_mesh(&v, &t, &phi, T_MIN);
        let mut a: Vec<[i64; 3]> = cut.vertices[v.len()..]
            .iter()
            .map(|p| [p.x, p.y, p.z].map(|c| (c * 1e6).round() as i64))
            .collect();
        let mut b: Vec<[i64; 3]> = m
            .points
            .iter()
            .map(|p| [p.x, p.y, p.z].map(|c| (c * 1e6).round() as i64))
            .collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
    }
}
