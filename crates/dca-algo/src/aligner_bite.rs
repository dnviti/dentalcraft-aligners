//! Bite with the aligners worn: occlusal surfaces of the aligners, closing
//! of the mouth, contacts, occlusal thickness and equilibration.
//!
//! Reference: `ortho_aligner/aligner_bite.py` (plain numpy there). The case
//! level orchestration (`case_occlusal.py`) is [`occlusal`].
//!
//! The outer surface of an aligner is the surface of the model at distance
//! clearance + wall thickness; seen from the occlusal side (`+Z` of the arch
//! coordinates) it is the upper envelope of the balls of that radius
//! centred on the crown points ([`envelope`]). Teeth the aligner does not
//! cover take part with their own surface (radius 0).
//!
//! Occlusal thickness: the outer surface is the envelope of ellipsoids
//! instead of balls, of radius clearance + wall across (the axial walls keep
//! their thickness) and clearance + occlusal thickness along `+Z`
//! ([`with_occlusal`]). Equilibration ([`equilibrate`]): the material of
//! both aligners where they would interpenetrate when the mouth closes a
//! little more is taken off, never under the minimum thickness, until both
//! sides touch; the result is a height map per arch.
//!
//! Closing of the mouth: the upper arch, from the relation of the scans, is
//! moved until the first contact of the two surfaces, either by a rotation
//! about the hinge axis of the condyles ([`BiteClosing::Hinge`]) or by a
//! translation along the occlusal axis ([`BiteClosing::Vertical`]).
//!
//! The height maps are [`Raster`]s on a [`Grid`] in plan (`x`, `y`) of the
//! arch coordinates; `-inf` ([`NO`]) means no surface in a cell. Everything
//! is in millimetres; the relation `rel` maps the upper arch coordinates to
//! the lower arch coordinates.

pub mod occlusal;

use std::collections::BTreeMap;

use dc_geom::knn::KnnIndex;
use dca_model::BiteClosing;
use glam::{DMat4, DVec3, Vec3};
use serde::Serialize;

/// No surface in a cell.
pub const NO: f64 = f64::NEG_INFINITY;
/// A grid cell steeper than this is a wall (sampled at its lowest corner).
pub const STEEP_DEG: f64 = 60.0;

// ----------------------------------------------------------------------------
// grids and envelopes
// ----------------------------------------------------------------------------

/// A raster of values at the nodes of a [`Grid`] (`nx` x `ny`, `i` major).
#[derive(Debug, Clone, PartialEq)]
pub struct Raster<T> {
    /// Nodes along `x`.
    pub nx: usize,
    /// Nodes along `y`.
    pub ny: usize,
    /// Values, `i * ny + j`.
    pub data: Vec<T>,
}

impl<T: Copy> Raster<T> {
    /// A raster of `n` nodes filled with `v`.
    pub fn filled(n: [usize; 2], v: T) -> Self {
        Self {
            nx: n[0],
            ny: n[1],
            data: vec![v; n[0] * n[1]],
        }
    }

    /// Value at node `(i, j)`.
    #[inline]
    pub fn at(&self, i: usize, j: usize) -> T {
        self.data[i * self.ny + j]
    }

    /// Sets node `(i, j)`.
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: T) {
        self.data[i * self.ny + j] = v;
    }

    /// The raster with `f` applied to every value.
    pub fn map<U: Copy>(&self, f: impl Fn(T) -> U) -> Raster<U> {
        Raster {
            nx: self.nx,
            ny: self.ny,
            data: self.data.iter().map(|&v| f(v)).collect(),
        }
    }

    /// The raster of `f(self, other)` per node.
    pub fn zip<U: Copy, V: Copy>(&self, other: &Raster<U>, f: impl Fn(T, U) -> V) -> Raster<V> {
        Raster {
            nx: self.nx,
            ny: self.ny,
            data: self
                .data
                .iter()
                .zip(&other.data)
                .map(|(&a, &b)| f(a, b))
                .collect(),
        }
    }

    /// `self` moved so that `out[i, j] = self[i + di, j + dj]`, `fill`
    /// outside (`_shift`).
    pub fn shifted(&self, di: i64, dj: i64, fill: T) -> Raster<T> {
        let mut out = Raster::filled([self.nx, self.ny], fill);
        for i in 0..self.nx {
            let si = i as i64 + di;
            if si < 0 || si >= self.nx as i64 {
                continue;
            }
            for j in 0..self.ny {
                let sj = j as i64 + dj;
                if sj >= 0 && sj < self.ny as i64 {
                    out.set(i, j, self.at(si as usize, sj as usize));
                }
            }
        }
        out
    }

    /// `self` rolled periodically (`np.roll(a, shift, axis)`).
    fn rolled(&self, di: i64, dj: i64) -> Raster<T> {
        let (nx, ny) = (self.nx as i64, self.ny as i64);
        let mut out = self.clone();
        for i in 0..nx {
            for j in 0..ny {
                let (ti, tj) = ((i + di).rem_euclid(nx), (j + dj).rem_euclid(ny));
                out.data[(ti * ny + tj) as usize] = self.data[(i * ny + j) as usize];
            }
        }
        out
    }
}

/// Nodes of a regular grid in plan (`x`, `y`): `lo + (i, j) * cell`.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Cell size (mm).
    pub cell: f64,
    /// Plan position of node `(0, 0)`.
    pub lo: [f64; 2],
    /// Nodes along `x` and `y`.
    pub n: [usize; 2],
}

impl Grid {
    /// The grid from `lo` to `hi` (the last node at or beyond `hi`).
    pub fn new(lo: [f64; 2], hi: [f64; 2], cell: f64) -> Self {
        let n = |k: usize| ((hi[k] - lo[k]) / cell).ceil().max(0.0) as usize + 1;
        Self {
            cell,
            lo,
            n: [n(0), n(1)],
        }
    }

    /// Plan position of node `(i, j)`.
    #[inline]
    pub fn node(&self, i: usize, j: usize) -> [f64; 2] {
        [
            self.lo[0] + i as f64 * self.cell,
            self.lo[1] + j as f64 * self.cell,
        ]
    }

