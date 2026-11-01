#!/usr/bin/env python3
"""Generates autoseed.json: the plugin's automatic seeds (autoseed.py) on demo arches,
to compare the Rust port (dca_algo::autoseed) against.

    python3 -I gen_autoseed.py [path/to/blender-ortho-aligner] > autoseed.json

Needs only python3 and numpy (no Blender). The plugin modules are loaded by path the
way `tests/unit/_carica.py` does. Stubs:

* `bpy`: an empty module (autoseed.py does not use it);
* `gpu_query`: `knn` is the exact brute-force nearest point in float64, ties to the
  smallest index, which is what the plugin's GPU and CPU paths both return
  (`gpu_query.Points.knn`); the real module needs Blender's `mathutils`.

The input meshes are the demo arches of `demo.generate` before Blender's voxel remesh
(the plain concatenation of the tooth and gum shells, like `dca_algo::demo`), with the
random draws replaced by fixed ones (`U`, `Z`) that the Rust test replays, rounded to
single precision (the Rust mesh stores `f32` positions) and optionally moved by a
rigid pose (`R`, `t`) like an arbitrary scanner frame.
"""
import importlib
import inspect
import json
import os
import sys
import types

import numpy as np

REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 6, "blender-ortho-aligner"))
PKG = os.path.join(REPO, "ortho_aligner")

# ---- loading ---------------------------------------------------------------------------------------
sys.modules["bpy"] = types.ModuleType("bpy")
pkg = types.ModuleType("oa_prova")
pkg.__path__ = [PKG]
sys.modules["oa_prova"] = pkg


def knn(S, Q, k=1):
    """Exact nearest point of S to every row of Q (float64, ties to the smallest index)."""
    assert k == 1
    S = np.asarray(S, np.float64)
    Q = np.asarray(Q, np.float64)
    idx = np.empty((len(Q), 1), np.int64)
    dist = np.empty((len(Q), 1))
    for a in range(0, len(Q), 1000):
        q = Q[a:a + 1000]
        D = S[None, :, :] - q[:, None, :]
        d = D[..., 0] * D[..., 0] + D[..., 1] * D[..., 1] + D[..., 2] * D[..., 2]
        j = np.argmin(d, axis=1)
        idx[a:a + 1000, 0] = j
        dist[a:a + 1000, 0] = np.sqrt(d[np.arange(len(q)), j])
    return idx, dist


gq = types.ModuleType("oa_prova.gpu_query")
gq.knn = knn
sys.modules["oa_prova.gpu_query"] = gq
pkg.gpu_query = gq


def load(name):
    return importlib.import_module(f"oa_prova.{name}")


an, af, mu, auto, demo = (load(n) for n in ("anatomy", "archform", "meshutil", "autoseed", "demo"))


def L(a, nd=None):
    a = np.asarray(a, dtype=np.float64)
    if nd is not None:
        a = np.round(a, nd)
    return [None if not np.isfinite(x) else float(x) for x in a.ravel()] if a.ndim <= 1 else \
        [L(r, nd) for r in a]


rng = np.random.default_rng(20261101)
out = {}

# ---- dynamic programming on synthetic profiles -------------------------------------------------------
dps = []
for case in range(3):
    nb = 180 + 20 * case
    dip = np.abs(rng.normal(0.4, 0.4, nb))
    for c in np.cumsum(rng.uniform(22, 34, 8)).astype(int):
        if c < nb:
            dip[max(0, c - 1):c + 2] += rng.uniform(0.8, 2.0)
    crown = np.clip(rng.uniform(0.3, 1.4, nb), 0, 1)
    if case == 1:
        crown[90:118] = 0.05   # a gap (missing tooth)
    BL = rng.uniform(0.7, 1.3, nb)
    C = np.concatenate([[0.0], np.cumsum(BL / np.median(BL))])
    n = 6
    w = rng.uniform(22, 32, n)
    bl = rng.uniform(0.8, 1.2, n)
    sig = rng.uniform(0.1, 0.14, n)
    skip = np.array([0 < j < n - 1 for j in range(n)])
    mid = (2, float(nb) / 2 + 3.3, 16.0) if case != 2 else (None, 0.0, 1.0)
    cost, segs = auto._dp(w, bl, dip, C, nb, sigma=sig, mid=mid, crown=crown, skippable=skip)
    dps.append({"nb": nb, "dip": L(dip), "crown": L(crown), "bl_cum": L(C), "w": L(w), "bl": L(bl),
                "sigma": L(sig), "skippable": skip.tolist(), "mid": [mid[0], mid[1], mid[2]],
                "cost": cost, "segs": [None if s is None else list(s) for s in segs]})
out["dp"] = dps

# ---- small helpers -----------------------------------------------------------------------------------
names = ["Patient_UpperJaw.stl", "scan-LowerJaw", "arcata_superiore", "mandibular_export", "maxillary",
         "OA_Demo_Inferiore", "scansione", "upper and lower", "MaxillaScan", "x_inf_01", None, ""]
