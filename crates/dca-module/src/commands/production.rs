//! The production settings of the case and where the aligner ends.
//!
//! * `aligners.settings.production` changes the production settings by
//!   group (the plugin's scene settings of the page "Produzione",
//!   `props.py:515-750`): the trim line (`trim`), the shell (`shell`), the
//!   identification text (`stamp`), the bite with the aligners worn
//!   (`bite`), the ramps (`ramps`) and the print quality (`quality`). Each
//!   number stays inside the range of the plugin's slider
//!   ([`dca_model::PRODUCTION_NUMBERS`]): a value outside it, or a case code
//!   that cannot be stamped, changes nothing. The commands of production
//!   take a parameter not given from these.
//! * `aligners.production.aligner-end` sets where the aligner of an arch
//!   ends on a side (`set`, the plugin's `oa.aligner_end`: the last tooth
//!   covered, the side its own), takes the end back to every tooth
//!   (`clear`), or lists the ends the plugin suggests (`suggest`,
//!   `case_print.aligner_suggestions`; nothing changes). The teeth behind
//!   the end stay out of the aligner: the final position keeps them at
//!   rest, the shell, the trim line, the stamp and the bite with the
//!   aligners worn leave them out. Every action returns, for each arch
//!   concerned, its end, the teeth left out, the suggestions and whether
//!   its final position was computed with another end
//!   (`setup_outdated`, the plugin's `setup_end_changed`: compute the final
//!   position and the aligners again).

use dc_case::{CaseState, Fdi, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dc_module_api::CommandSpec;
use dca_algo::teeth::Which;
use dca_model::{
    AlignersState, BiteClosing, InsertionAxis, MODULE_ID, OcclusalMode, Quality, Side, StampDate,
    StampDateFormat, StampFace, StampRead, StampSettings, StampTooth, TrimMode,
    production_out_of_range,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::messages::{invalid, tr};
use super::occlusion::aligners_data;

/// Id of the command that changes the production settings of the case.
pub(crate) const PRODUCTION_SETTINGS: &str = "aligners.settings.production";
/// Id of the command that sets where the aligner ends.
pub(crate) const ALIGNER_END: &str = "aligners.production.aligner-end";

/// The registered commands of this stage.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: PRODUCTION_SETTINGS,
            label: "aligners-cmd-production-settings",
            doc: "aligners-cmd-production-settings-doc",
            params: SettingsPatch,
            run: run_settings,
        },
        stage_command! {
            id: ALIGNER_END,
            label: "aligners-cmd-production-aligner-end",
            doc: "aligners-cmd-production-aligner-end-doc",
            params: EndParams,
            run: run_end,
        },
    ]
}

/// The trim line (the plugin's `trim_*`, `insertion_axis`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TrimPatch {
    /// Shape of the edge (`equator`, `straight`, `scalloped`, `smooth`).
    mode: Option<TrimMode>,
    /// Edge above the gingival margin, positive on the tooth (-2 to 5 mm).
    margin_gap_mm: Option<f64>,
    /// Recess between the teeth (0.6 to 2.5 mm).
    recess_mm: Option<f64>,
    /// Height of the scallops (0 to 4 mm).
    scallop_mm: Option<f64>,
    /// Edge beyond the equator (-2 to 1.5 mm).
    equator_offset_mm: Option<f64>,
    /// Edge extension (0 to 4 mm).
    extension_mm: Option<f64>,
    /// Axis of insertion (`vertical`, `common`, `per_tooth`).
    insertion_axis: Option<InsertionAxis>,
}

/// The shell (the plugin's `shell_thickness`, `clearance`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ShellPatch {
    /// Wall thickness (0.3 to 2 mm).
    shell_thickness_mm: Option<f64>,
    /// Inner clearance between tooth and shell (0 to 0.3 mm).
    clearance_mm: Option<f64>,
}

