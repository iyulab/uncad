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
- **Modified**: **yes**, in two files. Each change carries an
  `uncad local patch` comment in the source saying what changed and why
  (GPLv3 §5(a)); `grep -rn "uncad local patch" vendor/libredwg/` finds every
  marker, and `build.rs` refuses to build when the markers per file differ
  from the list it carries. The same changes are in `patches/`, one unified
  diff per file against the upstream commit `vendor/UPSTREAM` names.

### The changes

The marker in `common.c` was written when it was one of the first two
changes, and cites this list as "The two changes"; it is this one. Other
changes this crate once carried (the order of an entity's RGB and transparency,
an R13/R14 entity's linetype flags, an R2010+ ATTRIB's text style) are now
upstream's own code and are no longer patched.

1. **`vendor/libredwg/src/common.c`**, 2026-09-23 — `cvt_TIMEBLL()` zeroes its
   `struct tm` and clamps every field into the range `strftime()` accepts, so
   a corrupt date in a hostile file cannot abort the process from inside a
   library call.
2. **`vendor/libredwg/src/acds.spec`**, 2026-09-30 — an ACIS body in an
   R2013+ drawing's data storage goes to the entity whose handle its `_data_`
   record names, instead of to the drawing's solids in the order a signature
   search found the bodies (which put bodies on the wrong solids and found no
   R2018 body, whose signature differs).
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
