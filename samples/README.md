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
`to_json()` against *this* directory. It does not mean those calls are untested -- `cargo test`
covers them through self-contained unit tests on the pure-Rust side, plus fixture-based tests
that read the LibreDWG submodule's own corpus (`git submodule update --init` first: the build
compiles a vendored copy and does not need the submodule, but those tests do).

That inventory is not restated here, so it can't drift out of sync -- it lives in
`docs/CAVEATS.md`'s "파일 기반 회귀 테스트는 소수" section, which lists what each test file
currently covers and what breadth is still missing. For how unit, integration and example
targets divide up, see `docs/ARCHITECTURE.md`'s "테스트 구조" section.

Use files dropped here for manual spot-checks:

```bash
cargo run -p uncad-cli -- samples/whatever.dwg
cargo run -p uncad-cli -- samples/whatever.dwg -o out.svg
cargo run -p uncad --example dump samples/whatever.dwg
```
