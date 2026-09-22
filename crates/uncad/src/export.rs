//! The LLM/VLM package (docs/VLM_EXPORT_DESIGN.md, sections 2, 3 and 5):
//! [`export_package`] turns a parsed drawing into a directory an agent can
//! read -- an overview image sized for the model, a pyramid of overlapping
//! tiles with JSON sidecars saying what is on each, and JSON records with
//! the exact numbers (lengths, areas, dimension values, texts) that a
//! picture cannot give.
//!
//! What this 0.3.0 form writes:
//!
//! ```text
//! dir/
//!   README.txt        reading order
//!   manifest.json     source, units, profile, crop, overview, frames, sheets, legibility, capabilities, counts, warnings, files, shard_index, guidance
//!   drawing.json      header, units, layers with their state, blocks, counts
//!   overview.png      the whole crop, fitted to the profile (Claude: <= 1568 px edge, <= 1568 patches)
//!   frames/fN/tiles/z{z}/r{rr}_c{cc}.png + .json   tiles (1092 px, 224 px overlap) and sidecars
//!   frames/fN/overview.png   one per frame when the drawing splits into several; a single-frame drawing has overview.png alone
//!   tiles.json        every tile of every level and frame, written or empty
//!   sheets.json       the paper layouts: sheet rectangle and its source, plot settings, viewports
//!   sheets/<layout>/overview.png   each layout, with the model composited through its viewports
//!   texts.json        TEXT/MTEXT/ATTRIB, block contents included, with world boxes and tiles
//!   dimensions.json   measured value, display string, definition points
//!   geometry.json     every other visible entity: key points, length, area, bbox, tiles
//!   regions.json      closed polylines: area, perimeter, centroid, the texts inside
//!   blocks.json       block definitions and INSERT instances with attributes
//!   strings.json      NFKC-normalised string -> record ids
//!   report.json       excluded and hidden entities with reasons, unsupported types, timings
//!   drawing.svg       with `svg: true`;  entities.json  with `full: true`
//! ```
//!
//! Two manifest fields are worth knowing about: `capabilities` says which
//! questions this package answers exactly, and `svg_origin` is the origin
//! `drawing.svg` is written relative to -- set only when the drawing sits
//! far enough from zero that single-precision rasterizing would lose it.
//!
//! Every JSON file carries `"$schema": "uncad-package/1"` and a `units`
//! block; record files above `shard_kb` are split into `name.NNN.json` and
//! listed in the manifest's `shard_index`. Tile sidecars are written compact,
//! the form their 32 KB cap measures. Output is deterministic for a given
//! input and options (maps are sorted, records ordered by id) except for
//! `report.json`'s timings. Re-exporting into a directory first clears what
//! the previous `manifest.json` listed, and leaves anything else there alone.
//!
//! `texts.json` boxes are measured from the shaped glyph outlines of the
//! bundled `Uncad Sans` ([`crate::png::Fonts::Bundled`], `bbox_confidence:
//! "measured"`), and those boxes widen the drawn extents before the frames
//! are grouped and the tiles culled. The crop is decided before any of that,
//! from the 0.6-em estimate in [`crate::text`].
//!
//! Still open (0.4.0): lineweights and linetypes in the picture, MINSERT,
//! frames grouped by drawing scale rather than by proximity, and feeding the
//! measured text boxes back into the crop.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::crop::{self, CropMode, CropReport, Extent, Rect};
use crate::model::{Entity, InsertEntity, Point2D, Point3D};
use crate::png::{self, Fonts, PngError};
use crate::svg::{self, Space, ToSvgOptions, ViewBox};
use crate::text::{estimate_mtext_box, estimate_text_box};
use crate::visibility::hidden_reason;
use crate::{CadDatabase, ToJsonOptions};

/// The `$schema` value every JSON file in the package carries.
pub const SCHEMA: &str = "uncad-package/1";

/// The image budget of one model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub name: &'static str,
    /// The overview's longest edge, in pixels.
    pub overview_edge: u32,
    /// The most patches (lattice x lattice squares) the overview may hold.
    pub overview_patches: u32,
    /// Tile edge, in pixels.
    pub tile: u32,
    /// Overlap between neighbouring tiles, in pixels.
    pub overlap: u32,
    /// The patch size every image size is rounded up to.
    pub lattice: u32,
}

impl Profile {
    /// Claude's standard tier: 1568 px edge, 1568 patches of 28 px.
    pub const CLAUDE: Profile = Profile {
        name: "claude",
        overview_edge: 1568,
        overview_patches: 1568,
        tile: 1092,
        overlap: 224,
        lattice: 28,
    };
    /// Claude's high-resolution tier.
    pub const CLAUDE_HIRES: Profile = Profile {
        name: "claude-hires",
        overview_edge: 2576,
        overview_patches: 4784,
        tile: 1932,
        overlap: 392,
        lattice: 28,
    };
    /// OpenAI's patch-based models (gpt-5.x): 32 px patches.
    pub const OPENAI_PATCH: Profile = Profile {
        name: "openai-patch",
        overview_edge: 2048,
        overview_patches: 4096,
        tile: 1600,
        overlap: 320,
        lattice: 32,
    };

    pub fn by_name(name: &str) -> Option<Profile> {
        [
            Profile::CLAUDE,
            Profile::CLAUDE_HIRES,
            Profile::OPENAI_PATCH,
        ]
        .into_iter()
        .find(|p| p.name == name)
    }

    fn step(&self) -> u32 {
        self.tile - self.overlap
    }
}

impl Default for Profile {
    fn default() -> Self {
        Profile::CLAUDE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    pub profile: Profile,
    /// The deepest zoom level written (levels are `z1..=z_max`, each 2x the
    /// previous). Default 5.
    pub max_levels: u32,
    /// The most tiles written across all levels; deeper levels are dropped
    /// whole when this would be exceeded. Default 400.
    pub max_tiles: usize,
    /// The pixel height the dominant text class should reach at the deepest
    /// level. Default 14.
    pub target_text_px: f64,
    pub crop: CropMode,
    /// The padding around the crop, in drawing units, for the overview and
    /// each frame's window. `None` (the default) is
    /// [`crate::crop::auto_padding`]: 2 % of the longer side, at least 24
    /// output pixels. The sheets keep their own zero padding -- a sheet is
    /// the paper exactly. Since 0.3.0.
    pub padding: Option<f64>,
    /// Draw hidden entities at 50 % (they never enter the records or the
    /// crop). Default `false`.
    pub include_hidden: bool,
    /// Record files larger than this are sharded. Default 96.
    pub shard_kb: usize,
    /// Also write `drawing.svg`. Default `false`.
    pub svg: bool,
    /// Also write `entities.json`, the whole model. Default `false`.
    pub full: bool,
    /// What the manifest records as the source's name (a file name, say).
    pub source_name: Option<String>,
    /// Entities closer than this fraction of the crop's diagonal belong to
    /// the same group; a detached group becomes its own frame. Default 0.05.
    pub frame_gap: f64,
    /// A detached group needs this many entities, or one text, to become a
    /// frame. Default 20.
    pub min_frame_entities: usize,
    /// The most frames written (the primary one included); further groups
    /// stay in the overview only and are listed as dropped. Default 8.
    pub max_frames: usize,
    /// The fonts text is shaped and drawn with. Default [`Fonts::Bundled`].
    pub fonts: Fonts,
    /// Write `sheets.json` and one image per paper layout. Default `true`.
    pub sheets: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            profile: Profile::CLAUDE,
            max_levels: 5,
            max_tiles: 400,
            target_text_px: 14.0,
            crop: CropMode::Auto,
            padding: None,
            include_hidden: false,
            shard_kb: 96,
            svg: false,
            full: false,
            source_name: None,
            frame_gap: 0.05,
            min_frame_entities: 20,
            max_frames: 8,
            fonts: Fonts::Bundled,
            sheets: true,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ExportError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Render(PngError),
    Json(serde_json::Error),
    /// The whole-model `entities.json` could not be serialized.
    Model(crate::JsonError),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io { path, source } => {
                write!(f, "cannot write {}: {source}", path.display())
            }
            ExportError::Render(e) => write!(f, "rendering failed: {e}"),
            ExportError::Json(e) => write!(f, "JSON serialization failed: {e}"),
            ExportError::Model(e) => write!(f, "entities.json failed: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<PngError> for ExportError {
    fn from(e: PngError) -> Self {
        ExportError::Render(e)
    }
}

impl From<serde_json::Error> for ExportError {
    fn from(e: serde_json::Error) -> Self {
        ExportError::Json(e)
    }
}

impl From<crate::JsonError> for ExportError {
    fn from(e: crate::JsonError) -> Self {
        ExportError::Model(e)
    }
}

/// One image of the package: where it is and how its pixels map to the
/// drawing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageInfo {
    pub id: String,
    pub png: String,
    /// `[width, height]` in pixels.
    pub px: [u32; 2],
    /// The world rectangle the image shows.
    pub world: Rect,
    /// Pixels per drawing unit.
    pub ppu: f64,
    /// Row-major `[a, b, c, d, e, f]` with `px = a x + b y + c` and
    /// `py = d x + e y + f` (the design's `[s, 0, -x0 s, 0, -s, y1 s]`).
    pub world_to_px: [f64; 6],
    pub px_to_world: [f64; 6],
}

impl ImageInfo {
    fn new(id: &str, png: &str, world: Rect, ppu: f64, width: u32, height: u32) -> ImageInfo {
        ImageInfo {
            id: id.to_string(),
            png: png.to_string(),
            px: [width, height],
            world,
            ppu,
            world_to_px: [ppu, 0.0, -world.min_x * ppu, 0.0, -ppu, world.max_y * ppu],
            px_to_world: [1.0 / ppu, 0.0, world.min_x, 0.0, -1.0 / ppu, world.max_y],
        }
    }

    /// A world rectangle in this image's pixels, `[x0, y0, x1, y1]` with y
    /// down, rounded to whole pixels.
    fn px_box(&self, r: &Rect) -> [i64; 4] {
        let x0 = ((r.min_x - self.world.min_x) * self.ppu).floor();
        let x1 = ((r.max_x - self.world.min_x) * self.ppu).ceil();
        let y0 = ((self.world.max_y - r.max_y) * self.ppu).floor();
        let y1 = ((self.world.max_y - r.min_y) * self.ppu).ceil();
        [x0 as i64, y0 as i64, x1 as i64, y1 as i64]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelInfo {
    pub z: u32,
    pub ppu: f64,
    pub canvas_px: [u32; 2],
    pub cols: u32,
    pub rows: u32,
    pub tile_px: u32,
    pub overlap_px: u32,
    pub step_px: u32,
    pub tiles_written: usize,
    pub tiles_empty: usize,
}

/// One text height class of a frame and how legible it is at the deepest
/// level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeightClass {
    pub height: f64,
    pub count: usize,
    pub px_at_zmax: f64,
    pub legible: bool,
}

/// A frame: a region of the drawing with its own overview and tile pyramid
/// under `frames/<id>/`. `f0` is the primary frame (the largest connected
/// group of entities); detached groups -- a detail drawn beside the plan --
/// get `f1`, `f2`, ...
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrameReport {
    pub id: String,
    /// `primary` or `detached`.
    pub kind: String,
    /// The tight bounds of the frame's entities.
    pub content: Rect,
    pub entities: usize,
    pub texts: usize,
    pub overview: ImageInfo,
    pub levels: Vec<LevelInfo>,
    pub z_max: u32,
    /// `false` when the tile budget cut the pyramid short.
    pub reached: bool,
    pub height_classes: Vec<HeightClass>,
}

/// A viewport on a sheet, as `sheets.json` lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetViewport {
    pub handle: String,
    pub id: u16,
    pub on: bool,
    /// The sheet's own frame (the paper), never composited.
    pub overall: bool,
    /// Whether the model was drawn through it.
    pub composited: bool,
    /// The frame on the sheet, paper units.
    pub frame: Rect,
    /// Paper units per model unit.
    pub scale: Option<f64>,
    pub twist_deg: f64,
    /// World corners of the model window (lower-left, lower-right,
    /// upper-right, upper-left on the sheet).
    pub model_window: Option<[[f64; 2]; 4]>,
    pub frozen_layers: Vec<String>,
}

/// One paper layout: its sheet, its viewports and its image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetReport {
    pub name: String,
    pub tab_order: u16,
    pub block: String,
    /// `mm`, `in` or `px` -- the layout's paper unit.
    pub units: String,
    /// The plot settings, when the file has a LAYOUT for this sheet.
    pub plot: Option<crate::tables::PlotSettings>,
    /// The sheet rectangle in paper units and where it came from, in order
    /// of preference: `layout_limits` (the LAYOUT's LIMMIN/LIMMAX, AutoCAD's
    /// own placement of the paper, taken whenever they span a rectangle),
    /// `paper_size` (computed from the plot settings, see
    /// [`crate::tables::PlotSettings::sheet_rect`]), `entities` (the paper
    /// entities' extents) or `empty`.
    pub rect: Rect,
    pub rect_source: String,
    pub overview: ImageInfo,
    pub viewports: Vec<SheetViewport>,
    pub entities: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WrittenFile {
    pub path: String,
    /// `None` for the three files written after the listing was made
    /// (`manifest.json`, `README.txt`, `report.json` with its timings).
    pub bytes: Option<u64>,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Counts {
    pub entities: usize,
    pub texts: usize,
    pub dimensions: usize,
    pub geometry: usize,
    pub regions: usize,
    pub blocks: usize,
    pub hidden: usize,
    pub excluded: usize,
    pub tiles: usize,
    pub frames: usize,
    pub sheets: usize,
}

/// What [`export_package`] wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    pub dir: PathBuf,
    pub files: Vec<WrittenFile>,
    /// The whole crop in one image.
    pub overview: ImageInfo,
    /// The frames, `f0` first.
    pub frames: Vec<FrameReport>,
    /// The paper layouts, in tab order.
    pub sheets: Vec<SheetReport>,
    pub crop: CropReport,
    pub counts: Counts,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- records

/// One entity's export record before its images are known.
struct Record {
    id: String,
    bbox: Rect,
    value: Map<String, Value>,
}

/// A text placed in the world (top level or inside a block reference).
struct PlacedText {
    id: String,
    kind: &'static str,
    layer: String,
    text: String,
    raw: String,
    height: f64,
    rotation: f64,
    anchor: Point2D,
    /// The estimate at first; the metrics pre-pass replaces it with the
    /// shaped glyphs' box.
    bbox: Rect,
    /// `estimated` or `measured`.
    bbox_confidence: &'static str,
    /// Glyphs the font could not shape (drawn as .notdef boxes); 0 when
    /// every character was covered.
    unshaped: usize,
    style: String,
    tag: Option<String>,
}

/// A 2D affine `p' = (a x + c y + e, b x + d y + f)`: one INSERT's
/// placement (`origin + R(rotation) diag(sx, sy)`, with a mirrored OCS
/// folded in as a negative x scale and a negated rotation, as the renderer
/// does) or the composition of nested ones. A full matrix, because the
/// origin + rotation + scale form cannot represent a rotated child under a
/// mirrored or non-uniformly scaled parent (`diag(-1, 1) R(t) = R(-t)
/// diag(-1, 1)`): the sum-of-rotations composition reflected such a
/// child's texts about the parent's insertion point.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn placement(origin: Point2D, x_scale: f64, y_scale: f64, rotation: f64) -> Affine {
        let (c, s) = (rotation.cos(), rotation.sin());
        Affine {
            a: x_scale * c,
            b: x_scale * s,
            c: -y_scale * s,
            d: y_scale * c,
            e: origin.x,
            f: origin.y,
        }
    }

