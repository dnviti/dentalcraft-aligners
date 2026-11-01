# Appendix: observations (measured unless marked "by code" or "inferred")

1. OBJ axis. Plugin world coordinates of an OBJ = (x, -z, y) of the file coordinates (Blender OBJ importer default forward -Z / up Y, kept as `matrix_world` rotation; mesh data stays in file coordinates). DentalCraft keeps file coordinates. Measured on A-obj: file bbox (-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062); plugin world (-33.8424,-9.0062,-2.4837)..(33.8424,11.9975,51.7909); DentalCraft identical to the file. STL and PLY: no remap on either side.
2. Units (`fix_units` vs `normalize_units`). Plugin: 0.02<=size<=0.2 -> x1000, 2<=size<=20 -> x10, else x1 (no micrometre rule). DentalCraft: same two ranges plus 20000..200000 -> x0.001 (um). Exact-boundary extents differ because the plugin compares the float32 `dimensions` with double literals: extent f32(0.02) and f32(0.2) are NOT rescaled by the plugin (stay 0.02 mm / 0.2 mm) and ARE rescaled by DentalCraft (20 mm / 200 mm). 2.0 and 20.0 behave the same on both (x10). Extents 60000, 20000, 200000: plugin x1 (a 60 m model), DentalCraft x0.001 (60 mm). 15 -> x10 (150 mm) on both, 3 -> x10 on both, 2.4 -> x10 (24 mm) on both ("inch" scans are treated as cm on both).
3. Messages. DentalCraft `case.import` returns only `source_units`; no warning or info text for any unit case. Plugin emits INFO "riscalato xN" when rescaled and the WARNING "Il modello e molto piccolo" when `max(obj.dimensions) < 5`. That warning is evaluated on STALE `obj.dimensions` right after `fix_units` (measured with a one-off probe script, not included: dimensions stay 0.06 until `view_layer.update()`), so the plugin shows the "very small" warning on models it has just rescaled to 60 mm (B-0.06, B-2.4, B-2.0, B-3). Measured headless; the GUI path runs the same calls in the same order (inferred, not run).
4. Role detection. Plugin: whole-word tokens after camelCase split (+ `upperjaw`, `lowerjaw`, `maxill*`, `mandib*` substrings), a name with both an upper and a lower token gives None; no bite role; no-hint default is always LOWER whatever the case holds. DentalCraft: lowercase substring match on `bite|morso`, then `upper|maxil|superior`, then `lower|mandib|inferior`; no-hint default = first free of upper, lower, bite (upper when all taken). Directory names are ignored by both (the plugin passes the basename).
5. Duplicates and degenerate faces. Blender importers clean the mesh (removed the 50 repeated-index faces, and all 300 duplicate/reversed duplicate faces: 4738 vs 4788 and 4720 vs 5020). DentalCraft keeps them (4788 and 5020 triangles).
6. Weld. Both weld bit-identical STL corners. DentalCraft also welds within 1e-6 x bbox diagonal (cell hash), the plugin does not: A-stl-bin 83171 vs 83172 vertices (two source vertices 7.4e-5 mm apart merged, 2 faces become degenerate and are kept: faces stay 166392); jitter 2e-5 mm: 13665 vs 35699 vertices (partial, cell-boundary dependent merging); jitter 1e-3 mm: 35695 vs 35700.
7. Failure handling. Plugin: an empty file (STL, OBJ, PLY), a 60-byte STL, an ASCII STL with no facets, a PLY with 0 vertices and random-byte PLY raise an uncaught RuntimeError from Blender's importer inside `_import_mesh` (Python traceback shown to the user, no clean message); a comment-only OBJ and random-byte OBJ give the clean "Nessuna mesh importata". The plugin ACCEPTS: truncated binary STL (11900 declared, 5000 present, with and without triangle alignment), random bytes and text named .stl (all become an empty 0-vertex object turned into a case, with the misleading "very small" warning), NaN coordinate (6002 vertices, bbox min y shown as 0 instead of 15.43), an OBJ vertex "inf" (bbox min shown as 0 instead of 15.43, no error) and an OBJ whose face index is out of range (imported as 2 vertices, 0 faces). DentalCraft rejects all of those with a clear message (truncated, garbage STL, non-finite, bad number, index out of range). DentalCraft ACCEPTS as an EMPTY scan (0 v, 0 f, no bbox): a comment-only OBJ, random bytes named .obj, an ASCII STL `solid x/endsolid x` and a 0-vertex PLY. An empty scan whose name hints at a role already taken REPLACES the valid scan (E-good-upper-then-empty-upper-obj: `replaced: true`; undoable); the plugin keeps the first scan and reports the error. Extensions: .txt, .3mf, no extension are rejected by both; uppercase and mixed-case extensions work on both.
8. Multi-object OBJ. Plugin takes only the first object (`new[0]`, alphabetical) and leaves the other mesh object in the scene unprocessed (1800 v / 3540 f processed). DentalCraft merges all objects into one scan (3600 v / 7080 f, bbox spans both).
9. Same role twice / same file twice. Plugin keeps every object as a separate case. DentalCraft keeps one scan per role; the later import replaces the earlier (`replaced: true`, no prompt).
10. Workspace (GUI) path, by code only (`dentalcraft-app/crates/dc-workspace/src/import.rs`): same `role_from_name`, same read/normalize_units, so the geometry results above apply. Role assignment differs from the CLI: `assign_roles` reserves hinted roles for the whole batch first and gives unhinted files the first free role, so [scan1.stl, x_upper.stl] gives scan1=lower, x_upper=upper, whereas the CLI/session path (sequential `case.import`) gives scan1=upper then x_upper replaces it. The workspace read is all-or-nothing (first unreadable file aborts the batch); the plugin loop is per file (an uncaught exception aborts the operator but files already imported stay).

