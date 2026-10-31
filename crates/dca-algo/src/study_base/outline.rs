//! The plan outline of a study-model base: a convex polygon built as the
//! intersection of named half-planes `n · p ≤ c` in the common occlusion
//! frame (`x` lateral towards the patient's right, `y` anterior along the
//! raphe).

use glam::DVec2;
use serde::Serialize;

/// One wall of the base seen from above: the half-plane `n · p ≤ c`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HalfPlane {
    /// Name of the wall (`back`, `heel_r`, `side_l`, `front_r`, `arc_3` ...).
    pub name: String,
    /// Outward unit normal (plan).
    #[serde(serialize_with = "ser_vec2")]
    pub n: DVec2,
    /// Offset along `n` (mm).
    pub c: f64,
}

fn ser_vec2<S: serde::Serializer>(v: &DVec2, s: S) -> Result<S::Ok, S::Error> {
    [v.x, v.y].serialize(s)
}

impl HalfPlane {
    /// The wall `name` whose outward normal makes `psi_deg` with `−y`,
    /// turned towards `+x` (`side = 1`, patient's right) or `−x` (`side = −1`).
    pub fn at_angle(name: impl Into<String>, psi_deg: f64, side: f64) -> Self {
        let psi = psi_deg.to_radians();
        Self {
            name: name.into(),
            n: DVec2::new(side * psi.sin(), -psi.cos()),
            c: f64::NEG_INFINITY,
        }
    }

    /// Pushes the wall out so that every point of `pts` lies at least
    /// `clear` inside it.
    pub fn clear_points(&mut self, pts: &[DVec2], clear: f64) {
        for p in pts {
            self.c = self.c.max(self.n.dot(*p) + clear);
        }
    }

    /// Pushes the wall out to at least the point `p`.
    pub fn through(&mut self, p: DVec2) {
        self.c = self.c.max(self.n.dot(p));
    }

    /// Signed distance of `p` beyond the wall (negative inside).
    pub fn distance(&self, p: DVec2) -> f64 {
        self.n.dot(p) - self.c
    }

    /// Intersection of the two wall lines (`None` when parallel).
    pub fn meet(&self, other: &HalfPlane) -> Option<DVec2> {
        let det = self.n.x * other.n.y - self.n.y * other.n.x;
        if det.abs() < 1e-12 {
            return None;
        }
        Some(DVec2::new(
            (self.c * other.n.y - self.n.y * other.c) / det,
            (self.n.x * other.c - self.c * other.n.x) / det,
        ))
    }
}

/// A convex outline: its vertices counter-clockwise and, per vertex, the
/// wall of the edge that starts there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outline {
    /// Vertices (plan, mm), counter-clockwise.
    #[serde(serialize_with = "ser_points")]
    pub points: Vec<DVec2>,
    /// Index into `walls` of the edge `points[i] → points[i + 1]`.
    pub edge_wall: Vec<usize>,
    /// The walls, in the order they were applied.
    pub walls: Vec<HalfPlane>,
}

fn ser_points<S: serde::Serializer>(v: &[DVec2], s: S) -> Result<S::Ok, S::Error> {
    v.iter()
        .map(|p| [p.x, p.y])
        .collect::<Vec<_>>()
        .serialize(s)
}

/// Half size of the square the walls clip (mm).
const WORLD: f64 = 1000.0;

