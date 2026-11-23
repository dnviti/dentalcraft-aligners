//! Closing an open scan with a flat base: the plugin's `aligner.make_base`
//! (operator `oa.make_base`, "Chiudi con base"; `case_core.make_base` gives
//! it the occlusal direction of the arch frame).
//!
//! An intraoral scan is open under the gum. The plugin, with bmesh:
//!
//! 1. takes every boundary edge (`e.is_boundary`); none: the scan is
//!    already closed and nothing changes (it reports 0 edges);
//! 2. finds the lowest point of the scan along the occlusal direction `z`
//!    and puts the base plane `height` mm below it;
//! 3. extrudes the boundary edges (`extrude_edge_only`: one new vertex per
//!    boundary vertex, a quad between each boundary edge and its copy) and
//!    moves every new vertex along `z` onto the plane;
//! 4. fills the new boundary loops with one face each (`holes_fill`),
//!    triangulates them and makes the normals consistent and outward
//!    (`recalc_face_normals`).
//!
//! The same here, on a [`TriMesh`]: the vertices and triangles of the scan
//! keep their index and the new ones follow, so the scan as it was read is
//! the start of the closed solid ([`Based::scan`], [`open_part`]). The
//! result must be a closed, manifold solid whatever the scan gives (the
//! view, the bite, the trim line and the export read it, and the print
//! check counts its edges), so a few deviations from the plugin, which
//! extrudes every boundary loop as it is:
//!
//! * Repair: an edge shared by more than two triangles (the scan folds back
//!   on itself, or a patch of the scanner lies over its surface) keeps the
//!   two triangles that run it opposite ways and turn least from each
//!   other; the others are taken out of the scan ([`Based::removed_faces`]:
//!   the smallest change that leaves the edge between two triangles, no
//!   point moves) and the hole they leave is filled like any other.
//! * Cycles: the open edges are followed into closed cycles, each round one
//!   gap of the surface: at a point where the boundary passes more than
//!   once (two holes touching, a hole touching the border) each incoming
//!   edge goes on with the first outgoing one around the point, and a
//!   cycle through a point twice is split there.
//! * Border: the cycle of a piece that encloses the most of it seen along
//!   the occlusal direction, at least [`BORDER_SHARE`] of its footprint
//!   (the convex hull of its points); other cycles of the piece that go
//!   round the same way and are as large, sharing no point with it, are
//!   borders too. Only the borders are walled down to the base. A piece
//!   with no such cycle (a closed model with a few holes) gets no base.
//! * Holes: every other cycle, whatever its size or shape, is filled in
//!   place: a smooth patch that follows the surrounding surface
//!   ([`dc_geom::holefill`]); a hole touching another cycle at a point, or
//!   one that the patching cannot take, gets the triangulation of its
//!   points of least area (a fan from its middle point when it is long).
//!   Never extruded into a chimney from the hole down to the base.
//! * Check: the result is counted as a solid ([`dc_mesh::solid_topology`],
//!   points identified by position as in an STL file); the open and
//!   non-manifold edges left are in [`Based`], for the caller to tell.
//!
//! The flat caps are triangulated by ear clipping in the base plane
//! (Blender's "beauty" fill gives other triangles over the same flat area).
//! The new faces follow the winding of the scan; when the closed result has
//! a negative volume every face is turned, the outward orientation
//! `recalc_face_normals` gives.
//!
//! Two more deviations from bmesh, which copies every attribute of the
//! source vertex: a point of the base or of a patch gets the label `0`
//! (gingiva), never a tooth, so no tooth reaches into the base (its colour
//! still follows its source vertex); and the points of the scan keep their
//! normals (turned with the faces), so that the scan as it was read comes
//! back exactly (without the triangles the repair took out).
//!
//! The import closes every open arch scan before anything is computed on it
//! (the case has no tooth marks yet): its occlusal direction is then the one
//! [`occlusal_direction`] finds without them, where the plugin needs the
//! arch frame of the tooth marks. The study models close the scan of each
//! arch with the same cycles and borders (`openings`, `borders`).

use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

use dc_geom::holefill::{self, HoleFillParams};
use dc_mesh::TriMesh;
use dca_model::{ClosedBy, ScanBase};
use glam::{DVec2, DVec3, Vec3};

use crate::autoseed;

/// Default thickness of the base (mm): the plugin's `base_height`.
pub const HEIGHT_DEFAULT_MM: f64 = 3.0;
/// Smallest base thickness the plugin's slider allows (mm).
pub const HEIGHT_MIN_MM: f64 = 0.5;
/// Largest base thickness the plugin's slider allows (mm).
pub const HEIGHT_MAX_MM: f64 = 20.0;

/// Errors of [`make_base`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BaseError {
    /// The height is outside the range of the plugin's slider.
    #[error("the base height must be between 0.5 and 20 mm (got {0})")]
    Height(f64),
    /// The occlusal direction is zero or not finite.
    #[error("the occlusal direction is not a valid direction")]
    Direction,
    /// The scan has no triangles, or a point that is not finite.
    #[error("the scan is empty or has an invalid point")]
    Mesh,
}

/// A cycle of the boundary is an open border, walled down to the base, when
/// it encloses at least this share of the footprint of its piece seen along
/// the occlusal direction (the convex hull of its points). The border of an
/// arch scan encloses most of it (0.79 and 0.84 of it on the two scans of
/// the test piece), a hole a few thousandths of it at most.
pub const BORDER_SHARE: f64 = 0.1;

/// A hole with more points than this, filled without the patching, gets a
/// fan from its middle point instead of the triangulation of least area
/// (whose cost grows with the cube of the points).
const FAN_FROM: usize = 96;

/// A scan closed by [`make_base`].
#[derive(Debug, Clone, PartialEq)]
pub struct Based {
    /// The closed scan: the vertices of the scan in their order, then the
    /// points of the fillings of its holes, then the vertices of the base;
    /// the triangles of the scan in their order (without those the repair
    /// took out), then the fillings, the walls and the caps.
    pub mesh: TriMesh,
    /// Boundary edges of the scan as it was given (the plugin's return
    /// value): those of the border and of the holes.
    pub closed_edges: usize,
    /// For each point added (the fillings, then the base), in order, the
    /// point of the scan it comes from: a point of the border of its hole
    /// for a filling, the boundary vertex it was extruded from for the base.
    pub sources: Vec<u32>,
    /// Height of the base plane along the occlusal direction (mm).
    pub plane: f64,
    /// Where the scan ends in [`Based::mesh`], for [`open_part`].
    pub scan: ScanBase,
    /// Triangles of the scan the repair took out, on edges of more than two
    /// triangles (see the module documentation): `0` for a clean scan.
    pub removed_faces: usize,
    /// Edges of one triangle left in [`Based::mesh`], counted as a solid
    /// (points identified by position): `0` when it is closed.
    pub open_edges: usize,
    /// Edges of more than two triangles left in [`Based::mesh`], counted as
    /// a solid: `0` when it is manifold.
    pub nonmanifold_edges: usize,
}

impl Based {
    /// [`Based::mesh`] is a clean solid: closed and manifold.
    pub fn is_solid(&self) -> bool {
        self.open_edges == 0 && self.nonmanifold_edges == 0
    }
}

