# Architecture

## Crate layout

```
lib/libredwg/            git submodule pointing at LibreDWG upstream
                         (github.com/LibreDWG/libredwg). Not used by the build -- it is
                         the source vendor/ is copied from, and the origin of the
                         real-file test fixtures (test/test-data/). Left unmodified:
                         the local patches are in the vendored copy, not here.
crates/
  libredwg-sys/          raw FFI: build.rs compiles vendor/libredwg/src/*.c (see "Build")
                         directly through the cc crate, no autotools, and generates the
                         bindings with bindgen.
    shim/                uncad_shim.c -- accessors that reach entity pointers behind
                         opaque types, a walker that flattens nested structs dynapi
                         cannot reach (MULTILEADER leader lines), a 3DSOLID SAB->SAT
                         conversion that runs on a copy rather than the original,
                         readers that decode a DWG or DXF from a memory buffer rather
                         than a path, and the file-header fields (version, codepage,
                         string width, a pre-R13 header's length, whether the Template
                         section was read) that decoding a drawing's text and header
                         need. Strings themselves are decoded on the Rust side.
    vendor/libredwg/     the upstream C sources actually compiled (see "Build"), five
                         of them carrying a local patch (NOTICE.md)
    vendor-config/       config.h -- hand-written, standing in for autotools' output
    examples/            smoke.rs -- manual check of the raw FFI (see "Test layout")
  uncad/                 the safe API, layered: dynapi.rs (reflection helpers) ->
                         convert.rs (raw Dwg_Data* -> uncad_model's Entity) ->
                         table_convert.rs (LAYER/BLOCK_RECORD/DIMSTYLE/MLINESTYLE and
                         LAYOUT with its plot settings), with acis.rs
                         for 3DSOLID wireframes and hatch_color.rs for the gradient
                         stop colors the model carries. The model and its JSON form
                         are the uncad-model crate's; SVG/PNG rendering is the
                         iron-render-cad crate's (uncad-cli and this crate's tests use
                         it). Read-only: there is no DWG/DXF write path.
    tests/               integration tests against the public API (dxf_pipeline.rs,
                         acis_sab.rs)
    examples/            dump.rs / blocks.rs -- manual checks
  uncad-cli/             the CLI binary (uncad)
    tests/               documented_invocations.rs -- every call README and --help
                         advertise; release_invariants.rs -- the licence text and the
                         vendored-patch notice every published crate has to carry
```

## Build: the `cc` crate instead of autotools, a vendored copy instead of the submodule

`build.rs` compiles `crates/libredwg-sys/vendor/libredwg/src/*.c` directly, with no
`configure`, `autoreconf` or `libtool`. `crates/libredwg-sys/vendor-config/config.h` is a
hand-written stand-in for what autotools would generate.

**Why a vendored copy rather than the `lib/libredwg` submodule**: `cargo package` and
`cargo publish` never include files outside the crate directory
(`crates/libredwg-sys/`), and someone installing this crate from crates.io has no `.git`
and no submodule at all. A `build.rs` that referenced `repo_root/lib/libredwg` would work
only inside this workspace and fail for every published consumer -- a failure mode
confirmed with `cargo publish --dry-run`. So `crates/libredwg-sys/vendor/libredwg/` holds
a copy of exactly the files this crate compiles: 24 `.c` files plus every header,
`.spec`, `.inc` and codepage table they `#include`, 112 files in total
(`git ls-files crates/libredwg-sys/vendor | wc -l`), a subset rather than the whole
submodule. Five of those files carry a local patch each, marked in the source with a
dated `uncad local patch` comment and listed in `crates/libredwg-sys/NOTICE.md` (see
`docs/CAVEATS.md`, "Local patches to the vendored LibreDWG"); everything else is
byte for byte what the submodule holds. The
submodule itself stays: it is the diff target when upstream moves, and the real-file
tests (`png.rs`, `tests/dxf_pipeline.rs`, `tests/acis_sab.rs` in `uncad`,
`tests/documented_invocations.rs` in `uncad-cli`) read fixtures from
`lib/libredwg/test/test-data/`. In short, the submodule is a precondition of
`cargo test`, not of `cargo build`.

