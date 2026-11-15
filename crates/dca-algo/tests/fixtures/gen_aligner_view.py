#!/usr/bin/env python3
"""Generates aligner_view.json: the plugin's aligner of the 3D view moved to a
stage it was not built at (`animation._bind`, `_sparse`, `_relative`,
`_deform`, `animation.py:540-641`, as `update_aligner_preview` calls them,
`animation.py:1243-1256`), to compare `dca_algo::aligner_view` against.

    PYTHONDONTWRITEBYTECODE=1 python3 -B gen_aligner_view.py > aligner_view.json

Needs python3 and numpy only: `animation.py` imports `bpy`, so its four
functions (and `_knn`, `KNN_CELL_MM`) are pulled out of the source with the
golden oracle's `plugin.extract`; `reconstruct`, `meshutil` and `gpu_query`
(its CPU path) are the plugin's own modules. `_bind` reads the case through
`case._data`, `case._model`, `case.tooth_local` and `case.unflat`: a stand-in
`case` module serves the synthetic case below, and `_bind` itself runs
unchanged.

The case (synthetic, no scan): the plugin's demo crowns (`demo.tooth_mesh`,
12 directions) of 31-34 along a curve, their rest frames, and a band of
gingiva under them with smooth weights on 31-35 (35 has no closed tooth: its
column is only the gingiva's). A patch of gingiva beyond 31 lies on an exact
0.5 mm grid with weights on 31 only, and aligner vertices at the centres of
its squares: the eight nearest are ties, decided by the smallest index.

The built aligner: the crowns posed at the stage of the base, every vertex
pushed 0.6 mm out from the centre of its crown, the band of gingiva lifted
0.4 mm, and the tie vertices; single precision, as the mesh stores it.

The stages: the base, where 31 is at rest, 34 has no motion and 35 moves;
"moved", where every tooth moves except 32 (the motion of the base: from the
base it stays, within the plugin's 1e-12) and 35 (no motion: left out); and
the base itself (the vertices come back as they are).

Doubles are written as their IEEE-754 bits (u64), so the Rust test reads
exactly the plugin's inputs; singles as decimals (exact once rounded back to
single precision).
"""
import json
import os
import struct
import sys
import types

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.normpath(os.path.join(HERE, "../../../../golden"))
sys.path.insert(0, GOLDEN)
sys.dont_write_bytecode = True

from oracle import plugin  # noqa: E402

FS = ["31", "32", "33", "34", "35"]
TEETH = ["31", "32", "33", "34"]
N_THETA = 12
OUT_MM = 0.6
GUM_LIFT_MM = 0.4


def bits(a):
    """The IEEE-754 bits of every double of `a` (nested lists, same shape)."""
    a = np.asarray(a, np.float64)
    flat = [struct.unpack("<Q", struct.pack("<d", float(x)))[0] for x in a.ravel()]
    return np.array(flat, dtype=object).reshape(a.shape).tolist()


def singles(a):
    return [[float(x) for x in row] for row in np.asarray(a, np.float32)]


def rot(axis, deg):
    a = np.asarray(axis, np.float64)
    a = a / np.linalg.norm(a)
    t = np.radians(deg)
    K = np.array([[0, -a[2], a[1]], [a[2], 0, -a[0]], [-a[1], a[0], 0]])
    return np.eye(3) + np.sin(t) * K + (1 - np.cos(t)) * K @ K


def about(c, axis, deg, shift):
    """Rotation of `deg` about `axis` through `c`, then the shift."""
    R = rot(axis, deg)
    M = np.eye(4)
    M[:3, :3] = R
    M[:3, 3] = c - R @ c + np.asarray(shift, np.float64)
    return M


