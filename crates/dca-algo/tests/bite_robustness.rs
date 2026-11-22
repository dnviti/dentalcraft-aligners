//! Robustness of the bite registration (`dca_algo::bite::register`) to the
//! bite scans of real life, on the demo arches in their virtual bite.
//!
//! A bite scan is cut from the two arches in occlusion (their buccal faces
//! within 7 mm of the occlusal plane, around chosen teeth), spoiled as real
//! scans are, and placed in the frame of a scanner; the upper arch is moved
//! away by a known rigid move. A registration that holds must bring the
//! upper arch back within [`TOL_MM`] (largest displacement of its crowns)
//! and [`TOL_DEG`]; one that does not must be refused. Positives: the front
//! teeth only, one side only, the molars missing, holes over 40-55% of the
//! scan, soft tissue (lips) in front of the teeth, Gaussian noise of 0.05
//! and 0.15 mm, a scan decimated to 1 mm, a scan round the whole arch
//! without its premolars and with the bases of the models, everything at
//! once, and two takes of the same teeth (one of them from another patient
//! in the last case). Negatives: the bite of another patient, a mirrored
//! bite, a mesh that is not a dentition, a bite without the upper teeth.
//!
//! Deterministic: the random spoiling comes from a fixed seed.

use std::sync::LazyLock;

use dca_algo::anatomy::Arch;
use dca_algo::bite::{self, BiteArch, BiteError, BiteScan, ManualInit, Registration};
use dca_algo::demo;
use dca_algo::frame;
use dca_algo::occlusion::{self, ArchModel};
use glam::{DMat3, DMat4, DVec3};

/// Largest displacement of the upper crowns from their bite after the
/// registration of a spoiled scan (mm).
const TOL_MM: f64 = 0.25;
/// Largest rotation of the upper arch from its bite (degrees).
const TOL_DEG: f64 = 0.5;

/// A mesh: vertices (case coordinates) and triangles.
#[derive(Debug, Clone, Default)]
struct Mesh {
    v: Vec<DVec3>,
    t: Vec<[u32; 3]>,
}

impl Mesh {
    fn area(&self) -> f64 {
        self.t
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.v[i as usize]);
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    /// `self` and `other` in one mesh.
    fn merged(mut self, other: &Mesh) -> Mesh {
        let n = self.v.len() as u32;
        self.v.extend(&other.v);
        self.t
            .extend(other.t.iter().map(|t| [t[0] + n, t[1] + n, t[2] + n]));
        self
    }

    /// Moved rigidly by `x` (into the frame of a scanner).
    fn moved(&self, x: &DMat4) -> Mesh {
        Mesh {
            v: self.v.iter().map(|&p| x.transform_point3(p)).collect(),
            t: self.t.clone(),
        }
    }

    /// The faces `keep` picks, with only their vertices.
    fn faces(&self, mut keep: impl FnMut(&[u32; 3]) -> bool) -> Mesh {
        let mut map = vec![u32::MAX; self.v.len()];
        let mut out = Mesh::default();
        for t in self.t.iter().filter(|t| keep(t)) {
            let mut tri = [0; 3];
            for (k, &i) in t.iter().enumerate() {
                if map[i as usize] == u32::MAX {
                    map[i as usize] = out.v.len() as u32;
                    out.v.push(self.v[i as usize]);
                }
                tri[k] = map[i as usize];
            }
            out.t.push(tri);
        }
        out
    }

    fn scan(&self, name: &str) -> BiteScan {
        BiteScan::new(name, &self.v, &self.t).unwrap()
    }
}

