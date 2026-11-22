//! Command of the check of the segmented teeth (`dca_algo::seed_check`, the
//! plugin's `case_auto.check_segmentation_iter`).
//!
//! `aligners.preparation.segmentation.check` measures the teeth of the
//! segmentation of every segmented arch (or the one asked for): boundaries
//! with no interproximal fold, fragments and crowns of two teeth. With
//! `correct` the seeds are corrected (parts of one crown merged, fragments
//! and seeds in a gap removed, a crown of two teeth split) and the arch is
//! segmented again, at most [`SEG_ROUNDS`] times; after a correction the
//! teeth are numbered again from their widths and segmented with the new
//! numbers, unless some teeth are still implausible (then the numbers of the
//! proposal are kept and a note says so). Without `correct` (seeds placed by
//! hand) the problems are only reported. `aligners.auto` runs it after the
//! segmentation, correcting the arches whose seeds it proposed.
//!
//! New seeds drop what was computed from the old ones
//! ([`dca_model::AlignersState::replace_arch_seeds`]), as the automatic
//! numbering does, and the separations run with the parameters of the case
//! settings; the ids of what went are in the output (`invalidated`). The
//! problems and the notes left on each checked arch are kept in the case
//! (`segmentation_doubts`, the plugin's `segmentation_doubts`): the export,
//! the report and `aligners.auto` warn about them.
//!
//! An arch with no arch curve (not a dental arch) is not checked: it is
//! reported in `skipped` and the command goes on, as in the plugin.

use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::autoseed;
use dca_algo::seed_check::{self, Action, Issue, NumberNote, Report, SEG_ROUNDS};
use dca_algo::teeth::Which;
use dca_model::{MODULE_ID, Seed};
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::auto::{Step, run_step};
use super::messages::{self, tr};
use super::reconstruct::{aligners_data, arch_name};
use super::segmentation::{SEGMENT, in_cleared_order};

/// Id of the check.
pub(crate) const CHECK: &str = "aligners.preparation.segmentation.check";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: CHECK,
        label: "aligners-cmd-segmentation-check",
        doc: "aligners-cmd-segmentation-check-doc",
        params: CheckParams,
        run: run_check,
    }]
}

/// Parameters of the check.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CheckParams {
    /// Arches (default all the segmented arches).
    pub arch: Which,
    /// Correct the seeds and segment again (automatic seeds); without it the
    /// problems are only reported (seeds placed by hand).
    pub correct: bool,
    /// Third molars may be present (numbering from the widths).
    pub third_molars: bool,
    /// FDI numbers of the teeth known to be missing (numbering from the
    /// widths).
    pub missing: Vec<u8>,
}

/// The segmented scan of an arch in case coordinates.
struct Scan {
    vertices: Vec<DVec3>,
    tris: Vec<[u32; 3]>,
    labels: Vec<u16>,
}

fn scan_of(state: &CaseState, arch: Arch) -> Result<Option<Scan>, CommandError> {
    let data = aligners_data(state)?;
    let role = arch.role();
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
    };
    let closed = state.blobs.require(&scan.mesh)?;
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .as_ref()
        .ok_or_else(|| messages::labels_missing(arch))?;
    if labels.len() != closed.vertex_count() {
        return Err(messages::labels_mismatch(
            arch,
            labels.len(),
            closed.vertex_count(),
        ));
    }
    // the scan as it was read, as the separation it checks
    let mesh = crate::open_scan::open_part(&data, &scan.mesh, closed)?;
    Ok(Some(Scan {
        vertices: mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
        tris: mesh.tris.clone(),
        labels: crate::open_scan::open_labels(labels, &mesh).to_vec(),
    }))
}

fn seeds_of(state: &CaseState, arch: Arch) -> Result<Vec<(Fdi, DVec3)>, CommandError> {
    Ok(aligners_data(state)?
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == arch)
        .map(|s| (s.fdi, s.position))
        .collect())
}

/// Puts `seeds` in the place of the seeds of `arch`; what the old seeds gave
/// goes, and its ids are returned (without `labels`, which the separation
/// that follows makes again).
fn put_seeds(
    state: &mut CaseState,
    arch: Arch,
    seeds: &[(Fdi, DVec3)],
) -> Result<Vec<&'static str>, CommandError> {
    let mut data = aligners_data(state)?;
    let seeds: Vec<Seed> = seeds
        .iter()
        .map(|&(fdi, position)| Seed { fdi, position })
        .collect();
    let mut dropped = data.replace_arch_seeds(arch.role(), &seeds);
    dropped.retain(|&id| id != "labels");
    state.set_module(MODULE_ID, ModuleState::new(data));
    Ok(dropped)
}

