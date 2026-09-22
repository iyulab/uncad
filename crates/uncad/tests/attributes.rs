//! Attribute definitions in a block and attribute values on an INSERT must
//! all come back -- every one of them, not the last one.
//!
//! The fixture is an R2000 ASCII DXF written by the test itself (owner
//! handles and all), so this file does not depend on the corpus checkout.
//! Each assertion pairs a count that must hold with the value that proves
//! the right entities were read, so "three of something" cannot pass by
//! accident.

use std::fs;
use std::path::PathBuf;
use uncad::model::Ref;
use uncad::Entity;

/// Three ATTDEFs and a LINE inside a block, an INSERT of that block with
/// three ATTRIBs and a SEQEND. Owner handles: table records point at their
/// table, block entities at their BLOCK_RECORD, top-level entities at the
/// model-space record, the ATTRIBs and the SEQEND at the INSERT.
fn dxf_with_attributes() -> String {
    let pairs: &[(u16, &str)] = &[
        (0, "SECTION"),
        (2, "HEADER"),
        (9, "$ACADVER"),
        (1, "AC1015"),
        (9, "$HANDSEED"),
        (5, "200"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "TABLES"),
        (0, "TABLE"),
        (2, "LAYER"),
        (5, "1"),
        (100, "AcDbSymbolTable"),
        (70, "1"),
        (0, "LAYER"),
        (5, "2"),
        (330, "1"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbLayerTableRecord"),
        (2, "0"),
        (70, "0"),
        (62, "7"),
        (6, "CONTINUOUS"),
        (0, "ENDTAB"),
        (0, "TABLE"),
        (2, "BLOCK_RECORD"),
        (5, "3"),
        (100, "AcDbSymbolTable"),
        (70, "3"),
        (0, "BLOCK_RECORD"),
        (5, "A"),
        (330, "3"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbBlockTableRecord"),
        (2, "*Model_Space"),
        (0, "BLOCK_RECORD"),
        (5, "B"),
        (330, "3"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbBlockTableRecord"),
        (2, "*Paper_Space"),
        (0, "BLOCK_RECORD"),
        (5, "C"),
        (330, "3"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbBlockTableRecord"),
        (2, "TITLE"),
        (0, "ENDTAB"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "BLOCKS"),
        (0, "BLOCK"),
        (5, "100"),
        (330, "A"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockBegin"),
        (2, "*Model_Space"),
        (70, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (3, "*Model_Space"),
        (1, ""),
        (0, "ENDBLK"),
        (5, "101"),
        (330, "A"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockEnd"),
        (0, "BLOCK"),
        (5, "102"),
        (330, "B"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockBegin"),
        (2, "*Paper_Space"),
        (70, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (3, "*Paper_Space"),
        (1, ""),
        (0, "ENDBLK"),
        (5, "103"),
        (330, "B"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockEnd"),
        (0, "BLOCK"),
        (5, "104"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockBegin"),
        (2, "TITLE"),
        (70, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (3, "TITLE"),
        (1, ""),
        (0, "ATTDEF"),
        (5, "105"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "1.0"),
        (20, "3.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "-"),
        (100, "AcDbAttributeDefinition"),
        (3, "Number"),
        (2, "NUMBER"),
        (70, "0"),
        (0, "LINE"),
        (5, "106"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbLine"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (11, "10.0"),
        (21, "0.0"),
        (31, "0.0"),
        (0, "ATTDEF"),
        (5, "107"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "1.0"),
        (20, "2.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "-"),
        (100, "AcDbAttributeDefinition"),
        (3, "Revision"),
        (2, "REV"),
        (70, "0"),
        (0, "ATTDEF"),
        (5, "108"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "1.0"),
        (20, "1.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "-"),
        (100, "AcDbAttributeDefinition"),
        (3, "Material"),
        (2, "MATERIAL"),
        (70, "0"),
        (0, "ENDBLK"),
        (5, "109"),
        (330, "C"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbBlockEnd"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "INSERT"),
        (5, "120"),
        (330, "A"),
        (100, "AcDbEntity"),
        (8, "0"),
        (66, "1"),
        (100, "AcDbBlockReference"),
        (2, "TITLE"),
        (10, "50.0"),
        (20, "50.0"),
        (30, "0.0"),
        (41, "1.0"),
        (42, "1.0"),
        (43, "1.0"),
        (50, "0.0"),
        (0, "ATTRIB"),
        (5, "121"),
        (330, "120"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "51.0"),
        (20, "53.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "A-100"),
        (100, "AcDbAttribute"),
        (2, "NUMBER"),
        (70, "0"),
        (0, "ATTRIB"),
        (5, "122"),
        (330, "120"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "51.0"),
        (20, "52.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "B"),
        (100, "AcDbAttribute"),
        (2, "REV"),
        (70, "0"),
        (0, "ATTRIB"),
        (5, "123"),
        (330, "120"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbText"),
        (10, "51.0"),
        (20, "51.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "STEEL"),
        (100, "AcDbAttribute"),
        (2, "MATERIAL"),
        (70, "0"),
        (0, "SEQEND"),
        (5, "124"),
        (330, "120"),
        (100, "AcDbEntity"),
        (8, "0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    pairs
        .iter()
        .map(|(code, value)| format!("{code:>3}\n{value}\n"))
        .collect()
}

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct Fixture(PathBuf);

impl Fixture {
    fn write(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name));
        fs::write(&path, dxf_with_attributes()).expect("the temp dir should be writable");
        Fixture(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn every_attribute_definition_in_a_block_is_read() {
    let fixture = Fixture::write("attributes-attdef.dxf");
    let db = uncad::parse(&fixture.0).expect("the DXF should parse");

    let block = &db.tables.block_records["TITLE"];
    let kinds: Vec<&str> = block.entities.iter().map(|e| e.type_name()).collect();
    assert_eq!(
        kinds,
        ["ATTDEF", "LINE", "ATTDEF", "ATTDEF"],
        "file order, nothing skipped"
    );
    let handles: Vec<&str> = block
        .entities
        .iter()
        .map(|e| e.common().handle.as_str())
        .collect();
    assert_eq!(handles, ["105", "106", "107", "108"]);
    let defaults: Vec<&str> = block
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Attdef(a) => Some(a.default_value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(defaults, ["-", "-", "-"]);
}

#[test]
fn every_attribute_value_on_an_insert_is_read() {
    let fixture = Fixture::write("attributes-attrib.dxf");
    let db = uncad::parse(&fixture.0).expect("the DXF should parse");

    let insert = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Insert(i) => Some(i),
            _ => None,
        })
        .expect("the INSERT is read");
    assert_eq!(insert.block_name, Ref::Resolved("TITLE".to_string()));
    let values: Vec<(&str, &str)> = insert
        .attribs
        .iter()
        .map(|a| (a.common.handle.as_str(), a.text.as_str()))
        .collect();
    assert_eq!(
        values,
        [("121", "A-100"), ("122", "B"), ("123", "STEEL")],
        "all three, in file order"
    );

    // The same three are listed at the top level after their INSERT, in
    // file order, which is what a renderer draws; the SEQEND is not an
    // entity of the drawing.
    let top: Vec<&str> = db.entities.iter().map(|e| e.type_name()).collect();
    assert_eq!(top, ["INSERT", "ATTRIB", "ATTRIB", "ATTRIB"]);
    assert!(db.read_diagnostics.is_clean());
}
