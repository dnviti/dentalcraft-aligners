//! Tunnels through a study model: holes through the solid, the handles of
//! its surface.
//!
//! The model is the scanned anatomy as it is, so a tunnel of the scan itself
//! (the embrasure under a contact point, scanned from both sides and open
//! through) is a tunnel of the model when it is wider than the shell the scan
//! is thickened into. The builder makes others where the anatomy meets what
//! it adds: where the walls dropped from a ragged border pleat above the
//! land, or the land meets the rim of a scanned model.
//!
//! [`plan`] finds, on the meshed model, the shortest loop round each tunnel
//! (the loops of every handle from the tree-cotree decomposition of the
//! surface, then the shortest loop near each, as the shells of the aligners
//! are plugged, [`crate::trim::holes`]) and closes it on the level set the
//! model is meshed from, where nothing can cross: the disk the loop bounds
//! (a fan to its middle point, thickened to a few voxels) is added when its
//! middle lies outside the solid (a hole, plugged) and taken away when it
//! lies inside (a bridge over it, cut); a disk that does not lower the
//! genus that way is tried the other way round. A loop round a tunnel of
//! the scan ([`of_scan`]: their middle points within [`SCAN_TUNNEL_MM`] and
//! the radius of the loop, the loop at most twice as long and a millimetre)
//! goes round that tunnel: it is closed only when the disk stays within the
//! tolerance of the scan, otherwise the tunnel is kept and listed
//! ([`Kept`]). A tunnel of the scan whose shortest loop is shorter than the
//! shell is round (`π` times its thickness) is filled by the shell and goes
//! round nothing; one longer than [`SCAN_TUNNEL_MAX_MM`] is no embrasure.

use dc_geom::bvh::Bvh;
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};

use crate::trim::holes::{self, CROP_MM, ROOTS};

/// A loop of the model whose middle point is within this and its radius of
/// the middle of the loop of a tunnel of the scan, and at most twice as
/// long and a millimetre, goes round that tunnel (mm).
pub(super) const SCAN_TUNNEL_MM: f64 = 2.0;

/// The shortest loop round the embrasure under a contact point is at most
/// this long (mm): a longer handle of the scan is no tunnel of the anatomy.
pub(super) const SCAN_TUNNEL_MAX_MM: f64 = 6.0;

/// `true` when the loop of the model of length `len` round `at` goes round
/// the tunnel of the scan `tunnel` ([`SCAN_TUNNEL_MM`]).
fn round(at: DVec3, len: f64, tunnel: (DVec3, f64)) -> bool {
    at.distance(tunnel.0) <= SCAN_TUNNEL_MM + len / std::f64::consts::TAU
        && len <= 2.0 * tunnel.1 + 1.0
}

/// The middle point of the loop `lp`.
fn middle(pos: &[DVec3], lp: &[u32]) -> DVec3 {
    lp.iter().map(|&v| pos[v as usize]).sum::<DVec3>() / lp.len() as f64
}

/// The tunnels of the scan: the middle point and the length of the shortest
/// loop near each loop of a handle of `scan`, the anatomy closed on the
/// floor, from `shortest` to [`SCAN_TUNNEL_MAX_MM`] long. None when `scan`
/// is not a closed surface with two faces on every edge.
pub(super) fn of_scan(scan: &TriMesh, shortest: f64) -> Vec<(DVec3, f64)> {
    let pos: Vec<DVec3> = scan.positions.iter().map(|p| p.as_dvec3()).collect();
    holes::handle_loops(&pos, &scan.tris)
        .iter()
        .filter_map(|l| {
            holes::short_loops(&pos, &scan.tris, l, CROP_MM, ROOTS, 1)
                .into_iter()
                .next()
        })
        .filter(|(len, _)| (shortest..=SCAN_TUNNEL_MAX_MM).contains(len))
        .map(|(len, lp)| (middle(&pos, &lp), len))
        .collect()
}

/// The anatomy a tunnel is measured against.
pub(super) struct Scan {
    /// The middle points and lengths of its tunnels ([`of_scan`]).
    pub tunnels: Vec<(DVec3, f64)>,
    /// Its surface.
    pub bvh: Bvh,
}

impl Scan {
    /// `closed`, the anatomy closed on the floor, its tunnels at least as
    /// long round as a shell of `shell` (mm).
    ///
    /// # Errors
    /// Errors of the BVH.
    pub(super) fn new(closed: &TriMesh, shell: f64) -> Result<Self, dc_geom::GeomError> {
        Ok(Self {
            tunnels: of_scan(closed, std::f64::consts::PI * shell),
            bvh: Bvh::build(closed)?,
        })
    }
}

/// A tunnel of the scan kept in the model.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Kept {
    /// The middle of its shortest loop (working frame).
    pub at: DVec3,
    /// The length of that loop (mm).
    pub loop_mm: f64,
}

/// A disk closing a tunnel: the fan of its shortest loop to its middle
/// point, added across a hole or taken away through a bridge.
#[derive(Debug, Clone)]
pub(super) struct Disk {
    /// Added (the middle of the loop outside the solid) or taken away.
    pub plug: bool,
    /// Its triangles.
    pub fan: Vec<[Vec3; 3]>,
}

