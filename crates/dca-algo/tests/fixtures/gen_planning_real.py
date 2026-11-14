#!/usr/bin/env python3
"""Runs the plugin's planning (`teeth.compute_teeth`, the diagnosis, `setup_conservativo.conservative_setup`
and `staging.plan`) on the real scans, from the plugin's own segmentation of them, and writes what it
finds to a folder OUTSIDE the repository, for `tests/planning_real.rs` to compare the Rust port against.

    out=$(mktemp -d /tmp/dc-planreal-XXXX)
    DC_REAL_SCANS=/path/to/scans python3 -B gen_segmentation_real.py "$out" --only c   # one per scan
    DC_REAL_SCANS=/path/to/scans python3 -B gen_planning_real.py "$out" --only c       # one per scan
    DC_REAL_SCANS=... DC_PLANNING_REAL_ORACLE="$out" cargo test --release -p dca-algo \
        --test planning_real -- --nocapture --test-threads=1

The tests are not ignored: they run with the rest and skip themselves (a line on stderr) when
either variable is missing.

The scans are patient data: nothing this script writes (teeth, poses, closed teeth, the plan) may be
committed; delete the folder after the run.

The step is isolated: its inputs are the output of `gen_segmentation_real.py` in the same folder
(`<letter>.json`: the plugin's arch frame and final vertex labels). Per scan (`a` to `d`, as
`gen_segmentation_real.py`) it reads the STL again (same welding, checked with the fingerprint), moves
it to arch coordinates with the plugin's frame and runs the golden oracle's own stage functions
(`golden/oracle/stages.py`: `stage_teeth`, `stage_diagnosis`, `stage_setup`, `stage_staging`, the
plugin's glue `case_plan.run_setup` and `run_staging` with the default scene settings: nothing locked or
left out of the aligner, no drawn line, no opposing arch, pattern AUTO). It writes the folder
`<letter>/` with the files of a golden case: `teeth.json`, `diagnosis.json`, `setup.json`,
`staging.json` and `closed_<fdi>.dcm` (the closed crowns in their rest frames), plus
`overlaps.json`: per adjacent pair the interpenetration of the crowns at rest and in the final setup
(the plugin's `staging.CollisionChecker`) and the setup's own `overlap_max`.

The plugin is loaded read-only through `golden/oracle` (CPU only, `OA_GPU=0`; `mathutils` is the exact
float64 shim). The closed teeth (`reconstruct.closed_tooth`, the slow part) run in `--jobs` spawned
processes; `stage_staging` then reads them from memory. Needs python3 + numpy only.
"""
import argparse
import json
import multiprocessing
import os
import subprocess
import sys
import time
from types import SimpleNamespace

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)

from gen_segmentation_real import ARCH, fingerprint, read_stl  # noqa: E402  (puts golden/ on the path)
from gen_teeth_real import close_init, close_one  # noqa: E402
from oracle import mesh, plugin, settings, stages  # noqa: E402


class RealCase(stages.Case):
    """A real scan as a golden case: the plugin's frame and labels of `gen_segmentation_real.py`."""

    def __init__(self, letter, path, seg):
        self.args = SimpleNamespace(closed_teeth=True)
        self.mu, self.an = plugin.load("meshutil"), plugin.load("anatomy")
        self.V, self.T = read_stl(path)
        fp, scale = fingerprint(self.V)
        assert abs(fp - seg["fingerprint"][0]) <= 1e-9 * max(scale, seg["fingerprint"][1]), \
            f"scan {letter}: not the mesh the segmentation oracle saw"
        self.meta, self.gt, self._dec, self._seeds = None, None, None, None
        self.arch = seg["arch"]
        self.name = f"real_{letter}"
        self.s = settings.defaults(start_date=stages.START_DATE)
        self.M = np.asarray(seg["frame"], np.float64)        # shadows the cached property
        lab = mesh.unpack_labels(seg["vertex_labels"]).astype(np.int64)
        self._state = {"segmentation": {"labels": lab}}


def run(letter, path, out, jobs):
    seg = json.load(open(os.path.join(out, f"{letter}.json")))
    assert seg["arch"] == ARCH[letter], f"scan {letter}: the segmentation oracle has another arch"
    ctx = RealCase(letter, path, seg)
    dst = os.path.join(out, letter)
    os.makedirs(dst, exist_ok=True)
    sec = {}
    t0 = time.time()
    teeth = ctx.run("teeth", dst)
    ctx.run("diagnosis", dst)
    su = ctx.run("setup", dst)
    sec["teeth_diagnosis_setup"] = time.time() - t0

    # the closed teeth in spawned workers, then stage_staging reads them from memory
    seq, mu = teeth["seq"], ctx.mu
    E = mu.unique_edges(ctx.T)
    args = (ctx.V_arch, ctx.T, teeth["labels"], E, {f: np.asarray(teeth["teeth"][f].e_ax, np.float64) for f in seq})
    t0 = time.time()
    with multiprocessing.get_context("spawn").Pool(max(1, min(jobs, len(seq))), close_init,
                                                   (plugin.PLUGIN_ROOT, args)) as pool:
        done = {f: (Vt, Tt, info) for f, Vt, Tt, info, _ in pool.map(close_one, seq, chunksize=1)}
    sec["closed_teeth_wall"] = time.time() - t0
    rc = plugin.load("reconstruct")
    closed_tooth = rc.closed_tooth
    rc.closed_tooth = lambda V, T, lab, E, f, ax: done[f]
    try:
        t0 = time.time()
        stg = ctx.run("staging", dst)
        sec["staging"] = time.time() - t0
    finally:
        rc.closed_tooth = closed_tooth

    # the overlaps of the crowns at rest and in the final setup, the plugin's metric
    st, pl = plugin.load("staging"), plugin.load("planning")
    moves = stg["moves"]
    pairs = pl.adjacent_pairs(seq)
    cc = st.CollisionChecker(moves, pairs, st.StagingParams().sample_points)
    rows = [{"a": int(a), "b": int(b), "rest": float(cc.penetration(a, b, moves[a].start(), moves[b].start())),
             "final": float(cc.penetration(a, b, moves[a].q, moves[b].q))} for a, b in cc.pairs]
    stages.dump(dst, "overlaps.json", {
        "pairs": rows, "setup_overlap_max": float(su["result"].get("overlap_max", 0.0)),
        "seconds": {k: round(v, 1) for k, v in sec.items()},
        "provenance": {"plugin_rev": subprocess.run(["git", "-C", plugin.PLUGIN_ROOT, "rev-parse", "HEAD"],
                                                    capture_output=True, text=True).stdout.strip(),
                       "numpy": np.__version__}})
    print(f"scan {letter}: {len(seq)} teeth, {stg['plan']['n_stages']} aligners, closed teeth "
          f"{sec['closed_teeth_wall']:.0f} s ({jobs} jobs)", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(prog="gen_planning_real.py", description=__doc__.split("\n\n")[0])
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
