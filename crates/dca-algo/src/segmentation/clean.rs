//! Clean-up of vertex labels at full resolution
//! (`segmentation.clean_vertex_labels`, `face_labels`; `meshutil.majority_filter`,
//! `label_components`, `fill_unassigned`).

use std::collections::BTreeMap;

use super::mesh;

/// Each label replaced by the most common one among itself (weight 2) and
/// its 1-ring, `iterations` times; ties to the smallest label.
fn majority_filter(labels: &[i32], edges: &[[u32; 2]], iterations: usize) -> Vec<i32> {
    let mut uniq: Vec<i32> = labels.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    let k = uniq.len();
    let mut inv: Vec<usize> = labels
        .iter()
        .map(|l| uniq.binary_search(l).unwrap_or(0))
        .collect();
    let n = labels.len();
    let mut counts = vec![0u32; n * k];
    for _ in 0..iterations {
        counts.iter_mut().for_each(|c| *c = 0);
        for (i, &l) in inv.iter().enumerate() {
            counts[i * k + l] += 2;
        }
        for e in edges {
            let (a, b) = (e[0] as usize, e[1] as usize);
            counts[a * k + inv[b]] += 1;
            counts[b * k + inv[a]] += 1;
        }
        for (i, l) in inv.iter_mut().enumerate() {
            let row = &counts[i * k..(i + 1) * k];
            let mut best = 0;
            for j in 1..k {
                if row[j] > row[best] {
                    best = j;
                }
            }
            *l = best;
        }
    }
    inv.iter().map(|&i| uniq[i]).collect()
}

/// Unassigned vertices (`unassigned`) repeatedly given the majority label of
/// their assigned neighbours (ties to the smallest label).
fn fill_unassigned(labels: &[i32], edges: &[[u32; 2]], unassigned: i32) -> Vec<i32> {
    const MAX_ITER: usize = 500;
    let mut lab = labels.to_vec();
    for _ in 0..MAX_ITER {
        let un: Vec<bool> = lab.iter().map(|&l| l == unassigned).collect();
        if !un.iter().any(|&u| u) {
            break;
        }
        // votes per target vertex: label -> count
        let mut votes: BTreeMap<usize, BTreeMap<i32, u32>> = BTreeMap::new();
        for e in edges {
            let (a, b) = (e[0] as usize, e[1] as usize);
            if un[a] && !un[b] {
                *votes.entry(a).or_default().entry(lab[b]).or_default() += 1;
            }
            if un[b] && !un[a] {
                *votes.entry(b).or_default().entry(lab[a]).or_default() += 1;
            }
        }
        if votes.is_empty() {
            break;
        }
        for (v, by) in votes {
            // first maximum in label order
            let mut best: Option<(i32, u32)> = None;
            for (l, c) in by {
                if best.is_none_or(|(_, bc)| c > bc) {
                    best = Some((l, c));
                }
            }
            if let Some((l, _)) = best {
                lab[v] = l;
            }
        }
    }
    lab
}

/// Label of every face: the label shared by at least two of its vertices,
/// else that of the second vertex.
pub(crate) fn face_labels(t: &[[u32; 3]], labels: &[u16]) -> Vec<u16> {
    t.iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| labels[i as usize]);
            if a == b || a == c { a } else { b }
        })
        .collect()
}

/// Majority filter (2 iterations by default), then one connected component
/// per label: a tooth keeps the component of its seed vertex (when the seed
/// carries the tooth's label), else the largest; the rest is refilled from
/// the neighbours.
pub(crate) fn clean_vertex_labels(
    labels: &[u16],
    edges: &[[u32; 2]],
    seed_vertex: &BTreeMap<u16, usize>,
    smooth_iterations: usize,
) -> Vec<u16> {
    let lab0: Vec<i32> = labels.iter().map(|&l| i32::from(l)).collect();
    let lab = majority_filter(&lab0, edges, smooth_iterations);
    let comp = mesh::components(
        lab.len(),
        edges
            .iter()
            .filter(|e| lab[e[0] as usize] == lab[e[1] as usize])
            .map(|e| (e[0] as usize, e[1] as usize)),
    );
    // per label: size of every component (root -> count)
    let mut sizes: BTreeMap<i32, BTreeMap<usize, usize>> = BTreeMap::new();
    for (i, &l) in lab.iter().enumerate() {
        *sizes.entry(l).or_default().entry(comp[i]).or_default() += 1;
    }
    let mut keep: BTreeMap<i32, usize> = BTreeMap::new();
    for (&l, roots) in &sizes {
        let mut best: Option<(usize, usize)> = None;
        for (&r, &c) in roots {
            if best.is_none_or(|(_, bc)| c > bc) {
                best = Some((r, c));
            }
        }
        let mut k = best.map_or(0, |b| b.0);
        if l != 0
            && let Ok(f) = u16::try_from(l)
            && let Some(&sv) = seed_vertex.get(&f)
            && lab[sv] == l
        {
            k = comp[sv];
        }
        keep.insert(l, k);
    }
    let out: Vec<i32> = lab
        .iter()
        .enumerate()
        .map(|(i, &l)| if keep[&l] == comp[i] { l } else { -1 })
        .collect();
    fill_unassigned(&out, edges, -1)
        .into_iter()
        .map(|l| u16::try_from(l).unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_island_is_absorbed() {
        // path 0-1-2-3-4-5-6: a second piece of tooth 11 on 3, a small piece
        // of gingiva on 2; each label keeps one component, the rest is refilled
        let edges: Vec<[u32; 2]> = (0..6).map(|i| [i, i + 1]).collect();
        let labels = [11, 11, 0, 11, 0, 0, 0];
        let seeds = BTreeMap::from([(11u16, 0usize)]);
        let out = clean_vertex_labels(&labels, &edges, &seeds, 0);
        assert_eq!(out, vec![11, 11, 11, 0, 0, 0, 0]);
    }

    #[test]
    fn face_label_is_the_majority_of_its_corners() {
        let t = [[0, 1, 2], [3, 4, 5]];
        assert_eq!(face_labels(&t, &[1, 2, 1, 3, 4, 5]), vec![1, 4]);
    }
}
