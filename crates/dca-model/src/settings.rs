//! User-tunable parameters. Defaults come from the old plugin
//! (`staging.py` `StagingParams`, `evidence.py`, `props.py`).

use serde::{Deserialize, Serialize};

/// How the movement is distributed over the aligners (old `staging.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagingPattern {
    /// Chosen from the complexity of the case.
    #[default]
    Auto,
    /// All teeth move together and finish together (lowest force).
    Proportional,
    /// All teeth start together at maximum velocity ("X" pattern).
    MaxVelocity,
    /// Sequential distalization 1/2 ("V" pattern).
    SequentialDistal,
    /// Horizontal movements first, vertical movements afterwards.
    Structured,
}

/// Periodontal condition; reduces the movement per aligner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerioStage {
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

impl PerioStage {
    /// Largest linear step per aligner in mm (Ma & Li 2021).
    pub fn linear_step_mm(self) -> f64 {
        match self {
            PerioStage::Healthy => 0.25,
            PerioStage::Stage1 => 0.18,
            PerioStage::Stage2 => 0.15,
            PerioStage::Stage3 => 0.10,
        }
    }
}

/// Limits and options of the staging.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StagingSettings {
    /// Largest translation per aligner (mm).
    pub step_linear_mm: f64,
    /// Largest intrusion or extrusion per aligner (mm).
    pub step_vertical_mm: f64,
    /// Largest rotation per aligner (deg).
    pub step_rotation_deg: f64,
    /// Largest rotation per aligner for round crowns, canines and
    /// premolars (deg).
    pub step_rotation_round_deg: f64,
    /// Largest torque and tip per aligner (deg).
    pub step_angular_deg: f64,
    /// Largest move of any crown point per aligner (mm).
    pub max_crown_step_mm: f64,
    /// Periodontal condition.
    pub perio: PerioStage,
    /// Distribution pattern.
    pub pattern: StagingPattern,
    /// Delay the movement of teeth that would collide with a neighbour.
    pub collision_check: bool,
    /// Interpenetration of adjacent crowns tolerated at a stage (mm).
    pub collision_tol_mm: f64,
}

impl Default for StagingSettings {
    fn default() -> Self {
        Self {
            step_linear_mm: 0.25,
            step_vertical_mm: 0.25,
            step_rotation_deg: 2.0,
            step_rotation_round_deg: 1.5,
            step_angular_deg: 1.0,
            max_crown_step_mm: 0.33,
            perio: PerioStage::Healthy,
            pattern: StagingPattern::Auto,
            collision_check: true,
            collision_tol_mm: 0.10,
        }
    }
}

/// Shape of the aligner's edge (old `trim_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimMode {
    /// Ends on the equator of each crown.
    Equator,
    /// Straight line beyond the gingival zenith.
    Straight,
    /// Follows the gingival margin with scallops.
    Scalloped,
    /// Smooth line along the gingival margin with soft scallops.
    #[default]
    Smooth,
}

/// Parameters of the trim line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimParams {
    /// Shape of the edge.
    pub mode: TrimMode,
    /// Distance of the edge from the gingival margin, positive on the
    /// crown (mm).
    pub margin_gap_mm: f64,
    /// Recess between adjacent teeth (mm).
    pub recess_mm: f64,
    /// Height of the scallops (mm).
    pub scallop_mm: f64,
    /// Edge beyond the equator (mm).
    pub equator_offset_mm: f64,
    /// Edge extension (mm).
    pub extension_mm: f64,
}

impl Default for TrimParams {
    fn default() -> Self {
        Self {
            mode: TrimMode::Smooth,
            margin_gap_mm: 1.5,
            recess_mm: 1.6,
            scallop_mm: 1.5,
            equator_offset_mm: 0.0,
            extension_mm: 2.0,
        }
    }
}

/// General settings of the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Staging limits and options.
    pub staging: StagingSettings,
    /// Days each aligner is worn.
    pub interval_days: u32,
    /// Required hours of wear per day.
    pub wear_hours: f64,
    /// Thickness of the aligner shell (mm).
    pub shell_thickness_mm: f64,
    /// Internal clearance between tooth and shell (mm).
    pub clearance_mm: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            staging: StagingSettings::default(),
            interval_days: 14,
            wear_hours: 22.0,
            shell_thickness_mm: 1.0,
            clearance_mm: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_old_plugin() {
        let s = Settings::default();
        assert_eq!(s.staging.step_linear_mm, 0.25);
        assert_eq!(s.staging.max_crown_step_mm, 0.33);
        assert_eq!(s.interval_days, 14);
        assert_eq!(PerioStage::Stage3.linear_step_mm(), 0.10);
    }

    #[test]
    fn partial_json_fills_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{"interval_days": 7}"#).unwrap();
        assert_eq!(s.interval_days, 7);
        assert_eq!(s.staging, StagingSettings::default());
    }
}
