//! Can a closed tooth be built from the labelled region of a tooth?
//! (`manual.check_region`.)

use std::collections::HashMap;

use dc_case::Fdi;
use glam::{DVec2, DVec3};
use serde::Serialize;

use super::Scan;
use crate::anatomy;

/// A crown region lower than this (mm) is gingiva, not a crown.
pub const MIN_HEIGHT_MM: f64 = 1.5;
/// Narrower than this share of the typical mesio-distal diameter: too small.
pub const MIN_WIDTH: f64 = 0.4;
/// Wider than this share: two teeth, or the gingiva.
pub const MAX_WIDTH: f64 = 2.2;
/// Area below this share of the squared typical diameter: too small.
pub const MIN_AREA: f64 = 0.35;
/// Share of the outline of the region on the open border of the scan above
/// which it is not a crown.
pub const BORDER_ERROR: f64 = 0.25;
/// A hole (gingiva inside the crown) with a longer outline (mm) is reported.
pub const HOLE_MM: f64 = 3.0;
/// A hole longer than this share of the outline: a ring of surface, not a crown.
pub const HOLE_ERROR: f64 = 0.4;

/// Result of [`check_region`].
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct RegionReport {
    /// Reasons why no tooth can be built from the region.
    pub errors: Vec<String>,
    /// Things to check; the tooth can be built.
    pub warnings: Vec<String>,
    /// Vertices with the label.
    pub vertices: usize,
    /// Area of the faces fully inside the region (mm²).
    pub area_mm2: f64,
    /// Width of the main piece along its principal axis (mm).
    pub width_mm: f64,
    /// Height of the main piece along the occlusal direction (mm).
    pub height_mm: f64,
    /// Connected pieces of the region.
    pub parts: usize,
}

impl RegionReport {
    /// One line for the user: errors first, then warnings.
    pub fn message(&self, fdi: Fdi) -> String {
        if !self.errors.is_empty() {
            return format!("Tooth {fdi}: {}", self.errors.join("; "));
        }
        let mut txt = format!(
            "Tooth {fdi}: {} vertices, {:.0} mm2, {:.1} mm wide",
            self.vertices, self.area_mm2, self.width_mm
        );
        if !self.warnings.is_empty() {
            txt.push_str(&format!(". Warning: {}", self.warnings.join("; ")));
        }
        txt
    }
}

fn round(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k
}

/// Edges used by exactly one of `tris` (key `(a, b)`, `a < b`).
fn border_edges(tris: impl Iterator<Item = [u32; 3]>) -> Vec<(u32, u32)> {
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(u32, u32)> = count
        .into_iter()
        .filter(|&(_, c)| c == 1)
        .map(|(k, _)| k)
        .collect();
    out.sort_unstable();
    out
}

