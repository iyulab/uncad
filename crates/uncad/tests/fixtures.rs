//! The hand-authored DXF fixtures in `tests/fixtures/`, read through the
//! public API.
//!
//! Unlike the corpus-based tests, these files were written by this project
//! from group codes (`tests/fixtures/make_fixtures.py`; what each one holds,
//! and what LibreDWG itself reads back from it, is in
//! `tests/fixtures/README.md`), so the values below are the ones the files
//! state -- worked out from the definitions, not copied out of this crate's
//! own output.
//!
//! Only what the *model* carries is asserted here. How a renderer draws a
//! fixture, or what an export writes for it, is a question for those crates.

use std::collections::BTreeMap;

use uncad::model::Ref;
use uncad::{CadDatabase, Entity};

macro_rules! fixture {
    ($name:literal) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $name)
    };
}

const CP949: &str = fixture!("cp949_r2000.dxf");
const MIRRORED: &str = fixture!("mirrored_ocs_r2000.dxf");
const DIMLFAC12: &str = fixture!("dimlfac12_r2000.dxf");
const TWISTED_VIEWPORT: &str = fixture!("twisted_viewport_r2000.dxf");
const MIRRORED_BULGE: &str = fixture!("mirrored_bulge_r2000.dxf");
const HATCHED_VIEWPORT: &str = fixture!("hatched_viewport_r2000.dxf");
const NESTED_ATTRIB: &str = fixture!("nested_attrib_r2000.dxf");
const HIDDEN_LAYERS: &str = fixture!("hidden_layers_r2000.dxf");
const PLOT_ORIGIN: &str = fixture!("plot_origin_r2000.dxf");
const ANGULAR_ORDINATE: &str = fixture!("angular_ordinate_r2000.dxf");
const VIEWPORT_STATES: &str = fixture!("viewport_states_r2000.dxf");
const RADIAL: &str = fixture!("radial_r2000.dxf");
const INFINITE_LINES: &str = fixture!("infinite_lines_r2000.dxf");
const POLYLINE_VERTICES: &str = fixture!("polyline_vertices_r2000.dxf");
const ENTITY_TRUECOLOR: &str = fixture!("entity_truecolor_r2000.dxf");
const POLYFACE_MESH: &str = fixture!("polyface_mesh_r2000.dxf");
const BLOCK_LAYER0: &str = fixture!("block_layer0_r2000.dxf");
const TITLE_BLOCK: &str = fixture!("title_block_r2000.dxf");

/// Every shipped fixture, so the checks that must hold for all of them
/// (parsing, the JSON round trip) cover each new file from the day it
/// lands.
const ALL: [&str; 18] = [
    CP949,
    MIRRORED,
    DIMLFAC12,
    TWISTED_VIEWPORT,
    MIRRORED_BULGE,
    HATCHED_VIEWPORT,
    NESTED_ATTRIB,
    HIDDEN_LAYERS,
    PLOT_ORIGIN,
    ANGULAR_ORDINATE,
    VIEWPORT_STATES,
    RADIAL,
    INFINITE_LINES,
    POLYLINE_VERTICES,
    ENTITY_TRUECOLOR,
    POLYFACE_MESH,
    BLOCK_LAYER0,
    TITLE_BLOCK,
];

fn parse(path: &str) -> CadDatabase {
    uncad::parse(path).unwrap_or_else(|e| panic!("{path} should parse: {e}"))
}

fn type_counts(db: &CadDatabase) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for entity in &db.entities {
        *counts.entry(entity.type_name().to_string()).or_insert(0) += 1;
    }
    counts
}

fn expected(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
    pairs
        .iter()
        .map(|(name, n)| (name.to_string(), *n))
        .collect()
}

fn resolved(name: &str) -> Ref<String> {
    Ref::Resolved(name.to_string())
}

/// The handle the file gave an entity, as the model records it.
fn handle(entity: &Entity) -> &str {
    match &entity.common().source_handle {
        Ref::Resolved(h) => h,
        other => panic!("every fixture entity has a handle: {other:?}"),
    }
}

#[test]
fn every_fixture_parses_and_survives_a_json_round_trip() {
    for path in ALL {
        let db = parse(path);
        assert!(!db.entities.is_empty(), "{path} should have entities");
        assert!(
            db.read_diagnostics.is_clean(),
            "{path}: {:?}",
            db.read_diagnostics.warnings
        );
        let json = db
            .to_json(uncad::ToJsonOptions::default())
            .expect("serializing the model should succeed");
        let back: CadDatabase =
            serde_json::from_str(&json).expect("what this crate wrote, it should read back");
        assert_eq!(back, db, "{path} changed across a JSON round trip");
    }
}