impl Outline {
    /// The polygon of the walls (Sutherland–Hodgman clipping of a large
    /// square, each edge labelled with the wall that made it).
    pub fn new(walls: Vec<HalfPlane>) -> Self {
        const NONE: usize = usize::MAX;
        let mut poly: Vec<(DVec2, usize)> = vec![
            (DVec2::new(-WORLD, -WORLD), NONE),
            (DVec2::new(WORLD, -WORLD), NONE),
            (DVec2::new(WORLD, WORLD), NONE),
            (DVec2::new(-WORLD, WORLD), NONE),
        ];
        for (k, w) in walls.iter().enumerate() {
            if !w.c.is_finite() {
                continue;
            }
            let mut out = Vec::with_capacity(poly.len() + 1);
            for i in 0..poly.len() {
                let (a, la) = poly[i];
                let (b, _) = poly[(i + 1) % poly.len()];
                let (da, db) = (w.distance(a), w.distance(b));
                let cross = |da: f64, db: f64| a + (b - a) * (da / (da - db));
                match (da <= 0.0, db <= 0.0) {
                    (true, true) => out.push((a, la)),
                    (true, false) => {
                        out.push((a, la));
                        out.push((cross(da, db), k));
                    }
                    (false, true) => out.push((cross(da, db), la)),
                    (false, false) => {}
                }
            }
            poly = out;
        }
        // drop edges shorter than a micron (a wall that only touches a corner)
        let mut clean: Vec<(DVec2, usize)> = Vec::with_capacity(poly.len());
        for (p, l) in poly {
            if let Some(last) = clean.last_mut()
                && last.0.distance(p) < 1e-6
            {
                last.1 = l;
                continue;
            }
            clean.push((p, l));
        }
        if clean.len() > 1 && clean[0].0.distance(clean[clean.len() - 1].0) < 1e-6 {
            clean.pop();
        }
        Self {
            points: clean.iter().map(|(p, _)| *p).collect(),
            edge_wall: clean.iter().map(|(_, l)| *l).collect(),
            walls,
        }
    }

    /// `true` when `p` lies at least `clear` inside every wall.
    pub fn contains(&self, p: DVec2, clear: f64) -> bool {
        self.walls
            .iter()
            .filter(|w| w.c.is_finite())
            .all(|w| w.distance(p) <= -clear + 1e-9)
    }

    /// The wall named `name`, when it makes an edge of the polygon.
    pub fn edge(&self, name: &str) -> Option<&HalfPlane> {
        self.edge_wall
            .iter()
            .filter(|&&k| k != usize::MAX)
            .map(|&k| &self.walls[k])
            .find(|w| w.name == name)
    }

    /// Length of the edge of the wall `name` (0 when it is not an edge).
    pub fn edge_length(&self, name: &str) -> f64 {
        (0..self.points.len())
            .filter(|&i| {
                self.edge_wall[i] != usize::MAX && self.walls[self.edge_wall[i]].name == name
            })
            .map(|i| self.points[i].distance(self.points[(i + 1) % self.points.len()]))
            .sum()
    }

    /// Interior angle (degrees) between the edges of two walls:
    /// `180° − ∠(n_a, n_b)`.
    pub fn interior_deg(&self, a: &str, b: &str) -> Option<f64> {
        let (a, b) = (self.edge(a)?, self.edge(b)?);
        Some(180.0 - a.n.angle_to(b.n).abs().to_degrees())
    }

    /// Signed area (positive counter-clockwise).
    pub fn area(&self) -> f64 {
        let n = self.points.len();
        (0..n)
            .map(|i| self.points[i].perp_dot(self.points[(i + 1) % n]))
            .sum::<f64>()
            * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_of_four_walls() {
        let mut walls = Vec::new();
        for (name, psi) in [("back", 0.0), ("front", 180.0)] {
            let mut w = HalfPlane::at_angle(name, psi, 1.0);
            w.c = 10.0;
            walls.push(w);
        }
        for (name, side) in [("side_r", 1.0), ("side_l", -1.0)] {
            let mut w = HalfPlane::at_angle(name, 90.0, side);
            w.c = 10.0;
            walls.push(w);
        }
        let o = Outline::new(walls);
        assert_eq!(o.points.len(), 4);
        assert!((o.area() - 400.0).abs() < 1e-9);
        assert!((o.interior_deg("back", "side_r").unwrap() - 90.0).abs() < 1e-9);
        assert!((o.edge_length("front") - 20.0).abs() < 1e-9);
        assert!(o.contains(DVec2::new(9.0, -9.0), 0.5));
        assert!(!o.contains(DVec2::new(9.8, 0.0), 0.5));
    }

    #[test]
    fn a_wall_beyond_the_polygon_makes_no_edge() {
        let mut a = HalfPlane::at_angle("back", 0.0, 1.0);
        a.c = 5.0;
        let mut b = HalfPlane::at_angle("front", 180.0, 1.0);
        b.c = 5.0;
        let mut far = HalfPlane::at_angle("far", 90.0, 1.0);
        far.c = 2000.0;
        let o = Outline::new(vec![a, b, far]);
        assert!(o.edge("far").is_none());
        assert!(o.edge("back").is_some());
    }
}