/// Closes `mesh` with a flat base perpendicular to `occlusal` (the
/// occlusal direction of the arch, towards the teeth), `height_mm` below
/// its lowest point, its holes filled in place and its folded triangles
/// repaired (see the module documentation). `Ok(None)` when the scan has no
/// boundary edge and nothing to repair: it is a closed solid already and
/// stays as it is.
pub fn make_base(
    mesh: &TriMesh,
    occlusal: DVec3,
    height_mm: f64,
) -> Result<Option<Based>, BaseError> {
    if !(HEIGHT_MIN_MM..=HEIGHT_MAX_MM).contains(&height_mm) {
        return Err(BaseError::Height(height_mm));
    }
    let z = occlusal.normalize_or_zero();
    if !z.is_finite() || z == DVec3::ZERO {
        return Err(BaseError::Direction);
    }
    let n = mesh.positions.len();
    if mesh.tris.is_empty()
        || mesh.positions.iter().any(|p| !p.is_finite())
        || mesh.tris.iter().flatten().any(|&i| i as usize >= n)
    {
        return Err(BaseError::Mesh);
    }
    let op = openings(&mesh.positions, &mesh.tris, z);
    let removed_faces = op.kept.iter().filter(|k| !**k).count();
    if op.open_edges == 0 && removed_faces == 0 {
        return Ok(None);
    }
    let border = borders(&op);
    let scan_tris: Vec<[u32; 3]> = mesh
        .tris
        .iter()
        .zip(&op.kept)
        .filter(|(_, k)| **k)
        .map(|(t, _)| *t)
        .collect();
    let mut fill = Filling {
        positions: mesh.positions.clone(),
        sources: Vec::new(),
        tris: Vec::new(),
        edges: scan_tris.iter().flat_map(|t| tri_edges(*t)).collect(),
    };
    // a hole that touches another cycle at a point gets the triangulation
    // of least area at once: the patching follows the boundary by itself
    // and would not part the cycles there
    let mut on_cycles: HashMap<u32, u32> = HashMap::new();
    for c in &op.cycles {
        for &w in &c.points {
            *on_cycles.entry(w).or_default() += 1;
        }
    }
    let mut smooth = Vec::new();
    for (k, c) in op.cycles.iter().enumerate() {
        if border[k] {
            continue;
        }
        if c.points.iter().any(|w| on_cycles[w] > 1) {
            fill.least_area(&c.points);
        } else {
            smooth.push(k);
        }
    }
    // the other holes a smooth patch; what it cannot take, the triangulation
    for k in fill.patch(&scan_tris, &op, &border, &smooth)? {
        fill.least_area(&op.cycles[k].points);
    }
    let Filling {
        mut positions,
        mut sources,
        tris: fills,
        ..
    } = fill;

    let height = |p: Vec3| p.as_dvec3().dot(z);
    let lowest = positions
        .iter()
        .map(|p| height(*p))
        .fold(f64::INFINITY, f64::min);
    let plane = lowest - height_mm;

    // one new vertex per border vertex, on the plane
    let rings: Vec<&[u32]> = op
        .cycles
        .iter()
        .zip(&border)
        .filter(|(_, b)| **b)
        .map(|(c, _)| c.points.as_slice())
        .collect();
    let mut copy = vec![u32::MAX; positions.len()];
    for &v in rings.iter().copied().flatten() {
        if copy[v as usize] == u32::MAX {
            copy[v as usize] = positions.len() as u32;
            sources.push(v);
            let p = positions[v as usize].as_dvec3();
            positions.push((p - z * (p.dot(z) - plane)).as_vec3());
        }
    }

    let scan_faces = scan_tris.len();
    let mut tris = scan_tris;
    tris.extend(fills);
    // the walls: the quad between each border edge a -> b and its copy,
    // with the edge the other way round (b -> a) to follow the winding
    for &ring in &rings {
        for (a, b) in ring_edges(ring) {
            let (a2, b2) = (copy[a as usize], copy[b as usize]);
            tris.push([b, a, a2]);
            tris.push([b, a2, b2]);
        }
    }
    // the caps: each border on the plane, the other way round
    let (u, v) = plane_axes(z);
    for &ring in &rings {
        let ring: Vec<u32> = ring.iter().rev().map(|&w| copy[w as usize]).collect();
        let flat: Vec<DVec2> = ring
            .iter()
            .map(|&w| {
                let p = positions[w as usize].as_dvec3();
                DVec2::new(p.dot(u), p.dot(v))
            })
            .collect();
        for [i, j, k] in ear_clip(&flat) {
            tris.push([ring[i], ring[j], ring[k]]);
        }
    }

    let mut out = TriMesh::new(positions, tris);
    // the fillings and the base are gingiva, never a tooth (bmesh would copy
    // the label)
    if let Some(labels) = &mesh.vertex_labels {
        let mut l = labels.clone();
        l.resize(out.positions.len(), 0);
        out.vertex_labels = Some(l);
    }
    if let Some(colors) = &mesh.vertex_colors {
        let mut c = colors.clone();
        c.extend(sources.iter().map(|&s| colors[s as usize]));
        out.vertex_colors = Some(c);
    }
    if let Some(labels) = &mesh.face_labels {
        let mut l: Vec<u16> = labels
            .iter()
            .zip(&op.kept)
            .filter(|(_, k)| **k)
            .map(|(l, _)| *l)
            .collect();
        l.resize(out.tris.len(), 0);
        out.face_labels = Some(l);
    }
    // outward normals
    let flipped = signed_volume(&out) < 0.0;
    if flipped {
        for t in &mut out.tris {
            t.swap(1, 2);
        }
    }
    // the points of the scan keep their normals, turned with the faces; the
    // points of the fillings and of the base get those of the closed solid
    if let Some(normals) = &mesh.normals {
        let all = out.compute_vertex_normals();
        let mut nv: Vec<Vec3> = if flipped {
            normals.iter().map(|v| -*v).collect()
        } else {
            normals.clone()
        };
        nv.extend_from_slice(&all[n..]);
        out.normals = Some(nv);
    }
    // who closed it is the caller's to say (the import by default)
    let scan = ScanBase {
        vertices: n as u32,
        faces: scan_faces as u32,
        flipped,
        closed_by: ClosedBy::default(),
    };
    let solid = dc_mesh::solid_topology(&out);
    Ok(Some(Based {
        mesh: out,
        closed_edges: op.open_edges,
        sources,
        plane,
        scan,
        removed_faces,
        open_edges: solid.boundary_edges,
        nonmanifold_edges: solid.nonmanifold_edges,
    }))
}

/// The fillings of the holes of a scan as they are made: the points (the
/// scan's, then the new ones), the point of the scan each new one comes
/// from, the triangles, and every edge there is (the scan's and the
/// fillings'), so that no filling puts a third triangle on an edge.
struct Filling {
    positions: Vec<Vec3>,
    sources: Vec<u32>,
    tris: Vec<[u32; 3]>,
    edges: HashSet<u64>,
}

impl Filling {
    fn add(&mut self, t: [u32; 3]) {
        self.edges.extend(tri_edges(t));
        self.tris.push(t);
    }

    /// Fills the hole `points` (a cycle, its triangles on the left) with
    /// the triangulation of its points of least area whose diagonals are
    /// not edges yet; with a fan from its middle point when it is longer
    /// than [`FAN_FROM`] points or has no such triangulation.
    fn least_area(&mut self, points: &[u32]) {
        // the triangles run the cycle the other way round
        let ring: Vec<u32> = points.iter().rev().copied().collect();
        let p: Vec<DVec3> = ring
            .iter()
            .map(|&w| self.positions[w as usize].as_dvec3())
            .collect();
        let edges = &self.edges;
        let local = (ring.len() <= FAN_FROM)
            .then(|| least_area(&p, &|a, b| edges.contains(&edge_key(ring[a], ring[b]))))
            .flatten();
        match local {
            Some(local) => {
                for t in local {
                    self.add(t.map(|i| ring[i]));
                }
            }
            None => {
                let c = self.positions.len() as u32;
                let mid = p.iter().sum::<DVec3>() / p.len() as f64;
                self.positions.push(mid.as_vec3());
                self.sources.push(points[0]);
                for (a, b) in ring_edges(points) {
                    self.add([b, a, c]);
                }
            }
        }
    }

