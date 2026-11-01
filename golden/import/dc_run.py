"""Run every case through dentalcraft-cli (session path: project.new + case.import)."""
import json, os, subprocess, sys, re
from concurrent.futures import ThreadPoolExecutor
from paths import CASES, CLI, FX, OUT
# command that imports each file: the core `case.import` (default) or the Aligners
# `aligners.case.import-scans`, which also recognises the arch from the geometry
IMPORT_CMD = os.environ.get('DC_IMPORT_CMD') or 'case.import'
os.makedirs(os.path.join(OUT, 'dc'), exist_ok=True)
cases = json.load(open(CASES))
only = sys.argv[1:]

def safe(cid): return re.sub(r'[^A-Za-z0-9._-]', '_', cid)

def run_case(c):
    ok_files = []; steps = []; final = None
    for rel in c['files']:
        path = os.path.join(FX, rel)
        cmds = ['--cmd', 'project.new={"patient_code":"X"}']
        for p in ok_files + [path]:
            cmds += ['--cmd', IMPORT_CMD + '=' + json.dumps({'path': p})]
        out = os.path.join(OUT, 'dc', safe(c['id']) + '.dcprj')
        if os.path.exists(out): os.remove(out)
        r = subprocess.run([CLI, 'run'] + cmds + ['--save', out, '--print'], capture_output=True, text=True, timeout=600)
        if r.returncode == 0:
            j = json.loads(r.stdout)
            steps.append({'file': rel, 'ok': True, 'output': j['results'][-1]['output']})
            ok_files.append(path); final = j['case']
        else:
            steps.append({'file': rel, 'ok': False, 'exit': r.returncode, 'stderr': r.stderr.strip().replace(path, os.path.basename(path))[:600]})
    res = {'id': c['id'], 'group': c['group'], 'steps': steps,
           'scans': (final or {}).get('scans', []), 'world_frame': (final or {}).get('case', {}).get('world_frame')}
    json.dump(res, open(os.path.join(OUT, 'dc', safe(c['id']) + '.json'), 'w'), indent=1)
    return res

todo = [c for c in cases if not only or c['group'] in only or c['id'] in only]
with ThreadPoolExecutor(4) as ex:
    allres = list(ex.map(run_case, todo))
json.dump(allres, open(os.path.join(OUT, 'dc_all.json' if not only else 'dc_part.json'), 'w'), indent=1)
print(len(allres), 'done')
