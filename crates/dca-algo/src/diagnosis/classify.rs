//! Deterministic, evidence-based classifications (the reference "math"
//! classifications of the plugin).
//!
//! Reference: `ortho_aligner/classify.py`.
//!
//! * tooth type from the crown dimensions: nearest typical Wheeler crown
//!   (Nelson 2019);
//! * segmentation plausibility: crown dimensions within anatomical ranges;
//! * movement predictability risk: Simon 2014, Kravitz 2008/2009, Haouili
//!   2020, Koletsi 2021, Houle 2017, Zhou 2020, Wang 2025 consensus;
//! * attachment indication: Wang 2025 consensus, Elshazly 2024, Nucera 2022,
//!   Ravera 2016;
//! * case difficulty: CAT-CAT items (Long 2020) to consensus levels (Wang
//!   2025), see [`super::Level`];
//! * staging pattern: Martinez-Lozano 2024;
//! * aligner-only suitability: extraction threshold 8 mm, predictable
//!   distalization below 2 mm (Wang 2025).
//!
//! The texts of the flags and of the reasons are the plugin's (Italian) and
//! are kept verbatim, like the notes of [`crate::evidence`]: they are
//! clinical documentation attached to a source, not interface strings.

use dc_case::Fdi;
use dca_model::StagingPattern;
use serde::Serialize;

use crate::anatomy::{self, Arch};
use crate::evidence;

/// Distance scale of [`tooth_type`] (mm): a crown 0.7 mm off in one diameter
/// is one unit from the typical one.
const TYPE_SCALE_MM: f64 = 0.7;

/// Names of the tooth types in the plugin (`TOOTH_TYPES`), by position 1..=7.
pub const TOOTH_TYPES: [&str; 7] = [
    "incisivo_centrale",
    "incisivo_laterale",
    "canino",
    "primo_premolare",
    "secondo_premolare",
    "primo_molare",
    "secondo_molare",
];

/// Plugin name of the type of a tooth position (1..=7), `None` for a third
/// molar or an invalid position.
pub fn type_of_position(position: u8) -> Option<&'static str> {
    TOOTH_TYPES
        .get(usize::from(position).wrapping_sub(1))
        .copied()
}

/// Result of [`tooth_type`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToothType {
    /// Nearest typical crown (plugin name, e.g. `"canino"`).
    pub best: &'static str,
    /// Distance from each typical crown, in the order of [`TOOTH_TYPES`].
    pub scores: [f64; 7],
}

/// Nearest typical crown (mesio-distal, bucco-lingual) of the same arch.
pub fn tooth_type(fdi: Fdi, width_md: f64, width_bl: f64) -> ToothType {
    let base = if anatomy::arch_of(fdi) == Arch::Upper {
        10
    } else {
        40
    };
    let mut scores = [0.0; 7];
    for (i, score) in scores.iter_mut().enumerate() {
        let typical = Fdi::new(base + i as u8 + 1).expect("valid tooth number");
        let (md, bl) = anatomy::typical_diameters(typical);
        *score = ((width_md - md) / TYPE_SCALE_MM).hypot((width_bl - bl) / TYPE_SCALE_MM);
    }
    let mut best = 0;
    for (i, s) in scores.iter().enumerate() {
        if *s < scores[best] {
            best = i;
        }
    }
    ToothType {
        best: TOOTH_TYPES[best],
        scores,
    }
}

/// Whether the crown dimensions are anatomically plausible for the tooth
/// (`segmentation_plausible`): the reasons when they are not.
pub fn segmentation_plausible(
    fdi: Fdi,
    width_md: f64,
    width_bl: f64,
    height: f64,
) -> (bool, Vec<String>) {
    let (md, bl) = anatomy::typical_diameters(fdi);
    let mut reasons = Vec::new();
    if !(0.7 * md..=1.35 * md).contains(&width_md) {
        reasons.push(format!(
            "diametro MD {width_md:.1} mm fuori dall'intervallo atteso ({:.1}-{:.1})",
            0.7 * md,
            1.35 * md
        ));
    }
    if !(0.6 * bl..=1.45 * bl).contains(&width_bl) {
        reasons.push(format!(
            "diametro VL {width_bl:.1} mm fuori dall'intervallo atteso ({:.1}-{:.1})",
            0.6 * bl,
            1.45 * bl
        ));
    }
    if !(2.5..=13.0).contains(&height) {
        reasons.push(format!("altezza clinica {height:.1} mm non plausibile"));
    }
    (reasons.is_empty(), reasons)
}

