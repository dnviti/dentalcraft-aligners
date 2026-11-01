#!/usr/bin/env python3
"""Generates teeth.json: the plugin's per-tooth geometry (`teeth.py`) on two labelled
demo arches, to compare the Rust port (`dca_algo::teeth`) against.

    python3 -B -I gen_teeth.py [path/to/blender-ortho-aligner] > teeth.json

Needs only python3 and numpy (no Blender): the plugin modules are loaded by path the
way `gen_foundations.py` does, and `mu.kdtree` (a `mathutils` KD-tree in the plugin) is
replaced by an exact search over single-precision coordinates, like the original.

The input of each case is the plugin's synthetic arch (`demo.generate` up to the voxel
remesh, which needs Blender) with a coarser tooth mesh and gum, made into one connected
labelled surface:

* the two lowest rings of every crown (the root stub) are labelled as gingiva, so the
  ring at the cervical level is the margin (a border between a tooth and the gum);
* a second labelling of the same vertices, `plain`, keeps every part separate (crowns with their stub,
  and the gum, not connected): no margin border and no contacts, so the fallback margin (the lowest
  10% of the crown) and the teeth without contacts are compared too;
* the crown vertices that touch the neighbour's crown (within 0.4 mm) are joined to it
  by a degenerate triangle, which makes the interproximal contact areas.

The mesh is stored as base64 (float32 vertices, uint16 triangles, uint8 labels) so the
fixture stays small; the Rust test reads the very same values.
"""
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


an, mu, demo, tg = (load(n) for n in ("anatomy", "meshutil", "demo", "teeth"))

N_THETA = 10
GUM_STEP = 2.5
CONTACT_MM = 0.4


class KDTree:
    """Stand-in for mathutils.kdtree.KDTree: nearest point among single-precision
    coordinates, distance rounded to single precision (returned as a Python float)."""

    def __init__(self, points):
        self.P = np.asarray(points, np.float32).astype(np.float64)

    def find(self, co):
        q = np.asarray(co, np.float32).astype(np.float64)
        d = np.sqrt(((self.P - q) ** 2).sum(axis=1))
        i = int(np.argmin(d))
        return None, i, float(np.float32(d[i]))


mu.kdtree = lambda points: KDTree(points)


class Stop(Exception):
    pass


def labelled_arch(arch, severity, seed, third):
    """V, T, labels of the connected labelled demo arch, the labels of the separate parts, and
    the number of triangles of the parts (the first ones of T)."""
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
        demo.generate(None, arch, severity, seed, third_molars=third)
    except Stop:
        pass
    finally:
        demo.tooth_mesh, demo.gum_mesh = real_tooth, real_gum
        mu.mesh_from_arrays_fast = real_mesh
    V, T = cap["V"], cap["T"]
    per_tooth = N_THETA * 23 + 2
    labels = np.zeros(len(V), np.int64)
    for k, f in enumerate(order):
        s = k * per_tooth
        labels[s:s + per_tooth] = f
    plain = labels.copy()
    for k, f in enumerate(order):
        s = k * per_tooth
        labels[s:s + 2 * N_THETA] = 0           # the two lowest rings: root stub, gingiva
        labels[s + per_tooth - 1] = 0           # centre of the bottom cap
    n_parts = len(T)
    # contacts: crown vertices within CONTACT_MM of the neighbour's crown
    seq = an.arch_sequence(order, arch)
    extra = []
    for f in seq:
        nb = an.mesial_neighbor(seq, f)
        if nb is None:
            continue
        a = np.nonzero(labels == f)[0]
        b = np.nonzero(labels == nb)[0]
        d = np.sqrt(((V[a][:, None, :] - V[b][None, :, :]) ** 2).sum(axis=2))
        for i in np.nonzero(d.min(axis=1) < CONTACT_MM)[0]:
            extra.append([a[i], a[i], b[int(np.argmin(d[i]))]])
    if extra:
        T = np.concatenate([T, np.array(extra, np.int64)])
    # the single precision values are the input of both sides
    V = V.astype(np.float32).astype(np.float64)
    return V, T, labels, plain, n_parts


