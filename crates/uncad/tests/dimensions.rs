//! DIMENSION values (docs/VLM_EXPORT_DESIGN.md, P3): the stored measurement,
//! the one recomputed from the definition points, the label the drawing
//! shows and where it came from. Two sources: the `dimlfac12_r2000.dxf`
//! fixture (this project's own, ground truth in tests/fixtures/README.md)
//! and the corpus's `example_2000.dwg`, whose labels were written by
//! AutoCAD and are therefore the reference for the values. The
//! `angular_ordinate_r2000.dxf` fixture and the corpus's `example_2000.dxf`
//! (the same drawing as the DWG) cover the two dimension kinds whose
//! definition points LibreDWG's DXF reader lays out differently from its
//! DWG decoder. The three kinds `example_2000` has none of -- RADIUS,
//! DIAMETER and ANGULAR_3POINT -- come from the corpus's `2000/TS1.dwg`
//! (AutoCAD-written, so its cached labels are the reference) and, since
//! every TS1 DXF fails LibreDWG's reader, from this project's
//! `radial_r2000.dxf` for the DXF side.

use uncad::model::{DimensionGeometry, DisplaySource, Point3D};
use uncad::Entity;

const DIMLFAC12: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dimlfac12_r2000.dxf"
);
const ANGULAR_ORDINATE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/angular_ordinate_r2000.dxf"
);
const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);
const RADIAL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/radial_r2000.dxf"
);
const TS1_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/TS1.dwg"
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

/// Every point a geometry holds, in a fixed order, so two geometries can be
/// compared numerically.
fn geometry_points(g: &DimensionGeometry) -> Vec<Point3D> {
    match *g {
        DimensionGeometry::Linear { xline1, xline2, .. }
        | DimensionGeometry::Aligned { xline1, xline2 } => vec![xline1, xline2],
        DimensionGeometry::Angular3Point {
            center,
            xline1,
            xline2,
        }
        | DimensionGeometry::Arc {
            center,
            xline1,
            xline2,
        } => vec![center, xline1, xline2],
        DimensionGeometry::Angular2Line {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
        } => vec![line1_start, line1_end, line2_start, line2_end],
        DimensionGeometry::Radius {
            center,
            chord_point,
            ..
        } => vec![center, chord_point],
        DimensionGeometry::Diameter {
            chord_start,
            chord_end,
            ..
        } => vec![chord_start, chord_end],
        DimensionGeometry::Ordinate {
            feature,
            leader_end,
            ..
        } => vec![feature, leader_end],
        _ => Vec::new(),
    }
}

fn same_point(a: Point3D, b: Point3D) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
}

fn p3(x: f64, y: f64) -> Point3D {
    Point3D { x, y, z: 0.0 }
}

