//! The golden cases: synthetic drawings whose spec is the oracle.
//!
//! `tests/golden/` holds, per case, the DXF the `uncad-model` golden writer
//! produced and the model it says a reader must produce from it. Both are
//! copies of that writer's output (see `tests/golden/README.md`); these
//! tests read each DXF and require the model to come back exactly as
//! stated -- every entity, every value, every reference, in file order.
//!
//! This is the check that caught three silent defects at once on the first
//! case: attribute definitions and values dropped by the library's chain
//! walkers, and closed LWPOLYLINEs reported as open.

use std::fs;
use std::path::PathBuf;
use uncad::CadDatabase;

/// Every case, as (name, DXF text, expected model JSON).
const CASES: [(&str, &str, &str); 5] = [
    (
        "g1",
        include_str!("golden/g1.dxf"),
        include_str!("golden/g1.expected.json"),
    ),
    (
        "g2",
        include_str!("golden/g2.dxf"),
        include_str!("golden/g2.expected.json"),
    ),
    (
        "g6",
        include_str!("golden/g6.dxf"),
        include_str!("golden/g6.expected.json"),
    ),
    (
        "g9",
        include_str!("golden/g9.dxf"),
        include_str!("golden/g9.expected.json"),
    ),
    (
        "g10",
        include_str!("golden/g10.dxf"),
        include_str!("golden/g10.expected.json"),
    ),
];

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct Fixture(PathBuf);

impl Fixture {
    fn write(name: &str, contents: &str) -> Self {
        let path = std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name));
        fs::write(&path, contents).expect("the temp dir should be writable");
        Fixture(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn assert_reads_back_exactly(name: &str, dxf: &str, expected_json: &str) {
    let fixture = Fixture::write(&format!("golden-{name}.dxf"), dxf);
    let db = uncad::parse(&fixture.0).unwrap_or_else(|e| panic!("{name} should parse: {e}"));
    let expected: CadDatabase =
        serde_json::from_str(expected_json).expect("the expected model deserializes");

    // Entity by entity first, so a failure names the entity rather than
    // dumping two whole drawings.
    assert_eq!(
        db.entities.len(),
        expected.entities.len(),
        "{name}: entity count: {:?}",
        db.entities
            .iter()
            .map(|e| e.type_name())
            .collect::<Vec<_>>()
    );
    for (got, want) in db.entities.iter().zip(&expected.entities) {
        assert_eq!(got, want, "{name}");
    }
    for (block, want) in &expected.tables.block_records {
        let got = db
            .tables
            .block_records
            .get(block)
            .unwrap_or_else(|| panic!("{name}: block {block} was not read"));
        assert_eq!(got, want, "{name}: block {block}");
    }
    assert_eq!(db, expected, "{name}");
}

#[test]
fn every_golden_case_reads_back_exactly_as_its_spec_says() {
    for (name, dxf, expected) in CASES {
        assert_reads_back_exactly(name, dxf, expected);
    }
}

#[test]
fn the_fixtures_are_the_r2000_dxfs_the_writer_produces() {
    // The fixtures are copies; the umbrella's verify script regenerates them
    // and fails when a copy has drifted from the writer. Here only the shape
    // a reader depends on is pinned.
    for (name, dxf, expected) in CASES {
        assert!(
            dxf.contains("  9\n$ACADVER\n  1\nAC1015\n"),
            "{name} is R2000"
        );
        assert!(dxf.ends_with("  0\nEOF\n"), "{name} ends with EOF");
        assert!(expected.starts_with('{'), "{name} has a JSON model");
    }
}

/// G4: a file cut short must never read as a clean, complete drawing. Either
/// the read fails, or the diagnostics say something, or the drawing is in
/// fact complete -- a silently shortened drawing is the one outcome that is
/// never acceptable.
#[test]
fn a_truncated_file_is_never_a_clean_shorter_drawing() {
    let (_, dxf, expected) = CASES[0];
    let full: CadDatabase = serde_json::from_str(expected).unwrap();
    let entities_section = dxf
        .find("  2\nENTITIES\n")
        .expect("G1 has an ENTITIES section");
    // Cut points inside the ENTITIES section, from just after its start to
    // just before EOF, so each truncation loses drawing content.
    let end = dxf.len() - "  0\nEOF\n".len();
    let step = (end - entities_section) / 6;
    for i in 1..6 {
        let cut = entities_section + i * step;
        // Cut on a line boundary so the file stays a (code, value) stream up
        // to the cut; a mid-line cut is the same case with more noise.
        let cut = dxf[..cut].rfind('\n').map(|p| p + 1).unwrap_or(cut);
        let truncated = &dxf[..cut];
        let fixture = Fixture::write(&format!("golden-g4-{i}.dxf"), truncated);
        match uncad::parse(&fixture.0) {
            Err(_) => {}
            Ok(db) => {
                let complete = db.entities == full.entities;
                assert!(
                    complete || !db.read_diagnostics.is_clean(),
                    "cut at byte {cut}: read back {} of {} entities with clean diagnostics -- \
                     a truncated file must not look like a clean, shorter drawing",
                    db.entities.len(),
                    full.entities.len()
                );
            }
        }
    }
}
