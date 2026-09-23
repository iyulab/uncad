//! Layer colors read from a DWG record, checked against what the same
//! drawing saved as DXF states for them.

const DYNBLOCKS_2018_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2018/Dynblocks.dwg"
);

#[test]
fn an_index_color_is_its_number_even_when_it_matches_a_palette_rgb() {
    // Stored as index color 104, whose value read as an RGB color matches
    // palette entry 176. The DXF twin writes 62 = 104.
    let db = uncad::parse(DYNBLOCKS_2018_DWG).expect("the corpus drawing should parse");
    assert_eq!(db.tables.layers["Grass_0.20"].color_index, 104);
    assert_eq!(db.tables.layers["0"].color_index, 7);
}
