//! The region of a tooth on the scan and its margin line
//! (`aligners.manual.tooth-region`, `oa.tooth_highlight` of the plugin).
//!
//! It answers from which part of the scan a tooth is built: the vertices and
//! faces that carry its tooth colour, the check of that region (the one a
//! rebuild makes first), and the margin line, the crown / gingiva boundary
//! the closed tooth is cut along ([`dca_algo::manual::margin_line`]). The
//! margin line is not stored: it is worked out from the colours each time,
//! so it is always the line the next rebuild of the tooth would use.
//!
//! The case is not modified (the command returns it as it was, like the
//! check and report commands): this is the data of the "Show the region" view
//! of the 3D teeth, which the window draws without running the command.

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::manual::{self, Scan};
use glam::DVec3;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::common::{arch_labels, arch_name, arch_to_case, data_of, fdi_of};
use crate::commands::messages;

/// Parameters of the query of the region of one tooth.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RegionParams {
    /// Tooth number (FDI) (required).
    pub fdi: u8,
    /// Also list the vertices and faces of the region (indices into the
    /// scan) and the points of the margin line (scan coordinates).
    pub detail: bool,
}

pub(crate) fn run_region(
    state: &CaseState,
    p: RegionParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let f = fdi_of(p.fdi)?;
    let want = u16::from(f.get());
    let arch = Arch::of(f);
    let data = data_of(state)?;
    let al = arch_labels(state, &data, arch)?;
    if !al.had_labels {
        return Err(messages::not_segmented(arch));
    }
    let vertices = al.vertices();
    let tris = &al.scan.tris;
    let inside: Vec<u32> = (0..vertices.len() as u32)
        .filter(|&i| al.labels[i as usize] == want)
        .collect();
    let faces: Vec<u32> = (0..tris.len() as u32)
        .filter(|&k| {
            tris[k as usize]
                .iter()
                .all(|&i| al.labels[i as usize] == want)
        })
        .collect();

    // the check a rebuild makes, when the arch frame can be measured
    let scan = Scan::new(&vertices, tris).map_err(|e| CommandError::Invalid(e.to_string()))?;
    let (region, region_error) = match arch_to_case(&data, arch, &vertices) {
        Ok(to_case) => {
            let to_arch = frame::rigid_inverse(&to_case);
            let in_arch: Vec<DVec3> = vertices
                .iter()
                .map(|&v| to_arch.transform_point3(v))
                .collect();
            (
                Some(manual::check_region(&scan, &al.labels, f, &in_arch)),
                None,
            )
        }
        Err(e) => (None, Some(e.to_string())),
    };

    let line = manual::margin_line(&vertices, tris, &al.labels, want);
    let polylines = line.polylines();
    let mut margin = Map::new();
    margin.insert("points".to_owned(), json!(line.points.len()));
    margin.insert("segments".to_owned(), json!(line.segments.len()));
    margin.insert("length_mm".to_owned(), json!(line.length()));
    margin.insert(
        "closed".to_owned(),
        json!(!polylines.is_empty() && polylines.iter().all(|l| l.closed)),
    );
    let summary: Vec<Value> = polylines
        .iter()
        .map(|l| json!({ "closed": l.closed, "points": l.points.len() }))
        .collect();
    margin.insert("loops".to_owned(), Value::Array(summary));
    let message = match &region {
        Some(r) => r.message(f),
        None => format!("Tooth {f}: {} vertices", inside.len()),
    };
    let mut out = Map::new();
    out.insert("fdi".to_owned(), json!(f.get()));
    out.insert("arch".to_owned(), json!(arch_name(arch)));
    out.insert("vertices".to_owned(), json!(inside.len()));
    out.insert("faces".to_owned(), json!(faces.len()));
    out.insert("region".to_owned(), json!(region));
    out.insert("region_error".to_owned(), json!(region_error));
    out.insert("message".to_owned(), json!(message));
    if p.detail {
        out.insert("vertex_indices".to_owned(), json!(inside));
        out.insert("face_indices".to_owned(), json!(faces));
        let lines: Vec<Value> = polylines
            .iter()
            .map(|l| {
                let pts: Vec<[f64; 3]> = l
                    .points
                    .iter()
                    .map(|&i| line.points[i as usize].to_array())
                    .collect();
                json!({ "closed": l.closed, "points": pts })
            })
            .collect();
        margin.insert("polylines".to_owned(), Value::Array(lines));
    }
    out.insert("margin".to_owned(), Value::Object(margin));
    // a query: the case is returned as it was
    Ok((state.clone(), Value::Object(out)))
}
