//! Numbering by hand, one tooth after the other (`oa.place_seeds` and
//! `oa.clear_seeds` of the plugin).
//!
//! `aligners.preparation.numbering.sequence`: the user (or an agent) gives, in
//! order, a point on each tooth or a "missing" step. The teeth are taken in
//! the arch order, from the patient's right distal to the left distal (upper:
//! 18..11, 21..28; lower: 48..41, 31..38), without the third molars unless
//! asked, and without the teeth that already have a mark or are missing. A
//! point puts the mark of the next tooth; a skip step leaves the next tooth
//! without a mark and goes on to the following one, as the `S` key does in the
//! plugin (it does not add the tooth to the list of missing teeth: that is
//! `aligners.preparation.numbering.mark-missing`). Fewer steps than teeth is
//! fine (the user pressed Enter or Esc): the marks given are kept and the
//! report says which tooth is next. It is one command, so one step on the
//! undo stack. A step may name the tooth it is meant for (`fdi`, as the app
//! does): if the teeth to number changed since the user started, the step is
//! for another tooth and the command refuses the whole numbering.
//!
//! `aligners.preparation.numbering.clear-seeds`: removes the marks of the
//! case so that the numbering starts again; the list of missing teeth stays
//! (as in the plugin) unless `keep_missing` is false.
//!
//! Neither command removes what was computed from the marks (the colours of
//! the teeth and what follows them): as in the plugin it stays, out of date,
//! and the report names it in `invalidated`.

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::{Arch, numbering_sequence};
use dca_algo::manual::Scan;
use dca_model::Seed;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::common::{arch_labels, arch_name, data_of, finish, invalid, pick_arch, point};
use crate::commands::import::require_settled;
use crate::commands::messages;

/// One step of the numbering, for the next tooth of the sequence.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    /// A point on the tooth, in case coordinates (mm).
    Point {
        position: [f64; 3],
        /// The tooth the step is meant for (optional): refused when the
        /// sequence asks for another one.
        #[serde(default)]
        fdi: Option<u8>,
    },
    /// The tooth gets no mark now and the next step is for the following
    /// tooth (it is not added to the missing teeth).
    Skip {
        /// The tooth the step is meant for (optional), as for a point.
        #[serde(default)]
        fdi: Option<u8>,
    },
}

impl Step {
    fn fdi(&self) -> Option<u8> {
        match self {
            Step::Point { fdi, .. } | Step::Skip { fdi } => *fdi,
        }
    }
}

/// Parameters of the numbering in order.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SequenceParams {
    /// `upper` or `lower`; default: the arch of the only scan of the case.
    pub arch: Option<Arch>,
    /// Number the third molars too (default false).
    pub third_molars: bool,
    /// The steps, in order (required, at least one).
    pub steps: Vec<Step>,
    /// Put each mark on the nearest point of the scan (default false).
    pub snap_to_scan: bool,
}

/// Parameters of the clearing of the marks.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ClearParams {
    /// `upper` or `lower`; default: both arches.
    pub arch: Option<Arch>,
    /// Keep the list of missing teeth (default true, as the plugin does;
    /// false clears it too).
    pub keep_missing: bool,
}

impl Default for ClearParams {
    fn default() -> Self {
        Self {
            arch: None,
            keep_missing: true,
        }
    }
}

