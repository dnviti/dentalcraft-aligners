//! The plan under noise of the segmentation.
//!
//! On a real two-arch case (2026-11-23) a new segmentation relabelled 9
//! vertices of a lower arch: four of the last molar given to its neighbour
//! at their contact, four of it and one of the neighbour given to the
//! gingiva. The plugin's mirror-symmetry search turned the axis of the molar
//! by 6.7 degrees, and the lower plan went from 8 aligners with IPR 0.35 mm
//! per contact to 15 aligners with 0.20 mm and a collision left (21db055).
//!
//! Here a golden demo arch (`demo_upper_mild`, synthetic, `golden/fixtures`)
//! is planned with every tooth to align, so that the rotation of the last
//! molar 17 against the arch line decides by itself whether the molar moves:
//! from the port's own teeth to the staging, as the commands chain them (the
//! staging collides the labelled crown points, as the command does when no
//! closed crowns exist). Then it is planned again under two seeded families of
//! noise at 17 and its neighbour 16, 32 plans each:
//!
//! * the real case's 9 flips: four vertices of 17 at the contact given to
//!   16, four of 17 and one of 16 at the gingiva given to the gingiva;
//! * 5 to 10 vertices anywhere on the borders of 17 and 16 given the label
//!   across the border.
//!
//! A noisy plan jumps, on the criterion the user chose (2026-11-23), when
//! the moving teeth differ, the IPR of a contact differs by more than one
//! 0.05 mm step, the number of aligners differs by more than one, or a
//! collision is left between a pair that had none. With the plugin's mirror
//! search (before 21db055) 46 of the 64 noisy plans jumped on the strict
//! criterion (any IPR difference, more collisions), 17 among the moving
//! teeth in 40 of them; 18 with the new search, 16 once the collision check
//! uses every crown point, 15 once a tooth is derotated only by the excess
//! over its tolerance (eddf352), [`JUMPS_NOW`] once a delay is kept only
//! when it lowers the worst overlap of its pair (fa0cd85). 11 of those are
//! the molar 17 in or out of the moving teeth: its two cusps are single
//! highest points, which swap under the noise, so its measured rotation
//! crosses the 5 degree tolerance (and the 0.5 degree smallest turn beyond
//! it). With every tooth free along the arch (2026-11-24) 11 jumped with a
//! 0.5 mm distal slack at the ends of the arch (55a87aa), 20 without it.
//! The goal is none even on the strict criterion, and
//! [`a_last_molar_moved_by_noise_never_jumps_the_plan`] holds it.
//!
//! The six demo arches are planned the same way with the default settings,
//! then again under the 9 flips at each of their two last molars and under
//! 5 to 10 vertices anywhere on a border given the label across it, 16
//! seeds each: 384 noisy plans. With the incisors and canines aligned (the
//! default until 2026-11-24), on the user's criterion 53 of them jumped
//! before 41ccdab, 24 after it, 28 after eddf352, 27 after fa0cd85 and 21
//! once the robust arch line takes its scale once from the canines and
//! premolars (5bdaa21).
//!
//! Since 2026-11-24 every tooth is aligned by default, the spaces of the
//! arch close with the posterior teeth, the last tooth of each side never
//! goes distally and keeps its height, and with the midline kept each side
//! of the arch makes its own room against the midline:
//! [`FAMILY_JUMPS_NOW`] of the 384 jump. The user accepted the measured
//! count on 2026-11-24 for the release (99 with a 0.5 mm distal slack at
//! the ends, which then moved the last molars distally on every case and
//! was removed; 132 before the two sides made their own room), as a known
//! debt to fix first after it; the goal stays about 5 % (21 of 384). The
//! real case under 20 noisy plans (10 seeds of the same two families, both
//! arches) jumped in none with the slack and in none with one chain for the
//! whole arch, in 3 (all upper) with each side making its own room: the
//! contact of the midline reduced once both sides are crowded (18 to 28
//! aligners) and a last molar at the 0.1 mm threshold of a move. What each
//! rule gives (the counts are deterministic; fixture of 64 / demo arches of
//! 384):
//!
//! | rules | fixture | demo arches |
//! |---|---|---|
//! | the plugin's, incisors and canines, molars held (before) | - | 21 |
//! | the new ones, incisors and canines, molars held | - | 62 |
//! | ... without the no-IPR step at a contact open at rest | - | 33 |
//! | ... without the contacts closed on the crowns | - | 50 |
//! | the plugin's, every tooth (the new default) | 26 | 117 |
//! | babed69: the new ones, every tooth | 35 | 143 |
//! | 55a87aa: the last tooth 0.5 mm distally at most, at its height | 11 | 99 |
//! | the last tooth never distally, at its height | 20 | 125 |
//! | ... and the central incisors held (midline kept), 999f357; its variants: | 20 | 132 |
//! | ... and a crowding of 0.05 mm per contact left to the contacts | 25 | 181 |
//! | ... the ends strict (no distal slack) | 20 | 125 |
//! | ... the last tooth levelled | 10 | 126 |
//! | ... no contact closed on the crowns | 19 | 127 |
//! | ... IPR on a contact open at rest | 11 | 130 |
//! | ... the contacts closed within 0.10 mm (0.05 mm) | 25 (37) | 141 (197) |
//! | ... the IPR cap falling with the gap at rest | 11 | 136 |
//! | ... 8 rounds of the 3D check of the contacts | 7 | 110 |
//! | 999f357 and each side making its own room against the midline: this state | 22 | 160 |
//!
//! Of the 99 of 55a87aa: 81 change the number of aligners by more than one, 26 leave a
//! new collision, 9 change an IPR by more than one step, 1 changes the
//! moving teeth (a plan can jump in several ways). In 97 of them the final
//! position itself moves (in the median a crown point by 0.41 mm, a tooth
//! by 2.6 degrees): the setup jumps, not the staging. The causes, the known
//! debt:
//!
//! * the space the arch misses comes from extremes of the crowns (their
//!   extent along the line, the contact calibrated on its smallest cell,
//!   the 3D overlaps), which a few vertices move by 0.1 to 0.4 mm;
//! * the 3D check of the contacts stops after 4 rounds before it converges
//!   (0.17 mm left in one plan of ten), and its corrections feed that
//!   space, so the IPR of the whole stretch;
//! * the IPR is rounded up per contact, so one crossing of a step moves
//!   every contact of the stretch.

