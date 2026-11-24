//! Embrasure fills of the print model (`case_print.embrasure_fills`): under
//! the contact of every two touching neighbouring teeth, closed solids that
//! fill the plan between their crowns up to the two lines tangent to both
//! (labial and lingual), from under the crowns up to the top of the contact.
//!
//! The closing of the model joins two crowns at their contact; the gingival
//! embrasure under it stays open (the papilla fills it in the mouth) and the
//! model gets a tunnel under every contact, a hole through the printed
//! model, and the shell a fin into it. The lab blocks it out with wax, the
//! sheet spans from one crown to the other: the fill does the same.
//!
//! Every fill is a stack of slices: the convex hull of the sections of the
//! two crowns at the height of the slice, cut to the stretch from
//! [`INTO_MM`] inside one crown to as far inside the other, each slice
//! holding the one above it (no overhang: the shell would go under a wider
//! slice). Above the contact the occlusal embrasure stays as it is.
//!
//! Deviations from the plugin: the bottom of a fill is [`BELOW_MM`] under the
//! lowest point of the two closed teeth near the contact, but not deeper
//! than that under the gingiva under the contact (the plugin takes the
//! lowest scanned point of the crowns; the closed teeth here may reach under
//! the gingiva, and a fill held at the width of the crowns down there stood
//! out of it). Where the papilla of one side (labial or lingual) stands
//! lower than that gingiva, the half of the fill on that side of the plane
//! of the contact goes on down to as far under its tip: the embrasure over
//! it stayed open (an upper molar, its palatal gingiva near the contact,
//! the buccal papilla 5 mm lower, the shell 1.5 mm into the buccal
//! embrasure).
//!
//! Under the shell of an aligner with the smooth border
//! (`stage_parts(smooth=True)`) the labial and lingual faces of every slice
//! stand `inset` inside the tangents ([`inset`], the plugin's `trim_recess`):
//! the rounded closing of the shell draws a hollow from one crown to the
//! other over it, as deep as the inset, and the wall bridges the embrasure
//! instead of entering it down to the papilla. The plugin's draft of those
//! slices (`EMBRASURE_FILL_DRAFT`) is not ported: every slice is the hull of
//! the one above it with the two sections before it is drawn in, so the
//! draft changes nothing.

use dc_case::Fdi;
use dc_geom::knn::KnnIndex;
use glam::{DVec2, DVec3, Vec3};

use crate::anatomy::{Arch, arch_order};

/// Two teeth touch where a vertex of one is this close to a vertex of the
/// other (mm, `CONTACT_TOUCH_MM`).
pub const TOUCH_MM: f64 = 0.8;
/// Height of the slices (mm, `EMBRASURE_FILL_STEP_MM`).
pub const STEP_MM: f64 = 0.15;
/// A slice reaches this far into each crown along the arch (mm,
/// `EMBRASURE_FILL_INTO_MM`).
pub const INTO_MM: f64 = 1.0;
/// The fill goes down to this far under the crowns near the contact (mm,
/// `EMBRASURE_FILL_BELOW_MM`), into the gingiva of the model.
pub const BELOW_MM: f64 = 2.0;
/// ... the points of the crowns within this of the contact in plan (mm,
/// `EMBRASURE_FILL_NEAR_MM`).
pub const NEAR_MM: f64 = 4.0;
/// ... and at most that far under the highest point of the gingiva within
/// this of the contact in plan (mm; the col under it).
pub const COL_MM: f64 = 1.0;
/// The whole embrasure ([`fills_whole`], under the shell of an aligner): two
/// neighbouring teeth apart by at most this (mm) are bridged as if they
/// touched (a space the plan closes, a diastema; a missing tooth is wider).
pub const SPACE_MM: f64 = 3.0;
/// ... the fill starts at the top of the embrasure, the lower of the two
/// crowns within this (mm) of the plane of the contact (the marginal ridges,
/// the incisal corners), ...
pub const RIDGE_MM: f64 = 1.5;
/// ... this far (mm) under it.
pub const RIDGE_TOP_MM: f64 = 0.3;

