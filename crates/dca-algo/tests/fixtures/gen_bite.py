"""Generates bite.json: the plugin's bite registration (`bite.register`, `oa.register_bite`), its
manual pairing with 3 points (`bite.kabsch_init`, `oa.bite_points`) and the correction of the
interpenetration (`bite.correct_penetration`, `oa.bite_correct`), to compare the Rust port
(`dca_algo::bite::register`, `kabsch_init`, `correct_penetration`) against.

    blender -b --factory-startup -P gen_bite.py -- [--plugin path/to/blender-ortho-aligner] --out bite.json

Needs Blender (mathutils BVH and ray casts of the plugin's bite simulator; the CPU paths of
`gpu_query`, OA_GPU=0). The plugin is loaded read-only through the golden oracle package
(`golden/oracle/plugin.py`), with the Blender-bound `case` module replaced by a stand-in that
serves the case data and arch matrix of plain objects. Every other module (`bite`, `occlusion`,
`visit`, `gpu_query`, `meshutil`, `evidence`, `demo`) runs as it is: the search, the ICP, the
metrics, the checks and the report of `_register_fit` and `_register_apply`, the
interpenetration (`occlusion.penetration`), the first contact (`occlusion.contact_offset`) and
`correct_penetration`. Only the Blender side effects are stubbed: `occlusion.move_case`
multiplies the arch matrix of the stand-in, `visit.chain` is the case alone, `follow_stage` and
`refresh_summary` do nothing.

Arches: the plugin's synthetic arches (`demo.generate` up to the voxel remesh; lower MODERATE seed
7, upper MODERATE seed 11, as `gen_occlusion.py`) with tooth shells of N_THETA points per ring and
a gum of GUM_STEP mm, every tooth shell labelled with its FDI number and the gum 0, single
precision. The coarse arches of `occlusion.json` (10 points per ring, gum step 2.5 mm) are too
coarse for the plugin's own search: on them it lands upside down (about 178 degrees off) and
rejects the fit; with a gum step of 1 mm (the coarse search runs on the crowns and the gum within
2 mm of them) and 16 points per ring it finds the true bite within 0.1 degrees on both sides.
The upper arch is turned onto the lower one (`occlusion.S_FLIP`), its incisors 2 mm in front,
lowered with `occlusion._place` to the first contact on the height grid and then LOAD_MM further
(the bite under load of the plugin's scenario C): the "true" bite. The lower arch is placed in
the case with the rigid frame `lower_frame`, the upper one in the true bite, then displaced by
`upper_displacement`.

Bite scans: the plugin's own simulator (`tests/bitesim.py`, `crop_buccal`, unchanged: the faces of
both arches in the true bite next to the lower teeth 3 to 7 of one side, within 11 mm of the
occlusal plane, facing the cheek and visible from it), its points moved by a Gaussian noise of
0.02 mm (as `bitesim.resample`, whose voxel remesh is left out to keep the scans small), the left
one wound inwards, then put in the frame of the scanner (`scanner`). NOT A PATIENT SCAN. Single
precision.

Scenarios: `two_scans` (right and left, automatic), `one_scan` (right, automatic),
`three_points_lower` (right, 3 points on the lower arch and on the bite scan, upper arch
automatic), `three_points_both` (also 3 points on the upper arch), picked as the plugin's
scenario D (bite points on the crowns within 0.2 mm: the most distal, the most mesial and the
farthest from both; on the arch the true point with a picking error of 1 mm), and `correct` (the
correction after `two_scans`). Output: the arches, the bite scans, the moves of the upper arch and
of the bite scans in case coordinates, the metrics of every fit, the agreement, the
interpenetration, the contacts, the opening at the first contact, the facing and the reasons (as
the codes of the port), and the opening of the correction with the report after it.
"""
import argparse
import base64
import importlib.util
import json
import os
import re
import sys
import types

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
os.environ["OA_GPU"] = "0"                      # the exact CPU paths of gpu_query
HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.normpath(os.path.join(HERE, "../../../../golden"))
sys.path.insert(0, GOLDEN)

import bpy  # noqa: E402,F401  (Blender: the real mathutils)
import numpy as np  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--plugin", default=None)
ap.add_argument("--out", required=True)
ARGS = ap.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
if ARGS.plugin:
    os.environ["OA_PLUGIN"] = ARGS.plugin

