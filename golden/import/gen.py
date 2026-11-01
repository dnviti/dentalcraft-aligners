import os, sys, json, shutil, struct, zipfile
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from meshw import *
from dcmio import read_dcm
from paths import CASES, FX, REF_DCM
shutil.rmtree(FX, ignore_errors=True)
cases = []
def d(*a):
    p = os.path.join(FX, *a); os.makedirs(os.path.dirname(p), exist_ok=True); return p
def case(cid, group, files, note=''):
    cases.append({'id': cid, 'group': group, 'files': [os.path.relpath(f, FX) for f in files], 'note': note})

# ---------------- A: formats, same mesh (real synthetic demo arch, 83172 v / 166392 f)
base_p, base_i = read_dcm(REF_DCM)
np.save(d('A', 'base_pos.npy'), base_p); np.save(d('A', 'base_idx.npy'), base_i)
f = d('A', 'meshA_stlbin.stl'); write_stl_bin(f, base_p, base_i); case('A-stl-bin', 'A', [f])
f = d('A', 'meshA_stlascii.stl'); write_stl_ascii(f, base_p, base_i); case('A-stl-ascii', 'A', [f])
f = d('A', 'meshA_obj.obj'); write_obj(f, base_p, base_i); case('A-obj', 'A', [f])
f = d('A', 'meshA_plyascii.ply'); write_ply(f, base_p, base_i, 'ascii'); case('A-ply-ascii', 'A', [f])
f = d('A', 'meshA_plyle.ply'); write_ply(f, base_p, base_i, 'le'); case('A-ply-bin-le', 'A', [f])
f = d('A', 'meshA_plybe.ply'); write_ply(f, base_p, base_i, 'be'); case('A-ply-bin-be', 'A', [f])

# ---------------- small arch used by B, C, D, E
ap, ai = make_arch()          # 11900 faces
np.save(d('S', 'arch_pos.npy'), ap); np.save(d('S', 'arch_idx.npy'), ai)

# ---------------- B: units (largest extent EXACTLY float32(X), min at 0)
B = [('0.06', 0.06), ('6', 6.0), ('60', 60.0), ('60000', 60000.0), ('2.4', 2.4), ('0.02', 0.02), ('0.2', 0.2),
     ('2.0', 2.0), ('20.0', 20.0), ('15', 15.0), ('3', 3.0),
     ('0.019', 0.019), ('0.21', 0.21), ('1.9', 1.9), ('21', 21.0), ('100', 100.0), ('19999', 19999.0),
     ('20000', 20000.0), ('200000', 200000.0), ('200001', 200001.0), ('0.0199', 0.0199), ('2.0001', 2.0001)]
for lab, X in B:
    f = d('B', f'u_{lab}.stl'); write_stl_bin(f, scale_extent(ap, X), ai); case(f'B-{lab}', 'B', [f], f'largest extent {X}')

# ---------------- C: role from name
names = ['upper.stl', 'lower.stl', 'maxilla.stl', 'mandible.stl', 'mascellare.stl', 'mandibolare.stl', 'superiore.stl',
         'inferiore.stl', 'sup.stl', 'inf.stl', 'Patient_sup.stl', 'Oberkiefer.stl', 'Unterkiefer.stl', 'UpperJaw.stl',
         'lowerJaw.stl', 'scan_upper_lower.stl', 'flower.stl', 'superman.stl', 'bite.stl', 'morso.stl', 'scan1.stl',
         'UPPER.STL', 'maxillary.stl', 'mandibular.stl', 'Arcata_Superiore.stl', 'arcata-inferiore.stl', 'UpperJaw_bite.stl',
         'inferior.stl', 'superior.stl', 'maxill.stl', 'mandib.stl', 'Lower.ply', 'case_UPPER_final.obj', 'upperlower.stl',
         'Max.stl', 'MAN.stl', 'Mandible_Upper.stl', 'p123_Maxillary_Scan.stl', 'bitescan.stl', 'occlusion.stl',
         '12_upper.stl', 'UpperJawScan.stl', 'LowerJawScan.stl', 'LOWER.STL', 'unterkiefer_oberkiefer.stl', 'superioreinferiore.stl',
         'upper-lower.stl', 'lowercase.stl', 'uppercase.stl', 'supply.stl', 'info.stl', 'insup.stl']
for n in names:
    f = d('C', n)
    ext = os.path.splitext(n)[1].lower()
    if ext == '.stl': write_stl_bin(f, ap, ai)
    elif ext == '.obj': write_obj(f, ap, ai)
    else: write_ply(f, ap, ai, 'le')
    case('C-' + n, 'C', [f])
