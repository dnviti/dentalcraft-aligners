//! Staging: divides the movement of every tooth into aligners without
//! exceeding the per-aligner limits, with collision-free delays; IPR
//! scheduling.
//!
//! Reference: `ortho_aligner/staging.py` (and `case_plan.py` for the glue);
//! the defaults are in `ortho_aligner/evidence.py` ([`crate::evidence`]).
//!
//! Each tooth movement is expressed in the tooth rest frame about the
//! estimated centre of resistance as `q = (t_md, t_bl, t_ax, r_md, r_bl,
//! r_ax)`: translation of the centre of resistance (mm) and rotation vector
//! (rad), i.e. torque about the MD axis, tip about the BL axis and rotation
//! about the long axis. The number of aligners of a tooth is the smallest `n`
//! such that no component exceeds its velocity limit per aligner (0.25 mm
//! linear, 1 deg torque/tip, 2 deg rotation, 1.5 deg for round crowns;
//! Castroflorio 2023, Simon 2014) and no crown point moves more than the
//! global cap per aligner (0.25-0.33 mm, Wang 2023). For periodontally
//! compromised patients the steps are reduced (Ma and Li 2021).
//!
//! Patterns (Martinez-Lozano 2024; Sterental 2006; Ravera 2016):
//! proportional (all teeth move together and finish together), maximum
//! velocity ("X"), sequential distalization ("V"), structured (space
//! creation, alignment, then vertical movements), auto (from the case
//! complexity). Adjacent crowns are checked for interpenetration at every
//! stage and the movement of the tooth that moves into its neighbour is
//! delayed (collision-free staging).
//!
//! # Coordinates and conventions
//!
//! The planning runs in **arch coordinates** (`+Z` occlusal): the collision
//! metric looks along the horizontal direction joining two crowns. A
//! [`ToothMove`] carries the rest frame `frame` (4x4, origin at the centre of
//! resistance, axes mesio-distal, bucco-lingual, long axis: the plugin's
//! `ToothGeom.frame()`, [`crate::teeth::rest_pose`]) and the crown points in
//! that frame.
//!
//! The poses stored in `AlignersState` (`Setup::poses`, `Stage::poses`) are
//! the local poses `q` of this module as a [`Pose`]: translation `q[0..3]`
//! and the rotation of the rotation vector `q[3..6]`, both in the rest
//! frame of the tooth. [`pose_from_q`] and [`q_from_pose`] convert,
//! [`world_delta`] gives the rigid transform of the tooth in the coordinates
//! of its rest frame. Every rotation vector is the shortest rotation (angle
//! in `[0, π]`, [`crate::rotation`]), whatever the sign of the stored
//! quaternion: the plugin reads its rotation vectors from matrices, with the
//! angle from the trace, so it never meets the sign of a quaternion.
//!
//! The crown points should be the closed crown of `reconstruct.rs`
//! (`reconstruct.closed_tooth`). Until that stage lands, the command feeds
//! the labelled vertices of the scan.

use std::collections::{BTreeMap, HashMap};

use dc_case::{Fdi, Pose};
use dca_model::{PerioStage, StagingPattern, StagingSettings, TOOTH_SPEED};
use glam::{DMat3, DMat4, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{anatomy, evidence, frame, rotation, teeth};

/// Movement of a tooth: `(t_md, t_bl, t_ax, r_md, r_bl, r_ax)` in the tooth
/// rest frame (mm and rad).
pub type Q6 = [f64; 6];

/// Names of the components of [`Q6`].
pub const COMPONENTS: [&str; 6] = [
    "mesiodistal",
    "buccolingual",
    "vertical",
    "torque",
    "tip",
    "rotation",
];

const H_MASK: [bool; 6] = [true, true, false, true, true, true];
const V_MASK: [bool; 6] = [false, false, true, false, false, false];
const ALL: [bool; 6] = [true; 6];

/// Distribution pattern of the movement over the aligners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    /// Chosen from the complexity of the case.
    #[default]
    Auto,
    /// All teeth move together and finish together (lowest force).
    Proportional,
    /// All teeth start together at maximum velocity ("X" pattern).
    MaxVelocity,
    /// Sequential distalization ("V" pattern).
    SequentialDistal,
    /// Space creation, alignment, then vertical movements.
    Structured,
}

impl From<StagingPattern> for Pattern {
    fn from(p: StagingPattern) -> Self {
        match p {
            StagingPattern::Auto => Pattern::Auto,
            StagingPattern::Proportional => Pattern::Proportional,
            StagingPattern::MaxVelocity => Pattern::MaxVelocity,
            StagingPattern::SequentialDistal => Pattern::SequentialDistal,
            StagingPattern::Structured => Pattern::Structured,
        }
    }
}

impl From<Pattern> for StagingPattern {
    fn from(p: Pattern) -> Self {
        match p {
            Pattern::Auto => StagingPattern::Auto,
            Pattern::Proportional => StagingPattern::Proportional,
            Pattern::MaxVelocity => StagingPattern::MaxVelocity,
            Pattern::SequentialDistal => StagingPattern::SequentialDistal,
            Pattern::Structured => StagingPattern::Structured,
        }
    }
}

/// Periodontal condition (reduces the movement per aligner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Perio {
    /// Healthy periodontium.
    #[default]
    Healthy,
    /// Stage I.
    Stage1,
    /// Stage II.
    Stage2,
    /// Stage III.
    Stage3,
}

impl From<PerioStage> for Perio {
    fn from(p: PerioStage) -> Self {
        match p {
            PerioStage::Healthy => Perio::Healthy,
            PerioStage::Stage1 => Perio::Stage1,
            PerioStage::Stage2 => Perio::Stage2,
            PerioStage::Stage3 => Perio::Stage3,
        }
    }
}

impl Perio {
    /// Linear step per aligner in mm (`perio_step` table of the evidence).
    pub fn step_mm(self) -> f64 {
        let key = match self {
            Perio::Healthy => "HEALTHY",
            Perio::Stage1 => "STAGE1",
            Perio::Stage2 => "STAGE2",
            Perio::Stage3 => "STAGE3",
        };
        evidence::param("perio_step")
            .and_then(|p| p.entry(key))
            .unwrap_or(match self {
                Perio::Healthy => 0.25,
                Perio::Stage1 => 0.18,
                Perio::Stage2 => 0.15,
                Perio::Stage3 => 0.10,
            })
    }
}

/// Overcorrection fractions (Ravera 2016 and others): the final movement is
/// increased by this fraction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Overcorrection {
    /// Torque.
    pub torque: f64,
    /// Tip.
    pub tip: f64,
    /// Rotation.
    pub rotation: f64,
    /// Extrusion.
    pub extrusion: f64,
    /// Intrusion.
    pub intrusion: f64,
}

impl Default for Overcorrection {
    fn default() -> Self {
        Self {
            torque: 0.205,
            tip: 0.145,
            rotation: 0.284,
            extrusion: 0.117,
            intrusion: 0.22,
        }
    }
}

fn ev(key: &str, fallback: f64) -> f64 {
    evidence::param_f64(key).unwrap_or(fallback)
}

/// Parameters of the staging (the plugin's `StagingParams`). The defaults
/// are the evidence values of [`crate::evidence::PARAMS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Largest translation per aligner (mm).
    pub step_linear_mm: f64,
    /// Largest intrusion or extrusion per aligner (mm).
    pub step_vertical_mm: f64,
    /// Largest rotation per aligner (deg).
    pub step_rotation_deg: f64,
    /// Largest rotation per aligner for canines and premolars (deg).
    pub step_rotation_round_deg: f64,
    /// Largest torque and tip per aligner (deg).
    pub step_angular_deg: f64,
    /// Largest move of any crown point per aligner (mm).
    pub max_crown_step_mm: f64,
    /// Periodontal condition.
    pub perio: Perio,
    /// Distribution pattern.
    pub pattern: Pattern,
    /// Delay the movement of teeth that would collide with a neighbour.
    pub collision: bool,
    /// Interpenetration of adjacent crowns tolerated at a stage (mm).
    pub collision_tol_mm: f64,
    /// Increase the final movement by the overcorrection fractions.
    pub overcorrection: bool,
    /// The fractions of the overcorrection.
    pub overcorrection_fractions: Overcorrection,
    /// Share of a phase after which the next one starts.
    pub seq_overlap: f64,
    /// Smallest anti-collision delay (aligners).
    pub delay_step: u32,
    /// Most anti-collision iterations.
    pub max_iter: u32,
    /// Crown points used by the collision metric.
    pub sample_points: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            step_linear_mm: ev("step_linear", 0.25),
            step_vertical_mm: ev("step_vertical", 0.25),
            step_rotation_deg: ev("step_rotation", 2.0),
            step_rotation_round_deg: ev("step_rotation_round", 1.5),
            step_angular_deg: ev("step_angular", 1.0),
            max_crown_step_mm: ev("max_crown_step", 0.33),
            perio: Perio::Healthy,
            pattern: Pattern::Auto,
            collision: true,
            collision_tol_mm: 0.10,
            overcorrection: false,
            overcorrection_fractions: Overcorrection::default(),
            seq_overlap: 0.5,
            delay_step: 2,
            max_iter: 300,
            sample_points: 2500,
        }
    }
}

impl Params {
    /// The parameters of the settings stored in the case.
    pub fn from_settings(s: &StagingSettings) -> Self {
        Self {
            step_linear_mm: s.step_linear_mm,
            step_vertical_mm: s.step_vertical_mm,
            step_rotation_deg: s.step_rotation_deg,
            step_rotation_round_deg: s.step_rotation_round_deg,
            step_angular_deg: s.step_angular_deg,
            max_crown_step_mm: s.max_crown_step_mm,
            perio: s.perio.into(),
            pattern: s.pattern.into(),
            collision: s.collision_check,
            collision_tol_mm: s.collision_tol_mm,
            overcorrection: s.overcorrection,
            ..Self::default()
        }
    }
}

