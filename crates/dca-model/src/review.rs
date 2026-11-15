//! The clinical review of the case: the classifications the program proposes
//! and the clinician's decision on each (the plugin's `OA_ClassRecord`,
//! `props.py`, and `review.py`).
//!
//! A record comes from the evidence-based calculation of
//! `dca_algo::review` (the plugin's `classify.py`; the AI classifier of the
//! plugin is not ported). Its proposal is preset as the decision and stays
//! "to review" ([`ReviewStatus::Pending`]) until the clinician confirms it or
//! chooses another value. The values are stable ids (`easy`, `structured`,
//! `rotation`, ...) that the interface words in its language.

use dc_case::{Fdi, ScanRole};
use serde::{Deserialize, Serialize};

/// What a record classifies (the plugin's `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    /// Difficulty of the case (`complessita`), one per arch.
    Complexity,
    /// Staging pattern (`schema_staging`), one per arch.
    StagingPattern,
    /// Treatable with aligners only (`idoneita_allineatori`), one per arch.
    AlignerSuitability,
    /// How predictable the planned movement of a tooth is
    /// (`rischio_movimento`).
    MovementRisk,
    /// The attachment the movement of a tooth calls for (`attachment`).
    Attachment,
    /// The crown measures of a tooth are plausible (`segmentazione`).
    Segmentation,
    /// The type the number of a tooth names (`tipo_dente`).
    ToothType,
}

impl ReviewKind {
    /// Every kind, in the order the plugin makes the records of a tooth.
    pub const ALL: [ReviewKind; 7] = [
        ReviewKind::Complexity,
        ReviewKind::StagingPattern,
        ReviewKind::AlignerSuitability,
        ReviewKind::MovementRisk,
        ReviewKind::Attachment,
        ReviewKind::Segmentation,
        ReviewKind::ToothType,
    ];

    /// The id of the kind, as it is stored (`staging_pattern`).
    pub fn id(self) -> &'static str {
        match self {
            ReviewKind::Complexity => "complexity",
            ReviewKind::StagingPattern => "staging_pattern",
            ReviewKind::AlignerSuitability => "aligner_suitability",
            ReviewKind::MovementRisk => "movement_risk",
            ReviewKind::Attachment => "attachment",
            ReviewKind::Segmentation => "segmentation",
            ReviewKind::ToothType => "tooth_type",
        }
    }

    /// The kind of an id ([`ReviewKind::id`]).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// Where the clinician's decision on a record stands (the plugin's
/// `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    /// The proposal is preset as the decision, still to review.
    #[default]
    Pending,
    /// The clinician confirmed the proposal.
    Accepted,
    /// The clinician kept the computed value over a different proposal.
    Rejected,
    /// The clinician chose another value.
    Modified,
}

/// One classification and the decision on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRecord {
    /// The arch it is about.
    pub arch: ScanRole,
    /// What it classifies.
    pub kind: ReviewKind,
    /// The tooth (`None`: the whole arch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fdi: Option<Fdi>,
    /// The value of the calculation (`math_value`).
    pub computed: String,
    /// The value proposed (`proposed`): the computed one, as no other
    /// source refines it.
    pub proposed: String,
    /// The values the clinician can choose from (`options_json`).
    pub options: Vec<String>,
    /// What the value was worked out from (measures, thresholds met), for
    /// the interface to word the method (`math_detail`).
    #[serde(default)]
    pub detail: serde_json::Value,
    /// Keys of the references behind it (`dca_algo::evidence`).
    #[serde(default)]
    pub refs: Vec<String>,
    /// A note asking for a check (`nota`), by id; such a record is never
    /// confirmed in bulk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The decision.
    #[serde(default)]
    pub status: ReviewStatus,
    /// The value decided (`decided_value`): the proposal while the record
    /// is still to review.
    #[serde(default)]
    pub decided: String,
    /// When the clinician decided (Unix seconds); `None` while to review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
}

impl ReviewRecord {
    /// The key of the record, unique in a case: `lower:attachment:31`,
    /// `upper:complexity:arch`.
    pub fn key(&self) -> String {
        let arch = match self.arch {
            ScanRole::Upper => "upper",
            ScanRole::Lower => "lower",
            ScanRole::Bite => "bite",
        };
        let subject = self
            .fdi
            .map_or_else(|| "arch".to_owned(), |f| f.to_string());
        format!("{arch}:{}:{subject}", self.kind.id())
    }

    /// The value in force: the decision, or the proposal while the record
    /// has none (the plugin's `decided_value or proposed`).
    pub fn in_force(&self) -> &str {
        if self.decided.is_empty() {
            &self.proposed
        } else {
            &self.decided
        }
    }

    /// The record has a note asking for a check (`review.needs_attention`).
    pub fn needs_check(&self) -> bool {
        self.note.is_some()
    }
}