    /// Bilinear value of the raster `z` at the plan points `p`; `fill`
    /// where a corner has no surface or the point is outside. With
    /// `steep_deg` a cell whose corners differ more than its side at that
    /// slope is a wall (the labial face of an incisor): its lowest corner.
    pub fn sample(
        &self,
        z: &Raster<f64>,
        p: &[DVec3],
        fill: f64,
        steep_deg: Option<f64>,
    ) -> Vec<f64> {
        let wall_dz = steep_deg.map(|d| self.cell * d.to_radians().tan());
        p.iter()
            .map(|q| {
                let u = (q.x - self.lo[0]) / self.cell;
                let v = (q.y - self.lo[1]) / self.cell;
                let (i0, j0) = (u.floor(), v.floor());
                if !(i0 >= 0.0
                    && j0 >= 0.0
                    && i0 < (self.n[0] - 1) as f64
                    && j0 < (self.n[1] - 1) as f64)
                {
                    return fill;
                }
                let (i0, j0) = (i0 as usize, j0 as usize);
                let (u, v) = (u - i0 as f64, v - j0 as f64);
                let c = [
                    z.at(i0, j0),
                    z.at(i0 + 1, j0),
                    z.at(i0, j0 + 1),
                    z.at(i0 + 1, j0 + 1),
                ];
                if !c.iter().all(|x| x.is_finite()) {
                    return fill;
                }
                let val = c[0] * (1.0 - u) * (1.0 - v)
                    + c[1] * u * (1.0 - v)
                    + c[2] * (1.0 - u) * v
                    + c[3] * u * v;
                if let Some(dz) = wall_dz {
                    let hi = c.iter().cloned().fold(NO, f64::max);
                    let lo = c.iter().cloned().fold(f64::INFINITY, f64::min);
                    if hi - lo > dz {
                        return lo;
                    }
                }
                val
            })
            .collect()
    }

    /// Index `(i, j)` of the nearest node of a plan point and whether it is
    /// inside the grid.
    #[inline]
    pub fn nearest(&self, p: DVec3) -> (i64, i64, bool) {
        let i = ((p.x - self.lo[0]) / self.cell).round_ties_even() as i64;
        let j = ((p.y - self.lo[1]) / self.cell).round_ties_even() as i64;
        let ok = i >= 0 && j >= 0 && i < self.n[0] as i64 && j < self.n[1] as i64;
        (i, j, ok)
    }
}

/// Upper envelope of the balls of radius `r` centred on the points `p`, at
/// the nodes of `grid`: the height of the top of their union over every node
/// ([`NO`] where none reaches). With `rz` (< `r`) ellipsoids of radius `r`
/// across and `rz` along `Z`. A radius under 0.75 cell is taken as 0.75 cell
/// and the height lowered by the difference (the surface itself, within a
/// fraction of a cell). With `labels` also the label of the point that gives
/// the height at every node (0 where none).
pub fn envelope(
    p: &[DVec3],
    r: f64,
    grid: &Grid,
    labels: Option<&[i64]>,
    rz: Option<f64>,
) -> (Raster<f64>, Raster<i64>) {
    let [nx, ny] = grid.n;
    let mut z = Raster::filled(grid.n, NO);
    let mut l = Raster::filled(grid.n, 0i64);
    if p.is_empty() {
        return (z, l);
    }
    let reff = r.max(0.75 * grid.cell);
    let drop = reff - r;
    let rz = rz.map(|v| v.min(reff));
    let k = (reff / grid.cell).ceil() as i64;
    let mut off = Vec::new();
    for di in -k..=k {
        for dj in -k..=k {
            if (di as f64).hypot(dj as f64) * grid.cell <= reff + grid.cell {
                off.push((di, dj));
            }
        }
    }
    let height = |pt: &DVec3, dx: f64, dy: f64| -> Option<f64> {
        let h2 = reff * reff - dx * dx - dy * dy;
        (h2 >= 0.0).then(|| match rz {
            None => pt.z + h2.max(0.0).sqrt() - drop,
            Some(rz) => pt.z + rz * h2.max(0.0).sqrt() / reff,
        })
    };
    let each = |f: &mut dyn FnMut(usize, usize, usize, f64)| {
        for (a, pt) in p.iter().enumerate() {
            let (bi, bj, _) = grid.nearest(*pt);
            for &(di, dj) in &off {
                let (ii, jj) = (bi + di, bj + dj);
                if ii < 0 || jj < 0 || ii >= nx as i64 || jj >= ny as i64 {
                    continue;
                }
                let dx = grid.lo[0] + ii as f64 * grid.cell - pt.x;
                let dy = grid.lo[1] + jj as f64 * grid.cell - pt.y;
                if let Some(zz) = height(pt, dx, dy) {
                    f(a, ii as usize, jj as usize, zz);
                }
            }
        }
    };
    each(&mut |_, i, j, zz| {
        let c = &mut z.data[i * ny + j];
        if zz > *c {
            *c = zz;
        }
    });
    if let Some(lab) = labels {
        each(&mut |a, i, j, zz| {
            if zz >= z.data[i * ny + j] - 1e-9 {
                l.data[i * ny + j] = lab[a];
            }
        });
    }
    (z, l)
}

/// Closing of the height map `z` by a ball of `radius` rolling over it from
/// above (the blockout of the shell seen from the occlusal side: crevices
/// narrower than about twice the radius are filled). Never under `z`; where
/// a node has no surface the closing does not fill across it.
pub fn ball_closing(z: &Raster<f64>, cell: f64, radius: f64) -> Raster<f64> {
    if radius <= 0.0 {
        return z.clone();
    }
    let k = (radius / cell).ceil() as i64;
    let mut offs = Vec::new();
    for di in -k..=k {
        for dj in -k..=k {
            let d2 = (di as f64 * cell).powi(2) + (dj as f64 * cell).powi(2);
            if d2 <= radius * radius {
                offs.push((di, dj, (radius * radius - d2).max(0.0).sqrt()));
            }
        }
    }
    let mut d = Raster::filled([z.nx, z.ny], NO);
    for &(di, dj, h) in &offs {
        let s = z.shifted(di, dj, NO);
        for (a, b) in d.data.iter_mut().zip(&s.data) {
            *a = a.max(b + h);
        }
    }
    let mut e = Raster::filled([z.nx, z.ny], f64::INFINITY);
    for &(di, dj, h) in &offs {
        let s = d.shifted(di, dj, f64::INFINITY);
        for (a, b) in e.data.iter_mut().zip(&s.data) {
            *a = a.min(b - h);
        }
    }
    z.zip(&e, |zv, ev| {
        let out = zv.max(if ev.is_finite() { ev } else { NO });
        if zv.is_finite() { out } else { zv }
    })
}