/// One tooth to stage.
#[derive(Debug, Clone, PartialEq)]
pub struct ToothMove {
    /// Tooth number.
    pub fdi: Fdi,
    /// Rest frame (arch coordinates, origin at the centre of resistance).
    pub frame: DMat4,
    /// Final movement `q`.
    pub q: Q6,
    /// Starting movement (re-planning from a delivered aligner); `None` is
    /// the rest position.
    pub q0: Option<Q6>,
    /// Crown points in the rest frame.
    pub pts: Vec<DVec3>,
    /// `+1` when the local x axis points mesially, `-1` distally.
    pub md_sign: i8,
    /// Velocity factor (below 1 slows a tooth that did not track), inside
    /// [`TOOTH_SPEED`]: the planning refuses any other value.
    pub speed: f64,
}

impl ToothMove {
    /// Tooth that moves from its rest position to `q`.
    pub fn new(fdi: Fdi, frame: DMat4, q: Q6, pts: Vec<DVec3>, md_sign: i8) -> Self {
        Self {
            fdi,
            frame,
            q,
            q0: None,
            pts,
            md_sign,
            speed: 1.0,
        }
    }

    /// Tooth whose final position is given as a rigid transform `delta` of
    /// the rest position, in the coordinates of `frame` (`x' = delta x`):
    /// the plugin's `q_from_delta(frame, targets[f])`.
    pub fn from_delta(fdi: Fdi, frame: DMat4, delta: &DMat4, pts: Vec<DVec3>, md_sign: i8) -> Self {
        let q = q_from_delta(&frame, delta);
        Self::new(fdi, frame, q, pts, md_sign)
    }

    /// Starting movement.
    pub fn start(&self) -> Q6 {
        self.q0.unwrap_or([0.0; 6])
    }

    /// Movement still to do.
    pub fn delta(&self) -> Q6 {
        let s = self.start();
        std::array::from_fn(|i| self.q[i] - s[i])
    }
}

/// Difficulty level of the case (CAT-CAT consensus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Score up to 20.
    #[default]
    Easy,
    /// Score up to 40.
    Moderate,
    /// Score up to 60.
    Difficult,
    /// Score above 60.
    VeryComplex,
}

impl Level {
    /// The level of a CAT-CAT score (`planning.consensus_level`).
    pub fn from_score(score: f64) -> Self {
        if score <= 20.0 {
            Level::Easy
        } else if score <= 40.0 {
            Level::Moderate
        } else if score <= 60.0 {
            Level::Difficult
        } else {
            Level::VeryComplex
        }
    }
}

/// What the staging needs to know about the complexity of the case
/// (the plugin's `complexity` dictionary).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Complexity {
    /// Difficulty level.
    pub level: Level,
    /// Crowding (mm, positive when crowded).
    pub crowding_mm: f64,
    /// Little irregularity index (mm).
    pub little_mm: f64,
    /// Distal movement of the molars (mm, positive when distal).
    pub distalization_mm: f64,
    /// Pattern decided by the clinician; overrides the automatic choice.
    pub pattern_override: Option<Pattern>,
}

/// The pattern the automatic choice picks (`choose_pattern`).
pub fn choose_pattern(c: &Complexity) -> Pattern {
    if c.distalization_mm > 0.5 {
        Pattern::SequentialDistal
    } else if c.level != Level::Easy || c.crowding_mm > 3.0 || c.little_mm > 6.0 {
        Pattern::Structured
    } else {
        Pattern::Proportional
    }
}

/// Interproximal reduction planned on a contact.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IprPlan {
    /// First tooth of the contact.
    #[schemars(with = "u8")]
    pub a: Fdi,
    /// Second tooth of the contact.
    #[schemars(with = "u8")]
    pub b: Fdi,
    /// Enamel to remove, in total over both surfaces (mm).
    pub mm: f64,
}

/// Input of the staging.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The teeth to stage, **in arch order** (from the patient's right
    /// distal): adjacent contacts are taken from this order.
    pub moves: &'a [ToothMove],
    /// IPR planned by the setup.
    pub ipr: &'a [IprPlan],
    /// Complexity of the case (decides the `auto` pattern).
    pub complexity: Complexity,
}

/// Errors of the staging.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StagingError {
    /// The progress callback asked to stop.
    #[error("cancelled")]
    Cancelled,
    /// A step limit or the crown cap is not a positive finite number (a
    /// cap of 0 is not "no cap"), or the contact tolerance is negative or
    /// not finite.
    #[error("the staging parameter {name} is out of its range (got {value})")]
    InvalidLimit {
        /// Name of the parameter (a field of [`Params`]).
        name: &'static str,
        /// Its value.
        value: f64,
    },
    /// The speed factor of a tooth is outside [`TOOTH_SPEED`] or not finite.
    #[error("tooth {fdi}: the speed {value} is outside {min}-{max}")]
    InvalidSpeed {
        /// The tooth.
        fdi: Fdi,
        /// Its speed factor.
        value: f64,
        /// Smallest speed.
        min: f64,
        /// Largest speed.
        max: f64,
    },
    /// A movement, the rest frame or a crown point of a tooth is not finite
    /// (the plugin raises here).
    #[error("tooth {fdi}: the {what} is not a finite number")]
    NotFinite {
        /// The tooth.
        fdi: Fdi,
        /// What is not finite.
        what: &'static str,
    },
    /// A tooth needs more than [`MAX_ALIGNERS`] aligners.
    #[error("tooth {fdi} needs more than {max} aligners: check its step limits and speed")]
    TooManyAligners {
        /// The tooth.
        fdi: Fdi,
        /// The largest number of aligners of a tooth.
        max: i64,
    },
}

/// Most aligners a tooth may need: the plugin numbers the aligners up to 999
/// (`props.py`, `visit_last_aligner` and `replan_stage`). A count above it
/// comes from limits or a speed far outside their ranges, and would hang the
/// collision scan or exhaust the memory.
pub const MAX_ALIGNERS: i64 = 999;

/// A phase of the movement of a tooth: the components in `mask` go from 0 to
/// their final value in `n` aligners starting after aligner `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Components moved by this phase.
    pub mask: [bool; 6],
    /// Aligner after which the phase starts.
    pub start: i64,
    /// Length in aligners.
    pub n: i64,
}

/// IPR timing: the aligner before which the contact needs the reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IprStage {
    /// First tooth of the contact.
    pub a: Fdi,
    /// Second tooth of the contact.
    pub b: Fdi,
    /// Aligner (1-based).
    pub stage: u32,
}

/// A collision that no delay could solve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Unresolved {
    /// Aligner.
    pub stage: u32,
    /// First tooth.
    pub a: Fdi,
    /// Second tooth.
    pub b: Fdi,
    /// Interpenetration (mm).
    pub penetration_mm: f64,
}

/// Interpenetration of a contact in the final setup.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Overlap {
    /// First tooth.
    pub a: Fdi,
    /// Second tooth.
    pub b: Fdi,
    /// Interpenetration (mm).
    pub mm: f64,
}

/// Movement statistics of a tooth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToothStats {
    /// Largest step per aligner of each component.
    pub max_step: Q6,
    /// The limits per aligner of each component.
    pub limits: Q6,
    /// Cap of a crown point per aligner (mm).
    pub cap: f64,
    /// Largest displacement of a crown point in one aligner (mm).
    pub max_crown_step: f64,
    /// Aligners in which the tooth moves.
    pub moving_stages: u32,
}

/// A step of a plan over its limit per aligner: what the post-condition of
/// the planning reports (the limits hold by construction; this catches what
/// would break them).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct OverLimit {
    /// The tooth.
    pub fdi: Fdi,
    /// The component (a name of [`COMPONENTS`]), or `crown` for the cap of
    /// a crown point.
    pub what: &'static str,
    /// The largest step (mm, or rad for torque, tip and rotation).
    pub step: f64,
    /// Its limit per aligner.
    pub limit: f64,
}

/// Result of the staging.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// Pattern used.
    pub pattern: Pattern,
    /// Number of aligners.
    pub n_stages: u32,
    /// Movement `q` of each tooth at stage 0..=`n_stages`.
    pub q: BTreeMap<Fdi, Vec<Q6>>,
    /// Phases of each tooth.
    pub phases: BTreeMap<Fdi, Vec<Phase>>,
    /// Aligners each tooth needs by itself.
    pub need: BTreeMap<Fdi, u32>,
    /// IPR timing, in the order of the contacts.
    pub ipr_stage: Vec<IprStage>,
    /// Collisions that remain.
    pub unresolved: Vec<Unresolved>,
    /// Interpenetration of the contacts in the final setup.
    pub final_overlap: Vec<Overlap>,
    /// Statistics per tooth.
    pub stats: BTreeMap<Fdi, ToothStats>,
    /// Final movement of each tooth (after overcorrection, if any).
    pub q_final: BTreeMap<Fdi, Q6>,
    /// Steps over their limit per aligner (empty in a sound plan; each one
    /// is also a note of `log`).
    pub over_limit: Vec<OverLimit>,
    /// Notes for the clinician.
    pub log: Vec<String>,
}

// ---- movement maths -----------------------------------------------------------------------------------------

fn rotvec(q: &Q6) -> DVec3 {
    DVec3::new(q[3], q[4], q[5])
}

/// The rigid transform of a movement in the rest frame (`local_pose`).
pub fn local_pose(q: &Q6) -> DMat4 {
    let r = rotation::rotvec_to_matrix(rotvec(q));
    DMat4::from_cols(
        r.x_axis.extend(0.0),
        r.y_axis.extend(0.0),
        r.z_axis.extend(0.0),
        DVec3::new(q[0], q[1], q[2]).extend(1.0),
    )
}

