//! Targets of the rebuilt surface: the convex outline of every horizontal
//! section of a crown, the contact surfaces between neighbouring crowns,
//! and the thickness of thin sheets (`reconstruct._hull2d`,
//! `convex_targets`, `contact_surfaces`, `contact_targets`, `thickness`).

use glam::{DVec2, DVec3};

use super::geom::{KdTree, normalize, vertex_normals};
use super::{CONTACT_FILL_MM, CONTACT_REACH_MM, CONVEX_MIN_PTS, CONVEX_SLAB_MM};

/// `numpy.round(x, 6)`.
fn round6(x: f64) -> f64 {
    (x * 1e6).round_ties_even() / 1e6
}

fn cross2(o: DVec2, a: DVec2, b: DVec2) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// Convex hull of plan points, counter-clockwise (monotone chain on the
/// points rounded to 1e-6 and made unique, `_hull2d`).
pub fn hull2d(p: &[DVec2]) -> Vec<DVec2> {
    let mut pts: Vec<DVec2> = p
        .iter()
        .map(|q| DVec2::new(round6(q.x), round6(q.y)))
        .collect();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let half = |it: &mut dyn Iterator<Item = DVec2>| {
        let mut h: Vec<DVec2> = Vec::new();
        for q in it {
            while h.len() >= 2 && cross2(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h
    };
    let mut lo = half(&mut pts.iter().copied());
    let mut hi = half(&mut pts.iter().rev().copied());
    lo.pop();
    hi.pop();
    lo.extend(hi);
    lo
}

/// Where the points `q` (rebuilt surface of an open crown) fall inside the
/// convex outline of the scanned crown at their height: that outline, along
/// the line from its centre through the point; `None` where the point is
/// outside it or no crown was scanned at that height (`convex_targets`).
/// `vc`: the open crown (its mean is the origin of the sections); the
/// outline is made of `vc[keep]`, or of `h3` when given.
pub fn convex_targets(
    vc: &[DVec3],
    keep: &[bool],
    axis: DVec3,
    q: &[DVec3],
    tol: f64,
    h3: Option<&[DVec3]>,
) -> Vec<Option<DVec3>> {
    let a = normalize(axis);
    let e1 = normalize(a.cross(if a.x.abs() < 0.9 { DVec3::X } else { DVec3::Y }));
    let e2 = a.cross(e1);
    let c = super::geom::mean(vc);
    let pts: Vec<DVec3> = match h3 {
        Some(h) => h.to_vec(),
        None => vc
            .iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|(p, _)| *p)
            .collect(),
    };
    let proj = |p: &DVec3| {
        let d = *p - c;
        (d.dot(a), d.dot(e1), d.dot(e2))
    };
    let pp: Vec<(f64, f64, f64)> = pts.iter().map(proj).collect();
    let pq: Vec<(f64, f64, f64)> = q.iter().map(proj).collect();
    let mut out = vec![None; q.len()];
    let half = 0.5 * CONVEX_SLAB_MM;
    let slab: Vec<i64> = pq
        .iter()
        .map(|x| (x.0 / CONVEX_SLAB_MM).round_ties_even() as i64)
        .collect();
    let mut ks = slab.clone();
    ks.sort_unstable();
    ks.dedup();
    for k in ks {
        let hs = k as f64 * CONVEX_SLAB_MM;
        let sel: Vec<DVec2> = pp
            .iter()
            .filter(|x| (x.0 - hs).abs() <= half + 0.1)
            .map(|x| DVec2::new(x.1, x.2))
            .collect();
        if sel.len() < CONVEX_MIN_PTS {
            continue;
        }
        let h = hull2d(&sel);
        if h.len() < 3 {
            continue;
        }
        let cen = h.iter().copied().sum::<DVec2>() / h.len() as f64;
        for (j, x) in pq.iter().enumerate() {
            if slab[j] != k {
                continue;
            }
            let d = DVec2::new(x.1, x.2) - cen;
            let r = d.length();
            let u = d / r.max(1e-9);
            // exit of the ray cen + t u through the hull: the largest t on its edges
            let mut tbest = f64::NEG_INFINITY;
            for m in 0..h.len() {
                let (pa, pb) = (h[m], h[(m + 1) % h.len()]);
                let e = pb - pa;
                let w = pa - cen;
                let den = u.x * e.y - u.y * e.x;
                let t = (w.x * e.y - w.y * e.x) / den;
                let sp = (w.x * u.y - w.y * u.x) / den;
                if t.is_finite() && t > 0.0 && (-1e-9..=1.0 + 1e-9).contains(&sp) && t > tbest {
                    tbest = t;
                }
            }
            if tbest.is_finite() && r < tbest - tol {
                let p2 = cen + u * tbest;
                out[j] = Some(c + p2.x * e1 + p2.y * e2 + x.0 * a);
            }
        }
    }
    out
}

/// The contact surface between a tooth and one neighbouring crown: sample
/// points and, at each, the unit normal towards the neighbour.
#[derive(Debug, Clone)]
pub struct ContactSurface {
    /// Sample points.
    pub points: Vec<DVec3>,
    /// Normal at each point, towards the neighbour.
    pub normals: Vec<DVec3>,
}

/// `numpy.arange(start, stop, step)`.
fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    (0..n).map(|i| start + i as f64 * step).collect()
}

