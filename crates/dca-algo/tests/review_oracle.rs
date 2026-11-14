//! The classifications of the clinical review (`dca_algo::review`) compared
//! with the plugin's `oa.classify` without the AI classifier
//! (`ortho_aligner/review.py`, `classify.py`) on the six demo cases of
//! `golden/fixtures` (`golden/oracle/stages.py`, `stage_classify`).
//!
//! The review is fed what the oracle fed the plugin: the CAT-CAT score and
//! the discrepancy of the oracle's diagnosis, the complexity of
//! `case_plan.complexity` without the planned moves, the widths and height
//! of every tooth, its movement in the final position (`describe_q`) and its
//! aligners. Equal: the records in order (kind, subject, computed value,
//! proposal, options, a note to check or not), the calendar of the plan
//! with the attachments of the review (to review, then with two attachment
//! decisions) and the bulk confirmation (how many confirmed, which left).

use std::path::PathBuf;

use dc_case::{Fdi, ScanRole};
use dca_algo::anatomy::Arch;
use dca_algo::diagnosis::classify::PatternInput;
use dca_algo::review::{self, Action, ArchInput, Movement, ToothInput};
use dca_algo::schedule::{self, Lang, Policy};
use dca_algo::staging;
use dca_model::{AlignersState, ReviewKind, ReviewStatus};
use serde_json::Value;

const CASES: [&str; 6] = [
    "demo_lower_mild",
    "demo_lower_moderate",
    "demo_lower_severe",
    "demo_upper_mild",
    "demo_upper_moderate",
    "demo_upper_severe",
];

fn dir(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/fixtures")
        .join(case)
}

fn json(case: &str, rel: &str) -> Value {
    let text = std::fs::read_to_string(dir(case).join(rel)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn fdi(n: u64) -> Fdi {
    Fdi::new(n as u8).unwrap()
}

/// The kind of the review of a plugin kind.
fn kind(tipo: &str) -> ReviewKind {
    match tipo {
        "complessita" => ReviewKind::Complexity,
        "schema_staging" => ReviewKind::StagingPattern,
        "idoneita_allineatori" => ReviewKind::AlignerSuitability,
        "rischio_movimento" => ReviewKind::MovementRisk,
        "attachment" => ReviewKind::Attachment,
        "segmentazione" => ReviewKind::Segmentation,
        "tipo_dente" => ReviewKind::ToothType,
        other => panic!("unknown kind {other}"),
    }
}

/// The value of the review of a plugin value.
fn value(v: &str) -> &'static str {
    match v {
        "FACILE" => "easy",
        "MODERATO" => "moderate",
        "DIFFICILE" => "difficult",
        "MOLTO COMPLESSO" => "very_complex",
        "PROPORTIONAL" => "proportional",
        "STRUCTURED" => "structured",
        "SEQUENTIAL_DISTAL" => "sequential_distal",
        "MAX_VELOCITY" => "max_velocity",
        "si" => "yes",
        "no" => "no",
        "basso" => "low",
        "medio" => "medium",
        "alto" => "high",
        "nessuno" => "none",
        "rotazione" => "rotation",
        "estrusione" => "extrusion",
        "controllo_radicolare" => "root_control",
        "corporeo_ancoraggio" => "bodily_anchorage",
        "plausibile" => "plausible",
        "da verificare" => "check",
        "incisivo centrale" => "central_incisor",
        "incisivo laterale" => "lateral_incisor",
        "canino" => "canine",
        "primo premolare" => "first_premolar",
        "secondo premolare" => "second_premolar",
        "primo molare" => "first_molar",
        "secondo molare" => "second_molar",
        other => panic!("unknown value {other:?}"),
    }
}

/// The review of the case fed the oracle's inputs.
fn input(case: &str, c: &Value) -> ArchInput {
    let i = &c["inputs"];
    let cx = &i["complexity"];
    let teeth = i["teeth"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(f, t)| {
            let d = t["desc"].as_array().unwrap();
            ToothInput {
                fdi: fdi(f.parse().unwrap()),
                width_md: num(&t["width_md"]),
                width_bl: num(&t["width_bl"]),
                height: num(&t["height"]),
                movement: Some(Movement {
                    desc: std::array::from_fn(|k| num(&d[k])),
                    aligners: t["need"].as_u64().map(|n| n as u32),
                }),
            }
        })
        .collect();
    ArchInput {
        arch: if case.contains("upper") {
            Arch::Upper
        } else {
            Arch::Lower
        },
        catcat_score: i["analysis"]["catcat_score"].as_u64().unwrap() as u32,
        catcat_items: Default::default(),
        discrepancy_mm: num(&i["analysis"]["discrepancy"]),
        distal_mm: num(&i["distal"]),
        pattern: PatternInput {
            level_above_easy: cx["level"] != "FACILE",
            crowding_mm: num(&cx["crowding"]),
            little_mm: num(&cx["little"]),
            distalization_mm: num(&cx["distalization"]),
        },
        teeth,
    }
}

/// The calendar (fixed, from 2026-01-05) of the oracle's plan with the
/// attachments of `data`, in Italian as the plugin writes it.
fn calendar(case: &str, data: &AlignersState, role: ScanRole) -> Vec<Vec<String>> {
    let staging = json(case, "staging/staging.json");
    let setup = json(case, "setup/setup.json");
    let plan = &staging["plan"];
    let q = plan["q"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(f, v)| {
            let series: Vec<staging::Q6> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    let a = s.as_array().unwrap();
                    std::array::from_fn(|i| num(&a[i]))
                })
                .collect();
            (fdi(f.parse().unwrap()), series)
        })
        .collect();
    let ipr = plan["ipr_stage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| schedule::IprAt {
            a: fdi(r[0].as_u64().unwrap()),
            b: fdi(r[1].as_u64().unwrap()),
            stage: r[2].as_u64().unwrap() as u32,
            mm: setup["setup"]["ipr"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["a"] == r[0] && x["b"] == r[1])
                .map_or(0.0, |x| num(&x["mm"])),
        })
        .collect();
    let input = schedule::Input {
        n_stages: plan["n_stages"].as_u64().unwrap() as u32,
        n_total: None,
        q,
        ipr,
        attachments: data.attachment_teeth(role),
        offset: 0,
    };
    let cal = schedule::run(
        &input,
        &schedule::Params {
            start_date: Some("2026-01-05".into()),
            interval_days: plan["interval_days"].as_u64().unwrap() as u32,
            policy: Policy::Fixed,
            lang: Lang::It,
            ..schedule::Params::default()
        },
    )
    .unwrap();
    cal.rows.into_iter().map(|r| r.todo).collect()
}

