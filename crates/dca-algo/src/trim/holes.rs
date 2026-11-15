//! Holes through the wall of a trimmed shell (`case_print._plug_holes`,
//! `_window_cuts`, `_open_holes`): a handle of the surface (a thin neck of
//! the wall perforated where the border folds, a tunnel of a voxel or two
//! where two walls meet) is plugged where it is when its shortest loop is
//! short ([`plug_holes`]); a larger one is opened to the border by a slot of
//! the trim surface ([`window_cuts`], [`apply_slots`]), the shell built
//! again with it.
//!
//! The loops come from the topology of the surface: two per handle from a
//! tree-cotree decomposition ([`handle_loops`]), then the shortest loops
//! near each one ([`short_loops`]). A plug takes off the ring of faces along
//! a loop and closes the two openings by fans to their centres, or else by
//! the triangles of least area ([`cap_loop`]), nothing else moved.
//!
//! The functions follow the plugin's numpy step by step (the same orders,
//! ties and sums), so that the loops, the caps and the slots are the
//! plugin's (`tests/holes_oracle.rs`). Deviations: the distances from the
//! model are single precision (the BVH), and a plug whose every cap crosses
//! the wall is made by meshing the plugged surface again on the voxels of
//! the shell (the plugin: OpenVDB on the same voxels).

use std::collections::HashMap;

use dc_geom::bvh::Bvh;
use dc_geom::knn::KnnIndex;
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};

use super::grid::PlanGrid;
use super::warp::PathWarp;

/// A hole through the wall is opened to the border in at most this many
/// builds (`TRIM_HOLE_ROUNDS`) ...
pub const ROUNDS: usize = 3;
/// ... by a slot this half wide (at least 1.5 voxels), wider at every round
/// (`TRIM_HOLE_SLOT_MM`) ...
pub const SLOT_MM: f64 = 0.45;
/// ... only through a neck at most this wide, a thin wall perforated
/// (`TRIM_HOLE_BRIDGE_MM`) ...
pub const BRIDGE_MM: f64 = 2.0;
/// ... a hole of the outer side of the shell this long around at least
/// (less: voxel noise; `TRIM_HOLE_MIN_MM`) ...
pub const MIN_MM: f64 = 1.5;
/// ... taking off at most this share of the volume, every cap kept
/// (`TRIM_HOLE_VOLUME_SHARE`).
pub const VOLUME_SHARE: f64 = 0.05;
/// A hole whose shortest loop is at most this long around is plugged where
/// it is (`TRIM_HOLE_PLUG_MM`) ...
pub const PLUG_MM: f64 = 3.5;
/// ... at most this long when the loop lies on the outer side of the wall
/// only (`TRIM_HOLE_PLUG_OUTER_MM`).
pub const PLUG_OUTER_MM: f64 = 6.0;
/// A plug whose caps cross the faces within this (mm) of them is not made
/// (`TRIM_HOLE_CAP_PAD_MM`).
pub const CAP_PAD_MM: f64 = 0.5;
/// An opening capped by the triangles of least area has at most this many
/// vertices (`TRIM_HOLE_CAP_MAX`).
pub const CAP_MAX: usize = 400;
/// The shortest loops are searched within this of a loop of the handle
/// (`TRIM_HOLE_CROP_MM`) ...
pub const CROP_MM: f64 = 2.0;
/// ... through this many vertices along it (`TRIM_HOLE_ROOTS`).
pub const ROOTS: usize = 6;
/// Shortest loops kept per loop of a handle (`_short_loops`, `keep`).
pub const KEEP: usize = 4;

