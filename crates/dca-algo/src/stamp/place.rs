//! Where the stamp goes (port of the geometry of `stamp.py`): the reading
//! frame on a tooth, the map of the face where the text fits, the search of
//! the spot and the projection of the glyphs on the surface.
//!
//! Rays go from the crown of the tooth along `n`, the direction from the
//! crown towards the chosen face, over a grid of the face; the surface they
//! hit is the wall of the aligner (shell) or the tooth of the model.

use std::collections::HashMap;

use dc_geom::bvh::{Bvh, RayHit};
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use rayon::prelude::*;

use super::text::TextShape;
use super::{Config, Face, Kind, Read, StampError};

/// Step of the grid of rays on the face (mm).
pub(super) const GRID_MM: f64 = 0.2;
/// Crown points taken as the tooth surface along a ray lie this close to it (mm).
const RAY_RADIUS_MM: f64 = 0.35;
/// The surface hit is at most this far from the crown, beyond the clearance (mm).
const SURFACE_TOL_MM: f64 = 0.6;
/// Smallest cosine between the surface normal and the ray (53 degrees).
const COS_MIN: f64 = 0.6;
/// Distance of the text from the most occlusal part of the crown (mm).
const CUSP_MARGIN_MM: f64 = 0.8;
/// Distance of the text from the mesial and distal sides (contacts, IPR) (mm).
const CONTACT_MARGIN_MM: f64 = 1.0;
/// Half side of the grid of rays (mm).
const HALF_MM: f64 = 6.0;
/// Tilts of the text towards the occlusal side tried in order (degrees): the
/// faces of the molars slope, and with the edge on the equator the covered
/// part is a low strip.
pub(super) const TILTS: [f64; 4] = [0.0, 10.0, 20.0, 30.0];
/// Two hits closer than this along a ray are the same crossing (mm).
const SAME_HIT_MM: f32 = 1e-4;

/// Reading frame of the text: `n` towards the surface, `u` up for the
/// reader (the text's `y`), `r` to the reader's right (the text's `x`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Direction of the rays, from the crown towards the face.
    pub n: DVec3,
    /// Up of the text.
    pub u: DVec3,
    /// Right of the text.
    pub r: DVec3,
}

/// A mesh with its BVH and vertex normals, for the rays.
pub(super) struct Surface<'a> {
    pub mesh: &'a TriMesh,
    bvh: Bvh,
    normals: Vec<Vec3>,
}

impl<'a> Surface<'a> {
    pub fn new(mesh: &'a TriMesh) -> Result<Self, StampError> {
        let bvh = Bvh::build(mesh).map_err(|e| StampError::Invalid(format!("mesh: {e}")))?;
        Ok(Self {
            mesh,
            bvh,
            normals: mesh.compute_vertex_normals(),
        })
    }

    /// Up to `max_hits` successive crossings of each ray `origin + t dir`.
    fn cast(&self, origins: &[DVec3], dir: DVec3, max_hits: usize) -> Vec<Vec<RayHit>> {
        let o: Vec<Vec3> = origins.iter().map(|p| p.as_vec3()).collect();
        let d = vec![dir.as_vec3(); o.len()];
        self.bvh
            .ray_hits_batch(&o, &d, f32::INFINITY, max_hits, SAME_HIT_MM)
    }

    /// Triangles whose box overlaps the box `min`..`max`.
    pub fn faces_in(&self, min: Vec3, max: Vec3) -> Vec<u32> {
        self.bvh.overlap_aabb(min, max)
    }

    /// Distance along `dir` to the first crossing of each ray (`None` if it misses).
    pub fn first_t(&self, origins: &[DVec3], dir: DVec3) -> Vec<Option<f64>> {
        self.cast(origins, dir, 1)
            .iter()
            .map(|h| h.first().map(|h| f64::from(h.t)))
            .collect()
    }

    /// Unit surface normal (outwards from the solid) at a hit.
    fn normal_at(&self, hit: &RayHit) -> DVec3 {
        let t = self.mesh.tris[hit.face as usize];
        let w = hit.barycentric;
        let n = self.normals[t[0] as usize] * w.x
            + self.normals[t[1] as usize] * w.y
            + self.normals[t[2] as usize] * w.z;
        n.as_dvec3().normalize_or_zero()
    }
}

fn unit(v: DVec3) -> DVec3 {
    v.normalize_or_zero()
}

