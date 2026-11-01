# Golden tests: the Blender plugin as oracle

The Rust port of the aligners pipeline is checked stage by stage against the
old Blender add-on, the `blender-ortho-aligner` repository, checked out next to
the superproject or wherever `OA_PLUGIN` points (path `ortho_aligner/`, called P
below). That repository is **read-only**:
nothing here writes to it, copies its code or vendors it (the scripts run with
`python3 -B` / `PYTHONDONTWRITEBYTECODE=1` and Blender with `--factory-startup`,
so not even `__pycache__` appears there).

## Tolerances (PLAN section 5)

| Quantity | Tolerance |
| --- | --- |
| mesh vertices | Hausdorff < 0.01 mm |
| segmentation labels | >= 99 % agreement |
| tooth poses | < 0.01 mm and < 0.05 deg |
| diagnosis values | relative error < 1e-3 |

The measures are in `dc_geom::metrics` (Hausdorff, label agreement). Each stage
is compared on its own: the Rust stage is fed the oracle's intermediate inputs
for that stage (the files in the previous stage's folder), not the output of the
Rust stage before it, so one stage's error does not leak into the next
comparison.

The oracle computes in float64 on float32 inputs (Blender stores f32 vertices,
the `.dcm` files are f32 too). Where P calls Blender's `mathutils` (BVH, KD
tree), Tier A uses an exact float64 numpy stand-in (`oracle/mu_shim.py`);
Blender's own float32 trees differ from it by ~1e-5 mm, far below the table.

## Layout

```text
golden/
  README.md  Makefile  .gitignore
  oracle.py            Tier A command line (system python3 + numpy)
  blender_dump.py      Tier B (Blender 5.2.2 headless)
  check.py             size budget and format checks of fixtures/
  oracle/              package: plugin.py (read-only loader), mesh.py (DCM1, PLY, labels),
                       frame.py, settings.py, mu_shim.py, stages.py (one function per stage)
  fixtures/<case>/     scan.dcm  decimated.dcm  meta.json        (Tier B)
                       <stage>/<stage>.json (+ extra files)       (Tier A)
```

Cases: `demo_{upper,lower}_{mild,moderate,severe}` (the synthetic arch of
`P/demo.py`, seed 7, voxel remesh 0.35 mm).

## Regenerate

```sh
cd dentalcraft-aligners/golden
make                  # Tier B (Blender) then Tier A, every case, one after the other: ~30 min
make -j6 tier-a       # Tier A only, the six cases in parallel: ~10 min (28 cores)
make tier-b           # demo scans and decimated meshes only (~10 s)
make case-demo_lower_mild   # one case, Tier A
make check            # budget and format checks
```

Variables: `PLUGIN` (default `~/Documents/repos/blender-ortho-aligner`; the
`OA_PLUGIN` environment variable does the same for the scripts), `BLENDER`,
`PYTHON`. Tier B output is deterministic (the same bytes on a rerun), so a
regeneration that changes a fixture means the plugin changed: look at the diff.

## Tier A: `oracle.py`, no Blender

```sh
python3 -B golden/oracle.py list
python3 -B golden/oracle.py <stage> --in golden/fixtures/demo_lower_mild [--out DIR]
python3 -B golden/oracle.py all    --in golden/fixtures/demo_lower_mild
python3 -B golden/oracle.py autoseed --in scan.stl --arch UPPER --out /tmp/x     # a bare mesh
```

`--in` is a case folder (or its `meta.json`), or a bare mesh (`.dcm`, `.ply`,
`.stl`; then `--arch` is required and there is no ground truth). `--out` is the
folder the stage writes (default `<case>/<stage>/`, for `all` the parent of
the stage folders). A stage that needs an earlier one recomputes it in memory
without writing it; `--reuse` reads the segmentation labels back from
`<case>/segmentation/` (recomputing it takes 1.5-2.5 min: the label transfer is the slow part). Other options: `--seeds FILE.json`
(world seeds `{fdi: [x, y, z]}` instead of the ground truth), `--decimated FILE`,
`--plugin DIR`, `--no-closed-teeth`.

