//! Gingiva: the gum surface without holes and its deformation driven by
//! the tooth movements.
//!
//! Reference: `ortho_aligner/reconstruct.py` (`gum_surface`,
//! `_solve_membrane`, `interdental`, `cotan_weights`, `gum_weights`,
//! `deform_prep`, `deform`) for the closed gingiva of the model, and
//! `ortho_aligner/gingiva.py` (`margin_vertices`, `distance_matrix`,
//! `blend_weights`, `deform`) for the deformation of the whole scan between
//! stages (simulated control scans).
//!
//! The gingiva keeps the scan surface; under the crowns it continues with a
//! harmonic membrane (the ridge under the teeth, and the interdental col
//! visible between teeth that move apart), lowered by up to
//! [`Params::col_mm`]. Its vertices follow the teeth with harmonic blend
//! weights (Joshi et al. 2007, cotangent Laplacian): fixed on the margins
//! (blended between teeth whose margins are within [`Params::papilla_mm`]),
//! on the soft tissue down to the alveolar crest where only one tooth is
//! near, and to zero far from the teeth; within [`Params::root_mm`] of the
//! margins the weights of a vertex sum to one.
//!
//! The scan deformation of `gingiva.py` moves the crown vertices rigidly
//! with their tooth and blends the tooth displacements on the gingiva with
//! inverse-distance weights (Shepard 1968) times a compactly supported
//! Wendland C2 function (Wendland 1995).
//!
//! Everything is in arch coordinates (occlusal = `+Z`).

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::sparse::{CsrMatrix, PcgParams, pcg_solve_many};
use dc_mesh::TriMesh;
use glam::{DMat4, DVec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::evidence;
use crate::reconstruct::geom::{self, KdTree, unique_edges};
use crate::reconstruct::solve;
use crate::reconstruct::{APEX_MM, SMOOTH_ITER, T_MIN};

/// Membrane under the crowns / interdental col lowered up to this depth, mm.
pub const COL_MM: f64 = 0.5;
/// Margin blended with every other tooth whose margin is nearer than this, mm.
pub const PAPILLA_MM: f64 = 3.0;
/// The collar follows its tooth rigidly only where the next tooth's margin
/// is this much farther than its own, mm.
pub const COLLAR_GAP_MM: f64 = 6.0;
/// Beyond the collar, the gingiva stops following the teeth this far, mm.
pub const GUM_RADIUS: f64 = 8.0;
/// Back to the harmonic weights over this much beyond [`Params::root_mm`], mm.
pub const ROOT_FADE_MM: f64 = 3.0;
/// Cotangent edge weights are clamped to `[COT_MIN, COT_MAX]`.
pub const COT_MIN: f64 = 0.05;
/// See [`COT_MIN`].
pub const COT_MAX: f64 = 10.0;
/// Smaller gingiva weights move no vertex.
pub const DEFORM_EPS: f32 = 1e-5;
/// Version of the gingiva weights (`GUM_WEIGHTS_VERSION` of the plugin).
pub const GUM_WEIGHTS_VERSION: u32 = 3;

/// Vertex label flag of a stored gingiva mesh: vertex of the margin cut.
pub const FLAG_MARGIN: u16 = 0x4000;
/// Vertex label flag of a stored gingiva mesh: membrane under the crowns.
pub const FLAG_MEMBRANE: u16 = 0x8000;
/// Vertex label flag of a stored gingiva mesh: interdental col.
pub const FLAG_COL: u16 = 0x2000;
/// Mask of the tooth number in a stored vertex label.
pub const TOOTH_MASK: u16 = 0x00ff;

/// Errors of the gingiva stage.
#[derive(Debug, thiserror::Error)]
pub enum GingivaError {
    /// The input is not usable.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The caller asked to stop; nothing was produced.
    #[error("cancelled")]
    Cancelled,
}

/// Parameters of the gingiva.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to work on (the command uses it).
    pub arch: crate::teeth::Which,
    /// Depth the membrane under the crowns (and the interdental col) is
    /// lowered to, mm.
    pub col_mm: f64,
    /// Margins nearer than this blend their teeth (the papilla follows
    /// both), mm.
    pub papilla_mm: f64,
    /// Soft tissue from the margin to the alveolar crest, following its tooth
    /// rigidly, mm (Gargiulo 1961).
    pub collar_mm: f64,
    /// The collar is rigid only where the next tooth's margin is this much
    /// farther, mm.
    pub collar_gap_mm: f64,
    /// Beyond the collar the gingiva stops following the teeth this far, mm.
    pub radius_mm: f64,
    /// Within this of the margins the weights of a vertex sum to one, mm.
    pub root_mm: f64,
    /// Fade back to the smooth weights over this, mm.
    pub root_fade_mm: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: crate::teeth::Which::All,
            col_mm: COL_MM,
            papilla_mm: PAPILLA_MM,
            collar_mm: evidence::param_f64("margin_to_crest").unwrap_or(2.73),
            collar_gap_mm: COLLAR_GAP_MM,
            radius_mm: GUM_RADIUS,
            root_mm: APEX_MM,
            root_fade_mm: ROOT_FADE_MM,
        }
    }
}

