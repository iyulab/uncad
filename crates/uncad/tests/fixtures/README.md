# Test fixtures for the 0.3.0 "Readable" work (P-1)

Hand-authored DXF files that pin the facts `docs/VLM_INVESTIGATION.md` only
traced in source: the pre-R2007 code-page path, the LWPOLYLINE closed bit
and mirrored OCS, `DIMLFAC` versus `act_measurement`, and a twisted VIEWPORT.
`tests/fixtures.rs` asserts against them; the roadmap phases in
`docs/VLM_EXPORT_DESIGN.md` (section 10) flip the placeholder assertions as
they land.

Every file was written from scratch by `make_fixtures.py` in this directory
on 2026-09-21 (the viewport fixture's LAYOUT, the plot-origin, the
angular/ordinate, the hatched-viewport, the nested-attrib, the
viewport-states, the radial and the infinite-lines fixtures on 2026-09-22; the
polyline-vertices, entity-truecolor, polyface-mesh and block-layer0 fixtures on
2026-09-23)
-- no third-party drawing was copied, so they are redistributable under the
repository's GPL-3. All are R2000 (`$ACADVER AC1015`) text DXF with CRLF
line endings, between 470 and 4387 bytes, and above the 256-byte minimum
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
| `mirrored_bulge_r2000.dxf` | 470 | 3 | The bulged outline of the mirrored fixture in an OCS with extrusion (0,0,-1), next to an ARC tracing the same arc: the bulge sign flips with the reflection |
| `dimlfac12_r2000.dxf` | 1304 | 12 | `$DIMLFAC 12.0` (header and STANDARD style) with a rotated DIMENSION whose `act_measurement` is 10.0, its `*D1` block bound |
| `twisted_viewport_r2000.dxf` | 2253 | 16 | A paper-space VIEWPORT with `VIEWTWIST` 30 degrees and every AcDbViewport view field, plus a LAYOUT `Layout1` (A4 landscape, embedded plot settings) bound to `*Paper_Space` and to the VIEWPORT |
| `hidden_layers_r2000.dxf` | 1925 | 9 entities, 7 layers, 2 linetypes | One LINE per layer state (on, off, frozen, non-plotting, `Defpoints`, locked), an invisible LINE and a 0.50 mm DASHED one |
| `plot_origin_r2000.dxf` | 2430 | 17 | A LAYOUT `Layout1` in inches (ANSI B landscape, rotation 0) with asymmetric margins and a non-zero plot origin (DXF 46/47), so the sheet is not at `(-left, -bottom)`; a paper-space border LWPOLYLINE and a 1:5 plan VIEWPORT over a model LINE |
| `angular_ordinate_r2000.dxf` | 1491 | 10 | A 2-line angular DIMENSION (60 degrees) and an X- and a Y-type ordinate DIMENSION (30 and 50): the two kinds whose definition points LibreDWG's DXF reader lays out differently from its DWG decoder |
| `viewport_states_r2000.dxf` | 4387 | 5 entities, 2 layers, 2 layouts | One VIEWPORT per state the sheet compositing rules distinguish (on; on a frozen layer; off; non-plan) plus two page setups no other fixture has: plot rotation 2 with asymmetric margins in mm, and plot rotation 3 in inches |
| `radial_r2000.dxf` | 1425 | 6 entities, 1 DIMSTYLE | A RADIUS, a DIAMETER and a 3-point angular DIMENSION -- the three kinds no corpus DXF carries -- each with its circle or arc, no cached `*D` block |
| `hatched_viewport_r2000.dxf` | 3117 | 18 | The twisted-viewport fixture plus a pattern HATCH in model space (under the viewport) and one in paper space (outside its frame): the composited sheet must keep their `<defs>` apart |
| `nested_attrib_r2000.dxf` | 2174 | 3 model-space entities, 2 blocks (6 entities) | A block with an ATTDEF inserted inside another block, its ATTRIB value owned by the block record: the attribute of a *nested* block reference, which the export used to drop |
| `infinite_lines_r2000.dxf` | 673 | 4 | An XLINE and a RAY through the middle of a drawing 0.002 units across: the two entities with no end, at a scale where drawing them a fixed 1e6 units long panicked the rasterizer |
| `polyline_vertices_r2000.dxf` | 2322 | 3 | A closed 4-vertex `POLYLINE_2D`, a 2-vertex one with bulge 1.0 and a 5-vertex `POLYLINE_3D`: the last vertex of each is what LibreDWG's own point accessors drop on any pre-R2004 file |
| `entity_truecolor_r2000.dxf` | 795 | 4 | One LINE per way a DXF can state a colour (420 alone, 62 alone, 62 then 420, neither) on a layer whose own ACI is 3 |
| `polyface_mesh_r2000.dxf` | 3921 | 3 | A `POLYLINE_PFACE` and a `POLYLINE_MESH` whose VERTEX records name the block record as their owner, next to a LINE: the shape that made LibreDWG refuse the whole file |
| `block_layer0_r2000.dxf` | 1295 | 1 entity, 2 blocks (4 entities), 3 layers | A block drawn on layer 0 inserted on a coloured layer: AutoCAD's "layer 0 in a block means the layer of the reference" rule |

