//! The PNG contract for image consumers (docs/VLM_EXPORT_DESIGN.md,
//! "Rendering changes"): the image is sized in pixels, the background is
//! opaque white, strokes are a fixed pixel width, oversized requests are
//! refused, and the result says how its pixels map to the drawing. The
//! fixture is the corpus's one-entity `circle.dwg` (a 37-unit viewBox), so
//! every expectation follows from the option, not from the drawing.

use std::io::Cursor;

use uncad::{Background, PngError, PngSize, ToPngOptions, ViewBox};

const CIRCLE_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/circle.dwg"
);

struct Decoded {
    width: u32,
    height: u32,
    color_type: png::ColorType,
    /// Interleaved samples, `color_type.samples()` per pixel.
    data: Vec<u8>,
}

impl Decoded {
    fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let n = self.color_type.samples();
        let start = ((y * self.width + x) as usize) * n;
        &self.data[start..start + n]
    }

    fn count_pixels(&self, pred: impl Fn(&[u8]) -> bool) -> usize {
        let n = self.color_type.samples();
        self.data.chunks_exact(n).filter(|p| pred(p)).count()
    }
}

fn decode(png_bytes: &[u8]) -> Decoded {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("valid PNG");
    let info = reader.info().clone();
    let mut data = vec![0u8; (info.width * info.height * 4) as usize];
    let frame = reader.next_frame(&mut data).expect("decodable frame");
    data.truncate(frame.buffer_size());
    Decoded {
        width: frame.width,
        height: frame.height,
        color_type: frame.color_type,
        data,
    }
}

fn circle() -> uncad::CadDatabase {
    uncad::parse(CIRCLE_DWG).expect("corpus file must parse")
}

/// Sizing rules without the 28 px lattice, so each promises an exact count.
fn options(size: PngSize) -> ToPngOptions {
    ToPngOptions {
        size,
        lattice: 0,
        ..Default::default()
    }
}

#[test]
fn the_default_fits_the_long_edge_to_1568_pixels_on_white_rgb() {
    let result = circle().to_png(ToPngOptions::default()).expect("renders");
    assert_eq!(result.width.max(result.height), 1568);
    let image = decode(&result.png);
    assert_eq!((image.width, image.height), (result.width, result.height));
    assert_eq!(image.color_type, png::ColorType::Rgb);
    assert_eq!(image.pixel(0, 0), [255, 255, 255], "the corner is white");
    assert!(
        image.count_pixels(|p| p != [255, 255, 255]) > 1000,
        "the circle must have been drawn"
    );
}

#[test]
fn every_size_rule_produces_the_pixel_count_it_promises() {
    let db = circle();
    let fit = db
        .to_png(options(PngSize::FitLongEdge(400)))
        .expect("renders");
    assert_eq!(fit.width.max(fit.height), 400);
    // With the default lattice the fit rounds down to a multiple of 28.
    let snapped = db
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(400),
            ..Default::default()
        })
        .expect("renders");
    assert_eq!(snapped.width.max(snapped.height), 392);
    assert_eq!(snapped.width % 28, 0);
    assert_eq!(snapped.height % 28, 0);

    let one_to_one = db.to_png(options(PngSize::Scale(1.0))).expect("renders");
    assert_eq!(
        (one_to_one.width, one_to_one.height),
        (
            one_to_one.view_box.width.round() as u32,
            one_to_one.view_box.height.round() as u32
        ),
        "Scale(1.0) is one pixel per drawing unit"
    );

    let ppu = db
        .to_png(options(PngSize::PxPerUnit(10.0)))
        .expect("renders");
    assert_eq!(ppu.width, (ppu.view_box.width * 10.0).round() as u32);
    assert!((ppu.px_per_unit - 10.0).abs() < 1e-12);
}

#[test]
fn oversized_requests_are_refused_before_allocating() {
    let db = circle();
    let err = db
        .to_png(options(PngSize::FitLongEdge(9000)))
        .expect_err("9000 px exceeds the default 8000 cap");
    assert!(
        matches!(err, PngError::TooLarge { max_edge: 8000, .. }),
        "{err:?}"
    );

    let err = db
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(200),
            max_edge: 100,
            ..Default::default()
        })
        .expect_err("a lowered cap applies too");
    assert!(matches!(err, PngError::TooLarge { .. }), "{err:?}");

    let err = db
        .to_png(options(PngSize::Scale(f64::NAN)))
        .expect_err("NaN is not a size");
    assert!(matches!(err, PngError::InvalidSize), "{err:?}");
}

