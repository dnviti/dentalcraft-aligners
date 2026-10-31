//! Bite ramps: placement and shape of the bite ramps on the aligners.
//!
//! Reference: `ortho_aligner/ramps.py` (shape in
//! `ortho_aligner/ramp_shape.py`).
//!
//! A ramp is a small bump on the outer surface of an aligner the antagonist
//! rests on: anterior bite ramps on the palatal face of the upper incisors
//! open the bite at the back in a deep bite; on other teeth wherever they
//! are placed by hand. Its shape ([`ramp_mesh`]) is half a superellipsoid
//! (exponent [`RAMP_EXP`]: a flat top, the platform the antagonist rests on)
//! of half axes 0.5 x 0.5 across and 1 high, on a base that goes [`EMBED`]
//! deep into the wall; the size of a ramp (width, depth, height, mm) scales
//! it.
//!
//! In the plugin a ramp is a Blender object, child of its tooth. Here it is
//! a [`Ramp`] of the case: its pose is in the coordinates of its tooth at
//! rest, so it follows the tooth at every stage. [`solids`] puts the ramps
//! in the arch at a stage; [`points`] gives the points the antagonist can
//! touch (the bite with the aligners worn,
//! [`crate::aligner_bite::occlusal`]); [`apply_to_grid`] and
//! [`apply_to_mesh`] unite them with a shell and cut the cavity of the tooth
//! again (a ramp never fills the place of the tooth).
//!
//! Proposal ([`propose`]): in a deep bite (overbite of the incisors beyond
//! the setting) a ramp on the palatal face of every upper central and
//! lateral incisor under the aligner, where the lower incisors reach it, its
//! top turned half way between the face and the occlusal plane (a platform
//! for the lower incisor), of the size of the settings.
//!
//! Deviations from the plugin: the shell is united with the ramps on the
//! signed distance field (the plugin uses mesh booleans, and drops a ramp
//! whose boolean fails; here a ramp whose base is not inside the wall is
//! dropped with a note); `propose` takes the stage geometry as inputs
//! instead of reading the Blender objects.

use std::collections::BTreeMap;

use dc_case::{Fdi, Pose};
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use dca_model::{Ramp, RampKind, RampSettings};
use glam::{DMat3, DMat4, DQuat, DVec3};
use serde::Serialize;

use crate::aligner_bite::occlusal::vertex_normals;
use crate::occlusion::percentile;

/// Superellipsoid exponent of the top (2 = dome, larger = flatter platform).
pub const RAMP_EXP: f64 = 4.0;
/// Depth of the base into the wall, in units of the height.
pub const EMBED: f64 = 0.35;
/// Rings around the unit ramp.
pub const N_AROUND: usize = 32;
/// Rings up the unit ramp.
pub const N_UP: usize = 10;
/// Teeth that get a proposed ramp, in the order of the proposal.
pub const UPPER_INCISORS: [i64; 4] = [11, 21, 12, 22];
/// Teeth whose incisal edges the ramps meet.
pub const LOWER_INCISORS: [i64; 4] = [31, 41, 32, 42];

/// Errors of the ramps stage.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RampError {
    /// A distance-field operation failed.
    #[error("geometry: {0}")]
    Geometry(String),
}

fn geom(e: impl std::fmt::Display) -> RampError {
    RampError::Geometry(e.to_string())
}