/// Labels of the nodes with a surface but no label (rebuilt walls, fills
/// between the teeth) from their labelled neighbours (up to `passes` cells).
pub fn fill_labels(l: &Raster<i64>, z: &Raster<f64>, passes: usize) -> Raster<i64> {
    let mut l = l.clone();
    let has: Vec<bool> = z.data.iter().map(|v| v.is_finite()).collect();
    for _ in 0..passes {
        let miss: Vec<bool> = has
            .iter()
            .zip(&l.data)
            .map(|(&h, &v)| h && v == 0)
            .collect();
        if !miss.iter().any(|&m| m) {
            break;
        }
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nb = l.shifted(di, dj, 0);
            for ((m, cur), n) in miss.iter().zip(l.data.iter_mut()).zip(&nb.data) {
                if *m && *cur == 0 && *n != 0 {
                    *cur = *n;
                }
            }
        }
    }
    l
}

/// Top surface of the model points `p` (labels: FDI, 0 unknown) on `grid`,
/// closed by the blockout ball, with a label on every node.
pub fn heights(
    p: &[DVec3],
    labels: &[i64],
    grid: &Grid,
    blockout: f64,
) -> (Raster<f64>, Raster<i64>) {
    let (s, l) = envelope(p, 0.0, grid, Some(labels), None);
    let s = ball_closing(&s, grid.cell, blockout);
    let l = fill_labels(&l, &s, 8);
    (s, l)
}

/// The envelopes of one arch that do not depend on the occlusal thickness
/// (base of [`with_occlusal`]).
#[derive(Debug, Clone)]
pub struct Base {
    /// The points of the covered nodes (top of the model).
    pub p: Vec<DVec3>,
    /// Envelope at the wall thickness.
    pub zw: Raster<f64>,
    /// Label of the envelope at the wall thickness.
    pub lw: Raster<i64>,
    /// Envelope at the minimum occlusal thickness.
    pub zm: Raster<f64>,
    /// The bare (uncovered) teeth.
    pub zb: Raster<f64>,
    /// Labels of the bare teeth.
    pub lb: Raster<i64>,
    /// Every crown bare (the teeth without aligners).
    pub zall: Raster<f64>,
    /// Labels of every crown.
    pub lall: Raster<i64>,
    /// Clearance (mm).
    pub clearance: f64,
    /// Wall thickness (mm).
    pub wall: f64,
    /// Minimum occlusal thickness (mm).
    pub minimum: f64,
    /// The grid.
    pub grid: Grid,
}

fn is_in(l: i64, set: &[i64]) -> bool {
    set.contains(&l)
}

/// [`Base`] from the top surface of the model (`base_from_heights`): the
/// aligner is the offset of that surface (as the shell is the offset of the
/// closed model), the uncovered teeth take part bare.
pub fn base_from_heights(
    s: &Raster<f64>,
    l: &Raster<i64>,
    covered: &[i64],
    clearance: f64,
    wall: f64,
    minimum: f64,
    grid: &Grid,
) -> Base {
    let ok: Vec<bool> = s.data.iter().map(|v| v.is_finite()).collect();
    let on: Vec<bool> = ok
        .iter()
        .zip(&l.data)
        .map(|(&o, &lv)| o && !covered.is_empty() && is_in(lv, covered))
        .collect();
    let mut pn = Vec::new();
    let mut ln = Vec::new();
    for i in 0..grid.n[0] {
        for j in 0..grid.n[1] {
            if on[i * grid.n[1] + j] {
                let [x, y] = grid.node(i, j);
                pn.push(DVec3::new(x, y, s.at(i, j)));
                ln.push(l.at(i, j));
            }
        }
    }
    let rw = clearance + wall;
    let (zw, lw) = envelope(&pn, rw, grid, Some(&ln), None);
    let (zm, _) = envelope(&pn, rw, grid, None, Some(clearance + minimum.min(wall)));
    let bare: Vec<bool> = ok.iter().zip(&on).map(|(&o, &c)| o && !c).collect();
    let zb = Raster {
        nx: s.nx,
        ny: s.ny,
        data: s
            .data
            .iter()
            .zip(&bare)
            .map(|(&v, &b)| if b { v } else { NO })
            .collect(),
    };
    let lb = Raster {
        nx: l.nx,
        ny: l.ny,
        data: l
            .data
            .iter()
            .zip(&bare)
            .map(|(&v, &b)| if b { v } else { 0 })
            .collect(),
    };
    Base {
        p: pn,
        zw,
        lw,
        zm,
        zb,
        lb,
        zall: s.clone(),
        lall: l.clone(),
        clearance,
        wall,
        minimum,
        grid: grid.clone(),
    }
}

/// [`Base`] from crown points (`base_envelopes`): the covered teeth by the
/// ball of the wall, the others bare.
pub fn base_envelopes(
    p: &[DVec3],
    labels: &[i64],
    covered: &[i64],
    clearance: f64,
    wall: f64,
    minimum: f64,
    grid: &Grid,
) -> Base {
    let on: Vec<bool> = labels
        .iter()
        .map(|&l| !covered.is_empty() && is_in(l, covered))
        .collect();
    let pick = |want: bool| -> (Vec<DVec3>, Vec<i64>) {
        p.iter()
            .zip(labels)
            .zip(&on)
            .filter(|(_, o)| **o == want)
            .map(|((&q, &l), _)| (q, l))
            .unzip()
    };
    let (pon, lon) = pick(true);
    let (poff, loff) = pick(false);
    let rw = clearance + wall;
    let (zw, lw) = envelope(&pon, rw, grid, Some(&lon), None);
    let (zm, _) = envelope(&pon, rw, grid, None, Some(clearance + minimum.min(wall)));
    let (zb, lb) = envelope(&poff, 0.0, grid, Some(&loff), None);
    let (zall, lall) = envelope(p, 0.0, grid, Some(labels), None);
    Base {
        p: pon,
        zw,
        lw,
        zm,
        zb,
        lb,
        zall,
        lall,
        clearance,
        wall,
        minimum,
        grid: grid.clone(),
    }
}

