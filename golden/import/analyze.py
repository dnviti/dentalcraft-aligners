import json, os, re
from paths import CASES, OUT
cases = json.load(open(CASES))
PL = {x['id']: x for x in json.load(open(f'{OUT}/pl_op_all.json'))}
DC = {x['id']: x for x in json.load(open(f'{OUT}/dc_all.json'))}
PN = json.load(open(f'{OUT}/pl_names.json')); DN = json.load(open(f'{OUT}/dc_names.json'))
UNIT_F = {'m': 1000.0, 'cm': 10.0, 'mm': 1.0, 'um': 0.001}

def r4(v):
    if v is None: return 'none'
    return '(' + ','.join(('%.4f' % x).rstrip('0').rstrip('.') if abs(x) < 1e4 else '%.2f' % x for x in v) + ')'

def pl_factor(step):
    for t, m in step['reports']:
        mm = re.search(r'riscalato x([0-9.e+-]+)', m)
        if mm: return float(mm.group(1))
    return 1.0

def pl_obj_for_step(pl, i):
    names = [n for n in pl['steps'][i]['new_objects'] if not n.startswith('OA_')]
    objs = {o['name']: o for o in pl['objects']}
    return [objs[n] for n in names if n in objs]

def short_err(e):
    lines = [l for l in e.strip().splitlines() if l.strip()]
    pick = [l for l in lines if l.startswith('RuntimeError:')]
    t = (pick[0] if pick else lines[-1] if lines else '')
    t = re.sub(r"'[^']*/fixtures/[^']*/([^'/]*)'", r"'\1'", t)
    t = t.replace('RuntimeError: Error: ', 'uncaught RuntimeError: ')
    if pick and 'Traceback' in e: t += ' (python traceback to the user)'
    return t

def plugin_step(pl, i):
    s = pl['steps'][i]
    objs = pl_obj_for_step(pl, i)
    msgs = [f'{t}: {m}' for t, m in s['reports'] if not m.startswith('Importate:')]
    if s['result'] != 'FINISHED' or not objs:
        err = short_err(s['error']) if s.get('error') else '; '.join(m for t, m in s['reports'] if t == 'ERROR')
        return {'ok': False, 'text': f"{s['result']}" + (f" [{err[:220]}]" if err else '') + (f" objs={len(objs)}" if objs else ''), 'objs': objs, 'msgs': msgs}
    o = objs[0]
    t = f"v={o['verts']} f={o['polys']}" + (f"(tri {o['tris']})" if o['tris'] != o['polys'] else '') + f" wbb={r4(o['world_min'])}..{r4(o['world_max'])} x{pl_factor(s):g} {o['arch']}"
    if len(objs) > 1: t += f' +{len(objs)-1} extra objs'
    if msgs: t += ' | ' + ' | '.join(msgs)
    return {'ok': True, 'text': t, 'o': o, 'objs': objs, 'f': pl_factor(s), 'msgs': msgs}

def dc_step(dc, i):
    s = dc['steps'][i]
    if not s['ok']:
        e = s['stderr']; e = re.sub(r'^error: --cmd \d+ \(case.import\): case.import: failed: ', '', e)
        return {'ok': False, 'text': f'ERR [{e[:170]}]'}
    o = s['output']
    sc = [x for x in dc['scans'] if x['id'] == o['scan_id']]
    t = f"v={o['vertices']} f={o['faces']}"
    if sc:
        a = sc[0]['aabb']
        t += (f" wbb={r4(a['min'])}..{r4(a['max'])}" if a else ' wbb=none(empty)') + f" x{UNIT_F[o['source_units']]:g} {o['role']}"
        d = {'ok': True, 'text': t, 'o': o, 'aabb': a, 'f': UNIT_F[o['source_units']], 'role': o['role']}
    else:
        d = {'ok': True, 'text': t + f" (scan replaced later) {o['role']}", 'o': o, 'role': o['role'], 'aabb': None, 'f': UNIT_F[o['source_units']]}
    if o.get('replaced'): d['text'] += ' REPLACED-existing'
    return d

rows = []
def close(a, b, ext):
    tol = 1e-4 + 2e-5 * ext
    return all(abs(x - y) <= tol for x, y in zip(a, b))

for c in cases:
    cid, g = c['id'], c['group']
    pl, dc = PL[cid], DC[cid]
    n = len(c['files'])
    pt, dt = [], []; diffs = []
    steps = []
    for i in range(n):
        p = plugin_step(pl, i); d = dc_step(dc, i)
        steps.append((p, d))
        pt.append(p['text']); dt.append(d['text'])
        if p['ok'] != d['ok']:
            diffs.append(f"step{i+1}: plugin {'accepts' if p['ok'] else 'rejects'}, DC {'accepts' if d['ok'] else 'rejects'}")
            continue
        if not p['ok']: continue
        o = p['o']; do = d['o']
        if o['verts'] != do['vertices']: diffs.append(f"step{i+1}: verts {o['verts']} vs {do['vertices']}")
        if o['tris'] != do['faces']: diffs.append(f"step{i+1}: tris {o['tris']} vs {do['faces']}")
        if d['aabb'] is not None and o['world_min'] is not None:
            ext = max(max(o['world_max'][k] - o['world_min'][k] for k in range(3)), 1)
            if not close(o['world_min'], d['aabb']['min'], ext) or not close(o['world_max'], d['aabb']['max'], ext):
                mn, mx = d['aabb']['min'], d['aabb']['max']
                if c['files'][i].lower().endswith('.obj') and close(o['world_min'], [mn[0], -mx[2], mn[1]], ext) and close(o['world_max'], [mx[0], -mn[2], mx[1]], ext):
                    diffs.append(f"step{i+1}: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox)")
                else:
                    diffs.append(f"step{i+1}: world bbox differs")
            if abs(p['f'] - d['f']) > 1e-9: diffs.append(f"step{i+1}: unit factor {p['f']:g} vs {d['f']:g}")
        want = {'UPPER': 'upper', 'LOWER': 'lower'}[o['arch']]
        if g in ('C', 'E', 'R') and want != d['role']: diffs.append(f"step{i+1}: role/arch {o['arch']} vs {d['role']}")
        if len(p['objs']) > 1: diffs.append(f"step{i+1}: plugin leaves {len(p['objs'])-1} extra unprocessed mesh object(s)")
        if bool(p['msgs']) != False and g == 'B':
            pass
    if g in ('E',):
        pcount = len([o for o in pl['objects'] if o['is_case']]); dcount = len(dc['scans'])
        if pcount != dcount: diffs.append(f"final case holds {pcount} scan objects (plugin) vs {dcount} scans (DC)")
    rows.append({'id': cid, 'group': g, 'plugin': pt, 'dc': dt, 'diffs': diffs, 'same': not diffs, 'note': c['note']})
json.dump(rows, open(f'{OUT}/rows.json', 'w'), indent=1)
# compact table for expected.json: one line per case, real-scan rows left out
expected = [{k: r[k] for k in ('id', 'group', 'note', 'plugin', 'dc', 'same', 'diffs')} for r in rows if r['group'] != 'R']
with open(f'{OUT}/expected.json', 'w') as fh:
    fh.write('[\n' + ',\n'.join(json.dumps(x, ensure_ascii=False) for x in expected) + '\n]\n')
for r in rows:
    if not r['same']: print(r['id'], '|', r['diffs'])
print(sum(r['same'] for r in rows), 'same of', len(rows))