    /// Fills the holes `holes` (cycles of `op` that touch no other one)
    /// with a smooth patch ([`holefill::fill_holes`]) and returns those it
    /// could not patch. The patching fills every loop of the mesh it is
    /// given but the largest one: it gets the kept triangles of the scan
    /// `scan_tris` with the fillings so far, the borders closed by a fan
    /// for the time being, and one triangle far away larger than any hole.
    fn patch(
        &mut self,
        scan_tris: &[[u32; 3]],
        op: &Openings,
        border: &[bool],
        holes: &[usize],
    ) -> Result<Vec<usize>, BaseError> {
        if holes.is_empty() {
            return Ok(Vec::new());
        }
        let mut positions = self.positions.clone();
        let mut tris: Vec<[u32; 3]> = scan_tris.iter().chain(&self.tris).copied().collect();
        for c in op
            .cycles
            .iter()
            .zip(border)
            .filter(|(_, b)| **b)
            .map(|(c, _)| c)
        {
            let mid = c
                .points
                .iter()
                .map(|&w| positions[w as usize].as_dvec3())
                .sum::<DVec3>()
                / c.points.len() as f64;
            let center = positions.len() as u32;
            positions.push(mid.as_vec3());
            tris.extend(ring_edges(&c.points).map(|(a, b)| [b, a, center]));
        }
        let d = positions.len() as u32;
        let (far, side) = (1.0e6_f32, 1.0e5_f32);
        positions.extend([
            Vec3::splat(far),
            Vec3::new(far + side, far, far),
            Vec3::new(far, far + side, far),
        ]);
        tris.push([d, d + 1, d + 2]);
        let work = TriMesh::new(positions, tris);
        let params = HoleFillParams {
            max_boundary_edges: usize::MAX,
            ..HoleFillParams::default()
        };
        let (patched, _) = holefill::fill_holes(&work, &params).map_err(|_| BaseError::Mesh)?;
        // the points of the patches follow those made so far
        let (base, first) = (work.positions.len(), self.positions.len());
        let shift = |w: u32| {
            if w as usize >= base {
                (w as usize - base + first) as u32
            } else {
                w
            }
        };
        let new: Vec<[u32; 3]> = patched.tris[work.tris.len()..]
            .iter()
            .map(|t| t.map(shift))
            .collect();
        self.positions.extend_from_slice(&patched.positions[base..]);
        let sources = sources_through(first, self.positions.len(), &new);
        self.sources.extend(sources);
        let made: HashSet<(u32, u32)> = new
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .collect();
        for t in new {
            self.add(t);
        }
        // a patch runs the edges of its hole the other way round
        Ok(holes
            .iter()
            .copied()
            .filter(|&k| {
                let p = &op.cycles[k].points;
                !made.contains(&(p[1], p[0]))
            })
            .collect())
    }
}

