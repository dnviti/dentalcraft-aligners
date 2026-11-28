//! Plausibility of the teeth of a segmentation and the correction of the
//! automatic seeds (`segmentation.check_teeth`, `plan_corrections`,
//! `apply_corrections`, `renumber_spans`, `report_issues` of the plugin,
//! driven by `case_auto.check_segmentation_iter`).
//!
//! A wrong seed proposal (two seeds on one crown, a seed in a gap or on the
//! gingiva) gives teeth that the boundary refinement cannot separate on a
//! fold: the boundary between two parts of one crown runs over the crown
//! from the buccal to the lingual gingiva with almost no fold. The check
//! measures, along the arch curve of [`crate::autoseed::arch_data`], the
//! span of the top band of every tooth, the boundaries between teeth with
//! no interproximal fold ([`Report::weak`]), the fragments narrower than a
//! real crown ([`Report::slivers`]) and the crowns of two teeth
//! ([`Report::wide`]). [`plan_corrections`] turns them into seed changes
//! (merge two parts of one crown, drop a fragment or a seed in a gap, split
//! a crown of two teeth), [`apply_corrections`] moves the seeds, and after a
//! correction [`renumber_spans`] numbers the teeth again from their widths.
//!
//! Beyond the plugin, adjacent separate teeth are never merged: a weak
//! boundary alone is no reason to merge. On a printed model the contacts
//! between the teeth are fused and shallow: the plugin read them as weak
//! and merged real neighbours (11+12, 14+15, 15+16 on the test piece), each
//! merge renumbering the whole arch. Only a weak boundary with positive
//! evidence of one crown ([`one_crown`]: no groove at all, and the two parts
//! together as wide as one tooth of that kind, each narrower than one) is
//! planned ([`vetted`]); the others keep both teeth and stay a doubt for
//! the user. A correction renumbers only the quadrants it touched
//! ([`renumber_touched`]).
//!
//! Vertices are in scan (case) coordinates; the arch frame of the
//! [`ArchData`] moves them to arch coordinates. Labels are FDI numbers, `0`
//! for the gingiva. Messages are enums here; the module words them.

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use glam::{DVec2, DVec3};
use serde::Serialize;

use crate::anatomy::{self, Arch};
use crate::autoseed::numpy::{median, percentile};
use crate::autoseed::{self, ArchData, MIDLINE_SIGMA, MISSING_COST};
use crate::frame;

/// Median fold below which a tooth-tooth boundary is no interproximal fold.
pub const WEAK_FOLD: f64 = 2.0;
/// ... when longer than this (mm): gum to gum over a crown.
pub const WEAK_LEN_MM: f64 = 12.0;
/// Narrowest real crown along the arch (mm).
pub const SLIVER_MM: f64 = 3.5;
/// Crown width along the arch measured on its top band this deep (mm).
pub const CROWN_BAND_MM: f64 = 3.0;
/// Two parts of one crown measure at most this many widest molars.
pub const ONE_CROWN: f64 = 1.45;
/// A weak boundary runs over one crown, not in an interproximal groove,
/// only when its median fold is below this ([`one_crown`]): the boundaries
/// of crowns split in two by their labels measured 0.04-1.06 on the
/// printed test piece and on a patient scan (14 crowns), the fused contacts
/// of the test piece 1.45-1.96, real contacts 4.6 and more.
pub const OVER_CROWN_FOLD: f64 = 1.25;
/// Two parts of one crown measure together at most this many typical
/// widths of the kept tooth, scaled to the arch ([`one_crown`]): crowns
/// split in two measured 0.95-1.38 (1.53 where a part ran onto the next
/// tooth), real neighbours 1.62-2.24 on the test piece (with the typical
/// widths of [`arch_widths`], any two neighbours measure 1.78 or more).
pub const ONE_TOOTH: f64 = 1.5;
/// Each part of one crown is narrower than this many typical widths of the
/// kept tooth, scaled to the arch ([`one_crown`]): a part as wide as a whole
/// tooth is a tooth.
pub const PART_OF_TOOTH: f64 = 0.9;
/// The scale of an arch ([`arch_scale`]) stays in the range of the scales
/// of the automatic seeds.
const SCALE_RANGE: (f64, f64) = (0.82, 1.18);
/// A crown wider than this many widest molars is two teeth.
pub const SPLIT_CROWN: f64 = 1.6;
/// Midline halfway between the canines (mm).
pub const CANINE_MID_SIGMA: f64 = 1.5;
/// Cost within which another numbering of other teeth makes it uncertain.
pub const NUMBER_MARGIN: f64 = 3.0;
/// Rounds of seed correction and new segmentation after the first one
/// (`case_auto.SEG_ROUNDS`).
pub const SEG_ROUNDS: usize = 3;
/// Fold measure of the boundaries (`REFINE_SIGMA`, `REFINE_NORMAL_SCALE`).
const REFINE_SIGMA: f64 = 0.25;
const REFINE_NORMAL_SCALE: f64 = 0.3;
/// Arc-length sampling of the curve projection (`autoseed._project_s`).
const PROJECT_STEP: f64 = 0.05;

