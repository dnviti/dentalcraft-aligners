//! Where the aligner ends and the production settings of the case.
//!
//! The plugin keeps the end of the aligner of each arch on its case object
//! (`aligner_end_right` / `_left`, `props.py:342-347`; applied by
//! `oa.aligner_end` and `oa.aligner_end_apply`, `ops_misc.py:96,132`;
//! `case_print.aligner_teeth`, `aligner_suggestions`, `setup_end_changed`).
//! Here it is [`AlignersState::aligner_end`]; the final position records the
//! end it was computed with ([`SetupSummary::aligner_end`]).

use std::collections::BTreeSet;

use dc_case::{Fdi, ScanRole};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::in_arch;
use crate::{AlignersState, RampSettings};

/// A side of the patient.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The patient's right (quadrants 1 and 4).
    Right,
    /// The patient's left (quadrants 2 and 3).
    Left,
}

impl Side {
    /// The side of tooth `f` (the plugin: `f // 10 in (1, 4)` is the right).
    pub fn of(f: Fdi) -> Side {
        match f.quadrant() {
            1 | 4 | 5 | 8 => Side::Right,
            _ => Side::Left,
        }
    }

    /// The permanent quadrant of this side in the arch `role` (`None` for a
    /// bite role).
    pub fn quadrant(self, role: ScanRole) -> Option<u8> {
        match (role, self) {
            (ScanRole::Upper, Side::Right) => Some(1),
            (ScanRole::Upper, Side::Left) => Some(2),
            (ScanRole::Lower, Side::Left) => Some(3),
            (ScanRole::Lower, Side::Right) => Some(4),
            (ScanRole::Bite, _) => None,
        }
    }
}

/// Where the aligner of an arch ends on each side of the patient: the last
/// tooth it covers; `None`, every tooth of that side. The teeth behind it
/// stay out of the aligner: not covered, and still in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignerEnd {
    /// Last tooth covered on the patient's right.
    pub right: Option<Fdi>,
    /// Last tooth covered on the patient's left.
    pub left: Option<Fdi>,
}

impl AlignerEnd {
    /// The last tooth covered on `side`.
    pub fn get(&self, side: Side) -> Option<Fdi> {
        match side {
            Side::Right => self.right,
            Side::Left => self.left,
        }
    }

    /// Sets the last tooth covered on `side` (`None`: every tooth).
    pub fn set(&mut self, side: Side, last: Option<Fdi>) {
        match side {
            Side::Right => self.right = last,
            Side::Left => self.left = last,
        }
    }

    /// The aligner covers tooth `f`: no end on its side, or `f` is not
    /// behind it (`case_print.aligner_teeth`).
    pub fn covers(&self, f: Fdi) -> bool {
        self.get(Side::of(f))
            .is_none_or(|last| f.position() <= last.position())
    }

    /// The aligner covers every tooth.
    pub fn is_every_tooth(&self) -> bool {
        self.right.is_none() && self.left.is_none()
    }
}

/// A suggestion of where the aligner had better end on a side
/// (`case_print.aligner_suggestions`): tooth `missing` (a premolar or a
/// first or second molar) is missing and teeth behind it are covered; the
/// shell behind the space is weak, so it ends at `last`, the tooth before
/// the space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignerEndSuggestion {
    /// The side.
    pub side: Side,
    /// The last tooth to cover.
    pub last: Fdi,
    /// The missing tooth.
    pub missing: Fdi,
}

impl AlignersState {
    /// The teeth of the arch `role`: its built teeth (closed crowns), else
    /// the teeth of its seeds not marked missing (the plugin's `oa.teeth`).
    pub fn arch_teeth(&self, role: ScanRole) -> BTreeSet<Fdi> {
        let built: BTreeSet<Fdi> = self
            .crowns
            .keys()
            .copied()
            .filter(|f| in_arch(*f, role))
            .collect();
        if !built.is_empty() {
            return built;
        }
        self.seeds
            .iter()
            .map(|s| s.fdi)
            .filter(|f| in_arch(*f, role) && !self.missing.contains(f))
            .collect()
    }

    /// Where the aligner of the arch `role` ends.
    pub fn aligner_end_of(&self, role: ScanRole) -> AlignerEnd {
        self.aligner_end.get(&role).copied().unwrap_or_default()
    }

    /// The teeth the aligner of the arch `role` covers
    /// (`case_print.aligner_teeth`): those of [`AlignersState::arch_teeth`]
    /// not behind its end. The shell, the trim line, the stamp and the bite
    /// with the aligners worn are made on these only.
    pub fn aligner_teeth(&self, role: ScanRole) -> BTreeSet<Fdi> {
        let end = self.aligner_end_of(role);
        self.arch_teeth(role)
            .into_iter()
            .filter(|f| end.covers(*f))
            .collect()
    }

