//! Diagnosis: space analysis, Little irregularity index, Bolton analysis,
//! complexity score and the evidence-based classifications.
//!
//! Reference: `ortho_aligner/planning.py` (space analysis, irregularity,
//! complexity), `case_plan.analyze` (the combination the plugin stores),
//! `report.bolton` (Bolton) and `ortho_aligner/classify.py` (classifications,
//! in [`classify`]).
//!
//! * Space analysis: the available perimeter is the arc length, on a
//!   4th-order polynomial fitted to the crown reference points (AlHarbi
//!   2008), between the mesial contacts of the two first molars (the last
//!   tooth of a side when it has no first molar); the required space is the
//!   sum of the mesio-distal widths of the teeth mesial to them. The
//!   discrepancy is available minus required: negative means crowding.
//! * Little 1975: sum of the five linear displacements between the
//!   anatomical contact points of the six anterior teeth, projected on the
//!   occlusal plane.
//! * Largest space: the 3D clearance between adjacent crowns (needs the
//!   crown vertices).
//! * Complexity: the model-analysis items of CAT-CAT (Long 2020) that can be
//!   computed from the scan, with the difficulty levels of the 2025 expert
//!   consensus (Wang 2025).
//! * Bolton 1958: overall (6-6) and anterior (3-3) ratios, lower over upper.
//!
//! All the geometry is in arch coordinates (origin between the last molars,
//! `+Y` anterior, `+Z` occlusal; see [`crate::frame::ArchFrame`]). The
//! values the plugin rounds before storing (rotations to 0.1 degree, widths
//! to 0.01 mm) are rounded the same way in the report fields.

use std::collections::{BTreeMap, HashMap};

use dc_case::Fdi;
use dca_model::{ArchSpace, ToothMeasures};
use glam::{DVec2, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::{self, Arch};
use crate::archform::{self, ArchModel};
use crate::teeth::{ToothGeom, Which};

pub mod classify;

/// Errors of the diagnosis stage.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DiagnosisError {
    /// An arch asked for has no measured teeth.
    #[error("the {0} arch has no measured teeth")]
    NoTeeth(&'static str),
    /// Nothing to analyse.
    #[error("no arch has measured teeth")]
    NothingToDo,
}

fn arch_name(arch: Arch) -> &'static str {
    if arch.is_upper() { "upper" } else { "lower" }
}

/// Parameters of the diagnosis stage.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to analyse.
    pub arch: Which,
}

/// Plugin category of the Little index (`little_category`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LittleCategory {
    /// Below 1 mm.
    #[serde(rename = "allineamento perfetto")]
    Perfect,
    /// Up to 3 mm.
    #[serde(rename = "irregolarita minima")]
    Minimal,
    /// Up to 6 mm.
    #[serde(rename = "irregolarita moderata")]
    Moderate,
    /// Up to 9 mm.
    #[serde(rename = "irregolarita grave")]
    Severe,
    /// Above 9 mm.
    #[serde(rename = "irregolarita molto grave")]
    VerySevere,
}

impl LittleCategory {
    /// Category of an index value (mm).
    pub fn of(v: f64) -> Self {
        if v < 1.0 {
            Self::Perfect
        } else if v <= 3.0 {
            Self::Minimal
        } else if v <= 6.0 {
            Self::Moderate
        } else if v <= 9.0 {
            Self::Severe
        } else {
            Self::VerySevere
        }
    }

    /// The plugin's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Perfect => "allineamento perfetto",
            Self::Minimal => "irregolarita minima",
            Self::Moderate => "irregolarita moderata",
            Self::Severe => "irregolarita grave",
            Self::VerySevere => "irregolarita molto grave",
        }
    }
}

/// Difficulty level of the case (`consensus_level`, Wang 2025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    /// Score up to 20.
    #[serde(rename = "FACILE")]
    Easy,
    /// Score up to 40.
    #[serde(rename = "MODERATO")]
    Moderate,
    /// Score up to 60.
    #[serde(rename = "DIFFICILE")]
    Difficult,
    /// Score above 60.
    #[serde(rename = "MOLTO COMPLESSO")]
    VeryComplex,
}

