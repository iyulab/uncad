# Third-Party Notices

This project bundles or builds against the following third-party components.

## LibreDWG

- **Source**: https://github.com/LibreDWG/libredwg (the real upstream project, tracked as a **git submodule** at `lib/libredwg`, plus a subset of it, carrying the five local patches described below, vendored directly into `crates/libredwg-sys/vendor/libredwg/`)
- **Copyright**: Free Software Foundation, Inc.
- **License**: GNU General Public License v3.0 or later (GPLv3+), original `COPYING` file included at `crates/libredwg-sys/vendor/libredwg/COPYING`
- **Used for**: DWG and DXF parsing (core engine, compiled as a native static library, linked via Rust FFI — `crates/libredwg-sys`)
- **Modified**: yes, in five files, each change dated 2026-09-23 and marked in the source with an `uncad local patch` comment naming what changed and why (GPLv3 §5(a)). The same notice travels inside the published crate as `crates/libredwg-sys/NOTICE.md`, because `docs/` is not part of that tarball. `src/dwg.c`: `dwg_find_tablehandle()`, `dwg_find_dicthandle_objname()` and `dwg_handle_name()` read a table record's name through a new `uncad_record_name_utf8()` helper, so an R2007+ DXF's UTF-16 record names are compared correctly. `src/common.c`: `cvt_TIMEBLL()` zeroes its `struct tm` and clamps every field into the ranges `strftime()` accepts, so a corrupt date cannot fail-fast the process. `src/in_dxf.c` and `src/dynapi.c`: a polygon mesh's vertex marker `AcDbPolygonMeshVertex` is known, so one polygon mesh no longer makes the DXF reader refuse the whole file. `src/common_entity_data.spec`: an R2004+ entity colour reads its RGB before its transparency, as LibreDWG's own `bit_read_ENC()` does, so an entity carrying both no longer has the two swapped. `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG", has the full reasoning. Everything else is upstream as written; autotools' generated `config.h` is stood in for by this project's own `crates/libredwg-sys/vendor-config/config.h`, a separate file, not a patch to LibreDWG's own sources.
- **Vendored subset**: `crates/libredwg-sys` is published to crates.io as a standalone, self-contained crate, and crates.io only packages files inside a crate's own directory (no git submodules for downstream consumers). So `crates/libredwg-sys/vendor/libredwg/` holds a copy of just the 112 upstream files this crate's `build.rs` actually compiles/includes (24 `.c` files plus every header/`.spec`/`.inc`/codepage table they `#include`, traced from the real include graph — not a hand-picked subset), with the five patches above, rather than the full `lib/libredwg` submodule (which also has tests, docs, examples, and program sources this crate never builds). See `docs/ARCHITECTURE.md`'s "Build" section and `scripts/sync-libredwg-vendor.sh` for how it's kept in sync with the submodule.

## acadrust

- **Source**: https://github.com/hakanaktt/acadrust — the Source Code Form for the exact version this workspace resolves is also published on crates.io at https://crates.io/crates/acadrust, and `cargo package`/`cargo vendor` will fetch it from there.
- **License**: Mozilla Public License 2.0 (MPL-2.0), original `LICENSE` file included in the published crate. The notice in the license's Exhibit B is **not** attached to it, so it is not "Incompatible With Secondary Licenses".
- **Used for**: reading DWG files in this workspace's **tests only** — a second, independent implementation that gives this crate's own output an oracle not sharing its engine. It is a development dependency: nothing a consumer of these crates builds, links or distributes contains it.
- **Modified**: no — consumed as published.
- **Why it is named here**: MPL-2.0 § 3.2 asks anyone distributing an Executable Form built from Covered Software to tell recipients how to obtain the Source Code Form, and it asks that whether or not the Covered Software was modified. This entry is that notice. Its § 3.3 is why the rest of this workspace keeps its own terms.

## Noto Sans KR (bundled font subset)

- **Source**: https://github.com/notofonts/noto-cjk (`Sans/SubsetOTF/KR/NotoSansKR-Regular.otf`, v2.004)
- **Copyright**: Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'
- **License**: SIL Open Font License, Version 1.1 (OFL-1.1), licence text at `crates/uncad-export/fonts/OFL-NotoSansKR.txt`, inside the crate so it travels with every copy of the font
- **Used for**: drawing and measuring text in the images of the LLM/VLM package `crates/uncad-export` writes (Latin, Greek, the 2350 common Hangul syllables and the CAD symbols), so they are the same on every machine
- **Modified**: yes -- `crates/uncad-export/fonts/UncadSans-Regular.otf` is a subset (2755 glyphs, no layout tables) renamed to "Uncad Sans", as the OFL asks of a Modified Version; `crates/uncad-export/fonts/README.md` records the exact steps and the tools that produce it from the source above
- **Why it is named here**: a font is data, not a Rust dependency, so cargo-deny never sees it. The crate states it in its licence expression (`GPL-3.0-or-later AND OFL-1.1`), `deny.toml` grants the OFL to that crate alone, and every binary linking `uncad-export` embeds the font bytes.

---

Each published crate carries the full GPLv3 text as a `LICENSE` file in its own directory
(`crates/libredwg-sys/`, `crates/uncad/`, `crates/uncad-export/`, `crates/uncad-cli/`), byte-identical to the
repository root's. They are copies rather than one shared file because `cargo package`
never reaches outside a crate directory, and GPLv3 §4 asks for the licence to be conveyed
with the source that is conveyed — a crates.io tarball is exactly that.

This project is distributed under **GPLv3-or-later**, matching LibreDWG's own license (the only third-party *code* bundled; the second bundled component, the Noto Sans KR subset above, is data under the SIL Open Font License 1.1, which permits bundling in GPL software, and is why `uncad-export`'s licence expression names both). See [`LICENSE`](../LICENSE).

DWF/DWFx support (previously provided by a vendored subset of
[dwf-viewer](https://github.com/flyfish-dev/dwf-viewer), AGPL-3.0-only) was
dropped when this project moved to a native Rust implementation. That's also
why the project license could move from AGPL-3.0 (required while combining a
GPLv3+ component with an AGPL-3.0 one, per GPLv3 §13) back down to plain
GPLv3-or-later.