/// `q` from an arch-space delta `D` (`x' = D x`) of a tooth with rest frame
/// `frame` (`q_from_delta`).
pub fn q_from_delta(frame_m: &DMat4, delta: &DMat4) -> Q6 {
    let l = frame::rigid_inverse(frame_m) * *delta * *frame_m;
    let r = rotation::matrix_to_rotvec(&DMat3::from_mat4(l));
    let t = l.w_axis.truncate();
    [t.x, t.y, t.z, r.x, r.y, r.z]
}

/// The arch-space delta of a movement `q` of a tooth with rest frame
/// `frame`: `F L(q) F^-1`.
pub fn delta_from_q(frame_m: &DMat4, q: &Q6) -> DMat4 {
    *frame_m * local_pose(q) * frame::rigid_inverse(frame_m)
}

/// The movement `q` as a [`Pose`] (translation and rotation, both in the
/// rest frame of the tooth; the quaternion [`rotation::canonical`]).
pub fn pose_from_q(q: &Q6) -> Pose {
    Pose::new(
        DVec3::new(q[0], q[1], q[2]),
        rotation::from_rotvec(rotvec(q)),
    )
}

/// The movement `q` of a [`Pose`] stored by [`pose_from_q`]: the shortest
/// rotation vector, whatever the sign of the quaternion.
pub fn q_from_pose(p: &Pose) -> Q6 {
    let r = rotation::to_rotvec(p.q);
    [p.t.x, p.t.y, p.t.z, r.x, r.y, r.z]
}

/// The rigid transform of a tooth (`x' = D x`, in the coordinates of its
/// rest pose `rest`) for the local pose `local`: `rest * local * rest^-1`.
pub fn world_delta(rest: &Pose, local: &Pose) -> Pose {
    rest.compose(local).compose(&rest.inverse())
}

fn xform(m: &DMat4, pts: &[DVec3]) -> Vec<DVec3> {
    let r = DMat3::from_mat4(*m);
    let t = m.w_axis.truncate();
    pts.iter().map(|&p| r * p + t).collect()
}

fn max_abs(q: &Q6) -> f64 {
    q.iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// Clinical description of a movement (for the reports): mesial(+), buccal(+),
/// extrusion(+) in mm and crown torque buccal(+), crown tip mesial(+) and
/// mesio-buccal rotation(+) in degrees.
pub fn describe_q(q: &Q6, md_sign: i8) -> [f64; 6] {
    let s = f64::from(md_sign);
    [
        q[0] * s,
        q[1],
        q[2],
        -q[3].to_degrees(),
        q[4].to_degrees() * s,
        q[5].to_degrees() * s,
    ]
}

// ---- limits and number of aligners -------------------------------------------------------------------------

/// Velocity limits per aligner of one tooth (mm and rad) and its crown cap.
pub fn limits(fdi: Fdi, p: &Params) -> (Q6, f64) {
    let mut scale = 1.0;
    let (mut lin, mut vert, mut cap) = (p.step_linear_mm, p.step_vertical_mm, p.max_crown_step_mm);
    if p.perio != Perio::Healthy {
        let pv = p.perio.step_mm();
        scale = pv / 0.25;
        lin = lin.min(pv);
        vert = vert.min(pv);
        cap = cap.min(pv);
    }
    let rot = if anatomy::is_round_crown(fdi) {
        p.step_rotation_round_deg
    } else {
        p.step_rotation_deg
    };
    let ang = (p.step_angular_deg * scale).to_radians();
    ([lin, lin, vert, ang, ang, (rot * scale).to_radians()], cap)
}

/// The limits of a tooth times its speed factor ([`check_input`] keeps it
/// inside [`TOOTH_SPEED`]).
fn tooth_limits(m: &ToothMove, p: &Params) -> (Q6, f64) {
    let (lim, cap) = limits(m.fdi, p);
    (lim.map(|l| l * m.speed), cap * m.speed)
}

fn apply_overcorrection(q: &Q6, oc: &Overcorrection) -> Q6 {
    let mut q = *q;
    q[3] *= 1.0 + oc.torque;
    q[4] *= 1.0 + oc.tip;
    q[5] *= 1.0 + oc.rotation;
    q[2] *= 1.0
        + if q[2] > 0.0 {
            oc.extrusion
        } else {
            oc.intrusion
        };
    q
}

/// Smallest number of aligners for the movement `q` (only the components in
/// `mask`). `None` when a limit or the cap is not a positive finite number,
/// the movement is not finite, or the count is above [`MAX_ALIGNERS`].
pub fn stages_needed(
    q: &Q6,
    pts: &[DVec3],
    lim: &Q6,
    cap: f64,
    mask: Option<&[bool; 6]>,
) -> Option<i64> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(cap) || !lim.iter().all(|&l| positive(l)) || !q.iter().all(|v| v.is_finite()) {
        return None;
    }
    let qq: Q6 = std::array::from_fn(|i| match mask {
        Some(m) if !m[i] => 0.0,
        _ => q[i],
    });
    if max_abs(&qq) < 1e-9 {
        return Some(0);
    }
    // checked: a huge or infinite ratio is no count (`as i64` would saturate)
    let count = |ratio: f64| {
        let n = (ratio - 1e-9).ceil();
        (n <= MAX_ALIGNERS as f64).then_some(n as i64)
    };
    let mut n = 1;
    for i in 0..6 {
        n = n.max(count(qq[i].abs() / lim[i])?);
    }
    let t = DVec3::new(qq[0], qq[1], qq[2]).length();
    let th = DVec3::new(qq[3], qq[4], qq[5]).length();
    let mut r = 0.0_f64;
    if th > 1e-12 && !pts.is_empty() {
        let ax = DVec3::new(qq[3], qq[4], qq[5]) / th;
        r = pts
            .iter()
            .map(|&p| (p - ax * p.dot(ax)).length())
            .fold(0.0, f64::max);
    }
    Some(n.max(count((t + r * th) / cap)?))
}

/// Components of a movement that create space in the arch (Martinez-Lozano
/// 2024): bucco-labial translation, labial crown torque of anterior teeth,
/// distal movement of posterior teeth.
fn space_mask(m: &ToothMove) -> [bool; 6] {
    let q = m.delta();
    let mut mask = [false; 6];
    if q[1] > 1e-6 {
        mask[1] = true;
    }
    if anatomy::index(m.fdi) >= 4 && q[0] * f64::from(m.md_sign) < -1e-6 {
        mask[0] = true;
    }
    if anatomy::is_anterior(m.fdi) && -q[3] > 1e-6 {
        mask[3] = true;
    }
    mask
}

// ---- schedule ----------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Track {
    start: Q6,
    delta: Q6,
    phases: Vec<Phase>,
}

#[derive(Debug, Clone)]
struct Schedule {
    tracks: BTreeMap<Fdi, Track>,
}

