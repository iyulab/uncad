# Design proposal: the LLM export package

Status: **proposal, implementation in progress** (2026-09-21; section 10
records what has landed). It answers the question
investigated in [`VLM_INVESTIGATION.md`](./VLM_INVESTIGATION.md): how uncad
should change so that a drawing becomes a directory of PNG images plus JSON that
a vision-language model or an LLM can read, with exact numbers, overlapping
tiles and a defined crop boundary. Numbers quoted here trace to that document.

The proposal went through three independent drafts, three scored reviews, a
synthesis, and three adversarial reviews plus a completeness check; the
corrections from that last round are folded in.

## 1. Answers to the five questions

**Package API and structure.** Delivery becomes a directory written by
`uncad export in.dwg --out dir/` (library: `CadDatabase::export`), tiered so an
agent opens the smallest file that answers its question: `manifest.json` ->
`drawing.json` -> one kind-specific shard (`geometry`, `texts`, `dimensions`,
`regions`, `blocks`, `strings`) -> one tile PNG with its JSON sidecar. Every
record carries an id (the entity handle), a world bounding box and its pixel box
in each image that shows it, so the agent moves between "what" (JSON) and
"where" (PNG) by id. `parse()` gains a `header`, and a parse-once `Renderer`
sits between the model and the images. The 0.2.0 API survives with additive
fields; two things break (section 8).

**Image versus structured text.** "Image for where, JSON for what." VLMs read
text well but symbols badly, cannot count reliably and misread 12-20 % of
dimensions from pixels, so every number an agent may ask for -- lengths, areas,
dimension values, counts, attribute values, units -- is computed once in Rust
from DWG fields and written with an explicit unit and a confidence tag. Images
carry spatial context. SVG is a rendering intermediate and never an LLM input.
Volume is `unavailable`: a DWG stores only the ACIS stream.

**Tiling.** Yes. One `usvg::Tree` per zoom level is rendered into 1092 x 1092
tiles (39 x 39 patches of 28 px = 1521 Claude tokens, the largest square the
standard tier accepts untouched) with 224 px overlap (20.5 %), in parallel,
one root transform per tile. An overview plus a pyramid (z1, z2 complete; z3+
only where text is still small) replaces the unbounded canvas; on-demand
windows cover the rest. Overlap duplicates are reconciled by record id, never by
box IoU.

**Crop boundary.** A deterministic, reported rule (section 4): the overview
always covers the true extents of every visible entity (no cluster trim);
clustering only splits the drawing into "frames" that each get their own tile
pyramid; validated header `EXTMIN/EXTMAX` is an automatic candidate; every
image writes its world rectangle and both affines; everything excluded is
listed with a reason. Paper layouts crop to the PLOTSETTINGS sheet, one image
set per layout.

**In (dwg|dwf) -> Out (directory).** Feasible now for DWG and DXF. `.dwf` is
refused with an explanation (no reader exists; the format has no CAD semantics
to extract); `.dwfx` could later yield a paper-only package under the same
schema from a separate crate. DXF input is flagged `input_fidelity:
"dxf-partial"` because LibreDWG's DXF reader is documented as incomplete.

## 2. Output package layout

```
dir/
  README.txt          reading order; entities.json and drawing.svg are tool inputs, not LLM inputs
  manifest.json       source{version,codepage,name}, units, profile, crop, frames[] (each with its own
                      levels[] and legibility), frames_dropped[], overview{png,px,world,both affines},
                      legibility{height_classes}, counts, capabilities, sheets[], svg_origin, generator,
                      guidance, shard_index, warnings[], files[]  (per-tile hashes live in tiles.json).
                      17 KB to 90 KB on the nine drawings docs/EVAL.md measures, not the 12 KB this
                      proposal budgeted: files[] holds one entry per written file with its byte count
                      (89 for example_2000.dwg, 771 for example_2018.dxf) and is most of the weight
  drawing.json        header variables, units, layers[] (state flags, hex + rendered_hex, entity counts),
                      block definitions (xref/dynamic flags), layouts[] (paper size, per-viewport scale and model window)
  overview.png        fitted to the profile budget (claude: <= 1568 px edge and <= 1568 patches), opaque white
  frames/f0/          primary frame: overview.png, tiles/z1..zN/rRR_cCC.png + .json sidecars, tiles.json
  frames/f1/ ...      secondary frames (detached clusters, scale groups), same layout
  geometry.json       every visible non-text entity as a summary record (type, layer, key points, length, bbox, tiles);
                      sharded per layer above the shard size
  texts.json          TEXT/MTEXT/ATTRIB/dimension labels: plain + raw, world bbox, tiles, px boxes
  dimensions.json     measured value, displayed string, unit, definition points, agreement
  regions.json        closed polygons: area, perimeter, centroid, labels; vertex lists only under --full (geometry ref otherwise)
  blocks.json         definitions {name, count, count_by_layer, attrib_tags, dynamic, xref} and instances
                      {id, block, at, rot, scale, mirrored, attribs key/value, array}
  strings.json        normalized string -> [id] inverted index, numeric strings first, id -> shard file
  tiles.json          every frame/level/tile, including empty ones with a reason; sha256 and bytes
  sheets.json + sheets/<layout>/...   0.3.0: paper size, scale, viewport windows; 0.4.0: model composited per viewport
  report.json         excluded entities with reasons, unshaped characters, code-page fallbacks, dimension paths, timings
  entities.json       --full only: the whole model without the 0.2.0 duplication (block_records definition-only),
                      sharded by (layer, 2 MiB)
  drawing.svg         --svg only
```

Every JSON file follows one `--shard-kb` rule (default 96) and sorts records by
id, so `manifest.shard_index {file, kind, first_id, last_id, count, bytes}`
resolves any id to one file. A typical question costs the manifest +
`strings.json` lookup + one shard + one sidecar + one tile (1521 tokens). The
manifest is the largest of the JSON reads and the one this proposal sized
wrongly: 16 728 bytes on `example_2000.dwg` and 89 532 on `example_2018.dxf`,
against the ~3.5k tokens estimated here. Token counts per file are still
estimated; measuring them with the vendor tokenizer is open (`docs/EVAL.md`,
section 4).

## 3. JSON conventions and example records

