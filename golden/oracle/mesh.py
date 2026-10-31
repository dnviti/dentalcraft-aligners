"""Mesh and label serialisation of the golden fixtures.

* `DCM1` blob, byte for byte the encoding of `dc_mesh::dcm` (see
  dentalcraft-core/crates/dc-mesh/src/dcm.rs and PLAN.md section 2):
  magic, version u32 = 1, nv, nf, attr_count (u32 each), attr kinds (u32,
  strictly increasing: 1 normals f32x3, 2 vertex labels u16, 3 face labels
  u16), f32x3 positions, u32x3 indices, attributes in table order.
* binary little-endian PLY (geometry only), the fallback format.
* binary/ASCII STL and PLY/DCM readers for the CLI.
* compact (run-length) label arrays for JSON: `pack_labels` / `unpack_labels`.
"""
import json
import struct

import numpy as np

MAGIC = b"DCM1"
VERSION = 1
ATTR_NORMALS, ATTR_VLABELS, ATTR_FLABELS = 1, 2, 3


def encode_dcm(V, T, normals=None, vertex_labels=None, face_labels=None):
    """DCM1 bytes of a mesh.  Positions are rounded to f32 (as the format says)."""
    V = np.ascontiguousarray(V, dtype="<f4").reshape(-1, 3)
    T = np.ascontiguousarray(T, dtype="<u4").reshape(-1, 3)
    attrs = []
    if normals is not None:
        attrs.append((ATTR_NORMALS, np.ascontiguousarray(normals, dtype="<f4").reshape(-1, 3)))
    if vertex_labels is not None:
        vl = np.asarray(vertex_labels)
        assert len(vl) == len(V), "one vertex label per vertex"
        attrs.append((ATTR_VLABELS, np.ascontiguousarray(vl, dtype="<u2")))
    if face_labels is not None:
        fl = np.asarray(face_labels)
        assert len(fl) == len(T), "one face label per face"
        attrs.append((ATTR_FLABELS, np.ascontiguousarray(fl, dtype="<u2")))
    out = [MAGIC, struct.pack("<4I", VERSION, len(V), len(T), len(attrs))]
    out += [struct.pack("<I", k) for k, _ in attrs]
    out += [V.tobytes(), T.tobytes()] + [a.tobytes() for _, a in attrs]
    return b"".join(out)


def decode_dcm(buf):
    """-> dict(V f32 (n,3), T uint32 (m,3), normals|None, vertex_labels|None, face_labels|None)."""
    if buf[:4] != MAGIC:
        raise ValueError("bad DCM1 magic")
    ver, nv, nf, na = struct.unpack_from("<4I", buf, 4)
    if ver != VERSION:
        raise ValueError(f"unsupported DCM1 version {ver}")
    kinds = list(struct.unpack_from(f"<{na}I", buf, 20))
    p = 20 + 4 * na
    V = np.frombuffer(buf, "<f4", 3 * nv, p).reshape(nv, 3)
    p += 12 * nv
    T = np.frombuffer(buf, "<u4", 3 * nf, p).reshape(nf, 3)
    p += 12 * nf
    out = {"V": V, "T": T, "normals": None, "vertex_labels": None, "face_labels": None}
    for k in kinds:
        if k == ATTR_NORMALS:
            out["normals"] = np.frombuffer(buf, "<f4", 3 * nv, p).reshape(nv, 3)
            p += 12 * nv
        elif k == ATTR_VLABELS:
            out["vertex_labels"] = np.frombuffer(buf, "<u2", nv, p)
            p += 2 * nv
        elif k == ATTR_FLABELS:
            out["face_labels"] = np.frombuffer(buf, "<u2", nf, p)
            p += 2 * nf
        else:
            raise ValueError(f"unknown DCM1 attribute {k}")
    if p != len(buf):
        raise ValueError("trailing bytes in DCM1")
    return out


def write_dcm(path, V, T, **attrs):
    with open(path, "wb") as f:
        f.write(encode_dcm(V, T, **attrs))


def read_dcm(path):
    with open(path, "rb") as f:
        return decode_dcm(f.read())


def write_ply(path, V, T):
    """Binary little-endian PLY, geometry only (the fallback format)."""
    V = np.ascontiguousarray(V, dtype="<f4").reshape(-1, 3)
    T = np.asarray(T).reshape(-1, 3)
    head = ("ply\nformat binary_little_endian 1.0\n"
            f"element vertex {len(V)}\nproperty float x\nproperty float y\nproperty float z\n"
            f"element face {len(T)}\nproperty list uchar int vertex_indices\nend_header\n").encode()
    faces = np.zeros(len(T), dtype=[("n", "u1"), ("i", "<i4", 3)])
    faces["n"] = 3
    faces["i"] = T
    with open(path, "wb") as f:
        f.write(head + V.tobytes() + faces.tobytes())


