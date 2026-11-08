#!/usr/bin/env python3
"""Runs the plugin's segmentation (`case_teeth.segment_iter`) on the real scans and writes what
it finds to a folder OUTSIDE the repository, for `tests/segmentation_real.rs` to compare the
Rust port against.

    DC_REAL_SCANS=/path/to/scans python3 -B gen_segmentation_real.py /outside/the/repo/seg_real [--only a,c]
    DC_SEGMENTATION_REAL_ORACLE=/outside/the/repo/seg_real DC_REAL_SCANS=... cargo test --release -p dca-algo \
        --test segmentation_real -- --nocapture --test-threads=1

The tests are not ignored: they run with the rest and skip themselves (a line on stderr) when
either variable is missing.

The scans are patient data: nothing this script writes (seeds, arch frame, labels, the
simplified mesh) may be committed; delete the folder after the run.

Per scan (`a` to `d`: the four STL files of `DC_REAL_SCANS` sorted by name, arches lower,
upper, lower, upper, as `gen_autoseed_real.py`) it writes `<letter>.json` and
`<letter>_decimated.dcm`, exactly the steps of `segment_iter` with the default settings
(`props.OA_Settings`: `rw_faces` 30000, `rw_eta_convex`, `rw_sigma`):

1. the scan, welded on bit-identical f32 positions in first-use order: the vertex numbering of
   `dc_mesh::io` (relative tolerance 1e-6 of the bbox diagonal, the same on these files), so the
   labels compare vertex by vertex; `fingerprint` lets the Rust test check that;
2. the plugin's automatic seeds (`autoseed.arch_data` + `propose`, defaults as
   `case_auto.auto_seeds_iter`), the same ones `gen_autoseed_real.py` records; the Rust test
   reads them from here, so both sides segment from identical seeds;
3. the arch frame from the seeds (`case_core.compute_frame`, mirrored in `golden/oracle/frame.py`;
   no occlusal flip), the scan and the seeds moved to arch coordinates;
4. Blender's Decimate modifier to 30000 faces (`case_teeth._decimate_here`, run by this same
   script inside `blender -b --factory-startup`), written as `<letter>_decimated.dcm` (DCM1,
   arch coordinates, f32) and read back from that file, as the Rust test reads it;
5. `segmentation.segment_faces`, `case_teeth._labels_full` (label transfer and first clean-up),
   `segmentation.refine_boundaries`, `segmentation.clean_vertex_labels`.

The plugin is loaded read-only through `golden/oracle` (CPU only, `OA_GPU=0`; `mathutils` is the
exact float64 shim there, the real module inside Blender). Needs python3 + numpy and Blender
(`--blender`, default `blender` on PATH). A full-size scan takes several minutes, mostly the label
transfer: run the scans in parallel processes with `--only`.
"""
import argparse
import json
import os
import subprocess
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.normpath(os.path.join(HERE, *[".."] * 4, "golden"))
sys.dont_write_bytecode = True
sys.path.insert(0, GOLDEN)

from oracle import plugin  # noqa: E402  (sets OA_GPU=0, never writes into the plugin)

ARCH = {"a": "LOWER", "b": "UPPER", "c": "LOWER", "d": "UPPER"}


def blender_decimate(argv):
    """Inside Blender: `case_teeth._decimate_here` on the arrays of an .npz, to a .dcm."""
    import bpy
    from oracle import mesh
    root, src, dst, faces = argv
    plugin.PLUGIN_ROOT = root
    plugin.install(root)
    ns = {"bpy": bpy, "mu": plugin.load("meshutil"), "np": np}
    plugin.extract("case_teeth", ["_decimate_here"], ns)
    d = np.load(src)
    Vd, Td = ns["_decimate_here"](d["V"], d["T"], int(faces))
    mesh.write_dcm(dst, Vd, Td)