- Every file starts with `"$schema": "uncad-package/1"` and a `units` block
  `{name, insunits, to_mm | null, source, guess?}`.
- ids: upper-case hex handle; `"<insert>/<child>"` through INSERTs;
  `"<dim>/T"` for a dimension's cached text; `"<handle>#r3c4"` for MINSERT
  cells. Image ids: `ov`, `f0/z2/r03_c05`, `sheet:Layout1`, `w:<8hex>`.
- Points are `[x, y]` arrays rounded to `max(LUPREC, 3)` decimals, derived
  values to two more. `LUPREC` is a display setting, so the decimals also have
  a floor from the package's own scale: enough that one unit in the last place
  is a thousandth of a pixel at the deepest level the package can reach
  (0.3.0; without it every tile rectangle of a drawing a few thousandths of a
  unit across printed the same numbers). Angles are in degrees; pixels are
  integers; boxes are `[x0, y0, x1, y1]` in world units.
- `confidence` in `{exact, stored, numeric, cached, estimated, unavailable}`
  with an optional `why`; `numeric` carries `tol`.
- Every record lists `tiles: [...]` and `px: {"ov": [...], "f0/z1/r02_c00": [...]}`.

LINE. `from`/`to` are the plan projection (every box and every pixel map in the
package is planar), but `length` is the length the *file* holds -- the 3D
distance -- so a line that rises does not publish its plan length as `exact`.
When it does rise, `length_plan` (the key POLYLINE_3D uses for the same
quantity) and `dz` are written beside it, and they are the only thing in an
otherwise planar record that says so; a line in the plane carries neither.

```json
{"id":"2F3A","type":"LINE","layer":"A-WALL","space":"model","from":[120.5,48.0],"to":[240.5,48.0],
 "length":120.0,"unit":"mm","confidence":"exact","bbox":[120.5,48.0,240.5,48.0],"tiles":["f0/z1/r02_c00"]}
```

```json
{"id":"2F3B","type":"LINE","layer":"A-ROOF","from":[0.0,50.0],"to":[300.0,50.0],
 "length":500.0,"length_plan":300.0,"dz":400.0,"unit":"mm","confidence":"exact"}
```

Closed LWPOLYLINE with one 90-degree arc. The bulge on vertex i applies to the
segment i -> i+1 (including the closing segment); bulge = tan(theta/4), positive
= counter-clockwise. theta = 4 atan(b), r = c (1 + b^2) / (4 b), circular
segment area r^2/2 (theta - sin theta), signed by orientation. A polyline closed
by repeating its first vertex is deduplicated and marked `closed_by_repeat`.

```json
{"id":"3B0","type":"LWPOLYLINE","layer":"A-FLOR","space":"model","closed":true,"vfmt":"[x,y,bulge]",
 "vertices":[[0,0,0],[100,0,0.41421356],[100,50,0],[0,50,0]],
 "perimeter":305.536,"area":5356.748,"area_unit":"mm2","orientation":"ccw","simple":true,"confidence":"exact",
 "why":"shoelace 5000 + circular segment r^2/2(theta - sin theta) = 356.748",
 "segments":[{"k":"line","len":100},{"k":"arc","len":55.536,"r":35.355,"center":[75,25],"sweep_deg":90},
             {"k":"line","len":100},{"k":"line","len":50}],
 "centroid":[53.61,25.0],"bbox":[0,0,110.355,50],"region":"3B0","tiles":["f0/z1/r00_c00"]}
```

DIMENSION:

```json
{"id":"3A1","kind":"LINEAR","layer":"A-DIMS","dimstyle":"ARCH-48",
 "measurement":2.5,"measurement_source":"act_measurement","measurement_from_points":2.5,"delta":0.0,"unit":"in",
 "dimlfac":1.0,"display_value":2.5,"display":"2 1/2\"","display_raw":"\\A1;2{\\H0.750000x;\\S1#2;}\"",
 "display_source":"cached_block","agreement":"exact","display_tolerance":0.03125,"user_text":"",
 "style_used":{"DIMLUNIT":4,"DIMDEC":4,"DIMZIN":0,"DIMPOST":"","DIMDSEP":".","DIMRND":0,"overrides_xdata":"not_decoded"},
 "from":[10,20],"to":[12.5,20],"dir_deg":0,"text_at":[11.25,24.4],"text_id":"3A1/T",
 "bbox":[10,20,12.5,24.8],"tiles":["f0/z1/r01_c03"],"px":{"ov":[420,910,470,930],"f0/z1/r01_c03":[88,310,420,362]}}
```