    fn for_insert(i: &InsertEntity) -> Affine {
        let (x_scale, rotation) = if i.extrusion.z < 0.0 {
            (-i.scale.x, -i.rotation)
        } else {
            (i.scale.x, i.rotation)
        };
        Affine::placement(p2(i.insertion_point), x_scale, i.scale.y, rotation)
    }

    fn apply(&self, p: Point2D) -> Point2D {
        Point2D {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// The axis-aligned box of `r`'s four transformed corners.
    fn apply_rect(&self, r: &Rect) -> Rect {
        let corners = [
            self.apply(Point2D {
                x: r.min_x,
                y: r.min_y,
            }),
            self.apply(Point2D {
                x: r.max_x,
                y: r.min_y,
            }),
            self.apply(Point2D {
                x: r.max_x,
                y: r.max_y,
            }),
            self.apply(Point2D {
                x: r.min_x,
                y: r.max_y,
            }),
        ];
        let mut out = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for p in corners {
            out.min_x = out.min_x.min(p.x);
            out.min_y = out.min_y.min(p.y);
            out.max_x = out.max_x.max(p.x);
            out.max_y = out.max_y.max(p.y);
        }
        out
    }

    /// `self` then `outer`: the transform of a child placed by `self`
    /// inside a block that `outer` places (the matrix product `outer *
    /// self`).
    fn then(&self, outer: &Affine) -> Affine {
        Affine {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            e: outer.a * self.e + outer.c * self.f + outer.e,
            f: outer.b * self.e + outer.d * self.f + outer.f,
        }
    }

    /// The determinant of the linear part: negative for a mirrored frame.
    fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// The length scale of the frame, `sqrt(|det|)` -- exact for a uniform
    /// scale, the geometric mean of the axis scales otherwise.
    fn length_scale(&self) -> f64 {
        self.det().abs().sqrt()
    }

    /// The world rotation of a text drawn with `rotation` in this frame:
    /// the angle of its transformed *up* axis less 90 degrees, i.e. how
    /// the glyphs are oriented on the page. Through the up axis rather
    /// than the baseline so that a frame mirrored about the vertical axis
    /// (the usual MIRROR of a block) reports an upright, mirror-written
    /// text as 0 rather than 180, as the records did before for a
    /// top-level mirror; in a mirrored frame the string advances the other
    /// way along that orientation.
    fn text_rotation(&self, rotation: f64) -> f64 {
        let (ux, uy) = (-rotation.sin(), rotation.cos());
        let (wx, wy) = (self.a * ux + self.c * uy, self.b * ux + self.d * uy);
        wy.atan2(wx) - std::f64::consts::FRAC_PI_2
    }
}

const MAX_BLOCK_DEPTH: u32 = 8;
const MAX_PLACED_TEXTS: usize = 200_000;

/// The most vertices an outline may have before its self-intersection test
/// is skipped. [`crate::geom::is_simple`] compares every pair of
/// non-adjacent segments, so the test costs O(n^2): a single closed
/// polyline of 64 000 vertices (a surveyed contour or a traced boundary,
/// routine in a GIS import) held the export for 175 s where the same
/// drawing at 16 000 held it for 9 s -- four times the vertices, nineteen
/// times the time -- for one boolean on one record. At this cap the test
/// costs a tenth of a second in an unoptimised build and a few
/// milliseconds in a release one, and no outline in the corpus comes near
/// it: the largest region in `AutoCADSamples5.dwg` has 378 vertices.
const MAX_SIMPLE_TEST_VERTICES: usize = 2_000;

/// What a record says instead of claiming an untested outline is simple.
const UNTESTED_OUTLINE: &str =
    "outline too large to test for self-intersection; the area assumes it does not cross itself";

/// Whether `vertices` outline a non-self-intersecting polygon, or `None`
/// when there are too many of them to ask (see
/// [`MAX_SIMPLE_TEST_VERTICES`]). Unknown is reported as `null`, never as
/// `true`: a crossing outline's area is meaningless, and a reader must be
/// able to tell "checked, fine" from "not checked".
fn simple_outline(vertices: &[Point2D]) -> Option<bool> {
    (vertices.len() <= MAX_SIMPLE_TEST_VERTICES).then(|| crate::geom::is_simple(vertices))
}

fn p2(p: Point3D) -> Point2D {
    Point2D { x: p.x, y: p.y }
}

/// Collects every visible text at the top level and inside visible block
/// references (ids `<insert>/<child>`), transformed to world coordinates.
fn placed_texts(db: &CadDatabase, top: &[&Entity]) -> Vec<PlacedText> {
    let mut out = Vec::new();
    for e in top {
        collect_texts(db, e, &Affine::IDENTITY, "", 0, &mut out);
    }
    // One text can be reached twice: a DXF whose ATTRIB is owned by the
    // block record gives the containing block an ATTRIB child *and* (from
    // R2004 on) links the same ATTRIB into the nested INSERT's `attribs`.
    // Both paths mint the same id, which is also the one `<text>` the
    // renderer draws, so the second is a duplicate.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    out.retain(|t| seen.insert(t.id.clone()));
    out
}

fn collect_texts(
    db: &CadDatabase,
    e: &Entity,
    affine: &Affine,
    prefix: &str,
    depth: u32,
    out: &mut Vec<PlacedText>,
) {
    if out.len() >= MAX_PLACED_TEXTS || hidden_reason(e.common(), &db.tables).is_some() {
        return;
    }
    let id = |handle: &str| {
        if prefix.is_empty() {
            handle.to_string()
        } else {
            format!("{prefix}/{handle}")
        }
    };
    let scale = affine.length_scale();
    match e {
        Entity::Text(t) => {
            if t.text_plain.trim().is_empty() {
                return;
            }
            let base = if t.horizontal_alignment != 0 || t.vertical_alignment != 0 {
                t.alignment_point.unwrap_or(t.start_point)
            } else {
                t.start_point
            };
            let anchor = affine.apply(base);
            let height = t.text_height * scale;
            let rotation = affine.text_rotation(t.rotation);
            // The estimate in the block's own frame, then through the
            // frame: right under a mirror or a non-uniform scale, where a
            // world-space estimate from the anchor and rotation is not.
            let bbox = affine.apply_rect(&estimate_text_box(
                base,
                t.text_height,
                t.rotation,
                &t.text_plain,
                t.width_factor,
                t.horizontal_alignment,
                t.vertical_alignment,
            ));
            out.push(PlacedText {
                id: id(&t.common.handle),
                kind: "TEXT",
                layer: t.common.layer.clone(),
                text: t.text_plain.clone(),
                raw: t.text.clone(),
                height,
                rotation,
                anchor,
                bbox,
                bbox_confidence: "estimated",
                unshaped: 0,
                style: t.style.clone(),
                tag: None,
            });
        }
        Entity::Attrib(a) => {
            if a.invisible || a.text_plain.trim().is_empty() {
                return;
            }
            let base = if a.horizontal_alignment != 0 || a.vertical_alignment != 0 {
                a.alignment_point.unwrap_or(a.start_point)
            } else {
                a.start_point
            };
            let anchor = affine.apply(base);
            let height = a.text_height * scale;
            let rotation = affine.text_rotation(a.rotation);
            let bbox = affine.apply_rect(&estimate_text_box(
                base,
                a.text_height,
                a.rotation,
                &a.text_plain,
                a.width_factor,
                a.horizontal_alignment,
                a.vertical_alignment,
            ));
            out.push(PlacedText {
                id: id(&a.common.handle),
                kind: "ATTRIB",
                layer: a.common.layer.clone(),
                text: a.text_plain.clone(),
                raw: a.text.clone(),
                height,
                rotation,
                anchor,
                bbox,
                bbox_confidence: "estimated",
                unshaped: 0,
                style: a.style.clone(),
                tag: Some(a.tag.clone()),
            });
        }
        Entity::MText(m) => {
            if m.text_plain.trim().is_empty() {
                return;
            }
            let anchor = affine.apply(p2(m.insertion_point));
            let height = m.text_height * scale;
            let rotation = affine.text_rotation(m.rotation);
            let bbox = affine.apply_rect(&estimate_mtext_box(
                p2(m.insertion_point),
                m.text_height,
                m.rotation,
                &m.text_plain,
                m.attachment,
                m.extents_width,
                m.extents_height,
            ));
            out.push(PlacedText {
                id: id(&m.common.handle),
                kind: "MTEXT",
                layer: m.common.layer.clone(),
                text: m.text_plain.clone(),
                raw: m.text.clone(),
                height,
                rotation,
                anchor,
                bbox,
                bbox_confidence: "estimated",
                unshaped: 0,
                style: m.style.clone(),
                tag: None,
            });
        }
        Entity::Insert(i) => {
            if depth >= MAX_BLOCK_DEPTH {
                return;
            }
            let Some(block) = db.tables.block_records.get(&i.block_name) else {
                return;
            };
            let child_affine = Affine::for_insert(i).then(affine);
            let child_prefix = id(&i.common.handle);
            // A *nested* INSERT's attribute values hang off the INSERT
            // itself and nothing else sees them: `convert.rs` duplicates
            // only a top-level INSERT's attribs into the entity list, which
            // is why the top level is left to that walk (`depth > 0`
            // here). They are placed in the containing block's own frame,
            // like the INSERT's insertion point, so they go through the
            // parent affine and the current prefix -- the id the renderer
            // draws them with.
            if depth > 0 {
                for a in &i.attribs {
                    collect_texts(db, &Entity::Attrib(a.clone()), affine, prefix, depth, out);
                }
            }
            for child in &block.entities {
                // Only the attribute *template* is skipped. A block child
                // that is an ATTRIB is the value LibreDWG builds from a DXF
                // whose ATTRIB is owned by the block record: it is drawn,
                // so it belongs in the records too.
                if matches!(child, Entity::Attdef(_)) {
                    continue;
                }
                collect_texts(db, child, &child_affine, &child_prefix, depth + 1, out);
            }
        }
        Entity::AcadTable(t) => {
            // An ACAD_TABLE draws the block it caches its laid-out cells in
            // through the same block-reference path as an INSERT
            // (`svg.rs`), so its cell texts are drawn, with the ids
            // `<table>/<text>` this mints -- and used to be in no text
            // record and no `strings.json` key, so a reader searching for a
            // value they can see in a cell found nothing. The frame is the
            // INSERT one without the extrusion flip: the renderer passes
            // the table's `scale.x` through as it stands.
            if depth >= MAX_BLOCK_DEPTH {
                return;
            }
            let Some(block) = db.tables.block_records.get(&t.block_name) else {
                return;
            };
            let child_affine =
                Affine::placement(p2(t.insertion_point), t.scale.x, t.scale.y, t.rotation)
                    .then(affine);
            let child_prefix = id(&t.common.handle);
            for child in &block.entities {
                if matches!(child, Entity::Attdef(_)) {
                    continue;
                }
                collect_texts(db, child, &child_affine, &child_prefix, depth + 1, out);
            }
        }
        Entity::Tolerance(t) => {
            // A TOLERANCE draws one <text> of its own (the feature control
            // frame's codes and values), anchored at the insertion point,
            // unrotated, at the height `dimension::tolerance_text_height`
            // resolved -- so it is as findable as any other string in the
            // picture only if it is indexed like one.
            if t.text_plain.trim().is_empty() {
                return;
            }
            let base = p2(t.insertion_point);
            out.push(PlacedText {
                id: id(&t.common.handle),
                kind: "TOLERANCE",
                layer: t.common.layer.clone(),
                text: t.text_plain.clone(),
                raw: t.text_value.clone(),
                height: t.text_height * scale,
                rotation: affine.text_rotation(0.0),
                anchor: affine.apply(base),
                bbox: affine.apply_rect(&estimate_text_box(
                    base,
                    t.text_height,
                    0.0,
                    &t.text_plain,
                    1.0,
                    0,
                    0,
                )),
                bbox_confidence: "estimated",
                unshaped: 0,
                style: String::new(),
                tag: None,
            });
        }
        _ => {}
    }
}

// ------------------------------------------------------------- rounding

fn round_to(v: f64, decimals: u16) -> f64 {
    let f = 10f64.powi(i32::from(decimals));
    let r = (v * f).round() / f;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

struct Rounder {
    coords: u16,
    derived: u16,
}

impl Rounder {
    fn coord(&self, v: f64) -> f64 {
        round_to(v, self.coords)
    }

    fn derived(&self, v: f64) -> f64 {
        round_to(v, self.derived)
    }

    fn pt2(&self, p: Point2D) -> Value {
        json!([self.coord(p.x), self.coord(p.y)])
    }

    fn pt3(&self, p: Point3D) -> Value {
        json!([self.coord(p.x), self.coord(p.y)])
    }

    fn rect(&self, r: &Rect) -> Value {
        json!([
            self.coord(r.min_x),
            self.coord(r.min_y),
            self.coord(r.max_x),
            self.coord(r.max_y)
        ])
    }
}

/// The decimals a world coordinate is written with: `$LUPREC` (clamped to
/// 3..=12), but never fewer than the package's own scale needs.
///
/// `$LUPREC` says how precisely the drawing's units are *displayed*; it
/// says nothing about how far the package zooms into them. A drawing a few
/// thousandths of a unit across is drawn at hundreds of thousands of pixels
/// per unit, where LUPREC's three or four decimals put every tile rectangle
/// of a frame on the same numbers: the `world` a consumer reads is then a
/// different rectangle from the one the tile shows, and the affine beside
/// it maps to the wrong pixels.
///
/// The floor is the scale's. `deepest_ppu` is the finest scale the package
/// may reach -- the overview's, doubled once per zoom level -- and the
/// coordinate carries three decimals beyond one pixel there, so one unit in
/// the last place is a thousandth of a pixel. The three (rather than, say,
/// one) also cover a detached frame drawn up to a hundred times finer than
/// the whole crop, which gets its own `fit_overview` and so its own scale.
/// A drawing of ordinary size is drawn at a fraction of a pixel per unit, so
/// its floor is the three or four decimals `$LUPREC` usually asks for
/// anyway.
fn coord_decimals(luprec: u16, deepest_ppu: f64) -> u16 {
    let from_scale = if deepest_ppu.is_finite() && deepest_ppu > 0.0 {
        (deepest_ppu.log10().ceil() + 3.0).clamp(3.0, 12.0) as u16
    } else {
        3
    };
    luprec.clamp(3, 12).max(from_scale)
}

/// Sorts handles numerically (they are hex), then lexically.
fn id_key(id: &str) -> (u64, String) {
    let first = id.split('/').next().unwrap_or(id);
    (
        u64::from_str_radix(first, 16).unwrap_or(u64::MAX),
        id.to_string(),
    )
}

/// The normalisation `strings.json` keys use: Unicode NFKC (so `㎡` is
/// `m2`, `²` is `2`, full-width digits are ASCII), the fraction slash
/// U+2044 as `/`, case folded, whitespace collapsed to single spaces.
pub fn normalize_string(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let folded: String = s.nfkc().collect::<String>().replace('\u{2044}', "/");
    folded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// [`normalize_string`] with every space removed as well, the second key
/// each string is indexed under (so `32.5 m2` and `32.5m2` meet).
pub fn compact_string(s: &str) -> String {
    normalize_string(s).replace(' ', "")
}

// ----------------------------------------------------------- the package

struct Writer<'a> {
    dir: &'a Path,
    files: Vec<WrittenFile>,
    shard_index: Vec<Value>,
    units: Value,
    shard_kb: usize,
}

impl Writer<'_> {
    fn write_bytes(&mut self, rel: &str, bytes: &[u8], kind: &str) -> Result<(), ExportError> {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ExportError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&path, bytes).map_err(|source| ExportError::Io { path, source })?;
        self.files.push(WrittenFile {
            path: rel.to_string(),
            bytes: Some(bytes.len() as u64),
            kind: kind.to_string(),
        });
        Ok(())
    }

    fn write_json(&mut self, rel: &str, value: &Value, kind: &str) -> Result<(), ExportError> {
        let text = serde_json::to_string_pretty(value)?;
        self.write_bytes(rel, text.as_bytes(), kind)
    }

    /// [`Writer::write_json`] without the indentation, for a file whose own
    /// size is the thing being kept: a sidecar drops record rows until its
    /// compact form fits [`SIDECAR_LIMIT`], so pretty-printing it afterwards
    /// put a file three times the measured size on disk -- rows cut to
    /// satisfy a limit the file then broke anyway. Record shards are written
    /// compact for the same reason.
    fn write_json_compact(
        &mut self,
        rel: &str,
        value: &Value,
        kind: &str,
    ) -> Result<(), ExportError> {
        let text = serde_json::to_string(value)?;
        self.write_bytes(rel, text.as_bytes(), kind)
    }

    /// Writes `records` (already sorted by id) as `name.json`, or as
    /// `name.NNN.json` shards under the size rule, and indexes them.
    fn write_records(
        &mut self,
        name: &str,
        kind: &str,
        records: &[Record],
    ) -> Result<(), ExportError> {
        let limit = self.shard_kb.max(1) * 1024;
        let mut shards: Vec<Vec<&Record>> = vec![Vec::new()];
        let mut bytes = 0usize;
        for r in records {
            let size = serde_json::to_string(&r.value)?.len() + 8;
            if bytes + size > limit && !shards.last().is_none_or(Vec::is_empty) {
                shards.push(Vec::new());
                bytes = 0;
            }
            shards.last_mut().expect("one shard").push(r);
            bytes += size;
        }
        let single = shards.len() == 1;
        for (n, shard) in shards.iter().enumerate() {
            let file = if single {
                format!("{name}.json")
            } else {
                format!("{name}.{:03}.json", n + 1)
            };
            let value = json!({
                "$schema": SCHEMA,
                "kind": kind,
                "units": self.units,
                "count": shard.len(),
                "records": shard.iter().map(|r| Value::Object(r.value.clone())).collect::<Vec<_>>(),
            });
            let text = serde_json::to_string(&value)?;
            self.write_bytes(&file, text.as_bytes(), kind)?;
            self.shard_index.push(json!({
                "file": file,
                "kind": kind,
                "first_id": shard.first().map(|r| r.id.as_str()),
                "last_id": shard.last().map(|r| r.id.as_str()),
                "count": shard.len(),
                "bytes": text.len(),
            }));
        }
        Ok(())
    }
}

struct Tile {
    id: String,
    frame: String,
    z: u32,
    row: u32,
    col: u32,
    /// Pixel origin on the level canvas.
    origin_px: (u32, u32),
    width: u32,
    height: u32,
    world: Rect,
    empty: bool,
}

/// Writes the package for `db` into `dir` (created if needed). A previous
/// uncad package in `dir` is cleared first -- every file its
/// `manifest.json` listed, and the directories under `frames/` and
/// `sheets/` that empties -- so a re-export with other options leaves no
/// stale shards or tiles behind; nothing a manifest did not list is
/// touched.
pub fn export_package(
    db: &CadDatabase,
    dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let profile = options.profile;
    std::fs::create_dir_all(dir).map_err(|source| ExportError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    clear_previous_package(dir);
    let mut warnings: Vec<String> = Vec::new();

    // --- render once, decide the crop ---------------------------------
    let svg_options = ToSvgOptions {
        crop: options.crop,
        include_hidden: options.include_hidden,
        padding: None,
        space: Space::Model,
        ..Default::default()
    };
    let rendered = svg::render(db, svg_options);
    let content = rendered.choice.rect;
    let top: Vec<&Entity> = svg::select_entities_for_space(db, Space::Model);
    // Records cover what the picture shows: neither hidden entities nor
    // the ones the crop left out (they are listed in report.json).
    let excluded_handles: BTreeSet<&str> = rendered
        .choice
        .excluded
        .iter()
        .map(|e| e.handle.as_str())
        .collect();
    let shown: Vec<&Entity> = top
        .iter()
        .copied()
        .filter(|e| {
            hidden_reason(e.common(), &db.tables).is_none()
                && !excluded_handles.contains(e.common().handle.as_str())
        })
        .collect();
    let visible: &[&Entity] = &shown;
    // Extents of what is drawn: the crop's exclusions are not, and would
    // otherwise make frames and tiles of their own.
    let mut drawn_extents: Vec<Extent> = rendered
        .extents
        .iter()
        .filter(|e| !excluded_handles.contains(e.handle.as_str()))
        .cloned()
        .collect();

    // --- overview: the whole crop, sized to the profile ---------------------
    let stroke_px = 1.25;
    let fit = fit_overview(&content, &profile, options.padding);
    // The short edge: `fit_overview` pins the long edge at the patch
    // budget, so the aspect ratio can only squeeze the other one.
    if fit.width.min(fit.height) < 200 {
        warnings.push(format!(
            "TinyOverview: the overview is only {} x {} px; the drawing's aspect leaves little of the patch budget",
            fit.width, fit.height
        ));
    }
    let overview_svg = svg::assemble(
        &rendered,
        &ViewBox::from_world(&fit.rect),
        stroke_px / fit.ppu,
    );
    let overview_png = png::render_region(
        &png::parse_tree(&overview_svg, options.fonts)?,
        fit.ppu,
        (0.0, 0.0),
        fit.width,
        fit.height,
    )?;
    let overview = ImageInfo::new(
        "ov",
        "overview.png",
        fit.rect,
        fit.ppu,
        fit.width,
        fit.height,
    );
    let padding = fit.padding;

    // --- records: texts, dimensions, geometry, regions, blocks ---------
    // `max_levels` bounds the zoom (`build_frame` stops at `z_max`), so
    // `fit.ppu` doubled that often is the finest scale any image can have.
    let levels = i32::try_from(options.max_levels)
        .unwrap_or(i32::MAX)
        .min(64);
    let coords = coord_decimals(db.header.luprec, fit.ppu * 2f64.powi(levels));
    let rounder = Rounder {
        coords,
        derived: coords + 2,
    };
    let unit = db.header.units.name.clone();
    let mut texts = placed_texts(db, visible);
    let measured = measure_texts(&rendered, &content, &mut texts, options.fonts)?;
    let unshaped_texts = texts.iter().filter(|t| t.unshaped > 0).count();
    if unshaped_texts > 0 {
        warnings.push(format!(
            "UnshapedGlyphs: {unshaped_texts} texts hold characters the bundled font lacks (drawn as boxes); Fonts::BundledAndSystem / --fonts bundled+system uses the host's fonts for them"
        ));
    }
    // The extents the renderer measured hold `text::CHAR_ADVANCE`
    // (0.8186 text heights) per character; the metrics pass above has the
    // real glyph boxes. A Hangul syllable advances about 0.92 heights, so
    // a long Korean note runs some 12 % past its estimate -- 200 units for
    // a 100-syllable note at height 20 -- and the frames, the frame
    // overviews and the tiles are all culled by these extents, while
    // `texts.json` lists a text's tiles from the measured box. The tile
    // the records named then showed no text at all. Widening each drawn
    // part by the boxes of the texts it draws (the part is the top-level
    // entity or INSERT, i.e. the first segment of a text's id) keeps the
    // two in step, and never shrinks an estimate that was generous.
    widen_extents_with_texts(&mut drawn_extents, &texts);
    let extents: &[Extent] = &drawn_extents;

    // --- frames: the primary group and each detached group -----------------
    // Each tile rasterizes only the entities whose extent touches it (plus
    // a margin for strokes and text overhang): parsing and rendering cost
    // what is on the tile, not the whole drawing.
    let extent_of_handle: std::collections::HashMap<&str, Rect> = extents
        .iter()
        .map(|e| (e.handle.as_str(), e.rect))
        .collect();
    let groups = crop::detached_groups(extents, options.frame_gap * content.diagonal());
    let group_rect =
        |g: &[usize]| Rect::bounding(g.iter().map(|i| &extents[*i].rect)).expect("non-empty");
    let group_texts = |r: &Rect| texts.iter().filter(|t| r.intersects(&t.bbox)).count();
    let mut specs: Vec<(String, &str, Rect, usize)> = Vec::new();
    let mut dropped: Vec<Value> = Vec::new();
    if groups.len() <= 1 {
        specs.push(("f0".to_string(), "primary", content, extents.len()));
    } else {
        for (n, g) in groups.iter().enumerate() {
            let r = group_rect(g);
            let qualifies = n == 0 || g.len() >= options.min_frame_entities || group_texts(&r) > 0;
            if !qualifies {
                continue;
            }
            if specs.len() >= options.max_frames.max(1) {
                dropped.push(json!({ "content": rounder.rect(&r), "entities": g.len(), "texts": group_texts(&r) }));
                continue;
            }
            let id = format!("f{}", specs.len());
            specs.push((id, if n == 0 { "primary" } else { "detached" }, r, g.len()));
        }
        if !dropped.is_empty() {
            warnings.push(format!(
                "MaxFrames: {} detached groups beyond the {} frames written stay in the overview only",
                dropped.len(),
                options.max_frames
            ));
        }
    }
    let mut frame_builds: Vec<FrameBuild> = Vec::new();
    let mut tile_budget = options.max_tiles;
    for (id, kind, frame_content, entities_in) in &specs {
        let reuse = if specs.len() == 1 {
            Some(overview.clone())
        } else {
            None
        };
        let build = build_frame(
            id,
            kind,
            *frame_content,
            *entities_in,
            &texts,
            extents,
            &profile,
            options,
            reuse,
            &mut tile_budget,
            &mut warnings,
        );
        frame_builds.push(build);
    }
    let written_total: usize = frame_builds
        .iter()
        .map(|f| {
            f.report
                .levels
                .iter()
                .map(|l| l.tiles_written)
                .sum::<usize>()
        })
        .sum();

    // --- write images ---------------------------------------------------
    let mut writer = Writer {
        dir,
        files: Vec::new(),
        shard_index: Vec::new(),
        units: json!({
            "name": unit,
            "insunits": db.header.insunits,
            "to_mm": db.header.units.to_mm,
            "source": "header",
        }),
        shard_kb: options.shard_kb,
    };
    writer.write_bytes("overview.png", &overview_png, "image")?;
    let mut tile_images: Vec<ImageInfo> = Vec::new();
    let mut frame_overviews: Vec<ImageInfo> = Vec::new();
    for build in &frame_builds {
        // The frame's own overview, unless it is the whole crop already.
        if build.report.overview.id != "ov" {
            let ov = &build.report.overview;
            let window = ov.world.padded(16.0 / ov.ppu);
            let svg_text = svg::assemble_subset(
                &rendered,
                &ViewBox::from_world(&ov.world),
                stroke_px / ov.ppu,
                |handle| {
                    rendered.unbounded.contains(handle)
                        || extent_of_handle
                            .get(handle)
                            .is_none_or(|rect| rect.intersects(&window))
                },
            );
            let bytes = png::render_region(
                &png::parse_tree(&svg_text, options.fonts)?,
                ov.ppu,
                (0.0, 0.0),
                ov.px[0],
                ov.px[1],
            )?;
            writer.write_bytes(&ov.png, &bytes, "image")?;
            frame_overviews.push(ov.clone());
        }
        for level in &build.report.levels {
            let level_tiles: Vec<&Tile> = build
                .tiles
                .iter()
                .filter(|t| t.z == level.z && !t.empty)
                .collect();
            let margin = 16.0 / level.ppu;
            let rendered_tiles = render_tiles_parallel(
                &rendered,
                &extent_of_handle,
                level.ppu,
                stroke_px,
                margin,
                options.fonts,
                &level_tiles,
            )?;
            for (tile, bytes) in level_tiles.iter().zip(rendered_tiles) {
                let png_path = format!(
                    "frames/{}/tiles/z{}/r{:02}_c{:02}.png",
                    tile.frame, tile.z, tile.row, tile.col
                );
                writer.write_bytes(&png_path, &bytes, "tile")?;
                tile_images.push(ImageInfo::new(
                    &tile.id,
                    &png_path,
                    tile.world,
                    level.ppu,
                    tile.width,
                    tile.height,
                ));
            }
        }
    }

    // --- sheets: one image per paper layout -----------------------------------
    let layer_of: std::collections::HashMap<&str, &str> = visible
        .iter()
        .map(|e| (e.common().handle.as_str(), e.common().layer.as_str()))
        .collect();
    let mut sheet_reports: Vec<SheetReport> = Vec::new();
    let mut sheet_dirs: BTreeSet<String> = BTreeSet::new();
    if options.sheets {
        for spec in sheet_specs(db) {
            let block = &db.tables.block_records[&spec.block];
            let paper_entities: Vec<&Entity> = block
                .entities
                .iter()
                .filter(|e| hidden_reason(e.common(), &db.tables).is_none())
                .collect();
            let paper_rendered = svg::render_selected(
                db,
                paper_entities.clone(),
                ToSvgOptions {
                    crop: CropMode::Raw,
                    padding: Some(0.0),
                    include_hidden: options.include_hidden,
                    ..Default::default()
                },
                "p",
            );
            // The layout's own LIMMIN/LIMMAX first: AutoCAD keeps them equal
            // to the paper's placement (margins and plot origin folded in,
            // rotation included), which the page-setup formula in
            // `PlotSettings::sheet_rect` only approximates.
            let limits_ok = [spec.limmin.x, spec.limmin.y, spec.limmax.x, spec.limmax.y]
                .iter()
                .all(|v| v.is_finite())
                && spec.limmax.x > spec.limmin.x
                && spec.limmax.y > spec.limmin.y;
            let (rect, rect_source) = match spec.plot.as_ref().and_then(|p| p.sheet_rect()) {
                _ if limits_ok => (
                    Rect::new(spec.limmin.x, spec.limmin.y, spec.limmax.x, spec.limmax.y),
                    "layout_limits",
                ),
                Some(r) => (r, "paper_size"),
                None if paper_rendered.choice.content.is_some() => {
                    (paper_rendered.choice.rect, "entities")
                }
                None => (crop::EMPTY_RECT, "empty"),
            };
            // No paper size, no limits and nothing but point-like content (a
            // lone POINT, a zero-length LINE, an empty TEXT): there is no
            // rectangle to fit, and a sheet is fitted with zero padding, so
            // the scale would come out infinite and the render fail with
            // "render size is zero" -- aborting a package that is already
            // half written. One unusable sheet is worth a warning, not the
            // whole export.
            let usable = [rect.min_x, rect.min_y, rect.max_x, rect.max_y]
                .iter()
                .all(|v| v.is_finite() && v.abs() < 1e15)
                && (rect.width() > 0.0 || rect.height() > 0.0);
            if !usable {
                warnings.push(format!(
                    "UnusableSheet: layout {} ({}) has no paper size, no limits and no usable content ({rect_source}); its sheet is skipped",
                    spec.name, spec.block
                ));
                continue;
            }
            // Every viewport of the block, hidden layer or not: a frame on
            // an off, frozen or non-plotting layer (the usual way to hide
            // the border) still shows its model window; only the border
            // goes, since the hidden entity is not among the paper parts.
            let viewports: Vec<&crate::model::ViewportEntity> = block
                .entities
                .iter()
                .filter_map(|e| match e {
                    Entity::Viewport(v) => Some(v),
                    _ => None,
                })
                .collect();
            let composited: Vec<&crate::model::ViewportEntity> = viewports
                .iter()
                .copied()
                .filter(|v| v.on && !v.is_overall() && v.is_plan() && v.scale().is_some())
                .collect();
            let fit = fit_overview(&rect, &profile, Some(0.0));
            let view_box = ViewBox::from_world(&fit.rect);
            let svg_text = svg::assemble_sheet(
                &paper_rendered,
                &rendered,
                &composited,
                &extent_of_handle,
                &layer_of,
                &view_box,
                stroke_px / fit.ppu,
            );
            let bytes = png::render_region(
                &png::parse_tree(&svg_text, options.fonts)?,
                fit.ppu,
                (0.0, 0.0),
                fit.width,
                fit.height,
            )?;
            let png_path = format!(
                "sheets/{}/overview.png",
                sheet_dir(&spec.name, &mut sheet_dirs)
            );
            writer.write_bytes(&png_path, &bytes, "sheet")?;
            let overview_info = ImageInfo::new(
                &format!("sheet:{}", spec.name),
                &png_path,
                fit.rect,
                fit.ppu,
                fit.width,
                fit.height,
            );
            let vp_reports: Vec<SheetViewport> = viewports
                .iter()
                .map(|v| SheetViewport {
                    handle: v.common.handle.clone(),
                    id: v.id,
                    on: v.on,
                    overall: v.is_overall(),
                    composited: composited
                        .iter()
                        .any(|c| c.common.handle == v.common.handle),
                    frame: Rect::new(
                        v.center.x - v.width / 2.0,
                        v.center.y - v.height / 2.0,
                        v.center.x + v.width / 2.0,
                        v.center.y + v.height / 2.0,
                    ),
                    scale: v.scale(),
                    twist_deg: v.twist.to_degrees(),
                    model_window: v
                        .model_window()
                        .map(|w| w.map(|p| [rounder.coord(p.x), rounder.coord(p.y)])),
                    frozen_layers: v.frozen_layers.clone(),
                })
                .collect();
            sheet_reports.push(SheetReport {
                name: spec.name.clone(),
                tab_order: spec.tab_order,
                block: spec.block.clone(),
                units: spec.units.clone(),
                plot: spec.plot.clone(),
                rect,
                rect_source: rect_source.to_string(),
                overview: overview_info,
                viewports: vp_reports,
                entities: paper_entities.len(),
            });
        }
    }

    // --- records with their images ---------------------------------------
    let images_for = |bbox: &Rect| -> Vec<&ImageInfo> {
        tile_images
            .iter()
            .filter(|img| img.world.intersects(bbox))
            .collect()
    };
    let px_map = |bbox: &Rect| -> Value {
        let mut m = Map::new();
        m.insert("ov".to_string(), json!(overview.px_box(bbox)));
        for ov in frame_overviews.iter().filter(|o| o.world.intersects(bbox)) {
            m.insert(ov.id.clone(), json!(ov.px_box(bbox)));
        }
        for img in images_for(bbox) {
            m.insert(img.id.clone(), json!(img.px_box(bbox)));
        }
        Value::Object(m)
    };
    let tiles_for =
        |bbox: &Rect| -> Vec<String> { images_for(bbox).iter().map(|i| i.id.clone()).collect() };

    let extent_of = |handle: &str| -> Option<Rect> {
        extents.iter().find(|e| e.handle == handle).map(|e| e.rect)
    };

    // texts
    let mut text_records: Vec<Record> = texts
        .iter()
        .map(|t| {
            let mut v = Map::new();
            v.insert("id".into(), json!(t.id));
            v.insert("kind".into(), json!(t.kind));
            v.insert("text".into(), json!(t.text));
            if t.raw != t.text {
                v.insert("raw".into(), json!(t.raw));
            }
            if let Some(tag) = &t.tag {
                v.insert("tag".into(), json!(tag));
            }
            v.insert("layer".into(), json!(t.layer));
            v.insert("height".into(), json!(rounder.derived(t.height)));
            v.insert(
                "rotation_deg".into(),
                json!(rounder.derived(t.rotation.to_degrees())),
            );
            v.insert("anchor".into(), rounder.pt2(t.anchor));
            if !t.style.is_empty() {
                v.insert("style".into(), json!(t.style));
            }
            v.insert("bbox".into(), rounder.rect(&t.bbox));
            v.insert("bbox_confidence".into(), json!(t.bbox_confidence));
            v.insert(
                "why".into(),
                json!(if t.bbox_confidence == "measured" {
                    "usvg glyph outlines, bundled font"
                } else {
                    "0.6 em per character from the anchor"
                }),
            );
            v.insert("font_ok".into(), json!(t.unshaped == 0));
            if t.unshaped > 0 {
                v.insert("unshaped_glyphs".into(), json!(t.unshaped));
            }
            v.insert("tiles".into(), json!(tiles_for(&t.bbox)));
            v.insert("px".into(), px_map(&t.bbox));
            Record {
                id: t.id.clone(),
                bbox: t.bbox,
                value: v,
            }
        })
        .collect();
    text_records.sort_by_key(|r| id_key(&r.id));

    // dimensions
    let mut dim_records: Vec<Record> = Vec::new();
    for e in visible {
        let Entity::Dimension(d) = e else { continue };
        let bbox = extent_of(&d.common.handle).unwrap_or_else(|| {
            Rect::new(
                d.definition_point.x,
                d.definition_point.y,
                d.definition_point.x,
                d.definition_point.y,
            )
        });
        let angular = d.geometry.is_angular();
        // `confidence` follows the source, not merely "there is a number":
        // a value the definition points gave is exact arithmetic on what
        // the file stores, but it is not what the drawing was measured at,
        // and a reader deciding whether to trust the number over the label
        // needs the two kept apart.
        let (measurement, source, confidence) = match (d.measurement, d.measurement_from_points) {
            (Some(m), _) => (Some(m), "act_measurement", "stored"),
            (None, Some(p)) => (Some(p), "from_points", "exact"),
            (None, None) => (None, "none", "unavailable"),
        };
        let delta = match (d.measurement, d.measurement_from_points) {
            (Some(m), Some(p)) => Some(rounder.derived(m - p)),
            _ => None,
        };
        let mut v = Map::new();
        v.insert("id".into(), json!(d.common.handle));
        v.insert(
            "kind".into(),
            serde_json::to_value(&d.geometry)?["kind"].clone(),
        );
        v.insert("layer".into(), json!(d.common.layer));
        v.insert("dimstyle".into(), json!(d.dimstyle));
        v.insert(
            "measurement".into(),
            json!(measurement.map(|m| rounder.derived(m))),
        );
        v.insert("measurement_source".into(), json!(source));
        v.insert(
            "measurement_from_points".into(),
            json!(d.measurement_from_points.map(|m| rounder.derived(m))),
        );
        v.insert("delta".into(), json!(delta));
        v.insert(
            "unit".into(),
            json!(if angular { "deg" } else { unit.as_str() }),
        );
        v.insert("dimlfac".into(), json!(d.dimlfac));
        v.insert("display".into(), json!(d.display_text));
        if d.display_text_raw != d.display_text {
            v.insert("display_raw".into(), json!(d.display_text_raw));
        }
        v.insert(
            "display_source".into(),
            serde_json::to_value(d.display_source)?,
        );
        if !d.user_text.is_empty() {
            v.insert("user_text".into(), json!(d.user_text));
        }
        v.insert("geometry".into(), serde_json::to_value(&d.geometry)?);
        v.insert("definition_point".into(), rounder.pt3(d.definition_point));
        v.insert("text_at".into(), rounder.pt2(d.text_midpoint));
        v.insert("confidence".into(), json!(confidence));
        v.insert("bbox".into(), rounder.rect(&bbox));
        v.insert("tiles".into(), json!(tiles_for(&bbox)));
        v.insert("px".into(), px_map(&bbox));
        dim_records.push(Record {
            id: d.common.handle.clone(),
            bbox,
            value: v,
        });
    }
    dim_records.sort_by_key(|r| id_key(&r.id));

    // geometry and regions
    let mut geo_records: Vec<Record> = Vec::new();
    let mut region_records: Vec<Record> = Vec::new();
    for e in visible {
        if matches!(
            e,
            Entity::Text(_)
                | Entity::MText(_)
                | Entity::Attrib(_)
                | Entity::Attdef(_)
                | Entity::Dimension(_)
                | Entity::Insert(_)
        ) {
            continue;
        }
        let handle = e.common().handle.clone();
        let Some(bbox) = extent_of(&handle) else {
            continue;
        };
        let mut v = Map::new();
        v.insert("id".into(), json!(handle));
        v.insert("type".into(), json!(e.type_name()));
        v.insert("layer".into(), json!(e.common().layer));
        let mut confidence = "exact";
        match e {
            Entity::Line(l) => {
                v.insert("from".into(), rounder.pt3(l.start_point));
                v.insert("to".into(), rounder.pt3(l.end_point));
                let len = (l.end_point.x - l.start_point.x).hypot(l.end_point.y - l.start_point.y);
                v.insert("length".into(), json!(rounder.derived(len)));
            }
            Entity::Arc(a) => {
                let mut sweep = a.end_angle - a.start_angle;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                v.insert("center".into(), rounder.pt3(a.center));
                v.insert("r".into(), json!(rounder.derived(a.radius)));
                v.insert(
                    "start_deg".into(),
                    json!(rounder.derived(a.start_angle.to_degrees())),
                );
                v.insert(
                    "end_deg".into(),
                    json!(rounder.derived(a.end_angle.to_degrees())),
                );
                v.insert(
                    "sweep_deg".into(),
                    json!(rounder.derived(sweep.to_degrees())),
                );
                v.insert("length".into(), json!(rounder.derived(a.radius * sweep)));
            }
            Entity::Circle(c) => {
                v.insert("center".into(), rounder.pt3(c.center));
                v.insert("r".into(), json!(rounder.derived(c.radius)));
                v.insert(
                    "length".into(),
                    json!(rounder.derived(std::f64::consts::TAU * c.radius)),
                );
                v.insert(
                    "area".into(),
                    json!(rounder.derived(std::f64::consts::PI * c.radius * c.radius)),
                );
            }
            Entity::Ellipse(el) => {
                let a = el.major_axis_endpoint.x.hypot(el.major_axis_endpoint.y);
                v.insert("center".into(), rounder.pt3(el.center));
                v.insert("major_axis".into(), rounder.pt3(el.major_axis_endpoint));
                v.insert("ratio".into(), json!(rounder.derived(el.axis_ratio)));
                let full = (el.end_angle - el.start_angle - std::f64::consts::TAU).abs() < 1e-9
                    || (el.start_angle == 0.0 && el.end_angle == 0.0);
                if full {
                    v.insert(
                        "area".into(),
                        json!(rounder.derived(std::f64::consts::PI * a * a * el.axis_ratio)),
                    );
                } else {
                    confidence = "unavailable";
                    v.insert("why".into(), json!("elliptical arc length is not computed"));
                }
            }
            Entity::LwPolyline(p) | Entity::Polyline2D(p) => {
                let bulged = !p.bulges.is_empty();
                v.insert("closed".into(), json!(p.closed));
                v.insert(
                    "vfmt".into(),
                    json!(if bulged { "[x,y,bulge]" } else { "[x,y]" }),
                );
                let vertices: Vec<Value> = p
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(i, pt)| {
                        if bulged {
                            json!([
                                rounder.coord(pt.x),
                                rounder.coord(pt.y),
                                p.bulges.get(i).copied().unwrap_or(0.0)
                            ])
                        } else {
                            rounder.pt2(*pt)
                        }
                    })
                    .collect();
                v.insert("vertices".into(), json!(vertices));
                let length = p.length();
                v.insert(
                    if p.closed { "perimeter" } else { "length" }.into(),
                    json!(rounder.derived(length)),
                );
                if let Some(area) = p.area() {
                    let signed = p.signed_area();
                    let simple = simple_outline(&p.vertices);
                    v.insert("area".into(), json!(rounder.derived(area)));
                    v.insert(
                        "orientation".into(),
                        json!(if signed >= 0.0 { "ccw" } else { "cw" }),
                    );
                    v.insert("simple".into(), json!(simple));
                    match simple {
                        Some(true) => {}
                        Some(false) => {
                            confidence = "unavailable";
                            v.insert(
                                "why".into(),
                                json!("self-intersecting outline: the area has no meaning"),
                            );
                        }
                        None => {
                            confidence = "estimated";
                            v.insert("why".into(), json!(UNTESTED_OUTLINE));
                        }
                    }
                    if p.closed && p.vertices.len() >= 3 {
                        let centroid = polygon_centroid(&p.vertices);
                        let mut r = Map::new();
                        r.insert("id".into(), json!(handle));
                        r.insert("src".into(), json!(e.type_name()));
                        r.insert("layer".into(), json!(e.common().layer));
                        r.insert("area".into(), json!(rounder.derived(area)));
                        r.insert("area_unit".into(), json!(area_unit(&unit)));
                        r.insert(
                            "area_si".into(),
                            json!(db
                                .header
                                .units
                                .to_mm
                                .map(|mm| rounder.derived(area * mm * mm / 1e6))),
                        );
                        r.insert("perimeter".into(), json!(rounder.derived(length)));
                        r.insert("centroid".into(), rounder.pt2(centroid));
                        r.insert("vertex_count".into(), json!(p.vertices.len()));
                        r.insert("simple".into(), json!(simple));
                        r.insert(
                            "confidence".into(),
                            json!(match simple {
                                Some(true) => "exact",
                                Some(false) => "unavailable",
                                None => "estimated",
                            }),
                        );
                        if simple.is_none() {
                            r.insert("why".into(), json!(UNTESTED_OUTLINE));
                        }
                        r.insert("bbox".into(), rounder.rect(&bbox));
                        r.insert("tiles".into(), json!(tiles_for(&bbox)));
                        r.insert("px".into(), px_map(&bbox));
                        region_records.push(Record {
                            id: handle.clone(),
                            bbox,
                            value: r,
                        });
                    }
                }
            }
            Entity::Polyline3D(p) => {
                v.insert("closed".into(), json!(p.closed));
                v.insert("vertex_count".into(), json!(p.vertices.len()));
                let len: f64 = p
                    .vertices
                    .windows(2)
                    .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
                    .sum();
                v.insert("length_plan".into(), json!(rounder.derived(len)));
            }
            Entity::Spline(s) => {
                v.insert("control_points".into(), json!(s.control_points.len()));
                v.insert("fit_points".into(), json!(s.fit_points.len()));
                confidence = "estimated";
                v.insert(
                    "why".into(),
                    json!("spline length is not evaluated in 0.3.0"),
                );
            }
            Entity::Point(p) => {
                v.insert("at".into(), rounder.pt3(p.position));
            }
            Entity::Solid(s) => {
                v.insert(
                    "corners".into(),
                    json!([
                        rounder.pt2(s.corner1),
                        rounder.pt2(s.corner2),
                        rounder.pt2(s.corner3),
                        rounder.pt2(s.corner4)
                    ]),
                );
            }
            Entity::Hatch(h) => {
                v.insert("paths".into(), json!(h.boundary_paths.len()));
                v.insert("solid_fill".into(), json!(h.solid_fill));
                confidence = "estimated";
            }
            Entity::Leader(l) => {
                v.insert("vertex_count".into(), json!(l.vertices.len()));
                v.insert("arrowhead".into(), json!(l.has_arrowhead));
            }
            Entity::Ray(r) | Entity::XLine(r) => {
                v.insert("point".into(), rounder.pt3(r.point));
                v.insert("vector".into(), rounder.pt3(r.vector));
            }
            _ => {
                confidence = "estimated";
            }
        }
        v.insert("unit".into(), json!(unit));
        v.insert("confidence".into(), json!(confidence));
        v.insert("bbox".into(), rounder.rect(&bbox));
        v.insert("tiles".into(), json!(tiles_for(&bbox)));
        v.insert("px".into(), px_map(&bbox));
        geo_records.push(Record {
            id: handle,
            bbox,
            value: v,
        });
    }
    geo_records.sort_by_key(|r| id_key(&r.id));
    region_records.sort_by_key(|r| id_key(&r.id));
    label_regions(db, visible, &texts, &mut region_records);

    // blocks
    let mut instances: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut block_records: Vec<Record> = Vec::new();
    for e in visible {
        let Entity::Insert(i) = e else { continue };
        let Some(bbox) = extent_of(&i.common.handle) else {
            continue;
        };
        instances
            .entry(i.block_name.clone())
            .or_default()
            .push(i.common.handle.clone());
        let attribs: Map<String, Value> = i
            .attribs
            .iter()
            .filter(|a| !a.tag.is_empty())
            .map(|a| (a.tag.clone(), json!(a.text_plain)))
            .collect();
        let mut v = Map::new();
        v.insert("id".into(), json!(i.common.handle));
        v.insert("block".into(), json!(i.block_name));
        v.insert("layer".into(), json!(i.common.layer));
        v.insert("at".into(), rounder.pt3(i.insertion_point));
        v.insert(
            "rotation_deg".into(),
            json!(rounder.derived(i.rotation.to_degrees())),
        );
        v.insert("scale".into(), json!([i.scale.x, i.scale.y, i.scale.z]));
        v.insert(
            "mirrored".into(),
            json!(i.extrusion.z < 0.0 || i.scale.x * i.scale.y < 0.0),
        );
        v.insert("attribs".into(), Value::Object(attribs));
        v.insert("bbox".into(), rounder.rect(&bbox));
        v.insert("tiles".into(), json!(tiles_for(&bbox)));
        v.insert("px".into(), px_map(&bbox));
        block_records.push(Record {
            id: i.common.handle.clone(),
            bbox,
            value: v,
        });
    }
    block_records.sort_by_key(|r| id_key(&r.id));
    let definitions: Vec<Value> = db
        .tables
        .block_records
        .values()
        .filter(|b| {
            let upper = b.name.to_uppercase();
            !upper.starts_with("*MODEL_SPACE") && !upper.starts_with("*PAPER_SPACE")
        })
        .map(|b| {
            let mut by_layer: BTreeMap<String, usize> = BTreeMap::new();
            let mut tags: BTreeSet<String> = BTreeSet::new();
            for e in &b.entities {
                *by_layer.entry(e.common().layer.clone()).or_default() += 1;
                if let Entity::Attdef(a) = e {
                    tags.insert(a.tag.clone());
                }
            }
            let ids = instances.get(&b.name).cloned().unwrap_or_default();
            json!({
                "name": b.name,
                "entity_count": b.entities.len(),
                "count_by_layer": by_layer,
                "attrib_tags": tags,
                "anonymous": b.name.starts_with('*'),
                "instances": ids.len(),
                "instance_ids": ids,
            })
        })
        .collect();

    // --- strings ----------------------------------------------------------
    let mut strings: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut index = |text: &str, id: &str| {
        let key = normalize_string(text);
        if key.is_empty() {
            return;
        }
        let compact = compact_string(text);
        if compact != key {
            strings.entry(compact).or_default().insert(id.to_string());
        }
        strings.entry(key).or_default().insert(id.to_string());
    };
    for t in &texts {
        index(&t.text, &t.id);
    }
    for r in &dim_records {
        if let Some(Value::String(d)) = r.value.get("display") {
            index(d, &r.id);
        }
    }
    for r in &block_records {
        if let Some(Value::Object(attribs)) = r.value.get("attribs") {
            for value in attribs.values() {
                if let Value::String(s) = value {
                    index(s, &r.id);
                }
            }
        }
    }

    // --- write the JSON files ----------------------------------------------
    writer.write_records("texts", "text", &text_records)?;
    writer.write_records("dimensions", "dimension", &dim_records)?;
    writer.write_records("geometry", "geometry", &geo_records)?;
    writer.write_records("regions", "region", &region_records)?;
    let blocks_value = json!({
        "$schema": SCHEMA,
        "units": writer.units,
        "definitions": definitions,
        "instances": block_records.iter().map(|r| Value::Object(r.value.clone())).collect::<Vec<_>>(),
    });
    writer.write_json("blocks.json", &blocks_value, "blocks")?;
    let strings_value = json!({
        "$schema": SCHEMA,
        "normalization": "NFKC, fraction slash to '/', case fold, whitespace collapsed; each string also under its key with spaces removed",
        "strings": strings,
    });
    writer.write_json("strings.json", &strings_value, "strings")?;
    if options.sheets {
        let sheets_value = json!({
            "$schema": SCHEMA,
            "twist_convention": "model_to_paper = C + s (R(twist) (p - T) - V), a positive twist turning the picture counter-clockwise (ezdxf's rule; not checked against a plotted sheet)",
            "sheets": sheet_reports,
        });
        writer.write_json("sheets.json", &sheets_value, "sheets")?;
    }

    // sidecars and tiles.json
    let mut tiles_json: Vec<Value> = Vec::new();
    for build in &frame_builds {
        for tile in &build.tiles {
            let image = tile_images.iter().find(|i| i.id == tile.id);
            let mut entry = json!({
                "id": tile.id,
                "frame": tile.frame,
                "z": tile.z,
                "row": tile.row,
                "col": tile.col,
                "px": [tile.width, tile.height],
                "world": rounder.rect(&tile.world),
                "empty": tile.empty,
            });
            if let Some(img) = image {
                entry["png"] = json!(img.png);
                let sidecar = sidecar(
                    img,
                    tile,
                    &build.tiles,
                    &profile,
                    &text_records,
                    &dim_records,
                    &block_records,
                    &region_records,
                    &geo_records,
                    &rounder,
                );
                let sidecar_path = img.png.replace(".png", ".json");
                writer.write_json_compact(&sidecar_path, &sidecar, "sidecar")?;
                entry["sidecar"] = json!(sidecar_path);
            }
            tiles_json.push(entry);
        }
    }
    let frame_reports: Vec<FrameReport> = frame_builds.iter().map(|b| b.report.clone()).collect();
    writer.write_json(
        "tiles.json",
        &json!({
            "$schema": SCHEMA,
            "frames": frame_reports.iter().map(|f| json!({ "id": f.id, "kind": f.kind, "content": rounder.rect(&f.content), "levels": f.levels })).collect::<Vec<_>>(),
            "tiles": tiles_json,
        }),
        "tiles",
    )?;
    // drawing.json
    let mut layer_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut type_counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &top {
        *layer_counts.entry(e.common().layer.clone()).or_default() += 1;
        *type_counts.entry(e.type_name().to_string()).or_default() += 1;
    }
    let layers: Vec<Value> = db
        .tables
        .layers
        .values()
        .map(|l| {
            json!({
                "name": l.name,
                "color_index": l.color_index,
                "on": l.on,
                "frozen": l.frozen,
                "locked": l.locked,
                "plot": l.plot,
                "lineweight_mm": l.lineweight_mm,
                "linetype": l.linetype,
                "entity_count": layer_counts.get(&l.name).copied().unwrap_or(0),
            })
        })
        .collect();
    let drawing_value = json!({
        "$schema": SCHEMA,
        "units": writer.units,
        "header": serde_json::to_value(&db.header)?,
        "layers": layers,
        "blocks": db.tables.block_records.values().map(|b| json!({"name": b.name, "entity_count": b.entities.len()})).collect::<Vec<_>>(),
        "counts": { "model_space": top.len(), "by_type": type_counts },
    });
    writer.write_json("drawing.json", &drawing_value, "drawing")?;

    // optional whole-model files
    let mut svg_origin: Option<[f64; 2]> = None;
    if options.svg {
        let doc = db.to_svg(svg_options);
        writer.write_bytes("drawing.svg", doc.svg.as_bytes(), "svg")?;
        svg_origin = Some(doc.origin);
    }
    if options.full {
        let text = db.to_json(ToJsonOptions { pretty: false })?;
        writer.write_bytes("entities.json", text.as_bytes(), "entities")?;
    }

    // report.json
    let mut hidden_by_reason: BTreeMap<String, usize> = BTreeMap::new();
    let mut hidden_handles: Vec<String> = Vec::new();
    for e in &top {
        if let Some(reason) = hidden_reason(e.common(), &db.tables) {
            *hidden_by_reason
                .entry(reason.as_str().to_string())
                .or_default() += 1;
            if hidden_handles.len() < 100 {
                hidden_handles.push(e.common().handle.clone());
            }
        }
    }
    let crop_report = rendered.choice.report(fit.rect, padding);
    let counts = Counts {
        entities: top.len(),
        texts: text_records.len(),
        dimensions: dim_records.len(),
        geometry: geo_records.len(),
        regions: region_records.len(),
        blocks: block_records.len(),
        hidden: rendered.hidden,
        excluded: crop_report.excluded.len(),
        tiles: written_total,
        frames: frame_reports.len(),
        sheets: sheet_reports.len(),
    };
    let hidden_top_level: usize = hidden_by_reason.values().sum();
    // Written after the manifest (its timings would change the byte count
    // the manifest lists), so it is listed without a size.
    let report_value = json!({
        "$schema": SCHEMA,
        "excluded": crop_report.excluded,
        "hidden": {
            "count": hidden_top_level,
            "inside_blocks": rendered.hidden.saturating_sub(hidden_top_level),
            "by_reason": hidden_by_reason,
            "handles": hidden_handles,
        },
        "unsupported_types": rendered.unsupported_types(),
        "warnings": warnings,
        "timings_ms": { "total": started.elapsed().as_millis() as u64 },
    });
    writer.files.push(WrittenFile {
        path: "report.json".into(),
        bytes: None,
        kind: "report".into(),
    });

    // manifest.json, README.txt and report.json, last (they list the files)
    // "exact" is reserved for values the file itself measured: a package of
    // an R13/R14 drawing (no act_measurement anywhere) carries values this
    // crate recomputed from the definition points, and used to advertise
    // them as the drawing's own.
    let dim_source = |want: &str| {
        dim_records
            .iter()
            .any(|r| r.value.get("measurement_source").is_some_and(|s| s == want))
    };
    let capabilities = json!({
        "dimension_values": if dim_records.is_empty() { "none" } else if dim_source("act_measurement") { "exact" } else if dim_source("from_points") { "computed" } else { "text_only" },
        "areas": "exact",
        "text_boxes": if measured > 0 { "measured" } else if texts.is_empty() { "none" } else { "estimated" },
        "fonts": match options.fonts { Fonts::Bundled => "bundled", Fonts::BundledAndSystem => "bundled+system" },
        "paper_layouts": if sheet_reports.is_empty() { "none" } else { "composited" },
        "frames": frame_reports.len(),
    });
    // The tile and overlap numbers come from the profile in use, not from
    // the prose: --profile claude-hires writes 1932 px tiles with 392 px of
    // overlap, and the sentence used to say 224 whatever the levels said.
    let guidance = format!(
        "Read manifest.json first. Numbers (lengths, areas, dimension values, text) come from the JSON records, never from pixels; each record's `confidence` says how the value was obtained. To find something: look its text up in strings.json (normalised: trimmed, lower-case, single spaces), open the record in the file shard_index names for its kind, then open the tile(s) in its `tiles` list; every tile's .json sidecar lists what is on it with pixel boxes. overview.png shows the whole crop; each frame in `frames` (f0 the main drawing, f1.. details drawn beside it) has its own overview and tiles z1..zN, {} px with {} px overlap (2x zooms), row 0 at the top; report.json lists what was left out and why.",
        profile.tile, profile.overlap
    );
    writer.files.push(WrittenFile {
        path: "manifest.json".into(),
        bytes: None,
        kind: "manifest".into(),
    });
    writer.files.push(WrittenFile {
        path: "README.txt".into(),
        bytes: None,
        kind: "readme".into(),
    });
    let manifest = json!({
        "$schema": SCHEMA,
        "generator": { "name": "uncad", "version": env!("CARGO_PKG_VERSION") },
        "profile": profile.name,
        "source": {
            "name": options.source_name,
            "version": db.header.version,
            "codepage": db.header.codepage_name,
        },
        "units": writer.units,
        "crop": crop_report,
        // drawing.svg's user units are world minus this (null without it).
        "svg_origin": svg_origin,
        "overview": overview,
        "frames": frame_reports,
        "frames_dropped": dropped,
        "sheets": sheet_reports.iter().map(|s| json!({"name": s.name, "tab_order": s.tab_order, "png": s.overview.png, "px": s.overview.px, "rect": rounder.rect(&s.rect), "units": s.units, "viewports": s.viewports.len()})).collect::<Vec<_>>(),
        "legibility": { "target_px": options.target_text_px, "per_frame": frame_reports.iter().map(|f| json!({"frame": f.id, "z_max": f.z_max, "reached": f.reached, "height_classes": f.height_classes})).collect::<Vec<_>>() },
        "counts": counts,
        "capabilities": capabilities,
        "guidance": guidance,
        "files": writer.files,
        "shard_index": writer.shard_index,
        "warnings": warnings,
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(dir.join("manifest.json"), &manifest_text).map_err(|source| {
        ExportError::Io {
            path: dir.join("manifest.json"),
            source,
        }
    })?;
    let readme = format!(
        "uncad package ({SCHEMA})\n\nReading order:\n  1. manifest.json   what is here, the crop, the images and their affines\n  2. strings.json    find a text or a number, get record ids\n  3. texts.json / dimensions.json / geometry.json / regions.json / blocks.json   the records (sharded above {} KB, see shard_index)\n  4. overview.png    the whole drawing; frames/f*/overview.png and frames/f*/tiles/z*/  zoomed tiles with .json sidecars\n  5. sheets.json     paper layouts: sheet size, viewports with their scale and model window; sheets/<layout>/overview.png\n  6. report.json     what was left out and why\n\ndrawing.json holds the header, units and layer states; entities.json and drawing.svg (when present) are tool inputs, not for reading.\n",
        options.shard_kb
    );
    std::fs::write(dir.join("README.txt"), readme.as_bytes()).map_err(|source| {
        ExportError::Io {
            path: dir.join("README.txt"),
            source,
        }
    })?;
    let report_text = serde_json::to_string_pretty(&report_value)?;
    std::fs::write(dir.join("report.json"), &report_text).map_err(|source| ExportError::Io {
        path: dir.join("report.json"),
        source,
    })?;

    Ok(ExportReport {
        dir: dir.to_path_buf(),
        files: writer.files,
        overview,
        frames: frame_reports,
        sheets: sheet_reports,
        crop: crop_report,
        counts,
        warnings,
    })
}

/// Removes what a previous uncad package in `dir` left behind, so a second
/// export with other options does not leave stale shards, tile PNGs and
/// sidecars beside the new ones: they look valid (same schema, same record
/// ids) and a consumer that walks the tree -- as the generated README.txt
/// invites -- would mix two exports, following `parent`/`children` links
/// into a pyramid the new manifest does not have.
///
/// Only the files the previous `manifest.json` lists are removed, and only
/// when it is an uncad manifest: a directory holding anything else is left
/// alone, and a listed path that is not a plain relative path inside `dir`
/// is ignored. Directories under `frames/` and `sheets/` go when they are
/// left empty. Every failure is ignored -- the write that follows reports
/// what actually matters.
fn clear_previous_package(dir: &Path) {
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
        return;
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let ours = manifest
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|s| s.starts_with("uncad-package/"));
    if !ours {
        return;
    }
    let files = manifest.get("files").and_then(Value::as_array);
    for file in files.into_iter().flatten() {
        let Some(rel) = file.get("path").and_then(Value::as_str) else {
            continue;
        };
        let inside = !rel.is_empty()
            && Path::new(rel)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if inside {
            let _ = std::fs::remove_file(dir.join(rel));
        }
    }
    for sub in ["frames", "sheets"] {
        remove_empty_dirs(&dir.join(sub));
    }
}

/// Removes `dir` and every directory under it that is empty once its own
/// empty children are gone; a directory still holding a file stays (with
/// everything above it).
fn remove_empty_dirs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&entry.path());
        }
    }
    // Fails, harmlessly, when anything is left in it.
    let _ = std::fs::remove_dir(dir);
}

