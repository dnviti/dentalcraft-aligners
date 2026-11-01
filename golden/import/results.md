# Appendix: observations (measured unless marked "by code" or "inferred")

Second run, after the shared import pipeline (`dc_case::import`, cleanup, structured warnings, one batch allocation, geometric arch detection in the Aligners module). Release `dentalcraft-cli`, 136 synthetic cases plus 8 real-scan runs. `case.import` path: 114 SAME of 144 (the first run, on the old `case.import`: 68 SAME of 136). `aligners.case.import-scans` path (what `--in` uses): 92 SAME of 144, the 22 extra differences all being the role of an arch without a name hint (appendix 12). Every remaining difference is classified in appendix 13; none is unintended.

1. OBJ axis. Plugin world coordinates of an OBJ = (x, -z, y) of the file coordinates (Blender OBJ importer default forward -Z / up Y, kept as `matrix_world` rotation; mesh data stays in file coordinates). DentalCraft now applies the same rotation at import (`rotate_y_up_to_z_up`), so A-obj is SAME. STL and PLY: no remap on either side.
2. Units (`fix_units` vs `normalize_units`). Plugin: 0.02<=size<=0.2 -> x1000, 2<=size<=20 -> x10, else x1 (no micrometre rule). DentalCraft: same two ranges plus 20000..200000 -> x0.001 (um). Exact-boundary extents differ because the plugin compares the float32 `dimensions` with double literals: extent f32(0.02) and f32(0.2) are NOT rescaled by the plugin (stay 0.02 mm / 0.2 mm) and ARE rescaled by DentalCraft (20 mm / 200 mm). 2.0 and 20.0 behave the same on both (x10). Extents 60000, 20000, 200000: plugin x1 (a 60 m model), DentalCraft x0.001 (60 mm). 15 -> x10 (150 mm) on both, 3 -> x10 on both, 2.4 -> x10 (24 mm) on both ("inch" scans are treated as cm on both).
3. Messages. DentalCraft returns structured warnings (`code`, `key`, `args`, translated `message`): `rescaled`, `tiny`, `low_polygons`, `obj_objects_merged`, `bad_faces_removed`, `replaced`, `arch_conflict`, `provisional`, `bite_limit`. `rescaled` appears exactly when the plugin prints its INFO "riscalato xN" (the analysis checks it). Plugin: the WARNING "Il modello e molto piccolo" when `max(obj.dimensions) < 5` is evaluated on STALE `obj.dimensions` right after `fix_units` (measured with a one-off probe script, not included: dimensions stay 0.06 until `view_layer.update()`), so the plugin shows the "very small" warning on models it has just rescaled to 60 mm (B-0.06, B-2.4, B-2.0, B-3). DentalCraft's `tiny` looks at the rescaled mesh and does not fire there. Measured headless; the GUI path runs the same calls in the same order (inferred, not run). DentalCraft also warns under 1000 faces (`low_polygons`), which the plugin does only at its arch step.
4. Role detection. Plugin: whole-word tokens after camelCase split (+ `upperjaw`, `lowerjaw`, `maxill*`, `mandib*` substrings), a name with both an upper and a lower token gives None; no bite role; no-hint default is always LOWER whatever the case holds. DentalCraft: the same words and rules (the C2 table agrees on every row except the three bite names, which the plugin reads as upper, lower or no hint) plus the bite words (`bite`, `morso`, as whole words). A file with no hint takes the first free arch, lower first, as a provisional scan (`provisional` warning), never replaces a scan and never lands in bite; with both arches taken it is refused (`needs_role`). Directory names are ignored by both (the plugin passes the basename).
5. Duplicates and degenerate faces. Blender importers clean the mesh (removed the 50 repeated-index faces, and all 300 duplicate/reversed duplicate faces: 4738 vs 4788 and 4720 vs 5020). DentalCraft now does the same at import (`remove_bad_faces`: repeated index first, then a face with the vertex set of an earlier one; reported as `cleanup` and the `bad_faces_removed` warning): 4738 and 4720 faces on both sides. Collinear faces with three distinct indices and isolated vertices stay on both sides.
6. Weld. Both weld bit-identical STL corners. DentalCraft also welds within 1e-6 x bbox diagonal (cell hash), the plugin does not: A-stl-bin 83171 vs 83172 vertices (two source vertices 7.4e-5 mm apart merge and 2 faces collapse onto repeated indices; the cleanup removes them: 166390 faces against 166392); jitter 2e-5 mm: 13665 vs 35699 vertices (partial, cell-boundary dependent merging); jitter 1e-3 mm: 35695 vs 35700.
7. Failure handling. Plugin: an empty file (STL, OBJ, PLY), a 60-byte STL, an ASCII STL with no facets, a PLY with 0 vertices and random-byte PLY raise an uncaught RuntimeError from Blender's importer inside `_import_mesh` (Python traceback shown to the user, no clean message); a comment-only OBJ and random-byte OBJ give the clean "Nessuna mesh importata". The plugin ACCEPTS: a binary STL with 0 triangles, an OBJ and a PLY with vertices and no faces, truncated binary STL (11900 declared, 5000 present, with and without triangle alignment), random bytes and text named .stl (all become an empty 0-vertex object turned into a case, with the misleading "very small" warning), NaN coordinate (6002 vertices, bbox min y shown as 0 instead of 15.43), an OBJ vertex "inf" (bbox min shown as 0 instead of 15.43, no error) and an OBJ whose face index is out of range (imported as 2 vertices, 0 faces). DentalCraft rejects all of those with a clear message and a stable code: `invalid` (truncated, garbage STL, non-finite, bad number, index out of range), `empty` (no usable face: empty OBJ, ASCII STL `solid x/endsolid x`, 0-vertex PLY, vertices without faces, 0-triangle STL, every face removed by the cleanup), `unsupported` (.txt, .3mf, no extension; rejected by both). An empty file named like an arch no longer replaces a valid scan (E-good-upper-then-empty-upper-obj: both sides keep the first scan and report the error). Extensions: uppercase and mixed-case extensions work on both.
8. Multi-object OBJ. Plugin takes only the first object (`new[0]`, alphabetical) and leaves the other mesh object in the scene unprocessed (1800 v / 3540 f processed). DentalCraft merges all objects into one scan (3600 v / 7080 f, bbox spans both) and warns (`obj_objects_merged`).
9. Same role twice / same file twice. Plugin keeps every object as a separate case. DentalCraft keeps one scan per role; the later import replaces the earlier (`replaced: true`, `replaced` warning, undoable; the old mesh blob is dropped).
10. Workspace (GUI) path, by code only (`dentalcraft-app/crates/dc-workspace/src/import.rs`): the workspace, the CLI `--in`, the MCP `import_scan` tool and the batch `case.import` all call the same `import_files` / `import_scans`, so the geometry results above apply and the role allocation is the same: hinted files reserve their arch first, then detected ones (by confidence), then unhinted ones fill the free arches lower first. This harness replays the files one `case.import` per file, so a sequence of single-file imports (C-seq, E rows) behaves as "later replaces earlier" or "no place", not as one batch.
11. Real scans (group R, 4 STL, mm, 100k-430k vertices; file names and coordinates are not recorded here). Vertices, faces and bbox agree on both sides for all 8 runs (original and neutral names, units x1 on both, no face removed by the cleanup): scan_a 429605 v / 855739 f; scan_b 402802 / 802606; scan_c 109716 / 215588; scan_d 100094 / 197756. Original names: both give the right arch (the name hints match the true arch). Neutral names: no hint on either side; the plugin import assigns LOWER to all four (2 wrong); DentalCraft `case.import` gives a provisional lower to all four (2 wrong, as the plugin; it is a guess and says so). Plugin `detect_arch` on the neutral copies finds the correct arch on all four (palate, Wilson curve, scanner orientation), and so does DentalCraft at import through `aligners.case.import-scans`, with the same arch, confidence and cue (R2).
12. Second path, `aligners.case.import-scans` (the command `--in` runs when the Aligners module is loaded): a file with neither an explicit role nor a name hint gets its arch from the geometry (`detect_arch`, same function as the plugin's arch step). The harness arch (a synthetic upper horseshoe) is therefore detected as upper, and the plugin at import says LOWER (it detects only later, at the arch step): 22 extra "role/arch LOWER vs upper" rows (20 rows of group C, the name-without-hint rows and the C sequences, and R-neutral scan_b and scan_d, where DentalCraft is right and the plugin import is wrong). Nothing else changes: geometry, units, cleanup, rejections and sequences are identical on both paths.
13. Remaining differences on the `case.import` path (30 of 136 synthetic rows, all intended; R rows: 0). Intended means a deliberate behaviour of the new pipeline, recorded in `dentalcraft-core/crates/dc-case/tests/import_golden.rs` (DIVERGES) and in the notes above: weld tolerance (A-stl-bin, A-stl-ascii, D-jitter2e5, D-jitter1e3: appendix 6); micrometre rule (B-60000, B-20000, B-200000) and inclusive f32 boundaries (B-0.02, B-0.2): appendix 2; bite role (C-bite, C-morso, C-UpperJaw_bite): appendix 4; one scan per role and refusal when both arches are taken (C-seq-nohint-4, C-seq-nohint-then-upper-then-nohint, E-same-role-twice, E-same-file-twice, E-upper-lower-upper): appendices 4 and 9; files the plugin accepts as empty or half-empty objects and DentalCraft refuses (D-tri0_bin.stl, D-noface.obj, D-face0.ply, D-trunc_mid.stl, D-trunc_aligned.stl, D-garbage.stl, D-garbage_text.stl, D-nan.stl, D-inf.obj, D-badidx.obj, E-good-upper-then-garbage-upper-stl, E-good-upper-then-truncated-upper-stl): appendix 7; multi-object OBJ merged (D-multiobj.obj): appendix 8.


# Scan import: Blender plugin (oracle) vs DentalCraft

Generated by `report.py` from the `rows.json` that `analyze.py` builds from the plugin and DentalCraft runs.

## Method

- Plugin: the real operator `bpy.ops.oa.import_scans(filepath=...)`, run headless in Blender 5.2.2 (`--background --factory-startup`, user dirs inside the work folder). Add-on loaded from its path via `sys.path` and `register()`; not installed. One fresh scene per case (all objects and meshes removed, `oa_world` cleared). `report()` calls were captured by wrapping `execute` with a proxy (reports are forwarded, behaviour unchanged). Arch = `obj.oa.arch`. World bbox = `matrix_world @ vertices` of the first imported mesh object.
- DentalCraft: `dentalcraft-cli run --cmd project.new --cmd case.import={path} ... --save --print` (session/CLI path, `dc-session` `case.import` on the shared import pipeline; release build of `dentalcraft-app/target/release/dentalcraft-cli`, built inside the superproject). `case.import` has no geometric arch detection; the same run with `DC_IMPORT_CMD=aligners.case.import-scans` (what `--in` uses when the Aligners module is loaded) adds it, see R2 and appendix 12. World bbox = scan `aabb` from the case summary; `import_xform` and `world_frame` are identity after import, so scan coordinates are world coordinates. The CLI aborts on the first failing command, so one process per file, imports replayed cumulatively.
- The DentalCraft workspace (GUI) path in `dc-workspace/src/import.rs` was NOT run: it is described "by code" in the notes of the summary.
- v = vertices, f = faces/polygons as reported by each side, tri = triangle count when different from f. wbb = world bbox min..max in mm after import. `xN` = unit factor (plugin: parsed from the INFO message "riscalato xN", 1 when no message; DentalCraft: `source_units`, m=1000, cm=10, mm=1, um=0.001).
- Fixtures are synthetic only (`gen.py`, `fixtures/`). Group A: `demo_upper_mild` arch from `dentalcraft-aligners/golden/fixtures` (83172 v, 166392 f) written as six formats. Groups B-E: a generated asymmetric horseshoe tube (6000 v, 11900 f) and variants.


## A. Same mesh, different formats

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| A-stl-bin | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83171 f=166390 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 lower \| warn: bad_faces_removed,provisional | DIFFERENT: step1: verts 83172 vs 83171; step1: tris 166392 vs 166390 |
| A-stl-ascii | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83171 f=166390 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 lower \| warn: bad_faces_removed,provisional | DIFFERENT: step1: verts 83172 vs 83171; step1: tris 166392 vs 166390 |
| A-obj | v=83172 f=166392 wbb=(-33.8424,-9.0062,-2.4837)..(33.8424,11.9975,51.7909) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-9.0062,-2.4837)..(33.8424,11.9975,51.7909) x1 lower \| warn: provisional | SAME |
| A-ply-ascii | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 lower \| warn: provisional | SAME |
| A-ply-bin-le | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 lower \| warn: provisional | SAME |
| A-ply-bin-be | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 lower \| warn: provisional | SAME |