The plugin modules are loaded by path without importing the `ortho_aligner`
package (its `__init__.py` imports `bpy`), as the plugin's own
`tests/unit/_carica.py` does. Modules that need `bpy` at import time (`case_*`,
`worldframe`, `props`) are not loaded: the functions needed are pulled out of
the source with `ast` (`plugin.extract`: `transfer_labels`, `analyze`,
`conservative_config`, `rest_cusps`, `complexity`, `_decimate_here`, ...) or,
where they take a Blender object, their numerical core is mirrored and marked
so (`oracle/frame.py`: `compute_frame`, `worldframe.delta`; `stage_setup` and
`stage_staging`: `run_setup`, `run_staging`, `_moves`). The scene settings are
the defaults of `props.OA_Settings` (`oracle/settings.py`, parsed from the file).

### Stages and their files

Every stage JSON has `case`, `arch`, an `inputs` block (what the stage was fed)
and its outputs. Matrices are 4x4 row-major nested lists; points are `[x, y, z]`
in mm; FDI numbers are JSON object keys (strings). **Arch coordinates** are the
frame of the arch (origin between the last molars on the occlusal plane, +Y
anterior, +Z occlusal) = `worldframe.arch_frame.M` applied inversely.

| Stage | Plugin code | Output | Notes |
| --- | --- | --- | --- |
| `archform` | `archform`, `demo.ideal_arch` | `archform/archform.json` | beta curve samples (`s`, `xy`, tangent, outward normal), `s` of the seeds, chord search, quartic and even-polynomial fits of the seeds |
| `worldframe` | `case_core.compute_frame`, `worldframe.delta` (mirrored) | `worldframe/worldframe.json` | arch frame `M` and `M_inv`, seeds in arch coordinates, arch centre, world delta |
| `autoseed` | `autoseed.arch_data`, `propose` | `autoseed/autoseed.json` | proposed seeds (world), arch frame, profiles, DP diagnostics, error vs the ground-truth seeds |
| `segmentation` | `segmentation.segment_faces`, `case_teeth.transfer_labels`, `refine_boundaries`, `clean_vertex_labels` | `segmentation/segmentation.json` | inputs: ground-truth seeds in arch coordinates and `decimated.dcm`; outputs: labels of the decimated faces, of the full-scan vertices after transfer and after refinement, agreement and per-tooth IoU vs the ground truth |
| `teeth` | `teeth.compute_teeth` | `teeth/teeth.json` | per tooth: centroid, axes, widths, height, contact points, margin, centre of resistance, `rest_frame` (arrays `vidx`/`fidx` are left out) |
| `diagnosis` | `case_plan.analyze` (extracted) | `diagnosis/diagnosis.json` | space analysis, Little index, rotations, largest space, CAT-CAT items and level |
| `setup` | `setup_conservativo.conservative_setup` | `setup/setup.json` | target delta and pose per tooth (teeth that stay are absent from `targets_delta`), IPR, log, arch line |
| `staging` | `reconstruct.closed_tooth`, `staging.plan`, `case_plan.complexity` (extracted) | `staging/staging.json`, `staging/closed_<fdi>.dcm` | `plan`: pattern, `n_stages`, per-stage `q` (6 numbers: tx ty tz, rx ry rz) per tooth, phases, IPR stage, stats; closed crown solids in tooth-local coordinates |
| `schedule` | `schedule.build`, `stage_intervals`, `to_ics` | `schedule/schedule.json`, `schedule_{fixed,adaptive}.ics` | start date fixed to 2026-01-05; FIXED and ADAPTIVE policies |

Assumptions of `setup`/`staging` (the plugin reads them from the Blender case):
no tooth locked or left out of the aligner, no drawn arch line, no opposing arch
or bite, no clinician decision in the classification review, default scene settings.

### Label arrays

