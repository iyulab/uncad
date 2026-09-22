# Known limitations and caveats

## Entity type coverage

`parse()`/`to_svg()` support: LINE, CIRCLE, ARC, ELLIPSE, LWPOLYLINE, TEXT, POINT, SOLID,
RAY, XLINE, INSERT (including recursive block-reference rendering), ATTRIB, ATTDEF,
VIEWPORT, 3DFACE, SPLINE, MTEXT, POLYLINE_3D, POLYLINE_2D, DIMENSION, HATCH, 3DSOLID,
LEADER, MULTILEADER, MLINE, REGION, POLYLINE_PFACE, POLYLINE_MESH, TOLERANCE, ACAD_TABLE,
WIPEOUT and LIGHT. Details worth knowing:

- **DIMENSION** folds all 8 subtypes (ALIGNED, ANG2LN, ANG3PT, DIAMETER, LINEAR, ORDINATE,
  RADIUS, ARC_DIMENSION) into one type. They share the `DIMENSION_COMMON` layout, including
  the `block` handle to the cached-geometry block that is what actually gets drawn. The
  subtype survives as `DimensionEntity::geometry` with its own definition points
  (`LINEAR`, `ALIGNED`, `ANGULAR_3POINT`, `ANGULAR_2LINE`, `RADIUS`, `DIAMETER`,
  `ORDINATE`, `ARC_LENGTH`, and `UNKNOWN` when the points cannot be read).
- **HATCH** handles boundary paths that are polylines as well as lists of
  line/arc/ellipse/spline edges. Pattern fills are really reproduced, by reading
  `Dwg_HATCH_DefLine` and tiling an SVG `<pattern>` (see "HATCH pattern fill" below);
  solid fills are painted as a translucent color; gradient fills are approximated with
  SVG's `linearGradient`/`radialGradient` (unverified, see "HATCH gradient fill").
- **ELLIPSE** draws the arc its stored sweep describes, as an SVG elliptical arc path
  (since 0.3.0 -- before it, every elliptical arc was closed into a full oval). DXF 41/42
  are *parameters*, not angles: the point is `center + major cos t + minor sin t`, so on a
  flattened ellipse the parameter runs ahead of the polar angle everywhere but on the
  axes. The extent is the arc's own, not the whole ellipse's. The model does not keep
  ELLIPSE's extrusion, so an ellipse whose plane normal points away from +Z has its arc
  drawn mirrored about the major axis (the closed ones are unaffected).
- **LEADER** draws only the polyline through its vertices plus an optional arrowhead at
  the first one. Spline paths and text-box size are not in the model, because nothing
  renders them.
- **POLYLINE_2D** reuses `LwPolylineEntity` and renders through exactly the same code path
  as LWPOLYLINE, the same way `Entity::XLine` reuses `RayEntity`.
- **3DSOLID**, **REGION**, **POLYLINE_PFACE** and **POLYLINE_MESH** render as wireframes -- the edges, not a
  reading of the B-rep. A body that is *flat in the XY plane* is drawn in that plane,
  exactly where the file puts it, which is the usual case for a REGION (it is built from a
  closed 2D profile). A body with real depth is drawn through a fixed 30-degree isometric
  projection, and that **moves it**: `(x, y, z) -> ((x - z) cos30, y + (x + z) sin30)`
  scales x and shears y, so such a body's drawn position, size and shape -- and the `bbox`,
  `tiles` and `px` its package record carries, which are measured from what is drawn -- are
  in projection space, not world space. The record says `confidence: "estimated"`. Until
  0.3.0 *every* body went through the projection, so a flat axis-aligned REGION came out as
  a parallelogram of the wrong size, well away from the rest of the drawing. A solid whose
  ACIS data cannot be read or converted is reported as unsupported.
- **POLYLINE_MESH** ("polygon mesh", since 0.3.0) draws the grid lines of its `m` by `n`
  vertex grid, wrapped where group 70's bit 1 (closed in M) or bit 32 (closed in N) says
  so. It draws nothing unless exactly `m * n` vertices were found: a smooth-surface mesh
  (`curve_type` non-zero) stores spline control points alongside the approximated ones,
  and a grid guessed from counts that do not add up would be a lie. The surface itself is
  not shaded or hidden-line-removed -- same approximation as the other wireframes.
- **MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE, WIPEOUT, LIGHT**
  are all **experimental** -- see the next section.

**Permanently not supported: ACAD_PROXY_ENTITY.** It is the proxy representation of a
custom entity from another program, so it has no fixed geometry to render at all: just
`proxy_id`, `class_id` and serialized entity bytes, with no coordinates or shape. Leaving
it as `Unknown` *is* the accurate representation, and "supporting" it would change nothing
in practice -- `Unknown` preserves the real DXF name, so a CLI summary already counts it
correctly as `ACAD_PROXY_ENTITY`.

**Not converted yet, although they do carry geometry:** MINSERT (a block reference
repeated on a row/column grid), TRACE (a filled quadrilateral, the same shape as SOLID),
SHAPE, BODY and OLEFRAME/OLE2FRAME. They are
read and counted, but nothing is drawn for them, so a drawing whose grid of columns is one
MINSERT loses that grid from the picture. They are scheduled for a later release (see the
roadmap in `docs/VLM_EXPORT_DESIGN.md`).

Nothing is dropped silently either way: at the `parse()` stage such a type becomes
`Entity::Unknown` carrying its real DXF name, at `to_svg()` it is listed in
`unsupported_types`, the CLI prints it ("left out of the image, unsupported entity types:
..."), and `export` records it in `report.json`.

### Eight types are experimental

(MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE, WIPEOUT, LIGHT)

None of these has been exercised by a real drawing. The spot-check set used during
development -- 9 independent AutoCAD drawings -- contains no instance of any of them, so
each was verified only by a clean `cargo build`/`cargo test`/`cargo clippy` on both MSVC
and Linux (Docker) plus a direct reading of the real `dwg.h`/`dynapi.c` layouts. Scope and
risk per type:

**MULTILEADER** renders only the leader-line geometry (`ctx.leaders[].lines[].points`),
with spline leaders chord-approximated the way SPLINE and HATCH curves are. The text or
block content (`ctx.content`) is neither extracted nor drawn -- the same best-effort
precedent as 3DSOLID's wireframe-only and VIEWPORT's frame-only rendering. An arrowhead is
drawn at the end of every leader line unconditionally, because the real "show arrowhead"
bit in `LEADER_Line.flags` is not extracted by the geometry-only C shim
(`uncad_multileader_get_lines` in `libredwg-sys/shim/uncad_shim.c`).

That shim exists because MULTILEADER's nested structure
(`entity.ctx.leaders[].lines[].points[]`) is unreachable through dynapi's flat
`dwg_dynapi_entity_value`, which only exposes top-level entity fields by name, and binding
`Dwg_MLEADER_AnnotContext` and friends with bindgen runs into the same struct-codegen
failure HATCH_Path already hit (see `crates/libredwg-sys/build.rs`). Walking the structs
inside the vendored C source, where dwg.h's real layouts are visible, and handing Rust a
flat `(x, y, z)` array sidesteps both problems.

**MLINE** is really a set of parallel lines (wall-style multi-line). Each line's offset is
read from the MLINESTYLE object the MLINE references, and drawn as a true offset polyline;
before MLINESTYLE was parsed this was approximated with a single centerline through each
vertex's `vertex` point, which is still the fallback when the style cannot be resolved.
`Dwg_MLINE_vertex` had to be hand-written in `crates/libredwg-sys/src/lib.rs` for the
bindgen reason above, with a compile-time assertion against clang's real `sizeof()` (96
bytes). A Linux/gcc Docker build turned up an extra wrinkle: blocklisting
`Dwg_MLINE_vertex` alone was not enough there, because bindgen still generated
`Dwg_MLINE_line` referring to the now-blocklisted type, which does not compile.
Blocklisting both fixes it -- the same class of platform-conditional divergence as the
other entries under "Windows/Linux".

**REGION** is `typedef Dwg_Entity__3DSOLID Dwg_Entity_REGION` in `dwg.h`, and `dynapi.c`'s
`"REGION"` entry points at the very same `_dwg_3DSOLID_fields` table -- byte-for-byte the
same struct. So `acis.rs`'s existing 3DSOLID wireframe extraction was reused directly,
with one change: `dwg_dynapi_entity_value` strictly compares the type name passed in
against the object's actual dxfname and silently fails on a mismatch, so `"3DSOLID"`
cannot be hardcoded and `extract_wireframe()` takes the name as an argument. Rendering
shares the isometric wireframe path.

**POLYLINE_PFACE** ("polyface mesh") could not reuse a dedicated C function the way
POLYLINE_3D does: LibreDWG's own `dwg_ent_polyline_pface_get_points` is marked
`/* not implemented. use the dynapi instead */` in `dwg_api.h`. Instead the
`VERTEX_PFACE` (vertex positions) and `VERTEX_PFACE_FACE` (up to 4 vertex indices per
face) subentity chain is walked with `get_first_owned_subentity`, and each face's indices
become wireframe edges -- rendered through the same isometric path as REGION, a polyface
mesh being just as inherently 3D as an ACIS solid's wireframe.

A DXF-read polyface's position vertices may come back typed `VERTEX_MESH` instead of
`VERTEX_PFACE`, so both are accepted inside a POLYLINE_PFACE's own chain. `in_dxf.c`
chooses between the two by resolving the VERTEX's group 330 and asking whether it names a
POLYLINE_PFACE; a file that points a vertex's owner at the *block record* instead -- which
is what ezdxf writes, and `ezdxf.audit()` passes it -- gets the VERTEX_MESH fallback. Until
0.3.0 that made every such polyface mesh draw nothing.

**TOLERANCE** renders exactly like ATTRIB/TEXT (position plus text). It carries
`text_plain` (`text::decode_text(text_value)`) next to the raw `text_value`, so the
`%%c`/`%%d`/`%%p`/`%%%`/`%%nnn` codes are decoded like any other text; the GD&T
feature-control-frame codes (`%%v` and similar) are left literally, because
`uncad::text` has no GD&T-specific handling. Readable, but not real GD&T symbols. Its
`text_height` is the entity's own only in R13/R14 files (LibreDWG decodes `height` for
those alone);
an R2000+ frame takes its DIMSTYLE's `DIMTXT`, else the header's, else 1.0 -- it used
to come out as 0 and draw nothing.

