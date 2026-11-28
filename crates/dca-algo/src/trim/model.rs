//! The trim inputs of a labelled model in arch coordinates: the closed
//! teeth, the gingiva, the visible crowns, the gingival margin of every
//! tooth and the untrimmed shell field.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_geom::sdf::{SdfGrid, SdfParams};
use dc_mesh::TriMesh;
use glam::{DVec3, Vec3};

use super::{Tooth, TrimError, geom};

/// What [`super::Input`] takes of a labelled model.
#[derive(Debug, Clone)]
pub struct ModelParts {
    /// The triangles of the teeth.
    pub teeth_mesh: TriMesh,
    /// The triangles of the gingiva.
    pub gingiva: TriMesh,
    /// The visible crowns: the tooth triangles outside the gingiva.
    pub crowns: TriMesh,
    /// The teeth, in FDI order.
    pub teeth: Vec<Tooth>,
    /// Vertices on the open border of the model (empty when closed).
    pub scan_border: Vec<DVec3>,
}

fn sub_mesh(v: &[DVec3], tris: &[[u32; 3]], keep: impl Fn(&[u32; 3]) -> bool) -> TriMesh {
    let mut map = vec![u32::MAX; v.len()];
    let mut pos = Vec::new();
    let tris = tris
        .iter()
        .filter(|t| keep(t))
        .map(|t| {
            t.map(|i| {
                let m = &mut map[i as usize];
                if *m == u32::MAX {
                    *m = pos.len() as u32;
                    pos.push(v[i as usize].as_vec3());
                }
                *m
            })
        })
        .collect();
    TriMesh::new(pos, tris)
}

/// Edges used by one triangle only.
fn open_edges(tris: &[[u32; 3]]) -> Vec<(u32, u32)> {
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut e: Vec<(u32, u32)> = count
        .into_iter()
        .filter(|&(_, c)| c == 1)
        .map(|(e, _)| e)
        .collect();
    e.sort_unstable();
    e
}

/// `true` for a point inside the closed mesh of `bvh` (crossings above it).
fn inside(bvh: &Bvh, p: Vec3) -> bool {
    bvh.ray_hits(p, Vec3::Z, f32::INFINITY, 256, 1e-5).len() % 2 == 1
}

