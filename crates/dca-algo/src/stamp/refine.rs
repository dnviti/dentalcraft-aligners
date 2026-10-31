//! Local refinement of a triangle mesh by longest-edge bisection (Rivara):
//! the triangles of a zone are split until their edges are short, and the
//! neighbours are split along as far as needed to keep the mesh conforming.
//!
//! Splitting an edge puts a vertex on it and cuts the two triangles that
//! share it in two, so the surface does not move and the topology stays that
//! of the input: a closed manifold mesh stays closed and manifold, an open
//! one keeps its boundary. Vertex normals and labels of the new vertices are
//! those of the edge ends.

use std::collections::HashMap;

use dc_mesh::TriMesh;
use glam::DVec3;

const NONE: u32 = u32::MAX;
/// Splits allowed in one call (a guard against runaway refinement).
const MAX_SPLITS: usize = 4_000_000;
/// Longest chain of neighbours followed to find the edge to split.
const MAX_CHAIN: usize = 4096;

#[derive(Clone, Copy)]
struct Edge {
    faces: [u32; 2],
    count: u8,
}

/// A mesh being refined.
pub(super) struct Refiner {
    pub positions: Vec<DVec3>,
    pub normals: Vec<DVec3>,
    pub labels: Option<Vec<u16>>,
    pub tris: Vec<[u32; 3]>,
    active: Vec<bool>,
    edges: HashMap<(u32, u32), Edge>,
    queue: Vec<u32>,
    splits: usize,
}

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

impl Refiner {
    /// The mesh with its edge table and area-weighted vertex normals.
    pub fn new(mesh: &TriMesh) -> Self {
        let mut r = Self {
            positions: mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
            normals: mesh
                .compute_vertex_normals()
                .iter()
                .map(|n| n.as_dvec3())
                .collect(),
            labels: mesh.vertex_labels.clone(),
            tris: mesh.tris.clone(),
            active: vec![false; mesh.tris.len()],
            edges: HashMap::with_capacity(mesh.tris.len() * 3 / 2),
            queue: Vec::new(),
            splits: 0,
        };
        for f in 0..r.tris.len() {
            let t = r.tris[f];
            for k in 0..3 {
                r.edge_add(t[k], t[(k + 1) % 3], f as u32);
            }
        }
        r
    }

    /// Marks a triangle as one to refine.
    pub fn activate(&mut self, f: u32) {
        if !self.active[f as usize] {
            self.active[f as usize] = true;
            self.queue.push(f);
        }
    }

    fn edge_add(&mut self, a: u32, b: u32, f: u32) {
        let e = self.edges.entry(key(a, b)).or_insert(Edge {
            faces: [NONE; 2],
            count: 0,
        });
        if e.count < 2 {
            e.faces[e.count as usize] = f;
        }
        e.count += 1;
    }

    fn edge_replace(&mut self, a: u32, b: u32, old: u32, new: u32) {
        if let Some(e) = self.edges.get_mut(&key(a, b)) {
            for slot in &mut e.faces {
                if *slot == old {
                    *slot = new;
                    return;
                }
            }
        }
    }

    fn len2(&self, a: u32, b: u32) -> f64 {
        self.positions[a as usize].distance_squared(self.positions[b as usize])
    }

