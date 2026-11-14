//! Commands of the `study_base` stage (`dca_algo::study_base`, B6): the
//! orthodontic study models on a trimmed base, European or ABO style.
//!
//! * `aligners.output.study-models` builds the bases of the arches that have
//!   a scan and tooth labels (in occlusion: after a bite registration or
//!   the virtual bite). Its parameters start from the case settings
//!   (`Settings::study_base`); any [`study_base::Params`] field given
//!   overrides them. With `dir` it writes `study-model-upper.stl` and
//!   `study-model-lower.stl` there. It returns the report (frame, planes,
//!   outline, design and measured angles, heights, warnings) and does not
//!   change the case.
//! * `aligners.settings.study-models` (not a menu leaf) changes the study
//!   base settings of the case: the fields given replace the current ones.

use std::path::PathBuf;

use dc_case::{CaseState, ModuleState};
use dc_command::{CommandError, JobCtx};
use dc_mesh::io::{MeshFormat, write};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::Arch;
use dca_algo::study_base::{self, ArchInput, Input, Params, Style};
use dca_algo::{frame, teeth::Which};
use dca_model::{AlignersState, MODULE_ID, StudyBaseSettings};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Id of the command that builds the study models (a menu leaf).
pub(crate) const STUDY_MODELS_ID: &str = "aligners.output.study-models";
/// Id of the command that changes the study base settings.
pub(crate) const SETTINGS_ID: &str = "aligners.settings.study-models";

/// Parameters of `aligners.output.study-models` (the schema; missing
/// [`Params`] fields take the case settings, then the defaults).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[allow(dead_code)] // only its schema is used: the parameters are merged by hand
struct StudyModelsParams {
    /// Arches to build (default: every arch with a scan and labels).
    arch: Which,
    /// Folder where the STL files are written; none: report only.
    dir: Option<String>,
    /// Style and geometry (default: the case settings).
    #[serde(flatten)]
    base: Params,
}

/// Parameters of `aligners.settings.study-models`: the fields to change.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsPatch {
    /// Style of the base.
    style: Option<Style>,
    /// ABO: height of the articulated pair (mm).
    total_height_mm: Option<f64>,
    /// ABO: fraction of each model taken by the art portion.
    art_fraction: Option<f64>,
    /// European: thickness of the base under the anatomy (mm).
    base_thickness_mm: Option<f64>,
    /// Clearance under the vault and the deepest sulcus (mm).
    m_base_mm: Option<f64>,
    /// Back plane behind the most distal anatomy (mm).
    back_margin_mm: Option<f64>,
    /// ABO: angle of the side cuts to the back line (degrees).
    side_angle_deg: Option<f64>,
    /// Interior angle of the heels with the back plane (degrees).
    heel_angle_deg: Option<f64>,
    /// ABO: side cuts outside the deepest sulcus (mm).
    side_clear_mm: Option<f64>,
    /// European: walls outside the vestibule (mm).
    vestibule_clear_mm: Option<f64>,
}

/// The registered commands of this stage.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: STUDY_MODELS_ID,
            label_key: "aligners-menu-output-study-models",
            doc_key: "aligners-cmd-study-models-doc",
            params_schema: || schemars::schema_for!(StudyModelsParams).to_value(),
            run: run_study_models,
        },
        stage_command! {
            id: SETTINGS_ID,
            label: "aligners-cmd-study-models-settings",
            doc: "aligners-cmd-study-models-settings-doc",
            params: SettingsPatch,
            run: run_settings,
        },
    ]
}

fn failed(e: impl ToString) -> CommandError {
    CommandError::Failed(e.to_string())
}

fn data_of(state: &CaseState) -> Result<&AlignersState, CommandError> {
    state
        .module_data::<AlignersState>(MODULE_ID)
        .ok_or_else(crate::commands::messages::no_case_data)
}

/// The geometry parameters: the settings, overridden by the fields of
/// `given` (an object without `arch` and `dir`).
pub(crate) fn params_from(
    settings: &StudyBaseSettings,
    given: &Map<String, Value>,
) -> Result<Params, CommandError> {
    let mut base = match serde_json::to_value(settings).map_err(failed)? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    // the lower sides follow the upper ones unless asked otherwise (flush heels)
    base.insert(
        "lower_side_angle_deg".to_owned(),
        json!(settings.side_angle_deg),
    );
    for (k, v) in given {
        base.insert(k.clone(), v.clone());
    }
    let p: Params = serde_json::from_value(Value::Object(base))
        .map_err(|e| CommandError::Invalid(format!("parameters: {e}")))?;
    p.check()
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    Ok(p)
}

