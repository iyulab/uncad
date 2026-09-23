//! The package itself: [`export_package`] writes a directory an agent can
//! read. See the crate documentation for what it holds.

mod output;
mod records;
mod tiles;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use iron_render_cad::{
    limits::Cap, Background, Fonts, Hidden, LeftOutReason, Part, PngError, Scene, Space,
    ToSvgOptions, View,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use uncad_model::model::{Entity, EntityId};
use uncad_model::{CadDatabase, ToJsonOptions};

use crate::dimension::{
    cached_labels, display_text, is_angular, measurement_from_points, usable_stored_measurement,
    DimDefaults, EffectiveStyle,
};
use crate::fonts;
use crate::frame::{
    auto_padding, crop_source, header_extents, renderer_crop, snap_to_lattice, CropMode,
    CropSource, Rect, EMPTY_RECT,
};
use crate::geom;
use crate::text::decode_text;

pub use output::WrittenFile;
use output::{clear_previous_package, sha256_hex, to_rgb, Writer};
use records::{
    capped_confidence, coord_decimals, handle_of, id_key, layer_name, placed_texts, text_record,
    PlacedText, Record, Rounder,
};

/// The `$schema` value every JSON file in the package carries.
pub const SCHEMA: &str = "uncad-package/1";

/// The stroke width of every image, in pixels.
const STROKE_PX: f64 = 1.25;

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

    /// The profiles by name: `claude`, `claude-hires`, `openai-patch`.
    pub fn by_name(name: &str) -> Option<Profile> {
        [
            Profile::CLAUDE,
            Profile::CLAUDE_HIRES,
            Profile::OPENAI_PATCH,
        ]
        .into_iter()
        .find(|p| p.name == name)
    }
}

impl Default for Profile {
    fn default() -> Self {
        Profile::CLAUDE
    }
}

/// What [`export_package`] writes, and how.
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
    /// [`crate::frame::auto_padding`]: 2 % of the longer side, at least 24
    /// output pixels. The sheets keep their own zero padding -- a sheet is
    /// the paper exactly.
    pub padding: Option<f64>,
    /// Draw hidden entities faded (they never enter the records or the
    /// crop). Default `false`.
    pub include_hidden: bool,
    /// Record files larger than this many KB are sharded. Default 96.
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
    /// frame. A group that does not stays in the overview only and is
    /// listed in `frames_dropped` with the reason `below_min_entities`.
    /// Default 20.
    pub min_frame_entities: usize,
    /// The most frames written (the primary one included); further groups
    /// stay in the overview only and are listed in `frames_dropped` with
    /// the reason `max_frames`. Default 8.
    pub max_frames: usize,
    /// The fonts text is drawn and measured with. Default
    /// [`fonts::bundled()`], the same pixels and text boxes on every
    /// machine; with any other the capital height is the renderer's default
    /// (0.7 of the em), since this crate knows only the bundled face's.
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
            fonts: fonts::bundled(),
            sheets: true,
        }
    }
}

/// Why [`export_package`] or [`export_file`] wrote no package.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExportError {
    /// A file or directory could not be written.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The renderer could not draw an image.
    Render(PngError),
    /// An image could not be re-encoded as 8-bit RGB.
    Encode(String),
    Json(serde_json::Error),
    /// The whole-model `entities.json` could not be serialized.
    Model(uncad_model::JsonError),
    /// [`export_file`] could not read the drawing.
    Parse(uncad::ParseError),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io { path, source } => {
                write!(f, "cannot write {}: {source}", path.display())
            }
            ExportError::Render(e) => write!(f, "rendering failed: {e}"),
            ExportError::Encode(e) => write!(f, "PNG encoding failed: {e}"),
            ExportError::Json(e) => write!(f, "JSON serialization failed: {e}"),
            ExportError::Model(e) => write!(f, "entities.json failed: {e}"),
            ExportError::Parse(e) => write!(f, "cannot read the drawing: {e}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::Io { source, .. } => Some(source),
            ExportError::Render(e) => Some(e),
            ExportError::Json(e) => Some(e),
            ExportError::Model(e) => Some(e),
            ExportError::Parse(e) => Some(e),
            ExportError::Encode(_) => None,
        }
    }
}

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

impl From<uncad_model::JsonError> for ExportError {
    fn from(e: uncad_model::JsonError) -> Self {
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
    /// `py = d x + e y + f`: `[s, 0, -x0 s, 0, -s, y1 s]`.
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

    /// The renderer's view of these pixels: the image's top-left corner at
    /// the world rectangle's, `ppu` pixels to the unit -- exactly the map
    /// `world_to_px` publishes.
    fn view(&self) -> View {
        View {
            left: self.world.min_x,
            top: self.world.max_y,
            px_per_unit: self.ppu,
            width: self.px[0],
            height: self.px[1],
        }
    }

    /// A world rectangle in this image's pixels, `[x0, y0, x1, y1]` with y
    /// down, rounded to whole pixels and **clipped to the image**.
    ///
    /// A record is listed on an image whenever its world box merely meets
    /// it, so the unclipped box of a record that continues past the edge
    /// named pixels the image does not have. The record's full extent is
    /// its world `bbox`, and the rest of it is on the other images it
    /// lists.
    fn px_box(&self, r: &Rect) -> [i64; 4] {
        let x0 = ((r.min_x - self.world.min_x) * self.ppu).floor();
        let x1 = ((r.max_x - self.world.min_x) * self.ppu).ceil();
        let y0 = ((self.world.max_y - r.max_y) * self.ppu).floor();
        let y1 = ((self.world.max_y - r.min_y) * self.ppu).ceil();
        let (w, h) = (i64::from(self.px[0]), i64::from(self.px[1]));
        let clamp = |v: f64, hi: i64| -> i64 {
            if v.is_nan() {
                0
            } else {
                (v.clamp(-9e15, 9e15) as i64).clamp(0, hi)
            }
        };
        [
            clamp(x0, w),
            clamp(y0, h),
            clamp(x1, w).max(clamp(x0, w)),
            clamp(y1, h).max(clamp(y0, h)),
        ]
    }
}

/// One zoom level of a frame's tile pyramid.
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

/// Why the picture leaves an entity out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeReason {
    /// Far larger than the rest of the drawing (the renderer's guard, 20x):
    /// not drawn.
    ScaleOutlier,
    /// Far away from the rest of the drawing (20 of its diagonals): not
    /// drawn.
    FarOutlier,
    /// Outside the crop's window.
    OutsideView,
}

/// A top-level entity the picture does not show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Excluded {
    /// Its reference ID in the model.
    pub id: u64,
    /// The file's handle for it, when it came from a file.
    pub handle: Option<String>,
    pub type_name: String,
    /// The world box it measured.
    pub rect: Rect,
    pub reason: ExcludeReason,
}

/// What the overview shows and what it leaves out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CropReport {
    pub source: CropSource,
    /// The world rectangle the overview shows, padding and lattice growth
    /// included.
    pub rect: Rect,
    /// The tight bounds of the entities inside the crop; `None` when
    /// nothing is.
    pub content: Option<Rect>,
    /// The padding added on each side, in drawing units (the lattice snap
    /// may add a little more on the right and bottom).
    pub padding_units: f64,
    /// The header's `$EXTMIN/$EXTMAX`, when stated and sane.
    pub header_extents: Option<Rect>,
    /// Entities outside the picture, in drawing order.
    pub excluded: Vec<Excluded>,
}

/// How many of each thing the package holds.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Counts {
    /// Top-level entities of model space.
    pub entities: usize,
    /// Every text record, model space and paper space together.
    pub texts: usize,
    /// The part of `texts` that is paper-space text (a sheet's title
    /// block): records with `space: "paper"` and a `sheet`.
    pub texts_paper: usize,
    pub dimensions: usize,
    pub geometry: usize,
    pub regions: usize,
    pub blocks: usize,
    /// Every hidden entity the renderer met, those inside block definitions
    /// included -- the number `report.json` prints as `hidden.count`.
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
    pub crop: CropReport,
    pub counts: Counts,
    pub warnings: Vec<String>,
}

