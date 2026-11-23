//! The checked automatic seeds (`autoseed::run`, `propose_checked`) on demo
//! arches whose scan runs past the last molars: a smooth bulge of gum behind
//! each last molar (the tuberosity, the retromolar pad or the back wall of a
//! plaster model), up to about a millimetre below the crown tops, as on
//! real scans of models; or a rough heel of the base of a printed model as
//! high as the crowns, as on the printed test piece. The plugin's proposal
//! (`autoseed::propose`) must cover the whole profiled arch and takes the
//! bulges and the heels as teeth: a third molar, or stretched teeth with
//! every number after them shifted. The checked proposal leaves them out
//! and finds the true teeth.

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::autoseed::{self, Doubt, Input, Options, Params};
use dca_algo::demo::{self, Severity};
use glam::{DVec2, DVec3};

/// A closed ellipsoid: centre `c`, semi-axes `a` along `u` (unit, in the
/// occlusal plane), `b` across it and `h` up.
fn ellipsoid(c: DVec3, u: DVec2, a: f64, b: f64, h: f64) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    const RINGS: u32 = 24;
    const SECTORS: u32 = 48;
    let w = DVec2::new(-u.y, u.x);
    let mut v = vec![c + DVec3::Z * h];
    for i in 1..RINGS {
        let th = std::f64::consts::PI * f64::from(i) / f64::from(RINGS);
        for j in 0..SECTORS {
            let ph = std::f64::consts::TAU * f64::from(j) / f64::from(SECTORS);
            let p = u * (a * th.sin() * ph.cos()) + w * (b * th.sin() * ph.sin());
            v.push(c + DVec3::new(p.x, p.y, h * th.cos()));
        }
    }
    v.push(c - DVec3::Z * h);
    let ring = |i: u32, j: u32| 1 + (i - 1) * SECTORS + j % SECTORS;
    let bottom = 1 + (RINGS - 1) * SECTORS;
    let mut t = Vec::new();
    for j in 0..SECTORS {
        t.push([0, ring(1, j), ring(1, j + 1)]);
        for i in 1..RINGS - 1 {
            let (p, q, r, s) = (
                ring(i, j),
                ring(i + 1, j),
                ring(i + 1, j + 1),
                ring(i, j + 1),
            );
            t.push([p, q, r]);
            t.push([p, r, s]);
        }
        t.push([ring(RINGS - 1, j), bottom, ring(RINGS - 1, j + 1)]);
    }
    (v, t)
}

/// A demo arch with a bulge of gum behind each last molar: 9 mm long, its
/// top `below` mm under the molar's top.
fn arch_with_bulges(arch: Arch, seed: u64, below: f64) -> (demo::Demo, Vec<DVec3>, Vec<[u32; 3]>) {
    let d = demo::run(&demo::Params {
        arch,
        severity: Severity::Mild,
        seed,
        third_molars: false,
    });
    let mut v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let mut t = d.mesh.tris.clone();
    let n = d.seeds.len();
    for (last, next) in [(0, 1), (n - 1, n - 2)] {
        let (p, q) = (d.seeds[last].position, d.seeds[next].position);
        let u = (p - q).truncate().normalize();
        let c = p + (u * 10.0).extend(-below - 3.0);
        let (bv, bt) = ellipsoid(c, u, 4.5, 4.0, 3.0);
        let base = v.len() as u32;
        v.extend(bv);
        t.extend(bt.iter().map(|f| f.map(|i| i + base)));
    }
    (d, v, t)
}

fn teeth(seeds: &[dca_model::Seed]) -> Vec<Fdi> {
    seeds.iter().map(|s| s.fdi).collect()
}

/// The seeds that are not on their own crown (the nearest vertex carries
/// another label).
fn off_crown(d: &demo::Demo, v: &[DVec3], seeds: &[dca_model::Seed]) -> Vec<Fdi> {
    off_crown_in(d.mesh.vertex_labels.as_ref().unwrap(), v, seeds)
}

/// [`off_crown`] with the label of each of the first `labels.len()`
/// vertices of `v`.
fn off_crown_in(labels: &[u16], v: &[DVec3], seeds: &[dca_model::Seed]) -> Vec<Fdi> {
    seeds
        .iter()
        .filter(|s| {
            let j = (0..labels.len())
                .min_by(|&a, &b| {
                    (v[a] - s.position)
                        .length_squared()
                        .total_cmp(&(v[b] - s.position).length_squared())
                })
                .unwrap();
            labels[j] != u16::from(s.fdi.get())
        })
        .map(|s| s.fdi)
        .collect()
}

