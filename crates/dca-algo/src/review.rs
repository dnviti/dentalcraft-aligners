//! The clinical review: the classifications the program proposes and the
//! clinician's decisions on them (the plugin's `review.py` with the rules
//! of `classify.py`; the plugin's AI classifier is not ported, so every
//! proposal is the computed value, as the plugin makes it when the
//! classifier is not configured).
//!
//! [`build_records`] makes the records of one arch from what the case
//! knows about it ([`ArchInput`], the plugin's `jev_case_data`):
//!
//! * per arch: the case difficulty from the CAT-CAT score
//!   ([`classify::complexity_math`]), the staging pattern
//!   ([`classify::pattern_math`]) and whether aligners alone can treat it
//!   ([`classify::aligner_suitable`]);
//! * per tooth, in the order of the arch, when the arch has a final
//!   position: the predictability risk of its movement
//!   ([`classify::movement_risk`]) and the attachment it calls for
//!   ([`classify::attachment_rule`]); then for every tooth: whether its
//!   crown measures are plausible ([`classify::segmentation_plausible`]) and
//!   the type its number names, with a note when the diameters look like
//!   another type ([`classify::tooth_type`]).
//!
//! [`store`] keeps the decisions already taken where the computed value
//! did not change (`review.store`); [`decide`], [`decide_all`] and
//! [`counts`] are `review.decide`, `review.decide_all` and `review.counts`.

use dc_case::{Fdi, ScanRole};
use dca_model::{AlignersState, ReviewKind, ReviewRecord, ReviewStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::anatomy::{self, Arch};
use crate::diagnosis::classify::{self, Attachment, MovementDesc, PatternInput};
use crate::diagnosis::{CatcatItems, edit};
use crate::staging::{self, Pattern};

/// The difficulty levels (`classify.LEVELS`), by consensus level.
pub const LEVELS: [&str; 4] = ["easy", "moderate", "difficult", "very_complex"];
/// The staging patterns, in the order of the plugin's `PATTERNS`.
pub const PATTERNS: [&str; 4] = [
    "proportional",
    "structured",
    "sequential_distal",
    "max_velocity",
];
/// The answers on aligners alone.
pub const SUITABILITY: [&str; 2] = ["yes", "no"];
/// The risk levels (`classify.RISK`).
pub const RISK: [&str; 3] = ["low", "medium", "high"];
/// The answers on the crown measures.
pub const SEGMENTATION: [&str; 2] = ["plausible", "check"];
/// The tooth types, by position 1..=7 (`classify.TOOTH_TYPES`).
pub const TOOTH_TYPES: [&str; 7] = [
    "central_incisor",
    "lateral_incisor",
    "canine",
    "first_premolar",
    "second_premolar",
    "first_molar",
    "second_molar",
];
/// The note of a tooth whose diameters look like another type: its
/// number has to be checked.
pub const NOTE_CHECK_NUMBERING: &str = "check_numbering";

/// The planned movement of one tooth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    /// The movement of the final position in the clinical frame of the
    /// tooth ([`staging::describe_q`]): mesial, buccal, extrusion (mm),
    /// torque, tip, rotation (degrees).
    pub desc: [f64; 6],
    /// Aligners in which the tooth moves (`None` before the aligners are
    /// planned).
    pub aligners: Option<u32>,
}

impl Movement {
    fn classify_desc(&self) -> MovementDesc {
        let d = &self.desc;
        MovementDesc {
            mesial_mm: d[0],
            buccal_mm: d[1],
            extrusion_mm: d[2],
            torque_deg: d[3],
            rotation_deg: d[5],
        }
    }
}

/// What the review knows of one tooth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToothInput {
    /// The tooth.
    pub fdi: Fdi,
    /// Mesio-distal diameter (mm).
    pub width_md: f64,
    /// Bucco-lingual diameter (mm).
    pub width_bl: f64,
    /// Clinical crown height (mm).
    pub height: f64,
    /// The planned movement; `None` when the arch has no final position.
    pub movement: Option<Movement>,
}