| `title_block_r2000.dxf` | 2939 | 4 entities, 3 block records, 1 layout | A drawing whose every string is in *paper* space: a title TEXT and a title-block INSERT on an A4 sheet over a model with no text at all -- the shape that made a package answer "this drawing contains no text" |

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

## mirrored_bulge_r2000.dxf

HEADER: `$INSUNITS 4` only; no TABLES section (`layer: ""` as in the
mirrored fixture). Added 2026-09-22 for the bulge-sign fix: the OCS-to-world
map of extrusion (0,0,-1) is the reflection `x -> -x`, which reverses every
arc's turning direction, so a polyline's bulges must change sign together
with its vertices (`convert.rs`, `mirror_bulges`). The ARC next to it is the
same arc through the ARC branch, which already negated its angles.

| Handle | Entity | DXF groups | uncad (asserted in `tests/polyline_geometry.rs`) |
|---|---|---|---|
| 20 | LWPOLYLINE | `70 = 0`, vertices (0,0) (100,0) (100,50) (0,50), `42 = 0.41421356` after the second vertex, `210/220/230 = 0,0,-1` | WCS vertices (0,0) (-100,0) (-100,50) (0,50), `bulges [0, -0.41421356, 0, 0]`: a clockwise 90-degree arc from (-100,0) to (-100,50), centre (-75,25), apex (-110.355,25), `polyline_bounds` min x -110.355; SVG sweep flag 1 |
| 21 | ARC | centre (75,25) r 35.35533906, 315 to 45 degrees, `210/220/230 = 0,0,-1` | WCS centre (-75,25), angles 135 and -135 degrees (the reader swaps and negates them for a mirrored OCS): the same arc, drawn with sweep flag 0 |

Both curves rasterize with ink at (-110.355,25) and none at (-89.645,25),
the apex's mirror image inside the rectangle.

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

## plot_origin_r2000.dxf

The same skeleton as the viewport fixture (`*Model_Space` 1F, `*Paper_Space`
1C with `340 = 2B`, BLOCK/ENDBLK 20-23, the named object dictionary `C`,
`ACAD_LAYOUT` `1A`, LAYOUT `2B`, VIEWPORT `2A`), `$INSUNITS 1` (inches),
and the page setup AutoCAD-written drawings usually carry: `72 = 0` (inch
paper units), `73 = 0` (no rotation), ANSI B `44/45 = 431.8 x 279.4` mm
(17 x 11 in), margins `40..43 = 6.35 / 19.05 / 6.35 / 19.05` mm (0.25 /
0.75 / 0.25 / 0.75 in) and a plot origin `46/47 = -6.35 / -12.7` mm
(-0.25 / -0.5 in). AutoCAD places the layout origin at the printable
corner moved by the plot origin, so the paper runs from
`-(margin + origin)` = `(-(0.25 - 0.25), -(0.75 - 0.5))` = `(0, -0.25)` to
`(17, 10.75)` in, and that is what the file's `LIMMIN/LIMMAX` (10/11) say.
The rule is ezdxf 1.4.4's `reset_paper_limits`; on 2026-09-22 it was
checked against the seven AutoCAD-written sample layouts (`-(margin +
origin)` equals the stored limits within 0.005 in on the six with rotation
0; the rotated one keeps limits matching neither formula, which is why
`export` trusts the stored limits first).