from oracle import plugin  # noqa: E402

plugin.install()


class CaseError(Exception):
    pass


case = types.ModuleType(f"{plugin.PKG}.case")
case.CaseError = CaseError
case._data = lambda obj: obj.d
case.arch_matrix = lambda obj: obj.M
case.np_matrix = lambda m: np.asarray(m, np.float64)
case.bl_matrix = lambda m: np.asarray(m, np.float64)
case.model_world_verts = lambda obj: obj.d["V"] @ obj.M[:3, :3].T + obj.M[:3, 3]
sys.modules[case.__name__] = case

an = plugin.load("anatomy")
mu = plugin.load("meshutil")
demo = plugin.load("demo")
visit = plugin.load("visit")
occlusion = plugin.load("occlusion")
bite = plugin.load("bite")

occlusion.move_case = lambda obj, D: setattr(obj, "M", D @ obj.M)
occlusion.follow_stage = lambda obj: None
occlusion.refresh_summary = lambda lower, upper: None
visit.chain = lambda obj: [obj]


class Tris:
    """`obj.data` of a stand-in: its triangles (`get_tris` of the simulator)."""

    def __init__(self, T):
        self.T = T


class Obj(dict):
    """Stand-in for a case or a bite scan object."""

    def __init__(self, name, d=None, M=None):
        super().__init__()
        self.name = name
        self.d = d
        self.M = M
        self.data = Tris(d["T"]) if d is not None else None
        self.matrix_world = np.eye(4)
        self.oa = types.SimpleNamespace(bite_json="", plan_json="", is_case=d is not None)


# the plugin's bite simulator (tests/bitesim.py) as it is, its `ortho_aligner` imports served by the
# modules above (`get_tris` of a stand-in: its triangles)
sim_mu = types.ModuleType("bitesim_meshutil")
sim_mu.__dict__.update(mu.__dict__)
sim_mu.get_tris = lambda data: data.T if isinstance(data, Tris) else mu.get_tris(data)
pkg = types.ModuleType("ortho_aligner")
pkg.__path__ = []
pkg.anatomy, pkg.case, pkg.meshutil = an, case, sim_mu
sys.modules.update({"ortho_aligner": pkg, "ortho_aligner.anatomy": an, "ortho_aligner.case": case,
                    "ortho_aligner.meshutil": sim_mu})
_spec = importlib.util.spec_from_file_location(
    "bitesim", os.path.join(os.path.dirname(plugin.plugin_dir()), "tests", "bitesim.py"))
bitesim = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(bitesim)

N_THETA = 16        # points per ring of a tooth shell (demo default 48)
GUM_STEP = 1.0      # mm between the rings of the gum (demo default 0.5)
LOAD_MM = 0.3       # the true bite: under load, this far past the first contact (interpenetration clear of
                    # the plugin's warning at 0.10 mm)
FIRST, LAST = 3, 6  # lower teeth of a bite scan: canine to first molar (to the second molar a scan of these
                    # arches is 45.2 mm long, a scan round the whole arch for the plugin: FULL_ARCH_MM)


class Stop(Exception):
    pass


def labelled_arch(arch, severity, seed):
    """V, T, labels (FDI per tooth shell, 0 for the gum) of the plugin's demo arch, moved down along
    its occlusal axis so that the occlusal plane (the mean height of the tops of the crowns) is at
    z = 0, as in the arch frame of a case (the demo puts z = 0 on the gum line; the bite simulator
    crops around z = 0)."""
    real_tooth, real_gum, real_mesh = demo.tooth_mesh, demo.gum_mesh, mu.mesh_from_arrays_fast
    order, cap = [], {}

    def tooth(fdi, n_theta=48):
        order.append(fdi)
        return real_tooth(fdi, N_THETA)

    def gum(curve, s0, s1, **kw):
        return real_gum(curve, s0, s1, step=GUM_STEP)

    def fake_mesh(bpy, name, V, T, validate=True):
        cap["V"], cap["T"] = V, T
        raise Stop

    demo.tooth_mesh, demo.gum_mesh, mu.mesh_from_arrays_fast = tooth, gum, fake_mesh
    try:
        demo.generate(None, arch, severity, seed, third_molars=False)
    except Stop:
        pass
    finally:
        demo.tooth_mesh, demo.gum_mesh, mu.mesh_from_arrays_fast = real_tooth, real_gum, real_mesh
    V, T = cap["V"], cap["T"]
    per_tooth = N_THETA * 23 + 2
    labels = np.zeros(len(V), np.int64)
    for k, f in enumerate(order):
        labels[k * per_tooth:(k + 1) * per_tooth] = f
    top = np.mean([V[labels == f, 2].max() for f in order])
    V = V - np.array([0.0, 0.0, top])
    return V.astype(np.float32).astype(np.float64), np.asarray(T, np.int64), labels


