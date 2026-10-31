#!/usr/bin/env python3
"""Generates foundations.json: the plugin's results for the foundations of dca-algo
(anatomy, archform, evidence, frame, demo), to compare the Rust port against.

    python3 -I gen_foundations.py [path/to/blender-ortho-aligner] > foundations.json

Needs only python3 and numpy (no Blender): the plugin modules are loaded by path
the way `tests/unit/_carica.py` does, `worldframe.py` with a stubbed `bpy` and a
stubbed `case` module, `compute_frame` of `case_core.py` by extracting its source.
The random draws of the demo generator are replaced by fixed ones (`U`, `Z`), which
the Rust test replays, so the comparison does not depend on numpy's generator.
"""
import ast
import hashlib
import importlib
import json
import os
import sys
import types

import numpy as np

REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO", "/home/daniele/Documents/repos/blender-ortho-aligner")
PKG = os.path.join(REPO, "ortho_aligner")

# ---- loading ---------------------------------------------------------------------------------------
sys.modules["bpy"] = types.ModuleType("bpy")
pkg = types.ModuleType("oa_prova")
pkg.__path__ = [PKG]
sys.modules["oa_prova"] = pkg


def load(name):
    return importlib.import_module(f"oa_prova.{name}")


an, af, mu, ev, demo = (load(n) for n in ("anatomy", "archform", "meshutil", "evidence", "demo"))

# worldframe: `from . import case` gets a stub with what delta()/arch_centre() read
case_stub = types.ModuleType("oa_prova.case")
case_stub.arch_matrix = lambda obj: obj.M
case_stub.get_seeds = lambda obj: obj.seeds
case_stub.is_case = lambda o: True
case_stub.bl_matrix = lambda M: M
sys.modules["oa_prova.case"] = case_stub
pkg.case = case_stub
wf = load("worldframe")


def compute_frame_fn():
    """`compute_frame` of case_core.py, which imports bpy and mathutils at module level."""
    src = open(os.path.join(PKG, "case_core.py")).read()
    tree = ast.parse(src)
    fn = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "compute_frame")
    ns = {"np": np, "mu": mu, "an": an, "CaseError": RuntimeError,
          "flat": lambda a: [float(x) for x in np.asarray(a).ravel()], "bl_matrix": lambda M: M}
    exec(compile(ast.Module([fn], []), "case_core.compute_frame", "exec"), ns)
    return ns["compute_frame"]


compute_frame = compute_frame_fn()


def L(a):
    return np.asarray(a, dtype=np.float64).tolist()


rng = np.random.default_rng(20261031)
out = {}

# ---- anatomy -----------------------------------------------------------------------------------------
fdis = [q * 10 + i for q in (1, 2, 3, 4) for i in range(1, 9)]
out["anatomy"] = {
    "order": {a: an.ARCH_ORDER[a] for a in ("UPPER", "LOWER")},
    "teeth": {str(f): {
        "arch": an.arch_of(f), "right": an.is_right(f), "anterior": an.is_anterior(f),
        "incisor": an.is_incisor(f), "round": an.is_round_crown(f), "molar": an.is_molar(f),
        "name": an.tooth_name(f), "crown_root": list(an.crown_root_length(f)),
        "diameters": list(an.typical_diameters(f)),
        "enamel": [an.enamel_thickness(f, "M"), an.enamel_thickness(f, "D")],
        "ipr": [an.max_ipr_surface(f, "M"), an.max_ipr_surface(f, "D")],
        "ipr_tight": an.max_ipr_surface(f, "D", 0.25, 0.2),
    } for f in fdis},
}
seqs = []
for fl, arch in (([11, 21, 12, 22, 16, 26, 18, 28], "UPPER"), ([41, 31, 42, 32, 43, 36, 46], "LOWER"),
                 ([12, 22], "UPPER"), ([11, 12, 13, 14, 15, 16, 17, 18], "UPPER")):
    seq = an.arch_sequence(fl, arch)
    seqs.append({"arch": arch, "teeth": fl, "seq": seq,
                 "mesial": {str(f): an.mesial_neighbor(seq, f) for f in seq},
                 "distal": {str(f): an.distal_neighbor(seq, f) for f in seq}})
out["anatomy"]["sequences"] = seqs