11. Real scans (group R, 4 STL, mm, 100k-430k vertices; file names and coordinates are not recorded here). Vertices, faces and bbox agree on both sides for all 8 runs (original and neutral names, units x1 on both): scan_a 429605 v / 855739 f; scan_b 402802 / 802606; scan_c 109716 / 215588; scan_d 100094 / 197756. Original names: both give the right arch (the name hints match the true arch). Neutral names: no hint on either side; the plugin import assigns LOWER to all four (2 wrong), DentalCraft assigns upper to all four when imported as the first scan of a fresh case (2 wrong). Plugin `detect_arch` on the neutral copies then finds the correct arch on all four (palate, Wilson curve, scanner orientation); DentalCraft has no geometric detection.

# Scan import: Blender plugin (oracle) vs DentalCraft

Generated by `report.py` from the `rows.json` that `analyze.py` builds from the plugin and DentalCraft runs.

## Method

- Plugin: the real operator `bpy.ops.oa.import_scans(filepath=...)`, run headless in Blender 5.2.2 (`--background --factory-startup`, user dirs inside the work folder). Add-on loaded from its path via `sys.path` and `register()`; not installed. One fresh scene per case (all objects and meshes removed, `oa_world` cleared). `report()` calls were captured by wrapping `execute` with a proxy (reports are forwarded, behaviour unchanged). Arch = `obj.oa.arch`. World bbox = `matrix_world @ vertices` of the first imported mesh object.
- DentalCraft: `dentalcraft-cli run --cmd project.new --cmd case.import={path} ... --save --print` (session/CLI path, `dc-session` `case.import`; debug build of `dentalcraft-app/target/debug/dentalcraft-cli`, built inside the superproject). World bbox = scan `aabb` from the case summary; `import_xform` and `world_frame` are identity after import, so scan coordinates are world coordinates. The CLI aborts on the first failing command, so one process per file, imports replayed cumulatively.
- The DentalCraft workspace (GUI) path in `dc-workspace/src/import.rs` was NOT run: it is described "by code" in the notes of the summary.
- v = vertices, f = faces/polygons as reported by each side, tri = triangle count when different from f. wbb = world bbox min..max in mm after import. `xN` = unit factor (plugin: parsed from the INFO message "riscalato xN", 1 when no message; DentalCraft: `source_units`, m=1000, cm=10, mm=1, um=0.001).
- Fixtures are synthetic only (`gen.py`, `fixtures/`). Group A: `demo_upper_mild` arch from `dentalcraft-aligners/golden/fixtures` (83172 v, 166392 f) written as six formats. Groups B-E: a generated asymmetric horseshoe tube (6000 v, 11900 f) and variants.


