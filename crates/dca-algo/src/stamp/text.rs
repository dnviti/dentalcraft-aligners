//! The text of the stamp as 2D triangles: glyph outlines of the bundled font
//! (`ttf-parser`), laid out in lines and filled (`lyon_tessellation`).
//!
//! The plugin asks Blender for a text curve of the default font and takes the
//! mesh it fills. Here the font is B612 Mono Bold (SIL OFL 1.1, the brand's
//! font for measured values), subset to the characters a label can hold.
//! Its size is set like the plugin does: the height of the digit `0` is the
//! requested character height.
//!
//! The advances of a monospaced font are made for a screen: at these sizes
//! they left a label 23 % wider than the plugin's, and the one-line layout,
//! the one that fits between the contacts of a molar, did not fit any more.
//! So the glyphs are set by their ink, with the gap and the word space of the
//! plugin's text ([`GAP`], [`WORD_SPACE`]), and drawn [`CONDENSE`] as wide (the
//! digits of the bold are wider than the plugin's): a label is as wide as the
//! plugin's whatever its digits.

use glam::DVec2;
use lyon_tessellation::geom::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use ttf_parser::{Face, OutlineBuilder};

use super::StampError;

/// The bundled font (B612 Mono Bold, subset; see `assets/OFL-B612-Mono.txt`).
static FONT: &[u8] = include_bytes!("../../assets/B612Mono-Bold-subset.ttf");

/// Distance between the baselines of two lines, in character heights (the
/// plugin's `LINE_PITCH`: a gap of 0.45 between the lines).
pub(super) const LINE_PITCH: f64 = 1.45;

/// Space between the ink of two neighbouring characters, in character
/// heights (the digits of the plugin's font: 0.06 mm at 0.7 mm).
const GAP: f64 = 0.085;
/// Width of a space, in character heights (added to the gaps on its two
/// sides; the plugin's font: 0.28 mm at 0.7 mm).
const WORD_SPACE: f64 = 0.4;

/// Width of the glyphs against their height.
const CONDENSE: f64 = 0.94;

/// Largest deviation of the flattened curves from the outline (mm).
const TOLERANCE_MM: f32 = 0.002;

/// `true` when the bundled font has a glyph for `c`.
pub fn supported(c: char) -> bool {
    c == ' ' || c.is_ascii_digit() || c.is_ascii_uppercase() || "-./:_#".contains(c)
}

/// The text as a flat triangulation centred on `(0, 0)` (mm; `x` to the
/// right of the reader, `y` up).
#[derive(Debug, Clone, PartialEq)]
pub struct TextShape {
    /// Vertices.
    pub points: Vec<DVec2>,
    /// Counter-clockwise triangles.
    pub tris: Vec<[u32; 3]>,
    /// Width of the ink (mm).
    pub width: f64,
    /// Height of the ink (mm).
    pub height: f64,
}

/// Outline of one glyph appended to a path.
struct Outline<'a> {
    builder: &'a mut lyon_tessellation::path::path::Builder,
    /// Origin of the glyph (mm).
    x: f32,
    y: f32,
    scale: f32,
    /// Horizontal scale.
    scale_x: f32,
}

impl Outline<'_> {
    fn at(&self, x: f32, y: f32) -> lyon_tessellation::geom::Point<f32> {
        point(self.x + x * self.scale_x, self.y + y * self.scale)
    }
}

impl OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.builder.begin(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.builder.line_to(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.at(x1, y1), self.at(x, y));
        self.builder.quadratic_bezier_to(c, p);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.builder.cubic_bezier_to(c1, c2, p);
    }
    fn close(&mut self) {
        self.builder.end(true);
    }
}

/// Lays out `lines` (centred line by line, then as a block) with characters
/// `height` mm high and fills the glyphs.
///
/// # Errors
/// [`StampError::Invalid`] for a character the font does not have, or text
/// without any ink.
pub fn layout(lines: &[String], height: f64) -> Result<TextShape, StampError> {
    let face = Face::parse(FONT, 0).map_err(|e| StampError::Font(e.to_string()))?;
    let zero = face
        .glyph_index('0')
        .and_then(|g| face.glyph_bounding_box(g))
        .ok_or_else(|| StampError::Font("the font has no digit 0".to_owned()))?;
    let scale = height / f64::from(zero.y_max - zero.y_min);
    let scale_x = scale * CONDENSE;
    let pitch = LINE_PITCH * height;

    let mut builder = Path::builder();
    for (row, line) in lines.iter().enumerate() {
        let mut glyphs = Vec::new();
        // the pen stands where the ink of the next character starts
        let mut pen = 0.0f64;
        for c in line.chars() {
            let gid = face
                .glyph_index(c)
                .filter(|_| supported(c))
                .ok_or_else(|| {
                    StampError::Invalid(format!("the character {c:?} cannot be stamped"))
                })?;
            match face.glyph_bounding_box(gid) {
                Some(ink) => {
                    glyphs.push((gid, pen - f64::from(ink.x_min) * scale_x));
                    pen += f64::from(ink.x_max - ink.x_min) * scale_x + GAP * height;
                }
                None => pen += WORD_SPACE * height,
            }
        }
        let x0 = -0.5 * pen;
        let y0 = -(row as f64) * pitch;
        for (gid, dx) in glyphs {
            let mut outline = Outline {
                builder: &mut builder,
                x: (x0 + dx) as f32,
                y: y0 as f32,
                scale: scale as f32,
                scale_x: scale_x as f32,
            };
            face.outline_glyph(gid, &mut outline);
        }
    }
    let path = builder.build();

    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::tolerance(TOLERANCE_MM).with_fill_rule(FillRule::NonZero),
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
        )
        .map_err(|e| StampError::Font(format!("the glyphs cannot be filled: {e:?}")))?;

    let mut points: Vec<DVec2> = buffers
        .vertices
        .iter()
        .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[1])))
        .collect();
    let mut tris = Vec::with_capacity(buffers.indices.len() / 3);
    for t in buffers.indices.as_chunks::<3>().0 {
        let (a, b, c) = (
            points[t[0] as usize],
            points[t[1] as usize],
            points[t[2] as usize],
        );
        let area2 = (b - a).perp_dot(c - a);
        if area2.abs() < 1e-12 {
            continue;
        }
        tris.push(if area2 > 0.0 {
            [t[0], t[1], t[2]]
        } else {
            [t[0], t[2], t[1]]
        });
    }
    // weld the vertices that coincide, so that the outline is the set of edges used once
    let mut canon: std::collections::HashMap<(i64, i64), u32> = std::collections::HashMap::new();
    let remap: Vec<u32> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let key = ((p.x * 1e5).round() as i64, (p.y * 1e5).round() as i64);
            *canon.entry(key).or_insert(i as u32)
        })
        .collect();
    for t in &mut tris {
        *t = t.map(|i| remap[i as usize]);
    }
    tris.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    if tris.is_empty() {
        return Err(StampError::Invalid(
            "the text has no visible character".to_owned(),
        ));
    }

    // only the vertices of the triangles, centred on the ink
    let mut used = vec![false; points.len()];
    for t in &tris {
        for &i in t {
            used[i as usize] = true;
        }
    }
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for (p, _) in points.iter().zip(&used).filter(|(_, u)| **u) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let centre = 0.5 * (lo + hi);
    for p in &mut points {
        *p -= centre;
    }
    Ok(TextShape {
        points,
        tris,
        width: hi.x - lo.x,
        height: hi.y - lo.y,
    })
}
