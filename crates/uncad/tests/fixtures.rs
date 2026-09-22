//! The P-1 fixtures for the 0.3.0 "Readable" work (`docs/VLM_EXPORT_DESIGN.md`,
//! section 10), read through the public API.
//!
//! Unlike the corpus-based tests, these files were authored by this project
//! (`tests/fixtures/README.md` lists how, and every value below was read back
//! through LibreDWG before it was written down), so exact values can be
//! pinned without copying them out of this crate's own output.
//!
//! Where a value is still the lossy one the roadmap replaces, the assertion
//! pins it *as a placeholder* and names the value it is meant to become, so
//! the phase that fixes it (P2 text, P3 dimensions, P4 polylines/OCS, P7
//! layouts) fails this test and flips it deliberately, rather than silently
//! changing behaviour nobody was watching. The code-page (P-1/P0) placeholders
//! have already been flipped: they now pin the decoded strings.

use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

use uncad::Entity;

const CP949: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/cp949_r2000.dxf"
);
const MIRRORED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mirrored_ocs_r2000.dxf"
);
const DIMLFAC12: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dimlfac12_r2000.dxf"
);
const TWISTED_VIEWPORT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/twisted_viewport_r2000.dxf"
);

const ALL: [&str; 4] = [CP949, MIRRORED, DIMLFAC12, TWISTED_VIEWPORT];

fn parse(path: &str) -> uncad::CadDatabase {
    uncad::parse(path).unwrap_or_else(|e| panic!("{path} should parse: {e}"))
}