/// Closed mesh of the unit ramp (outward faces): vertices and triangles
/// (`ramp_shape.ramp_mesh`). Its own coordinates: `Z` along the outward
/// normal of the aligner where it stands, the centre of its base at the
/// origin; half axes 0.5 across, 1 high, the base ring at `-EMBED`.
pub fn ramp_mesh(n_around: usize, n_up: usize) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    let p = RAMP_EXP;
    let ring_r: Vec<(f64, f64, f64)> = (0..n_around)
        .map(|j| {
            let a = std::f64::consts::TAU * j as f64 / n_around as f64;
            let (ca, sa) = (a.cos(), a.sin());
            // superellipse outline |x|^p + |y|^p = 0.5^p
            let r = 0.5 / (ca.abs().powf(p) + sa.abs().powf(p)).powf(1.0 / p);
            (ca, sa, r)
        })
        .collect();
    let mut v: Vec<DVec3> = Vec::new();
    // the base ring (into the wall)
    for &(ca, sa, r) in &ring_r {
        v.push(DVec3::new(r * ca, r * sa, -EMBED));
    }
    // the rings of the top, flatter towards the platform
    for i in 0..n_up {
        let t = i as f64 / n_up as f64;
        let scale = (1.0 - t.powf(p)).powf(1.0 / p);
        for &(ca, sa, r) in &ring_r {
            v.push(DVec3::new(r * ca * scale, r * sa * scale, t));
        }
    }
    let rings = n_up + 1;
    v.push(DVec3::new(0.0, 0.0, 1.0));
    v.push(DVec3::new(0.0, 0.0, -EMBED));
    let (top, bottom) = ((v.len() - 2) as u32, (v.len() - 1) as u32);
    let mut t: Vec<[u32; 3]> = Vec::new();
    let n = n_around as u32;
    for i in 0..(rings - 1) as u32 {
        let (a, b) = (i * n, (i + 1) * n);
        for j in 0..n {
            let j2 = (j + 1) % n;
            t.push([a + j, a + j2, b + j2]);
            t.push([a + j, b + j2, b + j]);
        }
    }
    let last = (rings as u32 - 1) * n;
    for j in 0..n {
        let j2 = (j + 1) % n;
        t.push([last + j, last + j2, top]);
        t.push([j2, j, bottom]);
    }
    (v, t)
}

/// 4x4 of a ramp standing at `origin` with its `Z` along `normal`; its `X`
/// as near as possible to `toward` (e.g. across the tooth), `(1, 0, 0)` by
/// default (`ramp_shape.frame`).
pub fn frame(origin: DVec3, normal: DVec3, toward: Option<DVec3>) -> DMat4 {
    let z = normal / normal.length().max(1e-12);
    let x = toward.unwrap_or(DVec3::X);
    let mut x = x - z * x.dot(z);
    if x.length() < 1e-6 {
        x = if z.z.abs() < 0.9 {
            z.cross(DVec3::Z)
        } else {
            z.cross(DVec3::X)
        };
    }
    let x = x / x.length();
    let y = z.cross(x);
    DMat4::from_cols(
        x.extend(0.0),
        y.extend(0.0),
        z.extend(0.0),
        origin.extend(1.0),
    )
}

/// Vertical overlap (mm) of the incisors in lower arch coordinates (`+Z`
/// towards the upper arch): the top of the lower incisal edges (99th
/// percentile) minus the bottom of the upper ones (1st percentile) (positive
/// = the upper incisors cover the lower ones). None without points.
pub fn overbite(lower_incisors: &[DVec3], upper_incisors: &[DVec3]) -> Option<f64> {
    if lower_incisors.is_empty() || upper_incisors.is_empty() {
        return None;
    }
    let mut lo: Vec<f64> = lower_incisors.iter().map(|p| p.z).collect();
    let mut up: Vec<f64> = upper_incisors.iter().map(|p| p.z).collect();
    Some(percentile(&mut lo, 99.0) - percentile(&mut up, 1.0))
}

fn quat_of(m: &DMat4) -> DQuat {
    DQuat::from_mat3(&DMat3::from_cols(
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    ))
    .normalize()
}

/// The pose of a rigid 4x4.
pub fn pose_of(m: &DMat4) -> Pose {
    Pose::new(m.w_axis.truncate(), quat_of(m))
}

/// 4x4 of a ramp in the coordinates of its tooth with its size as the scale
/// (`T R S`, the Blender object matrix).
pub fn matrix(ramp: &Ramp) -> DMat4 {
    let r = DMat3::from_quat(ramp.pose.q);
    let s = ramp.size_mm;
    DMat4::from_cols(
        (r.x_axis * s[0]).extend(0.0),
        (r.y_axis * s[1]).extend(0.0),
        (r.z_axis * s[2]).extend(0.0),
        ramp.pose.t.extend(1.0),
    )
}

