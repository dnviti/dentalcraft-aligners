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

/// Id of the check.
pub(crate) const CHECK: &str = "aligners.preparation.segmentation.check";
const SEGMENT: &str = "aligners.preparation.segmentation.automatic";

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
    let mesh = state.blobs.require(&scan.mesh)?;
    let labelled = state.blobs.require(labels_ref)?;
    let labels = labelled
        .vertex_labels
        .clone()
        .ok_or_else(|| messages::labels_missing(arch))?;
    if labels.len() != mesh.vertex_count() {
        return Err(messages::labels_mismatch(
            arch,
            labels.len(),
            mesh.vertex_count(),
        ));
    }
    Ok(Some(Scan {
        vertices: mesh.positions.iter().map(|p| p.as_dvec3()).collect(),
        tris: mesh.tris.clone(),
        labels,
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

fn put_seeds(
    state: &mut CaseState,
    arch: Arch,
    seeds: &[(Fdi, DVec3)],
) -> Result<(), CommandError> {
    let mut data = aligners_data(state)?;
    data.seeds.retain(|s| Arch::of(s.fdi) != arch);
    data.seeds
        .extend(seeds.iter().map(|&(fdi, position)| Seed { fdi, position }));
    state.set_module(MODULE_ID, ModuleState::new(data));
    Ok(())
}

/// The arch segmented again from its seeds.
fn segment(state: &mut CaseState, arch: Arch, k: usize, ctx: &JobCtx) -> Result<(), CommandError> {
    let step = Step::new(SEGMENT, json!({ "arch": arch_name(arch) }));
    *state = run_step(state, &step, k, ctx)?.0;
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

fn check(scan: &Scan, arch: Arch, data: &autoseed::ArchData) -> Report {
    seed_check::check_teeth(&scan.vertices, &scan.tris, &scan.labels, arch, data)
}

/// The check (and the corrections) of one arch.
fn check_arch(
    state: &mut CaseState,
    arch: Arch,
    params: &CheckParams,
    ctx: &JobCtx,
) -> Result<Option<Value>, CommandError> {
    let Some(scan) = scan_of(state, arch)? else {
        return Ok(None);
    };
    // the arch curve of the scan (as the numbering computes it): the same
    // scan for every round, only the labels change
    let Ok(data) = autoseed::arch_data(&scan.vertices, &scan.tris) else {
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
            put_seeds(state, arch, &new)?;
            done.extend(acts.iter().map(describe_action));
            actions.extend(acts);
            segment(state, arch, k, ctx)?;
            k += 1;
            rounds += 1;
            rep = check(&scan_of(state, arch)?.expect("segmented"), arch, &data);
        }
        if !done.is_empty() && rep.doubtful() {
            // teeth still implausible: their widths do not count the teeth,
            // the numbers of the proposal are kept, to be checked
            notes.push(tr("aligners-check-note-not-renumbered", &[]));
        } else if !done.is_empty() {
            let missing: Vec<u16> = params.missing.iter().map(|&m| u16::from(m)).collect();
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
                put_seeds(state, arch, &renamed)?;
                segment(state, arch, k, ctx)?;
                rep = check(&scan_of(state, arch)?.expect("segmented"), arch, &data);
            }
        }
    }
    let issues = seed_check::report_issues(&rep);
    Ok(Some(json!({
        "rounds": rounds,
        "actions": actions,
        "renumbered": renumbered,
        "corrections": done,
        "issues": issues,
        "problems": issues.iter().map(describe_issue).collect::<Vec<_>>(),
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
    let mut next = state.clone();
    let mut out = Map::new();
    let mut skipped = Vec::new();
    for arch in params.arch.arches() {
        match check_arch(&mut next, arch, &params, ctx)? {
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
    out.insert("skipped".to_owned(), json!(skipped));
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
}
