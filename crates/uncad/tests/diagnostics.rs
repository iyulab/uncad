//! The places where a read could come back empty *without saying so*, and
//! the signals that now say so. Each test pairs an input that must produce
//! the signal with one that must not -- a check that only looked for the
//! signal could not tell "reported when it should be" from "reported always".

use std::collections::BTreeMap;

use uncad::model::{
    Confidence, EntityCommon, EntityId, InsertEntity, LineEntity, Origin, Point3D, Ref,
};
use uncad::tables::{BlockRecord, Tables};
use uncad::{
    read_diagnostics_from_libredwg_bits, CadDatabase, Entity, ReadDiagnostics, Space, ToSvgOptions,
};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

// --- LibreDWG's non-fatal error bits travel with the result ---------------

#[test]
fn libredwg_non_fatal_bits_are_reported_and_named() {
    // Measured: LibreDWG returns UNHANDLEDCLASS | VALUEOUTOFBOUNDS (0x44) for
    // this file and still reads it. Before, both bits were thrown away.
    let db =
        uncad::parse(format!("{CORPUS}/example_2018.dwg")).expect("the corpus DWG should parse");
    assert_eq!(
        db.read_diagnostics.warnings,
        vec!["UNHANDLEDCLASS".to_string(), "VALUEOUTOFBOUNDS".to_string()]
    );
    assert!(!db.read_diagnostics.is_clean());
    assert!(
        !db.entities.is_empty(),
        "the bits are a warning, not a failure: the drawing is still read"
    );
}

#[test]
fn a_clean_read_reports_nothing() {
    let db = uncad::parse(format!("{CORPUS}/2000/entities-2d.dxf"))
        .expect("the corpus DXF should parse");
    assert_eq!(db.read_diagnostics, ReadDiagnostics::default());
    assert!(db.read_diagnostics.is_clean());
}

#[test]
fn bit_names_follow_dwg_h_and_unknown_bits_are_not_dropped() {
    let d = read_diagnostics_from_libredwg_bits(1 | 2 | 64);
    assert_eq!(
        d.warnings,
        ["WRONGCRC", "NOTYETSUPPORTED", "VALUEOUTOFBOUNDS"]
    );
    // A bit this crate has no name for is still listed, never silently lost.
    let d = read_diagnostics_from_libredwg_bits(1 << 20);
    assert_eq!(d.warnings, ["BIT20"]);
    assert_eq!(
        read_diagnostics_from_libredwg_bits(0),
        ReadDiagnostics::default()
    );
}

#[test]
fn diagnostics_survive_the_json_round_trip_and_default_when_absent() {
    let db =
        uncad::parse(format!("{CORPUS}/example_2018.dwg")).expect("the corpus DWG should parse");
    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serialize");
    let back: CadDatabase = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.read_diagnostics, db.read_diagnostics);

    // JSON written before the field existed still loads, as a clean read.
    let old: CadDatabase = serde_json::from_str(
        r#"{"entities":[],"tables":{"layers":{},"block_records":{},"mlinestyles":{}}}"#,
    )
    .expect("old shape");
    assert_eq!(old.read_diagnostics, ReadDiagnostics::default());
}

// --- A block reference that draws nothing is reported --------------------

fn common(handle: &str) -> EntityCommon {
    EntityCommon {
        id: EntityId::new(u64::from_str_radix(handle, 16).unwrap()),
        origin: Origin::Vector,
        confidence: Confidence::High,
        source_handle: Ref::Resolved(handle.to_string()),
        layer: Ref::Resolved("0".to_string()),
        color_index: 256,
        true_color: None,
    }
}

fn p3(x: f64, y: f64, z: f64) -> Point3D {
    Point3D { x, y, z }
}

fn insert(handle: &str, block_name: &str) -> Entity {
    Entity::Insert(InsertEntity {
        common: common(handle),
        block_name: Ref::Resolved(block_name.to_string()),
        insertion_point: p3(0.0, 0.0, 0.0),
        scale: p3(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: Vec::new(),
    })
}

fn block(name: &str, entities: Vec<Entity>) -> (String, BlockRecord) {
    (
        name.to_string(),
        BlockRecord {
            name: name.to_string(),
            entities,
        },
    )
}

#[test]
fn a_block_reference_that_draws_nothing_is_named_and_one_that_draws_is_not() {
    let line = Entity::Line(LineEntity {
        common: common("10"),
        start_point: p3(0.0, 0.0, 0.0),
        end_point: p3(10.0, 0.0, 0.0),
    });
    let mut block_records = BTreeMap::new();
    block_records.extend([
        block("EMPTY", Vec::new()),
        block("FULL", vec![line.clone()]),
    ]);
    let db = CadDatabase {
        entities: vec![
            insert("20", "EMPTY"),
            insert("21", "EMPTY"), // referenced twice: reported once
            insert("22", "FULL"),
            line,
        ],
        tables: Tables {
            block_records,
            ..Tables::default()
        },
        read_diagnostics: ReadDiagnostics::default(),
    };

    // Space::All: this synthetic model has no *Model_Space record to select by.
    let result = uncad::to_svg(
        &db,
        ToSvgOptions {
            space: Space::All,
            ..ToSvgOptions::default()
        },
    );
    assert_eq!(result.empty_blocks, vec!["EMPTY".to_string()]);
    assert!(
        result.svg.contains("<line") || result.svg.contains("<path"),
        "the FULL block must still draw: {}",
        result.svg
    );
}
