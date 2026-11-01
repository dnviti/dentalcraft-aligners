import sys, os, json
sys.dont_write_bytecode = True
SP = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SP)
from paths import CASES, OUT, PLUGIN
sys.path.insert(0, PLUGIN)
import bpy
import ortho_aligner
ortho_aligner.register()
from ortho_aligner import autoseed
cases = json.load(open(CASES))
out = {}
for c in cases:
    if c['group'] != 'C': continue
    for rel in c['files']:
        b = os.path.basename(rel)
        out[rel] = {'basename': b, 'arch_from_name(basename)': autoseed.arch_from_name(b), 'arch_from_name(full_path)': autoseed.arch_from_name(rel)}
json.dump(out, open(os.path.join(OUT, 'pl_names.json'), 'w'), indent=1)
print('NAMES_DONE', len(out))
