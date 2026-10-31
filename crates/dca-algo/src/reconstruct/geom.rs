//! Mesh helpers of the reconstruction, in float64 (`meshutil.py` and the
//! helpers at the top of `reconstruct.py`): unique edges, boundary loops,
//! edge-connected pieces, compaction, the marching-triangles cut, umbrella
//! smoothing of a scalar, exact nearest distances (k-d tree) and the
//! self-intersection test (`printcheck.self_intersecting_pairs`).

use glam::DVec3;

/// Undirected edges of the triangles, each once, sorted by `(min, max)`
/// (`mu.unique_edges`).
pub fn unique_edges(tris: &[[u32; 3]]) -> Vec<[u32; 2]> {
    let mut e: Vec<[u32; 2]> = Vec::with_capacity(tris.len() * 3);
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            e.push(if a < b { [a, b] } else { [b, a] });
        }
    }
    e.sort_unstable();
    e.dedup();
    e
}

/// Number of edges at every vertex (`bincount(a) + bincount(b)` of the
/// unique edges).
pub fn edge_degree(edges: &[[u32; 2]], n: usize) -> Vec<usize> {
    let mut d = vec![0usize; n];
    for e in edges {
        d[e[0] as usize] += 1;
        d[e[1] as usize] += 1;
    }
    d
}

/// Neighbour face across each edge `k = (T[k], T[(k+1)%3])` of every
/// triangle; `None` for a boundary or non-manifold edge (`mu.face_adjacency`).
pub fn face_adjacency(tris: &[[u32; 3]]) -> Vec<[Option<u32>; 3]> {
    let mut keys: Vec<(u64, u32)> = Vec::with_capacity(tris.len() * 3);
    for (f, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let (a, b) = if a < b { (a, b) } else { (b, a) };
            keys.push((((a as u64) << 32) | b as u64, (f * 3 + k) as u32));
        }
    }
    keys.sort_unstable();
    let mut nbr = vec![[None; 3]; tris.len()];
    let mut i = 0;
    while i < keys.len() {
        let mut j = i + 1;
        while j < keys.len() && keys[j].0 == keys[i].0 {
            j += 1;
        }
        if j - i == 2 {
            let (x, y) = (keys[i].1 as usize, keys[i + 1].1 as usize);
            nbr[x / 3][x % 3] = Some((y / 3) as u32);
            nbr[y / 3][y % 3] = Some((x / 3) as u32);
        }
        i = j;
    }
    nbr
}

/// Ordered loops of the open border, in face order (`mu.boundary_loops`:
/// the boundary edges of corner 0 of every face, then corner 1, then 2;
/// loops shorter than 3 are dropped).
pub fn boundary_loops(tris: &[[u32; 3]]) -> Vec<Vec<u32>> {
    let nbr = face_adjacency(tris);
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for k in 0..3 {
        for (f, t) in tris.iter().enumerate() {
            if nbr[f][k].is_none() {
                edges.push((t[k], t[(k + 1) % 3]));
            }
        }
    }
    let mut nxt: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for &(a, b) in &edges {
        nxt.entry(a).or_default().push(b);
    }
    let mut used: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    let mut loops = Vec::new();
    for &(a0, b0) in &edges {
        if used.contains(&(a0, b0)) {
            continue;
        }
        let mut lp = vec![a0];
        let (mut a, mut b) = (a0, b0);
        let ok = loop {
            used.insert((a, b));
            if b == a0 {
                break true;
            }
            lp.push(b);
            let cand = nxt
                .get(&b)
                .and_then(|cs| cs.iter().copied().find(|&c| !used.contains(&(b, c))));
            match cand {
                Some(c) => {
                    a = b;
                    b = c;
                }
                None => break false,
            }
        };
        let _ = a;
        if ok && lp.len() >= 3 {
            loops.push(lp);
        }
    }
    loops
}

/// Faces of the largest edge-connected piece (`mu.largest_face_component`;
/// ties go to the piece holding the lowest face index).
pub fn largest_face_component(tris: &[[u32; 3]]) -> Vec<bool> {
    let n = tris.len();
    if n == 0 {
        return Vec::new();
    }
    let nbr = face_adjacency(tris);
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for (f, nb) in nbr.iter().enumerate() {
        for g in nb.iter().flatten() {
            let (a, b) = (find(&mut parent, f as u32), find(&mut parent, *g));
            if a != b {
                let (lo, hi) = (a.min(b), a.max(b));
                parent[hi as usize] = lo;
            }
        }
    }
    let roots: Vec<u32> = (0..n as u32).map(|f| find(&mut parent, f)).collect();
    let mut count = vec![0usize; n];
    for &r in &roots {
        count[r as usize] += 1;
    }
    // np.unique sorts the roots (the lowest face of each piece), argmax takes the first maximum
    let mut best = 0usize;
    for r in 0..n {
        if count[r] > count[best] {
            best = r;
        }
    }
    roots.iter().map(|&r| r as usize == best).collect()
}

