# Changelog

Notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- TRACE is read and drawn: `Entity::Trace`, which reuses `SolidEntity` the way `XLine`
  reuses `RayEntity`. It used to arrive as `Entity::Unknown`. `Entity` is
  `#[non_exhaustive]`, so this is not a breaking change.
- All three crates declare `rust-version = "1.88"`. Until now the minimum was whatever
  happened to build. The value is measured (1.87 fails, 1.88 passes) and CI has an `msrv`
  job that checks the workspace with exactly the declared toolchain.

### Fixed

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
- Documented, not fixed: a DXF saved as R2007 or later reads as an empty drawing without
  any error. `docs/CAVEATS.md` explains the cause (string width in LibreDWG's DXF
  importer) and what to do meanwhile.
- `docs/CAVEATS.md` claimed every entity type with geometry was handled. It is not --
  several types that have a shape still arrive as `Unknown`. The section now states the
  supported list as the contract.

### Changed

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