/// The outer occlusal surface of one aligner (or of the bare teeth).
#[derive(Debug, Clone)]
pub struct Surface {
    /// The top of the aligner, or of the bare teeth.
    pub z: Raster<f64>,
    /// The envelope at the wall thickness.
    pub z_wall: Raster<f64>,
    /// The envelope at the minimum occlusal thickness.
    pub z_min: Raster<f64>,
    /// FDI at every node.
    pub l: Raster<i64>,
    /// Node on the aligner (not on a bare tooth).
    pub cov: Raster<bool>,
    /// The grid.
    pub grid: Grid,
    /// The occlusal thickness of this surface (mm; the wall for the bare
    /// teeth).
    pub occlusal: f64,
}

/// [`Surface`] from its base at the occlusal thickness `occlusal`.
pub fn with_occlusal(base: &Base, occlusal: f64) -> Surface {
    let (wall, c) = (base.wall, base.clearance);
    let occlusal = occlusal.min(wall);
    let zo = if occlusal < wall - 1e-9 {
        envelope(&base.p, c + wall, &base.grid, None, Some(c + occlusal)).0
    } else {
        base.zw.clone()
    };
    let cov = zo.zip(&base.zb, |o, b| o.is_finite() && o >= b);
    let zm = if occlusal < base.minimum {
        zo.clone()
    } else {
        base.zm.clone()
    };
    let pick = |a: &Raster<f64>, b: &Raster<f64>| {
        a.zip(&cov, |x, c| (x, c))
            .zip(b, |(x, c), y| if c { x } else { y })
    };
    let z = pick(&zo, &base.zb);
    let z_min = zm.zip(&zo, |m, o| if m.is_finite() { m } else { o });
    let l = base
        .lw
        .zip(&cov, |x, c| (x, c))
        .zip(&base.lb, |(x, c), y| if c { x } else { y });
    Surface {
        z,
        z_wall: base.zw.clone(),
        z_min,
        l,
        cov,
        grid: base.grid.clone(),
        occlusal,
    }
}

/// The teeth without aligners as a surface (the closing of the teeth).
pub fn bare_surface(base: &Base) -> Surface {
    let z = base.zall.clone();
    Surface {
        z_wall: z.clone(),
        z_min: z.clone(),
        l: base.lall.clone(),
        cov: Raster::filled([z.nx, z.ny], false),
        grid: base.grid.clone(),
        occlusal: base.wall,
        z,
    }
}

/// [`Surface`] of one aligner from crown points (`occlusal_surface`).
#[allow(clippy::too_many_arguments)]
pub fn occlusal_surface(
    p: &[DVec3],
    labels: &[i64],
    covered: &[i64],
    clearance: f64,
    wall: f64,
    occlusal: f64,
    minimum: f64,
    grid: &Grid,
) -> Surface {
    with_occlusal(
        &base_envelopes(p, labels, covered, clearance, wall, minimum, grid),
        occlusal,
    )
}

// ----------------------------------------------------------------------------
// closing of the mouth
// ----------------------------------------------------------------------------

/// Point of the hinge axis of the condyles (lower arch coordinates: `+Y`
/// towards the incisors, `+Z` towards the upper arch): `posterior_mm` behind
/// and `height_mm` above the incisal point. The axis is the `X` axis through
/// it.
pub fn hinge_axis(incisal: DVec3, posterior_mm: f64, height_mm: f64) -> DVec3 {
    DVec3::new(0.0, incisal.y - posterior_mm, incisal.z + height_mm)
}

/// The points `q` (lower arch coordinates) of the upper arch moved by
/// `value`: vertical a translation of `value` mm along `+Z`, hinge a
/// rotation of `value` radians about the hinge axis; positive values open
/// the mouth.
pub fn pose(q: &[DVec3], model: BiteClosing, value: f64, axis: Option<DVec3>) -> Vec<DVec3> {
    match model {
        BiteClosing::Vertical => q
            .iter()
            .map(|p| DVec3::new(p.x, p.y, p.z + value))
            .collect(),
        BiteClosing::Hinge => {
            let h = axis.unwrap_or(DVec3::ZERO);
            let (c, s) = (value.cos(), value.sin());
            q.iter()
                .map(|p| {
                    let (y, z) = (p.y - h.y, p.z - h.z);
                    DVec3::new(p.x, h.y + y * c - z * s, h.z + y * s + z * c)
                })
                .collect()
        }
    }
}

/// Vertical gap of every upper point `q` (lower coordinates) above the lower
/// surface (negative = interpenetration; inf where no lower surface).
pub fn gaps(grid: &Grid, z: &Raster<f64>, q: &[DVec3]) -> Vec<f64> {
    let h = grid.sample(z, q, NO, Some(STEEP_DEG));
    q.iter()
        .zip(h)
        .map(|(p, h)| {
            if h.is_finite() {
                p.z - h
            } else {
                f64::INFINITY
            }
        })
        .collect()
}

/// Teeth whose contacts are measured in 3D ([`Front`]): incisors and
/// canines, in the lower arch also the first premolars (the upper canine
/// rests between the lower canine and first premolar).
pub fn is_front(fdi: i64, upper: bool) -> bool {
    let i = fdi.rem_euclid(10);
    (1..=if upper { 3 } else { 4 }).contains(&i)
}

