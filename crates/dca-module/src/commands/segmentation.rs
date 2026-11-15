//! Commands of the `segmentation` stage (`dca_algo::segmentation`).
//!
//! `aligners.preparation.segmentation.automatic` segments the scan of every
//! arch that has tooth seeds (or the one arch asked for): the scan is moved
//! to the arch frame its seeds define, simplified to about 30 000 faces,
//! partitioned by a random walk from the seeds, the labels are transferred
//! back to the full scan, cleaned and their boundaries refined at full
//! resolution. The per-vertex FDI labels (`0` = gingiva) are stored as the
//! arch's labels blob in the Aligners data (the scan mesh with vertex
//! labels, as `aligners.demo` stores its ground truth), replacing the
//! previous labels of that arch. The scans and the seeds are not modified.
//! The teeth it gave and its seconds are kept per arch in the Aligners data
//! (`AlignersState::segmentation_runs`), so the inspector shows them after
//! the case is reopened; they go with the labels they describe.
//!
//! What was computed from the old labels of a separated arch goes
//! ([`AlignersState::clear_label_dependents`]: 3D teeth, gingiva, measures,
//! diagnosis, setup, stages, ramps, the teeth flagged for a rebuild and the
//! doubts of the check), and its ids are in the output (`invalidated`), as
//! the automatic numbering reports what new seeds drop. The status bar shows
//! the sub-steps of every arch ([`segmentation::Phase`]); a stop between
//! them leaves the case as it was.
//!
//! The separation parameters not given come from the case settings
//! (`Settings::segmentation`), which `aligners.settings.segmentation`
//! changes; the menu leaf, `aligners.auto` and the check of the separated
//! teeth give none, so they use the settings.
//!
//! Like the plugin's `compute_frame`, an arch is refused (error naming it,
//! nothing changed, whichever arches were asked for) when its seeds are
//! fewer than 4 or are missing on a side or on the incisors: the arch frame
//! always comes from the seeds, never from the shape of the scan
//! ([`dca_algo::frame::run_from_seeds`]). The parameters must be inside the
//! ranges of the plugin's sliders ([`segmentation::Params::validate`]).

use std::time::Instant;

use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::segmentation::{self, Phase};
use dca_model::{AlignersState, CLEARED_IDS, MODULE_ID, SegmentationRun, SegmentationSettings};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::import::require_settled;

/// Id of the automatic separation of teeth and gum.
pub(crate) const SEGMENT: &str = "aligners.preparation.segmentation.automatic";
/// Id of the command that changes the separation parameters of the case.
pub(crate) const SEGMENT_SETTINGS: &str = "aligners.settings.segmentation";

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: SEGMENT,
            label: "aligners-menu-preparation-segmentation-automatic",
            doc: "aligners-cmd-segmentation-automatic-doc",
            params: AutomaticParams,
            run: run_automatic,
        },
        stage_command! {
            id: SEGMENT_SETTINGS,
            label: "aligners-cmd-segmentation-settings",
            doc: "aligners-cmd-segmentation-settings-doc",
            params: SettingsPatch,
            run: run_settings,
        },
    ]
}

/// Parameters of `aligners.preparation.segmentation.automatic`: a separation
/// parameter not given is the one of the case settings.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AutomaticParams {
    /// Arch to segment (`upper` or `lower`); all the arches with a scan and
    /// seeds when absent.
    arch: Option<Arch>,
    /// Faces of the simplified scan of the first separation (5000-200000);
    /// default: the case settings.
    target_faces: Option<usize>,
    /// Weight of the convex edges in the fold measure (0.01-1); default: the
    /// case settings.
    eta_convex: Option<f64>,
    /// Sharpness of the fold weights (0.1-10); default: the case settings.
    sigma: Option<f64>,
    /// Refine the boundaries at full resolution (default true).
    refine: Option<bool>,
}

