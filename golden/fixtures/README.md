# Golden fixtures

Oracle outputs, one folder per case and stage (`<case>/<stage>/`), written by
`golden/oracle.py` (Tier A) and `golden/blender_dump.py` (Tier B). See
`../README.md` for the layout, the file formats, how to regenerate and how
Rust tests load them (`env!("CARGO_MANIFEST_DIR")/../../golden/fixtures/...`).

Synthetic data only (the demo arch); never Teeth3DS or patient scans.
Size budget: under 40 MB in total, every file under 4 MB (`make check`).
