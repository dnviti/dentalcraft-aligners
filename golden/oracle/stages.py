"""Per-stage oracle functions.

Every stage is one function `stage_<name>(ctx, out)` that runs the plugin code of
that pipeline stage on the inputs of one case and writes its inputs/outputs under
`out` (JSON, `.dcm`).  Porting agents extend a stage by editing its function (or add one
to `STAGES`).  `ctx` is a `Case` (below): the loaded scan, the decimated scan, the
ground truth of the demo, and the memoised state of earlier stages.

Stages that need Blender are not here (see ../blender_dump.py).  Where a plugin
function lives in a bpy-bound module it is pulled out by `plugin.extract`, or its
numerical core is mirrored and said so.
"""
import datetime
import json
import os
import sys
import time
import types
from functools import cached_property
from dataclasses import asdict, is_dataclass
from types import SimpleNamespace

import numpy as np

from . import frame as fr
from . import mesh, plugin, settings

ORDER = ["archform", "worldframe", "autoseed", "segmentation", "teeth", "diagnosis", "setup", "staging",
         "schedule"]
START_DATE = "2026-01-05"       # fixed: schedule.build would use today's date otherwise


def J(x):
    """JSON-able copy: numpy -> lists, non-finite floats -> null, dataclasses -> dicts, dates -> ISO."""
    if isinstance(x, dict):
        return {str(k): J(v) for k, v in x.items()}
    if isinstance(x, (list, tuple, set)):
        return [J(v) for v in x]
    if isinstance(x, np.ndarray):
        return J(x.tolist())
    if isinstance(x, (np.bool_, bool)):
        return bool(x)
    if isinstance(x, (np.integer, int)):
        return int(x)
    if isinstance(x, (np.floating, float)):
        return float(x) if np.isfinite(x) else None
    if isinstance(x, (datetime.date, datetime.datetime)):
        return x.isoformat()
    if is_dataclass(x) and not isinstance(x, type):
        return J(asdict(x))
    return x


def lst(a, nd=None):
    a = np.asarray(a, np.float64)
    return J(np.round(a, nd) if nd is not None else a)


def dump(out, name, obj):
    os.makedirs(out, exist_ok=True)
    mesh.dump_json(os.path.join(out, name), J(obj))


def agreement(a, b):
    a, b = np.asarray(a), np.asarray(b)
    return float(np.mean(a == b)) if len(a) else 1.0