/// A demo arch with erupted third molars, one of them (the first tooth of
/// the arch order when `first`, else the last) taken out and replaced by a
/// bulge of gum behind the second molar, its top `below` mm under the
/// molar's top. The true teeth, the label of each demo vertex left, the
/// mesh.
fn arch_with_one_third_molar(
    arch: Arch,
    seed: u64,
    first: bool,
    below: f64,
) -> (Vec<Fdi>, Vec<u16>, Vec<DVec3>, Vec<[u32; 3]>) {
    let d = demo::run(&demo::Params {
        arch,
        severity: Severity::Mild,
        seed,
        third_molars: true,
    });
    let labels = d.mesh.vertex_labels.as_ref().unwrap();
    let n = d.seeds.len();
    let (gone, last, next) = if first {
        (0, 1, 2)
    } else {
        (n - 1, n - 2, n - 3)
    };
    let fdi = u16::from(d.seeds[gone].fdi.get());
    let mut index = vec![u32::MAX; labels.len()];
    let mut v = Vec::new();
    let mut kept = Vec::new();
    for (i, (p, &l)) in d.mesh.positions.iter().zip(labels).enumerate() {
        if l != fdi {
            index[i] = v.len() as u32;
            v.push(p.as_dvec3());
            kept.push(l);
        }
    }
    let mut t: Vec<[u32; 3]> = d
        .mesh
        .tris
        .iter()
        .filter(|f| f.iter().all(|&i| index[i as usize] != u32::MAX))
        .map(|f| f.map(|i| index[i as usize]))
        .collect();
    let (p, q) = (d.seeds[last].position, d.seeds[next].position);
    let u = (p - q).truncate().normalize();
    let c = p + (u * 10.0).extend(-below - 3.0);
    let (bv, bt) = ellipsoid(c, u, 4.5, 4.0, 3.0);
    let base = v.len() as u32;
    v.extend(bv);
    t.extend(bt.iter().map(|f| f.map(|i| i + base)));
    let truth = d
        .seeds
        .iter()
        .map(|s| s.fdi)
        .filter(|f| u16::from(f.get()) != fdi)
        .collect();
    (truth, kept, v, t)
}

fn checked(arch: Arch, v: &[DVec3], t: &[[u32; 3]], missing: &[u8]) -> autoseed::Output {
    autoseed::run(
        &Input {
            arch,
            vertices: v,
            triangles: t,
            occlusal_flipped: false,
        },
        &Params {
            missing: missing.to_vec(),
            ..Default::default()
        },
    )
    .unwrap()
}

/// The bulges are not third molars: the plugin numbers them 18 and 28 (38
/// and 48), the checked proposal finds the true teeth, each seed on its
/// own crown.
#[test]
fn the_gum_behind_the_last_molars_is_not_a_tooth() {
    for (arch, seed) in [(Arch::Lower, 3), (Arch::Upper, 5), (Arch::Lower, 7)] {
        let name = format!("{arch:?} seed {seed}");
        let (d, v, t) = arch_with_bulges(arch, seed, 1.0);
        let truth = teeth(&d.seeds);
        let plugin = autoseed::propose(&v, &t, arch, &Options::default(), None).unwrap();
        assert!(
            plugin.seeds.iter().any(|s| s.fdi.get() % 10 == 8),
            "{name}: the plugin finds third molars"
        );
        let out = checked(arch, &v, &t, &[]);
        assert_eq!(teeth(&out.seeds), truth, "{name}");
        assert_eq!(off_crown(&d, &v, &out.seeds), Vec::<Fdi>::new(), "{name}");
        let check = out.diagnostics.checked.as_ref().expect("checked");
        assert!(check.doubts.contains(&Doubt::ThirdMolar), "{name}");
        assert_eq!(check.plugin_teeth.len(), truth.len() + 2, "{name}");
    }
}

