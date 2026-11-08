#!/usr/bin/env python3
"""Runs the plugin's automatic seeds (autoseed.py `arch_data` + `propose`) on the real
scans and writes what it finds to a JSON file OUTSIDE the repository, for
`tests/autoseed_real.rs` to compare the Rust port against.

    DC_REAL_SCANS=/path/to/scans python3 -I gen_autoseed_real.py [plugin repo] > /outside/the/repo/autoseed_real.json
    DC_AUTOSEED_REAL_ORACLE=/outside/the/repo/autoseed_real.json DC_REAL_SCANS=... cargo test -p dca-algo \
        --test autoseed_real -- --nocapture

The tests are not ignored: they run with the rest and skip themselves (a line on stderr)
when either variable is missing.

The scans are patient data: neither the output of this script (arch, tooth lists, seed
points, arch frame) nor the meshes may be committed. The Rust test reads everything it
compares from this file, the tooth lists included.

Needs only python3 and numpy (no Blender). Loading and stubs as in `gen_autoseed.py`
(`bpy` empty, `gpu_query.knn` = exact brute-force nearest point). The scans are the four
STL files of `DC_REAL_SCANS` sorted by name and called `a` to `d` (like
`dca-module/tests/import_real.rs`); arches lower, upper, lower, upper. Each binary STL is
welded on bit-identical f32 positions, which is what `dc_mesh::io` (relative tolerance
1e-6 of the bbox diagonal) and Blender's STL importer give on these files; the vertex
order does not matter (`arch_points` subsamples in a canonical order). Default settings
of the plugin: third molars off, no missing teeth, ends chosen by the data
(`case_auto.auto_seeds_iter`).
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
SCANS = os.environ["DC_REAL_SCANS"]
ARCH = {"a": "LOWER", "b": "UPPER", "c": "LOWER", "d": "UPPER"}

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
auto = importlib.import_module("oa_prova.autoseed")


def read_stl(path):
    """Binary STL -> (V float64 of the f32 positions, T int64), exact weld."""
    raw = open(path, "rb").read()
    n = int(np.frombuffer(raw, "<u4", 1, 80)[0])
    assert len(raw) == 84 + 50 * n, "only binary STL"
    rec = np.frombuffer(raw, np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")]), n, 84)
    soup = np.ascontiguousarray(rec["v"].reshape(-1, 3)) + np.float32(0.0)   # -0.0 -> 0.0
    V, inv = np.unique(soup.view(np.dtype((np.void, 12))).ravel(), return_inverse=True)
    V = V.view(np.float32).reshape(-1, 3).astype(np.float64)
    return V, inv.reshape(-1, 3).astype(np.int64)


files = sorted(f for f in os.listdir(SCANS) if f.lower().endswith(".stl"))
assert len(files) == 4, "the folder must hold the four scans"
out = {}
for letter, name in zip("abcd", files):
    V, T = read_stl(os.path.join(SCANS, name))
    data = auto.arch_data(V, T)
    seeds, M, diag = auto.propose(V, T, ARCH[letter], False, (), data=data)
    out[letter] = {
        "arch": ARCH[letter], "vertices": len(V), "faces": len(T),
        "fdi": sorted(int(f) for f in seeds),
        "seeds": {str(int(f)): [float(x) for x in p] for f, p in seeds.items()},
        "frame": np.asarray(M, np.float64).tolist(),
        "scale": diag["scale"], "cost": diag["cost"], "molars": [diag["molars"]["destra"], diag["molars"]["sinistra"]],
        "arch_length_mm": diag["arch_length_mm"], "absent_inside": [int(f) for f in diag["absent_inside"]],
        "nb": int(data[2]["nb"]), "s_min": float(data[2]["s_min"]),
    }
    print(f"scan {letter}: {len(seeds)} seeds", file=sys.stderr)
json.dump(out, sys.stdout, separators=(",", ":"))
