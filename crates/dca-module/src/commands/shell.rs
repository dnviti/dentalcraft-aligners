//! Commands of the `shell` stage (`dca_algo::shell`): the print geometry of
//! every stage, built from the closed teeth (`AlignersState::crowns`), the
//! gingiva without holes (`AlignersState::gingiva`, moved with the teeth by
//! its harmonic weights) and the staging.
//!
//! * `aligners.production.shell.automatic` builds the aligner shell before
//!   trimming (the model with its crevices closed, offset by the clearance
//!   and by the clearance plus the wall thickness);
//! * `aligners.production.base-labels` builds the printable model (teeth,
//!   gingiva cast, flat base; the labels are not part of it yet);
//! * `aligners.production.blockout` (not a menu leaf) builds the model with
//!   every undercut relative to the insertion axis filled.
//!
//! The solids are generated on demand and are not stored in the case (a
//! solid of one stage is a few megabytes, a series hundreds): the commands
//! build the stages asked (all by default), check them and return their
//! report (triangles, closed and manifold, wall thickness measured, seconds).
//! The export builds the files with [`ArchPrint`] the same way.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi, Pose, ScanRole};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::shell::{self, Design, Outputs, Params, StageSolids};
use dca_algo::{frame, gingiva, teeth};
use dca_model::{AlignersState, MODULE_ID, Stage};
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.production.shell.automatic",
            label: "aligners-menu-production-shell-automatic",
            doc: "aligners-cmd-shell-automatic-doc",
            params: dca_algo::shell::Params,
            run: run_shell,
        },
        stage_command! {
            id: "aligners.production.base-labels",
            label: "aligners-menu-production-base-labels",
            doc: "aligners-cmd-shell-model-doc",
            params: dca_algo::shell::Params,
            run: run_model,
        },
        stage_command! {
            id: "aligners.production.blockout",
            label: "aligners-cmd-shell-blockout",
            doc: "aligners-cmd-shell-blockout-doc",
            params: dca_algo::shell::Params,
            run: run_blockout,
        },
    ]
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

fn failed(e: impl ToString) -> CommandError {
    CommandError::Failed(e.to_string())
}

/// Pose of every tooth of `arch` at aligner `k` relative to its rest pose:
/// the pose of the last stage up to `k` that mentions it.
pub(crate) fn local_poses(stages: &[Stage], arch: Arch, k: u32) -> BTreeMap<Fdi, Pose> {
    let mut ordered: Vec<&Stage> = stages.iter().filter(|s| s.index <= k).collect();
    ordered.sort_by_key(|s| s.index);
    let mut out = BTreeMap::new();
    for s in ordered {
        for (&f, &p) in &s.poses {
            if Arch::of(f) == arch {
                out.insert(f, p);
            }
        }
    }
    out
}

/// The last aligner of `arch` (`0`: no staging).
pub(crate) fn last_stage(stages: &[Stage], arch: Arch) -> u32 {
    stages
        .iter()
        .filter(|s| s.poses.keys().any(|f| Arch::of(*f) == arch))
        .map(|s| s.index)
        .max()
        .unwrap_or(0)
}

/// What the print geometry of one arch needs, prepared once for all its
/// stages: the closed teeth and the gingiva at rest (case coordinates), the
/// weights that move the gingiva, the rest poses and the arch frame.
pub(crate) struct ArchPrint {
    /// The arch.
    pub arch: Arch,
    to_arch: glam::DMat4,
    crowns: BTreeMap<Fdi, TriMesh>,
    rest: BTreeMap<Fdi, Pose>,
    gum: gingiva::GumSurface,
    prep: gingiva::DeformPrep,
    cover: Vec<bool>,
}

