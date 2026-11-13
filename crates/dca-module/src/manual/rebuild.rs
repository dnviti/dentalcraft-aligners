//! Rebuild a single tooth by hand (`aligners.manual.rebuild-tooth`,
//! `manual.rebuild_steps` of the plugin).
//!
//! The tooth is built again from its region on the scan: from the current
//! labels, from a selection of vertices that becomes the tooth (its other
//! vertices go to the gingiva), or after the region was found again from the
//! tooth's seed by the crown fill (`resegment`). The region is checked first
//! (size, height, border of the scan, holes): an invalid one is refused with
//! the reason and nothing changes. Then the teeth of the arch are measured
//! again (the frame of this tooth changes with its region) and the closed
//! solid of the tooth replaces the old one; the other teeth keep their
//! solids, frames and movements. The setup and the staging of the tooth are
//! out of date: the report says so.

use dc_case::CaseState;
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::manual::{self, FillParams, Scan};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::common::{
    arch_labels, arch_name, arch_to_case, data_of, fdi_of, finish, invalid, run_command,
    store_labels,
};
use crate::commands::messages;
use dca_algo::anatomy::Arch;
use dca_algo::frame;

/// Parameters of the rebuild of one tooth.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RebuildParams {
    /// Tooth number (FDI) (required).
    pub fdi: u8,
    /// Vertices of the scan that become the tooth; its other vertices go to
    /// the gingiva. Absent: the current labels.
    pub vertices: Option<Vec<u32>>,
    /// Find the region again from the seed of the tooth (the crown fill)
    /// before closing it.
    pub resegment: bool,
    /// Depth of the neck below the lowest margin point (mm).
    pub neck_depth_mm: Option<f64>,
    /// Depth of the end of the closed tooth below the lowest margin point (mm).
    pub apex_mm: Option<f64>,
    /// Section of the neck over the section of the crown.
    pub neck_scale: Option<f64>,
}

pub(crate) fn run_rebuild(
    state: &CaseState,
    p: RebuildParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let f = fdi_of(p.fdi)?;
    let want = u16::from(f.get());
    let arch = Arch::of(f);
    let mut data = data_of(state)?;
    if data.missing.contains(&f) {
        return Err(invalid(format!(
            "tooth {f} is marked missing: mark it present first"
        )));
    }
    let al = arch_labels(state, &data, arch)?;
    if !al.had_labels && p.vertices.is_none() && !p.resegment {
        return Err(messages::not_segmented(arch));
    }
    let vertices = al.vertices();
    let n = vertices.len();
    let scan = Scan::new(&vertices, &al.scan.tris).map_err(|e| invalid(e.to_string()))?;

    // the labels the tooth is built from
    let mut labels = al.labels.clone();
    if p.resegment {
        let seed = data.seeds.iter().find(|s| s.fdi == f).ok_or_else(|| {
            messages::invalid("aligners-err-tooth-no-mark", &[("fdi", &f.to_string())])
        })?;
        let v = scan
            .nearest(seed.position)
            .ok_or_else(|| invalid("the scan has no vertices"))?;
        let up = arch_to_case(&data, arch, &vertices)?.z_axis.truncate();
        let region = manual::fill_region(&scan, &labels, v, want, up, &FillParams::default());
        if region.is_empty() {
            return Err(messages::invalid(
                "aligners-err-no-crown-at-mark",
                &[("fdi", &f.to_string())],
            ));
        }
        for l in &mut labels {
            if *l == want {
                *l = 0;
            }
        }
        for i in region {
            labels[i as usize] = want;
        }
    }
    if let Some(sel) = &p.vertices {
        if sel.is_empty() {
            return Err(invalid("the selection is empty"));
        }
        if sel.iter().any(|&v| v as usize >= n) {
            return Err(invalid(format!(
                "a vertex index is out of range (the {} scan has {n} vertices)",
                arch_name(arch)
            )));
        }
        for l in &mut labels {
            if *l == want {
                *l = 0;
            }
        }
        for &v in sel {
            labels[v as usize] = want;
        }
    }

    // the region must make a crown
    let framed_to_case = arch_to_case(&data, arch, &vertices)?;
    let to_arch = frame::rigid_inverse(&framed_to_case);
    let arch_vertices: Vec<_> = vertices
        .iter()
        .map(|&v| to_arch.transform_point3(v))
        .collect();
    let report = manual::check_region(&scan, &labels, f, &arch_vertices);
    if !report.errors.is_empty() {
        return Err(invalid(report.message(f)));
    }

    // labels first, then the measures of the arch and the solid of the tooth
    let mut next = state.clone();
    let mut stale = Vec::new();
    let changed = labels != al.labels;
    if changed || !al.had_labels {
        store_labels(&mut next, &mut data, &mut stale, arch, &al.scan, labels);
    }
    data.dirty.remove(&f);
    data.rebuilt_by_hand(arch.role());
    // a tooth rebuilt by hand: the doubts of the check on the arch are answered
    data.forget_separation(arch.role());
    // what the setup and the staging computed for the tooth is out of date
    let moved =
        data.setup.poses.contains_key(&f) || data.stages.iter().any(|s| s.poses.contains_key(&f));
    // the old solid is replaced by the rebuild below
    finish(&mut next, data, &stale);
    let (s, _) = run_command(
        "aligners.teeth.frames",
        &next,
        json!({ "arch": arch_name(arch) }),
        ctx,
    )?;
    next = s;
    let mut sub = Map::new();
    sub.insert("fdi".to_owned(), json!(f.get()));
    for (k, v) in [
        ("neck_depth_mm", p.neck_depth_mm),
        ("apex_mm", p.apex_mm),
        ("neck_scale", p.neck_scale),
    ] {
        if let Some(v) = v {
            sub.insert(k.to_owned(), json!(v));
        }
    }
    let (s, closed) = run_command(
        "aligners.preparation.reconstruction.rebuild-tooth",
        &next,
        Value::Object(sub),
        ctx,
    )?;
    next = s;
    Ok((
        next,
        json!({
            "fdi": f.get(),
            "arch": arch_name(arch),
            "labels_changed": changed,
            "region": report,
            "message": report.message(f),
            "setup_outdated": moved,
            "solid": closed,
        }),
    ))
}
