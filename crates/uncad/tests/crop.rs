//! The crop rule (docs/VLM_EXPORT_DESIGN.md, section 4 / P6) on real
//! files: `example_2018.dwg` carries an INSERT scaled 3256x that made the
//! 0.2.0 viewBox 3.4 million units wide (and a 36 TB allocation at
//! `--scale 1`); `example_2000.dwg` is the same drawing family, with the
//! same 3256x INSERT (handle 756) and header extents that include it.

use uncad::crop::{CropMode, CropSource, ExcludeReason, Rect};
use uncad::{ToPngOptions, ToSvgOptions};

const EXAMPLE_2018_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dwg"
);
const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

#[test]
fn the_3256x_insert_of_example_2018_is_a_scale_outlier() {
    let db = uncad::parse(EXAMPLE_2018_DWG).expect("corpus file must parse");
    let png = db
        .to_png(ToPngOptions::default())
        .expect("renders at the default fit");
    assert!(
        png.width <= 1568 && png.height <= 1568,
        "{}x{}",
        png.width,
        png.height
    );
    assert_eq!(
        png.width.max(png.height),
        1568,
        "the long edge fills the fit"
    );
    assert_eq!(png.width % 28, 0);
    assert_eq!(png.height % 28, 0);

    let crop = &png.crop;
    assert!(
        crop.excluded
            .iter()
            .any(|e| e.type_name == "INSERT" && e.reason == ExcludeReason::ScaleOutlier),
        "{:?}",
        crop.excluded
    );
    assert!(
        crop.rect.diagonal() < 1e5,
        "the overview is the drawing, not the outlier: {:?}",
        crop.rect
    );
    assert_ne!(crop.source, CropSource::Raw);

    let raw = db
        .to_svg(ToSvgOptions {
            crop: CropMode::Raw,
            ..Default::default()
        })
        .crop;
    assert_eq!(raw.source, CropSource::Raw);
    assert!(raw.rect.diagonal() > 1e6, "{:?}", raw.rect);
    assert!(raw.excluded.is_empty());
}

#[test]
fn pixels_and_units_stay_in_exact_proportion() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let png = db.to_png(ToPngOptions::default()).expect("renders");
    let s = png.px_per_unit;
    assert!(
        (png.view_box.width * s - f64::from(png.width)).abs() < 1e-6,
        "{} x {s} vs {}",
        png.view_box.width,
        png.width
    );
    assert!((png.view_box.height * s - f64::from(png.height)).abs() < 1e-6);
    assert_eq!(png.width % 28, 0);
    assert_eq!(png.height % 28, 0);

    // The report's rectangle is the viewBox's world rectangle.
    let (x0, y0, x1, y1) = png.view_box.world_bounds();
    let rect = png.crop.rect;
    for (a, b) in [
        (x0, rect.min_x),
        (y0, rect.min_y),
        (x1, rect.max_x),
        (y1, rect.max_y),
    ] {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }
    // Corners map to the image's corners, and the affine round-trips.
    let (px, py) = png.view_box.world_to_px(rect.max_x, rect.min_y, s);
    assert!((px - f64::from(png.width)).abs() < 1e-6 && (py - f64::from(png.height)).abs() < 1e-6);
    let (wx, wy) = png.view_box.px_to_world(100.5, 200.25, s);
    let (bx, by) = png.view_box.world_to_px(wx, wy, s);
    assert!((bx - 100.5).abs() < 1e-9 && (by - 200.25).abs() < 1e-9);

    // Padding: at least 2 % of the longer side and at least 24 px.
    let content = png.crop.content.expect("the drawing has content");
    assert!(png.crop.padding_units >= 0.02 * content.longer_side() - 1e-9);
    assert!(png.crop.padding_units * s >= 24.0 - 1e-6);

    // AutoCAD's extents are sane but include the 3256x INSERT, so they are
    // no candidate (over 4x the content); the INSERT is the one exclusion.
    let header = png.crop.header_extents.expect("$EXTMIN/$EXTMAX are sane");
    assert!(header.intersects(&content), "{header:?} vs {content:?}");
    assert!(header.area() > 4.0 * content.area());
    assert_eq!(png.crop.source, CropSource::Content);
    // The INSERT by its size, and the ATTRIB it drags along by distance.
    let mut excluded: Vec<(&str, &str, ExcludeReason)> = png
        .crop
        .excluded
        .iter()
        .map(|e| (e.type_name.as_str(), e.handle.as_str(), e.reason))
        .collect();
    excluded.sort_by_key(|(t, h, _)| (*t, *h));
    assert_eq!(
        excluded,
        [
            ("ATTRIB", "757", ExcludeReason::FarOutlier),
            ("INSERT", "756", ExcludeReason::ScaleOutlier),
        ]
    );

    // Without a lattice the long edge is exactly the fit.
    let plain = db
        .to_png(ToPngOptions {
            lattice: 0,
            ..Default::default()
        })
        .expect("renders");
    assert_eq!(plain.width.max(plain.height), 1568);
}

#[test]
fn header_and_fixed_crops_are_honoured() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let by_header = db
        .to_svg(ToSvgOptions {
            crop: CropMode::Header,
            padding: Some(0.0),
            ..Default::default()
        })
        .crop;
    assert_eq!(by_header.source, CropSource::Header);
    assert_eq!(Some(by_header.rect), by_header.header_extents);

    let window = Rect::new(0.0, 0.0, 100.0, 100.0);
    let fixed = db.to_svg(ToSvgOptions {
        crop: CropMode::Fixed(window),
        padding: Some(0.0),
        ..Default::default()
    });
    assert_eq!(fixed.crop.source, CropSource::Fixed);
    assert_eq!(fixed.crop.rect, window);
    let (x0, y0, x1, y1) = fixed.view_box.world_bounds();
    assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 100.0, 100.0));
    assert!(
        fixed.svg.contains("viewBox=\"0 -100 100 100\""),
        "{}",
        &fixed.svg[..200]
    );
    assert!(fixed
        .crop
        .excluded
        .iter()
        .all(|e| e.reason == ExcludeReason::OutsideCrop));
}

#[test]
fn an_empty_drawing_gets_a_ten_unit_canvas() {
    let db = uncad::CadDatabase::new(Vec::new(), uncad::Tables::default());
    let svg = db.to_svg(ToSvgOptions::default());
    assert_eq!(svg.crop.source, CropSource::Empty);
    assert_eq!(svg.crop.content, None);
    assert_eq!(svg.crop.rect, Rect::new(-0.2, -0.2, 10.2, 10.2));
    assert!((svg.view_box.width - 10.4).abs() < 1e-9);
    let png = db
        .to_png(ToPngOptions::default())
        .expect("an empty canvas still renders");
    assert!(png.width > 0 && png.height > 0);
}