/// Reading frame on a tooth (arch coordinates, occlusal = `+Z`).
///
/// `n` goes from the crown towards the chosen face (lingual / palatal: to
/// the centre of the arch; buccal: away from it), horizontal; `u` towards
/// the aligner edge (the gingiva); `r` is the reader's right, seen from
/// inside the aligner (`Read::Inside`, looking along `+n`) or from outside
/// through the transparent wall (`Read::Outside`, looking along `-n`).
pub fn tooth_frame(
    e_md: DVec3,
    center: DVec3,
    arch_center: DVec3,
    face: Face,
    read: Read,
) -> Frame {
    let z = DVec3::Z;
    let md = unit(e_md - e_md.dot(z) * z);
    let mut bl = unit(z.cross(md));
    let mut to_center = arch_center - center;
    to_center.z = 0.0;
    if bl.dot(to_center) < 0.0 {
        bl = -bl;
    }
    let n = if face == Face::Lingual { bl } else { -bl };
    let u = -z;
    let r = match read {
        Read::Inside => n.cross(u),
        Read::Outside => (-n).cross(u),
    };
    Frame { n, u, r: unit(r) }
}

/// The frame turned about `r` by `deg` degrees towards the occlusal side.
pub(super) fn tilt_frame(frame: &Frame, deg: f64) -> Frame {
    let a = deg.to_radians();
    Frame {
        n: a.cos() * frame.n - a.sin() * frame.u,
        u: a.sin() * frame.n + a.cos() * frame.u,
        r: frame.r,
    }
}

/// Faces to try: the chosen one, then the other (both are the inside of the aligner).
pub(super) fn faces(first: Face) -> [Face; 2] {
    match first {
        Face::Lingual => [Face::Lingual, Face::Buccal],
        Face::Buccal => [Face::Buccal, Face::Lingual],
    }
}

/// For each ray the depth `t` of the crown surface: the farthest point of the
/// crown along `n` among those within `RAY_RADIUS_MM` of the ray (`NAN` when
/// the ray does not pass over the crown).
fn crown_depth(crown: &[DVec3], origins: &[DVec3], n: DVec3) -> Vec<f64> {
    let a0 = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    let p1 = unit(n.cross(a0));
    let p2 = n.cross(p1);
    let cell = |v: f64| (v / RAY_RADIUS_MM).floor() as i64;
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (i, p) in crown.iter().enumerate() {
        grid.entry((cell(p.dot(p1)), cell(p.dot(p2))))
            .or_default()
            .push(i as u32);
    }
    origins
        .par_iter()
        .map(|o| {
            let (ox, oy) = (o.dot(p1), o.dot(p2));
            let (cx, cy) = (cell(ox), cell(oy));
            let mut best = f64::NEG_INFINITY;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(list) = grid.get(&(cx + dx, cy + dy)) else {
                        continue;
                    };
                    for &j in list {
                        let p = crown[j as usize];
                        if (p.dot(p1) - ox).hypot(p.dot(p2) - oy) <= RAY_RADIUS_MM {
                            best = best.max((p - *o).dot(n));
                        }
                    }
                }
            }
            if best.is_finite() { best } else { f64::NAN }
        })
        .collect()
}

/// Where the text can go on one face of a tooth: points of a grid of rays
/// (`x` along `r`, `y` along `u`, from the centre of the crown) that are valid.
#[derive(Debug, Clone)]
pub(super) struct FaceMap {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    /// `valid[iy * xs.len() + ix]`.
    pub valid: Vec<bool>,
    /// Centre of the crown, the origin of the grid.
    pub center: DVec3,
    pub frame: Frame,
    /// Why no point is valid.
    pub reason: Option<String>,
}

fn arange(half: f64, step: f64) -> Vec<f64> {
    let count = ((2.0 * half + 1e-9) / step).ceil() as usize;
    (0..count).map(|k| -half + k as f64 * step).collect()
}