/// Incisors and canines of the two arches as point sets in 3D: the upper
/// incisors overlap the lower ones (overbite), which no height map can hold.
/// The aligners touch where the crowns come nearer than the sum of their
/// radii; the occlusal thickness makes them ellipsoids thinner along `Z`, the
/// same for both: in coordinates with `Z` scaled by `k = across / along Z`
/// they are balls.
pub struct Front {
    /// Scale of `Z`.
    pub k: f64,
    s: DVec3,
    pl: Vec<DVec3>,
    /// FDI of the lower points.
    pub lab_l: Vec<i64>,
    rl: Vec<f64>,
    /// Upper points in the lower coordinates, in the relation of the scans.
    pub q0: Vec<DVec3>,
    /// FDI of the upper points.
    pub lab_u: Vec<i64>,
    ru: Vec<f64>,
    index: Option<KnnIndex>,
}

impl Front {
    /// Lower points `pl` (radius `rl`), upper points `pu` in upper
    /// coordinates (radius `ru`), `rel` upper to lower.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pl: Vec<DVec3>,
        lab_l: Vec<i64>,
        rl: Vec<f64>,
        pu: &[DVec3],
        lab_u: Vec<i64>,
        ru: Vec<f64>,
        rel: &DMat4,
        k: f64,
    ) -> Self {
        let s = DVec3::new(1.0, 1.0, k);
        let q0 = pu.iter().map(|&p| rel.transform_point3(p)).collect();
        let index = (!pl.is_empty())
            .then(|| {
                let pts: Vec<Vec3> = pl.iter().map(|&p| (p * s).as_vec3()).collect();
                KnnIndex::build(&pts).ok()
            })
            .flatten();
        Self {
            k,
            s,
            pl,
            lab_l,
            rl,
            q0,
            lab_u,
            ru,
            index,
        }
    }

    /// `r_u + r_l - distance` for every upper point `q` (lower coordinates,
    /// posed; > 0 the aligners go into each other) and its nearest lower
    /// point.
    pub fn interference(&self, q: &[DVec3]) -> (Vec<f64>, Vec<usize>) {
        let Some(index) = &self.index else {
            return (vec![f64::NEG_INFINITY; q.len()], vec![0; q.len()]);
        };
        if q.is_empty() || self.pl.is_empty() {
            return (vec![f64::NEG_INFINITY; q.len()], vec![0; q.len()]);
        }
        let qs: Vec<Vec3> = q.iter().map(|&p| (p * self.s).as_vec3()).collect();
        let hits = index.nearest_batch(&qs);
        let mut f = Vec::with_capacity(q.len());
        let mut idx = Vec::with_capacity(q.len());
        for (a, h) in hits.into_iter().enumerate() {
            let h = h.expect("a non-empty index has a nearest point");
            f.push(self.ru[a] + self.rl[h.index as usize] - f64::from(h.distance));
            idx.push(h.index as usize);
        }
        (f, idx)
    }

    /// The largest interference at the pose `value` (None without points).
    pub fn worst(&self, model: BiteClosing, value: f64, axis: Option<DVec3>) -> Option<f64> {
        let (f, _) = self.interference(&pose(&self.q0, model, value, axis));
        f.iter().cloned().reduce(f64::max)
    }
}

/// Smallest opening (pose value) at which the upper points `q` (lower
/// coordinates, in the relation of the scans) do not go into the lower
/// surface, nor the front teeth into each other: the first contact. None
/// when the surfaces never overlap in plan.
#[allow(clippy::too_many_arguments)]
pub fn first_contact(
    grid: &Grid,
    z: &Raster<f64>,
    q: &[DVec3],
    model: BiteClosing,
    axis: Option<DVec3>,
    span: Option<f64>,
    iters: usize,
    front: Option<&Front>,
) -> Option<f64> {
    let span = span.unwrap_or(match model {
        BiteClosing::Vertical => 8.0,
        BiteClosing::Hinge => 12.0f64.to_radians(),
    });
    let worst = |v: f64| -> Option<f64> {
        let g = gaps(grid, z, &pose(q, model, v, axis));
        let mut w = g
            .iter()
            .filter(|x| x.is_finite())
            .cloned()
            .reduce(f64::min)
            .map(|m| -m);
        let f = front.and_then(|f| f.worst(model, v, axis));
        if let Some(f) = f.filter(|f| f.is_finite()) {
            w = Some(w.map_or(f, |w| w.max(f)));
        }
        w
    };
    let (mut lo, mut hi) = (-span, span);
    let (w_lo, w_hi) = (worst(lo), worst(hi));
    let (w_lo, w_hi) = (w_lo?, w_hi?);
    if w_hi > 0.0 {
        hi = 3.0 * span;
        if worst(hi).unwrap_or(0.0) > 0.0 {
            return Some(hi);
        }
    }
    if w_lo <= 0.0 {
        return Some(lo);
    }
    for _ in 0..iters {
        let mid = 0.5 * (lo + hi);
        if worst(mid).unwrap_or(0.0) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi)
}

/// `'R'` for quadrants 1 and 4, `'L'` for 2 and 3, `' '` for 0.
pub fn side_of(fdi: i64) -> char {
    match fdi / 10 {
        1 | 4 => 'R',
        2 | 3 => 'L',
        _ => ' ',
    }
}

fn side_index(c: char) -> Option<usize> {
    match c {
        'R' => Some(0),
        'L' => Some(1),
        _ => None,
    }
}

/// Teeth touching on each side of the arches.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Sides {
    /// Right side (quadrants 1 and 4), ascending.
    pub right: Vec<i64>,
    /// Left side (quadrants 2 and 3), ascending.
    pub left: Vec<i64>,
}

impl Sides {
    /// The teeth of a side (`'R'` or `'L'`).
    pub fn of(&self, side: char) -> &[i64] {
        if side == 'R' { &self.right } else { &self.left }
    }
}

