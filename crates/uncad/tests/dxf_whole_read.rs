//! A DXF reads whole. `example_r14.dxf` is the file the old read path lost
//! all but one entity of: its ENTITIES section holds 68 top-level records,
//! and the drawing holds 70 -- the records and the paper-space content the
//! file keeps in BLOCKS.

const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

#[test]
fn example_r14_dxf_reads_every_entity_it_holds() {
    let db = uncad::parse(format!("{TEST_DATA}example_r14.dxf")).expect("reads");
    assert_eq!(db.entities.len(), 70);
    assert!(db.read_diagnostics.is_clean(), "{:?}", db.read_diagnostics);
}