/// splitmix64: the random spoiling, the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box-Muller).
    fn normal(&mut self) -> f64 {
        let u = self.uniform().max(1e-300);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Every vertex moved by isotropic Gaussian noise of `sigma` mm per axis.
fn noisy(m: &Mesh, sigma: f64, rng: &mut Rng) -> Mesh {
    let mut out = m.clone();
    for p in &mut out.v {
        *p += DVec3::new(rng.normal(), rng.normal(), rng.normal()) * sigma;
    }
    out
}

/// Holes in patches of `radius` mm until `share` of the area is gone.
fn holed(m: &Mesh, share: f64, radius: f64, rng: &mut Rng) -> Mesh {
    let centre = |t: &[u32; 3]| t.iter().map(|&i| m.v[i as usize]).sum::<DVec3>() / 3.0;
    let total = m.area();
    let mut gone = vec![false; m.t.len()];
    let mut out = m.clone();
    while out.area() > (1.0 - share) * total {
        let c = centre(&m.t[(rng.next() % m.t.len() as u64) as usize]);
        for (k, t) in m.t.iter().enumerate() {
            if centre(t).distance(c) < radius {
                gone[k] = true;
            }
        }
        let mut k = 0;
        out = m.faces(|_| {
            k += 1;
            !gone[k - 1]
        });
    }
    out
}

/// Vertex clustering on a grid of `cell` mm: a low-resolution scan.
fn decimated(m: &Mesh, cell: f64) -> Mesh {
    use std::collections::HashMap;
    let mut cells: HashMap<[i64; 3], (u32, DVec3, f64)> = HashMap::new();
    let mut of = Vec::with_capacity(m.v.len());
    for p in &m.v {
        let key = (*p / cell).floor().as_i64vec3().to_array();
        let n = cells.len() as u32;
        let e = cells.entry(key).or_insert((n, DVec3::ZERO, 0.0));
        e.1 += *p;
        e.2 += 1.0;
        of.push(e.0);
    }
    let mut v = vec![DVec3::ZERO; cells.len()];
    for (i, sum, n) in cells.into_values() {
        v[i as usize] = sum / n;
    }
    let mut t: Vec<[u32; 3]> =
        m.t.iter()
            .map(|t| t.map(|i| of[i as usize]))
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
    t.sort_unstable();
    t.dedup();
    Mesh { v, t }
}

/// A grid of quads over the parameter rectangle, mapped by `f`.
fn sheet(nu: usize, nv: usize, f: impl Fn(f64, f64) -> DVec3) -> Mesh {
    let mut m = Mesh::default();
    for j in 0..=nv {
        for i in 0..=nu {
            m.v.push(f(i as f64 / nu as f64, j as f64 / nv as f64));
        }
    }
    let at = |i: usize, j: usize| (j * (nu + 1) + i) as u32;
    for j in 0..nv {
        for i in 0..nu {
            m.t.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            m.t.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    m
}

/// The two arches in their bite, the upper one also moved by [`y`].
struct World {
    lower: BiteArch,
    upper_true: BiteArch,
    upper_moved: BiteArch,
    /// Crown points of the upper arch in its bite (case coordinates).
    upper_pts: Vec<DVec3>,
}

fn model(arch: Arch, seed: u64, severity: demo::Severity) -> ArchModel {
    let d = demo::run(&demo::Params {
        arch,
        seed,
        severity,
        ..demo::Params::default()
    });
    let v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let labels = d.mesh.vertex_labels.clone().unwrap();
    ArchModel::new(arch, &v, &d.mesh.tris, &labels, &d.seeds, false).unwrap()
}

/// The lower and upper demo arches of these seeds, in their virtual bite.
fn in_bite(lower_seed: u64, upper_seed: u64, severity: demo::Severity) -> (BiteArch, BiteArch) {
    let lower = model(Arch::Lower, lower_seed, severity);
    let upper = model(Arch::Upper, upper_seed, severity);
    let vb = occlusion::align(&lower, &upper, &occlusion::Params::default()).unwrap();
    let upper = ArchModel {
        to_case: vb.delta * upper.to_case,
        ..upper
    };
    (BiteArch::new(lower).unwrap(), BiteArch::new(upper).unwrap())
}

/// The known move of the upper arch away from its bite.
fn y() -> DMat4 {
    DMat4::from_translation(DVec3::new(-3.0, 4.0, 6.0))
        * DMat4::from_axis_angle(DVec3::new(1.0, 0.2, -0.4).normalize(), 0.25)
}

/// Two frames of a scanner.
fn scanner(k: usize) -> DMat4 {
    match k {
        0 => {
            DMat4::from_translation(DVec3::new(14.0, -9.0, 22.0))
                * DMat4::from_axis_angle(DVec3::new(0.3, -1.0, 0.5).normalize(), 0.9)
        }
        _ => {
            DMat4::from_translation(DVec3::new(-30.0, 12.0, 5.0))
                * DMat4::from_axis_angle(DVec3::new(-0.6, 0.2, 1.0).normalize(), 2.1)
        }
    }
}

/// The demo arches of the tests (seeds 7 and 11).
static WORLD: LazyLock<World> = LazyLock::new(|| {
    let (lower, upper_true) = in_bite(7, 11, demo::Severity::Moderate);
    let moved = ArchModel {
        to_case: y() * upper_true.model.to_case,
        ..upper_true.model.clone()
    };
    let (pu, _) = upper_true.model.crown_points(None);
    let upper_pts = pu
        .iter()
        .map(|&p| upper_true.model.to_case.transform_point3(p))
        .collect();
    World {
        upper_moved: BiteArch::new(moved).unwrap(),
        lower,
        upper_true,
        upper_pts,
    }
});

/// Another patient: other demo arches in their own bite.
static OTHER_PATIENT: LazyLock<(BiteArch, BiteArch)> =
    LazyLock::new(|| in_bite(21, 33, demo::Severity::Moderate));

/// A nearly symmetric mouth (mild crowding): its lower arch, its upper arch
/// in the bite, and the upper arch moved by [`y`].
static SYMMETRIC: LazyLock<(BiteArch, BiteArch, BiteArch)> = LazyLock::new(|| {
    let (lower, upper) = in_bite(7, 11, demo::Severity::Mild);
    let moved = ArchModel {
        to_case: y() * upper.model.to_case,
        ..upper.model.clone()
    };
    (lower, upper, BiteArch::new(moved).unwrap())
});

fn world() -> &'static World {
    &WORLD
}

fn other_patient() -> &'static (BiteArch, BiteArch) {
    &OTHER_PATIENT
}

fn tooth(f: u16) -> u16 {
    f % 10
}

fn right(f: u16) -> bool {
    matches!(f / 10, 1 | 4)
}

/// The buccal faces of the arches in their bite within 7 mm of the
/// occlusal plane around the teeth `keep` picks (FDI number), in case
/// coordinates: what a buccal bite scan sees. The gingiva goes with the
/// nearest tooth.
fn cut(lower: &BiteArch, upper: &BiteArch, keep: impl Fn(u16) -> bool) -> Mesh {
    let rel = occlusion::relation(&lower.model, &upper.model);
    let (cl, _) = lower.model.crown_points(None);
    let centre = cl.iter().copied().sum::<DVec3>() / cl.len() as f64;
    let mut out = Mesh::default();
    for (a, to_lower) in [(lower, DMat4::IDENTITY), (upper, rel)] {
        let m = &a.model;
        let back = frame::rigid_inverse(&to_lower);
        let v: Vec<DVec3> = m
            .vertices
            .iter()
            .map(|&p| to_lower.transform_point3(p))
            .collect();
        let whole = Mesh {
            v: v.iter()
                .map(|&p| lower.model.to_case.transform_point3(p))
                .collect(),
            t: m.tris.clone(),
        };
        let part = whole.faces(|t| {
            let [p0, p1, p2] = t.map(|i| v[i as usize]);
            let g = (p0 + p1 + p2) / 3.0;
            let n = (p1 - p0).cross(p2 - p0).normalize_or_zero() * a.sign;
            let radial = DVec3::new(g.x - centre.x, g.y - centre.y, 0.0).normalize_or_zero();
            if g.z.abs() >= 7.0 || n.dot(radial) <= 0.2 {
                return false;
            }
            let own = m.labels[t[0] as usize];
            let fdi = if own != 0 {
                own
            } else {
                a.crowns
                    .nearest(back.transform_point3(g), 3.0)
                    .map_or(0, |(_, f, _)| {
                        m.labels[a.crowns.tris[f as usize][0] as usize]
                    })
            };
            fdi != 0 && keep(fdi)
        });
        out = out.merged(&part);
    }
    out
}

/// Lips the scanner caught round the front (soft tissue): two bumpy
/// curved sheets 1.5 mm outside the outline of `scan`, above and below it
/// (case coordinates).
fn lips(scan: &Mesh, lower: &BiteArch) -> Mesh {
    let to_arch = frame::rigid_inverse(&lower.model.to_case);
    let (cl, _) = lower.model.crown_points(None);
    let centre = cl.iter().copied().sum::<DVec3>() / cl.len() as f64;
    // the outline of the scan round the front, every 5 degrees
    let bin = |p: DVec3| ((p.x - centre.x).atan2(p.y - centre.y).to_degrees() / 5.0).round() as i64;
    let mut outline = std::collections::BTreeMap::new();
    for p in &scan.v {
        let q = to_arch.transform_point3(*p);
        let r = DVec3::new(q.x - centre.x, q.y - centre.y, 0.0).length();
        let e = outline.entry(bin(q)).or_insert(0.0_f64);
        *e = e.max(r);
    }
    let mut out = Mesh::default();
    for (z0, z1) in [(-12.0, -7.5), (7.5, 12.0)] {
        let s = sheet(32, 8, |u, v| {
            let th = (u - 0.5) * 80f64.to_radians();
            let edge = outline
                .get(&((th.to_degrees() / 5.0).round() as i64))
                .copied()
                .unwrap_or(0.0);
            let r = edge + 1.5 + 0.4 * (7.0 * th).sin() * (9.0 * v).cos();
            let p = DVec3::new(
                centre.x + r * th.sin(),
                centre.y + r * th.cos(),
                z0 + (z1 - z0) * v,
            );
            lower.model.to_case.transform_point3(p)
        });
        out = out.merged(&s);
    }
    out
}

/// The bases of plaster models round the arches (garbage far from the
/// teeth): two plates 20 mm above and below the occlusal plane.
fn bases(lower: &BiteArch) -> Mesh {
    let mut out = Mesh::default();
    for z in [-20.0, 20.0] {
        let s = sheet(20, 20, |u, v| {
            let p = DVec3::new(-35.0 + 70.0 * u, -15.0 + 60.0 * v, z);
            lower.model.to_case.transform_point3(p)
        });
        out = out.merged(&s);
    }
    out
}

/// `m` mirrored left to right (across the midsagittal plane of the lower
/// arch).
fn mirrored(m: &Mesh, lower: &BiteArch) -> Mesh {
    let to_case = lower.model.to_case;
    let x =
        to_case * DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)) * frame::rigid_inverse(&to_case);
    m.moved(&x)
}

