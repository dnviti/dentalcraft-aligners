//! Print readiness of a triangle mesh (`ortho_aligner/printcheck.py`).
//!
//! A file is ready to print when it is one closed solid: every edge shared
//! by exactly two triangles with opposite orientation (watertight, manifold,
//! consistently oriented), no degenerate triangles, no self-intersections,
//! one connected piece and positive volume. A shell (aligner) and a model
//! must also have no hole through them (genus 0), a shell a minimum wall,
//! and the part must fit the build plate.
//!
//! The mesh is welded first (identical positions become one vertex): that is
//! what the slicer sees in the STL file, which has no connectivity. The
//! topology counts come from that welded mesh; the self-intersections use
//! the robust crossing test of the shell stage
//! ([`crate::shell::crossing_pairs`]: an edge of a triangle through another
//! one, nearly coplanar triangles left out); the thickness casts a ray into
//! the solid from the centre of the faces.
//!
//! The reach of a shell ([`shell_reach`], `printcheck.shell_reach`): an
//! EQUATOR shell has no vertex farther from the teeth of its stage (and the
//! blockout of their contacts) than its wall and [`REACH_FAR_MM`], else it
//! has a fin or a flap over the gingiva ([`add_reach`] makes that a problem
//! of the check). The export measures it, since it needs the teeth.

use std::collections::HashMap;

use dc_geom::bvh::Bvh;
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::text;
use crate::schedule::Lang;

/// Area (twice) under which a triangle is degenerate (the plugin's value).
const DEGENERATE_AREA2: f64 = 1e-10;
/// Inward ray start offset along the face (mm), to leave the face itself.
const RAY_EPS: f32 = 1e-4;
/// Pairs of intersecting triangles kept in the report.
const MAX_PAIRS: usize = 8;

/// What the check looks at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CheckOptions {
    /// Count the pairs of triangles that cross each other.
    pub self_intersections: bool,
    /// Wall that no part of a shell may be thinner than (mm); `None` skips
    /// the thickness (a model is solid).
    pub min_thickness_mm: Option<f64>,
    /// Faces probed for the thickness at most (an even sample of the mesh).
    pub thickness_samples: usize,
    /// Build volume of the printer `[x, y, z]` (mm); `None` skips the fit.
    pub build_plate_mm: Option<[f64; 3]>,
    /// Holes through the solid allowed (a model and a shell have none).
    pub max_genus: Option<u32>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            self_intersections: true,
            min_thickness_mm: None,
            thickness_samples: 20_000,
            build_plate_mm: Some(DEFAULT_PLATE_MM),
            max_genus: Some(0),
        }
    }
}

/// Build plate of the default printer, the one the print popup prepares
/// (Elegoo Saturn 4 Ultra 16K, `dc-print` `PrinterProfile::saturn_4_ultra_16k`:
/// 211.68 x 118.37 x 220 mm), less the margin its nesting keeps from every
/// edge (`dc-print` `LayoutParams::default`, 4 mm) in X and Y: the room a
/// part has on the plate. dca-algo does not depend on dc-print (core): keep
/// these figures in step with the profile.
pub const PLATE_MARGIN_MM: f64 = 4.0;
/// See [`PLATE_MARGIN_MM`].
pub const DEFAULT_PLATE_MM: [f64; 3] = [
    211.68 - 2.0 * PLATE_MARGIN_MM,
    118.37 - 2.0 * PLATE_MARGIN_MM,
    220.0,
];

/// Something that makes a file unfit to print.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Problem {
    /// The mesh has no triangle.
    Empty,
    /// Edges used by one triangle only (holes).
    BoundaryEdges {
        /// How many.
        count: usize,
    },
    /// Edges used by more than two triangles.
    NonManifoldEdges {
        /// How many.
        count: usize,
    },
    /// Edges whose two triangles disagree on the orientation.
    FlippedEdges {
        /// How many.
        count: usize,
    },
    /// Triangles with (almost) no area.
    Degenerate {
        /// How many.
        count: usize,
    },
    /// More than one connected piece.
    Pieces {
        /// How many.
        count: usize,
    },
    /// Pairs of triangles that cross each other.
    SelfIntersections {
        /// How many pairs.
        count: usize,
    },
    /// Holes through the wall.
    Genus {
        /// Handles of the surface.
        count: i64,
        /// Where they are: the centre of the shortest loop found through
        /// each (coordinates of the file, mm); empty when none is found.
        at: Vec<[f64; 3]>,
    },
    /// Zero or negative volume (inverted normals).
    NonPositiveVolume,
    /// A part of the wall is thinner than the minimum.
    TooThin {
        /// Thinnest wall found (mm).
        min_mm: f64,
        /// Minimum wall (mm).
        limit_mm: f64,
    },
    /// The bounding box does not fit the build plate.
    DoesNotFit {
        /// Size of the part (mm).
        size_mm: [f64; 3],
        /// Build volume (mm).
        plate_mm: [f64; 3],
    },
    /// Shell vertices far from the teeth: a fin or a flap ([`shell_reach`]).
    Flap {
        /// Vertices beyond the limit.
        count: usize,
    },
}

