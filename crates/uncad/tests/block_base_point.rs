//! A block reference puts its block's base point on its insertion point.
//!
//! The drawings are the LibreDWG corpus's `r2.10/entities`, built by the
//! AutoCAD script beside them (`entities.scr`): `line 6,1 7,2`, then
//! `block BLOCK1 6,1` around that line, then `insert BLOCK1 6,1 0.5 0.5 30`;
//! and `block BLOCK2 1,2`. The script is the oracle: the base points are
//! (6, 1) and (1, 2), and the placed line starts on the insertion point,
//! whatever the scale and rotation.

use std::path::Path;
use uncad::model::{Entity, Point2D, Ref};

fn corpus(name: &str) -> String {
    format!(
        "{}/../../lib/libredwg/test/test-data/r2.10/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn base(db: &uncad::CadDatabase, block: &str) -> (f64, f64) {
    let b = &db.tables.block_records[block].base_point;
    (b.x, b.y)
}

#[test]
fn the_base_points_are_read_from_a_dwg_and_a_dxf() {
    for file in ["entities.dwg", "entities.dxf"] {
        let db = uncad::parse(Path::new(&corpus(file))).expect("the corpus file parses");
        assert_eq!(base(&db, "BLOCK1"), (6.0, 1.0), "{file}");
        assert_eq!(base(&db, "BLOCK2"), (1.0, 2.0), "{file}");
    }
}

#[test]
fn the_placed_line_starts_on_the_insertion_point() {
    let db = uncad::parse(Path::new(&corpus("entities.dwg"))).expect("the corpus file parses");
    let block = &db.tables.block_records["BLOCK1"];
    let insert = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Insert(i) if i.block_name == Ref::Resolved("BLOCK1".to_string()) => Some(i),
            _ => None,
        })
        .expect("the script inserts BLOCK1");
    let line = block
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Line(l) => Some(l),
            _ => None,
        })
        .expect("BLOCK1 holds the line");
    let t = insert
        .world_transform(block.base_point)
        .expect("a flat plane");
    let start = t.apply(Point2D {
        x: line.start_point.x,
        y: line.start_point.y,
    });
    assert!(
        (start.x - 6.0).abs() < 1e-9 && (start.y - 1.0).abs() < 1e-9,
        "{start:?}"
    );
}