/// A new ramp from the 4x4 `m_local` (place and turn in the coordinates of
/// the tooth, from [`frame`]) and its size.
pub fn new_ramp(fdi: Fdi, m_local: &DMat4, size_mm: [f64; 3], kind: RampKind) -> Ramp {
    Ramp {
        fdi,
        pose: pose_of(m_local),
        size_mm,
        kind,
    }
}

/// A ramp placed by hand on the surface of a tooth at `point` with the
/// outward `normal` (tooth coordinates): on the outer surface of the
/// aligner there (`ops_occlusion.OA_OT_ramps_add`).
pub fn manual_ramp(
    fdi: Fdi,
    point: DVec3,
    normal: DVec3,
    size_mm: [f64; 3],
    clearance: f64,
    wall: f64,
) -> Ramp {
    let n = normal / normal.length().max(1e-9);
    new_ramp(
        fdi,
        &frame(point + n * (clearance + wall), n, None),
        size_mm,
        RampKind::Manual,
    )
}

/// A ramp solid in the arch coordinates of a stage.
#[derive(Debug, Clone, PartialEq)]
pub struct RampSolid {
    /// The tooth it stands on.
    pub fdi: i64,
    /// Vertices (arch coordinates).
    pub vertices: Vec<DVec3>,
    /// Triangles.
    pub tris: Vec<[u32; 3]>,
}

impl RampSolid {
    /// The solid as a mesh.
    pub fn mesh(&self) -> TriMesh {
        TriMesh::new(
            self.vertices.iter().map(|v| v.as_vec3()).collect(),
            self.tris.clone(),
        )
    }
}

/// The ramps of the case in the arch at a stage (`solids`): `to_arch` maps
/// the coordinates of every tooth at rest to the arch coordinates at the
/// stage (a ramp of a tooth missing from it is left out).
pub fn solids(ramps: &[Ramp], to_arch: &BTreeMap<Fdi, DMat4>) -> Vec<RampSolid> {
    let (v0, t0) = ramp_mesh(N_AROUND, N_UP);
    ramps
        .iter()
        .filter_map(|r| {
            let a = to_arch.get(&r.fdi)?;
            let m = *a * matrix(r);
            Some(RampSolid {
                fdi: i64::from(r.fdi.get()),
                vertices: v0.iter().map(|&v| m.transform_point3(v)).collect(),
                tris: t0.clone(),
            })
        })
        .collect()
}

/// Points of the surfaces of the ramps with the FDI of their tooth: what the
/// antagonist can touch (`points`).
pub fn points(solids: &[RampSolid]) -> (Vec<DVec3>, Vec<i64>) {
    let mut p = Vec::new();
    let mut l = Vec::new();
    for s in solids {
        p.extend_from_slice(&s.vertices);
        l.extend(std::iter::repeat_n(s.fdi, s.vertices.len()));
    }
    (p, l)
}

/// A tooth the ramps are proposed on.
#[derive(Debug, Clone, Copy)]
pub struct ProposeTooth<'a> {
    /// The closed tooth in its own coordinates (at rest).
    pub mesh: &'a TriMesh,
    /// Its own coordinates to the upper arch coordinates at the stage.
    pub to_arch: DMat4,
}

