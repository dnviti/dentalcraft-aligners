#!/usr/bin/env python3
"""Generates holes.json: the plugin's handling of the holes through the wall of a trimmed
shell (`case_print._handle_loops`, `_short_loops`, `_cap_loop`, `_plug_holes`,
`_window_cuts`; `aligner.trim_cutter` with slots) on synthetic shells, to compare the Rust
port (`dca_algo::trim::holes`) against.

    python3 -B -I gen_holes.py [path/to/blender-ortho-aligner] > holes.json

Needs only python3 and numpy (no Blender): the plugin modules are loaded by path with `bpy`
and `mathutils` replaced by empty modules; `gpu_query` (the distances from the model, the
nearest points) by exact numpy searches with the plugin's ties, and
`printcheck.self_intersecting_pairs` (a mathutils BVH overlap) by an exact test of every pair
of faces sharing no vertex (an edge of one crossing the other).

The shells are slabs of voxels (0.125 mm, 4 layers: a 0.5 mm wall) on a box, the model,
pierced by square tunnels (a hole through the wall: a handle of the surface), their
vertices moved by less than 0.02 mm (multiples of 2^-12: exact in single precision) so that
no two loops have the same length. "two": two tunnels, 0.5 and 0.75 mm wide, the first
1 mm from the border; "rim": one tunnel 0.125 mm from the border (a loop along it).

A surface made by the plugin (a capped or plugged shell) is stored as the vertices of the
input it dropped, the vertices it added (in order), the number of its faces and an FNV-1a
hash of them (64 bits over the vertex indices), and so for its cap faces.
"""
import importlib
import json
import os
import sys
import types

import numpy as np

REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 6, "blender-ortho-aligner"))
PKG = os.path.join(REPO, "ortho_aligner")

sys.modules["bpy"] = types.ModuleType("bpy")
mathutils = types.ModuleType("mathutils")
mathutils.Matrix = mathutils.Vector = object
sys.modules["mathutils"] = mathutils
sys.modules["mathutils.kdtree"] = types.ModuleType("mathutils.kdtree")
pkg = types.ModuleType("oa_holes")
pkg.__path__ = [PKG]
sys.modules["oa_holes"] = pkg


def point_triangle(P, A, B, C):
    """Distance of every point P (k, 3) from the triangle A, B, C (Ericson, float64)."""
    ab, ac = B - A, C - A
    out = np.empty(len(P))
    for i, p in enumerate(P):
        ap = p - A
        d1, d2 = ab @ ap, ac @ ap
        if d1 <= 0 and d2 <= 0:
            q = A
        else:
            bp = p - B
            d3, d4 = ab @ bp, ac @ bp
            if d3 >= 0 and d4 <= d3:
                q = B
            else:
                vc = d1 * d4 - d3 * d2
                if vc <= 0 and d1 >= 0 and d3 <= 0:
                    q = A + d1 / (d1 - d3) * ab
                else:
                    cp_ = p - C
                    d5, d6 = ab @ cp_, ac @ cp_
                    if d6 >= 0 and d5 <= d6:
                        q = C
                    else:
                        vb = d5 * d2 - d1 * d6
                        if vb <= 0 and d2 >= 0 and d6 <= 0:
                            q = A + d2 / (d2 - d6) * ac
                        else:
                            va = d3 * d6 - d5 * d4
                            if va <= 0 and (d4 - d3) >= 0 and (d5 - d6) >= 0:
                                q = B + (d4 - d3) / ((d4 - d3) + (d5 - d6)) * (C - B)
                            else:
                                den = 1.0 / (va + vb + vc)
                                q = A + ab * (vb * den) + ac * (vc * den)
        out[i] = np.linalg.norm(p - q)
    return out


