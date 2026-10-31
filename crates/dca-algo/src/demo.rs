//! Synthetic dental model (typodont) with ground-truth labels, for tests,
//! golden fixtures and the demo case.
//!
//! Reference: `ortho_aligner/demo.py`.
//!
//! Tooth sizes follow typical Wheeler values ([`crate::anatomy`]);
//! malposition is random with a chosen [`Severity`]. THE RESULT IS A
//! SYNTHETIC MODEL, NOT PATIENT DATA. Orientation: occlusal surfaces towards
//! +Z, incisors towards +Y (plaster-cast orientation for both arches), mm.
//!
//! The plugin joins the teeth and the gum with a Blender voxel remesh (a
//! union of volumes). Here the mesh is the plain concatenation of the closed
//! shells: one shell per tooth plus the gum shell. Every vertex carries the
//! FDI number of its shell (`0` for the gum) in `vertex_labels`, so the
//! labels are exact. The generator is deterministic: the same [`Params`]
//! always give the same mesh (the random stream is this module's own
//! generator, not numpy's, so the malpositions differ from the plugin's for
//! the same seed; [`generate_with`] takes any source of draws).

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use dc_case::Fdi;
use dc_mesh::TriMesh;
use dca_model::Seed;
use glam::{DMat3, DVec2, DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::archform::{ArchCurve, ArchModel, curve_from_model};

/// How malpositioned the teeth are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Rotations up to 8°, 2 mm of crowding.
    Mild,
    /// Rotations up to 18°, 5 mm of crowding.
    #[default]
    Moderate,
    /// Rotations up to 30°, 8 mm of crowding.
    Severe,
}

/// Magnitudes of a [`Severity`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeverityTable {
    /// Largest rotation (degrees).
    pub rotation_deg: f64,
    /// Largest bucco-lingual displacement (mm).
    pub bucco_lingual_mm: f64,
    /// Largest vertical displacement (mm).
    pub vertical_mm: f64,
    /// Largest tip and torque (degrees).
    pub tip_torque_deg: f64,
    /// Crowding of the whole arch (mm).
    pub crowding_mm: f64,
}

impl Severity {
    /// The magnitudes of this level (the plugin's `SEVERITY` table).
    pub fn table(self) -> SeverityTable {
        let (rotation_deg, bucco_lingual_mm, vertical_mm, tip_torque_deg, crowding_mm) = match self
        {
            Severity::Mild => (8.0, 0.6, 0.4, 2.0, 2.0),
            Severity::Moderate => (18.0, 1.2, 0.7, 4.0, 5.0),
            Severity::Severe => (30.0, 2.0, 1.1, 6.0, 8.0),
        };
        SeverityTable {
            rotation_deg,
            bucco_lingual_mm,
            vertical_mm,
            tip_torque_deg,
            crowding_mm,
        }
    }
}

/// Parameters of the demo generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arch to generate.
    pub arch: Arch,
    /// How crowded and rotated the teeth are.
    pub severity: Severity,
    /// Seed of the random generator (same seed, same model).
    pub seed: u64,
    /// Include the third molars (default: 14 teeth per arch).
    pub third_molars: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            arch: Arch::Lower,
            severity: Severity::Moderate,
            seed: 7,
            third_molars: false,
        }
    }
}

/// Ground truth of one generated tooth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToothTruth {
    /// Rotation about the long axis (degrees).
    pub rotation_deg: f64,
    /// Bucco-lingual displacement (mm).
    pub bucco_lingual_mm: f64,
    /// Vertical displacement (mm).
    pub vertical_mm: f64,
    /// Tip (degrees).
    pub tip_deg: f64,
    /// Torque, including the curve of Wilson (degrees).
    pub torque_deg: f64,
    /// Seed point: the centre of the occlusal face.
    pub seed_point: DVec3,
}

/// The generated model.
#[derive(Debug, Clone, PartialEq)]
pub struct Demo {
    /// Arch of the model.
    pub arch: Arch,
    /// Teeth and gum; `vertex_labels` holds the FDI number of each vertex
    /// (`0` for the gum).
    pub mesh: TriMesh,
    /// One seed per tooth (the ground-truth seed points), in arch order.
    pub seeds: Vec<Seed>,
    /// Ground truth of each tooth.
    pub teeth: BTreeMap<Fdi, ToothTruth>,
}

