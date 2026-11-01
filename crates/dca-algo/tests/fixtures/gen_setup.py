#!/usr/bin/env python3
"""Generates setup_options.json: what the plugin's conservative setup
(`setup_conservativo.conservative_setup`) gives on the synthetic arches of its own unit
tests (`tests/unit/test_setup_conservativo.py`: `make_arch`, `_crowded`, `_crown`), for
the options the golden demo cases do not cover (locked and excluded teeth, a drawn line,
the order of the therapy, crowns, levelling, midline, scope).

    python3 -B -I gen_setup.py [path/to/blender-ortho-aligner] > setup_options.json

Needs only python3 and numpy (no Blender). The plugin repository is read-only: nothing is
written there (no bytecode either). The Rust test (`dca_algo::setup` unit tests) builds
the same inputs with its port of the helpers and compares the outputs.
"""
import json
import os
import sys

import numpy as np

REPO = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "ORTHO_ALIGNER_REPO",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 6, "blender-ortho-aligner"))
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(REPO, "tests", "unit"))

import test_setup_conservativo as t  # noqa: E402

sc = t.sc


def full(**kw):
    return t.full(**kw)


def translate_md(teeth, f, mm):
    D = np.eye(4)
    D[:3, 3] = teeth[f].e_md * mm
    return D


def scenarios():
    """(name, teeth, cusps, cfg, kwargs of conservative_setup)."""
    out = []
    arch = t.make_arch

    teeth, cusps = arch(shift={32: (0.0, -1.2, 0.0)})
    out.append(("shifted", teeth, cusps, full(), {}))
    teeth, cusps = arch(shift={32: (0.0, -1.2, 0.0), 42: (0.0, -1.2, 0.0)})
    out.append(("excluded", teeth, cusps, full(), {"fixed": (32,)}))
    teeth, cusps = arch()
    out.append(("locked_moved", teeth, cusps, full(), {"poses": {42: translate_md(teeth, 42, 1.0)}}))
    teeth, cusps = arch(shift={42: (0.0, -1.2, 0.0)})
    out.append(("locked_crooked", teeth, cusps, sc.Config(), {"poses": {42: np.eye(4)}}))
    teeth, cusps = arch()
    L = sc.cusp_line(cusps, t.SEQ, sigma=5.0)
    L2 = L.copy()
    L2[:, :2] += 0.8 * sc.line_normals(L2, cusps)
    out.append(("drawn_line", teeth, cusps, full(), {"line": L2}))
    widths = {42: 6.3, 41: 5.8, 31: 5.8, 32: 6.3}
    teeth, cusps = t._crowded(widths)
    out.append(("ipr_fixed_canines", teeth, cusps, full(fix_canines=True), {}))
    out.append(("exp_first", teeth, cusps, full(priority=("EXP", "IPR", "PRO"), expansion_canine=2.0), {}))
    out.append(("pro_first", teeth, cusps, full(priority=("PRO", "IPR", "EXP"), advance_max=2.0), {}))
    out.append(("expansion_in_place", teeth, cusps,
                sc.Config(align_scope="ALL", priority=("EXP", "IPR", "PRO"), expansion_posterior=2.0,
                          expansion_canine=1.0, advance_max=0.0, ipr_zone="NONE"), {}))
    teeth, cusps = t._crowded({41: 4.65, 31: 4.65})
    out.append(("diastema_retract", teeth, cusps, full(), {}))
    out.append(("diastema_open", teeth, cusps, full(close_spaces=False), {}))
    teeth, cusps = t._crowded({43: 8.8, 42: 7.5, 41: 7.0, 31: 7.0, 32: 7.5, 33: 8.8})
    out.append(("severe", teeth, cusps, full(expansion_posterior=0.0, advance_max=0.0), {}))
    teeth, cusps = arch(rot={42: 30.0, 43: 15.0})
    for f, a in ((42, 30.0), (43, 15.0)):
        g = teeth[f]
        w = t.WIDTHS[f % 10] / np.cos(np.radians(a))
        c = cusps[f].mean(0)
        g.width_md, g.contact_m, g.contact_d = w, c + g.e_md * w / 2, c - g.e_md * w / 2
    out.append(("partial_derotation", teeth, cusps,
                sc.Config(ipr_surface_cap=0.05, advance_max=0.0, expansion_canine=0.0), {}))
    teeth, cusps = arch(rot={41: 20.0})
    teeth[41].e_md = arch()[0][41].e_md
    cusps[41] = arch()[1][41]
    straight, scusps = arch()
    crowns = {f: t._crown(straight, scusps, f, 20.0 if f == 41 else 0.0) for f in (42, 41, 31, 32)}
    out.append(("crown_axis", teeth, cusps, sc.Config(ipr_zone="ANTERIOR_PREMOLARS"), {"crowns": crowns}))
    teeth, cusps = arch()
    crowns = {f: t._crown(teeth, cusps, f) for f in (46, 45, 44, 43, 33, 34, 35, 36)}
    for f in (45, 44, 34, 35):
        g = teeth[f]
        c = cusps[f].mean(0)
        w = g.width_md + 0.8
        g.contact_m, g.contact_d = c + g.e_md * w / 2, c - g.e_md * w / 2
    out.append(("crown_contacts", teeth, cusps, sc.Config(align_scope="ALL"), {"crowns": crowns}))
    teeth, cusps = arch(shift={44: (0.0, 0.0, 0.6), 32: (0.0, -0.6, 0.4)})
    out.append(("flat_leveling", teeth, cusps, full(leveling="FLAT"), {}))
    teeth, cusps = arch(shift={43: (0.0, 0.0, 1.0), 33: (0.0, 0.0, 1.0), 42: (0.0, -1.0, 0.0)})
    out.append(("level_canines", teeth, cusps, sc.Config(align_scope="ALL", level_canines=True), {}))
    teeth, cusps = arch(shift={44: (0.0, -0.8, 0.0), 32: (0.0, -1.0, 0.0)}, rot={46: 6.0})
    out.append(("premolars", teeth, cusps, sc.Config(align_scope="PREMOLARS"), {}))
    teeth, cusps = arch(shift={41: (1.0, 0.0, 0.0), 31: (1.0, 0.0, 0.0), 32: (0.0, -0.8, 0.0)})
    out.append(("midline_balance", teeth, cusps, sc.Config(midline="BALANCE"), {}))
    teeth, cusps = arch(shift={46: (0.0, -0.6, 0.0)}, rot={36: 8.0})
    out.append(("molars_free", teeth, cusps,
                sc.Config(align_scope="ALL", fix_molars=False, fix_third=False, align_fixed=False), {}))
    return out


def mat(D):
    return [[float(x) for x in row] for row in np.asarray(D)]


def main():
    cases = {}
    for name, teeth, cusps, cfg, kw in scenarios():
        r = sc.conservative_setup(teeth, t.SEQ, cusps, cfg, **kw)
        cases[name] = {
            "targets": {str(f): mat(D) for f, D in sorted(r["targets"].items())},
            "ipr": [[a, b, mm] for (a, b), mm in sorted(r["ipr"].items())],
            "expansion": float(r["expansion"]),
            "expansion_canine": float(r["expansion_canine"]),
            "advance": float(r["advance"]),
            "residual": float(r["residual"]),
            "spacing": float(r["spacing"]),
            "overlap_max": float(r["overlap_max"]),
            "midline_shift": float(r["midline_shift"]),
            "held": [int(f) for f in r["held"]],
            "line": [[float(c) for c in p] for p in np.asarray(r["line"])[::8]],
        }
    json.dump({"generator": "gen_setup.py", "cases": cases}, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