/// Grows every extent by the measured boxes of the texts its part draws.
/// A text's id is `<handle>` at the top level and `<insert>/<child>`
/// inside a block reference, and the renderer emits one part per top-level
/// entity, so the first segment of the id names the extent to grow.
/// Estimated boxes are left alone: they are what the extent already holds.
fn widen_extents_with_texts(extents: &mut [Extent], texts: &[PlacedText]) {
    let mut measured: std::collections::HashMap<&str, Rect> = std::collections::HashMap::new();
    for t in texts {
        if t.bbox_confidence != "measured" {
            continue;
        }
        let handle = t.id.split('/').next().unwrap_or(t.id.as_str());
        measured
            .entry(handle)
            .and_modify(|r| *r = r.union(&t.bbox))
            .or_insert(t.bbox);
    }
    for e in extents.iter_mut() {
        if let Some(box_of_texts) = measured.get(e.handle.as_str()) {
            e.rect = e.rect.union(box_of_texts);
        }
    }
}

/// The metrics pre-pass: lays the text-bearing entities out once through
/// usvg (the same shaping the images get) and gives every placed text the
/// tight box of its glyph outlines, in world units, plus the count of
/// glyphs the font could not shape. Texts usvg drops (whitespace-only, or
/// no font at all) keep their estimate. Returns how many were measured.
fn measure_texts(
    rendered: &svg::Rendered,
    content: &Rect,
    texts: &mut [PlacedText],
    fonts: Fonts,
) -> Result<usize, ExportError> {
    if texts.is_empty() {
        return Ok(0);
    }
    // Parts are per top-level entity; a text's first id segment is the
    // handle of the entity (or INSERT) whose part draws it.
    let bearing: BTreeSet<&str> = texts
        .iter()
        .map(|t| t.id.split('/').next().unwrap_or(t.id.as_str()))
        .collect();
    let view_box = ViewBox::from_world(content);
    let svg_text =
        svg::assemble_subset(rendered, &view_box, 1.0, |handle| bearing.contains(handle));
    let tree = png::parse_tree(&svg_text, fonts)?;
    let mut boxes: std::collections::HashMap<String, (Rect, usize)> =
        std::collections::HashMap::new();
    // The document's coordinates are relative to `rendered.origin`, and so
    // is its viewBox, so a canvas box (relative to the viewBox corner)
    // plus the world viewBox corner is a world box: the origin cancels.
    collect_text_boxes(tree.root(), &view_box, &mut boxes);
    let mut measured = 0;
    for t in texts.iter_mut() {
        if let Some((rect, unshaped)) = boxes.get(&t.id) {
            t.bbox = *rect;
            t.bbox_confidence = "measured";
            t.unshaped = *unshaped;
            measured += 1;
        }
    }
    Ok(measured)
}