class Mesh:
    """Stand-in for gpu_query.Mesh: exact distances from the surface (inf beyond maxd)."""

    def __init__(self, V, T):
        self.V, self.T = np.asarray(V, np.float64), np.asarray(T, np.int64)

    def nearest(self, P, maxd=np.inf, exact=True):
        P = np.asarray(P, np.float64).reshape(-1, 3)
        D = np.full(len(P), np.inf)
        for t in self.T:
            D = np.minimum(D, point_triangle(P, *self.V[t]))
        D[D > maxd] = np.inf
        return D, np.full(len(P), -1), np.full((len(P), 3), np.nan)

    def ray_cast(self, O, D, tmax=np.inf, hits=1, gap=1e-4):
        """Hits along the rays (Moller-Trumbore, float64): t (m, hits), nan where none, and
        faces, -1; hit h+1 the nearest at least `gap` beyond hit h, ties to the smaller face."""
        O = np.asarray(O, np.float64).reshape(-1, 3)
        D = np.broadcast_to(np.asarray(D, np.float64) / np.linalg.norm(D), O.shape)
        A, B, C = self.V[self.T[:, 0]], self.V[self.T[:, 1]], self.V[self.T[:, 2]]
        e1, e2 = B - A, C - A
        t_out = np.full((len(O), hits), np.nan)
        f_out = np.full((len(O), hits), -1, np.int64)
        for i in range(len(O)):
            h = np.cross(D[i], e2)
            a = (e1 * h).sum(axis=1)
            ok = np.abs(a) > 1e-15
            f = np.where(ok, 1.0 / np.where(ok, a, 1.0), 0.0)
            s = O[i] - A
            u = f * (s * h).sum(axis=1)
            q = np.cross(s, e1)
            v = f * (q @ D[i])
            t = f * (e2 * q).sum(axis=1)
            hit = ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t >= 0) & (t <= tmax)
            idx = np.flatnonzero(hit)
            idx = idx[np.lexsort((idx, t[idx]))]
            last, k = -np.inf, 0
            for j in idx:
                if k >= hits:
                    break
                if t[j] >= last + gap:
                    t_out[i, k], f_out[i, k], last, k = t[j], j, t[j], k + 1
        return t_out, f_out

    def release(self):
        pass


def knn(S, Q, k=1, cpu=None):
    """Stand-in for gpu_query.knn: the nearest row of S to every row of Q, ties to the
    smallest index, float64 squared distances summed in the plugin's order."""
    S, Q = np.asarray(S, np.float64), np.asarray(Q, np.float64)
    idx, dist = np.zeros((len(Q), 1), np.int64), np.zeros((len(Q), 1))
    for i, q in enumerate(Q):
        D = S - q
        dk = D[:, 0] * D[:, 0] + D[:, 1] * D[:, 1] + D[:, 2] * D[:, 2]
        j = int(np.argmin(dk))
        idx[i, 0], dist[i, 0] = j, np.sqrt(dk[j])
    return idx, dist


gq = types.ModuleType("oa_holes.gpu_query")
gq.Mesh, gq.knn = Mesh, knn
sys.modules["oa_holes.gpu_query"] = gq
pkg.gpu_query = gq


def load(name):
    return importlib.import_module(f"oa_holes.{name}")


al, cp, mu, pc = (load(n) for n in ("aligner", "case_print", "meshutil", "printcheck"))


def segment_crosses(P0, P1, A, B, C, eps=1e-12):
    """The segments P0-P1 cross the triangles A, B, C (Moller-Trumbore, not in their plane)."""
    d = P1 - P0
    e1, e2 = B - A, C - A
    h = np.cross(d, e2)
    a = (e1 * h).sum(axis=1)
    ok = np.abs(a) > eps
    f = np.where(ok, 1.0 / np.where(ok, a, 1.0), 0.0)
    s = P0 - A
    u = f * (s * h).sum(axis=1)
    q = np.cross(s, e1)
    v = f * (d * q).sum(axis=1)
    t = f * (e2 * q).sum(axis=1)
    return ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t >= 0) & (t <= 1)


def self_intersecting_pairs(V, T):
    """Pairs of faces sharing no vertex where an edge of one crosses the other."""
    V, T = np.asarray(V, np.float64), np.asarray(T, np.int64)
    i, j = np.triu_indices(len(T), 1)
    share = (T[i][:, :, None] == T[j][:, None, :]).any(axis=(1, 2))
    i, j = i[~share], j[~share]
    hit = np.zeros(len(i), bool)
    for x, y in ((i, j), (j, i)):
        A, B, C = V[T[y, 0]], V[T[y, 1]], V[T[y, 2]]
        for a, b in ((0, 1), (1, 2), (2, 0)):
            hit |= segment_crosses(V[T[x, a]], V[T[x, b]], A, B, C)
    return np.stack([i[hit], j[hit]], axis=1)


pc.self_intersecting_pairs = self_intersecting_pairs

H = 0.125            # the voxel of the slabs
OUTER = 0.5          # clearance 0, wall 0.5
VOXEL = 0.1          # the voxel of the shell the plugin is told


def jitter(n, seed):
    rng = np.random.default_rng(seed)
    return np.round(rng.uniform(-0.02, 0.02, (n, 3)) * 4096.0) / 4096.0