/// For each point from `first` to `end`, a point before `first` it is joined
/// to through the triangles `tris` (the first one found); `0` for a point
/// they join to none.
fn sources_through(first: usize, end: usize, tris: &[[u32; 3]]) -> Vec<u32> {
    let mut src = vec![u32::MAX; end - first];
    loop {
        let mut changed = false;
        for t in tris {
            let known = t.iter().find_map(|&w| {
                let s = if (w as usize) < first {
                    w
                } else {
                    src[w as usize - first]
                };
                (s != u32::MAX).then_some(s)
            });
            let Some(s) = known else { continue };
            for &w in t {
                if w as usize >= first && src[w as usize - first] == u32::MAX {
                    src[w as usize - first] = s;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    src.into_iter()
        .map(|s| if s == u32::MAX { 0 } else { s })
        .collect()
}

/// Triangulation of least area of the closed polygon `p` (3D): triangles
/// `[i, m, j]` with `i < m < j`, which run the polygon in its order; no
/// diagonal `(i, j)` for which `forbidden` holds. `None` when there is
/// none.
fn least_area(p: &[DVec3], forbidden: &dyn Fn(usize, usize) -> bool) -> Option<Vec<[usize; 3]>> {
    let n = p.len();
    if n < 3 {
        return None;
    }
    let at = |i: usize, j: usize| i * n + j;
    let mut cost = vec![f64::INFINITY; n * n];
    let mut best = vec![usize::MAX; n * n];
    for i in 0..n - 1 {
        cost[at(i, i + 1)] = 0.0;
    }
    for gap in 2..n {
        for i in 0..n - gap {
            let j = i + gap;
            // (0, n - 1) is the edge that closes the polygon, any other a
            // diagonal
            if (i, j) != (0, n - 1) && forbidden(i, j) {
                continue;
            }
            for m in i + 1..j {
                let c = cost[at(i, m)] + cost[at(m, j)] + (p[m] - p[i]).cross(p[j] - p[i]).length();
                if c < cost[at(i, j)] {
                    cost[at(i, j)] = c;
                    best[at(i, j)] = m;
                }
            }
        }
    }
    if !cost[at(0, n - 1)].is_finite() {
        return None;
    }
    let mut out = Vec::with_capacity(n - 2);
    let mut stack = vec![(0, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j - i < 2 {
            continue;
        }
        let m = best[at(i, j)];
        out.push([i, m, j]);
        stack.push((i, m));
        stack.push((m, j));
    }
    Some(out)
}

/// The edges of the closed ring `ring`, in its order.
pub(crate) fn ring_edges(ring: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()]))
}

/// Key of the undirected edge `a b`.
fn edge_key(a: u32, b: u32) -> u64 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    (u64::from(lo) << 32) | u64::from(hi)
}

/// The undirected edges of the triangle `t` (none between two equal
/// corners).
fn tri_edges(t: [u32; 3]) -> impl Iterator<Item = u64> {
    [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
        .into_iter()
        .filter(|(a, b)| a != b)
        .map(|(a, b)| edge_key(a, b))
}

/// The boundary of a mesh as [`make_base`] and the study models close it
/// (see the module documentation): the triangles kept by the repair, the
/// cycles of their open edges, and the pieces they form with their
/// footprints, seen along a direction.
#[derive(Debug, Clone, Default)]
pub(crate) struct Openings {
    /// Per triangle: kept, or taken out by the repair.
    pub kept: Vec<bool>,
    /// Open edges (of one triangle) of the triangles as they were given.
    pub open_edges: usize,
    /// The cycles of the open edges of the kept triangles.
    pub cycles: Vec<Cycle>,
    /// Per piece, its footprint: the area of the convex hull of its points
    /// seen along the direction (mm²).
    pub footprints: Vec<f64>,
    /// Per point, its piece (`u32::MAX` for a point of no kept triangle).
    pub piece_of: Vec<u32>,
}

/// A cycle of open edges of [`Openings`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cycle {
    /// Its points in the winding of the triangles (theirs on the left of
    /// each edge), each once.
    pub points: Vec<u32>,
    /// Its piece.
    pub piece: u32,
    /// The area it encloses seen along the direction, positive when it goes
    /// round anticlockwise (mm²).
    pub area: f64,
}

impl Openings {
    /// The share of the footprint of its piece that the cycle `c` encloses.
    pub(crate) fn share(&self, c: &Cycle) -> f64 {
        let f = self.footprints[c.piece as usize];
        if f > 0.0 { c.area.abs() / f } else { 0.0 }
    }
}

/// The boundary of the triangles `tris` on the points `positions`, seen
/// along the unit vector `z` ([`Openings`]).
pub(crate) fn openings(positions: &[Vec3], tris: &[[u32; 3]], z: DVec3) -> Openings {
    let open_edges = {
        let mut keys: Vec<u64> = tris.iter().flat_map(|t| tri_edges(*t)).collect();
        keys.sort_unstable();
        keys.chunk_by(|a, b| a == b)
            .filter(|r| r.len() == 1)
            .count()
    };
    let kept = repair(positions, tris);
    let rings = boundary_cycles(positions, tris, &kept);
    let (piece_of, pieces) = pieces(positions.len(), tris, &kept);
    let (u, v) = plane_axes(z);
    let flat = |w: u32| {
        let p = positions[w as usize].as_dvec3();
        DVec2::new(p.dot(u), p.dot(v))
    };
    let mut members: Vec<Vec<DVec2>> = vec![Vec::new(); pieces];
    for (w, &k) in piece_of.iter().enumerate() {
        if k != u32::MAX {
            members[k as usize].push(flat(w as u32));
        }
    }
    let footprints = members
        .iter()
        .map(|m| polygon_area(&convex_hull(m)))
        .collect();
    let cycles = rings
        .into_iter()
        .map(|points| {
            let q: Vec<DVec2> = points.iter().map(|&w| flat(w)).collect();
            Cycle {
                piece: piece_of[points[0] as usize],
                area: polygon_area(&q),
                points,
            }
        })
        .collect();
    Openings {
        kept,
        open_edges,
        cycles,
        footprints,
        piece_of,
    }
}

/// Which cycles of `op` are borders, walled down to a base (see the module
/// documentation): in each piece the cycle that encloses the most of it,
/// when that is at least [`BORDER_SHARE`] of its footprint, and the other
/// cycles that go round the same way, enclose as much and share no point
/// with a border.
pub(crate) fn borders(op: &Openings) -> Vec<bool> {
    let mut border = vec![false; op.cycles.len()];
    let mut order: Vec<usize> = (0..op.cycles.len()).collect();
    order.sort_by(|&a, &b| {
        op.cycles[b]
            .area
            .abs()
            .total_cmp(&op.cycles[a].area.abs())
            .then(a.cmp(&b))
    });
    // the way the main border of each piece goes round
    let mut way: HashMap<u32, bool> = HashMap::new();
    let mut taken: HashSet<u32> = HashSet::new();
    for k in order {
        let c = &op.cycles[k];
        if op.share(c) < BORDER_SHARE {
            continue;
        }
        let anticlockwise = *way.entry(c.piece).or_insert(c.area > 0.0);
        if (c.area > 0.0) != anticlockwise || c.points.iter().any(|w| taken.contains(w)) {
            continue;
        }
        taken.extend(c.points.iter().copied());
        border[k] = true;
    }
    border
}

/// The triangles a closed solid keeps (see the module documentation): on an
/// edge of more than two triangles the pair that runs it opposite ways and
/// turns least stays (then the pair with fewer such edges); with no such
/// pair the triangle with fewer such edges, then the largest. The others
/// go. Two triangles that run their edge the same way stay as they are
/// (turned triangles are no edge too many: taking them out would take out
/// every other one of a surface wound both ways). Once over every such
/// edge: taking triangles out makes none.
fn repair(positions: &[Vec3], tris: &[[u32; 3]]) -> Vec<bool> {
    let mut kept = vec![true; tris.len()];
    let normal = |f: u32| {
        let [a, b, c] = tris[f as usize].map(|i| positions[i as usize].as_dvec3());
        (b - a).cross(c - a)
    };
    // the half-edges by edge: (edge, run from its smaller point, triangle)
    let mut half: Vec<(u64, bool, u32)> = Vec::with_capacity(3 * tris.len());
    for (f, t) in tris.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if a != b {
                half.push((edge_key(a, b), a < b, f as u32));
            }
        }
    }
    half.sort_unstable();
    let bad: Vec<&[(u64, bool, u32)]> = half
        .chunk_by(|a, b| a.0 == b.0)
        .filter(|r| r.len() > 2)
        .collect();
    let mut faults: HashMap<u32, u32> = HashMap::new();
    for h in bad.iter().copied().flatten() {
        *faults.entry(h.2).or_default() += 1;
    }
    let fault = |f: u32| faults.get(&f).copied().unwrap_or(0);
    for run in bad {
        let live: Vec<(bool, u32)> = run
            .iter()
            .filter(|h| kept[h.2 as usize])
            .map(|h| (h.1, h.2))
            .collect();
        if live.len() <= 2 {
            continue;
        }
        let forward = live.iter().filter(|h| h.0).count();
        let stay: Vec<u32> = if forward > 0 && forward < live.len() {
            let mut best: Option<(f64, u32, [u32; 2])> = None;
            for &(_, f) in live.iter().filter(|h| h.0) {
                for &(_, g) in live.iter().filter(|h| !h.0) {
                    let turn = normal(f)
                        .normalize_or_zero()
                        .dot(normal(g).normalize_or_zero());
                    let faults = fault(f) + fault(g);
                    if best.is_none_or(|(t, k, _)| turn > t || (turn == t && faults < k)) {
                        best = Some((turn, faults, [f, g]));
                    }
                }
            }
            best.map(|(_, _, pair)| pair.to_vec()).unwrap_or_default()
        } else {
            live.iter()
                .map(|h| h.1)
                .min_by(|&f, &g| {
                    fault(f)
                        .cmp(&fault(g))
                        .then(normal(g).length().total_cmp(&normal(f).length()))
                })
                .into_iter()
                .collect()
        };
        for &(_, f) in &live {
            if !stay.contains(&f) {
                kept[f as usize] = false;
            }
        }
    }
    kept
}

