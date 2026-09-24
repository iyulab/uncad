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
- **Modified**: **yes**, in six files. Each change carries an
  `uncad local patch` comment in the source saying what changed and why
  (GPLv3 §5(a)); `grep -rn "uncad local patch" vendor/libredwg/` finds every
  marker, and `build.rs` refuses to build when the markers per file differ
  from the list it carries.

### The changes

The markers in `dwg.c` and `common.c` were written when those two were the
only changes, and cite this list as "The two changes"; it is this one.

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
3. **`vendor/libredwg/src/in_dxf.c`**, 2026-09-23 — the VERTEX_2D upgrade
   chain knows `AcDbPolygonMeshVertex`, the subclass marker AutoCAD writes on
   a polygon mesh's vertices. Without it the object stayed a VERTEX_2D whose
   subclass list does not hold that name, which is a *critical* error: one
   polygon mesh made the whole DXF unreadable.
4. **`vendor/libredwg/src/dynapi.c`**, 2026-09-23 — the same name added to
   VERTEX_MESH's row in `dwg_name_subclasses[]`, which is the table the check
   above consults.
5. **`vendor/libredwg/src/common_entity_data.spec`**, 2026-09-23 — the R2004+
   entity colour reads its RGB before its transparency, the order LibreDWG's
   own `bit_read_ENC()` uses. With the two the other way round, an entity
   carrying both had its colour and its transparency swapped.
6. **`vendor/libredwg/src/in_dxf.c`**, 2026-09-24 — a HATCH spline edge's
   weights are read from group 42, where the format writes them; upstream
   read no 42 on an edge path and left them at 0. And before R2010, where a
   spline edge has no fit data, the 97 after one is taken as the path's count
   of boundary objects, not as a fit-point count that made up a fit point and
   tangents of 0.
7. **`vendor/libredwg/src/in_dxf.c`**, 2026-09-24 — an entity's transparency
   (DXF 440) is stored in `alpha_raw` and split into method and alpha as the
   DWG decoder splits it; upstream never set `alpha_raw` and read the two
   bytes the wrong way round, so every stated transparency read as BYLAYER.
8. **`vendor/libredwg/src/common_entity_data.spec`**, 2026-09-24 — an R13/R14
   entity whose "BYLAYER" bit is clear gets linetype flags 3 (by handle);
   upstream set the flags only when the bit was set, so it read as BYLAYER.
9. **`vendor/libredwg/src/dwg.spec`**, 2026-09-24 — an R2010+ ATTRIB no longer
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
