//! The normal (DXF 210, `extrusion`) of an entity whose record stores none.
//!
//! LibreDWG leaves the field zero when the record does not store it -- an
//! LWPOLYLINE unless its flag says so, a pre-R13 entity unless its options
//! do. A zero vector is no direction: it is the absent group, whose default
//! is the world Z axis. The DXF twins state no normal either, and the
//! library's DXF importer fills in (0, 0, 1) itself, so the two formats of
//! each drawing have to agree.

use std::collections::BTreeMap;

use uncad::model::Point3D;
use uncad::Entity;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

const Z_AXIS: Point3D = Point3D {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// Every normal the drawing's blocks carry, by entity type.
fn normals(path: &str) -> BTreeMap<String, Vec<Point3D>> {
    let db = uncad::parse(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut out: BTreeMap<String, Vec<Point3D>> = BTreeMap::new();
    for record in db.tables.block_records.values() {
        for e in &record.entities {
            let normal = match e {
                Entity::Circle(c) => c.extrusion,
                Entity::Arc(a) => a.extrusion,
                Entity::LwPolyline(p) | Entity::Polyline2D(p) => p.extrusion,
                Entity::Solid(s) | Entity::Trace(s) => s.extrusion,
                Entity::Text(t) => t.extrusion,
                Entity::Attrib(a) => a.extrusion,
                Entity::Attdef(a) => a.extrusion,
                Entity::Insert(i) => i.extrusion,
                _ => continue,
            };
            out.entry(e.type_name().to_string())
                .or_default()
                .push(normal);
        }
    }
    out
}

#[test]
fn a_normal_the_record_does_not_store_is_the_world_z_axis() {
    // (drawing, the kinds it must have been met with)
    let drawings: [(&str, &[&str]); 3] = [
        // Before R13 every one of these stores its normal only as an option.
        (
            "r11/entities-2d",
            &["ARC", "CIRCLE", "POLYLINE_2D", "SOLID", "TRACE"],
        ),
        (
            "r10/entities",
            &["ARC", "CIRCLE", "POLYLINE_2D", "SOLID", "TRACE"],
        ),
        // From R13 an LWPOLYLINE stores one only when its flag bit 1 says so;
        // none of this drawing's eleven does.
        ("example_2000", &["LWPOLYLINE"]),
    ];
    for (twin, kinds) in drawings {
        for extension in ["dwg", "dxf"] {
            let path = format!("{CORPUS}{twin}.{extension}");
            let found = normals(&path);
            for kind in kinds {
                let met = found.get(*kind).map_or(0, Vec::len);
                assert!(met > 0, "{path}: no {kind}");
            }
            for (kind, normals) in &found {
                assert!(
                    normals.iter().all(|n| *n == Z_AXIS),
                    "{path}: {kind} {normals:?}"
                );
            }
        }
    }
}
