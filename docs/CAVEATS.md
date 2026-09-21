# Known limitations and caveats

## Entity type coverage

`parse()`/`to_svg()` support: LINE, CIRCLE, ARC, ELLIPSE, LWPOLYLINE, TEXT, POINT, SOLID,
TRACE, RAY, XLINE, INSERT (including recursive block-reference rendering), ATTRIB, ATTDEF,
VIEWPORT, 3DFACE, SPLINE, MTEXT, POLYLINE_3D, POLYLINE_2D, DIMENSION, HATCH, 3DSOLID,
LEADER, MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE, WIPEOUT and
LIGHT. Details worth knowing:

- **DIMENSION** folds all 7 subtypes (ALIGNED, ANG2LN, ANG3PT, DIAMETER, LINEAR, ORDINATE,
  ARC_DIMENSION) into one type. They share the `DIMENSION_COMMON` layout, including the
  `block` handle to the cached-geometry block that is what actually gets drawn.
- **HATCH** handles boundary paths that are polylines as well as lists of
  line/arc/ellipse/spline edges. Pattern fills are really reproduced, by reading
  `Dwg_HATCH_DefLine` and tiling an SVG `<pattern>` (see "HATCH pattern fill" below);
  solid fills are painted as a translucent color; gradient fills are approximated with
  SVG's `linearGradient`/`radialGradient` (unverified, see "HATCH gradient fill").
- **LEADER** draws only the polyline through its vertices plus an optional arrowhead at
  the first one. Spline paths and text-box size are not in the model, because nothing
  renders them.
- **TRACE** reuses `SolidEntity` (the two have the same four corners, in the same order) and
  is filled the same way.
- **POLYLINE_2D** reuses `LwPolylineEntity` and renders through exactly the same code path
  as LWPOLYLINE, the same way `Entity::XLine` reuses `RayEntity`.
- **3DSOLID**, **REGION** and **POLYLINE_PFACE** render as isometric wireframes, which is
  an approximation, not a reading of the B-rep. A solid whose ACIS data cannot be read or
  converted is reported as unsupported.
- **MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE, WIPEOUT, LIGHT**
  are all **experimental** -- see the next section.

**The list above is the whole contract.** A type that is not on it becomes
`Entity::Unknown` at the `parse()` stage -- never dropped silently, and `Unknown` keeps the
real DXF name, so a CLI summary still counts it under that name -- and `to_svg()` reports
it through `unsupported_types` (sorted by name, so the report is the same on every run).
A listed type can end up there too when a particular entity gives the renderer nothing to
draw, e.g. a DIMENSION without its cached-geometry block.

Being off the list says nothing about whether the type has geometry. POLYLINE_MESH, IMAGE
and HELIX, for instance, all have a shape and none of them is covered; that is simply work
that has not been done. Do not read the list as "everything with a shape".

**ACAD_PROXY_ENTITY is the one type that will stay unsupported.** It is the proxy
representation of a custom entity from another program, so it has no fixed geometry to
render at all: just `proxy_id`, `class_id` and serialized entity bytes, with no
coordinates or shape. Leaving it as `Unknown` *is* the accurate representation.

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

**TOLERANCE** renders exactly like ATTRIB/TEXT (position plus text), except that
`text_value` still carries GD&T feature-control-frame codes (`%%v` and similar), which
this project does not parse or strip (there is no dedicated stripper the way MTEXT has
`strip_mtext_formatting`). Readable, but not real GD&T symbols.

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

Which entities the importer keeps also depends on the declared version, not only on the
content: the corpus R2000 file `2000/entities-2d.dxf` reads as 12 entities, and the same
bytes with `$ACADVER` rewritten to `AC1018` (R2004) read as 14 -- an ATTDEF and an ATTRIB
appear that the R2000 pass drops without a message. Nothing on this side can tell that
they were dropped.

### DXF saved as R2007 or later is refused with an explicit error

`parse()` returns `Err(ParseError::UnsupportedDxfVersion)` for a DXF whose `$ACADVER`
is `AC1021` (R2007) or later -- R2007, R2010, R2013 and R2018 files, which is what
current CAD software writes by default. The decision is made from the file's own HEADER
section before LibreDWG sees it. R2000/R2004 DXF (`AC1015`/`AC1018`) and older, DXF files
without a `$ACADVER` (pre-R10), and every DWG version are unaffected. Binary DXF is not
inspected and goes to LibreDWG as before.

Why refuse rather than read: LibreDWG's DXF importer stores strings as UTF-16 for R2007+
input, but its own text accessor treats imported data as 8-bit. Every name therefore comes
back cut off after its first character -- `*Model_Space` arrives as `*` -- and this crate
finds the entities by looking up the model- and paper-space block records by name. The
entities are in memory (the block record that arrives as `*` has them); they are just
never matched, and LibreDWG reports no error. Left alone, that surfaced as `Ok` with zero
entities, indistinguishable from an empty drawing.

