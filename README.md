# dentalcraft-aligners

The *Aligners* module of [DentalCraft](https://dentalcraft.app), a desktop application for digital dentistry, written in Rust. Module id `aligners`, shown as "Aligners" in English and "Allineatori" in Italian.

The workspace has the aligner state model, the planning algorithms (one file per pipeline stage) and the module that plugs into the [`dentalcraft-core`](https://github.com/dnviti/dentalcraft-core) engine. It is headless: no window and no GPU.

> **Status: alpha.** Algorithms, commands and the saved module state can still change between releases.
>
> **Not a medical device.** The plans this module produces are not clinically validated. See [Disclaimer](#disclaimer).

## What it does

The module takes the intraoral scans of the upper and lower arch (and a bite scan) and goes from the scans to the models of the aligners, ready to print.

| Function (menu group) | What it does |
|---|---|
| Case: import scans | Imports the arch and bite scans; recognises the arch of a scan. |
| Teeth | Finds the teeth, separates teeth and gum, builds closed 3D teeth, with tools to mark, delete, renumber or mark missing teeth and a brush for manual corrections. |
| Diagnosis | Space analysis, Little irregularity index, Bolton ratios and the difficulty of the case. |
| Plan: bite | Shows how the two arches meet, from the bite scan when there is one, with a map of the contacts. |
| Plan: final position | Proposes where every tooth should end up, within safe movement limits, with the arch line and tools to move a tooth. |
| Plan: aligners | Divides the movement into a sequence of aligners and gives their calendar. |
| Clinical review | Classifications proposed from the evidence (difficulty, aligner pattern, attachments, each tooth), to confirm or change before production. |
| Production | Writes to a folder the models and the aligners of every stage, ready to print, with the print check, the report and the list of files. |
| Follow-up | Compares a new scan with the plan, and makes a new series of aligners from the last control visit. |
| Study models | Orthodontic study models on a trimmed base (European or ABO style). |
| Run everything automatically | A complete plan from the scans to the calendar. |

Every parameter default has a bibliographic source in `dca-algo/src/evidence.rs`. Every function is also a registered command, so it works from `dentalcraft-cli` and over MCP, not only from the window.

## Crates

| Crate | What it does |
|---|---|
| `dca-model` | The state of the module (seeds, teeth, settings, visits, production) and its migrations. |
| `dca-algo` | The algorithms, one file per stage: arch, seeds, segmentation, teeth, diagnosis, bite, setup, staging, shell, trim, stamp, visits, export, plus shared parts (world frame, rotations, arch forms, tooth anatomy, evidence base). |
| `dca-module` | The `Module` implementation: menu tree, commands, inspector, scene, project codec. |

## Build and test

You need the Rust toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml). No C++ toolchain is needed. The dependencies on `dentalcraft-core` are git dependencies pinned by tag in `Cargo.toml`.

```sh
cargo build --workspace
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use synthetic arches only. Tests that read real scans take their path from an environment variable and are skipped without it. Never commit patient scans or datasets. The `golden/` folder holds a harness that compares the Rust stages with the author's Blender add-on, which is not public; it is run by the maintainer.

To change core and aligners together, work in the [`dentalcraft`](https://github.com/dnviti/dentalcraft) superproject, which redirects the dependencies to the working trees. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Architecture

- [`dentalcraft`](https://github.com/dnviti/dentalcraft) (superproject): pins all repositories and holds the architecture decision records in `docs/adr`.
- [`dentalcraft-docs`](https://github.com/dnviti/dentalcraft-docs): documentation.
- Layering: this repository depends on `dentalcraft-core`. [`dentalcraft-app`](https://github.com/dnviti/dentalcraft-app) depends on core and on this module.

## Licence

Source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE.md). You may read, use, change and share the code for any noncommercial purpose. Commercial use is not allowed.

This is not an open source licence in the OSI sense. Parts of the code are ported from the author's own Blender add-on, and the evidence table quotes measurements on the Teeth3DS+ dataset: see [`NOTICE.md`](NOTICE.md) for the origin, the third-party components and the citation.

**Commercial licensing.** If you want to use DentalCraft commercially, contact the author at hello@dentalcraft.app.

## Disclaimer

DentalCraft is not a certified medical device and has no regulatory clearance. The plans, the aligner models and the reports it produces are not clinically validated. It is provided as is, without warranty. Do not use it on patients or for clinical decisions without independent professional validation. See [`SECURITY.md`](SECURITY.md).

## Contributing

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) (DCO sign-off, inbound licence grant, checks) and the [Code of Conduct](CODE_OF_CONDUCT.md). Report security problems privately, as described in [`SECURITY.md`](SECURITY.md).

## Links

| | |
|---|---|
| This repository | https://github.com/dnviti/dentalcraft-aligners |
| Superproject | https://github.com/dnviti/dentalcraft |
| Engine | https://github.com/dnviti/dentalcraft-core |
| Application | https://github.com/dnviti/dentalcraft-app |
| Website | https://github.com/dnviti/dentalcraft-web |
| Documentation | https://github.com/dnviti/dentalcraft-docs |
| Product site | https://dentalcraft.app |

Versions are cut by [release-please](https://github.com/googleapis/release-please) from Conventional Commits on `main` (tags `vX.Y.Z`).
