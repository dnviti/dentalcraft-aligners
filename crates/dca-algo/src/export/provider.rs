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
//! `dca-module/src/commands/export.rs`, function `mesh_provider`: the
//! shell-based provider (`ShellProvider`, the printable model and the
//! trimmed aligner of every stage) when the case has the closed teeth and
//! the gingiva, [`ScanProvider`] otherwise.

use dc_mesh::TriMesh;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ExportError, Input, deform_series, stage_deltas};
use crate::anatomy::Arch;

/// Quality of the print files: the voxel of the distance fields the models
/// and the shells are built on and the mesh tolerance of the files
/// (`quality.PRESETS` of the plugin: `export_voxel`, `shell_voxel`,
/// `mesh_tolerance`), plus a draft for quick runs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Quick look and tests: 0.25 mm voxel, 0.05 mm tolerance (not a preset
    /// of the plugin; the wall stays above four voxels).
    Draft,
    /// The plugin's Leggera: 0.15 mm voxel, 0.03 mm tolerance.
    Light,
    /// The plugin's Standard, the production files: 0.1 mm voxel, 0.01 mm
    /// tolerance.
    #[default]
    #[serde(alias = "final")]
    Standard,
    /// The plugin's Alta: 0.08 mm voxel, 0.005 mm tolerance.
    High,
}

impl Quality {
    /// Voxel of the model and shell fields (mm).
    pub fn voxel_mm(self) -> f64 {
        match self {
            Self::Draft => 0.25,
            Self::Light => 0.15,
            Self::Standard => 0.1,
            Self::High => 0.08,
        }
    }

    /// Mesh tolerance of the print files (mm).
    pub fn tolerance_mm(self) -> f64 {
        match self {
            Self::Draft => 0.05,
            Self::Light => 0.03,
            Self::Standard => 0.01,
            Self::High => 0.005,
        }
    }
}

/// Something the provider could not do as asked for one file (a stamp with
/// no room, a ramp left out, the bite not shaped), or for the whole export
/// (`arch` and `stage` `None`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Note {
    /// The arch of the file.
    pub arch: Option<Arch>,
    /// The aligner (`0` the initial model).
    pub stage: Option<u32>,
    /// Model (`false`) or aligner (`true`).
    pub aligner: bool,
    /// Key of the text (`export::text`).
    pub key: &'static str,
    /// Details (numbers, the reason given by the step), as they are.
    pub detail: String,
}

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

    /// The notes gathered so far, sorted (arch, stage, kind): the export reads
    /// them once every mesh is made.
    fn notes(&self) -> Vec<Note> {
        Vec::new()
    }

    /// Meshes built at the same time at most (the memory of one build);
    /// `None`: as many as the threads.
    fn parallel_hint(&self) -> Option<usize> {
        None
    }
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