/// A bumpy ellipsoid the size of a bite scan: not a dentition.
fn blob() -> Mesh {
    sheet(60, 30, |u, v| {
        let (th, ph) = (u * std::f64::consts::TAU, (v - 0.5) * std::f64::consts::PI);
        let r = 1.0 + 0.08 * (5.0 * th).sin() * (4.0 * ph).cos();
        DVec3::new(
            15.0 * r * ph.cos() * th.cos(),
            10.0 * r * ph.cos() * th.sin(),
            8.0 * r * ph.sin(),
        )
    })
}

fn register(scans: &[(&str, Mesh)]) -> Result<Registration, BiteError> {
    let w = world();
    let bites: Vec<BiteScan> = scans.iter().map(|(n, m)| m.scan(n)).collect();
    bite::register(&w.lower, &w.upper_moved, &bites, &ManualInit::default())
}

fn rot_deg(m: &DMat4) -> f64 {
    let r = DMat3::from_mat4(*m);
    let tr = r.x_axis.x + r.y_axis.y + r.z_axis.z;
    ((tr - 1.0) * 0.5).clamp(-1.0, 1.0).acos().to_degrees()
}

/// The registration holds: the upper arch is back in its bite.
fn holds(name: &str, scans: &[(&str, Mesh)]) -> Registration {
    let reg = register(scans).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
    let back = reg.upper_delta * y();
    let d = world()
        .upper_pts
        .iter()
        .map(|&p| back.transform_point3(p).distance(p))
        .fold(0.0, f64::max);
    let deg = rot_deg(&back);
    eprintln!(
        "bite robustness {name}: upper arch {d:.3} mm, {deg:.3} deg; reasons {:?}",
        reg.reasons
    );
    assert!(d < TOL_MM && deg < TOL_DEG, "{name}: {d} mm, {deg} deg");
    assert!(reg.facing < -0.99, "{name}: {reg:?}");
    reg
}

