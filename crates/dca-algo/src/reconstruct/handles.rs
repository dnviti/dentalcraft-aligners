//! Handles of an open crown: tunnels through the scanned surface.
//!
//! The surface a scanner builds can hold tunnels a few triangles wide where
//! its two sides come close: through a thin incisal edge, under a cusp tip,
//! across an occlusal fissure (measured on real cases: loops of 0.08 to
//! 3 mm around them, on incisors, canines and molars of both arches).
//! Inside one crown such a tunnel is a handle of the crown surface, and the
//! crown closed around it is a solid with a hole through it: not one solid
//! for a slicer, the print check or the aligner built on it.
//!
//! [`cut`] takes every handle away where it is narrowest: the shortest loop
//! of edges that does not split the crown. A loop is told by its class
//! (Z/2 homology of the crown with its holes closed: a spanning tree of the
//! vertices, a spanning tree of the faces and holes across the other edges,
//! a bit for every edge left out of both, and every edge of the second tree
//! the sum of the bits on its side): a loop whose edge classes do not add up
//! to zero does not split the crown. The shortest of them is looked for from
//! every vertex in turn (shortest paths from it, a loop over every edge
//! whose two paths differ in class), never farther than half the shortest
//! loop found so far; the loops of a greedy system (Erickson and Whittlesey
//! 2005: from a base point, the loops over the edges left out of its tree of
//! shortest paths and of the longest-first spanning tree of the faces) from
//! base points spread over the crown bound it first, and are tried next. The
//! triangles touching the loop are taken away, which leaves two small holes
//! in place of the handle, and the next loop is looked for until no handle
//! is left. Each hole a cut leaves is then closed by a fan to its centre,
//! kept where the scan was (the fairing of the closed tooth does not move
//! it). A crown without handles is left as it is.
//!
//! Departure from the plugin: `reconstruct.closed_tooth` keeps the handles of
//! the scan, and its closed tooth has the tunnels.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use glam::DVec3;

use super::geom;

/// Base points spread over the crown the greedy loops start from.
const BASES: usize = 16;
/// Loops tried in turn, shortest first, when cutting one does not take a
/// handle away.
const TRIES: usize = 8;
/// Share of the crown surface the cuts may take away; past it the crown is
/// left as it is (and the closed tooth fails by name).
const MAX_CUT_SHARE: f64 = 0.1;
/// No vertex, face or hole.
const NONE: u32 = u32::MAX;

/// An open crown with its handles cut away ([`cut`]).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Cut {
    /// The vertices kept, then the centre of every hole closed.
    pub vertices: Vec<DVec3>,
    /// The triangles.
    pub tris: Vec<[u32; 3]>,
    /// Index in the input of every vertex kept (the first `kept.len()`
    /// vertices).
    pub kept: Vec<u32>,
    /// Indices in the input of the border of the hole every centre closes
    /// (the vertices after the kept ones, in order).
    pub caps: Vec<Vec<u32>>,
    /// Handles taken away.
    pub handles: usize,
}

/// The undirected edges of triangles and their faces.
struct Edges {
    /// Ends `(min, max)`, sorted.
    ends: Vec<[u32; 2]>,
    /// The first two faces of every edge (`NONE` for an edge of one face).
    faces: Vec<[u32; 2]>,
    /// Number of faces of every edge.
    count: Vec<u32>,
}

impl Edges {
    fn of(t: &[[u32; 3]]) -> Self {
        let mut keys: Vec<(u64, u32)> = Vec::with_capacity(3 * t.len());
        for (f, tri) in t.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                keys.push(((u64::from(a.min(b)) << 32) | u64::from(a.max(b)), f as u32));
            }
        }
        keys.sort_unstable();
        let mut e = Self {
            ends: Vec::new(),
            faces: Vec::new(),
            count: Vec::new(),
        };
        for run in keys.chunk_by(|x, y| x.0 == y.0) {
            let k = run[0].0;
            e.ends.push([(k >> 32) as u32, k as u32]);
            e.faces.push([run[0].1, run.get(1).map_or(NONE, |x| x.1)]);
            e.count.push(run.len() as u32);
        }
        e
    }

    /// The end of edge `i` that is not `x`.
    fn other(&self, i: u32, x: u32) -> u32 {
        let [a, b] = self.ends[i as usize];
        if a == x { b } else { a }
    }
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Joins the sets of `a` and `b`; `false` when they were one set already.
fn union(parent: &mut [u32], a: u32, b: u32) -> bool {
    let (a, b) = (find(parent, a), find(parent, b));
    if a == b {
        return false;
    }
    parent[a.max(b) as usize] = a.min(b);
    true
}