/// The cycles of the open edges of the kept triangles (see the module
/// documentation); open edges that close none (where the windings of the
/// triangles around a point disagree, which the repair leaves none of)
/// are left out.
fn boundary_cycles(positions: &[Vec3], tris: &[[u32; 3]], kept: &[bool]) -> Vec<Vec<u32>> {
    let live = || tris.iter().zip(kept).filter(|(_, k)| **k).map(|(t, _)| t);
    let mut half: Vec<(u64, u32, u32)> = Vec::with_capacity(3 * tris.len());
    for t in live() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if a != b {
                half.push((edge_key(a, b), a, b));
            }
        }
    }
    half.sort_unstable();
    let mut open: Vec<(u32, u32)> = half
        .chunk_by(|a, b| a.0 == b.0)
        .filter(|r| r.len() == 1)
        .map(|r| (r[0].1, r[0].2))
        .collect();
    drop(half);
    if open.is_empty() {
        return Vec::new();
    }
    open.sort_unstable();
    let points = positions.len();
    let mut first = vec![u32::MAX; points];
    let mut outs = vec![0u32; points];
    let mut ins = vec![0u32; points];
    for (i, &(a, b)) in open.iter().enumerate() {
        if first[a as usize] == u32::MAX {
            first[a as usize] = i as u32;
        }
        outs[a as usize] += 1;
        ins[b as usize] += 1;
    }
    let mut next = vec![u32::MAX; open.len()];
    // the normal about each point where the boundary passes more than once
    // (or the windings disagree), and the open edges coming into it
    let mut around: HashMap<u32, (DVec3, Vec<u32>)> = HashMap::new();
    for (i, &(_, b)) in open.iter().enumerate() {
        if outs[b as usize] == 1 && ins[b as usize] == 1 {
            next[i] = first[b as usize];
        } else {
            around.entry(b).or_default().1.push(i as u32);
        }
    }
    if !around.is_empty() {
        for t in live() {
            if t.iter().any(|w| around.contains_key(w)) {
                let [a, b, c] = t.map(|i| positions[i as usize].as_dvec3());
                let n = (b - a).cross(c - a);
                for w in t {
                    if let Some(e) = around.get_mut(w) {
                        e.0 += n;
                    }
                }
            }
        }
        let mut special: Vec<u32> = around.keys().copied().collect();
        special.sort_unstable();
        for p in special {
            let (n, incoming) = &around[&p];
            // seen from outside the triangles turn anticlockwise about the
            // normal: the gap an incoming edge opens closes at the first
            // outgoing edge anticlockwise from it
            let (e1, e2) = plane_axes(if n.length() > 0.0 {
                n.normalize()
            } else {
                DVec3::Z
            });
            let o = positions[p as usize].as_dvec3();
            let angle = |q: u32| {
                let d = positions[q as usize].as_dvec3() - o;
                d.dot(e2).atan2(d.dot(e1))
            };
            let outgoing: Vec<(f64, u32)> = match first[p as usize] {
                u32::MAX => Vec::new(),
                s => (s..s + outs[p as usize])
                    .map(|j| (angle(open[j as usize].1), j))
                    .collect(),
            };
            let mut incoming: Vec<(f64, u32)> = incoming
                .iter()
                .map(|&i| (angle(open[i as usize].0), i))
                .collect();
            incoming.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut free = vec![true; outgoing.len()];
            for (a, i) in incoming {
                let pick = (0..outgoing.len()).filter(|&k| free[k]).min_by(|&x, &y| {
                    anticlockwise(a, outgoing[x].0)
                        .total_cmp(&anticlockwise(a, outgoing[y].0))
                        .then(x.cmp(&y))
                });
                if let Some(k) = pick {
                    free[k] = false;
                    next[i as usize] = outgoing[k].1;
                }
            }
        }
    }
    // follow them; a walk that does not come back is no cycle
    let mut done = vec![false; open.len()];
    let mut cycles = Vec::new();
    for s in 0..open.len() {
        if done[s] {
            continue;
        }
        let mut walk = Vec::new();
        let mut e = s;
        let closed = loop {
            done[e] = true;
            walk.push(open[e].0);
            match next[e] {
                u32::MAX => break false,
                nx => e = nx as usize,
            }
            if e == s {
                break true;
            }
            if done[e] {
                break false;
            }
        };
        if closed {
            split_simple(walk, &mut cycles);
        }
    }
    cycles
}

/// The angle anticlockwise from the direction `a` to the direction `b`
/// (radians), in `(0, 2π]`.
fn anticlockwise(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(TAU);
    if d > 0.0 { d } else { TAU }
}

/// The cycle `walk` split at each point it passes twice into cycles that
/// pass each point once, added to `out`.
fn split_simple(walk: Vec<u32>, out: &mut Vec<Vec<u32>>) {
    let mut stack: Vec<u32> = Vec::with_capacity(walk.len());
    let mut at: HashMap<u32, usize> = HashMap::new();
    for w in walk {
        if let Some(&i) = at.get(&w) {
            // from the first visit of w to here: a cycle back to w
            let sub: Vec<u32> = stack.drain(i..).collect();
            for q in &sub[1..] {
                at.remove(q);
            }
            if sub.len() >= 3 {
                out.push(sub);
            }
            stack.push(w);
        } else {
            at.insert(w, stack.len());
            stack.push(w);
        }
    }
    if stack.len() >= 3 {
        out.push(stack);
    }
}

/// The pieces of the kept triangles, joined by their points: the piece of
/// each point (`u32::MAX` for a point of no kept triangle) and how many.
fn pieces(points: usize, tris: &[[u32; 3]], kept: &[bool]) -> (Vec<u32>, usize) {
    fn find(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            let up = parent[parent[x as usize] as usize];
            parent[x as usize] = up;
            x = up;
        }
        x
    }
    let mut parent: Vec<u32> = (0..points as u32).collect();
    let mut used = vec![false; points];
    for (t, _) in tris.iter().zip(kept).filter(|(_, k)| **k) {
        let r0 = find(&mut parent, t[0]);
        for &w in t {
            used[w as usize] = true;
            let r = find(&mut parent, w);
            if r != r0 {
                parent[r as usize] = r0;
            }
        }
    }
    let mut number = vec![u32::MAX; points];
    let mut piece = vec![u32::MAX; points];
    let mut count = 0;
    for w in 0..points as u32 {
        if !used[w as usize] {
            continue;
        }
        let r = find(&mut parent, w) as usize;
        if number[r] == u32::MAX {
            number[r] = count;
            count += 1;
        }
        piece[w as usize] = number[r];
    }
    (piece, count as usize)
}

/// Signed area of the polygon `q`, positive anticlockwise.
fn polygon_area(q: &[DVec2]) -> f64 {
    (0..q.len())
        .map(|i| q[i].perp_dot(q[(i + 1) % q.len()]))
        .sum::<f64>()
        / 2.0
}

/// Convex hull, counter-clockwise (monotone chain).
pub(crate) fn convex_hull(pts: &[DVec2]) -> Vec<DVec2> {
    let mut p: Vec<DVec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<DVec2> = Vec::with_capacity(2 * p.len());
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for q in it {
            while h.len() >= start + 2 {
                let (a, b) = (h[h.len() - 2], h[h.len() - 1]);
                if (b - a).perp_dot(*q - a) <= 0.0 {
                    h.pop();
                } else {
                    break;
                }
            }
            h.push(*q);
        }
        h.pop();
    }
    h
}

/// The scan [`make_base`] closed into `closed`, as `base` (its
/// [`Based::scan`]) says where it ends: its points and triangles in their
/// order, the triangles turned back when the base turned them, with their
/// labels, colours and normals. Exactly the scan [`make_base`] was given.
/// `None` when `base` does not fit `closed`: more points or triangles than
/// it has, or a triangle of the scan on a point of the base.
pub fn open_part(closed: &TriMesh, base: &ScanBase) -> Option<TriMesh> {
    let (nv, nf) = (base.vertices as usize, base.faces as usize);
    if nv > closed.positions.len() || nf > closed.tris.len() {
        return None;
    }
    let mut tris = closed.tris[..nf].to_vec();
    if tris.iter().flatten().any(|&i| i as usize >= nv) {
        return None;
    }
    if base.flipped {
        for t in &mut tris {
            t.swap(1, 2);
        }
    }
    let mut out = TriMesh::new(closed.positions[..nv].to_vec(), tris);
    out.normals = closed.normals.as_ref().map(|n| {
        if base.flipped {
            n[..nv].iter().map(|v| -*v).collect()
        } else {
            n[..nv].to_vec()
        }
    });
    out.vertex_labels = closed.vertex_labels.as_ref().map(|l| l[..nv].to_vec());
    out.face_labels = closed.face_labels.as_ref().map(|l| l[..nf].to_vec());
    out.vertex_colors = closed.vertex_colors.as_ref().map(|c| c[..nv].to_vec());
    Some(out)
}

/// The occlusal direction of an open arch scan without its tooth marks, for
/// the base the import closes it with: the smallest principal axis of its
/// points, pointing away from its open border, as the automatic tooth marks
/// start (`autoseed::occlusal_axis`, the plugin's `autoseed._occlusal_axis`).
///
/// Measured against the occlusal normal of the tooth marks
/// (`frame::run_from_seeds`): 1.6 to 12.6 degrees on four real scans, 0.5
/// to 4.8 degrees on the six demo arches opened under the gum; never on the
/// side of the teeth. The base of an arch with its tooth marks can be made
/// again with their frame (`aligners.case.make-base`).
pub fn occlusal_direction(mesh: &TriMesh) -> Result<DVec3, BaseError> {
    let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    autoseed::occlusal_axis(&v, &mesh.tris)
        .map(|(n, _)| n)
        .map_err(|_| BaseError::Mesh)
}

