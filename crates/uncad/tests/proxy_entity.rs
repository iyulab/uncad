//! A proxy entity -- an object whose class the writing application did not
//! know how to export, saved as `ACAD_PROXY_ENTITY` -- is kept as what it
//! is: an `Entity::Unknown` carrying that type name and its handle. It is
//! neither dropped (the drawing would look complete without it) nor read as
//! some other type (its geometry is not something the file states).

use uncad::model::{Entity, Ref};

const LEADER_R14_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/r14/Leader.dxf"
);

#[test]
fn a_proxy_entity_is_kept_as_unknown_under_its_own_name() {
    let db = uncad::parse(LEADER_R14_DXF).expect("reads");
    let proxies: Vec<&Entity> = db
        .entities
        .iter()
        .filter(|e| e.type_name() == "ACAD_PROXY_ENTITY")
        .collect();
    assert_eq!(proxies.len(), 1, "the file holds one proxy entity");
    let Entity::Unknown { common, type_name } = proxies[0] else {
        panic!("a proxy is not read as a known type: {:?}", proxies[0]);
    };
    assert_eq!(type_name, "ACAD_PROXY_ENTITY");
    assert_eq!(common.source_handle, Ref::Resolved("732".to_string()));
}
