//! What a DIMENSION carries: its kind, the points it was built from by DXF
//! group, the measurement the file stored, its text and style. Sources: the
//! `dimlfac12_r2000.dxf` fixture (ground truth in tests/fixtures/README.md),
//! the corpus's `example_2000.dwg` and its text twin -- the same drawing
//! through LibreDWG's DWG decoder and its DXF importer, which lay two kinds'
//! points out differently -- `2000/TS1.dwg` for the radial and 3-point
//! kinds `example_2000` lacks, and the `example_2007` / R13 / R14 family,
//! the same drawing with and without a stored measurement.
//!
//! How a measurement is displayed, and whether it agrees with the points,
//! are consumers' questions: the model carries what the file states.

use uncad::model::{DimensionEntity, DimensionKind, OrdinateAxis, Point3D, Ref, TextOverride};
use uncad::Entity;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);
const DIMLFAC12: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dimlfac12_r2000.dxf"
);

fn parse(path: &str) -> uncad::CadDatabase {
    uncad::parse(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn corpus(name: &str) -> uncad::CadDatabase {
    parse(&format!("{CORPUS}{name}"))
}

fn dimensions(db: &uncad::CadDatabase) -> Vec<&DimensionEntity> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Dimension(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn handle(d: &DimensionEntity) -> &str {
    match &d.common.source_handle {
        Ref::Resolved(h) => h,
        other => panic!("{other:?}"),
    }
}

fn same(a: Option<Point3D>, b: Option<Point3D>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
        }
        _ => false,
    }
}

fn p3(x: f64, y: f64) -> Option<Point3D> {
    Some(Point3D { x, y, z: 0.0 })
}

#[test]
fn the_fixture_dimension_carries_its_value_points_and_style() {
    let db = parse(DIMLFAC12);
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 1);
    let dim = dims[0];
    assert_eq!(dim.kind, Some(DimensionKind::Rotated));
    assert_eq!(dim.measurement, Some(10.0), "DXF 42 as written");
    assert_eq!(dim.definition_point, p3(10.0, 5.0), "group 10");
    assert_eq!((dim.text_midpoint.x, dim.text_midpoint.y), (5.0, 6.0));
    assert_eq!(dim.points.extension1, p3(0.0, 0.0), "group 13");
    assert_eq!(dim.points.extension2, p3(10.0, 0.0), "group 14");
    assert_eq!((dim.points.radial, dim.points.arc), (None, None));
    assert_eq!(dim.rotation, 0.0);
    assert_eq!(
        dim.text_override,
        TextOverride::Measured,
        "an empty group 1"
    );
    assert_eq!(dim.style_name, Ref::Resolved("STANDARD".to_string()));
    assert_eq!(dim.ordinate_axis, None);
    // The style's own DIMLFAC (144), which is what the displayed 120 is 12
    // times the measurement by.
    assert_eq!(db.tables.dim_styles["STANDARD"].length_factor, Some(12.0));
}