/// What the review knows of one arch (the plugin's `jev_case_data`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArchInput {
    /// The arch.
    pub arch: Arch,
    /// CAT-CAT score of the arch with the values in force.
    pub catcat_score: u32,
    /// Its items.
    pub catcat_items: CatcatItems,
    /// Space available minus required (mm, negative is crowding).
    pub discrepancy_mm: f64,
    /// Molar distalization planned by the final position (mm).
    pub distal_mm: f64,
    /// What the staging pattern is chosen from, without the planned molar
    /// moves (the plugin's `case_plan.complexity(obj)`).
    pub pattern: PatternInput,
    /// The teeth, in any order.
    pub teeth: Vec<ToothInput>,
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

fn record(
    arch: Arch,
    kind: ReviewKind,
    fdi: Option<Fdi>,
    computed: &str,
    options: Vec<String>,
    detail: Value,
    refs: &[&str],
) -> ReviewRecord {
    ReviewRecord {
        arch: arch.role(),
        kind,
        fdi,
        computed: computed.to_owned(),
        proposed: computed.to_owned(),
        options,
        detail,
        refs: strings(refs),
        note: None,
        status: ReviewStatus::Pending,
        decided: computed.to_owned(),
        decided_at: None,
    }
}

/// The id of a staging pattern of the review.
pub fn pattern_id(p: Pattern) -> &'static str {
    match p {
        Pattern::Proportional => "proportional",
        Pattern::Structured => "structured",
        Pattern::SequentialDistal => "sequential_distal",
        Pattern::MaxVelocity => "max_velocity",
        Pattern::Auto => "auto",
    }
}

/// The pattern of a review id ([`pattern_id`]).
pub fn pattern_of(id: &str) -> Option<Pattern> {
    [
        Pattern::Proportional,
        Pattern::Structured,
        Pattern::SequentialDistal,
        Pattern::MaxVelocity,
    ]
    .into_iter()
    .find(|p| pattern_id(*p) == id)
}

fn items_json(items: &CatcatItems) -> Value {
    json!({
        "crowding": items.crowding,
        "rotations": items.rotations,
        "molar_movement": items.molar_movement,
        "midline": items.midline,
        "diastema": items.diastema,
    })
}