/// Clinical description of one tooth movement (`staging.describe_q` of the
/// plugin, with the plugin's keys in the comments).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct MovementDesc {
    /// `rotazione mesio-vestibolare(+) deg`.
    pub rotation_deg: f64,
    /// `estrusione(+)/intrusione(-) mm`.
    pub extrusion_mm: f64,
    /// `torque corona vestibolare(+) deg`.
    pub torque_deg: f64,
    /// `vestibolare(+)/linguale(-) mm`.
    pub buccal_mm: f64,
    /// `mesiale(+)/distale(-) mm`.
    pub mesial_mm: f64,
}

/// A note about the predictability of a movement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MovementFlag {
    /// 1 = caution, 2 = high risk.
    pub severity: u8,
    /// Which note it is (stable, for the interface to word it):
    /// `round-rotation`, `canine-rotation`, `extrusion`,
    /// `lower-incisor-intrusion`, `torque`, `expansion-tipping`,
    /// `distalisation`, `distalisation-limit`.
    pub id: &'static str,
    /// The note (plugin text).
    pub text: String,
    /// Keys of the references behind it ([`evidence::reference`]).
    pub refs: Vec<&'static str>,
}

fn flag(severity: u8, id: &'static str, text: &str, refs: &[&'static str]) -> MovementFlag {
    MovementFlag {
        severity,
        id,
        text: text.to_owned(),
        refs: refs.to_vec(),
    }
}

/// Evidence-based predictability notes for one tooth movement.
pub fn movement_flags(fdi: Fdi, d: &MovementDesc) -> Vec<MovementFlag> {
    let mut out = Vec::new();
    let rot = d.rotation_deg.abs();
    let ext = d.extrusion_mm;
    let tor = d.torque_deg.abs();
    let (bl, md) = (d.buccal_mm, d.mesial_mm);
    if anatomy::is_round_crown(fdi) && rot > 15.0 {
        out.push(flag(
            if rot > 30.0 { 2 } else { 1 },
            "round-rotation",
            "rotazione >15 gradi di corona tonda: accuratezza ~24%; valutare attachment e \
             ipercorrezione",
            &["simon2014", "palone2023"],
        ));
    }
    if anatomy::index(fdi) == 3 && rot > 5.0 {
        out.push(flag(
            1,
            "canine-rotation",
            "rotazione del canino poco prevedibile",
            &["kravitz2008", "koletsi2021"],
        ));
    }
    if ext > 0.5 {
        out.push(flag(
            if ext > 1.5 { 2 } else { 1 },
            "extrusion",
            "estrusione: movimento meno prevedibile; attachment rettangolare orizzontale",
            &["kravitz2009", "elshazly2024"],
        ));
    }
    if ext < -0.5 && anatomy::arch_of(fdi) == Arch::Lower && anatomy::is_incisor(fdi) {
        out.push(flag(
            1,
            "lower-incisor-intrusion",
            "intrusione incisivi inferiori: accuratezza 35%",
            &["haouili2020"],
        ));
    }
    if tor > 3.0 {
        out.push(flag(
            1,
            "torque",
            "torque >3 gradi: power ridge/attachment",
            &["wang2025consensus"],
        ));
    }
    if !anatomy::is_anterior(fdi) && bl > 0.5 {
        out.push(flag(
            1,
            "expansion-tipping",
            "espansione ottenuta soprattutto per tipping; attachment consigliati",
            &["houle2017", "zhou2020", "wang2025consensus"],
        ));
    }
    if anatomy::is_molar(fdi) && md < -0.5 {
        out.push(flag(
            1,
            "distalisation",
            "distalizzazione: schema sequenziale e attachment",
            &["ravera2016"],
        ));
        if md < -distal_predictable() {
            out.push(flag(
                2,
                "distalisation-limit",
                "oltre il limite prevedibile di 2 mm",
                &["wang2025consensus"],
            ));
        }
    }
    out
}

