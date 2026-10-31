"""Loading the Blender add-on modules without Blender, read-only.

The add-on package `ortho_aligner/__init__.py` imports `bpy`, so the package is
never imported.  As the plugin's own `tests/unit/_carica.py` does, an empty
package module whose `__path__` points at the plugin directory is registered and
`<package>.<stage>` is imported, so relative imports (`from . import anatomy`)
resolve.

Modules that need `bpy` at import time (`case*`, `worldframe`, `props`, ...) are
not loadable; their bpy-free functions are pulled out of the source with
`extract()` (ast: only the requested `def`s and constants are executed, the file
is never modified or imported).

`mathutils` (Blender's C module) is replaced by a numpy shim (`mu_shim.py`) so
that `gpu_query`'s CPU paths work in system python.  Inside Blender the real
module is used.
"""
import ast
import importlib
import os
import sys
import types

PLUGIN_ROOT = os.environ.get("OA_PLUGIN", "/home/daniele/Documents/repos/blender-ortho-aligner")
PKG = "oa_golden"

sys.dont_write_bytecode = True


def plugin_dir(root=None):
    return os.path.join(root or PLUGIN_ROOT, "ortho_aligner")


def install(root=None, shim_mathutils=True):
    """Register the package stub (idempotent).  Returns the package module."""
    if PKG in sys.modules:
        return sys.modules[PKG]
    d = plugin_dir(root)
    if not os.path.isdir(d):
        raise FileNotFoundError(f"plugin not found: {d}")
    if shim_mathutils and "mathutils" not in sys.modules:
        try:
            import mathutils  # noqa: F401  (inside Blender)
        except ImportError:
            from . import mu_shim
            mu_shim.install()
    pkg = types.ModuleType(PKG)
    pkg.__path__ = [d]
    sys.modules[PKG] = pkg
    return pkg


def load(name, root=None):
    """The plugin module `name` (e.g. 'autoseed'), imported as `oa_golden.<name>`."""
    install(root)
    return importlib.import_module(f"{PKG}.{name}")


def read_source(name, root=None):
    with open(os.path.join(plugin_dir(root), name + ".py"), encoding="utf-8") as f:
        return f.read()


def extract(name, symbols, namespace, root=None):
    """Execute only the top-level defs / assignments called `symbols` of the plugin
    module file `name` inside `namespace` (a dict, the functions' globals) and
    return the namespace.  For bpy-bound modules whose chosen functions do not
    touch bpy.  Symbols are taken in file order."""
    src = read_source(name, root)
    tree = ast.parse(src)
    want = set(symbols)
    keep = []
    for node in tree.body:
        names = []
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names = [node.name]
        elif isinstance(node, ast.Assign):
            names = [t.id for t in node.targets if isinstance(t, ast.Name)]
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names = [node.target.id]
        if want & set(names):
            keep.append(node)
    found = {n for node in keep for n in (
        [node.name] if isinstance(node, (ast.FunctionDef, ast.ClassDef))
        else [t.id for t in getattr(node, "targets", [])] or [node.target.id])}
    missing = want - found
    if missing:
        raise KeyError(f"{name}.py: symbols not found: {sorted(missing)}")
    mod = ast.Module(body=keep, type_ignores=[])
    namespace.setdefault("__package__", PKG)      # so `from . import x` inside the functions resolves
    namespace.setdefault("__name__", f"{PKG}.{name}")
    exec(compile(mod, f"<{name}.py extract>", "exec"), namespace)
    return namespace
