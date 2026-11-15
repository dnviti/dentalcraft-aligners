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
//! the start of the closed solid ([`Based::scan`], [`open_part`]). Every
//! boundary loop is extruded, holes inside the scan too, as in the plugin.
//! The flat caps are triangulated by ear clipping in the base plane
//! (Blender's "beauty" fill gives other triangles over the same flat area).
//! The new faces follow the winding of the scan; when the closed result has
//! a negative volume every face is turned, the outward orientation
//! `recalc_face_normals` gives. A boundary chain that does not close on
//! itself (around a non-manifold vertex) gets its wall but no cap, as
//! `holes_fill` only fills closed loops.
//!
//! Two deviations from bmesh, which copies every attribute of the source
//! vertex: a point of the base gets the label `0` (gingiva), never a tooth,
//! so no tooth reaches into the base (its colour still follows the source
//! vertex); and the points of the scan keep their normals (turned with the
//! faces), so that the scan as it was read comes back exactly.
//!
//! The import closes every open arch scan before anything is computed on it
//! (the case has no tooth marks yet): its occlusal direction is then the one
//! [`occlusal_direction`] finds without them, where the plugin needs the
//! arch frame of the tooth marks.

use std::collections::HashSet;

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

/// A scan closed by [`make_base`].
#[derive(Debug, Clone, PartialEq)]
pub struct Based {
    /// The closed scan: the vertices of the scan in their order, then the
    /// vertices of the base.
    pub mesh: TriMesh,
    /// Boundary edges closed (the plugin's return value).
    pub closed_edges: usize,
    /// For each vertex of the base, in order, the boundary vertex of the
    /// scan it was extruded from.
    pub sources: Vec<u32>,
    /// Height of the base plane along the occlusal direction (mm).
    pub plane: f64,
    /// Where the scan ends in [`Based::mesh`], for [`open_part`].
    pub scan: ScanBase,
}

/// Closes `mesh` with a flat base perpendicular to `occlusal` (the
/// occlusal direction of the arch, towards the teeth), `height_mm` below
/// its lowest point. `Ok(None)` when the scan has no boundary edge: it is
/// already closed and stays as it is.
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
    if mesh.tris.is_empty() || mesh.positions.iter().any(|p| !p.is_finite()) {
        return Err(BaseError::Mesh);
    }
    let adjacency = mesh.edge_adjacency();
    let closed_edges = adjacency.boundary_edge_count();
    if closed_edges == 0 {
        return Ok(None);
    }
    // the boundary half-edges, in the winding of their face
    let boundary: Vec<(u32, u32)> = mesh
        .tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .filter(|&(a, b)| a != b && adjacency.faces_of(a, b).len() == 1)
        .collect();
    let half_edges: HashSet<(u32, u32)> = boundary.iter().copied().collect();

    let height = |p: Vec3| p.as_dvec3().dot(z);
    let lowest = mesh
        .positions
        .iter()
        .map(|p| height(*p))
        .fold(f64::INFINITY, f64::min);
    let plane = lowest - height_mm;

    // one new vertex per boundary vertex, on the plane
    let n = mesh.positions.len() as u32;
    let mut copy = vec![u32::MAX; mesh.positions.len()];
    let mut sources = Vec::new();
    let mut positions = mesh.positions.clone();
    for &(a, b) in &boundary {
        for v in [a, b] {
            if copy[v as usize] == u32::MAX {
                copy[v as usize] = n + sources.len() as u32;
                sources.push(v);
                let p = mesh.positions[v as usize].as_dvec3();
                positions.push((p - z * (p.dot(z) - plane)).as_vec3());
            }
        }
    }

    // the walls: the quad between each boundary edge a -> b and its copy,
    // with the edge the other way round (b -> a) to follow the winding
    let mut tris = mesh.tris.clone();
    for &(a, b) in &boundary {
        let (a2, b2) = (copy[a as usize], copy[b as usize]);
        tris.push([b, a, a2]);
        tris.push([b, a2, b2]);
    }

    // the caps: each closed loop of the base, the other way round
    let (u, v) = plane_axes(z);
    for lp in adjacency.boundary_loops() {
        let (&first, &last) = (lp.first().expect("a loop"), lp.last().expect("a loop"));
        if lp.len() < 3 || !half_edges.contains(&(last, first)) {
            continue;
        }
        let ring: Vec<u32> = lp.iter().rev().map(|&w| copy[w as usize]).collect();
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
    // the base is gingiva, never a tooth (bmesh would copy the label)
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
        let mut l = labels.clone();
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
    // points of the base get those of the closed solid
    if let Some(normals) = &mesh.normals {
        let all = out.compute_vertex_normals();
        let mut n: Vec<Vec3> = if flipped {
            normals.iter().map(|v| -*v).collect()
        } else {
            normals.clone()
        };
        n.extend_from_slice(&all[mesh.positions.len()..]);
        out.normals = Some(n);
    }
    // who closed it is the caller's to say (the import by default)
    let scan = ScanBase {
        vertices: mesh.positions.len() as u32,
        faces: mesh.tris.len() as u32,
        flipped,
        closed_by: ClosedBy::default(),
    };
    Ok(Some(Based {
        mesh: out,
        closed_edges,
        sources,
        plane,
        scan,
    }))
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
}