/// The identification text (the plugin's `stamp_*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct StampPatch {
    /// The aligners carry the text.
    enabled: Option<bool>,
    /// Tooth tried first (`M6R`, `M6L`, `M7R`, `M7L`, `PM5R`, `PM5L`).
    tooth: Option<StampTooth>,
    /// Face tried first (`lingual`, `buccal`).
    face: Option<StampFace>,
    /// Read from `inside` or `outside`.
    read: Option<StampRead>,
    /// Date of the text (`production`, `start`).
    date: Option<StampDate>,
    /// Format of the date (`DDMMYY`, `DD.MM.YY`, `YYMMDD`).
    date_format: Option<StampDateFormat>,
    /// Case code: at most 8 letters, digits and `-`; never the patient's name.
    code: Option<String>,
    /// Height of the characters (0.3 to 2.5 mm).
    height_mm: Option<f64>,
    /// Depth (0.02 to 0.3 mm).
    depth_mm: Option<f64>,
    /// Wall under the engraving (0.2 to 1 mm).
    min_wall_mm: Option<f64>,
}

/// The bite with the aligners worn (the plugin's `occlusal_*`, `bite_*`,
/// `hinge_*`, `equilibrate*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BitePatch {
    /// Occlusal thickness (`auto`, `manual`, `same`).
    mode: Option<OcclusalMode>,
    /// Occlusal thickness of the manual mode (0.3 to 2 mm).
    thickness_mm: Option<f64>,
    /// Smallest occlusal thickness (0.3 to 1.5 mm).
    min_thickness_mm: Option<f64>,
    /// Largest opening at the molars (0 to 5 mm).
    opening_target_mm: Option<f64>,
    /// How the mouth closes (`hinge`, `vertical`).
    closing: Option<BiteClosing>,
    /// Hinge axis: distance (40 to 120 mm).
    hinge_posterior_mm: Option<f64>,
    /// Hinge axis: height (0 to 80 mm).
    hinge_height_mm: Option<f64>,
    /// Equilibrate the contacts (virtual grinding).
    equilibrate: Option<bool>,
    /// Teeth in contact per side (1 to 6).
    equilibrate_teeth_per_side: Option<u32>,
    /// Largest grinding (0.05 to 1 mm).
    equilibrate_max_mm: Option<f64>,
}

/// The ramps (the plugin's `ramps_enabled`, `ramp_*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RampsPatch {
    /// The ramps of the case enter the aligners.
    enabled: Option<bool>,
    /// Width of a new ramp (0.5 to 8 mm).
    width_mm: Option<f64>,
    /// Depth of a new ramp (0.5 to 6 mm).
    depth_mm: Option<f64>,
    /// Height of a new ramp (0.2 to 4 mm).
    height_mm: Option<f64>,
    /// Overbite beyond which the ramps are proposed (1 to 8 mm).
    overbite_min_mm: Option<f64>,
}

/// Parameters of `aligners.settings.production`: the settings to change by
/// group (the others stay).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsPatch {
    /// The trim line.
    trim: Option<TrimPatch>,
    /// The shell.
    shell: Option<ShellPatch>,
    /// The identification text.
    stamp: Option<StampPatch>,
    /// The bite with the aligners worn.
    bite: Option<BitePatch>,
    /// The ramps.
    ramps: Option<RampsPatch>,
    /// Quality of the print files (`draft`, `light`, `standard`, `high`).
    quality: Option<Quality>,
}

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(e.to_string())
}

fn run_settings(
    state: &CaseState,
    patch: SettingsPatch,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let values = data.production_values();
    let given = serde_json::to_value(&patch).map_err(failed)?;
    let Value::Object(groups) = given else {
        return Err(failed("the parameters are not an object"));
    };
    for (group, changes) in groups {
        let mut merged = values[&group].clone();
        match changes {
            Value::Null => continue,
            Value::Object(fields) => {
                for (k, v) in fields.into_iter().filter(|(_, v)| !v.is_null()) {
                    merged[k] = v;
                }
            }
            v => merged = v,
        }
        // the sliders' ranges: a value outside them changes nothing
        if let Some((name, value, lo, hi)) = production_out_of_range(&group, &merged) {
            return Err(invalid(
                "aligners-err-production-settings-range",
                &[
                    ("name", &format!("{group}.{name}")),
                    ("value", &value.to_string()),
                    ("min", &lo.to_string()),
                    ("max", &hi.to_string()),
                ],
            ));
        }
        if group == "stamp" {
            let code = merged["code"].as_str().unwrap_or_default();
            if !StampSettings::code_ok(code) {
                return Err(invalid(
                    "aligners-err-production-stamp-code",
                    &[("code", code)],
                ));
            }
        }
        data.set_production_group(&group, merged).map_err(failed)?;
    }
    let out = data.production_values();
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, out))
}