/// `numpy.linspace(0, 1, k)`.
fn linspace01(k: usize) -> Vec<f64> {
    if k == 1 {
        return vec![0.0];
    }
    let step = 1.0 / (k - 1) as f64;
    (0..k)
        .map(|i| if i == k - 1 { 1.0 } else { i as f64 * step })
        .collect()
}

/// Where the scan joins tooth `fdi` to a neighbouring crown: for every
/// neighbour the contact surface between the two (`contact_surfaces`). The
/// border between the two labels runs on the outer surface the scanner
/// closed over them; at every height (slices of `slab`) the contact surface
/// is the segment between its two ends across the arch.
pub fn contact_surfaces(
    v: &[DVec3],
    labels: &[u16],
    edges: &[[u32; 2]],
    fdi: u16,
    slab: f64,
    step: f64,
) -> Vec<ContactSurface> {
    let own = |i: u32| labels[i as usize] == fdi;
    let mut gs: Vec<u16> = edges
        .iter()
        .flat_map(|e| {
            let mut x = Vec::new();
            if own(e[0]) {
                x.push(labels[e[1] as usize]);
            }
            if own(e[1]) {
                x.push(labels[e[0] as usize]);
            }
            x
        })
        .collect();
    gs.sort_unstable();
    gs.dedup();
    let own_pts: Vec<DVec3> = (0..v.len())
        .filter(|&i| labels[i] == fdi)
        .map(|i| v[i])
        .collect();
    let own_mean = super::geom::mean(&own_pts);
    let mut out = Vec::new();
    for g in gs {
        if g == 0 || g == fdi {
            continue;
        }
        let mut sv: Vec<u32> = edges
            .iter()
            .filter(|e| {
                (own(e[0]) && labels[e[1] as usize] == g)
                    || (own(e[1]) && labels[e[0] as usize] == g)
            })
            .flat_map(|e| *e)
            .collect();
        sv.sort_unstable();
        sv.dedup();
        if sv.len() < 10 {
            continue;
        }
        let s: Vec<DVec3> = sv.iter().map(|&i| v[i as usize]).collect();
        let g_pts: Vec<DVec3> = (0..v.len())
            .filter(|&i| labels[i] == g)
            .map(|i| v[i])
            .collect();
        let mut n = super::geom::mean(&g_pts) - own_mean;
        n.z = 0.0;
        if n.length() < 1e-6 {
            continue;
        }
        n /= n.length();
        let across = DVec3::new(-n.y, n.x, 0.0);
        let (zmin, zmax) = s
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p.z), b.max(p.z))
            });
        let mut pts = Vec::new();
        let mut nrm = Vec::new();
        for z0 in arange(zmin, zmax + slab, slab) {
            let p: Vec<DVec3> = s
                .iter()
                .filter(|q| (q.z - z0).abs() <= 0.5 * slab)
                .copied()
                .collect();
            if p.len() < 2 {
                continue;
            }
            let u: Vec<f64> = p.iter().map(|q| q.dot(across)).collect();
            let (mut i0, mut i1) = (0usize, 0usize);
            for (i, &x) in u.iter().enumerate() {
                if x < u[i0] {
                    i0 = i;
                }
                if x > u[i1] {
                    i1 = i;
                }
            }
            if u[i1] - u[i0] < 0.3 {
                continue;
            }
            let k = 2usize.max(((u[i1] - u[i0]) / step).ceil() as usize);
            for t in linspace01(k) {
                pts.push(p[i0] + t * (p[i1] - p[i0]));
            }
            let dv = p[i1] - p[i0];
            let mut nn = DVec3::new(dv.y, -dv.x, 0.0);
            nn /= nn.length().max(1e-9);
            let nn = if nn.dot(n) >= 0.0 { nn } else { -nn };
            nrm.extend(std::iter::repeat_n(nn, k));
        }
        if !pts.is_empty() {
            out.push(ContactSurface {
                points: pts,
                normals: nrm,
            });
        }
    }
    out
}

