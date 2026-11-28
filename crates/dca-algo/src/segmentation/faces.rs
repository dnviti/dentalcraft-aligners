//! First partition on the simplified mesh (`segmentation.build_seed_labels`,
//! `random_walk`, `geodesic_init`, `recentre_seeds`, `segment_faces`).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use dc_case::Fdi;
use glam::{DVec2, DVec3};

use super::graph::{FaceGraph, face_graph};
use super::mesh::{self, NONE};
use super::solve::{Ell, repro_cg};
use super::{FaceDiagnostics, FaceParams, GUM};
use crate::anatomy::{self, Arch};

/// mm below the clicked point kept in a seed disk.
const SEED_DEPTH: f64 = 3.5;
/// Seed disk radius, fraction of the mesio-distal diameter.
const SEED_RADIUS: f64 = 0.40;
/// Face rings along the open border used as gingival constraints.
const BORDER_RINGS: usize = 3;
/// mm beyond the typical half width of the crowns: gingiva.
const GUM_MARGIN_H: f64 = 3.5;
/// mm below the typical crown length under the occlusal level: gingiva.
const GUM_DEPTH_EXTRA: f64 = 1.5;
/// Radius (mm) around every seed never constrained to the gingiva.
const PROTECT_RADIUS: f64 = 3.0;
/// mm beyond the distal surface of the last tooth of a side (the end of the
/// arch line) where the gingiva starts, whatever its height.
const GUM_DISTAL_MM: f64 = 2.0;
/// Fraction of the crown length below the click: the crown whose centre
/// becomes the seed.
const RECENTRE_BAND: f64 = 0.5;
/// Largest move of a seed, fraction of the mesio-distal diameter.
const RECENTRE_MAX: f64 = 0.3;

