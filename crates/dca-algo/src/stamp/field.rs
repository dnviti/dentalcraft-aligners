//! The text as a 2D signed distance field: the distance from a point of the
//! plane of the text to the outline of the glyphs, negative inside the ink.
//!
//! Extruded along the surface normals this is the signed distance of the
//! stamp's solid, which `stamp.rs` evaluates on the vertices of the (refined)
//! surface instead of sampling a volume.

use glam::DVec2;

use super::text::TextShape;

/// Bucket grid over the plane of the text.
pub(super) struct TextField {
    tris: Vec<[DVec2; 3]>,
    segments: Vec<[DVec2; 2]>,
    reach: f64,
    cell: f64,
    lo: DVec2,
    nx: usize,
    ny: usize,
    tri_cells: Vec<Vec<u32>>,
    seg_cells: Vec<Vec<u32>>,
}

fn bbox(points: &[DVec2]) -> (DVec2, DVec2) {
    points.iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

fn distance_to_segment(p: DVec2, [a, b]: [DVec2; 2]) -> f64 {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 > 0.0 {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + t * ab)
}

fn in_triangle(p: DVec2, [a, b, c]: [DVec2; 3]) -> bool {
    let s = |u: DVec2, v: DVec2, w: DVec2| (v - u).perp_dot(w - u);
    // counter-clockwise triangles: the point is left of every edge
    s(a, b, p) >= -1e-12 && s(b, c, p) >= -1e-12 && s(c, a, p) >= -1e-12
}

impl TextField {
    /// The field of `shape`; distances are exact up to `reach` (mm) from the
    /// outline and clamped to it beyond.
    pub fn new(shape: &TextShape, reach: f64) -> Self {
        let tris: Vec<[DVec2; 3]> = shape
            .tris
            .iter()
            .map(|t| t.map(|i| shape.points[i as usize]))
            .collect();
        // the outline: edges used by one triangle only
        let mut count: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::new();
        for t in &shape.tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let segments: Vec<[DVec2; 2]> = count
            .into_iter()
            .filter(|(_, n)| *n == 1)
            .map(|((a, b), _)| [shape.points[a as usize], shape.points[b as usize]])
            .collect();
        let (lo, hi) = bbox(&shape.points);
        let cell = (4.0 * reach).max(0.05);
        let lo = lo - DVec2::splat(reach + cell);
        let nx = ((hi.x + reach + cell - lo.x) / cell).ceil() as usize + 1;
        let ny = ((hi.y + reach + cell - lo.y) / cell).ceil() as usize + 1;
        let mut field = Self {
            tris,
            segments,
            reach,
            cell,
            lo,
            nx,
            ny,
            tri_cells: vec![Vec::new(); nx * ny],
            seg_cells: vec![Vec::new(); nx * ny],
        };
        for (i, t) in field.tris.clone().iter().enumerate() {
            let (a, b) = bbox(t);
            field.insert(true, i as u32, a, b);
        }
        for (i, s) in field.segments.clone().iter().enumerate() {
            let (a, b) = bbox(s);
            field.insert(
                false,
                i as u32,
                a - DVec2::splat(reach),
                b + DVec2::splat(reach),
            );
        }
        field
    }

    fn cell_of(&self, p: DVec2) -> (usize, usize) {
        let c = (p - self.lo) / self.cell;
        (
            (c.x.floor().max(0.0) as usize).min(self.nx - 1),
            (c.y.floor().max(0.0) as usize).min(self.ny - 1),
        )
    }

    fn insert(&mut self, tri: bool, id: u32, a: DVec2, b: DVec2) {
        let ((x0, y0), (x1, y1)) = (self.cell_of(a), self.cell_of(b));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let list = if tri {
                    &mut self.tri_cells
                } else {
                    &mut self.seg_cells
                };
                list[y * self.nx + x].push(id);
            }
        }
    }

    /// `true` when `p` is inside the ink.
    pub fn inside(&self, p: DVec2) -> bool {
        let c = (p - self.lo) / self.cell;
        if c.x < 0.0 || c.y < 0.0 || c.x >= self.nx as f64 || c.y >= self.ny as f64 {
            return false;
        }
        let (x, y) = self.cell_of(p);
        self.tri_cells[y * self.nx + x]
            .iter()
            .any(|&t| in_triangle(p, self.tris[t as usize]))
    }

    /// Signed distance to the outline (negative inside), clamped to `±reach`.
    pub fn signed_distance(&self, p: DVec2) -> f64 {
        let c = (p - self.lo) / self.cell;
        if c.x < 0.0 || c.y < 0.0 || c.x >= self.nx as f64 || c.y >= self.ny as f64 {
            return self.reach;
        }
        let (x, y) = self.cell_of(p);
        let d = self.seg_cells[y * self.nx + x]
            .iter()
            .map(|&s| distance_to_segment(p, self.segments[s as usize]))
            .fold(self.reach, f64::min);
        if self.inside(p) { -d } else { d }
    }
}