/// The faces to drop so that the faces around every vertex make one fan
/// (faces joined through their edges at the vertex): where they make
/// several (two parts of the surface touching at a point), every fan but the
/// largest goes (the first of the largest on a tie).
fn pinched(t: &[[u32; 3]], e: &Edges) -> Vec<bool> {
    // corner 3 f + k: vertex t[f][k] in face f
    let corner = |f: u32, v: u32| {
        let k = t[f as usize].iter().position(|&x| x == v).unwrap_or(0);
        3 * f + k as u32
    };
    let mut parent: Vec<u32> = (0..3 * t.len() as u32).collect();
    for (i, ends) in e.ends.iter().enumerate() {
        if e.count[i] == 2 {
            let [f, g] = e.faces[i];
            for &v in ends {
                union(&mut parent, corner(f, v), corner(g, v));
            }
        }
    }
    // (vertex, fan, face) of every corner, grouped by vertex then fan
    let mut fans: Vec<(u32, u32, u32)> = (0..3 * t.len() as u32)
        .map(|c| {
            let v = t[(c / 3) as usize][(c % 3) as usize];
            (v, find(&mut parent, c), c / 3)
        })
        .collect();
    fans.sort_unstable();
    let mut drop = vec![false; t.len()];
    for at in fans.chunk_by(|a, b| a.0 == b.0) {
        let fan: Vec<&[(u32, u32, u32)]> = at.chunk_by(|a, b| a.1 == b.1).collect();
        if fan.len() < 2 {
            continue;
        }
        let mut keep = 0;
        for (i, f) in fan.iter().enumerate() {
            if f.len() > fan[keep].len() {
                keep = i;
            }
        }
        for (i, f) in fan.iter().enumerate() {
            if i != keep {
                for c in *f {
                    drop[c.2 as usize] = true;
                }
            }
        }
    }
    drop
}

/// Drops the faces of the pinched vertices ([`pinched`]) and keeps the
/// largest piece (pieces joined through edges, as the crown is cut out),
/// until the faces are one piece with one fan at every vertex.
fn tidy(t: &mut Vec<[u32; 3]>) {
    loop {
        let drop = pinched(t, &Edges::of(t));
        let piece = geom::largest_face_component(t);
        if drop.iter().zip(&piece).all(|(d, p)| !d && *p) {
            return;
        }
        let mut k = 0;
        t.retain(|_| {
            let keep = !drop[k] && piece[k];
            k += 1;
            keep
        });
    }
}

/// Topology of a connected surface whose edges have at most two faces and
/// whose vertices have one fan each ([`tidy`]).
struct Topology {
    /// Twice its genus, `2 − χ − b` with `b` its holes: the handles.
    twice_genus: i64,
    /// The hole of every edge of one face (`NONE` for the others).
    hole: Vec<u32>,
    /// Number of holes.
    holes: usize,
}

fn topology(t: &[[u32; 3]], e: &Edges, nv: usize) -> Topology {
    let mut used = vec![false; nv];
    for &i in t.iter().flatten() {
        used[i as usize] = true;
    }
    // the border edges of a hole are joined through their ends
    let mut parent: Vec<u32> = (0..nv as u32).collect();
    for (i, [a, b]) in e.ends.iter().enumerate() {
        if e.count[i] == 1 {
            union(&mut parent, *a, *b);
        }
    }
    let mut id = vec![NONE; nv];
    let mut holes = 0u32;
    let mut hole = vec![NONE; e.ends.len()];
    for (i, h) in hole.iter_mut().enumerate() {
        if e.count[i] == 1 {
            let r = find(&mut parent, e.ends[i][0]) as usize;
            if id[r] == NONE {
                id[r] = holes;
                holes += 1;
            }
            *h = id[r];
        }
    }
    let chi = used.iter().filter(|u| **u).count() as i64 - e.ends.len() as i64 + t.len() as i64;
    Topology {
        twice_genus: 2 - chi - i64::from(holes),
        hole,
        holes: holes as usize,
    }
}

