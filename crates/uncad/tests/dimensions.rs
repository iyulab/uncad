//! DIMENSION values (docs/VLM_EXPORT_DESIGN.md, P3): the stored measurement,
//! the one recomputed from the definition points, the label the drawing
//! shows and where it came from. Two sources: the `dimlfac12_r2000.dxf`
//! fixture (this project's own, ground truth in tests/fixtures/README.md)
//! and the corpus's `example_2000.dwg`, whose labels were written by
//! AutoCAD and are therefore the reference for the values.

use uncad::model::{DimensionGeometry, DisplaySource};
use uncad::Entity;

const DIMLFAC12: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dimlfac12_r2000.dxf"
);
const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

fn dimensions(db: &uncad::CadDatabase) -> Vec<&uncad::model::DimensionEntity> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Dimension(d) => Some(d),
            _ => None,
        })
        .collect()
}

#[test]
fn the_fixture_dimension_carries_its_value_points_and_cached_label() {
    let db = uncad::parse(DIMLFAC12).expect("fixture must parse");
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 1);
    let dim = dims[0];

    match &dim.geometry {
        DimensionGeometry::Linear {
            xline1,
            xline2,
            rotation,
        } => {
            assert_eq!((xline1.x, xline1.y), (0.0, 0.0));
            assert_eq!((xline2.x, xline2.y), (10.0, 0.0));
            assert_eq!(*rotation, 0.0);
        }
        other => panic!("expected a LINEAR dimension, got {other:?}"),
    }
    assert_eq!(dim.measurement, Some(10.0), "DXF 42 as written");
    assert_eq!(dim.measurement_from_points, Some(10.0));
    assert_eq!(
        (dim.definition_point.x, dim.definition_point.y),
        (10.0, 5.0)
    );
    assert_eq!((dim.text_midpoint.x, dim.text_midpoint.y), (5.0, 6.0));
    assert_eq!(dim.dimstyle, "STANDARD");
    assert_eq!(dim.user_text, "");
    // The label AutoCAD would show: the TEXT "120" cached in *D1 (12 x 10
    // under $DIMLFAC 12).
    assert_eq!(dim.display_text, "120");
    assert_eq!(dim.display_source, DisplaySource::CachedBlock);
    // The style's DIMLFAC (144 on STANDARD), which the fixture sets to the
    // same 12 as the header.
    assert_eq!(dim.dimlfac, 12.0);
    assert_eq!(db.tables.dimstyles["STANDARD"].dimlfac, 12.0);
}

#[test]
fn autocad_written_dimensions_agree_with_their_definition_points_and_labels() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let dims = dimensions(&db);
    assert!(
        dims.len() >= 5,
        "example_2000.dwg has dimensions: {}",
        dims.len()
    );

    for dim in &dims {
        let stored = dim.measurement.expect("R2000 files store act_measurement");
        let recomputed = dim
            .measurement_from_points
            .expect("every kind here has a geometry");
        assert!(
            (stored - recomputed).abs() <= 1e-6 * stored.abs().max(1.0),
            "{} {:?}: stored {stored} vs from points {recomputed}",
            dim.common.handle,
            dim.geometry
        );
        assert_ne!(
            dim.display_source,
            DisplaySource::None,
            "{} has no label at all",
            dim.common.handle
        );
    }

    // An ALIGNED dimension whose cached label AutoCAD wrote with a decimal
    // comma (DIMDSEP) and two decimals.
    let aligned = dims
        .iter()
        .find(|d| matches!(d.geometry, DimensionGeometry::Aligned { .. }))
        .expect("an ALIGNED dimension");
    let value = aligned.measurement.unwrap();
    assert!((value - 1504.6795).abs() < 1e-3, "{value}");
    assert_eq!(aligned.display_text, "1504,68");
    assert_eq!(aligned.display_source, DisplaySource::CachedBlock);

    // A two-line angular dimension: 108 degrees, stored in radians, labelled
    // with a degree sign.
    let angular = dims
        .iter()
        .find(|d| matches!(d.geometry, DimensionGeometry::Angular2Line { .. }))
        .expect("an ANGULAR_2LINE dimension");
    let degrees = angular.measurement.unwrap();
    assert!((degrees - 108.0).abs() < 1e-3, "{degrees}");
    assert_eq!(angular.display_text, "108\u{00B0}");
}

#[test]
fn dimension_fields_survive_the_json_round_trip() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializes");
    assert!(json.contains("\"kind\":\"ALIGNED\""), "{}", &json[..200]);
    assert!(json.contains("\"display_source\":\"cached_block\""));
    let back: uncad::CadDatabase = serde_json::from_str(&json).expect("deserializes");
    let (a, b) = (dimensions(&db), dimensions(&back));
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.display_text, y.display_text);
        assert_eq!(x.display_source, y.display_source);
        assert_eq!(x.dimstyle, y.dimstyle);
    }
}

#[test]
fn a_tolerance_takes_its_text_height_from_its_dimension_style() {
    // An R2000+ TOLERANCE stores no height of its own (LibreDWG decodes
    // `height` for R13/R14 only), so the feature control frame 4F1 in
    // example_2000.dwg draws at its style's DIMTXT: ISO-25, 2.5 (which the
    // header's DIMTXT also is). It used to come out as 0 and render as a
    // `font-size="0"` text usvg drops.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tolerance = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Tolerance(t) if t.common.handle == "4F1" => Some(t),
            _ => None,
        })
        .expect("TOLERANCE 4F1");
    assert_eq!(tolerance.dimstyle, "ISO-25");
    let style = &db.tables.dimstyles["ISO-25"];
    assert_eq!(style.dimtxt, 2.5);
    assert_eq!(tolerance.text_height, style.dimtxt);
    assert!(tolerance.text_height > 0.0);
    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    assert!(!svg.contains("font-size=\"0\""), "a zero-height text");
    assert!(
        svg.contains("id=\"4F1\"") && svg.contains("font-size=\"2.5\""),
        "4F1 at its style's height"
    );
}