def voxel_surface(occ, seed, h=H):
    """Closed surface of the full voxels of occ (nx, ny, nz), spacing h, outward: two
    triangles per face between a full and an empty voxel; the corners welded, numbered as
    they are first used; moved by jitter()."""
    nx, ny, nz = occ.shape
    ids, V, T = {}, [], []

    def vid(c):
        if c not in ids:
            ids[c] = len(V)
            V.append(c)
        return ids[c]

    def full(i, j, k):
        return 0 <= i < nx and 0 <= j < ny and 0 <= k < nz and occ[i, j, k]
    # the 4 corners of every face, counter-clockwise seen from outside
    faces = {(-1, 0, 0): [(0, 0, 0), (0, 0, 1), (0, 1, 1), (0, 1, 0)],
             (1, 0, 0): [(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)],
             (0, -1, 0): [(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)],
             (0, 1, 0): [(0, 1, 0), (0, 1, 1), (1, 1, 1), (1, 1, 0)],
             (0, 0, -1): [(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)],
             (0, 0, 1): [(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)]}
    for i in range(nx):
        for j in range(ny):
            for k in range(nz):
                if not occ[i, j, k]:
                    continue
                for (di, dj, dk), quad in faces.items():
                    if full(i + di, j + dj, k + dk):
                        continue
                    a, b, c, d = (vid((i + x, j + y, k + z)) for x, y, z in quad)
                    T += [(a, b, c), (a, c, d)]
    V = np.asarray(V, np.float64) * h + jitter(len(V), seed)
    return V, np.asarray(T, np.int64)


