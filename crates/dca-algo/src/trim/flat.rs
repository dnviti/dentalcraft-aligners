//! The trim surface of a border taken from the gingival margins (STRAIGHT,
//! SCALLOPED, SMOOTH) held under the walls of the shell it would slice.
//!
//! The border lies a fixed height above (SMOOTH) or below the margin of
//! every tooth, so the trim surface, a height field, meets the walls of
//! the shell where they stand: on the sides of the crowns and on the
//! gingiva it cuts across them, a rim the rounding finishes. Where a wall
//! lies flat at that height and about parallel to the surface (the top of
//! a crown whose label stops short of its gingival margin, so that the
//! margin taken from it runs over the crown) the surface runs along inside
//! the wall instead: what it leaves is a sliver thinner than the wall,
//! perforated where the surface rises over the wall and broken into a lace
//! by the rounding of the rim, holes through the wall that no fill closes
//! (a real upper arch: three to five on every aligner, one left after the
//! plugs).
//!
//! [`under_flat_walls`] reads, on every node of the trim surface, the first
//! wall of the shell met going down its vertical, and whether the surface
//! lies inside it and the wall is flat (at most [`FLAT_EXTENT`] times its
//! thickness deep along the vertical: it leans at most 60 degrees). Across
//! a wall the nodes where the surface lies inside it make a narrow stretch,
//! as wide as the wall over the angle between them (1.3 mm for a gingiva
//! leaning 51 degrees under a level surface): the rim, left as it is. Along
//! a wall they make a wide one: every stretch with a node farther than
//! [`SLICE_MM`] from its edge is a slice, and there the surface goes down
//! to [`FLAT_CLEAR_VOXELS`] voxels under the wall: the wall is kept whole
//! and cut across where the surface comes back up beside it. A node whose
//! first wall lies wholly under the surface, enclosed by nodes where the
//! wall is kept (a window through it), is held under its wall the same way.

use std::collections::VecDeque;

use dc_geom::sdf::SdfGrid;
use glam::{DVec2, Vec3};
use rayon::prelude::*;

use super::grid::PlanGrid;

/// A wall at most this many times its thickness deep along the vertical is
/// flat (it leans at most 60 degrees: a sheet of thickness `t` at the angle
/// `a` is `t / cos a` deep).
pub const FLAT_EXTENT: f64 = 2.0;
/// The trim surface is held this many voxels of the shell under a flat wall.
pub const FLAT_CLEAR_VOXELS: f64 = 2.0;
/// A stretch of nodes where the trim surface lies inside a flat wall,
/// reaching farther than this (mm, plan) from its edge, is a slice along the
/// wall (twice this: the stretch across a 1 mm wall at 23 degrees to it).
pub const SLICE_MM: f64 = 1.2;

/// Steps of [`crossing`].
const BISECT_STEPS: usize = 32;

/// The height of the zero level of `at` between `inside` (negative) and
/// `outside` (not negative).
fn crossing(at: &impl Fn(f64) -> f64, inside: f64, outside: f64) -> f64 {
    let (mut a, mut b) = (inside, outside);
    for _ in 0..BISECT_STEPS {
        let m = 0.5 * (a + b);
        if at(m) < 0.0 {
            a = m;
        } else {
            b = m;
        }
    }
    0.5 * (a + b)
}

/// The first wall of `shell` met going down the vertical through `p` from
/// `from` (above the shell), its top not under `floor`: its top, and its
/// bottom unless the wall is deeper than `extent` along the vertical.
/// `None` without such a wall (or when `from` is inside one).
pub fn first_wall(
    shell: &SdfGrid,
    p: DVec2,
    from: f64,
    floor: f64,
    extent: f64,
) -> Option<(f64, Option<f64>)> {
    // the field is at most as large as the distance to its zero level: steps
    // of half the value never pass a wall, and at least a quarter of a voxel
    let h = 0.25 * f64::from(shell.voxel_size);
    let at = |z: f64| f64::from(shell.sample(Vec3::new(p.x as f32, p.y as f32, z as f32)));
    let (mut z, mut v) = (from, at(from));
    if v.is_nan() || v < 0.0 {
        return None;
    }
    let top = loop {
        let next = z - (0.5 * v).max(h);
        if next < floor {
            return None;
        }
        let w = at(next);
        if w < 0.0 {
            break crossing(&at, next, z);
        }
        (z, v) = (next, w);
    };
    // down through the wall
    let mut z = top;
    let mut v = at(z).min(0.0);
    loop {
        let next = z - (0.5 * -v).max(h);
        if top - next > extent {
            return Some((top, None));
        }
        let w = at(next);
        if w >= 0.0 {
            return Some((top, Some(crossing(&at, z.min(top - 1e-9), next))));
        }
        (z, v) = (next, w);
    }
}