/// Face seed labels (`-1` free, `0` gingiva, `i` the i-th tooth of the arch
/// sequence) and the FDI of every label.
fn build_seed_labels(
    c: &[DVec3],
    seeds: &[(Fdi, DVec3)],
    arch: Arch,
    border: Option<&[bool]>,
    allowed: Option<&[i64]>,
) -> (Vec<i64>, Vec<u16>) {
    let m = c.len();
    let mut lab = vec![-1i64; m];
    let fdis: Vec<Fdi> = seeds.iter().map(|s| s.0).collect();
    let seq = anatomy::arch_sequence(&fdis, arch);
    let pt = |f: Fdi| {
        seeds
            .iter()
            .find(|s| s.0 == f)
            .map(|s| s.1)
            .unwrap_or_default()
    };
    let dist = |a: DVec3, b: DVec3| (a - b).length();
    for (i, &f) in seq.iter().enumerate() {
        let label = i as i64 + 1;
        let p = pt(f);
        let r = SEED_RADIUS * anatomy::typical_diameters(f).0;
        let d: Vec<f64> = c.iter().map(|&q| dist(q, p)).collect();
        let mut sel: Vec<bool> = (0..m)
            .map(|j| d[j] < r && c[j].z > p.z - SEED_DEPTH)
            .collect();
        if let Some(al) = allowed {
            let n = (0..m).filter(|&j| sel[j] && al[j] == label).count();
            if n >= 4 {
                for j in 0..m {
                    sel[j] &= al[j] == label;
                }
            }
        }
        if sel.iter().filter(|&&s| s).count() < 4 {
            let mut order: Vec<usize> = (0..m).collect();
            order.sort_by(|&a, &b| d[a].total_cmp(&d[b]).then(a.cmp(&b)));
            sel = vec![false; m];
            for &j in order.iter().take(6) {
                sel[j] = true;
            }
        }
        for j in 0..m {
            if sel[j] {
                lab[j] = label;
            }
        }
    }
    // gingival constraints: far from the arch polyline in the occlusal plane,
    // well below the crown, or past the distal surface of the last tooth of
    // a side; the line reaches that surface. What lies behind the last
    // tooth is the tuberosity, the retromolar pad or the base of a model,
    // never a crown, even when it stands as high as one
    let mut p: Vec<DVec3> = seq.iter().map(|&f| pt(f)).collect();
    let mut half_bl: Vec<f64> = seq
        .iter()
        .map(|&f| anatomy::typical_diameters(f).1 / 2.0)
        .collect();
    let mut crown: Vec<f64> = seq
        .iter()
        .map(|&f| anatomy::crown_root_length(f).0)
        .collect();
    if p.len() >= 2 {
        let k = p.len();
        let ext = |end: usize, prev: usize| {
            let mut u = p[end] - p[prev];
            u.z = 0.0;
            u /= u.length().max(1e-9);
            p[end] + u * (0.5 * anatomy::typical_diameters(seq[end]).0)
        };
        let (e0, e1) = (ext(0, 1), ext(k - 1, k - 2));
        p.insert(0, e0);
        p.push(e1);
        half_bl.insert(0, half_bl[0]);
        half_bl.push(half_bl[k]);
        crown.insert(0, crown[0]);
        crown.push(crown[k]);
    }
    let p2: Vec<DVec2> = p.iter().map(|q| q.truncate()).collect();
    let mut gum = vec![false; m];
    if !p.is_empty() {
        let last = p2.len() - 1;
        for j in 0..m {
            let q = c[j].truncate();
            let (dist_h, seg, t) = polyline_projection(&p2, q);
            let j2 = (seg + 1).min(p.len() - 1);
            let z_seed = p[seg].z * (1.0 - t) + p[j2].z * t;
            let r_h = (half_bl[seg] * (1.0 - t) + half_bl[j2] * t) + GUM_MARGIN_H;
            let h_d = (crown[seg] * (1.0 - t) + crown[j2] * t) + GUM_DEPTH_EXTRA;
            // how far past an end of the line, along its last segment
            let past = match (seg, t) {
                _ if seq.len() < 2 => 0.0,
                (0, t) if t <= 0.0 => (q - p2[0]).dot((p2[0] - p2[1]).normalize_or_zero()),
                (s, t) if s + 1 == last && t >= 1.0 => {
                    (q - p2[last]).dot((p2[last] - p2[last - 1]).normalize_or_zero())
                }
                _ => 0.0,
            };
            gum[j] = dist_h > r_h || c[j].z < z_seed - h_d || past > GUM_DISTAL_MM;
        }
    }
    if let Some(b) = border {
        for j in 0..m {
            gum[j] |= b[j];
        }
    }
    for j in 0..m {
        let near = seq.iter().any(|&f| dist(c[j], pt(f)) < PROTECT_RADIUS);
        if gum[j] && !near && lab[j] < 0 {
            lab[j] = GUM;
        }
    }
    let mut fdi_of = vec![0u16];
    fdi_of.extend(seq.iter().map(|f| u16::from(f.get())));
    (lab, fdi_of)
}

/// Distance of `q` to the polyline `p`, segment and parameter of the
/// projection (first segment on ties).
fn polyline_projection(p: &[DVec2], q: DVec2) -> (f64, usize, f64) {
    if p.len() == 1 {
        let d = q - p[0];
        return ((d.x * d.x + d.y * d.y).sqrt(), 0, 0.0);
    }
    let mut best = (f64::INFINITY, 0usize, 0.0f64);
    for s in 0..p.len() - 1 {
        let (a, b) = (p[s], p[s + 1]);
        let ab = b - a;
        let l2 = (ab.x * ab.x + ab.y * ab.y).max(1e-12);
        let ap = q - a;
        let t = ((ap.x * ab.x + ap.y * ab.y) / l2).clamp(0.0, 1.0);
        let proj = a + t * ab;
        let e = q - proj;
        let d = (e.x * e.x + e.y * e.y).sqrt();
        if d < best.0 {
            best = (d, s, t);
        }
    }
    best
}