impl Problem {
    /// The problem in words.
    pub fn text(&self, lang: Lang) -> String {
        let n = |k: &str, args: &[String]| text::fmt(lang, k, args);
        match self {
            Problem::Empty => n("chk-empty", &[]),
            Problem::BoundaryEdges { count } => n("chk-boundary", &[count.to_string()]),
            Problem::NonManifoldEdges { count } => n("chk-nonmanifold", &[count.to_string()]),
            Problem::FlippedEdges { count } => n("chk-flipped", &[count.to_string()]),
            Problem::Degenerate { count } => n("chk-degenerate", &[count.to_string()]),
            Problem::Pieces { count } => n("chk-pieces", &[count.to_string()]),
            Problem::SelfIntersections { count } => n("chk-selfint", &[count.to_string()]),
            Problem::Genus { count, at } if at.is_empty() => {
                text::fmt_n(lang, "chk-genus", *count as u64, &[count.to_string()])
            }
            Problem::Genus { count, at } => {
                // the decimal mark of the language; the Italian coordinates
                // are told apart by semicolons
                let places: Vec<String> = at
                    .iter()
                    .map(|p| {
                        let c = p.map(|x| super::html::num(lang, x, 1));
                        let sep = if lang == Lang::It { "; " } else { ", " };
                        format!("({})", c.join(sep))
                    })
                    .collect();
                text::fmt_n(
                    lang,
                    "chk-genus-at",
                    *count as u64,
                    &[count.to_string(), places.join(", ")],
                )
            }
            Problem::NonPositiveVolume => n("chk-volume", &[]),
            Problem::TooThin { min_mm, limit_mm } => n(
                "chk-thin",
                &[
                    super::html::num(lang, *min_mm, 2),
                    super::html::num(lang, *limit_mm, 2),
                ],
            ),
            Problem::DoesNotFit { size_mm, plate_mm } => n(
                "chk-fit",
                &[
                    format!(
                        "{} x {} x {}",
                        super::html::num(lang, size_mm[0], 1),
                        super::html::num(lang, size_mm[1], 1),
                        super::html::num(lang, size_mm[2], 1)
                    ),
                    format!(
                        "{:.0} x {:.0} x {:.0}",
                        plate_mm[0], plate_mm[1], plate_mm[2]
                    ),
                ],
            ),
            Problem::Flap { count } => n("chk-flap", &[count.to_string()]),
        }
    }
}

/// Print-readiness report of one mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrintCheck {
    /// Triangles.
    pub triangles: usize,
    /// Vertices after welding.
    pub vertices: usize,
    /// Edges used by one triangle only.
    pub boundary_edges: usize,
    /// Edges used by more than two triangles.
    pub nonmanifold_edges: usize,
    /// Edges whose two triangles are oriented the same way.
    pub flipped_edges: usize,
    /// Triangles with (almost) no area.
    pub degenerate: usize,
    /// Connected pieces.
    pub pieces: usize,
    /// Holes through the solid (`None` when the surface is not closed).
    pub genus: Option<i64>,
    /// Pairs of crossing triangles (`None` when not counted).
    pub self_intersections: Option<usize>,
    /// The first pairs of crossing triangles (face indices of the input).
    pub intersecting_pairs: Vec<[u32; 2]>,
    /// Enclosed volume (mm3, negative when the normals point inwards).
    pub volume_mm3: f64,
    /// Corner of the bounding box with the smallest coordinates.
    pub bounds_min: [f64; 3],
    /// Corner of the bounding box with the largest coordinates.
    pub bounds_max: [f64; 3],
    /// Size of the bounding box (mm).
    pub size_mm: [f64; 3],
    /// The part fits the build plate, in any of its two orientations about
    /// the vertical (`None` when not checked).
    pub fits_plate: Option<bool>,
    /// Thinnest wall found (mm; `None` when not measured).
    pub min_thickness_mm: Option<f64>,
    /// How far a shell reaches from the teeth (`None` when not measured,
    /// [`add_reach`]).
    pub reach: Option<Reach>,
    /// Nothing prevents printing it.
    pub ok: bool,
    /// What does.
    pub problems: Vec<Problem>,
}

impl PrintCheck {
    /// Volume in millilitres.
    pub fn volume_ml(&self) -> f64 {
        self.volume_mm3 / 1000.0
    }

    /// The problems in words.
    pub fn problem_texts(&self, lang: Lang) -> Vec<String> {
        self.problems.iter().map(|p| p.text(lang)).collect()
    }
}

/// The mesh with identical positions welded: `(positions, triangles)`.
fn weld(mesh: &TriMesh) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut index: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.positions.len());
    let mut positions: Vec<Vec3> = Vec::new();
    let map: Vec<u32> = mesh
        .positions
        .iter()
        .map(|p| {
            // + 0.0 turns -0.0 into 0.0
            let key = [
                (p.x + 0.0).to_bits(),
                (p.y + 0.0).to_bits(),
                (p.z + 0.0).to_bits(),
            ];
            *index.entry(key).or_insert_with(|| {
                positions.push(*p);
                (positions.len() - 1) as u32
            })
        })
        .collect();
    let tris = mesh
        .tris
        .iter()
        .map(|t| t.map(|i| map.get(i as usize).copied().unwrap_or(u32::MAX)))
        .collect();
    (positions, tris)
}

fn corners(pos: &[Vec3], t: [u32; 3]) -> [DVec3; 3] {
    t.map(|i| pos[i as usize].as_dvec3())
}

/// Twice the area of a triangle.
fn area2(c: &[DVec3; 3]) -> f64 {
    (c[1] - c[0]).cross(c[2] - c[0]).length()
}

