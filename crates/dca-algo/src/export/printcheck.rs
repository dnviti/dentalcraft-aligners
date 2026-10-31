//! Print readiness of a triangle mesh (`ortho_aligner/printcheck.py`).
//!
//! A file is ready to print when it is one closed solid: every edge shared
//! by exactly two triangles with opposite orientation (watertight, manifold,
//! consistently oriented), no degenerate triangles, no self-intersections,
//! one connected piece and positive volume. A shell (aligner) and a model
//! must also have no hole through them (genus 0), a shell a minimum wall,
//! and the part must fit the build plate.
//!
//! The mesh is welded first (identical positions become one vertex): that is
//! what the slicer sees in the STL file, which has no connectivity. The
//! topology counts come from that welded mesh; the self-intersections use
//! the BVH of `dc-geom` (candidate triangles by box overlap, then an exact
//! triangle/triangle test); the thickness casts a ray into the solid from
//! the centre of the faces.
//!
//! Not ported: `shell_reach` (fins and flaps of an EQUATOR shell far from
//! the teeth) needs the teeth and bars of the stage, so it belongs to the
//! shell code.

use std::collections::HashMap;

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::text;
use crate::schedule::Lang;

/// Distance under which a point counts as on a plane (mm).
const PLANE_EPS: f64 = 1e-6;
/// Area (twice) under which a triangle is degenerate (the plugin's value).
const DEGENERATE_AREA2: f64 = 1e-10;
/// Inward ray start offset along the face (mm), to leave the face itself.
const RAY_EPS: f32 = 1e-4;
/// Pairs of intersecting triangles kept in the report.
const MAX_PAIRS: usize = 8;

/// What the check looks at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CheckOptions {
    /// Count the pairs of triangles that cross each other.
    pub self_intersections: bool,
    /// Wall that no part of a shell may be thinner than (mm); `None` skips
    /// the thickness (a model is solid).
    pub min_thickness_mm: Option<f64>,
    /// Faces probed for the thickness at most (an even sample of the mesh).
    pub thickness_samples: usize,
    /// Build volume of the printer `[x, y, z]` (mm); `None` skips the fit.
    pub build_plate_mm: Option<[f64; 3]>,
    /// Holes through the solid allowed (a model and a shell have none).
    pub max_genus: Option<u32>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            self_intersections: true,
            min_thickness_mm: None,
            thickness_samples: 20_000,
            build_plate_mm: Some(DEFAULT_PLATE_MM),
            max_genus: Some(0),
        }
    }
}

/// Build volume of the default printer (Formlabs Form 3: 145 x 145 x 185 mm).
pub const DEFAULT_PLATE_MM: [f64; 3] = [145.0, 145.0, 185.0];

/// Something that makes a file unfit to print.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Problem {
    /// The mesh has no triangle.
    Empty,
    /// Edges used by one triangle only (holes).
    BoundaryEdges {
        /// How many.
        count: usize,
    },
    /// Edges used by more than two triangles.
    NonManifoldEdges {
        /// How many.
        count: usize,
    },
    /// Edges whose two triangles disagree on the orientation.
    FlippedEdges {
        /// How many.
        count: usize,
    },
    /// Triangles with (almost) no area.
    Degenerate {
        /// How many.
        count: usize,
    },
    /// More than one connected piece.
    Pieces {
        /// How many.
        count: usize,
    },
    /// Pairs of triangles that cross each other.
    SelfIntersections {
        /// How many pairs.
        count: usize,
    },
    /// Holes through the wall.
    Genus {
        /// Handles of the surface.
        count: i64,
    },
    /// Zero or negative volume (inverted normals).
    NonPositiveVolume,
    /// A part of the wall is thinner than the minimum.
    TooThin {
        /// Thinnest wall found (mm).
        min_mm: f64,
        /// Minimum wall (mm).
        limit_mm: f64,
    },
    /// The bounding box does not fit the build plate.
    DoesNotFit {
        /// Size of the part (mm).
        size_mm: [f64; 3],
        /// Build volume (mm).
        plate_mm: [f64; 3],
    },
}

