//! Commands of the `ramps` stage (`dca_algo::ramps`) and of the bite with the
//! aligners worn (`dca_algo::aligner_bite`), which the ramps are part of.
//!
//! * `aligners.occlusion.bite-with-aligners` closes the mouth on the two
//!   aligners of a stage (the ramps of the case included when they are
//!   enabled): the occlusal thickness (manual or automatic), the opening of
//!   the bite caused by the aligners, the teeth in contact on each side and,
//!   with `equilibrate`, the virtual grinding until both sides touch. The
//!   settings it used are kept in the case (`save`, on by default) for the
//!   shell. Needs both arches segmented with their seeds.
//! * `aligners.occlusion.bite-ramps` proposes the anterior ramps of a deep
//!   bite (`propose`), places one by hand (`add`), takes them off (`remove`)
//!   or changes the settings (`set`: enabled, sizes, overbite threshold).
//!
//! The case is modified only in its bite settings and ramps
//! (`AlignersState::aligner_bite`); the shell adds the ramps with
//! [`dca_algo::ramps::apply_to_grid`].

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{CaseState, Fdi, ModuleState, Pose};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::aligner_bite::occlusal::{self, ArchStage, Design};
use dca_algo::anatomy::Arch;
use dca_algo::occlusion::{self, ArchModel};
use dca_algo::ramps::{self, ProposeInput, ProposeTooth, RampSolid};
use dca_algo::{frame, staging};
use dca_model::{
    AlignersState, BiteClosing, MODULE_ID, OcclusalMode, OcclusalSettings, Ramp, RampKind,
};
use glam::{DMat4, DVec3};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::occlusion::{aligners_data, arch_model};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.occlusion.bite-with-aligners",
            label: "aligners-cmd-bite-with-aligners",
            doc: "aligners-cmd-bite-with-aligners-doc",
            params: BiteParams,
            run: run_bite,
        },
        stage_command! {
            id: "aligners.occlusion.bite-ramps",
            label: "aligners-cmd-bite-ramps",
            doc: "aligners-cmd-bite-ramps-doc",
            params: RampParams,
            run: run_ramps,
        },
    ]
}

/// How the occlusal thickness is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThicknessMode {
    /// From the bite with the aligners worn.
    Auto,
    /// `thickness_mm`.
    Manual,
    /// The wall thickness everywhere.
    Same,
}

/// How the mouth closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Closing {
    /// Rotation about the hinge axis of the condyles.
    Hinge,
    /// Translation along the occlusal axis.
    Vertical,
}

/// Parameters of the bite with the aligners worn; what is missing is the
/// setting of the case (`aligner_bite.occlusal`).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BiteParams {
    /// Aligner number (0 the initial situation); the arch with fewer
    /// aligners stays at its last.
    pub stage: u32,
    /// How the occlusal thickness is chosen.
    pub mode: Option<ThicknessMode>,
    /// Occlusal thickness of the manual mode (mm).
    pub thickness_mm: Option<f64>,
    /// Smallest occlusal thickness (mm).
    pub min_thickness_mm: Option<f64>,
    /// Opening at the first molars the automatic thickness does not exceed
    /// (mm).
    pub opening_target_mm: Option<f64>,
    /// How the mouth closes.
    pub closing: Option<Closing>,
    /// Distance of the hinge axis behind the lower incisal point (mm).
    pub hinge_posterior_mm: Option<f64>,
    /// Height of the hinge axis above the lower incisal point (mm).
    pub hinge_height_mm: Option<f64>,
    /// Virtual grinding until both sides touch.
    pub equilibrate: Option<bool>,
    /// Teeth in contact per side at which the equilibration stops.
    pub equilibrate_teeth_per_side: Option<u32>,
    /// Largest grinding of each aligner (mm).
    pub equilibrate_max_mm: Option<f64>,
    /// Keep the settings used in the case (default true).
    pub save: Option<bool>,
}

