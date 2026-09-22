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
- **Modified**: **yes**, in two places. Both carry an `uncad local patch`
  comment in the source saying what changed and why (GPLv3 §5(a)); `grep -rn
  "uncad local patch" vendor/libredwg/` finds every marker.

### The two changes

1. **`vendor/libredwg/src/dwg.c`**, 2026-09-23 — `dwg_find_tablehandle()`,
   `dwg_find_dicthandle_objname()` and `dwg_handle_name()` read a table
   record's name through a new `uncad_record_name_utf8()` helper instead of
   `dwg_dynapi_entity_utf8text()`, so an R2007+ DXF's UTF-16 record names are
   decoded before they are compared. Upstream compares the raw UTF-16 bytes
   and so never matches such a name.
2. **`vendor/libredwg/src/common.c`**, 2026-09-23 — `cvt_TIMEBLL()` zeroes its
   `struct tm` and clamps every field into the range `strftime()` accepts, so
   a corrupt date in a hostile file cannot abort the process from inside a
   library call.

Neither change alters LibreDWG's file formats, its API or its output for a
well-formed drawing.

The full reasoning, with the reproduction for each, is in the project's
`docs/CAVEATS.md` under "Local patches to the vendored LibreDWG" — that file
is *not* in this tarball; it is at https://github.com/iyulab/uncad.

## Everything else

`libredwg-sys` bundles no other third-party source. The project-wide notice
file, covering the Rust dependency licences and the bundled font the `uncad`
crate carries, is `docs/THIRD_PARTY_NOTICES.md` in the repository above.
