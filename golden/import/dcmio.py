import struct, numpy as np
def read_dcm(p):
    b = open(p,'rb').read()
    assert b[:4]==b'DCM1'
    ver,nv,nf,ac = struct.unpack('<4I', b[4:20])
    off = 20+4*ac
    pos = np.frombuffer(b, '<f4', nv*3, off).reshape(nv,3).copy(); off += nv*12
    idx = np.frombuffer(b, '<u4', nf*3, off).reshape(nf,3).copy()
    return pos, idx
