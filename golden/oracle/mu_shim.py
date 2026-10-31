"""numpy stand-in for the parts of Blender's `mathutils` the pipeline modules use,
so that `gpu_query`, `meshutil` and `teeth` run in system python.

Covered: `mathutils.Vector` (a thin tuple), `bvhtree.BVHTree` (FromPolygons,
find_nearest, find_nearest_range, ray_cast), `kdtree.KDTree` (insert, balance,
find, find_range, find_n).  Everything is computed in float64 and exact (the real
module works in float32), so results can differ from Blender's by float32
rounding, ~1e-5 mm, far below the golden tolerances; ties go to the lowest index.
Not covered (NotImplementedError): BVHTree.overlap, Matrix and the rest.
"""
import sys
import types

import numpy as np


class Vector(tuple):
    def __new__(cls, seq=(0.0, 0.0, 0.0)):
        return super().__new__(cls, (float(x) for x in seq))

    @property
    def length(self):
        return float(np.linalg.norm(self))


def closest_on_tris(P, A, B, C):
    """Closest point of triangles (A, B, C: (n, 3)) to points P ((n, 3) or (3,)); Ericson 5.1.5."""
    with np.errstate(all="ignore"):
        return _closest_on_tris(P, A, B, C)


def _closest_on_tris(P, A, B, C):
    P = np.broadcast_to(P, A.shape)
    ab, ac, ap = B - A, C - A, P - A
    d1, d2 = np.einsum("ij,ij->i", ab, ap), np.einsum("ij,ij->i", ac, ap)
    bp = P - B
    d3, d4 = np.einsum("ij,ij->i", ab, bp), np.einsum("ij,ij->i", ac, bp)
    cp = P - C
    d5, d6 = np.einsum("ij,ij->i", ab, cp), np.einsum("ij,ij->i", ac, cp)
    vc = d1 * d4 - d3 * d2
    vb = d5 * d2 - d1 * d6
    va = d3 * d6 - d5 * d4
    denom = va + vb + vc
    v_in = vb / denom
    w_in = vc / denom
    t_ab = d1 / (d1 - d3)
    t_ac = d2 / (d2 - d6)
    t_bc = (d4 - d3) / ((d4 - d3) + (d5 - d6))
    out = A + ab * v_in[:, None] + ac * w_in[:, None]            # interior
    m = (vc <= 0) & (d1 >= 0) & (d3 <= 0)                      # edge AB
    out[m] = (A + ab * t_ab[:, None])[m]
    m = (vb <= 0) & (d2 >= 0) & (d6 <= 0)                       # edge AC
    out[m] = (A + ac * t_ac[:, None])[m]
    m = (va <= 0) & ((d4 - d3) >= 0) & ((d5 - d6) >= 0)         # edge BC
    out[m] = (B + (C - B) * t_bc[:, None])[m]
    m = (d6 >= 0) & (d5 <= d6)                                  # vertex C
    out[m] = C[m]
    m = (d3 >= 0) & (d4 <= d3)                                  # vertex B
    out[m] = B[m]
    m = (d1 <= 0) & (d2 <= 0)                                   # vertex A
    out[m] = A[m]
    return out


