//! The golden cases: synthetic drawings whose spec is the oracle.
//!
//! `tests/golden/` holds, per case, the DXF the `uncad-model` golden writer
//! produced and the model it says a reader must produce from it. Both are
//! copies of that writer's output (see `tests/golden/README.md`); this test
//! reads the DXF and requires the model to come back exactly as stated --
//! every entity, every value, every reference, in file order.
//!
//! This is the check that caught three silent defects at once: attribute
//! definitions and values dropped by the library's chain walkers, and closed
//! LWPOLYLINEs reported as open.

use std::fs;
use std::path::PathBuf;
use uncad::CadDatabase;

const G1_DXF: &str = include_str!("golden/g1.dxf");
const G1_EXPECTED: &str = include_str!("golden/g1.expected.json");

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

#[test]
fn g1_reads_back_exactly_as_its_spec_says() {
    let fixture = Fixture::write("golden-g1.dxf", G1_DXF);
    let db = uncad::parse(&fixture.0).expect("G1 should parse");
    let expected: CadDatabase =
        serde_json::from_str(G1_EXPECTED).expect("the expected model deserializes");

    // Entity by entity first, so a failure names the entity rather than
    // dumping two whole drawings.
    assert_eq!(
        db.entities.len(),
        expected.entities.len(),
        "entity count: {:?}",
        db.entities
            .iter()
            .map(|e| e.type_name())
            .collect::<Vec<_>>()
    );
    for (got, want) in db.entities.iter().zip(&expected.entities) {
        assert_eq!(got, want);
    }
    for (name, want) in &expected.tables.block_records {
        let got = db
            .tables
            .block_records
            .get(name)
            .unwrap_or_else(|| panic!("block {name} was not read"));
        assert_eq!(got, want, "block {name}");
    }
    assert_eq!(db, expected);
}

#[test]
fn g1_is_the_r2000_dxf_the_writer_produces() {
    // The fixture is a copy; the umbrella's verify script regenerates it
    // and fails when the copy has drifted from the writer. Here only the
    // shape a reader depends on is pinned.
    assert!(G1_DXF.contains("  9\n$ACADVER\n  1\nAC1015\n"));
    assert!(G1_DXF.ends_with("  0\nEOF\n"));
    assert!(
        G1_EXPECTED.contains("\"BP-1042\""),
        "the title block's drawing number"
    );
}
