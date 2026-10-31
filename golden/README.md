# Golden tests: the Blender plugin as oracle

The Rust port of the aligners pipeline is checked stage by stage against the
old Blender add-on, `/home/daniele/Documents/repos/blender-ortho-aligner`
(path `ortho_aligner/`, called P below). That repository is **read-only**:
nothing here writes to it, copies its code or vendors it.

Tolerances (PLAN section 5): vertices Hausdorff < 0.01 mm, labels >= 99 %
agreement, poses < 0.01 mm / 0.05 deg, diagnosis values relative error < 1e-3.
The measures are in `dc_geom::metrics` (Hausdorff, label agreement).

Each stage is compared on its own: the Rust stage is fed the oracle's
intermediate inputs for that stage, not the output of the Rust stage before
it, so one stage's error does not leak into the next comparison.

## Tier A: system Python, no Blender

`golden/oracle.py` (port plan B1.5) runs with the system `python3` and
`numpy`. It loads the plugin modules **read-only, by path**, without
importing the `ortho_aligner` package (whose `__init__.py` imports `bpy`),
the same way the plugin's own `tests/unit/_carica.py` does: it registers an
empty package module whose `__path__` points at P, then imports
`<package>.<stage>` so the relative imports (`from . import anatomy`) work.

Most of the plugin is pure numpy (world frame, arch forms, anatomy, demo
arch, autoseed, segmentation, teeth, diagnosis, setup, staging, schedule,
occlusion), so Tier A covers it. The oracle dumps, per stage, the inputs and
the outputs as JSON (numbers, labels, poses) and meshes as `.dcm`
(`DCM1` blobs, see `dc_mesh::dcm`).

Planned usage:

```sh
python3 golden/oracle.py --plugin /home/daniele/Documents/repos/blender-ortho-aligner \
    --stage <stage> --case demo --out golden/fixtures/<case>/<stage>/
```

## Tier B: Blender in background

Only a few plugin steps need Blender itself: decimation, the Geometry Nodes /
OpenVDB shell, the stamp booleans and `fill_holes`. Tier B runs them with
Blender 5.2.2 headless:

```sh
blender -b --factory-startup -P golden/blender_dump.py -- --out golden/fixtures/<case>/
```

`--factory-startup` keeps the user's Blender preferences and add-ons out of
the run. B1 dumps the demo scans and decimated meshes; the full pipeline,
shell and stamp come later (B5).

## Fixtures

`golden/fixtures/` holds the committed oracle outputs, one folder per case
and stage. Only synthetic data is committed: the demo arch of `P/demo.py`
(`THE RESULT IS A SYNTHETIC MODEL, NOT PATIENT DATA`) and meshes derived from
it. Teeth3DS or any patient scan is **never** committed; tests on it read
`$DC_TEETH3DS` and are `#[ignore]`.

Size budget: keep `golden/fixtures/` under **20 MB** in total and every file
under **2 MB**. Decimate meshes, store only the stages needed, prefer `.dcm`
over text formats for meshes, and regenerate rather than commit variants.
