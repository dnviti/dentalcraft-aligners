"""Arch frame and world frame, mirrored from the bpy-bound plugin modules.

`case_core.compute_frame(obj)` and `worldframe.delta/arch_centre(obj)` take a
Blender object and write properties, so they cannot be imported; these are the
same numerical steps on arrays (kept line for line, marked below).  If the
plugin changes them, update here.
"""
import numpy as np

from . import plugin


def mods():
    return plugin.load("anatomy"), plugin.load("meshutil")


def compute_frame(seeds, V_world):
    """Arch frame (4x4, columns x y n o) from the seeds {fdi: world point}:
    occlusal plane by least squares.  Mirror of case_core.compute_frame."""
    an, mu = mods()
    items = sorted(seeds.items())
    if len(items) < 4:
        raise ValueError("need at least 4 seeds")
    pts = np.array([p for _, p in items], np.float64)
    c, _, vt = mu.pca(pts)
    n = vt[2]
    if np.dot(np.asarray(V_world).mean(axis=0) - c, n) > 0:
        n = -n
    ant = [p for f, p in items if an.index(f) <= 2] or [p for f, p in items if an.index(f) <= 3]
    right = [(an.index(f), p) for f, p in items if an.is_right(f)]
    left = [(an.index(f), p) for f, p in items if not an.is_right(f)]
    if not right or not left or not ant:
        raise ValueError("seeds needed on both sides and on the incisors")
    pr = max(right, key=lambda t: t[0])[1]
    pl_ = max(left, key=lambda t: t[0])[1]
    post = 0.5 * (pr + pl_)
    y = np.mean(ant, axis=0) - post
    y = mu.normalize(y - np.dot(y, n) * n)
    x = mu.normalize(np.cross(y, n))
    o = post - np.dot(post - c, n) * n
    return mu.make_frame(x, y, n, o)


FRONT = np.array([0.0, -1.0, 0.0])      # worldframe.FRONT: incisors towards Blender's front view


def arch_centre(M, seeds):
    """Mirror of worldframe.arch_centre."""
    an, _ = mods()
    o, y = M[:3, 3], M[:3, 1]
    ant = [np.asarray(p) for f, p in sorted(seeds.items()) if an.index(f) == 1] or \
          [np.asarray(p) for f, p in sorted(seeds.items()) if an.index(f) <= 3]
    if not ant:
        return o.copy()
    depth = float(np.dot(np.mean(ant, axis=0) - o, y))
    return o + 0.5 * depth * y


def world_delta(M, seeds, arch):
    """Rigid transform (4x4) that puts the arch on the world axes.  Mirror of worldframe.delta."""
    x, y, n = M[:3, 0], M[:3, 1], M[:3, 2]
    up = np.array([0.0, 0.0, 1.0 if arch != "UPPER" else -1.0])
    hand = 1.0 if np.dot(x, np.cross(y, n)) >= 0 else -1.0
    W = np.column_stack([hand * np.cross(FRONT, up), FRONT, up])
    R = W @ np.column_stack([x, y, n]).T
    c = arch_centre(M, seeds)
    D = np.eye(4)
    D[:3, :3] = R
    D[:3, 3] = -R @ c
    return D