/// Crown widths along the arch per tooth position 1..=8 (`ARCH_WIDTHS`).
pub fn arch_widths(arch: Arch) -> [f64; 8] {
    if arch.is_upper() {
        [8.52, 6.69, 6.81, 6.66, 6.77, 10.78, 9.75, 9.21]
    } else {
        [5.35, 5.98, 6.33, 6.65, 7.02, 11.11, 10.83, 10.31]
    }
}

/// Span of a tooth along the arch (mm of arc length).
pub type Span = (f64, f64);

/// A boundary between two teeth with no interproximal fold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Weak {
    /// The two teeth (smaller label first).
    pub a: u16,
    /// The other tooth.
    pub b: u16,
    /// Length of the boundary (mm, rounded to 0.1).
    pub len_mm: f64,
    /// Median fold (rounded to 0.01).
    pub fold: f64,
    /// 10th and 90th percentile of the boundary along the arch.
    pub s: (f64, f64),
}

/// Result of [`check_teeth`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Span of every tooth along the arch.
    pub spans: BTreeMap<u16, Span>,
    /// Boundaries with no interproximal fold.
    pub weak: Vec<Weak>,
    /// Widest molar of [`arch_widths`].
    pub molar: f64,
    /// Teeth narrower than [`SLIVER_MM`].
    pub slivers: Vec<u16>,
    /// Teeth wider than [`SPLIT_CROWN`] molars.
    pub wide: Vec<u16>,
}

impl Report {
    /// Something is still implausible.
    pub fn doubtful(&self) -> bool {
        !(self.weak.is_empty() && self.slivers.is_empty() && self.wide.is_empty())
    }

    fn width(&self, f: u16) -> f64 {
        self.spans.get(&f).map_or(0.0, |(lo, hi)| hi - lo)
    }
}

/// A correction of the seeds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// The seed of a fragment (or of a gap) is removed.
    Drop {
        /// The tooth.
        fdi: u16,
    },
    /// Two parts of one crown: one seed for both.
    Merge {
        /// The number kept.
        keep: u16,
        /// The number removed.
        drop: u16,
    },
    /// A crown of two teeth: one seed on each side of `cut`.
    Split {
        /// The tooth.
        fdi: u16,
        /// Arc length of the cut.
        cut: f64,
    },
}

/// An implausible tooth (`segmentation.report_issues`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Issue {
    /// No interproximal fold between `a` and `b`.
    NoFold {
        /// A tooth.
        a: u16,
        /// The other tooth.
        b: u16,
        /// Length of the boundary over the crown (mm).
        len_mm: f64,
    },
    /// A tooth narrower than a crown.
    Narrow {
        /// The tooth.
        fdi: u16,
        /// Its width along the arch (mm).
        width_mm: f64,
    },
    /// A tooth wider than one crown.
    Wide {
        /// The tooth.
        fdi: u16,
        /// Its width along the arch (mm).
        width_mm: f64,
    },
}

/// A note on the numbering from the widths (`renumber_spans`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NumberNote {
    /// One premolar only in the quadrant: first and second cannot be told
    /// apart from the widths.
    OnePremolar {
        /// The quadrant.
        quadrant: u8,
        /// The number given.
        fdi: u16,
    },
    /// Another numbering of other teeth is almost as likely.
    Uncertain {
        /// The pairs that differ (this number or that one), at most six.
        pairs: Vec<(u16, u16)>,
    },
}

/// Label of every face: the label shared by at least two of its vertices
/// (else the second one, as numpy's `where`).
pub fn face_labels(t: &[[u32; 3]], labels: &[u16]) -> Vec<u16> {
    t.iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| labels[i as usize]);
            if a == b || a == c { a } else { b }
        })
        .collect()
}

/// Arc-length interval (2nd-98th percentile) of the top band of every tooth
/// along the arch curve (`crown_spans`; vertices in arch coordinates).
pub fn crown_spans(
    va: &[DVec3],
    labels: &[u16],
    curve: &crate::archform::ArchCurve,
    band: f64,
) -> BTreeMap<u16, Span> {
    let teeth: BTreeSet<u16> = labels.iter().copied().filter(|&l| l != 0).collect();
    let mut out = BTreeMap::new();
    for f in teeth {
        let p: Vec<DVec3> = va
            .iter()
            .zip(labels)
            .filter(|(_, l)| **l == f)
            .map(|(p, _)| *p)
            .collect();
        let zmax = p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
        let q: Vec<DVec3> = p.into_iter().filter(|q| q.z > zmax - band).collect();
        let order = autoseed::lexsort_xyz(&q);
        let step = (q.len() / 3000).max(1);
        let xy: Vec<DVec2> = order
            .iter()
            .step_by(step)
            .map(|&i| q[i].truncate())
            .collect();
        if xy.is_empty() {
            continue;
        }
        let s = autoseed::project_s(curve, &xy, PROJECT_STEP);
        out.insert(f, (percentile(&s, 2.0), percentile(&s, 98.0)));
    }
    out
}

/// A boundary between two neighbouring teeth (`tooth_boundaries`).
#[derive(Debug, Clone, PartialEq)]
pub struct Boundary {
    /// Total length (mm).
    pub len: f64,
    /// Median fold of its edges.
    pub fold: f64,
    /// Midpoints of its edges.
    pub points: Vec<DVec3>,
}

