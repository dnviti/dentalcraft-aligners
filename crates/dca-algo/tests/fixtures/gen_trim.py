#!/usr/bin/env python3
"""Generates trim.json: the plugin's trim lines (`case_print.py`) on three synthetic teeth,
to compare the Rust port (`dca_algo::trim`) against.

    python3 -B -I gen_trim.py [path/to/blender-ortho-aligner] > trim.json

Needs only python3 and numpy (no Blender): the plugin modules are loaded by path with `bpy`
and `mathutils` replaced by empty modules, and `mu.kdtree` (a `mathutils` KD-tree) by an
exact search over single-precision coordinates, like the original.

The teeth are the plugin's demo crowns (`demo.tooth_mesh`, 24 directions) of 31, 32 and 34,
the incisors proclined (an undercut on the labial face), the cervical ring scalloped
(+-0.6 mm), placed along a curve so that the loops of 31 and 32 touch. For every tooth:
the scanned crown (the vertices at or above the cervical ring), the closed tooth (every
vertex), the margin (the cervical ring), the frame origin and its buccal axis.

The expected values are computed with the plugin's own functions, glued as
`tooth_equators` (vertical insertion axis: no warp), `equator_trim`, `_bridge_contacts`,
`trim_points` (STRAIGHT and SCALLOPED, no scan border) and `aligner.trim_cutter` do it.
"""
import importlib
import json
import os
import sys
import types

import numpy as np

REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO", "/home/daniele/Documents/repos/blender-ortho-aligner")
PKG = os.path.join(REPO, "ortho_aligner")

sys.modules["bpy"] = types.ModuleType("bpy")
mathutils = types.ModuleType("mathutils")
mathutils.Matrix = mathutils.Vector = object
sys.modules["mathutils"] = mathutils
sys.modules["mathutils.kdtree"] = types.ModuleType("mathutils.kdtree")
pkg = types.ModuleType("oa_trim")
pkg.__path__ = [PKG]
sys.modules["oa_trim"] = pkg


def load(name):
    return importlib.import_module(f"oa_trim.{name}")


al, cp, demo, mu = (load(n) for n in ("aligner", "case_print", "demo", "meshutil"))


class KDTree:
    """Stand-in for mathutils.kdtree.KDTree: nearest point among single-precision
    coordinates, distance rounded to single precision."""

    def __init__(self, points):
        self.P = np.asarray(points, np.float32).astype(np.float64)

    def find(self, co):
        q = np.asarray(co, np.float32).astype(np.float64)
        d = np.sqrt(((self.P - q) ** 2).sum(axis=1))
        i = int(np.argmin(d))
        return None, i, float(np.float32(d[i]))


mu.kdtree = lambda points: KDTree(points)

N_THETA = 24
WALL = 0.6
EXT = 2.0


def rot(axis, deg):
    a = np.radians(deg)
    c, s = np.cos(a), np.sin(a)
    if axis == "x":
        return np.array([[1, 0, 0], [0, c, -s], [0, s, c]])
    return np.array([[c, -s, 0], [s, c, 0], [0, 0, 1]])


def tooth(fdi, pos, yaw, tilt):
    V, _, _ = demo.tooth_mesh(fdi, N_THETA)
    V = np.asarray(V, np.float64).copy()
    th = np.linspace(0, 2 * np.pi, N_THETA, endpoint=False)
    ring = slice(2 * N_THETA, 3 * N_THETA)              # the cervical ring (z = 0)
    V[ring, 2] += 0.6 * np.cos(2.0 * th)
    R = rot("z", yaw) @ rot("x", tilt)
    P = np.round(V @ R.T + np.asarray(pos, np.float64), 9)      # the values the fixture stores
    crown = np.arange(2 * N_THETA, len(V) - 1)         # the ring and above (not the bottom centre)
    return {
        "fdi": fdi,
        "crown": P[crown],
        "closed": P,
        "margin": P[ring],
        "origin": np.asarray(pos, np.float64),
        "e_bl": np.round(R[:, 1], 9),
    }