impl BiteParams {
    fn settings(&self, base: &OcclusalSettings) -> Result<OcclusalSettings, CommandError> {
        let mut o = *base;
        if let Some(m) = self.mode {
            o.mode = match m {
                ThicknessMode::Auto => OcclusalMode::Auto,
                ThicknessMode::Manual => OcclusalMode::Manual,
                ThicknessMode::Same => OcclusalMode::Same,
            };
        }
        if let Some(c) = self.closing {
            o.closing = match c {
                Closing::Hinge => BiteClosing::Hinge,
                Closing::Vertical => BiteClosing::Vertical,
            };
        }
        let set = |dst: &mut f64, v: Option<f64>, name: &str, lo: f64, hi: f64| {
            if let Some(v) = v {
                if !(lo..=hi).contains(&v) {
                    return Err(CommandError::Invalid(format!(
                        "{name} must be between {lo} and {hi}"
                    )));
                }
                *dst = v;
            }
            Ok(())
        };
        set(
            &mut o.thickness_mm,
            self.thickness_mm,
            "thickness_mm",
            0.3,
            2.0,
        )?;
        set(
            &mut o.min_thickness_mm,
            self.min_thickness_mm,
            "min_thickness_mm",
            0.3,
            1.5,
        )?;
        set(
            &mut o.opening_target_mm,
            self.opening_target_mm,
            "opening_target_mm",
            0.0,
            5.0,
        )?;
        set(
            &mut o.hinge_posterior_mm,
            self.hinge_posterior_mm,
            "hinge_posterior_mm",
            40.0,
            120.0,
        )?;
        set(
            &mut o.hinge_height_mm,
            self.hinge_height_mm,
            "hinge_height_mm",
            0.0,
            80.0,
        )?;
        set(
            &mut o.equilibrate_max_mm,
            self.equilibrate_max_mm,
            "equilibrate_max_mm",
            0.05,
            1.0,
        )?;
        if let Some(e) = self.equilibrate {
            o.equilibrate = e;
        }
        if let Some(n) = self.equilibrate_teeth_per_side {
            if !(1..=6).contains(&n) {
                return Err(CommandError::Invalid(
                    "equilibrate_teeth_per_side must be between 1 and 6".to_owned(),
                ));
            }
            o.equilibrate_teeth_per_side = n;
        }
        Ok(o)
    }
}

/// What the ramps command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RampAction {
    /// Propose the anterior ramps of a deep bite, replacing the ones
    /// proposed before (the ones placed by hand stay).
    #[default]
    Propose,
    /// Place a ramp by hand on `tooth` at `point` with the outward `normal`.
    Add,
    /// Take off the ramps of `tooth` (all of them without `tooth`).
    Remove,
    /// Only change the settings.
    Set,
}

/// Parameters of the ramps command.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RampParams {
    /// What to do.
    pub action: RampAction,
    /// Aligner number the proposal reads (0 the initial situation).
    pub stage: u32,
    /// The ramps enter the aligners (default: on after a proposal or an
    /// added ramp).
    pub enabled: Option<bool>,
    /// Tooth (FDI number) of `add` and `remove`.
    pub tooth: Option<u8>,
    /// Point on the surface of the tooth for `add`, case coordinates of the
    /// tooth at rest (mm).
    pub point: Option<[f64; 3]>,
    /// Outward normal of the tooth there for `add`.
    pub normal: Option<[f64; 3]>,
    /// Width of new ramps, along the tooth (mm).
    pub width_mm: Option<f64>,
    /// Depth of new ramps, towards the gingiva (mm).
    pub depth_mm: Option<f64>,
    /// Height of new ramps (mm).
    pub height_mm: Option<f64>,
    /// Overbite beyond which the anterior ramps are proposed (mm).
    pub overbite_min_mm: Option<f64>,
}

// ----------------------------------------------------------------------------
// the arches at a stage
// ----------------------------------------------------------------------------

fn invalid(e: impl std::fmt::Display) -> CommandError {
    CommandError::Invalid(e.to_string())
}

fn fdi_of(n: i64) -> Option<Fdi> {
    u8::try_from(n).ok().and_then(|n| Fdi::new(n).ok())
}

/// An arch of the case with its closed crowns.
struct ArchCase {
    arch: Arch,
    model: ArchModel,
    /// Case coordinates to arch coordinates.
    to_arch: DMat4,
    /// The closed crown of every tooth at rest (case coordinates): the
    /// reconstructed solids, else the labelled triangles of the scan.
    crowns: BTreeMap<Fdi, TriMesh>,
}