f = d('C', 'upper_case', 'scan.stl'); write_stl_bin(f, ap, ai); case('C-dir-upper_case/scan.stl', 'C', [f], 'directory name only')
f = d('C', 'lower_dir', 'scan.stl'); write_stl_bin(f, ap, ai); case('C-dir-lower_dir/scan.stl', 'C', [f], 'directory name only')
seq = []
for n in ['scan1.stl', 'scan2.stl', 'scan3.stl', 'scan4.stl']:
    f = d('C', 'seq', n); write_stl_bin(f, ap, ai); seq.append(f)
case('C-seq-nohint-4', 'C', seq, 'no-hint default: 4 files into one case')
# hinted second file after a no-hint first
f1 = d('C', 'seq2', 'scan1.stl'); write_stl_bin(f1, ap, ai)
f2 = d('C', 'seq2', 'x_upper.stl'); write_stl_bin(f2, ap, ai)
f3 = d('C', 'seq2', 'scan2.stl'); write_stl_bin(f3, ap, ai)
case('C-seq-nohint-then-upper-then-nohint', 'C', [f1, f2, f3], 'scan1 (no hint), x_upper, scan2 (no hint)')

# ---------------- D: robustness
def raw(name, data, cid=None, note=''):
    f = d('D', name); open(f, 'wb').write(data); case(cid or 'D-' + name, 'D', [f], note); return f
for ext in ('stl', 'obj', 'ply'):
    raw(f'empty0.{ext}', b'', note='0 bytes')
raw('tri0_bin.stl', b'synthetic'.ljust(80, b' ') + struct.pack('<I', 0), note='binary STL with 0 triangles')
raw('solid_empty.stl', b'solid x\nendsolid x\n', note='ASCII STL, no facets')
raw('noface.obj', b'# no faces\nv 0 0 0\nv 1 0 0\nv 0 1 0\n', note='OBJ with 3 vertices and no faces')
raw('empty_comment.obj', b'# nothing\n', note='OBJ with only a comment')
h = ply_header('ascii', 3, 0, ['float x', 'float y', 'float z']).encode() + b'0 0 0\n1 0 0\n0 1 0\n'
raw('face0.ply', h, note='PLY with 3 vertices and 0 faces')
h = ply_header('ascii', 0, 0, ['float x', 'float y', 'float z']).encode()
raw('vert0.ply', h, note='PLY with 0 vertices and 0 faces')
# truncated binary stl
f = d('D', '_full.stl'); write_stl_bin(f, ap, ai); full = open(f, 'rb').read(); os.remove(f)
raw('trunc_mid.stl', full[:84 + 50*5000 + 25], note='binary STL cut mid-triangle (misaligned), header says 11900')
raw('trunc_aligned.stl', full[:84 + 50*5000], note='binary STL cut on a triangle boundary: 5000 of 11900 declared')
raw('trunc_header.stl', full[:60], note='binary STL cut inside the 80 byte header')
raw('garbage.stl', np.random.default_rng(1).integers(0, 256, 4096, dtype=np.uint8).tobytes(), note='random bytes')
raw('garbage_text.stl', b'this is not a mesh\nhello world\n' * 20, note='text garbage .stl')
raw('garbage.obj', np.random.default_rng(2).integers(0, 256, 4096, dtype=np.uint8).tobytes(), note='random bytes .obj')
raw('garbage.ply', np.random.default_rng(3).integers(0, 256, 4096, dtype=np.uint8).tobytes(), note='random bytes .ply')
f = d('D', 'valid_as_txt.txt'); write_stl_bin(f, ap, ai); case('D-valid_as_txt.txt', 'D', [f], 'valid binary STL with .txt extension')
f = d('D', 'model.3mf')
with zipfile.ZipFile(f, 'w') as z:
    z.writestr('[Content_Types].xml', '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>')
    z.writestr('_rels/.rels', '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>')
    z.writestr('3D/3dmodel.model', '<?xml version="1.0"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="40" y="0" z="0"/><vertex x="0" y="40" z="0"/><vertex x="0" y="0" z="40"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="2" v3="3"/><triangle v1="1" v2="2" v3="3"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>')