/// The map of the face (see the module docs). A point is valid when
/// - the ray passes over the crown (not above the cusps, not under the
///   margin) and the surface it hits is the one of the crown (within
///   `SURFACE_TOL_MM` beyond the clearance) and not too oblique;
/// - it is `CUSP_MARGIN_MM` from the most occlusal part of the crown and
///   `CONTACT_MARGIN_MM` from the mesial and distal sides;
/// - shell: the wall behind is at least `depth + min_wall` thick along the normal;
/// - model: it lies in the occlusal half of the face (covered by every aligner edge).
pub(super) fn face_map(
    surface: &Surface<'_>,
    crown: &[DVec3],
    frame: Frame,
    kind: Kind,
    cfg: &Config,
) -> FaceMap {
    let Frame { n, u, r } = frame;
    let c = crown.iter().copied().sum::<DVec3>() / crown.len() as f64;
    let xs = arange(HALF_MM, GRID_MM);
    let ys = xs.clone();
    let (nx, ny) = (xs.len(), ys.len());
    let origins: Vec<DVec3> = (0..nx * ny)
        .map(|k| c + xs[k % nx] * r + ys[k / nx] * u)
        .collect();
    let t_c = crown_depth(crown, &origins, n);
    let hits = surface.cast(&origins, n, 2);

    let total = nx * ny;
    let mut t1 = vec![f64::INFINITY; total];
    let mut t2 = vec![f64::INFINITY; total];
    let mut cosv = vec![0.0f64; total];
    for (k, h) in hits.iter().enumerate() {
        if let Some(first) = h.first() {
            t1[k] = f64::from(first.t);
            cosv[k] = surface.normal_at(first).dot(n).abs();
            if let Some(second) = h.get(1) {
                t2[k] = f64::from(second.t);
            }
        }
    }
    let on_crown: Vec<bool> = t_c.iter().map(|t| t.is_finite()).collect();
    let hit: Vec<bool> = t1.iter().map(|t| t.is_finite()).collect();
    let mut valid: Vec<bool> = (0..total).map(|k| on_crown[k] && hit[k]).collect();
    let mut reason =
        (!valid.iter().any(|&v| v)).then(|| "the surface was not found on the crown".to_owned());
    let off = if kind == Kind::Shell {
        cfg.clearance
    } else {
        0.0
    };
    for k in 0..total {
        let gap = if hit[k] && on_crown[k] {
            t1[k] - t_c[k] - off
        } else {
            f64::INFINITY
        };
        valid[k] &= gap.abs() <= SURFACE_TOL_MM && cosv[k] >= COS_MIN;
    }
    if kind == Kind::Shell {
        let thick: Vec<bool> = (0..total)
            .map(|k| {
                t1[k].is_finite()
                    && t2[k].is_finite()
                    && (t2[k] - t1[k]) * cosv[k] >= cfg.depth + cfg.min_wall
            })
            .collect();
        let some_thin = (0..total).any(|k| on_crown[k] && hit[k] && !thick[k]);
        let some_good = (0..total).any(|k| valid[k] && thick[k]);
        if some_thin && !some_good {
            reason = Some(format!(
                "the wall is too thin to engrave {:.2} mm and leave {:.2} mm",
                cfg.depth, cfg.min_wall
            ));
        }
        for k in 0..total {
            valid[k] &= thick[k];
        }
    }
    // limits of the crown on the face: cusps (smallest y: u goes to the gingiva) and sides
    let rel: Vec<DVec3> = crown.iter().map(|p| *p - c).collect();
    let front: Vec<&DVec3> = rel.iter().filter(|p| p.dot(n) > 0.0).collect();
    let ycs: Vec<f64> = if front.is_empty() {
        rel.iter().map(|p| p.dot(u)).collect()
    } else {
        front.iter().map(|p| p.dot(u)).collect()
    };
    let y_top = ycs.iter().copied().fold(f64::INFINITY, f64::min);
    let y_bot = ycs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let x_lo = rel.iter().map(|p| p.dot(r)).fold(f64::INFINITY, f64::min);
    let x_hi = rel
        .iter()
        .map(|p| p.dot(r))
        .fold(f64::NEG_INFINITY, f64::max);
    for k in 0..total {
        let (x, y) = (xs[k % nx], ys[k / nx]);
        valid[k] &= y >= y_top + CUSP_MARGIN_MM
            && x >= x_lo + CONTACT_MARGIN_MM
            && x <= x_hi - CONTACT_MARGIN_MM;
        if kind == Kind::Model {
            valid[k] &= y <= 0.5 * (y_top + y_bot);
        }
    }
    if reason.is_none() && !valid.iter().any(|&v| v) {
        reason = Some("the face of the tooth is too small".to_owned());
    }
    FaceMap {
        xs,
        ys,
        valid,
        center: c,
        frame,
        reason,
    }
}