fn arch_case(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<ArchCase, CommandError> {
    let model = arch_model(state, data, arch)?;
    let to_arch = frame::rigid_inverse(&model.to_case);
    let mut crowns = BTreeMap::new();
    for (&f, r) in data.crowns.iter().filter(|(f, _)| Arch::of(**f) == arch) {
        crowns.insert(f, state.blobs.require(r)?.as_ref().clone());
    }
    let verts = model.case_vertices();
    let mut by_tooth: BTreeMap<u16, Vec<[u32; 3]>> = BTreeMap::new();
    for t in &model.tris {
        let l = model.labels[t[0] as usize];
        if l != 0 && t.iter().all(|&i| model.labels[i as usize] == l) {
            by_tooth.entry(l).or_default().push(*t);
        }
    }
    for (l, tris) in by_tooth {
        let Some(f) = fdi_of(i64::from(l)) else {
            continue;
        };
        if crowns.contains_key(&f) {
            continue;
        }
        let mut map = vec![u32::MAX; verts.len()];
        let mut pos = Vec::new();
        let tris = tris
            .iter()
            .map(|t| {
                t.map(|i| {
                    if map[i as usize] == u32::MAX {
                        map[i as usize] = pos.len() as u32;
                        pos.push(verts[i as usize].as_vec3());
                    }
                    map[i as usize]
                })
            })
            .collect();
        crowns.insert(f, TriMesh::new(pos, tris));
    }
    Ok(ArchCase {
        arch,
        model,
        to_arch,
        crowns,
    })
}

/// The rigid transform of every tooth of an arch at aligner `k` in case
/// coordinates (`x' = D x`): the local pose of the last stage up to `k` that
/// mentions the tooth, in the rest frame of the tooth.
fn stage_deltas(data: &AlignersState, arch: Arch, k: u32) -> BTreeMap<Fdi, DMat4> {
    let mut stages: Vec<_> = data.stages.iter().filter(|s| s.index <= k).collect();
    stages.sort_by_key(|s| s.index);
    let mut local: BTreeMap<Fdi, Pose> = BTreeMap::new();
    for s in stages {
        for (&f, &p) in &s.poses {
            if Arch::of(f) == arch {
                local.insert(f, p);
            }
        }
    }
    local
        .into_iter()
        .filter_map(|(f, p)| {
            let rest = data.tooth_frames.get(&f)?.pose;
            Some((f, staging::world_delta(&rest, &p).to_mat4()))
        })
        .collect()
}

/// The aligners in the series.
fn n_stages(data: &AlignersState) -> u32 {
    data.stages.iter().map(|s| s.index).max().unwrap_or(0)
}

/// One arch at one stage, in its arch coordinates.
struct Placed {
    stage: ArchStage,
    /// Crown points of each tooth (about every 0.2 mm).
    crowns: BTreeMap<i64, Vec<DVec3>>,
    /// Tooth coordinates at rest to arch coordinates at the stage.
    to_arch: BTreeMap<Fdi, DMat4>,
}

fn place(c: &ArchCase, deltas: &BTreeMap<Fdi, DMat4>, ramps: &[Ramp], use_ramps: bool) -> Placed {
    let delta = |f: Fdi| deltas.get(&f).copied().unwrap_or(DMat4::IDENTITY);
    let to_case = c.model.to_case;
    let mut s = ArchStage::default();
    let mut crowns = BTreeMap::new();
    for (&l, idx) in &c.model.crown_idx {
        let Some(f) = fdi_of(i64::from(l)) else {
            continue;
        };
        let own: Vec<DVec3> = idx.iter().map(|&i| c.model.vertices[i as usize]).collect();
        // arch coordinates at rest to arch coordinates at the stage
        let a = c.to_arch * delta(f) * to_case;
        let pts: Vec<DVec3> = occlusal::crown_cells(&own, occlusal::STEP)
            .into_iter()
            .map(|p| a.transform_point3(p))
            .collect();
        s.crown_labels
            .extend(std::iter::repeat_n(i64::from(l), pts.len()));
        s.crown_points.extend_from_slice(&pts);
        s.covered.push(i64::from(l));
        crowns.insert(i64::from(l), pts);
    }
    let to_arch: BTreeMap<Fdi, DMat4> = c
        .crowns
        .keys()
        .map(|&f| (f, c.to_arch * delta(f)))
        .collect();
    let teeth: Vec<TriMesh> = c
        .crowns
        .iter()
        .map(|(f, m)| {
            let a = to_arch[f];
            TriMesh::new(
                m.positions
                    .iter()
                    .map(|p| a.transform_point3(p.as_dvec3()).as_vec3())
                    .collect(),
                m.tris.clone(),
            )
        })
        .collect();
    (s.model_points, s.model_labels) =
        occlusal::model_points(&teeth, &s.crown_points, &s.crown_labels);
    if use_ramps {
        let own: Vec<Ramp> = ramps
            .iter()
            .filter(|r| Arch::of(r.fdi) == c.arch)
            .copied()
            .collect();
        (s.ramp_points, s.ramp_labels) = ramps::points(&ramps::solids(&own, &to_arch));
    }
    Placed {
        stage: s,
        crowns,
        to_arch,
    }
}

/// The two arches of the case, ready for a stage.
struct Pair {
    lower: ArchCase,
    upper: ArchCase,
    data: AlignersState,
}

fn pair(state: &CaseState) -> Result<Pair, CommandError> {
    let data = aligners_data(state)?;
    Ok(Pair {
        lower: arch_case(state, &data, Arch::Lower)?,
        upper: arch_case(state, &data, Arch::Upper)?,
        data,
    })
}

// ----------------------------------------------------------------------------
// bite with the aligners worn
// ----------------------------------------------------------------------------

fn run_bite(
    state: &CaseState,
    params: BiteParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let p = pair(state)?;
    let occlusal_settings = params.settings(&p.data.aligner_bite.occlusal)?;
    let n = n_stages(&p.data);
    let (kl, ku) = occlusal::stage_pair(params.stage, n, n);
    let use_ramps = p.data.aligner_bite.ramps.enabled;
    let ramps_list = &p.data.aligner_bite.ramps.ramps;
    let lower = place(
        &p.lower,
        &stage_deltas(&p.data, Arch::Lower, kl),
        ramps_list,
        use_ramps,
    );
    let upper = place(
        &p.upper,
        &stage_deltas(&p.data, Arch::Upper, ku),
        ramps_list,
        use_ramps,
    );
    ctx.check_cancelled()?;
    let rel = occlusion::relation(&p.lower.model, &p.upper.model);
    let design = Design::new(&p.data.settings, &occlusal_settings, &p.data.trim);
    let b = occlusal::bite(&lower.stage, &upper.stage, &rel, &design, [kl, ku])
        .ok_or_else(|| invalid("the arches have no crown points"))?;
    let (pts, gaps) = occlusal::contact_points(&b);
    let mut out =
        serde_json::to_value(&b.report).map_err(|e| CommandError::Failed(e.to_string()))?;
    out["contacts"] = json!(pts.len());
    out["contact_gap_min_mm"] = json!(gaps.iter().cloned().reduce(f64::min));
    out["ramps"] = json!(if use_ramps { ramps_list.len() } else { 0 });
    out["excluded_nodes"] = json!(b.excluded);
    let mut next = state.clone();
    if params.save.unwrap_or(true) && p.data.aligner_bite.occlusal != occlusal_settings {
        let mut data = p.data.clone();
        data.aligner_bite.occlusal = occlusal_settings;
        next.set_module(MODULE_ID, ModuleState::new(data));
    }
    Ok((next, out))
}

// ----------------------------------------------------------------------------
// ramps
// ----------------------------------------------------------------------------

fn ramp_json(r: &Ramp) -> Value {
    json!({
        "tooth": r.fdi.get(),
        "kind": match r.kind { RampKind::Auto => "auto", RampKind::Manual => "manual" },
        "position": r.pose.t.to_array(),
        "size_mm": r.size_mm,
    })
}

fn check_size(name: &str, v: Option<f64>, lo: f64, hi: f64) -> Result<(), CommandError> {
    match v {
        Some(v) if !(lo..=hi).contains(&v) => Err(CommandError::Invalid(format!(
            "{name} must be between {lo} and {hi} mm"
        ))),
        _ => Ok(()),
    }
}

/// The ramps and the bite with the aligners worn, prepared once for the
/// print files of every stage (the export): the arches of the case with
/// their crowns, in the arch coordinates of the bite (`ArchModel`).
pub(crate) struct StageBite {
    lower: Option<ArchCase>,
    upper: Option<ArchCase>,
    rel: Option<DMat4>,
    data: AlignersState,
    design: Design,
}

impl StageBite {
    /// The arches of the case that are segmented (an arch that is not has
    /// no ramps and no bite).
    pub(crate) fn new(state: &CaseState) -> Result<Self, CommandError> {
        let data = aligners_data(state)?;
        let lower = arch_case(state, &data, Arch::Lower).ok();
        let upper = arch_case(state, &data, Arch::Upper).ok();
        let rel = match (&lower, &upper) {
            (Some(l), Some(u)) => Some(occlusion::relation(&l.model, &u.model)),
            _ => None,
        };
        let design = Design::new(&data.settings, &data.aligner_bite.occlusal, &data.trim);
        Ok(Self {
            lower,
            upper,
            rel,
            data,
            design,
        })
    }

    fn arch(&self, arch: Arch) -> Option<&ArchCase> {
        if arch.is_upper() {
            self.upper.as_ref()
        } else {
            self.lower.as_ref()
        }
    }

    /// Case to the arch coordinates of the ramps and of the bite.
    pub(crate) fn to_arch(&self, arch: Arch) -> Option<DMat4> {
        self.arch(arch).map(|c| c.to_arch)
    }

    /// The design of the bite (clearance, wall, blockout, occlusal settings).
    pub(crate) fn design(&self) -> &Design {
        &self.design
    }

    /// The occlusal limit changes the shells (`case_occlusal.active`).
    pub(crate) fn shapes_shells(&self) -> bool {
        self.data.aligner_bite.occlusal.active()
    }

    /// Why there is no bite to shape the shells with; `None`: there is one.
    pub(crate) fn missing(&self) -> Option<&'static str> {
        match (&self.lower, &self.upper) {
            (Some(_), Some(_)) => None,
            _ => Some("the bite needs both arches, with teeth and gum separated"),
        }
    }

    /// The ramps of `arch` at aligner `k` as solids (arch coordinates of
    /// [`Self::to_arch`]); empty when the ramps are off.
    pub(crate) fn ramps(&self, arch: Arch, k: u32) -> Vec<RampSolid> {
        let ramps = &self.data.aligner_bite.ramps;
        let Some(c) = self.arch(arch).filter(|_| ramps.enabled) else {
            return Vec::new();
        };
        let own: Vec<Ramp> = ramps
            .ramps
            .iter()
            .filter(|r| Arch::of(r.fdi) == arch)
            .copied()
            .collect();
        if own.is_empty() {
            return Vec::new();
        }
        let placed = place(c, &stage_deltas(&self.data, arch, k), &own, false);
        ramps::solids(&own, &placed.to_arch)
    }

    /// The bite of the two aligners `kl` (lower) and `ku` (upper), the ramps
    /// included when they are on; `None` without both arches or crown points.
    pub(crate) fn bite(&self, kl: u32, ku: u32) -> Option<occlusal::Bite> {
        let (l, u, rel) = (self.lower.as_ref()?, self.upper.as_ref()?, self.rel?);
        let use_ramps = self.data.aligner_bite.ramps.enabled;
        let list = &self.data.aligner_bite.ramps.ramps;
        let lower = place(
            l,
            &stage_deltas(&self.data, Arch::Lower, kl),
            list,
            use_ramps,
        );
        let upper = place(
            u,
            &stage_deltas(&self.data, Arch::Upper, ku),
            list,
            use_ramps,
        );
        occlusal::bite(&lower.stage, &upper.stage, &rel, &self.design, [kl, ku])
    }

    /// The teeth under the aligner of `arch` (all its crowns).
    pub(crate) fn covered(&self, arch: Arch) -> Vec<i64> {
        self.arch(arch)
            .map(|c| c.crowns.keys().map(|f| i64::from(f.get())).collect())
            .unwrap_or_default()
    }
}