/// `example_2000.dxf` is AutoCAD's DXF of `example_2000.dwg`, so every
/// dimension must come out the same whichever of LibreDWG's readers read
/// it -- in particular the 2-line angular (43B, 108 degrees) and the
/// X-type ordinate (430, 4630.52), whose points and axis the DXF importer
/// stores in different fields from the DWG decoder.
#[test]
fn the_dwg_and_dxf_readers_agree_on_every_dimension() {
    let (dwg, dxf) = (corpus("example_2000.dwg"), corpus("example_2000.dxf"));
    let (from_dwg, from_dxf) = (dimensions(&dwg), dimensions(&dxf));
    assert_eq!(from_dwg.len(), from_dxf.len());
    assert!(from_dwg.len() >= 10, "{}", from_dwg.len());
    for a in &from_dwg {
        let b = from_dxf
            .iter()
            .find(|d| handle(d) == handle(a))
            .unwrap_or_else(|| panic!("{} missing from the DXF", handle(a)));
        let what = handle(a);
        assert_eq!(a.kind, b.kind, "{what}");
        assert!(
            same(a.definition_point, b.definition_point),
            "{what}: {a:?} vs {b:?}"
        );
        for (x, y) in [
            (a.points.extension1, b.points.extension1),
            (a.points.extension2, b.points.extension2),
            (a.points.radial, b.points.radial),
        ] {
            assert!(same(x, y), "{what}: {:?} vs {:?}", a.points, b.points);
        }
        // An arc-length dimension without a leader is where the two readers
        // part: the DWG record states group 16 whatever the leader, and the
        // DXF importer leaves the group zero when the file omits it, so the
        // DXF side cannot tell a stated point from silence and says none.
        let leaderless_arc_length =
            a.kind == Some(DimensionKind::ArcLength) && b.points.arc.is_none();
        if leaderless_arc_length {
            assert!(a.points.arc.is_some(), "{what}: the DWG states group 16");
        } else {
            assert!(
                same(a.points.arc, b.points.arc),
                "{what}: {:?} vs {:?}",
                a.points,
                b.points
            );
        }
        assert_eq!(a.ordinate_axis, b.ordinate_axis, "{what}");
        let (ma, mb) = (a.measurement.unwrap(), b.measurement.unwrap());
        assert!((ma - mb).abs() < 1e-6 * ma.abs().max(1.0), "{what}");
    }

    let two_line = from_dxf.iter().find(|d| handle(d) == "43B").expect("43B");
    assert_eq!(two_line.kind, Some(DimensionKind::Angular2Line));
    let radians = two_line.measurement.unwrap();
    assert!((radians - 108f64.to_radians()).abs() < 1e-9, "{radians}");
    let ordinate = from_dxf.iter().find(|d| handle(d) == "430").expect("430");
    assert_eq!(ordinate.ordinate_axis, Some(OrdinateAxis::X));
    let x = ordinate.measurement.unwrap();
    assert!((x - 4630.519359).abs() < 1e-5, "{x}");
    // The feature 430 measures sits at x = 4630.52 from the datum at the
    // origin: group 13's x is the measurement.
    assert!((ordinate.points.extension1.unwrap().x - x).abs() < 1e-6);
}

/// The one dimension of `kind` in `dims`.
fn of_kind<'a>(dims: &[&'a DimensionEntity], kind: DimensionKind) -> &'a DimensionEntity {
    let mut found = dims.iter().filter(|d| d.kind == Some(kind));
    let dim = *found
        .next()
        .unwrap_or_else(|| panic!("a {kind:?} dimension"));
    assert!(found.next().is_none(), "one {kind:?} dimension only");
    dim
}

/// `2000/TS1.dwg` is a sampler AutoCAD wrote (its TS1.txt lists a 3-point
/// angular, a diametric and a radial dimension); its cached labels
/// `R1.1897`, `2.3794` and `45` degrees are AutoCAD's own. Each stored
/// measurement agrees with the points it was built from, which is what
/// shows the points are in the right fields: the centre in group 10 and the
/// point on the arc in 15 for a radius, the chord's ends for a diameter,
/// the vertex in 15 and the two rays' points in 13 and 14 for an angle.
#[test]
fn the_radial_and_three_point_kinds_read_from_an_autocad_dwg() {
    let db = corpus("2000/TS1.dwg");
    let dims = dimensions(&db);
    let distance = |a: Option<Point3D>, b: Option<Point3D>| {
        let (a, b) = (a.expect("a point"), b.expect("a point"));
        (b.x - a.x).hypot(b.y - a.y)
    };

    let radius = of_kind(&dims, DimensionKind::Radius);
    let by_points = distance(radius.definition_point, radius.points.radial);
    let stored = radius.measurement.unwrap();
    assert!((stored - by_points).abs() < 1e-9, "{stored} vs {by_points}");
    assert!(
        (stored - 1.1897).abs() < 5e-5,
        "the label says R1.1897: {stored}"
    );

    let diameter = of_kind(&dims, DimensionKind::Diameter);
    let by_points = distance(diameter.definition_point, diameter.points.radial);
    let stored = diameter.measurement.unwrap();
    assert!((stored - by_points).abs() < 1e-9, "{stored} vs {by_points}");
    assert!(
        (stored - 2.3794).abs() < 5e-5,
        "the label says 2.3794: {stored}"
    );

    let angular = of_kind(&dims, DimensionKind::Angular3Point);
    let centre = angular.points.radial.expect("group 15");
    let ray = |p: Option<Point3D>| {
        let p = p.expect("a point");
        (p.y - centre.y).atan2(p.x - centre.x)
    };
    let by_points = (ray(angular.points.extension2) - ray(angular.points.extension1)).abs();
    let stored = angular.measurement.unwrap();
    assert!((stored - by_points).abs() < 1e-9, "{stored} vs {by_points}");
    assert!(
        (stored - 45f64.to_radians()).abs() < 1e-9,
        "the label says 45 degrees; the file states radians: {stored}"
    );

    for dim in [radius, diameter, angular] {
        assert_eq!(dim.style_name, Ref::Resolved("Standard".to_string()));
        assert!(matches!(dim.block_name, Ref::Resolved(_)), "{dim:?}");
    }
}

