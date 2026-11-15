//! Commands of the aligner shown in the 3D view (the plugin's
//! `show_aligner`, `props.py:715`, `animation.set_aligner_preview`,
//! `animation.py:1188`, and `oa.aligner_exact`, `ops_view_anim.py:128`).
//!
//! * `aligners.view.aligner` shows or hides the aligner. Shown, it builds,
//!   for every arch with aligners, the preview of the stage asked for when
//!   the arch has no aligner of the view up to date at that stage: the
//!   construction of the print file (the shell of the wall on the model of
//!   the stage, the ramps, the trim, the occlusal shaping, as
//!   `aligners.output.export-all` makes it) at the detail of the view (the
//!   plugin's `preview_voxel` of the quality, kept within
//!   [`PREVIEW_WALL_SHARE`] of the wall), reduced within the plugin's
//!   `preview_tolerance`, with no stamp and no print check
//!   (`preview_params`, `build_shell_steps`, `_reduce`). Hidden, every
//!   aligner built for the view goes, as the plugin frees them.
//! * `aligners.view.aligner-exact` builds the aligner of a stage as the
//!   export writes it (the same provider and choice as `export-all`: the
//!   quality, the stamp and its code of the case) and shows it.
//!
//! Both arches are built together, each on its own thread. An aligner built
//! is kept as a base of its arch, with the key of what it was built from
//! ([`key`]): at most the plugin's `preview_max_bases` of the quality per
//! arch, the farthest from the new one going first (`_add_base`). A stage
//! without a base of its own shows the nearest base of its arch moved with
//! the teeth and the gingiva (`crate::aligner_scene`). A change of the
//! design, the teeth or the plan gives another key: the bases built before
//! are out of date, are no longer drawn, and go at the next build.
//!
//! Deviations from the plugin:
//! - the plugin builds the stage shown again by itself 0.6 s after a design
//!   change (`props._aligner_design_update`); here nothing is built but by
//!   the command (the menu law: only the toggle or the button builds), and
//!   the view says that the aligner shown is out of date;
//! - the plugin's exact aligner is its preview built at the stage shown
//!   (`exact=True`: a hole the preview voxel leaves is built again at the
//!   print voxel); here it is the print file itself.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::export::provider::Note;
use dca_algo::{schedule, teeth};
use dca_model::{AlignerBase, AlignerView, AlignersState, MODULE_ID, Quality};
use glam::DMat4;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::export::{self, ViewAligner, ViewBuild};
use super::messages;
use super::occlusion::aligners_data;

/// `aligners.view.aligner`.
pub(crate) const SHOW: &str = "aligners.view.aligner";
/// `aligners.view.aligner-exact`.
pub(crate) const EXACT: &str = "aligners.view.aligner-exact";

/// The detail of the preview is at most this share of the wall: a coarser
/// grid opens holes in the shell (the plugin's `PREVIEW_WALL_SHARE`,
/// `animation.py:32`).
pub const PREVIEW_WALL_SHARE: f64 = 0.45;

/// The rigid motion of every closed tooth of `arch` at stage `k` (case
/// coordinates, `x' = D x`; the identity for a tooth the plan does not move),
/// the one the aligners are built with (`ArchPrint::deltas`: about the
/// centre of resistance of the tooth).
pub(crate) fn stage_deltas(data: &AlignersState, arch: Arch, k: u32) -> BTreeMap<Fdi, DMat4> {
    let poses = super::shell::local_poses(&data.stages, arch, k);
    data.crowns
        .keys()
        .filter(|f| Arch::of(**f) == arch)
        .map(|&f| {
            let d = match (poses.get(&f), data.tooth_frames.get(&f)) {
                (Some(p), Some(fr)) => {
                    let rest = teeth::rest_pose(fr);
                    rest.compose(p).compose(&rest.inverse()).to_mat4()
                }
                _ => DMat4::IDENTITY,
            };
            (f, d)
        })
        .collect()
}

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: SHOW,
            label: "aligners-cmd-view-aligner",
            doc: "aligners-cmd-view-aligner-doc",
            params: ShowParams,
            run: run_show,
        },
        stage_command! {
            id: EXACT,
            label: "aligners-cmd-view-aligner-exact",
            doc: "aligners-cmd-view-aligner-exact-doc",
            params: ExactParams,
            run: run_exact,
        },
    ]
}

