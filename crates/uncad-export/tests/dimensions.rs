//! Dimension values against real files: the stored measurement, the one
//! recomputed from the definition points, and the label the drawing shows.
//! `example_2000.dwg`'s labels were written by AutoCAD and are the reference
//! for the values; `example_2007.dwg` is the same drawing in a format that
//! stores `act_measurement`, and the R13/R14 files are the same drawing in
//! formats that do not; `dimlfac12_r2000.dxf` is this project's own fixture
//! (ground truth in crates/uncad/tests/fixtures/README.md).

use uncad::Entity;
use uncad_export::dimension::{
    cached_labels, display_text, is_angular, measurement_from_points, usable_stored_measurement,
    DimDefaults, DisplaySource, EffectiveStyle,
};
use uncad_model::model::{DimensionEntity, DimensionKind};

const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);
const DIMLFAC12: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../uncad/tests/fixtures/dimlfac12_r2000.dxf"
);

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
    d.common.source_handle.name()
}

/// The measurement the package reports: the stored one when it can be
/// believed, the definition points' otherwise.
fn value(d: &DimensionEntity) -> Option<f64> {
    let from_points = measurement_from_points(d);
    usable_stored_measurement(d.measurement, d.kind, from_points).or(from_points)
}

fn label(
    db: &uncad::CadDatabase,
    header: &uncad::Header,
    d: &DimensionEntity,
) -> (String, DisplaySource) {
    let style = EffectiveStyle::resolve(
        d.style_name
            .resolved()
            .and_then(|name| db.tables.dim_styles.get(name)),
        &DimDefaults::from_header(header),
    );
    let labels = cached_labels(&db.tables);
    let shown = display_text(d, &style, value(d), labels.get(d.block_name.name()));
    (shown.text, shown.source)
}

#[test]
fn autocad_written_dimensions_agree_with_their_definition_points_and_labels() {
    let (db, header) = uncad::parse_with_header(format!("{TEST_DATA}/example_2000.dwg"))
        .expect("corpus file must parse");
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 10, "example_2000.dwg has ten dimensions");
    for d in &dims {
        let stored = usable_stored_measurement(d.measurement, d.kind, measurement_from_points(d))
            .unwrap_or_else(|| panic!("{} stores a believable measurement", handle(d)));
        let recomputed = measurement_from_points(d)
            .unwrap_or_else(|| panic!("{} {:?} has its points", handle(d), d.kind));
        assert!(
            (stored - recomputed).abs() <= 1e-6 * stored.abs().max(1.0),
            "{} {:?}: stored {stored} vs from points {recomputed}",
            handle(d),
            d.kind
        );
        let (_, source) = label(&db, &header, d);
        assert_ne!(source, DisplaySource::None, "{} has no label", handle(d));
    }

    // An ALIGNED dimension whose cached label AutoCAD wrote with a decimal
    // comma (DIMDSEP) and two decimals.
    let aligned = dims
        .iter()
        .find(|d| d.kind == Some(DimensionKind::Aligned))
        .expect("an ALIGNED dimension");
    assert!((value(aligned).unwrap() - 1504.6795).abs() < 1e-3);
    assert_eq!(
        label(&db, &header, aligned),
        ("1504,68".to_string(), DisplaySource::CachedBlock)
    );

    // A two-line angular dimension: 108 degrees, stored in radians,
    // reported in degrees and labelled with a degree sign.
    let angular = dims
        .iter()
        .find(|d| d.kind == Some(DimensionKind::Angular2Line))
        .expect("an ANGULAR_2LINE dimension");
    assert!(is_angular(angular.kind));
    let stored = angular.measurement.expect("R2000 stores it");
    assert!((stored.to_degrees() - 108.0).abs() < 1e-3, "{stored} rad");
    assert!((value(angular).unwrap() - 108.0).abs() < 1e-3);
    assert_eq!(label(&db, &header, angular).0, "108\u{00B0}");
}