/// The arch segmented again from its seeds (with the parameters of the case
/// settings); the ids of what the old labels gave and went.
fn segment(
    state: &mut CaseState,
    arch: Arch,
    k: usize,
    ctx: &JobCtx,
) -> Result<Vec<&'static str>, CommandError> {
    let step = Step::new(SEGMENT, json!({ "arch": arch_name(arch) }));
    let (next, record) = run_step(state, &step, k, ctx)?;
    *state = next;
    let ids = record["output"]["invalidated"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(in_cleared_order(ids))
}

/// Keeps the problems and the notes of the check of `arch` in the case (none:
/// the arch has no doubts).
fn keep_doubts(state: &mut CaseState, arch: Arch, doubts: Vec<String>) -> Result<(), CommandError> {
    let mut data = aligners_data(state)?;
    if doubts.is_empty() {
        data.segmentation_doubts.remove(&arch.role());
    } else {
        data.segmentation_doubts.insert(arch.role(), doubts);
    }
    state.set_module(MODULE_ID, ModuleState::new(data));
    Ok(())
}

/// A correction in words.
fn describe_action(a: &Action) -> String {
    match *a {
        Action::Drop { fdi } => tr("aligners-check-act-drop", &[("fdi", &fdi.to_string())]),
        Action::Merge { keep, drop } => tr(
            "aligners-check-act-merge",
            &[("keep", &keep.to_string()), ("drop", &drop.to_string())],
        ),
        Action::Split { fdi, .. } => tr("aligners-check-act-split", &[("fdi", &fdi.to_string())]),
    }
}

/// A problem in words.
fn describe_issue(i: &Issue) -> String {
    let mm = |x: f64| format!("{x:.1}");
    match *i {
        Issue::NoFold { a, b, len_mm } => tr(
            "aligners-check-issue-no-fold",
            &[
                ("a", &a.to_string()),
                ("b", &b.to_string()),
                ("mm", &format!("{len_mm:.0}")),
            ],
        ),
        Issue::Narrow { fdi, width_mm } => tr(
            "aligners-check-issue-narrow",
            &[("fdi", &fdi.to_string()), ("mm", &mm(width_mm))],
        ),
        Issue::Wide { fdi, width_mm } => tr(
            "aligners-check-issue-wide",
            &[("fdi", &fdi.to_string()), ("mm", &mm(width_mm))],
        ),
    }
}

/// A note on the numbering in words.
fn describe_note(n: &NumberNote) -> String {
    match n {
        NumberNote::OnePremolar { quadrant, fdi } => tr(
            "aligners-check-note-one-premolar",
            &[
                ("quadrant", &quadrant.to_string()),
                ("fdi", &fdi.to_string()),
            ],
        ),
        NumberNote::Uncertain { pairs } => {
            let list: Vec<String> = pairs
                .iter()
                .map(|(x, y)| {
                    tr(
                        "aligners-check-or",
                        &[("a", &x.to_string()), ("b", &y.to_string())],
                    )
                })
                .collect();
            tr(
                "aligners-check-note-uncertain",
                &[("pairs", &list.join(", "))],
            )
        }
    }
}

/// The teeth the renumbering leaves out: those of the parameters and the
/// ones marked missing by hand (the manual tools), as the numbering does.
fn missing_of(state: &CaseState, params: &CheckParams) -> Result<Vec<u16>, CommandError> {
    let mut missing: Vec<u16> = params.missing.iter().map(|&m| u16::from(m)).collect();
    missing.extend(
        aligners_data(state)?
            .missing
            .iter()
            .map(|f| u16::from(f.get())),
    );
    missing.sort_unstable();
    missing.dedup();
    Ok(missing)
}

fn check(scan: &Scan, arch: Arch, data: &autoseed::ArchData) -> Report {
    seed_check::check_teeth(&scan.vertices, &scan.tris, &scan.labels, arch, data)
}