class Case:
    """One golden case: <fixtures>/<case>/{scan.dcm, decimated.dcm, meta.json} from Tier B.

    `src` is the case directory, its `meta.json`, or any mesh file (.dcm/.ply/.stl); a mesh without
    a `meta.json` next to it needs `args.arch` and has no ground truth: its seeds come from
    `args.seeds` (JSON {fdi: [x,y,z]} in world coordinates) or from the autoseed stage, and its
    decimated scan from `args.decimated` or a `decimated.dcm` next to it (made by Blender,
    Tier B)."""

    def __init__(self, src, args=None):
        self.args = args or SimpleNamespace()
        src = os.path.abspath(src)
        if os.path.isdir(src):
            self.dir, scan_path = src, os.path.join(src, "scan.dcm")
        elif src.endswith("meta.json"):
            self.dir, scan_path = os.path.dirname(src), os.path.join(os.path.dirname(src), "scan.dcm")
        else:
            self.dir, scan_path = os.path.dirname(src), src
        meta_path = os.path.join(self.dir, "meta.json")
        self.meta = json.load(open(meta_path)) if os.path.exists(meta_path) else None
        self.mu = plugin.load("meshutil")
        self.an = plugin.load("anatomy")
        sc = mesh.read_mesh(scan_path)
        self.V, self.T = np.asarray(sc["V"], np.float64), np.asarray(sc["T"], np.int64)
        gt = sc.get("vertex_labels")
        self.gt = None if gt is None else gt.astype(np.int64)
        self.arch = getattr(self.args, "arch", None) or (self.meta or {}).get("arch")
        if self.arch not in ("UPPER", "LOWER"):
            raise SystemExit("--arch UPPER|LOWER is required (no meta.json next to the mesh)")
        self.name = (self.meta or {}).get("case") or os.path.splitext(os.path.basename(scan_path))[0]
        self.s = settings.defaults(start_date=START_DATE)
        self._state = {}
        self._dec = None
        self._seeds = None

    # -- inputs that may come from the Tier B metadata or be derived -----------------------
    @property
    def seeds_world(self):
        if self._seeds is None:
            path = getattr(self.args, "seeds", None)
            if path:
                raw = json.load(open(path))
                self._seeds = {int(f): np.array(p, np.float64) for f, p in raw.items()}
            elif self.meta:
                self._seeds = {int(f): np.array(p) for f, p in self.meta["seeds_gt_world"].items()}
            else:
                self._seeds = self.state("autoseed")["seeds"]
        return self._seeds

    @cached_property
    def M(self):
        if self.meta and not getattr(self.args, "seeds", None):
            return np.array(self.meta["frame_gt"], np.float64)
        return fr.compute_frame(self.seeds_world, self.V)

    @cached_property
    def Minv(self):
        return self.mu.rigid_inverse(self.M)

    @cached_property
    def V_arch(self):
        return self.mu.transform_points(self.Minv, self.V)

    @cached_property
    def seeds_arch(self):
        Minv = self.Minv
        return [(f, self.mu.transform_points(Minv, p[None])[0]) for f, p in sorted(self.seeds_world.items())]

    def _decimated(self):
        if self._dec is None:
            path = getattr(self.args, "decimated", None) or os.path.join(self.dir, "decimated.dcm")
            if not os.path.exists(path):
                raise SystemExit(f"{path} not found: the decimated scan (Blender's Decimate, arch coordinates) "
                                 "comes from Tier B, blender_dump.py")
            d = mesh.read_mesh(path)
            self._dec = (np.asarray(d["V"], np.float64), np.asarray(d["T"], np.int64))
        return self._dec

    @property
    def Vd(self):
        return self._decimated()[0]

    @property
    def Td(self):
        return self._decimated()[1]

    def run(self, stage, out=None):
        """Run a stage (writing to `out` if given) and memoise its Python state."""
        self._state[stage] = STAGES[stage](self, out)
        return self._state[stage]

    def state(self, stage):
        """Python state of an earlier stage, computed (not written) when not memoised.  With `--reuse`
        the segmentation (minutes) is read back from `<case dir>/segmentation/segmentation.json`."""
        if stage not in self._state and stage == "segmentation" and getattr(self.args, "reuse", False):
            path = os.path.join(self.dir, "segmentation", "segmentation.json")
            if os.path.exists(path):
                self._state[stage] = {"labels": mesh.unpack_labels(json.load(open(path))["vertex_labels"])}
        if stage not in self._state:
            self.run(stage)
        return self._state[stage]


# ---------------------------------------------------------------------------
# archform
# ---------------------------------------------------------------------------

def stage_archform(ctx, out):
    """Arch curve models (archform.py, demo.ideal_arch): the Braun beta curve fitted to the teeth
    present, its arc-length parametrisation, and the quartic / even-polynomial fits to the
    ground-truth seeds in arch coordinates."""
    af, demo, an, mu = plugin.load("archform"), plugin.load("demo"), ctx.an, ctx.mu
    fdis = [f for f in an.ARCH_ORDER[ctx.arch] if an.index(f) != 8]
    curve, W = demo.ideal_arch(ctx.arch, fdis)
    s = np.linspace(curve.s[0], curve.s[-1], 241)
    P = curve.point(s)
    seeds_xy = np.array([p[:2] for _, p in ctx.seeds_arch])
    abc = af.fit_quartic(seeds_xy)
    ids = [f for f, _ in ctx.seeds_arch]
    xs = np.linspace(-30, 30, 25)
    pts4 = [(0.0, float(abc[0]))] + [(x, float(af.quartic_y(x, abc))) for x in (10.0, 20.0, 28.0)]
    even = af.poly_even_through(pts4)
    state = {"curve": curve, "W": W, "quartic": abc}
    if out:
        qc = af.curve_from_model("QUARTIC", abc, 30.0)
        sq = np.linspace(qc.s[0], qc.s[-1], 121)
        dump(out, "archform.json", {
            "case": ctx.name, "arch": ctx.arch, "teeth_present": fdis,
            "inputs": {"seeds_arch_xy": {str(f): lst(p[:2]) for f, p in ctx.seeds_arch}},
            "ideal_beta": {"W": W, "s": lst(s), "xy": lst(P),
                           "tangent": lst(curve.tangent(s)), "normal_out": lst(curve.normal_out(s)),
                           "beta_y_at_x": {"x": lst(xs), "y": lst(af.beta_y(xs, W, 0.5 * W))}},
            "seed_projection_s": {str(f): float(curve.project(p[None, :2])[0]) for f, p in ctx.seeds_arch},
            "chord_s": {"s0": 0.0, "chord_mm": [7.0, 9.0], "right": [float(curve.s_at_chord(0.0, c, 1.0)) for c in (7.0, 9.0)],
                        "left": [float(curve.s_at_chord(0.0, c, -1.0)) for c in (7.0, 9.0)]},
            "quartic_fit_seeds": {"abc": lst(abc), "residual_mm": lst(af.quartic_y(seeds_xy[:, 0], abc) - seeds_xy[:, 1]),
                                  "ids": ids},
            "quartic_curve": {"half_extent": 30.0, "s": lst(sq), "xy": lst(qc.point(sq))},
            "poly_even_through": {"points": pts4, "coef": lst(even)},
            "note": "ideal_beta is demo.ideal_arch (the demo's own arch model), 241 samples of s over its full extent",
        })
    return state