mod golden_case;

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use dca_algo::anatomy::Arch;
use dca_algo::frame;
use dca_algo::setup::{self, ToothInput};
use dca_algo::staging::{self, Complexity, IprPlan, ToothMove};
use dca_algo::teeth::{self, Which};
use dca_model::{SetupScope, SetupSettings};
use glam::DVec3;
use golden_case::Case;

/// Noisy plans of the fixture that still jump (of 64; on the strict
/// criterion 46 with the plugin's mirror search, 18 with its sample of the
/// crowns in the collision check, 16 with the plugin's derotation, 15 with
/// the plugin's delays).
const JUMPS_NOW: usize = 14;
/// Seeds of each family of noise.
const SEEDS: u64 = 32;
/// Noisy plans of the six demo arches that still jump (of 384), with every
/// tooth aligned: the measured count the user accepted on 2026-11-24 as a
/// known debt (see the module documentation); the goal is about 5 % (21).
const FAMILY_JUMPS_NOW: usize = 121;
/// Seeds of each family of noise at each last molar of the six demo arches.
const FAMILY_SEEDS: u64 = 16;

const CASE: &str = "demo_upper_mild";

fn fdi(n: u8) -> Fdi {
    Fdi::new(n).unwrap()
}

struct Plan {
    moved: BTreeSet<Fdi>,
    ipr: BTreeMap<(Fdi, Fdi), f64>,
    aligners: u32,
    unresolved: BTreeSet<(Fdi, Fdi)>,
}