impl Level {
    /// The levels, easiest first.
    pub const ALL: [Level; 4] = [
        Level::Easy,
        Level::Moderate,
        Level::Difficult,
        Level::VeryComplex,
    ];

    /// Level of a CAT-CAT score.
    pub fn of(score: u32) -> Self {
        match score {
            0..=20 => Self::Easy,
            21..=40 => Self::Moderate,
            41..=60 => Self::Difficult,
            _ => Self::VeryComplex,
        }
    }

    /// The plugin's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Easy => "FACILE",
            Self::Moderate => "MODERATO",
            Self::Difficult => "DIFFICILE",
            Self::VeryComplex => "MOLTO COMPLESSO",
        }
    }
}

/// The CAT-CAT model-analysis items that can be computed from the scan
/// (`catcat_model_items`); the serialised keys are the plugin's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct CatcatItems {
    /// Crowding: 0-3 mm = 0, 3-5 = 2, 5-7 = 3, above 7 = 4.
    #[serde(rename = "affollamento")]
    pub crowding: u32,
    /// Rotations of premolars and lateral incisors: 2 points per 10 degrees.
    #[serde(rename = "rotazioni (premolari/incisivi laterali)")]
    pub rotations: u32,
    /// Molar movement (distalization or mesialization).
    #[serde(rename = "movimento molare")]
    pub molar_movement: u32,
    /// Midline deviation: 2 points per mm beyond 2 mm.
    #[serde(rename = "deviazione mediana")]
    pub midline: u32,
    /// Largest diastema: 2 points per mm beyond 2 mm.
    #[serde(rename = "diastema maggiore")]
    pub diastema: u32,
}

impl CatcatItems {
    /// Sum of the items.
    pub fn score(&self) -> u32 {
        self.crowding + self.rotations + self.molar_movement + self.midline + self.diastema
    }
}

/// Model-analysis items of CAT-CAT (`catcat_model_items`).
///
/// `discrepancy` is available minus required space (mm), `rotations` the
/// rotation of each tooth about its long axis (degrees), `molar_moves` the
/// planned molar movement per side (mm, `+` distal, `-` mesial; empty at the
/// diagnosis), `midline_dev` the dental midline offset and `largest_gap` the
/// largest interdental space (mm).
pub fn catcat_model_items(
    discrepancy: f64,
    rotations: &BTreeMap<Fdi, f64>,
    molar_moves: &[f64],
    midline_dev: f64,
    largest_gap: f64,
) -> CatcatItems {
    let crowd = (-discrepancy).max(0.0);
    let crowding = if crowd <= 3.0 {
        0
    } else if crowd <= 5.0 {
        2
    } else if crowd <= 7.0 {
        3
    } else {
        4
    };
    let mut rot = 0;
    for (&f, &deg) in rotations {
        let i = anatomy::index(f);
        if i == 4 || i == 5 || i == 2 {
            rot += 2 * (deg.abs() / 10.0).floor() as u32;
        }
    }
    let mut mol = 0;
    for &mv in molar_moves {
        if mv > 0.25 {
            mol = mol.max(if mv < 2.0 {
                0
            } else if mv <= 5.0 {
                3
            } else {
                7
            });
        } else if mv < -0.25 {
            let m = -mv;
            mol = mol.max(if m < 2.0 {
                3
            } else if m <= 5.0 {
                7
            } else {
                12
            });
        }
    }
    CatcatItems {
        crowding,
        rotations: rot,
        molar_movement: mol,
        midline: ((midline_dev.abs() - 2.0) * 2.0).max(0.0) as u32,
        diastema: if largest_gap > 2.0 {
            ((largest_gap - 2.0).max(0.0) * 2.0) as u32
        } else {
            0
        },
    }
}

// ---- space analysis -----------------------------------------------------------------------------------

fn xy(p: DVec3) -> DVec2 {
    DVec2::new(p.x, p.y)
}

/// The teeth of each side of the arch, ordered from the midline outwards
/// (`side_lists`): right (quadrants 1 and 4) and left.
fn side_lists(seq: &[Fdi]) -> (Vec<Fdi>, Vec<Fdi>) {
    let mut right: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| anatomy::is_right(*f))
        .collect();
    let mut left: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| !anatomy::is_right(*f))
        .collect();
    right.sort_by_key(|f| anatomy::index(*f));
    left.sort_by_key(|f| anatomy::index(*f));
    (right, left)
}

