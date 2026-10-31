//! Algorithms of the aligners pipeline, one module per stage.
//!
//! Every module is a skeleton for now: it names the file of the old Python
//! plugin (`/home/daniele/Documents/repos/blender-ortho-aligner/ortho_aligner/`)
//! that is the reference and the golden-test oracle for the Rust port. The
//! stages follow the processing order of the old `steps.py`:
//! arch, seeds, segmentation, teeth, diagnosis, bite, setup, staging, then
//! production (shell, trim, stamp), follow-up (visits) and export. Shared
//! building blocks: world frame (`frame`), arch forms (`archform`), tooth
//! anatomy (`anatomy`), reference values (`evidence`) and the synthetic
//! demo arch (`demo`).
//!
//! Port convention: each stage exposes `run(&Input, &Params)` with
//! `Params: Deserialize + JsonSchema + Default`, and `dca-module` registers
//! it as a command with `stage_command!`.

pub mod aligner_bite;
pub mod anatomy;
pub mod archform;
pub mod autoseed;
pub mod bite;
pub mod demo;
pub mod diagnosis;
pub mod evidence;
pub mod export;
pub mod frame;
pub mod gingiva;
pub mod manual;
pub mod occlusion;
pub mod ramps;
pub mod reconstruct;
pub mod schedule;
pub mod segmentation;
pub mod setup;
pub mod shell;
pub mod staging;
pub mod stamp;
pub mod teeth;
pub mod trim;
pub mod visits;
