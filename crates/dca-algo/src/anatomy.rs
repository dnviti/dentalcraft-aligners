//! Tooth anatomy tables and per-tooth helpers: FDI numbering along the arch,
//! typical sizes, crown and root lengths, proximal enamel thickness.
//!
//! Reference: `ortho_aligner/anatomy.py`.
//!
//! Crown and root lengths: Wheeler's Dental Anatomy (Nelson 2019). Proximal
//! enamel thickness: Firinciogullari et al. 2025 (CBCT) and Gonzalez-Garcia
//! et al. 2025 (CBCT); the smaller value is used (conservative). Crown
//! diameters are *typical* Wheeler values: they feed heuristics (seed search
//! radii) and the synthetic demo model only. Every clinical measurement of
//! the plan is taken from the patient's scan.
//!
//! Teeth are [`Fdi`] numbers from `dc-case`. Unlike the plugin (which takes
//! any integer), the arch of a tooth follows [`Fdi::is_upper`], so deciduous
//! teeth are placed correctly; their sizes use the permanent tooth of the
//! same position.

use dc_case::{Fdi, ScanRole};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An arch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    /// Upper (maxillary) arch.
    Upper,
    /// Lower (mandibular) arch.
    Lower,
}

impl Arch {
    /// Arch of a tooth.
    pub fn of(fdi: Fdi) -> Self {
        if fdi.is_upper() {
            Arch::Upper
        } else {
            Arch::Lower
        }
    }

    /// Arch of a scan role (`None` for a bite scan).
    pub fn from_role(role: ScanRole) -> Option<Self> {
        match role {
            ScanRole::Upper => Some(Arch::Upper),
            ScanRole::Lower => Some(Arch::Lower),
            ScanRole::Bite => None,
        }
    }

    /// The scan role of this arch.
    pub fn role(self) -> ScanRole {
        match self {
            Arch::Upper => ScanRole::Upper,
            Arch::Lower => ScanRole::Lower,
        }
    }

    /// `true` for the upper arch.
    pub fn is_upper(self) -> bool {
        self == Arch::Upper
    }

    /// The other arch.
    pub fn opposite(self) -> Self {
        match self {
            Arch::Upper => Arch::Lower,
            Arch::Lower => Arch::Upper,
        }
    }
}

/// Permanent teeth along the arch, from the patient's right distal to the
/// left distal (upper: 18..11, 21..28; lower: 48..41, 31..38).
pub fn arch_order(arch: Arch) -> [Fdi; 16] {
    let mut out = [Fdi::PERMANENT[0]; 16];
    for (slot, f) in out.iter_mut().zip(Fdi::permanent_arch(arch.is_upper())) {
        *slot = f;
    }
    out
}

/// Names of the tooth types by position (1 = central incisor ... 8 = third
/// molar), in Italian as in the plugin's reports.
pub const TYPE_NAMES_IT: [&str; 8] = [
    "incisivo centrale",
    "incisivo laterale",
    "canino",
    "primo premolare",
    "secondo premolare",
    "primo molare",
    "secondo molare",
    "terzo molare",
];

/// Position digit of the tooth from the midline (1..=8).
pub fn index(fdi: Fdi) -> u8 {
    fdi.position()
}

/// Quadrant digit.
pub fn quadrant(fdi: Fdi) -> u8 {
    fdi.quadrant()
}

/// Arch of a tooth.
pub fn arch_of(fdi: Fdi) -> Arch {
    Arch::of(fdi)
}

/// Patient's right side (quadrants 1 and 4).
pub fn is_right(fdi: Fdi) -> bool {
    fdi.is_right()
}

/// Incisors and canines.
pub fn is_anterior(fdi: Fdi) -> bool {
    index(fdi) <= 3
}

/// Central and lateral incisors.
pub fn is_incisor(fdi: Fdi) -> bool {
    index(fdi) <= 2
}

/// Canines and premolars: rounded crowns, derotation is less predictable
/// (Simon 2014; Castroflorio 2023).
pub fn is_round_crown(fdi: Fdi) -> bool {
    matches!(index(fdi), 3..=5)
}

/// Molars (first, second, third).
pub fn is_molar(fdi: Fdi) -> bool {
    index(fdi) >= 6
}

