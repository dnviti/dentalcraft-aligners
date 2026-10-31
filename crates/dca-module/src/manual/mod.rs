//! The manual tools of the Aligners module: every automatic step can be
//! done or corrected by hand (`manual.py` and `ops_manual.py` of the
//! plugin).
//!
//! Each tool is a registered command with JSON parameters, so that an agent
//! drives it from `dentalcraft-cli` or the MCP server exactly as the
//! interface does: the viewport turns a click or a drag into the parameters
//! of the same command and runs it through the workspace, so it is one step
//! on the undo stack.
//!
//! | Tool | Command |
//! | --- | --- |
//! | label brush (paint, erase, fill) | `aligners.preparation.segmentation.manual-brush` |
//! | clean the labels | `aligners.preparation.segmentation.clean-borders` |
//! | renumber or swap a tooth | `aligners.preparation.numbering.renumber` |
//! | mark a tooth missing or present | `aligners.preparation.numbering.mark-missing` |
//! | seed edit (add, move, delete) | `aligners.preparation.numbering.place-seeds` |
//! | rebuild one tooth | `aligners.manual.rebuild-tooth` |
//! | adjust a tooth (setup or stage) | `aligners.planning.movements` |
//!
//! The commands are registered in `commands/manual.rs`. The geometry (brush, crown fill, region check, clean-up, collision
//! metric) is in `dca_algo::manual`.

pub(crate) mod adjust;
pub(crate) mod brush;
pub(crate) mod common;
pub(crate) mod numbering;
pub(crate) mod rebuild;
pub(crate) mod seeds;

#[cfg(test)]
mod tests;
