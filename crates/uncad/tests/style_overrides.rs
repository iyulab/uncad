//! A dimension or leader that sets style variables for itself: the `DSTYLE`
//! list in its extended data under `ACAD`, read as the file states it --
//! through the DWG decoder and through the DXF importer alike. The expected
//! values are the files' own (the DXF twins' text, and a second DWG reader
//! agrees on the DWG ones).

use uncad::model::{Entity, OverrideValue, StyleOverride};

const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

/// The `style_overrides` of the entity with `handle`.
fn overrides_of(file: &str, handle: &str) -> Option<Vec<StyleOverride>> {
    let db = uncad::parse(format!("{TEST_DATA}{file}")).expect("reads");
    db.entities.into_iter().find_map(|e| {
        (e.common().source_handle.name() == handle).then(|| match e {
            Entity::Dimension(d) => d.style_overrides,
            Entity::Leader(l) => l.style_overrides,
            other => panic!("{handle} is a {}", other.type_name()),
        })?
    })
}

fn pairs(list: &[StyleOverride]) -> Vec<(u16, OverrideValue)> {
    list.iter().map(|o| (o.variable, o.value.clone())).collect()
}

#[test]
fn a_dwg_leader_carries_its_arrow_size_scale_gap_and_arrow_block() {
    let list = overrides_of("2000/Leader.dwg", "72E").expect("the reader looked");
    assert_eq!(
        pairs(&list),
        [
            (40, OverrideValue::Real(0.0)),
            (41, OverrideValue::Real(0.24)),
            (341, OverrideValue::Handle("77A".to_string())),
            (147, OverrideValue::Real(0.09)),
            (77, OverrideValue::Integer(0)),
        ]
    );
}

#[test]
fn a_string_value_is_read_as_text() {
    let list = overrides_of("r14/Leader.dwg", "72E").expect("the reader looked");
    assert_eq!(list[0].variable, 5);
    assert_eq!(list[0].value, OverrideValue::Text("OPEN30".to_string()));
}

#[test]
fn the_dxf_importer_keeps_the_list_too() {
    let list = overrides_of("example_r13.dxf", "43B").expect("the reader looked");
    assert_eq!(
        pairs(&list),
        [
            (79, OverrideValue::Integer(2)),
            (179, OverrideValue::Integer(2))
        ]
    );
}

#[test]
fn a_dimension_without_a_list_has_an_empty_one() {
    assert_eq!(overrides_of("example_r13.dwg", "43B"), Some(Vec::new()));
}