Correcting the width on this side is not enough either. The importer resolves layer and
block names through that same accessor while it builds the drawing, so those lookups have
already failed by the time the data gets here: with the width corrected, the corpus files
do yield their 72 entities, but 65 of them without a layer, every INSERT and DIMENSION
without its block, and MTEXT content garbled (the importer stores that one field 8-bit). A
drawing that looks read and is quietly missing that much is worse than an obviously empty
one, and an obviously empty one is worse than an error. The fix belongs in LibreDWG's
accessor, or in reading DXF without LibreDWG.

What to do: save the drawing as R2004 DXF or as DWG (any version). A test walks every DXF
in the LibreDWG corpus and checks that exactly the R2007+ files are refused; a second one
feeds the same minimal drawing with two `$ACADVER` values and requires one refusal and one
successful read.

## What LibreDWG reported but did not fail on

`dwg_read_file`/`dxf_read_file` return a bit set. Bits at or above `DWG_ERR_CLASSESNOTFOUND`
make `parse()` fail with `ParseError::Critical`; the bits below it used to be discarded. They
are now carried in `CadDatabase::read_diagnostics` (the raw bits and their dwg.h names), and
the CLI prints them as a warning. Every example DWG in the LibreDWG corpus comes back with
`UNHANDLEDCLASS` set, and `example_2018.dwg` with `UNHANDLEDCLASS | VALUEOUTOFBOUNDS`; the
R2000 DXF from the same corpus comes back clean. What the bits mean for the result is
LibreDWG's to say -- `UNHANDLEDCLASS` in particular means objects of a class it did not know
were skipped, and nothing else in the model shows that they existed.

Two more places report what used to vanish: `ToSvgResult::empty_blocks` names the blocks an
INSERT referenced that drew nothing (an empty definition, or one whose every entity was left
out), and `Solid3DEntity::skipped_edges` counts the ACIS edges that could not be turned into
wireframe segments. Neither is an error; both are the difference between "empty" and "not
read".

## The polyline "closed" flag

`dwg.h`'s field comment documents bit 512 of `flag` as "closed", but the rendering logic
checks bit 1 (`flag & 1`), the standard DXF group-70 convention -- see the
`POLYLINE_CLOSED_FLAG` constant in `crates/uncad/src/convert.rs`. With no AutoCAD
reference available to settle which reading is correct, the DXF convention was kept rather
than "corrected" on a guess.

## MTEXT rotation is always 0

`dwg.h` says the `x_axis_dir` field "defines the rotation", and deriving an angle from it
(`atan2(x_axis_dir.y, x_axis_dir.x)`) looks technically more accurate. Without a verified
reference to confirm it, the value stays fixed at `0` rather than diverging on an
unverified guess.

## There is no single ACI colour table

`ACI_PALETTE` in `crates/uncad/src/color.rs` maps colour index 1-255 to RGB, and which RGB
values are "right" has no one answer. AutoCAD's *displayed* colours depend on the
drawing-area background, so a table captured from a dark model space and one captured from
a white sheet disagree with each other; a third-party reader may carry a table that matches
neither. Comparing this crate's table against the one LibreDWG keeps for its own JSON export
(`rgb_palette` in `src/dwg.c`), 222 of 256 entries differ -- the two agree only on the
primaries and a handful of others, starting to diverge at index 8. That is not evidence
either is wrong; LibreDWG does not render, so its table answers a different question.

What this crate uses is the table that has been published unchanged for decades and is the
one mirrored by the widely used ACI charts. The
`aci_palette_matches_the_published_table` test pins fifteen entries to it -- the first
primary, the two greys at index 8 and 9 where implementations tend to split, the head of
the red ramp, and the grey ladder at 250-255 -- so a re-import from some other project's
table cannot land silently.

Two entries are not colours: index 0 is a placeholder and index 256 is the BYLAYER slot.
Both are `0`, which is what lets `aci_to_hex` stay total over `0..=256` without a branch --
and what makes the unresolved-layer case below come out black rather than panicking.

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
Now the MLINE's `mlinestyle` handle is resolved, looked up in `Tables::mlinestyles`
(MLINESTYLE name -> list of per-line offsets), and each vertex's
`point + miter_direction * offset` gives a true offset polyline. `miter_direction` is a
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

## File-based regression tests are few (broad real-file coverage is still missing)

This section is the single list of what is actually verified. `samples/README.md` only
explains why that directory is gitignored and links here. `docs/ARCHITECTURE.md`'s "Test
layout" covers where a new test belongs.

