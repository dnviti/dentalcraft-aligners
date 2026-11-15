//! Commands of the clinical review (`dca_algo::review`, the plugin's
//! `ops_review.py`).
//!
//! * `aligners.review.classify` (`oa.classify`, rules only): the
//!   classifications of every arch with a diagnosis, each with its proposal
//!   preset as the decision and to review; the decisions already taken stay
//!   where the computed value did not change. As in the plugin, when the
//!   staging pattern is automatic and the pattern in force of the review
//!   differs from the one the aligners of the arch were made with, the
//!   aligners of the arch are computed again in the same undo step.
//! * `aligners.review.decide` (`oa.review_decide`): confirm the proposal,
//!   keep the computed value, or reset a record to review; the output names
//!   the next record still to review.
//! * `aligners.review.set-value` (`oa.review_set_value`): another value of
//!   the record's options.
//! * `aligners.review.bulk` (`oa.review_bulk`): confirm every proposal
//!   still to review that has no note asking for a check, or reset them all.
//!
//! Each command is one undo step. The decisions feed the staging pattern
//! (`staging::complexity_of`), the calendar (the attachments to bond before
//! the first aligner) and the report.

use std::time::{SystemTime, UNIX_EPOCH};

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::diagnosis::classify;
use dca_algo::review::{self, Action};
use dca_algo::staging::Pattern;
use dca_algo::teeth::Which;
use dca_model::{AlignersState, MODULE_ID, ReviewRecord, ReviewStatus, StagingPattern};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::messages;
use super::staging::{STAGING, aligners_data, arch_name};

/// Id of the classification (the menu leaf "Clinical review").
pub(crate) const CLASSIFY: &str = "aligners.review.classify";
/// Id of the decision on one record.
pub(crate) const DECIDE: &str = "aligners.review.decide";
/// Id of the value chosen for one record.
pub(crate) const SET_VALUE: &str = "aligners.review.set-value";
/// Id of the decision on every record.
pub(crate) const BULK: &str = "aligners.review.bulk";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: CLASSIFY,
            label: "aligners-menu-review-classify",
            doc: "aligners-cmd-review-classify-doc",
            params: ClassifyParams,
            run: run_classify,
        },
        stage_command! {
            id: DECIDE,
            label: "aligners-cmd-review-decide",
            doc: "aligners-cmd-review-decide-doc",
            params: DecideParams,
            run: run_decide,
        },
        stage_command! {
            id: SET_VALUE,
            label: "aligners-cmd-review-set-value",
            doc: "aligners-cmd-review-set-value-doc",
            params: SetValueParams,
            run: run_set_value,
        },
        stage_command! {
            id: BULK,
            label: "aligners-cmd-review-bulk",
            doc: "aligners-cmd-review-bulk-doc",
            params: BulkParams,
            run: run_bulk,
        },
    ]
}

/// Parameters of `aligners.review.classify`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ClassifyParams {
    /// Arches to classify (all that have a diagnosis).
    arch: Which,
}

/// What `aligners.review.decide` does with the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecideAction {
    /// Confirm the proposal.
    #[default]
    Confirm,
    /// Keep the computed value.
    Computed,
    /// Back to the proposal, to review.
    Reset,
}

/// Parameters of `aligners.review.decide`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecideParams {
    /// The record (`lower:attachment:33`, `upper:complexity:arch`; the
    /// keys are in the output of the classification and in the case).
    key: String,
    /// `confirm` (default), `computed` or `reset`.
    #[serde(default)]
    action: DecideAction,
}

/// Parameters of `aligners.review.set-value`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetValueParams {
    /// The record.
    key: String,
    /// One of the options of the record.
    value: String,
}

/// What `aligners.review.bulk` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BulkMode {
    /// Confirm the proposals still to review that have no note asking for
    /// a check.
    #[default]
    ConfirmUnflagged,
    /// Every record back to its proposal, to review.
    Reset,
}

/// Parameters of `aligners.review.bulk`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BulkParams {
    /// `confirm_unflagged` (default) or `reset`.
    mode: BulkMode,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn counts_json(records: &[ReviewRecord]) -> Value {
    json!(review::counts(records))
}

fn with_data(state: &CaseState, data: AlignersState) -> CaseState {
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    next
}

/// The pattern of the aligners of the arch (`None` when not staged).
fn staged_pattern(data: &AlignersState, arch: Arch) -> Option<Pattern> {
    data.staging_results
        .get(&arch.role())
        .map(|r| Pattern::from(r.pattern))
}

