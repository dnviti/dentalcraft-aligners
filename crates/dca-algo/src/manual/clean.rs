//! Clean-up of the tooth labels after the brush: islands removed, holes of
//! gingiva inside a crown filled and the jagged border smoothed
//! (`manual.clean_labels`).

use std::collections::BTreeMap;

use serde::Serialize;

use super::Adjacency;

/// A piece of a tooth smaller than this share of its main piece is an island.
pub const ISLAND_SHARE: f64 = 0.25;
/// A piece of gingiva smaller than this share of the gingiva is a hole in a
/// crown.
pub const GUM_HOLE_SHARE: f64 = 0.02;

/// What [`clean_labels`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct CleanReport {
    /// Islands (and holes) given the label of their surroundings.
    pub islands: usize,
    /// Vertices whose label changed.
    pub vertices: usize,
}

/// Connected pieces of equal label.
fn label_components(labels: &[i32], adj: &Adjacency) -> Vec<u32> {
    let n = labels.len();
    let mut comp = vec![u32::MAX; n];
    let mut next = 0;
    for s in 0..n {
        if comp[s] != u32::MAX {
            continue;
        }
        let mut stack = vec![s as u32];
        comp[s] = next;
        while let Some(u) = stack.pop() {
            for &v in adj.neighbors(u) {
                if comp[v as usize] == u32::MAX && labels[v as usize] == labels[s] {
                    comp[v as usize] = next;
                    stack.push(v);
                }
            }
        }
        next += 1;
    }
    comp
}

/// The unassigned (`-1`) vertices take, ring by ring, the label most of
/// their assigned neighbours have (`meshutil.fill_unassigned`).
fn fill_unassigned(labels: &mut [i32], adj: &Adjacency) {
    loop {
        let mut changed = Vec::new();
        for v in 0..labels.len() {
            if labels[v] != -1 {
                continue;
            }
            let mut count: BTreeMap<i32, usize> = BTreeMap::new();
            for &u in adj.neighbors(v as u32) {
                if labels[u as usize] >= 0 {
                    *count.entry(labels[u as usize]).or_insert(0) += 1;
                }
            }
            // the most frequent label, the smallest on ties
            if let Some((&l, _)) = count.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))) {
                changed.push((v, l));
            }
        }
        if changed.is_empty() {
            break;
        }
        for (v, l) in changed {
            labels[v] = l;
        }
    }
}

/// Each vertex takes the label most common among itself and its neighbours
/// (its own on ties), `iterations` times (`meshutil.majority_filter`).
fn majority_filter(labels: &[i32], adj: &Adjacency, iterations: usize) -> Vec<i32> {
    let mut cur = labels.to_vec();
    for _ in 0..iterations {
        let prev = cur.clone();
        for v in 0..prev.len() {
            let mut count: BTreeMap<i32, usize> = BTreeMap::new();
            *count.entry(prev[v]).or_insert(0) += 1;
            for &u in adj.neighbors(v as u32) {
                *count.entry(prev[u as usize]).or_insert(0) += 1;
            }
            let best = count.values().copied().max().unwrap_or(0);
            if count[&prev[v]] < best {
                cur[v] = *count
                    .iter()
                    .find(|(_, c)| **c == best)
                    .map(|(l, _)| l)
                    .unwrap_or(&prev[v]);
            }
        }
    }
    cur
}

