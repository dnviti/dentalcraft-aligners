//! Rigid removal of an aligner (`case_print.removal_lock`, `rigid_lock`):
//! the shell taken off the teeth in one piece along a straight path. On
//! lines along the path through a grid of the plan, a layer of the shell
//! whose top lies more than [`TOL_MM`] under a surface of a closed tooth
//! hits that tooth when lifted: the line is locked. The locked area, the
//! part of it deeper than [`DEEP_MM`] across the path and the deepest
//! reach tell whether the shell comes off rigid (VERTICAL and COMMON axes:
//! along their one path) or has to flex (PER_TOOTH: along the least locked
//! of a few rigid paths around the mean path of the teeth).

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use rayon::prelude::*;
use serde::Serialize;

use super::warp::tilted;

/// The lines of a path are this far apart (mm, `REMOVAL_GRID_MM`) ...
pub const GRID_MM: f64 = 0.1;
/// ... a layer of the shell more than this under a tooth surface is locked
/// (mm, `REMOVAL_TOL_MM`) ...
pub const TOL_MM: f64 = 0.1;
/// ... engaged where the locked cells reach deeper than this across the
/// path (mm, `REMOVAL_DEEP_MM`).
pub const DEEP_MM: f64 = 0.25;
/// Surfaces met along a line, of the shell and of the teeth each
/// (`REMOVAL_HITS`).
pub const HITS: usize = 8;
/// PER_TOOTH: the paths of the check lean this much towards the mean path
/// of the teeth (degrees, `REMOVAL_CHECK_TILTS_DEG`) ...
pub const CHECK_TILTS_DEG: [f64; 4] = [10.0, 15.0, 20.0, 25.0];
/// ... measured on lines this far apart (mm, `REMOVAL_CHECK_GRID_MM`; the
/// paths are surveyed on twice this).
pub const CHECK_GRID_MM: f64 = 0.15;
/// Locked more than this along the best rigid path found: reported (mm²,
/// `REMOVAL_WARN_MM2`).
pub const WARN_MM2: f64 = 3.0;
/// Two hits along a line closer than this are one (mm, the plugin's ray
/// gap).
const HIT_GAP_MM: f32 = 1e-4;

/// The lock of a shell taken off along a path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Lock {
    /// Locked area (mm², to 0.01).
    pub area_mm2: f64,
    /// The part of it deeper than [`DEEP_MM`] across the path (mm²).
    pub deep_mm2: f64,
    /// Deepest reach of the locked cells across the path (mm).
    pub deepest_mm: f64,
}

/// The rigid removal of a shell checked for the panel: the lock along the
/// path and the path (its shear: `(x, y)` per unit of height).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Removal {
    /// The lock.
    pub lock: Lock,
    /// The path.
    pub shear: [f64; 2],
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round_ties_even() / 100.0
}

/// numpy's float `arange(start, stop, step)`: the first two values, then
/// `start + i * (a[1] - a[0])`.
pub fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    let delta = (start + step) - start;
    (0..n)
        .map(|i| match i {
            0 => start,
            1 => start + step,
            _ => start + i as f64 * delta,
        })
        .collect()
}

/// The heights along a line of the hits of `bvh` (at most [`HITS`]).
fn heights(bvh: &Bvh, o: Vec3, d: Vec3, z0: f64, dz: f64) -> Vec<(f64, u32)> {
    bvh.ray_hits(o, d, f32::INFINITY, HITS, HIT_GAP_MM)
        .iter()
        .map(|h| (z0 - f64::from(h.t) * dz, h.face))
        .collect()
}