def main():
    demo, rc, mu, gq = (plugin.load(n) for n in ("demo", "reconstruct", "meshutil", "gpu_query"))

    # the teeth: crowns in tooth-local coordinates, rest frames along a curve
    local, rest = {}, {}
    for i, f in enumerate(TEETH):
        V, _, _ = demo.tooth_mesh(int(f), N_THETA)
        local[f] = np.asarray(V, np.float64)
        x = 6.0 * i
        y = -0.04 * x * x
        ang = np.degrees(np.arctan(-0.08 * x))
        M = np.eye(4)
        M[:3, :3] = rot((0, 0, 1), ang)
        M[:3, 3] = (x, y, 0.0)
        rest[f] = M
    centre = {f: rest[f][:3, 3] for f in TEETH}
    centre["35"] = np.array([24.0, -0.04 * 24.0 * 24.0, 0.0])

    # the gingiva: a band under the teeth with smooth weights on 31-35
    gum, gw = [], []
    for s in np.linspace(-4.0, 28.0, 33):
        for off in (-3.0, -1.5, 0.0, 1.5, 3.0):
            for z in (-2.5, -1.0):
                p = np.array([s, -0.04 * s * s + off, z])
                gum.append(p)
                d2 = np.array([np.sum((p[:2] - centre[f][:2]) ** 2) for f in FS])
                raw = np.exp(-d2 / (2.0 * 3.5 ** 2))
                raw[raw < 1e-3] = 0.0
                tot = raw.sum()
                row = raw / tot * min(1.0, tot) if tot > 0 else raw
                gw.append(row)
    # the patch on an exact grid beyond 31, on 31 only
    patch0 = len(gum)
    for a in range(6):
        for b in range(6):
            p = np.array([-14.0 + 0.5 * a, -1.0 + 0.5 * b, -4.0])
            gum.append(p)
            row = np.zeros(len(FS))
            row[0] = 0.05 + 0.025 * ((a * 7 + b * 3) % 11)
            gw.append(row)
    gum = np.array(gum, np.float64)
    gw = np.array(gw, np.float32)

    # the stages (deltas: motions from the rest pose of each tooth)
    base = {
        "31": np.eye(4),
        "32": about(centre["32"], (0.2, 0.1, 1.0), 4.0, (0.15, -0.1, 0.05)),
        "33": about(centre["33"], (1.0, 0.0, 0.3), -3.0, (-0.2, 0.25, 0.0)),
        "35": about(centre["35"], (0.0, 1.0, 0.0), 2.0, (0.1, 0.0, -0.05)),
    }
    moved = {
        "31": about(centre["31"], (0.0, 0.0, 1.0), 6.0, (0.3, 0.2, 0.0)),
        "32": base["32"].copy(),
        "33": about(centre["33"], (0.3, 1.0, 0.0), 5.0, (0.1, 0.4, -0.1)),
        "34": about(centre["34"], (1.0, 1.0, 0.2), -7.0, (-0.35, 0.1, 0.2)),
    }

    # the stand-in case and object read by _bind
    model = {"fs": FS, "G": {"V": gum}, "W": gw}
    case = types.ModuleType(plugin.PKG + ".case")
    case._data = lambda obj: {}
    case._model = lambda obj, d=None: model
    case.tooth_local = lambda obj, d: {f: (local[f], None) for f in TEETH}
    case.unflat = lambda m: np.asarray(m, np.float64).reshape(4, 4)
    sys.modules[case.__name__] = case
    pkg = sys.modules[plugin.PKG]
    pkg.case = case
    obj = types.SimpleNamespace(oa=types.SimpleNamespace(
        teeth=[types.SimpleNamespace(fdi=f, rest=rest[f].ravel().tolist()) for f in TEETH]))

    ns = plugin.extract("animation", ["KNN_CELL_MM", "_knn", "_bind", "_sparse", "_deform", "_relative"],
                        {"np": np, "mu": mu})

    # the points _bind measures from (S): captured from its own call
    seen = {}
    knn = gq.knn

    def spy(S, Q, k=1, cpu=None):
        seen["S"] = np.array(S, np.float64)
        return knn(S, Q, k, cpu)

    gq.knn = spy

    # the built aligner at the stage of the base
    verts = []
    for f in TEETH:
        P = mu.transform_points(base.get(f, np.eye(4)) @ rest[f], local[f])
        c = P.mean(axis=0)
        d = P - c
        n = np.linalg.norm(d, axis=1, keepdims=True)
        verts.append(P + OUT_MM * d / np.maximum(n, 1e-9))
    G = rc.deform(gum[:patch0], gw[:patch0], FS, base)
    verts.append(G[::3] + np.array([0.0, 0.0, GUM_LIFT_MM]))
    tie = [(-14.0 + 0.5 * a + 0.25, -1.0 + 0.5 * b + 0.25, -4.0) for a in range(1, 4) for b in range(1, 4)]
    verts.append(np.array(tie))
    Vs = np.concatenate(verts).astype(np.float32)

    # update_aligner_preview: _binding of a fresh base (no weights stored on the mesh)
    rest_v = np.ascontiguousarray(Vs, np.float32).reshape(-1, 3)
    W = ns["_bind"](obj, rest_v.astype(np.float64), base)
    bind = (rest_v, ns["_sparse"](W, rest_v), base)
    S = seen["S"]
    SW = np.concatenate([gw] + [np.tile(np.eye(len(FS), dtype=np.float32)[FS.index(f)], (len(local[f]), 1))
                                for f in TEETH])
    assert len(S) == len(SW)

    stages = []
    for name, deltas in (("moved", moved), ("base", base)):
        rel = ns["_relative"](deltas, bind[2])
        V = ns["_deform"](bind, FS, rel)
        stages.append({
            "name": name,
            "deltas": {f: bits(m) for f, m in deltas.items()},
            "relative": {f: bits(m) for f, m in rel.items()},
            "vertices": singles(V),
        })
    sparse = {FS[j]: [int(i) for i in nz] for j, nz, _, _ in bind[1]}

    out = {
        "plugin": "blender-ortho-aligner 42e6d87, animation.py _bind/_sparse/_relative/_deform",
        "fdis": FS,
        "points": bits(S),
        "point_weights": singles(SW),
        "gum_points": len(gum),
        "gum_rest": bits(gum),
        "gum_weights": singles(gw),
        "vertices": singles(Vs),
        "tie_vertices": [len(Vs) - len(tie), len(Vs)],
        "base": {f: bits(m) for f, m in base.items()},
        "weights": singles(W),
        "sparse_rows": sparse,
        "stages": stages,
    }
    json.dump(out, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
