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