/// Source of the random draws of the malposition.
pub trait Draws {
    /// Uniform number in `[lo, hi)`.
    fn uniform(&mut self, lo: f64, hi: f64) -> f64;
    /// Normal number with this mean and standard deviation.
    fn normal(&mut self, mean: f64, sd: f64) -> f64;
}

/// Seeded pseudo-random generator (xoshiro256++ started by SplitMix64).
#[derive(Debug, Clone)]
pub struct SeededDraws {
    s: [u64; 4],
}

impl SeededDraws {
    /// Generator for this seed.
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Self {
            s: [next(), next(), next(), next()],
        }
    }

    fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform number in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Draws for SeededDraws {
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        // Box-Muller; 1 - u is in (0, 1]
        let (u1, u2) = (1.0 - self.unit(), self.unit());
        mean + sd * (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
    }
}

fn smooth(u: f64, a: f64, b: f64) -> f64 {
    let t = ((u - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Incisor,
    Canine,
    Premolar,
    Molar,
}

fn kind(fdi: Fdi) -> Kind {
    match anatomy::index(fdi) {
        0..=2 => Kind::Incisor,
        3 => Kind::Canine,
        4 | 5 => Kind::Premolar,
        _ => Kind::Molar,
    }
}

/// Relative mesio-distal (f) and bucco-lingual (g) half-widths along the
/// crown height `u` in `[0, 1]`.
fn profile(kind: Kind, u: f64) -> (f64, f64) {
    match kind {
        Kind::Incisor => (
            0.74 + 0.26 * smooth(u, 0.0, 0.6),
            1.0 - 0.70 * smooth(u, 0.35, 1.0),
        ),
        Kind::Canine => (
            0.78 + 0.22 * smooth(u, 0.0, 0.45) - 0.35 * smooth(u, 0.65, 1.0),
            1.0 - 0.45 * smooth(u, 0.45, 1.0),
        ),
        Kind::Premolar => (
            0.76 + 0.24 * smooth(u, 0.0, 0.45),
            0.86 + 0.14 * smooth(u, 0.0, 0.4),
        ),
        Kind::Molar => (
            0.86 + 0.14 * smooth(u, 0.0, 0.4),
            0.88 + 0.12 * smooth(u, 0.0, 0.4),
        ),
    }
}

/// Relative height of the occlusal surface at the normalised position
/// `(xn, yn)` of the crown.
fn cap(kind: Kind, xn: f64, yn: f64, lower: bool) -> f64 {
    match kind {
        Kind::Incisor => (1.0 - 0.5 * yn.powi(2) - 0.15 * xn.powi(4)).clamp(0.0, 1.0),
        Kind::Canine => (1.0 - 0.85 * (xn * xn + yn * yn).sqrt()).clamp(0.0, 1.0),
        Kind::Premolar => {
            let g =
                |cx: f64, cy: f64| (-((xn - cx).powi(2) / 0.25 + (yn - cy).powi(2) / 0.12)).exp();
            0.5 + 0.5 * g(0.0, 0.45).max(0.85 * g(0.0, -0.45))
        }
        Kind::Molar => {
            // the higher cusps are the palatal ones in the upper arch, the
            // buccal ones in the lower arch (curve of Wilson; y > 0 buccal):
            // no 4-fold symmetry
            let (hb, hl) = if lower { (1.0, 0.75) } else { (0.75, 1.0) };
            let cusps = [
                (0.45, 0.45, hb),
                (-0.45, 0.45, hb),
                (0.45, -0.45, hl),
                (-0.45, -0.45, hl),
            ];
            let top = cusps
                .iter()
                .map(|&(cx, cy, h)| h * (-((xn - cx).powi(2) + (yn - cy).powi(2)) / 0.10).exp())
                .fold(f64::NEG_INFINITY, f64::max);
            0.5 + 0.5 * top
        }
    }
}

/// `sign(v) |v|^e` as numpy writes it (`sign(0) = 0`).
fn spow(v: f64, e: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v.signum() * v.abs().powf(e)
    }
}

/// Closed crown (plus a short root stub) in local coordinates: x =
/// mesio-distal, y = bucco-lingual, z = occlusal, origin at the cervical
/// centre on the gum level. Returns the mesh and the top point of the crown
/// (the seed point).
pub fn tooth_mesh(fdi: Fdi, n_theta: usize) -> (Vec<DVec3>, Vec<[u32; 3]>, DVec3) {
    let (md, bl) = anatomy::typical_diameters(fdi);
    let (crown_len, _) = anatomy::crown_root_length(fdi);
    let h = crown_len - 1.5;
    let kind = kind(fdi);
    let lower = anatomy::arch_of(fdi) == Arch::Lower;
    let ne = 2.0 / 2.6;
    let cs: Vec<(f64, f64)> = (0..n_theta)
        .map(|i| {
            let th = TAU * i as f64 / n_theta as f64;
            (spow(th.cos(), ne), spow(th.sin(), ne))
        })
        .collect();
    let mut zs = vec![-4.0, -2.0];
    zs.extend((0..12).map(|i| 0.8 * h * i as f64 / 11.0));

    let mut rings: Vec<Vec<DVec3>> = Vec::new();
    for &z in &zs {
        let u = z.max(0.0) / h;
        let (mut f, mut g) = profile(kind, u);
        if z < 0.0 {
            (f, g) = profile(kind, 0.0);
            f *= 0.95;
            g *= 0.95;
        }
        rings.push(
            cs.iter()
                .map(|&(cx, cy)| DVec3::new(cx * md / 2.0 * f, cy * bl / 2.0 * g, z))
                .collect(),
        );
    }
    let (f_top, g_top) = profile(kind, 0.8);
    let (a_top, c_top) = (md / 2.0 * f_top, bl / 2.0 * g_top);
    for j in 1..10 {
        let s = 1.0 - j as f64 / 10.0;
        let w = smooth(1.0 - s, 0.0, 0.3);
        rings.push(
            cs.iter()
                .map(|&(cx, cy)| {
                    let (xn, yn) = (cx * s, cy * s);
                    let z = if matches!(kind, Kind::Premolar | Kind::Molar) {
                        0.8 * h + (h - 0.8 * h) * cap(kind, xn, yn, lower) * w
                    } else {
                        0.8 * h + 0.2 * h * cap(kind, xn, yn, false) * w
                    };
                    DVec3::new(xn * a_top, yn * c_top, z)
                })
                .collect(),
        );
    }
    let nr = rings.len();
    let mut v: Vec<DVec3> = rings.into_iter().flatten().collect();
    let top_z = 0.8 * h + 0.2 * h * cap(kind, 0.0, 0.0, false);
    v.push(DVec3::new(0.0, 0.0, top_z));
    v.push(DVec3::new(0.0, 0.0, zs[0]));
    let n = n_theta as u32;
    let mut tris = Vec::new();
    for r in 0..(nr as u32 - 1) {
        for i in 0..n {
            let a = r * n + i;
            let b = r * n + (i + 1) % n;
            let (c, d) = (a + n, b + n);
            tris.push([a, b, d]);
            tris.push([a, d, c]);
        }
    }
    let (top, bot) = (v.len() as u32 - 2, v.len() as u32 - 1);
    let last = (nr as u32 - 1) * n;
    for i in 0..n {
        tris.push([last + i, last + (i + 1) % n, top]);
        tris.push([bot, (i + 1) % n, i]);
    }
    (v, tris, DVec3::new(0.0, 0.0, top_z))
}

/// Closed gum tube along the arch curve between arc lengths `s0` and `s1`.
pub fn gum_mesh(
    curve: &ArchCurve,
    s0: f64,
    s1: f64,
    step: f64,
    half_w: f64,
    depth: f64,
) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    // numpy.arange(s0, s1 + step, step)
    let count = (((s1 + step - s0) / step).ceil().max(0.0)) as usize;
    let ss: Vec<f64> = (0..count).map(|k| s0 + k as f64 * step).collect();
    let m = 40usize;
    let ex = 2.0 / 3.5;
    let ring_uv: Vec<(f64, f64)> = (0..m)
        .map(|i| {
            let phi = TAU * i as f64 / m as f64;
            (
                spow(phi.cos(), ex) * half_w,
                spow(phi.sin(), ex) * depth / 2.0 - depth / 2.0,
            )
        })
        .collect();
    let mut v: Vec<DVec3> = Vec::with_capacity(ss.len() * m + 2);
    for &s in &ss {
        let (p, n) = (curve.point(s), curve.normal_out(s));
        v.extend(
            ring_uv
                .iter()
                .map(|&(un, uz)| DVec3::new(p.x + n.x * un, p.y + n.y * un, uz)),
        );
    }
    let mean_of = |pts: &[DVec3]| pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
    let (c0, c1) = (mean_of(&v[..m]), mean_of(&v[v.len() - m..]));
    let m32 = m as u32;
    let mut tris = Vec::new();
    for r in 0..(ss.len() as u32 - 1) {
        for i in 0..m32 {
            let a = r * m32 + i;
            let b = r * m32 + (i + 1) % m32;
            let (c, d) = (a + m32, b + m32);
            tris.push([a, d, b]);
            tris.push([a, c, d]);
        }
    }
    v.push(c0);
    v.push(c1);
    let (i0, i1) = (v.len() as u32 - 2, v.len() as u32 - 1);
    let off = (ss.len() as u32 - 1) * m32;
    for i in 0..m32 {
        tris.push([i0, i, (i + 1) % m32]);
        tris.push([i1, off + (i + 1) % m32, off + i]);
    }
    // make the normals point outwards
    let centre = mean_of(&v);
    let mut acc = 0.0;
    for t in &tris {
        let [a, b, c] = t.map(|i| v[i as usize]);
        let cr = (b - a).cross(c - a);
        let area2 = cr.length();
        let n = if area2 > 0.0 { cr / area2 } else { cr };
        acc += n.dot((a + b + c) / 3.0 - centre);
    }
    if acc / (tris.len() as f64) < 0.0 {
        for t in &mut tris {
            t.reverse();
        }
    }
    (v, tris)
}