/// The records of one arch, each with its proposal preset as the decision
/// and to review (`review.build_records` without the classifier).
pub fn build_records(input: &ArchInput) -> Vec<ReviewRecord> {
    let arch = input.arch;
    let mut out = Vec::new();

    // complexity (score)
    let (level, _) = classify::complexity_math(f64::from(input.catcat_score));
    out.push(record(
        arch,
        ReviewKind::Complexity,
        None,
        LEVELS[level],
        strings(&LEVELS),
        json!({ "score": input.catcat_score, "items": items_json(&input.catcat_items) }),
        &["long2020catcat", "wang2025consensus"],
    ));

    // staging pattern (choice)
    let (pattern, _) = classify::pattern_math(&input.pattern);
    let cx = &input.pattern;
    out.push(record(
        arch,
        ReviewKind::StagingPattern,
        None,
        pattern_id(pattern.into()),
        strings(&PATTERNS),
        json!({
            "distalization_mm": cx.distalization_mm,
            "crowding_mm": cx.crowding_mm,
            "little_mm": cx.little_mm,
            "level": LEVELS[level],
        }),
        &["martinezlozano2024", "ravera2016"],
    ));

    // aligner-only suitability
    let problems = classify::suitability_problems(input.discrepancy_mm, input.distal_mm);
    out.push(record(
        arch,
        ReviewKind::AlignerSuitability,
        None,
        if problems.is_empty() { "yes" } else { "no" },
        strings(&SUITABILITY),
        json!({
            "discrepancy_mm": input.discrepancy_mm,
            "distal_mm": input.distal_mm,
            "problems": problems,
        }),
        &["wang2025consensus"],
    ));

    // per tooth, in the order of the arch
    let order = anatomy::arch_order(arch);
    let mut teeth: Vec<&ToothInput> = input.teeth.iter().collect();
    teeth.sort_by_key(|t| order.iter().position(|f| *f == t.fdi).unwrap_or(usize::MAX));
    for t in teeth {
        let f = t.fdi;
        if let Some(mv) = &t.movement {
            let desc = mv.classify_desc();
            let (risk, flags) = classify::movement_risk(f, &desc);
            let mut refs: Vec<&str> = Vec::new();
            for r in flags.iter().flat_map(|x| x.refs.iter()) {
                if !refs.contains(r) {
                    refs.push(r);
                }
            }
            if refs.is_empty() {
                refs.push("haouili2020");
            }
            out.push(record(
                arch,
                ReviewKind::MovementRisk,
                Some(f),
                RISK[usize::from(risk)],
                strings(&RISK),
                json!({
                    "movement": mv.desc,
                    "aligners": mv.aligners,
                    "flags": flags.iter().map(|x| x.id).collect::<Vec<_>>(),
                }),
                &refs,
            ));
            let att = classify::attachment_rule(f, &desc);
            out.push(record(
                arch,
                ReviewKind::Attachment,
                Some(f),
                att.id(),
                Attachment::ALL.iter().map(|a| a.id().to_owned()).collect(),
                json!({}),
                &["wang2025consensus", "elshazly2024", "nucera2022"],
            ));
        }
        let problems = classify::segmentation_problems(f, t.width_md, t.width_bl, t.height);
        let (md, bl) = anatomy::typical_diameters(f);
        out.push(record(
            arch,
            ReviewKind::Segmentation,
            Some(f),
            if problems.is_empty() {
                "plausible"
            } else {
                "check"
            },
            strings(&SEGMENTATION),
            json!({
                "width_md": t.width_md,
                "width_bl": t.width_bl,
                "height": t.height,
                "typical_md": md,
                "typical_bl": bl,
                "problems": problems,
            }),
            &["wheeler"],
        ));
        let tt = classify::tooth_type(f, t.width_md, t.width_bl);
        let nearest = classify::TOOTH_TYPES
            .iter()
            .position(|n| *n == tt.best)
            .unwrap_or(0);
        let assigned = usize::from(anatomy::index(f).clamp(1, 7)) - 1;
        let mut r = record(
            arch,
            ReviewKind::ToothType,
            Some(f),
            TOOTH_TYPES[assigned],
            strings(&TOOTH_TYPES),
            json!({
                "nearest": TOOTH_TYPES[nearest],
                "distance_nearest": tt.scores[nearest],
                "distance_assigned": tt.scores[assigned],
            }),
            &["wheeler"],
        );
        if nearest != assigned && tt.scores[nearest] + 1.0 < tt.scores[assigned] {
            r.note = Some(NOTE_CHECK_NUMBERING.to_owned());
        }
        out.push(r);
    }
    out
}

/// The records of the case after a new classification of the arches of
/// `new`: the records of those arches are replaced, those of the others
/// kept. A decision already taken stays where the record has the same key
/// and the same computed value (`review.store`); every other record starts
/// with its proposal preset as the decision, to review.
pub fn store(old: &[ReviewRecord], new: Vec<ReviewRecord>) -> Vec<ReviewRecord> {
    let arches: Vec<ScanRole> = new.iter().map(|r| r.arch).collect();
    let mut out: Vec<ReviewRecord> = old
        .iter()
        .filter(|r| !arches.contains(&r.arch))
        .cloned()
        .collect();
    for mut r in new {
        let key = r.key();
        let prev = old.iter().find(|p| p.key() == key);
        match prev {
            Some(p) if p.computed == r.computed && p.status != ReviewStatus::Pending => {
                r.status = p.status;
                r.decided = p.decided.clone();
                r.decided_at = p.decided_at;
            }
            _ => {
                r.status = ReviewStatus::Pending;
                r.decided = r.proposed.clone();
                r.decided_at = None;
            }
        }
        out.push(r);
    }
    // the arches in the order of the case: lower, then upper, as the menu
    out.sort_by_key(|r| u8::from(r.arch == ScanRole::Upper));
    out
}

/// What the clinician does with a record (`review.decide`; the plugin's
/// `JEV` action belongs to the classifier and is not ported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Confirm the proposal (`PROPOSED`).
    Confirm,
    /// Keep the computed value (`MATH`).
    Computed,
    /// Choose this value, one of the record's options (`VALUE`).
    Value(String),
    /// Back to the proposal, to review (`RESET`).
    Reset,
}

