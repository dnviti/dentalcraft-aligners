#!/usr/bin/env python3
"""Tier A of the golden harness: the Blender add-on's pure-numpy pipeline as an oracle.

    python3 -B golden/oracle.py <stage> --in <case dir | meta.json | mesh> --out <dir>
    python3 -B golden/oracle.py all --in golden/fixtures/demo_lower_mild
    python3 -B golden/oracle.py list

Stages (in pipeline order): archform worldframe autoseed segmentation teeth diagnosis setup
staging schedule.  `all` runs them in order, each writing to `<case dir>/<stage>/`.

`--in` is a fixture case directory made by Tier B (`scan.dcm`, `decimated.dcm`, `meta.json`),
or a bare mesh (.dcm/.ply/.stl; then `--arch` is required).  `--out` is the directory the stage
writes to (default `<case dir>/<stage>`; with `all`, the parent of the stage folders).  Stages
that need an earlier stage compute it in memory (and do not write it).

Needs only python3 + numpy.  The plugin repository is only read
(`--plugin`, default $OA_PLUGIN or ~/Documents/repos/blender-ortho-aligner); run with `-B`.
See golden/README.md for the file formats.
"""
import argparse
import os
import sys
import time

sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from oracle import plugin, stages  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(prog="oracle.py", description=__doc__.split("\n\n")[0])
    ap.add_argument("stage", help="a stage name, 'all' or 'list'")
    ap.add_argument("--in", dest="src", help="case directory, meta.json or mesh file (.dcm .ply .stl)")
    ap.add_argument("--out", help="output directory (default: <case dir>/<stage>)")
    ap.add_argument("--arch", choices=["UPPER", "LOWER"], help="arch of a bare mesh without meta.json")
    ap.add_argument("--seeds", help="JSON {fdi: [x, y, z]} world seeds (default: ground truth, else autoseed)")
    ap.add_argument("--decimated", help="decimated scan in arch coordinates (default: decimated.dcm next to the scan)")
    ap.add_argument("--plugin", default=plugin.PLUGIN_ROOT, help="plugin repository root (read only)")
    ap.add_argument("--reuse", action="store_true",
                    help="read the segmentation labels back from <case dir>/segmentation/ instead of recomputing them")
    ap.add_argument("--no-closed-teeth", dest="closed_teeth", action="store_false",
                    help="staging: do not write the closed tooth meshes")
    a = ap.parse_args(argv)
    if a.stage == "list":
        print("\n".join(stages.ORDER))
        return 0
    if a.stage != "all" and a.stage not in stages.STAGES:
        ap.error(f"unknown stage {a.stage!r}; one of: {', '.join(stages.ORDER)}, all, list")
    if not a.src:
        ap.error("--in is required")
    plugin.PLUGIN_ROOT = a.plugin
    plugin.install(a.plugin)
    ctx = stages.Case(a.src, a)
    todo = stages.ORDER if a.stage == "all" else [a.stage]
    failed = []
    for name in todo:
        out = os.path.join(a.out, name) if (a.out and a.stage == "all") else (a.out or os.path.join(ctx.dir, name))
        t0 = time.time()
        try:
            ctx.run(name, out)
            print(f"[oracle] {ctx.name}/{name}: ok ({time.time() - t0:.1f}s) -> {out}")
        except Exception as e:
            import traceback
            traceback.print_exc()
            print(f"[oracle] {ctx.name}/{name}: FAILED {type(e).__name__}: {e}")
            failed.append(name)
            if a.stage == "all":
                continue
            return 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