class BVHTree:
    """Uniform-grid triangle index with exact float64 queries."""

    def __init__(self, V, T):
        self.V = np.asarray(V, np.float64).reshape(-1, 3)
        self.T = np.asarray(T, np.int64).reshape(-1, 3)
        self.A, self.B, self.C = (self.V[self.T[:, i]] for i in range(3))
        self.lo = np.minimum(np.minimum(self.A, self.B), self.C)
        self.hi = np.maximum(np.maximum(self.A, self.B), self.C)
        n = max(len(self.T), 1)
        ext = (self.hi.max(axis=0) - self.lo.min(axis=0)) if len(self.T) else np.ones(3)
        vol = float(np.prod(np.maximum(ext, 1e-6)))
        self.h = max((vol / max(n / 4.0, 1.0)) ** (1.0 / 3.0), 1e-3)     # about 4 triangles per cell
        self.o = self.lo.min(axis=0) if len(self.T) else np.zeros(3)
        self.cells = {}
        if len(self.T):
            c0 = np.floor((self.lo - self.o) / self.h).astype(np.int64)
            c1 = np.floor((self.hi - self.o) / self.h).astype(np.int64)
            for f in range(len(self.T)):
                for x in range(c0[f, 0], c1[f, 0] + 1):
                    for y in range(c0[f, 1], c1[f, 1] + 1):
                        for z in range(c0[f, 2], c1[f, 2] + 1):
                            self.cells.setdefault((x, y, z), []).append(f)
            self.cells = {k: np.asarray(v, np.int64) for k, v in self.cells.items()}

    @classmethod
    def FromPolygons(cls, vertices, polygons, all_triangles=False, epsilon=0.0):
        T = np.asarray(polygons, np.int64)
        if T.ndim != 2 or T.shape[1] != 3:
            raise NotImplementedError("only triangle meshes")
        return cls(vertices, T)

    def _candidates(self, p, r):
        """Faces whose grid cells meet the cube of half-size r around p (a superset of the faces within r)."""
        a = np.floor((np.asarray(p, np.float64) - r - self.o) / self.h).astype(np.int64)
        b = np.floor((np.asarray(p, np.float64) + r - self.o) / self.h).astype(np.int64)
        if np.prod((b - a + 1).astype(np.float64)) > len(self.cells):
            idx = np.arange(len(self.T))
        else:
            parts = [self.cells[(x, y, z)] for x in range(a[0], b[0] + 1) for y in range(a[1], b[1] + 1)
                     for z in range(a[2], b[2] + 1) if (x, y, z) in self.cells]
            idx = np.unique(np.concatenate(parts)) if parts else np.zeros(0, np.int64)
        if len(idx):
            p = np.asarray(p, np.float64)
            idx = idx[np.all((self.lo[idx] <= p + r) & (self.hi[idx] >= p - r), axis=1)]
        return idx

    def _dist(self, p, idx):
        p = np.asarray(p, np.float64)
        q = closest_on_tris(p, self.A[idx], self.B[idx], self.C[idx])
        return q, np.linalg.norm(q - p, axis=1)

    def _normal(self, f):
        n = np.cross(self.B[f] - self.A[f], self.C[f] - self.A[f])
        return Vector(n / max(np.linalg.norm(n), 1e-300))

    def find_nearest_range(self, p, distance=1.84467e19):
        if not len(self.T):
            return []
        r = min(float(distance), 1e9)
        idx = self._candidates(p, r)
        if not len(idx):
            return []
        q, d = self._dist(p, idx)
        ok = d <= r
        return [(Vector(q[i]), self._normal(int(idx[i])), int(idx[i]), float(d[i])) for i in np.nonzero(ok)[0]]

    def find_nearest(self, p, distance=1.84467e19):
        none = (None, None, None, None)
        if not len(self.T):
            return none
        maxd = float(distance)
        r = self.h
        diag = float(np.linalg.norm(self.hi.max(axis=0) - self.lo.min(axis=0))) + float(
            np.linalg.norm(np.asarray(p) - self.o)) + 1.0
        while True:
            rr = min(r, maxd, diag)
            idx = self._candidates(p, rr)
            if len(idx):
                q, d = self._dist(p, idx)
                j = int(np.argmin(d))              # ties: lowest face index (idx is sorted)
                if d[j] <= rr:
                    return (Vector(q[j]), self._normal(int(idx[j])), int(idx[j]), float(d[j])) \
                        if d[j] <= maxd else none
            if rr >= maxd or rr >= diag:
                return none
            r *= 2.0

    def ray_cast(self, origin, direction, distance=1.84467e19):
        none = (None, None, None, None)
        if not len(self.T):
            return none
        o, d = np.asarray(origin, np.float64), np.asarray(direction, np.float64)
        d = d / max(np.linalg.norm(d), 1e-300)
        e1, e2 = self.B - self.A, self.C - self.A
        pv = np.cross(d, e2)
        det = np.einsum("ij,ij->i", e1, pv)
        with np.errstate(all="ignore"):
            inv = 1.0 / det
            tv = o - self.A
            u = np.einsum("ij,ij->i", tv, pv) * inv
            qv = np.cross(tv, e1)
            v = (qv @ d) * inv
            t = np.einsum("ij,ij->i", e2, qv) * inv
        ok = (np.abs(det) > 1e-18) & (u >= 0) & (v >= 0) & (u + v <= 1) & (t >= 0) & (t <= float(distance))
        if not ok.any():
            return none
        f = int(np.nonzero(ok)[0][np.argmin(t[ok])])
        return (Vector(o + d * t[f]), self._normal(f), f, float(t[f]))

    def overlap(self, other):
        """Pairs (i, j) of triangles of two trees that intersect.  Only `tree.overlap(tree)` (the
        self-intersection test of printcheck); candidates from shared grid cells and bounding boxes,
        then the exact test: some edge of one triangle crosses the other (coplanar pairs are ignored).
        Both orders of every pair are returned, shared-vertex pairs included (the caller filters them)."""
        if other is not self:
            raise NotImplementedError("mu_shim: BVHTree.overlap(other tree)")
        cand = []
        for fa in self.cells.values():
            if len(fa) > 1:
                i, j = np.triu_indices(len(fa), 1)
                cand.append(np.stack([fa[i], fa[j]], axis=1))
        if not cand:
            return []
        P = np.concatenate(cand)
        n = np.int64(len(self.T))
        key = np.unique(P[:, 0] * n + P[:, 1])
        P = np.stack([key // n, key % n], axis=1)
        P = P[np.all((self.lo[P[:, 0]] <= self.hi[P[:, 1]]) & (self.hi[P[:, 0]] >= self.lo[P[:, 1]]), axis=1)]
        hit = _tri_tri(self, P[:, 0], P[:, 1])
        P = P[hit]
        return [tuple(map(int, p)) for p in np.concatenate([P, P[:, ::-1]])]


def _edge_hits_tri(p0, p1, a, b, c):
    d = p1 - p0
    e1, e2 = b - a, c - a
    pv = np.cross(d, e2)
    det = np.einsum("ij,ij->i", e1, pv)
    ok = np.abs(det) > 1e-14
    inv = 1.0 / np.where(ok, det, 1.0)
    tv = p0 - a
    u = np.einsum("ij,ij->i", tv, pv) * inv
    qv = np.cross(tv, e1)
    v = np.einsum("ij,ij->i", d, qv) * inv
    t = np.einsum("ij,ij->i", e2, qv) * inv
    eps = 1e-12
    return ok & (u >= -eps) & (v >= -eps) & (u + v <= 1 + eps) & (t >= -eps) & (t <= 1 + eps)


def _tri_tri(tree, ia, ib):
    A = [tree.A[ia], tree.B[ia], tree.C[ia]]
    B = [tree.A[ib], tree.B[ib], tree.C[ib]]
    hit = np.zeros(len(ia), bool)
    for X, Y in ((A, B), (B, A)):
        for k in range(3):
            hit |= _edge_hits_tri(X[k], X[(k + 1) % 3], Y[0], Y[1], Y[2])
    return hit


class KDTree:
    """Exact nearest-point queries by brute force on a grid-free point array."""

    def __init__(self, size):
        self._p = []
        self._i = []
        self.P = None

    def insert(self, co, index):
        self._p.append(tuple(co))
        self._i.append(int(index))

    def balance(self):
        self.P = np.asarray(self._p, np.float64).reshape(-1, 3)
        self.I = np.asarray(self._i, np.int64)

    def find(self, co, filter=None):
        d = np.einsum("ij,ij->i", self.P - np.asarray(co, np.float64), self.P - np.asarray(co, np.float64))
        j = int(np.argmin(d))
        return (Vector(self.P[j]), int(self.I[j]), float(np.sqrt(d[j])))

    def find_n(self, co, n):
        c = np.asarray(co, np.float64)
        d = np.linalg.norm(self.P - c, axis=1)
        o = np.argsort(d, kind="stable")[:n]
        return [(Vector(self.P[j]), int(self.I[j]), float(d[j])) for j in o]

    def find_range(self, co, radius):
        c = np.asarray(co, np.float64)
        d = np.linalg.norm(self.P - c, axis=1)
        o = np.nonzero(d <= radius)[0]
        o = o[np.argsort(d[o], kind="stable")]
        return [(Vector(self.P[j]), int(self.I[j]), float(d[j])) for j in o]


def install():
    root = types.ModuleType("mathutils")
    root.Vector = Vector
    bvh = types.ModuleType("mathutils.bvhtree")
    bvh.BVHTree = BVHTree
    kd = types.ModuleType("mathutils.kdtree")
    kd.KDTree = KDTree
    root.bvhtree, root.kdtree = bvh, kd
    root.__path__ = []
    root.__golden_shim__ = True
    sys.modules["mathutils"] = root
    sys.modules["mathutils.bvhtree"] = bvh
    sys.modules["mathutils.kdtree"] = kd
