//! Setup: the final position of the teeth, every aligned tooth with its
//! prominent cusps on the patient's own arch line, the space made first by
//! the therapy (IPR, expansion, incisor advancement) in the chosen order and
//! only as much as needed.
//!
//! Reference: `ortho_aligner/setup_conservativo.py` (the only automatic
//! setup), `planning.cusp_points`, and the numerical core of
//! `case_plan.run_setup` (`conservative_config`, `rest_cusps`,
//! `rest_crowns`, `_resample`).
//!
//! Steps (`conservative_setup`):
//! 1. the arch line: the prominent cusps in arch order smoothed by a robust
//!    local quadratic regression (`line::cusp_line`), or the line drawn by
//!    the clinician;
//! 2. the evaluation: a tooth to align moves only when it is off the line,
//!    rotated or stepped beyond the tolerances of its zone;
//! 3. the space along the line: every aligned tooth takes its full
//!    mesio-distal width; the space missing between the fixed teeth (and,
//!    by default, between the last tooth of each side, which goes distally
//!    at most [`END_SLACK`]) is made by the therapy in the order of
//!    `priority`, after the spaces of the arch are used (no IPR on a contact
//!    open at rest);
//!    what is left is spread over the contacts of the stretch; with
//!    `close_spaces` the gaps close, the posterior teeth going mesially as
//!    far as needed (between two fixed teeth retracting the incisors up to
//!    `retract_max`);
//! 4. the positions along the arch: the nearest to the current ones,
//!    without overlaps (weighted isotonic regression per stretch between the
//!    fixed teeth);
//! 5. each tooth: derotated about its own long axis by the part of its
//!    rotation against the line beyond the tolerance of its zone (none when
//!    that part is under [`MIN_DEROTATION_DEG`]), moved with the cusps onto
//!    the line, and levelled (only the step beyond `level_tol`; by default
//!    the last tooth of each side keeps its height); tip and torque
//!    unchanged;
//! 6. with the crowns, the real 3D overlap between neighbours adds space
//!    where needed and a contact still open takes space away (at most 4
//!    rounds); when the space is not enough even with the therapy the teeth
//!    are derotated only as far as it allows (posterior teeth give up first,
//!    then the canines, the incisors last).
//!
//! Departures from the plugin, each with a [`Config`] flag the parity tests
//! turn on: the derotation of the excess only (`plugin_derotation`), the
//! scale of the arch line (`plugin_line_scale`) and the spaces
//! (`plugin_spaces`, 2026-11-24: the ends of the arch bounded, the last
//! tooth of each side at its height, no IPR on an open contact, the open
//! contacts closed on the crowns). The parameters
//! align every tooth and hold no molar by default ([`Params`]); the
//! plugin's default aligns the incisors and canines with the molars held.
//!
//! Coordinates: arch coordinates (origin between the last molars on the
//! occlusal plane, +Y anterior, +Z occlusal). Targets are deltas `D`: the
//! final pose of a tooth is `D * rest`.

mod crown;
mod line;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use dc_case::Fdi;
use dca_model::{
    SetupIprZone, SetupLeveling, SetupMidline, SetupScope, SetupSettings, SpacePriority,
};
use glam::{DMat3, DMat4, DVec2, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crown::{cusp_points, rest_crown};
pub use line::{Chain, Crowded, cusp_line, isotonic, line_distance, order_points, solve_chain};

use crate::anatomy::{self, Arch, Surface};
use crate::evidence;
use crate::frame::rot_axis_angle;
use crate::teeth::{ToothGeom, Which};
use line::{
    arc, clip, cross2, interior, interp, line_normals, local_quadratic, oriented, outward,
    point_at, project, tangent_at,
};

/// Total space of a stretch (mm) below which the incisors are not retracted
/// (imperfect contacts, curvature of the line).
pub const SPACE_TOL: f64 = 0.2;
/// A locked tooth farther than this from the line (mm) ...
const LOCK_WARN_MM: f64 = 0.3;
/// ... or rotated more than this (degrees) is reported.
const LOCK_WARN_DEG: f64 = 5.0;
/// Overlap at rest (mm) between the crowns of two teeth left in place that is
/// accepted as it is (scan, segmentation); beyond it is crowding.
const REST_OVERLAP_MAX: f64 = 0.5;
/// Beyond this (mm) the difference between the model and the crowns is not a
/// calibration error: the contact of the model stays.
const GAP_CAL_MAX: f64 = 2.0;
/// How far (mm) the last tooth of each side may go distally (the ends of the
/// arch are bounded, [`Config::plugin_spaces`] off): a crowding of the arch
/// up to twice this is taken there instead of by the therapy, so the noise
/// of the measured widths of the last molars (their derotation, a few
/// vertices of a contact given to the neighbour) does not turn into IPR.
pub const END_SLACK: f64 = 0.5;
/// Smallest derotation the setup makes (degrees). A tooth rotated against
/// the line beyond the tolerance of its zone (`rot_tol_deg`,
/// `post_rot_tol_deg`) is turned back only by the excess, so the turn grows
/// from nothing at the tolerance instead of jumping to the whole rotation; an
/// excess under this is not turned at all. 0.5 degrees is the smallest turn
/// the plugin's setup makes itself (a tooth left in place that follows the
/// curve is not turned below it; its staging gives any turn an aligner), a
/// quarter of the 2 degrees an aligner turns a tooth, and it moves the
/// contacts of an incisor (about 4 mm from its axis) by 0.035 mm, under the
/// 0.05 mm steps of the IPR and of the levelling. It is above the noise of
/// the measured rotation (about 0.3 degrees when a few vertices of a contact
/// change label).
pub const MIN_DEROTATION_DEG: f64 = 0.5;

// ---- options ------------------------------------------------------------------------------------------

/// Which teeth are aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignScope {
    /// Incisors and canines (premolars and molars stay as they are).
    #[default]
    Anterior,
    /// Incisors, canines and premolars.
    Premolars,
    /// Every tooth.
    All,
}

impl AlignScope {
    /// Last tooth index aligned.
    fn last(self) -> u8 {
        match self {
            AlignScope::Anterior => 3,
            AlignScope::Premolars => 5,
            AlignScope::All => 8,
        }
    }
}

/// Vertical levelling of the aligned teeth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Leveling {
    /// The height of the smoothed line of the neighbours.
    #[default]
    Smooth,
    /// The least-squares plane of the cusps.
    Flat,
    /// The current height.
    None,
}

/// Where the dental midline goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Midline {
    /// Where it is (the central incisors weigh more).
    #[default]
    Keep,
    /// To the centre of the arch.
    Center,
    /// Balanced space (every tooth weighs the same).
    Balance,
}

/// A way to make space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Therapy {
    /// Interproximal reduction.
    Ipr,
    /// Expansion of the arch.
    Exp,
    /// Advancement of the incisors.
    Pro,
}

/// Contacts where IPR is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IprZone {
    /// Nowhere.
    None,
    /// Between incisors and canines.
    Anterior,
    /// Up to the second premolars.
    #[default]
    AnteriorPremolars,
    /// Up to the second molars.
    All,
}

impl IprZone {
    fn contains(self, a: Fdi, b: Fdi) -> bool {
        let (ia, ib) = (anatomy::index(a), anatomy::index(b));
        match self {
            IprZone::None => false,
            IprZone::Anterior => ia <= 3 && ib <= 3,
            IprZone::AnteriorPremolars => ia <= 5 && ib <= 5,
            IprZone::All => ia <= 7 && ib <= 7,
        }
    }
}

/// Options of [`conservative_setup`] (`setup_conservativo.Config`; the
/// defaults are those of the dataclass, the command uses [`Params`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Smoothing of the line from the cusps (mm).
    pub sigma: f64,
    /// Move along the arch below which the tooth stays where it is (mm).
    pub pos_tol: f64,
    /// Distance from the line below which an anterior tooth is not moved
    /// onto it (mm).
    pub normal_tol: f64,
    /// Rotation an anterior tooth keeps (degrees): beyond it only the excess
    /// is derotated, and not under [`MIN_DEROTATION_DEG`] of excess.
    pub rot_tol_deg: f64,
    /// Largest derotation (degrees).
    pub max_derotation: f64,
    /// Teeth to align.
    pub align_scope: AlignScope,
    /// Vertical levelling.
    pub leveling: Leveling,
    /// Largest vertical correction (mm).
    pub vertical_max: f64,
    /// Premolars and molars: distance from the line below which they stay (mm).
    pub post_normal_tol: f64,
    /// Premolars and molars: rotation they keep (degrees); beyond it only the
    /// excess is derotated, and not under [`MIN_DEROTATION_DEG`] of excess.
    pub post_rot_tol_deg: f64,
    /// Vertical step tolerated (only the excess is corrected, mm).
    pub level_tol: f64,
    /// Level the canines too (normally longer than the other teeth).
    pub level_canines: bool,
    /// First and second molars held along the arch (anchorage).
    pub fix_molars: bool,
    /// Third molars held completely.
    pub fix_third: bool,
    /// Canines held along the arch.
    pub fix_canines: bool,
    /// The teeth held along the arch are still derotated and moved onto the
    /// line.
    pub align_fixed: bool,
    /// Overlap already present that is left as it is (mm).
    pub contact_tol: f64,
    /// Weight of the central incisors (midline).
    pub incisor_weight: f64,
    /// Midline.
    pub midline: Midline,
    /// Close the spaces between the teeth.
    pub close_spaces: bool,
    /// Order in which space is made.
    pub priority: Vec<Therapy>,
    /// Largest expansion per side at the molars (mm).
    pub expansion_posterior: f64,
    /// Largest expansion per side at the canines (mm).
    pub expansion_canine: f64,
    /// Largest advancement of the incisors (mm).
    pub advance_max: f64,
    /// Largest retraction of the incisors (closing spaces, mm).
    pub retract_max: f64,
    /// Contacts where IPR is allowed.
    pub ipr_zone: IprZone,
    /// Cap of the reduction per surface (mm).
    pub ipr_surface_cap: f64,
    /// Largest fraction of the enamel removed.
    pub enamel_fraction: f64,
    /// Derotate as the plugin: a tooth rotated beyond the tolerance of its
    /// zone turns fully onto the line. Off by default (2026-11-23): only the
    /// excess over the tolerance turns, none under [`MIN_DEROTATION_DEG`] of
    /// excess, so the turn grows from nothing at the tolerance. The parity
    /// tests with the plugin turn it on.
    pub plugin_derotation: bool,
    /// The plugin's scale of the robust arch line: 6 x the median residual of
    /// every tooth, taken again at every round ([`cusp_line`]). Off by default
    /// (2026-11-23): the scale is taken once, from the canines and premolars.
    /// The parity tests with the plugin turn it on.
    pub plugin_line_scale: bool,
    /// The plugin's handling of the spaces and of the ends of the arch: the
    /// last tooth of each side free to go distally ([`solve_chain`] without
    /// bounded ends) and levelled like the others, IPR on any contact of a
    /// crowded stretch, and the final contacts checked on the crowns for
    /// overlaps only. Off by default (2026-11-24): the last tooth of each
    /// side goes distally at most [`END_SLACK`] but mesially as far as the
    /// spaces need, and keeps its height (the levelling extrapolates the
    /// line of the cusps there: up to 1 mm on a real case, which pushed the
    /// last molar into its neighbour); no IPR on a contact open at rest (the
    /// space is used first), and a contact the crowns find still open in
    /// the final position is closed. The parity tests with the plugin turn
    /// it on.
    pub plugin_spaces: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sigma: 5.0,
            pos_tol: 0.1,
            normal_tol: 0.1,
            rot_tol_deg: 1.0,
            max_derotation: 35.0,
            align_scope: AlignScope::Anterior,
            leveling: Leveling::Smooth,
            vertical_max: 1.0,
            post_normal_tol: 0.3,
            post_rot_tol_deg: 5.0,
            level_tol: 0.5,
            level_canines: false,
            fix_molars: true,
            fix_third: true,
            fix_canines: false,
            align_fixed: true,
            contact_tol: 0.15,
            incisor_weight: 4.0,
            midline: Midline::Keep,
            close_spaces: true,
            priority: vec![Therapy::Ipr, Therapy::Exp, Therapy::Pro],
            expansion_posterior: 2.0,
            expansion_canine: 0.0,
            advance_max: 1.0,
            retract_max: 2.0,
            ipr_zone: IprZone::Anterior,
            ipr_surface_cap: 0.25,
            enamel_fraction: 0.5,
            plugin_derotation: false,
            plugin_line_scale: false,
            plugin_spaces: false,
        }
    }
}