/// A closed tooth at the stage (arch coordinates, `+Z` occlusal).
#[derive(Debug, Clone, Copy)]
pub struct Tooth<'a> {
    /// Tooth number.
    pub fdi: Fdi,
    /// Vertices.
    pub points: &'a [DVec3],
    /// Triangles.
    pub tris: &'a [[u32; 3]],
}

/// A closed solid: vertices and outward triangles.
pub type Solid = (Vec<DVec3>, Vec<[u32; 3]>);

/// Points where the edges of a mesh cross the height `z` (plan).
fn section(points: &[DVec3], tris: &[[u32; 3]], z: f64) -> Vec<DVec2> {
    let mut out = Vec::new();
    for t in tris {
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            // every edge once (the other triangle sees it reversed)
            if i > j {
                continue;
            }
            let (p, q) = (points[i as usize], points[j as usize]);
            if p.z.min(q.z) < z && p.z.max(q.z) >= z {
                let s = (z - p.z) / (q.z - p.z);
                let x = p + (q - p) * s;
                out.push(DVec2::new(x.x, x.y));
            }
        }
    }
    out
}

/// Convex hull of plan points, counter-clockwise (monotone chain).
fn hull(points: &[DVec2]) -> Vec<DVec2> {
    let mut p: Vec<DVec2> = points.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| a.distance_squared(*b) < 1e-12);
    if p.len() < 3 {
        return p;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut h: Vec<DVec2> = Vec::with_capacity(2 * p.len());
    for pass in 0..2 {
        let start = h.len();
        let iter: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in iter {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// Convex polygon cut to `n . p <= c` (Sutherland-Hodgman).
fn clip(poly: &[DVec2], n: DVec2, c: f64) -> Vec<DVec2> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n.dot(a) - c, n.dot(b) - c);
        if da <= 0.0 {
            out.push(a);
        }
        if da * db < 0.0 {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Convex counter-clockwise polygon with its sides moved `d` inwards, except
/// the sides square to `u` (the ends of a fill along the arch; `_inset`).
fn inset(poly: &[DVec2], u: DVec2, d: f64) -> Vec<DVec2> {
    let mut out = poly.to_vec();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = b - a;
        let l = e.length();
        if l < 1e-9 {
            continue;
        }
        // outward for a counter-clockwise polygon
        let n = DVec2::new(e.y, -e.x) / l;
        if n.dot(u).abs() > 0.9 {
            continue;
        }
        out = clip(&out, n, n.dot(a) - d);
        if out.len() < 3 {
            break;
        }
    }
    out
}

/// Closed prism over a convex counter-clockwise plan polygon, outward faces.
fn prism(poly: &[DVec2], z0: f64, z1: f64) -> Solid {
    let k = poly.len() as u32;
    let mut v: Vec<DVec3> = poly.iter().map(|p| DVec3::new(p.x, p.y, z0)).collect();
    v.extend(poly.iter().map(|p| DVec3::new(p.x, p.y, z1)));
    let mut t = Vec::new();
    for i in 1..k - 1 {
        t.push([0, i + 1, i]);
        t.push([k, k + i, k + i + 1]);
    }
    for i in 0..k {
        let j = (i + 1) % k;
        t.push([i, j, k + j]);
        t.push([i, k + j, k + i]);
    }
    (v, t)
}

/// The tip of the papilla on the side `n` (unit, plan, square to the arch
/// direction `u`) of the plane of the contact `c` (plan): the highest point
/// of the gingiva `gum` within [`COL_MM`] of that plane along the arch and
/// [`NEAR_MM`] of the contact, not above `z_top`; `−∞` without one.
fn side_top(gum: &[DVec3], c: DVec2, u: DVec2, n: DVec2, z_top: f64) -> f64 {
    gum.iter()
        .filter(|q| {
            let d = DVec2::new(q.x, q.y) - c;
            q.z <= z_top && d.dot(n) > 0.0 && d.dot(u).abs() < COL_MM && d.length() < NEAR_MM
        })
        .map(|q| q.z)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The fill under the contact of two neighbouring teeth `a`, `b`, its
/// labial and lingual faces `inset` inside the tangents; `None` when they
/// do not touch. `whole`: the whole embrasure, from the top of the lower
/// crown beside the contact ([`RIDGE_MM`]) down, and between two teeth up
/// to [`SPACE_MM`] apart: the shell of an aligner formed on it goes straight
/// from one crown to the other and never into the space between them (the
/// user's rule, 2026-11-24: in there it is in the way, crooked and fragile).
fn pair_fill(
    a: &Tooth<'_>,
    b: &Tooth<'_>,
    gum: &[DVec3],
    inset_mm: f64,
    whole: bool,
) -> Option<Solid> {
    if a.points.is_empty() || b.points.is_empty() {
        return None;
    }
    let bounds = |p: &[DVec3]| {
        p.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), q| (lo.min(*q), hi.max(*q)),
        )
    };
    let (lo, hi) = bounds(a.points);
    let reach = if whole { SPACE_MM } else { TOUCH_MM };
    let (lo, hi) = (lo - DVec3::splat(reach), hi + DVec3::splat(reach));
    let cand: Vec<usize> = (0..b.points.len())
        .filter(|&i| b.points[i].cmpgt(lo).all() && b.points[i].cmplt(hi).all())
        .collect();
    if cand.is_empty() {
        return None;
    }
    let pa: Vec<Vec3> = a.points.iter().map(|p| p.as_vec3()).collect();
    let index = KnnIndex::build(&pa).ok()?;
    let near: Vec<(f64, DVec3, DVec3)> = cand
        .iter()
        .filter_map(|&i| {
            let n = index.nearest(b.points[i].as_vec3())?;
            Some((
                f64::from(n.distance),
                b.points[i],
                a.points[n.index as usize],
            ))
        })
        .collect();
    let closest = near.iter().map(|n| n.0).fold(f64::INFINITY, f64::min);
    // the teeth touch within TOUCH_MM; apart (whole only, up to SPACE_MM),
    // the zone is where they are closest
    let within = if closest <= TOUCH_MM {
        TOUCH_MM
    } else if whole && closest <= SPACE_MM {
        closest + 0.3
    } else {
        return None;
    };
    let mut zone = Vec::new();
    for &(d, pb, pa) in &near {
        if d <= within {
            zone.push(pb);
            zone.push(pa);
        }
    }
    let top = zone
        .iter()
        .copied()
        .reduce(|p, q| if q.z > p.z { q } else { p })?;
    let z_top = top.z;
    let c = DVec2::new(top.x, top.y);
    let low = |p: &[DVec3]| {
        p.iter()
            .filter(|q| DVec2::new(q.x, q.y).distance(c) < NEAR_MM)
            .map(|q| q.z)
            .fold(f64::INFINITY, f64::min)
    };
    let z_low = low(a.points).min(low(b.points)) - BELOW_MM;
    let mean = |p: &[DVec3]| {
        let s: DVec3 = p.iter().copied().sum();
        let m = s / p.len() as f64;
        DVec2::new(m.x, m.y)
    };
    let u = (mean(b.points) - mean(a.points)).normalize_or_zero();
    if u == DVec2::ZERO {
        return None;
    }
    // across the arch, from the plane of the contact
    let ev = DVec2::new(-u.y, u.x);
    // the closed teeth may reach far under the gingiva (their roots): the
    // fill ends that far under the gingiva under the contact, where the
    // embrasure is closed by it
    let col = gum
        .iter()
        .filter(|q| DVec2::new(q.x, q.y).distance(c) < COL_MM && q.z <= z_top)
        .map(|q| q.z)
        .fold(f64::NEG_INFINITY, f64::max);
    // ... but on each side of the plane of the contact at least that far
    // under the tip of the papilla of that side ([`side_top`]): the papilla
    // of one side may stand mm lower than the gingiva under the contact
    // (the palatal papilla of an upper molar), and its embrasure stayed
    // open over it
    let bottom = |s: f64| {
        let t = side_top(gum, c, u, ev * s, z_top);
        let g = match (col.is_finite(), t.is_finite()) {
            (true, true) => col.min(t),
            (true, false) => col,
            _ => t,
        };
        if g.is_finite() {
            z_low.max(g - BELOW_MM)
        } else {
            z_low
        }
    };
    let sides = [(ev, bottom(1.0)), (-ev, bottom(-1.0))];
    let z_bot = sides[0].1.min(sides[1].1);
    if !z_bot.is_finite() {
        return None;
    }
    let mut parts: Vec<Solid> = Vec::new();
    let mut above: Option<Vec<DVec2>> = None;
    // the whole embrasure: from the top of the lower crown beside the contact,
    // each crown within RIDGE_MM of its own face towards the other (`s`: +1 for
    // `a`, which faces along `u`, -1 for `b`), so two teeth apart count too
    let ridge = |p: &[DVec3], s: f64| {
        let near: Vec<(f64, f64)> = p
            .iter()
            .filter(|q| DVec2::new(q.x, q.y).distance(c) < NEAR_MM)
            .map(|q| (s * DVec2::new(q.x, q.y).dot(u), q.z))
            .collect();
        let face = near.iter().map(|n| n.0).fold(f64::NEG_INFINITY, f64::max);
        near.iter()
            .filter(|n| face - n.0 <= RIDGE_MM)
            .map(|n| n.1)
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let z_start = if whole {
        (ridge(a.points, 1.0).min(ridge(b.points, -1.0)) - RIDGE_TOP_MM).max(z_top)
    } else {
        z_top
    };
    let mut z = z_start - STEP_MM;
    while z > z_bot - STEP_MM {
        let (sa, sb) = (section(a.points, a.tris, z), section(b.points, b.tris, z));
        let poly = if sa.len() < 3 || sb.len() < 3 {
            above.clone()
        } else {
            let xa = sa
                .iter()
                .map(|p| p.dot(u))
                .fold(f64::NEG_INFINITY, f64::max);
            let xb = sb.iter().map(|p| p.dot(u)).fold(f64::INFINITY, f64::min);
            let (lo, hi) = (xa.min(xb) - INTO_MM, xa.max(xb) + INTO_MM);
            let mut pts = sa;
            pts.extend(sb);
            if let Some(h) = &above {
                pts.extend(h.iter().copied());
            }
            let poly = clip(&clip(&hull(&pts), u, hi), -u, -lo);
            if poly.len() < 3 {
                None
            } else {
                // every slice holds the one above it
                Some(match &above {
                    Some(h) => {
                        let mut both = poly;
                        both.extend(h.iter().copied());
                        hull(&both)
                    }
                    None => poly,
                })
            }
        };
        if let Some(poly) = poly.filter(|p| p.len() >= 3) {
            let z1 = (z + 1.05 * STEP_MM).min(z_start);
            let mut faces = if inset_mm > 0.0 {
                inset(&poly, u, inset_mm)
            } else {
                poly.clone()
            };
            // under the bottom of a side only the other side goes on down
            for &(n, zb) in &sides {
                if z < zb && faces.len() >= 3 {
                    faces = clip(&faces, n, n.dot(c));
                }
            }
            if faces.len() >= 3 {
                parts.push(prism(&faces, z, z1));
            }
            above = Some(poly);
        }
        z -= STEP_MM;
    }
    if parts.is_empty() {
        return None;
    }
    let mut v = Vec::new();
    let mut t = Vec::new();
    for (pv, pt) in parts {
        let off = v.len() as u32;
        v.extend(pv);
        t.extend(pt.iter().map(|f| f.map(|i| i + off)));
    }
    Some((v, t))
}

/// The fills under the contacts of every two neighbouring teeth along the
/// arch (`teeth` in any order; the arch is the one of the first tooth), their
/// labial and lingual faces `inset_mm` inside the tangents to the two crowns
/// (`0`: on them, the printed model). `gum`: points of the gingiva (same
/// coordinates; empty: the fills end under the closed teeth).
pub fn fills(teeth: &[Tooth<'_>], gum: &[DVec3], inset_mm: f64) -> Vec<Solid> {
    fills_of(teeth, gum, inset_mm, false)
}

/// [`fills`] of the whole embrasure of every two neighbouring teeth, those
/// up to [`SPACE_MM`] apart too ([`pair_fill`] `whole`): the model the shell
/// of an aligner is formed on.
pub fn fills_whole(teeth: &[Tooth<'_>], gum: &[DVec3], inset_mm: f64) -> Vec<Solid> {
    fills_of(teeth, gum, inset_mm, true)
}

fn fills_of(teeth: &[Tooth<'_>], gum: &[DVec3], inset_mm: f64, whole: bool) -> Vec<Solid> {
    let Some(first) = teeth.first() else {
        return Vec::new();
    };
    let order: Vec<&Tooth<'_>> = arch_order(Arch::of(first.fdi))
        .iter()
        .filter_map(|f| teeth.iter().find(|t| t.fdi == *f))
        .collect();
    order
        .windows(2)
        .filter_map(|w| pair_fill(w[0], w[1], gum, inset_mm, whole))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed box `[lo, hi]`, outward faces.
    fn cuboid(lo: DVec3, hi: DVec3) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let poly = [
            DVec2::new(lo.x, lo.y),
            DVec2::new(hi.x, lo.y),
            DVec2::new(hi.x, hi.y),
            DVec2::new(lo.x, hi.y),
        ];
        prism(&poly, lo.z, hi.z)
    }

    fn volume(v: &[DVec3], t: &[[u32; 3]]) -> f64 {
        t.iter()
            .map(|f| {
                let [a, b, c] = f.map(|i| v[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn the_hull_and_the_prism_are_convex_and_closed() {
        let pts = [
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(1.0, 0.5),
            DVec2::new(2.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let h = hull(&pts);
        assert_eq!(h.len(), 4);
        let (v, t) = prism(&h, 0.0, 1.0);
        assert!((volume(&v, &t) - 4.0).abs() < 1e-9);
        let c = clip(&h, DVec2::X, 1.0);
        let (v, t) = prism(&c, 0.0, 1.0);
        assert!((volume(&v, &t) - 2.0).abs() < 1e-9);
    }

    /// Two "crowns" that touch at their top and leave a gap under it: the
    /// fill spans the gap from under the crowns up to the contact; two
    /// crowns apart are not filled.
    #[test]
    fn touching_teeth_get_a_fill_under_the_contact() {
        let f = |n: u8| Fdi::new(n).unwrap();
        let a = cuboid(DVec3::new(-4.0, -3.0, -5.0), DVec3::new(-0.3, 3.0, 0.0));
        // b is narrower at the bottom: it touches a only at its top
        let b_top = cuboid(DVec3::new(0.2, -3.0, -1.0), DVec3::new(4.0, 3.0, 0.0));
        let ta = Tooth {
            fdi: f(41),
            points: &a.0,
            tris: &a.1,
        };
        let tb = Tooth {
            fdi: f(42),
            points: &b_top.0,
            tris: &b_top.1,
        };
        let out = fills(&[tb, ta], &[], 0.0);
        assert_eq!(out.len(), 1);
        let (v, t) = &out[0];
        assert!(volume(v, t) > 0.0);
        let (lo, hi) = v.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        // across the gap, under the crowns, not above the contact
        assert!(lo.x < -0.3 && hi.x > 0.2, "{lo} {hi}");
        assert!(lo.z <= -5.0 - BELOW_MM + STEP_MM + 1e-9, "{lo}");
        assert!(hi.z <= 1e-9, "{hi}");
        // apart: no fill
        let far = cuboid(DVec3::new(2.0, -3.0, -1.0), DVec3::new(6.0, 3.0, 0.0));
        let tf = Tooth {
            fdi: f(42),
            points: &far.0,
            tris: &far.1,
        };
        assert!(fills(&[ta, tf], &[], 0.0).is_empty());
        // the gingiva under the contact limits the depth
        let gum = [DVec3::new(0.0, -3.0, -2.0)];
        let out = fills(&[ta, tb], &gum, 0.0);
        let lowest = out[0].0.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        assert!(lowest >= -2.0 - BELOW_MM - STEP_MM - 1e-9, "{lowest}");
    }

    /// Under the shell of an aligner the whole embrasure is filled: from the
    /// top of the lower crown, not from the contact, and between two teeth a
    /// little apart too (the user's rule, 2026-11-24: the aligner goes
    /// straight from one crown to the next, never between the teeth).
    #[test]
    fn the_whole_embrasure_is_filled_under_an_aligner() {
        let f = |n: u8| Fdi::new(n).unwrap();
        // each crown is a step: they touch 2 mm under their tops, over the contact
        // the embrasure is open (2 mm wide)
        let step = |x0: f64, x1: f64, x2: f64| {
            let (lo_x, hi_x) = (x0.min(x1), x0.max(x1));
            let mut m = cuboid(DVec3::new(lo_x, -3.0, -5.0), DVec3::new(hi_x, 3.0, -2.0));
            let (lo_x, hi_x) = (x0.min(x2), x0.max(x2));
            let high = cuboid(DVec3::new(lo_x, -3.0, -5.0), DVec3::new(hi_x, 3.0, 0.0));
            let off = m.0.len() as u32;
            m.0.extend(high.0.iter().copied());
            m.1.extend(high.1.iter().map(|t| t.map(|i| i + off)));
            m
        };
        let a = step(-4.0, -0.3, -1.0);
        let b = step(4.0, 0.2, 1.0);
        let ta = Tooth {
            fdi: f(41),
            points: &a.0,
            tris: &a.1,
        };
        let tb = Tooth {
            fdi: f(42),
            points: &b.0,
            tris: &b.1,
        };
        let top = |s: &[Solid]| s[0].0.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        // under the contact only: the fill ends at the contact, 2 mm under the crowns
        let under = fills(&[ta, tb], &[], 0.0);
        assert_eq!(under.len(), 1);
        assert!(top(&under) <= -2.0 + 1e-9, "{}", top(&under));
        // the whole embrasure: up to RIDGE_TOP_MM under the top of the crowns
        let whole = fills_whole(&[ta, tb], &[], 0.0);
        assert_eq!(whole.len(), 1);
        assert!(
            top(&whole) >= -RIDGE_TOP_MM - STEP_MM - 1e-9,
            "{}",
            top(&whole)
        );
        // never wider than the two crowns (the faces on their tangents)
        assert!(whole[0].0.iter().all(|p| p.y.abs() <= 3.0 + 1e-9));
        // two teeth 2.3 mm apart: bridged under an aligner, not on the printed model
        let far = cuboid(DVec3::new(2.0, -3.0, -5.0), DVec3::new(6.0, 3.0, 0.0));
        let tf = Tooth {
            fdi: f(42),
            points: &far.0,
            tris: &far.1,
        };
        assert!(fills(&[ta, tf], &[], 0.0).is_empty());
        let bridged = fills_whole(&[ta, tf], &[], 0.0);
        assert_eq!(bridged.len(), 1);
        let (lo, hi) = bridged[0].0.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        assert!(lo.x < -0.3 && hi.x > 2.0, "{lo} {hi}");
        // a missing tooth is not a space to bridge
        let gone = cuboid(DVec3::new(4.0, -3.0, -5.0), DVec3::new(8.0, 3.0, 0.0));
        let tg = Tooth {
            fdi: f(42),
            points: &gone.0,
            tris: &gone.1,
        };
        assert!(fills_whole(&[ta, tg], &[], 0.0).is_empty());
    }

    /// The fill under the shell of the smooth border stands `inset` inside
    /// the labial and lingual tangents, its ends along the arch unchanged.
    #[test]
    fn an_inset_fill_stands_inside_the_tangents() {
        let f = |n: u8| Fdi::new(n).unwrap();
        let a = cuboid(DVec3::new(-4.0, -3.0, -5.0), DVec3::new(-0.3, 3.0, 0.0));
        let b = cuboid(DVec3::new(0.2, -3.0, -1.0), DVec3::new(4.0, 3.0, 0.0));
        let teeth = [
            Tooth {
                fdi: f(41),
                points: &a.0,
                tris: &a.1,
            },
            Tooth {
                fdi: f(42),
                points: &b.0,
                tris: &b.1,
            },
        ];
        let bounds = |v: &[DVec3]| {
            v.iter().fold(
                (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                |(lo, hi), p| (lo.min(*p), hi.max(*p)),
            )
        };
        let (lo0, hi0) = bounds(&fills(&teeth, &[], 0.0)[0].0);
        let out = fills(&teeth, &[], 0.5);
        assert_eq!(out.len(), 1);
        assert!(volume(&out[0].0, &out[0].1) > 0.0);
        let (lo, hi) = bounds(&out[0].0);
        assert!((lo.y - (lo0.y + 0.5)).abs() < 1e-9, "{lo} {lo0}");
        assert!((hi.y - (hi0.y - 0.5)).abs() < 1e-9, "{hi} {hi0}");
        assert!((lo.x - lo0.x).abs() < 1e-9 && (hi.x - hi0.x).abs() < 1e-9);
    }

    /// The papilla of one side far lower than the gingiva under the contact
    /// (the palatal gingiva of an upper molar near it): the half of the fill
    /// on that side goes on down to [`BELOW_MM`] under its tip, the other
    /// half ends as before.
    #[test]
    fn a_lower_papilla_is_filled_down_on_its_own_side() {
        let f = |n: u8| Fdi::new(n).unwrap();
        let pa = [
            DVec2::new(-4.0, -3.0),
            DVec2::new(-1.0, -3.0),
            DVec2::new(-0.3, 0.0),
            DVec2::new(-1.0, 3.0),
            DVec2::new(-4.0, 3.0),
        ];
        let pb = [
            DVec2::new(0.2, 0.0),
            DVec2::new(1.0, -3.0),
            DVec2::new(4.0, -3.0),
            DVec2::new(4.0, 3.0),
            DVec2::new(1.0, 3.0),
        ];
        let a = prism(&pa, -5.0, 0.0);
        let b = prism(&pb, -1.0, 0.0);
        let teeth = [
            Tooth {
                fdi: f(26),
                points: &a.0,
                tris: &a.1,
            },
            Tooth {
                fdi: f(27),
                points: &b.0,
                tris: &b.1,
            },
        ];
        // the gingiva under the contact (-y) and the papilla of +y, 2.5 mm
        // lower
        let high = DVec3::new(0.0, -0.8, -1.5);
        let low_tip = DVec3::new(0.0, 2.0, -4.0);
        let lowest = |v: &[DVec3]| v.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        for inset in [0.0, 0.5] {
            let before = fills(&teeth, &[high], inset);
            assert!(
                (lowest(&before[0].0) - (-1.5 - BELOW_MM)).abs() <= STEP_MM + 1e-9,
                "{}",
                lowest(&before[0].0)
            );
            let out = fills(&teeth, &[high, low_tip], inset);
            assert_eq!(out.len(), 1);
            let (v, t) = &out[0];
            assert!(volume(v, t) > 0.0);
            let z = lowest(v);
            assert!((z - (-4.0 - BELOW_MM)).abs() <= STEP_MM + 1e-9, "{z}");
            // under the bottom of the side of the gingiva only the side of the
            // papilla, on to the plane of the contact
            for p in v.iter().filter(|p| p.z < -1.5 - BELOW_MM - STEP_MM) {
                assert!(p.y >= -1e-9, "{p}");
            }
            assert!(v.iter().any(|p| p.z < -5.0 && p.y > 1.0));
        }
    }
}