// ---------------------------------------------------------------- cp949

#[test]
fn cp949_fixture_has_four_texts_one_mtext_and_two_layers() {
    let db = parse(CP949);
    assert_eq!(type_counts(&db), expected(&[("TEXT", 4), ("MTEXT", 1)]));
    assert_eq!(
        db.tables.layers.keys().collect::<Vec<_>>(),
        ["0", "\u{BCBD}\u{CCB4}"],
        "layer 0 plus the Korean-named layer"
    );
    assert_eq!(db.tables.layers["0"].color_index, 7);
    assert_eq!(db.tables.layers["\u{BCBD}\u{CCB4}"].color_index, 1);
}

#[test]
fn cp949_strings_decode_through_the_file_code_page() {
    let db = parse(CP949);
    let texts: Vec<(&str, &str)> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some((handle(e), t.text.as_str())),
            _ => None,
        })
        .collect();
    // The bytes are B5B5 B8E9 / A1BE 33 / 3332 2E35 A7B3 (README): "±" is a
    // two-byte KS X 1001 character, the case a single-byte reading gets
    // wrong.
    assert_eq!(
        texts,
        [
            ("23", "\u{B3C4}\u{BA74}"),
            ("24", "\u{B1}3"),
            ("25", "32.5\u{33A1}"),
            ("27", "PLAIN"),
        ]
    );
    // The MTEXT keeps its format code `\P` verbatim.
    let mtext = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::MText(m) => Some(m.text.as_str()),
            _ => None,
        })
        .expect("MTEXT");
    assert_eq!(mtext, "\u{BC29} 101\\P\u{BA74}\u{C801} 32.5\u{33A1}");

    // The TEXT on the Korean layer names it the way the table does.
    assert_eq!(
        db.entities[0].common().layer,
        resolved("\u{BCBD}\u{CCB4}"),
        "{:?}",
        db.entities[0]
    );
}

// ------------------------------------------------------------ dimlfac12

#[test]
fn dimlfac12_fixture_has_a_line_and_a_dimension_bound_to_its_cached_block() {
    let db = parse(DIMLFAC12);
    assert_eq!(type_counts(&db), expected(&[("DIMENSION", 1), ("LINE", 1)]));

    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("DIMENSION");
    assert_eq!(dim.common.source_handle, resolved("45"));
    // `2 *D1`, bound through the BLOCK_RECORD table, and `42 = 10.0`.
    assert_eq!(dim.block_name, resolved("*D1"));
    assert_eq!(dim.measurement, Some(10.0));
    assert_eq!(dim.style_name, resolved("STANDARD"));
    assert_eq!(db.tables.block_records["*Model_Space"].entities.len(), 2);
    // The cached label is the block's one TEXT: 12 x 10 under DIMLFAC 12.
    let cached = &db.tables.block_records["*D1"].entities;
    assert_eq!(cached.len(), 1, "{cached:?}");
    match &cached[0] {
        Entity::Text(t) => assert_eq!(t.text, "120"),
        other => panic!("expected the cached dimension label, got {other:?}"),
    }
}

// ------------------------------------------------------- twisted viewport

#[test]
fn twisted_viewport_fixture_has_a_model_line_and_a_paper_viewport() {
    let db = parse(TWISTED_VIEWPORT);
    assert_eq!(type_counts(&db), expected(&[("LINE", 1), ("VIEWPORT", 1)]));
    let vp = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Viewport(v) => Some(v),
            _ => None,
        })
        .expect("VIEWPORT");
    assert_eq!(vp.common.source_handle, resolved("2A"));
    assert_eq!((vp.center.x, vp.center.y, vp.center.z), (150.0, 100.0, 0.0));
    assert_eq!((vp.width, vp.height), (200.0, 120.0));
    // `67 = 1` and `330 = 1C`: the VIEWPORT is paper space's, the LINE the
    // model's.
    assert_eq!(db.tables.block_records["*Model_Space"].entities.len(), 1);
    let paper = &db.tables.block_records["*Paper_Space"].entities;
    assert_eq!(paper.len(), 1, "{paper:?}");
    assert!(matches!(paper[0], Entity::Viewport(_)), "{paper:?}");
}

// ------------------------------------------------------- infinite lines

