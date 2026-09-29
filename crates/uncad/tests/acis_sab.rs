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
//! - A body in the file's data storage (R2013 on) is not read: LibreDWG
//!   attaches those bodies to solids in the order it finds them rather than by
//!   the handle each names, and misses R2018's altogether. Such an entity is
//!   one this crate does not read, not an empty solid and not a solid with
//!   another solid's edges.
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

/// R2013 and later keep a solid's body in the file's data storage. LibreDWG
/// attaches those bodies in the order it finds them, not by the handle each
/// names: in `example_2013.dwg` the 3DSOLID got a REGION's 4 edges and a REGION
/// the 3DSOLID's 18 (its DXF twin says which is which). In `example_2018.dwg`
/// it attaches none -- an R2018 body opens with `ASM BinaryFile4`, and the
/// search looks for `ACIS BinaryFile`. Every such solid is not read, rather
/// than read with another solid's edges or with none.
#[test]
fn a_body_in_the_data_storage_is_not_read() {
    for (drawing, solids) in [
        (
            "example_2013.dwg",
            [(0x2E1, "3DSOLID"), (0x37D, "REGION"), (0x176, "REGION")],
        ),
        (
            "example_2018.dwg",
            [(0x2E1, "3DSOLID"), (0x37D, "REGION"), (0x176, "REGION")],
        ),
    ] {
        let path = format!(
            "{}/../../lib/libredwg/test/test-data/{drawing}",
            env!("CARGO_MANIFEST_DIR")
        );
        let db = uncad::parse(&path).expect("the drawing should parse");
        for (handle, name) in solids {
            let e = db
                .entities
                .iter()
                .find(|e| e.common().id.value() == handle)
                .unwrap_or_else(|| panic!("{drawing} holds {name} {handle:X}"));
            assert!(
                matches!(e, uncad::Entity::Unknown { type_name, .. } if type_name == name),
                "{drawing} {handle:X}: {e:?}"
            );
        }
    }
}