impl Disk {
    /// The same disk the other way: taken away where it was added.
    pub(super) fn flipped(&self) -> Self {
        Self {
            plug: !self.plug,
            ..self.clone()
        }
    }
}

/// What one round of [`plan`] closes and keeps.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// The disks of the tunnels to close, shortest loop first.
    pub disks: Vec<Disk>,
    /// Tunnels of the scan kept.
    pub kept: Vec<Kept>,
}

/// The tunnels of `mesh` (the model meshed from `solid`, one closed piece):
/// the shortest loop round each, shortest first, those within half their
/// length of a shorter one left out (the other loop of the same handle).
fn loops(mesh: &TriMesh) -> Vec<(f64, Vec<DVec3>)> {
    let pos: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let mut found: Vec<(f64, Vec<u32>)> = holes::handle_loops(&pos, &mesh.tris)
        .iter()
        .filter_map(|l| {
            holes::short_loops(&pos, &mesh.tris, l, CROP_MM, ROOTS, 1)
                .into_iter()
                .next()
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, Vec<DVec3>)> = Vec::new();
    for (len, lp) in found {
        let at = middle(&pos, &lp);
        let same = out.iter().any(|(l, q)| {
            let m = q.iter().sum::<DVec3>() / q.len() as f64;
            m.distance(at) <= 0.5 * len.max(*l)
        });
        if !same {
            out.push((len, lp.iter().map(|&v| pos[v as usize]).collect()));
        }
    }
    out
}

/// One round of closing the tunnels of `mesh`, the model meshed from
/// `solid` (see the module documentation): `scan` the anatomy, `tolerance`
/// how far from it the disk of a tunnel of the scan may be (mm), `kept` the
/// tunnels of the scan kept so far, which are not tried again.
pub(super) fn plan(
    mesh: &TriMesh,
    solid: &SdfGrid,
    scan: &Scan,
    tolerance: f64,
    kept: &[Kept],
) -> Plan {
    let mut out = Plan::default();
    for (len, lp) in loops(mesh) {
        let at = lp.iter().sum::<DVec3>() / lp.len() as f64;
        if kept.iter().any(|k| round(at, len, (k.at, k.loop_mm))) {
            continue;
        }
        let fan: Vec<[Vec3; 3]> = (0..lp.len())
            .map(|i| [lp[i], lp[(i + 1) % lp.len()], at].map(|p| p.as_vec3()))
            .collect();
        if scan.tunnels.iter().any(|&t| round(at, len, t))
            && fan.iter().any(|t| {
                let m = (t[0] + t[1] + t[2]) / 3.0;
                scan.bvh.closest_point(m, tolerance as f32).is_none()
            })
        {
            out.kept.push(Kept { at, loop_mm: len });
            continue;
        }
        out.disks.push(Disk {
            plug: solid.sample(at.as_vec3()) >= 0.0,
            fan,
        });
    }
    out
}

/// `solid` with `disks` added or taken away, each thickened to `half` on
/// both sides (mm) on the voxels of `sdf`, then cut by `walls`.
///
/// # Errors
/// Errors of the level sets.
pub(super) fn apply(
    solid: &SdfGrid,
    disks: &[&Disk],
    half: f32,
    sdf: &SdfParams,
    walls: &SdfGrid,
) -> Result<SdfGrid, dc_geom::GeomError> {
    let grid = |plug: bool| -> Result<Option<SdfGrid>, dc_geom::GeomError> {
        let fans: Vec<[Vec3; 3]> = disks
            .iter()
            .filter(|d| d.plug == plug)
            .flat_map(|d| d.fan.iter().copied())
            .collect();
        if fans.is_empty() {
            return Ok(None);
        }
        let n = fans.len() as u32;
        let mesh = TriMesh::new(
            fans.into_iter().flatten().collect(),
            (0..n).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
        );
        let mut g = SdfGrid::from_mesh(&mesh, sdf)?;
        g.shift(half)?;
        Ok(Some(g))
    };
    let mut out = solid.clone();
    if let Some(g) = grid(true)? {
        out.union(&g)?;
    }
    if let Some(g) = grid(false)? {
        out.subtract(&g)?;
    }
    out.intersect(walls)?;
    Ok(out)
}

/// `true` when `next` (`solid` with `disks` added or taken away) keeps the
/// scanned anatomy where it was: no point of `anatomy` within the reach of
/// a disk (the sphere round its fan, and a millimetre) ends up farther than
/// `tolerance` (mm) from the surface than it was, buried under a plug or
/// left out by a cut.
pub(super) fn keeps_anatomy(
    solid: &SdfGrid,
    next: &SdfGrid,
    disks: &[&Disk],
    anatomy: &[Vec3],
    tolerance: f32,
) -> bool {
    disks.iter().all(|d| {
        let n = (3 * d.fan.len()) as f32;
        let at = d.fan.iter().flatten().sum::<Vec3>() / n;
        let reach = d
            .fan
            .iter()
            .flatten()
            .map(|p| p.distance(at))
            .fold(0.0, f32::max)
            + 1.0;
        anatomy
            .iter()
            .filter(|p| p.distance(at) <= reach)
            .all(|&p| next.sample(p).abs() <= solid.sample(p).abs().max(tolerance))
    })
}