def box(lo, hi):
    x0, y0, z0 = lo
    x1, y1, z1 = hi
    V = np.array([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0],
                  [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], np.float64)
    T = np.array([[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
                  [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]], np.int64)
    return V, T


def slab(size, holes, seed):
    """A slab of size (nx, ny) voxels, 0.5 mm thick, with square tunnels [(i0, j0, side)]."""
    occ = np.ones((size[0], size[1], 4), bool)
    for i0, j0, s in holes:
        occ[i0:i0 + s, j0:j0 + s, :] = False
    return voxel_surface(occ, seed)


def r(a, nd=12):
    return np.round(np.asarray(a, np.float64), nd).tolist()


def fnv(values):
    h = 0xcbf29ce484222325
    for x in np.asarray(values, np.int64).ravel().tolist():
        h = ((h ^ int(x)) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return str(h)


def surface(V, V2, T2, extra=None):
    """A surface made from (V, T): the vertices of V it dropped, those it added, its faces."""
    at = {tuple(p): i for i, p in enumerate(np.asarray(V, np.float64).tolist())}
    kept, new = [], []
    for p in np.asarray(V2, np.float64).tolist():
        i = at.get(tuple(p))
        if i is None or new:
            new.append(p)
        else:
            kept.append(i)
    assert kept == sorted(kept)
    out = {"dropped": sorted(set(range(len(V))) - set(kept)), "new": r(new), "faces": len(T2),
           "hash": fnv(T2)}
    out.update(extra or {})
    return out


def case(name, V, T, model):
    geo = {"warp": None, "outer": OUTER, "clearance": 0.0}
    out = {"name": name, "V": V.tolist(), "T": T.tolist(), "genus": cp._genus(V, T)}
    loops = cp._handle_loops(V, T)
    out["handle_loops"] = [L.tolist() for L in loops]
    out["short_loops"] = [[[ln, S.tolist()] for ln, S in cp._short_loops(V, T, L)] for L in loops]
    short = sorted((x for L in loops for x in cp._short_loops(V, T, L)), key=lambda t: t[0])
    out["capped_loop"] = short[0][1].tolist()
    caps = {}
    for fill in ("fan", "area"):
        res = cp._cap_loop(V, T, short[0][1], fill)
        caps[fill] = None if res is None else surface(V, res[0], res[1], {
            "cap": np.flatnonzero(res[2]).tolist(), "genus": res[3]})
    out["caps"] = caps
    Vp, Tp, g, plugs = cp._plug_holes(V, T, geo, model, VOXEL)
    out["plugged"] = surface(V, Vp, Tp, {"genus": g, "plugs": plugs})
    out["slots"] = {}
    for half in (0.45, 0.9):
        cuts = cp._window_cuts(geo, V, T, model, VOXEL, half)
        out["slots"][str(half)] = [[list(p0), list(p1), z, h] for p0, p1, z, h in cuts]
    return out


def removal():
    """The rigid removal of a shell (case_print._removal_lock; glued as rigid_lock does): a
    mushroom tooth (an undercut under its bulge) and its shell two voxels thick from 1.8 mm
    up, both of voxels of 0.3125 mm; their vertices stored x 4096 (integers: the jitter is in
    steps of 1/4096)."""
    h, n = 0.3125, 28
    c = 0.5 * n * h
    X, Y, Z = np.meshgrid(*[(np.arange(m) + 0.5) * h for m in (n, n, 22)], indexing="ij")
    R = np.hypot(X - c, Y - c)
    tooth = (((Z < 2.6) & (R < 1.8)) | ((Z >= 2.6) & (Z < 4.8) & (R < 2.8))
             | ((Z >= 4.8) & (Z < 5.6) & (R < 2.2)))
    grown = tooth.copy()
    for _ in range(2):
        g = grown.copy()
        for di in (-1, 0, 1):
            for dj in (-1, 0, 1):
                for dk in (-1, 0, 1):
                    g |= np.roll(np.roll(np.roll(grown, di, 0), dj, 1), dk, 2)
        grown = g
    Vt, Tt = voxel_surface(tooth, 3, h)
    Vs, Ts = voxel_surface(grown & ~tooth & (Z >= 1.8), 4, h)
    teeth, shell = Mesh(Vt, Tt), Mesh(Vs, Ts)

    def ints(V):
        Q = np.rint(V * 4096.0)
        assert (Q / 4096.0 == V).all()
        return Q.astype(np.int64).tolist()
    out = {"tooth": {"V4096": ints(Vt), "T": Tt.tolist()}, "shell": {"V4096": ints(Vs), "T": Ts.tolist()}}
    # VERTICAL and a leaning path, on the lines of the check
    paths = [(0.0, 0.0), (0.18, -0.07)]
    out["paths"] = [list(p) for p in paths]
    out["locks"] = [list(x) for x in cp._removal_lock(Vs, Ts, paths, cp.REMOVAL_CHECK_GRID_MM, teeth, shell)]
    # PER_TOOTH (rigid_lock): the mean path of the teeth and paths leaning towards it
    S = np.array([[0.05, 0.02], [0.1, -0.03]])
    mean = S.mean(axis=0)
    ph = float(np.arctan2(mean[1], mean[0])) if np.linalg.norm(mean) > 1e-9 else 0.5 * np.pi
    cands = [mean] + [cp._tilted(np.radians(t), ph) for t in cp.REMOVAL_CHECK_TILTS_DEG]
    res = cp._removal_lock(Vs, Ts, cands, 2.0 * cp.REMOVAL_CHECK_GRID_MM, teeth, shell)
    i = min(range(len(cands)), key=lambda j: (res[j][1], res[j][0]))
    lock = cp._removal_lock(Vs, Ts, [cands[i]], cp.REMOVAL_CHECK_GRID_MM, teeth, shell)[0]
    out["rigid"] = {"shears": S.tolist(), "coarse": [list(x) for x in res], "lock": list(lock),
                    "shear": np.asarray(cands[i]).tolist()}
    return out


def main():
    model = box((-1.0, -1.0, -3.0), (6.0, 5.0, 0.0))
    out = {"outer": OUTER, "clearance": 0.0, "voxel": VOXEL, "model": {"V": model[0].tolist(),
                                                                       "T": model[1].tolist()}}
    out["cases"] = [case("two", *slab((40, 28), [(8, 12, 4), (24, 12, 6)], 1), model),
                    case("rim", *slab((32, 24), [(14, 1, 4)], 2), model)]
    # the slots in the trim surface (aligner.trim_cutter, its bottom)
    ctrl = np.array([[0.5, 0.5, -1.0], [5.5, 0.5, -1.2], [3.0, 3.5, -0.8], [1.0, 3.0, -1.1]])
    lo, hi = np.array([0.0, 0.0, -3.0]), np.array([5.0, 3.5, 0.5])
    cuts = [(tuple(c[0]), tuple(c[1]), c[2], c[3]) for c in out["cases"][0]["slots"]["0.45"]]
    cuts += [(tuple(c[0]), tuple(c[1]), c[2], c[3]) for c in out["cases"][1]["slots"]["0.45"]]
    Vc, _ = al.trim_cutter(ctrl, lo, hi, hi[2] + 5.0, cell=0.4, k=4, reach=40.0, cuts=cuts)
    out["cutter"] = {"ctrl": ctrl.tolist(), "lo": lo.tolist(), "hi": hi.tolist(), "cell": 0.4,
                     "cuts": [[list(p0), list(p1), z, h] for p0, p1, z, h in cuts],
                     "z": Vc[:len(Vc) // 2, 2].tolist()}
    out["removal"] = removal()
    json.dump(out, sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main()