/// Points per tooth in contact for the upper and lower arches, and the
/// teeth per side (`contact_teeth`): points of the upper surface with a gap
/// within `contact_mm`; a tooth counts with at least `min_points`.
pub fn contact_teeth(
    g: &[f64],
    lab_u: &[i64],
    lab_l: &[i64],
    contact_mm: f64,
    min_points: usize,
) -> (BTreeMap<i64, usize>, BTreeMap<i64, usize>, Sides) {
    let mut up: BTreeMap<i64, usize> = BTreeMap::new();
    let mut lo: BTreeMap<i64, usize> = BTreeMap::new();
    for ((&gv, &u), &l) in g.iter().zip(lab_u).zip(lab_l) {
        if gv.is_finite() && gv <= contact_mm {
            if u != 0 {
                *up.entry(u).or_default() += 1;
            }
            if l != 0 {
                *lo.entry(l).or_default() += 1;
            }
        }
    }
    let mut sides: [std::collections::BTreeSet<i64>; 2] = Default::default();
    for d in [&up, &lo] {
        for (&f, &n) in d {
            if n >= min_points
                && let Some(s) = side_index(side_of(f))
            {
                sides[s].insert(f);
            }
        }
    }
    let [r, l] = sides;
    (
        up,
        lo,
        Sides {
            right: r.into_iter().collect(),
            left: l.into_iter().collect(),
        },
    )
}

/// The nodes of an occlusal surface as points of its own arch with their FDI
/// and node index (only the nodes of `mask` when given).
pub fn surface_points(
    surf: &Surface,
    mask: Option<&Raster<bool>>,
) -> (Vec<DVec3>, Vec<i64>, Vec<(usize, usize)>) {
    let mut p = Vec::new();
    let mut l = Vec::new();
    let mut idx = Vec::new();
    for i in 0..surf.grid.n[0] {
        for j in 0..surf.grid.n[1] {
            let z = surf.z.at(i, j);
            if z.is_finite() && mask.is_none_or(|m| m.at(i, j)) {
                let [x, y] = surf.grid.node(i, j);
                p.push(DVec3::new(x, y, z));
                l.push(surf.l.at(i, j));
                idx.push((i, j));
            }
        }
    }
    (p, l, idx)
}

/// Nodes of the upper surface that are not under the lower one in the
/// relation of the scans (false where an upper point lies under a lower
/// tooth by more than `tol_mm`: the upper incisors under the lower ones in an
/// overbite, which a height map cannot tell from an interpenetration), the
/// excluded region grown by `grow_mm` in plan (the offset of the aligners).
/// Evaluated on the bare teeth ([`bare_surface`]), used for every closing.
pub fn overlap_mask(
    lower: &Surface,
    upper: &Surface,
    rel: &DMat4,
    tol_mm: f64,
    grow_mm: f64,
) -> Raster<bool> {
    let (pu, _, idx) = surface_points(upper, None);
    let q: Vec<DVec3> = pu.iter().map(|&p| rel.transform_point3(p)).collect();
    let g = gaps(&lower.grid, &lower.z, &q);
    let mut bad = Raster::filled([upper.z.nx, upper.z.ny], false);
    for (gv, &(i, j)) in g.iter().zip(&idx) {
        if *gv < -tol_mm {
            bad.set(i, j, true);
        }
    }
    let k = (grow_mm / upper.grid.cell).ceil() as i64;
    if k > 0 && bad.data.iter().any(|&b| b) {
        let mut grown = bad.clone();
        for di in -k..=k {
            for dj in -k..=k {
                if di * di + dj * dj > k * k {
                    continue;
                }
                let r = bad.rolled(di, dj);
                for (a, b) in grown.data.iter_mut().zip(&r.data) {
                    *a |= *b;
                }
            }
        }
        bad = grown;
    }
    bad.map(|b| !b)
}

/// The mouth closed on the two aligners (`closing`).
#[derive(Debug, Clone)]
pub struct Closing {
    /// Pose at the first contact.
    pub value: f64,
    /// The upper points before the pose (lower coordinates).
    pub q0: Vec<DVec3>,
    /// The upper points at the pose (lower coordinates).
    pub q: Vec<DVec3>,
    /// Their gaps to the lower surface.
    pub g: Vec<f64>,
    /// FDI of the upper points.
    pub lab_u: Vec<i64>,
    /// FDI of the lower surface under each upper point.
    pub lab_l: Vec<i64>,
    /// Points in contact per upper tooth.
    pub upper: BTreeMap<i64, usize>,
    /// Points in contact per lower tooth.
    pub lower: BTreeMap<i64, usize>,
    /// Teeth in contact per side.
    pub sides: Sides,
    /// The upper front points at the pose.
    pub front_q: Vec<DVec3>,
    /// Their gaps (minus the interference).
    pub front_g: Vec<f64>,
    /// Smallest gap per side (right, left).
    pub side_gap: [Option<f64>; 2],
}

/// Closing of the mouth on the two surfaces (`closing`; `rel` upper to
/// lower; `mask`: the upper nodes taken into account, [`overlap_mask`];
/// `front`: the front teeth in 3D). None when the surfaces never overlap.
#[allow(clippy::too_many_arguments)]
pub fn closing(
    lower: &Surface,
    upper: &Surface,
    rel: &DMat4,
    model: BiteClosing,
    axis: Option<DVec3>,
    contact_mm: f64,
    mask: Option<&Raster<bool>>,
    front: Option<&Front>,
) -> Option<Closing> {
    let (pu, lab_u, _) = surface_points(upper, mask);
    let q0: Vec<DVec3> = pu.iter().map(|&p| rel.transform_point3(p)).collect();
    let v = first_contact(&lower.grid, &lower.z, &q0, model, axis, None, 50, front)?;
    Some(at_pose(
        lower, q0, lab_u, model, v, axis, contact_mm, front, None,
    ))
}

