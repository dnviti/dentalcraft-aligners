//! Commands of the `stamp` stage (`dca_algo::stamp`).
//!
//! `aligners.production.stamp` puts the identification text (arch, aligner
//! number, date, case code) on the scan of an arch: embossed (`kind` model,
//! the default) or engraved (`kind` shell). Until the shell and the model of
//! a stage exist, the target is the scan itself, which has to be a closed
//! solid (the parts of the demo arch overlap and are merged). The text goes
//! on a molar or premolar of the segmented arch (tooth frames from the teeth
//! stage when there are any), or at a point given by hand in case
//! coordinates (`placement` `{"mode": "manual", ...}`), which needs no
//! segmentation. The case is not modified: the stamped meshes are returned
//! as statistics and, with `dir`, written there as binary STL in case
//! coordinates (`<arch>_stamped.stl`).

use std::path::Path;
use std::time::Instant;

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_mesh::TriMesh;
use dc_mesh::io::MeshFormat;
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::stamp::{self, Outcome, Placement, ToothSite};
use dca_algo::teeth::Which;
use dca_model::{AlignersState, MODULE_ID};
use glam::{DMat4, DVec3};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.production.stamp",
        label: "aligners-cmd-stamp",
        doc: "aligners-cmd-stamp-doc",
        params: StampParams,
        run: run_stamp,
    }]
}

/// Parameters of `aligners.production.stamp`: the arches and the output, then
/// the parameters of the stamp (`dca_algo::stamp::Params`) in the same object;
/// those not given come from the stamp settings of the case
/// ([`case_stamp_params`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct StampParams {
    arch: Which,
    dir: Option<String>,
    /// The keys of `stamp::Params` given (checked when read).
    stamp: Map<String, Value>,
}

/// What the schema of [`StampParams`] shows.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct StampSchema {
    /// Arches to stamp.
    arch: Which,
    /// Directory where the stamped models are written as binary STL
    /// (`upper_stamped.stl`, `lower_stamped.stl`, in case coordinates); not
    /// written when missing.
    dir: Option<String>,
    #[serde(flatten)]
    stamp: stamp::Params,
}

impl JsonSchema for StampParams {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "StampParams".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        StampSchema::json_schema(generator)
    }
}

/// Reads `arch` and `dir`, and the rest as `stamp::Params`, which refuses
/// the keys it does not know (a flattened struct cannot).
impl<'de> Deserialize<'de> for StampParams {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut map = Map::<String, Value>::deserialize(d)?;
        let arch = map
            .remove("arch")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or_default();
        let dir = map
            .remove("dir")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or_default();
        serde_json::from_value::<stamp::Params>(Value::Object(map.clone()))
            .map_err(D::Error::custom)?;
        Ok(Self {
            arch,
            dir,
            stamp: map,
        })
    }
}