**ACAD_TABLE** has nearly INSERT's field shape in `dwg.h` (`ins_pt`/`scale`/`rotation`/
`block_header`), and its own `flag_for_table_value` comment states that 0x06 ("has a
block") is normally always set. So rather than computing a cell grid from
`num_cols`/`num_rows`/`col_widths`/`row_heights`, the cached block that `block_header`
points at is drawn through the same path as INSERT/DIMENSION -- reusing geometry AutoCAD
already computed instead of reconstructing cell content. Structurally that makes it about
as trustworthy as REGION. Note that dynapi's field-table key is `"TABLE"`, not the real
DXF name `"ACAD_TABLE"` (the same class of name mismatch as REGION/3DSOLID -- see the
`DWG_TYPE_TABLE` case in `convert.rs`).

**WIPEOUT** is the one type here carrying risk *beyond* "not verified against a real
file": its `pt0 + u*uvec + v*vvec` pixel-space-to-world transform comes from general
knowledge of the standard DXF image-entity convention (insertion point + U/V pixel vectors
+ clip vertices in that pixel space), not from LibreDWG's own source. LibreDWG does not
render images, only read and write the fields, so there is no reference implementation in
this codebase to check against, and neither `dwg.h`'s field comments nor anything else
here confirms the formula. See `wipeout_boundary` in `crates/uncad/src/convert.rs`.
Rendering is outline-only, deliberately: a filled shape risks becoming an opaque box
hiding other geometry.

**LIGHT** has no drawable geometry at all -- a light is correctly invisible in a 2D plan
view. It renders as an arbitrary placeholder: a small circular marker at `position`, plus a
dashed line to `target` for distant and spot lights. Same spirit as VIEWPORT's frame, but
weaker: a viewport frame at least means something, whereas a LIGHT marker conveys nothing
beyond "a light exists here".

## DXF reading inherits LibreDWG's own limits

`dxf_read_file()` is documented by LibreDWG itself as working "for most objects", so it is
not as complete as DWG reading. An LWPOLYLINE has been observed dropping silently out of a
real `.dxf` whose `ENTITIES` section clearly contained it, while ARC and ELLIPSE from the
same file parsed fine. This is an upstream limit, not something this project can patch
around.

A sharper example: taking `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg` (60
entities), writing it out as R2007 DXF with LibreDWG's own DXF writer, and reading that
back with `dxf_read_file` returns exactly 1 entity.

Two DXF-only defects that *were* patched are recorded under "Local patches to the vendored
LibreDWG" below: a polygon mesh made the reader refuse the whole file, and an entity
carrying both a true colour and a transparency had the two swapped. The per-entity
degradation this section describes -- a type dropping out, or arriving as
`Entity::Unknown` -- is what a DXF should cost; a whole-file refusal was not.

**An old-style POLYLINE's vertices are read from the owned-subentity chain, not from
LibreDWG's accessors.** `dwg_object_polyline_{2,3}d_get_points` and their `..._numpoints`
siblings walk `first_vertex .. last_vertex` as
`do { ... } while ((vobj = dwg_next_object (vobj)) && vobj != vlast);` for any file below
R2004, and that condition ends the loop before the body ever reaches `vlast`: they return
N-1 points on every R13, R14 and R2000 file, DWG and DXF alike. `get_next_owned_subentity`
stops *at* `last_vertex` and yields all N, so `convert::polyline_vertices` walks that
chain and reads each VERTEX's point and bulge together. Pre-R13 files fill neither
`first_vertex` nor `vertex[]`, so for them the chain is empty and the walk falls back to
the object list -- forward from the POLYLINE to the first object that is not a VERTEX,
which is what LibreDWG's own pre-R13 branch does. Measured against the shipped reference
DXF of `test-data/2000/PolyLine3D.dwg` (6 VERTEX records, where uncad used to report 5);
`crates/uncad/tests/fixtures/polyline_vertices_r2000.dxf` is the regression.

**A polyline's VERTEX records are skipped in the block-owned entity walk.** LibreDWG
documents `get_next_owned_entity` as returning entities and "not subentities: ATTRIB,
VERTEX", and its R13-R2000 branch does skip them -- but its R2004+ branch indexes
`BLOCK_HEADER.entities[]`, which the DXF reader fills with every object between the BLOCK
and the ENDBLK, vertices included. `convert::owned_entities` filters the five VERTEX_*
types itself, so one polyface mesh is one entity rather than nine, in `db.entities`, in
`tables.block_records`, in the CLI counts and in `report.json`'s unsupported list.

**DXF parse time grows faster than the file does.** Reading a DXF costs roughly the square
of its entity count, so sizes that are unremarkable for a real drawing take minutes. On
DXFs of nothing but LINEs (release build, wall clock around `uncad <file>`, summary only,
no rendering):

| Entities | Bytes | Time |
|---|---|---|
| 25 000 | 2.1 MB | 0.8 s |
| 50 000 | 4.2 MB | 1.4 s |
| 100 000 | 8.5 MB | 4.6 s |
| 400 000 | 34.5 MB | 204.5 s |

All of that is inside the vendored LibreDWG reader, not this crate. Instrumenting
`uncad_dxf_read_bytes` and the phases of `dwg_read_dxf` puts 4.6 of the 4.7 s at 100 000
entities inside `dxf_entities_read` (the TABLES, BLOCKS and OBJECTS phases and the
post-read fix-ups together are under 0.2 s), and this crate's own three conversion walks
plus `dwg_free` are the linear remainder of well under a second. Inside that phase the
super-linear term is LibreDWG re-resolving its *entire* object-reference vector every time
`dwg_add_object` reallocates the object pool and dirties the refs: 26 calls over 3.0 M
references at 100 000 entities, against 10 calls over 0.22 M at 25 000 -- the resolve work
alone grows about as n^1.9 while the entity count doubles. There is nothing to fix on this
side of the FFI boundary and the honest workaround is a DWG: the same drawing as DWG
parses in linear time. A caller that must accept large DXFs should bound the work itself
(a size or entity-count limit before calling `parse`, and a timeout).

## A DIMSTYLE's 0 is read as 0, unless the record is a husk

A dimension's label is formatted at its DIMSTYLE's `DIMDEC`/`DIMADEC`, and 0 is
an ordinary setting for both (whole millimetres, whole degrees). Until 0.3.0 a
0 there was taken as "unset" and the header's `$DIMDEC` stood in, so a metric
style asking for "50" produced "50.0000" -- a string neither the picture, nor
the cached label, nor `strings.json` carries.

Telling "the file said 0" from "the file said nothing" is not something the
record's numbers answer: LibreDWG's DXF reader creates the DIMSTYLE object,
presets the few fields it has defaults for (`DIMSCALE = DIMLFAC = DIMTFAC = 1`,
`DIMLUNIT = DIMALTU = 2`, `in_dxf.c`) and leaves the rest zeroed, exactly as if
the file had written 0. What separates the two in practice is the sizing pair:
no usable style has both a text height and an arrow size of 0, since a
zero-height dimension text draws nothing. `sample_2000.dxf` is the proof -- it
names the same `Standard` style that `sample_2000.dwg` writes in full (DIMDEC
4, DIMTXT 0.18, DIMASZ 0.18) and leaves all three at 0. So a record with a
`DIMTXT` or `DIMASZ` above zero is taken at its word, 0 included, and one with
neither is treated as a husk whose fields say nothing (`carries_a_body` in
`crates/uncad/src/dimension.rs`).

The residual case: a DXF that writes a style's body but omits group 271 gets
DIMDEC 0 where AutoCAD would apply its own default of 4. No file written by a
CAD program does that -- every one of the 40-odd real styles in the corpus
writes all of them -- and patching the vendored reader to default the field
instead would change what "0" means for every consumer of `tables.dimstyles`.
`DIMLUNIT` and `DIMLFAC` keep their "0 means unset" reading, since 0 is not a
value either can hold (the unit codes run 1..=6, and a factor of 0 would zero
every label). Precision is clamped to the DXF reference's maximum of 8: the
fields are 16-bit, and AutoCAD's `DIMADEC` of -1 ("use DIMDEC") reads back as
65535, which `format!` would honour literally.

## The polyline "closed" flag

Fixed in 0.3.0. Until then LWPOLYLINE's `closed` was read from bit 1 of `flag`, the DXF
group-70 convention, on the assumption that LibreDWG normalised to it; `dwg.h`'s own comment
documents bit 512 and was taken for an inconsistency. Ground truth settled it: comparing
`test-data/example_2000.dxf` (group code 70) with `example_2000.dwg` handle by handle, the
DWG's in-memory flag agrees with bit 512 on 11/11 polylines and with bit 1 on 0/11. Bit 1
means "an extrusion is stored" (every such polyline in the sample drawings carries extrusion
(0,0,-1), i.e. mirrored OCS geometry), and `in_dxf.c` rewrites DXF bit 1 to 512 on input,
so both input paths use 512 (`LWPOLYLINE_CLOSED_FLAG` in `crates/uncad/src/convert.rs`,
tested in `crates/uncad/tests/polyline_closed.rs`). The old reading exported every closed
LWPOLYLINE as open -- 741 of them across the sample drawings -- and the mirrored open ones
as closed. `POLYLINE_2D`/`POLYLINE_3D` keep bit 1, which is their real convention. The
extrusion itself is applied since 0.3.0 -- see "Coordinates are world" below; the
pre-0.3.0 state is what `docs/VLM_INVESTIGATION.md`, section 1, probed.

## The package: what `uncad export` does not do yet (since 0.3.0)

`export_package` writes the directory the design describes (section 2) in
its 0.3.0 form. Known gaps:

- **Text boxes are measured, the crop is not.** `texts.json` boxes come from
  a usvg pre-pass over the drawing's texts with the bundled font
  (`bbox_confidence: "measured"`, the tight box of the glyph outlines;
  rotated texts get the axis-aligned box of the rotated outlines). The
  crop still uses the 0.6-em estimate, since it is decided before anything
  is laid out; the frames and the tiles do not -- the measured boxes widen
  the drawn extents before the frame grouping and the tile culling, so a
  wide-glyph string (Hangul advances about a full em) is drawn on every
  tile its record names. usvg and tiny-skia work in single
  precision, so the renderer writes its SVG relative to the drawing's own
  origin (the rounded median of its entities, `ToSvgResult::origin`,
  `ToPngResult::origin`) whenever the coordinates exceed 32768 units: a
  plan at projected coordinates renders like one at the origin. A block
  reference's interior follows, written about the point its own placement
  sends to that origin, so a DIMENSION's cached geometry block -- which is
  placed through an identity transform because it already holds world
  coordinates -- is shifted like everything else. (Until 0.3.0 the interior
  of every block reference was written unshifted, which quantised every
  dimension out of the picture at 2.5e8 while the plain lines beside it
  drew perfectly.) What is
  left is the drawing's own span -- a box a million units from that origin
  (a drawing a million units across) is only exact to about 1/16 unit.
  A character the bundled subset lacks is drawn as a box: the record says
  `font_ok: false` with `unshaped_glyphs`, and the manifest warns.
- **A tile rasterizes what touches it.** Each tile's SVG holds only the
  entities whose extent meets the tile grown by 16 px, so cost follows the
  content on the tile; a dense drawing whose every entity touches every
  tile still costs `tiles x content`. Tiles of a level render in parallel;
  `--max-tiles` (400) and `--max-levels` (5) bound the total.
- **Sheets are plan views composited by rule.** The sheet is the layout's
  own limits (`LIMMIN`/`LIMMAX`, AutoCAD's placement of the paper, margins
  and plot origin folded in), else the paper size from the plot settings
  (portrait size turned by the rotation code, the printable corner moved
  by the plot origin at the layout origin), else the paper entities'
  extents; `sheets.json` says which (`rect_source`). A layout that has
  none of the three -- no paper size, no limits and nothing but point-like
  content -- has no rectangle to draw on and is skipped with an
  `UnusableSheet` warning rather than failing the export. The model is drawn
  through every viewport that is on, looks down the z axis and is not the
  sheet's overall frame, without the layers frozen in that viewport
  (`frozen_layers`, listed per viewport in `sheets.json`). An overall frame
  is detected as a DXF `id` of 1, or a view at scale 1 centred exactly on
  its frame (a DWG stores no id; the rule held on every corpus file). A
  viewport whose own layer is off, frozen or non-plotting
  still shows its window and loses only its border (AutoCAD hides the whole
  viewport for a frozen layer; the content is worth more to a reader than
  that fidelity). The twist sign follows ezdxf (a positive twist turns the
  picture counter-clockwise) and has not been checked against a plotted
  sheet; perspective and non-plan views are frames only; R13/R14 files
  store no view fields at all. A sheet's **texts** are records like any
  other since 0.3.0 -- in `texts.json`, with `space: "paper"`, the `sheet`
  they are on and that sheet image's pixel box, and indexed in
  `strings.json` -- so the drawing's title and its title block are
  findable; paper-space *geometry* is drawn but not listed, and sheets get
  an overview only, no tiles. `sheets.json` states how a model point lands
  on the sheet in the fields its own viewport records carry (`frame`,
  `model_window`, `scale`, `twist_deg`).
- **Frames follow proximity, not meaning.** Entities within 5 % of the crop
  diagonal of each other (`--frame-gap`) are one group; a detached group
  with 20 entities or a text becomes a frame. Two details drawn close
  together share a frame, and a title block touching the plan joins it.
  Frames are model space only; the sheets above are the paper side.
- Text is rendered with the bundled `Uncad Sans` (a Noto Sans KR subset:
  Latin, Greek, the 2350 KS X 1001 Hangul syllables, CAD symbols -- see
  `crates/uncad/fonts/README.md`), the same on every machine. Hanja, the
  other syllables and `⌀` (U+2300) are outside it; `--fonts bundled+system`
  lets the host's fonts fill them in at the cost of host-dependent output.
  SHX fonts and the drawing's own text styles are not used: every text is
  drawn in the one face.
- Records for entities inside block references are limited to texts;
  geometry inside blocks is drawn but not listed (INSERT instances are).
  The texts cover every string the picture shows except one: the label
  inside a DIMENSION's cached `*D` block, which `dimensions.json` carries
  as `display` (and `strings.json` indexes from there) rather than
  duplicating as a text record. The walk follows block references as deep
  as the renderer does (`limits::MAX_BLOCK_REF_DEPTH`, 20); until 0.3.0 it
  stopped at 8, so a string nested nine or more blocks deep -- a bound
  XREF of an assembly of assemblies -- was drawn in the tiles and indexed
  nowhere. An ACAD_TABLE's cells and a TOLERANCE's
  frame are indexed like any other text, under the ids the picture draws
  them with (`<table>/<cell>`, the tolerance's own handle); until 0.3.0
  they were drawn and indexed nowhere, so a reader searching
  `strings.json` for a value they could see in a cell found nothing.
  Indexing them also lets the legibility model see them, so a drawing
  whose only small text is in a table now zooms deep enough to read it.
- **A sidecar over budget loses rows, then layer names.** `layers_present`
  used to be written whole whatever it weighed -- 900 layers with
  45-character names put a tile 37 % past the 32 KB cap while
  `records_truncated` said nothing had been dropped. The geometry rows go
  first (they are the bulk of a dense tile, and `counts` and
  `geometry_by_kind` survive whatever is cut), then the other record rows,
  then the layer list; `records_truncated` / `layers_truncated` (with
  `layers_total`) say which. A sidecar's `counts` is always the true
  number of records of each kind on the tile, so a reader can tell how
  much of it the rows are missing.
- **A pixel box is clipped to the image it is quoted in.** A record is
  listed on an image whenever its world box merely meets it, so the box of
  a record that continues past the edge named pixels the image does not
  have (a dimension quoted 8115 px beyond a 1092 px tile). Every box in a
  record's `px` map and in a tile sidecar is clipped to that image; the
  record's full extent is its world `bbox`, and the rest of it is on the
  other images in its `tiles` list.
- **A detached group too small to frame gets no tiles.** A group below
  `--min-frame-entities` (20, or one text) stays in the overview only, so
  its records carry `tiles: []`. It is listed in `manifest.frames_dropped`
  with `reason: "below_min_entities"` and raises a `SmallGroups` warning,
  as the groups past `--max-frames` do with `reason: "max_frames"`; the
  list is capped at 100 entries, with `frames_dropped_total` beside it.
- **A very large outline is not checked for self-intersection.** A closed
  polyline of more than 2 000 vertices reports `simple: null` with
  `confidence: "estimated"` and a `why`, because the check compares every
  pair of segments: one 64 000-vertex contour held the export for 175 s
  for that one boolean. The area is still reported; it assumes the outline
  does not cross itself. The largest outline in the corpus has 378
  vertices.
- **A layout name is cut to 100 characters in the path.** `sheets/<name>/`
  is the sanitised layout name, and a name past the filesystem's 255-byte
  per-component limit used to fail the whole export, manifest and all.
  `sheets.json` and the manifest still carry the name in full.

## The crop: what the picture shows (since 0.3.0)

`uncad::crop` decides the viewBox (design section 4). The overview shows
every visible entity except at most `max(3, 1 %)` *outliers*. Each pass
sets aside the largest entities and the farthest from the median centre
(at most a quarter of the drawing) and measures them against the rest: a
diagonal over 20x the rest's is a `scale_outlier` (the 3256x INSERT in
`example_2000.dwg`), a gap of more than 20 rest diagonals a `far_outlier`
(a stray point a million units out) -- never more than a
fifth of the drawing, so a notes block one drawing-width away stays. More
candidates than that means the drawing really is that big and nothing is
excluded. Each exclusion is reported with its handle and
reason; `--crop raw` keeps everything. 0.2.0's cluster trim (keep the
dominant corner-connected cluster, absorb neighbours within 30 %) could
silently drop a detail drawn beside the plan; it is gone.