ENTITIES: a model LINE `24` (0,0) -> (100,50) owned by 1F; on paper (owner
1C, `67 = 1`) a closed border LWPOLYLINE `25` (0.5, 0.25) .. (16.5, 10.5)
-- its top edge lies above the 10.25 in a margins-only sheet would end at
-- and the VIEWPORT `2A`: centre (8.5, 5.5), 12 x 8 in, `VIEWCTR` (50,
25), `VIEWSIZE` 40 (scale 8 / 40 = 1:5), no twist, `68 = 1`, `69 = 2`,
`90 = 32864`.

Verified through LibreDWG (17 objects; the probe of the viewport fixture's
section): `LIMMIN (0, -0.25)`, `LIMMAX (17, 10.75)`, `plotsettings`
`margins 6.35/19.05/6.35/19.05`, `paper 431.8 x 279.4`, `plot_origin
(-6.35, -12.7)`, `plot_paper_unit 0`, `plot_rotation_mode 0`, VIEWPORT
`entmode 1`. `tests/sheets.rs` asserts `PlotSettings::sheet_rect()` and the
limits both give `(0, -0.25) .. (17, 10.75)`, that the export takes the
limits (`rect_source: "layout_limits"`) and that the border's top edge is
inside the sheet image.

## angular_ordinate_r2000.dxf

HEADER: `$INSUNITS 4`, `$DIMDEC 2`, `$DIMADEC 0`, `$DIMLUNIT 2`. TABLES: a
LAYER table and DIMSTYLE `STANDARD` (handle 30). ENTITIES: two LINEs
(0,0) -> (10,0) and (0,0) -> (5, 8.660254), then three DIMENSIONs with no
cached `*D` blocks (the labels are formatted from the values), every
number derived by hand:

| Handle | Subclass | 70 | 10 | 13 | 14 | 15 | 16 | 42 | Value |
|---|---|---|---|---|---|---|---|---|---|
| 33 | `AcDb2LineAngularDimension` | 34 | (5, 8.660254) = line 2's end | (0,0) | (10,0) | (0,0) | (4.330127, 2.5) = arc point, 30 degrees along r = 5 | pi/3 | 60 degrees |
| 34 | `AcDbOrdinateDimension` | 102 (bit 64: X type) | (100, 200) datum | (130, 250) feature | (130, 270) leader | | | 30.0 | 130 - 100 = 30 |
| 35 | `AcDbOrdinateDimension` | 38 (Y type) | (100, 200) | (130, 250) | (150, 250) | | | 50.0 | 250 - 200 = 50 |

What the file pins (verified 2026-09-22 through `uncad <file> -o out.json`,
10 objects read): LibreDWG's DXF reader maps groups by code (`def_pt` =
10, `xline2end_pt` = 16), its DWG decoder by stream order (the leading
`def_pt` 2RD is the arc point, `xline2end_pt` the last point = group 10),
so `convert.rs` swaps the two for DXF input. Read with them swapped the
angular probe sits at 60 degrees between rays at 30 and 180 degrees and
the value comes out as 150; the ordinate's type is bit 64 of group 70 in a
DXF (`flag`) and bit 1 of the stream-only `flag2` in a DWG, so without the
branch both ordinates read as Y type and handle 34 gives 50.
`tests/dimensions.rs` asserts 60 / 30 / 50 from the definition points,
`x_datum` true / false, the arc point as `definition_point` and (5,
8.660254) as `line2_end`; the corpus's `example_2000.dwg`/`.dxf` pair
(same drawing, ten dimensions including one of each kind) is asserted to
agree between the two readers.
## hatched_viewport_r2000.dxf

