//! What the case holds for each menu leaf ([`dc_module_api::Module::leaf_done`]),
//! read from its data: a step is done when every arch of the case has its
//! result, so undoing a step, or replacing a scan, undoes its tick.
//!
//! Some leaves leave no trace in the case and are never done here: the two
//! exports (they write files), the re-plan (it replaces the aligners) and the
//! whole pipeline, which is not a step.
//!
//! "Bite" with one arch has nothing to do: there is no bite
//! ([`dca_model::bite_use`]), so the step never stands as the next one to do
//! nor leaves the progress short.

use dc_case::{CaseState, RoleSource, ScanRole};
use dca_algo::anatomy::Arch;
use dca_algo::schedule;
use dca_model::{AlignersState, BiteUse, MODULE_ID};

use crate::commands::import::IMPORT_SCANS;
use crate::commands::wrappers::BITE;

/// The arches with a scan in the case.
fn arches(state: &CaseState) -> Vec<Arch> {
    [Arch::Lower, Arch::Upper]
        .into_iter()
        .filter(|a| state.case.scan(a.role()).is_some())
        .collect()
}

/// `true` when the case holds what leaf `id` produces.
pub(crate) fn leaf_done(state: &CaseState, id: &str) -> bool {
    if id == IMPORT_SCANS {
        let scans = &state.case.scans;
        return scans.iter().any(|s| s.role != ScanRole::Bite)
            && scans
                .iter()
                .all(|s| s.role_source != RoleSource::Provisional);
    }
    let Some(data) = state.module_data::<AlignersState>(MODULE_ID) else {
        return false;
    };
    let arches = arches(state);
    let every_arch =
        |has: &dyn Fn(Arch) -> bool| !arches.is_empty() && arches.iter().all(|a| has(*a));
    let of = |a: Arch| move |f: &dc_case::Fdi| Arch::of(*f) == a;
    match id {
        "aligners.teeth.find" | "aligners.preparation.numbering.automatic" => {
            every_arch(&|a| data.seeds.iter().any(|s| Arch::of(s.fdi) == a))
        }
        "aligners.preparation.segmentation.automatic" => {
            every_arch(&|a| data.labels.contains_key(&a.role()))
        }
        "aligners.teeth.build" => {
            every_arch(&|a| data.crowns.keys().any(of(a)) && data.gingiva.contains_key(&a.role()))
        }
        "aligners.diagnosis.space-analysis" => {
            every_arch(&|a| data.diagnosis.arches.contains_key(&a.role()))
        }
        BITE => match BiteUse::of(state) {
            // one arch: the bite does not apply, nothing is left to do
            BiteUse::OneArch => true,
            u => u.applies() && data.bite_registered,
        },
        "aligners.planning.setup.automatic" => every_arch(&|a| data.setup.poses.keys().any(of(a))),
        "aligners.planning.aligners" => {
            every_arch(&|a| schedule::active_stages(a, &data.stages) > 0)
        }
        // every arch classified (the decisions are the clinician's own pace)
        "aligners.review.classify" => {
            every_arch(&|a| data.review.iter().any(|r| r.arch == a.role()))
        }
        "aligners.followup.visits" => !data.visits.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ModuleState;
    use dc_command::JobCtx;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> CaseState {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
            .unwrap()
            .0
    }

    fn demo() -> CaseState {
        exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
    }

    #[test]
    fn an_empty_case_has_nothing_done() {
        let s = CaseState::default();
        for leaf in crate::tree::leaves() {
            assert!(!leaf_done(&s, leaf.id), "{}", leaf.id);
        }
    }

    #[test]
    fn the_steps_follow_the_data_of_the_case() {
        let s = demo();
        assert!(leaf_done(&s, IMPORT_SCANS));
        let marks = "aligners.teeth.find";
        let colours = "aligners.preparation.segmentation.automatic";
        assert!(leaf_done(&s, marks) && leaf_done(&s, colours));
        assert!(!leaf_done(&s, "aligners.teeth.build"));
        assert!(!leaf_done(&s, "aligners.planning.setup.automatic"));

        // the setup of the arch is there, then an arch without one: not done
        let s = exec("aligners.teeth.frames", &s, Value::Null);
        let s = exec("aligners.planning.setup.automatic", &s, Value::Null);
        assert!(leaf_done(&s, "aligners.planning.setup.automatic"));
        let two = exec("aligners.demo", &s, json!({ "arch": "upper" }));
        assert!(!leaf_done(&two, "aligners.planning.setup.automatic"));
        assert!(leaf_done(&two, marks), "the demo marks both arches");

        // what another step cleared is not done any more
        let mut data = two.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.labels.clear();
        data.bite_registered = true;
        let mut cleared = two.clone();
        cleared.set_module(MODULE_ID, ModuleState::new(data));
        assert!(!leaf_done(&cleared, colours));
        assert!(leaf_done(&cleared, BITE));
    }

    #[test]
    fn a_guessed_arch_is_not_an_imported_case() {
        let mut s = demo();
        s.case.scans[0].role_source = RoleSource::Provisional;
        assert!(!leaf_done(&s, IMPORT_SCANS));
    }

    #[test]
    fn the_bite_has_nothing_to_do_with_one_arch() {
        // a new case: nothing to do yet
        let mut empty = CaseState::default();
        empty.set_module(MODULE_ID, ModuleState::new(AlignersState::default()));
        assert!(!leaf_done(&empty, BITE));
        // one arch: there is no bite, whatever an old file stored
        let one = demo();
        assert!(leaf_done(&one, BITE));
        let mut data = one.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.bite_registered = true;
        let mut stale = one.clone();
        stale.set_module(MODULE_ID, ModuleState::new(data));
        assert!(leaf_done(&stale, BITE));
        // so the next step of one arch is never the bite
        let next = |s: &CaseState| {
            crate::tree::menu_tree()
                .step_ids()
                .into_iter()
                .find(|id| !leaf_done(s, id))
        };
        assert_eq!(next(&one), Some("aligners.teeth.build"));
        // both arches: done once they are related
        let two = exec("aligners.demo", &one, json!({ "arch": "upper" }));
        assert!(!leaf_done(&two, BITE));
        let mut data = two.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.bite_registered = true;
        let mut related = two.clone();
        related.set_module(MODULE_ID, ModuleState::new(data));
        assert!(leaf_done(&related, BITE));
    }

    #[test]
    fn the_whole_pipeline_is_never_done_and_never_a_step() {
        let s = demo();
        assert!(!leaf_done(&s, "aligners.auto"));
        let leaf = crate::tree::find("aligners.auto").unwrap();
        assert!(!leaf.counts_as_step);
        assert!(!leaf_done(&s, "aligners.nope"));
    }
}
