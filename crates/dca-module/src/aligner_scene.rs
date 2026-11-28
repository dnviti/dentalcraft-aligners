//! The aligner of the 3D view as scene entries (`aligners.view.aligner`; the
//! plugin's `update_aligner_preview`, `animation.py:1223`).
//!
//! While the case shows the aligner, every arch with aligners draws, at the
//! stage shown (an arch past its last aligner: its last one), the aligner
//! built at that stage, or else the nearest one built for the case as it is,
//! moved with the teeth and the gingiva from its stage to the stage shown
//! ([`dca_algo::aligner_view`]). An aligner built for an earlier design,
//! teeth or plan is not drawn. The entry is translucent (the plugin's
//! `OA_Allineatore` material), not pickable, and hides nothing.
//!
//! Binding a built aligner to the model (the nearest points of the gingiva
//! and of the teeth of each vertex) takes a moment: it is done once per
//! aligner and kept ([`BindCache`]); without `wait` it is done on a
//! background thread and the arch is left out until it is ready.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use dc_case::{BlobRef, CaseState, Fdi, Pose};
use dc_mesh::TriMesh;
use dc_scene::{Color, MeshColoring, SceneEntry, SceneItem};
use dca_algo::aligner_view::{self as av, Binding};
use dca_algo::anatomy::Arch;
use dca_algo::gingiva;
use dca_model::{AlignerBase, AlignersState, MODULE_ID, view_key};
use glam::DMat4;

use crate::commands::aligner_view::{arch_stages, stage_deltas};
use crate::scene::{Gum, GumCache, GumPrep};

/// Colour of the aligner in the view: the plugin's material
/// `OA_Allineatore`, `(0.72, 0.9, 1.0, 0.35)` (`animation.py:826`).
pub const ALIGNER_COLOR: Color = Color::rgba(0.72, 0.9, 1.0, 0.35);

/// Bindings kept in memory (a few aligners of each arch).
const BIND_CACHE_SIZE: usize = 8;

/// A built aligner (its mesh) and the key of the case it is bound for.
type BindKey = (BlobRef, String);

#[derive(Debug, Clone)]
enum Slot {
    Pending,
    Ready(Option<Arc<Binding>>),
}

#[derive(Debug, Default)]
struct Inner {
    entries: Mutex<Vec<(BindKey, Slot)>>,
    done: Condvar,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Vec<(BindKey, Slot)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(&self, key: &BindKey, b: Option<Arc<Binding>>) {
        let mut entries = self.lock();
        if let Some((_, slot)) = entries.iter_mut().find(|(k, _)| k == key) {
            *slot = Slot::Ready(b);
        }
        self.done.notify_all();
    }
}

/// The bindings of the aligners of the view, by aligner and case key; one
/// computation per key however many callers ask (as `scene::GumCache`).
#[derive(Debug, Default)]
pub(crate) struct BindCache {
    inner: Arc<Inner>,
}

/// What the stage shown draws of a built aligner.
enum Moved {
    Mesh(TriMesh),
    /// Its binding (or the gum weights it needs) is being computed.
    Pending,
    /// It cannot be moved: left out.
    Failed,
}

/// What a binding is computed from (owned: it may go to another thread).
struct BindInput {
    gum: Arc<GumPrep>,
    crowns: Vec<(Fdi, Arc<TriMesh>)>,
    fdis: Vec<Fdi>,
    base: BTreeMap<Fdi, DMat4>,
    mesh: Arc<TriMesh>,
}

/// The binding of a built aligner to the model at the stage it was built at
/// (`_bind`): the points of the gingiva moved to that stage with their own
/// weights, then the points of every closed tooth posed at that stage, all
/// of it to that tooth. `None` when the model does not fit (the gingiva of
/// other teeth).
fn compute(input: &BindInput) -> Option<Binding> {
    let nf = input.fdis.len();
    let w = input.gum.weights();
    if w.fdis != input.fdis {
        return None;
    }
    // the gingiva as built on the scan: its base (after it, weighing nothing)
    // stays out
    let gum = input.gum.gum();
    let scanned = gum.scanned_vertices();
    let mut points = gingiva::deform(&gum.vertices[..scanned], input.gum.prep(), &input.base);
    let mut weights = w.w[..scanned * nf].to_vec();
    for (f, mesh) in &input.crowns {
        let j = input.fdis.iter().position(|x| x == f)?;
        let d = input.base.get(f).copied().unwrap_or(DMat4::IDENTITY);
        points.extend(
            mesh.positions
                .iter()
                .map(|p| d.transform_point3(p.as_dvec3())),
        );
        let mut row = vec![0.0f32; nf];
        row[j] = 1.0;
        for _ in 0..mesh.positions.len() {
            weights.extend_from_slice(&row);
        }
    }
    let model = av::Model {
        fdis: &input.fdis,
        points: &points,
        weights: &weights,
    };
    av::bind(&input.mesh.positions, &model).ok()
}