/// Beta-function arch (Braun 1998) whose half length fits the teeth;
/// returns the curve and the arch width.
pub fn ideal_arch(arch: Arch, teeth: &[Fdi]) -> (ArchCurve, f64) {
    let w = if arch == Arch::Lower { 50.0 } else { 54.0 };
    let half: f64 = teeth
        .iter()
        .filter(|f| anatomy::is_right(**f))
        .map(|f| anatomy::typical_diameters(*f).0)
        .sum();
    let (mut lo, mut hi) = (10.0, 45.0);
    for _ in 0..50 {
        let d = 0.5 * (lo + hi);
        let c = curve_from_model(&ArchModel::Beta { w, d, y0: 0.0 }, w / 2.0, false);
        let l = c.project(DVec2::new(w / 2.0 * 0.999, 0.0));
        if l < half + 1.0 {
            lo = d;
        } else {
            hi = d;
        }
    }
    let d = 0.5 * (lo + hi);
    (
        curve_from_model(&ArchModel::Beta { w, d, y0: 0.0 }, w / 2.0, false),
        w,
    )
}

/// Rotation `Rz(rot) Ry(tip) Rx(tor)` of the local malposition.
fn local_rotation(rot_deg: f64, tip_deg: f64, tor_deg: f64) -> DMat3 {
    DMat3::from_rotation_z(rot_deg.to_radians())
        * DMat3::from_rotation_y(tip_deg.to_radians())
        * DMat3::from_rotation_x(tor_deg.to_radians())
}