/// Walks a parsed tree for `<text id>` nodes: usvg keeps every text as a
/// `Node::Text` carrying its id (inside the unnamed groups the viewBox and
/// any `transform` add), and `abs_stroke_bounding_box` is the box of the
/// flattened glyph outlines in canvas units -- user units minus the
/// viewBox origin, y down.
fn collect_text_boxes(
    group: &resvg::usvg::Group,
    view_box: &ViewBox,
    out: &mut std::collections::HashMap<String, (Rect, usize)>,
) {
    use resvg::usvg::Node;
    for node in group.children() {
        match node {
            Node::Group(g) => collect_text_boxes(g, view_box, out),
            Node::Text(t) if !t.id().is_empty() => {
                let b = t.abs_stroke_bounding_box();
                let rect = Rect::new(
                    f64::from(b.left()) + view_box.x,
                    -(f64::from(b.bottom()) + view_box.y),
                    f64::from(b.right()) + view_box.x,
                    -(f64::from(b.top()) + view_box.y),
                );
                if ![rect.min_x, rect.min_y, rect.max_x, rect.max_y]
                    .iter()
                    .all(|v| v.is_finite())
                {
                    continue;
                }
                let unshaped = t
                    .layouted()
                    .iter()
                    .flat_map(|span| span.positioned_glyphs.iter())
                    .filter(|g| g.id.0 == 0)
                    .count();
                out.insert(t.id().to_string(), (rect, unshaped));
            }
            _ => {}
        }
    }
}