#[allow(clippy::too_many_arguments)]
fn at_pose(
    lower: &Surface,
    q0: Vec<DVec3>,
    lab_u: Vec<i64>,
    model: BiteClosing,
    v: f64,
    axis: Option<DVec3>,
    contact_mm: f64,
    front: Option<&Front>,
    g: Option<Vec<f64>>,
) -> Closing {
    let q = pose(&q0, model, v, axis);
    let g = g.unwrap_or_else(|| gaps(&lower.grid, &lower.z, &q));
    let mut lab_l = vec![0i64; q.len()];
    for (a, p) in q.iter().enumerate() {
        let (i, j, ok) = lower.grid.nearest(*p);
        if ok {
            lab_l[a] = lower.l.at(i as usize, j as usize);
        }
    }
    let mut f_q = Vec::new();
    let mut f_g = Vec::new();
    let (mut g_all, mut lu_all, mut ll_all) = (g.clone(), lab_u.clone(), lab_l.clone());
    if let Some(fr) = front.filter(|f| !f.q0.is_empty()) {
        f_q = pose(&fr.q0, model, v, axis);
        let (f, fi) = fr.interference(&f_q);
        f_g = f.iter().map(|x| -x).collect();
        g_all.extend_from_slice(&f_g);
        lu_all.extend_from_slice(&fr.lab_u);
        ll_all.extend(fi.iter().map(|&i| fr.lab_l[i]));
    }
    let (up, lo, sides) = contact_teeth(&g_all, &lu_all, &ll_all, contact_mm, 2);
    let mut side_gap = [None, None];
    for (gv, &u) in g_all.iter().zip(&lu_all) {
        if let Some(s) = side_index(side_of(u))
            && gv.is_finite()
        {
            side_gap[s] = Some(side_gap[s].map_or(*gv, |m: f64| m.min(*gv)));
        }
    }
    Closing {
        value: v,
        q0,
        q,
        g,
        lab_u,
        lab_l,
        upper: up,
        lower: lo,
        sides,
        front_q: f_q,
        front_g: f_g,
        side_gap,
    }
}

/// Vertical opening (mm) of a point of the upper arch (lower coordinates)
/// when the pose goes from 0 to `value`.
pub fn opening_at(point: DVec3, model: BiteClosing, value: f64, axis: Option<DVec3>) -> f64 {
    pose(&[point], model, value, axis)[0].z - point.z
}

// ----------------------------------------------------------------------------
// equilibration
// ----------------------------------------------------------------------------

/// Result of [`equilibrate`].
#[derive(Debug, Clone)]
pub struct Equilibration {
    /// Both sides touch (the wanted teeth per side).
    pub done: bool,
    /// The pose at the end.
    pub value: Option<f64>,
    /// Grinding of the lower aligner (its grid).
    pub grind_l: Raster<f64>,
    /// Grinding of the upper aligner (its grid).
    pub grind_u: Raster<f64>,
    /// The closing after the grinding.
    pub closing: Option<Closing>,
    /// Steps taken.
    pub steps: usize,
    /// The closing without grinding.
    pub before: Option<Closing>,
    /// Why it stopped short: `"spessore"` (thickness or grinding limit),
    /// `"anteriori"` (the front teeth touch), `"passi"` (steps used up), or
    /// empty.
    pub stop: &'static str,
}

/// Material that can be ground off at every node of a surface: down to its
/// minimum thickness, at most `max_mm`, nothing on a bare tooth.
fn room(surf: &Surface, max_mm: f64) -> Raster<f64> {
    let mut out = Raster::filled([surf.z.nx, surf.z.ny], 0.0);
    for k in 0..surf.z.data.len() {
        let (z, zm) = (surf.z.data[k], surf.z_min.data[k]);
        if surf.cov.data[k] && z.is_finite() && zm.is_finite() {
            out.data[k] = (z - zm).clamp(0.0, max_mm.max(0.0));
        }
    }
    out
}

/// The cells of a scattered grinding map left between ground cells get the
/// larger of their neighbours (one cell: the upper points fall on the lower
/// grid with gaps where the arches are turned).
fn close_gaps(g: &Raster<f64>) -> Raster<f64> {
    let (nx, ny) = (g.nx, g.ny);
    let mut out = g.clone();
    let get = |i: i64, j: i64| -> f64 {
        if i < 0 || j < 0 || i >= nx as i64 || j >= ny as i64 {
            0.0
        } else {
            g.at(i as usize, j as usize)
        }
    };
    let roll = |i: i64, j: i64| -> f64 {
        g.at(
            i.rem_euclid(nx as i64) as usize,
            j.rem_euclid(ny as i64) as usize,
        )
    };
    for i in 0..nx as i64 {
        for j in 0..ny as i64 {
            let nb = get(i - 1, j)
                .max(get(i + 1, j))
                .max(get(i, j - 1))
                .max(get(i, j + 1));
            let hole = g.at(i as usize, j as usize) == 0.0 && nb > 0.0;
            if !hole {
                continue;
            }
            let cnt = [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)]
                .iter()
                .filter(|&&(a, b)| roll(a, b) > 0.0)
                .count();
            if cnt >= 2 {
                out.set(i as usize, j as usize, nb);
            }
        }
    }
    out
}