impl Config {
    /// The rotation tolerance of the zone of `f` (radians): `rot_tol_deg`
    /// for incisors and canines, `post_rot_tol_deg` for premolars and molars.
    fn rot_tol(&self, f: Fdi) -> f64 {
        if anatomy::index(f) >= 4 {
            self.post_rot_tol_deg
        } else {
            self.rot_tol_deg
        }
        .to_radians()
    }

    /// Whether a tooth of `f` rotated `rot` (radians, at rest against the
    /// line) is to be derotated.
    fn rotated(&self, f: Fdi, rot: f64) -> bool {
        if self.plugin_derotation {
            rot > self.rot_tol(f)
        } else {
            derotation(rot, self.rot_tol(f)) != 0.0
        }
    }

    /// The turn (radians, signed) of a tooth of `f` rotated `psi` (radians,
    /// signed) against the line: the excess over the tolerance of its zone
    /// ([`derotation`]), or, as the plugin, all of it from `rot_tol_deg` on.
    fn turn(&self, f: Fdi, psi: f64) -> f64 {
        if !self.plugin_derotation {
            derotation(psi, self.rot_tol(f))
        } else if psi.abs() < self.rot_tol_deg.to_radians() {
            0.0
        } else {
            psi
        }
    }
}

// ---- inputs and results -------------------------------------------------------------------------------

/// The geometry of a tooth the setup works on (arch coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct Tooth {
    /// Mesio-distal width (mm).
    pub width_md: f64,
    /// Mesial contact point.
    pub contact_m: DVec3,
    /// Distal contact point.
    pub contact_d: DVec3,
    /// Mesio-distal axis (mesial is `e_md * md_sign`).
    pub e_md: DVec3,
    /// Sign that turns `e_md` mesial.
    pub md_sign: f64,
    /// Long axis, towards the occlusal side.
    pub e_ax: DVec3,
    /// The mesio-distal axis comes from the crown (incisal edge), not from
    /// the contacts of the segmentation.
    pub edge_md: bool,
}

/// What the setup reads of a measured tooth ([`ToothGeom`]), arch
/// coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ToothInput {
    /// Tooth number.
    pub fdi: Fdi,
    /// Centroid of the crown.
    pub centroid: DVec3,
    /// Mesio-distal axis.
    pub e_md: DVec3,
    /// Bucco-lingual axis (buccal positive).
    pub e_bl: DVec3,
    /// Long axis.
    pub e_ax: DVec3,
    /// Sign that turns `e_md` mesial.
    pub md_sign: f64,
    /// Crown reference point.
    pub reference: DVec3,
    /// Occlusal height.
    pub occ_z: f64,
    /// Gingival margin points.
    pub margin: Vec<DVec3>,
    /// Mesial contact point.
    pub contact_m: DVec3,
    /// Distal contact point.
    pub contact_d: DVec3,
    /// Mesio-distal width (mm).
    pub width_md: f64,
}

impl From<&ToothGeom> for ToothInput {
    fn from(t: &ToothGeom) -> Self {
        Self {
            fdi: t.fdi,
            centroid: t.centroid,
            e_md: t.e_md,
            e_bl: t.e_bl,
            e_ax: t.e_ax,
            md_sign: f64::from(t.md_sign),
            reference: t.reference,
            occ_z: t.occ_z,
            margin: t.margin.clone(),
            contact_m: t.contact_m,
            contact_d: t.contact_d,
            width_md: t.width_md,
        }
    }
}

impl ToothInput {
    /// The [`Tooth`] of the setup.
    pub fn tooth(&self) -> Tooth {
        Tooth {
            width_md: self.width_md,
            contact_m: self.contact_m,
            contact_d: self.contact_d,
            e_md: self.e_md,
            md_sign: self.md_sign,
            e_ax: self.e_ax,
            edge_md: false,
        }
    }
}

/// A tooth off the line before the setup.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Deviation {
    /// Tooth number.
    pub fdi: Fdi,
    /// Largest distance of its cusps from the line (mm).
    pub off_mm: f64,
    /// Rotation against the line (degrees).
    pub rot_deg: f64,
}

/// A rotation left on a tooth (degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LeftRotation {
    /// Tooth number.
    pub fdi: Fdi,
    /// Rotation left (degrees).
    pub deg: f64,
}

/// One entry of the setup log (the plugin's `log` lines, as data: the user
/// interface words them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Note {
    /// Teeth left out of the aligner: held where they are.
    OutOfAligner {
        /// The teeth.
        teeth: Vec<Fdi>,
    },
    /// The arch line: from the real cusps, or drawn by the clinician.
    ArchLine {
        /// Drawn by the clinician.
        drawn: bool,
    },
    /// The teeth that are aligned.
    Scope {
        /// The choice.
        scope: AlignScope,
    },
    /// A locked tooth out of the alignment: it stays and the neighbours
    /// arrange around it, with no IPR on its contacts; unlock it to align it.
    LockedOffLine {
        /// The tooth and how far it is off.
        tooth: Deviation,
    },
    /// Teeth locked in their final position (reference for the others).
    Locked {
        /// The teeth.
        teeth: Vec<Fdi>,
    },
    /// The evaluation before acting.
    Evaluation {
        /// Teeth to align.
        to_align: Vec<Deviation>,
        /// Teeth within the tolerances: they stay as they are.
        in_place: Vec<Fdi>,
    },
    /// Teeth held along the arch.
    Anchors {
        /// The teeth.
        teeth: Vec<Fdi>,
        /// Those derotated or moved onto the line in place.
        aligned_in_place: Vec<Fdi>,
    },
    /// Teeth that do not move at all.
    Held {
        /// The teeth.
        teeth: Vec<Fdi>,
    },
    /// How many teeth move and how far (largest cusp displacement).
    Moved {
        /// Teeth that move.
        count: usize,
        /// Teeth of the arch.
        of: usize,
        /// Largest displacement (mm).
        max_mm: f64,
        /// Mean displacement (mm).
        mean_mm: f64,
    },
    /// The arch is already aligned on the line.
    NothingToMove,
    /// Expansion per side.
    Expansion {
        /// Posterior sectors (mm).
        posterior_mm: f64,
        /// Canines (mm).
        canine_mm: f64,
    },
    /// Interproximal reduction.
    Ipr {
        /// Total (mm).
        total_mm: f64,
        /// Contacts.
        contacts: usize,
    },
    /// Incisors advanced (positive) or retracted (negative).
    Incisors {
        /// Advancement (mm).
        advance_mm: f64,
    },
    /// The midline moved along the arch.
    Midline {
        /// Shift (mm).
        shift_mm: f64,
    },
    /// Teeth held along the arch not fully derotated, not to hit the
    /// neighbours.
    AnchorsNotDerotated {
        /// The teeth and the rotation left.
        teeth: Vec<LeftRotation>,
    },
    /// Not enough space to derotate fully (no overlap beyond the IPR).
    PartialDerotation {
        /// The teeth and the rotation left.
        teeth: Vec<LeftRotation>,
    },
    /// Crowding not resolved, spread over the contacts.
    UnresolvedCrowding {
        /// Residual crowding (mm).
        residual_mm: f64,
        /// Largest overlap (mm).
        overlap_max_mm: f64,
    },
    /// Space left next to the fixed teeth.
    Spacing {
        /// The space (mm).
        mm: f64,
    },
}

/// Result of [`conservative_setup`].
#[derive(Debug, Clone, PartialEq)]
pub struct SetupResult {
    /// Target delta `D` of each tooth that moves (final pose `D * rest`).
    pub targets: BTreeMap<Fdi, DMat4>,
    /// IPR per contact (total over both surfaces, mm, steps of 0.05), keyed
    /// by the pair in arch order.
    pub ipr: BTreeMap<(Fdi, Fdi), f64>,
    /// The line the teeth were aligned on.
    pub line: Vec<DVec3>,
    /// Expansion per side at the molars (mm).
    pub expansion: f64,
    /// Expansion per side at the canines (mm).
    pub expansion_canine: f64,
    /// Incisor advancement (mm; negative: retracted).
    pub advance: f64,
    /// Crowding not resolved (mm).
    pub residual: f64,
    /// Space left between the fixed teeth (mm).
    pub spacing: f64,
    /// Largest overlap along the arch (mm).
    pub overlap_max: f64,
    /// Shift of the midline along the arch (mm).
    pub midline_shift: f64,
    /// Teeth that move.
    pub moved: Vec<Fdi>,
    /// Teeth locked by the clinician.
    pub held: Vec<Fdi>,
    /// The log.
    pub log: Vec<Note>,
    /// Calibration of each contact on the real crowns (mm).
    pub gap_cal: Vec<f64>,
    /// Gap of each contact along the line (mm).
    pub gap: Vec<f64>,
}

impl SetupResult {
    fn empty(held: Vec<Fdi>, log: Vec<Note>) -> Self {
        Self {
            targets: BTreeMap::new(),
            ipr: BTreeMap::new(),
            line: Vec::new(),
            expansion: 0.0,
            expansion_canine: 0.0,
            advance: 0.0,
            residual: 0.0,
            spacing: 0.0,
            overlap_max: 0.0,
            midline_shift: 0.0,
            moved: Vec::new(),
            held,
            log,
            gap_cal: Vec::new(),
            gap: Vec::new(),
        }
    }
}

// ---- small helpers ------------------------------------------------------------------------------------

fn mean(p: &[DVec3]) -> DVec3 {
    p.iter().copied().sum::<DVec3>() / p.len() as f64
}

fn unit(v: DVec3) -> DVec3 {
    v / v.length().max(1e-12)
}

fn rigid(r: DMat3, t: DVec3) -> DMat4 {
    DMat4::from_cols(
        r.x_axis.extend(0.0),
        r.y_axis.extend(0.0),
        r.z_axis.extend(0.0),
        t.extend(1.0),
    )
}

fn apply(d: &DMat4, p: DVec3) -> DVec3 {
    d.transform_point3(p)
}

