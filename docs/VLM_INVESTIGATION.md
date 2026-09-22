# Investigation: handing a drawing to a VLM/LLM

Date: 2026-09-21, against main `b566920` (0.2.0 plus the bindgen 0.73 bump).
Companion document: [`VLM_EXPORT_DESIGN.md`](./VLM_EXPORT_DESIGN.md) holds the
proposal that came out of this investigation. This file records what was found,
what was measured, and which earlier assumptions turned out to be wrong.

Status note: this is a snapshot of 0.2.0. Some findings below have since been
fixed on the way to 0.3.0 -- the pre-R2007/DXF string decoding, the Windows
Hangul-path failure, the LWPOLYLINE closed bit, the missing header variables
and the POLYLINE_PFACE index overflow -- see `CHANGELOG.md` ("Unreleased") and
`docs/CAVEATS.md`; the tables are left as measured.

The question asked was: can uncad become `In (DWG or DWF) -> Out (a directory of
PNG images + JSON)` such that a vision-language model or an LLM can read the
result, with exact numbers (lengths, areas, volumes, dimension text, units)
preserved, large drawings split into overlapping tiles, and a clearly defined
rule for where an image is cropped?

## Method

1. Every file under `crates/uncad/src`, `crates/uncad-cli/src` and the C shim was
   read against the vendored LibreDWG sources (`include/dwg.h`, `src/dynapi.c`,
   `src/dwg.spec`, `src/in_dxf.c`, `src/bits.c`, `src/codepages.c`).
2. The release CLI was run over the nine AutoCAD drawings in `samples/` and four
   files from `lib/libredwg/test/test-data/`, timing every output mode and
   measuring JSON/SVG/PNG sizes, viewBoxes and pixel dimensions.
3. A throw-away probe program linked against `libredwg-sys` read header
   variables, LWPOLYLINE flags, DIMENSION fields, TEXT alignment, LAYER state
   and raw string bytes from the same files. Its ground truth for the polyline
   closed bit was `test-data/example_2000.dxf` (text, group code 70) compared
   handle by handle with `example_2000.dwg`.
4. A throw-away benchmark linked against `resvg 0.48.1` measured parse-once /
   render-per-tile against render-whole-canvas-then-crop, parallel scaling,
   PNG encodings and stroke widths.
5. Vendor documentation for the Claude, OpenAI and Gemini image inputs was
   fetched on 2026-09-21, and 2024-2026 papers on CAD/floor-plan understanding
   by VLMs were reviewed.

The probe and the benchmark are not part of the repository. Numbers below that
came from them are marked "(probe)" or "(bench)"; everything else is either in
the source or reproducible with the CLI.

## Summary

uncad 0.2.0 turns a drawing into a model that keeps "what rendering needs"
(`docs/ARCHITECTURE.md`, "Model") and dumps that model as JSON, or renders it
into an SVG whose viewBox is a per-render heuristic and a PNG whose pixel size
is the drawing's unit count. For an LLM consumer this fails on three fronts:

- **The numbers have no meaning.** No header variable is read, so nothing says
  whether `1250` is millimetres or inches. DIMENSION carries only the name of
  its cached block, not the measured value. LWPOLYLINE arcs (bulges) are
  dropped, and the closed flag tests the wrong bit, so polygon areas cannot be
  computed. Pre-R2007 strings lose every non-ASCII character.
