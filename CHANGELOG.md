# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Work towards 0.3.0 "Readable" (see `docs/VLM_EXPORT_DESIGN.md`).

### Added

- PNG sizing in pixels: `ToPngOptions { size: PngSize, background: Background,
  stroke_px, max_edge }`. `PngSize::FitLongEdge(px)` (the default, 1568 px -- the
  largest a Claude standard-tier image keeps unresized), `PxPerUnit(f64)` and
  `Scale(f64)` (0.2.0's units-times-factor rule). `Background::White` (the default,
  written as 8-bit RGB) or `Transparent` (8-bit RGBA). `stroke_px` (default 1.25)
  sizes every stroke in output pixels. `max_edge` (default 8000) refuses larger images
  with `PngError::TooLarge` instead of allocating them; `PngError::InvalidSize` rejects
  a non-finite or non-positive size. `ToPngResult` gains `width`, `height`,
  `view_box` and `px_per_unit`, and `ToSvgResult` gains `view_box`
  (`uncad::ViewBox`, with `world_bounds`, `world_to_px` and `px_to_world`), so a
  consumer can map a pixel back to drawing coordinates. CLI: `--fit <px>`,
  `--ppu <n>`, `--bg white|transparent`, `--stroke <px>`, `--max-edge <px>`.
- `uncad::color::contrast_on_white`: the renderer darkens colours whose luminance
  exceeds 0.45 (ACI yellow `#ffff00` draws as `#828200`, cyan as `#00a4a4`) so they read
  on the white page; the model and JSON keep the file's colours.
