//! Seeds by hand: add, move or delete the seed of a tooth
//! (`aligners.preparation.numbering.place-seeds`).
//!
//! A seed is the centre of the occlusal face of a tooth with its number; the
//! segmentation starts from the seeds, so the labels of the arch (and what
//! was built from them) are out of date after an edit. As in the plugin
//! (`case_core.add_seed` and `steps.mark_manual`), they are kept: the report
//! names them in `invalidated` and the user runs the segmentation again.
//! The position is in case coordinates; the viewport sends the point where
//! the user clicked on the scan.

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::manual::Scan;
use dca_model::Seed;
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
        }),
    ))
}
