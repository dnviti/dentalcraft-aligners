//! The local method: refine the surface under the text and move its vertices
//! along their normals by the stamp field (see the module docs of `stamp`).

use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};

use super::field::TextField;
use super::place::Surface;
use super::refine::Refiner;
use super::{Located, Params};

/// A triangle (or vertex) is on the surface seen from the crown when the first
/// crossing of the ray through it is this close to it (mm).
const SHEET_TOL_MM: f64 = 0.02;

/// The stamped mesh and the number of triangles the refinement added.
///
/// # Errors
/// The reason when there is no surface under the text.
pub(super) fn stamp(
    surface: &Surface<'_>,
    located: &Located,
    params: &Params,
    h: f64,
) -> Result<(TriMesh, usize), String> {
    let frame = located.frame;
    let origin = located.origin;
    let reach = 0.5 * h;
    let field = TextField::new(&located.shape, reach);
    let half =
        DVec2::new(located.shape.width, located.shape.height) * 0.5 + DVec2::splat(reach + h);
    let plane = |p: DVec3| DVec2::new((p - origin).dot(frame.r), (p - origin).dot(frame.u));
    let depth_of = |p: DVec3| (p - origin).dot(frame.n);
    let in_zone = |t: &[DVec3; 3]| {
        let q = t.map(plane);
        let lo = q[0].min(q[1]).min(q[2]);
        let hi = q[0].max(q[1]).max(q[2]);
        lo.x <= half.x && hi.x >= -half.x && lo.y <= half.y && hi.y >= -half.y
    };
    // the point of the plane of the rays that sees `p` along `n`
    let ray_start = |p: DVec3| p - depth_of(p) * frame.n;
    let on_sheet = |points: &[DVec3]| -> Vec<bool> {
        let starts: Vec<DVec3> = points.iter().map(|p| ray_start(*p)).collect();
        surface
            .first_t(&starts, frame.n)
            .iter()
            .zip(points)
            .map(|(t, p)| t.is_some_and(|t| (t - depth_of(*p)).abs() <= SHEET_TOL_MM))
            .collect()
    };

    // the triangles of the surface that the text can reach
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in &located.solid.positions {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let pad = Vec3::splat((reach + h + 0.1) as f32);
    let mesh = surface.mesh;
    let mut refiner = Refiner::new(mesh);
    let mut seen = 0usize;
    for f in surface.faces_in(lo - pad, hi + pad) {
        let t = mesh.tris[f as usize].map(|i| mesh.positions[i as usize].as_dvec3());
        if !in_zone(&t) {
            continue;
        }
        let c = (t[0] + t[1] + t[2]) / 3.0;
        let samples = [c, (c + t[0]) / 2.0, (c + t[1]) / 2.0, (c + t[2]) / 2.0];
        if on_sheet(&samples).iter().any(|&s| s) {
            refiner.activate(f);
            seen += 1;
        }
    }
    if seen == 0 {
        return Err("no surface under the text".to_owned());
    }
    let before = refiner.tris.len();
    refiner.refine(h, &in_zone);

    // the vertices in the plane of the text and on the surface: moved by the field
    let candidates: Vec<usize> = (0..refiner.positions.len())
        .filter(|&i| {
            let q = plane(refiner.positions[i]);
            q.x.abs() <= half.x && q.y.abs() <= half.y && depth_of(refiner.positions[i]) > 0.0
        })
        .collect();
    let points: Vec<DVec3> = candidates.iter().map(|&i| refiner.positions[i]).collect();
    let sign = if params.kind == super::Kind::Shell {
        -1.0
    } else {
        1.0
    };
    for (&i, on) in candidates.iter().zip(on_sheet(&points)) {
        if !on {
            continue;
        }
        let sd = field.signed_distance(plane(refiner.positions[i]));
        let s = (0.5 - sd / (2.0 * reach)).clamp(0.0, 1.0);
        if s > 0.0 {
            let n = refiner.normals[i];
            refiner.positions[i] += sign * params.depth * s * n;
        }
    }
    let added = refiner.tris.len() - before;
    Ok((refiner.into_mesh(), added))
}
