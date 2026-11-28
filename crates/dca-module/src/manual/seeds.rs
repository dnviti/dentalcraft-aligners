//! Seeds by hand: add, move or delete the seed of a tooth
//! (`aligners.preparation.numbering.place-seeds`), or delete it alone
//! (`aligners.preparation.numbering.delete-seed`, the "Delete a tooth" tool).
//!
//! A seed is the centre of the occlusal face of a tooth with its number; the
//! segmentation starts from the seeds, so the labels of the arch (and what
//! was built from them) are out of date after an edit. As in the plugin
//! (`case_core.add_seed` and `steps.mark_manual`), they are kept: the report
//! names them in `invalidated` and the user runs the segmentation again.
//! The position is in case coordinates; the viewport sends the point where
//! the user clicked on the scan. Deleting a seed does not mark the tooth
//! missing (that is `aligners.preparation.numbering.mark-missing`).

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::manual::Scan;
use dca_model::Seed;
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::{arch_labels, arch_name, data_of, fdi_of, finish, invalid, point};
use crate::commands::import::require_settled;
use crate::commands::messages;

/// What to do with the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SeedOp {
    /// Put a new seed (the tooth has none).
    #[default]
    Add,
    /// Move the seed of the tooth.
    Move,
    /// Remove the seed of the tooth.
    Delete,
}

/// Parameters of the seed edit.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SeedParams {
    /// `add` (default), `move` or `delete`.
    pub op: SeedOp,
    /// Tooth number (FDI) (required).
    pub fdi: u8,
    /// Position in case coordinates (mm); required to add or move.
    pub position: Option<[f64; 3]>,
    /// Put the seed on the nearest vertex of the scan (default false).
    pub snap_to_scan: bool,
}

/// Parameters of delete-seed.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DeleteSeedParams {
    /// Number (FDI) of the tooth whose mark is removed (required); its arch
    /// follows from the number.
    pub fdi: u8,
}

/// Removes the seed of one tooth: the seed edit `delete`, so the dependents
/// of the arch are out of date exactly as after any other change of the
/// marks, the list of missing teeth stays as it is, and a tooth without a
/// seed is refused with nothing changed.
pub(crate) fn run_delete_seed(
    state: &CaseState,
    p: DeleteSeedParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    run_seed(
        state,
        SeedParams {
            op: SeedOp::Delete,
            fdi: p.fdi,
            ..SeedParams::default()
        },
        ctx,
    )
}

pub(crate) fn run_seed(
    state: &CaseState,
    p: SeedParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let f = fdi_of(p.fdi)?;
    let arch = Arch::of(f);
    // seeds on a scan whose arch is still a guess would be thrown away with it
    if let Some(scan) = state.case.scan(arch.role()) {
        require_settled(scan)?;
    }
    let mut data = data_of(state)?;
    let before = data.seeds.clone();
    let had = data.seeds.iter().position(|s| s.fdi == f);
    let mut position = None;
    match p.op {
        SeedOp::Delete => {
            let Some(i) = had else {
                return Err(messages::invalid(
                    "aligners-err-tooth-no-mark",
                    &[("fdi", &f.to_string())],
                ));
            };
            data.seeds.remove(i);
        }
        SeedOp::Add | SeedOp::Move => {
            let mut at = point(
                p.position
                    .ok_or_else(|| invalid("position is required to add or move a tooth mark"))?,
            )?;
            match (p.op, had) {
                (SeedOp::Add, Some(_)) => {
                    return Err(messages::invalid(
                        "aligners-err-tooth-has-mark",
                        &[("fdi", &f.to_string())],
                    ));
                }
                (SeedOp::Move, None) => {
                    return Err(messages::invalid(
                        "aligners-err-tooth-no-mark",
                        &[("fdi", &f.to_string())],
                    ));
                }
                _ => {}
            }
            if data.missing.contains(&f) {
                return Err(invalid(format!(
                    "tooth {f} is marked missing: mark it present first"
                )));
            }
            if p.snap_to_scan {
                let al = arch_labels(state, &data, arch)?;
                let vertices = al.vertices();
                let scan =
                    Scan::new(&vertices, &al.scan.tris).map_err(|e| invalid(e.to_string()))?;
                if let Some(v) = scan.nearest(at) {
                    at = vertices[v as usize];
                }
            }
            match had {
                Some(i) => data.seeds[i].position = at,
                None => data.seeds.push(Seed {
                    fdi: f,
                    position: at,
                }),
            }
            position = Some(at);
        }
    }
    let arch_seeds = data
        .seeds
        .iter()
        .filter(|s| Arch::of(s.fdi) == arch)
        .count();
    // what the tooth marks of the arch decided is kept, out of date
    let edited: Vec<Seed> = std::mem::replace(&mut data.seeds, before)
        .into_iter()
        .filter(|s| Arch::of(s.fdi) == arch)
        .collect();
    let invalidated = data.edit_arch_seeds(arch.role(), &edited);
    // a mark closer than about half a tooth to another: the separation's
    // later disk takes the faces of the earlier one, whose tooth then merges
    // into its neighbour (warned, not refused: the user may be moving it)
    let warnings = position.map_or_else(Vec::new, |at| close_marks(&edited, f, at));
    let mut next = state.clone();
    finish(&mut next, data, &[]);
    Ok((
        next,
        json!({
            "op": p.op,
            "fdi": f.get(),
            "arch": arch_name(arch),
            "position": position.map(|v| v.to_array()),
            "seeds_in_arch": arch_seeds,
            "segmentation_outdated": invalidated.contains(&"labels"),
            "invalidated": invalidated,
            "warnings": warnings,
        }),
    ))
}

/// Marks closer than this times the mean typical mesio-distal width of the
/// two teeth are warned about ([`close_marks`]).
const CLOSE_MARKS: f64 = 0.5;

/// The warnings for the marks of `seeds` (one arch) closer to the mark of
/// `f` at `at` than [`CLOSE_MARKS`] times the typical width of the teeth.
fn close_marks(seeds: &[Seed], f: Fdi, at: DVec3) -> Vec<String> {
    let md = |x: Fdi| dca_algo::anatomy::typical_diameters(x).0;
    seeds
        .iter()
        .filter(|s| s.fdi != f)
        .filter_map(|s| {
            let d = s.position.distance(at);
            (d < CLOSE_MARKS * 0.5 * (md(f) + md(s.fdi))).then(|| {
                messages::tr(
                    "aligners-warn-seeds-close",
                    &[
                        ("a", &f.to_string()),
                        ("b", &s.fdi.to_string()),
                        ("mm", &format!("{d:.1}")),
                    ],
                )
            })
        })
        .collect()
}
