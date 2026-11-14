//! `aligners.planning.setup.line`: the arch line the final position aligns
//! the prominent cusps on, drawn by the clinician (the plugin's
//! `oa.arch_line_apply` and `oa.arch_line_reset`, `case_plan.store_arch_line`
//! and `reset_arch_line`).
//!
//! - `points` (at least two, case coordinates, in order along the arch)
//!   become the drawn line of the arch, stored in `Setup::drawn`: every
//!   later computation of the final position aligns the cusps on it instead
//!   of the automatic line, as long as it is there.
//! - No `points`: back to the automatic line (the drawn one is dropped).
//! - `recompute` (default, as the plugin's Apply and Reset): the final
//!   position of the arch is computed again at once, with the parameters in
//!   force, in the same undo step; the hand corrections of the moved teeth
//!   are lost as with any new computation. Without it the line is only
//!   stored, for the next computation ("save without recompute").
//!
//! Showing and hiding the line, and moving its points, belong to the view;
//! the stored line and the line of the last computation
//! (`Setup::lines`) are what it shows.

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_model::{AlignersState, MODULE_ID};
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::messages;

/// Command id.
pub(crate) const LINE: &str = "aligners.planning.setup.line";

/// The registered commands of this file.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: LINE,
        label: "aligners-menu-planning-setup-line",
        doc: "aligners-cmd-setup-line-doc",
        params: LineParams,
        run: run_line,
    }]
}

/// Parameters of `aligners.planning.setup.line`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LineParams {
    /// The arch: upper or lower (default: the only arch with tooth colours).
    pub arch: Option<Arch>,
    /// The drawn arch line: at least two points [x, y, z] in case
    /// coordinates (mm), in order along the arch. Empty: back to the
    /// automatic line.
    pub points: Vec<[f64; 3]>,
    /// Compute the final position of the arch again at once; `false` only
    /// stores the line, used at the next computation.
    pub recompute: bool,
}