/// The check (and the corrections) of one arch; the ids of what the
/// corrections dropped go to `invalidated`.
fn check_arch(
    state: &mut CaseState,
    arch: Arch,
    params: &CheckParams,
    invalidated: &mut Vec<&'static str>,
    ctx: &JobCtx,
) -> Result<Option<Value>, CommandError> {
    let Some(scan) = scan_of(state, arch)? else {
        return Ok(None);
    };
    // the arch curve of the scan (as the numbering computes it, the occlusal
    // side turned over by hand included): the same scan for every round,
    // only the labels change
    let flipped = aligners_data(state)?
        .occlusal_flipped
        .contains(&arch.role());
    let Ok(data) = autoseed::arch_data_oriented(&scan.vertices, &scan.tris, flipped) else {
        // nothing checked: what an earlier check left is no longer known
        keep_doubts(state, arch, Vec::new())?;
        return Ok(Some(json!({ "skipped": true })));
    };
    let mut rep = check(&scan, arch, &data);
    let mut actions: Vec<Action> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut renumbered: Vec<(u16, u16)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut rounds = 0;
    let mut k = 0;
    if params.correct {
        for _ in 0..SEG_ROUNDS {
            ctx.check_cancelled()?;
            let acts = seed_check::plan_corrections(&rep);
            if acts.is_empty() {
                break;
            }
            let cur = scan_of(state, arch)?.expect("segmented");
            let seeds = seeds_of(state, arch)?;
            let new =
                seed_check::apply_corrections(&cur.vertices, &cur.labels, &seeds, &acts, &data);
            invalidated.extend(put_seeds(state, arch, &new)?);
            done.extend(acts.iter().map(describe_action));
            actions.extend(acts);
            invalidated.extend(segment(state, arch, k, ctx)?);
            k += 1;
            rounds += 1;
            rep = check(&scan_of(state, arch)?.expect("segmented"), arch, &data);
        }
        if !done.is_empty() && rep.doubtful() {
            // teeth still implausible: their widths do not count the teeth,
            // the numbers of the proposal are kept, to be checked
            notes.push(tr("aligners-check-note-not-renumbered", &[]));
        } else if !done.is_empty() {
            let missing = missing_of(state, params)?;
            let (new, more) =
                seed_check::renumber_spans(&rep.spans, arch, params.third_molars, &missing);
            notes.extend(more.iter().map(describe_note));
            let seeds = seeds_of(state, arch)?;
            let renamed: Vec<(Fdi, DVec3)> = seeds
                .iter()
                .filter_map(|&(f, p)| {
                    let g = new.get(&u16::from(f.get()))?;
                    let g = u8::try_from(*g).ok().and_then(|n| Fdi::new(n).ok())?;
                    Some((g, p))
                })
                .collect();
            let same_teeth = {
                let mut a: Vec<Fdi> = renamed.iter().map(|s| s.0).collect();
                let mut b: Vec<Fdi> = seeds.iter().map(|s| s.0).collect();
                a.sort_unstable();
                b.sort_unstable();
                a == b
            };
            if !same_teeth || new.iter().any(|(f, g)| f != g) {
                renumbered = new
                    .iter()
                    .filter(|(f, g)| f != g)
                    .map(|(f, g)| (*f, *g))
                    .collect();
                if !renumbered.is_empty() {
                    let list: Vec<String> = renumbered
                        .iter()
                        .map(|(f, g)| format!("{f}->{g}"))
                        .collect();
                    done.push(tr(
                        "aligners-check-act-renumbered",
                        &[("list", &list.join(", "))],
                    ));
                }
                invalidated.extend(put_seeds(state, arch, &renamed)?);
                invalidated.extend(segment(state, arch, k, ctx)?);
                rep = check(&scan_of(state, arch)?.expect("segmented"), arch, &data);
            }
        }
    }
    let issues = seed_check::report_issues(&rep);
    let problems: Vec<String> = issues.iter().map(describe_issue).collect();
    // what is left implausible stays in the case, for the export to warn
    let doubts = problems.iter().chain(&notes).cloned().collect();
    keep_doubts(state, arch, doubts)?;
    Ok(Some(json!({
        "rounds": rounds,
        "actions": actions,
        "renumbered": renumbered,
        "corrections": done,
        "issues": issues,
        "problems": problems,
        "notes": notes,
        "spans": rep.spans,
    })))
}