def case_data(V, T, L):
    """What `case._data` gives for an arch: arch-space vertices, triangles, labels, crown vertices per
    tooth (in tooth order) and the edges."""
    crown_idx = {int(f): np.nonzero(L == f)[0] for f in sorted(set(L.tolist())) if f}
    return {"V": V, "T": T, "lab": L, "crown_idx": crown_idx, "E": mu.unique_edges(T)}


def rigid(axis, deg, t):
    a = np.asarray(axis, np.float64)
    a = a / np.linalg.norm(a)
    th = np.radians(deg)
    K = np.array([[0, -a[2], a[1]], [a[2], 0, -a[0]], [-a[1], a[0], 0]])
    M = np.eye(4)
    M[:3, :3] = np.eye(3) + np.sin(th) * K + (1 - np.cos(th)) * K @ K
    M[:3, 3] = t
    return M


def true_relation(Vl, Ll, Vu, Lu):
    """Upper -> lower arch coordinates: flipped onto the lower arch, incisors 2 mm in front, first
    contact on the height grid (`occlusion._place`), then LOAD_MM into the lower arch."""
    G = np.eye(4)
    G[:3, :3] = occlusion.S_FLIP
    inc_l, inc_u = np.isin(Ll, [31, 41]), np.isin(Lu, [11, 21])
    G[1, 3] = (Vl[inc_l, 1].max() + 2.0) - (Vu[inc_u] @ occlusion.S_FLIP.T)[:, 1].max()
    top = occlusion._TopSurface(Vl[Ll != 0])
    G[2, 3] += occlusion._place(Vu[Lu != 0], top, G, 0.0)[0] - LOAD_MM
    return G


Vl, Tl, Ll = labelled_arch("LOWER", "MODERATE", 7)
Vu, Tu, Lu = labelled_arch("UPPER", "MODERATE", 11)
G = true_relation(Vl, Ll, Vu, Lu)
ML = rigid((0.2, -0.3, 1.0), 25.0, (12.0, -7.0, 30.0))         # lower arch -> case
Y = rigid((1.0, 0.4, -0.2), 9.0, (-2.5, 3.0, 4.5))              # displacement of the upper arch (case)
MU = Y @ ML @ G                                                 # upper arch -> case, displaced
X = rigid((-0.4, 1.0, 0.3), 70.0, (40.0, -25.0, 15.0))          # case -> scanner frame of the bite scans
DL, DU = case_data(Vl, Tl, Ll), case_data(Vu, Tu, Lu)


def bite_scan(right, seed, flip):
    """`bitesim.crop_buccal` of one side on the arches in the true bite (case coordinates), with
    noise: (vertices in the case in the true bite, vertices in the scanner frame, triangles)."""
    V, T = bitesim.crop_buccal(Obj("lower", DL, ML.copy()), Obj("upper", DU, ML @ G), right, FIRST, LAST)
    V = V + np.random.default_rng(seed).normal(0.0, 0.02, V.shape)
    if flip:
        T = T[:, ::-1]
    P = (V @ X[:3, :3].T + X[:3, 3]).astype(np.float32).astype(np.float64)
    return V, P, np.asarray(T, np.int64)


SCANS = {"bite-right.stl": bite_scan(True, 21, False), "bite-left.stl": bite_scan(False, 22, True)}


def bite_of(name):
    """`bite.Bite(obj)` of a bite scan: its points, area-weighted normals and areas."""
    _, V, T = SCANS[name]
    A = bite._vertex_areas(V, T)
    use = A > 0
    return bite.Bite(Obj(name), V[use], mu.vertex_normals(V, T)[use], A[use], name=name)


def cases():
    return Obj("lower", DL, ML.copy()), Obj("upper", DU, MU.copy())