fn run_study_models(
    state: &CaseState,
    params: &Value,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut given = match params {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => {
            return Err(CommandError::Invalid(
                "parameters: expected an object".to_owned(),
            ));
        }
    };
    let which: Which = match given.remove("arch") {
        Some(v) => serde_json::from_value(v)
            .map_err(|e| CommandError::Invalid(format!("parameters: arch: {e}")))?,
        None => Which::All,
    };
    let dir: Option<PathBuf> = match given.remove("dir") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(PathBuf::from(s)),
        Some(other) => {
            return Err(CommandError::Invalid(format!(
                "parameters: dir must be a string, got {other}"
            )));
        }
    };
    let data = data_of(state)?;
    let p = params_from(&data.settings.study_base, &given)?;
    let explicit = which != Which::All;

    // scans, labels and arch frames
    struct Loaded {
        arch: Arch,
        scan: std::sync::Arc<dc_mesh::TriMesh>,
        labels: Vec<u16>,
        frame: frame::ArchFrame,
    }
    let mut loaded = Vec::new();
    for arch in which.arches() {
        let role = arch.role();
        let (Some(scan), Some(lab)) = (state.case.scan(role), data.labels.get(&role)) else {
            if explicit {
                return Err(super::messages::not_segmented(arch));
            }
            continue;
        };
        let mesh = state.blobs.require(&scan.mesh)?.clone();
        let labels = state
            .blobs
            .require(lab)?
            .vertex_labels
            .clone()
            .ok_or_else(super::messages::tooth_labels_missing)?;
        let v: Vec<_> = mesh.positions.iter().map(|q| q.as_dvec3()).collect();
        let framed = frame::run(
            &frame::Input {
                arch,
                vertices: &v,
                seeds: &data.seeds,
                occlusal_flipped: data.occlusal_flipped.contains(&arch.role()),
            },
            &frame::Params::default(),
        )
        .map_err(failed)?;
        loaded.push(Loaded {
            arch,
            scan: mesh,
            labels,
            frame: framed.arch_frame,
        });
    }
    if loaded.is_empty() {
        return Err(super::messages::none_segmented());
    }
    let arch_input = |want: Arch| {
        loaded.iter().find(|l| l.arch == want).map(|l| ArchInput {
            scan: &l.scan,
            labels: &l.labels,
            arch_frame: l.frame,
        })
    };
    let input = Input {
        upper: arch_input(Arch::Upper),
        lower: arch_input(Arch::Lower),
    };
    ctx.check_cancelled()?;
    let out = study_base::run(&input, &p).map_err(|e| match e {
        study_base::StudyBaseError::Invalid(m) => CommandError::Invalid(m),
        other => failed(other),
    })?;
    ctx.check_cancelled()?;

    let mut files = Vec::new();
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir).map_err(failed)?;
        for (name, mesh) in [("upper", &out.upper_base), ("lower", &out.lower_base)] {
            if let Some(m) = mesh {
                let path = dir.join(format!("study-model-{name}.stl"));
                write(&path, m, MeshFormat::Stl).map_err(failed)?;
                files.push(path.display().to_string());
            }
        }
    }
    let report = serde_json::to_value(&out.report).map_err(failed)?;
    Ok((
        state.clone(),
        json!({
            "params": serde_json::to_value(&p).map_err(failed)?,
            "report": report,
            "files": files,
        }),
    ))
}

