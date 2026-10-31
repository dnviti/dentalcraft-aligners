#!/usr/bin/env python3
"""Generates aligner_bite.json: the plugin's bite with the aligners worn (`aligner_bite.py`), the
shape of the bite ramps (`ramp_shape.py`) and their placement (`ramps.propose`), to compare the Rust
port (`dca_algo::aligner_bite`, `dca_algo::ramps`) against.

    python3 -B -I gen_aligner_bite.py [path/to/blender-ortho-aligner] > aligner_bite.json

Needs only python3 and numpy (no Blender). `aligner_bite.py` and `ramp_shape.py` are plain numpy and
are imported as they are. `ramps.py` and `case_occlusal.py` import Blender-bound modules, so the
functions compared are taken from their source by name (`ast`) and run as they are: `propose` (with
stand-ins for the case objects of `case_core`, `case_print`, `case_teeth` and `case_occlusal` that
return the arches below) and `_with_ramps`, `_smooth_grind`.

Input: the two demo arches of `occlusion.json` (see `gen_occlusion.py`: the plugin's synthetic arches,
single precision, upper arch turned onto the lower one 0.6 mm into it), read from that file so that it
is not stored twice. The fixture holds only what the Rust test cannot rebuild from it: the parameters
and the results (probes and totals of the height maps, the closings, the equilibration, the ramps).
"""
import ast
import base64
import importlib
import json
import os
import sys
import types

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO", "/home/daniele/Documents/repos/blender-ortho-aligner")
PKG = os.path.join(REPO, "ortho_aligner")

sys.modules["bpy"] = types.ModuleType("bpy")
pkg = types.ModuleType("oa_prova")
pkg.__path__ = [PKG]
sys.modules["oa_prova"] = pkg


def load(name):
    return importlib.import_module(f"oa_prova.{name}")


mu, ab, rs = (load(n) for n in ("meshutil", "aligner_bite", "ramp_shape"))


def functions(path, names, ns):
    """The top-level functions `names` of a plugin module, from its source, run in `ns`."""
    tree = ast.parse(open(path, encoding="utf-8").read())
    keep = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in names]
    exec(compile(ast.Module(body=keep, type_ignores=[]), path, "exec"), ns)
    missing = [n for n in names if n not in ns]
    assert not missing, missing
    return ns


# ---------------------------------------------------------------------------------------------
# the arches
# ---------------------------------------------------------------------------------------------

def unb64(s, dtype, shape):
    return np.frombuffer(base64.b64decode(s), dtype=dtype).reshape(shape)


occ = json.load(open(os.path.join(HERE, "occlusion.json")))


def arch(rec):
    n, m = rec["n_vertices"], rec["n_tris"]
    V = unb64(rec["vertices"], "<f4", (n, 3)).astype(np.float64)
    T = unb64(rec["tris"], "<u2", (m, 3)).astype(np.int64)
    L = unb64(rec["labels"], "u1", (n,)).astype(np.int64)
    return V, T, L


Vl, Tl, Ll = arch(occ["lower"])
Vu, Tu, Lu = arch(occ["upper"])
REL = np.array(occ["relation"], np.float64).reshape(4, 4)        # upper -> lower arch coordinates
Pl, Ql = Vl[Ll != 0], Ll[Ll != 0]
Pu, Qu = Vu[Lu != 0], Lu[Lu != 0]
COVERED = sorted(set(int(f) for f in Ql) | set(int(f) for f in Qu))

CELL = 0.5
CLEAR, WALL, MINIMUM = 0.0, 1.0, 0.5
MARGIN = 3.0 + CLEAR + WALL


def grid_of(P):
    lo = np.floor((P.min(axis=0)[:2] - MARGIN) / CELL) * CELL
    hi = P.max(axis=0)[:2] + MARGIN
    return ab.Grid(lo, hi, CELL), lo.tolist(), hi.tolist()


Gl, lo_l, hi_l = grid_of(Pl)
Gu, lo_u, hi_u = grid_of(Pu)

PROBES = np.random.default_rng(3).integers(0, Gl.n[0] * Gl.n[1], 40).tolist()


def stats(A):
    """Totals and probes of a raster (the Rust test compares the same numbers)."""
    A = np.asarray(A)
    fin = np.isfinite(A) if A.dtype.kind == "f" else np.ones(A.shape, bool)
    flat = A.ravel()
    return {"n": int(fin.sum()), "sum": float(A[fin].sum()),
            "probes": [None if (A.dtype.kind == "f" and not np.isfinite(flat[i])) else float(flat[i])
                       for i in PROBES if i < flat.size]}


