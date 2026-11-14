//! User-tunable parameters. Defaults come from the old plugin
//! (`staging.py` `StagingParams`, `evidence.py`, `props.py`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the movement is distributed over the aligners (old `staging.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
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

/// Limits and options of the staging (the plugin's scene settings,
/// `props.py:470-498`, read by `case_plan.staging_params`), with the
/// plugin's defaults. The automatic staging uses them for every parameter it
/// is not given; `aligners.settings.staging` changes them inside the ranges
/// of the plugin's sliders ([`StagingSettings::NUMBERS`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    /// Increase the final movement by the overcorrection fractions.
    pub overcorrection: bool,
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
            overcorrection: false,
        }
    }
}

impl StagingSettings {
    /// The numeric settings with the range of the plugin's slider
    /// (`props.py`): field name, smallest and largest value.
    pub const NUMBERS: [(&'static str, f64, f64); 7] = [
        ("step_linear_mm", 0.05, 0.5),
        ("step_vertical_mm", 0.05, 0.5),
        ("step_rotation_deg", 0.25, 4.0),
        ("step_rotation_round_deg", 0.25, 4.0),
        ("step_angular_deg", 0.1, 3.0),
        ("max_crown_step_mm", 0.05, 0.6),
        ("collision_tol_mm", 0.0, 0.5),
    ];

    /// The numeric setting `name` (a name of [`StagingSettings::NUMBERS`]).
    pub fn number(&self, name: &str) -> Option<f64> {
        let mut copy = *self;
        copy.number_mut(name).copied()
    }

    /// Sets the numeric setting `name`; `false` when there is none.
    pub fn set_number(&mut self, name: &str, v: f64) -> bool {
        match self.number_mut(name) {
            Some(slot) => {
                *slot = v;
                true
            }
            None => false,
        }
    }

    /// The first numeric setting outside its range (or not finite): its
    /// name, value and range.
    pub fn out_of_range(&self) -> Option<(&'static str, f64, f64, f64)> {
        Self::NUMBERS.iter().find_map(|&(name, lo, hi)| {
            let v = self.number(name)?;
            (!v.is_finite() || v < lo - 1e-9 || v > hi + 1e-9).then_some((name, v, lo, hi))
        })
    }

    fn number_mut(&mut self, name: &str) -> Option<&mut f64> {
        Some(match name {
            "step_linear_mm" => &mut self.step_linear_mm,
            "step_vertical_mm" => &mut self.step_vertical_mm,
            "step_rotation_deg" => &mut self.step_rotation_deg,
            "step_rotation_round_deg" => &mut self.step_rotation_round_deg,
            "step_angular_deg" => &mut self.step_angular_deg,
            "max_crown_step_mm" => &mut self.max_crown_step_mm,
            "collision_tol_mm" => &mut self.collision_tol_mm,
            _ => return None,
        })
    }
}

/// How the days of wear of each aligner are chosen (the plugin's
/// `interval_policy`, `props.py:728-732`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntervalPolicy {
    /// Every aligner for the same number of days (`interval_days`).
    #[default]
    Fixed,
    /// 7 or 14 days, depending on the movements of the aligner.
    Adaptive,
}

/// Range of the days each aligner is worn (the plugin's `interval_days`
/// slider, `props.py:499`).
pub const INTERVAL_DAYS: (u32, u32) = (5, 21);

/// Relative speed of a slowed tooth (the plugin's `oa.tooth_speed`,
/// `ops_manual.py:978`): smallest and largest value; 1 is the normal speed.
pub const TOOTH_SPEED: (f64, f64) = (0.2, 1.0);

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

/// Parameters of "Separate teeth and gum" (the plugin's `rw_faces`,
/// `rw_eta_convex`, `rw_sigma`): the values
/// `aligners.preparation.segmentation.automatic` uses for a parameter it is
/// not given (the menu leaf, `aligners.auto`, the check of the separated
/// teeth). Their ranges are checked by the segmentation
/// (`dca_algo::segmentation::Params::validate`) when they are changed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SegmentationSettings {
    /// Faces of the simplified scan of the first separation (default 30000).
    pub target_faces: usize,
    /// Weight of the convex edges in the fold measure (default 0.2).
    pub eta_convex: f64,
    /// Sharpness of the fold weights (default 1.0).
    pub sigma: f64,
}

