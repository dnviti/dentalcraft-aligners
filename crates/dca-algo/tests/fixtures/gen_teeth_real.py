#!/usr/bin/env python3
"""Runs the plugin's 3D teeth step (`teeth.compute_teeth`, `reconstruct.closed_tooth`,
`gum_surface`, `gum_weights`, `deform_prep` / `deform`) on the real scans, from the plugin's own
segmentation of them, and writes what it finds to a folder OUTSIDE the repository, for
`tests/teeth_real.rs` to compare the Rust port against.

    out=$(mktemp -d /tmp/dc-teethreal-XXXX)
    DC_REAL_SCANS=/path/to/scans python3 -B gen_segmentation_real.py "$out" --only a   # one per scan
    DC_REAL_SCANS=/path/to/scans python3 -B gen_teeth_real.py "$out" --only a          # one per scan
    DC_REAL_SCANS=... DC_TEETH_REAL_ORACLE="$out" cargo test --release -p dca-algo \
        --test teeth_real -- --nocapture --test-threads=1

The tests are not ignored: they run with the rest and skip themselves (a line on stderr) when
either variable is missing.

The scans are patient data: nothing this script writes (labels, frames, closed teeth, the
gingiva) may be committed; delete the folder after the run.

The step is isolated: its inputs are the output of `gen_segmentation_real.py` in the same folder
(`<letter>.json`: the plugin's arch frame and final vertex labels), so both sides start from
identical labels. Per scan (`a` to `d`, as `gen_segmentation_real.py`) it reads the STL again
(same welding, checked with the fingerprint), moves it to arch coordinates with the plugin's frame
(`mu.rigid_inverse`, `mu.transform_points`) and writes:

* `<letter>_teeth.json`: the sequence and every tooth of `compute_teeth` (the fields of
  `gen_teeth.py`), per closed tooth its info dict, sizes, whether every vertex is finite and the
  seconds; the gingiva counts and flags histogram; two test motions, `mild` (`gen_gingiva.motion`:
  the two central teeth of the sequence moved (0.6, -0.4, 0.2) mm and turned 6 degrees about their
  long axis through the centre of resistance, the first tooth of the sequence moved (0, 0.5, 0) mm)
  and `strong` (`strong_motion`: 15 degrees and 2 mm, every third tooth 2 mm inwards), and per
  motion the deformation quality of the plugin's gingiva under it, measured with the plugin's own
  formulas (`tests/archive/test_gingiva_real.py` `tri_stats`: flipped triangles of the whole and
  of the shown gingiva, longest edge over its rest length; `tests/archive/dev/dev_gum_quality.py`
  `quality`: folded triangles, edge stretch p99.9 / max / above 3x on the visible and hidden
  parts) and the seconds of every stage;
* `<letter>_closed_<fdi>.dcm`: every closed tooth, arch coordinates (DCM1, f32);
* the gingiva as raw little-endian arrays: `<letter>_gum_rest.f64`, `<letter>_gum_mild.f64` and
  `<letter>_gum_strong.f64` (n x 3 float64: rest and deformed positions), `<letter>_gum_tris.u32` (m x 3),
  `<letter>_gum_flags.u16` (n: tooth | 0x4000 margin | 0x8000 membrane | 0x2000 col, the flags
  of `dca_algo::gingiva`), `<letter>_gum_weights.f32` (n x teeth, row-major, column order the
  sequence).

The plugin is loaded read-only through `golden/oracle` (CPU only, `OA_GPU=0`; `mathutils` is the
exact float64 shim, as in the golden staging oracle). Needs python3 + numpy only. The closed
teeth of a scan run in `--jobs` spawned processes (default 4); a full-size scan takes
minutes, mostly the closed teeth and the gingiva weights: run the scans in parallel with `--only`.
"""
import argparse
import json
import multiprocessing
import os
import subprocess
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)

from gen_gingiva import motion, rot  # noqa: E402  (also puts golden/ on the path)
from gen_segmentation_real import ARCH, fingerprint, read_stl  # noqa: E402
from oracle import mesh, plugin, stages  # noqa: E402

FLAG_MARGIN, FLAG_MEMBRANE, FLAG_COL = 0x4000, 0x8000, 0x2000

