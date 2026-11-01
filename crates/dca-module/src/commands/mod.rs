//! The registry of the module's agent-drivable commands ([`CommandSpec`]).
//!
//! One file per pipeline stage (`commands/<stage>.rs`), each with a
//! `specs()` function returning the commands of that stage, built with
//! [`stage_command!`]. [`all`] concatenates them; it is what
//! [`dc_module_api::Module::command_specs`] returns and what the CLI and
//! the MCP server list.
//!
//! The registry also drives the menu: a menu leaf is `Ready` if and only if
//! a command with the leaf's id is registered here, `Stub` otherwise (see
//! [`crate::tree::Node::status`]). Most commands are not menu leaves: the
//! menu holds what the end user needs, and every command stays here, listed
//! and run by the CLI and the MCP server. A command that is not a leaf has
//! its own `aligners-cmd-<stage>-<name>` label.
//!
//! Adding a command to a stage:
//!
//! ```ignore
//! // commands/segmentation.rs
//! pub(crate) fn specs() -> Vec<CommandSpec> {
//!     vec![stage_command! {
//!         id: "aligners.preparation.segmentation.automatic",
//!         label: "aligners-menu-preparation-segmentation-automatic",
//!         doc: "aligners-cmd-segmentation-automatic-doc",
//!         params: dca_algo::segmentation::Params,
//!         run: run_automatic,
//!     }]
//! }
//!
//! fn run_automatic(
//!     state: &CaseState,
//!     params: dca_algo::segmentation::Params,
//!     ctx: &JobCtx,
//! ) -> Result<CommandOutput, CommandError> { ... }
//! ```
//!
//! `Params` derives `serde::Deserialize`, `schemars::JsonSchema` and
//! `Default`, and should carry `#[serde(default)]` so that missing fields
//! take their default value. The label and doc keys must exist in both
//! `i18n/en/dca-module.ftl` and `i18n/it/dca-module.ftl` (under the stage's
//! section); a test checks it.

use std::sync::OnceLock;

use dc_command::CommandError;
use dc_module_api::CommandSpec;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Builds the [`CommandSpec`] of a stage command from a typed parameter
/// struct and a typed run function.
///
/// - `id`: command id (a menu leaf id to make that leaf `Ready`);
/// - `label`, `doc`: i18n keys (en and it);
/// - `params`: type deriving `Deserialize + JsonSchema + Default`;
/// - `run`: `fn(&CaseState, Params, &JobCtx) -> Result<CommandOutput, CommandError>`.
///
/// The parameters arrive as JSON: `null` means all defaults, an object is
/// deserialized into `params` (an invalid one is [`CommandError::Invalid`]).
#[allow(unused_macros)] // used by the stage files as the stages land
macro_rules! stage_command {
    (
        id: $id:expr,
        label: $label:expr,
        doc: $doc:expr,
        params: $params:ty,
        run: $run:expr $(,)?
    ) => {
        ::dc_module_api::CommandSpec {
            id: $id,
            label_key: $label,
            doc_key: $doc,
            params_schema: || ::schemars::schema_for!($params).to_value(),
            run: |state, params, ctx| {
                let params: $params = $crate::commands::parse_params(params)?;
                ($run)(state, params, ctx)
            },
        }
    };
}

mod auto;
mod autoseed;
mod bite;
mod demo;
mod diagnosis;
mod export;
mod frame;
mod gingiva;
pub mod import;
mod manual;
pub(crate) mod messages;
mod occlusion;
mod ramps;
mod reconstruct;
mod schedule;
mod segmentation;
mod setup;
mod shell;
mod staging;
mod stamp;
pub(crate) mod study_base;
mod teeth;
mod trim;
mod visits;
pub(crate) mod wrappers;

/// Every registered command, stage by stage in pipeline order.
pub fn all() -> Vec<CommandSpec> {
    [
        import::specs,
        frame::specs,
        demo::specs,
        autoseed::specs,
        segmentation::specs,
        teeth::specs,
        reconstruct::specs,
        gingiva::specs,
        diagnosis::specs,
        setup::specs,
        staging::specs,
        schedule::specs,
        occlusion::specs,
        bite::specs,
        ramps::specs,
        shell::specs,
        trim::specs,
        stamp::specs,
        export::specs,
        study_base::specs,
        visits::specs,
        manual::specs,
        wrappers::specs,
        auto::specs,
    ]
    .iter()
    .flat_map(|specs| specs())
    .collect()
}

/// The registry, built once.
fn registry() -> &'static [CommandSpec] {
    static REGISTRY: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    REGISTRY.get_or_init(all)
}

/// The registered command with this id.
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    registry().iter().find(|c| c.id == id)
}

/// `true` when a command with this id is registered.
pub fn is_registered(id: &str) -> bool {
    find(id).is_some()
}

/// Reads the JSON parameters of a command: `null` gives the defaults,
/// anything else is deserialized into `P`.
pub fn parse_params<P: DeserializeOwned + Default>(params: &Value) -> Result<P, CommandError> {
    if params.is_null() {
        return Ok(P::default());
    }
    serde_json::from_value(params.clone())
        .map_err(|e| CommandError::Invalid(format!("parameters: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::CaseState;
    use dc_command::JobCtx;
    use dc_module_api::CommandOutput;
    use serde_json::json;

    #[derive(Debug, Default, PartialEq, serde::Deserialize, schemars::JsonSchema)]
    #[serde(default, deny_unknown_fields)]
    struct DemoParams {
        /// How many times.
        count: u32,
        /// Optional label.
        label: Option<String>,
    }

    fn run_demo(
        state: &CaseState,
        params: DemoParams,
        _ctx: &JobCtx,
    ) -> Result<CommandOutput, CommandError> {
        Ok((
            state.clone(),
            json!({ "count": params.count, "label": params.label }),
        ))
    }

    fn demo_spec() -> CommandSpec {
        stage_command! {
            id: "aligners.test.demo",
            label: "aligners-cmd-test-demo",
            doc: "aligners-cmd-test-demo-doc",
            params: DemoParams,
            run: run_demo,
        }
    }

    #[test]
    fn stage_command_builds_a_working_spec() {
        let spec = demo_spec();
        assert_eq!(spec.id, "aligners.test.demo");
        assert_eq!(spec.label_key, "aligners-cmd-test-demo");
        assert_eq!(spec.doc_key, "aligners-cmd-test-demo-doc");

        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["count"].is_object(), "{schema}");

        let ctx = JobCtx::detached();
        let state = CaseState::default();
        let (_, out) = spec.execute(&state, &Value::Null, &ctx).unwrap();
        assert_eq!(out, json!({ "count": 0, "label": null }));
        let (_, out) = spec.execute(&state, &json!({ "count": 3 }), &ctx).unwrap();
        assert_eq!(out["count"], 3);
        assert!(matches!(
            spec.execute(&state, &json!({ "count": "x" }), &ctx),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            spec.execute(&state, &json!({ "nope": 1 }), &ctx),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn parse_params_defaults_on_null() {
        let p: DemoParams = parse_params(&Value::Null).unwrap();
        assert_eq!(p, DemoParams::default());
        let p: DemoParams = parse_params(&json!({})).unwrap();
        assert_eq!(p, DemoParams::default());
    }

    #[test]
    fn registry_lookups_agree_with_all() {
        let all = all();
        for c in &all {
            assert!(is_registered(c.id));
            assert_eq!(find(c.id).map(|f| f.id), Some(c.id));
        }
        assert!(!is_registered("aligners.nope"));
    }
}