Two things to know:

- **Text extents are estimates while the crop is chosen.** The renderer
  measures TEXT/MTEXT/ATTRIB from their anchor and a 0.6-em-per-character
  guess, not from glyph metrics, so a label at the edge of a drawing can be
  clipped by a few characters. The design's metrics pre-pass exists
  (`measure_texts` in `export.rs`, a usvg parse of the drawing's texts at
  world scale, which is where `texts.json`'s `bbox_confidence: "measured"`
  comes from), but it runs on the rendered drawing, so the crop is already
  decided by the time its boxes exist; the frames and the tile culling do
  use them (see "Text boxes are measured, the crop is not" above). Feeding
  them back into the crop is the open item.
- **The header extents are a candidate, not the truth.** `$EXTMIN/$EXTMAX`
  are used (in `Auto`) only when sane, no more than 4x the content area,
  containing 90 % of the entities and covering more of them than the
  computed content does; `--crop header` forces them when sane. AutoCAD's
  own extents include outliers (`example_2018.dwg`'s 3256x INSERT), so they
  are no rescue there.

Excluded entities are not drawn at all (a 3256x INSERT clipped by the
viewBox would still cross the whole picture); `--crop raw` shows them.

## Hidden entities are left out of the picture (since 0.3.0)

A DWG carries entities nobody sees: layers switched off or frozen, layers
marked "do not plot", AutoCAD's own `DEFPOINTS` layer (dimension definition
points), and entities with their own invisible flag (DXF 60). 0.2.0 drew all
of them. The renderer now skips them -- inside blocks too -- and counts them
in `ToSvgResult::hidden`; `ToSvgOptions::include_hidden` (CLI
`--include-hidden`) draws them at 50 % opacity instead. The rule is one
function, `visibility::hidden_reason`, and the JSON still lists every
entity: it is the model, not the picture.

Two limits of the layer state, both inherited from how LibreDWG reads:

- **The plot flag is only trusted from R2000+ DWG files.** A DXF's group 290
  is optional (AutoCAD omits it on plotting layers), and LibreDWG's reader
  leaves an omitted 290 and `290 = 0` looking the same, so every DXF layer
  reads as plotting. R13/R14 DWG files store no plot flag at all. `DEFPOINTS`
  is hidden by name, which covers the common case.
- **Lineweights are unknown where the file stores none:** R13/R14 files, and
  a DXF layer without group 370 (which LibreDWG leaves at code 0, i.e.
  0.00 mm -- reported as unknown rather than as the thinnest weight).

Per-viewport frozen layers are read into `ViewportEntity.frozen_layers` and
listed in `sheets.json`, but only the sheet compositor acts on them: a layer
frozen in a viewport is left out of that viewport's part of
`sheets/<layout>/overview.png`. The model-space picture and
`visibility::hidden_reason` ignore them by design -- they are a property of a
viewport, not of the drawing. Not modelled at all: layer overrides in layouts,
and xref layer state. Locked layers are drawn, as in AutoCAD. Linetypes and
lineweights are read but not rendered (0.4.0).

## Coordinates are world; the OCS is applied on read (since 0.3.0)

DXF stores several 2D entity types in their own object coordinate system
(OCS, group 210 "extrusion"): CIRCLE, ARC, LWPOLYLINE, POLYLINE_2D, TEXT,
ATTRIB, INSERT, SOLID, plus HATCH boundaries and a few point fields of other
types. In practice the only OCS that shows up is the mirrored one, normal
(0,0,-1), which AutoCAD's MIRROR command produces: the entity's x is
negated in world terms. 0.2.0 reported the stored OCS values as if they
were world coordinates, so every mirrored circle, arc, polyline, text and
block reference sat on the wrong side of the y axis.

`convert.rs` now applies the arbitrary-axis algorithm (`geom::ocs_to_wcs`)
to CIRCLE/ARC centres, LWPOLYLINE/POLYLINE_2D vertices (at their
`elevation`), TEXT/ATTRIB anchors, INSERT insertion points and SOLID
corners, and each of those entities carries its `extrusion` so a consumer
can tell a mirrored one apart. A mirrored ARC's angles are mirrored and
swapped so the arc still runs counter-clockwise from `start_angle` to
`end_angle`, and a mirrored polyline's `bulges` change sign with its
vertices (the reflection reverses each arc's turn; `mirrored_bulge_r2000.dxf`
draws the same arc both ways). A mirrored INSERT keeps its stored `rotation` and `scale`; the
renderer draws it with the x scale and the rotation negated, which is the
same transform. `tests/fixtures.rs` checks all of this against
`mirrored_ocs_r2000.dxf`.