/// With the third molars marked missing by hand the plugin stretches the
/// teeth over the bulges: seeds on the neighbouring crown, two seeds on
/// one crown. The checked proposal puts every seed on its own crown.
#[test]
fn the_teeth_are_not_stretched_over_the_gum() {
    for seed in [3, 5, 7] {
        let name = format!("upper seed {seed}");
        let (d, v, t) = arch_with_bulges(Arch::Upper, seed, 0.8);
        let missing = [18, 28];
        let opts = Params {
            missing: missing.to_vec(),
            ..Default::default()
        }
        .options()
        .unwrap();
        let plugin = autoseed::propose(&v, &t, Arch::Upper, &opts, None).unwrap();
        assert!(
            !off_crown(&d, &v, &plugin.seeds).is_empty(),
            "{name}: the plugin stretches the teeth"
        );
        let out = checked(Arch::Upper, &v, &t, &missing);
        assert_eq!(teeth(&out.seeds), teeth(&d.seeds), "{name}");
        assert_eq!(off_crown(&d, &v, &out.seeds), Vec::<Fdi>::new(), "{name}");
        assert!(out.diagnostics.checked.is_some(), "{name}");
    }
}

/// Erupted third molars, not looked for: the plugin numbers them, they are
/// crowns, the checked proposal cannot leave them out and keeps the
/// plugin's teeth.
#[test]
fn erupted_third_molars_are_kept() {
    for (arch, seed) in [(Arch::Lower, 3), (Arch::Upper, 5), (Arch::Lower, 7)] {
        let name = format!("{arch:?} seed {seed}");
        let d = demo::run(&demo::Params {
            arch,
            severity: Severity::Mild,
            seed,
            third_molars: true,
        });
        let v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
        let plugin = autoseed::propose(&v, &d.mesh.tris, arch, &Options::default(), None).unwrap();
        assert_eq!(teeth(&plugin.seeds), teeth(&d.seeds), "{name}: plugin");
        let out = checked(arch, &v, &d.mesh.tris, &[]);
        assert_eq!(teeth(&out.seeds), teeth(&d.seeds), "{name}");
        assert_eq!(off_crown(&d, &v, &out.seeds), Vec::<Fdi>::new(), "{name}");
        assert!(out.diagnostics.checked.is_none(), "{name}");
    }
}

/// An erupted third molar on one side and a bulge of gum behind the last
/// molar on the other, upper and lower, either side: the plugin numbers
/// the bulge as a third molar; the checked proposal keeps the erupted
/// third molar (a crown) and leaves the bulge out.
#[test]
fn one_erupted_third_molar_keeps_only_its_side() {
    for (arch, seed) in [(Arch::Lower, 3), (Arch::Upper, 5)] {
        for first in [true, false] {
            let name = format!("{arch:?} seed {seed} bulge first {first}");
            let (truth, labels, v, t) = arch_with_one_third_molar(arch, seed, first, 1.0);
            let plugin = autoseed::propose(&v, &t, arch, &Options::default(), None).unwrap();
            assert_eq!(
                plugin
                    .seeds
                    .iter()
                    .filter(|s| s.fdi.get() % 10 == 8)
                    .count(),
                2,
                "{name}: the plugin numbers the bulge as a third molar"
            );
            let out = checked(arch, &v, &t, &[]);
            assert_eq!(teeth(&out.seeds), truth, "{name}");
            assert_eq!(
                off_crown_in(&labels, &v, &out.seeds),
                Vec::<Fdi>::new(),
                "{name}"
            );
            let check = out.diagnostics.checked.as_ref().expect("checked");
            assert!(check.doubts.contains(&Doubt::ThirdMolar), "{name}");
            assert_eq!(check.plugin_teeth.len(), truth.len() + 1, "{name}");
        }
    }
}

/// Without a doubt the checked proposal is the plugin's, bit for bit.
#[test]
fn a_plausible_proposal_is_the_plugins() {
    let d = demo::run(&demo::Params {
        arch: Arch::Lower,
        severity: Severity::Mild,
        seed: 3,
        third_molars: false,
    });
    let v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let plugin =
        autoseed::propose(&v, &d.mesh.tris, Arch::Lower, &Options::default(), None).unwrap();
    let checked =
        autoseed::propose_checked(&v, &d.mesh.tris, Arch::Lower, &Options::default(), None)
            .unwrap();
    assert_eq!(checked, plugin);
    assert!(checked.diagnostics.checked.is_none());
    assert_eq!(teeth(&checked.seeds), teeth(&d.seeds));
}

