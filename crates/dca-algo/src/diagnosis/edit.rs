//! Diagnosis values written by hand (`manual.diagnosis_override` of the
//! plugin, the operator `oa.diagnosis_edit`).
//!
//! The clinician measures four values of an arch themselves: the
//! discrepancy (space available minus required, negative is crowding), the
//! Little index, the dental midline offset and the largest space between
//! crowns. As in the plugin the values are kept in `mm`, with these limits
//! (the ranges of its dialog): discrepancy -30 to 30, Little 0 to 40,
//! midline -10 to 10, largest space 0 to 20.
//!
//! The complexity follows them: the CAT-CAT items are worked out again from
//! the values in force, with the rotations of the teeth as the diagnosis
//! stores them (rounded to 0.1 degree, as the plugin keeps them), and the
//! category of the Little index again. A value equal to the computed one is
//! not "by hand" (`abs(v - automatico) > 1e-9` in the plugin).
//!
//! Deviation: the plugin writes the hand values over the stored diagnosis
//! and keeps the computed ones in `automatico`; here the hand values live
//! apart in [`ArchEdits`] and the computed ones are never changed, so a reset
//! (`None`) brings the computed value back without a copy to keep in step.

use std::collections::BTreeMap;

use dc_case::{Fdi, ScanRole};
use dca_model::{AlignersState, ArchEdits};

use super::{CatcatItems, Level, LittleCategory, catcat_model_items, round_to};
use crate::anatomy::Arch;

/// A value the clinician can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    /// Space available minus required (mm), negative is crowding.
    Discrepancy,
    /// Little irregularity index (mm).
    Little,
    /// Dental midline offset (mm).
    MidlineOffset,
    /// Largest space between crowns (mm).
    LargestSpace,
}

impl EditField {
    /// Every field, in the order the plugin lists them.
    pub const ALL: [EditField; 4] = [
        EditField::Discrepancy,
        EditField::Little,
        EditField::MidlineOffset,
        EditField::LargestSpace,
    ];

    /// The name of the field in the command parameters (the plugin's key).
    pub fn key(self) -> &'static str {
        match self {
            Self::Discrepancy => "discrepancy",
            Self::Little => "little",
            Self::MidlineOffset => "midline_offset",
            Self::LargestSpace => "largest_space",
        }
    }

    /// The field with that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }

    /// The limits of the value, in mm, both included.
    pub fn range(self) -> (f64, f64) {
        match self {
            Self::Discrepancy => (-30.0, 30.0),
            Self::Little => (0.0, 40.0),
            Self::MidlineOffset => (-10.0, 10.0),
            Self::LargestSpace => (0.0, 20.0),
        }
    }

    /// `true` when `v` is a number inside the limits.
    pub fn accepts(self, v: f64) -> bool {
        let (lo, hi) = self.range();
        v.is_finite() && (lo..=hi).contains(&v)
    }

    /// The value written by hand in `edits`.
    pub fn get(self, edits: &ArchEdits) -> Option<f64> {
        match self {
            Self::Discrepancy => edits.discrepancy_mm,
            Self::Little => edits.little_index_mm,
            Self::MidlineOffset => edits.midline_offset_mm,
            Self::LargestSpace => edits.largest_space_mm,
        }
    }

    fn slot(self, edits: &mut ArchEdits) -> &mut Option<f64> {
        match self {
            Self::Discrepancy => &mut edits.discrepancy_mm,
            Self::Little => &mut edits.little_index_mm,
            Self::MidlineOffset => &mut edits.midline_offset_mm,
            Self::LargestSpace => &mut edits.largest_space_mm,
        }
    }
}

/// The four values of an arch, in mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values {
    /// Space available minus required, negative is crowding.
    pub discrepancy: f64,
    /// Little irregularity index.
    pub little: f64,
    /// Dental midline offset.
    pub midline_offset: f64,
    /// Largest space between crowns.
    pub largest_space: f64,
}

impl Values {
    /// One of the values.
    pub fn get(&self, field: EditField) -> f64 {
        match field {
            EditField::Discrepancy => self.discrepancy,
            EditField::Little => self.little,
            EditField::MidlineOffset => self.midline_offset,
            EditField::LargestSpace => self.largest_space,
        }
    }
}

/// What the diagnosis of an arch is with a set of hand values: the values
/// in force and what follows from them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effective {
    /// The values in force (hand values over the computed ones).
    pub values: Values,
    /// Category of the Little index in force.
    pub little_category: LittleCategory,
    /// CAT-CAT items worked out from them.
    pub items: CatcatItems,
    /// Sum of the items.
    pub score: u32,
    /// Difficulty level of the score.
    pub level: Level,
}