/// The first molar of a side, or the last tooth of the side (`anchor_of`).
fn anchor_of(side: &[Fdi]) -> Option<Fdi> {
    side.iter()
        .copied()
        .find(|f| anatomy::index(*f) == 6)
        .or_else(|| side.last().copied())
}

/// Pairs of neighbours along the arch, across the midline included
/// (`adjacent_pairs`).
pub fn adjacent_pairs(seq: &[Fdi]) -> Vec<(Fdi, Fdi)> {
    seq.windows(2)
        .filter(|w| {
            let (a, b) = (w[0], w[1]);
            anatomy::is_right(a) == anatomy::is_right(b)
                || (anatomy::index(a) == 1 && anatomy::index(b) == 1)
        })
        .map(|w| (w[0], w[1]))
        .collect()
}

/// Quartic fitted to the crown reference points (AlHarbi 2008), evaluated
/// only over the teeth and continued with straight tangents
/// (`current_curve`).
fn current_curve(teeth: &BTreeMap<Fdi, ToothMeasures>, seq: &[Fdi]) -> archform::ArchCurve {
    let pts: Vec<DVec2> = seq.iter().map(|f| xy(teeth[f].reference)).collect();
    let abc = archform::fit_quartic(&pts, None);
    let ext = pts.iter().map(|p| p.x.abs()).fold(0.0, f64::max) + 0.5;
    archform::curve_from_model(&ArchModel::Quartic(abc), ext, false)
}

/// Space analysis of an arch (`space_analysis`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Space {
    /// Arc length between the mesial contacts of the anchor molars (mm).
    pub available: f64,
    /// Sum of the mesio-distal widths of the teeth mesial to the anchors
    /// (mm).
    pub required: f64,
    /// Available minus required (mm); negative means crowding.
    pub discrepancy: f64,
    /// The anchor teeth (right, left); `None` when a side has no teeth.
    pub anchors: Option<[Fdi; 2]>,
}

/// Space analysis of an arch whose teeth (arch coordinates) are `teeth`, in
/// arch order `seq`.
pub fn space_analysis(teeth: &BTreeMap<Fdi, ToothMeasures>, seq: &[Fdi]) -> Space {
    let curve = current_curve(teeth, seq);
    let (right, left) = side_lists(seq);
    let (Some(ar), Some(al)) = (anchor_of(&right), anchor_of(&left)) else {
        return Space {
            available: 0.0,
            required: 0.0,
            discrepancy: 0.0,
            anchors: None,
        };
    };
    let s_r = curve.project(xy(teeth[&ar].contact_m));
    let s_l = curve.project(xy(teeth[&al].contact_m));
    let available = (s_r - s_l).abs();
    let required: f64 = right
        .iter()
        .filter(|f| anatomy::index(**f) < anatomy::index(ar))
        .chain(
            left.iter()
                .filter(|f| anatomy::index(**f) < anatomy::index(al)),
        )
        .map(|f| teeth[f].width_md)
        .sum();
    Space {
        available,
        required,
        discrepancy: available - required,
        anchors: Some([ar, al]),
    }
}

/// The contact point of tooth `a` that faces tooth `b` (`facing_point`).
fn facing_point(a: &ToothMeasures, b: &ToothMeasures) -> DVec3 {
    if (a.contact_m - b.centroid).length() <= (a.contact_d - b.centroid).length() {
        a.contact_m
    } else {
        a.contact_d
    }
}

/// Displacement between the facing contact points of two neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LittleStep {
    /// First tooth.
    pub a: Fdi,
    /// Second tooth.
    pub b: Fdi,
    /// Distance in the occlusal plane (mm).
    pub mm: f64,
}