/// The rigid removal of the shell `mesh` (arch coordinates) off the closed
/// teeth `teeth` along every path `(shear, 1)` of `shears`, on lines `grid`
/// mm apart (`_removal_lock`).
pub fn removal_lock(
    mesh: &TriMesh,
    shell: &Bvh,
    teeth: Option<&Bvh>,
    shears: &[DVec2],
    grid: f64,
) -> Vec<Lock> {
    let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    if v.is_empty() {
        return shears
            .iter()
            .map(|_| Lock {
                area_mm2: 0.0,
                deep_mm2: 0.0,
                deepest_mm: 0.0,
            })
            .collect();
    }
    let normal = |f: u32| {
        let [a, b, c] = mesh.tris[f as usize].map(|i| v[i as usize]);
        let cr = (b - a).cross(c - a);
        let len = (cr.x * cr.x + cr.y * cr.y + cr.z * cr.z).sqrt();
        cr / if len > 0.0 { len } else { 1.0 }
    };
    let ztop = v.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
    shears
        .iter()
        .map(|&sh| {
            // the plan of the sheared space (the path vertical there)
            let (lo, hi) = v.iter().fold(
                (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
                |(lo, hi), p| {
                    let u = p.truncate() - p.z * sh;
                    (lo.min(u), hi.max(u))
                },
            );
            let xs = arange(lo.x, hi.x + grid, grid);
            let ys = arange(lo.y, hi.y + grid, grid);
            let (nx, ny) = (xs.len(), ys.len());
            let z0 = ztop + 5.0;
            let len = (sh.x * sh.x + sh.y * sh.y + 1.0).sqrt();
            let d = -DVec3::new(sh.x, sh.y, 1.0) / len;
            let dz = -d.z;
            let df = d.as_vec3();
            let locked: Vec<bool> = (0..nx * ny)
                .into_par_iter()
                .map(|k| {
                    let g = DVec2::new(xs[k / ny], ys[k % ny]);
                    let o = (g + z0 * sh).extend(z0).as_vec3();
                    let tz: Vec<f64> = teeth.map_or_else(Vec::new, |t| {
                        heights(t, o, df, z0, dz)
                            .into_iter()
                            .map(|(z, _)| z)
                            .collect()
                    });
                    if tz.is_empty() {
                        return false;
                    }
                    heights(shell, o, df, z0, dz).into_iter().any(|(zs, f)| {
                        // the top of a layer (a face facing up the path) under a tooth surface
                        normal(f).dot(-d) > 0.0 && tz.iter().any(|&z| z > zs + TOL_MM)
                    })
                })
                .collect();
            let area = locked.iter().filter(|x| **x).count() as f64 * grid * grid;
            // erosions of the locked cells: how deep across the path
            let mut m = locked;
            let mut deep = vec![false; m.len()];
            let mut n = 0usize;
            let at = |m: &[bool], i: isize, j: isize| {
                i >= 0
                    && j >= 0
                    && (i as usize) < nx
                    && (j as usize) < ny
                    && m[i as usize * ny + j as usize]
            };
            while m.iter().any(|x| *x) && n < 200 {
                n += 1;
                m = (0..nx * ny)
                    .map(|k| {
                        let (i, j) = ((k / ny) as isize, (k % ny) as isize);
                        m[k] && at(&m, i - 1, j)
                            && at(&m, i + 1, j)
                            && at(&m, i, j - 1)
                            && at(&m, i, j + 1)
                    })
                    .collect();
                if n as f64 * grid > DEEP_MM {
                    for (dk, mk) in deep.iter_mut().zip(&m) {
                        *dk |= *mk;
                    }
                }
            }
            Lock {
                area_mm2: round2(area),
                deep_mm2: round2(deep.iter().filter(|x| **x).count() as f64 * grid * grid),
                deepest_mm: round2(n as f64 * grid),
            }
        })
        .collect()
}

/// The least locked of `cands` (deep area, then area; the first of equals).
fn least(res: &[Lock]) -> usize {
    let mut best = 0;
    for (i, r) in res.iter().enumerate() {
        let b = &res[best];
        if (r.deep_mm2, r.area_mm2) < (b.deep_mm2, b.area_mm2) {
            best = i;
        }
    }
    best
}

/// PER_TOOTH: the rigid removal along the least locked of the mean path of
/// the teeth (`paths`, their shears) and of paths leaning
/// [`CHECK_TILTS_DEG`] towards it, surveyed on lines twice
/// [`CHECK_GRID_MM`] apart, measured again on [`CHECK_GRID_MM`]
/// (`rigid_lock`).
pub fn rigid_lock(mesh: &TriMesh, shell: &Bvh, teeth: Option<&Bvh>, paths: &[DVec2]) -> Removal {
    let mean = if paths.is_empty() {
        DVec2::ZERO
    } else {
        paths.iter().fold(DVec2::ZERO, |s, p| s + *p) / paths.len() as f64
    };
    let ph = if (mean.x * mean.x + mean.y * mean.y).sqrt() > 1e-9 {
        mean.y.atan2(mean.x)
    } else {
        0.5 * std::f64::consts::PI
    };
    let mut cands = vec![mean];
    cands.extend(CHECK_TILTS_DEG.iter().map(|t| tilted(t.to_radians(), ph)));
    let res = removal_lock(mesh, shell, teeth, &cands, 2.0 * CHECK_GRID_MM);
    let sh = cands[least(&res)];
    let lock = removal_lock(mesh, shell, teeth, &[sh], CHECK_GRID_MM)[0];
    Removal {
        lock,
        shear: sh.to_array(),
    }
}

/// VERTICAL and COMMON: the rigid removal along the one path of the arch
/// (`shear`), on lines [`CHECK_GRID_MM`] apart.
pub fn path_lock(mesh: &TriMesh, shell: &Bvh, teeth: Option<&Bvh>, shear: DVec2) -> Removal {
    Removal {
        lock: removal_lock(mesh, shell, teeth, &[shear], CHECK_GRID_MM)[0],
        shear: shear.to_array(),
    }
}