/// Boundaries between neighbouring teeth of a segmentation (vertex labels,
/// gingiva excluded), keyed by the two labels, smaller first.
pub fn tooth_boundaries(
    v: &[DVec3],
    t: &[[u32; 3]],
    labels: &[u16],
) -> BTreeMap<(u16, u16), Boundary> {
    let fl = face_labels(t, labels);
    let g = crate::segmentation::face_graph(v, t, 0.2, REFINE_SIGMA, REFINE_NORMAL_SCALE);
    // edge lengths, folds and midpoints of each boundary
    type Edges = (Vec<f64>, Vec<f64>, Vec<DVec3>);
    let mut acc: BTreeMap<(u16, u16), Edges> = BTreeMap::new();
    for (i, nb) in g.nbr.iter().enumerate() {
        for k in 0..3 {
            let n = nb[k];
            if n == u32::MAX || n as usize >= t.len() {
                continue;
            }
            let n = n as usize;
            let (a, b) = (fl[i], fl[n]);
            if a == b || a == 0 || b == 0 || i >= n {
                continue;
            }
            let p0 = v[t[i][k] as usize];
            let p1 = v[t[i][(k + 1) % 3] as usize];
            let e = acc.entry((a.min(b), a.max(b))).or_default();
            e.0.push((p1 - p0).length());
            e.1.push(g.d[i][k]);
            e.2.push(0.5 * (p0 + p1));
        }
    }
    acc.into_iter()
        .map(|(k, (len, d, points))| {
            (
                k,
                Boundary {
                    len: len.iter().sum(),
                    fold: median(&d),
                    points,
                },
            )
        })
        .collect()
}

fn round_to(x: f64, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    (x * m).round() / m
}

/// Vertices of a scan in the arch coordinates of `data`.
fn to_arch(v_case: &[DVec3], data: &ArchData) -> Vec<DVec3> {
    let inv = frame::rigid_inverse(&data.frame);
    v_case.iter().map(|p| inv.transform_point3(*p)).collect()
}

/// Plausibility of the teeth of a segmentation (`check_teeth`).
pub fn check_teeth(
    v_case: &[DVec3],
    t: &[[u32; 3]],
    labels: &[u16],
    arch: Arch,
    data: &ArchData,
) -> Report {
    let va = to_arch(v_case, data);
    let spans = crown_spans(&va, labels, &data.curve, CROWN_BAND_MM);
    let mut weak = Vec::new();
    for ((a, b), bd) in tooth_boundaries(&va, t, labels) {
        if bd.fold < WEAK_FOLD && bd.len > WEAK_LEN_MM {
            let xy: Vec<DVec2> = bd.points.iter().map(|p| p.truncate()).collect();
            let s = autoseed::project_s(&data.curve, &xy, PROJECT_STEP);
            weak.push(Weak {
                a,
                b,
                len_mm: round_to(bd.len, 1),
                fold: round_to(bd.fold, 2),
                s: (percentile(&s, 10.0), percentile(&s, 90.0)),
            });
        }
    }
    let molar = arch_widths(arch).into_iter().fold(0.0, f64::max);
    let slivers = spans
        .iter()
        .filter(|(_, (lo, hi))| hi - lo < SLIVER_MM)
        .map(|(f, _)| *f)
        .collect();
    let wide = spans
        .iter()
        .filter(|(_, (lo, hi))| hi - lo > SPLIT_CROWN * molar)
        .map(|(f, _)| *f)
        .collect();
    Report {
        spans,
        weak,
        molar,
        slivers,
        wide,
    }
}

/// The implausible teeth of a report (`report_issues`), empty when
/// plausible.
pub fn report_issues(rep: &Report) -> Vec<Issue> {
    let mut out: Vec<Issue> = rep
        .weak
        .iter()
        .map(|w| Issue::NoFold {
            a: w.a,
            b: w.b,
            len_mm: w.len_mm,
        })
        .collect();
    out.extend(rep.slivers.iter().map(|&f| Issue::Narrow {
        fdi: f,
        width_mm: rep.width(f),
    }));
    out.extend(rep.wide.iter().map(|&f| Issue::Wide {
        fdi: f,
        width_mm: rep.width(f),
    }));
    out
}

fn index_of(f: u16) -> u8 {
    u8::try_from(f)
        .ok()
        .and_then(|n| Fdi::new(n).ok())
        .map_or(0, anatomy::index)
}