/// Little 1975 irregularity index of an arch: the sum, and the five
/// displacements it is made of (`little_index`).
pub fn little_index(teeth: &BTreeMap<Fdi, ToothMeasures>, arch: Arch) -> (f64, Vec<LittleStep>) {
    let pairs: [(u8, u8); 5] = if arch == Arch::Upper {
        [(13, 12), (12, 11), (11, 21), (21, 22), (22, 23)]
    } else {
        [(43, 42), (42, 41), (41, 31), (31, 32), (32, 33)]
    };
    let mut total = 0.0;
    let mut steps = Vec::new();
    for (a, b) in pairs {
        let (Ok(a), Ok(b)) = (Fdi::new(a), Fdi::new(b)) else {
            continue;
        };
        if let (Some(ta), Some(tb)) = (teeth.get(&a), teeth.get(&b)) {
            let d = (xy(facing_point(ta, tb)) - xy(facing_point(tb, ta))).length();
            total += d;
            steps.push(LittleStep { a, b, mm: d });
        }
    }
    (total, steps)
}

// ---- clearance between crowns -------------------------------------------------------------------------

/// Cell of the grid used by [`contact_separation`] (mm).
const CLEARANCE_CELL_MM: f64 = 0.35;

fn normalize(v: DVec3) -> DVec3 {
    let n = v.length();
    if n == 0.0 { v } else { v / n }
}

/// Signed clearance between two point-sampled crowns along the direction
/// `u` (a to b): the minimum over the cells of a grid on the plane
/// orthogonal to `u` of (lowest b along `u` minus highest a along `u`).
/// Negative means interpenetration; infinite when the crowns do not face
/// each other (`meshutil.contact_separation`).
fn contact_separation(pa: &[DVec3], pb: &[DVec3], u: DVec3) -> f64 {
    if pa.is_empty() || pb.is_empty() {
        return f64::INFINITY;
    }
    let v = normalize(DVec3::Z - DVec3::Z.dot(u) * u);
    let w = u.cross(v);
    let cell = |p: DVec3| -> i64 {
        (p.dot(v) / CLEARANCE_CELL_MM).floor() as i64 * 1_000_003
            + (p.dot(w) / CLEARANCE_CELL_MM).floor() as i64
    };
    // per cell: the highest a along u and the lowest b along u
    let mut cells: HashMap<i64, (f64, f64)> = HashMap::new();
    for &p in pa {
        let e = cells
            .entry(cell(p))
            .or_insert((f64::NEG_INFINITY, f64::INFINITY));
        e.0 = e.0.max(p.dot(u));
    }
    let mut best = f64::INFINITY;
    let mut lowest_b: HashMap<i64, f64> = HashMap::new();
    for &p in pb {
        let e = lowest_b.entry(cell(p)).or_insert(f64::INFINITY);
        *e = e.min(p.dot(u));
    }
    for (k, (top_a, _)) in &cells {
        if let Some(low_b) = lowest_b.get(k) {
            best = best.min(low_b - top_a);
        }
    }
    best
}

/// Clearance between two crowns along the horizontal direction joining their
/// centroids, using the facing halves only (`meshutil.facing_clearance`).
pub fn facing_clearance(pa: &[DVec3], pb: &[DVec3]) -> f64 {
    if pa.is_empty() || pb.is_empty() {
        return f64::INFINITY;
    }
    let mean = |p: &[DVec3]| p.iter().copied().sum::<DVec3>() / p.len() as f64;
    let (ca, cb) = (mean(pa), mean(pb));
    let mut u = cb - ca;
    u.z = 0.0;
    if u.length() < 1e-9 {
        return f64::INFINITY;
    }
    let u = normalize(u);
    let a: Vec<DVec3> = pa
        .iter()
        .copied()
        .filter(|p| (*p - ca).dot(u) > 0.0)
        .collect();
    let b: Vec<DVec3> = pb
        .iter()
        .copied()
        .filter(|p| (*p - cb).dot(u) < 0.0)
        .collect();
    contact_separation(&a, &b, u)
}

/// Crown vertices of each tooth: the vertices carrying its label.
pub type Crowns = BTreeMap<Fdi, Vec<DVec3>>;

/// Groups the vertices by tooth label (`0` and labels that are no tooth
/// number are skipped).
pub fn crowns_of(vertices: &[DVec3], labels: &[u16]) -> Crowns {
    let mut out = Crowns::new();
    for (&p, &label) in vertices.iter().zip(labels) {
        if let Some(fdi) = u8::try_from(label)
            .ok()
            .and_then(|n| Fdi::new(n).ok())
            .filter(|_| label != 0)
        {
            out.entry(fdi).or_default().push(p);
        }
    }
    out
}