    /// The teeth of the arch `role` left out of the aligner
    /// (`case_plan.out_of_aligner_teeth`): the final position keeps them
    /// where they are.
    pub fn out_of_aligner(&self, role: ScanRole) -> Vec<Fdi> {
        let end = self.aligner_end_of(role);
        self.arch_teeth(role)
            .into_iter()
            .filter(|f| !end.covers(*f))
            .collect()
    }

    /// The final position of the arch `role` was computed with another end
    /// of the aligner than the one stored (`case_print.setup_end_changed`):
    /// the final position and the aligners are to compute again. `false`
    /// when the arch has no final position.
    pub fn setup_end_changed(&self, role: ScanRole) -> bool {
        self.setup
            .results
            .get(&role)
            .is_some_and(|r| r.aligner_end != self.aligner_end_of(role))
    }

    /// Where the aligner of the arch `role` had better end, side by side
    /// (`case_print.aligner_suggestions`, its rule of the missing teeth):
    /// behind a missing premolar or first or second molar with covered
    /// teeth behind it, the teeth after the space are left out. None when
    /// nothing is to change. (The plugin's other rule, the back teeth of a
    /// shell built with a defect, needs the defects of the shells, which
    /// the port does not record.)
    pub fn aligner_end_suggestions(&self, role: ScanRole) -> Vec<AlignerEndSuggestion> {
        let present = self.arch_teeth(role);
        let covered = self.aligner_teeth(role);
        let mut out = Vec::new();
        for side in [Side::Right, Side::Left] {
            let Some(q) = side.quadrant(role) else {
                continue;
            };
            let side_teeth: Vec<Fdi> = present
                .iter()
                .copied()
                .filter(|f| f.quadrant() == q)
                .collect();
            for i in 4..8u8 {
                let Ok(f) = Fdi::new(q * 10 + i) else {
                    continue;
                };
                let behind = side_teeth
                    .iter()
                    .any(|g| g.position() > i && covered.contains(g));
                if present.contains(&f) || !behind {
                    continue;
                }
                if let Some(last) = side_teeth
                    .iter()
                    .copied()
                    .filter(|g| g.position() < i)
                    .max_by_key(|g| g.position())
                {
                    out.push(AlignerEndSuggestion {
                        side,
                        last,
                        missing: f,
                    });
                }
                break;
            }
        }
        out
    }

    /// The production settings by group ([`crate::PRODUCTION_GROUPS`]), as
    /// `aligners.settings.production` takes and returns them.
    pub fn production_values(&self) -> Value {
        let mut ramps = serde_json::to_value(&self.aligner_bite.ramps).unwrap_or_default();
        if let Some(m) = ramps.as_object_mut() {
            m.remove("ramps");
        }
        json!({
            "trim": self.trim,
            "shell": {
                "shell_thickness_mm": self.settings.shell_thickness_mm,
                "clearance_mm": self.settings.clearance_mm,
                "gum_clearance_mm": self.settings.gum_clearance_mm,
            },
            "stamp": self.settings.stamp,
            "bite": self.aligner_bite.occlusal,
            "ramps": ramps,
            "quality": self.settings.quality,
        })
    }