/// `"11 incisivo centrale"`: number and type name (Italian, as the plugin).
pub fn tooth_name(fdi: Fdi) -> String {
    let name = TYPE_NAMES_IT
        .get(usize::from(index(fdi)).wrapping_sub(1))
        .copied()
        .unwrap_or("");
    format!("{} {}", fdi.get(), name)
}

/// A pair of values (mesio-distal and bucco-lingual, or mesial and distal).
type Pair = (f64, f64);

/// Table index of a tooth position (positions above 8 count as 8).
fn slot(position: u8, max: u8) -> usize {
    usize::from(position.clamp(1, max)) - 1
}

/// (crown length, root length) in mm per position 1..=8, Wheeler. Molar
/// roots: mean of the buccal and lingual root lengths.
const LENGTHS_UPPER: [(f64, f64); 8] = [
    (10.5, 13.0),
    (9.0, 13.0),
    (10.0, 17.0),
    (8.5, 14.0),
    (8.5, 14.0),
    (7.5, 12.5),
    (7.0, 11.5),
    (6.5, 11.0),
];
const LENGTHS_LOWER: [(f64, f64); 8] = [
    (9.0, 12.5),
    (9.5, 14.0),
    (11.0, 16.0),
    (8.5, 14.0),
    (8.0, 14.5),
    (7.5, 14.0),
    (7.0, 13.0),
    (7.0, 11.0),
];

/// Typical crown diameters (mesio-distal, bucco-lingual) in mm, Wheeler
/// (heuristics and demo only).
const DIAMETERS_UPPER: [(f64, f64); 8] = [
    (8.5, 7.0),
    (6.5, 6.0),
    (7.5, 8.0),
    (7.0, 9.0),
    (7.0, 9.0),
    (10.0, 11.0),
    (9.0, 11.0),
    (8.5, 10.0),
];
const DIAMETERS_LOWER: [(f64, f64); 8] = [
    (5.0, 6.0),
    (5.5, 6.5),
    (7.0, 7.5),
    (7.0, 7.5),
    (7.0, 8.0),
    (11.0, 10.5),
    (10.5, 10.0),
    (10.0, 9.5),
];

/// Mean proximal enamel thickness (mesial, distal) in mm, positions 1..=7
/// (Firinciogullari 2025).
const ENAMEL_FIRINCIOGULLARI_UPPER: [(f64, f64); 7] = [
    (1.24, 1.30),
    (1.11, 1.10),
    (1.45, 1.45),
    (1.18, 1.16),
    (1.16, 1.20),
    (1.30, 1.41),
    (1.35, 1.58),
];
const ENAMEL_FIRINCIOGULLARI_LOWER: [(f64, f64); 7] = [
    (1.03, 1.07),
    (1.06, 1.07),
    (1.31, 1.33),
    (1.25, 1.12),
    (1.20, 1.13),
    (1.29, 1.38),
    (1.32, 1.38),
];
/// Gonzalez-Garcia 2025, right/left averaged, positions 1..=5 (second
/// premolar to second premolar).
const ENAMEL_GONZALEZ_UPPER: [(f64, f64); 5] = [
    (1.16, 1.15),
    (1.07, 1.03),
    (1.22, 1.20),
    (1.06, 1.02),
    (1.05, 1.09),
];
const ENAMEL_GONZALEZ_LOWER: [(f64, f64); 5] = [
    (1.02, 1.02),
    (1.02, 0.99),
    (1.26, 1.12),
    (1.07, 1.05),
    (1.07, 1.10),
];

/// A proximal surface of a tooth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Towards the midline.
    Mesial,
    /// Away from the midline.
    Distal,
}

/// Crown and root length in mm (Wheeler).
pub fn crown_root_length(fdi: Fdi) -> (f64, f64) {
    let table = match arch_of(fdi) {
        Arch::Upper => &LENGTHS_UPPER,
        Arch::Lower => &LENGTHS_LOWER,
    };
    table[slot(index(fdi), 8)]
}

/// Typical crown diameters (mesio-distal, bucco-lingual) in mm.
pub fn typical_diameters(fdi: Fdi) -> (f64, f64) {
    let table = match arch_of(fdi) {
        Arch::Upper => &DIAMETERS_UPPER,
        Arch::Lower => &DIAMETERS_LOWER,
    };
    table[slot(index(fdi), 8)]
}

