//! Ctrl+click of the brush: the crown around a vertex up to its margin
//! (`manual.fill_region`).
//!
//! As the plugin: a ball around the click (the typical mesio-distal
//! diameter plus a margin) is segmented in two classes by a random walker
//! (Grady 2006) whose edge weights come from the folds of the surface
//! (Lai et al. 2009), seeded by a disk of the typical diameter around the
//! click as the tooth, by the gingiva well below the typical crown, at the
//! edge of the ball and on the open border, and by the other teeth as they
//! are labelled. A second pass is centred on the crown just found.
//!
//! Deviation from the plugin: the walker runs on the face graph of the scan
//! at full resolution instead of on the face graph of the simplified scan,
//! and a vertex is the tooth when every face around it is (the plugin
//! transfers the labels from the simplified faces to the vertices).

use std::collections::HashMap;

use dc_case::Fdi;
use dc_geom::sparse::{CsrMatrix, PcgParams, dirichlet_solve};
use glam::DVec3;

use super::Scan;
use crate::anatomy;

/// Parameters of the fill (the constants of the plugin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillParams {
    /// Ball around the click: the typical diameter plus this (mm).
    pub ball_extra_mm: f64,
    /// Disk around the click, as a share of the typical diameter.
    pub seed_share: f64,
    /// How far below the click the disk reaches (mm).
    pub depth_mm: f64,
    /// Gingiva starts this far (mm) below the top of the typical crown.
    pub gum_extra_mm: f64,
    /// Weight of the convex folds.
    pub eta_convex: f64,
    /// Sharpness of the fold weights.
    pub sigma: f64,
}

impl Default for FillParams {
    fn default() -> Self {
        Self {
            ball_extra_mm: 4.0,
            seed_share: 0.4,
            depth_mm: 3.5,
            gum_extra_mm: 1.5,
            eta_convex: 0.2,
            sigma: 1.0,
        }
    }
}

/// The part of the scan inside the ball, as the dual (face) graph of Lai et
/// al. 2009.
struct Patch {
    /// Global index of every local vertex.
    global: Vec<u32>,
    /// Faces fully inside the patch (local vertex indices).
    faces: Vec<[u32; 3]>,
    /// Faces sharing an edge, with the weight of the transition.
    edges: Vec<(u32, u32, f64)>,
    /// Faces with an edge that has no neighbour in the patch.
    border: Vec<bool>,
}

fn components_from(start: u32, member: &[bool], scan: &Scan<'_>) -> Vec<u32> {
    let mut seen = vec![false; member.len()];
    seen[start as usize] = true;
    let mut stack = vec![start];
    let mut out = Vec::new();
    while let Some(u) = stack.pop() {
        out.push(u);
        for &v in scan.adjacency().neighbors(u) {
            if member[v as usize] && !seen[v as usize] {
                seen[v as usize] = true;
                stack.push(v);
            }
        }
    }
    out.sort_unstable();
    out
}

