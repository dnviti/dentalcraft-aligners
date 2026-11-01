//! Identification stamp: arch and number (and date, case code) engraved on
//! the inside wall of every aligner, or embossed on the model the sheet is
//! thermoformed on, placed on a tooth of the stage.
//!
//! Reference: `ortho_aligner/stamp.py` (label, tooth frame, face map, search
//! of the spot, projection on the surface) and `ortho_aligner/case_stamp.py`
//! (characters, boolean operation).
//!
//! # Content
//!
//! The text comes from a template (default
//! `"{arch} {number} {passive}\n{date}\n{code}"`): `{arch}` is `SUP` / `INF`
//! (never a single letter: `I` and `1` look alike), `{number}` the aligner
//! number on two digits, `{passive}` a `P` for a passive aligner, `{date}` the
//! date in one of three formats (`DDMMYY`, `DD.MM.YY`, `YYMMDD`) and `{code}`
//! the case code (at most 8 of `A-Z 0-9 -`). Empty lines are dropped. When
//! the full text does not fit, the code is dropped, then the first two lines
//! are put on one line.
//!
//! # Place
//!
//! The face of the chosen tooth (lingual / palatal by default, first right
//! molar, then the other molars and the second premolars) where the surface
//! is the one of the crown, at least 0.8 mm from the cusps and 1 mm from the
//! contacts, with the wall thick enough (shell) or in the occlusal half of
//! the face (model). The text is tilted towards the occlusal side by 0, 10,
//! 20 and 30 degrees until it fits. A spot given by hand (`Placement::Manual`)
//! skips the search.
//!
//! # Deviation from the plugin
//!
//! The plugin builds the characters with a Blender text curve, projects them
//! on the surface and cuts them with a Blender boolean (Manifold, then
//! Exact). Here the characters are the outlines of a bundled font
//! (B612 Mono Bold subset, SIL OFL 1.1, `assets/`), filled with
//! `lyon_tessellation` and set by their ink to the width of the plugin's
//! label (`text`); the stamp is the signed distance field of the ink
//! (2D, to the outline) extruded along the surface normals. There is no mesh
//! boolean. Two ways to apply that field:
//!
//! * [`Method::Local`] (default): the triangles of the surface under the text
//!   are refined by longest-edge bisection to `resolution_mm`, and the
//!   vertices are moved by `depth` times a smooth ramp of the field (0.5 at
//!   the outline), all along the rays the text was projected with (the
//!   patch is a height field along them and stays one: the text cannot fold
//!   the surface). The surface keeps its topology (a closed
//!   manifold mesh stays so, an open one keeps its boundary), its labels and
//!   its triangles away from the text, and it costs a few milliseconds.
//! * [`Method::Sdf`]: the solid of the text (prisms along the surface) is
//!   sampled as a signed distance field on the voxel grid of the target,
//!   subtracted from it (engraving) or added to it (embossing) with
//!   `SdfGrid::subtract` / `union`, and the whole result is meshed again:
//!   closed and manifold by construction, but the whole target is resampled
//!   at the voxel size (an arch at 0.05 mm gives millions of triangles and
//!   takes tens of seconds) and loses its labels. For a caller that already
//!   holds the field of a shell at a fine voxel size: [`locate`] and [`apply`].

mod field;
mod local;
mod place;
mod refine;
mod text;

use dc_case::Fdi;
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anatomy::Arch;
use crate::evidence;
use crate::schedule::Date;

pub use place::{Frame, tooth_frame};
pub use text::{TextShape, layout, supported};

use place::Surface;

/// The solid of the text starts this far inside the cavity (shell) or inside
/// the model (the plugin's `UNDER_MM`), so that the operation leaves no film.
const UNDER_MM: f64 = 0.3;
/// Characters of the case code.
const CODE_MAX: usize = 8;
/// Probes of the check of the result.
const PROBES: usize = 32;
/// Fraction of the probes that must behave as expected.
const PROBE_SHARE: f64 = 0.8;

/// Errors of the stamp.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StampError {
    /// A parameter or the text is not valid.
    #[error("{0}")]
    Invalid(String),
    /// The bundled font cannot be read.
    #[error("font: {0}")]
    Font(String),
    /// A geometry kernel failed.
    #[error("geometry: {0}")]
    Geom(String),
    /// The result is not what the operation should give.
    #[error("{0}")]
    Result(String),
}

/// What the stamp is made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The model the sheet is thermoformed on: the text is embossed.
    #[default]
    Model,
    /// The aligner (printed directly): the text is engraved in the wall,
    /// from the side of the tooth, never raised towards the tooth.
    Shell,
}

