#!/usr/bin/env python3
"""Sanity checks of the committed fixtures: size budget, DCM1 files decode, JSON parses,
stage outputs present.  `python3 -B golden/check.py golden/fixtures` (exit 1 on a violation)."""
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from oracle import mesh  # noqa: E402

TOTAL_MB, FILE_MB = 40.0, 4.0
STAGE_FILES = {"archform": "archform.json", "worldframe": "worldframe.json", "autoseed": "autoseed.json",
               "segmentation": "segmentation.json", "teeth": "teeth.json", "diagnosis": "diagnosis.json",
               "setup": "setup.json", "staging": "staging.json", "schedule": "schedule.json"}


def main(root):
    bad, total = [], 0
    cases = sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)))
    for c in cases:
        have = []
        for dp, _, fs in os.walk(os.path.join(root, c)):
            for f in fs:
                p = os.path.join(dp, f)
                n = os.path.getsize(p)
                total += n
                if n > FILE_MB * 2 ** 20:
                    bad.append(f"{p}: {n / 2**20:.2f} MB > {FILE_MB} MB")
                if f.endswith(".dcm"):
                    try:
                        mesh.read_dcm(p)
                    except Exception as e:
                        bad.append(f"{p}: {e}")
                elif f.endswith(".json"):
                    try:
                        json.load(open(p))
                    except Exception as e:
                        bad.append(f"{p}: {e}")
        for stage, f in STAGE_FILES.items():
            if os.path.exists(os.path.join(root, c, stage, f)):
                have.append(stage)
        print(f"{c}: stages {','.join(have) or '-'}")
    print(f"total {total / 2**20:.1f} MB (budget {TOTAL_MB} MB)")
    if total > TOTAL_MB * 2 ** 20:
        bad.append(f"total {total / 2**20:.1f} MB > {TOTAL_MB} MB")
    for b in bad:
        print("FAIL", b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "fixtures")))
