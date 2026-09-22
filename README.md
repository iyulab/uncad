# uncad

[![CI](https://github.com/iyulab/uncad/actions/workflows/ci.yml/badge.svg)](https://github.com/iyulab/uncad/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

An open-source Rust library that parses CAD files (DWG/DXF) into a model and
exports that model as JSON, SVG or PNG. Read-only: it does not write DWG or DXF,
and does not convert between them.

## Quick start

```bash
git submodule update --init   # test fixtures only -- not needed to build (see "Platform")
cargo build --workspace
cargo test --workspace
```

```rust
let db = uncad::parse("drawing.dwg")?;   // same model for DWG and DXF (entities + tables)
println!("{} entities", db.entities.len());

let json = db.to_json(uncad::ToJsonOptions { pretty: true })?;   // the model, serialized as-is
std::fs::write("drawing.json", json)?;

let result = db.to_svg(uncad::ToSvgOptions::default());
std::fs::write("drawing.svg", result.svg)?;

let png = db.to_png(uncad::ToPngOptions::default())?;   // 1568 px long edge, white, 1.25 px strokes
std::fs::write("drawing.png", png.png)?;                 // png.view_box + png.px_per_unit map pixels back

// The LLM/VLM package: overview + tiles with sidecars + JSON records with exact numbers.
let report = uncad::export::export_package(&db, std::path::Path::new("drawing_pkg"), &uncad::ExportOptions::default())?;
println!("{} files, {} tiles", report.files.len(), report.counts.tiles);
```

## CLI

```bash
cargo run -p uncad-cli -- drawing.dwg                            # summary: version, units, entity count per type
cargo run -p uncad-cli -- drawing.dwg -o drawing.json --pretty   # export the parsed model
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg             # render to SVG (model space)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png             # render to PNG, long edge 1568 px
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --fit 4000  # long edge 4000 px
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --scale 2   # two pixels per drawing unit instead
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg --crop raw  # keep outlying coordinates
cargo run -p uncad-cli -- drawing.dwg --output drawing.svg       # --output is the long form of -o
cargo run -p uncad-cli -- export drawing.dwg -o drawing_pkg      # the LLM/VLM package (docs/VLM_EXPORT_DESIGN.md)
cargo run -p uncad-cli -- export drawing.dwg -o pkg --padding 0  # the package, flush to the drawing's edge
cargo run -p uncad-cli -- drawing.dwg -o sheet.svg --space paper # sheet borders / title blocks
cargo run -p uncad-cli -- drawing.dwg -o all.svg --space all     # every space in one document
```

## Scope

1. **DWG** — read through [LibreDWG](https://www.gnu.org/software/libredwg/)
   (GPLv3+), bound directly via Rust FFI (`bindgen`). All versions.
2. **DXF** — read through the same LibreDWG engine, chosen by file extension.
   LibreDWG's own DXF importer is documented as working "for most objects", so
   it is less complete than its DWG reading ([`docs/CAVEATS.md`](./docs/CAVEATS.md)).
3. **Output** — the parsed model as JSON (`to_json`), SVG (`to_svg`) or PNG
   (`to_png`), and the LLM/VLM package (`export::export_package`, `uncad export`):
   an overview image sized for the model, overlapping zoom tiles with JSON
   sidecars, and records with exact lengths, areas, dimension values and texts
   ([`docs/VLM_EXPORT_DESIGN.md`](./docs/VLM_EXPORT_DESIGN.md)). Writing DWG/DXF
   is not offered; the write API that existed in 0.1.0 was removed (see
   [`CHANGELOG.md`](./CHANGELOG.md)).

## Platform

Pure Rust plus native FFI. WebAssembly and the browser are not targets — the
intended consumers are libraries and binaries (CLI, server, desktop app).

`bindgen` needs `libclang`, so LLVM/Clang has to be installed (Windows:
`winget install LLVM.LLVM`, Ubuntu: `apt install libclang-dev`). The LibreDWG C
sources are vendored into `crates/libredwg-sys/vendor/libredwg/`, so **building
does not need the `lib/libredwg` submodule**. Running `cargo test --workspace`
does: 18 of the 22 integration test files in `uncad` (plus `png.rs`'s own
end-to-end test and `tests/documented_invocations.rs` in `uncad-cli`) read
fixtures from that submodule's `test/test-data/`; the four that do not
(`fixtures.rs`, `block_transforms.rs`, `control_chars.rs`,
`sheets_compositing.rs`) use this project's own DXF fixtures. Clone with
`git clone --recurse-submodules`, or run `git submodule update --init` in an
existing clone. Builds and tests pass on Linux (`x86_64-unknown-linux-gnu`) as
well as Windows.

## License

**GPLv3-or-later**, because LibreDWG (GPLv3+) is linked in and its license
carries over. It is the only bundled third-party source, carrying two local
patches marked in it, but not the only third-party code in a binary: `uncad`
also links `libc`, `resvg`, `png`, `serde`, `serde_json` and
`unicode-normalization`, and through them 68 crates in all, every one of them
under a permissive license. The bundled font
(`crates/uncad/fonts/UncadSans-Regular.otf`, a Noto Sans KR subset) is under the
SIL Open Font License 1.1, with its licence text beside it. Copyright and
license details, including which dependency notices have to travel with a
redistributed binary, are in
[`docs/THIRD_PARTY_NOTICES.md`](./docs/THIRD_PARTY_NOTICES.md).

## Repository layout

```
lib/libredwg/            LibreDWG upstream, as a git submodule. Not used by the build --
                         it is the source vendor/ is regenerated from, and where the
                         real-file test fixtures (test/test-data/) come from
crates/
  libredwg-sys/          raw FFI (cc + bindgen). vendor/libredwg/ holds the subset of C
                         sources actually compiled (for publishing to crates.io), with
                         two local patches marked "uncad local patch"; shim/ holds the C
                         accessors for opaque types, and vendor-config/config.h stands
                         in for autotools
  uncad/                 the safe API: parse() -> CadDatabase::{to_json,to_svg,to_png}()
  uncad-cli/             the CLI binary (uncad)
crates/*/tests/          integration tests against the public API. crates/*/examples/ are
                         manual-check tools, and #[cfg(test)] blocks inside src/*.rs are
                         unit tests -- docs/ARCHITECTURE.md's "Test layout" says which
                         belongs where
scripts/                 sync-libredwg-vendor.sh -- regenerates vendor/ after a submodule
                         update
samples/                 gitignored except its README -- drop any DWG/DXF in here for
                         manual testing, no license clearance needed. No automated tests
                         read from it
docs/                    architecture, known limitations, third-party notices
```

Further reading:

- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — crate layout, build system,
  test layout, the FFI/bindgen boundary, thread safety, the entity model
- [`docs/CAVEATS.md`](./docs/CAVEATS.md) — entity type coverage, known
  limitations and bugs, cross-platform notes
- [`docs/VLM_INVESTIGATION.md`](./docs/VLM_INVESTIGATION.md) — what it would
  take to hand a drawing to a VLM/LLM: extraction and rendering audit, measured
  numbers, facts settled against real files (2026-09-21)
- [`docs/VLM_EXPORT_DESIGN.md`](./docs/VLM_EXPORT_DESIGN.md) — the design the
  0.3.0 package follows (`export::export_package`, `uncad export`; section 10
  records what has landed): export package, crop and tiling rules, JSON schema,
  API, roadmap