impl Schedule {
    fn new(moves: &BTreeMap<Fdi, ToothMove>) -> Self {
        Self {
            tracks: moves
                .iter()
                .map(|(&f, m)| {
                    (
                        f,
                        Track {
                            start: m.start(),
                            delta: m.delta(),
                            phases: Vec::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn q_at(&self, f: Fdi, k: i64) -> Q6 {
        let t = &self.tracks[&f];
        let mut q = t.start;
        for ph in &t.phases {
            let p = if ph.n <= 0 {
                if k >= ph.start { 1.0 } else { 0.0 }
            } else {
                ((k - ph.start) as f64 / ph.n as f64).clamp(0.0, 1.0)
            };
            for (i, v) in q.iter_mut().enumerate() {
                *v += if ph.mask[i] { t.delta[i] } else { 0.0 } * p;
            }
        }
        q
    }

    fn end(&self, f: Fdi) -> i64 {
        self.tracks[&f]
            .phases
            .iter()
            .map(|p| p.start + p.n)
            .max()
            .unwrap_or(0)
            .max(0)
    }

    fn n_stages(&self) -> i64 {
        self.tracks.keys().map(|&f| self.end(f)).max().unwrap_or(0)
    }

    fn delay(&mut self, f: Fdi, d: i64) {
        if let Some(t) = self.tracks.get_mut(&f) {
            for ph in &mut t.phases {
                ph.start += d;
            }
        }
    }
}

fn crown_steps(sch: &Schedule, f: Fdi, pts: &[DVec3]) -> Vec<f64> {
    let n = sch.end(f);
    if n <= 0 {
        return Vec::new();
    }
    let mut out = vec![0.0; n as usize];
    let mut prev: Option<Vec<DVec3>> = None;
    for k in 0..=n {
        let p = xform(&local_pose(&sch.q_at(f, k)), pts);
        if let Some(prev) = &prev {
            out[(k - 1) as usize] = p
                .iter()
                .zip(prev)
                .map(|(a, b)| (*a - *b).length())
                .fold(0.0, f64::max);
        }
        prev = Some(p);
    }
    out
}

/// Stretches the phases of a tooth whenever overlapping phases make a crown
/// point move more than the per-aligner cap (Wang 2023).
fn enforce_cap(sch: &mut Schedule, moves: &BTreeMap<Fdi, ToothMove>, p: &Params) {
    const MAX_ROUNDS: usize = 6;
    for (&f, m) in moves {
        let (_, cap) = tooth_limits(m, p);
        let cap = cap * 0.98; // margin for the discrete sampling of the crown
        let step = (m.pts.len() / 400).max(1);
        let pts: Vec<DVec3> = m.pts.iter().copied().step_by(step).collect();
        if pts.is_empty() {
            continue;
        }
        for _ in 0..MAX_ROUNDS {
            let steps = crown_steps(sch, f, &pts);
            let max = steps.iter().copied().fold(0.0, f64::max);
            if steps.is_empty() || max <= cap + 1e-9 {
                break;
            }
            let ratio = max / cap;
            let bad: Vec<i64> = steps
                .iter()
                .enumerate()
                .filter(|(_, s)| **s > cap + 1e-9)
                .map(|(i, _)| i as i64 + 1)
                .collect();
            if let Some(t) = sch.tracks.get_mut(&f) {
                for ph in &mut t.phases {
                    let (s0, n) = (ph.start, ph.n);
                    if n > 0 && bad.iter().any(|&b| b > s0 && b <= s0 + n) {
                        ph.n = (n as f64 * ratio).ceil() as i64;
                    }
                }
            }
        }
    }
}

fn build_schedule(
    moves: &BTreeMap<Fdi, ToothMove>,
    p: &Params,
    pattern: Pattern,
) -> Result<(Schedule, BTreeMap<Fdi, i64>), StagingError> {
    let mut sch = Schedule::new(moves);
    let lim: BTreeMap<Fdi, (Q6, f64)> = moves
        .iter()
        .map(|(&f, m)| (f, tooth_limits(m, p)))
        .collect();
    let needed = |f: Fdi, mask: Option<&[bool; 6]>| {
        let m = &moves[&f];
        let (l, c) = &lim[&f];
        stages_needed(&m.delta(), &m.pts, l, *c, mask)
            .map(|n| (f, n))
            .ok_or(StagingError::TooManyAligners {
                fdi: f,
                max: MAX_ALIGNERS,
            })
    };
    let need: BTreeMap<Fdi, i64> = moves
        .keys()
        .map(|&f| needed(f, None))
        .collect::<Result<_, _>>()?;
    let phase = |mask: [bool; 6], start: i64, n: i64| Phase { mask, start, n };
    match pattern {
        Pattern::MaxVelocity => {
            for &f in moves.keys() {
                set_phases(&mut sch, f, vec![phase(ALL, 0, need[&f])]);
            }
        }
        Pattern::Structured => {
            // macro-staging: (A) space creation, (B) alignment starting when A
            // is half way, (C) absolute vertical movements last
            let ma: BTreeMap<Fdi, [bool; 6]> =
                moves.iter().map(|(&f, m)| (f, space_mask(m))).collect();
            let mb: BTreeMap<Fdi, [bool; 6]> = ma
                .iter()
                .map(|(&f, a)| (f, std::array::from_fn(|i| H_MASK[i] && !a[i])))
                .collect();
            let na: BTreeMap<Fdi, i64> = moves
                .keys()
                .map(|&f| {
                    if ma[&f].iter().any(|&x| x) {
                        needed(f, Some(&ma[&f]))
                    } else {
                        Ok((f, 0))
                    }
                })
                .collect::<Result<_, _>>()?;
            let nb: BTreeMap<Fdi, i64> = moves
                .keys()
                .map(|&f| needed(f, Some(&mb[&f])))
                .collect::<Result<_, _>>()?;
            let nv: BTreeMap<Fdi, i64> = moves
                .keys()
                .map(|&f| needed(f, Some(&V_MASK)))
                .collect::<Result<_, _>>()?;
            let big_a = na.values().copied().max().unwrap_or(0);
            let big_b = nb.values().copied().max().unwrap_or(0);
            let sb = (p.seq_overlap * big_a as f64).ceil() as i64;
            let sv = sb + big_b;
            for &f in moves.keys() {
                set_phases(
                    &mut sch,
                    f,
                    vec![
                        phase(ma[&f], 0, if na[&f] != 0 { big_a } else { 0 }),
                        phase(mb[&f], sb, if nb[&f] != 0 { big_b } else { 0 }),
                        phase(V_MASK, sv, nv[&f]),
                    ],
                );
            }
        }
        Pattern::SequentialDistal => {
            let mut starts: BTreeMap<Fdi, i64> = BTreeMap::new();
            let mut side_end = Vec::new();
            for right in [true, false] {
                let mut side: Vec<Fdi> = moves
                    .keys()
                    .copied()
                    .filter(|&f| anatomy::is_right(f) == right && anatomy::index(f) >= 3)
                    .collect();
                // distal -> mesial (stable)
                side.sort_by_key(|&f| std::cmp::Reverse(anatomy::index(f)));
                let mut s = 0_i64;
                let mut prev: Option<Fdi> = None;
                for f in side {
                    if need[&f] == 0 {
                        starts.insert(f, s);
                        continue;
                    }
                    if let Some(pv) = prev {
                        s = starts[&pv] + (p.seq_overlap * need[&pv] as f64).ceil() as i64;
                    }
                    starts.insert(f, s);
                    prev = Some(f);
                }
                side_end.push(match prev {
                    Some(pv) => starts[&pv] + (p.seq_overlap * need[&pv] as f64).ceil() as i64,
                    None => 0,
                });
            }
            let inc_start = side_end.into_iter().max().unwrap_or(0);
            for &f in moves.keys() {
                let st = starts.get(&f).copied().unwrap_or(inc_start);
                set_phases(&mut sch, f, vec![phase(ALL, st, need[&f])]);
            }
        }
        Pattern::Proportional | Pattern::Auto => {
            let n = need.values().copied().max().unwrap_or(0);
            for &f in moves.keys() {
                set_phases(
                    &mut sch,
                    f,
                    vec![phase(ALL, 0, if need[&f] != 0 { n } else { 0 })],
                );
            }
        }
    }
    Ok((sch, need))
}

fn set_phases(sch: &mut Schedule, f: Fdi, phases: Vec<Phase>) {
    if let Some(t) = sch.tracks.get_mut(&f) {
        t.phases = phases;
    }
}

// ---- collisions --------------------------------------------------------------------------------------------

/// Signed clearance between two point-sampled crowns along the horizontal
/// direction `u` (a to b): the minimum over the cells of a grid on the plane
/// orthogonal to `u` of `min_b u - max_a u`. Negative means interpenetration;
/// `None` when the crowns do not face each other (`contact_separation`).
fn contact_separation(pa: &[DVec3], pb: &[DVec3], u: DVec3) -> Option<f64> {
    const CELL: f64 = 0.35;
    let up = DVec3::Z;
    let v = (up - u * up.dot(u)).normalize_or_zero();
    let w = u.cross(v);
    if pa.is_empty() || pb.is_empty() {
        return None;
    }
    let cell = |p: DVec3| -> i64 {
        ((p.dot(v) / CELL).floor() as i64)
            .wrapping_mul(1_000_003)
            .wrapping_add((p.dot(w) / CELL).floor() as i64)
    };
    let mut top_a: HashMap<i64, f64> = HashMap::with_capacity(pa.len());
    for &p in pa {
        let e = top_a.entry(cell(p)).or_insert(f64::NEG_INFINITY);
        *e = e.max(p.dot(u));
    }
    let mut low_b: HashMap<i64, f64> = HashMap::with_capacity(pb.len());
    for &p in pb {
        let e = low_b.entry(cell(p)).or_insert(f64::INFINITY);
        *e = e.min(p.dot(u));
    }
    top_a
        .iter()
        .filter_map(|(k, &a)| low_b.get(k).map(|&b| b - a))
        .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
}

/// Clearance between two crowns (arch coordinates) along the horizontal
/// direction joining their centroids, using the facing halves only.
fn facing_clearance(pa: &[DVec3], pb: &[DVec3]) -> Option<f64> {
    if pa.is_empty() || pb.is_empty() {
        return None;
    }
    let ca = pa.iter().copied().sum::<DVec3>() / pa.len() as f64;
    let cb = pb.iter().copied().sum::<DVec3>() / pb.len() as f64;
    let mut u = cb - ca;
    u.z = 0.0;
    if u.length() < 1e-9 {
        return None;
    }
    let u = u.normalize();
    let fa: Vec<DVec3> = pa
        .iter()
        .copied()
        .filter(|&p| (p - ca).dot(u) > 0.0)
        .collect();
    let fb: Vec<DVec3> = pb
        .iter()
        .copied()
        .filter(|&p| (p - cb).dot(u) < 0.0)
        .collect();
    contact_separation(&fa, &fb, u)
}

type PenKey = (Fdi, Fdi, [u64; 6], [u64; 6]);

/// Interpenetration of adjacent crowns measured along the direction joining
/// them. Crown points within 0.6 mm of the cervical margin are ignored.
struct CollisionChecker<'a> {
    moves: &'a BTreeMap<Fdi, ToothMove>,
    pairs: Vec<(Fdi, Fdi)>,
    pts: BTreeMap<Fdi, Vec<DVec3>>,
    memo: HashMap<PenKey, f64>,
}

fn bits(q: &Q6) -> [u64; 6] {
    q.map(f64::to_bits)
}

impl<'a> CollisionChecker<'a> {
    fn new(moves: &'a BTreeMap<Fdi, ToothMove>, pairs: &[(Fdi, Fdi)], n_samples: usize) -> Self {
        let mut pts = BTreeMap::new();
        for (&f, m) in moves {
            if m.pts.is_empty() {
                continue;
            }
            let zmin = m.pts.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
            let upper: Vec<DVec3> = m.pts.iter().copied().filter(|p| p.z > zmin + 0.6).collect();
            pts.insert(f, teeth::subsample(&upper, n_samples));
        }
        Self {
            moves,
            pairs: pairs
                .iter()
                .copied()
                .filter(|(a, b)| moves.contains_key(a) && moves.contains_key(b))
                .collect(),
            pts,
            memo: HashMap::new(),
        }
    }

    fn penetration(&mut self, a: Fdi, b: Fdi, qa: &Q6, qb: &Q6) -> f64 {
        let (Some(pa), Some(pb)) = (self.pts.get(&a), self.pts.get(&b)) else {
            return 0.0;
        };
        let key = (a, b, bits(qa), bits(qb));
        if let Some(&hit) = self.memo.get(&key) {
            return hit;
        }
        let wa = xform(&(self.moves[&a].frame * local_pose(qa)), pa);
        let wb = xform(&(self.moves[&b].frame * local_pose(qb)), pb);
        let hit = match facing_clearance(&wa, &wb) {
            Some(sep) if sep.is_finite() => (-sep).max(0.0),
            _ => 0.0,
        };
        self.memo.insert(key, hit);
        hit
    }
}

/// Pairs of neighbouring teeth along the arch (`planning.adjacent_pairs`).
pub fn adjacent_pairs(seq: &[Fdi]) -> Vec<(Fdi, Fdi)> {
    seq.windows(2)
        .filter(|w| {
            anatomy::is_right(w[0]) == anatomy::is_right(w[1])
                || (anatomy::index(w[0]) == 1 && anatomy::index(w[1]) == 1)
        })
        .map(|w| (w[0], w[1]))
        .collect()
}

fn ipr_mm(ipr: &[IprPlan], a: Fdi, b: Fdi) -> f64 {
    ipr.iter()
        .find(|r| r.a == a && r.b == b)
        .or_else(|| ipr.iter().find(|r| r.a == b && r.b == a))
        .map_or(0.0, |r| r.mm)
}

// ---- the plan ----------------------------------------------------------------------------------------------

/// Plans the aligners of the teeth (`staging.plan`).
pub fn run(input: &Input<'_>, params: &Params) -> Result<Plan, StagingError> {
    run_with_progress(input, params, &mut |_| true)
}

/// [`run`] with a progress callback (a fraction in `0..1`); the planning
/// stops with [`StagingError::Cancelled`] when it returns `false`.
pub fn run_with_progress(
    input: &Input<'_>,
    p: &Params,
    progress: &mut dyn FnMut(f64) -> bool,
) -> Result<Plan, StagingError> {
    check_input(input, p)?;
    let seq: Vec<Fdi> = input.moves.iter().map(|m| m.fdi).collect();
    let pairs = adjacent_pairs(&seq);
    let mut moves: BTreeMap<Fdi, ToothMove> =
        input.moves.iter().map(|m| (m.fdi, m.clone())).collect();
    let ipr = input.ipr;

    let pattern = match p.pattern {
        Pattern::Auto => input
            .complexity
            .pattern_override
            .filter(|&o| o != Pattern::Auto)
            .unwrap_or_else(|| choose_pattern(&input.complexity)),
        other => other,
    };
    if p.overcorrection {
        for m in moves.values_mut() {
            let s = m.start();
            let d = apply_overcorrection(&m.delta(), &p.overcorrection_fractions);
            m.q = std::array::from_fn(|i| s[i] + d[i]);
        }
    }
    let (mut sch, need) = build_schedule(&moves, p, pattern)?;
    enforce_cap(&mut sch, &moves, p);
    let mut log = vec![format!("Staging pattern: {}", pattern_name(pattern))];
    let mut unresolved = Vec::new();
    let mut ipr_stage: Vec<IprStage> = Vec::new();
    let mut final_overlap: Vec<Overlap> = Vec::new();

    if p.collision && moves.len() > 1 {
        let mut cc = CollisionChecker::new(&moves, &pairs, p.sample_points);
        // (pair, allowed penetration, start threshold, ipr)
        let mut base: Vec<((Fdi, Fdi), [f64; 3])> = Vec::new();
        let cc_pairs = cc.pairs.clone();
        for &(a, b) in &cc_pairs {
            let (ma, mb) = (&moves[&a], &moves[&b]);
            let pen0 = cc.penetration(a, b, &ma.start(), &mb.start());
            let penf = cc.penetration(a, b, &ma.q, &mb.q);
            final_overlap.push(Overlap { a, b, mm: penf });
            let allow_ipr = ipr_mm(ipr, a, b);
            let allowed = (pen0 + p.collision_tol_mm + allow_ipr).max(penf + p.collision_tol_mm);
            base.push(((a, b), [allowed, pen0 + p.collision_tol_mm, allow_ipr]));
        }
        let mut cache: HashMap<(Fdi, Fdi, i64), f64> = HashMap::new();
        let mut total_delay: BTreeMap<Fdi, i64> = moves.keys().map(|&f| (f, 0)).collect();
        let max_delay: BTreeMap<Fdi, i64> =
            moves.keys().map(|&f| (f, 12.max(2 * need[&f]))).collect();

        macro_rules! pen_at {
            ($a:expr, $b:expr, $k:expr) => {{
                let (a, b, k): (Fdi, Fdi, i64) = ($a, $b, $k);
                match cache.get(&(a, b, k)) {
                    Some(&v) => v,
                    None => {
                        let v = cc.penetration(a, b, &sch.q_at(a, k), &sch.q_at(b, k));
                        cache.insert((a, b, k), v);
                        v
                    }
                }
            }};
        }

        let mut it = 0_u32;
        let mut converged = false;
        while it < p.max_iter {
            it += 1;
            let n = sch.n_stages();
            let mut viol: Option<(i64, Fdi, Fdi)> = None;
            'search: for k in 1..=n {
                for &((a, b), [thr, _, _]) in &base {
                    if pen_at!(a, b, k) > thr {
                        viol = Some((k, a, b));
                        break 'search;
                    }
                }
            }
            if !progress((it as f64 / p.max_iter as f64).min(0.99)) {
                return Err(StagingError::Cancelled);
            }
            let Some((k, a, b)) = viol else {
                converged = true;
                break;
            };
            let pen = pen_at!(a, b, k);
            // delay proportional to the interference (stages needed to clear it)
            let dly = (p.delay_step as f64)
                .max((pen / (0.5 * p.step_linear_mm).max(0.05)).ceil())
                .min(8.0) as i64;
            let mut best: Option<(f64, i64, Fdi)> = None;
            for x in [a, b] {
                let movable = sch.tracks[&x].phases.iter().any(|ph| ph.n > 0);
                if total_delay[&x] + dly > max_delay[&x] || !movable {
                    continue;
                }
                let qa = sch.q_at(a, if x == a { k - dly } else { k });
                let qb = sch.q_at(b, if x == b { k - dly } else { k });
                let trial = cc.penetration(a, b, &qa, &qb);
                let cand = (trial, total_delay[&x], x);
                if best.is_none_or(|bst| {
                    (cand.0, cand.1, cand.2)
                        .partial_cmp(&(bst.0, bst.1, bst.2))
                        .is_some_and(|o| o.is_lt())
                }) {
                    best = Some(cand);
                }
            }
            let Some((_, _, f)) = best else {
                unresolved.push(Unresolved {
                    stage: k as u32,
                    a,
                    b,
                    penetration_mm: pen,
                });
                if let Some(e) = base.iter_mut().find(|(pair, _)| *pair == (a, b)) {
                    e.1[0] = 1e9;
                }
                continue;
            };
            sch.delay(f, dly);
            *total_delay.entry(f).or_insert(0) += dly;
            cache.retain(|key, _| key.0 != f && key.1 != f);
        }
        if !converged {
            log.push("Maximum number of anti-collision iterations reached".to_owned());
        }
        let delayed: Vec<String> = total_delay
            .iter()
            .filter(|(_, d)| **d != 0)
            .map(|(f, d)| format!("{f}: {d}"))
            .collect();
        if !delayed.is_empty() {
            log.push(format!(
                "Anti-collision delays (aligners): {}",
                delayed.join(", ")
            ));
        }
        let n = sch.n_stages();
        for &((a, b), [_, thr0, allow_ipr]) in &base {
            if allow_ipr > 0.0 {
                let first = (1..=n).find(|&k| pen_at!(a, b, k) > thr0);
                ipr_stage.push(IprStage {
                    a,
                    b,
                    stage: first.unwrap_or(n) as u32,
                });
            }
        }
        for o in &final_overlap {
            if o.mm > p.collision_tol_mm + ipr_mm(ipr, o.a, o.b) {
                log.push(format!(
                    "Residual overlap in the final setup {}|{}: {:.2} mm (correct the setup or add IPR)",
                    o.a, o.b, o.mm
                ));
            }
        }
    } else {
        for r in ipr {
            ipr_stage.push(IprStage {
                a: r.a,
                b: r.b,
                stage: 1,
            });
        }
    }

    let n = sch.n_stages();
    // the plugin keeps the stage of such an IPR (0, or 1 of a plan without
    // aligners): no aligner carries it, so it is reported
    for r in &ipr_stage {
        if r.stage == 0 || i64::from(r.stage) > n {
            log.push(format!(
                "IPR {}|{} is in no aligner (stage {} of a plan of {n} aligners): do it by hand",
                r.a, r.b, r.stage
            ));
        }
    }
    let q: BTreeMap<Fdi, Vec<Q6>> = moves
        .keys()
        .map(|&f| (f, (0..=n).map(|k| sch.q_at(f, k)).collect()))
        .collect();
    let stats = stage_stats(&moves, &q, p);
    let over_limit = over_limits(&stats);
    for o in &over_limit {
        let (step, limit, unit) = if o.what == "crown" || COMPONENTS[..3].contains(&o.what) {
            (o.step, o.limit, "mm")
        } else {
            (o.step.to_degrees(), o.limit.to_degrees(), "deg")
        };
        log.push(format!(
            "Over the limit: tooth {} moves {step:.4} {unit} ({}) in one aligner, the limit is {limit:.4} {unit}",
            o.fdi, o.what
        ));
    }
    Ok(Plan {
        pattern,
        n_stages: n as u32,
        q,
        phases: sch
            .tracks
            .iter()
            .map(|(&f, t)| (f, t.phases.clone()))
            .collect(),
        need: need.iter().map(|(&f, &n)| (f, n as u32)).collect(),
        ipr_stage,
        unresolved,
        final_overlap,
        stats,
        q_final: moves.iter().map(|(&f, m)| (f, m.q)).collect(),
        over_limit,
        log,
    })
}

/// Refuses what would be planned without limits or as NaN poses: a step
/// limit or crown cap that is not a positive finite number (a cap of 0 is
/// not "no cap"), a negative or non-finite contact tolerance, a speed factor
/// outside [`TOOTH_SPEED`], a movement, rest frame or crown point that is not
/// finite (the plugin raises on a NaN movement). The command checks the
/// ranges of the settings too; this guards every other caller.
fn check_input(input: &Input<'_>, p: &Params) -> Result<(), StagingError> {
    for (name, value) in [
        ("step_linear_mm", p.step_linear_mm),
        ("step_vertical_mm", p.step_vertical_mm),
        ("step_rotation_deg", p.step_rotation_deg),
        ("step_rotation_round_deg", p.step_rotation_round_deg),
        ("step_angular_deg", p.step_angular_deg),
        ("max_crown_step_mm", p.max_crown_step_mm),
    ] {
        if !(value.is_finite() && value > 0.0) {
            return Err(StagingError::InvalidLimit { name, value });
        }
    }
    if !(p.collision_tol_mm.is_finite() && p.collision_tol_mm >= 0.0) {
        return Err(StagingError::InvalidLimit {
            name: "collision_tol_mm",
            value: p.collision_tol_mm,
        });
    }
    let (min, max) = TOOTH_SPEED;
    let finite = |v: &Q6| v.iter().all(|x| x.is_finite());
    for m in input.moves {
        let fdi = m.fdi;
        if !(m.speed.is_finite() && (min - 1e-9..=max + 1e-9).contains(&m.speed)) {
            return Err(StagingError::InvalidSpeed {
                fdi,
                value: m.speed,
                min,
                max,
            });
        }
        let what = if !finite(&m.q) {
            "final movement"
        } else if m.q0.is_some_and(|q0| !finite(&q0)) {
            "starting movement"
        } else if !m.frame.is_finite() {
            "rest frame"
        } else if !m.pts.iter().all(|p| p.is_finite()) {
            "crown points"
        } else {
            continue;
        };
        return Err(StagingError::NotFinite { fdi, what });
    }
    Ok(())
}

/// The post-condition of the planning: every step of a component within
/// its limit and every step of a crown point within the cap, with the
/// tolerance of [`stages_needed`] (`1e-9` relative, `1e-12` absolute).
fn over_limits(stats: &BTreeMap<Fdi, ToothStats>) -> Vec<OverLimit> {
    let over = |step: f64, limit: f64| step.is_nan() || step > limit * (1.0 + 1e-9) + 1e-12;
    let mut out = Vec::new();
    for (&fdi, st) in stats {
        for (i, &what) in COMPONENTS.iter().enumerate() {
            if over(st.max_step[i], st.limits[i]) {
                out.push(OverLimit {
                    fdi,
                    what,
                    step: st.max_step[i],
                    limit: st.limits[i],
                });
            }
        }
        if over(st.max_crown_step, st.cap) {
            out.push(OverLimit {
                fdi,
                what: "crown",
                step: st.max_crown_step,
                limit: st.cap,
            });
        }
    }
    out
}

fn pattern_name(p: Pattern) -> &'static str {
    match p {
        Pattern::Auto => "AUTO",
        Pattern::Proportional => "PROPORTIONAL",
        Pattern::MaxVelocity => "MAX_VELOCITY",
        Pattern::SequentialDistal => "SEQUENTIAL_DISTAL",
        Pattern::Structured => "STRUCTURED",
    }
}

fn stage_stats(
    moves: &BTreeMap<Fdi, ToothMove>,
    all_q: &BTreeMap<Fdi, Vec<Q6>>,
    p: &Params,
) -> BTreeMap<Fdi, ToothStats> {
    let mut out = BTreeMap::new();
    for (&f, qs) in all_q {
        let m = &moves[&f];
        let mut max_step = [0.0; 6];
        let mut moving = 0;
        for w in qs.windows(2) {
            let d: Q6 = std::array::from_fn(|i| (w[1][i] - w[0][i]).abs());
            for i in 0..6 {
                max_step[i] = f64::max(max_step[i], d[i]);
            }
            if d.iter().any(|&x| x > 1e-9) {
                moving += 1;
            }
        }
        let step = (m.pts.len() / 300).max(1);
        let sel: Vec<DVec3> = m.pts.iter().copied().step_by(step).collect();
        let mut max_disp = 0.0_f64;
        for w in qs.windows(2) {
            let a = xform(&local_pose(&w[0]), &sel);
            let b = xform(&local_pose(&w[1]), &sel);
            let d = a
                .iter()
                .zip(&b)
                .map(|(x, y)| (*y - *x).length())
                .fold(0.0, f64::max);
            max_disp = max_disp.max(d);
        }
        let (limits, cap) = tooth_limits(m, p);
        out.insert(
            f,
            ToothStats {
                max_step,
                limits,
                cap,
                max_crown_step: max_disp,
                moving_stages: moving,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// A box-shaped crown of 7 x 7 x 8 mm around the origin, in a frame at `x`.
    fn tooth(n: u8, x: f64, q: Q6) -> ToothMove {
        let mut pts = Vec::new();
        for i in 0..8 {
            for j in 0..8 {
                for k in 0..9 {
                    pts.push(DVec3::new(
                        -3.5 + i as f64 * 1.0,
                        -3.5 + j as f64 * 1.0,
                        -1.0 + k as f64,
                    ));
                }
            }
        }
        let frame = DMat4::from_translation(DVec3::new(x, 0.0, 0.0));
        ToothMove::new(fdi(n), frame, q, pts, 1)
    }

    #[test]
    fn q_and_delta_round_trip() {
        let frame = DMat4::from_rotation_translation(
            DQuat::from_rotation_z(0.4) * DQuat::from_rotation_x(0.1),
            DVec3::new(3.0, -2.0, 1.0),
        );
        let q = [0.5, -0.2, 0.1, 0.02, -0.03, 0.05];
        let d = delta_from_q(&frame, &q);
        let back = q_from_delta(&frame, &d);
        for i in 0..6 {
            assert!((q[i] - back[i]).abs() < 1e-12, "{i}");
        }
        let pose = pose_from_q(&q);
        let q2 = q_from_pose(&pose);
        for i in 0..6 {
            assert!((q[i] - q2[i]).abs() < 1e-12);
        }
        // world_delta agrees with delta_from_q
        let rest = Pose::from_mat4(&frame);
        let wd = world_delta(&rest, &pose).to_mat4();
        assert!(wd.abs_diff_eq(d, 1e-12));
    }

    /// The largest rotation (degrees) of a tooth over the stages of a plan, and
    /// the largest rotation from one stage to the next.
    fn turns(plan: &Plan, f: Fdi) -> (f64, f64) {
        let poses: Vec<Pose> = plan.q[&f].iter().map(pose_from_q).collect();
        let most = poses.iter().map(rotation::degrees).fold(0.0, f64::max);
        let step = poses
            .windows(2)
            .map(|w| rotation::degrees(&w[1].compose(&w[0].inverse())))
            .fold(0.0, f64::max);
        (most, step)
    }

    fn plan_one(q: Q6) -> Plan {
        let moves = [tooth(11, 0.0, q)];
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        run(&input, &Params::default()).unwrap()
    }

    #[test]
    fn a_stored_pose_with_negative_w_is_staged_the_short_way() {
        // the setup stored a 4 degree turn with w < 0 (the 719-stage anomaly): read the long
        // way round it was a 356 degree turn, about 180 aligners spinning the tooth
        let turn =
            DQuat::from_axis_angle(DVec3::new(0.1, -0.2, 1.0).normalize(), 4f64.to_radians());
        let t = DVec3::new(0.3, -0.2, 0.1);
        let stored = Pose { t, q: -turn };
        assert!(stored.q.w < 0.0);
        let q = q_from_pose(&stored);
        let short = q_from_pose(&Pose::new(t, turn));
        assert!(
            q.iter().zip(&short).all(|(a, b)| (a - b).abs() < 1e-15),
            "{q:?} {short:?}"
        );
        assert!((rotvec(&q).length().to_degrees() - 4.0).abs() < 1e-9);
        let plan = plan_one(q);
        assert_eq!(plan.n_stages, plan_one(short).n_stages);
        // 4 degrees about the long axis (2 per aligner) and 0.37 mm, with crown points 6 mm
        // from the axis: 0.8 mm of crown travel at 0.33 mm per aligner (the crown cap)
        assert_eq!(plan.n_stages, 3);
        let (most, step) = turns(&plan, fdi(11));
        assert!(most <= 4.0 + 1e-9 && step <= 2.0 + 1e-9, "{most} {step}");
        // the last aligner is the stored pose (the same rotation, w >= 0)
        let last = pose_from_q(plan.q[&fdi(11)].last().unwrap());
        assert!(last.t.distance(stored.t) < 1e-12);
        assert!(rotation::angle(last.q * stored.q.inverse()) < 1e-12);
        assert!(last.q.w >= 0.0);
    }

    #[test]
    fn a_turn_next_to_180_degrees_is_staged_the_short_way() {
        // 181 degrees one way is 179 degrees the other way: the same aligners
        for (deg, short) in [(179.0, 179.0), (181.0, -179.0)] {
            let q = q_from_pose(&Pose::from_rotation(DQuat::from_rotation_z(
                f64::to_radians(deg),
            )));
            assert!(
                (q[5] - f64::to_radians(short)).abs() < 1e-12,
                "{deg}: {q:?}"
            );
            let plan = plan_one(q);
            // 179 degrees at 2 degrees per aligner, capped by the crown travel
            let (most, step) = turns(&plan, fdi(11));
            assert!(
                most <= 179.0 + 1e-9 && step <= 2.0 + 1e-9,
                "{deg}: {most} {step}"
            );
            assert_eq!(
                plan.n_stages,
                plan_one([0.0, 0.0, 0.0, 0.0, 0.0, f64::to_radians(179.0)]).n_stages
            );
        }
    }

    #[test]
    fn a_tooth_needs_the_stages_its_largest_component_asks() {
        let (lim, cap) = limits(fdi(11), &Params::default());
        // 1 mm mesial at 0.25 mm per aligner: 4 aligners
        let q = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(stages_needed(&q, &[], &lim, cap, None), Some(4));
        // masked out: nothing to do
        let mask = [false, true, false, false, false, false];
        assert_eq!(stages_needed(&q, &[], &lim, cap, Some(&mask)), Some(0));
        // 10 degrees of rotation at 2 deg: 5 aligners; a canine is limited to 1.5 deg: 7
        let r = [0.0, 0.0, 0.0, 0.0, 0.0, 10.0_f64.to_radians()];
        assert_eq!(stages_needed(&r, &[], &lim, cap, None), Some(5));
        let (lim13, cap13) = limits(fdi(13), &Params::default());
        assert_eq!(stages_needed(&r, &[], &lim13, cap13, None), Some(7));
        // periodontal stage III: 0.10 mm per aligner
        let p = Params {
            perio: Perio::Stage3,
            ..Params::default()
        };
        let (lim3, cap3) = limits(fdi(11), &p);
        assert_eq!(stages_needed(&q, &[], &lim3, cap3, None), Some(10));
    }

    #[test]
    fn the_crown_cap_adds_aligners_for_a_long_lever() {
        let (lim, cap) = limits(fdi(11), &Params::default());
        // 2 degrees of tip on a crown point 20 mm from the axis: 0.7 mm of travel
        let q = [0.0, 0.0, 0.0, 0.0, 2.0_f64.to_radians(), 0.0];
        let pts = [DVec3::new(0.0, 0.0, 20.0)];
        assert_eq!(stages_needed(&q, &[], &lim, cap, None), Some(2));
        assert_eq!(stages_needed(&q, &pts, &lim, cap, None), Some(3));
    }

    #[test]
    fn two_teeth_move_together_and_finish_together() {
        let moves = [
            tooth(16, -12.0, [0.0; 6]),
            tooth(15, 0.0, [0.0, 0.75, 0.0, 0.0, 0.0, 0.0]),
        ];
        // leave room between the crowns so there is no collision
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        let plan = run(&input, &Params::default()).unwrap();
        assert_eq!(plan.pattern, Pattern::Proportional);
        assert_eq!(plan.n_stages, 3);
        assert_eq!(plan.q[&fdi(15)].len(), 4);
        assert!((plan.q[&fdi(15)][3][1] - 0.75).abs() < 1e-12);
        assert!((plan.q[&fdi(15)][1][1] - 0.25).abs() < 1e-12);
        assert_eq!(plan.need[&fdi(16)], 0);
        assert!(plan.q[&fdi(16)].iter().all(|q| *q == [0.0; 6]));
        assert!(plan.unresolved.is_empty());
        assert_eq!(plan.stats[&fdi(15)].moving_stages, 3);
    }

    #[test]
    fn a_tooth_that_runs_into_its_neighbour_is_delayed() {
        // two crowns touching at 7 mm: 15 moves 1 mm towards 14 at 0.25 mm per
        // aligner, 14 follows at half that speed, so 15 would push into it
        let a = tooth(15, -7.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let mut b = tooth(14, 0.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        b.speed = 0.5;
        let moves = [a, b];
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        let p = Params {
            pattern: Pattern::MaxVelocity,
            ..Params::default()
        };
        let plan = run(&input, &p).unwrap();
        assert!(
            plan.log.iter().any(|l| l.contains("delays")),
            "{:?}",
            plan.log
        );
        assert!(plan.unresolved.is_empty());
        assert!(plan.phases[&fdi(15)][0].start > 0);
        // the crowns never overlap by more than the tolerance
        for (qa, qb) in plan.q[&fdi(15)].iter().zip(&plan.q[&fdi(14)]) {
            assert!(qa[0] - qb[0] <= p.collision_tol_mm + 1e-9, "{qa:?} {qb:?}");
        }
        let free = run(
            &input,
            &Params {
                collision: false,
                ..p.clone()
            },
        )
        .unwrap();
        assert_eq!(free.phases[&fdi(15)][0].start, 0);
        assert!(plan.n_stages >= free.n_stages);
        // a contact that stays overlapped in the final setup is reported
        let moves = [
            tooth(15, -7.0, [0.0; 6]),
            tooth(14, -0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ];
        let mut moves = moves;
        moves[0].frame = DMat4::from_translation(DVec3::new(-6.0, 0.0, 0.0));
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        let plan = run(&input, &p).unwrap();
        assert!(plan.final_overlap[0].mm > 0.9);
        assert!(plan.log.iter().any(|l| l.contains("Residual overlap")));
    }

    #[test]
    fn ipr_without_the_collision_check_is_done_before_aligner_one() {
        let moves = [tooth(15, -7.0, [0.0; 6]), tooth(14, 0.0, [0.0; 6])];
        let ipr = [IprPlan {
            a: fdi(15),
            b: fdi(14),
            mm: 0.3,
        }];
        let input = Input {
            moves: &moves,
            ipr: &ipr,
            complexity: Complexity::default(),
        };
        let p = Params {
            collision: false,
            ..Params::default()
        };
        let plan = run(&input, &p).unwrap();
        assert_eq!(
            plan.ipr_stage,
            vec![IprStage {
                a: fdi(15),
                b: fdi(14),
                stage: 1
            }]
        );
        assert_eq!(plan.n_stages, 0);
    }

    #[test]
    fn auto_pattern_follows_the_complexity() {
        let mut c = Complexity::default();
        assert_eq!(choose_pattern(&c), Pattern::Proportional);
        c.crowding_mm = 3.5;
        assert_eq!(choose_pattern(&c), Pattern::Structured);
        c.distalization_mm = 0.6;
        assert_eq!(choose_pattern(&c), Pattern::SequentialDistal);
        assert_eq!(Level::from_score(20.0), Level::Easy);
        assert_eq!(Level::from_score(21.0), Level::Moderate);
        assert_eq!(Level::from_score(61.0), Level::VeryComplex);
    }

    #[test]
    fn overcorrection_enlarges_the_final_movement() {
        let moves = [tooth(11, 0.0, [0.0, 0.0, 0.5, 0.0, 0.0, 0.1])];
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        let p = Params {
            overcorrection: true,
            ..Params::default()
        };
        let plan = run(&input, &p).unwrap();
        let q = plan.q_final[&fdi(11)];
        assert!((q[2] - 0.5 * 1.117).abs() < 1e-12);
        assert!((q[5] - 0.1 * 1.284).abs() < 1e-12);
    }

    #[test]
    fn cancelling_stops_the_planning() {
        let moves = [
            tooth(15, -7.6, [4.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            tooth(14, 0.0, [0.0; 6]),
        ];
        let input = Input {
            moves: &moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        let r = run_with_progress(&input, &Params::default(), &mut |_| false);
        assert_eq!(r, Err(StagingError::Cancelled));
    }

    #[test]
    fn adjacent_pairs_cross_the_midline_only_between_the_centrals() {
        let seq: Vec<Fdi> = [12, 11, 21, 22].into_iter().map(fdi).collect();
        assert_eq!(
            adjacent_pairs(&seq),
            vec![(fdi(12), fdi(11)), (fdi(11), fdi(21)), (fdi(21), fdi(22))]
        );
        let gap: Vec<Fdi> = [12, 22].into_iter().map(fdi).collect();
        assert!(adjacent_pairs(&gap).is_empty());
    }

    #[test]
    fn describe_q_follows_the_clinical_signs() {
        let q = [
            0.3,
            0.2,
            -0.1,
            1.0_f64.to_radians(),
            2.0_f64.to_radians(),
            3.0_f64.to_radians(),
        ];
        let d = describe_q(&q, -1);
        assert!((d[0] + 0.3).abs() < 1e-12 && (d[1] - 0.2).abs() < 1e-12);
        assert!(
            (d[3] + 1.0).abs() < 1e-9 && (d[4] + 2.0).abs() < 1e-9 && (d[5] + 3.0).abs() < 1e-9
        );
    }

    fn plan_of(moves: &[ToothMove], p: &Params) -> Result<Plan, StagingError> {
        let input = Input {
            moves,
            ipr: &[],
            complexity: Complexity::default(),
        };
        run(&input, p)
    }

    #[test]
    fn limits_that_are_not_positive_are_refused_and_a_cap_of_zero_is_not_no_cap() {
        let moves = [tooth(11, 0.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.1])];
        type Set = fn(&mut Params, f64);
        let fields: [(&str, Set); 6] = [
            ("step_linear_mm", |p, v| p.step_linear_mm = v),
            ("step_vertical_mm", |p, v| p.step_vertical_mm = v),
            ("step_rotation_deg", |p, v| p.step_rotation_deg = v),
            ("step_rotation_round_deg", |p, v| {
                p.step_rotation_round_deg = v
            }),
            ("step_angular_deg", |p, v| p.step_angular_deg = v),
            ("max_crown_step_mm", |p, v| p.max_crown_step_mm = v),
        ];
        for (field, set) in fields {
            for v in [0.0, -0.25, f64::NAN, f64::INFINITY] {
                let mut p = Params::default();
                set(&mut p, v);
                match plan_of(&moves, &p) {
                    Err(StagingError::InvalidLimit { name, .. }) => assert_eq!(name, field),
                    other => panic!("{field} = {v}: {other:?}"),
                }
            }
        }
        let p = Params {
            collision_tol_mm: -0.1,
            ..Params::default()
        };
        assert!(matches!(
            plan_of(&moves, &p),
            Err(StagingError::InvalidLimit {
                name: "collision_tol_mm",
                ..
            })
        ));
        // the count itself refuses a cap or a limit of 0 instead of ignoring it
        let (lim, cap) = limits(fdi(11), &Params::default());
        let q = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(stages_needed(&q, &[], &lim, 0.0, None), None);
        let mut zero = lim;
        zero[0] = 0.0;
        assert_eq!(stages_needed(&q, &[], &zero, cap, None), None);
        // up to MAX_ALIGNERS a count, above it none (`as i64` saturated to i64::MAX)
        let most = [249.75, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            stages_needed(&most, &[], &lim, 1e9, None),
            Some(MAX_ALIGNERS)
        );
        let more = [250.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(stages_needed(&more, &[], &lim, 1e9, None), None);
        let huge = [tooth(11, 0.0, [1e6, 0.0, 0.0, 0.0, 0.0, 0.0])];
        assert_eq!(
            plan_of(&huge, &Params::default()).unwrap_err(),
            StagingError::TooManyAligners {
                fdi: fdi(11),
                max: MAX_ALIGNERS
            }
        );
    }

    #[test]
    fn a_speed_outside_its_range_is_refused() {
        for speed in [-0.5, 1e-300, 1e-6, 0.0, 0.19, 1.01, f64::NAN, f64::INFINITY] {
            let mut m = tooth(11, 0.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            m.speed = speed;
            let r = plan_of(&[m], &Params::default());
            assert!(
                matches!(r, Err(StagingError::InvalidSpeed { value, .. }) if value.to_bits() == speed.to_bits()),
                "{speed}: {r:?}"
            );
        }
        // the ends of the range plan: 1 mm at 0.25 mm times the speed per aligner
        for (speed, n) in [(0.2, 20), (1.0, 4)] {
            let mut m = tooth(11, 0.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            m.speed = speed;
            assert_eq!(plan_of(&[m], &Params::default()).unwrap().n_stages, n);
        }
    }

    #[test]
    fn movements_that_are_not_finite_are_refused() {
        let base = || tooth(11, 0.0, [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let mut a = base();
        a.q[3] = f64::NAN;
        let mut b = base();
        b.q0 = Some([0.0, f64::INFINITY, 0.0, 0.0, 0.0, 0.0]);
        let mut c = base();
        c.frame.w_axis.x = f64::NAN;
        let mut d = base();
        d.pts[7].z = f64::NAN;
        for (m, what) in [
            (a, "final movement"),
            (b, "starting movement"),
            (c, "rest frame"),
            (d, "crown points"),
        ] {
            assert_eq!(
                plan_of(&[m], &Params::default()).unwrap_err(),
                StagingError::NotFinite { fdi: fdi(11), what }
            );
        }
        // a NaN quaternion of a stored pose is refused, not planned as NaN poses
        let nan = Pose {
            t: DVec3::ZERO,
            q: DQuat::from_xyzw(f64::NAN, 0.0, 0.0, 1.0),
        };
        assert_eq!(
            plan_of(&[tooth(11, 0.0, q_from_pose(&nan))], &Params::default()).unwrap_err(),
            StagingError::NotFinite {
                fdi: fdi(11),
                what: "final movement"
            }
        );
        // the zero quaternion is no rotation
        let zero = Pose {
            t: DVec3::new(0.5, 0.0, 0.0),
            q: DQuat::from_xyzw(0.0, 0.0, 0.0, 0.0),
        };
        assert_eq!(q_from_pose(&zero), [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn ipr_that_no_aligner_carries_is_reported() {
        // no movement: no aligner, and the IPR keeps the plugin's stage (0, or 1)
        let moves = [tooth(15, -7.0, [0.0; 6]), tooth(14, 0.0, [0.0; 6])];
        let ipr = [IprPlan {
            a: fdi(15),
            b: fdi(14),
            mm: 0.3,
        }];
        let input = Input {
            moves: &moves,
            ipr: &ipr,
            complexity: Complexity::default(),
        };
        for (collision, stage) in [(false, 1), (true, 0)] {
            let p = Params {
                collision,
                ..Params::default()
            };
            let plan = run(&input, &p).unwrap();
            assert_eq!(plan.n_stages, 0);
            assert_eq!(plan.ipr_stage[0].stage, stage);
            assert!(
                plan.log
                    .iter()
                    .any(|l| l.starts_with("IPR 15|14 is in no aligner")),
                "{:?}",
                plan.log
            );
        }
    }

    /// A deterministic xorshift generator: a value in `[-1, 1)`.
    struct Rng(u64);

    impl Rng {
        fn uniform(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
        }
    }

    #[test]
    fn random_plans_keep_every_step_within_its_limit() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let patterns = [
            Pattern::Proportional,
            Pattern::MaxVelocity,
            Pattern::SequentialDistal,
            Pattern::Structured,
        ];
        let mut plans = 0;
        for case in 0..12 {
            // a premolar, a premolar and a canine 8 mm apart: up to 1.5 mm and 12 degrees
            let mut moves: Vec<ToothMove> = [15_u8, 14, 13]
                .iter()
                .enumerate()
                .map(|(i, &n)| {
                    let q: Q6 = std::array::from_fn(|c| {
                        if c < 3 {
                            1.5 * rng.uniform()
                        } else {
                            12f64.to_radians() * rng.uniform()
                        }
                    });
                    tooth(n, 8.0 * i as f64, q)
                })
                .collect();
            // a slowed tooth, and every fourth case re-planned from a delivered aligner
            moves[case % 3].speed = 0.5 + 0.5 * rng.uniform().abs();
            if case % 4 == 1 {
                moves[0].q0 = Some(moves[0].q.map(|v| 0.3 * v));
            }
            for pattern in patterns {
                let p = Params {
                    pattern,
                    collision: case % 2 == 0,
                    ..Params::default()
                };
                let plan = plan_of(&moves, &p).unwrap();
                assert!(
                    plan.over_limit.is_empty(),
                    "{case} {pattern:?}: {:?}",
                    plan.over_limit
                );
                // checked again here, on every crown point, apart from the stats
                for m in &moves {
                    let (lim, cap) = tooth_limits(m, &p);
                    let qs = &plan.q[&m.fdi];
                    for w in qs.windows(2) {
                        for c in 0..6 {
                            let step = (w[1][c] - w[0][c]).abs();
                            assert!(
                                step <= lim[c] * (1.0 + 1e-9) + 1e-12,
                                "{case} {pattern:?} {c}"
                            );
                        }
                        let (a, b) = (local_pose(&w[0]), local_pose(&w[1]));
                        let crown = m
                            .pts
                            .iter()
                            .map(|&x| a.transform_point3(x).distance(b.transform_point3(x)))
                            .fold(0.0, f64::max);
                        assert!(crown <= cap + 1e-9, "{case} {pattern:?}: {crown} > {cap}");
                    }
                    // the last aligner reaches the target
                    let last = qs.last().unwrap();
                    assert!(
                        (0..6).all(|c| (last[c] - m.q[c]).abs() < 1e-12),
                        "{case} {pattern:?}"
                    );
                }
                plans += 1;
            }
        }
        assert_eq!(plans, 48);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// A component from {-2, -0.6, -0.25, 0, 1e-10, 0.25, 0.6, 2}: no movement, one below
    /// the threshold, exact multiples of the limits and values between them.
    fn any_component() -> f64 {
        match kani::any::<u8>() % 8 {
            0 => -2.0,
            1 => -0.6,
            2 => -0.25,
            3 => 0.0,
            4 => 1e-10,
            5 => 0.25,
            6 => 0.6,
            _ => 2.0,
        }
    }

    /// A limit from {0.05, 0.25, 0.5}.
    fn any_limit() -> f64 {
        match kani::any::<u8>() % 3 {
            0 => 0.05,
            1 => 0.25,
            _ => 0.5,
        }
    }

    // Dividing a movement into the aligners `stages_needed` counts keeps every component within
    // its limit per aligner, with the tolerance of the post-condition of the planning:
    // |q_i| / n <= lim_i (1 + 1e-9) + 1e-12. The count is never above MAX_ALIGNERS, and it is
    // 0 only when the movement is below 1e-9 in every component. Positive limits and cap, finite
    // movement from the sets above, with or without a mask; no crown points (the crown cap only
    // adds aligners, and the square roots of arbitrary points keep the solver busy).
    #[kani::proof]
    fn stages_needed_keeps_every_component_within_its_limit() {
        let q: Q6 = std::array::from_fn(|_| any_component());
        let lim: Q6 = std::array::from_fn(|_| any_limit());
        let cap = any_limit();
        let mask: [bool; 6] = kani::any();
        let masked: bool = kani::any();
        let mask = if masked { Some(&mask) } else { None };
        let n = stages_needed(&q, &[], &lim, cap, mask);
        assert!(n.is_some());
        let n = n.unwrap_or(0);
        assert!((0..=MAX_ALIGNERS).contains(&n));
        for i in 0..6 {
            let qi = match mask {
                Some(m) if !m[i] => 0.0,
                _ => q[i],
            };
            if n == 0 {
                assert!(qi.abs() < 1e-9);
            } else {
                assert!(qi.abs() / n as f64 <= lim[i] * (1.0 + 1e-9) + 1e-12);
            }
        }
    }
}