impl Problem {
    /// The problem in words.
    pub fn text(&self, lang: Lang) -> String {
        let n = |k: &str, args: &[String]| text::fmt(lang, k, args);
        match self {
            Problem::Empty => n("chk-empty", &[]),
            Problem::BoundaryEdges { count } => n("chk-boundary", &[count.to_string()]),
            Problem::NonManifoldEdges { count } => n("chk-nonmanifold", &[count.to_string()]),
            Problem::FlippedEdges { count } => n("chk-flipped", &[count.to_string()]),
            Problem::Degenerate { count } => n("chk-degenerate", &[count.to_string()]),
            Problem::Pieces { count } => n("chk-pieces", &[count.to_string()]),
            Problem::SelfIntersections { count } => n("chk-selfint", &[count.to_string()]),
            Problem::Genus { count } => n("chk-genus", &[count.to_string()]),
            Problem::NonPositiveVolume => n("chk-volume", &[]),
            Problem::TooThin { min_mm, limit_mm } => n(
                "chk-thin",
                &[format!("{min_mm:.2}"), format!("{limit_mm:.2}")],
            ),
            Problem::DoesNotFit { size_mm, plate_mm } => n(
                "chk-fit",
                &[
                    format!("{:.1} x {:.1} x {:.1}", size_mm[0], size_mm[1], size_mm[2]),
                    format!(
                        "{:.0} x {:.0} x {:.0}",
                        plate_mm[0], plate_mm[1], plate_mm[2]
                    ),
                ],
            ),
        }
    }
}

/// Print-readiness report of one mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrintCheck {
    /// Triangles.
    pub triangles: usize,
    /// Vertices after welding.
    pub vertices: usize,
    /// Edges used by one triangle only.
    pub boundary_edges: usize,
    /// Edges used by more than two triangles.
    pub nonmanifold_edges: usize,
    /// Edges whose two triangles are oriented the same way.
    pub flipped_edges: usize,
    /// Triangles with (almost) no area.
    pub degenerate: usize,
    /// Connected pieces.
    pub pieces: usize,
    /// Holes through the solid (`None` when the surface is not closed).
    pub genus: Option<i64>,
    /// Pairs of crossing triangles (`None` when not counted).
    pub self_intersections: Option<usize>,
    /// The first pairs of crossing triangles (face indices of the input).
    pub intersecting_pairs: Vec<[u32; 2]>,
    /// Enclosed volume (mm3, negative when the normals point inwards).
    pub volume_mm3: f64,
    /// Corner of the bounding box with the smallest coordinates.
    pub bounds_min: [f64; 3],
    /// Corner of the bounding box with the largest coordinates.
    pub bounds_max: [f64; 3],
    /// Size of the bounding box (mm).
    pub size_mm: [f64; 3],
    /// The part fits the build plate, in any of its two orientations about
    /// the vertical (`None` when not checked).
    pub fits_plate: Option<bool>,
    /// Thinnest wall found (mm; `None` when not measured).
    pub min_thickness_mm: Option<f64>,
    /// Nothing prevents printing it.
    pub ok: bool,
    /// What does.
    pub problems: Vec<Problem>,
}

impl PrintCheck {
    /// Volume in millilitres.
    pub fn volume_ml(&self) -> f64 {
        self.volume_mm3 / 1000.0
    }

    /// The problems in words.
    pub fn problem_texts(&self, lang: Lang) -> Vec<String> {
        self.problems.iter().map(|p| p.text(lang)).collect()
    }
}

/// The mesh with identical positions welded: `(positions, triangles)`.
fn weld(mesh: &TriMesh) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut index: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.positions.len());
    let mut positions: Vec<Vec3> = Vec::new();
    let map: Vec<u32> = mesh
        .positions
        .iter()
        .map(|p| {
            // + 0.0 turns -0.0 into 0.0
            let key = [
                (p.x + 0.0).to_bits(),
                (p.y + 0.0).to_bits(),
                (p.z + 0.0).to_bits(),
            ];
            *index.entry(key).or_insert_with(|| {
                positions.push(*p);
                (positions.len() - 1) as u32
            })
        })
        .collect();
    let tris = mesh
        .tris
        .iter()
        .map(|t| t.map(|i| map.get(i as usize).copied().unwrap_or(u32::MAX)))
        .collect();
    (positions, tris)
}

fn corners(pos: &[Vec3], t: [u32; 3]) -> [DVec3; 3] {
    t.map(|i| pos[i as usize].as_dvec3())
}

