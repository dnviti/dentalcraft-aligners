# Notice

Required Notice: Copyright (c) Daniele Viti

## Licence

The source code of dentalcraft-aligners is source-available under the PolyForm Noncommercial License 1.0.0. The full text is in `LICENSE.md`.

You may use, change and share it for any noncommercial purpose. Commercial use is not allowed. This is not an open source licence in the OSI sense.

## Commercial licensing

For commercial use, contact the author, Daniele Viti: hello@dentalcraft.app.

## Not a medical device

DentalCraft is not a certified medical device. It has no regulatory clearance. Do not use it on patients or for clinical decisions without independent professional validation. It is provided as is, without warranty.

## Third-party components

This repository does not copy third-party source code, apart from the font listed below. Its Rust dependencies are fetched from crates.io and keep their own licences. The dependency tree of this repository is under MIT, Apache-2.0, or both, plus a few other permissive licences (Unicode-3.0, BSD-3-Clause, Zlib, Unlicense, CC0-1.0).

- A few crates offer a choice of licences, and DentalCraft uses them under the permissive option. For example `self_cell` is `Apache-2.0 OR GPL-2.0-only` and is used under Apache-2.0.
- `epaint_default_fonts` (pulled in by egui) is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`: the code is MIT or Apache-2.0, and the bundled fonts are under the SIL Open Font License 1.1 and the Ubuntu Font Licence 1.0.
- No dependency is under a GPL, AGPL or LGPL licence without a permissive alternative.

The complete list, with licence texts, is in `THIRD-PARTY-NOTICES.md` of `dentalcraft-app`, which is generated for each application release. The exact versions are in `Cargo.lock`.

### Fonts

`crates/dca-algo/assets/B612Mono-Bold-subset.ttf` is a subset of B612 Mono Bold, under the SIL Open Font License 1.1. The licence text is in `crates/dca-algo/assets/OFL-B612-Mono.txt`. The font declares no Reserved Font Name.

## Origin of the code: the author's Blender add-on

Large parts of this repository are ported from the author's own Blender add-on `blender-ortho-aligner`, by the same author. The add-on was published under GPL-3.0-or-later. Daniele Viti is its sole copyright holder, and he relicenses the code ported here under the PolyForm Noncommercial License 1.0.0. The port does not use Blender or its Python API.

In particular, `crates/dca-algo/src/evidence.rs` reproduces the add-on's evidence table (bibliographic references and parameter notes), entry by entry. The Italian texts of the findings are kept as in the add-on, because they are bibliographic data. The folder `golden/` holds the harness that compares the Rust stages with the add-on, which is not public.

## Dataset citation: Teeth3DS+

Some measurements quoted in the notes of `crates/dca-algo/src/evidence.rs` (for example the bite RMS limit `bite_rms_max`, measured on 4 patients of Teeth3DS+) come from the Teeth3DS+ dataset. Only aggregate numbers are quoted. No data of the dataset is included in this repository, and no mesh or fixture is derived from it.

Teeth3DS+ is released under CC BY-NC-ND 4.0. Citation:

> Ben-Hamadou A., Neifar N., Rekik A., Smaoui O., Bouzguenda F., Pujades S., Boyer E., Ladroit E. *Teeth3DS+: An Extended Benchmark for Intraoral 3D Scans Analysis.* arXiv:2210.06094, 2022 (v3, 2026). https://arxiv.org/abs/2210.06094. Dataset: https://crns-smartvision.github.io/teeth3ds

The test fixtures in this repository are synthetic arches made by the add-on's demo generator, never patient scans.