/// A triangle with a repeated corner or (almost) no area
/// ([`DEGENERATE_AREA2`]): a degenerate triangle of the print check.
pub(crate) fn degenerate(pos: &[Vec3], t: [u32; 3]) -> bool {
    t[0] == t[1] || t[1] == t[2] || t[0] == t[2] || area2(&corners(pos, t)) < DEGENERATE_AREA2
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Checks `mesh`.
pub fn check(mesh: &TriMesh, options: &CheckOptions) -> PrintCheck {
    let (pos, tris) = weld(mesh);
    let in_range = |t: &[u32; 3]| t.iter().all(|&i| (i as usize) < pos.len());
    let mut out = PrintCheck {
        triangles: mesh.tris.len(),
        vertices: pos.len(),
        boundary_edges: 0,
        nonmanifold_edges: 0,
        flipped_edges: 0,
        degenerate: 0,
        pieces: 0,
        genus: None,
        self_intersections: None,
        intersecting_pairs: Vec::new(),
        volume_mm3: 0.0,
        bounds_min: [0.0; 3],
        bounds_max: [0.0; 3],
        size_mm: [0.0; 3],
        fits_plate: None,
        min_thickness_mm: None,
        reach: None,
        ok: false,
        problems: Vec::new(),
    };
    if tris.is_empty() || !tris.iter().all(in_range) {
        out.problems.push(Problem::Empty);
        return out;
    }

    // bounds
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for t in &tris {
        for p in corners(&pos, *t) {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    out.bounds_min = lo.to_array();
    out.bounds_max = hi.to_array();
    out.size_mm = (hi - lo).to_array();

    // edges
    let mut undirected: HashMap<(u32, u32), u32> = HashMap::with_capacity(tris.len() * 2);
    let mut directed: HashMap<(u32, u32), u32> = HashMap::with_capacity(tris.len() * 3);
    let mut degenerate_count = 0;
    for t in &tris {
        if degenerate(&pos, *t) {
            degenerate_count += 1;
        }
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if a == b {
                continue;
            }
            *directed.entry((a, b)).or_insert(0) += 1;
            *undirected.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    out.degenerate = degenerate_count;
    out.boundary_edges = undirected.values().filter(|&&c| c == 1).count();
    out.nonmanifold_edges = undirected.values().filter(|&&c| c > 2).count();
    out.flipped_edges = directed.values().filter(|&&c| c > 1).count();

    // pieces: triangles that share a vertex
    let mut parent: Vec<u32> = (0..pos.len() as u32).collect();
    let mut used = vec![false; pos.len()];
    for t in &tris {
        for &i in t {
            used[i as usize] = true;
        }
        let (a, b, c) = (
            find(&mut parent, t[0]),
            find(&mut parent, t[1]),
            find(&mut parent, t[2]),
        );
        parent[b as usize] = a;
        let c = find(&mut parent, c);
        let a = find(&mut parent, a);
        parent[c as usize] = a;
    }
    let mut roots: Vec<u32> = (0..pos.len() as u32)
        .filter(|&i| used[i as usize])
        .map(|i| find(&mut parent, i))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    out.pieces = roots.len();

    // genus of a closed manifold surface: handles = pieces - chi / 2
    if out.boundary_edges == 0 && out.nonmanifold_edges == 0 {
        let v = used.iter().filter(|u| **u).count() as f64;
        let chi = v - undirected.len() as f64 + tris.len() as f64;
        out.genus = Some((out.pieces as f64 - chi / 2.0).round() as i64);
    }

    // volume, around the first vertex to keep the sums small
    let origin = pos[tris[0][0] as usize].as_dvec3();
    out.volume_mm3 = tris
        .iter()
        .map(|t| {
            let c = corners(&pos, *t).map(|p| p - origin);
            c[0].dot(c[1].cross(c[2])) / 6.0
        })
        .sum();

    if options.self_intersections {
        let (count, pairs) = self_intersections(&pos, &tris);
        out.self_intersections = Some(count);
        out.intersecting_pairs = pairs;
    }
    let bvh = Bvh::build(&TriMesh::new(pos.clone(), tris.clone())).ok();

    let closed = out.boundary_edges == 0 && out.nonmanifold_edges == 0;
    if let (Some(_), true, true, Some(b)) = (
        options.min_thickness_mm,
        closed,
        out.volume_mm3 > 0.0,
        bvh.as_ref(),
    ) {
        out.min_thickness_mm = min_thickness(
            &pos,
            &tris,
            b,
            options.thickness_samples,
            options.min_thickness_mm.unwrap_or(0.0),
        );
    }

    if let Some(plate) = options.build_plate_mm {
        let s = out.size_mm;
        let flat = |a: f64, b: f64| s[0] <= a && s[1] <= b;
        out.fits_plate =
            Some((flat(plate[0], plate[1]) || flat(plate[1], plate[0])) && s[2] <= plate[2]);
    }

    // verdict
    let p = &mut out.problems;
    if out.boundary_edges > 0 {
        p.push(Problem::BoundaryEdges {
            count: out.boundary_edges,
        });
    }
    if out.nonmanifold_edges > 0 {
        p.push(Problem::NonManifoldEdges {
            count: out.nonmanifold_edges,
        });
    }
    if out.flipped_edges > 0 {
        p.push(Problem::FlippedEdges {
            count: out.flipped_edges,
        });
    }
    if out.degenerate > 0 {
        p.push(Problem::Degenerate {
            count: out.degenerate,
        });
    }
    if out.pieces != 1 {
        p.push(Problem::Pieces { count: out.pieces });
    }
    if let Some(n) = out.self_intersections.filter(|&n| n > 0) {
        p.push(Problem::SelfIntersections { count: n });
    }
    if let (Some(max), Some(g)) = (options.max_genus, out.genus)
        && g > i64::from(max)
    {
        p.push(Problem::Genus {
            count: g,
            at: hole_places(&pos, &tris, g.max(0) as usize),
        });
    }
    if out.volume_mm3 <= 0.0 {
        p.push(Problem::NonPositiveVolume);
    }
    if let (Some(limit), Some(min)) = (options.min_thickness_mm, out.min_thickness_mm)
        && min < limit
    {
        p.push(Problem::TooThin {
            min_mm: min,
            limit_mm: limit,
        });
    }
    if let (Some(plate), Some(false)) = (options.build_plate_mm, out.fits_plate) {
        p.push(Problem::DoesNotFit {
            size_mm: out.size_mm,
            plate_mm: plate,
        });
    }
    out.ok = out.problems.is_empty();
    out
}

/// The centre of the shortest loop found through every handle of the
/// closed surface `pos`, `tris` (at most `count`, a millimetre apart at
/// least): where its holes through the wall are
/// ([`crate::trim::holes::handle_loops`], [`crate::trim::holes::short_loops`]).
fn hole_places(pos: &[Vec3], tris: &[[u32; 3]], count: usize) -> Vec<[f64; 3]> {
    use crate::trim::holes;
    let p: Vec<DVec3> = pos.iter().map(|v| v.as_dvec3()).collect();
    let mut short: Vec<(f64, Vec<u32>)> = holes::handle_loops(&p, tris)
        .iter()
        .flat_map(|l| holes::short_loops(&p, tris, l, holes::CROP_MM, holes::ROOTS, 1))
        .collect();
    short.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<DVec3> = Vec::new();
    for (_, l) in short {
        if out.len() == count {
            break;
        }
        let c = l.iter().map(|&v| p[v as usize]).sum::<DVec3>() / l.len().max(1) as f64;
        if out.iter().all(|q| (*q - c).length() > 1.0) {
            out.push(c);
        }
    }
    out.into_iter().map(|c| c.to_array()).collect()
}

/// Checks the STL file at `path`.
pub fn check_file(
    path: impl AsRef<std::path::Path>,
    options: &CheckOptions,
) -> Result<PrintCheck, dc_mesh::MeshError> {
    let mesh = dc_mesh::io::read(path)?;
    Ok(check(&mesh, options))
}

// ---- reach of a shell (fins, flaps) ---------------------------------------------------------------------------

/// EQUATOR border: a shell vertex farther than the wall (clearance +
/// thickness) and this from every closed tooth of its stage and from every
/// contact bridge (the blockout of a contact and of the embrasure under it,
/// on which the wall rests) stands off the teeth: a fin, or a flap over the
/// gingiva (`REACH_FAR_MM`).
pub const REACH_FAR_MM: f64 = 0.6;
/// Vertices allowed beyond that (`REACH_FAR_VERTS`: none, a flap of 24
/// vertices is a flap).
pub const REACH_FAR_VERTS: usize = 0;
/// Vertices beyond the limit within this of the worst one are one fin,
/// reported together (mm, `REACH_CLUSTER_MM`).
pub const REACH_CLUSTER_MM: f64 = 3.0;
/// Distances are measured up to this; a vertex farther away counts as this
/// (mm, `REACH_QUERY_MM`).
pub const REACH_QUERY_MM: f64 = 10.0;
/// Fins reported at most (the plugin's 5).
const REACH_FINS: usize = 5;

/// The vertices beyond the limit around the farthest one of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fin {
    /// The farthest vertex (mm, to 0.1 mm as the plugin reports it).
    pub at: [f64; 3],
    /// Its distance from the teeth (mm, to 0.01 mm).
    pub max_mm: f64,
    /// Vertices beyond the limit within [`REACH_CLUSTER_MM`] of it.
    pub vertices: usize,
    /// The tooth nearest to it: `Some(0)` a contact bridge, `None` when the
    /// faces of the teeth are not labelled.
    pub fdi: Option<u8>,
}

/// How far the vertices of a shell are from the teeth
/// (`printcheck.shell_reach`); the vertices on a bar are set aside.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reach {
    /// Largest distance (mm).
    pub max_mm: f64,
    /// 99.99th percentile (mm).
    pub p9999_mm: f64,
    /// Median (mm).
    pub median_mm: f64,
    /// Vertices beyond the limit.
    pub n_far: usize,
    /// Vertices beyond the limit set aside because they are on a bar.
    pub n_bar: usize,
    /// The limit: the wall and [`REACH_FAR_MM`] (mm).
    pub limit_mm: f64,
    /// The vertices beyond the limit by fin, worst first (at most 5).
    pub fins: Vec<Fin>,
}

/// Linear percentile `q` (0..=100) of sorted values (`np.percentile`).
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q / 100.0 * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (pos - lo as f64) * (sorted[hi] - sorted[lo])
}

/// Shell material far from every tooth (`printcheck.shell_reach`): the
/// distance of every vertex of `shell` from the surface of `teeth` (the
/// closed teeth of the stage and its contact bridges, in the coordinates of
/// the shell; `face_fdi` the tooth of every face of it, `0` on a bridge,
/// empty when not known). The shell is the wall between the clearance and
/// `outer` (clearance + thickness) from the model, so a vertex farther than
/// `outer` + [`REACH_FAR_MM`] stands off the teeth ([`check`] passes it: one
/// piece, genus 0). A vertex beyond that within the limit of `bar_boxes`
/// (the closed solids the bars over gaps are the offset of; empty: no bar)
/// is on a bar and set aside.
///
/// # Errors
/// A mesh with a triangle index out of range.
pub fn shell_reach(
    shell: &[Vec3],
    teeth: &TriMesh,
    face_fdi: &[u8],
    outer: f64,
    bar_boxes: &[TriMesh],
) -> Result<Reach, dc_geom::GeomError> {
    let limit = outer + REACH_FAR_MM;
    let hits = Bvh::build(teeth)?.closest_points(shell, REACH_QUERY_MM as f32);
    let dist: Vec<f64> = hits
        .iter()
        .map(|h| h.map_or(REACH_QUERY_MM, |h| f64::from(h.distance)))
        .collect();
    let boxes = if bar_boxes.iter().any(|b| !b.tris.is_empty()) {
        let mut all = TriMesh::new(Vec::new(), Vec::new());
        for b in bar_boxes {
            let off = all.positions.len() as u32;
            all.positions.extend_from_slice(&b.positions);
            all.tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        }
        Some(Bvh::build(&all)?)
    } else {
        None
    };
    let on_bar: Vec<bool> = (0..shell.len())
        .into_par_iter()
        .map(|i| {
            dist[i] > limit
                && boxes.as_ref().is_some_and(|b| {
                    b.closest_point(shell[i], REACH_QUERY_MM as f32)
                        .is_some_and(|h| f64::from(h.distance) <= limit)
                })
        })
        .collect();
    let kept: Vec<usize> = (0..shell.len()).filter(|&i| !on_bar[i]).collect();
    let mut sorted: Vec<f64> = kept.iter().map(|&i| dist[i]).collect();
    sorted.sort_unstable_by(f64::total_cmp);
    let median = percentile(&sorted, 50.0);
    let far: Vec<usize> = kept.iter().copied().filter(|&i| dist[i] > limit).collect();
    let mut fins = Vec::new();
    let mut left = vec![true; far.len()];
    let r2 = REACH_CLUSTER_MM * REACH_CLUSTER_MM;
    while fins.len() < REACH_FINS {
        // the farthest vertex left (the first of equals, as argmax)
        let Some(w) = (0..far.len())
            .filter(|&k| left[k])
            .reduce(|a, b| if dist[far[b]] > dist[far[a]] { b } else { a })
        else {
            break;
        };
        let i = far[w];
        let p = shell[i].as_dvec3();
        let mut vertices = 0;
        for (k, &j) in far.iter().enumerate() {
            if left[k] && shell[j].as_dvec3().distance_squared(p) < r2 {
                left[k] = false;
                vertices += 1;
            }
        }
        let fdi = hits[i].and_then(|h| face_fdi.get(h.face as usize).copied());
        fins.push(Fin {
            at: p.to_array().map(|x| (x * 10.0).round_ties_even() / 10.0),
            max_mm: (dist[i] * 100.0).round_ties_even() / 100.0,
            vertices,
            fdi,
        });
    }
    Ok(Reach {
        max_mm: sorted.last().copied().unwrap_or(0.0),
        p9999_mm: percentile(&sorted, 99.99),
        median_mm: median,
        n_far: far.len(),
        n_bar: on_bar.iter().filter(|b| **b).count(),
        limit_mm: limit,
        fins,
    })
}

/// `check` of a shell with its reach (`_check` of the export): more than
/// [`REACH_FAR_VERTS`] vertices far from the teeth is a problem (a fin, a
/// flap over the gingiva).
pub fn add_reach(check: &mut PrintCheck, reach: Reach) {
    if reach.n_far > REACH_FAR_VERTS {
        check.problems.push(Problem::Flap { count: reach.n_far });
        check.ok = false;
    }
    check.reach = Some(reach);
}

// ---- self-intersections ---------------------------------------------------------------------------------------

/// Number of pairs of triangles that cross each other and share no vertex,
/// and the first few of them. Indices are those of the welded mesh, which
/// keeps the triangle order of the input.
///
/// The test is [`crate::shell::crossing_pairs`] (an edge of one triangle
/// meets the other, edges within a sine of 1e-4 of the plane of the other
/// left out): two nearly coplanar triangles of a flat wall, which rounding
/// tilts by about that much, do not count as crossing (the Moller
/// interval test read them as crossing along the ill-conditioned line of
/// their planes). Degenerate triangles are counted apart.
fn self_intersections(pos: &[Vec3], tris: &[[u32; 3]]) -> (usize, Vec<[u32; 2]>) {
    let bad = |t: [u32; 3]| degenerate(pos, t);
    let welded = TriMesh::new(pos.to_vec(), tris.to_vec());
    let pairs: Vec<[u32; 2]> = crate::shell::crossing_pairs(&welded)
        .into_iter()
        .filter(|&(i, j)| !bad(tris[i as usize]) && !bad(tris[j as usize]))
        .map(|(i, j)| [i, j])
        .collect();
    let count = pairs.len();
    (count, pairs.into_iter().take(MAX_PAIRS).collect())
}

// ---- thickness -----------------------------------------------------------------------------------------------

/// Thinnest wall: from the centre of an even sample of the faces, the
/// distance to the surface straight into the solid (the normals point out).
///
/// A span thinner than `limit` counts only when its two sides are not joined
/// along the surface within `2 * limit` of the probe ([`joined_near`]): the
/// faces of a feature smaller than that (the lands between the strokes of an
/// engraved text, a sharp crease, the very edge of a rounded rim) face each
/// other across less than the limit but are no wall.
fn min_thickness(
    pos: &[Vec3],
    tris: &[[u32; 3]],
    bvh: &Bvh,
    samples: usize,
    limit: f64,
) -> Option<f64> {
    let step = (tris.len() / samples.max(1)).max(1);
    let around = vertex_faces(pos.len(), tris);
    let reach = (2.0 * limit) as f32;
    (0..tris.len())
        .step_by(step)
        .collect::<Vec<_>>()
        .par_iter()
        .filter_map(|&i| {
            let [a, b, c] = tris[i].map(|v| pos[v as usize]);
            let n = (b - a).cross(c - a);
            let len = n.length();
            if len < 1e-9 {
                return None;
            }
            let n = n / len;
            let centre = (a + b + c) / 3.0;
            let hit = bvh.ray_cast_range(centre - n * RAY_EPS, -n, 0.0, f32::INFINITY)?;
            let t = f64::from(hit.t + RAY_EPS);
            if hit.face as usize == i
                || (t < limit
                    && joined_near(pos, tris, &around, i, hit.face as usize, centre, reach))
            {
                return None;
            }
            Some(t)
        })
        .reduce_with(f64::min)
}

/// The point of the triangle `a b c` closest to `p` (Ericson, Real-Time
/// Collision Detection 5.1.5).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom.abs() < f32::MIN_POSITIVE {
        return a;
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

/// The faces around every vertex (compressed rows: `(start, faces)`).
fn vertex_faces(n: usize, tris: &[[u32; 3]]) -> (Vec<u32>, Vec<u32>) {
    let mut start = vec![0u32; n + 1];
    for t in tris {
        for &v in t {
            start[v as usize + 1] += 1;
        }
    }
    for k in 0..n {
        start[k + 1] += start[k];
    }
    let mut fill = start.clone();
    let mut faces = vec![0u32; start[n] as usize];
    for (f, t) in tris.iter().enumerate() {
        for &v in t {
            faces[fill[v as usize] as usize] = f as u32;
            fill[v as usize] += 1;
        }
    }
    (start, faces)
}

/// Face `to` is reached from face `from` across faces that share a vertex,
/// all within `reach` of `centre`.
fn joined_near(
    pos: &[Vec3],
    tris: &[[u32; 3]],
    (start, faces): &(Vec<u32>, Vec<u32>),
    from: usize,
    to: usize,
    centre: Vec3,
    reach: f32,
) -> bool {
    let r2 = reach * reach;
    let near = |f: usize| {
        let [a, b, c] = tris[f].map(|v| pos[v as usize]);
        closest_on_triangle(centre, a, b, c).distance_squared(centre) <= r2
    };
    let mut seen = std::collections::HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(f) = stack.pop() {
        if f == to {
            return true;
        }
        for &v in &tris[f] {
            let (s, e) = (start[v as usize] as usize, start[v as usize + 1] as usize);
            for &g in &faces[s..e] {
                let g = g as usize;
                if near(g) && seen.insert(g) {
                    stack.push(g);
                }
            }
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A closed cube of side `s` at `o`, outward normals.
    pub(crate) fn cube(o: Vec3, s: f32) -> TriMesh {
        let p: Vec<Vec3> = [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (1.0, 1.0, 1.0),
            (0.0, 1.0, 1.0),
        ]
        .iter()
        .map(|&(x, y, z)| o + Vec3::new(x, y, z) * s)
        .collect();
        let t = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        TriMesh::new(p, t)
    }

    /// A box with a cubic cavity: a closed shell of the given wall.
    pub(crate) fn hollow(outer: f32, wall: f32) -> TriMesh {
        let a = cube(Vec3::ZERO, outer);
        let mut b = cube(Vec3::splat(wall), outer - 2.0 * wall);
        for t in &mut b.tris {
            t.swap(1, 2);
        }
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        TriMesh::new(pos, tris)
    }

    fn opts() -> CheckOptions {
        CheckOptions::default()
    }

    #[test]
    fn a_cube_is_ready_to_print() {
        let c = check(&cube(Vec3::ZERO, 10.0), &opts());
        assert!(c.ok, "{:?}", c.problems);
        assert_eq!((c.vertices, c.triangles, c.pieces), (8, 12, 1));
        assert_eq!(c.genus, Some(0));
        assert_eq!(c.self_intersections, Some(0));
        assert!((c.volume_mm3 - 1000.0).abs() < 1e-6);
        assert!((c.volume_ml() - 1.0).abs() < 1e-9);
        assert_eq!(c.size_mm, [10.0; 3]);
        assert_eq!(c.fits_plate, Some(true));
        assert!(c.problem_texts(Lang::En).is_empty());
    }

    #[test]
    fn unwelded_soups_are_welded_first() {
        let c = cube(Vec3::ZERO, 10.0);
        let soup = TriMesh::new(
            c.tris
                .iter()
                .flat_map(|t| t.map(|i| c.positions[i as usize]))
                .collect(),
            (0..12).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
        );
        let r = check(&soup, &opts());
        assert!(r.ok, "{:?}", r.problems);
        assert_eq!(r.vertices, 8);
    }

    #[test]
    fn a_hole_is_flagged() {
        let mut m = cube(Vec3::ZERO, 10.0);
        m.tris.pop();
        let c = check(&m, &opts());
        assert!(!c.ok);
        assert!(
            c.problems
                .iter()
                .any(|p| matches!(p, Problem::BoundaryEdges { count: 3 }))
        );
        assert_eq!(c.genus, None);
        assert!(
            c.problem_texts(Lang::It)
                .iter()
                .any(|t| t.contains("aperti"))
        );
    }

    #[test]
    fn inverted_normals_are_flagged() {
        let mut m = cube(Vec3::ZERO, 10.0);
        for t in &mut m.tris {
            t.swap(1, 2);
        }
        let c = check(&m, &opts());
        assert!(c.problems.contains(&Problem::NonPositiveVolume));
        assert!(c.volume_mm3 < 0.0);
        // one flipped triangle: an inconsistent orientation
        let mut m = cube(Vec3::ZERO, 10.0);
        m.tris[0].swap(1, 2);
        let c = check(&m, &opts());
        assert!(c.flipped_edges > 0 && !c.ok);
    }

    #[test]
    fn crossing_solids_are_flagged() {
        // two overlapping cubes in one mesh
        let a = cube(Vec3::ZERO, 10.0);
        let b = cube(Vec3::new(5.0, 5.0, 5.0), 10.0);
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        let c = check(&TriMesh::new(pos, tris), &opts());
        assert!(c.self_intersections.unwrap() > 0);
        assert!(!c.intersecting_pairs.is_empty());
        assert_eq!(c.pieces, 2);
        assert!(!c.ok);
        // two separate cubes: two pieces but no crossing
        let a = cube(Vec3::ZERO, 10.0);
        let b = cube(Vec3::new(20.0, 0.0, 0.0), 10.0);
        let off = a.positions.len() as u32;
        let mut pos = a.positions;
        pos.extend(b.positions);
        let mut tris = a.tris;
        tris.extend(b.tris.iter().map(|t| t.map(|i| i + off)));
        let c = check(&TriMesh::new(pos, tris), &opts());
        assert_eq!(c.self_intersections, Some(0));
        assert_eq!(c.pieces, 2);
    }

    #[test]
    fn a_cavity_is_not_a_hole_but_a_tunnel_is() {
        let c = check(&hollow(10.0, 2.0), &opts());
        // a cavity is a second piece of surface, not a handle
        assert_eq!(c.pieces, 2);
        assert_eq!(c.genus, Some(0));
        assert!((c.volume_mm3 - (1000.0 - 216.0)).abs() < 1e-6);
    }

    #[test]
    fn the_wall_is_measured_and_compared_with_the_minimum() {
        let shell = hollow(10.0, 0.5);
        let thin = check(
            &shell,
            &CheckOptions {
                min_thickness_mm: Some(1.0),
                ..opts()
            },
        );
        let t = thin.min_thickness_mm.unwrap();
        assert!((t - 0.5).abs() < 1e-3, "{t}");
        assert!(
            thin.problems
                .iter()
                .any(|p| matches!(p, Problem::TooThin { .. }))
        );
        let fine = check(
            &hollow(10.0, 2.0),
            &CheckOptions {
                min_thickness_mm: Some(1.0),
                ..opts()
            },
        );
        assert!(
            !fine
                .problems
                .iter()
                .any(|p| matches!(p, Problem::TooThin { .. })),
            "{:?}",
            fine.problems
        );
        assert!((fine.min_thickness_mm.unwrap() - 2.0).abs() < 1e-3);
    }

    /// A closed prism of the plan polygon `xz` (counter-clockwise, its cap
    /// triangles `cap`) extruded along `y` by `depth`.
    fn extruded(xz: &[[f32; 2]], cap: &[[u32; 3]], depth: f32) -> TriMesh {
        let k = xz.len() as u32;
        let mut positions: Vec<Vec3> = xz.iter().map(|p| Vec3::new(p[0], 0.0, p[1])).collect();
        positions.extend(xz.iter().map(|p| Vec3::new(p[0], depth, p[1])));
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for t in cap {
            tris.push(*t);
            tris.push([t[0] + k, t[2] + k, t[1] + k]);
        }
        for i in 0..k {
            let j = (i + 1) % k;
            tris.push([i, i + k, j + k]);
            tris.push([i, j + k, j]);
        }
        let m = TriMesh::new(positions, tris);
        let c = check(&m, &opts());
        if c.volume_mm3 < 0.0 {
            TriMesh::new(
                m.positions,
                m.tris.iter().map(|t| [t[0], t[2], t[1]]).collect(),
            )
        } else {
            m
        }
    }

    #[test]
    fn a_small_feature_is_no_thin_wall_but_a_thin_slab_is() {
        let thick = CheckOptions {
            min_thickness_mm: Some(0.3),
            ..opts()
        };
        // a block with a land 0.1 mm wide and 0.08 mm high on top (as between
        // the strokes of an engraved text)
        let xz = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [5.1, 10.0],
            [5.1, 10.08],
            [5.0, 10.08],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        let cap = [
            [0, 1, 2],
            [0, 2, 3],
            [0, 3, 6],
            [0, 6, 7],
            [6, 3, 4],
            [6, 4, 5],
        ];
        let land = check(&extruded(&xz, &cap, 10.0), &thick);
        assert!(land.ok, "{:?}", land.problems);
        assert!(land.min_thickness_mm.unwrap() > 1.0, "{land:?}");
        // a slab 0.2 mm thick: its faces are joined only far from the probes
        let slab = check(
            &extruded(
                &[[0.0, 0.0], [10.0, 0.0], [10.0, 0.2], [0.0, 0.2]],
                &[[0, 1, 2], [0, 2, 3]],
                10.0,
            ),
            &thick,
        );
        let t = slab.min_thickness_mm.unwrap();
        assert!((t - 0.2).abs() < 1e-3, "{t}");
        assert!(
            slab.problems
                .iter()
                .any(|p| matches!(p, Problem::TooThin { .. }))
        );
    }

    #[test]
    fn the_default_plate_is_the_saturn_inside_its_margin() {
        let p = DEFAULT_PLATE_MM;
        assert!(
            (p[0] - 203.68).abs() < 1e-9 && (p[1] - 110.37).abs() < 1e-9,
            "{p:?}"
        );
        assert_eq!(p[2], 220.0);
        // 115 mm fits a 145 mm Form 3 plate but not the 118.37 mm Saturn side
        // less its margins, in either turn
        let c = check(&cube(Vec3::ZERO, 115.0), &opts());
        assert_eq!(c.fits_plate, Some(false));
        let c = check(&cube(Vec3::ZERO, 105.0), &opts());
        assert_eq!(c.fits_plate, Some(true));
    }

    #[test]
    fn the_build_plate_limits_the_size() {
        let c = check(
            &cube(Vec3::ZERO, 100.0),
            &CheckOptions {
                build_plate_mm: Some([120.0, 90.0, 200.0]),
                ..opts()
            },
        );
        // 100 x 100 does not fit 120 x 90 in either orientation
        assert_eq!(c.fits_plate, Some(false));
        assert!(
            c.problems
                .iter()
                .any(|p| matches!(p, Problem::DoesNotFit { .. }))
        );
        let c = check(
            &cube(Vec3::ZERO, 100.0),
            &CheckOptions {
                build_plate_mm: None,
                ..opts()
            },
        );
        assert_eq!(c.fits_plate, None);
        assert!(c.ok);
    }

    #[test]
    fn degenerate_and_empty_meshes_are_flagged() {
        let c = check(&TriMesh::new(Vec::new(), Vec::new()), &opts());
        assert_eq!(c.problems, vec![Problem::Empty]);
        let mut m = cube(Vec3::ZERO, 10.0);
        m.positions.push(Vec3::new(20.0, 0.0, 0.0));
        m.positions.push(Vec3::new(30.0, 0.0, 0.0));
        m.positions.push(Vec3::new(40.0, 0.0, 0.0));
        m.tris.push([8, 9, 10]);
        let c = check(&m, &opts());
        assert_eq!(c.degenerate, 1);
        assert!(!c.ok);
    }

    #[test]
    fn the_triangle_test_handles_the_cases() {
        let pairs = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
            let positions = a.iter().chain(&b).map(|p| Vec3::from_array(*p)).collect();
            let (pos, tris) = weld(&TriMesh::new(positions, vec![[0, 1, 2], [3, 4, 5]]));
            self_intersections(&pos, &tris).0
        };
        let base = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
        // pierced by a triangle standing on it
        assert_eq!(
            pairs(base, [[1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0]]),
            1
        );
        // above it
        assert_eq!(
            pairs(base, [[1.0, 1.0, 1.0], [1.0, 1.0, 2.0], [2.0, 1.0, 2.0]]),
            0
        );
        // in the plane of the other but past its edge
        assert_eq!(
            pairs(base, [[3.0, 3.0, -1.0], [3.0, 3.0, 1.0], [4.0, 3.0, 1.0]]),
            0
        );
        // two triangles of a flat wall 0.1 mm apart, one tilted by rounding:
        // not a crossing (the interval test of Moller counted them)
        assert_eq!(
            pairs(
                [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]],
                [[4.1, 0.1, 2e-5], [0.1, 4.1, -2e-5], [4.1, 4.1, 1e-5]]
            ),
            0
        );
        // a thin wedge through the wall is one
        assert_eq!(
            pairs(
                base,
                [[1.0, 1.0, -0.01], [2.0, 1.0, 0.01], [1.0, 2.0, 0.01]]
            ),
            1
        );
    }

    /// A closed torus around the `z` axis through `centre` (radii `big`,
    /// `small`), outward faces.
    fn torus(centre: Vec3, big: f32, small: f32) -> TriMesh {
        let (nu, nv) = (48usize, 24usize);
        let mut positions = Vec::with_capacity(nu * nv);
        for i in 0..nu {
            let u = std::f32::consts::TAU * i as f32 / nu as f32;
            for j in 0..nv {
                let v = std::f32::consts::TAU * j as f32 / nv as f32;
                let r = big + small * v.cos();
                positions.push(centre + Vec3::new(r * u.cos(), r * u.sin(), small * v.sin()));
            }
        }
        let at = |i: usize, j: usize| ((i % nu) * nv + j % nv) as u32;
        let mut tris = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                tris.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                tris.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        TriMesh::new(positions, tris)
    }

    #[test]
    fn a_hole_through_the_wall_is_flagged_where_it_is() {
        let centre = Vec3::new(20.0, -5.0, 3.0);
        let c = check(&torus(centre, 3.0, 1.0), &opts());
        assert!(c.volume_mm3 > 0.0, "{c:?}");
        assert_eq!(c.genus, Some(1));
        let at = c
            .problems
            .iter()
            .find_map(|p| match p {
                Problem::Genus { count: 1, at } => Some(at.clone()),
                _ => None,
            })
            .expect("the hole is a problem");
        // one place, on the torus: the loop round its tube or round its hole
        assert_eq!(at.len(), 1, "{at:?}");
        let q = DVec3::from(at[0]) - centre.as_dvec3();
        let r = q.truncate().length();
        assert!(q.z.abs() < 0.5 && (r < 0.5 || (r - 3.0).abs() < 0.5), "{q}");
        let en = c.problem_texts(Lang::En).join("; ");
        let it = c.problem_texts(Lang::It).join("; ");
        let place = |lang, sep| {
            let c = at[0].map(|x| crate::export::html::num(lang, x, 1));
            format!("({})", c.join(sep))
        };
        // the singular, the decimal mark and the separator of each language
        assert!(
            en.contains(&format!(
                "1 hole through the wall, near {}",
                place(Lang::En, ", ")
            )),
            "{en}"
        );
        assert!(
            it.contains(&format!(
                "1 foro passante nella parete, vicino a {}",
                place(Lang::It, "; ")
            )),
            "{it}"
        );
        assert!(!it.contains("hole") && !it.contains("coordinates"), "{it}");
        // without the loops (none found) the count alone, in the plural
        let bare = Problem::Genus {
            count: 2,
            at: Vec::new(),
        };
        assert_eq!(bare.text(Lang::En), "2 holes through the wall");
        assert_eq!(bare.text(Lang::It), "2 fori passanti nella parete");
        let one = Problem::Genus {
            count: 1,
            at: Vec::new(),
        };
        assert_eq!(one.text(Lang::En), "1 hole through the wall");
        assert_eq!(one.text(Lang::It), "1 foro passante nella parete");
    }
}