impl AutomaticParams {
    /// The parameters of the segmentation: the ones given, the settings for
    /// the others.
    fn resolve(&self, settings: &SegmentationSettings) -> segmentation::Params {
        segmentation::Params {
            arch: self.arch,
            target_faces: self.target_faces.unwrap_or(settings.target_faces),
            eta_convex: self.eta_convex.unwrap_or(settings.eta_convex),
            sigma: self.sigma.unwrap_or(settings.sigma),
            refine: self.refine.unwrap_or(true),
        }
    }
}

/// Parameters of `aligners.settings.segmentation`: the fields to change.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsPatch {
    /// Faces of the simplified scan (5000-200000).
    target_faces: Option<usize>,
    /// Weight of the convex edges (0.01-1).
    eta_convex: Option<f64>,
    /// Sharpness of the fold weights (0.1-10).
    sigma: Option<f64>,
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The Aligners data of the case (new ones when there are none).
fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

/// `ids` (of [`CLEARED_IDS`]) once each, in the order of [`CLEARED_IDS`].
pub(crate) fn in_cleared_order<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let ids: Vec<&str> = ids.into_iter().collect();
    CLEARED_IDS
        .iter()
        .copied()
        .filter(|c| ids.contains(c))
        .collect()
}

/// Fluent key of the sub-step `phase` in the status bar.
fn phase_key(phase: Phase) -> &'static str {
    match phase {
        Phase::Simplify => "aligners-seg-step-simplify",
        Phase::FirstSeparation => "aligners-seg-step-first",
        Phase::Transfer => "aligners-seg-step-transfer",
        Phase::Borders => "aligners-seg-step-borders",
        Phase::Cleanup => "aligners-seg-step-cleanup",
    }
}

fn run_settings(
    state: &CaseState,
    patch: SettingsPatch,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = aligners_data(state)?;
    let s = &mut data.settings.segmentation;
    let new = SegmentationSettings {
        target_faces: patch.target_faces.unwrap_or(s.target_faces),
        eta_convex: patch.eta_convex.unwrap_or(s.eta_convex),
        sigma: patch.sigma.unwrap_or(s.sigma),
    };
    // the sliders' ranges: a value outside them changes nothing
    AutomaticParams::default()
        .resolve(&new)
        .validate()
        .map_err(|e| crate::commands::messages::segmentation_params_error(&e))?;
    *s = new;
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((
        next,
        json!({
            "target_faces": new.target_faces,
            "eta_convex": new.eta_convex,
            "sigma": new.sigma,
        }),
    ))
}