/// The trim inputs of a labelled model (vertices in arch coordinates, `+Z`
/// occlusal, labels the FDI number of every vertex, 0 the gingiva).
///
/// With a closed gingiva (a model made of closed parts, as the demo arch)
/// the visible crown of a tooth is what lies outside the gingiva and its
/// margin is where its edges cross the gingiva; with an open one (a
/// scan) the whole tooth is crown and the margin is its vertices next to
/// the gingiva. `frames` gives the origin and the buccal axis of the teeth
/// (default: the centroid of the crown and the plan direction away from the
/// centre of the arch); `covered` the teeth the aligner covers (default
/// all).
pub fn parts(
    vertices: &[DVec3],
    tris: &[[u32; 3]],
    labels: &[u16],
    frames: &BTreeMap<Fdi, (DVec3, DVec3)>,
    covered: Option<&BTreeSet<Fdi>>,
) -> ModelParts {
    let gum = |i: u32| labels[i as usize] == 0;
    let gingiva = sub_mesh(vertices, tris, |t| t.iter().all(|&i| gum(i)));
    let teeth_mesh = sub_mesh(vertices, tris, |t| t.iter().all(|&i| !gum(i)));
    let gum_tris: Vec<[u32; 3]> = tris
        .iter()
        .copied()
        .filter(|t| t.iter().all(|&i| gum(i)))
        .collect();
    let closed_gum = !gum_tris.is_empty() && open_edges(&gum_tris).is_empty();
    let gb = closed_gum.then(|| Bvh::build(&gingiva).ok()).flatten();
    let visible: Vec<bool> = vertices
        .iter()
        .zip(labels)
        .map(|(p, &l)| l != 0 && gb.as_ref().is_none_or(|b| !inside(b, p.as_vec3())))
        .collect();
    let crowns = sub_mesh(vertices, tris, |t| t.iter().all(|&i| visible[i as usize]));
    let mut margin: BTreeMap<u16, Vec<DVec3>> = BTreeMap::new();
    let mut next_to_gum: BTreeMap<u16, BTreeSet<usize>> = BTreeMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
            let l = labels[a];
            if l == 0 {
                continue;
            }
            match &gb {
                Some(g) if visible[a] && !visible[b] && labels[b] == l => {
                    let (pa, pb) = (vertices[a], vertices[b]);
                    let len = (pb - pa).length();
                    if len > 0.0
                        && let Some(h) =
                            g.ray_cast(pa.as_vec3(), ((pb - pa) / len).as_vec3(), len as f32)
                    {
                        margin
                            .entry(l)
                            .or_default()
                            .push(pa + (pb - pa) / len * f64::from(h.t));
                    }
                }
                None if labels[b] == 0 => {
                    next_to_gum.entry(l).or_default().insert(a);
                }
                _ => {}
            }
        }
    }
    for (l, ix) in next_to_gum {
        margin.insert(l, ix.into_iter().map(|i| vertices[i]).collect());
    }
    let mut per: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for (i, &l) in labels.iter().enumerate() {
        if l != 0 {
            per.entry(l).or_default().push(i);
        }
    }
    let centroid =
        |ix: &[usize]| ix.iter().map(|&i| vertices[i]).sum::<DVec3>() / ix.len().max(1) as f64;
    let mid = per.values().map(|ix| centroid(ix)).sum::<DVec3>() / per.len().max(1) as f64;
    let mut teeth = Vec::new();
    for (&l, ix) in &per {
        let Ok(fdi) = u8::try_from(l)
            .map_err(|_| ())
            .and_then(|f| Fdi::new(f).map_err(|_| ()))
        else {
            continue;
        };
        let vis: Vec<usize> = ix.iter().copied().filter(|&i| visible[i]).collect();
        let crown: Vec<DVec3> = vis.iter().map(|&i| vertices[i]).collect();
        let c = centroid(if vis.is_empty() { ix } else { &vis });
        let (origin, e_bl) = frames
            .get(&fdi)
            .copied()
            .unwrap_or_else(|| (c, (c - mid).with_z(0.0).normalize_or(DVec3::Y)));
        let m = margin.remove(&l).unwrap_or_default();
        teeth.push(Tooth {
            fdi,
            crown,
            closed: ix.iter().map(|&i| vertices[i]).collect(),
            margin: m,
            origin,
            e_bl,
            covered: covered.is_none_or(|c| c.contains(&fdi)),
        });
    }
    let mut border: Vec<u32> = open_edges(tris)
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .collect();
    border.sort_unstable();
    border.dedup();
    ModelParts {
        teeth_mesh,
        gingiva,
        crowns,
        teeth,
        scan_border: border.into_iter().map(|i| vertices[i as usize]).collect(),
    }
}

/// The untrimmed shell field of a closed model: the closing by `blockout`
/// of its field (crevices narrower than twice it filled, cavities filled),
/// with the region within `gum.1` of the gingiva `gum.0` added
/// ([`crate::shell::gum_relieved`]: the shell stands that far off the
/// gingiva), then the material between the offsets at `clearance` and
/// `clearance + thickness` (`max(S - outer, inner - S)`). A stand-in for the
/// field of the shell stage, with the same operations, for a model that is
/// already one closed solid (or a union of closed parts).
///
/// # Errors
/// [`TrimError::Geom`] when a distance field operation fails.
pub fn untrimmed_shell(
    model: &TriMesh,
    voxel: f64,
    blockout: f64,
    clearance: f64,
    thickness: f64,
    gum: Option<(&TriMesh, f64)>,
) -> Result<SdfGrid, TrimError> {
    let band = ((blockout / voxel).ceil() + 3.0) * voxel;
    let mut closed = SdfGrid::from_mesh(
        model,
        &SdfParams {
            voxel_size: voxel as f32,
            band: band as f32,
        },
    )
    .map_err(geom)?;
    if blockout > 0.0 {
        closed.close(blockout as f32).map_err(geom)?;
    }
    closed.fill_interior().map_err(geom)?;
    if let Some((gum, gap)) = gum
        && let Some(relieved) = crate::shell::gum_relieved(&closed, gum, gap).map_err(geom)?
    {
        closed = relieved;
    }
    let mut shell = closed.clone();
    shell.offset((clearance + thickness) as f32).map_err(geom)?;
    if clearance > 0.0 {
        closed.offset(clearance as f32).map_err(geom)?;
    }
    shell.subtract(&closed).map_err(geom)?;
    Ok(shell)
}
