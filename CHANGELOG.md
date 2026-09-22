# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Work towards 0.3.0 "Readable" (see `docs/VLM_EXPORT_DESIGN.md`).

### Added

- Paper layouts. `tables.layouts` holds every LAYOUT (`LayoutRecord`: tab order, the
  block it draws, limits, extents, the active viewport) with its plot settings
  (`PlotSettings`: paper name and size in mm, margins, units, rotation, plot type, scale)
  read through LibreDWG's embedded-struct dynapi path, and `ViewportEntity` gains the view
  fields (`view_center`, `view_size`, `view_target`, `view_direction`, `twist` in radians,
  `status_flag`, `on`, `id`, `frozen_layers`) with `scale()`, `model_window()`,
  `model_to_paper()` / `paper_to_model()` and `is_overall()`. `uncad export` writes
  `sheets.json` and `sheets/<layout>/overview.png` for every paper layout: the sheet
  (from the paper size, else the limits, else the paper entities) at the profile's size
  with the layout's own entities and the model composited through each on, plan-view,
  non-overall viewport at its scale and twist, clipped to its frame, per-viewport frozen
  layers honoured; a viewport on an off, frozen or non-plotting layer (the usual way to
  hide the border) still shows its window, only the border is left out (`--no-sheets`
  skips it). The twisted-viewport fixture now carries a LAYOUT with A4 plot settings. `header.format` says `dwg` or `dxf`.