/// The stage geometry [`propose`] reads, in the arch coordinates of each
/// arch.
#[derive(Debug, Clone, Copy)]
pub struct ProposeInput<'a> {
    /// Crown points of every lower tooth (about one every 0.2 mm) at the
    /// stage, lower arch coordinates.
    pub lower_crowns: &'a BTreeMap<i64, Vec<DVec3>>,
    /// Crown points of every upper tooth at the stage, upper arch
    /// coordinates.
    pub upper_crowns: &'a BTreeMap<i64, Vec<DVec3>>,
    /// Upper to lower arch coordinates.
    pub rel: DMat4,
    /// The upper teeth.
    pub upper_teeth: &'a BTreeMap<i64, ProposeTooth<'a>>,
    /// Teeth under the upper aligner.
    pub covered: &'a [i64],
    /// Clearance between tooth and shell (mm).
    pub clearance: f64,
    /// Wall thickness of the shell (mm).
    pub wall: f64,
}

/// The ramps proposed for a deep bite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Proposal {
    /// The ramps (place and turn in the coordinates of their tooth).
    #[serde(skip)]
    pub ramps: Vec<Ramp>,
    /// Overbite of the incisors (mm).
    pub overbite_mm: Option<f64>,
    /// Why there is none (empty when there is).
    pub reason: String,
}

/// Ramps proposed for a deep bite at a stage (see the module): a ramp on
/// every covered upper incisor the lower incisors reach, `size` as the
/// settings, `overbite_min` the threshold of the settings.
pub fn propose(input: &ProposeInput<'_>, s: &RampSettings) -> Proposal {
    let pick = |m: &BTreeMap<i64, Vec<DVec3>>, ids: &[i64], t: &DMat4| -> Vec<DVec3> {
        ids.iter()
            .filter_map(|f| m.get(f))
            .flat_map(|v| v.iter().map(|&p| t.transform_point3(p)))
            .collect()
    };
    let low = pick(input.lower_crowns, &LOWER_INCISORS, &DMat4::IDENTITY);
    let upp = pick(input.upper_crowns, &UPPER_INCISORS, &input.rel);
    if low.is_empty() || upp.is_empty() {
        return Proposal {
            ramps: vec![],
            overbite_mm: None,
            reason: "mancano gli incisivi di una delle due arcate".to_owned(),
        };
    }
    let ob = overbite(&low, &upp);
    let Some(ob_v) = ob.filter(|&o| o >= s.overbite_min_mm) else {
        let reason = match ob {
            Some(o) => format!(
                "morso non profondo: overbite {o:.1} mm (soglia {:.1} mm)",
                s.overbite_min_mm
            ),
            None => "overbite non misurabile".to_owned(),
        };
        return Proposal {
            ramps: vec![],
            overbite_mm: ob,
            reason,
        };
    };
    // the incisal edges of the lower incisors, in upper arch coordinates
    let mut zs: Vec<f64> = low.iter().map(|p| p.z).collect();
    let z90 = percentile(&mut zs, 90.0);
    let inv = crate::frame::rigid_inverse(&input.rel);
    let tips: Vec<DVec3> = low
        .iter()
        .filter(|p| p.z >= z90)
        .map(|&p| inv.transform_point3(p))
        .collect();
    let stride = (tips.len() / 200).max(1);
    let sub: Vec<DVec3> = tips.iter().step_by(stride).copied().collect();
    let size = [s.width_mm, s.depth_mm, s.height_mm];
    let up = DVec3::Z;
    let mut ramps = Vec::new();
    for f in UPPER_INCISORS {
        let (Some(tooth), true) = (input.upper_teeth.get(&f), input.covered.contains(&f)) else {
            continue;
        };
        let Some(fdi) = u8::try_from(f).ok().and_then(|n| Fdi::new(n).ok()) else {
            continue;
        };
        let a = tooth.to_arch;
        let vt: Vec<DVec3> = tooth
            .mesh
            .positions
            .iter()
            .map(|p| a.transform_point3(p.as_dvec3()))
            .collect();
        let nt = vertex_normals(&vt, &tooth.mesh.tris);
        // the point of the tooth nearest to a lower incisal edge, on the face that looks at it
        let (mut j, mut best) = (0usize, f64::INFINITY);
        for (k, v) in vt.iter().enumerate() {
            let m = sub
                .iter()
                .map(|t| (*t - *v).length())
                .fold(f64::INFINITY, f64::min);
            if m < best {
                best = m;
                j = k;
            }
        }
        let tip = sub
            .iter()
            .copied()
            .fold((f64::INFINITY, sub[0]), |(bd, bt), t| {
                let d = (t - vt[j]).length();
                if d < bd { (d, t) } else { (bd, bt) }
            })
            .1;
        let (p, n) = (vt[j], nt[j] / nt[j].length().max(1e-9));
        if (tip - p).dot(n) < 0.0 {
            continue; // the lower incisor is not in front of it
        }
        let z = (n + up) / (n + up).length().max(1e-9);
        let origin = p + n * (input.clearance + input.wall); // on the outer surface of the aligner
        let m_arch = frame(origin, z, Some(up.cross(n)));
        let m_local = a.inverse() * m_arch;
        ramps.push(new_ramp(fdi, &m_local, size, RampKind::Auto));
    }
    let reason = if ramps.is_empty() {
        "nessun incisivo superiore sotto l'allineatore davanti agli inferiori".to_owned()
    } else {
        String::new()
    };
    Proposal {
        ramps,
        overbite_mm: Some(ob_v),
        reason,
    }
}