/// Where the points `q` (rebuilt surface, outward normals `nq`) behind a
/// contact surface facing it should be: on it, [`CONTACT_FILL_MM`] before it
/// (`contact_targets`).
pub fn contact_targets(
    q: &[DVec3],
    nq: &[DVec3],
    contacts: &[ContactSurface],
) -> Vec<Option<DVec3>> {
    let clear = CONTACT_FILL_MM;
    let reach = CONTACT_REACH_MM;
    let mut out = vec![None; q.len()];
    let mut best = vec![f64::INFINITY; q.len()];
    for cs in contacts {
        let tree = KdTree::new(&cs.points);
        for (i, &p) in q.iter().enumerate() {
            let Some((k, _)) = tree.nearest(p) else {
                continue;
            };
            let (qc, n) = (cs.points[k], cs.normals[k]);
            let sgn = (p - qc).dot(n);
            let lat = ((p - qc) - sgn * n).length();
            let facing = nq[i].dot(n) > 0.3;
            if facing && sgn < -clear && -sgn < reach && lat < 0.6 && -sgn < best[i] {
                out[i] = Some(p + (-clear - sgn) * n);
                best[i] = -sgn;
            }
        }
    }
    out
}

/// Distance from the vertices `idx` to the opposite side of the surface:
/// nearest hit along plus or minus the vertex normal, within 2 mm
/// (`thickness`; infinite without a hit).
pub fn thickness(vc: &[DVec3], tc: &[[u32; 3]], idx: &[usize]) -> Vec<f64> {
    let mesh = dc_mesh::TriMesh::new(vc.iter().map(|p| p.as_vec3()).collect(), tc.to_vec());
    let Ok(bvh) = dc_geom::bvh::Bvh::build(&mesh) else {
        return vec![f64::INFINITY; idx.len()];
    };
    let nrm = vertex_normals(vc, tc);
    idx.iter()
        .map(|&i| {
            let mut best = f64::INFINITY;
            for sgn in [-1.0, 1.0] {
                let d = sgn * nrm[i];
                if d == DVec3::ZERO {
                    continue;
                }
                let o = vc[i] + d * 1e-3;
                if let Some(h) = bvh.ray_cast_range(o.as_vec3(), d.as_vec3(), 1e-5, 2.0) {
                    best = best.min(h.t as f64);
                }
            }
            best
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_of_a_square_with_inner_points() {
        let p = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
            DVec2::new(0.5, 0.5),
            DVec2::new(0.5, 0.0),
        ];
        let h = hull2d(&p);
        assert_eq!(h.len(), 4);
        assert_eq!(h[0], DVec2::new(0.0, 0.0));
        assert_eq!(h[1], DVec2::new(1.0, 0.0));
    }

    #[test]
    fn helpers_match_numpy() {
        assert_eq!(arange(0.0, 1.0, 0.5), vec![0.0, 0.5]);
        assert_eq!(arange(0.0, 1.1, 0.5).len(), 3);
        assert_eq!(linspace01(3), vec![0.0, 0.5, 1.0]);
    }
}