#[test]
fn a_transparent_background_keeps_the_rgba_output() {
    let result = circle()
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(200),
            background: Background::Transparent,
            ..Default::default()
        })
        .expect("renders");
    let image = decode(&result.png);
    assert_eq!(image.color_type, png::ColorType::Rgba);
    assert_eq!(image.pixel(0, 0)[3], 0, "the corner is transparent");
}

#[test]
fn stroke_width_is_in_output_pixels() {
    let db = circle();
    let thin = db
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(400),
            stroke_px: Some(1.0),
            ..Default::default()
        })
        .expect("renders");
    let thick = db
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(400),
            stroke_px: Some(6.0),
            ..Default::default()
        })
        .expect("renders");
    let inked = |png_bytes: &[u8]| decode(png_bytes).count_pixels(|p| p != [255, 255, 255]);
    let (thin_ink, thick_ink) = (inked(&thin.png), inked(&thick.png));
    // A circle of radius ~185 px: a 1 px ring is ~1200 px, a 6 px ring ~7000.
    assert!(thin_ink > 500, "thin: {thin_ink}");
    assert!(
        thick_ink > thin_ink * 3,
        "6 px strokes must ink far more than 1 px: {thin_ink} vs {thick_ink}"
    );
    // The same 1 px request at twice the resolution inks about twice as many
    // pixels (the ring is twice as long), not four times (it did not get
    // twice as wide too).
    let thin_big = db
        .to_png(ToPngOptions {
            size: PngSize::FitLongEdge(800),
            stroke_px: Some(1.0),
            ..Default::default()
        })
        .expect("renders");
    let big_ink = inked(&thin_big.png);
    assert!(
        big_ink > thin_ink && big_ink < thin_ink * 3,
        "1 px at 800 px vs 400 px: {big_ink} vs {thin_ink}"
    );
}

#[test]
fn the_result_maps_pixels_back_to_the_drawing() {
    let result = circle().to_png(ToPngOptions::default()).expect("renders");
    let vb: ViewBox = result.view_box;
    let (min_x, min_y, max_x, max_y) = vb.world_bounds();
    assert!(max_x > min_x && max_y > min_y);
    // The viewBox's top-left corner is pixel (0, 0); its bottom-right is
    // (width, height) in pixels, within rounding.
    let (px, py) = vb.world_to_px(min_x, max_y, result.px_per_unit);
    assert!(px.abs() < 1e-9 && py.abs() < 1e-9, "{px} {py}");
    let (px, py) = vb.world_to_px(max_x, min_y, result.px_per_unit);
    assert!((px - f64::from(result.width)).abs() < 1.0, "{px}");
    assert!((py - f64::from(result.height)).abs() < 1.0, "{py}");
    // And back.
    let (wx, wy) = vb.px_to_world(px, py, result.px_per_unit);
    assert!((wx - max_x).abs() < 1e-9 && (wy - min_y).abs() < 1e-9);
    // The SVG's viewBox is its own (2 % padding, no lattice) but shows the
    // same content, and its attribute is the reported one.
    let svg = circle().to_svg(Default::default());
    assert_eq!(svg.crop.content, result.crop.content);
    let vb = svg.view_box;
    assert!(svg.svg.contains(&format!(
        "viewBox=\"{} {} {} {}\"",
        vb.x, vb.y, vb.width, vb.height
    )));
}

#[test]
fn hangul_renders_with_the_bundled_font() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cp949_r2000.dxf"
    );
    let db = uncad::parse(fixture).expect("fixture must parse");
    // The SVG names the bundled family and ids its texts.
    let svg = db.to_svg(Default::default()).svg;
    assert!(svg.contains("font-family=\"Uncad Sans\""), "{svg}");
    assert!(svg.contains("id=\"23\""), "{svg}");
    assert!(svg.contains(">\u{b3c4}\u{ba74}<"), "{svg}");

    let bundled = db.to_png(ToPngOptions::default()).expect("renders");
    let image = decode(&bundled.png);
    let ink = image.count_pixels(|p| p != [255, 255, 255]);
    // Five texts and a few padding lines at 1568 px: thousands of dark pixels.
    assert!(
        ink > 2000,
        "only {ink} dark pixels: the text did not render"
    );

    let with_system = db
        .to_png(ToPngOptions {
            fonts: uncad::Fonts::BundledAndSystem,
            ..Default::default()
        })
        .expect("renders");
    let ink_system = decode(&with_system.png).count_pixels(|p| p != [255, 255, 255]);
    // Every character is in the bundled face, so the host's fonts add nothing.
    assert_eq!(ink, ink_system);
}
