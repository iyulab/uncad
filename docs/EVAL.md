# Evaluation: what the tests measure and the numbers they last produced

This file records how the 0.3.0 "Readable" work is checked beyond the unit
tests, and the numbers those checks produced.

**Everything below was measured on 2026-09-22** against the `fix/docs` branch
(a merge of the two rounds of 0.3.0 fixes), on Windows 11 with rustc 1.95.0,
on an AMD Ryzen 5 6600H with 12 logical cores, **release profile** unless a
line says otherwise. Sizes are decimal MB (10^6 bytes) of the whole package
directory; times are wall clock around the process, the median of three runs,
parsing included. Rerun the commands below and replace the tables when
something changes.

## 1. Corpus sweep

`crates/uncad/tests/corpus_sweep.rs` parses every `.dwg` and `.dxf` under
`lib/libredwg/test/test-data` (LibreDWG's own corpus: 141 DWG and 67 DXF,
R1.4 to R2018) and renders each parsed file to the default PNG. It fails
only on a panic; parse failures are counted and listed. It is ignored by
default because it takes minutes in a debug build:

```bash
cargo test --release -p uncad --test corpus_sweep -- --ignored --nocapture
```

Last run (release profile, 9.4 s for the whole test):

| ext | files | parsed | rendered | entities | excluded by crop | hidden | seconds |
|---|---|---|---|---|---|---|---|
| dwg | 141 | 141 | 141 | 3799 | 16 | 3353 | 5.5 |
| dxf | 67 | 58 | 58 | 843 | 8 | 110 | 3.7 |

No panics. The nine DXF files LibreDWG refuses are its own known limits
(its DXF reader is documented as incomplete): `2000/TS1.dxf`,
`2004/Surface.dxf`, `2013/gh109_1.dxf`, `2018/Constraints.dxf`,
`2018/Dynblocks.dxf`, `2018/LiveSection1.dxf`, `2018/TS1.dxf` and
`example_r12.dxf` fail with error 2048, `r1.4/entities.dxf` with 4096. The
"hidden" column is dominated by dimension definition points on `DEFPOINTS`
inside dimension blocks (`docs/CAVEATS.md`, "Hidden entities"). The entity,
exclusion and hidden counts are the same as the run recorded before the two
rounds of fixes; only the timing changed with the profile (the earlier run
was a debug build, 232 s).

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
5. What is not in the picture? (`report.json`: the 3256x INSERT handle `756`
   and its ATTRIB `757`, both `scale_outlier`, matching the manifest's count;
   one entity on the frozen `ADSK_SYSTEM_LIGHTS` layer)

`crates/uncad/tests/export.rs` adds the structural checks (20 of them): every
file the design lists, the overview within the profile's edge and patch
budget, the tile grid formula per level, sidecar affines that round-trip,
every world rectangle written as the documented `[x0, y0, x1, y1]` array,
sidecars under their 32 KB cap, records that point only at written tiles,
capitals drawn as tall as the CAD text height, a re-export clearing the
previous package and nothing else, and byte-identical output on a second run
(`report.json` excepted, for its timings).

## 3. Export timings and package sizes (release profile)

`cargo build --release -p uncad-cli`, then `uncad export <file> -o <dir>`
with the defaults (Claude profile, up to 5 levels, 400 tiles). Tiles of a
level render in parallel, so the timings depend on the core count given
above. "Entities" is `manifest.json`'s `counts.entities` (model space; the
sheets add the paper-space ones). "Frames and levels" gives tiles written /
tiles empty per zoom level, per frame.

