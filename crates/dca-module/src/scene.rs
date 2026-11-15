//! The built teeth and gingiva as scene entries ([`built_scene`]), with the
//! gingiva deformed at a stage by its blend weights on the teeth.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use dc_case::{BlobRef, CaseState, Fdi, Pose, ScanRole};
use dc_mesh::TriMesh;
use dc_scene::{Color, LabelPalette, MeshColoring, SceneEntry, SceneItem};
use dca_algo::gingiva::{self, DeformPrep, GumSurface};
use dca_algo::teeth;
use dca_model::{AlignersState, MODULE_ID};
use glam::DMat4;

/// Colour of the built gingiva (as the Blender plugin).
pub const GINGIVA_COLOR: Color = Color::rgb(0.86, 0.45, 0.47);

/// Colour of the study models (dental stone).
pub const STUDY_MODEL_COLOR: Color = Color::rgb(0.90, 0.88, 0.82);

/// Gum weights kept in memory (a few arches and tooth sets).
const GUM_CACHE_SIZE: usize = 4;

/// The rest gingiva of an arch and the part of the deformation that does not
/// depend on the movement (`gum_weights` + `deform_prep`), with the weights
/// themselves (the aligner of the view blends them: `crate::aligner_scene`).
#[derive(Debug)]
pub(crate) struct GumPrep {
    gum: GumSurface,
    prep: DeformPrep,
    weights: gingiva::GumWeights,
}

impl GumPrep {
    /// The rest gingiva.
    pub(crate) fn gum(&self) -> &GumSurface {
        &self.gum
    }

    /// The deformation prepared from the weights.
    pub(crate) fn prep(&self) -> &DeformPrep {
        &self.prep
    }

    /// The weights of every vertex on the teeth.
    pub(crate) fn weights(&self) -> &gingiva::GumWeights {
        &self.weights
    }
}

/// A gingiva blob and the teeth its weights blend.
type GumKey = (BlobRef, Vec<Fdi>);

/// A computed entry (`None`: the mesh has no gingiva flags) or one being
/// computed.
#[derive(Debug, Clone)]
enum Slot {
    Pending,
    Ready(Option<Arc<GumPrep>>),
}

#[derive(Debug, Default)]
struct GumInner {
    entries: Mutex<Vec<(GumKey, Slot)>>,
    done: Condvar,
}

impl GumInner {
    fn lock(&self) -> MutexGuard<'_, Vec<(GumKey, Slot)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores the result of `key` (which is `Pending`) and wakes the waiters.
    fn finish(&self, key: &GumKey, prep: Option<Arc<GumPrep>>) {
        let mut entries = self.lock();
        if let Some((_, slot)) = entries.iter_mut().find(|(k, _)| k == key) {
            *slot = Slot::Ready(prep);
        }
        self.done.notify_all();
    }
}

/// What [`GumCache::get`] found.
pub(crate) enum Gum {
    /// Computed (`None`: the mesh is not a gingiva).
    Ready(Option<Arc<GumPrep>>),
    /// Being computed on another thread.
    Pending,
}

/// The expensive part of the deformation: `gum_weights` + `deform_prep`.
/// A panic counts as "not a gingiva", so a waiter is never left hanging.
fn compute(mesh: &TriMesh, fdis: &[Fdi]) -> Option<Arc<GumPrep>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        GumSurface::from_mesh(mesh).ok().map(|gum| {
            let weights = gingiva::gum_weights(&gum, fdis, &gingiva::Params::default());
            let prep = gingiva::deform_prep(&weights);
            Arc::new(GumPrep { gum, prep, weights })
        })
    }))
    .unwrap_or(None)
}

/// Gum weights by gingiva blob and the teeth they blend (the build does not
/// store them: they are computed when a stage first moves a tooth of the
/// arch, then reused). One computation per key, however many callers ask.
#[derive(Debug, Default)]
pub(crate) struct GumCache {
    inner: Arc<GumInner>,
}