/// A block of the base of a printed model behind a last molar (a heel): a
/// closed box whose top is centred on `c`, `a` mm along `u` (unit, in the
/// occlusal plane) and `b` mm across it on each side of `c`, `depth` mm
/// deep. The top is a grid of 0.5 mm raised and lowered by up to `rough` mm,
/// as the rough top of a printed base, which the crown score takes for
/// enamel; its border stays at the height of `c`.
fn heel(c: DVec3, u: DVec2, a: f64, b: f64, depth: f64, rough: f64) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    const STEP: f64 = 0.5;
    let w = DVec2::new(-u.y, u.x);
    let (nu, nw) = ((2.0 * a / STEP) as u32 + 1, (2.0 * b / STEP) as u32 + 1);
    let mut v = Vec::new();
    for k in 0..2 {
        for i in 0..nu {
            for j in 0..nw {
                let border = i == 0 || j == 0 || i == nu - 1 || j == nw - 1;
                let z = match k {
                    0 if border => 0.0,
                    0 => rough * (f64::from(i) * 1.9).sin() * (f64::from(j) * 2.3).cos(),
                    _ => -depth,
                };
                let p = u * (-a + f64::from(i) * STEP) + w * (-b + f64::from(j) * STEP);
                v.push(c + DVec3::new(p.x, p.y, z));
            }
        }
    }
    let id = |k: u32, i: u32, j: u32| k * nu * nw + i * nw + j;
    let mut t = Vec::new();
    for i in 0..nu - 1 {
        for j in 0..nw - 1 {
            let [p, q, r, s] = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            t.push([id(0, p.0, p.1), id(0, q.0, q.1), id(0, r.0, r.1)]);
            t.push([id(0, p.0, p.1), id(0, r.0, r.1), id(0, s.0, s.1)]);
            t.push([id(1, p.0, p.1), id(1, r.0, r.1), id(1, q.0, q.1)]);
            t.push([id(1, p.0, p.1), id(1, s.0, s.1), id(1, r.0, r.1)]);
        }
    }
    // the sides, along the border of the grid (counter-clockwise from above)
    let mut ring: Vec<(u32, u32)> = (0..nu - 1).map(|i| (i, 0)).collect();
    ring.extend((0..nw - 1).map(|j| (nu - 1, j)));
    ring.extend((1..nu).rev().map(|i| (i, nw - 1)));
    ring.extend((1..nw).rev().map(|j| (0, j)));
    for k in 0..ring.len() {
        let ((i0, j0), (i1, j1)) = (ring[k], ring[(k + 1) % ring.len()]);
        let (p0, p1, q0, q1) = (id(0, i0, j0), id(0, i1, j1), id(1, i0, j0), id(1, i1, j1));
        t.push([p0, q0, q1]);
        t.push([p0, q1, p1]);
    }
    (v, t)
}

/// A demo arch with a heel of the base of a printed model behind each last
/// molar, as on the printed test piece of the project: 10 mm long, 24 mm
/// wide across the arch (wider than any crown), half a millimetre behind
/// the distal surface of the molar, its rough top `rise` mm over the top of
/// the molar.
fn arch_with_heels(arch: Arch, seed: u64, rise: f64) -> (demo::Demo, Vec<DVec3>, Vec<[u32; 3]>) {
    let d = demo::run(&demo::Params {
        arch,
        severity: Severity::Mild,
        seed,
        third_molars: false,
    });
    let mut v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let mut t = d.mesh.tris.clone();
    let n = d.seeds.len();
    for (last, next) in [(0, 1), (n - 1, n - 2)] {
        let (p, q) = (d.seeds[last].position, d.seeds[next].position);
        let u = (p - q).truncate().normalize();
        let half = 0.5 * dca_algo::anatomy::typical_diameters(d.seeds[last].fdi).0;
        let c = p + (u * (half + 0.5 + 5.0)).extend(rise);
        let (bv, bt) = heel(c, u, 5.0, 12.0, 12.0, 0.6);
        let base = v.len() as u32;
        v.extend(bv);
        t.extend(bt.iter().map(|f| f.map(|i| i + base)));
    }
    (d, v, t)
}