/// A paper layout to export.
struct SheetSpec {
    name: String,
    tab_order: u16,
    block: String,
    units: String,
    plot: Option<crate::tables::PlotSettings>,
    limmin: Point2D,
    limmax: Point2D,
}

/// The paper layouts of `db`, in tab order: the LAYOUT objects whose block
/// exists, or -- for a file without them (R13/R14, a DXF without an
/// OBJECTS section) -- every paper-space block with entities.
fn sheet_specs(db: &CadDatabase) -> Vec<SheetSpec> {
    let mut specs: Vec<SheetSpec> = db
        .tables
        .layouts
        .values()
        .filter(|l| l.tab_order > 0 && db.tables.block_records.contains_key(&l.block_name))
        .map(|l| SheetSpec {
            name: l.name.clone(),
            tab_order: l.tab_order,
            block: l.block_name.clone(),
            units: match l.plot.paper_units {
                0 => "in",
                2 => "px",
                _ => "mm",
            }
            .to_string(),
            plot: Some(l.plot.clone()),
            limmin: l.limmin,
            limmax: l.limmax,
        })
        .collect();
    if specs.is_empty() {
        let mut tab = 1;
        for (name, block) in &db.tables.block_records {
            if name.to_uppercase().starts_with("*PAPER_SPACE") && !block.entities.is_empty() {
                specs.push(SheetSpec {
                    name: name.trim_start_matches('*').to_string(),
                    tab_order: tab,
                    block: name.clone(),
                    units: db.header.units.name.clone(),
                    plot: None,
                    limmin: Point2D { x: 0.0, y: 0.0 },
                    limmax: Point2D { x: 0.0, y: 0.0 },
                });
                tab += 1;
            }
        }
    }
    specs.sort_by(|a, b| a.tab_order.cmp(&b.tab_order).then(a.name.cmp(&b.name)));
    specs
}