#[test]
fn infinite_lines_fixture_has_a_line_a_text_an_xline_and_a_ray() {
    let db = parse(INFINITE_LINES);
    assert_eq!(
        type_counts(&db),
        expected(&[("LINE", 1), ("RAY", 1), ("TEXT", 1), ("XLINE", 1)])
    );
    let (mut xline, mut ray) = (None, None);
    for e in &db.entities {
        match e {
            Entity::XLine(x) => xline = Some(x),
            Entity::Ray(r) => ray = Some(r),
            _ => {}
        }
    }
    let (xline, ray) = (xline.expect("XLINE"), ray.expect("RAY"));
    assert_eq!(xline.common.source_handle, resolved("32"));
    assert_eq!((xline.point.x, xline.point.y), (0.001, 0.001));
    assert_eq!((xline.vector.x, xline.vector.y), (1.0, 0.0));
    assert_eq!(ray.common.source_handle, resolved("33"));
    assert_eq!((ray.point.x, ray.point.y), (0.001, 0.001));
    assert_eq!((ray.vector.x, ray.vector.y), (0.0, 1.0));
}

// ------------------------------------------------------- block on layer 0

/// AutoCAD's rule that a block's layer-0 geometry takes the layer of the
/// reference is a property of the reference, not of the block: the model
/// keeps what the file stores, and the children stay on layer 0.
#[test]
fn block_geometry_on_layer_0_stays_on_layer_0_in_the_model() {
    let db = parse(BLOCK_LAYER0);
    let insert = match &db.entities[0] {
        Entity::Insert(i) => i,
        other => panic!("expected the INSERT, got {other:?}"),
    };
    assert_eq!(insert.common.layer, resolved("RED"));
    assert_eq!(insert.block_name, resolved("SYM"));
    let children: Vec<&Ref<String>> = db.tables.block_records["SYM"]
        .entities
        .iter()
        .map(|e| &e.common().layer)
        .collect();
    assert_eq!(
        children,
        [
            &resolved("0"),
            &resolved("0"),
            &resolved("BLUE"),
            &resolved("0")
        ]
    );
}

// ------------------------------------------------------------ title_block

