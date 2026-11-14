//! `aligners.case.set-arch`: the arch of a scan chosen by hand, the plugin's
//! `oa.arch_set` ("Scegli l'arcata", `manual.set_arch_steps`). The Import
//! scans inspector runs it when the user moves a scan between the upper and
//! the lower arch, and for "Flip the occlusal side"; `case.set-role` stays
//! the command of the shell for the bite and for a case without the
//! Aligners data.
//!
//! Parameters: `scan`, the arch scan by the arch it has now (default the only
//! arch scan); `arch`, the arch it is (default the one it has); and
//! `flip_occlusal`. The scan's role becomes explicit, as with `case.set-role`.
//!
//! A new arch (the plugin, in this order):
//! * the scan takes the arch; a scan already there takes the old one (the
//!   port has one scan per arch; the plugin keeps every arch in an object of
//!   its own), and the world frame, which came from the old arches, is reset;
//! * the tooth marks, the tooth colours (a relabelled blob), the teeth marked
//!   missing (the marks and the "missing teeth" setting of Find the teeth)
//!   and the control visits of each moved arch are renumbered to the other
//!   arch, same side and same tooth (`other_arch_fdi`: 13 ↔ 43, 24 ↔ 34);
//! * the arch's 3D teeth, when it had some, are built again
//!   (`aligners.teeth.build`, the plugin's `build_teeth_steps`): the tooth
//!   type decides the axes and the centre of resistance. What that clears in
//!   the plugin goes here too: the final position and the aligners of the
//!   arch (`setup_json`, `plan_json`), with the diagnosis made again. The
//!   tooth axes, measures, closed teeth and gingiva are made again from the
//!   renumbered colours, never renumbered; the ramps of the arch, the
//!   doubts of the separation check and the bite registration (the arches it
//!   related moved) go.
//!
//! `flip_occlusal` turns the occlusal side of the arch over (the plugin's
//! `oa_occlusal_flip`, toggled each time): every arch frame of the arch is
//! turned over from now on (`dca_algo::frame::Input::occlusal_flipped`). As
//! in the plugin nothing is cleared: the separation and what follows it are
//! out of date (`invalidated`, the plugin's "rifai la segmentazione"; the 3D
//! teeth show it). With a new arch the flip comes first, so the teeth built
//! again stand on the new side.
//!
//! The output: the scan (`scan_id`, `source_name`, `from`, `to`,
//! `role_source`), `swapped` (the scan that took the old arch), `renumbered`,
//! `rebuilt` (teeth built again), `flipped`, `occlusal_flipped` (the arch's
//! side now), `world_frame_reset`, `module_cleared` (what is gone, with the
//! ids of `case.set-role`), `invalidated` (what is kept but out of date) and
//! `teeth_build` (the output of the rebuild, `null` without one). One
//! undoable change.

use std::collections::BTreeSet;

use dc_case::import::roles_changed;
use dc_case::{BlobRef, CaseState, ModuleState, RoleSource, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_model::{AlignersState, MODULE_ID, other_arch_label};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::messages;
use crate::manual::common::{arch_name, data_of, finish_blobs, pick_arch, run_command};

/// Id of the command.
pub const SET_ARCH: &str = "aligners.case.set-arch";

/// The 3D teeth of an arch, built again after a new arch.
const BUILD_TEETH: &str = "aligners.teeth.build";

/// The registered commands of the scan corrections of "Import scans", built
/// with `stage_command!`.
pub(crate) fn specs() -> Vec<dc_module_api::CommandSpec> {
    vec![
        stage_command! {
            id: SET_ARCH,
            label: "aligners-cmd-set-arch",
            doc: "aligners-cmd-set-arch-doc",
            params: SetArchParams,
            run: run_set_arch,
        },
        stage_command! {
            id: super::make_base::MAKE_BASE,
            label: "aligners-cmd-make-base",
            doc: "aligners-cmd-make-base-doc",
            params: super::make_base::MakeBaseParams,
            run: super::make_base::run_make_base,
        },
    ]
}

/// Parameters of `aligners.case.set-arch`.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SetArchParams {
    /// The arch scan to correct, by the arch it has now: upper or lower
    /// (default: the only arch scan of the case).
    pub scan: Option<Arch>,
    /// The arch the scan is: upper or lower (default: the one it has). A
    /// scan already there takes the other arch.
    pub arch: Option<Arch>,
    /// Turn the occlusal side of the arch over (again: back as it was).
    pub flip_occlusal: bool,
}