/// The angle (radians, 0 to 90 degrees) between a plan direction `e` and the
/// unit line tangent `tg`, whatever their sense.
fn plan_angle(e: DVec2, tg: DVec2) -> f64 {
    (e.dot(tg).abs() / e.length().max(1e-12))
        .clamp(0.0, 1.0)
        .acos()
}

/// Teeth in contact in the arch (consecutive on one side, or the two central
/// incisors).
fn adjacent(a: Fdi, b: Fdi) -> bool {
    if anatomy::is_right(a) == anatomy::is_right(b) {
        return anatomy::index(a).abs_diff(anatomy::index(b)) == 1;
    }
    anatomy::index(a) == 1 && anatomy::index(b) == 1
}

/// Surface of tooth `a` that faces tooth `b` (across the midline always the
/// mesial one), `planning.facing_surface`.
pub fn facing_surface(a: Fdi, b: Fdi) -> Surface {
    if anatomy::is_right(a) != anatomy::is_right(b) {
        return Surface::Mesial;
    }
    if anatomy::index(b) < anatomy::index(a) {
        Surface::Mesial
    } else {
        Surface::Distal
    }
}

fn ipr_cap(a: Fdi, b: Fdi, cfg: &Config) -> f64 {
    if !cfg.ipr_zone.contains(a, b) {
        return 0.0;
    }
    let cap = |f: Fdi, g: Fdi| {
        anatomy::max_ipr_surface(
            f,
            facing_surface(f, g),
            cfg.ipr_surface_cap,
            cfg.enamel_fraction,
        )
    };
    cap(a, b) + cap(b, a)
}

/// The homologous tooth of the other side (16 <-> 26, 46 <-> 36).
fn homologous(f: Fdi) -> Option<Fdi> {
    let q = match f.get() / 10 {
        1 => 2,
        2 => 1,
        3 => 4,
        4 => 3,
        5 => 6,
        6 => 5,
        7 => 8,
        8 => 7,
        q => q,
    };
    Fdi::new(q * 10 + f.get() % 10).ok()
}

/// Mesio-distal direction (3D) of a tooth: the one from the crown
/// (`front_axes`), otherwise the chord between its extreme cusps (incisal
/// edge, buccal cusps of the molars) when they are far apart, otherwise the
/// axis of the tooth.
fn md3(g: &Tooth, c: &[DVec3]) -> DVec3 {
    let mut e3 = g.e_md * g.md_sign;
    if c.len() >= 2 && !g.edge_md {
        let dc = c[c.len() - 1] - c[0];
        if dc.truncate().length() >= 1.5 {
            e3 = if dc.truncate().dot(e3.truncate()) >= 0.0 {
                dc
            } else {
                -dc
            };
        }
    }
    e3
}

/// Least-squares plane `z = a x + b y + c` of the points.
fn plane(c: &[DVec3]) -> [f64; 3] {
    let a = nalgebra::DMatrix::from_fn(c.len(), 3, |r, k| match k {
        0 => c[r].x,
        1 => c[r].y,
        _ => 1.0,
    });
    let b = nalgebra::DVector::from_iterator(c.len(), c.iter().map(|p| p.z));
    let svd = a.svd(true, true);
    let smax = svd.singular_values.max();
    let eps = f64::EPSILON * c.len().max(3) as f64 * smax;
    match svd.solve(&b, eps) {
        Ok(x) => [x[0], x[1], x[2]],
        Err(_) => [0.0, 0.0, mean(c).z],
    }
}

/// The value nearest to `lo` in `[lo, hi]` for which `ok` holds (`ok`
/// monotone); `hi` when none.
fn bisect(mut ok: impl FnMut(f64) -> bool, mut lo: f64, mut hi: f64, steps: usize) -> f64 {
    if ok(lo) {
        return lo;
    }
    if !ok(hi) {
        return hi;
    }
    for _ in 0..steps {
        let mid = 0.5 * (lo + hi);
        if ok(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Angle about the axis `ax` of the tooth that leaves the direction `e3` at
/// the plan angle `keep` from `m` (parallel for 0; secant method from the
/// plan angle `psi` until the plan angle is within `tol` rad, within 1.2
/// times the limit); `psi` when it does not converge. The plugin stops
/// within 1e-5 rad; the port runs to the root, which left the turn of a
/// tooth kept at its tolerance moving by up to that with the round-off of
/// its stored frame.
fn plan_turn(ax: DVec3, e3: DVec3, m: DVec2, psi: f64, keep: f64, lim: f64, tol: f64) -> f64 {
    let left = |a: f64| {
        let v = (rot_axis_angle(ax, a) * e3).truncate();
        cross2(v, m).atan2(v.dot(m)) - keep
    };
    let (mut a0, mut r0) = (0.0, left(0.0));
    let (mut a1, mut r1) = (psi, left(psi));
    for _ in 0..8 {
        if r1.abs() < tol || (r1 - r0).abs() < 1e-7 * tol {
            break;
        }
        let a2 = a1 - r1 * (a1 - a0) / (r1 - r0);
        (a0, r0, a1) = (a1, r1, a2);
        r1 = left(a1);
    }
    if r1.abs() < left(psi).abs() && a1.abs() <= 1.2 * lim {
        a1
    } else {
        psi
    }
}

/// The derotation (radians) of a tooth rotated `psi` (radians, signed)
/// against the line, with the tolerance `tol` (radians) of its zone: the
/// excess over the tolerance, with the sign of `psi`, and none when the
/// excess is under [`MIN_DEROTATION_DEG`]. It never exceeds the excess and
/// grows with `|psi|`.
fn derotation(psi: f64, tol: f64) -> f64 {
    let excess = psi.abs() - tol;
    if excess >= MIN_DEROTATION_DEG.to_radians() {
        excess.copysign(psi)
    } else {
        0.0
    }
}

/// Arc length where the line crosses the symmetry axis of the arch (from the
/// midpoints of the homologous right / left teeth from the canine on).
fn midline_s(l: &[DVec3], sl: &[f64], pc: &[DVec3], seq: &[Fdi]) -> Option<f64> {
    let (mut mids, mut dirs) = (Vec::new(), Vec::new());
    for (i, &f) in seq.iter().enumerate() {
        let Some(g) = homologous(f) else { continue };
        if let Some(j) = seq.iter().position(|&x| x == g)
            && anatomy::is_right(f)
            && anatomy::index(f) >= 3
        {
            mids.push(0.5 * (pc[i].truncate() + pc[j].truncate()));
            dirs.push(pc[j].truncate() - pc[i].truncate());
        }
    }
    if mids.is_empty() {
        return None;
    }
    let m = mids.iter().copied().sum::<DVec2>() / mids.len() as f64;
    let d = dirs.iter().copied().sum::<DVec2>() / dirs.len() as f64;
    let d = d / d.length().max(1e-12);
    let dist: Vec<f64> = l.iter().map(|p| (p.truncate() - m).dot(d)).collect();
    let sign = |x: f64| {
        if x > 0.0 {
            1
        } else if x < 0.0 {
            -1
        } else {
            0
        }
    };
    let ks: Vec<usize> = (0..dist.len().saturating_sub(1))
        .filter(|&k| sign(dist[k]) != sign(dist[k + 1]))
        .collect();
    let mut j = *ks.first()?;
    for &k in &ks {
        if l[k].y > l[j].y {
            j = k;
        }
    }
    let t = dist[j] / (dist[j] - dist[j + 1]);
    Some(sl[j] + t * (sl[j + 1] - sl[j]))
}

/// The teeth and cusps the final position works on: incisors and canines
/// with the axes from the crown (`front_axes`) and the locked teeth in the
/// position set by the clinician.
#[allow(clippy::type_complexity)]
fn prepare(
    teeth: &BTreeMap<Fdi, Tooth>,
    cusps: &BTreeMap<Fdi, Vec<DVec3>>,
    crowns: Option<&BTreeMap<Fdi, Vec<DVec3>>>,
    poses: &BTreeMap<Fdi, DMat4>,
) -> (
    BTreeMap<Fdi, Tooth>,
    BTreeMap<Fdi, Vec<DVec3>>,
    Option<BTreeMap<Fdi, Vec<DVec3>>>,
    BTreeMap<Fdi, DMat4>,
) {
    let (mut teeth, mut cusps) = (teeth.clone(), cusps.clone());
    let mut crowns = crowns.cloned();
    if let Some(cr) = &crowns {
        crown::front_axes(&mut teeth, &mut cusps, cr);
    }
    let poses: BTreeMap<Fdi, DMat4> = poses
        .iter()
        .filter(|(f, _)| teeth.contains_key(f))
        .map(|(f, d)| (*f, *d))
        .collect();
    for (f, d) in &poses {
        let r = DMat3::from_mat4(*d);
        let g = &teeth[f];
        let moved = Tooth {
            width_md: g.width_md,
            md_sign: g.md_sign,
            contact_m: apply(d, g.contact_m),
            contact_d: apply(d, g.contact_d),
            e_md: r * g.e_md,
            e_ax: r * g.e_ax,
            edge_md: false,
        };
        teeth.insert(*f, moved);
        if let Some(c) = cusps.get_mut(f) {
            c.iter_mut().for_each(|p| *p = apply(d, *p));
        }
        if let Some(c) = crowns.as_mut().and_then(|cr| cr.get_mut(f)) {
            c.iter_mut().for_each(|p| *p = apply(d, *p));
        }
    }
    (teeth, cusps, crowns, poses)
}

/// The automatic arch line of the final position before the therapy: from
/// the cusps of the prepared teeth, through the locked teeth.
pub fn base_line(
    teeth: &BTreeMap<Fdi, Tooth>,
    seq: &[Fdi],
    cusps: &BTreeMap<Fdi, Vec<DVec3>>,
    sigma: f64,
    crowns: Option<&BTreeMap<Fdi, Vec<DVec3>>>,
    poses: &BTreeMap<Fdi, DMat4>,
) -> Vec<DVec3> {
    let (teeth, cusps, _, poses) = prepare(teeth, cusps, crowns, poses);
    let seq: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| teeth.contains_key(f) && cusps.contains_key(f))
        .collect();
    let Some(first) = seq.first() else {
        return Vec::new();
    };
    let anchors: Vec<Fdi> = poses.keys().copied().collect();
    oriented(
        &cusp_line(&cusps, &seq, sigma, &anchors, false),
        mean(&cusps[first]),
    )
}

/// Evaluation of each tooth against the line at rest: (largest distance of
/// its cusps from the line in mm, rotation against the line in degrees, off
/// the line, rotated) with the tolerances of its zone.
fn evaluate(
    teeth: &[Tooth],
    seq: &[Fdi],
    c: &[Vec<DVec3>],
    l: &[DVec3],
    cfg: &Config,
) -> Vec<(f64, f64, bool, bool)> {
    let sl = arc(l);
    seq.iter()
        .enumerate()
        .map(|(i, &f)| {
            let post = anatomy::index(f) >= 4;
            let dev = line_distance(l, &c[i])
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            let e = md3(&teeth[i], &c[i]).truncate();
            let tg = tangent_at(l, &sl, project(l, &sl, mean(&c[i])).0);
            let rot = plan_angle(e, tg).to_degrees();
            (
                dev,
                rot,
                dev > if post {
                    cfg.post_normal_tol
                } else {
                    cfg.normal_tol
                },
                cfg.rotated(f, rot.to_radians()),
            )
        })
        .collect()
}

/// Correction of the space of each contact (mm) from the crowns in the final
/// position: positive (extra space needed) where they overlap more than at
/// rest beyond IPR + `tol`; with `close` (per contact: to be closed, a tooth
/// of the pair free along the arch), negative (space to take away) where
/// they are still apart beyond `tol` from their contact at rest (the
/// overlap at rest, or touching when they were apart) less the IPR. The
/// widths of the model come from the crown extents along the line, so a
/// contact it closes, of a tooth derotated or slid along the curve, can be
/// left open on the crowns by some tenths of a millimetre.
fn overlaps_3d(
    seq: &[Fdi],
    adj: &[bool],
    crowns: &BTreeMap<Fdi, Vec<DVec3>>,
    targets: &BTreeMap<Fdi, DMat4>,
    ipr: &[f64],
    tol: f64,
    close: Option<&[bool]>,
) -> Vec<f64> {
    let mut err = vec![0.0; seq.len().saturating_sub(1)];
    for (i, w) in seq.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let (Some(pa), Some(pb)) = (crowns.get(&a), crowns.get(&b)) else {
            continue;
        };
        if !adj[i] || (!targets.contains_key(&a) && !targets.contains_key(&b)) {
            continue;
        }
        let rest = crown::facing_clearance(pa, pb);
        let moved = |f: Fdi, p: &[DVec3]| -> Vec<DVec3> {
            match targets.get(&f) {
                Some(d) => p.iter().map(|&q| apply(d, q)).collect(),
                None => p.to_vec(),
            }
        };
        let sep = crown::facing_clearance(&moved(a, pa), &moved(b, pb));
        let floor = if rest.is_finite() { rest.min(0.0) } else { 0.0 };
        if sep.is_finite() && sep < floor - (ipr[i] + tol) {
            err[i] = floor - sep - ipr[i] - 0.5 * tol;
        } else if close.is_some_and(|c| c[i]) && sep.is_finite() && sep > floor - ipr[i] + tol {
            err[i] = floor - ipr[i] - sep + 0.5 * tol;
        }
    }
    err
}

// ---- the solver ---------------------------------------------------------------------------------------

/// Positions and widths of the teeth along a line.
struct Measure {
    sl: Vec<f64>,
    s0: Vec<f64>,
    n0: Vec<f64>,
    nc: Vec<f64>,
    wm: Vec<f64>,
    wp: Vec<f64>,
}

/// One solution of the positions along a line.
#[derive(Clone)]
struct Solved {
    sl: Vec<f64>,
    s0: Vec<f64>,
    n0: Vec<f64>,
    nc: Vec<f64>,
    wm: Vec<f64>,
    wp: Vec<f64>,
    c: Vec<f64>,
    deficit: f64,
    segs: Vec<Crowded>,
    gap: Vec<f64>,
    spacing: f64,
    ipr: Vec<f64>,
    k: Vec<f64>,
}

/// Everything fixed during one setup.
struct Solver<'a> {
    cfg: &'a Config,
    seq: Vec<Fdi>,
    n: usize,
    teeth: Vec<Tooth>,
    c: Vec<Vec<DVec3>>,
    pc: Vec<DVec3>,
    interior: DVec2,
    fx: Vec<bool>,
    aligned: Vec<bool>,
    need_off: Vec<bool>,
    need_rot: Vec<bool>,
    wts: Vec<f64>,
    keep: Vec<bool>,
    base: Vec<DVec3>,
    nrm_b: Vec<DVec2>,
    exp_prof: Vec<f64>,
    pro_prof: Vec<f64>,
    ext_pts: Vec<Option<Vec<DVec2>>>,
    gap_cal: Vec<f64>,
    rest_pair: Vec<bool>,
    n_base: Vec<f64>,
}