# ---------------------------------------------------------------------------
# worldframe
# ---------------------------------------------------------------------------

def stage_worldframe(ctx, out):
    """Arch frame from the seeds (case_core.compute_frame, mirrored in oracle.frame) and the
    world-axes transform (worldframe.delta, mirrored)."""
    seeds = ctx.seeds_world
    M = fr.compute_frame(seeds, ctx.V)
    D = fr.world_delta(M, seeds, ctx.arch)
    state = {"M": M, "D": D}
    if out:
        dump(out, "worldframe.json", {
            "case": ctx.name, "arch": ctx.arch,
            "inputs": {"seeds_world": {str(f): lst(p) for f, p in sorted(seeds.items())},
                       "mesh": "../scan.dcm (world), only its mean is used"},
            "arch_frame": {"M": lst(M), "M_inv": lst(ctx.mu.rigid_inverse(M)),
                           "origin": lst(M[:3, 3]), "x": lst(M[:3, 0]), "y": lst(M[:3, 1]), "z": lst(M[:3, 2])},
            "seeds_arch": {str(f): lst(p) for f, p in ctx.seeds_arch},
            "arch_centre_world": lst(fr.arch_centre(M, seeds)),
            "world_delta": lst(D),
            "note": "arch_frame.M equals meta.json frame_gt (it is the frame the decimated scan was cut in)",
        })
    return state


# ---------------------------------------------------------------------------
# autoseed
# ---------------------------------------------------------------------------

def stage_autoseed(ctx, out):
    """Automatic seed proposal (autoseed.arch_data + propose) on the full scan in world coordinates,
    compared with the demo's ground-truth seeds."""
    aseed = plugin.load("autoseed")
    t0 = time.time()
    data = aseed.arch_data(ctx.V, ctx.T)
    M, curve, pr = data
    seeds, M2, diag = aseed.propose(ctx.V, ctx.T, ctx.arch, False, (), data=data)
    state = {"seeds": seeds, "M": M2, "data": data, "diag": diag}
    if out:
        truth = ctx.meta["seeds_gt_world"] if ctx.meta else {}
        err = {f: float(np.linalg.norm(seeds[f] - np.array(truth[str(f)]))) for f in sorted(seeds) if str(f) in truth}
        prof = {k: lst(v, 6) for k, v in pr.items() if isinstance(v, np.ndarray) and v.ndim == 1 and len(v) < 2000}
        dump(out, "autoseed.json", {
            "case": ctx.name, "arch": ctx.arch,
            "inputs": {"mesh": "../scan.dcm (world coordinates)", "include_third_molars": False, "missing": [],
                       "band_depth": 6.0, "bin_mm": 0.25},
            "arch_frame": {"M": lst(M2)},
            "arch_polyline": lst(curve.P if hasattr(curve, "P") else curve),
            "profiles": {"nb": int(pr["nb"]), "s_min": float(pr["s_min"]), "s_max": float(pr["s_max"]), **prof},
            "seeds_world": {str(f): lst(p) for f, p in sorted(seeds.items())},
            "diagnostics": diag,
            "vs_ground_truth": {"seed_error_mm": err, "max_mm": max(err.values()) if err else None,
                                "teeth_found": sorted(seeds), "teeth_truth": sorted(int(f) for f in truth)},
            "seconds": round(time.time() - t0, 1),
        })
    return state