/// The ramps of the case as solids in the arch coordinates of `arch` at
/// stage `k` (for the shell: [`dca_algo::ramps::apply_to_grid`]); empty when
/// the ramps are off.
#[allow(dead_code)]
pub(crate) fn ramp_solids(
    state: &CaseState,
    arch: Arch,
    k: u32,
) -> Result<Vec<RampSolid>, CommandError> {
    let data = aligners_data(state)?;
    if !data.aligner_bite.ramps.enabled {
        return Ok(vec![]);
    }
    let c = arch_case(state, &data, arch)?;
    let placed = place(
        &c,
        &stage_deltas(&data, arch, k),
        &data.aligner_bite.ramps.ramps,
        true,
    );
    let own: Vec<Ramp> = data
        .aligner_bite
        .ramps
        .ramps
        .iter()
        .filter(|r| Arch::of(r.fdi) == arch)
        .copied()
        .collect();
    Ok(ramps::solids(&own, &placed.to_arch))
}

fn run_ramps(
    state: &CaseState,
    params: RampParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    check_size("width_mm", params.width_mm, 0.5, 8.0)?;
    check_size("depth_mm", params.depth_mm, 0.5, 6.0)?;
    check_size("height_mm", params.height_mm, 0.2, 4.0)?;
    check_size("overbite_min_mm", params.overbite_min_mm, 1.0, 8.0)?;
    let mut data = aligners_data(state)?;
    {
        let s = &mut data.aligner_bite.ramps;
        if let Some(v) = params.width_mm {
            s.width_mm = v;
        }
        if let Some(v) = params.depth_mm {
            s.depth_mm = v;
        }
        if let Some(v) = params.height_mm {
            s.height_mm = v;
        }
        if let Some(v) = params.overbite_min_mm {
            s.overbite_min_mm = v;
        }
    }
    let tooth = params
        .tooth
        .map(|n| Fdi::new(n).map_err(invalid))
        .transpose()?;
    let mut out = json!({});
    match params.action {
        RampAction::Set => {}
        RampAction::Remove => {
            let list = &mut data.aligner_bite.ramps.ramps;
            let before = list.len();
            list.retain(|r| tooth.is_some_and(|t| r.fdi != t));
            out["removed"] = json!(before - list.len());
        }
        RampAction::Add => {
            let (Some(t), Some(point), Some(normal)) = (tooth, params.point, params.normal) else {
                return Err(invalid("add needs tooth, point and normal"));
            };
            let normal = DVec3::from_array(normal);
            if normal.length() < 1e-9 {
                return Err(invalid("the normal must not be zero"));
            }
            let p = pair(state)?;
            let c = if Arch::of(t) == Arch::Upper {
                &p.upper
            } else {
                &p.lower
            };
            if !c.crowns.contains_key(&t) {
                return Err(invalid(format!(
                    "the tooth {} has no crown in the case",
                    t.get()
                )));
            }
            let rs = &data.aligner_bite.ramps;
            let size = [rs.width_mm, rs.depth_mm, rs.height_mm];
            let r = ramps::manual_ramp(
                t,
                DVec3::from_array(point),
                normal,
                size,
                data.settings.clearance_mm,
                data.settings.shell_thickness_mm,
            );
            let rs = &mut data.aligner_bite.ramps;
            rs.ramps.push(r);
            rs.enabled |= params.enabled.is_none();
            out["added"] = ramp_json(&r);
        }
        RampAction::Propose => {
            let p = pair(state)?;
            let n = n_stages(&data);
            let (kl, ku) = occlusal::stage_pair(params.stage, n, n);
            // the proposal sees the teeth, not the ramps already there
            let lower = place(&p.lower, &stage_deltas(&data, Arch::Lower, kl), &[], false);
            let upper = place(&p.upper, &stage_deltas(&data, Arch::Upper, ku), &[], false);
            let upper_deltas = stage_deltas(&data, Arch::Upper, ku);
            let teeth: BTreeMap<i64, ProposeTooth<'_>> = p
                .upper
                .crowns
                .iter()
                .map(|(f, m)| {
                    (
                        i64::from(f.get()),
                        ProposeTooth {
                            mesh: m,
                            to_arch: p.upper.to_arch
                                * upper_deltas.get(f).copied().unwrap_or(DMat4::IDENTITY),
                        },
                    )
                })
                .collect();
            let covered = upper.stage.covered.clone();
            let input = ProposeInput {
                lower_crowns: &lower.crowns,
                upper_crowns: &upper.crowns,
                rel: occlusion::relation(&p.lower.model, &p.upper.model),
                upper_teeth: &teeth,
                covered: &covered,
                clearance: data.settings.clearance_mm,
                wall: data.settings.shell_thickness_mm,
            };
            let proposal = ramps::propose(&input, &data.aligner_bite.ramps);
            out["overbite_mm"] = json!(proposal.overbite_mm.map(|o| (o * 100.0).round() / 100.0));
            out["reason"] = json!(proposal.reason);
            out["stages"] = json!([kl, ku]);
            if !proposal.ramps.is_empty() {
                let rs = &mut data.aligner_bite.ramps;
                rs.ramps.retain(|r| r.kind != RampKind::Auto);
                rs.ramps.extend(proposal.ramps.iter().copied());
                rs.enabled |= params.enabled.is_none();
                out["proposed"] = Value::Array(proposal.ramps.iter().map(ramp_json).collect());
            }
        }
    }
    if let Some(e) = params.enabled {
        data.aligner_bite.ramps.enabled = e;
    }
    let s = &data.aligner_bite.ramps;
    out["enabled"] = json!(s.enabled);
    out["ramps"] = Value::Array(s.ramps.iter().map(ramp_json).collect());
    out["teeth"] = json!(s.ramps.iter().map(|r| r.fdi.get()).collect::<BTreeSet<_>>());
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, out))
}

#[cfg(test)]
mod tests;
