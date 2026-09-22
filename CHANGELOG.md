# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Work towards 0.3.0 "Readable" (see `docs/VLM_EXPORT_DESIGN.md`).

### Added

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
- Polyline geometry (`uncad::geom`): `ocs_to_wcs` (the DXF arbitrary-axis algorithm),
  `bulge_arc`, `polyline_segments`, `polyline_length`, `polyline_signed_area` /
  `polyline_area` (shoelace plus each arc's circular segment, signed by orientation),
  `polyline_bounds` (arc extremes included) and `is_simple`. `LwPolylineEntity` gains
  `bulges`, `widths`, `const_width`, `elevation` and `extrusion`, with `length()`,
  `area()` and `signed_area()`; POLYLINE_2D bulges are collected from its VERTEX_2D
  subentities. The renderer draws bulges as SVG arcs instead of chords (the 25-arc
  revision cloud in `example_2000.dwg` was a 25-gon). The design document's worked
  example -- a 100 x 50 outline with one 90-degree arc -- measures 305.536 around and
  5356.748 in area.
- `extrusion` on CIRCLE, ARC, LWPOLYLINE, POLYLINE_2D, INSERT and SOLID (serde default
  `(0,0,1)`), so a consumer can tell a mirrored entity apart.
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
  without spaces. The renderer's text extents (and so the crop) come from a
  0.6-em-per-character estimate (`uncad::text::estimate_text_box`) instead of the anchor
  point alone. Frames: entities are grouped by proximity (closer than `frame_gap`, 5 % of
  the crop diagonal, on a square grid); the largest group is the primary frame `f0` and
  every detached group with `min_frame_entities` (20) entities or a text gets its own
  overview and tile pyramid under `frames/fN/`, up to `max_frames` (8) -- a detail drawn
  beside the plan is readable at its own scale. The manifest lists `frames` (content,
  counts, overview, levels, per-frame legibility) and `frames_dropped`; a single
  connected drawing keeps one frame whose overview is `overview.png` itself. CLI
  `--frame-gap`, `--min-frame-entities`, `--max-frames`.
- Paper layouts. `tables.layouts` holds every LAYOUT (`LayoutRecord`: tab order, the
  block it draws, limits, extents, the active viewport) with its plot settings
  (`PlotSettings`: paper name and size in mm, margins, units, rotation, plot type, scale)
  read through LibreDWG's embedded-struct dynapi path, and `ViewportEntity` gains the view
  fields (`view_center`, `view_size`, `view_target`, `view_direction`, `twist` in radians,
  `status_flag`, `on`, `id`, `frozen_layers`) with `scale()`, `model_window()`,
  `model_to_paper()` / `paper_to_model()` and `is_overall()`. `uncad export` writes
  `sheets.json` and `sheets/<layout>/overview.png` for every paper layout: the sheet
  (the layout's limits, else the paper size, else the paper entities) at the profile's size
  with the layout's own entities and the model composited through each on, plan-view,
  non-overall viewport at its scale and twist, clipped to its frame, per-viewport frozen
  layers honoured; a viewport on an off, frozen or non-plotting layer (the usual way to
  hide the border) still shows its window, only the border is left out (`--no-sheets`
  skips it). The twisted-viewport fixture now carries a LAYOUT with A4 plot settings. `header.format` says `dwg` or `dxf`.
- Tests and evaluation: `tests/corpus_sweep.rs` (ignored by default) parses and renders
  every file under LibreDWG's `test/test-data` and fails on any panic; `tests/acceptance.rs`
  answers five agent questions from the package alone; `tests/export.rs` checks every
  file, the tile grid, sidecar affines and byte-identical output on a second run.
  `tests/sheets_compositing.rs` pins which viewports a sheet composites and where the
  paper lies for every plot rotation and paper unit, from the new
  `viewport_states_r2000.dxf` fixture (one viewport per state, two page setups); the new
  `radial_r2000.dxf` fixture and the corpus's `2000/TS1.dwg` cover the RADIUS, DIAMETER
  and ANGULAR_3POINT dimensions no other test file has.
  `docs/EVAL.md` records the sweep, the timings and how to rerun them.
  `uncad-cli/tests/documented_invocations.rs` runs every flag the README and `--help`
  document -- the `export` subcommand and its options included -- and the parser's
  refusals, and `tests/read_paths.rs` reads a DWG and a DXF under a Korean
  directory name (`parse` against `parse_bytes`, error kinds).
- `ExportOptions::padding` (CLI `uncad export --padding <units>`): the margin around the
  overview and around each frame's own window, in drawing units. `None`, the default,
  keeps the automatic 2 %; `Some(0.0)` puts the drawing flush against the image's edge.
  The flag used to be refused as a rendering-only option, so a package could never have
  anything but the automatic padding. The sheets keep their own zero padding -- a sheet
  is the paper exactly -- and `--lattice` stays a rendering option, since the package's
  patch size is the profile's.

### Changed

- `libredwg-sys` generates its bindings with bindgen 0.73 (was 0.72), which also
  requires prettyplease 0.3 -- see the commit for why the pair has to move
  together. The generated bitfield accessors raise one more harmless clippy lint
  (`manual_div_ceil`), allowed at crate level with the existing ones (see
  `docs/CAVEATS.md`, "Clippy").

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

### Fixed

- A DXF of R2007 or later resolved no layer name and no block reference. Every entity
  whose layer name was longer than one character came back with `layer: ""`, and every
  INSERT with `block_name: ""`, so the renderer drew nothing for any block reference,
  `blocks.json` listed no instances, ByLayer colour fell back to black and the
  layer-off / frozen / non-plotting / DEFPOINTS rules never fired. `example_2018.dxf`
  reported 65 of 72 entities on layer `""` and 0 hidden entities where the same drawing
  as DWG reported 33. LibreDWG stores a table record's name as UTF-16 for R2007+ input
  of *any* format but decodes it back only for DWG input, so the DXF reader's
  name-to-handle lookups compared `"Tavolo 3"` as `"T"`. Fixed with a local patch to the
  vendored `dwg.c`; see `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG".
- A corrupt DWG could terminate the whole process instead of returning a `ParseError`.
  LibreDWG hands every header date it decodes to `strftime()`, and Microsoft's UCRT
  `strftime` fail-fasts the process (0xC0000409, reported as
  STATUS_STACK_BUFFER_OVERRUN) when a `struct tm` field is out of range -- which a
  corrupt `TIMEBLL` makes it. One changed byte in a corpus drawing was enough, and a
  fuzz sweep hit it on 8 of 12 seeds. `cvt_TIMEBLL` in the vendored `common.c` now
  returns a fully initialized, in-range `struct tm`. The same sweep now completes
  cleanly, but a decoder that large stays a risk: `parse`/`parse_bytes` and
  `docs/CAVEATS.md` now say to parse untrusted drawings in a separate process.
- Every DIMENSION vanished from a drawing far from the origin. Above 32768 units the
  renderer writes its SVG relative to the drawing's own origin, because usvg and
  tiny-skia keep path points and transforms in `f32`; the interior of a block reference
  was exempt from that shift, and a DIMENSION's cached geometry block is placed through
  an identity transform precisely because its children already hold *world* coordinates.
  So at 2.5e8, where the `f32` step is 16 units, every dimension line, extension line and
  label was quantised away while the plain LINEs of the same drawing drew perfectly --
  silently, with `dimensions.json` still listing the dimension and no warning anywhere.
  A block reference's interior is now written about the point its own placement sends to
  the render origin, which covers the dimension case and equally a block whose geometry
  sits far from its own base point.
- An ARC whose stored angle was garbage (1e20 from a hand-written DXF, 1.4e247 from a
  corrupt one) made `to_svg`, `to_png` and `export_package` spin forever: the arc-bounds
  walk stepped one quarter turn at a time from one angle to the other, and past 2^53 the
  step stopped advancing at all. The walk is now bounded at the four quarter crossings
  any arc can have, and an angle that is not finite or exceeds 1e6 radians -- where an
  `f64` no longer resolves a radian -- leaves that entity undrawn.
- `NaN` and `inf` reached SVG attributes (`x1="NaN"`, `r="inf"`), neither of which is in
  SVG's number grammar, and a block reference scaled by 1e-300 printed a 300-digit
  decimal in its `transform`. An entity whose coordinates, radius or scale are not real
  numbers is now left undrawn (a polyline keeps the vertices that are), and every number
  written into the document goes through a filter that maps a non-finite or negligible
  value to `0`.
- An ELLIPSE's stored start and end parameters were ignored, so every elliptical arc was
  closed into a full oval -- a half-round slot end came out as a complete ring. The arc
  is now drawn over its own parameter range as an SVG elliptical-arc path, and its
  extent is the arc's own rather than the whole ellipse's.
- MTEXT dropped empty lines, so every line after a paragraph break (`\P\P`) was drawn one
  line height too high while `text_plain` and the extent estimate still counted it. A
  blank line now keeps its line height.
- Bottom-attached MTEXT (DXF 71 = 7/8/9) was drawn about a third of a line too low: the
  descender term had the wrong sign, and disagreed both with single-line TEXT's bottom
  alignment and with the box `estimate_mtext_box` reports. The attachment row now places
  the same cap band in the renderer and in the estimate, with the bottom row putting the
  text's bottom -- descender included -- on the anchor.
- `docs/CAVEATS.md` claimed "Not supported: ACAD_PROXY_ENTITY, and nothing else". MINSERT,
  TRACE, POLYLINE_MESH, SHAPE, BODY and OLE2FRAME are not converted either, and draw
  nothing; they are reported through `unsupported_types` like any other unsupported type.
  The documentation now says so.

- Every DIMENSION of an R13/R14 drawing exported a measurement of 0. Only an
  `act_measurement` of exactly -1.0 was taken as "not computed", but an R13/R14 DXF
  (and any DXF without a group 42) leaves the field at 0.0 instead, and that 0 was
  reported as the drawing's own measurement with `measurement_source:
  "act_measurement"`, `confidence: "stored"` and `capabilities.dimension_values:
  "exact"` -- beside a `display` reading "1504,68". A stored value is now refused when
  it cannot be a measurement of that dimension (the -1.0 sentinel; 0 or a negative
  value on any kind but ORDINATE, which is a signed offset; a value disagreeing with
  the definition points by more than 1 %, where the corpus's worst honest disagreement
  is 2.3e-7), and the definition points supply it instead. `confidence` now follows the
  source -- `stored`, `exact` or `unavailable` -- and `capabilities.dimension_values`
  has a fourth value, `computed`, for a package whose values were all recomputed.
- A DIMSTYLE asking for whole numbers was overruled by the header. `DIMDEC` 0 (and
  `DIMADEC` 0) is the ordinary metric setting, and it was read as "unset": a style that
  asks for "50" was formatted at `$DIMDEC`'s precision as "50.0000", a string the
  picture, the cached label and `strings.json` all disagree with. A style record the
  file wrote is now taken at its word, 0 included; a husk -- a record naming a style
  whose body the file never wrote, which LibreDWG's DXF reader hands over as all zeros
  -- still falls back to the header (see `docs/CAVEATS.md`). Label precision is also
  clamped to the DXF maximum of 8 decimals, so a `DIMADEC` of -1 read back from the
  16-bit field no longer formats every label to 65 535 places.
- Text drawn by an ACAD_TABLE or a TOLERANCE reached no record and no `strings.json`
  key. Both put readable strings in the picture, with ids the export mints, and
  `collect_texts` matched only TEXT, ATTRIB, MTEXT and INSERT -- so a reader following
  the manifest's own instruction to look a value up in `strings.json` could not find
  what a table cell plainly shows. Indexing them also lets the legibility model see
  them, so a drawing whose only small text is in a table now zooms deep enough to read
  it.
- Building the records was quadratic in entity count. Every dimension, geometry and
  block record looked its entity's extent up with a linear scan over all of them,
  although the map the tiles are culled with was already built: a generated
  100 000-LINE drawing spent 59 s in the export phase where 25 000 spent 8.6 s, and now
  spends 12 s. The largest sample (`AutoCADSamples5.dwg`) went from 27.0 s to 17.6 s.
- One large closed polyline could hold the whole export. The self-intersection test
  compares every pair of segments, and ran on every closed polyline whatever its size:
  a single 64 000-vertex contour (a surveyed boundary, a GIS import) took 175 s for the
  one boolean it produces, where 16 000 took 9 s. Above 2 000 vertices the test is
  skipped and the record says `simple: null` with `confidence: "estimated"` and a
  `why`, rather than claiming an outline is simple without having looked; the same
  drawings now export in 1.0 s and 0.7 s.
- A tile sidecar could pass the documented 32 KB cap and report itself complete. The
  shrink loop cut only the record rows, so a plan of 900 layers with 45-character
  names -- each drawn across the sheet, so every tile sees nearly all of them -- wrote
  43 866-byte sidecars with `records_truncated: false`. The layer list is now cut too,
  after the rows, and a sidecar carries `layers_truncated` (and `layers_total` when it
  is set) beside `records_truncated`, so it says which of the two a reader is missing.
- A layout name longer than the filesystem's 255-byte component limit failed the whole
  export and left a directory with no `manifest.json` -- not a package, and not
  something the next run could clear either, since clearing reads the manifest. The
  sheet directory name is capped at 100 characters (the full name stays in
  `sheets.json` and the manifest), and `manifest.json` is now written last, after every
  file it names.
- Every tile rectangle of a small drawing was the same rectangle. The world boxes in
  `tiles.json`, the sidecars and the records were rounded to `$LUPREC` decimals (3 at
  the least), which says how precisely the drawing's units are *displayed* and nothing
  about how far the package zooms into them: on a drawing four thousandths of a unit
  across -- a jewellery detail, a PCB pad -- all 30 tiles of a level printed
  `[0, 0, 0.004, 0.003]`, and the `world` a consumer read described a rectangle 48 px
  away from the one the `world_to_px` beside it maps. The decimals now have a floor
  taken from the scale: enough that one unit in the last place is a thousandth of a
  pixel at the deepest level the package can reach. A drawing of ordinary size is drawn
  at a fraction of a pixel per unit, so its floor is the three or four decimals it was
  already written with.
- `--output`, the long form of `-o`, appeared in neither `--help` nor the README, so the
  only documented spelling was `-o`. Both now list it, in both commands.
- A panic inside the rasterizer took the process with it. tiny-skia's scan converter
  asserts rather than returning an error when a path's coordinates overflow its
  fixed-point edge list, and the export ran it on worker threads, where the panic came
  back as an `expect` on the join. Every `resvg::render` call is now caught and reported
  as `PngError::RenderPanic` with the panic's own message, and a tile thread that panics
  ends the export with that error rather than aborting.
- A RAY or an XLINE could kill the process. Both are infinite lines, and the renderer
  drew them as a segment 1e6 drawing units long -- a length that is really a coordinate:
  in a drawing a few thousandths of a unit across, or at a deep tile level, that endpoint
  lands 1e12 pixels off the canvas, overflows tiny-skia's fixed-point scan converter and
  panics inside the dependency (`assertion failed: edges[curr_idx].last_y >= curr_y`),
  for the export inside a worker thread, which took the whole run down with it. An
  infinite line is now drawn exactly as far as the image shows: the entity emits a
  placeholder (like the stroke widths) carrying its base point, direction and the block
  matrices above it, and every assembly path -- the SVG, a tile, a sheet's paper and the
  model inside each viewport, where the viewport's own matrix is composed in -- clips it
  to that document's viewBox grown by a small margin, dropping it entirely when the
  window shows none of it. A RAY still starts at its base point and runs one way, an
  XLINE both; the extent of either is still the base point alone, so neither enlarges the
  crop, and both are now kept in a tile or a viewport whose window their extent misses
  but their line crosses. New fixture `infinite_lines_r2000.dxf`.
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
- The DOS-era BIG5 (24) and GB2312 (31) code pages paired every byte, ASCII included, so
  a DWG or DXF declaring either lost all its table names and parsed to zero entities; they
  now pair only bytes >= 0x80, GB2312's EUC-CN bytes are looked up in the 7-bit form its
  table uses (so `中国` decodes instead of becoming U+FFFD), and CP932 (22, DOS Shift-JIS)
  is decoded as the double-byte encoding it is instead of one byte at a time.
- A control character in a text (a raw byte below 0x20 other than tab/LF/CR, or a `%%nnn`
  code for one) made `to_png` fail with `InvalidSvg` and `export_package` abort with an
  empty directory, because the SVG carried a character XML forbids. `text_plain` now marks
  such a character with U+FFFD, `%%001`..`%%031` (other than 9/10/13) are left as written
  like any other non-code, and the SVG writer strips whatever still reaches it.
- `parse()` opens files under paths with non-ASCII characters on Windows
  (`docs/CAVEATS.md`, "Fixed: a path with non-ASCII characters ...").
- A POLYLINE_PFACE face whose vertex index is 0 ("no vertex") made `parse()` panic
  with an integer overflow in debug builds (`example_2000.dwg` and `example_2018.dwg`
  from the LibreDWG corpus); release builds silently wrapped the index instead.
- LWPOLYLINE `closed` is read from bit 512 of `flag`, LibreDWG's in-memory closed bit,
  instead of bit 1 (which marks a stored extrusion). Every closed LWPOLYLINE in a DWG was
  exported and drawn open before, and mirrored open ones as closed
  (`docs/CAVEATS.md`, "The polyline closed flag").
- `polyline_signed_area` / `polyline_area` (and so `LwPolylineEntity::area()` and the
  package's `area` / `orientation`) applied the bulge stored on the last vertex of an
  *open* polyline to the straight segment that closes it for the area, which AutoCAD
  leaves behind after BREAK/TRIM: a 0.77 x 0.45 in sketch reported 63646 in^2. The
  helpers take the `closed` flag now, like `polyline_length` and `polyline_bounds`.
- Two-line angular and ordinate DIMENSIONs read from DXF input got the wrong definition
  points: LibreDWG's DXF reader maps groups by code where its DWG decoder follows the
  stream order, so `line2_end` held the arc point and the sector probe lay on the line
  itself (`example_2000.dxf` 43B measured 42.3 degrees from its points instead of 108),
  and the ordinate's X/Y type -- bit 64 of group 70 in a DXF, the stream-only `flag2`
  in a DWG -- was never read, so every DXF ordinate was a Y datum. `definition_point`
  is the arc point (DXF 16) for `ANGULAR_2LINE` from both readers now, and the model
  docs say so.
- TOLERANCE (a GD&T feature control frame) rendered invisibly in every R2000+ file:
  LibreDWG decodes its `height` for R13/R14 only, so `text_height` was 0 and the SVG
  carried `font-size="0"`. The height is the DIMSTYLE's `DIMTXT` now (the new
  `ToleranceEntity::dimstyle` names it), else the header's, else 1.0.
- Polyline arcs were drawn as their mirror image across the chord: the renderer emitted
  SVG sweep flag 1 for a positive (counter-clockwise) bulge, so every fillet, slot,
  rounded corner and revision-cloud scallop bent the wrong way -- away from the space
  the crop reserved for it (the ARC entity was right all along). A polyline in a
  mirrored OCS (extrusion `(0,0,-1)`) now has its bulges negated together with its
  vertices, since the reflection reverses each arc's turn; `bulges` are documented as
  world-orientation values. New fixture `mirrored_bulge_r2000.dxf`.
- The extent of a CIRCLE, ARC, ELLIPSE or bulged polyline inside a block reference
  rotated by other than a multiple of 90 degrees was measured from two corners of its
  box, so a circle in a block inserted at 45 degrees had a zero-width extent and a
  door swing lost its far half: `--crop raw` cut it off, `blocks.json` boxes were short
  and the tiles the swing crossed were rendered blank. All four corners are measured
  now (conservative by up to sqrt 2 at 45 degrees, never short).
- A rotated block nested inside a mirrored (negative x scale or extrusion `(0,0,-1)`)
  or non-uniformly scaled block reference had its bounds, crop, tile membership,
  `blocks.json` box and `texts.json` anchors reflected about the parent's insertion
  point, although the picture (nested `<g transform>` groups) was right: the composed
  transform added rotations and multiplied scales, which is only valid for a uniform
  scale. Both the renderer's transform and the export's affine are full 2 x 3 matrices
  composed by multiplication now. A nested text's estimated box goes through the same
  matrix, and its `rotation_deg` is the orientation of its glyphs (the transformed up
  axis), so an upright mirror-written label stays 0.
- Text was drawn 27 % too small: the CAD text height (the height of the capitals) was
  used as the SVG em size. TEXT, ATTRIB, MTEXT, TOLERANCE and dimension labels are now
  drawn at `font-size = height / 0.733`, the bundled face's cap-height ratio
  (`png::BUNDLED_CAP_HEIGHT`, from the font's OS/2 table), so a height-2.5 label has
  2.5-unit capitals; the justification offsets are one height for top and half for
  middle. MTEXT baselines are spaced 5/3 of the height (AutoCAD's single spacing, and
  what the extents estimate already assumed) instead of 1.2. `texts.json` records keep
  the drawing's `height`; their measured boxes reflect the larger glyphs, and the 0.6-em
  estimate (`text::CHAR_ADVANCE`) scales with the font size.
- A TEXT, ATTRIB or TOLERANCE whose stored height is 0 was written with
  `font-size="0"` and silently dropped by the rasterizer; it is drawn at height 1, as
  MTEXT already was.
- Justified text is drawn at its alignment point (`text-anchor` middle/end, baseline
  offset for middle/top/bottom): a center- or right-justified TEXT/ATTRIB used to be
  anchored at its left-baseline point, i.e. displaced by up to its own width (541 of
  861 texts on one sample drawing). MTEXT is rotated by its `x_axis_dir` (always drawn
  unrotated before, `docs/CAVEATS.md` "Text placement") and positioned by its
  attachment point instead of always top-left. Invisible ATTRIBs are no longer drawn.
- Drawings far from the origin lost their lines and got garbled text in every PNG, tile
  and sheet, silently: the SVG carried absolute world coordinates and usvg/tiny-skia keep
  path points in `f32`, so at 1e7 units the 1.25 px strokes collapsed and at 2.5e8 (a
  millimetre plan at projected coordinates) the glyph outlines were quantized to 16 mm.
  The renderer now writes every coordinate relative to the drawing's own origin -- the
  rounded per-axis median of its entities' reference points, used only when it exceeds
  32768 units, so every drawing near the origin keeps its SVG byte for byte -- and
  reports it as `ToSvgResult::origin` / `ToPngResult::origin` (SVG user units = world
  minus origin; `view_box` and every JSON record stay in world units); the package's
  `manifest.json` carries `drawing.svg`'s origin as `svg_origin`. A composited sheet
  folds the model's and the paper's origins into the viewport matrix.
- On a composited sheet the paper's and the model's hatch pattern definitions shared ids
  (`hp0`, `hg0`, ...), so a paper-space hatch was filled with the model's pattern (the
  legend swatches of `AutoCADSamples1.dwg`'s Layout1 came out blank), and the model's
  pattern lines were not scaled by the viewport, so a 1:16 viewport drew them 0.08 px
  wide. A paper render prefixes its ids with `p`, and every composited viewport gets its
  own copy of the model's defs (ids suffixed with the viewport handle) with the pattern
  strokes scaled by its scale. New fixture `hatched_viewport_r2000.dxf`.
- The sheet rectangle ignored the plot origin (DXF 46/47). `PlotSettings::sheet_rect`
  placed the paper by the margins alone, so with the usual "origin = minus the margins"
  page setup the exported sheet was shifted by a margin and the title block's top and
  right edges fell off `sheets/<layout>/overview.png` (six of seven AutoCAD-written
  samples). The export now takes the layout's own `LIMMIN`/`LIMMAX` first
  (`rect_source: "layout_limits"`; AutoCAD keeps them equal to the paper's placement,
  rotation included) and `sheet_rect` folds the offset in as ezdxf does for the
  `paper_size` fallback.
- A paper layout with no paper size, no limits and nothing but point-like content (a
  lone POINT or a zero-length LINE in `*Paper_Space`, the R13/R14 and OBJECTS-less DXF
  case) failed the whole export with `rendering failed: render size is zero`, leaving a
  half-written directory with no manifest: the sheet was fitted with zero padding, so a
  zero-size rectangle gave an infinite scale. Such a layout is skipped now with an
  `UnusableSheet` warning naming it, and the rest of the package is written.
- Two paper layouts whose names differ only outside `[A-Za-z0-9_-]` shared one sheet
  image: the directory came from the sanitised name alone, so every all-Hangul name of
  the same length (평면도 / 입면도, the usual Korean set) became `___`, the second layout's
  `sheets/___/overview.png` overwrote the first's, both `sheets.json` entries pointed at
  the survivor and `manifest.files` listed that path twice with two byte counts; an empty
  layout name wrote `sheets//overview.png` into the manifest while the file landed
  elsewhere. Sheet directories are unique now -- an empty name becomes `sheet`, a repeat
  takes `_2`, `_3` in tab order -- so the manifest never lists a path twice.
- The attribute values of a block reference nested inside another block -- a tag block
  inside an assembly, the standard CAD pattern -- reached no record: `texts.json` and
  `strings.json` listed only top-level attributes, so "which door is D-101" could not be
  answered, and for DWG input (where the value hangs off the nested INSERT rather than
  being a child of the block) the text was not even drawn. Both shapes are now collected
  and rendered once each, under the id `<insert>/<attrib>`. New fixture
  `nested_attrib_r2000.dxf`.
- A long Hangul (or wide-glyph) text vanished from tiles it reaches: frames, frame
  overviews and tiles are culled by the renderer's extents, which hold the 0.6-em
  estimate, while `texts.json` lists a text's tiles from its measured glyph box -- so a
  record said the text is on a tile the picture drew without it, and a detached group's
  own frame could cut the string. The measured boxes now widen the drawn extents (per
  top-level entity or INSERT) before the frames and the tile culling are computed.
- A drawing whose whole content is one point -- a single POINT, coincident entities, or
  only RAY/XLINE entities, which contribute just their base point -- was rendered into a
  window 5e-11 units wide at 2.4e13 px/unit: every image blank, `crop.padding_units`
  2e-11, and every `world` box in tiles.json and the sidecars collapsing to zero size
  when it was rounded, so the affines no longer matched it. The padding of a zero-size
  rectangle is now at least half a unit whatever scale it is seeded with (the plain PNG
  path too), the package gives such content a ten-unit window, and the scale is capped,
  so the entity is visible at a sane scale and the boxes are real rectangles.
- A tile sidecar's `layers_present` listed only the layers of the texts, dimensions and
  block instances on the tile, so a tile drawn from geometry alone -- the usual case --
  reported no layers at all and an agent filtering tiles by layer skipped it. It now
  covers every record the tile shows, geometry and regions included, and is computed
  before the size trim so cutting rows never shortens the layer list.
- Tile sidecars broke their own 32 KB cap: the trim loop measured the compact JSON but
  the file was written pretty-printed, about 3.3x larger, so a dense drawing's sidecars
  reached 100 KB *and* dropped a quarter of their record rows (`records_truncated: true`)
  to satisfy a limit the file then exceeded threefold. Sidecars are written compact now,
  like the record shards, so the measured and the written form are the same file.
- The package wrote world rectangles in two shapes: `manifest.json`'s `overview.world`,
  `frames[].content`, `crop.*` and everything in `sheets.json` came out of serde's derive
  as `{"min_x": .., "min_y": .., "max_x": .., "max_y": ..}`, while tiles.json, every
  sidecar `world`, every record `bbox` and `manifest.sheets[].rect` used the
  `[x0, y0, x1, y1]` array the design documents -- the same layout's rectangle read one
  way in the manifest and the other in `sheets.json`. `crop::Rect` serializes as that
  array now, and reads both forms back, so an 0.3.0 document still loads.
- Re-exporting into a directory that already held a package left the previous run's
  files beside the new ones -- record shards (`texts.003.json`), deeper tile levels and
  their sidecars, sheets of layouts that no longer exist -- all valid-looking and none of
  them in the new `manifest.json`, so a consumer that walks the tree (as the generated
  README.txt invites) mixed two exports. `export_package` now clears what the previous
  `manifest.json` listed, and the `frames/`, `sheets/` directories that empties, before
  it writes; a directory without an uncad manifest, and any file such a manifest does not
  list, is left untouched.
- `manifest.guidance` quoted "224 px overlap" whatever the profile was, contradicting
  `frames[].levels[].overlap_px` (392 for `claude-hires`, 320 for `openai-patch`). The
  sentence is built from the profile in use now and names the tile size as well.
- The `TinyOverview` warning tested the overview's long edge, which the profile always
  keeps at several hundred pixels, so it never fired; it now tests the short edge (a
  500:1 drawing's 700 x 28 px overview is reported).
- The CLI took an option it did not know as the input file, or dropped it in silence.
  `parse_args` matched only the render flags and ended in `other if input.is_none() =>
  input = other` and `_ => {}`, while `uncad export` scanned its own eleven options in a
  second loop that also ended in `_ => {}`: `uncad export --max-levels 2 d.dwg -o out`
  failed with "cannot open input file '--max-levels'", `uncad export d.dwg -o out
  --max-level 1 --sharkb 1` exited 0 and wrote the default package, and a second input
  path or an option missing its value went the same way. One parser now owns every flag
  of both commands, so an option is either understood wherever it stands or refused by
  name: an unknown option, a second positional and a missing value are errors, an
  option of the other command names the command it belongs to (`--fit` is not an export
  option; `--max-levels` is one), and `--shard-kb`'s error says kilobytes instead of
  pixels. `uncad export` accepts `--no-trim` and `--fonts` as the usage said it should.

### Removed

- The `regex` dependency: the MTEXT code stripper it powered is replaced by
  `uncad::text`.
- `ParseError::InvalidPath`: paths are no longer passed to C, so a path that is not
  UTF-8 or contains a NUL byte is no longer a distinct failure (the OS reports it
  through `ParseError::Io`).

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
