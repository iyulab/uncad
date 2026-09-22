# Third-Party Notices

This project bundles or builds against the following third-party components.

## LibreDWG

- **Source**: https://github.com/LibreDWG/libredwg (the real upstream project, tracked as a **git submodule** at `lib/libredwg`, plus an unmodified subset of it vendored directly into `crates/libredwg-sys/vendor/libredwg/` — see below)
- **Copyright**: Free Software Foundation, Inc.
- **License**: GNU General Public License v3.0 or later (GPLv3+), original `COPYING` file included at `crates/libredwg-sys/vendor/libredwg/COPYING`
- **Used for**: DWG and DXF parsing (core engine, compiled as a native static library, linked via Rust FFI — `crates/libredwg-sys`)
- **Modified**: no — built directly from unmodified upstream sources (autotools' generated `config.h` is stood in for by this project's own `crates/libredwg-sys/vendor-config/config.h`, a separate file, not a patch to LibreDWG's own sources).
- **Vendored subset**: `crates/libredwg-sys` is published to crates.io as a standalone, self-contained crate, and crates.io only packages files inside a crate's own directory (no git submodules for downstream consumers). So `crates/libredwg-sys/vendor/libredwg/` holds an unmodified, byte-for-byte copy of just the 112 upstream files this crate's `build.rs` actually compiles/includes (24 `.c` files plus every header/`.spec`/`.inc`/codepage table they `#include`, traced from the real include graph — not a hand-picked subset) — not the full `lib/libredwg` submodule (which also has tests, docs, examples, and program sources this crate never builds). See `docs/ARCHITECTURE.md`'s "Build" section and `scripts/sync-libredwg-vendor.sh` for how it's kept in sync with the submodule.

## acadrust

- **Source**: https://github.com/hakanaktt/acadrust — the Source Code Form for the exact version this workspace resolves is also published on crates.io at https://crates.io/crates/acadrust, and `cargo package`/`cargo vendor` will fetch it from there.
- **License**: Mozilla Public License 2.0 (MPL-2.0), original `LICENSE` file included in the published crate. The notice in the license's Exhibit B is **not** attached to it, so it is not "Incompatible With Secondary Licenses".
- **Used for**: reading DWG files in this workspace's **tests only** — a second, independent implementation that gives this crate's own output an oracle not sharing its engine. It is a development dependency: nothing a consumer of these crates builds, links or distributes contains it.
- **Modified**: no — consumed as published.
- **Why it is named here**: MPL-2.0 § 3.2 asks anyone distributing an Executable Form built from Covered Software to tell recipients how to obtain the Source Code Form, and it asks that whether or not the Covered Software was modified. This entry is that notice. Its § 3.3 is why the rest of this workspace keeps its own terms.

---

This project is distributed under **GPLv3-or-later**, matching LibreDWG's own license (the only third-party component bundled). See [`LICENSE`](../LICENSE).

DWF/DWFx support (previously provided by a vendored subset of
[dwf-viewer](https://github.com/flyfish-dev/dwf-viewer), AGPL-3.0-only) was
dropped when this project moved to a native Rust implementation. That's also
why the project license could move from AGPL-3.0 (required while combining a
GPLv3+ component with an AGPL-3.0 one, per GPLv3 §13) back down to plain
GPLv3-or-later.
