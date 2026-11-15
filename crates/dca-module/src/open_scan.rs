//! The scan of an arch as it was read.
//!
//! The import closes an open arch scan with a base and the case keeps the
//! closed solid (`AlignersState::scan_bases`, `dca_algo::scan_base`): the
//! view, the bite, the stamp, the trim line and the export use it, and the
//! tooth colours (the labels blob) are given for every one of its points,
//! `0` on the base.
//!
//! The steps that read the open border, or that the plugin is compared with
//! on open scans, read the scan as it was read instead, so that they give
//! what they gave before the base: finding the teeth and the orientation,
//! the frame of the case, the separation and its corrections by hand, the
//! check of the tooth marks, the measures of the teeth, the 3D teeth and the
//! gingiva, the case analysis, the final position, the aligners, the print
//! geometry, the control visits and the study models. The scan as it was
//! read is the start of the closed solid: its points keep their index, so a
//! point picked on the closed solid is the same point of the scan.

use std::sync::Arc;

use dc_case::{BlobRef, CaseState};
use dc_command::CommandError;
use dc_mesh::TriMesh;
use dca_algo::scan_base;
use dca_model::AlignersState;
use glam::Vec3;

/// The scan blob `blob` as it was read: the blob itself, or, for a scan
/// closed with a base, its first points and triangles.
pub(crate) fn open_scan(
    state: &CaseState,
    data: &AlignersState,
    blob: &BlobRef,
) -> Result<Arc<TriMesh>, CommandError> {
    let mesh = state.blobs.require(blob)?;
    open_part(data, blob, mesh)
}

/// [`open_scan`] of the mesh `mesh` of the blob `blob`, already read.
pub(crate) fn open_part(
    data: &AlignersState,
    blob: &BlobRef,
    mesh: &Arc<TriMesh>,
) -> Result<Arc<TriMesh>, CommandError> {
    match data.scan_base(blob) {
        None => Ok(Arc::clone(mesh)),
        Some(base) => scan_base::open_part(mesh, &base)
            .map(Arc::new)
            .ok_or_else(|| {
                CommandError::Failed(format!(
                    "the scan {blob} does not hold the scan it was closed from"
                ))
            }),
    }
}

/// The labels of the points of `open` (the scan as it was read) among
/// `labels`, one per point of the closed solid: the first ones.
pub(crate) fn open_labels<'a>(labels: &'a [u16], open: &TriMesh) -> &'a [u16] {
    &labels[..open.vertex_count().min(labels.len())]
}

/// The points of the scan blob `blob` as it was read, among those of its
/// mesh `mesh`: the first ones (all of them for a scan not closed with a
/// base). For a step that needs the points only (an arch frame).
pub(crate) fn open_points<'a>(
    data: &AlignersState,
    blob: &BlobRef,
    mesh: &'a TriMesh,
) -> &'a [Vec3] {
    let n = data.scan_base(blob).map_or(mesh.positions.len(), |b| {
        (b.vertices as usize).min(mesh.positions.len())
    });
    &mesh.positions[..n]
}