#[test]
fn the_dwg_and_dxf_readers_agree_on_every_dimension() {
    // example_2000.dxf is AutoCAD's DXF of example_2000.dwg, so every
    // dimension must come out the same whichever reader LibreDWG used --
    // in particular the 2-line angular (43B, 108 degrees) and the X-type
    // ordinate (430, 4630.52), whose definition points the DXF reader
    // stores in different fields from the DWG decoder.
    let dwg = uncad::parse(EXAMPLE_2000_DWG).expect("corpus DWG must parse");
    let dxf = uncad::parse(EXAMPLE_2000_DXF).expect("corpus DXF must parse");
    let (from_dwg, from_dxf) = (dimensions(&dwg), dimensions(&dxf));
    assert_eq!(from_dwg.len(), from_dxf.len());
    assert!(from_dwg.len() >= 10, "{}", from_dwg.len());
    for a in &from_dwg {
        let b = from_dxf
            .iter()
            .find(|d| d.common.handle == a.common.handle)
            .unwrap_or_else(|| panic!("{} missing from the DXF", a.common.handle));
        assert_eq!(
            std::mem::discriminant(&a.geometry),
            std::mem::discriminant(&b.geometry),
            "{}: {:?} vs {:?}",
            a.common.handle,
            a.geometry,
            b.geometry
        );
        let (pa, pb) = (geometry_points(&a.geometry), geometry_points(&b.geometry));
        assert!(
            pa.iter().zip(&pb).all(|(x, y)| same_point(*x, *y)),
            "{}: {:?} vs {:?}",
            a.common.handle,
            a.geometry,
            b.geometry
        );
        if let (
            DimensionGeometry::Ordinate { x_datum: xa, .. },
            DimensionGeometry::Ordinate { x_datum: xb, .. },
        ) = (&a.geometry, &b.geometry)
        {
            assert_eq!(xa, xb, "{}", a.common.handle);
        }
        assert!(
            same_point(a.definition_point, b.definition_point),
            "{}: {:?} vs {:?}",
            a.common.handle,
            a.definition_point,
            b.definition_point
        );
        let (ma, mb) = (a.measurement.unwrap(), b.measurement.unwrap());
        assert!(
            (ma - mb).abs() < 1e-6 * ma.abs().max(1.0),
            "{}",
            a.common.handle
        );
        let (fa, fb) = (
            a.measurement_from_points.unwrap(),
            b.measurement_from_points.unwrap(),
        );
        assert!(
            (fa - fb).abs() < 1e-6 * fa.abs().max(1.0),
            "{}: from points {fa} (dwg) vs {fb} (dxf)",
            a.common.handle
        );
        // And both agree with what AutoCAD measured.
        assert!(
            (fb - mb).abs() < 1e-6 * mb.abs().max(1.0),
            "{}: dxf from points {fb} vs stored {mb}",
            a.common.handle
        );
    }
    // The pair covers both of the kinds that differ between the readers.
    let two_line = from_dxf
        .iter()
        .find(|d| d.common.handle == "43B")
        .expect("the 2-line angular dimension 43B");
    assert!(matches!(
        two_line.geometry,
        DimensionGeometry::Angular2Line { .. }
    ));
    let degrees = two_line.measurement_from_points.unwrap();
    assert!((degrees - 108.0).abs() < 1e-6, "{degrees}");
    let ordinate = from_dxf
        .iter()
        .find(|d| d.common.handle == "430")
        .expect("the ordinate dimension 430");
    assert!(matches!(
        ordinate.geometry,
        DimensionGeometry::Ordinate { x_datum: true, .. }
    ));
    let x = ordinate.measurement_from_points.unwrap();
    assert!((x - 4630.519359).abs() < 1e-5, "{x}");
}

#[test]
fn the_angular_ordinate_fixture_reads_its_definition_points() {
    // Derived by hand (tests/fixtures/README.md): line 1 (0,0)-(10,0),
    // line 2 (0,0)-(5, 8.660254) with the arc point 30 degrees along a
    // radius of 5, so the dimensioned sector is 60 degrees (150 if the
    // arc point and the line end were swapped); the ordinates share the
    // datum (100, 200) and feature (130, 250): 30 along x, 50 along y.
    let db = uncad::parse(ANGULAR_ORDINATE).expect("fixture must parse");
    assert_eq!(db.header.format, "dxf");
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 3);

    let angular = dims
        .iter()
        .find(|d| matches!(d.geometry, DimensionGeometry::Angular2Line { .. }))
        .expect("an ANGULAR_2LINE dimension");
    let DimensionGeometry::Angular2Line {
        line1_start,
        line1_end,
        line2_start,
        line2_end,
    } = angular.geometry
    else {
        unreachable!()
    };
    assert!(same_point(line1_start, p3(0.0, 0.0)));
    assert!(same_point(line1_end, p3(10.0, 0.0)));
    assert!(same_point(line2_start, p3(0.0, 0.0)));
    assert!(
        same_point(line2_end, p3(5.0, 8.660254037844386)),
        "{line2_end:?}"
    );
    assert!(
        same_point(angular.definition_point, p3(4.330127018922194, 2.5)),
        "{:?}",
        angular.definition_point
    );
    let degrees = angular.measurement_from_points.unwrap();
    assert!((degrees - 60.0).abs() < 1e-9, "{degrees}");
    assert!((angular.measurement.unwrap() - 60.0).abs() < 1e-9);
    assert_eq!(angular.display_text, "60\u{00B0}");
    assert_eq!(angular.display_source, DisplaySource::Formatted);

    let ordinates: Vec<_> = dims
        .iter()
        .filter(|d| matches!(d.geometry, DimensionGeometry::Ordinate { .. }))
        .collect();
    assert_eq!(ordinates.len(), 2);
    for (d, want_x, want) in [(ordinates[0], true, 30.0), (ordinates[1], false, 50.0)] {
        let DimensionGeometry::Ordinate {
            feature, x_datum, ..
        } = d.geometry
        else {
            unreachable!()
        };
        assert_eq!(x_datum, want_x, "{}", d.common.handle);
        assert!(same_point(feature, p3(130.0, 250.0)));
        assert!(same_point(d.definition_point, p3(100.0, 200.0)));
        assert_eq!(d.measurement_from_points, Some(want), "{}", d.common.handle);
        assert_eq!(d.measurement, Some(want));
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
    // The renderer draws capitals at the CAD height, so the em size is the
    // height over the bundled face's cap-height ratio.
    let start = svg.find("id=\"4F1\"").expect("4F1 is drawn");
    let element = &svg[start..start + 400];
    let font_size = element
        .split("font-size=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|v| v.parse::<f64>().ok())
        .expect("4F1 has a font-size");
    let expected = 2.5 / uncad::png::BUNDLED_CAP_HEIGHT;
    assert!(
        (font_size - expected).abs() < 1e-9,
        "4F1 at its style's height: font-size {font_size} vs {expected}"
    );
}

/// The one dimension of `kind` in `dims`.
fn of_kind<'a>(
    dims: &'a [&'a uncad::model::DimensionEntity],
    kind: &str,
) -> &'a uncad::model::DimensionEntity {
    let mut found = dims.iter().filter(|d| {
        matches!(
            (&d.geometry, kind),
            (DimensionGeometry::Radius { .. }, "RADIUS")
                | (DimensionGeometry::Diameter { .. }, "DIAMETER")
                | (DimensionGeometry::Angular3Point { .. }, "ANGULAR_3POINT")
        )
    });
    let dim = found.next().unwrap_or_else(|| panic!("a {kind} dimension"));
    assert!(found.next().is_none(), "one {kind} dimension only");
    dim
}