/// Cleans the labels of `teeth` (all the teeth when `None`): an island (a
/// piece of a tooth smaller than [`ISLAND_SHARE`] of its main piece, or a
/// small piece of gingiva enclosed by one tooth) takes the label of its
/// surroundings, then the border is smoothed by `rings` majority filters
/// where a cleaned tooth is involved. `seed_vertex` keeps, for a tooth, the
/// piece that holds its seed vertex as the main piece.
pub fn clean_labels(
    labels: &[u16],
    adj: &Adjacency,
    teeth: Option<&[u16]>,
    rings: usize,
    seed_vertex: &BTreeMap<u16, u32>,
) -> (Vec<u16>, CleanReport) {
    let lab: Vec<i32> = labels.iter().map(|&l| i32::from(l)).collect();
    let mut out = lab.clone();
    let present: Vec<i32> = {
        let mut p: Vec<i32> = lab.iter().copied().filter(|&l| l != 0).collect();
        p.sort_unstable();
        p.dedup();
        p
    };
    let teeth: Vec<i32> = match teeth {
        None => present.clone(),
        Some(t) => t
            .iter()
            .map(|&f| i32::from(f))
            .filter(|f| present.contains(f))
            .collect(),
    };
    let comp = label_components(&out, adj);
    let mut islands = 0;
    for &f in &teeth {
        let sel: Vec<usize> = (0..out.len()).filter(|&i| out[i] == f).collect();
        let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
        for &i in &sel {
            *counts.entry(comp[i]).or_insert(0) += 1;
        }
        // the largest piece, the one of the seed when it has the label
        let mut keep = *counts
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(r, _)| r)
            .unwrap_or(&0);
        if let Some(&sv) = u16::try_from(f).ok().and_then(|k| seed_vertex.get(&k))
            && lab[sv as usize] == f
        {
            keep = comp[sv as usize];
        }
        let main = counts.get(&keep).copied().unwrap_or(0) as f64;
        let small: Vec<u32> = counts
            .iter()
            .filter(|(r, c)| **r != keep && (**c as f64) < ISLAND_SHARE * main)
            .map(|(r, _)| *r)
            .collect();
        if !small.is_empty() {
            for &i in &sel {
                if small.contains(&comp[i]) {
                    out[i] = -1;
                }
            }
            islands += small.len();
        }
    }
    // holes: small pieces of gingiva inside one crown (a piece between two teeth is an
    // interdental papilla cut off by the contact: kept)
    let mut gum_counts: BTreeMap<u32, usize> = BTreeMap::new();
    for i in 0..out.len() {
        if out[i] == 0 {
            *gum_counts.entry(comp[i]).or_insert(0) += 1;
        }
    }
    let big = gum_counts.values().copied().max().unwrap_or(0) as f64;
    for (&r, &c) in &gum_counts {
        if (c as f64) >= GUM_HOLE_SHARE * big {
            continue;
        }
        let piece: Vec<usize> = (0..out.len())
            .filter(|&i| out[i] == 0 && comp[i] == r)
            .collect();
        let mut around: Vec<i32> = Vec::new();
        for &v in &piece {
            for &u in adj.neighbors(v as u32) {
                let l = out[u as usize];
                if comp[u as usize] != r && l > 0 && !around.contains(&l) {
                    around.push(l);
                }
            }
        }
        if around.len() == 1 && teeth.contains(&around[0]) {
            for &v in &piece {
                out[v] = -1;
            }
            islands += 1;
        }
    }
    fill_unassigned(&mut out, adj);
    for l in &mut out {
        if *l < 0 {
            *l = 0;
        }
    }
    // jagged border: majority of the 1-ring, only where one of the teeth is involved
    if rings > 0 && !teeth.is_empty() {
        let sm = majority_filter(&out, adj, rings);
        for i in 0..out.len() {
            if teeth.contains(&out[i]) || teeth.contains(&sm[i]) {
                out[i] = sm[i];
            }
        }
    }
    let new: Vec<u16> = out.iter().map(|&l| l as u16).collect();
    let vertices = new.iter().zip(labels).filter(|(a, b)| a != b).count();
    (new, CleanReport { islands, vertices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manual::tests::grid;

    /// 24 x 24 grid: tooth 36 is the block `[6, 16)`, with a one-vertex hole in it,
    /// and a two-vertex island of 36 in the corner.
    fn labelled() -> (Adjacency, Vec<u16>) {
        let (v, t) = grid(24);
        let adj = Adjacency::new(v.len(), &t);
        let at = |x: usize, y: usize| y * 24 + x;
        let mut l = vec![0u16; v.len()];
        for y in 6..16 {
            for x in 6..16 {
                l[at(x, y)] = 36;
            }
        }
        l[at(10, 10)] = 0; // hole
        l[at(1, 1)] = 36; // island
        l[at(2, 1)] = 36;
        (adj, l)
    }

    #[test]
    fn islands_and_holes_take_the_label_of_their_surroundings() {
        let (adj, l) = labelled();
        let (clean, rep) = clean_labels(&l, &adj, None, 0, &BTreeMap::new());
        let at = |x: usize, y: usize| y * 24 + x;
        assert_eq!(clean[at(10, 10)], 36, "hole filled");
        assert_eq!(clean[at(1, 1)], 0, "island removed");
        assert_eq!(clean[at(2, 1)], 0);
        assert_eq!(rep.islands, 2);
        assert_eq!(rep.vertices, 3);
        // the rest is untouched
        let changed = clean.iter().zip(&l).filter(|(a, b)| a != b).count();
        assert_eq!(changed, 3);
        // only the teeth asked for
        let (only, _) = clean_labels(&l, &adj, Some(&[11]), 0, &BTreeMap::new());
        assert_eq!(only, l, "tooth 11 is not there");
        let (only, rep) = clean_labels(&l, &adj, Some(&[36]), 0, &BTreeMap::new());
        assert_eq!(only, clean);
        assert_eq!(rep.islands, 2);
    }

    #[test]
    fn the_piece_with_the_seed_is_the_tooth() {
        let (adj, mut l) = labelled();
        let at = |x: usize, y: usize| y * 24 + x;
        // a second piece of tooth 36 beside the block: 2 x 10 vertices, a fifth of its size
        for y in 6..16 {
            for x in 18..20 {
                l[at(x, y)] = 36;
            }
        }
        let (clean, _) = clean_labels(&l, &adj, None, 0, &BTreeMap::new());
        assert_eq!(clean[at(19, 8)], 0, "the small piece is an island");
        assert_eq!(clean[at(8, 8)], 36);
        // the seed is on the small piece: it is the tooth, and the block is not an island
        let seeds = BTreeMap::from([(36u16, at(19, 8) as u32)]);
        let (clean, _) = clean_labels(&l, &adj, None, 0, &seeds);
        assert_eq!(clean[at(19, 8)], 36);
        assert_eq!(clean[at(8, 8)], 36);
    }

    #[test]
    fn smoothing_rounds_a_jagged_border() {
        let (adj, mut l) = labelled();
        let at = |x: usize, y: usize| y * 24 + x;
        l[at(10, 10)] = 36;
        l[at(1, 1)] = 0;
        l[at(2, 1)] = 0;
        // a spike of the tooth into the gingiva
        l[at(16, 10)] = 36;
        let (flat, _) = clean_labels(&l, &adj, None, 1, &BTreeMap::new());
        assert_eq!(flat[at(16, 10)], 0, "the spike is smoothed away");
        assert_eq!(flat[at(10, 10)], 36);
    }
}