def run_fit(gen):
    while True:
        try:
            next(gen)
        except StopIteration as e:
            return e.value


SIDE = {"destro": "right", "sinistro": "left", "destro e sinistro": "both", None: None}
ARCH = {"inferiore": "lower", "superiore": "upper"}


def codes(why):
    out = []
    for w in why:
        m = re.match(r"(.+): scarto RMS sull'arcata (\w+) ", w)
        if m:
            out.append(f"rms_above_good:{m.group(1)}:{ARCH[m.group(2)]}")
            continue
        m = re.match(r"(.+): solo \d+ denti dell'arcata (\w+) ", w)
        if m:
            out.append(f"few_teeth:{m.group(1)}:{ARCH[m.group(2)]}")
            continue
        for start, code in (("Morso da una scansione che gira", "full_arch_scan"),
                            ("Morso di un solo lato", "one_side_only"),
                            ("I due morsi non concordano", "scans_disagree"),
                            ("Compenetrazione tra le arcate", "interpenetration"),
                            ("Le arcate registrate non si toccano", "arches_not_in_contact")):
            if w.startswith(start):
                out.append(code)
                break
        else:
            raise ValueError(f"unknown reason: {w}")
    return out


def metrics(m):
    return {"area_mm2": m["area_mm2"], "teeth": {str(k): v for k, v in m["denti"].items()},
            "n_teeth": m["n_denti"], "rms_mm": m["rms_mm"], "p95_mm": m["p95_mm"],
            "coherence": m["coerenza"], "bite_fraction": m["frazione_morso"],
            "icp_overlap": m["sovrapposizione_icp"]}


def mat(M):
    return np.round(M, 15).ravel().tolist()


def report(rep):
    return {"scans": [{"scan": s["scansione"], "side": SIDE[s["lato"]], "lower": metrics(s["inferiore"]),
                       "upper": metrics(s["superiore"])} for s in rep["scansioni"]],
            "agreement_mm": rep["concordanza_mm"], "penetration_mm": rep["compenetrazione_mm"],
            "registered_penetration_mm": rep.get("compenetrazione_registrata_mm"),
            "contact_points": rep["punti_di_contatto"], "opening_at_contact_mm": rep["apertura_al_contatto_mm"],
            "facing": rep["fronteggiamento"], "correction_mm": rep["correzione_mm"],
            "verdict": "good" if rep["verdetto"] == "BUONA" else "to_review", "reasons": codes(rep["motivi"])}


def rot_deg(A, B):
    return float(np.degrees(np.linalg.norm(mu.matrix_to_rotvec(A[:3, :3] @ B[:3, :3].T))))


def register(names, init_lower=None, init_upper=None):
    """`bite.register_steps` without Blender objects: the fit (`_register_fit`), then the move and the
    checks (`_register_apply`) on the stand-ins. Returns (record, lower, upper)."""
    lower, upper = cases()
    bites = [bite_of(n) for n in names]
    al, au = bite.Arch(lower, DL, lower.M), bite.Arch(upper, DU, upper.M)
    low, up, agree, Tu = run_fit(bite._register_fit(al, au, bites, init_lower, init_upper))
    Ml, Mu = al.M, au.M
    rep = bite._register_apply(lower, upper, bites, low, up, agree, Tu, Ml, Mu, occlusion)
    D = mu.rigid_inverse(Tu) @ mu.rigid_inverse(Mu)
    rec = {"scans": list(names), "upper_delta": mat(D), "bite_deltas": [mat(Ml @ T) for T, _, _ in low],
           "upper_after": mat(upper.M), "report": report(rep)}
    # how far from the truth the plugin lands (the upper arch in the true bite, the scans on the arches)
    pts = mu.transform_points(ML @ G, Vu[Lu != 0])
    err = mu.transform_points(upper.M @ mu.rigid_inverse(ML @ G), pts) - pts
    rec["plugin_error"] = {"upper_mm": float(np.linalg.norm(err, axis=1).max()),
                           "upper_deg": rot_deg(upper.M, ML @ G),
                           "bite_deg": [rot_deg(Ml @ T, mu.rigid_inverse(X)) for T, _, _ in low]}
    print(f"{names}: plugin {rec['plugin_error']}, verdict {rep['verdetto']}, {rep['motivi']}", file=sys.stderr)
    return rec, lower, upper


