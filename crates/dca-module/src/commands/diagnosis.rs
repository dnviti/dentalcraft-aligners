//! Commands of the `diagnosis` stage (`dca_algo::diagnosis`).
//!
//! The four computing leaves of the Diagnosis menu (space analysis, Little
//! index, Bolton analysis, complexity score) are one analysis of the
//! measured teeth: each leaf runs all of it, stores it in
//! `AlignersState::diagnosis` and returns the whole JSON report, so a leaf
//! never shows a half-updated diagnosis. Needs the segmentation and the
//! teeth frames (`aligners.teeth.frames`), which keep the measures in
//! `AlignersState::tooth_geometry`. The "correct the values by hand" leaf is
//! not a command yet.
//!
//! Report, per arch (`lower`, `upper`): `space` (available, required,
//! discrepancy; negative means crowding), `little`, `little_details`,
//! `little_category`, `rotations`, `widths`, `midline_offset`,
//! `largest_space`, `catcat_items`, `catcat_score`, `level`, and
//! `classification` (`aligner_suitable` with its `reasons`, and per tooth
//! the nearest typical crown type and the plausibility of its dimensions).
//! Top level: `bolton` (overall, anterior, with the norms), `complexity_score`
//! and the `references` the values come from.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::diagnosis::{self, ArchDiagnosis, ArchInput, Crowns, Input, Params, classify};
use dca_algo::evidence;
use dca_algo::frame;
use dca_model::{AlignersState, ArchDetails, MODULE_ID, ToothMeasures};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.diagnosis.space-analysis",
            label: "aligners-menu-diagnosis-space-analysis",
            doc: "aligners-cmd-diagnosis-space-analysis-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.little-index",
            label: "aligners-menu-diagnosis-little-index",
            doc: "aligners-cmd-diagnosis-little-index-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.bolton",
            label: "aligners-menu-diagnosis-bolton",
            doc: "aligners-cmd-diagnosis-bolton-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.complexity",
            label: "aligners-menu-diagnosis-complexity",
            doc: "aligners-cmd-diagnosis-complexity-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
    ]
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn failed(e: impl ToString) -> CommandError {
    CommandError::Failed(e.to_string())
}

/// The measured teeth of an arch in arch coordinates and, when `with_crowns`,
/// the crown vertices of each tooth (for the largest space). `None` when the
/// arch has no measured teeth or no segmented scan.
type ArchData = (BTreeMap<Fdi, ToothMeasures>, Option<Crowns>);

fn arch_data(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
    with_crowns: bool,
) -> Result<Option<ArchData>, CommandError> {
    let role: ScanRole = arch.role();
    let measured: Vec<_> = data
        .tooth_geometry
        .iter()
        .filter(|(f, _)| Arch::of(**f) == arch)
        .collect();
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
    };
    if measured.is_empty() {
        return Ok(None);
    }
    let mesh = state.blobs.require(&scan.mesh)?;
    let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    // the arch frame the teeth stage measured in (the one the seeds define)
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: &case_vertices,
            seeds: &data.seeds,
        },
        &frame::Params::default(),
    )
    .map_err(failed)?;
    let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
    let teeth = measured
        .iter()
        .map(|(f, m)| (**f, diagnosis::transform_measures(m, &to_arch)))
        .collect();
    if !with_crowns {
        return Ok(Some((teeth, None)));
    }
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .as_ref()
        .ok_or_else(|| crate::commands::messages::labels_missing(arch))?;
    if labels.len() != case_vertices.len() {
        return Err(crate::commands::messages::labels_mismatch(
            arch,
            labels.len(),
            case_vertices.len(),
        ));
    }
    let in_arch: Vec<_> = case_vertices
        .iter()
        .map(|&p| to_arch.transform_point3(p))
        .collect();
    let crowns = diagnosis::crowns_of(&in_arch, labels);
    Ok(Some((teeth, Some(crowns))))
}

/// The classification block of an arch: aligner-only suitability and, per
/// tooth, the nearest typical crown and the plausibility of its dimensions.
fn classification(
    d: &ArchDiagnosis,
    teeth: &BTreeMap<Fdi, ToothMeasures>,
) -> Result<Value, CommandError> {
    let (suitable, reasons) = classify::aligner_suitable(d.space.discrepancy, 0.0);
    let mut per_tooth = Map::new();
    for fdi in &d.sequence {
        let t = &teeth[fdi];
        let kind = classify::tooth_type(*fdi, t.width_md, t.width_bl);
        let (plausible, why) =
            classify::segmentation_plausible(*fdi, t.width_md, t.width_bl, t.height);
        per_tooth.insert(
            fdi.to_string(),
            json!({
                "type": kind.best,
                "expected_type": classify::type_of_position(anatomy::index(*fdi)),
                "type_matches": Some(kind.best) == classify::type_of_position(anatomy::index(*fdi)),
                "plausible": plausible,
                "reasons": why,
            }),
        );
    }
    Ok(json!({
        "aligner_suitable": suitable,
        "reasons": reasons,
        "teeth": per_tooth,
    }))
}