/// The most entities `report.json`'s `hidden` names; the file says so with
/// `handles_limit` and `handles_truncated`.
const MAX_HIDDEN_NAMED: usize = 100;

/// The most vertices an outline may have before its self-intersection test
/// is skipped. [`geom::is_simple`] compares every pair of non-adjacent
/// segments, so the test costs O(n^2): a single closed polyline of 64 000
/// vertices (a surveyed contour, routine in a GIS import) held the export
/// for 175 s for one boolean on one record. No outline in the corpus comes
/// near this cap: the largest region in `AutoCADSamples5.dwg` has 378
/// vertices.
const MAX_SIMPLE_TEST_VERTICES: usize = 2_000;

/// What a record says instead of claiming an untested outline is simple.
const UNTESTED_OUTLINE: &str =
    "outline too large to test for self-intersection; the area assumes it does not cross itself";

/// Whether `vertices` outline a non-self-intersecting polygon, or `None`
/// when there are too many of them to ask. Unknown is reported as `null`,
/// never as `true`: a crossing outline's area is meaningless, and a reader
/// must be able to tell "checked, fine" from "not checked".
fn simple_outline(vertices: &[uncad_model::Point2D]) -> Option<bool> {
    (vertices.len() <= MAX_SIMPLE_TEST_VERTICES).then(|| geom::is_simple(vertices))
}

/// The most pixels one drawing unit may become (see [`fit_overview`]).
const MAX_PPU: f64 = 1e9;

/// The smallest world window an image of degenerate content gets, in
/// drawing units. Content with no size at all -- a lone POINT, coincident
/// entities, the base points of RAY/XLINE, which is all those entities
/// contribute -- gives the fit nothing to scale to. Such content gets a
/// window of its own, sized so the picture still reads as a drawing: at ten
/// units the scale stays in the hundreds of pixels per unit.
const MIN_CONTENT_EXTENT: f64 = 10.0;

/// An image fitted to the profile: the pixel size within both the edge and
/// the patch budget, the world rectangle (padded and lattice-snapped) and
/// the scale.
struct OverviewFit {
    rect: Rect,
    width: u32,
    height: u32,
    ppu: f64,
    padding: f64,
}

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
    let padding = padding.unwrap_or_else(|| auto_padding(content, Some(seed_ppu)));
    let padded = content.padded(padding);
    let ppu = (pw * lattice / padded.width()).min(ph * lattice / padded.height());
    // A zero-size window (a caller that forced zero padding on degenerate
    // content) makes `1568 / 0` infinite; the cap also keeps the scale of a
    // drawing a millionth of a unit across representable.
    let ppu = if ppu.is_finite() && ppu > 0.0 {
        ppu.min(MAX_PPU)
    } else {
        1.0
    };
    let (rect, width, height) = snap_to_lattice(&padded, ppu, profile.lattice);
    OverviewFit {
        rect,
        width,
        height,
        ppu,
        padding,
    }
}

/// The name the package's JSON gives a reason the drawing hides an entity.
fn hidden_name(h: Hidden) -> &'static str {
    match h {
        Hidden::Invisible => "invisible",
        Hidden::Defpoints => "defpoints",
        Hidden::LayerOff => "layer_off",
        Hidden::LayerFrozen => "layer_frozen",
        Hidden::FrozenInViewport => "frozen_in_viewport",
        Hidden::LayerNoPlot => "layer_no_plot",
        _ => "hidden",
    }
}

/// The name the package's JSON gives a renderer bound.
fn cap_name(c: Cap) -> &'static str {
    match c {
        Cap::EntityPoints => "entity_points",
        Cap::DocumentBytes => "document_bytes",
        Cap::EntityBytes => "entity_bytes",
        Cap::BlockRefs => "block_refs",
        Cap::HatchTile => "hatch_tile",
        Cap::NotANumber => "not_a_number",
        Cap::OutOfRange => "out_of_range",
        _ => "other",
    }
}

