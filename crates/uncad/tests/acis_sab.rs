//! Regression guard for a parse-time mutation of SAB-encoded (ACIS
//! BinaryFile, `version == 2`) 3DSOLID/REGION entities.
//!
//! `acis.rs` used to call LibreDWG's `dwg_convert_SAB_to_SAT1` on the *live*
//! entity to get SAT text for the wireframe. That function converts in place
//! (version 2 -> 1, plaintext SAT into `encr_sat_data`, `acis_data` left as
//! SAB bytes). `parse()` reads every solid twice -- `convert_entities` for
//! model space, then `convert_tables` for the owning block record -- so the
//! second read saw a `version == 1` entity, parsed its binary SAB as SAT text,
//! and got no wireframe: the solid had edges in `entities` and none in
//! `tables.block_records`. (While the write API existed, the same mutation
//! also corrupted every later DXF/DWG write.) Fixed by converting on a copy
//! (`uncad_3dsolid_sab_to_sat_text` in `libredwg-sys`'s shim); see
//! `docs/CAVEATS.md`, "3DSOLID SAB conversion".
//!
//! Fixture: `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`, the one file
//! in the bundled corpus whose solids are stored as SAB (58 of them).
//!
//! The assertion is reference-free: whatever wireframe a solid gets in the
//! drawing's entity list, its copy inside the model-space block record must
//! get the very same one.

use std::collections::BTreeMap;

const SAB_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg"
);

/// reference ID -> wireframe edges, for every 3DSOLID/REGION in `entities`.
fn wireframes_by_handle(
    entities: &[uncad::Entity],
) -> BTreeMap<u64, &[[uncad::model::Point3D; 2]]> {
    entities
        .iter()
        .filter_map(|e| match e {
            uncad::Entity::Solid3D(s) | uncad::Entity::Region(s) => {
                Some((s.common.id.value(), s.wireframe_edges.as_slice()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn sab_solids_get_the_same_wireframe_in_entities_and_in_their_block_record() {
    let db = uncad::parse(SAB_DWG).expect("the SAB fixture should parse");

    let in_entities = wireframes_by_handle(&db.entities);
    let extracted = in_entities.values().filter(|w| !w.is_empty()).count();
    assert!(
        extracted > 0,
        "the fixture should yield at least one SAB solid wireframe, or this test guards nothing"
    );

    let model_space = db
        .tables
        .block_records
        .values()
        .find(|r| r.name.eq_ignore_ascii_case("*MODEL_SPACE"))
        .expect("the drawing should have a model-space block record");
    let in_block = wireframes_by_handle(&model_space.entities);

    assert_eq!(
        in_block.len(),
        in_entities.len(),
        "model space should list the same solids in both walks"
    );
    for (handle, edges) in &in_entities {
        let block_edges = in_block
            .get(handle)
            .unwrap_or_else(|| panic!("solid {handle} is missing from the block record"));
        assert_eq!(
            block_edges, edges,
            "solid {handle}: the block-record walk extracted a different wireframe than the \
             entity walk -- the first extraction mutated the entity the second one read"
        );
    }
}

/// The converter writes a body's doubles to full precision: the solid of
/// `example_2010.dwg` (a SAB body) reads its first vertex to every digit its
/// DXF twin's SAT text states, not rounded to six significant digits
/// (4235.41, 14168.8).
#[test]
fn a_sab_body_keeps_every_digit_of_its_coordinates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/example_2010.dwg"
    );
    let db = uncad::parse(path).expect("example_2010.dwg should parse");
    let solid = db
        .entities
        .iter()
        .find_map(|e| match e {
            uncad::Entity::Solid3D(s) => Some(s),
            _ => None,
        })
        .expect("example_2010.dwg holds a 3DSOLID");
    let first = solid.wireframe_edges[0][0];
    assert!(
        (first.x - 4235.406760796846).abs() < 1e-9 && (first.y - 14168.837206813574).abs() < 1e-9,
        "{first:?}"
    );
}