/// Checks the region of `fdi` in `labels`. `arch_vertices` are the vertices
/// of the scan in arch coordinates (`+Z` occlusal), for the width and the
/// height.
pub fn check_region(
    scan: &Scan<'_>,
    labels: &[u16],
    fdi: Fdi,
    arch_vertices: &[DVec3],
) -> RegionReport {
    let want = u16::from(fdi.get());
    let n = scan.vertices.len();
    let sel: Vec<bool> = labels.iter().map(|&l| l == want).collect();
    let mut out = RegionReport {
        vertices: sel.iter().filter(|&&s| s).count(),
        ..RegionReport::default()
    };
    if out.vertices == 0 {
        out.errors.push(format!(
            "tooth {fdi} has no region on the scan: paint it or give it a selection"
        ));
        return out;
    }
    let full: Vec<[u32; 3]> = scan
        .tris
        .iter()
        .filter(|t| t.iter().all(|&i| sel[i as usize]))
        .copied()
        .collect();
    let area: f64 = full
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| scan.vertices[i as usize]);
            0.5 * (b - a).cross(c - a).length()
        })
        .sum();
    out.area_mm2 = round(area, 1);
    let md = anatomy::typical_diameters(fdi).0;

    // the pieces of the region
    let mut comp = vec![u32::MAX; n];
    let mut sizes: Vec<usize> = Vec::new();
    for s in 0..n {
        if !sel[s] || comp[s] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        let mut stack = vec![s as u32];
        comp[s] = id;
        let mut size = 0;
        while let Some(u) = stack.pop() {
            size += 1;
            for &v in scan.adjacency().neighbors(u) {
                if sel[v as usize] && comp[v as usize] == u32::MAX {
                    comp[v as usize] = id;
                    stack.push(v);
                }
            }
        }
        sizes.push(size);
    }
    out.parts = sizes.len();
    // the largest piece (the first one on ties)
    let main_id = sizes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))
        .map_or(0, |(i, _)| i as u32);
    let share = sizes[main_id as usize] as f64 / out.vertices.max(1) as f64;
    if share < 0.9 {
        out.warnings.push(format!(
            "the region is split in {} parts: only the largest ({:.0}%) becomes the tooth (clean it or paint it again)",
            sizes.len(),
            share * 100.0
        ));
    }
    let in_main = |i: u32| sel[i as usize] && comp[i as usize] == main_id;

    // height and width of the main piece in arch coordinates
    let pts: Vec<DVec3> = (0..n as u32)
        .filter(|&i| in_main(i))
        .map(|i| {
            arch_vertices
                .get(i as usize)
                .copied()
                .unwrap_or(scan.vertices[i as usize])
        })
        .collect();
    let (zmin, zmax) = pts
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.z), hi.max(p.z))
        });
    let height = if pts.is_empty() { 0.0 } else { zmax - zmin };
    let width = if pts.len() >= 3 {
        let mean = pts.iter().map(|p| DVec2::new(p.x, p.y)).sum::<DVec2>() / pts.len() as f64;
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for p in &pts {
            let d = DVec2::new(p.x, p.y) - mean;
            sxx += d.x * d.x;
            sxy += d.x * d.y;
            syy += d.y * d.y;
        }
        // principal axis of the 2x2 covariance
        let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let axis = DVec2::new(theta.cos(), theta.sin());
        let (lo, hi) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                let t = (DVec2::new(p.x, p.y) - mean).dot(axis);
                (lo.min(t), hi.max(t))
            });
        hi - lo
    } else {
        0.0
    };
    out.width_mm = round(width, 2);
    out.height_mm = round(height, 2);
    if out.area_mm2 < MIN_AREA * md * md || width < MIN_WIDTH * md {
        out.errors.push(format!(
            "region too small for a crown ({:.0} mm2, {:.1} mm wide; tooth {fdi} is about {md:.1} mm wide): enlarge it or paint it again",
            out.area_mm2, width
        ));
    } else if height < MIN_HEIGHT_MM {
        out.errors.push(format!(
            "region too low ({height:.1} mm): it looks like gingiva, not a crown"
        ));
    }
    if width > MAX_WIDTH * md {
        out.warnings.push(format!(
            "region very wide ({width:.1} mm, typical tooth {md:.1} mm): it may include two teeth or some gingiva"
        ));
    }

    // the open border of the scan: its share of the outline of the region (by length)
    let scan_border = border_edges(scan.tris.iter().copied());
    let mut on_border = vec![false; n];
    for &(a, b) in &scan_border {
        on_border[a as usize] = true;
        on_border[b as usize] = true;
    }
    let border_vertices = (0..n).filter(|&i| sel[i] && on_border[i]).count();
    if border_vertices > 0 && !full.is_empty() {
        let region_border = border_edges(full.iter().copied());
        let on_scan_border: std::collections::HashSet<(u32, u32)> =
            scan_border.iter().copied().collect();
        let len =
            |&(a, b): &(u32, u32)| (scan.vertices[a as usize] - scan.vertices[b as usize]).length();
        let total: f64 = region_border.iter().map(len).sum();
        let shared: f64 = region_border
            .iter()
            .filter(|e| on_scan_border.contains(e))
            .map(len)
            .sum();
        let bshare = shared / total.max(1e-12);
        if bshare > BORDER_ERROR {
            out.errors.push(format!(
                "the region touches the border of the scan along {:.0}% of its outline: the crown is not surrounded by gingiva (incomplete scan or wrong region)",
                bshare * 100.0
            ));
        } else {
            out.warnings.push(format!(
                "the region touches the border of the scan ({border_vertices} vertices): the missing part will be rebuilt"
            ));
        }
    }

    // holes: gingiva inside the crown (more than one outline of the main piece). The
    // outlines are the boundary edges of the main piece, grouped by the connected piece of
    // the rest of the scan that lies across them (a hole is an enclosed piece; the outer
    // outline is the longest).
    let outside: Vec<bool> = (0..n as u32).map(|i| !in_main(i)).collect();
    let mut piece = vec![u32::MAX; n];
    let mut pieces = 0u32;
    for s in 0..n {
        if !outside[s] || piece[s] != u32::MAX {
            continue;
        }
        piece[s] = pieces;
        let mut stack = vec![s as u32];
        while let Some(u) = stack.pop() {
            for &v in scan.adjacency().neighbors(u) {
                if outside[v as usize] && piece[v as usize] == u32::MAX {
                    piece[v as usize] = pieces;
                    stack.push(v);
                }
            }
        }
        pieces += 1;
    }
    let main_tris = full
        .iter()
        .filter(|t| t.iter().all(|&i| in_main(i)))
        .copied();
    let boundary: std::collections::HashSet<(u32, u32)> =
        border_edges(main_tris).into_iter().collect();
    let edge_len =
        |a: u32, b: u32| (scan.vertices[a as usize] - scan.vertices[b as usize]).length();
    let mut contour: HashMap<u32, f64> = HashMap::new();
    for t in scan.tris {
        for k in 0..3 {
            let (a, b, w) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            if boundary.contains(&(a.min(b), a.max(b))) && !in_main(w) {
                *contour.entry(piece[w as usize]).or_insert(0.0) += edge_len(a, b);
            }
        }
    }
    // a boundary edge of the scan itself is part of the outer outline
    for &(a, b) in &scan_border {
        if boundary.contains(&(a, b)) {
            *contour.entry(u32::MAX).or_insert(0.0) += edge_len(a, b);
        }
    }
    let mut lens: Vec<f64> = contour.into_values().filter(|&l| l > 0.0).collect();
    lens.sort_by(|a, b| b.total_cmp(a));
    if lens.len() > 1 {
        let holes: Vec<f64> = lens[1..].iter().copied().filter(|&x| x > HOLE_MM).collect();
        let biggest = holes.iter().copied().fold(0.0, f64::max);
        if !holes.is_empty() && biggest > HOLE_ERROR * lens[0] {
            out.errors.push(format!(
                "the region has a hole as big as the crown (outline {biggest:.1} mm): it is not a crown; fill it with the brush or clean the labels"
            ));
        } else if !holes.is_empty() {
            out.warnings.push(format!(
                "the region has {} holes (gingiva inside the crown, outline up to {biggest:.1} mm), closed without anatomical shape: better fill them with the brush or clean the labels",
                holes.len()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manual::tests::bump;

    fn fdi() -> Fdi {
        Fdi::new(36).unwrap()
    }

    fn check(labels: &[u16], v: &[DVec3], t: &[[u32; 3]]) -> RegionReport {
        let scan = Scan::new(v, t).unwrap();
        check_region(&scan, labels, fdi(), v)
    }

    #[test]
    fn a_crown_on_the_gingiva_is_accepted() {
        let (v, t, r) = bump(5.5, 7.5);
        let labels: Vec<u16> = r.iter().map(|&d| if d < 5.6 { 36 } else { 0 }).collect();
        let rep = check(&labels, &v, &t);
        assert!(rep.errors.is_empty(), "{rep:?}");
        assert!(rep.warnings.is_empty(), "{rep:?}");
        assert_eq!(rep.parts, 1);
        assert!((rep.width_mm - 11.0).abs() < 0.6, "{}", rep.width_mm);
        assert!((rep.height_mm - 7.5).abs() < 0.1);
        assert!(rep.area_mm2 > 90.0);
        assert!(rep.message(fdi()).starts_with("Tooth 36: "));
    }

    #[test]
    fn a_region_that_cannot_be_a_crown_is_refused_with_the_reason() {
        let (v, t, r) = bump(5.5, 7.5);
        // nothing labelled
        let none = vec![0u16; v.len()];
        assert!(check(&none, &v, &t).errors[0].contains("no region"));
        // a speck
        let speck: Vec<u16> = r.iter().map(|&d| if d < 1.2 { 36 } else { 0 }).collect();
        assert!(check(&speck, &v, &t).errors[0].contains("too small"));
        // a flat disc of gingiva
        let flat: Vec<u16> = r
            .iter()
            .map(|&d| if d > 8.0 && d < 14.0 { 36 } else { 0 })
            .collect();
        let rep = check(&flat, &v, &t);
        assert!(rep.errors[0].contains("too low"), "{rep:?}");
        assert!(rep.message(fdi()).contains("gingiva"));
        // the corner of the scan: the outline lies on its open border
        let corner: Vec<u16> = v
            .iter()
            .map(|p| {
                if p.x < 7.0 && p.y < 7.0 && p.z >= 0.0 {
                    36
                } else {
                    0
                }
            })
            .collect();
        let rep = check(&corner, &v, &t);
        assert!(
            rep.errors.iter().any(|e| e.contains("border")),
            "border: {rep:?}"
        );
        // a ring: a hole as big as the crown
        let ring: Vec<u16> = r
            .iter()
            .map(|&d| if d > 4.5 && d < 5.5 { 36 } else { 0 })
            .collect();
        let rep = check(&ring, &v, &t);
        assert!(rep.errors.iter().any(|e| e.contains("hole")), "{rep:?}");
    }

    #[test]
    fn warnings_do_not_refuse_the_region() {
        let (v, t, r) = bump(5.5, 7.5);
        // the crown and a distant piece: split in two parts
        let split: Vec<u16> = (0..v.len())
            .map(|i| {
                let far = (v[i].x - 4.0).powi(2) + (v[i].y - 4.0).powi(2) < 4.0;
                if r[i] < 5.6 || far { 36 } else { 0 }
            })
            .collect();
        let rep = check(&split, &v, &t);
        assert!(rep.errors.is_empty(), "{rep:?}");
        assert_eq!(rep.parts, 2);
        assert!(rep.warnings.iter().any(|w| w.contains("split")), "{rep:?}");
        // a small hole in the crown is reported, not refused
        let holed: Vec<u16> = r
            .iter()
            .map(|&d| {
                if d < 5.6 && !(d > 2.0 && d < 2.8) {
                    36
                } else {
                    0
                }
            })
            .collect();
        let rep = check(&holed, &v, &t);
        assert!(rep.errors.is_empty(), "{rep:?}");
        assert!(rep.warnings.iter().any(|w| w.contains("hole")), "{rep:?}");
    }
}
