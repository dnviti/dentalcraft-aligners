"""Add the real scans as group R, read in place. Run after gen.py.

Usage: python3 add_real.py DATASET_DIR

The first four *.stl files of DATASET_DIR, sorted by file name, are used. Nothing is
copied, and no file name is written to a committed file: the scans are exposed as
scan_a.stl .. scan_d.stl symlinks in the work folder, and the neutral cases use those.
"""
import glob
import json
import os
import sys

from paths import CASES, FX

if len(sys.argv) != 2:
    sys.exit(__doc__.strip())
src = os.path.abspath(sys.argv[1])
found = sorted(glob.glob(os.path.join(src, '*.stl')))[:4]
if not found:
    sys.exit(f'no .stl files in {src}')

cases = [c for c in json.load(open(CASES)) if c['group'] != 'R']
os.makedirs(os.path.join(FX, 'R'), exist_ok=True)
for i, path in enumerate(found):
    cases.append({'id': f'R-orig-{i + 1}', 'group': 'R', 'files': [path],
                  'note': 'real scan, original name (read in place)'})
for i, path in enumerate(found):
    neutral = f'scan_{chr(ord("a") + i)}.stl'
    link = os.path.join(FX, 'R', neutral)
    if os.path.lexists(link):
        os.remove(link)
    os.symlink(path, link)
    cases.append({'id': f'R-neutral-{neutral}', 'group': 'R', 'files': [f'R/{neutral}'],
                  'note': f'symlink {neutral} -> real scan {i + 1}'})
json.dump(cases, open(CASES, 'w'), indent=1)
print(len(found), 'real scans;', len(cases), 'cases')