impl Solver<'_> {
    /// The line moved outwards by the expansion `xe` (fraction of the
    /// largest) and the advancement `xp` (mm at the incisors).
    fn line_of(&self, xe: f64, xp: f64) -> Vec<DVec3> {
        self.base
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let off = xe * self.exp_prof[k] + xp * self.pro_prof[k];
                *p + (off * self.nrm_b[k]).extend(0.0)
            })
            .collect()
    }

    fn measure(&self, l: &[DVec3], k: Option<&[f64]>, asis: bool) -> Measure {
        let n = self.n;
        let sl = arc(l);
        let (mut s0, mut n0, mut nc, mut wm, mut wp) = (
            vec![0.0; n],
            vec![0.0; n],
            vec![0.0; n],
            vec![0.0; n],
            vec![0.0; n],
        );
        for i in 0..n {
            let (s, q) = project(l, &sl, self.pc[i]);
            let tg = tangent_at(l, &sl, s);
            s0[i] = s;
            n0[i] = (self.pc[i].truncate() - q).dot(outward(tg, self.interior, q));
            // mean offset of the cusps themselves from the line (the midpoint of the incisal
            // edge of an incisor falls inside the curve even with the cusps on the line)
            let sd: Vec<f64> = self.c[i]
                .iter()
                .map(|&cp| {
                    let (sc, qc) = project(l, &sl, cp);
                    (cp.truncate() - qc).dot(outward(tangent_at(l, &sl, sc), self.interior, qc))
                })
                .collect();
            nc[i] = sd.iter().sum::<f64>() / sd.len() as f64;
            let g = &self.teeth[i];
            let (cm, cd) = (g.contact_m, g.contact_d);
            // the space it takes now along the line: the extent of the crown along the
            // tangent (with the crowns), otherwise between its contacts
            let pq = self.ext_pts[i].as_deref();
            let mut w = match pq {
                Some(pq) => crown::extent(pq, tg),
                None => (cm - cd).truncate().dot(tg).abs(),
            }
            .max(1.0);
            let ends = [project(l, &sl, cm).0, project(l, &sl, cd).0];
            let (emin, emax) = (ends[0].min(ends[1]), ends[0].max(ends[1]));
            let span = emax - emin;
            let frac = if span > 1e-6 {
                clip((s - emin) / span, 0.2, 0.8)
            } else {
                0.5
            };
            if self.aligned[i] && self.need_rot[i] && !asis {
                // derotated by the excess over its tolerance: more only by what that turn adds
                let e = md3(g, &self.c[i]).truncate();
                let phi = plan_angle(e, tg).min(self.cfg.max_derotation.to_radians());
                let turn = if self.cfg.plugin_derotation {
                    phi
                } else {
                    derotation(phi, self.cfg.rot_tol(self.seq[i]))
                };
                let kf = k.map_or(1.0, |k| k[i]);
                match pq {
                    Some(pq) => {
                        if phi > 1e-12 {
                            let we = crown::extent(pq, e / e.length().max(1e-12)).max(1.0);
                            w += kf * turn / phi * (we - w);
                        }
                    }
                    None => w = w * (phi - kf * turn).cos() / phi.cos().max(0.2),
                }
            }
            wm[i] = frac * w;
            wp[i] = (1.0 - frac) * w;
        }
        Measure {
            sl,
            s0,
            n0,
            nc,
            wm,
            wp,
        }
    }

    fn solve(&self, l: &[DVec3], ipr: &[f64], extra: &[f64], k: Option<&[f64]>) -> Solved {
        let (n, cfg) = (self.n, self.cfg);
        let m = self.measure(l, k, false);
        let gap: Vec<f64> = (0..n - 1)
            .map(|i| (m.s0[i + 1] - m.wm[i + 1]) - (m.s0[i] + m.wp[i]) + self.gap_cal[i])
            .collect();
        let mut req = vec![0.0; n - 1];
        for i in 0..n - 1 {
            // an overlap within contact_tol stays as it is; between two teeth the
            // evaluation leaves in place the contact measured on the crowns stays up to
            // REST_OVERLAP_MAX
            let allow = if -gap[i] <= cfg.contact_tol || self.rest_pair[i] {
                (-gap[i]).max(0.0)
            } else {
                0.0
            };
            req[i] = m.wp[i] + m.wm[i + 1] - ipr[i] - allow - self.gap_cal[i] + extra[i];
            // a space within contact_tol is a closed contact (widths measured as chords)
            if gap[i] > 0.0 && gap[i] <= cfg.contact_tol {
                req[i] += gap[i];
            }
            if self.keep[i] {
                req[i] = req[i].max(m.s0[i + 1] - m.s0[i]);
            }
        }
        let mut u0 = m.s0.clone();
        let mut w = self.wts.clone();
        if cfg.midline == Midline::Center {
            let sm = midline_s(l, &m.sl, &self.pc, &self.seq);
            let cen = (0..n - 1).find(|&i| {
                anatomy::index(self.seq[i]) == 1 && anatomy::index(self.seq[i + 1]) == 1
            });
            if let (Some(sm), Some(j)) = (sm, cen) {
                u0[j] = sm - m.wp[j];
                u0[j + 1] = sm + m.wm[j + 1];
                w[j] = 25.0 * cfg.incisor_weight;
                w[j + 1] = 25.0 * cfg.incisor_weight;
            }
        }
        let cumsum = |r: &[f64]| -> Vec<f64> {
            let mut out = vec![0.0];
            let mut acc = 0.0;
            for x in r {
                acc += x;
                out.push(acc);
            }
            out
        };
        let sub =
            |a: &[f64], b: &[f64]| -> Vec<f64> { a.iter().zip(b).map(|(x, y)| x - y).collect() };
        let mut off = cumsum(&req);
        let ends = (!cfg.plugin_spaces).then_some(END_SLACK);
        let ch = solve_chain(&sub(&u0, &off), &w, &self.fx, cfg.close_spaces, ends);
        let (deficit, segs) = (ch.deficit, ch.segs.clone());
        let (mut u, mut spacing) = (ch.u, ch.spacing);
        if deficit > 1e-9 {
            // crowding left: spread over the contacts of the stretch (never onto the fixed teeth)
            let mut req2 = req.clone();
            for sg in &segs {
                let lo = sg.left.max(0) as usize;
                let hi = sg.right.min(n as i64 - 1) as usize;
                let sel: Vec<usize> = (lo..hi).filter(|&i| !self.keep[i]).collect();
                let tot: f64 = sel.iter().map(|&i| req2[i]).sum();
                if tot > 1e-9 {
                    for &i in &sel {
                        req2[i] -= sg.deficit * req2[i] / tot;
                    }
                }
            }
            off = cumsum(&req2);
            let ch2 = solve_chain(&sub(&u0, &off), &w, &self.fx, cfg.close_spaces, ends);
            u = ch2.u;
            spacing = ch2.spacing;
        }
        let c = u.iter().zip(&off).map(|(a, b)| a + b).collect();
        Solved {
            sl: m.sl,
            s0: m.s0,
            n0: m.n0,
            nc: m.nc,
            wm: m.wm,
            wp: m.wp,
            c,
            deficit,
            segs,
            gap,
            spacing,
            ipr: ipr.to_vec(),
            k: k.map_or_else(|| vec![1.0; n], <[f64]>::to_vec),
        }
    }

    fn free(&self, l: &[DVec3], ipr: &[f64], extra: &[f64]) -> bool {
        self.solve(l, ipr, extra, None).deficit <= 1e-6
    }

    fn grow_ipr(&self, l: &[DVec3], ipr: &[f64], extra: &[f64], caps: &[f64]) -> Vec<f64> {
        let n = self.n;
        let mut ipr = ipr.to_vec();
        let mut r = self.solve(l, &ipr, extra, None);
        for _ in 0..6 {
            if r.deficit <= 1e-6 {
                break;
            }
            let mut grew = false;
            for sg in &r.segs {
                let lo = sg.left.max(0) as usize;
                let hi = sg.right.min(n as i64 - 1) as usize;
                let mut room = vec![0.0; n - 1];
                for i in lo..hi {
                    room[i] = (caps[i] - ipr[i]).max(0.0);
                }
                let total: f64 = room.iter().sum();
                if total <= 1e-9 {
                    continue;
                }
                let f = (sg.deficit / total).min(1.0);
                for i in 0..n - 1 {
                    ipr[i] += room[i] * f;
                }
                grew = true;
            }
            if !grew {
                break;
            }
            r = self.solve(l, &ipr, extra, None);
        }
        ipr
    }

    /// Height of the line of the cusps at `s_t` from the neighbouring teeth
    /// only (without the tooth itself and the `skip` teeth).
    fn level_z(
        &self,
        l: &[DVec3],
        sl: &[f64],
        i_self: usize,
        s_t: f64,
        skip: &[bool],
    ) -> Option<f64> {
        let (mut s, mut z) = (Vec::new(), Vec::new());
        for (g, (cg, &sk)) in self.c.iter().zip(skip).enumerate() {
            if g == i_self || sk {
                continue;
            }
            for &cp in cg {
                s.push(project(l, sl, cp).0);
                z.push([cp.z]);
            }
        }
        if s.len() < 3 {
            return None;
        }
        let ones = vec![1.0; s.len()];
        Some(local_quadratic(&s, &z, &[s_t], self.cfg.sigma, &ones)[0][0])
    }

    /// The final pose of every aligned tooth that moves.
    fn targets(
        &self,
        r: &Solved,
        l: &[DVec3],
        rest_rot: &mut BTreeMap<Fdi, f64>,
    ) -> BTreeMap<Fdi, DMat4> {
        let cfg = self.cfg;
        let sl = &r.sl;
        let skip: Vec<bool> = self
            .seq
            .iter()
            .map(|&f| !cfg.level_canines && anatomy::index(f) == 3)
            .collect();
        let plane_abc = (cfg.leveling == Leveling::Flat).then(|| {
            let mut pts: Vec<DVec3> = (0..self.n)
                .filter(|&i| !skip[i])
                .flat_map(|i| self.c[i].clone())
                .collect();
            if pts.is_empty() {
                pts = self.c.iter().flatten().copied().collect();
            }
            plane(&pts)
        });
        let lim = cfg.max_derotation.to_radians();
        let mut out = BTreeMap::new();
        for i in 0..self.n {
            if !self.aligned[i] {
                continue;
            }
            let f = self.seq[i];
            let ds = if self.fx[i] { 0.0 } else { r.c[i] - r.s0[i] };
            let s_t = if self.fx[i] { r.s0[i] } else { r.c[i] };
            let p = point_at(l, sl, s_t);
            let tg = tangent_at(l, sl, s_t);
            let nrm = outward(tg, self.interior, p.truncate());
            // only the tooth the evaluation found off the line goes with its cusps onto it;
            // the others keep their offset (and follow the line when it moves)
            let snap = r.nc[i].abs() > cfg.normal_tol && self.need_off[i];
            let n_t = if snap {
                r.n0[i] - r.nc[i]
            } else {
                self.n_base[i]
            };
            let z0 = self.pc[i].z;
            let mut z = z0;
            // the last tooth of each side keeps its height: the line of the cusps of its
            // neighbours is extrapolated there
            let end = !cfg.plugin_spaces && (i == 0 || i + 1 == self.n);
            if !self.fx[i] && !skip[i] && !end {
                match cfg.leveling {
                    Leveling::Smooth => {
                        z = self.level_z(l, sl, i, s_t, &skip).unwrap_or(p.z);
                    }
                    Leveling::Flat => {
                        let [a, b, c] = plane_abc.unwrap_or([0.0, 0.0, z0]);
                        z = a * p.x + b * p.y + c;
                    }
                    Leveling::None => {}
                }
                // only the step beyond the tolerance: the tooth to the edge of the tolerance
                let dev = z0 - z;
                z = if dev.abs() <= cfg.level_tol {
                    z0
                } else {
                    z + dev.signum() * cfg.level_tol
                };
                z = clip(z, z0 - cfg.vertical_max, z0 + cfg.vertical_max);
                if (z - z0).abs() < 0.05 {
                    z = z0;
                }
            }
            let mut target = DVec3::new(p.x + n_t * nrm.x, p.y + n_t * nrm.y, z);
            let e3 = md3(&self.teeth[i], &self.c[i]);
            let e = e3.truncate() / e3.truncate().length().max(1e-12);
            let m = if tg.dot(e) >= 0.0 { tg } else { -tg };
            let mut psi = cross2(e, m).atan2(e.dot(m));
            // the plan angle the tooth keeps against the line: its tolerance when derotated
            let mut keep = 0.0;
            if !self.need_rot[i] {
                // in place: not derotated, but it follows the curve when it slides along
                // the arch or the line turns (expansion)
                let t0 = tangent_at(l, sl, r.s0[i]);
                psi = cross2(t0, tg).atan2(t0.dot(tg));
                if psi.to_degrees().abs() < 0.5 {
                    psi = 0.0;
                }
            } else {
                // only the excess over the tolerance of its zone, none under the smallest turn
                let turn = cfg.turn(f, psi);
                keep = psi - turn;
                psi = turn;
            }
            psi = clip(psi, -lim, lim);
            let still = ds.abs() < cfg.pos_tol && (n_t - r.n0[i]).abs() < cfg.pos_tol && !snap;
            if still && psi == 0.0 && z == z0 {
                continue;
            }
            if still {
                // only derotation or levelling: no move in plan
                target.x = self.pc[i].x;
                target.y = self.pc[i].y;
            }
            let ax = unit(self.teeth[i].e_ax);
            if psi != 0.0 {
                let tol = if cfg.plugin_derotation { 1e-5 } else { 1e-12 };
                psi = plan_turn(ax, e3, m, psi, keep, lim, tol);
                let kf = r.k[i];
                if kf < 1.0 {
                    let left = (1.0 - kf) * psi.abs();
                    if left >= 1f64.to_radians() {
                        rest_rot.insert(f, left.to_degrees());
                    }
                    psi *= kf;
                }
            }
            let rot = if psi != 0.0 {
                rot_axis_angle(ax, psi)
            } else {
                DMat3::IDENTITY
            };
            out.insert(f, rigid(rot, target - rot * self.pc[i]));
        }
        out
    }
}