out["names"] = [[n, auto.arch_from_name(n)] for n in names]
widths = {11: 8.9, 21: 8.4, 12: 6.4, 13: 7.9, 14: 7.2, 16: 10.6, 26: 12.9, 17: 4.1, 22: 0.0}
out["widths"] = {"widths": {str(k): v for k, v in widths.items()}, "fit": list(auto.width_fit(widths)),
                 "outliers": [list(o) for o in auto.width_outliers(widths)],
                 "ratio": auto.incisor_molar_ratio(widths), "ratio_none": auto.incisor_molar_ratio({11: 8.0})}
out["sequences"] = {
    "upper": auto.expected_sequence("UPPER"),
    "lower_third_missing36": auto.expected_sequence("LOWER", True, (36,)),
    "candidates_lower_missing45": [[c[0], c[1], list(c[2])]
                                   for c in auto.candidate_sequences("LOWER", False, (45,))],
}

# ---- demo arches -------------------------------------------------------------------------------------
U = rng.uniform(0, 1, 120).tolist()
Zs = rng.normal(0, 1, 40).tolist()


class Replay:
    def __init__(self):
        self.iu = self.iz = 0

    def uniform(self, a, b):
        v = a + (b - a) * U[self.iu]
        self.iu += 1
        return v

    def normal(self, m, s):
        v = m + s * Zs[self.iz]
        self.iz += 1
        return v


class Stop(Exception):
    pass


cap = {}


def fake_mesh(bpy, name, V, T, validate=True):
    cap["V"], cap["T"] = V, T
    cap["info"] = inspect.currentframe().f_back.f_locals["info"]
    raise Stop


mu.mesh_from_arrays_fast = fake_mesh
np.random.default_rng = lambda seed=None: Replay()

R = mu.rotvec_to_matrix(np.array([0.7, -1.9, 0.4]))
t = np.array([12.5, -31.0, 7.25])
runs = []
for arch, sev, third, posed in (("LOWER", "MODERATE", False, False), ("UPPER", "SEVERE", False, True),
                                ("LOWER", "MILD", True, True), ("UPPER", "MILD", False, False)):
    try:
        demo.generate(None, arch, sev, 7, third_molars=third)
    except Stop:
        pass
    V = np.asarray(cap["V"], np.float32).astype(np.float64)
    T = np.asarray(cap["T"], np.int64)
    if posed:
        V = V @ R.T + t
    n, c = auto.occlusal_axis(V, T)
    M, curve, pr = data = auto.arch_data(V, T)
    seeds, M2, diag = auto.propose(V, T, arch, include_third=third, data=data)
    det = auto.detect_arch(V, T)
    ss = np.linspace(curve.s[0], curve.s[-1], 9)
    run = {"arch": arch, "severity": sev, "third_molars": third, "posed": posed,
           "n_vertices": len(V), "vertex_sum": L(V.sum(axis=0)),
           "occlusal": {"n": L(n), "c": L(c)}, "frame": L(M),
           "curve": {"n": len(curve.P), "s0": float(curve.s[0]), "s1": float(curve.s[-1]),
                     "s": L(ss), "points": L(curve.point(ss))},
           "profiles": {"s_min": pr["s_min"], "s_max": pr["s_max"], "nb": pr["nb"], "n_points": len(pr["P"]),
                        "H": L(pr["H"], 6), "dip": L(pr["dip"], 6), "BL": L(pr["BL"], 6),
                        "crown": L(pr["crown"], 6), "off_top": L(pr["off_top"], 6), "dHbl": L(pr["dHbl"], 6)},
           "seeds": {str(f): L(p) for f, p in sorted(seeds.items())},
           "truth": {str(f): L(v["seed_point"]) for f, v in sorted(cap["info"].items())},
           "diag": {"scale": diag["scale"], "arch_length_mm": diag["arch_length_mm"], "teeth": diag["teeth"],
                    "cost": diag["cost"], "cost_raw": diag["cost_raw"],
                    "molars": [diag["molars"]["destra"], diag["molars"]["sinistra"]],
                    "candidates": [[list(e), c1, c2] for e, c1, c2 in diag["candidates"]],
                    "absent_inside": diag["absent_inside"]},
           "palate_coverage": auto.palate_coverage(V, T, M),
           "posterior_cusp_side": auto.posterior_cusp_side(pr),
           "detect": {"arch": det[0], "confidence": det[1], "details": {
               k: (float(v) if isinstance(v, (float, np.floating)) else v) for k, v in det[2].items()}}}
    if posed:
        run["R"], run["t"] = L(R), L(t)
    runs.append(run)
out["U"], out["Z"] = U, Zs
out["runs"] = runs

json.dump(out, sys.stdout, separators=(",", ":"))
