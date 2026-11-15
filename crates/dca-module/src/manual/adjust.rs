//! Adjust one tooth by hand, in the setup or at a stage, with collision
//! feedback (`aligners.planning.movements`, `case_plan.adjust_tooth` of the
//! plugin).
//!
//! The movement is an increment in the frame of the tooth (origin at its
//! centre of resistance): clinical components (mesial, buccal, extrusion in
//! mm; torque, tip, rotation in degrees) and/or a pose `delta`, composed
//! after the pose the tooth has now. Poses are stored as the other stages
//! store them (the local pose in the rest frame of the tooth), in
//! `Setup::poses` or in the poses of the stage `stage`. After the move, the
//! crowns of the tooth and of its neighbours are checked for interpenetration
//! with the metric of the staging; the report gives the penetration with
//! each neighbour before and after, and `refuse_on_collision` turns a
//! collision into an error (nothing changes).
//!
//! A move of the setup leaves the staging out of date (the report says so,
//! the stages are not touched); `lock: true` keeps the tooth where it was put
//! when the setup is computed again, `lock: false` frees it. With `exact` the
//! clinical components are the whole movement from rest (the values the
//! inspector shows), not an increment.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use dc_case::{CaseState, Fdi, Pose};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::{self, Arch};
use dca_algo::frame;
use dca_algo::manual::{self, ClinicalDelta};
use dca_algo::rotation;
use dca_algo::staging;
use dca_algo::teeth;
use dca_model::AlignersState;
use glam::{DQuat, DVec3};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::common::{arch_labels, arch_to_case, data_of, fdi_of, finish, invalid, pose_in};
use crate::commands::messages;

/// Points per crown used by the collision check.
const SAMPLES: usize = 1500;

/// A pose in the rest frame of the tooth: translation in mm and rotation as
/// a quaternion `[x, y, z, w]`.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PoseDelta {
    /// Translation (mm).
    pub t: [f64; 3],
    /// Rotation, unit quaternion `[x, y, z, w]`.
    pub q: [f64; 4],
}