#[test]
fn the_radial_and_three_point_kinds_read_from_an_autocad_dwg() {
    // 2000/TS1.dwg is a sampler AutoCAD wrote in 2011 (its TS1.txt lists
    // "11. 3 point angular dimension", "13. Diametric Dimension" and
    // "14. Radial Dimension"); example_2000 has none of the three. The
    // labels cached in the *D blocks are AutoCAD's own, at the Standard
    // style's DIMDEC 4, so they -- not this crate's arithmetic -- are what
    // the recomputed values are checked against.
    let db = uncad::parse(TS1_2000_DWG).expect("corpus file must parse");
    let dims = dimensions(&db);

    let radius = of_kind(&dims, "RADIUS");
    let DimensionGeometry::Radius {
        center,
        chord_point,
        ..
    } = radius.geometry
    else {
        unreachable!()
    };
    // The radius is the distance from the centre to the point on the arc.
    // Were the chord point mapped to the definition point (the centre
    // itself), it would be 0.
    let by_hand = (chord_point.x - center.x).hypot(chord_point.y - center.y);
    assert!(by_hand > 1.0, "the two points are distinct: {by_hand}");
    assert!(
        same_point(radius.definition_point, center),
        "10 is the centre"
    );
    assert_eq!(radius.display_text, "R1.1897");
    for got in [
        radius.measurement.unwrap(),
        radius.measurement_from_points.unwrap(),
    ] {
        assert!((got - by_hand).abs() < 1e-9, "{got} vs {by_hand}");
        assert!((got - 1.1897).abs() < 5e-5, "the label says R1.1897: {got}");
    }

    let diameter = of_kind(&dims, "DIAMETER");
    let DimensionGeometry::Diameter {
        chord_start,
        chord_end,
        ..
    } = diameter.geometry
    else {
        unreachable!()
    };
    // The diameter is the chord through the circle: both dimensions
    // annotate the same circle, so it is twice the radius above.
    let by_hand = (chord_end.x - chord_start.x).hypot(chord_end.y - chord_start.y);
    assert_eq!(diameter.display_text, "\u{2205}2.3794");
    for got in [
        diameter.measurement.unwrap(),
        diameter.measurement_from_points.unwrap(),
    ] {
        assert!((got - by_hand).abs() < 1e-9, "{got} vs {by_hand}");
        assert!((got - 2.3794).abs() < 5e-5, "the label says 2.3794: {got}");
        assert!(
            (got - 2.0 * radius.measurement.unwrap()).abs() < 1e-3,
            "the diameter is twice the radius: {got}"
        );
    }

    let angular = of_kind(&dims, "ANGULAR_3POINT");
    let DimensionGeometry::Angular3Point {
        center,
        xline1,
        xline2,
    } = angular.geometry
    else {
        unreachable!()
    };
    // The angle between the two rays from the centre, in degrees.
    let ray = |p: Point3D| (p.y - center.y).atan2(p.x - center.x);
    let by_hand = (ray(xline2) - ray(xline1)).to_degrees().abs();
    assert_eq!(angular.display_text, "45\u{00B0}");
    for got in [
        angular.measurement.unwrap(),
        angular.measurement_from_points.unwrap(),
    ] {
        assert!((got - by_hand).abs() < 1e-9, "{got} vs {by_hand}");
        assert!(
            (got - 45.0).abs() < 1e-9,
            "the label says 45 degrees: {got}"
        );
    }

    // All three carry AutoCAD's cached label rather than a formatted one,
    // and its measurement agrees with the definition points.
    for dim in [radius, diameter, angular] {
        assert_eq!(dim.display_source, DisplaySource::CachedBlock);
        assert_eq!(dim.dimstyle, "Standard");
        let (stored, recomputed) = (
            dim.measurement.unwrap(),
            dim.measurement_from_points.unwrap(),
        );
        assert!(
            (stored - recomputed).abs() <= 1e-6 * stored.abs().max(1.0),
            "{}: {stored} vs {recomputed}",
            dim.common.handle
        );
    }
}

