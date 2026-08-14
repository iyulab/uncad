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

Practically, this means: there's currently no automated regression coverage for `parse()`/
`to_svg()`/`dwg_to_dxf()` against real files -- only `crates/uncad/src/color.rs`'s
self-contained unit tests (no external file dependency) still run in `cargo test`. Use files
dropped here for manual spot-checks instead:

```bash
cargo run -p uncad-cli -- samples/whatever.dwg
cargo run -p uncad-cli -- samples/whatever.dwg -o out.svg
cargo run -p uncad --example dump samples/whatever.dwg
```
