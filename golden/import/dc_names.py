import json, os, subprocess, re
from paths import CASES, CLI, FX, OUT
cases = json.load(open(CASES))
filler = os.path.join(FX, 'E', 'a_upper.stl'); fillerL = os.path.join(FX, 'E', 'c_lower.stl')
def go(extra, path):
    cmds = ['--cmd', 'project.new={"patient_code":"X"}']
    for p, role in extra: cmds += ['--cmd', 'case.import=' + json.dumps({'path': p, 'role': role})]
    cmds += ['--cmd', 'case.import=' + json.dumps({'path': path})]
    r = subprocess.run([CLI, 'run'] + cmds, capture_output=True, text=True)
    if r.returncode: return 'ERR'
    return json.loads(r.stdout)['results'][-1]['output']['role']
out = {}
for c in cases:
    if c['group'] != 'C': continue
    for rel in c['files']:
        p = os.path.join(FX, rel)
        a = go([], p); b = go([(filler, 'upper'), (fillerL, 'lower')], p)
        hint = None if (a == 'upper' and b == 'bite') else b
        out[rel] = {'role_empty': a, 'role_full': b, 'hint': hint}
json.dump(out, open(os.path.join(OUT, 'dc_names.json'), 'w'), indent=1)
print('done', len(out))