/// Seed corrections for the implausible teeth of a report
/// (`plan_corrections`): first the clear cases (a sliver loses its seed,
/// two parts of one crown are merged, keeping the number of the part nearer
/// the midline); only when none is left, a tooth with weak boundaries on
/// both sides loses its seed and a crown wider than [`SPLIT_CROWN`] molars
/// is split. This is the plugin's plan; the app plans on the report
/// [`vetted`] first, so that only parts of one crown are merged.
pub fn plan_corrections(rep: &Report) -> Vec<Action> {
    let spans = &rep.spans;
    let span = |f: u16| spans.get(&f).copied().unwrap_or((0.0, 0.0));
    let mut acts: Vec<Action> = rep
        .slivers
        .iter()
        .map(|&f| Action::Drop { fdi: f })
        .collect();
    let mut used: BTreeSet<u16> = rep.slivers.iter().copied().collect();
    let mut weak = rep.weak.clone();
    weak.sort_by(|x, y| {
        x.fold
            .total_cmp(&y.fold)
            .then(x.a.cmp(&y.a))
            .then(x.b.cmp(&y.b))
    });
    for w in &weak {
        let (a, b) = (w.a, w.b);
        let (sa, sb) = (span(a), span(b));
        let (lo, hi) = (sa.0.min(sb.0), sa.1.max(sb.1));
        if used.contains(&a) || used.contains(&b) || hi - lo > ONE_CROWN * rep.molar {
            continue;
        }
        let keep = kept(a, b, sa.1 - sa.0, sb.1 - sb.0);
        let drop = if keep == a { b } else { a };
        acts.push(Action::Merge { keep, drop });
        used.insert(a);
        used.insert(b);
    }
    if !acts.is_empty() {
        return acts;
    }
    let mut count: BTreeMap<u16, usize> = BTreeMap::new();
    for w in &rep.weak {
        *count.entry(w.a).or_default() += 1;
        *count.entry(w.b).or_default() += 1;
    }
    let mut acts: Vec<Action> = count
        .iter()
        .filter(|(_, n)| **n >= 2)
        .map(|(f, _)| Action::Drop { fdi: *f })
        .collect();
    acts.extend(
        rep.wide
            .iter()
            .filter(|f| !count.contains_key(f))
            .map(|&f| {
                let (lo, hi) = span(f);
                Action::Split {
                    fdi: f,
                    cut: 0.5 * (lo + hi),
                }
            }),
    );
    acts
}

/// The number two merged parts keep ([`plan_corrections`]): that of the
/// part nearer the midline, of two parts at the same position the wider
/// one's (`wa`, `wb`: their widths along the arch).
fn kept(a: u16, b: u16, wa: f64, wb: f64) -> u16 {
    let (ia, ib) = (index_of(a), index_of(b));
    if ia < ib || (ia == ib && wa >= wb) {
        a
    } else {
        b
    }
}

/// Typical width (mm) along the arch of the tooth `f` ([`arch_widths`]);
/// `None` for a number that is no permanent tooth.
fn typical_width(f: u16, arch: Arch) -> Option<f64> {
    let i = usize::from(index_of(f));
    (1..=8).contains(&i).then(|| arch_widths(arch)[i - 1])
}

/// Scale of the teeth of a report: the median ratio of their widths along
/// the arch to their typical widths ([`arch_widths`]), within the range of
/// the scales of the automatic seeds; 1 without teeth.
pub fn arch_scale(rep: &Report, arch: Arch) -> f64 {
    let r: Vec<f64> = rep
        .spans
        .iter()
        .filter_map(|(f, (lo, hi))| typical_width(*f, arch).map(|w| (hi - lo) / w))
        .collect();
    if r.is_empty() {
        return 1.0;
    }
    median(&r).clamp(SCALE_RANGE.0, SCALE_RANGE.1)
}

/// The two teeth on the sides of the weak boundary `w` are two parts of one
/// crown: the positive evidence a merge needs, beyond the plugin (which
/// merges any two teeth with a weak boundary that measure together at most
/// [`ONE_CROWN`] widest molars, and so real neighbours with the fused
/// contacts of a printed model). The boundary has no groove at all
/// ([`OVER_CROWN_FOLD`]); with the typical width `e` of the tooth whose
/// number the merge keeps, scaled to the arch ([`arch_scale`]), the two
/// parts measure together at most [`ONE_TOOTH`] `e` (and the plugin's
/// limit), and each of them less than [`PART_OF_TOOTH`] `e`.
pub fn one_crown(rep: &Report, w: &Weak, arch: Arch) -> bool {
    let (Some(&sa), Some(&sb)) = (rep.spans.get(&w.a), rep.spans.get(&w.b)) else {
        return false;
    };
    let (wa, wb) = (sa.1 - sa.0, sb.1 - sb.0);
    let Some(e) = typical_width(kept(w.a, w.b, wa, wb), arch) else {
        return false;
    };
    let e = e * arch_scale(rep, arch);
    let both = sa.1.max(sb.1) - sa.0.min(sb.0);
    w.fold < OVER_CROWN_FOLD
        && both <= (ONE_TOOTH * e).min(ONE_CROWN * rep.molar)
        && wa.max(wb) < PART_OF_TOOTH * e
}

/// The report with only the weak boundaries over one crown ([`one_crown`]):
/// what the app plans its corrections on ([`plan_corrections`]). Every
/// boundary left is merged by the plan, so a tooth is never dropped for
/// its weak boundaries either. Two teeth with any other weak boundary stay
/// two teeth; the boundary stays in [`Report::weak`] of the check, a doubt
/// for the user.
pub fn vetted(rep: &Report, arch: Arch) -> Report {
    let mut out = rep.clone();
    out.weak.retain(|w| one_crown(rep, w, arch));
    out
}