# ---------------------------------------------------------------------------
# segmentation
# ---------------------------------------------------------------------------

def _full_res_fns():
    """transfer_labels / _labels_full of case_teeth (bpy-bound module: only those defs)."""
    ns = {"np": np, "mu": plugin.load("meshutil"), "seg": plugin.load("segmentation")}
    plugin.extract("case_teeth", ["TIE_MM", "transfer_labels", "_labels_full"], ns)
    return ns


def stage_segmentation(ctx, out):
    """Segmentation of the arch from the ground-truth seeds, as `case_teeth.segment_iter`: random walk
    on the decimated scan, label transfer to the full scan, boundary refinement, cleaning.  Written
    as: segmentation.json (inputs, parameters, diagnostics, agreement with the ground truth),
    face labels of the decimated scan and vertex labels of the full scan (run-length encoded, see README)."""
    seg, mu, s = plugin.load("segmentation"), ctx.mu, ctx.s
    fns = _full_res_fns()
    t0 = time.time()
    seeds = ctx.seeds_arch                                   # [(fdi, arch point)] sorted by FDI
    flab, diag = seg.segment_faces(ctx.Vd, ctx.Td, seeds, ctx.arch, s.rw_eta_convex, s.rw_sigma)
    t1 = time.time()
    E, lab, seed_v = fns["_labels_full"](ctx.V_arch, ctx.T, ctx.Vd, ctx.Td, flab, seeds)
    lab_transfer = lab.copy()
    t2 = time.time()
    lab = seg.refine_boundaries(ctx.V_arch, ctx.T, lab)
    t3 = time.time()
    lab = seg.clean_vertex_labels(lab, E, seed_v)
    diag["teeth"] = int(len(np.unique(lab)) - (1 if 0 in lab else 0))
    state = {"labels": lab, "face_labels_decimated": flab, "E": E, "diag": diag}
    if out:
        gt = ctx.gt
        per = {}
        if gt is not None:
            for f in sorted(set(np.unique(gt)) | set(np.unique(lab))):
                a, b = gt == f, lab == f
                u = int((a | b).sum())
                per[str(int(f))] = {"truth": int(a.sum()), "got": int(b.sum()),
                                    "iou": float((a & b).sum() / u) if u else 1.0}
        dump(out, "segmentation.json", {
            "case": ctx.name, "arch": ctx.arch,
            "inputs": {"mesh_decimated": "../decimated.dcm (arch coordinates)", "mesh_full": "../scan.dcm (world), "
                       "moved to arch coordinates with worldframe.arch_frame.M_inv",
                       "seeds_arch": {str(f): lst(p) for f, p in seeds},
                       "params": {"eta_convex": s.rw_eta_convex, "sigma": s.rw_sigma, "tol": 1e-4, "maxiter": 1500,
                                  "refine": "defaults of segmentation.refine_boundaries"}},
            "diagnostics": diag,
            "face_labels_decimated": mesh.pack_labels(flab),
            "vertex_labels_transfer": mesh.pack_labels(lab_transfer),
            "vertex_labels": mesh.pack_labels(lab),
            "label_agreement_vs_ground_truth": None if gt is None else agreement(gt, lab),
            "label_agreement_transfer_vs_ground_truth": None if gt is None else agreement(gt, lab_transfer),
            "per_tooth_vs_ground_truth": per,
            "seconds": {"segment_faces": round(t1 - t0, 1), "transfer": round(t2 - t1, 1),
                        "refine": round(t3 - t2, 1), "total": round(time.time() - t0, 1)},
            "note": "vertex_labels: one FDI (0 = gingiva) per vertex of scan.dcm, in its order; rle = [label, count, ...]",
        })
    return state


# ---------------------------------------------------------------------------
# teeth
# ---------------------------------------------------------------------------