/// Removes the unreferenced vertices: `(vertices, triangles, old index of
/// every new vertex)` (`mu.compact`).
pub fn compact(v: &[DVec3], tris: &[[u32; 3]]) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<usize>) {
    let mut used = vec![false; v.len()];
    for t in tris {
        for &i in t {
            used[i as usize] = true;
        }
    }
    let mut remap = vec![u32::MAX; v.len()];
    let mut old = Vec::new();
    for (i, &u) in used.iter().enumerate() {
        if u {
            remap[i] = old.len() as u32;
            old.push(i);
        }
    }
    let v2 = old.iter().map(|&i| v[i]).collect();
    let t2 = tris
        .iter()
        .map(|t| {
            [
                remap[t[0] as usize],
                remap[t[1] as usize],
                remap[t[2] as usize],
            ]
        })
        .collect();
    (v2, t2, old)
}

/// Umbrella smoothing of a per-vertex scalar (`smooth_scalar`).
pub fn smooth_scalar(x: &[f64], edges: &[[u32; 2]], iterations: usize, lam: f64) -> Vec<f64> {
    let n = x.len();
    let deg: Vec<f64> = edge_degree(edges, n)
        .into_iter()
        .map(|d| d.max(1) as f64)
        .collect();
    let mut x = x.to_vec();
    let mut s1 = vec![0.0; n];
    let mut s2 = vec![0.0; n];
    for _ in 0..iterations {
        s1.fill(0.0);
        s2.fill(0.0);
        for e in edges {
            s1[e[0] as usize] += x[e[1] as usize];
        }
        for e in edges {
            s2[e[1] as usize] += x[e[0] as usize];
        }
        for i in 0..n {
            x[i] += lam * ((s1[i] + s2[i]) / deg[i] - x[i]);
        }
    }
    x
}

/// Result of [`cut_mesh`].
#[derive(Debug, Clone)]
pub struct Cut {
    /// The vertices, then one new vertex per cut edge.
    pub vertices: Vec<DVec3>,
    /// The triangles of the cut mesh.
    pub tris: Vec<[u32; 3]>,
    /// Per triangle: on the positive side.
    pub side: Vec<bool>,
    /// The new vertex `n + i` lies on edge `(p[i], q[i])` at parameter `t[i]`.
    pub p: Vec<u32>,
    /// See `p`.
    pub q: Vec<u32>,
    /// See `p`.
    pub t: Vec<f64>,
}

