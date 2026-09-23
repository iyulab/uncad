# Port status: feat/0.3-readable onto the three-repository split

Handoff for whoever continues this work (a person or an agent). Written 2026-09-23.

## What this is

The 0.3 "readable" work (LLM/VLM export, ~120 review fixes) was done in the old
monolithic uncad, preserved as the tag `archive/feat-0.3-readable` (c0213bb, pushed).
Upstream then split the project into three repositories: `uncad` (parser, this repo),
`uncad-model` (entity model, MIT) and `iron-render-cad` (renderer, MIT). The work is
being re-expressed on that split, feature by feature, on branch `integrate/0.3` in
all three repositories (pushed; each contains its repository's current `main`).

Rules the owner set: do not publish crates (upstream releases them); do not push to
`main`; after each step, merge upstream `main` into `integrate/0.3` (where both sides
did the same thing, adopt upstream's shape), run the gates, push `integrate/0.3`.
Upstream's own rules (MSRV 1.88, no std HashMap/HashSet, cargo-deny, "a version
dependency, never a path", golden tests exact, "propose first" for new public API)
apply to everything.

## Local setup to build the three together

Clone `uncad`, `uncad-model` and `iron-render-cad` as siblings and check out
`integrate/0.3` in each. The unpublished model and renderer are wired in by an
UNCOMMITTED `.cargo/config.toml` in each consumer (add `.cargo/` to
`.git/info/exclude`; never commit it, and never commit a `Cargo.lock` that records
path sources):

```toml
# uncad/.cargo/config.toml
[patch.crates-io]
uncad-model = { path = "<abs path>/uncad-model" }
iron-render-cad = { path = "<abs path>/iron-render-cad" }
```

```toml
# iron-render-cad/.cargo/config.toml
[patch.crates-io]
uncad-model = { path = "<abs path>/uncad-model" }
```

Gates per repository: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace --no-fail-fast`. At 2026-09-23: uncad 304, iron-render-cad
265, uncad-model 59, all passing.

## Done

- Native layer: five vendored LibreDWG patches (see `crates/libredwg-sys/NOTICE.md`),
  shim, build.rs patch-marker check, per-crate LICENSE.
- Model fields (uncad-model), parser reading them (uncad), renderer phases A and B
  (iron-render-cad), a sync with upstream's own OCS/bulge/vertex work.
- R2007+ DXF read, DOS-era code pages, `parse_bytes`, `uncad::Header`.
- iron-render-cad's render-once/assemble-many Scene API (a proposal for upstream).
- `crates/uncad-export` and `uncad export`; `--version`, `--include-hidden`, a strict
  CLI parser.
- Docs: README, ARCHITECTURE, CAVEATS ("The LLM/VLM package"), VLM_EXPORT_DESIGN.

## Remaining, in suggested order

1. **CLI options not yet ported** (the library already has each): `--fit`, `--ppu`,
   `--max-edge`, `--stroke`, `--bg`, `--crop`, `--padding`, `--lattice`, `--fonts`,
   `--text-px`, `--shard-kb`, `--frame-gap`, `--min-frame-entities`,
   `--max-frames`, `--full`. Source: `crates/uncad-cli/src/main.rs` and its tests at
   the archive tag.
2. **Export parity not checked item by item.** The agent porting `export.rs` was
   stopped after 9 commits (up to 895b8e2). Walk the archive tag's
   `crates/uncad/src/export.rs`, `crop.rs`, `dimension.rs` and their tests and
   confirm every behaviour exists in `crates/uncad-export`.
3. **Corpus differences to classify.** `scripts/compare-export-with-archive.py`
   (needs the archive tag's release binary and this branch's) over all 208 corpus
   files: 199 read by both, none by only one; manifest counts differ on 9 files
   (entities), 5 (texts), 18 (geometry), 1 each (frames, paper texts, hidden).
   Entities/texts are ATTDEF/ATTRIB records upstream's attribute walk now reads
   (intended). The 18 geometry differences are unclassified.
4. **Speed.** Release build, the port is 17-41 % slower than the archive
   (CAVEATS has the numbers). A rough stage split on AutoCADSamples5 showed parse,
   JSON, SVG, PNG and export all 20-30 % slower, i.e. a shared path, but it was
   measured under load; re-measure alone, then profile.
5. **Adversarial review** of all three `integrate/0.3` branches (none has been run
   on the ported code).
6. **Deferred, needs a model decision:** HATCH extrusion (mirrored hatches drawn as
   stated), EntityCommon lineweight/linetype/ltype_scale, LayoutRecord flags/extents/
   active viewport and more PlotSettings fields, the renderer's light-colour
   darkening on white (a proposal).
7. **docs/EVAL.md** from the archive tag, re-measured on this layout.
8. **Upstream review.** `docs/UPSTREAM_PR_DRAFTS.md` has the three PR descriptions,
   including the three decisions that reverse upstream's documents and the new public
   API that is "propose first". Opening the PRs and releasing are the owner's and
   upstream's calls.
