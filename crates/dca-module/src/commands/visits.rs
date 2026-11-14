//! Commands of the `visits` stage (`dca_algo::visits`).
//!
//! `aligners.followup.visits` analyses a control visit: a progress scan taken
//! while the patient wears the aligners is placed on the situation the plan
//! expects after the aligner `stage` (the case's scan with its teeth and
//! gingiva moved to that stage), every tooth of the plan is registered to the
//! scan, and the position it really has is compared with the planned one. A
//! tooth is on track when every component of the difference is within 0.5 mm
//! and 2 degrees. The scan is then segmented with seeds on the achieved crowns
//! (`segment`, on by default), so that a new plan can start from it.
//!
//! The visit is appended to `AlignersState::visits`: the figures of every
//! tooth, the decision (on track, or refinement suggested with the velocity
//! factor of the lagging teeth), and the scan placed on the plan as a mesh
//! blob (with the tooth labels when segmented). The plan itself, the scans
//! and the tooth frames are not modified.
//!
//! `aligners.followup.tracking` re-plans from where the treatment really is
//! (`case_plan.replan_from_stage`, `visit.new_visit`): the aligners already
//! delivered (1..=k) are kept and a new series starts after them, towards the
//! same final setup, numbered k+1, k+2... The starting position of every
//! tooth is the one measured at a control visit (`visit`; the teeth that were
//! behind move at the visit's velocity factor) or, without a scan, the
//! planned position at the delivered aligner `stage`.
//!
//! Inputs from the case: the labelled scan of the arch, the tooth frames
//! (`aligners.teeth.frames`), the planned aligners (`Stage::poses`; none is
//! needed to compare with the initial situation, `stage` 0), and the closed
//! crowns of the reconstruction when there are (else the labelled vertices of
//! the scan are the crowns).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use dc_case::import::{self, ImportError, ImportWarning, PrepareOptions};
use dc_case::{CaseState, Fdi, ModuleState, Pose, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::frame;
use dca_algo::gingiva;
use dca_algo::schedule;
use dca_algo::segmentation;
use dca_algo::staging::{self, IprPlan, Pattern, ToothMove};
use dca_algo::teeth;
use dca_algo::visits::{self, MeshRef, ToothInput, VisitError};
use dca_model::{AlignersState, IprStep, MODULE_ID, Seed, Stage, ToothTracking, Visit};
use glam::{DMat3, DMat4, DVec3};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::messages;
use super::staging::{aligners_data, arch_name, complexity_of, labelled_scan, md_sign};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.followup.visits",
            label: "aligners-menu-followup-visits",
            doc: "aligners-cmd-visits-doc",
            params: Params,
            run: run_visit,
        },
        stage_command! {
            id: "aligners.followup.tracking",
            label: "aligners-menu-followup-tracking",
            doc: "aligners-cmd-tracking-doc",
            params: ReplanParams,
            run: run_replan,
        },
    ]
}

/// Parameters of the command.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Params {
    /// Path of the progress scan (STL, OBJ or PLY; the unit is detected).
    scan: String,
    /// Arch of the scan (`upper` or `lower`); the only arch with tooth frames
    /// when absent.
    arch: Option<Arch>,
    /// Date of the visit, `YYYY-MM-DD` (default today).
    date: Option<String>,
    /// The aligner the patient wears, i.e. the stage of the plan the scan is
    /// compared with (0 is the initial situation); the last aligner of the
    /// arch when absent.
    stage: Option<u32>,
    /// Linear deviation (mm) beyond which a tooth is not on track (default
    /// the evidence value, 0.5).
    linear_mm: Option<f64>,
    /// Angular deviation (degrees) beyond which a tooth is not on track
    /// (default the evidence value, 2).
    angular_deg: Option<f64>,
    /// Velocity factor of the teeth that are behind, for the new aligners
    /// (default the evidence value, 0.75).
    lag_speed: Option<f64>,
    /// Segment the scan with seeds on the achieved crowns (default true).
    segment: Option<bool>,
}

impl Params {
    fn visit_params(&self) -> Result<visits::Params, CommandError> {
        let mut p = visits::Params::default();
        for (name, value, target) in [
            ("linear_mm", self.linear_mm, &mut p.linear_mm),
            ("angular_deg", self.angular_deg, &mut p.angular_deg),
            ("lag_speed", self.lag_speed, &mut p.lag_speed),
        ] {
            if let Some(v) = value {
                if !(v.is_finite() && v > 0.0) {
                    return Err(CommandError::Invalid(format!(
                        "{name} must be a positive number"
                    )));
                }
                *target = v;
            }
        }
        if p.lag_speed > 1.0 {
            return Err(CommandError::Invalid(
                "lag_speed is a factor between 0 and 1".to_owned(),
            ));
        }
        Ok(p)
    }
}

fn map_err(e: VisitError) -> CommandError {
    match e {
        VisitError::Invalid(_) | VisitError::NoTeeth => CommandError::Invalid(e.to_string()),
        VisitError::Frame(..) | VisitError::Registration(_) => CommandError::Failed(e.to_string()),
    }
}

/// The arch asked for, or the only one with tooth frames.
fn pick_arch(params: &Params, data: &AlignersState) -> Result<Arch, CommandError> {
    if let Some(arch) = params.arch {
        return Ok(arch);
    }
    let has = |a: Arch| data.tooth_frames.keys().any(|&f| Arch::of(f) == a);
    match (has(Arch::Upper), has(Arch::Lower)) {
        (true, false) => Ok(Arch::Upper),
        (false, true) => Ok(Arch::Lower),
        (false, false) => Err(messages::invalid("aligners-err-no-teeth-any", &[])),
        (true, true) => Err(CommandError::Invalid(
            "the case has both arches: say which one (arch)".to_owned(),
        )),
    }
}

/// Points and unit normals of a crown, in the rest frame of its tooth.
type Crown = (Vec<DVec3>, Vec<DVec3>);