/// Face of the tooth the text goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    /// Lingual / palatal (towards the centre of the arch).
    #[default]
    Lingual,
    /// Buccal / labial.
    Buccal,
}

impl Face {
    /// Name for messages.
    pub fn name(self) -> &'static str {
        match self {
            Face::Lingual => "lingual/palatal",
            Face::Buccal => "buccal",
        }
    }
}

/// From where the text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Read {
    /// From inside the aligner (looking at the wall from the tooth).
    #[default]
    Inside,
    /// From outside, through the transparent wall.
    Outside,
}

/// Format of the date in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum DateFormat {
    /// Day, month, year: `131026`.
    #[default]
    #[serde(rename = "DDMMYY")]
    DdMmYy,
    /// With dots: `13.10.26`.
    #[serde(rename = "DD.MM.YY")]
    DotDdMmYy,
    /// Year first: `261013`.
    #[serde(rename = "YYMMDD")]
    YyMmDd,
}

/// Tooth the text is tried on first (then the others of the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum ToothChoice {
    /// First molar, patient's right.
    #[default]
    #[serde(rename = "M6R")]
    FirstMolarRight,
    /// First molar, patient's left.
    #[serde(rename = "M6L")]
    FirstMolarLeft,
    /// Second molar, patient's right.
    #[serde(rename = "M7R")]
    SecondMolarRight,
    /// Second molar, patient's left.
    #[serde(rename = "M7L")]
    SecondMolarLeft,
    /// Second premolar, patient's right.
    #[serde(rename = "PM5R")]
    SecondPremolarRight,
    /// Second premolar, patient's left.
    #[serde(rename = "PM5L")]
    SecondPremolarLeft,
}

const TOOTH_ORDER: [ToothChoice; 6] = [
    ToothChoice::FirstMolarRight,
    ToothChoice::FirstMolarLeft,
    ToothChoice::SecondMolarRight,
    ToothChoice::SecondMolarLeft,
    ToothChoice::SecondPremolarRight,
    ToothChoice::SecondPremolarLeft,
];

impl ToothChoice {
    /// FDI number of this tooth in `arch`.
    pub fn fdi(self, arch: Arch) -> Option<Fdi> {
        let (pos, right) = match self {
            ToothChoice::FirstMolarRight => (6, true),
            ToothChoice::FirstMolarLeft => (6, false),
            ToothChoice::SecondMolarRight => (7, true),
            ToothChoice::SecondMolarLeft => (7, false),
            ToothChoice::SecondPremolarRight => (5, true),
            ToothChoice::SecondPremolarLeft => (5, false),
        };
        let quadrant = match (arch, right) {
            (Arch::Upper, true) => 1,
            (Arch::Upper, false) => 2,
            (Arch::Lower, false) => 3,
            (Arch::Lower, true) => 4,
        };
        Fdi::new(10 * quadrant + pos).ok()
    }
}

/// How the text is put on the mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Refine the surface under the text and move its vertices (fast, keeps
    /// the topology, the labels and the rest of the mesh).
    #[default]
    Local,
    /// Sample the mesh and the text as distance fields on a voxel grid,
    /// subtract or add, and mesh the whole result again (slow, resamples
    /// the whole mesh).
    Sdf,
}

/// Where the stamp goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Placement {
    /// On a tooth of the arch, found by the search of the plugin: the chosen
    /// tooth first, then the other molars and second premolars that are
    /// present. Needs the teeth in the [`Input`].
    Tooth {
        /// Tooth tried first.
        #[serde(default)]
        tooth: ToothChoice,
        /// Face tried first (then the other one).
        #[serde(default)]
        face: Face,
        /// From where the text is read.
        #[serde(default)]
        read: Read,
    },
    /// At a point of the surface given by hand, read from outside looking at
    /// the surface. The text is not moved if it does not fit well: it fails
    /// when the surface under it is not continuous.
    Manual {
        /// Centre of the text on the surface (mesh coordinates, mm).
        point: [f64; 3],
        /// Outward normal of the surface there.
        normal: [f64; 3],
        /// Direction of the top of the text (made perpendicular to the normal).
        up: [f64; 3],
    },
}

impl Default for Placement {
    fn default() -> Self {
        Placement::Tooth {
            tooth: ToothChoice::default(),
            face: Face::default(),
            read: Read::default(),
        }
    }
}

