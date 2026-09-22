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
//! walkers, and closed LWPOLYLINEs reported as open. The Korean case then
//! caught a fourth: every pre-R2007 string read as raw codepage bytes.
//!
//! The DXFs are bytes, not text: a case with a codepage (G8, CP949) is not
//! UTF-8 on disk, by design.

use std::fs;
use std::path::PathBuf;
use uncad::model::Ref;
use uncad::{CadDatabase, Entity};

/// Every case, as (name, DXF bytes, expected model JSON).
const CASES: [(&str, &[u8], &str); 7] = [
    (
        "g1",
        include_bytes!("golden/g1.dxf"),
        include_str!("golden/g1.expected.json"),
    ),
    (
        "g2",
        include_bytes!("golden/g2.dxf"),
        include_str!("golden/g2.expected.json"),
    ),
    (
        "g6",
        include_bytes!("golden/g6.dxf"),
        include_str!("golden/g6.expected.json"),
    ),
    (
        "g7",
        include_bytes!("golden/g7.dxf"),
        include_str!("golden/g7.expected.json"),
    ),
    (
        "g8",
        include_bytes!("golden/g8.dxf"),
        include_str!("golden/g8.expected.json"),
    ),
    (
        "g9",
        include_bytes!("golden/g9.dxf"),
        include_str!("golden/g9.expected.json"),
    ),
    (
        "g10",
        include_bytes!("golden/g10.dxf"),
        include_str!("golden/g10.expected.json"),
    ),
];

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct Fixture(PathBuf);

impl Fixture {
    fn write(name: &str, contents: &[u8]) -> Self {
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

fn assert_reads_back_exactly(name: &str, dxf: &[u8], expected_json: &str) {
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
    assert!(
        db.read_diagnostics.is_clean(),
        "{name}: a drawing read exactly must have nothing to warn about: {:?}",
        db.read_diagnostics.warnings
    );
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
        let text = String::from_utf8_lossy(dxf);
        assert!(
            text.contains("  9\n$ACADVER\n  1\nAC1015\n"),
            "{name} is R2000"
        );
        assert!(text.ends_with("  0\nEOF\n"), "{name} ends with EOF");
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
    let dxf = std::str::from_utf8(dxf).expect("G1 is ASCII");
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
        let fixture = Fixture::write(&format!("golden-g4-{i}.dxf"), truncated.as_bytes());
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

/// G8 is the measurement behind the codepage decoding: the file declares
/// `$DWGCODEPAGE = ANSI_949` and stores its Korean text as CP949 bytes, and
/// the model must carry it as UTF-8 -- layer and block names, attribute
/// values and defaults, a text -- with nothing to warn about.
#[test]
fn g8_korean_text_in_cp949_comes_back_as_utf8_in_every_place_it_appears() {
    let (_, dxf, _) = CASES.iter().find(|(n, _, _)| *n == "g8").unwrap();
    assert!(
        std::str::from_utf8(dxf).is_err(),
        "the fixture is CP949, not UTF-8"
    );
    let fixture = Fixture::write("golden-g8-places.dxf", dxf);
    let db = uncad::parse(&fixture.0).expect("G8 parses");
    assert!(
        db.read_diagnostics.is_clean(),
        "{:?}",
        db.read_diagnostics.warnings
    );

    let title_layer = "\u{D45C}\u{C81C}\u{B780}";
    let block = "\u{D45C}\u{C81C}\u{BE14}\u{B85D}";
    assert!(db.tables.layers.contains_key(title_layer), "layer table");
    assert!(db.tables.block_records.contains_key(block), "block record");
    let Some(Entity::Insert(insert)) = db.entities.get(1) else {
        panic!("the second entity is the INSERT: {:?}", db.entities.get(1));
    };
    assert_eq!(insert.common.layer, Ref::Resolved(title_layer.to_string()));
    assert_eq!(insert.block_name, Ref::Resolved(block.to_string()));
    let values: Vec<&str> = insert.attribs.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(
        values,
        [
            "BP-1042",
            "SS400 \u{C77C}\u{BC18}\u{AD6C}\u{C870}\u{C6A9} \u{C555}\u{C5F0}\u{AC15}\u{C7AC}",
            "\u{D64D}\u{AE38}\u{B3D9}",
        ]
    );
    let defaults: Vec<&str> = db.tables.block_records[block]
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Attdef(a) => Some(a.default_value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(defaults, ["", "SS400", "\u{BBF8}\u{C815}"]);
    let Some(Entity::Text(text)) = db.entities.last() else {
        panic!("the last entity is the TEXT");
    };
    assert_eq!(text.text, "\u{CD95}\u{CC99} 1:1");
}

/// The same CP949 file stamped with a different codepage reads as a
/// different drawing: the declared codepage is what the bytes mean. Every
/// byte of this case happens to have a Windows-1252 character, so the
/// misdeclared read is clean -- a wrong declaration is not detectable when
/// every byte maps, which is why the declaration is trusted and documented
/// as such rather than second-guessed.
#[test]
fn the_declared_codepage_is_what_the_bytes_mean() {
    let (_, dxf, expected) = *CASES.iter().find(|(n, _, _)| *n == "g8").unwrap();
    let needle = b"\nANSI_949\n";
    let at = dxf.windows(needle.len()).position(|w| w == needle).unwrap();
    let mut restamped = dxf[..at].to_vec();
    restamped.extend_from_slice(b"\nANSI_1252\n");
    restamped.extend_from_slice(&dxf[at + needle.len()..]);

    let fixture = Fixture::write("golden-g8-restamped.dxf", &restamped);
    let db = uncad::parse(&fixture.0).expect("still parses");
    let expected: CadDatabase = serde_json::from_str(expected).unwrap();
    assert_ne!(
        db.entities, expected.entities,
        "CP949 bytes read as CP1252 cannot match"
    );
    let title_layer = "\u{D45C}\u{C81C}\u{B780}";
    assert!(!db.tables.layers.contains_key(title_layer));
    // The CP949 lead/trail bytes of that name, each read as one CP1252
    // character.
    assert!(db
        .tables
        .layers
        .contains_key("\u{C7}\u{A5}\u{C1}\u{A6}\u{B6}\u{F5}"));
}