/// Risk level of a movement: 0 low (`basso`), 1 medium (`medio`), 2 high
/// (`alto`), with the notes behind it.
pub fn movement_risk(fdi: Fdi, d: &MovementDesc) -> (u8, Vec<MovementFlag>) {
    let flags = movement_flags(fdi, d);
    if flags.is_empty() {
        return (0, flags);
    }
    let level = if flags.iter().any(|f| f.severity >= 2) || flags.len() >= 3 {
        2
    } else {
        1
    };
    (level, flags)
}

/// Names of the risk levels (`RISK`).
pub const RISK: [&str; 3] = ["basso", "medio", "alto"];

/// Attachment indicated for a movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attachment {
    /// No attachment needed.
    Nessuno,
    /// Rotation of a round crown.
    Rotazione,
    /// Extrusion.
    Estrusione,
    /// Torque or root control (power ridge).
    ControlloRadicolare,
    /// Bodily movement, expansion or anchorage.
    CorporeoAncoraggio,
}

impl Attachment {
    /// The plugin's description (`ATTACH`).
    pub fn description(self) -> &'static str {
        match self {
            Attachment::Nessuno => "Nessun attachment necessario per i movimenti pianificati",
            Attachment::Rotazione => "Attachment per la rotazione (denti a corona tonda)",
            Attachment::Estrusione => {
                "Attachment rettangolare orizzontale smussato per l'estrusione"
            }
            Attachment::ControlloRadicolare => {
                "Power ridge / attachment per torque o controllo radicolare"
            }
            Attachment::CorporeoAncoraggio => {
                "Attachment rettangolare per movimento corporeo, espansione o ancoraggio"
            }
        }
    }
}

/// The attachment a movement calls for (`attachment_rule`), first matching
/// rule.
pub fn attachment_rule(fdi: Fdi, d: &MovementDesc) -> Attachment {
    let rot = d.rotation_deg.abs();
    let tor = d.torque_deg.abs();
    if d.extrusion_mm > 0.5 {
        Attachment::Estrusione
    } else if anatomy::is_round_crown(fdi) && rot > 15.0 {
        Attachment::Rotazione
    } else if anatomy::is_anterior(fdi) && tor > 3.0 {
        Attachment::ControlloRadicolare
    } else if (!anatomy::is_anterior(fdi) && d.buccal_mm > 0.5)
        || (anatomy::is_molar(fdi) && d.mesial_mm < -0.5)
    {
        Attachment::CorporeoAncoraggio
    } else {
        Attachment::Nessuno
    }
}

/// Consensus level of a CAT-CAT score as an index into
/// [`super::Level::ALL`], and whether the score is within 3 points of a
/// threshold (`complexity_math`).
pub fn complexity_math(score: f64) -> (usize, bool) {
    let level = if score <= 20.0 {
        0
    } else if score <= 40.0 {
        1
    } else if score <= 60.0 {
        2
    } else {
        3
    };
    let borderline = [20.0, 40.0, 60.0]
        .iter()
        .map(|t| (score - t).abs())
        .fold(f64::INFINITY, f64::min)
        <= 3.0;
    (level, borderline)
}

/// What the staging pattern is chosen from (`case_plan.complexity` of the
/// plugin).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct PatternInput {
    /// The case is not at the easiest consensus level.
    pub level_above_easy: bool,
    /// Crowding (mm, 0 when there is none).
    pub crowding_mm: f64,
    /// Little irregularity index (mm).
    pub little_mm: f64,
    /// Largest molar distalization (mm).
    pub distalization_mm: f64,
}