/// Crown points and normals of every tooth in its rest frame: the closed
/// crown of the reconstruction when there is one, else the labelled vertices
/// of the scan with the triangles that have all three.
fn crowns(
    state: &CaseState,
    data: &AlignersState,
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    rest: &BTreeMap<Fdi, Pose>,
) -> Result<BTreeMap<Fdi, Crown>, CommandError> {
    let mut out = BTreeMap::new();
    let mut by_label: BTreeMap<u16, Vec<[u32; 3]>> = BTreeMap::new();
    for t in tris {
        let l = labels[t[0] as usize];
        if l != 0 && labels[t[1] as usize] == l && labels[t[2] as usize] == l {
            by_label.entry(l).or_default().push(*t);
        }
    }
    for (&f, r) in rest {
        let inv = frame::rigid_inverse(&r.to_mat4());
        let rot = DMat3::from_mat4(inv);
        if let Some(blob) = data.crowns.get(&f) {
            let mesh = state.blobs.require(blob)?;
            let pts: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
            let normals = visits::vertex_normals(&pts, &mesh.tris);
            out.insert(
                f,
                (
                    pts.iter().map(|&p| inv.transform_point3(p)).collect(),
                    normals.iter().map(|&n| rot * n).collect(),
                ),
            );
        } else {
            let tri_f = by_label.remove(&u16::from(f.get())).unwrap_or_default();
            let normals = visits::vertex_normals(vertices, &tri_f);
            let idx: Vec<usize> = (0..vertices.len())
                .filter(|&i| labels[i] == u16::from(f.get()))
                .collect();
            out.insert(
                f,
                (
                    idx.iter()
                        .map(|&i| inv.transform_point3(vertices[i]))
                        .collect(),
                    idx.iter().map(|&i| rot * normals[i]).collect(),
                ),
            );
        }
    }
    Ok(out)
}