def _labels(ctx):
    return ctx.state("segmentation")["labels"]


def stage_teeth(ctx, out):
    """Tooth frames, widths, contact points, centres of resistance (teeth.compute_teeth) from the
    segmentation labels of the oracle."""
    tg = plugin.load("teeth")
    lab = _labels(ctx)
    teeth, seq = tg.compute_teeth(ctx.V_arch, ctx.T, lab, ctx.arch)
    state = {"teeth": teeth, "seq": seq, "labels": lab}
    if out:
        rows = {}
        for f in seq:
            t = teeth[f]
            row = {k: v for k, v in asdict(t).items() if k not in ("vidx", "fidx")}
            row["n_vertices"], row["n_faces"] = int(len(t.vidx)), int(len(t.fidx))
            row["rest_frame"] = t.frame()
            rows[str(f)] = row
        dump(out, "teeth.json", {
            "case": ctx.name, "arch": ctx.arch, "sequence": seq,
            "inputs": {"mesh": "../scan.dcm moved to arch coordinates", "labels": "../segmentation/segmentation.json "
                       "vertex_labels"},
            "teeth": rows})
    return state


# ---------------------------------------------------------------------------
# diagnosis
# ---------------------------------------------------------------------------

def _plan_ns(ctx):
    """Namespace for the functions extracted from case_plan.py (a bpy-bound module)."""
    return {"np": np, "json": json, "time": time, "an": ctx.an, "mu": ctx.mu, "pl": plugin.load("planning"),
            "st": plugin.load("staging")}


def _fake_obj(ctx):
    return SimpleNamespace(name="golden", oa=SimpleNamespace(arch=ctx.arch, analysis_json="", setup_json="",
                                                             plan_json="", aligner_offset=0, visit_date="",
                                                             classes=[], parent=None))


def _data_dict(ctx):
    t = ctx.state("teeth")
    return {"teeth": t["teeth"], "seq": t["seq"], "V": ctx.V_arch, "T": ctx.T, "lab": t["labels"]}


def stage_diagnosis(ctx, out):
    """Space analysis, Little index, largest space, CAT-CAT items and level: `case_plan.analyze`
    itself (extracted), on the oracle's teeth."""
    d = _data_dict(ctx)
    ns = _plan_ns(ctx)
    ns["_data"] = lambda obj, s=None: d
    plugin.extract("case_plan", ["analyze"], ns)
    obj = _fake_obj(ctx)
    res = ns["analyze"](obj)
    state = {"analysis": res, "obj": obj}
    if out:
        dump(out, "diagnosis.json", {"case": ctx.name, "arch": ctx.arch,
                                     "inputs": {"teeth": "../teeth/teeth.json"}, "analysis": res})
    return state


# ---------------------------------------------------------------------------
# setup
# ---------------------------------------------------------------------------

def stage_setup(ctx, out):
    """Conservative set-up (setup_conservativo.conservative_setup) with the default scene
    settings: nothing locked, no tooth out of the aligner, no drawn arch line, no opposing arch.
    The numerical core of case_plan.run_setup (mirrored; the helper functions are extracted)."""
    sc = plugin.load("setup_conservativo")
    d = _data_dict(ctx)
    ns = _plan_ns(ctx)
    plugin.extract("case_plan", ["PRIORITY", "conservative_config", "rest_cusps", "rest_crowns", "_resample"], ns)
    obj = _fake_obj(ctx)
    cfg = ns["conservative_config"](ctx.s, obj)
    cusps, crowns = ns["rest_cusps"](d), ns["rest_crowns"](d)
    t0 = time.time()
    r = sc.conservative_setup(d["teeth"], d["seq"], cusps, cfg, line=None, fixed=[], crowns=crowns, poses={})
    pl = plugin.load("planning")
    residual = {}
    if r["residual"] > 1e-6:
        residual["affollamento"] = round(float(r["residual"]), 2)
    if r["spacing"] > sc.SPACE_TOL and ctx.s.close_spaces:
        residual["spazio"] = round(float(r["spacing"]), 2)
    ipr = [{"a": a, "b": b, "mm": mm, "a_surface": pl.facing_surface(a, b), "b_surface": pl.facing_surface(b, a)}
           for (a, b), mm in sorted(r["ipr"].items())]
    out_json = {"ipr": ipr, "expansion_canine": float(r["expansion_canine"]), "expansion_posterior": float(r["expansion"]),
                "advance": float(r["advance"]), "distal": {}, "residual": residual,
                "gaps_initial": {}, "dz": {}, "log": list(r["log"]), "mode": "ALLINEAMENTO"}
    if len(r["line"]) >= 4:
        out_json["line"] = [[round(float(c), 4) for c in p] for p in ns["_resample"](np.asarray(r["line"]), 96)]
    targets = r["targets"]
    rest = {f: d["teeth"][f].frame() for f in d["seq"]}
    state = {"result": r, "setup_json": out_json, "targets": targets, "obj": obj, "config": cfg}
    if out:
        dump(out, "setup.json", {
            "case": ctx.name, "arch": ctx.arch,
            "inputs": {"teeth": "../teeth/teeth.json", "config": asdict(cfg) if is_dataclass(cfg) else vars(cfg),
                       "line": None, "fixed": [], "poses": {}},
            "setup": out_json,
            "targets_delta": {str(f): D for f, D in sorted(targets.items())},
            "targets_pose": {str(f): targets[f] @ rest[f] for f in sorted(targets)},
            "rest_frames": {str(f): rest[f] for f in sorted(rest)},
            "seconds": round(time.time() - t0, 1)})
    return state