/// Multi-label random walker (Grady 2006) on the face graph: label of every
/// face, CG iterations and largest residual.
fn random_walk(
    g: &FaceGraph,
    seed_lab: &[i64],
    n_labels: usize,
    init: Option<&[i64]>,
    tol: f64,
    maxiter: usize,
) -> (Vec<i64>, usize, f64) {
    let m = g.nbr.len();
    let seeded: Vec<bool> = seed_lab.iter().map(|&l| l >= 0).collect();
    let comp = mesh::face_components(&g.nbr);
    let mut comp_has_seed = vec![false; m];
    for i in 0..m {
        if seeded[i] {
            comp_has_seed[comp[i]] = true;
        }
    }
    let active: Vec<bool> = (0..m)
        .map(|i| comp_has_seed[comp[i]] && !seeded[i])
        .collect();
    let u: Vec<usize> = (0..m).filter(|&i| active[i]).collect();
    let mut out = seed_lab.to_vec();
    for i in 0..m {
        if !active[i] && !seeded[i] {
            out[i] = GUM;
        }
    }
    if u.is_empty() {
        return (out, 0, 0.0);
    }
    let mut uidx = vec![u32::MAX; m];
    for (k, &i) in u.iter().enumerate() {
        uidx[i] = k as u32;
    }
    let n = u.len();
    let mut ell = Ell {
        nb: vec![[0; 3]; n],
        w: vec![[0.0; 3]; n],
        d: vec![0.0; n],
    };
    let mut bs = vec![vec![0.0; n]; n_labels];
    for (r, &i) in u.iter().enumerate() {
        let w = g.w[i];
        ell.d[r] = (w[0] + w[1]) + w[2];
        for (k, (&nb, &wk)) in g.nbr[i].iter().zip(&w).enumerate() {
            if nb == NONE {
                continue;
            }
            let nb = nb as usize;
            if uidx[nb] != u32::MAX {
                ell.nb[r][k] = uidx[nb];
                ell.w[r][k] = wk;
            } else if seeded[nb] {
                bs[seed_lab[nb] as usize][r] += wk;
            }
        }
    }
    let x0 = init.map(|init| {
        let mut x0 = vec![vec![0.0; n]; n_labels];
        for (r, &i) in u.iter().enumerate() {
            let l = init[i];
            if l >= 0 && (l as usize) < n_labels {
                x0[l as usize][r] = 1.0;
            }
        }
        x0
    });
    let diag = ell.d.clone();
    let sol = repro_cg(&ell, &bs, &diag, x0, tol, maxiter);
    for (r, &i) in u.iter().enumerate() {
        // first maximum (np.argmax)
        let mut best = 0usize;
        for l in 1..n_labels {
            if sol.x[l][r] > sol.x[best][r] {
                best = l;
            }
        }
        out[i] = best as i64;
    }
    let res = sol
        .residuals
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    (out, sol.iterations, res)
}

/// Curvature-weighted multi-source Dijkstra: the label of the nearest seed,
/// the initial guess of the first walk.
fn geodesic_init(g: &FaceGraph, seed_lab: &[i64]) -> Vec<i64> {
    let m = g.nbr.len();
    let mut dist = vec![f64::INFINITY; m];
    let mut lab = vec![-1i64; m];
    // (distance bits, face): the bits of a non-negative f64 sort as its value
    let mut heap = BinaryHeap::new();
    for s in 0..m {
        if seed_lab[s] >= 0 && dist[s] > 0.0 {
            dist[s] = 0.0;
            lab[s] = seed_lab[s];
            heap.push(Reverse((0.0f64.to_bits(), s)));
        }
    }
    while let Some(Reverse((bits, u))) = heap.pop() {
        let d = f64::from_bits(bits);
        if d > dist[u] {
            continue;
        }
        let lu = lab[u];
        for k in 0..3 {
            let v = g.nbr[u][k];
            if v == NONE {
                continue;
            }
            let v = v as usize;
            let e = g.c[v] - g.c[u];
            let length = (e.x * e.x + e.y * e.y + e.z * e.z).sqrt();
            let nd = d + length * (1.0 + g.d[u][k]);
            if nd < dist[v] {
                dist[v] = nd;
                lab[v] = lu;
                heap.push(Reverse((nd.to_bits(), v)));
            }
        }
    }
    for l in &mut lab {
        if *l < 0 {
            *l = GUM;
        }
    }
    lab
}