/// Parameters of `aligners.view.aligner`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ShowParams {
    /// Show the aligner (`true`) or hide it (`false`: the aligners built for
    /// the view go).
    pub show: bool,
    /// The stage whose aligner is built when the aligner is shown (`0` is
    /// the initial position; an arch past its last aligner takes its last
    /// one).
    pub stage: u32,
}

impl Default for ShowParams {
    fn default() -> Self {
        Self {
            show: true,
            stage: 0,
        }
    }
}

/// Parameters of `aligners.view.aligner-exact`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ExactParams {
    /// The stage (`0` is the initial position; an arch past its last aligner
    /// takes its last one).
    pub stage: u32,
    /// Quality of the print files (default: the quality of the case).
    pub quality: Option<Quality>,
}

/// The detail and the tolerance of the preview for the settings of the case
/// (the plugin's `preview_params`).
pub fn preview_params(data: &AlignersState) -> (f64, f64) {
    let q = data.settings.quality;
    let voxel = q
        .preview_voxel_mm()
        .min(PREVIEW_WALL_SHARE * data.settings.shell_thickness_mm);
    (voxel, q.preview_tolerance_mm())
}

/// The stage of every arch with aligners for the stage `stage` of the
/// timeline: its last aligner when `stage` is past it.
pub fn arch_stages(data: &AlignersState, stage: u32) -> Vec<(Arch, u32)> {
    [Arch::Upper, Arch::Lower]
        .into_iter()
        .filter_map(|a| {
            let n = schedule::active_stages(a, &data.stages);
            (n > 0).then(|| (a, stage.min(n)))
        })
        .collect()
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn note_json(n: &Note) -> Value {
    json!({
        "arch": n.arch.map(arch_name),
        "stage": n.stage,
        "key": n.key,
        "detail": n.detail,
    })
}

fn base_json(b: &AlignerBase) -> Value {
    json!({
        "arch": role_name(b.arch),
        "stage": b.stage,
        "exact": b.exact,
        "voxel_mm": b.voxel_mm,
        "tolerance_mm": b.tolerance_mm,
        "seconds": b.seconds,
    })
}

fn role_name(r: dc_case::ScanRole) -> &'static str {
    match r {
        dc_case::ScanRole::Upper => "upper",
        dc_case::ScanRole::Lower => "lower",
        dc_case::ScanRole::Bite => "bite",
    }
}

/// `next` with `data` holding the view `view`; the meshes of the bases that
/// went are dropped from its blobs.
fn with_view(mut next: CaseState, mut data: AlignersState, view: AlignerView) -> CaseState {
    let gone: Vec<dc_case::BlobRef> = data
        .aligner_view
        .bases
        .iter()
        .map(|b| b.mesh)
        .filter(|m| !view.bases.iter().any(|b| b.mesh == *m))
        .collect();
    data.aligner_view = view;
    next.set_module(MODULE_ID, ModuleState::new(data));
    if !gone.is_empty() {
        let used = next.referenced_blobs();
        let keep = next
            .blobs
            .iter()
            .map(|(r, _)| *r)
            .filter(|r| used.contains(r) || !gone.contains(r))
            .collect();
        next.blobs.retain_only(&keep);
    }
    next
}

fn run_show(state: &CaseState, p: ShowParams, ctx: &JobCtx) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if !p.show {
        let dropped = data.aligner_view.bases.len();
        let next = with_view(state.clone(), data, AlignerView::default());
        return Ok((next, json!({ "shown": false, "dropped": dropped })));
    }
    let (voxel_mm, tolerance_mm) = preview_params(&data);
    build(
        state,
        data,
        p.stage,
        ViewBuild::Preview {
            voxel_mm,
            tolerance_mm,
        },
        ctx,
    )
}

fn run_exact(
    state: &CaseState,
    p: ExactParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let quality = p.quality.unwrap_or(data.settings.quality);
    build(state, data, p.stage, ViewBuild::Exact { quality }, ctx)
}