# ---------------------------------------------------------------------------
# staging
# ---------------------------------------------------------------------------

def stage_staging(ctx, out):
    """Staging (staging.plan) of the set-up of the oracle: closed teeth (reconstruct.closed_tooth) in
    their rest frames, default staging parameters and pattern AUTO.  Mirrors case_plan._moves and
    run_staging (the plan numbers are the same JSON the plugin stores in plan_json)."""
    st, rc, pl = plugin.load("staging"), plugin.load("reconstruct"), plugin.load("planning")
    mu = ctx.mu
    d = _data_dict(ctx)
    su = ctx.state("setup")
    ns = _plan_ns(ctx)
    plugin.extract("case_plan", ["staging_params"], ns)
    E = mu.unique_edges(ctx.T)
    t0 = time.time()
    moves, closed = {}, {}
    for f in d["seq"]:
        t = d["teeth"][f]
        F = t.frame()
        Vt, Tt, info = rc.closed_tooth(d["V"], d["T"], d["lab"], E, f, t.e_ax)
        loc = mu.transform_points(mu.rigid_inverse(F), Vt)
        nbr = mu.face_adjacency(Tt, len(loc))
        bfaces = np.nonzero((nbr < 0).any(axis=1))[0]
        q = st.q_from_delta(F, su["targets"].get(f, np.eye(4)))     # run_setup: no target = stays at rest
        moves[f] = st.ToothMove(f, F, q, loc, Tt, bfaces, int(t.md_sign), q0=None, speed=1.0)
        closed[f] = (loc, Tt, info)
    t1 = time.time()
    pairs = pl.adjacent_pairs(d["seq"])
    ipr = {(r["a"], r["b"]): r["mm"] for r in su["setup_json"]["ipr"]}
    diag = ctx.state("diagnosis")
    ns["_data"] = lambda obj, s=None: d
    plugin.extract("case_plan", ["complexity"], ns)
    stub = types.ModuleType(f"{plugin.PKG}.review")           # no decision of the clinician in the classification review
    stub.decided = lambda obj, key: ""
    prev = sys.modules.get(stub.__name__)
    sys.modules[stub.__name__] = stub
    try:
        obj = diag["obj"]
        obj.oa.setup_json = json.dumps(su["setup_json"])
        cx = ns["complexity"](obj, moves)
    finally:
        if prev is None:
            sys.modules.pop(stub.__name__, None)
        else:
            sys.modules[stub.__name__] = prev
    p = ns["staging_params"](ctx.s)
    plan = st.plan(moves, pairs, ipr, p, cx, None)
    plan_json = {
        "pattern": plan["pattern"], "n_stages": plan["n_stages"], "complexity": cx,
        "q": {str(f): np.round(q, 6).tolist() for f, q in plan["q"].items()},
        "need": {str(f): int(n) for f, n in plan["need"].items()},
        "phases": {str(f): [[[int(x) for x in np.flatnonzero(m)], int(sa), int(n)] for m, sa, n in ph]
                   for f, ph in plan["schedule"].phases.items()},
        "ipr_stage": [[a, b, int(k)] for (a, b), k in plan["ipr_stage"].items()],
        "unresolved": [[int(k), a, b, float(pn)] for k, a, b, pn in plan["unresolved"]],
        "final_overlap": [[a, b, float(v)] for (a, b), v in plan["final_overlap"].items()],
        "stats": {str(f): {"max_step": np.asarray(v["max_step"]).tolist(), "limits": np.asarray(v["limits"]).tolist(),
                           "cap": v["cap"], "max_crown_step": v["max_crown_step"],
                           "moving_stages": v["moving_stages"]} for f, v in plan["stats"].items()},
        "q_final": {str(f): moves[f].q.tolist() for f in moves},
        "md_sign": {str(f): int(d["teeth"][f].md_sign) for f in moves},
        "overcorrection": bool(p.overcorrection), "interval_days": int(ctx.s.interval_days),
        "log": plan["log"],
        "params": {"step_linear": p.step_linear, "step_vertical": p.step_vertical, "step_rotation": p.step_rotation,
                   "step_rotation_round": p.step_rotation_round, "step_angular": p.step_angular,
                   "max_crown_step": p.max_crown_step, "perio": p.perio, "collision": p.collision,
                   "collision_tol": p.collision_tol}}
    state = {"plan": plan, "plan_json": plan_json, "moves": moves, "closed": closed, "obj": obj}
    if out:
        os.makedirs(out, exist_ok=True)
        dump(out, "staging.json", {
            "case": ctx.name, "arch": ctx.arch,
            "inputs": {"setup": "../setup/setup.json (targets_delta, ipr)", "params": plan_json["params"],
                       "pattern": ctx.s.pattern, "pairs": pairs,
                       "closed_teeth": "closed_<fdi>.dcm: crown solids in tooth-local coordinates (reconstruct.closed_tooth)"},
            "plan": plan_json,
            "closed_tooth_info": {str(f): v[2] for f, v in closed.items()},
            "seconds": {"closed_teeth": round(t1 - t0, 1), "plan": round(time.time() - t1, 1)}})
        if getattr(ctx.args, "closed_teeth", True):
            for f, (loc, Tt, _) in closed.items():
                mesh.write_dcm(os.path.join(out, f"closed_{f}.dcm"), loc, Tt)
    return state