`cargo test --workspace` runs 86 tests. 64 of them are `uncad` unit tests: `color.rs` 11
(ACI/BYLAYER resolution and the gradient helper `tint_toward_white`), `acis.rs` 6 (SAT
record parsing, pointer resolution, wireframe extraction), `convert.rs` 6 (HATCH gradient
color resolution, stop ordering, `gradient_name` classification), `json.rs` 6, `svg*.rs`
28 (outlier-trim clustering, HATCH edge approximation, MTEXT formatting stripping,
stroke-width substitution, transform composition, HATCH pattern fill, MLINE offsets,
TEXT/ATTRIB rotation transforms, non-finite coordinate defense, block-reference recursion
blowup), `tables.rs` 3 (the LAYER TRUECOLOR 256-sentinel fallback), and `png.rs` 4 (SVG ->
PNG size, scaling, errors, plus the `circle.dwg` pipeline). Most are pure-function tests
verifiable with synthetic data, which makes them genuinely useful regression guards:
whether `dominant_cluster_box` picks the right cluster out of a synthetic set of boxes, or
whether `parse_sat_records` really stops at the `End-of-ACIS-data` marker, is decidable
without a DWG file at all.

**Real-file tests**: `png.rs`'s `to_png_renders_a_real_dwg_to_a_valid_png` runs the full
`parse()` -> `to_svg()` -> `to_png()` pipeline against one real DWG
(`lib/libredwg/test/test-data/2000/circle.dwg`, committed as part of the git submodule,
unlike `samples/`; the build uses the vendored copy, so the submodule is a test-only
precondition) and checks that a valid PNG comes out. That is a smoke test on one file, not
broad per-entity-type rendering accuracy. From the same corpus:
`tests/dxf_pipeline.rs` (5: DXF parse/render, a JSON round trip through
`serde_json::from_str` and `PartialEq`, two parses agreeing and producing identical JSON
with `CadDatabase` being `Send + Sync + Clone`, and an error rather than a panic on
garbage input), `tests/acis_sab.rs` (1: a SAB-solid file yielding the same wireframe in
`entities` and in `tables.block_records`), and `uncad-cli`'s
`tests/documented_invocations.rs` (16: every call the README documents, run against the
real binary; `--scale`/`--space`/`--no-trim`/`--pretty` are each checked for actually
changing the result, with `--no-trim` using a five-line DXF the test writes from group
codes itself). All of them assert properties rather than pinned expected values. The 6
`json.rs` unit tests build one instance of every `Entity` variant and check that the JSON
`type` tag matches `type_name()`, that HATCH path and edge tags are right, that a round
trip holds, that non-finite floats become `null` and do not come back, and that a wrong or
missing `type` tag is an error rather than a panic.

**Still missing**: broad end-to-end verification of `parse()`/`to_svg()`/`to_json()`
against real DWG/DXF files -- entity-count parity across many files, byte-level rendering
comparison and so on -- is not automated. `samples/` is entirely gitignored (a deliberate
choice, so anyone can drop any file in without license clearance), so CI has nothing
committed to read. See `samples/README.md`. Restoring that coverage means committing files
with a clear license and verifying expected values against an independent reference rather
than against this project's own output.

## Clippy

A separate `lint` job in `.github/workflows/ci.yml` runs `cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings` on every push and pull request, so
whether the tree is clean is tracked automatically. A separate `security-audit` job runs
`cargo audit` against `Cargo.lock` for dependency vulnerabilities (without submodules --
the vendored C sources are not part of the Rust dependency graph). A `licenses` job runs
`cargo deny check licenses sources` against the same file: this crate is GPL because it
links a GPL library, and the gate is there so a *second* copyleft component cannot arrive
through a dependency bump unnoticed. `deny.toml` allows only the permissive licenses the
graph actually uses and names the three crates of this workspace as the sole GPL
exceptions; it reads `Cargo.lock` too, so the same submodule caveat applies -- what
watches the vendored C side is the source-file drift detector in
`crates/libredwg-sys/build.rs`. The
bindgen-generated `bindings.rs` in `libredwg-sys` is regenerated on every build rather
than written by hand, and the handful of harmless lints it raises (`useless_transmute`,
`missing_safety_doc`, `ptr_offset_with_cast`, `unsafe_op_in_unsafe_fn`, `manual_div_ceil`) are allowed at
crate level in `crates/libredwg-sys/src/lib.rs` so `-D warnings` does not fail on
generated code. New warnings in hand-written code still fail CI.

## Windows/Linux

Builds and tests pass on Linux (`x86_64-unknown-linux-gnu`), verified locally in a Docker
`rust:latest` image with `libclang-dev` and re-verified on every push by the
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
