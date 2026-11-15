//! Commands of the `autoseed` stage (`dca_algo::autoseed`).
//!
//! * `aligners.preparation.orientation.automatic` puts the patient on the
//!   world axes, like `aligners.frame`, and stores the transform in
//!   `case.world_frame`; without seeds on the reference arch the arch frame
//!   comes from the crowns of the scan (the plugin's automatic arch frame)
//!   instead of the shape-only estimate. The scans are not modified.
//! * `aligners.preparation.numbering.automatic` proposes one seed per tooth
//!   on each arch scan (`case_auto.auto_seeds` of the plugin) and replaces
//!   the seeds of those arches in the Aligners data. The proposal must be
//!   checked by the user. With `arch: all` each arch is numbered on its own,
//!   as the plugin numbers each arch object: an arch that fails keeps what
//!   it had and is reported in `failed`, the others keep their new seeds; it
//!   fails only when every arch fails. The run reads the teeth known to be
//!   missing and does not store them: the plugin's proposal only reads its
//!   list (`parse_missing(s.missing_teeth)`), and the menu leaf passes the
//!   list of the case settings.
//!
//! Both refuse a scan still marked provisional: its arch is a guess, and what
//! they compute depends on it (`aligners.case.detect-arch` settles it).

use dc_case::{CaseState, Fdi, ModuleState, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::autoseed::{self, AutoseedError};
use dca_algo::frame::{self, Reference};
use dca_model::{AlignersState, MODULE_ID};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::import::require_settled;

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.preparation.orientation.automatic",
            label: "aligners-cmd-autoseed-orientation",
            doc: "aligners-cmd-autoseed-orientation-doc",
            params: frame::Params,
            run: run_orientation,
        },
        stage_command! {
            id: "aligners.preparation.numbering.automatic",
            label: "aligners-cmd-autoseed-numbering",
            doc: "aligners-cmd-autoseed-numbering-doc",
            params: NumberingParams,
            run: run_numbering,
        },
        stage_command! {
            id: FIND_SETTINGS,
            label: "aligners-cmd-find-teeth-settings",
            doc: "aligners-cmd-find-teeth-settings-doc",
            params: FindSettingsPatch,
            run: run_find_settings,
        },
    ]
}

/// Id of the command that changes the settings of "Find the teeth".
pub(crate) const FIND_SETTINGS: &str = "aligners.settings.find-teeth";

/// Parameters of `aligners.settings.find-teeth`: the fields to change.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FindSettingsPatch {
    /// Look for the third molars too.
    third_molars: Option<bool>,
    /// FDI numbers of the teeth known to be missing (replaces the list).
    missing: Option<Vec<u8>>,
}

/// Which arch scans get automatic seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Arches {
    /// Every arch scan of the case.
    #[default]
    All,
    /// The upper scan.
    Upper,
    /// The lower scan.
    Lower,
}

/// Parameters of `aligners.preparation.numbering.automatic`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NumberingParams {
    /// Which arch scans (default: all of them).
    pub arch: Arches,
    /// Third molars may be present (default false).
    pub third_molars: bool,
    /// FDI numbers of the teeth known to be missing.
    pub missing: Vec<u8>,
    /// Forced distal tooth of the right and left side, as tooth positions
    /// 5..=8 (e.g. `[7, 7]`); default: chosen by the data.
    pub ends: Option<[u8; 2]>,
}

/// The error in the words of the app; `arch` is the scan it happened on.
fn seed_error(arch: Option<Arch>, e: AutoseedError) -> CommandError {
    crate::commands::messages::autoseed_error(arch, &e)
}

/// The Aligners data of the case (new ones when there are none).
fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