impl Default for SegmentationSettings {
    fn default() -> Self {
        Self {
            target_faces: 30_000,
            eta_convex: 0.2,
            sigma: 1.0,
        }
    }
}

/// Teeth the final position aligns (the plugin's `cons_align_scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupScope {
    /// Incisors and canines (premolars and molars stay as they are).
    #[default]
    Anterior,
    /// Incisors, canines and premolars.
    Premolars,
    /// Every tooth.
    All,
}

/// Where the dental midline goes (the plugin's `midline`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupMidline {
    /// Where it is (the central incisors weigh more).
    #[default]
    Keep,
    /// To the centre of the arch.
    Center,
    /// Balanced space between the two sides.
    Balance,
}

/// Order in which the final position makes space (the plugin's
/// `priority`): expansion, IPR, incisor advancement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpacePriority {
    /// Expansion, then IPR, then advancement.
    ExpIprPro,
    /// IPR, then expansion, then advancement.
    #[default]
    IprExpPro,
    /// Expansion, then advancement, then IPR.
    ExpProIpr,
}

/// Contacts where the final position may plan IPR (the plugin's
/// `ipr_zone`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupIprZone {
    /// Nowhere.
    None,
    /// Between incisors and canines.
    Anterior,
    /// Incisors, canines and the premolars to align.
    #[default]
    AnteriorPremolars,
    /// Every contact.
    All,
}

/// Vertical levelling of the final position (the plugin's `leveling`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupLeveling {
    /// Steps removed, the occlusal curve kept.
    #[default]
    Smooth,
    /// The curve of Spee flattened.
    Flat,
    /// No automatic vertical movement.
    None,
}

/// Parameters of "Final position" (the plugin's scene settings,
/// `props.py:388-468`, read by `case_plan.conservative_config`), with the
/// plugin's defaults, and the steps of the − and + buttons of "Move a
/// tooth" (`adjust_step_mm`, `adjust_step_deg`, `props.py:686-687`). The
/// automatic final position uses them for every parameter it is not given;
/// `aligners.settings.setup` changes them inside the ranges of the plugin's
/// sliders ([`SetupSettings::NUMBERS`]). The field names are those of the
/// parameters of `aligners.planning.setup.automatic`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SetupSettings {
    /// Teeth to align.
    pub align_scope: SetupScope,
    /// Smoothing of the arch line from the cusps (mm).
    pub line_sigma_mm: f64,
    /// Incisors and canines: distance from the line below which they stay (mm).
    pub normal_tol_mm: f64,
    /// Incisors and canines: rotation below which they stay (degrees).
    pub rot_tol_deg: f64,
    /// Premolars and molars: distance from the line below which they stay (mm).
    pub post_normal_tol_mm: f64,
    /// Premolars and molars: rotation below which they stay (degrees).
    pub post_rot_tol_deg: f64,
    /// Move along the arch below which a tooth stays where it is (mm).
    pub pos_tol_mm: f64,
    /// Overlap already present left as it is (mm).
    pub contact_tol_mm: f64,
    /// First and second molars held along the arch (anchorage).
    pub fix_molars: bool,
    /// The teeth held along the arch are still derotated and put on the line.
    pub align_fixed: bool,
    /// Third molars held completely.
    pub fix_third: bool,
    /// Midline.
    pub midline: SetupMidline,
    /// Weight of the central incisors (keeps the midline).
    pub incisor_weight: f64,
    /// Order in which space is made.
    pub priority: SpacePriority,
    /// Largest expansion per side of the upper arch (mm).
    pub expansion_upper_mm: f64,
    /// Largest expansion per side of the lower posterior sectors (mm).
    pub expansion_lower_posterior_mm: f64,
    /// Largest expansion per side of the lower canines (mm).
    pub expansion_lower_canine_mm: f64,
    /// Largest advancement of the incisors (mm).
    pub advance_max_mm: f64,
    /// Largest retraction of the incisors to close spaces (mm).
    pub retract_max_mm: f64,
    /// Close the spaces between the teeth.
    pub close_spaces: bool,
    /// Contacts where IPR is allowed.
    pub ipr_zone: SetupIprZone,
    /// Largest reduction per surface (mm).
    pub ipr_surface_cap_mm: f64,
    /// Largest fraction of the enamel removed.
    pub enamel_fraction: f64,
    /// Vertical levelling.
    pub leveling: SetupLeveling,
    /// Largest vertical correction (mm).
    pub vertical_max_mm: f64,
    /// Vertical step tolerated (mm).
    pub level_tol_mm: f64,
    /// Level the canines too.
    pub level_canines: bool,
    /// Step of the − and + buttons of "Move a tooth" for the translations (mm).
    pub adjust_step_mm: f64,
    /// Step of the − and + buttons of "Move a tooth" for the rotations (degrees).
    pub adjust_step_deg: f64,
}

