//! SVG -> PNG rasterization, layered on top of [`crate::svg`]'s renderer via
//! resvg/usvg/tiny-skia (all pure Rust, no system library dependency -- see
//! [`crate::svg`]'s own doc comment for the CAD -> SVG side; this module only
//! handles the raster step).
//!
//! The output is meant to be read by people and by vision models, which set
//! the defaults here (see `docs/VLM_EXPORT_DESIGN.md`, "Rendering changes"):
//! the image is sized in pixels ([`PngSize::FitLongEdge`], 1568 px), not in
//! drawing units; the background is opaque white; strokes are a fixed number
//! of output pixels wide rather than a fraction of the drawing; and both
//! dimensions are capped ([`ToPngOptions::max_edge`]) so a drawing with
//! runaway coordinates fails with [`PngError::TooLarge`] instead of trying to
//! allocate terabytes. Every result carries the viewBox and the pixel scale,
//! so a consumer can map a pixel back to drawing coordinates.
//!
//! A single `resvg` dependency is used rather than separate `usvg` /
//! `tiny-skia` / `fontdb` crates: `resvg` re-exports both (`resvg::usvg`,
//! `resvg::tiny_skia`, and `usvg::fontdb`), which keeps the three in lock
//! step instead of risking a version mismatch across independently pinned
//! crates. The `png` crate (already a dependency of tiny-skia) writes the
//! 8-bit RGB output; tiny-skia's own encoder only writes RGBA.

use crate::crop::{self, CropReport};
use crate::svg::{self, ToSvgOptions, ViewBox};
use crate::CadDatabase;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};
use std::sync::{Arc, OnceLock};

/// How the image's pixel size is chosen from the rendering's viewBox.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PngSize {
    /// The longer side of the image is this many pixels; the other follows
    /// the viewBox's aspect ratio. The default is 1568, the largest edge a
    /// Claude standard-tier image keeps without server-side resizing.
    FitLongEdge(u32),
    /// A fixed number of pixels per drawing unit.
    PxPerUnit(f64),
    /// 0.2.0's rule: the viewBox's size in drawing units times this factor.
    /// `Scale(1.0)` gives one pixel per drawing unit.
    Scale(f64),
}

impl Default for PngSize {
    fn default() -> Self {
        PngSize::FitLongEdge(1568)
    }
}

/// What the pixels the drawing does not touch are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Background {
    /// Opaque white, encoded as 8-bit RGB. The default: the colours are
    /// normalized for a white page (`crate::color`), and a transparent image
    /// composited on black by a viewer or a JPEG conversion showed nothing
    /// but the coloured lines.
    #[default]
    White,
    /// Fully transparent, encoded as 8-bit RGBA (0.2.0's output).
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToPngOptions {
    pub svg: ToSvgOptions,
    pub size: PngSize,
    pub background: Background,
    /// Stroke width in output pixels for every stroke, converted to drawing
    /// units once the pixel scale is known. `None` keeps [`ToSvgOptions`]'s
    /// rule (an explicit `stroke_width`, else ~1/6000th of the viewBox
    /// diagonal, which is a sub-pixel hairline at most sizes). An explicit
    /// `svg.stroke_width` takes precedence over this. Default 1.25.
    pub stroke_px: Option<f64>,
    /// Neither pixel dimension may exceed this; larger requests fail with
    /// [`PngError::TooLarge`]. Default 8000 (the largest image the Claude
    /// API accepts; also keeps a runaway viewBox from allocating gigabytes).
    pub max_edge: u32,
    /// Round the pixel size up to a multiple of this (the model's patch
    /// size: 28 for Claude, 32 for OpenAI's gpt-5.x); the world rectangle
    /// grows on the right and bottom to match, so pixels and units stay in
    /// exact proportion. 0 turns it off. Default 28. Since 0.3.0.
    pub lattice: u32,
    /// The fonts text is drawn with. Default [`Fonts::Bundled`]. Since 0.3.0.
    pub fonts: Fonts,
}