_CLOSE = {}     # inputs of the closed-tooth workers


def close_init(root, args):
    """A spawned worker: the plugin (read-only) and the inputs of its closed teeth."""
    plugin.PLUGIN_ROOT = root
    plugin.install(root)
    _CLOSE["args"] = args


def L(a):
    return np.asarray(a, np.float64).tolist()


def tooth_record(t):
    """Every field `gen_teeth.py` records, unrounded."""
    return {
        "fdi": int(t.fdi), "md_sign": int(t.md_sign), "e_md": L(t.e_md), "e_bl": L(t.e_bl), "e_ax": L(t.e_ax),
        "width_md": float(t.width_md), "width_bl": float(t.width_bl), "height": float(t.height),
        "occ_z": float(t.occ_z), "tip": L(t.tip), "margin_mean": L(t.margin_mean), "cej": L(t.cej),
        "crest": L(t.crest), "apex": L(t.apex), "cres": L(t.cres), "contact_m": L(t.contact_m),
        "contact_d": L(t.contact_d), "rot_dev": float(t.rot_dev), "zen_b": float(t.zen_b),
        "zen_l": float(t.zen_l), "mesial_dir": L(t.mesial_dir), "out_dir": L(t.out_dir),
        "axis_tilt": float(t.axis_tilt), "top_elong": float(t.top_elong), "top_spread": float(t.top_spread),
        "centroid": L(t.centroid), "ref": L(t.ref), "n_margin": len(t.margin), "n_vertices": len(t.vidx),
        "n_faces": len(t.fidx), "frame": L(t.frame()),
    }


def close_one(f):
    """One closed tooth in a worker: (fdi, V, T, info, seconds)."""
    rc, (Va, T, lab, E, axes) = plugin.load("reconstruct"), _CLOSE["args"]
    t0 = time.time()
    Vt, Tt, info = rc.closed_tooth(Va, T, lab, E, f, axes[f])
    return f, np.asarray(Vt, np.float64), np.asarray(Tt, np.int64), info, time.time() - t0


def tri_stats(V, V0, T):
    """test_gingiva_real.tri_stats: (stretch of the longest edge, cosine between the deformed and the
    rest normal, rest area x 2)."""
    def longest(W):
        return np.max([np.linalg.norm(W[T[:, (i + 1) % 3]] - W[T[:, i]], axis=1) for i in range(3)], axis=0)

    def normal(W):
        return np.cross(W[T[:, 1]] - W[T[:, 0]], W[T[:, 2]] - W[T[:, 0]])
    n1, n0 = normal(V), normal(V0)
    a1, a0 = np.linalg.norm(n1, axis=1), np.linalg.norm(n0, axis=1)
    cos = np.einsum("ij,ij->i", n1, n0) / np.maximum(a1 * a0, 1e-12)
    return longest(V) / np.maximum(longest(V0), 1e-6), cos, a0


def quality(mu, V0, V1, T):
    """dev_gum_quality.quality: (folded, stretch p99.9, stretch max, edges above 3x)."""
    if not len(T):
        return 0, 0.0, 0.0, 0
    n0, a0 = mu.tri_normals(V0, T)
    n1, a1 = mu.tri_normals(V1, T)
    ok = a0 > 1e-8
    flip = np.sum((np.einsum("ij,ij->i", n0, n1) < 0) & ok)
    e = lambda V: np.stack([np.linalg.norm(V[T[:, k]] - V[T[:, (k + 1) % 3]], axis=1) for k in range(3)], 1)  # noqa: E731
    r = e(V1) / np.maximum(e(V0), 1e-6)
    stretch = r.max(axis=1)
    return int(flip), float(np.percentile(stretch, 99.9)), float(stretch.max()), int(np.sum(stretch > 3.0))