    /// Stores the values of the production group `group` (all its fields,
    /// as [`AlignersState::production_values`] gives them). The ranges are
    /// not checked here ([`crate::production_out_of_range`]).
    pub fn set_production_group(
        &mut self,
        group: &str,
        values: Value,
    ) -> Result<(), serde_json::Error> {
        use serde::de::Error;
        match group {
            "trim" => self.trim = serde_json::from_value(values)?,
            "shell" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Shell {
                    shell_thickness_mm: f64,
                    clearance_mm: f64,
                    gum_clearance_mm: f64,
                }
                let s: Shell = serde_json::from_value(values)?;
                self.settings.shell_thickness_mm = s.shell_thickness_mm;
                self.settings.clearance_mm = s.clearance_mm;
                self.settings.gum_clearance_mm = s.gum_clearance_mm;
            }
            "stamp" => self.settings.stamp = serde_json::from_value(values)?,
            "bite" => self.aligner_bite.occlusal = serde_json::from_value(values)?,
            "ramps" => {
                let ramps = std::mem::take(&mut self.aligner_bite.ramps.ramps);
                let mut r: RampSettings = serde_json::from_value(values)?;
                r.ramps = ramps;
                self.aligner_bite.ramps = r;
            }
            "quality" => self.settings.quality = serde_json::from_value(values)?,
            other => {
                return Err(serde_json::Error::custom(format!(
                    "unknown production group {other}"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PRODUCTION_GROUPS, PRODUCTION_NUMBERS, Seed, production_out_of_range};
    use glam::DVec3;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    fn with_seeds(teeth: &[u8]) -> AlignersState {
        AlignersState {
            seeds: teeth
                .iter()
                .map(|&n| Seed {
                    fdi: fdi(n),
                    position: DVec3::ZERO,
                })
                .collect(),
            ..AlignersState::default()
        }
    }

    #[test]
    fn the_end_leaves_out_the_teeth_behind_it_on_its_side() {
        let mut s = with_seeds(&[31, 32, 33, 34, 35, 36, 37, 41, 42, 46, 47, 11]);
        let lower = ScanRole::Lower;
        assert_eq!(s.aligner_teeth(lower).len(), 11);
        assert!(s.out_of_aligner(lower).is_empty());
        s.aligner_end.insert(
            lower,
            AlignerEnd {
                right: Some(fdi(46)),
                left: Some(fdi(35)),
            },
        );
        assert_eq!(s.out_of_aligner(lower), [fdi(36), fdi(37), fdi(47)]);
        assert!(s.aligner_teeth(lower).contains(&fdi(46)));
        // the upper arch is not touched
        assert_eq!(s.aligner_teeth(ScanRole::Upper), BTreeSet::from([fdi(11)]));
        assert_eq!(Side::of(fdi(14)), Side::Right);
        assert_eq!(Side::of(fdi(36)), Side::Left);
        assert_eq!(Side::Left.quadrant(lower), Some(3));
    }

    #[test]
    fn a_missing_premolar_or_molar_suggests_ending_before_the_space() {
        // 35 missing with 36 and 37 behind it on the left; the right is whole
        let mut s = with_seeds(&[31, 32, 33, 34, 36, 37, 41, 42, 43, 44, 45, 46, 47]);
        let lower = ScanRole::Lower;
        assert_eq!(
            s.aligner_end_suggestions(lower),
            [AlignerEndSuggestion {
                side: Side::Left,
                last: fdi(34),
                missing: fdi(35),
            }]
        );
        // once it ends there, nothing more to suggest
        s.aligner_end.insert(
            lower,
            AlignerEnd {
                left: Some(fdi(34)),
                right: None,
            },
        );
        assert!(s.aligner_end_suggestions(lower).is_empty());
        // a missing last molar (nothing behind it) is no reason
        let s = with_seeds(&[41, 42, 43, 44, 45, 46]);
        assert!(s.aligner_end_suggestions(lower).is_empty());
        // a tooth marked missing is not a tooth of the arch
        let mut s = with_seeds(&[11, 12, 13, 14, 15, 16, 17]);
        s.missing.insert(fdi(16));
        assert_eq!(s.aligner_end_suggestions(ScanRole::Upper)[0].last, fdi(15));
    }

    #[test]
    fn the_setup_end_differs_only_when_there_is_a_setup() {
        let mut s = with_seeds(&[41, 42, 46, 47]);
        let lower = ScanRole::Lower;
        s.aligner_end.insert(
            lower,
            AlignerEnd {
                right: Some(fdi(46)),
                left: None,
            },
        );
        assert!(!s.setup_end_changed(lower));
        s.setup.results.insert(lower, Default::default());
        assert!(s.setup_end_changed(lower));
        s.setup.results.get_mut(&lower).unwrap().aligner_end = s.aligner_end_of(lower);
        assert!(!s.setup_end_changed(lower));
    }

    #[test]
    fn every_production_group_goes_out_and_back() {
        let mut s = AlignersState::default();
        s.aligner_bite.ramps.ramps.push(crate::Ramp {
            fdi: fdi(11),
            pose: dc_case::Pose::IDENTITY,
            size_mm: [3.0, 2.0, 1.5],
            kind: crate::RampKind::Manual,
        });
        let values = s.production_values();
        for g in PRODUCTION_GROUPS {
            assert!(production_out_of_range(g, &values[g]).is_none(), "{g}");
            let before = s.clone();
            s.set_production_group(g, values[g].clone()).unwrap();
            assert_eq!(s, before, "{g}");
        }
        for (g, name, lo, hi) in PRODUCTION_NUMBERS {
            assert!(lo < hi, "{name}");
            let v = values[g][name]
                .as_f64()
                .unwrap_or_else(|| panic!("{g}.{name}"));
            assert!((lo..=hi).contains(&v), "{g}.{name} default {v}");
            let mut bad = values[g].clone();
            bad[name] = json!(hi + 0.5);
            assert_eq!(production_out_of_range(g, &bad).map(|o| o.0), Some(name));
        }
        // the ramps placed stay when their sizes change
        let mut ramps = values["ramps"].clone();
        ramps["width_mm"] = json!(4.0);
        s.set_production_group("ramps", ramps).unwrap();
        assert_eq!(s.aligner_bite.ramps.width_mm, 4.0);
        assert_eq!(s.aligner_bite.ramps.ramps.len(), 1);
        assert!(s.set_production_group("nothing", json!({})).is_err());
        assert!(
            s.set_production_group("shell", json!({ "shell_thickness_mm": 1.0 }))
                .is_err()
        );
    }
}