/// Twice the area of a triangle.
fn area2(c: &[DVec3; 3]) -> f64 {
    (c[1] - c[0]).cross(c[2] - c[0]).length()
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Checks `mesh`.
pub fn check(mesh: &TriMesh, options: &CheckOptions) -> PrintCheck {
    let (pos, tris) = weld(mesh);
    let in_range = |t: &[u32; 3]| t.iter().all(|&i| (i as usize) < pos.len());
    let mut out = PrintCheck {
        triangles: mesh.tris.len(),
        vertices: pos.len(),
        boundary_edges: 0,
        nonmanifold_edges: 0,
        flipped_edges: 0,
        degenerate: 0,
        pieces: 0,
        genus: None,
        self_intersections: None,
        intersecting_pairs: Vec::new(),
        volume_mm3: 0.0,
        bounds_min: [0.0; 3],
        bounds_max: [0.0; 3],
        size_mm: [0.0; 3],
        fits_plate: None,
        min_thickness_mm: None,
        ok: false,
        problems: Vec::new(),
    };
    if tris.is_empty() || !tris.iter().all(in_range) {
        out.problems.push(Problem::Empty);
        return out;
    }

    // bounds
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for t in &tris {
        for p in corners(&pos, *t) {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    out.bounds_min = lo.to_array();
    out.bounds_max = hi.to_array();
    out.size_mm = (hi - lo).to_array();

    // edges
    let mut undirected: HashMap<(u32, u32), u32> = HashMap::with_capacity(tris.len() * 2);
    let mut directed: HashMap<(u32, u32), u32> = HashMap::with_capacity(tris.len() * 3);
    let mut degenerate = 0;
    for t in &tris {
        if t[0] == t[1]
            || t[1] == t[2]
            || t[0] == t[2]
            || area2(&corners(&pos, *t)) < DEGENERATE_AREA2
        {
            degenerate += 1;
        }
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if a == b {
                continue;
            }
            *directed.entry((a, b)).or_insert(0) += 1;
            *undirected.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    out.degenerate = degenerate;
    out.boundary_edges = undirected.values().filter(|&&c| c == 1).count();
    out.nonmanifold_edges = undirected.values().filter(|&&c| c > 2).count();
    out.flipped_edges = directed.values().filter(|&&c| c > 1).count();

    // pieces: triangles that share a vertex
    let mut parent: Vec<u32> = (0..pos.len() as u32).collect();
    let mut used = vec![false; pos.len()];
    for t in &tris {
        for &i in t {
            used[i as usize] = true;
        }
        let (a, b, c) = (
            find(&mut parent, t[0]),
            find(&mut parent, t[1]),
            find(&mut parent, t[2]),
        );
        parent[b as usize] = a;
        let c = find(&mut parent, c);
        let a = find(&mut parent, a);
        parent[c as usize] = a;
    }
    let mut roots: Vec<u32> = (0..pos.len() as u32)
        .filter(|&i| used[i as usize])
        .map(|i| find(&mut parent, i))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    out.pieces = roots.len();

    // genus of a closed manifold surface: handles = pieces - chi / 2
    if out.boundary_edges == 0 && out.nonmanifold_edges == 0 {
        let v = used.iter().filter(|u| **u).count() as f64;
        let chi = v - undirected.len() as f64 + tris.len() as f64;
        out.genus = Some((out.pieces as f64 - chi / 2.0).round() as i64);
    }

    // volume, around the first vertex to keep the sums small
    let origin = pos[tris[0][0] as usize].as_dvec3();
    out.volume_mm3 = tris
        .iter()
        .map(|t| {
            let c = corners(&pos, *t).map(|p| p - origin);
            c[0].dot(c[1].cross(c[2])) / 6.0
        })
        .sum();

    let welded = TriMesh::new(pos.clone(), tris.clone());
    let bvh = Bvh::build(&welded).ok();
    if options.self_intersections {
        let (count, pairs) = bvh
            .as_ref()
            .map_or((0, Vec::new()), |b| self_intersections(&pos, &tris, b));
        out.self_intersections = Some(count);
        out.intersecting_pairs = pairs;
    }

    let closed = out.boundary_edges == 0 && out.nonmanifold_edges == 0;
    if let (Some(_), true, true, Some(b)) = (
        options.min_thickness_mm,
        closed,
        out.volume_mm3 > 0.0,
        bvh.as_ref(),
    ) {
        out.min_thickness_mm = min_thickness(&pos, &tris, b, options.thickness_samples);
    }

    if let Some(plate) = options.build_plate_mm {
        let s = out.size_mm;
        let flat = |a: f64, b: f64| s[0] <= a && s[1] <= b;
        out.fits_plate =
            Some((flat(plate[0], plate[1]) || flat(plate[1], plate[0])) && s[2] <= plate[2]);
    }

    // verdict
    let p = &mut out.problems;
    if out.boundary_edges > 0 {
        p.push(Problem::BoundaryEdges {
            count: out.boundary_edges,
        });
    }
    if out.nonmanifold_edges > 0 {
        p.push(Problem::NonManifoldEdges {
            count: out.nonmanifold_edges,
        });
    }
    if out.flipped_edges > 0 {
        p.push(Problem::FlippedEdges {
            count: out.flipped_edges,
        });
    }
    if out.degenerate > 0 {
        p.push(Problem::Degenerate {
            count: out.degenerate,
        });
    }
    if out.pieces != 1 {
        p.push(Problem::Pieces { count: out.pieces });
    }
    if let Some(n) = out.self_intersections.filter(|&n| n > 0) {
        p.push(Problem::SelfIntersections { count: n });
    }
    if let (Some(max), Some(g)) = (options.max_genus, out.genus)
        && g > i64::from(max)
    {
        p.push(Problem::Genus { count: g });
    }
    if out.volume_mm3 <= 0.0 {
        p.push(Problem::NonPositiveVolume);
    }
    if let (Some(limit), Some(min)) = (options.min_thickness_mm, out.min_thickness_mm)
        && min < limit
    {
        p.push(Problem::TooThin {
            min_mm: min,
            limit_mm: limit,
        });
    }
    if let (Some(plate), Some(false)) = (options.build_plate_mm, out.fits_plate) {
        p.push(Problem::DoesNotFit {
            size_mm: out.size_mm,
            plate_mm: plate,
        });
    }
    out.ok = out.problems.is_empty();
    out
}

/// Checks the STL file at `path`.
pub fn check_file(
    path: impl AsRef<std::path::Path>,
    options: &CheckOptions,
) -> Result<PrintCheck, dc_mesh::MeshError> {
    let mesh = dc_mesh::io::read(path)?;
    Ok(check(&mesh, options))
}

// ---- self-intersections ---------------------------------------------------------------------------------------

/// Number of pairs of triangles that intersect and share no vertex, and the
/// first few of them. Indices are those of the welded mesh, which keeps the
/// triangle order of the input.
fn self_intersections(pos: &[Vec3], tris: &[[u32; 3]], bvh: &Bvh) -> (usize, Vec<[u32; 2]>) {
    let per_face: Vec<Vec<u32>> = (0..tris.len())
        .into_par_iter()
        .map(|i| {
            let t = tris[i];
            let c = corners(pos, t);
            if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] || area2(&c) < DEGENERATE_AREA2 {
                return Vec::new();
            }
            let lo = pos[t[0] as usize]
                .min(pos[t[1] as usize])
                .min(pos[t[2] as usize]);
            let hi = pos[t[0] as usize]
                .max(pos[t[1] as usize])
                .max(pos[t[2] as usize]);
            bvh.overlap_aabb(lo, hi)
                .into_iter()
                .filter(|&j| j as usize > i)
                .filter(|&j| {
                    let u = tris[j as usize];
                    !t.iter().any(|v| u.contains(v))
                })
                .filter(|&j| {
                    let u = tris[j as usize];
                    let d = corners(pos, u);
                    area2(&d) >= DEGENERATE_AREA2 && tri_tri(&c, &d)
                })
                .collect()
        })
        .collect();
    let count = per_face.iter().map(Vec::len).sum();
    let pairs = per_face
        .iter()
        .enumerate()
        .flat_map(|(i, js)| js.iter().map(move |&j| [i as u32, j]))
        .take(MAX_PAIRS)
        .collect();
    (count, pairs)
}

/// Signed distances of the corners of `a` from the plane of `b`, with the
/// ones within [`PLANE_EPS`] set to zero.
fn plane_distances(a: &[DVec3; 3], b: &[DVec3; 3]) -> Option<(DVec3, [f64; 3])> {
    let n = (b[1] - b[0]).cross(b[2] - b[0]);
    let len = n.length();
    if len == 0.0 {
        return None;
    }
    let n = n / len;
    let d = a.map(|p| {
        let v = n.dot(p - b[0]);
        if v.abs() < PLANE_EPS { 0.0 } else { v }
    });
    Some((n, d))
}

/// Exact intersection test of two triangles (Moller 1997, with the coplanar
/// case in 2D). Touching counts as intersecting.
fn tri_tri(a: &[DVec3; 3], b: &[DVec3; 3]) -> bool {
    let Some((nb, da)) = plane_distances(a, b) else {
        return false;
    };
    if da.iter().all(|&d| d > 0.0) || da.iter().all(|&d| d < 0.0) {
        return false;
    }
    let Some((na, db)) = plane_distances(b, a) else {
        return false;
    };
    if db.iter().all(|&d| d > 0.0) || db.iter().all(|&d| d < 0.0) {
        return false;
    }
    if da.iter().all(|&d| d == 0.0) {
        return coplanar(a, b, na);
    }
    // both triangles cross the line of the two planes: compare the intervals
    let dir = na.cross(nb);
    let axis = if dir.x.abs() >= dir.y.abs() && dir.x.abs() >= dir.z.abs() {
        0
    } else if dir.y.abs() >= dir.z.abs() {
        1
    } else {
        2
    };
    let (Some(ia), Some(ib)) = (
        interval(a.map(|p| p[axis]), da),
        interval(b.map(|p| p[axis]), db),
    ) else {
        return false;
    };
    ia.0 <= ib.1 && ib.0 <= ia.1
}

/// Interval of the line covered by a triangle crossing the plane; `p` are
/// the corners projected on the line, `d` their distances from the plane.
fn interval(p: [f64; 3], d: [f64; 3]) -> Option<(f64, f64)> {
    let (i0, i1, i2) = if d[0] * d[1] > 0.0 {
        (2, 0, 1)
    } else if d[0] * d[2] > 0.0 {
        (1, 0, 2)
    } else if d[1] * d[2] > 0.0 || d[0] != 0.0 {
        (0, 1, 2)
    } else if d[1] != 0.0 {
        (1, 0, 2)
    } else if d[2] != 0.0 {
        (2, 0, 1)
    } else {
        return None;
    };
    let at = |j: usize| p[i0] + (p[j] - p[i0]) * d[i0] / (d[i0] - d[j]);
    let (t1, t2) = (at(i1), at(i2));
    Some((t1.min(t2), t1.max(t2)))
}

/// Two coplanar triangles intersect: an edge crosses an edge, or one holds a
/// corner of the other.
fn coplanar(a: &[DVec3; 3], b: &[DVec3; 3], n: DVec3) -> bool {
    // drop the dominant axis of the normal
    let drop = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() {
        0
    } else if n.y.abs() >= n.z.abs() {
        1
    } else {
        2
    };
    let flat = |p: DVec3| match drop {
        0 => DVec2::new(p.y, p.z),
        1 => DVec2::new(p.x, p.z),
        _ => DVec2::new(p.x, p.y),
    };
    let (a, b) = (a.map(flat), b.map(flat));
    for i in 0..3 {
        for j in 0..3 {
            if segments_cross(a[i], a[(i + 1) % 3], b[j], b[(j + 1) % 3]) {
                return true;
            }
        }
    }
    inside(&b, a[0]) || inside(&a, b[0])
}

fn orient(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    (b - a).perp_dot(c - a)
}

fn segments_cross(p: DVec2, q: DVec2, r: DVec2, s: DVec2) -> bool {
    let (o1, o2, o3, o4) = (
        orient(p, q, r),
        orient(p, q, s),
        orient(r, s, p),
        orient(r, s, q),
    );
    o1 * o2 < 0.0 && o3 * o4 < 0.0
}

fn inside(t: &[DVec2; 3], p: DVec2) -> bool {
    let (d1, d2, d3) = (
        orient(t[0], t[1], p),
        orient(t[1], t[2], p),
        orient(t[2], t[0], p),
    );
    (d1 > 0.0 && d2 > 0.0 && d3 > 0.0) || (d1 < 0.0 && d2 < 0.0 && d3 < 0.0)
}

// ---- thickness -----------------------------------------------------------------------------------------------

/// Thinnest wall: from the centre of an even sample of the faces, the
/// distance to the surface straight into the solid (the normals point out).
fn min_thickness(pos: &[Vec3], tris: &[[u32; 3]], bvh: &Bvh, samples: usize) -> Option<f64> {
    let step = (tris.len() / samples.max(1)).max(1);
    (0..tris.len())
        .step_by(step)
        .collect::<Vec<_>>()
        .par_iter()
        .filter_map(|&i| {
            let [a, b, c] = tris[i].map(|v| pos[v as usize]);
            let n = (b - a).cross(c - a);
            let len = n.length();
            if len < 1e-9 {
                return None;
            }
            let n = n / len;
            let centre = (a + b + c) / 3.0;
            let hit = bvh.ray_cast_range(centre - n * RAY_EPS, -n, 0.0, f32::INFINITY)?;
            (hit.face as usize != i).then_some(f64::from(hit.t + RAY_EPS))
        })
        .reduce_with(f64::min)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A closed cube of side `s` at `o`, outward normals.
    pub(crate) fn cube(o: Vec3, s: f32) -> TriMesh {
        let p: Vec<Vec3> = [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (1.0, 1.0, 1.0),
            (0.0, 1.0, 1.0),
        ]
        .iter()
        .map(|&(x, y, z)| o + Vec3::new(x, y, z) * s)
        .collect();
        let t = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        TriMesh::new(p, t)
    }

    /// A box with a cubic cavity: a closed shell of the given wall.
    pub(crate) fn hollow(outer: f32, wall: f32) -> TriMesh {
        let a = cube(Vec3::ZERO, outer);
        let mut b = cube(Vec3::splat(wall), outer - 2.0 * wall);
        for t in &mut b.tris {
            t.swap(1, 2);
        }
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        TriMesh::new(pos, tris)
    }

    fn opts() -> CheckOptions {
        CheckOptions::default()
    }

    #[test]
    fn a_cube_is_ready_to_print() {
        let c = check(&cube(Vec3::ZERO, 10.0), &opts());
        assert!(c.ok, "{:?}", c.problems);
        assert_eq!((c.vertices, c.triangles, c.pieces), (8, 12, 1));
        assert_eq!(c.genus, Some(0));
        assert_eq!(c.self_intersections, Some(0));
        assert!((c.volume_mm3 - 1000.0).abs() < 1e-6);
        assert!((c.volume_ml() - 1.0).abs() < 1e-9);
        assert_eq!(c.size_mm, [10.0; 3]);
        assert_eq!(c.fits_plate, Some(true));
        assert!(c.problem_texts(Lang::En).is_empty());
    }

    #[test]
    fn unwelded_soups_are_welded_first() {
        let c = cube(Vec3::ZERO, 10.0);
        let soup = TriMesh::new(
            c.tris
                .iter()
                .flat_map(|t| t.map(|i| c.positions[i as usize]))
                .collect(),
            (0..12).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
        );
        let r = check(&soup, &opts());
        assert!(r.ok, "{:?}", r.problems);
        assert_eq!(r.vertices, 8);
    }

    #[test]
    fn a_hole_is_flagged() {
        let mut m = cube(Vec3::ZERO, 10.0);
        m.tris.pop();
        let c = check(&m, &opts());
        assert!(!c.ok);
        assert!(
            c.problems
                .iter()
                .any(|p| matches!(p, Problem::BoundaryEdges { count: 3 }))
        );
        assert_eq!(c.genus, None);
        assert!(
            c.problem_texts(Lang::It)
                .iter()
                .any(|t| t.contains("aperti"))
        );
    }

    #[test]
    fn inverted_normals_are_flagged() {
        let mut m = cube(Vec3::ZERO, 10.0);
        for t in &mut m.tris {
            t.swap(1, 2);
        }
        let c = check(&m, &opts());
        assert!(c.problems.contains(&Problem::NonPositiveVolume));
        assert!(c.volume_mm3 < 0.0);
        // one flipped triangle: an inconsistent orientation
        let mut m = cube(Vec3::ZERO, 10.0);
        m.tris[0].swap(1, 2);
        let c = check(&m, &opts());
        assert!(c.flipped_edges > 0 && !c.ok);
    }

    #[test]
    fn crossing_solids_are_flagged() {
        // two overlapping cubes in one mesh
        let a = cube(Vec3::ZERO, 10.0);
        let b = cube(Vec3::new(5.0, 5.0, 5.0), 10.0);
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        let c = check(&TriMesh::new(pos, tris), &opts());
        assert!(c.self_intersections.unwrap() > 0);
        assert!(!c.intersecting_pairs.is_empty());
        assert_eq!(c.pieces, 2);
        assert!(!c.ok);
        // two separate cubes: two pieces but no crossing
        let a = cube(Vec3::ZERO, 10.0);
        let b = cube(Vec3::new(20.0, 0.0, 0.0), 10.0);
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        let c = check(&TriMesh::new(pos, tris), &opts());
        assert_eq!(c.self_intersections, Some(0));
        assert_eq!(c.pieces, 2);
    }

    #[test]
    fn a_cavity_is_not_a_hole_but_a_tunnel_is() {
        let c = check(&hollow(10.0, 2.0), &opts());
        // a cavity is a second piece of surface, not a handle
        assert_eq!(c.pieces, 2);
        assert_eq!(c.genus, Some(0));
        assert!((c.volume_mm3 - (1000.0 - 216.0)).abs() < 1e-6);
    }

    #[test]
    fn the_wall_is_measured_and_compared_with_the_minimum() {
        let shell = hollow(10.0, 0.5);
        let thin = check(
            &shell,
            &CheckOptions {
                min_thickness_mm: Some(1.0),
                ..opts()
            },
        );
        let t = thin.min_thickness_mm.unwrap();
        assert!((t - 0.5).abs() < 1e-3, "{t}");
        assert!(
            thin.problems
                .iter()
                .any(|p| matches!(p, Problem::TooThin { .. }))
        );
        let fine = check(
            &hollow(10.0, 2.0),
            &CheckOptions {
                min_thickness_mm: Some(1.0),
                ..opts()
            },
        );
        assert!(
            !fine
                .problems
                .iter()
                .any(|p| matches!(p, Problem::TooThin { .. })),
            "{:?}",
            fine.problems
        );
        assert!((fine.min_thickness_mm.unwrap() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn the_build_plate_limits_the_size() {
        let c = check(
            &cube(Vec3::ZERO, 100.0),
            &CheckOptions {
                build_plate_mm: Some([120.0, 90.0, 200.0]),
                ..opts()
            },
        );
        // 100 x 100 does not fit 120 x 90 in either orientation
        assert_eq!(c.fits_plate, Some(false));
        assert!(
            c.problems
                .iter()
                .any(|p| matches!(p, Problem::DoesNotFit { .. }))
        );
        let c = check(
            &cube(Vec3::ZERO, 100.0),
            &CheckOptions {
                build_plate_mm: None,
                ..opts()
            },
        );
        assert_eq!(c.fits_plate, None);
        assert!(c.ok);
    }

    #[test]
    fn degenerate_and_empty_meshes_are_flagged() {
        let c = check(&TriMesh::new(Vec::new(), Vec::new()), &opts());
        assert_eq!(c.problems, vec![Problem::Empty]);
        let mut m = cube(Vec3::ZERO, 10.0);
        m.positions.push(Vec3::new(20.0, 0.0, 0.0));
        m.positions.push(Vec3::new(30.0, 0.0, 0.0));
        m.positions.push(Vec3::new(40.0, 0.0, 0.0));
        m.tris.push([8, 9, 10]);
        let c = check(&m, &opts());
        assert_eq!(c.degenerate, 1);
        assert!(!c.ok);
    }

    #[test]
    fn the_triangle_test_handles_the_cases() {
        let t = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| [a, b, c].map(DVec3::from_array);
        let base = t([0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]);
        // pierced by a triangle standing on it
        assert!(tri_tri(
            &base,
            &t([1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0])
        ));
        // above it
        assert!(!tri_tri(
            &base,
            &t([1.0, 1.0, 1.0], [1.0, 1.0, 2.0], [2.0, 1.0, 2.0])
        ));
        // coplanar: overlapping and apart, nested
        assert!(tri_tri(
            &base,
            &t([1.0, 1.0, 0.0], [5.0, 1.0, 0.0], [1.0, 5.0, 0.0])
        ));
        assert!(!tri_tri(
            &base,
            &t([5.0, 5.0, 0.0], [6.0, 5.0, 0.0], [5.0, 6.0, 0.0])
        ));
        assert!(tri_tri(
            &base,
            &t([0.5, 0.5, 0.0], [1.0, 0.5, 0.0], [0.5, 1.0, 0.0])
        ));
        // in the plane of the other but past its edge
        assert!(!tri_tri(
            &base,
            &t([3.0, 3.0, -1.0], [3.0, 3.0, 1.0], [4.0, 3.0, 1.0])
        ));
    }
}