case('D-model.3mf', 'D', [f], 'valid minimal 3MF (tetrahedron)')
f = d('D', 'noext'); write_stl_bin(f, ap, ai); case('D-noext', 'D', [f], 'valid binary STL, no extension')
# small mesh < 1000 faces
sp, si = make_arch(nu=20, nv=10)  # 380 faces
f = d('D', 'small380.stl'); write_stl_bin(f, sp, si); case('D-small380.stl', 'D', [f], f'{len(si)} faces')
f = d('D', 'small380.obj'); write_obj(f, sp, si); case('D-small380.obj', 'D', [f], f'{len(si)} faces')
# degenerate + duplicate
dp, di = make_arch(nu=60, nv=40)   # 4720 faces
nvv = len(dp)
extra_v = np.stack([np.linspace(0, 10, 20), np.zeros(20), np.zeros(20)], -1).astype(np.float32) + np.float32(100)
dp2 = np.vstack([dp, extra_v]).astype(np.float32)
rep = np.array([[i, i, i+1] for i in range(0, 50)], np.uint32)          # repeated index
col = np.array([[nvv+i, nvv+i+1, nvv+i+2] for i in range(0, 18)], np.uint32)  # collinear
di_deg = np.vstack([di, rep, col])
for ext, wr in (('stl', lambda p: write_stl_bin(p, dp2, di_deg)), ('obj', lambda p: write_obj(p, dp2, di_deg)),
                ('ply', lambda p: write_ply(p, dp2, di_deg, 'le'))):
    f = d('D', f'degen.{ext}'); wr(f); case(f'D-degen.{ext}', 'D', [f], f'{len(di)} good + 50 repeated-index + 18 collinear faces; {len(dp2)} verts (20 isolated/collinear)')
di_dup = np.vstack([di, di[:200], di[200:300][:, ::-1]])
for ext, wr in (('stl', lambda p: write_stl_bin(p, dp, di_dup)), ('obj', lambda p: write_obj(p, dp, di_dup)),
                ('ply', lambda p: write_ply(p, dp, di_dup, 'le'))):
    f = d('D', f'dupface.{ext}'); wr(f); case(f'D-dupface.{ext}', 'D', [f], f'{len(di)} faces + 200 identical duplicates + 100 reversed duplicates')
# near-weld STL: shared vertices differ by ~2e-5 mm per triangle copy
rng = np.random.default_rng(5)
t = ap[ai].copy()
t += (rng.uniform(-1, 1, t.shape) * 2e-5).astype(np.float32)
tp = t.reshape(-1, 3); ti = np.arange(len(tp), dtype=np.uint32).reshape(-1, 3)
f = d('D', 'jitter2e5.stl'); write_stl_bin(f, tp, ti); case('D-jitter2e5.stl', 'D', [f], 'unwelded STL, shared corners jittered by up to 2e-5 mm')
t = ap[ai].copy(); t += (rng.uniform(-1, 1, t.shape) * 1e-3).astype(np.float32)
tp = t.reshape(-1, 3)
f = d('D', 'jitter1e3.stl'); write_stl_bin(f, tp, ti); case('D-jitter1e3.stl', 'D', [f], 'unwelded STL, shared corners jittered by up to 1e-3 mm')
# multi-object OBJ
p1, i1 = make_arch(nu=60, nv=30); p2 = (p1 + np.array([100, 0, 0], np.float32)).astype(np.float32)
f = d('D', 'multiobj.obj')
with open(f, 'w') as fh:
    fh.write('o a\n')
    for p in p1: fh.write('v %.9g %.9g %.9g\n' % tuple(p))
    for t in i1: fh.write('f %d %d %d\n' % tuple(t + 1))
    fh.write('o b\n')
    for p in p2: fh.write('v %.9g %.9g %.9g\n' % tuple(p))
    for t in i1: fh.write('f %d %d %d\n' % tuple(t + 1 + len(p1)))
case('D-multiobj.obj', 'D', [f], f'o a ({len(p1)}v,{len(i1)}f) + o b shifted +100 in x ({len(p2)}v,{len(i1)}f)')
# PLY colours + normals
nrm = np.zeros_like(ap); nrm[:, 2] = 1
col = (np.arange(len(ap))[:, None] * np.array([3, 5, 7]) % 256).astype(np.uint8)
for fmt in ('ascii', 'le'):
    f = d('D', f'colnrm_{fmt}.ply'); write_ply(f, ap, ai, fmt, colors=col, nrm=nrm); case(f'D-colnrm_{fmt}.ply', 'D', [f], 'PLY with vertex colours (uchar) and normals')
f = d('D', 'colnrm_be.ply'); write_ply(f, ap, ai, 'be', colors=col, nrm=nrm); case('D-colnrm_be.ply', 'D', [f], 'PLY BE with colours and normals')
# uppercase / mixed extension
f = d('D', 'scanU.STL'); write_stl_bin(f, ap, ai); case('D-scanU.STL', 'D', [f], 'uppercase ext')
f = d('D', 'scanU.OBJ'); write_obj(f, ap, ai); case('D-scanU.OBJ', 'D', [f], 'uppercase ext')
f = d('D', 'scanU.Ply'); write_ply(f, ap, ai, 'le'); case('D-scanU.Ply', 'D', [f], 'mixed-case ext')
# OBJ extras: quads + vt/vn, negative indices, vertex colours
q = make_arch(nu=40, nv=30)[0]
nu_, nv_ = 40, 30
ids = np.arange(nu_*nv_).reshape(nu_, nv_)
quads = np.stack([ids[:-1, :], ids[1:, :], ids[1:, np.r_[1:nv_, 0]], ids[:-1, np.r_[1:nv_, 0]]], -1).reshape(-1, 4)
f = d('D', 'quads_vtvn.obj')
with open(f, 'w') as fh:
    for p in q: fh.write('v %.9g %.9g %.9g\n' % tuple(p))
    fh.write('vt 0 0\nvn 0 0 1\n')
    for t in quads: fh.write('f ' + ' '.join(f'{k+1}/1/1' for k in t) + '\n')