/// Proximal enamel thickness in mm: the smaller of the two studies where
/// both give a value (third molars use the second molar).
pub fn enamel_thickness(fdi: Fdi, surface: Surface) -> f64 {
    let (first, second): (&[Pair], &[Pair]) = match arch_of(fdi) {
        Arch::Upper => (&ENAMEL_FIRINCIOGULLARI_UPPER, &ENAMEL_GONZALEZ_UPPER),
        Arch::Lower => (&ENAMEL_FIRINCIOGULLARI_LOWER, &ENAMEL_GONZALEZ_LOWER),
    };
    let pick = |pair: (f64, f64)| match surface {
        Surface::Mesial => pair.0,
        Surface::Distal => pair.1,
    };
    let i = slot(index(fdi), 7);
    let mut v = pick(first[i]);
    if let Some(&pair) = second.get(i) {
        v = v.min(pick(pair));
    }
    v
}

/// Consensus cap of the enamel reduction on one proximal surface (mm,
/// Wang 2025).
pub const IPR_CAP_MM: f64 = 0.25;
/// Largest fraction of the enamel thickness that may be removed.
pub const IPR_ENAMEL_FRACTION: f64 = 0.5;

/// Maximum enamel reduction on one proximal surface: the consensus cap and
/// never more than a fraction of the enamel thickness.
pub fn max_ipr_surface(fdi: Fdi, surface: Surface, cap_mm: f64, enamel_fraction: f64) -> f64 {
    cap_mm.min(enamel_fraction * enamel_thickness(fdi, surface))
}

/// [`max_ipr_surface`] with the consensus defaults (0.25 mm, 50%).
pub fn max_ipr_surface_default(fdi: Fdi, surface: Surface) -> f64 {
    max_ipr_surface(fdi, surface, IPR_CAP_MM, IPR_ENAMEL_FRACTION)
}

/// The teeth to number by hand, one after the other (`oa.place_seeds` of the
/// plugin): the arch order, from the patient's right distal to the left
/// distal, without the third molars unless `third_molars`, and without the
/// teeth in `taken` (those that already have a mark or are missing).
pub fn numbering_sequence(arch: Arch, third_molars: bool, taken: &[Fdi]) -> Vec<Fdi> {
    arch_order(arch)
        .into_iter()
        .filter(|f| (third_molars || index(*f) != 8) && !taken.contains(f))
        .collect()
}

/// The teeth of `fdis` that belong to the arch sequence, in arch order.
pub fn arch_sequence(fdis: &[Fdi], arch: Arch) -> Vec<Fdi> {
    arch_order(arch)
        .into_iter()
        .filter(|f| fdis.contains(f))
        .collect()
}

/// Neighbour on the mesial side along the arch sequence (`None` at the end
/// of the sequence, or when `fdi` is not in it). Across the midline the
/// mesial neighbour of a central incisor is the contralateral central.
pub fn mesial_neighbor(seq: &[Fdi], fdi: Fdi) -> Option<Fdi> {
    let i = seq.iter().position(|&f| f == fdi)?;
    let j = if is_right(fdi) {
        i.checked_add(1)?
    } else {
        i.checked_sub(1)?
    };
    let other = *seq.get(j)?;
    (is_right(fdi) == is_right(other) || (index(fdi) == 1 && index(other) == 1)).then_some(other)
}