**Updating the submodule**: after moving the `lib/libredwg` pointer (e.g. with
`git submodule update --remote`), run `scripts/sync-libredwg-vendor.sh` to regenerate the
vendored copy -- it re-traces the real `#include` graph and rebuilds the file list. It
deletes and recopies the whole directory, so the local patches are gone afterwards and
have to be re-applied (or dropped, with `NOTICE.md` and `docs/CAVEATS.md` updated to
match). Then run `cargo build --workspace`: a newly required `.c` or header shows up
immediately as a compile error. `build.rs` registers the whole `vendor/libredwg/`
directory with `cargo:rerun-if-changed`, so an incremental build really does recompile the
C sources and regenerate the bindings after a re-vendor -- no `cargo clean` needed.

`build.rs` also carries three drift detectors:

1. It compares the `.c` file count in `vendor/libredwg/src` against `LIBREDWG_SOURCES`.
   A mismatch means the vendored copy is damaged or out of step, and it `panic!`s.
2. It counts the `uncad local patch` markers in every file under `vendor/libredwg` and
   compares them, file by file, against `LOCAL_PATCHES`. A re-vendor replaces a patched
   file with upstream's without changing any file count, so this is what notices that the
   local patches are gone; it `panic!`s, naming the files.
3. When the `lib/libredwg` submodule is checked out (local development and CI only, never
   for a published-crate consumer), it cross-checks that submodule's `.c` file count
   against the vendored copy and emits a `cargo:warning` if they have diverged, without
   failing the build.

## Test layout: unit, integration, example

The standard Rust layout. Which of the three a new test belongs in comes down to **what
it needs access to**.

| Location | Compilation unit | Reach | Purpose |
|---|---|---|---|
| `#[cfg(test)] mod tests` in `src/*.rs` | inside the crate | everything, including private items | pure logic verifiable with synthetic data, no fixture file |
| `tests/*.rs` | one crate per file | the public API only | end-to-end runs against real files |
| `examples/*.rs` | standalone binaries | the public API only | manual tools and usage examples |

**Unit tests** exist where they are precisely because they can call private helpers.
Everything that needs no external file -- SAT parsing, dynapi size checks, the hatch
stop colors -- lives here. (SVG generation and outlier-trim clustering moved to the
renderer crate together with the code.)

**Integration tests** compile as separate crates and therefore see only the public API, so
what they cover matches exactly what someone who installed the crate can do. Fixtures come
from `lib/libredwg/test/test-data/` (see "Build" -- the submodule is a precondition of
`cargo test`, not `cargo build`). `uncad-cli`'s tests run the built binary itself rather
than the library.

**Examples** assert nothing. Instead, `cargo test` and
`cargo clippy --workspace --all-targets` compile them, so a broken public API signature
shows up as a CI build error: they are executable documentation and a compile guard at
once. All three take a file path as an argument, for pointing at whatever is in `samples/`
(see `samples/README.md`).

```bash
cargo run -p uncad --example dump <file>            # dump every entity
cargo run -p uncad --example blocks <file> [name]   # list block records, or one block's contents
cargo run -p libredwg-sys --example smoke <file>    # read through the raw FFI, print the object count
```

**Assertion policy**: real-file tests do not pin expected values. They assert only
properties that survive a change of file -- a round trip (where the drawing is its own
expectation), or that an option actually changes the result. An earlier test file
(`crates/uncad/tests/core.rs`) hardcoded numbers copied from this project's own output;
with no way to regenerate correct expectations for a different file, it was deleted along
with its fixtures. See `samples/README.md`.

What is actually covered today -- test counts, per-file breakdown, and what is still not
automated -- is in `docs/CAVEATS.md`, "File-based regression tests". This section only
covers where things go.

## FFI boundary: opaque types plus dynapi reflection

`Dwg_Object`'s `tio` field is a C union over ~90 `Dwg_Entity_*`/`Dwg_Object_*` types, and
it breaks bindgen's struct code generation (clang itself parses and sizes it fine -- only
bindgen's layout step fails). `Dwg_Data`, `Dwg_Object` and their subtypes are therefore
all declared `.opaque_type()`.