- `measurement_source` in `{act_measurement, from_points, none}`, with
  `confidence` following it: `stored` for the file's own `act_measurement`,
  `exact` for a value recomputed from the definition points, `unavailable` for
  neither. A stored value is refused -- and the definition points used instead
  -- when it is not a measurement this dimension could have: `-1.0` exactly
  (R13/R14's "not computed"), `0.0` on any kind but ORDINATE (how an R13/R14
  DXF, and any DXF without a group 42, leaves the field), a negative value on
  any kind but ORDINATE, or one disagreeing with the definition points by more
  than 1 % (the corpus's worst honest disagreement is 2.3e-7 relative).
  ORDINATE keeps its sign, and may legitimately measure 0.
- `capabilities.dimension_values` in `{exact, computed, text_only, none}`:
  `exact` when at least one dimension carries the file's own measurement,
  `computed` when every value was recomputed from the definition points (an
  R13/R14 drawing), `text_only` when there are dimensions but no values.
- Angular kinds (ANG3PT, ANG2LN): `act_measurement` is radians and is exported
  in degrees with `unit: "deg"`; DIMLFAC does not apply; the sector is the one
  containing `def_pt`; display follows DIMAUNIT/DIMADEC.
- `display_tolerance`: `0.5 x 10^-DIMDEC` for DIMLUNIT 1/2/3/6; for DIMLUNIT
  4/5 (architectural, fractional) DIMDEC is a fraction-precision index, so the
  tolerance is `0.5 / 2^DIMDEC` inches. All nine samples are DIMLUNIT 4.
- `display_source` in `{user_text, cached_block, formatted_basic, formatted,
  suppressed}`; `agreement` in `{exact, whitespace, mismatch, no_cache,
  override}`. A whitespace-only `user_text` is `suppressed`, without a warning.

TEXT:

```json
{"id":"4B2","kind":"TEXT","text":"BOOK RETURN","raw":"%%UBOOK RETURN","decorations":["underline"],"layer":"A-ANNO",
 "height":4.5,"rotation_deg":0,"anchor":[1500.25,820.5],"align":"MC","style":"ROMANS","font_ok":true,"visible":true,
 "bbox":[1484.9,818.3,1515.6,824.7],"bbox_confidence":"stored","why":"usvg glyph box, bundled font",
 "tiles":["f0/z1/r00_c02"],"px":{"ov":[610,402,624,405],"f0/z1/r00_c02":[212,640,571,693]}}
```

Other records: **region** `{id, src, layer, area, area_unit, area_si | null,
perimeter, centroid, bbox, vertex_count, simple, holes, hatch, labels,
confidence, tiles, px}` where labels are the texts whose anchor lies inside the
polygon and in no smaller region, and `simple` is `null` (with `confidence:
"estimated"`) for an outline of more than 2 000 vertices, whose pairwise
self-intersection test is skipped rather than run at O(n^2); **tile sidecar** `{id, png, z, row, col, px,
world, ppu, world_to_px, px_to_world, overlap_px, neighbors, parent, children,
empty, layers_present, layers_truncated, layers_total (only when trimmed),
fits_profile, expected_encoded_px, resize_factor, norm_to_px, records: {texts:
[[id, px, t]], dims: [[id, px, s, v]], blocks, regions}}`, capped at 32 KB with
text truncated to 24 characters: the record rows are cut first
(`records_truncated`), then the layer list (`layers_truncated`, with
`layers_total` saying how many names there were), so the file always says which
of the two a reader is missing. Sidecars are
authoritative for "what is on this image"; shards hold the full record.
`strings.json` normalisation is NFKC + case fold + whitespace collapse + a
canonical fraction form + unit-suffix handling (m2, mm, ㎡, ㎜).

## 4. Crop boundary rule

Model space, one crop per frame; every step is deterministic and reported.

1. **Visible set V**: model-space entities minus layers that are off, frozen or
   non-plotting, DEFPOINTS, `invisible` entities, invisible ATTRIBs, RAY/XLINE,
   ACAD_PROXY_ENTITY/Unknown, empty text. `--include-hidden` draws hidden
   layers at 50 % and never affects the crop.
2. **True WCS extents** per entity after OCS -> WCS: LINE endpoints; CIRCLE
   c +/- r; ARC endpoints plus the axis crossings inside the sweep; ELLIPSE
   parametric extremes clipped to the range; LWPOLYLINE/POLYLINE_2D with
   bulge-arc extremes and widths; SPLINE by evaluation; TEXT/MTEXT/ATTRIB from a
   **metrics pre-pass** (one parse of a world-unit SVG with the bundled font,
   an id -> node index built once) with the 0.62-em estimate only as a seed
   before that pass; INSERT/DIMENSION/MINSERT nested through the composed
   transform; HATCH loops; 3D entities as plan (x, y). Computed once, reused for
   record boxes.
3. **Scale-outlier guard**: an entity whose diagonal exceeds 20x the seed
   cluster's and whose cluster holds at most `max(3, 1 %)` entities is excluded
   with reason `scale_outlier`; if the majority rule fails and the raw diagonal
   exceeds 100x the seed's, the seed is forced (`cluster_forced`). The 3256x
   INSERT of `example_2018.dwg` becomes a listed exclusion, not a blank page.
4. **Candidates and automatic choice**: the computed content extents E versus
   the header `EXTMIN/EXTMAX` accepted only if finite, `|v| < 1e15`, area > 0,
   containing >= 90 % of visible boxes and at most 4x E's area. `--crop auto`
   picks whichever covers more visible entities and records it in `crop.mode`.
   Priority: `--crop x0,y0,x1,y1` > `--crop header` > `auto` > `raw`. An empty
   drawing yields `[0, 0, 10, 10]`, mode `empty`.
5. **The overview is never cluster-trimmed.** Clustering (today's union-find,
   made deterministic by sorting clusters by score, min_x, min_y, count; absorb
   threshold `--trim-absorb`, default 0.30 until a corpus sweep says otherwise)
   only splits V into **frames**: the primary frame f0 and one secondary frame
   per excluded cluster holding at least one text or N entities. Text-height
   clusters (scale groups) are framed the same way, which covers model spaces
   that hold several drawings or details side by side. `crop.excluded_text_count
   > 0` raises `CROP_EXCLUDES_TEXT`.
6. **Padding**, solved once: L = longer side, s = fit_px / (1.04 L),
   pad = max(0.02 L, 24 / s) on all sides; `padding_units` written.
7. **Lattice snap**: W = ceil(w_pad s / 28) x 28, H likewise; the world rect
   grows on the right and bottom so W / s and H / s equal the world size
   exactly (`--lattice 32` for OpenAI-first use).
8. **Affine per image**: `px = (x - x0) s`, `py = (y1 - y) s`;
   `world_to_px = [s, 0, -x0 s, 0, -s, y1 s]`, `px_to_world = [1/s, 0, x0, 0,
   -1/s, y1]`, `units_per_px = 1/s`; absolute pixels, never normalised; round
   trip exact to 1e-9 by unit test. Any image above the profile cap carries
   `fits_profile: false`, `expected_encoded_px` and `resize_factor` so a
   coordinate returned from a server-resized image can be mapped back; every
   sidecar carries `norm_to_px` for models that return 0-1000 coordinates.
9. **Reporting**: `manifest.crop {mode, rect, content_rect, padding_units,
   header_extents, excluded {count, by_type, rect, first 100 handles}}`;
   `report.json` lists every excluded id with a reason in `{excluded_by_trim,
   hidden_layer, invisible, scale_outlier, unsupported_type}`; excluded records
   carry `tiles: []`.
10. **Paper layouts**: crop = the full sheet from PLOTSETTINGS
    `paper_width/height` via `plot_paper_unit`, rotated by
    `plot_rotation_mode`, margins applied, no padding, no trim; LAYOUT
    `LIMMIN/LIMMAX` then layout extents as fallbacks; one image set per layout,
    never merged. Viewport scale = `height / VIEWSIZE`, model window =
    `VIEWCTR +/- (VIEWSIZE w/h / 2, VIEWSIZE / 2)` when `VIEWDIR = (0,0,1)`.
    The overall viewport (`entmode 0`) is skipped; content of a viewport whose
    own layer is off is still drawn, only its frame is hidden; the sign of
    `VIEWTWIST` is fixed by a fixture before `twist_convention` is recorded.

## 5. Tiling rule

| Profile | Overview cap | Tile T | Overlap O | Step | Cost per tile |
|---|---|---|---|---|---|
| claude (default) | 1568 px edge and 1568 patches | 1092 | 224 (20.5 %) | 868 | 1521 tokens |
| claude-hires | 2576 px / 4784 patches | 1932 | 392 (20.3 %) | 1540 | 4761 tokens |
| openai-patch (gpt-5.x) | 2048 | 1600 | 320 (20 %) | 1280 | 2500 patches |
| openai-tile (gpt-4o/4.1/5.1) | 768 short side | 768 | 160 | 608 | 4 x 512 tiles = 765 tokens |
| gemini (2.x cost model) | 3072 | 1536 | 308 | 1228 | 1032 tokens (Gemini 3: fixed `media_resolution`) |

- **Overview**: for aspect a = w/h, pw = min(56, floor(sqrt(1568 a))),
  ph = min(56, floor(1568 / pw), ceil(pw / a)); image (pw x 28) x (ph x 28)
  after the lattice snap; ppu_0 = min(pw 28 / w, ph 28 / h). Sample 6
  (2886 x 1959 units): 48 x 32 patches = 1344 x 896 px, 1536 tokens, ppu 0.457.
  A short edge under 200 px raises `TinyOverview`.
- **Levels**: ppu_z = ppu_0 x 2^z; no canvas above 8000 px is ever
  materialised. Grid: cols = 1 if W_z <= T else ceil((W_z - T) / step) + 1,
  rows likewise; the last row and column are shifted inward (SAHI) so every
  tile is exactly T x T; row 0 is north; ids `f0/z{z}/r{rr}_c{cc}`. Sample 6:
  z1 3 x 2, z2 6 x 4, z3 12 x 8 candidates.
- **Depth**: from the **dominant text-height class** (count-weighted median of
  visible text heights, not the 5th percentile, which the small title-block
  text would drag to a depth that blows the tile budget): z_max = ceil(log2
  (target_px / (h ppu_0))), default target 14 px (floor-plan text measured
  legible at 9-12 px), `--min-text-px 20` for detail work, `--max-levels 5`.
  `manifest.legibility.height_classes = [{height, count, px_at_zmax, legible}]`
  tells the agent which classes still need a window. z1 and z2 are complete;
  from `--sparse-from 3` a tile is written only if it holds text of a class
  that first crosses the target at that level. Tiles that intersect no visible
  box are listed `empty: true` and not written. `--max-tiles 400` counts written
  tiles; when exceeded z_max drops and `legibility.reached = false` points to
  windows.
- **Implementation**: one SVG string and one `usvg::Tree` per level (numbers
  rounded to 0.01 px at the level's ppu, which cuts the SVG ~40 %); `fontdb`
  built once per process (`OnceLock<Arc<Database>>`); tiles rendered in
  `std::thread::scope` with `resvg::render(&tree,
  Transform::from_scale(s, s).post_translate(-x0, -y0), &mut pixmap)`.
  `resvg::render_node` is not used (it drops axis-aligned lines). Tile culling
  is at tile level through the entity-box index. Low levels (overview, z1) may
  render one canvas and crop. PNG: 8-bit RGB (Gray8 under `--mono`) through the
  `png` crate, `Compression::Fast`.
- **On demand**: `uncad render --around <id> --margin 2.0 --fit 1092`,
  `--window x0,y0,x1,y1 --ppu 8`, `--layers`/`--exclude-layers` for
  de-cluttered views, `--ruler` for a world-unit scale overlay, `--batch
  windows.json` for many windows per parse; output `windows/w_<8hex>.png` +
  sidecar; 8000 px cap per edge.

## 6. Numeric exactness strategy

| Class | What | Tag |
|---|---|---|
| Exact (closed form on dynapi fields) | LINE length; ARC r x sweep; CIRCLE 2 pi r, pi r^2; polyline length and area with bulges (signed); full ELLIPSE pi a b; counts by type/layer/block/attribute; MINSERT rows x cols; `uncad measure` (distance between ids, area from an id or point list, per-layer/block/hatch-pattern sums) | `exact` |
| Stored (copied) | `act_measurement` (R2000+, not `-1.0`); MTEXT extents; usvg glyph boxes | `stored` |
| Numeric (quadrature, `tol` given) | ELLIPSE arc length (adaptive Gauss-Kronrod); SPLINE length by de Boor on knots/weights (1.0; chords in 0.3.0) | `numeric` |
| Cached | Dimension display string from the `*D` block, decoded | `cached` |
| Estimated | Pre-render text boxes (0.62 em); SHX-styled widths from the bundled face; fit-point-only splines | `estimated` + `why` |
| Unavailable | Volume/mass of 3DSOLID/REGION/BODY; area of a self-intersecting polygon (`simple: false`); non-planar REGION wires | `unavailable` + `why` |

- **Units**: INSUNITS -> the DXF reference table (0 unitless, 1 in 25.4,
  2 ft 304.8, 4 mm 1, 5 cm 10, 6 m 1000, ... 21-24 US survey), hardcoded
  because LibreDWG has no such table. INSUNITS 0 gives `unit: "du"`,
  `to_mm: null` and `units.guess {name, reason}` from MEASUREMENT / DIMLUNIT /
  extent span; a guess is never written into `unit`. `--units mm` is the user
  override, recorded as `unit_source: "user"`. `area_si`/`length_si` are
  `null` when `to_mm` is null. `BLOCK_HEADER.insert_units` is exported, never
  re-applied.
- **Dimensions**: `measurement` = `act_measurement` in drawing units, or the
  definition-point recomputation (LINEAR projected on `dim_rotation`; ALIGNED
  `|x2 - x1|`; RADIUS/DIAMETER `|first_arc_pt - def_pt|`; ORDINATE the signed
  x or y offset per `flag2`; ARC r x |end - start|); `measurement_from_points`
  and `delta` always exported. Effective `dimlfac`: XDATA DSTYLE override (1.0;
  until then `overrides_xdata: "not_decoded"`) > DIMSTYLE (read in 0.3.0 via
  `dwg_resolve_handle`) > header. `display_value = measurement x dimlfac`.
  Display text: suppressed > `user_text` with `<>` substituted > cached `*D`
  text > `formatted_basic` (LUPREC/DIMDEC decimal, 0.3.0) > `formatted` (full
  DIMLUNIT 1-6, DIMRND, DIMZIN, DIMDSEP, DIMPOST, DIMALT, DIMAUNIT/DIMADEC, 1.0).
  Caveat: every probed file has DIMLFAC = 1.0, so "act_measurement is
  pre-DIMLFAC" (the DXF reference reading) is untested; a DIMLFAC = 12 fixture
  is required before that claim is tagged `exact`.
- **OCS**: a per-entity table of which points to transform (LINE, POINT, MTEXT,
  3DFACE: none; TEXT/ATTRIB: `ins_pt` and `alignment_pt`; INSERT: `ins_pt`;
  ARC/CIRCLE: centre, plus start/end swap when the normal's z < 0; LWPOLYLINE:
  all vertices at `elevation`; HATCH paths; SOLID corners; DIMENSION: only
  groups 11, 12 and 16 are OCS, groups 10/13/14/15 are WCS). Presence is decided
  by the entity's flag bit, since the DWG decoder leaves (0,0,0) otherwise.
- **Text decoding** (one tokenizer for JSON and SVG): `\P`, `\~`, `\{ \} \\`,
  groups, stacked fractions with all three separators (`\S a/b;`, `\S a#b;`,
  `\S a^b;` per DIMFRAC) with a space inserted after a leading digit
  (`3{\H0.7x;\S1#2;}"` -> `3 1/2"`), `\U+XXXX`, `\M+nXXXX` MIF codes, other
  format codes dropped, `%%c` -> U+2205 (U+2300 fallback), `%%d` -> degree,
  `%%p` -> plus-minus, `%%u`/`%%o` -> decorations, `%%nnn` through the file
  code page. Raw and plain are both kept.
- **Pre-R2007 strings**: a shim `uncad_tv_to_utf8(Dwg_Data*, const char*)`
  applies LibreDWG's own `IS_FROM_TU_DWG` rule (version < R2007, or any DXF
  input) and `header.codepage`, calls `bit_TV_to_utf8` (vendored CP949/CP936
  tables) and returns a fresh buffer. U+FFFD counts go to `report.json`.
- **Determinism**: `BTreeMap` everywhere, sorted clusters, bundled font by
  default; identical input + options + version -> byte-identical JSON; PNG
  goldens compared with a 2-level per-channel tolerance.

## 7. Rendering changes

| Area | 0.3.0 | 0.4.0 / 1.0 |
|---|---|---|
| Background / PNG | opaque white rect; RGB8 or Gray8 via `png`; `--bg white\|black` | -- |
| Palette | ACI -> RGB with luminance > 0.45 darkened (yellow -> #8f8f00, cyan -> #007a7a, ACI 7 -> black); `hex` + `rendered_hex` in the layer legend; `--mono` | `--weights color-map:<json>` (ACI -> px, the CTB convention) |
| Strokes | pixel-constant through the existing `@@SW@@` placeholder: default 1.25 px, lineweight classes 1.0-3.5 px, floor 1.0; POINT 5 px cross; arrowheads DIMASZ x DIMSCALE but >= 6 px; dashes 6,4 px | LTYPE dashes x LTSCALE x `ltype_scale`, solid below a 6 px period |
| Fonts | `bundled-fonts` feature (default on): Noto Sans Latin + symbols (degree, plus-minus, U+2205, U+2300, ㎡, ㎜) and a Noto Sans KR KS X 1001 subset (target ~1 MB; 4.4 MB unsubsetted), registered as family `uncad-sans` via `push_face_info`; `font-family` on every `<text>`; unshaped characters counted; `--fonts bundled+system` | SHX look-alike face; `--shx-dir` |
| Text | `text-anchor` from `horiz/vert_alignment` with `alignment_pt` as an `Option` set only when they are non-zero; `id="<handle>"` on `<text>` so `Node::abs_bounding_box` gives exact boxes | `width_factor`, `oblique`, generation flags, MTEXT attachment / wrap / rotation from `x_axis_dir` |
| Visibility | hidden layers, DEFPOINTS, invisible entities not drawn; `--include-hidden` at 50 % | per-viewport frozen layers |
| Geometry | OCS applied; RAY/XLINE clipped to the crop; all 3D in plan (`--view iso` opt-in); bulges as arc paths; ELLIPSE arcs from the stored parameter range, with the arc's own extent | NURBS, HATCH spline edges |
| Sizing | `PngSize::{FitLongEdge, FitTokens, PxPerUnit, Scale}`, default `FitLongEdge(1568)`, hard cap 8000 px -> `TooLarge` | -- |
| Sheets | LAYOUT/PLOTSETTINGS read; `layouts[]` with paper size and per-viewport scale; frames-only sheet overview | model composited per VIEWPORT inside a `clipPath`; `sheets.json` complete |
| Input | bytes read in Rust, decoded through a `dwg_decode`-from-memory shim (fixes Hangul paths on Windows, enables `parse_bytes`) | -- |

## 8. Rust API

```rust
pub struct CadDatabase { pub entities: Vec<Entity>, pub tables: Tables, pub header: Header }  // + CadDatabase::new(..)
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError>;      // + ParseError::UnsupportedFormat
pub fn parse_bytes(bytes: &[u8], format: Format) -> Result<CadDatabase, ParseError>;
pub fn parse_with(path, opts: &ParseOptions) -> Result<CadDatabase, ParseError>;   // { decode_codepage, codepage_override }
impl CadDatabase {
    pub fn to_json(&self, o: ToJsonOptions) -> Result<String, JsonError>;     // unchanged; output gains header + fields
    pub fn to_svg(&self, o: ToSvgOptions) -> ToSvgResult;                     // unchanged; + crop, background, hidden, decimals
    pub fn to_png(&self, o: ToPngOptions) -> Result<ToPngResult, PngError>;   // ToPngOptions { svg, size: PngSize, max_edge, fonts }
    pub fn renderer(&self, o: RenderOptions) -> Result<Renderer, RenderError>;            // parse-once handle, Send + Sync
    pub fn export(&self, out: impl AsRef<Path>, o: &ExportOptions) -> Result<ExportReport, ExportError>;
    pub fn measure(&self, q: MeasureQuery) -> Measure;
}
// header.rs   Header { dwg_version, from_version, codepage, insunits: InsUnits, units, measurement, lunits, luprec, aunits, auprec,
//             extmin/extmax, limmin/limmax, pextmin/pextmax, plimmin/plimmax, dimscale, dimlfac, dimdec, dimlunit, dimpost, dimrnd,
//             dimzin, dimfrac, ltscale, textsize, clayer }
// geom.rs     Box2D; Affine { sx, sy, tx, ty } + to_px/to_world/inverse; Confidence; Measure { value, unit, confidence, why };
//             ocs_to_wcs(p, normal); bulge_arc(p1, p2, b); nurbs_eval / nurbs_length (1.0)
// measure.rs  bbox(&Entity, &Tables, &dyn FontMetrics) -> Option<Extents>; length / area(&Entity) -> Option<Measure>;
//             polyline_segments(&[LwVertex], closed) -> Vec<Segment>; dimension_text(&DimensionEntity, &CadDatabase) -> DimensionText
// text.rs     decode(raw) -> DecodedText { plain, lines, decorations, fractions };   dimfmt.rs (1.0)
// crop.rs     CropSpec { Auto, Raw, Header, Rect(Box2D) }; compute_crop(&CadDatabase, Space, CropSpec, &TrimOptions) -> Crop { frames, excluded, .. }
// render.rs   Renderer { crop(), render_fit(max_edge, max_tokens), render_window(Box2D, max_edge), tile_plan(&TileOptions),
//             render_tile(&TileSpec), render_tiles_parallel(plan, jobs, sink), text_boxes() };  Image { png, width, height, world, affine }
// export.rs   ExportOptions { target, tile, overlap, min_text_px: 14.0, max_levels: 5, max_tiles: 400, sparse_from: 3, crop, padding,
//             spaces, layers, render, full, svg, shard_bytes: 98_304, jobs, overwrite };  ExportReport { manifest, files, warnings, timing }
// Model and table additions, all #[serde(default)]:
//   EntityCommon += space (from entmode), visible, invisible, lineweight_mm, linetype, ltype_scale, extrusion
//   LwPolyline   += bulges, widths, const_width, elevation   (closed = flag & 512)
//   Text/Attrib  += text_plain, decorations, horiz/vert alignment, alignment_pt: Option, width_factor, oblique_angle, style, tag
//   MText        += text_plain, lines, attachment, rect_width, extents, x_axis_dir (rotation becomes real)
//   Dimension    += subtype, measurement: Option<f64>, user_text, display_text(_raw), display_source, def_pt, text_midpt, dimstyle, points
//   Viewport     += view_center, view_size, twist, status_flag, entmode, id, frozen_layers
//   Spline       += degree, knots, weights, closed, periodic, rational;   Hatch += pattern_name, angle, scale_spacing, associative (+ bulges)
//   LayerRecord  += on, frozen, locked, plot, lineweight, linetype;   Tables += linetypes, text_styles, dimstyles, layouts
//   Entity::MInsert (0.4.0);   block_records become definition-only (no model/paper-space duplication)
```

| 0.2.0 | New |
|---|---|
| `ToPngOptions { scale }` | `ToPngOptions { size: PngSize::Scale(s), ..Default::default() }` (**break**) |
| `CadDatabase { entities, tables }` literal | add `header`, or `CadDatabase::new` (**break**) |
| `to_png` default: units x scale, transparent | `PngSize::FitLongEdge(1568)`, opaque white RGB |
| `ToSvgOptions { padding, stroke_width, space, outlier_trim }` | kept; `..Default::default()` covers the new fields |
| `svg_to_png(svg, scale)` | kept with the 8000 px cap; deprecated in 0.4.0, removed in 1.0 |
| `block_records` duplicating model/paper-space entities in JSON | definition-only (the one intentional shape change) |
| 1.0 | option/result structs `#[non_exhaustive]` with builders; one `uncad::Error` |

## 9. CLI

```
uncad <in> -o out.{json|svg|png} [--space model|paper|all] [--no-trim] [--scale f] [--pretty]   legacy form kept; png fits 1568 px
uncad export <in> --out <dir> [--target claude|claude-hires|openai-patch|openai-tile|gemini|custom] [--tile 1092] [--overlap 224]
     [--min-text-px 14] [--max-levels 5] [--max-tiles 400] [--sparse-from 3|--dense] [--crop auto|raw|header|x0,y0,x1,y1]
     [--pad 0.02] [--pad-min-px 24] [--trim-absorb 0.30] [--lattice 28|32] [--space model|paper|all] [--layout <name>]
     [--layers <glob>] [--exclude-layers <glob>] [--stroke 1.25] [--bg white|black] [--mono] [--include-hidden] [--view plan|iso]
     [--fonts bundled|bundled+system] [--weights color-map:<json>] [--units mm] [--full] [--svg] [--shard-kb 96] [--jobs N]
     [--force] [--dry-run] [--json-report] [-q]
uncad info <in> [--json]            header, units, header vs computed extents, counts, layers, layouts
uncad render <in> -o out.png (--window x0,y0,x1,y1 | --around <id> [--margin 2.0]) [--fit 1092|--ppu f] [--layers ..] [--ruler]
     [--batch windows.json]
uncad dims <in> [--json]            per dimension: measurement / from_points / display / source / agreement
uncad measure <in> --between A B | --area A,B,C | --sum-length --layer <glob>
```

Exit codes: 0 (warnings in `manifest.warnings`), 1 (error; `export` writes to
`<dir>.tmp-<pid>` and renames on success), 2 (unsupported input, with distinct
messages for `.dwf` and `.dwfx`). `--dry-run` writes only the manifest, crop and
tile plan with token estimates. `uncad info` is not a fast path: the C decode
dominates parse time.

## 10. Roadmap

Effort in person-days is the review panel's estimate.

Implementation status (2026-09-23, see `CHANGELOG.md` "Unreleased"): **P-1 done**
except the AutoCAD-written Korean DWG fixture (LibreDWG's own writer mangles
CP949 text, so `tests/fixtures/` ships DXF fixtures only); **P0 done**
(`CadDatabase::header`); **P1 mostly done** (`PngSize` fit-to-pixels with the
8000 px cap, white RGB background, pixel strokes, contrast-normalized palette,
fonts loaded once, `ViewBox` + `px_per_unit` in every result, RAY/XLINE clipped
to each document's viewBox instead of a fixed 1e6-unit segment, and the bundled
`Uncad Sans` with `font-family` on every `<text>` -- shipped as a plain
`include_bytes!`, not behind the `bundled-fonts` cargo feature the table in
section 7 proposes) -- POINT now draws the table's 5 px cross, sized through the
same `@@SW@@` placeholder as the strokes, so it is the same size whatever the
drawing's scale; still open in P1: the other pixel-sized symbols (arrowheads a
fixed 2.5 units, dashes `4,2` user units);
**P2 done** (`uncad::text`
decoder with all three stack separators and the `%%` codes, `text_plain` on
every text type, TEXT/ATTRIB justification fields and ATTRIB `tag`, MTEXT
attachment/width/extents/`x_axis_dir` with the rotation derived, renderer
anchoring) except MTEXT word-wrapping at `rect_width`; **P3 done** in its
0.3.0 form (`DimensionGeometry` per subtype, stored and recomputed
measurements, `user_text`/cached-label/formatted display text with its
source, DIMSTYLE table and effective DIMLFAC, basic formatter; the full
DIMSTYLE formatter with DIMPOST/DIMRND/DIMDSEP and XDATA overrides stays in
1.0); **P4** is in (bit 512, bulges and widths, the VERTEX_2D walk, `geom.rs`
with OCS -> WCS, segments, length, signed area, arc-aware bounds and a
self-intersection check; the OCS applied to CIRCLE, ARC, LWPOLYLINE,
POLYLINE_2D, TEXT, ATTRIB, INSERT and SOLID -- HATCH boundaries and the
DIMENSION text point are still stored-frame); **P5** is in (`visibility.rs`
with `hidden_reason` and the lineweight table, layer on/frozen/locked/plot/
lineweight/linetype, entity invisible/lineweight/linetype/ltype_scale, the
renderer skipping hidden entities with `include_hidden` fading them, hidden
counts in the results and the CLI); **P6** is in its 0.3.0 form (`crop.rs`:
scale/far outlier guard (largest-entities and median-centre rules -- the
design's cluster-based wording did not survive contact with real files,
whose corner-connected clusters are too fragmented for a majority rule),
header candidate,
`CropMode::{Auto, Raw, Header, Fixed}`, automatic padding, lattice snap with
exact pixel/unit proportion, `CropReport` in both results, CLI `--crop`,
`--padding`, `--lattice`; the frame split landed with P7, and the text metrics
pre-pass exists but does not feed the crop -- `measure_texts` runs on the
already-rendered drawing, so the crop is chosen from the 0.6-em estimate and
only the frames and the tile culling see the measured boxes);
**P7** is in its 0.3.0 form (`export.rs`: the overview fitted
to the profile's edge and patch budget, the tile pyramid with 224 px overlap
and inward-shifted edge tiles, sidecars with both affines and per-tile record
lists, texts/dimensions/geometry/regions/blocks/strings/report/drawing JSON
with sharding and a manifest, CLI `uncad export`; frames for detached
groups, NFKC string keys, per-tile culling and parallel tiles landed after;
the bundled `Uncad Sans`, usvg-measured text boxes and the paper layouts
(`sheets.json`, composited sheet images) landed too -- no P7 feature of this
section's 0.3.0 scope is open, but the package diverges from section 2 in two
places: `manifest.json` is 17 KB to 90 KB rather than the 12 KB budgeted
there, because it carries a `files[]` array the design never listed (one entry
per written file, 89 of them for `example_2000.dwg` and 771 for
`example_2018.dxf`), and `source` is `{codepage, name, version}` -- there is no
`producer`, and no `input_fidelity: "dxf-partial"` for DXF input as section 1
promises; `header.format` and the source file's extension are what say the
input was a DXF); **P8** (README, ARCHITECTURE, CAVEATS, `--help`) and **P9**
(`tests/corpus_sweep.rs` over the 208 corpus files, `tests/acceptance.rs`
with five package questions, determinism test, `docs/EVAL.md`) are in --
goldens (byte-exact reference packages) are not: with the bundled font the
tile PNGs should be identical across machines running the same resvg version,
but no reference package is checked in, so the determinism test compares two
runs on one machine.

Four items the tables below place in 0.4.0 landed in 0.3.0 instead: the frame
split (`frames/fN/`), viewport compositing with `sheets.json`, the bundled
font, and per-viewport frozen layers (honoured by the sheet compositor only,
`docs/CAVEATS.md`). The section 7 "Rendering changes" table and the section 9
CLI sketch are the proposal as written, not a description of the binary.
`uncad export` takes `-o`/`--output`, not `--out`; `--target` is spelled
`--profile` and `--min-text-px` is `--text-px`; the tile and padding geometry
comes from the profile and the crop rule, so `--tile`, `--overlap`,
`--sparse-from`, `--dense`, `--pad`, `--pad-min-px` and `--trim-absorb` do
not exist; and neither do `--layers`, `--exclude-layers`, `--layout`,
`--units`, `--weights`, `--view`, `--mono`, `--jobs`, `--force`,
`--dry-run`, `--json-report` or `-q`. Only `--crop`, `--no-trim`,
`--include-hidden`, `--padding` and `--fonts` are shared with the plain
command; the other render flags (`--space`, `--stroke`, `--fit`, `--lattice`,
...) are refused by name on an `export` line -- the package's patch size is
the profile's, not a flag. The `info`, `render`, `dims` and `measure`
subcommands do not exist. `uncad --help` is the current list.

The same goes for what section 9 says about exit and failure handling, and for
section 11's DWF paragraph. The CLI exits 0 or 1, never 2; `export` writes into
the target directory itself (clearing what the previous `manifest.json` listed
and writing the new manifest last) rather than into `<dir>.tmp-<pid>` and
renaming; and there is no `.dwf`/`.dwfx` branch at all -- `Format::from_path`
reads a `.dxf` extension as DXF and everything else as DWG, so a `.dwf` is
handed to the DWG decoder and comes back as `ParseError::Critical`, not as an
`UnsupportedFormat` with an explanation. `ParseError` has two variants,
`Critical(i32)` and `Io`.

| Release | Phase | Files |
|---|---|---|
| **0.3.0 "Readable"** (~42 pd, ~50 with contingency) | P-1 fixtures and spikes (3): a Korean R2000 CP949 DWG (redistributable, e.g. written with LibreDWG's own tools), a mirrored-OCS fixture in the style of sample 9, a DIMLFAC = 12 fixture, a twisted-viewport layout, a Hangul-path test, the `uncad_tv_to_utf8` and decode-from-memory shims compiling against the vendored `bits.h`/`decode.h` | `libredwg-sys/shim`, `tests/` |
| | P0 header, units, version, code page (2) | `dynapi.rs`, `header.rs`, `lib.rs`, shim |
| | P1 render hygiene (4): `PngSize`, fit, 8000 cap, white background, px strokes, RAY clip, contrast, bundled fonts + `OnceLock`, RGB/Gray8 | `png.rs`, `svg.rs`, `color.rs`, `fonts/` |
| | P2 text (4): decoder (three stack separators, MIF, `%%`), `text_plain`, alignment fields + `text-anchor`, CP949 wiring | `text.rs`, `convert.rs`, `svg.rs` |
| | P3 dimensions (4.5): all subtypes, `*D` harvest, suppressed/override paths, from-points, angular kinds, DIMSTYLE scalars (`dwg_resolve_handle` allowlisted), basic formatter, `-1.0` sentinel | `convert.rs`, `model.rs`, `measure.rs`, `build.rs` |
| | P4 polylines, areas, OCS (4): bit 512, bulges, VERTEX_2D walk, segments/length/area, self-intersection check, the OCS table | `convert.rs`, `geom.rs` |
| | P5 visibility (1.5) | `tables.rs`, `convert.rs`, `svg.rs` |
| | P6 crop (5): true extents with the metrics pre-pass, outlier guard, header candidate, frame split, padding, lattice snap, reporting | `crop.rs`, `geom.rs`, `svg/bounds.rs` |
| | P7 renderer and export (8): `Renderer`, parallel tiles, pyramid, sidecars, all shards, manifest, sharding, LAYOUT read + `layouts[]`, tmp-rename, schema, README | `render.rs`, `export.rs`, `schema/` |
| | P8 CLI and docs (2); P9 tests (4.5): corpus sweep over `lib/libredwg/test/test-data` (141 DWG, 67 DXF), goldens, determinism, the five example questions as acceptance tests, `docs/EVAL.md` | `uncad-cli`, `docs/`, `tests/` |
| **0.4.0 "Faithful"** (~30 pd) | text placement (4); MINSERT/BODY/IMAGE/LARGE_RADIAL_DIMENSION (2); lineweight, linetype, CTB colour map (3); viewport compositing + `sheets.json` (6); scale-group frames (2); tables, leaders, dynamic blocks (`*U` -> effective name) (4); xref reporting (1); batch/serve ergonomics (2); display parser (2); CI (4) | |
| **1.0 "Contract"** (~22 pd) | full DIMSTYLE formatter + XDATA overrides (4); NURBS and ellipse arcs (3); schema freeze, `non_exhaustive`, `Error` (2); docs (3); evaluation harness measuring VLM accuracy against glyph height (3); dynapi offset cache (2); EED/GROUP (2); release engineering (3) | |

Each release is independently useful: 0.3.0 already answers "area of room 101",
"where is 1250", "how many D2 doors" and dimension questions from JSON, with
legible tiles.

## 11. DWF

`.dwf` returns `ParseError::UnsupportedFormat("DWF")` and the CLI exits 2:
"DWF is a publishing format without CAD semantics (no DIMENSION objects,
act_measurement, DIMSTYLE, layer state or INSUNITS); ask for the DWG/DXF or
convert upstream." DWFx (OPC zip + XPS FixedPage XML) could be read by a
separate `uncad-dwfx` crate (15-20 pd) producing a paper-only package under the
same schema with `capabilities.dimension_values: "text_only"`; scheduled only if
real DWFx inputs appear after 1.0.

## 12. Risks and open questions

- SHX glyph widths are unknowable without the `.shx` files; boxes from the
  bundled face may differ 10-20 % (`estimated`).
- The DIMSTYLE formatter is reimplemented from the DXF reference; the cached
  `*D` text stays the value of record and every mismatch is reported.
- OCS sign errors mirror geometry silently; gated on the mirrored fixture.
  `VIEWTWIST` sign and non-plan viewports fall back to frames-only until the
  fixture exists.
- Bundled font: an OFL notice inside a GPL crate; the subset size is a target.
- The CP949 path is traced in source only (no Korean file in the corpus); the
  fixture is a prerequisite and CI needs a redistributable one.
- The absorb threshold and the frame-split rule are decided by the corpus
  sweep; both are exposed as options.
- Dynapi reads grow ~5x per entity; if sample 5's 1.4 s parse doubles, the
  offset cache moves forward.
- Rooms drawn as lines rather than closed polylines (common in older Korean
  plans) yield no region; guidance tells the agent to fall back to the image.
  Models with floors stacked in z collapse into one plan; a z histogram warning
  is the mitigation.
- Vendor token rules drift; profiles are data and the manifest records the one
  used.
- GPL-3 propagates to embedders; the practical pattern for proprietary hosts
  is a subprocess plus a directory.
- Nothing stops an agent from reading numbers off pixels; the guidance string,
  values in every sidecar and the evaluation harness are mitigations.