/// A distance from the base point and its vertex, the nearest first in a
/// [`BinaryHeap`] (then the lowest vertex).
#[derive(PartialEq)]
struct Near(f64, u32);

impl Eq for Near {}

impl Ord for Near {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

impl PartialOrd for Near {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Shortest paths from a base point ([`Graph::paths`]).
struct Paths {
    dist: Vec<f64>,
    /// The edge to the parent of every vertex (`NONE` for the base and the
    /// vertices not reached).
    up: Vec<u32>,
    /// Edges from the base.
    depth: Vec<u32>,
}

/// A surface as a graph for its loops.
struct Graph<'a> {
    e: &'a Edges,
    topo: &'a Topology,
    faces: usize,
    /// Length of every edge.
    len: Vec<f64>,
    /// The edges at vertex `x`: `at[start[x]..start[x + 1]]`.
    start: Vec<u32>,
    at: Vec<u32>,
}

impl<'a> Graph<'a> {
    fn new(v: &[DVec3], faces: usize, e: &'a Edges, topo: &'a Topology) -> Self {
        let len = e
            .ends
            .iter()
            .map(|[a, b]| (v[*a as usize] - v[*b as usize]).length())
            .collect();
        let mut start = vec![0u32; v.len() + 1];
        for [a, b] in &e.ends {
            start[*a as usize + 1] += 1;
            start[*b as usize + 1] += 1;
        }
        for x in 0..v.len() {
            start[x + 1] += start[x];
        }
        let mut fill = start.clone();
        let mut at = vec![0u32; 2 * e.ends.len()];
        for (i, [a, b]) in e.ends.iter().enumerate() {
            for x in [a, b] {
                at[fill[*x as usize] as usize] = i as u32;
                fill[*x as usize] += 1;
            }
        }
        Self {
            e,
            topo,
            faces,
            len,
            start,
            at,
        }
    }

    /// The edges at vertex `x`.
    fn edges_at(&self, x: u32) -> &[u32] {
        &self.at[self.start[x as usize] as usize..self.start[x as usize + 1] as usize]
    }

    /// The two nodes of the faces and holes across edge `i`: its faces, or
    /// its face and its hole (numbered after the faces).
    fn sides(&self, i: u32) -> (u32, u32) {
        let [f, g] = self.e.faces[i as usize];
        if g == NONE {
            (f, self.faces as u32 + self.topo.hole[i as usize])
        } else {
            (f, g)
        }
    }

    /// The shortest paths from `base` to every vertex (Dijkstra, the lowest
    /// vertex first on a tie).
    fn paths(&self, base: u32) -> Paths {
        let n = self.start.len() - 1;
        let mut p = Paths {
            dist: vec![f64::INFINITY; n],
            up: vec![NONE; n],
            depth: vec![0; n],
        };
        let mut done = vec![false; n];
        let mut heap = BinaryHeap::new();
        p.dist[base as usize] = 0.0;
        heap.push(Near(0.0, base));
        while let Some(Near(d, x)) = heap.pop() {
            if std::mem::replace(&mut done[x as usize], true) {
                continue;
            }
            if p.up[x as usize] != NONE {
                p.depth[x as usize] = p.depth[self.e.other(p.up[x as usize], x) as usize] + 1;
            }
            for &i in self.edges_at(x) {
                let y = self.e.other(i, x);
                let nd = d + self.len[i as usize];
                if nd < p.dist[y as usize] {
                    p.dist[y as usize] = nd;
                    p.up[y as usize] = i;
                    heap.push(Near(nd, y));
                }
            }
        }
        p
    }

    /// The loop of edge `i` off the tree of `p`: up the tree from both ends
    /// to where their paths meet; `(length, vertices)`.
    fn cycle(&self, i: u32, p: &Paths) -> (f64, Vec<u32>) {
        let parent = |x: u32| self.e.other(p.up[x as usize], x);
        let depth = |x: u32| p.depth[x as usize];
        let [a, b] = self.e.ends[i as usize];
        let (mut x, mut y) = (a, b);
        let (mut px, mut py) = (vec![a], vec![b]);
        while depth(x) > depth(y) {
            x = parent(x);
            px.push(x);
        }
        while depth(y) > depth(x) {
            y = parent(y);
            py.push(y);
        }
        while x != y {
            x = parent(x);
            px.push(x);
            y = parent(y);
            py.push(y);
        }
        py.pop();
        px.extend(py.into_iter().rev());
        let d = |x: u32| p.dist[x as usize];
        (d(a) + self.len[i as usize] + d(b) - 2.0 * d(x), px)
    }