/// Centre `(x, y)` of the `w` x `h` box of the text, with `margin` around,
/// all on valid points of the map: `x` the closest to the centre of the
/// face, `y` the middle of the longest valid stretch. `None` if it does not fit.
pub(super) fn place(fm: &FaceMap, w: f64, h: f64, margin: f64) -> Option<(f64, f64)> {
    let (nx, ny) = (fm.xs.len(), fm.ys.len());
    let g = if nx > 1 { fm.xs[1] - fm.xs[0] } else { GRID_MM };
    let hw = ((0.5 * w + margin) / g - 1e-9).ceil().max(0.0) as usize;
    let hh = ((0.5 * h + margin) / g - 1e-9).ceil().max(0.0) as usize;
    if 2 * hw + 1 > nx || 2 * hh + 1 > ny {
        return None;
    }
    // summed-area table of the valid points
    let w1 = nx + 1;
    let mut s = vec![0i64; (ny + 1) * w1];
    for y in 0..ny {
        for x in 0..nx {
            s[(y + 1) * w1 + x + 1] =
                i64::from(fm.valid[y * nx + x]) + s[y * w1 + x + 1] + s[(y + 1) * w1 + x]
                    - s[y * w1 + x];
        }
    }
    let area = ((2 * hh + 1) * (2 * hw + 1)) as i64;
    let full = |cy: usize, cx: usize| {
        let (y0, y1, x0, x1) = (cy - hh, cy + hh + 1, cx - hw, cx + hw + 1);
        s[y1 * w1 + x1] - s[y0 * w1 + x1] - s[y1 * w1 + x0] + s[y0 * w1 + x0] == area
    };
    let rows: Vec<usize> = (hh..ny - hh).collect();
    // the column whose box fits and is the closest to x = 0 (the first on ties)
    let mut best_col: Option<(f64, usize)> = None;
    for cx in hw..nx - hw {
        if rows.iter().any(|&cy| full(cy, cx)) {
            let d = fm.xs[cx].abs();
            if best_col.is_none_or(|(bd, _)| d < bd) {
                best_col = Some((d, cx));
            }
        }
    }
    let (_, cx) = best_col?;
    let (mut best, mut run, mut start, mut best_start) = (0usize, 0usize, 0usize, 0usize);
    for (i, &cy) in rows.iter().enumerate() {
        if full(cy, cx) {
            if run == 0 {
                start = i;
            }
            run += 1;
            if run > best {
                best = run;
                best_start = start;
            }
        } else {
            run = 0;
        }
    }
    let i = best_start + (best - 1) / 2;
    Some((fm.xs[cx], fm.ys[rows[i]]))
}

/// A spot found on one tooth.
#[derive(Debug, Clone)]
pub(super) struct Found {
    pub face: Face,
    pub tilt: f64,
    /// Index of the layout (into the sizes given).
    pub layout: usize,
    /// Centre of the text on the face map.
    pub pos: (f64, f64),
    /// Centre of the crown (origin of the rays).
    pub center: DVec3,
    pub frame: Frame,
}

/// The spot of the text on a tooth: `frames` are the faces to try in order,
/// `sizes` the `(w, h)` of the layouts in order of preference. For each face
/// the most legible layout that fits, at the smallest tilt.
pub(super) fn find_place(
    surface: &Surface<'_>,
    crown: &[DVec3],
    frames: &[(Face, Frame)],
    kind: Kind,
    cfg: &Config,
    sizes: &[(f64, f64)],
) -> Result<Found, String> {
    let mut reasons = Vec::new();
    for (face, fr0) in frames {
        let mut maps: Vec<(f64, FaceMap)> = Vec::new();
        for (li, &(w, h)) in sizes.iter().enumerate() {
            for &a in &TILTS {
                let idx = match maps.iter().position(|(t, _)| *t == a) {
                    Some(i) => i,
                    None => {
                        maps.push((a, face_map(surface, crown, tilt_frame(fr0, a), kind, cfg)));
                        maps.len() - 1
                    }
                };
                let fm = &maps[idx].1;
                if fm.reason.is_some() {
                    continue;
                }
                if let Some(pos) = place(fm, w, h, cfg.margin) {
                    return Ok(Found {
                        face: *face,
                        tilt: a,
                        layout: li,
                        pos,
                        center: fm.center,
                        frame: fm.frame,
                    });
                }
            }
        }
        let why = maps
            .iter()
            .find_map(|(_, fm)| fm.reason.clone())
            .unwrap_or_else(|| "the text does not fit".to_owned());
        reasons.push(format!("{} face: {why}", face.name()));
    }
    Err(reasons.join(", "))
}

/// The glyph points projected on the surface along `n` from the crown:
/// the hit points and the outward surface normals there. `None` when a ray
/// misses the surface.
pub(super) fn project(
    surface: &Surface<'_>,
    center: DVec3,
    frame: &Frame,
    pos: (f64, f64),
    text: &TextShape,
) -> Option<(Vec<DVec3>, Vec<DVec3>)> {
    let origins: Vec<DVec3> = text
        .points
        .iter()
        .map(|p: &DVec2| center + (p.x + pos.0) * frame.r + (p.y + pos.1) * frame.u)
        .collect();
    let hits = surface.cast(&origins, frame.n, 1);
    let mut points = Vec::with_capacity(hits.len());
    let mut normals = Vec::with_capacity(hits.len());
    for (o, h) in origins.iter().zip(&hits) {
        let hit = h.first()?;
        points.push(*o + f64::from(hit.t) * frame.n);
        normals.push(surface.normal_at(hit));
    }
    Some((points, normals))
}

/// Rays for a placement given by hand: the spot is the point of the surface
/// with its outward normal, the text reads from outside looking at the surface.
pub(super) fn manual_frame(normal: DVec3, up: DVec3) -> Option<Frame> {
    let n = -unit(normal);
    let u = unit(up - up.dot(n) * n);
    (n != DVec3::ZERO && u != DVec3::ZERO).then(|| Frame {
        n,
        u,
        r: unit(n.cross(u)),
    })
}