## B. Units

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| B-0.06 <br>_largest extent 0.06_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1000 LOWER \| WARNING: u_0.06: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_0.06: riscalato x1000 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1000 lower \| warn: rescaled,provisional | SAME geometry and factor; the "very small" warning differs (the plugin reads a stale size, appendix 3) |
| B-6 <br>_largest extent 6.0_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x10 LOWER \| INFO: u_6: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x10 lower \| warn: rescaled,provisional | SAME |
| B-60 <br>_largest extent 60.0_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1 lower \| warn: provisional | SAME |
| B-60000 <br>_largest extent 60000.0_ | v=6000 f=11900 wbb=(0,0,0)..(60000.00,13312.76,8415.0205) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x0.001 lower \| warn: rescaled,provisional | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-2.4 <br>_largest extent 2.4_ | v=6000 f=11900 wbb=(0,0,0)..(24,5.3251,3.366) x10 LOWER \| WARNING: u_2.4: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.4: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(24,5.3251,3.366) x10 lower \| warn: rescaled,provisional | SAME geometry and factor; the "very small" warning differs (the plugin reads a stale size, appendix 3) |
| B-0.02 <br>_largest extent 0.02_ | v=6000 f=11900 wbb=(0,0,0)..(0.02,0.0044,0.0028) x1 LOWER \| WARNING: u_0.02: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x1000 lower \| warn: rescaled,provisional | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 1000 |
| B-0.2 <br>_largest extent 0.2_ | v=6000 f=11900 wbb=(0,0,0)..(0.2,0.0444,0.0281) x1 LOWER \| WARNING: u_0.2: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x1000 lower \| warn: rescaled,provisional | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 1000 |
| B-2.0 <br>_largest extent 2.0_ | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x10 LOWER \| WARNING: u_2.0: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.0: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x10 lower \| warn: rescaled,provisional | SAME geometry and factor; the "very small" warning differs (the plugin reads a stale size, appendix 3) |
| B-20.0 <br>_largest extent 20.0_ | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x10 LOWER \| INFO: u_20.0: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x10 lower \| warn: rescaled,provisional | SAME |
| B-15 <br>_largest extent 15.0_ | v=6000 f=11900 wbb=(0,0,0)..(150,33.2819,21.0376) x10 LOWER \| INFO: u_15: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(150,33.2819,21.0376) x10 lower \| warn: rescaled,provisional | SAME |
| B-3 <br>_largest extent 3.0_ | v=6000 f=11900 wbb=(0,0,0)..(30,6.6564,4.2075) x10 LOWER \| WARNING: u_3: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_3: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(30,6.6564,4.2075) x10 lower \| warn: rescaled,provisional | SAME geometry and factor; the "very small" warning differs (the plugin reads a stale size, appendix 3) |
| B-0.019 <br>_largest extent 0.019_ | v=6000 f=11900 wbb=(0,0,0)..(0.019,0.0042,0.0027) x1 LOWER \| WARNING: u_0.019: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.019,0.0042,0.0027) x1 lower \| warn: tiny,provisional | SAME |
| B-0.21 <br>_largest extent 0.21_ | v=6000 f=11900 wbb=(0,0,0)..(0.21,0.0466,0.0295) x1 LOWER \| WARNING: u_0.21: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.21,0.0466,0.0295) x1 lower \| warn: tiny,provisional | SAME |
| B-1.9 <br>_largest extent 1.9_ | v=6000 f=11900 wbb=(0,0,0)..(1.9,0.4216,0.2665) x1 LOWER \| WARNING: u_1.9: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(1.9,0.4216,0.2665) x1 lower \| warn: tiny,provisional | SAME |
| B-21 <br>_largest extent 21.0_ | v=6000 f=11900 wbb=(0,0,0)..(21,4.6595,2.9453) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(21,4.6595,2.9453) x1 lower \| warn: provisional | SAME |
| B-100 <br>_largest extent 100.0_ | v=6000 f=11900 wbb=(0,0,0)..(100,22.1879,14.025) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(100,22.1879,14.025) x1 lower \| warn: provisional | SAME |
| B-19999 <br>_largest extent 19999.0_ | v=6000 f=11900 wbb=(0,0,0)..(19999.00,4437.3643,2804.8667) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(19999.00,4437.3643,2804.8667) x1 lower \| warn: provisional | SAME |
| B-20000 <br>_largest extent 20000.0_ | v=6000 f=11900 wbb=(0,0,0)..(20000.00,4437.5864,2805.0071) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x0.001 lower \| warn: rescaled,provisional | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-200000 <br>_largest extent 200000.0_ | v=6000 f=11900 wbb=(0,0,0)..(200000.00,44375.86,28050.07) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x0.001 lower \| warn: rescaled,provisional | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-200001 <br>_largest extent 200001.0_ | v=6000 f=11900 wbb=(0,0,0)..(200001.00,44376.09,28050.21) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(200001.00,44376.09,28050.21) x1 lower \| warn: provisional | SAME |
| B-0.0199 <br>_largest extent 0.0199_ | v=6000 f=11900 wbb=(0,0,0)..(0.0199,0.0044,0.0028) x1 LOWER \| WARNING: u_0.0199: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.0199,0.0044,0.0028) x1 lower \| warn: tiny,provisional | SAME |
| B-2.0001 <br>_largest extent 2.0001_ | v=6000 f=11900 wbb=(0,0,0)..(20.001,4.4378,2.8051) x10 LOWER \| WARNING: u_2.0001: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.0001: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(20.001,4.4378,2.8051) x10 lower \| warn: rescaled,provisional | SAME geometry and factor; the "very small" warning differs (the plugin reads a stale size, appendix 3) |

