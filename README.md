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

// The header variables that give the numbers a meaning come beside the model:
let (db, header) = uncad::parse_with_header("drawing.dxf")?;
println!("units: {:?}", header.units());   // $INSUNITS; None when the file does not say

// Bytes already in memory (from the network, an archive, ...):
let bytes = std::fs::read("drawing.dwg")?;
let db = uncad::parse_bytes(&bytes, uncad::Format::Dwg)?;

let json = db.to_json(uncad::ToJsonOptions { pretty: true })?;   // the model, serialized as-is
std::fs::write("drawing.json", json)?;

// Rendering is the iron-render-cad crate's (uncad-cli uses it):
let result = iron_render_cad::to_svg(&db, iron_render_cad::ToSvgOptions::default());
std::fs::write("drawing.svg", result.svg)?;
```

## CLI

```bash
cargo run -p uncad-cli -- drawing.dwg                            # summary: entity count per type
cargo run -p uncad-cli -- drawing.dwg -o drawing.json --pretty   # export the parsed model
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg             # render to SVG (model space)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png             # render to PNG (via SVG)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --scale 2   # rasterize at twice the size
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --fit 4000  # the longer side 4000 px
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg --no-trim   # keep outlying coordinates
cargo run -p uncad-cli -- drawing.dwg -o sheet.svg --space paper # sheet borders / title blocks
cargo run -p uncad-cli -- drawing.dwg -o all.svg --space all     # every space in one document
```

## Scope

1. **DWG** — read through [LibreDWG](https://www.gnu.org/software/libredwg/)
   (GPLv3+), bound directly via Rust FFI (`bindgen`). All versions.
2. **DXF** — read through the same LibreDWG engine, chosen by file extension
   (`parse`) or by the caller (`parse_bytes`). Every version, R2007 and later
   included: such a file holds its strings in two widths in LibreDWG's memory,
   and each is read in its own — see `docs/CAVEATS.md`, "DXF saved as R2007 or
   later is read". LibreDWG's own DXF importer is documented as working "for most
   objects", so it is less complete than its DWG reading, and slower than linear
   in the entity count ([`docs/CAVEATS.md`](./docs/CAVEATS.md)).
3. **Output** — the parsed model as JSON (`CadDatabase::to_json`, from
   `uncad-model`). SVG and PNG come from the
   [`iron-render-cad`](https://github.com/iyulab/iron-render-cad) crate (MIT), which
   `uncad-cli` uses. Writing DWG/DXF is not offered; the write API that existed in
   0.1.0 was removed (see [`CHANGELOG.md`](./CHANGELOG.md)).

The model itself -- `CadDatabase`, `Entity`, `Tables` and their JSON form -- is
the [`uncad-model`](https://github.com/iyulab/uncad-model) crate (MIT), re-exported
here as `uncad::model` / `uncad::tables` / `uncad::json`. This crate is one backend
that fills it; anything that only needs to *read* a drawing depends on the model
crate alone and inherits nothing from this crate's license.

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
in, and its license carries over. The vendored copy carries five local patches;
`crates/libredwg-sys/NOTICE.md` is their modification notice, inside the crate
so that it reaches the published tarball. Copyright and license details for
third-party components are in
[`docs/THIRD_PARTY_NOTICES.md`](./docs/THIRD_PARTY_NOTICES.md).

## Repository layout

```
lib/libredwg/            LibreDWG upstream, as a git submodule. Not used by the build --
                         it is the source vendor/ is regenerated from, and where the
                         real-file test fixtures (test/test-data/) come from
crates/
  libredwg-sys/          raw FFI (cc + bindgen). vendor/libredwg/ holds the subset of C
                         sources actually compiled (for publishing to crates.io), with
                         five local patches marked "uncad local patch" and listed in
                         NOTICE.md; shim/ holds the C accessors for opaque types, and
                         vendor-config/config.h stands in for autotools
  uncad/                 the safe API: parse() / parse_bytes() -> uncad_model::CadDatabase,
                         and the drawing's Header beside it from parse_with_header()
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
