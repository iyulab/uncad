//! A DWG/DXF drawing as a package an LLM or a vision model can read.
//!
//! [`uncad`] parses a drawing into the neutral [`uncad_model`] entity model
//! and draws nothing; `iron-render-cad` draws that model and knows nothing
//! of who looks at the picture. This crate is the consumer between them that
//! writes for one kind of reader: images sized to a model's patch budget,
//! and JSON records with the exact numbers a picture cannot give --
//! lengths, areas, dimension values, texts -- each pointing at the pixels it
//! is drawn in.
//!
//! [`export_package`] writes the package of a parsed drawing (with its
//! [`uncad::Header`], which the model does not carry), [`export_file`] of a
//! file:
//!
//! ```text
//! dir/
//!   README.txt        reading order
//!   manifest.json     source, units, profile, crop, overview, frames, frames_dropped,
//!                     legibility, capabilities, counts, warnings, files, shard_index,
//!                     legend, guidance
//!   drawing.json      header, units, layers with their state, the block definitions, counts
//!   overview.png      the whole crop, fitted to the profile (Claude: <= 1568 px edge,
//!                     <= 1568 patches)
//!   frames/fN/tiles/z{z}/r{rr}_c{cc}.png + .json   tiles (Claude: 1092 px, 224 px
//!                     overlap) and their sidecars
//!   frames/fN/overview.png   one per frame when the drawing splits into several
//!   tiles.json        every tile of every level and frame: written (with its bytes and
//!                     sha256) or empty (with a reason)
//!   texts.json        TEXT/MTEXT/ATTRIB/TOLERANCE, block contents included, with the
//!                     world box of their glyph outlines and the tiles they are on
//!   dimensions.json   measured value and where it came from, display string, points
//!   geometry.json     every other visible entity: key points, length, area, bbox
//!   regions.json      closed polylines: area, perimeter, centroid, the texts inside
//!   blocks.json       the INSERT instances with their attributes
//!   strings.json      NFKC-normalised string -> record ids
//!   report.json       excluded and hidden entities with reasons, unsupported types,
//!                     the renderer's robustness limits
//!   drawing.svg       with `svg: true`;  entities.json  with `full: true`
//! ```
//!
//! Every JSON file carries `"$schema": "uncad-package/1"` and a `units`
//! block; record files above `shard_kb` are split into `name.NNN.json` and
//! listed in the manifest's `shard_index`. The package is the same bytes for
//! the same input and options, `report.json` included. Re-exporting into a
//! directory first clears what the previous `manifest.json` listed, and
//! leaves anything else there alone.
//!
//! The drawing is walked once (the renderer's `Scene`), and every image is
//! a window of that walk: the overview, one per frame (a detail drawn
//! beside the plan is a frame of its own), and a pyramid of overlapping
//! tiles per frame, 2x per level, as deep as the frame's dominant text needs
//! to reach the target pixel height and the tile budget allows. A tile is
//! drawn from the parts that reach it, on as many threads as there are.
//!
//! A text record's box is measured from the glyph outlines of the bundled
//! font ([`fonts`]) as the renderer laid them out, and those boxes widen the
//! drawn extents before the frames are grouped and the tiles culled; a
//! record's id is the model's reference ID (a path of them for a text
//! inside a block), and the file's handle is beside it. What the package
//! derives -- a text's readable string ([`text`]), a dimension's value, the
//! trust in its stored measurement and its label ([`dimension`]), lengths,
//! areas and outlines ([`geom`]), the frame of each picture ([`frame`]) --
//! is computed here, not by the model or the renderer.
//!
//! # Licence
//!
//! GPL-3.0-or-later, like the rest of the workspace, and OFL-1.1 for the
//! bundled font (`fonts/OFL-NotoSansKR.txt`).

pub mod dimension;
pub mod fonts;
pub mod frame;
pub mod geom;
pub mod text;

mod package;

pub use frame::{CropMode, CropSource, Rect};
pub use package::{
    compact_string, export_file, export_package, normalize_string, Counts, CropReport,
    ExcludeReason, Excluded, ExportError, ExportOptions, ExportReport, FrameReport, HeightClass,
    ImageInfo, LevelInfo, Profile, WrittenFile, SCHEMA,
};