## C. Role from file name and defaults

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| C-upper.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-lower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-maxilla.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-mandible.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-mascellare.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-mandibolare.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-superiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-inferiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-sup.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-inf.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-Patient_sup.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-Oberkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-Unterkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-UpperJaw.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-lowerJaw.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-scan_upper_lower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-flower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-superman.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-bite.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch LOWER vs bite |
| C-morso.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch LOWER vs bite |
| C-scan1.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-UPPER.STL | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-maxillary.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-mandibular.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-Arcata_Superiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-arcata-inferiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-UpperJaw_bite.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch UPPER vs bite |
| C-inferior.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-superior.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-maxill.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-mandib.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-Lower.ply | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-case_UPPER_final.obj | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 upper | SAME |
| C-upperlower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-Max.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-MAN.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-Mandible_Upper.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-p123_Maxillary_Scan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-bitescan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-occlusion.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-12_upper.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-UpperJawScan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-LowerJawScan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-LOWER.STL | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-unterkiefer_oberkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-superioreinferiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-upper-lower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-lowercase.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-uppercase.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-supply.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-info.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-insup.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-dir-upper_case/scan.stl <br>_directory name only_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-dir-lower_dir/scan.stl <br>_directory name only_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| C-seq-nohint-4 <br>_no-hint default: 4 files into one case_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 (scan replaced later) lower \| warn: provisional <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper \| warn: provisional <br>then ERR [No scan was imported. scan3.stl: both arches are already taken. Say which role this scan has.] <br>then ERR [No scan was imported. scan4.stl: both arches are already taken. Say which role this scan has.] | DIFFERENT: step2: role/arch LOWER vs upper; step3: plugin accepts, DC rejects; step4: plugin accepts, DC rejects |
| C-seq-nohint-then-upper-then-nohint <br>_scan1 (no hint), x_upper, scan2 (no hint)_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 (scan replaced later) lower \| warn: provisional <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper <br>then ERR [No scan was imported. scan2.stl: both arches are already taken. Say which role this scan has.] | DIFFERENT: step3: plugin accepts, DC rejects |