impl GumCache {
    /// The weights of the gingiva `blob` (mesh `mesh`) on the teeth `fdis`.
    ///
    /// With `wait`, the caller waits for them (computing them itself when
    /// nobody does). Without, a missing entry is computed on a background
    /// thread and [`Gum::Pending`] is returned until it is ready.
    pub(crate) fn get(&self, blob: BlobRef, mesh: &Arc<TriMesh>, fdis: &[Fdi], wait: bool) -> Gum {
        let key = (blob, fdis.to_vec());
        let mut entries = self.inner.lock();
        loop {
            match entries.iter().find(|(k, _)| *k == key).map(|(_, s)| s) {
                Some(Slot::Ready(p)) => return Gum::Ready(p.clone()),
                Some(Slot::Pending) if wait => {
                    entries = self
                        .inner
                        .done
                        .wait(entries)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Some(Slot::Pending) => return Gum::Pending,
                None => break,
            }
        }
        if entries.len() >= GUM_CACHE_SIZE
            && let Some(i) = entries
                .iter()
                .position(|(_, s)| matches!(s, Slot::Ready(_)))
        {
            entries.remove(i);
        }
        entries.push((key.clone(), Slot::Pending));
        drop(entries);
        if wait {
            let prep = compute(mesh, fdis);
            self.inner.finish(&key, prep.clone());
            return Gum::Ready(prep);
        }
        let (inner, mesh, fdis) = (Arc::clone(&self.inner), Arc::clone(mesh), fdis.to_vec());
        let spawned = std::thread::Builder::new()
            .name("gum-weights".into())
            .spawn({
                let key = key.clone();
                move || {
                    let prep = compute(&mesh, &fdis);
                    inner.finish(&key, prep);
                }
            });
        if spawned.is_err() {
            self.inner.finish(&key, None);
        }
        Gum::Pending
    }
}

fn arch_of(fdi: Fdi) -> ScanRole {
    if fdi.is_upper() {
        ScanRole::Upper
    } else {
        ScanRole::Lower
    }
}

/// The built teeth and gingiva as scene entries (case coordinates), at
/// treatment stage `stage`.
///
/// - One entry per tooth in `crowns`: a pickable mesh coloured with the
///   [`LabelPalette`] colour of its FDI number, tagged with its arch and
///   FDI, and hiding the scan of its arch.
/// - One entry per arch in `gingiva`: uniform [`GINGIVA_COLOR`], tagged
///   with the arch (it hides no scan itself: the teeth of the arch do).
/// - One entry per arch in `study_models`: uniform [`STUDY_MODEL_COLOR`],
///   tagged with the arch, at rest whatever the stage (the cast of the
///   initial situation). It hides nothing itself: the viewer that draws it
///   (the "Study models" layer) draws it in place of the scan, the teeth
///   and the gingiva of its arch, which it overlaps.
/// - For `Some(n)`, `n > 0`, a tooth is moved by the pose of the stage with
///   index `n`, relative to its rest pose ([`teeth::rest_pose`]: the frame
///   of the tooth with its origin at the centre of resistance), that is by
///   `rest * pose * rest^-1`, as the setup, the staging and the export read
///   it; a tooth with no pose or frame stays.
///   The gingiva follows the moved teeth of its arch, blended with its gum
///   weights (`gingiva::deform`, as the plugin animates it): the entry
///   carries the deformed mesh.
///
/// The weights are slow the first time. With `wait`, they are computed
/// before returning; without, the gingiva of an arch whose weights are not
/// ready is left at rest (no mesh) and the result is flagged incomplete: ask
/// again later.
pub(crate) fn built_scene(
    state: &CaseState,
    stage: Option<u32>,
    gums: &GumCache,
    wait: bool,
) -> (Vec<SceneEntry>, bool) {
    let Some(data) = state.module_data::<AlignersState>(MODULE_ID) else {
        return (Vec::new(), true);
    };
    let mut complete = true;
    let palette = LabelPalette::default();
    let posed = stage
        .filter(|&n| n > 0)
        .and_then(|n| data.stages.iter().find(|s| s.index == n));
    let moves: BTreeMap<Fdi, Pose> = data
        .crowns
        .keys()
        .filter_map(|fdi| {
            let p = posed?.poses.get(fdi)?;
            let rest = teeth::rest_pose(data.tooth_frames.get(fdi)?);
            Some((*fdi, rest.compose(p).compose(&rest.inverse())))
        })
        .collect();
    let mut out = Vec::new();
    for (fdi, blob) in &data.crowns {
        out.push(
            SceneEntry::new(SceneItem::Mesh {
                blob: *blob,
                coloring: MeshColoring::Uniform(palette.color(u16::from(fdi.get()))),
                xform: moves.get(fdi).copied().unwrap_or(Pose::IDENTITY),
                pickable: true,
            })
            .with_arch(arch_of(*fdi))
            .with_tooth(fdi.get())
            .hiding_scan(),
        );
    }
    for (&arch, blob) in &data.gingiva {
        let mut e = SceneEntry::new(SceneItem::Mesh {
            blob: *blob,
            coloring: MeshColoring::Uniform(GINGIVA_COLOR),
            xform: Pose::IDENTITY,
            pickable: true,
        })
        .with_arch(arch);
        match deformed_gum(state, data, arch, *blob, &moves, gums, wait) {
            Deformed::Mesh(m) => e = e.with_mesh(Arc::new(m)),
            Deformed::Pending => complete = false,
            Deformed::Rest => {}
        }
        out.push(e);
    }
    for (&arch, blob) in &data.study_models {
        out.push(study_entry(arch, *blob));
    }
    (out, complete)
}

/// The scene entry of the study model `blob` of `arch`.
fn study_entry(arch: ScanRole, blob: BlobRef) -> SceneEntry {
    SceneEntry::new(SceneItem::Mesh {
        blob,
        coloring: MeshColoring::Uniform(STUDY_MODEL_COLOR),
        xform: Pose::IDENTITY,
        pickable: true,
    })
    .with_arch(arch)
}

/// `e` is the study model of its arch in `data`.
pub fn is_study_model(data: &AlignersState, e: &SceneEntry) -> bool {
    match &e.item {
        SceneItem::Mesh { blob, .. } => e
            .arch
            .is_some_and(|a| data.study_models.get(&a) == Some(blob)),
        _ => false,
    }
}

/// The entries a render draws for the "Study models" layer: `entries`
/// (the module's, with or without their study models) without any study
/// model when `show` is `false`; with `show`, each arch that has a study
/// model is drawn as its study model alone: the other entries of the arch
/// (3D teeth, gingiva) are left out and the study model hides its scan.
pub fn study_models_drawn(
    state: &CaseState,
    mut entries: Vec<SceneEntry>,
    show: bool,
) -> Vec<SceneEntry> {
    let Some(data) = state.module_data::<AlignersState>(MODULE_ID) else {
        return entries;
    };
    entries.retain(|e| !is_study_model(data, e));
    if !show {
        return entries;
    }
    let study: Vec<(ScanRole, BlobRef)> = data
        .study_models
        .iter()
        .filter(|(_, b)| state.blobs.contains(b))
        .map(|(a, b)| (*a, *b))
        .collect();
    entries.retain(|e| e.arch.is_none_or(|a| !study.iter().any(|(s, _)| *s == a)));
    entries.extend(
        study
            .into_iter()
            .map(|(a, b)| study_entry(a, b).hiding_scan()),
    );
    entries
}

/// The gingiva of an arch at a stage.
enum Deformed {
    /// Drawn at rest: no tooth of the arch moves, or the mesh is not a
    /// gingiva.
    Rest,
    /// Its weights are being computed.
    Pending,
    Mesh(TriMesh),
}

/// The gingiva of `arch` moved with the teeth in `moves`.
fn deformed_gum(
    state: &CaseState,
    data: &AlignersState,
    arch: ScanRole,
    blob: BlobRef,
    moves: &BTreeMap<Fdi, Pose>,
    gums: &GumCache,
    wait: bool,
) -> Deformed {
    let deltas: BTreeMap<Fdi, DMat4> = moves
        .iter()
        .filter(|(f, p)| arch_of(**f) == arch && **p != Pose::IDENTITY)
        .map(|(f, p)| (*f, p.to_mat4()))
        .collect();
    if deltas.is_empty() {
        return Deformed::Rest;
    }
    let Some(mesh) = state.mesh(&blob) else {
        return Deformed::Rest;
    };
    let fdis: Vec<Fdi> = data
        .crowns
        .keys()
        .copied()
        .filter(|f| arch_of(*f) == arch)
        .collect();
    match gums.get(blob, mesh, &fdis, wait) {
        Gum::Pending => Deformed::Pending,
        Gum::Ready(None) => Deformed::Rest,
        Gum::Ready(Some(g)) => {
            let moved = gingiva::deform(&g.gum.vertices, &g.prep, &deltas);
            Deformed::Mesh(TriMesh::new(
                moved.iter().map(|p| p.as_vec3()).collect(),
                g.gum.tris.clone(),
            ))
        }
    }
}