- **The image is not reliably legible.** Pixel size is `viewBox units x scale`
  with no cap or fit option (a 12.7 MB drawing renders at 69 x 70 px, another
  aborts on a 36 TB allocation), the background is transparent, strokes are
  sub-pixel hairlines, and MTEXT fractions such as 3 1/2" come out as `31/2"`.
- **The crop is unexplained.** The viewBox is the bounding box of whatever the
  renderer happened to "consider", trimmed to a dominant cluster that can and
  does discard real content, padded by five drawing units, and nothing reports
  what was left out.

Almost every gap is a matter of reading more fields through `dwg_dynapi_*`
(already allowlisted in `build.rs`). The C-side work is small: accessors for
`Dwg_Data.header.version/codepage`, a wrapper over `bit_TV_to_utf8`, a
decode-from-memory entry point, and walkers for ACAD_TABLE cells and EED.

## 1. Extraction fidelity

What `parse()` keeps and drops, per the areas that matter for exact numbers.
"(probe)" marks claims confirmed by running against real files.

| Area | Today | Evidence | Consequence |
|---|---|---|---|
| Header variables (probe) | None read | `lib.rs` never calls `dwg_dynapi_header_value`; the function is bound. Samples 1-8: INSUNITS=1 (inches), sample 9 and `gh109_1.dwg`: 0 (unitless), the LibreDWG examples: 4 (mm). DIMLFAC=1.0 in every file probed; LTSCALE 0.5 in every sample | Every number in the JSON is unitless |
| DIMENSION (probe) | `{common, block_name}` only (`convert.rs:614-628`, `model.rs:368-371`) | `DIMENSION_COMMON` in dwg.h has `act_measurement`, `user_text`, `def_pt`, `text_midpt`, `flag`, `dimstyle`. Probe: `act_measurement` present on all 891 dimensions; 826 of them equal the definition-point geometry within 1e-6 (LINEAR must be projected on `dim_rotation`; ALIGNED is `|xline2-xline1|`; RADIUS/DIAMETER is `|first_arc_pt-def_pt|`). The 63 dimensions in samples 8 and 9 hold exactly `-1.0` (R14-style, "not computed"); their geometry reproduces the label. The 111 dimension blocks without text all have `user_text == " "` (AutoCAD's "suppress text") | The most important number in a drawing is not in the JSON. The displayed string exists only as raw MTEXT inside `block_records["*D.."]`, and the join is not 1:1 (sample 7: 195 dimensions, 114 texts) |
| LWPOLYLINE closed flag (probe) | `flag & 1` (`convert.rs:33`, `POLYLINE_CLOSED_FLAG`) | **Wrong for LWPOLYLINE.** LibreDWG's in-memory flag uses 512 = closed and 1 = "has extrusion" (`dwg.spec` LWPOLYLINE block, `dwg.h` `FLAG_LWPOLYLINE_CLOSED`); `in_dxf.c` rewrites DXF bit 1 to 512 on read. Ground truth: the 11 LWPOLYLINEs of `example_2000.dxf` (group 70) agree with bit 512 of `example_2000.dwg` 11/11 and with bit 1 0/11. Across the samples 741 polylines carry 512 and are exported open (the committed `samples/AutoCADSamples2.svg` renders handle 36A, a 4-vertex rectangle with flag 512, as `<polyline>`); 44 open polylines in sample 9 carry bit 1 and are exported closed. `POLYLINE_2D` correctly uses 1 | Closed rooms, slabs and hatch outlines are open in JSON and PNG; polygon area is unrecoverable |
| OCS / extrusion (probe) | Never read anywhere in `crates/uncad/src` | Every one of the 95 polylines with bit 1 set carries extrusion (0,0,-1), i.e. mirrored OCS. Ignoring it places sample 9's 93 polylines at x -672..-97 instead of +97..672, and those are the negative-x outliers that inflated the sample 6/7/9 bounds. The DWG decoder leaves `extrusion` at (0,0,0) when the bit is clear (the DXF path stores (0,0,1)) | Mirrored blocks and exports land X-flipped, labelled as world coordinates |
| LWPOLYLINE bulges, widths, elevation | Dropped (`convert.rs:365-374` vs dwg.h `num_bulges/bulges`, `widths`, `const_width`, `elevation`) | Sample 5 has 1,425 polylines with bulges | Curved segments become chords; perimeter and area are wrong; wall widths are lost |
| POLYLINE_2D vertex bulges | `dwg_object_polyline_2d_get_points` copies x,y only | `dwg_api.c` | Same chord error for legacy heavy polylines |
| TEXT / ATTRIB (probe) | `ins_pt`, height, value, rotation only | dwg.h has `alignment_pt`, `horiz_alignment`, `vert_alignment`, `width_factor`, `oblique_angle`, `style`; ATTRIB has `tag` and `flags` (invisible). Probe: non-default alignment on 541/861 texts in sample 6, 284/405 in sample 7, 177/179 in sample 3 | About 1,400 sample texts are anchored at the wrong point; attributes are values with no key |
| MTEXT (probe) | Rotation hardcoded 0 (`convert.rs:565`); `attachment`, `rect_width`, `extents_width/height`, `x_axis_dir` dropped; JSON `text` keeps inline codes | Probe: `attachment != 1` on 5-100 % of MTEXT per file (5 = middle-centre dominates because dimension text uses it); rotated MTEXT (x_axis_dir != (1,0,0)) 74 in sample 1, 97 in sample 6 | Dimension and leader notes are mis-placed; the LLM sees `\A1;{\H0.7x;\S1#2;}` |
| Pre-R2007 strings (probe) | `dwg_dynapi_entity_utf8text` returns raw codepage bytes for `from_version < R2007` (and for every DXF input); `dynapi.rs` applies `CStr::to_string_lossy`, producing U+FFFD | All nine samples are R2004 with `Dwg_Data.header.codepage = 30` (ANSI_1252): sample 3's `\A1;±3 1/2"` (0xB1) and `108°` (0xB0) in the examples already arrive as U+FFFD. This corrects an earlier reading that blamed the DXF path. R2013 Chinese text in `gh109_1.dwg` (UTF-16 path) arrives as valid UTF-8. LibreDWG ships `bit_TV_to_utf8` with CP949/CP936 tables (`bits.c`, `codepages.c`), used only by its DXF writer | Every Korean string in an R2000/R2004 file (still the common exchange versions) becomes U+FFFD runs |
| LAYER state (probe) | Name and colour only (`tables.rs`) | `off`, `frozen`, `locked`, `plotflag`, `linewt`, `ltype` are 1-byte `B`/`RC` fields via dynapi. Every sample has an off/frozen viewport layer and a non-plotting `Defpoints` | Hidden geometry is drawn, exported, counted |
| Entity common | Only layer, ACI index, truecolor | `linewt`, `ltype`, `ltype_scale`, `invisible`, `entmode`, EED all unread. Probe: `linewt` is BYLAYER/BYLWDEFAULT on 97-100 % of entities (Korean practice puts weight in CTB colour tables, not in the DWG) | Walls and dimension lines have the same weight; no model/paper tag per entity |
| MINSERT | Falls to `Entity::Unknown` | `convert.rs:781-784` | Arrayed block references vanish |
| VIEWPORT (probe) | centre, width, height | `VIEWCTR`, `VIEWSIZE`, `VIEWTWIST`, `VIEWDIR`, `status_flag`, `frozen_layers`, `entmode` all readable. The first VIEWPORT of every paper-space block is the layout's own overall viewport (`entmode 0`, `on_off 0`) | Sheet scale (`height / VIEWSIZE`) cannot be stated; paper space renders empty frames |
| LAYOUT / PLOTSETTINGS, DIMSTYLE, STYLE, LTYPE, GROUP, XRECORD | Never visited (`tables.rs:66-112`) | All fixed/stable types with dynapi tables. `LAYOUT.plotsettings` is an embedded struct readable through `dwg_dynapi_subclass_value` (no shim). Handle -> object needs `dwg_resolve_handle` allowlisted (`dwg_ref_get_object` does not resolve) | No paper size, plot scale, dimension formatting or font information |
| SPLINE | Fit/control points only | `degree`, `knots`, `weights`, `closed`, `periodic` dropped | Length cannot be computed; control-point-only splines draw the hull |
| 3DSOLID / REGION / BODY | ACIS edge chords (`acis.rs`) | LibreDWG has no volume/area/mass-property or extents API (grep of `dwg_api.h`, `dwg.h`); a DWG stores only the ACIS stream | **Volume is not obtainable** without a B-rep evaluator |
| ACAD_TABLE | Cached block only | `num_rows/num_cols/cells[].text_value` unread; `Dwg_TABLE_Cell` is bindgen-hostile, needs a shim walker | Schedules come out as loose text |
| HATCH | Pattern name, angle, `scale_spacing`, associativity and polyline-path bulges dropped | `convert.rs:654-697, 850-905` | No material semantics; rounded hatch areas wrong |
| Windows path with Hangul | `parse("한글경로/도면.dwg")` fails with `DWG_ERR_IOERROR` (4096) while the same file parses from an ASCII path | `lib.rs` passes the UTF-8 path to `dwg_read_file`, which calls `fopen` (`dwg.c:274`); the MSVC CRT interprets the bytes in the ANSI code page | First run on a Korean machine fails |

Handles are unpadded upper-case hex. Entity order is per-block owned-chain
order. `block_records` duplicates every model/paper-space entity and INSERT
attribs are duplicated at the top level, so the JSON is roughly twice the size
it needs to be and no entity says which space it belongs to.

## 2. Rendering pipeline

| Area | Today | Measured |
|---|---|---|
| Pixel size | `viewBox size x --scale`, no fit-to-pixels, no cap (`png.rs:101-104`) | Sample 5 (12.7 MB, 19,896 entities, drawn in ~20 units): 69 x 70 px, 1 KB. Sample 7: 7724 x 3047 px. Paper space: ~55 x 35 px (sheet inches). `example_2018.dwg`: one INSERT scaled 3256x gives a 3.4 M-unit viewBox and `memory allocation of 36100186821324 bytes failed`. `--scale 150` on sample 4: 51288 x 38273 px, 66 s, 7.7 GB working set |
| Background | No background element; RGBA with transparent pixels | Sample 6: 90 % of pixels alpha 0, 0.0 % fully opaque. Composited on black (as many viewers and JPEG converters do) nothing but coloured walls is visible |
| Stroke width | `viewBox diagonal / 6000` (`svg.rs:955-957`), i.e. `image diagonal px / 6000` at any scale | 0.73-0.96 px on a 4096-px render; only 0.1-1.6 % of inked pixels are fully opaque. At 1.2 px, 19-21 % are, and the tile reads clearly (bench, viewed) |
| Symbol sizes | POINT r 0.5, arrowhead 2.5, dash arrays `4,2`/`2,2`, RAY length 1e6 -- all drawing units | Invisible on a mm drawing, huge on a metre drawing; RAY/XLINE hit tiny-skia's 2^22-px float limit at scale >= 4 |
| Fonts | No `font-family`; system fonts loaded per call (`png.rs:92-93`) | usvg falls back to Times New Roman. A bundled face loaded naively shapes 0 glyphs: the generic family must be pointed at it (`fontdb::Database::set_serif_family` etc.) or an alias registered with `push_face_info` (bench). CJK glyphs vanish silently on a host without CJK fonts |
| MTEXT / TEXT codes | `strip_mtext_formatting` (`svg/format.rs:84-102`) | `3{\H0.7x;\S1#2;}"` -> `31/2"`; `\S+0.1^-0.2;` -> `+0.1/-0.2`; `%%U`, `%%C`, `%%P`, `%%D` emitted literally (sample 7: 96 strings). Only the `#` stack separator is handled; `/` (DIMFRAC 0, AutoCAD's default) and `^` are not |
| Colour | ACI 7 -> black; yellow/cyan on white at ~1.07:1 contrast (`color.rs:57-63`) | Sample 1 is mostly ACI 2 yellow |
| Paper space | All layouts merged into one document; VIEWPORT draws a dashed frame (`svg.rs:874-902, 710-735`) | Sample 9 paper render: border, title block, 24 empty frames |
| SVG numbers | Full `f64` precision (`svg/format.rs:54`) | 0.6-18 MB SVGs; ~40 % of the bytes are digits below 1e-12 of a stroke |
| Per-call cost | Fonts reloaded (647 faces: 45 ms warm, ~3 s cold) and SVG re-parsed (120-340 ms, dominated by text shaping, not file size) on every `to_png` | An 18 MB SVG with no text parses in 133 ms; a 1.4 MB one with 1,242 texts in 310-340 ms (bench) |
| Determinism | Cluster ties follow `HashMap` order; `unsupported_types` is a `HashSet` | Metadata can differ between runs |

## 3. The crop rule today

`svg.rs:932-957` and `svg/bounds.rs`:

1. Raw bounds = min/max of every world point the renderer "considered" while
   drawing. Contributions differ by type: LINE endpoints; CIRCLE and ARC the
   full circle box regardless of sweep; ELLIPSE the major-radius square;
   TEXT/MTEXT/ATTRIB the insertion point only (glyphs can be clipped);
   RAY/XLINE the base point only (drawn 1e6 units); INSERT/DIMENSION their
   block contents through the composed transform; 3D solids their isometric
   projection.
2. `outlier_trim` (default on, when more than two entity boxes): corner-
   proximity union-find with eps = 0.5 % of the inter-quartile diagonal; the
   highest-scoring cluster (count x diagonal) is the seed; other clusters are
   absorbed while their gap is within 30 % of the seed diagonal; the result is
   used only if the seed holds a majority, otherwise the code silently falls
   back to raw bounds. Nothing reports what was excluded.
3. `padding` = 5.0 drawing units on each side (14 % per side on sample 5,
   0.06 % on sample 7).
4. Header `EXTMIN/EXTMAX`, `LIMMIN/LIMMAX`, LAYOUT extents and VIEWPORT windows
   are never consulted.

Header extents versus the trimmed viewBox (probe; drawing units):

| Sample | `EXTMIN`-`EXTMAX` size | Trimmed viewBox | Verdict |
|---|---|---|---|
| 1 | 555 x 805 | 765 x 867 | Trim keeps outliers down to x = -89 |
| 2 | 2126 x 1733 | 2035 x 1899 | Same content |
| 3 | 1707 x 278 | 1747 x 621 | Trimmed height comes from one ARC's whole-circle box |
| 4 | 333 x 245 | 342 x 255 | Same |
| 5 | 19.6 x 13.7 | 69 x 70 | Trim inflated 3x by tree/turf ARC boxes |
| 6 | 5528 x 2573 | 2886 x 1959 | **Trim discards a kitchen elevation at x 5079-5930: 631 entities, 33 TEXT, 5 MTEXT, 19 dimension-layer entities, 30 INSERTs** |
| 7 | 3665 x 2529 | 7724 x 3047 | Trim keeps a mirrored-OCS outlier at x = -3666 |
| 8 | 3601 x 2493 | 3611 x 2696 | Same |
| 9 | 688 x 291 | 1396 x 301 | Trim keeps mirrored-OCS polylines at x -672..-97 |
| `example_2018.dwg` | 3.5 M x 2.7 M | identical | AutoCAD's own extents include the 3256x INSERT; the header does not rescue this file |

`LIMMIN/LIMMAX` is the 12 x 9 template default on most samples and useless.
`PEXTMIN/PEXTMAX` can be the +/-1e20 "unset" sentinel and must be validated
before use. `$DWGCODEPAGE` is empty for every DWG; the real code page is
`Dwg_Data.header.codepage`, which needs a small accessor shim.

Conclusion for "where is the image cut": today the boundary is decided by a
renderer heuristic rather than by the file, the consumer cannot learn what was
cut, and real drawing content can be dropped wholesale.

## 4. Measurements on real files

CLI, release build, warm cache (the first PNG of a session adds ~3 s of font
cache warm-up):

| File | Entities | Parse ms | JSON | SVG viewBox (units) | PNG px at scale 1 | Note |
|---|---|---|---|---|---|---|
| AutoCADSamples1 | 6,818 | 111 | 3.7 MB | 765 x 867 | 765 x 867 | untrimmed 1.8 M x 1.6 M |
| AutoCADSamples4 | 3,143 | 94 | 1.8 MB | 342 x 255 | 342 x 255 | 0.75-unit text; readable at ~4000 px width |
| AutoCADSamples5 | 19,896 | 1,422 | 47 MB | 69 x 70 | 69 x 70 | POLYLINE_2D vertex lists alone 19.7 MB |
| AutoCADSamples6 | 4,983 | 119 | 3.4 MB | 2886 x 1959 | 2886 x 1959 | 0 % fully opaque pixels |
| AutoCADSamples7 | 5,384 | 124 | 4.6 MB | 7724 x 3047 | 7724 x 3047 | 96 strings with `%%` |
| test-data/example_2018.dwg | 72 | 74 | 0.4 MB | 3.4 M x 2.6 M | abort | 36 TB allocation |

Parsing dominates (0.1-1.4 s; the C decode is >95 % of it, so a "header only"
mode would not be faster without a LibreDWG-level option). SVG generation adds
under 50 ms; PNG adds 150-1000 ms. A 4000-px-wide render costs 0.5-0.8 s and
0.4-1.0 MB. JSON is 500-1,600 bytes per entity: not something an LLM context can
take directly.

## 5. resvg tile-rendering benchmark (bench)

resvg 0.48.1 / usvg 0.48.1 / tiny-skia 0.12.0, Ryzen 5 6600H, release build.

- **Parse once, render each tile with a root transform (A)** is pixel-equivalent
  to rendering the whole canvas and cropping (B): bit-identical at x offset 0,
  under 1 % of pixels differing by float rounding elsewhere, no seams.
- resvg does **no viewport culling**: a tile far outside the drawing still walks
  every node (13 ms on sample 7's 14k elements, 21-57 ms on sample 5's). The sum
  over tiles is 2-5x one full render single-threaded; 8 threads over the shared
  `usvg::Tree` (`Send + Sync`) give 5-6x, so A beats B in wall time at 16384 px
  (0.7 s vs 1.05 s on sample 7) with under 200 MB instead of a 404 MB (sample 7)
  or 1 GB (sample 5) canvas.
- `resvg::render_node` as a per-node culling device is **not usable**: it refuses
  any path whose fill bounding box is empty, i.e. every horizontal or vertical
  `<line>` (156 of 534 in-tile leaves on sample 4, 1596 of 15,406 on sample 5).
  Culling has to happen at the tile level (skip tiles that intersect no entity
  box) or by grouping entities into `<g>` buckets in the writer.
- Re-parsing per tile (C) costs 100-340 ms per tile and is out.
- Encoding a 1024 px tile: RGBA `encode_png` 246 KB / 44 ms; composite over
  white then Gray8 via the `png` crate 111 KB / 13 ms, or 123 KB / 1 ms with
  `Compression::Fast`; Gray4 67 KB; 1-bit destroys sub-pixel hairlines.
- Strokes at 1.2 px cost +8 % on a line-heavy file and up to +140 % on a
  polyline-heavy one (stroker path instead of hairline rasterizer) and are the
  difference between a tile that reads at 1:1 and one that needs zoom.
- A bundled font must be registered under a family name usvg will select;
  `usvg::Options::fontdb` is an `Arc<Database>` and should be built once per
  process.

## 6. VLM input constraints and prior art (fetched 2026-09-21)

| Vendor | Billing | Limits | Consequence |
|---|---|---|---|
| Claude | 28 x 28-px patches, tokens = ceil(w/28) x ceil(h/28) | Standard tier: long edge <= 1568 px **and** <= 1568 tokens (largest untouched square 1092 x 1092 = 1521 tokens); high-resolution tier (Claude 4.7+): 2576 px / 4784 tokens; hard max 8000 x 8000; more than 20 images per request => each <= 2000 px; wants absolute pixel coordinates; images under 200 px hallucinate | Tiles should be multiples of 28 and <= 1092 px; anything larger is resized server-side and returned coordinates no longer match the sender's affine |
| OpenAI tile models (gpt-4o, 4.1, 5.1) | fit 2048, short side to 768, 512-px tiles (85 + 170 per tile on 4o) | | A 1024 tile is downscaled to 768 |
| OpenAI patch models (gpt-5.x) | 32-px patches, ~2,500 patches at "high" (1600 x 1600 native), up to 6,144 | | 1024-1600 px tiles keep native resolution |
| Gemini | 768 x 768 tiles at 258 tokens, max 3072; Gemini 3 bills a fixed `media_resolution` per image | Returns 0-1000 normalized coordinates | |

Benchmarks (AECV-Bench, ArchPlanVQA, MechVQA, Enginuity, DrawingVQA,
BlueprintSymVL, 2025-2026): frontier VLMs read text at ~0.95 but symbols at
0.40-0.55, counting doors/windows is unsolved, semantic QA on floor-plan CAD
sheets scores 33-38 %, dimension reading 79-88 %, and failures are attributed to
annotation density and symbol clutter. Structured representations beat raw
images (ChatP&ID: +18 % accuracy, -85 % tokens); a practitioner report found raw
DXF entity lists failed where semantic rows (walls, openings, rooms with areas)
worked. Raw SVG as LLM input collapses to ~33 % accuracy at ~1k control points
(VGBench, SVGenius); a CAD sheet has 10^4-10^6. SAHI-style tiling uses 20-25 %
overlap and pairs slices with a full-image pass; every high-resolution VLM uses
an overview thumbnail plus local tiles. Existing CAD MCP servers use tiered
retrieval (summary -> by type -> by layer -> full) and write outputs to a
directory, returning paths.

## 7. DWF

- LibreDWG has no DWF reader (`grep dwf_read` over the vendored sources: none;
  only `DWFUNDERLAY` references). The DWF support uncad once had was an AGPL
  vendored viewer and was removed for licence reasons
  (`docs/THIRD_PARTY_NOTICES.md`).
- Autodesk no longer distributes the DWF Toolkit 7.7; an ODA-modified copy
  exists under Autodesk's proprietary royalty-free licence (not OSI), whose
  compatibility with the GPL-3 build has not been assessed.
- No reusable open-source C/Rust/Python DWF reader exists as of 2026; the only
  open parser is a TypeScript/WASM AGPL viewer with partial W2D coverage.
- DWFx (ISO 29500-2 OPC zip + XPS FixedPage XML) is tractable with `zip` +
  `roxmltree` for 2D paper-space paths, but carries no CAD semantics (no
  DIMENSION objects, no units, no layer state).

Verdict: `.dwf` input is not feasible on this stack and should be refused
explicitly; `.dwfx` is a possible later, separate crate.

## 8. Corrections to earlier documentation

- `docs/CAVEATS.md`, "The polyline closed flag": the ground truth now exists
  (section 1). For LWPOLYLINE the closed bit is 512, bit 1 marks a stored
  extrusion, and the DXF reader normalises to the same layout. The observation
  that bit 1 "matches observed rendering" came from files where no closed
  polyline was inspected.
- `docs/CAVEATS.md`, "MTEXT rotation is always 0": `atan2(x_axis_dir.y,
  x_axis_dir.x)` is the standard DXF group-11 direction vector; rotated MTEXT is
  common (74-97 per sample), so the placeholder is visibly wrong, not merely
  imprecise.
- The earlier reading that the lost degree sign was a DXF-path defect was
  wrong: the same byte (0xB0) comes back from the DWG path of the same file; the
  cause is the missing pre-R2007 code-page conversion.

## 9. Facts settled by this investigation

Recorded so the next reader does not re-derive them.

1. LWPOLYLINE closed = `flag & 512`; `flag & 1` = extrusion present (DWG and
   DXF input alike). The DWG decoder leaves `extrusion` at (0,0,0) when the bit
   is clear.
2. `act_measurement` is populated on every R2000+ dimension; exactly `-1.0`
   means "not computed"; LINEAR values need projection on `dim_rotation`.
   Dimension blocks without text have `user_text == " "`.
3. Header `EXTMIN/EXTMAX` is a valid crop on all nine samples and better than
   the cluster trim where they differ; it is useless on `example_2018.dwg`.
   `$DWGCODEPAGE` is empty for DWG; use `Dwg_Data.header.codepage`.
4. Pre-R2007 (and every DXF-input) string loss is a code-page conversion gap,
   not a DXF reader defect; R2007+ CJK is correct.
5. resvg 0.48: no viewport culling; `render_node` drops axis-aligned lines; one
   tree rendered per tile with a root transform equals the full render; 1.2 px
   is the legibility threshold; a bundled font needs a family alias.
6. `parse()` fails on a Hangul path on Windows (`DWG_ERR_IOERROR`).
7. `LAYOUT.plotsettings` is readable with `dwg_dynapi_subclass_value`; handle
   resolution needs `dwg_resolve_handle` in the allowlist (`dwg_ref_get_object`
   only returns an already-resolved pointer).
8. No LibreDWG API computes extents, areas or volumes.