/// Largest interdental space: the largest 3D clearance between adjacent
/// crowns (`largest_space`). `crowns` holds the crown vertices of each tooth
/// in arch coordinates; a pair with a missing or non-facing crown is skipped.
pub fn largest_space(seq: &[Fdi], crowns: &Crowns) -> f64 {
    let mut best = 0.0f64;
    for (a, b) in adjacent_pairs(seq) {
        let (Some(pa), Some(pb)) = (crowns.get(&a), crowns.get(&b)) else {
            continue;
        };
        let sep = facing_clearance(pa, pb);
        if sep.is_finite() {
            best = best.max(sep);
        }
    }
    best
}

// ---- Bolton -------------------------------------------------------------------------------------------

/// Bolton 1958 norm of the overall ratio, mean and standard deviation (%).
pub const BOLTON_OVERALL_NORM: (f64, f64) = (91.3, 1.91);
/// Bolton 1958 norm of the anterior ratio, mean and standard deviation (%).
pub const BOLTON_ANTERIOR_NORM: (f64, f64) = (77.2, 1.65);

/// Bolton ratios (lower over upper, per cent); `None` when a tooth of the
/// segment is missing in either arch.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Bolton {
    /// Overall ratio, first molar to first molar.
    pub overall: Option<f64>,
    /// Anterior ratio, canine to canine.
    pub anterior: Option<f64>,
}

/// Bolton ratios from the mesio-distal widths of the upper and lower teeth
/// (`report.bolton`).
pub fn bolton(upper: &BTreeMap<Fdi, f64>, lower: &BTreeMap<Fdi, f64>) -> Bolton {
    fn total(w: &BTreeMap<Fdi, f64>, arch: Arch, max_index: u8) -> Option<f64> {
        let mut sum = 0.0;
        for f in anatomy::arch_order(arch) {
            if anatomy::index(f) <= max_index {
                sum += w.get(&f)?;
            }
        }
        Some(sum)
    }
    let ratio = |max_index| match (
        total(upper, Arch::Upper, max_index),
        total(lower, Arch::Lower, max_index),
    ) {
        (Some(u), Some(l)) if u != 0.0 && l != 0.0 => Some(100.0 * l / u),
        _ => None,
    };
    Bolton {
        overall: ratio(6),
        anterior: ratio(3),
    }
}

// ---- the stage ----------------------------------------------------------------------------------------

/// The measured teeth of one arch.
#[derive(Debug, Clone, Copy)]
pub struct ArchInput<'a> {
    /// The arch.
    pub arch: Arch,
    /// Measures of its teeth, **in arch coordinates**.
    pub teeth: &'a BTreeMap<Fdi, ToothMeasures>,
    /// Crown vertices of each tooth in arch coordinates, for the largest
    /// space; without them it is reported as 0.
    pub crowns: Option<&'a Crowns>,
}

/// Input of the diagnosis stage.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    /// Data of the arches that are available.
    pub arches: Vec<ArchInput<'a>>,
}

/// Diagnosis of one arch (`case_plan.analyze`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchDiagnosis {
    /// The teeth, along the arch from the patient's right distal.
    pub sequence: Vec<Fdi>,
    /// Space analysis.
    pub space: Space,
    /// Little irregularity index (mm).
    pub little: f64,
    /// The displacements the index is made of.
    pub little_details: Vec<LittleStep>,
    /// Category of the index.
    pub little_category: LittleCategory,
    /// Rotation of each tooth about its long axis relative to the arch
    /// (degrees, rounded to 0.1 as the plugin stores it).
    pub rotations: BTreeMap<Fdi, f64>,
    /// Mesio-distal width of each tooth (mm, rounded to 0.01).
    pub widths: BTreeMap<Fdi, f64>,
    /// Mean `x` of the mesial contacts of the central incisors: the dental
    /// midline against the middle of the arch (mm).
    pub midline_offset: f64,
    /// Largest interdental space (mm).
    pub largest_space: f64,
    /// CAT-CAT items.
    pub catcat_items: CatcatItems,
    /// Sum of the items.
    pub catcat_score: u32,
    /// Difficulty level of the score.
    pub level: Level,
}