That suits the design rather than fighting it: entity fields were always going to be read
through `dwg_dynapi_entity_value`/`dwg_dynapi_common_value`, LibreDWG's own
reflection API keyed by string field name, with runtime type and range checks.
`uncad::dynapi` wraps it in the generic helpers `get_field::<T>`, `get_common_field::<T>`,
`get_header_field::<T>`, `get_text` and `get_array_field::<C, T>`, comparing the field size
dynapi reports against the requested Rust type's size so a wrong type mapping fails loudly
instead of quietly corrupting data. Text fields come back undecoded on purpose, as LibreDWG
hands them out: a UTF-8 copy it converted (R2007+ DWG), or the stored string -- codepage
bytes before R2007, and in an R2007+ DXF the UTF-16 its importer wrote or, for the few
fields it copies byte for byte, the file's UTF-8. `uncad::text::TextDecoder` (one per
`parse()`) says which width a stored string is read in and is the only place any of them
become `String`s -- through LibreDWG's codepage tables where a codepage applies, with what
could not be decoded reported in `read_diagnostics` (see `docs/CAVEATS.md`).

**The same bindgen failure recurs for individual types.** Even with the whole `tio` union
opaque, allowlisting a nested struct on its own (`Dwg_HATCH_Path`, `Dwg_HATCH_PathSeg`,
`Dwg_HATCH_ControlPoint`, `Dwg_HATCH_DefLine`, `Dwg_HATCH_Color`, `Dwg_MLINE_vertex`,
`Dwg_MLINESTYLE_line`) reproduces it. The tell is a `layout_tests()` assertion that can
never pass, computing something like `1usize - 96usize`: bindgen emitted a 1-byte
placeholder body while keeping clang's correct size in the assertion. The standard
response is to `.blocklist_type()` it in `crates/libredwg-sys/build.rs`, hand-write a
`#[repr(C)]` struct in `src/lib.rs` with dwg.h's exact field order and types, and add a
compile-time assertion against clang's real `sizeof()` (which the failing generated
`layout_tests()` conveniently reports). Fields whose pointee type is never dereferenced
(`Dwg_MLINE_vertex.lines`) stay `*mut c_void`, which avoids hand-transcribing yet more
types -- and a `parent` back-pointer typed as the owning entity is exactly what cascades,
so those are left untyped too.

**Cross-platform enum width**: `dwg_object_get_fixedtype`'s real C declaration returns
`int`, not `DWG_OBJECT_TYPE` -- a mismatch in `dwg_api.h` itself. bindgen infers
`DWG_OBJECT_TYPE`'s representation from clang's target-dependent choice of underlying
integer type for that C enum, and the two diverge in practice: `i32` on the MSVC target,
`u32` on `x86_64-unknown-linux-gnu` (confirmed by actually compiling in a Docker
`rust:latest` image). Every FFI call site casts the value to `DWG_OBJECT_TYPE`
immediately, so the rest of the code only ever compares one canonical type.

## Thread safety

The LibreDWG C library is not thread-safe: it has non-reentrant global state such as
`loglevel`. The `uncad` crate serializes every FFI entry point through a process-wide
`Mutex` (recovering from poisoning rather than propagating it) and so offers a safe public
API. Using `libredwg-sys` directly means upholding that constraint yourself -- concurrent
calls have reproducibly caused `STATUS_HEAP_CORRUPTION`.

## Model: `uncad-model`'s `CadDatabase`, with `Dwg_Data` living only inside `parse()`