/// Marching triangles: splits the mesh along the zero line of `phi`
/// (`cut_mesh`). The new vertex of an edge is shared by its two faces, so
/// the two sides stay watertight; cut points are kept `tmin` away from the
/// mesh vertices.
pub fn cut_mesh(v: &[DVec3], tris: &[[u32; 3]], phi: &[f64], tmin: f64) -> Cut {
    let phi: Vec<f64> = phi
        .iter()
        .map(|&x| if x.abs() < 1e-9 { 1e-9 } else { x })
        .collect();
    let n = v.len();
    let mut tk = Vec::new();
    let mut side_k = Vec::new();
    // crossing faces: (a, b, c) with a the lone vertex, and whether it is positive
    let mut cross: Vec<([u32; 3], bool)> = Vec::new();
    for t in tris {
        let pos = [
            phi[t[0] as usize] > 0.0,
            phi[t[1] as usize] > 0.0,
            phi[t[2] as usize] > 0.0,
        ];
        let npos = pos.iter().filter(|&&p| p).count();
        if npos == 0 || npos == 3 {
            tk.push(*t);
            side_k.push(npos == 3);
            continue;
        }
        let lone = if npos == 1 {
            pos.iter().position(|&p| p).unwrap_or(0)
        } else {
            pos.iter().position(|&p| !p).unwrap_or(0)
        };
        cross.push(([t[lone], t[(lone + 1) % 3], t[(lone + 2) % 3]], pos[lone]));
    }
    if cross.is_empty() {
        return Cut {
            vertices: v.to_vec(),
            tris: tris.to_vec(),
            side: side_k,
            p: Vec::new(),
            q: Vec::new(),
            t: Vec::new(),
        };
    }
    let nc = cross.len();
    let key = |a: u32, b: u32| if a < b { (a, b) } else { (b, a) };
    let mut keys: Vec<(u32, u32)> = Vec::with_capacity(2 * nc);
    for (abc, _) in &cross {
        keys.push(key(abc[0], abc[1]));
    }
    for (abc, _) in &cross {
        keys.push(key(abc[0], abc[2]));
    }
    let mut uk = keys.clone();
    uk.sort_unstable();
    uk.dedup();
    let inv: Vec<u32> = keys
        .iter()
        .map(|k| uk.binary_search(k).map(|i| i as u32).unwrap_or(0))
        .collect();
    let p: Vec<u32> = uk.iter().map(|k| k.0).collect();
    let q: Vec<u32> = uk.iter().map(|k| k.1).collect();
    let t: Vec<f64> = p
        .iter()
        .zip(&q)
        .map(|(&a, &b)| {
            let (fa, fb) = (phi[a as usize], phi[b as usize]);
            (fa / (fa - fb)).clamp(tmin, 1.0 - tmin)
        })
        .collect();
    let mut vertices = v.to_vec();
    for i in 0..p.len() {
        let (a, b) = (v[p[i] as usize], v[q[i] as usize]);
        vertices.push(a + t[i] * (b - a));
    }
    let mut out = tk;
    let mut side = side_k;
    let pab = |i: usize| n as u32 + inv[i];
    let pac = |i: usize| n as u32 + inv[nc + i];
    for (i, (abc, _)) in cross.iter().enumerate() {
        out.push([abc[0], pab(i), pac(i)]);
    }
    for (i, (abc, _)) in cross.iter().enumerate() {
        out.push([pab(i), abc[1], abc[2]]);
    }
    for (i, (abc, _)) in cross.iter().enumerate() {
        out.push([pab(i), abc[2], pac(i)]);
    }
    for (_, lp) in &cross {
        side.push(*lp);
    }
    for _ in 0..2 {
        for (_, lp) in &cross {
            side.push(!*lp);
        }
    }
    Cut {
        vertices,
        tris: out,
        side,
        p,
        q,
        t,
    }
}

/// Unit normals of the triangles (zero for a degenerate one) and their areas
/// (`mu.tri_normals`).
pub fn tri_normals(v: &[DVec3], tris: &[[u32; 3]]) -> (Vec<DVec3>, Vec<f64>) {
    tris.iter()
        .map(|t| {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            let cr = (b - a).cross(c - a);
            let l = cr.length();
            (if l > 0.0 { cr / l } else { cr }, 0.5 * l)
        })
        .unzip()
}

/// Area-weighted vertex normals (`mu.vertex_normals`).
pub fn vertex_normals(v: &[DVec3], tris: &[[u32; 3]]) -> Vec<DVec3> {
    let mut vn = vec![DVec3::ZERO; v.len()];
    for k in 0..3 {
        for t in tris {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            vn[t[k] as usize] += (b - a).cross(c - a);
        }
    }
    vn.into_iter().map(normalize).collect()
}

