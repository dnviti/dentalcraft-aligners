# Scan import: differential harness

Compares how scan import behaves in the Blender add-on (`blender-ortho-aligner`) with
how DentalCraft imports the same files through `dentalcraft-cli`. The add-on is the
oracle: its repository is read-only, and nothing here writes to it.

For each case the harness imports one or more files and records vertices, faces,
world bounding box, unit factor, arch/role and messages on both sides, then marks
each case SAME or DIFFERENT.

## Files

| File | Role |
| --- | --- |
| `paths.py` | Locations and environment overrides (see below). |
| `gen.py` | Writes the synthetic fixtures and `cases.json` into the work folder. |
| `add_real.py` | Optional: adds the four real scans as group R, read in place. |
| `runblender.sh` | Runs a Blender-side script headless (`plugin_run.py`, `names_run.py`). |
| `plugin_run.py` | Imports every case with the add-on (Blender side). |
| `names_run.py` | Role-from-file-name table of the add-on (Blender side, group C). |
| `dc_run.py` | Imports every case with `dentalcraft-cli`. |
| `dc_names.py` | Role-from-file-name table of DentalCraft (group C). |
| `analyze.py` | Compares both sides, writes `rows.json` and `expected.json`. |
| `report.py` | Writes the Markdown tables (`results_body.md`). |
| `meshw.py`, `dcmio.py` | Mesh writers and the `DCM1` reader used by `gen.py`. |
| `expected.json` | Committed baseline: one line per synthetic case, plugin and DentalCraft results. |
| `results.md` | Committed report. The appendix at the top is hand-written; the rest is `report.py` output. |

## Requirements

- Blender (tested with 5.2.2 LTS), run headless.
- Python 3 with numpy.
- A built `dentalcraft-cli`: `cd dentalcraft-app && cargo build --release -p dc-cli` (set `DENTALCRAFT_CLI` to the release binary; the default is the debug one).
- The add-on repository checked out next to this superproject (or set `BLENDER_ORTHO_ALIGNER`).

## How to run

All outputs go to the work folder, which defaults to `<system temp dir>/dentalcraft-import-diff`.
Set `IMPORT_DIFF_WORK` to choose another folder. The repository is never written to.

```sh
cd dentalcraft-aligners/golden/import
export PYTHONDONTWRITEBYTECODE=1           # keep __pycache__ out of the repo
python3 gen.py                             # synthetic fixtures + cases.json
python3 add_real.py /path/to/dataset       # optional: four real .stl scans, read in place
SCRIPT=plugin_run.py ./runblender.sh op    # plugin, real import operator
SCRIPT=names_run.py ./runblender.sh        # plugin role-from-name table
python3 dc_run.py                          # DentalCraft CLI, one process per file (case.import)
python3 dc_names.py                        # DentalCraft role-from-name table
python3 analyze.py                         # -> <work>/out/rows.json and expected.json
python3 report.py                          # -> <work>/out/results_body.md
```

`add_real.py` takes the first four `*.stl` files of the folder, sorted by name. Their
names are never printed or committed; they appear as `scan_a` to `scan_d`.
Skip `add_real.py` to run the synthetic cases only: `analyze.py` then has no R rows.

Environment overrides, read by `paths.py` and `runblender.sh`:

| Variable | Default |
| --- | --- |
| `IMPORT_DIFF_WORK` | `<system temp dir>/dentalcraft-import-diff` |
| `DENTALCRAFT_CLI` | `../../../dentalcraft-app/target/debug/dentalcraft-cli` |
| `DC_IMPORT_CMD` | `case.import`; `aligners.case.import-scans` runs the import with geometric arch detection (the command `--in` uses) |
| `IMPORT_DIFF_WORK_SCANS` | `report.py` only: work folder of a second `dc_run.py` pass with `DC_IMPORT_CMD=aligners.case.import-scans`, used for the DentalCraft columns of R2 |
| `BLENDER_ORTHO_ALIGNER` | `../../../../blender-ortho-aligner` |
| `BLENDER` | `blender` on `PATH` |

A second pass with the Aligners import: copy `cases.json`, `out/pl_op_all.json`, `out/pl_names.json`
and `out/dc_names.json` to another work folder, link its `fixtures` to the first, and run
`DC_IMPORT_CMD=aligners.case.import-scans IMPORT_DIFF_WORK=<second> python3 dc_run.py` then `analyze.py`
there. Its extra differences are the role of the files without a name hint (`results.md`, appendix 12).

## Privacy

Real scans are never committed, copied or named in a committed file. Group R in
`results.md` keeps counts and verdicts only, with the bounding box redacted.
Do not add real file names or mesh coordinates to any committed file.

## Known gaps

- The R3 timings in `results.md` came from a one-off timing script that is not part of
  this folder. They are kept as recorded.
- The "true arch" column of R2 assumes the dataset order lower, upper, lower, upper.