/// Size and wall settings of the stamp (the plugin's `stamp.Config`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Height of the characters, measured on the digit `0` (mm).
    pub height: f64,
    /// Depth of the engraving / height of the relief (mm).
    pub depth: f64,
    /// Wall left under the engraving (mm).
    pub min_wall: f64,
    /// Free border around the text (mm).
    pub margin: f64,
    /// Inner clearance of the shell (mm).
    pub clearance: f64,
}

/// Parameters of the stamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Text template with `{arch}`, `{number}`, `{passive}`, `{date}` and
    /// `{code}`; lines are separated by `\n`. Only `A-Z 0-9 space - . / : _ #`
    /// can be stamped.
    pub text: String,
    /// Aligner number.
    pub number: u32,
    /// The aligner is a passive one (a `P` after the number).
    pub passive: bool,
    /// Case code (letters, digits and `-`, at most 8).
    pub code: String,
    /// Date of the text, `YYYY-MM-DD` (default today).
    pub date: Option<String>,
    /// Format of the date.
    pub date_format: DateFormat,
    /// Height of the characters, measured on the digit `0` (mm, 0.3 to 2.5).
    pub height: f64,
    /// Depth of the engraving, or height of the relief (mm, 0.02 to 0.30).
    pub depth: f64,
    /// Wall that stays under an engraving (mm, 0.2 to 1.0; shell only).
    pub min_wall: f64,
    /// Free border around the text on the face (mm).
    pub margin: f64,
    /// Inner clearance of the shell (mm; shell only).
    pub clearance: f64,
    /// What the stamp is made on: a model (embossed) or a shell (engraved).
    pub kind: Kind,
    /// Where the text goes.
    pub placement: Placement,
    /// How the text is put on the mesh.
    pub method: Method,
    /// Longest edge of the refined surface under the text, or voxel size of
    /// the SDF method (mm, 0.01 to 0.2; default character height / 24, between
    /// 0.02 and 0.05). It is also the width of the wall of an engraving.
    pub resolution_mm: Option<f64>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            text: "{arch} {number} {passive}\n{date}\n{code}".to_owned(),
            number: 1,
            passive: false,
            code: String::new(),
            date: None,
            date_format: DateFormat::default(),
            height: evidence::param_f64("stamp_height").unwrap_or(0.7),
            depth: evidence::param_f64("stamp_depth").unwrap_or(0.08),
            min_wall: evidence::param_f64("stamp_min_wall").unwrap_or(0.35),
            margin: 0.2,
            clearance: 0.0,
            kind: Kind::default(),
            placement: Placement::default(),
            method: Method::default(),
            resolution_mm: None,
        }
    }
}

impl Params {
    /// The size and wall settings.
    pub fn config(&self) -> Config {
        Config {
            height: self.height,
            depth: self.depth,
            min_wall: self.min_wall,
            margin: self.margin,
            clearance: self.clearance,
        }
    }

    /// Longest edge of the refined surface / voxel size (mm).
    pub fn resolution(&self) -> f64 {
        self.resolution_mm
            .unwrap_or_else(|| (self.height / 24.0).clamp(0.02, 0.05))
    }