/// Generates the model with an explicit source of random draws. The draws
/// are consumed per tooth, in arch order of each side (right side first,
/// from the central incisor outwards): rotation (uniform), bucco-lingual
/// (uniform), vertical (normal), tip (uniform), torque (uniform).
pub fn generate_with<R: Draws>(
    arch: Arch,
    severity: Severity,
    third_molars: bool,
    rng: &mut R,
) -> Demo {
    let teeth: Vec<Fdi> = anatomy::arch_order(arch)
        .into_iter()
        .filter(|f| third_molars || anatomy::index(*f) != 8)
        .collect();
    let (curve, _) = ideal_arch(arch, &teeth);
    let sev = severity.table();
    let right_sign = if arch == Arch::Lower { 1.0 } else { -1.0 };
    let mut right: Vec<Fdi> = teeth
        .iter()
        .copied()
        .filter(|f| anatomy::is_right(*f))
        .collect();
    let mut left: Vec<Fdi> = teeth
        .iter()
        .copied()
        .filter(|f| !anatomy::is_right(*f))
        .collect();
    right.sort_by_key(|f| anatomy::index(*f));
    left.sort_by_key(|f| anatomy::index(*f));
    let half: f64 = right.iter().map(|f| anatomy::typical_diameters(*f).0).sum();
    let squeeze = 1.0 - (sev.crowding_mm / 2.0) / half;

    let mut s_ext = [0.0f64, 0.0];
    let mut positions: Vec<DVec3> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut labels: Vec<u16> = Vec::new();
    let mut truth = BTreeMap::new();
    let mut by_arch_order: Vec<(Fdi, DVec3)> = Vec::new();

    for (side, sgn) in [(&right, right_sign), (&left, -right_sign)] {
        let mut s = 0.0;
        for (k, &f) in side.iter().enumerate() {
            let w = anatomy::typical_diameters(f).0 * squeeze;
            let s2 = curve.s_at_chord(s, w, sgn);
            s_ext = [s_ext[0].min(s2), s_ext[1].max(s2)];
            let (pm, pd) = (curve.point(s), curve.point(s2));
            let c = 0.5 * (pm + pd);
            let t = (pd - pm).normalize_or_zero() * sgn; // direction of increasing x
            let n2 = DVec2::new(-t.y, t.x); // outward normal
            // malposition
            let alt = if k % 2 == 0 { 1.0 } else { -1.0 };
            let alt = if matches!(anatomy::index(f), 6 | 7) {
                alt * 0.3
            } else {
                alt
            };
            let rot = rng.uniform(-1.0, 1.0) * sev.rotation_deg;
            let bl = alt * rng.uniform(0.4, 1.0) * sev.bucco_lingual_mm;
            let vert = rng
                .normal(0.0, sev.vertical_mm / 2.0)
                .clamp(-sev.vertical_mm, sev.vertical_mm);
            let tip = rng.uniform(-1.0, 1.0) * sev.tip_torque_deg;
            let mut tor = rng.uniform(-1.0, 1.0) * sev.tip_torque_deg;
            // curve of Wilson: posterior crowns inclined lingually in the
            // lower arch (buccal cusps highest), buccally in the upper arch
            // (palatal cusps highest); about 10 degrees at the molars, 5 at
            // the premolars
            let wilson = match anatomy::index(f) {
                6.. => 10.0,
                4 | 5 => 5.0,
                _ => 0.0,
            };
            tor += if arch == Arch::Lower { wilson } else { -wilson };

            let (vt, tt, top) = tooth_mesh(f, 48);
            let rl = local_rotation(rot, tip, tor);
            let mut xa = DVec3::new(t.x, t.y, 0.0);
            let ya = DVec3::new(n2.x, n2.y, 0.0);
            let za = DVec3::Z;
            if xa.cross(ya).dot(za) < 0.0 {
                xa = -xa;
            }
            let rw = DMat3::from_cols(xa, ya, za);
            let pos = DVec3::new(c.x, c.y, 0.0) + ya * bl + za * vert;
            let to_world = |p: DVec3| rw * (rl * p) + pos;

            let base = positions.len() as u32;
            positions.extend(vt.iter().map(|&p| to_world(p)));
            tris.extend(tt.iter().map(|t| t.map(|i| i + base)));
            labels.extend(std::iter::repeat_n(u16::from(f.get()), vt.len()));
            let seed_point = to_world(top);
            by_arch_order.push((f, seed_point));
            truth.insert(
                f,
                ToothTruth {
                    rotation_deg: rot,
                    bucco_lingual_mm: bl,
                    vertical_mm: vert,
                    tip_deg: tip,
                    torque_deg: tor,
                    seed_point,
                },
            );
            s = s2;
        }
    }

    // gum along the arch, 3 mm beyond the terminal teeth
    let (vg, tg) = gum_mesh(&curve, s_ext[0] - 3.0, s_ext[1] + 3.0, 0.5, 6.8, 12.0);
    let base = positions.len() as u32;
    labels.extend(std::iter::repeat_n(0u16, vg.len()));
    positions.extend(vg);
    tris.extend(tg.iter().map(|t| t.map(|i| i + base)));

    let mut mesh = TriMesh::new(
        positions
            .iter()
            .map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32))
            .collect(),
        tris,
    );
    mesh.vertex_labels = Some(labels);
    let seeds = anatomy::arch_order(arch)
        .into_iter()
        .filter_map(|f| {
            by_arch_order
                .iter()
                .find(|(g, _)| *g == f)
                .map(|&(fdi, position)| Seed { fdi, position })
        })
        .collect();
    Demo {
        arch,
        mesh,
        seeds,
        teeth: truth,
    }
}

