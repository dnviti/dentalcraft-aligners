//! The checked automatic seeds (`autoseed::run`, `propose_checked`) on demo
//! arches whose scan runs past the last molars: a smooth bulge of gum behind
//! each last molar (the tuberosity, the retromolar pad or the back wall of a
//! plaster model), up to about a millimetre below the crown tops, as on
//! real scans of models. The plugin's proposal (`autoseed::propose`) must
//! cover the whole profiled arch and takes the bulges as teeth: a third
//! molar, or stretched teeth with every number after them shifted. The
//! checked proposal leaves the bulges out and finds the true teeth.

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
    let labels = d.mesh.vertex_labels.as_ref().unwrap();
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

fn checked(arch: Arch, v: &[DVec3], t: &[[u32; 3]], missing: &[u8]) -> autoseed::Output {
    autoseed::run(
        &Input {
            arch,
            vertices: v,
            triangles: t,
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
