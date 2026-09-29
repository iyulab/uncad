# Notices for `libredwg-sys`

`libredwg-sys` is GPL-3.0-or-later. The licence text is `LICENSE` beside this
file; it is the same text as LibreDWG's own `vendor/libredwg/COPYING`.

## LibreDWG — vendored, and **modified**

- **Upstream**: https://github.com/LibreDWG/libredwg
- **Copyright**: Free Software Foundation, Inc.
- **Licence**: GNU General Public License v3.0 or later; upstream's own copy
  travels in this tarball as `vendor/libredwg/COPYING`.
- **What is here**: not all of LibreDWG. `vendor/libredwg/` holds only the
  files `build.rs` actually compiles or `#include`s — 24 `.c` files plus the
  headers, `.spec`, `.inc` and code-page tables they reach, traced from the
  real include graph. Autotools' generated `config.h` is stood in for by this
  crate's own `vendor-config/config.h`, a separate file rather than a patch to
  LibreDWG's sources.
- **Modified**: **yes**, in four files. Each change carries an
  `uncad local patch` comment in the source saying what changed and why
  (GPLv3 §5(a)); `grep -rn "uncad local patch" vendor/libredwg/` finds every
  marker, and `build.rs` refuses to build when the markers per file differ
  from the list it carries. The same changes are in `patches/`, one unified
  diff per file against the upstream commit `vendor/UPSTREAM` names.

### The changes

The marker in `common.c` was written when it was one of the first two
changes, and cites this list as "The two changes"; it is this one.

1. **`vendor/libredwg/src/common.c`**, 2026-09-23 — `cvt_TIMEBLL()` zeroes its
   `struct tm` and clamps every field into the range `strftime()` accepts, so
   a corrupt date in a hostile file cannot abort the process from inside a
   library call.
2. **`vendor/libredwg/src/common_entity_data.spec`**, 2026-09-23 — the R2004+
   entity colour reads its RGB before its transparency, the order LibreDWG's
   own `bit_read_ENC()` uses. With the two the other way round, an entity
   carrying both had its colour and its transparency swapped.
3. **`vendor/libredwg/src/common_entity_data.spec`**, 2026-09-24 — an R13/R14
   entity whose "BYLAYER" bit is clear gets linetype flags 3 (by handle);
   upstream set the flags only when the bit was set, so it read as BYLAYER.
4. **`vendor/libredwg/src/dwg.spec`**, 2026-09-24 — an R2010+ ATTRIB no longer
   reads the version byte only an ATTDEF stores; read past the record, it
   ended the decode before the text style handle, so every such attribute
   came back with no style.

None of the changes alters LibreDWG's file formats or its API. Each changes
what the library reads only where upstream misreads a drawing, refuses it, or
ends the process on it.

The full reasoning, with the reproduction for each, is in the project's
`docs/CAVEATS.md` under "Local patches to the vendored LibreDWG" — that file
is *not* in this tarball; it is at https://github.com/iyulab/uncad.

## Everything else

`libredwg-sys` bundles no other third-party source. The project-wide notice
file, covering the Rust dependency licences, is `docs/THIRD_PARTY_NOTICES.md`
in the repository above.