/// Builds what the arches miss at `stage` as `how` says, keeps it as a base
/// of its arch and shows the aligner.
fn build(
    state: &CaseState,
    data: AlignersState,
    stage: u32,
    how: ViewBuild,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let arches = arch_stages(&data, stage);
    if arches.is_empty() {
        return Err(messages::invalid("aligners-err-no-aligners", &[]));
    }
    let key = dca_model::view_key(state, &data);
    let (exact, voxel_mm, tolerance_mm) = match how {
        ViewBuild::Preview {
            voxel_mm,
            tolerance_mm,
        } => (false, voxel_mm, tolerance_mm),
        ViewBuild::Exact { quality } => (true, quality.voxel_mm(), quality.tolerance_mm()),
    };
    // an aligner up to date at the stage serves: any for the preview, an
    // exact one of the same quality for the exact aligner
    let serves = |b: &AlignerBase| {
        !exact || (b.exact && b.voxel_mm == voxel_mm && b.tolerance_mm == tolerance_mm)
    };
    let view = &data.aligner_view;
    let wanted: Vec<(Arch, u32)> = arches
        .iter()
        .copied()
        .filter(|(a, k)| {
            !view
                .current(a.role(), &key)
                .iter()
                .any(|b| b.stage == *k && serves(b))
        })
        .collect();
    let (built, notes) = if wanted.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        export::view_aligners(state, &data, &wanted, how, ctx)?
    };
    ctx.check_cancelled()?;

    let cap = data.settings.quality.preview_max_bases().max(1);
    let mut next_view = AlignerView {
        shown: true,
        bases: view
            .bases
            .iter()
            .filter(|b| b.key == key)
            .cloned()
            .collect(),
    };
    let mut next = state.clone();
    let mut made = Vec::new();
    for a in built {
        let ViewAligner {
            arch,
            stage: k,
            mesh,
            seconds,
        } = a;
        let role = arch.role();
        let v = mesh.validate();
        made.push(json!({
            "arch": arch_name(arch),
            "stage": k,
            "exact": exact,
            "voxel_mm": voxel_mm,
            "tolerance_mm": tolerance_mm,
            "vertices": v.vertices,
            "faces": v.faces,
            "closed": v.boundary_edges == 0 && v.non_manifold_edges == 0,
            "seconds": seconds,
        }));
        // an older build of the same stage is replaced; past the most kept,
        // the farthest from the new stage goes (`_add_base`)
        next_view
            .bases
            .retain(|b| !(b.arch == role && b.stage == k));
        while next_view.bases.iter().filter(|b| b.arch == role).count() >= cap {
            let far = next_view
                .bases
                .iter()
                .filter(|b| b.arch == role)
                .map(|b| b.stage.abs_diff(k))
                .max()
                .unwrap_or(0);
            let Some(i) = next_view
                .bases
                .iter()
                .position(|b| b.arch == role && b.stage.abs_diff(k) == far)
            else {
                break;
            };
            next_view.bases.remove(i);
        }
        let mesh = next.blobs.insert(mesh);
        next_view.bases.push(AlignerBase {
            arch: role,
            stage: k,
            mesh,
            exact,
            voxel_mm,
            tolerance_mm,
            key: key.clone(),
            seconds,
        });
    }
    let kept: Vec<Value> = arches
        .iter()
        .filter(|(a, _)| !wanted.iter().any(|(w, _)| w == a))
        .map(|(a, k)| json!({ "arch": arch_name(*a), "stage": k }))
        .collect();
    let bases: Vec<Value> = next_view.bases.iter().map(base_json).collect();
    let next = with_view(next, data, next_view);
    let out = json!({
        "shown": true,
        "stage": stage,
        "exact": exact,
        "built": made,
        "kept": kept,
        "bases": bases,
        "notes": notes.iter().map(note_json).collect::<Vec<_>>(),
    });
    Ok((next, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_case::{BlobRef, ScanRole};
    use dca_model::Stage;

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn with(data: AlignersState) -> CaseState {
        let mut s = CaseState::default();
        s.set_module(MODULE_ID, ModuleState::new(data));
        s
    }

    /// The STL bytes the export writes for `mesh`.
    fn stl(mesh: &dc_mesh::TriMesh) -> Vec<u8> {
        let mut buf = Vec::new();
        dc_mesh::io::write_to(&mut buf, mesh, dc_mesh::io::MeshFormat::Stl).unwrap();
        buf
    }

    /// The aligner of the golden arches in the 3D view, end to end:
    /// - the preview of stage 2 of both arches, built together, closed, at
    ///   the plugin's preview detail of the case quality;
    /// - drawn at stage 2 as built; at the last stage of each arch moved
    ///   with the teeth (the same vertices, moved);
    /// - a second call at the same stage builds nothing;
    /// - the exact aligner of stage 2 is the file the export writes, byte for
    ///   byte (`aligners.output.report` with the shells, the same quality);
    /// - hidden, nothing is left.
    #[test]
    #[ignore = "the golden pipeline, the preview and the exact aligner of a stage and an export: run with --release --ignored --nocapture"]
    fn the_aligner_of_the_golden_arches_in_the_view() {
        let s = crate::commands::export::tests::golden_pipeline();
        let data = aligners_data(&s).unwrap();
        let last = arch_stages(&data, u32::MAX);
        eprintln!("aligners: {last:?}");
        let k = 2;
        let t = std::time::Instant::now();
        let (s1, out) = exec(SHOW, &s, json!({ "stage": k })).unwrap();
        eprintln!(
            "preview of stage {k}: {:.1} s\n{out:#}",
            t.elapsed().as_secs_f64()
        );
        let d1 = aligners_data(&s1).unwrap();
        let (voxel, tol) = preview_params(&d1);
        assert!(d1.aligner_view.shown);
        assert_eq!(d1.aligner_view.bases.len(), 2, "{out:#}");
        for b in &d1.aligner_view.bases {
            assert_eq!((b.stage, b.exact), (k, false));
            assert_eq!((b.voxel_mm, b.tolerance_mm), (voxel, tol));
            let v = s1.mesh(&b.mesh).unwrap().validate();
            assert!(v.faces > 1000 && v.boundary_edges == 0, "{v:?}");
            assert_eq!(v.non_manifold_edges, 0, "{v:?}");
        }
        // the stage it was built at shows it as built
        for arch in [Arch::Upper, Arch::Lower] {
            let base = d1
                .aligner_view
                .bases
                .iter()
                .find(|b| b.arch == arch.role())
                .unwrap();
            let built: Vec<glam::DVec3> = s1
                .mesh(&base.mesh)
                .unwrap()
                .positions
                .iter()
                .map(|p| p.as_dvec3())
                .collect();
            assert_eq!(crate::aligner_scene::drawn(&s1, arch, k).unwrap(), built);
            // the last stage: moved with the teeth from stage 2
            let (_, n) = *last.iter().find(|(a, _)| *a == arch).unwrap();
            let t = std::time::Instant::now();
            let moved = crate::aligner_scene::drawn(&s1, arch, n).unwrap();
            eprintln!(
                "{arch:?}: drawn at {n} (binding and moving): {:.2} s",
                t.elapsed().as_secs_f64()
            );
            assert_eq!(moved.len(), built.len());
            let shift: Vec<f64> = moved
                .iter()
                .zip(&built)
                .map(|(a, b)| (*a - *b).length())
                .collect();
            let max = shift.iter().copied().fold(0.0, f64::max);
            eprintln!("{arch:?}: largest move of a vertex from {k} to {n}: {max:.3} mm");
            assert!(max > 0.05, "the aligner did not follow the teeth");
        }
        // the same stage again: nothing to build
        let (_, again) = exec(SHOW, &s1, json!({ "stage": k })).unwrap();
        assert_eq!(again["built"], json!([]), "{again:#}");
        assert_eq!(again["kept"].as_array().unwrap().len(), 2);

        // the exact aligner of the stage is the file of the export
        let t = std::time::Instant::now();
        let (s2, out) = exec(EXACT, &s1, json!({ "stage": k, "quality": "draft" })).unwrap();
        eprintln!(
            "exact aligner of stage {k} (draft): {:.1} s\n{out:#}",
            t.elapsed().as_secs_f64()
        );
        let d2 = aligners_data(&s2).unwrap();
        assert_eq!(d2.aligner_view.bases.len(), 2);
        assert!(d2.aligner_view.bases.iter().all(|b| b.exact));
        let dir = std::env::temp_dir().join(format!("dca-view-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let t = std::time::Instant::now();
        exec(
            "aligners.output.report",
            &s2,
            json!({
                "dir": dir.display().to_string(),
                "shells": true,
                "quality": "draft",
                "workflow": "direct",
                "print_check": false,
                "report": false,
                "embed_renders": false,
            }),
        )
        .unwrap();
        eprintln!(
            "export (aligners, draft): {:.1} s",
            t.elapsed().as_secs_f64()
        );
        for b in &d2.aligner_view.bases {
            let arch = if b.arch == ScanRole::Upper {
                Arch::Upper
            } else {
                Arch::Lower
            };
            let name = dca_algo::export::package::aligner_name(arch, k, false);
            let file = std::fs::read(dir.join(&name)).unwrap();
            assert!(
                file == stl(s2.mesh(&b.mesh).unwrap()),
                "{name}: not the solid the export writes"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();

        // hidden: nothing left
        let (s3, _) = exec(SHOW, &s2, json!({ "show": false })).unwrap();
        assert!(aligners_data(&s3).unwrap().aligner_view.is_empty());
        assert!(crate::aligner_scene::drawn(&s3, Arch::Upper, k).is_none());
    }

    fn base(arch: ScanRole, stage: u32, key: &str) -> AlignerBase {
        AlignerBase {
            arch,
            stage,
            mesh: BlobRef::of_bytes(format!("{arch:?}{stage}").as_bytes()),
            exact: false,
            voxel_mm: 0.3,
            tolerance_mm: 0.04,
            key: key.to_owned(),
            seconds: 0.0,
        }
    }

    #[test]
    fn the_commands_are_registered_with_their_parameters() {
        for id in [SHOW, EXACT] {
            let spec = crate::commands::find(id).unwrap();
            let schema = spec.schema();
            assert!(schema["properties"]["stage"].is_object(), "{id}: {schema}");
        }
        let show = crate::commands::find(SHOW).unwrap().schema();
        assert!(show["properties"]["show"].is_object());
        let exact = crate::commands::find(EXACT).unwrap().schema();
        assert!(exact["properties"]["quality"].is_object());
    }

    #[test]
    fn the_preview_detail_is_the_plugins_within_a_share_of_the_wall() {
        let mut d = AlignersState::default();
        // Standard, wall 1.0 mm: 0.3 mm, 0.04 mm
        assert_eq!(preview_params(&d), (0.3, 0.04));
        d.settings.shell_thickness_mm = 0.5;
        let (v, t) = preview_params(&d);
        assert!((v - 0.225).abs() < 1e-12 && t == 0.04);
        d.settings.quality = Quality::High;
        d.settings.shell_thickness_mm = 1.0;
        assert_eq!(preview_params(&d), (0.2, 0.02));
        d.settings.quality = Quality::Light;
        assert_eq!(preview_params(&d), (0.4, 0.08));
    }

    #[test]
    fn the_stage_of_an_arch_stops_at_its_last_aligner() {
        let mut d = AlignersState::default();
        let f11 = dc_case::Fdi::new(11).unwrap();
        let f31 = dc_case::Fdi::new(31).unwrap();
        let moved = |n: u32| {
            dc_case::Pose::new(
                glam::DVec3::new(0.1 * f64::from(n), 0.0, 0.0),
                glam::DQuat::IDENTITY,
            )
        };
        // the upper arch moves up to aligner 5, the lower one up to 3
        for n in 1..=5 {
            let mut st = Stage::new(n);
            st.poses.insert(f11, moved(n));
            st.poses.insert(f31, moved(n.min(3)));
            d.stages.push(st);
        }
        assert_eq!(arch_stages(&d, 4), vec![(Arch::Upper, 4), (Arch::Lower, 3)]);
        assert_eq!(arch_stages(&d, 0), vec![(Arch::Upper, 0), (Arch::Lower, 0)]);
        assert!(arch_stages(&AlignersState::default(), 2).is_empty());
    }

    #[test]
    fn hiding_the_aligner_drops_what_was_built_for_the_view() {
        let mut d = AlignersState::default();
        d.aligner_view.shown = true;
        d.aligner_view.bases.push(base(ScanRole::Upper, 2, "k"));
        let mut s = with(d);
        let mesh = s.blobs.insert(dc_mesh::TriMesh::new(
            vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
            vec![[0, 1, 2]],
        ));
        let mut d = aligners_data(&s).unwrap();
        d.aligner_view.bases[0].mesh = mesh;
        s.set_module(MODULE_ID, ModuleState::new(d));
        let (next, out) = exec(SHOW, &s, json!({ "show": false })).unwrap();
        assert_eq!(out, json!({ "shown": false, "dropped": 1 }));
        let d = aligners_data(&next).unwrap();
        assert!(d.aligner_view.is_empty());
        assert!(!next.blobs.contains(&mesh));
        // the state before keeps it (undo)
        assert!(s.blobs.contains(&mesh));
    }

    #[test]
    fn a_case_without_aligners_has_no_aligner_to_show() {
        let err = exec(SHOW, &with(AlignersState::default()), json!({ "stage": 1 })).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid(m) if m.contains("aligner")),
            "{err:?}"
        );
        let err = exec(EXACT, &with(AlignersState::default()), Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(_)), "{err:?}");
    }
}