def picks(name, arch, M_true):
    """The plugin's scenario D: 3 bite points on the crowns of `arch` (within 0.2 mm, the bite scan in
    the true bite): the most distal, the most mesial and the farthest from both. Returns the points
    in the case in the true bite and in the frame of the scanner."""
    V, P, _ = SCANS[name]
    Q = mu.transform_points(mu.rigid_inverse(M_true), V)
    _, F, _ = arch.crowns.nearest(Q, 0.2)
    idx = np.nonzero(F >= 0)[0]
    W = V[idx]
    a = idx[np.argmin(W @ ML[:3, 1])]
    b = idx[np.argmax(W @ ML[:3, 1])]
    u = mu.normalize(V[b] - V[a])
    off = (W - V[a]) - np.outer((W - V[a]) @ u, u)
    c = idx[np.argmax(np.linalg.norm(off, axis=1))]
    k = [a, b, c]
    return V[k], P[k]


def main():
    out = {"n_theta": N_THETA, "gum_step_mm": GUM_STEP, "load_mm": LOAD_MM,
           "lower": {"n_vertices": len(Vl), "n_tris": len(Tl), "vertices": b64(Vl, "<f4"), "tris": b64(Tl, "<u2"),
                     "labels": b64(Ll, "u1")},
           "upper": {"n_vertices": len(Vu), "n_tris": len(Tu), "vertices": b64(Vu, "<f4"), "tris": b64(Tu, "<u2"),
                     "labels": b64(Lu, "u1")},
           "lower_frame": mat(ML), "upper_frame": mat(MU), "true_relation": mat(G),
           "upper_displacement": mat(Y), "scanner": mat(X), "bite_scans": {}, "scenarios": {}}
    for name, (_, P, T) in SCANS.items():
        out["bite_scans"][name] = {"n_vertices": len(P), "n_tris": len(T), "vertices": b64(P, "<f4"),
                                   "tris": b64(T, "<u4")}
        b = bite_of(name)
        print(f"{name}: {len(b.P)} points, {b.area:.0f} mm2, round the whole arch: {bite.is_full_arch(b)}",
              file=sys.stderr)

    rec, lower, upper = register(["bite-right.stl", "bite-left.stl"])
    out["scenarios"]["two_scans"] = rec
    dz = bite.correct_penetration(lower, upper)
    out["scenarios"]["correct"] = {"after": "two_scans", "opening_mm": dz, "upper_after": mat(upper.M),
                                   "report": report(bite.info(lower))}
    print(f"correct: opening {dz}", file=sys.stderr)
    out["scenarios"]["one_scan"] = register(["bite-right.stl"])[0]

    # 3 points (the plugin's scenario D): picked on the arches where they are (the true point moved with
    # its arch, 1 mm picking error) and on the bite scan
    lower, upper = cases()
    err = np.random.default_rng(31).normal(0.0, 1.0 / np.sqrt(3.0), (6, 3))
    bl, pl = picks("bite-right.stl", bite.Arch(lower, DL, ML), ML)
    bu, pu = picks("bite-right.stl", bite.Arch(upper, DU, ML @ G), ML @ G)
    lo_a, up_a = bl + err[:3], mu.transform_points(Y, bu) + err[3:]
    init_l = bite.kabsch_init(lower, lo_a, pl)
    rec = register(["bite-right.stl"], init_lower=init_l)[0]
    rec["picks"] = {"lower_points": lo_a.tolist(), "bite_points": pl.tolist(), "init_lower": mat(init_l)}
    out["scenarios"]["three_points_lower"] = rec

    init_u = bite.kabsch_init(upper, up_a, pu)
    rec = register(["bite-right.stl"], init_lower=init_l, init_upper=init_u)[0]
    rec["picks"] = {"lower_points": lo_a.tolist(), "bite_points": pl.tolist(), "init_lower": mat(init_l),
                    "upper_points": up_a.tolist(), "bite_points_upper": pu.tolist(), "init_upper": mat(init_u)}
    out["scenarios"]["three_points_both"] = rec

    with open(ARGS.out, "w") as f:
        json.dump(out, f, separators=(",", ":"))


def b64(a, dtype):
    return base64.b64encode(np.ascontiguousarray(a, dtype=dtype).tobytes()).decode()


main()