fn todo(v: &Value) -> Vec<Vec<String>> {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn the_classifications_match_the_plugin_on_the_demo_cases() {
    let mut attachments = 0;
    for case in CASES {
        let c = json(case, "classify/classify.json");
        let inp = input(case, &c);
        let role = inp.arch.role();
        let recs = review::build_records(&inp);
        let want = c["records"].as_array().unwrap();
        assert_eq!(recs.len(), want.len(), "{case}: number of records");
        for (r, w) in recs.iter().zip(want) {
            let label = format!("{case} {}", r.key());
            assert_eq!(r.kind, kind(w["tipo"].as_str().unwrap()), "{label}");
            let subject = w["oggetto"].as_str().unwrap();
            if subject == "arcata" {
                assert_eq!(r.fdi, None, "{label}");
            } else {
                assert_eq!(
                    r.fdi.map(|f| f.to_string()).as_deref(),
                    Some(subject),
                    "{label}"
                );
            }
            assert_eq!(r.computed, value(w["calcolo"].as_str().unwrap()), "{label}");
            assert_eq!(
                r.proposed,
                value(w["proposta"].as_str().unwrap()),
                "{label}"
            );
            let options: Vec<&str> = w["opzioni"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| value(o.as_str().unwrap()))
                .collect();
            assert_eq!(r.options, options, "{label}: options");
            let note = !w["nota"].as_str().unwrap().is_empty();
            assert_eq!(r.needs_check(), note, "{label}: note");
            assert_eq!(r.status, ReviewStatus::Pending, "{label}");
        }
        attachments += recs
            .iter()
            .filter(|r| r.kind == ReviewKind::Attachment && r.proposed != "none")
            .count();

        // the calendar with the proposals preset, to review
        let mut data = AlignersState {
            review: recs.clone(),
            ..AlignersState::default()
        };
        assert_eq!(
            calendar(case, &data, role),
            todo(&c["calendar"]["todo"]),
            "{case}: calendar"
        );

        // two attachments decided by the clinician
        for (subject, v) in c["decided"]["values"].as_object().unwrap() {
            let r = data
                .review
                .iter_mut()
                .find(|r| {
                    r.kind == ReviewKind::Attachment
                        && r.fdi.map(|f| f.to_string()).as_deref() == Some(subject.as_str())
                })
                .unwrap();
            review::decide(r, &Action::Value(value(v.as_str().unwrap()).to_owned()), 1);
        }
        assert_eq!(
            calendar(case, &data, role),
            todo(&c["decided"]["todo"]),
            "{case}: calendar with decisions"
        );

        // the bulk confirmation leaves the notes to the clinician
        let mut bulk = recs.clone();
        let n = review::decide_all(&mut bulk, 1);
        assert_eq!(n as u64, c["bulk"]["confirmed"].as_u64().unwrap(), "{case}");
        let left: Vec<String> = bulk
            .iter()
            .filter(|r| r.status == ReviewStatus::Pending)
            .map(|r| {
                let plugin_kind = match r.kind {
                    ReviewKind::ToothType => "tipo_dente",
                    ReviewKind::Segmentation => "segmentazione",
                    other => panic!("{other:?} left to review"),
                };
                format!("{}:{plugin_kind}", r.fdi.unwrap())
            })
            .collect();
        let want_left: Vec<String> = serde_json::from_value(c["bulk"]["left"].clone()).unwrap();
        assert_eq!(left, want_left, "{case}: left to review");
    }
    // the demo cases do call for attachments: the calendar check is not empty
    assert!(attachments > 0);
}
