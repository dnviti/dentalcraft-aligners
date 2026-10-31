"""Golden oracle for the Rust port of the aligners pipeline (see ../README.md).

Importable with the system python + numpy (Tier A) and from inside Blender
(Tier B, `blender_dump.py`).  Nothing here imports `bpy`; nothing writes into
the plugin repository.
"""
import os
import sys

# the plugin repository is read-only: never leave bytecode inside it
sys.dont_write_bytecode = True
os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
# the plugin may use CUDA kernels; the oracle is CPU only (exact, deterministic)
os.environ["OA_GPU"] = "0"
