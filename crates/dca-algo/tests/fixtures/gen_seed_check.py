#!/usr/bin/env python3
"""Generates seed_check.json: the plugin's seed correction after the segmentation
(segmentation.plan_corrections, renumber_spans, crown_seed, face_labels), to compare the
Rust port (dca_algo::seed_check) against.

    python3 -I gen_seed_check.py [path/to/blender-ortho-aligner] > seed_check.json

Needs only python3 and numpy (no Blender). The plugin modules are loaded by path as in
gen_autoseed.py (`bpy` and `gpu_query` stubbed: these functions use neither). The inputs
are synthetic: crown spans along the arch made from the typical widths with fixed random
perturbations, merged and split teeth, missing teeth; no patient data.
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
sys.modules["bpy"] = types.ModuleType("bpy")
pkg = types.ModuleType("oa_prova")
pkg.__path__ = [PKG]
sys.modules["oa_prova"] = pkg
gq = types.ModuleType("oa_prova.gpu_query")
sys.modules["oa_prova.gpu_query"] = gq
pkg.gpu_query = gq
an = importlib.import_module("oa_prova.anatomy")
seg = importlib.import_module("oa_prova.segmentation")

rng = np.random.default_rng(20261102)


def arch_spans(arch, teeth, noise):
    """Spans of `teeth` (in arch order) side by side, widths ARCH_WIDTHS * (1 + noise)."""
    s = -sum(seg.ARCH_WIDTHS[arch][an.index(f)] for f in teeth) / 2.0 + rng.normal(0, 1.5)
    out = {}
    for f in teeth:
        w = seg.ARCH_WIDTHS[arch][an.index(f)] * (1.0 + rng.normal(0, noise)) * 1.05
        out[f] = (float(s), float(s + w))
        s += w + abs(rng.normal(0, 0.2))
    return out


cases = []
for k in range(24):
    arch = an.UPPER if k % 2 else an.LOWER
    order = an.ARCH_ORDER[arch]
    lo, hi = int(rng.integers(0, 3)), 16 - int(rng.integers(0, 3))
    teeth = [f for f in order[lo:hi]]
    missing = []
    if k % 5 == 3:
        m = teeth[int(rng.integers(2, len(teeth) - 2))]
        teeth.remove(m)
        missing.append(m)
    spans = arch_spans(arch, teeth, 0.05 + 0.02 * (k % 4))
    # wrong numbers: shifted by one position along the arch
    shift = int(rng.integers(-1, 2))
    if shift:
        labs = [order[(order.index(f) + shift) % 16] for f in spans]
        if len(set(labs)) == len(labs):
            spans = dict(zip(labs, spans.values()))
    third = bool(k % 3 == 0)
    new, notes = seg.renumber_spans(spans, arch, third, missing)
    # a report: some weak boundaries, a sliver, a wide crown
    fs = sorted(spans, key=lambda f: sum(spans[f]))
    weak = []
    for j in range(len(fs) - 1):
        if rng.random() < 0.15:
            a, b = sorted((fs[j], fs[j + 1]))
            weak.append((a, b, 20.0, round(float(rng.uniform(0.3, 1.9)), 2), 0.0, 0.0))
    sp = dict(spans)
    if k % 4 == 1:
        f = fs[len(fs) // 2]
        sp[f] = (sp[f][0], sp[f][0] + 2.0)
    if k % 6 == 2:
        f = fs[-1]
        sp[f] = (sp[f][0], sp[f][0] + 19.0)
    molar = max(seg.ARCH_WIDTHS[arch].values())
    rep = {"spans": sp, "weak": weak, "molar": molar,
           "slivers": [f for f, (a, b) in sp.items() if b - a < seg.SLIVER_MM],
           "wide": [f for f, (a, b) in sp.items() if b - a > seg.SPLIT_CROWN * molar]}
    acts = seg.plan_corrections(rep)
    cases.append({
        "arch": "upper" if arch == an.UPPER else "lower",
        "third": third, "missing": missing,
        "spans": {str(f): list(v) for f, v in spans.items()},
        "renumber": {str(f): g for f, g in new.items()}, "notes": len(notes),
        "report": {"spans": {str(f): list(v) for f, v in sp.items()},
                   "weak": [list(w) for w in weak]},
        "slivers": rep["slivers"], "wide": rep["wide"],
        "issues": len(seg.report_issues(rep)),
        "actions": [list(a) for a in acts],
    })

pts = rng.normal(0, 3, (400, 3))
seed = seg.crown_seed(pts, np.ones(len(pts), bool))
T = rng.integers(0, 50, (200, 3))
lab = rng.integers(0, 3, 50)
out = {"cases": cases,
       "crown_seed": {"points": pts.tolist(), "seed": seed.tolist()},
       "face_labels": {"tris": T.tolist(), "labels": lab.tolist(),
                       "faces": seg.face_labels(T, lab).tolist()}}
json.dump(out, sys.stdout)