/// The scan segmented with one seed on every achieved crown: the labels of its
/// vertices. `Ok(None)` when too few teeth could be seeded.
fn segment_visit(
    arch: Arch,
    moved: &[DVec3],
    tris: &[[u32; 3]],
    data: &AlignersState,
    teeth_in: &[ToothInput],
    tracked: &BTreeMap<Fdi, ToothTracking>,
) -> Result<Option<Vec<u16>>, CommandError> {
    let mut seeds: Vec<Seed> = Vec::new();
    for t in teeth_in {
        let Some(tr) = tracked.get(&t.fdi) else {
            continue;
        };
        // the cusp tip of the tooth (else the middle of its crown), where the achieved crown is
        let tip = match data.tooth_geometry.get(&t.fdi) {
            Some(g) => frame::rigid_inverse(&t.rest).transform_point3(g.tip),
            None => t.points.iter().copied().sum::<DVec3>() / t.points.len().max(1) as f64,
        };
        let achieved: DMat4 = t.rest * tr.achieved.to_mat4();
        let p = achieved.transform_point3(tip);
        let (j, d) = moved
            .iter()
            .enumerate()
            .map(|(j, q)| (j, q.distance(p)))
            .fold((0, f64::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
        if d < 2.0 {
            seeds.push(Seed {
                fdi: t.fdi,
                position: moved[j],
            });
        }
    }
    if seeds.is_empty() {
        return Ok(None);
    }
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: moved,
            seeds: &seeds,
            occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
        },
        &frame::Params::default(),
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
    let vertices: Vec<DVec3> = moved.iter().map(|&p| to_arch.transform_point3(p)).collect();
    let mut arch_seeds: Vec<(Fdi, DVec3)> = seeds
        .iter()
        .map(|s| (s.fdi, to_arch.transform_point3(s.position)))
        .collect();
    arch_seeds.sort_by_key(|s| s.0);
    let out = segmentation::run(
        &segmentation::Input {
            arch,
            vertices: &vertices,
            tris,
            seeds: &arch_seeds,
            decimated: None,
        },
        &segmentation::Params::default(),
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok(Some(out.labels))
}

fn round(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k
}

fn tooth_json(t: &ToothTracking) -> Value {
    json!({
        "on_track": t.on_track,
        "deviation_mm": round(t.linear_mm, 2),
        "deviation_deg": round(t.angular_deg, 1),
        "efficacy_pct": t.efficacy_pct.map(|e| round(e, 0)),
        "planned_mm": round(t.planned_mm, 2),
        "rms_mm": round(t.rms_mm, 3),
        "deviation": {
            "mesial_mm": round(t.deviation.mesial_mm, 2),
            "buccal_mm": round(t.deviation.buccal_mm, 2),
            "extrusion_mm": round(t.deviation.extrusion_mm, 2),
            "torque_deg": round(t.deviation.torque_deg, 2),
            "tip_deg": round(t.deviation.tip_deg, 2),
            "rotation_deg": round(t.deviation.rotation_deg, 2),
        },
    })
}

/// Reads the scan of a visit through the import: the same cleanup and axes
/// as the scans of the case. Like the plugin with a bite scan, the units are
/// left alone ([`PrepareOptions::KEEP_UNITS`]): a progress scan may be a
/// partial one that a guessed unit would grow. A size that fits another unit
/// gives the `units_suspect` warning, returned with the mesh together with the
/// other warnings of the file.
fn read_visit_scan(
    path: &str,
    ctx: &JobCtx,
) -> Result<(Arc<TriMesh>, Vec<ImportWarning>), CommandError> {
    let scan = import::prepare_with(Path::new(path), &PrepareOptions::KEEP_UNITS, ctx).map_err(
        |e| match e {
            ImportError::Cancelled => CommandError::Cancelled,
            e => CommandError::Invalid(match e.detail() {
                Some(detail) => format!("{e} ({detail})"),
                None => e.to_string(),
            }),
        },
    )?;
    Ok((scan.mesh, scan.warnings))
}

/// The arch a visit scan shows, from the name of its file and its shape, as
/// the import recognises it.
fn scan_arch(mesh: &TriMesh, path: &str) -> Result<Arch, CommandError> {
    let name = Path::new(path)
        .file_name()
        .map_or_else(|| path.into(), |n| n.to_string_lossy());
    super::import::detect(mesh, &name)
        .map(|d| d.arch)
        .map_err(|_| messages::invalid("aligners-err-visit-arch", &[]))
}

fn run_visit(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if params.scan.trim().is_empty() {
        return Err(CommandError::Invalid(
            "scan is required: the path of the progress scan".to_owned(),
        ));
    }
    let vp = params.visit_params()?;
    let mut data = aligners_data(state)?;
    // a click gives only the file: with both arches, the file says which one
    let both = |a| data.tooth_frames.keys().any(|&f| Arch::of(f) == a);
    let read = if params.arch.is_none() && both(Arch::Upper) && both(Arch::Lower) {
        Some(read_visit_scan(&params.scan, ctx)?)
    } else {
        None
    };
    let arch = match &read {
        Some((mesh, _)) => scan_arch(mesh, &params.scan)?,
        None => pick_arch(&params, &data)?,
    };
    let role: ScanRole = arch.role();
    let name = arch_name(arch);
    let date = match &params.date {
        Some(text) => schedule::Date::parse(text)
            .ok_or_else(|| CommandError::Invalid(format!("date `{text}` is not YYYY-MM-DD")))?,
        None => schedule::Date::today(),
    }
    .to_iso();

    let fdis: Vec<Fdi> = data
        .tooth_frames
        .keys()
        .copied()
        .filter(|&f| Arch::of(f) == arch)
        .collect();
    if fdis.is_empty() {
        return Err(messages::invalid(
            "aligners-err-arch-no-teeth",
            &[("arch", &messages::arch_word(arch))],
        ));
    }
    let Some(case_scan) = labelled_scan(state, &data, role)? else {
        return Err(messages::not_segmented(arch));
    };
    let last = data.stages.iter().map(|s| s.index).max().unwrap_or(0);
    let worn = params
        .stage
        .unwrap_or_else(|| dca_algo::schedule::active_stages(arch, &data.stages));
    if worn > last {
        return Err(messages::invalid(
            "aligners-err-aligner-not-planned",
            &[("aligner", &worn.to_string()), ("total", &last.to_string())],
        ));
    }

    let (scan, warnings) = match read {
        Some(read) => read,
        None => read_visit_scan(&params.scan, ctx)?,
    };
    let scan_v: Vec<DVec3> = scan.positions.iter().map(|p| p.as_dvec3()).collect();

    // the model the plan expects at that aligner: teeth and gingiva at stage `worn`
    let rest: BTreeMap<Fdi, Pose> = fdis
        .iter()
        .map(|&f| (f, teeth::rest_pose(&data.tooth_frames[&f])))
        .collect();
    let case_v: Vec<DVec3> = case_scan
        .mesh
        .positions
        .iter()
        .map(|p| p.as_dvec3())
        .collect();
    let expected_v = if worn == 0 {
        case_v.clone()
    } else {
        let blend = gingiva::scan_blend(&case_v, &case_scan.mesh.tris, &case_scan.labels)
            .map_err(|e| CommandError::Failed(e.to_string()))?;
        let deltas: BTreeMap<Fdi, DMat4> = fdis
            .iter()
            .map(|&f| (f, visits::delta_at(&rest[&f], &data.stages, f, worn)))
            .collect();
        gingiva::deform_scan(&case_v, &blend, &deltas)
    };
    ctx.set_fraction(0.05);
    ctx.check_cancelled()?;

    // the crowns of the plan at their expected position
    let seq = anatomy::arch_sequence(&fdis, arch);
    let mut crown = crowns(
        state,
        &data,
        &case_v,
        &case_scan.mesh.tris,
        &case_scan.labels,
        &rest,
    )?;
    let teeth_in: Vec<ToothInput> = seq
        .iter()
        .map(|&f| {
            let (points, normals) = crown.remove(&f).unwrap_or_default();
            ToothInput {
                fdi: f,
                rest: rest[&f].to_mat4(),
                expected: visits::expected_frame(&rest[&f], &data.stages, f, worn),
                points,
                normals,
                md_sign: md_sign(&data, &seq, &rest, f),
            }
        })
        .collect();

    // 1. the scan on the plan; 2. every tooth on the scan
    let registration = visits::register_scan(
        MeshRef {
            vertices: &expected_v,
            tris: &case_scan.mesh.tris,
        },
        MeshRef {
            vertices: &scan_v,
            tris: &scan.tris,
        },
    )
    .map_err(map_err)?;
    ctx.set_fraction(0.3);
    ctx.check_cancelled()?;
    let (moved, normals) = visits::registered_scan(
        &registration,
        MeshRef {
            vertices: &scan_v,
            tris: &scan.tris,
        },
    );
    let order = anatomy::arch_order(arch);
    let tracked = visits::track(&moved, &normals, &teeth_in, &order, &vp);
    if tracked.is_empty() {
        return Err(map_err(VisitError::NoTeeth));
    }
    let outcome = visits::summarize(registration, tracked, &teeth_in, &vp);
    ctx.set_fraction(0.6);
    ctx.check_cancelled()?;

    // 3. the scan becomes the visit's case: placed on the plan, segmented
    let mut note = Value::Null;
    let mut labels = None;
    if params.segment.unwrap_or(true) {
        match segment_visit(arch, &moved, &scan.tris, &data, &teeth_in, &outcome.teeth) {
            Ok(Some(l)) => labels = Some(l),
            Ok(None) => note = json!("no tooth could be seeded on the scan: not segmented"),
            Err(CommandError::Cancelled) => return Err(CommandError::Cancelled),
            Err(e) => note = json!(format!("not segmented: {e}")),
        }
    }
    ctx.check_cancelled()?;
    let mut placed = TriMesh::new(
        moved.iter().map(|p| p.as_vec3()).collect(),
        scan.tris.clone(),
    );
    placed.vertex_labels = labels;
    let segmented = placed.vertex_labels.is_some();

    let mut next = state.clone();
    let blob = next.blobs.insert(placed);
    let number = data.visits.len() as u32 + 1;
    let source_name = Path::new(&params.scan)
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let visit = Visit {
        number,
        date: date.clone(),
        arch: role,
        stage: worn,
        source_name,
        registration_rms_mm: outcome.registration.rms_mm,
        thresholds: [vp.linear_mm, vp.angular_deg],
        teeth: outcome.teeth.clone(),
        untracked: outcome.untracked.clone(),
        on_track: outcome.on_track as u32,
        decision: outcome.decision,
        speed: outcome.speed.clone(),
        scan: Some(blob),
        segmented,
    };
    let behind: Map<String, Value> = visit
        .lagging()
        .iter()
        .map(|f| (f.to_string(), tooth_json(&visit.teeth[f])))
        .collect();
    let report = json!({
        "visit": number,
        "date": date,
        "arch": name,
        "stage": worn,
        "registration_rms_mm": round(visit.registration_rms_mm, 3),
        "scan_normals_inverted": outcome.registration.normal_sign < 0.0,
        "teeth": visit.teeth.len(),
        "on_track": visit.on_track,
        "decision": visit.decision,
        "thresholds": { "linear_mm": vp.linear_mm, "angular_deg": vp.angular_deg },
        "behind": behind,
        "untracked": visit.untracked,
        "speed": visit.speed.iter().map(|(f, s)| (f.to_string(), json!(s))).collect::<Map<_, _>>(),
        "detail": visit.teeth.iter().map(|(f, t)| (f.to_string(), tooth_json(t))).collect::<Map<_, _>>(),
        "segmented": segmented,
        "segmentation_note": note,
        "warnings": warnings,
        "scan_blob": blob.to_hex(),
        "summary": format!(
            "{}/{} teeth on track at aligner {worn}{}",
            visit.on_track,
            visit.teeth.len(),
            if visit.speed.is_empty() { "" } else { "; refinement suggested" },
        ),
    });
    data.visits.push(visit);
    next.set_module(MODULE_ID, ModuleState::new(data));
    ctx.set_fraction(1.0);
    Ok((next, report))
}

// ---- re-planning ----------------------------------------------------------------------------------------

/// Parameters of `aligners.followup.tracking`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ReplanParams {
    /// Arch to re-plan (`upper` or `lower`); the arch of the visit, else the
    /// only arch with a plan.
    arch: Option<Arch>,
    /// Number of the control visit to start from (its measured positions and
    /// velocity factors); the last visit of the arch when neither `visit` nor
    /// `stage` is given.
    visit: Option<u32>,
    /// Delivered aligner to start from, without a scan: the planned position
    /// at that stage. Not together with `visit`.
    stage: Option<u32>,
    /// Distribution pattern of the new series (default the case's setting).
    pattern: Option<Pattern>,
}

/// Puts the series of one arch (`new`, aligners 1..) into the stages of the
/// case, replacing what the arch had; the arch that finishes first repeats
/// its last pose (the staging command's `merge_arch`).
fn merge_arch(stages: &mut Vec<Stage>, arch: Arch, new: &[Stage], seq: &[Fdi]) {
    for s in stages.iter_mut() {
        s.poses.retain(|f, _| Arch::of(*f) != arch);
        s.ipr.retain(|r| Arch::of(r.between[0]) != arch);
    }
    stages.sort_by_key(|s| s.index);
    let total = stages.len().max(new.len());
    for k in stages.len() + 1..=total {
        stages.push(Stage::new(k as u32));
    }
    for (i, s) in stages.iter_mut().enumerate() {
        if new.is_empty() {
            break;
        }
        let src = &new[i.min(new.len() - 1)];
        for &f in seq {
            if let Some(p) = src.poses.get(&f) {
                s.poses.insert(f, *p);
            }
        }
        if let Some(own) = new.get(i) {
            s.ipr.extend(own.ipr.iter().copied());
        }
    }
    for i in 1..stages.len() {
        let (before, rest) = stages.split_at_mut(i);
        for (f, p) in &before[i - 1].poses {
            rest[0].poses.entry(*f).or_insert(*p);
        }
    }
    stages.retain(|s| !s.poses.is_empty() || !s.ipr.is_empty());
}

fn run_replan(
    state: &CaseState,
    params: ReplanParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if params.visit.is_some() && params.stage.is_some() {
        return Err(CommandError::Invalid(
            "give either visit (start from its measured positions) or stage (start from the plan), not both"
                .to_owned(),
        ));
    }
    let mut data = aligners_data(state)?;
    // the starting point: a visit, or a delivered aligner of the plan
    let visit: Option<Visit> = match (params.visit, params.stage) {
        (Some(n), _) => Some(
            data.visits
                .iter()
                .find(|v| v.number == n)
                .cloned()
                .ok_or_else(|| CommandError::Invalid(format!("there is no visit {n}")))?,
        ),
        (None, Some(_)) => None,
        (None, None) => {
            let arch_role = params.arch.map(Arch::role);
            Some(
                data.visits
                    .iter()
                    .rev()
                    .find(|v| arch_role.is_none_or(|r| r == v.arch))
                    .cloned()
                    .ok_or_else(|| messages::invalid("aligners-err-no-visit", &[]))?,
            )
        }
    };
    let arch = match (&visit, params.arch) {
        (Some(v), Some(a)) if v.arch != a.role() => {
            return Err(CommandError::Invalid(format!(
                "visit {} is of the {} arch",
                v.number,
                if v.arch == ScanRole::Upper {
                    "upper"
                } else {
                    "lower"
                }
            )));
        }
        (Some(v), _) => Arch::from_role(v.arch)
            .ok_or_else(|| CommandError::Invalid("the visit has no arch".to_owned()))?,
        (None, Some(a)) => a,
        (None, None) => pick_arch(&Params::default(), &data)?,
    };
    let name = arch_name(arch);
    let fdis: Vec<Fdi> = data
        .tooth_frames
        .keys()
        .copied()
        .filter(|&f| Arch::of(f) == arch)
        .collect();
    if fdis.is_empty() || !fdis.iter().any(|f| data.setup.poses.contains_key(f)) {
        return Err(messages::invalid(
            "aligners-err-arch-no-plan",
            &[("arch", &messages::arch_word(arch))],
        ));
    }
    let last = data.stages.iter().map(|s| s.index).max().unwrap_or(0);
    let from = visit
        .as_ref()
        .map_or_else(|| params.stage.unwrap_or(0), |v| v.stage);
    if from > last {
        return Err(CommandError::Invalid(format!(
            "aligner {from} is not in the plan ({last} aligners)"
        )));
    }
    let Some(scan) = labelled_scan(state, &data, arch.role())? else {
        return Err(messages::not_segmented(arch));
    };

    // the arch frame the staging works in, the crowns in the rest frames
    let case_v: Vec<DVec3> = scan.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: &case_v,
            seeds: &data.seeds,
            occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
        },
        &frame::Params::default(),
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
    let seq = anatomy::arch_sequence(&fdis, arch);
    let rest: BTreeMap<Fdi, Pose> = seq
        .iter()
        .map(|&f| (f, teeth::rest_pose(&data.tooth_frames[&f])))
        .collect();
    let mut crown = crowns(state, &data, &case_v, &scan.mesh.tris, &scan.labels, &rest)?;
    ctx.check_cancelled()?;

    let mut moves = Vec::with_capacity(seq.len());
    let mut speeds = Map::new();
    for &f in &seq {
        let target = data
            .setup
            .poses
            .get(&f)
            .map_or([0.0; 6], staging::q_from_pose);
        // where the tooth is: measured at the visit, else planned at `from`
        let start = match visit.as_ref().and_then(|v| v.teeth.get(&f)) {
            Some(t) => t.achieved,
            None => visits::pose_at(&data.stages, f, from),
        };
        let mut m = ToothMove::new(
            f,
            to_arch * rest[&f].to_mat4(),
            target,
            crown.remove(&f).unwrap_or_default().0,
            md_sign(&data, &seq, &rest, f),
        );
        m.q0 = Some(staging::q_from_pose(&start));
        if let Some(sp) = visit.as_ref().and_then(|v| v.speed.get(&f)) {
            m.speed = *sp;
            speeds.insert(f.to_string(), json!(sp));
        }
        moves.push(m);
    }

    // IPR still to do: what the setup plans less what the delivered aligners did
    let mut ipr: Vec<IprPlan> = Vec::new();
    for r in data
        .setup
        .ipr
        .iter()
        .filter(|r| Arch::of(r.between[0]) == arch)
    {
        let done: f64 = data
            .stages
            .iter()
            .filter(|s| s.index <= from)
            .flat_map(|s| &s.ipr)
            .filter(|d| d.between == r.between || d.between == [r.between[1], r.between[0]])
            .map(|d| d.amount_mm)
            .sum();
        let left = r.amount_mm - done;
        if left > 1e-9 {
            ipr.push(IprPlan {
                a: r.between[0],
                b: r.between[1],
                mm: left,
            });
        }
    }
    let mut sp = staging::Params::from_settings(&data.settings.staging);
    if let Some(p) = params.pattern {
        sp.pattern = p;
    }
    let complexity = complexity_of(&data, arch.role(), &moves);
    let mut progress = |fraction: f64| {
        ctx.set_fraction(fraction as f32);
        !ctx.is_cancelled()
    };
    let plan = staging::run_with_progress(
        &staging::Input {
            moves: &moves,
            ipr: &ipr,
            complexity,
        },
        &sp,
        &mut progress,
    )
    .map_err(|_| CommandError::Cancelled)?;

    // the delivered aligners of the arch, then the new series numbered from there
    let mut series: Vec<Stage> = (1..=from)
        .map(|k| {
            let mut st = Stage::new(k);
            for &f in &seq {
                st.poses.insert(f, visits::pose_at(&data.stages, f, k));
            }
            if let Some(old) = data.stages.iter().find(|s| s.index == k) {
                st.ipr = old
                    .ipr
                    .iter()
                    .filter(|r| Arch::of(r.between[0]) == arch)
                    .copied()
                    .collect();
            }
            st
        })
        .collect();
    for j in 1..=plan.n_stages {
        let mut st = Stage::new(from + j);
        for (&f, qs) in &plan.q {
            st.poses.insert(f, staging::pose_from_q(&qs[j as usize]));
        }
        for r in plan.ipr_stage.iter().filter(|r| r.stage == j) {
            let mm = ipr
                .iter()
                .find(|p| (p.a, p.b) == (r.a, r.b) || (p.a, p.b) == (r.b, r.a))
                .map_or(0.0, |p| p.mm);
            st.ipr.push(IprStep {
                between: [r.a, r.b],
                amount_mm: mm,
            });
        }
        series.push(st);
    }
    merge_arch(&mut data.stages, arch, &series, &seq);
    // the result line of the automatic staging described the plan replaced here
    data.staging_results.remove(&arch.role());
    let total = data.stages.len();

    let teeth_out: Map<String, Value> = plan
        .stats
        .iter()
        .map(|(f, st)| {
            (
                f.to_string(),
                json!({
                    "aligners": plan.need[f],
                    "moving_stages": st.moving_stages,
                    "max_crown_step_mm": round(st.max_crown_step, 3),
                }),
            )
        })
        .collect();
    let report = json!({
        "arch": name,
        "from": visit.as_ref().map_or(json!({ "stage": from }), |v| json!({ "visit": v.number, "stage": from })),
        "kept_aligners": from,
        "new_aligners": plan.n_stages,
        "first_new_aligner": from + 1,
        "total_aligners": total,
        "pattern": plan.pattern,
        "speed": speeds,
        "teeth": teeth_out,
        "ipr_remaining": ipr.iter().map(|p| json!({ "between": [p.a, p.b], "mm": round(p.mm, 3) })).collect::<Vec<_>>(),
        "unresolved": plan.unresolved,
        "notes": plan.log,
    });
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((next, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::Fdi;
    use dc_case::import::WarningCode;
    use dca_algo::demo::{Draws, SeededDraws};
    use dca_algo::staging;
    use glam::DQuat;
    use serde_json::json;

    const ID: &str = "aligners.followup.visits";

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    /// A demo arch with tooth frames and a staged plan.
    fn staged(arch: &str) -> CaseState {
        let s = exec(
            "aligners.demo",
            &CaseState::default(),
            json!({ "arch": arch, "severity": "mild" }),
        )
        .unwrap()
        .0;
        let s = exec("aligners.teeth.frames", &s, Value::Null).unwrap().0;
        let mut data = aligners_data(&s).unwrap();
        let fdis: Vec<Fdi> = data.tooth_frames.keys().copied().collect();
        for (i, f) in fdis.iter().enumerate() {
            let mm = 0.2 * ((i % 3) as f64 + 1.0);
            let rot = if i % 4 == 0 { 3f64.to_radians() } else { 0.0 };
            data.setup.poses.insert(
                *f,
                Pose::new(DVec3::new(mm, 0.0, 0.0), DQuat::from_rotation_z(rot)),
            );
        }
        let mut s = s;
        s.set_module(MODULE_ID, ModuleState::new(data));
        exec("aligners.planning.staging.automatic", &s, Value::Null)
            .unwrap()
            .0
    }

    /// Writes the control scan of a simulated visit: the arch of `state` at
    /// aligner `worn`, then the teeth in `deviation` moved again, the whole
    /// scan placed away. Returns the path.
    fn write_visit_scan(
        state: &CaseState,
        worn: u32,
        deviation: &BTreeMap<Fdi, DMat4>,
        name: &str,
    ) -> std::path::PathBuf {
        let data = aligners_data(state).unwrap();
        let arch = pick_arch(&Params::default(), &data).unwrap();
        let scan = labelled_scan(state, &data, arch.role()).unwrap().unwrap();
        let v: Vec<DVec3> = scan.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let blend = gingiva::scan_blend(&v, &scan.mesh.tris, &scan.labels).unwrap();
        let deltas: BTreeMap<Fdi, DMat4> = data
            .tooth_frames
            .iter()
            .map(|(&f, tf)| {
                let rest = teeth::rest_pose(tf);
                let planned = visits::delta_at(&rest, &data.stages, f, worn);
                let dev = deviation.get(&f).copied().unwrap_or(DMat4::IDENTITY);
                (f, dev * planned)
            })
            .collect();
        let moved = gingiva::deform_scan(&v, &blend, &deltas);
        let place = DMat4::from_translation(DVec3::new(11.0, -7.0, 5.0))
            * DMat4::from_quat(DQuat::from_axis_angle(
                DVec3::new(0.2, 0.3, 1.0).normalize(),
                20f64.to_radians(),
            ));
        let mesh = TriMesh::new(
            moved
                .iter()
                .map(|&p| place.transform_point3(p).as_vec3())
                .collect(),
            scan.mesh.tris.clone(),
        );
        let path =
            std::env::temp_dir().join(format!("dca-visit-{}-{name}.stl", std::process::id()));
        dc_mesh::io::write(&path, &mesh, dc_mesh::MeshFormat::Stl).unwrap();
        path
    }

    #[test]
    fn the_command_is_the_visits_leaf() {
        let spec = crate::commands::find(ID).unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        for p in ["scan", "date", "stage", "arch", "segment"] {
            assert!(schema["properties"][p].is_object(), "{p}");
        }
        assert!(crate::tree::find(ID).is_some());
    }

    #[test]
    fn bad_input_is_reported() {
        let bad = |s: &CaseState, p: Value| matches!(exec(ID, s, p), Err(CommandError::Invalid(_)));
        assert!(bad(&CaseState::default(), Value::Null));
        assert!(bad(&CaseState::default(), json!({ "scan": "x.stl" })));
        let s = staged("lower");
        assert!(bad(&s, Value::Null));
        assert!(bad(&s, json!({ "scan": "/nonexistent/scan.stl" })));
        assert!(bad(&s, json!({ "scan": "x.stl", "date": "yesterday" })));
        assert!(bad(&s, json!({ "scan": "x.stl", "stage": 99 })));
        assert!(bad(&s, json!({ "scan": "x.stl", "arch": "upper" })));
        assert!(bad(&s, json!({ "scan": "x.stl", "lag_speed": 2.0 })));
        assert!(exec(ID, &s, json!({ "scan": "x.stl", "nope": 1 })).is_err());
    }

    #[test]
    fn a_simulated_visit_is_analysed_and_stored() {
        let s = staged("lower");
        let data = aligners_data(&s).unwrap();
        let worn = data.stages.len() as u32;
        let fdis: Vec<Fdi> = data.tooth_frames.keys().copied().collect();
        let mut rng = SeededDraws::new(5);
        // one tooth clearly behind (a 0.9 mm shift), the others within 0.15 mm
        let behind = fdis[4];
        let mut deviation = BTreeMap::new();
        for &f in &fdis {
            let mm = if f == behind { 0.9 } else { 0.0 };
            let t = DVec3::new(
                mm + rng.uniform(-0.1, 0.1),
                rng.uniform(-0.1, 0.1),
                rng.uniform(-0.1, 0.1),
            );
            deviation.insert(f, DMat4::from_translation(t));
        }
        let path = write_visit_scan(&s, worn, &deviation, "analysed");
        let params = json!({
            "scan": path.display().to_string(),
            "date": "2026-03-02",
            "segment": false,
        });
        let result = exec(ID, &s, params.clone());
        let (next, out) = match result {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                panic!("{e}");
            }
        };
        assert_eq!(out["visit"], 1);
        assert_eq!(out["stage"], worn);
        assert_eq!(out["decision"], "refinement");
        assert_eq!(out["teeth"], 14);
        assert_eq!(out["on_track"], 13);
        assert!(out["behind"][behind.to_string()].is_object(), "{out}");
        assert_eq!(out["behind"].as_object().unwrap().len(), 1);
        assert_eq!(out["speed"][behind.to_string()], 0.75);
        assert_eq!(out["segmented"], false);
        assert!(out["registration_rms_mm"].as_f64().unwrap() < 0.1);

        let stored = aligners_data(&next).unwrap();
        assert_eq!(stored.visits.len(), 1);
        let v = &stored.visits[0];
        assert_eq!(
            (v.number, v.date.as_str(), v.stage),
            (1, "2026-03-02", worn)
        );
        assert_eq!(v.lagging(), vec![behind]);
        // the scan placed on the plan is a blob that survives pruning
        let blob = v.scan.unwrap();
        assert!(next.blobs.contains(&blob));
        assert!(next.referenced_blobs().contains(&blob));
        // the dev shift of 0.9 mm along world x is found in the tooth's pose
        let achieved = &v.teeth[&behind];
        assert!(
            achieved.linear_mm > 0.7 && achieved.linear_mm < 1.1,
            "{achieved:?}"
        );
        // the previous state, the plan and the labels are untouched
        assert_eq!(stored.stages, data.stages);
        assert_eq!(stored.labels, data.labels);

        // a second visit is number 2 (the same scan)
        let (again, out2) = exec(ID, &next, params).unwrap();
        assert_eq!(out2["visit"], 2);
        assert_eq!(aligners_data(&again).unwrap().visits.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_visit_scan_keeps_its_units_and_returns_the_warnings() {
        // an arch-sized scan written in metres: it stays as it is and the
        // import says that its size fits another unit
        let tiny = TriMesh::new(
            (0..40)
                .map(|i| {
                    let a = i as f32 * 0.08;
                    glam::Vec3::new(0.03 * a.cos(), 0.03 * a.sin(), 0.001 * (i % 3) as f32)
                })
                .collect(),
            (0..38).map(|i| [i, i + 1, i + 2]).collect(),
        );
        let path =
            std::env::temp_dir().join(format!("dca-visit-{}-metres.stl", std::process::id()));
        dc_mesh::io::write(&path, &tiny, dc_mesh::MeshFormat::Stl).unwrap();
        let read = read_visit_scan(&path.display().to_string(), &JobCtx::detached());
        let _ = std::fs::remove_file(&path);
        let (mesh, warnings) = read.unwrap();
        let extent = mesh.aabb().unwrap().size().max_element();
        assert!(extent < 0.1, "the units were rescaled: {extent}");
        let codes: Vec<WarningCode> = warnings.iter().map(|w| w.code).collect();
        assert!(codes.contains(&WarningCode::UnitsSuspect), "{codes:?}");
        assert!(!codes.contains(&WarningCode::Rescaled), "{codes:?}");
    }

    #[test]
    fn a_visit_reports_the_warnings_of_its_scan() {
        let s = staged("lower");
        let path = write_visit_scan(&s, 1, &BTreeMap::new(), "warned");
        // the file repeats a face: the import drops it and says so
        let mut mesh = dc_mesh::io::read(&path).unwrap();
        mesh.tris.push(mesh.tris[0]);
        dc_mesh::io::write(&path, &mesh, dc_mesh::MeshFormat::Stl).unwrap();
        let result = exec(
            ID,
            &s,
            json!({ "scan": path.display().to_string(), "stage": 1 }),
        );
        let _ = std::fs::remove_file(&path);
        let (_, out) = result.unwrap();
        let codes: Vec<&str> = out["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes, ["bad_faces_removed"], "{out}");
        assert!(out["warnings"][0]["message"].as_str().unwrap().len() > 10);
        // a clean scan has none
        let path = write_visit_scan(&s, 1, &BTreeMap::new(), "clean");
        let result = exec(
            ID,
            &s,
            json!({ "scan": path.display().to_string(), "stage": 1 }),
        );
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap().1["warnings"], json!([]));
    }

    #[test]
    fn a_visit_on_track_and_segmented() {
        let s = staged("lower");
        let data = aligners_data(&s).unwrap();
        // the patient wears aligner 1: every tooth does what the plan says (no deviation)
        let path = write_visit_scan(&s, 1, &BTreeMap::new(), "ontrack");
        let result = exec(
            ID,
            &s,
            json!({ "scan": path.display().to_string(), "stage": 1 }),
        );
        let _ = std::fs::remove_file(&path);
        let (next, out) = result.unwrap();
        assert_eq!(out["decision"], "on_track", "{out}");
        assert_eq!(out["on_track"], out["teeth"]);
        assert!(out["speed"].as_object().unwrap().is_empty());
        assert_eq!(out["segmented"], true, "{}", out["segmentation_note"]);
        let v = &aligners_data(&next).unwrap().visits[0];
        assert_eq!(v.stage, 1);
        assert!(v.segmented);
        // the labels blob has one label per vertex, with the teeth of the arch
        let mesh = next.blobs.require(v.scan.as_ref().unwrap()).unwrap();
        let labels = mesh.vertex_labels.as_ref().unwrap();
        assert_eq!(labels.len(), mesh.vertex_count());
        let mut present: Vec<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
        present.sort_unstable();
        present.dedup();
        assert_eq!(present.len(), data.tooth_frames.len());
    }

    #[test]
    fn an_obj_visit_scan_gets_the_axes_and_the_cleanup_of_the_import() {
        // an arch-sized mesh in Z up, the frame of the case
        let z_up = TriMesh::new(
            (0..40)
                .map(|i| {
                    let a = i as f32 * 0.08;
                    glam::Vec3::new(30.0 * a.cos(), 30.0 * a.sin(), (i % 3) as f32)
                })
                .collect(),
            (0..38).map(|i| [i, i + 1, i + 2]).collect(),
        );
        // the same scan as an OBJ, which is Y up, with a repeated face
        let mut y_up = z_up.clone();
        for p in &mut y_up.positions {
            *p = glam::Vec3::new(p.x, p.z, -p.y);
        }
        y_up.tris.push(y_up.tris[0]);
        let path = std::env::temp_dir().join(format!("dca-visit-{}-axes.obj", std::process::id()));
        dc_mesh::io::write(&path, &y_up, dc_mesh::MeshFormat::Obj).unwrap();
        let read = read_visit_scan(&path.display().to_string(), &JobCtx::detached());
        let _ = std::fs::remove_file(&path);
        let (read, warnings) = read.unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.code == WarningCode::BadFacesRemoved)
        );
        assert_eq!(read.face_count(), z_up.face_count());
        for (got, want) in read.positions.iter().zip(&z_up.positions) {
            assert!((*got - *want).length() < 1e-3, "{got} {want}");
        }
    }

    #[test]
    fn an_obj_visit_gives_the_result_of_the_same_stl() {
        let s = staged("lower");
        let data = aligners_data(&s).unwrap();
        let worn = data.stages.len() as u32;
        let stl = write_visit_scan(&s, worn, &BTreeMap::new(), "axes-stl");
        let mut mesh = dc_mesh::io::read(&stl).unwrap();
        for p in &mut mesh.positions {
            *p = glam::Vec3::new(p.x, p.z, -p.y);
        }
        let obj = stl.with_file_name(format!("dca-visit-{}-result.obj", std::process::id()));
        dc_mesh::io::write(&obj, &mesh, dc_mesh::MeshFormat::Obj).unwrap();

        let visit = |path: &Path| {
            let params = json!({
                "scan": path.display().to_string(),
                "stage": worn,
                "segment": false,
            });
            exec(ID, &s, params)
        };
        let (from_stl, from_obj) = (visit(&stl), visit(&obj));
        let _ = std::fs::remove_file(&stl);
        let _ = std::fs::remove_file(&obj);
        let (_, out) = from_obj.unwrap();
        let (_, want) = from_stl.unwrap();
        let rms = |v: &Value| v["registration_rms_mm"].as_f64().unwrap();
        assert!((rms(&out) - rms(&want)).abs() < 1e-3, "{out}\n{want}");
        assert_eq!(out["decision"], want["decision"], "{out}");
    }

    #[test]
    fn with_both_arches_the_file_says_which_arch_the_visit_is() {
        let lower = staged("lower");
        let both = exec(
            "aligners.demo",
            &lower,
            json!({ "arch": "upper", "severity": "mild" }),
        )
        .unwrap()
        .0;
        let both = exec("aligners.teeth.frames", &both, Value::Null).unwrap().0;
        // a click gives only the file: its name tells the arch
        let path = write_visit_scan(&lower, 2, &BTreeMap::new(), "lower");
        let out = exec(
            ID,
            &both,
            json!({ "scan": path.display().to_string(), "stage": 2, "segment": false }),
        );
        let _ = std::fs::remove_file(&path);
        assert_eq!(out.unwrap().1["arch"], "lower");
        // a file that tells nothing is refused with what to do
        let path = std::env::temp_dir().join(format!("dca-visit-{}-x.stl", std::process::id()));
        let tiny = TriMesh::new(
            vec![
                glam::Vec3::ZERO,
                glam::Vec3::new(40.0, 0.0, 0.0),
                glam::Vec3::new(0.0, 40.0, 0.0),
                glam::Vec3::new(0.0, 0.0, 40.0),
            ],
            vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        );
        dc_mesh::io::write(&path, &tiny, dc_mesh::MeshFormat::Stl).unwrap();
        let err = exec(ID, &both, json!({ "scan": path.display().to_string() }));
        let _ = std::fs::remove_file(&path);
        let m = err.unwrap_err().to_string();
        assert!(m.contains("upper or the lower"), "{m}");
    }

    #[test]
    fn a_click_on_re_plan_without_a_visit_says_to_do_the_visit() {
        let m = exec("aligners.followup.tracking", &staged("lower"), Value::Null)
            .unwrap_err()
            .to_string();
        assert!(m.contains("run Control visit first"), "{m}");
    }

    #[test]
    fn the_replan_continues_the_numbering_from_the_visit() {
        let s = staged("lower");
        let data = aligners_data(&s).unwrap();
        let n = data.stages.len() as u32;
        assert!(n >= 3, "{n}");
        let fdis: Vec<Fdi> = data.tooth_frames.keys().copied().collect();
        // the patient wears aligner 2; one tooth is 0.9 mm behind
        let behind = fdis[5];
        let deviation =
            BTreeMap::from([(behind, DMat4::from_translation(DVec3::new(0.0, 0.9, 0.0)))]);
        let path = write_visit_scan(&s, 2, &deviation, "replan");
        let visited = exec(
            ID,
            &s,
            json!({ "scan": path.display().to_string(), "stage": 2, "segment": false }),
        );
        let _ = std::fs::remove_file(&path);
        let (visited, out) = visited.unwrap();
        assert_eq!(out["decision"], "refinement", "{out}");

        let tracking = "aligners.followup.tracking";
        let (next, rep) = exec(tracking, &visited, Value::Null).unwrap();
        assert_eq!(rep["from"]["visit"], 1);
        assert_eq!(rep["kept_aligners"], 2);
        assert_eq!(rep["first_new_aligner"], 3);
        let new = rep["new_aligners"].as_u64().unwrap() as u32;
        let after = aligners_data(&next).unwrap();
        assert_eq!(after.stages.len() as u32, 2 + new);
        assert_eq!(after.stages[2].index, 3);
        // the delivered aligners are as they were
        for k in 0..2 {
            assert_eq!(after.stages[k], data.stages[k], "aligner {}", k + 1);
        }
        // the series ends at the setup, from the achieved position
        let last = after.stages.last().unwrap();
        for (f, want) in &after.setup.poses {
            assert!(last.poses[f].approx_eq(want, 1e-9, 1e-9), "{f}");
        }
        let visit = &after.visits[0];
        let first = staging::q_from_pose(&after.stages[2].poses[&behind]);
        let achieved = staging::q_from_pose(&visit.teeth[&behind].achieved);
        let step = DVec3::new(
            first[0] - achieved[0],
            first[1] - achieved[1],
            first[2] - achieved[2],
        )
        .length();
        // the tooth that was behind moves at 0.75 of the 0.25 mm per aligner
        assert!(step <= 0.25 * 0.75 + 1e-9, "{step}");
        assert_eq!(rep["speed"][behind.to_string()], 0.75);
        // the visit is kept
        assert_eq!(after.visits, aligners_data(&visited).unwrap().visits);

        // without a scan: from the delivered aligner 1 of the plan
        let (from_plan, rep) = exec(tracking, &s, json!({ "stage": 1 })).unwrap();
        assert_eq!(rep["from"]["stage"], 1);
        let plan = aligners_data(&from_plan).unwrap();
        assert_eq!(plan.stages[0], data.stages[0]);
        assert_eq!(plan.stages.last().unwrap().index, plan.stages.len() as u32);
        // nothing changed in the plan: the same series continues, 1 + the rest
        assert_eq!(plan.stages.len() as u32, n);
    }

    #[test]
    fn the_replan_reports_what_it_needs() {
        let tracking = "aligners.followup.tracking";
        let bad =
            |s: &CaseState, p: Value| matches!(exec(tracking, s, p), Err(CommandError::Invalid(_)));
        assert!(bad(&CaseState::default(), Value::Null));
        let s = staged("lower");
        // no visit yet, no stage
        assert!(bad(&s, Value::Null));
        assert!(bad(&s, json!({ "visit": 1 })));
        assert!(bad(&s, json!({ "visit": 1, "stage": 1 })));
        assert!(bad(&s, json!({ "stage": 99 })));
        assert!(crate::tree::find(tracking).is_some());
        assert!(crate::commands::is_registered(tracking));
    }
}