    /// The loops of the greedy system from `base` that do not split the
    /// surface, `(length, vertices)`, added to `out`.
    fn greedy(&self, base: u32, out: &mut Vec<(f64, Vec<u32>)>) {
        let p = self.paths(base);
        let through = |i: u32| {
            let [a, b] = self.e.ends[i as usize];
            p.dist[a as usize] + self.len[i as usize] + p.dist[b as usize]
        };
        let mut tree = vec![false; self.e.ends.len()];
        for &i in p.up.iter().filter(|&&i| i != NONE) {
            tree[i as usize] = true;
        }
        let mut off: Vec<u32> = (0..self.e.ends.len() as u32)
            .filter(|&i| !tree[i as usize] && through(i).is_finite())
            .collect();
        off.sort_by(|&i, &j| through(j).total_cmp(&through(i)).then(i.cmp(&j)));
        // the faces and the holes joined across the edges off the tree, the
        // longest loops first; an edge that joins nothing new closes a loop
        let mut parent: Vec<u32> = (0..(self.faces + self.topo.holes) as u32).collect();
        for i in off {
            let (f, g) = self.sides(i);
            if !union(&mut parent, f, g) {
                out.push(self.cycle(i, &p));
            }
        }
    }

    /// The class of every edge (bits over Z/2, see the module
    /// documentation): a loop does not split the surface when the classes of
    /// its edges do not add up to zero. `None` past 32 handles (64 bits).
    fn classes(&self, root: u32) -> Option<Vec<u64>> {
        let ne = self.e.ends.len();
        // a spanning tree of the vertices, breadth first
        let mut tree = vec![false; ne];
        let mut seen = vec![false; self.start.len() - 1];
        seen[root as usize] = true;
        let mut queue = VecDeque::from([root]);
        while let Some(x) = queue.pop_front() {
            for &i in self.edges_at(x) {
                let y = self.e.other(i, x);
                if !std::mem::replace(&mut seen[y as usize], true) {
                    tree[i as usize] = true;
                    queue.push_back(y);
                }
            }
        }
        // a spanning tree of the faces and holes across the other edges; a
        // bit for every edge left out of both
        let nodes = self.faces + self.topo.holes;
        let mut parent: Vec<u32> = (0..nodes as u32).collect();
        let mut dual = vec![false; ne];
        let mut class = vec![0u64; ne];
        let mut bits = 0;
        for i in (0..ne as u32).filter(|&i| !tree[i as usize]) {
            let (f, g) = self.sides(i);
            if union(&mut parent, f, g) {
                dual[i as usize] = true;
            } else {
                if bits == 64 {
                    return None;
                }
                class[i as usize] = 1 << bits;
                bits += 1;
            }
        }
        // every edge of the second tree: the bits of the edges left out
        // that cross the border of the part of the tree below it
        let mut first = vec![0u32; nodes + 1];
        for i in (0..ne).filter(|&i| dual[i]) {
            let (f, g) = self.sides(i as u32);
            first[f as usize + 1] += 1;
            first[g as usize + 1] += 1;
        }
        for k in 0..nodes {
            first[k + 1] += first[k];
        }
        let mut fill = first.clone();
        let mut next = vec![0u32; first[nodes] as usize];
        for i in (0..ne as u32).filter(|&i| dual[i as usize]) {
            let (f, g) = self.sides(i);
            for k in [f, g] {
                next[fill[k as usize] as usize] = i;
                fill[k as usize] += 1;
            }
        }
        let mut up = vec![NONE; nodes];
        let mut order = vec![0u32];
        let mut reached = vec![false; nodes];
        reached[0] = true;
        let mut k = 0;
        while k < order.len() {
            let n = order[k];
            k += 1;
            for &i in &next[first[n as usize] as usize..first[n as usize + 1] as usize] {
                let (f, g) = self.sides(i);
                let m = if f == n { g } else { f };
                if !std::mem::replace(&mut reached[m as usize], true) {
                    up[m as usize] = i;
                    order.push(m);
                }
            }
        }
        let mut sum = vec![0u64; nodes];
        for i in (0..ne as u32).filter(|&i| class[i as usize] != 0) {
            let (f, g) = self.sides(i);
            sum[f as usize] ^= class[i as usize];
            sum[g as usize] ^= class[i as usize];
        }
        for &n in order.iter().skip(1).rev() {
            let i = up[n as usize];
            let (f, g) = self.sides(i);
            let m = if f == n { g } else { f };
            class[i as usize] = sum[n as usize];
            sum[m as usize] ^= sum[n as usize];
        }
        Some(class)
    }