fn type_counts(db: &uncad::CadDatabase) -> BTreeMap<String, usize> {
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

#[test]
fn every_fixture_parses_and_survives_a_json_round_trip() {
    for path in ALL {
        let db = parse(path);
        assert!(!db.entities.is_empty(), "{path} should project to entities");
        let json = db
            .to_json(uncad::ToJsonOptions::default())
            .expect("serializing the model should succeed");
        let back: uncad::CadDatabase =
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
        db.tables.layers.len(),
        2,
        "layer 0 plus the Korean-named layer: {:?}",
        db.tables.layers.keys().collect::<Vec<_>>()
    );
    assert!(db.tables.layers.contains_key("0"));
    assert_eq!(db.tables.layers["0"].color_index, 7);
}

#[test]
fn cp949_header_names_the_code_page_and_units() {
    let db = parse(CP949);
    assert_eq!(db.header.version, "r2000");
    assert_eq!(
        (db.header.codepage, db.header.codepage_name.as_str()),
        (40, "ANSI_949")
    );
    assert_eq!(db.header.insunits, 4);
    assert_eq!(db.header.units.name, "mm");
    assert_eq!(db.header.measurement, 1);
}

#[test]
fn cp949_text_values_decode_through_the_file_code_page() {
    let db = parse(CP949);
    let texts: Vec<&str> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    let mtexts: Vec<&str> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::MText(m) => Some(m.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 4, "{texts:?}");

    // The ASCII control needs no conversion, so it is right already.
    assert!(texts.contains(&"PLAIN"), "{texts:?}");

    // LibreDWG keeps the CP949 bytes raw on DXF input (Dwg_Data.header.codepage
    // is 40 = ANSI_949); the uncad_tv_to_utf8 shim decodes them through that
    // code page (bytes B5B5 B8E9 / A1BE 33 / 3332 2E35 A7B3 -- see
    // fixtures/README.md). "±" is a two-byte KS X 1001 character (A1 BE), the
    // case a single-byte assumption gets wrong.
    for decoded in ["도면", "±3", "32.5㎡"] {
        assert!(
            texts.contains(&decoded),
            "{decoded:?} missing from {texts:?}"
        );
    }
    assert!(
        texts.iter().all(|t| !t.contains('\u{FFFD}')),
        "no replacement characters may survive: {texts:?}"
    );

    // MTEXT "방 101\P면적 32.5㎡": the raw string keeps the literal \P, the
    // decoded one has the paragraph break.
    assert_eq!(mtexts, ["방 101\\P면적 32.5㎡"]);
    let plain: Vec<&str> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::MText(m) => Some(m.text_plain.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(plain, ["방 101\n면적 32.5㎡"]);
}

#[test]
fn cp949_layer_name_decodes_through_the_file_code_page() {
    let db = parse(CP949);
    let korean_layer = db
        .tables
        .layers
        .keys()
        .find(|name| name.as_str() != "0")
        .expect("the fixture defines a second layer");

    // Bytes BA AE C3 BC ("벽체"). Before 0.3.0 the lossy path produced
    // "\u{FFFD}\u{FFFD}\u{00FC}": the first two bytes are invalid UTF-8, but
    // the last two happen to be a valid encoding of U+00FC, so it invented a
    // wrong character rather than just dropping one.
    assert_eq!(korean_layer.as_str(), "벽체");
    assert_eq!(db.tables.layers[korean_layer].color_index, 1);

    // The TEXT "도면" is the one entity on that layer, and it resolves its
    // layer to the same name the table uses.
    let on_layer: Vec<&Entity> = db
        .entities
        .iter()
        .filter(|e| e.common().layer == *korean_layer)
        .collect();
    assert_eq!(on_layer.len(), 1, "{on_layer:?}");
    match on_layer[0] {
        Entity::Text(t) => {
            assert_eq!(t.text, "도면");
            assert_eq!(t.common.handle, "23");
        }
        other => panic!("expected the TEXT on the Korean layer, got {other:?}"),
    }
}

// ------------------------------------------------------------- mirrored

#[test]
fn mirrored_ocs_fixture_has_the_expected_entity_mix() {
    let db = parse(MIRRORED);
    assert_eq!(
        type_counts(&db),
        expected(&[
            ("LWPOLYLINE", 2),
            ("CIRCLE", 1),
            ("ARC", 1),
            ("TEXT", 1),
            ("LINE", 1)
        ])
    );
    // No TABLES section in this file: every layer handle is unresolved.
    assert!(db.entities.iter().all(|e| e.common().layer.is_empty()));
}

#[test]
fn mirrored_ocs_polylines_come_out_in_world_coordinates_with_their_bulges() {
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
    assert_eq!(polylines[0].common.handle, "20");
    assert_eq!(polylines[1].common.handle, "21");
    let xy = |p: &uncad::model::LwPolylineEntity| -> Vec<(f64, f64)> {
        p.vertices.iter().map(|v| (v.x, v.y)).collect()
    };

    // Handle 20: DXF 70 = 1 with extrusion (0,0,-1); LibreDWG stores
    // flag = 513 (512 closed | 1 has-extrusion) and `closed` reads bit 512.
    // The OCS vertices (0,0) (100,0) (100,50) (0,50) are mirrored about the
    // y axis in world terms.
    let mirrored = polylines[0];
    assert!(mirrored.closed);
    let e = mirrored.extrusion;
    assert_eq!((e.x, e.y, e.z), (0.0, 0.0, -1.0));
    assert_eq!(
        xy(mirrored),
        [(0.0, 0.0), (-100.0, 0.0), (-100.0, 50.0), (0.0, 50.0)]
    );
    assert!(mirrored.bulges.is_empty(), "{:?}", mirrored.bulges);
    assert_eq!(mirrored.length(), 300.0);
    assert_eq!(mirrored.area(), Some(5000.0));

    // Handle 21: DXF 70 = 0 with a bulge; LibreDWG stores flag = 16, so
    // neither bit is set and it is open. Its bulge after the second vertex
    // is a 90-degree arc from (100,0) to (100,50) (radius 25 sqrt 2), which
    // adds to the length and to the area (an open outline is closed by a
    // straight segment for the area).
    let bulged = polylines[1];
    assert!(!bulged.closed);
    assert_eq!(bulged.extrusion.z, 1.0);
    assert_eq!(
        xy(bulged),
        [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]
    );
    assert_eq!(bulged.bulges, [0.0, 0.41421356, 0.0, 0.0]);
    assert!(
        (bulged.length() - 255.536037).abs() < 1e-5,
        "{}",
        bulged.length()
    );
    let area = bulged.area().expect("four vertices");
    assert!((area - 5356.7477).abs() < 1e-3, "{area}");
    assert_eq!(bulged.elevation, 0.0);
    assert_eq!(bulged.const_width, 0.0);
    assert!(bulged.widths.is_empty());
}

#[test]
fn mirrored_ocs_circle_arc_and_text_move_to_world_coordinates_and_the_line_stays() {
    let db = parse(MIRRORED);
    let mut circle = None;
    let mut arc = None;
    let mut text = None;
    let mut line = None;
    for e in &db.entities {
        match e {
            Entity::Circle(c) => circle = Some(c),
            Entity::Arc(a) => arc = Some(a),
            Entity::Text(t) => text = Some(t),
            Entity::Line(l) => line = Some(l),
            _ => {}
        }
    }
    // CIRCLE, ARC and TEXT carry extrusion (0,0,-1): the file's OCS values
    // (centre (10,10), arc 0..90 degrees, text at (10,10)) are mirrored
    // about the y axis in world terms. The LINE has no extrusion and never
    // moves.
    let circle = circle.expect("CIRCLE");
    assert_eq!((circle.center.x, circle.center.y), (-10.0, 10.0));
    assert_eq!(circle.center.z, 0.0);
    assert_eq!(circle.radius, 5.0);
    assert_eq!(circle.extrusion.z, -1.0);

    let arc = arc.expect("ARC");
    assert_eq!((arc.center.x, arc.center.y, arc.center.z), (0.0, 0.0, 0.0));
    assert_eq!(arc.radius, 20.0);
    assert_eq!(arc.extrusion.z, -1.0);
    // The first-quadrant arc becomes the second-quadrant one, still
    // counter-clockwise: 90 to 180 degrees (DXF 51 = 90 arrives in radians).
    assert!(
        (arc.start_angle - FRAC_PI_2).abs() < 1e-12,
        "start {}",
        arc.start_angle
    );
    assert!(
        (arc.end_angle - std::f64::consts::PI).abs() < 1e-12,
        "end {}",
        arc.end_angle
    );

    let text = text.expect("TEXT");
    assert_eq!(text.text, "MIRROR");
    assert_eq!((text.start_point.x, text.start_point.y), (-10.0, 10.0));
    assert_eq!(text.text_height, 2.5);

    let line = line.expect("LINE");
    assert_eq!((line.start_point.x, line.start_point.y), (-5.0, -5.0));
    assert_eq!((line.end_point.x, line.end_point.y), (5.0, 5.0));
}

// ------------------------------------------------------------ dimlfac12

#[test]
fn dimlfac12_fixture_has_a_line_and_a_dimension() {
    let db = parse(DIMLFAC12);
    assert_eq!(type_counts(&db), expected(&[("LINE", 1), ("DIMENSION", 1)]));

    let line = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Line(l) => Some(l),
            _ => None,
        })
        .expect("LINE");
    assert_eq!((line.start_point.x, line.start_point.y), (0.0, 0.0));
    assert_eq!((line.end_point.x, line.end_point.y), (10.0, 0.0));

    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("DIMENSION");
    assert_eq!(dim.common.handle, "45");
    // Today only the cached block's name is read. The fixture's BLOCK_RECORD
    // table binds the anonymous *D1 block, whose one TEXT is the cached label
    // "120" (12 x 10 under $DIMLFAC 12). LibreDWG also holds
    // act_measurement = 10.0, user_text = "", def_pt (10,5), text_midpt (5,6)
    // and xline points (0,0)/(10,0). P3: measurement == Some(10.0), display
    // "120.00" (DIMDEC 2), and the label read from the block.
    assert_eq!(dim.block_name, "*D1");
    assert_eq!(db.header.dimlfac, 12.0);
    assert_eq!((db.header.dimdec, db.header.dimlunit), (2, 2));
    assert_eq!(db.tables.block_records["*Model_Space"].entities.len(), 2);
    let cached = &db.tables.block_records["*D1"].entities;
    assert_eq!(cached.len(), 1, "{cached:?}");
    match &cached[0] {
        Entity::Text(t) => assert_eq!(t.text, "120"),
        other => panic!("expected the cached dimension label, got {other:?}"),
    }
}

