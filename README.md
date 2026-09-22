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

let png = db.to_png(uncad::ToPngOptions::default())?;   // via to_svg(); no SVG touches disk
std::fs::write("drawing.png", png.png)?;
```

## CLI

```bash
cargo run -p uncad-cli -- drawing.dwg                            # summary: version, units, entity count per type
cargo run -p uncad-cli -- drawing.dwg -o drawing.json --pretty   # export the parsed model
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg             # render to SVG (model space)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png             # render to PNG (via SVG)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --scale 2   # rasterize at twice the size
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg --no-trim   # keep outlying coordinates
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
   (`to_png`). Writing DWG/DXF is not offered; the write API that existed in
   0.1.0 was removed (see [`CHANGELOG.md`](./CHANGELOG.md)).

## Platform

Pure Rust plus native FFI. WebAssembly and the browser are not targets — the
intended consumers are libraries and binaries (CLI, server, desktop app).

`bindgen` needs `libclang`, so LLVM/Clang has to be installed (Windows:
`winget install LLVM.LLVM`, Ubuntu: `apt install libclang-dev`). The LibreDWG C
sources are vendored into `crates/libredwg-sys/vendor/libredwg/`, so **building
does not need the `lib/libredwg` submodule**. Running `cargo test --workspace`
does: the real-file tests (`png.rs`, `tests/dxf_pipeline.rs`, `tests/acis_sab.rs`
in `uncad`, and `tests/documented_invocations.rs` in `uncad-cli`) read fixtures
from that submodule's `test/test-data/`. Clone with
`git clone --recurse-submodules`, or run `git submodule update --init` in an
existing clone. Builds and tests pass on Linux (`x86_64-unknown-linux-gnu`) as
well as Windows.

## License

**GPLv3-or-later**. LibreDWG (GPLv3+) is the only third-party component linked
in, and its license carries over. Copyright and license details for third-party
components are in [`docs/THIRD_PARTY_NOTICES.md`](./docs/THIRD_PARTY_NOTICES.md).

## Repository layout

```
lib/libredwg/            LibreDWG upstream, as a git submodule. Not used by the build --
                         it is the source vendor/ is regenerated from, and where the
                         real-file test fixtures (test/test-data/) come from
crates/
  libredwg-sys/          raw FFI (cc + bindgen). vendor/libredwg/ holds the unmodified
                         subset of C sources actually compiled (for publishing to
                         crates.io); shim/ holds the C accessors for opaque types, and
                         vendor-config/config.h stands in for autotools
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
- [`docs/VLM_EXPORT_DESIGN.md`](./docs/VLM_EXPORT_DESIGN.md) — the resulting
  proposal (not implemented): export package, crop and tiling rules, JSON
  schema, API, roadmap