fn run_orientation(
    state: &CaseState,
    params: frame::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let has = |role| state.case.scan(role).is_some();
    let arch = params
        .reference
        .choose(has(ScanRole::Upper), has(ScanRole::Lower))
        .ok_or_else(|| match params.reference {
            Reference::Auto => crate::commands::messages::no_scan_any(),
            Reference::Upper => crate::commands::messages::no_scan(Arch::Upper),
            Reference::Lower => crate::commands::messages::no_scan(Arch::Lower),
        })?;
    let scan = state
        .case
        .scan(arch.role())
        .ok_or_else(crate::commands::messages::reference_missing)?;
    require_settled(scan)?;
    let data = aligners_data(state)?;
    // the scan as it was read: the side of the teeth comes from its open border
    let mesh = crate::open_scan::open_scan(state, &data, &scan.mesh)?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let input = frame::Input {
        arch,
        vertices: &vertices,
        seeds: &data.seeds,
        occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
    };
    let out = autoseed::orient(&input, &mesh.tris).map_err(|e| seed_error(Some(arch), e))?;
    ctx.check_cancelled()?;
    let mut next = state.clone();
    next.case.world_frame.from_case = out.from_case;
    let json = serde_json::to_value(out).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((next, json))
}

fn run_numbering(
    state: &CaseState,
    params: NumberingParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    // the teeth marked missing by hand (the manual tools) stay missing
    let mut missing = params.missing.clone();
    missing.extend(aligners_data(state)?.missing.iter().map(|f| f.get()));
    missing.sort_unstable();
    missing.dedup();
    let algo = autoseed::Params {
        third_molars: params.third_molars,
        missing,
        ends: params.ends,
    };
    // reject bad parameters before any work
    algo.options().map_err(|e| seed_error(None, e))?;
    if params
        .ends
        .is_some_and(|e| e.iter().any(|x| !(5..=8).contains(x)))
    {
        return Err(seed_error(None, AutoseedError::BadEnds));
    }
    let arches: Vec<Arch> = match params.arch {
        Arches::All => [Arch::Upper, Arch::Lower]
            .into_iter()
            .filter(|a| state.case.scan(a.role()).is_some())
            .collect(),
        Arches::Upper => vec![Arch::Upper],
        Arches::Lower => vec![Arch::Lower],
    };
    if arches.is_empty() {
        return Err(crate::commands::messages::no_scan_any());
    }
    for scan in arches.iter().filter_map(|a| state.case.scan(a.role())) {
        require_settled(scan)?;
    }
    // several arches: each one on its own, a failure is reported and the
    // others go on (one arch asked for: its failure is the error)
    let each_on_its_own = arches.len() > 1;
    let mut data = aligners_data(state)?;
    let mut report = Vec::new();
    let mut failed: Vec<(Arch, CommandError)> = Vec::new();
    for arch in arches {
        let scan = state
            .case
            .scan(arch.role())
            .ok_or_else(|| crate::commands::messages::no_scan(arch))?;
        // the scan as it was read: the side of the teeth comes from its open border
        let mesh = crate::open_scan::open_scan(state, &data, &scan.mesh)?;
        let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let input = autoseed::Input {
            arch,
            vertices: &vertices,
            triangles: &mesh.tris,
        };
        let out = match autoseed::run(&input, &algo) {
            Ok(out) => out,
            Err(e) if each_on_its_own => {
                ctx.check_cancelled()?;
                failed.push((arch, seed_error(Some(arch), e)));
                continue;
            }
            Err(e) => return Err(seed_error(Some(arch), e)),
        };
        ctx.check_cancelled()?;
        // new seeds make the colours, the 3D teeth and everything planned
        // from them out of date: they go (the plugin marks them to redo)
        let dropped = data.replace_arch_seeds(arch.role(), &out.seeds);
        report.push(json!({
            "arch": arch,
            "dropped": dropped,
            "teeth": out.seeds.iter().map(|s| s.fdi.get()).collect::<Vec<_>>(),
            "arch_frame": out.arch_frame,
            "diagnostics": out.diagnostics,
        }));
    }
    if report.is_empty()
        && let Some((_, first)) = failed.first()
    {
        // every arch failed: one error with the reason of each
        let text = failed
            .iter()
            .map(|(_, e)| error_text(e))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(match first {
            CommandError::Invalid(_) => CommandError::Invalid(text),
            _ => CommandError::Failed(text),
        });
    }
    let mut next = state.clone();
    // the blobs of the dropped colours, solids and gingiva go too
    let before = state.referenced_blobs();
    next.set_module(MODULE_ID, ModuleState::new(data));
    let after = next.referenced_blobs();
    let stale: Vec<_> = before.difference(&after).copied().collect();
    crate::manual::common::finish_blobs(&mut next, &stale);
    let failed: Vec<_> = failed
        .iter()
        .map(|(arch, e)| json!({ "arch": arch, "error": error_text(e) }))
        .collect();
    Ok((next, json!({ "arches": report, "failed": failed })))
}

