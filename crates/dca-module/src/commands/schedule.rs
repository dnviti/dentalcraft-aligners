//! Commands of the `schedule` stage (`dca_algo::schedule`).
//!
//! `aligners.planning.schedule` builds the change calendar of the aligners
//! stored by the staging: delivery and last day of each aligner, the days of
//! wear (fixed, or 7/14 days by the movements of the aligner), what to do
//! before it (attachments, IPR) and the passive aligners that keep the two
//! arches in step. The calendar is returned; the case is not modified. The
//! `.ics` file is written by the export.

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::schedule::{self, Lang, Policy};
use dca_algo::teeth::Which;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::messages;
use super::staging::{aligners_data, arch_name};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.planning.schedule",
        label: "aligners-cmd-schedule",
        doc: "aligners-cmd-schedule-doc",
        params: Params,
        run: run_schedule,
    }]
}

/// Parameters of the command.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Params {
    /// Arches (all that have aligners).
    arch: Which,
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    start_date: Option<String>,
    /// Days of wear of each aligner (default: the case settings).
    interval_days: Option<u32>,
    /// Interval policy.
    policy: Policy,
    /// Language of the text of the to-do items.
    lang: Lang,
    /// Teeth that get an attachment, bonded before aligner 1.
    attachments: Vec<String>,
}

/// The arches that have aligners in the case, lower first.
fn arches_with_stages(which: Which, stages: &[dca_model::Stage]) -> Vec<Arch> {
    which
        .arches()
        .into_iter()
        .filter(|&a| schedule::active_stages(a, stages) > 0)
        .collect()
}

fn run_schedule(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let arches = arches_with_stages(params.arch, &data.stages);
    if arches.is_empty() {
        return Err(messages::invalid("aligners-err-no-aligners", &[]));
    }
    let total = arches
        .iter()
        .map(|&a| schedule::active_stages(a, &data.stages))
        .max()
        .unwrap_or(0);
    let start = params
        .start_date
        .clone()
        .unwrap_or_else(|| schedule::Date::today().to_iso());
    let cal_params = schedule::Params {
        start_date: Some(start),
        interval_days: params.interval_days.unwrap_or(data.settings.interval_days),
        policy: params.policy,
        wear_hours: data.settings.wear_hours,
        lang: params.lang,
    };
    let mut out = serde_json::Map::new();
    for arch in arches {
        let mut input = schedule::input_from_stages(arch, &data.stages, params.attachments.clone());
        input.n_total = Some(total);
        let cal =
            schedule::run(&input, &cal_params).map_err(|e| CommandError::Invalid(e.to_string()))?;
        out.insert(
            arch_name(arch).to_owned(),
            serde_json::to_value(&cal).map_err(|e| CommandError::Failed(e.to_string()))?,
        );
    }
    Ok((state.clone(), Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_command_is_registered_for_the_cli_and_not_in_the_menu() {
        let spec = crate::commands::find("aligners.planning.schedule").unwrap();
        assert!(spec.schema()["properties"]["start_date"].is_object());
        assert!(crate::tree::find("aligners.planning.schedule").is_none());
    }

    #[test]
    fn it_gives_the_calendar_of_the_staged_arches() {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let (s, staged) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let n = staged["lower"]["aligners"].as_u64().unwrap();
        let (same, out) = exec(
            "aligners.planning.schedule",
            &s,
            json!({ "start_date": "2026-01-05", "interval_days": 10 }),
        )
        .unwrap();
        assert_eq!(
            same.module_data::<dca_model::AlignersState>("aligners"),
            s.module_data("aligners")
        );
        let cal = &out["lower"];
        assert_eq!(cal["n_active"], n);
        assert_eq!(cal["rows"].as_array().unwrap().len() as u64, n);
        assert_eq!(cal["rows"][0]["start"], "2026-01-05");
        assert_eq!(cal["rows"][0]["end"], "2026-01-14");
        assert_eq!(cal["total_days"].as_u64().unwrap(), 10 * n);
        assert!(out.get("upper").is_none());
        // the adaptive policy gives 7 or 14 days
        let (_, out) = exec(
            "aligners.planning.schedule",
            &s,
            json!({ "start_date": "2026-01-05", "policy": "adaptive" }),
        )
        .unwrap();
        assert!(
            out["lower"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["days"] == 7 || r["days"] == 14)
        );
        // attachments go with the first aligner
        let (_, out) = exec(
            "aligners.planning.schedule",
            &s,
            json!({ "start_date": "2026-01-05", "attachments": ["13", "23"] }),
        )
        .unwrap();
        assert_eq!(
            out["lower"]["rows"][0]["todo"][0],
            "bond the attachments (13, 23)"
        );
        assert!(matches!(
            exec(
                "aligners.planning.schedule",
                &s,
                json!({ "start_date": "soon" })
            ),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            exec("aligners.planning.schedule", &s, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn it_needs_the_staging() {
        let err = exec(
            "aligners.planning.schedule",
            &CaseState::default(),
            Value::Null,
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("no aligners")));
    }
}