/// `true` when `mesh` has an edge of one triangle only: an open scan that
/// [`make_base`] would close. Sorts the edges instead of building the
/// adjacency: fast enough to be asked once per scan by the window.
pub fn is_open(mesh: &TriMesh) -> bool {
    let mut edges: Vec<u64> = mesh
        .tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .filter(|(a, b)| a != b)
        .map(|(a, b)| (u64::from(a.min(b)) << 32) | u64::from(a.max(b)))
        .collect();
    edges.sort_unstable();
    edges.chunk_by(|a, b| a == b).any(|run| run.len() == 1)
}

/// Volume enclosed by a closed mesh, positive with outward normals (mm³).
pub fn signed_volume(mesh: &TriMesh) -> f64 {
    mesh.tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3());
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        / 6.0
}

/// Two unit axes spanning the plane perpendicular to the unit vector `z`,
/// with `u × v = z`.
fn plane_axes(z: DVec3) -> (DVec3, DVec3) {
    let helper = if z.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    let u = helper.cross(z).normalize();
    (u, z.cross(u))
}

/// Twice the signed area of the triangle `a b c` (positive anticlockwise).
fn cross(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    (b - a).perp_dot(c - a)
}

/// Triangles of the polygon `p` (indices into `p`), in the winding of the
/// polygon: ear clipping, `n - 2` triangles for `n` points. A polygon that
/// is not simple (or has repeated points) still gets `n - 2` triangles over
/// its boundary: when no ear is left the most convex corner is cut.
fn ear_clip(p: &[DVec2]) -> Vec<[usize; 3]> {
    let n = p.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    // orientation of the polygon: corners turning this way are convex
    let area: f64 = (0..n).map(|i| p[i].perp_dot(p[(i + 1) % n])).sum();
    let sign = if area < 0.0 { -1.0 } else { 1.0 };
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let turn =
        |prev: &[usize], next: &[usize], i: usize| sign * cross(p[prev[i]], p[i], p[next[i]]);
    let mut reflex: Vec<bool> = (0..n).map(|i| turn(&prev, &next, i) <= 0.0).collect();
    let mut alive = n;
    let mut i = 0;
    let mut tried = 0;
    while alive > 3 {
        let (a, c) = (prev[i], next[i]);
        let is_ear = !reflex[i] && {
            // no other corner of the polygon inside the triangle a i c
            let mut k = next[c];
            let mut clear = true;
            while k != a {
                if reflex[k] && inside(p[k], p[a], p[i], p[c], sign) {
                    clear = false;
                    break;
                }
                k = next[k];
            }
            clear
        };
        let cut = if is_ear {
            Some(i)
        } else if tried >= alive {
            // no ear in a whole turn: the most convex corner
            let mut best = i;
            let mut best_turn = f64::NEG_INFINITY;
            let mut k = i;
            for _ in 0..alive {
                let t = turn(&prev, &next, k);
                if t > best_turn {
                    best_turn = t;
                    best = k;
                }
                k = next[k];
            }
            Some(best)
        } else {
            None
        };
        match cut {
            Some(e) => {
                let (a, c) = (prev[e], next[e]);
                out.push([a, e, c]);
                next[a] = c;
                prev[c] = a;
                alive -= 1;
                reflex[a] = turn(&prev, &next, a) <= 0.0;
                reflex[c] = turn(&prev, &next, c) <= 0.0;
                i = c;
                tried = 0;
            }
            None => {
                i = next[i];
                tried += 1;
            }
        }
    }
    out.push([prev[i], i, next[i]]);
    out
}