`twisted_viewport_r2000.dxf` in its full form (the same HEADER, TABLES,
BLOCKS, LINE `24`, VIEWPORT `2A` and OBJECTS section, so everything that
section says holds here) plus one pattern HATCH per space, written by
`make_fixtures.py`'s `hatch` helper: a user-defined pattern (`76 = 0`, `2
USER`) of one definition line (`78 = 1`) with no dashes, one closed
polyline boundary (`92 = 3`, `73 = 1`), style outermost (`75 = 1`), and a
seed point one unit inside the first vertex. Added 2026-09-22 for the
sheet-defs fix.

| Handle | Entity | Space | Boundary | Definition line (53 / 45,46) | uncad |
|---|---|---|---|---|---|
| 30 | HATCH | model (`330 = 1F`) | (20,10) (60,10) (60,30) (20,30) | 90 degrees, offset (-2, 0): vertical lines 2 units apart | `pattern_lines[0].angle` pi/2, `offset (-2, 0)`; the first pattern of a model render, `<pattern id="hp0">` |
| 31 | HATCH | paper (`330 = 1C`, `67 = 1`) | (10,10) (40,10) (40,30) (10,30) | 0 degrees, offset (0, 4): horizontal lines 4 units apart | `angle` 0, `offset (0, 4)`; the first pattern of a paper render, `<pattern id="php0">` (prefix `p`) |

The paper hatch lies outside the viewport's 200 x 120 frame (x 50..250,
y 40..160 on the sheet), so on `sheets/Layout1/overview.png` its rows are
either full (a horizontal line) or empty; `tests/sheets.rs` asserts that,
and the `assemble_sheet` unit test in `svg.rs` asserts the ids are unique,
each hatch references its own pattern and the model pattern's line inside
the scale-2 viewport is half the sheet stroke. Objects read: the 16 of the
viewport fixture plus the two HATCHes.

## nested_attrib_r2000.dxf

The tag-inside-assembly pattern: an attributed block inside another block,
written by `make_fixtures.py`'s `nested_attrib()` on 2026-09-22 for the
nested-attribute fix. HEADER: `$INSUNITS 4`. TABLES: BLOCK_RECORDs `1F`
(`*Model_Space`), `40` (`TAG`) and `50` (`DOOR`). BLOCKS: `TAG` = LINE `42`
(0,0)-(10,0) and ATTDEF `43` (tag `NUM`, default `D-000`, height 2.5);
`DOOR` = LINEs `52` and `53` plus INSERT `55` of `TAG` at (20, 20) whose
ATTRIB `56` (`NUM` = `D-101`, at (21, 21)) is owned by the **block record**
(`330 = 50`) -- the shape ezdxf- and AutoCAD-written DXFs use. ENTITIES:
INSERT `60` of `DOOR` at (100, 100) and INSERT `61` of `TAG` at (0, 0) with
its own ATTRIB `62` (`NUM` = `D-TOP`), the control that always worked.

| Item | uncad |
|---|---|
| `tables.block_records["DOOR"].entities` | LINE `52`, LINE `53`, INSERT `55` (`attribs` empty at R2000), ATTRIB `56` = `D-101` |
| `tables.block_records["TAG"].entities` | LINE `42`, ATTDEF `43` |
| `entities` (model space) | INSERT `60`, INSERT `61`, ATTRIB `62` = `D-TOP` |
| `texts.json` | `60/56` = `D-101` at (121, 121), height 2.5, and `62` = `D-TOP` |
| `strings.json` | `d-101` -> `["60/56"]`, `d-top` -> `["62"]` |
| `drawing.svg` | one `<text id="60/56">D-101</text>` |

An R2000 DXF gives only this shape: LibreDWG links an ATTRIB to its INSERT
from R2004 on, and a DWG stores the value on the INSERT itself with no
block child. `tests/export.rs` covers that second shape with a database
built in the test, and both reach the same record id.

## viewport_states_r2000.dxf

The viewport fixture's skeleton with a second paper layout and four
viewports, so the sheet compositing rules (`export.rs`, the `composited`
filter) have one viewport per state. TABLES: LAYERs `0` (colour 7) and
`VPFROZEN` (colour 4, `70 = 1`: frozen, the usual way to hide a viewport's
border); BLOCK_RECORDs `*Model_Space` (`1F`), `*Paper_Space` (`1C`,
`340 = 2B`) and `*Paper_Space0` (`1D`, `340 = 2C`). BLOCKS: all three
(`20`/`21`, `22`/`23`, `26`/`27`), empty. ENTITIES: the model LINE `24`
(0,0) -> (100,50) of the other viewport fixtures, then four VIEWPORTs owned
by `1C` (`67 = 1`), each a 60 x 40 frame showing the model window 30 x 20
about (50,25) -- `12 = (50,25)`, `45 = 20`, so the scale is 40 / 20 = 2 --
with `69` counting up from 2 so none is the overall frame:

| Handle | `10` centre | Layer | `68` | `90` | `16` VIEWDIR | Composited | Border |
|---|---|---|---|---|---|---|---|
| `2A` | (50, 50) | `0` | 1 | 32864 | (0,0,1) | yes | drawn |
| `2D` | (50, 120) | `VPFROZEN` | 1 | 32864 | (0,0,1) | yes -- a hidden border does not hide the window | none: the entity is not among the sheet's paper parts |
| `2E` | (50, 190) | `0` | **0** | **163936** (`32864 \| 0x20000`) | (0,0,1) | no: switched off | drawn |
| `2F` | (140, 50) | `0` | 1 | 32864 | **(1,1,1)** | no: not a plan view | drawn |

OBJECTS: the named object dictionary (`C`), the `ACAD_LAYOUT` dictionary
(`1A`) with both entries, and two LAYOUTs:

- `2B` `Layout1`, tab 1, block `1C`, active viewport `2A`: ISO A4
  `44/45 = 210 x 297` mm, `73 = 2` (upside down, so the sheet keeps its
  portrait size), `72 = 1` (mm), margins `40..43 = 10 / 20 / 5 / 15` mm, no
  plot origin. The sheet therefore runs from `-(left, bottom)` = (-10, -20)
  to (210 - 10, 297 - 20) = (200, 277) mm, which is what `10`/`11`
  (LIMMIN/LIMMAX) say.
- `2C` `Layout2`, tab 2, block `1D` (empty), no active viewport: ANSI B
  `44/45 = 431.8 x 279.4` mm, `73 = 3` (90 degrees clockwise, so the
  landscape sheet becomes portrait 279.4 x 431.8), `72 = 0` (**inches**),
  margins 6.35 mm all round, no plot origin: (-0.25, -0.25) to
  (273.05 / 25.4, 425.45 / 25.4) = (10.75, 16.75) in.

Verified 2026-09-22 through `uncad <file> -o out.json` and `uncad export`:
the four VIEWPORTs read back with the layers, `on`, `status_flag`, `id` and
`view_direction` above, both LAYOUTs with those limits and plot settings,
and `sheets.json` reports `2A`/`2D` composited, `2E`/`2F` not, with
`Layout1` holding 3 paper entities (`2D`'s border is on the frozen layer).
`tests/sheets_compositing.rs` asserts all of that, that the picture has ink
at each composited frame's centre and none at the others', and that
`PlotSettings::sheet_rect()` agrees with both layouts' stored limits (the
rotation swap and the 1/25.4 inch conversion).

Handles avoid the ones LibreDWG's reader takes for itself (1, 2, 20-23) as
well as `C`, `1A`, `1C`, `1D`, `1F`, `24`, `26`, `27`, `2A`-`2F`; each
VIEWPORT with a `5` handle still gets its own `VX_TABLE_RECORD` artefact, as
the viewport fixture's section describes.

## radial_r2000.dxf

The three dimension kinds no corpus DXF carries. `2000/TS1.dwg` has all
three (`tests/dimensions.rs` reads them from it, and its AutoCAD-written
labels `R1.1897`, `∅2.3794` and `45°` are the
reference for the values), but every TS1 DXF in the corpus fails LibreDWG's
reader with a critical error, so the DXF side needs a fixture of its own.

HEADER: `$INSUNITS 4`, `$DIMDEC 2`, `$DIMADEC 0`, `$DIMLUNIT 2`. TABLES: a
LAYER table and DIMSTYLE `STANDARD` (handle 30). ENTITIES: the circle or arc
each dimension annotates, then the three DIMENSIONs with no cached `*D`
blocks, every number derived by hand:

| Handle | `70` | Subclass | `10` | `13` | `14` | `15` | `42` | Value |
|---|---|---|---|---|---|---|---|---|
| 34 | 36 (4 radius) | `AcDbRadialDimension` | (0,0) centre | | | (3,4) on the circle | 5.0 | 3-4-5: radius 5 |
| 35 | 35 (3 diameter) | `AcDbDiametricDimension` | (20,0) chord start | | | (20,10) chord end | 10.0 | the vertical diameter of the r = 5 circle at (20,5) |
| 36 | 37 (5 angular 3-point) | `AcDb3PointAngularDimension` | (44.330127, 2.5) arc point | (50,0) | (45, 8.660254) | (40,0) centre | pi/3 | the 60-degree sector |

The arc point of the angular dimension lies at 30 degrees along a radius of
5 -- inside the 60-degree sector between the rays at 0 and 60 degrees, not
the 300-degree one on the other side, which is what the sector probe in
`dimension::measurement_from_points` has to get right.

What the file pins (verified 2026-09-22 through `uncad <file> -o out.json`,
6 entities read): LibreDWG's DXF reader maps `def_pt` to group 10 and
`first_arc_pt` / `center_pt` to group 15, and `convert.rs` turns those into
`Radius { center, chord_point }`, `Diameter { chord_start, chord_end }` and
`Angular3Point { center, xline1, xline2 }`, so the recomputed measurement
equals the written `42` for each. Mapping the radius' chord point to the
definition point (the centre) makes the value 0 and fails the test. Since no
`*D` block is cached, the labels are formatted: `5.00`, `10.00` and
`60°` -- the `R` and diameter-sign prefixes are DIMPOST, which
the formatter does not apply yet (`docs/VLM_EXPORT_DESIGN.md`, "what stays
out").

## infinite_lines_r2000.dxf

The two entities that have no end, at a scale that makes the difference
between "long" and "as far as the picture goes" fatal. Written by
`make_fixtures.py`'s `infinite_lines()` on 2026-09-22 for the RAY/XLINE
clipping fix. HEADER: `$INSUNITS 4`. TABLES: a LAYER table with `0` only.
ENTITIES, all in model space:

| Handle | Entity | Groups | uncad |
|---|---|---|---|
| `30` | LINE | `10` (0,0), `11` (0.002, 0.002) | the diagonal; the only entity with a size, so the crop is its box |
| `31` | TEXT | `10` (0.0013, 0.0002), `40` 0.0006, `1` `X` | one text class, which is what lets the export build a tile pyramid at all (`depth_for` returns one level for a drawing with no text) |
| `32` | XLINE (`AcDbXline`) | `10` (0.001, 0.001), `11` (1, 0) | a horizontal line across the whole image, through the middle |
| `33` | RAY (`AcDbRay`) | `10` (0.001, 0.001), `11` (0, 1) | a vertical line from the middle to the top edge -- and nothing below it |

What the fixture pins (`tests/fixtures.rs`):

| Item | uncad |
|---|---|
| crop / viewBox | `[0, 0, 0.002, 0.002]` -> `viewBox="-0.00004 -0.00204 0.00208 0.00208"`, the same with the XLINE and the RAY removed: their extent is the base point alone |
| `drawing.svg` | two `stroke-dasharray="4,2"` lines, every coordinate within a few pictures of the picture (before 0.3.0 they ran to +/-1000000.001) |
| PNG at 256, 420, 512, 1024 and 1568 px | ink at (0.0004, 0.001) and (0.0016, 0.001) -- the XLINE both ways -- and at (0.001, 0.0016); white at (0.001, 0.0004), where a RAY must not be drawn |
| `export_package` with `target_text_px` 50000, 3 levels | 3 levels, deepest 4.2e6 px/unit, and the XLINE crosses a row of deep tiles whose own extent it never touches |

Before 0.3.0 both were drawn as a segment 1e6 units long. A 1568 px image of
this drawing is about 750 thousand pixels per drawing unit, so that endpoint
sat 7e11 px off the canvas: usvg passes it through, tiny-skia's scan
converter builds its 24.8 fixed-point edge list out of it, and the
assertion `edges[curr_idx].last_y >= curr_y as i32` fails (`--fit 256` and
`--fit 8000` panicked; 512, 1024 and 1568 happened to survive the same
document, which is how the bug stayed hidden). The corpus's
`2000/ConstructionLine.dwg` holds one XLINE and survives only because its
crop is around 1 unit wide.

## title_block_r2000.dxf

The drawing whose name is not in the drawing. Written by
`make_fixtures.py`'s `title_block()` on 2026-09-23 for the paper-space text
index. HEADER: `$INSUNITS 4`. TABLES: a LAYER table with `0`, and
BLOCK_RECORDs `1F` `*Model_Space`, `1C` `*Paper_Space` (LAYOUT `2B`) and
`40` `TITLEBLOCK`. BLOCKS: the two spaces plus `TITLEBLOCK`. OBJECTS: the
named object dictionary, `ACAD_LAYOUT` and one LAYOUT `Layout1`, A4
landscape (`ISO_A4_(210.00_x_297.00_MM)`, rotation 1, 6.35 mm margins, no
plot origin).

| Handle | Space | Entity | Groups | uncad |
|---|---|---|---|---|
| `24` | model | LINE | `10` (0,0), `11` (100,50) | the whole model: no text anywhere in it |
| `25` | paper | TEXT | `10` (150,20), `40` 8, `1` `GARDEN PAVILION` | the drawing's title |
| `26` | paper | INSERT `TITLEBLOCK` | `10` (200,10) | the title block |
| `42` | in `TITLEBLOCK` | TEXT | `10` (10,10), `40` 5, `1` `SHEET 1 OF 2` | drawn at (210,20); record id `26/42` |
| `43` | in `TITLEBLOCK` | LINE | `10` (0,5), `11` (90,5) | the rule under it |
| `2A` | paper | VIEWPORT | `10` (150,120), `40/41` 200 x 120, `12` (50,25), `45` 60 | the model at 2 paper units per model unit |

What the fixture pins (`tests/export.rs`, `tests/sheets.rs`):

| Item | uncad |
|---|---|
| sheet rectangle | `layout_limits` `[-6.35, -6.35, 290.65, 203.65]`: 297 x 210 with the printable corner at the origin |
| `texts.json` | two records, `25` and `26/42`, each `space: "paper"`, `sheet: "Layout1"`, `tiles: []` and a `px` entry for `sheet:Layout1` |
| `strings.json` | `garden pavilion` -> `["25"]`, `sheet 1 of 2` -> `["26/42"]` |
| `manifest.counts` | `texts` 2, `texts_paper` 2 (before the paper-space index: 0 and no such field) |
| viewport mapping | `frame` `[50, 60, 250, 180]`, `scale` 2.0, `model_window` `[[0,-5],[100,-5],[100,55],[0,55]]`: `frame[0] + (x - model_window[0][0]) * scale` maps the model window onto the frame exactly |

The model-space LINE is what keeps the package honest: the crop, the
overview and the tiles are all of the model, so the sheet image is the only
picture the strings appear in -- and before 0.3.0's paper-space index they
appeared in no JSON at all, while `capabilities.text_boxes` read `none`.

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
5. **A rotated sheet with a plot origin** is not in the plot-origin
   fixture: how AutoCAD folds the offset into a rotated layout's limits is
   not known from the one rotated sample (its limits match neither the
   margins-only nor the `-(margin + origin)` placement), and the export
   takes the stored limits first for exactly that reason.

## Regenerating

```
python crates/uncad/tests/fixtures/make_fixtures.py                    # the shipped files (a name such as `mirrored-bulge`, `hatched-viewport` or `nested-attrib` writes only that one)
python crates/uncad/tests/fixtures/make_fixtures.py . dimlfac-minimal  # the ENTITIES-only draft (unbound *D1)
python crates/uncad/tests/fixtures/make_fixtures.py . viewport-minimal # the ENTITIES-only draft (model-space VIEWPORT)
python crates/uncad/tests/fixtures/make_fixtures.py . plot-origin      # one file (also angular-ordinate, radial, viewport-states, infinite-lines, title-block, cp949, mirrored, dimlfac, viewport, hidden)
```

The default mode is meant to reproduce the shipped bytes exactly; check with
`git status` after running it. Verify any regenerated file the same way the
shipped ones were: read it through `libredwg-sys` (`dxf_read_file`, then
`dwg_dynapi_entity_value` / `dwg_dynapi_entity_utf8text` per entity and the
`codepage` field of `Dwg_Data.header`; for the viewport fixture also the
LAYOUT and its embedded `plotsettings`, see that section) and through
`uncad <file> -o out.json --pretty`, and update the tables above and
`tests/fixtures.rs`.

## polyline_vertices_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: the `0` layer. ENTITIES: three
old-style POLYLINEs, each with its own VERTEX chain and SEQEND, with the VERTEX
records naming the POLYLINE in group 330 (the shape AutoCAD writes).

| handle | entity | group 70 | vertices |
|---|---|---|---|
| `30` | `POLYLINE_2D` (`AcDb2dPolyline`) | 1, closed | (0,0) (100,0) (100,100) (0,100) |
| `35` | `POLYLINE_2D` | 0 | (0,1000) with bulge (group 42) 1.0, (100,1000) |
| `39` | `POLYLINE_3D` (`AcDb3dPolyline`) | 8 | (0,0,0) (10,0,0) (10,10,0) (0,10,5) (0,0,5) |

Ground truth, all of it a property of the geometry above rather than of any reading of
it: the square's perimeter is 400 and its area 10000; a bulge of 1.0 is a half turn, so
handle `35` is a semicircle over a 100-unit chord, radius 50, length `pi * 50` =
157.0796327; handle `39`'s last vertex is the only one with both y = 0 and z = 5.

Before 0.3.0 all three came back one vertex short (3, 1 and 4), because LibreDWG's
`dwg_object_polyline_{2,3}d_get_points` end their `first_vertex .. last_vertex` walk
before the body reaches `last_vertex`. Handle `35` lost its arc entirely: the bulges came
from the subentity chain and so were 2 long against 1 vertex, and the length mismatch
cleared them.

## entity_truecolor_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: layers `0` (ACI 7) and `GREEN`
(ACI 3, `00ff00`). ENTITIES: four LINEs on `GREEN`, differing only in their colour
groups, written in DXF order (62 before 420).

| handle | groups | `color_index` | `true_color` | drawn |
|---|---|---|---|---|
| `30` | `420 65407` | 256 | `0x00ff7f` | `#00b259` (darkened for the white page) |
| `31` | `62 1` | 1 | none | `#ff0000` |
| `32` | `62 1`, `420 255` | 1 | `0x0000ff` | `#0000ff` |
| `33` | neither | 256 | none | `#00c300`, the layer's green darkened |

