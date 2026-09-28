# Test fixtures

Hand-authored DXF files that pin what a reader must get out of a drawing: the
pre-R2007 code-page path, mirrored object coordinate systems and polyline bulges,
dimension points and values, viewports and layouts with their plot settings, layer
states, polyline vertices, meshes and colours. `tests/fixtures.rs` asserts what this
crate's model carries for each of them.

Every file is written from scratch by `make_fixtures.py` in this directory -- no
third-party drawing was copied, so they are redistributable under the repository's
GPL-3.0-or-later. They were authored on 2026-09-21..23 for the feature branch that is
being ported onto this repository (`archive/feat-0.3-readable`), where the renderer and
the export package of that time made assertions on them too; those assertions belong to
the renderer (`iron-render-cad`) and to the export crate now, and are not repeated here.

All are R2000 (`$ACADVER AC1015`) text DXF with CRLF line endings, between 470 and 4387
bytes. `.gitattributes` keeps their bytes (`-text`): the CP949 fixture is not UTF-8 by
design, and none of them may be normalised to LF.

Ground truth below is what each file states, worked out from its group codes.

| File | Bytes | Purpose |
|---|---|---|
| `cp949_r2000.dxf` | 855 | CP949 (`ANSI_949`) strings in TEXT, MTEXT and a LAYER name |
| `mirrored_ocs_r2000.dxf` | 925 | Closed LWPOLYLINE with extrusion (0,0,-1), an open one with a bulge, mirrored CIRCLE/ARC/TEXT |
| `mirrored_bulge_r2000.dxf` | 470 | The bulged outline of the mirrored fixture in an OCS with extrusion (0,0,-1), next to an ARC tracing the same arc |
| `dimlfac12_r2000.dxf` | 1304 | `$DIMLFAC 12.0` (header and STANDARD style) with a rotated DIMENSION whose measurement (group 42) is 10.0, its `*D1` block bound |
| `twisted_viewport_r2000.dxf` | 2253 | A paper-space VIEWPORT with `VIEWTWIST` 30 degrees and every AcDbViewport view field, plus a LAYOUT `Layout1` (A4 landscape, embedded plot settings) bound to `*Paper_Space` and to the VIEWPORT |
| `hidden_layers_r2000.dxf` | 1925 | 9 entities, 7 layers, 2 linetypes: one LINE per layer state (on, off, frozen, non-plotting, `Defpoints`, locked), an invisible LINE and a 0.50 mm DASHED one |
| `plot_origin_r2000.dxf` | 2430 | A LAYOUT `Layout1` in inches (ANSI B landscape, rotation 0) with asymmetric margins and a non-zero plot origin (DXF 46/47); a paper-space border LWPOLYLINE and a 1:5 plan VIEWPORT over a model LINE |
| `angular_ordinate_r2000.dxf` | 1491 | A 2-line angular DIMENSION (60 degrees) and an X- and a Y-type ordinate DIMENSION (30 and 50): the two kinds whose points and axis a DWG stores in other places than their DXF groups |
| `viewport_states_r2000.dxf` | 4387 | One VIEWPORT per state a sheet distinguishes (on; on a frozen layer; off; non-plan) plus two page setups: plot rotation 2 with asymmetric margins in mm, and plot rotation 3 in inches |
| `radial_r2000.dxf` | 1425 | A RADIUS, a DIAMETER and a 3-point angular DIMENSION, each with its circle or arc, no cached `*D` block |
| `hatched_viewport_r2000.dxf` | 3117 | The twisted-viewport fixture plus a pattern HATCH in model space (under the viewport) and one in paper space (outside its frame) |
| `nested_attrib_r2000.dxf` | 2174 | A block with an ATTDEF inserted inside another block, its ATTRIB value owned by the block record: the attribute of a *nested* block reference |
| `infinite_lines_r2000.dxf` | 673 | An XLINE and a RAY through the middle of a drawing 0.002 units across: the two entities with no end, at a scale where drawing them a fixed 1e6 units long panicked a rasterizer |
| `polyline_vertices_r2000.dxf` | 2322 | A closed 4-vertex `POLYLINE_2D`, a 2-vertex one with bulge 1.0 and a 5-vertex `POLYLINE_3D`, every vertex to be kept, the last included |
| `entity_truecolor_r2000.dxf` | 795 | One LINE per way a DXF can state a colour (420 alone, 62 alone, 62 then 420, neither) on a layer whose own ACI is 3 |
| `polyface_mesh_r2000.dxf` | 3921 | A `POLYLINE_PFACE` and a `POLYLINE_MESH` whose VERTEX records name the block record as their owner, next to a LINE |
| `block_layer0_r2000.dxf` | 1295 | A block drawn on layer 0 inserted on a coloured layer: AutoCAD's "layer 0 in a block means the layer of the reference" rule |
| `title_block_r2000.dxf` | 2939 | A drawing whose every string is in *paper* space: a title TEXT and a title-block INSERT on an A4 sheet over a model with no text at all |

