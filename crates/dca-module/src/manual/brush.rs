//! The label brush and the clean-up of the labels.
//!
//! `aligners.preparation.segmentation.manual-brush` paints or erases tooth
//! labels on the scan at full resolution (`PaintSession` and
//! `oa.label_assign` of the plugin): on explicit vertices, under a sphere
//! (a brush dab) or along a stroke of dabs, or on the whole crown around a
//! vertex up to its margin (`fill`, the Ctrl+click of the brush).
//! `aligners.preparation.segmentation.clean-borders` removes the islands and
//! the holes and smooths the borders (`oa.label_clean`).
//!
//! The labels are stored as the labels blob of the arch (the scan with one
//! FDI per vertex, `0` = gingiva); a tooth whose region changes after its
//! solid was built is marked to be rebuilt (`AlignersState::dirty`).

use std::collections::{BTreeMap, BTreeSet};

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use dca_algo::manual::{self, FillParams, Scan};
use dca_algo::segmentation;
use dca_model::AlignersState;
use glam::DVec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::common::{
    arch_labels, arch_name, arch_to_case, data_of, fdi_of, finish, invalid, label_of, pick_arch,
    point, store_labels,
};

/// What the brush does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrushMode {
    /// The target vertices get the label (click of the brush).
    #[default]
    Paint,
    /// The target vertices go back to the gingiva; with a label, only those
    /// that have it (Shift+click).
    Erase,
    /// The crown around a vertex, up to its margin, gets the label (Ctrl+click).
    Fill,
}

/// Parameters of the label brush. The target is the union of `vertices`, the
/// dab at `centre` and the stroke through `centre` and `stroke`; `fill`
/// needs `vertex` (or `centre`, whose nearest vertex is used).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BrushParams {
    /// Arch to paint (`upper` or `lower`); the arch of `label`, else the
    /// only arch that has a scan.
    pub arch: Option<Arch>,
    /// Tooth number (FDI) to paint, `0` for the gingiva.
    pub label: u8,
    /// `paint` (default), `erase` or `fill`.
    pub mode: BrushMode,
    /// Vertex indices of the scan to paint.
    pub vertices: Vec<u32>,
    /// Centre of a brush dab, in case coordinates (mm).
    pub centre: Option<[f64; 3]>,
    /// Radius of the dab (mm, default 1).
    pub radius_mm: Option<f64>,
    /// More points of a stroke: the brush moves from `centre` through them,
    /// a dab every half radius.
    pub stroke: Vec<[f64; 3]>,
    /// Direction the viewer looks along: the surface that looks away from
    /// the viewer is not painted. Not given: every vertex in the ball.
    pub view_dir: Option<[f64; 3]>,
    /// Vertex the fill starts from.
    pub vertex: Option<u32>,
    /// Paint only the vertices that have this label now (`0` = only the
    /// gingiva).
    pub only_over: Option<u8>,
}

/// Parameters of the clean-up of the labels.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CleanParams {
    /// Arch to clean (`upper` or `lower`); all the segmented arches when
    /// absent.
    pub arch: Option<Arch>,
    /// Tooth to clean; all the teeth when absent.
    pub fdi: Option<u8>,
    /// Rings of smoothing of the border (0 to 3, default 1).
    pub rings: usize,
}

impl Default for CleanParams {
    fn default() -> Self {
        Self {
            arch: None,
            fdi: None,
            rings: 1,
        }
    }
}

const DEFAULT_RADIUS_MM: f64 = 1.0;
const MAX_RADIUS_MM: f64 = 20.0;

fn changed_teeth(old: &[u16], new: &[u16]) -> BTreeSet<Fdi> {
    old.iter()
        .zip(new)
        .filter(|(a, b)| a != b)
        .flat_map(|(a, b)| [*a, *b])
        .filter(|&l| l != 0)
        .filter_map(|l| u8::try_from(l).ok().and_then(|n| Fdi::new(n).ok()))
        .collect()
}

