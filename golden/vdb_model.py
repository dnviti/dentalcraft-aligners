"""OpenVDB oracle of the closed model field (`aligner._sdf_mesh_gn`).

    blender -b --factory-startup -P golden/vdb_model.py -- <in> <voxel> <blockout> <out>

Reads a triangle soup (`<in>.v.bin`: float32 x, y, z per vertex; `<in>.t.bin`:
uint32 triangle corners, little endian), builds the plugin's model surface of
it through Geometry Nodes (OpenVDB: mesh to SDF, the closing by `blockout`,
volume to mesh) and writes the result the same way to `<out>.v.bin` and
`<out>.t.bin`.  Used by `tests/shell_oracle.rs` (ignored test
`closed_model_matches_openvdb`) with the soup of `shell::model_parts`.  The
plugin is loaded from its read-only repository by path (`OA_PLUGIN`).
"""
import os
import sys
import time

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402
import numpy as np  # noqa: E402

from oracle import plugin  # noqa: E402

argv = sys.argv[sys.argv.index("--") + 1:]
stem, voxel, blockout, out = argv[0], float(argv[1]), float(argv[2]), argv[3]
sys.path.insert(0, os.path.abspath(plugin.PLUGIN_ROOT))
from ortho_aligner import aligner  # noqa: E402

V = np.fromfile(stem + ".v.bin", dtype="<f4").reshape(-1, 3).astype(np.float64)
T = np.fromfile(stem + ".t.bin", dtype="<u4").reshape(-1, 3).astype(np.int64)
t = time.perf_counter()
Vs, Ts = aligner._sdf_mesh_gn(bpy, V, T, voxel, None, 0.0, blockout)
print(f"[vdb_model] {len(V)}v/{len(T)}f -> {len(Vs)}v/{len(Ts)}f in {time.perf_counter() - t:.2f}s", flush=True)
np.asarray(Vs, dtype="<f4").tofile(out + ".v.bin")
np.asarray(Ts, dtype="<u4").tofile(out + ".t.bin")