impl ArchDiagnosis {
    /// Crowding (positive) or spacing (negative), mm.
    pub fn crowding_mm(&self) -> f64 {
        -self.space.discrepancy
    }

    /// The part stored in the case ([`ArchSpace`]).
    pub fn arch_space(&self) -> ArchSpace {
        ArchSpace {
            available_mm: self.space.available,
            required_mm: self.space.required,
            little_index_mm: self.little,
        }
    }
}

fn round_to(v: f64, decimals: i32) -> f64 {
    let k = 10f64.powi(decimals);
    (v * k).round() / k
}

/// Diagnosis of one arch.
pub fn analyze_arch(input: &ArchInput<'_>) -> Result<ArchDiagnosis, DiagnosisError> {
    let fdis: Vec<Fdi> = input
        .teeth
        .keys()
        .copied()
        .filter(|f| anatomy::arch_of(*f) == input.arch)
        .collect();
    let seq = anatomy::arch_sequence(&fdis, input.arch);
    if seq.is_empty() {
        return Err(DiagnosisError::NoTeeth(arch_name(input.arch)));
    }
    let teeth = input.teeth;
    let space = space_analysis(teeth, &seq);
    let (little, little_details) = little_index(teeth, input.arch);
    let centrals: Vec<f64> = seq
        .iter()
        .filter(|f| anatomy::index(**f) == 1)
        .map(|f| teeth[f].contact_m.x)
        .collect();
    let midline_offset = if centrals.is_empty() {
        0.0
    } else {
        centrals.iter().sum::<f64>() / centrals.len() as f64
    };
    let rot: BTreeMap<Fdi, f64> = seq.iter().map(|f| (*f, teeth[f].rot_dev_deg)).collect();
    let largest = input
        .crowns
        .map_or(0.0, |crowns| largest_space(&seq, crowns));
    let catcat_items = catcat_model_items(space.discrepancy, &rot, &[], midline_offset, largest);
    let catcat_score = catcat_items.score();
    Ok(ArchDiagnosis {
        rotations: rot.iter().map(|(f, v)| (*f, round_to(*v, 1))).collect(),
        widths: seq
            .iter()
            .map(|f| (*f, round_to(teeth[f].width_md, 2)))
            .collect(),
        sequence: seq,
        space,
        little,
        little_details,
        little_category: LittleCategory::of(little),
        midline_offset,
        largest_space: largest,
        catcat_items,
        catcat_score,
        level: Level::of(catcat_score),
    })
}

/// Result of the diagnosis stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Diagnosis of each arch that was analysed.
    pub arches: BTreeMap<Arch, ArchDiagnosis>,
    /// Bolton ratios, from every arch given in the input.
    pub bolton: Bolton,
}

impl Output {
    /// Complexity score of the case: the CAT-CAT score of its most complex
    /// arch.
    pub fn complexity_score(&self) -> Option<u32> {
        self.arches.values().map(|d| d.catcat_score).max()
    }
}

/// Diagnosis of the arches selected by the parameters; the Bolton ratios use
/// both arches when both are in the input.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, DiagnosisError> {
    let selected = params.arch.arches();
    let explicit = params.arch != Which::All;
    let mut arches = BTreeMap::new();
    for arch in selected {
        match input.arches.iter().find(|a| a.arch == arch) {
            Some(a) => match analyze_arch(a) {
                Ok(d) => {
                    arches.insert(arch, d);
                }
                Err(e) if explicit => return Err(e),
                Err(_) => {}
            },
            None if explicit => return Err(DiagnosisError::NoTeeth(arch_name(arch))),
            None => {}
        }
    }
    if arches.is_empty() {
        return Err(DiagnosisError::NothingToDo);
    }
    let widths = |arch: Arch| -> BTreeMap<Fdi, f64> {
        input
            .arches
            .iter()
            .filter(|a| a.arch == arch)
            .flat_map(|a| a.teeth.iter())
            .filter(|(f, _)| anatomy::arch_of(**f) == arch)
            .map(|(f, t)| (*f, t.width_md))
            .collect()
    };
    Ok(Output {
        arches,
        bolton: bolton(&widths(Arch::Upper), &widths(Arch::Lower)),
    })
}