impl Default for LineParams {
    fn default() -> Self {
        Self {
            arch: None,
            points: Vec::new(),
            recompute: true,
        }
    }
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The arch given, else the only one with tooth colours.
fn line_arch(data: &AlignersState, given: Option<Arch>) -> Result<Arch, CommandError> {
    if let Some(arch) = given {
        return Ok(arch);
    }
    let coloured: Vec<Arch> = [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter(|a| data.labels.contains_key(&a.role()))
        .collect();
    match coloured.as_slice() {
        [one] => Ok(*one),
        [] => Err(messages::none_segmented()),
        _ => Err(messages::invalid("aligners-err-setup-line-arch", &[])),
    }
}

fn run_line(
    state: &CaseState,
    params: LineParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = state
        .module_data::<AlignersState>(MODULE_ID)
        .cloned()
        .ok_or_else(messages::no_case_data)?;
    let arch = line_arch(&data, params.arch)?;
    let role = arch.role();
    if state.case.scan(role).is_none() {
        return Err(messages::no_scan(arch));
    }
    let drawn = !params.points.is_empty();
    if drawn && (params.points.len() < 2 || params.points.iter().flatten().any(|v| !v.is_finite()))
    {
        return Err(messages::invalid("aligners-err-setup-line-points", &[]));
    }
    if drawn {
        let line: Vec<DVec3> = params.points.iter().map(|&p| DVec3::from(p)).collect();
        data.setup.drawn.insert(role, line);
    } else {
        data.setup.drawn.remove(&role);
    }
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    let name = arch_name(arch);
    let mut report = json!({
        "arch": name,
        "line": if drawn { "drawn" } else { "automatic" },
        "points": params.points.len(),
        "recomputed": params.recompute,
    });
    if !params.recompute {
        return Ok((next, report));
    }
    // `line: []`: the setup takes the line just stored (or the automatic one)
    let (after, out) = super::setup::run_resolved(&next, json!({ "arch": name, "line": [] }), ctx)?;
    report["setup"] = out[name].clone();
    Ok((after, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;
    use serde_json::Value;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn data(state: &CaseState) -> &AlignersState {
        state.module_data::<AlignersState>(MODULE_ID).unwrap()
    }

    /// A lower demo arch with its final position computed.
    fn set_up() -> CaseState {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower", "severity": "moderate" }),
        )
        .unwrap()
        .0;
        exec("aligners.planning.setup.automatic", &s, Value::Null)
            .unwrap()
            .0
    }

    /// The automatic line of the lower arch moved 1.5 mm towards the
    /// incisors (case coordinates).
    fn moved_line(state: &CaseState) -> Vec<[f64; 3]> {
        let ahead = state
            .case
            .world_frame
            .from_case
            .inverse()
            .apply_vector(dc_case::WorldFrame::ANTERIOR);
        data(state).setup.lines[&ScanRole::Lower]
            .iter()
            .map(|p| (*p + 1.5 * ahead).to_array())
            .collect()
    }

    fn drawn_note(out: &Value) -> bool {
        out["setup"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "arch_line" && n["drawn"] == true)
    }

    #[test]
    fn the_command_is_registered_as_a_tool_of_the_final_position() {
        let spec = crate::commands::find(LINE).unwrap();
        let schema = spec.schema();
        for key in ["arch", "points", "recompute"] {
            assert!(schema["properties"][key].is_object(), "{key}");
        }
        assert!(crate::tree::tools().iter().any(|n| n.id == LINE));
    }

    #[test]
    fn it_refuses_a_line_that_is_not_one() {
        let s = set_up();
        for bad in [
            json!({ "points": [[0.0, 0.0, 0.0]] }),
            json!({ "points": [[0.0, 0.0, 0.0], [f64::NAN, 0.0, 0.0]] }),
            json!({ "arch": "upper", "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] }),
            json!({ "arch": "middle" }),
            json!({ "unknown": 1 }),
        ] {
            assert!(
                matches!(exec(LINE, &s, bad.clone()), Err(CommandError::Invalid(_))),
                "{bad}"
            );
        }
        assert!(exec(LINE, &CaseState::default(), Value::Null).is_err());
    }

    #[test]
    fn a_drawn_line_is_stored_and_the_final_position_follows_it() {
        let s = set_up();
        let before = data(&s).setup.clone();
        let line = moved_line(&s);
        let (next, out) = exec(LINE, &s, json!({ "points": line })).unwrap();
        assert_eq!(out["arch"], "lower");
        assert_eq!(out["line"], "drawn");
        assert_eq!(out["recomputed"], true);
        assert!(drawn_note(&out), "{out}");
        let after = &data(&next).setup;
        let stored: Vec<[f64; 3]> = after.drawn[&ScanRole::Lower]
            .iter()
            .map(|p| p.to_array())
            .collect();
        assert_eq!(stored, line);
        // computed again on the line moved forward: the teeth follow it
        assert_ne!(after.poses, before.poses);
        assert_ne!(after.lines, before.lines);
        // a later computation keeps aligning on the drawn line
        let (again, out) = exec("aligners.planning.setup.automatic", &next, Value::Null).unwrap();
        assert!(
            out["lower"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["kind"] == "arch_line" && n["drawn"] == true)
        );
        assert_eq!(data(&again).setup.poses, after.poses);
    }

    #[test]
    fn without_recompute_the_line_is_only_stored() {
        let s = set_up();
        let line = moved_line(&s);
        let (next, out) = exec(LINE, &s, json!({ "points": line, "recompute": false })).unwrap();
        assert_eq!(out["recomputed"], false);
        assert!(out.get("setup").is_none());
        let (a, b) = (&data(&s).setup, &data(&next).setup);
        assert_eq!(a.poses, b.poses);
        assert_eq!(a.lines, b.lines);
        assert_eq!(b.drawn[&ScanRole::Lower].len(), line.len());
        // the next computation takes it
        let (_, out) = exec("aligners.planning.setup.automatic", &next, Value::Null).unwrap();
        assert!(
            out["lower"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["kind"] == "arch_line" && n["drawn"] == true)
        );
    }

    #[test]
    fn back_to_the_automatic_line() {
        let s = set_up();
        let auto = data(&s).setup.clone();
        let (drawn, _) = exec(LINE, &s, json!({ "points": moved_line(&s) })).unwrap();
        let (back, out) = exec(LINE, &drawn, json!({ "arch": "lower" })).unwrap();
        assert_eq!(out["line"], "automatic");
        assert!(!drawn_note(&out));
        let b = &data(&back).setup;
        assert!(b.drawn.is_empty());
        // the same final position as before the drawing
        assert_eq!(b.poses, auto.poses);
        assert_eq!(b.lines, auto.lines);
    }

    #[test]
    fn the_drawn_line_moves_with_its_arch_and_goes_with_its_scan() {
        let s = set_up();
        let line = moved_line(&s);
        let (next, _) = exec(LINE, &s, json!({ "points": line, "recompute": false })).unwrap();
        // the scan of the arch moved by hand: the drawn line goes with it
        let (moved, _) = exec(
            "aligners.components.transform",
            &next,
            json!({ "component": "scan", "arch": "lower", "move": { "t": [0.0, 1.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] } }),
        )
        .unwrap();
        let delta = data(&moved)
            .components
            .scan_move(&moved.case.scan(ScanRole::Lower).unwrap().id.to_string());
        let (a, b) = (
            &data(&next).setup.drawn[&ScanRole::Lower],
            &data(&moved).setup.drawn[&ScanRole::Lower],
        );
        for (p, q) in a.iter().zip(b) {
            assert!((delta.apply_point(*p) - *q).length() < 1e-9);
        }
        assert!((a[0] - b[0]).length() > 0.5);
        // a scan replaced takes the line drawn on the old one away
        let mut d = data(&next).clone();
        assert!(d.clear_arch(ScanRole::Lower).contains(&"setup"));
        assert!(d.setup.drawn.is_empty());
    }
}
