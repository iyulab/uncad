//! A block that states it is an external reference carries the referenced
//! drawing's path; its content is that drawing's, which is not in the file.

use uncad::tables::ExternalReference;

const GH44: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2013/gh44-error.dwg"
);

#[test]
fn an_external_reference_block_carries_its_path() {
    let db = uncad::parse(GH44).expect("the corpus DWG parses");
    let external: Vec<(&String, &ExternalReference)> = db
        .tables
        .block_records
        .iter()
        .filter_map(|(name, b)| b.external_reference.as_ref().map(|x| (name, x)))
        .collect();
    // Two architectural drawings, attached (not overlaid), neither loaded:
    // the blocks hold no entities.
    assert_eq!(external.len(), 2, "{external:?}");
    for (name, x) in external {
        assert!(!x.overlay, "{name}");
        assert!(
            x.path.ends_with(&format!("{name}.dwg")),
            "{name}: the path names the drawing ({})",
            x.path
        );
        assert!(db.tables.block_records[name].entities.is_empty(), "{name}");
    }
}
