"""Generates make_base.json: the plugin's base closing (`aligner.make_base`,
operator `oa.make_base`) on golden demo scans opened at the bottom, to compare
`dca_algo::scan_base` against.

    blender -b --factory-startup -P gen_make_base.py -- --out make_base.json

Needs Blender (bmesh): the plugin is loaded read-only through the golden oracle
package (`golden/oracle/plugin.py`), nothing is installed or written in its
repository. The demo scans are closed (voxel remesh), so each is opened first:
the faces whose three points all lie less than CUT_MM above the lowest point
along the occlusal axis of the ground-truth arch frame (`frame_gt`, the frame
`case_core.compute_frame` gives with those seeds) are removed, and the points no
face uses any more go, the others keeping their order. The Rust test cuts the
same way and checks the counts.

`case_core.make_base` passes column 2 of the arch frame as the occlusal
direction; the object here has the identity world matrix, as the scan of a case.

Output per case and base height: the boundary edges closed (the plugin's return
value), the counts of points and faces before and after, the plane, the
enclosed volume and the area of the result (independent of the order of the
points and of the triangulation of the flat cap), the sum and the extremes of
the new points, and the largest move of an old point (none expected).
"""
import argparse
import json
import os
import sys

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.normpath(os.path.join(HERE, "../../../../golden"))
sys.path.insert(0, GOLDEN)

import bpy  # noqa: E402
import numpy as np  # noqa: E402

from oracle import mesh, plugin  # noqa: E402

CASES = ["demo_lower_mild", "demo_upper_moderate"]
CUT_MM = 4.0
HEIGHTS = [3.0, 0.5]


def opened(V, T, z):
    """The scan without its faces lower than CUT_MM over its lowest point."""
    h = V.astype(np.float64) @ z
    low = h < h.min() + CUT_MM
    keep = ~low[T].all(axis=1)
    Tk = T[keep]
    used = np.zeros(len(V), bool)
    used[Tk.ravel()] = True
    remap = np.cumsum(used) - 1
    return V[used], remap[Tk].astype(np.int64), int((~keep).sum())


def volume_area(V, T):
    a, b, c = (V[T[:, k]].astype(np.float64) for k in range(3))
    vol = float(np.einsum("ij,ij->i", a, np.cross(b, c)).sum() / 6.0)
    area = float(0.5 * np.linalg.norm(np.cross(b - a, c - a), axis=1).sum())
    return vol, area


def run(V, T, z, height, al):
    me = bpy.data.meshes.new("oa_base_case")
    me.from_pydata([tuple(map(float, p)) for p in V], [], [tuple(map(int, t)) for t in T])
    me.update()
    obj = bpy.data.objects.new("oa_base_case", me)
    bpy.context.scene.collection.objects.link(obj)
    n = al.make_base(obj, tuple(float(x) for x in z), height)
    Vo = np.zeros(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", Vo)
    Vo = Vo.reshape(-1, 3)
    me.calc_loop_triangles()
    To = np.zeros(len(me.loop_triangles) * 3, np.int64)
    me.loop_triangles.foreach_get("vertices", To)
    To = To.reshape(-1, 3)
    bpy.data.objects.remove(obj, do_unlink=True)
    bpy.data.meshes.remove(me)
    return n, Vo, To


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser(prog="gen_make_base.py")
    ap.add_argument("--out", default=os.path.join(HERE, "make_base.json"))
    args = ap.parse_args(argv)
    plugin.install()
    al = plugin.load("aligner")
    out = {"cut_mm": CUT_MM, "cases": {}}
    for name in CASES:
        d = os.path.join(GOLDEN, "fixtures", name)
        meta = json.load(open(os.path.join(d, "meta.json")))
        z = np.asarray(meta["frame_gt"], np.float64)[:3, 2]
        scan = mesh.read_dcm(os.path.join(d, "scan.dcm"))
        V, T, removed = opened(np.asarray(scan["V"], np.float32), np.asarray(scan["T"], np.int64), z)
        runs = {}
        for height in HEIGHTS:
            n, Vo, To = run(V, T, z, height, al)
            new = Vo[len(V):].astype(np.float64)
            hz = V.astype(np.float64) @ z
            vol, area = volume_area(Vo, To)
            runs[str(height)] = {
                "closed_edges": int(n),
                "vertices": int(len(Vo)),
                "faces": int(len(To)),
                "plane": float(hz.min() - height),
                "new_heights": [float((new @ z).min()), float((new @ z).max())],
                "new_sum": [float(x) for x in new.sum(axis=0)],
                "new_min": [float(x) for x in new.min(axis=0)],
                "new_max": [float(x) for x in new.max(axis=0)],
                "old_moved": float(np.abs(Vo[:len(V)].astype(np.float64) - V).max()),
                "volume": vol,
                "area": area,
            }
        out["cases"][name] = {
            "occlusal": [float(x) for x in z],
            "removed_faces": removed,
            "vertices": int(len(V)),
            "faces": int(len(T)),
            "runs": runs,
        }
        print(f"[make_base] {name}: opened {len(V)}v/{len(T)}f, "
              + ", ".join(f"{h} mm: {r['closed_edges']} edges, volume {r['volume']:.1f}"
                          for h, r in runs.items()))
    out["provenance"] = {"blender": bpy.app.version_string, "numpy": np.__version__}
    with open(args.out, "w") as f:
        json.dump(out, f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    main()
