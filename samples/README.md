# samples/

Drop any DWG/DXF file here for manual testing. This directory is gitignored except for this
README (see root [`.gitignore`](../.gitignore)), so nothing placed here ever gets committed --
no license clearance needed.

There used to be three tracked sample files here (from
[LibreDWG](https://github.com/LibreDWG/libredwg)'s own `test/test-data/`, GPLv3+), with
`crates/uncad/tests/core.rs` hardcoding expected values against their exact content
(entity counts, coordinates, colors, byte-for-byte `to_svg()` output -- cross-checked against
this project's old JS/WASM predecessor before that predecessor was deleted). Both those sample
files and that whole test file were removed together, since every one of its tests depended on
those specific files and there's no way to regenerate correct expected values for different
files without an independent reference to check against. See `git log` for the full history
(this project's docs describe current state, not a running history log).

Practically, this means: there's no automated regression coverage for `parse()`/`to_svg()`/
`to_json()` against *this* directory. It does not mean those calls are untested. What
runs in `cargo test` is:

- the self-contained unit tests that need no external file -- across `acis.rs`, `color.rs`,
  `convert.rs`, `json.rs`, `png.rs`, `svg.rs` and `tables.rs`, covering the pure-Rust side
  (colour resolution, JSON tagging and round-trips, SVG emission, ACIS parsing, unit conversion,
  table lookup);
- `png.rs`'s `to_png_renders_a_real_dwg_to_a_valid_png`, which runs the whole
  `parse()` -> `to_svg()` -> `to_png()` pipeline against a DWG from the LibreDWG submodule
  (`git submodule update --init` first -- the build compiles a vendored copy and does not
  need the submodule; the fixture-based tests do);
- `crates/uncad/tests/dxf_pipeline.rs` and `tests/acis_sab.rs`, which parse corpus files from
  that same submodule and assert *properties* rather than pinned values: the SVG carries
  geometry, the model survives a `to_json()` -> `serde_json` round trip unchanged, SAB solids
  get the same wireframe in `entities` and in their block record, garbage input returns an
  error instead of panicking;
- `crates/uncad-cli/tests/documented_invocations.rs`, which runs the real `uncad` binary for
  every invocation the README documents. Its `--no-trim` test authors a five-line DXF from group
  codes inside the test (four lines forming a square plus one far-away outlier) and checks that
  the flag changes the viewBox -- an authored fixture needs no external reference for its
  expected values, which is exactly the property the removed `core.rs` tests lacked.

What is still missing is breadth: entity-count parity across many real drawings, byte-level
render comparison. Use files dropped here for manual spot-checks:

```bash
cargo run -p uncad-cli -- samples/whatever.dwg
cargo run -p uncad-cli -- samples/whatever.dwg -o out.svg
cargo run -p uncad --example dump samples/whatever.dwg
```
