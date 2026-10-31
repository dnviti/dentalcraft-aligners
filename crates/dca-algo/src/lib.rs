//! Algorithms of the aligners pipeline, one module per stage.
//!
//! Every module is a skeleton for now: it names the file of the old Python
//! plugin (`/home/daniele/Documents/repos/blender-ortho-aligner/ortho_aligner/`)
//! that is the reference and the golden-test oracle for the Rust port. The
//! stages follow the processing order of the old `steps.py`:
//! arch, seeds, segmentation, teeth, diagnosis, bite, setup, staging, then
//! production (shell, trim, stamp), follow-up (visits) and export.

pub mod autoseed;
pub mod bite;
pub mod diagnosis;
pub mod export;
pub mod occlusion;
pub mod reconstruct;
pub mod segmentation;
pub mod setup;
pub mod shell;
pub mod staging;
pub mod stamp;
pub mod trim;
pub mod visits;