## cp949_r2000.dxf

HEADER: `$DWGCODEPAGE ANSI_949`, `$INSUNITS 4`, `$MEASUREMENT 1`, `$LUNITS 2`,
`$DIMLFAC 1.0`. TABLES: a LAYER table with `0` and a layer whose name is the CP949 bytes
of "U+BCBD U+CCB4". ENTITIES: four TEXT and one MTEXT, all at height 2.5.

The header reads as version `r2000` with codepage **40** (`ANSI_949`).

| Handle | Entity | Layer | Raw bytes (CP949) | Meaning (UTF-8) |
|---|---|---|---|---|
| 23 | TEXT at (0,0) | U+BCBD U+CCB4 | `B5 B5 B8 E9` | `U+B3C4 U+BA74` |
| 24 | TEXT at (0,5) | 0 | `A1 BE 33` | `±3` |
| 25 | TEXT at (0,10) | 0 | `33 32 2E 35 A7 B3` | `32.5U+33A1` |
| 26 | MTEXT at (0,20), width 50 | 0 | `B9 E6 20 31 30 31 5C 50 B8 E9 C0 FB 20 33 32 2E 35 A7 B3` | `U+BC29 101\PU+BA74 U+C801 32.5U+33A1` (`\P` literal) |
| 27 | TEXT at (0,15) | 0 | `50 4C 41 49 4E` | `PLAIN` |
| 22 | LAYER (color 1) | | `BA AE C3 BC` | `U+BCBD U+CCB4` |

Notes:

- `±` is `A1 BE` in CP949 (KS X 1001 row 1, PLUS-MINUS SIGN); `A1 B1` would be the right
  double quotation mark. Python's `cp949` codec is the authority and the file carries
  `A1 BE`. Read as CP1252 the pair is `¡¾`, read as UTF-8 it is two U+FFFD.
- The layer name is the worst case for a lossy UTF-8 reading: `C3 BC` happens to be
  valid UTF-8 for U+00FC, so such a reading invents a wrong character (`��ü`) rather
  than only replacing bytes.

## mirrored_ocs_r2000.dxf

HEADER: `$INSUNITS 4` only, so the header's codepage is 30 (`ANSI_1252`), the default
before R2007. No TABLES section: no entity's layer resolves.

| Handle | Entity | DXF groups |
|---|---|---|
| 20 | LWPOLYLINE | `70 = 1`, 4 vertices (0,0) (100,0) (100,50) (0,50), `210/220/230 = 0,0,-1` |
| 21 | LWPOLYLINE | `70 = 0`, same vertices, `42 = 0.41421356` after the second vertex, `210/220/230 = 0,0,1` |
| 22 | CIRCLE | centre (10,10) r 5, `210/220/230 = 0,0,-1` |
| 23 | ARC | centre (0,0) r 20, 0 to 90 degrees (0 and 1.5707963267948966 rad), `210/220/230 = 0,0,-1` |
| 24 | TEXT | "MIRROR" at (10,10) h 2.5, `210/220/230 = 0,0,-1` |
| 25 | LINE | (-5,-5) to (5,5), no extrusion |

The coordinates are stated in each entity's object coordinate system. With extrusion
(0,0,-1) that system's x axis points the other way, so in the world the circle's centre
is (-10,10), the arc runs clockwise from 90 to 180 degrees and the text starts at
(-10,10); taking them there is the consumer's step, the model keeps what the file states.

The closed flag is bit 1 of group 70: handle 20 is closed, handle 21 open. A closed
polyline *without* an extrusion is not in this file; `tests/polyline_closed.rs` covers
that case with the corpus's `example_2000.dwg`/`.dxf` pair.

## mirrored_bulge_r2000.dxf

HEADER: `$INSUNITS 4` only; no TABLES section. The OCS-to-world map of extrusion
(0,0,-1) is the reflection `x -> -x`, which reverses every arc's turning direction: a
consumer that takes the polyline to the world negates its bulges together with the x of
its vertices, and the ARC next to it is the same arc stated the way an ARC states it.