/// The sum of `a` as numpy sums a contiguous float64 array (pairwise, eight
/// accumulators, blocks of 128): the lengths of the loops bit for bit.
pub fn np_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        a.iter().fold(-0.0, |s, &v| s + v)
    } else if n <= 128 {
        let mut r = [0.0; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i < n - n % 8 {
            for (j, rj) in r.iter_mut().enumerate() {
                *rj += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for &v in &a[i..] {
            res += v;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        np_sum(&a[..n2]) + np_sum(&a[n2..])
    }
}

/// The directed edges of every face (`mu.face_edges`): `(t0, t1)` of every
/// face, then `(t1, t2)`, then `(t2, t0)`; edge `i` is of face `i % m`.
fn face_edges(tris: &[[u32; 3]]) -> Vec<[u32; 2]> {
    let mut e = Vec::with_capacity(3 * tris.len());
    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
        e.extend(tris.iter().map(|t| [t[a], t[b]]));
    }
    e
}

fn key(a: u32, b: u32, n: usize) -> u64 {
    u64::from(a.min(b)) * (n as u64 + 1) + u64::from(a.max(b))
}

/// The root of `x` with path halving (the plugin's `find`).
fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// The label of every node (`mu.connected_components`): the smallest node of
/// its component.
fn components(n: usize, edges: &[[u32; 2]]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n).collect();
    for e in edges {
        let (a, b) = (
            find(&mut parent, e[0] as usize),
            find(&mut parent, e[1] as usize),
        );
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..n).map(|x| find(&mut parent, x)).collect()
}

/// The piece of every face (`_face_pieces`): faces joined across an edge of
/// exactly two of them; the label is the smallest face of the piece.
pub fn face_pieces(tris: &[[u32; 3]], n: usize) -> Vec<usize> {
    let m = tris.len();
    let fe = face_edges(tris);
    let mut o: Vec<(u64, usize)> = fe
        .iter()
        .enumerate()
        .map(|(i, e)| (key(e[0], e[1], n), i))
        .collect();
    o.sort_unstable();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < o.len() {
        let mut j = i + 1;
        while j < o.len() && o[j].0 == o[i].0 {
            j += 1;
        }
        if j - i == 2 {
            pairs.push([(o[i].1 % m) as u32, (o[i + 1].1 % m) as u32]);
        }
        i = j;
    }
    components(m, &pairs)
}

fn piece_count(tris: &[[u32; 3]], n: usize) -> usize {
    let mut lab = face_pieces(tris, n);
    lab.sort_unstable();
    lab.dedup();
    lab.len()
}

/// Genus of the closed surface from the Euler characteristic of its pieces
/// (`_genus`): the handles, holes through the wall of a shell.
pub fn genus(n: usize, tris: &[[u32; 3]]) -> i64 {
    if tris.is_empty() {
        return 0;
    }
    let mut used = vec![false; n];
    for t in tris {
        for &v in t {
            used[v as usize] = true;
        }
    }
    let mut keys: Vec<u64> = face_edges(tris)
        .iter()
        .map(|e| key(e[0], e[1], n))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let used = used.iter().filter(|u| **u).count() as f64;
    let pieces = piece_count(tris, n) as f64;
    (pieces - (used - keys.len() as f64 + tris.len() as f64) / 2.0).round() as i64
}

/// Spanning tree, breadth first from `root`, of the graph of `n` nodes and
/// the edges `(a[k], b[k])` (`_bfs_tree`): the edge to its parent of every
/// node (`None`: the root, or a node not reached) and the nodes in the order
/// they were reached. A level is taken in increasing order of its nodes,
/// every new node by the first edge reaching it.
pub fn bfs_tree(n: usize, a: &[u32], b: &[u32], root: usize) -> (Vec<Option<u32>>, Vec<u32>) {
    let mut ptr = vec![0usize; n + 1];
    for &x in a.iter().chain(b) {
        ptr[x as usize + 1] += 1;
    }
    for i in 0..n {
        ptr[i + 1] += ptr[i];
    }
    let mut fill = ptr.clone();
    let mut adj = vec![(0u32, 0u32); ptr[n]];
    for (k, (&x, &y)) in a.iter().zip(b).enumerate() {
        adj[fill[x as usize]] = (y, k as u32);
        fill[x as usize] += 1;
    }
    for (k, (&x, &y)) in a.iter().zip(b).enumerate() {
        adj[fill[y as usize]] = (x, k as u32);
        fill[y as usize] += 1;
    }
    let mut parent = vec![None; n];
    let mut order = Vec::new();
    if root >= n {
        return (parent, order);
    }
    let mut seen = vec![false; n];
    seen[root] = true;
    order.push(root as u32);
    let mut front = vec![root as u32];
    while !front.is_empty() {
        let mut cand: Vec<(u32, u32)> = Vec::new();
        for &v in &front {
            let v = v as usize;
            cand.extend(
                adj[ptr[v]..ptr[v + 1]]
                    .iter()
                    .filter(|c| !seen[c.0 as usize]),
            );
        }
        cand.sort_by_key(|c| c.0);
        cand.dedup_by_key(|c| c.0);
        for &(v, e) in &cand {
            parent[v as usize] = Some(e);
            seen[v as usize] = true;
        }
        front = cand.into_iter().map(|c| c.0).collect();
        order.extend(&front);
    }
    (parent, order)
}

/// The other end of the parent edge of every node.
fn ups(parent: &[Option<u32>], ev: &[[u32; 2]]) -> Vec<Option<u32>> {
    parent
        .iter()
        .enumerate()
        .map(|(v, p)| {
            p.map(|e| {
                let [x, y] = ev[e as usize];
                if x as usize == v { y } else { x }
            })
        })
        .collect()
}

/// The loop through the edge `(x, y)` closed by the tree `up`: from `x` up
/// to the first node shared with the path of `y`, then down to `y`.
fn tree_loop(up: &[Option<u32>], x: u32, y: u32) -> Option<Vec<u32>> {
    let path = |mut v: u32| {
        let mut p = vec![v];
        while let Some(u) = up[v as usize] {
            p.push(u);
            v = u;
        }
        p
    };
    let (pa, pb) = (path(x), path(y));
    let on_a: std::collections::HashSet<u32> = pa.iter().copied().collect();
    let ib = pb.iter().position(|v| on_a.contains(v))?;
    let ia = pa.iter().position(|v| *v == pb[ib])?;
    let mut out = pa[..=ia].to_vec();
    out.extend(pb[..ib].iter().rev());
    Some(out)
}

fn seg_len(a: DVec3, b: DVec3) -> f64 {
    let d = a - b;
    (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
}

/// Loops around or through the handles of the closed surface (2 per
/// handle; `_handle_loops`): tree-cotree, the edges neither in a spanning
/// tree of the vertices (breadth first from vertex 0) nor crossed by a
/// spanning tree of the faces that crosses none of its edges, each closed by
/// the tree; shortest first (length along the open path). Empty unless
/// every edge has exactly two faces.
pub fn handle_loops(pos: &[DVec3], tris: &[[u32; 3]]) -> Vec<Vec<u32>> {
    let (n, m) = (pos.len(), tris.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let fe = face_edges(tris);
    let mut keys: Vec<u64> = fe.iter().map(|e| key(e[0], e[1], n)).collect();
    let all = keys.clone();
    keys.sort_unstable();
    keys.dedup();
    if fe.len() != 2 * keys.len() {
        return Vec::new();
    }
    let n1 = n as u64 + 1;
    let ev: Vec<[u32; 2]> = keys
        .iter()
        .map(|k| [(k / n1) as u32, (k % n1) as u32])
        .collect();
    // the two faces of every edge, in the order of the face edges
    let mut ef = vec![[u32::MAX; 2]; keys.len()];
    let mut cnt = vec![0usize; keys.len()];
    for (i, k) in all.iter().enumerate() {
        let e = keys.binary_search(k).unwrap_or(0);
        if cnt[e] >= 2 {
            return Vec::new();
        }
        ef[e][cnt[e]] = (i % m) as u32;
        cnt[e] += 1;
    }
    let (a, b): (Vec<u32>, Vec<u32>) = ev.iter().map(|e| (e[0], e[1])).unzip();
    let (par, _) = bfs_tree(n, &a, &b, 0);
    let mut tree = vec![false; keys.len()];
    for e in par.iter().flatten() {
        tree[*e as usize] = true;
    }
    let rest: Vec<usize> = (0..keys.len()).filter(|&e| !tree[e]).collect();
    let (fa, fb): (Vec<u32>, Vec<u32>) = rest.iter().map(|&e| (ef[e][0], ef[e][1])).unzip();
    let (fpar, _) = bfs_tree(m, &fa, &fb, 0);
    let mut cotree = vec![false; keys.len()];
    for e in fpar.iter().flatten() {
        cotree[rest[*e as usize]] = true;
    }
    let up = ups(&par, &ev);
    let mut loops: Vec<(f64, Vec<u32>)> = (0..keys.len())
        .filter(|&e| !tree[e] && !cotree[e])
        .filter_map(|e| tree_loop(&up, ev[e][0], ev[e][1]))
        .map(|l| {
            let d: Vec<f64> = l
                .windows(2)
                .map(|w| seg_len(pos[w[1] as usize], pos[w[0] as usize]))
                .collect();
            (np_sum(&d), l)
        })
        .collect();
    loops.sort_by(|x, y| x.0.total_cmp(&y.0));
    loops.into_iter().map(|(_, l)| l).collect()
}

/// The distinct edges of the faces (sorted) and the two faces of every edge
/// held by exactly two (`_edges_two_faces`; `None` where it is not).
pub fn edges_two_faces(tris: &[[u32; 3]], n: usize) -> (Vec<[u32; 2]>, Vec<Option<[u32; 2]>>) {
    let m = tris.len();
    let fe = face_edges(tris);
    let mut o: Vec<(u64, usize)> = fe
        .iter()
        .enumerate()
        .map(|(i, e)| (key(e[0], e[1], n), i))
        .collect();
    o.sort_unstable();
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < o.len() {
        let mut j = i + 1;
        while j < o.len() && o[j].0 == o[i].0 {
            j += 1;
        }
        let e = fe[o[i].1];
        edges.push([e[0].min(e[1]), e[0].max(e[1])]);
        faces.push((j - i == 2).then(|| [(o[i].1 % m) as u32, (o[i + 1].1 % m) as u32]));
        i = j;
    }
    (edges, faces)
}

/// An insertion-ordered map of loops keyed by their sorted vertices.
#[derive(Default)]
struct LoopSet {
    at: HashMap<Vec<u32>, usize>,
    items: Vec<(f64, Vec<u32>)>,
}

impl LoopSet {
    /// `replace`: a loop already there takes the new value (a dict
    /// assignment), else it keeps the first one (`setdefault`).
    fn put(&mut self, len: f64, lp: Vec<u32>, replace: bool) {
        let mut k = lp.clone();
        k.sort_unstable();
        match self.at.get(&k) {
            Some(&i) => {
                if replace {
                    self.items[i] = (len, lp);
                }
            }
            None => {
                self.at.insert(k, self.items.len());
                self.items.push((len, lp));
            }
        }
    }

    /// Shortest first (ties in their order).
    fn sorted(mut self) -> Vec<(f64, Vec<u32>)> {
        self.items.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.items
    }
}

/// Short loops around or through the handles of the closed surface near the
/// loop `seed` (`_short_loops`): on the faces within the box of the seed
/// grown by `pad`, for `roots` vertices along the seed, the greedy system of
/// loops through the vertex (breadth-first tree, measured along it; cotree
/// of the dual edges by decreasing length of their loop). A loop that does
/// not split the crop goes around or through a handle. `(length mm, vertex
/// indices)`, shortest first, at most `keep`.
pub fn short_loops(
    pos: &[DVec3],
    tris: &[[u32; 3]],
    seed: &[u32],
    pad: f64,
    roots: usize,
    keep: usize,
) -> Vec<(f64, Vec<u32>)> {
    if seed.is_empty() {
        return Vec::new();
    }
    let (mut lo, mut hi) = (pos[seed[0] as usize], pos[seed[0] as usize]);
    for &s in seed {
        lo = lo.min(pos[s as usize]);
        hi = hi.max(pos[s as usize]);
    }
    let (lo, hi) = (lo - DVec3::splat(pad), hi + DVec3::splat(pad));
    let inb = |v: u32| {
        let p = pos[v as usize];
        p.cmpge(lo).all() && p.cmple(hi).all()
    };
    let tc: Vec<[u32; 3]> = tris
        .iter()
        .filter(|t| t.iter().all(|&v| inb(v)))
        .copied()
        .collect();
    if tc.is_empty() {
        return Vec::new();
    }
    let mut used: Vec<u32> = tc.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let mut remap = vec![u32::MAX; pos.len()];
    for (i, &v) in used.iter().enumerate() {
        remap[v as usize] = i as u32;
    }
    let tl: Vec<[u32; 3]> = tc.iter().map(|t| t.map(|v| remap[v as usize])).collect();
    let vl: Vec<DVec3> = used.iter().map(|&v| pos[v as usize]).collect();
    let n = used.len();
    let (e, f) = edges_two_faces(&tl, n);
    let len: Vec<f64> = e
        .iter()
        .map(|x| seg_len(vl[x[0] as usize], vl[x[1] as usize]))
        .collect();
    let rs: Vec<u32> = seed
        .iter()
        .map(|&s| remap[s as usize])
        .filter(|&r| r != u32::MAX)
        .collect();
    let (a, b): (Vec<u32>, Vec<u32>) = e.iter().map(|x| (x[0], x[1])).unzip();
    let num = roots.min(rs.len());
    let picks: Vec<u32> = (0..num)
        .map(|i| {
            let last = (rs.len() - 1) as f64;
            let t = if num == 1 {
                0.0
            } else if i == num - 1 {
                last
            } else {
                i as f64 * (last / (num - 1) as f64)
            };
            rs[t as usize]
        })
        .collect();
    let mut out = LoopSet::default();
    for r in picks {
        let (pe, order) = bfs_tree(n, &a, &b, r as usize);
        let up = ups(&pe, &e);
        let mut seen: Vec<bool> = pe.iter().map(Option::is_some).collect();
        seen[r as usize] = true;
        let mut dist = vec![0.0; n];
        for &v in &order {
            if let (Some(u), Some(k)) = (up[v as usize], pe[v as usize]) {
                dist[v as usize] = dist[u as usize] + len[k as usize];
            }
        }
        let mut tree = vec![false; e.len()];
        for k in pe.iter().flatten() {
            tree[*k as usize] = true;
        }
        let mut cand: Vec<usize> = (0..e.len())
            .filter(|&k| {
                !tree[k] && seen[e[k][0] as usize] && seen[e[k][1] as usize] && f[k].is_some()
            })
            .collect();
        let total = |k: usize| dist[e[k][0] as usize] + dist[e[k][1] as usize] + len[k];
        cand.sort_by(|&x, &y| (-total(x)).total_cmp(&(-total(y))));
        let mut parent: Vec<usize> = (0..tl.len()).collect();
        for k in cand {
            let Some([fa, fb]) = f[k] else { continue };
            let (ra, rb) = (
                find(&mut parent, fa as usize),
                find(&mut parent, fb as usize),
            );
            if ra != rb {
                parent[ra] = rb;
                continue;
            }
            let Some(lp) = tree_loop(&up, e[k][0], e[k][1]) else {
                continue;
            };
            let d: Vec<f64> = (0..lp.len())
                .map(|i| seg_len(vl[lp[i] as usize], vl[lp[(i + 1) % lp.len()] as usize]))
                .collect();
            let global: Vec<u32> = lp.iter().map(|&v| used[v as usize]).collect();
            out.put(np_sum(&d), global, true);
        }
    }
    let mut s = out.sorted();
    s.truncate(keep);
    s
}

/// Triangles closing the opening `b` (its vertices in border order) of
/// least total area (`_min_area_cap`: dynamic programming over the polygon,
/// no new vertex), each border edge reversed; no edge inside the cap joins a
/// vertex to itself or is in `forbid` (undirected edges `a * len + b`,
/// `a < b`). `None` when `b` is longer than [`CAP_MAX`] or no such cap
/// exists.
pub fn min_area_cap(
    b: &[u32],
    pos: &[DVec3],
    forbid: &std::collections::HashSet<u64>,
) -> Option<Vec<[u32; 3]>> {
    let m = b.len();
    if !(3..=CAP_MAX).contains(&m) {
        return None;
    }
    let p: Vec<DVec3> = b.iter().map(|&v| pos[v as usize]).collect();
    let n = pos.len() as u64;
    let mut area = vec![0.0f64; m * m];
    let mut split = vec![0usize; m * m];
    for gap in 2..m {
        for i in 0..m - gap {
            let j = i + gap;
            let (mut best, mut q) = (f64::NAN, i + 1);
            for k in i + 1..j {
                let c = (p[k] - p[i]).cross(p[j] - p[i]);
                let a = 0.5 * (c.x * c.x + c.y * c.y + c.z * c.z).sqrt();
                let tot = area[i * m + k] + area[k * m + j] + a;
                // np.argmin: the first smallest (NaN never here)
                if best.is_nan() || tot < best {
                    (best, q) = (tot, k);
                }
            }
            area[i * m + j] = best;
            split[i * m + j] = q;
            let (lo, hi) = (b[i].min(b[j]), b[i].max(b[j]));
            if gap < m - 1 && (lo == hi || forbid.contains(&(u64::from(lo) * n + u64::from(hi)))) {
                area[i * m + j] = f64::INFINITY;
            }
        }
    }
    if !area[m - 1].is_finite() {
        return None;
    }
    let mut out = Vec::new();
    let mut todo = vec![(0usize, m - 1)];
    while let Some((i, j)) = todo.pop() {
        if j - i < 2 {
            continue;
        }
        let k = split[i * m + j];
        out.push([b[j], b[k], b[i]]);
        todo.push((i, k));
        todo.push((k, j));
    }
    Some(out)
}

/// The face adjacency rule of `mu.face_adjacency`: an edge of exactly two
/// faces has a neighbour; the border edges (`mu.boundary_edges`) in the
/// order of the corners of the faces, then of the faces.
fn boundary_edges(tris: &[[u32; 3]], n: usize) -> Vec<[u32; 2]> {
    let mut count: HashMap<u64, u32> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            *count.entry(key(t[k], t[(k + 1) % 3], n)).or_default() += 1;
        }
    }
    let mut out = Vec::new();
    for k in 0..3 {
        for t in tris {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if count[&key(a, b, n)] != 2 {
                out.push([a, b]);
            }
        }
    }
    out
}

/// Ordered loops of the open border (`mu.boundary_loops`): vertex indices in
/// face order, loops of at least 3 vertices.
pub fn boundary_loops(tris: &[[u32; 3]], n: usize) -> Vec<Vec<u32>> {
    let e = boundary_edges(tris, n);
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for &[a, b] in &e {
        next.entry(a).or_default().push(b);
    }
    let mut used: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    let mut loops = Vec::new();
    for &[a0, b0] in &e {
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
            let Some(&c) = next
                .get(&b)
                .and_then(|cs| cs.iter().find(|&&c| !used.contains(&(b, c))))
            else {
                break false;
            };
            (a, b) = (b, c);
        };
        let _ = a;
        if ok && lp.len() >= 3 {
            loops.push(lp);
        }
    }
    loops
}

