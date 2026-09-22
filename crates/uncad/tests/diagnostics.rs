//! The places where a read could come back empty *without saying so*, and
//! the signals that now say so. Each test pairs an input that must produce
//! the signal with one that must not -- a check that only looked for the
//! signal could not tell "reported when it should be" from "reported always".
//! (The rendering-side signal, a block reference that draws nothing, is
//! tested in the renderer crate.)

use uncad::{read_diagnostics_from_libredwg_bits, CadDatabase, ReadDiagnostics};

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