/// The sentence of an error, without the prefix of its kind.
fn error_text(e: &CommandError) -> String {
    match e {
        CommandError::Invalid(m) | CommandError::Failed(m) | CommandError::NotAvailable(m) => {
            m.clone()
        }
        other => other.to_string(),
    }
}

fn run_find_settings(
    state: &CaseState,
    patch: FindSettingsPatch,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    if let Some(list) = patch.missing {
        if let Some(bad) = list.iter().find(|n| !permanent(**n)) {
            return Err(seed_error(None, AutoseedError::BadTooth(*bad)));
        }
        let mut list = list;
        list.sort_unstable();
        list.dedup();
        data.settings.find_teeth.missing = list;
    }
    if let Some(on) = patch.third_molars {
        data.settings.find_teeth.third_molars = on;
    }
    let find = data.settings.find_teeth.clone();
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((
        next,
        json!({ "third_molars": find.third_molars, "missing": find.missing }),
    ))
}

/// `true` for the FDI number of a permanent tooth (11..=18, 21..=28, 31..=38,
/// 41..=48).
pub(crate) fn permanent(n: u8) -> bool {
    Fdi::new(n).is_ok_and(|f| Fdi::PERMANENT.contains(&f))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dc_case::RoleSource;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn numbering(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        exec("aligners.preparation.numbering.automatic", state, params)
    }

    fn orientation(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        exec("aligners.preparation.orientation.automatic", state, params)
    }

    fn demo_state(arch: &str, seed: u64) -> CaseState {
        exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": arch, "seed": seed }),
        )
        .unwrap()
        .0
    }

    fn seeds_of(state: &CaseState) -> Vec<dca_model::Seed> {
        state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .seeds
            .clone()
    }

    #[test]
    fn an_empty_case_has_nothing_to_number_or_orient() {
        let empty = CaseState::default();
        assert!(matches!(
            numbering(&empty, Value::Null),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            orientation(&empty, Value::Null),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_scans_of_the_demo_are_not_provisional() {
        let state = demo_state("lower", 7);
        assert_eq!(state.case.scans[0].role_source, RoleSource::Explicit);
        assert!(numbering(&state, Value::Null).is_ok());
        assert!(orientation(&state, Value::Null).is_ok());
    }

    #[test]
    fn a_provisional_scan_is_refused_with_the_way_out() {
        let mut state = demo_state("lower", 7);
        state.case.scans[0].role_source = RoleSource::Provisional;
        for run in [numbering, orientation] {
            let Err(CommandError::Invalid(m)) = run(&state, Value::Null) else {
                panic!("not refused");
            };
            assert!(m.contains("demo-lower"), "{m}");
            assert!(m.contains("press Confirm"), "{m}");
            assert!(!m.contains("aligners.") && !m.contains("case."), "{m}");
        }
        // the arch step settles it, and both run
        let next = exec("aligners.case.detect-arch", &state, Value::Null)
            .unwrap()
            .0;
        assert_eq!(next.case.scans[0].role_source, RoleSource::Detected);
        assert!(numbering(&next, Value::Null).is_ok());
        assert!(orientation(&next, Value::Null).is_ok());
        // a detected scan is fine, and so is one that is not asked for
        let both = exec(
            "aligners.demo",
            &next,
            json!({ "arch": "upper", "seed": 3 }),
        )
        .unwrap()
        .0;
        let mut one = both.clone();
        let upper = one
            .case
            .scans
            .iter_mut()
            .find(|s| s.role == ScanRole::Upper);
        upper.unwrap().role_source = RoleSource::Provisional;
        assert!(numbering(&one, json!({ "arch": "lower" })).is_ok());
        assert!(numbering(&one, Value::Null).is_err());
    }

    #[test]
    fn bad_parameters_are_rejected() {
        let state = demo_state("lower", 7);
        for p in [
            json!({ "ends": [4, 7] }),
            json!({ "missing": [99] }),
            json!({ "arch": "upper" }),
            json!({ "nope": true }),
        ] {
            assert!(
                matches!(numbering(&state, p.clone()), Err(CommandError::Invalid(_))),
                "{p}"
            );
        }
    }

    #[test]
    fn the_demo_seeds_are_found_again() {
        let state = demo_state("lower", 7);
        let truth = seeds_of(&state);
        // forget the seeds of the demo, then propose them
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.seeds.clear();
        let mut bare = state.clone();
        bare.set_module(MODULE_ID, ModuleState::new(data));
        let (next, out) = numbering(&bare, Value::Null).unwrap();
        let got = seeds_of(&next);
        assert_eq!(
            got.iter().map(|s| s.fdi).collect::<Vec<_>>(),
            truth.iter().map(|s| s.fdi).collect::<Vec<_>>()
        );
        for (g, t) in got.iter().zip(&truth) {
            assert!((g.position - t.position).length() < 1.5, "{}", g.fdi);
        }
        let arches = out["arches"].as_array().unwrap();
        assert_eq!(arches.len(), 1);
        assert_eq!(arches[0]["arch"], "lower");
        assert_eq!(arches[0]["teeth"].as_array().unwrap().len(), 14);
        assert_eq!(arches[0]["diagnostics"]["molars"]["right"], 7);
        // the scans are untouched; a second run gives the same seeds
        assert_eq!(next.case.scans, state.case.scans);
        let (again, _) = numbering(&next, Value::Null).unwrap();
        assert_eq!(seeds_of(&again), got);
    }

    #[test]
    fn only_the_requested_arch_is_renumbered() {
        let both = exec(
            "aligners.demo",
            &demo_state("lower", 7),
            json!({ "arch": "upper", "seed": 3 }),
        )
        .unwrap()
        .0;
        let before = seeds_of(&both);
        let (next, out) = numbering(&both, json!({ "arch": "upper", "ends": [7, 7] })).unwrap();
        assert_eq!(out["arches"].as_array().unwrap().len(), 1);
        let after = seeds_of(&next);
        let lower = |v: &[dca_model::Seed]| -> Vec<dca_model::Seed> {
            v.iter()
                .filter(|s| Arch::of(s.fdi) == Arch::Lower)
                .copied()
                .collect()
        };
        assert_eq!(lower(&after), lower(&before));
        assert_eq!(after.len(), before.len());
    }

    /// The demo state with the data of the later stages: teeth built from the
    /// labels, and a final position.
    fn downstream_state(arch: &str) -> CaseState {
        let s = demo_state(arch, 7);
        let s = exec("aligners.teeth.frames", &s, Value::Null).unwrap().0;
        let s = exec("aligners.planning.setup.automatic", &s, Value::Null)
            .unwrap()
            .0;
        let d = s.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!d.labels.is_empty() && !d.tooth_frames.is_empty());
        assert!(!d.setup.poses.is_empty());
        s
    }

    #[test]
    fn new_seeds_clear_what_was_computed_from_the_old_ones() {
        let s = downstream_state("lower");
        let (next, out) = numbering(&s, json!({ "ends": [6, 6] })).unwrap();
        let d = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!d.seeds.is_empty(), "the new seeds are there");
        assert!(d.labels.is_empty() && d.tooth_frames.is_empty());
        assert!(d.tooth_geometry.is_empty() && d.crowns.is_empty());
        assert!(d.setup.poses.is_empty() && d.stages.is_empty());
        let cleared = out["arches"][0]["dropped"].as_array().unwrap();
        for id in ["labels", "tooth_frames", "setup"] {
            assert!(cleared.iter().any(|c| c == id), "{id} in {cleared:?}");
        }
        // the leaves read it from the data
        assert!(crate::done::leaf_done(&next, "aligners.teeth.find"));
        for id in [
            "aligners.preparation.segmentation.automatic",
            "aligners.planning.setup.automatic",
        ] {
            assert!(!crate::done::leaf_done(&next, id), "{id}");
        }
        // the input state is untouched (undo)
        let before = s.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!before.labels.is_empty() && !before.setup.poses.is_empty());
    }

    #[test]
    fn the_same_seeds_again_keep_the_results_and_the_other_arch_is_left_alone() {
        let s = downstream_state("lower");
        let (first, _) = numbering(&s, Value::Null).unwrap();
        // colours and teeth again on top of the numbering, then the same numbering
        let again = exec(
            "aligners.preparation.segmentation.automatic",
            &first,
            Value::Null,
        )
        .unwrap()
        .0;
        let (kept, out) = numbering(&again, Value::Null).unwrap();
        let d = kept.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(!d.labels.is_empty(), "nothing changed, nothing to redo");
        assert_eq!(out["arches"][0]["dropped"], json!([]));
        // an upper numbering leaves the lower arch's results where they are
        let both = exec(
            "aligners.demo",
            &downstream_state("lower"),
            json!({ "arch": "upper", "seed": 3 }),
        )
        .unwrap()
        .0;
        let (up, _) = numbering(&both, json!({ "arch": "upper", "ends": [6, 6] })).unwrap();
        let d = up.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(d.labels.contains_key(&ScanRole::Lower));
        assert!(!d.labels.contains_key(&ScanRole::Upper));
        assert!(d.tooth_frames.keys().all(|f| !f.is_upper()));
        assert!(!d.setup.poses.is_empty(), "the lower setup stays");
    }

    /// `state` with the mesh of the `role` scan replaced by one triangle, on
    /// which the automatic numbering cannot find an arch.
    pub(crate) fn with_broken_scan(state: &CaseState, role: ScanRole) -> CaseState {
        use glam::Vec3;
        let mut s = state.clone();
        let bad = s.blobs.insert(dc_mesh::TriMesh::new(
            vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            vec![[0, 1, 2]],
        ));
        let scan = s.case.scans.iter_mut().find(|x| x.role == role).unwrap();
        scan.mesh = bad;
        s
    }

    #[test]
    fn each_arch_is_numbered_on_its_own_and_a_failing_one_is_reported() {
        let lower = downstream_state("lower");
        let both = exec(
            "aligners.demo",
            &lower,
            json!({ "arch": "upper", "seed": 3 }),
        )
        .unwrap()
        .0;
        let upper_before: Vec<_> = seeds_of(&both)
            .into_iter()
            .filter(|x| x.fdi.is_upper())
            .collect();
        let broken = with_broken_scan(&both, ScanRole::Upper);
        let (next, out) = numbering(&broken, json!({ "ends": [6, 6] })).unwrap();
        // the lower arch has its new seeds, the upper one keeps what it had
        assert_eq!(out["arches"].as_array().unwrap().len(), 1);
        assert_eq!(out["arches"][0]["arch"], "lower");
        assert_eq!(out["failed"][0]["arch"], "upper");
        let why = out["failed"][0]["error"].as_str().unwrap();
        assert!(why.contains("upper") && !why.contains("aligners-"), "{why}");
        let upper_after: Vec<_> = seeds_of(&next)
            .into_iter()
            .filter(|x| x.fdi.is_upper())
            .collect();
        assert_eq!(upper_after, upper_before);
        let d = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(
            d.labels.contains_key(&ScanRole::Upper),
            "the upper results stay"
        );
        assert!(!d.labels.contains_key(&ScanRole::Lower));
        // asked for that arch alone, its failure is the error
        assert!(numbering(&broken, json!({ "arch": "upper" })).is_err());
        // every arch failing is an error with the reason of each
        let none = with_broken_scan(&broken, ScanRole::Lower);
        let err = numbering(&none, Value::Null).unwrap_err().to_string();
        assert!(err.contains("upper") && err.contains("lower"), "{err}");
        // nothing failed: the list is empty
        let (_, out) = numbering(&both, json!({ "ends": [6, 6] })).unwrap();
        assert_eq!(out["failed"], json!([]));
    }

    #[test]
    fn the_blobs_of_what_new_seeds_drop_leave_the_case() {
        let s = downstream_state("lower");
        let old = *s
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .labels
            .get(&ScanRole::Lower)
            .unwrap();
        assert!(s.blobs.iter().any(|(r, _)| *r == old));
        let (next, _) = numbering(&s, json!({ "ends": [6, 6] })).unwrap();
        assert!(!next.blobs.iter().any(|(r, _)| *r == old));
        // every blob left is still used
        let used = next.referenced_blobs();
        assert!(next.blobs.iter().all(|(r, _)| used.contains(r)));
    }

    #[test]
    fn the_missing_teeth_of_a_run_are_read_not_stored() {
        // as in the plugin, whose proposal only reads its list of missing teeth
        let s = demo_state("lower", 7);
        let (next, _) = numbering(&s, json!({ "missing": [45] })).unwrap();
        assert!(seeds_of(&next).iter().all(|x| x.fdi.get() != 45));
        let d = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(d.missing.is_empty());
        assert!(d.settings.find_teeth.missing.is_empty());
    }

    #[test]
    fn a_missing_second_molar_makes_room_for_the_third_one_as_in_the_plugin() {
        // `candidate_sequences` of the plugin: the third molars are always
        // candidates (with a prior cost when not searched for), so with 17
        // missing a seventh tooth on the right is 18
        use dca_algo::autoseed::candidate_sequences;
        let f = |n| Fdi::new(n).unwrap();
        let c = candidate_sequences(Arch::Upper, false, &[f(17)], Some([8, 7]));
        assert_eq!(c.len(), 1);
        let teeth: Vec<u8> = c[0].teeth.iter().map(|t| t.get()).collect();
        assert!(teeth.contains(&18) && !teeth.contains(&17), "{teeth:?}");
        assert_eq!(teeth.iter().filter(|t| **t / 10 == 1).count(), 7);
        assert!((c[0].prior - 3.0).abs() < 1e-12, "{}", c[0].prior);
    }

    #[test]
    fn errors_of_the_algorithm_are_sentences_of_the_app() {
        let keys = [
            "aligners-err-seed-degenerate",
            "aligners-err-seed-arch-not-found",
            "aligners-err-seed-too-few",
            "aligners-err-seed-bad-ends",
            "aligners-err-seed-bad-tooth",
        ];
        let errors = [
            AutoseedError::DegenerateMesh,
            AutoseedError::ArchNotFound,
            AutoseedError::TooFewSeeds(2),
            AutoseedError::BadEnds,
            AutoseedError::BadTooth(99),
        ];
        let en: Vec<String> = errors
            .into_iter()
            .map(|e| seed_error(Some(Arch::Lower), e).to_string())
            .collect();
        assert!(
            en[2].contains("only 2 teeth found on the lower arch"),
            "{}",
            en[2]
        );
        assert!(en[2].contains("by hand with Mark a tooth"), "{}", en[2]);
        assert!(en[4].contains("99"), "{}", en[4]);
        // not the English of the algorithm, and not a bare key
        for (m, k) in en.iter().zip(keys) {
            assert!(!m.contains(k) && !m.contains("could not be found"), "{m}");
        }
        assert!(matches!(
            seed_error(None, AutoseedError::BadEnds),
            CommandError::Invalid(_)
        ));
        assert!(matches!(
            seed_error(None, AutoseedError::TooFewSeeds(1)),
            CommandError::Failed(_)
        ));
    }

    #[test]
    fn orientation_uses_the_seeds_or_the_crowns() {
        let state = demo_state("lower", 7);
        let (with_seeds, out) = orientation(&state, Value::Null).unwrap();
        assert_eq!(out["source"], "seeds");
        // the same transform as aligners.frame
        let (framed, _) = exec("aligners.frame", &state, Value::Null).unwrap();
        assert_eq!(
            with_seeds.case.world_frame.from_case,
            framed.case.world_frame.from_case
        );
        // without seeds: the arch frame of the crowns, close to the seeds' one
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.seeds.clear();
        let mut bare = state.clone();
        bare.set_module(MODULE_ID, ModuleState::new(data));
        let (next, out) = orientation(&bare, Value::Null).unwrap();
        assert_eq!(out["source"], "crowns");
        let a = with_seeds.case.world_frame.from_case;
        let b = next.case.world_frame.from_case;
        assert!(a.q.angle_between(b.q).to_degrees() < 5.0);
        assert!((a.t - b.t).length() < 3.0, "{:?} {:?}", a.t, b.t);
        assert_eq!(next.case.scans, state.case.scans);
    }

    fn find_settings(state: &CaseState) -> dca_model::FindTeethSettings {
        state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .settings
            .find_teeth
            .clone()
    }

    #[test]
    fn the_find_settings_are_stored_in_the_case_and_validated() {
        let state = demo_state("lower", 7);
        assert_eq!(find_settings(&state), Default::default());
        let (a, out) = exec(FIND_SETTINGS, &state, json!({ "third_molars": true })).unwrap();
        assert_eq!(out, json!({ "third_molars": true, "missing": [] }));
        let (b, _) = exec(FIND_SETTINGS, &a, json!({ "missing": [45, 35, 45] })).unwrap();
        // a field not given stays; the list is sorted without repetitions
        let s = find_settings(&b);
        assert!(s.third_molars);
        assert_eq!(s.missing, [35, 45]);
        // the seeds and everything else are untouched
        assert_eq!(seeds_of(&b), seeds_of(&state));
        // bad numbers (not a tooth, a deciduous tooth, zero) change nothing
        for bad in [99, 0, 19, 51, 49] {
            let err = exec(FIND_SETTINGS, &b, json!({ "missing": [35, bad] })).unwrap_err();
            let CommandError::Invalid(m) = err else {
                panic!("{bad}: not refused");
            };
            assert!(m.contains(&bad.to_string()), "{m}");
        }
        assert!(exec(FIND_SETTINGS, &b, json!({ "nope": 1 })).is_err());
        // the settings are what the menu click gives to Find the teeth
        let (c, _) = exec(FIND_SETTINGS, &b, json!({ "missing": [] })).unwrap();
        assert!(find_settings(&c).missing.is_empty());
        // they survive a save: the JSON of the Aligners data
        let data = b.module_data::<AlignersState>(MODULE_ID).unwrap();
        let back: AlignersState =
            serde_json::from_value(serde_json::to_value(data).unwrap()).unwrap();
        assert_eq!(back.settings.find_teeth, s);
    }

    #[test]
    fn the_numbering_leaves_out_the_missing_teeth_of_the_settings() {
        let state = demo_state("lower", 7);
        let (set, _) = exec(FIND_SETTINGS, &state, json!({ "missing": [45] })).unwrap();
        let params = crate::find_teeth::leaf_params(&set);
        assert_eq!(params, json!({ "third_molars": false, "missing": [45] }));
        let (found, _) = exec("aligners.teeth.find", &set, params).unwrap();
        assert!(seeds_of(&found).iter().all(|s| s.fdi.get() != 45));
        assert!(seeds_of(&found).len() >= 10);
    }
}
