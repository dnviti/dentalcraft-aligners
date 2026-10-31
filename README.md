# dentalcraft-aligners

First DentalCraft module (display name "Aligners", id `aligners`): a Rust workspace with the aligner state model, the planning algorithms and the module that plugs into the `dentalcraft-core` engine. Depends on `dentalcraft-core`.

This repository is a submodule of the [`dentalcraft`](https://github.com/dnviti-personal-org/dentalcraft) superproject, which holds the plan, the architecture decisions and the pinned versions of all DentalCraft repositories. Clone the superproject with `--recurse-submodules` to work on the whole project.

- Visibility: private.
- Versioning: [release-please](https://github.com/googleapis/release-please) on `main` (Conventional Commits, tags `vX.Y.Z`).
- CI: GitHub Actions on the `golgari-dev` runner scale set.