/// Signed volume (positive with outward normals).
pub fn signed_volume(v: &[DVec3], tris: &[[u32; 3]]) -> f64 {
    tris.iter()
        .map(|t| {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        / 6.0
}

/// `v / |v|`, `v` itself when it is zero (`mu.normalize`).
pub fn normalize(v: DVec3) -> DVec3 {
    let n = v.length();
    if n == 0.0 { v } else { v / n }
}

/// Mean of points (zero for none).
pub fn mean(p: &[DVec3]) -> DVec3 {
    if p.is_empty() {
        return DVec3::ZERO;
    }
    p.iter().copied().sum::<DVec3>() / p.len() as f64
}

/// Unit normal of the best-fit plane (smallest principal direction) of
/// points around their mean `c` (`svd(R - c)[2][2]`, sign arbitrary).
pub fn plane_normal(p: &[DVec3], c: DVec3) -> DVec3 {
    let mut m = nalgebra::Matrix3::<f64>::zeros();
    for q in p {
        let d = *q - c;
        let d = nalgebra::Vector3::new(d.x, d.y, d.z);
        m += d * d.transpose();
    }
    let eig = nalgebra::SymmetricEigen::new(m);
    let k = (0..3)
        .min_by(|&a, &b| eig.eigenvalues[a].total_cmp(&eig.eigenvalues[b]))
        .unwrap_or(0);
    let c = eig.eigenvectors.column(k);
    normalize(DVec3::new(c[0], c[1], c[2]))
}

// ---- nearest points (exact, float64) -----------------------------------------------------------

/// Static k-d tree over points: exact nearest distances in float64
/// (`mu.min_distances`, `mu.kdtree`).
#[derive(Debug, Clone)]
pub struct KdTree {
    pts: Vec<DVec3>,
    /// Point indices, arranged as an implicit balanced tree.
    idx: Vec<u32>,
    /// Split axis of every node (by position in `idx`).
    axis: Vec<u8>,
}

impl KdTree {
    /// The tree of `points` (the index of a point is its position).
    pub fn new(points: &[DVec3]) -> Self {
        let mut idx: Vec<u32> = (0..points.len() as u32).collect();
        let mut axis = vec![0u8; points.len()];
        fn build(p: &[DVec3], idx: &mut [u32], axis: &mut [u8]) {
            if idx.len() <= 1 {
                if let Some(a) = axis.first_mut() {
                    *a = 0;
                }
                return;
            }
            let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
            for &i in idx.iter() {
                lo = lo.min(p[i as usize]);
                hi = hi.max(p[i as usize]);
            }
            let ext = hi - lo;
            let ax = if ext.x >= ext.y && ext.x >= ext.z {
                0
            } else if ext.y >= ext.z {
                1
            } else {
                2
            };
            let mid = idx.len() / 2;
            idx.select_nth_unstable_by(mid, |&a, &b| {
                p[a as usize][ax].total_cmp(&p[b as usize][ax])
            });
            axis[mid] = ax as u8;
            let (l, r) = idx.split_at_mut(mid);
            let (al, ar) = axis.split_at_mut(mid);
            build(p, l, al);
            build(p, &mut r[1..], &mut ar[1..]);
        }
        build(points, &mut idx, &mut axis);
        Self {
            pts: points.to_vec(),
            idx,
            axis,
        }
    }

    /// `true` without points.
    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// Nearest point: `(index, distance)`; ties go to the lowest index.
    pub fn nearest(&self, q: DVec3) -> Option<(usize, f64)> {
        if self.pts.is_empty() {
            return None;
        }
        let mut best = (usize::MAX, f64::INFINITY);
        self.search(0, self.idx.len(), q, &mut best);
        Some((best.0, best.1.sqrt()))
    }

    fn search(&self, lo: usize, hi: usize, q: DVec3, best: &mut (usize, f64)) {
        if lo >= hi {
            return;
        }
        let mid = lo + (hi - lo) / 2;
        let i = self.idx[mid] as usize;
        let p = self.pts[i];
        let d2 = (p - q).length_squared();
        if d2 < best.1 || (d2 == best.1 && i < best.0) {
            *best = (i, d2);
        }
        if hi - lo == 1 {
            return;
        }
        let ax = self.axis[mid] as usize;
        let diff = q[ax] - p[ax];
        let (first, second) = if diff < 0.0 {
            ((lo, mid), (mid + 1, hi))
        } else {
            ((mid + 1, hi), (lo, mid))
        };
        self.search(first.0, first.1, q, best);
        if diff * diff <= best.1 {
            self.search(second.0, second.1, q, best);
        }
    }
}

/// Distance from every point of `p` to the nearest point of `s` (infinite
/// without points in `s`).
pub fn min_distances(p: &[DVec3], s: &[DVec3]) -> Vec<f64> {
    use rayon::prelude::*;
    if s.is_empty() {
        return vec![f64::INFINITY; p.len()];
    }
    let tree = KdTree::new(s);
    p.par_iter()
        .with_min_len(256)
        .map(|&q| tree.nearest(q).map_or(f64::INFINITY, |x| x.1))
        .collect()
}

// ---- self-intersections ------------------------------------------------------------------------

/// Pairs `(i, j)`, `i < j`, of triangles that cross each other and share no
/// vertex (`printcheck.self_intersecting_pairs` with the exact float64 test
/// of the golden shim: some edge of one triangle crosses the other, coplanar
/// pairs ignored), sorted.
pub fn self_intersecting_pairs(v: &[DVec3], tris: &[[u32; 3]]) -> Vec<(u32, u32)> {
    use rayon::prelude::*;
    let n = tris.len();
    if n < 2 {
        return Vec::new();
    }
    let boxes: Vec<(DVec3, DVec3)> = tris
        .iter()
        .map(|t| {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            (a.min(b).min(c), a.max(b).max(c))
        })
        .collect();
    // sweep along x: candidates whose boxes overlap
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| boxes[a as usize].0.x.total_cmp(&boxes[b as usize].0.x));
    let mut pairs: Vec<(u32, u32)> = (0..n)
        .into_par_iter()
        .with_min_len(512)
        .flat_map_iter(|k| {
            let i = order[k] as usize;
            let (lo, hi) = boxes[i];
            let mut out = Vec::new();
            for &j in &order[k + 1..] {
                let j = j as usize;
                let (lo2, hi2) = boxes[j];
                if lo2.x > hi.x {
                    break;
                }
                if lo2.y > hi.y || hi2.y < lo.y || lo2.z > hi.z || hi2.z < lo.z {
                    continue;
                }
                let (ta, tb) = (tris[i], tris[j]);
                if ta.iter().any(|x| tb.contains(x)) {
                    continue;
                }
                if tri_tri(v, ta, tb) {
                    out.push((i.min(j) as u32, i.max(j) as u32));
                }
            }
            out
        })
        .collect();
    pairs.sort_unstable();
    pairs
}

/// Number of triangles that cross another triangle sharing no vertex with
/// it (`mu.self_intersections`).
pub fn self_intersections(v: &[DVec3], tris: &[[u32; 3]]) -> usize {
    let mut f: Vec<u32> = self_intersecting_pairs(v, tris)
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .collect();
    f.sort_unstable();
    f.dedup();
    f.len()
}

fn edge_hits_tri(p0: DVec3, p1: DVec3, a: DVec3, b: DVec3, c: DVec3) -> bool {
    let d = p1 - p0;
    let (e1, e2) = (b - a, c - a);
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det.abs() <= 1e-14 {
        return false;
    }
    let inv = 1.0 / det;
    let tv = p0 - a;
    let u = tv.dot(pv) * inv;
    let qv = tv.cross(e1);
    let w = d.dot(qv) * inv;
    let t = e2.dot(qv) * inv;
    let eps = 1e-12;
    u >= -eps && w >= -eps && u + w <= 1.0 + eps && t >= -eps && t <= 1.0 + eps
}

fn tri_tri(v: &[DVec3], ta: [u32; 3], tb: [u32; 3]) -> bool {
    let pa = [v[ta[0] as usize], v[ta[1] as usize], v[ta[2] as usize]];
    let pb = [v[tb[0] as usize], v[tb[1] as usize], v[tb[2] as usize]];
    for (x, y) in [(pa, pb), (pb, pa)] {
        for k in 0..3 {
            if edge_hits_tri(x[k], x[(k + 1) % 3], y[0], y[1], y[2]) {
                return true;
            }
        }
    }
    false
}

// ---- numpy helpers -----------------------------------------------------------------------------

/// `numpy.interp(x, xp, fp, period=period)`.
pub fn interp_periodic(x: &[f64], xp: &[f64], fp: &[f64], period: f64) -> Vec<f64> {
    let m = |a: f64| a.rem_euclid(period);
    let mut pairs: Vec<(f64, f64)> = xp.iter().zip(fp).map(|(&a, &b)| (m(a), b)).collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut xs = Vec::with_capacity(pairs.len() + 2);
    let mut fs = Vec::with_capacity(pairs.len() + 2);
    if let (Some(first), Some(last)) = (pairs.first().copied(), pairs.last().copied()) {
        xs.push(last.0 - period);
        fs.push(last.1);
        for (a, b) in &pairs {
            xs.push(*a);
            fs.push(*b);
        }
        xs.push(first.0 + period);
        fs.push(first.1);
    }
    x.iter().map(|&v| interp1(m(v), &xs, &fs)).collect()
}

/// `numpy.interp` of one value (`xp` sorted).
pub fn interp1(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if n == 0 {
        return f64::NAN;
    }
    if x.is_nan() {
        return x;
    }
    if x < xp[0] {
        return fp[0];
    }
    if x >= xp[n - 1] {
        return fp[n - 1];
    }
    // largest j with xp[j] <= x
    let j = xp.partition_point(|&a| a <= x) - 1;
    if x == xp[j] {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    slope * (x - xp[j]) + fp[j]
}

/// `numpy.unwrap` of angles (radians).
pub fn unwrap(p: &[f64]) -> Vec<f64> {
    use std::f64::consts::PI;
    let mut out = p.to_vec();
    let mut acc = 0.0;
    for i in 1..p.len() {
        let d = p[i] - p[i - 1];
        let mut dd = (d + PI).rem_euclid(2.0 * PI) - PI;
        if dd == -PI && d > 0.0 {
            dd = PI;
        }
        let corr = if d.abs() < PI { 0.0 } else { dd - d };
        acc += corr;
        out[i] = p[i] + acc;
    }
    out
}

/// Median (`numpy.median`: mean of the two middle values of an even count).
pub fn median(x: &[f64]) -> f64 {
    if x.is_empty() {
        return f64::NAN;
    }
    let mut s = x.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    }
}

/// At most `n` items, evenly spread in their order (`mu.subsample`).
pub fn subsample<T: Copy>(x: &[T], n: usize) -> Vec<T> {
    if x.len() <= n {
        return x.to_vec();
    }
    if n == 1 {
        return vec![x[0]];
    }
    let step = (x.len() - 1) as f64 / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let k = if i == n - 1 {
                x.len() - 1
            } else {
                (i as f64 * step) as usize
            };
            x[k]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetra() -> (Vec<DVec3>, Vec<[u32; 3]>) {
        (
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        )
    }

    #[test]
    fn edges_loops_and_volume() {
        let (v, t) = tetra();
        assert_eq!(unique_edges(&t).len(), 6);
        assert!(boundary_loops(&t).is_empty());
        assert!((signed_volume(&v, &t) - 1.0 / 6.0).abs() < 1e-12);
        let open = &t[1..];
        let loops = boundary_loops(open);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 3);
        assert_eq!(self_intersections(&v, &t), 0);
    }

    #[test]
    fn crossing_triangles_are_found() {
        let v = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
            DVec3::new(0.5, 0.5, -1.0),
            DVec3::new(0.5, 0.5, 1.0),
            DVec3::new(1.5, 1.5, 1.0),
        ];
        let t = vec![[0, 1, 2], [3, 4, 5]];
        assert_eq!(self_intersecting_pairs(&v, &t), vec![(0, 1)]);
    }

    #[test]
    fn kd_tree_matches_brute_force() {
        let pts: Vec<DVec3> = (0..500)
            .map(|i| {
                let f = i as f64;
                DVec3::new(
                    (f * 0.37).sin() * 5.0,
                    (f * 0.11).cos() * 3.0,
                    (f * 0.07).sin(),
                )
            })
            .collect();
        let tree = KdTree::new(&pts);
        for k in 0..50 {
            let q = DVec3::new(k as f64 * 0.2 - 5.0, 0.3, -0.2);
            let brute = pts
                .iter()
                .enumerate()
                .map(|(i, p)| (i, (*p - q).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            let got = tree.nearest(q).unwrap();
            assert_eq!(got.0, brute.0);
            assert!((got.1 - brute.1).abs() < 1e-12);
        }
    }

    #[test]
    fn numpy_helpers() {
        let x = interp_periodic(&[0.25, 0.9], &[0.0, 0.5], &[1.0, 3.0], 1.0);
        assert!(
            (x[0] - 2.0).abs() < 1e-12 && (x[1] - 1.4).abs() < 1e-12,
            "{x:?}"
        );
        let u = unwrap(&[3.0, -3.0]);
        assert!((u[1] - (2.0 * std::f64::consts::PI - 3.0)).abs() < 1e-12);
        assert_eq!(median(&[3.0, 1.0, 2.0, 4.0]), 2.5);
        assert_eq!(
            subsample(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], 4),
            vec![0, 3, 6, 9]
        );
    }

    #[test]
    fn cut_splits_a_square() {
        let v = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        ];
        let t = vec![[0, 1, 2], [0, 2, 3]];
        let phi: Vec<f64> = v.iter().map(|p| p.x - 0.5).collect();
        let cut = cut_mesh(&v, &t, &phi, 0.12);
        assert_eq!(cut.tris.len(), 6);
        assert_eq!(cut.vertices.len(), 4 + 3);
        let pos: Vec<_> = cut.tris.iter().zip(&cut.side).filter(|x| *x.1).collect();
        assert_eq!(pos.len(), 3);
    }
}
