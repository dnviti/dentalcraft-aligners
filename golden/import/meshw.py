"""Mesh writers/generators (synthetic data only). Positions float32, indices uint32."""
import struct, numpy as np

def make_arch(nu=120, nv=50, offset=(3.0, 5.0, -2.0)):
    """Open horseshoe tube, asymmetric, ~60x45x14 mm, welded grid, (nu-1)*nv*2 triangles."""
    t = np.linspace(0.12*np.pi, 0.88*np.pi, nu)
    a = np.linspace(0, 2*np.pi, nv, endpoint=False)
    T, A = np.meshgrid(t, a, indexing='ij')
    R = 28.0 + 3.0*np.sin(3*T)
    r = 4.0 + 1.0*np.cos(2*T + 0.7) + 0.5*np.sin(A*2)
    cx = R*np.cos(T)*1.15
    cy = R*np.sin(T)*0.95 + 0.4*T
    x = cx + r*np.cos(A)*np.cos(T)*0.3
    y = cy + r*np.cos(A)*np.sin(T)*0.3
    z = r*np.sin(A)*0.9 + 0.8*np.sin(T*2.0)
    pos = np.stack([x, y, z], -1).reshape(-1, 3)
    pos = (pos + np.array(offset)).astype(np.float32)
    ids = np.arange(nu*nv).reshape(nu, nv)
    a0 = ids[:-1, :]; a1 = ids[:-1, np.r_[1:nv, 0]]
    b0 = ids[1:, :];  b1 = ids[1:, np.r_[1:nv, 0]]
    f1 = np.stack([a0, b0, b1], -1).reshape(-1, 3)
    f2 = np.stack([a0, b1, a1], -1).reshape(-1, 3)
    idx = np.empty((f1.shape[0]*2, 3), np.uint32)
    idx[0::2] = f1; idx[1::2] = f2
    return pos, idx

def scale_extent(pos, target):
    """Translate min to 0, scale so the largest extent is EXACTLY float32(target)."""
    p = pos.astype(np.float64)
    p -= p.min(0)
    ax = int(np.argmax(p.max(0)))
    p = p * (float(np.float32(target)) / p[:, ax].max())
    q = p.astype(np.float32)
    q[:, ax][p[:, ax] >= p[:, ax].max() - 1e-12] = np.float32(target)
    return q

def tri_pos(pos, idx):
    return pos[idx]  # (F,3,3)

def normals(pos, idx):
    t = pos[idx].astype(np.float64)
    n = np.cross(t[:, 1]-t[:, 0], t[:, 2]-t[:, 0])
    l = np.linalg.norm(n, axis=1, keepdims=True); l[l == 0] = 1
    return (n/l).astype(np.float32)

def write_stl_bin(path, pos, idx, header=b'synthetic binary stl'):
    n = normals(pos, idx); t = pos[idx]
    rec = np.zeros(len(idx), dtype=[('n', '<f4', 3), ('v', '<f4', (3, 3)), ('a', '<u2')])
    rec['n'] = n; rec['v'] = t
    with open(path, 'wb') as f:
        f.write(header.ljust(80, b' ')[:80]); f.write(struct.pack('<I', len(idx))); f.write(rec.tobytes())

def write_stl_ascii(path, pos, idx, name='x'):
    n = normals(pos, idx); t = pos[idx]
    with open(path, 'w') as f:
        f.write(f'solid {name}\n')
        for i in range(len(idx)):
            f.write('facet normal %.9g %.9g %.9g\n outer loop\n' % tuple(n[i]))
            for k in range(3):
                f.write('  vertex %.9g %.9g %.9g\n' % tuple(t[i, k]))
            f.write(' endloop\nendfacet\n')
        f.write(f'endsolid {name}\n')

def write_obj(path, pos, idx, extra=''):
    with open(path, 'w') as f:
        f.write(extra)
        for p in pos: f.write('v %.9g %.9g %.9g\n' % tuple(p))
        for t in idx: f.write('f %d %d %d\n' % (t[0]+1, t[1]+1, t[2]+1))

def ply_header(fmt, nv, nf, props):
    h = f'ply\nformat {fmt} 1.0\ncomment synthetic\nelement vertex {nv}\n'
    for pr in props: h += f'property {pr}\n'
    h += f'element face {nf}\nproperty list uchar int vertex_indices\nend_header\n'
    return h

def write_ply(path, pos, idx, fmt='ascii', colors=None, nrm=None):
    props = ['float x', 'float y', 'float z']
    if nrm is not None: props += ['float nx', 'float ny', 'float nz']
    if colors is not None: props += ['uchar red', 'uchar green', 'uchar blue']
    h = ply_header({'ascii': 'ascii', 'le': 'binary_little_endian', 'be': 'binary_big_endian'}[fmt], len(pos), len(idx), props)
    if fmt == 'ascii':
        with open(path, 'w') as f:
            f.write(h)
            for i, p in enumerate(pos):
                s = '%.9g %.9g %.9g' % tuple(p)
                if nrm is not None: s += ' %.9g %.9g %.9g' % tuple(nrm[i])
                if colors is not None: s += ' %d %d %d' % tuple(colors[i])
                f.write(s + '\n')
            for t in idx: f.write('3 %d %d %d\n' % tuple(t))
    else:
        e = '<' if fmt == 'le' else '>'
        dt = [('p', e+'f4', 3)]
        if nrm is not None: dt.append(('n', e+'f4', 3))
        if colors is not None: dt.append(('c', 'u1', 3))
        vr = np.zeros(len(pos), dtype=dt); vr['p'] = pos
        if nrm is not None: vr['n'] = nrm
        if colors is not None: vr['c'] = colors
        fr = np.zeros(len(idx), dtype=[('k', 'u1'), ('i', e+'i4', 3)]); fr['k'] = 3; fr['i'] = idx
        with open(path, 'wb') as f:
            f.write(h.encode()); f.write(vr.tobytes()); f.write(fr.tobytes())