/// What `aligners.production.aligner-end` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndAction {
    /// List the ends suggested; nothing changes.
    #[default]
    Suggest,
    /// End the aligner with tooth `fdi` on its side.
    Set,
    /// The aligner covers every tooth again (of `arch`, on `side`).
    Clear,
}

/// Parameters of `aligners.production.aligner-end`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct EndParams {
    /// `suggest` (default), `set` or `clear`.
    action: EndAction,
    /// `set`: the last tooth the aligner covers (FDI); its arch and side
    /// are its own.
    fdi: Option<u8>,
    /// `suggest` and `clear`: the arches (`all`, `upper`, `lower`).
    arch: Which,
    /// `clear`: the side (`right`, `left`; both when missing).
    side: Option<Side>,
}

fn role_name(role: ScanRole) -> &'static str {
    match role {
        ScanRole::Upper => "upper",
        _ => "lower",
    }
}

/// What the case says of the end of the aligner of the arch `role`.
fn end_report(data: &AlignersState, role: ScanRole) -> Value {
    let end = data.aligner_end_of(role);
    let outdated = data.setup_end_changed(role);
    let mut r = json!({
        "end": end,
        "out_of_aligner": data.out_of_aligner(role),
        "suggestions": data.aligner_end_suggestions(role),
        "setup_outdated": outdated,
    });
    if outdated {
        r["warning"] = json!(tr("aligners-warn-aligner-end-setup", &[]));
    }
    r
}