impl ArchPrint {
    /// The data of `arch`; `None` when the arch has no scan, no gingiva or
    /// no closed tooth.
    pub(crate) fn new(
        state: &CaseState,
        data: &AlignersState,
        arch: Arch,
    ) -> Result<Option<Self>, CommandError> {
        let role: ScanRole = arch.role();
        let (Some(scan), Some(gum_ref)) = (state.case.scan(role), data.gingiva.get(&role)) else {
            return Ok(None);
        };
        let mut crowns = BTreeMap::new();
        let mut rest = BTreeMap::new();
        for (&f, r) in data.crowns.iter().filter(|(f, _)| Arch::of(**f) == arch) {
            crowns.insert(f, state.blobs.require(r)?.as_ref().clone());
            if let Some(fr) = data.tooth_frames.get(&f) {
                rest.insert(f, teeth::rest_pose(fr));
            }
        }
        if crowns.is_empty() {
            return Ok(None);
        }
        let mesh = state.blobs.require(&scan.mesh)?;
        let case_vertices: Vec<_> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: &case_vertices,
                seeds: &data.seeds,
            },
            &frame::Params::default(),
        )
        .map_err(failed)?;
        let to_arch = frame::rigid_inverse(&framed.arch_frame.matrix());
        let gum = gingiva::GumSurface::from_mesh(state.blobs.require(gum_ref)?).map_err(failed)?;
        let fdis: Vec<Fdi> = crowns.keys().copied().collect();
        let weights = gingiva::gum_weights(&gum, &fdis, &gingiva::Params::default());
        let prep = gingiva::deform_prep(&weights);
        // the membrane under the crowns is covered by the teeth: no prisms there
        // (the interdental col between two crowns is gingiva of the model)
        let under: Vec<bool> = gum
            .membrane
            .iter()
            .zip(&gum.col)
            .map(|(m, c)| *m && !*c)
            .collect();
        let cover = gum
            .tris
            .iter()
            .map(|t| t.iter().all(|&i| under[i as usize]))
            .collect();
        Ok(Some(Self {
            arch,
            to_arch,
            crowns,
            rest,
            gum,
            prep,
            cover,
        }))
    }

    /// The rigid transform of every moved tooth at aligner `k` (case
    /// coordinates, `x' = D x`).
    pub(crate) fn deltas(&self, stages: &[Stage], k: u32) -> BTreeMap<Fdi, glam::DMat4> {
        local_poses(stages, self.arch, k)
            .into_iter()
            .filter_map(|(f, local)| {
                let rest = self.rest.get(&f)?;
                Some((f, rest.compose(&local).compose(&rest.inverse()).to_mat4()))
            })
            .collect()
    }

    /// The solids of aligner `k` (case coordinates).
    pub(crate) fn build(
        &self,
        stages: &[Stage],
        k: u32,
        params: &Params,
        design: &Design,
        outputs: Outputs,
    ) -> Result<StageSolids, CommandError> {
        let deltas = self.deltas(stages, k);
        let teeth = shell::stage_teeth(&self.crowns, &deltas);
        let moved = gingiva::deform(&self.gum.vertices, &self.prep, &deltas);
        let gum = TriMesh::new(
            moved.iter().map(|p| p.as_vec3()).collect(),
            self.gum.tris.clone(),
        );
        let input = shell::Input {
            to_arch: self.to_arch,
            teeth: &teeth,
            gingiva: &gum,
            cover: &self.cover,
        };
        shell::run(&input, params, design, outputs).map_err(|e| match e {
            shell::ShellError::Invalid(m) => CommandError::Invalid(m),
            other => failed(other),
        })
    }
}

/// The closed/manifold check and the wall thickness of a stage, as JSON.
fn stage_report(out: &StageSolids, design: &Design) -> Result<Value, CommandError> {
    let mut rep = Map::new();
    for (name, mesh) in [
        ("model", &out.model),
        ("blockout", &out.blockout),
        ("shell", &out.shell),
    ] {
        if let Some(m) = mesh {
            let v = m.validate();
            rep.insert(
                name.to_owned(),
                json!({
                    "faces": v.faces,
                    "vertices": v.vertices,
                    "watertight": v.is_watertight(),
                    "components": v.components,
                }),
            );
        }
    }
    if let (Some(s), Some(m)) = (&out.shell, &out.model) {
        let d = shell::wall_distances(s, m).map_err(failed)?;
        let split = design.clearance_mm + 0.5 * design.thickness_mm;
        let outer: Vec<f64> = d.iter().copied().filter(|x| *x >= split).collect();
        let (lo, hi) = outer
            .iter()
            .fold((f64::INFINITY, 0.0f64), |(a, b), x| (a.min(*x), b.max(*x)));
        rep.insert(
            "wall_mm".to_owned(),
            json!({ "target": design.clearance_mm + design.thickness_mm, "min": lo, "max": hi }),
        );
    }
    rep.insert(
        "report".to_owned(),
        serde_json::to_value(&out.report).map_err(failed)?,
    );
    Ok(Value::Object(rep))
}