/// Final position (see the module documentation): every aligned tooth on the
/// arch line.
///
/// `teeth` and `cusps` (`{fdi: (k, 3)}` at rest) in arch coordinates;
/// `line` the line drawn by the clinician (`None`: from the cusps); `fixed`
/// teeth that do not move at all (out of the aligner); `crowns` crown points
/// at rest for the 3D check of the contacts (optional); `poses` teeth locked
/// by the clinician in their final position (arch-coordinate deltas, the
/// identity too): they stay, they are the reference (the line from the
/// cusps passes through them) and the other teeth arrange around them.
#[allow(clippy::too_many_arguments)]
pub fn conservative_setup(
    teeth: &BTreeMap<Fdi, Tooth>,
    seq: &[Fdi],
    cusps: &BTreeMap<Fdi, Vec<DVec3>>,
    cfg: &Config,
    line: Option<&[DVec3]>,
    fixed: &[Fdi],
    crowns: Option<&BTreeMap<Fdi, Vec<DVec3>>>,
    poses: &BTreeMap<Fdi, DMat4>,
) -> SetupResult {
    let (teeth, cusps, crowns, poses) = prepare(teeth, cusps, crowns, poses);
    let seq: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| teeth.contains_key(f) && cusps.contains_key(f))
        .collect();
    let n = seq.len();
    let mut log = Vec::new();
    if n < 2 {
        return SetupResult::empty(poses.keys().copied().collect(), log);
    }
    let c: Vec<Vec<DVec3>> = seq.iter().map(|f| cusps[f].clone()).collect();
    let pc: Vec<DVec3> = c.iter().map(|x| mean(x)).collect();
    let interior = interior(&cusps, &seq);
    let anchors_locked: Vec<Fdi> = poses.keys().copied().collect();
    let base = match line {
        None => cusp_line(
            &cusps,
            &seq,
            cfg.sigma,
            &anchors_locked,
            cfg.plugin_line_scale,
        ),
        Some(l) => oriented(l, pc[0]),
    };
    let base = oriented(&base, pc[0]);
    log.push(Note::ArchLine {
        drawn: line.is_some(),
    });

    let mut held: BTreeSet<Fdi> = fixed.iter().copied().chain(poses.keys().copied()).collect();
    let mut anchor = held.clone();
    for &f in &seq {
        let i = anatomy::index(f);
        if cfg.fix_third && i == 8 {
            held.insert(f);
            anchor.insert(f);
        }
        if (cfg.fix_molars && (i == 6 || i == 7)) || (cfg.fix_canines && i == 3) {
            anchor.insert(f);
        }
    }
    let last = cfg.align_scope.last();
    let beyond: BTreeSet<Fdi> = seq
        .iter()
        .copied()
        .filter(|&f| anatomy::index(f) > last)
        .collect();
    held.extend(&beyond);
    anchor.extend(&beyond);
    log.push(Note::Scope {
        scope: cfg.align_scope,
    });
    let teeth_i: Vec<Tooth> = seq.iter().map(|f| teeth[f].clone()).collect();
    // a locked tooth out of line: compared with the line of the other teeth (the one through
    // it would always find it in line)
    if !poses.is_empty() {
        let free_line = cusp_line(&cusps, &seq, cfg.sigma, &[], cfg.plugin_line_scale);
        let sb0 = arc(&free_line);
        for f in poses.keys() {
            let Some(i) = seq.iter().position(|x| x == f) else {
                continue;
            };
            let s_f = project(&free_line, &sb0, pc[i]).0;
            let off = line_distance(&free_line, &c[i])
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            let e = md3(&teeth_i[i], &c[i]).truncate();
            let tg = tangent_at(&free_line, &sb0, s_f);
            let rot = plan_angle(e, tg).to_degrees();
            if off > LOCK_WARN_MM || rot > LOCK_WARN_DEG {
                log.push(Note::LockedOffLine {
                    tooth: Deviation {
                        fdi: *f,
                        off_mm: off,
                        rot_deg: rot,
                    },
                });
            }
        }
        log.push(Note::Locked {
            teeth: poses.keys().copied().collect(),
        });
    }
    let fx: Vec<bool> = seq.iter().map(|f| anchor.contains(f)).collect();
    let aligned: Vec<bool> = seq
        .iter()
        .map(|f| !held.contains(f) && (!anchor.contains(f) || cfg.align_fixed))
        .collect();
    // evaluation before acting: only the tooth that needs it is aligned
    let ev = evaluate(&teeth_i, &seq, &c, &base, cfg);
    let need_off: Vec<bool> = (0..n).map(|i| aligned[i] && ev[i].2).collect();
    let need_rot: Vec<bool> = (0..n).map(|i| aligned[i] && ev[i].3).collect();
    log.push(Note::Evaluation {
        to_align: (0..n)
            .filter(|&i| need_off[i] || need_rot[i])
            .map(|i| Deviation {
                fdi: seq[i],
                off_mm: ev[i].0,
                rot_deg: ev[i].1,
            })
            .collect(),
        in_place: (0..n)
            .filter(|&i| aligned[i] && !(need_off[i] || need_rot[i]))
            .map(|i| seq[i])
            .collect(),
    });
    let w_inc = if cfg.midline == Midline::Keep {
        cfg.incisor_weight
    } else {
        1.0
    };
    let wts: Vec<f64> = seq
        .iter()
        .map(|&f| if anatomy::index(f) == 1 { w_inc } else { 1.0 })
        .collect();
    let adj: Vec<bool> = seq.windows(2).map(|w| adjacent(w[0], w[1])).collect();
    let keep: Vec<bool> = adj.iter().map(|a| !a).collect();
    // no IPR towards a tooth out of the aligner or locked; a tooth that stays only because it
    // is beyond the teeth to align can be reduced
    let no_ipr: BTreeSet<Fdi> = held.difference(&beyond).copied().collect();
    let mut caps: Vec<f64> = seq
        .windows(2)
        .zip(&adj)
        .map(|(w, &ad)| {
            if ad && !no_ipr.contains(&w[0]) && !no_ipr.contains(&w[1]) {
                ipr_cap(w[0], w[1], cfg)
            } else {
                0.0
            }
        })
        .collect();

    // profiles along the line: expansion (molars and canines, nothing at the incisors) and
    // advancement (incisors)
    let sb = arc(&base);
    let mut pos: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            (
                project(&base, &sb, pc[i]).0,
                f64::from(anatomy::index(seq[i])),
            )
        })
        .collect();
    pos.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let (ps, pk): (Vec<f64>, Vec<f64>) = pos.into_iter().unzip();
    let kv: Vec<f64> = sb.iter().map(|&s| interp(s, &ps, &pk)).collect();
    let nrm_b = line_normals(&base, interior);
    // posterior expansion only when the molars are aligned too
    let exp_post = if last >= 6 {
        cfg.expansion_posterior
    } else {
        0.0
    };
    let exp_prof: Vec<f64> = kv
        .iter()
        .map(|&k| {
            interp(
                k,
                &[1.0, 3.0, 6.0, 8.0],
                &[0.0, cfg.expansion_canine, exp_post, exp_post],
            )
        })
        .collect();
    let pro_prof: Vec<f64> = kv
        .iter()
        .map(|&k| interp(k, &[1.0, 2.0, 3.0, 4.0], &[1.0, 1.0, 0.5, 0.0]))
        .collect();

    // crown points above 30% of the height (as the contacts): its extent along a direction
    let ext_pts: Vec<Option<Vec<DVec2>>> = (0..n)
        .map(|i| {
            let p = crowns.as_ref()?.get(&seq[i])?;
            if p.len() < 20 {
                return None;
            }
            let ax = unit(teeth_i[i].e_ax);
            let h: Vec<f64> = p.iter().map(|q| q.dot(ax)).collect();
            let (hmin, hmax) = h
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                    (a.min(x), b.max(x))
                });
            Some(
                p.iter()
                    .zip(&h)
                    .filter(|(_, hh)| **hh >= hmin + 0.3 * (hmax - hmin))
                    .map(|(q, _)| q.truncate())
                    .collect(),
            )
        })
        .collect();

    let mut sv = Solver {
        cfg,
        seq: seq.clone(),
        n,
        teeth: teeth_i,
        c: c.clone(),
        pc: pc.clone(),
        interior,
        fx: fx.clone(),
        aligned: aligned.clone(),
        need_off: need_off.clone(),
        need_rot: need_rot.clone(),
        wts,
        keep,
        base,
        nrm_b,
        exp_prof,
        pro_prof,
        ext_pts,
        gap_cal: vec![0.0; n - 1],
        rest_pair: vec![false; n - 1],
        n_base: Vec::new(),
    };

    // calibration of the contacts on the real crowns: at rest the gap between two neighbours
    // along the line differing from the one measured in 3D between the crowns is an error
    // of the model, removed at every contact
    let mut measured_3d = vec![false; n - 1];
    if let Some(cr) = &crowns {
        let mr = sv.measure(&sv.base, None, true);
        for (i, w) in seq.windows(2).enumerate() {
            let gap_r = (mr.s0[i + 1] - mr.wm[i + 1]) - (mr.s0[i] + mr.wp[i]);
            if let (true, Some(pa), Some(pb)) = (adj[i], cr.get(&w[0]), cr.get(&w[1])) {
                let g3 = crown::facing_clearance(pa, pb);
                if g3.is_finite() && (g3 - gap_r).abs() <= GAP_CAL_MAX {
                    sv.gap_cal[i] = g3 - gap_r;
                    measured_3d[i] = g3 >= -REST_OVERLAP_MAX;
                }
            }
        }
    }
    // no IPR on a contact open at rest: its space is used before any enamel is removed
    if !cfg.plugin_spaces {
        let mr = sv.measure(&sv.base, None, true);
        for (i, cap) in caps.iter_mut().enumerate() {
            let gap = (mr.s0[i + 1] - mr.wm[i + 1]) - (mr.s0[i] + mr.wp[i]) + sv.gap_cal[i];
            if gap > cfg.contact_tol {
                *cap = 0.0;
            }
        }
    }
    // the distance of each tooth from the line at rest: a tooth in place keeps it when the
    // line moves (expansion, advancement)
    sv.n_base = sv.measure(&sv.base, None, true).n0;
    let moving: Vec<bool> = (0..n).map(|i| need_off[i] || need_rot[i]).collect();
    sv.rest_pair = (0..n - 1)
        .map(|i| !(moving[i] || moving[i + 1]) && measured_3d[i])
        .collect();
    let sv = sv;

    let drawn = line.is_some();
    // advancement and retraction only with an incisor to align, expansion with a canine or a
    // posterior tooth
    let can_pro = !drawn && (0..n).any(|i| aligned[i] && anatomy::index(seq[i]) <= 2);
    let can_exp = !drawn && (0..n).any(|i| aligned[i] && anatomy::index(seq[i]) >= 3);

    let mut extra = vec![0.0; n - 1];
    let mut xe;
    let mut xp;
    let mut r;
    let mut l;
    let mut targets;
    let mut rest_rot: BTreeMap<Fdi, f64>;
    let mut calib = 0;
    loop {
        calib += 1;
        // the space first, in the order of the therapy and only as much as needed
        xe = 0.0;
        xp = 0.0;
        let mut ipr = vec![0.0; n - 1];
        for &m in &cfg.priority {
            if sv.free(&sv.line_of(xe, xp), &ipr, &extra) {
                break;
            }
            match m {
                Therapy::Exp => {
                    if exp_post.max(cfg.expansion_canine) > 0.0 && can_exp {
                        xe = bisect(|v| sv.free(&sv.line_of(v, xp), &ipr, &extra), 0.0, 1.0, 24);
                    }
                }
                Therapy::Ipr => ipr = sv.grow_ipr(&sv.line_of(xe, xp), &ipr, &extra, &caps),
                Therapy::Pro => {
                    if cfg.advance_max > 0.0 && can_pro {
                        xp = bisect(
                            |v| sv.free(&sv.line_of(xe, v), &ipr, &extra),
                            0.0,
                            cfg.advance_max,
                            24,
                        );
                    }
                }
            }
        }
        r = sv.solve(&sv.line_of(xe, xp), &ipr, &extra, None);
        if r.deficit <= 1e-6
            && cfg.close_spaces
            && cfg.retract_max > 0.0
            && xp <= 0.0
            && can_pro
            && r.spacing > SPACE_TOL
        {
            // spaces left between the fixed teeth: closed retracting the incisors (at most
            // retract_max), never as far as making crowding
            let closed = |v: f64| {
                let rr = sv.solve(&sv.line_of(xe, -v), &ipr, &extra, None);
                rr.spacing <= SPACE_TOL || rr.deficit > 1e-6
            };
            let mut v = bisect(closed, 0.0, cfg.retract_max, 24);
            while v > 0.0 && !sv.free(&sv.line_of(xe, -v), &ipr, &extra) {
                v = (v - 0.02).max(0.0);
            }
            xp = -v;
            r = sv.solve(&sv.line_of(xe, xp), &ipr, &extra, None);
        }
        l = sv.line_of(xe, xp);
        if r.deficit > 1e-6 {
            // space not enough even with the therapy: in every crowded stretch the teeth are
            // derotated only as far as the space allows
            let mut k = vec![1.0; n];
            for sg in r.segs.clone() {
                let lo = (sg.left + 1).max(0) as usize;
                let hi = sg.right.min(n as i64) as usize;
                let free_i: Vec<usize> = (lo..hi).filter(|&i| aligned[i] && need_rot[i]).collect();
                let fits = |v: f64, idx: &[usize], k: &[f64]| {
                    let mut kk = k.to_vec();
                    for &i in idx {
                        kk[i] = 1.0 - v;
                    }
                    let rr = sv.solve(&l, &ipr, &extra, Some(&kk));
                    !rr.segs
                        .iter()
                        .any(|s| s.left == sg.left && s.right == sg.right && s.deficit > 1e-6)
                };
                // derotation is given up first on the posterior teeth, then the canines, the
                // incisors last
                let groups = [
                    free_i
                        .iter()
                        .copied()
                        .filter(|&i| anatomy::index(seq[i]) >= 4)
                        .collect::<Vec<_>>(),
                    free_i
                        .iter()
                        .copied()
                        .filter(|&i| anatomy::index(seq[i]) == 3)
                        .collect(),
                    free_i
                        .iter()
                        .copied()
                        .filter(|&i| anatomy::index(seq[i]) <= 2)
                        .collect(),
                ];
                for group in groups {
                    if group.is_empty() {
                        continue;
                    }
                    let v = bisect(|v| fits(v, &group, &k), 0.0, 1.0, 16);
                    for &i in &group {
                        k[i] = 1.0 - v;
                    }
                    if v < 1.0 || fits(1.0, &group, &k) {
                        break;
                    }
                }
            }
            r = sv.solve(&l, &ipr, &extra, Some(&k));
        }
        rest_rot = BTreeMap::new();
        targets = sv.targets(&r, &l, &mut rest_rot);
        let Some(cr) = &crowns else { break };
        // a tooth held along the arch (molar) derotated in place must not hit the neighbours
        // (they cannot make room): its derotation shrinks until the crowns no longer
        // interpenetrate beyond the IPR
        for i in 0..n {
            if !(fx[i] && targets.contains_key(&seq[i]) && need_rot[i]) {
                continue;
            }
            let nb: Vec<usize> = [i.wrapping_sub(1), i + 1]
                .into_iter()
                .filter(|&j| j < n && adj[i.min(j)])
                .collect();
            let clear = |kk: f64, r: &mut Solved| -> bool {
                r.k[i] = kk;
                if nb.is_empty() {
                    return true;
                }
                let tt = sv.targets(r, &l, &mut BTreeMap::new());
                let err = overlaps_3d(&seq, &adj, cr, &tt, &r.ipr, cfg.contact_tol, None);
                !nb.iter().any(|&j| err[i.min(j)] != 0.0)
            };
            let k0 = r.k[i];
            if !clear(k0, &mut r) {
                let v = {
                    let rr = &mut r;
                    bisect(|v| clear(1.0 - v, rr), 0.0, 1.0, 12)
                };
                r.k[i] = 1.0 - v;
                rest_rot = BTreeMap::new();
                targets = sv.targets(&r, &l, &mut rest_rot);
            }
        }
        // the contacts to close on the crowns: both teeth free along the arch (next to a tooth
        // held along the arch a space is what its stretch leaves, `spacing`), the spaces
        // closed (the plugin checks the overlaps only)
        let close: Vec<bool> = (0..n - 1)
            .map(|i| cfg.close_spaces && !cfg.plugin_spaces && !fx[i] && !fx[i + 1])
            .collect();
        let err = overlaps_3d(
            &seq,
            &adj,
            cr,
            &targets,
            &r.ipr,
            cfg.contact_tol,
            Some(&close),
        );
        if err.iter().map(|e| e.abs()).fold(0.0, f64::max) <= 1e-3 || calib >= 4 {
            break;
        }
        for (e, x) in extra.iter_mut().zip(&err) {
            *e += x;
        }
    }

    let mut ipr_mm = BTreeMap::new();
    for (i, w) in seq.windows(2).enumerate() {
        if r.ipr[i] > 1e-6 {
            // to 0.05 mm upwards, downwards when upwards would exceed the limit of the contact
            let mut v = (r.ipr[i] / 0.05 - 1e-6).ceil() * 0.05;
            if v > caps[i] + 1e-9 {
                v = (caps[i] / 0.05 + 1e-6).floor() * 0.05;
            }
            if v > 1e-9 {
                ipr_mm.insert((w[0], w[1]), (v * 1000.0).round() / 1000.0);
            }
        }
    }
    let mut overlap_max: f64 = 0.0;
    let mut any_moving = false;
    for i in 0..n - 1 {
        if adj[i] && !(fx[i] && fx[i + 1]) {
            let ov = (r.c[i] + r.wp[i]) - (r.c[i + 1] - r.wm[i + 1]) - r.ipr[i];
            overlap_max = if any_moving { overlap_max.max(ov) } else { ov };
            any_moving = true;
        }
    }
    let overlap_max = if any_moving {
        overlap_max.max(0.0)
    } else {
        0.0
    };
    let cen: Vec<usize> = (0..n).filter(|&i| anatomy::index(seq[i]) == 1).collect();
    let mid_shift = if cen.is_empty() {
        0.0
    } else {
        cen.iter().map(|&i| r.c[i] - r.s0[i]).sum::<f64>() / cen.len() as f64
    };
    let moved: Vec<Fdi> = targets.keys().copied().collect();
    let disp: Vec<f64> = moved
        .iter()
        .map(|f| {
            let i = seq.iter().position(|x| x == f).unwrap_or(0);
            c[i].iter()
                .map(|&p| (apply(&targets[f], p) - p).length())
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();
    let still: Vec<Fdi> = seq
        .iter()
        .copied()
        .filter(|f| anchor.contains(f) && !held.contains(f))
        .collect();
    log.push(Note::Anchors {
        aligned_in_place: still
            .iter()
            .copied()
            .filter(|f| targets.contains_key(f))
            .collect(),
        teeth: still,
    });
    let held_only: Vec<Fdi> = held
        .iter()
        .copied()
        .filter(|f| !poses.contains_key(f) && !beyond.contains(f))
        .collect();
    if !held_only.is_empty() {
        log.push(Note::Held { teeth: held_only });
    }
    if moved.is_empty() {
        log.push(Note::NothingToMove);
    } else {
        log.push(Note::Moved {
            count: moved.len(),
            of: n,
            max_mm: disp.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            mean_mm: disp.iter().sum::<f64>() / disp.len() as f64,
        });
    }
    if xe > 0.0 {
        log.push(Note::Expansion {
            posterior_mm: xe * exp_post,
            canine_mm: xe * cfg.expansion_canine,
        });
    }
    if !ipr_mm.is_empty() {
        log.push(Note::Ipr {
            total_mm: ipr_mm.values().sum(),
            contacts: ipr_mm.len(),
        });
    }
    if xp.abs() > 1e-3 {
        log.push(Note::Incisors { advance_mm: xp });
    }
    if mid_shift.abs() >= 0.1 {
        log.push(Note::Midline {
            shift_mm: mid_shift,
        });
    }
    let left_rot = |in_anchor: bool| -> Vec<LeftRotation> {
        seq.iter()
            .filter_map(|f| {
                let deg = rest_rot.get(f).copied().unwrap_or(0.0);
                (deg >= 1.0 && anchor.contains(f) == in_anchor)
                    .then_some(LeftRotation { fdi: *f, deg })
            })
            .collect()
    };
    let (part, stuck) = (left_rot(false), left_rot(true));
    if !stuck.is_empty() {
        log.push(Note::AnchorsNotDerotated { teeth: stuck });
    }
    if !part.is_empty() {
        log.push(Note::PartialDerotation { teeth: part });
    }
    if r.deficit > 1e-6 {
        log.push(Note::UnresolvedCrowding {
            residual_mm: r.deficit,
            overlap_max_mm: overlap_max,
        });
    }
    if r.spacing > SPACE_TOL && cfg.close_spaces {
        log.push(Note::Spacing { mm: r.spacing });
    }
    SetupResult {
        targets,
        ipr: ipr_mm,
        line: l,
        expansion: xe * exp_post,
        expansion_canine: xe * cfg.expansion_canine,
        advance: xp,
        residual: r.deficit,
        spacing: r.spacing,
        overlap_max,
        midline_shift: mid_shift,
        moved,
        held: poses.keys().copied().collect(),
        log,
        gap_cal: sv.gap_cal.clone(),
        gap: r.gap,
    }
}

// ---- the stage ----------------------------------------------------------------------------------------

/// Parameters of the setup stage (the scene settings of the plugin,
/// `props.OA_Settings`, read by `case_plan.conservative_config`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Arches to work on.
    pub arch: Which,
    /// Teeth to align. Every tooth by default (2026-11-24; the plugin's
    /// default is the incisors and canines): the final position corrects
    /// the whole arch, the premolars and molars too.
    pub align_scope: AlignScope,
    /// Smoothing of the arch line from the cusps (mm).
    pub line_sigma_mm: f64,
    /// Move along the arch below which a tooth stays where it is (mm).
    pub pos_tol_mm: f64,
    /// Incisors and canines: distance from the line below which they stay (mm).
    pub normal_tol_mm: f64,
    /// Incisors and canines: rotation they keep (degrees); beyond it only the
    /// excess is derotated ([`MIN_DEROTATION_DEG`]).
    pub rot_tol_deg: f64,
    /// Premolars and molars: distance from the line below which they stay (mm).
    pub post_normal_tol_mm: f64,
    /// Premolars and molars: rotation they keep (degrees); beyond it only the
    /// excess is derotated ([`MIN_DEROTATION_DEG`]).
    pub post_rot_tol_deg: f64,
    /// Largest derotation (degrees).
    pub max_derotation_deg: f64,
    /// Vertical levelling.
    pub leveling: Leveling,
    /// Largest vertical correction (mm).
    pub vertical_max_mm: f64,
    /// Vertical step tolerated (only the excess is corrected, mm).
    pub level_tol_mm: f64,
    /// Level the canines too.
    pub level_canines: bool,
    /// First and second molars held along the arch (anchorage). Off by
    /// default (2026-11-24; on in the plugin): the molars go mesially as far
    /// as the spaces need, never distally.
    pub fix_molars: bool,
    /// Third molars held completely.
    pub fix_third: bool,
    /// Canines held along the arch.
    pub fix_canines: bool,
    /// Teeth held along the arch are still derotated and moved onto the line.
    pub align_fixed: bool,
    /// Overlap already present left as it is (mm).
    pub contact_tol_mm: f64,
    /// Weight of the central incisors (keeps the midline).
    pub incisor_weight: f64,
    /// Midline.
    pub midline: Midline,
    /// Close the spaces between the teeth.
    pub close_spaces: bool,
    /// Order in which space is made.
    pub priority: Vec<Therapy>,
    /// Largest expansion per side of the upper arch (molars and canines, mm).
    pub expansion_upper_mm: f64,
    /// Largest expansion per side of the lower posterior sectors (mm).
    pub expansion_lower_posterior_mm: f64,
    /// Largest expansion per side of the lower canines (mm).
    pub expansion_lower_canine_mm: f64,
    /// Largest advancement of the incisors (mm).
    pub advance_max_mm: f64,
    /// Largest retraction of the incisors to close spaces (mm).
    pub retract_max_mm: f64,
    /// Contacts where IPR is allowed.
    pub ipr_zone: IprZone,
    /// Cap of the reduction per surface (mm).
    pub ipr_surface_cap_mm: f64,
    /// Largest fraction of the enamel removed.
    pub enamel_fraction: f64,
    /// Teeth left out of the aligner (FDI numbers): they stay where they are.
    pub excluded: Vec<u8>,
    /// Arch line drawn by the clinician (points in case coordinates, mm);
    /// needs `arch` upper or lower. Empty: from the cusps.
    pub line: Vec<[f64; 3]>,
}