    fn check(&self) -> Result<(), StampError> {
        let bad = |what: &str, v: f64, lo: f64, hi: f64| {
            (!(lo..=hi).contains(&v))
                .then(|| StampError::Invalid(format!("{what} {v} is outside {lo}..{hi}")))
        };
        if let Some(e) = bad("height", self.height, 0.3, 2.5)
            .or_else(|| bad("depth", self.depth, 0.02, 0.30))
            .or_else(|| bad("min_wall", self.min_wall, 0.2, 1.0))
            .or_else(|| bad("margin", self.margin, 0.0, 2.0))
            .or_else(|| bad("clearance", self.clearance, 0.0, 2.0))
            .or_else(|| {
                self.resolution_mm
                    .and_then(|v| bad("resolution_mm", v, 0.01, 0.2))
            })
        {
            return Err(e);
        }
        if let Some(d) = &self.date {
            Date::parse(d).ok_or_else(|| StampError::Invalid(format!("bad date {d:?}")))?;
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------
// text
// ----------------------------------------------------------------------------

/// Label of an arch in the text.
pub fn arch_label(arch: Arch) -> &'static str {
    match arch {
        Arch::Upper => "SUP",
        Arch::Lower => "INF",
    }
}

/// The date in the chosen format.
pub fn date_text(day: Date, format: DateFormat) -> String {
    let yy = day.year.rem_euclid(100);
    match format {
        DateFormat::DdMmYy => format!("{:02}{:02}{yy:02}", day.day, day.month),
        DateFormat::DotDdMmYy => format!("{:02}.{:02}.{yy:02}", day.day, day.month),
        DateFormat::YyMmDd => format!("{yy:02}{:02}{:02}", day.month, day.day),
    }
}

/// The case code with only characters legible at 1 mm (`A-Z`, `0-9`, `-`),
/// at most 8.
pub fn clean_code(code: &str) -> String {
    code.to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
        .take(CODE_MAX)
        .collect()
}

/// The lines of the text: the template with its fields filled, in capitals,
/// without empty lines.
///
/// # Errors
/// [`StampError::Invalid`] for an unknown field or a character the font does not have.
pub fn label_lines(
    template: &str,
    arch: Arch,
    number: u32,
    passive: bool,
    date: &str,
    code: &str,
) -> Result<Vec<String>, StampError> {
    let mut text = template.to_owned();
    for (key, value) in [
        ("{arch}", arch_label(arch).to_owned()),
        ("{number}", format!("{number:02}")),
        (
            "{passive}",
            if passive {
                "P".to_owned()
            } else {
                String::new()
            },
        ),
        ("{date}", date.to_owned()),
        ("{code}", clean_code(code)),
    ] {
        text = text.replace(key, &value);
    }
    if let Some(i) = text.find('{') {
        let end = text[i..].find('}').map_or(text.len(), |j| i + j + 1);
        return Err(StampError::Invalid(format!(
            "unknown field {} in the text",
            &text[i..end]
        )));
    }
    let lines: Vec<String> = text
        .to_uppercase()
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(StampError::Invalid("the text is empty".to_owned()));
    }
    if let Some(c) = lines
        .iter()
        .flat_map(|l| l.chars())
        .find(|c| !supported(*c))
    {
        return Err(StampError::Invalid(format!(
            "the character {c:?} cannot be stamped"
        )));
    }
    Ok(lines)
}

/// The layouts to try in order: all the lines, without the third line (the
/// code), the first two lines on one line.
pub fn layouts(lines: &[String]) -> Vec<Vec<String>> {
    let mut out = vec![lines.to_vec()];
    if lines.len() > 2 {
        out.push(lines[..2].to_vec());
    }
    let one = vec![lines.iter().take(2).cloned().collect::<Vec<_>>().join(" ")];
    if !out.contains(&one) {
        out.push(one);
    }
    out
}

/// The text on one line (reports, manifest).
pub fn text_of(lines: &[String]) -> String {
    lines.join(" / ")
}

// ----------------------------------------------------------------------------
// input and result
// ----------------------------------------------------------------------------

/// A tooth of the stage where the text can go.
#[derive(Debug, Clone, PartialEq)]
pub struct ToothSite {
    /// Tooth number.
    pub fdi: Fdi,
    /// Points of the crown in the stage, above the gingival margin (mesh coordinates).
    pub crown: Vec<DVec3>,
    /// Mesio-distal axis of the tooth (any sign); `None` takes the tangent
    /// of the arch at the tooth.
    pub e_md: Option<DVec3>,
}

/// What the stamp works on.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    /// The solid to stamp (the shell or the model of a stage), in arch
    /// coordinates (occlusal = `+Z`), outward normals. Parts that overlap
    /// are merged.
    pub mesh: &'a TriMesh,
    /// Arch of the mesh.
    pub arch: Arch,
    /// The teeth of the stage (only those with at least 20 crown points are used).
    pub teeth: Vec<ToothSite>,
}

/// Where the stamp was put.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Placed {
    /// The text on one line.
    pub text: String,
    /// The lines of the text, from the top.
    pub lines: Vec<String>,
    /// Tooth number, for a placement on a tooth.
    pub tooth: Option<u8>,
    /// Face of the tooth, for a placement on a tooth.
    pub face: Option<Face>,
    /// Tilt of the text towards the occlusal side (degrees).
    pub tilt_deg: f64,
    /// Centre of the text on the surface (mesh coordinates).
    pub center: [f64; 3],
    /// Outward normal of the surface at the centre.
    pub normal: [f64; 3],
    /// Direction of the top of the text.
    pub up: [f64; 3],
    /// Width of the ink (mm).
    pub width_mm: f64,
    /// Height of the ink (mm).
    pub height_mm: f64,
    /// Depth of the engraving / height of the relief (mm).
    pub depth_mm: f64,
    /// Engraved (`true`) or embossed.
    pub engraved: bool,
}

