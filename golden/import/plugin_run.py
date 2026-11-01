"""Runs inside Blender (background). Usage: blender -b --factory-startup -P plugin_run.py -- MODE [filter...]
MODE = op (real operator bpy.ops.oa.import_scans) | direct (_import_mesh, fix_units, arch_from_name, create_case)
"""
import sys, os, json, re, time, traceback
sys.dont_write_bytecode = True
SP = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SP)
from paths import CASES, FX, OUT, PLUGIN
sys.path.insert(0, PLUGIN)
import bpy, numpy as np
import ortho_aligner
ortho_aligner.register()
from ortho_aligner import ops_import, ops_common, case, autoseed, worldframe

argv = sys.argv[sys.argv.index('--') + 1:]
MODE = argv[0]; only = argv[1:]
cases = json.load(open(CASES))
os.makedirs(os.path.join(OUT, 'pl_' + MODE), exist_ok=True)

REPORTS = []
_orig_execute = ops_import.OA_OT_import_scans.execute
class _Proxy:
    def __init__(self, real): object.__setattr__(self, '_real', real)
    def __getattr__(self, k): return getattr(object.__getattribute__(self, '_real'), k)
    def report(self, t, m):
        REPORTS.append([sorted(t)[0] if isinstance(t, set) else str(t), m])
        return object.__getattribute__(self, '_real').report(t, m)
def _execute(self, context):
    return _orig_execute(_Proxy(self), context)
ops_import.OA_OT_import_scans.execute = _execute

def safe(cid): return re.sub(r'[^A-Za-z0-9._-]', '_', cid)

def reset():
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    for m in list(bpy.data.meshes):
        bpy.data.meshes.remove(m)
    sc = bpy.context.scene
    if worldframe.KEY in sc:
        del sc[worldframe.KEY]
    # default cube of the factory scene is gone with the loop above (so hide_default_scene_objects has nothing to do)

def describe(o):
    me = o.data
    nv = len(me.vertices); npoly = len(me.polygons)
    V = np.empty(nv * 3, np.float32); me.vertices.foreach_get('co', V); V = V.reshape(-1, 3)
    loops = np.empty(npoly, np.int32); me.polygons.foreach_get('loop_total', loops)
    M = np.array(o.matrix_world, dtype=np.float64)
    W = V.astype(np.float64) @ M[:3, :3].T + M[:3, 3] if nv else V
    used = np.zeros(nv, bool)
    li = np.empty(len(me.loops), np.int32); me.loops.foreach_get('vertex_index', li); used[li] = True
    d = {'name': o.name, 'type': o.type, 'verts': nv, 'polys': npoly, 'tris': int((loops - 2).sum()) if npoly else 0,
         'loose_verts': int((~used).sum()),
         'local_min': V.min(0).tolist() if nv else None, 'local_max': V.max(0).tolist() if nv else None,
         'world_min': W.min(0).tolist() if nv else None, 'world_max': W.max(0).tolist() if nv else None,
         'centroid_local': V.astype(np.float64).mean(0).tolist() if nv else None,
         'dimensions': list(o.dimensions), 'matrix_world_is_identity': bool(np.allclose(M, np.eye(4))),
         'scale': list(o.scale), 'is_case': bool(o.oa.is_case), 'arch': o.oa.arch if o.oa.is_case else None,
         'oa_source': o.get('oa_source')}
    # degenerate / duplicate polygon census (by vertex-index sets)
    if npoly:
        pv = []
        for p in me.polygons: pv.append(tuple(p.vertices))
        d['polys_with_repeated_vert'] = sum(1 for t in pv if len(set(t)) < len(t))
        keys = [tuple(sorted(t)) for t in pv]
        d['polys_dup_by_vertexset'] = len(keys) - len(set(keys))
    return d

def run_op(path):
    rec = {'file': os.path.relpath(path, FX)}
    REPORTS.clear()
    before = set(o.name for o in bpy.data.objects)
    t0 = time.perf_counter()
    try:
        r = bpy.ops.oa.import_scans(filepath=path)
        rec['result'] = sorted(r)[0] if isinstance(r, set) else str(r)
    except Exception as e:
        rec['result'] = 'EXC'; rec['error'] = str(e).strip()[-3000:]
    rec['sec'] = round(time.perf_counter() - t0, 3)
    rec['reports'] = [list(x) for x in REPORTS]
    rec['new_objects'] = sorted(set(o.name for o in bpy.data.objects) - before)
    return rec

def run_direct(path):
    rec = {'file': os.path.relpath(path, FX)}
    class Op:
        def report(self, t, m): REPORTS.append([sorted(t)[0], m])
    REPORTS.clear()
    before = set(o.name for o in bpy.data.objects)
    try:
        obj = ops_common._import_mesh(Op(), path)
        if obj is None:
            rec['result'] = 'NONE'
        else:
            src = os.path.basename(path)
            f = case.fix_units(obj)
            arch = autoseed.arch_from_name(src) or 'LOWER'
            warn = case.create_case(obj, arch)
            obj['oa_source'] = src
            rec['result'] = 'OK'; rec['factor'] = f; rec['warn'] = warn
    except Exception as e:
        rec['result'] = 'EXC'; rec['error'] = (type(e).__name__ + ': ' + str(e)).strip()[:500]
    rec['reports'] = [list(x) for x in REPORTS]
    rec['new_objects'] = sorted(set(o.name for o in bpy.data.objects) - before)
    return rec

runner = run_op if MODE == 'op' else run_direct
results = []
for c in cases:
    if only and not (c['group'] in only or c['id'] in only):
        continue
    reset()
    steps = []
    for rel in c['files']:
        steps.append(runner(os.path.join(FX, rel)))
    objs = [describe(o) for o in bpy.data.objects if o.type == 'MESH']
    if c['id'].startswith('R-neutral'):
        from ortho_aligner import meshutil as mu
        for o in bpy.data.objects:
            if o.type == 'MESH' and o.oa.is_case:
                t0 = time.perf_counter()
                try:
                    Vw = case.model_world_verts(o); T = mu.get_tris(o.data)
                    a, conf, det = autoseed.detect_arch(Vw, T, name=o.get('oa_source'))
                    dd = {'arch': a, 'confidence': float(conf), 'details': {k: (v if isinstance(v, (str, int, float, type(None))) else str(v)) for k, v in det.items()}}
                except Exception as e:
                    dd = {'error': type(e).__name__ + ': ' + str(e)[:300]}
                dd['sec'] = round(time.perf_counter() - t0, 3)
                for d_ in objs:
                    if d_['name'] == o.name: d_['detect_arch'] = dd
    res = {'id': c['id'], 'group': c['group'], 'steps': steps, 'objects': objs}
    results.append(res)
    json.dump(res, open(os.path.join(OUT, 'pl_' + MODE, safe(c['id']) + '.json'), 'w'), indent=1)
json.dump(results, open(os.path.join(OUT, 'pl_%s_%s.json' % (MODE, 'part' if only else 'all')), 'w'), indent=1)
print('PLUGIN_RUN_DONE', len(results))