#[test]
fn the_title_block_fixture_keeps_every_string_in_paper_space() {
    // The model holds one LINE and nothing else, while the sheet carries
    // the drawing's name: make_fixtures.py's `title_block()`.
    let db = parse(TITLE_BLOCK);
    assert_eq!(
        type_counts(&db),
        expected(&[("INSERT", 1), ("LINE", 1), ("TEXT", 1), ("VIEWPORT", 1)])
    );
    let model = &db.tables.block_records["*Model_Space"];
    assert_eq!(model.entities.len(), 1);
    assert!(matches!(model.entities[0], Entity::Line(_)));
    let texts = |block: &str| -> Vec<String> {
        db.tables.block_records[block]
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(texts("*Paper_Space"), ["GARDEN PAVILION"]);
    assert_eq!(texts("TITLEBLOCK"), ["SHEET 1 OF 2"]);
    // One A4 sheet, the layout that shows paper space.
    assert_eq!(db.tables.layouts.keys().collect::<Vec<_>>(), ["Layout1"]);
    assert_eq!(
        db.tables.layouts["Layout1"].block_name,
        resolved("*Paper_Space")
    );
}

// --------------------------------------------------- polyline vertices

/// Regression for the last vertex LibreDWG's own
/// `dwg_object_polyline_{2,3}d_get_points` drop on every R13/R14/R2000 file.
/// The vertices are the ones `make_fixtures.py` writes: a 100 by 100 closed
/// square, a two-vertex polyline whose one bulge makes it a semicircle, and a
/// five-point 3D polyline. Read through the library's accessors each came
/// back one vertex short: the square a right triangle, the semicircle a
/// single point, the 3D polyline ending at (0, 10, 5).
#[test]
fn every_polyline_vertex_survives_the_r2000_subentity_chain() {
    let db = parse(POLYLINE_VERTICES);
    assert_eq!(
        type_counts(&db),
        expected(&[("POLYLINE_2D", 2), ("POLYLINE_3D", 1)])
    );

    let square = match &db.entities[0] {
        Entity::Polyline2D(p) => p,
        other => panic!("expected the square first, got {other:?}"),
    };
    assert_eq!(square.common.source_handle, resolved("30"));
    assert!(square.closed);
    assert_eq!(
        square
            .vertices
            .iter()
            .map(|v| (v.x, v.y))
            .collect::<Vec<_>>(),
        [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
    );

    let arc = match &db.entities[1] {
        Entity::Polyline2D(p) => p,
        other => panic!("expected the arc polyline second, got {other:?}"),
    };
    assert_eq!(arc.common.source_handle, resolved("35"));
    assert!(!arc.closed);
    assert_eq!(
        arc.vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>(),
        [(0.0, 1000.0), (100.0, 1000.0)]
    );

    let p3d = match &db.entities[2] {
        Entity::Polyline3D(p) => p,
        other => panic!("expected the 3D polyline third, got {other:?}"),
    };
    assert_eq!(p3d.common.source_handle, resolved("39"));
    assert!(
        !p3d.closed,
        "group 70 = 8 is a 3D polyline, not a closed one"
    );
    assert_eq!(
        p3d.vertices
            .iter()
            .map(|v| (v.x, v.y, v.z))
            .collect::<Vec<_>>(),
        [
            (0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            (10.0, 10.0, 0.0),
            (0.0, 10.0, 5.0),
            (0.0, 0.0, 5.0),
        ]
    );
    // And no VERTEX record is an entity of the block that holds them.
    assert_eq!(db.tables.block_records["*Model_Space"].entities.len(), 3);
}

// --------------------------------------------------------- mesh polylines

/// A DXF whose one polygon mesh made an unpatched LibreDWG refuse the whole
/// file, and whose polyface mesh found no vertex positions: the importer
/// types its vertices VERTEX_MESH, because they name the block record as
/// their owner.
///
/// The edge counts are the grids `make_fixtures.py` writes, worked out from
/// the mesh definitions: the polyface has two quad faces, so 2 * 4 = 8
/// edges; the polygon mesh is an open 3 by 4 grid, so 4 * (3 - 1) edges
/// between the rows plus 3 * (4 - 1) along them = 17.
#[test]
fn a_dxf_with_mesh_polylines_reads_and_both_meshes_have_their_wireframe() {
    let db = parse(POLYFACE_MESH);
    assert_eq!(
        type_counts(&db),
        expected(&[("LINE", 1), ("POLYLINE_MESH", 1), ("POLYLINE_PFACE", 1)]),
        "no VERTEX record may be reported as an entity"
    );
    let model = &db.tables.block_records["*Model_Space"].entities;
    assert_eq!(model.len(), 3, "{model:?}");

    let Entity::PolylinePFace(pface) = &db.entities[1] else {
        panic!("expected the polyface second: {:?}", db.entities[1]);
    };
    assert_eq!(pface.common.source_handle, resolved("31"));
    assert_eq!((pface.wireframe_edges.len(), pface.skipped_edges), (8, 0));

    let Entity::PolylineMesh(mesh) = &db.entities[2] else {
        panic!("expected the polygon mesh third: {:?}", db.entities[2]);
    };
    assert_eq!(mesh.common.source_handle, resolved("50"));
    assert_eq!((mesh.wireframe_edges.len(), mesh.skipped_edges), (17, 0));
    // Vertex i*4 + j sits at (i*10, j*5, 0): the first edge joins row 0 to
    // row 1 in column 0, the first edge along a row is its column 0 to 1.
    let point = |i: f64, j: f64| uncad::model::Point3D {
        x: i * 10.0,
        y: j * 5.0,
        z: 0.0,
    };
    assert_eq!(mesh.wireframe_edges[0], [point(0.0, 0.0), point(1.0, 0.0)]);
    assert_eq!(mesh.wireframe_edges[8], [point(0.0, 0.0), point(0.0, 1.0)]);
}

// ------------------------------------------------------------- mirrored

const DOWN: uncad::model::Point3D = uncad::model::Point3D {
    x: 0.0,
    y: 0.0,
    z: -1.0,
};
const UP: uncad::model::Point3D = uncad::model::Point3D {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

#[test]
fn mirrored_ocs_fixture_has_the_expected_entity_mix() {
    let db = parse(MIRRORED);
    assert_eq!(
        type_counts(&db),
        expected(&[
            ("ARC", 1),
            ("CIRCLE", 1),
            ("LINE", 1),
            ("LWPOLYLINE", 2),
            ("TEXT", 1)
        ])
    );
    // No TABLES section in this file: no layer resolves.
    assert!(db
        .entities
        .iter()
        .all(|e| !matches!(e.common().layer, Ref::Resolved(_))));
}

/// Coordinates are carried as the file states them, in the entity's object
/// coordinate system, beside the normal (DXF 210) that defines it: taking
/// them to the world is the consumer's step. The feature branch moved them
/// to world coordinates while reading; the model does not.
#[test]
fn mirrored_ocs_entities_keep_their_stated_coordinates_and_carry_their_normal() {
    let db = parse(MIRRORED);
    let polylines: Vec<&uncad::model::LwPolylineEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::LwPolyline(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(polylines.len(), 2);
    let xy = |p: &uncad::model::LwPolylineEntity| -> Vec<(f64, f64)> {
        p.vertices.iter().map(|v| (v.x, v.y)).collect()
    };
    let outline = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)];

    // Handle 20: DXF 70 = 1 with extrusion (0,0,-1); LibreDWG stores
    // flag = 513 (512 closed | 1 has-extrusion).
    let mirrored = polylines[0];
    assert_eq!(mirrored.common.source_handle, resolved("20"));
    assert!(mirrored.closed);
    assert_eq!(mirrored.extrusion, DOWN);
    assert_eq!(xy(mirrored), outline);
    assert_eq!(mirrored.elevation, 0.0);

    // Handle 21: DXF 70 = 0 (flag = 16: neither bit set), extrusion
    // (0,0,1) stated.
    let upright = polylines[1];
    assert_eq!(upright.common.source_handle, resolved("21"));
    assert!(!upright.closed);
    assert_eq!(upright.extrusion, UP);
    assert_eq!(xy(upright), outline);

    let mut seen = 0;
    for e in &db.entities {
        match e {
            Entity::Circle(c) => {
                assert_eq!((c.center.x, c.center.y, c.center.z), (10.0, 10.0, 0.0));
                assert_eq!((c.radius, c.extrusion), (5.0, DOWN));
                seen += 1;
            }
            Entity::Arc(a) => {
                // Stated 0 to 90 degrees; in the world the arc runs
                // clockwise from 90 to 180, which is not the model's to say.
                assert_eq!((a.center.x, a.center.y, a.center.z), (0.0, 0.0, 0.0));
                assert_eq!((a.radius, a.extrusion), (20.0, DOWN));
                assert_eq!(a.start_angle, 0.0);
                assert_eq!(a.end_angle, std::f64::consts::FRAC_PI_2);
                seen += 1;
            }
            Entity::Text(t) => {
                assert_eq!(t.text, "MIRROR");
                assert_eq!((t.start_point.x, t.start_point.y), (10.0, 10.0));
                assert_eq!((t.extrusion, t.elevation), (DOWN, 0.0));
                seen += 1;
            }
            Entity::Line(l) => {
                // A LINE is stated in world coordinates and has no OCS.
                assert_eq!((l.start_point.x, l.start_point.y), (-5.0, -5.0));
                assert_eq!((l.end_point.x, l.end_point.y), (5.0, 5.0));
                seen += 1;
            }
            _ => {}
        }
    }
    assert_eq!(seen, 4);
}

// ------------------------------------------------------------- bulges

/// A bulge is carried as stated -- one per vertex, the sign the file wrote
/// -- and a polyline whose segments are all straight carries none.
#[test]
fn polyline_bulges_are_carried_as_the_file_states_them() {
    let bulged_polyline = |path: &str| -> Vec<(String, Vec<f64>, uncad::model::Point3D)> {
        parse(path)
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::LwPolyline(p) | Entity::Polyline2D(p) => {
                    Some((handle(e).to_string(), p.bulges.clone(), p.extrusion))
                }
                _ => None,
            })
            .collect()
    };
    // The mirrored fixture: handle 20 states no bulge, handle 21 one of
    // tan(22.5 degrees) after its second vertex -- a quarter circle.
    assert_eq!(
        bulged_polyline(MIRRORED),
        [
            ("20".to_string(), vec![], DOWN),
            ("21".to_string(), vec![0.0, 0.41421356, 0.0, 0.0], UP),
        ]
    );
    // The same outline in a mirrored OCS keeps the sign the file wrote:
    // the arc turns counter-clockwise in its OCS, and clockwise in the
    // world only once a consumer takes it there.
    assert_eq!(
        bulged_polyline(MIRRORED_BULGE),
        [("20".to_string(), vec![0.0, 0.41421356, 0.0, 0.0], DOWN)]
    );
    // A POLYLINE_2D's bulges are its VERTEX records' (group 42): the
    // semicircle's one bulge of 1.0, and the square's none.
    assert_eq!(
        bulged_polyline(POLYLINE_VERTICES),
        [
            ("30".to_string(), vec![], UP),
            ("35".to_string(), vec![1.0, 0.0], UP),
        ]
    );
    // The mirrored fixture's ARC is the same arc, stated as an ARC states
    // it: 315 to 45 degrees in the mirrored OCS.
    let arc = parse(MIRRORED_BULGE)
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Arc(a) => Some(a.clone()),
            _ => None,
        })
        .expect("the ARC");
    assert_eq!(arc.extrusion, DOWN);
    assert!((arc.start_angle - 315f64.to_radians()).abs() < 1e-9);
    assert!((arc.end_angle - 45f64.to_radians()).abs() < 1e-9);
}