impl Default for ToPngOptions {
    fn default() -> Self {
        ToPngOptions {
            svg: ToSvgOptions::default(),
            size: PngSize::default(),
            background: Background::default(),
            stroke_px: Some(1.25),
            max_edge: 8000,
            lattice: 28,
            fonts: Fonts::Bundled,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToPngResult {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The rendering's viewBox, i.e. what the image shows -- see
    /// [`ViewBox`] for the world-to-pixel mapping together with
    /// [`px_per_unit`](Self::px_per_unit). In world units.
    pub view_box: ViewBox,
    /// Pixels per drawing unit: `pixel = (world - viewBox origin) * this`.
    pub px_per_unit: f64,
    /// The world point the intermediate SVG's coordinates were relative
    /// to (see [`crate::ToSvgResult::origin`]); the pixel mapping above is
    /// unaffected. Since 0.3.0.
    pub origin: [f64; 2],
    pub unsupported_types: Vec<String>,
    /// Entities hidden by the drawing -- see [`crate::ToSvgResult::hidden`].
    pub hidden: usize,
    /// How the image's rectangle was chosen and what it leaves out; its
    /// `rect` is `view_box`'s world rectangle. Since 0.3.0.
    pub crop: CropReport,
    /// What the robustness caps in [`crate::limits`] took away -- see
    /// [`crate::ToSvgResult::limits`]. Since 0.3.0.
    pub limits: crate::limits::LimitReport,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum PngError {
    /// usvg couldn't parse the intermediate SVG string -- since that string
    /// comes from this crate's own `to_svg`, this would indicate a bug
    /// there rather than bad input from a caller.
    InvalidSvg(usvg::Error),
    /// The requested pixel size rounds to zero in at least one dimension.
    EmptyCanvas,
    /// [`PngSize`] holds a value that is not a finite positive number.
    InvalidSize,
    /// The requested pixel size exceeds [`ToPngOptions::max_edge`].
    TooLarge {
        width: u32,
        height: u32,
        max_edge: u32,
    },
    /// The PNG encoder failed. Stored as its `Display` text rather than the
    /// underlying error type so the encoder crate stays out of this crate's
    /// public API.
    Encode(String),
    /// The rasterizer panicked. tiny-skia's scan converter asserts instead
    /// of returning an error when a path's coordinates overflow its
    /// fixed-point edge list (`edges[curr_idx].last_y >= curr_y`), which a
    /// bug in this crate's SVG -- or a drawing nobody has thought of yet --
    /// can still provoke; [`draw`] catches it so a caller, and the export's
    /// tile threads, get an error rather than a dead process. Carries the
    /// panic's own message when it had one. Since 0.3.0.
    RenderPanic(String),
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PngError::InvalidSvg(e) => write!(f, "SVG parsing failed: {e}"),
            PngError::EmptyCanvas => write!(f, "render size is zero (check the size option)"),
            PngError::InvalidSize => write!(f, "the size option is not a finite positive number"),
            PngError::TooLarge {
                width,
                height,
                max_edge,
            } => write!(
                f,
                "render size {width}x{height} exceeds the {max_edge} px limit (raise max_edge, or use a smaller --fit/--scale)"
            ),
            PngError::Encode(e) => write!(f, "PNG encoding failed: {e}"),
            PngError::RenderPanic(e) => write!(f, "the rasterizer panicked: {e}"),
        }
    }
}
impl std::error::Error for PngError {}

/// [`resvg::render`] with a panic turned into a [`PngError::RenderPanic`].
///
/// resvg hands the path straight to tiny-skia, whose scan converter asserts
/// on coordinates its 24.8 fixed-point edge list cannot hold. Nothing is
/// read back out of `pixmap` after a panic -- the error is returned
/// instead -- so asserting unwind safety over it is sound.
fn draw(
    tree: &usvg::Tree,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::Pixmap,
) -> Result<(), PngError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(tree, transform, &mut pixmap.as_mut());
    }))
    .map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_string());
        PngError::RenderPanic(message)
    })
}