Not applied: HATCH boundary paths (stored in the hatch's OCS, so a mirrored
hatch still draws on the wrong side), DIMENSION's `text_midpoint` (an OCS
point in DXF terms, left as stored) and the glyphs of mirrored TEXT/ATTRIB,
which AutoCAD draws back to front from the anchor while this renderer draws
them reading left to right from the same anchor. A tilted normal (anything
but (0,0,+-1)) transforms points correctly, but an INSERT with one is drawn
as if it were upright.

## A corrupt coordinate, size or angle leaves that entity undrawn (since 0.3.0)

LibreDWG hands a decoded field through as it found it, so a corrupt or fuzzed file can
produce a `NaN` or infinite coordinate, a radius of `inf`, or an angle of 1e20 (a
fuzzed DWG in the test set stores 1.4e247). Neither `NaN` nor `inf` is in SVG's
`<number>` grammar, and an angle that large names no direction at all -- an `f64` step
past ~1e6 rad is already coarser than 1e-10 rad.

The renderer screens both at the entity level (`svg::finite`, `geom::is_sane_angle`, the
way `crop::Rect::is_sane` screens rectangles) and draws nothing for the entity whose
values fail; a polyline keeps the vertices that are real numbers and is dropped only when
fewer than two remain. Underneath that, every number written into an attribute goes
through `svg::format::clean`, which maps a non-finite value -- and anything below 1e-12,
which Rust's `f64 Display` would otherwise spell out with 300 leading zeros -- to `0`, so
the document is well-formed whatever the input. Such an entity is *not* listed in
`unsupported_types`: the type is supported, this instance's values are not.

Before 0.3.0 these reached the file as `x1="NaN"` and `r="inf"`, and an ARC with a huge
angle made the arc-bounds walk (`geom::BulgeArc::bounds`, one step per quarter turn)
spin forever -- `to_svg`, `to_png` and `export_package` never returned. That walk is now
bounded at four quarter crossings by construction, which is all any arc can have.

## Every number the renderer takes from the file is bounded (since 0.3.0)

A coordinate that is `NaN` costs one undrawn entity (the section above). A *count* that is
wrong costs the process: it becomes an allocation size or a loop bound, and nothing in the
arithmetic says how big is too big. One flipped byte of `lib/libredwg/test/test-data/`
`example_2000.dwg` (offset 130005, `0x80` -> `0x4B`) redirects the `CIRKLO_PUNKTOJ` block
record's owned-entity chain so the block holds eight INSERTs **of itself** beside its fifty
drawable entities. The file parsed in 0.03 s and reported nothing odd; rendering it then
spent three minutes growing one SVG string and aborted the process on a **12,074,460,607-byte**
reallocation. A depth cap alone does not help -- eight self-references reach 8^20 instances
long before twenty levels of nesting.

`uncad::limits` now names every such bound in one place, with the reasoning on each
constant:

| Constant | Value | What it bounds |
| --- | --- | --- |
| `MAX_BLOCK_REF_DEPTH` | 20 | how deep block references may nest |
| `MAX_BLOCK_REFS` | 100 000 | how many references one render expands in total (the *breadth* the depth cap cannot see) |
| `MAX_SVG_BODY_BYTES` | 64 MiB | how large the emitted drawing body may grow -- the backstop behind the rest |
| `MAX_ENTITY_SVG_BYTES` | `MAX_SVG_BODY_BYTES` / 4 = 16 MiB | how much one top-level entity may draw before its block expansion is cut short |
| `MAX_ENTITY_POINTS` | 100 000 | how many file-supplied points one entity may draw with |
| `MAX_HATCH_TILE_SPAN` | 16x the boundary | how much larger than the shape it fills a HATCH pattern's tile may be |
| `MAX_WORLD_COORDINATE` | 1e15 | the largest value the rasterizer may be handed where its work grows with the number (an arc radius; an entity's measured box, which the viewBox is built from) |
| `MAX_SUBENTITY_DEPTH` | 2 | how deep an owned-subentity walk may recurse on the way in (see the section below) |
| `MAX_OWNED_SUBENTITIES` | 100 000 | how many subentities one such walk may hand back |

Every cap **the renderer** applies is *reported*, never silent: `ToSvgResult::limits` and
`ToPngResult::limits` carry a `LimitReport`, the CLI prints it as a warning, and a package
puts it in `report.json` under `limits` and in `warnings`. The same drawing now renders in
0.6 s to a 385 KB SVG (394 554 bytes) whose warning reads "1824 block references past the
20 nesting / 100000 expansion limit; 8 entities that would each have drawn more than
4 MiB". The 4 MiB per-entity cap is what those eight hit; with it in place the drawing body
never approaches the 64 MiB backstop, so nothing is counted against `MAX_SVG_BODY_BYTES`
any more.

A count on its own is not enough to act on, so `LimitReport::dropped` also *names* the
entities -- handle, DXF type and which cap acted -- for the first 100 of them
(`MAX_REPORTED_HANDLES`); the CLI's warning names the first five, and a package copies them
into `report.json`'s `excluded` list beside the crop's own exclusions, so "what is missing
from the picture and why" is one list. Until 0.3.0 shipped, three of the caps reported
nothing at all, and a consumer auditing a package could not tell a dropped entity from one
the file never held.

Two caps in the table are *not* in `LimitReport`, and that is the one remaining gap:
`MAX_SUBENTITY_DEPTH` and `MAX_OWNED_SUBENTITIES` bound the owned-subentity walk in
`convert.rs`, which builds the model *before* any render, so there is no render for a
render-time report to belong to. Both are reached only by damaged handle chains (one level
of subentity nesting is what the format has, and no entity owns 100 000 children), so a
real drawing never meets them; a corrupt one meets them silently. Carrying the counts out
of conversion and into `CadDatabase` is the fix, and it is not done.

Not every one of these is about memory. Two of them bound the *rasterizer's* work, which a
finite number can run away with just as easily; both were found by the fuzz sweep after the
allocation caps landed, on drawings that produced a perfectly small SVG:

- **A coordinate of 1e150 is finite,** and one entity carrying it takes the measured
  extents with it -- and so the viewBox, the automatic stroke width and every length
  derived from them. One fuzzed `example_2000.dwg` wrote a 590 KB SVG with a viewBox
  1.45e150 units wide and `stroke-width="5.2e149"`; `to_png` had not returned after five
  minutes. `MAX_WORLD_COORDINATE` is the bound `crop::Rect::is_sane` already applied to a
  header's `$EXTMIN`/`$EXTMAX`, and it is applied to an entity's **measured box** -- after
  the block transform, since a corrupt block scale lands a sane coordinate just as far out.
  Such an entity is held out of the crop *decision* and listed in `report.json`'s
  `excluded` as a `scale_outlier`; it keeps its records. It is deliberately not a screen on
  the coordinate as written, which is what 0.3.0's first cut did: that deleted the entity
  from the picture, from every record and from every report at once, where the crop's
  outlier rule had already been drawing the same conclusion out loud. An entity a long way
  from its neighbours is a drawing problem, and the crop is what answers it; the magnitude
  bound is only there for what the rasterizer cannot take.
- **A bulge of 1e-160 over a hundred-unit segment is an arc of radius 1e238.** The same
  drawing emitted `A 7.1e238 7.1e238 ...` between two points a few thousand units apart,
  and rasterizing that did not finish in five minutes at *any* image size -- 200 px
  included, so it was the arc-to-bezier conversion and not the pixel count. A radius that
  large is a straight line, and is now drawn as one. (This was 0.46 s after the fix.)

Three of the caps deserve their reasoning spelled out:

- **One entity covering the whole picture is what costs a package, not a large drawing.**
  A tile rasterizes every part whose extent touches it, so a single INSERT that expanded
  into a picture-wide part is re-assembled and re-parsed for every tile at every zoom
  level, on up to sixteen threads at once. The 18 MB of body the largest real sample emits
  is spread over 40 000 small parts, so each tile keeps a handful and `uncad export` peaks
  at 402 MB in 3.2 s; a fuzzed `example_2000.dwg` whose body was the same order of
  magnitude but held in a few huge parts peaked at **5.7 GB over 132 s**. So rendering one
  part stops at `MAX_ENTITY_SVG_BYTES`, which bounds the work.

  What happens *at* the cap took two tries. Dropping the part whole (0.3.0's first cut, at
  a flat 4 MiB) made the cap the wrong shape: a drawing that wraps its content in one block
  and places it once -- a bound XREF, an imported survey, a "whole floor" block -- is a
  single top-level INSERT, so a per-*entity* cap was really a cap on the whole drawing, and
  past about 65 000 short lines the picture came out empty with a warning that named no
  handle. The cap is now a share of the document's budget (a quarter, 16 MiB, over 200 000
  `<line>` elements from one entity), and reaching it **truncates** the expansion at the
  next block boundary instead of deleting the part: the walk unwinds at an entity boundary,
  so every enclosing `<g>` still closes and the part stays well-formed. The entity is
  counted in `LimitReport::truncated_parts`, named in `LimitReport::dropped` under the
  `entity_bytes` cap, and keeps its records -- it *is* in the picture, only not all of it.
  The fuzzed file exports in a few seconds, and its package is a few MB instead of 150 MB
  (it had been writing 200 000 text records -- the same string repeated by the
  self-referencing block -- across 1 502 shard files). The package's own text walk carries
  the expansion budget too, for the same breadth-versus-depth reason. The trade-off is
  named: one truncated part is up to 16 MB of SVG that every tile touching it re-parses, so
  a file crafted to produce one still costs a package more than a real drawing does.


- **The output budget is what actually bounds the allocation.** It is checked before each
  entity, at every level of the block walk, so exhausting it unwinds the whole walk rather
  than merely skipping one entity. The finished document can exceed it only by the last
  entity drawn plus the `<g>` wrappers closing above it.
- **A HATCH pattern's spacing is the SVG `<pattern>` tile's size in user units,** and resvg
  allocates a pixmap for that tile at the *device* scale of the element being filled. A
  corrupt spacing of 1e12 over a ten-unit boundary therefore asks for a pixmap around 1e11
  pixels on a side. Such a tile can show at most one line anyway, so the pattern is dropped
  and the hatch keeps its outline. The opposite direction (a spacing far *below* the pixel
  grid) is safe without a cap: the tile rounds to a pixel or to nothing, and tiny-skia's
  pattern shader costs one pass over the filled pixels however many repeats that is.

The caps are far above any real drawing: the largest sample this project renders
(`AutoCADSamples5.dwg`, ~40 000 entities) emits 18 MB of SVG, under a third of the output
budget, and none of the corpus files or the seven AutoCAD samples engage any cap at all --
their documents are byte-for-byte what they were before. `crates/uncad/tests/limits.rs` is
the regression: the one-byte corruption above, a self-referencing block, a block chain
deeper than the cap, a block fanning out below it, a polyline past `MAX_ENTITY_POINTS` and
a hatch whose tile dwarfs its shape.

## Fixed: a corrupt attribute chain recursed until the stack ran out (since 0.3.0)

