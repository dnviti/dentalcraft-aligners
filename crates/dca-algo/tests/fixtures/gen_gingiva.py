#!/usr/bin/env python3
"""Generates gingiva.json: the plugin's gingiva (`reconstruct.gum_surface`,
`gum_weights`, `deform_prep` / `deform`) and its scan deformation
(`gingiva.margin_vertices`, `distance_matrix`, `blend_weights`, `deform`) on
golden demo cases, with a test motion, to compare `dca_algo::gingiva` against.

    PYTHONDONTWRITEBYTECODE=1 python3 -B gen_gingiva.py > gingiva.json

Needs python3 and numpy only: the plugin is loaded read-only through the golden
oracle package (`golden/oracle/plugin.py`, mathutils shim). Inputs per case are the
golden fixtures: `scan.dcm` moved to arch coordinates by the inverse of the
ground-truth frame (`Case.V_arch`), the oracle's segmentation labels
(`segmentation/segmentation.json`) and its teeth (`teeth/teeth.json`).

Output per case: global counts and checksums of the whole gum surface, of the
weights and of the deformed surfaces, and the values on an evenly spaced
sample of vertices (rest position, tooth and flags, non-zero weights, deformed
position). The test motion (arch coordinates): the two central teeth of the
sequence moved (0.6, -0.4, 0.2) mm and turned 6 degrees about their long axis
through the centre of resistance, the first molar of the sequence moved
(0, 0.5, 0) mm.
"""
import json
import os
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.normpath(os.path.join(HERE, "../../../../golden"))
sys.path.insert(0, GOLDEN)
sys.dont_write_bytecode = True

from oracle import mesh, plugin  # noqa: E402

CASES = ["demo_lower_mild", "demo_upper_moderate"]
SAMPLES = 1000


def rot(axis, deg):
    a = np.asarray(axis, np.float64)
    a = a / np.linalg.norm(a)
    t = np.radians(deg)
    K = np.array([[0, -a[2], a[1]], [a[2], 0, -a[0]], [-a[1], a[0], 0]])
    return np.eye(3) + np.sin(t) * K + (1 - np.cos(t)) * K @ K


def motion(teeth, seq):
    out = {}
    mid = len(seq) // 2
    for f in (seq[mid - 1], seq[mid]):
        t = teeth[str(f)]
        R = rot(t["e_ax"], 6.0)
        c = np.asarray(t["cres"], np.float64)
        M = np.eye(4)
        M[:3, :3] = R
        M[:3, 3] = c - R @ c + np.array([0.6, -0.4, 0.2])
        out[f] = M
    M = np.eye(4)
    M[:3, 3] = [0.0, 0.5, 0.0]
    out[seq[0]] = M
    return out


def r(x, nd=9):
    return [round(float(v), nd) for v in np.ravel(x)]


def case(name):
    rc, gv, mu = plugin.load("reconstruct"), plugin.load("gingiva"), plugin.load("meshutil")
    d = os.path.join(GOLDEN, "fixtures", name)
    meta = json.load(open(os.path.join(d, "meta.json")))
    sc = mesh.read_mesh(os.path.join(d, "scan.dcm"))
    V, T = np.asarray(sc["V"], np.float64), np.asarray(sc["T"], np.int64)
    Minv = mu.rigid_inverse(np.array(meta["frame_gt"], np.float64))
    Va = mu.transform_points(Minv, V)
    seg = json.load(open(os.path.join(d, "segmentation", "segmentation.json")))
    lab = mesh.unpack_labels(seg["vertex_labels"]).astype(np.int64)
    tj = json.load(open(os.path.join(d, "teeth", "teeth.json")))
    seq = [int(f) for f in tj["sequence"]]
    E = mu.unique_edges(T)
    t0 = time.time()
    G = rc.gum_surface(Va, T, lab, E)
    t1 = time.time()
    W = rc.gum_weights(G, seq)
    t2 = time.time()
    deltas = motion(tj["teeth"], seq)
    prep = rc.deform_prep(G["V"], W, seq)
    Vd = rc.deform(G["V"], W, seq, deltas, prep)
    # scan deformation (case_teeth.CaseData fs, D, A; case_plan.stage_vertices)
    samples = gv.margin_vertices(lab, E)
    gum_idx = np.nonzero(lab == 0)[0]
    crown_idx = {f: np.nonzero(lab == f)[0] for f in seq}
    fs, D = gv.distance_matrix(Va, gum_idx, samples, radius=8.0)
    A = gv.blend_weights(np.where(np.isfinite(D), D, 1e9), radius=6.0)
    Vs = gv.deform(Va, crown_idx, gum_idx, fs, A, deltas)
    t3 = time.time()
    n2 = len(G["V"])
    idx = np.unique(np.linspace(0, n2 - 1, SAMPLES).astype(np.int64))
    gi = np.unique(np.linspace(0, len(gum_idx) - 1, SAMPLES).astype(np.int64))
    return {
        "case": name, "fdis": seq,
        "motion": {str(f): M.tolist() for f, M in deltas.items()},
        "gum": {
            "n_vertices": int(n2), "n_faces": int(len(G["T"])), "n_scan": int(G["n_scan"]),
            "n_margin": int(G["margin"].sum()), "n_membrane": int(G["membrane"].sum()),
            "n_col": int(G["col"].sum()),
            "tooth_hist": {str(int(f)): int(c) for f, c in zip(*np.unique(G["tooth"], return_counts=True))},
            "rest_sum": r(G["V"].sum(axis=0)),
            "weights_col_sum": r(W.astype(np.float64).sum(axis=0), 6),
            "deformed_sum": r(Vd.sum(axis=0)),
            "sample_idx": idx.tolist(),
            "sample_rest": r(G["V"][idx]),
            "sample_tooth": G["tooth"][idx].astype(int).tolist(),
            "sample_flags": (G["margin"][idx].astype(int) + 2 * G["membrane"][idx].astype(int)
                             + 4 * G["col"][idx].astype(int)).tolist(),
            "sample_weights": [{str(j): round(float(W[i, j]), 7) for j in np.nonzero(W[i])[0]} for i in idx],
            "sample_deformed": r(Vd[idx]),
        },
        "scan": {
            "fdis": [int(f) for f in fs],
            "n_gum": int(len(gum_idx)),
            "weights_col_sum": r(A.astype(np.float64).sum(axis=0), 6),
            "deformed_sum": r(Vs.sum(axis=0)),
            "sample_gum": gi.tolist(),
            "sample_weights": [{str(j): round(float(A[k, j]), 7) for j in np.nonzero(A[k])[0]} for k in gi],
            "sample_deformed": r(Vs[gum_idx[gi]]),
        },
        "seconds": {"gum_surface": round(t1 - t0, 1), "gum_weights": round(t2 - t1, 1),
                    "scan_blend": round(t3 - t2, 1)},
    }


def main():
    out = {"note": "generated by gen_gingiva.py from the plugin (read-only) and the golden fixtures",
           "cases": [case(c) for c in CASES]}
    json.dump(out, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
