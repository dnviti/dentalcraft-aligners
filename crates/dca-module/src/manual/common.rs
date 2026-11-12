//! What the manual commands share: reading and writing the Aligners data
//! and the labels of an arch, the arch frame, forgetting what depends on a
//! tooth.

use std::sync::Arc;

use dc_case::{BlobRef, CaseState, Fdi, ModuleState, Pose};
use dc_command::CommandError;
use dc_mesh::TriMesh;
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_model::{AlignersState, MODULE_ID};
use glam::{DMat4, DVec3};
use serde_json::{Value, json};

use crate::commands::messages;

pub(crate) fn invalid(msg: impl Into<String>) -> CommandError {
    CommandError::Invalid(msg.into())
}

pub(crate) fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The Aligners data of the case (the defaults when there is none).
pub(crate) fn data_of(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(|| invalid("the Aligners data cannot be read")),
    }
}

/// A tooth number from a command parameter.
pub(crate) fn fdi_of(n: u8) -> Result<Fdi, CommandError> {
    Fdi::new(n).map_err(|_| invalid(format!("{n} is not a valid FDI tooth number")))
}

/// A tooth number from a command parameter, `0` for the gingiva.
pub(crate) fn label_of(n: u8) -> Result<u16, CommandError> {
    if n == 0 {
        Ok(0)
    } else {
        fdi_of(n).map(|f| u16::from(f.get()))
    }
}

/// A seed position or a point from three numbers.
pub(crate) fn point(p: [f64; 3]) -> Result<DVec3, CommandError> {
    let v = DVec3::from_array(p);
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid("a coordinate is not a finite number"))
    }
}

/// The arch a command works on: the one given, else the arch of `fdi`, else
/// the only arch that has a scan.
pub(crate) fn pick_arch(
    state: &CaseState,
    given: Option<Arch>,
    fdi: Option<Fdi>,
) -> Result<Arch, CommandError> {
    if let Some(a) = given {
        if let Some(f) = fdi
            && Arch::of(f) != a
        {
            return Err(invalid(format!(
                "tooth {f} is not in the {} arch",
                arch_name(a)
            )));
        }
        return Ok(a);
    }
    if let Some(f) = fdi {
        return Ok(Arch::of(f));
    }
    let have: Vec<Arch> = [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter(|a| state.case.scan(a.role()).is_some())
        .collect();
    match have.as_slice() {
        [one] => Ok(*one),
        [] => Err(invalid("the case has no scan")),
        _ => Err(invalid(
            "the case has both arches: give the arch (upper or lower)",
        )),
    }
}

/// The scan of an arch and its labels (all gingiva when the arch is not
/// segmented yet).
pub(crate) struct ArchLabels {
    pub scan: Arc<TriMesh>,
    pub labels: Vec<u16>,
    /// `true` when the arch already had a labels blob.
    pub had_labels: bool,
}

pub(crate) fn arch_labels(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<ArchLabels, CommandError> {
    let role = arch.role();
    let scan = state
        .case
        .scan(role)
        .ok_or_else(|| invalid(format!("the case has no {} scan", arch_name(arch))))?;
    let mesh = Arc::clone(state.blobs.require(&scan.mesh)?);
    match data.labels.get(&role) {
        Some(r) => {
            let labelled = state.blobs.require(r)?;
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
            Ok(ArchLabels {
                scan: mesh,
                labels,
                had_labels: true,
            })
        }
        None => {
            let n = mesh.vertex_count();
            Ok(ArchLabels {
                scan: mesh,
                labels: vec![0; n],
                had_labels: false,
            })
        }
    }
}

impl ArchLabels {
    pub(crate) fn vertices(&self) -> Vec<DVec3> {
        self.scan.positions.iter().map(|p| p.as_dvec3()).collect()
    }
}

/// Stores `labels` as the labels blob of the arch in `next`; the replaced
/// blob goes to `stale`. The doubts of the check of the separated teeth and
/// the result of the last separation on the arch go: the colours were
/// corrected by hand (the plugin's `segmentation_doubts` are none once the
/// step was done by hand).
pub(crate) fn store_labels(
    next: &mut CaseState,
    data: &mut AlignersState,
    stale: &mut Vec<BlobRef>,
    arch: Arch,
    scan: &TriMesh,
    labels: Vec<u16>,
) -> BlobRef {
    let mut labelled = scan.clone();
    labelled.vertex_labels = Some(labels);
    labelled.face_labels = None;
    let r = next.blobs.insert(labelled);
    data.forget_separation(arch.role());
    if let Some(old) = data.labels.insert(arch.role(), r)
        && old != r
    {
        stale.push(old);
    }
    r
}

/// Writes the Aligners data back and drops the blobs in `stale` that nothing
/// uses any more.
pub(crate) fn finish(next: &mut CaseState, data: AlignersState, stale: &[BlobRef]) {
    next.set_module(MODULE_ID, ModuleState::new(data));
    finish_blobs(next, stale);
}

/// Drops the blobs in `stale` that nothing in `next` uses any more.
pub(crate) fn finish_blobs(next: &mut CaseState, stale: &[BlobRef]) {
    if stale.is_empty() {
        return;
    }
    let used = next.referenced_blobs();
    let keep = next
        .blobs
        .iter()
        .map(|(r, _)| *r)
        .filter(|r| used.contains(r) || !stale.contains(r))
        .collect();
    next.blobs.retain_only(&keep);
}

/// The arch frame (arch to case coordinates, `+Z` occlusal) the seeds of the
/// arch define, as the other stages use it.
pub(crate) fn arch_to_case(
    data: &AlignersState,
    arch: Arch,
    case_vertices: &[DVec3],
) -> Result<DMat4, CommandError> {
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: case_vertices,
            seeds: &data.seeds,
        },
        &frame::Params::default(),
    )
    .map_err(|e| CommandError::Failed(format!("{} arch: {e}", arch_name(arch))))?;
    Ok(framed.arch_frame.matrix())
}