/// Teeth, final position and aligners of the arch with `labels` and the
/// setup `settings`.
fn plan(case: &Case, labels: &[u16], settings: &SetupSettings) -> Plan {
    let arch = if case.teeth["arch"] == "UPPER" {
        Arch::Upper
    } else {
        Arch::Lower
    };
    let geo = teeth::run(
        &teeth::Input {
            arch,
            vertices: &case.vertices,
            tris: &case.tris,
            labels,
        },
        &teeth::Params {
            arch: if arch.is_upper() {
                Which::Upper
            } else {
                Which::Lower
            },
            ..teeth::Params::default()
        },
    )
    .unwrap();
    let inputs: BTreeMap<Fdi, ToothInput> = geo
        .teeth
        .iter()
        .map(|(f, t)| (*f, ToothInput::from(t)))
        .collect();
    let out = setup::run(
        &setup::Input {
            arch,
            vertices: &case.vertices,
            labels,
            sequence: &geo.sequence,
            teeth: &inputs,
            line: None,
            fixed: &[],
            poses: &BTreeMap::new(),
        },
        &setup::Params::from_settings(settings),
    )
    .unwrap();
    let r = &out.result;
    let moves: Vec<ToothMove> = geo
        .sequence
        .iter()
        .map(|&f| {
            let g = &geo.teeth[&f];
            let rest = teeth::rest_pose(&teeth::tooth_frame(g)).to_mat4();
            let inv = frame::rigid_inverse(&rest);
            let lab = u16::from(f.get());
            let pts: Vec<DVec3> = case
                .vertices
                .iter()
                .zip(labels)
                .filter(|(_, l)| **l == lab)
                .map(|(p, _)| inv.transform_point3(*p))
                .collect();
            let q = r
                .targets
                .get(&f)
                .map_or([0.0; 6], |d| staging::q_from_delta(&rest, d));
            ToothMove::new(f, rest, q, pts, g.md_sign)
        })
        .collect();
    let ipr: Vec<IprPlan> = r
        .ipr
        .iter()
        .map(|(&(a, b), &mm)| IprPlan { a, b, mm })
        .collect();
    let staged = staging::run(
        &staging::Input {
            moves: &moves,
            ipr: &ipr,
            complexity: Complexity::default(),
        },
        &staging::Params::default(),
    )
    .unwrap();
    Plan {
        moved: r.moved.iter().copied().collect(),
        ipr: r.ipr.clone(),
        aligners: staged.n_stages,
        unresolved: staged.unresolved.iter().map(|u| (u.a, u.b)).collect(),
    }
}

/// xorshift64*: the same noise on every machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n.max(1)
    }
}

/// The vertices on a border of their label: `(vertex, label across)`.
fn borders(case: &Case, labels: &[u16]) -> BTreeSet<(u32, u16)> {
    let mut out = BTreeSet::new();
    for t in &case.tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let (la, lb) = (labels[a as usize], labels[b as usize]);
            if la != lb {
                out.insert((a, lb));
                out.insert((b, la));
            }
        }
    }
    out
}

/// The `k` vertices of `pool` nearest to `from`.
fn nearest(case: &Case, from: u32, pool: &[u32], k: usize) -> Vec<u32> {
    let p = case.vertices[from as usize];
    let mut d: Vec<(f64, u32)> = pool
        .iter()
        .map(|&i| (case.vertices[i as usize].distance_squared(p), i))
        .collect();
    d.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    d.into_iter().take(k).map(|(_, i)| i).collect()
}