/// The ramps united with a shell (a signed distance field with the voxel of
/// the shell) and the cavity of each tooth cut again (`add_to_shell`):
/// `teeth` the closed teeth at the stage (arch coordinates, by FDI). Returns
/// the notes of the ramps left out (not on the shell: outside its wall).
///
/// # Errors
/// [`RampError::Geometry`] when a distance field cannot be built.
pub fn apply_to_grid(
    shell: &mut SdfGrid,
    ramps: &[RampSolid],
    teeth: &BTreeMap<i64, TriMesh>,
) -> Result<Vec<String>, RampError> {
    let params = SdfParams {
        voxel_size: shell.voxel_size,
        band: shell.band(),
    };
    let mut notes = Vec::new();
    let mut tooth_grids: BTreeMap<i64, SdfGrid> = BTreeMap::new();
    for r in ramps {
        // the centre of the base of the ramp must be inside the wall
        let base = r.vertices[N_AROUND * (N_UP + 1) + 1];
        if shell.sample(base.as_vec3()) > 0.0 {
            notes.push(format!(
                "rampa sul dente {} non unita all'allineatore (fuori dalla sua superficie?)",
                r.fdi
            ));
            continue;
        }
        let g = SdfGrid::from_mesh(&r.mesh(), &params).map_err(geom)?;
        shell.union(&g).map_err(geom)?;
        if let Some(t) = teeth.get(&r.fdi) {
            let g = match tooth_grids.entry(r.fdi) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(SdfGrid::from_mesh(t, &params).map_err(geom)?)
                }
            };
            shell.subtract(g).map_err(geom)?;
        }
    }
    Ok(notes)
}

/// [`apply_to_grid`] on a shell mesh (arch coordinates): the shell is
/// sampled at `voxel` mm, the ramps added, and the surface meshed again.
///
/// # Errors
/// [`RampError::Geometry`] when a distance field cannot be built.
pub fn apply_to_mesh(
    shell: &TriMesh,
    ramps: &[RampSolid],
    teeth: &BTreeMap<i64, TriMesh>,
    voxel: f64,
) -> Result<(TriMesh, Vec<String>), RampError> {
    if ramps.is_empty() {
        return Ok((shell.clone(), vec![]));
    }
    let params = SdfParams {
        voxel_size: voxel as f32,
        band: (3.0 * voxel).max(0.5) as f32,
    };
    let mut grid = SdfGrid::from_mesh(shell, &params).map_err(geom)?;
    let notes = apply_to_grid(&mut grid, ramps, teeth)?;
    Ok((grid.to_mesh().map_err(geom)?, notes))
}

/// The unit ramp as a mesh.
pub fn unit_mesh() -> TriMesh {
    let (v, t) = ramp_mesh(N_AROUND, N_UP);
    TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t)
}

#[cfg(test)]
mod tests;
