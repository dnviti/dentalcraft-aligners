#!/usr/bin/env python3
"""Generates occlusion.json: the plugin's occlusal contact distances (`occlusion.py`) on two
demo arches in occlusion, to compare the Rust port (`dca_algo::occlusion`) against.

    python3 -B -I gen_occlusion.py [path/to/blender-ortho-aligner] > occlusion.json

Needs only python3 and numpy (no Blender). `occlusion.py` imports the Blender-bound case
modules, so the functions compared are taken from its source by name (`ast`) and run as they
are: `_TopSurface`, `_place` and `signed_distances`, with the plugin's constants. The exact
geometric queries of `gpu_query.Mesh` (closest point, first ray hit, face normals) use the
plugin's own float64 numpy twins (`_closest_tri`, `_ray_tri`) over every triangle, as the GPU
path does, instead of the mathutils BVH.

Input: the plugin's synthetic arches (`demo.generate` up to the voxel remesh, which needs
Blender) with a coarse tooth mesh and gum, every tooth shell labelled with its FDI number and
the gum 0. Both are in arch coordinates (incisors +Y, occlusal +Z). The upper arch is turned
onto the lower one (`S_FLIP`), its incisors 2 mm in front, lowered with `_place` to the first
contact on the height grid and then 0.6 mm further, so that the contact map has
interpenetration as well as contacts. Every vertex is single precision on both sides.
"""
import ast
import base64
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
pkg = types.ModuleType("oa_prova")
pkg.__path__ = [PKG]
sys.modules["oa_prova"] = pkg


def load(name):
    return importlib.import_module(f"oa_prova.{name}")


an, mu, demo, gq = (load(n) for n in ("anatomy", "meshutil", "demo", "gpu_query"))


def plugin_functions(path, names, consts):
    """The top-level functions / classes `names` and constants `consts` of a plugin module, from
    its source (the module itself cannot be imported without Blender)."""
    src = open(path, encoding="utf-8").read()
    tree = ast.parse(src)
    keep = []
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)) and node.name in names:
            keep.append(node)
        elif isinstance(node, ast.Assign) and any(
                isinstance(t, ast.Name) and t.id in consts for t in node.targets):
            keep.append(node)
    ns = {"np": np}
    exec(compile(ast.Module(body=keep, type_ignores=[]), path, "exec"), ns)
    missing = [n for n in names + consts if n not in ns]
    assert not missing, missing
    return ns


occ = plugin_functions(os.path.join(PKG, "occlusion.py"),
                       ["_TopSurface", "_place", "signed_distances"], ["CONTACT", "S_FLIP"])

N_THETA = 10
GUM_STEP = 2.5


class Stop(Exception):
    pass


def labelled_arch(arch, severity, seed):
    """V, T, labels (FDI per tooth shell, 0 for the gum) of the plugin's demo arch."""
    real_tooth, real_gum = demo.tooth_mesh, demo.gum_mesh
    order, cap = [], {}

    def tooth(fdi, n_theta=48):
        order.append(fdi)
        return real_tooth(fdi, N_THETA)

    def gum(curve, s0, s1, **kw):
        return real_gum(curve, s0, s1, step=GUM_STEP)

    def fake_mesh(bpy, name, V, T, validate=True):
        cap["V"], cap["T"] = V, T
        raise Stop

    demo.tooth_mesh, demo.gum_mesh = tooth, gum
    real_mesh = mu.mesh_from_arrays_fast
    mu.mesh_from_arrays_fast = fake_mesh
    try:
        demo.generate(None, arch, severity, seed, third_molars=False)
    except Stop:
        pass
    finally:
        demo.tooth_mesh, demo.gum_mesh = real_tooth, real_gum
        mu.mesh_from_arrays_fast = real_mesh
    V, T = cap["V"], cap["T"]
    per_tooth = N_THETA * 23 + 2
    labels = np.zeros(len(V), np.int64)
    for k, f in enumerate(order):
        labels[k * per_tooth:(k + 1) * per_tooth] = f
    V = V.astype(np.float32).astype(np.float64)
    return V, np.asarray(T, np.int64), labels


class BruteMesh:
    """Stand-in for gpu_query.Mesh: exact float64 queries over every triangle with the
    plugin's numpy twins (the answers of its GPU path)."""

    def __init__(self, V, T):
        self.W = V[T]

    def normals(self, face):
        face = np.asarray(face, np.int64)
        W = self.W[np.maximum(face, 0)]
        n = np.cross(W[..., 1, :] - W[..., 0, :], W[..., 2, :] - W[..., 0, :])
        n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-300)
        return np.where((face >= 0)[..., None], n, 0.0)

    def nearest(self, P, maxd=np.inf, exact=True):
        m = len(self.W)
        D = np.full(len(P), np.inf)
        F = np.full(len(P), -1, np.int64)
        Q = np.full((len(P), 3), np.nan)
        for s in range(0, len(P), 32):
            Pc = P[s:s + 32]
            rows = np.repeat(Pc, m, axis=0)
            Wr = np.tile(self.W, (len(Pc), 1, 1))
            C = gq._closest_tri(rows, Wr)
            d = np.sqrt(gq._d2(rows, C)).reshape(len(Pc), m)
            j = np.argmin(d, axis=1)
            dj = d[np.arange(len(Pc)), j]
            ok = dj <= maxd
            D[s:s + 32] = np.where(ok, dj, np.inf)
            F[s:s + 32] = np.where(ok, j, -1)
            Q[s:s + 32] = np.where(ok[:, None], C.reshape(len(Pc), m, 3)[np.arange(len(Pc)), j], np.nan)
        return D, F, Q

    def first_hit(self, O, D, tmax=np.inf):
        O = np.asarray(O, np.float64).reshape(-1, 3)
        D = np.broadcast_to(np.asarray(D, np.float64), O.shape)
        m = len(self.W)
        T = np.full(len(O), np.nan)
        F = np.full(len(O), -1, np.int64)
        for i in range(len(O)):
            t = gq._ray_tri(np.repeat(O[i:i + 1], m, axis=0), np.repeat(D[i:i + 1], m, axis=0), self.W)
            ok = (t > 1e-9) & (t <= tmax)
            if ok.any():
                j = int(np.argmin(np.where(ok, t, np.inf)))
                T[i], F[i] = t[j], j
        return T, F