/// Virtual grinding of the two aligners until both sides touch. From the
/// first contact the mouth closes by steps (about `step_mm` at the farthest
/// point of the arch); where the surfaces would interpenetrate the material
/// is taken off, half from each aligner (more from one when the other is
/// already at its minimum thickness: `Z - Z_min`), never deeper than
/// `max_mm` on either and never on a bare tooth. It stops at the first step
/// with at least `teeth_per_side` teeth touching on each side, or when going
/// on would need more than the aligners can give.
#[allow(clippy::too_many_arguments)]
pub fn equilibrate(
    lower: &Surface,
    upper: &Surface,
    rel: &DMat4,
    model: BiteClosing,
    axis: Option<DVec3>,
    contact_mm: f64,
    teeth_per_side: usize,
    max_mm: f64,
    mask: Option<&Raster<bool>>,
    front: Option<&Front>,
) -> Equilibration {
    const STEP_MM: f64 = 0.02;
    const MAX_STEPS: usize = 200;
    let first = closing(lower, upper, rel, model, axis, contact_mm, mask, front);
    let nul = Equilibration {
        done: false,
        value: first.as_ref().map(|c| c.value),
        grind_l: Raster::filled([lower.z.nx, lower.z.ny], 0.0),
        grind_u: Raster::filled([upper.z.nx, upper.z.ny], 0.0),
        closing: first.clone(),
        steps: 0,
        before: first.clone(),
        stop: "",
    };
    let Some(first) = first else {
        return nul;
    };
    let enough = |s: &Sides| s.right.len() >= teeth_per_side && s.left.len() >= teeth_per_side;
    if enough(&first.sides) {
        return Equilibration { done: true, ..nul };
    }
    let (_, lab_u, idx_u) = surface_points(upper, mask);
    let q0 = first.q0.clone();
    let gl = &lower.grid;
    let room_u = room(upper, max_mm);
    let avail_u: Vec<f64> = idx_u.iter().map(|&(i, j)| room_u.at(i, j)).collect();
    let avail_l_grid = room(lower, max_mm);
    let dv = match model {
        BiteClosing::Vertical => STEP_MM,
        BiteClosing::Hinge => {
            let a = axis.unwrap_or(DVec3::ZERO);
            let far = q0
                .iter()
                .map(|p| (p.y - a.y).hypot(p.z - a.z))
                .fold(0.0, f64::max);
            STEP_MM / far.max(1.0)
        }
    };
    struct Best {
        v: f64,
        su: Vec<f64>,
        sl: Vec<f64>,
        ij: Vec<(i64, i64, bool)>,
        after: Closing,
        n: usize,
    }
    let mut best: Option<Best> = None;
    let mut stop = "passi";
    for n in 1..=MAX_STEPS {
        let v = first.value - n as f64 * dv;
        let q = pose(&q0, model, v, axis);
        let g = gaps(gl, &lower.z, &q);
        let need: Vec<f64> = g
            .iter()
            .map(|&x| if x.is_finite() { (-x).max(0.0) } else { 0.0 })
            .collect();
        let ij: Vec<(i64, i64, bool)> = q.iter().map(|&p| gl.nearest(p)).collect();
        let avail_l: Vec<f64> = ij
            .iter()
            .map(|&(i, j, ok)| {
                if ok {
                    avail_l_grid.at(i as usize, j as usize)
                } else {
                    0.0
                }
            })
            .collect();
        if need
            .iter()
            .zip(avail_u.iter().zip(&avail_l))
            .any(|(&nd, (&au, &al))| nd > au + al + 1e-9)
        {
            stop = "spessore";
            break;
        }
        if let Some(fr) = front
            && fr.worst(model, v, axis).unwrap_or(-1.0) > 0.0
        {
            stop = "anteriori";
            break;
        }
        let su: Vec<f64> = need
            .iter()
            .zip(&avail_l)
            .zip(&avail_u)
            .map(|((&nd, &al), &au)| (0.5 * nd).max(nd - al).min(au))
            .collect();
        let sl: Vec<f64> = need.iter().zip(&su).map(|(&nd, &s)| nd - s).collect();
        let g_after: Vec<f64> = g
            .iter()
            .zip(&need)
            .map(|(&gv, &nd)| if nd > 0.0 { 0.0 } else { gv })
            .collect();
        let after = at_pose(
            lower,
            q0.clone(),
            lab_u.clone(),
            model,
            v,
            axis,
            contact_mm,
            front,
            Some(g_after),
        );
        let done = enough(&after.sides);
        best = Some(Best {
            v,
            su,
            sl,
            ij,
            after,
            n,
        });
        if done {
            stop = "";
            break;
        }
    }
    let Some(b) = best else {
        return Equilibration { stop, ..nul };
    };
    let mut grind_u = Raster::filled([upper.z.nx, upper.z.ny], 0.0);
    for (&(i, j), &s) in idx_u.iter().zip(&b.su) {
        grind_u.set(i, j, s);
    }
    let mut grind_l = Raster::filled(gl.n, 0.0);
    for (&(i, j, ok), &s) in b.ij.iter().zip(&b.sl) {
        if ok && s > 0.0 {
            let c = &mut grind_l.data[i as usize * gl.n[1] + j as usize];
            *c = f64::max(*c, s);
        }
    }
    let mut grind_l = close_gaps(&grind_l);
    for (a, m) in grind_l.data.iter_mut().zip(&avail_l_grid.data) {
        *a = a.min(*m);
    }
    let done = enough(&b.after.sides);
    Equilibration {
        done,
        value: Some(b.v),
        grind_l,
        grind_u,
        closing: Some(b.after),
        steps: b.n,
        before: Some(first),
        stop: if done { "" } else { stop },
    }
}

/// Height map of the limit solid of one aligner (its own grid): the top
/// allowed where the occlusal thickness or the grinding takes material off
/// (`Z - grind`), NaN where nothing is taken off (no limit there).
pub fn limit_heights(surf: &Surface, grind: Option<&Raster<f64>>) -> Raster<f64> {
    const EPS: f64 = 1e-3;
    let mut out = Raster::filled([surf.z.nx, surf.z.ny], f64::NAN);
    for k in 0..surf.z.data.len() {
        let zw = surf.z_wall.data[k];
        let cov = surf.cov.data[k];
        let mut target = if cov { surf.z.data[k] } else { zw };
        if let Some(g) = grind
            && cov
        {
            target -= g.data[k];
        }
        if zw.is_finite() && cov && target < zw - EPS {
            out.data[k] = target;
        }
    }
    out
}

// ----------------------------------------------------------------------------
// automatic occlusal thickness
// ----------------------------------------------------------------------------

/// Occlusal thickness (the same on both aligners) that brings the opening of
/// the bite caused by the aligners to `target` mm: the opening measured with
/// the wall thickness (`open_wall`) and with the minimum one (`open_min`) is
/// about linear in the thickness between them. Clamped to `[minimum, wall]`:
/// the wall when its opening is already within the target, the minimum when
/// even that one is not.
pub fn solve_thickness(open_wall: f64, open_min: f64, wall: f64, minimum: f64, target: f64) -> f64 {
    if open_wall <= target || wall <= minimum {
        return wall;
    }
    if open_min >= target || (open_wall - open_min).abs() < 1e-9 {
        return minimum;
    }
    let t = minimum + (wall - minimum) * (target - open_min) / (open_wall - open_min);
    t.clamp(minimum, wall)
}