    /// The shortest loop whose edge classes do not add up to zero:
    /// `(length, base, edge)`, the loop being the edge and the shortest paths
    /// from the base to its ends. Every vertex of `bases` is a base in turn
    /// (the first on a tie); the paths go no farther than half the shortest
    /// loop found so far, `bound` at the start.
    fn shortest(&self, class: &[u64], bases: &[u32], mut bound: f64) -> Option<(f64, u32, u32)> {
        let n = self.start.len() - 1;
        let (mut dist, mut sum, mut done) = (vec![f64::INFINITY; n], vec![0u64; n], vec![false; n]);
        let mut touched: Vec<u32> = Vec::new();
        let mut heap = BinaryHeap::new();
        let mut best = None;
        for &b in bases {
            for &x in &touched {
                dist[x as usize] = f64::INFINITY;
                done[x as usize] = false;
            }
            touched.clear();
            heap.clear();
            dist[b as usize] = 0.0;
            sum[b as usize] = 0;
            touched.push(b);
            heap.push(Near(0.0, b));
            while let Some(Near(d, x)) = heap.pop() {
                if 2.0 * d >= bound {
                    break;
                }
                if std::mem::replace(&mut done[x as usize], true) {
                    continue;
                }
                for &i in self.edges_at(x) {
                    let y = self.e.other(i, x);
                    let l = d + self.len[i as usize];
                    if done[y as usize] {
                        let total = l + dist[y as usize];
                        let c = sum[x as usize] ^ class[i as usize] ^ sum[y as usize];
                        if c != 0 && total < bound {
                            bound = total;
                            best = Some((total, b, i));
                        }
                    } else if l < dist[y as usize] {
                        if dist[y as usize].is_infinite() {
                            touched.push(y);
                        }
                        dist[y as usize] = l;
                        sum[y as usize] = sum[x as usize] ^ class[i as usize];
                        heap.push(Near(l, y));
                    }
                }
            }
        }
        best
    }
}

/// `k` of the vertices `used` spread over them: the first one, then each
/// time the one farthest from those taken (the first on a tie).
fn spread(v: &[DVec3], used: &[u32], k: usize) -> Vec<u32> {
    let mut out = vec![used[0]];
    let mut d: Vec<f64> = used
        .iter()
        .map(|&i| (v[i as usize] - v[used[0] as usize]).length_squared())
        .collect();
    while out.len() < k.min(used.len()) {
        let j = (0..d.len())
            .max_by(|&a, &b| d[a].total_cmp(&d[b]).then(b.cmp(&a)))
            .unwrap_or(0);
        let p = v[used[j] as usize];
        out.push(used[j]);
        for (x, &i) in d.iter_mut().zip(used) {
            *x = x.min((v[i as usize] - p).length_squared());
        }
    }
    out
}

/// Loops of edges of the faces `t` that do not split them: the shortest of
/// all, then the greedy ones shortest first (then by their vertices), each
/// set of vertices once, at most [`TRIES`].
fn shortest_loops(v: &[DVec3], t: &[[u32; 3]], e: &Edges, topo: &Topology) -> Vec<Vec<u32>> {
    let g = Graph::new(v, t.len(), e, topo);
    let mut used: Vec<u32> = t.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let mut all = Vec::new();
    for b in spread(v, &used, BASES) {
        g.greedy(b, &mut all);
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let bound = all.first().map_or(f64::INFINITY, |l| l.0);
    let shortest = g
        .classes(used[0])
        .and_then(|class| g.shortest(&class, &used, bound))
        .map(|(_, b, i)| g.cycle(i, &g.paths(b)));
    let mut seen: Vec<Vec<u32>> = Vec::new();
    let mut out = Vec::new();
    for (_, lp) in shortest.into_iter().chain(all) {
        let mut key = lp.clone();
        key.sort_unstable();
        if !seen.contains(&key) {
            seen.push(key);
            out.push(lp);
            if out.len() == TRIES {
                break;
            }
        }
    }
    out
}

fn area(v: &[DVec3], t: &[[u32; 3]]) -> f64 {
    t.iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| v[i as usize]);
            0.5 * (b - a).cross(c - a).length()
        })
        .sum()
}