| Handle | Entity | DXF groups |
|---|---|---|
| 20 | LWPOLYLINE | `70 = 0`, vertices (0,0) (100,0) (100,50) (0,50), `42 = 0.41421356` after the second vertex, `210/220/230 = 0,0,-1` |
| 21 | ARC | centre (75,25) r 35.35533906, 315 to 45 degrees, `210/220/230 = 0,0,-1` |

In the world both are the clockwise 90-degree arc from (-100,0) to (-100,50) about
(-75,25), whose apex is (-110.355,25).

## hidden_layers_r2000.dxf

HEADER: `$INSUNITS 4`. TABLES: an LTYPE table (`Continuous`, `DASHED`) and a LAYER table:

| Layer | DXF 62 | DXF 70 | DXF 290 | State |
|---|---|---|---|---|
| `0` | 7 | 0 | omitted | on |
| `VISIBLE` | 1 | 0 | omitted | on |
| `OFF` | -3 | 0 | omitted | off (negative colour) |
| `FROZEN` | 4 | 1 | omitted | frozen |
| `NOPLOT` | 5 | 0 | 0 | not plotted |
| `Defpoints` | 7 | 0 | 0 | not plotted |
| `LOCKED` | 6 | 4 | omitted | locked, still drawn |

ENTITIES: one LINE per layer, in table order, from `(0, 10 i)` to `(100, 10 i)`; then on
`VISIBLE` a LINE with `60 = 1` (invisible) at y = 100 and a LINE with `6 DASHED`,
`370 = 50` (0.50 mm) and `48 = 2.0` at y = 110.

A LAYER's group 70 means 1 frozen, 2 frozen in new viewports and 4 locked; "off" is the
negative colour. A layer with no 290 does not say whether it is plotted.

## dimlfac12_r2000.dxf

HEADER: `$INSUNITS 4`, `$DIMLFAC 12.0`, `$DIMDEC 2`, `$DIMLUNIT 2`, `$DIMSCALE 1.0`.
TABLES: a BLOCK_RECORD table with `*Model_Space` (handle `1F`) and `*D1` (handle `40`),
plus DIMSTYLE `STANDARD` with `144 = 12.0` (DIMLFAC). BLOCKS: a BLOCK `*D1` (flag 1,
anonymous, `330 = 40`) containing a TEXT "120" at (5,6), height 0.18 -- the label 12 x 10
under DIMLFAC 12. ENTITIES: a LINE (0,0)-(10,0) and a DIMENSION with `100 AcDbDimension`, `2 *D1`, `10 = (10,5)` dimension-line point, `11 = (5,6)` text
midpoint, `70 = 32`, `1` empty, `42 = 10.0`, `3 STANDARD`, `100 AcDbAlignedDimension`,
`13 = (0,0)`, `14 = (10,0)`, `50 = 0.0`, `100 AcDbRotatedDimension`.