/// How the two openings of a cut loop are closed ([`cap_loop`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapFill {
    /// A fan to the centre of the opening (one new vertex each).
    Fan,
    /// The triangles of least area on its border ([`min_area_cap`]).
    Area,
}

/// The surface cut along a loop and closed again ([`cap_loop`]).
#[derive(Debug, Clone)]
pub struct Capped {
    /// Vertices (compacted: those of the faces, in their order).
    pub pos: Vec<DVec3>,
    /// Faces.
    pub tris: Vec<[u32; 3]>,
    /// The faces of the two caps.
    pub cap: Vec<bool>,
    /// Genus of the result.
    pub genus: i64,
}

/// The vertices used by `tris` in their order and the faces renumbered
/// (`al._compact`).
fn compact(pos: &[DVec3], tris: &[[u32; 3]]) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    let mut used = vec![false; pos.len()];
    for t in tris {
        for &v in t {
            used[v as usize] = true;
        }
    }
    let mut remap = vec![u32::MAX; pos.len()];
    let mut out = Vec::new();
    for (i, p) in pos.iter().enumerate() {
        if used[i] {
            remap[i] = out.len() as u32;
            out.push(*p);
        }
    }
    (
        out,
        tris.iter().map(|t| t.map(|v| remap[v as usize])).collect(),
    )
}

