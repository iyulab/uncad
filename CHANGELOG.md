# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A SPLINE carries what defines its curve: `degree`, `knots`, `weights` (empty when
  the file gives none -- every weight is 1), and the `closed` / `periodic` bits as
  `Option<bool>`. A spline stored by its fit points has no periodic bit, and no
  closed bit before R2013; those are `None`, not `false`.

### Changed

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

- `MTextEntity::rotation` is the direction of the text's X axis instead of a constant `0`,
  which reported rotated multi-line text as horizontal.
- A two-line angular dimension's `definition_point` (DXF 10) is read instead of reported as
  not stated: the library keeps it under the field name `xline2end_pt`.
- An MTEXT from a drawing older than R2000 reports a line spacing factor of `1` (the format
  has no such field there) instead of `0`, which is outside the factor's valid range.
- A LEADER's `has_arrowhead` read from an R2010-or-later DWG is `None`. The vendored
  engine reads that record one field short from R2010 on (it skips the annotation offset,
  which those files still carry), so the value it returned for the flag was a bit of the
  offset's encoding -- "no arrowhead" whenever the offset's z was zero -- not the file's
  flag. Earlier versions are unaffected and still read the flag.
- Two user-facing messages carried a run of spaces in mid-sentence: the refusal of an
  R2007+ DXF (`ParseError::UnsupportedDxfVersion`'s `Display`) and the missing-tag
  diagnostic. Both now read as one sentence, pinned by tests.
- **Text before R2007 is decoded through the drawing's codepage** (`header.codepage`, DXF
  `$DWGCODEPAGE`). LibreDWG returns such strings as the 8-bit bytes
  the file holds; they were read as UTF-8, so every non-ASCII character of a CP949 or CP1252
  drawing -- text values, attribute values, layer and block names -- came back as mojibake
  or U+FFFD with clean diagnostics. The decoding goes through the library's own codepage
  tables, and a byte the declared codepage has no character for is now U+FFFD *and* a
  `TEXT_ENCODING: ...` warning in `read_diagnostics` naming the entity and field. See
  `docs/CAVEATS.md`, "Text before R2007 is decoded here".
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
  is now refused (see "Changed"). `docs/CAVEATS.md` explains the cause (string width in
  LibreDWG's DXF importer) and why reading it partially was rejected.
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
- A DXF saved as R2007 or later (`$ACADVER` `AC1021` and up) is now refused with
  `ParseError::UnsupportedDxfVersion` instead of being returned as a drawing with no
  entities and no error. The decision is made from the file's HEADER section before
  LibreDWG reads it; R2000/R2004 DXF, files without `$ACADVER`, binary DXF and every DWG
  are unaffected. Callers that treated the empty result as success will now see an error
  -- that is the point. `ParseError` is `#[non_exhaustive]`, so the new variant is not a
  breaking change to matches. Two tests pin it: a walk of the corpus that requires exactly
  the R2007+ files to be refused, and one drawing under two `$ACADVER` values.
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