/// Renders `db` straight to PNG bytes -- the intermediate SVG text never
/// touches disk.
///
/// Text is shaped with the bundled `Uncad Sans` face by default
/// ([`Fonts`]), so the image is the same on every machine. A character the
/// subset lacks is not an error and is not dropped: usvg keeps it as glyph
/// 0, which the subset carries as a crossed `.notdef` box with its own
/// advance, so it is drawn as a box and the rest of the string keeps its
/// layout (the export counts it in `unshaped_glyphs`). usvg logs a
/// `log::warn!` for it only when the embedding application installs a
/// `log` implementation; this crate and the CLI install none.
pub fn to_png(db: &CadDatabase, options: ToPngOptions) -> Result<ToPngResult, PngError> {
    let rendered = svg::render(db, options.svg);
    let content = rendered.choice.rect;
    let longer = content.longer_side().max(1e-9);

    // The scale and the padding depend on each other (24 px of padding is
    // more the smaller the scale), so the fit is solved in two steps: a
    // seed scale assuming 2 % padding a side, the padding from that, then
    // the scale that fits the padded crop into the snapped pixel count.
    let (px_per_unit, padding_units) = match options.size {
        PngSize::FitLongEdge(px) => {
            let fit = match options.lattice {
                0 => px,
                lattice => px - px % lattice,
            };
            if fit == 0 {
                return Err(PngError::InvalidSize);
            }
            let seed = f64::from(fit) / (1.04 * longer);
            let pad = options
                .svg
                .padding
                .unwrap_or_else(|| crop::auto_padding(&content, Some(seed)));
            (f64::from(fit) / (longer + 2.0 * pad), pad)
        }
        PngSize::PxPerUnit(s) | PngSize::Scale(s) => {
            let pad = options
                .svg
                .padding
                .unwrap_or_else(|| crop::auto_padding(&content, Some(s)));
            (s, pad)
        }
    };
    if !px_per_unit.is_finite() || px_per_unit <= 0.0 || !padding_units.is_finite() {
        return Err(PngError::InvalidSize);
    }
    let (rect, width, height) =
        crop::snap_to_lattice(&content.padded(padding_units), px_per_unit, options.lattice);
    let view_box = ViewBox::from_world(&rect);
    if width == 0 || height == 0 {
        return Err(PngError::EmptyCanvas);
    }
    if width > options.max_edge || height > options.max_edge {
        return Err(PngError::TooLarge {
            width,
            height,
            max_edge: options.max_edge,
        });
    }

    // Stroke width in drawing units: an explicit SVG width wins, then the
    // pixel width scaled back to units, then the viewBox-relative rule.
    let stroke_width = match (options.svg.stroke_width, options.stroke_px) {
        (Some(units), _) => units,
        (None, Some(px)) => px / px_per_unit,
        (None, None) => svg::auto_stroke_width(&view_box),
    };
    let svg_text = svg::assemble(&rendered, &view_box, stroke_width);

    let png = rasterize(
        &svg_text,
        px_per_unit,
        width,
        height,
        options.background,
        options.fonts,
    )?;
    Ok(ToPngResult {
        png,
        width,
        height,
        view_box,
        px_per_unit,
        origin: rendered.origin,
        unsupported_types: rendered.unsupported_types(),
        hidden: rendered.hidden,
        crop: rendered.choice.report(rect, padding_units),
        limits: rendered.limits,
    })
}

/// Rasterizes an already-built SVG string to RGBA PNG bytes at `scale`x the
/// SVG's own viewBox-derived size, on a transparent background: 0.2.0's
/// behaviour, kept for callers that hold an SVG string from a separately
/// cached [`crate::CadDatabase::to_svg`] call. [`to_png`] is the way to get
/// a sized, opaque image with a known pixel scale.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, PngError> {
    let tree = parse_tree(svg, Fonts::Bundled)?;
    let size = tree.size();
    let width = (size.width() * scale).round() as u32;
    let height = (size.height() * scale).round() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::EmptyCanvas)?;
    draw(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap,
    )?;
    pixmap
        .encode_png()
        .map_err(|e| PngError::Encode(e.to_string()))
}

/// The pixel size a viewBox gets at `px_per_unit`, rounded to whole pixels.
fn rasterize(
    svg_text: &str,
    px_per_unit: f64,
    width: u32,
    height: u32,
    background: Background,
    fonts: Fonts,
) -> Result<Vec<u8>, PngError> {
    let tree = parse_tree(svg_text, fonts)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::EmptyCanvas)?;
    if background == Background::White {
        pixmap.fill(tiny_skia::Color::WHITE);
    }
    let scale = px_per_unit as f32;
    draw(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap,
    )?;
    match background {
        Background::White => encode_rgb8(&pixmap),
        Background::Transparent => pixmap
            .encode_png()
            .map_err(|e| PngError::Encode(e.to_string())),
    }
}