- `CadDatabase::header` (`uncad::Header`, module `uncad::header`): the file version
  (LibreDWG's name, e.g. `r2004`) and code page, `$INSUNITS` resolved to
  `uncad::Units { name, to_mm }` from the DXF reference table (0 = unitless = `"du"`),
  `$MEASUREMENT`, `$LUNITS`/`$LUPREC`/`$AUNITS`/`$AUPREC`, `$EXTMIN`/`$EXTMAX`,
  `$LIMMIN`/`$LIMMAX`, `$PEXTMIN`/`$PEXTMAX`, `$PLIMMIN`/`$PLIMMAX`, `$DIMSCALE`,
  `$DIMLFAC`, `$DIMDEC`, `$DIMLUNIT`, `$DIMPOST`, `$DIMRND`, `$DIMZIN`, `$DIMFRAC`,
  `$DIMAUNIT`, `$DIMADEC`, `$DIMTXT`, `$DIMASZ`, `$LTSCALE`, `$TEXTSIZE` and `$CLAYER`.
  Serialized under `"header"` in `to_json`; a document without it (0.2.0) still
  deserializes with the default header. `CadDatabase::new(entities, tables)` builds a
  database with that default header. The CLI summary prints the version, code page and
  units.
- `uncad::parse_bytes(bytes, Format)` and `uncad::Format` (`Dwg`/`Dxf`, with
  `Format::from_path`): decode a drawing already held in memory. `parse()` now reads the
  file itself and calls it.
- `ParseError::Io(std::io::Error)` for a file `parse()` cannot read; `ParseError` now
  implements `Error::source`.
- `libredwg-sys`: shims `uncad_dwg_read_bytes`/`uncad_dxf_read_bytes` (memory-based
  copies of `dwg_read_file`/`dxf_read_file`), `uncad_dwg_version`/`uncad_dwg_from_version`/
  `uncad_dwg_codepage`/`uncad_dwg_is_tu` (file-header accessors for the opaque
  `Dwg_Data`), `uncad_tv_to_utf8`/`uncad_entity_tv_to_utf8`/`uncad_free_string`
  (code-page to UTF-8 through LibreDWG's own `bit_TV_to_utf8`), and the
  `dwg_resolve_handle` binding.
- Tests: `crates/uncad/tests/read_paths.rs` (a DWG and a DXF under a Korean directory
  name, `parse` vs `parse_bytes` equality, error kinds).

### Fixed

- LWPOLYLINE `closed` is read from bit 512 of `flag`, LibreDWG's in-memory closed bit,
  instead of bit 1 (which marks a stored extrusion). Every closed LWPOLYLINE in a DWG was
  exported and drawn open before, and mirrored open ones as closed
  (`docs/CAVEATS.md`, "The polyline closed flag").
- A POLYLINE_PFACE face whose vertex index is 0 ("no vertex") made `parse()` panic
  with an integer overflow in debug builds (`example_2000.dwg` and `example_2018.dwg`
  from the LibreDWG corpus); release builds silently wrapped the index instead.
- Text, layer names and block names in pre-R2007 DWGs and in pre-R2007 DXF input are now
  decoded through the file's code page instead of lossily as UTF-8, so Korean text in
  R2000/R2004 drawings and the plus-minus/degree signs in dimension text no longer come
  out as U+FFFD; an unmappable character becomes U+FFFD instead of truncating the string,
  and a corrupt code-page value in the file header no longer indexes LibreDWG's tables
  out of bounds (`docs/CAVEATS.md`, "Fixed: pre-R2007 and DXF text ...").
- An R2007+ DXF parsed to zero entities: LibreDWG stores its strings as UTF-16 but hands
  them out unconverted for DXF input, so `"*Model_Space"` read as `"*"` and no entity was
  selected. The same shim now converts them (same CAVEATS entry).
- `parse()` opens files under paths with non-ASCII characters on Windows
  (`docs/CAVEATS.md`, "Fixed: a path with non-ASCII characters ...").

### Changed (breaking)

- `to_png`'s defaults: the image's long edge is 1568 px (was: one pixel per drawing
  unit, so a 12.7 MB drawing rendered at 69 x 70 px and a LibreDWG corpus file tried
  to allocate 36 TB), the background is opaque white (was: transparent, which showed
  nothing when composited on black), strokes are 1.25 px (was: 1/6000th of the viewBox
  diagonal, a sub-pixel hairline at most sizes), and the output is RGB (was: RGBA).
  `ToPngOptions { scale }` is gone: use `size: PngSize::Scale(s)`. The CLI's
  `--scale` keeps its meaning; without a flag the CLI now fits to 1568 px.
- `ToSvgResult::unsupported_types` is sorted (was: `HashSet` order, different per run).
- The system font database is loaded once per process instead of on every `to_png`.
- `CadDatabase` has a third field, `header`; a struct literal without it no longer
  compiles (use `CadDatabase::new` or add `header: Header::default()`).

### Removed

- `ParseError::InvalidPath`: paths are no longer passed to C, so a path that is not
  UTF-8 or contains a NUL byte is no longer a distinct failure (the OS reports it
  through `ParseError::Io`).

### Changed

- `libredwg-sys` generates its bindings with bindgen 0.73 (was 0.72), which also
  requires prettyplease 0.3 -- see the commit for why the pair has to move
  together. The generated bitfield accessors raise one more harmless clippy lint
  (`manual_div_ceil`), allowed at crate level with the existing ones (see
  `docs/CAVEATS.md`, "Clippy").

## [0.2.0] - 2026-09-17

A breaking release: writing left the public API, and a cleanup pass renamed several
public items.

### Removed

- All DWG/DXF writing: `CadDatabase::write_dwg`/`write_dxf`, `uncad::dwg_to_dxf`,
  `WriteError`, the CLI's `-o <x>.dxf`/`-o <x>.dwg`, and `libredwg-sys`'s
  `uncad_write_dxf`/`uncad_write_dxf_file` shims and `dwg_write_file` binding. This
  project's scope is "DWG/DXF -> model -> JSON/SVG/PNG". The C build keeps LibreDWG's
  encoder sources and `USE_WRITE`, because `dxf_read_file()` depends on them (see
  `docs/ARCHITECTURE.md`, "Model").
- `CadDatabase` no longer holds LibreDWG's `Dwg_Data`: `parse()` frees it as soon as the
  conversion is done. `CadDatabase` is now a plain value deriving
  `Debug`/`Clone`/`PartialEq`, and can be constructed directly
  (`CadDatabase { entities, tables }`).
- Bindgen allowlist entries nothing in the workspace used
  (`dwg_object_get_supertype`, `dwg_object_get_name`, `dwg_object_to_entity`,
  `dwg_object_to_object`, `dwg_free_object`, `dwg_abandon`, `dwg_resolve_handleref`,
  `dwg_obj_*`, `dwg_ref_*`, `Dwg_Object_Supertype`, `Dwg_Error`,
  `dwg_field_name_type_offset`, `dwg_point_2d`, `DWG_NOERR`).

### Added

- JSON output: `CadDatabase::to_json(ToJsonOptions { pretty })`, the `uncad::json` module,
  and `JsonError`. The model (`entities` + `tables`) is serialized with serde as-is and
  reads back through `serde_json::from_str::<CadDatabase>`. Every entity carries a `"type"`
  tag holding the same DXF name `type_name()` reports (`"LINE"`, `"LWPOLYLINE"`,
  `"3DSOLID"`, ...; an unsupported type is `"UNKNOWN"` plus a `type_name` field). Every
  type in `model`/`tables`, and `Point2D`/`Point3D`, derives `serde::Serialize` and
  `Deserialize`. A HATCH's `boundary_paths` are
  `{"type":"POLYLINE"|"EDGES","data":[...]}`, and its edges
  `{"type":"LINE"|"ARC"|"ELLIPSE"|"SPLINE",...}`. The same input produces the same bytes
  (see Changed). New public dependencies: `serde` 1.x, `serde_json` 1.x.
- CLI: `uncad <input> -o <output.json>` and `--pretty`.
- Tests: JSON tag and round-trip unit tests for every `Entity` variant, a JSON round trip
  on a real DXF, CLI coverage for JSON output and `--pretty`, CLI checks that `--space`
  and `--no-trim` really change the output (`--no-trim` against a five-line DXF the test
  writes from group codes), and a test that a SAB solid yields the same wireframe in
  `entities` and in its block record.

### Fixed

- Wireframe extraction for 3DSOLID/REGION stored as SAB (ACIS BinaryFile, version 2) was
  calling LibreDWG's `dwg_convert_SAB_to_SAT1` on the live entity during `parse()`,
  mutating `Dwg_Data` in place. The second walk (`tables.block_records`) then read a
  half-converted entity and lost the wireframe, and the write path that existed at the
  time corrupted every solid it wrote back out (measured on
  `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`). The new
  `uncad_3dsolid_sab_to_sat_text` shim in `libredwg-sys` converts on a shallow copy, so
  `parse()` has no side effect on `Dwg_Data`.
- After refreshing `vendor/libredwg/` with `scripts/sync-libredwg-vendor.sh`, an
  incremental build reused stale C objects and a stale `bindings.rs`. `build.rs` now
  registers the whole vendor directory with `cargo:rerun-if-changed`.
- `uncad::tables::convert_tables` was `pub`, leaking a `*mut Dwg_Data` entry point that
  bypasses the global lock into the public API. Lowered to `pub(crate)`.

### Changed

- `Tables::{layers, block_records, mlinestyles}` moved from `HashMap` to `BTreeMap` (a
  public field type change). Iteration and JSON key order are now deterministic, so the
  same file always produces the same JSON.
- **Renamed, breaking**: the `render_model` module is now `model`, and its `RenderEntity`
  enum is now `Entity` (re-exported as `uncad::Entity`). `LeaderEntity`'s
  `is_arrowhead_enabled` field is now `has_arrowhead` and `LightEntity`'s
  `target_is_meaningful` is now `has_target` -- both are serde field names, so the JSON
  changes with them. `ToSvgResult`/`ToPngResult`'s `unsupported_entity_types` is now
  `unsupported_types`. `Entity::Polyline3D`'s type name and JSON tag changed from
  `"POLYLINE3D"` to `"POLYLINE_3D"`, matching `"POLYLINE_2D"`/`"POLYLINE_PFACE"`.
- The CLI's usage text and messages are in English, matching the rest of the project.
- `MTextEntity::rotation`'s rustdoc said the value was derived from `x_axis_dir`, which the
  code never did; corrected to "currently always 0".

### Docs

- README, `docs/ARCHITECTURE.md`, `docs/CAVEATS.md` and this changelog are in English.
- `svg.rs` was split into `svg/format.rs` (number and string formatting), `svg/hatch.rs`
  (HATCH fills) and `svg/bounds.rs` (viewBox and outlier trim), leaving the renderer
  itself in `svg.rs`.
- Comments no longer refer to the retired JavaScript/WASM predecessor this crate was
  originally ported from; the reasoning they carried is kept, the archaeology is not.
- Recorded that the `lib/libredwg` submodule is a precondition of the tests, not of the
  build, across README, ARCHITECTURE, CAVEATS and the test comments. Corrected the
  vendored file count to 112, fixed `samples/README.md` pointing at a nonexistent
  `tests/dxf_minimal.rs`, refreshed CAVEATS' test counts and CI job list, and cleaned up
  stale paths, versions and organization names in `config.h`.
- Recorded measured upstream behavior: LibreDWG's DXF reader recovers 1 of 60 entities
  from a drawing its own DXF writer produced (CAVEATS, "DXF reading").

## [0.1.0] - 2026-08-24

- First public release on crates.io (`libredwg-sys`, `uncad`, `uncad-cli`).