/// The patch of the scan within `radius` of `centre` that is connected to
/// `v0`, with the fold weights between its faces (`segmentation.face_graph`:
/// `d = eta (1 - cos(dihedral)) / mean`, `eta` 1 on concave edges and
/// `eta_convex` on convex ones, `w = |e| exp(-d / sigma)`).
fn patch(scan: &Scan<'_>, centre: DVec3, radius: f64, v0: u32, p: &FillParams) -> Option<Patch> {
    let n = scan.vertices.len();
    let mut near = vec![false; n];
    for (i, _) in scan.within(centre, radius) {
        near[i as usize] = true;
    }
    near[v0 as usize] = true;
    let global = components_from(v0, &near, scan);
    let mut loc = vec![u32::MAX; n];
    for (k, &g) in global.iter().enumerate() {
        loc[g as usize] = k as u32;
    }
    let faces: Vec<[u32; 3]> = scan
        .tris
        .iter()
        .filter(|t| t.iter().all(|&i| loc[i as usize] != u32::MAX))
        .map(|t| t.map(|i| loc[i as usize]))
        .collect();
    if faces.len() < 10 {
        return None;
    }
    let v = |l: u32| scan.vertices[global[l as usize] as usize];
    let normal_centroid: Vec<(DVec3, DVec3)> = faces
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(v);
            ((b - a).cross(c - a).normalize_or_zero(), (a + b + c) / 3.0)
        })
        .collect();
    // edge -> the faces on it
    let mut by_edge: HashMap<(u32, u32), [Option<u32>; 2]> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            let slot = by_edge.entry((a.min(b), a.max(b))).or_insert([None, None]);
            if slot[0].is_none() {
                slot[0] = Some(fi as u32);
            } else if slot[1].is_none() {
                slot[1] = Some(fi as u32);
            }
        }
    }
    let mut keys: Vec<(u32, u32)> = by_edge.keys().copied().collect();
    keys.sort_unstable();
    let mut border = vec![false; faces.len()];
    // folds across the edges that have two faces
    let mut folds: Vec<(u32, u32, f64, f64)> = Vec::new();
    for &(a, b) in &keys {
        match by_edge[&(a, b)] {
            [Some(f1), Some(f2)] => {
                let ((n1, c1), (n2, c2)) =
                    (normal_centroid[f1 as usize], normal_centroid[f2 as usize]);
                let cos = n1.dot(n2).clamp(-1.0, 1.0);
                let concave = (c2 - c1).dot(n1 - n2) > 0.0;
                let eta = if concave { 1.0 } else { p.eta_convex };
                folds.push((f1, f2, eta * (1.0 - cos), (v(a) - v(b)).length()));
            }
            [Some(f), None] => border[f as usize] = true,
            _ => {}
        }
    }
    let mean = if folds.is_empty() {
        1.0
    } else {
        folds.iter().map(|e| e.2).sum::<f64>() / folds.len() as f64
    }
    .max(1e-12);
    let edges = folds
        .into_iter()
        .map(|(f1, f2, d, len)| {
            let w = (len * (-(d / mean) / p.sigma).exp()).max(1e-8 * len);
            (f1, f2, w)
        })
        .collect();
    Some(Patch {
        global,
        faces,
        edges,
        border,
    })
}