impl Default for SetupSettings {
    fn default() -> Self {
        Self {
            align_scope: SetupScope::Anterior,
            line_sigma_mm: 5.0,
            normal_tol_mm: 0.1,
            rot_tol_deg: 1.0,
            post_normal_tol_mm: 0.3,
            post_rot_tol_deg: 5.0,
            pos_tol_mm: 0.1,
            contact_tol_mm: 0.15,
            fix_molars: true,
            align_fixed: true,
            fix_third: true,
            midline: SetupMidline::Keep,
            incisor_weight: 4.0,
            priority: SpacePriority::IprExpPro,
            expansion_upper_mm: 2.0,
            expansion_lower_posterior_mm: 2.0,
            expansion_lower_canine_mm: 0.0,
            advance_max_mm: 1.0,
            retract_max_mm: 2.0,
            close_spaces: true,
            ipr_zone: SetupIprZone::AnteriorPremolars,
            ipr_surface_cap_mm: 0.25,
            enamel_fraction: 0.5,
            leveling: SetupLeveling::Smooth,
            vertical_max_mm: 1.0,
            level_tol_mm: 0.5,
            level_canines: false,
            adjust_step_mm: 0.1,
            adjust_step_deg: 1.0,
        }
    }
}

impl SetupSettings {
    /// The numeric settings with the range of the plugin's slider
    /// (`props.py`): field name, smallest and largest value.
    pub const NUMBERS: [(&'static str, f64, f64); 19] = [
        ("line_sigma_mm", 1.0, 20.0),
        ("normal_tol_mm", 0.0, 2.0),
        ("rot_tol_deg", 0.0, 30.0),
        ("post_normal_tol_mm", 0.0, 2.0),
        ("post_rot_tol_deg", 0.0, 30.0),
        ("pos_tol_mm", 0.0, 1.0),
        ("contact_tol_mm", 0.0, 0.5),
        ("incisor_weight", 1.0, 20.0),
        ("expansion_upper_mm", 0.0, 4.0),
        ("expansion_lower_posterior_mm", 0.0, 4.0),
        ("expansion_lower_canine_mm", 0.0, 3.0),
        ("advance_max_mm", 0.0, 4.0),
        ("retract_max_mm", 0.0, 4.0),
        ("ipr_surface_cap_mm", 0.0, 0.5),
        ("enamel_fraction", 0.1, 0.5),
        ("vertical_max_mm", 0.0, 3.0),
        ("level_tol_mm", 0.0, 2.0),
        ("adjust_step_mm", 0.01, 1.0),
        ("adjust_step_deg", 0.1, 10.0),
    ];

    /// The numeric setting `name` (a name of [`SetupSettings::NUMBERS`]).
    pub fn number(&self, name: &str) -> Option<f64> {
        let mut copy = *self;
        copy.number_mut(name).copied()
    }

    /// Sets the numeric setting `name`; `false` when there is none.
    pub fn set_number(&mut self, name: &str, v: f64) -> bool {
        match self.number_mut(name) {
            Some(slot) => {
                *slot = v;
                true
            }
            None => false,
        }
    }

    /// The first numeric setting outside its range (or not finite): its
    /// name, value and range.
    pub fn out_of_range(&self) -> Option<(&'static str, f64, f64, f64)> {
        Self::NUMBERS.iter().find_map(|&(name, lo, hi)| {
            let v = self.number(name)?;
            (!v.is_finite() || v < lo - 1e-9 || v > hi + 1e-9).then_some((name, v, lo, hi))
        })
    }

    fn number_mut(&mut self, name: &str) -> Option<&mut f64> {
        Some(match name {
            "line_sigma_mm" => &mut self.line_sigma_mm,
            "normal_tol_mm" => &mut self.normal_tol_mm,
            "rot_tol_deg" => &mut self.rot_tol_deg,
            "post_normal_tol_mm" => &mut self.post_normal_tol_mm,
            "post_rot_tol_deg" => &mut self.post_rot_tol_deg,
            "pos_tol_mm" => &mut self.pos_tol_mm,
            "contact_tol_mm" => &mut self.contact_tol_mm,
            "incisor_weight" => &mut self.incisor_weight,
            "expansion_upper_mm" => &mut self.expansion_upper_mm,
            "expansion_lower_posterior_mm" => &mut self.expansion_lower_posterior_mm,
            "expansion_lower_canine_mm" => &mut self.expansion_lower_canine_mm,
            "advance_max_mm" => &mut self.advance_max_mm,
            "retract_max_mm" => &mut self.retract_max_mm,
            "ipr_surface_cap_mm" => &mut self.ipr_surface_cap_mm,
            "enamel_fraction" => &mut self.enamel_fraction,
            "vertical_max_mm" => &mut self.vertical_max_mm,
            "level_tol_mm" => &mut self.level_tol_mm,
            "adjust_step_mm" => &mut self.adjust_step_mm,
            "adjust_step_deg" => &mut self.adjust_step_deg,
            _ => return None,
        })
    }
}

/// General settings of the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Staging limits and options.
    pub staging: StagingSettings,
    /// Days each aligner is worn with the fixed calendar
    /// ([`INTERVAL_DAYS`]).
    pub interval_days: u32,
    /// How the days of wear of each aligner are chosen.
    pub interval_policy: IntervalPolicy,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (the plugin's
    /// `start_date`); `None` is the day the calendar is made.
    pub start_date: Option<String>,
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
    /// "Separate teeth and gum": the separation parameters.
    pub segmentation: SegmentationSettings,
    /// "Final position": the setup parameters and the steps of "Move a tooth".
    pub setup: SetupSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            staging: StagingSettings::default(),
            interval_days: 14,
            interval_policy: IntervalPolicy::Fixed,
            start_date: None,
            wear_hours: 22.0,
            shell_thickness_mm: 1.0,
            clearance_mm: 0.0,
            study_base: StudyBaseSettings::default(),
            find_teeth: FindTeethSettings::default(),
            segmentation: SegmentationSettings::default(),
            setup: SetupSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_settings_saved_without_them_load_the_plugin_defaults() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v.as_object_mut().unwrap().remove("setup");
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.setup, SetupSettings::default());
        // the plugin's defaults (`props.py`, `evidence.py`)
        let d = SetupSettings::default();
        assert_eq!(d.align_scope, SetupScope::Anterior);
        assert_eq!(d.priority, SpacePriority::IprExpPro);
        assert_eq!(d.ipr_zone, SetupIprZone::AnteriorPremolars);
        assert_eq!((d.adjust_step_mm, d.adjust_step_deg), (0.1, 1.0));
        assert!(d.out_of_range().is_none());
        // a field saved alone keeps the defaults of the others
        let s: Settings =
            serde_json::from_str(r#"{"setup":{"ipr_zone":"none","advance_max_mm":2.5}}"#).unwrap();
        assert_eq!(s.setup.ipr_zone, SetupIprZone::None);
        assert_eq!(s.setup.advance_max_mm, 2.5);
        assert_eq!(s.setup.retract_max_mm, 2.0);
    }