So the DIMENSION is a linear (rotated) one measuring **10.0**, with no text override, its
block `*D1` and its style `STANDARD`. The file binds the `*D1` BLOCK to its BLOCK_RECORD
through the BLOCK's `330` owner, as an R13-or-later DXF does; `make_fixtures.py .
dimlfac-minimal` writes an ENTITIES-only draft without the BLOCK_RECORD table.

## twisted_viewport_r2000.dxf

HEADER: `$INSUNITS 4`. TABLES: a BLOCK_RECORD table with `*Model_Space` (`1F`) and
`*Paper_Space` (`1C`, with `340 = 2B` naming its LAYOUT). BLOCKS: both blocks, empty.
ENTITIES: a LINE (0,0)-(100,50) owned by `*Model_Space` and a VIEWPORT owned by
`*Paper_Space` (`330 = 1C`) with handle `5 = 2A`, `67 = 1`, `100 AcDbViewport`,
`10 = (150,100,0)`, `40 = 200`, `41 = 120`, `68 = 1`, `69 = 2`, `12 = (50,25)`,
`13 = (0,0)`, `14 = (10,10)`, `15 = (10,10)`, `16 = (0,0,1)`, `17 = (0,0,0)`, `42 = 50`,
`43 = 0`, `44 = 0`, `45 = 60`, `50 = 0`, `51 = 30.0`, `72 = 100`, `90 = 32864`. OBJECTS:
the named object dictionary (`5 = C`, `330 = 0`, `281 = 1`, one entry `3 ACAD_LAYOUT` /
`350 1A`), the `ACAD_LAYOUT` dictionary (`5 = 1A`, `330 = C`, entry `3 Layout1` /
`350 2B`) and one LAYOUT (`5 = 2B`, `330 = 1A`):

- `100 AcDbPlotSettings`: `1` empty (page setup name), `2 none_device` (printer),
  `4 ISO_A4_(210.00_x_297.00_MM)` (canonical media name), `40`-`43 = 6.35` (margins,
  mm), `44 = 210.0`, `45 = 297.0` (the unrotated sheet, mm), `46`-`49`, `140`,
  `141 = 0.0` (plot origin and window), `142 = 1.0`, `143 = 1.0` (paper : drawing
  units), `70 = 688` (plot flags), `72 = 1` (mm), `73 = 1` (rotated 90 degrees
  counter-clockwise: landscape), `74 = 5` (plot the layout), `7` empty (style sheet),
  `75 = 16` (1:1), `147 = 1.0`, `148`, `149 = 0.0` (paper image origin).
- `100 AcDbLayout`: `1 Layout1`, `70 = 1`, `71 = 1` (tab order), `10 = (-6.35,-6.35)`,
  `11 = (290.65,203.65)` (LIMMIN/LIMMAX: the printable area of the rotated sheet),
  `12 = (0,0,0)`, `14 = (1e20,1e20,1e20)`, `15 = (-1e20,-1e20,-1e20)` (the "never
  computed" extents sentinels AutoCAD writes for a layout that has not been plotted or
  zoomed), `146 = 0.0`, `13 = (0,0,0)`, `16 = (1,0,0)`, `17 = (0,1,0)`, `76 = 0`,
  `330 = 1C` (block record), `331 = 2A` (active viewport).

Deliberately absent: a `Model` LAYOUT, a plot view (`6`), reactors and extension
dictionaries, `345`/`346` UCS handles, the R2004+ shade-plot groups.

`VIEWSIZE` 60 is 60 drawing units in a 120-unit frame, scale 2, and the twist of 30
degrees is 0.5235987755982988 rad. The file carries BLOCKS, so the `*Paper_Space` the
VIEWPORT's `330` names is a defined block; `make_fixtures.py . viewport-minimal` writes an
ENTITIES-only draft without them.

The LAYOUT's two `330`s are different things: the first (before the `100` markers) is its
owner, the `ACAD_LAYOUT` dictionary; the second, inside `AcDbLayout`, is its block record
`*Paper_Space`, whose own `340` names the LAYOUT back. `331` is the active VIEWPORT. The
extents are the sentinels exactly as written, a plot view name (`6`) is not written, and a
layout's viewport list has no DXF group. New handles in this file must avoid the ones it
uses: `C`, `1A`, `1C`, `1F`, `20`-`24`, `2A` and `2B`.

## plot_origin_r2000.dxf

The same skeleton as the viewport fixture (`*Model_Space` 1F, `*Paper_Space` 1C with
`340 = 2B`, BLOCK/ENDBLK 20-23, the named object dictionary `C`, `ACAD_LAYOUT` `1A`,
LAYOUT `2B`, VIEWPORT `2A`), `$INSUNITS 1` (inches), and the page setup AutoCAD-written
drawings usually carry: `72 = 0` (inch paper units), `73 = 0` (no rotation), ANSI B
`44/45 = 431.8 x 279.4` mm (17 x 11 in), margins `40..43 = 6.35 / 19.05 / 6.35 / 19.05`
mm (0.25 / 0.75 / 0.25 / 0.75 in) and a plot origin `46/47 = -6.35 / -12.7` mm (-0.25 /
-0.5 in). AutoCAD places the layout origin at the printable corner moved by the plot
origin, so the paper runs from `-(margin + origin)` = `(-(0.25 - 0.25), -(0.75 - 0.5))` =
`(0, -0.25)` to `(17, 10.75)` in, and that is what the file's `LIMMIN/LIMMAX` (10/11)
say. The rule is ezdxf 1.4.4's `reset_paper_limits`; it was checked against the seven
AutoCAD-written sample layouts (`-(margin + origin)` equals the stored limits within
0.005 in on the six with rotation 0; the rotated one keeps limits matching neither
formula, which is why a consumer should trust the stored limits first).

ENTITIES: a model LINE `24` (0,0) -> (100,50) owned by 1F; on paper (owner 1C, `67 = 1`)
a closed border LWPOLYLINE `25` (0.5, 0.25) .. (16.5, 10.5) -- its top edge lies above the
10.25 in a margins-only sheet would end at -- and the VIEWPORT `2A`: centre (8.5, 5.5),
12 x 8 in, `VIEWCTR` (50, 25), `VIEWSIZE` 40 (scale 8 / 40 = 1:5), no twist, `68 = 1`,
`69 = 2`, `90 = 32864`.

So the LAYOUT states `LIMMIN (0, -0.25)`, `LIMMAX (17, 10.75)`, margins
6.35/19.05/6.35/19.05, paper 431.8 x 279.4, plot origin (-6.35, -12.7), paper unit 0
(inches) and rotation 0, and the VIEWPORT is in paper space.

## angular_ordinate_r2000.dxf

HEADER: `$INSUNITS 4`, `$DIMDEC 2`, `$DIMADEC 0`, `$DIMLUNIT 2`. TABLES: a LAYER table
and DIMSTYLE `STANDARD` (handle 30). ENTITIES: two LINEs (0,0) -> (10,0) and (0,0) ->
(5, 8.660254), then three DIMENSIONs with no cached `*D` blocks, every number derived by
hand:

| Handle | Subclass | 70 | 10 | 13 | 14 | 15 | 16 | 42 | Value |
|---|---|---|---|---|---|---|---|---|---|
| 33 | `AcDb2LineAngularDimension` | 34 | (5, 8.660254) = line 2's end | (0,0) | (10,0) | (0,0) | (4.330127, 2.5) = arc point, 30 degrees along r = 5 | pi/3 | 60 degrees |
| 34 | `AcDbOrdinateDimension` | 102 (bit 64: X type) | (100, 200) datum | (130, 250) feature | (130, 270) leader | | | 30.0 | 130 - 100 = 30 |
| 35 | `AcDbOrdinateDimension` | 38 (Y type) | (100, 200) | (130, 250) | (150, 250) | | | 50.0 | 250 - 200 = 50 |

What the file pins: each point lands in the field of the group it is written in -- the
2-line angular dimension's arc point is group 16 and its last point group 10. Read
swapped, the arc point would sit at 60 degrees between rays at 30 and 180 degrees and the
dimension would measure 150. The ordinate's type is bit 64 of group 70 (a DWG from R13 on
states it elsewhere, as bit 1 of a separate byte); read from another bit both ordinates
would be Y type.

## hatched_viewport_r2000.dxf

`twisted_viewport_r2000.dxf` in its full form (the same HEADER, TABLES, BLOCKS, LINE
`24`, VIEWPORT `2A` and OBJECTS section, so everything that section says holds here) plus
one pattern HATCH per space, written by `make_fixtures.py`'s `hatch` helper: a
user-defined pattern (`76 = 0`, `2 USER`) of one definition line (`78 = 1`) with no
dashes, one closed polyline boundary (`92 = 3`, `73 = 1`), style outermost (`75 = 1`),
and a seed point one unit inside the first vertex.

| Handle | Entity | Space | Boundary | Definition line (53 / 45,46) |
|---|---|---|---|---|
| 30 | HATCH | model (`330 = 1F`) | (20,10) (60,10) (60,30) (20,30) | 90 degrees, offset (-2, 0): vertical lines 2 units apart |
| 31 | HATCH | paper (`330 = 1C`, `67 = 1`) | (10,10) (40,10) (40,30) (10,30) | 0 degrees, offset (0, 4): horizontal lines 4 units apart |

The paper hatch lies outside the viewport's 200 x 120 frame (x 50..250, y 40..160 on the
sheet): a sheet that composites the model through the viewport has to keep the two
patterns apart. Objects read: the 16 of the viewport fixture plus the two HATCHes.

## nested_attrib_r2000.dxf

The tag-inside-assembly pattern: an attributed block inside another block. HEADER:
`$INSUNITS 4`. TABLES: BLOCK_RECORDs `1F` (`*Model_Space`), `40` (`TAG`) and `50`
(`DOOR`). BLOCKS: `TAG` = LINE `42` (0,0)-(10,0) and ATTDEF `43` (tag `NUM`, default
`D-000`, height 2.5); `DOOR` = LINEs `52` and `53` plus INSERT `55` of `TAG` at (20, 20)
(`66 = 1`) followed by its ATTRIB `56` (`NUM` = `D-101`, at (21, 21)) and a SEQEND --
the ATTRIB owned by the **block record** (`330 = 50`), the shape ezdxf- and
AutoCAD-written DXFs use. ENTITIES: INSERT `60` of `DOOR` at (100, 100) and INSERT `61`
of `TAG` at (0, 0) with its own ATTRIB `62` (`NUM` = `D-TOP`).

The INSERT inside the block definition (`55`) has its attribute as the record that
follows it in the block, owned by the block record; the INSERT in ENTITIES (`61`) has its
own. Either way the ATTRIB belongs to the INSERT before it.

## viewport_states_r2000.dxf

The viewport fixture's skeleton with a second paper layout and four viewports, one per
state a sheet distinguishes. TABLES: LAYERs `0` (colour 7) and `VPFROZEN` (colour 4,
`70 = 1`: frozen, the usual way to hide a viewport's border); BLOCK_RECORDs
`*Model_Space` (`1F`), `*Paper_Space` (`1C`, `340 = 2B`) and `*Paper_Space0` (`1D`,
`340 = 2C`). BLOCKS: all three (`20`/`21`, `22`/`23`, `26`/`27`), empty. ENTITIES: the
model LINE `24` (0,0) -> (100,50) of the other viewport fixtures, then four VIEWPORTs
owned by `1C` (`67 = 1`), each a 60 x 40 frame showing the model window 30 x 20 about
(50,25) -- `12 = (50,25)`, `45 = 20`, so the scale is 40 / 20 = 2 -- with `69` counting
up from 2 so none is the overall frame:

| Handle | `10` centre | Layer | `68` | `90` | `16` VIEWDIR |
|---|---|---|---|---|---|
| `2A` | (50, 50) | `0` | 1 | 32864 | (0,0,1) |
| `2D` | (50, 120) | `VPFROZEN` | 1 | 32864 | (0,0,1) |
| `2E` | (50, 190) | `0` | **0** | **163936** (`32864 \| 0x20000`) | (0,0,1) |
| `2F` | (140, 50) | `0` | 1 | 32864 | **(1,1,1)** |

A frozen layer hides `2D`'s border and not the window; `2E` is switched off; `2F` is not
a plan view.

OBJECTS: the named object dictionary (`C`), the `ACAD_LAYOUT` dictionary (`1A`) with
both entries, and two LAYOUTs:

- `2B` `Layout1`, tab 1, block `1C`, active viewport `2A`: ISO A4 `44/45 = 210 x 297` mm,
  `73 = 2` (upside down, so the sheet keeps its portrait size), `72 = 1` (mm), margins
  `40..43 = 10 / 20 / 5 / 15` mm, no plot origin. The sheet therefore runs from
  `-(left, bottom)` = (-10, -20) to (210 - 10, 297 - 20) = (200, 277) mm, which is what
  `10`/`11` (LIMMIN/LIMMAX) say.
- `2C` `Layout2`, tab 2, block `1D` (empty), no active viewport: ANSI B
  `44/45 = 431.8 x 279.4` mm, `73 = 3` (90 degrees clockwise, so the landscape sheet
  becomes portrait 279.4 x 431.8), `72 = 0` (**inches**), margins 6.35 mm all round, no
  plot origin: (-0.25, -0.25) to (273.05 / 25.4, 425.45 / 25.4) = (10.75, 16.75) in.

New handles avoid the ones the file uses: `C`, `1A`, `1C`, `1D`, `1F`, `20`-`24`, `26`,
`27` and `2A`-`2F`.

## radial_r2000.dxf

Three dimension kinds with every value worked out by hand: a radius, a diameter and a
3-point angular dimension.

HEADER: `$INSUNITS 4`, `$DIMDEC 2`, `$DIMADEC 0`, `$DIMLUNIT 2`. TABLES: a LAYER table
and DIMSTYLE `STANDARD` (handle 30). ENTITIES: the circle or arc each dimension
annotates, then the three DIMENSIONs with no cached `*D` blocks, every number derived by
hand:

| Handle | `70` | Subclass | `10` | `13` | `14` | `15` | `42` | Value |
|---|---|---|---|---|---|---|---|---|
| 34 | 36 (4 radius) | `AcDbRadialDimension` | (0,0) centre | | | (3,4) on the circle | 5.0 | 3-4-5: radius 5 |
| 35 | 35 (3 diameter) | `AcDbDiametricDimension` | (20,0) chord start | | | (20,10) chord end | 10.0 | the vertical diameter of the r = 5 circle at (20,5) |
| 36 | 37 (5 angular 3-point) | `AcDb3PointAngularDimension` | (44.330127, 2.5) arc point | (50,0) | (45, 8.660254) | (40,0) centre | pi/3 | the 60-degree sector |

The arc point of the angular dimension lies at 30 degrees along a radius of 5 -- inside
the 60-degree sector between the rays at 0 and 60 degrees, not the 300-degree one on the
other side.

## infinite_lines_r2000.dxf

The two entities that have no end, at a scale that makes the difference between "long"
and "as far as the picture goes" fatal to a renderer. HEADER: `$INSUNITS 4`. TABLES: a
LAYER table with `0` only. ENTITIES, all in model space:

| Handle | Entity | Groups |
|---|---|---|
| `30` | LINE | `10` (0,0), `11` (0.002, 0.002): the only entity with a size |
| `31` | TEXT | `10` (0.0013, 0.0002), `40` 0.0006, `1` `X` |
| `32` | XLINE (`AcDbXline`) | `10` (0.001, 0.001), `11` (1, 0): a horizontal line through the middle |
| `33` | RAY (`AcDbRay`) | `10` (0.001, 0.001), `11` (0, 1): a vertical line from the middle upwards, nothing below |

At 1568 px across, this drawing is about 750 thousand pixels per drawing unit, so a
construction line drawn as a segment 1e6 units long ends 7e11 px off the canvas -- which
tiny-skia's scan converter does not survive at every size.

## title_block_r2000.dxf

The drawing whose name is not in the drawing. HEADER: `$INSUNITS 4`. TABLES: a LAYER
table with `0`, and BLOCK_RECORDs `1F` `*Model_Space`, `1C` `*Paper_Space` (LAYOUT `2B`)
and `40` `TITLEBLOCK`. BLOCKS: the two spaces plus `TITLEBLOCK`. OBJECTS: the named
object dictionary, `ACAD_LAYOUT` and one LAYOUT `Layout1`, A4 landscape
(`ISO_A4_(210.00_x_297.00_MM)`, rotation 1, 6.35 mm margins, no plot origin).

| Handle | Space | Entity | Groups |
|---|---|---|---|
| `24` | model | LINE | `10` (0,0), `11` (100,50): the whole model, no text anywhere in it |
| `25` | paper | TEXT | `10` (150,20), `40` 8, `1` `GARDEN PAVILION`: the drawing's title |
| `26` | paper | INSERT `TITLEBLOCK` | `10` (200,10) |
| `42` | in `TITLEBLOCK` | TEXT | `10` (10,10), `40` 5, `1` `SHEET 1 OF 2` |
| `43` | in `TITLEBLOCK` | LINE | `10` (0,5), `11` (90,5) |
| `2A` | paper | VIEWPORT | `10` (150,120), `40/41` 200 x 120, `12` (50,25), `45` 60: the model at 2 paper units per model unit |

## polyline_vertices_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: the `0` layer. ENTITIES: three
old-style POLYLINEs, each with its own VERTEX chain and SEQEND, with the VERTEX records
naming the POLYLINE in group 330 (the shape AutoCAD writes).

| handle | entity | group 70 | vertices |
|---|---|---|---|
| `30` | `POLYLINE_2D` (`AcDb2dPolyline`) | 1, closed | (0,0) (100,0) (100,100) (0,100) |
| `35` | `POLYLINE_2D` | 0 | (0,1000) with bulge (group 42) 1.0, (100,1000) |
| `39` | `POLYLINE_3D` (`AcDb3dPolyline`) | 8 | (0,0,0) (10,0,0) (10,10,0) (0,10,5) (0,0,5) |

Ground truth, all of it a property of the geometry above rather than of any reading of
it: the square's perimeter is 400 and its area 10000; a bulge of 1.0 is a half turn, so
handle `35` is a semicircle over a 100-unit chord, radius 50, length `pi * 50` =
157.0796327; handle `39`'s last vertex is the only one with both y = 0 and z = 5.

A reading one vertex short (3, 1 and 4) is the failure this file is for.

## entity_truecolor_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: layers `0` (ACI 7) and `GREEN` (ACI 3,
`00ff00`). ENTITIES: four LINEs on `GREEN`, differing only in their colour groups,
written in DXF order (62 before 420).

| handle | groups | colour index | true colour |
|---|---|---|---|
| `30` | `420 65407` | 256 (BYLAYER) | `0x00ff7f` |
| `31` | `62 1` | 1 | none |
| `32` | `62 1`, `420 255` | 1 | `0x0000ff` |
| `33` | neither | 256 (BYLAYER) | none |

65407 is `0x00ff7f` and 255 is `0x0000ff`, both plain 24-bit values with no method byte,
which is what a DXF carries. A plain group 62 states no RGB.

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

The LINE is read whatever the meshes do, and each VERTEX belongs to the POLYLINE before
it, whatever owner its `330` names.

## block_layer0_r2000.dxf

HEADER: `$ACADVER AC1015`, `$INSUNITS 4`. TABLES: layers `0` (ACI 7), `RED` (ACI 1) and
`BLUE` (ACI 5), and BLOCK_RECORDs `*Model_Space` (`1F`) and `SYM` (`40`). BLOCKS: block
`SYM`. ENTITIES: one INSERT of `SYM` at (0,0) on layer `RED`, colour BYLAYER.

| handle | child of `SYM` | layer | group 62 | colour it is drawn in |
|---|---|---|---|---|
| `42` | `LINE` (0,0)-(10,0) | 0 | 256 BYLAYER | red, the INSERT's layer |
| `43` | `LINE` (0,2)-(10,2) | 0 | 0 BYBLOCK | red, the INSERT's own resolved colour |
| `44` | `LINE` (0,4)-(10,4) | BLUE | 256 BYLAYER | blue -- a named layer inside a block is used as stored |
| `45` | `TEXT` "L0" at (0,6) | 0 | BYLAYER | red |

Layer 0's own ACI 7 is white (drawn black on a white page), so a child that resolved
against layer 0 instead of the reference is unmistakable. The rule is a property of the
reference: the file, and the model, keep the children on layer 0.

## What did not work

1. **`cp949_r2000.dwg` via LibreDWG's add/write API**: `dwg_add_Document`,
   `dwg_add_TEXT`, `dwg_add_MTEXT`, `dwg_add_LINE`, `dwg_add_LWPOLYLINE`, `dwg_add_LAYER`
   and `dwg_write_file` are compiled into the static library (`config.h` defines
   `USE_WRITE`; only the bindgen allowlist omits them). A scratch program that sets
   `header.version = R_2000` and `header.codepage = 40` before `dwg_add_Document`, adds
   the same strings as the DXF fixture and calls `dwg_write_file` does produce an R2000
   DWG that reads back with `codepage = 40` -- but LibreDWG's UTF-8 -> code-page encoder
   mangles every non-ASCII string on the way in (the layer came back as `;`, "±3" as
   `1B 33`, "32.5U+33A1" as `32.5{`, with `Warning: utf-8: BAD_CONTINUATION_BYTE` at write
   time), and setting `$DWGCODEPAGE` through `dwg_dynapi_header_set_value` crashed. So no
   DWG is shipped: the code-page fixture is DXF-only. An AutoCAD-written Korean R2000 DWG
   is still wanted as a fixture.
2. **A closed LWPOLYLINE without extrusion** in the mirrored fixture is not in the file;
   `tests/polyline_closed.rs` covers it with the corpus's `example_2000.dwg`/`.dxf` pair
   instead.
3. **`±` as `A1 B1`**: not a valid encoding of the character; the file uses `A1 BE`.
4. **A `Model` LAYOUT, a plot view (`6`) and a page-setup name** are not in the viewport
   fixture: none of them is needed, and a layout's viewport list has no DXF group.
5. **A rotated sheet with a plot origin** is not in the plot-origin fixture: how AutoCAD
   folds the offset into a rotated layout's limits is not known from the one rotated
   sample (its limits match neither the margins-only nor the `-(margin + origin)`
   placement).

## Regenerating

```
python crates/uncad/tests/fixtures/make_fixtures.py                    # the shipped files (a name such as `mirrored-bulge`, `hatched-viewport` or `nested-attrib` writes only that one)
python crates/uncad/tests/fixtures/make_fixtures.py . dimlfac-minimal  # the ENTITIES-only draft (unbound *D1)
python crates/uncad/tests/fixtures/make_fixtures.py . viewport-minimal # the ENTITIES-only draft (model-space VIEWPORT)
python crates/uncad/tests/fixtures/make_fixtures.py . plot-origin      # one file (also angular-ordinate, radial, viewport-states, infinite-lines, title-block, cp949, mirrored, dimlfac, viewport, hidden)
```

The default mode reproduces the shipped bytes exactly; check with `git status` after
running it. Verify a regenerated file by reading it with `uncad <file> -o out.json
--pretty` against the ground truth above, and update the tables above and
`tests/fixtures.rs`.
