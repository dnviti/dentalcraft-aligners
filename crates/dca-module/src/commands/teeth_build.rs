//! The menu leaf "3D teeth" (`aligners.teeth.build`, the plugin's
//! `case_teeth.build_teeth`): the closed teeth and the gingiva, then what the
//! plugin does after them, in one undoable change.
//!
//! Each arch is built in one of two ways:
//!
//! * **all its teeth** (`build_teeth_iter`): the closed teeth (which measures
//!   the teeth first) and the gingiva; the final position and the aligners
//!   of the arch are cleared (`plan_json`, `setup_json` and `n_stages`
//!   emptied), since they were computed on the old teeth;
//! * **some teeth** (`_rebuild_teeth_iter`): the teeth named in `teeth`, or,
//!   without it, the teeth whose labels were corrected by hand
//!   ([`AlignersState::dirty`]), the labelled teeth with no solid and the
//!   solids whose tooth has no labels left. The teeth of the arch are
//!   measured again, each of those teeth is closed again from its labels (a
//!   tooth without labels is removed: a missing tooth) and the gingiva is
//!   built again; the other teeth keep their solids, frames and measures. A
//!   rebuilt tooth keeps its lock and its planned movement in the world (the
//!   plugin's target `T0 * R0^-1 * F`: the setup pose, stored relative to the
//!   rest frame, is carried from the old frame to the new one), a locked
//!   tooth goes back to rest (target `F`), a removed tooth loses them; the
//!   aligners of the arch are cleared (so their poses need no carrying), the
//!   setup is kept.
//!
//! Without `teeth`, an arch is built in the second way when it already has
//! its solids and teeth corrected by hand (the plugin rebuilds `only` the
//! teeth of `oa_label_dirty`; the solids then follow every label change),
//! otherwise in the first. An empty `teeth`, or teeth of an arch with no
//! labels, are refused before anything runs. Without an arch asked, every
//! arch with a scan and labels is built; an arch with a scan and no labels
//! is named in `arches` (`built: false` and the reason), never skipped in
//! silence. Then the diagnosis runs on the new teeth (`case_plan.analyze`).
//! The output lists every step and what was done: the teeth and the gingiva
//! of each arch, what was cleared and whether the diagnosis ran.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{CaseState, Fdi, ModuleState, Pose};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::rotation;
use dca_algo::teeth::{self, Which};
use dca_model::{AlignersState, MODULE_ID, ToothFrame};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::auto::{Step, run_step, with_span};
use super::messages;
use super::reconstruct::{aligners_data, arch_name};

pub(super) const RECONSTRUCT: &str = "aligners.preparation.reconstruction.automatic";
pub(super) const REBUILD_TOOTH: &str = "aligners.preparation.reconstruction.rebuild-tooth";
pub(super) const GINGIVA: &str = "aligners.preparation.gingiva";
pub(super) const FRAMES: &str = "aligners.teeth.frames";
pub(super) const DIAGNOSIS: &str = "aligners.diagnosis.space-analysis";

/// Parameters of `aligners.teeth.build`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TeethParams {
    /// Arches (default all the arches with teeth and gum separated).
    pub arch: Which,
    /// FDI numbers of the teeth to build again; the other teeth keep their
    /// solids. Absent: every tooth, or only the teeth corrected by hand when
    /// the arch already has its solids.
    pub teeth: Option<Vec<u8>>,
}

/// How an arch is built.
enum Mode {
    /// Every tooth.
    All,
    /// These teeth only.
    Some(BTreeSet<Fdi>),
}

