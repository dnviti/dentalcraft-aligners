#!/usr/bin/env python3
"""Generates visits.json: the plugin's analysis of a control visit
(`visit._register_compute`, `visit._track_compute`) on a golden demo case, to
compare `dca_algo::visits` against.

    PYTHONDONTWRITEBYTECODE=1 python3 -B gen_visits.py > visits.json

Needs python3 and numpy only: the plugin is loaded read-only through the golden
oracle package (`golden/oracle/plugin.py`, mathutils shim); `visit.py` needs
`bpy`, so only its array functions are pulled out of the source with
`plugin.extract`. The visit is simulated without any random element in the
scan itself, so that Rust can rebuild it from the motions stored here:

* the expected model: the golden scan (arch coordinates) with every tooth
  moved by its planned rigid motion `planned[f]` (the gingiva stays);
* the control scan: the same with every tooth moved again by `deviation[f]`,
  then the whole scan placed by `placement`.

The tooth frames are the golden teeth's rest frames, the crowns the labelled
vertices with the triangles that have the three of them.

Output: the registration (transform, rms, normal sign) and, per tooth, the
transform of the second pass, the rms, the tracking figures and the
deviation components.
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

CASE = "demo_lower_mild"
SEED = 7


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


def r(x, nd=10):
    return [round(float(v), nd) for v in np.ravel(x)]


def main():
    mu, an, gq = plugin.load("meshutil"), plugin.load("anatomy"), plugin.load("gpu_query")
    au, ev = plugin.load("autoseed"), plugin.load("evidence")
    ns = {"np": np, "mu": mu, "an": an, "gq": gq, "autoseed": au, "P": ev.P}
    plugin.extract("visit", ["kabsch", "point_to_plane", "_nn", "icp", "_sub_idx", "_tooth_deviation",
                             "_register_compute", "_track_compute"], ns)

    d = os.path.join(GOLDEN, "fixtures", CASE)
    meta = json.load(open(os.path.join(d, "meta.json")))
    sc = mesh.read_mesh(os.path.join(d, "scan.dcm"))
    V, T = np.asarray(sc["V"], np.float64), np.asarray(sc["T"], np.int64)
    V = mu.transform_points(mu.rigid_inverse(np.array(meta["frame_gt"], np.float64)), V)
    seg = json.load(open(os.path.join(d, "segmentation", "segmentation.json")))
    lab = mesh.unpack_labels(seg["vertex_labels"]).astype(np.int64)
    tj = json.load(open(os.path.join(d, "teeth", "teeth.json")))
    seq = [int(f) for f in tj["sequence"]]
    rng = np.random.RandomState(SEED)

    F = {f: np.array(tj["teeth"][str(f)]["rest_frame"], np.float64) for f in seq}
    md = {f: int(tj["teeth"][str(f)]["md_sign"]) for f in seq}
    planned, deviation = {}, {}
    Ve, Vs = V.copy(), V.copy()
    for f in seq:
        idx = np.nonzero(lab == f)[0]
        c = V[idx].mean(axis=0)
        planned[f] = about(c, [0, 0, 1], rng.uniform(-4, 4), rng.uniform(-0.6, 0.6, 3))
        ce = mu.transform_points(planned[f], c[None])[0]
        # two teeth clearly behind, the others within the thresholds
        if f == seq[3]:
            dev = about(ce, [0.3, -0.4, 1], 0.5, [0.9, -0.5, 0.2])
        elif f == seq[9]:
            dev = about(ce, [0.2, 0.3, 1], 4.0, [0.1, 0.1, 0.0])
        else:
            dev = about(ce, [rng.uniform(-1, 1), rng.uniform(-1, 1), 1], rng.uniform(-0.8, 0.8),
                        rng.uniform(-0.2, 0.2, 3))
        deviation[f] = dev
        Ve[idx] = mu.transform_points(planned[f], V[idx])
        Vs[idx] = mu.transform_points(dev @ planned[f], V[idx])
    placement = np.eye(4)
    placement[:3, :3] = rot([0.3, 0.2, 1.0], 25.0)
    placement[:3, 3] = [14.0, -9.0, 6.0]
    Vscan = mu.transform_points(placement, Vs)

    t0 = time.time()
    Tr, err, sign = ns["_register_compute"](np.eye(4), Ve, T, Vscan, T)
    t1 = time.time()
    Vreg = mu.transform_points(Tr, Vscan)
    Nn = sign * mu.vertex_normals(Vreg, T)
    teeth = []
    for f in seq:
        idx = np.nonzero(lab == f)[0]
        ploc = mu.transform_points(mu.rigid_inverse(F[f]), V[idx])
        keep = np.isin(T, idx).all(axis=1)
        remap = -np.ones(len(V), np.int64)
        remap[idx] = np.arange(len(idx))
        nloc = mu.vertex_normals(ploc, remap[T[keep]])
        E = planned[f] @ F[f]
        teeth.append({"f": f, "E": E, "P_exp": mu.transform_points(E, ploc), "ploc": ploc, "nloc": nloc,
                      "F_p": F[f], "md": md[f]})
    out = ns["_track_compute"](Vreg, Nn, teeth, an.ARCH_ORDER[an.LOWER], True)
    t2 = time.time()

    res = {}
    for f, x in out.items():
        res[str(f)] = {
            "T": r(x["T"]), "E": r(x["E"]), "in_pari": x["in_pari"], "scostamento_mm": x["scostamento_mm"],
            "scostamento_gradi": x["scostamento_gradi"], "efficacia_pct": x["efficacia_pct"],
            "movimento_pianificato_mm": x["movimento_pianificato_mm"], "rms_icp_mm": x["rms_icp_mm"],
            "deviazione": x["deviazione"],
        }
    json.dump({
        "note": "generated by gen_visits.py from the plugin (read-only) and the golden fixtures",
        "case": CASE, "seed": SEED, "fdis": seq,
        "planned": {str(f): r(M, 12) for f, M in planned.items()},
        "deviation": {str(f): r(M, 12) for f, M in deviation.items()},
        "placement": r(placement, 12),
        "registration": {"T": r(Tr), "rms": float(err), "sign": float(sign)},
        "teeth": res,
        "seconds": {"register": round(t1 - t0, 1), "track": round(t2 - t1, 1)},
    }, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