/// Generates the demo model of these parameters (deterministic).
pub fn run(params: &Params) -> Demo {
    let mut rng = SeededDraws::new(params.seed);
    generate_with(params.arch, params.severity, params.third_molars, &mut rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(v: &[Vec3], t: &[[u32; 3]]) -> f64 {
        t.iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| v[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn the_stream_is_deterministic_and_well_distributed() {
        let mut a = SeededDraws::new(7);
        let mut b = SeededDraws::new(7);
        let mut c = SeededDraws::new(8);
        let xs: Vec<f64> = (0..2000).map(|_| a.uniform(-1.0, 1.0)).collect();
        let ys: Vec<f64> = (0..2000).map(|_| b.uniform(-1.0, 1.0)).collect();
        let zs: Vec<f64> = (0..2000).map(|_| c.uniform(-1.0, 1.0)).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
        assert!(xs.iter().all(|x| (-1.0..1.0).contains(x)));
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        assert!(mean.abs() < 0.06, "mean {mean}");
        let ns: Vec<f64> = (0..4000).map(|_| a.normal(2.0, 0.5)).collect();
        let m = ns.iter().sum::<f64>() / ns.len() as f64;
        let sd = (ns.iter().map(|x| (x - m).powi(2)).sum::<f64>() / ns.len() as f64).sqrt();
        assert!(
            (m - 2.0).abs() < 0.05 && (sd - 0.5).abs() < 0.05,
            "{m} {sd}"
        );
    }

    #[test]
    fn severity_table_matches_the_plugin() {
        let t = Severity::Moderate.table();
        assert_eq!(
            (
                t.rotation_deg,
                t.bucco_lingual_mm,
                t.vertical_mm,
                t.tip_torque_deg,
                t.crowding_mm
            ),
            (18.0, 1.2, 0.7, 4.0, 5.0)
        );
        assert_eq!(Severity::Mild.table().crowding_mm, 2.0);
        assert_eq!(Severity::Severe.table().rotation_deg, 30.0);
    }

    #[test]
    fn a_tooth_is_a_closed_outward_shell_on_the_gum_level() {
        for n in [11, 12, 13, 14, 15, 16, 17, 18, 31, 33, 36, 48] {
            let f = Fdi::new(n).unwrap();
            let (v, t, top) = tooth_mesh(f, 48);
            assert_eq!(v.len(), 23 * 48 + 2);
            assert_eq!(t.len(), 22 * 48 * 2 + 2 * 48);
            let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t.clone());
            let report = mesh.validate();
            assert!(
                (report.is_manifold() && report.is_watertight()),
                "{n}: {report:?}"
            );
            assert!(volume(&mesh.positions, &mesh.tris) > 100.0, "{n}");
            // the root stub is below the gum level, the crown above
            let zmin = v.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
            let zmax = v.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(zmin, -4.0);
            assert!((top.z - zmax).abs() < 0.9, "{n}: top {} max {zmax}", top.z);
            // mesio-distal and bucco-lingual size follow the table
            let (md, bl) = anatomy::typical_diameters(f);
            let xmax = v.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
            let ymax = v.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
            assert!(xmax <= md / 2.0 + 1e-9 && xmax > 0.7 * md / 2.0, "{n}");
            assert!(ymax <= bl / 2.0 + 1e-9 && ymax > 0.7 * bl / 2.0, "{n}");
        }
    }

    #[test]
    fn the_gum_is_an_outward_closed_shell() {
        let teeth: Vec<Fdi> = anatomy::arch_order(Arch::Lower)
            .into_iter()
            .filter(|f| anatomy::index(*f) != 8)
            .collect();
        let (curve, w) = ideal_arch(Arch::Lower, &teeth);
        assert_eq!(w, 50.0);
        let (v, t) = gum_mesh(&curve, -30.0, 30.0, 0.5, 6.8, 12.0);
        let mesh = TriMesh::new(v.iter().map(|p| p.as_vec3()).collect(), t);
        assert!({
            let r = mesh.validate();
            r.is_manifold() && r.is_watertight()
        });
        assert!(volume(&mesh.positions, &mesh.tris) > 0.0);
    }

    #[test]
    fn ideal_arch_length_fits_the_teeth() {
        let teeth: Vec<Fdi> = anatomy::arch_order(Arch::Upper)
            .into_iter()
            .filter(|f| anatomy::index(*f) != 8)
            .collect();
        let half: f64 = teeth
            .iter()
            .filter(|f| anatomy::is_right(**f))
            .map(|f| anatomy::typical_diameters(*f).0)
            .sum();
        let (curve, w) = ideal_arch(Arch::Upper, &teeth);
        assert_eq!(w, 54.0);
        let l = curve.project(DVec2::new(w / 2.0 * 0.999, 0.0));
        // the bisection stops where the end of the beta curve projects: its arc
        // length is the half length of the teeth (the plugin gives 55.49836 too)
        assert!((l - half).abs() < 0.01, "L {l} half {half}");
    }

    #[test]
    fn same_parameters_same_model_other_seed_other_model() {
        let p = Params::default();
        let a = run(&p);
        let b = run(&p);
        assert_eq!(a, b);
        let c = run(&Params { seed: 8, ..p });
        assert_ne!(a.mesh.positions, c.mesh.positions);
        assert_eq!(a.mesh.vertex_count(), c.mesh.vertex_count());
    }

    #[test]
    fn labels_are_exact_per_shell() {
        for arch in [Arch::Lower, Arch::Upper] {
            let d = run(&Params {
                arch,
                severity: Severity::Severe,
                seed: 3,
                third_molars: false,
            });
            d.mesh.check().unwrap();
            let labels = d.mesh.vertex_labels.as_ref().unwrap();
            assert_eq!(labels.len(), d.mesh.vertex_count());
            // 14 teeth, labelled with their FDI number, plus the gum (0)
            let mut seen: Vec<u16> = labels.clone();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), 15);
            assert_eq!(seen[0], 0);
            for f in anatomy::arch_order(arch) {
                let n = labels.iter().filter(|&&l| l == u16::from(f.get())).count();
                if anatomy::index(f) == 8 {
                    assert_eq!(n, 0);
                } else {
                    assert_eq!(n, 23 * 48 + 2, "{f}");
                }
            }
            // every triangle stays inside one shell
            for t in &d.mesh.tris {
                let l = t.map(|i| labels[i as usize]);
                assert!(l[0] == l[1] && l[1] == l[2]);
            }
            // seeds: one per tooth, in arch order, inside the right shell
            assert_eq!(d.seeds.len(), 14);
            let order: Vec<u8> = d.seeds.iter().map(|s| s.fdi.get()).collect();
            let expect: Vec<u8> = anatomy::arch_order(arch)
                .iter()
                .map(|f| f.get())
                .filter(|n| n % 10 != 8)
                .collect();
            assert_eq!(order, expect);
            for s in &d.seeds {
                let own: Vec<DVec3> = d
                    .mesh
                    .positions
                    .iter()
                    .zip(labels)
                    .filter(|(_, l)| **l == u16::from(s.fdi.get()))
                    .map(|(p, _)| p.as_dvec3())
                    .collect();
                let nearest = own
                    .iter()
                    .map(|p| p.distance(s.position))
                    .fold(f64::INFINITY, f64::min);
                assert!(nearest < 2.0, "{}: seed {nearest} mm from its crown", s.fdi);
                assert_eq!(d.teeth[&s.fdi].seed_point, s.position);
            }
        }
    }

    #[test]
    fn third_molars_add_two_teeth() {
        let d = run(&Params {
            third_molars: true,
            ..Params::default()
        });
        assert_eq!(d.seeds.len(), 16);
        assert!(d.teeth.contains_key(&Fdi::new(48).unwrap()));
        assert!(d.teeth.contains_key(&Fdi::new(38).unwrap()));
    }

    #[test]
    fn shells_are_closed_and_outward() {
        let d = run(&Params::default());
        let labels = d.mesh.vertex_labels.as_ref().unwrap();
        // each label is its own closed manifold shell with positive volume
        let comps = d.mesh.connected_components();
        assert_eq!(comps.len(), 15);
        let r = d.mesh.validate();
        assert!(r.is_manifold() && r.is_watertight());
        let _ = labels;
        assert!(volume(&d.mesh.positions, &d.mesh.tris) > 0.0);
    }

    #[test]
    fn plaster_orientation_and_size() {
        for arch in [Arch::Lower, Arch::Upper] {
            let d = run(&Params {
                arch,
                ..Params::default()
            });
            let bb = d.mesh.aabb().unwrap();
            let size = bb.size();
            // an arch is about 50-60 mm wide and 40-50 mm deep
            assert!((45.0..75.0).contains(&size.x), "{size:?}");
            assert!((35.0..60.0).contains(&size.y), "{size:?}");
            // incisors towards +Y, occlusal towards +Z
            let mean = |sel: &dyn Fn(Fdi) -> bool| {
                let v: Vec<DVec3> = d
                    .seeds
                    .iter()
                    .filter(|s| sel(s.fdi))
                    .map(|s| s.position)
                    .collect();
                v.iter().copied().sum::<DVec3>() / v.len() as f64
            };
            let ant = mean(&|f| anatomy::index(f) <= 2);
            let post = mean(&|f| anatomy::index(f) >= 6);
            assert!(ant.y > post.y + 15.0, "{arch:?}");
            assert!(ant.z > 5.0, "{arch:?}");
            // the patient's right is at +X for the lower arch and −X for the upper
            let right = mean(&|f| anatomy::is_right(f));
            assert_eq!(right.x > 0.0, arch == Arch::Lower);
        }
    }

    #[test]
    fn severity_increases_the_malposition() {
        let spread = |sev| {
            let d = run(&Params {
                severity: sev,
                seed: 11,
                ..Params::default()
            });
            d.teeth
                .values()
                .map(|t| t.rotation_deg.abs())
                .fold(0.0, f64::max)
        };
        let (m, s) = (spread(Severity::Mild), spread(Severity::Severe));
        assert!(m <= 8.0 && s <= 30.0 && s > m);
        let d = run(&Params {
            severity: Severity::Moderate,
            ..Params::default()
        });
        for t in d.teeth.values() {
            assert!(t.rotation_deg.abs() <= 18.0);
            assert!(t.vertical_mm.abs() <= 0.7);
            assert!(t.tip_deg.abs() <= 4.0);
        }
    }

    #[test]
    fn params_serde() {
        let p: Params =
            serde_json::from_str(r#"{"arch":"upper","severity":"severe","seed":9}"#).unwrap();
        assert_eq!(p.arch, Arch::Upper);
        assert_eq!(p.severity, Severity::Severe);
        assert_eq!(p.seed, 9);
        assert!(!p.third_molars);
        assert_eq!(
            serde_json::from_str::<Params>("{}").unwrap(),
            Params::default()
        );
        assert!(serde_json::from_str::<Params>(r#"{"arch":"x"}"#).is_err());
        assert!(serde_json::from_str::<Params>(r#"{"nope":1}"#).is_err());
    }
}