/// Records the decision on `rec` at `now` (Unix seconds).
pub fn decide(rec: &mut ReviewRecord, action: &Action, now: u64) {
    match action {
        Action::Reset => {
            rec.status = ReviewStatus::Pending;
            rec.decided = rec.proposed.clone();
            rec.decided_at = None;
            return;
        }
        Action::Confirm => {
            rec.decided = rec.proposed.clone();
            rec.status = ReviewStatus::Accepted;
        }
        Action::Computed => {
            rec.decided = rec.computed.clone();
            rec.status = if rec.proposed == rec.computed {
                ReviewStatus::Accepted
            } else {
                ReviewStatus::Rejected
            };
        }
        Action::Value(v) => {
            rec.decided = v.clone();
            rec.status = if *v == rec.proposed {
                ReviewStatus::Accepted
            } else {
                ReviewStatus::Modified
            };
        }
    }
    rec.decided_at = Some(now);
}

/// Confirms the proposals still to review, leaving out those with a note
/// asking for a check (`review.decide_all(skip_flagged=True)`); the number
/// confirmed.
pub fn decide_all(records: &mut [ReviewRecord], now: u64) -> usize {
    let mut n = 0;
    for r in records
        .iter_mut()
        .filter(|r| r.status == ReviewStatus::Pending && !r.needs_check())
    {
        decide(r, &Action::Confirm, now);
        n += 1;
    }
    n
}

/// The records by decision and those with a note (`review.counts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Counts {
    /// Proposals still to review.
    pub pending: usize,
    /// Confirmed.
    pub accepted: usize,
    /// Computed value kept.
    pub rejected: usize,
    /// Another value chosen.
    pub modified: usize,
    /// With a note asking for a check.
    pub notes: usize,
}

impl Counts {
    /// Decided by the clinician in any way.
    pub fn decided(&self) -> usize {
        self.accepted + self.rejected + self.modified
    }
}

/// [`Counts`] of `records`.
pub fn counts<'a>(records: impl IntoIterator<Item = &'a ReviewRecord>) -> Counts {
    let mut c = Counts::default();
    for r in records {
        match r.status {
            ReviewStatus::Pending => c.pending += 1,
            ReviewStatus::Accepted => c.accepted += 1,
            ReviewStatus::Rejected => c.rejected += 1,
            ReviewStatus::Modified => c.modified += 1,
        }
        if r.needs_check() {
            c.notes += 1;
        }
    }
    c
}

/// The pattern the review puts in place of the automatic choice `auto` for
/// the arch (the plugin's `case_plan.complexity`, `pattern_override`): the
/// value in force of its staging pattern record, when it differs from
/// `auto`. A pending proposal counts, as in the plugin.
pub fn pattern_override(data: &AlignersState, role: ScanRole, auto: Pattern) -> Option<Pattern> {
    data.review_value(role, ReviewKind::StagingPattern, None)
        .and_then(pattern_of)
        .filter(|p| *p != auto)
}

/// The aligners in which tooth `f` moves in `stages` (its pose differs from
/// the one before it, the rest pose before the first).
fn moving_aligners(data: &AlignersState, f: Fdi) -> u32 {
    let mut stages: Vec<&dca_model::Stage> = data.stages.iter().collect();
    stages.sort_by_key(|s| s.index);
    let mut before = dc_case::Pose::IDENTITY;
    let mut n = 0;
    for s in stages {
        if let Some(p) = s.poses.get(&f) {
            if !p.approx_eq(&before, 1e-9, 1e-9) {
                n += 1;
            }
            before = *p;
        }
    }
    n
}