/// The crown of tooth `value` around vertex `v0` (`_crown_fill`): global
/// vertex indices, only those labelled the clicked tooth, the gingiva or
/// `value`.
fn crown_fill(
    scan: &Scan<'_>,
    labels: &[u16],
    v0: u32,
    value: Fdi,
    up: DVec3,
    p: &FillParams,
) -> Vec<u32> {
    let clicked = labels[v0 as usize];
    let want = u16::from(value.get());
    let md = anatomy::typical_diameters(value).0;
    let crown_len = anatomy::crown_root_length(value).0;
    let mut centre = scan.vertices[v0 as usize];
    let mut result: Option<(Patch, Vec<bool>)> = None;
    for pass in 0..2 {
        let radius = md + p.ball_extra_mm;
        let Some(patch) = patch(scan, centre, radius, v0, p) else {
            return Vec::new();
        };
        let nf = patch.faces.len();
        let vpos = |l: u32| scan.vertices[patch.global[l as usize] as usize];
        let centroid: Vec<DVec3> = patch
            .faces
            .iter()
            .map(|f| (vpos(f[0]) + vpos(f[1]) + vpos(f[2])) / 3.0)
            .collect();
        let h: Vec<f64> = centroid.iter().map(|c| (*c - centre).dot(up)).collect();
        let dc: Vec<f64> = centroid.iter().map(|c| (*c - centre).length()).collect();
        // the label of a face: the one of its vertex nearest to the centroid
        let other: Vec<bool> = (0..nf)
            .map(|f| {
                let near = patch.faces[f]
                    .iter()
                    .copied()
                    .min_by(|&a, &b| {
                        (vpos(a) - centroid[f])
                            .length()
                            .total_cmp(&(vpos(b) - centroid[f]).length())
                    })
                    .map_or(0, |l| labels[patch.global[l as usize] as usize]);
                near != clicked && near != want && near != 0
            })
            .collect();
        let disk: Vec<bool> = dc.iter().map(|&d| d < p.seed_share * md).collect();
        let top = (0..nf)
            .filter(|&f| disk[f])
            .map(|f| h[f])
            .fold(f64::NEG_INFINITY, f64::max);
        let top = if top.is_finite() { top } else { 0.0 };
        // seed: -1 free, 0 gingiva, 1 tooth
        let mut seed = vec![-1i8; nf];
        for f in 0..nf {
            if h[f] < top - (crown_len + p.gum_extra_mm)
                || dc[f] > radius - 0.8
                || (patch.border[f] && dc[f] > 0.5 * radius)
                || other[f]
            {
                seed[f] = 0;
            }
        }
        let mut core: Vec<bool> = (0..nf)
            .map(|f| disk[f] && h[f] > -p.depth_mm && !other[f])
            .collect();
        if core.iter().filter(|&&c| c).count() < 4 {
            core = vec![false; nf];
            let mut order: Vec<usize> = (0..nf).collect();
            order.sort_by(|&a, &b| dc[a].total_cmp(&dc[b]));
            for &f in order.iter().take(6) {
                core[f] = true;
            }
        }
        for f in 0..nf {
            if core[f] {
                seed[f] = 1;
            }
        }
        if !seed.contains(&0) {
            return Vec::new();
        }
        // random walker: harmonic function with 1 on the tooth seeds, 0 on the gingiva seeds
        let mut triplets = Vec::with_capacity(patch.edges.len() * 4 + nf);
        let mut diag = vec![0.0f64; nf];
        for &(a, b, w) in &patch.edges {
            let (a, b) = (a as usize, b as usize);
            triplets.push((a, b, -w));
            triplets.push((b, a, -w));
            diag[a] += w;
            diag[b] += w;
        }
        for (f, d) in diag.iter().enumerate() {
            triplets.push((f, f, *d));
        }
        let Ok(a) = CsrMatrix::from_triplets(nf, &triplets) else {
            return Vec::new();
        };
        let fixed: Vec<usize> = (0..nf).filter(|&f| seed[f] >= 0).collect();
        let field: Vec<f64> = seed.iter().map(|&s| f64::from(s.max(0))).collect();
        let params = PcgParams {
            max_iterations: 3000,
            tolerance: 1e-4,
        };
        let Ok(sol) = dirichlet_solve(&a, &fixed, &[field], None, &params) else {
            return Vec::new();
        };
        // faces to vertices as the plugin's transfer does for a vertex on a boundary: a
        // vertex is the tooth only when every face around it is (ties go to the gingiva)
        let mut all_in = vec![true; patch.global.len()];
        let mut any = vec![false; patch.global.len()];
        for (f, tri) in patch.faces.iter().enumerate() {
            let inside = sol.x[0][f] > 0.5;
            for &l in tri {
                any[l as usize] = true;
                all_in[l as usize] &= inside;
            }
        }
        let vin: Vec<bool> = (0..patch.global.len())
            .map(|l| any[l] && all_in[l])
            .collect();
        if pass == 0 {
            // second pass centred on the crown just found (a click on a cusp is off the
            // centre of a molar)
            let pts: Vec<DVec3> = (0..vin.len())
                .filter(|&l| vin[l])
                .map(|l| vpos(l as u32))
                .collect();
            if pts.len() <= 10 {
                result = Some((patch, vin));
                break;
            }
            let hp: Vec<f64> = pts.iter().map(|q| (*q - centre).dot(up)).collect();
            let hmax = hp.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let (sum, cnt) = pts
                .iter()
                .zip(&hp)
                .filter(|(_, h)| **h > hmax - 0.5 * crown_len)
                .fold((DVec3::ZERO, 0usize), |(s, c), (q, _)| (s + *q, c + 1));
            let mean = sum / cnt.max(1) as f64;
            let nearest = pts
                .iter()
                .copied()
                .min_by(|a, b| (*a - mean).length().total_cmp(&(*b - mean).length()))
                .unwrap_or(mean);
            let sh = nearest - centre;
            centre = if sh.length() > 0.3 * md {
                centre + sh * (0.3 * md / sh.length())
            } else {
                nearest
            };
        }
        result = Some((patch, vin));
    }
    let Some((patch, vin)) = result else {
        return Vec::new();
    };
    // the piece of the crown that holds the click (else the largest one)
    let n = scan.vertices.len();
    let mut member = vec![false; n];
    for (l, &g) in patch.global.iter().enumerate() {
        member[g as usize] = vin[l];
    }
    let idx = if member[v0 as usize] {
        components_from(v0, &member, scan)
    } else {
        let mut best: Vec<u32> = Vec::new();
        let mut done = vec![false; n];
        for &g in &patch.global {
            if member[g as usize] && !done[g as usize] {
                let comp = components_from(g, &member, scan);
                for &c in &comp {
                    done[c as usize] = true;
                }
                if comp.len() > best.len() {
                    best = comp;
                }
            }
        }
        best
    };
    idx.into_iter()
        .filter(|&g| {
            let l = labels[g as usize];
            l == clicked || l == 0 || l == want
        })
        .collect()
}