/// The closed surface cut along `lp` (its ring of faces taken off) and the
/// two openings closed (`_cap_loop`): a tunnel plugged or a strand cut where
/// the loop is, nothing else moved. `None` when the ring does not leave
/// exactly two openings or the result is not one closed, consistently
/// oriented surface of `pieces` pieces (those of the input when `None`).
pub fn cap_loop(
    pos: &[DVec3],
    tris: &[[u32; 3]],
    lp: &[u32],
    fill: CapFill,
    pieces: Option<usize>,
) -> Option<Capped> {
    let n = pos.len();
    let mut on = vec![false; n];
    for &v in lp {
        on[v as usize] = true;
    }
    let t2: Vec<[u32; 3]> = tris
        .iter()
        .filter(|t| !t.iter().any(|&v| on[v as usize]))
        .copied()
        .collect();
    let openings = boundary_loops(&t2, n);
    if openings.len() != 2 {
        return None;
    }
    let mut vn = pos.to_vec();
    let mut tn = t2.clone();
    let mut forbid = std::collections::HashSet::new();
    let fkey = |a: u32, b: u32| u64::from(a.min(b)) * n as u64 + u64::from(a.max(b));
    if fill == CapFill::Area {
        let mut rim = vec![false; n];
        for &v in openings.iter().flatten() {
            rim[v as usize] = true;
        }
        for e in face_edges(&t2) {
            if rim[e[0] as usize] && rim[e[1] as usize] {
                forbid.insert(fkey(e[0], e[1]));
            }
        }
    }
    for b in &openings {
        match fill {
            CapFill::Area => {
                let f = min_area_cap(b, pos, &forbid)?;
                for e in face_edges(&f) {
                    forbid.insert(fkey(e[0], e[1]));
                }
                tn.extend(f);
            }
            CapFill::Fan => {
                let k = vn.len() as u32;
                let c = b.iter().fold(DVec3::ZERO, |s, &v| s + pos[v as usize]) / b.len() as f64;
                vn.push(c);
                let len = b.len();
                tn.extend((0..len).map(|i| [b[(i + 1) % len], b[i], k]));
            }
        }
    }
    let cap: Vec<bool> = (0..tn.len()).map(|i| i >= t2.len()).collect();
    let (v2, t2) = compact(&vn, &tn);
    let n2 = v2.len();
    let mut und: HashMap<u64, u32> = HashMap::new();
    let mut dir = std::collections::HashSet::new();
    for e in face_edges(&t2) {
        *und.entry(key(e[0], e[1], n2)).or_default() += 1;
        if !dir.insert((e[0], e[1])) {
            return None;
        }
    }
    if und.values().any(|&c| c != 2) {
        return None;
    }
    let k2 = piece_count(&t2, n2);
    if k2 != pieces.unwrap_or_else(|| piece_count(tris, n)) {
        return None;
    }
    let genus = (k2 as f64 - (n2 as f64 - 0.5 * t2.len() as f64) / 2.0).round() as i64;
    Some(Capped {
        pos: v2,
        tris: t2,
        cap,
        genus,
    })
}