### C2. Name hint only (separated from the default role)

Plugin hint = `autoseed.arch_from_name(basename)` (None = no hint, the operator then uses LOWER). DentalCraft hint inferred by importing the file into an empty case and into a case already holding upper and lower (a hint gives the same role in both; no hint gives a provisional arch in the empty case and "no place" in the full one).

| File | Plugin hint | DentalCraft hint | Verdict |
|---|---|---|---|
| C/upper.stl | UPPER | upper | SAME |
| C/lower.stl | LOWER | lower | SAME |
| C/maxilla.stl | UPPER | upper | SAME |
| C/mandible.stl | LOWER | lower | SAME |
| C/mascellare.stl | UPPER | upper | SAME |
| C/mandibolare.stl | LOWER | lower | SAME |
| C/superiore.stl | UPPER | upper | SAME |
| C/inferiore.stl | LOWER | lower | SAME |
| C/sup.stl | UPPER | upper | SAME |
| C/inf.stl | LOWER | lower | SAME |
| C/Patient_sup.stl | UPPER | upper | SAME |
| C/Oberkiefer.stl | UPPER | upper | SAME |
| C/Unterkiefer.stl | LOWER | lower | SAME |
| C/UpperJaw.stl | UPPER | upper | SAME |
| C/lowerJaw.stl | LOWER | lower | SAME |
| C/scan_upper_lower.stl | None | None | SAME |
| C/flower.stl | None | None | SAME |
| C/superman.stl | None | None | SAME |
| C/bite.stl | None | bite | DIFFERENT |
| C/morso.stl | None | bite | DIFFERENT |
| C/scan1.stl | None | None | SAME |
| C/UPPER.STL | UPPER | upper | SAME |
| C/maxillary.stl | UPPER | upper | SAME |
| C/mandibular.stl | LOWER | lower | SAME |
| C/Arcata_Superiore.stl | UPPER | upper | SAME |
| C/arcata-inferiore.stl | LOWER | lower | SAME |
| C/UpperJaw_bite.stl | UPPER | bite | DIFFERENT |
| C/inferior.stl | LOWER | lower | SAME |
| C/superior.stl | UPPER | upper | SAME |
| C/maxill.stl | UPPER | upper | SAME |
| C/mandib.stl | LOWER | lower | SAME |
| C/Lower.ply | LOWER | lower | SAME |
| C/case_UPPER_final.obj | UPPER | upper | SAME |
| C/upperlower.stl | None | None | SAME |
| C/Max.stl | None | None | SAME |
| C/MAN.stl | None | None | SAME |
| C/Mandible_Upper.stl | None | None | SAME |
| C/p123_Maxillary_Scan.stl | UPPER | upper | SAME |
| C/bitescan.stl | None | None | SAME |
| C/occlusion.stl | None | None | SAME |
| C/12_upper.stl | UPPER | upper | SAME |
| C/UpperJawScan.stl | UPPER | upper | SAME |
| C/LowerJawScan.stl | LOWER | lower | SAME |
| C/LOWER.STL | LOWER | lower | SAME |
| C/unterkiefer_oberkiefer.stl | None | None | SAME |
| C/superioreinferiore.stl | None | None | SAME |
| C/upper-lower.stl | None | None | SAME |
| C/lowercase.stl | None | None | SAME |
| C/uppercase.stl | None | None | SAME |
| C/supply.stl | None | None | SAME |
| C/info.stl | None | None | SAME |
| C/insup.stl | None | None | SAME |
| C/upper_case/scan.stl | None | None | SAME |
| C/lower_dir/scan.stl | None | None | SAME |
| C/seq/scan1.stl | None | None | SAME |
| C/seq/scan2.stl | None | None | SAME |
| C/seq/scan3.stl | None | None | SAME |
| C/seq/scan4.stl | None | None | SAME |
| C/seq2/scan1.stl | None | None | SAME |
| C/seq2/x_upper.stl | UPPER | upper | SAME |
| C/seq2/scan2.stl | None | None | SAME |