/// The gingiva of a whole scan without holes (`gum_surface`).
#[derive(Debug, Clone, PartialEq)]
pub struct GumSurface {
    /// Rest positions: the scan vertices (membrane ones moved), then the
    /// vertices of the margin cut.
    pub vertices: Vec<DVec3>,
    /// Triangles (the whole scan, cut along the margin).
    pub tris: Vec<[u32; 3]>,
    /// Per vertex: FDI number of the tooth it moves with at the margin and
    /// under the crown, `0` for the free gingiva.
    pub tooth: Vec<u16>,
    /// Vertices of the margin cut (shared with the crowns).
    pub margin: Vec<bool>,
    /// Vertices of the membrane under the crowns.
    pub membrane: Vec<bool>,
    /// The part of the membrane between two crowns that no tooth covers.
    pub col: Vec<bool>,
}

impl GumSurface {
    /// The surface as a mesh whose vertex labels hold the tooth number and
    /// the [`FLAG_MARGIN`], [`FLAG_MEMBRANE`] and [`FLAG_COL`] flags.
    pub fn to_mesh(&self) -> TriMesh {
        let mut m = TriMesh::new(
            self.vertices.iter().map(|p| p.as_vec3()).collect(),
            self.tris.clone(),
        );
        m.vertex_labels = Some(
            (0..self.vertices.len())
                .map(|i| {
                    (self.tooth[i] & TOOTH_MASK)
                        | if self.margin[i] { FLAG_MARGIN } else { 0 }
                        | if self.membrane[i] { FLAG_MEMBRANE } else { 0 }
                        | if self.col[i] { FLAG_COL } else { 0 }
                })
                .collect(),
        );
        m
    }

    /// The surface back from a mesh written by [`GumSurface::to_mesh`]
    /// (single-precision positions).
    pub fn from_mesh(m: &TriMesh) -> Result<Self, GingivaError> {
        let labels = m
            .vertex_labels
            .as_ref()
            .filter(|l| l.len() == m.vertex_count())
            .ok_or_else(|| GingivaError::Invalid("the gingiva mesh has no vertex flags".into()))?;
        Ok(Self {
            vertices: m.positions.iter().map(|p| p.as_dvec3()).collect(),
            tris: m.tris.clone(),
            tooth: labels.iter().map(|l| l & TOOTH_MASK).collect(),
            margin: labels.iter().map(|l| l & FLAG_MARGIN != 0).collect(),
            membrane: labels.iter().map(|l| l & FLAG_MEMBRANE != 0).collect(),
            col: labels.iter().map(|l| l & FLAG_COL != 0).collect(),
        })
    }
}

fn check(v: &[DVec3], tris: &[[u32; 3]], labels: &[u16]) -> Result<(), GingivaError> {
    if labels.len() != v.len() {
        return Err(GingivaError::Invalid(format!(
            "{} labels for {} vertices",
            labels.len(),
            v.len()
        )));
    }
    if tris.iter().flatten().any(|&i| i as usize >= v.len()) {
        return Err(GingivaError::Invalid(
            "a triangle refers to a missing vertex".to_owned(),
        ));
    }
    Ok(())
}

/// Gives unassigned vertices (`None`) the most frequent label of their
/// assigned neighbours, ties to the smallest label, repeatedly
/// (`mu.fill_unassigned`).
fn fill_unassigned(labels: &mut [Option<u16>], edges: &[[u32; 2]], max_iter: usize) {
    for _ in 0..max_iter {
        if labels.iter().all(|l| l.is_some()) {
            break;
        }
        let mut votes: BTreeMap<u32, BTreeMap<u16, u32>> = BTreeMap::new();
        for e in edges {
            let (a, b) = (e[0], e[1]);
            match (labels[a as usize], labels[b as usize]) {
                (None, Some(l)) => *votes.entry(a).or_default().entry(l).or_default() += 1,
                (Some(l), None) => *votes.entry(b).or_default().entry(l).or_default() += 1,
                _ => {}
            }
        }
        if votes.is_empty() {
            break;
        }
        for (i, cnt) in votes {
            let mut best = (0u16, 0u32);
            for (l, c) in cnt {
                if c > best.1 {
                    best = (l, c);
                }
            }
            labels[i as usize] = Some(best.0);
        }
    }
}

/// Vertices on the crown side of the gingival margin that belong to no
/// single crown: the interdental col (`interdental`); `n_total` entries.
pub fn interdental(labels: &[u16], edges: &[[u32; 2]], n_total: usize) -> Vec<bool> {
    let n = labels.len();
    let mut out = vec![false; n_total];
    let ind: Vec<f64> = labels
        .iter()
        .map(|&l| f64::from(u8::from(l != 0)))
        .collect();
    let any_side: Vec<bool> = geom::smooth_scalar(&ind, edges, SMOOTH_ITER, 0.5)
        .into_iter()
        .map(|x| x > 0.5)
        .collect();
    if !any_side.iter().any(|&x| x) {
        return out;
    }
    let mut fs: Vec<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
    fs.sort_unstable();
    fs.dedup();
    let smoothed: Vec<Vec<f64>> = fs
        .par_iter()
        .map(|&f| {
            let ind: Vec<f64> = labels
                .iter()
                .map(|&l| f64::from(u8::from(l == f)))
                .collect();
            geom::smooth_scalar(&ind, edges, SMOOTH_ITER, 0.5)
        })
        .collect();
    let mut best = vec![0.0f64; n];
    for s in &smoothed {
        for i in 0..n {
            best[i] = best[i].max(s[i]);
        }
    }
    for i in 0..n {
        out[i] = any_side[i] && best[i] <= 0.5;
    }
    out
}

