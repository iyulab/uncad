# Third-Party Notices

This project bundles or builds against the following third-party components.

## LibreDWG

- **Source**: https://github.com/LibreDWG/libredwg (the real upstream project, tracked as a **git submodule** at `lib/libredwg`, plus a subset of it, carrying the local patches described below, vendored directly into `crates/libredwg-sys/vendor/libredwg/`)
- **Copyright**: Free Software Foundation, Inc.
- **License**: GNU General Public License v3.0 or later (GPLv3+), original `COPYING` file included at `crates/libredwg-sys/vendor/libredwg/COPYING`
- **Used for**: DWG and DXF parsing (core engine, compiled as a native static library, linked via Rust FFI — `crates/libredwg-sys`)
- **Modified**: yes. Every change is marked in the source with a dated `uncad local patch` comment naming what changed and why (GPLv3 §5(a)), and `crates/libredwg-sys/NOTICE.md` lists each one with the file it touches. That notice travels inside the published crate, because `docs/` is not part of that tarball; the patch files themselves are in `crates/libredwg-sys/patches/`. `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG", has the full reasoning. Everything else is upstream as written; autotools' generated `config.h` is stood in for by this project's own `crates/libredwg-sys/vendor-config/config.h`, a separate file, not a patch to LibreDWG's own sources.
- **Vendored subset**: `crates/libredwg-sys` is published to crates.io as a standalone, self-contained crate, and crates.io only packages files inside a crate's own directory (no git submodules for downstream consumers). So `crates/libredwg-sys/vendor/libredwg/` holds a copy of just the 112 upstream files this crate's `build.rs` actually compiles/includes (24 `.c` files plus every header/`.spec`/`.inc`/codepage table they `#include`, traced from the real include graph — not a hand-picked subset), with the patches above, rather than the full `lib/libredwg` submodule (which also has tests, docs, examples, and program sources this crate never builds). See `docs/ARCHITECTURE.md`'s "Build" section and `scripts/sync-libredwg-vendor.sh` for how it's kept in sync with the submodule.

## acadrust

- **Source**: https://github.com/hakanaktt/acadrust — the Source Code Form for the exact version this workspace resolves is also published on crates.io at https://crates.io/crates/acadrust, and `cargo package`/`cargo vendor` will fetch it from there.
- **License**: Mozilla Public License 2.0 (MPL-2.0), original `LICENSE` file included in the published crate. The notice in the license's Exhibit B is **not** attached to it, so it is not "Incompatible With Secondary Licenses".
- **Used for**: reading DWG files in this workspace's **tests only** — a second, independent implementation that gives this crate's own output an oracle not sharing its engine. It is a development dependency: nothing a consumer of these crates builds, links or distributes contains it.
- **Modified**: no — consumed as published.
- **Why it is named here**: MPL-2.0 § 3.2 asks anyone distributing an Executable Form built from Covered Software to tell recipients how to obtain the Source Code Form, and it asks that whether or not the Covered Software was modified. This entry is that notice. Its § 3.3 is why the rest of this workspace keeps its own terms.

## Noto Sans KR (font subset, through `iron-pack-cad`)

- **Source**: https://github.com/notofonts/noto-cjk (`Sans/SubsetOTF/KR/NotoSansKR-Regular.otf`, v2.004), subset and renamed by the [`iron-pack-cad`](https://crates.io/crates/iron-pack-cad) crate, which records the exact steps
- **Copyright**: Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'
- **License**: SIL Open Font License, Version 1.1 (OFL-1.1); the licence text travels inside the `iron-pack-cad` crate with the font
- **Used for**: drawing and measuring text in the images `uncad export` writes, so they are the same on every machine
- **Modified**: yes, by that crate (a subset, renamed as the OFL asks of a Modified Version); not further by this project
- **Why it is named here**: a font is data, not a Rust dependency, so cargo-deny never sees it. `iron-pack-cad` states it in its licence expression (`MIT AND OFL-1.1`), `deny.toml` grants the OFL to that crate alone, and the `uncad-cli` binary embeds the font bytes.

---

Each published crate carries the full GPLv3 text as a `LICENSE` file in its own directory
(`crates/libredwg-sys/`, `crates/uncad/`, `crates/uncad-cli/`), byte-identical to the
repository root's. They are copies rather than one shared file because `cargo package`
never reaches outside a crate directory, and GPLv3 §4 asks for the licence to be conveyed
with the source that is conveyed — a crates.io tarball is exactly that.

This project is distributed under **GPLv3-or-later**, matching LibreDWG's own license (the only third-party *code* bundled; the Noto Sans KR subset above is data under the SIL Open Font License 1.1, which permits bundling in GPL software). See [`LICENSE`](../LICENSE).

DWF/DWFx support (previously provided by a vendored subset of
[dwf-viewer](https://github.com/flyfish-dev/dwf-viewer), AGPL-3.0-only) was
dropped when this project moved to a native Rust implementation. That's also
why the project license could move from AGPL-3.0 (required while combining a
GPLv3+ component with an AGPL-3.0 one, per GPLv3 §13) back down to plain
GPLv3-or-later.
