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
//!   checked by the user.
//!
//! Both refuse a scan still marked provisional: its arch is a guess, and what
//! they compute depends on it (`aligners.case.detect-arch` settles it).

use dc_case::{CaseState, ModuleState, ScanRole};
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
            label: "aligners-menu-preparation-orientation-automatic",
            doc: "aligners-cmd-autoseed-orientation-doc",
            params: frame::Params,
            run: run_orientation,
        },
        stage_command! {
            id: "aligners.preparation.numbering.automatic",
            label: "aligners-menu-preparation-numbering-automatic",
            doc: "aligners-cmd-autoseed-numbering-doc",
            params: NumberingParams,
            run: run_numbering,
        },
    ]
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

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn invalid_or_failed(e: AutoseedError) -> CommandError {
    match e {
        AutoseedError::BadEnds | AutoseedError::BadTooth(_) => CommandError::Invalid(e.to_string()),
        _ => CommandError::Failed(e.to_string()),
    }
}

/// The Aligners data of the case (new ones when there are none).
fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(|| CommandError::Invalid("the Aligners data cannot be read".to_owned())),
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
        .ok_or_else(|| {
            CommandError::Invalid(
                match params.reference {
                    Reference::Auto => "the case has no upper or lower scan",
                    Reference::Upper => "the case has no upper scan",
                    Reference::Lower => "the case has no lower scan",
                }
                .to_owned(),
            )
        })?;
    let scan = state
        .case
        .scan(arch.role())
        .ok_or_else(|| CommandError::Invalid("the reference scan is missing".to_owned()))?;
    require_settled(scan)?;
    let mesh = state.blobs.require(&scan.mesh)?;
    let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let seeds = aligners_data(state)?.seeds;
    let input = frame::Input {
        arch,
        vertices: &vertices,
        seeds: &seeds,
    };
    let out = autoseed::orient(&input, &mesh.tris).map_err(invalid_or_failed)?;
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
    algo.options().map_err(invalid_or_failed)?;
    if params
        .ends
        .is_some_and(|e| e.iter().any(|x| !(5..=8).contains(x)))
    {
        return Err(invalid_or_failed(AutoseedError::BadEnds));
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
        return Err(CommandError::Invalid(
            "the case has no upper or lower scan".to_owned(),
        ));
    }
    for scan in arches.iter().filter_map(|a| state.case.scan(a.role())) {
        require_settled(scan)?;
    }
    let mut data = aligners_data(state)?;
    let mut report = Vec::new();
    for arch in arches {
        let scan = state.case.scan(arch.role()).ok_or_else(|| {
            CommandError::Invalid(format!("the case has no {} scan", arch_name(arch)))
        })?;
        let mesh = state.blobs.require(&scan.mesh)?;
        let vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let input = autoseed::Input {
            arch,
            vertices: &vertices,
            triangles: &mesh.tris,
        };
        let out = autoseed::run(&input, &algo).map_err(|e| match e {
            AutoseedError::TooFewSeeds(_) | AutoseedError::ArchNotFound => {
                CommandError::Failed(format!("{} arch: {e}", arch_name(arch)))
            }
            e => invalid_or_failed(e),
        })?;
        ctx.check_cancelled()?;
        data.seeds.retain(|s| Arch::of(s.fdi) != arch);
        data.seeds.extend(out.seeds.iter().copied());
        report.push(json!({
            "arch": arch,
            "teeth": out.seeds.iter().map(|s| s.fdi.get()).collect::<Vec<_>>(),
            "arch_frame": out.arch_frame,
            "diagnostics": out.diagnostics,
        }));
    }
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, json!({ "arches": report })))
}

#[cfg(test)]
mod tests {
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
            assert!(m.contains("aligners.case.detect-arch"), "{m}");
            assert!(m.contains("case.set-role"), "{m}");
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
}
