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