case('D-quads_vtvn.obj', 'D', [f], f'{len(q)} verts, {len(quads)} quads with v/vt/vn -> {2*len(quads)} triangles')
f = d('D', 'negidx.obj')
with open(f, 'w') as fh:
    for p in sp: fh.write('v %.9g %.9g %.9g\n' % tuple(p))
    for t in si: fh.write('f %d %d %d\n' % tuple(t.astype(np.int64) - len(sp)))
case('D-negidx.obj', 'D', [f], 'OBJ with negative (relative) indices, 380 faces')
f = d('D', 'vcol.obj')
with open(f, 'w') as fh:
    for p in ap: fh.write('v %.9g %.9g %.9g 0.5 0.25 0.75\n' % tuple(p))
    for t in ai: fh.write('f %d %d %d\n' % tuple(t + 1))
case('D-vcol.obj', 'D', [f], 'OBJ with v x y z r g b')
# NaN / inf coordinates
bad = ap.copy(); bad[10, 0] = np.nan
f = d('D', 'nan.stl'); write_stl_bin(f, bad, ai); case('D-nan.stl', 'D', [f], 'one NaN coordinate')
bad = ap.copy(); bad[10, 0] = np.inf
f = d('D', 'inf.obj'); write_obj(f, bad, ai); case('D-inf.obj', 'D', [f], 'one inf coordinate (OBJ text "inf")')
# OBJ with out-of-range index
f = d('D', 'badidx.obj'); open(f, 'w').write('v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n'); case('D-badidx.obj', 'D', [f], 'face index out of range')
# binary STL starting with "solid" (common exporter header)
f = d('D', 'solidhdr.stl'); write_stl_bin(f, ap, ai, header=b'solid exported by scanner'); case('D-solidhdr.stl', 'D', [f], 'binary STL whose header starts with solid')
# CRLF ASCII STL
f = d('D', 'crlf.stl'); write_stl_ascii(f, sp, si); s = open(f, 'rb').read().replace(b'\n', b'\r\n'); open(f, 'wb').write(s); case('D-crlf.stl', 'D', [f], 'ASCII STL, CRLF, 380 faces')

# ---------------- E: multi-file and re-import
pa, ia = make_arch(offset=(0, 0, 0)); pb, ib = make_arch(nu=80, nv=40, offset=(10, 10, 10))
fa = d('E', 'a_upper.stl'); write_stl_bin(fa, pa, ia)
fb = d('E', 'b_upper.stl'); write_stl_bin(fb, pb, ib)
case('E-same-role-twice', 'E', [fa, fb], 'two different meshes, both named upper: a (11900f) then b (6320f)')
case('E-same-file-twice', 'E', [fa, fa], 'same file imported twice')
fl = d('E', 'c_lower.stl'); write_stl_bin(fl, pb, ib)
case('E-upper-lower-pair', 'E', [fa, fl], 'upper then lower, different shapes/offsets')
case('E-upper-lower-upper', 'E', [fa, fl, fb], 'upper, lower, upper again')

fe = d('E', 'upper_empty.obj'); open(fe, 'w').write('# nothing\n')
fg = d('E', 'upper_garbage.obj'); open(fg, 'wb').write(np.random.default_rng(2).integers(0, 256, 4096, dtype=np.uint8).tobytes())
fgs = d('E', 'upper_garbage.stl'); open(fgs, 'wb').write(np.random.default_rng(1).integers(0, 256, 4096, dtype=np.uint8).tobytes())
ft = d('E', 'upper_trunc.stl'); open(ft, 'wb').write(full[:84 + 50*5000 + 25])
case('E-good-upper-then-empty-upper-obj', 'E', [fa, fe], 'valid upper, then comment-only OBJ named upper')
case('E-good-upper-then-garbage-upper-obj', 'E', [fa, fg], 'valid upper, then random bytes named upper_garbage.obj')
case('E-good-upper-then-garbage-upper-stl', 'E', [fa, fgs], 'valid upper, then random bytes named upper_garbage.stl')
case('E-good-upper-then-truncated-upper-stl', 'E', [fa, ft], 'valid upper, then truncated binary STL named upper_trunc.stl')

json.dump(cases, open(CASES, 'w'), indent=1)
print(len(cases), 'cases')