fn run_classify(
    state: &CaseState,
    params: ClassifyParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let explicit = params.arch != Which::All;
    let mut new = Vec::new();
    let mut arches = Vec::new();
    for arch in params.arch.arches() {
        let Some(input) = review::arch_input(&data, arch) else {
            if explicit {
                return Err(messages::invalid(
                    "aligners-err-review-no-diagnosis",
                    &[("arch", &messages::arch_word(arch))],
                ));
            }
            continue;
        };
        new.extend(review::build_records(&input));
        // the automatic choice the review's pattern is compared with
        let auto = Pattern::from(classify::choose_pattern(&input.pattern));
        arches.push((arch, auto));
    }
    if arches.is_empty() {
        return Err(messages::invalid("aligners-err-review-nothing", &[]));
    }
    data.review = review::store(&data.review, new);

    // a pattern decided in the review the aligners were not made with: they
    // are made again (`run_classification`, `staging_ricalcolato`)
    let mut next = with_data(state, data.clone());
    let mut restaged = Vec::new();
    if data.settings.staging.pattern == StagingPattern::Auto {
        for (arch, auto) in &arches {
            let Some(want) = review::pattern_override(&data, arch.role(), *auto) else {
                continue;
            };
            if staged_pattern(&data, *arch).is_some_and(|p| p != want) {
                ctx.check_cancelled()?;
                let spec = super::find(STAGING).expect("the staging is registered");
                let (staged, _) = spec.execute(&next, &json!({ "arch": arch_name(*arch) }), ctx)?;
                next = staged;
                restaged.push(arch_name(*arch));
            }
        }
    }
    let after = aligners_data(&next)?;
    let mut by_arch = serde_json::Map::new();
    for (arch, _) in &arches {
        let recs: Vec<ReviewRecord> = after
            .review
            .iter()
            .filter(|r| r.arch == arch.role())
            .cloned()
            .collect();
        by_arch.insert(
            arch_name(*arch).to_owned(),
            json!({
                "records": recs.len(),
                "counts": counts_json(&recs),
                "attachments": after.attachment_teeth(arch.role()),
            }),
        );
    }
    Ok((
        next,
        json!({
            "records": after.review.iter().map(ReviewRecord::key).collect::<Vec<_>>(),
            "counts": counts_json(&after.review),
            "arches": by_arch,
            "restaged": restaged,
        }),
    ))
}

/// The index of the record `key`, with the errors of a case without a
/// review or with no such record.
fn find_record(data: &AlignersState, key: &str) -> Result<usize, CommandError> {
    if data.review.is_empty() {
        return Err(messages::invalid("aligners-err-review-none", &[]));
    }
    data.review
        .iter()
        .position(|r| r.key() == key)
        .ok_or_else(|| messages::invalid("aligners-err-review-key", &[("key", key)]))
}

/// The record after `i` still to review, from the next one round to the
/// start (`oa.review_decide` moves the selection there).
fn next_pending(records: &[ReviewRecord], i: usize) -> Option<String> {
    (i + 1..records.len())
        .chain(0..i)
        .find(|&j| records[j].status == ReviewStatus::Pending)
        .map(|j| records[j].key())
}

fn decided(state: &CaseState, data: AlignersState, i: usize, next: bool) -> CommandOutput {
    let r = &data.review[i];
    let out = json!({
        "key": r.key(),
        "status": r.status,
        "decided": r.decided,
        "next": if next { next_pending(&data.review, i) } else { None },
        "counts": counts_json(&data.review),
    });
    (with_data(state, data), out)
}

fn run_decide(
    state: &CaseState,
    p: DecideParams,
    _ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let mut data = aligners_data(state)?;
    let i = find_record(&data, &p.key)?;
    let action = match p.action {
        DecideAction::Confirm => Action::Confirm,
        DecideAction::Computed => Action::Computed,
        DecideAction::Reset => Action::Reset,
    };
    review::decide(&mut data.review[i], &action, now());
    Ok(decided(state, data, i, p.action != DecideAction::Reset))
}

fn run_set_value(
    state: &CaseState,
    p: SetValueParams,
    _ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let mut data = aligners_data(state)?;
    let i = find_record(&data, &p.key)?;
    let rec = &mut data.review[i];
    if !rec.options.contains(&p.value) {
        return Err(messages::invalid(
            "aligners-err-review-value",
            &[("value", &p.value), ("options", &rec.options.join(", "))],
        ));
    }
    review::decide(rec, &Action::Value(p.value), now());
    Ok(decided(state, data, i, false))
}

