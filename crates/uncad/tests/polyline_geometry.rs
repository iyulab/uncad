//! Polyline bulges and the OCS transform (docs/VLM_EXPORT_DESIGN.md, P4)
//! against real files: the corpus's `example_2000.dwg` has two revision
//! clouds -- LWPOLYLINEs whose every segment is a 110-degree arc -- and its
//! DXF twin states each bulge in group 42, which is the ground truth here.
//! The project's own `mirrored_ocs_r2000.dxf` covers the world-coordinate
//! side in `tests/fixtures.rs`; this file checks what the renderer and the
//! JSON make of the same data.

use uncad::model::LwPolylineEntity;
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);
const MIRRORED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mirrored_ocs_r2000.dxf"
);

/// The small revision cloud in model space (the other one has 3635 vertices).
const REVCLOUD: &str = "156";

fn lwpolyline<'a>(db: &'a uncad::CadDatabase, handle: &str) -> &'a LwPolylineEntity {
    db.entities
        .iter()
        .find_map(|e| match e {
            Entity::LwPolyline(p) if p.common.handle == handle => Some(p),
            _ => None,
        })
        .unwrap_or_else(|| panic!("LWPOLYLINE {handle} is in the drawing"))
}

/// Group 42 of one LWPOLYLINE, read straight out of the DXF text.
fn bulges_from_dxf_text(handle: &str) -> Vec<f64> {
    let bytes = std::fs::read(EXAMPLE_2000_DXF).expect("corpus DXF is readable");
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i] == "0" && lines[i + 1] == "LWPOLYLINE" {
            let mut found = false;
            let mut bulges = Vec::new();
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                match lines[j] {
                    "5" if lines[j + 1] == handle => found = true,
                    "42" => bulges.push(lines[j + 1].parse::<f64>().expect("a bulge")),
                    _ => {}
                }
                j += 2;
            }
            if found {
                return bulges;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    panic!("LWPOLYLINE {handle} is in the DXF text");
}

#[test]
fn the_revision_cloud_reads_every_bulge_autocad_wrote() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD);
    let expected = bulges_from_dxf_text(REVCLOUD);
    assert_eq!(cloud.vertices.len(), 25);
    assert_eq!(cloud.bulges.len(), expected.len());
    for (got, want) in cloud.bulges.iter().zip(&expected) {
        assert!((got - want).abs() < 1e-9, "{got} vs DXF 42 {want}");
    }
    // Every arc of a revision cloud bulges the same way: 110 degrees,
    // clockwise (negative bulge) here.
    let sweep = 4.0 * cloud.bulges[0].atan();
    assert!(
        (sweep.to_degrees() + 110.0).abs() < 1e-6,
        "{}",
        sweep.to_degrees()
    );
    assert!(cloud.closed);
    assert!(cloud.extrusion.z > 0.0 && cloud.elevation == 0.0);
    assert!(cloud.widths.is_empty() && cloud.const_width == 0.0);
}

#[test]
fn the_revision_clouds_length_is_the_sum_of_its_arcs() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD);
    // Independently: chord c and bulge b give r = c (1 + b^2) / (4 |b|) and
    // the arc length r * |4 atan b|, summed over the 25 closing segments.
    let n = cloud.vertices.len();
    let mut expected = 0.0;
    for i in 0..n {
        let (a, b) = (cloud.vertices[i], cloud.vertices[(i + 1) % n]);
        let chord = (b.x - a.x).hypot(b.y - a.y);
        let bulge = cloud.bulges[i].abs();
        let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge);
        expected += radius * 4.0 * bulge.atan();
    }
    let length = cloud.length();
    assert!(
        (length - expected).abs() < 1e-6 * expected,
        "{length} vs {expected}"
    );
    // Every segment is a 110-degree arc, so the ratio of arc length to
    // chord is the same for all of them: theta / (2 sin(theta / 2)).
    let chords = uncad::geom::polyline_length(&cloud.vertices, &[], true);
    let theta = 110f64.to_radians();
    let ratio = theta / (2.0 * (theta / 2.0).sin());
    assert!(
        (length / chords - ratio).abs() < 1e-9,
        "{length} / {chords} vs {ratio}"
    );
    // A cloud bulging outward encloses more than its vertex polygon.
    let area = cloud.area().expect("closed outline");
    let polygon = uncad::geom::polyline_area(&cloud.vertices, &[], true);
    assert!(area > polygon, "{area} vs polygon {polygon}");
}

