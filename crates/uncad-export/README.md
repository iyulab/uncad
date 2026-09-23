# uncad-export

A DWG/DXF drawing as a package an LLM or a vision model can read: an
overview image sized to the model's patch budget, a pyramid of overlapping
tiles with JSON sidecars saying what is on each, one image per paper layout,
and JSON records with the exact numbers a picture cannot give -- lengths,
areas, dimension values, texts -- each pointing at the pixels it is drawn in.

It is built on [`uncad`](https://crates.io/crates/uncad), which parses the
drawing, and [`iron-render-cad`](https://crates.io/crates/iron-render-cad),
which draws it. See the crate documentation for the package's layout.

## Licence

GPL-3.0-or-later (see `LICENSE`), like the rest of this repository. The
bundled font `fonts/UncadSans-Regular.otf` is a Noto Sans KR subset under the
SIL Open Font License 1.1 (see `fonts/OFL-NotoSansKR.txt` and
`fonts/README.md`).
