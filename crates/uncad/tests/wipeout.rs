//! A WIPEOUT's clip boundary, taken from its image's pixel space to the
//! entity's local space.

use uncad::model::Ref;
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

#[test]
fn a_wipeouts_boundary_fills_its_image_from_the_insertion_point() {
    // The drawing's WIPEOUT 258: inserted at (2788.21..., -762.09...), one
    // pixel square of 1757.89... on each side, its clip vertices spanning the
    // pixel from -0.5 to 0.5 across and from -0.1375... to 0.5 down. So its
    // boundary starts at the insertion point -- not half a pixel before it,
    // where a mapping that ignores the pixel's origin would put it -- and
    // reaches one pixel across and 0.6375... of one up.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("the corpus drawing should parse");
    let wipeout = db
        .entities
        .iter()
        .chain(
            db.tables
                .block_records
                .values()
                .flat_map(|b| b.entities.iter()),
        )
        .find_map(|e| match e {
            Entity::Wipeout(w) if w.common.source_handle == Ref::Resolved("258".to_string()) => {
                Some(w)
            }
            _ => None,
        })
        .expect("the drawing's WIPEOUT 258");
    let (x0, y0, size) = (2788.211922856368, -762.094446381714, 1757.893931278507);
    let fold = |f: fn(f64, f64) -> f64, pick: fn(&uncad::model::Point2D) -> f64, init: f64| {
        wipeout.boundary.iter().map(pick).fold(init, f)
    };
    let min_x = fold(f64::min, |p| p.x, f64::INFINITY);
    let max_x = fold(f64::max, |p| p.x, f64::NEG_INFINITY);
    let min_y = fold(f64::min, |p| p.y, f64::INFINITY);
    let max_y = fold(f64::max, |p| p.y, f64::NEG_INFINITY);
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(
        close(min_x, x0) && close(max_x, x0 + size),
        "x {min_x}..{max_x}"
    );
    assert!(
        close(min_y, y0) && close(max_y, y0 + size * (0.5 + 0.1375228853435013)),
        "y {min_y}..{max_y}"
    );
    // The vertex written at pixel (0.5, -0.0083) -- the right edge, just
    // below the middle -- is at the right edge, just *above* the middle:
    // pixel rows run down the image.
    assert!(wipeout
        .boundary
        .iter()
        .any(|p| close(p.x, x0 + size) && close(p.y, y0 + size * (0.5 + 0.0082551342292443))));
}