fn run_diagnosis(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = match state.module(MODULE_ID) {
        None => AlignersState::default(),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data)?,
    };
    // the measured teeth of each arch in arch coordinates (both, for the
    // Bolton ratios); the crowns only of the arches to analyse
    let selected = params.arch.arches();
    let mut teeth: BTreeMap<Arch, BTreeMap<Fdi, ToothMeasures>> = BTreeMap::new();
    let mut crowns: BTreeMap<Arch, Crowns> = BTreeMap::new();
    for arch in [Arch::Lower, Arch::Upper] {
        if let Some((t, c)) = arch_data(state, &data, arch, selected.contains(&arch))? {
            teeth.insert(arch, t);
            if let Some(c) = c {
                crowns.insert(arch, c);
            }
        }
        ctx.check_cancelled()?;
    }
    if teeth.is_empty() {
        return Err(CommandError::Invalid(
            "no arch has measured teeth: run the segmentation and the teeth frames first"
                .to_owned(),
        ));
    }

    let input = Input {
        arches: teeth
            .iter()
            .map(|(arch, t)| ArchInput {
                arch: *arch,
                teeth: t,
                crowns: crowns.get(arch),
            })
            .collect(),
    };
    let out = diagnosis::run(&input, &params).map_err(|e| {
        CommandError::Invalid(format!(
            "{e}: run the segmentation and the teeth frames first"
        ))
    })?;
    ctx.check_cancelled()?;

    // keep what was computed for the arches not analysed now
    let mut report = Map::new();
    for (arch, d) in &out.arches {
        let role = arch.role();
        data.diagnosis.arches.insert(role, d.arch_space());
        data.diagnosis.details.insert(
            role,
            ArchDetails {
                largest_space_mm: d.largest_space,
                midline_offset_mm: d.midline_offset,
                catcat_score: d.catcat_score,
            },
        );
        let mut value = serde_json::to_value(d).map_err(failed)?;
        value["classification"] = classification(d, &teeth[arch])?;
        report.insert(arch_name(*arch).to_owned(), value);
    }
    data.diagnosis.bolton_anterior = out.bolton.anterior;
    data.diagnosis.bolton_overall = out.bolton.overall;
    data.diagnosis.complexity_score = data
        .diagnosis
        .details
        .values()
        .map(|d| d.catcat_score)
        .max()
        .map(f64::from);

    report.insert(
        "bolton".to_owned(),
        json!({
            "overall": out.bolton.overall,
            "anterior": out.bolton.anterior,
            "overall_norm": diagnosis::BOLTON_OVERALL_NORM,
            "anterior_norm": diagnosis::BOLTON_ANTERIOR_NORM,
        }),
    );
    report.insert(
        "complexity_score".to_owned(),
        json!(data.diagnosis.complexity_score),
    );
    report.insert(
        "references".to_owned(),
        json!(evidence::cite(&[
            "alharbi2008",
            "little1975",
            "bolton1958",
            "long2020catcat",
            "wang2025consensus",
        ])),
    );
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;

    const IDS: [&str; 4] = [
        "aligners.diagnosis.space-analysis",
        "aligners.diagnosis.little-index",
        "aligners.diagnosis.bolton",
        "aligners.diagnosis.complexity",
    ];

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo(state: &CaseState, arch: &str) -> CaseState {
        exec("aligners.demo", state, json!({ "arch": arch }))
            .unwrap()
            .0
    }

    fn measured(state: &CaseState) -> CaseState {
        exec("aligners.teeth.frames", state, Value::Null).unwrap().0
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    #[test]
    fn the_four_computing_leaves_are_registered_and_ready() {
        for id in IDS {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object", "{id}");
            assert!(spec.schema()["properties"]["arch"].is_object(), "{id}");
            assert!(crate::tree::find(id).is_some(), "{id} is a menu leaf");
        }
        // correcting the values by hand is a later command
        assert!(crate::commands::find("aligners.diagnosis.edit-values").is_none());
    }

    #[test]
    fn a_case_without_measured_teeth_has_no_diagnosis() {
        for id in IDS {
            assert!(matches!(
                exec(id, &CaseState::default(), Value::Null),
                Err(CommandError::Invalid(m)) if m.contains("teeth")
            ));
        }
        // segmented, but the teeth were not measured yet
        let lower = demo(&CaseState::default(), "lower");
        assert!(matches!(
            exec(IDS[0], &lower, Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("measured")
        ));
        // an arch that has no teeth
        let lower = measured(&lower);
        assert!(matches!(
            exec(IDS[0], &lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper")
        ));
        assert!(matches!(
            exec(IDS[0], &lower, json!({ "arch": "sideways" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_teeth_stage_keeps_the_measures_the_diagnosis_reads() {
        let state = measured(&demo(&CaseState::default(), "lower"));
        let geometry = &data(&state).tooth_geometry;
        assert_eq!(geometry.len(), 14);
        assert_eq!(geometry.len(), data(&state).tooth_frames.len());
        for (fdi, m) in geometry {
            assert!((3.0..14.0).contains(&m.width_md), "{fdi}");
            assert!((m.contact_m - m.contact_d).length() > 2.0, "{fdi}");
        }
    }

    #[test]
    fn a_demo_arch_is_diagnosed_and_stored() {
        for arch in ["lower", "upper"] {
            let state = measured(&demo(&CaseState::default(), arch));
            let role = if arch == "lower" {
                ScanRole::Lower
            } else {
                ScanRole::Upper
            };
            assert!(data(&state).diagnosis.arches.is_empty());
            let (next, out) = exec(IDS[0], &state, Value::Null).unwrap();
            let r = &out[arch];
            assert_eq!(r["sequence"].as_array().unwrap().len(), 14);

            // the report and the stored diagnosis agree
            let stored = data(&next).diagnosis.arches[&role];
            assert_eq!(
                stored.available_mm,
                r["space"]["available"].as_f64().unwrap()
            );
            assert_eq!(stored.required_mm, r["space"]["required"].as_f64().unwrap());
            assert_eq!(stored.little_index_mm, r["little"].as_f64().unwrap());
            assert_eq!(
                stored.crowding_mm(),
                -r["space"]["discrepancy"].as_f64().unwrap()
            );
            let details = data(&next).diagnosis.details[&role];
            assert_eq!(
                f64::from(details.catcat_score),
                r["catcat_score"].as_f64().unwrap()
            );
            assert_eq!(
                data(&next).diagnosis.complexity_score,
                Some(f64::from(details.catcat_score))
            );
            assert_eq!(out["complexity_score"].as_f64(), r["catcat_score"].as_f64());

            // plausible numbers for a synthetic arch
            let available = stored.available_mm;
            assert!((40.0..100.0).contains(&available), "{available}");
            assert!((40.0..100.0).contains(&stored.required_mm));
            assert!(stored.little_index_mm > 0.0);
            assert_eq!(r["little_details"].as_array().unwrap().len(), 5);
            assert!(
                r["little_category"]
                    .as_str()
                    .unwrap()
                    .starts_with("irregolarita")
                    || r["little_category"] == "allineamento perfetto"
            );
            assert!(r["level"].is_string());
            assert!(r["catcat_items"]["affollamento"].is_number());
            assert_eq!(r["rotations"].as_object().unwrap().len(), 14);

            // classification: every tooth of the demo is a plausible crown
            let c = &r["classification"];
            assert!(c["aligner_suitable"].is_boolean());
            assert_eq!(c["teeth"].as_object().unwrap().len(), 14);
            for (fdi, t) in c["teeth"].as_object().unwrap() {
                assert_eq!(t["plausible"], true, "{fdi}: {t}");
            }

            // one arch alone has no Bolton ratio
            assert!(out["bolton"]["overall"].is_null());
            assert!(data(&next).diagnosis.bolton_overall.is_none());
            // the scans and the other data are untouched
            assert_eq!(next.case.scans, state.case.scans);
            assert_eq!(data(&next).tooth_frames, data(&state).tooth_frames);
            // every leaf gives the same diagnosis; a rerun is stable
            for id in IDS {
                let (again, again_out) = exec(id, &next, Value::Null).unwrap();
                assert_eq!(again_out, out, "{id}");
                assert_eq!(data(&again).diagnosis, data(&next).diagnosis, "{id}");
            }
        }
    }

    #[test]
    fn both_arches_give_the_bolton_ratios_and_one_can_be_redone_alone() {
        let both = measured(&demo(&demo(&CaseState::default(), "lower"), "upper"));
        let (all, out) = exec(IDS[2], &both, Value::Null).unwrap();
        assert!(out["lower"].is_object() && out["upper"].is_object());
        let overall = out["bolton"]["overall"].as_f64().unwrap();
        let anterior = out["bolton"]["anterior"].as_f64().unwrap();
        assert!((80.0..105.0).contains(&overall), "{overall}");
        assert!((65.0..95.0).contains(&anterior), "{anterior}");
        assert_eq!(data(&all).diagnosis.bolton_overall, Some(overall));
        assert_eq!(data(&all).diagnosis.bolton_anterior, Some(anterior));
        assert_eq!(data(&all).diagnosis.arches.len(), 2);
        let score = data(&all)
            .diagnosis
            .details
            .values()
            .map(|d| d.catcat_score)
            .max();
        assert_eq!(data(&all).diagnosis.complexity_score, score.map(f64::from));

        // the upper arch alone: the lower diagnosis stays, the ratios are still known
        let (redone, out) = exec(IDS[0], &all, json!({ "arch": "upper" })).unwrap();
        assert!(out.get("lower").is_none() && out["upper"].is_object());
        assert_eq!(data(&redone).diagnosis, data(&all).diagnosis);
        assert_eq!(out["bolton"]["overall"].as_f64(), Some(overall));
    }
}