#[test]
fn an_open_polylines_area_ignores_the_bulge_on_its_last_vertex() {
    // AutoCAD keeps a bulge on the last vertex of an open polyline (after
    // BREAK or TRIM, say); it belongs to no segment. The right triangle
    // (0,0) -> (10,0) -> (10,10) closed by a straight segment for the area
    // is 10 * 10 / 2 = 50 whatever that bulge says; a semicircle (bulge 1)
    // on the closing chord would have added pi * (5 sqrt 2)^2 / 2 =
    // 78.54. Open, the length is the two drawn sides: 20.
    let mut p: LwPolylineEntity = {
        let db = uncad::parse(MIRRORED).expect("fixture must parse");
        lwpolyline(&db, "21").clone()
    };
    p.vertices = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
        .into_iter()
        .map(|(x, y)| uncad::model::Point2D { x, y })
        .collect();
    p.closed = false;
    for bulge in [1.0, -1.0, 669.19] {
        p.bulges = vec![0.0, 0.0, bulge];
        let area = p.area().expect("three vertices");
        assert!((area - 50.0).abs() < 1e-9, "bulge {bulge}: {area}");
        assert!(p.signed_area() > 0.0, "counter-clockwise");
        assert!((p.length() - 20.0).abs() < 1e-9);
    }
    // Flagged closed, the same bulge is the third, real segment.
    p.closed = true;
    p.bulges = vec![0.0, 0.0, 1.0];
    let area = p.area().unwrap();
    assert!(
        (area - (50.0 + std::f64::consts::PI * 50.0 / 2.0)).abs() < 1e-9,
        "{area}"
    );
}

#[test]
fn bulges_render_as_svg_arcs_and_survive_json() {
    let db = uncad::parse(MIRRORED).expect("fixture must parse");
    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    // Handle 21's 90-degree arc from (100,0) to (100,50): radius
    // 50 sqrt(2) / 2, small arc, positive bulge -> sweep flag 1 on the
    // y-down canvas.
    assert!(svg.contains(" A 35.355"), "{svg}");
    assert!(svg.contains("0 0 1 100 -50"), "{svg}");
    // The mirrored rectangle (handle 20) is drawn at negative x.
    assert!(svg.contains("-100,"), "{svg}");

    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializes");
    assert!(
        json.contains("\"bulges\":[0.0,0.41421356,0.0,0.0]"),
        "{json}"
    );
    let back: uncad::CadDatabase = serde_json::from_str(&json).expect("deserializes");
    let (a, b) = (lwpolyline(&db, "21"), lwpolyline(&back, "21"));
    assert_eq!(a, b);
    assert_eq!(lwpolyline(&db, "20"), lwpolyline(&back, "20"));

    // 0.2.0 JSON, without the new fields, still loads.
    let mut value = serde_json::to_value(Entity::LwPolyline(a.clone())).expect("serializes");
    let object = value.as_object_mut().expect("an object");
    for key in ["bulges", "widths", "const_width", "elevation", "extrusion"] {
        assert!(object.remove(key).is_some(), "{key} is a top-level field");
    }
    let e: Entity = serde_json::from_value(value).expect("0.2.0 shape loads");
    let Entity::LwPolyline(p) = e else {
        panic!("an LWPOLYLINE");
    };
    assert!(p.bulges.is_empty());
    assert_eq!(p.extrusion, uncad::geom::WORLD_Z);
}

#[test]
fn the_revision_cloud_rasterizes() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD).clone();
    let mut only = uncad::CadDatabase::new(vec![Entity::LwPolyline(cloud)], db.tables.clone());
    only.header = db.header.clone();
    let svg = only.to_svg(uncad::ToSvgOptions::default()).svg;
    assert_eq!(svg.matches(" A ").count(), 25, "one arc per segment");
    let png = only
        .to_png(uncad::ToPngOptions::default())
        .expect("rasterizes");
    assert!(png.width > 0 && png.height > 0 && !png.png.is_empty());
}
