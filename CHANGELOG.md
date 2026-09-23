# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `libredwg-sys` reads a drawing from memory (`uncad_dwg_read_bytes`,
  `uncad_dxf_read_bytes`) -- LibreDWG's own file readers `fopen()` a byte string the
  MSVC runtime reads in the ANSI code page, so a non-ASCII path fails on Windows -- and
  exposes the file-header facts decoding a drawing's text and header needs:
  `uncad_dwg_version`, `uncad_dwg_from_version`, `uncad_dwg_from_dxf`,
  `uncad_dwg_numheader_vars` (how long a pre-R13 header is) and `uncad_dwg_template_read`
  (whether the section holding `$MEASUREMENT` was read), with bindings for
  `dwg_version_type` and `dwg_next_object`. Strings are not converted in C: `uncad`'s
  `TextDecoder` does that, and reports what it cannot.
- `uncad::parse_bytes(bytes, Format)` parses a drawing already in memory, and
  `uncad::Format` (`Dwg`, `Dxf`, with `Format::from_path`) says which it is. `parse()`
  now reads the file itself and decodes it from memory, so a path LibreDWG could not
  open -- any non-ASCII one on Windows, such as a Korean directory name, which failed
  with critical error 4096 -- parses; a file that cannot be read is the new
  `ParseError::Io`. `tests/read_paths.rs`.
- `uncad::parse_with_header` / `uncad::parse_bytes_with_header` return the drawing's
  `uncad::Header` beside the model: the format, `$ACADVER` and LibreDWG's release name,
  the codepage strings were decoded with, `$INSUNITS` (`Header::units()` gives the unit
  and its millimetre factor), `$MEASUREMENT`, `$LUNITS`/`$LUPREC`/`$AUNITS`/`$AUPREC`,
  the model and paper extents and limits, the `$DIM*` variables a dimension falls back
  on, `$LTSCALE`, `$TEXTSIZE` and `$CLAYER`. A variable the file does not state is
  `None` -- for a DWG by its version's header layout, for an ASCII DXF by what its HEADER
  section names -- never the zero or default LibreDWG's struct holds for it. The header
  is this crate's type, not the model's, which carries no header variables by design;
  `parse()` and `parse_bytes()` are unchanged and return the database alone.
  `tests/header.rs`.