#[test]
fn the_radial_fixture_reads_its_definition_points_from_dxf_groups() {
    // Derived by hand (tests/fixtures/README.md): a radius whose centre
    // (DXF 10) is the origin and whose point on the circle (15) is (3,4),
    // a 3-4-5 triangle, so the radius is 5; a diameter whose chord runs
    // from (20,0) (10) to (20,10) (15), so the diameter is 10; and a
    // 3-point angular dimension about (40,0) (15) between the rays to
    // (50,0) (13) and (45, 8.660254) (14), 60 degrees apart, with the arc
    // point (10) at 30 degrees -- inside that sector rather than in the
    // 300-degree one on the other side.
    let db = uncad::parse(RADIAL).expect("fixture must parse");
    assert_eq!(db.header.format, "dxf");
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 3);

    let radius = of_kind(&dims, "RADIUS");
    let DimensionGeometry::Radius {
        center,
        chord_point,
        leader_length,
    } = radius.geometry
    else {
        unreachable!()
    };
    assert!(same_point(center, p3(0.0, 0.0)), "{center:?}");
    assert!(same_point(chord_point, p3(3.0, 4.0)), "{chord_point:?}");
    assert_eq!(leader_length, 0.0);
    assert_eq!(radius.measurement, Some(5.0));
    assert_eq!(radius.measurement_from_points, Some(5.0));

    let diameter = of_kind(&dims, "DIAMETER");
    let DimensionGeometry::Diameter {
        chord_start,
        chord_end,
        ..
    } = diameter.geometry
    else {
        unreachable!()
    };
    assert!(same_point(chord_start, p3(20.0, 0.0)), "{chord_start:?}");
    assert!(same_point(chord_end, p3(20.0, 10.0)), "{chord_end:?}");
    assert_eq!(diameter.measurement, Some(10.0));
    assert_eq!(diameter.measurement_from_points, Some(10.0));

    let angular = of_kind(&dims, "ANGULAR_3POINT");
    let DimensionGeometry::Angular3Point {
        center,
        xline1,
        xline2,
    } = angular.geometry
    else {
        unreachable!()
    };
    assert!(same_point(center, p3(40.0, 0.0)), "{center:?}");
    assert!(same_point(xline1, p3(50.0, 0.0)), "{xline1:?}");
    assert!(
        same_point(xline2, p3(45.0, 8.660254037844386)),
        "{xline2:?}"
    );
    assert!(
        same_point(angular.definition_point, p3(44.33012701892219, 2.5)),
        "{:?}",
        angular.definition_point
    );
    let degrees = angular.measurement_from_points.unwrap();
    assert!((degrees - 60.0).abs() < 1e-9, "{degrees}");
    assert!((angular.measurement.unwrap() - 60.0).abs() < 1e-9);
    assert_eq!(angular.display_text, "60\u{00B0}");

    // No *D block is cached, so the labels are formatted from the values.
    // They carry no `R` or diameter sign: the DIMPOST prefix and suffix
    // are not applied yet (docs/VLM_EXPORT_DESIGN.md, "what stays out").
    assert_eq!(radius.display_source, DisplaySource::Formatted);
    assert_eq!(radius.display_text, "5.00");
    assert_eq!(diameter.display_text, "10.00");
}
