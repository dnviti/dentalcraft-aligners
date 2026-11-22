//! Manual tools: the geometry behind the label brush, the crown fill, the
//! clean-up of the labels, the region check before a tooth is rebuilt, the
//! margin line of a tooth region and the clinical adjustment of a tooth with
//! collision feedback.
//!
//! Reference: `ortho_aligner/manual.py` (`PaintSession.brush`, `stroke`,
//! `fill_region`, `check_region`, `clean_labels`) and
//! `ortho_aligner/case_plan.py` (`adjust_tooth`). The commands that apply
//! them to a case are in `dca-module` (`manual/`).
//!
//! Everything works on a labelled scan in one coordinate system (the case
//! coordinates for the brush, the arch coordinates, `+Z` occlusal, for the
//! fill and the region check). Labels are FDI numbers, `0` for the gingiva.

mod clean;
mod fill;
mod margin;
mod region;

use dc_case::Pose;
use dc_geom::knn::KnnIndex;
use glam::{DQuat, DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use clean::{CleanReport, clean_labels};
pub use fill::{FillParams, fill_region};
pub use margin::{MarginLine, Polyline, margin_line};
pub use region::{RegionIssue, RegionReport, check_region};

/// A vertex is painted when `cos(normal, view)` is below this: the surface
/// faces the viewer (`manual.FRONT_LIMIT`).
pub const FRONT_LIMIT: f64 = 0.35;

/// Errors of the manual tools.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ManualError {
    /// The input is not usable (empty scan, index out of range...).
    #[error("{0}")]
    Invalid(String),
}

/// Vertex neighbours of a triangle mesh (compressed rows, each list sorted
/// and without duplicates).
#[derive(Debug, Clone)]
pub struct Adjacency {
    indptr: Vec<usize>,
    indices: Vec<u32>,
}

impl Adjacency {
    /// The adjacency of `n` vertices connected by `tris`.
    pub fn new(n: usize, tris: &[[u32; 3]]) -> Self {
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); n];
        for t in tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                lists[a as usize].push(b);
                lists[b as usize].push(a);
            }
        }
        let mut indptr = Vec::with_capacity(n + 1);
        let mut indices = Vec::new();
        indptr.push(0);
        for l in &mut lists {
            l.sort_unstable();
            l.dedup();
            indices.extend_from_slice(l);
            indptr.push(indices.len());
        }
        Self { indptr, indices }
    }

    /// Number of vertices.
    pub fn len(&self) -> usize {
        self.indptr.len() - 1
    }

    /// `true` without vertices.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The neighbours of vertex `v`.
    pub fn neighbors(&self, v: u32) -> &[u32] {
        &self.indices[self.indptr[v as usize]..self.indptr[v as usize + 1]]
    }

    /// Each undirected edge once (`a < b`).
    pub fn edges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.len() as u32)
            .flat_map(move |a| self.neighbors(a).iter().map(move |&b| (a, b)))
            .filter(|&(a, b)| a < b)
    }
}

/// Area-weighted vertex normals (zero for an isolated vertex).
pub fn vertex_normals(vertices: &[DVec3], tris: &[[u32; 3]]) -> Vec<DVec3> {
    let mut n = vec![DVec3::ZERO; vertices.len()];
    for t in tris {
        let [a, b, c] = t.map(|i| vertices[i as usize]);
        let w = (b - a).cross(c - a); // twice the area times the unit normal
        for &i in t {
            n[i as usize] += w;
        }
    }
    n.into_iter().map(DVec3::normalize_or_zero).collect()
}

/// A scan prepared for the brush: the vertex neighbours, the normals and a
/// spatial index.
pub struct Scan<'a> {
    /// Vertices.
    pub vertices: &'a [DVec3],
    /// Triangles.
    pub tris: &'a [[u32; 3]],
    adj: Adjacency,
    normals: Vec<DVec3>,
    index: KnnIndex,
}