/// An overview fitted to the profile: the pixel size within both the edge
/// and the patch budget, the world rectangle (padded and lattice-snapped)
/// and the scale.
struct OverviewFit {
    rect: Rect,
    width: u32,
    height: u32,
    ppu: f64,
    padding: f64,
}

/// The directory one sheet's image goes in, under `sheets/`: the layout's
/// name with every character outside `[A-Za-z0-9_-]` replaced by `_` (so
/// the path is portable and an ASCII name stays readable), `sheet` when
/// nothing is left of it, and `_2`, `_3`, ... in tab order when an earlier
/// layout already took the name. `used` collects what has been handed out.
///
/// The suffix is what keeps two sheets apart: every Hangul syllable
/// sanitises to `_`, so two three-syllable Korean names both became `___`
/// and the second layout's PNG silently overwrote the first's while both
/// `sheets.json` entries pointed at the one surviving file.
fn sheet_dir(name: &str, used: &mut BTreeSet<String>) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if safe.is_empty() {
        "sheet".to_string()
    } else {
        safe
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

/// The most pixels one drawing unit may become (see [`fit_overview`]).
const MAX_PPU: f64 = 1e9;

/// The smallest world window an image of degenerate content gets, in
/// drawing units. Content with no size at all -- a lone POINT, coincident
/// entities, the base points of RAY/XLINE, which is all those entities
/// contribute -- gives the fit nothing to scale to. Half a unit of
/// padding around it would put the scale in the thousands of pixels per
/// unit, where the 1e6-unit line the renderer draws a RAY with reaches
/// billions of device pixels and tiny-skia's scan converter gives up; ten
/// units keeps it in the hundreds, and the point itself (drawn half a
/// unit across) is still ~50 px wide on a 1092 px overview.
const MIN_CONTENT_EXTENT: f64 = 10.0;

fn fit_overview(content: &Rect, profile: &Profile, padding: Option<f64>) -> OverviewFit {
    // Degenerate content first: a window around its centre, so the rest of
    // the fit works on a rectangle with a size.
    let grown;
    let content = if content.longer_side().is_finite() && content.longer_side() > 0.0 {
        content
    } else {
        let half = MIN_CONTENT_EXTENT / 2.0;
        let (cx, cy) = if content.min_x.is_finite() && content.min_y.is_finite() {
            (content.min_x, content.min_y)
        } else {
            (0.0, 0.0)
        };
        grown = Rect::new(cx - half, cy - half, cx + half, cy + half);
        &grown
    };
    let lattice = f64::from(profile.lattice.max(1));
    let edge_patches = (f64::from(profile.overview_edge) / lattice)
        .floor()
        .max(1.0);
    let (w, h) = (content.width().max(1e-9), content.height().max(1e-9));
    let aspect = w / h;
    let budget = f64::from(profile.overview_patches);
    let pw = edge_patches.min((budget * aspect).sqrt().floor()).max(1.0);
    let ph = edge_patches
        .min((budget / pw).floor())
        .min((pw / aspect).ceil())
        .max(1.0);
    let seed_ppu = (pw * lattice / (1.04 * w)).min(ph * lattice / (1.04 * h));
    let padding = padding.unwrap_or_else(|| crop::auto_padding(content, Some(seed_ppu)));
    let padded = content.padded(padding);
    let ppu = (pw * lattice / padded.width()).min(ph * lattice / padded.height());
    // Degenerate content (a single POINT, only RAY/XLINE base points, a
    // caller that forced zero padding) leaves a zero-size window, and
    // `1568 / 0` is infinite: `snap_to_lattice` would turn that into a
    // u32::MAX canvas and the render would fail. The cap also keeps the
    // scale of a drawing a millionth of a unit across representable; it
    // never binds on a real one, since it takes a window under 1.6e-6
    // units to reach it.
    let ppu = if ppu.is_finite() && ppu > 0.0 {
        ppu.min(MAX_PPU)
    } else {
        1.0
    };
    let (rect, width, height) = crop::snap_to_lattice(&padded, ppu, profile.lattice);
    OverviewFit {
        rect,
        width,
        height,
        ppu,
        padding,
    }
}

struct FrameBuild {
    report: FrameReport,
    tiles: Vec<Tile>,
}

/// Plans one frame: its overview (or the whole-crop one when `reuse` is
/// given), its depth from the texts inside it, and its tiles within the
/// remaining `tile_budget`.
#[allow(clippy::too_many_arguments)]
fn build_frame(
    id: &str,
    kind: &str,
    content: Rect,
    entities: usize,
    texts: &[PlacedText],
    extents: &[Extent],
    profile: &Profile,
    options: &ExportOptions,
    reuse: Option<ImageInfo>,
    tile_budget: &mut usize,
    warnings: &mut Vec<String>,
) -> FrameBuild {
    let fit = fit_overview(&content, profile, options.padding);
    let overview = reuse.unwrap_or_else(|| {
        ImageInfo::new(
            &format!("{id}/ov"),
            &format!("frames/{id}/overview.png"),
            fit.rect,
            fit.ppu,
            fit.width,
            fit.height,
        )
    });
    let (rect0, w0, h0, ppu_0) = (overview.world, overview.px[0], overview.px[1], overview.ppu);
    let inside: Vec<&PlacedText> = texts
        .iter()
        .filter(|t| content.intersects(&t.bbox))
        .collect();
    let heights = height_classes(&inside);
    let z_max = depth_for(&heights, ppu_0, options);
    let mut levels: Vec<LevelInfo> = Vec::new();
    let mut tiles: Vec<Tile> = Vec::new();
    let mut reached = true;
    for z in 1..=z_max {
        let ppu = ppu_0 * 2f64.powi(z as i32);
        let (cw, ch) = (w0 * 2u32.pow(z), h0 * 2u32.pow(z));
        let plan = plan_tiles(id, z, cw, ch, &rect0, ppu, profile, extents);
        let written = plan.iter().filter(|t| !t.empty).count();
        if written > *tile_budget {
            reached = false;
            warnings.push(format!(
                "MaxTiles: frame {id} level z{z} would need {written} tiles with {} left of {}; stopping at z{}",
                *tile_budget,
                options.max_tiles,
                z - 1
            ));
            break;
        }
        *tile_budget -= written;
        let (cols, rows) = grid(cw, ch, profile);
        levels.push(LevelInfo {
            z,
            ppu,
            canvas_px: [cw, ch],
            cols,
            rows,
            tile_px: profile.tile,
            overlap_px: profile.overlap,
            step_px: profile.step(),
            tiles_written: written,
            tiles_empty: plan.len() - written,
        });
        tiles.extend(plan);
    }
    let z_reached = levels.last().map_or(0, |l| l.z);
    let height_classes = heights
        .iter()
        .map(|(h, n)| {
            let px = h * ppu_0 * 2f64.powi(z_reached as i32);
            HeightClass {
                height: round_to(*h, 6),
                count: *n,
                px_at_zmax: round_to(px, 3),
                legible: px >= options.target_text_px,
            }
        })
        .collect();
    FrameBuild {
        report: FrameReport {
            id: id.to_string(),
            kind: kind.to_string(),
            content,
            entities,
            texts: inside.len(),
            overview,
            levels,
            z_max: z_reached,
            reached,
            height_classes,
        },
        tiles,
    }
}

/// Renders `tiles` on as many threads as the machine offers (at most one
/// per tile), returning the PNG bytes in the tiles' order. Each tile gets
/// its own SVG holding only the entities whose extent touches the tile
/// grown by `margin` (entities without an extent, and the infinite lines
/// whose extent is only a base point, are always included).
///
/// A tile thread that panics ends the export with
/// [`PngError::RenderPanic`], not with the process: a rasterizer that
/// asserts on one tile must not take the whole run down without a word.
#[allow(clippy::too_many_arguments)]
fn render_tiles_parallel(
    rendered: &svg::Rendered,
    extent_of_handle: &std::collections::HashMap<&str, Rect>,
    ppu: f64,
    stroke_px: f64,
    margin: f64,
    fonts: Fonts,
    tiles: &[&Tile],
) -> Result<Vec<Vec<u8>>, ExportError> {
    if tiles.is_empty() {
        return Ok(Vec::new());
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 16)
        .min(tiles.len());
    let chunk = tiles.len().div_ceil(threads);
    let render_one = |tile: &Tile| -> Result<Vec<u8>, PngError> {
        let window = tile.world.padded(margin);
        let svg_text = svg::assemble_subset(
            rendered,
            &ViewBox::from_world(&tile.world),
            stroke_px / ppu,
            |handle| {
                rendered.unbounded.contains(handle)
                    || extent_of_handle
                        .get(handle)
                        .is_none_or(|rect| rect.intersects(&window))
            },
        );
        let tree = png::parse_tree(&svg_text, fonts)?;
        png::render_region(&tree, ppu, (0.0, 0.0), tile.width, tile.height)
    };
    let results: Vec<Result<Vec<Vec<u8>>, PngError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = tiles
            .chunks(chunk)
            .map(|group| {
                let render_one = &render_one;
                scope.spawn(move || {
                    group
                        .iter()
                        .map(|tile| render_one(tile))
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    Err(PngError::RenderPanic(
                        "a tile rendering thread panicked".to_string(),
                    ))
                })
            })
            .collect()
    });
    let mut out = Vec::with_capacity(tiles.len());
    for group in results {
        out.extend(group?);
    }
    Ok(out)
}