// -------------------------------------------------------- nested attrib

/// The attribute of a block reference nested in another block. The inner
/// INSERT (`55`, inside `DOOR`) carries its value as its own attribute, and
/// the block's entity list does not carry that ATTRIB a second time; the
/// model-space INSERT (`61`) carries its value and the top level lists it
/// once more after it, as every drawn INSERT's attributes are.
#[test]
fn a_nested_block_reference_carries_its_attribute_once() {
    let db = parse(NESTED_ATTRIB);
    let attribs = |e: &Entity| -> Vec<(String, String, String)> {
        match e {
            Entity::Insert(i) => i
                .attribs
                .iter()
                .map(|a| {
                    let Ref::Resolved(h) = &a.common.source_handle else {
                        panic!("{a:?}");
                    };
                    (h.clone(), a.tag.clone(), a.text.clone())
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    let entry = |h: &str, tag: &str, text: &str| (h.to_string(), tag.to_string(), text.to_string());

    let door: Vec<(&str, &str)> = db.tables.block_records["DOOR"]
        .entities
        .iter()
        .map(|e| (e.type_name(), handle(e)))
        .collect();
    assert_eq!(door, [("LINE", "52"), ("LINE", "53"), ("INSERT", "55")]);
    assert_eq!(
        attribs(&db.tables.block_records["DOOR"].entities[2]),
        [entry("56", "NUM", "D-101")]
    );
    // The definition keeps its ATTDEF, with no flag set.
    let tag = db.tables.block_records["TAG"]
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Attdef(a) => Some(a),
            _ => None,
        })
        .expect("TAG's ATTDEF");
    assert_eq!(
        (tag.tag.as_str(), tag.default_value.as_str()),
        ("NUM", "D-000")
    );
    assert_eq!(tag.flags, uncad::model::AttributeFlags::default());

    let top: Vec<(&str, &str)> = db
        .entities
        .iter()
        .map(|e| (e.type_name(), handle(e)))
        .collect();
    assert_eq!(top, [("INSERT", "60"), ("INSERT", "61"), ("ATTRIB", "62")]);
    assert_eq!(attribs(&db.entities[0]), []);
    assert_eq!(attribs(&db.entities[1]), [entry("62", "NUM", "D-TOP")]);
}

// ------------------------------------------------------ entity truecolor

/// The four ways a DXF entity can state its colour, on a layer whose own ACI
/// is 3. 65407 is `0x00ff7f` and 255 is `0x0000ff`, the file's own group 420
/// values; a plain group 62 states no RGB, although LibreDWG's importer
/// synthesises one for it from its own palette.
#[test]
fn a_dxf_entity_carries_the_true_colour_it_states_and_no_other() {
    let db = parse(ENTITY_TRUECOLOR);
    let colors: Vec<(&str, i16, Option<u32>)> = db
        .entities
        .iter()
        .map(|e| (handle(e), e.common().color_index, e.common().true_color))
        .collect();
    assert_eq!(
        colors,
        [
            ("30", 256, Some(0x00_ff7f)),
            ("31", 1, None),
            ("32", 1, Some(0x00_00ff)),
            ("33", 256, None),
        ]
    );
}

// --------------------------------------------------- dimension points

fn dimensions(db: &CadDatabase) -> Vec<&uncad::model::DimensionEntity> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Dimension(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn p3(x: f64, y: f64) -> Option<uncad::model::Point3D> {
    Some(uncad::model::Point3D { x, y, z: 0.0 })
}

/// Rounds a point to the fixture's own precision, so a hand-derived
/// coordinate (8.660254 for 10 sin 60) compares with the stored one.
fn rounded(p: Option<uncad::model::Point3D>) -> Option<(i64, i64)> {
    p.map(|p| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64))
}

/// The two kinds whose points LibreDWG's DXF importer lays out differently
/// from its DWG decoder, every value derived by hand in the fixture's
/// README: a 2-line angular dimension between (0,0)-(10,0) and
/// (0,0)-(5, 8.660254) with its arc point 30 degrees along a radius of 5,
/// and an X- and a Y-type ordinate sharing the datum (100, 200) and the
/// feature (130, 250). Each point lands in the field of the group the file
/// wrote it in.
#[test]
fn the_angular_ordinate_fixture_reads_every_point_by_its_group() {
    use uncad::model::{DimensionKind, OrdinateAxis};
    let db = parse(ANGULAR_ORDINATE);
    let dims = dimensions(&db);
    assert_eq!(dims.len(), 3);

    let angular = dims[0];
    assert_eq!(angular.kind, Some(DimensionKind::Angular2Line));
    assert_eq!(
        rounded(angular.definition_point),
        rounded(p3(5.0, 8.660254)),
        "group 10"
    );
    assert_eq!(angular.points.extension1, p3(0.0, 0.0), "group 13");
    assert_eq!(angular.points.extension2, p3(10.0, 0.0), "group 14");
    assert_eq!(angular.points.radial, p3(0.0, 0.0), "group 15");
    assert_eq!(
        rounded(angular.points.arc),
        rounded(p3(4.330127, 2.5)),
        "group 16"
    );
    assert_eq!(angular.ordinate_axis, None);
    // Group 42 is pi/3, in radians as the file states it.
    let measured = angular.measurement.expect("group 42");
    assert!(
        (measured - std::f64::consts::FRAC_PI_3).abs() < 1e-12,
        "{measured}"
    );

    for (d, axis, leader, value) in [
        (dims[1], OrdinateAxis::X, (130.0, 270.0), 30.0),
        (dims[2], OrdinateAxis::Y, (150.0, 250.0), 50.0),
    ] {
        assert_eq!(d.kind, Some(DimensionKind::Ordinate));
        assert_eq!(d.ordinate_axis, Some(axis), "{:?}", d.common.source_handle);
        assert_eq!(d.definition_point, p3(100.0, 200.0), "the datum, group 10");
        assert_eq!(
            d.points.extension1,
            p3(130.0, 250.0),
            "the feature, group 13"
        );
        assert_eq!(
            d.points.extension2,
            p3(leader.0, leader.1),
            "the leader, group 14"
        );
        assert_eq!(d.measurement, Some(value));
    }
}

/// The three dimension kinds no corpus DXF carries, each point in the field
/// of the group the file wrote it in (the fixture's README).
#[test]
fn the_radial_fixture_reads_its_points_by_their_groups() {
    use uncad::model::DimensionKind;
    let db = parse(RADIAL);
    let dims = dimensions(&db);
    let kinds: Vec<Option<DimensionKind>> = dims.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        [
            Some(DimensionKind::Radius),
            Some(DimensionKind::Diameter),
            Some(DimensionKind::Angular3Point)
        ]
    );
    let (radius, diameter, angular) = (dims[0], dims[1], dims[2]);
    assert_eq!(
        radius.definition_point,
        p3(0.0, 0.0),
        "the centre, group 10"
    );
    assert_eq!(
        radius.points.radial,
        p3(3.0, 4.0),
        "on the circle, group 15"
    );
    assert_eq!(radius.measurement, Some(5.0));
    assert_eq!(diameter.definition_point, p3(20.0, 0.0), "group 10");
    assert_eq!(diameter.points.radial, p3(20.0, 10.0), "group 15");
    assert_eq!(diameter.measurement, Some(10.0));
    assert_eq!(
        rounded(angular.definition_point),
        rounded(p3(44.330127, 2.5)),
        "group 10"
    );
    assert_eq!(angular.points.extension1, p3(50.0, 0.0), "group 13");
    assert_eq!(
        rounded(angular.points.extension2),
        rounded(p3(45.0, 8.660254)),
        "group 14"
    );
    assert_eq!(angular.points.radial, p3(40.0, 0.0), "the centre, group 15");
}

// ------------------------------------------------------------ viewports

fn viewports(db: &CadDatabase) -> Vec<&uncad::model::ViewportEntity> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Viewport(v) => Some(v),
            _ => None,
        })
        .collect()
}

