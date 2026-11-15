//! Settings and ramps of the bite with the aligners worn.
//!
//! The plugin keeps these as scene settings (`occlusal_mode`, `bite_closing`,
//! `ramps_enabled`, `ramp_width`, ...) and the ramps as Blender objects
//! (`ramps.py`); here they are part of the case. The defaults are the values
//! of `evidence.py` / `props.py` (`dca_algo::evidence` checks them).

use dc_case::{Fdi, Pose};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the occlusal thickness of the aligners is chosen
/// (`occlusal_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OcclusalMode {
    /// Computed from the bite with the aligners worn: the thickness that
    /// brings the opening at the first molars to the target.
    #[default]
    Auto,
    /// The thickness of [`OcclusalSettings::thickness_mm`].
    Manual,
    /// The whole shell of the wall thickness (no occlusal shaping).
    Same,
}

/// How the mouth closes on the aligners (`bite_closing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BiteClosing {
    /// Rotation about the hinge axis of the condyles: the incisors open more
    /// than the molars.
    #[default]
    Hinge,
    /// Translation along the occlusal axis (as the closed mouth of the
    /// stages).
    Vertical,
}

/// Occlusal thickness, closing and equilibration of the aligners.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcclusalSettings {
    /// How the occlusal thickness is chosen.
    pub mode: OcclusalMode,
    /// Occlusal thickness in the manual mode (mm).
    pub thickness_mm: f64,
    /// Smallest occlusal thickness (mm): neither the automatic thickness nor
    /// the grinding go below it.
    pub min_thickness_mm: f64,
    /// Opening of the bite at the first molars caused by the aligners that
    /// the automatic thickness does not exceed (mm).
    pub opening_target_mm: f64,
    /// How the mouth closes.
    pub closing: BiteClosing,
    /// Distance of the hinge axis behind the lower incisal point (mm).
    pub hinge_posterior_mm: f64,
    /// Height of the hinge axis above the lower incisal point (mm).
    pub hinge_height_mm: f64,
    /// Virtual grinding until both sides touch.
    pub equilibrate: bool,
    /// Teeth in contact per side at which the equilibration stops.
    pub equilibrate_teeth_per_side: u32,
    /// Largest grinding of each aligner (mm).
    pub equilibrate_max_mm: f64,
}

impl Default for OcclusalSettings {
    fn default() -> Self {
        Self {
            mode: OcclusalMode::Auto,
            thickness_mm: 0.75,
            min_thickness_mm: 0.5,
            opening_target_mm: 1.5,
            closing: BiteClosing::Hinge,
            hinge_posterior_mm: 78.0,
            hinge_height_mm: 38.0,
            equilibrate: false,
            equilibrate_teeth_per_side: 2,
            equilibrate_max_mm: 0.3,
        }
    }
}

impl OcclusalSettings {
    /// The occlusal limit changes the shell (`case_occlusal.active`).
    pub fn active(&self) -> bool {
        self.mode != OcclusalMode::Same || self.equilibrate
    }
}

/// How a ramp was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RampKind {
    /// Proposed by the deep-bite rule (replaced by the next proposal).
    #[default]
    Auto,
    /// Placed by hand (kept by the proposals).
    Manual,
}

/// A bite ramp: a bump on the outer surface of an aligner the antagonist
/// rests on. It follows its tooth at every stage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ramp {
    /// The tooth it stands on.
    pub fdi: Fdi,
    /// Place and turn of the ramp in the case coordinates of the tooth at
    /// rest (the tooth mesh coordinates): Z along the outward normal of the
    /// aligner where it stands, the centre of its base at the origin.
    pub pose: Pose,
    /// Width (across the tooth), depth and height (mm).
    pub size_mm: [f64; 3],
    /// How it was made.
    pub kind: RampKind,
}

/// Ramps of the case and the sizes of the new ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RampSettings {
    /// The ramps enter the aligners (otherwise they stay in the case unused).
    pub enabled: bool,
    /// Width of a new ramp, along the tooth (mm).
    pub width_mm: f64,
    /// Depth of a new ramp, towards the gingiva (mm).
    pub depth_mm: f64,
    /// Height of a new ramp (mm).
    pub height_mm: f64,
    /// Overbite of the incisors beyond which the anterior ramps are
    /// proposed (mm).
    pub overbite_min_mm: f64,
    /// The ramps of the case.
    pub ramps: Vec<Ramp>,
}

impl Default for RampSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            width_mm: 3.0,
            depth_mm: 2.0,
            height_mm: 1.5,
            overbite_min_mm: 4.0,
            ramps: Vec::new(),
        }
    }
}

/// The bite with the aligners worn: occlusal settings and ramps.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignerBite {
    /// Occlusal thickness, closing and equilibration.
    pub occlusal: OcclusalSettings,
    /// Bite ramps.
    pub ramps: RampSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_json() {
        let b: AlignerBite = serde_json::from_str(r#"{"ramps": {"enabled": true}}"#).unwrap();
        assert!(b.ramps.enabled);
        assert_eq!(b.ramps.width_mm, 3.0);
        assert_eq!(b.occlusal, OcclusalSettings::default());
        assert!(b.occlusal.active());
        assert!(
            !OcclusalSettings {
                mode: OcclusalMode::Same,
                ..Default::default()
            }
            .active()
        );
    }

    #[test]
    fn a_ramp_round_trips() {
        let r = Ramp {
            fdi: Fdi::new(11).unwrap(),
            pose: Pose::IDENTITY,
            size_mm: [3.0, 2.0, 1.5],
            kind: RampKind::Manual,
        };
        let back: Ramp = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