/// The heels of the base of a printed model behind the last molars, as
/// high as the crowns (0 mm), a little lower (0.5 mm) or higher (0.5 and
/// 1.5 mm), rough: the plugin's crown score takes their top for enamel, its
/// arch runs onto them and its sequence covers them with a tooth of its own
/// (a third molar, or every number shifted), which the old check kept: it
/// leaves out only what stays lower than the last crown. The checked
/// proposal finds that its end teeth are no crowns (the scan does not fall
/// on both sides of them across the arch, [`Doubt::EndNotTooth`]) and finds
/// the ends of the arch again: the true teeth, each seed on its own crown,
/// nothing on a heel. The plugin's proposal stays in the diagnostics.
#[test]
fn the_base_of_a_model_behind_the_last_molars_is_not_a_tooth() {
    let (mut cases, mut fooled) = (0, 0);
    for (arch, seed) in [
        (Arch::Upper, 3),
        (Arch::Upper, 5),
        (Arch::Upper, 7),
        (Arch::Lower, 3),
        (Arch::Lower, 5),
        (Arch::Lower, 7),
    ] {
        for rise in [-0.5, 0.0, 0.5, 1.5] {
            let name = format!("{arch:?} seed {seed} heels {rise} mm over the molars");
            let (d, v, t) = arch_with_heels(arch, seed, rise);
            let truth = teeth(&d.seeds);
            let plugin = autoseed::propose(&v, &t, arch, &Options::default(), None).unwrap();
            let out = checked(arch, &v, &t, &[]);
            assert_eq!(teeth(&out.seeds), truth, "{name}");
            assert_eq!(off_crown(&d, &v, &out.seeds), Vec::<Fdi>::new(), "{name}");
            cases += 1;
            if teeth(&plugin.seeds) != truth {
                fooled += 1;
                let check = out.diagnostics.checked.as_ref().expect("checked");
                assert!(check.doubts.contains(&Doubt::EndNotTooth), "{name}");
                let plugin_teeth: Vec<u8> = plugin.seeds.iter().map(|s| s.fdi.get()).collect();
                assert_eq!(check.plugin_teeth, plugin_teeth, "{name}");
            }
        }
    }
    // the heels fool the plugin in most cases: the check is what finds the
    // true teeth there
    assert!(
        3 * fooled >= 2 * cases,
        "the plugin fooled {fooled} times of {cases}"
    );
}

/// The ridge of a missing tooth as the gum makes it: a closed block whose
/// top is a rough hump centred on `c` (its top), `a` mm long along `u`
/// (unit, in the occlusal plane) on each side of `c`, falling across the
/// arch as a parabola, `fall` mm at `b` mm from its middle, as gum does (no
/// walls), 6 mm deep. Its top is a grid of 0.2 mm raised and lowered by up
/// to `rough` mm, which the crown score takes for enamel.
fn ridge(c: DVec3, u: DVec2, a: f64, b: f64, fall: f64, rough: f64) -> (Vec<DVec3>, Vec<[u32; 3]>) {
    const STEP: f64 = 0.2;
    const DEPTH: f64 = 6.0;
    let w = DVec2::new(-u.y, u.x);
    let (nu, nw) = ((2.0 * a / STEP) as u32 + 1, (2.0 * b / STEP) as u32 + 1);
    let mut v = Vec::new();
    for k in 0..2 {
        for i in 0..nu {
            for j in 0..nw {
                let (x, y) = (-a + f64::from(i) * STEP, -b + f64::from(j) * STEP);
                let z = match k {
                    0 => {
                        -fall * (y / b).powi(2)
                            + rough * (f64::from(i) * 1.9).sin() * (f64::from(j) * 2.3).cos()
                    }
                    _ => -DEPTH,
                };
                let p = u * x + w * y;
                v.push(c + DVec3::new(p.x, p.y, z));
            }
        }
    }
    let id = |k: u32, i: u32, j: u32| k * nu * nw + i * nw + j;
    let mut t = Vec::new();
    for i in 0..nu - 1 {
        for j in 0..nw - 1 {
            let [p, q, r, s] = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            t.push([id(0, p.0, p.1), id(0, q.0, q.1), id(0, r.0, r.1)]);
            t.push([id(0, p.0, p.1), id(0, r.0, r.1), id(0, s.0, s.1)]);
            t.push([id(1, p.0, p.1), id(1, r.0, r.1), id(1, q.0, q.1)]);
            t.push([id(1, p.0, p.1), id(1, s.0, s.1), id(1, r.0, r.1)]);
        }
    }
    // the sides, along the border of the grid (counter-clockwise from above)
    let mut ring: Vec<(u32, u32)> = (0..nu - 1).map(|i| (i, 0)).collect();
    ring.extend((0..nw - 1).map(|j| (nu - 1, j)));
    ring.extend((1..nu).rev().map(|i| (i, nw - 1)));
    ring.extend((1..nw).rev().map(|j| (0, j)));
    for k in 0..ring.len() {
        let ((i0, j0), (i1, j1)) = (ring[k], ring[(k + 1) % ring.len()]);
        let (p0, p1, q0, q1) = (id(0, i0, j0), id(0, i1, j1), id(1, i0, j0), id(1, i1, j1));
        t.push([p0, q0, q1]);
        t.push([p0, q1, p1]);
    }
    (v, t)
}

