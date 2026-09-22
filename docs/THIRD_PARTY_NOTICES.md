# Third-Party Notices

This project bundles or builds against the following third-party components.

## LibreDWG

- **Source**: https://github.com/LibreDWG/libredwg (the real upstream project, tracked as a **git submodule** at `lib/libredwg`, plus an unmodified subset of it vendored directly into `crates/libredwg-sys/vendor/libredwg/` — see below)
- **Copyright**: Free Software Foundation, Inc.
- **License**: GNU General Public License v3.0 or later (GPLv3+), original `COPYING` file included at `crates/libredwg-sys/vendor/libredwg/COPYING`
- **Used for**: DWG and DXF parsing (core engine, compiled as a native static library, linked via Rust FFI — `crates/libredwg-sys`)
- **Modified**: no — built directly from unmodified upstream sources (autotools' generated `config.h` is stood in for by this project's own `crates/libredwg-sys/vendor-config/config.h`, a separate file, not a patch to LibreDWG's own sources).
- **Vendored subset**: `crates/libredwg-sys` is published to crates.io as a standalone, self-contained crate, and crates.io only packages files inside a crate's own directory (no git submodules for downstream consumers). So `crates/libredwg-sys/vendor/libredwg/` holds an unmodified, byte-for-byte copy of just the 112 upstream files this crate's `build.rs` actually compiles/includes (24 `.c` files plus every header/`.spec`/`.inc`/codepage table they `#include`, traced from the real include graph — not a hand-picked subset) — not the full `lib/libredwg` submodule (which also has tests, docs, examples, and program sources this crate never builds). See `docs/ARCHITECTURE.md`'s "Build" section and `scripts/sync-libredwg-vendor.sh` for how it's kept in sync with the submodule.

---

This project is distributed under **GPLv3-or-later**, matching LibreDWG's own license (the only third-party *code* bundled; the second bundled component, the Noto Sans KR subset below, is data under the SIL Open Font License 1.1, which permits bundling in GPL software). See [`LICENSE`](../LICENSE).

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