impl BindCache {
    /// The binding of `key`, computed from `input` when nobody has: at once
    /// with `wait`, else on a background thread (`None`: pending).
    fn get(
        &self,
        key: BindKey,
        input: impl FnOnce() -> Option<BindInput>,
        wait: bool,
    ) -> Option<Option<Arc<Binding>>> {
        let mut entries = self.inner.lock();
        loop {
            match entries.iter().find(|(k, _)| *k == key).map(|(_, s)| s) {
                Some(Slot::Ready(b)) => return Some(b.clone()),
                Some(Slot::Pending) if wait => {
                    entries = self
                        .inner
                        .done
                        .wait(entries)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Some(Slot::Pending) => return None,
                None => break,
            }
        }
        let Some(input) = input() else {
            return Some(None);
        };
        if entries.len() >= BIND_CACHE_SIZE
            && let Some(i) = entries
                .iter()
                .position(|(_, s)| matches!(s, Slot::Ready(_)))
        {
            entries.remove(i);
        }
        entries.push((key.clone(), Slot::Pending));
        drop(entries);
        let run = |input: &BindInput| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(input)))
                .unwrap_or(None)
                .map(Arc::new)
        };
        if wait {
            let b = run(&input);
            self.inner.finish(&key, b.clone());
            return Some(b);
        }
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("aligner-binding".into())
            .spawn({
                let key = key.clone();
                move || {
                    let b = run(&input);
                    inner.finish(&key, b);
                }
            });
        if spawned.is_err() {
            self.inner.finish(&key, None);
        }
        None
    }
}

/// What moving a built aligner reads besides the aligner and the stage.
struct Moving<'a> {
    state: &'a CaseState,
    data: &'a AlignersState,
    /// The key of the case as it is.
    case_key: &'a str,
    gums: &'a GumCache,
    binds: &'a BindCache,
    wait: bool,
}

/// The aligner `base` (mesh `mesh`) moved from its stage to stage `k`.
fn moved(c: &Moving<'_>, base: &AlignerBase, k: u32, mesh: &Arc<TriMesh>) -> Moved {
    let (state, data, wait) = (c.state, c.data, c.wait);
    let arch = if base.arch == dc_case::ScanRole::Upper {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let fdis: Vec<Fdi> = data
        .crowns
        .keys()
        .copied()
        .filter(|f| Arch::of(*f) == arch)
        .collect();
    let (Some(gum_blob), false) = (data.gingiva.get(&base.arch), fdis.is_empty()) else {
        return Moved::Failed;
    };
    let Some(gum_mesh) = state.mesh(gum_blob) else {
        return Moved::Failed;
    };
    let gum = match c.gums.get(*gum_blob, gum_mesh, &fdis, wait) {
        Gum::Pending => return Moved::Pending,
        Gum::Ready(None) => return Moved::Failed,
        Gum::Ready(Some(g)) => g,
    };
    let at_base = stage_deltas(data, arch, base.stage);
    let input = || {
        let crowns = fdis
            .iter()
            .map(|f| Some((*f, Arc::clone(state.mesh(data.crowns.get(f)?)?))))
            .collect::<Option<Vec<_>>>()?;
        Some(BindInput {
            gum,
            crowns,
            fdis: fdis.clone(),
            base: at_base.clone(),
            mesh: Arc::clone(mesh),
        })
    };
    let binding = match c.binds.get((base.mesh, c.case_key.to_owned()), input, wait) {
        None => return Moved::Pending,
        Some(None) => return Moved::Failed,
        Some(Some(b)) => b,
    };
    let rel = av::relative(&stage_deltas(data, arch, k), &at_base);
    Moved::Mesh(TriMesh::new(av::deform(&binding, &rel), mesh.tris.clone()))
}

/// `e` is an aligner entry of this module's scene.
pub(crate) fn is_aligner(e: &SceneEntry) -> bool {
    matches!(
        &e.item,
        SceneItem::Mesh { coloring: MeshColoring::Uniform(c), pickable: false, .. } if *c == ALIGNER_COLOR
    )
}

/// The aligner of every arch at stage `stage` (`None`: the initial
/// position) while the case shows it; the flag is `false` when an arch is
/// left out until its binding is ready (only without `wait`).
pub(crate) fn entries(
    state: &CaseState,
    stage: Option<u32>,
    gums: &GumCache,
    binds: &BindCache,
    wait: bool,
) -> (Vec<SceneEntry>, bool) {
    let Some(data) = state.module_data::<AlignersState>(MODULE_ID) else {
        return (Vec::new(), true);
    };
    let view = &data.aligner_view;
    if !view.shown || view.bases.is_empty() {
        return (Vec::new(), true);
    }
    let case_key = view_key(state, data);
    let ctx = Moving {
        state,
        data,
        case_key: &case_key,
        gums,
        binds,
        wait,
    };
    let mut out = Vec::new();
    let mut complete = true;
    for (arch, k) in arch_stages(data, stage.unwrap_or(0)) {
        let Some(base) = view.nearest(arch.role(), &case_key, k) else {
            continue;
        };
        let Some(mesh) = state.mesh(&base.mesh) else {
            continue;
        };
        let mut e = SceneEntry::new(SceneItem::Mesh {
            blob: base.mesh,
            coloring: MeshColoring::Uniform(ALIGNER_COLOR),
            xform: Pose::IDENTITY,
            pickable: false,
        })
        .with_arch(arch.role());
        if base.stage != k {
            match moved(&ctx, base, k, mesh) {
                Moved::Mesh(m) => e = e.with_mesh(Arc::new(m)),
                Moved::Pending => {
                    complete = false;
                    continue;
                }
                Moved::Failed => continue,
            }
        }
        out.push(e);
    }
    (out, complete)
}

/// The vertices of the aligner of `arch` at stage `k` as the view draws it
/// (for tests and the oracle): the base at its own stage, else the nearest
/// one moved; `None` when the case shows none.
#[cfg(test)]
pub(crate) fn drawn(state: &CaseState, arch: Arch, k: u32) -> Option<Vec<glam::DVec3>> {
    let (entries, _) = entries(
        state,
        Some(k),
        &GumCache::default(),
        &BindCache::default(),
        true,
    );
    let e = entries.into_iter().find(|e| e.arch == Some(arch.role()))?;
    let mesh = e.resolve_mesh(|b| state.blobs.get(b))?;
    Some(mesh.positions.iter().map(|p| p.as_dvec3()).collect())
}