/// The caps of [`cap_loop`] cross the surface (`_cap_crosses`): a fan over
/// a twisted loop, or the two caps of a ring one face wide, cut through each
/// other or the wall beside them. Only the faces within `pad` of the caps
/// are checked.
pub fn cap_crosses(c: &Capped, pad: f64) -> bool {
    let caps: Vec<DVec3> = c
        .tris
        .iter()
        .zip(&c.cap)
        .filter(|(_, k)| **k)
        .flat_map(|(t, _)| t.map(|v| c.pos[v as usize]))
        .collect();
    let Some(first) = caps.first() else {
        return false;
    };
    let (lo, hi) = caps
        .iter()
        .fold((*first, *first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let (lo, hi) = (lo - DVec3::splat(pad), hi + DVec3::splat(pad));
    let inb = |v: u32| {
        let p = c.pos[v as usize];
        p.cmpge(lo).all() && p.cmple(hi).all()
    };
    let near: Vec<usize> = (0..c.tris.len())
        .filter(|&f| c.tris[f].iter().any(|&v| inb(v)))
        .collect();
    let sub: Vec<[u32; 3]> = near.iter().map(|&f| c.tris[f]).collect();
    let (v, t) = compact(&c.pos, &sub);
    let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t);
    crate::shell::crossing_pairs(&mesh)
        .iter()
        .any(|&(a, b)| c.cap[near[a as usize]] || c.cap[near[b as usize]])
}

/// The model a shell was built from, for the distances of its vertices
/// (`gq.Mesh(*model).nearest`).
pub struct Model {
    trees: Vec<Bvh>,
}

impl Model {
    /// The model of the meshes (closed teeth, gingiva).
    ///
    /// # Errors
    /// Errors of the BVH build.
    pub fn new(meshes: &[&TriMesh]) -> Result<Self, dc_geom::GeomError> {
        let trees = meshes
            .iter()
            .filter(|m| !m.tris.is_empty())
            .map(|m| Bvh::build(m))
            .collect::<Result<_, _>>()?;
        Ok(Self { trees })
    }

    /// Distance of `p` from the model (`inf` beyond `maxd`).
    pub fn distance(&self, p: DVec3, maxd: f64) -> f64 {
        self.trees
            .iter()
            .filter_map(|t| t.closest_point(p.as_vec3(), maxd as f32))
            .map(|h| f64::from(h.distance))
            .fold(f64::INFINITY, f64::min)
    }
}

/// The design of the shell the holes are in.
#[derive(Debug, Clone, Copy)]
pub struct Design {
    /// Outer offset of the shell from the model (clearance + wall, mm).
    pub outer: f64,
    /// Inner offset (the clearance, mm).
    pub clearance: f64,
    /// Radius of the rounded rim (mm).
    pub rim: f64,
    /// Voxel of the shell (mm).
    pub voxel: f64,
}

/// Holes through the wall closed where they are (`_plug_holes`): the
/// shortest loop of a handle at most [`PLUG_MM`] long ([`PLUG_OUTER_MM`] on
/// the outer side of the wall only) is cut and capped. A cap stays as far
/// from the model as its loop, less a voxel; caps that would cross the wall
/// beside them are never left in the shell (when every cap crosses, the
/// first one plugged is meshed again on the voxels of the shell). `genus`:
/// that of `mesh` when known. Returns the mesh, its genus and the plugs.
pub fn plug_holes(
    mesh: TriMesh,
    model: &Model,
    d: Design,
    genus: Option<i64>,
) -> (TriMesh, i64, usize) {
    let mut pos: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let mut tris = mesh.tris.clone();
    let mut g = genus.unwrap_or_else(|| self::genus(pos.len(), &tris));
    let mut plugs = 0;
    if g <= 0 {
        return (mesh, g, 0);
    }
    // the outer side of the wall: also the rounded edge of the rim beside it on a thin wall
    let wall = d.outer - d.clearance;
    let rim = if super::shell_fills(wall, d.rim).is_empty() {
        0.0
    } else {
        d.rim
    };
    let tol = 0.5 * d.voxel + 0.03;
    let maxd = d.outer + 1.0;
    let mut changed = false;
    for _ in 0..(g + 2) {
        if g <= 0 {
            break;
        }
        let mut short = LoopSet::default();
        for l in handle_loops(&pos, &tris) {
            for (len, s) in short_loops(&pos, &tris, &l, CROP_MM, ROOTS, KEEP) {
                short.put(len, s, true);
            }
        }
        let mut pieces = None;
        let (mut done, mut crossing) = (false, None);
        'loops: for (len, s) in short.sorted() {
            if len > PLUG_MM.max(PLUG_OUTER_MM) {
                break;
            }
            let ds: Vec<f64> = s
                .iter()
                .map(|&v| model.distance(pos[v as usize], maxd))
                .collect();
            let outer_side = ds
                .iter()
                .all(|&x| (d.outer - rim - tol..=d.outer + tol).contains(&x));
            if len > PLUG_MM && !outer_side {
                continue;
            }
            let dmin = ds.iter().copied().fold(f64::INFINITY, f64::min);
            let pc = *pieces.get_or_insert_with(|| piece_count(&tris, pos.len()));
            for fill in [CapFill::Fan, CapFill::Area] {
                let Some(c) = cap_loop(&pos, &tris, &s, fill, Some(pc)) else {
                    continue;
                };
                // the centres of the two fans; the centres of the faces of the caps of least area
                let centres: Vec<DVec3> = match fill {
                    CapFill::Fan => c.pos[c.pos.len() - 2..].to_vec(),
                    CapFill::Area => c
                        .tris
                        .iter()
                        .zip(&c.cap)
                        .filter(|(_, k)| **k)
                        .map(|(t, _)| {
                            (c.pos[t[0] as usize] + c.pos[t[1] as usize] + c.pos[t[2] as usize])
                                / 3.0
                        })
                        .collect(),
                };
                let dc = centres
                    .iter()
                    .map(|&p| model.distance(p, maxd))
                    .fold(f64::INFINITY, f64::min);
                if c.genus >= g || dc < dmin - d.voxel {
                    continue;
                }
                if cap_crosses(&c, CAP_PAD_MM) {
                    crossing.get_or_insert(c);
                    continue;
                }
                (pos, tris, g) = (c.pos, c.tris, c.genus);
                plugs += 1;
                (done, changed) = (true, true);
                break 'loops;
            }
        }
        if !done && let Some(c) = crossing {
            // every cap crosses the wall beside it: the first one plugged and the
            // surface meshed again on the voxels of the shell
            if let Some((p2, t2)) = remeshed(&c, d.voxel) {
                let g2 = self::genus(p2.len(), &t2);
                if g2 < g {
                    (pos, tris, g) = (p2, t2, g2);
                    plugs += 1;
                    (done, changed) = (true, true);
                }
            }
        }
        if !done {
            break;
        }
    }
    if !changed {
        return (mesh, g, plugs);
    }
    (
        TriMesh::new(pos.iter().map(|p| p.as_vec3()).collect(), tris),
        g,
        plugs,
    )
}