## A. Same mesh, different formats

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| A-stl-bin | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83171 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | DIFFERENT: step1: verts 83172 vs 83171 |
| A-stl-ascii | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83171 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | DIFFERENT: step1: verts 83172 vs 83171 |
| A-obj | v=83172 f=166392 wbb=(-33.8424,-9.0062,-2.4837)..(33.8424,11.9975,51.7909) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| A-ply-ascii | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | SAME |
| A-ply-bin-le | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | SAME |
| A-ply-bin-be | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 LOWER | v=83172 f=166392 wbb=(-33.8424,-2.4837,-11.9975)..(33.8424,51.7909,9.0062) x1 upper | SAME |

## B. Units

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| B-0.06 <br>_largest extent 0.06_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1000 LOWER \| WARNING: u_0.06: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_0.06: riscalato x1000 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1000 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-6 <br>_largest extent 6.0_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x10 LOWER \| INFO: u_6: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-60 <br>_largest extent 60.0_ | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x1 upper | SAME |
| B-60000 <br>_largest extent 60000.0_ | v=6000 f=11900 wbb=(0,0,0)..(60000.00,13312.76,8415.0205) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(60,13.3128,8.415) x0.001 upper | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-2.4 <br>_largest extent 2.4_ | v=6000 f=11900 wbb=(0,0,0)..(24,5.3251,3.366) x10 LOWER \| WARNING: u_2.4: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.4: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(24,5.3251,3.366) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-0.02 <br>_largest extent 0.02_ | v=6000 f=11900 wbb=(0,0,0)..(0.02,0.0044,0.0028) x1 LOWER \| WARNING: u_0.02: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x1000 upper | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 1000 |
| B-0.2 <br>_largest extent 0.2_ | v=6000 f=11900 wbb=(0,0,0)..(0.2,0.0444,0.0281) x1 LOWER \| WARNING: u_0.2: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x1000 upper | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 1000 |
| B-2.0 <br>_largest extent 2.0_ | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x10 LOWER \| WARNING: u_2.0: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.0: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-20.0 <br>_largest extent 20.0_ | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x10 LOWER \| INFO: u_20.0: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-15 <br>_largest extent 15.0_ | v=6000 f=11900 wbb=(0,0,0)..(150,33.2819,21.0376) x10 LOWER \| INFO: u_15: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(150,33.2819,21.0376) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-3 <br>_largest extent 3.0_ | v=6000 f=11900 wbb=(0,0,0)..(30,6.6564,4.2075) x10 LOWER \| WARNING: u_3: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_3: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(30,6.6564,4.2075) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-0.019 <br>_largest extent 0.019_ | v=6000 f=11900 wbb=(0,0,0)..(0.019,0.0042,0.0027) x1 LOWER \| WARNING: u_0.019: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.019,0.0042,0.0027) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-0.21 <br>_largest extent 0.21_ | v=6000 f=11900 wbb=(0,0,0)..(0.21,0.0466,0.0295) x1 LOWER \| WARNING: u_0.21: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.21,0.0466,0.0295) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-1.9 <br>_largest extent 1.9_ | v=6000 f=11900 wbb=(0,0,0)..(1.9,0.4216,0.2665) x1 LOWER \| WARNING: u_1.9: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(1.9,0.4216,0.2665) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-21 <br>_largest extent 21.0_ | v=6000 f=11900 wbb=(0,0,0)..(21,4.6595,2.9453) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(21,4.6595,2.9453) x1 upper | SAME |
| B-100 <br>_largest extent 100.0_ | v=6000 f=11900 wbb=(0,0,0)..(100,22.1879,14.025) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(100,22.1879,14.025) x1 upper | SAME |
| B-19999 <br>_largest extent 19999.0_ | v=6000 f=11900 wbb=(0,0,0)..(19999.00,4437.3643,2804.8667) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(19999.00,4437.3643,2804.8667) x1 upper | SAME |
| B-20000 <br>_largest extent 20000.0_ | v=6000 f=11900 wbb=(0,0,0)..(20000.00,4437.5864,2805.0071) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(20,4.4376,2.805) x0.001 upper | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-200000 <br>_largest extent 200000.0_ | v=6000 f=11900 wbb=(0,0,0)..(200000.00,44375.86,28050.07) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(200,44.3759,28.0501) x0.001 upper | DIFFERENT: step1: world bbox differs; step1: unit factor 1 vs 0.001 |
| B-200001 <br>_largest extent 200001.0_ | v=6000 f=11900 wbb=(0,0,0)..(200001.00,44376.09,28050.21) x1 LOWER | v=6000 f=11900 wbb=(0,0,0)..(200001.00,44376.09,28050.21) x1 upper | SAME |
| B-0.0199 <br>_largest extent 0.0199_ | v=6000 f=11900 wbb=(0,0,0)..(0.0199,0.0044,0.0028) x1 LOWER \| WARNING: u_0.0199: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(0,0,0)..(0.0199,0.0044,0.0028) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| B-2.0001 <br>_largest extent 2.0001_ | v=6000 f=11900 wbb=(0,0,0)..(20.001,4.4378,2.8051) x10 LOWER \| WARNING: u_2.0001: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). \| INFO: u_2.0001: riscalato x10 (unita convertite in millimetri) | v=6000 f=11900 wbb=(0,0,0)..(20.001,4.4378,2.8051) x10 upper | SAME geometry and factor; plugin also emits a message, DC emits none |

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
| C-inf.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-Patient_sup.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-Oberkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-Unterkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-UpperJaw.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-lowerJaw.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-scan_upper_lower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-flower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-superman.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-bite.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch LOWER vs bite |
| C-morso.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch LOWER vs bite |
| C-scan1.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
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
| C-case_UPPER_final.obj | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| C-upperlower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-Max.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-MAN.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-Mandible_Upper.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-p123_Maxillary_Scan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-bitescan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 bite | DIFFERENT: step1: role/arch LOWER vs bite |
| C-occlusion.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-12_upper.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-UpperJawScan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| C-LowerJawScan.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-LOWER.STL | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-unterkiefer_oberkiefer.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-superioreinferiore.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-upper-lower.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-lowercase.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | SAME |
| C-uppercase.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-supply.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-info.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-insup.stl | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-dir-upper_case/scan.stl <br>_directory name only_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-dir-lower_dir/scan.stl <br>_directory name only_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| C-seq-nohint-4 <br>_no-hint default: 4 files into one case_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 (scan replaced later) upper <br>then v=6000 f=11900 (scan replaced later) lower <br>then v=6000 f=11900 (scan replaced later) bite <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper REPLACED-existing | DIFFERENT: step1: role/arch LOWER vs upper; step3: role/arch LOWER vs bite; step4: role/arch LOWER vs upper |
| C-seq-nohint-then-upper-then-nohint <br>_scan1 (no hint), x_upper, scan2 (no hint)_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 UPPER <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 (scan replaced later) upper <br>then v=6000 f=11900 (scan replaced later) upper REPLACED-existing <br>then v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 lower | DIFFERENT: step1: role/arch LOWER vs upper |