/// The stamp parameters of the case (`Settings::stamp`, the plugin's
/// `stamp_*`: tooth, face, reading side, date format, code, sizes), with the
/// keys of `given` on top.
///
/// # Errors
/// A value of `given` that `stamp::Params` refuses.
pub(crate) fn case_stamp_params(
    s: &dca_model::StampSettings,
    given: &Map<String, Value>,
) -> Result<stamp::Params, CommandError> {
    let invalid = |e: serde_json::Error| CommandError::Invalid(e.to_string());
    let mut v = serde_json::to_value(stamp::Params::default()).map_err(invalid)?;
    if let Value::Object(o) = &mut v {
        o.insert(
            "placement".into(),
            json!({ "mode": "tooth", "tooth": s.tooth, "face": s.face, "read": s.read }),
        );
        o.insert("date_format".into(), json!(s.date_format));
        o.insert("code".into(), json!(s.code));
        o.insert("height".into(), json!(s.height_mm));
        o.insert("depth".into(), json!(s.depth_mm));
        o.insert("min_wall".into(), json!(s.min_wall_mm));
        for (k, x) in given {
            o.insert(k.clone(), x.clone());
        }
    }
    serde_json::from_value(v).map_err(invalid)
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// The Aligners data of the case (a copy), or new ones.
fn aligners_data(state: &CaseState) -> Result<AlignersState, CommandError> {
    match state.module(MODULE_ID) {
        None => Ok(AlignersState::default()),
        Some(m) => m
            .downcast_ref::<AlignersState>()
            .cloned()
            .ok_or_else(crate::commands::messages::no_data),
    }
}

/// The segmented scan of an arch in its arch frame (the frame its seeds define).
struct ArchScan {
    /// The scan in arch coordinates.
    mesh: TriMesh,
    /// The tooth label of every vertex.
    labels: Vec<u16>,
    /// Arch to case coordinates.
    to_case: DMat4,
    /// Case to arch coordinates.
    to_arch: DMat4,
}

/// The segmented scan of an arch; `None` when the arch has no scan or no labels.
fn arch_scan(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
) -> Result<Option<ArchScan>, CommandError> {
    let role = arch.role();
    let (Some(scan), Some(labels_ref)) = (state.case.scan(role), data.labels.get(&role)) else {
        return Ok(None);
    };
    let mesh = state.blobs.require(&scan.mesh)?;
    let labels = state
        .blobs
        .require(labels_ref)?
        .vertex_labels
        .clone()
        .ok_or_else(crate::commands::messages::tooth_labels_missing)?;
    if labels.len() != mesh.vertex_count() {
        return Err(crate::commands::messages::tooth_labels_mismatch(
            labels.len(),
            mesh.vertex_count(),
        ));
    }
    let vertices: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let framed = frame::run(
        &frame::Input {
            arch,
            vertices: &vertices,
            seeds: &data.seeds,
            occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
        },
        &frame::Params::default(),
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    let to_case = framed.arch_frame.matrix();
    let to_arch = frame::rigid_inverse(&to_case);
    let moved: Vec<DVec3> = vertices
        .iter()
        .map(|p| to_arch.transform_point3(*p))
        .collect();
    Ok(Some(ArchScan {
        mesh: to_mesh(&moved, &mesh.tris),
        labels,
        to_case,
        to_arch,
    }))
}

fn to_mesh(vertices: &[DVec3], tris: &[[u32; 3]]) -> TriMesh {
    TriMesh::new(
        vertices.iter().map(|p| p.as_vec3()).collect(),
        tris.to_vec(),
    )
}

/// The mesh to stamp and the teeth where the text can go, with the way
/// back to case coordinates.
struct Target {
    mesh: TriMesh,
    teeth: Vec<ToothSite>,
    to_case: DMat4,
}

/// The target of an arch: for a placement on a tooth the segmented scan in
/// arch coordinates with the crown of every labelled tooth, for a placement
/// by hand the scan as it is.
fn target(
    state: &CaseState,
    data: &AlignersState,
    arch: Arch,
    manual: bool,
) -> Result<Option<Target>, CommandError> {
    if manual {
        let Some(scan) = state.case.scan(arch.role()) else {
            return Ok(None);
        };
        return Ok(Some(Target {
            mesh: (**state.blobs.require(&scan.mesh)?).clone(),
            teeth: Vec::new(),
            to_case: DMat4::IDENTITY,
        }));
    }
    let Some(ArchScan {
        mesh,
        labels,
        to_case,
        to_arch,
    }) = arch_scan(state, data, arch)?
    else {
        return Ok(None);
    };
    let mut crowns: std::collections::BTreeMap<u16, Vec<DVec3>> = std::collections::BTreeMap::new();
    for (p, &label) in mesh.positions.iter().zip(&labels) {
        if label != 0 {
            crowns.entry(label).or_default().push(p.as_dvec3());
        }
    }
    let teeth = crowns
        .into_iter()
        .filter_map(|(label, crown)| {
            let fdi = Fdi::new(u8::try_from(label).ok()?).ok()?;
            let e_md = data
                .tooth_frames
                .get(&fdi)
                .map(|f| to_arch.transform_vector3(f.pose.q * DVec3::X));
            (Arch::of(fdi) == arch).then_some(ToothSite { fdi, crown, e_md })
        })
        .collect();
    Ok(Some(Target {
        mesh,
        teeth,
        to_case,
    }))
}

fn run_stamp(
    state: &CaseState,
    params: StampParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    let sp = case_stamp_params(&data.settings.stamp, &params.stamp)?;
    let manual = matches!(sp.placement, Placement::Manual { .. });
    let explicit = params.arch != Which::All;
    let dir = params.dir.as_deref().map(Path::new);
    if let Some(d) = dir {
        std::fs::create_dir_all(d)
            .map_err(|e| CommandError::Failed(format!("{}: {e}", d.display())))?;
    }
    let mut out = Map::new();
    for arch in params.arch.arches() {
        ctx.check_cancelled()?;
        let Some(target) = target(state, &data, arch, manual)? else {
            if explicit {
                return Err(if manual {
                    crate::commands::messages::no_scan(arch)
                } else {
                    crate::commands::messages::not_segmented(arch)
                });
            }
            continue;
        };
        // the text goes on a tooth the aligner covers (any tooth when the
        // case does not know them yet)
        let covered = data.aligner_teeth(arch.role());
        let input = stamp::Input {
            mesh: &target.mesh,
            arch,
            teeth: target.teeth,
            covered: (!covered.is_empty()).then_some(&covered),
        };
        let t0 = Instant::now();
        let outcome = stamp::run(&input, &sp).map_err(|e| match e {
            stamp::StampError::Invalid(m) => CommandError::Invalid(m),
            e => CommandError::Failed(format!("{} arch: {e}", arch_name(arch))),
        })?;
        let report = match outcome {
            Outcome::Skipped { reason } => json!({ "stamped": false, "reason": reason }),
            Outcome::Stamped(done) => {
                let mut mesh = done.mesh;
                for p in &mut mesh.positions {
                    *p = target.to_case.transform_point3(p.as_dvec3()).as_vec3();
                }
                let written = match dir {
                    Some(d) => {
                        let file = d.join(format!("{}_stamped.stl", arch_name(arch)));
                        dc_mesh::io::write(&file, &mesh, MeshFormat::Stl).map_err(|e| {
                            CommandError::Failed(format!("{}: {e}", file.display()))
                        })?;
                        Some(file.display().to_string())
                    }
                    None => None,
                };
                json!({
                    "stamped": true,
                    "placed": done.placed,
                    "resolution_mm": done.resolution_mm,
                    "triangles_added": done.triangles_added,
                    "vertices": mesh.vertex_count(),
                    "faces": mesh.face_count(),
                    "seconds": t0.elapsed().as_secs_f64(),
                    "file": written,
                })
            }
        };
        out.insert(arch_name(arch).to_owned(), report);
    }
    if out.is_empty() {
        return Err(CommandError::Invalid(
            "no arch has a scan to stamp: import a scan or generate the demo arch".to_owned(),
        ));
    }
    Ok((state.clone(), Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use serde_json::json;

    const ID: &str = "aligners.production.stamp";

    fn run(state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(ID)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo(arch: &str) -> CaseState {
        crate::commands::find("aligners.demo")
            .unwrap()
            .execute(
                &CaseState::default(),
                &json!({ "arch": arch }),
                &JobCtx::detached(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn the_command_is_for_the_cli_with_a_schema_and_needs_a_scan() {
        assert!(crate::tree::find(ID).is_none());
        let spec = crate::commands::find(ID).unwrap();
        let schema = spec.schema();
        assert_eq!(schema["type"], "object");
        for key in [
            "arch",
            "dir",
            "text",
            "depth",
            "height",
            "placement",
            "kind",
        ] {
            assert!(schema["properties"][key].is_object(), "{key}: {schema}");
        }
        assert!(matches!(
            run(&CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
        let lower = demo("lower");
        assert!(matches!(
            run(&lower, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("upper")
        ));
        // a typo in the parameters is an error, not a silent default
        assert!(matches!(
            run(&lower, json!({ "dept": 0.1 })),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            run(&lower, json!({ "depth": 5.0 })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn the_demo_arch_is_stamped_and_written() {
        let state = demo("lower");
        let dir = std::env::temp_dir().join(format!("dc-stamp-{}", std::process::id()));
        let (next, out) = run(
            &state,
            json!({
                "number": 7,
                "date": "2026-10-13",
                "code": "demo-1",
                "resolution_mm": 0.06,
                "dir": dir.to_str().unwrap(),
            }),
        )
        .unwrap();
        assert_eq!(
            next.blobs.iter().count(),
            state.blobs.iter().count(),
            "the case is not modified"
        );
        assert_eq!(next.case.scans.len(), state.case.scans.len());
        let lower = &out["lower"];
        assert_eq!(lower["stamped"], true, "{out}");
        assert_eq!(lower["placed"]["engraved"], false);
        let text = lower["placed"]["text"].as_str().unwrap();
        // the case code is dropped when the full text does not fit on the tooth
        assert!(
            text == "INF 07 / 131026 / DEMO-1" || text == "INF 07 / 131026",
            "{text}"
        );
        assert!(lower["faces"].as_u64().unwrap() > 1000);
        let file = lower["file"].as_str().unwrap();
        let mesh = dc_mesh::io::read(file).unwrap();
        let report = mesh.validate();
        assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A case whose upper scan is a 16 x 16 x 6 mm block.
    fn block_case() -> CaseState {
        let mut state = CaseState::default();
        let (w, h) = (8.0f32, 3.0f32);
        let positions: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -w } else { w },
                    if i & 2 == 0 { -w } else { w },
                    if i & 4 == 0 { -h } else { h },
                )
            })
            .collect();
        // outward quads of the cube whose corner i has bits (x, y, z)
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let tris = quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect();
        let mesh = state.blobs.insert(TriMesh::new(positions, tris));
        let scan = serde_json::from_value(json!({
            "id": "de300000-0000-4000-8000-000000000001",
            "role": "upper",
            "source_name": "block",
            "source_units": "mm",
            "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
            "mesh": mesh,
        }))
        .unwrap();
        state.case.scans.push(scan);
        state
    }

    #[test]
    fn a_manual_placement_needs_no_teeth_and_gum_separated() {
        let state = block_case();
        let (_, out) = run(
            &state,
            json!({
                "arch": "upper",
                "kind": "shell",
                "date": "2026-10-13",
                "placement": {
                    "mode": "manual",
                    "point": [0.0, 0.0, 3.0],
                    "normal": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                },
            }),
        )
        .unwrap();
        let upper = &out["upper"];
        assert_eq!(upper["stamped"], true, "{out}");
        assert!(upper["placed"]["tooth"].is_null());
        assert_eq!(upper["placed"]["engraved"], true);
        assert_eq!(upper["placed"]["text"], "SUP 01 / 131026");
        // the same call on a tooth needs the segmentation
        assert!(matches!(
            run(&state, json!({ "arch": "upper" })),
            Err(CommandError::Invalid(m)) if m.contains("tooth colours")
        ));
    }
}