fn run_automatic(
    state: &CaseState,
    params: AutomaticParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let t_all = Instant::now();
    let mut data = aligners_data(state)?;
    let params = params.resolve(&data.settings.segmentation);
    // the sliders' ranges, before anything else
    params
        .validate()
        .map_err(|e| crate::commands::messages::segmentation_params_error(&e))?;
    let has_seeds = |arch: Arch| data.seeds.iter().any(|s| Arch::of(s.fdi) == arch);
    let arches: Vec<Arch> = match params.arch {
        Some(arch) => {
            if state.case.scan(arch.role()).is_none() {
                return Err(crate::commands::messages::no_scan(arch));
            }
            if !has_seeds(arch) {
                return Err(crate::commands::messages::no_seeds(arch));
            }
            vec![arch]
        }
        None => [Arch::Upper, Arch::Lower]
            .into_iter()
            .filter(|&a| state.case.scan(a.role()).is_some() && has_seeds(a))
            .collect(),
    };
    if arches.is_empty() {
        return Err(crate::commands::messages::none_seeded());
    }

    // the gate of every later stage: nothing is computed on a scan whose arch
    // is still a guess
    for scan in arches.iter().filter_map(|a| state.case.scan(a.role())) {
        require_settled(scan)?;
    }

    // the arch frame of every arch comes from its seeds, as in the plugin
    // (`case_core.compute_frame`): an arch with fewer than 4 seeds, or none
    // on a side or on the incisors, is refused before anything is computed
    // (the frame of a case also falls back to the shape of the scan, but a
    // separation placed by a guess would give the user wrong teeth)
    let mut prepared = Vec::with_capacity(arches.len());
    for &arch in &arches {
        let scan = state
            .case
            .scan(arch.role())
            .ok_or_else(|| crate::commands::messages::scan_missing(arch))?;
        // the scan as it was read: its open border is gingiva (the border
        // constraint of the separation)
        let mesh = crate::open_scan::open_scan(state, &data, &scan.mesh)?;
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let framed = frame::run_from_seeds(
            &frame::Input {
                arch,
                vertices: &case_vertices,
                seeds: &data.seeds,
                occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
            },
            &frame::Params::default(),
        )
        .map_err(|e| crate::commands::messages::frame_error(arch, &e))?;
        prepared.push((arch, framed, case_vertices, mesh));
    }

    let fraction = super::auto::step_fraction(ctx);
    let n_arches = prepared.len() as f64;
    let n_phases = Phase::ALL.len() as f64;
    let mut next = state.clone();
    let mut invalidated: Vec<&'static str> = Vec::new();
    let mut teeth = 0;
    let mut report = Map::new();
    for (k, (arch, framed, case_vertices, mesh)) in prepared.into_iter().enumerate() {
        ctx.check_cancelled()?;
        let t_arch = Instant::now();
        let role = arch.role();
        let scan = state
            .case
            .scan(role)
            .ok_or_else(|| crate::commands::messages::scan_missing(arch))?;
        let closed = state.blobs.require(&scan.mesh)?;
        // the arch frame its seeds define (case_teeth.segment_iter works in arch coordinates)
        let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
        let vertices: Vec<_> = case_vertices
            .iter()
            .map(|&p| to_arch.transform_point3(p))
            .collect();
        let mut seeds: Vec<(Fdi, _)> = data
            .seeds
            .iter()
            .filter(|s| Arch::of(s.fdi) == arch)
            .map(|s| (s.fdi, to_arch.transform_point3(s.position)))
            .collect();
        seeds.sort_by_key(|s| s.0);
        // the status bar names every sub-step; a stop is seen between them
        let mut progress = |phase: Phase| {
            let x = (k as f64 + phase.index() as f64 / n_phases) / n_arches;
            ctx.set_progress(fraction(x), Some(phase_key(phase)));
            !ctx.is_cancelled()
        };
        let out = segmentation::run_with_progress(
            &segmentation::Input {
                arch,
                vertices: &vertices,
                tris: &mesh.tris,
                seeds: &seeds,
                decimated: None,
            },
            &params,
            &mut progress,
        )
        .map_err(|e| crate::commands::messages::segmentation_error(arch, &e))?;
        ctx.check_cancelled()?;

        // what the old labels gave goes (3D teeth, measures, plan, doubts)
        invalidated.extend(data.clear_label_dependents(role));
        // the labels blob: the scan of the case with one FDI per vertex, the
        // base of a scan closed with one gingiva
        let mut labelled = (**closed).clone();
        let mut labels = out.labels.clone();
        labels.resize(closed.vertex_count(), 0);
        labelled.vertex_labels = Some(labels);
        labelled.face_labels = None;
        let labels_ref = next.blobs.insert(labelled);
        data.labels.insert(role, labels_ref);
        let seconds = t_arch.elapsed().as_secs_f64();
        data.segmentation_runs.insert(
            role,
            SegmentationRun {
                teeth: u32::try_from(out.teeth).unwrap_or(u32::MAX),
                seconds,
            },
        );
        let mut per_tooth = Map::new();
        for &l in &out.labels {
            let key = l.to_string();
            let n = per_tooth.get(&key).and_then(Value::as_u64).unwrap_or(0);
            per_tooth.insert(key, json!(n + 1));
        }
        teeth += out.teeth;
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "frame_source": framed.source,
                "vertices": out.labels.len(),
                "simplified_faces": out.decimated_tris.len(),
                "teeth": out.teeth,
                "seconds": seconds,
                "vertices_per_label": per_tooth,
                "labels_blob": labels_ref.to_hex(),
                "diagnostics": out.diagnostics,
            }),
        );
    }
    ctx.set_fraction(fraction(1.0));
    // the replaced labels and the dropped solids and gingiva go, unless
    // something else still uses them
    let before = state.referenced_blobs();
    next.set_module(MODULE_ID, ModuleState::new(data));
    let after = next.referenced_blobs();
    let stale: Vec<_> = before.difference(&after).copied().collect();
    crate::manual::common::finish_blobs(&mut next, &stale);
    report.insert("teeth".to_owned(), json!(teeth));
    report.insert("seconds".to_owned(), json!(t_all.elapsed().as_secs_f64()));
    report.insert(
        "invalidated".to_owned(),
        json!(in_cleared_order(invalidated)),
    );
    Ok((next, Value::Object(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::ScanRole;

    const ID: &str = SEGMENT;

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        run_with(state, params, &JobCtx::detached())
    }

    fn run_with(
        state: &CaseState,
        params: Value,
        ctx: &JobCtx,
    ) -> Result<CommandOutput, CommandError> {
        crate::commands::find(ID)
            .unwrap()
            .execute(state, &params, ctx)
    }

    fn settings(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(SEGMENT_SETTINGS).unwrap().execute(
            state,
            &params,
            &JobCtx::detached(),
        )
    }

    /// The output without the times, which change from run to run.
    fn untimed(mut out: Value) -> Value {
        if let Value::Object(m) = &mut out {
            m.remove("seconds");
            for v in m.values_mut() {
                if let Value::Object(a) = v {
                    a.remove("seconds");
                }
            }
        }
        out
    }

    /// `state` with the 3D teeth and the gingiva of its lower arch built,
    /// a tooth flagged for a rebuild and a doubt of the check.
    fn built(state: &CaseState) -> CaseState {
        use glam::Vec3;
        let mut s = state.clone();
        let mut d = s.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        let tri =
            |z: f32| dc_mesh::TriMesh::new(vec![Vec3::Z * z, Vec3::X, Vec3::Y], vec![[0, 1, 2]]);
        let crown = s.blobs.insert(tri(0.0));
        let gum = s.blobs.insert(tri(1.0));
        for seed in d.seeds.clone() {
            d.crowns.insert(seed.fdi, crown);
        }
        d.gingiva.insert(ScanRole::Lower, gum);
        d.dirty.insert(d.seeds[0].fdi);
        d.segmentation_doubts
            .insert(ScanRole::Lower, vec!["41 is 3.0 mm wide".to_owned()]);
        s.set_module(MODULE_ID, ModuleState::new(d));
        s
    }

    #[test]
    fn a_new_separation_drops_the_3d_teeth_and_says_what_went() {
        let s = built(&demo_state("lower"));
        assert!(crate::done::leaf_done(&s, "aligners.teeth.build"));
        let (next, out) = run(&s, Value::Null).unwrap();
        assert_eq!(out["invalidated"], json!(["crowns", "gingiva"]));
        assert_eq!(out["teeth"], 14);
        assert_eq!(out["lower"]["teeth"], 14);
        let total = out["seconds"].as_f64().unwrap();
        let lower = out["lower"]["seconds"].as_f64().unwrap();
        assert!(lower > 0.0 && total >= lower, "{total} {lower}");
        // the tick of the 3D teeth goes, the one of the separation stays
        assert!(!crate::done::leaf_done(&next, "aligners.teeth.build"));
        assert!(crate::done::leaf_done(&next, ID));
        let d = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(d.crowns.is_empty() && d.gingiva.is_empty() && d.dirty.is_empty());
        assert!(d.segmentation_doubts.is_empty());
        // the case keeps the teeth and the time of the run, as the output says them
        let stored = d.segmentation_runs[&ScanRole::Lower];
        assert_eq!(stored.teeth, 14);
        assert_eq!(stored.seconds, lower);
        assert_eq!(d.segmentation_runs.len(), 1);
        // their blobs go with them: the scan and the new labels are left
        assert_eq!(next.blobs.len(), 2);
        // nothing more to drop on a second run
        let (_, again) = run(&next, Value::Null).unwrap();
        assert_eq!(again["invalidated"], json!([]));
    }

    #[test]
    fn the_settings_are_stored_checked_and_used_when_a_parameter_is_not_given() {
        let lower = demo_state("lower");
        let (set, out) = settings(&lower, json!({ "target_faces": 5000 })).unwrap();
        assert_eq!(
            out,
            json!({ "target_faces": 5000, "eta_convex": 0.2, "sigma": 1.0 })
        );
        let d = set.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(d.settings.segmentation.target_faces, 5000);
        // they are saved with the case
        let back: AlignersState = serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(back.settings.segmentation, d.settings.segmentation);
        // a value outside its range: refused, nothing changes
        for bad in [
            json!({ "sigma": 0.0 }),
            json!({ "eta_convex": 2.0 }),
            json!({ "target_faces": 1, "sigma": 2.0 }),
        ] {
            assert!(matches!(
                settings(&set, bad.clone()),
                Err(CommandError::Invalid(m)) if m.contains("between")
            ));
        }
        assert!(settings(&set, json!({ "nope": 1 })).is_err());
        // a field not given stays
        let (set2, out) = settings(&set, json!({ "sigma": 2.0 })).unwrap();
        assert_eq!(out["target_faces"], 5000);
        assert_eq!(
            set2.module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .settings
                .segmentation
                .sigma,
            2.0
        );

        // no parameter: the settings; a parameter given wins
        let faces = |o: &Value| o["lower"]["simplified_faces"].as_u64().unwrap();
        let (_, from_settings) = run(&set, Value::Null).unwrap();
        let (_, given) = run(&lower, json!({ "target_faces": 5000 })).unwrap();
        let (_, overridden) = run(&set, json!({ "target_faces": 30000 })).unwrap();
        assert_eq!(untimed(from_settings.clone()), untimed(given));
        assert!(
            faces(&from_settings) < faces(&overridden),
            "{} {}",
            faces(&from_settings),
            faces(&overridden)
        );
    }

    #[test]
    fn a_stop_between_the_sub_steps_leaves_the_case_as_it_was() {
        let lower = demo_state("lower");
        let ctx = JobCtx::detached();
        let watcher = {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                // stop as soon as the first sub-step is shown
                let start = std::time::Instant::now();
                while start.elapsed() < std::time::Duration::from_secs(120) {
                    let key = ctx.progress().message_key;
                    if key.as_deref() == Some("aligners-seg-step-simplify") {
                        ctx.cancel_token().cancel();
                        return key;
                    }
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                None
            })
        };
        let r = run_with(&lower, Value::Null, &ctx);
        let seen = watcher.join().unwrap();
        assert_eq!(seen.as_deref(), Some("aligners-seg-step-simplify"));
        assert!(matches!(r, Err(CommandError::Cancelled)), "{r:?}");
        // every sub-step has its words in both languages
        for p in Phase::ALL {
            let key = phase_key(p);
            let text = crate::commands::messages::tr(key, &[]);
            assert!(!text.is_empty() && text != key, "{key}");
        }
    }

    fn demo_state(arch: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                &CaseState::default(),
                &json!({ "arch": arch, "severity": "mild" }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn the_command_is_the_automatic_segmentation_leaf() {
        let spec = crate::commands::find(ID).unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["arch"].is_object());
        assert!(crate::tree::find(ID).is_some());
    }

    #[test]
    fn missing_scans_or_seeds_are_reported() {
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(m)) if m.contains("tooth marks")
        ));
        let lower = demo_state("lower");
        assert!(matches!(
            run(&lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper scan")
        ));
        let mut no_seeds = lower.clone();
        let mut data = no_seeds
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        data.seeds.clear();
        no_seeds.set_module(MODULE_ID, ModuleState::new(data));
        assert!(matches!(
            run(&no_seeds, json!({ "arch": "lower" })),
            Err(CommandError::Invalid(m)) if m.contains("tooth marks")
        ));
        assert!(matches!(
            run(&lower, json!({ "arch": "sideways" })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn a_provisional_scan_is_refused_whatever_arch_is_asked() {
        let mut state = demo_state("lower");
        state.case.scans[0].role_source = dc_case::RoleSource::Provisional;
        for params in [json!({ "arch": "lower" }), Value::Null] {
            let Err(CommandError::Invalid(m)) = run(&state, params) else {
                panic!("a provisional scan was segmented");
            };
            assert!(m.contains("Import scans panel"), "{m}");
        }
    }

    #[test]
    fn the_demo_arch_is_segmented_like_its_ground_truth() {
        let demo = demo_state("lower");
        let data = demo.module_data::<AlignersState>(MODULE_ID).unwrap();
        let truth_ref = data.labels[&ScanRole::Lower];
        let truth = demo
            .mesh(&truth_ref)
            .unwrap()
            .vertex_labels
            .clone()
            .unwrap();
        // previous labels of the arch (all gingiva): replaced and dropped
        let mut state = demo.clone();
        let mut data = data.clone();
        let mut old = (**demo.mesh(&truth_ref).unwrap()).clone();
        old.vertex_labels = Some(vec![0; truth.len()]);
        let old_ref = state.blobs.insert(old);
        data.labels.insert(ScanRole::Lower, old_ref);
        state.set_module(MODULE_ID, ModuleState::new(data));
        state.blobs.retain_only(&state.referenced_blobs());

        let (next, out) = run(&state, Value::Null).unwrap();
        let report = &out["lower"];
        assert_eq!(report["teeth"], 14);
        assert!(out.get("upper").is_none());
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        let labels_ref = data.labels[&ScanRole::Lower];
        assert_eq!(report["labels_blob"], labels_ref.to_hex());
        let labelled = next.mesh(&labels_ref).unwrap();
        let scan = next.case.scan(ScanRole::Lower).unwrap();
        let mesh = next.mesh(&scan.mesh).unwrap();
        assert_eq!(labelled.positions, mesh.positions);
        assert_eq!(labelled.tris, mesh.tris);
        let labels = labelled.vertex_labels.as_ref().unwrap();
        let same = labels.iter().zip(&truth).filter(|(a, b)| a == b).count();
        let agreement = same as f64 / truth.len() as f64;
        assert!(
            agreement > 0.98,
            "agreement with the ground truth {agreement}"
        );
        // the scan and the seeds are untouched, the old labels are gone
        assert_eq!(next.case.scans, state.case.scans);
        assert_eq!(
            data.seeds,
            state.module_data::<AlignersState>(MODULE_ID).unwrap().seeds
        );
        assert!(next.blobs.get(&old_ref).is_none());
        assert_eq!(next.blobs.len(), 2);

        // deterministic: a second run stores the same labels
        let (again, again_out) = run(&next, json!({ "arch": "lower" })).unwrap();
        assert_eq!(untimed(again_out), untimed(out));
        assert_eq!(
            again
                .module_data::<AlignersState>(MODULE_ID)
                .unwrap()
                .labels,
            data.labels
        );
        assert_eq!(again.blobs.len(), 2);
    }

    /// `state` with only the seeds `keep` accepts.
    fn with_seeds(state: &CaseState, keep: impl Fn(u8) -> bool) -> CaseState {
        let mut out = state.clone();
        let mut data = out.module_data::<AlignersState>(MODULE_ID).unwrap().clone();
        data.seeds.retain(|s| keep(s.fdi.get()));
        out.set_module(MODULE_ID, ModuleState::new(data));
        out
    }

    fn refusal(state: &CaseState, params: Value) -> String {
        match run(state, params) {
            Err(CommandError::Invalid(m)) => m,
            other => panic!("not refused: {:?}", other.map(|(_, v)| v)),
        }
    }

    #[test]
    fn an_arch_with_fewer_than_four_marks_is_refused_naming_it() {
        let lower = demo_state("lower");
        let all: Vec<u8> = lower
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .seeds
            .iter()
            .map(|s| s.fdi.get())
            .collect();
        let three = with_seeds(&lower, |n| all[..3].contains(&n));
        for params in [json!({ "arch": "lower" }), Value::Null] {
            let m = refusal(&three, params);
            assert!(
                m.contains("lower arch") && m.contains("fewer than 4"),
                "{m}"
            );
        }
        // the refusal says how to get the marks
        let m = refusal(&three, Value::Null);
        assert!(
            m.contains(&crate::commands::messages::tr(
                "aligners-menu-teeth-find",
                &[]
            )),
            "{m}"
        );
    }

    #[test]
    fn marks_on_one_side_or_without_incisors_are_refused() {
        let lower = demo_state("lower");
        // the right side only (4x), the left side only (3x)
        for side in [4u8, 3] {
            let one = with_seeds(&lower, |n| n / 10 == side);
            let m = refusal(&one, json!({ "arch": "lower" }));
            assert!(
                m.contains("lower arch") && m.contains("both sides and on the incisors"),
                "{m}"
            );
        }
        // no incisor and no canine: units 4 to 8 only
        let none = with_seeds(&lower, |n| n % 10 >= 4);
        let m = refusal(&none, json!({ "arch": "lower" }));
        assert!(m.contains("both sides and on the incisors"), "{m}");
    }

    #[test]
    fn a_refused_arch_refuses_the_whole_run_and_names_it() {
        use super::super::occlusion::tests::two_demo_arches;
        let both = two_demo_arches();
        // the upper arch keeps 3 marks: the lower one is not segmented either
        let upper: Vec<u8> = both
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .seeds
            .iter()
            .map(|s| s.fdi.get())
            .filter(|n| n / 10 <= 2)
            .collect();
        let upper_bad = with_seeds(&both, |n| n / 10 > 2 || upper[..3].contains(&n));
        let m = refusal(&upper_bad, Value::Null);
        assert!(
            m.contains("upper arch") && m.contains("fewer than 4"),
            "{m}"
        );
        // asking for the good arch alone still works, and the other is untouched
        let (next, out) = run(&upper_bad, json!({ "arch": "lower" })).unwrap();
        assert!(out.get("lower").is_some() && out.get("upper").is_none());
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert!(data.labels.contains_key(&ScanRole::Lower));
    }

    #[test]
    fn parameters_outside_their_range_are_refused_before_any_work() {
        let lower = demo_state("lower");
        let cases = [
            (json!({ "target_faces": 4999 }), "target_faces"),
            (json!({ "target_faces": 200001 }), "target_faces"),
            (json!({ "eta_convex": 0.0 }), "eta_convex"),
            (json!({ "eta_convex": 1.5 }), "eta_convex"),
            (json!({ "sigma": 0.05 }), "sigma"),
            (json!({ "sigma": 11.0 }), "sigma"),
        ];
        for (params, name) in cases {
            // on a good case and on an empty one: the range comes first
            for state in [&lower, &CaseState::default()] {
                let m = refusal(state, params.clone());
                assert!(m.contains(name) && m.contains("between"), "{m}");
            }
        }
    }
}