/// Renders the `width` x `height` pixel window of `tree` whose top-left
/// corner sits at `origin_px` on the canvas `tree` covers at `px_per_unit`,
/// on white, as 8-bit RGB PNG bytes. The export writes its overview and
/// tiles through this with one parsed tree per zoom level.
pub(crate) fn render_region(
    tree: &usvg::Tree,
    px_per_unit: f64,
    origin_px: (f64, f64),
    width: u32,
    height: u32,
) -> Result<Vec<u8>, PngError> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::EmptyCanvas)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    let scale = px_per_unit as f32;
    let transform = tiny_skia::Transform::from_scale(scale, scale)
        .post_translate(-(origin_px.0 as f32), -(origin_px.1 as f32));
    draw(tree, transform, &mut pixmap)?;
    encode_rgb8(&pixmap)
}

/// Which fonts text is shaped and drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fonts {
    /// The bundled `Uncad Sans` only (a Noto Sans KR subset: Latin, Greek,
    /// the 2350 common Hangul syllables and the CAD symbols -- see
    /// `crates/uncad/fonts/README.md`). The same pixels and the same text
    /// boxes on every machine; a character outside the subset is drawn as
    /// a crossed `.notdef` box (the subset keeps the notdef outline) and
    /// counted as unshaped by the export -- see [`to_png`].
    #[default]
    Bundled,
    /// The bundled face first, then the host's installed fonts for the
    /// characters it lacks (Hanja, rare syllables). Slower to shape and
    /// host-dependent.
    BundledAndSystem,
}

/// The family name the bundled font is registered under.
pub const BUNDLED_FONT_FAMILY: &str = "Uncad Sans";

/// The bundled face's cap height as a fraction of its em: OS/2
/// `sCapHeight` 733 over `head.unitsPerEm` 1000 in
/// `fonts/UncadSans-Regular.otf` (read from the font's own tables; the
/// unit test below checks the value against the embedded bytes). CAD text
/// height is the height of the capitals, not the em, so the renderer draws
/// a text of height `h` at `font-size = h / BUNDLED_CAP_HEIGHT` and the
/// estimates in [`crate::text`] scale their per-character advance the same
/// way.
pub const BUNDLED_CAP_HEIGHT: f64 = 0.733;

/// The descender the renderer assumes below the baseline, as a fraction of
/// the em (a Latin `p`/`g` in the bundled face, not the deeper CJK
/// descender the `hhea` table carries).
pub const BUNDLED_DESCENDER: f64 = 0.2;

static UNCAD_SANS: &[u8] = include_bytes!("../fonts/UncadSans-Regular.otf");

pub(crate) fn parse_tree(svg_text: &str, fonts: Fonts) -> Result<usvg::Tree, PngError> {
    let options = usvg::Options {
        fontdb: font_database(fonts),
        // `<text>` without a font-family (0.2.0's SVGs) resolves to the
        // bundled face rather than usvg's "Times New Roman".
        font_family: BUNDLED_FONT_FAMILY.to_string(),
        ..Default::default()
    };
    usvg::Tree::from_str(svg_text, &options).map_err(PngError::InvalidSvg)
}

/// The font database for `fonts`, built once per process: the bundled
/// face (zero-copy from the embedded bytes) as the sans-serif and serif
/// generic families, plus the host's fonts when asked -- scanning those
/// cost 45-330 ms per call when it happened inside every `to_png`, which a
/// tiled export would repeat per tile.
pub(crate) fn font_database(fonts: Fonts) -> Arc<fontdb::Database> {
    static BUNDLED: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    static WITH_SYSTEM: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    let build = |system: bool| {
        let mut db = fontdb::Database::new();
        db.load_font_source(fontdb::Source::Binary(Arc::new(UNCAD_SANS)));
        db.set_sans_serif_family(BUNDLED_FONT_FAMILY);
        db.set_serif_family(BUNDLED_FONT_FAMILY);
        if system {
            db.load_system_fonts();
        }
        Arc::new(db)
    };
    match fonts {
        Fonts::Bundled => BUNDLED.get_or_init(|| build(false)).clone(),
        Fonts::BundledAndSystem => WITH_SYSTEM.get_or_init(|| build(true)).clone(),
    }
}

