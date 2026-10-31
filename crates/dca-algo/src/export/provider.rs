//! Where the meshes of the export come from.
//!
//! The export asks a [`MeshProvider`] for the model (the thermoforming model,
//! with the undercuts blocked out) and the aligner shell of every stage and
//! arch. The shell code (`shell.rs`, `trim.rs`, the stamp) implements the
//! trait when it lands and the command passes it to
//! [`super::package::run`]; until then [`ScanProvider`] gives the deformed
//! scan as the model and no shell.
//!
//! The provider is called from several threads at once (one stage per
//! task), so it must be `Sync` and its answers must not depend on the order
//! of the calls: the export is deterministic only if the provider is.
//!
//! # Wiring point
//!
//! `dca-module/src/commands/export.rs`, function `mesh_provider`: return the
//! shell-based provider there instead of [`ScanProvider`].

use dc_mesh::TriMesh;

use super::{ExportError, Input, deform_series, stage_deltas};
use crate::anatomy::Arch;

/// Source of the printable meshes of every stage, in the coordinates of the
/// case.
pub trait MeshProvider: Sync {
    /// The model of aligner `stage` of an arch (`0` is the initial
    /// situation), blocked out and with its base. `None`: no model.
    fn model(&self, arch: Arch, stage: u32) -> Result<Option<TriMesh>, ExportError>;

    /// The shell of aligner `stage` (`1..=n_active`) of an arch, as a closed
    /// solid, stamp included. `None`: no shell (the direct printing files
    /// are skipped). The passive aligners repeat the shell of the last
    /// active stage.
    fn aligner(&self, arch: Arch, stage: u32) -> Result<Option<TriMesh>, ExportError>;
}

/// The deformed scan as the model, no shells: what the export gave before
/// the shell code existed.
#[derive(Debug, Clone, Copy)]
pub struct ScanProvider<'a> {
    input: &'a Input<'a>,
}

impl<'a> ScanProvider<'a> {
    /// Provider over the series of `input`.
    pub fn new(input: &'a Input<'a>) -> Self {
        Self { input }
    }
}

impl MeshProvider for ScanProvider<'_> {
    fn model(&self, arch: Arch, stage: u32) -> Result<Option<TriMesh>, ExportError> {
        let Some(s) = self.input.arches.iter().find(|s| s.arch == arch) else {
            return Ok(None);
        };
        let deltas = stage_deltas(s, self.input.stages, stage);
        Ok(Some(deform_series(s, &deltas)))
    }

    fn aligner(&self, _arch: Arch, _stage: u32) -> Result<Option<TriMesh>, ExportError> {
        Ok(None)
    }
}
