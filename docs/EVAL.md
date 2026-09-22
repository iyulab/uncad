# Evaluation: what the tests measure and the numbers they last produced

This file records how the 0.3.0 "Readable" work is checked beyond the unit
tests, and the numbers the checks produced on 2026-09-22 (Windows 11, Rust
1.95, debug profile unless said otherwise). Rerun the commands below and
replace the tables when something changes.

## 1. Corpus sweep

`crates/uncad/tests/corpus_sweep.rs` parses every `.dwg` and `.dxf` under
`lib/libredwg/test/test-data` (LibreDWG's own corpus: 141 DWG and 67 DXF,
R1.4 to R2018) and renders each parsed file to the default PNG. It fails
only on a panic; parse failures are counted and listed. It is ignored by
default because it takes minutes:

```bash
cargo test -p uncad --test corpus_sweep -- --ignored --nocapture
```

Last run (debug profile, 232 s):

| ext | files | parsed | rendered | entities | excluded by crop | hidden | seconds |
|---|---|---|---|---|---|---|---|
| dwg | 141 | 141 | 141 | 3799 | 16 | 3353 | 155.7 |
| dxf | 67 | 58 | 58 | 843 | 8 | 110 | 76.0 |

No panics. The nine DXF files LibreDWG refuses are its own known limits
(its DXF reader is documented as incomplete): `2000/TS1.dxf`,
`2004/Surface.dxf`, `2013/gh109_1.dxf`, `2018/Constraints.dxf`,
`2018/Dynblocks.dxf`, `2018/LiveSection1.dxf`, `2018/TS1.dxf` and
`example_r12.dxf` fail with error 2048, `r1.4/entities.dxf` with 4096. The
"hidden" column is dominated by dimension definition points on `DEFPOINTS`
inside dimension blocks (`docs/CAVEATS.md`, "Hidden entities").

## 2. Acceptance questions

`crates/uncad/tests/acceptance.rs` exports `example_2000.dwg` and answers
five questions from the package alone, the way an agent would (manifest ->
`strings.json` -> the record's shard -> the tile's sidecar), never from
pixels:

1. What does the aligned dimension read, and does the stored value agree
   with the definition points? (`1504,68`, delta under 0.001 mm)
2. How many `CIRKLO_PUNKTOJ` blocks are placed, and where? (8, each with a
   position and a tile)
3. Where is the text `teksto simpla`? (its id from `strings.json`, its tile
   from `texts.json`, its pixel box from the sidecar -- the same box the
   record carries)
4. What is the largest closed area and what is written inside it?
   (`regions.json`, agreeing with `geometry.json` for the same entity)
5. What is not in the picture? (`report.json`: the 3256x INSERT as
   `scale_outlier` and its far-away attribute, matching the manifest's
   count; one entity on the frozen `ADSK_SYSTEM_LIGHTS` layer)

`crates/uncad/tests/export.rs` adds the structural checks: every file the
design lists, the overview within the profile's edge and patch budget, the
tile grid formula per level, sidecar affines that round-trip, records that
point only at written tiles, and byte-identical output on a second run
(`report.json` excepted, for its timings).

## 3. Export timings (release profile)

`cargo build --release -p uncad-cli`, then `uncad export <file> -o <dir>`
with the defaults (Claude profile, up to 5 levels, 400 tiles). Wall clock
includes parsing; the machine has 16 hardware threads and tiles of a level
render in parallel.

| File | Entities | Overview | Levels (tiles written / empty) | Files | Size | Time |
|---|---|---|---|---|---|---|
| `example_2000.dwg` | 68 | 1008 x 1176 | z1 9/0, z2 30/0 | 92 | 1.4 MB | 1.1 s |
| `samples/AutoCADSamples6.dwg` (floor plan) | 4845 | 1568 x 756 | z1 6/2, z2 20/8, z3 51/54 | 189 | 10 MB | 2.1 s |
| `samples/AutoCADSamples5.dwg` (19 891 entities) | 19891 | 1008 x 1176 | z1 9/0 | 236 | 20 MB | 3.4 s |

Sample 5 has no text, so it gets one zoom level; its `report.json` lists 27
exclusions -- two arcs hundreds of times the size of the 20-unit drawing
(`scale_outlier`) and 25 empty TEXT entities thousands of units away
(`far_outlier`). A debug build is 20-40x slower (the `example_2000.dwg`
package takes about 25 s), which is why the export tests dominate `cargo
test`'s wall clock.

## 4. What is not measured yet

- **Token counts per file** with the vendor tokenizer (the design's
  budget of about 3.5k tokens for the manifest is an estimate).
- **Byte-exact goldens**: text is rendered with the host's fonts, so
  tile PNGs differ between machines; the determinism test compares two runs
  on the same machine only.
- **Legibility**: whether the dominant text class really reads at 14 px in
  a given model is a judgement the `legibility.height_classes` block leaves
  to the agent; no VLM-in-the-loop check exists.
- **Frames and paper layouts** (design section 4, steps 5 and 10) are not
  exported, so nothing evaluates them.