impl Default for Params {
    fn default() -> Self {
        let ev = |key: &str, fallback: f64| evidence::param_f64(key).unwrap_or(fallback);
        Self {
            arch: Which::All,
            align_scope: AlignScope::All,
            line_sigma_mm: ev("cons_line_sigma", 5.0),
            pos_tol_mm: ev("cons_pos_tol", 0.1),
            normal_tol_mm: ev("cons_normal_tol", 0.1),
            rot_tol_deg: ev("cons_rot_tol", 1.0),
            post_normal_tol_mm: ev("cons_post_normal_tol", 0.3),
            post_rot_tol_deg: ev("cons_post_rot_tol", 5.0),
            max_derotation_deg: 35.0,
            leveling: Leveling::Smooth,
            vertical_max_mm: ev("vertical_auto_max", 1.0),
            level_tol_mm: ev("cons_level_tol", 0.5),
            level_canines: false,
            fix_molars: false,
            fix_third: true,
            fix_canines: false,
            align_fixed: true,
            contact_tol_mm: ev("cons_contact_tol", 0.15),
            incisor_weight: ev("cons_incisor_weight", 4.0),
            midline: Midline::Keep,
            close_spaces: true,
            priority: vec![Therapy::Ipr, Therapy::Exp, Therapy::Pro],
            expansion_upper_mm: ev("expansion_upper", 2.0),
            expansion_lower_posterior_mm: ev("expansion_lower_posterior", 2.0),
            expansion_lower_canine_mm: ev("expansion_lower_canine", 0.0),
            advance_max_mm: ev("advance_incisors", 1.0),
            retract_max_mm: ev("retract_incisors", 2.0),
            ipr_zone: IprZone::AnteriorPremolars,
            ipr_surface_cap_mm: ev("ipr_surface", 0.25),
            enamel_fraction: ev("ipr_enamel_fraction", 0.5),
            excluded: Vec::new(),
            line: Vec::new(),
        }
    }
}