/// The real case's 9 flips at the molar `m` and its neighbour `nb`, placed
/// by `seed`: the 4 vertices of `m` nearest to one drawn on its contact go
/// to `nb`, the 4 of `m` nearest to one drawn on its border with the gingiva
/// and one of `nb` on its border go to the gingiva.
fn nine(case: &Case, labels: &[u16], m: Fdi, nb: Fdi, seed: u64) -> Vec<u16> {
    let (lm, ln) = (u16::from(m.get()), u16::from(nb.get()));
    let b = borders(case, labels);
    let pool = |own: u16, other: u16| -> Vec<u32> {
        b.iter()
            .filter(|(i, o)| labels[*i as usize] == own && *o == other)
            .map(|(i, _)| *i)
            .collect()
    };
    let mut rng = Rng::new(seed);
    let mut out = labels.to_vec();
    let contact = pool(lm, ln);
    let v0 = contact[rng.below(contact.len())];
    for i in nearest(case, v0, &contact, 4) {
        out[i as usize] = ln;
    }
    let gum = pool(lm, 0);
    let g0 = gum[rng.below(gum.len())];
    for i in nearest(case, g0, &gum, 4) {
        out[i as usize] = 0;
    }
    let gn = pool(ln, 0);
    out[gn[rng.below(gn.len())] as usize] = 0;
    out
}

/// 5 to 10 vertices on the borders of `m` and `nb` (theirs, and the gingiva
/// bordering them), drawn by `seed`, given the label across the border.
fn local(case: &Case, labels: &[u16], m: Fdi, nb: Fdi, seed: u64) -> Vec<u16> {
    let (lm, ln) = (u16::from(m.get()), u16::from(nb.get()));
    let b: Vec<(u32, u16)> = borders(case, labels)
        .into_iter()
        .filter(|&(i, o)| {
            let l = labels[i as usize];
            l == lm || l == ln || (l == 0 && (o == lm || o == ln))
        })
        .collect();
    let mut rng = Rng::new(seed ^ 0xa5a5);
    let n = 5 + rng.below(6);
    let mut out = labels.to_vec();
    let mut done = BTreeSet::new();
    while done.len() < n {
        let (i, o) = b[rng.below(b.len())];
        if done.insert(i) {
            out[i as usize] = o;
        }
    }
    out
}

/// 5 to 10 vertices anywhere on a border of their label, drawn by `seed`,
/// given the label across the border.
fn flips(case: &Case, labels: &[u16], seed: u64) -> Vec<u16> {
    let b: Vec<(u32, u16)> = borders(case, labels).into_iter().collect();
    let mut rng = Rng::new(seed ^ 0x5151);
    let n = 5 + rng.below(6);
    let mut out = labels.to_vec();
    let mut done = BTreeSet::new();
    while done.len() < n {
        let (i, o) = b[rng.below(b.len())];
        if done.insert(i) {
            out[i as usize] = o;
        }
    }
    out
}