fn run_print(
    state: &CaseState,
    params: Params,
    outputs: Outputs,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = state
        .module_data::<AlignersState>(MODULE_ID)
        .ok_or_else(|| CommandError::Invalid("the case has no Aligners data".to_owned()))?;
    let design = params.design(&data.settings);
    let explicit = params.arch != teeth::Which::All;
    let mut work = Vec::new();
    for arch in params.arch.arches() {
        match ArchPrint::new(state, data, arch)? {
            Some(p) => {
                let last = last_stage(&data.stages, arch);
                let ks: Vec<u32> = if params.stages.is_empty() {
                    (0..=last).collect()
                } else {
                    params
                        .stages
                        .iter()
                        .copied()
                        .filter(|k| *k <= last)
                        .collect()
                };
                work.push((p, ks));
            }
            None if explicit => {
                return Err(CommandError::Invalid(format!(
                    "the {} arch has no closed teeth or gingiva: run the reconstruction and the gingiva first",
                    arch_name(arch)
                )));
            }
            None => {}
        }
    }
    if work.is_empty() {
        return Err(CommandError::Invalid(
            "no arch has closed teeth and gingiva: run the reconstruction and the gingiva first"
                .to_owned(),
        ));
    }
    let total: usize = work.iter().map(|(_, ks)| ks.len()).sum();
    let mut done = 0usize;
    let mut out = Map::new();
    for (print, ks) in &work {
        let mut per_stage = Map::new();
        for &k in ks {
            ctx.check_cancelled()?;
            let solids = print.build(&data.stages, k, &params, &design, outputs)?;
            per_stage.insert(k.to_string(), stage_report(&solids, &design)?);
            done += 1;
            ctx.set_fraction(done as f32 / total.max(1) as f32);
        }
        out.insert(arch_name(print.arch).to_owned(), Value::Object(per_stage));
    }
    out.insert(
        "design".to_owned(),
        json!({ "thickness_mm": design.thickness_mm, "clearance_mm": design.clearance_mm }),
    );
    // nothing is stored: the case is returned as it was
    Ok((state.clone(), Value::Object(out)))
}

fn run_shell(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let outputs = Outputs {
        model: true,
        blockout: false,
        shell: true,
    };
    run_print(state, params, outputs, ctx)
}

fn run_model(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let outputs = Outputs {
        model: true,
        blockout: false,
        shell: false,
    };
    run_print(state, params, outputs, ctx)
}