/// A spot found for the stamp, with the solid of the text.
#[derive(Debug, Clone)]
pub struct Located {
    /// Closed prisms (one per triangle of the glyphs) following the
    /// surface, from `0.3 mm` outside it to `depth` inside (engraving) or
    /// outside (relief). Overlapping prisms are merged when sampled.
    pub solid: TriMesh,
    /// Where it is.
    pub placed: Placed,
    engrave: bool,
    /// Points that are solid (engraving) or empty (relief) before the
    /// operation and the opposite after it.
    probes: Vec<DVec3>,
    /// The glyphs.
    shape: TextShape,
    /// Directions of the rays and of the text.
    frame: Frame,
    /// Where the centre of the text is in the plane of the rays.
    origin: DVec3,
}

/// The result of [`run`].
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The text was put on the mesh.
    Stamped(Box<Stamped>),
    /// No tooth could take the text (the mesh is left as it is).
    Skipped {
        /// Why, tooth by tooth.
        reason: String,
    },
}

/// A stamped mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamped {
    /// The mesh with the text (`Method::Local`: the input with a refined
    /// patch, same labels; `Method::Sdf`: closed and manifold, resampled, no
    /// labels).
    pub mesh: TriMesh,
    /// Where the text is.
    pub placed: Placed,
    /// Longest edge under the text / voxel size used (mm).
    pub resolution_mm: f64,
    /// Triangles added by the refinement (`Method::Local`).
    pub triangles_added: usize,
}

// ----------------------------------------------------------------------------
// placement and solid
// ----------------------------------------------------------------------------

/// Closed solid of the glyphs along the surface: a prism per triangle whose
/// points go from `under` mm on the empty side of the surface to `depth` mm
/// on the other one. `h` and `n` are the surface points under the glyph
/// points and the outward normals there.
fn text_solid(text: &TextShape, h: &[DVec3], n: &[DVec3], depth: f64, engrave: bool) -> TriMesh {
    let into: Vec<DVec3> = n.iter().map(|v| if engrave { -*v } else { *v }).collect();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for t in &text.tris {
        let ids = t.map(|i| i as usize);
        let near = ids.map(|i| h[i] - UNDER_MM * into[i]);
        let far = ids.map(|i| h[i] + depth * into[i]);
        let base = positions.len() as u32;
        positions.extend(near.iter().chain(far.iter()).map(|p| p.as_vec3()));
        // near cap 0-1-2, far cap 3-4-5, three side quads
        let mut faces: [[u32; 3]; 8] = [
            [0, 1, 2],
            [3, 5, 4],
            [0, 3, 4],
            [0, 4, 1],
            [1, 4, 5],
            [1, 5, 2],
            [2, 5, 3],
            [2, 3, 0],
        ];
        let p: Vec<DVec3> = near.iter().chain(far.iter()).copied().collect();
        let volume: f64 = faces
            .iter()
            .map(|f| p[f[0] as usize].dot(p[f[1] as usize].cross(p[f[2] as usize])))
            .sum();
        if volume.abs() < 1e-12 {
            positions.truncate(base as usize);
            continue;
        }
        if volume < 0.0 {
            for f in &mut faces {
                f.swap(1, 2);
            }
        }
        tris.extend(faces.iter().map(|f| f.map(|i| i + base)));
    }
    TriMesh::new(positions, tris)
}

/// The incentres of the `count` triangles of the text with the largest
/// inscribed circle: points well inside the ink, as the barycentric weights
/// in the triangle and its vertex indices.
fn deep_points(text: &TextShape, count: usize) -> Vec<([usize; 3], [f64; 3])> {
    let mut scored: Vec<(f64, [usize; 3], [f64; 3])> = text
        .tris
        .iter()
        .map(|t| {
            let ids = t.map(|i| i as usize);
            let [a, b, c] = ids.map(|i| text.points[i]);
            // the weight of a vertex is the length of the opposite side
            let (la, lb, lc) = ((b - c).length(), (c - a).length(), (a - b).length());
            let per = la + lb + lc;
            let area = 0.5 * (b - a).perp_dot(c - a).abs();
            (2.0 * area / per, ids, [la / per, lb / per, lc / per])
        })
        .collect();
    scored.sort_by(|x, y| y.0.total_cmp(&x.0));
    scored
        .into_iter()
        .take(count)
        .map(|(_, i, w)| (i, w))
        .collect()
}