def surf_stats(S):
    return {"z": stats(S["Z"]), "l": stats(S["L"]), "z_wall": stats(S["Z_wall"]),
            "z_min": stats(S["Z_min"]), "cov": int(S["cov"].sum())}


def closing_stats(c):
    if c is None:
        return None
    return {"value": float(c["value"]), "sides": c["sides"], "upper": {str(k): v for k, v in c["upper"].items()},
            "lower": {str(k): v for k, v in c["lower"].items()},
            "side_gap": {k: v for k, v in c["side_gap"].items()},
            "n_front": int(len(c["front_g"])), "g_sum": float(c["g"][np.isfinite(c["g"])].sum()),
            "g_n": int(np.isfinite(c["g"]).sum())}


# ---------------------------------------------------------------------------------------------
# aligner_bite
# ---------------------------------------------------------------------------------------------
out = {"params": {"cell": CELL, "clearance": CLEAR, "wall": WALL, "minimum": MINIMUM,
                  "grid_lower": {"lo": lo_l, "hi": hi_l}, "grid_upper": {"lo": lo_u, "hi": hi_u},
                  "probes": PROBES, "covered": COVERED}}

# envelope: balls and ellipsoids, with labels
Z0, L0 = ab.envelope(Pl, 0.0, Gl, Ql)
Zb, Lb = ab.envelope(Pl, 1.0, Gl, Ql)
Ze = ab.envelope(Pl, 1.0, Gl, rz=0.75)
out["envelope"] = {"bare": {"z": stats(Z0), "l": stats(L0)}, "ball": {"z": stats(Zb), "l": stats(Lb)},
                   "ellipsoid": stats(Ze)}
# heights closed by the blockout ball, and the labels filled
S, SL = ab.heights(Pl, Ql, Gl, 0.3)
S8, SL8 = ab.heights(Pl, Ql, Gl, 0.8)
out["heights"] = {"b03": {"z": stats(S), "l": stats(SL)}, "b08": {"z": stats(S8), "l": stats(SL8)}}
Sh = ab.ball_closing(np.where(np.isfinite(Z0), Z0, -np.inf), CELL, 0.8)
out["ball_closing"] = stats(Sh)
# the bases and surfaces
cov_l = set(COVERED) - {17, 27, 37, 47}                         # the last molars bare
base_l = ab.base_from_heights(S, SL, cov_l, CLEAR, WALL, MINIMUM, Gl)
Sf_l = ab.with_occlusal(base_l, 0.75)
Sf_w = ab.with_occlusal(base_l, WALL)
Sf_m = ab.with_occlusal(base_l, 0.4)
out["surface"] = {"occlusal_075": surf_stats(Sf_l), "wall": surf_stats(Sf_w), "occlusal_040": surf_stats(Sf_m),
                  "bare": surf_stats(ab.bare_surface(base_l)), "covered": sorted(cov_l)}
direct = ab.occlusal_surface(Pl, Ql, cov_l, CLEAR, WALL, 0.75, MINIMUM, Gl)
out["occlusal_surface_direct"] = surf_stats(direct)
lim = ab.limit_heights(Sf_l)
out["limit_heights"] = {"n": int(np.isfinite(lim).sum()), "sum": float(lim[np.isfinite(lim)].sum())}

# the upper arch the same way, then the closing (vertical and hinge), with and without the front teeth
Su, SLu = ab.heights(Pu, Qu, Gu, 0.3)
base_u = ab.base_from_heights(Su, SLu, set(COVERED), CLEAR, WALL, MINIMUM, Gu)
bl = ab.base_from_heights(S, SL, set(COVERED), CLEAR, WALL, MINIMUM, Gl)
sl = ab.with_occlusal(bl, 0.75)
su = ab.with_occlusal(base_u, 0.75)
bare_l, bare_u = ab.bare_surface(bl), ab.bare_surface(base_u)
mask = ab.overlap_mask(bare_l, bare_u, REL, 0.25, WALL + 0.5)
mask &= ~ab.is_front(base_u["Lall"], upper=True) & ~ab.is_front(base_u["Lw"], upper=True)
out["mask"] = {"n_true": int(mask.sum()), "n": int(mask.size)}
incisal = Pl[(np.isin(Ql, [31, 41]))]
inc = incisal[incisal[:, 2] >= np.quantile(incisal[:, 2], 0.8)].mean(axis=0)
axis = ab.hinge_axis(inc, 78.0, 38.0)
out["hinge"] = {"incisal": inc.tolist(), "axis": axis.tolist()}
vert = ab.closing(sl, su, REL, "VERTICAL", None, 0.1, mask=mask)
hinge = ab.closing(sl, su, REL, "HINGE", axis, 0.1, mask=mask)
out["closing"] = {"vertical": closing_stats(vert), "hinge": closing_stats(hinge)}