def crown_mesh(V, T, lab):
    """Closed tooth shells of an arch, each wound outwards (`occlusion._teeth_world`)."""
    lt = lab[T]
    keep = []
    for f in np.unique(lab[lab != 0]):
        Tf = T[(lt == f).all(axis=1)]
        if mu.signed_volume(V, Tf) < 0:
            Tf = Tf[:, ::-1]
        keep.append(Tf)
    return np.concatenate(keep)


Vl, Tl, Ll = labelled_arch("LOWER", "MODERATE", 7)
Vu, Tu, Lu = labelled_arch("UPPER", "MODERATE", 11)
cl = Ll != 0
cu = Lu != 0

# starting relation: upper arch flipped onto the lower one, incisors 2 mm in front
inc_l = np.isin(Ll, [31, 41])
inc_u = np.isin(Lu, [11, 21])
F = np.eye(4)
F[:3, :3] = occ["S_FLIP"]
G = F.copy()
G[1, 3] = (Vl[inc_l, 1].max() + 2.0) - (Vu[inc_u] @ occ["S_FLIP"].T)[:, 1].max()
top = occ["_TopSurface"](Vl[cl])

# _place on a few poses (the grid search of `align`)
poses = []
for ang, dx, dy in ((0.0, 0.0, 0.0), (1.5, 0.5, -0.5), (-3.0, -1.0, 1.0)):
    c, s = np.cos(np.radians(ang)), np.sin(np.radians(ang))
    Gp = np.eye(4)
    Gp[:3, :3] = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
    Gp[:3, 3] = (dx, dy, 0.0)
    Gp = Gp @ G
    r = occ["_place"](Vu[cu], top, Gp, 0.0)
    poses.append({"g": np.round(Gp, 15).ravel().tolist(),
                  "tz": float(r[0]), "contacts": r[1], "right": r[2], "left": r[3]})

# the relation of the contact map: first contact on the grid, then 0.6 mm into the lower arch
tz = occ["_place"](Vu[cu], top, G, 0.0)[0]
G[2, 3] += tz - 0.6

# heights of the grid at a few points
rng = np.random.default_rng(5)
probe = np.column_stack([rng.uniform(Vl[:, 0].min() - 2, Vl[:, 0].max() + 2, 40),
                         rng.uniform(Vl[:, 1].min() - 2, Vl[:, 1].max() + 2, 40), np.zeros(40)])
heights = top.at(probe)

# signed distances, both ways, in lower arch coordinates
Pu = Vu @ G[:3, :3].T + G[:3, 3]
mesh_l = BruteMesh(Vl, crown_mesh(Vl, Tl, Ll))
mesh_u = BruteMesh(Pu, crown_mesh(Pu, Tu, Lu))
z_l = np.array([0.0, 0.0, 1.0])            # occlusal axis of the lower arch (towards the upper)
z_u = G[:3, 2]                              # occlusal axis of the upper arch (towards the lower)
idx_u = np.nonzero(cu)[0]
idx_l = np.nonzero(cl)[0]
d_upper = occ["signed_distances"](Pu[idx_u], None, _tree=(mesh_l, z_l))
d_lower = occ["signed_distances"](Vl[idx_l], None, _tree=(mesh_u, z_u))


def b64(a, dtype):
    return base64.b64encode(np.ascontiguousarray(a, dtype=dtype).tobytes()).decode()


def finite(idx, d):
    ok = np.isfinite(d)
    return {"index": idx[ok].tolist(), "distance": np.round(d[ok], 12).tolist(), "n_measured": len(d)}


def arch_record(V, T, L):
    return {"n_vertices": len(V), "n_tris": len(T), "vertices": b64(V, "<f4"), "tris": b64(T, "<u2"),
            "labels": b64(L, "u1")}


json.dump({
    "lower": arch_record(Vl, Tl, Ll),
    "upper": arch_record(Vu, Tu, Lu),
    "relation": np.round(G, 15).ravel().tolist(),
    "contact": occ["CONTACT"],
    "poses": poses,
    "probes": {"points": np.round(probe, 12).tolist(),
               "heights": [None if not np.isfinite(h) else float(h) for h in heights]},
    "upper_to_lower": finite(idx_u, d_upper),
    "lower_to_upper": finite(idx_l, d_lower),
}, sys.stdout, separators=(",", ":"))
