//! Labels of the full scan from the faces of the simplified mesh
//! (`case_teeth.transfer_labels`, `gpu_query.Mesh.nearest` / `near_ties`).
//!
//! The nearest face is exact: candidates from the [`Bvh`] (float32 boxes)
//! within the exact distance of its answer plus a margin, every candidate
//! measured again in float64 with the plugin's closest-point routine on
//! coordinates centred on the mesh box, the smallest squared distance
//! winning, ties to the smallest face index.

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};
use rayon::prelude::*;

use super::mesh;

/// Faces this near the nearest one are near-ties (mm).
pub(crate) const TIE_MM: f64 = 1e-5;
/// Margin (mm) of the candidate search beyond the exact distance.
const MARGIN_MM: f64 = 0.01;

/// Closest point of the triangle `(a, b, c)` to `p` (Ericson 5.1.5), with
/// the operations and the order of `gpu_query._closest_tri`.
fn closest_tri(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let (abx, aby, abz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (acx, acy, acz) = (c.x - a.x, c.y - a.y, c.z - a.z);
    let (apx, apy, apz) = (p.x - a.x, p.y - a.y, p.z - a.z);
    let d1 = abx * apx + aby * apy + abz * apz;
    let d2 = acx * apx + acy * apy + acz * apz;
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let (bpx, bpy, bpz) = (p.x - b.x, p.y - b.y, p.z - b.z);
    let d3 = abx * bpx + aby * bpy + abz * bpz;
    let d4 = acx * bpx + acy * bpy + acz * bpz;
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return DVec3::new(a.x + v * abx, a.y + v * aby, a.z + v * abz);
    }
    let (cpx, cpy, cpz) = (p.x - c.x, p.y - c.y, p.z - c.z);
    let d5 = abx * cpx + aby * cpy + abz * cpz;
    let d6 = acx * cpx + acy * cpy + acz * cpz;
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return DVec3::new(a.x + w * acx, a.y + w * acy, a.z + w * acz);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return DVec3::new(
            b.x + w * (c.x - b.x),
            b.y + w * (c.y - b.y),
            b.z + w * (c.z - b.z),
        );
    }
    let s = va + vb + vc;
    let den = 1.0 / s;
    let (v, w) = (vb * den, vc * den);
    if s > 0.0 && v >= 0.0 && w >= 0.0 && v + w <= 1.0 {
        return DVec3::new(
            a.x + abx * v + acx * w,
            a.y + aby * v + acy * w,
            a.z + abz * v + acz * w,
        );
    }
    // zero area or a rounding at the border: nearest point of the edges,
    // ties to the first edge
    let (mut o, mut d) = seg_closest(p, a, b);
    for (x, y) in [(b, c), (c, a)] {
        let (o2, d2) = seg_closest(p, x, y);
        if d2 < d {
            (o, d) = (o2, d2);
        }
    }
    o
}

/// Closest point of the segment `a-b` to `p` and its squared distance
/// (`gpu_query._seg_closest`).
fn seg_closest(p: DVec3, a: DVec3, b: DVec3) -> (DVec3, f64) {
    let (ex, ey, ez) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let l2 = ex * ex + ey * ey + ez * ez;
    let s = if l2 > 0.0 {
        ((p.x - a.x) * ex + (p.y - a.y) * ey + (p.z - a.z) * ez) / l2
    } else {
        0.0
    };
    let s = s.clamp(0.0, 1.0);
    let o = DVec3::new(a.x + s * ex, a.y + s * ey, a.z + s * ez);
    (o, d2(o, p))
}

/// Squared distance in the order of `gpu_query._d2`.
fn d2(a: DVec3, b: DVec3) -> f64 {
    let e = a - b;
    e.x * e.x + e.y * e.y + e.z * e.z
}

/// Simplified mesh prepared for exact nearest-face queries.
struct Target {
    /// Centre of the box of the used vertices.
    c: DVec3,
    /// Centred triangle corners.
    w: Vec<[DVec3; 3]>,
    bvh: Bvh,
    /// Faces around every vertex (CSR, by face index).
    vf_start: Vec<usize>,
    vf: Vec<u32>,
}

impl Target {
    fn new(v: &[DVec3], t: &[[u32; 3]]) -> Result<Self, String> {
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for f in t {
            for &i in f {
                lo = lo.min(v[i as usize]);
                hi = hi.max(v[i as usize]);
            }
        }
        let c = if t.is_empty() {
            DVec3::ZERO
        } else {
            0.5 * (lo + hi)
        };
        let w = t.iter().map(|f| f.map(|i| v[i as usize] - c)).collect();
        let positions: Vec<Vec3> = v.iter().map(|p| p.as_vec3()).collect();
        let bvh = Bvh::build(&TriMesh::new(positions, t.to_vec())).map_err(|e| e.to_string())?;
        let mut vf_start = vec![0usize; v.len() + 1];
        for f in t {
            for &i in f {
                vf_start[i as usize + 1] += 1;
            }
        }
        for i in 0..v.len() {
            vf_start[i + 1] += vf_start[i];
        }
        let mut fill = vf_start.clone();
        let mut vf = vec![0u32; vf_start[v.len()]];
        for (fi, f) in t.iter().enumerate() {
            for &i in f {
                vf[fill[i as usize]] = fi as u32;
                fill[i as usize] += 1;
            }
        }
        Ok(Self {
            c,
            w,
            bvh,
            vf_start,
            vf,
        })
    }

