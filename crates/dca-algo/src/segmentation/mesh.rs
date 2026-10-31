//! Mesh helpers of the segmentation (`ortho_aligner/meshutil.py`): face
//! adjacency, edges, open border, connected components and the summation
//! order of numpy where the oracle's numbers depend on it.

use glam::DVec3;

/// No neighbour across a face edge (open border or non-manifold edge).
pub(crate) const NONE: u32 = u32::MAX;

/// Unit normal (zero for a degenerate face) and area of every triangle
/// (`meshutil.tri_normals`).
pub(crate) fn tri_normals(v: &[DVec3], t: &[[u32; 3]]) -> (Vec<DVec3>, Vec<f64>) {
    t.iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| v[i as usize]);
            let cr = (b - a).cross(c - a);
            let area2 = cr.length();
            let safe = if area2 > 0.0 { area2 } else { 1.0 };
            (cr / safe, 0.5 * area2)
        })
        .unzip()
}

/// Neighbour face across each edge `k = (t[k], t[(k + 1) % 3])`, [`NONE`]
/// for border and non-manifold edges (`meshutil.face_adjacency`).
pub(crate) fn face_adjacency(t: &[[u32; 3]], n_verts: usize) -> Vec<[u32; 3]> {
    let n = n_verts as u64;
    let key = |e: usize| {
        let f = t[e / 3];
        let (a, b) = (f[e % 3] as u64, f[(e % 3 + 1) % 3] as u64);
        a.min(b) * n + a.max(b)
    };
    let mut order: Vec<usize> = (0..t.len() * 3).collect();
    order.sort_by_key(|&e| key(e));
    let mut nbr = vec![[NONE; 3]; t.len()];
    let mut i = 0;
    while i < order.len() {
        let k = key(order[i]);
        let mut j = i + 1;
        while j < order.len() && key(order[j]) == k {
            j += 1;
        }
        if j - i == 2 {
            let (a, b) = (order[i], order[i + 1]);
            nbr[a / 3][a % 3] = (b / 3) as u32;
            nbr[b / 3][b % 3] = (a / 3) as u32;
        }
        i = j;
    }
    nbr
}

/// Undirected edges of the triangles, each once, sorted by `(min, max)`
/// (`meshutil.unique_edges`).
pub(crate) fn unique_edges(t: &[[u32; 3]]) -> Vec<[u32; 2]> {
    let mut e: Vec<[u32; 2]> = t
        .iter()
        .flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])].map(|(a, b)| [a.min(b), a.max(b)]))
        .collect();
    e.sort_unstable();
    e.dedup();
    e
}

/// Vertices on the open border (an edge without exactly one neighbour face)
/// (`meshutil.boundary_vertex_mask`).
pub(crate) fn boundary_vertex_mask(t: &[[u32; 3]], nbr: &[[u32; 3]], n_verts: usize) -> Vec<bool> {
    let mut mask = vec![false; n_verts];
    for (f, nb) in t.iter().zip(nbr) {
        for k in 0..3 {
            if nb[k] == NONE {
                mask[f[k] as usize] = true;
                mask[f[(k + 1) % 3] as usize] = true;
            }
        }
    }
    mask
}

/// Connected components of `n` nodes joined by `edges`: the root of every
/// node is the smallest node index of its component, as the vectorised
/// union-find of `meshutil.connected_components`.
pub(crate) fn components(n: usize, edges: impl IntoIterator<Item = (usize, usize)>) -> Vec<usize> {
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut parent: Vec<usize> = (0..n).collect();
    for (a, b) in edges {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            parent[hi] = lo;
        }
    }
    (0..n).map(|i| find(&mut parent, i)).collect()
}

/// Edge-connected pieces of the face graph: root of every face
/// (`meshutil.face_components`).
pub(crate) fn face_components(nbr: &[[u32; 3]]) -> Vec<usize> {
    components(
        nbr.len(),
        nbr.iter().enumerate().flat_map(|(f, nb)| {
            nb.iter()
                .filter(|&&g| g != NONE)
                .map(move |&g| (f, g as usize))
        }),
    )
}

/// Sum of a contiguous array in the order of numpy's pairwise summation
/// (`np.sum` / `np.mean` of a 1-D float64 array): blocks of 128 summed with
/// eight accumulators, larger arrays split in halves.
pub(crate) fn np_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &x in a {
            res += x;
        }
        res
    } else if n <= 128 {
        let mut r = [0.0f64; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i < n - n % 8 {
            for (j, rj) in r.iter_mut().enumerate() {
                *rj += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        np_sum(&a[..n2]) + np_sum(&a[n2..])
    }
}

/// Neighbour lists of the vertices from undirected edges (CSR: start
/// offsets and neighbours, in edge order).
pub(crate) fn vertex_neighbours(n: usize, edges: &[[u32; 2]]) -> (Vec<usize>, Vec<u32>) {
    let mut start = vec![0usize; n + 1];
    for e in edges {
        start[e[0] as usize + 1] += 1;
        start[e[1] as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut nb = vec![0u32; start[n]];
    for e in edges {
        let (a, b) = (e[0] as usize, e[1] as usize);
        nb[fill[a]] = e[1];
        fill[a] += 1;
        nb[fill[b]] = e[0];
        fill[b] += 1;
    }
    (start, nb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairwise_sum_matches_plain_sum_on_exact_values() {
        let a: Vec<f64> = (0..1000).map(f64::from).collect();
        assert_eq!(np_sum(&a), 499_500.0);
        assert_eq!(np_sum(&a[..5]), 10.0);
        assert_eq!(np_sum(&[]), 0.0);
    }

    #[test]
    fn adjacency_of_two_triangles() {
        let t = [[0, 1, 2], [2, 1, 3]];
        let nbr = face_adjacency(&t, 4);
        assert_eq!(nbr[0], [NONE, 1, NONE]);
        assert_eq!(nbr[1], [0, NONE, NONE]);
        let border = boundary_vertex_mask(&t, &nbr, 4);
        assert!(border.iter().all(|&b| b));
        assert_eq!(face_components(&nbr), vec![0, 0]);
        assert_eq!(
            unique_edges(&t),
            vec![[0, 1], [0, 2], [1, 2], [1, 3], [2, 3]]
        );
    }

    #[test]
    fn components_take_the_smallest_index_as_root() {
        let roots = components(6, [(5, 3), (3, 4), (1, 2)]);
        assert_eq!(roots, vec![0, 1, 1, 3, 3, 3]);
    }
}
