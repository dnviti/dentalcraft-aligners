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

mod aligner_view;
mod bite;
mod bite_report;
mod bite_use;
mod components;
mod migrate;
mod print;
mod production;
mod review;
mod scan_base;
mod settings;
mod state;
mod visit;

pub use aligner_view::{AlignerBase, AlignerView, view_key};
pub use bite::{
    AlignerBite, BiteClosing, OcclusalMode, OcclusalSettings, Ramp, RampKind, RampSettings,
};
pub use bite_report::{
    BiteFit, BiteMethod, BiteRegistration, BiteReport, BiteScanFit, BiteVerdict,
};
pub use bite_use::{BiteUse, StoredBite, bite_use};
pub use components::{ComponentEdits, TOOTH_SCALE_MAX, TOOTH_SCALE_MIN, ToothScale, tooth_centre};
pub use migrate::{ModelError, migrate};
pub use print::{
    BadPrintKey, MarkSource, PrintKey, PrintMark, PrintRecord, PrintStatus, print_transition,
};
pub use production::{AlignerEnd, AlignerEndSuggestion, Side};
pub use review::{ReviewKind, ReviewRecord, ReviewStatus};
pub use scan_base::{ClosedBy, ScanBase};
pub use settings::{
    FindTeethSettings, INTERVAL_DAYS, InsertionAxis, IntervalPolicy, PRODUCTION_GROUPS,
    PRODUCTION_NUMBERS, PerioStage, Quality, SegmentationSettings, Settings, SetupIprZone,
    SetupLeveling, SetupMidline, SetupScope, SetupSettings, SpacePriority, StagingPattern,
    StagingSettings, StampDate, StampDateFormat, StampFace, StampRead, StampSettings, StampTooth,
    StudyBaseSettings, StudyBaseStyle, TOOTH_SPEED, TrimMode, TrimParams, production_out_of_range,
};
pub use state::{
    AlignersState, ArchDetails, ArchEdits, ArchSpace, CLEARED_IDS, CollisionLeft, Diagnosis,
    IprStep, Seed, SegmentationRun, Setup, SetupSummary, Stage, StagingSummary, TeethBasis,
    TeethChange, ToothFrame, ToothMeasures, other_arch_fdi, other_arch_label,
};
pub use visit::{Decision, ToothDeviation, ToothTracking, Visit};

/// Id of the Aligners module (key of its data in the case state).
pub const MODULE_ID: &str = "aligners";

/// Newest schema version of [`AlignersState`] (written on save). Version 2
/// added the print tracking ([`AlignersState::prints`]) and the last export
/// folder ([`AlignersState::export_dir`]).
pub const SCHEMA_VERSION: u32 = 2;