    /// Exact squared distance of the centred query `q` to face `f`.
    fn d2(&self, q: DVec3, f: usize) -> f64 {
        let [a, b, c] = self.w[f];
        d2(closest_tri(q, a, b, c), q)
    }

    /// Nearest face of `p` (smallest exact squared distance, then index).
    fn nearest(&self, p: DVec3) -> Option<usize> {
        let hit = self.bvh.closest_point(p.as_vec3(), f32::INFINITY)?;
        let q = p - self.c;
        let d0 = self.d2(q, hit.face as usize).sqrt();
        let rad = d0 * (1.0 + 1e-6) + MARGIN_MM;
        let mut best: Option<(f64, usize)> = None;
        for f in self.bvh.overlap_sphere(p.as_vec3(), rad as f32) {
            let f = f as usize;
            let d = self.d2(q, f);
            if !d.is_finite() {
                continue;
            }
            // candidates come by face index: strict < keeps the smallest on ties
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, f));
            }
        }
        best.map(|b| b.1).or(Some(hit.face as usize))
    }

    /// Faces sharing a corner with `face` whose exact distance to `p` is at
    /// most that of `face` + `eps` (by index), when there are at least two.
    fn near_ties(&self, t: &[[u32; 3]], p: DVec3, face: usize, eps: f64) -> Vec<usize> {
        let q = p - self.c;
        let d0 = self.d2(q, face).sqrt();
        let mut cand: Vec<usize> = t[face]
            .iter()
            .flat_map(|&i| {
                self.vf[self.vf_start[i as usize]..self.vf_start[i as usize + 1]]
                    .iter()
                    .map(|&f| f as usize)
            })
            .collect();
        cand.sort_unstable();
        cand.dedup();
        cand.retain(|&f| self.d2(q, f).sqrt() <= d0 + eps);
        if cand.len() >= 2 { cand } else { Vec::new() }
    }
}

/// Labels of the vertices `v` (edges `e`) of the full scan from the labels
/// `flab` of the faces of the simplified mesh `(vd, td)`: the label of the
/// nearest face; where faces of another label are within `eps` of the
/// nearest one, a tie with the gingiva goes to the gingiva, a tie between
/// teeth to the tooth held by most of the vertex's neighbours, then to the
/// label of the smallest face.
pub(crate) fn transfer_labels(
    v: &[DVec3],
    e: &[[u32; 2]],
    vd: &[DVec3],
    td: &[[u32; 3]],
    flab: &[u16],
    eps: f64,
) -> Result<Vec<u16>, String> {
    let target = Target::new(vd, td)?;
    let face: Vec<Option<usize>> = v.par_iter().map(|&p| target.nearest(p)).collect();
    let mut vl: Vec<u16> = face.iter().map(|f| flab[f.unwrap_or(0)]).collect();
    // only a nearest face with a corner on a face of another label can be tied
    let mut lo = vec![u16::MAX; vd.len()];
    let mut hi = vec![0u16; vd.len()];
    for (f, &l) in td.iter().zip(flab) {
        for &i in f {
            lo[i as usize] = lo[i as usize].min(l);
            hi[i as usize] = hi[i as usize].max(l);
        }
    }
    let mixed: Vec<bool> = td
        .iter()
        .map(|f| f.iter().any(|&i| lo[i as usize] != hi[i as usize]))
        .collect();
    // (row, [(label, smallest face of the label)]) of the rows tied between labels
    let ties: Vec<(usize, Vec<(u16, usize)>)> = face
        .par_iter()
        .enumerate()
        .filter_map(|(row, f)| {
            let f = (*f)?;
            if !mixed[f] {
                return None;
            }
            let faces = target.near_ties(td, v[row], f, eps);
            let mut per: Vec<(u16, usize)> = Vec::new();
            for g in faces {
                let l = flab[g];
                match per.iter_mut().find(|x| x.0 == l) {
                    Some(x) => x.1 = x.1.min(g),
                    None => per.push((l, g)),
                }
            }
            (per.len() >= 2).then_some((row, per))
        })
        .collect();
    if ties.is_empty() {
        return Ok(vl);
    }
    let (start, nb) = mesh::vertex_neighbours(v.len(), e);
    let choice: Vec<(usize, u16)> = ties
        .iter()
        .map(|(row, per)| {
            let neigh = &nb[start[*row]..start[*row + 1]];
            let best = per
                .iter()
                .map(|&(l, f)| {
                    let count = neigh.iter().filter(|&&n| vl[n as usize] == l).count();
                    (l != 0, std::cmp::Reverse(count), f, l)
                })
                .min()
                .map_or(0, |b| b.3);
            (*row, best)
        })
        .collect();
    for (row, l) in choice {
        vl[row] = l;
    }
    Ok(vl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_regions() {
        let (a, b, c) = (DVec3::ZERO, DVec3::X, DVec3::Y);
        assert_eq!(closest_tri(DVec3::new(-1.0, -1.0, 0.0), a, b, c), a);
        assert_eq!(closest_tri(DVec3::new(2.0, -0.1, 0.0), a, b, c), b);
        assert_eq!(
            closest_tri(DVec3::new(0.25, 0.25, 3.0), a, b, c),
            DVec3::new(0.25, 0.25, 0.0)
        );
        assert_eq!(
            closest_tri(DVec3::new(0.5, -1.0, 0.0), a, b, c),
            DVec3::new(0.5, 0.0, 0.0)
        );
        // degenerate triangle: nearest edge point
        let p = closest_tri(DVec3::new(0.5, 1.0, 0.0), a, b, 2.0 * b);
        assert_eq!(p, DVec3::new(0.5, 0.0, 0.0));
    }
}
