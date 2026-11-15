//! Numerical comparison of `dca_algo::diagnosis` with the plugin
//! (`case_plan.analyze`, golden fixtures `diagnosis/diagnosis.json`).
//!
//! Each case is fed the oracle's own inputs: the tooth measures of
//! `teeth/teeth.json` and, for the largest space, the full scan moved to
//! arch coordinates with the labels of `segmentation/segmentation.json`.
//! Bound (PLAN section 5): relative error < 1e-3 on the values; the
//! classification labels must be equal.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::diagnosis::{self, ArchInput, Input, Params};
use dca_model::ToothMeasures;
use glam::{DMat4, DVec3};
use serde_json::Value;

const TOL: f64 = 1e-3;

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

fn json(case: &str, file: &str) -> Value {
    let bytes = std::fs::read(dir(case).join(file)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> DVec3 {
    DVec3::new(num(&v[0]), num(&v[1]), num(&v[2]))
}

fn fdi(key: &str) -> Fdi {
    Fdi::new(key.parse().unwrap()).unwrap()
}

fn measures(t: &Value) -> ToothMeasures {
    ToothMeasures {
        centroid: vec3(&t["centroid"]),
        reference: vec3(&t["ref"]),
        width_md: num(&t["width_md"]),
        width_bl: num(&t["width_bl"]),
        height: num(&t["height"]),
        tip: vec3(&t["tip"]),
        cej: vec3(&t["cej"]),
        crest: vec3(&t["crest"]),
        apex: vec3(&t["apex"]),
        contact_m: vec3(&t["contact_m"]),
        contact_d: vec3(&t["contact_d"]),
        rot_dev_deg: num(&t["rot_dev"]),
        axis_tilt_deg: num(&t["axis_tilt"]),
        md_sign: t["md_sign"].as_i64().unwrap() as i8,
    }
}

/// Crown vertices per tooth in arch coordinates, from the full scan.
fn crowns(case: &str) -> BTreeMap<Fdi, Vec<DVec3>> {
    let mesh = dc_mesh::dcm::decode(&std::fs::read(dir(case).join("scan.dcm")).unwrap()).unwrap();
    let frame = json(case, "worldframe/worldframe.json");
    let rows = &frame["arch_frame"]["M_inv"];
    let col = |c: usize| {
        glam::DVec4::new(
            num(&rows[0][c]),
            num(&rows[1][c]),
            num(&rows[2][c]),
            num(&rows[3][c]),
        )
    };
    let to_arch = DMat4::from_cols(col(0), col(1), col(2), col(3));
    let seg = json(case, "segmentation/segmentation.json");
    let rle = seg["vertex_labels"]["rle"].as_array().unwrap();
    let mut labels = Vec::new();
    for pair in rle.chunks(2) {
        labels.extend(std::iter::repeat_n(
            pair[0].as_u64().unwrap() as u16,
            pair[1].as_u64().unwrap() as usize,
        ));
    }
    assert_eq!(labels.len(), mesh.positions.len());
    let mut out: BTreeMap<Fdi, Vec<DVec3>> = BTreeMap::new();
    for (p, l) in mesh.positions.iter().zip(&labels) {
        if *l != 0 {
            out.entry(Fdi::new(*l as u8).unwrap())
                .or_default()
                .push(to_arch.transform_point3(p.as_dvec3()));
        }
    }
    out
}

/// Relative error with an absolute floor for values that are zero.
fn rel(got: f64, want: f64) -> f64 {
    (got - want).abs() / want.abs().max(1e-9)
}

struct Check {
    case: String,
    worst: f64,
}

impl Check {
    fn value(&mut self, name: &str, got: f64, want: f64) {
        let err = rel(got, want);
        assert!(
            err < TOL,
            "{}: {name}: got {got}, oracle {want} (relative error {err:.2e})",
            self.case
        );
        self.worst = self.worst.max(err);
    }
}

#[test]
fn the_diagnosis_matches_the_plugin_on_the_demo_cases() {
    let mut worst_all = 0.0f64;
    for case in CASES {
        let teeth_json = json(case, "teeth/teeth.json");
        let want = &json(case, "diagnosis/diagnosis.json")["analysis"];
        let arch = if case.contains("upper") {
            Arch::Upper
        } else {
            Arch::Lower
        };

        let teeth: BTreeMap<Fdi, ToothMeasures> = teeth_json["teeth"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, t)| (fdi(k), measures(t)))
            .collect();
        let crowns = crowns(case);
        let out = diagnosis::run(
            &Input {
                arches: vec![ArchInput {
                    arch,
                    teeth: &teeth,
                    crowns: Some(&crowns),
                }],
            },
            &Params::default(),
        )
        .unwrap();
        let got = &out.arches[&arch];
        let mut c = Check {
            case: case.to_owned(),
            worst: 0.0,
        };

        // classification labels are equal
        assert_eq!(
            got.little_category.label(),
            want["little_category"].as_str().unwrap(),
            "{case}"
        );
        assert_eq!(got.level.label(), want["level"].as_str().unwrap(), "{case}");
        assert_eq!(
            f64::from(got.catcat_score),
            num(&want["catcat_score"]),
            "{case}"
        );
        for (k, v) in want["catcat_items"].as_object().unwrap() {
            let items = serde_json::to_value(got.catcat_items).unwrap();
            assert_eq!(items[k], *v, "{case}: {k}");
        }
        let seq: Vec<u8> = teeth_json["sequence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(
            got.sequence.iter().map(|f| f.get()).collect::<Vec<_>>(),
            seq,
            "{case}"
        );

        // values
        c.value(
            "available",
            got.space.available,
            num(&want["space"]["available"]),
        );
        c.value(
            "required",
            got.space.required,
            num(&want["space"]["required"]),
        );
        c.value(
            "discrepancy",
            got.space.discrepancy,
            num(&want["space"]["discrepancy"]),
        );
        c.value("little", got.little, num(&want["little"]));
        let steps = want["little_details"].as_array().unwrap();
        assert_eq!(got.little_details.len(), steps.len(), "{case}");
        for (s, w) in got.little_details.iter().zip(steps) {
            assert_eq!(
                (u64::from(s.a.get()), u64::from(s.b.get())),
                (w[0].as_u64().unwrap(), w[1].as_u64().unwrap())
            );
            c.value("little step", s.mm, num(&w[2]));
        }
        c.value(
            "midline_offset",
            got.midline_offset,
            num(&want["midline_offset"]),
        );
        c.value(
            "largest_space",
            got.largest_space,
            num(&want["largest_space"]),
        );
        assert_eq!(
            got.rotations.len(),
            want["rotations"].as_object().unwrap().len(),
            "{case}"
        );
        for (k, w) in want["rotations"].as_object().unwrap() {
            c.value(&format!("rotation {k}"), got.rotations[&fdi(k)], num(w));
        }
        for (k, w) in want["widths"].as_object().unwrap() {
            c.value(&format!("width {k}"), got.widths[&fdi(k)], num(w));
        }
        println!(
            "{case}: worst relative error {:.2e}, level {}, little {:.3}, largest space {:.4}",
            c.worst,
            got.level.label(),
            got.little,
            got.largest_space
        );
        worst_all = worst_all.max(c.worst);
    }
    println!("worst relative error over the six cases: {worst_all:.2e}");
}

/// The diagnosis of a demo case, as computed from the oracle's own inputs.
fn diagnose(case: &str) -> (Arch, diagnosis::ArchDiagnosis) {
    let arch = if case.contains("upper") {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let teeth: BTreeMap<Fdi, ToothMeasures> = json(case, "teeth/teeth.json")["teeth"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, t)| (fdi(k), measures(t)))
        .collect();
    let crowns = crowns(case);
    let mut out = diagnosis::run(
        &Input {
            arches: vec![ArchInput {
                arch,
                teeth: &teeth,
                crowns: Some(&crowns),
            }],
        },
        &Params::default(),
    )
    .unwrap();
    (arch, out.arches.remove(&arch).unwrap())
}

/// `manual.diagnosis_override` of the plugin (`oa.diagnosis_edit`), run for
/// real by `golden/oracle.py` (`edits` of `diagnosis.json`): the values the
/// clinician writes, one call after the other on the same case, give the
/// plugin's discrepancy, Little category, CAT-CAT items, score, level and
/// list of values by hand. A value equal to the automatic one is a reset
/// here (the plugin has no other way back).
#[test]
fn hand_values_give_the_plugins_complexity_on_the_demo_cases() {
    use dca_algo::diagnosis::edit::{self, EditField, Values};
    use dca_model::ArchEdits;

    let mut steps_checked = 0;
    for case in CASES {
        let (_, got) = diagnose(case);
        let computed = Values {
            discrepancy: got.space.discrepancy,
            little: got.little,
            midline_offset: got.midline_offset,
            largest_space: got.largest_space,
        };
        let want = json(case, "diagnosis/diagnosis.json");
        let scenarios = want["edits"].as_object().unwrap();
        assert!(scenarios.len() >= 4, "{case}");
        for (name, steps) in scenarios {
            let mut edits = ArchEdits::default();
            for (k, step) in steps.as_array().unwrap().iter().enumerate() {
                let what = format!("{case}/{name}/{k}");
                let reset: Vec<&str> = step["reset"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                for (key, v) in step["values"].as_object().unwrap() {
                    let field = EditField::from_key(key).unwrap();
                    assert!(field.accepts(num(v)), "{what}: {key}");
                    let value = (!reset.contains(&key.as_str())).then(|| num(v));
                    edit::set_value(&mut edits, &computed, field, value);
                }
                let e = edit::with_edits(&computed, &got.rotations, &edits);
                let r = &step["result"];
                let mut c = Check {
                    case: what.clone(),
                    worst: 0.0,
                };
                // the values in force: the ones written, the rest computed
                c.value("little", e.values.little, num(&r["little"]));
                c.value(
                    "midline",
                    e.values.midline_offset,
                    num(&r["midline_offset"]),
                );
                c.value(
                    "largest space",
                    e.values.largest_space,
                    num(&r["largest_space"]),
                );
                c.value("discrepancy", e.values.discrepancy, num(&r["discrepancy"]));
                // what follows from them: equal to the unit
                assert_eq!(
                    e.little_category.label(),
                    r["little_category"].as_str().unwrap(),
                    "{what}"
                );
                assert_eq!(e.level.label(), r["level"].as_str().unwrap(), "{what}");
                assert_eq!(f64::from(e.score), num(&r["catcat_score"]), "{what}");
                let items = serde_json::to_value(e.items).unwrap();
                assert_eq!(items, r["catcat_items"], "{what}");
                // the values by hand, sorted as the plugin lists them
                let mut by_hand: Vec<&str> = EditField::ALL
                    .into_iter()
                    .filter(|f| f.get(&edits).is_some())
                    .map(EditField::key)
                    .collect();
                by_hand.sort_unstable();
                let want_hand: Vec<&str> = r["manuale"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                assert_eq!(by_hand, want_hand, "{what}");
                steps_checked += 1;
            }
        }
    }
    // 6 cases, 4 scenarios, 1 + 1 + 1 + 4 steps
    assert_eq!(steps_checked, 6 * 7);
}