/// The registration is refused.
fn refused(name: &str, scans: &[(&str, Mesh)]) -> BiteError {
    match register(scans) {
        Ok(reg) => {
            let back = reg.upper_delta * y();
            panic!(
                "{name}: registered ({:.2} deg off): {:?}",
                rot_deg(&back),
                reg.reasons
            )
        }
        Err(e) => {
            eprintln!("bite robustness {name}: refused: {e}");
            e
        }
    }
}

fn has(reg: &Registration, code: &str) -> bool {
    reg.reasons
        .iter()
        .any(|r| r == code || r.starts_with(&format!("{code}:")))
}

fn true_cut(keep: impl Fn(u16) -> bool) -> Mesh {
    let w = world();
    cut(&w.lower, &w.upper_true, keep)
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn the_front_teeth_alone_register() {
    let m = true_cut(|f| tooth(f) <= 3).moved(&scanner(0));
    let reg = holds("front only", &[("front", m)]);
    assert!(has(&reg, "front_only"), "{:?}", reg.reasons);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn one_side_alone_registers() {
    let m = true_cut(|f| right(f) && (3..=7).contains(&tooth(f))).moved(&scanner(0));
    let reg = holds("one side", &[("right", m)]);
    assert!(has(&reg, "one_side_only"), "{:?}", reg.reasons);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_scan_without_its_molars_registers() {
    let m = true_cut(|f| tooth(f) <= 5).moved(&scanner(0));
    let reg = holds("molars missing", &[("no-molars", m)]);
    assert!(!has(&reg, "front_only") && !has(&reg, "one_side_only"));
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_scan_full_of_holes_registers() {
    let mut rng = Rng(1);
    let m = holed(&true_cut(|f| tooth(f) <= 6), 0.45, 2.5, &mut rng);
    holds("45% holes", &[("holes", m.moved(&scanner(0)))]);
    let mut rng = Rng(2);
    let m = holed(&true_cut(|f| tooth(f) <= 4), 0.55, 1.5, &mut rng);
    holds("55% holes, front", &[("holes", m.moved(&scanner(1)))]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn soft_tissue_around_the_teeth_is_ignored() {
    let w = world();
    let m = true_cut(|f| tooth(f) <= 4);
    let soft = lips(&m, &w.lower);
    let m = m.merged(&soft);
    holds("lips", &[("lips", m.moved(&scanner(0)))]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_noisy_scan_registers() {
    let mut rng = Rng(3);
    let m = noisy(&true_cut(|f| tooth(f) <= 6), 0.05, &mut rng);
    holds("noise 0.05 mm", &[("noise", m.moved(&scanner(0)))]);
    let mut rng = Rng(4);
    let m = noisy(&true_cut(|f| tooth(f) <= 6), 0.15, &mut rng);
    let reg = holds("noise 0.15 mm", &[("noise", m.moved(&scanner(1)))]);
    // a noisy scan is a reason to review, never a refusal
    assert!(has(&reg, "rms_above_good"), "{:?}", reg.reasons);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_low_resolution_scan_registers() {
    let m = decimated(&true_cut(|f| tooth(f) <= 6), 1.0);
    holds("decimated to 1 mm", &[("coarse", m.moved(&scanner(0)))]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_scan_round_the_arch_without_its_middle_registers() {
    // models in occlusion on a desktop scanner: round the whole arch, the
    // premolars missing, holes, the bases of the models
    let w = world();
    let mut rng = Rng(5);
    let m = holed(
        &true_cut(|f| !(4..=5).contains(&tooth(f))),
        0.3,
        2.0,
        &mut rng,
    )
    .merged(&bases(&w.lower));
    let reg = holds("round the arch", &[("models", m.moved(&scanner(1)))]);
    assert!(!has(&reg, "front_only") && !has(&reg, "one_side_only"));
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_poor_scan_of_the_front_teeth_registers() {
    // everything at once: front and canines only with the lips, holes over
    // it all, noise, low resolution
    let w = world();
    let mut rng = Rng(6);
    let m = true_cut(|f| tooth(f) <= 4);
    let soft = lips(&m, &w.lower);
    let m = holed(&m.merged(&soft), 0.35, 2.0, &mut rng);
    let m = decimated(&noisy(&m, 0.1, &mut rng), 0.6);
    holds("poor front scan", &[("poor", m.moved(&scanner(0)))]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn two_takes_of_the_same_teeth_register_together() {
    let mut rng = Rng(7);
    let a = holed(
        &noisy(&true_cut(|f| tooth(f) <= 4), 0.08, &mut rng),
        0.3,
        2.0,
        &mut rng,
    );
    let b = holed(
        &noisy(&true_cut(|f| tooth(f) <= 4), 0.08, &mut rng),
        0.3,
        2.0,
        &mut rng,
    );
    let reg = holds(
        "two takes",
        &[
            ("take-1", a.moved(&scanner(0))),
            ("take-2", b.moved(&scanner(1))),
        ],
    );
    assert_eq!(reg.scans.len(), 2);
    assert!(reg.ignored.is_empty());
    assert!(reg.agreement_mm.unwrap() < 0.1, "{reg:?}");
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_take_that_does_not_fit_is_left_out() {
    let (ol, ou) = other_patient();
    let good = true_cut(|f| tooth(f) <= 5).moved(&scanner(0));
    let bad = cut(ol, ou, |f| tooth(f) <= 5).moved(&scanner(1));
    let reg = holds("one bad take", &[("good", good), ("bad", bad)]);
    assert_eq!(reg.scans.len(), 1);
    assert_eq!(reg.scans[0].scan, "good");
    assert_eq!(reg.ignored.len(), 1);
    assert!(has(&reg, "scan_ignored:bad"), "{:?}", reg.reasons);
    // the bad take stays where it was
    assert_eq!(reg.bite_deltas.len(), 2);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn the_bite_of_another_patient_is_refused() {
    let (ol, ou) = other_patient();
    let m = cut(ol, ou, |f| tooth(f) <= 6).moved(&scanner(0));
    refused("another patient", &[("other", m)]);
    let m = cut(ol, ou, |f| right(f) && (3..=7).contains(&tooth(f))).moved(&scanner(1));
    refused("another patient, one side", &[("other", m)]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_mirrored_bite_is_refused() {
    let w = world();
    let m = mirrored(&true_cut(|f| tooth(f) <= 6), &w.lower).moved(&scanner(0));
    refused("mirrored", &[("mirrored", m)]);
    let m = mirrored(
        &true_cut(|f| right(f) && (3..=7).contains(&tooth(f))),
        &w.lower,
    )
    .moved(&scanner(1));
    refused("mirrored side", &[("mirrored", m)]);
    // a nearly symmetric mouth, whose mirrored teeth almost fit
    let (lower, upper, moved) = &*SYMMETRIC;
    let m = mirrored(&cut(lower, upper, |f| tooth(f) <= 6), lower).moved(&scanner(0));
    let b = m.scan("mirrored");
    let got = bite::register(lower, moved, &[b], &ManualInit::default());
    assert!(
        got.is_err(),
        "symmetric mouth, mirrored: {:?}",
        got.map(|r| r.reasons)
    );
    eprintln!(
        "bite robustness mirrored symmetric mouth: refused: {}",
        got.unwrap_err()
    );
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_mesh_that_is_not_a_dentition_is_refused() {
    refused("blob", &[("blob", blob().moved(&scanner(0)))]);
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_bite_without_the_upper_teeth_is_refused() {
    let w = world();
    let m = true_cut(|f| tooth(f) <= 6);
    let to_arch = frame::rigid_inverse(&w.lower.model.to_case);
    let lower_only = m.faces(|t| {
        t.iter()
            .all(|&i| to_arch.transform_point3(m.v[i as usize]).z < -0.5)
    });
    refused(
        "no upper teeth",
        &[("lower", lower_only.moved(&scanner(0)))],
    );
}