fn run_settings(
    state: &CaseState,
    patch: SettingsPatch,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let mut data = data_of(state)?.clone();
    let mut current = match serde_json::to_value(&data.settings.study_base).map_err(failed)? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let patch = json!({
        "style": patch.style,
        "total_height_mm": patch.total_height_mm,
        "art_fraction": patch.art_fraction,
        "base_thickness_mm": patch.base_thickness_mm,
        "m_base_mm": patch.m_base_mm,
        "back_margin_mm": patch.back_margin_mm,
        "side_angle_deg": patch.side_angle_deg,
        "heel_angle_deg": patch.heel_angle_deg,
        "side_clear_mm": patch.side_clear_mm,
        "vestibule_clear_mm": patch.vestibule_clear_mm,
    });
    if let Value::Object(m) = patch {
        for (k, v) in m {
            if !v.is_null() {
                current.insert(k, v);
            }
        }
    }
    let settings: StudyBaseSettings =
        serde_json::from_value(Value::Object(current)).map_err(failed)?;
    // the values must make valid geometry parameters
    params_from(&settings, &Map::new())?;
    data.settings.study_base = settings.clone();
    let mut next = state.clone();
    next.set_module(MODULE_ID, ModuleState::new(data));
    Ok((
        next,
        json!({ "study_base": serde_json::to_value(&settings).map_err(failed)? }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    fn demo_pair() -> CaseState {
        let mut state = CaseState::default();
        for arch in ["lower", "upper"] {
            state = run("aligners.demo", &state, json!({ "arch": arch }))
                .unwrap()
                .0;
        }
        state
    }

    #[test]
    fn the_commands_are_registered_and_one_is_a_menu_leaf() {
        assert!(crate::commands::is_registered(STUDY_MODELS_ID));
        assert!(crate::tree::find(STUDY_MODELS_ID).is_some());
        assert!(crate::commands::is_registered(SETTINGS_ID));
        let schema = crate::commands::find(STUDY_MODELS_ID).unwrap().schema();
        let props = &schema["properties"];
        for k in [
            "style",
            "dir",
            "total_height_mm",
            "side_angle_deg",
            "heel_angle_deg",
        ] {
            assert!(props[k].is_object(), "{k}: {schema}");
        }
    }

    #[test]
    fn the_settings_command_changes_the_case_and_the_defaults_of_the_build() {
        let state = demo_pair();
        let (next, out) = run(
            SETTINGS_ID,
            &state,
            json!({ "style": "abo", "total_height_mm": 72.0 }),
        )
        .unwrap();
        assert_eq!(out["study_base"]["style"], "abo");
        let data = next.module_data::<AlignersState>(MODULE_ID).unwrap();
        assert_eq!(
            data.settings.study_base.style,
            dca_model::StudyBaseStyle::Abo
        );
        assert_eq!(data.settings.study_base.total_height_mm, 72.0);
        assert_eq!(data.settings.study_base.heel_angle_deg, 115.0);
        let p = params_from(&data.settings.study_base, &Map::new()).unwrap();
        assert_eq!(p.style, Style::Abo);
        assert_eq!(p.total_height_mm, 72.0);
        let over = json!({ "style": "european" });
        let p = params_from(&data.settings.study_base, over.as_object().unwrap()).unwrap();
        assert_eq!(p.style, Style::European);
        assert!(matches!(
            run(SETTINGS_ID, &state, json!({ "side_angle_deg": 150.0 })),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            run(SETTINGS_ID, &state, json!({ "nope": 1 })),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn without_labels_the_build_says_what_is_missing() {
        assert!(matches!(
            run(STUDY_MODELS_ID, &CaseState::default(), Value::Null),
            Err(CommandError::Invalid(_))
        ));
        let state = demo_pair();
        assert!(matches!(
            run(STUDY_MODELS_ID, &state, json!({ "dir": 3 })),
            Err(CommandError::Invalid(_))
        ));
        assert!(matches!(
            run(STUDY_MODELS_ID, &state, json!({ "heel_angle_deg": 30.0 })),
            Err(CommandError::Invalid(_))
        ));
    }

    /// Writes occlusal and frontal renders of the bases to `$DC_STUDY_PNG_DIR`
    /// when it is set (to look at them).
    fn render(tag: &str, state: &CaseState, report: &Value) {
        let Ok(dir) = std::env::var("DC_STUDY_PNG_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let f = &report["frame"];
        let v = |k: &str| {
            let a = f[k].as_array().unwrap();
            glam::DVec3::new(
                a[0].as_f64().unwrap(),
                a[1].as_f64().unwrap(),
                a[2].as_f64().unwrap(),
            )
        };
        let to_case = dca_algo::frame::make_frame(v("x"), v("y"), v("z"), v("origin"));
        // world of the renders: incisors towards -Y, patient's right on -X
        let flip = glam::DMat4::from_scale(glam::DVec3::new(-1.0, -1.0, 1.0));
        let xf = (flip * dca_algo::frame::rigid_inverse(&to_case)).as_mat4();
        let _ = state;
        let read =
            |n: &str| dc_mesh::io::read(dir.join(format!("{tag}-stl/study-model-{n}.stl"))).ok();
        let (up, lo) = (read("upper"), read("lower"));
        fn items<'a>(
            ms: &[&'a Option<dc_mesh::TriMesh>],
            xf: glam::Mat4,
        ) -> Vec<dc_raster::Item<'a>> {
            ms.iter()
                .filter_map(|m| m.as_ref())
                .map(|m| dc_raster::Item::new(m).with_xform(xf))
                .collect()
        }
        for (name, view, ms) in [
            ("frontal", dc_raster::View::Frontal, items(&[&up, &lo], xf)),
            ("back", dc_raster::View::Back, items(&[&up, &lo], xf)),
            (
                "occlusal-upper",
                dc_raster::View::OcclusalUpper,
                items(&[&up], xf),
            ),
            (
                "occlusal-lower",
                dc_raster::View::OcclusalLower,
                items(&[&lo], xf),
            ),
        ] {
            if ms.is_empty() {
                continue;
            }
            let img =
                dc_raster::render(&ms, &dc_raster::RenderOptions::new(900, 700, view)).unwrap();
            img.write_png(dir.join(format!("{tag}-{name}.png")))
                .unwrap();
        }
    }

    fn check_pair(tag: &str, state: &CaseState, style: &str) -> Value {
        let dir = std::env::var("DC_STUDY_PNG_DIR")
            .map(|d| std::path::PathBuf::from(d).join(format!("{tag}-stl")))
            .unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("dc-study-{tag}-{}", std::process::id()))
            });
        let params = json!({ "style": style, "dir": dir.display().to_string() });
        let (next, out) = run(STUDY_MODELS_ID, state, params.clone()).unwrap();
        assert_eq!(next.referenced_blobs(), state.referenced_blobs());
        let r = &out["report"];
        eprintln!("{tag}: {}", serde_json::to_string_pretty(r).unwrap());
        render(tag, state, r);
        // deterministic
        let (_, again) = run(STUDY_MODELS_ID, state, json!({ "style": style })).unwrap();
        let untimed = |r: &Value| {
            let mut r = r.clone();
            for a in r["arches"].as_object_mut().unwrap().values_mut() {
                a.as_object_mut().unwrap().remove("seconds");
            }
            r
        };
        assert_eq!(untimed(&again["report"]), untimed(&out["report"]));
        if std::env::var("DC_STUDY_PNG_DIR").is_err() {
            std::fs::remove_dir_all(&dir).ok();
        }
        out
    }

    fn arch_checks(r: &Value, arch: &str, style: &str) {
        let a = &r["arches"][arch];
        assert_eq!(a["watertight"], true, "{arch}: {a}");
        assert_eq!(a["components"], 1, "{arch}");
        assert!(a["base_tilt_deg"].as_f64().unwrap() < 0.5, "{arch}: {a}");
        for w in a["walls"].as_array().unwrap() {
            assert!(
                (w["to_base_deg"].as_f64().unwrap() - 90.0).abs() < 0.5,
                "{arch}: {w}"
            );
        }
        let back = a["walls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["name"] == "back")
            .unwrap();
        // perpendicular to the raphe (common y)
        let n = back["normal"].as_array().unwrap();
        let ny = n[1].as_f64().unwrap();
        assert!((-ny).acos().to_degrees() < 0.5, "{arch}: back {back}");
        let d = &a["design_angles_deg"];
        let m = &a["measured_angles_deg"];
        if style == "abo" {
            assert!(
                (a["art_mm"].as_f64().unwrap() - a["height_mm"].as_f64().unwrap() / 3.0).abs()
                    < 1e-6
            );
            if arch == "upper" {
                for (k, want) in [
                    ("side_to_back_line", 65.0),
                    ("back_heel", 115.0),
                    ("apex", 130.0),
                ] {
                    assert!((d[k].as_f64().unwrap() - want).abs() < 1e-6, "{k}: {d}");
                    assert!((m[k].as_f64().unwrap() - want).abs() < 0.5, "{k}: {m}");
                }
            }
        }
    }

    /// Both demo arches in the virtual occlusion, both styles.
    #[test]
    #[ignore = "two demo arches, virtual bite and four bases: run with --release --ignored"]
    fn the_demo_pair_gets_both_styles() {
        let state = demo_pair();
        let state = run("aligners.occlusion.virtual-bite", &state, Value::Null)
            .unwrap()
            .0;
        for style in ["abo", "european"] {
            let out = check_pair(&format!("demo-{style}"), &state, style);
            let r = &out["report"];
            for arch in ["upper", "lower"] {
                arch_checks(r, arch, style);
            }
            assert!(r["back_coplanarity_deg"].as_f64().unwrap() < 0.5, "{r}");
            assert!(r["back_offset_mm"].as_f64().unwrap() < 0.5, "{r}");
            if style == "abo" {
                let raised = r["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w["code"] == "height_raised");
                if !raised {
                    assert!((r["total_height_mm"].as_f64().unwrap() - 70.0).abs() < 1e-9);
                    assert!(
                        (r["measured_total_height_mm"].as_f64().unwrap() - 70.0).abs() < 0.5,
                        "{r}"
                    );
                }
            }
            assert_eq!(out["files"].as_array().unwrap().len(), 2);
        }
    }

    /// A golden fixture (single arch, real scan geometry), both styles.
    #[test]
    #[ignore = "golden scan and two bases: run with --release --ignored"]
    fn a_golden_case_gets_both_styles() {
        let state = super::super::reconstruct::golden_state("demo_upper_mild");
        for style in ["abo", "european"] {
            let out = check_pair(&format!("golden-{style}"), &state, style);
            arch_checks(&out["report"], "upper", style);
        }
    }
}