fn run_bulk(
    state: &CaseState,
    p: BulkParams,
    _ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let mut data = aligners_data(state)?;
    if data.review.is_empty() {
        return Err(messages::invalid("aligners-err-review-none", &[]));
    }
    let out = match p.mode {
        BulkMode::ConfirmUnflagged => {
            let n = review::decide_all(&mut data.review, now());
            json!({ "confirmed": n, "counts": counts_json(&data.review) })
        }
        BulkMode::Reset => {
            let t = now();
            for r in &mut data.review {
                review::decide(r, &Action::Reset, t);
            }
            json!({ "reset": data.review.len(), "counts": counts_json(&data.review) })
        }
    };
    Ok((with_data(state, data), out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_model::ReviewKind;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(s: &CaseState) -> AlignersState {
        s.module_data::<AlignersState>(MODULE_ID).unwrap().clone()
    }

    /// The lower demo arch with its teeth measured, the diagnosis, the
    /// final position and the aligners.
    fn staged() -> CaseState {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let s = exec("aligners.diagnosis.space-analysis", &s, Value::Null)
            .unwrap()
            .0;
        exec(STAGING, &s, Value::Null).unwrap().0
    }

    #[test]
    fn the_classification_needs_a_diagnosis() {
        let err = exec(CLASSIFY, &CaseState::default(), Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err:?}");
        let err = exec(DECIDE, &CaseState::default(), json!({ "key": "x" })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn the_classification_makes_the_records_of_the_plugin_to_review() {
        let s = staged();
        let (s, out) = exec(CLASSIFY, &s, Value::Null).unwrap();
        let d = data(&s);
        assert!(!d.review.is_empty());
        assert!(d.review.iter().all(|r| r.status == ReviewStatus::Pending));
        assert_eq!(out["counts"]["pending"], d.review.len());
        let kinds: Vec<ReviewKind> = d.review.iter().take(3).map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                ReviewKind::Complexity,
                ReviewKind::StagingPattern,
                ReviewKind::AlignerSuitability
            ]
        );
        // every tooth of the arch has its four records (a final position)
        let teeth = d.tooth_geometry.len();
        assert_eq!(d.review.len(), 3 + 4 * teeth);
        assert_eq!(
            out["arches"]["lower"]["attachments"],
            json!(d.attachment_teeth(dc_case::ScanRole::Lower))
        );
        // the proposal is the automatic pattern of the aligners: nothing restaged
        assert_eq!(out["restaged"], json!([]));
    }

    #[test]
    fn decisions_one_by_one_and_in_bulk() {
        let (s, _) = exec(CLASSIFY, &staged(), Value::Null).unwrap();
        let d = data(&s);
        let first = d.review[0].key();
        let (s, out) = exec(DECIDE, &s, json!({ "key": first })).unwrap();
        assert_eq!(out["status"], "accepted");
        assert_eq!(out["next"], d.review[1].key());
        assert_eq!(data(&s).review[0].status, ReviewStatus::Accepted);
        assert!(data(&s).review[0].decided_at.is_some());

        // a value outside the options is refused
        let att = d
            .review
            .iter()
            .find(|r| r.kind == ReviewKind::Attachment)
            .unwrap()
            .key();
        let err = exec(SET_VALUE, &s, json!({ "key": att, "value": "glue" })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("glue")));
        let (s, out) = exec(SET_VALUE, &s, json!({ "key": att, "value": "rotation" })).unwrap();
        assert!(matches!(
            out["status"].as_str(),
            Some("modified" | "accepted")
        ));
        let fdi = att.rsplit(':').next().unwrap().to_owned();
        assert!(
            data(&s)
                .attachment_teeth(dc_case::ScanRole::Lower)
                .contains(&fdi)
        );

        let (s, out) = exec(BULK, &s, Value::Null).unwrap();
        let d = data(&s);
        let flagged = d.review.iter().filter(|r| r.needs_check()).count();
        assert_eq!(
            out["confirmed"].as_u64().unwrap() as usize,
            d.review.len() - 2 - flagged
        );
        assert_eq!(review::counts(&d.review).pending, flagged);

        let (s, out) = exec(DECIDE, &s, json!({ "key": first, "action": "reset" })).unwrap();
        assert_eq!(out["status"], "pending");
        assert_eq!(out["next"], Value::Null);
        let (s, _) = exec(BULK, &s, json!({ "mode": "reset" })).unwrap();
        assert!(
            data(&s)
                .review
                .iter()
                .all(|r| r.status == ReviewStatus::Pending && r.decided == r.proposed)
        );
        let err = exec(DECIDE, &s, json!({ "key": "lower:complexity:11" })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)));
    }

    #[test]
    fn a_pattern_decided_in_the_review_makes_the_aligners_again() {
        let (s, _) = exec(CLASSIFY, &staged(), Value::Null).unwrap();
        let d = data(&s);
        let rec = d
            .review
            .iter()
            .find(|r| r.kind == ReviewKind::StagingPattern)
            .unwrap();
        let other = if rec.proposed == "proportional" {
            "max_velocity"
        } else {
            "proportional"
        };
        // the decision itself changes nothing else
        let (s, _) = exec(SET_VALUE, &s, json!({ "key": rec.key(), "value": other })).unwrap();
        assert_eq!(data(&s).stages, d.stages);
        // classified again: the decision stays, the aligners follow it
        let (s, out) = exec(CLASSIFY, &s, Value::Null).unwrap();
        assert_eq!(out["restaged"], json!(["lower"]));
        let pattern = data(&s).staging_results[&dc_case::ScanRole::Lower].pattern;
        assert_eq!(review::pattern_id(Pattern::from(pattern)), other);
        // and so does any later staging
        let (s, _) = exec(STAGING, &s, Value::Null).unwrap();
        let pattern = data(&s).staging_results[&dc_case::ScanRole::Lower].pattern;
        assert_eq!(review::pattern_id(Pattern::from(pattern)), other);
    }
}