### C2. Name hint only (separated from the default role)

Plugin hint = `autoseed.arch_from_name(basename)` (None = no hint, the operator then uses LOWER). DentalCraft hint inferred by importing the file into an empty case and into a case already holding upper and lower (no hint gives upper, then bite; a hint gives the same role in both).

| File | Plugin hint | DentalCraft hint | Verdict |
|---|---|---|---|
| C/upper.stl | UPPER | upper | SAME |
| C/lower.stl | LOWER | lower | SAME |
| C/maxilla.stl | UPPER | upper | SAME |
| C/mandible.stl | LOWER | lower | SAME |
| C/mascellare.stl | UPPER | None | DIFFERENT |
| C/mandibolare.stl | LOWER | lower | SAME |
| C/superiore.stl | UPPER | upper | SAME |
| C/inferiore.stl | LOWER | lower | SAME |
| C/sup.stl | UPPER | None | DIFFERENT |
| C/inf.stl | LOWER | None | DIFFERENT |
| C/Patient_sup.stl | UPPER | None | DIFFERENT |
| C/Oberkiefer.stl | UPPER | None | DIFFERENT |
| C/Unterkiefer.stl | LOWER | None | DIFFERENT |
| C/UpperJaw.stl | UPPER | upper | SAME |
| C/lowerJaw.stl | LOWER | lower | SAME |
| C/scan_upper_lower.stl | None | upper | DIFFERENT |
| C/flower.stl | None | lower | DIFFERENT |
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
| C/upperlower.stl | None | upper | DIFFERENT |
| C/Max.stl | None | None | SAME |
| C/MAN.stl | None | None | SAME |
| C/Mandible_Upper.stl | None | upper | DIFFERENT |
| C/p123_Maxillary_Scan.stl | UPPER | upper | SAME |
| C/bitescan.stl | None | bite | DIFFERENT |
| C/occlusion.stl | None | None | SAME |
| C/12_upper.stl | UPPER | upper | SAME |
| C/UpperJawScan.stl | UPPER | upper | SAME |
| C/LowerJawScan.stl | LOWER | lower | SAME |
| C/LOWER.STL | LOWER | lower | SAME |
| C/unterkiefer_oberkiefer.stl | None | None | SAME |
| C/superioreinferiore.stl | None | upper | DIFFERENT |
| C/upper-lower.stl | None | upper | DIFFERENT |
| C/lowercase.stl | None | lower | DIFFERENT |
| C/uppercase.stl | None | upper | DIFFERENT |
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
| D-empty0.stl <br>_0 bytes_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'empty0.stl' (python traceback to the user)] | ERR [empty0.stl: invalid STL data: file too short to be an STL] | SAME |
| D-empty0.obj <br>_0 bytes_ | EXC [uncaught RuntimeError: OBJ Import: Cannot mmap file 'empty0.obj' (python traceback to the user)] | v=0 f=0 wbb=none(empty) x1 upper | DIFFERENT: step1: plugin rejects, DC accepts |
| D-empty0.ply <br>_0 bytes_ | EXC [uncaught RuntimeError: PLY Importer: empty0: Invalid PLY header. (python traceback to the user)] | ERR [empty0.ply: invalid PLY data: missing `ply` magic] | SAME |
| D-tri0_bin.stl <br>_binary STL with 0 triangles_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: tri0_bin: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=0 f=0 wbb=none(empty) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| D-solid_empty.stl <br>_ASCII STL, no facets_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'solid_empty.stl' (python traceback to the user)] | v=0 f=0 wbb=none(empty) x1 upper | DIFFERENT: step1: plugin rejects, DC accepts |
| D-noface.obj <br>_OBJ with 3 vertices and no faces_ | v=3 f=0 wbb=(0,0,0)..(1,0,1) x1 LOWER \| WARNING: noface: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=3 f=0 wbb=(0,0,0)..(1,1,0) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-empty_comment.obj <br>_OBJ with only a comment_ | EXC [Error: Nessuna mesh importata] | v=0 f=0 wbb=none(empty) x1 upper | DIFFERENT: step1: plugin rejects, DC accepts |
| D-face0.ply <br>_PLY with 3 vertices and 0 faces_ | v=3 f=0 wbb=(0,0,0)..(1,1,0) x1 LOWER \| WARNING: face0: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=3 f=0 wbb=(0,0,0)..(1,1,0) x1 upper | SAME geometry and factor; plugin also emits a message, DC emits none |
| D-vert0.ply <br>_PLY with 0 vertices and 0 faces_ | EXC [uncaught RuntimeError: PLY Importer: failed importing, no vertices (python traceback to the user)] | v=0 f=0 wbb=none(empty) x1 upper | DIFFERENT: step1: plugin rejects, DC accepts |
| D-trunc_mid.stl <br>_binary STL cut mid-triangle (misaligned), header says 11900_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: trunc_mid: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [trunc_mid.stl: invalid STL data: truncated binary STL: header announces 11900 triangles, data holds 5000] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-trunc_aligned.stl <br>_binary STL cut on a triangle boundary: 5000 of 11900 declared_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: trunc_aligned: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [trunc_aligned.stl: invalid STL data: truncated binary STL: header announces 11900 triangles, data holds 5000] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-trunc_header.stl <br>_binary STL cut inside the 80 byte header_ | EXC [uncaught RuntimeError: STL Import: Failed to read file 'trunc_header.stl' (python traceback to the user)] | ERR [trunc_header.stl: invalid STL data: file too short to be an STL] | SAME |
| D-garbage.stl <br>_random bytes_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: garbage: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [garbage.stl: invalid STL data: truncated binary STL: header announces 3717673525 triangles, data holds 80] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-garbage_text.stl <br>_text garbage .stl_ | v=0 f=0 wbb=none..none x1 LOWER \| WARNING: garbage_text: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [garbage_text.stl: invalid STL data: truncated binary STL: header announces 1818585098 triangles, data holds 10] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-garbage.obj <br>_random bytes .obj_ | EXC [Error: Nessuna mesh importata] | v=0 f=0 wbb=none(empty) x1 upper | DIFFERENT: step1: plugin rejects, DC accepts |
| D-garbage.ply <br>_random bytes .ply_ | EXC [uncaught RuntimeError: PLY Importer: garbage: Invalid PLY header. (python traceback to the user)] | ERR [garbage.ply: invalid PLY data: missing `ply` magic] | SAME |
| D-valid_as_txt.txt <br>_valid binary STL with .txt extension_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [valid_as_txt.txt: unsupported mesh format: valid_as_txt.txt] | SAME |
| D-model.3mf <br>_valid minimal 3MF (tetrahedron)_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [model.3mf: unsupported mesh format: model.3mf] | SAME |
| D-noext <br>_valid binary STL, no extension_ | EXC [Error: Nessun file STL/OBJ/PLY] | ERR [noext: unsupported mesh format: noext] | SAME |
| D-small380.stl <br>_380 faces_ | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 upper | SAME |
| D-small380.obj <br>_380 faces_ | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-degen.stl <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 LOWER | v=2420 f=4788 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 upper | DIFFERENT: step1: tris 4738 vs 4788 |
| D-degen.obj <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,-100,15.4263)..(110,7.224,100) x1 LOWER | v=2420 f=4788 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 upper | DIFFERENT: step1: tris 4738 vs 4788; step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-degen.ply <br>_4720 good + 50 repeated-index + 18 collinear faces; 2420 verts (20 isolated/collinear)_ | v=2420 f=4738 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 LOWER | v=2420 f=4788 wbb=(-31.2609,15.4263,-7.224)..(110,100,100) x1 upper | DIFFERENT: step1: tris 4738 vs 4788 |
| D-dupface.stl <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 LOWER | v=2400 f=5020 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 upper | DIFFERENT: step1: tris 4720 vs 5020 |
| D-dupface.obj <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,-2.3443,15.4263)..(37.0179,7.224,30.5719) x1 LOWER | v=2400 f=5020 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 upper | DIFFERENT: step1: tris 4720 vs 5020; step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-dupface.ply <br>_4720 faces + 200 identical duplicates + 100 reversed duplicates_ | v=2400 f=4720 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 LOWER | v=2400 f=5020 wbb=(-31.2609,15.4263,-7.224)..(37.0179,30.5719,2.3443) x1 upper | DIFFERENT: step1: tris 4720 vs 5020 |
| D-jitter2e5.stl <br>_unwelded STL, shared corners jittered by up to 2e-5 mm_ | v=35699 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=13665 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5754,2.3496) x1 upper | DIFFERENT: step1: verts 35699 vs 13665 |
| D-jitter1e3.stl <br>_unwelded STL, shared corners jittered by up to 1e-3 mm_ | v=35700 f=11900 wbb=(-31.2599,15.4252,-7.2265)..(37.0191,30.5763,2.3503) x1 LOWER | v=35695 f=11900 wbb=(-31.2599,15.4252,-7.2265)..(37.0191,30.5763,2.3503) x1 upper | DIFFERENT: step1: verts 35700 vs 35695 |
| D-multiobj.obj <br>_o a (1800v,3540f) + o b shifted +100 in x (1800v,3540f)_ | v=1800 f=3540 wbb=(-31.2605,-2.3253,15.4256)..(37.0199,7.21,30.5752) x1 LOWER +1 extra objs | v=3600 f=7080 wbb=(-31.2605,15.4256,-7.21)..(137.0199,30.5752,2.3253) x1 upper | DIFFERENT: step1: verts 1800 vs 3600; step1: tris 3540 vs 7080; step1: world bbox differs; step1: plugin leaves 1 extra unprocessed mesh object(s) |
| D-colnrm_ascii.ply <br>_PLY with vertex colours (uchar) and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-colnrm_le.ply <br>_PLY with vertex colours (uchar) and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-colnrm_be.ply <br>_PLY BE with colours and normals_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-scanU.STL <br>_uppercase ext_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-scanU.OBJ <br>_uppercase ext_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-scanU.Ply <br>_mixed-case ext_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-quads_vtvn.obj <br>_1200 verts, 1170 quads with v/vt/vn -> 2340 triangles_ | v=1200 f=1170(tri 2340) wbb=(-31.2605,-2.3253,15.4256)..(37.0199,7.2107,30.5756) x1 LOWER | v=1200 f=2340 wbb=(-31.2605,15.4256,-7.2107)..(37.0199,30.5756,2.3253) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-negidx.obj <br>_OBJ with negative (relative) indices, 380 faces_ | v=200 f=380 wbb=(-31.2355,-2.3228,15.4376)..(36.9895,7.1672,30.5316) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-vcol.obj <br>_OBJ with v x y z r g b_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,15.4261)..(37.0185,7.2263,30.5755) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | DIFFERENT: step1: OBJ axis: plugin world=(x,-z,y) of file coords, DC keeps file coords (confirmed on bbox) |
| D-nan.stl <br>_one NaN coordinate_ | v=6002 f=11900 wbb=(-31.2589,0,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | ERR [nan.stl: invalid STL data: non-finite vertex coordinate] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-inf.obj <br>_one inf coordinate (OBJ text "inf")_ | v=6000 f=11900 wbb=(-31.2589,-2.3496,0)..(37.0185,7.2263,30.5755) x1 LOWER | ERR [inf.obj: invalid OBJ data: line 11: bad number] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-badidx.obj <br>_face index out of range_ | v=2 f=0 wbb=(0,0,0)..(1,0,0) x1 LOWER \| WARNING: badidx: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | ERR [badidx.obj: invalid OBJ data: line 4: face index 9 out of range (vertices: 3)] | DIFFERENT: step1: plugin accepts, DC rejects |
| D-solidhdr.stl <br>_binary STL whose header starts with solid_ | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 LOWER | v=6000 f=11900 wbb=(-31.2589,15.4261,-7.2263)..(37.0185,30.5755,2.3496) x1 upper | SAME |
| D-crlf.stl <br>_ASCII STL, CRLF, 380 faces_ | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 LOWER | v=200 f=380 wbb=(-31.2355,15.4376,-7.1672)..(36.9895,30.5316,2.3228) x1 upper | SAME |

## E. Multi-file and re-import

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| E-same-role-twice <br>_two different meshes, both named upper: a (11900f) then b (6320f)_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 upper REPLACED-existing | DIFFERENT: final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-same-file-twice <br>_same file imported twice_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper REPLACED-existing | DIFFERENT: final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-upper-lower-pair <br>_upper then lower, different shapes/offsets_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 LOWER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 lower | SAME |
| E-upper-lower-upper <br>_upper, lower, upper again_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 LOWER <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 UPPER | v=6000 f=11900 (scan replaced later) upper <br>then v=3200 f=6320 (scan replaced later) lower <br>then v=3200 f=6320 wbb=(-24.2609,20.4263,4.7752)..(44.0179,35.5713,14.3443) x1 upper REPLACED-existing | DIFFERENT: final case holds 3 scan objects (plugin) vs 2 scans (DC) |
| E-good-upper-then-empty-upper-obj <br>_valid upper, then comment-only OBJ named upper_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then EXC [Error: Nessuna mesh importata] | v=6000 f=11900 (scan replaced later) upper <br>then v=0 f=0 wbb=none(empty) x1 upper REPLACED-existing | DIFFERENT: step2: plugin rejects, DC accepts |
| E-good-upper-then-garbage-upper-obj <br>_valid upper, then random bytes named upper_garbage.obj_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then EXC [Error: Nessuna mesh importata] | v=6000 f=11900 (scan replaced later) upper <br>then v=0 f=0 wbb=none(empty) x1 upper REPLACED-existing | DIFFERENT: step2: plugin rejects, DC accepts |
| E-good-upper-then-garbage-upper-stl <br>_valid upper, then random bytes named upper_garbage.stl_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=0 f=0 wbb=none..none x1 UPPER \| WARNING: upper_garbage: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [upper_garbage.stl: invalid STL data: truncated binary STL: header announces 3717673525 triangles, data holds 80] | DIFFERENT: step2: plugin accepts, DC rejects; final case holds 2 scan objects (plugin) vs 1 scans (DC) |
| E-good-upper-then-truncated-upper-stl <br>_valid upper, then truncated binary STL named upper_trunc.stl_ | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 UPPER <br>then v=0 f=0 wbb=none..none x1 UPPER \| WARNING: upper_trunc: Il modello e molto piccolo: verifica che le unita siano millimetri (1 unita Blender = 1 mm). | v=6000 f=11900 wbb=(-34.2589,10.4261,-5.2263)..(34.0185,25.5755,4.3496) x1 upper <br>then ERR [upper_trunc.stl: invalid STL data: truncated binary STL: header announces 11900 triangles, data holds 5000] | DIFFERENT: step2: plugin accepts, DC rejects; final case holds 2 scan objects (plugin) vs 1 scans (DC) |

## R. Real scans (4 STL, read in place; measurements only)

| Case | Plugin | DentalCraft | Verdict |
|---|---|---|---|
| R-orig-1 <br>_real scan, original name (read in place)_ | v=429605 f=855739 wbb=[redacted] x1 LOWER | v=429605 f=855739 wbb=[redacted] x1 lower | SAME |
| R-orig-2 <br>_real scan, original name (read in place)_ | v=402802 f=802606 wbb=[redacted] x1 UPPER | v=402802 f=802606 wbb=[redacted] x1 upper | SAME |
| R-orig-3 <br>_real scan, original name (read in place)_ | v=109716 f=215588 wbb=[redacted] x1 LOWER | v=109716 f=215588 wbb=[redacted] x1 lower | SAME |
| R-orig-4 <br>_real scan, original name (read in place)_ | v=100094 f=197756 wbb=[redacted] x1 UPPER | v=100094 f=197756 wbb=[redacted] x1 upper | SAME |
| R-neutral-scan_a.stl <br>_symlink scan_a.stl -> real scan 1_ | v=429605 f=855739 wbb=[redacted] x1 LOWER | v=429605 f=855739 wbb=[redacted] x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| R-neutral-scan_b.stl <br>_symlink scan_b.stl -> real scan 2_ | v=402802 f=802606 wbb=[redacted] x1 LOWER | v=402802 f=802606 wbb=[redacted] x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| R-neutral-scan_c.stl <br>_symlink scan_c.stl -> real scan 3_ | v=109716 f=215588 wbb=[redacted] x1 LOWER | v=109716 f=215588 wbb=[redacted] x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |
| R-neutral-scan_d.stl <br>_symlink scan_d.stl -> real scan 4_ | v=100094 f=197756 wbb=[redacted] x1 LOWER | v=100094 f=197756 wbb=[redacted] x1 upper | DIFFERENT: step1: role/arch LOWER vs upper |

### R2. Plugin geometric `detect_arch` on the neutral-named copies (name=scan_x.stl, no hint)

Neutral copies: scan_a to scan_d are symlinks to the four real scans in sorted file-name order (`add_real.py`); the true arch column assumes the order lower, upper, lower, upper. Operator arch = what `import_scans` assigns at import (no detection there); detect_arch = `autoseed.detect_arch(world verts, tris, name=...)` called as `case_auto` does in phase 2. DentalCraft has no geometric detection (import role = name hint, else first free role).

| Neutral file | True arch | Operator arch at import | detect_arch arch | confidence | source (fonte) | detect time s | DentalCraft role |
|---|---|---|---|---|---|---|---|
| scan_a.stl | lower | LOWER | LOWER | 0.80 | curva di Wilson: cuspidi piu alte dei molari vestibolari (arcata inferiore) | 3.633 | upper |
| scan_b.stl | upper | LOWER | UPPER | 1.00 | palato presente | 3.205 | upper |
| scan_c.stl | lower | LOWER | LOWER | 0.90 | curva di Wilson: cuspidi piu alte dei molari vestibolari (arcata inferiore) | 1.138 | upper |
| scan_d.stl | upper | LOWER | UPPER | 0.80 | orientamento dello scanner (denti rivolti in basso) | 1.662 | upper |

### R3. Import time, largest real scan (scan_a, 42787034 bytes, 429605 v, 855739 f)

| Side | What was timed | Runs (s) |
|---|---|---|
| Plugin | `bpy.ops.oa.import_scans` (Blender STL import + `_import_mesh` + `fix_units` + `create_case`), in-process, warm, 5 runs, Blender 5.2.2 headless | 0.581, 0.579, 0.544, 0.563, 0.591 (first run of the full suite: 0.565) |
| DentalCraft release | wall time of `dentalcraft-cli run project.new + case.import` (process start included; baseline project.new only: 0.001-0.003) | 0.305, 0.331, 0.302, 0.303, 0.305 |
| DentalCraft debug | same, debug build | 1.353, 1.301, 1.301, 1.306, 1.275 |
Other real scans, plugin operator time (single run): scan_b 0.52 s, scan_c 0.10 s, scan_d 0.08 s. The R3 timings were recorded with a separate timing script that is not part of this folder; they are kept as recorded.