/// A demo arch without the tooth `gone`: in its place the ridge of a
/// missing tooth as on the printed test piece (rough, its top `below` mm
/// under the top of the tooth that is gone, running under the crowns of
/// the neighbours with no valley in between, falling across the arch as
/// gum: 3 mm at 6 mm from its middle). The true teeth, the label of every
/// demo vertex left, the teeth beside the ridge, the mesh.
#[allow(clippy::type_complexity)]
fn arch_with_ridge(
    arch: Arch,
    seed: u64,
    gone: u8,
    below: f64,
) -> (Vec<Fdi>, Vec<u16>, [u8; 2], Vec<DVec3>, Vec<[u32; 3]>) {
    let d = demo::run(&demo::Params {
        arch,
        severity: Severity::Mild,
        seed,
        third_molars: false,
    });
    let labels = d.mesh.vertex_labels.as_ref().unwrap();
    let k = d.seeds.iter().position(|s| s.fdi.get() == gone).unwrap();
    let mut index = vec![u32::MAX; labels.len()];
    let (mut v, mut kept) = (Vec::new(), Vec::new());
    for (i, (p, &l)) in d.mesh.positions.iter().zip(labels).enumerate() {
        if l != u16::from(gone) {
            index[i] = v.len() as u32;
            v.push(p.as_dvec3());
            kept.push(l);
        }
    }
    let mut t: Vec<[u32; 3]> = d
        .mesh
        .tris
        .iter()
        .filter(|f| f.iter().all(|&i| index[i as usize] != u32::MAX))
        .map(|f| f.map(|i| index[i as usize]))
        .collect();
    let (prev, next) = (d.seeds[k - 1], d.seeds[k + 1]);
    let u = (next.position - prev.position).truncate().normalize();
    let md = dca_algo::anatomy::typical_diameters(d.seeds[k].fdi).0;
    let top = d.seeds[k].position - DVec3::Z * below;
    let (rv, rt) = ridge(top, u, 0.5 * md + 1.0, 6.0, 3.0, 0.1);
    let base = v.len() as u32;
    v.extend(rv);
    t.extend(rt.iter().map(|f| f.map(|i| i + base)));
    let truth = d
        .seeds
        .iter()
        .map(|s| s.fdi)
        .filter(|f| f.get() != gone)
        .collect();
    (truth, kept, [prev.fdi.get(), next.fdi.get()], v, t)
}

/// The ridge of a missing tooth 3-5 mm under the crowns beside it, rough
/// as on the printed test piece (21 missing there), with no tooth marked
/// missing: the checked proposal finds a stretch with no crown
/// ([`Doubt::EmptyRidge`]: it sinks between the crowns, nothing stands on
/// it, it falls across the arch as gum) and counts it as the space of a
/// missing tooth: the true teeth, each seed on its own crown, and the
/// teeth beside the space named. On the test piece the plugin is fooled by
/// such a ridge (a seed on it, or every number on its side shifted).
#[test]
fn the_ridge_of_a_missing_tooth_is_a_space() {
    for (arch, seed, gone, below) in [
        (Arch::Upper, 3, 21, 5.0),
        (Arch::Upper, 5, 21, 5.0),
        (Arch::Upper, 7, 12, 4.0),
        (Arch::Upper, 3, 24, 3.0),
        (Arch::Lower, 5, 31, 3.0),
        (Arch::Lower, 5, 31, 4.0),
    ] {
        let name = format!("{arch:?} seed {seed} without {gone}, ridge {below} mm under");
        let (truth, labels, beside, v, t) = arch_with_ridge(arch, seed, gone, below);
        let out = checked(arch, &v, &t, &[]);
        assert_eq!(teeth(&out.seeds), truth, "{name}");
        assert_eq!(
            off_crown_in(&labels, &v, &out.seeds),
            Vec::<Fdi>::new(),
            "{name}"
        );
        let check = out.diagnostics.checked.as_ref().expect("checked");
        assert!(check.doubts.contains(&Doubt::EmptyRidge), "{name}");
        let mut spaces = check.spaces.clone();
        for s in &mut spaces {
            s.sort_unstable();
        }
        let mut beside = beside;
        beside.sort_unstable();
        assert_eq!(spaces, vec![beside], "{name}");
    }
}