fn run_blockout(
    state: &CaseState,
    params: Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    let outputs = Outputs {
        model: false,
        blockout: true,
        shell: false,
    };
    run_print(state, params, outputs, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_model::Stage;

    fn run(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_commands_are_registered_and_two_are_menu_leaves() {
        for id in [
            "aligners.production.shell.automatic",
            "aligners.production.base-labels",
        ] {
            assert!(crate::commands::is_registered(id));
            assert!(crate::tree::find(id).is_some(), "{id} is a menu leaf");
        }
        assert!(crate::commands::is_registered(
            "aligners.production.blockout"
        ));
        let schema = crate::commands::find("aligners.production.shell.automatic")
            .unwrap()
            .schema();
        assert!(schema["properties"]["thickness_mm"].is_object(), "{schema}");
    }

    #[test]
    fn without_closed_teeth_the_command_says_what_is_missing() {
        let state = CaseState::default();
        assert!(matches!(
            run("aligners.production.shell.automatic", &state, Value::Null),
            Err(CommandError::Invalid(_))
        ));
        let state = crate::commands::find("aligners.demo")
            .unwrap()
            .execute(&state, &json!({ "arch": "lower" }), &JobCtx::detached())
            .unwrap()
            .0;
        let err = run(
            "aligners.production.base-labels",
            &state,
            json!({ "arch": "lower" }),
        )
        .unwrap_err();
        assert!(
            matches!(err, CommandError::Invalid(ref m) if m.contains("reconstruction")),
            "{err:?}"
        );
        assert!(matches!(
            run(
                "aligners.production.blockout",
                &state,
                json!({ "model_voxel_mm": -1.0 })
            ),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn local_poses_take_the_last_stage_that_moves_a_tooth() {
        let f = Fdi::new(31).unwrap();
        let g = Fdi::new(11).unwrap();
        let mut s1 = Stage::new(1);
        s1.poses.insert(f, Pose::from_translation(glam::DVec3::X));
        s1.poses.insert(g, Pose::from_translation(glam::DVec3::Y));
        let mut s3 = Stage::new(3);
        s3.poses.insert(f, Pose::from_translation(glam::DVec3::Z));
        let stages = vec![s3, s1];
        assert!(local_poses(&stages, Arch::Lower, 0).is_empty());
        assert_eq!(local_poses(&stages, Arch::Lower, 2)[&f].t, glam::DVec3::X);
        assert_eq!(local_poses(&stages, Arch::Lower, 5)[&f].t, glam::DVec3::Z);
        assert!(!local_poses(&stages, Arch::Lower, 5).contains_key(&g));
        assert_eq!(last_stage(&stages, Arch::Lower), 3);
        assert_eq!(last_stage(&stages, Arch::Upper), 1);
    }

    /// The commands on a golden case with the closed teeth and the gingiva of
    /// the other stages (coarse voxels): every solid closed, the wall thick.
    #[test]
    #[ignore = "reconstruction, gingiva and two stages of print geometry: run with --release --ignored"]
    fn a_reconstructed_case_gives_closed_solids() {
        let mut state = super::super::reconstruct::golden_state("demo_lower_mild");
        for id in [
            "aligners.teeth.frames",
            "aligners.preparation.reconstruction.automatic",
            "aligners.preparation.gingiva",
        ] {
            if crate::commands::is_registered(id) {
                state = run(id, &state, Value::Null).unwrap().0;
            }
        }
        let mut data = state
            .module_data::<AlignersState>(MODULE_ID)
            .unwrap()
            .clone();
        let f = *data.crowns.keys().next().unwrap();
        let mut s1 = Stage::new(1);
        s1.poses
            .insert(f, Pose::from_translation(glam::DVec3::new(0.25, 0.0, 0.0)));
        data.stages = vec![s1];
        state.set_module(MODULE_ID, dc_case::ModuleState::new(data));
        let params = json!({ "model_voxel_mm": 0.2, "shell_voxel_mm": 0.2, "tolerance_mm": 0.02 });
        let (next, out) = run(
            "aligners.production.shell.automatic",
            &state,
            params.clone(),
        )
        .unwrap();
        assert_eq!(next.referenced_blobs(), state.referenced_blobs());
        for k in ["0", "1"] {
            let st = &out["lower"][k];
            assert_eq!(st["shell"]["watertight"], true, "{st}");
            assert_eq!(st["model"]["watertight"], true, "{st}");
            assert_eq!(st["model"]["components"], 1, "{st}");
            let wall = &st["wall_mm"];
            assert!((wall["max"].as_f64().unwrap() - 1.0).abs() < 0.1, "{wall}");
            assert!((wall["min"].as_f64().unwrap() - 1.0).abs() < 0.1, "{wall}");
        }
        let (_, out) = run("aligners.production.blockout", &state, params).unwrap();
        assert_eq!(out["lower"]["1"]["blockout"]["watertight"], true);
    }
}