fn run_end(state: &CaseState, p: EndParams, ctx: &JobCtx) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let roles: Vec<ScanRole> = match p.action {
        EndAction::Set => {
            let n = p
                .fdi
                .ok_or_else(|| invalid("aligners-err-aligner-end-fdi", &[]))?;
            let f = Fdi::new(n).map_err(|_| {
                invalid("aligners-err-aligner-end-tooth", &[("fdi", &n.to_string())])
            })?;
            let role = if f.is_upper() {
                ScanRole::Upper
            } else {
                ScanRole::Lower
            };
            if !data.arch_teeth(role).contains(&f) {
                return Err(invalid(
                    "aligners-err-aligner-end-tooth",
                    &[("fdi", &n.to_string())],
                ));
            }
            let mut end = data.aligner_end_of(role);
            end.set(Side::of(f), Some(f));
            data.aligner_end.insert(role, end);
            vec![role]
        }
        EndAction::Clear | EndAction::Suggest => {
            let roles: Vec<ScanRole> = p.arch.arches().iter().map(|a| a.role()).collect();
            if p.action == EndAction::Clear {
                for role in &roles {
                    let mut end = data.aligner_end_of(*role);
                    match p.side {
                        Some(side) => end.set(side, None),
                        None => end = Default::default(),
                    }
                    if end.is_every_tooth() {
                        data.aligner_end.remove(role);
                    } else {
                        data.aligner_end.insert(*role, end);
                    }
                }
            }
            roles
        }
    };
    let mut out = Map::new();
    for role in roles {
        out.insert(role_name(role).to_owned(), end_report(&data, role));
    }
    let mut next = state.clone();
    if p.action != EndAction::Suggest {
        next.set_module(MODULE_ID, ModuleState::new(data));
    }
    Ok((next, Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::Pose;
    use dca_model::{Seed, SetupSummary};
    use glam::DVec3;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    fn with_teeth(teeth: &[u8]) -> CaseState {
        let mut state = CaseState::default();
        let d = AlignersState {
            seeds: teeth
                .iter()
                .map(|&n| Seed {
                    fdi: fdi(n),
                    position: DVec3::ZERO,
                })
                .collect(),
            ..AlignersState::default()
        };
        state.set_module(MODULE_ID, ModuleState::new(d));
        state
    }

    #[test]
    fn the_commands_are_registered_with_their_schema() {
        for id in [PRODUCTION_SETTINGS, ALIGNER_END] {
            let spec = crate::commands::find(id).unwrap();
            let schema = (spec.params_schema)();
            assert!(schema["properties"].is_object(), "{id}");
        }
        let schema = (crate::commands::find(PRODUCTION_SETTINGS)
            .unwrap()
            .params_schema)();
        for g in dca_model::PRODUCTION_GROUPS {
            assert!(schema["properties"][g].is_object(), "{g}");
        }
    }

    #[test]
    fn the_settings_change_inside_their_ranges_and_out_of_range_changes_nothing() {
        let state = CaseState::default();
        let (next, out) = exec(
            PRODUCTION_SETTINGS,
            &state,
            json!({
                "trim": { "mode": "equator", "margin_gap_mm": -1.0, "insertion_axis": "common" },
                "shell": { "shell_thickness_mm": 0.8 },
                "stamp": { "tooth": "PM5L", "code": "AB-12", "date_format": "DD.MM.YY" },
                "bite": { "mode": "manual", "thickness_mm": 0.6, "equilibrate_teeth_per_side": 3 },
                "ramps": { "enabled": true, "width_mm": 4.0 },
                "quality": "light",
            }),
        )
        .unwrap();
        let d = data(&next);
        assert_eq!(d.trim.mode, TrimMode::Equator);
        assert_eq!(d.trim.margin_gap_mm, -1.0);
        assert_eq!(d.trim.insertion_axis, InsertionAxis::Common);
        assert_eq!(d.trim.recess_mm, 1.6);
        assert_eq!(d.settings.shell_thickness_mm, 0.8);
        assert_eq!(d.settings.clearance_mm, 0.0);
        assert_eq!(d.settings.stamp.tooth, StampTooth::SecondPremolarLeft);
        assert_eq!(d.settings.stamp.code, "AB-12");
        assert_eq!(d.settings.stamp.date_format, StampDateFormat::DotDdMmYy);
        assert!(d.settings.stamp.enabled);
        assert_eq!(d.aligner_bite.occlusal.mode, OcclusalMode::Manual);
        assert_eq!(d.aligner_bite.occlusal.equilibrate_teeth_per_side, 3);
        assert!(d.aligner_bite.ramps.enabled);
        assert_eq!(d.aligner_bite.ramps.width_mm, 4.0);
        assert_eq!(d.settings.quality, Quality::Light);
        assert_eq!(out, d.production_values());
        assert_eq!(out["quality"], "light");

        for bad in [
            json!({ "trim": { "margin_gap_mm": 5.5 } }),
            json!({ "shell": { "clearance_mm": 0.31 } }),
            json!({ "stamp": { "depth_mm": 0.01 } }),
            json!({ "stamp": { "code": "TOO-LONG-1" } }),
            json!({ "stamp": { "code": "A B" } }),
            json!({ "bite": { "hinge_posterior_mm": 30.0 } }),
            json!({ "bite": { "equilibrate_teeth_per_side": 7 } }),
            json!({ "ramps": { "overbite_min_mm": 9.0 } }),
            json!({ "quality": "ultra" }),
            json!({ "trim": { "nothing": 1.0 } }),
            json!({ "other": {} }),
            // a valid group with an invalid one: nothing changes
            json!({ "shell": { "shell_thickness_mm": 1.2 }, "ramps": { "depth_mm": 0.1 } }),
        ] {
            assert!(
                matches!(
                    exec(PRODUCTION_SETTINGS, &next, bad.clone()),
                    Err(CommandError::Invalid(_))
                ),
                "{bad}"
            );
        }
        // null changes nothing and gives the values
        let (same, out) = exec(PRODUCTION_SETTINGS, &next, Value::Null).unwrap();
        assert_eq!(data(&same), d);
        assert_eq!(out["shell"]["shell_thickness_mm"], 0.8);
    }

    #[test]
    fn the_settings_survive_the_case_file() {
        let (next, _) = exec(
            PRODUCTION_SETTINGS,
            &CaseState::default(),
            json!({ "stamp": { "enabled": false, "read": "outside" }, "quality": "high",
                    "trim": { "insertion_axis": "vertical" } }),
        )
        .unwrap();
        let d = data(&next).clone();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["settings"]["quality"], "high");
        assert_eq!(v["trim"]["insertion_axis"], "vertical");
        let back: AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
        // a case saved before them gets the plugin's defaults
        let mut v = serde_json::to_value(AlignersState::default()).unwrap();
        v["settings"].as_object_mut().unwrap().remove("stamp");
        v["settings"].as_object_mut().unwrap().remove("quality");
        v["trim"].as_object_mut().unwrap().remove("insertion_axis");
        let old: AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(old.settings.quality, Quality::Standard);
        assert_eq!(old.settings.stamp, StampSettings::default());
        assert_eq!(old.trim.insertion_axis, InsertionAxis::PerTooth);
    }

    #[test]
    fn the_end_is_set_on_the_side_of_the_tooth_and_cleared() {
        let state = with_teeth(&[31, 32, 33, 34, 35, 36, 37, 41, 42, 43, 44, 45, 46, 47, 16]);
        let (next, out) = exec(ALIGNER_END, &state, json!({ "action": "set", "fdi": 46 })).unwrap();
        let end = data(&next).aligner_end_of(ScanRole::Lower);
        assert_eq!((end.right, end.left), (Some(fdi(46)), None));
        assert_eq!(out["lower"]["out_of_aligner"], json!([47]));
        assert_eq!(out["lower"]["setup_outdated"], false);
        assert!(out.get("upper").is_none());
        let (next, _) = exec(ALIGNER_END, &next, json!({ "action": "set", "fdi": 36 })).unwrap();
        assert_eq!(
            data(&next).out_of_aligner(ScanRole::Lower),
            [fdi(37), fdi(47)]
        );
        // clear one side
        let (one, _) = exec(
            ALIGNER_END,
            &next,
            json!({ "action": "clear", "arch": "lower", "side": "left" }),
        )
        .unwrap();
        assert_eq!(data(&one).out_of_aligner(ScanRole::Lower), [fdi(47)]);
        // clear every end: the entry goes
        let (all, out) = exec(ALIGNER_END, &next, json!({ "action": "clear" })).unwrap();
        assert!(data(&all).aligner_end.is_empty());
        assert!(out["upper"].is_object() && out["lower"].is_object());
        // a tooth that is not in the arch, or no tooth, is refused
        for bad in [
            json!({ "action": "set", "fdi": 26 }),
            json!({ "action": "set", "fdi": 99 }),
            json!({ "action": "set" }),
            json!({ "action": "set", "fdi": 46, "side": "up" }),
        ] {
            assert!(
                matches!(
                    exec(ALIGNER_END, &state, bad.clone()),
                    Err(CommandError::Invalid(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn suggest_lists_the_plugins_suggestions_and_changes_nothing() {
        // 35 missing with 36 and 37 behind it
        let state = with_teeth(&[31, 32, 33, 34, 36, 37, 41, 42, 43, 44, 45, 46]);
        let (next, out) = exec(ALIGNER_END, &state, json!({ "arch": "lower" })).unwrap();
        assert_eq!(data(&next), data(&state));
        assert_eq!(
            out["lower"]["suggestions"],
            json!([{ "side": "left", "last": 34, "missing": 35 }])
        );
        // following it leaves 36 and 37 out and nothing more is suggested
        let (next, out) = exec(ALIGNER_END, &state, json!({ "action": "set", "fdi": 34 })).unwrap();
        assert_eq!(out["lower"]["out_of_aligner"], json!([36, 37]));
        assert_eq!(out["lower"]["suggestions"], json!([]));
        assert_eq!(data(&next).aligner_teeth(ScanRole::Lower).len(), 10);
    }

    #[test]
    fn a_setup_computed_with_another_end_is_reported() {
        let mut state = with_teeth(&[41, 42, 43, 44, 45, 46, 47]);
        let mut d = data(&state).clone();
        d.setup
            .results
            .insert(ScanRole::Lower, SetupSummary::default());
        d.setup.poses.insert(fdi(41), Pose::IDENTITY);
        state.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(ALIGNER_END, &state, json!({ "action": "set", "fdi": 46 })).unwrap();
        assert_eq!(out["lower"]["setup_outdated"], true);
        assert!(
            out["lower"]["warning"]
                .as_str()
                .is_some_and(|w| !w.is_empty())
        );
        // back to every tooth: as the setup was computed
        let (_, out) = exec(ALIGNER_END, &next, json!({ "action": "clear" })).unwrap();
        assert_eq!(out["lower"]["setup_outdated"], false);
    }
}