65407 is `0x00ff7f` and 255 is `0x0000ff`, both plain 24-bit values with no method byte,
which is what a DXF carries.

Before 0.3.0 handles `30` and `32` reported `true_color: null` and rendered in the
layer's colour and in ACI 1 respectively, while handle `31` reported
`true_color: 0xff0000` -- an RGB the file never wrote, synthesised by LibreDWG's DXF
reader from its own ACI palette.

## polyface_mesh_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: the `0` layer. ENTITIES: a LINE, a
polyface mesh and a polygon mesh. Every VERTEX record names the `*Model_Space` block
record (`330 1F`) rather than its POLYLINE -- what ezdxf and several exporters write, and
what `ezdxf.audit()` passes with 0 errors.

| handle | entity | contents |
|---|---|---|
| `30` | `LINE` | (0,0) to (1000,0) -- the control: it must survive whatever the meshes do |
| `31` | `POLYLINE_PFACE` (`AcDbPolyFaceMesh`, 70 = 64, 71 = 8, 72 = 2) | 8 `AcDbPolyFaceMeshVertex` records (a 10 x 10 square at z 0 and the same at z 10) and 2 `AcDbFaceRecord` records, `1 2 3 4` and `5 6 7 8` |
| `50` | `POLYLINE_MESH` (`AcDbPolygonMesh`, 70 = 16, 71 = 3, 72 = 4) | 12 `AcDbPolygonMeshVertex` records at `(i*10, j*5, 0)`, row-major over i in 0..3, j in 0..4 |