/// Neighbour on the distal side along the arch sequence (`None` at the end
/// of the sequence, or when `fdi` is not in it).
pub fn distal_neighbor(seq: &[Fdi], fdi: Fdi) -> Option<Fdi> {
    let i = seq.iter().position(|&f| f == fdi)?;
    let j = if is_right(fdi) {
        i.checked_sub(1)?
    } else {
        i.checked_add(1)?
    };
    let other = *seq.get(j)?;
    (is_right(other) == is_right(fdi)).then_some(other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    fn nums(fdis: &[Fdi]) -> Vec<u8> {
        fdis.iter().map(|x| x.get()).collect()
    }

    #[test]
    fn arch_order_matches_the_plugin_tables() {
        assert_eq!(
            nums(&arch_order(Arch::Upper)),
            [
                18, 17, 16, 15, 14, 13, 12, 11, 21, 22, 23, 24, 25, 26, 27, 28
            ]
        );
        assert_eq!(
            nums(&arch_order(Arch::Lower)),
            [
                48, 47, 46, 45, 44, 43, 42, 41, 31, 32, 33, 34, 35, 36, 37, 38
            ]
        );
    }

    #[test]
    fn the_numbering_sequence_skips_third_molars_and_taken_teeth() {
        assert_eq!(
            nums(&numbering_sequence(Arch::Upper, false, &[])),
            [17, 16, 15, 14, 13, 12, 11, 21, 22, 23, 24, 25, 26, 27]
        );
        assert_eq!(numbering_sequence(Arch::Lower, true, &[]).len(), 16);
        assert_eq!(
            nums(&numbering_sequence(Arch::Lower, true, &[f(48), f(41)])[..3]),
            [47, 46, 45]
        );
        // the other arch's teeth are never in it
        assert!(
            numbering_sequence(Arch::Lower, true, &[])
                .iter()
                .all(|x| x.get() >= 31)
        );
    }

    #[test]
    fn predicates() {
        assert_eq!(arch_of(f(11)), Arch::Upper);
        assert_eq!(arch_of(f(26)), Arch::Upper);
        assert_eq!(arch_of(f(36)), Arch::Lower);
        assert_eq!(arch_of(f(48)), Arch::Lower);
        assert!(is_right(f(11)) && is_right(f(46)));
        assert!(!is_right(f(21)) && !is_right(f(31)));
        assert!(is_anterior(f(13)) && !is_anterior(f(14)));
        assert!(is_incisor(f(12)) && !is_incisor(f(13)));
        assert!(is_round_crown(f(13)) && is_round_crown(f(25)) && !is_round_crown(f(16)));
        assert!(is_molar(f(36)) && !is_molar(f(35)));
        assert_eq!(tooth_name(f(11)), "11 incisivo centrale");
        assert_eq!(tooth_name(f(38)), "38 terzo molare");
        assert_eq!(index(f(46)), 6);
        assert_eq!(quadrant(f(46)), 4);
    }

    #[test]
    fn sizes_come_from_the_wheeler_tables() {
        assert_eq!(crown_root_length(f(11)), (10.5, 13.0));
        assert_eq!(crown_root_length(f(33)), (11.0, 16.0));
        assert_eq!(crown_root_length(f(28)), (6.5, 11.0));
        assert_eq!(typical_diameters(f(16)), (10.0, 11.0));
        assert_eq!(typical_diameters(f(31)), (5.0, 6.0));
        assert_eq!(typical_diameters(f(48)), (10.0, 9.5));
    }

    #[test]
    fn enamel_takes_the_smaller_study() {
        // upper central: 1.24/1.30 against 1.16/1.15
        assert_eq!(enamel_thickness(f(11), Surface::Mesial), 1.16);
        assert_eq!(enamel_thickness(f(11), Surface::Distal), 1.15);
        // lower lateral distal: 1.07 against 0.99
        assert_eq!(enamel_thickness(f(32), Surface::Distal), 0.99);
        // molars have only the first study; third molars use the second
        assert_eq!(enamel_thickness(f(16), Surface::Mesial), 1.30);
        assert_eq!(enamel_thickness(f(28), Surface::Distal), 1.58);
        assert_eq!(enamel_thickness(f(38), Surface::Mesial), 1.32);
    }

    #[test]
    fn ipr_is_capped_by_the_surface_and_the_enamel() {
        for fdi in Fdi::PERMANENT {
            for s in [Surface::Mesial, Surface::Distal] {
                let m = max_ipr_surface_default(fdi, s);
                assert!(m > 0.0 && m <= 0.25, "{fdi} {s:?} {m}");
                assert!(m <= 0.5 * enamel_thickness(fdi, s) + 1e-12);
            }
        }
        assert_eq!(max_ipr_surface(f(11), Surface::Mesial, 0.25, 0.5), 0.25);
        // thin enamel binds before the cap
        assert!((max_ipr_surface(f(32), Surface::Distal, 0.25, 0.2) - 0.198).abs() < 1e-12);
    }

    #[test]
    fn neighbours_follow_the_arch_sequence() {
        let all: Vec<Fdi> = arch_order(Arch::Upper).to_vec();
        let seq = arch_sequence(&[f(11), f(21), f(12), f(22), f(16)], Arch::Upper);
        assert_eq!(nums(&seq), [16, 12, 11, 21, 22]);
        assert_eq!(nums(&arch_sequence(&all, Arch::Upper)), nums(&all));

        // right side: mesial is towards the midline
        assert_eq!(mesial_neighbor(&seq, f(12)), Some(f(11)));
        assert_eq!(distal_neighbor(&seq, f(12)), Some(f(16)));
        assert_eq!(mesial_neighbor(&seq, f(16)), Some(f(12)));
        assert_eq!(distal_neighbor(&seq, f(16)), None);
        // across the midline only the centrals are mesial neighbours
        assert_eq!(mesial_neighbor(&seq, f(11)), Some(f(21)));
        assert_eq!(mesial_neighbor(&seq, f(21)), Some(f(11)));
        assert_eq!(distal_neighbor(&seq, f(11)), Some(f(12)));
        assert_eq!(distal_neighbor(&seq, f(21)), Some(f(22)));
        assert_eq!(mesial_neighbor(&seq, f(22)), Some(f(21)));
        assert_eq!(mesial_neighbor(&seq, f(26)), None);
        // missing central: the lateral is not a mesial neighbour across the midline
        let gap = arch_sequence(&[f(12), f(22)], Arch::Upper);
        assert_eq!(mesial_neighbor(&gap, f(12)), None);
        assert_eq!(distal_neighbor(&gap, f(12)), None);
    }

    #[test]
    fn arch_and_scan_role() {
        assert_eq!(Arch::from_role(ScanRole::Upper), Some(Arch::Upper));
        assert_eq!(Arch::from_role(ScanRole::Bite), None);
        assert_eq!(Arch::Lower.role(), ScanRole::Lower);
        assert_eq!(Arch::Lower.opposite(), Arch::Upper);
        assert_eq!(serde_json::to_string(&Arch::Upper).unwrap(), "\"upper\"");
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// A valid tooth number (permanent or deciduous).
    fn any_fdi() -> Fdi {
        let n: u8 = kani::any();
        match Fdi::new(n) {
            Ok(f) => f,
            Err(_) => {
                kani::assume(false);
                unreachable!()
            }
        }
    }

    /// A tooth of the permanent arch order.
    fn any_in_order(order: &[Fdi; 16]) -> Fdi {
        let i: usize = kani::any();
        kani::assume(i < 16);
        order[i]
    }

    // Every table index is inside its table, for any position digit and any table of 1 to 8
    // rows.
    #[kani::proof]
    fn slot_is_inside_the_table() {
        let position: u8 = kani::any();
        let max: u8 = kani::any();
        kani::assume((1..=8).contains(&max));
        assert!(slot(position, max) < usize::from(max));
    }

    // The anatomy tables answer every valid tooth, deciduous ones included, without panicking,
    // and every length, diameter and enamel thickness they give is positive.
    #[kani::proof]
    fn the_tables_answer_every_tooth() {
        let f = any_fdi();
        let (crown, root) = crown_root_length(f);
        let (md, bl) = typical_diameters(f);
        let mesial = enamel_thickness(f, Surface::Mesial);
        let distal = enamel_thickness(f, Surface::Distal);
        assert!(crown > 0.0 && root > 0.0 && md > 0.0 && bl > 0.0);
        assert!(mesial > 0.0 && distal > 0.0);
    }

    // Along any subset of the arch, a tooth is the distal neighbour of its mesial neighbour,
    // and the other way round, except across the midline, where each central incisor is the
    // mesial neighbour of the other. (The subset is built in an array: a vector pushed a
    // symbolic number of times made the solver run out of memory.)
    #[kani::proof]
    #[kani::unwind(17)]
    fn mesial_and_distal_neighbours_are_symmetric() {
        for arch in [Arch::Upper, Arch::Lower] {
            let order = arch_order(arch);
            let mask: u16 = kani::any();
            let mut subset = order;
            let mut len = 0;
            for (i, f) in order.iter().enumerate() {
                if (mask >> i) & 1 == 1 {
                    subset[len] = *f;
                    len += 1;
                }
            }
            let seq = &subset[..len];
            let a = any_in_order(&order);
            let b = any_in_order(&order);
            let centrals = index(a) == 1 && index(b) == 1;
            if mesial_neighbor(seq, a) == Some(b) && !centrals {
                assert_eq!(distal_neighbor(seq, b), Some(a));
            }
            if distal_neighbor(seq, b) == Some(a) {
                assert_eq!(mesial_neighbor(seq, a), Some(b));
            }
        }
    }
}
