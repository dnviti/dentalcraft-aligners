//! Data model of the Aligners module.
//!
//! [`AlignersState`] is the module's part of a case: it is stored in the
//! case state under the module id [`MODULE_ID`] and written to `.dcprj`
//! files as JSON (schema version [`SCHEMA_VERSION`]). It holds the results
//! of the pipeline stages (seeds, tooth labels, tooth frames, diagnosis,
//! setup, stages) and the user's settings. Meshes are never embedded: they
//! are referenced by [`dc_case::BlobRef`].
//!
//! Units are millimetres and degrees unless a field name says otherwise;
//! poses are [`dc_case::Pose`] (millimetres, radians in the quaternion).

mod bite;
mod migrate;
mod settings;
mod state;
mod visit;

pub use bite::{
    AlignerBite, BiteClosing, OcclusalMode, OcclusalSettings, Ramp, RampKind, RampSettings,
};
pub use migrate::{ModelError, migrate};
pub use settings::{
    FindTeethSettings, PerioStage, SegmentationSettings, Settings, StagingPattern, StagingSettings,
    StudyBaseSettings, StudyBaseStyle, TrimMode, TrimParams,
};
pub use state::{
    AlignersState, ArchDetails, ArchSpace, CLEARED_IDS, Diagnosis, IprStep, Seed, Setup, Stage,
    ToothFrame, ToothMeasures,
};
pub use visit::{Decision, ToothDeviation, ToothTracking, Visit};

/// Id of the Aligners module (key of its data in the case state).
pub const MODULE_ID: &str = "aligners";

/// Newest schema version of [`AlignersState`] (written on save).
pub const SCHEMA_VERSION: u32 = 1;