/// `q` inside or on the triangle `a b c` of winding `sign` (and not one of
/// its corners).
fn inside(q: DVec2, a: DVec2, b: DVec2, c: DVec2, sign: f64) -> bool {
    if q == a || q == b || q == c {
        return false;
    }
    sign * cross(a, b, q) >= 0.0 && sign * cross(b, c, q) >= 0.0 && sign * cross(c, a, q) >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An open box: a square tube `w` wide from z = 0 to z = `h`, closed on
    /// top, open at the bottom, with outward faces; `k` points per side of
    /// the bottom rim.
    fn open_box(w: f32, h: f32, k: usize) -> TriMesh {
        let mut ring = Vec::new();
        for s in 0..4 {
            for j in 0..k {
                let t = j as f32 / k as f32 * w;
                ring.push(match s {
                    0 => Vec3::new(t, 0.0, 0.0),
                    1 => Vec3::new(w, t, 0.0),
                    2 => Vec3::new(w - t, w, 0.0),
                    _ => Vec3::new(0.0, w - t, 0.0),
                });
            }
        }
        let m = ring.len() as u32;
        let mut positions = ring.clone();
        positions.extend(ring.iter().map(|p| Vec3::new(p.x, p.y, h)));
        let top = positions.len() as u32;
        positions.push(Vec3::new(w / 2.0, w / 2.0, h));
        let mut tris = Vec::new();
        for i in 0..m {
            let j = (i + 1) % m;
            // the ring runs anticlockwise seen from above: outward walls
            tris.push([i, j, j + m]);
            tris.push([i, j + m, i + m]);
            tris.push([i + m, j + m, top]);
        }
        TriMesh::new(positions, tris)
    }

    #[test]
    fn an_open_box_gets_a_flat_watertight_base() {
        let mut mesh = open_box(10.0, 5.0, 3);
        mesh.vertex_labels = Some((0..mesh.positions.len() as u16).collect());
        let n = mesh.positions.len();
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        // 12 rim edges, 12 new points under them, 3 mm below the lowest point
        assert_eq!(out.closed_edges, 12);
        assert_eq!(out.sources.len(), 12);
        assert_eq!(out.plane, -3.0);
        let report = out.mesh.validate();
        assert!(report.is_watertight(), "{report:?}");
        // the scan keeps its points and their order
        assert_eq!(&out.mesh.positions[..n], &mesh.positions[..]);
        for (k, &s) in out.sources.iter().enumerate() {
            let p = out.mesh.positions[n + k];
            let q = mesh.positions[s as usize];
            assert_eq!((p.x, p.y, p.z), (q.x, q.y, -3.0));
            // gingiva, whatever the label of the point it comes from
            assert_eq!(out.mesh.vertex_labels.as_ref().unwrap()[n + k], 0);
        }
        // the scan as it was given is the start of the closed box
        assert_eq!(
            out.scan,
            ScanBase {
                vertices: n as u32,
                faces: mesh.tris.len() as u32,
                flipped: false,
                closed_by: ClosedBy::Import,
            }
        );
        assert_eq!(open_part(&out.mesh, &out.scan).as_ref(), Some(&mesh));
        // the closed box: 10 x 10 x 8 mm, outward normals
        assert!((signed_volume(&out.mesh) - 800.0).abs() < 1e-3);
        // every face of the cap lies on the plane and faces down
        for f in mesh.tris.len() + 24..out.mesh.tris.len() {
            assert!(out.mesh.triangle(f).iter().all(|p| p.z == -3.0));
            assert!(out.mesh.face_normal(f).z < -0.999, "face {f}");
        }
    }

    #[test]
    fn an_inverted_scan_comes_out_with_outward_normals_and_back_as_it_was() {
        let mut mesh = open_box(10.0, 5.0, 2);
        for t in &mut mesh.tris {
            t.swap(1, 2);
        }
        // normals as wound (inwards), labels and colours: all come back
        mesh.update_normals();
        mesh.vertex_labels = Some(vec![11; mesh.positions.len()]);
        mesh.vertex_colors = Some(vec![[1, 2, 3, 255]; mesh.positions.len()]);
        mesh.face_labels = Some(vec![7; mesh.tris.len()]);
        let out = make_base(&mesh, DVec3::Z, 0.5).unwrap().unwrap();
        assert!(out.mesh.validate().is_watertight());
        assert!((signed_volume(&out.mesh) - 550.0).abs() < 1e-3);
        assert!(out.scan.flipped);
        // the normals of the closed solid point out, those of the scan too
        let normals = out.mesh.normals.as_ref().unwrap();
        assert_eq!(normals.len(), out.mesh.positions.len());
        let n = mesh.positions.len();
        assert_eq!(normals[n - 1], -mesh.normals.as_ref().unwrap()[n - 1]);
        assert!(
            normals[n..].iter().all(|v| v.z < 0.0),
            "the base faces down"
        );
        let back = open_part(&out.mesh, &out.scan).unwrap();
        assert_eq!(back, mesh);
    }

    #[test]
    fn a_record_that_does_not_fit_the_mesh_gives_no_scan() {
        let mesh = open_box(10.0, 5.0, 2);
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        let n = out.mesh.positions.len() as u32;
        let f = out.mesh.tris.len() as u32;
        for bad in [
            ScanBase {
                vertices: n + 1,
                ..out.scan
            },
            ScanBase {
                faces: f + 1,
                ..out.scan
            },
            // a triangle of the scan on a point of the base
            ScanBase {
                faces: out.scan.faces + 1,
                ..out.scan
            },
        ] {
            assert_eq!(open_part(&out.mesh, &bad), None, "{bad:?}");
        }
        // the whole solid is a record that fits
        let whole = ScanBase {
            vertices: n,
            faces: f,
            flipped: false,
            closed_by: ClosedBy::Import,
        };
        assert_eq!(open_part(&out.mesh, &whole).as_ref(), Some(&out.mesh));
    }

    #[test]
    fn a_hole_inside_the_scan_is_filled_in_place_never_walled_down() {
        // a flat 10 x 10 mm sheet at z = 0 (1 mm cells, faces up) with a
        // 2 x 2 mm hole in its middle, as a few points missing on a cusp
        let k = 11u32;
        let positions: Vec<Vec3> = (0..k * k)
            .map(|i| Vec3::new((i % k) as f32, (i / k) as f32, 0.0))
            .collect();
        let mut tris = Vec::new();
        for y in 0..k - 1 {
            for x in 0..k - 1 {
                if (4..6).contains(&x) && (4..6).contains(&y) {
                    continue;
                }
                let a = y * k + x;
                tris.push([a, a + 1, a + k + 1]);
                tris.push([a, a + k + 1, a + k]);
            }
        }
        let mut mesh = TriMesh::new(positions, tris);
        mesh.vertex_labels = Some(vec![23; mesh.positions.len()]);
        mesh.vertex_colors = Some(vec![[9, 9, 9, 255]; mesh.positions.len()]);
        let n = mesh.positions.len();
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        // the border (40 edges) and the hole (8) are closed
        assert_eq!(out.closed_edges, 48);
        assert!(out.mesh.validate().is_watertight());
        // a solid slab 10 x 10 x 3 mm: no chimney from the hole to the base
        assert!(
            (signed_volume(&out.mesh) - 300.0).abs() < 1e-3,
            "{}",
            signed_volume(&out.mesh)
        );
        // the base has a point under each of the 40 points of the border
        // only; the patch lies in the plane of the sheet
        let added = &out.mesh.positions[n..];
        assert_eq!(added.len(), out.sources.len());
        assert_eq!(added.iter().filter(|p| p.z == -3.0).count(), 40);
        assert!(added.iter().all(|p| p.z == -3.0 || p.z.abs() < 1e-6));
        assert!(out.sources.iter().all(|&s| (s as usize) < n));
        // gingiva on the patch and the base, the colour of the scan
        let labels = out.mesh.vertex_labels.as_ref().unwrap();
        assert!(labels[n..].iter().all(|&l| l == 0));
        let colors = out.mesh.vertex_colors.as_ref().unwrap();
        assert!(colors[n..].iter().all(|c| *c == [9, 9, 9, 255]));
        // the scan as it was read comes back
        assert_eq!(open_part(&out.mesh, &out.scan).as_ref(), Some(&mesh));
    }

    #[test]
    fn an_open_scan_is_told_and_its_teeth_side_found_without_tooth_marks() {
        // 32 rim points: the open border gives the side (away from it, +Z)
        let mesh = open_box(10.0, 5.0, 8);
        assert!(is_open(&mesh));
        let n = occlusal_direction(&mesh).unwrap();
        assert!(n.dot(DVec3::Z) > 0.999, "{n}");
        let out = make_base(&mesh, n, 3.0).unwrap().unwrap();
        assert!(!is_open(&out.mesh));
        assert!(!is_open(&TriMesh::default()));
        assert_eq!(
            occlusal_direction(&TriMesh::default()),
            Err(BaseError::Mesh)
        );
    }

    #[test]
    fn a_closed_scan_is_left_alone_and_bad_input_is_refused() {
        let box_mesh = open_box(10.0, 5.0, 2);
        let closed = make_base(&box_mesh, DVec3::Z, 3.0).unwrap().unwrap().mesh;
        assert_eq!(make_base(&closed, DVec3::Z, 3.0), Ok(None));
        for h in [0.4, 20.5, f64::NAN] {
            assert!(matches!(
                make_base(&box_mesh, DVec3::Z, h),
                Err(BaseError::Height(_))
            ));
        }
        assert_eq!(
            make_base(&box_mesh, DVec3::ZERO, 3.0),
            Err(BaseError::Direction)
        );
        assert_eq!(
            make_base(&TriMesh::default(), DVec3::Z, 3.0),
            Err(BaseError::Mesh)
        );
        // the slider's ends are allowed
        assert!(
            make_base(&box_mesh, DVec3::Z, HEIGHT_MIN_MM)
                .unwrap()
                .is_some()
        );
        assert!(
            make_base(&box_mesh, DVec3::Z, HEIGHT_MAX_MM)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn the_base_is_under_the_lowest_point_along_any_direction() {
        // the box turned on its side (a rotation): the occlusal direction is -X
        let mut mesh = open_box(8.0, 4.0, 2);
        for p in &mut mesh.positions {
            *p = Vec3::new(-p.z, p.y, p.x);
        }
        let out = make_base(&mesh, DVec3::NEG_X, 2.0).unwrap().unwrap();
        // heights along -X are -x: the lowest is the open rim at x = 0, the
        // plane 2 mm further, at x = 2
        assert!((out.plane + 2.0).abs() < 1e-6);
        let n = mesh.positions.len();
        assert!(
            out.mesh.positions[n..]
                .iter()
                .all(|p| (p.x - 2.0).abs() < 1e-5)
        );
        assert!(out.mesh.validate().is_watertight());
        assert!((signed_volume(&out.mesh) - 8.0 * 8.0 * 6.0).abs() < 1e-3);
    }

    #[test]
    fn ear_clipping_covers_a_concave_polygon_once() {
        // a U, like the base of an arch, both windings
        let mut u: Vec<DVec2> = [
            (0.0, 0.0),
            (6.0, 0.0),
            (6.0, 6.0),
            (4.0, 6.0),
            (4.0, 2.0),
            (2.0, 2.0),
            (2.0, 6.0),
            (0.0, 6.0),
        ]
        .iter()
        .map(|&(x, y)| DVec2::new(x, y))
        .collect();
        for _ in 0..2 {
            let tris = ear_clip(&u);
            assert_eq!(tris.len(), u.len() - 2);
            let total: f64 = tris
                .iter()
                .map(|t| cross(u[t[0]], u[t[1]], u[t[2]]) / 2.0)
                .sum();
            assert!((total.abs() - 28.0).abs() < 1e-9, "{total}");
            // every triangle turns the way of the polygon
            let sign = total.signum();
            assert!(
                tris.iter()
                    .all(|t| sign * cross(u[t[0]], u[t[1]], u[t[2]]) > 0.0)
            );
            u.reverse();
        }
    }

    /// A flat sheet of `kx` × `ky` cells of 1 mm at z = 0, its faces up
    /// (+Z), without the cells `(x, y)` for which `hole` holds.
    fn sheet(kx: u32, ky: u32, hole: impl Fn(u32, u32) -> bool) -> TriMesh {
        let n = kx + 1;
        let positions = (0..n * (ky + 1))
            .map(|i| Vec3::new((i % n) as f32, (i / n) as f32, 0.0))
            .collect();
        let mut tris = Vec::new();
        for y in 0..ky {
            for x in 0..kx {
                if hole(x, y) {
                    continue;
                }
                let a = y * n + x;
                tris.push([a, a + 1, a + n + 1]);
                tris.push([a, a + n + 1, a + n]);
            }
        }
        TriMesh::new(positions, tris)
    }

    /// `mesh` counted as a solid, points by position: (open edges, edges of
    /// more than two triangles, pieces).
    fn solid(mesh: &TriMesh) -> (usize, usize, usize) {
        let s = dc_mesh::solid_topology(mesh);
        (s.boundary_edges, s.nonmanifold_edges, s.pieces)
    }

    #[test]
    fn holes_touching_at_a_point_are_filled_in_place_into_a_clean_solid() {
        // three 2 x 2 mm holes in a 10 x 10 mm sheet, the middle one touching
        // the others at (4, 4) and at (6, 4), one on each diagonal: the
        // boundary of the scan passes each of those points twice
        let mesh = sheet(10, 10, |x, y| {
            let block = |x0: u32, y0: u32| (x0..x0 + 2).contains(&x) && (y0..y0 + 2).contains(&y);
            block(2, 2) || block(4, 4) || block(6, 2)
        });
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        assert_eq!(solid(&out.mesh), (0, 0, 1));
        assert!(out.is_solid());
        // a slab 10 x 10 x 3 mm: the holes filled in the plane of the sheet,
        // only the border walled down
        let volume = signed_volume(&out.mesh);
        assert!((volume - 300.0).abs() < 1e-3, "{volume}");
        let n = mesh.positions.len();
        let added = &out.mesh.positions[n..];
        assert_eq!(added.iter().filter(|p| p.z == -3.0).count(), 40);
        assert!(added.iter().all(|p| p.z == -3.0 || p.z.abs() < 1e-6));
        assert_eq!(open_part(&out.mesh, &out.scan).as_ref(), Some(&mesh));
    }

    #[test]
    fn a_hole_touching_the_border_at_a_point_is_filled_and_the_border_walled_once() {
        // a notch in the side of a 10 x 10 mm sheet and a 2 x 2 mm hole whose
        // corner is a corner of the notch: the border passes (2, 6) twice
        let mesh = sheet(10, 10, |x, y| {
            (x < 2 && (4..6).contains(&y)) || ((2..4).contains(&x) && (6..8).contains(&y))
        });
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        assert_eq!(solid(&out.mesh), (0, 0, 1));
        // the 44 points of the border get one point of the base each, the hole
        // none: it is filled in place, the sheet without its notch on a slab
        assert_eq!(out.sources.len(), 44);
        let volume = signed_volume(&out.mesh);
        assert!((volume - 96.0 * 3.0).abs() < 1e-3, "{volume}");
        assert_eq!(open_part(&out.mesh, &out.scan).as_ref(), Some(&mesh));
    }

    #[test]
    fn a_face_folded_over_an_edge_is_taken_out_and_its_hole_filled() {
        // one more triangle on the edge (5, 5)-(6, 5) of a 10 x 10 mm sheet,
        // folded up from it: the edge has three triangles, the fold two edges
        // of its own that close nothing
        let mut mesh = sheet(10, 10, |_, _| false);
        let (a, b) = (5 * 11 + 5, 5 * 11 + 6);
        let c = mesh.positions.len() as u32;
        mesh.positions.push(Vec3::new(5.5, 5.0, 1.0));
        mesh.tris.push([a, b, c]);
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        assert_eq!(solid(&out.mesh), (0, 0, 1));
        assert_eq!(out.removed_faces, 1);
        // the slab of the sheet, 3 mm under it: the fold is not on it
        let volume = signed_volume(&out.mesh);
        assert!((volume - 300.0).abs() < 1e-3, "{volume}");
        // the scan comes back without the fold
        let mut clean = mesh.clone();
        clean.tris.pop();
        assert_eq!(open_part(&out.mesh, &out.scan).as_ref(), Some(&clean));
    }

    #[test]
    fn a_closed_model_with_a_hole_and_a_chimney_under_it_gets_no_base() {
        // a closed box 10 x 10 mm, 8 mm high, as an old closing left it: a
        // hole on its top and a strip hanging 4 mm into it from the hole
        let m = 32u32;
        let boxed = make_base(&open_box(10.0, 5.0, 8), DVec3::Z, 3.0)
            .unwrap()
            .unwrap()
            .mesh;
        let volume = signed_volume(&boxed);
        let mut mesh = boxed.clone();
        // the top triangle on the ring points m, m + 1 (the third of the box)
        let top = 2 * m;
        assert_eq!(mesh.tris.remove(2), [m, m + 1, top]);
        let down = |w: u32| mesh.positions[w as usize] - Vec3::new(0.0, 0.0, 4.0);
        let (a, b) = (down(top), down(m + 1));
        let k = mesh.positions.len() as u32;
        mesh.positions.extend([a, b]);
        mesh.tris.extend([[m + 1, top, k], [m + 1, k, k + 1]]);
        let lowest = mesh
            .positions
            .iter()
            .map(|p| p.z)
            .fold(f32::INFINITY, f32::min);
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        assert_eq!(solid(&out.mesh), (0, 0, 1));
        // the hole and the strip closed where they are: no base under the
        // model, the box as it was
        let n = mesh.positions.len();
        assert!(
            out.mesh.positions[n..].iter().all(|p| p.z >= lowest - 1e-5),
            "a base under a closed model"
        );
        let after = signed_volume(&out.mesh);
        assert!((after - volume).abs() < 5.0, "{after} for {volume}");
    }

    #[test]
    fn what_is_left_open_or_shared_is_counted_on_the_closed_scan() {
        // two sheets side by side, each with its own points on the line
        // x = 5: closed one by one, but where an STL file puts their points
        // together their walls stand on one another along that line
        let a = sheet(5, 10, |_, _| false);
        let n = a.positions.len() as u32;
        let mesh = TriMesh::new(
            a.positions
                .iter()
                .copied()
                .chain(a.positions.iter().map(|p| *p + Vec3::X * 5.0))
                .collect(),
            a.tris
                .iter()
                .copied()
                .chain(a.tris.iter().map(|t| t.map(|i| i + n)))
                .collect(),
        );
        let out = make_base(&mesh, DVec3::Z, 3.0).unwrap().unwrap();
        let (open, shared, _) = solid(&out.mesh);
        assert_eq!((out.open_edges, out.nonmanifold_edges), (open, shared));
        assert!(
            shared > 0 && !out.is_solid(),
            "{open} open, {shared} shared"
        );
    }
}