def equator(t):
    """case_print.tooth_equators for one tooth along +Z (no warp)."""
    n = cp.EQUATOR_AZIMUTHS
    ang = 2.0 * np.pi * np.arange(n) / n
    P, Pc, M = t["crown"], t["closed"], t["margin"]
    E, ext, Dir, c = al.survey_loop(P, n, cp.EQUATOR_BAND_MM, cp.EQUATOR_SIDE_MM)
    top = float(P[:, 2].max())
    a = np.arctan2(M[:, 1] - c[1], M[:, 0] - c[0])
    diff = np.abs((a[:, None] - ang[None, :] + np.pi) % (2.0 * np.pi) - np.pi)
    sector = diff <= 2.0 * np.pi / n
    zm = np.where(sector, M[:, 2:3], -np.inf).max(axis=0)
    empty = ~sector.any(axis=0)
    zm[empty] = M[np.argmin(diff[:, empty], axis=0), 2]
    Pc = Pc[Pc[:, 2] >= P[:, 2].min() - 0.5]
    Pc = Pc[Pc[:, 2] >= np.interp(np.arctan2(Pc[:, 1] - c[1], Pc[:, 0] - c[0]), ang, zm,
                                  period=2.0 * np.pi) - cp.EQUATOR_SCAN_GUM_MM]
    if len(Pc) >= 20:
        Ec, ext_c, _, _ = al.survey_loop(Pc, n, cp.EQUATOR_BAND_MM, cp.EQUATOR_SIDE_MM, centre=c)
        miss = ext_c - ext > cp.EQUATOR_SCAN_GAP_MM
        E[miss] = Ec[miss]
    zf = cp._slope_floor(zm, cp.EQUATOR_FLOOR_SLOPE_MM)

    def clamp(z):
        return np.minimum(np.maximum(z, zf), top - cp.EQUATOR_TOP_MM)
    E[:, 2] = clamp(cp._circular_smooth(clamp(E[:, 2])))
    return E, {"margin": zm, "top": top, "centre": c}


def equator_trim(E, info, off):
    """case_print.equator_trim for one tooth."""
    r = cp.EQUATOR_TAB_WALL * WALL
    E = E.copy()
    E[:, 2] = E[:, 2] - off
    F = E.copy()
    F[:, :2] = cp._loop_smooth_plan(E[:, :2], info["centre"], cp.EQUATOR_PLAN_PASSES)
    E[:, 2] = np.minimum(cp._round_tabs(F, r), info["top"] - cp.EQUATOR_TOP_MM)
    zm = info["margin"]
    E[:, 2] = np.maximum(E[:, 2], np.minimum(zm + cp.EQUATOR_GUM_GAP_MM, info["top"] - cp.EQUATOR_GUM_GAP_MM))
    return E


def margin_points(t, mode):
    """The loop body of case_print.trim_points for one tooth."""
    M = t["margin"]
    idx = np.arange(len(M))[::max(1, len(M) // 60)]
    P = M[idx]
    if mode == "SCALLOPED":
        return np.column_stack([P[:, :2], P[:, 2] - EXT])
    side = (P - t["origin"]) @ t["e_bl"] > 0
    out = []
    for sd in (side, ~side):
        if sd.any():
            zen = P[sd][int(np.argmin(P[sd, 2]))]
            out.append([zen[0], zen[1], zen[2] - EXT])
    return np.asarray(out, np.float64)


def r(a, nd=9):
    return np.round(np.asarray(a, np.float64), nd).tolist()


def main():
    teeth = [tooth(31, (2.9, 0.0, 0.0), 8.0, -14.0),
             tooth(32, (8.2, -1.6, 0.0), 30.0, -10.0),
             tooth(34, (17.5, -9.5, 0.0), 70.0, 0.0)]
    out = {"wall": WALL, "extension": EXT, "teeth": []}
    loops = {}
    for t in teeth:
        E, info = equator(t)
        rec = {k: r(t[k]) for k in ("crown", "closed", "margin", "origin", "e_bl")}
        rec["fdi"] = t["fdi"]
        rec["equator"] = r(E)
        rec["margin_z"] = r(info["margin"])
        rec["top"] = info["top"]
        rec["centre"] = r(info["centre"])
        rec["trim"] = {str(off): r(equator_trim(E, info, off)) for off in (0.0, 0.5, -0.5)}
        rec["straight"] = r(margin_points(t, "STRAIGHT"))
        rec["scalloped"] = r(margin_points(t, "SCALLOPED"))
        loops[t["fdi"]] = equator_trim(E, info, 0.0)
        out["teeth"].append(rec)
    out["bridged"] = r(cp._bridge_contacts([loops[f] for f in sorted(loops)]))
    # the trim surface of STRAIGHT and SCALLOPED (trim_cutter: the bottom of the slab)
    V = np.concatenate([t["closed"] for t in teeth])
    lo, hi = V.min(axis=0), V.max(axis=0)
    out["cutter"] = {"lo": r(lo), "hi": r(hi), "cell": 0.8}
    for mode, k in (("straight", 2), ("scalloped", 4)):
        ctrl = np.concatenate([np.asarray(rec[mode]) for rec in out["teeth"]])
        Vc, _ = al.trim_cutter(ctrl, lo, hi, hi[2] + 5.0, cell=0.8, k=k, reach=40.0)
        nn = len(Vc) // 2
        out["cutter"][mode] = r(Vc[:nn, 2])
    json.dump(out, sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main()
