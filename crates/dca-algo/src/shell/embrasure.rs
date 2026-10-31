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
//! out of it); the
//! draft and the recess of the smooth border and the contact bridges of
//! `contact_bridges` (`pair_bridge`, the narrow slots between teeth that do
//! not touch) are not ported.

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

/// The fill under the contact of two neighbouring teeth `a`, `b`; `None`
/// when they do not touch.
fn pair_fill(a: &Tooth<'_>, b: &Tooth<'_>, gum: &[DVec3]) -> Option<Solid> {
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
    let (lo, hi) = (lo - DVec3::splat(TOUCH_MM), hi + DVec3::splat(TOUCH_MM));
    let cand: Vec<usize> = (0..b.points.len())
        .filter(|&i| b.points[i].cmpgt(lo).all() && b.points[i].cmplt(hi).all())
        .collect();
    if cand.is_empty() {
        return None;
    }
    let pa: Vec<Vec3> = a.points.iter().map(|p| p.as_vec3()).collect();
    let index = KnnIndex::build(&pa).ok()?;
    let mut zone = Vec::new();
    for &i in &cand {
        if let Some(n) = index.nearest(b.points[i].as_vec3())
            && f64::from(n.distance) <= TOUCH_MM
        {
            zone.push(b.points[i]);
            zone.push(a.points[n.index as usize]);
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
    let mut z_bot = low(a.points).min(low(b.points)) - BELOW_MM;
    // the closed teeth may reach far under the gingiva (their roots): the
    // fill ends that far under the gingiva under the contact, where the
    // embrasure is closed by it
    let col = gum
        .iter()
        .filter(|q| DVec2::new(q.x, q.y).distance(c) < COL_MM && q.z <= z_top)
        .map(|q| q.z)
        .fold(f64::NEG_INFINITY, f64::max);
    if col.is_finite() {
        z_bot = z_bot.max(col - BELOW_MM);
    }
    if !z_bot.is_finite() {
        return None;
    }
    let mean = |p: &[DVec3]| {
        let s: DVec3 = p.iter().copied().sum();
        let m = s / p.len() as f64;
        DVec2::new(m.x, m.y)
    };
    let u = (mean(b.points) - mean(a.points)).normalize_or_zero();
    if u == DVec2::ZERO {
        return None;
    }
    let mut parts: Vec<Solid> = Vec::new();
    let mut above: Option<Vec<DVec2>> = None;
    let mut z = z_top - STEP_MM;
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
            let z1 = (z + 1.05 * STEP_MM).min(z_top);
            parts.push(prism(&poly, z, z1));
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
/// arch (`teeth` in any order; the arch is the one of the first tooth).
/// `gum`: points of the gingiva (same coordinates; empty: the fills end
/// under the closed teeth).
pub fn fills(teeth: &[Tooth<'_>], gum: &[DVec3]) -> Vec<Solid> {
    let Some(first) = teeth.first() else {
        return Vec::new();
    };
    let order: Vec<&Tooth<'_>> = arch_order(Arch::of(first.fdi))
        .iter()
        .filter_map(|f| teeth.iter().find(|t| t.fdi == *f))
        .collect();
    order
        .windows(2)
        .filter_map(|w| pair_fill(w[0], w[1], gum))
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
        let out = fills(&[tb, ta], &[]);
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
        assert!(fills(&[ta, tf], &[]).is_empty());
        // the gingiva under the contact limits the depth
        let gum = [DVec3::new(0.0, -3.0, -2.0)];
        let out = fills(&[ta, tb], &gum);
        let lowest = out[0].0.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        assert!(lowest >= -2.0 - BELOW_MM - STEP_MM - 1e-9, "{lowest}");
    }
}