#[test]
fn a_file_that_stores_no_measurement_gets_the_definition_points_value() {
    // The expected values are the `act_measurement`s AutoCAD itself wrote
    // into `example_2007.dwg`, the same drawing saved in a newer format
    // (identical handles, identical definition points). Each old file's
    // value has to match its 2007 twin's stored one.
    let reference = uncad::parse(format!("{TEST_DATA}/example_2007.dwg")).expect("parses");
    let reference: Vec<(String, f64)> = dimensions(&reference)
        .iter()
        .map(|d| {
            let stored =
                usable_stored_measurement(d.measurement, d.kind, measurement_from_points(d))
                    .unwrap_or_else(|| panic!("2007 writes act_measurement for {}", handle(d)));
            (handle(d).to_string(), stored)
        })
        .collect();
    assert_eq!(reference.len(), 10, "the drawing has ten dimensions");

    for name in ["example_r14.dwg", "example_r13.dwg", "example_r13.dxf"] {
        let db = uncad::parse(format!("{TEST_DATA}/{name}")).expect("corpus file must parse");
        let dims = dimensions(&db);
        for (h, want) in &reference {
            let Some(d) = dims.iter().find(|d| handle(d) == h) else {
                // The R13 DXF drops the ARC_LENGTH dimension entirely.
                continue;
            };
            // Nothing believable is stored, so the definition points are
            // the source ...
            assert_eq!(
                usable_stored_measurement(d.measurement, d.kind, measurement_from_points(d)),
                None,
                "{name} {h} stored"
            );
            // ... and they give the value the 2007 file measured, to the
            // tolerance the two files' own float storage needs (the worst
            // pair in the drawing differs by 2.3e-7 relative).
            let got = measurement_from_points(d)
                .unwrap_or_else(|| panic!("{name} {h} has definition points"));
            assert!(
                (got - want).abs() <= 1e-6 * want.abs(),
                "{name} {h}: {got} vs the 2007 file's {want}"
            );
        }
    }
}

#[test]
fn a_believable_stored_measurement_is_still_preferred() {
    // `example_2007.dwg`'s ALIGNED 37E stores 1504.6794770244742 while its
    // definition points give 1504.6798093211207 -- a real 3.3e-4
    // disagreement in the file itself, which the stored value must survive.
    let db = uncad::parse(format!("{TEST_DATA}/example_2007.dwg")).expect("parses");
    let dims = dimensions(&db);
    let aligned = dims
        .iter()
        .find(|d| handle(d) == "37E")
        .expect("handle 37E");
    let computed = measurement_from_points(aligned).expect("it has points");
    let stored = usable_stored_measurement(aligned.measurement, aligned.kind, Some(computed))
        .expect("2007 stores a believable measurement");
    assert!(stored != computed, "the file's two values differ");
    assert!((stored - 1504.679477024474).abs() < 1e-9, "{stored}");
}

#[test]
fn the_fixture_dimension_reads_its_style_and_its_cached_label() {
    // One LINEAR dimension 10 units long under $DIMLFAC 12 (the STANDARD
    // style says 12 too): the cached label AutoCAD would show is "120", and
    // a label formatted here from the same style says the same.
    let (db, header) = uncad::parse_with_header(DIMLFAC12).expect("fixture must parse");
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 1);
    let d = dims[0];
    assert_eq!(d.kind, Some(DimensionKind::Rotated));
    assert_eq!(measurement_from_points(d), Some(10.0));
    assert_eq!(value(d), Some(10.0));
    assert_eq!(
        label(&db, &header, d),
        ("120".to_string(), DisplaySource::CachedBlock)
    );
    let style = EffectiveStyle::resolve(
        db.tables.dim_styles.get("STANDARD"),
        &DimDefaults::from_header(&header),
    );
    assert_eq!(style.dimlfac, 12.0);
    let formatted = uncad_export::dimension::format_measurement(10.0, false, &style);
    assert!(formatted.starts_with("120"), "{formatted}");
}
