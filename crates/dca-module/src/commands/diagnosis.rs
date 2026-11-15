//! Commands of the `diagnosis` stage (`dca_algo::diagnosis`).
//!
//! The four computing leaves of the Diagnosis menu (space analysis, Little
//! index, Bolton analysis, complexity score) are one analysis of the
//! measured teeth: each leaf runs all of it, stores it in
//! `AlignersState::diagnosis` and returns the whole JSON report, so a leaf
//! never shows a half-updated diagnosis. Needs the segmentation and the
//! teeth frames (`aligners.teeth.frames`), which keep the measures in
//! `AlignersState::tooth_geometry`.
//!
//! `aligners.diagnosis.edit-values` is the plugin's `oa.diagnosis_edit`: the
//! clinician writes the discrepancy, the Little index, the midline offset
//! and the largest space of an arch by hand (a number, or `null` to go back
//! to the computed one), kept apart in `Diagnosis::edits`. The complexity
//! follows them (the CAT-CAT items are worked out again from the values in
//! force, `dca_algo::diagnosis::edit`), the space, Little index and
//! complexity the staging reads are the values in force, and the export
//! report shows them and marks the ones written by hand. The aligners
//! planned before are left, out of date (`staging_outdated`, as after a
//! movement of the final position). A new analysis of an arch writes its
//! diagnosis anew, as the plugin's `analyze` does, and drops its hand values
//! (`hand_values_reset` in the report).
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

use super::messages;
use dc_case::{CaseState, Fdi, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::diagnosis::edit::{self, EditField};
use dca_algo::diagnosis::{self, ArchDiagnosis, ArchInput, Crowns, Input, Params, classify};
use dca_algo::evidence;
use dca_algo::frame;
use dca_model::{AlignersState, ArchDetails, ArchEdits, MODULE_ID, ToothMeasures};
use schemars::JsonSchema;
use serde::Deserialize;
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
            label: "aligners-cmd-diagnosis-little-index",
            doc: "aligners-cmd-diagnosis-little-index-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.bolton",
            label: "aligners-cmd-diagnosis-bolton",
            doc: "aligners-cmd-diagnosis-bolton-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.complexity",
            label: "aligners-cmd-diagnosis-complexity",
            doc: "aligners-cmd-diagnosis-complexity-doc",
            params: dca_algo::diagnosis::Params,
            run: run_diagnosis,
        },
        stage_command! {
            id: "aligners.diagnosis.edit-values",
            label: "aligners-cmd-diagnosis-edit-values",
            doc: "aligners-cmd-diagnosis-edit-values-doc",
            params: EditParams,
            run: run_edit,
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
    // the scan as it was read, where the teeth stage measured
    let closed = state.blobs.require(&scan.mesh)?;
    let case_vertices: Vec<_> = crate::open_scan::open_points(data, &scan.mesh, closed)
        .iter()
        .map(|p| p.as_dvec3())
        .collect();
    // the arch frame the teeth stage measured in (the one the seeds define)
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: &case_vertices,
            seeds: &data.seeds,
            occlusal_flipped: data.occlusal_flipped.contains(&role),
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
    if labels.len() != closed.vertex_count() {
        return Err(crate::commands::messages::labels_mismatch(
            arch,
            labels.len(),
            closed.vertex_count(),
        ));
    }
    let in_arch: Vec<_> = case_vertices
        .iter()
        .map(|&p| to_arch.transform_point3(p))
        .collect();
    let crowns = diagnosis::crowns_of(&in_arch, &labels[..in_arch.len()]);
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
    let mut data = data_of(state)?;
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
        return Err(messages::invalid("aligners-err-no-measured-teeth", &[]));
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
        messages::invalid("aligners-err-then-measure", &[("reason", &e.to_string())])
    })?;
    ctx.check_cancelled()?;

    // keep what was computed for the arches not analysed now; the analysis
    // of an arch writes its diagnosis anew, without the values the clinician
    // wrote by hand (the plugin's `analyze` does not read them)
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
        let reset = data.diagnosis.edits.remove(&role).unwrap_or_default();
        let mut value = serde_json::to_value(d).map_err(failed)?;
        value["classification"] = classification(d, &teeth[arch])?;
        let by_hand = by_hand_keys(&reset);
        if !by_hand.is_empty() {
            value["hand_values_reset"] = json!(by_hand);
        }
        report.insert(arch_name(*arch).to_owned(), value);
    }
    data.diagnosis.bolton_anterior = out.bolton.anterior;
    data.diagnosis.bolton_overall = out.bolton.overall;
    // the arches not analysed now keep the values written by hand
    data.diagnosis.complexity_score = edit::complexity_score(&data);

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