/// Staging pattern of the case (`staging.choose_pattern`).
pub fn choose_pattern(cx: &PatternInput) -> StagingPattern {
    if cx.distalization_mm > 0.5 {
        StagingPattern::SequentialDistal
    } else if cx.level_above_easy || cx.crowding_mm > 3.0 || cx.little_mm > 6.0 {
        StagingPattern::Structured
    } else {
        StagingPattern::Proportional
    }
}

/// The pattern and whether the case is close to a threshold of the choice
/// (`pattern_math`).
pub fn pattern_math(cx: &PatternInput) -> (StagingPattern, bool) {
    let borderline = (cx.crowding_mm - 3.0).abs() <= 0.5
        || (cx.little_mm - 6.0).abs() <= 1.0
        || (cx.distalization_mm - 0.5).abs() <= 0.25;
    (choose_pattern(cx), borderline)
}

fn distal_predictable() -> f64 {
    evidence::param_f64("distal_predictable").unwrap_or(2.0)
}

/// Whether the case can be treated with aligners only, and why not
/// (`aligner_suitable_math`). `discrepancy` is available minus required
/// space (negative = crowding), `distal` the molar distalization (mm).
pub fn aligner_suitable(discrepancy: f64, distal: f64) -> (bool, Vec<String>) {
    let extraction = evidence::param_f64("extraction_threshold").unwrap_or(8.0);
    let cite = evidence::cite(&["wang2025consensus"]);
    let mut reasons = Vec::new();
    if -discrepancy > extraction {
        reasons.push(format!(
            "discrepanza {discrepancy:.1} mm oltre la soglia di estrazione ({cite})"
        ));
    }
    if distal > distal_predictable() {
        reasons.push(format!(
            "distalizzazione {distal:.1} mm oltre il limite prevedibile ({cite})"
        ));
    }
    (reasons.is_empty(), reasons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn a_typical_crown_is_its_own_type() {
        for n in [11, 12, 13, 14, 15, 16, 17, 31, 32, 33, 34, 35, 36, 37] {
            let f = fdi(n);
            let (md, bl) = anatomy::typical_diameters(f);
            let t = tooth_type(f, md, bl);
            // typical premolars can share their diameters: the first of them wins a tie
            let own = usize::from(anatomy::index(f)) - 1;
            assert_eq!(t.scores[own], 0.0, "{n}");
            assert!(t.scores.iter().all(|s| *s >= 0.0));
            if !matches!(anatomy::index(f), 4 | 5) {
                assert_eq!(Some(t.best), type_of_position(anatomy::index(f)), "{n}");
            }
        }
        // a wide round crown on a lower arch is a molar, not a canine
        assert_eq!(tooth_type(fdi(36), 11.0, 10.5).best, "primo_molare");
    }

    #[test]
    fn plausibility_reports_each_out_of_range_dimension() {
        let (md, bl) = anatomy::typical_diameters(fdi(11));
        assert_eq!(segmentation_plausible(fdi(11), md, bl, 9.0), (true, vec![]));
        let (ok, reasons) = segmentation_plausible(fdi(11), 3.0, 30.0, 20.0);
        assert!(!ok);
        assert_eq!(reasons.len(), 3);
        assert!(reasons[0].starts_with("diametro MD 3.0 mm"));
        assert!(reasons[2].contains("altezza clinica 20.0 mm"));
    }

    #[test]
    fn movement_flags_follow_the_thresholds() {
        let none = MovementDesc::default();
        assert_eq!(movement_risk(fdi(14), &none).0, 0);
        assert_eq!(attachment_rule(fdi(14), &none), Attachment::Nessuno);

        // 20 degree rotation of a premolar: medium; 35: high; both want an attachment
        let rot = |deg| MovementDesc {
            rotation_deg: deg,
            ..none
        };
        assert_eq!(movement_risk(fdi(14), &rot(20.0)).0, 1);
        assert_eq!(movement_risk(fdi(14), &rot(-35.0)).0, 2);
        assert_eq!(attachment_rule(fdi(14), &rot(20.0)), Attachment::Rotazione);
        // a lateral incisor is not a round crown
        assert_eq!(movement_risk(fdi(12), &rot(20.0)).0, 0);
        // canine rotation above 5 degrees is flagged by its own rule
        assert_eq!(movement_flags(fdi(13), &rot(8.0)).len(), 1);

        // extrusion wins over the other attachment rules
        let ext = MovementDesc {
            extrusion_mm: 1.6,
            torque_deg: 5.0,
            ..none
        };
        assert_eq!(attachment_rule(fdi(11), &ext), Attachment::Estrusione);
        assert_eq!(movement_risk(fdi(11), &ext).0, 2);
        let torque = MovementDesc {
            torque_deg: -4.0,
            ..none
        };
        assert_eq!(
            attachment_rule(fdi(11), &torque),
            Attachment::ControlloRadicolare
        );
        // lower incisor intrusion
        let intr = MovementDesc {
            extrusion_mm: -0.8,
            ..none
        };
        assert_eq!(movement_flags(fdi(41), &intr).len(), 1);
        assert!(movement_flags(fdi(11), &intr).is_empty());
    }

    #[test]
    fn distalization_beyond_two_millimetres_is_high_risk() {
        let d = |mm| MovementDesc {
            mesial_mm: mm,
            ..MovementDesc::default()
        };
        let (level, flags) = movement_risk(fdi(16), &d(-1.0));
        assert_eq!((level, flags.len()), (1, 1));
        let (level, flags) = movement_risk(fdi(16), &d(-2.5));
        assert_eq!((level, flags.len()), (2, 2));
        assert_eq!(flags[1].refs, vec!["wang2025consensus"]);
        assert_eq!(
            attachment_rule(fdi(26), &d(-1.0)),
            Attachment::CorporeoAncoraggio
        );
        assert_eq!(
            attachment_rule(
                fdi(14),
                &MovementDesc {
                    buccal_mm: 0.8,
                    ..d(0.0)
                }
            ),
            Attachment::CorporeoAncoraggio
        );
        // three notes make a high risk even when none is severe
        let many = MovementDesc {
            rotation_deg: 20.0,
            torque_deg: 4.0,
            buccal_mm: 0.8,
            ..MovementDesc::default()
        };
        let (level, flags) = movement_risk(fdi(14), &many);
        assert_eq!((level, flags.len()), (2, 3));
    }

    #[test]
    fn complexity_levels_and_borderline_scores() {
        assert_eq!(complexity_math(0.0), (0, false));
        assert_eq!(complexity_math(20.0), (0, true));
        assert_eq!(complexity_math(21.0), (1, true));
        assert_eq!(complexity_math(30.0), (1, false));
        assert_eq!(complexity_math(41.0), (2, true));
        assert_eq!(complexity_math(70.0), (3, false));
    }

    #[test]
    fn the_staging_pattern_follows_the_complexity() {
        let base = PatternInput::default();
        assert_eq!(pattern_math(&base), (StagingPattern::Proportional, false));
        let crowded = PatternInput {
            crowding_mm: 3.2,
            ..base.clone()
        };
        assert_eq!(pattern_math(&crowded), (StagingPattern::Structured, true));
        let distal = PatternInput {
            distalization_mm: 0.6,
            crowding_mm: 5.0,
            ..base.clone()
        };
        assert_eq!(
            pattern_math(&distal),
            (StagingPattern::SequentialDistal, true)
        );
        let little = PatternInput {
            little_mm: 6.5,
            ..base.clone()
        };
        assert_eq!(choose_pattern(&little), StagingPattern::Structured);
        let level = PatternInput {
            level_above_easy: true,
            ..base
        };
        assert_eq!(choose_pattern(&level), StagingPattern::Structured);
    }

    #[test]
    fn aligner_suitability_uses_the_consensus_limits() {
        assert_eq!(aligner_suitable(-7.9, 1.9), (true, vec![]));
        let (ok, reasons) = aligner_suitable(-8.5, 2.5);
        assert!(!ok);
        assert_eq!(reasons.len(), 2);
        assert!(reasons[0].contains("soglia di estrazione (Wang"));
        assert!(reasons[1].contains("2.5 mm oltre il limite prevedibile"));
    }
}