fn run_check(
    state: &CaseState,
    params: CheckParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    for &n in &params.missing {
        crate::manual::common::fdi_of(n)?;
    }
    let mut next = state.clone();
    let mut out = Map::new();
    let mut skipped = Vec::new();
    let mut invalidated = Vec::new();
    for arch in params.arch.arches() {
        match check_arch(&mut next, arch, &params, &mut invalidated, ctx)? {
            None if params.arch != Which::All => return Err(messages::not_segmented(arch)),
            None => {}
            Some(v) if v["skipped"] == true => skipped.push(arch_name(arch)),
            Some(v) => {
                out.insert(arch_name(arch).to_owned(), v);
            }
        }
    }
    if out.is_empty() && skipped.is_empty() {
        return Err(messages::none_segmented());
    }
    // the blobs of the dropped colours, solids and gingiva go too
    let before = state.referenced_blobs();
    let after = next.referenced_blobs();
    let stale: Vec<_> = before.difference(&after).copied().collect();
    crate::manual::common::finish_blobs(&mut next, &stale);
    out.insert("skipped".to_owned(), json!(skipped));
    out.insert(
        "invalidated".to_owned(),
        json!(in_cleared_order(invalidated)),
    );
    Ok((next, Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn lower_demo() -> CaseState {
        exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": "lower" }),
        )
        .unwrap()
        .0
    }

    /// The lower demo with a fragment of a tooth on the top of 46, labelled
    /// and seeded as 48: what a seed proposal that saw one tooth too many
    /// leaves.
    fn with_fragment() -> CaseState {
        let mut s = lower_demo();
        let mut d = aligners_data(&s).unwrap();
        assert!(d.seeds.iter().all(|x| x.fdi.get() != 48));
        let seed = d.seeds.iter().find(|x| x.fdi.get() == 46).unwrap().position;
        let mut mesh =
            dc_mesh::TriMesh::clone(s.blobs.require(&d.labels[&ScanRole::Lower]).unwrap());
        let mut n = 0;
        for (p, l) in mesh
            .positions
            .iter()
            .zip(mesh.vertex_labels.as_mut().unwrap())
        {
            if *l == 46 && (p.as_dvec3() - seed).length() < 1.2 {
                *l = 48;
                n += 1;
            }
        }
        assert!(n > 3, "{n}");
        let r = s.blobs.insert(mesh);
        d.labels.insert(ScanRole::Lower, r);
        d.seeds.push(Seed {
            fdi: Fdi::new(48).unwrap(),
            position: seed,
        });
        s.set_module(MODULE_ID, ModuleState::new(d));
        s
    }

    #[test]
    fn the_teeth_of_the_demo_are_checked_and_the_seeds_kept() {
        let s = lower_demo();
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower" })).unwrap();
        let lower = &out["lower"];
        assert_eq!(lower["rounds"], 0);
        assert!(lower["corrections"].as_array().unwrap().is_empty());
        assert!(lower["spans"].as_object().unwrap().len() >= 10, "{lower}");
        assert_eq!(
            aligners_data(&next).unwrap().seeds,
            aligners_data(&s).unwrap().seeds
        );
        // no upper scan
        assert!(exec(CHECK, &s, json!({ "arch": "upper" })).is_err());
    }

    #[test]
    fn a_fragment_is_reported_and_with_correct_loses_its_seed() {
        let s = with_fragment();
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower" })).unwrap();
        let issues = out["lower"]["issues"].as_array().unwrap();
        assert!(
            issues
                .iter()
                .any(|i| i["kind"] == "narrow" && i["fdi"] == 48),
            "{issues:?}"
        );
        assert!(!out["lower"]["problems"].as_array().unwrap().is_empty());
        // seeds placed by hand: only reported
        assert_eq!(
            aligners_data(&next).unwrap().seeds,
            aligners_data(&s).unwrap().seeds
        );
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower", "correct": true })).unwrap();
        let lower = &out["lower"];
        assert!(lower["rounds"].as_u64().unwrap() >= 1, "{lower}");
        assert_eq!(lower["actions"][0], json!({ "kind": "drop", "fdi": 48 }));
        let d = aligners_data(&next).unwrap();
        assert!(d.seeds.iter().all(|x| x.fdi.get() != 48));
        let labels = next
            .blobs
            .require(&d.labels[&ScanRole::Lower])
            .unwrap()
            .vertex_labels
            .clone()
            .unwrap();
        assert!(labels.iter().all(|&l| l != 48), "segmented again");
    }

    /// A tooth marked missing by hand stays missing when the check
    /// renumbers the teeth after a correction: no seed carries its number.
    #[test]
    fn a_tooth_marked_missing_is_never_renumbered_back() {
        let mut s = with_fragment();
        let mut d = aligners_data(&s).unwrap();
        d.seeds.retain(|x| x.fdi.get() != 35);
        d.missing.insert(Fdi::new(35).unwrap());
        s.set_module(MODULE_ID, ModuleState::new(d));
        let params: CheckParams = serde_json::from_value(json!({ "arch": "lower" })).unwrap();
        assert_eq!(missing_of(&s, &params).unwrap(), vec![35]);
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower", "correct": true })).unwrap();
        let lower = &out["lower"];
        assert!(lower["rounds"].as_u64().unwrap() >= 1, "{lower}");
        let d = aligners_data(&next).unwrap();
        assert!(
            d.seeds.iter().all(|x| !d.missing.contains(&x.fdi)),
            "{lower}"
        );
    }

    /// A check that cannot run (no arch curve on the scan) says so and
    /// leaves no doubt of an earlier check in the case.
    #[test]
    fn a_skipped_check_clears_the_doubts_of_an_earlier_one() {
        let mut s = lower_demo();
        let i = s
            .case
            .scans
            .iter()
            .position(|x| x.role == ScanRole::Lower)
            .unwrap();
        let mut mesh = dc_mesh::TriMesh::clone(s.blobs.require(&s.case.scans[i].mesh).unwrap());
        mesh.positions.fill(glam::Vec3::ZERO);
        s.case.scans[i].mesh = s.blobs.insert(mesh);
        let mut d = aligners_data(&s).unwrap();
        d.segmentation_doubts
            .insert(ScanRole::Lower, vec!["stale".to_owned()]);
        s.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower" })).unwrap();
        assert_eq!(out["skipped"], json!(["lower"]), "{out}");
        let d = aligners_data(&next).unwrap();
        assert!(!d.segmentation_doubts.contains_key(&ScanRole::Lower));
    }

    /// A deciduous tooth among the missing ones is refused by name.
    #[test]
    fn a_deciduous_missing_tooth_is_refused() {
        let s = lower_demo();
        let e = exec(CHECK, &s, json!({ "arch": "lower", "missing": [75] })).unwrap_err();
        assert!(e.to_string().contains("75"), "{e}");
        assert!(e.to_string().contains("deciduous"), "{e}");
    }

    #[test]
    fn what_the_check_leaves_is_kept_in_the_case_until_the_arch_is_separated_or_painted() {
        let s = with_fragment();
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower" })).unwrap();
        let list = |v: &Value| -> Vec<String> { serde_json::from_value(v.clone()).unwrap() };
        let mut expect = list(&out["lower"]["problems"]);
        expect.extend(list(&out["lower"]["notes"]));
        let d = aligners_data(&next).unwrap();
        let kept = &d.segmentation_doubts[&ScanRole::Lower];
        assert!(!kept.is_empty());
        assert_eq!(*kept, expect);
        // nothing went: the seeds are only checked
        assert_eq!(out["invalidated"], json!([]));
        // they are the warning of the export and of the whole run
        let (by_arch, warnings) = messages::segmentation_doubts(&d);
        assert_eq!(by_arch, json!({ "lower": kept }));
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("lower arch") && warnings[0].contains(&kept[0]),
            "{}",
            warnings[0]
        );
        // a new separation clears them
        let (again, _) = exec(SEGMENT, &next, json!({ "arch": "lower" })).unwrap();
        assert!(
            aligners_data(&again)
                .unwrap()
                .segmentation_doubts
                .is_empty()
        );
        // and so does a colour painted by hand (a point of 46 to the gingiva)
        let tooth = next
            .blobs
            .require(&d.labels[&ScanRole::Lower])
            .unwrap()
            .vertex_labels
            .as_ref()
            .unwrap()
            .iter()
            .position(|&l| l == 46)
            .unwrap();
        let (painted, _) = exec(
            "aligners.preparation.segmentation.manual-brush",
            &next,
            json!({ "arch": "lower", "label": 0, "vertices": [tooth] }),
        )
        .unwrap();
        assert!(
            aligners_data(&painted)
                .unwrap()
                .segmentation_doubts
                .is_empty()
        );
    }

    #[test]
    fn corrected_seeds_drop_the_3d_teeth_of_the_arch_and_say_so() {
        let mut s = with_fragment();
        let mut d = aligners_data(&s).unwrap();
        let crown = s.blobs.insert(dc_mesh::TriMesh::new(
            vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
            vec![[0, 1, 2]],
        ));
        for seed in d.seeds.clone() {
            d.crowns.insert(seed.fdi, crown);
        }
        s.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(CHECK, &s, json!({ "arch": "lower", "correct": true })).unwrap();
        assert_eq!(out["invalidated"], json!(["crowns"]));
        let d = aligners_data(&next).unwrap();
        assert!(d.crowns.is_empty());
        assert!(next.blobs.get(&crown).is_none());
        assert!(!crate::done::leaf_done(&next, "aligners.teeth.build"));
    }
}