impl<'a> Scan<'a> {
    /// Prepares `vertices` and `tris` (indices checked).
    pub fn new(vertices: &'a [DVec3], tris: &'a [[u32; 3]]) -> Result<Self, ManualError> {
        if vertices.is_empty() {
            return Err(ManualError::Invalid("the scan has no vertices".to_owned()));
        }
        if tris.iter().flatten().any(|&i| i as usize >= vertices.len()) {
            return Err(ManualError::Invalid(
                "a triangle refers to a missing vertex".to_owned(),
            ));
        }
        let points: Vec<Vec3> = vertices.iter().map(|p| p.as_vec3()).collect();
        let index =
            KnnIndex::build(&points).map_err(|e| ManualError::Invalid(format!("scan: {e}")))?;
        Ok(Self {
            vertices,
            tris,
            adj: Adjacency::new(vertices.len(), tris),
            normals: vertex_normals(vertices, tris),
            index,
        })
    }

    /// The vertex neighbours.
    pub fn adjacency(&self) -> &Adjacency {
        &self.adj
    }

    /// Unit normal of vertex `v`.
    pub fn normal(&self, v: u32) -> DVec3 {
        self.normals[v as usize]
    }

    /// The vertices within `radius` of `centre`, with their distance.
    pub fn within(&self, centre: DVec3, radius: f64) -> Vec<(u32, f64)> {
        self.index
            .within_radius(centre.as_vec3(), radius as f32)
            .into_iter()
            .map(|n| (n.index, f64::from(n.distance)))
            .collect()
    }

    /// The vertex nearest to `p`.
    pub fn nearest(&self, p: DVec3) -> Option<u32> {
        self.index.nearest(p.as_vec3()).map(|n| n.index)
    }

    /// Vertices under a brush of `radius` at `centre`: within the radius,
    /// facing the viewer (when `view_dir`, the direction the viewer looks
    /// along, is given) and connected to the vertex nearest to the centre
    /// (`PaintSession.brush`). Sorted.
    pub fn brush(&self, centre: DVec3, radius: f64, view_dir: Option<DVec3>) -> Vec<u32> {
        let mut found = self.within(centre, radius);
        if found.is_empty() {
            return Vec::new();
        }
        if let Some(d) = view_dir.map(DVec3::normalize_or_zero)
            && d != DVec3::ZERO
        {
            // the orientation of the normals: the surface under the brush faces the viewer
            let near = found
                .iter()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(0.0, |&(i, _)| self.normals[i as usize].dot(d));
            let sgn = if near > 0.0 { -1.0 } else { 1.0 };
            found.retain(|&(i, _)| sgn * self.normals[i as usize].dot(d) < FRONT_LIMIT);
            if found.is_empty() {
                return Vec::new();
            }
        }
        let start = found
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |&(i, _)| i);
        let mut inside = vec![false; self.vertices.len()];
        for &(i, _) in &found {
            inside[i as usize] = true;
        }
        let mut seen = vec![false; self.vertices.len()];
        seen[start as usize] = true;
        let mut stack = vec![start];
        let mut out = Vec::new();
        while let Some(u) = stack.pop() {
            out.push(u);
            for &v in self.adj.neighbors(u) {
                if inside[v as usize] && !seen[v as usize] {
                    seen[v as usize] = true;
                    stack.push(v);
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// The union of the brush at every point of a stroke: dabs from one
    /// point to the next every half radius, so a fast stroke leaves no gaps
    /// (`PaintSession.stroke`). Sorted.
    pub fn stroke(&self, points: &[DVec3], radius: f64, view_dir: Option<DVec3>) -> Vec<u32> {
        let mut hit = vec![false; self.vertices.len()];
        let mut mark = |idx: Vec<u32>| {
            for i in idx {
                hit[i as usize] = true;
            }
        };
        let Some(&first) = points.first() else {
            return Vec::new();
        };
        mark(self.brush(first, radius, view_dir));
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let n = ((b - a).length() / (0.5 * radius).max(1e-6))
                .ceil()
                .max(1.0) as usize;
            for k in 1..=n {
                mark(self.brush(a + (b - a) * (k as f64 / n as f64), radius, view_dir));
            }
        }
        (0..hit.len() as u32).filter(|&i| hit[i as usize]).collect()
    }
}

// ---- adjusting a tooth by hand ------------------------------------------------------------------------------

/// A movement in clinical terms (`case_plan.adjust_tooth`): mesial(+),
/// buccal(+) and extrusion(+) in mm, crown torque buccal(+), crown tip
/// mesial(+) and mesio-buccal rotation(+) in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClinicalDelta {
    /// Mesial (+) or distal (-) translation (mm).
    pub mesial_mm: f64,
    /// Buccal (+) or lingual (-) translation (mm).
    pub buccal_mm: f64,
    /// Extrusion (+) or intrusion (-) (mm).
    pub extrusion_mm: f64,
    /// Crown torque, buccal (+) (degrees).
    pub torque_deg: f64,
    /// Crown tip, mesial (+) (degrees).
    pub tip_deg: f64,
    /// Rotation about the long axis, mesio-buccal (+) (degrees).
    pub rotation_deg: f64,
}

impl ClinicalDelta {
    /// `true` when every component is zero.
    pub fn is_zero(&self) -> bool {
        [
            self.mesial_mm,
            self.buccal_mm,
            self.extrusion_mm,
            self.torque_deg,
            self.tip_deg,
            self.rotation_deg,
        ]
        .iter()
        .all(|v| v.abs() < 1e-12)
    }