// ------------------------------------------------------- twisted viewport

#[test]
fn twisted_viewport_fixture_has_a_line_and_a_viewport() {
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
    assert_eq!(vp.common.handle, "2A");
    assert_eq!((vp.center.x, vp.center.y, vp.center.z), (150.0, 100.0, 0.0));
    assert_eq!(vp.width, 200.0);
    assert_eq!(vp.height, 120.0);
    // LibreDWG holds the rest of AcDbViewport for this entity (probe-verified):
    // VIEWCTR (50,25), VIEWSIZE 60, VIEWTWIST 0.5235987755982988 rad (30
    // degrees, converted by the reader), VIEWDIR (0,0,1), on_off 1, id 2,
    // status_flag 32864. P7 exposes them as view_center / view_size / twist /
    // status_flag / id. The fixture's *Paper_Space block owns the VIEWPORT
    // (entmode 1) and *Model_Space the LINE.
    assert_eq!(db.tables.block_records["*Model_Space"].entities.len(), 1);
    let paper = &db.tables.block_records["*Paper_Space"].entities;
    assert_eq!(paper.len(), 1, "{paper:?}");
    assert!(matches!(paper[0], Entity::Viewport(_)), "{paper:?}");
    // And the paper-space render draws that frame alone: the viewBox is the
    // 200 x 120 frame plus the default 5-unit padding on each side.
    let svg = db
        .to_svg(uncad::ToSvgOptions {
            space: uncad::Space::Paper,
            ..Default::default()
        })
        .svg;
    assert!(svg.contains("viewBox=\"45 -165 210 130\""), "{svg}");
}