/// 8-bit RGB PNG of an opaque pixmap. tiny-skia stores premultiplied RGBA;
/// with every alpha at 255 (the white background guarantees it) the colour
/// bytes are the straight values, so they are copied out directly.
fn encode_rgb8(pixmap: &tiny_skia::Pixmap) -> Result<Vec<u8>, PngError> {
    let data = pixmap.data();
    let mut rgb = Vec::with_capacity(data.len() / 4 * 3);
    for px in data.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PngError::Encode(e.to_string()))?;
        writer
            .write_image_data(&rgb)
            .map_err(|e| PngError::Encode(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| PngError::Encode(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table of an OpenType font: its bytes, found through the table
    /// directory at the start of the file (a 12-byte header, then 16-byte
    /// records of tag, checksum, offset, length).
    fn otf_table<'a>(font: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..count)
            .map(|i| &font[12 + 16 * i..12 + 16 * (i + 1)])
            .find(|record| &record[..4] == tag)
            .map(|record| {
                let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
                let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
                &font[offset..offset + length]
            })
            .unwrap_or_else(|| panic!("the font has a {} table", String::from_utf8_lossy(tag)))
    }

    #[test]
    fn the_cap_height_constant_is_the_bundled_fonts_own() {
        // head.unitsPerEm sits at offset 18; OS/2 sCapHeight at offset 88
        // (version 2+; the field exists since version 2).
        let head = otf_table(UNCAD_SANS, b"head");
        let upem = f64::from(u16::from_be_bytes([head[18], head[19]]));
        let os2 = otf_table(UNCAD_SANS, b"OS/2");
        let version = u16::from_be_bytes([os2[0], os2[1]]);
        assert!(version >= 2, "OS/2 version {version} has no sCapHeight");
        let cap = f64::from(i16::from_be_bytes([os2[88], os2[89]]));
        assert_eq!(upem, 1000.0);
        assert_eq!(cap, 733.0);
        assert!((cap / upem - BUNDLED_CAP_HEIGHT).abs() < 1e-12);
    }

    /// Decodes just the IHDR chunk's width/height (bytes 16..24 of any
    /// PNG) rather than pulling in an image-decoding dependency purely for
    /// test assertions.
    fn png_dimensions(png: &[u8]) -> (u32, u32) {
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        (width, height)
    }

    /// Byte 25 of a PNG is the IHDR colour type: 2 = RGB, 6 = RGBA.
    fn png_color_type(png: &[u8]) -> u8 {
        png[25]
    }

    #[test]
    fn svg_to_png_produces_a_valid_png_sized_to_the_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"><rect width="10" height="20" fill="red"/></svg>"#;
        let png = svg_to_png(svg, 1.0).expect("svg_to_png should succeed");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "output should start with the PNG signature"
        );
        assert_eq!(png_dimensions(&png), (10, 20));
        assert_eq!(png_color_type(&png), 6, "the legacy path stays RGBA");
    }

    #[test]
    fn svg_to_png_scale_multiplies_the_viewbox_pixel_size() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"></svg>"#;
        let png = svg_to_png(svg, 2.5).expect("svg_to_png should succeed");
        assert_eq!(png_dimensions(&png), (25, 50));
    }

    #[test]
    fn svg_to_png_rejects_unparseable_svg() {
        let err = svg_to_png("not an svg document", 1.0).unwrap_err();
        assert!(matches!(err, PngError::InvalidSvg(_)));
    }

    /// Exercises the full `CadDatabase::to_png` -> `render` -> rasterize
    /// pipeline against a real DWG, not just the SVG -> PNG half tested above.
    /// The fixture comes from the submodule-tracked LibreDWG corpus rather than
    /// `samples/`, which is gitignored by design (see `samples/README.md`). The
    /// build itself does not need the submodule -- `libredwg-sys` compiles its
    /// own vendored copy -- so it is a precondition of `cargo test` only.
    #[test]
    fn to_png_renders_a_real_dwg_to_a_valid_png() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../lib/libredwg/test/test-data/2000/circle.dwg"
        );
        let db = crate::parse(path).expect("parse should succeed");
        let result = db
            .to_png(ToPngOptions::default())
            .expect("to_png should succeed");
        assert!(result.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let (width, height) = png_dimensions(&result.png);
        assert_eq!((width, height), (result.width, result.height));
        assert_eq!(
            width.max(height),
            1568,
            "the default fits the long edge to 1568 px"
        );
        assert_eq!(
            png_color_type(&result.png),
            2,
            "white background is written as RGB"
        );
        assert!((result.view_box.width * result.px_per_unit - f64::from(width)).abs() < 1.0);
    }
}
