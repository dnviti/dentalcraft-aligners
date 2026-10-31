"""Default scene settings (`OA_Settings` of props.py) without Blender.

props.py needs `bpy`; its property declarations are plain annotations, so the
file is parsed with `ast` and each `default=` expression is evaluated (against
`P` of evidence.py, and the `quality` presets).  Enum defaults are literal
strings, float/int/bool defaults are numbers.  Properties whose default cannot
be evaluated are left out.  `defaults()` is the `s` of `case.settings()` for a
fresh file: the values the plugin's pipeline uses unless the clinician changes
them.  Overrides: `defaults(rw_faces=20000)`.
"""
import ast
from types import SimpleNamespace

from . import plugin


def defaults(**override):
    tree = ast.parse(plugin.read_source("props"))
    ns = {"P": plugin.load("evidence").P, "quality": plugin.load("quality")}
    plugin.extract("props", ["_preset"], ns)
    cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "OA_Settings")
    out = {}
    for node in cls.body:
        if not (isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)
                and isinstance(node.annotation, ast.Call)):
            continue
        for kw in node.annotation.keywords:
            if kw.arg == "default":
                try:
                    out[node.target.id] = eval(compile(ast.Expression(kw.value), "<props default>", "eval"), ns)
                except Exception:
                    pass
    out.update(override)
    return SimpleNamespace(**out)