def front_of(r, k):
    fl, fu = ab.is_front(Ql, upper=False), ab.is_front(Qu, upper=True)
    return ab.Front(Pl[fl], Ql[fl], np.full(fl.sum(), r), Pu[fu], Qu[fu], np.full(fu.sum(), r), REL, k)


front = front_of(0.3, 1.0)
cf = ab.closing(sl, su, REL, "HINGE", axis, 0.1, mask=mask, front=front)
out["closing_front"] = closing_stats(cf)
out["front"] = {"r": 0.3, "k": 1.0,
                "worst": [front.worst("HINGE", v, axis) for v in (-0.05, 0.0, 0.05)]}
# equilibration
eq = ab.equilibrate(sl, su, REL, "HINGE", axis, 0.1, 2, 0.3, mask=mask, front=front)
out["equilibrate"] = {"done": bool(eq["done"]), "value": eq["value"], "steps": int(eq["steps"]),
                      "stop": eq["stop"], "grind_l": stats(eq["grind_l"]), "grind_u": stats(eq["grind_u"]),
                      "closing": closing_stats(eq["closing"]), "before": closing_stats(eq["before"])}
for name, model, ax in (("equilibrate_vertical", "VERTICAL", None), ("equilibrate_hinge", "HINGE", axis)):
    e2 = ab.equilibrate(sl, su, REL, model, ax, 0.1, 2, 0.3, mask=mask)
    out[name] = {"done": bool(e2["done"]), "value": e2["value"], "steps": int(e2["steps"]), "stop": e2["stop"],
                 "grind_l": stats(e2["grind_l"]), "grind_u": stats(e2["grind_u"]),
                 "closing": closing_stats(e2["closing"]), "before": closing_stats(e2["before"])}
# the openings of the aligners beyond the teeth
teeth = ab.closing(bare_l, bare_u, REL, "HINGE", axis, 0.1, mask=mask)
out["teeth_closing"] = closing_stats(teeth)
out["opening_at"] = [ab.opening_at(inc, "HINGE", v, axis) for v in (0.0, 0.02, -0.03)]
out["solve_thickness"] = [[a, b, 1.0, 0.5, t, ab.solve_thickness(a, b, 1.0, 0.5, t)]
                          for a, b, t in ((3.0, 1.0, 1.5), (1.0, 0.2, 1.5), (3.0, 2.0, 1.5), (2.5, 0.5, 1.5),
                                          (2.0, 1.2, 1.5))]
# first_contact on the lower height map for a few point sets
Pq = (Pu @ REL[:3, :3].T + REL[:3, 3])[::7]
out["first_contact"] = {"vertical": ab.first_contact(Gl, sl["Z"], Pq, "VERTICAL"),
                        "hinge": ab.first_contact(Gl, sl["Z"], Pq, "HINGE", axis)}
out["pose"] = {"vertical": ab.pose(Pq[:3], "VERTICAL", 0.3).tolist(),
               "hinge": ab.pose(Pq[:3], "HINGE", 0.05, axis).tolist(), "q": Pq[:3].tolist()}

# the ramps on the surface (case_occlusal._with_ramps) and the smoothed grinding
ns = functions(os.path.join(PKG, "case_occlusal.py"), ["_with_ramps", "_smooth_grind"], {"np": np, "ab": ab})
Rp = np.array([[8.0, 28.0, 14.0], [-8.0, 28.0, 14.0]])
Rl = np.array([31, 41])
wr = ns["_with_ramps"](sl, Rp, Rl)
out["with_ramps"] = {"points": Rp.tolist(), "labels": Rl.tolist(), "surface": surf_stats(wr)}
grind = np.zeros(Gl.n)
grind[10:14, 12:20] = 0.2
out["smooth_grind"] = {"sum": float(ns["_smooth_grind"](grind).sum()),
                       "probes": [float(ns["_smooth_grind"](grind).ravel()[i]) for i in PROBES[:20]]}

# ---------------------------------------------------------------------------------------------
# ramp_shape
# ---------------------------------------------------------------------------------------------
V, T = rs.ramp_mesh()
out["ramp_shape"] = {
    "n_vertices": len(V), "n_tris": len(T), "vertex_sum": np.round(V.sum(axis=0), 12).tolist(),
    "vertices": np.round(V, 12).tolist(), "tris": T.tolist(),
    "frames": [{"origin": o, "normal": n, "toward": t, "m": np.round(rs.frame(o, n, t), 12).tolist()}
               for o, n, t in (((1.0, 2.0, 3.0), (0.0, 0.0, 1.0), None),
                               ((0.0, 5.0, -1.0), (0.3, -0.8, 0.5), (0.0, 1.0, 0.0)),
                               ((2.0, 2.0, 2.0), (1.0, 0.0, 0.0), (1.0, 0.0, 0.0)))],
    "overbite": [rs.overbite(Pl[np.isin(Ql, [31, 41])], (Pu[np.isin(Qu, [11, 21])] @ REL[:3, :3].T + REL[:3, 3])),
                 rs.overbite([], [(0, 0, 0)])],
}