| File | Entities | Overview px | Frames and levels | Files | Size | Time |
|---|---|---|---|---|---|---|
| `example_2000.dwg` | 68 | 1008 x 1176 | f0: z1 9/0, z2 27/3 | 89 | 1.13 MB | 0.47 s |
| `example_2018.dxf` | 70 | 1036 x 1148 | f0: z1 9/0, z2 25/5 | 85 | 0.93 MB | 0.53 s |
| `samples/AutoCADSamples1.dwg` | 6798 | 952 x 1260 | f0: z1 6/0, z2 30/0, z3 89/19 | 303 | 13.8 MB | 1.25 s |
| `samples/AutoCADSamples2.dwg` | 5656 | 1204 x 1008 | f0: z1 8/0, z2 21/0; f1: z1 4/0, z2 21/0 | 153 | 5.4 MB | 1.81 s |
| `samples/AutoCADSamples3.dwg` | 6486 | 1456 x 280 | f0: z1 9/0, z2 18/7, z3 66/44; f1: z1 6/0; f2: z1 9/0 | 268 | 13.9 MB | 1.45 s |
| `samples/AutoCADSamples4.dwg` | 2979 | 1288 x 952 | f0: z1 8/0, z2 21/0, z3 94/4; f1: z1 4/0, z2 14/0 | 314 | 9.3 MB | 1.11 s |
| `samples/AutoCADSamples5.dwg` | 19891 | 1568 x 756 | f0: z1 8/0 | 223 | 21.7 MB | 3.69 s |
| `samples/AutoCADSamples6.dwg` (floor plan) | 4845 | 1568 x 756 | f0: z1 6/0, z2 24/0; f1: z1 3/0, z2 14/0 | 129 | 7.9 MB | 0.95 s |
| `samples/AutoCADSamples7.dwg` | 5351 | 1316 x 924 | f0: z1 6/0, z2 24/0, z3 107/1 | 312 | 10.7 MB | 2.11 s |

Every one of the nine exits 0 and writes one sheet image per paper layout:
two each for `example_2000.dwg` and `example_2018.dxf` (Letter), one for each
sample.

Sample 5 has no text at all, so the depth rule gives it one zoom level and
its 19 891 entities land on 8 tiles. It is still the slowest run and the
largest package, and the records are why, not the images: 19 853 geometry
and 4073 region records shard into 195 JSON files taking 18.7 of the 21.7 MB,
against 2.1 MB of tiles and 0.8 MB of overviews.

Its `report.json` lists 25 exclusions, all empty
TEXT entities thousands of units from the 44-unit drawing (`far_outlier`);
the two large fillet arcs that used to be `scale_outlier`s stopped being
outliers once an ARC was bounded by its own sweep rather than its whole
circle. `example_2000.dwg`'s two exclusions are the 3256x INSERT (handle
`756`) and its ATTRIB (`757`).

Sample 1 is the sheet-heavy case: 6798 model entities on a 36 x 54 inch
`UserDefinedImperial` sheet whose rectangle comes from the layout's own
limits (`rect_source: "layout_limits"`), with one overall viewport left alone
and one 1:16 viewport composited.

A debug build is about 17x slower: the `example_2000.dwg` package takes 7.8 s
against 0.47 s, which is why the export tests dominate `cargo test`'s wall
clock.

## 4. What is not measured yet

- **Token counts per file** with the vendor tokenizer (the design's
  budget of about 3.5k tokens for the manifest is an estimate).
- **Byte-exact goldens**: with the bundled font, tile PNGs should now be
  identical across machines with the same resvg version, but no reference
  package is checked in yet; the determinism test compares two runs on the
  same machine only.
- **Correctness against an independent reference**: the corpus sweep proves
  nothing panics and the counts are stable run to run, not that they are
  right. No file's entity count, area or dimension value has been compared
  with AutoCAD or another reader.
- **Legibility**: whether the dominant text class really reads at 14 px in
  a given model is a judgement the `legibility.height_classes` block leaves
  to the agent; no VLM-in-the-loop check exists.
- **Frames** are checked structurally (`tests/export.rs`: two islands of
  lines become two frames with their own overviews and tiles) and by eye on
  the samples; whether the 5 % gap splits real drawings the way a reader
  would is not measured. Four of the nine files above are split into more
  than one frame (samples 2, 4 and 6 into two, sample 3 into three).
- **Paper layouts** are checked on `example_2000.dwg` (two Letter sheets,
  overall viewports left alone), on the twisted viewport fixture (the model
  line composited at scale 2 and 30 degrees, pixels sampled on and off the
  line) and on `viewport_states_r2000.dxf`, which pins which viewport states
  composite and where the paper lands for each plot rotation and paper unit.
  The twist sign has no AutoCAD reference plot to compare against.