def read_stl(path):
    """Binary STL -> (V float64 of the f32 positions, T int64), welded on bit-identical
    positions, vertices numbered in first-use order (as `dc_mesh::io`)."""
    raw = open(path, "rb").read()
    n = int(np.frombuffer(raw, "<u4", 1, 80)[0])
    assert len(raw) == 84 + 50 * n, "only binary STL"
    rec = np.frombuffer(raw, np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")]), n, 84)
    soup = np.ascontiguousarray(rec["v"].reshape(-1, 3)) + np.float32(0.0)   # -0.0 -> 0.0
    _, first, inv = np.unique(soup.view(np.dtype((np.void, 12))).ravel(), return_index=True,
                              return_inverse=True)
    order = np.argsort(first)
    rank = np.empty_like(order)
    rank[order] = np.arange(len(order))
    V = soup[first[order]].astype(np.float64)
    return V, rank[inv.ravel()].reshape(-1, 3).astype(np.int64)


def fingerprint(V):
    """Order-sensitive checksum of the vertices (and its scale): no coordinate leaves the file."""
    w = (np.arange(len(V)) % 997 + 1).astype(np.float64)
    s = V @ np.array([1.0, 2.0, 3.0])
    return float(np.sum(w * s)), float(np.sum(w * np.abs(s)))


def counts(lab):
    u, c = np.unique(np.asarray(lab), return_counts=True)
    return {str(int(f)): int(k) for f, k in zip(u, c)}


def run(letter, path, args, out):
    from oracle import frame, mesh, settings, stages
    s = settings.defaults()
    auto, seg, mu = plugin.load("autoseed"), plugin.load("segmentation"), plugin.load("meshutil")
    fns = stages._full_res_fns()
    arch = ARCH[letter]
    V, T = read_stl(path)
    t0 = time.time()
    data = auto.arch_data(V, T)
    seeds_world, M_auto, _ = auto.propose(V, T, arch, False, (), data=data)
    t_seeds = time.time() - t0

    # case_teeth.segment_iter from here on
    t0 = time.time()
    M = frame.compute_frame(seeds_world, V)
    Minv = mu.rigid_inverse(M)
    V_arch = mu.transform_points(Minv, V)
    seeds = [(f, mu.transform_points(Minv, np.asarray(p)[None])[0]) for f, p in sorted(seeds_world.items())]
    npz = os.path.join(out, f"{letter}_arch.npz")
    dcm = os.path.join(out, f"{letter}_decimated.dcm")
    np.savez(npz, V=V_arch, T=T.astype(np.int32))
    try:
        subprocess.run([args.blender, "-b", "--factory-startup", "--python-exit-code", "1",
                        "-P", os.path.abspath(__file__), "--", "--blender-decimate", plugin.PLUGIN_ROOT,
                        npz, dcm, str(int(s.rw_faces))], check=True, stdout=subprocess.DEVNULL)
    finally:
        os.remove(npz)
    t_dec = time.time() - t0
    d = mesh.read_mesh(dcm)
    Vd, Td = np.asarray(d["V"], np.float64), np.asarray(d["T"], np.int64)
    t0 = time.time()
    flab, diag = seg.segment_faces(Vd, Td, seeds, arch, s.rw_eta_convex, s.rw_sigma)
    t1 = time.time()
    E, lab, seed_v = fns["_labels_full"](V_arch, T, Vd, Td, flab, seeds)
    lab_transfer = lab.copy()
    t2 = time.time()
    lab = seg.refine_boundaries(V_arch, T, lab)
    lab = seg.clean_vertex_labels(lab, E, seed_v)
    t3 = time.time()
    mesh.dump_json(os.path.join(out, f"{letter}.json"), stages.J({
        "arch": arch, "vertices": len(V), "faces": len(T), "fingerprint": fingerprint(V),
        "seeds_world": {str(int(f)): [float(x) for x in p] for f, p in sorted(seeds_world.items())},
        "frame": np.asarray(M, np.float64).tolist(),
        "autoseed_frame": np.asarray(M_auto, np.float64).tolist(),
        "params": {"faces": int(s.rw_faces), "eta_convex": s.rw_eta_convex, "sigma": s.rw_sigma},
        "decimated": {"vertices": len(Vd), "faces": len(Td)},
        "diagnostics": diag,
        "face_labels_decimated": mesh.pack_labels(flab),
        "vertex_labels_transfer": mesh.pack_labels(lab_transfer),
        "vertex_labels": mesh.pack_labels(lab),
        "vertices_per_label": counts(lab),
        "seconds": {"autoseed": round(t_seeds, 1), "frame_decimate": round(t_dec, 1),
                    "segment_faces": round(t1 - t0, 1), "transfer": round(t2 - t1, 1),
                    "refine_clean": round(t3 - t2, 1), "segmentation": round(t_dec + t3 - t0, 1)},
        "provenance": {"plugin_rev": subprocess.run(["git", "-C", plugin.PLUGIN_ROOT, "rev-parse", "HEAD"],
                                                    capture_output=True, text=True).stdout.strip(),
                       "numpy": np.__version__},
    }))
    teeth = sum(1 for f in np.unique(lab) if f != 0)
    print(f"scan {letter}: {len(seeds)} seeds, {teeth} teeth, segmentation {t_dec + t3 - t0:.0f} s",
          file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(prog="gen_segmentation_real.py", description=__doc__.split("\n\n")[0])
    ap.add_argument("out", help="output folder, outside the repository")
    ap.add_argument("--only", default="abcd", help="scan letters, e.g. a,c (default: all four)")
    ap.add_argument("--plugin", default=plugin.PLUGIN_ROOT, help="plugin repository root (read only)")
    ap.add_argument("--blender", default="blender", help="Blender executable (Decimate modifier)")
    args = ap.parse_args()
    plugin.PLUGIN_ROOT = os.path.abspath(args.plugin)
    plugin.install(plugin.PLUGIN_ROOT)
    scans = os.environ["DC_REAL_SCANS"]
    files = sorted(f for f in os.listdir(scans) if f.lower().endswith(".stl"))
    assert len(files) == 4, "the folder must hold the four scans"
    os.makedirs(args.out, exist_ok=True)
    for letter, name in zip("abcd", files):
        if letter in args.only:
            run(letter, os.path.join(scans, name), args, args.out)


if __name__ == "__main__":
    if "--blender-decimate" in sys.argv:
        blender_decimate(sys.argv[sys.argv.index("--blender-decimate") + 1:])
    else:
        main()