/// The numbers of [`renumber_spans`] (`new`: old number -> new number)
/// kept only where a correction changed the seeds: `touched` holds, for
/// each quadrant (1-4) whose seeds changed, the position (1-8) of the most
/// mesial seed that changed. A merge, a drop or a split shifts the numbers
/// of the teeth distal to it in its quadrant, never those of the teeth
/// mesial to it or of another quadrant: they keep their numbers (the plugin
/// renumbers the whole arch from the widths, and one wrong merge moved the
/// numbers of the other side too). `None` when the renumbering would give a
/// tooth a number of a quadrant left alone, or two teeth the same number.
pub fn renumber_touched(
    new: &BTreeMap<u16, u16>,
    touched: &BTreeMap<u16, u8>,
) -> Option<BTreeMap<u16, u16>> {
    let free = |f: u16| touched.get(&(f / 10)).is_some_and(|&m| index_of(f) >= m);
    let mut out = BTreeMap::new();
    for (&f, &g) in new {
        let g = if free(f) { g } else { f };
        if g != f && !touched.contains_key(&(g / 10)) {
            return None;
        }
        out.insert(f, g);
    }
    let numbers: BTreeSet<u16> = out.values().copied().collect();
    (numbers.len() == out.len()).then_some(out)
}

/// Seed of a crown (arch coordinates): the highest point near the centre of
/// its top (`crown_seed`). `None` for an empty selection.
pub fn crown_seed(p: &[DVec3]) -> Option<DVec3> {
    let zmax = p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
    let top: Vec<DVec2> = p
        .iter()
        .filter(|q| q.z >= zmax - 2.5)
        .map(|q| q.truncate())
        .collect();
    if top.is_empty() {
        return None;
    }
    let cxy = top.iter().fold(DVec2::ZERO, |a, b| a + *b) / top.len() as f64;
    let mut best: Option<(f64, DVec3)> = None;
    for q in p {
        let c = (q.truncate() - cxy).length() - 0.2 * (q.z - zmax);
        if best.is_none_or(|(b, _)| c < b) {
            best = Some((c, *q));
        }
    }
    best.map(|(_, q)| q)
}

/// The seeds after [`plan_corrections`] (`apply_corrections`), in case
/// coordinates, sorted by tooth number. A merged crown gets one seed on its
/// two parts, a split crown one seed on each side of the cut (the second
/// with the free number of the arch nearest to it: the teeth are numbered
/// again afterwards, [`renumber_spans`]).
pub fn apply_corrections(
    v_case: &[DVec3],
    labels: &[u16],
    seeds: &[(Fdi, DVec3)],
    acts: &[Action],
    data: &ArchData,
) -> Vec<(Fdi, DVec3)> {
    let va = to_arch(v_case, data);
    let m = data.frame;
    let fdi = |f: u16| u8::try_from(f).ok().and_then(|n| Fdi::new(n).ok());
    let mut out: BTreeMap<Fdi, DVec3> = seeds.iter().copied().collect();
    let pick = |sel: &dyn Fn(usize) -> bool| -> Option<DVec3> {
        let p: Vec<DVec3> = (0..va.len()).filter(|&i| sel(i)).map(|i| va[i]).collect();
        crown_seed(&p).map(|q| m.transform_point3(q))
    };
    for act in acts {
        match *act {
            Action::Drop { fdi: f } => {
                if let Some(f) = fdi(f) {
                    out.remove(&f);
                }
            }
            Action::Merge { keep, drop } => {
                let (Some(k), Some(d)) = (fdi(keep), fdi(drop)) else {
                    continue;
                };
                out.remove(&d);
                if let Some(p) = pick(&|i| labels[i] == keep || labels[i] == drop) {
                    out.insert(k, p);
                }
            }
            Action::Split { fdi: f, cut } => {
                let Some(fd) = fdi(f) else { continue };
                let idx: Vec<usize> = (0..va.len()).filter(|&i| labels[i] == f).collect();
                let xy: Vec<DVec2> = idx.iter().map(|&i| va[i].truncate()).collect();
                let s = autoseed::project_s(&data.curve, &xy, PROJECT_STEP);
                let sv: BTreeMap<usize, f64> = idx.iter().copied().zip(s).collect();
                let free: Vec<Fdi> = anatomy::arch_order(Arch::of(fd))
                    .into_iter()
                    .filter(|g| !out.contains_key(g))
                    .collect();
                let Some(&g) = free.iter().min_by_key(|g| {
                    (
                        (i16::from(anatomy::index(**g)) - i16::from(anatomy::index(fd))).abs(),
                        g.get(),
                    )
                }) else {
                    continue;
                };
                let a = pick(&|i| sv.get(&i).is_some_and(|s| *s < cut));
                let b = pick(&|i| sv.get(&i).is_some_and(|s| *s >= cut));
                if let (Some(a), Some(b)) = (a, b) {
                    out.insert(fd, a);
                    out.insert(g, b);
                }
            }
        }
    }
    out.into_iter().collect()
}

/// `itertools.combinations(items, r)` in its order.
fn combinations(items: &[u8], r: usize) -> Vec<Vec<u8>> {
    let n = items.len();
    if r > n {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..r).collect();
    let mut out = vec![idx.iter().map(|&i| items[i]).collect()];
    loop {
        let Some(i) = (0..r).rev().find(|&i| idx[i] != i + n - r) else {
            return out;
        };
        idx[i] += 1;
        for j in i + 1..r {
            idx[j] = idx[j - 1] + 1;
        }
        out.push(idx.iter().map(|&i| items[i]).collect());
    }
}