/// What was dropped by [`forget_tooth`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Forgotten {
    pub frame: bool,
    pub solid: bool,
    pub movement: bool,
}

impl Forgotten {
    pub(crate) fn json(self) -> Value {
        json!({ "frame": self.frame, "solid": self.solid, "movement": self.movement })
    }

    pub(crate) fn merge(self, o: Forgotten) -> Forgotten {
        Forgotten {
            frame: self.frame || o.frame,
            solid: self.solid || o.solid,
            movement: self.movement || o.movement,
        }
    }
}

/// Drops everything the later stages computed for tooth `f`: its frame and
/// measures, its closed solid (the blob goes to `stale`), its setup pose, its
/// lock, its poses in the stages and the IPR on its contacts.
pub(crate) fn forget_tooth(
    data: &mut AlignersState,
    stale: &mut Vec<BlobRef>,
    f: Fdi,
) -> Forgotten {
    let mut out = Forgotten {
        frame: data.tooth_frames.remove(&f).is_some(),
        ..Forgotten::default()
    };
    out.frame |= data.tooth_geometry.remove(&f).is_some();
    if let Some(r) = data.crowns.remove(&f) {
        stale.push(r);
        out.solid = true;
    }
    out.movement = data.setup.poses.remove(&f).is_some();
    out.movement |= data.setup.locked.remove(&f);
    let n = data.setup.ipr.len();
    data.setup.ipr.retain(|s| !s.between.contains(&f));
    out.movement |= data.setup.ipr.len() != n;
    for s in &mut data.stages {
        out.movement |= s.poses.remove(&f).is_some();
        let n = s.ipr.len();
        s.ipr.retain(|i| !i.between.contains(&f));
        out.movement |= s.ipr.len() != n;
    }
    data.dirty.remove(&f);
    out
}

/// The local pose of tooth `f` in the setup, or at `stage` (identity when it
/// does not move).
pub(crate) fn pose_in(data: &AlignersState, stage: Option<u32>, f: Fdi) -> Pose {
    match stage {
        None => data.setup.poses.get(&f).copied(),
        Some(k) => data
            .stages
            .iter()
            .find(|s| s.index == k)
            .and_then(|s| s.poses.get(&f).copied()),
    }
    .unwrap_or(Pose::IDENTITY)
}

/// Runs another registered command on `state` (the stage commands are the
/// way to measure and rebuild teeth, so the manual tools chain them).
pub(crate) fn run_command(
    id: &str,
    state: &CaseState,
    params: Value,
    ctx: &dc_command::JobCtx,
) -> Result<(CaseState, Value), CommandError> {
    let spec = crate::commands::find(id)
        .ok_or_else(|| CommandError::Failed(format!("the command {id} is not available")))?;
    spec.execute(state, &params, ctx)
}