def strong_motion(teeth, seq):
    """The larger test motion: the two central teeth of the sequence turned 15 degrees about their
    long axis through the centre of resistance and moved (2, -1.5, 0.5) mm, every third tooth from
    the second moved 2 mm inwards (against `out_dir`), the first tooth moved (0, 2, 0) mm."""
    out = {}
    for k, f in enumerate(seq):
        if k % 3 == 1:
            M = np.eye(4)
            M[:3, 3] = -2.0 * np.asarray(teeth[str(f)]["out_dir"], np.float64)
            out[f] = M
    mid = len(seq) // 2
    for f in (seq[mid - 1], seq[mid]):
        t = teeth[str(f)]
        R = rot(t["e_ax"], 15.0)
        c = np.asarray(t["cres"], np.float64)
        M = np.eye(4)
        M[:3, :3] = R
        M[:3, 3] = c - R @ c + np.array([2.0, -1.5, 0.5])
        out[f] = M
    M = np.eye(4)
    M[:3, 3] = [0.0, 2.0, 0.0]
    out[seq[0]] = M
    return out


def raw(path, a, dtype):
    np.ascontiguousarray(a, dtype=dtype).tofile(path)


def run(letter, path, out, jobs):
    mu, tg, rc = plugin.load("meshutil"), plugin.load("teeth"), plugin.load("reconstruct")
    seg = json.load(open(os.path.join(out, f"{letter}.json")))
    arch = ARCH[letter]
    assert seg["arch"] == arch, f"scan {letter}: the segmentation oracle has another arch"
    V, T = read_stl(path)
    fp, scale = fingerprint(V)
    assert abs(fp - seg["fingerprint"][0]) <= 1e-9 * max(scale, seg["fingerprint"][1]), \
        f"scan {letter}: not the mesh the segmentation oracle saw"
    Va = mu.transform_points(mu.rigid_inverse(np.asarray(seg["frame"], np.float64)), V)
    lab = mesh.unpack_labels(seg["vertex_labels"]).astype(np.int64)
    sec = {}

    t0 = time.time()
    teeth, seq = tg.compute_teeth(Va, T, lab, arch)
    sec["compute_teeth"] = time.time() - t0
    t0 = time.time()
    E = mu.unique_edges(T)
    sec["unique_edges"] = time.time() - t0

    # closed teeth, in spawned workers (forked ones hung, measured)
    args = (Va, T, lab, E, {f: np.asarray(teeth[f].e_ax, np.float64) for f in seq})
    t0 = time.time()
    with multiprocessing.get_context("spawn").Pool(max(1, min(jobs, len(seq))), close_init,
                                                   (plugin.PLUGIN_ROOT, args)) as pool:
        done = pool.map(close_one, seq, chunksize=1)
    sec["closed_teeth_wall"] = time.time() - t0
    closed = {}
    for f, Vt, Tt, info, s in done:
        mesh.write_dcm(os.path.join(out, f"{letter}_closed_{f}.dcm"), Vt, Tt)
        closed[str(f)] = {"info": info, "n_vertices": len(Vt), "n_tris": len(Tt),
                          "finite": bool(np.isfinite(Vt).all()), "seconds": s}

    t0 = time.time()
    G = rc.gum_surface(Va, T, lab, E)
    sec["gum_surface"] = time.time() - t0
    t0 = time.time()
    W = np.asarray(rc.gum_weights(G, seq), np.float32)
    sec["gum_weights"] = time.time() - t0
    trec = {str(f): {"e_ax": L(teeth[f].e_ax), "cres": L(teeth[f].cres), "out_dir": L(teeth[f].out_dir)}
            for f in seq}
    motions = {"mild": motion(trec, seq), "strong": strong_motion(trec, seq)}
    t0 = time.time()
    prep = rc.deform_prep(G["V"], W, seq)
    moved = {k: rc.deform(G["V"], W, seq, deltas, prep) for k, deltas in motions.items()}
    sec["deform"] = (time.time() - t0) / len(motions)

    Vg, Tg = np.asarray(G["V"], np.float64), np.asarray(G["T"], np.int64)
    flags = (np.asarray(G["tooth"], np.int64) & 0xFF) | FLAG_MARGIN * G["margin"].astype(np.int64) \
        | FLAG_MEMBRANE * G["membrane"].astype(np.int64) | FLAG_COL * G["col"].astype(np.int64)
    raw(os.path.join(out, f"{letter}_gum_rest.f64"), Vg, "<f8")
    raw(os.path.join(out, f"{letter}_gum_tris.u32"), Tg, "<u4")
    raw(os.path.join(out, f"{letter}_gum_flags.u16"), flags, "<u2")
    raw(os.path.join(out, f"{letter}_gum_weights.f32"), W, "<f4")

    # deformation quality, the plugin's formulas
    under = G["membrane"] & ~G["col"]
    shown = ~under[Tg].all(axis=1)
    vis = ~G["membrane"][Tg].all(axis=1)
    per_motion = {}
    for name, Vd in moved.items():
        raw(os.path.join(out, f"{letter}_gum_{name}.f64"), Vd, "<f8")
        st, cos, a0 = tri_stats(Vd, Vg, Tg)
        flipped = (cos < 0) & (a0 > 1e-8)
        dev = {}
        for part, sel in (("visible", vis), ("hidden", ~vis)):
            fl, p999, mx, n3 = quality(mu, Vg, Vd, Tg[sel])
            dev[part] = {"triangles": int(sel.sum()), "folded": fl, "stretch_p999": p999, "stretch_max": mx,
                         "stretch_gt3": n3}
        per_motion[name] = {
            "deltas": {str(f): np.asarray(M, np.float64).tolist() for f, M in motions[name].items()},
            "finite": bool(np.isfinite(Vd).all()),
            "quality": {"triangles": len(Tg), "flipped": int(flipped.sum()), "shown": int(shown.sum()),
                        "flipped_shown": int((flipped & shown).sum()), "stretch_max": float(st.max()),
                        "stretch_gt3": int((st > 3.0).sum()),
                        "moved_max_mm": float(np.linalg.norm(Vd - Vg, axis=1).max())},
            "quality_dev": dev,
        }
    gum = {
        "n_vertices": len(Vg), "n_faces": len(Tg), "n_scan": int(G["n_scan"]),
        "n_margin": int(G["margin"].sum()), "n_membrane": int(G["membrane"].sum()), "n_col": int(G["col"].sum()),
        "tooth_hist": {str(int(f)): int(c) for f, c in zip(*np.unique(G["tooth"], return_counts=True))},
        "finite": bool(np.isfinite(Vg).all()),
        "weights_col_sum": W.astype(np.float64).sum(axis=0).tolist(),
    }
    mesh.dump_json(os.path.join(out, f"{letter}_teeth.json"), stages.J({
        "arch": arch, "sequence": [int(f) for f in seq], "teeth": [tooth_record(teeth[f]) for f in seq],
        "closed": closed, "gum": gum, "motions": per_motion,
        "seconds": {k: round(v, 2) for k, v in sec.items()},
        "provenance": {"plugin_rev": subprocess.run(["git", "-C", plugin.PLUGIN_ROOT, "rev-parse", "HEAD"],
                                                    capture_output=True, text=True).stdout.strip(),
                       "numpy": np.__version__},
    }))
    bad = [f for f, c in closed.items() if not c["finite"]]
    print(f"scan {letter}: {len(seq)} teeth, compute_teeth {sec['compute_teeth']:.0f} s, closed teeth "
          f"{sec['closed_teeth_wall']:.0f} s ({jobs} jobs), gum surface {sec['gum_surface']:.0f} s, weights "
          f"{sec['gum_weights']:.0f} s; non-finite closed teeth: {bad or 'none'}", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(prog="gen_teeth_real.py", description=__doc__.split("\n\n")[0])
    ap.add_argument("out", help="the folder of gen_segmentation_real.py, outside the repository")
    ap.add_argument("--only", default="abcd", help="scan letters, e.g. a,c (default: all four)")
    ap.add_argument("--plugin", default=plugin.PLUGIN_ROOT, help="plugin repository root (read only)")
    ap.add_argument("--jobs", type=int, default=4, help="processes for the closed teeth of a scan")
    args = ap.parse_args()
    plugin.PLUGIN_ROOT = os.path.abspath(args.plugin)
    plugin.install(plugin.PLUGIN_ROOT)
    scans = os.environ["DC_REAL_SCANS"]
    files = sorted(f for f in os.listdir(scans) if f.lower().endswith(".stl"))
    assert len(files) == 4, "the folder must hold the four scans"
    for letter, name in zip("abcd", files):
        if letter in args.only:
            run(letter, os.path.join(scans, name), args.out, args.jobs)


if __name__ == "__main__":
    main()