/// The twisted viewport's view, as its AcDbViewport groups state it: centre
/// (50, 25) and height 60 in the view's own coordinates (so the 120-high
/// frame shows the model at scale 2), looking down (0, 0, 1) at (0, 0, 0),
/// turned 30 degrees, lens 50; on (68 = 1), number 2 (69), nothing frozen.
#[test]
fn the_twisted_viewport_carries_its_view_as_stated() {
    use uncad::model::{Point2D, Point3D, ViewportView};
    let db = parse(TWISTED_VIEWPORT);
    let vp = viewports(&db)[0];
    let view = vp.view.expect("an R2000 viewport carries its view");
    assert_eq!(
        view,
        ViewportView {
            center: Point2D { x: 50.0, y: 25.0 },
            height: 60.0,
            target: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0
            },
            direction: UP,
            twist: view.twist,
            lens_length: 50.0,
        }
    );
    assert!(
        (view.twist - 30f64.to_radians()).abs() < 1e-12,
        "{}",
        view.twist
    );
    assert_eq!((vp.on, vp.viewport_id), (Some(true), Some(2)));
    assert!(vp.frozen_layers.is_empty());
}

/// One viewport per state a sheet tells apart: on; on, on a frozen layer
/// (the frame hidden, not the window); off (68 = 0, status bit 0x20000);
/// not a plan view (VIEWDIR (1,1,1)). Numbers 2 to 5.
#[test]
fn the_viewport_states_fixture_carries_each_state() {
    use uncad::model::Point3D;
    let db = parse(VIEWPORT_STATES);
    /// (handle, layer, on, number, view direction)
    type State<'a> = (&'a str, Ref<String>, Option<bool>, Option<i32>, Point3D);
    let states: Vec<State> = viewports(&db)
        .iter()
        .map(|v| {
            let Ref::Resolved(h) = &v.common.source_handle else {
                panic!("{v:?}");
            };
            (
                h.as_str(),
                v.common.layer.clone(),
                v.on,
                v.viewport_id,
                v.view.expect("R2000").direction,
            )
        })
        .collect();
    let diagonal = Point3D {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };
    assert_eq!(
        states,
        [
            ("2A", resolved("0"), Some(true), Some(2), UP),
            ("2D", resolved("VPFROZEN"), Some(true), Some(3), UP),
            ("2E", resolved("0"), Some(false), Some(4), UP),
            ("2F", resolved("0"), Some(true), Some(5), diagonal),
        ]
    );
    for vp in viewports(&db) {
        let view = vp.view.expect("R2000");
        assert_eq!(
            (view.center.x, view.center.y, view.height),
            (50.0, 25.0, 20.0)
        );
    }
}