/// The capped surface meshed again on voxels of `voxel` as one clean solid
/// (its largest piece), or `None` when that is not a closed manifold.
fn remeshed(c: &Capped, voxel: f64) -> Option<(Vec<DVec3>, Vec<[u32; 3]>)> {
    let mesh = TriMesh::new(c.pos.iter().map(|p| p.as_vec3()).collect(), c.tris.clone());
    let vs = voxel as f32;
    let g = SdfGrid::from_mesh(
        &mesh,
        &SdfParams {
            voxel_size: vs,
            band: 3.0 * vs,
        },
    )
    .ok()?;
    let out = g.to_mesh().ok()?;
    let (out, _) = super::largest_piece(out);
    let r = out.validate();
    if !(r.is_watertight() && r.is_manifold()) || out.tris.is_empty() {
        return None;
    }
    Some((
        out.positions.iter().map(|p| p.as_dvec3()).collect(),
        out.tris,
    ))
}

/// A slot of the trim surface opening a hole through the wall to the border
/// (`cuts` of `aligner.trim_cutter`): within `half` of the segment
/// `p0`-`p1` (plan, the warped space of the insertion paths) the surface
/// rises to `z` at least.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Start of the segment (plan).
    pub p0: DVec2,
    /// End of the segment (plan).
    pub p1: DVec2,
    /// Height the surface rises to (mm).
    pub z: f64,
    /// Half width (mm).
    pub half: f64,
}

