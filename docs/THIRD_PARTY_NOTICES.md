# Third-Party Notices

This project bundles or builds against the following third-party components.

## LibreDWG

- **Source**: https://github.com/LibreDWG/libredwg (the real upstream project, tracked as a **git submodule** at `lib/libredwg`, plus a subset of it, carrying the two local patches described below, vendored directly into `crates/libredwg-sys/vendor/libredwg/`)
- **Copyright**: Free Software Foundation, Inc.
- **License**: GNU General Public License v3.0 or later (GPLv3+), original `COPYING` file included at `crates/libredwg-sys/vendor/libredwg/COPYING`
- **Used for**: DWG and DXF parsing (core engine, compiled as a native static library, linked via Rust FFI — `crates/libredwg-sys`)
- **Modified**: yes, in two places, both dated 2026-09-23 and marked in the source with an `uncad local patch` comment naming what changed and why (GPLv3 §5(a)). The same notice travels inside the published crate as `crates/libredwg-sys/NOTICE.md`, because `docs/` is not part of that tarball. `src/dwg.c`: `dwg_find_tablehandle()`, `dwg_find_dicthandle_objname()` and `dwg_handle_name()` read a table record's name through a new `uncad_record_name_utf8()` helper, so an R2007+ DXF's UTF-16 record names are compared correctly. `src/common.c`: `cvt_TIMEBLL()` zeroes its `struct tm` and clamps every field into the ranges `strftime()` accepts, so a corrupt date cannot fail-fast the process. `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG", has the full reasoning. Everything else is upstream as written; autotools' generated `config.h` is stood in for by this project's own `crates/libredwg-sys/vendor-config/config.h`, a separate file, not a patch to LibreDWG's own sources.
- **Vendored subset**: `crates/libredwg-sys` is published to crates.io as a standalone, self-contained crate, and crates.io only packages files inside a crate's own directory (no git submodules for downstream consumers). So `crates/libredwg-sys/vendor/libredwg/` holds a copy of just the 112 upstream files this crate's `build.rs` actually compiles/includes (24 `.c` files plus every header/`.spec`/`.inc`/codepage table they `#include`, traced from the real include graph, not a hand-picked subset), with the two patches above, rather than the full `lib/libredwg` submodule (which also has tests, docs, examples, and program sources this crate never builds). See `docs/ARCHITECTURE.md`'s "Build" section and `scripts/sync-libredwg-vendor.sh` for how it's kept in sync with the submodule.

## Rust dependencies

LibreDWG is the only third-party component whose source this repository *bundles*, but
it is not the only one linked into a binary. The `uncad` library's direct dependencies,
from `crates/uncad/Cargo.toml`:

| Crate | Version | License |
|---|---|---|
| `libredwg-sys` | 0.2 (this workspace) | GPL-3.0-or-later |
| `libc` | 0.2 | MIT OR Apache-2.0 |
| `resvg` | 0.48 | Apache-2.0 OR MIT |
| `png` | 0.18 | MIT OR Apache-2.0 |
| `serde` | 1.0 | MIT OR Apache-2.0 |
| `serde_json` | 1.0 | MIT OR Apache-2.0 |
| `unicode-normalization` | 0.1 | MIT OR Apache-2.0 |

`uncad-cli` adds nothing beyond `uncad` itself. Counting what they pull in,
`cargo tree -p uncad -e normal` is 68 third-party crates besides `libredwg-sys`, and none
of them is copyleft: 41 are `MIT OR Apache-2.0` (in either order and spelling), 11 are
MIT, and the remaining 16 offer MIT, Apache-2.0, Zlib, Unlicense or 0BSD alternatives.
Three are outside that family, and their notices are the ones easiest to miss when
redistributing a binary:

- `tiny-skia` 0.12 and `tiny-skia-path` 0.12, the rasterizer under `resvg`:
  **BSD-3-Clause**, Copyright 2011 Google Inc. and 2020 Yevhenii Reizner, whose terms
  require the copyright notice and disclaimer to accompany a binary redistribution.
- `arrayref` 0.3: **BSD-2-Clause**, Copyright 2015 David Roundy, same requirement.

The MIT and Apache-2.0 crates carry the same kind of attribution requirement, so a
product shipping an `uncad` binary should generate the full notice file from the lock
file (`cargo about`, `cargo license` or equivalent) rather than copy this table: it names
the licenses, not every copyright holder. Versions above are what `Cargo.toml` asks for;
`Cargo.lock` has the resolved ones.

---

Each published crate carries the full GPLv3 text as a `LICENSE` file in its own directory
(`crates/libredwg-sys/`, `crates/uncad/`, `crates/uncad-cli/`), byte-identical to the
repository root's. They are copies rather than one shared file because `cargo package`
never reaches outside a crate directory, and GPLv3 §4 asks for the licence to be conveyed
with the source that is conveyed — a crates.io tarball is exactly that.

This project is distributed under **GPLv3-or-later**, matching LibreDWG's own license (the only third-party *code* bundled; the second bundled component, the Noto Sans KR subset below, is data under the SIL Open Font License 1.1, which permits bundling in GPL software). The Rust dependencies above are all permissive, so none of them constrains that choice. See [`LICENSE`](../LICENSE).

DWF/DWFx support (previously provided by a vendored subset of
[dwf-viewer](https://github.com/flyfish-dev/dwf-viewer), AGPL-3.0-only) was
dropped when this project moved to a native Rust implementation. That's also
why the project license could move from AGPL-3.0 (required while combining a
GPLv3+ component with an AGPL-3.0 one, per GPLv3 §13) back down to plain
GPLv3-or-later.

## Noto Sans KR (bundled font subset)

`crates/uncad/fonts/UncadSans-Regular.otf` is a subset of Noto Sans KR Regular
v2.004, Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font
Name 'Source', licensed under the SIL Open Font License, Version 1.1. The
licence text is `crates/uncad/fonts/OFL-NotoSansKR.txt`; the subset is a
Modified Version renamed to "Uncad Sans" (`crates/uncad/fonts/README.md`
records the exact steps). The font is embedded in every binary that links the
`uncad` crate.