    #[test]
    fn every_setup_number_has_a_range_and_one_slot() {
        let mut s = SetupSettings::default();
        for (name, lo, hi) in SetupSettings::NUMBERS {
            assert!(lo < hi, "{name}");
            let v = s.number(name).unwrap();
            assert!((lo..=hi).contains(&v), "{name} default {v}");
            assert!(s.set_number(name, hi + 1.0));
            assert_eq!(s.out_of_range().map(|o| o.0), Some(name));
            assert!(s.set_number(name, v));
        }
        assert!(s.out_of_range().is_none());
        assert!(!s.set_number("midline", 1.0));
        assert_eq!(s.number("nothing"), None);
        s.contact_tol_mm = f64::NAN;
        assert_eq!(s.out_of_range().map(|o| o.0), Some("contact_tol_mm"));
    }

    #[test]
    fn every_staging_number_has_a_range_and_one_slot() {
        let mut s = StagingSettings::default();
        for (name, lo, hi) in StagingSettings::NUMBERS {
            assert!(lo < hi, "{name}");
            let v = s.number(name).unwrap();
            assert!((lo..=hi).contains(&v), "{name} default {v}");
            assert!(s.set_number(name, hi + 0.01));
            assert_eq!(s.out_of_range().map(|o| o.0), Some(name));
            assert!(s.set_number(name, lo - 0.01));
            assert_eq!(s.out_of_range().map(|o| o.0), Some(name));
            assert!(s.set_number(name, v));
        }
        assert!(s.out_of_range().is_none());
        assert!(!s.set_number("pattern", 1.0));
        s.step_angular_deg = f64::INFINITY;
        assert_eq!(s.out_of_range().map(|o| o.0), Some("step_angular_deg"));
    }