/// Harmonic positions of the `inner` vertices (all others fixed), and the
/// solution `u` of `L u = 1` (`u = 0` on the fixed vertices), which grows
/// with the distance from the margin (`_solve_membrane`); `None` when
/// `cancelled` says so during the solve.
fn solve_membrane(
    vs: &[DVec3],
    edges: &[[u32; 2]],
    inner: &[bool],
    eps: f64,
    maxiter: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<(Vec<usize>, Vec<DVec3>, Vec<f64>)> {
    let n = vs.len();
    let deg: Vec<f64> = geom::edge_degree(edges, n)
        .into_iter()
        .map(|d| d as f64)
        .collect();
    let fidx: Vec<usize> = (0..n).filter(|&i| inner[i]).collect();
    let mut pos = vec![usize::MAX; n];
    for (k, &i) in fidx.iter().enumerate() {
        pos[i] = k;
    }
    let nf = fidx.len();
    let mut trip: Vec<(usize, usize, f64)> = Vec::new();
    for (k, &i) in fidx.iter().enumerate() {
        trip.push((k, k, deg[i] + eps));
    }
    let both: Vec<&[u32; 2]> = edges
        .iter()
        .filter(|e| inner[e[0] as usize] && inner[e[1] as usize])
        .collect();
    for e in &both {
        trip.push((pos[e[0] as usize], pos[e[1] as usize], -1.0));
    }
    for e in &both {
        trip.push((pos[e[1] as usize], pos[e[0] as usize], -1.0));
    }
    let mut b1 = vec![[0.0f64; 3]; nf];
    let mut b2 = vec![[0.0f64; 3]; nf];
    for e in edges {
        let (a, b) = (e[0] as usize, e[1] as usize);
        if inner[a] && !inner[b] {
            for c in 0..3 {
                b1[pos[a]][c] += vs[b][c];
            }
        }
    }
    for e in edges {
        let (a, b) = (e[0] as usize, e[1] as usize);
        if inner[b] && !inner[a] {
            for c in 0..3 {
                b2[pos[b]][c] += vs[a][c];
            }
        }
    }
    let bs: [Vec<f64>; 4] = std::array::from_fn(|c| {
        if c < 3 {
            (0..nf)
                .map(|k| (b1[k][c] + b2[k][c]) + eps * vs[fidx[k]][c])
                .collect()
        } else {
            vec![1.0; nf]
        }
    });
    let x0: [Vec<f64>; 4] = std::array::from_fn(|c| {
        if c < 3 {
            fidx.iter().map(|&i| vs[i][c]).collect()
        } else {
            vec![0.0; nf]
        }
    });
    let params = PcgParams {
        max_iterations: maxiter,
        tolerance: 1e-4,
    };
    // the four right-hand sides iterated together in parallel passes
    // (the same bits as `pcg_solve_many`)
    let a = solve::csr_from_triplets(nf, &trip);
    let sol = solve::pcg_solve_many_par(&a, &bs, &x0, &params, cancelled)?;
    let x = (0..nf)
        .map(|k| DVec3::new(sol[0][k], sol[1][k], sol[2][k]))
        .collect();
    Some((fidx, x, sol[3].clone()))
}

/// Gingiva of the whole scan without holes (`gum_surface`): the scan cut
/// along the smoothed margin of all the teeth; on the crown side a harmonic
/// membrane lowered by up to `col_mm` under the crowns.
pub fn gum_surface(
    v: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    params: &Params,
) -> Result<GumSurface, GingivaError> {
    gum_surface_with_progress(v, tris, labels, params, &|_| {}, &|| false)
}

/// [`gum_surface`], calling `progress(fraction)` (`0` to `1`) as its stages
/// end: the margin cut, the membrane solve (most of the time), the col.
/// `cancelled` is asked between the stages and every few iterations of the
/// membrane solve; once `true` the result is [`GingivaError::Cancelled`].
pub fn gum_surface_with_progress(
    v: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    params: &Params,
    progress: &(dyn Fn(f32) + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<GumSurface, GingivaError> {
    check(v, tris, labels)?;
    let stop = || {
        if cancelled() {
            Err(GingivaError::Cancelled)
        } else {
            Ok(())
        }
    };
    stop()?;
    let n = v.len();
    let edges = unique_edges(tris);
    let ind: Vec<f64> = labels
        .iter()
        .map(|&l| f64::from(u8::from(l != 0)))
        .collect();
    let phi: Vec<f64> = geom::smooth_scalar(&ind, &edges, SMOOTH_ITER, 0.5)
        .into_iter()
        .map(|x| x - 0.5)
        .collect();
    stop()?;
    let cut = geom::cut_mesh(v, tris, &phi, T_MIN);
    stop()?;
    let n2 = cut.vertices.len();
    // crown side: tooth of every vertex (scan gingiva engulfed by the smoothing takes its neighbours' tooth)
    let mut lab_c: Vec<Option<u16>> = (0..n)
        .map(|i| {
            if phi[i] > 0.0 {
                if labels[i] == 0 {
                    None
                } else {
                    Some(labels[i])
                }
            } else {
                Some(0)
            }
        })
        .collect();
    stop()?;
    if lab_c.iter().any(|l| l.is_none()) {
        fill_unassigned(&mut lab_c, &edges, 500);
    }
    let lab_c: Vec<u16> = lab_c.into_iter().map(|l| l.unwrap_or(0)).collect();
    let mut margin = vec![false; n2];
    margin[n..].fill(true);
    let mut tooth = lab_c.clone();
    for i in 0..cut.p.len() {
        let (p, q) = (cut.p[i] as usize, cut.q[i] as usize);
        tooth.push(lab_c[if phi[p] > 0.0 { p } else { q }]);
    }
    let mut membrane = vec![false; n2];
    for i in 0..n {
        membrane[i] = phi[i] > 0.0;
    }
    stop()?;
    let e2 = unique_edges(&cut.tris);
    let mut vr = cut.vertices.clone();
    if membrane.iter().any(|&m| m) {
        progress(0.1);
        let Some((fidx, x, u)) =
            solve_membrane(&cut.vertices, &e2, &membrane, 1e-4, 3000, cancelled)
        else {
            return Err(GingivaError::Cancelled);
        };
        // depth: u ~ (graph distance)^2 / 4 in edge units -> distance in mm
        let lens: Vec<f64> = e2
            .iter()
            .map(|e| (cut.vertices[e[0] as usize] - cut.vertices[e[1] as usize]).length())
            .collect();
        let el = geom::median(&lens);
        for (k, &i) in fidx.iter().enumerate() {
            let dist = 2.0 * u[k].max(0.0).sqrt() * el;
            let s = (dist / 2.0).clamp(0.0, 1.0);
            let mut p = x[k];
            p.z -= params.col_mm * s * s * (3.0 - 2.0 * s);
            vr[i] = p;
        }
    }
    stop()?;
    progress(0.85);
    let col: Vec<bool> = interdental(labels, &edges, n2)
        .into_iter()
        .zip(&membrane)
        .map(|(a, b)| a && *b)
        .collect();
    progress(1.0);
    Ok(GumSurface {
        vertices: vr,
        tris: cut.tris,
        tooth,
        margin,
        membrane,
        col,
    })
}

/// Cotangent weights `(cot a + cot b) / 2` of the `edges` (`unique_edges`
/// of the triangles), clamped to `[lo, hi]` (`cotan_weights`, float64).
pub fn cotan_weights(
    v: &[DVec3],
    tris: &[[u32; 3]],
    edges: &[[u32; 2]],
    lo: f64,
    hi: f64,
) -> Vec<f64> {
    let mut w = vec![0.0f64; edges.len()];
    let mut tmp = vec![0.0f64; edges.len()];
    for c in 0..3 {
        tmp.fill(0.0);
        for t in tris {
            let (i0, i1, i2) = (t[c], t[(c + 1) % 3], t[(c + 2) % 3]);
            let (u, x) = (
                v[i1 as usize] - v[i0 as usize],
                v[i2 as usize] - v[i0 as usize],
            );
            let cot = u.dot(x) / u.cross(x).length().max(1e-12);
            let key = [i1.min(i2), i1.max(i2)];
            if let Ok(k) = edges.binary_search(&key) {
                tmp[k] += cot;
            }
        }
        for (wk, tk) in w.iter_mut().zip(&tmp) {
            *wk += 0.5 * tk;
        }
    }
    w.into_iter().map(|x| x.clamp(lo, hi)).collect()
}

/// `(n_points, n_teeth)` distances to each tooth's margin samples, row-major
/// (infinite outside the box of the samples grown by `radius`, and for a
/// tooth without samples) (`gingiva.distance_matrix`).
pub fn distance_matrix(points: &[DVec3], samples: &[Vec<DVec3>], radius: f64) -> Vec<f64> {
    let nf = samples.len();
    let mut d = vec![f64::INFINITY; points.len() * nf];
    let cols: Vec<Vec<f64>> = samples
        .par_iter()
        .map(|s| {
            let mut col = vec![f64::INFINITY; points.len()];
            if s.is_empty() {
                return col;
            }
            let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
            for p in s {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            let (lo, hi) = (lo - radius, hi + radius);
            let tree = KdTree::new(s);
            for (i, p) in points.iter().enumerate() {
                if p.cmpge(lo).all() && p.cmple(hi).all() {
                    col[i] = tree.nearest(*p).map_or(f64::INFINITY, |x| x.1);
                }
            }
            col
        })
        .collect();
    for (j, col) in cols.iter().enumerate() {
        for (i, x) in col.iter().enumerate() {
            d[i * nf + j] = *x;
        }
    }
    d
}

/// Blend weights of the gingiva on the teeth (`gum_weights`).
#[derive(Debug, Clone, PartialEq)]
pub struct GumWeights {
    /// The teeth, in column order.
    pub fdis: Vec<Fdi>,
    /// Weights, `(n_vertices, n_teeth)` row-major; the rows sum to at most 1.
    pub w: Vec<f32>,
}

impl GumWeights {
    /// Weight of vertex `i` on the tooth of column `j`.
    pub fn get(&self, i: usize, j: usize) -> f32 {
        self.w[i * self.fdis.len() + j]
    }
}

fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

/// Harmonic blend weights of the gingiva on the teeth `fdis`
/// (`gum_weights`, version [`GUM_WEIGHTS_VERSION`]).
pub fn gum_weights(g: &GumSurface, fdis: &[Fdi], params: &Params) -> GumWeights {
    let v = &g.vertices;
    let n = v.len();
    let nf = fdis.len();
    let fs: Vec<u16> = fdis.iter().map(|f| u16::from(f.get())).collect();
    let col_of = |t: u16| fs.iter().position(|&f| f == t);
    let mut w = vec![0.0f32; n * nf];
    // margin samples of every tooth
    let samples: Vec<Vec<DVec3>> = fs
        .iter()
        .map(|&f| {
            let idx: Vec<usize> = (0..n).filter(|&i| g.margin[i] && g.tooth[i] == f).collect();
            geom::subsample(&idx, 300)
                .into_iter()
                .map(|i| v[i])
                .collect()
        })
        .collect();
    let far = params.collar_mm + params.radius_mm;
    let dfull = distance_matrix(v, &samples, far + 1.0);
    let dmin: Vec<f64> = (0..n)
        .map(|i| {
            dfull[i * nf..(i + 1) * nf]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
        })
        .collect();
    // Dirichlet data: the margin follows its tooth, blended with every other tooth near its margin
    let rig: Vec<usize> = (0..n)
        .filter(|&i| g.margin[i] && col_of(g.tooth[i]).is_some())
        .collect();
    let mt: Vec<u16> = rig.iter().map(|&i| g.tooth[i]).collect();
    let mut al = vec![0.0f64; rig.len() * nf];
    for (j, &f) in fs.iter().enumerate() {
        let own: Vec<DVec3> = rig
            .iter()
            .zip(&mt)
            .filter(|(_, t)| **t == f)
            .map(|(&i, _)| v[i])
            .collect();
        if own.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for p in &own {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let (lo, hi) = (lo - params.papilla_mm, hi + params.papilla_mm);
        let sel: Vec<usize> = (0..rig.len())
            .filter(|&k| mt[k] != f && v[rig[k]].cmpge(lo).all() && v[rig[k]].cmple(hi).all())
            .collect();
        if sel.is_empty() {
            continue;
        }
        let q: Vec<DVec3> = sel.iter().map(|&k| v[rig[k]]).collect();
        let d = geom::min_distances(&q, &own);
        for (m, &k) in sel.iter().enumerate() {
            let sm = (d[m] / params.papilla_mm).clamp(0.0, 1.0);
            al[k * nf + j] = 0.5 * (1.0 - smoothstep(sm));
        }
    }
    for x in &mut al {
        if *x <= 1e-3 {
            *x = 0.0;
        }
    }
    for (k, &i) in rig.iter().enumerate() {
        let row = &mut al[k * nf..(k + 1) * nf];
        let s: f64 = row.iter().sum();
        if s > 1.0 {
            for x in row.iter_mut() {
                *x /= s;
            }
        }
        for j in 0..nf {
            w[i * nf + j] = row[j] as f32;
        }
        if let Some(j) = col_of(mt[k]) {
            w[i * nf + j] += (1.0 - s.min(1.0)) as f32;
        }
    }
    let mut known_margin = vec![false; n];
    for &i in &rig {
        known_margin[i] = true;
    }
    // the soft tissue down to the alveolar crest follows its tooth rigidly where only one tooth is near
    let mut collar = vec![false; n];
    for i in 0..n {
        let row = &dfull[i * nf..(i + 1) * nf];
        let mut s = row.to_vec();
        s.sort_by(f64::total_cmp);
        let d0 = s.first().copied().unwrap_or(f64::INFINITY);
        let d2 = s.get(1).copied().unwrap_or(f64::INFINITY);
        if !g.margin[i]
            && !g.membrane[i]
            && d0 <= params.collar_mm
            && d2 > d0 + params.collar_gap_mm
        {
            collar[i] = true;
            let mut jmin = 0;
            for j in 1..nf {
                if row[j] < row[jmin] {
                    jmin = j;
                }
            }
            for j in 0..nf {
                w[i * nf + j] = 0.0;
            }
            w[i * nf + jmin] = 1.0;
        }
    }
    let fixed: Vec<bool> = (0..n)
        .map(|i| known_margin[i] || collar[i] || (!g.membrane[i] && !g.margin[i] && dmin[i] >= far))
        .collect();
    // harmonic interpolation, one tooth at a time on its zone of influence
    let edges = unique_edges(&g.tris);
    let ew = cotan_weights(v, &g.tris, &edges, COT_MIN, COT_MAX);
    let mut deg = vec![0.0f64; n];
    let mut deg2 = vec![0.0f64; n];
    for (e, &x) in edges.iter().zip(&ew) {
        deg[e[0] as usize] += x;
    }
    for (e, &x) in edges.iter().zip(&ew) {
        deg2[e[1] as usize] += x;
    }
    for i in 0..n {
        deg[i] += deg2[i];
    }
    let any_membrane = g.membrane.iter().any(|&m| m);
    /// Column, zone vertices and their solved weights.
    type Solved = Option<(usize, Vec<usize>, Vec<f64>)>;
    let solved: Vec<Solved> = (0..nf)
        .into_par_iter()
        .map(|j| {
            let zone: Vec<bool> = (0..n)
                .map(|i| {
                    let near = dfull[i * nf + j] < far;
                    (!fixed[i] && near) || (any_membrane && g.membrane[i] && near)
                })
                .collect();
            let u: Vec<usize> = (0..n).filter(|&i| zone[i]).collect();
            if u.is_empty() {
                return None;
            }
            let mut pos = vec![usize::MAX; n];
            for (k, &i) in u.iter().enumerate() {
                pos[i] = k;
            }
            let wj: Vec<f64> = (0..n).map(|i| f64::from(w[i * nf + j])).collect();
            let mut b1 = vec![0.0f64; u.len()];
            let mut b2 = vec![0.0f64; u.len()];
            let mut ia = Vec::new();
            let mut ib = Vec::new();
            let mut wi = Vec::new();
            for (e, &x) in edges.iter().zip(&ew) {
                let (a, b) = (e[0] as usize, e[1] as usize);
                if zone[a] && zone[b] {
                    ia.push(pos[a]);
                    ib.push(pos[b]);
                    wi.push(x);
                } else if zone[a] {
                    b1[pos[a]] += x * wj[b];
                }
            }
            for (e, &x) in edges.iter().zip(&ew) {
                let (a, b) = (e[0] as usize, e[1] as usize);
                if zone[b] && !zone[a] {
                    b2[pos[b]] += x * wj[a];
                }
            }
            let bvec: Vec<f64> = b1.iter().zip(&b2).map(|(x, y)| x + y).collect();
            let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(u.len() + 2 * ia.len());
            for (k, &i) in u.iter().enumerate() {
                trip.push((k, k, deg[i]));
            }
            for m in 0..ia.len() {
                trip.push((ia[m], ib[m], -wi[m]));
            }
            for m in 0..ia.len() {
                trip.push((ib[m], ia[m], -wi[m]));
            }
            let x0: Vec<f64> = u.iter().map(|&i| wj[i]).collect();
            let params = PcgParams {
                max_iterations: 6000,
                tolerance: 1e-6,
            };
            let x = CsrMatrix::from_triplets(u.len(), &trip)
                .and_then(|a| pcg_solve_many(&a, &[bvec], Some(std::slice::from_ref(&x0)), &params))
                .map(|s| s.x.into_iter().next().unwrap_or_default())
                .unwrap_or(x0);
            Some((j, u, x))
        })
        .collect();
    for (j, u, x) in solved.into_iter().flatten() {
        for (k, &i) in u.iter().enumerate() {
            w[i * nf + j] = x[k].clamp(0.0, 1.0) as f32;
        }
    }
    for i in 0..n {
        let row = &mut w[i * nf..(i + 1) * nf];
        let s: f32 = row.iter().sum();
        if s > 1.0 {
            for x in row.iter_mut() {
                *x /= s;
            }
        }
    }
    // the gingiva over the closed teeth follows them as a whole
    for i in 0..n {
        let x = ((dmin[i] - params.root_mm) / params.root_fade_mm.max(1e-9)).clamp(0.0, 1.0);
        let fade = 1.0 - smoothstep(x);
        let row = &mut w[i * nf..(i + 1) * nf];
        let s = f64::from(row.iter().sum::<f32>());
        if fade > 0.0 && s > 0.0 && s < 1.0 {
            let k = (1.0 + fade * (1.0 / s - 1.0)) as f32;
            for x in row.iter_mut() {
                *x *= k;
            }
        }
    }
    GumWeights {
        fdis: fdis.to_vec(),
        w,
    }
}

/// The part of [`deform`] that does not depend on the movement: for every
/// tooth the vertices it moves (weight above [`DEFORM_EPS`]) and their
/// weights (`deform_prep`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeformPrep {
    teeth: Vec<(Fdi, Vec<usize>, Vec<f64>)>,
}

/// Precomputes [`deform`] for the weights `w`.
pub fn deform_prep(w: &GumWeights) -> DeformPrep {
    let nf = w.fdis.len();
    let n = w.w.len().checked_div(nf).unwrap_or(0);
    let teeth = w
        .fdis
        .iter()
        .enumerate()
        .filter_map(|(j, &f)| {
            let nz: Vec<usize> = (0..n).filter(|&i| w.w[i * nf + j] > DEFORM_EPS).collect();
            if nz.is_empty() {
                return None;
            }
            let ws = nz.iter().map(|&i| f64::from(w.w[i * nf + j])).collect();
            Some((f, nz, ws))
        })
        .collect();
    DeformPrep { teeth }
}

/// Blend of the rigid tooth motions with the weights (`deform`): `deltas`
/// holds the rigid motion of each moved tooth (arch coordinates); the rest
/// of every row stays still, like the base of the model.
pub fn deform(rest: &[DVec3], prep: &DeformPrep, deltas: &BTreeMap<Fdi, DMat4>) -> Vec<DVec3> {
    let mut v = rest.to_vec();
    for (f, nz, ws) in &prep.teeth {
        let Some(d) = deltas.get(f) else {
            continue;
        };
        if *d == DMat4::IDENTITY {
            continue; // tooth in its rest position
        }
        for (k, &i) in nz.iter().enumerate() {
            let q = rest[i];
            let moved = d.transform_point3(q);
            v[i] += ws[k] * (moved - q);
        }
    }
    v
}

// ---- deformation of the whole scan (gingiva.py) ------------------------------------------------

/// Blend of the scan on the teeth (`gingiva.py`: the crown vertices of
/// each tooth, the gingiva vertices and their Shepard-Wendland weights).
#[derive(Debug, Clone, PartialEq)]
pub struct ScanBlend {
    /// The teeth, in column order (sorted).
    pub fdis: Vec<Fdi>,
    /// Crown vertices of each tooth.
    pub crown_idx: BTreeMap<Fdi, Vec<usize>>,
    /// Gingiva vertices (label 0).
    pub gum_idx: Vec<usize>,
    /// Weights, `(gum_idx.len(), fdis.len())` row-major.
    pub a: Vec<f32>,
}

/// Up to `max_per_tooth` gingiva-side margin vertices of every tooth: the
/// tooth vertices with an edge to a gingiva vertex, sorted and evenly
/// subsampled (`margin_vertices`).
pub fn margin_vertices(
    labels: &[u16],
    edges: &[[u32; 2]],
    max_per_tooth: usize,
) -> BTreeMap<u16, Vec<usize>> {
    let mut m: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for e in edges {
        let (la, lb) = (labels[e[0] as usize], labels[e[1] as usize]);
        if la != 0 && lb == 0 {
            m.entry(la).or_default().push(e[0] as usize);
        }
        if lb != 0 && la == 0 {
            m.entry(lb).or_default().push(e[1] as usize);
        }
    }
    m.into_iter()
        .map(|(f, mut v)| {
            v.sort_unstable();
            v.dedup();
            (f, geom::subsample(&v, max_per_tooth))
        })
        .collect()
}

/// Shepard inverse-distance weights times a Wendland C2 function of the
/// distances `d` (`(rows, cols)` row-major, finite) (`blend_weights`).
pub fn blend_weights(d: &[f64], cols: usize, radius: f64, eps: f64) -> Vec<f32> {
    let anchor = 1.0 / (radius * radius);
    d.chunks(cols.max(1))
        .flat_map(|row| {
            let w: Vec<f64> = row
                .iter()
                .map(|&x| {
                    let dd = x.min(radius);
                    let q = dd / radius;
                    let phi = if x < radius {
                        (1.0 - q).powi(4) * (4.0 * q + 1.0)
                    } else {
                        0.0
                    };
                    phi / (dd * dd + eps)
                })
                .collect();
            let s: f64 = w.iter().sum();
            w.into_iter()
                .map(move |x| (x / (s + anchor)) as f32)
                .collect::<Vec<f32>>()
        })
        .collect()
}

/// The blend of a labelled scan (`case_teeth.CaseData` fs, D, A: margin
/// samples of 200 vertices per tooth, distances within 8 mm, weights of
/// radius 6 mm).
pub fn scan_blend(
    v: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
) -> Result<ScanBlend, GingivaError> {
    check(v, tris, labels)?;
    let edges = unique_edges(tris);
    let samples = margin_vertices(labels, &edges, 200);
    let fdis: Vec<Fdi> = samples
        .keys()
        .filter_map(|&f| u8::try_from(f).ok().and_then(|x| Fdi::new(x).ok()))
        .collect();
    let gum_idx: Vec<usize> = (0..v.len()).filter(|&i| labels[i] == 0).collect();
    let pts: Vec<DVec3> = gum_idx.iter().map(|&i| v[i]).collect();
    let s: Vec<Vec<DVec3>> = samples
        .values()
        .map(|idx| idx.iter().map(|&i| v[i]).collect())
        .collect();
    let mut d = distance_matrix(&pts, &s, 8.0);
    for x in &mut d {
        if !x.is_finite() {
            *x = 1e9;
        }
    }
    let a = blend_weights(&d, s.len(), 6.0, 1e-3);
    let mut crown_idx: BTreeMap<Fdi, Vec<usize>> = BTreeMap::new();
    for (i, &l) in labels.iter().enumerate() {
        if let Some(f) = u8::try_from(l).ok().and_then(|x| Fdi::new(x).ok()) {
            crown_idx.entry(f).or_default().push(i);
        }
    }
    Ok(ScanBlend {
        fdis,
        crown_idx,
        gum_idx,
        a,
    })
}

/// The scan deformed by the tooth motions (`gingiva.deform`): crown
/// vertices rigidly with their tooth, gingiva vertices by the blend of the
/// tooth displacements.
pub fn deform_scan(rest: &[DVec3], blend: &ScanBlend, deltas: &BTreeMap<Fdi, DMat4>) -> Vec<DVec3> {
    let mut v = rest.to_vec();
    for (f, idx) in &blend.crown_idx {
        if let Some(d) = deltas.get(f) {
            for &i in idx {
                v[i] = d.transform_point3(rest[i]);
            }
        }
    }
    let nf = blend.fdis.len();
    let mut disp = vec![DVec3::ZERO; blend.gum_idx.len()];
    for (j, f) in blend.fdis.iter().enumerate() {
        let Some(d) = deltas.get(f) else {
            continue;
        };
        for (k, &i) in blend.gum_idx.iter().enumerate() {
            let a = blend.a[k * nf + j];
            if a > 1e-4 {
                let q = rest[i];
                disp[k] += f64::from(a) * (d.transform_point3(q) - q);
            }
        }
    }
    for (k, &i) in blend.gum_idx.iter().enumerate() {
        v[i] = rest[i] + disp[k];
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_weights_vanish_far_away_and_sum_below_one() {
        let d = vec![0.5, 2.0, 7.0, 1e9];
        let w = blend_weights(&d, 2, 6.0, 1e-3);
        assert!(w[0] > w[1] && w[0] + w[1] < 1.0);
        assert_eq!(w[2], 0.0);
        assert_eq!(w[3], 0.0);
    }

    #[test]
    fn cotan_weights_of_a_right_triangle_pair() {
        let v = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        ];
        let t = vec![[0, 1, 2], [0, 2, 3]];
        let e = unique_edges(&t);
        let w = cotan_weights(&v, &t, &e, 0.0, 10.0);
        // diagonal 0-2: two right angles opposite -> 0; sides: cot 45 / 2
        let k = e.binary_search(&[0, 2]).unwrap();
        assert!(w[k].abs() < 1e-12);
        let k = e.binary_search(&[0, 1]).unwrap();
        assert!((w[k] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn stored_flags_round_trip() {
        let g = GumSurface {
            vertices: vec![DVec3::ZERO, DVec3::X, DVec3::Y],
            tris: vec![[0, 1, 2]],
            tooth: vec![0, 41, 31],
            margin: vec![false, true, false],
            membrane: vec![false, false, true],
            col: vec![false, false, true],
        };
        let back = GumSurface::from_mesh(&g.to_mesh()).unwrap();
        assert_eq!(back, g);
    }

    /// A flat grid with a labelled square (a crown) in the middle.
    fn flat_crown(n: usize) -> (Vec<DVec3>, Vec<[u32; 3]>, Vec<u16>) {
        let id = |i: usize, j: usize| (i * n + j) as u32;
        let v = (0..n * n)
            .map(|k| DVec3::new((k / n) as f64 * 0.5, (k % n) as f64 * 0.5, 0.0))
            .collect();
        let mut tris = Vec::new();
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                tris.push([id(i, j), id(i + 1, j), id(i + 1, j + 1)]);
                tris.push([id(i, j), id(i + 1, j + 1), id(i, j + 1)]);
            }
        }
        let labels = (0..n * n)
            .map(|k| {
                u16::from(
                    (n / 4..3 * n / 4).contains(&(k / n)) && (n / 4..3 * n / 4).contains(&(k % n)),
                ) * 41
            })
            .collect();
        (v, tris, labels)
    }

    #[test]
    fn a_cancelled_gum_stops_between_stages_and_inside_the_solve() {
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        let (v, t, l) = flat_crown(40);
        let params = Params::default();
        let run = |progress: &(dyn Fn(f32) + Sync), cancelled: &(dyn Fn() -> bool + Sync)| {
            gum_surface_with_progress(&v, &t, &l, &params, progress, cancelled)
        };
        let g = run(&|_| {}, &|| false).unwrap();
        assert!(g.membrane.iter().any(|&m| m));
        // before anything is done
        assert!(matches!(
            run(&|_| {}, &|| true),
            Err(GingivaError::Cancelled)
        ));
        // from the moment the membrane solve starts (`progress(0.1)`): only
        // the solver loop can see it
        let solving = AtomicBool::new(false);
        let r = run(&|x| solving.store(x >= 0.1, Relaxed), &|| {
            solving.load(Relaxed)
        });
        assert!(matches!(r, Err(GingivaError::Cancelled)), "{r:?}");
        // the solver itself, asked at the first iteration
        let inner: Vec<bool> = l.iter().map(|&x| x != 0).collect();
        let e = unique_edges(&t);
        assert!(solve_membrane(&v, &e, &inner, 1e-4, 3000, &|| true).is_none());
        assert!(solve_membrane(&v, &e, &inner, 1e-4, 3000, &|| false).is_some());
    }
}
