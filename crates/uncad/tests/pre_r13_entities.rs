//! A DWG older than R13 holds what its DXF twin holds: not the entities the
//! file has erased (making a block erases the originals of what went into it,
//! and they stay in the entity section), and an INSERT's ATTRIBs -- the
//! records that follow it, as the format keeps them -- belong to the INSERT.

use uncad::{CadDatabase, Entity};

fn corpus(path: &str) -> String {
    format!(
        "{}/../../lib/libredwg/test/test-data/{path}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn names(db: &CadDatabase) -> Vec<&str> {
    db.entities.iter().map(Entity::type_name).collect()
}

#[test]
fn a_line_moved_into_a_block_is_not_also_in_model_space() {
    for file in ["r10/tmp_line.dwg", "r10/tmp_line.dxf"] {
        let db = uncad::parse(corpus(file)).expect("the corpus drawing should parse");
        assert_eq!(names(&db), ["INSERT"], "{file}");
        let block = db
            .tables
            .block_records
            .get("BLOCK1")
            .unwrap_or_else(|| panic!("{file}: BLOCK1"));
        let inside: Vec<_> = block.entities.iter().map(Entity::type_name).collect();
        assert_eq!(inside, ["LINE"], "{file}");
    }
}

#[test]
fn an_inserts_attributes_are_its_own_before_r13() {
    for version in ["r2.6", "r9", "r10", "r11"] {
        let file = if version == "r11" {
            format!("{version}/entities-2d.dwg")
        } else {
            format!("{version}/entities.dwg")
        };
        let db = uncad::parse(corpus(&file)).expect("the corpus drawing should parse");
        let tags: Vec<Vec<String>> = db
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Insert(i) if i.block_name.name() == "BLOCK2" => {
                    Some(i.attribs.iter().map(|a| a.tag.clone()).collect())
                }
                _ => None,
            })
            .collect();
        assert_eq!(tags, [vec!["ATTR2".to_string()]], "{file}");
    }
}
