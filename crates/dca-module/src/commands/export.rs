//! Commands of the `export` stage (`dca_algo::export`).
//!
//! `aligners.output.export-stl` writes the aligners into a target directory:
//! the scan of each arch with the teeth at every stage as binary STL
//! (`upper_stage_00.stl` is the initial situation), `stages.csv` with the
//! movement of every tooth at every stage and the dates, and `visits.ics`
//! with the aligner changes of both arches. Parameters: `dir`,
//! `start_date`, `interval_days`, `policy`, `lang`. The case is not
//! modified. The print check, the shells and the report come later.
//!
//! The gingiva stays where it is in the models until the gingiva
//! deformation lands.

use std::collections::BTreeMap;

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_module_api::{CommandOutput, CommandSpec};
use dca_algo::anatomy::{self, Arch};
use dca_algo::export::{self, ArchSeries};
use dca_algo::schedule;
use dca_algo::teeth;

use super::staging::{aligners_data, labelled_scan, md_sign};

/// The registered commands of this stage, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![stage_command! {
        id: "aligners.output.export-stl",
        label: "aligners-menu-output-export-stl",
        doc: "aligners-cmd-export-doc",
        params: dca_algo::export::Params,
        run: run_export,
    }]
}

fn run_export(
    state: &CaseState,
    mut params: export::Params,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let data = aligners_data(state)?;
    if data.stages.is_empty() {
        return Err(CommandError::Invalid(
            "there are no aligners yet: run the staging first".to_owned(),
        ));
    }
    if params.dir.trim().is_empty() {
        return Err(CommandError::Invalid(
            "dir: the target directory is required".to_owned(),
        ));
    }
    params
        .interval_days
        .get_or_insert(data.settings.interval_days);

    // the scans and the rest poses of the arches that have aligners
    let mut scans = Vec::new();
    for arch in [Arch::Upper, Arch::Lower] {
        if schedule::active_stages(arch, &data.stages) == 0 {
            continue;
        }
        if let Some(scan) = labelled_scan(state, &data, arch.role())? {
            let fdis: Vec<Fdi> = data
                .tooth_frames
                .keys()
                .copied()
                .filter(|&f| Arch::of(f) == arch)
                .collect();
            let seq = anatomy::arch_sequence(&fdis, arch);
            let rest: BTreeMap<_, _> = seq
                .iter()
                .map(|&f| (f, teeth::rest_pose(&data.tooth_frames[&f])))
                .collect();
            let md: BTreeMap<_, _> = seq
                .iter()
                .map(|&f| (f, md_sign(&data, &seq, &rest, f)))
                .collect();
            scans.push((arch, scan, rest, md));
        }
    }
    if scans.is_empty() {
        return Err(CommandError::Invalid(
            "no arch with aligners has a segmented scan".to_owned(),
        ));
    }
    let input = export::Input {
        arches: scans
            .iter()
            .map(|(arch, scan, rest, md)| ArchSeries {
                arch: *arch,
                mesh: &scan.mesh,
                labels: &scan.labels,
                rest,
                md_sign: md,
            })
            .collect(),
        stages: &data.stages,
    };
    ctx.check_cancelled()?;
    let report = export::run(&input, &params).map_err(|e| match e {
        export::ExportError::Io { .. } | export::ExportError::Mesh(_) => {
            CommandError::Failed(e.to_string())
        }
        other => CommandError::Invalid(other.to_string()),
    })?;
    let out = serde_json::to_value(&report).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok((state.clone(), out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn exec(id: &str, state: &CaseState, params: Value) -> Result<CommandOutput, CommandError> {
        crate::commands::find(id)
            .unwrap()
            .execute(state, &params, &JobCtx::detached())
    }

    #[test]
    fn the_command_is_registered_as_the_menu_leaf() {
        let spec = crate::commands::find("aligners.output.export-stl").unwrap();
        assert!(spec.schema()["properties"]["dir"].is_object());
        assert!(crate::tree::find("aligners.output.export-stl").is_some());
    }

    #[test]
    fn it_needs_aligners_and_a_directory() {
        let err = exec(
            "aligners.output.export-stl",
            &CaseState::default(),
            json!({"dir": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("staging")));
        let s = super::super::staging::tests::staged_input("lower", &CaseState::default());
        let (s, _) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let err = exec("aligners.output.export-stl", &s, Value::Null).unwrap_err();
        assert!(matches!(err, CommandError::Invalid(m) if m.contains("dir")));
    }

    #[test]
    fn it_exports_the_stl_the_csv_and_the_calendar_of_both_arches() {
        use super::super::staging::tests::staged_input;
        let s = staged_input("lower", &CaseState::default());
        let s = staged_input("upper", &s);
        let (s, staged) = exec("aligners.planning.staging.automatic", &s, Value::Null).unwrap();
        let nl = staged["lower"]["aligners"].as_u64().unwrap();
        let nu = staged["upper"]["aligners"].as_u64().unwrap();
        let dir = std::env::temp_dir().join(format!("dca-export-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (same, out) = exec(
            "aligners.output.export-stl",
            &s,
            json!({ "dir": dir.display().to_string(), "start_date": "2026-01-05", "interval_days": 7 }),
        )
        .unwrap();
        assert_eq!(same.case.scans, s.case.scans);
        assert_eq!(out["aligners"]["lower"], nl);
        assert_eq!(out["aligners"]["upper"], nu);
        assert_eq!(out["total_aligners"], nl.max(nu));
        assert_eq!(out["total_days"].as_u64().unwrap(), 7 * nl.max(nu));
        let files = out["files"].as_array().unwrap();
        assert_eq!(
            files.iter().filter(|f| f["kind"] == "stl").count() as u64,
            nl + nu + 2
        );

        // every STL loads back with dc-mesh; the initial model is the scan
        let scan = s
            .mesh(&s.case.scan(dc_case::ScanRole::Lower).unwrap().mesh)
            .unwrap()
            .clone();
        let w = (nl.max(nu)).to_string().len().max(2);
        let first = dc_mesh::io::read(dir.join(format!("lower_stage_{:0w$}.stl", 0))).unwrap();
        assert_eq!(first.face_count(), scan.face_count());
        let last = dc_mesh::io::read(dir.join(format!("lower_stage_{nl:0w$}.stl"))).unwrap();
        assert_eq!(last.face_count(), scan.face_count());
        // the final model differs from the initial one: the teeth moved
        // (the vertex order of an STL is the order of its triangles, so compare sorted sets)
        let key = |m: &dc_mesh::TriMesh| {
            let mut v: Vec<[u32; 3]> = m
                .positions
                .iter()
                .map(|p| p.to_array().map(f32::to_bits))
                .collect();
            v.sort_unstable();
            v
        };
        let moved = |a: &dc_mesh::TriMesh, b: &dc_mesh::TriMesh| key(a) != key(b);
        assert!(moved(&first, &last));
        for f in files.iter().filter(|f| f["kind"] == "stl") {
            let m = dc_mesh::io::read(dir.join(f["name"].as_str().unwrap())).unwrap();
            assert!(m.check().is_ok());
        }

        // the CSV is parseable, the ICS has an event per aligner of both arches
        let csv = std::fs::read_to_string(dir.join("stages.csv")).unwrap();
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        assert_eq!(header[0], "arch");
        for line in csv.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), header.len(), "{line}");
            for c in &cols[4..10] {
                c.parse::<f64>().unwrap();
            }
        }
        assert_eq!(
            csv.lines().count() as u64,
            1 + 14 * (nl + 1) + 14 * (nu + 1)
        );
        let ics = std::fs::read_to_string(dir.join("visits.ics")).unwrap();
        assert_eq!(
            dca_algo::schedule::validate_ics(&ics).unwrap() as u64,
            nl.max(nu) * 2 + 2
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