/// Marks the teeth whose region changed and that have a solid or a frame as
/// to rebuild.
fn mark_dirty(data: &mut AlignersState, changed: &BTreeSet<Fdi>) -> Vec<u8> {
    let mut dirty = Vec::new();
    for &f in changed {
        if data.tooth_frames.contains_key(&f) || data.crowns.contains_key(&f) {
            data.dirty.insert(f);
            dirty.push(f.get());
        }
    }
    dirty
}

pub(crate) fn run_brush(
    state: &CaseState,
    p: BrushParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let value = label_of(p.label)?;
    let fdi = (value != 0).then(|| fdi_of(p.label)).transpose()?;
    let only_over = p.only_over.map(label_of).transpose()?;
    let radius = p.radius_mm.unwrap_or(DEFAULT_RADIUS_MM);
    if !(radius.is_finite() && radius > 0.0 && radius <= MAX_RADIUS_MM) {
        return Err(invalid(format!(
            "radius_mm must be between 0 and {MAX_RADIUS_MM} mm"
        )));
    }
    let mut data = data_of(state)?;
    if let Some(f) = fdi
        && data.missing.contains(&f)
    {
        return Err(invalid(format!(
            "tooth {f} is marked missing: mark it present first"
        )));
    }
    let arch = pick_arch(state, p.arch, fdi)?;
    let al = arch_labels(state, &data, arch)?;
    let vertices = al.vertices();
    let n = vertices.len();
    if p.vertices.iter().any(|&v| v as usize >= n) {
        return Err(invalid(format!(
            "a vertex index is out of range (the {} scan has {n} vertices)",
            arch_name(arch)
        )));
    }
    let scan = Scan::new(&vertices, &al.scan.tris).map_err(|e| invalid(e.to_string()))?;
    let view_dir = p.view_dir.map(point).transpose()?;

    // the target vertices
    let mut target = vec![false; n];
    for &v in &p.vertices {
        target[v as usize] = true;
    }
    let centre = p.centre.map(point).transpose()?;
    let stroke_points: Vec<DVec3> = p
        .stroke
        .iter()
        .copied()
        .map(point)
        .collect::<Result<_, _>>()?;
    let mut fill_from = None;
    match p.mode {
        BrushMode::Fill => {
            let v = match (p.vertex, centre) {
                (Some(v), _) if (v as usize) < n => v,
                (Some(v), _) => return Err(invalid(format!("vertex {v} is out of range"))),
                (None, Some(c)) => scan
                    .nearest(c)
                    .ok_or_else(|| invalid("the scan has no vertices"))?,
                (None, None) => {
                    return Err(invalid("fill needs a vertex (or a centre) to start from"));
                }
            };
            fill_from = Some(v);
        }
        BrushMode::Paint | BrushMode::Erase => {
            if let Some(c) = centre {
                let mut points = vec![c];
                points.extend(stroke_points.iter().copied());
                for v in scan.stroke(&points, radius, view_dir) {
                    target[v as usize] = true;
                }
            } else if !stroke_points.is_empty() {
                return Err(invalid("a stroke needs a centre as its first point"));
            }
            if p.vertex.is_some() {
                return Err(invalid("vertex is only used by the fill mode"));
            }
        }
    }
    if let Some(v) = fill_from {
        // the occlusal direction in case coordinates: the arch frame of the seeds, else the
        // opposite of the view
        let up = match arch_to_case(&data, arch, &vertices) {
            Ok(m) => m.z_axis.truncate(),
            Err(e) => match view_dir {
                Some(d) => -d,
                None => return Err(e),
            },
        };
        for i in manual::fill_region(&scan, &al.labels, v, value, up, &FillParams::default()) {
            target[i as usize] = true;
        }
    } else if p.mode == BrushMode::Paint && !target.iter().any(|&t| t) {
        return Err(invalid(
            "nothing to paint: give vertices, or a centre (and radius_mm)",
        ));
    }

    // the new labels
    let new_value = if p.mode == BrushMode::Erase { 0 } else { value };
    let only = match (p.mode, only_over) {
        (BrushMode::Erase, None) if value != 0 => Some(value),
        (_, o) => o,
    };
    let mut labels = al.labels.clone();
    for (i, t) in target.iter().enumerate() {
        if *t && only.is_none_or(|o| labels[i] == o) {
            labels[i] = new_value;
        }
    }
    let changed = labels
        .iter()
        .zip(&al.labels)
        .filter(|(a, b)| a != b)
        .count();
    let teeth = changed_teeth(&al.labels, &labels);
    let mut next = state.clone();
    let mut stale = Vec::new();
    let mut blob = None;
    let mut dirty = Vec::new();
    if changed > 0 || !al.had_labels {
        blob = Some(store_labels(
            &mut next, &mut data, &mut stale, arch, &al.scan, labels,
        ));
        dirty = mark_dirty(&mut data, &teeth);
    }
    finish(&mut next, data, &stale);
    Ok((
        next,
        json!({
            "arch": arch_name(arch),
            "label": p.label,
            "mode": p.mode,
            "targeted_vertices": target.iter().filter(|&&t| t).count(),
            "changed_vertices": changed,
            "changed_teeth": teeth.iter().map(|f| f.get()).collect::<Vec<_>>(),
            "to_rebuild": dirty,
            "labels_blob": blob.map(|b| b.to_hex()),
        }),
    ))
}

