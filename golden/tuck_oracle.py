"""Plugin oracle of the stumps drawn under the gingiva (`case_print._tuck_below_margin`).

    blender -b --factory-startup -P golden/tuck_oracle.py -- <in> <out>

Reads `<in>/tuck.json`: the cast seen from above (`h`: `<in>/h.bin`,
float64 per node, row-major in x, `nx`, `ny`, `lo` [x, y], `cell`: the
`info` of `aligner.gum_cast`) and the closed teeth (`teeth`: `fdi`,
`crown_vertices`), every tooth as `<in>/tooth_<fdi>.p.bin` (float64 x, y, z
per vertex in its own frame) and `<in>/tooth_<fdi>.m.bin` (its placement in
arch coordinates, 4 x 4 float64, row-major), little endian.  The vertices
below the gingival margin are those of `_below_margin` (the plugin reads its
`oa_rebuilt` mask from the mesh: every vertex from `crown_vertices` on, as
`case_teeth._tooth_object` writes it); every tooth goes through the plugin's
`_tuck_below_margin` and is written as `<out>/tooth_<fdi>.w.bin` (float64,
arch coordinates) with its mask `<out>/tooth_<fdi>.below.bin` (uint8).  Used
by `tests/shell_oracle.rs` (ignored test `stumps_are_tucked_as_the_plugin_does`).
The plugin is loaded from its read-only repository by path (`OA_PLUGIN`).
"""
import json
import os
import sys

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import numpy as np  # noqa: E402

from oracle import plugin  # noqa: E402

argv = sys.argv[sys.argv.index("--") + 1:]
src, out = argv[0], argv[1]
sys.path.insert(0, os.path.abspath(plugin.PLUGIN_ROOT))
from ortho_aligner import case_print, meshutil  # noqa: E402

with open(os.path.join(src, "tuck.json"), encoding="utf-8") as fh:
    spec = json.load(fh)
H = np.fromfile(os.path.join(src, "h.bin"), dtype="<f8").reshape(spec["nx"], spec["ny"])
info = {"H": H, "lo": np.asarray(spec["lo"], np.float64), "cell": float(spec["cell"])}
os.makedirs(out, exist_ok=True)
moved = 0
for t in spec["teeth"]:
    stem = os.path.join(src, f"tooth_{t['fdi']}")
    P = np.fromfile(stem + ".p.bin", dtype="<f8").reshape(-1, 3)
    M = np.fromfile(stem + ".m.bin", dtype="<f8").reshape(4, 4)
    reb = np.zeros(len(P), bool)
    reb[t["crown_vertices"]:] = True
    # case_print._below_margin on that mask
    low = P[~reb, 2].min() - case_print.GUM_TUCK_MM if (~reb).any() else -np.inf
    below = reb & (P[:, 2] < low)
    Pw = case_print._tuck_below_margin(P, M, below, info)
    moved += int((np.abs(Pw - meshutil.transform_points(M, P)).max(axis=1) > 0).sum())
    o = os.path.join(out, f"tooth_{t['fdi']}")
    np.asarray(Pw, "<f8").tofile(o + ".w.bin")
    below.astype(np.uint8).tofile(o + ".below.bin")
print(f"[tuck_oracle] {len(spec['teeth'])} teeth, {moved} vertices drawn in", flush=True)