// ------------------------------------------------ hidden layers, hatches

/// One LINE per layer, in table order, then an invisible LINE (`60 = 1`) and
/// a visible one on `VISIBLE`: the entity's own invisible flag is carried,
/// and every line keeps the layer it names whatever that layer's state.
#[test]
fn the_hidden_layers_fixture_s_entities_keep_their_layers_and_their_flag() {
    let db = parse(HIDDEN_LAYERS);
    let lines: Vec<(Ref<String>, bool)> = db
        .entities
        .iter()
        .map(|e| (e.common().layer.clone(), e.common().invisible))
        .collect();
    assert_eq!(
        lines,
        [
            (resolved("0"), false),
            (resolved("VISIBLE"), false),
            (resolved("OFF"), false),
            (resolved("FROZEN"), false),
            (resolved("NOPLOT"), false),
            (resolved("Defpoints"), false),
            (resolved("LOCKED"), false),
            (resolved("VISIBLE"), true),
            (resolved("VISIBLE"), false),
        ]
    );
}

/// One pattern HATCH per space: the model one under the viewport (vertical
/// lines 2 apart, `53 = 90`), the paper one beside it (horizontal lines 4
/// apart), each owned by its own space's block.
#[test]
fn the_hatched_viewport_fixture_keeps_each_hatch_in_its_space() {
    let db = parse(HATCHED_VIEWPORT);
    let pattern = |block: &str| -> Vec<(f64, f64, f64)> {
        db.tables.block_records[block]
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Hatch(h) => Some(
                    h.pattern_lines
                        .iter()
                        .map(|l| (l.angle, l.offset.x, l.offset.y))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    };
    assert_eq!(
        pattern("*Model_Space"),
        [(std::f64::consts::FRAC_PI_2, -2.0, 0.0)]
    );
    assert_eq!(pattern("*Paper_Space"), [(0.0, 0.0, 4.0)]);
}