/// Points that are solid (engraving) or empty (relief) before the operation
/// and the opposite after it: under the incentres of the best triangles of
/// the text, half a depth into the wall or off the surface.
fn probes(text: &TextShape, h: &[DVec3], n: &[DVec3], depth: f64, engrave: bool) -> Vec<DVec3> {
    deep_points(text, PROBES)
        .into_iter()
        .map(|(ids, w)| {
            let p: DVec3 = (0..3).map(|k| w[k] * h[ids[k]]).sum();
            let nn: DVec3 = (0..3)
                .map(|k| w[k] * n[ids[k]])
                .sum::<DVec3>()
                .normalize_or_zero();
            p + nn * (depth * 0.5 * if engrave { -1.0 } else { 1.0 })
        })
        .collect()
}

/// The text layouts of a call: lines and their glyphs, in order of preference.
struct Layouts {
    list: Vec<(Vec<String>, TextShape)>,
}

impl Layouts {
    fn new(params: &Params, arch: Arch) -> Result<Self, StampError> {
        params.check()?;
        let day = match &params.date {
            Some(d) => {
                Date::parse(d).ok_or_else(|| StampError::Invalid(format!("bad date {d:?}")))?
            }
            None => Date::today(),
        };
        let lines = label_lines(
            &params.text,
            arch,
            params.number,
            params.passive,
            &date_text(day, params.date_format),
            &params.code,
        )?;
        let list = layouts(&lines)
            .into_iter()
            .map(|l| layout(&l, params.height).map(|shape| (l, shape)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { list })
    }

    fn sizes(&self) -> Vec<(f64, f64)> {
        self.list.iter().map(|(_, s)| (s.width, s.height)).collect()
    }
}

/// Mean distance along `n` from the plane of the rays to the surface.
fn text_reach(shape: &TextShape, h: &[DVec3], found: &place::Found) -> f64 {
    let f = &found.frame;
    let total: f64 = shape
        .points
        .iter()
        .zip(h)
        .map(|(p, h)| {
            let o = found.center + (p.x + found.pos.0) * f.r + (p.y + found.pos.1) * f.u;
            (*h - o).dot(f.n)
        })
        .sum();
    total / h.len().max(1) as f64
}

/// Builds the located stamp from a spot: projects the glyphs and makes the solid.
fn locate_at(
    surface: &Surface<'_>,
    lay: &Layouts,
    found: &place::Found,
    params: &Params,
    tooth: Option<(u8, Face)>,
) -> Result<Located, String> {
    let (lines, shape) = &lay.list[found.layout];
    let (h, n) = place::project(surface, found.center, &found.frame, found.pos, shape)
        .ok_or_else(|| "the surface under the text is not continuous".to_owned())?;
    let engrave = params.kind == Kind::Shell;
    let solid = text_solid(shape, &h, &n, params.depth, engrave);
    let k = h.len() as f64;
    // the text origin on the surface: the mean depth of the rays along `n`
    let reach = text_reach(shape, &h, found);
    let center = found.center
        + found.pos.0 * found.frame.r
        + found.pos.1 * found.frame.u
        + reach * found.frame.n;
    let normal = (n.iter().copied().sum::<DVec3>() / k).normalize_or_zero();
    let up = (found.frame.u - found.frame.u.dot(normal) * normal).normalize_or_zero();
    Ok(Located {
        solid,
        placed: Placed {
            text: text_of(lines),
            lines: lines.clone(),
            tooth: tooth.map(|t| t.0),
            face: tooth.map(|t| t.1),
            tilt_deg: found.tilt,
            center: center.to_array(),
            normal: normal.to_array(),
            up: up.to_array(),
            width_mm: shape.width,
            height_mm: shape.height,
            depth_mm: params.depth,
            engraved: engrave,
        },
        engrave,
        probes: probes(shape, &h, &n, params.depth, engrave),
        shape: shape.clone(),
        frame: found.frame,
        origin: found.center + found.pos.0 * found.frame.r + found.pos.1 * found.frame.u,
    })
}

/// The tooth numbers to try, in order: the chosen one, then the other molars
/// and second premolars of the list, only those present.
fn candidates<'a>(input: &'a Input<'_>, first: ToothChoice) -> Vec<&'a ToothSite> {
    let mut order = vec![first];
    order.extend(TOOTH_ORDER.iter().copied().filter(|c| *c != first));
    let mut out: Vec<&ToothSite> = Vec::new();
    for c in order {
        let Some(fdi) = c.fdi(input.arch) else {
            continue;
        };
        if let Some(site) = input
            .teeth
            .iter()
            .find(|s| s.fdi == fdi && s.crown.len() >= 20)
            && !out.iter().any(|s| s.fdi == fdi)
        {
            out.push(site);
        }
    }
    out
}

fn arch_center(input: &Input<'_>) -> DVec3 {
    let sites: Vec<DVec3> = input
        .teeth
        .iter()
        .filter(|s| s.crown.len() >= 20)
        .map(|s| s.crown.iter().copied().sum::<DVec3>() / s.crown.len() as f64)
        .collect();
    sites.iter().copied().sum::<DVec3>() / sites.len().max(1) as f64
}

/// Spot of the text on one tooth, with the solid; `Err` carries the reason.
fn locate_tooth(
    surface: &Surface<'_>,
    lay: &Layouts,
    site: &ToothSite,
    center_of_arch: DVec3,
    params: &Params,
    (first_face, read): (Face, Read),
) -> Result<Located, String> {
    let cfg = params.config();
    let c = site.crown.iter().copied().sum::<DVec3>() / site.crown.len() as f64;
    let e_md = site.e_md.unwrap_or_else(|| {
        // tangent of the arch: horizontal, perpendicular to the direction from the arch centre
        let radial = DVec3::new(c.x - center_of_arch.x, c.y - center_of_arch.y, 0.0);
        let t = DVec3::Z.cross(radial);
        if t.length() > 1e-9 { t } else { DVec3::X }
    });
    let frames: Vec<(Face, Frame)> = place::faces(first_face)
        .into_iter()
        .map(|f| (f, tooth_frame(e_md, c, center_of_arch, f, read)))
        .collect();
    let found = place::find_place(
        surface,
        &site.crown,
        &frames,
        params.kind,
        &cfg,
        &lay.sizes(),
    )?;
    locate_at(
        surface,
        lay,
        &found,
        params,
        Some((site.fdi.get(), found.face)),
    )
}

/// Spot of a manual placement.
fn locate_manual(
    surface: &Surface<'_>,
    lay: &Layouts,
    params: &Params,
    point: [f64; 3],
    normal: [f64; 3],
    up: [f64; 3],
) -> Result<Located, String> {
    let normal = DVec3::from(normal);
    let frame = place::manual_frame(normal, DVec3::from(up)).ok_or_else(|| {
        "the normal and the up direction must be non-zero and not parallel".to_owned()
    })?;
    let found = place::Found {
        face: Face::Lingual,
        tilt: 0.0,
        layout: 0,
        pos: (0.0, 0.0),
        center: DVec3::from(point) + normal.normalize_or_zero(),
        frame,
    };
    locate_at(surface, lay, &found, params, None)
}

/// Finds the place of the text on the mesh of `input` and builds its solid,
/// without changing the mesh. Use it with [`apply`] on a distance field you
/// already hold.
///
/// # Errors
/// [`StampError::Invalid`] for bad parameters or text; [`StampError::Result`]
/// with the reason when no place is good.
pub fn locate(input: &Input<'_>, params: &Params) -> Result<Located, StampError> {
    let lay = Layouts::new(params, input.arch)?;
    let surface = Surface::new(input.mesh)?;
    match &params.placement {
        Placement::Manual { point, normal, up } => {
            locate_manual(&surface, &lay, params, *point, *normal, *up).map_err(StampError::Result)
        }
        Placement::Tooth { tooth, face, read } => {
            let center = arch_center(input);
            let mut tried = Vec::new();
            for site in candidates(input, *tooth) {
                match locate_tooth(&surface, &lay, site, center, params, (*face, *read)) {
                    Ok(l) => return Ok(l),
                    Err(why) => tried.push(format!("{}: {why}", site.fdi.get())),
                }
            }
            Err(StampError::Result(no_tooth(tried)))
        }
    }
}

fn no_tooth(tried: Vec<String>) -> String {
    if tried.is_empty() {
        "no suitable tooth in the stage".to_owned()
    } else {
        tried.join("; ")
    }
}

/// Subtracts (engraving) or adds (relief) the text solid on `target`, a
/// distance field of the stamped mesh.
///
/// # Errors
/// [`StampError::Geom`] when the grids do not line up or sampling fails;
/// [`StampError::Result`] when the text does not change the field as
/// expected (the surface under it is not a closed solid).
pub fn apply(target: &mut SdfGrid, located: &Located) -> Result<(), StampError> {
    let geom = |e: dc_geom::GeomError| StampError::Geom(e.to_string());
    let share = |g: &SdfGrid, solid: bool| {
        let n = located
            .probes
            .iter()
            .filter(|p| (g.sample(p.as_vec3()) < 0.0) == solid)
            .count();
        n as f64 / located.probes.len().max(1) as f64
    };
    // the wall must be there (engraving) or the space free (relief) before
    if share(target, located.engrave) < PROBE_SHARE {
        return Err(StampError::Result(
            "the surface under the text is not a closed solid".to_owned(),
        ));
    }
    let text = SdfGrid::from_mesh(
        &located.solid,
        &SdfParams {
            voxel_size: target.voxel_size,
            band: target.band(),
        },
    )
    .map_err(geom)?;
    if located.engrave {
        target.subtract(&text).map_err(geom)?;
    } else {
        target.union(&text).map_err(geom)?;
    }
    if share(target, !located.engrave) < PROBE_SHARE {
        return Err(StampError::Result(
            "the text did not change the surface as expected".to_owned(),
        ));
    }
    Ok(())
}

/// Puts the text on the mesh: finds the place (the candidate teeth in
/// order) and applies the stamp field with `params.method` (see the module
/// docs). A tooth where this fails is skipped for the next one.
///
/// # Errors
/// [`StampError::Invalid`] for bad parameters or text and [`StampError::Geom`]
/// for a mesh that cannot be sampled. A text that cannot be placed is not an
/// error: it is [`Outcome::Skipped`].
pub fn run(input: &Input<'_>, params: &Params) -> Result<Outcome, StampError> {
    let lay = Layouts::new(params, input.arch)?;
    let surface = Surface::new(input.mesh)?;
    let resolution = params.resolution();
    let mut base: Option<SdfGrid> = None;
    let mut tried = Vec::new();

    let mut attempt =
        |located: Result<Located, String>, label: String| -> Result<Option<Stamped>, StampError> {
            let located = match located {
                Ok(l) => l,
                Err(why) => {
                    tried.push(format!("{label}{why}"));
                    return Ok(None);
                }
            };
            let (mesh, triangles_added) = match params.method {
                Method::Local => match local::stamp(&surface, &located, params, resolution) {
                    Ok(done) => done,
                    Err(why) => {
                        tried.push(format!("{label}{why}"));
                        return Ok(None);
                    }
                },
                Method::Sdf => {
                    if base.is_none() {
                        base = Some(
                            SdfGrid::from_mesh(
                                input.mesh,
                                &SdfParams {
                                    voxel_size: resolution as f32,
                                    band: (4.0 * resolution) as f32,
                                },
                            )
                            .map_err(|e| StampError::Geom(e.to_string()))?,
                        );
                    }
                    let mut grid = base.clone().expect("built above");
                    if let Err(e) = apply(&mut grid, &located) {
                        tried.push(format!("{label}{e}"));
                        return Ok(None);
                    }
                    let mesh = grid
                        .to_mesh()
                        .map_err(|e| StampError::Geom(e.to_string()))?;
                    let report = mesh.validate();
                    if mesh.is_empty() || !report.is_watertight() || !report.is_manifold() {
                        tried.push(format!("{label}the result is not a closed manifold"));
                        return Ok(None);
                    }
                    (mesh, 0)
                }
            };
            Ok(Some(Stamped {
                mesh,
                placed: located.placed,
                resolution_mm: resolution,
                triangles_added,
            }))
        };

    match &params.placement {
        Placement::Manual { point, normal, up } => {
            let l = locate_manual(&surface, &lay, params, *point, *normal, *up);
            if let Some(s) = attempt(l, String::new())? {
                return Ok(Outcome::Stamped(Box::new(s)));
            }
        }
        Placement::Tooth { tooth, face, read } => {
            let center = arch_center(input);
            for site in candidates(input, *tooth) {
                let l = locate_tooth(&surface, &lay, site, center, params, (*face, *read));
                if let Some(s) = attempt(l, format!("{}: ", site.fdi.get()))? {
                    return Ok(Outcome::Stamped(Box::new(s)));
                }
            }
        }
    }
    Ok(Outcome::Skipped {
        reason: no_tooth(tried),
    })
}

/// The text of a mesh point in the frame `(r, u)` of the placement: used by
/// tests and tools to read the stamp back.
pub fn text_plane(placed: &Placed, point: DVec3) -> DVec2 {
    let n = DVec3::from(placed.normal);
    let u = DVec3::from(placed.up);
    let r = (-n).cross(u);
    let d = point - DVec3::from(placed.center);
    DVec2::new(d.dot(r), d.dot(u))
}

#[cfg(test)]
mod tests;