/// The measures the diagnosis reads from a tooth of the teeth stage,
/// in the coordinates of `tooth`.
pub fn tooth_measures(tooth: &ToothGeom) -> ToothMeasures {
    ToothMeasures {
        centroid: tooth.centroid,
        reference: tooth.reference,
        width_md: tooth.width_md,
        width_bl: tooth.width_bl,
        height: tooth.height,
        tip: tooth.tip,
        cej: tooth.cej,
        crest: tooth.crest,
        apex: tooth.apex,
        contact_m: tooth.contact_m,
        contact_d: tooth.contact_d,
        rot_dev_deg: tooth.rot_dev_deg,
        axis_tilt_deg: tooth.axis_tilt_deg,
        md_sign: tooth.md_sign,
    }
}

/// The measures moved by a rigid transform (case to arch coordinates, for
/// instance). Distances and angles are unchanged.
pub fn transform_measures(m: &ToothMeasures, to: &glam::DMat4) -> ToothMeasures {
    let p = |v: DVec3| to.transform_point3(v);
    ToothMeasures {
        centroid: p(m.centroid),
        reference: p(m.reference),
        tip: p(m.tip),
        cej: p(m.cej),
        crest: p(m.crest),
        apex: p(m.apex),
        contact_m: p(m.contact_m),
        contact_d: p(m.contact_d),
        ..*m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn little_categories_and_levels_follow_the_thresholds() {
        use LittleCategory::*;
        for (v, c) in [
            (0.5, Perfect),
            (1.0, Minimal),
            (3.0, Minimal),
            (3.1, Moderate),
            (6.0, Moderate),
            (9.0, Severe),
            (9.5, VerySevere),
        ] {
            assert_eq!(LittleCategory::of(v), c, "{v}");
        }
        for (s, l) in [
            (0, Level::Easy),
            (20, Level::Easy),
            (21, Level::Moderate),
            (40, Level::Moderate),
            (41, Level::Difficult),
            (60, Level::Difficult),
            (61, Level::VeryComplex),
        ] {
            assert_eq!(Level::of(s), l, "{s}");
        }
        assert_eq!(Level::Easy.label(), "FACILE");
        assert_eq!(
            serde_json::to_value(Level::VeryComplex).unwrap(),
            "MOLTO COMPLESSO"
        );
        assert_eq!(
            serde_json::to_value(LittleCategory::Moderate).unwrap(),
            LittleCategory::Moderate.label()
        );
    }

    #[test]
    fn catcat_items_score_each_model_finding() {
        let mut rot = BTreeMap::new();
        rot.insert(fdi(14), 25.0); // 2 x 2
        rot.insert(fdi(12), -10.0); // 1 x 2
        rot.insert(fdi(11), 40.0); // incisor central: not scored
        rot.insert(fdi(16), 50.0); // molar: not scored
        let items = catcat_model_items(-6.0, &rot, &[], 3.0, 3.5);
        assert_eq!(items.crowding, 3);
        assert_eq!(items.rotations, 6);
        assert_eq!(items.molar_movement, 0);
        assert_eq!(items.midline, 2);
        assert_eq!(items.diastema, 3);
        assert_eq!(items.score(), 14);
        // spacing is no crowding
        assert_eq!(
            catcat_model_items(4.0, &BTreeMap::new(), &[], 0.0, 0.0).score(),
            0
        );
        // molars: distal 3 mm = 3, mesial 6 mm = 12, the worst side counts
        assert_eq!(
            catcat_model_items(0.0, &BTreeMap::new(), &[3.0, -6.0], 0.0, 0.0).molar_movement,
            12
        );
        assert_eq!(
            catcat_model_items(0.0, &BTreeMap::new(), &[0.2, -0.2], 0.0, 0.0).molar_movement,
            0
        );
        for (crowd, expected) in [(3.0, 0), (3.1, 2), (5.0, 2), (7.0, 3), (7.1, 4)] {
            assert_eq!(
                catcat_model_items(-crowd, &BTreeMap::new(), &[], 0.0, 0.0).crowding,
                expected
            );
        }
    }

    #[test]
    fn the_bolton_ratios_need_a_complete_segment() {
        let arch_widths = |arch: Arch, skip: Option<u8>, w: f64| -> BTreeMap<Fdi, f64> {
            anatomy::arch_order(arch)
                .into_iter()
                .filter(|f| Some(f.get()) != skip)
                .map(|f| (f, if anatomy::index(f) <= 6 { w } else { 10.0 }))
                .collect()
        };
        let up = arch_widths(Arch::Upper, None, 8.0);
        let lo = arch_widths(Arch::Lower, None, 7.0);
        let b = bolton(&up, &lo);
        assert!((b.overall.unwrap() - 87.5).abs() < 1e-12);
        assert!((b.anterior.unwrap() - 87.5).abs() < 1e-12);
        // no lower first molar: the overall ratio is lost, the anterior one is not
        let b = bolton(&up, &arch_widths(Arch::Lower, Some(36), 7.0));
        assert_eq!(b.overall, None);
        assert!(b.anterior.is_some());
        // no upper canine: neither
        let b = bolton(&arch_widths(Arch::Upper, Some(13), 8.0), &lo);
        assert_eq!((b.overall, b.anterior), (None, None));
        assert_eq!(
            bolton(&BTreeMap::new(), &BTreeMap::new()),
            Bolton::default()
        );
    }

    #[test]
    fn two_blocks_facing_each_other_have_a_clearance() {
        // two 2 mm cubes sampled on a grid, 1.5 mm apart along x
        let cube = |x0: f64| -> Vec<DVec3> {
            let mut p = Vec::new();
            for i in 0..=8 {
                for j in 0..=8 {
                    for k in 0..=8 {
                        p.push(DVec3::new(
                            x0 + f64::from(i) * 0.25,
                            f64::from(j) * 0.25,
                            f64::from(k) * 0.25,
                        ));
                    }
                }
            }
            p
        };
        let (a, b) = (cube(0.0), cube(3.5));
        let sep = facing_clearance(&a, &b);
        assert!((sep - 1.5).abs() < 1e-9, "{sep}");
        // overlapping crowns have a negative clearance
        let sep = facing_clearance(&a, &cube(1.5));
        assert!(sep < 0.0, "{sep}");
        // identical centroids, or an empty crown, do not face
        assert_eq!(facing_clearance(&a, &a), f64::INFINITY);
        assert_eq!(facing_clearance(&a, &[]), f64::INFINITY);
        // the largest space skips what it cannot measure
        let mut crowns = BTreeMap::new();
        crowns.insert(fdi(11), a);
        crowns.insert(fdi(21), b);
        let seq = [fdi(11), fdi(21), fdi(22)];
        assert!((largest_space(&seq, &crowns) - 1.5).abs() < 1e-9);
        assert_eq!(largest_space(&[], &crowns), 0.0);
    }

    #[test]
    fn adjacent_pairs_cross_the_midline_but_not_the_quadrants() {
        let seq: Vec<Fdi> = [12, 11, 21, 22].map(fdi).to_vec();
        assert_eq!(
            adjacent_pairs(&seq),
            vec![(fdi(12), fdi(11)), (fdi(11), fdi(21)), (fdi(21), fdi(22))]
        );
        // a gap in the arch: 11 and 22 are no neighbours
        let seq: Vec<Fdi> = [11, 22].map(fdi).to_vec();
        assert!(adjacent_pairs(&seq).is_empty());
        assert_eq!(anchor_of(&[fdi(11), fdi(13)]), Some(fdi(13)));
        assert_eq!(anchor_of(&[fdi(11), fdi(16), fdi(17)]), Some(fdi(16)));
        assert_eq!(anchor_of(&[]), None);
    }

    #[test]
    fn an_arch_without_teeth_is_an_error() {
        let none = BTreeMap::new();
        let input = Input {
            arches: vec![ArchInput {
                arch: Arch::Upper,
                teeth: &none,
                crowns: None,
            }],
        };
        assert_eq!(
            run(&input, &Params::default()),
            Err(DiagnosisError::NothingToDo)
        );
        let lower = Params { arch: Which::Lower };
        assert_eq!(run(&input, &lower), Err(DiagnosisError::NoTeeth("lower")));
    }
}