    /// The movement as a pose in the frame of the tooth (translation, then
    /// the rotations in the order of the plugin: torque, tip and rotation
    /// applied one after the other, each in the frame the previous left).
    /// `md_sign` is `+1` when the tooth's x axis points mesially, `-1`
    /// distally.
    pub fn pose(&self, md_sign: i8) -> Pose {
        let md = f64::from(md_sign);
        let step = |t: DVec3, q: DQuat| Pose::new(t, q);
        // each component is an increment in the current frame of the tooth
        let mut p = Pose::IDENTITY;
        p = p.compose(&step(
            DVec3::new(md * self.mesial_mm, self.buccal_mm, self.extrusion_mm),
            DQuat::IDENTITY,
        ));
        p = p.compose(&step(
            DVec3::ZERO,
            DQuat::from_axis_angle(DVec3::X, -self.torque_deg.to_radians()),
        ));
        p = p.compose(&step(
            DVec3::ZERO,
            DQuat::from_axis_angle(DVec3::Y, self.tip_deg.to_radians() * md),
        ));
        p.compose(&step(
            DVec3::ZERO,
            DQuat::from_axis_angle(DVec3::Z, self.rotation_deg.to_radians() * md),
        ))
    }
}

/// Clearance between two crowns along the horizontal direction joining their
/// centroids, in arch coordinates (`+Z` occlusal): the minimum over a grid on
/// the plane orthogonal to that direction of `min_b u - max_a u`, using the
/// facing halves of the crowns. Negative is interpenetration; `None` when
/// the crowns do not face each other. The same metric as the collision
/// check of the staging.
pub fn facing_clearance(pa: &[DVec3], pb: &[DVec3]) -> Option<f64> {
    use std::collections::HashMap;
    const CELL: f64 = 0.35;
    if pa.is_empty() || pb.is_empty() {
        return None;
    }
    let ca = pa.iter().copied().sum::<DVec3>() / pa.len() as f64;
    let cb = pb.iter().copied().sum::<DVec3>() / pb.len() as f64;
    let mut u = cb - ca;
    u.z = 0.0;
    if u.length() < 1e-9 {
        return None;
    }
    let u = u.normalize();
    let up = DVec3::Z;
    let v = (up - u * up.dot(u)).normalize_or_zero();
    let w = u.cross(v);
    let cell = |p: DVec3| -> (i64, i64) {
        (
            (p.dot(v) / CELL).floor() as i64,
            (p.dot(w) / CELL).floor() as i64,
        )
    };
    let mut top_a: HashMap<(i64, i64), f64> = HashMap::new();
    for &p in pa.iter().filter(|&&p| (p - ca).dot(u) > 0.0) {
        let e = top_a.entry(cell(p)).or_insert(f64::NEG_INFINITY);
        *e = e.max(p.dot(u));
    }
    let mut low_b: HashMap<(i64, i64), f64> = HashMap::new();
    for &p in pb.iter().filter(|&&p| (p - cb).dot(u) < 0.0) {
        let e = low_b.entry(cell(p)).or_insert(f64::INFINITY);
        *e = e.min(p.dot(u));
    }
    top_a
        .iter()
        .filter_map(|(k, &a)| low_b.get(k).map(|&b| b - a))
        .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
}

/// How deep two adjacent crowns (arch coordinates) interpenetrate, in mm
/// (`0` when they are apart or do not face each other). Crown points within
/// 0.6 mm of the cervical margin are ignored, as in the staging.
pub fn penetration(pa: &[DVec3], pb: &[DVec3]) -> f64 {
    let upper = |p: &[DVec3]| -> Vec<DVec3> {
        let zmin = p.iter().map(|q| q.z).fold(f64::INFINITY, f64::min);
        p.iter().copied().filter(|q| q.z > zmin + 0.6).collect()
    };
    match facing_clearance(&upper(pa), &upper(pb)) {
        Some(sep) if sep.is_finite() => (-sep).max(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid of `n x n` vertices (1 mm apart) in the XY plane, normals +Z.
    pub(crate) fn grid(n: usize) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let mut v = Vec::new();
        for y in 0..n {
            for x in 0..n {
                v.push(DVec3::new(x as f64, y as f64, 0.0));
            }
        }
        let mut t = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let i = (y * n + x) as u32;
                let n = n as u32;
                t.push([i, i + 1, i + n + 1]);
                t.push([i, i + n + 1, i + n]);
            }
        }
        (v, t)
    }

