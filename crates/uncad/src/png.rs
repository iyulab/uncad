//! SVG -> PNG rasterization, layered on top of [`crate::svg::to_svg`]'s
//! output via resvg/usvg/tiny-skia (all pure Rust, no system library
//! dependency -- see [`crate::svg`]'s own doc comment for the CAD -> SVG
//! side; this module only handles the SVG -> PNG raster step).
//!
//! A single `resvg` dependency is used rather than separate `usvg` /
//! `tiny-skia` / `fontdb` crates: `resvg` re-exports both (`resvg::usvg`,
//! `resvg::tiny_skia`, and `usvg::fontdb`), which keeps the three in lock
//! step instead of risking a version mismatch across independently pinned
//! crates.

use crate::svg::{to_svg, ToSvgOptions};
use crate::CadDatabase;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToPngOptions {
    pub svg: ToSvgOptions,
    /// Multiplies the SVG's own viewBox-derived pixel size -- e.g. `2.0`
    /// renders at 2x resolution. Must be finite and > 0.
    pub scale: f32,
}

impl Default for ToPngOptions {
    fn default() -> Self {
        ToPngOptions {
            svg: ToSvgOptions::default(),
            scale: 1.0,
        }
    }
}

pub struct ToPngResult {
    pub png: Vec<u8>,
    pub unsupported_entity_types: Vec<String>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum PngError {
    /// usvg couldn't parse the intermediate SVG string -- since that string
    /// comes from this crate's own `to_svg`, this would indicate a bug
    /// there rather than bad input from a caller.
    InvalidSvg(usvg::Error),
    /// The requested pixel size (viewBox size * `scale`) rounds to zero in
    /// at least one dimension.
    EmptyCanvas,
    /// tiny-skia's PNG encoder failed. Stored as its `Display` text rather
    /// than the underlying `png::EncodingError` type itself, so this crate
    /// doesn't need its own direct dependency on the `png` crate (tiny-skia
    /// doesn't re-export it) just to name the error type.
    Encode(String),
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PngError::InvalidSvg(e) => write!(f, "SVG 파싱 실패: {e}"),
            PngError::EmptyCanvas => write!(f, "렌더링 크기가 0입니다 (scale 값을 확인하세요)"),
            PngError::Encode(e) => write!(f, "PNG 인코딩 실패: {e}"),
        }
    }
}
impl std::error::Error for PngError {}

/// Renders `db` straight to PNG bytes, via [`to_svg`] internally -- the
/// intermediate SVG text never touches disk.
///
/// Fonts come from the host's installed system fonts, loaded fresh on every
/// call (`fontdb::Database::load_system_fonts`, matching this crate having
/// no bundled font of its own). A host with no matching font installed
/// renders `<text>` entities (dimension/MTEXT labels) as blank rather than
/// erroring -- usvg treats an unresolved glyph as empty, not a parse
/// failure.
pub fn to_png(db: &CadDatabase, options: ToPngOptions) -> Result<ToPngResult, PngError> {
    let svg_result = to_svg(db, options.svg);
    let png = svg_to_png(&svg_result.svg, options.scale)?;
    Ok(ToPngResult {
        png,
        unsupported_entity_types: svg_result.unsupported_entity_types,
    })
}

/// Rasterizes an already-built SVG string to PNG bytes at `scale`x the
/// SVG's own viewBox-derived size. Split out from [`to_png`] so a caller
/// that already has an SVG string (e.g. from a separately cached
/// [`to_svg`] call) doesn't have to re-render the CAD geometry to get a
/// PNG out of it.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, PngError> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let opt = usvg::Options {
        fontdb: Arc::new(db),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &opt).map_err(PngError::InvalidSvg)?;

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

    #[test]
    fn svg_to_png_produces_a_valid_png_sized_to_the_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"><rect width="10" height="20" fill="red"/></svg>"#;
        let png = svg_to_png(svg, 1.0).expect("svg_to_png should succeed");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "output should start with the PNG signature"
        );
        assert_eq!(png_dimensions(&png), (10, 20));
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

    /// Exercises the full `CadDatabase::to_png` -> `to_svg` -> `svg_to_png`
    /// pipeline against a real DWG, rather than only the SVG -> PNG half
    /// tested above. Uses a file from the (git-submodule-tracked, always
    /// present) LibreDWG test-data corpus rather than `samples/`, which is
    /// gitignored and manual-spot-check-only by design -- see
    /// `samples/README.md` for why real-file regression tests don't hang
    /// off that directory. `libredwg-sys` itself already requires this
    /// submodule to build, so depending on one of its files here adds no
    /// new precondition.
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
        assert!(width > 0 && height > 0);
    }
}
