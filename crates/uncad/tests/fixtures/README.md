# Test fixtures for the 0.3.0 "Readable" work (P-1)

Hand-authored DXF files that pin the facts `docs/VLM_INVESTIGATION.md` only
traced in source: the pre-R2007 code-page path, the LWPOLYLINE closed bit
and mirrored OCS, `DIMLFAC` versus `act_measurement`, and a twisted VIEWPORT.
`tests/fixtures.rs` asserts against them; the roadmap phases in
`docs/VLM_EXPORT_DESIGN.md` (section 10) flip the placeholder assertions as
they land.

Every file was written from scratch by `make_fixtures.py` in this directory
on 2026-09-21 (the viewport fixture's LAYOUT on 2026-09-22) -- no
third-party drawing was copied, so they are redistributable under the
repository's GPL-3. All are R2000 (`$ACADVER AC1015`) text DXF with CRLF
line endings, at most 2.3 KB each, and above the 256-byte minimum
LibreDWG's `dwg_read_dxf` enforces.

Ground truth below was read back through LibreDWG itself (a throw-away probe
linked against `libredwg-sys`, reading `Dwg_Data.header.codepage` and every
entity field through `dwg_dynapi_*`) and through the CLI
(`uncad <file> -o out.json --pretty`). "0.2.0" columns show what the code
produced before the 0.3.0 work; the code-page conversion (P-1), the header
(P0) and P4's closed bit, bulges and OCS transform have landed since, and
`tests/fixtures.rs` asserts the decoded values.

| File | Bytes | Objects read | Purpose |
|---|---|---|---|
| `cp949_r2000.dxf` | 855 | 9 | CP949 (`ANSI_949`) strings in TEXT, MTEXT and a LAYER name |
| `mirrored_ocs_r2000.dxf` | 925 | 7 | Closed LWPOLYLINE with extrusion (0,0,-1), an open one with a bulge, mirrored CIRCLE/ARC/TEXT |
| `dimlfac12_r2000.dxf` | 1304 | 9 | `$DIMLFAC 12.0` (header and STANDARD style) with a rotated DIMENSION whose `act_measurement` is 10.0, its `*D1` block bound |
| `twisted_viewport_r2000.dxf` | 2253 | 16 | A paper-space VIEWPORT with `VIEWTWIST` 30 degrees and every AcDbViewport view field, plus a LAYOUT `Layout1` (A4 landscape, embedded plot settings) bound to `*Paper_Space` and to the VIEWPORT |
| `hidden_layers_r2000.dxf` | 1925 | 9 entities, 7 layers, 2 linetypes | One LINE per layer state (on, off, frozen, non-plotting, `Defpoints`, locked), an invisible LINE and a 0.50 mm DASHED one |

## cp949_r2000.dxf

HEADER: `$DWGCODEPAGE ANSI_949`, `$INSUNITS 4`, `$MEASUREMENT 1`,
`$LUNITS 2`, `$DIMLFAC 1.0`. TABLES: a LAYER table with `0` and a layer
whose name is the CP949 bytes of "벽체" (the reader accepts the TABLES
section; both layers come back as LAYER objects). ENTITIES: four TEXT and
one MTEXT, all at height 2.5.

Verified through LibreDWG (`dxf_read_file` rc 0):

| Item | Value |
|---|---|
| `Dwg_Data.header.version` | 25 (`R_2000`) |
| `Dwg_Data.header.codepage` | **40** (`CP_ANSI_949`, `src/codepages.h`) |
| `$DWGCODEPAGE` | `"ANSI_949"` |
| `$INSUNITS` / `$MEASUREMENT` / `$LUNITS` / `$DIMLFAC` | 4 / 1 / 2 / 1.0 |

Strings, as raw bytes in the file and as `dwg_dynapi_entity_utf8text` hands
them back (unchanged: LibreDWG does no code-page conversion on DXF input):

| Handle | Entity | Layer | Raw bytes (CP949) | Meaning (UTF-8) | uncad 0.2.0 JSON | uncad 0.3.0 JSON (`uncad_tv_to_utf8`, asserted) |
|---|---|---|---|---|---|---|
| 23 | TEXT at (0,0) | 벽체 | `B5 B5 B8 E9` | `도면` | `"����"` | `"도면"` |
| 24 | TEXT at (0,5) | 0 | `A1 BE 33` | `±3` | `"��3"` | `"±3"` |
| 25 | TEXT at (0,10) | 0 | `33 32 2E 35 A7 B3` | `32.5㎡` | `"32.5��"` | `"32.5㎡"` |
| 26 | MTEXT at (0,20), width 50 | 0 | `B9 E6 20 31 30 31 5C 50 B8 E9 C0 FB 20 33 32 2E 35 A7 B3` | `방 101\P면적 32.5㎡` (`\P` literal) | `"�� 101\\P���� 32.5��"` | `"방 101\\P면적 32.5㎡"` |
| 27 | TEXT at (0,15) | 0 | `50 4C 41 49 4E` | `PLAIN` | `"PLAIN"` | `"PLAIN"` |
| 22 | LAYER (color 1) | | `BA AE C3 BC` | `벽체` | `"��ü"` | `"벽체"` |

Notes:

- `±` is `A1 BE` in CP949 (KS X 1001 row 1, PLUS-MINUS SIGN); the P-1 brief
  quoted `A1 B1`, which is the right double quotation mark. Python's `cp949`
  codec is the authority and the file carries `A1 BE`. The trap the case was
  meant to set still holds: under the CP1252 single-byte assumption the pair
  reads as `¡¾`, and under `from_utf8_lossy` as two U+FFFD.
- The layer name showed the worst case of the 0.2.0 lossy path: `C3 BC`
  happens to be valid UTF-8 for U+00FC, so the name came back as `��ü`
  rather than as pure replacement characters -- a wrong character, not a
  missing one. The TEXT on that layer (handle 23) carries the same layer name
  in `common.layer`.
- 0.2.0's exact U+FFFD counts followed `String::from_utf8_lossy`: one
  replacement per invalid byte, and one per truncated multi-byte lead
  (`E9` before `C0`).

## mirrored_ocs_r2000.dxf

HEADER: `$INSUNITS 4` only, so `header.codepage` is LibreDWG's DXF default
30 (`ANSI_1252`) and `$DWGCODEPAGE` reads `"ANSI_1252"`. No TABLES section:
every entity's `layer` handle is unresolved and uncad reports `layer: ""`.

| Handle | Entity | DXF groups | LibreDWG in-memory (probe) | uncad 0.2.0 | uncad 0.3.0 (asserted in `tests/fixtures.rs`) |
|---|---|---|---|---|---|
| 20 | LWPOLYLINE | `70 = 1`, 4 vertices (0,0) (100,0) (100,50) (0,50), `210/220/230 = 0,0,-1` | `flag = 513` (512 closed + 1 has-extrusion), `num_bulges = 0`, `extrusion = (0,0,-1)` | `closed: true`, 4 vertices, no extrusion | `closed: true` (now read from bit 512), `extrusion (0,0,-1)`, WCS x in [-100, 0] |
| 21 | LWPOLYLINE | `70 = 0`, same vertices, `42 = 0.41421356` after the second vertex, `210/220/230 = 0,0,1` | `flag = 16`, `num_bulges = 4`, `bulges = [0, 0.41421356, 0, 0]`, `extrusion = (0,0,1)` | `closed: false`, 4 vertices, bulges dropped | `bulges [0, 0.41421356, 0, 0]`: a 90-degree arc from (100,0) to (100,50) |
| 22 | CIRCLE | centre (10,10) r 5, `210/220/230 = 0,0,-1` | `extrusion = (0,0,-1)` | centre (10,10,0) r 5 | WCS centre (-10,10) |
| 23 | ARC | centre (0,0) r 20, 0 to 90 degrees, `210/220/230 = 0,0,-1` | angles 0 and 1.5707963267948966 rad, `extrusion = (0,0,-1)` | same angles, centre (0,0,0) | mirrored about x = 0 |
| 24 | TEXT | "MIRROR" at (10,10) h 2.5, `210/220/230 = 0,0,-1` | `extrusion = (0,0,-1)` | (10,10), "MIRROR" | WCS x = -10 |
| 25 | LINE | (-5,-5) to (5,5), no extrusion | | same | unchanged (WCS anchor) |

Caveat for the closed flag: 0.2.0's `flag & 1` test returned `true` for
handle 20 only because the reader sets bit 1 for "has extrusion", so the
right answer came out for the wrong reason; `closed` now reads bit 512
(`tests/polyline_closed.rs` checks the bit against `example_2000.dxf`'s
group 70 on 11 polylines). Handle 21 (`flag = 16`) is the one that proves
bit 1 is not "closed". A closed polyline *without* an extrusion (in-memory
`flag = 512` exactly) is not in this file; the corpus test covers that case.

## hidden_layers_r2000.dxf

HEADER: `$INSUNITS 4`. TABLES: an LTYPE table (`Continuous`, `DASHED`) and
a LAYER table:

| Layer | DXF 62 | DXF 70 | DXF 290 | State |
|---|---|---|---|---|
| `0` | 7 | 0 | omitted | on |
| `VISIBLE` | 1 | 0 | omitted | on |
| `OFF` | -3 | 0 | omitted | off (negative colour) |
| `FROZEN` | 4 | 1 | omitted | frozen |
| `NOPLOT` | 5 | 0 | 0 | non-plotting -- **reads as plotting**: LibreDWG's DXF reader cannot tell an omitted 290 from a 0, so `plot` is only trusted from R2000+ DWG files |
| `Defpoints` | 7 | 0 | 0 | hidden by name |
| `LOCKED` | 6 | 4 | omitted | locked, still drawn |

ENTITIES: one LINE per layer, in table order, from `(0, 10 i)` to
`(100, 10 i)`; then on `VISIBLE` a LINE with `60 = 1` (invisible) at
y = 100 and a LINE with `6 DASHED`, `370 = 50` (0.50 mm) and `48 = 2.0` at
y = 110. `tests/visibility.rs` asserts the layer records, each entity's
`hidden_reason` (off, frozen, defpoints, invisible; the `NOPLOT` line is
shown, see above), the entity's `lineweight_mm` / `linetype` /
`ltype_scale`, and that the renderer draws 5 of the 9 lines (all 9 with
`include_hidden`, the hidden four at 50 % opacity).

## dimlfac12_r2000.dxf

HEADER: `$INSUNITS 4`, `$DIMLFAC 12.0`, `$DIMDEC 2`, `$DIMLUNIT 2`,
`$DIMSCALE 1.0`. TABLES: a BLOCK_RECORD table with `*Model_Space` (the
reader's pre-created handle `1F`) and `*D1` (handle `40`), plus DIMSTYLE
`STANDARD` with `144 = 12.0` (DIMLFAC; a dimension uses its style's factor,
and the reader defaults a missing one to 1.0). BLOCKS: a BLOCK `*D1` (flag 1,
anonymous, `330 = 40`) containing
a TEXT "120" at (5,6), height 0.18. ENTITIES: a LINE (0,0)-(10,0) and a
DIMENSION with `100 AcDbDimension`, `2 *D1`, `10 = (10,5)` dimension-line
point, `11 = (5,6)` text midpoint, `70 = 32`, `1` empty, `42 = 10.0`,
`3 STANDARD`, `100 AcDbAlignedDimension`, `13 = (0,0)`, `14 = (10,0)`,
`50 = 0.0`, `100 AcDbRotatedDimension`.

| Item | LibreDWG in-memory (probe) | uncad 0.2.0 | Now / expected after P3 |
|---|---|---|---|
| `$DIMLFAC` / DIMSTYLE `DIMLFAC` | 12.0 / 12.0 | not read | `header.dimlfac = 12.0` (P0), `tables.dimstyles["STANDARD"].dimlfac = 12.0` and the dimension's effective `dimlfac = 12.0` (P3, asserted) |
| `$DIMDEC` / `$DIMLUNIT` / `$DIMSCALE` | 2 / 2 / 1.0 | not read | read (P0, asserted) |
| DIMENSION type | `DIMENSION_LINEAR` (upgraded on `AcDbRotatedDimension`) | `DIMENSION` | subtype `linear` |
| `act_measurement` (group 42) | **10.0** | not read | `measurement: 10.0` |
| `user_text` | `""` | not read | `user_text: ""`, display from `DIMLFAC x measurement` = 120 -> `"120.00"` with `DIMDEC 2` |
| `flag` / `flag1` | 32 / 35 | | |
| `def_pt` / `text_midpt` | (10,5,0) / (5,6) | not read | read |
| `xline1_pt` / `xline2_pt` / `dim_rotation` | (0,0,0) / (10,0,0) / 0.0 | not read | from-points length 10.0 agrees with `act_measurement` |
| `block` (group 2) / `dimstyle` (group 3) | BLOCK_RECORD `40` (name from its BLOCK: `*D1`) / `STANDARD` | `block_name: "*D1"`, `block_records["*D1"]` = [TEXT "120"] (asserted) | the label read straight from the block |
| Objects | BLOCK_HEADER `*Model_Space`, BLOCK_RECORD `*D1`, BLOCK, TEXT "120", ENDBLK, LINE, DIMENSION_LINEAR | `entities`: LINE + DIMENSION; `block_records`: `*Model_Space` (2), `*D1` (1) | |

Why the file carries a BLOCK_RECORD table: without it (the earlier
ENTITIES-only draft, `make_fixtures.py . dimlfac-minimal`) the `*D1` BLOCK,
its TEXT and ENDBLK are read but no BLOCK_HEADER named `*D1` exists, so the
DIMENSION's `2 *D1` cannot resolve and uncad reports `block_name: ""`.
Reason (`src/in_dxf.c`, `dxf_blocks_read`): for R13+ input a BLOCK is bound
to its BLOCK_HEADER only through a `330` owner handle that names a
BLOCK_RECORD table entry; the "find or create the BLOCK_HEADER by name" path
exists only for R11/R12 files. The shipped file has the table and the `330`
codes, and the probe confirms the binding (`block_texts=["TEXT:120"]`).

## twisted_viewport_r2000.dxf

HEADER: `$INSUNITS 4`. TABLES: a BLOCK_RECORD table with `*Model_Space`
(`1F`) and `*Paper_Space` (`1C`, with `340 = 2B` naming its LAYOUT).
BLOCKS: both blocks, empty. ENTITIES: a LINE (0,0)-(100,50) owned by
`*Model_Space` and a VIEWPORT owned by `*Paper_Space` (`330 = 1C`) with
handle `5 = 2A`, `67 = 1`, `100 AcDbViewport`, `10 = (150,100,0)`,
`40 = 200`, `41 = 120`, `68 = 1`, `69 = 2`, `12 = (50,25)`, `13 = (0,0)`,
`14 = (10,10)`, `15 = (10,10)`, `16 = (0,0,1)`, `17 = (0,0,0)`, `42 = 50`,
`43 = 0`, `44 = 0`, `45 = 60`, `50 = 0`, `51 = 30.0`, `72 = 100`,
`90 = 32864`. OBJECTS: the named object dictionary (`5 = C`, `330 = 0`,
`281 = 1`, one entry `3 ACAD_LAYOUT` / `350 1A`), the `ACAD_LAYOUT`
dictionary (`5 = 1A`, `330 = C`, entry `3 Layout1` / `350 2B`) and one
LAYOUT (`5 = 2B`, `330 = 1A`):

- `100 AcDbPlotSettings`: `1` empty (page setup name), `2 none_device`
  (printer), `4 ISO_A4_(210.00_x_297.00_MM)` (canonical media name),
  `40`-`43 = 6.35` (margins, mm), `44 = 210.0`, `45 = 297.0` (the unrotated
  sheet, mm), `46`-`49`, `140`, `141 = 0.0` (plot origin and window),
  `142 = 1.0`, `143 = 1.0` (paper : drawing units), `70 = 688` (plot
  flags), `72 = 1` (mm), `73 = 1` (rotated 90 degrees counter-clockwise:
  landscape), `74 = 5` (plot the layout), `7` empty (style sheet),
  `75 = 16` (1:1), `147 = 1.0`, `148`, `149 = 0.0` (paper image origin).
- `100 AcDbLayout`: `1 Layout1`, `70 = 1`, `71 = 1` (tab order),
  `10 = (-6.35,-6.35)`, `11 = (290.65,203.65)` (LIMMIN/LIMMAX: the printable
  area of the rotated sheet), `12 = (0,0,0)`, `14 = (1e20,1e20,1e20)`,
  `15 = (-1e20,-1e20,-1e20)` (the "never computed" extents sentinels
  AutoCAD writes for a layout that has not been plotted or zoomed),
  `146 = 0.0`, `13 = (0,0,0)`, `16 = (1,0,0)`, `17 = (0,1,0)`, `76 = 0`,
  `330 = 1C` (block record), `331 = 2A` (active viewport).

Deliberately absent: a `Model` LAYOUT, a plot view (`6`), reactors and
extension dictionaries, `345`/`346` UCS handles, the R2004+ shade-plot
groups.

| Field | LibreDWG in-memory (probe) | uncad 0.2.0 | Now / expected after P7 |
|---|---|---|---|
| `center` / `width` / `height` | (150,100,0) / 200 / 120 | same | same |
| `on_off` (68) / `id` (69) | 1 / 2 | not read | `status_flag`, `id` |
| `VIEWCTR` (12) | (50,25) | not read | `view_center` |
| `VIEWSIZE` (45) | 60.0 | not read | `view_size`; sheet scale = 120 / 60 = 2 |
| `VIEWTWIST` (51) | **0.5235987755982988 rad** (the reader converts DXF degrees to radians) | not read | `twist` = 30 degrees |
| `VIEWDIR` (16) / `view_target` (17) | (0,0,1) / (0,0,0) | not read | plan view |
| `LENSLENGTH` (42) / `status_flag` (90) | 50.0 / 32864 | not read | |
| `entmode` | **1 (paper space)** | `block_records["*Paper_Space"]` = [VIEWPORT], `--space paper` renders the 200 x 120 frame (viewBox `45 -165 210 130`) (asserted) | model content composited through the viewport |
| Extra objects | `VX_CONTROL` (handle 1), `VX_TABLE_RECORD` (handle 2) | ignored | LibreDWG's own R2000 artefact: a VIEWPORT with a `5` handle gets a VX record (`in_dxf.c`, "special-case VIEWPORT -> VX") |

Why the file carries BLOCKS: `67 = 1` on its own does not move an entity to
paper space. `dxf_entities_read` assigns `entmode 1` only when the entity's
`330` owner handle equals `BLOCK_RECORD_PSPACE`, and that header handle is
only set after a BLOCKS section defines `*Paper_Space`. The earlier
ENTITIES-only draft (`make_fixtures.py . viewport-minimal`) therefore put the
VIEWPORT in model space. The shipped file has the blocks and owners, and the
probe confirms `entmode 1`.

The LAYOUT as LibreDWG hands it back (2026-09-22; the AcDbLayout fields
through `dwg_dynapi_entity_value(obj, "LAYOUT", ...)`, the embedded plot
settings at `dwg_dynapi_entity_field("LAYOUT", "plotsettings")->offset`
through `dwg_dynapi_entity_value(sub, "PLOTSETTINGS", ...)` or
`dwg_dynapi_subclass_value(sub, "Dwg_Object_PLOTSETTINGS", ...)` --
`dwg_dynapi_subclass_value(sub, "PLOTSETTINGS", ...)` returns false):

| Field (DXF group) | LibreDWG in-memory (probe) | uncad 0.3.0 today | After P7 |
|---|---|---|---|
| object | `LAYOUT`, handle `2B`, `ownerhandle` `1A` (the `ACAD_LAYOUT` DICTIONARY) | ignored: no layouts in the model, `entities` and `block_records` unchanged | `layouts["Layout1"]` |
| `layout_name` (1) / `tab_order` (71) / `layout_flags` (70) | `"Layout1"` / 1 / 1 | | |
| `block_header` (330) | `1C` -> BLOCK_RECORD `*Paper_Space`; and `BLOCK_HEADER 1C.layout` -> `2B` from the table's `340` | | the layout's block |
| `active_viewport` (331) | `2A` -> VIEWPORT | | |
| `LIMMIN` / `LIMMAX` (10/11) | (-6.35,-6.35) / (290.65,203.65) | | sheet extents in paper units |
| `EXTMIN` / `EXTMAX` (14/15) | (1e20,1e20,1e20) / (-1e20,-1e20,-1e20), the sentinels exactly as written | | "unset" |
| `INSBASE` (12) / `UCSORG` (13) / `UCSXDIR` (16) / `UCSYDIR` (17) / `ucs_elevation` (146) / `UCSORTHOVIEW` (76) | (0,0,0) / (0,0,0) / (1,0,0) / (0,1,0) / 0.0 / 0 | | |
| `num_viewports` / `viewports` | 0 / empty: a DWG-only (R2004+) list, DXF has no group for it | | |
| `plotsettings.printer_cfg_file` (1) / `paper_size` (2) / `canonical_media_name` (4) | `""` (set, not NULL) / `"none_device"` / **`"ISO_A4_(210.00_x_297.00_MM)"`** | | paper name |
| `plotsettings.plotview_name` (6) / `plotview` / `stylesheet` (7) | NULL (no `6` written) / null ref / `""` | | |
| `left/bottom/right/top_margin` (40-43) | 6.35 each | | |
| `paper_width` / `paper_height` (44/45) | **210.0 / 297.0** (mm, unrotated) | | 297 x 210 sheet once `plot_rotation_mode` is applied |
| `plot_origin` (46) / `plot_window_ll` (48) / `plot_window_ur` (140) / `paper_image_origin` (148) | (0,0) each | | |
| `plot_paper_unit` (72) / `plot_rotation_mode` (73) / `plot_type` (74) | 1 (mm) / **1** (90 degrees counter-clockwise) / 5 (layout) | | |
| `std_scale_type` (75) / `std_scale_factor` (147) / `paper_units` (142) / `drawing_units` (143) | 16 (1:1) / 1.0 / 1.0 / 1.0 | | scale 1:1 |
| `plot_flags` (70 under `AcDbPlotSettings`) | 688 (0x2b0) | | |
| `shadeplot_type` / `shadeplot_reslevel` / `shadeplot_customdpi` | 0 / 0 / 0 (R2004+ groups, not written) | | |
| `HEADER.DICTIONARY_NAMED_OBJECT` / `DICTIONARY_LAYOUT` / `DICTIONARY_PLOTSETTINGS` | `C` / `1A` / null | | |
| Objects | 16: the 13 above (`BLOCK_HEADER`, `LAYER_CONTROL`, `LAYER`, `BLOCK_CONTROL`, `BLOCK_RECORD`, 2 x `BLOCK`/`ENDBLK`, `LINE`, `VIEWPORT`, `VX_CONTROL`, `VX_TABLE_RECORD`) plus `DICTIONARY` `C`, `DICTIONARY` `1A`, `LAYOUT` `2B` | | |

Why the file carries an OBJECTS section, and what the reader needs from it
(`src/in_dxf.c`, read at trace level so every group's landing is visible):

- `dxf_objects_read` turns any `0 LAYOUT` into a LAYOUT object by itself;
  no dictionary is needed for the object to exist. Every group above landed
  in the field named (`set LAYOUT.plotsettings.<field> [.. <group>]` and
  `LAYOUT.<field> = ... [.. <group>]` in the trace); nothing written was
  ignored or altered. Values that equal a default cannot be told from the
  default by value alone: the zero points, and `paper_units` (142), which
  the reader forces to 1.0 for every LAYOUT before it reads the groups.
- The two `330`s matter. The first (before the `100` markers) becomes the
  object's `ownerhandle` through the generic path; the second, inside
  `AcDbLayout`, is taken as `block_header` only because an owner is already
  set (the LAYOUT branch guards on `ownerhandle`). With a single `330` the
  block record would become the owner and `block_header` would stay null.
  `331` is stored as an absolute reference and resolves to the VIEWPORT.
- The named object dictionary (which must be the file's first DICTIONARY)
  and its `ACAD_LAYOUT` entry are what `CHECK_DICTIONARY_HDR(LAYOUT)` needs
  to set `HEADER.DICTIONARY_LAYOUT` (it tries the entry names `LAYOUT`, then
  `ACAD_LAYOUT`). Without any DICTIONARY the reader makes an empty NOD of
  its own and that header handle stays null; the LAYOUT is unaffected.
- `340` on the BLOCK_RECORD goes through the generic handle path (codes
  above 300 are hex handles) into `BLOCK_HEADER.layout`, so the block and
  the layout point at each other.
- A `6` plot-view name is only taken when non-empty (and then looked up as
  a table handle in `dxf_postprocess_LAYOUT`), so it is omitted;
  `plotview_name` comes back NULL rather than `""`.
- At warning level the reader prints the same three "Duplicate handle
  20/21/22" errors it printed before the OBJECTS section existed: its own
  LAYER_CONTROL, LAYER and BLOCK_CONTROL take 20-22 before the BLOCK/ENDBLK
  entities with those explicit handles arrive. Harmless so far (nothing
  refers to an ENDBLK), but new handles in this file must avoid 1, 2, 20-23
  as well as C, 1A, 1C, 1F, 24, 2A and 2B.

## What did not work

1. **`cp949_r2000.dwg` via LibreDWG's add/write API**: `dwg_add_Document`,
   `dwg_add_TEXT`, `dwg_add_MTEXT`, `dwg_add_LINE`, `dwg_add_LWPOLYLINE`,
   `dwg_add_LAYER` and `dwg_write_file` are compiled into the static library
   (`config.h` defines `USE_WRITE`; only the bindgen allowlist omits them).
   A scratch program that sets `header.version = R_2000` and
   `header.codepage = 40` before `dwg_add_Document`, adds the same strings as
   the DXF fixture and calls `dwg_write_file` does produce an R2000 DWG that
   reads back with `codepage = 40` -- but LibreDWG's UTF-8 -> code-page
   encoder mangles every non-ASCII string on the way in (the layer came back
   as `;`, "±3" as `1B 33`, "32.5㎡" as `32.5{`, with
   `Warning: utf-8: BAD_CONTINUATION_BYTE` at write time), and setting
   `$DWGCODEPAGE` through `dwg_dynapi_header_set_value` crashed. So no DWG is
   shipped: the code-page fixture is DXF-only. That is not a gap in what the
   test proves -- the shim takes the same non-TU path for a pre-R2007 DWG and
   for any DXF, and the R2004 sample drawings (CP1252, `±` and `°` in
   dimension text) exercise the DWG side. An AutoCAD-written Korean R2000
   DWG is still wanted as a fixture.
2. **A closed LWPOLYLINE without extrusion** in the mirrored fixture (the
   case where `flag & 1` and `flag & 512` disagree directly) is not in the
   file; `tests/polyline_closed.rs` covers it with the corpus's
   `example_2000.dwg`/`.dxf` pair instead.
3. **`±` as `A1 B1`**: not a valid encoding of the character; the file uses
   `A1 BE`.
4. **A `Model` LAYOUT, a plot view (`6`) and a page-setup name** are not
   in the viewport fixture (its `Layout1` LAYOUT was added 2026-09-22): the
   reader needs none of them, and the `num_viewports` list cannot come from
   DXF at all.

## Regenerating

```
python crates/uncad/tests/fixtures/make_fixtures.py                    # the shipped files
python crates/uncad/tests/fixtures/make_fixtures.py . dimlfac-minimal  # the ENTITIES-only draft (unbound *D1)
python crates/uncad/tests/fixtures/make_fixtures.py . viewport-minimal # the ENTITIES-only draft (model-space VIEWPORT)
```

The default mode is meant to reproduce the shipped bytes exactly; check with
`git status` after running it. Verify any regenerated file the same way the
shipped ones were: read it through `libredwg-sys` (`dxf_read_file`, then
`dwg_dynapi_entity_value` / `dwg_dynapi_entity_utf8text` per entity and the
`codepage` field of `Dwg_Data.header`; for the viewport fixture also the
LAYOUT and its embedded `plotsettings`, see that section) and through
`uncad <file> -o out.json --pretty`, and update the tables above and
`tests/fixtures.rs`.