/// Vertices that become `value` when the user fills from `v0`
/// (`manual.fill_region`); `up` is the occlusal direction in the coordinates
/// of the scan.
///
/// `value` is a tooth: the crown around the click up to its margin, never
/// taking the vertices of another tooth. `value` is `0` (gingiva): the part
/// of the clicked tooth's region beyond its crown (a strip of gingiva the
/// tooth took), found by filling its crown from the top. Sorted; empty when
/// nothing changes or `value` is not a tooth number.
pub fn fill_region(
    scan: &Scan<'_>,
    labels: &[u16],
    v0: u32,
    value: u16,
    up: DVec3,
    params: &FillParams,
) -> Vec<u32> {
    let up = up.normalize_or_zero();
    if up == DVec3::ZERO || labels.len() != scan.vertices.len() || v0 as usize >= labels.len() {
        return Vec::new();
    }
    if value != 0 {
        let Some(fdi) = u8::try_from(value).ok().and_then(|n| Fdi::new(n).ok()) else {
            return Vec::new();
        };
        let mut out = crown_fill(scan, labels, v0, fdi, up, params);
        out.sort_unstable();
        return out;
    }
    let clicked = labels[v0 as usize];
    let Some(fdi) = u8::try_from(clicked).ok().and_then(|n| Fdi::new(n).ok()) else {
        return Vec::new();
    };
    // the vertex of the top of the region nearest to its centre
    let region: Vec<u32> = (0..labels.len() as u32)
        .filter(|&i| labels[i as usize] == clicked)
        .collect();
    let hmax = region
        .iter()
        .map(|&i| scan.vertices[i as usize].dot(up))
        .fold(f64::NEG_INFINITY, f64::max);
    let top: Vec<u32> = region
        .iter()
        .copied()
        .filter(|&i| scan.vertices[i as usize].dot(up) >= hmax - 1.5)
        .collect();
    let mean = top
        .iter()
        .map(|&i| scan.vertices[i as usize])
        .sum::<DVec3>()
        / top.len() as f64;
    let Some(&vt) = top.iter().min_by(|&&a, &&b| {
        (scan.vertices[a as usize] - mean)
            .length()
            .total_cmp(&(scan.vertices[b as usize] - mean).length())
    }) else {
        return Vec::new();
    };
    let mut crown = vec![false; labels.len()];
    for i in crown_fill(scan, labels, vt, fdi, up, params) {
        crown[i as usize] = true;
    }
    if crown[v0 as usize] {
        return Vec::new(); // the click is on the crown itself
    }
    let rest: Vec<bool> = labels
        .iter()
        .enumerate()
        .map(|(i, &l)| l == clicked && !crown[i])
        .collect();
    components_from(v0, &rest, scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manual::tests::grid;

    /// A gingival plane (40 x 40 mm, 0.4 mm apart) with a tooth-shaped bump of
    /// radius `r_tooth` and height `h`: a rounded rim and a vertical wall that
    /// meets the gingiva at a sharp concave fold, as a crown does. `+Z` is up.
    fn plateau(r_tooth: f64, h: f64) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<f64>) {
        let (mut v, t) = grid(101);
        let mut r = Vec::new();
        for p in &mut v {
            p.x *= 0.4;
            p.y *= 0.4;
            let d = ((p.x - 20.0).powi(2) + (p.y - 20.0).powi(2)).sqrt();
            p.z = if d < r_tooth {
                h * (1.0 - (d / r_tooth).powi(6)).sqrt()
            } else {
                0.0
            };
            r.push(d);
        }
        (v, t, r)
    }

    fn iou(got: &[u32], truth: &[u32]) -> f64 {
        let inter = got.iter().filter(|g| truth.contains(g)).count();
        inter as f64 / (got.len() + truth.len() - inter) as f64
    }

    #[test]
    fn the_fill_follows_the_fold_at_the_foot_of_the_crown() {
        let (v, t, r) = plateau(5.5, 7.5);
        let scan = Scan::new(&v, &t).unwrap();
        let truth: Vec<u32> = (0..v.len() as u32)
            .filter(|&i| r[i as usize] < 5.5)
            .collect();
        // nothing is labelled yet; a click near the middle of the top
        let labels = vec![0u16; v.len()];
        let v0 = scan.nearest(DVec3::new(21.0, 19.0, 7.5)).unwrap();
        let got = fill_region(&scan, &labels, v0, 36, DVec3::Z, &FillParams::default());
        assert!(!got.is_empty());
        assert!(iou(&got, &truth) > 0.9, "IoU {}", iou(&got, &truth));
        // the other teeth are never taken
        let mut other = labels.clone();
        for &i in &truth {
            if r[i as usize] > 4.0 {
                other[i as usize] = 35; // the wall of the plateau belongs to a neighbour
            }
        }
        let got = fill_region(&scan, &other, v0, 36, DVec3::Z, &FillParams::default());
        assert!(got.iter().all(|&i| other[i as usize] != 35));
        // a number that is not a tooth, or no direction, finds nothing
        assert!(fill_region(&scan, &labels, v0, 99, DVec3::Z, &FillParams::default()).is_empty());
        assert!(
            fill_region(&scan, &labels, v0, 36, DVec3::ZERO, &FillParams::default()).is_empty()
        );
    }

    #[test]
    fn filling_with_the_gingiva_takes_back_the_strip_the_tooth_took() {
        let (v, t, r) = plateau(5.5, 7.5);
        let scan = Scan::new(&v, &t).unwrap();
        // the tooth label covers the crown and a 2 mm strip of gingiva around its foot
        let labels: Vec<u16> = r.iter().map(|&d| if d < 7.6 { 36 } else { 0 }).collect();
        let strip: Vec<u32> = (0..v.len() as u32)
            .filter(|&i| r[i as usize] > 6.4 && r[i as usize] < 7.6)
            .collect();
        let click = strip[strip.len() / 3];
        let got = fill_region(&scan, &labels, click, 0, DVec3::Z, &FillParams::default());
        assert!(
            got.len() as f64 > 0.8 * strip.len() as f64,
            "{} of {}",
            got.len(),
            strip.len()
        );
        assert!(got.iter().all(|&i| r[i as usize] > 5.5), "the crown stays");
        // a click on the crown itself, or on the gingiva, changes nothing
        let top = scan.nearest(DVec3::new(20.0, 20.0, 7.5)).unwrap();
        assert!(fill_region(&scan, &labels, top, 0, DVec3::Z, &FillParams::default()).is_empty());
        let gum = (0..v.len() as u32)
            .find(|&i| labels[i as usize] == 0)
            .unwrap();
        assert!(fill_region(&scan, &labels, gum, 0, DVec3::Z, &FillParams::default()).is_empty());
    }
}