/// The Aligners data of the case.
fn data_of(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(messages::no_data),
    }
}

/// The names of the values written by hand, sorted (the plugin's
/// `manuale`).
fn by_hand_keys(edits: &ArchEdits) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = EditField::ALL
        .into_iter()
        .filter(|f| f.get(edits).is_some())
        .map(EditField::key)
        .collect();
    keys.sort_unstable();
    keys
}

/// Parameters of `aligners.diagnosis.edit-values`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct EditParams {
    /// The arch: upper or lower (default: the only arch with a case
    /// analysis).
    pub arch: Option<Arch>,
    /// The values to write (a number, in mm) or to reset to the computed
    /// one (`null`), by name: `discrepancy` (space available minus
    /// required, -30 to 30, negative is crowding), `little` (0 to 40),
    /// `midline_offset` (-10 to 10), `largest_space` (0 to 20). The ones not
    /// named stay as they are.
    pub values: BTreeMap<String, Option<f64>>,
}

/// The arch an edit is about: the one given, else the only one analysed.
fn edited_arch(data: &AlignersState, given: Option<Arch>) -> Result<Arch, CommandError> {
    if let Some(arch) = given {
        return Ok(arch);
    }
    let analysed: Vec<Arch> = [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter(|a| data.diagnosis.arches.contains_key(&a.role()))
        .collect();
    match analysed.as_slice() {
        [one] => Ok(*one),
        [] => Err(messages::invalid(
            "aligners-err-diagnosis-edit-nothing",
            &[],
        )),
        _ => Err(messages::invalid("aligners-err-diagnosis-edit-arch", &[])),
    }
}

/// `aligners.diagnosis.edit-values`: the values of the case analysis the
/// clinician measured themselves (the plugin's `oa.diagnosis_edit`,
/// `manual.diagnosis_override`). Every value is checked before anything is
/// written: one out of range, or unknown, refuses the whole command and the
/// case is as it was.
fn run_edit(
    state: &CaseState,
    params: EditParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if params.values.is_empty() {
        return Err(messages::invalid("aligners-err-diagnosis-edit-empty", &[]));
    }
    let mut changes = Vec::new();
    for (key, value) in &params.values {
        let field = EditField::from_key(key).ok_or_else(|| {
            messages::invalid("aligners-err-diagnosis-edit-field", &[("field", key)])
        })?;
        if let Some(v) = value
            && !field.accepts(*v)
        {
            let (min, max) = field.range();
            return Err(messages::invalid(
                "aligners-err-diagnosis-edit-range",
                &[
                    ("field", key),
                    ("min", &min.to_string()),
                    ("max", &max.to_string()),
                    ("value", &v.to_string()),
                ],
            ));
        }
        changes.push((field, *value));
    }
    let mut data = data_of(state)?;
    let arch = edited_arch(&data, params.arch)?;
    let role = arch.role();
    let computed = edit::computed_values(&data, role).ok_or_else(|| {
        messages::invalid(
            "aligners-err-diagnosis-edit-first",
            &[("arch", &messages::arch_word(arch))],
        )
    })?;
    let mut edits = data.diagnosis.edits_of(role);
    for (field, value) in changes {
        edit::set_value(&mut edits, &computed, field, value);
    }
    if edits.is_empty() {
        data.diagnosis.edits.remove(&role);
    } else {
        data.diagnosis.edits.insert(role, edits);
    }
    // the complexity follows the values in force
    data.diagnosis.complexity_score = edit::complexity_score(&data);
    let effective = edit::effective(&data, role)
        .ok_or_else(|| messages::invalid("aligners-err-diagnosis-edit-nothing", &[]))?;
    let named = |v: &edit::Values| {
        let mut m = Map::new();
        for f in EditField::ALL {
            m.insert(f.key().to_owned(), json!(v.get(f)));
        }
        Value::Object(m)
    };
    // the aligners were planned with the old values
    let staging_outdated = data
        .stages
        .iter()
        .any(|s| s.poses.keys().any(|f| Arch::of(*f) == arch));
    let out = json!({
        "arch": arch_name(arch),
        "values": named(&effective.values),
        "computed": named(&computed),
        "by_hand": by_hand_keys(&edits),
        "little_category": effective.little_category,
        "catcat_items": effective.items,
        "catcat_score": effective.score,
        "level": effective.level,
        "complexity_score": data.diagnosis.complexity_score,
        "staging_outdated": staging_outdated,
    });
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, out))
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
    fn the_four_computing_commands_are_registered() {
        for id in IDS {
            let spec = crate::commands::find(id).unwrap();
            assert_eq!(spec.schema()["type"], "object", "{id}");
            assert!(spec.schema()["properties"]["arch"].is_object(), "{id}");
            assert!(crate::commands::is_registered(id), "{id}");
        }
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

    const EDIT: &str = "aligners.diagnosis.edit-values";

    fn analysed(arch: &str) -> CaseState {
        exec(
            IDS[0],
            &measured(&demo(&CaseState::default(), arch)),
            Value::Null,
        )
        .unwrap()
        .0
    }

    #[test]
    fn the_edit_command_is_registered_with_its_schema() {
        let spec = crate::commands::find(EDIT).unwrap();
        assert!(crate::commands::is_registered(EDIT));
        assert_eq!(spec.schema()["type"], "object");
        assert!(spec.schema()["properties"]["arch"].is_object());
        assert!(spec.schema()["properties"]["values"].is_object());
    }

    #[test]
    fn values_written_by_hand_change_the_complexity_and_a_reset_brings_the_computed_ones_back() {
        let state = analysed("lower");
        let role = ScanRole::Lower;
        let before = data(&state).diagnosis.clone();
        let computed = before.arches[&role];

        // 9 mm of crowding, a midline 4 mm off and a 5 mm space, by hand
        let (next, out) = exec(
            EDIT,
            &state,
            json!({ "values": { "discrepancy": -9.0, "midline_offset": 4.0, "largest_space": 5.0 } }),
        )
        .unwrap();
        let d = &data(&next).diagnosis;
        assert_eq!(out["arch"], "lower");
        assert_eq!(
            out["by_hand"],
            json!(["discrepancy", "largest_space", "midline_offset"])
        );
        assert_eq!(out["values"]["discrepancy"], -9.0);
        assert_eq!(out["values"]["little"], computed.little_index_mm);
        assert_eq!(
            out["computed"]["largest_space"],
            before.largest_space_mm(role)
        );
        // the items follow: crowding 4, midline (4 - 2) * 2, diastema (5 - 2) * 2
        assert_eq!(out["catcat_items"]["affollamento"], 4);
        assert_eq!(out["catcat_items"]["deviazione mediana"], 4);
        assert_eq!(out["catcat_items"]["diastema maggiore"], 6);
        let score = out["catcat_score"].as_u64().unwrap();
        assert_eq!(out["level"], diagnosis::Level::of(score as u32).label());
        assert_eq!(d.complexity_score, Some(score as f64));
        assert_eq!(out["complexity_score"].as_f64(), Some(score as f64));
        // kept apart: the computed values are as they were
        assert_eq!(d.arches, before.arches);
        assert_eq!(d.details, before.details);
        assert_eq!(d.crowding_mm(role), Some(9.0));
        assert_eq!(d.little_index_mm(role), Some(computed.little_index_mm));
        assert_eq!(out["staging_outdated"], false);

        // one more value on top: the others stay
        let (more, out) = exec(EDIT, &next, json!({ "values": { "little": 12.5 } })).unwrap();
        assert_eq!(
            out["by_hand"],
            json!(["discrepancy", "largest_space", "little", "midline_offset"])
        );
        assert_eq!(out["little_category"], "irregolarita molto grave");
        assert_eq!(data(&more).diagnosis.crowding_mm(role), Some(9.0));

        // null resets one value
        let (reset, out) = exec(
            EDIT,
            &more,
            json!({ "values": { "discrepancy": null, "little": null } }),
        )
        .unwrap();
        assert_eq!(out["by_hand"], json!(["largest_space", "midline_offset"]));
        assert_eq!(out["values"]["discrepancy"], out["computed"]["discrepancy"]);
        // the last two reset: the diagnosis is the computed one again
        let (clean, out) = exec(
            EDIT,
            &reset,
            json!({ "values": { "largest_space": null, "midline_offset": null } }),
        )
        .unwrap();
        assert_eq!(out["by_hand"], json!([]));
        assert_eq!(data(&clean).diagnosis, before);

        // a value equal to the computed one is not by hand
        let same = computed.little_index_mm;
        let (same, out) = exec(EDIT, &state, json!({ "values": { "little": same } })).unwrap();
        assert_eq!(out["by_hand"], json!([]));
        assert_eq!(data(&same).diagnosis, before);
    }

    #[test]
    fn a_value_out_of_range_or_unknown_changes_nothing() {
        let state = analysed("lower");
        let before = data(&state).clone();
        for values in [
            json!({ "discrepancy": 31.0 }),
            json!({ "discrepancy": -30.5 }),
            json!({ "little": -0.1 }),
            json!({ "little": 40.5 }),
            json!({ "midline_offset": 10.5 }),
            json!({ "largest_space": 21.0 }),
            // one good value next to a bad one: still nothing changes
            json!({ "little": 3.0, "largest_space": -1.0 }),
            json!({ "overjet": 2.0 }),
            json!({}),
        ] {
            let r = exec(EDIT, &state, json!({ "values": values }));
            assert!(
                matches!(r, Err(CommandError::Invalid(_))),
                "{values}: {r:?}"
            );
        }
        // the limits themselves are fine
        for (k, v) in [
            ("discrepancy", -30.0),
            ("discrepancy", 30.0),
            ("little", 0.0),
            ("little", 40.0),
            ("midline_offset", -10.0),
            ("largest_space", 20.0),
        ] {
            exec(EDIT, &state, json!({ "values": { k: v } })).unwrap();
        }
        // the case was never touched
        assert_eq!(*data(&state), before);
    }

    #[test]
    fn the_arch_is_the_one_analysed_or_given_and_must_be_analysed() {
        // no analysis at all
        assert!(matches!(
            exec(
                EDIT,
                &CaseState::default(),
                json!({ "values": { "little": 1.0 } })
            ),
            Err(CommandError::Invalid(_))
        ));
        // measured, not analysed
        let measured = measured(&demo(&CaseState::default(), "lower"));
        assert!(matches!(
            exec(
                EDIT,
                &measured,
                json!({ "arch": "lower", "values": { "little": 1.0 } })
            ),
            Err(CommandError::Invalid(_))
        ));
        // the other arch is not analysed
        let lower = analysed("lower");
        assert!(matches!(
            exec(
                EDIT,
                &lower,
                json!({ "arch": "upper", "values": { "little": 1.0 } })
            ),
            Err(CommandError::Invalid(_))
        ));
        // both analysed: the arch has to be given, and only that one changes
        let both = exec(IDS[0], &measured_both(), Value::Null).unwrap().0;
        assert!(matches!(
            exec(EDIT, &both, json!({ "values": { "little": 1.0 } })),
            Err(CommandError::Invalid(m)) if m.contains("arch")
        ));
        let (next, out) = exec(
            EDIT,
            &both,
            json!({ "arch": "upper", "values": { "discrepancy": -12.0 } }),
        )
        .unwrap();
        assert_eq!(out["arch"], "upper");
        assert!(data(&next).diagnosis.edits_of(ScanRole::Lower).is_empty());
        assert_eq!(
            data(&next)
                .diagnosis
                .edits_of(ScanRole::Upper)
                .discrepancy_mm,
            Some(-12.0)
        );
        // the case complexity is the largest of the arches, hand values included
        let upper = out["catcat_score"].as_u64().unwrap() as f64;
        let lower = f64::from(data(&both).diagnosis.details[&ScanRole::Lower].catcat_score);
        assert_eq!(
            data(&next).diagnosis.complexity_score,
            Some(upper.max(lower))
        );
    }

    fn measured_both() -> CaseState {
        measured(&demo(&demo(&CaseState::default(), "lower"), "upper"))
    }

    #[test]
    fn a_new_analysis_of_the_arch_writes_the_diagnosis_anew_without_the_hand_values() {
        // as the plugin: `analyze` does not read what `diagnosis_override` wrote
        let state = analysed("lower");
        let before = data(&state).diagnosis.clone();
        let (edited, _) = exec(
            EDIT,
            &state,
            json!({ "values": { "discrepancy": -9.0, "little": 11.0 } }),
        )
        .unwrap();
        assert_ne!(data(&edited).diagnosis, before);
        for id in IDS {
            let (again, out) = exec(id, &edited, Value::Null).unwrap();
            assert_eq!(data(&again).diagnosis, before, "{id}");
            assert_eq!(
                out["lower"]["hand_values_reset"],
                json!(["discrepancy", "little"]),
                "{id}"
            );
            // the report is the computed one
            assert_eq!(
                out["lower"]["space"]["discrepancy"].as_f64().unwrap(),
                -before.arches[&ScanRole::Lower].crowding_mm(),
                "{id}"
            );
        }
        // nothing was written: nothing to say
        let (_, out) = exec(IDS[0], &state, Value::Null).unwrap();
        assert!(out["lower"].get("hand_values_reset").is_none());
    }

    #[test]
    fn the_other_arch_keeps_its_hand_values_when_one_arch_is_analysed_again() {
        let both = exec(IDS[0], &measured_both(), Value::Null).unwrap().0;
        let (edited, _) = exec(
            EDIT,
            &both,
            json!({ "arch": "lower", "values": { "little": 8.0 } }),
        )
        .unwrap();
        let (again, out) = exec(IDS[0], &edited, json!({ "arch": "upper" })).unwrap();
        assert_eq!(
            data(&again)
                .diagnosis
                .edits_of(ScanRole::Lower)
                .little_index_mm,
            Some(8.0)
        );
        assert!(out.get("lower").is_none());
    }
}