    /// A gingival plane (40 x 40 mm, 0.4 mm apart) with a tooth-shaped bump of
    /// radius `r_tooth` and height `h` in the middle: a rounded rim and a
    /// vertical wall that meets the gingiva at a sharp concave fold, as a
    /// crown does. `+Z` is up. Returns the distance of every vertex from the
    /// axis of the bump too.
    pub(crate) fn bump(r_tooth: f64, h: f64) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<f64>) {
        let (mut v, t) = grid(101);
        let mut r = Vec::new();
        for p in &mut v {
            p.x *= 0.4;
            p.y *= 0.4;
            let d = ((p.x - 20.0).powi(2) + (p.y - 20.0).powi(2)).sqrt();
            p.z = if d < r_tooth {
                h * (1.0 - (d / r_tooth).powi(6)).sqrt()
            } else {
                0.0
            };
            r.push(d);
        }
        (v, t, r)
    }

    #[test]
    fn adjacency_lists_each_neighbour_once() {
        let (v, t) = grid(3);
        let a = Adjacency::new(v.len(), &t);
        assert_eq!(a.neighbors(4), &[0, 1, 3, 5, 7, 8]);
        assert_eq!(a.edges().count(), 16);
        let n = vertex_normals(&v, &t);
        assert!(n.iter().all(|n| (*n - DVec3::Z).length() < 1e-12));
    }

    #[test]
    fn the_brush_takes_the_vertices_in_the_ball_connected_to_the_centre() {
        let (v, t) = grid(9);
        let scan = Scan::new(&v, &t).unwrap();
        let b = scan.brush(DVec3::new(4.0, 4.0, 0.0), 1.01, None);
        assert_eq!(b.len(), 5);
        assert!(b.contains(&(4 * 9 + 4)));
        // a ball centred off the surface still finds the nearest vertex
        assert!(!scan.brush(DVec3::new(4.0, 4.0, 0.5), 1.2, None).is_empty());
        assert!(scan.brush(DVec3::new(40.0, 4.0, 0.0), 1.0, None).is_empty());
    }

    #[test]
    fn the_brush_follows_the_orientation_of_the_surface_under_it() {
        // a two-sided sheet: a pit-shaped (concave) strip viewed from its inside and from behind
        let (mut v, t) = grid(9);
        for p in &mut v {
            p.z = 0.04 * (p.x - 4.0).powi(2); // a gentle valley along y
        }
        let scan = Scan::new(&v, &t).unwrap();
        let c = DVec3::new(4.0, 4.0, 0.0);
        // the viewer looks along the surface's own side or the opposite one: the
        // orientation is taken from the normal under the brush, so both paint it
        let a = scan.brush(c, 1.01, Some(DVec3::NEG_Z));
        let b = scan.brush(c, 1.01, Some(DVec3::Z));
        assert_eq!(a, b);
        assert_eq!(a.len(), 5);
        // a degenerate view direction means no facing test
        assert_eq!(scan.brush(c, 1.01, Some(DVec3::ZERO)), a);
    }

    #[test]
    fn a_stroke_leaves_no_gaps() {
        let (v, t) = grid(15);
        let scan = Scan::new(&v, &t).unwrap();
        let line = [DVec3::new(2.0, 7.0, 0.0), DVec3::new(12.0, 7.0, 0.0)];
        let s = scan.stroke(&line, 1.01, None);
        for x in 2..=12 {
            assert!(s.contains(&(7 * 15 + x)), "vertex {x} of the line");
        }
        assert!(scan.stroke(&[], 1.0, None).is_empty());
    }

    #[test]
    fn clinical_increments_follow_the_plugin_conventions() {
        let d = ClinicalDelta {
            mesial_mm: 1.0,
            buccal_mm: 0.5,
            extrusion_mm: -0.25,
            ..ClinicalDelta::default()
        };
        // x points distally: a mesial movement is along -x
        let p = d.pose(-1);
        assert!((p.t - DVec3::new(-1.0, 0.5, -0.25)).length() < 1e-12);
        let r = ClinicalDelta {
            rotation_deg: 10.0,
            ..ClinicalDelta::default()
        };
        let (axis, angle) = r.pose(1).q.to_axis_angle();
        assert!((axis - DVec3::Z).length() < 1e-12 && (angle.to_degrees() - 10.0).abs() < 1e-9);
        let (axis, _) = r.pose(-1).q.to_axis_angle();
        assert!((axis + DVec3::Z).length() < 1e-12);
        assert!(ClinicalDelta::default().is_zero() && !d.is_zero());
    }

    #[test]
    fn penetration_measures_the_overlap_of_two_adjacent_crowns() {
        // two 4 x 4 x 6 mm boxes of points along x, 0.5 mm apart or 0.3 mm overlapping
        let boxed = |x0: f64| -> Vec<DVec3> {
            let mut p = Vec::new();
            for i in 0..=8 {
                for j in 0..=8 {
                    for k in 0..=12 {
                        p.push(DVec3::new(
                            x0 + i as f64 * 0.5,
                            j as f64 * 0.5,
                            k as f64 * 0.5,
                        ));
                    }
                }
            }
            p
        };
        let a = boxed(0.0); // x in [0, 4]
        assert_eq!(penetration(&a, &boxed(4.5)), 0.0);
        let pen = penetration(&a, &boxed(3.5));
        assert!((pen - 0.5).abs() < 0.2, "{pen}");
        assert_eq!(penetration(&a, &[]), 0.0);
    }
}