/// `np.round(x, 4)`.
fn round4(x: f64) -> f64 {
    (x * 1e4).round_ties_even() / 1e4
}

/// The slots raising the trim surface `z` of `grid` (`trim_cutter`, `cuts`):
/// at most `z_top - 1`.
pub fn apply_slots(grid: &PlanGrid, z: &mut [f64], slots: &[Slot], z_top: f64) {
    for s in slots {
        let ab = s.p1 - s.p0;
        let den = ab.dot(ab).max(1e-12);
        for (k, zk) in z.iter_mut().enumerate() {
            let p = grid.node(k);
            let t = ((p - s.p0).dot(ab) / den).clamp(0.0, 1.0);
            let q = p - (s.p0 + t * ab);
            if (q.x * q.x + q.y * q.y).sqrt() <= s.half {
                *zk = zk.max(s.z).min(z_top - 1.0);
            }
        }
    }
}

/// Slots opening the holes through the wall of the shell `mesh` to its
/// border (`_window_cuts`). Every hole is found by the shortest loop of its
/// handle, or else by a loop of the outer side of the shell (its faces at
/// the outer offset from the model: one sheet bounded by the border, one
/// more loop around every hole, [`MIN_MM`] of perimeter at least). The slot
/// goes from the hole to the nearest point of the border, at most
/// [`BRIDGE_MM`] away, up to the top of the hole where it meets it (warped
/// space: along the path); a loop along the border is cut along itself
/// (slots about 1 mm long, each up to the top of the loop there).
pub fn window_cuts(
    mesh: &TriMesh,
    model: &Model,
    d: Design,
    warp: Option<&PathWarp>,
    half: f64,
) -> Vec<Slot> {
    let pos: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let tris = &mesh.tris;
    if tris.is_empty() {
        return Vec::new();
    }
    let n = pos.len();
    let maxd = d.outer + 1.0;
    let tol = 0.5 * d.voxel + 0.03;
    let sheet: Vec<[u32; 3]> = tris
        .iter()
        .filter(|t| {
            let c = (pos[t[0] as usize] + pos[t[1] as usize] + pos[t[2] as usize]) / 3.0;
            (model.distance(c, maxd) - d.outer).abs() <= tol
        })
        .copied()
        .collect();
    if sheet.is_empty() {
        return Vec::new();
    }
    // the outer sheet: the largest piece (ties: the smallest label)
    let lab = face_pieces(&sheet, n);
    let mut count: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for &l in &lab {
        *count.entry(l).or_default() += 1;
    }
    let mut big = (0, 0);
    for (&l, &c) in &count {
        if c > big.1 {
            big = (l, c);
        }
    }
    let sheet: Vec<[u32; 3]> = sheet
        .iter()
        .zip(&lab)
        .filter(|(_, l)| **l == big.0)
        .map(|(t, _)| *t)
        .collect();
    // its boundary edges, sorted
    let mut keys: Vec<u64> = face_edges(&sheet)
        .iter()
        .map(|e| key(e[0], e[1], n))
        .collect();
    keys.sort_unstable();
    let n1 = n as u64 + 1;
    let mut b: Vec<[u32; 2]> = Vec::new();
    let mut i = 0;
    while i < keys.len() {
        let mut j = i + 1;
        while j < keys.len() && keys[j] == keys[i] {
            j += 1;
        }
        if j - i == 1 {
            b.push([(keys[i] / n1) as u32, (keys[i] % n1) as u32]);
        }
        i = j;
    }
    if b.is_empty() {
        return Vec::new();
    }
    let comp = components(n, &b);
    let mut roots: Vec<usize> = b.iter().map(|e| comp[e[0] as usize]).collect();
    let edge_root = roots.clone();
    roots.sort_unstable();
    roots.dedup();
    let inv: Vec<usize> = edge_root
        .iter()
        .map(|r| roots.binary_search(r).unwrap_or(0))
        .collect();
    let mut per = vec![0.0; roots.len()];
    for (e, &l) in b.iter().zip(&inv) {
        per[l] += seg_len(pos[e[0] as usize], pos[e[1] as usize]);
    }
    let mut border = 0;
    for (l, &p) in per.iter().enumerate() {
        if p > per[border] {
            border = l;
        }
    }
    let vw: Vec<DVec3> = match warp {
        Some(w) => pos.iter().map(|&p| w.to_warp(p)).collect(),
        None => pos.clone(),
    };
    let verts_of = |l: usize| -> Vec<u32> {
        let mut v: Vec<u32> = b
            .iter()
            .zip(&inv)
            .filter(|(_, k)| **k == l)
            .flat_map(|(e, _)| *e)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let ring = verts_of(border);
    let band = super::rim_round(d.outer - d.clearance, None) + 2.0 * d.voxel;
    // the handles themselves (their shorter loops: one per handle); the holes of the outer side when the
    // loops cannot be found
    let loops = handle_loops(&pos, tris);
    let mut short = LoopSet::default();
    for l in &loops {
        for (len, s) in short_loops(&pos, tris, l, CROP_MM, ROOTS, KEEP) {
            short.put(len, s, false);
        }
    }
    let short: Vec<Vec<u32>> = short.sorted().into_iter().map(|(_, s)| s).collect();
    let mut holes: Vec<Vec<u32>> = if short.is_empty() {
        loops.clone()
    } else {
        short
    };
    holes.truncate(loops.len() / 2);
    if holes.is_empty() {
        holes = (0..roots.len())
            .filter(|&l| per[l] >= MIN_MM && l != border)
            .map(verts_of)
            .collect();
    }
    let mut out = Vec::new();
    for hole in &holes {
        // every point of the border: its nearest point of the hole (ties to the first)
        let (mut bi, mut bd, mut bj) = (0usize, f64::INFINITY, 0usize);
        for (ri, &r) in ring.iter().enumerate() {
            let q = vw[r as usize];
            let (mut jd, mut jj) = (f64::INFINITY, 0usize);
            for (hi, &h) in hole.iter().enumerate() {
                let dd = vw[h as usize] - q;
                let s = dd.x * dd.x + dd.y * dd.y + dd.z * dd.z;
                if s < jd {
                    (jd, jj) = (s, hi);
                }
            }
            let jd = jd.sqrt();
            if jd < bd {
                (bi, bd, bj) = (ri, jd, jj);
            }
        }
        if bd > BRIDGE_MM {
            // not a thin neck perforated: left as it is (reported)
            continue;
        }
        let (pw, po) = (vw[hole[bj] as usize], vw[ring[bi] as usize]);
        if bd < 2.0 * d.voxel {
            // a loop that runs along the border: slots along the loop itself, about 1 mm each, up to its top
            let q: Vec<DVec3> = hole.iter().map(|&h| vw[h as usize]).collect();
            let mut t = vec![0.0];
            for i in 0..q.len() {
                let next = q[(i + 1) % q.len()];
                let s = seg_len(next, q[i]);
                t.push(t[i] + s);
            }
            let total = t[t.len() - 1];
            // np.searchsorted(t, np.arange(0, total, 1)), unique
            let steps = total.ceil().max(0.0) as usize;
            let mut ks: Vec<usize> = (0..steps)
                .map(|s| t.partition_point(|&v| v < s as f64))
                .collect();
            ks.dedup();
            let mut ks: Vec<usize> = ks.into_iter().filter(|&k| k < q.len()).collect();
            ks.push(q.len());
            for w in ks.windows(2) {
                let (i0, i1) = (w[0], w[1]);
                let mut piece: Vec<DVec3> = q[i0..i1].to_vec();
                piece.push(q[i1 % q.len()]);
                let top = piece.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
                let (a, z) = (piece[0], piece[piece.len() - 1]);
                out.push(Slot {
                    p0: DVec2::new(round4(a.x), round4(a.y)),
                    p1: DVec2::new(round4(z.x), round4(z.y)),
                    z: round4(top + band),
                    half,
                });
            }
            continue;
        }
        let u = (po - pw).truncate();
        let u = u / (u.x * u.x + u.y * u.y).sqrt().max(1e-9);
        let top = hole
            .iter()
            .map(|&h| vw[h as usize])
            .filter(|p| {
                let q = p.truncate() - pw.truncate();
                (q.x * q.x + q.y * q.y).sqrt() <= 2.0 * half
            })
            .map(|p| p.z)
            .fold(f64::NEG_INFINITY, f64::max);
        let z = top.max(po.z) + band;
        let (a, e) = (pw.truncate() - half * u, po.truncate() + half * u);
        out.push(Slot {
            p0: DVec2::new(round4(a.x), round4(a.y)),
            p1: DVec2::new(round4(e.x), round4(e.y)),
            z: round4(z),
            half,
        });
    }
    out
}

/// Teeth whose cap is not in the shell (`_uncovered_teeth`): the 30 highest
/// scanned points of a covered crown (20 at least) farther than `reach` from
/// every second vertex of the shell (median).
pub fn uncovered_teeth(mesh: &TriMesh, teeth: &[super::Tooth], reach: f64) -> Vec<dc_case::Fdi> {
    let half: Vec<Vec3> = mesh.positions.iter().step_by(2).copied().collect();
    let Ok(index) = KnnIndex::build(&half) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for t in teeth.iter().filter(|t| t.covered && t.crown.len() >= 20) {
        let mut idx: Vec<usize> = (0..t.crown.len()).collect();
        // np.argsort (stable for ties of the heights); the 30 highest
        idx.sort_by(|&a, &b| t.crown[a].z.total_cmp(&t.crown[b].z));
        let top = &idx[idx.len().saturating_sub(30)..];
        let mut d: Vec<f64> = top
            .iter()
            .map(|&i| {
                index
                    .nearest(t.crown[i].as_vec3())
                    .map_or(f64::INFINITY, |nb| f64::from(nb.distance))
            })
            .collect();
        d.sort_by(f64::total_cmp);
        let m = d.len();
        let median = if m % 2 == 1 {
            d[m / 2]
        } else {
            0.5 * (d[m / 2 - 1] + d[m / 2])
        };
        if median > reach {
            out.push(t.fdi);
        }
    }
    out
}

/// Volume enclosed by a closed mesh (`mu.mesh_volume`).
pub fn volume(mesh: &TriMesh) -> f64 {
    mesh.tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| mesh.positions[v as usize].as_dvec3());
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        .abs()
        / 6.0
}
