//! A DXF read that loses entities says so. The importer can stop partway
//! through a file and still return success; the file's own ENTITIES section
//! states how many top-level records it holds, and a model holding fewer
//! carries an `ENTITIES_MISSING` warning with both counts.

const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

fn missing_warnings(name: &str) -> Vec<String> {
    let db = uncad::parse(format!("{TEST_DATA}{name}")).expect("reads");
    db.read_diagnostics
        .warnings
        .into_iter()
        .filter(|w| w.starts_with("ENTITIES_MISSING"))
        .collect()
}

/// `example_r14.dxf` holds 68 top-level entity records (its DWG twin reads
/// 72); the importer logs malformed hex values and returns one. This pins
/// today's loss: a reader that recovers the file changes the count here,
/// and then the warning must go away with it.
#[test]
fn a_dxf_read_that_keeps_one_of_sixty_eight_entities_is_reported() {
    let db = uncad::parse(format!("{TEST_DATA}example_r14.dxf")).expect("reads");
    assert_eq!(db.entities.len(), 1);
    let warnings: Vec<_> = db
        .read_diagnostics
        .warnings
        .iter()
        .filter(|w| w.starts_with("ENTITIES_MISSING"))
        .collect();
    assert_eq!(warnings.len(), 1, "{:?}", db.read_diagnostics.warnings);
    assert!(
        warnings[0].contains("holds 68 entity records"),
        "{}",
        warnings[0]
    );
    assert!(
        warnings[0].ends_with("1 reached the drawing"),
        "{}",
        warnings[0]
    );
}

/// The same drawing saved by other versions reads whole: no warning. The
/// model holds more than the section states (72 against 69) -- paper-space
/// content these versions keep in BLOCKS -- which is not a loss.
#[test]
fn the_same_drawing_read_whole_carries_no_warning() {
    for name in ["example_r13.dxf", "example_2000.dxf", "example_2018.dxf"] {
        assert_eq!(missing_warnings(name), Vec::<String>::new(), "{name}");
    }
}

/// Polylines with vertices and inserts with attributes: the owned records
/// are counted with their owner, so a whole read is not reported.
#[test]
fn owned_records_do_not_count_as_lost_entities() {
    for name in [
        "r11/entities-2d.dxf",
        "r11/entities-3d.dxf",
        "2000/PolyLine2D.dxf",
    ] {
        assert_eq!(missing_warnings(name), Vec::<String>::new(), "{name}");
    }
}
