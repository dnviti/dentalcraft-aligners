//! Dual (face) graph of Lai et al. 2009 (`segmentation.face_graph`,
//! `smooth_face_normals`, `smoothing_iterations`).

use glam::DVec3;

use super::mesh::{self, NONE};

/// Face graph: neighbours, transition weights, centroids and normalised
/// fold measures, three per face (zero across a border edge).
pub(crate) struct FaceGraph {
    pub nbr: Vec<[u32; 3]>,
    pub w: Vec<[f64; 3]>,
    pub c: Vec<DVec3>,
    pub d: Vec<[f64; 3]>,
}

/// Face normals averaged with the neighbouring faces (area weighted),
/// `iterations` times.
fn smooth_face_normals(n: &[DVec3], a: &[f64], nbr: &[[u32; 3]], iterations: usize) -> Vec<DVec3> {
    let mut ns = n.to_vec();
    for _ in 0..iterations {
        ns = (0..n.len())
            .map(|i| {
                let term = |k: usize| {
                    let g = nbr[i][k];
                    let (wa, v) = if g == NONE {
                        (0.0, ns[0])
                    } else {
                        (a[g as usize], ns[g as usize])
                    };
                    wa * v
                };
                let s = a[i] * ns[i] + ((term(0) + term(1)) + term(2));
                s / s.length().max(1e-12)
            })
            .collect();
    }
    ns
}

/// Iterations of the 1-ring average whose kernel has RMS radius `scale`
/// (mm) on faces of edge `h`.
fn smoothing_iterations(scale: f64, h: f64) -> usize {
    if scale > 0.0 {
        (4.0 * (scale / h.max(1e-9)).powi(2)).round_ties_even() as usize
    } else {
        0
    }
}

/// Dual graph weights: `d = eta (1 - cos(dihedral)) / mean`, eta = 1 on
/// concave edges and `eta_convex` on convex ones; `w = |e| exp(-d / sigma)`
/// (at least `floor |e|`). `normal_scale` (mm): the folds are measured
/// between normals averaged over this radius.
pub(crate) fn face_graph(
    v: &[DVec3],
    t: &[[u32; 3]],
    eta_convex: f64,
    sigma: f64,
    normal_scale: f64,
) -> FaceGraph {
    const FLOOR: f64 = 1e-8;
    let (mut n, a) = mesh::tri_normals(v, t);
    let c: Vec<DVec3> = t
        .iter()
        .map(|f| (v[f[0] as usize] + v[f[1] as usize] + v[f[2] as usize]) / 3.0)
        .collect();
    let nbr = mesh::face_adjacency(t, v.len());
    let elen: Vec<[f64; 3]> = t
        .iter()
        .map(|f| {
            let [p0, p1, p2] = f.map(|i| v[i as usize]);
            [(p1 - p0).length(), (p2 - p1).length(), (p0 - p2).length()]
        })
        .collect();
    let flat: Vec<f64> = elen.iter().flatten().copied().collect();
    let h = if flat.is_empty() {
        0.0
    } else {
        mesh::np_sum(&flat) / flat.len() as f64
    };
    let it = smoothing_iterations(normal_scale, h);
    if it > 0 {
        n = smooth_face_normals(&n, &a, &nbr, it);
    }
    let mut d1 = vec![[0.0f64; 3]; t.len()];
    let mut valid_d1 = Vec::with_capacity(t.len() * 3);
    for i in 0..t.len() {
        for k in 0..3 {
            let g = nbr[i][k];
            if g == NONE {
                continue;
            }
            let g = g as usize;
            let (ni, ng) = (n[i], n[g]);
            let cosang = (ni.x * ng.x + ni.y * ng.y + ni.z * ng.z).clamp(-1.0, 1.0);
            let (dc, dn) = (c[g] - c[i], ni - ng);
            let concave = dc.x * dn.x + dc.y * dn.y + dc.z * dn.z > 0.0;
            let eta = if concave { 1.0 } else { eta_convex };
            d1[i][k] = eta * (1.0 - cosang);
            valid_d1.push(d1[i][k]);
        }
    }
    let mean_d1 = if valid_d1.is_empty() {
        1.0
    } else {
        mesh::np_sum(&valid_d1) / valid_d1.len() as f64
    };
    let norm = mean_d1.max(1e-12);
    let mut d = vec![[0.0f64; 3]; t.len()];
    let mut w = vec![[0.0f64; 3]; t.len()];
    for i in 0..t.len() {
        for k in 0..3 {
            d[i][k] = d1[i][k] / norm;
            if nbr[i][k] != NONE {
                let wi = elen[i][k] * (-d[i][k] / sigma).exp();
                w[i][k] = wi.max(FLOOR * elen[i][k]);
            }
        }
    }
    FaceGraph { nbr, w, c, d }
}