    /// The longest edge of a triangle: its index (the edge from vertex `i`
    /// to `i + 1`) and squared length. Ties are broken by the vertex ids, so
    /// that two triangles agree on whether a shared edge is their longest.
    fn longest(&self, f: u32) -> (usize, f64) {
        let t = self.tris[f as usize];
        let mut best = (0usize, f64::NEG_INFINITY, (0u32, 0u32));
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let l = self.len2(a, b);
            let k = key(a, b);
            if l > best.1 || (l == best.1 && k > best.2) {
                best = (i, l, k);
            }
        }
        (best.0, best.1)
    }

    fn longest_key(&self, f: u32) -> (u32, u32) {
        let (i, _) = self.longest(f);
        let t = self.tris[f as usize];
        key(t[i], t[(i + 1) % 3])
    }

    /// Splits the edge `(a, b)` in the middle, in the triangles that share it.
    fn split(&mut self, a: u32, b: u32) {
        let k = key(a, b);
        let Some(edge) = self.edges.remove(&k) else {
            return;
        };
        let m = self.positions.len() as u32;
        self.positions
            .push(0.5 * (self.positions[a as usize] + self.positions[b as usize]));
        let n = self.normals[a as usize] + self.normals[b as usize];
        self.normals.push(if n.length_squared() > 1e-12 {
            n.normalize()
        } else {
            self.normals[a as usize]
        });
        if let Some(labels) = &mut self.labels {
            labels.push(labels[a as usize]);
        }
        for slot in 0..edge.count.min(2) as usize {
            let f = edge.faces[slot];
            let t = self.tris[f as usize];
            let i = (0..3)
                .find(|&i| key(t[i], t[(i + 1) % 3]) == k)
                .expect("the face has the edge");
            let (e0, e1, e2) = (t[i], t[(i + 1) % 3], t[(i + 2) % 3]);
            let nf = self.tris.len() as u32;
            self.tris[f as usize] = [e0, m, e2];
            self.tris.push([m, e1, e2]);
            let act = self.active[f as usize];
            self.active.push(act);
            self.edge_replace(e1, e2, f, nf);
            self.edge_add(e0, m, f);
            self.edge_add(m, e1, nf);
            self.edge_add(m, e2, f);
            self.edge_add(m, e2, nf);
            if act {
                self.queue.push(f);
                self.queue.push(nf);
            }
        }
        self.splits += 1;
    }

    /// Splits the longest edge of `f` after making the neighbours across it
    /// split theirs first when it is not their longest (Rivara). `false`
    /// when a non-manifold edge or a runaway chain stops it.
    fn refine_face(&mut self, f: u32) -> bool {
        let mut stack = vec![f];
        while let Some(&cur) = stack.last() {
            if stack.len() > MAX_CHAIN {
                return false;
            }
            let (i, _) = self.longest(cur);
            let t = self.tris[cur as usize];
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let Some(&edge) = self.edges.get(&key(a, b)) else {
                return false;
            };
            match edge.count {
                1 => {
                    self.split(a, b);
                    stack.pop();
                }
                2 => {
                    let g = if edge.faces[0] == cur {
                        edge.faces[1]
                    } else {
                        edge.faces[0]
                    };
                    if self.longest_key(g) == key(a, b) {
                        self.split(a, b);
                        stack.pop();
                    } else {
                        stack.push(g);
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// Splits the active triangles whose longest edge is longer than `h` and
    /// that `in_zone` accepts, until there are none. A triangle that falls
    /// out of the zone, or cannot be split, stops being active.
    pub fn refine(&mut self, h: f64, in_zone: &dyn Fn(&[DVec3; 3]) -> bool) {
        let h2 = h * h;
        while let Some(f) = self.queue.pop() {
            if !self.active[f as usize] {
                continue;
            }
            if self.longest(f).1 <= h2 {
                continue;
            }
            let t = self.tris[f as usize].map(|i| self.positions[i as usize]);
            if !in_zone(&t) {
                self.active[f as usize] = false;
                continue;
            }
            if self.splits >= MAX_SPLITS || !self.refine_face(f) {
                self.active[f as usize] = false;
                continue;
            }
            self.queue.push(f);
        }
    }

    /// The refined (and possibly displaced) mesh.
    pub fn into_mesh(self) -> TriMesh {
        let mut mesh = TriMesh::new(
            self.positions.iter().map(|p| p.as_vec3()).collect(),
            self.tris,
        );
        mesh.vertex_labels = self.labels;
        mesh
    }
}
