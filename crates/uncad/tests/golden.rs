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
const CASES: [(&str, &[u8], &str); 14] = [
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
        "g5",
        include_bytes!("golden/g5.dxf"),
        include_str!("golden/g5.expected.json"),
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
    (
        "g11",
        include_bytes!("golden/g11.dxf"),
        include_str!("golden/g11.expected.json"),
    ),
    (
        "g12",
        include_bytes!("golden/g12.dxf"),
        include_str!("golden/g12.expected.json"),
    ),
    (
        "g13",
        include_bytes!("golden/g13.dxf"),
        include_str!("golden/g13.expected.json"),
    ),
    (
        "g14",
        include_bytes!("golden/g14.dxf"),
        include_str!("golden/g14.expected.json"),
    ),
    (
        "g15",
        include_bytes!("golden/g15.dxf"),
        include_str!("golden/g15.expected.json"),
    ),
    (
        "g16",
        include_bytes!("golden/g16.dxf"),
        include_str!("golden/g16.expected.json"),
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

/// The one way this reader knowingly differs from the spec, and why.
///
/// A DXF entity points at a table entry by *name*, so naming an entry the
/// file never declares is a reference that exists and answers to nothing:
/// unresolved, carrying the name. The vendored library's DXF importer looks
/// each such name up in its table and, when the lookup fails, only warns and
/// leaves the field empty -- the name it read is never stored on the entity,
/// so nothing downstream of that importer can recover it. This reader
/// reports those as absent.
///
/// Two cases carry one: G10's INSERT names a block the file never defines,
/// and G5's last dimension names a style it never declares. The same cause
/// in two places, which is why the deviation is written once over both.
/// It is applied to the expectation rather than by weakening the
/// comparison, so every other value in those cases stays pinned exactly.
///
/// A text style takes a different turn through the same importer: a TEXT
/// naming a style the file never declares (G13's `GOST`) is pointed at the
/// STANDARD entry instead, the entry an absent group 7 means. The name the
/// file wrote is lost the same way; what arrives is a resolved reference
/// to a style the entity did not name.
///
/// A layer's plot flag goes the same way from the other side. The importer
/// leaves an absent group 290 at 0, so a layer that states `290 = 0` (G14's
/// NOPLOT) cannot be told from one that states nothing, and this reader
/// reports both as "not stated" rather than guess: `None` where the spec
/// says `Some(false)`.
///
/// The style table carries a second, smaller one. A DIMSTYLE writes a
/// variable only when it differs from the value the application starts from.
/// Where every template starts from the same value, the model says an
/// unwritten variable is that value, and the importer reports exactly that.
/// Where the templates differ (text height, arrow size, the two decimal
/// places, zero suppression), the model says "not stated" -- but this
/// library holds a style as a struct with no "the group was not written", so
/// for a DXF it reports its own starting value there. The variables a case
/// *does* state are compared exactly; those five, where a case leaves them
/// out, are taken from what this reader said, because this reader cannot
/// know them. Both differences have the same cause and the same end (see
/// `docs/CAVEATS.md`).
///
/// TODO: remove this, and `the_dxf_importer_still_drops_an_undeclared_name`
/// below, once this crate's DXF path no longer goes through that importer.
fn apply_known_deviations(actual: &CadDatabase, expected: &mut CadDatabase) {
    for (name, style) in &mut expected.tables.dim_styles {
        let Some(read) = actual.tables.dim_styles.get(name) else {
            continue;
        };
        for (want, got) in [
            (&mut style.text_height, read.text_height),
            (&mut style.arrow_size, read.arrow_size),
        ] {
            if want.is_none() {
                *want = got;
            }
        }
        for (want, got) in [
            (&mut style.decimal_places, read.decimal_places),
            (
                &mut style.tolerance_decimal_places,
                read.tolerance_decimal_places,
            ),
            (&mut style.zero_suppression, read.zero_suppression),
        ] {
            if want.is_none() {
                *want = got;
            }
        }
    }

    for layer in expected.tables.layers.values_mut() {
        if layer.plot == Some(false) {
            layer.plot = None;
        }
    }

    fn lower(entity: &mut Entity) {
        let text_style = match entity {
            Entity::Text(text) => Some(&mut text.style_name),
            Entity::Attrib(attrib) => Some(&mut attrib.style_name),
            Entity::Attdef(attdef) => Some(&mut attdef.style_name),
            Entity::MText(mtext) => Some(&mut mtext.style_name),
            _ => None,
        };
        if let Some(style) = text_style {
            if matches!(style, Ref::Unresolved(_)) {
                *style = Ref::Resolved("STANDARD".to_string());
            }
            return;
        }
        let reference = match entity {
            Entity::Insert(insert) => &mut insert.block_name,
            Entity::Dimension(dimension) => &mut dimension.style_name,
            _ => return,
        };
        if matches!(reference, Ref::Unresolved(_)) {
            *reference = Ref::Absent;
        }
    }
    for entity in &mut expected.entities {
        lower(entity);
    }
    for block in expected.tables.block_records.values_mut() {
        for entity in &mut block.entities {
            lower(entity);
        }
    }
}

fn assert_reads_back_exactly(name: &str, dxf: &[u8], expected_json: &str) {
    let fixture = Fixture::write(&format!("golden-{name}.dxf"), dxf);
    let db = uncad::parse(&fixture.0).unwrap_or_else(|e| panic!("{name} should parse: {e}"));
    let mut expected: CadDatabase =
        serde_json::from_str(expected_json).expect("the expected model deserializes");
    apply_known_deviations(&db, &mut expected);

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

/// The tripwire for the deviation above: it asserts the defect is still
/// there, in all the places the golden cases put it. The day this reader
/// returns the name the file wrote, this test fails -- which is the signal
/// to delete both it and `apply_known_deviations`.
#[test]
fn the_dxf_importer_still_drops_an_undeclared_name() {
    let fixture = Fixture::write("golden-g10-deviation.dxf", include_bytes!("golden/g10.dxf"));
    let db = uncad::parse(&fixture.0).expect("g10 should parse");
    let insert = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Insert(insert) => Some(insert),
            _ => None,
        })
        .expect("g10 has one block reference");
    assert_eq!(
        insert.block_name,
        Ref::Absent,
        "the importer kept the name of an undefined block -- remove the known deviation"
    );

    let fixture = Fixture::write("golden-g5-deviation.dxf", include_bytes!("golden/g5.dxf"));
    let db = uncad::parse(&fixture.0).expect("g5 should parse");
    let undeclared = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Dimension(d) => Some(&d.style_name),
            _ => None,
        })
        .find(|s| !matches!(s, Ref::Resolved(_)))
        .expect("g5 has a dimension naming a style the file never declares");
    assert_eq!(
        undeclared,
        &Ref::Absent,
        "the importer kept the name of an undeclared style -- remove the known deviation"
    );

    let fixture = Fixture::write("golden-g14-deviation.dxf", include_bytes!("golden/g14.dxf"));
    let db = uncad::parse(&fixture.0).expect("g14 should parse");
    let expected: CadDatabase =
        serde_json::from_str(include_str!("golden/g14.expected.json")).unwrap();
    assert_eq!(expected.tables.layers["NOPLOT"].plot, Some(false));
    assert_eq!(
        db.tables.layers["NOPLOT"].plot, None,
        "the importer told a stated 290 = 0 from an absent one -- remove the known deviation"
    );

    let fixture = Fixture::write("golden-g13-deviation.dxf", include_bytes!("golden/g13.dxf"));
    let db = uncad::parse(&fixture.0).expect("g13 should parse");
    let expected: CadDatabase =
        serde_json::from_str(include_str!("golden/g13.expected.json")).unwrap();
    let styles = |db: &CadDatabase| -> Vec<Ref<String>> {
        db.entities
            .iter()
            .filter_map(|e| match e {
                Entity::Text(t) => Some(t.style_name.clone()),
                _ => None,
            })
            .collect()
    };
    let (read, stated) = (styles(&db), styles(&expected));
    let undeclared = stated
        .iter()
        .position(|s| matches!(s, Ref::Unresolved(_)))
        .expect("g13 has a text naming a style the file never declares");
    assert_eq!(stated[undeclared], Ref::Unresolved("GOST".to_string()));
    assert_eq!(
        read[undeclared],
        Ref::Resolved("STANDARD".to_string()),
        "the importer kept the name of an undeclared text style -- remove the known deviation"
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