pub(crate) fn run_clean(
    state: &CaseState,
    p: CleanParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    if p.rings > 3 {
        return Err(invalid("rings must be between 0 and 3"));
    }
    let fdi = p.fdi.filter(|&n| n != 0).map(fdi_of).transpose()?;
    let mut data = data_of(state)?;
    let arches: Vec<Arch> = match (p.arch, fdi) {
        (Some(a), _) => vec![pick_arch(state, Some(a), fdi)?],
        (None, Some(f)) => vec![Arch::of(f)],
        (None, None) => [Arch::Upper, Arch::Lower]
            .into_iter()
            .filter(|a| data.labels.contains_key(&a.role()) && state.case.scan(a.role()).is_some())
            .collect(),
    };
    if arches.is_empty() {
        return Err(invalid(
            "the scan is not segmented: paint or segment the teeth first",
        ));
    }
    let mut next = state.clone();
    let mut stale = Vec::new();
    let mut report = serde_json::Map::new();
    for arch in arches {
        ctx.check_cancelled()?;
        let al = arch_labels(state, &data, arch)?;
        if !al.had_labels {
            return Err(invalid(format!(
                "the {} scan is not segmented: paint or segment the teeth first",
                arch_name(arch)
            )));
        }
        let vertices = al.vertices();
        let scan = Scan::new(&vertices, &al.scan.tris).map_err(|e| invalid(e.to_string()))?;
        let seeds: Vec<(Fdi, DVec3)> = data
            .seeds
            .iter()
            .filter(|s| Arch::of(s.fdi) == arch)
            .map(|s| (s.fdi, s.position))
            .collect();
        let seed_vertex: BTreeMap<u16, usize> = segmentation::seed_vertices(&vertices, &seeds);
        let seed_vertex: BTreeMap<u16, u32> = seed_vertex
            .into_iter()
            .map(|(k, v)| (k, v as u32))
            .collect();
        let teeth: Option<Vec<u16>> = fdi.map(|f| vec![u16::from(f.get())]);
        let (labels, rep) = manual::clean_labels(
            &al.labels,
            scan.adjacency(),
            teeth.as_deref(),
            p.rings,
            &seed_vertex,
        );
        let changed = changed_teeth(&al.labels, &labels);
        let mut dirty = Vec::new();
        if rep.vertices > 0 {
            store_labels(&mut next, &mut data, &mut stale, arch, &al.scan, labels);
            dirty = mark_dirty(&mut data, &changed);
        }
        report.insert(
            arch_name(arch).to_owned(),
            json!({
                "islands": rep.islands,
                "changed_vertices": rep.vertices,
                "changed_teeth": changed.iter().map(|f| f.get()).collect::<Vec<_>>(),
                "to_rebuild": dirty,
            }),
        );
    }
    finish(&mut next, data, &stale);
    Ok((next, Value::Object(report)))
}
