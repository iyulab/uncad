//! Dimension points read from a DWG record, checked against what the same
//! drawing saved as DXF states for them.

use uncad::model::{DimensionKind, Point3D, Ref};
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

#[test]
fn an_arc_length_dimension_without_a_leader_still_carries_its_group_16() {
    // The record stores the first leader point (group 16) whether or not the
    // dimension has a leader, and this one has none (group 71 is 0). The
    // DXF twin writes the same point, which here is the first extension
    // line's origin.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("the corpus drawing should parse");
    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) if d.common.source_handle == Ref::Resolved("399".to_string()) => {
                Some(d)
            }
            _ => None,
        })
        .expect("the drawing's arc-length dimension");
    assert_eq!(dim.kind, Some(DimensionKind::ArcLength));
    let stated = Point3D {
        x: 5486.627993960748,
        y: 2529.600165964192,
        z: 0.0,
    };
    let arc = dim.points.arc.expect("group 16 is stated");
    assert!(
        (arc.x - stated.x).abs() < 1e-9 && (arc.y - stated.y).abs() < 1e-9 && arc.z == 0.0,
        "{arc:?}"
    );
}