fn area_unit(unit: &str) -> String {
    format!("{unit}2")
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

/// Parses the drawing at `input` with its header and writes its package
/// into `dir` -- [`export_package`] for a file. When `options` names no
/// source, the manifest records the file's name.
pub fn export_file(
    input: &Path,
    dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let (db, header) = uncad::parse_with_header(input).map_err(ExportError::Parse)?;
    let named;
    let options = if options.source_name.is_none() {
        named = ExportOptions {
            source_name: input.file_name().map(|n| n.to_string_lossy().into_owned()),
            ..options.clone()
        };
        &named
    } else {
        options
    };
    export_package(&db, Some(&header), dir, options)
}

/// Writes the package for `db` into `dir` (created if needed). `header` is
/// the drawing's header as [`uncad::parse_with_header`] returns it -- the
/// units, the extents the `Auto` crop may take, `$LUPREC` and the dimension
/// variables a style falls back on; without one the package says its units
/// are drawing units (`du`) and uses the format's defaults.
///
/// A previous uncad package in `dir` is cleared first -- every file its
/// `manifest.json` listed, and the directories under `frames/` and
/// `sheets/` that empties -- so a re-export with other options leaves no
/// stale shards or tiles behind; nothing a manifest did not list is
/// touched. `manifest.json` is written last: a directory holding one is a
/// finished package.
pub fn export_package(
    db: &CadDatabase,
    header: Option<&uncad::Header>,
    dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let profile = options.profile;
    std::fs::create_dir_all(dir).map_err(|source| ExportError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    clear_previous_package(dir);
    let mut warnings: Vec<String> = Vec::new();
    let fonts = &options.fonts;
    let cap_height = if *fonts == fonts::bundled() {
        fonts::UNCAD_SANS_CAP_HEIGHT
    } else {
        iron_render_cad::DEFAULT_CAP_HEIGHT
    };

    // --- render once, and read what the renderer framed -------------------
    let header_rect = header.and_then(header_extents);
    let svg_options = ToSvgOptions {
        space: Space::Model,
        padding: 0.0,
        crop: renderer_crop(options.crop, header_rect),
        include_hidden: options.include_hidden,
        cap_height,
        ..ToSvgOptions::default()
    };
    let scene = Scene::new(db, svg_options);
    let parts = scene.parts();
    let top: BTreeMap<EntityId, &Entity> = db.entities.iter().map(|e| (e.common().id, e)).collect();
    // The picture's frame, before padding: the window of a window crop,
    // the entities' bounds otherwise.
    let framed: Option<Rect> = scene.crop.content.map(Rect::from);
    let window_crop = matches!(options.crop, CropMode::Fixed(_))
        || (options.crop == CropMode::Header && header_rect.is_some());
    // What the crop shows. Records cover what the picture shows: not the
    // hidden entities and not the ones the crop left out (they are listed
    // in report.json).
    let shown: Vec<(&Part, &Entity)> = parts
        .iter()
        .filter(|p| p.hidden.is_none() && p.left_out.is_none())
        .filter_map(|p| top.get(&p.id).map(|e| (p, *e)))
        .collect();
    let shown_ids: BTreeSet<EntityId> = shown.iter().map(|(p, _)| p.id).collect();
    let content: Option<Rect> = if window_crop {
        Rect::bounding(
            shown
                .iter()
                .filter_map(|(p, _)| p.extent.map(Rect::from))
                .collect::<Vec<_>>()
                .iter(),
        )
    } else {
        framed
    };
    let crop_rect = framed.unwrap_or(EMPTY_RECT);
    let source = crop_source(
        options.crop,
        header_rect,
        scene.crop.stated_taken,
        framed.as_ref(),
    );

    // --- the overview: the whole crop, sized to the profile ----------------
    let fit = fit_overview(&crop_rect, &profile, options.padding);
    // The short edge: `fit_overview` pins the long edge at the patch
    // budget, so the aspect ratio can only squeeze the other one.
    if fit.width.min(fit.height) < 200 {
        warnings.push(format!(
            "TinyOverview: the overview is only {} x {} px; the drawing's aspect leaves little of the patch budget",
            fit.width, fit.height
        ));
    }
    let overview = ImageInfo::new(
        "ov",
        "overview.png",
        fit.rect,
        fit.ppu,
        fit.width,
        fit.height,
    );
    // What the crop leaves out is not drawn either: a 3256x INSERT clipped
    // by the viewBox would still cross the whole picture.
    let overview_png =
        to_rgb(
            &scene.png(&overview.view(), STROKE_PX, fonts, Background::White, |p| {
                p.left_out.is_none()
            })?,
        )?;

    // --- units and rounding -----------------------------------------------
    let units = header.and_then(uncad::Header::units);
    let unit = units.as_ref().map_or("du".to_string(), |u| u.name.clone());
    let levels = i32::try_from(options.max_levels)
        .unwrap_or(i32::MAX)
        .min(64);
    let coords = coord_decimals(
        header.and_then(|h| h.luprec).unwrap_or(4),
        fit.ppu * 2f64.powi(levels),
    );
    let rounder = Rounder {
        coords,
        derived: coords + 2,
    };

    // --- texts, measured by the renderer with the package's fonts ----------
    let boxes = scene.text_boxes(fonts)?;
    let dimension_ids: BTreeSet<EntityId> = shown
        .iter()
        .filter(|(_, e)| matches!(e, Entity::Dimension(_)))
        .map(|(p, _)| p.id)
        .collect();
    // A dimension's label is drawn from its cached block, and is carried by
    // its own record's `display`.
    let texts = placed_texts(&db.tables, &top, &boxes, |id| {
        shown_ids.contains(&id) && !dimension_ids.contains(&id)
    });
    let unshaped_texts = texts.iter().filter(|t| t.unshaped > 0).count();
    if unshaped_texts > 0 {
        warnings.push(format!(
            "UnshapedGlyphs: {unshaped_texts} texts hold characters the fonts lack (drawn as boxes)"
        ));
    }
    let measured_why = if *fonts == fonts::bundled() {
        "glyph outlines as the renderer laid them out, bundled font"
    } else {
        "glyph outlines as the renderer laid them out"
    };

    // --- the drawn extents, widened by the measured texts ------------------
    // The renderer's extents count a text by its 0.6-em estimate; the text
    // boxes above are its glyphs. A Hangul syllable advances about 0.92
    // heights, so a long Korean note runs some 12 % past its estimate, while
    // `texts.json` lists a text's tiles from its measured box: the frames,
    // the frame overviews and the tiles are culled by these extents, so each
    // drawn part grows by the measured boxes of the texts it draws (the part
    // is the first ID of a text's path). An estimate that was generous is
    // never shrunk.
    let mut extent_of: BTreeMap<EntityId, Rect> = shown
        .iter()
        .filter_map(|(p, _)| p.extent.map(|r| (p.id, Rect::from(r))))
        .collect();
    for t in texts.iter().filter(|t| t.measured) {
        if let Some(e) = extent_of.get_mut(&t.part) {
            *e = e.union(&t.bbox);
        }
    }
    // In drawing order: the frames are the groups of these.
    let extents: Vec<Rect> = shown
        .iter()
        .filter_map(|(p, _)| extent_of.get(&p.id).copied())
        .collect();

    // --- frames: the primary group and each detached group ----------------
    let plan = tiles::plan_frames(
        &extents,
        crop_rect,
        &texts,
        &overview,
        options,
        &rounder,
        &mut warnings,
    );
    let frame_overviews: Vec<&ImageInfo> = plan
        .frames
        .iter()
        .map(|f| &f.report.overview)
        .filter(|ov| ov.id != "ov")
        .collect();
    let mut tile_images: Vec<ImageInfo> = Vec::new();
    for build in &plan.frames {
        for level in &build.report.levels {
            for tile in build.tiles.iter().filter(|t| t.z == level.z && !t.empty) {
                let png = format!(
                    "frames/{}/tiles/z{}/r{:02}_c{:02}.png",
                    tile.frame, tile.z, tile.row, tile.col
                );
                tile_images.push(ImageInfo::new(
                    &tile.id,
                    &png,
                    tile.world,
                    level.ppu,
                    tile.width,
                    tile.height,
                ));
            }
        }
    }
    // Every image but the overview, drawn on as many threads as there are:
    // each is a window of the one walk, holding only the parts that reach
    // it.
    let to_draw: Vec<&ImageInfo> = frame_overviews
        .iter()
        .copied()
        .chain(tile_images.iter())
        .collect();
    let drawn = tiles::render_images(&scene, fonts, &extent_of, &to_draw)?;

    // --- records with their images -------------------------------------------
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
    let with_images = |mut r: Record| -> Record {
        r.value.insert("tiles".into(), json!(tiles_for(&r.bbox)));
        r.value.insert("px".into(), px_map(&r.bbox));
        r
    };

    let mut text_records: Vec<Record> = texts
        .iter()
        .map(|t| with_images(text_record(t, &rounder, "model", None, measured_why)))
        .collect();
    text_records.sort_by_cached_key(|r| id_key(&r.id));

    let defaults = header.map(DimDefaults::from_header).unwrap_or_default();
    let dim_records = dimension_records(db, &shown, &extent_of, &defaults, &unit, &rounder)?
        .into_iter()
        .map(&with_images)
        .collect::<Vec<_>>();
    let (geo_records, region_records) =
        geometry_records(&shown, &extent_of, &texts, units.as_ref(), &unit, &rounder);
    let geo_records: Vec<Record> = geo_records.into_iter().map(&with_images).collect();
    let region_records: Vec<Record> = region_records.into_iter().map(&with_images).collect();
    let (block_records, instances) = block_records(&shown, &extent_of, &rounder);
    let block_records: Vec<Record> = block_records.into_iter().map(&with_images).collect();

    // --- strings -------------------------------------------------------------
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
    let frame_reports: Vec<FrameReport> = plan.frames.iter().map(|b| b.report.clone()).collect();
    let written_total: usize = frame_reports
        .iter()
        .flat_map(|f| f.levels.iter())
        .map(|l| l.tiles_written)
        .sum();

    // --- write -----------------------------------------------------------------
    let mut writer = Writer {
        dir,
        files: Vec::new(),
        shard_index: Vec::new(),
        units: json!({
            "name": unit,
            "insunits": header.and_then(|h| h.insunits),
            "to_mm": units.as_ref().and_then(|u| u.to_mm),
            "source": if units.is_some() { "header" } else { "none" },
        }),
        shard_kb: options.shard_kb,
    };
    writer.write_bytes("overview.png", &overview_png, "image")?;
    // Tile id -> its PNG's byte count and SHA-256, for `tiles.json`.
    let mut tile_png: BTreeMap<&str, (u64, String)> = BTreeMap::new();
    for (image, bytes) in to_draw.iter().zip(&drawn) {
        let is_tile = !frame_overviews.iter().any(|ov| ov.id == image.id);
        if is_tile {
            tile_png.insert(&image.id, (bytes.len() as u64, sha256_hex(bytes)));
        }
        writer.write_bytes(&image.png, bytes, if is_tile { "tile" } else { "image" })?;
    }
    writer.write_records("texts", "text", &text_records)?;
    writer.write_records("dimensions", "dimension", &dim_records)?;
    writer.write_records("geometry", "geometry", &geo_records)?;
    writer.write_records("regions", "region", &region_records)?;
    // The INSERT instances are records like any other kind: they go through
    // the same shard rule and land in `shard_index`. The definitions -- a
    // table, not a per-entity record -- are in drawing.json.
    writer.write_records("blocks", "blocks", &block_records)?;
    writer.write_json(
        "strings.json",
        &json!({
            "$schema": SCHEMA,
            "normalization": "NFKC, fraction slash to '/', case fold, whitespace collapsed; each string also under its key with spaces removed",
            "ids": "record ids: the drawing's entity reference ID as a decimal number, or `<insert>/<child>` for a text drawn inside a block. The kind is not in the id -- resolve it with manifest.shard_index, whose `first_key`/`last_key` bracket each file's ids by their first number (`int(id.split('/')[0])`). One id can be in two kinds at once: a closed polyline is both a geometry and a region record.",
            "strings": strings,
        }),
        "strings",
    )?;

    // --- sidecars and tiles.json -------------------------------------------------
    let on_tiles = tiles::OnTiles {
        texts: &text_records,
        dims: &dim_records,
        blocks: &block_records,
        regions: &region_records,
        geometry: &geo_records,
    };
    let mut tiles_json: Vec<Value> = Vec::new();
    for build in &plan.frames {
        for tile in &build.tiles {
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
            if tile.empty {
                // One reason a planned tile is not written: nothing visible
                // reaches it. A reader following `children` or a record's
                // `tiles` must know an absent file was meant to be absent.
                entry["reason"] = json!("no_visible_entity_on_tile");
            }
            if let Some(img) = tile_images.iter().find(|i| i.id == tile.id) {
                entry["png"] = json!(img.png);
                if let Some((bytes, sha)) = tile_png.get(img.id.as_str()) {
                    entry["bytes"] = json!(bytes);
                    entry["sha256"] = json!(sha);
                }
                let sidecar =
                    tiles::sidecar(img, tile, &build.tiles, &profile, &on_tiles, &rounder);
                let sidecar_path = img.png.replace(".png", ".json");
                writer.write_json_compact(&sidecar_path, &sidecar, "sidecar")?;
                entry["sidecar"] = json!(sidecar_path);
            }
            tiles_json.push(entry);
        }
    }
    writer.write_json(
        "tiles.json",
        &json!({
            "$schema": SCHEMA,
            "frames": frame_reports.iter().map(|f| json!({ "id": f.id, "kind": f.kind, "content": rounder.rect(&f.content), "levels": f.levels })).collect::<Vec<_>>(),
            "tiles": tiles_json,
        }),
        "tiles",
    )?;
    writer.write_json(
        "drawing.json",
        &drawing_json(db, header, parts, &top, &instances, &writer.units)?,
        "drawing",
    )?;

    // optional whole-model files
    let mut svg_origin: Option<[f64; 2]> = None;
    if options.svg {
        let window = crop_rect.padded(auto_padding(&crop_rect, None));
        let stroke = (window.diagonal() / 6000.0).max(0.01);
        let svg = scene.svg(window.into(), stroke, |p| p.left_out.is_none());
        writer.write_bytes("drawing.svg", svg.as_bytes(), "svg")?;
        svg_origin = Some([scene.origin.x, scene.origin.y]);
    }
    if options.full {
        let text = db.to_json(ToJsonOptions { pretty: false })?;
        writer.write_bytes("entities.json", text.as_bytes(), "entities")?;
    }

    // --- report.json -------------------------------------------------------
    let mut hidden_by_reason: BTreeMap<&str, usize> = BTreeMap::new();
    let mut hidden_named: Vec<(u64, Value)> = Vec::new();
    for p in parts {
        if let Some(reason) = p.hidden {
            *hidden_by_reason.entry(hidden_name(reason)).or_default() += 1;
            if hidden_named.len() < MAX_HIDDEN_NAMED {
                let handle = top
                    .get(&p.id)
                    .map_or(Value::Null, |e| handle_of(e.common()));
                hidden_named.push((p.id.value(), handle));
            }
        }
    }
    let limits = &scene.limits;
    if limits.engaged() {
        warnings.push(format!(
            "the drawing hit the renderer's robustness limits: {}",
            limits_summary(limits)
        ));
    }
    let excluded: Vec<Excluded> = scene
        .crop
        .left_out
        .iter()
        .map(|l| Excluded {
            id: l.id.value(),
            handle: top
                .get(&l.id)
                .and_then(|e| e.common().source_handle.resolved().cloned()),
            type_name: l.type_name.clone(),
            rect: Rect::from(l.extent),
            reason: match l.reason {
                LeftOutReason::ScaleOutlier => ExcludeReason::ScaleOutlier,
                LeftOutReason::FarOutlier => ExcludeReason::FarOutlier,
                _ => ExcludeReason::OutsideView,
            },
        })
        .collect();
    let crop_report = CropReport {
        source,
        rect: fit.rect,
        content,
        padding_units: fit.padding,
        header_extents: header_rect,
        excluded,
    };
    // "What is missing from the picture, and why" is one list: the crop's
    // own exclusions plus every entity a robustness bound acted on, each
    // naming the bound under `reason`.
    let excluded_json: Vec<Value> = crop_report
        .excluded
        .iter()
        .map(|e| json!(e))
        .chain(limits.dropped.iter().map(|d| {
            json!({
                "id": d.id.value(),
                "handle": top.get(&d.id).map_or(Value::Null, |e| handle_of(e.common())),
                "type_name": d.type_name,
                "reason": cap_name(d.cap),
            })
        }))
        .collect();
    let written_texts = &text_records;
    let counts = Counts {
        entities: parts.len(),
        texts: written_texts.len(),
        texts_paper: 0,
        dimensions: dim_records.len(),
        geometry: geo_records.len(),
        regions: region_records.len(),
        blocks: block_records.len(),
        hidden: scene.hidden,
        excluded: excluded_json.len(),
        tiles: written_total,
        frames: frame_reports.len(),
        sheets: 0,
    };
    let hidden_top_level: usize = hidden_by_reason.values().sum();
    // `count` is the number `manifest.counts.hidden` prints, every hidden
    // entity the renderer met; the split says how many of them the
    // top-level reasons cover.
    let report_value = json!({
        "$schema": SCHEMA,
        "excluded": excluded_json,
        "hidden": {
            "count": scene.hidden,
            "top_level": hidden_top_level,
            "inside_blocks": scene.hidden.saturating_sub(hidden_top_level),
            "covers": "top_level",
            "by_reason": hidden_by_reason,
            "ids": hidden_named.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            "handles": hidden_named.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>(),
            "handles_limit": MAX_HIDDEN_NAMED,
            "handles_truncated": hidden_top_level > hidden_named.len(),
        },
        "unsupported_types": scene.unsupported_types,
        "empty_blocks": scene.empty_blocks,
        "unresolved_block_refs": scene.unresolved_block_refs.iter().map(|id| id.value()).collect::<Vec<_>>(),
        "limits": limits_json(limits, &top),
        "warnings": warnings,
    });
    writer.files.push(WrittenFile {
        path: "report.json".into(),
        bytes: None,
        kind: "report".into(),
    });

    // --- manifest.json and README.txt, last (they list the files) ------------
    let manifest = manifest_json(ManifestInput {
        header,
        options,
        writer: &mut writer,
        crop: &crop_report,
        svg_origin,
        overview: &overview,
        frames: &frame_reports,
        frames_dropped: &plan.dropped,
        frames_dropped_total: plan.dropped_total,
        counts: &counts,
        texts: written_texts,
        dims: &dim_records,
        regions: &region_records,
        warnings: &warnings,
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)?;
    let readme = readme(options);
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
    // manifest.json is written last, after every file it names: a directory
    // that holds one is a finished package, and one that does not is
    // nothing a reader should trust. (It is also what the next run's
    // `clear_previous_package` reads.)
    std::fs::write(dir.join("manifest.json"), &manifest_text).map_err(|source| {
        ExportError::Io {
            path: dir.join("manifest.json"),
            source,
        }
    })?;

    Ok(ExportReport {
        dir: dir.to_path_buf(),
        files: writer.files,
        overview,
        frames: frame_reports,
        crop: crop_report,
        counts,
        warnings,
    })
}

/// The robustness bounds a render engaged, in a sentence.
fn limits_summary(l: &iron_render_cad::limits::LimitReport) -> String {
    let mut parts = Vec::new();
    for (n, what) in [
        (l.oversized_entities, "entities with too many points"),
        (
            l.block_refs_dropped,
            "block references past the depth or expansion budget",
        ),
        (l.hatch_patterns_dropped, "hatch patterns too fine to draw"),
        (l.entities_dropped, "entities past the document budget"),
        (l.truncated_parts, "entities drawn only in part"),
        (
            l.unreadable_entities,
            "entities with coordinates that are not numbers",
        ),
        (l.out_of_range_entities, "entities too far from the origin"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join(", ")
}

fn limits_json(
    l: &iron_render_cad::limits::LimitReport,
    top: &BTreeMap<EntityId, &Entity>,
) -> Value {
    json!({
        "oversized_entities": l.oversized_entities,
        "block_refs_dropped": l.block_refs_dropped,
        "hatch_patterns_dropped": l.hatch_patterns_dropped,
        "entities_dropped": l.entities_dropped,
        "truncated_parts": l.truncated_parts,
        "unreadable_entities": l.unreadable_entities,
        "out_of_range_entities": l.out_of_range_entities,
        "dropped": l.dropped.iter().map(|d| json!({
            "id": d.id.value(),
            "handle": top.get(&d.id).map_or(Value::Null, |e| handle_of(e.common())),
            "type_name": d.type_name,
            "cap": cap_name(d.cap),
        })).collect::<Vec<_>>(),
    })
}

/// The dimension records: every shown DIMENSION with its value, where the
/// value came from, the value its points give, the label and its source.
fn dimension_records(
    db: &CadDatabase,
    shown: &[(&Part, &Entity)],
    extent_of: &BTreeMap<EntityId, Rect>,
    defaults: &DimDefaults,
    unit: &str,
    rounder: &Rounder,
) -> Result<Vec<Record>, ExportError> {
    let labels = cached_labels(&db.tables);
    let mut out = Vec::new();
    for (part, e) in shown {
        let Entity::Dimension(d) = e else { continue };
        let bbox = extent_of.get(&part.id).copied().unwrap_or_else(|| {
            let p = d.definition_point.unwrap_or_default();
            Rect::at(p.x, p.y)
        });
        let angular = is_angular(d.kind);
        let from_points = measurement_from_points(d);
        let stored = usable_stored_measurement(d.measurement, d.kind, from_points);
        // `confidence` follows the source, not merely "there is a number": a
        // value the definition points gave is exact arithmetic on what the
        // file stores, but it is not what the drawing was measured at, and
        // a reader deciding whether to trust the number over the label
        // needs the two kept apart.
        let (measurement, source, confidence) = match (stored, from_points) {
            (Some(m), _) => (Some(m), "act_measurement", "stored"),
            (None, Some(p)) => (Some(p), "from_points", "exact"),
            (None, None) => (None, "none", "unavailable"),
        };
        let delta = match (stored, from_points) {
            (Some(m), Some(p)) => Some(rounder.derived(m - p)),
            _ => None,
        };
        let style = EffectiveStyle::resolve(
            d.style_name
                .resolved()
                .and_then(|name| db.tables.dim_styles.get(name)),
            defaults,
        );
        let shown_text = display_text(d, &style, measurement, labels.get(d.block_name.name()));
        let (confidence, why) = capped_confidence(confidence, d.common.confidence);
        let point = |p: Option<uncad_model::Point3D>| p.map_or(Value::Null, |p| rounder.pt3(p));
        let mut v = Map::new();
        v.insert("id".into(), json!(d.common.id.value().to_string()));
        v.insert("handle".into(), handle_of(&d.common));
        v.insert("kind".into(), serde_json::to_value(d.kind)?);
        v.insert("layer".into(), json!(layer_name(&d.common.layer)));
        v.insert("dimstyle".into(), json!(d.style_name.name()));
        v.insert(
            "measurement".into(),
            json!(measurement.map(|m| rounder.derived(m))),
        );
        v.insert("measurement_source".into(), json!(source));
        v.insert(
            "measurement_from_points".into(),
            json!(from_points.map(|m| rounder.derived(m))),
        );
        v.insert("delta".into(), json!(delta));
        v.insert("unit".into(), json!(if angular { "deg" } else { unit }));
        v.insert("dimlfac".into(), json!(style.dimlfac));
        v.insert("display".into(), json!(shown_text.text));
        if shown_text.raw != shown_text.text && !shown_text.raw.is_empty() {
            v.insert("display_raw".into(), json!(shown_text.raw));
        }
        v.insert(
            "display_source".into(),
            serde_json::to_value(shown_text.source)?,
        );
        if let uncad_model::model::TextOverride::Literal(s) = &d.text_override {
            v.insert("user_text".into(), json!(s));
        }
        v.insert(
            "points".into(),
            json!({
                "definition": point(d.definition_point),
                "extension1": point(d.points.extension1),
                "extension2": point(d.points.extension2),
                "radial": point(d.points.radial),
                "arc": point(d.points.arc),
            }),
        );
        if d.kind == Some(uncad_model::model::DimensionKind::Rotated) {
            v.insert(
                "rotation_deg".into(),
                json!(rounder.derived(d.rotation.to_degrees())),
            );
        }
        if let Some(axis) = d.ordinate_axis {
            v.insert("ordinate_axis".into(), serde_json::to_value(axis)?);
        }
        v.insert("definition_point".into(), point(d.definition_point));
        v.insert("text_at".into(), rounder.pt2(d.text_midpoint));
        v.insert("confidence".into(), json!(confidence));
        if let Some(why) = why {
            v.insert("why".into(), json!(why));
        }
        v.insert("bbox".into(), rounder.rect(&bbox));
        out.push(Record {
            id: d.common.id.value().to_string(),
            bbox,
            value: v,
        });
    }
    out.sort_by_cached_key(|r| id_key(&r.id));
    Ok(out)
}

/// The geometry records (every shown entity that is not a text, a
/// dimension or a block instance, and measured an extent) and the region
/// records (the closed polylines among them that enclose an area).
fn geometry_records(
    shown: &[(&Part, &Entity)],
    extent_of: &BTreeMap<EntityId, Rect>,
    texts: &[PlacedText],
    units: Option<&uncad::Units>,
    unit: &str,
    rounder: &Rounder,
) -> (Vec<Record>, Vec<Record>) {
    use std::f64::consts::{PI, TAU};
    let mut geometry: Vec<Record> = Vec::new();
    let mut regions: Vec<Record> = Vec::new();
    // The outlines of the regions, world coordinates, for their labels.
    let mut outlines: BTreeMap<String, Vec<uncad_model::Point2D>> = BTreeMap::new();
    for (part, e) in shown {
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
        let Some(bbox) = extent_of.get(&part.id).copied() else {
            continue;
        };
        let id = part.id.value().to_string();
        let mut v = Map::new();
        v.insert("id".into(), json!(id));
        v.insert("handle".into(), handle_of(e.common()));
        v.insert("type".into(), json!(e.type_name()));
        v.insert("layer".into(), json!(layer_name(&e.common().layer)));
        let mut confidence = "exact";
        let mut why: Option<&str> = None;
        match e {
            Entity::Line(l) => {
                v.insert("from".into(), rounder.pt3(l.start_point));
                v.insert("to".into(), rounder.pt3(l.end_point));
                // `length` is the length the file holds: the 3D distance.
                // `length_plan` and `dz` are added only when the line leaves
                // the plane, the only signal in an otherwise planar record
                // that it does.
                let (dx, dy, dz) = (
                    l.end_point.x - l.start_point.x,
                    l.end_point.y - l.start_point.y,
                    l.end_point.z - l.start_point.z,
                );
                let plan = dx.hypot(dy);
                v.insert("length".into(), json!(rounder.derived(plan.hypot(dz))));
                if rounder.derived(dz) != 0.0 {
                    v.insert("length_plan".into(), json!(rounder.derived(plan)));
                    v.insert("dz".into(), json!(rounder.derived(dz)));
                }
            }
            Entity::Arc(a) => {
                // Its own plane to the world: a mirrored arc (normal
                // (0,0,-1)) runs clockwise there, so its world arc runs
                // counter-clockwise from the image of its end to the image
                // of its start.
                let plane = geom::plane_to_world(a.extrusion, a.center.z);
                let center = plane.apply(uncad_model::Point2D {
                    x: a.center.x,
                    y: a.center.y,
                });
                let (start, end) = if geom::is_similarity(&plane) {
                    let turn = |angle: f64| {
                        let (s, c) = angle.sin_cos();
                        (plane.b * c + plane.d * s).atan2(plane.a * c + plane.c * s)
                    };
                    if plane.determinant() < 0.0 {
                        (turn(a.end_angle), turn(a.start_angle))
                    } else {
                        (turn(a.start_angle), turn(a.end_angle))
                    }
                } else {
                    confidence = "estimated";
                    why = Some("drawn in a tilted plane: the centre is its plan projection, the angles are the arc's own");
                    (a.start_angle, a.end_angle)
                };
                let mut sweep = end - start;
                if sweep <= 0.0 {
                    sweep += TAU;
                }
                v.insert("center".into(), rounder.pt2(center));
                v.insert("r".into(), json!(rounder.derived(a.radius)));
                v.insert(
                    "start_deg".into(),
                    json!(rounder.derived(start.to_degrees())),
                );
                v.insert("end_deg".into(), json!(rounder.derived(end.to_degrees())));
                v.insert(
                    "sweep_deg".into(),
                    json!(rounder.derived(sweep.to_degrees())),
                );
                v.insert("length".into(), json!(rounder.derived(a.radius * sweep)));
            }
            Entity::Circle(c) => {
                let plane = geom::plane_to_world(c.extrusion, c.center.z);
                let center = plane.apply(uncad_model::Point2D {
                    x: c.center.x,
                    y: c.center.y,
                });
                if !geom::is_similarity(&plane) {
                    why = Some("drawn in a tilted plane: the centre is its plan projection");
                }
                v.insert("center".into(), rounder.pt2(center));
                v.insert("r".into(), json!(rounder.derived(c.radius)));
                v.insert("length".into(), json!(rounder.derived(TAU * c.radius)));
                v.insert(
                    "area".into(),
                    json!(rounder.derived(PI * c.radius * c.radius)),
                );
            }
            Entity::Ellipse(el) => {
                let a = el.major_axis_endpoint.x.hypot(el.major_axis_endpoint.y);
                v.insert("center".into(), rounder.pt3(el.center));
                v.insert("major_axis".into(), rounder.pt3(el.major_axis_endpoint));
                v.insert("ratio".into(), json!(rounder.derived(el.axis_ratio)));
                let full = (el.end_angle - el.start_angle - TAU).abs() < 1e-9
                    || (el.start_angle == 0.0 && el.end_angle == 0.0);
                if full {
                    v.insert(
                        "area".into(),
                        json!(rounder.derived(PI * a * a * el.axis_ratio)),
                    );
                } else {
                    confidence = "unavailable";
                    why = Some("elliptical arc length is not computed");
                }
            }
            Entity::LwPolyline(p) | Entity::Polyline2D(p) => {
                // The vertices in the world, when the plane maps arcs to
                // arcs; a tilted plane's are projected, and its lengths and
                // area are measured in its own plane instead.
                let plane = geom::plane_to_world(p.extrusion, p.elevation);
                let world = geom::map_polyline(&p.vertices, &plane);
                let tilted = world.is_none();
                let world_vertices = world.unwrap_or_else(|| {
                    p.vertices
                        .iter()
                        .map(|v| uncad_model::PolylineVertex {
                            point: plane.apply(v.point),
                            ..*v
                        })
                        .collect()
                });
                let measured_on = if tilted { &p.vertices } else { &world_vertices };
                let bulged = world_vertices.iter().any(|v| v.bulge != 0.0);
                v.insert("closed".into(), json!(p.closed));
                v.insert(
                    "vfmt".into(),
                    json!(if bulged { "[x,y,bulge]" } else { "[x,y]" }),
                );
                let vertices: Vec<Value> = world_vertices
                    .iter()
                    .map(|vx| {
                        if bulged {
                            json!([
                                rounder.coord(vx.point.x),
                                rounder.coord(vx.point.y),
                                vx.bulge
                            ])
                        } else {
                            rounder.pt2(vx.point)
                        }
                    })
                    .collect();
                v.insert("vertices".into(), json!(vertices));
                let length = geom::polyline_length(measured_on, p.closed);
                v.insert(
                    if p.closed { "perimeter" } else { "length" }.into(),
                    json!(rounder.derived(length)),
                );
                if tilted {
                    confidence = "estimated";
                    why = Some("drawn in a tilted plane: the vertices are their plan projection, the length and area are measured in the entity's own plane");
                }
                if let Some(area) = geom::polyline_area(measured_on, p.closed) {
                    let signed = geom::polyline_signed_area(&world_vertices, p.closed);
                    let points: Vec<uncad_model::Point2D> =
                        world_vertices.iter().map(|v| v.point).collect();
                    let simple = simple_outline(&points);
                    v.insert("area".into(), json!(rounder.derived(area)));
                    v.insert(
                        "orientation".into(),
                        json!(if signed >= 0.0 { "ccw" } else { "cw" }),
                    );
                    v.insert("simple".into(), json!(simple));
                    let outline_why = match simple {
                        Some(true) => None,
                        Some(false) => {
                            confidence = "unavailable";
                            Some("self-intersecting outline: the area has no meaning")
                        }
                        None => {
                            if confidence == "exact" {
                                confidence = "estimated";
                            }
                            Some(UNTESTED_OUTLINE)
                        }
                    };
                    why = outline_why.or(why);
                    // Whatever the area could be measured of gets a region
                    // record: a closed two-vertex bulged polyline (AutoCAD's
                    // DONUT) is as much a closed shape as a triangle.
                    if p.closed && p.vertices.len() >= 2 {
                        let (region_confidence, capped) =
                            capped_confidence(confidence, e.common().confidence);
                        let mut r = Map::new();
                        r.insert("id".into(), json!(id));
                        r.insert("handle".into(), handle_of(e.common()));
                        r.insert("src".into(), json!(e.type_name()));
                        r.insert("layer".into(), json!(layer_name(&e.common().layer)));
                        r.insert("area".into(), json!(rounder.derived(area)));
                        r.insert("area_unit".into(), json!(area_unit(unit)));
                        r.insert(
                            "area_si".into(),
                            json!(units
                                .and_then(|u| u.to_mm)
                                .map(|mm| rounder.derived(area * mm * mm / 1e6))),
                        );
                        r.insert("perimeter".into(), json!(rounder.derived(length)));
                        r.insert(
                            "centroid".into(),
                            rounder.pt2(geom::polygon_centroid(&points)),
                        );
                        r.insert("vertex_count".into(), json!(p.vertices.len()));
                        r.insert("simple".into(), json!(simple));
                        // The same `confidence` *and the same `why`* as the
                        // geometry record this region is built from.
                        r.insert("confidence".into(), json!(region_confidence));
                        if let Some(w) = capped.or(why) {
                            r.insert("why".into(), json!(w));
                        }
                        r.insert("bbox".into(), rounder.rect(&bbox));
                        regions.push(Record {
                            id: id.clone(),
                            bbox,
                            value: r,
                        });
                        if points.len() >= 3 {
                            outlines.insert(id.clone(), points);
                        }
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
                why = Some("spline length is not evaluated");
            }
            Entity::Point(p) => {
                v.insert("at".into(), rounder.pt3(p.position));
            }
            Entity::Solid(s) | Entity::Trace(s) => {
                let plane = geom::plane_to_world(s.extrusion, s.elevation);
                v.insert(
                    "corners".into(),
                    json!([
                        rounder.pt2(plane.apply(s.corner1)),
                        rounder.pt2(plane.apply(s.corner2)),
                        rounder.pt2(plane.apply(s.corner3)),
                        rounder.pt2(plane.apply(s.corner4))
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
        let (confidence, capped) = capped_confidence(confidence, e.common().confidence);
        v.insert("unit".into(), json!(unit));
        v.insert("confidence".into(), json!(confidence));
        if let Some(w) = capped.or(why) {
            v.insert("why".into(), json!(w));
        }
        v.insert("bbox".into(), rounder.rect(&bbox));
        geometry.push(Record { id, bbox, value: v });
    }
    geometry.sort_by_cached_key(|r| id_key(&r.id));
    regions.sort_by_cached_key(|r| id_key(&r.id));
    label_regions(&outlines, texts, &mut regions);
    (geometry, regions)
}

/// Gives every region the ids of the texts whose anchor lies inside it and
/// in no smaller region.
fn label_regions(
    outlines: &BTreeMap<String, Vec<uncad_model::Point2D>>,
    texts: &[PlacedText],
    regions: &mut [Record],
) {
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in texts {
        let mut best: Option<(f64, &str)> = None;
        for r in regions.iter() {
            if !r.bbox.intersects(&Rect::at(t.anchor.x, t.anchor.y)) {
                continue;
            }
            let Some(vertices) = outlines.get(&r.id) else {
                continue;
            };
            if geom::point_in_polygon(t.anchor, vertices) {
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

/// The block instance records -- every shown INSERT that measured an
/// extent, with its attributes -- and the instances of each block, by name.
fn block_records(
    shown: &[(&Part, &Entity)],
    extent_of: &BTreeMap<EntityId, Rect>,
    rounder: &Rounder,
) -> (Vec<Record>, BTreeMap<String, Vec<String>>) {
    let mut instances: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut out = Vec::new();
    for (part, e) in shown {
        let Entity::Insert(i) = e else { continue };
        let Some(bbox) = extent_of.get(&part.id).copied() else {
            continue;
        };
        let id = part.id.value().to_string();
        instances
            .entry(i.block_name.name().to_string())
            .or_default()
            .push(id.clone());
        let attribs: Map<String, Value> = i
            .attribs
            .iter()
            .filter(|a| !a.tag.is_empty())
            .map(|a| (a.tag.clone(), json!(decode_text(&a.text).plain)))
            .collect();
        // Where the block's origin lands: the placement the model computes,
        // its own plane included.
        let placement = uncad_model::Affine2::from_insert(i);
        let mut v = Map::new();
        v.insert("id".into(), json!(id));
        v.insert("handle".into(), handle_of(&i.common));
        v.insert("block".into(), json!(i.block_name.name()));
        v.insert("layer".into(), json!(layer_name(&i.common.layer)));
        v.insert(
            "at".into(),
            rounder.pt2(uncad_model::Point2D {
                x: placement.e,
                y: placement.f,
            }),
        );
        v.insert(
            "rotation_deg".into(),
            json!(rounder.derived(i.rotation.to_degrees())),
        );
        v.insert("scale".into(), json!([i.scale.x, i.scale.y, i.scale.z]));
        v.insert("mirrored".into(), json!(placement.determinant() < 0.0));
        v.insert("attribs".into(), Value::Object(attribs));
        v.insert("bbox".into(), rounder.rect(&bbox));
        out.push(Record { id, bbox, value: v });
    }
    out.sort_by_cached_key(|r| id_key(&r.id));
    (out, instances)
}

/// `drawing.json`: the header, the units, the layers with their state and
/// how many top-level entities each holds, the block definitions with their
/// instances, and the entity counts.
fn drawing_json(
    db: &CadDatabase,
    header: Option<&uncad::Header>,
    parts: &[Part],
    top: &BTreeMap<EntityId, &Entity>,
    instances: &BTreeMap<String, Vec<String>>,
    units: &Value,
) -> Result<Value, ExportError> {
    let mut layer_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut type_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for p in parts {
        *type_counts.entry(p.type_name.as_str()).or_default() += 1;
        if let Some(e) = top.get(&p.id) {
            *layer_counts
                .entry(layer_name(&e.common().layer))
                .or_default() += 1;
        }
    }
    let layers: Vec<Value> = db
        .tables
        .layers
        .values()
        .map(|l| {
            json!({
                "name": l.name,
                "color_index": l.color_index,
                "on": !l.off,
                "frozen": l.frozen,
                "locked": l.locked,
                "plot": l.plot,
                // Hundredths of a millimetre in the model; -3 is "the
                // default", which is no width of its own.
                "lineweight_mm": l.lineweight.filter(|w| *w >= 0).map(|w| f64::from(w) / 100.0),
                "linetype": l.linetype.name(),
                "entity_count": layer_counts.get(l.name.as_str()).copied().unwrap_or(0),
            })
        })
        .collect();
    let definitions: Vec<Value> = db
        .tables
        .block_records
        .values()
        .filter(|b| {
            let upper = b.name.to_uppercase();
            !upper.starts_with("*MODEL_SPACE") && !upper.starts_with("*PAPER_SPACE")
        })
        .map(|b| {
            let mut by_layer: BTreeMap<&str, usize> = BTreeMap::new();
            let mut tags: BTreeSet<&str> = BTreeSet::new();
            for e in &b.entities {
                *by_layer.entry(layer_name(&e.common().layer)).or_default() += 1;
                if let Entity::Attdef(a) = e {
                    tags.insert(a.tag.as_str());
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
    Ok(json!({
        "$schema": SCHEMA,
        "units": units,
        "header": match header {
            Some(h) => serde_json::to_value(h)?,
            None => Value::Null,
        },
        "read_diagnostics": db.read_diagnostics.warnings,
        "layers": layers,
        "blocks": definitions,
        "counts": { "model_space": parts.len(), "by_type": type_counts },
    }))
}

struct ManifestInput<'a, 'w> {
    header: Option<&'a uncad::Header>,
    options: &'a ExportOptions,
    writer: &'a mut Writer<'w>,
    crop: &'a CropReport,
    svg_origin: Option<[f64; 2]>,
    overview: &'a ImageInfo,
    frames: &'a [FrameReport],
    frames_dropped: &'a [Value],
    frames_dropped_total: usize,
    counts: &'a Counts,
    texts: &'a [Record],
    dims: &'a [Record],
    regions: &'a [Record],
    warnings: &'a [String],
}

fn manifest_json(m: ManifestInput<'_, '_>) -> Value {
    let profile = m.options.profile;
    // "exact" is reserved for values the file itself measured: a package of
    // an R13/R14 drawing (no act_measurement anywhere) carries values this
    // crate recomputed from the definition points.
    let dim_source = |want: &str| {
        m.dims
            .iter()
            .any(|r| r.value.get("measurement_source").is_some_and(|s| s == want))
    };
    // `areas` says what the region records say, the way `dimension_values`
    // does.
    let area_confidence = |want: &str| {
        m.regions
            .iter()
            .filter(|r| r.value.get("confidence").is_some_and(|c| c == want))
            .count()
    };
    let (areas_exact, areas_estimated, areas_unavailable) = (
        area_confidence("exact"),
        area_confidence("estimated"),
        area_confidence("unavailable"),
    );
    let measured_total = m
        .texts
        .iter()
        .filter(|r| {
            r.value
                .get("bbox_confidence")
                .is_some_and(|c| c == "measured")
        })
        .count();
    let fonts_name = if m.options.fonts == fonts::bundled() {
        "bundled"
    } else if m.options.fonts == Fonts::System {
        "system"
    } else {
        "custom"
    };
    let capabilities = json!({
        "dimension_values": if m.dims.is_empty() { "none" } else if dim_source("act_measurement") { "exact" } else if dim_source("from_points") { "computed" } else { "text_only" },
        "areas": if m.regions.is_empty() { "none" } else if areas_exact == m.regions.len() { "exact" } else if areas_exact > 0 { "mixed" } else if areas_unavailable > 0 && areas_estimated == 0 { "unavailable" } else { "estimated" },
        "areas_by_confidence": { "exact": areas_exact, "estimated": areas_estimated, "unavailable": areas_unavailable },
        "text_boxes": if m.texts.is_empty() { "none" } else if measured_total == m.texts.len() { "measured" } else if measured_total == 0 { "estimated" } else { "mixed" },
        "fonts": fonts_name,
        "frames": m.frames.len(),
    });
    let legend = json!({
        "confidence": {
            "carried_by": ["geometry", "region", "dimension"],
            "exact": "computed from the file's own coordinates",
            "stored": "the value the file stores (a dimension's act_measurement)",
            "estimated": "approximated, or a check that was skipped; `why` says which",
            "unavailable": "no meaningful value; `why` says why, and any number beside it is not to be used",
            "none": "there was nothing to compute (used by capabilities, not by records)",
        },
        "text_records": "texts carry `bbox_confidence` (`measured` from the glyph outlines as the renderer laid them out, or `estimated` at 0.6 em per character) and no `confidence`; block instances carry neither.",
        "ids": "a record's `id` is the drawing's entity reference ID in decimal (a path of them for a text inside a block); its `handle` is the file's own handle for the entity, in hexadecimal, when it came from a file",
        "measurement_source": ["act_measurement", "from_points", "none"],
        "display_source": ["user_text", "cached_block", "formatted", "suppressed", "none"],
        "region_labels": "`labels` holds the ids of the text records whose anchor falls inside the region",
        "px_boxes": "`px` maps an image id to [x0, y0, x1, y1] in that image's pixels, y down, clipped to the image; the record's full extent is its world `bbox`, and the rest of it is on the other images it lists",
        "tile_sidecar": "`records` holds positional rows described by the sidecar's own `columns`; `counts` is the true number of records of each kind on the tile, which `records_truncated` does not affect, and `geometry_by_kind` summarises the geometry whether or not its rows fit",
        "shard_lookup": "manifest.shard_index resolves a record id to its file: the entry whose [first_key, last_key] contains int(id.split('/')[0]). `first_id`/`last_id` are the same bounds as strings and do not compare as numbers",
        "legibility": "`target_met` is whether every text height class reaches `target_px` in the deepest image of that frame; `pyramid_complete` is whether the tile budget let the pyramid reach the depth the text asked for",
    });
    // The tile and overlap numbers come from the profile in use, not from
    // the prose: claude-hires writes 1932 px tiles with 392 px of overlap.
    let guidance = format!(
        "Read manifest.json first. Numbers (lengths, areas, dimension values, text) come from the JSON records, never from pixels; `legend` says what `confidence` and the package's other vocabularies mean and which records carry them. To find something: look its text up in strings.json (normalised: trimmed, lower-case, single spaces), resolve the id through shard_index -- the entry whose [first_key, last_key] contains int(id.split('/')[0]), since the id does not say its kind and one id can be in two -- then open the tile(s) in its `tiles` list; every tile's .json sidecar lists what is on it with pixel boxes, its own `columns` legend and a `counts` object that stays exact when rows are cut. Pixel boxes are clipped to the image they are quoted in; the record's world `bbox` is its full extent. overview.png shows the whole crop; each frame in `frames` (f0 the main drawing, f1.. details drawn beside it) has its own overview and tiles z1..zN, {} px with {} px overlap (2x zooms), row 0 at the top; a group too small to be framed is in `frames_dropped` and its records carry `tiles: []`. report.json lists what was left out and why.",
        profile.tile, profile.overlap
    );
    m.writer.files.push(WrittenFile {
        path: "manifest.json".into(),
        bytes: None,
        kind: "manifest".into(),
    });
    m.writer.files.push(WrittenFile {
        path: "README.txt".into(),
        bytes: None,
        kind: "readme".into(),
    });
    json!({
        "$schema": SCHEMA,
        "generator": { "name": "uncad-export", "version": env!("CARGO_PKG_VERSION") },
        "profile": profile.name,
        "source": {
            "name": m.options.source_name,
            "format": m.header.map(|h| h.format),
            "acadver": m.header.and_then(|h| h.acadver.clone()),
            "version": m.header.and_then(|h| h.version.clone()),
            "codepage": m.header.and_then(|h| h.codepage_name.clone()),
        },
        "units": m.writer.units,
        "crop": m.crop,
        // drawing.svg's user units are world minus this (null without it).
        "svg_origin": m.svg_origin,
        "overview": m.overview,
        "frames": m.frames,
        "frames_dropped": m.frames_dropped,
        "frames_dropped_total": m.frames_dropped_total,
        "legibility": { "target_px": m.options.target_text_px, "per_frame": m.frames.iter().map(|f| json!({"frame": f.id, "z_max": f.z_max, "target_met": f.height_classes.iter().all(|c| c.legible), "pyramid_complete": f.reached, "height_classes": f.height_classes})).collect::<Vec<_>>() },
        "counts": m.counts,
        "capabilities": capabilities,
        "legend": legend,
        "guidance": guidance,
        "files": m.writer.files,
        "shard_index": m.writer.shard_index,
        "warnings": m.warnings,
    })
}

fn readme(options: &ExportOptions) -> String {
    format!(
        "uncad package ({SCHEMA})\n\nReading order:\n  1. manifest.json   what is here, the crop, the images and their affines; `legend` explains the record vocabularies, `guidance` how to look something up\n  2. strings.json    find a text or a number, get record ids; shard_index turns an id into a file (compare int(id.split('/')[0]) against first_key/last_key, not the strings)\n  3. texts.json / dimensions.json / geometry.json / regions.json / blocks.json   the records (sharded above {} KB, see shard_index); blocks.json holds the INSERT instances, drawing.json the block definitions\n  4. overview.png    the whole drawing; frames/f*/overview.png and frames/f*/tiles/z*/  zoomed tiles with .json sidecars; tiles.json lists every tile, written or empty with a reason, with its size and sha256\n  5. report.json     what was left out and why\n\ndrawing.json holds the header, units, layer states and block definitions; entities.json and drawing.svg (when present) are tool inputs, not for reading.\n",
        options.shard_kb
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_normalized_for_lookup() {
        assert_eq!(normalize_string("  Room   101 \n"), "room 101");
        assert_eq!(normalize_string("32.5㎡"), "32.5m2");
        assert_eq!(normalize_string("½\" Ø"), "1/2\" ø");
        assert_eq!(normalize_string("Ａ１"), "a1");
        assert_eq!(compact_string(" 32.5 m 2 "), "32.5m2");
        assert_eq!(area_unit("mm"), "mm2");
        assert_eq!(area_unit("du"), "du2");
    }

    #[test]
    fn profiles_by_name() {
        assert_eq!(Profile::by_name("claude"), Some(Profile::CLAUDE));
        assert_eq!(
            Profile::by_name("claude-hires"),
            Some(Profile::CLAUDE_HIRES)
        );
        assert_eq!(
            Profile::by_name("openai-patch"),
            Some(Profile::OPENAI_PATCH)
        );
        assert_eq!(Profile::by_name("gemini"), None);
    }

    #[test]
    fn an_overview_uses_its_budget_and_a_point_gets_a_window() {
        // A 1000 x 500 drawing on Claude's budget: 56 patches wide is the
        // edge limit, and 28 of them tall keeps 1568 patches.
        let fit = fit_overview(&Rect::new(0.0, 0.0, 1000.0, 500.0), &Profile::CLAUDE, None);
        assert_eq!(fit.width % 28, 0);
        assert_eq!(fit.height % 28, 0);
        assert!(fit.width <= 1568 && (fit.width / 28) * (fit.height / 28) <= 1568);
        assert!((fit.rect.width() * fit.ppu - f64::from(fit.width)).abs() < 1e-6);
        // A single point: a ten-unit window, not a 1e-11-unit one.
        let fit = fit_overview(&Rect::at(3.0, 3.0), &Profile::CLAUDE, None);
        assert!((10.0..=12.0).contains(&fit.rect.width()), "{:?}", fit.rect);
        assert!(fit.ppu.is_finite() && fit.ppu < 1000.0);
    }
}