def b64(a, dtype):
    return base64.b64encode(np.ascontiguousarray(a, dtype=dtype).tobytes()).decode()


def L(a):
    """Nested lists of floats rounded to 1e-10 (far below the tolerances; keeps the fixture small)."""
    return np.round(np.asarray(a, dtype=np.float64), 10).tolist()


def R(x):
    return round(float(x), 10)


def tooth_record(t):
    return {
        "fdi": t.fdi, "md_sign": t.md_sign, "e_md": L(t.e_md), "e_bl": L(t.e_bl), "e_ax": L(t.e_ax),
        "width_md": R(t.width_md), "width_bl": R(t.width_bl), "height": R(t.height), "occ_z": R(t.occ_z),
        "tip": L(t.tip), "margin_mean": L(t.margin_mean), "cej": L(t.cej), "crest": L(t.crest),
        "apex": L(t.apex), "cres": L(t.cres), "contact_m": L(t.contact_m), "contact_d": L(t.contact_d),
        "rot_dev": R(t.rot_dev), "zen_b": R(t.zen_b), "zen_l": R(t.zen_l), "mesial_dir": L(t.mesial_dir),
        "out_dir": L(t.out_dir), "axis_tilt": R(t.axis_tilt), "top_elong": R(t.top_elong),
        "top_spread": R(t.top_spread), "centroid": L(t.centroid), "ref": L(t.ref),
        "n_margin": len(t.margin), "n_vertices": len(t.vidx), "n_faces": len(t.fidx),
        "frame": L(t.frame()),
    }


cases = []
for arch, severity, seed, third in (("LOWER", "MODERATE", 7, False), ("UPPER", "SEVERE", 11, True)):
    V, T, lab, plain, n_parts = labelled_arch(arch, severity, seed, third)
    teeth, seq = tg.compute_teeth(V, T, lab, arch)
    p_teeth, p_seq = tg.compute_teeth(V, T[:n_parts], plain, arch)
    lt = lab[T]
    full = (lt[:, 0] == lt[:, 1]) & (lt[:, 1] == lt[:, 2])
    walls = {}
    for f in seq:
        fidx = np.nonzero(full & (lt[:, 0] == f))[0]
        walls[str(f)] = L(tg.wall_axis(V, T, fidx))
    cases.append({
        "arch": arch, "severity": severity, "seed": seed, "third_molars": third,
        "n_vertices": len(V), "n_tris": len(T),
        "vertices": b64(V, "<f4"), "tris": b64(T, "<u2"), "labels": b64(lab, "u1"),
        "sequence": seq, "teeth": [tooth_record(teeth[f]) for f in seq], "wall_axis": walls,
        "plain": {"n_tris": n_parts, "labels": b64(plain, "u1"), "sequence": p_seq,
                  "teeth": [tooth_record(p_teeth[f]) for f in p_seq]},
    })

# numpy.percentile, subsample and the sign convention of the helpers
rng = np.random.default_rng(31)
pct = []
for n in (1, 2, 5, 17, 100, 333):
    a = np.round(rng.normal(0, 3, n), 10)
    pct.append({"values": L(a), "q": {str(q): float(np.percentile(a, q)) for q in (5, 10, 80, 85, 90)}})
sub = []
for n, k in ((300, 250), (251, 250), (1000, 250), (4096, 250), (97, 30)):
    X = np.arange(n * 3, dtype=np.float64).reshape(n, 3)
    sub.append({"n": n, "k": k, "first_col": L(mu.subsample(X, k)[:, 0])})

json.dump({"cases": cases, "percentile": pct, "subsample": sub}, sys.stdout, separators=(",", ":"))