- Every crate carries the GPLv3 text as its own `LICENSE`; `cargo package` never
  reaches the repository root's, so of the 0.2.0 tarballs only `libredwg-sys` had the
  text (as LibreDWG's own `COPYING`) and `uncad-cli` had no licence file at all.
  `libredwg-sys` also carries `NOTICE.md`, the modification notice for its vendored
  LibreDWG. `uncad-cli`'s `tests/release_invariants.rs` keeps both true.
- `libredwg-sys`'s build names libclang, and the command that installs it, when bindgen
  cannot find it, instead of bindgen's own message.
- A SPLINE carries what defines its curve: `degree`, `knots`, `weights` (empty when
  the file gives none -- every weight is 1), and the `closed` / `periodic` bits as
  `Option<bool>`. A spline stored by its fit points has no periodic bit, and no
  closed bit before R2013; those are `None`, not `false`.
- An MTEXT carries its `attachment` point (DXF 71): which of the text block's nine
  points the insertion point is. Without it the insertion point does not say where the
  text goes.
- Every entity carries `invisible` (DXF 60): the drawing hides it -- a dynamic block's
  hidden visibility states are the common case (5,499 of the test corpus's entities).
- An ELLIPSE carries its `extrusion` (DXF 210), the normal of its plane: a mirrored
  ellipse's parameters run the other way.
- A layer's state is read: `off`, `frozen`, `locked`, `plot` (DXF 290), `lineweight`
  (DXF 370, hundredths of a millimetre or -3 for the default) and the `linetype` it names.
  A DXF's plot flag and lineweight are `None` where its importer cannot tell a stated 0
  from an absent group, and a drawing older than R2000 states neither. See
  `docs/CAVEATS.md`, "Layer state: what a DXF cannot say".
- POLYLINE_MESH (a polygon mesh) is read, as `Entity::PolylineMesh`: the wireframe of its
  M by N grid, closed in either direction where the file says so. It used to arrive as
  `Entity::Unknown`. A polyface mesh whose vertices the DXF importer types `VERTEX_MESH`
  (they name the block record as their owner, the shape ezdxf writes) finds its vertex
  positions: `example_2000.dxf`'s and `example_r13.dxf`'s polyface had no edges where
  their DWG twins have six.
- The object coordinate system of CIRCLE, ARC, LWPOLYLINE, POLYLINE_2D, TEXT, ATTRIB,
  ATTDEF, INSERT, SOLID and TRACE is read: each carries its `extrusion` (DXF 210) and the
  planar ones their `elevation`, with the coordinates as the file states them. A
  mirrored entity used to be indistinguishable from an upright one. See
  `docs/CAVEATS.md`, "Object coordinate systems".
- LWPOLYLINE and POLYLINE_2D carry what runs between their vertices: `bulges` (with the
  sign the file wrote), per-vertex `widths` and `const_width`, empty lists when every
  segment is straight and every width the constant one. A DXF POLYLINE's default widths
  (its groups 40/41) are the widths of the vertices that state none, so a DXF and its DWG
  twin agree. See `docs/CAVEATS.md`, "Polyline bulges and widths are carried as stated".
- TEXT, ATTRIB and ATTDEF carry how they are placed beyond their start point: the
  horizontal and vertical justification (DXF 72, 73/74), the alignment point (DXF 11,
  only for a justified text), the width factor (DXF 41), the oblique angle (DXF 51,
  radians) and the text style they name (DXF 7). An MTEXT carries its reference width
  (DXF 41), the extents its writer measured (DXF 42/43, `None` when not stated) and its
  text style.
- ATTRIB and ATTDEF carry their `flags` (DXF 70): invisible, constant, verify, preset. An
  invisible attribute (a title block's hidden field, say) is no longer indistinguishable
  from a shown one.
- A DIMSTYLE carries the rest of what a dimension's displayed text depends on, each as an
  `Option` like the others: `arrow_size` (DIMASZ), `linear_unit_format` (DIMLUNIT),
  `zero_suppression` (DIMZIN), `rounding` (DIMRND), `angular_unit_format` (DIMAUNIT),
  `angular_decimal_places` (DIMADEC) and `fraction_format` (DIMFRAC). What to use where a
  style states nothing is the consumer's decision.
- An MLINE carries its `scale` (DXF 40), the factor its style's offsets are drawn at: a
  wall drawn 20 units thick in a style of unit offsets was drawn 1 unit thick.
- The drawing's layouts are read into `Tables::layouts`: every LAYOUT object -- a tab, the
  block it shows (`block_name`), its tab order and limits -- with the plot settings
  embedded in it (paper name and size, margins, plot origin, paper unit, rotation and the
  custom scale), read through LibreDWG's dynapi into the embedded `PLOTSETTINGS` struct.
- A VIEWPORT carries what it shows of the model: its `view` (centre and height in the
  view's own coordinates, target, direction, twist and lens length; `None` before R2000,
  whose viewports keep it in extended data), whether it is `on`, its `viewport_id` (a
  DXF's; the binary format stores none) and the layers frozen in it alone.
- An ordinate DIMENSION carries its `ordinate_axis` (DXF 70, bit 64): whether it measures
  its feature's x or y distance from the datum.
### Changed

- A dimension whose group 42 is `-1`, the value writers leave for a dimension they did not
  measure, reports `measurement: None` rather than `-1.0`, the way a `0` already did.
- A LEADER's `annotation_id` is a three-state `Ref<EntityId>`: `Resolved` names an
  entity of the drawing, `Unresolved` keeps the handle the file wrote (hex) when no
  entity answers to it, `Absent` is a leader that names nothing. The `Option` it
  replaces carried the first two as the same `Some`.
- A LEADER's `has_arrowhead` is `Option<bool>`; `None` where the flag cannot be read.
- `LightEntity::has_target` is gone and `LightEntity::light_type`
  (`Option<LightType>`: distant, point, spot) takes its place. `has_target` was not
  something the file states but a conclusion drawn from the type and two points; the
  type is what the file states, and whether a light aims at its target follows from
  it. Breaking for consumers reading any of the three fields.
- `HatchGradient` carries its stops as packed 24-bit RGB (`color1: u32`,
  `color2: Option<u32>`) plus the single-color `tint`, as the file states them,
  instead of two rendered hex strings. The parser no longer decides how a
  single-color gradient fades or whether white is flipped for a white background;
  those are a renderer's derivations. Breaking for consumers reading the two fields.

### Added

- `AttribEntity::tag` and `AttdefEntity::tag` (DXF 2): the name an attribute value
  answers to. A title block's values were readable but not which field each one filled.
- `CadDatabase::read_diagnostics`: the non-fatal problems LibreDWG reported while
  reading, as a list of warning names (`WRONGCRC`, `UNHANDLEDCLASS`, `VALUEOUTOFBOUNDS`,
  ...) in bit order; `read_diagnostics_from_libredwg_bits` is the decoder. They used to
  be discarded, so a file LibreDWG read while skipping objects it could not decode was
  indistinguishable from a clean read. Every corpus example DWG in fact comes back with
  `UNHANDLEDCLASS` set. The field is serialized with the model (`{"warnings":[..]}`) and
  defaults to "clean" when absent from older JSON; the CLI prints a warning when it is
  not clean.
- `ToSvgResult::empty_blocks` / `ToPngResult::empty_blocks`: names of blocks an INSERT
  referenced that contributed nothing to the image (empty definition, or nothing in it
  drawable). Previously such a reference vanished without trace. The CLI warns about them.
- `Solid3DEntity::skipped_edges`: how many ACIS edges of a 3DSOLID/REGION could not be
  turned into wireframe segments. A solid with no `wireframe_edges` and a non-zero count
  here was not read, not empty; the edges used to be skipped in silence.
- TRACE is read and drawn: `Entity::Trace`, which reuses `SolidEntity` the way `XLine`
  reuses `RayEntity`. It used to arrive as `Entity::Unknown`.
- All three crates declare `rust-version = "1.88"`. Until now the minimum was whatever
  happened to build. The value is measured (1.87 fails, 1.88 passes) and CI has an `msrv`
  job that checks the workspace with exactly the declared toolchain.

### Fixed

- **The vendored LibreDWG carries five local patches**, each marked `uncad local patch`
  in the source and listed in `crates/libredwg-sys/NOTICE.md` and `docs/CAVEATS.md`,
  "Local patches to the vendored LibreDWG". A DXF holding a polygon mesh is read instead
  of refused as a whole (critical error 2048: the importer did not know the mesh
  vertices' `AcDbPolygonMeshVertex` marker). A corrupt header date no longer ends the
  process from inside the C library (0xC0000409 on Windows, from `strftime`). The
  importer compares an R2007+ DXF's table-record names decoded, so its layer and block
  lookups no longer stop at the first character: without that patch `example_2018.dxf`
  reads with 65 of its 72 entities on no layer (`tests/r2007_dxf_handles.rs`). One more is
  not visible in this crate's output yet: an R2004+ entity carrying both a true colour and
  a transparency no longer has the two swapped in the library's fields (this crate reads a
  true colour only under the TRUECOLOR method, which the entities measured do not have).
  Before R2007+ DXF was read, the 208 corpus drawings gave byte-identical JSON with and
  without the patches. `build.rs` refuses to build when a patch's marker has gone missing,
  which a re-vendor through `scripts/sync-libredwg-vendor.sh` would otherwise do in
  silence.
  process from inside the C library (0xC0000409 on Windows, from `strftime`). An R2004+
  entity carrying both a true colour and a transparency no longer has the two swapped in
  the library's fields (`2004/HatchG.dwg`'s HATCH 29F: `0x1ae464`, not `0x0000e5`). One
  more is not visible in this crate's output yet: the importer compares an R2007+ DXF's
  table-record names decoded, so its layer and block lookups no longer stop at the first
  character (such a DXF is still refused). When the patches landed, the JSON output of
  the 208 corpus drawings was byte-identical with and without them; the colour-order one
  shows since true colours are read as the file states them (below). `build.rs` refuses
  to build when a patch's marker has gone missing, which a re-vendor through
  `scripts/sync-libredwg-vendor.sh` would otherwise do in silence.
- **Every vertex of a POLYLINE_2D/3D is read.** LibreDWG's point accessors end their walk
  one vertex early on every file older than R2004, so the last vertex was dropped: a closed
  square came back a triangle, a two-vertex arc a single point. The vertices now come from
  the polyline's own subentity chain; all 22 POLYLINEs of the corpus DXFs and the
  fixtures whose vertices can be counted in the file match that count (the accessors had
  20 of them one short). A polyline's VERTEX records are no longer reported as entities of
  their own either -- 62 `Unknown` VERTEX entities in seven pre-R13 DXFs. See
  `docs/CAVEATS.md`, "Polyline vertices come from the polyline's own chain".
- **An entity's true colour is the one the file states.** It was read only when the
  colour's method said TRUECOLOR: an R2004+ DWG never sets the method (the RGB comes under
  the colour's `0x80` flag), so no DWG entity reported its true colour, while the DXF
  importer sets it for a plain group 62 with an RGB taken from its own palette, so a DXF
  entity with only an ACI index reported an RGB the file never wrote. The flag is read
  first, and a DXF RGB that is the one the library synthesises for the entity's index is
  not a true colour. See `docs/CAVEATS.md`, "An entity's true colour is what the file
  states".
- `MTextEntity::rotation` is the direction of the text's X axis instead of a constant `0`,
  which reported rotated multi-line text as horizontal.
- A two-line angular dimension's `definition_point` (DXF 10) is read instead of reported as
  not stated: the library keeps it under the field name `xline2end_pt`. Read from a DXF,
  the same dimension had its groups 10 and 16 (`definition_point` and `points.arc`)
  exchanged: LibreDWG's DXF importer fills those two fields by group code, its DWG decoder
  in stream order.
- An MTEXT from a drawing older than R2000 reports a line spacing factor of `1` (the format
  has no such field there) instead of `0`, which is outside the factor's valid range.
- A LEADER's `has_arrowhead` read from an R2010-or-later DWG is `None`. The vendored
  engine reads that record one field short from R2010 on (it skips the annotation offset,
  which those files still carry), so the value it returned for the flag was a bit of the
  offset's encoding -- "no arrowhead" whenever the offset's z was zero -- not the file's
  flag. Earlier versions are unaffected and still read the flag.
- Two user-facing messages carried a run of spaces in mid-sentence: the R2007+ DXF error
  (`ParseError::UnsupportedDxfVersion`'s `Display`) and the missing-tag diagnostic. Both
  now read as one sentence, pinned by tests.
- **Text before R2007 is decoded through the drawing's codepage** (`header.codepage`, DXF
  `$DWGCODEPAGE`). LibreDWG returns such strings as the 8-bit bytes
  the file holds; they were read as UTF-8, so every non-ASCII character of a CP949 or CP1252
  drawing -- text values, attribute values, layer and block names -- came back as mojibake
  or U+FFFD with clean diagnostics. The decoding goes through the library's own codepage
  tables, and a byte the declared codepage has no character for is now U+FFFD *and* a
  `TEXT_ENCODING: ...` warning in `read_diagnostics` naming the entity and field. See
  `docs/CAVEATS.md`, "Text before R2007 is decoded here".
- **The DOS-era double-byte codepages decode.** LibreDWG's tables pair every byte of a
  Big5 or GB2312 string, ASCII included, so a drawing declaring either lost its
  `*Model_Space` (read as `*M`, `od`, ...) and every entity, and `中国 AB` read as four
  U+FFFD; CP932 (DOS Shift-JIS) was read one byte at a time. Only bytes >= 0x80 open a
  pair now, a GB2312 pair is looked up in the 7-bit form the library's table is indexed
  by, and CP932 is double-byte. ASCII is never looked up in any codepage's table, so a
  0x5C stays the backslash of `\P` and `\U+XXXX` where CP932's and JOHAB's tables say yen
  and won. `tests/codepage.rs`.
- **Closed LWPOLYLINEs are closed.** The `closed` field read bit 1 of the entity's `flag`,
  which in the library's LWPOLYLINE layout means "has extrusion"; closed is bit 512. Not one
  of the corpus's 1,137 LWPOLYLINEs had ever been reported closed, so every closed outline
  rendered as an open polyline. POLYLINE_2D/3D were unaffected (their bit 1 is closed).
- **Every attribute definition in a block, and every attribute value on an INSERT, is
  read** in R13..R2000 drawings. The library's chain walkers skip ATTDEF as if it were a
  sub-entity (all but the last were lost) and read an unresolved `first_attrib` pointer
  after a DXF import (an imported INSERT had no attributes at all). This crate now walks
  both chains itself for that version band; see `docs/CAVEATS.md`, "Attributes". Measured:
  36 more entities across the corpus, all in blocks with several ATTDEFs. The top-level
  copies of an INSERT's attributes now follow the INSERT, in file order (they preceded it).
- `tests/golden.rs` reads the first synthetic golden case (`tests/golden/g1.dxf`, written by
  the `uncad-model` golden writer from a spec) and requires the model to come back exactly as
  the spec states. It is what caught the two defects above.
- Table references in pre-R13 drawings (R1.4 to R12) resolve. Such a drawing points at its
  LAYER and BLOCK tables by index rather than by handle, and every one of those references
  used to come back `Unresolved("0")` even though the tables themselves were read: 304 layer
  and 46 block references across the corpus, now all resolved (the R11 drawing names the
  same layers as its R2000 twin). An index the table does not answer to is kept as
  `Unresolved("idx:<n>")` -- the index stands in for the handle -- which happens for the
  22 layers of the one R1.4 drawing whose LAYER table LibreDWG does not read.
- From R13 on, a reference whose handle is zero is `Absent`, not `Unresolved("0")`: the file
  carries no reference there. Measured: the 18 DIMENSIONs inside one R2018 file's
  dynamic-block definitions that have no block.
- A 3DSOLID/REGION whose SAT text (from LibreDWG's SAB-to-SAT conversion) contains pointers
  past its last record is now reported as entirely unread -- `skipped_edges` equals its edge
  count and `wireframe_edges` is empty -- instead of edges being resolved against whatever
  record happened to sit at a stale index. Measured on one R2007 drawing, 62 of 116 solids are
  like this (the converter drops records without renumbering); the other 54 extract fully.
- `ToSvgResult::unsupported_types` / `ToPngResult::unsupported_types` (and the CLI
  warning built from them) listed the same types in a different order from run to run.
  They are now sorted by name. A new `tests/determinism.rs` regenerates the JSON, the SVG
  and this list repeatedly and requires byte-identical results.
- A polyface mesh with an unused face-index slot made `parse()` panic in debug builds
  (index underflow; release builds were unaffected because the wrapped value was
  discarded). The corpus example DXFs are now all parsed in a test.
- The viewBox outlier trim grouped entity boxes into clusters whose order came out of a
  hash map, so a tie between equally scored clusters could be broken differently from
  run to run. Clusters now come out in input order.
- Building on Windows no longer needs a Visual Studio developer prompt. The C compile
  always located MSVC by itself, but bindgen's libclang only found the C standard headers
  when `INCLUDE` was already set; `libredwg-sys`'s build script now forwards the header
  directories `cc` located. Bindings are also generated *before* the C compile, so a
  libclang problem fails in seconds rather than after the whole LibreDWG compile.
- A DXF saved as R2007 or later used to read as an empty drawing without any error; it
  is now read (see "Changed"). `docs/CAVEATS.md` explains the cause (string width, both in
  LibreDWG's DXF importer and in how this crate read what it stored).
- `docs/CAVEATS.md` claimed every entity type with geometry was handled. It is not --
  several types that have a shape still arrive as `Unknown`. The section now states the
  supported list as the contract.

### Changed

- **Rendering moved to the `iron-render-cad` crate.** `to_svg`, `to_png`, `svg_to_png`,
  `Space`, the options and results, and the color resolution behind them are now
  [`iron-render-cad`](https://github.com/iyulab/iron-render-cad) (MIT); `uncad-cli` depends
  on it for `-o *.svg` / `-o *.png`, and this crate's tests take it as a dev-dependency.
  `uncad` itself no longer depends on `resvg` or `regex`. What stays here is the parser's
  own color need: the two hex stops of a HATCH gradient, which the model carries.
- **Every entity carries a reference ID, its origin and its confidence.** `EntityCommon`
  gains `id` (an `EntityId`, the name consumers point at an entity by -- minted by this
  backend as the file handle's value, or from the object's position in the file for an
  entity without a handle), `origin` (`Vector`, for everything this backend reads) and
  `confidence` (`High`: the values are what the file states). The file handle moves to
  `source_handle: Ref<String>`, provenance rather than identity, `Absent` when the file
  gave none. **JSON shape**: `common.handle` is gone; `common.id` is an integer,
  `common.origin`/`common.confidence` upper-case strings, `common.source_handle` a
  three-state reference like `common.layer`. None of the three has a default: a producer
  states them or the entity cannot be built.
- **The entity model is now the `uncad-model` crate.** `CadDatabase`, `Entity`, every
  `*Entity` struct, `Ref`, `Tables`/`LayerRecord`/`BlockRecord`, `ReadDiagnostics`, the ACI
  palette and the JSON serialization (`CadDatabase::to_json`, `ToJsonOptions`, `JsonError`)
  moved to [`uncad-model`](https://github.com/iyulab/uncad-model) (MIT), which this crate
  depends on and re-exports as `uncad::model`, `uncad::tables` and `uncad::json` -- so
  paths such as `uncad::model::Ref` and `uncad::Entity` keep working. What changes:
  - `to_svg` and `to_png` are free functions, `uncad::to_svg(&db, options)` and
    `uncad::to_png(&db, options)`, no longer methods on `CadDatabase` (a type this crate
    no longer defines). `CadDatabase::to_json` stays a method, in the model crate.
  - `Point2D`/`Point3D` are the model's plain structs. They are no longer `#[repr(C)]`
    mirrors of LibreDWG's layout; `dynapi.rs` keeps private `RawPoint2D`/`RawPoint3D`
    for the C reads and converts at the boundary, and a `DwgRaw` marker trait on every
    dynapi accessor makes reading C memory through a model type a compile error.
  - `Entity`, `HatchEdge` and `HatchBoundaryPath` are no longer `#[non_exhaustive]`: a
    new entity kind must reach every consumer that matches on them as a compile error,
    not as a wildcard arm that quietly ignores it. Adding a variant is a 0.x minor bump.
  - `ReadDiagnostics` is `{ warnings: Vec<String> }` -- the model does not carry a
    LibreDWG bit set; the bit-to-name decoding is `read_diagnostics_from_libredwg_bits`
    in this crate. (The field was unreleased, so no published JSON shape changes.)
  - `color.rs` keeps only what rendering needs (BYLAYER/BYBLOCK resolution, the
    white-background flip, hex formatting); the palette itself is
    `uncad_model::color::ACI_PALETTE`.
- **Reference fields are three-state values, not strings.** `EntityCommon::layer`,
  `InsertEntity::block_name`, `DimensionEntity::block_name`, `AcadTableEntity::block_name`
  and `MLineEntity::mlinestyle_name` are now `Ref<String>`: `Resolved(name)`, `Absent` (the
  file carries no handle for the field) or `Unresolved(handle)` (the handle points at
  nothing -- the handle is kept, since it is what tells one missing table row from
  references broken wholesale; for a pre-R13 drawing, which references tables by index,
  the payload is `idx:<n>` instead of a hex handle). Until now all three came back as `""`, so a consumer could
  not tell a layer called nothing from a layer that could not be read. In JSON the field
  is adjacently tagged like hatch boundary paths: `{"type":"RESOLVED","data":"0"}`,
  `{"type":"ABSENT"}`, `{"type":"UNRESOLVED","data":"2A"}`. **This changes the JSON
  shape** of every entity (`common.layer`) and is the reason the next release is a 0.x
  minor. `Ref::name()` gives the resolved name or `""` for consumers that only need a
  lookup key.
- **A DXF saved as R2007 or later (`$ACADVER` `AC1021` and up) is read**, instead of being
  returned as a drawing with no entities and no error. LibreDWG's importer holds such a
  file's strings in two widths -- UTF-16 for everything it sets through its field setter,
  the file's own UTF-8 for MTEXT text and the HEADER variables -- and hands both out as if
  they were 8-bit; `TextDecoder` now reads each in its width, and the vendored `dwg.c`
  patch does the same for the importer's own layer and block lookups. Measured on the
  corpus: 27 of the 32 R2007+ DXFs read (the other 5 fail inside LibreDWG with critical
  error 2048), and 22 of the 24 with a DWG twin state the same layers and INSERT blocks
  as it; `tests/dxf_pipeline.rs`, `tests/r2007_dxf_handles.rs` and `tests/codepage.rs`
  pin it. `ParseError::UnsupportedDxfVersion` stays, for the case this used to be: an
  R2007+ DXF whose entities LibreDWG placed in model or paper space and none of which
  reached the model is an error, not an empty drawing (no corpus file is).
- `ParseError::InvalidPath` is gone: `parse()` no longer hands LibreDWG a C path, and a
  file it cannot read is `ParseError::Io`. Breaking for code that names the variant.
- No `std` hash collections anywhere in the workspace: `clippy.toml` disallows `HashMap`
  and `HashSet`, and the `iter_over_hash_type` lint is on. Both earlier ordering bugs went
  through `into_iter()`/`into_values()`, which no lint on `for` loops would have seen.

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