/// What the review is made from for the arch `arch` of the case, `None`
/// before its diagnosis or without measured teeth: the CAT-CAT score and
/// the space with the values in force, the teeth measured (not those marked
/// missing) and, when the arch has a final position, the movement of each
/// tooth and the aligners in which it moves.
pub fn arch_input(data: &AlignersState, arch: Arch) -> Option<ArchInput> {
    let role = arch.role();
    let eff = edit::effective(data, role)?;
    let score = edit::effective_score(data, role).unwrap_or(eff.score);
    let discrepancy = data.diagnosis.discrepancy_mm(role)?;
    let teeth: Vec<Fdi> = data
        .tooth_geometry
        .keys()
        .copied()
        .filter(|f| Arch::of(*f) == arch && !data.missing.contains(f))
        .collect();
    if teeth.is_empty() {
        return None;
    }
    let has_setup = data.setup.results.contains_key(&role)
        || data.setup.poses.keys().any(|f| Arch::of(*f) == arch);
    let staged = crate::schedule::active_stages(arch, &data.stages) > 0;
    let teeth = teeth
        .into_iter()
        .map(|f| {
            let g = &data.tooth_geometry[&f];
            let movement = has_setup.then(|| {
                let pose = data
                    .setup
                    .poses
                    .get(&f)
                    .copied()
                    .unwrap_or(dc_case::Pose::IDENTITY);
                Movement {
                    desc: staging::describe_q(&staging::q_from_pose(&pose), g.md_sign),
                    aligners: staged.then(|| moving_aligners(data, f)),
                }
            });
            ToothInput {
                fdi: f,
                width_md: g.width_md,
                width_bl: g.width_bl,
                height: g.height,
                movement,
            }
        })
        .collect();
    Some(ArchInput {
        arch,
        catcat_score: score,
        catcat_items: eff.items,
        discrepancy_mm: discrepancy,
        // the final position of the program never plans a distalization
        // (`setup_json["distal"]` is always empty)
        distal_mm: 0.0,
        pattern: PatternInput {
            level_above_easy: score > 20,
            crowding_mm: (-discrepancy).max(0.0),
            little_mm: data.diagnosis.little_index_mm(role).unwrap_or(0.0),
            distalization_mm: 0.0,
        },
        teeth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    fn tooth(n: u8, desc: Option<[f64; 6]>) -> ToothInput {
        let (md, bl) = anatomy::typical_diameters(fdi(n));
        ToothInput {
            fdi: fdi(n),
            width_md: md,
            width_bl: bl,
            height: 8.0,
            movement: desc.map(|desc| Movement {
                desc,
                aligners: Some(10),
            }),
        }
    }

    fn input(teeth: Vec<ToothInput>) -> ArchInput {
        ArchInput {
            arch: Arch::Lower,
            catcat_score: 22,
            catcat_items: CatcatItems::default(),
            discrepancy_mm: -4.0,
            distal_mm: 0.0,
            pattern: PatternInput {
                level_above_easy: true,
                crowding_mm: 4.0,
                little_mm: 5.0,
                distalization_mm: 0.0,
            },
            teeth,
        }
    }

    fn find(recs: &[ReviewRecord], kind: ReviewKind, f: Option<u8>) -> &ReviewRecord {
        recs.iter()
            .find(|r| r.kind == kind && r.fdi.map(Fdi::get) == f)
            .unwrap()
    }

    #[test]
    fn the_records_follow_the_rules_of_the_plugin_in_its_order() {
        // 33 extruded 1 mm (extrusion attachment, medium risk), 41 untouched
        let recs = build_records(&input(vec![
            tooth(41, Some([0.0; 6])),
            tooth(33, Some([0.0, 0.0, 1.0, 0.0, 0.0, 0.0])),
        ]));
        let kinds: Vec<(ReviewKind, Option<u8>)> =
            recs.iter().map(|r| (r.kind, r.fdi.map(Fdi::get))).collect();
        use ReviewKind::*;
        assert_eq!(
            kinds,
            [
                (Complexity, None),
                (StagingPattern, None),
                (AlignerSuitability, None),
                // the arch order: 43 ... 41, 31 ... 33
                (MovementRisk, Some(41)),
                (Attachment, Some(41)),
                (Segmentation, Some(41)),
                (ToothType, Some(41)),
                (MovementRisk, Some(33)),
                (Attachment, Some(33)),
                (Segmentation, Some(33)),
                (ToothType, Some(33)),
            ]
        );
        assert_eq!(find(&recs, Complexity, None).proposed, "moderate");
        assert_eq!(find(&recs, StagingPattern, None).proposed, "structured");
        assert_eq!(find(&recs, AlignerSuitability, None).proposed, "yes");
        assert_eq!(find(&recs, MovementRisk, Some(33)).proposed, "medium");
        assert_eq!(find(&recs, MovementRisk, Some(41)).proposed, "low");
        assert_eq!(find(&recs, Attachment, Some(33)).proposed, "extrusion");
        assert_eq!(find(&recs, Attachment, Some(41)).proposed, "none");
        assert_eq!(find(&recs, Segmentation, Some(33)).proposed, "plausible");
        assert_eq!(find(&recs, ToothType, Some(33)).proposed, "canine");
        // every proposal is the computed value, preset and to review
        for r in &recs {
            assert_eq!(r.proposed, r.computed);
            assert_eq!(r.decided, r.proposed);
            assert_eq!(r.status, ReviewStatus::Pending);
            assert!(r.options.contains(&r.proposed), "{}", r.key());
            assert!(!r.refs.is_empty());
        }
        // no final position: no movement records
        let recs = build_records(&input(vec![tooth(33, None)]));
        assert_eq!(recs.len(), 5);
    }

    #[test]
    fn a_crown_like_another_type_asks_to_check_its_number() {
        // a lower canine with the diameters of a first molar
        let (md, bl) = anatomy::typical_diameters(fdi(36));
        let t = ToothInput {
            fdi: fdi(33),
            width_md: md,
            width_bl: bl,
            height: 7.0,
            movement: None,
        };
        let recs = build_records(&input(vec![t]));
        let r = find(&recs, ReviewKind::ToothType, Some(33));
        assert_eq!(r.proposed, "canine");
        assert_eq!(r.detail["nearest"], "first_molar");
        assert_eq!(r.note.as_deref(), Some(NOTE_CHECK_NUMBERING));
        assert!(r.needs_check());
        // and its measures are out of range for a canine
        let s = find(&recs, ReviewKind::Segmentation, Some(33));
        assert_eq!(s.proposed, "check");
        assert_eq!(s.detail["problems"][0], "md");
    }

    #[test]
    fn decisions_follow_the_plugin() {
        let mut recs = build_records(&input(vec![tooth(33, Some([0.0; 6]))]));
        let r = &mut recs[0];
        decide(r, &Action::Value("difficult".to_owned()), 100);
        assert_eq!(
            (r.status, r.decided.as_str(), r.decided_at),
            (ReviewStatus::Modified, "difficult", Some(100))
        );
        assert_eq!(r.in_force(), "difficult");
        decide(r, &Action::Value("moderate".to_owned()), 101);
        assert_eq!(r.status, ReviewStatus::Accepted);
        decide(r, &Action::Reset, 102);
        assert_eq!(
            (r.status, r.decided.as_str(), r.decided_at),
            (ReviewStatus::Pending, "moderate", None)
        );
        decide(r, &Action::Confirm, 103);
        assert_eq!(r.status, ReviewStatus::Accepted);
        // the computed value kept over another proposal
        r.proposed = "difficult".to_owned();
        decide(r, &Action::Computed, 104);
        assert_eq!(
            (r.status, r.decided.as_str()),
            (ReviewStatus::Rejected, "moderate")
        );
    }

    #[test]
    fn the_bulk_confirmation_leaves_the_notes_to_the_clinician() {
        let (md, bl) = anatomy::typical_diameters(fdi(36));
        let odd = ToothInput {
            fdi: fdi(33),
            width_md: md,
            width_bl: bl,
            height: 7.0,
            movement: None,
        };
        let mut recs = build_records(&input(vec![odd, tooth(41, None)]));
        decide(&mut recs[0], &Action::Value("easy".to_owned()), 5);
        let before = counts(&recs);
        assert_eq!(before.modified, 1);
        assert_eq!(before.notes, 1);
        let n = decide_all(&mut recs, 9);
        assert_eq!(n, recs.len() - 2);
        let after = counts(&recs);
        assert_eq!(after.pending, 1);
        assert_eq!(after.modified, 1);
        assert_eq!(after.accepted, n);
        // the decision taken before is untouched
        assert_eq!(recs[0].decided, "easy");
        assert_eq!(recs[0].decided_at, Some(5));
    }

    #[test]
    fn a_new_classification_keeps_the_decisions_whose_value_did_not_change() {
        let mut old = build_records(&input(vec![tooth(33, Some([0.0; 6]))]));
        let mut other = old.clone();
        for r in &mut other {
            r.arch = ScanRole::Upper;
        }
        old.extend(other.clone());
        decide(&mut old[0], &Action::Value("easy".to_owned()), 1);
        decide(&mut old[1], &Action::Confirm, 2);
        // the arch again, its difficulty now computed as difficult
        let mut again = input(vec![tooth(33, Some([0.0; 6]))]);
        again.catcat_score = 45;
        let stored = store(&old, build_records(&again));
        assert_eq!(stored.len(), old.len());
        let lower: Vec<&ReviewRecord> = stored
            .iter()
            .filter(|r| r.arch == ScanRole::Lower)
            .collect();
        // the value changed: back to review with the new proposal
        assert_eq!(lower[0].status, ReviewStatus::Pending);
        assert_eq!(lower[0].decided, "difficult");
        // same value: the decision stays
        assert_eq!(lower[1].status, ReviewStatus::Accepted);
        assert_eq!(lower[1].decided_at, Some(2));
        // the other arch is untouched, lower first
        assert_eq!(stored[0].arch, ScanRole::Lower);
        assert_eq!(
            stored
                .iter()
                .filter(|r| r.arch == ScanRole::Upper)
                .cloned()
                .collect::<Vec<_>>(),
            other
        );
    }

    #[test]
    fn the_staging_pattern_in_force_overrides_the_automatic_choice() {
        let mut data = AlignersState::default();
        assert_eq!(
            pattern_override(&data, ScanRole::Lower, Pattern::Structured),
            None
        );
        data.review = build_records(&input(vec![tooth(33, None)]));
        // the proposal (structured) is the automatic choice: nothing to override
        assert_eq!(
            pattern_override(&data, ScanRole::Lower, Pattern::Structured),
            None
        );
        // a pending proposal counts against another automatic choice
        assert_eq!(
            pattern_override(&data, ScanRole::Lower, Pattern::SequentialDistal),
            Some(Pattern::Structured)
        );
        decide(
            &mut data.review[1],
            &Action::Value("proportional".to_owned()),
            1,
        );
        assert_eq!(
            pattern_override(&data, ScanRole::Lower, Pattern::Structured),
            Some(Pattern::Proportional)
        );
        assert_eq!(
            pattern_override(&data, ScanRole::Upper, Pattern::Structured),
            None
        );
        for id in PATTERNS {
            assert_eq!(pattern_id(pattern_of(id).unwrap()), id);
        }
    }

    #[test]
    fn the_attachments_in_force_are_the_teeth_of_the_calendar() {
        let mut data = AlignersState {
            review: build_records(&input(vec![
                tooth(41, Some([0.0; 6])),
                tooth(33, Some([0.0, 0.0, 1.0, 0.0, 0.0, 0.0])),
                tooth(32, Some([0.0, 0.0, 0.0, 5.0, 0.0, 0.0])),
            ])),
            ..AlignersState::default()
        };
        assert_eq!(data.attachment_teeth(ScanRole::Lower), ["32", "33"]);
        let i = data
            .review
            .iter()
            .position(|r| r.kind == ReviewKind::Attachment && r.fdi == Some(fdi(41)))
            .unwrap();
        decide(
            &mut data.review[i],
            &Action::Value("rotation".to_owned()),
            1,
        );
        let i = data
            .review
            .iter()
            .position(|r| r.kind == ReviewKind::Attachment && r.fdi == Some(fdi(33)))
            .unwrap();
        decide(&mut data.review[i], &Action::Value("none".to_owned()), 1);
        assert_eq!(data.attachment_teeth(ScanRole::Lower), ["41", "32"]);
        assert!(data.attachment_teeth(ScanRole::Upper).is_empty());
    }
}
