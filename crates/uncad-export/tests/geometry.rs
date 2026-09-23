//! The package's polyline arithmetic against a real file: the corpus's
//! `example_2000.dwg` has a revision cloud -- an LWPOLYLINE whose every
//! segment is a 110-degree arc -- whose length and area are checked here
//! against a computation that does not go through this crate.

use uncad::Entity;
use uncad_export::geom;
use uncad_model::model::{LwPolylineEntity, PolylineVertex};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

/// The small revision cloud in model space (the other one has 3635
/// vertices), by the handle the file gives it.
const REVCLOUD: &str = "156";

fn lwpolyline<'a>(db: &'a uncad::CadDatabase, handle: &str) -> &'a LwPolylineEntity {
    db.entities
        .iter()
        .find_map(|e| match e {
            Entity::LwPolyline(p)
                if p.common.source_handle.resolved().map(String::as_str) == Some(handle) =>
            {
                Some(p)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("LWPOLYLINE {handle} is in the drawing"))
}

#[test]
fn the_revision_clouds_length_is_the_sum_of_its_arcs() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD);
    assert_eq!(cloud.vertices.len(), 25);
    assert!(cloud.closed);
    // Independently: chord c and bulge b give r = c (1 + b^2) / (4 |b|) and
    // the arc length r * |4 atan b|, summed over the 25 closing segments.
    let n = cloud.vertices.len();
    let mut expected = 0.0;
    for i in 0..n {
        let (a, b) = (cloud.vertices[i].point, cloud.vertices[(i + 1) % n].point);
        let chord = (b.x - a.x).hypot(b.y - a.y);
        let bulge = cloud.vertices[i].bulge.abs();
        let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge);
        expected += radius * 4.0 * bulge.atan();
    }
    let length = geom::polyline_length(&cloud.vertices, true);
    assert!(
        (length - expected).abs() < 1e-6 * expected,
        "{length} vs {expected}"
    );
    // Every segment is a 110-degree arc, so the ratio of arc length to
    // chord is the same for all of them: theta / (2 sin(theta / 2)).
    let chords: Vec<PolylineVertex> = cloud
        .vertices
        .iter()
        .map(|v| PolylineVertex::straight(v.point))
        .collect();
    let chord_length = geom::polyline_length(&chords, true);
    let theta = 110f64.to_radians();
    let ratio = theta / (2.0 * (theta / 2.0).sin());
    assert!(
        (length / chord_length - ratio).abs() < 1e-9,
        "{length} / {chord_length} vs {ratio}"
    );
    // A cloud bulging outward encloses more than its vertex polygon.
    let area = geom::polyline_area(&cloud.vertices, true).expect("closed outline");
    let polygon = geom::polyline_area(&chords, true).expect("closed outline");
    assert!(area > polygon, "{area} vs polygon {polygon}");
}