/// The same drawing in four files that differ in whether they carry
/// `act_measurement`: `example_2007.dwg` writes it, and the R13 and R14
/// files leave it at 0.0 -- which reads as "not stated", not as a zero
/// measurement.
#[test]
fn a_stored_measurement_of_zero_is_not_a_measurement() {
    let reference = corpus("example_2007.dwg");
    let reference: Vec<(String, f64)> = dimensions(&reference)
        .iter()
        .map(|d| {
            let m = d
                .measurement
                .unwrap_or_else(|| panic!("2007 writes act_measurement for {}", handle(d)));
            (handle(d).to_string(), m)
        })
        .collect();
    assert_eq!(reference.len(), 10, "the drawing has ten dimensions");

    for name in ["example_r14.dwg", "example_r13.dwg", "example_r13.dxf"] {
        let db = corpus(name);
        let dims = dimensions(&db);
        let mut seen = 0;
        for (h, _) in &reference {
            // The R13 DXF drops the ARC_LENGTH dimension entirely.
            let Some(d) = dims.iter().find(|d| handle(d) == h) else {
                continue;
            };
            assert_eq!(d.measurement, None, "{name} {h}");
            seen += 1;
        }
        assert!(seen >= 9, "{name}: {seen} of the ten dimensions");
    }
}

/// Where the file carries a measurement, it is carried to the last bit:
/// `example_2007.dwg`'s ALIGNED 37E states 1504.6794770244742, the value the
/// R13 and R14 saves of the same drawing leave out.
#[test]
fn a_stored_measurement_is_carried_as_stated() {
    let db = corpus("example_2007.dwg");
    let dims = dimensions(&db);
    let aligned = dims.iter().find(|d| handle(d) == "37E").expect("37E");
    assert_eq!(aligned.kind, Some(DimensionKind::Aligned));
    assert_eq!(aligned.measurement, Some(1504.6794770244742));
}

/// An R2000+ TOLERANCE stores no height of its own (LibreDWG decodes
/// `height` for R13/R14 only), so the model says none and names the style
/// its height comes from: `example_2000.dwg`'s 4F1 draws at ISO-25's
/// DIMTXT, 2.5 -- a consumer's lookup.
#[test]
fn a_tolerance_names_the_style_its_height_comes_from() {
    let db = corpus("example_2000.dwg");
    let tolerance = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Tolerance(t) if t.common.source_handle == Ref::Resolved("4F1".to_string()) => {
                Some(t)
            }
            _ => None,
        })
        .expect("TOLERANCE 4F1");
    assert_eq!(tolerance.text_height, None);
    assert_eq!(tolerance.style_name, Ref::Resolved("ISO-25".to_string()));
    assert_eq!(db.tables.dim_styles["ISO-25"].text_height, Some(2.5));
}

#[test]
fn an_arc_length_dimension_without_a_leader_still_carries_its_group_16() {
    // The record stores the first leader point (group 16) whether or not the
    // dimension has a leader, and this one has none (group 71 is 0). The
    // DXF twin writes the same point, which here is the first extension
    // line's origin.
    let db = corpus("example_2000.dwg");
    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) if d.common.source_handle == Ref::Resolved("399".to_string()) => {
                Some(d)
            }
            _ => None,
        })
        .expect("the drawing's arc-length dimension");
    assert_eq!(dim.kind, Some(DimensionKind::ArcLength));
    let stated = Point3D {
        x: 5486.627993960748,
        y: 2529.600165964192,
        z: 0.0,
    };
    let arc = dim.points.arc.expect("group 16 is stated");
    assert!(
        (arc.x - stated.x).abs() < 1e-9 && (arc.y - stated.y).abs() < 1e-9 && arc.z == 0.0,
        "{arc:?}"
    );
}