# ---------------------------------------------------------------------------
# schedule
# ---------------------------------------------------------------------------

def stage_schedule(ctx, out):
    """Aligner change calendar (`schedule.build`, `stage_intervals`, `to_ics`) of the staging of the
    oracle, FIXED and ADAPTIVE policies, start date fixed to START_DATE."""
    sch = plugin.load("schedule")
    stg = ctx.state("staging")
    su = ctx.state("setup")
    obj = stg["obj"]
    obj.oa.plan_json = json.dumps(stg["plan_json"])
    obj.oa.setup_json = json.dumps(su["setup_json"])
    plan = json.loads(obj.oa.plan_json)
    res = {}
    ics = {}
    for policy in ("FIXED", "ADAPTIVE"):
        s = types.SimpleNamespace(**{**vars(ctx.s), "interval_policy": policy})
        cal = sch.build(obj, s)
        res[policy] = {"intervals": sch.stage_intervals(plan, policy, plan.get("interval_days", s.interval_days)),
                       "calendar": cal}
        try:
            ics[policy] = sch.to_ics(cal, "Golden", ctx.arch)
        except Exception as e:                      # keep the calendar even if the ICS builder needs more input
            ics[policy] = None
            res[policy]["ics_error"] = repr(e)
    state = {"calendar": res}
    if out:
        dump(out, "schedule.json", {"case": ctx.name, "arch": ctx.arch,
                                    "inputs": {"plan": "../staging/staging.json plan", "start_date": START_DATE},
                                    "policies": res})
        for k, v in ics.items():
            if v:
                with open(os.path.join(out, f"schedule_{k.lower()}.ics"), "w", newline="") as f:
                    f.write(v)
    return state


STAGES = {n: globals()[f"stage_{n}"] for n in ORDER}