/// A short tooth, its crown sunk 1.5-3.5 mm under its neighbours (partly
/// erupted, intruded), front and back, upper and lower: it stands in the
/// gum with walls, so it is a tooth, never a space, even where it is as
/// flat along the arch as a ridge. (Sunk 3.5 mm, the plugin's seed point
/// of a premolar can fall on the slope of its neighbour: there only the
/// teeth are compared.)
#[test]
fn a_low_tooth_is_a_tooth() {
    for (arch, seed, low) in [
        (Arch::Upper, 3, 12),
        (Arch::Upper, 5, 25),
        (Arch::Upper, 7, 21),
        (Arch::Lower, 3, 42),
        (Arch::Lower, 5, 35),
        (Arch::Lower, 7, 33),
    ] {
        for sink in [1.5, 2.5, 3.5] {
            let name = format!("{arch:?} seed {seed}, {low} {sink} mm low");
            let d = demo::run(&demo::Params {
                arch,
                severity: Severity::Mild,
                seed,
                third_molars: false,
            });
            let labels = d.mesh.vertex_labels.as_ref().unwrap();
            let v: Vec<DVec3> = d
                .mesh
                .positions
                .iter()
                .zip(labels)
                .map(|(p, &l)| p.as_dvec3() - DVec3::Z * if l == low { sink } else { 0.0 })
                .collect();
            let out = checked(arch, &v, &d.mesh.tris, &[]);
            assert_eq!(teeth(&out.seeds), teeth(&d.seeds), "{name}");
            if sink <= 2.5 {
                assert_eq!(off_crown(&d, &v, &out.seeds), Vec::<Fdi>::new(), "{name}");
            }
            let doubts = out
                .diagnostics
                .checked
                .map(|c| c.doubts)
                .unwrap_or_default();
            assert!(!doubts.contains(&Doubt::EmptyRidge), "{name}: {doubts:?}");
        }
    }
}

/// A demo arch (with third molars when `third`) whose teeth `changed` are
/// sunk `sink` mm and scaled `along` x `across` the arch about the middle
/// of their crown (the direction from the seed of the tooth next to them
/// in the arch order to theirs). The true teeth, the mesh.
fn arch_with_changed_teeth(
    arch: Arch,
    seed: u64,
    third: bool,
    changed: &[u8],
    sink: f64,
    (along, across): (f64, f64),
) -> (Vec<Fdi>, Vec<DVec3>, Vec<[u32; 3]>) {
    let d = demo::run(&demo::Params {
        arch,
        severity: Severity::Mild,
        seed,
        third_molars: third,
    });
    let labels = d.mesh.vertex_labels.as_ref().unwrap();
    let mut v: Vec<DVec3> = d.mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let n = d.seeds.len();
    for &f in changed {
        let i = d.seeds.iter().position(|s| s.fdi.get() == f).unwrap();
        let next = if i == 0 { 1 } else { i - 1 };
        let u = (d.seeds[i].position - d.seeds[next.min(n - 1)].position)
            .truncate()
            .normalize();
        let w = DVec2::new(-u.y, u.x);
        let on = |l: &u16| *l == u16::from(f);
        let count = labels.iter().filter(|l| on(l)).count() as f64;
        let c = v
            .iter()
            .zip(labels)
            .filter(|(_, l)| on(l))
            .map(|(p, _)| p.truncate())
            .sum::<DVec2>()
            / count;
        for (p, l) in v.iter_mut().zip(labels) {
            if on(l) {
                let r = p.truncate() - c;
                let xy = c + u * (r.dot(u) * along) + w * (r.dot(w) * across);
                *p = DVec3::new(xy.x, xy.y, p.z - sink);
            }
        }
    }
    (teeth(&d.seeds), v, d.mesh.tris.clone())
}