pub(crate) fn run_sequence(
    state: &CaseState,
    p: SequenceParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if p.steps.is_empty() {
        return Err(messages::invalid("aligners-err-seq-no-steps", &[]));
    }
    let arch = pick_arch(state, p.arch, None)?;
    let scan = state
        .case
        .scan(arch.role())
        .ok_or_else(|| messages::no_scan(arch))?;
    require_settled(scan)?;
    let mut data = data_of(state)?;
    let taken: Vec<Fdi> = data
        .seeds
        .iter()
        .map(|s| s.fdi)
        .chain(data.missing.iter().copied())
        .collect();
    let order = numbering_sequence(arch, p.third_molars, &taken);
    if p.steps.len() > order.len() {
        return Err(messages::invalid(
            "aligners-err-seq-too-many",
            &[
                ("steps", &p.steps.len().to_string()),
                ("teeth", &order.len().to_string()),
                ("arch", &messages::arch_word(arch)),
            ],
        ));
    }
    // validate every step before anything changes
    let mut points = Vec::new();
    for (k, (step, f)) in p.steps.iter().zip(&order).enumerate() {
        if let Some(want) = step.fdi()
            && want != f.get()
        {
            return Err(messages::invalid(
                "aligners-err-seq-wrong-tooth",
                &[
                    ("step", &(k + 1).to_string()),
                    ("fdi", &want.to_string()),
                    ("expected", &f.get().to_string()),
                ],
            ));
        }
        points.push(match step {
            Step::Point { position, .. } => Some(point(*position)?),
            Step::Skip { .. } => None,
        });
    }
    let labels = if p.snap_to_scan {
        Some(arch_labels(state, &data, arch)?)
    } else {
        None
    };
    let vertices = labels.as_ref().map(|al| al.vertices());
    let snap = match (&labels, &vertices) {
        (Some(al), Some(v)) => {
            Some(Scan::new(v, &al.scan.tris).map_err(|e| invalid(e.to_string()))?)
        }
        _ => None,
    };
    let mut assigned = Vec::new();
    let mut arch_seeds: Vec<Seed> = data
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == arch)
        .copied()
        .collect();
    for (&f, at) in order.iter().zip(points) {
        match at {
            Some(mut at) => {
                if let (Some(scan), Some(vertices)) = (&snap, &vertices)
                    && let Some(v) = scan.nearest(at)
                {
                    at = vertices[v as usize];
                }
                arch_seeds.push(Seed {
                    fdi: f,
                    position: at,
                });
                assigned
                    .push(json!({ "fdi": f.get(), "kind": "point", "position": at.to_array() }));
            }
            None => assigned.push(json!({ "fdi": f.get(), "kind": "skip" })),
        }
    }
    // what the tooth marks of the arch decided is kept, out of date
    let invalidated = data.edit_arch_seeds(arch.role(), &arch_seeds);
    let mut next = state.clone();
    finish(&mut next, data, &[]);
    let seeds_in_arch = data_of(&next)?
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == arch)
        .count();
    let remaining: Vec<u8> = order[p.steps.len()..].iter().map(|f| f.get()).collect();
    Ok((
        next,
        json!({
            "arch": arch_name(arch),
            "assigned": assigned,
            "next_fdi": remaining.first(),
            "remaining": remaining,
            "seeds_in_arch": seeds_in_arch,
            "segmentation_outdated": invalidated.contains(&"labels"),
            "invalidated": invalidated,
        }),
    ))
}

pub(crate) fn run_clear(
    state: &CaseState,
    p: ClearParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = data_of(state)?;
    let mine = |f: Fdi| p.arch.is_none_or(|a| Arch::of(f) == a);
    let removed = data.seeds.iter().filter(|s| mine(s.fdi)).count();
    let mut missing_cleared = Vec::new();
    if !p.keep_missing {
        missing_cleared = data.missing.iter().copied().filter(|f| mine(*f)).collect();
        data.missing.retain(|f| !mine(*f));
    }
    let arches: Vec<Arch> = [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter(|a| p.arch.is_none_or(|g| g == *a))
        .collect();
    // what the tooth marks decided is kept, out of date
    let mut invalidated: Vec<&'static str> = Vec::new();
    for a in &arches {
        invalidated.extend(data.edit_arch_seeds(a.role(), &[]));
    }
    // one id per group, in the order of `CLEARED_IDS`
    let invalidated: Vec<&'static str> = dca_model::CLEARED_IDS
        .into_iter()
        .filter(|id| invalidated.contains(id))
        .collect();
    let mut next = state.clone();
    finish(&mut next, data, &[]);
    Ok((
        next,
        json!({
            "seeds_removed": removed,
            "missing_cleared": missing_cleared.iter().map(|f| f.get()).collect::<Vec<_>>(),
            "arches": arches.iter().map(|a| arch_name(*a)).collect::<Vec<_>>(),
            "segmentation_outdated": invalidated.contains(&"labels"),
            "invalidated": invalidated,
        }),
    ))
}