/// The trim surface at a node against the first wall under it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Node {
    /// Over that wall (or no wall at all): nothing of it kept.
    Open,
    /// Inside a flat wall, whose bottom is given.
    Inside(f64),
    /// Under the wall, or inside a steep one.
    Held,
}

/// The nodes beside node `k` of an `nx` x `ny` grid (4 neighbours).
fn beside(k: usize, nx: usize, ny: usize) -> impl Iterator<Item = usize> {
    let (i, j) = (k / ny, k % ny);
    [
        (i > 0).then(|| k - ny),
        (i + 1 < nx).then(|| k + ny),
        (j > 0).then(|| k - 1),
        (j + 1 < ny).then(|| k + 1),
    ]
    .into_iter()
    .flatten()
}

/// Holds the trim surface `z` of the nodes of `grid` under the flat walls of
/// the untrimmed `shell` it would slice, and under the walls of the windows
/// it would open (see the module docs). `wall`: the thickness of the wall;
/// `from`: a height above the shell; `floor`: a height under it. Returns the
/// nodes lowered.
pub fn under_flat_walls(
    grid: &PlanGrid,
    z: &mut [f64],
    shell: &SdfGrid,
    wall: f64,
    from: f64,
    floor: f64,
) -> usize {
    let (nx, ny, n) = (grid.nx, grid.ny, grid.len());
    if n == 0 {
        return 0;
    }
    let clear = FLAT_CLEAR_VOXELS * f64::from(shell.voxel_size);
    let extent = FLAT_EXTENT * wall;
    let nodes: Vec<Node> = (0..n)
        .into_par_iter()
        .map(|k| {
            let f = z[k];
            if !f.is_finite() {
                return Node::Held;
            }
            match first_wall(shell, grid.node(k), from, f, extent) {
                None => Node::Open,
                Some((_, Some(bottom))) if bottom < f => Node::Inside(bottom),
                Some(_) => Node::Held,
            }
        })
        .collect();
    let inside = |k: usize| matches!(nodes[k], Node::Inside(_));
    // how far (in nodes) every node inside a wall is from the edge of its
    // stretch: 1 next to a node that is not, or on the border of the grid
    let mut depth = vec![0usize; n];
    let mut queue = VecDeque::new();
    for k in (0..n).filter(|&k| inside(k)) {
        let (i, j) = (k / ny, k % ny);
        let edge = i == 0 || j == 0 || i + 1 == nx || j + 1 == ny;
        if edge || beside(k, nx, ny).any(|m| !inside(m)) {
            depth[k] = 1;
            queue.push_back(k);
        }
    }
    while let Some(k) = queue.pop_front() {
        for m in beside(k, nx, ny) {
            if inside(m) && depth[m] == 0 {
                depth[m] = depth[k] + 1;
                queue.push_back(m);
            }
        }
    }
    // the stretches reaching SLICE_MM from their edge: slices, held under the wall
    let dy = if ny > 1 {
        (grid.hi.y - grid.lo.y) / (ny - 1) as f64
    } else {
        0.0
    };
    let cell = grid.dx().max(dy);
    let mut seen = vec![false; n];
    let mut lowered = 0;
    for start in 0..n {
        if !inside(start) || seen[start] {
            continue;
        }
        let mut stretch = vec![start];
        seen[start] = true;
        let mut at = 0;
        while at < stretch.len() {
            let k = stretch[at];
            at += 1;
            for m in beside(k, nx, ny) {
                if inside(m) && !seen[m] {
                    seen[m] = true;
                    stretch.push(m);
                }
            }
        }
        let deepest = stretch.iter().map(|&k| depth[k]).max().unwrap_or(0);
        if (deepest as f64) * cell + 1e-9 < SLICE_MM {
            continue;
        }
        for k in stretch {
            if let Node::Inside(bottom) = nodes[k] {
                z[k] = z[k].min(bottom - clear);
                lowered += 1;
            }
        }
    }
    // the open nodes reached from the border of the grid through open nodes
    let open = |k: usize| nodes[k] == Node::Open;
    let mut reached = vec![false; n];
    for (k, r) in reached.iter_mut().enumerate() {
        let (i, j) = (k / ny, k % ny);
        if (i == 0 || j == 0 || i + 1 == nx || j + 1 == ny) && open(k) {
            *r = true;
            queue.push_back(k);
        }
    }
    while let Some(k) = queue.pop_front() {
        for m in beside(k, nx, ny) {
            if open(m) && !reached[m] {
                reached[m] = true;
                queue.push_back(m);
            }
        }
    }
    // windows: open nodes enclosed by nodes where the wall is kept
    let windows: Vec<usize> = (0..n).filter(|&k| open(k) && !reached[k]).collect();
    let down: Vec<(usize, Option<f64>)> = windows
        .par_iter()
        .map(|&k| {
            let w = first_wall(shell, grid.node(k), from, floor, f64::INFINITY);
            (k, w.and_then(|(_, bottom)| bottom).map(|b| b - clear))
        })
        .collect();
    for (k, d) in down {
        if let Some(d) = d {
            z[k] = z[k].min(d);
            lowered += 1;
        }
    }
    lowered
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_geom::sdf::SdfParams;
    use dc_mesh::TriMesh;

    /// A closed box from `lo` to `hi`, outward faces.
    fn cuboid(lo: Vec3, hi: Vec3) -> TriMesh {
        let p = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { hi.x } else { lo.x },
                if y { hi.y } else { lo.y },
                if z { hi.z } else { lo.z },
            )
        };
        let positions = vec![
            p(false, false, false),
            p(true, false, false),
            p(true, true, false),
            p(false, true, false),
            p(false, false, true),
            p(true, false, true),
            p(true, true, true),
            p(false, true, true),
        ];
        let tris = vec![
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
        TriMesh::new(positions, tris)
    }

    /// The shell of `wall` on a block 10 x 10 x 6 mm, its top at `z = 0`.
    fn block_shell(wall: f64) -> SdfGrid {
        let params = SdfParams {
            voxel_size: 0.1,
            band: wall as f32 + 0.5,
        };
        let block = cuboid(Vec3::new(-5.0, -5.0, -6.0), Vec3::new(5.0, 5.0, 0.0));
        SdfGrid::from_mesh(&block, &params)
            .unwrap()
            .shell(0.0, wall as f32)
            .unwrap()
    }

    #[test]
    fn the_first_wall_down_a_vertical() {
        let shell = block_shell(1.0);
        // over the top: the flat wall from 1 down to 0
        let (top, bottom) = first_wall(&shell, DVec2::new(1.0, 2.0), 5.0, -20.0, 2.0).unwrap();
        assert!((top - 1.0).abs() < 0.02, "{top}");
        assert!((bottom.unwrap() - 0.0).abs() < 0.02, "{bottom:?}");
        // in the side wall, under the rounded edge (`z = sqrt(1 - 0.5^2)`):
        // deeper than twice the wall along the vertical
        let (top, bottom) = first_wall(&shell, DVec2::new(5.5, 0.0), 5.0, -20.0, 2.0).unwrap();
        assert!((top - 0.75f64.sqrt()).abs() < 0.03, "{top}");
        assert_eq!(bottom, None);
        // beside the block, and a top under the floor: none
        assert_eq!(
            first_wall(&shell, DVec2::new(8.0, 0.0), 5.0, -20.0, 2.0),
            None
        );
        assert_eq!(
            first_wall(&shell, DVec2::new(0.0, 0.0), 5.0, 1.5, 2.0),
            None
        );
    }

    #[test]
    fn a_surface_in_a_flat_wall_goes_under_it_and_a_window_closes() {
        let shell = block_shell(1.0);
        let grid = PlanGrid {
            lo: DVec2::splat(-8.0),
            hi: DVec2::splat(8.0),
            nx: 41,
            ny: 41,
        };
        // inside the top wall over half the block, over it (a window) on a
        // patch in the middle of that half, under it elsewhere
        let mut z: Vec<f64> = (0..grid.len())
            .map(|k| {
                let p = grid.node(k);
                if p.x.abs() < 1.0 && p.y.abs() < 1.0 {
                    1.4
                } else if p.x < 3.0 && p.x.abs() < 4.0 && p.y.abs() < 4.0 {
                    0.5
                } else {
                    -3.0
                }
            })
            .collect();
        let before = z.clone();
        let n = under_flat_walls(&grid, &mut z, &shell, 1.0, 5.0, -20.0);
        assert!(n > 0);
        for k in 0..grid.len() {
            let p = grid.node(k);
            if p.x.abs() < 4.0 && p.y.abs() < 4.0 && p.x < 3.0 {
                // the wall is kept whole: the surface 0.2 mm under it
                assert!((z[k] - -0.2).abs() < 0.03, "{p}: {}", z[k]);
            } else {
                assert_eq!(z[k], before[k], "{p}");
            }
        }
        // a narrow stretch inside the wall (a rim across it): left as it is
        let strip = |p: DVec2| (p.x - 1.0).abs() < 0.5 && p.y.abs() < 4.0;
        let mut z: Vec<f64> = (0..grid.len())
            .map(|k| if strip(grid.node(k)) { 0.5 } else { -3.0 })
            .collect();
        assert!(z.iter().filter(|&&v| v == 0.5).count() > 20);
        assert_eq!(under_flat_walls(&grid, &mut z, &shell, 1.0, 5.0, -20.0), 0);
        // a surface over the wall that reaches the border of the grid: no window
        let mut z = vec![1.4; grid.len()];
        assert_eq!(under_flat_walls(&grid, &mut z, &shell, 1.0, 5.0, -20.0), 0);
        assert!(z.iter().all(|&v| v == 1.4));
    }
}