/// The position of the last tooth of each side (right, left) of `teeth`.
fn last_teeth(teeth: &[Fdi]) -> [u8; 2] {
    let mut out = [0; 2];
    for f in teeth {
        let s = usize::from(!f.is_right());
        out[s] = out[s].max(f.position());
    }
    out
}

/// Every way the checks can leave an end of the arch shorter than the
/// plugin's proposal, with a real last tooth there: a very wide last molar
/// (1.3-1.5 times across the arch), which fails the end test, the ends
/// found again from the crowns leave it out ([`Doubt::EndNotTooth`]); a
/// last molar sunk until 1-2 mm of it stand over the gum, left out by the
/// free ends ([`Doubt::EndNotCrown`]); partly erupted third molars (sunk
/// 1.5-3.5 mm), left out as gum ([`Doubt::ThirdMolar`]). Whichever teeth
/// are kept, an end made shorter always carries a doubt for the ends of the
/// arch: the end left out ([`Doubt::EndNotTooth`] with no end tooth kept,
/// or [`Doubt::EndLeftOut`]), which the app shows as "check the last tooth
/// of each side".
#[test]
fn an_arch_end_shortened_by_the_checks_carries_a_doubt() {
    let wide = |w: f64| (1.0, w);
    // the way, arch, seed, third molars, the teeth changed, sunk (mm), scaled
    type Case = (
        &'static str,
        Arch,
        u64,
        bool,
        &'static [u8],
        f64,
        (f64, f64),
    );
    let cases: [Case; 7] = [
        ("wide", Arch::Upper, 3, false, &[17], 0.0, wide(1.3)),
        ("wide", Arch::Upper, 3, false, &[27], 0.0, wide(1.5)),
        ("wide", Arch::Lower, 5, false, &[37], 0.0, wide(1.4)),
        ("low", Arch::Lower, 3, false, &[37], 4.5, wide(1.0)),
        ("low", Arch::Upper, 5, false, &[17], 4.5, wide(1.0)),
        ("third", Arch::Lower, 3, true, &[38, 48], 2.5, wide(1.0)),
        ("third", Arch::Upper, 7, true, &[18, 28], 1.5, wide(1.0)),
    ];
    let mut shortened = std::collections::BTreeMap::<&str, usize>::new();
    for (path, arch, seed, third, changed, sink, scale) in cases {
        let name = format!("{path}: {arch:?} seed {seed} {changed:?} sunk {sink} scaled {scale:?}");
        let (truth, v, t) = arch_with_changed_teeth(arch, seed, third, changed, sink, scale);
        let plugin = autoseed::propose(&v, &t, arch, &Options::default(), None).unwrap();
        let out = checked(arch, &v, &t, &[]);
        let (before, after) = (
            last_teeth(&teeth(&plugin.seeds)),
            last_teeth(&teeth(&out.seeds)),
        );
        let short = before.iter().zip(after).any(|(&b, a)| a < b);
        let check = out.diagnostics.checked.as_ref();
        let doubts = check.map(|c| c.doubts.clone()).unwrap_or_default();
        let not_crowns = check.map(|c| c.not_crowns.clone()).unwrap_or_default();
        let lost: Vec<Fdi> = truth
            .iter()
            .filter(|f| !out.seeds.iter().any(|s| s.fdi == **f))
            .copied()
            .collect();
        eprintln!(
            "{name}: plugin ends {before:?}, checked {after:?}, real teeth lost {lost:?}, \
             doubts {doubts:?}, not crowns {not_crowns:?}"
        );
        if short {
            *shortened.entry(path).or_default() += 1;
            let told = doubts.contains(&Doubt::EndLeftOut)
                || doubts.contains(&Doubt::EndNotTooth) && not_crowns.is_empty();
            assert!(told, "{name}: an end made shorter with no doubt on it");
        }
    }
    // every way is taken at least once
    for path in ["wide", "low", "third"] {
        assert!(
            shortened.get(path).is_some_and(|&n| n > 0),
            "{path}: {shortened:?}"
        );
    }
}