An INSERT owns its ATTRIBs, and an ATTRIB is itself an entity, so converting an INSERT
converts them too -- the one place `convert::convert_entity` recurses. Three flipped bytes
of `example_2000.dwg` (offsets 581356, 581784 and 582336, bisected out of a fuzzed file's
1 155 mutated offsets) point that chain back at the INSERT, and the conversion then
recursed endlessly: a 512 MB stack was not enough either, and the process died with
STATUS_STACK_OVERFLOW (0xC00000FD). The decoder itself was untouched by this -- reading the
same file through `uncad_dwg_read_bytes` alone returned normally -- so it was this crate's
walk, above the FFI boundary, that died.

The walk now stops at the first subentity whose type is not ATTRIB (an INSERT owns nothing
else, and LibreDWG's own R2000 walker uses the same condition to terminate), and is bounded
besides by `limits::MAX_SUBENTITY_DEPTH` (2) and `limits::MAX_OWNED_SUBENTITIES` (100 000)
-- the second against a chain damage has turned into a *ring*, which is not recursion but a
loop that never ends. The polyline walk (`polyline_subentities`, shared by the vertex and
the polyface readers) carries the same length bound, on both its owned-subentity chain and
its pre-R13 object-list fallback. `crates/uncad/tests/corrupt_dwg.rs` is
the regression.

**A null dereference below the boundary is still reachable from here.**
`get_next_owned_subentity()` in the vendored `dwg.c` (line 1426) calls
`dwg_next_object (current)` and then reads `obj->fixedtype` in the R13-R2000 INSERT, MINSERT
and POLYLINE branches without checking for null -- and `dwg_next_object` returns null when
`current` is the last object in the file. With only the recursion bounded, the three-byte
file above reached exactly that and died with an access violation (0xC0000005); adding
`if (!obj) return NULL;` at the top of the function made the same file parse cleanly, which
is how the diagnosis was confirmed. Stopping the walk at the first non-ATTRIB takes this
crate off that path for that file, but not in general: a corrupt drawing whose *last* object
is a real ATTRIB on an INSERT's chain can still reach the unchecked read. It is left in
place here because it is below the FFI boundary -- the one-line fix belongs upstream, and
this vendored copy already carries two local patches that a submodule update must re-apply.

## Text placement is approximate (and MTEXT rotation was 0 until 0.3.0)

`MTextEntity::rotation` is `atan2(x_axis_dir.y, x_axis_dir.x)`, the angle of the DXF
group-11 direction vector, since 0.3.0; it was a fixed `0` before, which mis-placed the
74-97 rotated MTEXTs found per sample drawing. Justified TEXT/ATTRIB is anchored at its
alignment point with SVG `text-anchor`, and MTEXT at its attachment point. The CAD text
height is the height of the capitals, so a text is drawn at `font-size = height / 0.733`
(the bundled face's cap-height ratio, `png::BUNDLED_CAP_HEIGHT`) and its capitals come
out the drawing's height; before this the height was used as the em and every label was
27 % too small. The vertical offsets (one text height for top, half for middle, a
0.2 em descender for bottom) and the glyph widths still come from the renderer's font,
not from AutoCAD's SHX fonts, so the extent of a string is approximate even though its
anchor is exact. MTEXT baselines are 5/3 of the text height apart (AutoCAD's single
spacing) times the line spacing factor. MTEXT word-wrapping at `rect_width` is not
performed: a paragraph is one line until `\P`.

An MTEXT attachment row places the *cap band* -- the cap top of the first line down to
the last baseline -- the same way in the renderer and in `text::estimate_mtext_box`, which
is what the crop and the entity extent are built from: the top row hangs it below the
anchor, the middle row centres it, and the bottom row puts the text's bottom on the
anchor, so the last baseline sits a descender (0.2 em) above it, exactly as vertical
alignment 1 does for a single-line TEXT. Until 0.3.0 the bottom row had that descender's
sign the other way, drawing the block about a third of a line low. An empty line is a real
line and keeps its line height (`\P\P` is how a note spaces its paragraphs); until 0.3.0
blank lines were dropped, which moved every line after a paragraph break up by one line
height.

## Layer colors: `Dwg_Color.rgb` is untrustworthy, and `color_index` needs a fallback

`Tables` does not expose a layer's `Dwg_Color.rgb`. On an older LibreDWG that field was
measured as a constant `0xFFFFFF` placeholder, unrelated to the layer's real color, across
22 real files. The principle still holds: BYLAYER resolution goes through `color_index`
only, and `rgb` is never trusted. The
`bylayer_resolves_through_layer_colorindex_not_layer_rgb` regression test in
`crates/uncad/src/color.rs` guards that.

Since the submodule moved to a newer upstream, `rgb` is no longer always `0xFFFFFF` --
and rendering 9 real files to PNG showed every one of them coming out entirely black
(already at the SVG stage, so not a rasterization problem). Traced to `bit_read_CMC` in
`lib/libredwg/src/bits.c`:

All 9 files are AC1018 (R2004) native, with layer colors stored as `method=0xc3`
(TRUECOLOR) but abnormally small `rgb` values (`0x000001`-`0x000008`). After reading,
`bit_read_CMC` tries a reverse palette lookup with
`color->index = dwg_find_color_index(color->rgb)`, and such a small `rgb` matches no
palette entry exactly, so the no-match sentinel `256` comes back. `ACI_PALETTE` holds `0`
(black) at index 256, so calling `aci_to_hex(256)` flattens every layer to black.

Those low bytes are not random: they are deterministic across layer names and files. The
`"0"` layer is `rgb=..07` in all 9, matching AutoCAD's real default of ACI 7 for it;
`"...TITL"` is always `5`, `"...BOLD"` always `6`, `"...FINE"` always `8`. That is the ACI
index sitting directly in the `rgb` field. LibreDWG's own `bit_downconvert_CMC` already
carries the identical `if (index == 256) index = rgb & 0xff;` fallback on the opposite
conversion path; only the pure read path (`bit_read_CMC`) lacks it.

`resolve_layer_color_index` in `crates/uncad/src/tables.rs` mirrors that fallback for the
read path. All 9 files were rendered and visually checked against AutoCAD's default and
AIA layer-color conventions (roofs yellow, doors and windows green, outlines blue, piping
cyan), but **never compared against an actual AutoCAD screen** -- unlike this project's
other color bugs, this one was verified by plausibility rather than by reference. It also
inherits `bit_downconvert_CMC`'s own limitation: a genuine arbitrary truecolor that
happens not to match the palette is misread as a small ACI index.

## Entity colors: which field means "this entity states an RGB"

`EntityCommon::true_color` is not `Dwg_Color.rgb`, and not "`method` is TRUECOLOR"
either. `Dwg_Color` always holds *something* in `rgb`, and the two readers fill the struct
differently enough that a single test is wrong in both directions. What each one really
does (`convert::split_entity_color` is the single place that knows):

| written as | reader | `flag` | `method` | `rgb` |
|---|---|---|---|---|
| R2004+ DWG, true colour | `bit_read_ENC` (bits.c) | `0x80` | never set | the value |
| R2004+ DWG, DBCOLOR reference | `bit_read_ENC` | `0x40` | never set | not the colour |
| R13-R2000 DWG | `bit_read_CMC` | 0 | 0 | 0 -- no true colour exists before R2004 |
| DXF group 62 alone | `dxf_set_CMC_index` (in_dxf.c) | 0 | `0xc3` | **synthesised** from LibreDWG's ACI palette |
| DXF group 62, BYLAYER/BYBLOCK/none | `dxf_set_CMC_index` | 0 | `0xc2`/`0xc1`/`0xc8` | the method byte alone |
| DXF group 420 | the common-entity loop | 0 | the value's top byte (0 for a plain 24-bit RGB) | the value |

So the old `method == DWG_COLOR_METHOD_TRUECOLOR` test dropped every real group 420 and
every R2004+ DWG true colour (no entity in the 105 corpus DWGs or the 7 AutoCAD samples
ever reported one), while reporting an RGB for every DXF entity that stated only an index.

Two cases stay indistinguishable from the fields available, and are reported as "no true
colour" -- which renders identically either way: a group 420 of pure black (`rgb` 0 is
also what an untouched field holds), and a group 420 that repeats the entity's own ACI
colour exactly. `crates/uncad/tests/fixtures/entity_truecolor_r2000.dxf` pins the four
shapes a DXF can write.

## The ACI palette is the vendored one, and the contrast rule still merges two greys

`color::ACI_PALETTE` is `rgb_palette[256]` from
`crates/libredwg-sys/vendor/libredwg/src/dwg.c`, transcribed as hex. Until 0.3.0 it was a
formula-generated approximation -- the pure hues and every multiple of 10 were right and
the shades between them were a linear ramp -- which differed from AutoCAD at 222 of the
255 real indices. The grey ramp is the clearest case: 250-254 read
`333333 5B5B5B 848484 ADADAD D6D6D6`, an exact interpolation from `0x33` to `0xFF`, where
AutoCAD has `333333 505050 696969 828282 BEBEBE`.

Using LibreDWG's own table also settles a second inconsistency: `in_dxf.c` resolves a DXF
group 62 through `dwg_rgb_palette_index()`, so an index LibreDWG expanded and an index
this crate expands now agree byte for byte.

**Still merged:** the white-page contrast rule (`contrast_on_white`) scales any colour
brighter than `MAX_LUMINANCE_ON_WHITE` down to exactly that luminance, so every grey from
`#737373` up renders as `#737373`. ACI 8 (`#414141`) is now distinct, but ACI 9
(`#808080`) and ACI 254 (`#BEBEBE`) still come out the same in an image. The JSON and
`resolve_color` keep the file's own colours; only the rendered image is affected.

## Layer 0 inside a block takes the layer of the reference

AutoCAD's rule: geometry created on layer 0 in a block *definition* is placed on the layer
of the block *reference* when the block is inserted, and resolves its BYLAYER colour
against that layer. Drawing a symbol on layer 0 so it picks up the insert layer is the
standard idiom -- 377 of the 813 INSERTs in one AutoCAD sample rely on it -- and until
0.3.0 every such child rendered in layer 0's own colour (black).

This is deliberately a *render-time* resolution, not something `convert.rs` bakes into the
model: one block definition is placed by many INSERTs on many layers, so "the layer of
this entity" only has an answer per reference. `EntityCommon::layer` keeps what the file
stores, `color::effective_layer` is the one place that says what layer 0 means in context,
and `svg::Ctx::inherited_layer` carries the enclosing reference's *effective* layer down,
so a layer-0 block nested in a layer-0 block still ends at the outermost reference's
layer. The export's text records name the same effective layer, so the records and the
picture agree. `crates/uncad/tests/fixtures/block_layer0_r2000.dxf` is the regression.

Not resolved this way: **visibility**. `visibility::hidden_reason` still asks about the
entity's own layer, so a layer-0 child is not hidden by freezing the INSERT's layer -- in
practice the INSERT itself is hidden first, and nothing inside a hidden reference is
walked. Linetype and lineweight are not inherited either; nothing renders them yet.

## Fixed: pre-R2007 and DXF text lost every non-ASCII character

Before 0.3.0, `parse()` turned every non-ASCII character in an R2000/R2004 DWG (and in a
DXF of any version) into U+FFFD: LibreDWG's dynapi only transcodes the R2007+ UTF-16
storage, and for older files it returns the raw code-page bytes, which this crate then
decoded with `to_string_lossy`. Korean text in the still-common R2000/R2004 exchange
files, and the plus-minus and degree signs in dimension text, were all lost (measured on
the sample drawings: `\A1;±3 1/2"` came out as `\A1;\uFFFD3 1/2"`). The
`uncad_tv_to_utf8` shim now transcodes with LibreDWG's own code-page tables -- see
`docs/ARCHITECTURE.md`, "Strings and paths". Verified on the samples for CP1252; the CP949
path is exercised by `crates/uncad/tests/fixtures/` (a DXF this project wrote itself), not
yet by an AutoCAD-written Korean DWG. A character the table cannot map becomes U+FFFD
(LibreDWG's own converter wrote a NUL there and truncated the string). Related, also fixed:
an R2007+ **DXF** parsed to zero entities, because LibreDWG stores its strings as UTF-16 but
hands them out unconverted for DXF input, so every block name was cut at the first NUL
(`"*Model_Space"` read as `"*"`); and a corrupt or unknown code-page value in the file
header is now replaced by ANSI_1252 instead of being used to index LibreDWG's tables. That
restored the string *contents*; the same UTF-16 mismatch also broke every name-to-handle
lookup the DXF reader makes -- see the next section.

## Fixed: an R2007+ DXF resolved no layer and no block reference

Until this was fixed, every entity of a DXF whose `$ACADVER` is AC1021 (R2007) or later
came back with `layer: ""` unless its layer name was a single character, and every INSERT
with `block_name: ""`. Nothing warned: the entity count looked right, but the renderer
resolved no block record for any INSERT and drew none of their contents, `blocks.json`
listed no instances, ByLayer colour fell back to black, and the layer-off / frozen /
non-plotting / DEFPOINTS rules could never fire because no entity had a layer to match.
`example_2018.dxf` reported 65 of its 72 entities on layer `""` and 0 hidden entities,
where the same drawing as `example_2018.dwg` reported 33.

The cause is the same UTF-16 mismatch as the text above, on the other side of the API.
LibreDWG's dynapi *writes* a string field as UTF-16 as soon as `dwg->header.version >=
R_2007`, whatever the input format, but *reads* it back as UTF-16 only under
`IS_FROM_TU_DWG()`, which additionally demands the data did not come from a DXF or JSON
import (`bits.h` admits the gap in its own comment: "only if from r2007+ DWG. not JSON,
DXF (FIXME TABLE.name)"). So `dwg_find_tablehandle()` and its siblings compared the
DXF's 8-bit group-8 / group-2 name against a UTF-16 buffer read as a C string, which stops
at the first NUL: `"Tavolo 3"` compared as `"T"`. Only one-character names such as layer
`0` matched, which is exactly the set of entities that survived. The entity's `layer` and
the INSERT's `block_header` handle were then left NULL. Fixed by a local patch to the
vendored `dwg.c` (see below); `crates/uncad/tests/r2007_dxf_handles.rs` compares
`example_2018.dxf` against `example_2018.dwg` on layer assignment, block names and hidden
entities, and checks that no R2007+ corpus DXF leaves an entity without a layer.

## A corrupt DWG can abort the process below the FFI boundary

`parse`/`parse_bytes` hand the file's bytes to LibreDWG's C decoder, and a malformed DWG
can terminate the whole process there rather than returning `ParseError`. No Rust guard
can intercept it: `catch_unwind`, the `LIBREDWG_LOCK` and the caught rasterizer panic all
sit above the FFI boundary, and a C-level `abort()`/fail-fast unwinds nothing. **A service
or agent tool that parses untrusted drawings should do it in a separate process** it can
lose, not in the one serving other requests.

One such abort is fixed -- see `cvt_TIMEBLL` below; before the fix a single changed byte in
a corpus DWG (`2000/Helix.dwg`, offset 27644) killed the process with 0xC0000409 on every
run, and a sweep of 24 corpus drawings x (5 truncations + 4 byte-flip mutations + a version
tag) hit the same abort on 8 of 12 seeds. After the fix the same sweep over 12 seeds (5 760
CLI runs: summary, PNG and `export`) plus 18 parse-only seeds (4 320 more runs) produced no
abort at all. That is evidence, not a guarantee: the decoder is ~100 000 lines of C over
attacker-controlled offsets and lengths, so treat the risk as still present.

The sweep also found that a corrupt drawing could make the *renderer* (not the parser)
attempt a multi-gigabyte allocation and abort on the failure. That one was above the FFI
boundary and is fixed -- see "Every number the renderer takes from the file is bounded"
above.

**A one-byte DXF corruption can still cost eleven gigabytes, inside the decoder.** Re-running
the sweep against the fixed renderer turned up `lib/libredwg/test/test-data/2018/Leader.dxf`
with byte 8479 changed from `0x65` to `0xEF` -- the final `e` of `AcDbVisualStyle`, a class
name in the CLASSES section. `uncad_dxf_read_bytes` then peaks at **11.3 GB of working set
over 7 seconds** on a 143 KB file, and *succeeds*: it returns 0 with 182 objects, and the
summary, the SVG and the PNG that follow are all fine. Nothing above the boundary sees the
allocation happen, and nothing above it can refuse it; on a machine with less memory than
this one the allocation fails and the process dies with it. Bisected to that single byte
from a fuzzed file's 71 mutated offsets; measured with `GetProcessMemoryInfo` on the
release build, calling the shim directly so the cost is unambiguously the C decoder's.
There is no regression test for it -- a test that allocates 11 GB does not belong in a
suite -- which is the other half of why the out-of-process advice above is not optional.
The same sweep found a milder DWG case in the same place: a 31 KB `2000/Spline.dwg` with
32 flipped bytes reaches 634 MB before the decoder gives up and returns a clean
`ParseError` (critical read error 320), which is a transient cost rather than a hazard but
has the same shape and the same absence of anything this crate can do about it.

## Local patches to the vendored LibreDWG

`crates/libredwg-sys/vendor/libredwg/` is a copy of the submodule sources (see
`docs/ARCHITECTURE.md`, "Build"), and it now carries five local patches. Each is
marked in the source with a dated `uncad local patch` comment saying why, and each
is listed again in `crates/libredwg-sys/NOTICE.md` -- inside the crate, because that
is what a crates.io consumer receives and this file is not in the tarball
(GPLv3 §5(a)).
**`scripts/sync-libredwg-vendor.sh` deletes and recopies that directory, so re-applying
these patches is part of any submodule update.**

- **`src/dwg.c`** -- `dwg_find_tablehandle()`, `dwg_find_dicthandle_objname()` and
  `dwg_handle_name()` read a table record's `name` with `IS_FROM_TU_DWG()`, which is false
  for DXF and JSON input even when the record's name is stored as UTF-16 (see the section
  above). They now share one helper, `uncad_record_name_utf8()`, whose predicate
  `UNCAD_IS_TU_DWG()` mirrors what this crate's own shim does in `uncad_tv_to_utf8`. The
  patch deliberately stops there: the strings `in_dxf.c` stores through
  `dwg_add_u8_input()` (`DICTIONARY.texts`, `LTYPE.dashes[].text`) really are 8-bit for
  DXF input, so `dwg_find_dictionary()` and `dwg_find_dicthandle()` keep the original
  predicate. Upstream has the same gap; it is not reported there yet.
- **`src/common.c`** -- `cvt_TIMEBLL()` left `tm_wday`/`tm_yday`/`tm_isdst` uninitialized
  and let a corrupt date drive `tm_year`, `tm_mon` and `tm_hour` far out of range. Every
  caller passes the result straight to `strftime()` (`dec_macros.h`'s `FIELD_TIMEBLL` and
  the `DECODER` block in `header_variables.spec`, which runs at any log level), and
  Microsoft's UCRT `strftime` *validates* its `struct tm`: measured against
  `ucrtbase.dll`, a `tm_year` outside [-1900, 8099], `tm_mon` outside [0, 11], `tm_mday`
  outside [1, 31], `tm_hour` outside [0, 23], `tm_min` outside [0, 59] or `tm_sec` outside
  [0, 60] calls the invalid-parameter handler, which fail-fasts the process with
  0xC0000409. Windows reports that code as STATUS_STACK_BUFFER_OVERRUN even though nothing
  overran, which is why it looked like a stack smash. The patch zeroes the `struct tm` and
  clamps every field into those ranges. A real drawing's date already satisfies them, so
  no valid file's parse changes; the only visible difference is that the debug string for a
  TDINDWG/TDUSRTIMER *duration* longer than a day now caps its hour at 23 (a `LOG_TRACE`
  line this crate never enables, and `strftime` cannot print a larger hour anyway).
  `crates/uncad/tests/corrupt_dwg.rs` is the regression.
- **`src/in_dxf.c` and `src/dynapi.c`** -- a polygon mesh's vertices carry the subclass
  marker `AcDbPolygonMeshVertex`, which appeared nowhere in LibreDWG: the VERTEX_2D
  upgrade chain in `in_dxf.c` knows `AcDb3dPolylineVertex`, `AcDbPolyFaceMeshVertex` and
  `AcDbFaceRecord` only, and `dwg_name_subclasses[]` does not list it either. The object
  stayed a VERTEX_2D, failed the "is this subclass allowed in this object" check and took
  `goto invalid_dxf` -- `DWG_ERR_INVALIDDWG`, a *critical* error, so **the whole file was
  refused**. One POLYLINE written by REVSURF, RULESURF, EDGESURF or `ezdxf.add_polymesh()`
  cost every other entity in the DXF. The patch adds the missing spelling to the upgrade
  chain (to VERTEX_MESH, which is where the neighbouring branch already sends a polygon
  mesh's vertices) and to VERTEX_MESH's subclass list.
  `crates/uncad/tests/fixtures/polyface_mesh_r2000.dxf` is the regression.
- **`src/common_entity_data.spec`** -- for an R2004+ entity whose ENC flag has both `0x80`
  (an inline RGB follows) and `0x20` (a transparency follows), the spec read the two BLs
  in the wrong order, so the colour landed in `alpha_raw` and the transparency in `rgb`.
  LibreDWG's own standalone `bit_read_ENC` (`src/bits.c`) already reads rgb first; only
  this spec, and the encoder that shares it, had them the other way round. Measured on
  `test-data/2004/HatchG.dwg`, whose HATCH `29F` (flag `0xa0`) sits inside LWPOLYLINE
  `28D` (flag `0x80`, one BL, unambiguous): with alpha first the HATCH read
  `alpha_raw = 0xc21ae464` -- byte for byte the LWPOLYLINE's `rgb` -- and
  `rgb = 0x020000e5`, which has exactly the `alpha_type << 24 | alpha` shape every
  flag-`0x20` entity in the corpus shows. Only the `0xa0` combination changes: with one of
  the two bits set there is a single BL and the order cannot matter.

## Fixed: a path with non-ASCII characters could not be opened on Windows

`parse()` used to pass the path to LibreDWG, whose `fopen()` call the MSVC runtime
resolves in the ANSI code page; `parse("한글경로/도면.dwg")` failed with
`DWG_ERR_IOERROR` (4096) while the same file parsed from an ASCII path. The file is now
read in Rust and decoded from memory (`parse_bytes`), tested in
`crates/uncad/tests/read_paths.rs`.

## Fixed: SPLINE control points read at the wrong stride

Rendering 9 real files to SVG and reviewing the screenshots turned up nonsensical
diagonals crossing two SPLINE-heavy drawings end to end. The cause was
`SplineControlPoint` in `crates/uncad/src/dynapi.rs` being defined without the leading
`parent` pointer that `dwg.h`'s `Dwg_SPLINE_control_point` has (pointer + `x,y,z,w`, 40
bytes), so it measured 32. `get_array_field` walks `ctrl_pts` at `size_of::<T>()` intervals,
so from the second control point on it read at a progressively wrong offset, mixing the
tail of one element with the head of the next element's `parent` pointer. SPLINEs with fit
points were unaffected, since rendering prefers those -- only control-point-only SPLINEs
showed it, which is why it went unnoticed for a while.

The same bug explains a second, seemingly unrelated symptom: coordinates serializing as
absurdly long strings of leading zeros (a pointer bit-pattern read as `f64` often lands in
the subnormal range, and Rust's `f64` `Display` never switches to scientific notation).
The `clean()` helper in `svg.rs` still guards that at the formatting level, but the real
cause was the layout here.

`get_array_field`'s debug assertion cannot catch this class of bug: it compares the size
of the *pointer-to-array* field (8 bytes, always matching), never the element stride. A
compile-time assertion against clang's real `sizeof()` (40 bytes) was added instead, the
same pattern the hand-written HATCH structs use. **When adding another struct read as a
stride-based raw array, never omit a `parent` back-pointer from the C field list.**

## HATCH pattern fill

The same bindgen struct-codegen failure as `Dwg_HATCH_Path`/`PathSeg`/`ControlPoint`
recurred for `Dwg_HATCH_DefLine` (the pattern definition line: `angle`/`pt0`/`offset`/
`dashes`), for a slightly different reason: allowlisting `DefLine` directly generates fine
on its own, but its `parent` field is declared `struct _dwg_entity_HATCH *`, which forces
bindgen to materialize a real (non-opaque) `_dwg_entity_HATCH`, and *that* is what fails.
Nothing else allowlists `_dwg_entity_HATCH` -- HATCH's fields are all read through
dynapi's `void*`. The fix is the same as before: blocklist and hand-write the struct,
leaving `parent` as `*mut c_void` so `_dwg_entity_HATCH` is never pulled in, with a
compile-time assertion against clang's real `sizeof()` (64 bytes).

**Rendering**: one SVG `<pattern>` element per defline, accumulated into `<defs>` and
emitted once by `to_svg`, with the HATCH boundary path filled via `fill="url(#...)"`.
Clipping is delegated to SVG's own fill mechanism rather than hand-rolled polygon
clipping, so a boundary with several loops or islands keeps working through the existing
`fill-rule="evenodd"` path. Two deliberate simplifications (see
`render_pattern_line` in `crates/uncad/src/svg/hatch.rs`): the component of `offset`
parallel to the line direction, used for brick-style staggering, is ignored and only the
perpendicular spacing is honored; and the line is drawn at the center of its tile rather
than exactly on `base_point`, so `<pattern>`'s default tile-edge clipping cannot cut it in
half. Being half a tile out of phase is immaterial for an infinitely repeating pattern.

**A real bug found and fixed here**: the first implementation followed standard DXF
pattern-fill documentation, which describes group 52 (`angle`) and 41 (`scale_spacing`) as
applying on top of the group 78 definition-line data, and multiplied `pattern_angle`/
`pattern_scale` back into each `HatchPatternLine`. Rendering a real file showed no pattern
at all -- hatches inside small door and furniture symbols came out empty. Investigation:
a HATCH whose `pattern_angle` was exactly 90 degrees had a defline whose own `angle` was
also exactly 90, which is only possible if LibreDWG's parsed defline data already has
52/41 applied (a 0-degree source pattern plus 90 would have given 180). The numbers agreed:
multiplying a `pattern_scale` of 60 into a defline's ~6.5-unit spacing gives ~390 units,
while the shape being filled was only ~90 units across, so not a single line fell inside
one tile. Removing the reapplication entirely, and using the defline's
`angle`/`base_point`/`offset`/`dash_pattern` as final values, made dense crosshatch and
tile patterns appear correctly on the same file. The now-unused `pattern_angle`/
`pattern_scale` fields were dropped from `HatchEntity`. The lesson generalizes: **do not
take the DXF spec document at face value -- check what LibreDWG actually parses, against a
real file.**

**Still unverified** beyond the two deliberate simplifications above: whether
`pattern_type` (0 = user-defined, 1 = predefined, 2 = custom) actually changes how defline
data should be read. Measured: all ~2500 HATCHes across the 9 spot-check files are
`pattern_type=1`, with 0 and 2 never appearing, so there has simply been no case that
would distinguish them.

## HATCH gradient fill (unverified)

`is_gradient_fill` was `0` for all ~2500 HATCHes across the 9 spot-check files: not one
contains a gradient fill. The implementation below therefore rests on `dwg.h`'s field
comments and general DXF knowledge rather than comparison with an actual AutoCAD
rendering -- the same risk level as the other experimental types.

**bindgen**: `Dwg_HATCH_Color` (one color stop: `shift_value` + `Dwg_Color`) hits exactly
the same `parent: struct _dwg_entity_HATCH *` cascade as `Dwg_HATCH_DefLine`, and is
handled the same way (blocklist, hand-write, assert clang's `sizeof()` of 64 bytes).

**Rendering**: `gradient_name` (`SPHERICAL`/`HEMISPHERICAL`/`CURVED`/`LINEAR`/`CYLINDER`)
collapses to a binary choice -- the two spherical names become an SVG `radialGradient`
(close to AutoCAD's center-out look), everything else including unrecognized names becomes
a `linearGradient`. `CURVED` and `CYLINDER` are directional but not radial in AutoCAD too,
so a linear approximation misses the curvature but is still closer than a radial one. For
the two stops: with `single_color_gradient` off, `colors[]` is sorted by `shift_value` and
the ends are used; with it on (only one color is stored), the second stop is that color
blended toward white by `gradient_tint` (`color::tint_toward_white`, a plain per-channel
linear blend -- whether AutoCAD uses the same formula is unverified). `gradient_shift`
(DXF 461, the "Centered" option) is not applied, the same kind of deliberate
simplification as ignoring `HatchPatternLine.offset`'s parallel component.

## MLINESTYLE parsing (unverified)

MLINE used to be approximated with a single centerline because MLINESTYLE was not parsed.
Now the MLINE's `mlinestyle` handle is resolved and looked up in `Tables::mlinestyles`
(MLINESTYLE name -> list of per-line offsets), the offsets are scaled by the entity's own
`MLSCALE` (`MLineEntity::scale`, DXF 40), and each vertex's
`point + miter_direction * offset * scale` gives a true offset polyline. The scale is what
turns *style* units into drawing units, and setting it is how MLINE is normally used: a
200 mm wall is the STANDARD style (offsets +-0.5) at scale 200. Until 0.3.0 it was dropped,
so every multi-line came out exactly one unit wide whatever the drawing said. The entity's
`justification` (DXF 70) is still ignored -- the offsets are used as the style stores them,
i.e. as if the justification were "zero". The extent an MLINE contributes is measured from
the offset lines it draws, not from its centerline, so the crop, the frames and the
package's `bbox` see the width of the wall. `miter_direction` is a
vector LibreDWG has already computed with the miter angle applied, so this is one scalar
multiply, no trigonometry. When the style cannot be found (empty handle, failed
resolution) it is treated as a single `offset = 0.0`, which reproduces the old
centerline rendering exactly through the same `mline_offset_points` function, with no
separate branch.

**bindgen**: `Dwg_MLINESTYLE_line` (`offset`/`color`/`lt_index`/`lt_ltype`) hits the same
cascade through `parent: struct _dwg_object_MLINESTYLE *`, handled the same way (hand
written, clang `sizeof()` of 80 bytes asserted).

**Measured**: none of the 9 spot-check files contains an MLINE, so like MULTILEADER this
has never been compared with a real AutoCAD file or rendering. The offset arithmetic
itself is covered by two unit tests (a zero offset equals the centerline, a nonzero one
displaces exactly along `miter_direction`), but that only confirms the arithmetic, not
that using LibreDWG's `miter_direction` this way matches AutoCAD's real MLINE geometry. It
is a reasonable reading of `dwg.h`'s field names and semantics, not a confirmed fact.

## 3DSOLID SAB conversion runs on a copy

`acis.rs` used to call LibreDWG's `dwg_convert_SAB_to_SAT1` directly on the live entity to
get SAT text for the wireframe. That function converts in place: `version` 2 -> 1,
plaintext SAT into `encr_sat_data`, `acis_data` left as SAB bytes. Since `parse()` reads
every solid twice (`convert_entities`, then `convert_tables`'s walk over block records),
the second read took the `version != 2` branch, parsed binary SAB as SAT text, and lost
the wireframe -- present in `entities`, missing from
`tables.block_records["*Model_Space"]`. With the write path that existed at the time, the
same mutation reached the encoder and corrupted every solid on the way back out (measured
on `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`, 58 SAB solids). The
`uncad_3dsolid_sab_to_sat_text` shim in `libredwg-sys` now converts on a shallow copy and
returns only the text, so `parse()` never touches the `Dwg_Data` at all.
`crates/uncad/tests/acis_sab.rs` guards this with the same file, checking that both walks
extract the same wireframe for every solid.

## No DWG/DXF writing

0.1.0's `CadDatabase::write_dwg`/`write_dxf`, `uncad::dwg_to_dxf`, `WriteError` and the
CLI's `-o x.dxf`/`-o x.dwg` were all removed. This project's scope is
"DWG/DXF -> model -> JSON/SVG/PNG". What was measured about the upstream encoder before
removal (stable only up to R_2004, 4 of 9 fixtures failing in `dwg_encode`, refusing to
overwrite an existing file, promoting an R2007 source to R2010, `dwg_write_dxf` converting
SAB solids to SAT1 in place) lives in this section's earlier versions in git history. The
C build still includes `USE_WRITE` and the encoder sources -- see `docs/ARCHITECTURE.md`,
"Model".

## Thread safety

See `docs/ARCHITECTURE.md`. The `uncad` crate is safe to call from multiple threads; using
`libredwg-sys` directly means serializing the calls yourself.

## What the tests actually verify

This section is the single list of what is actually verified. `samples/README.md` only
explains why that directory is gitignored and links here. `docs/ARCHITECTURE.md`'s "Test
layout" covers where a new test belongs. The counts below are what
`cargo test --workspace -- --list` reports at 0.3.0; regenerate them from that command
rather than editing them by hand.

`cargo test --workspace` runs 382 tests (381 of them by default; `corpus_sweep` is
listed but `#[ignore]`d). 158 of them are `uncad` unit tests, by module:
`svg*.rs` 50 -- `svg.rs` 21 (HATCH edge approximation, stroke-width substitution,
block-transform composition, MLINE offsets, TEXT/ATTRIB anchoring and rotation,
non-finite coordinate defense, block-reference recursion blowup), `svg/infinite.rs` 16
(clipping a RAY and an XLINE to a viewBox: the four edges, a line that misses the window,
the direction a RAY keeps, the block matrix folded in), `svg/format.rs` 7 (number and
string formatting) and `svg/hatch.rs` 6 (pattern and gradient fills) --
`color.rs` 14 (ACI/BYLAYER resolution against the vendored palette, the
white-background normalization and the gradient helper `tint_toward_white`), `geom.rs` 11 (OCS to world, bulge arcs, polyline
length/area/bounds, the bounded arc-bounds walk, `is_sane_angle`), `dimension.rs` 10
(the formatter, the DIMSTYLE zero rule, the measurement sentinel), `crop.rs` 9 (the
outlier rules, the header candidate, padding, lattice snap, `detached_groups`),
`text.rs` 8 (the `%%` and `\S` decoders, the 0.6-em box estimate), `convert.rs` 8 (HATCH
gradient color resolution, stop ordering, `gradient_name` classification, and which
`Dwg_Color` fields mean "this entity states an RGB"), `export.rs` 7,
`json.rs` 6, `acis.rs` 6 (SAT record
parsing, pointer resolution, wireframe extraction), `png.rs` 5 (SVG -> PNG size, scaling,
errors, the bundled face's cap height, plus the `circle.dwg` pipeline),
`tables.rs` 4 (the LAYER TRUECOLOR 256-sentinel fallback), `limits.rs` 3 (the
bounds a corrupt file is held to, and how they are reported), `visibility.rs` 3 and
`header.rs` 3. Most are pure-function tests verifiable with synthetic data, which makes
them genuinely useful regression guards: whether `crop::outliers` sets aside the 3256x
INSERT and nothing else, or whether `parse_sat_records` really stops at the
`End-of-ACIS-data` marker, is decidable without a DWG file at all.

**Real-file tests**: 173 across the 23 integration files in `crates/uncad/tests/`,
plus 45 in `uncad-cli`'s `documented_invocations.rs` and 6 in its
`release_invariants.rs`. `png.rs`'s `to_png_renders_a_real_dwg_to_a_valid_png` runs the full
`parse()` -> `to_svg()` -> `to_png()` pipeline against one real DWG
(`lib/libredwg/test/test-data/2000/circle.dwg`, committed as part of the git submodule,
unlike `samples/`; the build uses the vendored copy, so the submodule is a test-only
precondition). 19 of the 23 integration files read that same corpus:
`export.rs` (27: every file the design lists, the overview budget, the tile grid per
level, sidecar affines that round-trip, the 32 KB sidecar cap, records pointing only at
written tiles, byte-identical output on a second run, a re-export clearing the previous
package, the padding option, a drawing far from the origin, a drawing that is one point,
nested attributes, a sidecar's layer list, an outline too big to test, and that record
building does not grow with the square of the entity count), `dimensions.rs` (10,
the RADIUS/DIAMETER/ANGULAR_3POINT kinds and the stored-measurement rule included),
`png_output.rs` (10), `sheets.rs` (8), `codepage.rs` (8), `polyline_geometry.rs`
(8), `header.rs` (7), `read_paths.rs` (6, Korean directory names),
`crop.rs` (5), `dxf_pipeline.rs` (5: DXF parse/render, a JSON round trip through
`serde_json::from_str` and `PartialEq`, two parses agreeing and producing identical JSON
with `CadDatabase` being `Send + Sync + Clone`, and an error rather than a panic on
garbage input), `visibility.rs` (5), `text_fields.rs` (5), `polyline_closed.rs` (3),
`r2007_dxf_handles.rs` (3: an R2018 DXF resolving the same layers, block names and hidden
entities as its DWG twin, and no R2007+ corpus DXF leaving an entity without a layer),
`corrupt_dwg.rs` (3: a one-byte corruption of a header date, a truncated file and an
attribute chain that points back at its own INSERT return an error or a bounded walk
instead of killing the process), `limits.rs` (10: the one-byte corruption that used to
ask for 12 GB, a block that references itself, a block chain past the depth cap, the
whole-document and per-entity SVG budgets, a polyline past the point cap, a hatch whose
pattern tile dwarfs its shape, an entity at 1e150 and an arc of absurd radius -- each
asserting a bound or a screened entity, never a crash),
`acceptance.rs` (1: five agent questions answered from an exported package alone, see
`docs/EVAL.md`), `acis_sab.rs` (1: a SAB-solid file yielding the same wireframe in
`entities` and in `tables.block_records`) and `corpus_sweep.rs` (1, `#[ignore]`d: parses
and renders all 208 corpus files and fails on any panic -- `docs/EVAL.md` records what it
found). The other four run against this project's own committed DXF fixtures
(`crates/uncad/tests/fixtures/`, 18 files written by `make_fixtures.py`): `fixtures.rs`
(19), `block_transforms.rs` (4), `sheets_compositing.rs` (4) and `control_chars.rs` (2).
`uncad-cli`'s `tests/release_invariants.rs` (6) reads the repository rather than a
drawing: that every published crate directory holds the GPLv3 text (`cargo package`
reaches no further than the crate directory, so the repository root's copy never travels),
that the bundled font and the vendored LibreDWG keep their own licence beside them, that
nothing calls the vendored copy unmodified while it carries `uncad local patch` markers,
that `uncad --version` prints the version in the manifest, that README, this file and
`docs/ARCHITECTURE.md` state the integration-test counts the tree actually has, and that
`ci.yml` triggers on every push as the Clippy section below claims. Its numbers are
derived from the tree, never copied from the documents.
`uncad-cli`'s `tests/documented_invocations.rs` (45) runs every call the README and
`--help` document against the real binary -- the `export` subcommand and each of its
options included -- and the parser's refusals (an unknown option, a second positional, a
missing value, a flag of the other command); `--scale`, `--fit`, `--ppu`, `--stroke`,
`--bg`, `--space`, `--crop`, `--no-trim`, `--include-hidden`, `--fonts` and `--pretty`
are each checked for actually changing the result. All of them assert properties rather
than pinned expected values. The 6 `json.rs` unit tests build one instance of every
`Entity` variant and check that the JSON `type` tag matches `type_name()`, that HATCH
path and edge tags are right, that a round trip holds, that non-finite floats become
`null` and do not come back, and that a wrong or missing `type` tag is an error rather
than a panic.

**Still missing**: the corpus sweep proves nothing panics and nothing regresses in entity
counts run to run, but not that the numbers are *right* -- there is no comparison against
an independent reference, and no byte-exact golden package is checked in (`docs/EVAL.md`,
section 4). `samples/` is entirely gitignored (a deliberate choice, so anyone can drop any
file in without license clearance), so CI has nothing committed to read beyond the
LibreDWG corpus and this project's own fixtures. See `samples/README.md`. Closing that gap
means committing files with a clear license and verifying expected values against an
independent reference rather than against this project's own output.

## Clippy

A separate `lint` job in `.github/workflows/ci.yml` runs `cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings` on every push and pull request, so
whether the tree is clean is tracked automatically. "Every push" means every branch, not
just `main`: the workflow's trigger was `push: branches: [main]` until 0.3.0, which left a
long-lived release branch with no automated coverage at all until someone opened a pull
request for it. A `concurrency` group keyed on `github.head_ref || github.ref` with
`cancel-in-progress` keeps the wider trigger cheap -- a branch's push run is superseded as
soon as its pull-request run, or the next push, starts. A separate `security-audit` job runs
`cargo audit` against `Cargo.lock` for dependency vulnerabilities (without submodules --
the vendored C sources are not part of the Rust dependency graph). The
bindgen-generated `bindings.rs` in `libredwg-sys` is regenerated on every build rather
than written by hand, and the handful of harmless lints it raises (`useless_transmute`,
`missing_safety_doc`, `ptr_offset_with_cast`, `unsafe_op_in_unsafe_fn`, `manual_div_ceil`) are allowed at
crate level in `crates/libredwg-sys/src/lib.rs` so `-D warnings` does not fail on
generated code. New warnings in hand-written code still fail CI.

## Windows/Linux

Builds and tests pass on Linux (`x86_64-unknown-linux-gnu`), verified locally in a Docker
`rust:latest` image with `libclang-dev` and re-verified on every push (any branch) by the
`ubuntu-latest` job in `.github/workflows/ci.yml`. A `windows-latest` job covers the MSVC
platform this project is actually developed on. Three real bugs were found and fixed while
getting Linux working:

- `config.h` hardcoded `SIZEOF_WCHAR_T` to the Windows value (2), so `BITCODE_TU` resolved
  to the wrong pointer type on Linux.
- Several LibreDWG functions' real C return type (`int`) diverged from the bindgen-inferred
  type of the related enum, differently per platform -- see `docs/ARCHITECTURE.md`,
  "Cross-platform enum width".
- Blocklisting `Dwg_MLINE_vertex` was not enough on Linux, where bindgen additionally
  generated `Dwg_MLINE_line` referring to the blocklisted type (a compile error). Not
  reproducible on Windows/MSVC.

All three were latent portability problems in the upstream C headers or in bindgen itself,
which MSVC simply happened not to expose.

**32-bit targets are not supported, and now fail at build time.** `config.h` fixes
`SIZEOF_SIZE_T` at 8 (64-bit) with no platform branch, unlike its neighbor
`SIZEOF_WCHAR_T`. That value feeds LibreDWG's `MAX_MEM_ALLOC` allocation-size guard
(`bits.h`) and several word-aligned fast-path reads in `bits.c`, so on a 32-bit target
(`i686-*`, `wasm32-*`, `arm-*`) the assumption breaks, leading to misaligned reads and a
far too permissive allocation-size gate against attacker-controlled DWG size fields -- the
hardest kind of bug to diagnose, since it compiles and then quietly misbehaves. This
project has only ever been built and verified for x86_64, with nothing enforcing it.
`crates/libredwg-sys/build.rs` now checks `CARGO_CFG_TARGET_POINTER_WIDTH` at the start of
the build and `panic!`s with a clear message if it is not 64: a build failure beats
compiling silently with wrong values. Real 32-bit support would mean adding a platform
branch to `config.h` the way `SIZEOF_WCHAR_T` has, and actually building and testing on
such a target.