fn area_unit(unit: &str) -> String {
    if unit == "du" {
        "du2".to_string()
    } else {
        format!("{unit}2")
    }
}

/// Count-weighted text height classes (heights rounded to 3 decimals),
/// most common first.
fn height_classes(texts: &[&PlacedText]) -> Vec<(f64, usize)> {
    let mut classes: BTreeMap<i64, usize> = BTreeMap::new();
    for t in texts {
        if t.height > 0.0 && t.height.is_finite() {
            *classes
                .entry((t.height * 1000.0).round() as i64)
                .or_default() += 1;
        }
    }
    let mut out: Vec<(f64, usize)> = classes
        .into_iter()
        .map(|(k, n)| (k as f64 / 1000.0, n))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.total_cmp(&b.0)));
    out
}

/// The deepest level: where the dominant text class (the count-weighted
/// median height) reaches the target pixel height; one level without text.
fn depth_for(heights: &[(f64, usize)], ppu_0: f64, options: &ExportOptions) -> u32 {
    if options.max_levels == 0 {
        return 0;
    }
    let total: usize = heights.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return 1;
    }
    let mut sorted: Vec<(f64, usize)> = heights.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut seen = 0;
    let mut median = sorted[0].0;
    for (h, n) in &sorted {
        seen += n;
        if seen * 2 >= total {
            median = *h;
            break;
        }
    }
    let px_now = median * ppu_0;
    if px_now <= 0.0 {
        return 1;
    }
    let z = (options.target_text_px / px_now).log2().ceil();
    if z.is_finite() {
        (z.max(1.0) as u32).min(options.max_levels)
    } else {
        1
    }
}

fn grid(canvas_w: u32, canvas_h: u32, profile: &Profile) -> (u32, u32) {
    let count = |extent: u32| -> u32 {
        if extent <= profile.tile {
            1
        } else {
            (extent - profile.tile).div_ceil(profile.step()) + 1
        }
    };
    (count(canvas_w), count(canvas_h))
}

/// The tiles of one level: SAHI-style, the last row and column shifted
/// inward so every tile is the full size (or the whole canvas when that
/// is smaller); a tile is empty when no visible extent touches it.
#[allow(clippy::too_many_arguments)]
fn plan_tiles(
    frame: &str,
    z: u32,
    canvas_w: u32,
    canvas_h: u32,
    rect0: &Rect,
    ppu: f64,
    profile: &Profile,
    extents: &[Extent],
) -> Vec<Tile> {
    let (cols, rows) = grid(canvas_w, canvas_h, profile);
    let origin = |index: u32, extent: u32| -> (u32, u32) {
        if extent <= profile.tile {
            (0, extent)
        } else {
            let o = (index * profile.step()).min(extent - profile.tile);
            (o, profile.tile)
        }
    };
    let mut tiles = Vec::new();
    for row in 0..rows {
        let (oy, th) = origin(row, canvas_h);
        for col in 0..cols {
            let (ox, tw) = origin(col, canvas_w);
            let x0 = rect0.min_x + f64::from(ox) / ppu;
            let y1 = rect0.max_y - f64::from(oy) / ppu;
            let world = Rect::new(x0, y1 - f64::from(th) / ppu, x0 + f64::from(tw) / ppu, y1);
            let empty = !extents.iter().any(|e| e.rect.intersects(&world));
            tiles.push(Tile {
                id: format!("{frame}/z{z}/r{row:02}_c{col:02}"),
                frame: frame.to_string(),
                z,
                row,
                col,
                origin_px: (ox, oy),
                width: tw,
                height: th,
                world,
                empty,
            });
        }
    }
    tiles
}

const SIDECAR_LIMIT: usize = 32 * 1024;

#[allow(clippy::too_many_arguments)]
fn sidecar(
    img: &ImageInfo,
    tile: &Tile,
    tiles: &[Tile],
    profile: &Profile,
    texts: &[Record],
    dims: &[Record],
    blocks: &[Record],
    regions: &[Record],
    geometry: &[Record],
    rounder: &Rounder,
) -> Value {
    let find = |z: u32, row: i64, col: i64| -> Option<String> {
        if row < 0 || col < 0 {
            return None;
        }
        tiles
            .iter()
            .find(|t| t.z == z && i64::from(t.row) == row && i64::from(t.col) == col)
            .map(|t| t.id.clone())
    };
    let (r, c) = (i64::from(tile.row), i64::from(tile.col));
    let neighbors = json!({
        "n": find(tile.z, r - 1, c),
        "s": find(tile.z, r + 1, c),
        "w": find(tile.z, r, c - 1),
        "e": find(tile.z, r, c + 1),
    });
    let centre = Point2D {
        x: (tile.world.min_x + tile.world.max_x) / 2.0,
        y: (tile.world.min_y + tile.world.max_y) / 2.0,
    };
    let parent = tiles
        .iter()
        .find(|t| {
            t.z + 1 == tile.z
                && t.world.min_x <= centre.x
                && centre.x <= t.world.max_x
                && t.world.min_y <= centre.y
                && centre.y <= t.world.max_y
        })
        .map(|t| t.id.clone());
    let children: Vec<String> = tiles
        .iter()
        .filter(|t| t.z == tile.z + 1 && t.world.intersects(&tile.world) && !t.empty)
        .map(|t| t.id.clone())
        .collect();
    fn on_tile<'a>(records: &'a [Record], world: &Rect) -> Vec<&'a Record> {
        records
            .iter()
            .filter(|rec| rec.bbox.intersects(world))
            .collect()
    }
    let truncate = |s: &str| -> String {
        if s.chars().count() > 24 {
            let cut: String = s.chars().take(24).collect();
            format!("{cut}...")
        } else {
            s.to_string()
        }
    };
    let mut text_rows: Vec<Value> = on_tile(texts, &tile.world)
        .iter()
        .map(|rec| {
            json!([
                rec.id,
                img.px_box(&rec.bbox),
                truncate(rec.value.get("text").and_then(Value::as_str).unwrap_or(""))
            ])
        })
        .collect();
    let mut dim_rows: Vec<Value> = on_tile(dims, &tile.world)
        .iter()
        .map(|rec| {
            json!([
                rec.id,
                img.px_box(&rec.bbox),
                truncate(
                    rec.value
                        .get("display")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                ),
                rec.value.get("measurement").cloned().unwrap_or(Value::Null)
            ])
        })
        .collect();
    let mut block_rows: Vec<Value> = on_tile(blocks, &tile.world)
        .iter()
        .map(|rec| {
            json!([
                rec.id,
                img.px_box(&rec.bbox),
                rec.value.get("block").cloned().unwrap_or(Value::Null)
            ])
        })
        .collect();
    let mut region_rows: Vec<Value> = on_tile(regions, &tile.world)
        .iter()
        .map(|rec| {
            json!([
                rec.id,
                img.px_box(&rec.bbox),
                rec.value.get("area").cloned().unwrap_or(Value::Null)
            ])
        })
        .collect();
    // Every record the tile draws, geometry and regions included: geometry
    // is the bulk of a tile, and a tile full of walls used to report no
    // layers at all, so filtering tiles by layer skipped it. Computed
    // before the truncation loop, so the layer set stays complete even
    // when rows are cut.
    let mut layer_set: BTreeSet<String> = BTreeSet::new();
    for rec in on_tile(texts, &tile.world)
        .iter()
        .chain(on_tile(dims, &tile.world).iter())
        .chain(on_tile(blocks, &tile.world).iter())
        .chain(on_tile(regions, &tile.world).iter())
        .chain(on_tile(geometry, &tile.world).iter())
    {
        if let Some(Value::String(l)) = rec.value.get("layer") {
            layer_set.insert(l.clone());
        }
    }
    let layers_total = layer_set.len();
    let mut layers: Vec<String> = layer_set.into_iter().collect();
    let build = |text_rows: &[Value],
                 dim_rows: &[Value],
                 block_rows: &[Value],
                 region_rows: &[Value],
                 layers: &[String],
                 truncated: bool| {
        let mut value = json!({
            "$schema": SCHEMA,
            "id": img.id,
            "png": img.png,
            "z": tile.z,
            "row": tile.row,
            "col": tile.col,
            "px": img.px,
            "canvas_origin_px": [tile.origin_px.0, tile.origin_px.1],
            "world": rounder.rect(&img.world),
            "ppu": img.ppu,
            "world_to_px": img.world_to_px,
            "px_to_world": img.px_to_world,
            "overlap_px": profile.overlap,
            "neighbors": neighbors,
            "parent": parent,
            "children": children,
            "empty": tile.empty,
            "layers_present": layers,
            "layers_truncated": layers.len() < layers_total,
            "records": { "texts": text_rows, "dims": dim_rows, "blocks": block_rows, "regions": region_rows },
            "records_truncated": truncated,
        });
        if layers.len() < layers_total {
            value["layers_total"] = json!(layers_total);
        }
        value
    };
    let mut truncated = false;
    let mut value = build(
        &text_rows,
        &dim_rows,
        &block_rows,
        &region_rows,
        &layers,
        truncated,
    );
    // The shrink used to cut the four row lists and nothing else, and stop
    // as soon as they were empty -- so a tile whose records were few but
    // whose layers were many (a plan of 900 AIA-named layers, each drawn
    // across the whole sheet) wrote a 44 KB sidecar, 37 % over the budget,
    // with `records_truncated: false` to say the file was complete. The
    // layer list is cut the same way once the rows are gone, and each list
    // has its own flag, so the file always says which of the two the reader
    // is missing.
    while serde_json::to_string(&value).map_or(0, |s| s.len()) > SIDECAR_LIMIT {
        let rows_left = [
            text_rows.len(),
            dim_rows.len(),
            block_rows.len(),
            region_rows.len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if rows_left > 0 {
            truncated = true;
            for rows in [
                &mut text_rows,
                &mut dim_rows,
                &mut block_rows,
                &mut region_rows,
            ] {
                let keep = rows.len() * 3 / 4;
                rows.truncate(keep);
            }
        } else if !layers.is_empty() {
            // Records first, layers after: a record is what a reader came
            // for, the layer list is an index into them. Alphabetical, so
            // which names survive is at least predictable.
            let keep = layers.len() * 3 / 4;
            layers.truncate(keep);
        } else {
            break;
        }
        value = build(
            &text_rows,
            &dim_rows,
            &block_rows,
            &region_rows,
            &layers,
            truncated,
        );
    }
    value
}