/// The arch in the other jaw.
fn opposite(role: ScanRole) -> ScanRole {
    match role {
        ScanRole::Upper => ScanRole::Lower,
        _ => ScanRole::Upper,
    }
}

fn role_word(role: ScanRole) -> &'static str {
    match role {
        ScanRole::Upper => "upper",
        ScanRole::Lower => "lower",
        ScanRole::Bite => "bite",
    }
}

/// The ids of what the label dependents of the arches `roles` hold
/// (`clear_label_dependents` without clearing).
fn held(data: &AlignersState, roles: &BTreeSet<ScanRole>) -> BTreeSet<&'static str> {
    let mut d = data.clone();
    roles
        .iter()
        .flat_map(|r| d.clear_label_dependents(*r))
        .collect()
}

/// `r` with every vertex label in the other arch; the old blob goes to
/// `stale`.
fn relabelled(
    next: &mut CaseState,
    r: BlobRef,
    stale: &mut Vec<BlobRef>,
) -> Result<BlobRef, CommandError> {
    let mut mesh = next.blobs.require(&r)?.as_ref().clone();
    if let Some(labels) = &mut mesh.vertex_labels {
        for l in labels.iter_mut() {
            *l = other_arch_label(*l);
        }
    }
    stale.push(r);
    Ok(next.blobs.insert(mesh))
}