## D. Robustness

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| D-empty0.stl <br>_0 bytes_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'empty0.stl' (python traceback to the user)] | ERR [No scan was imported. empty0.stl: the file is damaged or is not a valid scan. (invalid STL data: file too short to be an STL)] | SAME |
| D-empty0.obj <br>_0 bytes_ | EXC [uncaught RuntimeError: OBJ Import: Cannot mmap file 'empty0.obj' (python traceback to the user)] | ERR [No scan was imported. empty0.obj: the file contains no usable faces.] | SAME |
| D-empty0.ply <br>_0 bytes_ | EXC [uncaught RuntimeError: PLY Importer: empty0: Invalid PLY header. (python traceback to the user)] | ERR [No scan was imported. empty0.ply: the file is damaged or is not a valid scan. (invalid PLY data: missing `ply` magic)] | SAME |
| D-tri0_bin.stl <br>_binary STL with 0 triangles_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: tri0_bin: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. tri0_bin.stl: the file contains no usable faces.] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-solid_empty.stl <br>_ASCII STL, no facets_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'solid_empty.stl' (python traceback to the user)] | ERR [No scan was imported. solid_empty.stl: the file contains no usable faces.] | SAME |
| D-noface.obj <br>_OBJ with 3 vertices and no faces_ | v=3 f=0 wbb=(0,0,0)..(1,0,1) x1 LOWER \| WARNING: noface: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. noface.obj: the file contains no usable faces.] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-empty_comment.obj <br>_OBJ with only a comment_ | EXC [Error: Nessuna mesh importata] | ERR [No scan was imported. empty_comment.obj: the file contains no usable faces.] | SAME |
| D-face0.ply <br>_PLY with 3 vertices and 0 faces_ | v=3 f=0 wbb=(0,0,0)..(1,1,0) x1 LOWER \| WARNING: face0: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. face0.ply: the file contains no usable faces.] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-vert0.ply <br>_PLY with 0 vertices and 0 faces_ | EXC [uncaught RuntimeError: PLY Importer: failed importing, no vertices (python traceback to the user)] | ERR [No scan was imported. vert0.ply: the file contains no usable faces.] | SAME |
| D-trunc_mid.stl <br>_binary STL cut mid-triangle (misaligned), header says 11900_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: trunc_mid: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. trunc_mid.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 11900 triangles, data holds 500] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-trunc_aligned.stl <br>_binary STL cut on a triangle boundary: 5000 of 11900 declared_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: trunc_aligned: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. trunc_aligned.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 11900 triangles, data holds] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-trunc_header.stl <br>_binary STL cut inside the 80 byte header_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'trunc_header.stl' (python traceback to the user)] | ERR [No scan was imported. trunc_header.stl: the file is damaged or is not a valid scan. (invalid STL data: file too short to be an STL)] | SAME |
| D-garbage.stl <br>_random bytes_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: garbage: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. garbage.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 3717673525 triangles, data holds ] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-garbage_text.stl <br>_text garbage .stl_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: garbage_text: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. garbage_text.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 1818585098 triangles, data h] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-garbage.obj <br>_random bytes .obj_ | EXC [Error: Nessuna mesh importata] | ERR [No scan was imported. garbage.obj: the file contains no usable faces.] | SAME |
| D-garbage.ply <br>_random bytes .ply_ | EXC [uncaught RuntimeError: PLY Importer: garbage: Invalid PLY header. (python traceback to the user)] | ERR [No scan was imported. garbage.ply: the file is damaged or is not a valid scan. (invalid PLY data: missing `ply` magic)] | SAME |
| D-valid_as_txt.txt <br>_valid binary STL with .txt extension_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [No scan was imported. valid_as_txt.txt: unsupported file format. Use STL, OBJ or PLY.] | SAME |
| D-model.3mf <br>_valid minimal 3MF (tetrahedron)_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [No scan was imported. model.3mf: unsupported file format. Use STL, OBJ or PLY.] | SAME |
| D-noext <br>_valid binary STL, no extension_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [No scan was imported. noext: unsupported file format. Use STL, OBJ or PLY.] | SAME |
| D-small380.stl <br>_380 faces_ | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 lower \| warn: low_polygons,provisional | SAME |
| D-small380.obj <br>_380 faces_ | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 LOWER | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 lower \| warn: low_polygons,provisional | SAME |
| D-degen.stl <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 LOWER | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-degen.obj <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,-100,15.4263)..(110,7.224,100) x1 LOWER | v=2420 f=4738 wbb=(-31.2609,-100,15.4263)..(110,7.224,100) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-degen.ply <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 LOWER | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-dupface.stl <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 LOWER | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-dupface.obj <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,-2.3443,15.4263)..(37.0179,7.224,30.5719) x1 LOWER | v=2400 f=4720 wbb=(-31.2609,-2.3443,15.4263)..(37.0179,7.224,30.5719) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-dupface.ply <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 LOWER | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 lower \| warn: bad_faces_removed,provisional | SAME |
| D-jitter2e5.stl <br>_unwelded STL, shared corners jittered by up to 2e-5 mm_ | v=35699 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=13665 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5754,2.3496) x1 lower \| warn: provisional | DIFFERENT: step1: verts 35699 vs 13665 |
| D-jitter1e3.stl <br>_unwelded STL, shared corners jittered by up to 1e-3 mm_ | v=35700 f=11900 wbb=(-31.2599,15.4252,-7.2265)..(37.0191,30.5763,2.3503) x1 LOWER | v=35695 f=11900 wbb=(-31.2599,15.4252,-7.2265)..(37.0191,30.5763,2.3503) x1 lower \| warn: provisional | DIFFERENT: step1: verts 35700 vs 35695 |
| D-multiobj.obj <br>_o a (1800v,3540f) + o b shifted +100 in x (1800v,3540f)_ | v=1800 f=3540 wbb=(-31.2605,-2.3253,15.4256)..(37.0199,7.21,30.5752) x1 LOWER +1 extra objs | v=3600 f=7080 wbb=(-31.2605,-2.3253,15.4256)..(137.0199,7.21,30.5752) x1 lower \| warn: obj_objects_merged,provisional | DIFFERENT: step1: verts 1800 vs 3600; step1: tris 3540 vs 7080; step1: world bbox differs; step1: plugin leaves 1 extra unprocessed mesh object(s) |
| D-colnrm_ascii.ply <br>_PLY with vertex colours (uchar) and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-colnrm_le.ply <br>_PLY with vertex colours (uchar) and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-colnrm_be.ply <br>_PLY BE with colours and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-scanU.STL <br>_uppercase ext_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-scanU.OBJ <br>_uppercase ext_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 lower \| warn: provisional | SAME |
| D-scanU.Ply <br>_mixed-case ext_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-quads_vtvn.obj <br>_1200 verts, 1170 quads with v/vt/vn -> 2340 triangles_ | v=1200 f=1170(tri 2340) wbb=(-31.2605,-2.3253,15.4256)..(37.0199,7.2107,30.5756) x1 LOWER | v=1200 f=2340 wbb=(-31.2605,-2.3253,15.4256)..(37.0199,7.2107,30.5756) x1 lower \| warn: provisional | SAME |
| D-negidx.obj <br>_OBJ with negative (relative) indices, 380 faces_ | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 LOWER | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 lower \| warn: low_polygons,provisional | SAME |
| D-vcol.obj <br>_OBJ with v x y z r g b_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 lower \| warn: provisional | SAME |
| D-nan.stl <br>_one NaN coordinate_ | v=6002 f=11900 wbb=(-31.2589,0,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | ERR [No scan was imported. nan.stl: the file is damaged or is not a valid scan. (invalid STL data: non-finite vertex coordinate)] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-inf.obj <br>_one inf coordinate (OBJ text "inf")_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,0)..(37.0185,7.2263,30.5755) x1 LOWER | ERR [No scan was imported. inf.obj: the file is damaged or is not a valid scan. (invalid OBJ data: line 11: bad number)] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-badidx.obj <br>_face index out of range_ | v=2 f=0 wbb=(0,0,0)..(1,0,0) x1 LOWER \| WARNING: badidx: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [No scan was imported. badidx.obj: the file is damaged or is not a valid scan. (invalid OBJ data: line 4: face index 9 out of range (vertices: 3))] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-solidhdr.stl <br>_binary STL whose header starts with solid_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower \| warn: provisional | SAME |
| D-crlf.stl <br>_ASCII STL, CRLF, 380 faces_ | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 lower \| warn: low_polygons,provisional | SAME |