Vertex or face labels in JSON are run-length encoded:
`{"n": N, "rle": [label0, count0, label1, count1, ...]}` (FDI, `0` = gingiva).
Python: `oracle.mesh.unpack_labels`. Rust: expand the `(label, count)` pairs;
the result has `n` entries in the vertex (or face) order of the mesh file named
in `inputs`. Vertex labels of the full scan follow the order of `scan.dcm`.

### Mesh files

`.dcm` is the `DCM1` blob of `dc_mesh::dcm` (little endian: magic, version 1,
`nv`, `nf`, attribute table, f32x3 positions, u32x3 indices, optional normals,
u16 vertex labels, u16 face labels). `oracle/mesh.py` writes and reads it
(checked byte for byte against `dc_mesh::dcm::decode`/`encode`) and also binary
little-endian PLY (`write_ply`, geometry only) as a fallback.

## Tier B: Blender in background

Only a few plugin steps need Blender itself: the demo generator (voxel remesh),
decimation, and later the Geometry Nodes / OpenVDB shell, the stamp booleans and
`fill_holes`. Tier B runs them with Blender 5.2.2 headless:

```sh
blender -b --factory-startup -P golden/blender_dump.py -- --out golden/fixtures
```

`--factory-startup` keeps the user's Blender preferences and add-ons out of the
run; the plugin is loaded from its read-only repository by path (`sys.path`
stub package as above, nothing installed). Options: `--only lower_mild,...`,
`--voxel`, `--seed`, `--faces`, `--plugin`.

Per case it writes `scan.dcm` (full scan, world coordinates, ground-truth FDI
per vertex), `decimated.dcm` (the scan in arch coordinates after Blender's
Decimate modifier to 30000 faces, i.e. what `case_teeth.segment_iter` hands to
the random walker) and `meta.json` (ground-truth seeds and malposition, arch
frame, provenance: Blender, plugin commit). B1 dumps the demo scans and
decimated meshes; the full pipeline, shell and stamp come later (B5): extend
`dump_case` and add a stage function next to the others.

## Using the fixtures from Rust tests

Fixtures live in this repository, so tests reach them from any crate of
`dentalcraft-aligners/crates/<crate>` with a path relative to the manifest:

```rust
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../golden/fixtures");

fn case(name: &str) -> std::path::PathBuf {
    std::path::Path::new(GOLDEN).join(name)
}

#[test]
fn segmentation_matches_the_plugin() {
    let dir = case("demo_lower_mild");
    let decimated = dc_mesh::dcm::decode(&std::fs::read(dir.join("decimated.dcm")).unwrap()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("segmentation/segmentation.json")).unwrap()).unwrap();
    // seeds_arch and parameters are in expected["inputs"]; run the Rust stage on `decimated` and compare
    // with dc_geom::metrics (label agreement >= 0.99).
}
```

Read the stage's `inputs` block to feed the Rust stage, and compare with the
tolerances above. Teeth3DS or any patient scan is never in this folder; tests
on it read `$DC_TEETH3DS` and are `#[ignore]`.

## Fixtures and size budget

`golden/fixtures/` holds the committed oracle outputs, one folder per case.
Only synthetic data is committed: the demo arch of `P/demo.py`
(`THE RESULT IS A SYNTHETIC MODEL, NOT PATIENT DATA`) and what the oracle derives
from it.

Size budget: **40 MB** in total and **4 MB** per file (raised from 20 / 2 MB
at B1.5: a full scan at the planned voxel of 0.35 mm has about 80k vertices and
160k faces, 2.8-3.2 MB as `.dcm`, and six of them plus their decimated meshes
already make 21 MB; the full-resolution scan is needed for the label transfer
and boundary refinement stages). Current size: 33 MB (`make check`): 21 MB of scans, 11 MB of closed teeth (`staging/closed_*.dcm`, use `--no-closed-teeth` to skip them). Prefer `.dcm`
over text formats for meshes, keep per-stage data in JSON of a few hundred KB,
and regenerate rather than commit variants. Do not commit anything above the
budget; `check.py` fails when it is exceeded.