/// Area-weighted centroid of a polygon's vertices (straight segments).
fn polygon_centroid(vertices: &[Point2D]) -> Point2D {
    let n = vertices.len();
    let (mut cx, mut cy, mut area) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (vertices[i], vertices[(i + 1) % n]);
        let cross = a.x * b.y - b.x * a.y;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
        area += cross;
    }
    if area.abs() < 1e-12 {
        let sx: f64 = vertices.iter().map(|p| p.x).sum();
        let sy: f64 = vertices.iter().map(|p| p.y).sum();
        return Point2D {
            x: sx / n as f64,
            y: sy / n as f64,
        };
    }
    Point2D {
        x: cx / (3.0 * area),
        y: cy / (3.0 * area),
    }
}

fn point_in_polygon(p: Point2D, vertices: &[Point2D]) -> bool {
    let n = vertices.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (vertices[i], vertices[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Gives every region the ids of the texts whose anchor lies inside it and
/// in no smaller region.
fn label_regions(
    db: &CadDatabase,
    visible: &[&Entity],
    texts: &[PlacedText],
    regions: &mut [Record],
) {
    let polygons: BTreeMap<&str, &[Point2D]> = visible
        .iter()
        .filter_map(|e| match e {
            Entity::LwPolyline(p) | Entity::Polyline2D(p) if p.closed && p.vertices.len() >= 3 => {
                Some((p.common.handle.as_str(), p.vertices.as_slice()))
            }
            _ => None,
        })
        .collect();
    let _ = db;
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in texts {
        let mut best: Option<(f64, &str)> = None;
        for r in regions.iter() {
            if !r
                .bbox
                .intersects(&Rect::new(t.anchor.x, t.anchor.y, t.anchor.x, t.anchor.y))
            {
                continue;
            }
            let Some(vertices) = polygons.get(r.id.as_str()) else {
                continue;
            };
            if point_in_polygon(t.anchor, vertices) {
                let area = r
                    .value
                    .get("area")
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::INFINITY);
                if best.is_none_or(|(a, _)| area < a) {
                    best = Some((area, r.id.as_str()));
                }
            }
        }
        if let Some((_, id)) = best {
            labels.entry(id.to_string()).or_default().push(t.id.clone());
        }
    }
    for r in regions.iter_mut() {
        let ids = labels.remove(&r.id).unwrap_or_default();
        r.value.insert("labels".into(), json!(ids));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_affines_compose_and_mirror() {
        let quarter = std::f64::consts::FRAC_PI_2;
        let inner = Affine::placement(Point2D { x: 1.0, y: 0.0 }, 2.0, 2.0, 0.0);
        let outer = Affine::placement(Point2D { x: 0.0, y: 10.0 }, 1.0, 1.0, quarter);
        let both = inner.then(&outer);
        // inner: (1,1) -> (3,2); outer rotates 90 degrees about the origin and lifts by 10: (-2, 13).
        let p = both.apply(Point2D { x: 1.0, y: 1.0 });
        assert!(
            (p.x + 2.0).abs() < 1e-9 && (p.y - 13.0).abs() < 1e-9,
            "{p:?}"
        );
        assert!((both.length_scale() - 2.0).abs() < 1e-12);
        assert!((both.text_rotation(0.0) - quarter).abs() < 1e-12);

        // A 90-degree child under a parent mirrored about the vertical
        // axis (x scale -1) at (100,100): the child sends (10,0) to (0,10),
        // the mirror leaves y alone, so the point is at (100,110) -- where
        // the nested <g> groups draw it. Summing rotations gave (100,90).
        let child = Affine::placement(Point2D { x: 0.0, y: 0.0 }, 1.0, 1.0, quarter);
        let mirror = Affine::placement(Point2D { x: 100.0, y: 100.0 }, -1.0, 1.0, 0.0);
        let both = child.then(&mirror);
        let p = both.apply(Point2D { x: 10.0, y: 0.0 });
        assert!(
            (p.x - 100.0).abs() < 1e-9 && (p.y - 110.0).abs() < 1e-9,
            "{p:?}"
        );
        assert!(both.det() < 0.0, "a mirrored frame");
        assert!((both.length_scale() - 1.0).abs() < 1e-12);
        // A text drawn along the child's x axis has its baseline running
        // up the page ((1,0) -> (0,1)) and, after the mirror, the tops of
        // its glyphs pointing to +x ((0,1) -> (-1,0) -> (1,0)): the glyphs
        // are oriented like a text turned -90 degrees (mirror writing
        // advancing the other way), which is what the rotation reports.
        assert!((both.text_rotation(0.0) + quarter).abs() < 1e-12);
        // The top-level mirror alone keeps a horizontal text at 0 (the
        // glyphs are mirrored, not turned), as the records always said.
        assert!(mirror.text_rotation(0.0).abs() < 1e-12);
        // The estimate box of a 4-character height-2 text at the child's
        // (10,0) goes through the same frame: locally x 10..16.548, y
        // 0..2; the child turns it to x -2..0, y 10..16.548; the mirror at
        // (100,100) to x 100..102, y 110..116.548.
        let local = estimate_text_box(Point2D { x: 10.0, y: 0.0 }, 2.0, 0.0, "ABCD", 1.0, 0, 0);
        let world = both.apply_rect(&local);
        let width = 4.0 * 0.6 * 2.0 / 0.733;
        assert!(
            (world.min_x - 100.0).abs() < 1e-9
                && (world.max_x - 102.0).abs() < 1e-9
                && (world.min_y - 110.0).abs() < 1e-9
                && (world.max_y - (110.0 + width)).abs() < 1e-9,
            "{world:?}"
        );

        // A non-uniform parent (2,1) over the same child: (10,0) -> (0,10)
        // -> (100,110) and (0,2) -> (-2,0) -> (96,100); length scale
        // sqrt 2.
        let stretch = Affine::placement(Point2D { x: 100.0, y: 100.0 }, 2.0, 1.0, 0.0);
        let both = child.then(&stretch);
        let p = both.apply(Point2D { x: 0.0, y: 2.0 });
        assert!(
            (p.x - 96.0).abs() < 1e-9 && (p.y - 100.0).abs() < 1e-9,
            "{p:?}"
        );
        assert!((both.length_scale() - 2f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn grid_and_tile_plan_match_the_design() {
        let profile = Profile::CLAUDE;
        assert_eq!(grid(1000, 500, &profile), (1, 1));
        // 2688 px: (2688 - 1092) / 868 = 1.84 -> 2 + 1 = 3 columns.
        assert_eq!(grid(2688, 1792, &profile), (3, 2));
        let rect0 = Rect::new(0.0, 0.0, 2688.0, 1792.0);
        let extents = vec![Extent {
            handle: "1".into(),
            type_name: "LINE".into(),
            rect: Rect::new(2600.0, 100.0, 2650.0, 150.0),
        }];
        let tiles = plan_tiles("f0", 1, 2688, 1792, &rect0, 1.0, &profile, &extents);
        assert_eq!(tiles.len(), 6);
        assert!(tiles.iter().all(|t| t.width == 1092 && t.height == 1092));
        // The last column is shifted inward to end at the canvas edge.
        let last = tiles.iter().find(|t| t.row == 0 && t.col == 2).unwrap();
        assert_eq!(last.origin_px, (2688 - 1092, 0));
        assert_eq!(last.id, "f0/z1/r00_c02");
        // Only the tiles touching the line at the top right are non-empty:
        // rows are y-down, so row 0 holds y in [700, 1792] and the line at
        // y 100..150 lies in row 1.
        let non_empty: Vec<&str> = tiles
            .iter()
            .filter(|t| !t.empty)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(non_empty, ["f0/z1/r01_c02"]);
    }

    #[test]
    fn depth_reaches_the_target_text_height() {
        let options = ExportOptions::default();
        // 2.5-unit text at 0.5 px/unit is 1.25 px: 14 / 1.25 = 11.2 -> 2^4 = 16x.
        assert_eq!(depth_for(&[(2.5, 10)], 0.5, &options), 4);
        // Already legible: still one level.
        assert_eq!(depth_for(&[(50.0, 10)], 1.0, &options), 1);
        // No text: one level; max_levels caps.
        assert_eq!(depth_for(&[], 1.0, &options), 1);
        let capped = ExportOptions {
            max_levels: 2,
            ..Default::default()
        };
        assert_eq!(depth_for(&[(0.01, 1)], 0.1, &capped), 2);
    }

    #[test]
    fn sheet_directories_are_unique() {
        let mut used = BTreeSet::new();
        // Every Hangul syllable is outside [A-Za-z0-9_-], so both
        // three-syllable names sanitise to "___" and the second one (in tab
        // order) takes the suffix.
        assert_eq!(sheet_dir("\u{d3c9}\u{ba74}\u{b3c4}", &mut used), "___");
        assert_eq!(sheet_dir("\u{c785}\u{ba74}\u{b3c4}", &mut used), "____2");
        assert_eq!(sheet_dir("Layout 1", &mut used), "Layout_1");
        assert_eq!(sheet_dir("Layout_1", &mut used), "Layout_1_2");
        assert_eq!(sheet_dir("Layout-1", &mut used), "Layout-1");
        // Nothing survives sanitising: the placeholder, then its suffix.
        assert_eq!(sheet_dir("", &mut used), "sheet");
        assert_eq!(sheet_dir("", &mut used), "sheet_2");
        assert_eq!(used.len(), 7, "every name got its own directory");
    }

    #[test]
    fn coordinate_decimals_never_fall_below_the_scale() {
        // An ordinary drawing: a 10 000-unit plan on a 1568 px overview is
        // about 0.15 px/unit, 4.8 at the deepest of five levels, so four
        // decimals put one ulp at 5e-4 px -- the precision such a drawing
        // is stored with anyway.
        assert_eq!(coord_decimals(0, 4.8), 4);
        assert_eq!(coord_decimals(4, 4.8), 4);
        assert_eq!(coord_decimals(8, 4.8), 8, "a precise drawing keeps its 8");
        // A drawing 0.004 units across: 273 000 px/unit on the overview,
        // 8.7e6 at z5. Three or four decimals put every tile rectangle on
        // the same numbers; 10 keep one ulp at a thousandth of a pixel.
        assert_eq!(coord_decimals(4, 8.7e6), 10);
        assert!(10f64.powi(-10) * 8.7e6 < 1e-3);
        // Clamped at both ends, and a scale that is not a number at all
        // (a degenerate fit) falls back to the LUPREC floor.
        assert_eq!(coord_decimals(99, 4.8), 12);
        assert_eq!(coord_decimals(4, 1e30), 12);
        assert_eq!(coord_decimals(4, f64::NAN), 4);
        assert_eq!(coord_decimals(4, 0.0), 4);
    }

    #[test]
    fn rounding_ids_and_strings() {
        assert_eq!(round_to(1.23456789, 3), 1.235);
        assert_eq!(round_to(-0.0001, 3), 0.0);
        assert!(id_key("1F") < id_key("20"));
        assert!(id_key("A") < id_key("A/3"));
        assert_eq!(normalize_string("  Room   101 \n"), "room 101");
        assert_eq!(normalize_string("32.5㎡"), "32.5m2");
        assert_eq!(normalize_string("½\" Ø"), "1/2\" ø");
        assert_eq!(normalize_string("Ａ１"), "a1");
        assert_eq!(compact_string(" 32.5 m 2 "), "32.5m2");
        assert_eq!(area_unit("mm"), "mm2");
        assert_eq!(area_unit("du"), "du2");
        let sq = [
            Point2D { x: 0.0, y: 0.0 },
            Point2D { x: 4.0, y: 0.0 },
            Point2D { x: 4.0, y: 2.0 },
            Point2D { x: 0.0, y: 2.0 },
        ];
        let c = polygon_centroid(&sq);
        assert!((c.x - 2.0).abs() < 1e-12 && (c.y - 1.0).abs() < 1e-12);
        assert!(point_in_polygon(Point2D { x: 1.0, y: 1.0 }, &sq));
        assert!(!point_in_polygon(Point2D { x: 5.0, y: 1.0 }, &sq));
    }
}