## E. Multi-file and re-import

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| E-same-role-twice <br>_two different meshes, both named upper: a (11900f) then b (6320f)_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 upper REPLACED-existing \| warn: replaced | DIFFERENT: final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-same-file-twice <br>_same file imported twice_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper REPLACED-existing \| warn: replaced | DIFFERENT: final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-upper-lower-pair <br>_upper then lower, different shapes/offsets_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 LOWER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 lower | SAME |
| E-upper-lower-upper <br>_upper, lower, upper again_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 LOWER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 (scan replaced later) lower <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 upper REPLACED-existing \| warn: replaced | DIFFERENT: final case holds 3 scan objects (plugin) vs 2 scans (DC) |
| E-good-upper-then-empty-upper-obj <br>_valid upper, then comment-only OBJ named upper_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then EXC [Error: Nessuna mesh importata] | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [No scan was imported. upper_empty.obj: the file contains no usable faces.] | SAME |
| E-good-upper-then-garbage-upper-obj <br>_valid upper, then random bytes named upper_garbage.obj_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then EXC [Error: Nessuna mesh importata] | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [No scan was imported. upper_garbage.obj: the file contains no usable faces.] | SAME |
| E-good-upper-then-garbage-upper-stl <br>_valid upper, then random bytes named upper_garbage.stl_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=0 f=0 wbb=none..none x1 UPPER \| WARNING: upper_garbage: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [No scan was imported. upper_garbage.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 3717673525 triangles, data ] | DIFFERENT: step2: plugin accepts, DC rejects; final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-good-upper-then-truncated-upper-stl <br>_valid upper, then truncated binary STL named upper_trunc.stl_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=0 f=0 wbb=none..none x1 UPPER \| WARNING: upper_trunc: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [No scan was imported. upper_trunc.stl: the file is damaged or is not a valid scan. (invalid STL data: truncated binary STL: header announces 11900 triangles, data holds 5] | DIFFERENT: step2: plugin accepts, DC rejects; final case holds 2 scan objects (plugin) vs 1 scans (DC) |

## R. Real scans (4 STL, read in place; measurements only)

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| R-orig-1 <br>_real scan, original name (read in place)_ | v=429605 f=855739 wbb=[redacted] x1 LOWER | v=429605 f=855739 wbb=[redacted] x1 lower | SAME |
| R-orig-2 <br>_real scan, original name (read in place)_ | v=402802 f=802606 wbb=[redacted] x1 UPPER | v=402802 f=802606 wbb=[redacted] x1 upper | SAME |
| R-orig-3 <br>_real scan, original name (read in place)_ | v=109716 f=215588 wbb=[redacted] x1 LOWER | v=109716 f=215588 wbb=[redacted] x1 lower | SAME |
| R-orig-4 <br>_real scan, original name (read in place)_ | v=100094 f=197756 wbb=[redacted] x1 UPPER | v=100094 f=197756 wbb=[redacted] x1 upper | SAME |
| R-neutral-scan_a.stl <br>_symlink scan_a.stl -> real scan 1_ | v=429605 f=855739 wbb=[redacted] x1 LOWER | v=429605 f=855739 wbb=[redacted] x1 lower \| warn: provisional | SAME |
| R-neutral-scan_b.stl <br>_symlink scan_b.stl -> real scan 2_ | v=402802 f=802606 wbb=[redacted] x1 LOWER | v=402802 f=802606 wbb=[redacted] x1 lower \| warn: provisional | SAME |
| R-neutral-scan_c.stl <br>_symlink scan_c.stl -> real scan 3_ | v=109716 f=215588 wbb=[redacted] x1 LOWER | v=109716 f=215588 wbb=[redacted] x1 lower \| warn: provisional | SAME |
| R-neutral-scan_d.stl <br>_symlink scan_d.stl -> real scan 4_ | v=100094 f=197756 wbb=[redacted] x1 LOWER | v=100094 f=197756 wbb=[redacted] x1 lower \| warn: provisional | SAME |

### R2. Plugin geometric `detect_arch` on the neutral-named copies (name=scan_x.stl, no hint)

Neutral copies: scan_a to scan_d are symlinks to the four real scans in sorted file-name order (`add_real.py`); the true arch column assumes the order lower, upper, lower, upper. Operator arch = what `import_scans` assigns at import (no detection there); detect_arch = `autoseed.detect_arch(world verts, tris, name=...)` called as `case_auto` does in phase 2. The DentalCraft columns come from `aligners.case.import-scans` (`DC_IMPORT_CMD`, work folder in `IMPORT_DIFF_WORK_SCANS` when set): arch, confidence and cue of its detection at import. Plugin confidence and DentalCraft confidence are the same numbers.

| Neutral file | True arch | Operator arch at import | detect_arch arch | confidence | source (fonte) | detect time s | DentalCraft role (import-scans) | DC confidence | DC cue |
|---|---|---|---|---|---|---|---|---|---|
| scan_a.stl | lower | LOWER | LOWER | 0.80 | curva di Wilson: cuspidi piu alte dei molari vestibolari (arcata inferiore) | 1.535 | lower (detected) | 0.80 | curve_of_wilson |
| scan_b.stl | upper | LOWER | UPPER | 1.00 | palato presente | 1.261 | upper (detected) | 1.00 | palate |
| scan_c.stl | lower | LOWER | LOWER | 0.90 | curva di Wilson: cuspidi piu alte dei molari vestibolari (arcata inferiore) | 0.396 | lower (detected) | 0.90 | curve_of_wilson |
| scan_d.stl | upper | LOWER | UPPER | 0.80 | orientamento dello scanner (denti rivolti in basso) | 0.358 | upper (detected) | 0.80 | scanner_orientation |

### R3. Import time, largest real scan (scan_a, 42787034 bytes, 429605 v, 855739 f)

| Side | What was timed | Runs (s) |
|---|---|---|
| Plugin | `bpy.ops.oa.import_scans` (Blender STL import + `_import_mesh` + `fix_units` + `create_case`), in-process, warm, 5 runs, Blender 5.2.2 headless | 0.581, 0.579, 0.544, 0.563, 0.591 (first run of the full suite: 0.565) |
| DentalCraft release, `case.import` | wall time of `dentalcraft-cli run project.new + case.import` (process start included; baseline project.new only: 0.003). Read, weld, units and cleanup of bad faces, no detection | 0.303, 0.298, 0.314, 0.298, 0.329 |
| DentalCraft release, `aligners.case.import-scans` | same with `aligners.case.import-scans`: adds the geometric arch detection (about 0.3 s) | 0.643, 0.635, 0.619, 0.612, 0.632 |
| DentalCraft before the shared pipeline (no cleanup, no detection) | release, `case.import` as recorded earlier | 0.305, 0.331, 0.302, 0.303, 0.305 |
Other real scans, plugin operator time (single run): scan_b 0.52 s, scan_c 0.10 s, scan_d 0.08 s. The R3 timings were recorded with a separate timing script that is not part of this folder; they are kept as recorded.