/// One numbering of the teeth of one quadrant.
struct SideRow {
    cost: f64,
    slots: Vec<u8>,
    canine: f64,
}

/// `numpy.linspace(a, b, n)`.
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    let step = (b - a) / (n - 1) as f64;
    (0..n)
        .map(|i| if i == n - 1 { b } else { a + i as f64 * step })
        .collect()
}

/// Numbers of the teeth from their crown widths along the arch
/// (`renumber_spans`), counted from the midline as a clinician does: every
/// split of the teeth between the two quadrants is tried, each quadrant
/// numbered outwards from the central incisor, any tooth before its last
/// molar possibly missing, the last molar 5-8. Returns {old number: new
/// number} and the notes for the clinician.
pub fn renumber_spans(
    spans: &BTreeMap<u16, Span>,
    arch: Arch,
    include_third: bool,
    missing: &[u16],
) -> (BTreeMap<u16, u16>, Vec<NumberNote>) {
    let mut order: Vec<u16> = spans.keys().copied().collect();
    order.sort_by(|a, b| {
        let (sa, sb) = (spans[a].0 + spans[a].1, spans[b].0 + spans[b].1);
        sa.total_cmp(&sb)
    });
    let identity = || order.iter().map(|&f| (f, f)).collect();
    let n = order.len();
    if n < 2 {
        return (identity(), Vec::new());
    }
    let lo: Vec<f64> = order.iter().map(|f| spans[f].0).collect();
    let hi: Vec<f64> = order.iter().map(|f| spans[f].1).collect();
    let w: Vec<f64> = lo.iter().zip(&hi).map(|(l, h)| h - l).collect();
    let c: Vec<f64> = lo.iter().zip(&hi).map(|(l, h)| 0.5 * (l + h)).collect();
    let (qa, qb): (u16, u16) = if arch.is_upper() { (1, 2) } else { (3, 4) };
    let miss: BTreeSet<u16> = missing.iter().copied().collect();
    let prior = |e: u8| match e {
        5 => 0.5 + MISSING_COST,
        6 => 0.5,
        7 => 0.0,
        _ => {
            if include_third {
                0.0
            } else {
                3.0
            }
        }
    };
    let md = arch_widths(arch);
    let sig: Vec<f64> = (1..=8u8)
        .map(|i| autoseed::width_sigma(Fdi::new(10 + i).expect("a tooth")))
        .collect();

    let side = |idx: &[usize], q: u16, cost_u: &[Vec<f64>]| -> Vec<SideRow> {
        let n = idx.len();
        let mut rows = Vec::new();
        for e in 5u8..=8 {
            let slots: Vec<u8> = (1..=e)
                .filter(|&i| !miss.contains(&(q * 10 + u16::from(i))))
                .collect();
            if (e as usize) < n || slots.last() != Some(&e) {
                continue;
            }
            for comb in combinations(&slots[..slots.len() - 1], n - 1) {
                let mut sl = comb;
                sl.push(e);
                let mut sum = 0.0;
                for t in 0..n {
                    sum += cost_u[usize::from(sl[t]) - 1][idx[t]];
                }
                let cst = prior(e) + sum + MISSING_COST * (slots.len() - n) as f64;
                let canine = if [1u8, 2, 3].iter().all(|i| sl.contains(i)) {
                    let k = sl.iter().position(|&x| x == 3).expect("in sl");
                    c[idx[k]]
                } else {
                    f64::NAN
                };
                rows.push(SideRow {
                    cost: cst,
                    slots: sl,
                    canine,
                });
            }
        }
        rows
    };

    // the numbers in arch order -> (cost, slots of the two quadrants)
    type Key = Vec<u16>;
    type Found = (f64, Vec<u8>, Vec<u8>);
    let mut found: BTreeMap<Key, Found> = BTreeMap::new();
    for k in linspace(0.82, 1.18, 10) {
        let cost_u: Vec<Vec<f64>> = (0..8)
            .map(|i| {
                w.iter()
                    .map(|wj| {
                        let x = (wj - k * md[i]) / (sig[i] * k * md[i]);
                        x * x
                    })
                    .collect()
            })
            .collect();
        for m in 1..n {
            let ia: Vec<usize> = (0..m).rev().collect();
            let ib: Vec<usize> = (m..n).collect();
            let (ra, rb) = (side(&ia, qa, &cost_u), side(&ib, qb, &cost_u));
            if ra.is_empty() || rb.is_empty() {
                continue;
            }
            let s_mid = 0.5 * (hi[m - 1] + lo[m]);
            let base = (s_mid / MIDLINE_SIGMA).powi(2);
            let cols = rb.len();
            let mut flat = Vec::with_capacity(ra.len() * cols);
            for a in &ra {
                for b in &rb {
                    let sym = ((0.5 * (a.canine + b.canine) - s_mid) / CANINE_MID_SIGMA).powi(2);
                    let sym = if sym.is_finite() { sym } else { 0.0 };
                    flat.push(base + a.cost + b.cost + sym);
                }
            }
            let mut idx: Vec<usize> = (0..flat.len()).collect();
            idx.sort_by(|x, y| flat[*x].total_cmp(&flat[*y]));
            for &j in idx.iter().take(8) {
                let (a, b) = (j / cols, j % cols);
                let mut key: Key = ra[a].slots[..ia.len()]
                    .iter()
                    .map(|&s| qa * 10 + u16::from(s))
                    .collect();
                key.reverse();
                key.extend(
                    rb[b].slots[..ib.len()]
                        .iter()
                        .map(|&s| qb * 10 + u16::from(s)),
                );
                let better = found.get(&key).is_none_or(|v| flat[j] < v.0);
                if better {
                    found.insert(key, (flat[j], ra[a].slots.clone(), rb[b].slots.clone()));
                }
            }
        }
    }
    if found.is_empty() {
        return (identity(), Vec::new());
    }
    let mut ranked: Vec<(Key, Found)> = found.into_iter().collect();
    ranked.sort_by(|x, y| x.1.0.total_cmp(&y.1.0).then(x.0.cmp(&y.0)));
    let (labels, (cost, sa, sb)) = ranked[0].clone();
    let new: BTreeMap<u16, u16> = order.iter().copied().zip(labels.iter().copied()).collect();
    let mut notes = Vec::new();
    for (q, sl) in [(qa, &sa), (qb, &sb)] {
        let (has4, has5) = (sl.contains(&4), sl.contains(&5));
        if has4 != has5 && !miss.contains(&(q * 10 + 4)) && !miss.contains(&(q * 10 + 5)) {
            notes.push(NumberNote::OnePremolar {
                quadrant: q as u8,
                fdi: q * 10 + if has4 { 4 } else { 5 },
            });
        }
    }
    let set: BTreeSet<u16> = labels.iter().copied().collect();
    let alt = ranked[1..]
        .iter()
        .find(|(lab, _)| lab.iter().copied().collect::<BTreeSet<u16>>() != set);
    if let Some((alt_labels, (alt_cost, _, _))) = alt
        && alt_cost - cost < NUMBER_MARGIN
    {
        let pairs: Vec<(u16, u16)> = labels
            .iter()
            .zip(alt_labels)
            .filter(|(x, y)| x != y)
            .map(|(x, y)| (*x, *y))
            .take(6)
            .collect();
        notes.push(NumberNote::Uncertain { pairs });
    }
    (new, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(spans: &[(u16, f64, f64)], weak: &[(u16, u16, f64)], arch: Arch) -> Report {
        let spans: BTreeMap<u16, Span> = spans.iter().map(|&(f, a, b)| (f, (a, b))).collect();
        let molar = arch_widths(arch).into_iter().fold(0.0, f64::max);
        Report {
            slivers: spans
                .iter()
                .filter(|(_, (a, b))| b - a < SLIVER_MM)
                .map(|(f, _)| *f)
                .collect(),
            wide: spans
                .iter()
                .filter(|(_, (a, b))| b - a > SPLIT_CROWN * molar)
                .map(|(f, _)| *f)
                .collect(),
            spans,
            weak: weak
                .iter()
                .map(|&(a, b, fold)| Weak {
                    a,
                    b,
                    len_mm: 20.0,
                    fold,
                    s: (0.0, 0.0),
                })
                .collect(),
            molar,
        }
    }

    #[test]
    fn combinations_follow_itertools() {
        let c = combinations(&[1, 2, 3, 4], 2);
        assert_eq!(
            c,
            vec![
                vec![1, 2],
                vec![1, 3],
                vec![1, 4],
                vec![2, 3],
                vec![2, 4],
                vec![3, 4]
            ]
        );
        assert_eq!(combinations(&[1, 2], 0), vec![Vec::<u8>::new()]);
        assert!(combinations(&[1], 2).is_empty());
    }

    #[test]
    fn a_sliver_is_dropped_and_two_parts_of_a_crown_merged_first() {
        let r = rep(
            &[(36, 0.0, 6.0), (37, 6.0, 12.0), (35, -2.5, 0.0)],
            &[(36, 37, 0.5)],
            Arch::Lower,
        );
        assert_eq!(
            plan_corrections(&r),
            vec![
                Action::Drop { fdi: 35 },
                Action::Merge { keep: 36, drop: 37 }
            ]
        );
        assert!(r.doubtful());
        assert_eq!(report_issues(&r).len(), 2);
    }

    #[test]
    fn a_seed_in_a_gap_is_dropped_and_a_wide_crown_split() {
        // 34 between 33 and 35, weak on both sides and the three too wide to merge
        let r = rep(
            &[
                (33, 0.0, 7.0),
                (34, 7.0, 14.0),
                (35, 14.0, 21.0),
                (36, 30.0, 49.0),
            ],
            &[(33, 34, 1.0), (34, 35, 1.2)],
            Arch::Lower,
        );
        // the pairs are not merged: together wider than ONE_CROWN molars? 14 mm < 16.1
        let acts = plan_corrections(&r);
        assert_eq!(acts[0], Action::Merge { keep: 33, drop: 34 });
        let r = rep(
            &[
                (33, 0.0, 9.0),
                (34, 9.0, 18.0),
                (35, 18.0, 27.0),
                (36, 30.0, 49.0),
            ],
            &[(33, 34, 1.0), (34, 35, 1.2)],
            Arch::Lower,
        );
        assert_eq!(
            plan_corrections(&r),
            vec![
                Action::Drop { fdi: 34 },
                Action::Split { fdi: 36, cut: 39.5 }
            ]
        );
    }

    #[test]
    fn the_widths_of_a_lower_arch_give_its_numbers() {
        // six and six teeth side by side with the typical widths, numbered all wrong
        let typical = arch_widths(Arch::Lower);
        let mut spans = BTreeMap::new();
        let mut s = -(typical[..6].iter().sum::<f64>());
        for (wrong, &w) in (11u16..).zip(typical[..6].iter().rev().chain(&typical[..6])) {
            spans.insert(wrong, (s, s + w));
            s += w;
        }
        let (new, _) = renumber_spans(&spans, Arch::Lower, false, &[]);
        let got: Vec<u16> = new.values().copied().collect();
        assert_eq!(got, vec![36, 35, 34, 33, 32, 31, 41, 42, 43, 44, 45, 46]);
    }

    /// The crown spans of the printed test piece (upper arch) after the
    /// segmentation of the automatic chain, as the check measured them.
    fn test_piece() -> Vec<(u16, f64, f64)> {
        vec![
            (11, -13.98, -4.38),
            (12, -19.78, -13.18),
            (13, -28.18, -20.03),
            (14, -34.93, -28.58),
            (15, -40.48, -35.04),
            (16, -48.83, -40.53),
            (22, 3.92, 11.72),
            (23, 12.22, 21.27),
            (24, 21.02, 28.37),
            (25, 28.1, 43.77),
            (26, 34.62, 45.42),
        ]
    }

    fn merges(acts: &[Action]) -> Vec<(u16, u16)> {
        acts.iter()
            .filter_map(|a| match *a {
                Action::Merge { keep, drop } => Some((keep, drop)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_fused_contacts_of_the_test_piece_keep_their_teeth() {
        // the four contacts the plugin read as weak (folds 1.45-1.96)
        let weak = [
            (11, 12, 1.78),
            (12, 13, 1.83),
            (14, 15, 1.96),
            (15, 16, 1.45),
        ];
        let r = rep(&test_piece(), &weak, Arch::Upper);
        assert_eq!(merges(&plan_corrections(&r)), vec![(15, 16), (11, 12)]);
        let v = vetted(&r, Arch::Upper);
        assert!(v.weak.is_empty(), "{:?}", v.weak);
        assert!(plan_corrections(&v).is_empty());
        // with no groove at all, they are still two teeth each: too wide
        let flat = weak.map(|(a, b, _)| (a, b, 0.3));
        let r = rep(&test_piece(), &flat, Arch::Upper);
        assert!(vetted(&r, Arch::Upper).weak.is_empty());
    }

    #[test]
    fn a_crown_split_in_two_on_the_test_piece_is_merged() {
        // the seed of 16 moved distally and a second seed (17) on its crown
        let mut spans = test_piece();
        spans[5] = (16, -48.93, -40.53);
        spans.push((17, -55.08, -47.37));
        let weak = [
            (11, 12, 1.78),
            (12, 13, 1.83),
            (14, 15, 1.89),
            (15, 16, 1.41),
            (16, 17, 0.45),
        ];
        let r = rep(&spans, &weak, Arch::Upper);
        let v = vetted(&r, Arch::Upper);
        assert_eq!(v.weak.len(), 1);
        assert_eq!(merges(&plan_corrections(&v)), vec![(16, 17)]);
        // a part as wide as a whole tooth is a tooth: not merged
        spans[5] = (16, -51.0, -40.53);
        spans[11] = (17, -55.08, -50.0);
        let r = rep(&spans, &weak, Arch::Upper);
        assert!(vetted(&r, Arch::Upper).weak.is_empty());
    }

    #[test]
    fn only_the_teeth_distal_to_a_correction_are_renumbered() {
        // 16 merged into 15: the tooth after it (17) becomes 16; the
        // renumbering from the widths also moved 13, mesial to the merge,
        // and the teeth of the other side
        let all: BTreeMap<u16, u16> = [
            (11, 11),
            (12, 12),
            (13, 14),
            (14, 14),
            (15, 15),
            (17, 16),
            (21, 21),
            (25, 26),
            (26, 27),
        ]
        .into_iter()
        .collect();
        let touched: BTreeMap<u16, u8> = [(1, 5)].into_iter().collect();
        let got = renumber_touched(&all, &touched).unwrap();
        let moved: Vec<(u16, u16)> = got.into_iter().filter(|(f, g)| f != g).collect();
        assert_eq!(moved, vec![(17, 16)]);
        // a tooth after the merge taking the number of one before it: refused
        let clash: BTreeMap<u16, u16> = [(14, 14), (15, 14)].into_iter().collect();
        assert_eq!(renumber_touched(&clash, &touched), None);
        // a number of a quadrant no correction touched: refused
        let into_other: BTreeMap<u16, u16> = [(15, 25), (21, 21)].into_iter().collect();
        assert_eq!(renumber_touched(&into_other, &touched), None);
    }
}