/// Writes (`Some`) or resets (`None`) one value in `edits`. A value equal to
/// the computed one (within 1e-9) is not kept: it is not "by hand".
pub fn set_value(edits: &mut ArchEdits, computed: &Values, field: EditField, value: Option<f64>) {
    *field.slot(edits) = value.filter(|v| (v - computed.get(field)).abs() > 1e-9);
}

/// The values the diagnosis computed for the arch; `None` until it is
/// analysed.
pub fn computed_values(data: &AlignersState, role: ScanRole) -> Option<Values> {
    let space = data.diagnosis.arches.get(&role)?;
    let details = data
        .diagnosis
        .details
        .get(&role)
        .copied()
        .unwrap_or_default();
    Some(Values {
        discrepancy: -space.crowding_mm(),
        little: space.little_index_mm,
        midline_offset: details.midline_offset_mm,
        largest_space: details.largest_space_mm,
    })
}

/// The rotation of each measured tooth of the arch about its long axis,
/// rounded to 0.1 degree (what the plugin's `analysis["rotations"]` holds).
pub fn rotations(data: &AlignersState, role: ScanRole) -> BTreeMap<Fdi, f64> {
    let Some(arch) = Arch::from_role(role) else {
        return BTreeMap::new();
    };
    data.tooth_geometry
        .iter()
        .filter(|(f, _)| Arch::of(**f) == arch)
        .map(|(f, m)| (*f, round_to(m.rot_dev_deg, 1)))
        .collect()
}

/// The diagnosis of an arch with the hand values `edits` over the `computed`
/// ones, the items worked out from `rotations` (`diagnosis_override`).
pub fn with_edits(
    computed: &Values,
    rotations: &BTreeMap<Fdi, f64>,
    edits: &ArchEdits,
) -> Effective {
    let values = Values {
        discrepancy: edits.discrepancy_mm.unwrap_or(computed.discrepancy),
        little: edits.little_index_mm.unwrap_or(computed.little),
        midline_offset: edits.midline_offset_mm.unwrap_or(computed.midline_offset),
        largest_space: edits.largest_space_mm.unwrap_or(computed.largest_space),
    };
    let items = catcat_model_items(
        values.discrepancy,
        rotations,
        &[],
        values.midline_offset,
        values.largest_space,
    );
    let score = items.score();
    Effective {
        values,
        little_category: LittleCategory::of(values.little),
        items,
        score,
        level: Level::of(score),
    }
}

/// The diagnosis of the arch with its hand values; `None` until analysed.
pub fn effective(data: &AlignersState, role: ScanRole) -> Option<Effective> {
    let computed = computed_values(data, role)?;
    Some(with_edits(
        &computed,
        &rotations(data, role),
        &data.diagnosis.edits_of(role),
    ))
}

/// The CAT-CAT score of the arch: the computed one while nothing was written
/// by hand, else the score worked out from the values in force.
pub fn effective_score(data: &AlignersState, role: ScanRole) -> Option<u32> {
    if data.diagnosis.edits_of(role).is_empty() {
        return data.diagnosis.details.get(&role).map(|d| d.catcat_score);
    }
    effective(data, role).map(|e| e.score)
}