    #[test]
    fn staging_and_calendar_settings_saved_without_them_load_the_plugin_defaults() {
        // a case saved before the overcorrection, the policy and the start date
        let s: Settings = serde_json::from_str(
            r#"{"staging":{"step_linear_mm":0.2,"pattern":"max_velocity"},"interval_days":10}"#,
        )
        .unwrap();
        assert_eq!(s.staging.step_linear_mm, 0.2);
        assert_eq!(s.staging.pattern, StagingPattern::MaxVelocity);
        assert!(!s.staging.overcorrection);
        assert_eq!(s.staging.max_crown_step_mm, 0.33);
        assert_eq!(s.interval_days, 10);
        assert_eq!(s.interval_policy, IntervalPolicy::Fixed);
        assert_eq!(s.start_date, None);
        // and they go through the file as they are
        let s = Settings {
            interval_policy: IntervalPolicy::Adaptive,
            start_date: Some("2026-01-05".to_owned()),
            staging: StagingSettings {
                overcorrection: true,
                perio: PerioStage::Stage2,
                ..StagingSettings::default()
            },
            ..Settings::default()
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["interval_policy"], "adaptive");
        assert_eq!(v["staging"]["perio"], "stage2");
        assert_eq!(serde_json::from_value::<Settings>(v).unwrap(), s);
        assert!((INTERVAL_DAYS.0..=INTERVAL_DAYS.1).contains(&Settings::default().interval_days));
    }

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
    fn settings_saved_without_segmentation_load_the_defaults() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v.as_object_mut().unwrap().remove("segmentation");
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.segmentation, SegmentationSettings::default());
        assert_eq!(
            (
                s.segmentation.target_faces,
                s.segmentation.eta_convex,
                s.segmentation.sigma
            ),
            (30_000, 0.2, 1.0)
        );
        // a field saved alone keeps the defaults of the others
        let s: Settings = serde_json::from_str(r#"{"segmentation":{"sigma":2.5}}"#).unwrap();
        assert_eq!(s.segmentation.sigma, 2.5);
        assert_eq!(s.segmentation.target_faces, 30_000);
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