/// Every seed moved to the centre of its crown as found by a segmentation
/// (occlusal plane, faces of the tooth less than RECENTRE_BAND of the crown
/// length below the click), by at most RECENTRE_MAX of the mesio-distal
/// diameter.
fn recentre_seeds(
    c: &[DVec3],
    lab: &[i64],
    seeds: &[(Fdi, DVec3)],
    label_of: impl Fn(Fdi) -> i64,
) -> Vec<(Fdi, DVec3)> {
    seeds
        .iter()
        .map(|&(f, p)| {
            let l = label_of(f);
            let zmin = p.z - RECENTRE_BAND * anatomy::crown_root_length(f).0;
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
            for (q, &fl) in c.iter().zip(lab) {
                if fl == l && q.z > zmin {
                    sx += q.x;
                    sy += q.y;
                    n += 1;
                }
            }
            let mut q = p;
            if n >= 10 {
                let s = DVec2::new(sx / n as f64 - p.x, sy / n as f64 - p.y);
                let cap = RECENTRE_MAX * anatomy::typical_diameters(f).0;
                let len = (s.x * s.x + s.y * s.y).sqrt();
                let k = 1.0f64.min(cap / len.max(1e-9));
                q.x += s.x * k;
                q.y += s.y * k;
            }
            (f, q)
        })
        .collect()
}

/// Face FDI labels of a (simplified) mesh in arch coordinates.
pub(crate) fn segment_faces(
    v: &[DVec3],
    t: &[[u32; 3]],
    seeds: &[(Fdi, DVec3)],
    arch: Arch,
    params: &FaceParams,
) -> (Vec<u16>, FaceDiagnostics) {
    let g = face_graph(v, t, params.eta_convex, params.sigma, 0.0);
    // the cut edge of an intraoral scan lies on the soft tissue
    let bmask = mesh::boundary_vertex_mask(t, &g.nbr, v.len());
    let border = bmask.iter().any(|&b| b).then(|| {
        let mut border: Vec<bool> = t
            .iter()
            .map(|f| f.iter().any(|&i| bmask[i as usize]))
            .collect();
        for _ in 0..BORDER_RINGS {
            border = (0..t.len())
                .map(|i| border[i] || g.nbr[i].iter().any(|&nb| nb != NONE && border[nb as usize]))
                .collect();
        }
        border
    });
    let (mut seed_lab, fdi_of) = build_seed_labels(&g.c, seeds, arch, border.as_deref(), None);
    let n_labels = fdi_of.len();
    let init = geodesic_init(&g, &seed_lab);
    let (mut lab, it, mut res) = random_walk(
        &g,
        &seed_lab,
        n_labels,
        Some(&init),
        params.tol,
        params.maxiter,
    );
    let mut iterations = it;
    let label_of = |f: Fdi| {
        fdi_of
            .iter()
            .position(|&x| x == u16::from(f.get()))
            .map_or(-1, |i| i as i64)
    };
    let mut seeds2 = seeds.to_vec();
    for _ in 0..params.recentre_passes {
        seeds2 = recentre_seeds(&g.c, &lab, &seeds2, label_of);
        seed_lab = build_seed_labels(&g.c, &seeds2, arch, border.as_deref(), Some(&lab)).0;
        let (l2, it2, r2) = random_walk(
            &g,
            &seed_lab,
            n_labels,
            Some(&lab),
            params.tol,
            params.maxiter,
        );
        lab = l2;
        iterations += it2;
        res = r2;
    }
    let seed_shift_mm = (params.recentre_passes > 0).then(|| {
        let m = seeds
            .iter()
            .zip(&seeds2)
            .map(|(a, b)| (a.1 - b.1).length())
            .fold(0.0f64, f64::max);
        (m * 100.0).round() / 100.0
    });
    let fdi_lab = lab.iter().map(|&l| fdi_of[l as usize]).collect();
    let diag = FaceDiagnostics {
        faces: t.len(),
        cg_iterations: iterations,
        cg_residual: res,
        seed_shift_mm,
        gum_seed_faces: seed_lab.iter().filter(|&&l| l == GUM).count(),
        tooth_seed_faces: seed_lab.iter().filter(|&&l| l > 0).count(),
    };
    (fdi_lab, diag)
}
