# Pull request drafts: integrate/0.3 -> main

Three pull requests, one per repository, all from the pushed branch `integrate/0.3`.
Each branch already contains its repository's current `main`, so every merge is a
fast-forward. They depend on each other: merge uncad-model first, then
iron-render-cad, then uncad. Nothing here publishes a crate; releases stay with the
maintainer.

---

## uncad-model: carry what the files state for OCS, polylines, text, viewports, layouts

**What this adds.** Fields the parser now reads and the renderer and the LLM/VLM
export consume. Every addition follows the model's principles: `Option` for what a
file may leave unstated, `Ref` for every reference by name or handle, a doc sentence
per field, and `serde(default)` so a 0.1.0 document still loads, except a 0.1.0
polyline. That last case is pinned by a tripwire test.

- Extrusion on TEXT, ATTRIB, ATTDEF and INSERT, alongside the extrusion upstream
  added for CIRCLE, ARC, polylines, SOLID and TRACE. `Affine2::from_insert` applies
  an INSERT's extrusion.
- Per-vertex `start_width` and `end_width` on upstream's `PolylineVertex`, and
  `const_width` on the entity.
- Text placement: horizontal and vertical justification, the alignment point,
  width factor, oblique angle and the style name. Attribute flags.
- MTEXT: `rect_width`, the extents and the style name.
- VIEWPORT: the stated view, `on`, `viewport_id` and the frozen layers.
- DIMENSION: the ordinate axis. MLINE: its scale. The new variant `Entity::PolylineMesh`.
- LayerRecord: off, frozen, locked, plot, lineweight and linetype.
- DimStyleRecord: the rest of the display variables, as `Option`.
- `Tables.layouts`: a LayoutRecord with its plot settings.
- Golden cases g11 to g16 pin all of the above.

**Needs your decision.** These are "propose first" under the principles.
- The CadDatabase doc said the model was deliberately without linetypes,
  lineweights and styles. Layers now carry state, lineweight and a linetype name,
  and texts name their style. The linetype and style definitions, dictionaries and
  header variables are still left out.
- A 0.1.0 polyline document no longer loads, because of your `PolylineVertex`
  change combined with ours. This is a breaking 0.2.0.

**Tests.** 59 pass. The fields were checked against a DXF twin of every drawing
where one exists.

---

## iron-render-cad: robustness, hidden entities, sheets, and a render-once API

**What this adds.**
- Robustness: a PNG size cap with an error, where `example_2000.dwg` used to abort
  on a 36 TB allocation. Limits on block expansion and SVG size are reported in a
  `LimitReport`. A rasterizer panic is caught, NaN is screened, and XML-illegal
  characters are dropped.
- Correctness: a render origin for drawings far from 0, where f32 in
  usvg/tiny-skia lost lines. RAY and XLINE are clipped to the picture. POINT is
  drawn as a pixel-sized cross, and MTEXT keeps its blank lines. Text is drawn at
  cap height. Layer-0 geometry inside a block inherits the reference's layer.
  Text justification and text boxes, attribute invisibility, and MLINE scale.
- Hidden-entity rules for layers that are off, frozen, non-plotting or DEFPOINTS,
  with `include_hidden` and a hidden count.
- `layout_to_svg` and `layout_to_png`: a paper layout with its viewports composited.
- A render-once, assemble-many surface: a Scene of per-entity parts, a window
  assembly, crop modes with a report, text-box measurement and sheet reports. The
  LLM/VLM export builds on it.

**Needs your decision.** Everything below is a new public type or function, so it
is "propose first" under section 9. Fields added to returned results need no
discussion under your newest principle.
- `PngSize`, `Fonts` (with `Custom(bytes)`, used to inject a font; nothing is
  bundled here), `Background`, `DEFAULT_MAX_EDGE`, `DEFAULT_CAP_HEIGHT`.
- `limits::{LimitReport, Dropped, Cap}`.
- `layout_to_svg`, `layout_to_png` and `LayoutError`.
- The Scene API: each item's reason is in commit 6e33818.

**Behaviour taken from your side.** The single-pass text code reader, OCS drawing
for circles, arcs, polylines, SOLID and TRACE, bulge arcs and 3DFACE hidden edges
all replaced our versions. The remaining differences are listed in the merge commit
message: %%c, %%nnn range, and the tolerance-stack spelling.

**Tests.** 265 pass. No test compares rendered images.

---

## uncad: R2007+ DXF, code pages, header, every new field, and uncad-export

**What this adds.**
- Five local patches to the vendored LibreDWG, listed in `crates/libredwg-sys/NOTICE.md`.
  `build.rs` counts their markers so a re-vendor cannot drop one silently:
  - R2007+ DXF table names are compared decoded.
  - A corrupt date no longer aborts the process in strftime.
  - A polygon-mesh DXF is no longer refused whole (two patches).
  - An entity's true colour and transparency are no longer swapped.
- R2007+ DXF is read instead of refused. A guard turns an R2007+ DXF that would
  come out empty into an error. The 27 readable corpus files match their DWG twins.
- TextDecoder handles the DOS-era Big5, GB2312 and CP932 code pages.
  `parse_bytes`, `Format` and `ParseError::Io` are added, and a non-ASCII path
  now opens.
- `uncad::Header`, returned beside the model by `parse_with_header`. Unstated
  values are `None`.
- Every new model field is read. VERTEX records are no longer entities, and
  POLYLINE_MESH, true colour, layer state, layouts and viewports are read.
  Subentity walks are bounded.
- `crates/uncad-export` is new. It writes the LLM/VLM package: overview, tile
  pyramid, sheets, records, strings index, manifest. It bundles a Noto Sans KR
  subset under OFL-1.1, granted in `deny.toml`.
- The CLI adds `uncad export`, `--version` and `--include-hidden`. Unknown options
  are now errors.

**Needs your decision.** Three reverse documented positions.
- CAVEATS said the vendored copy is unmodified. It now carries five patches, each
  measured and pinned by a test.
- CAVEATS said R2007+ DXF "cannot be fixed on this side". It is now read, with the
  twin comparison as evidence.
- Header variables are returned beside the model rather than inside it.

**Tests.** 304 in the workspace, the CLI's 23 included. The corpus sweep pins were
re-measured, with the reasons in the commit messages. On five drawings the export's
counts match the earlier monolithic implementation. The two differences come from
your attribute walk reading more ATTDEF/ATTRIB records, which is intended.

**Release order when you are ready.** uncad-model 0.2.0, then iron-render-cad 0.2.0,
then libredwg-sys 0.3.0, uncad 0.3.0, uncad-export 0.1.0 and uncad-cli 0.3.0. Publish
from a clean checkout without any local `[patch]`.
