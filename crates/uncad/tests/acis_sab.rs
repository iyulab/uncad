//! 3DSOLID/REGION bodies stored as SAB (ACIS BinaryFile, `version == 2`),
//! read from their bytes by `uncad_model::acis::wireframe_sab`.
//!
//! - Every SAB body in `2007/ATMOS-DC22S.dwg` (58 solids) reads in full. The
//!   SAB-to-SAT conversion LibreDWG offers dropped records it did not know
//!   without renumbering the rest, which left 62 of the file's 116 solid
//!   copies unread.
//! - Reading a body leaves the entity as it was: `parse()` reads every solid
//!   twice (the entity list, then the owning block record), and both walks
//!   must get the same wireframe. (An in-place conversion once made the
//!   second walk read binary SAB as SAT text and get nothing.)
//! - A body is read to every digit its file states.
//! - A body in the file's data storage (R2013 on) goes to the entity whose
//!   handle its record names: every solid of `example_2013.dwg` and
//!   `example_2018.dwg` reads the edges its DXF twin gives it.
//!
//! The assertions against the entity list and the block record are
//! reference-free: whatever wireframe a solid gets in one, it must get in the
//! other.

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

/// A SAB body's doubles are read to full precision: the solid of
/// `example_2010.dwg` reads its first vertex to every digit its DXF twin's
/// SAT text states, not rounded to six significant digits (4235.41, 14168.8)
/// as a SAB-to-SAT text conversion printing `%g` would.
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

/// Every SAB solid of the fixture reads, with no edge skipped -- in the entity
/// list and in the model-space block record alike.
#[test]
fn every_sab_body_of_the_fixture_reads_in_full() {
    let db = uncad::parse(SAB_DWG).expect("the SAB fixture should parse");
    let solids: Vec<&uncad::model::Solid3DEntity> = db
        .all_entities()
        .filter_map(|e| match e {
            uncad::Entity::Solid3D(s) | uncad::Entity::Region(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(
        solids.len(),
        116,
        "58 solids, each in the entity list and its block record"
    );
    for s in &solids {
        assert_eq!(s.skipped_edges, 0, "solid {:?}", s.common.id);
    }
    // One solid (two copies) is a torus: a single face with no edge record,
    // so it has no wireframe to draw.
    let without_edges = solids
        .iter()
        .filter(|s| s.wireframe_edges.is_empty())
        .count();
    assert_eq!(without_edges, 2);
}

/// R2013 and later keep a solid's body in the file's data storage, one record
/// per body under the handle of the entity it belongs to. The records are not
/// written in entity order (`example_2013.dwg`: REGION 176, then the thumbnail,
/// REGION 37D, 3DSOLID 2E1; `example_2018.dwg`: another order), and R2018's
/// bodies open with `ASM BinaryFile4` rather than `ACIS BinaryFile`. Each
/// solid reads the edges its DXF twin gives it: 3DSOLID 2E1 18, the two
/// REGIONs 4 each -- the vendored LibreDWG attaches the bodies by handle
/// (upstream attached them in the order it found them, which swapped them
/// here, and found none in R2018).
#[test]
fn a_body_in_the_data_storage_goes_to_the_entity_its_record_names() {
    for drawing in ["example_2013.dwg", "example_2018.dwg"] {
        let path = format!(
            "{}/../../lib/libredwg/test/test-data/{drawing}",
            env!("CARGO_MANIFEST_DIR")
        );
        let db = uncad::parse(&path).expect("the drawing should parse");
        for (handle, name, edges) in [
            (0x2E1, "3DSOLID", 18),
            (0x37D, "REGION", 4),
            (0x176, "REGION", 4),
        ] {
            let e = db
                .entities
                .iter()
                .find(|e| e.common().id.value() == handle)
                .unwrap_or_else(|| panic!("{drawing} holds {name} {handle:X}"));
            let (uncad::Entity::Solid3D(s) | uncad::Entity::Region(s)) = e else {
                panic!("{drawing} {handle:X}: {e:?}");
            };
            assert_eq!(e.type_name(), name, "{drawing} {handle:X}");
            assert_eq!(
                (s.wireframe_edges.len(), s.skipped_edges),
                (edges, 0),
                "{drawing} {name} {handle:X}"
            );
        }
    }
}