- A bundled font and measured text boxes. `to_png`, the tiles and the package's text
  metrics are shaped with `Uncad Sans`, a 370 KB subset of Noto Sans KR (OFL 1.1; Latin,
  Greek, the 2350 KS X 1001 Hangul syllables, the CAD symbols `∅ ° ± ² ³ Ø ㎡ ㎜ ㎥`) embedded
  in the crate, so images are the same on every machine and Hangul labels no longer
  depend on the host (`Fonts::Bundled`, the default; `Fonts::BundledAndSystem` / CLI
  `--fonts bundled+system` adds the host's fonts for characters the subset lacks). Every
  `<text>` in the SVG carries `font-family="Uncad Sans"` and an `id` (its handle, or
  `insert/handle` inside a block). `texts.json` boxes are measured from the shaped glyph
  outlines through usvg (`bbox_confidence: "measured"`, `font_ok`, `unshaped_glyphs`), the
  0.6-em estimate remaining only for the crop and for texts usvg drops.
- Tests and evaluation: `tests/corpus_sweep.rs` (ignored by default) parses and renders
  every file under LibreDWG's `test/test-data` and fails on any panic; `tests/acceptance.rs`
  answers five agent questions from the package alone; `tests/export.rs` checks every
  file, the tile grid, sidecar affines and byte-identical output on a second run.
  `docs/EVAL.md` records the sweep, the timings and how to rerun them.
- The package (`uncad::export::export_package`, CLI `uncad export <input> -o <dir>`): the
  LLM/VLM output directory of the design -- `manifest.json`; `overview.png` fitted to the
  profile's edge and patch budget (Claude: 1568 px / 1568 patches of 28 px); a tile
  pyramid (`frames/f0/tiles/z*/rRR_cCC.png`, 1092 px with 224 px overlap, edge tiles
  shifted inward, empty tiles listed but not written, depth chosen so the dominant text
  height reaches 14 px) with a JSON sidecar per tile (world rectangle, both affines,
  neighbours, parent and children, the texts, dimensions, blocks and regions on it with
  pixel boxes); and the records -- `texts.json` (block contents included, ids
  `<insert>/<child>`), `dimensions.json`, `geometry.json`, `regions.json` (area,
  perimeter, centroid, the texts inside), `blocks.json`, `strings.json` (normalised
  string -> ids), `drawing.json`, `report.json` (excluded and hidden entities with
  reasons), `tiles.json`, `README.txt`. `--svg` / `--full` add `drawing.svg` /
  `entities.json`; record files above `--shard-kb` (96) are sharded and indexed in the
  manifest; profiles `claude`, `claude-hires`, `openai-patch`. Output is deterministic
  except for the timings in `report.json`. Each tile rasterizes only the entities whose
  extent touches it, in parallel per level; `strings.json` keys are NFKC-normalised
  (`㎡` -> `m2`, full-width digits, fraction slash) and every string is also indexed
  without spaces. The renderer's text extents (and so the crop) use the same 0.6-em
  estimate as `texts.json` (`uncad::text::estimate_text_box`) instead of the anchor
  point alone. Frames: entities are grouped by proximity (closer than `frame_gap`, 5 % of
  the crop diagonal, on a square grid); the largest group is the primary frame `f0` and
  every detached group with `min_frame_entities` (20) entities or a text gets its own
  overview and tile pyramid under `frames/fN/`, up to `max_frames` (8) -- a detail drawn
  beside the plan is readable at its own scale. The manifest lists `frames` (content,
  counts, overview, levels, per-frame legibility) and `frames_dropped`; a single
  connected drawing keeps one frame whose overview is `overview.png` itself. CLI
  `--frame-gap`, `--min-frame-entities`, `--max-frames`.
- The crop rule (`uncad::crop`, design section 4): every visible entity's world extent
  is measured while rendering; `CropMode::Auto` shows all of them minus *outliers* (at
  most `max(3, 1 %)` entities: the largest when they dwarf the rest of the drawing 20x,
  or those more than 20 diagonals away from it), and uses the
  header `$EXTMIN/$EXTMAX` instead when those are sane and cover more; `Raw`, `Header`
  and `Fixed(rect)` are explicit. Padding is automatic (2 % of the longer side, at least
  24 px in a PNG) unless `padding: Some(units)`. PNG sizes are rounded up to a multiple
  of `ToPngOptions::lattice` (28 px, Claude's patch size; 0 = off) with the world
  rectangle grown to match, so `view_box.width * px_per_unit == width` exactly and the
  `ViewBox` affine round-trips. `ToSvgResult::crop` / `ToPngResult::crop` report the
  source, the rectangle, the tight content bounds, the padding, the header extents and
  every excluded entity with its reason (`scale_outlier`, `far_outlier`,
  `outside_crop`). CLI: `--crop auto|raw|header|x0,y0,x1,y1`, `--padding`, `--lattice`;
  `--no-trim` stays as `--crop raw`. The 3256x INSERT of `example_2000.dwg` /
  `example_2018.dwg` is now a listed exclusion instead of a 3.4-million-unit viewBox.
- Visibility (`uncad::visibility`): `hidden_reason(common, tables)` says why the drawing
  does not show an entity -- its own invisible flag, the `DEFPOINTS` layer, a layer that
  is off, frozen or non-plotting -- and `lineweight_mm` decodes the lineweight codes.
  `LayerRecord` gains `on`, `frozen`, `locked`, `plot`, `lineweight_mm` and `linetype`;
  `EntityCommon` gains `invisible`, `lineweight_mm`, `linetype` and `ltype_scale`. The
  renderer leaves hidden entities out (block contents included) and reports how many in
  `ToSvgResult::hidden` / `ToPngResult::hidden`; `ToSvgOptions::include_hidden` (CLI
  `--include-hidden`) draws them at 50 % opacity instead. A layer's plot flag is trusted
  from R2000+ DWG files only: LibreDWG's DXF reader cannot tell an omitted group 290 from
  a cleared one, so DXF layers always read as plotting (`docs/CAVEATS.md`).
- Polyline geometry (`uncad::geom`): `ocs_to_wcs` (the DXF arbitrary-axis algorithm),
  `bulge_arc`, `polyline_segments`, `polyline_length`, `polyline_signed_area` /
  `polyline_area` (shoelace plus each arc's circular segment, signed by orientation),
  `polyline_bounds` (arc extremes included) and `is_simple`. `LwPolylineEntity` gains
  `bulges`, `widths`, `const_width`, `elevation` and `extrusion`, with `length()` and
  `area()`; POLYLINE_2D bulges are collected from its VERTEX_2D subentities. The renderer
  draws bulges as SVG arcs instead of chords (the 25-arc revision cloud in
  `example_2000.dwg` was a 25-gon). The design document's worked example -- a 100 x 50
  outline with one 90-degree arc -- measures 305.536 around and 5356.748 in area.
- `extrusion` on CIRCLE, ARC, LWPOLYLINE, POLYLINE_2D, INSERT and SOLID (serde default
  `(0,0,1)`), so a consumer can tell a mirrored entity apart.
- Dimension values (`uncad::dimension`, `DimensionEntity`): `geometry` (the kind --
  LINEAR, ALIGNED, ANGULAR_3POINT, ANGULAR_2LINE, RADIUS, DIAMETER, ORDINATE, ARC_LENGTH
  -- with its definition points), `measurement` (the stored `act_measurement`: drawing
  units before DIMLFAC, degrees for angular kinds, `None` for the R14-era `-1`
  sentinel), `measurement_from_points` (recomputed: LINEAR projected on its rotation,
  ALIGNED/RADIUS/DIAMETER distances, angular sectors chosen by the arc point, ordinate
  offsets, arc length), `user_text`, `display_text` with `display_text_raw` and
  `display_source` (suppressed / user text with `<>` substituted / the cached `*D`
  block's label / formatted by this crate), `definition_point`, `text_midpoint`,
  `dimstyle` and the effective `dimlfac`. `tables.dimstyles` holds every DIMSTYLE's
  formatting variables (`DimStyleRecord`). The built-in formatter handles decimal,
  architectural and fractional units, DIMZIN trailing-zero suppression and decimal
  degrees; the cached label is preferred whenever the file has one. Verified against
  `example_2000.dwg`: every stored value equals the recomputed one to 1e-6 and the
  labels read `1504,68` and `108°`.
- Text decoding (`uncad::text`): `decode_mtext` and `decode_text` turn the stored
  strings into readable ones -- `\P` paragraphs, `\S` stacked fractions in all three
  forms (`1/2`, `1#2`, `+0.1^-0.2`) with a space after a preceding digit so
  `3{\H0.7x;\S1#2;}"` reads `3 1/2"` (it rendered as `31/2"`), `{}` groups and
  `\A`/`\H`/`\f`/`\C`/... format codes removed, `\U+XXXX`, and the `%%c` (diameter),
  `%%d`, `%%p`, `%%%`, `%%nnn` symbol codes with `%%u`/`%%o` reported as decorations.
  TEXT, ATTRIB, MTEXT and TOLERANCE carry the result as `text_plain` next to the raw
  `text`; the renderer draws `text_plain`.
- Text placement fields: TEXT and ATTRIB gain `horizontal_alignment`,
  `vertical_alignment`, `alignment_point`, `width_factor`, `oblique_angle` and `style`;
  ATTRIB and ATTDEF gain `tag` (the attribute's name), ATTRIB `invisible`; MTEXT gains
  `attachment`, `rect_width`, `extents_width`, `extents_height`, `x_axis_dir` and
  `style`. All have serde defaults, so 0.2.0 JSON still loads.
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

- Justified text is drawn at its alignment point (`text-anchor` middle/end, baseline
  offset for middle/top/bottom): a center- or right-justified TEXT/ATTRIB used to be
  anchored at its left-baseline point, i.e. displaced by up to its own width (541 of
  861 texts on one sample drawing). MTEXT is rotated by its `x_axis_dir` (always drawn
  unrotated before, `docs/CAVEATS.md` "Text placement") and positioned by its
  attachment point instead of always top-left. Invisible ATTRIBs are no longer drawn.
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
- MTEXT text and `header.dimpost` of an R2007+ DXF were read as UTF-16 although LibreDWG
  stores both 8-bit (MTEXT's text chunks are `strdup`'d with no version branch, and the
  HEADER section is parsed before the version is known), so every MTEXT and the header's
  `$DIMPOST` came back as CJK-looking garbage that carried bytes read past the end of the
  allocation (`example_2018.dxf`'s only MTEXT read `"敔獫潴..."` instead of
  `"Teksto granda nur por testi..."`). Those strings now go through an 8-bit path
  (`uncad_bytes_to_utf8`) that decodes them as the file's own encoding -- UTF-8 for an
  R2007+ DXF, the code page otherwise -- with the `\U+XXXX` escapes expanded.
- A control character in a text (a raw byte below 0x20 other than tab/LF/CR, or a `%%nnn`
  code for one) made `to_png` fail with `InvalidSvg` and `export_package` abort with an
  empty directory, because the SVG carried a character XML forbids. `text_plain` now marks
  such a character with U+FFFD, `%%001`..`%%031` (other than 9/10/13) are left as written
  like any other non-code, and the SVG writer strips whatever still reaches it.
- `parse()` opens files under paths with non-ASCII characters on Windows
  (`docs/CAVEATS.md`, "Fixed: a path with non-ASCII characters ...").

### Changed (breaking)

- `ToSvgOptions::outlier_trim` is replaced by `crop: CropMode` and `padding` is an
  `Option<f64>` (`None` = automatic; 0.2.0's fixed 5 units is `Some(5.0)`). The overview
  is never cluster-trimmed any more: 0.2.0's dominant-cluster trim could drop real
  geometry (a detail drawn beside the plan), and now only scale/far outliers are left
  out, each reported. `assemble`/`Rendered` are crate-private and changed shape.
- Hidden entities are no longer drawn. 0.2.0 rendered every entity in the space,
  including those on layers switched off or frozen, dimension definition points on
  `DEFPOINTS` and entities with the invisible flag -- what AutoCAD's screen and plots
  leave out. `include_hidden` restores them, faded.
- Coordinates are world coordinates. Entities DXF stores in their own OCS -- CIRCLE and
  ARC centres, LWPOLYLINE/POLYLINE_2D vertices, TEXT/ATTRIB anchors, INSERT insertion
  points, SOLID corners -- are transformed on read; 0.2.0 reported the stored OCS values
  as if they were world, which put every mirrored entity (extrusion `(0,0,-1)`, what
  AutoCAD's MIRROR produces) on the wrong side of the y axis in both JSON and PNG. A
  mirrored ARC's angles are mirrored and swapped so it still runs counter-clockwise; a
  mirrored INSERT is drawn with its x scale and rotation negated. HATCH boundaries and
  DIMENSION `text_midpoint` are still as stored (`docs/CAVEATS.md`, "Coordinates are
  world").
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

- The `regex` dependency: the MTEXT code stripper it powered is replaced by
  `uncad::text`.
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