/// The teeth with labels on the segmented scan of `arch` (`None` when the
/// arch is not segmented).
fn labelled_teeth(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<Option<BTreeSet<Fdi>>, CommandError> {
    let role = arch.role();
    let (Some(_), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
    };
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .as_ref()
        .ok_or_else(|| messages::labels_missing(arch))?;
    let set: BTreeSet<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
    Ok(Some(
        set.into_iter()
            .filter_map(|l| u8::try_from(l).ok().and_then(|n| Fdi::new(n).ok()))
            .filter(|f| Arch::of(*f) == arch)
            .collect(),
    ))
}

/// The teeth of `arch` that changed since its solids were built: corrected
/// by hand, labelled without a solid, or with a solid and no labels.
fn changed_teeth(data: &AlignersState, labelled: &BTreeSet<Fdi>, arch: Arch) -> BTreeSet<Fdi> {
    let of_arch = |f: &&Fdi| Arch::of(**f) == arch;
    let mut out: BTreeSet<Fdi> = data.dirty.iter().filter(of_arch).copied().collect();
    out.extend(labelled.iter().filter(|f| !data.crowns.contains_key(f)));
    out.extend(
        data.crowns
            .keys()
            .filter(of_arch)
            .filter(|f| !labelled.contains(f)),
    );
    out
}

/// The tooth numbers of `teeth`, each checked to be a tooth of the arches
/// asked for.
fn asked_teeth(teeth: &[u8], which: Which) -> Result<BTreeSet<Fdi>, CommandError> {
    let arches = which.arches();
    teeth
        .iter()
        .map(|&n| {
            let f = Fdi::new(n).map_err(|_| {
                messages::invalid(
                    "aligners-err-teeth-build-not-tooth",
                    &[("fdi", &n.to_string())],
                )
            })?;
            if !arches.contains(&Arch::of(f)) {
                return Err(messages::invalid(
                    "aligners-err-teeth-build-other-arch",
                    &[
                        ("fdi", &n.to_string()),
                        ("arch", &messages::arch_word(Arch::of(f))),
                    ],
                ));
            }
            Ok(f)
        })
        .collect()
}

/// Removes the final position (`setup`) and the aligners of `arch`: what
/// the plugin empties after building the teeth (`setup_json`, `plan_json`,
/// `n_stages`). Returns (setup cleared, stages cleared).
fn clear_plan(data: &mut AlignersState, arch: Arch, setup: bool) -> (bool, bool) {
    let mine = |f: &Fdi| Arch::of(*f) == arch;
    let mut setup_cleared = false;
    if setup {
        let s = &mut data.setup;
        let before = (s.poses.len(), s.locked.len(), s.ipr.len(), s.lines.len());
        s.poses.retain(|f, _| !mine(f));
        s.locked.retain(|f| !mine(f));
        s.ipr.retain(|r| !mine(&r.between[0]));
        s.lines.remove(&arch.role());
        setup_cleared = before != (s.poses.len(), s.locked.len(), s.ipr.len(), s.lines.len());
    }
    // the result of the last staging of the arch goes with its aligners
    let mut stages_cleared = data.staging_results.remove(&arch.role()).is_some();
    for st in &mut data.stages {
        let before = (st.poses.len(), st.ipr.len());
        st.poses.retain(|f, _| !mine(f));
        st.ipr.retain(|r| !mine(&r.between[0]));
        stages_cleared |= before != (st.poses.len(), st.ipr.len());
    }
    if data
        .stages
        .iter()
        .all(|s| s.poses.is_empty() && s.ipr.is_empty())
    {
        stages_cleared |= !data.stages.is_empty();
        data.stages.clear();
    }
    (setup_cleared, stages_cleared)
}

/// A tooth with no labels left: its solid, frame, measures and movement go
/// (the plugin removes its item).
fn remove_tooth(data: &mut AlignersState, f: Fdi) -> bool {
    let had = data.crowns.remove(&f).is_some() | data.tooth_frames.remove(&f).is_some();
    data.tooth_geometry.remove(&f);
    data.setup.poses.remove(&f);
    data.setup.locked.remove(&f);
    data.setup.ipr.retain(|r| !r.between.contains(&f));
    had
}

/// The setup pose `local` of a tooth whose frame goes from `old` to `new`,
/// with the same move in the world. The pose is relative to the rest pose of
/// the frame ([`teeth::rest_pose`], centre of resistance origin), so with
/// `R0` and `F` the old and new rest poses: `F^-1 * (R0 * local * R0^-1) * F`
/// (the plugin's target `T0 * R0^-1 * F`).
fn carried_pose(old: &ToothFrame, new: &ToothFrame, local: &Pose) -> Pose {
    let (old, new) = (teeth::rest_pose(old), teeth::rest_pose(new));
    let world = old.compose(local).compose(&old.inverse());
    rotation::canonical_pose(&new.inverse().compose(&world).compose(&new))
}

/// The steps run so far and how many are planned (for the progress bar).
struct Steps {
    records: Vec<Value>,
    total: usize,
}

/// Runs `step` on `current` and keeps its record; the stage moves the
/// progress bar inside its part.
fn step(
    current: &mut CaseState,
    run: &mut Steps,
    id: &'static str,
    params: Value,
    ctx: &JobCtx,
) -> Result<Value, CommandError> {
    let k = run.records.len();
    let n = run.total.max(k + 1) as f32;
    let (from, to) = (k as f32 / n, (k + 1) as f32 / n);
    ctx.set_fraction(from);
    let (next, record) = with_span(from, to, || {
        run_step(current, &Step::new(id, params), k, ctx)
    })?;
    *current = next;
    let out = record["output"].clone();
    run.records.push(record);
    Ok(out)
}

pub(super) fn run_teeth(
    state: &CaseState,
    params: TeethParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if params.teeth.as_ref().is_some_and(|t| t.is_empty()) {
        return Err(messages::invalid("aligners-err-teeth-build-empty", &[]));
    }
    let asked = params
        .teeth
        .as_deref()
        .map(|t| asked_teeth(t, params.arch))
        .transpose()?;

    // how each arch is built; an arch with a scan that cannot be built is
    // named in the output with the reason, never skipped in silence
    let mut modes: BTreeMap<Arch, Mode> = BTreeMap::new();
    let mut not_built: BTreeMap<Arch, String> = BTreeMap::new();
    for arch in params.arch.arches() {
        let Some(labelled) = labelled_teeth(state, &data, arch)? else {
            let named = asked
                .as_ref()
                .is_some_and(|a| a.iter().any(|f| Arch::of(*f) == arch));
            if params.arch != Which::All || named {
                return Err(messages::not_segmented(arch));
            }
            if state.case.scan(arch.role()).is_some() {
                let word = messages::arch_word(arch);
                let reason = messages::tr("aligners-err-arch-not-segmented", &[("arch", &word)]);
                not_built.insert(arch, reason);
            }
            continue;
        };
        let mode = match &asked {
            Some(asked) => {
                let mine: BTreeSet<Fdi> = asked
                    .iter()
                    .filter(|f| Arch::of(**f) == arch)
                    .copied()
                    .collect();
                if mine.is_empty() {
                    continue;
                }
                Mode::Some(mine)
            }
            None => {
                let built = data.crowns.keys().any(|f| Arch::of(*f) == arch)
                    && data.gingiva.contains_key(&arch.role());
                let changed = changed_teeth(&data, &labelled, arch);
                // the plugin's trigger: teeth corrected by hand (`oa_label_dirty`)
                if built && data.dirty.iter().any(|f| Arch::of(*f) == arch) {
                    Mode::Some(changed)
                } else {
                    Mode::All
                }
            }
        };
        modes.insert(arch, mode);
    }
    if modes.is_empty() {
        // nothing to build: the error of the first step, named as the user sees it
        let arch = json!({ "arch": params.arch });
        run_step(state, &Step::new(RECONSTRUCT, arch), 0, ctx)?;
        return Err(messages::none_segmented());
    }

    let mut current = state.clone();
    let planned: usize = modes
        .values()
        .map(|m| match m {
            Mode::All => 2,
            Mode::Some(todo) => todo.len() + 2,
        })
        .sum();
    let mut run = Steps {
        records: Vec::new(),
        total: planned + 1,
    };
    let mut report = Map::new();
    let mut removed = Vec::new();
    // every tooth: the closed teeth (measured first) and the gingiva
    let all: Vec<Arch> = modes
        .iter()
        .filter(|(_, m)| matches!(m, Mode::All))
        .map(|(a, _)| *a)
        .collect();
    if !all.is_empty() {
        let which = if all.len() == 2 {
            "all"
        } else {
            arch_name(all[0])
        };
        let arch = json!({ "arch": which });
        step(&mut current, &mut run, RECONSTRUCT, arch.clone(), ctx)?;
        step(&mut current, &mut run, GINGIVA, arch, ctx)?;
    }
    // some teeth: measured again, closed again one by one, then the gingiva
    for (&arch, mode) in &modes {
        let Mode::Some(todo) = mode else {
            continue;
        };
        let arch_json = json!({ "arch": arch_name(arch) });
        let before = aligners_data(&current)?;
        step(&mut current, &mut run, FRAMES, arch_json.clone(), ctx)?;
        let mut d = aligners_data(&current)?;
        // the other teeth keep their frames and measures (the plugin keeps
        // their items: `_teeth_frames_iter`, the stored frames winning)
        let kept = |f: &&Fdi| Arch::of(**f) == arch && !todo.contains(*f);
        for (&f, fr) in before.tooth_frames.iter().filter(|(f, _)| kept(f)) {
            d.tooth_frames.insert(f, *fr);
        }
        for (&f, g) in before.tooth_geometry.iter().filter(|(f, _)| kept(f)) {
            d.tooth_geometry.insert(f, *g);
        }
        let mut gone = Vec::new();
        for &f in todo {
            let Some(new) = d.tooth_frames.get(&f).copied() else {
                // no labels left: a missing tooth
                if remove_tooth(&mut d, f) {
                    gone.push(f.get());
                }
                continue;
            };
            // the movement in the world is kept, a locked tooth goes to rest
            if d.setup.locked.contains(&f) {
                d.setup.poses.remove(&f);
            } else if let (Some(old), Some(p)) =
                (before.tooth_frames.get(&f), d.setup.poses.get(&f).copied())
            {
                d.setup.poses.insert(f, carried_pose(old, &new, &p));
            }
        }
        current.set_module(MODULE_ID, ModuleState::new(d));
        for &f in todo {
            if aligners_data(&current)?.tooth_frames.contains_key(&f) {
                step(
                    &mut current,
                    &mut run,
                    REBUILD_TOOTH,
                    json!({ "fdi": f.get() }),
                    ctx,
                )?;
            }
        }
        removed.extend(gone);
        step(&mut current, &mut run, GINGIVA, arch_json, ctx)?;
    }

    // what was planned on the old teeth
    let mut d = aligners_data(&current)?;
    let mut setup_cleared = false;
    let mut stages_cleared = false;
    for (&arch, mode) in &modes {
        let (s, st) = clear_plan(&mut d, arch, matches!(mode, Mode::All));
        setup_cleared |= s;
        stages_cleared |= st;
        match mode {
            Mode::All => d.dirty.retain(|f| Arch::of(*f) != arch),
            Mode::Some(todo) => {
                d.dirty.retain(|f| !todo.contains(f));
                // teeth rebuilt by hand (the plugin's `mark_manual(obj,
                // "denti")`): the doubts of the check on the arch are answered
                d.forget_separation(arch.role());
            }
        }
        // the 3D teeth of the arch match its colours and marks again
        d.record_teeth_basis(arch.role());
        let teeth: Vec<u8> = d
            .crowns
            .keys()
            .filter(|f| Arch::of(**f) == arch)
            .map(|f| f.get())
            .collect();
        let rebuilt: Vec<u8> = match mode {
            Mode::All => teeth.clone(),
            Mode::Some(todo) => todo
                .iter()
                .filter(|f| d.crowns.contains_key(f))
                .map(|f| f.get())
                .collect(),
        };
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "built": true,
                "mode": if matches!(mode, Mode::All) { "all" } else { "some" },
                "teeth": teeth.len(),
                "rebuilt": rebuilt,
                "gingiva": d.gingiva.contains_key(&arch.role()),
            }),
        );
    }
    for (arch, reason) in not_built {
        report.insert(
            arch_name(arch).to_owned(),
            json!({ "built": false, "reason": reason }),
        );
    }
    let any_tooth = !d.tooth_frames.is_empty();
    current.set_module(MODULE_ID, ModuleState::new(d));

    // the diagnosis of the new teeth (`case_plan.analyze`)
    let diagnosis = any_tooth;
    if diagnosis {
        step(&mut current, &mut run, DIAGNOSIS, Value::Null, ctx)?;
    }
    ctx.set_fraction(1.0);
    Ok((
        current,
        json!({
            "steps": run.records,
            "arches": report,
            "removed": removed,
            "setup_cleared": setup_cleared,
            "stages_cleared": stages_cleared,
            "diagnosis": diagnosis,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;
    use glam::DVec3;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    const BUILD: &str = "aligners.teeth.build";

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    /// The lower demo with a final position and its aligners.
    fn planned() -> CaseState {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        exec("aligners.planning.aligners", &s, Value::Null)
            .unwrap()
            .0
    }

    #[test]
    fn building_every_tooth_clears_the_plan_and_makes_the_diagnosis() {
        let s = planned();
        let before = aligners_data(&s).unwrap();
        assert!(!before.setup.poses.is_empty() && !before.stages.is_empty());
        let (next, out) = exec(BUILD, &s, Value::Null).unwrap();
        let d = aligners_data(&next).unwrap();
        let ids: Vec<&str> = out["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [RECONSTRUCT, GINGIVA, DIAGNOSIS]);
        assert_eq!(out["arches"]["lower"]["mode"], "all");
        assert_eq!(
            out["arches"]["lower"]["teeth"].as_u64().unwrap() as usize,
            d.crowns.len()
        );
        assert_eq!(out["arches"]["lower"]["gingiva"], true);
        assert_eq!(out["diagnosis"], true);
        assert_eq!(out["setup_cleared"], true);
        assert_eq!(out["stages_cleared"], true);
        assert!(d.setup.poses.is_empty() && d.stages.is_empty());
        assert!(d.diagnosis.arches.contains_key(&ScanRole::Lower));
        assert!(d.gingiva.contains_key(&ScanRole::Lower));
        // one undoable change: the first state is untouched
        assert_eq!(aligners_data(&s).unwrap(), before);
    }

    #[test]
    fn every_arch_is_built_and_an_arch_that_cannot_be_is_named() {
        let both = super::super::occlusion::tests::two_demo_arches();
        let (next, out) = exec(BUILD, &both, Value::Null).unwrap();
        let d = aligners_data(&next).unwrap();
        for (arch, role) in [("upper", ScanRole::Upper), ("lower", ScanRole::Lower)] {
            assert_eq!(out["arches"][arch]["built"], true, "{arch}");
            assert!(
                d.crowns
                    .keys()
                    .any(|f| f.is_upper() == (role == ScanRole::Upper))
            );
            assert!(d.gingiva.contains_key(&role), "{arch}");
            assert!(!d.teeth_out_of_date(role), "{arch}");
        }
        // the upper arch without its colours: the lower one is built, the
        // upper one is named with the reason
        let mut s = both.clone();
        let mut d = aligners_data(&s).unwrap();
        d.labels.remove(&ScanRole::Upper);
        s.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(BUILD, &s, Value::Null).unwrap();
        assert_eq!(out["arches"]["lower"]["built"], true);
        assert_eq!(out["arches"]["upper"]["built"], false);
        let reason = out["arches"]["upper"]["reason"].as_str().unwrap();
        assert!(reason.contains("upper"), "{reason}");
        let d = aligners_data(&next).unwrap();
        assert!(d.crowns.keys().all(|f| !f.is_upper()));
        // an arch with no scan is not an arch to build
        let lower = exec("aligners.demo", &CaseState::default(), Value::Null)
            .unwrap()
            .0;
        let (_, out) = exec(BUILD, &lower, Value::Null).unwrap();
        assert!(out["arches"].get("upper").is_none(), "{out}");
    }

    #[test]
    fn the_gingiva_of_both_arches_is_closed_with_a_flat_base() {
        use super::super::make_base::tests::opened_under_gum;
        let mut open = super::super::occlusion::tests::two_demo_arches();
        for role in [ScanRole::Upper, ScanRole::Lower] {
            open = opened_under_gum(open, role);
        }
        // as read (open), and closed with a base as the import closes it
        let mut based = open.clone();
        for arch in ["upper", "lower"] {
            based = exec("aligners.case.make-base", &based, json!({ "arch": arch }))
                .unwrap()
                .0;
        }
        for state in [&open, &based] {
            let (next, out) = exec(BUILD, state, Value::Null).unwrap();
            let d = aligners_data(&next).unwrap();
            let gum_step = out["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["id"] == GINGIVA)
                .unwrap();
            for (arch, role) in [("upper", ScanRole::Upper), ("lower", ScanRole::Lower)] {
                let scan = next.mesh(&next.case.scan(role).unwrap().mesh).unwrap();
                let open_scan =
                    crate::open_scan::open_part(&d, &next.case.scan(role).unwrap().mesh, scan)
                        .unwrap();
                assert!(open_scan.validate().boundary_edges > 0, "{arch}");
                let mesh = next.mesh(&d.gingiva[&role]).unwrap();
                let report = mesh.validate();
                assert_eq!(report.boundary_edges, 0, "{arch}: {report:?}");
                assert_eq!(report.non_manifold_edges, 0, "{arch}: {report:?}");
                assert!(report.is_watertight(), "{arch}: {report:?}");
                let g = dca_algo::gingiva::GumSurface::from_mesh(mesh).unwrap();
                let base = g.base.iter().filter(|&&b| b).count();
                assert!(base > 0, "{arch}");
                assert_eq!(gum_step["output"][arch]["base_vertices"], base, "{arch}");
                assert!(
                    gum_step["output"][arch]["base_edges"].as_u64().unwrap() > 0,
                    "{arch}"
                );
                // without the base: the gingiva of the open scan, its margin cut after it
                let gum = g.without_base();
                let margin = gum.margin.iter().filter(|&&m| m).count();
                assert_eq!(
                    gum.vertices.len(),
                    open_scan.vertex_count() + margin,
                    "{arch}"
                );
                assert!(gum.tris.len() < mesh.face_count(), "{arch}");
                assert!(!gum.to_mesh().validate().is_watertight(), "{arch}");
            }
        }
    }

    /// `p` moved by the rest pose of `frame` into the world (the plan's
    /// convention: centre of resistance origin).
    fn world(frame: &ToothFrame, p: &Pose) -> Pose {
        let rest = teeth::rest_pose(frame);
        rest.compose(p).compose(&rest.inverse())
    }

    #[test]
    fn a_tooth_corrected_by_hand_is_the_only_one_built_again() {
        let s = planned();
        let (built, _) = exec(BUILD, &s, Value::Null).unwrap();
        let mut d = aligners_data(&built).unwrap();
        let old = d.crowns.clone();
        d.dirty.insert(fdi(36));
        d.dirty.insert(fdi(37));
        // old frames that the new measure does not give again
        let nudge = Pose::new(
            DVec3::new(0.8, -0.3, 0.2),
            glam::DQuat::from_rotation_z(0.15),
        );
        for f in [36, 41] {
            let fr = d.tooth_frames.get_mut(&fdi(f)).unwrap();
            fr.pose = nudge.compose(&fr.pose);
        }
        let old_36 = d.tooth_frames[&fdi(36)];
        let moved = Pose::new(
            DVec3::new(0.5, 0.1, 0.0),
            glam::DQuat::from_rotation_x(0.05),
        );
        d.setup.poses.insert(fdi(36), moved);
        d.setup.poses.insert(fdi(37), moved);
        d.setup.locked.insert(fdi(37));
        d.stages = aligners_data(&s).unwrap().stages;
        let kept_frames = d.tooth_frames.clone();
        let kept_geometry = d.tooth_geometry.clone();
        let mut st = built.clone();
        st.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(BUILD, &st, Value::Null).unwrap();
        assert_eq!(out["arches"]["lower"]["mode"], "some");
        assert_eq!(out["arches"]["lower"]["rebuilt"], json!([36, 37]));
        let d = aligners_data(&next).unwrap();
        assert!(d.dirty.is_empty());
        // the other teeth keep their solids, frames and measures
        for (f, b) in &old {
            if ![36, 37].contains(&f.get()) {
                assert_eq!(d.crowns[f], *b, "{f}");
                assert_eq!(d.tooth_frames[f], kept_frames[f], "{f}");
                assert_eq!(d.tooth_geometry.get(f), kept_geometry.get(f), "{f}");
            }
        }
        // the rebuilt tooth is measured again and keeps its move in the world
        let new_36 = d.tooth_frames[&fdi(36)];
        assert!(!new_36.pose.approx_eq(&old_36.pose, 1e-3, 1e-3));
        let p = d.setup.poses[&fdi(36)];
        assert!(
            !p.approx_eq(&moved, 1e-3, 1e-3),
            "the local pose follows the frame"
        );
        assert!(world(&new_36, &p).approx_eq(&world(&old_36, &moved), 1e-9, 1e-9));
        // a locked tooth keeps its lock and goes back to rest (the plugin's target F)
        assert!(d.crowns.contains_key(&fdi(37)));
        assert!(d.setup.locked.contains(&fdi(37)));
        assert!(!d.setup.poses.contains_key(&fdi(37)));
        assert_eq!(out["setup_cleared"], false);
        assert!(d.stages.is_empty(), "the aligners are planned again");
        assert_eq!(out["diagnosis"], true);
        // named teeth: the same, the others untouched
        let (_, out) = exec(BUILD, &built, json!({ "teeth": [41, 42] })).unwrap();
        assert_eq!(out["arches"]["lower"]["rebuilt"], json!([41, 42]));
    }

    #[test]
    fn carrying_a_pose_keeps_the_move_in_the_world() {
        let frame = |t: DVec3, q: glam::DQuat, cres: DVec3| ToothFrame {
            pose: Pose::new(t, q),
            center_of_resistance: cres,
        };
        // the centre of resistance far from the frame origin: the pivot matters
        let old = frame(
            DVec3::new(1.0, 2.0, 3.0),
            glam::DQuat::from_rotation_y(0.4),
            DVec3::new(0.2, -0.1, -9.0),
        );
        let new = frame(
            DVec3::new(-1.0, 0.5, 2.0),
            glam::DQuat::from_rotation_z(-0.7),
            DVec3::new(-0.3, 0.4, -8.5),
        );
        let p = Pose::new(
            DVec3::new(0.3, -0.2, 0.1),
            glam::DQuat::from_rotation_x(0.2),
        );
        let q = carried_pose(&old, &new, &p);
        assert!(world(&new, &q).approx_eq(&world(&old, &p), 1e-12, 1e-12));
        assert!(carried_pose(&old, &old, &p).approx_eq(&p, 1e-12, 1e-12));
        // turning about the frame origin instead would move the tooth elsewhere
        let about_origin =
            |fr: &ToothFrame, l: &Pose| fr.pose.compose(l).compose(&fr.pose.inverse());
        assert!(about_origin(&old, &p).t.distance(world(&old, &p).t) > 1.0);
        // a stored quaternion with w < 0 comes back with w >= 0
        let flipped = Pose { t: p.t, q: -p.q };
        let r = carried_pose(&old, &new, &flipped);
        assert!(r.q.w >= 0.0 && r.approx_eq(&q, 1e-12, 1e-12));
    }

    #[test]
    fn an_empty_list_or_teeth_of_an_unsegmented_arch_are_refused() {
        let s = planned();
        let (built, _) = exec(BUILD, &s, Value::Null).unwrap();
        let err = exec(BUILD, &built, json!({ "teeth": [] })).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("no tooth to build again")),
            "{err}"
        );
        // the lower demo has no upper scan: tooth 11 cannot be built
        let err = exec(BUILD, &built, json!({ "teeth": [11] })).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("upper")),
            "{err}"
        );
        let err = exec(BUILD, &built, json!({ "teeth": [11, 41] })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
    }

    #[test]
    fn a_tooth_without_labels_is_removed_and_bad_numbers_are_refused() {
        let s = planned();
        let (built, _) = exec(BUILD, &s, Value::Null).unwrap();
        let mut st = built.clone();
        let mut d = aligners_data(&st).unwrap();
        let mut mesh =
            dc_mesh::TriMesh::clone(st.blobs.require(&d.labels[&ScanRole::Lower]).unwrap());
        for l in mesh.vertex_labels.as_mut().unwrap() {
            if *l == 47 {
                *l = 0;
            }
        }
        let r = st.blobs.insert(mesh);
        d.labels.insert(ScanRole::Lower, r);
        d.dirty.insert(fdi(47));
        st.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(BUILD, &st, Value::Null).unwrap();
        assert_eq!(out["removed"], json!([47]));
        let d = aligners_data(&next).unwrap();
        assert!(!d.crowns.contains_key(&fdi(47)));
        assert!(!d.tooth_frames.contains_key(&fdi(47)));
        assert!(!d.setup.poses.contains_key(&fdi(47)));

        let err = exec(BUILD, &built, json!({ "teeth": [19] })).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err}");
        let err = exec(BUILD, &built, json!({ "arch": "lower", "teeth": [11] })).unwrap_err();
        assert!(err.to_string().contains("11"), "{err}");
    }
}