Ground truth from the definitions: two quad faces are 2 * 4 = 8 wireframe edges; an open
`m` by `n` grid is `n * (m - 1)` column edges plus `m * (n - 1)` row edges, so
4 * 2 + 3 * 3 = 17.

Before 0.3.0 this file did not parse at all -- `AcDbPolygonMeshVertex` matched no known
subclass, which is `DWG_ERR_INVALIDDWG` and so a whole-file refusal, taking the LINE with
it. With the polygon mesh removed, the polyface drew nothing (its vertices came back
typed `VERTEX_MESH`, which the wireframe walk did not look for) and its ten VERTEX records
were reported as top-level `Entity::Unknown` values.

## block_layer0_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: layers `0` (ACI 7), `RED` (ACI 1) and
`BLUE` (ACI 5), and BLOCK_RECORDs `*Model_Space` (`1F`) and `SYM` (`40`). BLOCKS: block
`SYM`. ENTITIES: one INSERT of `SYM` at (0,0) on layer `RED`, colour BYLAYER.

| handle | child of `SYM` | layer | group 62 | drawn |
|---|---|---|---|---|
| `42` | `LINE` (0,0)-(10,0) | 0 | 256 BYLAYER | `#ff0000`, the INSERT's layer |
| `43` | `LINE` (0,2)-(10,2) | 0 | 0 BYBLOCK | `#ff0000`, the INSERT's own resolved colour |
| `44` | `LINE` (0,4)-(10,4) | BLUE | 256 BYLAYER | `#0000ff` -- a named layer inside a block is used as stored |
| `45` | `TEXT` "L0" at (0,6) | 0 | BYLAYER | `#ff0000`, and its text record's layer is `RED` |

Layer 0's own ACI 7 is white, which this renderer draws black on a white page, so a child
that resolved against layer 0 instead of the reference is unmistakable. Before 0.3.0
handle `42` was `#000000`; handle `43` was already correct, since the BYBLOCK path
inherits the INSERT's resolved colour and that path was never broken.