/// The complexity score of the case: the largest score of the arches that
/// have a diagnosis, hand values included.
pub fn complexity_score(data: &AlignersState) -> Option<f64> {
    data.diagnosis
        .details
        .keys()
        .filter_map(|role| effective_score(data, *role))
        .max()
        .map(f64::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_model::{ArchDetails, ArchSpace};

    fn computed() -> Values {
        Values {
            discrepancy: -4.0,
            little: 5.0,
            midline_offset: 0.5,
            largest_space: 1.0,
        }
    }

    fn rot(pairs: &[(u8, f64)]) -> BTreeMap<Fdi, f64> {
        pairs
            .iter()
            .map(|(f, v)| (Fdi::new(*f).unwrap(), *v))
            .collect()
    }

    #[test]
    fn the_fields_have_the_plugin_names_and_limits() {
        let want = [
            ("discrepancy", -30.0, 30.0),
            ("little", 0.0, 40.0),
            ("midline_offset", -10.0, 10.0),
            ("largest_space", 0.0, 20.0),
        ];
        for (f, (key, lo, hi)) in EditField::ALL.into_iter().zip(want) {
            assert_eq!(f.key(), key);
            assert_eq!(EditField::from_key(key), Some(f));
            assert_eq!(f.range(), (lo, hi));
            assert!(f.accepts(lo) && f.accepts(hi) && f.accepts((lo + hi) / 2.0));
            assert!(!f.accepts(lo - 0.01) && !f.accepts(hi + 0.01));
            assert!(!f.accepts(f64::NAN) && !f.accepts(f64::INFINITY));
        }
        assert_eq!(EditField::from_key("overjet"), None);
    }

    #[test]
    fn the_items_follow_the_values_in_force() {
        let rotations = rot(&[(14, 12.0), (12, -21.0), (11, 30.0)]);
        // nothing written: the computed values
        let e = with_edits(&computed(), &rotations, &ArchEdits::default());
        assert_eq!(e.values, computed());
        // crowding 4 -> 2, rotations of 14 and 12: 2 * 1 + 2 * 2, the central is not counted
        assert_eq!(e.items.crowding, 2);
        assert_eq!(e.items.rotations, 6);
        assert_eq!((e.items.midline, e.items.diastema), (0, 0));
        assert_eq!(e.score, 8);
        assert_eq!(e.level, Level::Easy);
        assert_eq!(e.little_category, LittleCategory::Moderate);

        let hand = ArchEdits {
            discrepancy_mm: Some(-8.5),
            little_index_mm: Some(12.3),
            midline_offset_mm: Some(3.1),
            largest_space_mm: Some(4.0),
        };
        let e = with_edits(&computed(), &rotations, &hand);
        assert_eq!(e.items.crowding, 4);
        assert_eq!(e.items.midline, 2); // (3.1 - 2) * 2 = 2.2
        assert_eq!(e.items.diastema, 4); // (4 - 2) * 2
        assert_eq!(e.score, 4 + 6 + 2 + 4);
        assert_eq!(e.little_category, LittleCategory::VerySevere);
        assert_eq!(e.values.little, 12.3);
        // spacing is no crowding
        let hand = ArchEdits {
            discrepancy_mm: Some(2.0),
            ..ArchEdits::default()
        };
        assert_eq!(with_edits(&computed(), &rotations, &hand).items.crowding, 0);
    }

    #[test]
    fn a_value_equal_to_the_computed_one_is_not_by_hand() {
        let mut edits = ArchEdits::default();
        set_value(&mut edits, &computed(), EditField::Little, Some(7.0));
        assert_eq!(edits.little_index_mm, Some(7.0));
        // back to the computed value, or reset
        set_value(&mut edits, &computed(), EditField::Little, Some(5.0));
        assert!(edits.is_empty());
        set_value(&mut edits, &computed(), EditField::Little, Some(7.0));
        set_value(&mut edits, &computed(), EditField::Little, None);
        assert!(edits.is_empty());
        // the others are untouched
        set_value(&mut edits, &computed(), EditField::Discrepancy, Some(-1.0));
        set_value(&mut edits, &computed(), EditField::Little, Some(9.0));
        set_value(&mut edits, &computed(), EditField::Little, None);
        assert_eq!(edits.discrepancy_mm, Some(-1.0));
    }

    #[test]
    fn the_case_score_is_the_largest_of_the_arches_with_their_hand_values() {
        let mut data = AlignersState::default();
        for (role, req, score) in [(ScanRole::Upper, 74.0, 9), (ScanRole::Lower, 70.0, 5)] {
            data.diagnosis.arches.insert(
                role,
                ArchSpace {
                    available_mm: 70.0,
                    required_mm: req,
                    little_index_mm: 3.0,
                },
            );
            data.diagnosis.details.insert(
                role,
                ArchDetails {
                    catcat_score: score,
                    ..ArchDetails::default()
                },
            );
        }
        assert_eq!(complexity_score(&data), Some(9.0));
        assert_eq!(
            computed_values(&data, ScanRole::Upper).unwrap().discrepancy,
            -4.0
        );
        // the lower arch gets a crowding of 9 mm by hand: 4 points, below the upper 9
        data.diagnosis.edits.insert(
            ScanRole::Lower,
            ArchEdits {
                discrepancy_mm: Some(-9.0),
                ..ArchEdits::default()
            },
        );
        assert_eq!(effective_score(&data, ScanRole::Lower), Some(4));
        assert_eq!(effective_score(&data, ScanRole::Upper), Some(9));
        assert_eq!(complexity_score(&data), Some(9.0));
        data.diagnosis.edits.insert(
            ScanRole::Lower,
            ArchEdits {
                midline_offset_mm: Some(8.0),
                ..ArchEdits::default()
            },
        );
        assert_eq!(effective_score(&data, ScanRole::Lower), Some(12));
        assert_eq!(complexity_score(&data), Some(12.0));
        // an arch not analysed has none
        assert_eq!(effective(&AlignersState::default(), ScanRole::Lower), None);
    }
}