/// The open crown `v`, `t` (one piece) with its handles cut away and the
/// holes the cuts leave closed (see the module documentation); a vertex
/// where two parts of the crown touch is parted as well. `None` when there
/// is nothing to cut, and when the crown is not a surface with at most two
/// faces on every edge or its handles cannot be cut within
/// [`MAX_CUT_SHARE`] of its surface: then the crown is closed as it is, and
/// the closed tooth is refused by its check.
pub(super) fn cut(v: &[DVec3], t: &[[u32; 3]]) -> Option<Cut> {
    let e = Edges::of(t);
    let degenerate = t
        .iter()
        .any(|f| f[0] == f[1] || f[1] == f[2] || f[0] == f[2]);
    if t.is_empty() || degenerate || e.count.iter().any(|&c| c > 2) {
        return None;
    }
    if !pinched(t, &e).contains(&true) && topology(t, &e, v.len()).twice_genus == 0 {
        return None;
    }
    // the border of the scan, as its faces run it: closed with the tooth
    let mut border: Vec<(u32, u32)> = Vec::new();
    for (i, &[a, b]) in e.ends.iter().enumerate() {
        if e.count[i] == 1 {
            let f = t[e.faces[i][0] as usize];
            let k = f.iter().position(|&x| x == a).unwrap_or(0);
            border.push(if f[(k + 1) % 3] == b { (a, b) } else { (b, a) });
        }
    }
    border.sort_unstable();
    let floor = (1.0 - MAX_CUT_SHARE) * area(v, t);
    let mut cur = t.to_vec();
    tidy(&mut cur);
    if cur.is_empty() {
        return None;
    }
    let mut handles = None;
    loop {
        let e = Edges::of(&cur);
        let topo = topology(&cur, &e, v.len());
        handles.get_or_insert(topo.twice_genus.max(0) as usize / 2);
        if topo.twice_genus <= 0 {
            break;
        }
        let mut cut = None;
        for lp in shortest_loops(v, &cur, &e, &topo) {
            let mut on = vec![false; v.len()];
            for &i in &lp {
                on[i as usize] = true;
            }
            let mut next: Vec<[u32; 3]> = cur
                .iter()
                .filter(|f| !f.iter().any(|&i| on[i as usize]))
                .copied()
                .collect();
            tidy(&mut next);
            if next.is_empty() || area(v, &next) < floor {
                continue;
            }
            if topology(&next, &Edges::of(&next), v.len()).twice_genus < topo.twice_genus {
                cut = Some(next);
                break;
            }
        }
        cur = cut?;
    }
    // the faces kept, then a fan over every hole of a cut
    let mut kept: Vec<u32> = cur.iter().flatten().copied().collect();
    kept.sort_unstable();
    kept.dedup();
    let mut at = vec![NONE; v.len()];
    for (k, &i) in kept.iter().enumerate() {
        at[i as usize] = k as u32;
    }
    let mut vertices: Vec<DVec3> = kept.iter().map(|&i| v[i as usize]).collect();
    let mut tris: Vec<[u32; 3]> = cur.iter().map(|f| f.map(|i| at[i as usize])).collect();
    let mut caps = Vec::new();
    for lp in geom::boundary_loops(&cur) {
        let m = lp.len();
        if (0..m).any(|k| border.binary_search(&(lp[k], lp[(k + 1) % m])).is_ok()) {
            continue;
        }
        let c = vertices.len() as u32;
        let ring: Vec<DVec3> = lp.iter().map(|&i| v[i as usize]).collect();
        vertices.push(geom::mean(&ring));
        for k in 0..m {
            tris.push([at[lp[(k + 1) % m] as usize], at[lp[k] as usize], c]);
        }
        caps.push(lp);
    }
    let e = Edges::of(&tris);
    let surface = e.count.iter().all(|&c| c <= 2) && !pinched(&tris, &e).contains(&true);
    (surface && topology(&tris, &e, vertices.len()).twice_genus == 0).then(|| Cut {
        vertices,
        tris,
        kept,
        caps,
        handles: handles.unwrap_or(0),
    })
}