The model is not this crate's: `CadDatabase`, `Entity`, `Tables` and their JSON form are
the [`uncad-model`](https://github.com/iyulab/uncad-model) crate (MIT, pure data), which
this crate depends on by version and re-exports as `uncad::model` / `uncad::tables` /
`uncad::json`. `CadDatabase` is a plain Rust value holding `entities` (what the model and
paper spaces own), `tables` (LAYER, every BLOCK_RECORD, DIMSTYLE, MLINESTYLE, and the
LAYOUTs with their plot settings) and `read_diagnostics` (the reader's non-fatal
warnings). `parse()` reads the file into memory itself (LibreDWG's own readers `fopen()` a
path, which on Windows cannot open a non-ASCII one), and the `Dwg_Data` that the shim's
`uncad_dwg_read_bytes`/`uncad_dxf_read_bytes` fill from those bytes is walked inside
`parse()` (`convert_entities`, `convert_tables`, then the header read), freed with
`dwg_free` immediately afterwards, and never reaches the return value. The hub of
"DWG/DXF -> one model -> several outputs" is therefore the model
crate's value, and the outputs are `CadDatabase::to_json()` (serde, in `uncad-model`) and,
in the `iron-render-cad` crate, `to_svg(&db, ..)` and `to_png(&db, ..)`.

Coordinates cross that boundary by conversion, not by sharing a type: `dynapi.rs` reads
LibreDWG's point fields into its own `#[repr(C)]` `RawPoint2D`/`RawPoint3D` (whose layout
is pinned to `dwg.h`'s) and converts them into the model's plain `Point2D`/`Point3D`. The
`DwgRaw` marker trait bounds every dynapi accessor, so a model type can never be used to
read C memory by accident -- the compiler refuses it.

The model is deliberately lossy: it keeps what consumers of the drawing's content need and
nothing else. A layer carries its state (off, frozen, locked, plotted), its lineweight and
the name of its linetype, but the linetype and text style definitions those names resolve
in and object dictionaries are not carried, and neither are header variables (a consumer
that needs them gets them beside the model, as this crate's own `uncad::Header`, from
`parse_with_header`). It cannot be used to write a DWG/DXF back out, and this
project offers no writing (0.1.0's `write_dwg`/`write_dxf`/`dwg_to_dxf` were removed; see
`CHANGELOG.md`).

The C build still includes the encoder sources and defines `USE_WRITE`, because reading
depends on them: `dwg.c` gates `dxf_read_file()` on `USE_WRITE`, `in_dxf.c` uses
`encode.c`'s handle post-processing helpers, and `out_dxf.c` hosts
`dwg_convert_SAB_to_SAT1`, which the 3DSOLID wireframe extraction needs. No write entry
point is bound to Rust: `dwg_write_file` is left out of the bindgen allowlist and the DXF
write shim was deleted.

## The entity model and block-based traversal

`CadDatabase::entities` is not a global scan classifying every object by fixedtype. It is
built in this order:

1. Walk the `BLOCK_HEADER` objects and keep only those named `*Model_Space` or
   `*Paper_Space*` (case-insensitive).
2. Walk the entities those blocks own, through
   `get_first_owned_entity`/`get_next_owned_entity` -- LibreDWG's own type-agnostic
   iterator.

Entities inside a block *definition* an INSERT references are reachable only through that
block's own `BLOCK_RECORD.entities`, never through the top-level `entities`. A global scan
cannot make that distinction and has in fact leaked entities that should not have been
there.

An INSERT's ATTRIBs are duplicated into the top-level `entities` (they really are drawn),
but not into the entity list of the block that owns the INSERT.

`BLOCK_HEADER.name` holds only an abbreviated name for anonymous blocks (`*D` for
dimension caches, and so on). The disambiguating name (`*D30`) lives on the `BLOCK` entity
the block owns, reached through `BLOCK_HEADER`'s `block_entity` handle field.
`table_convert::resolve_block_name` shares that logic between INSERT and DIMENSION.

## 3DSOLID/REGION ACIS wireframes (`acis.rs`)

`crates/uncad/src/acis.rs` is not a general ACIS/B-rep parser. It is a minimal wireframe
extractor that pulls one straight segment per `edge` record out of ACIS SAT (v1, ASCII)
text. Spatial's public "SAT Save File Format" documentation (long mirrored at, for
example, paulbourke.net/dataformats/sat) was used only as a reference for what records and
fields *mean*; no code or text from it is reproduced, and the implementation was written
independently. Curved edges are approximated as chords, and faces and surfaces are not
interpreted at all -- the result is always a wireframe, never a filled solid.

A solid stored as SAB (v2, binary) has to be converted to SAT text first, and LibreDWG's
`dwg_convert_SAB_to_SAT1` converts **in place**: it sets `version` to 1, fills
`encr_sat_data` with plaintext SAT, and leaves `acis_data` as the original SAB bytes.
`parse()` reads every solid twice (`convert_entities` for model space, then
`convert_tables` for block records), so calling it on the live entity made the second read
take the `version == 1` branch, parse binary SAB as text, and lose the wireframe. (While
the removed write path existed, the same mutation corrupted written files too.) The
`uncad_3dsolid_sab_to_sat_text` shim in `libredwg-sys` therefore runs the conversion on a
shallow copy and returns only the text, leaving `parse()` with no side effect on the
`Dwg_Data` at all.

`extract_wireframe(entity_ptr, dxfname)` takes `dxfname` as an argument because REGION is
a `typedef` of `Dwg_Entity__3DSOLID` in `dwg.h` and shares its dynapi field table, yet
`dwg_dynapi_entity_value` strictly compares the name passed in against the object's actual
`obj->name` and silently fails every field read on a mismatch. Structural identity is not
enough to let the name be hardcoded.