pub(super) fn run_set_arch(
    state: &CaseState,
    params: SetArchParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let from = pick_arch(state, params.scan, None)?;
    let to = params.arch.unwrap_or(from);
    let (from_role, to_role) = (from.role(), to.role());
    let scan = state
        .case
        .scan(from_role)
        .ok_or_else(|| messages::no_scan(from))?;
    let scan_id = scan.id;
    let mut data = data_of(state)?;
    let mut next = state.clone();
    let mut stale = Vec::new();
    let mut cleared: BTreeSet<&'static str> = BTreeSet::new();
    let occupant = (from != to)
        .then(|| state.case.scan(to_role))
        .flatten()
        .map(|s| (s.id, s.source_name.clone()));

    for s in &mut next.case.scans {
        if s.id == scan_id {
            s.role = to_role;
            s.role_source = RoleSource::Explicit;
        } else if occupant.as_ref().is_some_and(|(id, _)| *id == s.id) {
            s.role = from_role;
            s.role_source = RoleSource::Explicit;
        }
    }
    next.case.scans.sort_by_key(|s| s.role);

    // the arches that change jaw, and those of them with 3D teeth
    let mut moved = BTreeSet::new();
    let mut rebuild = BTreeSet::new();
    if from != to {
        moved.insert(from_role);
        if occupant.is_some() {
            moved.insert(to_role);
        }
        for &r in &moved {
            let arch = Arch::from_role(r).expect("an arch");
            if data.crowns.keys().any(|f| Arch::of(*f) == arch) {
                rebuild.insert(opposite(r));
            }
        }
        // what the shell does to the case and to the other modules (the
        // world frame, `scan_replaced`, an opaque module that locks the
        // arches): `roles_changed` without the Aligners data, which is
        // renumbered here instead of cleared
        let touched: BTreeSet<ScanRole> = [from_role, to_role].into();
        next.modules.remove(MODULE_ID);
        let before = next.referenced_blobs();
        let others = roles_changed(&mut next, &touched, &before)
            .map_err(|e| CommandError::Invalid(e.to_string()))?;
        cleared.extend(others);

        let had = held(&data, &moved);
        if std::mem::take(&mut data.bite_registered) {
            cleared.insert("bite_registered");
        }
        for &r in &moved {
            data.clear_label_dependents(r);
        }
        let visits: Vec<usize> = (0..data.visits.len())
            .filter(|&k| moved.contains(&data.visits[k].arch))
            .collect();
        data.renumber_arches(&moved);
        for &r in &moved {
            let new = opposite(r);
            if let Some(&old) = data.labels.get(&new) {
                let r = relabelled(&mut next, old, &mut stale)?;
                data.labels.insert(new, r);
            }
        }
        for k in visits {
            if let (true, Some(old)) = (data.visits[k].segmented, data.visits[k].scan) {
                data.visits[k].scan = Some(relabelled(&mut next, old, &mut stale)?);
            }
        }
        // what was there and is not made again (the rebuild remakes the rest)
        cleared.extend(had);
    }
    if params.flip_occlusal && !data.occlusal_flipped.remove(&to_role) {
        data.occlusal_flipped.insert(to_role);
    }
    next.set_module(MODULE_ID, ModuleState::new(data));
    finish_blobs(&mut next, &stale);
    ctx.check_cancelled()?;

    // the 3D teeth of the renumbered arches, built again
    let mut teeth_build = Value::Null;
    if !rebuild.is_empty() {
        let which = if rebuild.len() == 2 {
            "all"
        } else {
            role_word(*rebuild.first().expect("one arch"))
        };
        let (built, out) = run_command(BUILD_TEETH, &next, json!({ "arch": which }), ctx)?;
        next = built;
        teeth_build = out;
    }
    let mut data = data_of(&next)?;
    let ids: Vec<String> = next.case.scans.iter().map(|s| s.id.to_string()).collect();
    data.prune_component_edits(ids.iter().map(String::as_str));
    let new_roles: BTreeSet<ScanRole> = moved.iter().map(|r| opposite(*r)).collect();
    for id in held(&data, &new_roles) {
        cleared.remove(id);
    }
    let rebuilt = data
        .crowns
        .keys()
        .filter(|f| rebuild.iter().any(|r| Arch::of(**f).role() == *r))
        .count();
    let invalidated = if params.flip_occlusal {
        data.seed_dependents(to_role)
    } else {
        Vec::new()
    };
    let occlusal_flipped = data.occlusal_flipped.contains(&to_role);
    next.set_module(MODULE_ID, ModuleState::new(data));

    let module_cleared: Vec<&str> = dca_model::CLEARED_IDS
        .iter()
        .copied()
        .filter(|id| cleared.contains(id))
        .chain(
            cleared
                .iter()
                .copied()
                .filter(|id| !dca_model::CLEARED_IDS.contains(id)),
        )
        .collect();
    let swapped = occupant.map(|(id, name)| {
        json!({
            "scan_id": id,
            "source_name": name,
            "from": role_word(to_role),
            "to": role_word(from_role),
        })
    });
    Ok((
        next,
        json!({
            "scan_id": scan_id,
            "source_name": scan.source_name,
            "from": arch_name(from),
            "to": arch_name(to),
            "role_source": "explicit",
            "swapped": swapped,
            "renumbered": from != to,
            "rebuilt": rebuilt,
            "flipped": params.flip_occlusal,
            "occlusal_flipped": occlusal_flipped,
            "world_frame_reset": from != to,
            "module_cleared": module_cleared,
            "invalidated": invalidated,
            "teeth_build": teeth_build,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::commands::occlusion::tests::{exec, try_exec, two_demo_arches};
    use dc_case::{Fdi, Pose};
    use dc_command::UndoStack;
    use dca_model::{TeethChange, other_arch_fdi};
    use glam::DVec3;

    fn data(s: &CaseState) -> AlignersState {
        s.module_data::<AlignersState>(MODULE_ID).unwrap().clone()
    }

    fn demo(arch: &str) -> CaseState {
        exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": arch }),
        )
        .0
    }

    /// The lower demo arch with its 3D teeth, built once for every test.
    fn built_lower() -> CaseState {
        static BUILT: std::sync::LazyLock<CaseState> = std::sync::LazyLock::new(|| {
            exec("aligners.teeth.build", &demo("lower"), Value::Null).0
        });
        BUILT.clone()
    }

    fn labels(s: &CaseState, role: ScanRole) -> Vec<u16> {
        let d = data(s);
        let blob = s.blobs.require(&d.labels[&role]).unwrap();
        blob.vertex_labels.clone().unwrap()
    }

    fn seeds(s: &CaseState) -> BTreeMap<u8, DVec3> {
        data(s)
            .seeds
            .iter()
            .map(|s| (s.fdi.get(), s.position))
            .collect()
    }

    #[test]
    fn it_is_a_command_of_the_cli_with_its_parameters() {
        let spec = crate::commands::find(SET_ARCH).unwrap();
        let schema = spec.schema();
        for p in ["scan", "arch", "flip_occlusal"] {
            assert!(schema["properties"][p].is_object(), "{p}: {schema}");
        }
        assert!(crate::tree::find(SET_ARCH).is_none());
    }

    #[test]
    fn a_lower_arch_set_as_upper_takes_the_upper_numbers() {
        let mut state = demo("lower");
        let mut d = data(&state);
        d.missing.insert(Fdi::new(35).unwrap());
        d.settings.find_teeth.missing = vec![18, 35];
        state.set_module(MODULE_ID, ModuleState::new(d));
        state.case.world_frame.from_case = Pose::new(DVec3::X, Default::default());
        let before = seeds(&state);
        let old_labels = labels(&state, ScanRole::Lower);

        let (next, out) = exec(
            SET_ARCH,
            &state,
            json!({ "scan": "lower", "arch": "upper" }),
        );
        assert_eq!(out["from"], "lower", "{out}");
        assert_eq!(out["to"], "upper", "{out}");
        assert_eq!(out["renumbered"], true, "{out}");
        assert_eq!(out["swapped"], Value::Null, "{out}");
        assert_eq!(out["world_frame_reset"], true, "{out}");
        let scan = next.case.scan(ScanRole::Upper).unwrap();
        assert_eq!(scan.id, state.case.scan(ScanRole::Lower).unwrap().id);
        assert_eq!(scan.role_source, RoleSource::Explicit);
        assert!(next.case.scan(ScanRole::Lower).is_none());
        assert_eq!(next.case.world_frame, Default::default());

        // the plugin's table (manual._QUAD_SWAP): each mark keeps its place
        let after = seeds(&next);
        assert_eq!(after.len(), before.len());
        for (n, p) in &before {
            let f = other_arch_fdi(Fdi::new(*n).unwrap()).get();
            assert_eq!(after[&f], *p, "{n} -> {f}");
            assert_eq!(f % 10, n % 10);
            assert!(f / 10 == 1 || f / 10 == 2, "{f}");
        }
        // the colours: every point the same tooth in the other arch, the gum gum
        let d = data(&next);
        assert!(
            d.labels.contains_key(&ScanRole::Upper) && !d.labels.contains_key(&ScanRole::Lower)
        );
        let new_labels = labels(&next, ScanRole::Upper);
        assert_eq!(new_labels.len(), old_labels.len());
        for (a, b) in old_labels.iter().zip(&new_labels) {
            assert_eq!(*b, other_arch_label(*a));
        }
        assert!(new_labels.contains(&11) && !new_labels.iter().any(|&l| l >= 30));
        // missing teeth: `other_arch_fdi(f) if arch_of(f) != arch else f`
        assert_eq!(d.missing.iter().map(|f| f.get()).collect::<Vec<_>>(), [25]);
        assert_eq!(d.settings.find_teeth.missing, [18, 25]);
        // the old labels blob is gone
        assert!(
            next.blobs
                .get(&data(&state).labels[&ScanRole::Lower])
                .is_none()
        );
        // no teeth were built: nothing to rebuild, nothing cleared
        assert_eq!(out["rebuilt"], 0, "{out}");
        assert_eq!(out["module_cleared"], json!([]), "{out}");
        assert_eq!(out["teeth_build"], Value::Null, "{out}");

        // and back: the case of the start, but for the explicit role
        let (back, _) = exec(SET_ARCH, &next, json!({ "scan": "upper", "arch": "lower" }));
        assert_eq!(seeds(&back), before);
        assert_eq!(labels(&back, ScanRole::Lower), old_labels);
        assert_eq!(data(&back).missing, data(&state).missing);

        // one undo step brings everything back
        let mut undo = UndoStack::new(Arc::new(state.clone()), UndoStack::DEFAULT_BUDGET);
        undo.push(Arc::new(next), SET_ARCH);
        undo.undo().unwrap();
        assert_eq!(undo.current().case, state.case);
        assert_eq!(data(undo.current()), data(&state));
    }

    #[test]
    fn built_teeth_are_built_again_under_their_new_numbers() {
        let state = built_lower();
        let before = data(&state);
        let old: Vec<Fdi> = before.crowns.keys().copied().collect();
        assert!(!old.is_empty() && before.diagnosis.arches.contains_key(&ScanRole::Lower));
        let (next, out) = exec(SET_ARCH, &state, json!({ "arch": "upper" }));
        let d = data(&next);
        // the same teeth, numbered in the upper arch, built from the new colours
        let mut want: Vec<Fdi> = old.iter().map(|f| other_arch_fdi(*f)).collect();
        want.sort();
        assert_eq!(d.crowns.keys().copied().collect::<Vec<_>>(), want);
        assert_eq!(d.tooth_frames.keys().copied().collect::<Vec<_>>(), want);
        assert_eq!(out["rebuilt"], old.len(), "{out}");
        assert!(
            d.gingiva.contains_key(&ScanRole::Upper) && !d.gingiva.contains_key(&ScanRole::Lower)
        );
        assert!(d.diagnosis.arches.contains_key(&ScanRole::Upper));
        assert!(!d.diagnosis.arches.contains_key(&ScanRole::Lower));
        // they match the case, and the menu sees them built
        assert!(!d.teeth_out_of_date(ScanRole::Upper));
        assert!(crate::done::leaf_done(&next, "aligners.teeth.build"));
        assert!(out["teeth_build"]["steps"].is_array(), "{out}");
        // remade, so not reported as cleared
        let cleared = out["module_cleared"].as_array().unwrap();
        for id in ["crowns", "tooth_frames", "gingiva", "diagnosis"] {
            assert!(!cleared.iter().any(|c| c == id), "{id}: {out}");
        }
        // nothing of the lower arch is left
        assert!(d.crowns.keys().all(|f| f.is_upper()));
        assert!(d.seeds.iter().all(|s| s.fdi.is_upper()));
    }

    #[test]
    fn two_scans_swap_and_both_are_renumbered() {
        let mut state = two_demo_arches();
        let mut d = data(&state);
        d.bite_registered = true;
        state.set_module(MODULE_ID, ModuleState::new(d));
        let lower = state.case.scan(ScanRole::Lower).unwrap().clone();
        let upper = state.case.scan(ScanRole::Upper).unwrap().clone();
        let (lower_labels, upper_labels) = (
            labels(&state, ScanRole::Lower),
            labels(&state, ScanRole::Upper),
        );
        let before = seeds(&state);
        let (next, out) = exec(
            SET_ARCH,
            &state,
            json!({ "scan": "lower", "arch": "upper" }),
        );
        assert_eq!(out["swapped"]["scan_id"], json!(upper.id), "{out}");
        assert_eq!(out["swapped"]["to"], "lower", "{out}");
        assert_eq!(next.case.scan(ScanRole::Upper).unwrap().id, lower.id);
        assert_eq!(next.case.scan(ScanRole::Lower).unwrap().id, upper.id);
        let after = seeds(&next);
        for (n, p) in &before {
            assert_eq!(after[&other_arch_fdi(Fdi::new(*n).unwrap()).get()], *p);
        }
        let relabel = |l: &[u16]| l.iter().map(|&x| other_arch_label(x)).collect::<Vec<_>>();
        assert_eq!(labels(&next, ScanRole::Upper), relabel(&lower_labels));
        assert_eq!(labels(&next, ScanRole::Lower), relabel(&upper_labels));
        // the bite related the old arches: to be done again
        assert!(!data(&next).bite_registered);
        assert!(
            out["module_cleared"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "bite_registered")
        );
    }

    #[test]
    fn turning_the_occlusal_side_over_keeps_everything_and_toggles() {
        let state = built_lower();
        let (next, out) = exec(SET_ARCH, &state, json!({ "flip_occlusal": true }));
        assert_eq!(out["flipped"], true, "{out}");
        assert_eq!(out["occlusal_flipped"], true, "{out}");
        assert_eq!(out["renumbered"], false, "{out}");
        assert_eq!(out["world_frame_reset"], false, "{out}");
        assert_eq!(out["module_cleared"], json!([]), "{out}");
        // the plugin: "rifai la segmentazione"
        let invalidated = out["invalidated"].as_array().unwrap();
        for id in ["labels", "crowns"] {
            assert!(invalidated.iter().any(|c| c == id), "{id}: {out}");
        }
        let (a, b) = (data(&state), data(&next));
        assert_eq!(b.occlusal_flipped, BTreeSet::from([ScanRole::Lower]));
        assert_eq!(
            (&b.seeds, &b.labels, &b.crowns),
            (&a.seeds, &a.labels, &a.crowns)
        );
        assert_eq!(b.teeth_changes(ScanRole::Lower), [TeethChange::Orientation]);
        // every arch frame of the arch is turned over (X and Z, Y kept)
        let mesh = next
            .blobs
            .require(&next.case.scan(ScanRole::Lower).unwrap().mesh)
            .unwrap();
        let v: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let m0 = crate::manual::common::arch_to_case(&a, Arch::Lower, &v).unwrap();
        let m1 = crate::manual::common::arch_to_case(&b, Arch::Lower, &v).unwrap();
        assert_eq!(m1.z_axis, -m0.z_axis);
        assert_eq!(m1.x_axis, -m0.x_axis);
        assert_eq!(m1.y_axis, m0.y_axis);
        // again: back as it was
        let (back, out) = exec(SET_ARCH, &next, json!({ "flip_occlusal": true }));
        assert_eq!(out["occlusal_flipped"], false, "{out}");
        assert!(data(&back).occlusal_flipped.is_empty());
        assert!(!data(&back).teeth_out_of_date(ScanRole::Lower));
        // the same arch without a flip only confirms the role
        let mut guess = state.clone();
        guess.case.scans[0].role_source = RoleSource::DetectedUnsure;
        let (confirmed, out) = exec(SET_ARCH, &guess, json!({ "arch": "lower" }));
        assert_eq!(confirmed.case.scans[0].role_source, RoleSource::Explicit);
        assert_eq!(data(&confirmed), data(&guess));
        assert_eq!(out["renumbered"], false, "{out}");
    }

    #[test]
    fn the_flip_comes_with_the_arch_and_a_missing_scan_is_refused() {
        let state = demo("lower");
        let (next, _) = exec(
            SET_ARCH,
            &state,
            json!({ "scan": "lower", "arch": "upper", "flip_occlusal": true }),
        );
        assert_eq!(
            data(&next).occlusal_flipped,
            BTreeSet::from([ScanRole::Upper])
        );
        // and it moves with the scan
        let (back, _) = exec(SET_ARCH, &next, json!({ "arch": "lower" }));
        assert_eq!(
            data(&back).occlusal_flipped,
            BTreeSet::from([ScanRole::Lower])
        );
        let e = try_exec(
            SET_ARCH,
            &state,
            json!({ "scan": "upper", "arch": "lower" }),
        )
        .unwrap_err();
        assert!(matches!(e, CommandError::Invalid(_)), "{e:?}");
        assert!(try_exec(SET_ARCH, &CaseState::default(), Value::Null).is_err());
    }
}