impl Default for PoseDelta {
    fn default() -> Self {
        Self {
            t: [0.0; 3],
            q: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl PoseDelta {
    pub(crate) fn pose(&self) -> Result<Pose, CommandError> {
        let t = DVec3::from_array(self.t);
        let q = DQuat::from_array(self.q);
        if !t.is_finite() || !q.is_finite() || q.length() < 1e-9 {
            return Err(invalid("delta is not a valid pose"));
        }
        Ok(Pose::new(t, q.normalize()))
    }
}

/// Parameters of the manual adjustment of a tooth.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AdjustParams {
    /// Tooth number (FDI) (required).
    pub fdi: u8,
    /// Index of the stage to adjust; absent: the final position (setup).
    pub stage: Option<u32>,
    /// Mesial (+) or distal (-) translation (mm).
    pub mesial_mm: f64,
    /// Buccal (+) or lingual (-) translation (mm).
    pub buccal_mm: f64,
    /// Extrusion (+) or intrusion (-) (mm).
    pub extrusion_mm: f64,
    /// Crown torque, buccal (+) (degrees).
    pub torque_deg: f64,
    /// Crown tip, mesial (+) (degrees).
    pub tip_deg: f64,
    /// Rotation about the long axis, mesio-buccal (+) (degrees).
    pub rotation_deg: f64,
    /// A pose in the rest frame of the tooth, composed after its current
    /// pose (or replacing it with `absolute`), before the clinical
    /// components.
    pub delta: Option<PoseDelta>,
    /// `delta` is the new pose of the tooth, not an increment.
    pub absolute: bool,
    /// The clinical components are the whole movement of the tooth from its
    /// rest position, as the inspector and the reports give it (mm and
    /// degrees, `staging::describe_q`), not an increment: the plugin's
    /// "Write the exact values of the tooth". Not with `delta` or `reset`.
    pub exact: bool,
    /// Bring the tooth back to its rest position first.
    pub reset: bool,
    /// Lock (`true`) or unlock (`false`) the tooth in the setup: computing
    /// the setup again keeps a locked tooth where it is put (setup only).
    /// Absent: the lock stays as it is.
    pub lock: Option<bool>,
    /// Refuse the move when the tooth ends up colliding with a neighbour.
    pub refuse_on_collision: bool,
}

impl AdjustParams {
    fn clinical(&self) -> ClinicalDelta {
        ClinicalDelta {
            mesial_mm: self.mesial_mm,
            buccal_mm: self.buccal_mm,
            extrusion_mm: self.extrusion_mm,
            torque_deg: self.torque_deg,
            tip_deg: self.tip_deg,
            rotation_deg: self.rotation_deg,
        }
    }

    /// The pose whose clinical description is the components given (the
    /// inverse of `staging::describe_q`).
    fn exact_pose(&self, md_sign: i8) -> Result<Pose, CommandError> {
        let s = f64::from(md_sign);
        let q = [
            self.mesial_mm * s,
            self.buccal_mm,
            self.extrusion_mm,
            -self.torque_deg.to_radians(),
            self.tip_deg.to_radians() * s,
            self.rotation_deg.to_radians() * s,
        ];
        let turn = DVec3::new(q[3], q[4], q[5]).length();
        if q.iter().any(|v| !v.is_finite()) || turn >= std::f64::consts::PI {
            return Err(invalid(
                "the exact values must be finite, the rotations below 180 degrees in all",
            ));
        }
        Ok(staging::pose_from_q(&q))
    }
}

fn pose_json(p: &Pose, md_sign: i8) -> Value {
    let q = staging::q_from_pose(p);
    let c = staging::describe_q(&q, md_sign);
    json!({
        "translation_mm": p.t.to_array(),
        "rotation_deg": rotation::degrees(p),
        "pose": p,
        "clinical": {
            "mesial_mm": c[0], "buccal_mm": c[1], "extrusion_mm": c[2],
            "torque_deg": c[3], "tip_deg": c[4], "rotation_deg": c[5],
        },
    })
}

/// One neighbour of the adjusted tooth.
struct Contact {
    with: Fdi,
    penetration_mm: f64,
    tolerance_mm: f64,
}

impl Contact {
    fn collides(&self) -> bool {
        self.penetration_mm > self.tolerance_mm
    }

    fn json(&self) -> Value {
        json!({
            "with": self.with.get(),
            "penetration_mm": (self.penetration_mm * 1000.0).round() / 1000.0,
            "tolerance_mm": self.tolerance_mm,
            "collides": self.collides(),
        })
    }
}

/// IPR planned on the contact of two teeth, in the setup or up to `stage`.
fn ipr_between(data: &AlignersState, stage: Option<u32>, a: Fdi, b: Fdi) -> f64 {
    let hit = |s: &dca_model::IprStep| s.between.contains(&a) && s.between.contains(&b);
    match stage {
        None => data
            .setup
            .ipr
            .iter()
            .filter(|s| hit(s))
            .map(|s| s.amount_mm)
            .sum(),
        Some(k) => data
            .stages
            .iter()
            .filter(|s| s.index <= k)
            .flat_map(|s| s.ipr.iter())
            .filter(|s| hit(s))
            .map(|s| s.amount_mm)
            .sum(),
    }
}

/// Penetration of the adjusted tooth `f`, with its pose `pose_f`, into the
/// neighbours along the arch (everything else as the setup or the stage
/// has it).
fn contacts(
    state: &CaseState,
    data: &AlignersState,
    stage: Option<u32>,
    f: Fdi,
    pose_f: &Pose,
) -> Result<Vec<Contact>, CommandError> {
    let arch = Arch::of(f);
    let fdis: Vec<Fdi> = data
        .tooth_frames
        .keys()
        .copied()
        .filter(|g| Arch::of(*g) == arch)
        .collect();
    let seq = anatomy::arch_sequence(&fdis, arch);
    let neighbours: Vec<Fdi> = staging::adjacent_pairs(&seq)
        .into_iter()
        .filter_map(|(a, b)| {
            if a == f {
                Some(b)
            } else if b == f {
                Some(a)
            } else {
                None
            }
        })
        .collect();
    if neighbours.is_empty() {
        return Ok(Vec::new());
    }
    let al = arch_labels(state, data, arch)?;
    let case_vertices = al.vertices();
    let to_arch = frame::rigid_inverse(&arch_to_case(data, arch, &case_vertices)?);

    // the crown of a tooth in the arch coordinates, posed
    let mut cache: BTreeMap<Fdi, Vec<DVec3>> = BTreeMap::new();
    let mut crown = |t: Fdi, pose: &Pose| -> Result<Vec<DVec3>, CommandError> {
        if let Entry::Vacant(slot) = cache.entry(t) {
            let pts: Vec<DVec3> = match data.crowns.get(&t) {
                Some(r) => state
                    .blobs
                    .require(r)?
                    .positions
                    .iter()
                    .map(|p| p.as_dvec3())
                    .collect(),
                None => case_vertices
                    .iter()
                    .zip(&al.labels)
                    .filter(|(_, l)| **l == u16::from(t.get()))
                    .map(|(p, _)| *p)
                    .collect(),
            };
            slot.insert(teeth::subsample(&pts, SAMPLES));
        }
        let rest = teeth::rest_pose(&data.tooth_frames[&t]);
        let moved = rest.compose(pose).compose(&rest.inverse());
        Ok(cache[&t]
            .iter()
            .map(|&p| to_arch.transform_point3(moved.apply_point(p)))
            .collect())
    };
    let tol = data.settings.staging.collision_tol_mm;
    let pa = crown(f, pose_f)?;
    let mut out = Vec::new();
    for t in neighbours {
        let pb = crown(t, &pose_in(data, stage, t))?;
        out.push(Contact {
            with: t,
            penetration_mm: manual::penetration(&pa, &pb),
            tolerance_mm: tol + ipr_between(data, stage, f, t),
        });
    }
    Ok(out)
}

fn contacts_json(c: &Result<Vec<Contact>, CommandError>) -> Value {
    match c {
        Ok(v) => Value::Array(v.iter().map(Contact::json).collect()),
        Err(e) => json!({ "unavailable": e.to_string() }),
    }
}

pub(crate) fn run_adjust(
    state: &CaseState,
    p: AdjustParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let f = fdi_of(p.fdi)?;
    let mut data = data_of(state)?;
    if data.missing.contains(&f) {
        return Err(invalid(format!("tooth {f} is marked missing")));
    }
    if !data.tooth_frames.contains_key(&f) {
        return Err(messages::invalid(
            "aligners-err-tooth-not-measured",
            &[("fdi", &f.to_string())],
        ));
    }
    if p.lock.is_some() && p.stage.is_some() {
        return Err(invalid("lock only applies to the setup"));
    }
    if p.exact && (p.delta.is_some() || p.reset) {
        return Err(invalid(
            "exact gives the whole movement: it does not go with delta or reset",
        ));
    }
    if let Some(k) = p.stage
        && !data.stages.iter().any(|s| s.index == k)
    {
        return Err(invalid(format!("the case has no stage {k}")));
    }
    let clinical = p.clinical();
    let delta = p.delta.as_ref().map(PoseDelta::pose).transpose()?;
    if clinical.is_zero() && delta.is_none() && !p.reset && !p.exact && p.lock.is_none() {
        return Err(invalid(
            "no movement given: set a clinical component, a delta, exact values, reset or lock",
        ));
    }
    let md_sign = data.tooth_geometry.get(&f).map_or(1, |g| g.md_sign);
    let before = pose_in(&data, p.stage, f);
    let moves = p.exact || p.reset || delta.is_some() || !clinical.is_zero();
    let pose = if !moves {
        // a lock alone: the pose stays exactly as it is
        before
    } else if p.exact {
        rotation::canonical_pose(&p.exact_pose(md_sign)?)
    } else {
        let mut pose = if p.reset { Pose::IDENTITY } else { before };
        if let Some(d) = delta {
            pose = if p.absolute { d } else { pose.compose(&d) };
        }
        // stored as the staging stores it: the shortest rotation, w >= 0
        rotation::canonical_pose(&pose.compose(&clinical.pose(md_sign)))
    };

    let contacts_before = contacts(state, &data, p.stage, f, &before);
    let contacts_after = contacts(state, &data, p.stage, f, &pose);
    if p.refuse_on_collision
        && let Ok(list) = &contacts_after
        && let Some(c) = list.iter().find(|c| c.collides())
    {
        return Err(invalid(format!(
            "tooth {f} would collide with tooth {} (penetration {:.2} mm, tolerance {:.2} mm)",
            c.with, c.penetration_mm, c.tolerance_mm
        )));
    }

    match p.stage {
        None => {
            if pose.approx_eq(&Pose::IDENTITY, 1e-12, 1e-12) {
                data.setup.poses.remove(&f);
            } else {
                data.setup.poses.insert(f, pose);
            }
            match p.lock {
                Some(true) => {
                    data.setup.locked.insert(f);
                }
                Some(false) => {
                    data.setup.locked.remove(&f);
                }
                None => {}
            }
        }
        Some(k) => {
            if let Some(s) = data.stages.iter_mut().find(|s| s.index == k) {
                s.poses.insert(f, pose);
            }
        }
    }
    let moved = !pose.approx_eq(&before, 1e-12, 1e-12);
    let staging_outdated = p.stage.is_none() && moved && !data.stages.is_empty();
    let data_locked = data.setup.locked.contains(&f);
    let collides = contacts_after
        .as_ref()
        .is_ok_and(|l| l.iter().any(Contact::collides));
    let mut next = state.clone();
    finish(&mut next, data, &[]);
    Ok((
        next,
        json!({
            "fdi": f.get(),
            "stage": p.stage,
            "before": pose_json(&before, md_sign),
            "after": pose_json(&pose, md_sign),
            "locked": p.stage.is_none() && data_locked,
            "collisions_before": contacts_json(&contacts_before),
            "collisions": contacts_json(&contacts_after),
            "collides": collides,
            "staging_outdated": staging_outdated,
        }),
    ))
}