/// A torus around `+Z` (radius `big`, and `small(u)` around its tube at
/// angle `u` around `+Z`), `nu` × `nw` quads, each split in two triangles,
/// faces outward.
#[cfg(test)]
pub(super) fn torus(
    big: f64,
    small: impl Fn(f64) -> f64,
    nu: u32,
    nw: u32,
) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    use std::f64::consts::TAU;
    let mut v = Vec::new();
    for i in 0..nu {
        let u = TAU * f64::from(i) / f64::from(nu);
        let s = small(u);
        for j in 0..nw {
            let w = TAU * f64::from(j) / f64::from(nw);
            let r = big + s * w.cos();
            v.push(DVec3::new(r * u.cos(), r * u.sin(), s * w.sin()));
        }
    }
    let id = |i: u32, j: u32| (i % nu) * nw + j % nw;
    let mut t = Vec::new();
    for i in 0..nu {
        for j in 0..nw {
            let (a, b, c, d) = (id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1));
            t.extend([[a, b, c], [a, c, d]]);
        }
    }
    (v, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square of `n` × `n` quads in the plane `z = 0`, side 1.
    fn disk(n: u32) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let mut v = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                v.push(DVec3::new(f64::from(i), f64::from(j), 0.0) / f64::from(n));
            }
        }
        let id = |i: u32, j: u32| j * (n + 1) + i;
        let mut t = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (a, b, c, d) = (id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1));
                t.extend([[a, b, c], [a, c, d]]);
            }
        }
        (v, t)
    }

    /// The outside of the unit cubes `solid` among `x, y, z` in `-1..8`,
    /// every square face split in `k` × `k` quads of two triangles, faces
    /// outward.
    fn cubes(solid: impl Fn([i32; 3]) -> bool, k: i32) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let mut v: Vec<DVec3> = Vec::new();
        let mut at = std::collections::BTreeMap::new();
        let mut t = Vec::new();
        let cells =
            (-1..8).flat_map(|x| (-1..8).flat_map(move |y| (-1..8).map(move |z| [x, y, z])));
        for cell in cells.filter(|c| solid(*c)) {
            for (d, s) in [(0, 1), (0, -1), (1, 1), (1, -1), (2, 1), (2, -1)] {
                let mut n = cell;
                n[d] += s;
                if solid(n) {
                    continue;
                }
                // the face to the empty neighbour, around the axis d
                let (a, b) = ((d + 1) % 3, (d + 2) % 3);
                let mut id = |da: i32, db: i32| {
                    let mut p = cell.map(|x| x * k);
                    p[d] += k * i32::from(s > 0);
                    p[a] += da;
                    p[b] += db;
                    *at.entry(p).or_insert_with(|| {
                        v.push(DVec3::new(p[0].into(), p[1].into(), p[2].into()) / f64::from(k));
                        v.len() as u32 - 1
                    })
                };
                for i in 0..k {
                    for j in 0..k {
                        let q = [id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)];
                        if s > 0 {
                            t.extend([[q[0], q[1], q[2]], [q[0], q[2], q[3]]]);
                        } else {
                            t.extend([[q[0], q[2], q[1]], [q[0], q[3], q[2]]]);
                        }
                    }
                }
            }
        }
        (v, t)
    }

    /// Twice the genus, the holes and whether a vertex is pinched.
    fn shape(v: &[DVec3], t: &[[u32; 3]]) -> (i64, usize, bool) {
        let e = Edges::of(t);
        let topo = topology(t, &e, v.len());
        (topo.twice_genus, topo.holes, pinched(t, &e).contains(&true))
    }

    #[test]
    fn a_crown_without_handles_is_left_as_it_is() {
        let (v, t) = disk(6);
        assert_eq!(shape(&v, &t), (0, 1, false));
        assert_eq!(cut(&v, &t), None);
        // a closed surface without a handle: a square with its back
        let (v, mut t) = disk(1);
        t.extend([[1, 0, 2], [1, 2, 3]]);
        assert_eq!(shape(&v, &t), (0, 0, false));
        assert_eq!(cut(&v, &t), None);
    }

    /// A torus with a hole in it (a crown with one handle): the handle is
    /// cut across its tube where the tube is narrowest, the two ends are
    /// closed, and the hole of the scan stays open as it was.
    #[test]
    fn a_handle_is_cut_where_it_is_narrowest_and_its_holes_closed() {
        let pi = std::f64::consts::PI;
        let narrow = |u: f64| 1.0 - 0.5 * (-((u - pi) / 0.3).powi(2)).exp();
        let (v, mut t) = torus(10.0, narrow, 48, 12);
        let hole: Vec<[u32; 3]> = t.drain(..2).collect();
        assert_eq!(shape(&v, &t), (2, 1, false));
        let c = cut(&v, &t).expect("a handle to cut");
        assert_eq!(c.handles, 1);
        assert_eq!(shape(&c.vertices, &c.tris), (0, 1, false));
        // the open border left is the one of the scan, run the same way
        let border: Vec<Vec<u32>> = geom::boundary_loops(&c.tris)
            .into_iter()
            .map(|l| l.into_iter().map(|i| c.kept[i as usize]).collect())
            .collect();
        assert_eq!(border.len(), 1);
        let [a, b, _] = hole[0];
        let lp = &border[0];
        let k = lp
            .iter()
            .position(|&x| x == b)
            .expect("the hole of the scan");
        assert_eq!(lp[(k + 1) % lp.len()], a, "{lp:?}");
        // the narrowest ring (24) goes with its faces; the rings either side
        // are the ends, each closed by a fan
        let ring = |i: u32| (0..12).map(|j| i * 12 + j).collect::<Vec<u32>>();
        assert!(ring(24).iter().all(|i| c.kept.binary_search(i).is_err()));
        let mut ends: Vec<Vec<u32>> = c
            .caps
            .iter()
            .map(|l| {
                let mut l = l.clone();
                l.sort_unstable();
                l
            })
            .collect();
        ends.sort();
        assert_eq!(ends, [ring(23), ring(25)]);
        assert_eq!(t.len() - (c.tris.len() - 24), 4 * 12);
        // every kept vertex is where it was, every centre amid its hole
        for (k, &i) in c.kept.iter().enumerate() {
            assert_eq!(c.vertices[k], v[i as usize]);
        }
        for (k, l) in c.caps.iter().enumerate() {
            let ring: Vec<DVec3> = l.iter().map(|&i| v[i as usize]).collect();
            assert_eq!(c.vertices[c.kept.len() + k], geom::mean(&ring));
        }
        // the same input gives the same cut
        assert_eq!(cut(&v, &t), Some(c));
    }

    /// A plate with three tunnels through it and an opening on its side
    /// (a crown with three handles): every handle is cut.
    #[test]
    fn every_handle_is_cut() {
        let plate = |[x, y, z]: [i32; 3]| {
            (0..7).contains(&x) && (0..3).contains(&y) && z == 0 && !(y == 1 && x % 2 == 1)
        };
        let (v, mut t) = cubes(plate, 6);
        let side = t
            .iter()
            .position(|f| f.iter().all(|&i| v[i as usize].x == 0.0))
            .unwrap();
        t.drain(side..side + 2);
        assert_eq!(shape(&v, &t), (6, 1, false));
        let c = cut(&v, &t).expect("handles to cut");
        assert_eq!(c.handles, 3);
        assert_eq!(shape(&c.vertices, &c.tris), (0, 1, false));
        assert_eq!(geom::boundary_loops(&c.tris).len(), 1);
        assert_eq!(c.caps.len(), 6);
    }

    #[test]
    fn two_parts_touching_at_a_point_are_parted() {
        // a square and a triangle touching it at its corner 0
        let (mut v, mut t) = disk(3);
        let n = v.len() as u32;
        v.extend([DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0)]);
        t.push([0, n + 1, n]);
        assert!(shape(&v, &t).2);
        let c = cut(&v, &t).expect("a pinch to part");
        assert_eq!(c.handles, 0);
        assert_eq!(c.tris, t[..18]);
        assert_eq!(shape(&c.vertices, &c.tris), (0, 1, false));
    }

    #[test]
    fn what_is_not_a_surface_is_left_alone() {
        // an edge of three faces
        let (mut v, mut t) = torus(10.0, |_| 1.0, 12, 6);
        v.push(DVec3::new(0.0, 0.0, 30.0));
        let [a, b, _] = t[0];
        t.push([a, b, v.len() as u32 - 1]);
        assert_eq!(cut(&v, &t), None);
        // a face with a corner twice
        let (v, mut t) = torus(10.0, |_| 1.0, 12, 6);
        t[0][1] = t[0][0];
        assert_eq!(cut(&v, &t), None);
        assert_eq!(cut(&v, &[]), None);
    }
}
