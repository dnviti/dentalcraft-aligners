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

/// Style of the orthodontic study-model bases (B6,
/// `docs/plan/B6-STUDY-MODEL-TRIMMING.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyBaseStyle {
    /// European: walls at 90° to the base, 10 mm base under the anatomy.
    #[default]
    European,
    /// American Board of Orthodontics: 70 mm pair, 65°/115° heptagon.
    Abo,
}

/// Main parameters of the study-model bases; the command
/// `aligners.output.study-models` starts from them (its other parameters
/// keep the defaults of the specification).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StudyBaseSettings {
    /// Style of the base.
    pub style: StudyBaseStyle,
    /// ABO: height of the articulated pair, base to base (mm).
    pub total_height_mm: f64,
    /// ABO: fraction of each model taken by the art portion.
    pub art_fraction: f64,
    /// European: thickness of the base under the anatomy (mm).
    pub base_thickness_mm: f64,
    /// Clearance of the base plane under the vault and the deepest sulcus (mm).
    pub m_base_mm: f64,
    /// Distance of the back plane behind the most distal anatomy (mm).
    pub back_margin_mm: f64,
    /// ABO: angle of the side cuts to the back line (degrees).
    pub side_angle_deg: f64,
    /// Interior angle of the heels with the back plane (degrees).
    pub heel_angle_deg: f64,
    /// ABO: side cuts outside the deepest sulcus (mm).
    pub side_clear_mm: f64,
    /// European: walls outside the vestibule (mm).
    pub vestibule_clear_mm: f64,
}

impl Default for StudyBaseSettings {
    fn default() -> Self {
        Self {
            style: StudyBaseStyle::European,
            total_height_mm: 70.0,
            art_fraction: 1.0 / 3.0,
            base_thickness_mm: 10.0,
            m_base_mm: 3.0,
            back_margin_mm: 2.0,
            side_angle_deg: 65.0,
            heel_angle_deg: 115.0,
            side_clear_mm: 2.5,
            vestibule_clear_mm: 3.0,
        }
    }
}

/// Settings of "Find the teeth" (the plugin's `include_third_molars` and
/// `missing_teeth`): they feed the parameters of `aligners.teeth.find` when
/// its leaf is run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FindTeethSettings {
    /// Look for the third molars too (default false).
    pub third_molars: bool,
    /// FDI numbers of the teeth known to be missing, sorted, without
    /// repetitions (default none).
    pub missing: Vec<u8>,
}

impl FindTeethSettings {
    /// The parameters of `aligners.teeth.find` when its menu leaf is run:
    /// these settings (the module and the workspace both run the leaf).
    pub fn leaf_params(&self) -> serde_json::Value {
        serde_json::json!({ "third_molars": self.third_molars, "missing": self.missing })
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
    /// Study-model bases.
    pub study_base: StudyBaseSettings,
    /// "Find the teeth": third molars and teeth known to be missing.
    pub find_teeth: FindTeethSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            staging: StagingSettings::default(),
            interval_days: 14,
            wear_hours: 22.0,
            shell_thickness_mm: 1.0,
            clearance_mm: 0.0,
            study_base: StudyBaseSettings::default(),
            find_teeth: FindTeethSettings::default(),
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
        assert_eq!(s.find_teeth, FindTeethSettings::default());
        assert!(!s.find_teeth.third_molars && s.find_teeth.missing.is_empty());
    }

    #[test]
    fn settings_saved_without_find_teeth_still_load() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v.as_object_mut().unwrap().remove("find_teeth");
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.find_teeth, FindTeethSettings::default());
        let s: Settings =
            serde_json::from_str(r#"{"find_teeth":{"third_molars":true,"missing":[35,45]}}"#)
                .unwrap();
        assert!(s.find_teeth.third_molars);
        assert_eq!(s.find_teeth.missing, [35, 45]);
        assert_eq!(
            s.find_teeth.leaf_params(),
            serde_json::json!({ "third_molars": true, "missing": [35, 45] })
        );
    }

    #[test]
    fn partial_json_fills_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{"interval_days": 7}"#).unwrap();
        assert_eq!(s.interval_days, 7);
        assert_eq!(s.staging, StagingSettings::default());
        assert_eq!(s.study_base, StudyBaseSettings::default());
    }

    #[test]
    fn study_base_defaults_follow_the_specification() {
        let s: StudyBaseSettings = serde_json::from_str(r#"{"style": "abo"}"#).unwrap();
        assert_eq!(s.style, StudyBaseStyle::Abo);
        assert_eq!(s.total_height_mm, 70.0);
        assert_eq!(s.side_angle_deg, 65.0);
        assert_eq!(s.heel_angle_deg, 115.0);
        assert_eq!(StudyBaseSettings::default().style, StudyBaseStyle::European);
    }
}