def read_ply(path):
    with open(path, "rb") as f:
        buf = f.read()
    end = buf.index(b"end_header\n") + len(b"end_header\n")
    head = buf[:end].decode().split("\n")
    fmt = next(h for h in head if h.startswith("format")).split()[1]
    nv = int(next(h for h in head if h.startswith("element vertex")).split()[2])
    nf = int(next(h for h in head if h.startswith("element face")).split()[2])
    props = [h.split() for h in head if h.startswith("property")]
    if fmt == "ascii":
        rows = buf[end:].decode().split("\n")
        V = np.array([[float(x) for x in r.split()[:3]] for r in rows[:nv]])
        T = np.array([[int(x) for x in r.split()[1:4]] for r in rows[nv:nv + nf]])
        return {"V": V, "T": T}
    if fmt != "binary_little_endian":
        raise ValueError(f"PLY format {fmt} not supported")
    names = {"float": "<f4", "double": "<f8", "uchar": "u1", "int": "<i4", "uint": "<u4", "short": "<i2",
             "ushort": "<u2", "char": "i1"}
    vdt = np.dtype([(p[2], names[p[1]]) for p in props if p[1] != "list"])
    vert = np.frombuffer(buf, vdt, nv, end)
    V = np.stack([vert["x"], vert["y"], vert["z"]], axis=1).astype(np.float64)
    lst = next(p for p in props if p[1] == "list")
    cdt, idt = names[lst[2]], names[lst[3]]
    fdt = np.dtype([("n", cdt), ("i", idt, 3)])
    T = np.frombuffer(buf, fdt, nf, end + vdt.itemsize * nv)["i"].astype(np.int64)
    return {"V": V, "T": T}


def read_stl(path):
    """Binary or ASCII STL with welded vertices."""
    with open(path, "rb") as f:
        buf = f.read()
    tris = None
    if len(buf) >= 84:
        n = struct.unpack_from("<I", buf, 80)[0]
        if 84 + 50 * n == len(buf):
            rec = np.frombuffer(buf, np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")]), n, 84)
            tris = rec["v"].astype(np.float64).reshape(-1, 3)
    if tris is None:
        toks = [l.split() for l in buf.decode(errors="ignore").splitlines() if l.strip().startswith("vertex")]
        tris = np.array([[float(x) for x in t[1:4]] for t in toks])
    uniq, inv = np.unique(np.round(tris, 6), axis=0, return_inverse=True)
    return {"V": uniq, "T": inv.reshape(-1, 3).astype(np.int64)}


def read_mesh(path):
    """Mesh file by extension (.dcm, .ply, .stl) -> dict with V, T (+ labels for .dcm)."""
    p = str(path).lower()
    if p.endswith(".dcm"):
        return read_dcm(path)
    if p.endswith(".ply"):
        return read_ply(path)
    if p.endswith(".stl"):
        return read_stl(path)
    raise ValueError(f"unknown mesh format: {path}")


def pack_labels(labels):
    """Compact JSON form of an integer label array (FDI numbers, 0 = gum): run-length encoded,
    {"n": N, "rle": [label0, count0, label1, count1, ...]}.  The scans' vertex order is spatially
    coherent, so a full scan has a few thousand runs.  No decompression library is needed to read it."""
    a = np.asarray(labels).astype(np.int64).ravel()
    if a.size == 0:
        return {"n": 0, "rle": []}
    cut = np.nonzero(a[1:] != a[:-1])[0] + 1
    starts = np.concatenate([[0], cut])
    counts = np.diff(np.concatenate([starts, [a.size]]))
    rle = np.stack([a[starts], counts], axis=1).ravel()
    return {"n": int(a.size), "rle": [int(x) for x in rle]}


def unpack_labels(d):
    r = np.asarray(d["rle"], np.int64).reshape(-1, 2)
    out = np.repeat(r[:, 0], r[:, 1])
    assert len(out) == d["n"], "label run lengths do not add up to n"
    return out


def dump_json(path, obj):
    with open(path, "w") as f:
        json.dump(obj, f, separators=(",", ":"), allow_nan=False)
        f.write("\n")