# ---- archform ----------------------------------------------------------------------------------------
xs = np.linspace(-30, 30, 25)
xy = np.stack([np.linspace(-24, 24, 17), 30 - 0.02 * np.linspace(-24, 24, 17) ** 2 + rng.normal(0, 0.3, 17)], 1)
w = rng.uniform(0.5, 2.0, 17)
curves = {}
for name, model, params, half, post in (
        ("beta", "BETA", (50.0, 33.0, 0.0), 25.0, False),
        ("quartic", "QUARTIC", (31.0, -0.032, 1.1e-6), 26.0, False),
        ("quartic_posterior", "QUARTIC", (31.0, -0.032, 1.1e-6), 26.0, True),
        ("even_posterior", "EVEN", af.poly_even_through([[0, 38], [10, 34], [20, 22], [28, 3]]), 28.0, True)):
    c = af.curve_from_model(model, params, half, posterior=post)
    ss = np.array([-40.0, -23.5, -7.0, 0.0, 3.3, 12.0, 30.0, 58.0])
    pts = c.point(ss)
    probe = np.array([[0.0, 20.0], [10.0, 25.0], [-14.0, 18.0], [5.0, 40.0], [-20.0, 0.0]])
    curves[name] = {
        "n": len(c.P), "first": L(c.P[0]), "last": L(c.P[-1]), "mid": L(c.P[len(c.P) // 2]),
        "s_first": float(c.s[0]), "s_last": float(c.s[-1]), "s": L(ss), "points": L(pts),
        "tangent": L(c.tangent(ss)), "normal": L(c.normal_out(ss)),
        "probe": L(probe), "project": L(c.project(probe)),
        "chords": [[s0, ch, d, float(c.s_at_chord(s0, ch, d))]
                   for s0, ch, d in ((0.0, 8.5, 1), (0.0, 5.0, -1), (12.0, 7.0, 1), (-9.0, 9.0, -1))],
    }
out["archform"] = {
    "beta_x": L(xs), "beta_y": L(af.beta_y(xs, 50.0, 35.0)),
    "quartic_y": L(af.quartic_y(xs, (30.0, -0.04, 1.5e-6))),
    "fit_xy": L(xy), "fit_w": L(w), "fit": L(af.fit_quartic(xy)), "fit_weighted": L(af.fit_quartic(xy, w)),
    "fit_two": L(af.fit_quartic(np.array([[1.0, 2.0], [2.0, 5.0]]))),
    "fit_one": L(af.fit_quartic(np.array([[2.0, 4.0]]))),
    "through": L(af.quartic_through(40.0, 8.0, 36.0, 25.0, 5.0)),
    "through_singular": L(af.quartic_through(10.0, 0.0, 5.0, 0.0, 4.0)),
    "even_points": [[0, 38], [10, 34], [20, 22], [28, 3]],
    "even_coef": L(af.poly_even_through([[0, 38], [10, 34], [20, 22], [28, 3]])),
    "even_y": L(af.poly_even_y(xs, af.poly_even_through([[0, 38], [10, 34], [20, 22], [28, 3]]))),
    "curves": curves,
}

# ---- evidence ----------------------------------------------------------------------------------------
out["evidence"] = {
    "n_refs": len(ev.REFERENCES), "n_params": len(ev.P),
    "ref_keys": list(ev.REFERENCES), "param_keys": list(ev.P),
    "short": {k: r.short() for k, r in ev.REFERENCES.items()},
    "full": {k: r.full() for k, r in ev.REFERENCES.items()},
    "tooltips": {k: p.tooltip() for k, p in ev.P.items()},
    "values": {k: p.value for k, p in ev.P.items()},
    "units": {k: p.unit for k, p in ev.P.items()},
    "refs": {k: list(p.refs) for k, p in ev.P.items()},
    "to_confirm": [k for k, p in ev.P.items() if p.da_confermare],
    "text_lines": len(ev.references_text().split("\n")),
    "text_chars": len(ev.references_text()),
    "text_sha256": hashlib.sha256(ev.references_text().encode()).hexdigest(),
    "text_head": ev.references_text().split("\n")[:6],
    "text_tail": ev.references_text().split("\n")[-4:],
    "cite": ev.cite("simon2014", "castroflorio2023", "wang2025consensus"),
}

# ---- frame -------------------------------------------------------------------------------------------
cloud = rng.normal(size=(300, 3)) * np.array([12.0, 7.0, 2.0])
cloud = cloud @ mu.rot_axis_angle([1, 2, 3], 0.9).T + np.array([5.0, -3.0, 40.0])
c, evals, vt = mu.pca(cloud)
out["frame"] = {
    "pca": {"cloud": L(cloud), "mean": L(c), "eigenvalues": L(evals), "axes": L(vt)},
    "rot_axis_angle": {"axis": [1.0, 2.0, -0.5], "angle": 0.8, "matrix": L(mu.rot_axis_angle([1.0, 2.0, -0.5], 0.8))},
    "rotvec": {"matrix": L(mu.rot_axis_angle([1.0, 2.0, -0.5], 0.8)),
               "vec": L(mu.matrix_to_rotvec(mu.rot_axis_angle([1.0, 2.0, -0.5], 0.8))),
               "vec_pi": L(mu.matrix_to_rotvec(mu.rot_axis_angle([0, 0, 1], np.pi)))},
    "rigid_inverse": {"m": L(mu.make_frame([0, 1, 0], [0, 0, 1], [1, 0, 0], [1, 2, 3])),
                      "inv": L(mu.rigid_inverse(mu.make_frame([0, 1, 0], [0, 0, 1], [1, 0, 0], [1, 2, 3])))},
}
# arch seeds in arch coordinates, put in an arbitrary pose, with a gum mass below
pos = [(1, 3.0, 36.0), (2, 8.0, 34.0), (3, 12.0, 29.0), (4, 16.0, 23.0), (5, 19.0, 17.0), (6, 22.0, 9.0),
       (7, 23.0, 1.0)]
R = mu.rot_axis_angle([0.3, -1.0, 0.5], 2.2)
T0 = np.array([31.0, -12.0, 200.0])
cases = []
for arch, base in (("LOWER", 40), ("UPPER", 10)):
    seeds_local = []
    for i, x, y in pos:
        seeds_local.append((base + i, np.array([x, y, 8.0])))
        seeds_local.append(({10: 20, 40: 30}[base] + i, np.array([-x, y, 8.0])))
    seeds_local.sort(key=lambda t: t[0])
    seeds = [(f, R @ p + T0) for f, p in seeds_local]
    V = np.array([R @ (np.array([0.0, 20.0, -4.0]) + rng.normal(size=3) * 3) + T0 for _ in range(120)])
    obj = types.SimpleNamespace(oa=types.SimpleNamespace(arch=arch, frame=None, root=types.SimpleNamespace(matrix_world=None)),
                                seeds=[(f, p) for f, p in seeds])
    case_stub.get_seeds = lambda o: o.seeds
    ns = compute_frame.__globals__
    ns["get_seeds"] = lambda o: o.seeds
    ns["model_world_verts"] = lambda o, V=V: V
    M = compute_frame(obj)
    obj.M = M
    cases.append({
        "arch": arch, "seeds": [[f, L(p)] for f, p in seeds], "vertices_mean": L(V.mean(axis=0)),
        "frame": L(M), "centre": L(wf.arch_centre(obj, M)), "delta": L(wf.delta(obj)),
        "identity": bool(wf.is_identity(wf.delta(obj))),
    })
out["frame"]["cases"] = cases
eyes = [np.eye(4), np.block([[np.eye(3), np.array([[0.005], [0], [0]])], [np.zeros((1, 3)), np.ones((1, 1))]]),
        np.block([[np.eye(3), np.array([[0.02], [0], [0]])], [np.zeros((1, 3)), np.ones((1, 1))]])]
rot = np.eye(4)
rot[:3, :3] = mu.rot_axis_angle([1, 0, 0], np.radians(0.04))
rot2 = np.eye(4)
rot2[:3, :3] = mu.rot_axis_angle([1, 0, 0], np.radians(0.06))
out["frame"]["is_identity"] = [[L(m), bool(wf.is_identity(m))] for m in eyes + [rot, rot2]]

# ---- demo --------------------------------------------------------------------------------------------
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


def sample(V, T, step):
    idx = list(range(0, len(V), step))
    return {"n_vertices": len(V), "n_tris": len(T), "tri_sum": int(np.asarray(T).sum()),
            "tri_first": L(T[0]), "tri_last": L(T[-1]), "vertex_sum": L(np.asarray(V).sum(axis=0)),
            "min": L(np.asarray(V).min(axis=0)), "max": L(np.asarray(V).max(axis=0)),
            "step": step, "samples": L(np.asarray(V)[idx])}


demo_out = {"U": U, "Z": Zs}
teeth = {}
for f in (11, 13, 15, 16, 22, 36, 38, 44):
    V, T, top = demo.tooth_mesh(f)
    teeth[str(f)] = {**sample(V, T, 97), "top": L(top)}
demo_out["teeth"] = teeth
fd = [f for f in an.ARCH_ORDER["LOWER"] if an.index(f) != 8]
curve, W = demo.ideal_arch("LOWER", fd)
demo_out["ideal_lower"] = {"w": W, "points": L(curve.point(np.array([-30.0, -10.0, 0.0, 17.0, 40.0]))),
                           "n": len(curve.P), "s_last": float(curve.s[-1])}
fdu = [f for f in an.ARCH_ORDER["UPPER"]]
cu, Wu = demo.ideal_arch("UPPER", fdu)
demo_out["ideal_upper_all"] = {"w": Wu, "points": L(cu.point(np.array([-30.0, -10.0, 0.0, 17.0, 40.0])))}
Vg, Tg = demo.gum_mesh(curve, -33.0, 31.0)
demo_out["gum"] = sample(Vg, Tg, 61)

cap = {}
real_tooth_mesh = demo.tooth_mesh
orders = []


def spy(fdi, n_theta=48):
    orders.append(fdi)
    return real_tooth_mesh(fdi, n_theta)


def fake_mesh(bpy, name, V, T, validate=True):
    cap["V"], cap["T"] = V, T
    raise Stop


demo.tooth_mesh = spy
mu.mesh_from_arrays_fast = fake_mesh
real_default_rng = np.random.default_rng
np.random.default_rng = lambda seed=None: Replay()
runs = []
for arch, sev, third in (("LOWER", "MODERATE", False), ("UPPER", "SEVERE", True), ("LOWER", "MILD", False)):
    orders.clear()
    try:
        demo.generate(None, arch, sev, 7, third_molars=third)
    except Stop:
        pass
    V, T = cap["V"], cap["T"]
    runs.append({"arch": arch, "severity": sev, "third_molars": third, "order": list(orders), **sample(V, T, 211)})
np.random.default_rng = real_default_rng
demo_out["runs"] = runs
out["demo"] = demo_out

json.dump(out, sys.stdout, separators=(",", ":"))
