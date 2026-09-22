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
}

impl Default for ToPngOptions {
    fn default() -> Self {
        ToPngOptions {
            svg: ToSvgOptions::default(),
            size: PngSize::default(),
            background: Background::default(),
            stroke_px: Some(1.25),
            max_edge: 8000,
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
    /// [`px_per_unit`](Self::px_per_unit).
    pub view_box: ViewBox,
    /// Pixels per drawing unit: `pixel = (world - viewBox origin) * this`.
    pub px_per_unit: f64,
    pub unsupported_types: Vec<String>,
    /// Entities hidden by the drawing -- see [`crate::ToSvgResult::hidden`].
    pub hidden: usize,
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
        }
    }
}
impl std::error::Error for PngError {}

/// Renders `db` straight to PNG bytes -- the intermediate SVG text never
/// touches disk.
///
/// Fonts come from the host's installed system fonts, loaded once per
/// process (`fontdb::Database::load_system_fonts`; this crate bundles no
/// font of its own yet). A host with no matching font renders `<text>`
/// entities (dimension/MTEXT labels) as blank rather than erroring -- usvg
/// treats an unresolved glyph as empty, not a parse failure.
pub fn to_png(db: &CadDatabase, options: ToPngOptions) -> Result<ToPngResult, PngError> {
    let rendered = svg::render(db, options.svg);
    let view_box = rendered.view_box;

    let px_per_unit = match options.size {
        PngSize::FitLongEdge(px) => f64::from(px) / view_box.width.max(view_box.height),
        PngSize::PxPerUnit(ppu) => ppu,
        PngSize::Scale(scale) => scale,
    };
    if !px_per_unit.is_finite() || px_per_unit <= 0.0 {
        return Err(PngError::InvalidSize);
    }
    let (width, height) = pixel_size(&view_box, px_per_unit);
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
    let svg_text = svg::assemble(&rendered, stroke_width);

    let png = rasterize(&svg_text, px_per_unit, width, height, options.background)?;
    Ok(ToPngResult {
        png,
        width,
        height,
        view_box,
        px_per_unit,
        unsupported_types: rendered.unsupported_types(),
        hidden: rendered.hidden,
    })
}

/// Rasterizes an already-built SVG string to RGBA PNG bytes at `scale`x the
/// SVG's own viewBox-derived size, on a transparent background: 0.2.0's
/// behaviour, kept for callers that hold an SVG string from a separately
/// cached [`crate::CadDatabase::to_svg`] call. [`to_png`] is the way to get
/// a sized, opaque image with a known pixel scale.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, PngError> {
    let tree = parse_tree(svg)?;
    let size = tree.size();
    let width = (size.width() * scale).round() as u32;
    let height = (size.height() * scale).round() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::EmptyCanvas)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| PngError::Encode(e.to_string()))
}

/// The pixel size a viewBox gets at `px_per_unit`, rounded to whole pixels.
fn pixel_size(view_box: &ViewBox, px_per_unit: f64) -> (u32, u32) {
    let to_px = |units: f64| -> u32 {
        let px = (units * px_per_unit).round();
        if px.is_finite() && px >= 0.0 {
            px.min(f64::from(u32::MAX)) as u32
        } else {
            0
        }
    };
    (to_px(view_box.width), to_px(view_box.height))
}

fn rasterize(
    svg_text: &str,
    px_per_unit: f64,
    width: u32,
    height: u32,
    background: Background,
) -> Result<Vec<u8>, PngError> {
    let tree = parse_tree(svg_text)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::EmptyCanvas)?;
    if background == Background::White {
        pixmap.fill(tiny_skia::Color::WHITE);
    }
    let scale = px_per_unit as f32;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    match background {
        Background::White => encode_rgb8(&pixmap),
        Background::Transparent => pixmap
            .encode_png()
            .map_err(|e| PngError::Encode(e.to_string())),
    }
}

fn parse_tree(svg_text: &str) -> Result<usvg::Tree, PngError> {
    let options = usvg::Options {
        fontdb: font_database(),
        ..Default::default()
    };
    usvg::Tree::from_str(svg_text, &options).map_err(PngError::InvalidSvg)
}

/// The system font database, loaded once per process: scanning the
/// installed fonts cost 45-330 ms per call when it happened inside every
/// `to_png`, which a tiled export would repeat per tile.
fn font_database() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
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

    #[test]
    fn pixel_size_rounds_and_never_goes_negative() {
        let vb = ViewBox {
            x: 0.0,
            y: 0.0,
            width: 37.06,
            height: 37.06,
        };
        assert_eq!(pixel_size(&vb, 1.0), (37, 37));
        assert_eq!(pixel_size(&vb, 1568.0 / 37.06), (1568, 1568));
        assert_eq!(pixel_size(&vb, 0.001), (0, 0));
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