/// How the noisy plan `b` jumps from the plan `a` (empty: it holds). The
/// criterion the user chose (2026-11-23): the same moving teeth, the IPR
/// of every contact within one 0.05 mm step, the aligners within one, and
/// no collision left between a pair that had none; `strict`: the IPR of
/// every contact the same and no more collisions left.
fn jumps(a: &Plan, b: &Plan, strict: bool) -> Vec<String> {
    let mut out = Vec::new();
    if a.moved != b.moved {
        let list = |s: BTreeSet<&Fdi>| {
            s.iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let gone = list(a.moved.difference(&b.moved).collect());
        let new = list(b.moved.difference(&a.moved).collect());
        out.push(format!("moving teeth -[{gone}] +[{new}]"));
    }
    let step = if strict { 1e-9 } else { 0.05 + 1e-9 };
    let contacts: BTreeSet<&(Fdi, Fdi)> = a.ipr.keys().chain(b.ipr.keys()).collect();
    for c in contacts {
        let (x, y) = (
            a.ipr.get(c).copied().unwrap_or(0.0),
            b.ipr.get(c).copied().unwrap_or(0.0),
        );
        if (x - y).abs() > step {
            out.push(format!("IPR {}|{} {x:.2} -> {y:.2} mm", c.0, c.1));
        }
    }
    if a.aligners.abs_diff(b.aligners) > 1 {
        out.push(format!("aligners {} -> {}", a.aligners, b.aligners));
    }
    let more = if strict {
        b.unresolved.len() > a.unresolved.len()
    } else {
        !b.unresolved.is_subset(&a.unresolved)
    };
    if more {
        out.push(format!(
            "collisions left {:?} -> {:?}",
            a.unresolved, b.unresolved
        ));
    }
    out
}

/// The jumps of the 64 noisy plans of the fixture, one line each.
fn noisy_jumps(strict: bool) -> Vec<String> {
    let case = Case::load(CASE);
    let (m, nb) = (fdi(17), fdi(16));
    let settings = SetupSettings {
        align_scope: SetupScope::All,
        ..SetupSettings::default()
    };
    let base = plan(&case, &case.labels, &settings);
    let mut out = Vec::new();
    for seed in 1..=SEEDS {
        for (family, labels) in [
            ("9 flips", nine(&case, &case.labels, m, nb, seed)),
            ("border flips", local(&case, &case.labels, m, nb, seed)),
        ] {
            let flipped = labels
                .iter()
                .zip(&case.labels)
                .filter(|(a, b)| a != b)
                .count();
            assert!((5..=10).contains(&flipped), "{family} {seed}: {flipped}");
            let j = jumps(&base, &plan(&case, &labels, &settings), strict);
            if !j.is_empty() {
                out.push(format!(
                    "{family}, seed {seed} ({flipped} vertices): {}",
                    j.join("; ")
                ));
            }
        }
    }
    out
}

/// The jumps of the 384 noisy plans of the six demo arches, one line each:
/// each arch with the default settings, then under the 9 flips at each of
/// its two last molars and 5 to 10 border flips anywhere (one arch a
/// thread).
fn family_jumps(strict: bool) -> Vec<String> {
    std::thread::scope(|s| {
        let arches: Vec<_> = golden_case::CASES
            .iter()
            .map(|&name| {
                s.spawn(move || {
                    let case = Case::load(name);
                    let settings = SetupSettings::default();
                    let base = plan(&case, &case.labels, &settings);
                    let seq = case.fdis();
                    let ends = [(seq[0], seq[1]), (seq[seq.len() - 1], seq[seq.len() - 2])];
                    let mut out = Vec::new();
                    for (m, nb) in ends {
                        for seed in 1..=FAMILY_SEEDS {
                            let border = seed * 7919 + u64::from(m.get());
                            for (family, labels) in [
                                ("9 flips", nine(&case, &case.labels, m, nb, seed)),
                                ("border flips", flips(&case, &case.labels, border)),
                            ] {
                                let j = jumps(&base, &plan(&case, &labels, &settings), strict);
                                if !j.is_empty() {
                                    out.push(format!(
                                        "{name}, {family} at {m}, seed {seed}: {}",
                                        j.join("; ")
                                    ));
                                }
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        arches.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn a_last_molar_moved_by_noise_jumps_the_plan_less_often() {
    let j = noisy_jumps(false);
    eprintln!(
        "{} of {} noisy plans jump:\n{}",
        j.len(),
        2 * SEEDS,
        j.join("\n")
    );
    assert!(
        j.len() <= JUMPS_NOW,
        "{} of {} noisy plans jump, at most {JUMPS_NOW} expected:\n{}",
        j.len(),
        2 * SEEDS,
        j.join("\n")
    );
}

#[test]
#[ignore = "the goal: no noisy plan jumps even on the strict criterion (every IPR the same)"]
fn a_last_molar_moved_by_noise_never_jumps_the_plan() {
    let j = noisy_jumps(true);
    assert!(j.is_empty(), "noisy plans that jump:\n{}", j.join("\n"));
}

#[test]
#[cfg_attr(
    not(optimized),
    ignore = "slow without optimisations: cargo test --release"
)]
fn the_demo_arches_under_noise_jump_the_plan_less_often() {
    let j = family_jumps(false);
    let total = golden_case::CASES.len() as u64 * 2 * 2 * FAMILY_SEEDS;
    eprintln!("{} of {total} noisy plans jump:\n{}", j.len(), j.join("\n"));
    assert!(
        j.len() <= FAMILY_JUMPS_NOW,
        "{} of {total} noisy plans jump, at most {FAMILY_JUMPS_NOW} expected:\n{}",
        j.len(),
        j.join("\n")
    );
}