# ---------------------------------------------------------------------------------------------
# ramps.propose, run as it is on the arches
# ---------------------------------------------------------------------------------------------


class Item:
    rest = np.eye(4).ravel()
    obj = object()


def crown_cells(P, step):
    key = np.floor(P / step).astype(np.int64)
    _, first = np.unique(key, axis=0, return_index=True)
    return P[np.sort(first)]


def tooth_shell(V, T, L, f):
    """Closed shell of tooth f, wound outwards, with its own vertices."""
    Tf = T[(L[T] == f).all(axis=1)]
    if mu.signed_volume(V, Tf) < 0:
        Tf = Tf[:, ::-1]
    used = np.unique(Tf)
    remap = np.full(len(V), -1)
    remap[used] = np.arange(len(used))
    return V[used], remap[Tf]


crowns_l = {int(f): crown_cells(Vl[Ll == f], 0.2) for f in np.unique(Ll[Ll != 0])}
crowns_u = {int(f): crown_cells(Vu[Lu == f], 0.2) for f in np.unique(Lu[Lu != 0])}
local_u = {int(f): tooth_shell(Vu, Tu, Lu, f) for f in (11, 21, 12, 22, 13, 23)}

lower, upper = types.SimpleNamespace(name="lower"), types.SimpleNamespace(name="upper")
fakes = {
    "case_core": {"arch_matrix": lambda o: REL if o is upper else np.eye(4),
                  "tooth_items": lambda o: {f: Item for f in local_u},
                  "unflat": lambda r: np.eye(4)},
    "case_print": {"aligner_teeth": lambda o: CURRENT["covered"],
                   "crown_points": lambda o, d, step=0.4, per_tooth=False: crowns_l if o is lower else crowns_u},
    "case_teeth": {"tooth_local": lambda o: local_u},
    "case_occlusal": {"_stage_pair": lambda lo, up, k, obj: (0, 0), "_deltas": lambda o, k: {}},
}
for name, attrs in fakes.items():
    m = types.ModuleType(f"oa_prova.{name}")
    m.__dict__.update(attrs)
    sys.modules[f"oa_prova.{name}"] = m
CURRENT = {}
ns = functions(os.path.join(PKG, "ramps.py"), ["propose"],
               {"np": np, "rs": rs, "LOWER_INCISORS": (31, 41, 32, 42), "UPPER_INCISORS": (11, 21, 12, 22),
                "__package__": "oa_prova", "__name__": "oa_prova.ramps_prova"})
propose = ns["propose"]


def case(name, covered, s_over, shift=0.0):
    """propose on the arches with the upper arch `shift` mm lower in the lower arch coordinates."""
    global REL
    saved = REL.copy()
    REL = REL.copy()
    REL[2, 3] += shift
    CURRENT["covered"] = set(covered)
    s = types.SimpleNamespace(ramp_overbite_min=s_over, ramp_width=3.0, ramp_depth=2.0, ramp_height=1.5,
                              clearance=0.0, shell_thickness=1.0)
    res, ob, why = propose(lower, upper, s, 0)
    rec = {"name": name, "covered": sorted(covered), "overbite_min": s_over, "shift": shift,
           "overbite": None if ob is None else float(ob), "why": why,
           "ramps": [{"fdi": int(f), "m": np.round(M, 12).ravel().tolist(), "size": [float(x) for x in size]}
                     for f, M, size in res]}
    REL = saved
    return rec


# upper incisors of the demo arches in occlusion: shift the upper arch down so the bite is deep
cases = [case("deep_all_covered", COVERED, 1.0, -2.0),
         case("deep_11_bare", [f for f in COVERED if f != 11], 1.0, -2.0),
         case("not_deep", COVERED, 4.0, 0.0),
         case("deep_tight_threshold", COVERED, 3.0, -2.0)]
out["propose"] = {
    "relation_note": "the relation of the occlusion.json arches; `shift` is added to its Z translation",
    "incisors": {str(f): {"vertices": local_u[f][0].tolist(), "tris": local_u[f][1].tolist()}
                 for f in (11, 21, 12, 22)},
    "cases": cases,
}

json.dump(out, sys.stdout, separators=(",", ":"))
