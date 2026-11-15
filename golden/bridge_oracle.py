"""Plugin oracle of the contact bridges (`case_print.pair_bridge`).

    blender -b --factory-startup -P golden/bridge_oracle.py -- <in> <out>

Reads the closed teeth of a stage placed in arch coordinates (`+Z`
occlusal), one raw soup per tooth (`<in>/tooth_<fdi>.v.bin`: float32 x, y, z
per vertex; `<in>/tooth_<fdi>.t.bin`: uint32 triangle corners, little
endian), in the order of `<in>/order.txt` (the arch order, one FDI number per
line) and the arguments of `pair_bridge` in `<in>/params.json` (`reach`,
`gingival`, `touch`, `above`, `inset`).  Every two consecutive teeth go
through the plugin's `pair_bridge` (identity placements: the teeth are placed
already, which is the same geometry as the plugin's local frames); every
bridge made is written as `<out>/bridge_<fa>_<fb>.v.bin` / `.t.bin` (the same
raw format; `.v64.bin`: the vertices in float64) and `<out>/bridges.json`
lists the pairs with the top of their contact, the filled cells and the
volume, and `gpu`: whether the queries ran on the GPU.  Used by
`tests/shell_oracle.rs` (ignored test `contact_bridges_match_the_plugin`).
The plugin is loaded from its read-only repository by path (`OA_PLUGIN`).

Unlike the other oracles this one runs the plugin's GPU queries
(`gpu_query`, float64, the reference its CPU path emulates): the bridges are
compared vertex for vertex, and on the CPU path the float32 BVH of mathutils
misses a hit nearer the origin of a ray than its rounding (demo_upper_mild,
stage 0, 23-24: one line starting 4.5e-7 mm inside 23 left out).
"""
import json
import os
import sys
import time

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import numpy as np  # noqa: E402
from mathutils.kdtree import KDTree  # noqa: E402

from oracle import plugin  # noqa: E402

os.environ["OA_GPU"] = "1"      # after `oracle`, which sets the CPU path
argv = sys.argv[sys.argv.index("--") + 1:]
src, out = argv[0], argv[1]
sys.path.insert(0, os.path.abspath(plugin.PLUGIN_ROOT))
from ortho_aligner import accel, case_print, gpu_query, meshutil  # noqa: E402

with open(os.path.join(src, "order.txt"), encoding="utf-8") as fh:
    order = [int(line) for line in fh if line.strip()]
with open(os.path.join(src, "params.json"), encoding="utf-8") as fh:
    params = json.load(fh)


def tooth(fdi):
    stem = os.path.join(src, f"tooth_{fdi}")
    P = np.fromfile(stem + ".v.bin", dtype="<f4").reshape(-1, 3).astype(np.float64)
    T = np.fromfile(stem + ".t.bin", dtype="<u4").reshape(-1, 3).astype(np.int64)
    kd = KDTree(len(P))
    for i, p in enumerate(P):
        kd.insert(p, i)
    kd.balance()
    return (P, kd, None, gpu_query.Mesh(P, T))


teeth = {f: tooth(f) for f in order}
eye = np.eye(4)
os.makedirs(out, exist_ok=True)
pairs = []
t0 = time.perf_counter()
for fa, fb in zip(order[:-1], order[1:]):
    info = {}
    solid = case_print.pair_bridge(teeth[fa], teeth[fb], eye, eye, reach=params["reach"],
                                   gingival=params["gingival"], info=info, touch=params["touch"],
                                   above=params["above"], inset=params["inset"])
    rec = {"a": fa, "b": fb, "bridged": solid is not None}
    if "z_c" in info:
        rec["z_c"] = float(info["z_c"])
    if solid is not None:
        V, T = solid
        stem = os.path.join(out, f"bridge_{fa}_{fb}")
        np.asarray(V, dtype="<f4").tofile(stem + ".v.bin")
        np.asarray(V, dtype="<f8").tofile(stem + ".v64.bin")
        np.asarray(T, dtype="<u4").tofile(stem + ".t.bin")
        rec["volume"] = meshutil.signed_volume(np.asarray(V, np.float64), np.asarray(T, np.int64))
        rec["cells"] = int(np.asarray(info["cells"]).sum())
    pairs.append(rec)
with open(os.path.join(out, "bridges.json"), "w", encoding="utf-8") as fh:
    json.dump({"pairs": pairs, "seconds": time.perf_counter() - t0, "gpu": bool(accel.active())}, fh, indent=1)
print(f"[bridge_oracle] {sum(p['bridged'] for p in pairs)} of {len(pairs)} pairs bridged "
      f"in {time.perf_counter() - t0:.1f}s", flush=True)