impl Params {
    /// The drawn arch line as points (case coordinates).
    pub fn line_points(&self) -> Vec<DVec3> {
        self.line.iter().map(|&p| DVec3::from(p)).collect()
    }

    /// The [`Config`] of one arch (`case_plan.conservative_config`: the
    /// upper arch uses `expansion_upper_mm` at the molars and the canines).
    pub fn config(&self, arch: Arch) -> Config {
        let upper = arch.is_upper();
        Config {
            sigma: self.line_sigma_mm,
            pos_tol: self.pos_tol_mm,
            normal_tol: self.normal_tol_mm,
            rot_tol_deg: self.rot_tol_deg,
            max_derotation: self.max_derotation_deg,
            align_scope: self.align_scope,
            leveling: self.leveling,
            vertical_max: self.vertical_max_mm,
            post_normal_tol: self.post_normal_tol_mm,
            post_rot_tol_deg: self.post_rot_tol_deg,
            level_tol: self.level_tol_mm,
            level_canines: self.level_canines,
            fix_molars: self.fix_molars,
            fix_third: self.fix_third,
            fix_canines: self.fix_canines,
            align_fixed: self.align_fixed,
            contact_tol: self.contact_tol_mm,
            incisor_weight: self.incisor_weight,
            midline: self.midline,
            close_spaces: self.close_spaces,
            priority: self.priority.clone(),
            expansion_posterior: if upper {
                self.expansion_upper_mm
            } else {
                self.expansion_lower_posterior_mm
            },
            expansion_canine: if upper {
                self.expansion_upper_mm
            } else {
                self.expansion_lower_canine_mm
            },
            advance_max: self.advance_max_mm,
            retract_max: self.retract_max_mm,
            ipr_zone: self.ipr_zone,
            ipr_surface_cap: self.ipr_surface_cap_mm,
            enamel_fraction: self.enamel_fraction,
            plugin_derotation: false,
            plugin_line_scale: false,
            plugin_spaces: false,
        }
    }

    /// The parameters the case settings give (`case_plan.conservative_config`
    /// reads the scene settings): every arch, no tooth excluded, no drawn
    /// line; the derotation limit and the canines held along the arch are
    /// not settings of the plugin, they keep their defaults.
    pub fn from_settings(s: &SetupSettings) -> Self {
        Self {
            align_scope: s.align_scope.into(),
            line_sigma_mm: s.line_sigma_mm,
            pos_tol_mm: s.pos_tol_mm,
            normal_tol_mm: s.normal_tol_mm,
            rot_tol_deg: s.rot_tol_deg,
            post_normal_tol_mm: s.post_normal_tol_mm,
            post_rot_tol_deg: s.post_rot_tol_deg,
            leveling: s.leveling.into(),
            vertical_max_mm: s.vertical_max_mm,
            level_tol_mm: s.level_tol_mm,
            level_canines: s.level_canines,
            fix_molars: s.fix_molars,
            fix_third: s.fix_third,
            align_fixed: s.align_fixed,
            contact_tol_mm: s.contact_tol_mm,
            incisor_weight: s.incisor_weight,
            midline: s.midline.into(),
            close_spaces: s.close_spaces,
            priority: priority(s.priority),
            expansion_upper_mm: s.expansion_upper_mm,
            expansion_lower_posterior_mm: s.expansion_lower_posterior_mm,
            expansion_lower_canine_mm: s.expansion_lower_canine_mm,
            advance_max_mm: s.advance_max_mm,
            retract_max_mm: s.retract_max_mm,
            ipr_zone: s.ipr_zone.into(),
            ipr_surface_cap_mm: s.ipr_surface_cap_mm,
            enamel_fraction: s.enamel_fraction,
            ..Self::default()
        }
    }
}

/// The order of the therapies of a priority (`case_plan.PRIORITY`).
fn priority(p: SpacePriority) -> Vec<Therapy> {
    use Therapy::{Exp, Ipr, Pro};
    match p {
        SpacePriority::ExpIprPro => vec![Exp, Ipr, Pro],
        SpacePriority::IprExpPro => vec![Ipr, Exp, Pro],
        SpacePriority::ExpProIpr => vec![Exp, Pro, Ipr],
    }
}

impl From<SetupScope> for AlignScope {
    fn from(s: SetupScope) -> Self {
        match s {
            SetupScope::Anterior => AlignScope::Anterior,
            SetupScope::Premolars => AlignScope::Premolars,
            SetupScope::All => AlignScope::All,
        }
    }
}

impl From<SetupLeveling> for Leveling {
    fn from(l: SetupLeveling) -> Self {
        match l {
            SetupLeveling::Smooth => Leveling::Smooth,
            SetupLeveling::Flat => Leveling::Flat,
            SetupLeveling::None => Leveling::None,
        }
    }
}

impl From<SetupMidline> for Midline {
    fn from(m: SetupMidline) -> Self {
        match m {
            SetupMidline::Keep => Midline::Keep,
            SetupMidline::Center => Midline::Center,
            SetupMidline::Balance => Midline::Balance,
        }
    }
}

impl From<SetupIprZone> for IprZone {
    fn from(z: SetupIprZone) -> Self {
        match z {
            SetupIprZone::None => IprZone::None,
            SetupIprZone::Anterior => IprZone::Anterior,
            SetupIprZone::AnteriorPremolars => IprZone::AnteriorPremolars,
            SetupIprZone::All => IprZone::All,
        }
    }
}

/// Error of the setup stage.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// The input is inconsistent.
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// Input of the setup stage: one measured arch in arch coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The arch.
    pub arch: Arch,
    /// Scan vertices, arch coordinates.
    pub vertices: &'a [DVec3],
    /// Tooth label of each vertex (FDI, 0 gingiva).
    pub labels: &'a [u16],
    /// The teeth in arch order.
    pub sequence: &'a [Fdi],
    /// The geometry of each tooth.
    pub teeth: &'a BTreeMap<Fdi, ToothInput>,
    /// Arch line drawn by the clinician (arch coordinates), if any.
    pub line: Option<&'a [DVec3]>,
    /// Teeth left out of the aligner.
    pub fixed: &'a [Fdi],
    /// Teeth locked by the clinician: their final delta (arch coordinates).
    pub poses: &'a BTreeMap<Fdi, DMat4>,
}

/// Output of the setup stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The final position.
    pub result: SetupResult,
    /// The line the teeth were aligned on, 96 points evenly spaced (empty
    /// for fewer than 4 points), as the plugin stores it.
    pub line: Vec<DVec3>,
}

/// `n` points evenly spaced along the polyline (3D arc length).
pub fn resample(p: &[DVec3], n: usize) -> Vec<DVec3> {
    if p.is_empty() {
        return Vec::new();
    }
    let mut s = vec![0.0];
    for w in p.windows(2) {
        s.push(s[s.len() - 1] + (w[1] - w[0]).length());
    }
    let total = s[s.len() - 1];
    (0..n)
        .map(|i| {
            let u = if n > 1 {
                total * i as f64 / (n - 1) as f64
            } else {
                0.0
            };
            let col = |k: usize| -> f64 {
                let fp: Vec<f64> = p.iter().map(|q| q[k]).collect();
                interp(u, &s, &fp)
            };
            DVec3::new(col(0), col(1), col(2))
        })
        .collect()
}

/// Runs the setup of one arch: the cusps and the crowns from the labelled
/// scan, then [`conservative_setup`] with the configuration of the arch.
pub fn run(input: &Input<'_>, params: &Params) -> Result<Output, SetupError> {
    run_with(input, &params.config(input.arch))
}

/// [`run`] with the configuration `cfg` (the parity tests run it with the
/// plugin's derotation, [`Config::plugin_derotation`]).
pub fn run_with(input: &Input<'_>, cfg: &Config) -> Result<Output, SetupError> {
    if input.labels.len() != input.vertices.len() {
        return Err(SetupError::Invalid(format!(
            "{} labels for {} vertices",
            input.labels.len(),
            input.vertices.len()
        )));
    }
    let mut pts: BTreeMap<u16, Vec<DVec3>> = BTreeMap::new();
    for (p, &lab) in input.vertices.iter().zip(input.labels) {
        if lab != 0 {
            pts.entry(lab).or_default().push(*p);
        }
    }
    let (mut teeth, mut cusps, mut crowns) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for f in input.sequence {
        let Some(t) = input.teeth.get(f) else {
            continue;
        };
        let p = pts.get(&u16::from(f.get())).map_or(&[][..], Vec::as_slice);
        teeth.insert(*f, t.tooth());
        cusps.insert(*f, cusp_points(t, p));
        crowns.insert(*f, rest_crown(t, p));
    }
    let mut result = conservative_setup(
        &teeth,
        input.sequence,
        &cusps,
        cfg,
        input.line,
        input.fixed,
        Some(&crowns),
        input.poses,
    );
    let out: Vec<Fdi> = input
        .sequence
        .iter()
        .copied()
        .filter(|f| input.fixed.contains(f))
        .collect();
    if !out.is_empty() {
        result.log.insert(0, Note::OutOfAligner { teeth: out });
    }
    let line = if result.line.len() >= 4 {
        resample(&result.line, 96)
    } else {
        Vec::new()
    };
    Ok(Output { result, line })
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// A signed rotation from {0, ±0.2, ±0.9, ±1.2, ±1.5, ±1.7, ±4.9, ±5.6, ±12, ±35}
    /// degrees, in radians: none, under and at the tolerances, just under and over the
    /// smallest turn beyond them, and large.
    fn any_rotation() -> f64 {
        let deg: f64 = match kani::any::<u8>() % 10 {
            0 => 0.0,
            1 => 0.2,
            2 => 0.9,
            3 => 1.2,
            4 => 1.5,
            5 => 1.7,
            6 => 4.9,
            7 => 5.6,
            8 => 12.0,
            _ => 35.0,
        };
        let sign = if kani::any() { -1.0 } else { 1.0 };
        (sign * deg).to_radians()
    }

    /// A tolerance from {0, 1, 5} degrees, in radians.
    fn any_tolerance() -> f64 {
        match kani::any::<u8>() % 3 {
            0 => 0.0,
            1 => 1.0f64.to_radians(),
            _ => 5.0f64.to_radians(),
        }
    }

    // The derotation of a tooth never turns it by more than the excess of its rotation over
    // the tolerance, never past the line (the same sign as the rotation, or none), and not at
    // all when the excess is under the smallest turn MIN_DEROTATION_DEG.
    #[kani::proof]
    fn derotation_never_exceeds_the_excess() {
        let (psi, tol) = (any_rotation(), any_tolerance());
        let d = derotation(psi, tol);
        let excess = psi.abs() - tol;
        assert!(d.abs() <= excess.max(0.0));
        assert!(d == 0.0 || d.signum() == psi.signum());
        if excess < MIN_DEROTATION_DEG.to_radians() {
            assert!(d == 0.0);
        }
    }

    // The derotation grows with the rotation: a tooth rotated more (the same way) is never
    // turned less.
    #[kani::proof]
    fn derotation_grows_with_the_rotation() {
        let (a, b, tol) = (any_rotation(), any_rotation(), any_tolerance());
        kani::assume(a.abs() <= b.abs() && a.signum() == b.signum());
        assert!(derotation(a, tol).abs() <= derotation(b, tol).abs());
    }
}
