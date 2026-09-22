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
const MIRRORED_BULGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mirrored_bulge_r2000.dxf"
);

const HATCHED_VIEWPORT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hatched_viewport_r2000.dxf"
);

const NESTED_ATTRIB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/nested_attrib_r2000.dxf"
);

const HIDDEN_LAYERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hidden_layers_r2000.dxf"
);

const PLOT_ORIGIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/plot_origin_r2000.dxf"
);

const ANGULAR_ORDINATE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/angular_ordinate_r2000.dxf"
);

const VIEWPORT_STATES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/viewport_states_r2000.dxf"
);

const RADIAL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/radial_r2000.dxf"
);

const INFINITE_LINES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/infinite_lines_r2000.dxf"
);

const POLYLINE_VERTICES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/polyline_vertices_r2000.dxf"
);

const ENTITY_TRUECOLOR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/entity_truecolor_r2000.dxf"
);

const POLYFACE_MESH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/polyface_mesh_r2000.dxf"
);

const BLOCK_LAYER0: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/block_layer0_r2000.dxf"
);

const TITLE_BLOCK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/title_block_r2000.dxf"
);

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
    // 200 x 120 frame plus 5 units of padding on each side.
    let svg = db
        .to_svg(uncad::ToSvgOptions {
            space: uncad::Space::Paper,
            padding: Some(5.0),
            ..Default::default()
        })
        .svg;
    assert!(svg.contains("viewBox=\"45 -165 210 130\""), "{svg}");
}

// ------------------------------------------------------- infinite lines

/// Decoded PNG pixels, enough to ask whether a drawing point was inked.
struct Image {
    width: u32,
    height: u32,
    samples: usize,
    data: Vec<u8>,
}

impl Image {
    fn decode(bytes: &[u8]) -> Image {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("valid PNG");
        let info = reader.info().clone();
        let mut data = vec![0u8; (info.width * info.height * 4) as usize];
        let frame = reader.next_frame(&mut data).expect("decodable frame");
        data.truncate(frame.buffer_size());
        Image {
            width: frame.width,
            height: frame.height,
            samples: frame.color_type.samples(),
            data,
        }
    }

    /// The red channel at `(x, y)`, or 255 (white) outside the image.
    fn value(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return 255;
        }
        self.data[(y as usize * self.width as usize + x as usize) * self.samples]
    }

    /// The darkest pixel within two of `(px, py)`: a 1.25 px stroke lands
    /// wherever rounding puts it, and antialiasing splits it over two
    /// pixels, so the exact pixel is not the thing to ask about.
    fn darkest_near(&self, (px, py): (f64, f64)) -> u8 {
        let (x0, y0) = (px.round() as i64, py.round() as i64);
        let mut darkest = 255;
        for dy in -2..=2 {
            for dx in -2..=2 {
                darkest = darkest.min(self.value(x0 + dx, y0 + dy));
            }
        }
        darkest
    }

    /// How much of the row `py` is ink, as a fraction of the width.
    fn inked_fraction_of_row(&self, py: f64) -> f64 {
        let y = py.round() as i64;
        let inked = (0..i64::from(self.width))
            .filter(|&x| (-1..=1).any(|dy| self.value(x, y + dy) < INK))
            .count();
        inked as f64 / f64::from(self.width)
    }

    /// Whether some row, and whether some column, is inked nearly end to
    /// end -- a line crossing the whole image. One pass over the pixels:
    /// asking row by row is the same answer and a hundred times the work
    /// on a tile pyramid.
    fn crossed_end_to_end(&self) -> (bool, bool) {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut rows = vec![0usize; h];
        let mut cols = vec![0usize; w];
        for (y, row) in rows.iter_mut().enumerate() {
            for (x, column) in cols.iter_mut().enumerate() {
                if self.data[(y * w + x) * self.samples] < INK {
                    *row += 1;
                    *column += 1;
                }
            }
        }
        (
            rows.iter().any(|&n| n as f64 > 0.9 * w as f64),
            cols.iter().any(|&n| n as f64 > 0.9 * h as f64),
        )
    }
}

const INK: u8 = 200;

/// The fixture's construction lines: the XLINE first, then the RAY.
fn construction_lines(
    db: &uncad::CadDatabase,
) -> (&uncad::model::RayEntity, &uncad::model::RayEntity) {
    let mut xline = None;
    let mut ray = None;
    for e in &db.entities {
        match e {
            Entity::XLine(x) => xline = Some(x),
            Entity::Ray(r) => ray = Some(r),
            _ => {}
        }
    }
    (xline.expect("XLINE"), ray.expect("RAY"))
}

#[test]
fn infinite_lines_fixture_has_a_line_a_text_an_xline_and_a_ray() {
    let db = parse(INFINITE_LINES);
    assert_eq!(
        type_counts(&db),
        expected(&[("LINE", 1), ("TEXT", 1), ("XLINE", 1), ("RAY", 1)])
    );
    let (xline, ray) = construction_lines(&db);
    assert_eq!(xline.common.handle, "32");
    assert_eq!((xline.point.x, xline.point.y), (0.001, 0.001));
    assert_eq!((xline.vector.x, xline.vector.y), (1.0, 0.0));
    assert_eq!(ray.common.handle, "33");
    assert_eq!((ray.point.x, ray.point.y), (0.001, 0.001));
    assert_eq!((ray.vector.x, ray.vector.y), (0.0, 1.0));
}

#[test]
fn an_infinite_line_never_enlarges_the_crop() {
    let db = parse(INFINITE_LINES);
    let mut finite = parse(INFINITE_LINES);
    finite
        .entities
        .retain(|e| !matches!(e, Entity::Ray(_) | Entity::XLine(_)));
    assert_eq!(finite.entities.len(), 2, "the LINE and the TEXT are left");

    let with = db.to_svg(uncad::ToSvgOptions::default());
    let without = finite.to_svg(uncad::ToSvgOptions::default());
    // A RAY and an XLINE reach every corner of the world, so only their
    // base point may count towards the crop -- and that one sits inside
    // the LINE's box, which leaves the picture exactly as it was.
    assert_eq!(
        with.view_box, without.view_box,
        "the construction lines moved the crop"
    );
    // They are still drawn -- two dashed elements -- and every coordinate
    // in them is within a few pictures of the picture, not at the 1e6-unit
    // endpoint the renderer used to give them (which is what overflowed
    // the rasterizer at this scale).
    let dashed: Vec<&str> = with
        .svg
        .lines()
        .filter(|l| l.contains("stroke-dasharray"))
        .collect();
    assert_eq!(dashed.len(), 2, "{}", with.svg);
    let (x0, _, x1, _) = with.view_box.world_bounds();
    let reach = with.view_box.width.max(with.view_box.height) * 4.0;
    for element in dashed {
        for coordinate in element.split('"').filter_map(|t| t.parse::<f64>().ok()) {
            assert!(
                coordinate.abs() <= reach,
                "{coordinate} is far outside a {} unit picture: {element}",
                x1 - x0
            );
        }
    }
}

#[test]
fn the_construction_lines_are_cut_to_the_image_at_every_size() {
    let db = parse(INFINITE_LINES);
    // Every size the old 1e6-unit segment either panicked tiny-skia at
    // (256, 8000) or survived by luck. The base point is (0.001, 0.001):
    // the XLINE runs left and right through it, the RAY only upwards.
    for fit in [256u32, 420, 512, 1024, 1568] {
        let result = db
            .to_png(uncad::ToPngOptions {
                size: uncad::PngSize::FitLongEdge(fit),
                ..Default::default()
            })
            .unwrap_or_else(|e| panic!("{fit} px: {e}"));
        let image = Image::decode(&result.png);
        let at = |x: f64, y: f64| result.view_box.world_to_px(x, y, result.px_per_unit);
        for (point, what) in [
            ((0.0004, 0.001), "the XLINE to the left of its base point"),
            ((0.0016, 0.001), "the XLINE to the right of its base point"),
            ((0.001, 0.0016), "the RAY above its base point"),
        ] {
            assert!(
                image.darkest_near(at(point.0, point.1)) < INK,
                "{fit} px: {what} is missing"
            );
        }
        assert_eq!(
            image.darkest_near(at(0.001, 0.0004)),
            255,
            "{fit} px: the RAY was drawn backwards, below its base point"
        );
        assert!(
            image.inked_fraction_of_row(at(0.001, 0.001).1) > 0.9,
            "{fit} px: the XLINE does not cross the whole image"
        );
    }
}

#[test]
fn a_deep_tile_pyramid_keeps_the_construction_lines_and_does_not_panic() {
    use uncad::export::{export_package, ExportOptions};

    let db = parse(INFINITE_LINES);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("infinite_lines_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // The drawing is 0.002 units across, so its one text already stands a
    // thousand pixels tall in the overview and the pyramid would stop at
    // one level; an absurd legibility target asks for the deep one the
    // panic needed. Level 3 is ~4.2 million pixels per drawing unit, where
    // the segment the renderer used to draw would have reached 4e12 px.
    let options = ExportOptions {
        target_text_px: 50_000.0,
        max_levels: 3,
        max_tiles: 200,
        ..Default::default()
    };
    let report = export_package(&db, &dir, &options).expect("the package must be written");
    let frame = report.frames.first().expect("one frame");
    assert_eq!(frame.levels.len(), 3, "{:?}", frame.levels);
    let deepest = frame.levels.last().expect("a deepest level");
    assert!(deepest.ppu > 4e6, "{}", deepest.ppu);

    // The XLINE crosses a whole row of the deepest level's tiles and the
    // RAY a whole column, although the extent of both is one base point in
    // a single tile: a window that keeps entities by their extent has to
    // make an exception for the lines that have none.
    //
    // (The world rectangles in `tiles.json` are rounded to the drawing's
    // own `$LUPREC` precision, 3 decimals, which says nothing at this
    // scale -- so the tiles are read as images, not as coordinates.)
    let tiles: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tiles.json")).expect("tiles.json"))
            .expect("tiles.json is JSON");
    let mut crossed_by_the_xline = 0;
    let mut crossed_by_the_ray = 0;
    let mut deepest_tiles = 0;
    for tile in tiles["tiles"].as_array().expect("tiles") {
        if tile["z"].as_u64() != Some(u64::from(deepest.z)) {
            continue;
        }
        deepest_tiles += 1;
        let png =
            std::fs::read(dir.join(tile["png"].as_str().expect("a path"))).expect("the tile image");
        let (row, column) = Image::decode(&png).crossed_end_to_end();
        crossed_by_the_xline += usize::from(row);
        crossed_by_the_ray += usize::from(column);
    }
    assert!(
        deepest_tiles >= 9,
        "{deepest_tiles} tiles at the deepest level"
    );
    assert!(
        crossed_by_the_xline >= 5,
        "only {crossed_by_the_xline} of {deepest_tiles} deep tiles show the XLINE across them"
    );
    assert!(
        crossed_by_the_ray >= 2,
        "only {crossed_by_the_ray} of {deepest_tiles} deep tiles show the RAY down them"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------------------------- polyline vertices

/// Regression for the last vertex LibreDWG's own
/// `dwg_object_polyline_{2,3}d_get_points` dropped on every R13/R14/R2000
/// file. Every number below comes from the fixture's own generator (see
/// `tests/fixtures/README.md`), not from this crate's output: the square is
/// 100 by 100, so its perimeter is 400 and its area 10000; the arc
/// polyline's single bulge of 1.0 is a half turn over a 100-unit chord,
/// i.e. a semicircle of radius 50, whose length is `pi * 50`; and the 3D
/// polyline's five points are listed in the generator.
///
/// Before the fix each of these came back one vertex short: the square was
/// a right triangle (area 5000, perimeter 100 + 100 + 141.42), the arc
/// polyline was a single point whose bulge list had been cleared for
/// disagreeing with the vertex count, and the 3D polyline ended at
/// (0, 10, 5).
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
    assert_eq!(square.common.handle, "30");
    assert!(square.closed);
    assert_eq!(
        square
            .vertices
            .iter()
            .map(|v| (v.x, v.y))
            .collect::<Vec<_>>(),
        [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
    );
    assert_eq!(square.length(), 400.0, "4 sides of 100");
    assert_eq!(square.area(), Some(10000.0), "100 x 100");

    let arc = match &db.entities[1] {
        Entity::Polyline2D(p) => p,
        other => panic!("expected the arc polyline second, got {other:?}"),
    };
    assert_eq!(arc.common.handle, "35");
    assert_eq!(
        arc.vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>(),
        [(0.0, 1000.0), (100.0, 1000.0)]
    );
    assert_eq!(arc.bulges, [1.0, 0.0], "a bulge per vertex, or none at all");
    let semicircle = std::f64::consts::PI * 50.0;
    assert!(
        (arc.length() - semicircle).abs() < 1e-6,
        "{} vs {semicircle}",
        arc.length()
    );

    let p3d = match &db.entities[2] {
        Entity::Polyline3D(p) => p,
        other => panic!("expected the 3D polyline third, got {other:?}"),
    };
    assert_eq!(p3d.common.handle, "39");
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

    // And the arc really is drawn: an `A` path command, from the one bulge.
    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    assert!(svg.contains(" A "), "no arc in the render: {svg}");
}

// ------------------------------------------------------ entity truecolor

/// Regression for the two halves of the DXF colour read: a real group 420
/// was dropped (the entity fell back to its layer's colour) and a plain
/// group 62 was reported as a `true_color` the file never wrote.
///
/// The expected values are the fixture's own group codes: 65407 is
/// `0x00ff7f` and 255 is `0x0000ff`. The layer is ACI 3 (`#00ff00`, which
/// the renderer darkens to `#00c300` for the white page), so an entity that
/// fell back to its layer is recognisable in the SVG.
#[test]
fn a_dxf_entity_carries_the_true_colour_it_states_and_no_other() {
    let db = parse(ENTITY_TRUECOLOR);
    let colors: Vec<(&str, i16, Option<u32>)> = db
        .entities
        .iter()
        .map(|e| {
            let c = e.common();
            (c.handle.as_str(), c.color_index, c.true_color)
        })
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

    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    let strokes: Vec<&str> = svg
        .match_indices("stroke=\"#")
        .map(|(i, _)| &svg[i + 8..i + 15])
        .collect();
    // 420 wins over the layer; 420 wins over 62; 62 is the ACI palette's own
    // red; no colour at all is the layer's green, darkened for white.
    assert_eq!(
        strokes,
        ["#00b259", "#ff0000", "#0000ff", "#00c300"],
        "{svg}"
    );
}

// --------------------------------------------------------- mesh polylines

/// Regression for a DXF that could not be read at all because of one
/// polygon mesh, and for a polyface mesh that drew nothing while its
/// vertices were reported as top-level entities.
///
/// The edge counts are the grids the generator writes, worked out from the
/// mesh definitions rather than from the output: the polyface has two quad
/// faces, so 2 * 4 = 8 edges; the polygon mesh is an open 3 by 4 grid, so
/// 4 * (3 - 1) columns plus 3 * (4 - 1) rows = 17.
#[test]
fn a_dxf_with_mesh_polylines_reads_and_both_meshes_have_geometry() {
    let db = parse(POLYFACE_MESH);
    assert_eq!(
        type_counts(&db),
        expected(&[("LINE", 1), ("POLYLINE_PFACE", 1), ("POLYLINE_MESH", 1),]),
        "no VERTEX record may be reported as an entity"
    );
    // The same three, and only those three, in the block record.
    let model = &db.tables.block_records["*Model_Space"].entities;
    assert_eq!(model.len(), 3, "{model:?}");

    let edges = |e: &Entity| match e {
        Entity::PolylinePFace(p) | Entity::PolylineMesh(p) => p.wireframe_edges.len(),
        other => panic!("expected a mesh, got {other:?}"),
    };
    assert_eq!(edges(&db.entities[1]), 8, "2 quad faces");
    assert_eq!(edges(&db.entities[2]), 17, "4 * 2 columns + 3 * 3 rows");

    // Both meshes reach the image, and nothing is reported as unsupported.
    let render = db.to_svg(uncad::ToSvgOptions::default());
    assert!(
        render.unsupported_types.is_empty(),
        "{:?}",
        render.unsupported_types
    );
    assert_eq!(
        render.svg.matches("<line").count(),
        1 + 8 + 17,
        "{}",
        render.svg
    );
}

// ------------------------------------------------------- block on layer 0

/// Regression for AutoCAD's layer-0-in-a-block rule: a BYLAYER child drawn
/// on layer 0 inside a block definition resolves against the layer of the
/// INSERT, not against layer 0.
///
/// The fixture inserts block `SYM` on layer `RED` (ACI 1 = `#ff0000`) and
/// gives it one layer-0 BYLAYER child, one layer-0 BYBLOCK child and one
/// child on layer `BLUE` (ACI 5 = `#0000ff`). Layer 0 is ACI 7, which this
/// renderer draws black, so before the fix the first two children were
/// `#000000`.
#[test]
fn block_geometry_on_layer_0_takes_the_inserts_layer() {
    let db = parse(BLOCK_LAYER0);
    // The model keeps what the file stores: the children are still on
    // layer 0. The rule is a property of the reference, not of the block.
    let children = &db.tables.block_records["SYM"].entities;
    let layers: Vec<&str> = children.iter().map(|e| e.common().layer.as_str()).collect();
    assert_eq!(layers, ["0", "0", "BLUE", "0"]);

    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    let strokes: Vec<&str> = svg
        .match_indices("stroke=\"#")
        .map(|(i, _)| &svg[i + 8..i + 15])
        .collect();
    assert_eq!(
        strokes,
        ["#ff0000", "#ff0000", "#0000ff"],
        "BYLAYER and BYBLOCK on layer 0 both follow the INSERT: {svg}"
    );
    // The TEXT is drawn in the INSERT's colour too.
    assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
}

// ------------------------------------------------------------ title_block

#[test]
fn the_title_block_fixture_keeps_every_string_in_paper_space() {
    // The shape the paper-space text index exists for: the model holds one
    // LINE and nothing else, while the sheet carries the drawing's name.
    // Read off `make_fixtures.py`'s `title_block()`, which writes these
    // four entities and this one block.
    let db = parse(TITLE_BLOCK);
    assert_eq!(
        type_counts(&db),
        expected(&[("INSERT", 1), ("LINE", 1), ("TEXT", 1), ("VIEWPORT", 1)])
    );
    let model = &db.tables.block_records["*Model_Space"];
    assert_eq!(model.entities.len(), 1);
    assert!(matches!(model.entities[0], uncad::Entity::Line(_)));
    let paper = &db.tables.block_records["*Paper_Space"];
    let paper_texts: Vec<&str> = paper
        .entities
        .iter()
        .filter_map(|e| match e {
            uncad::Entity::Text(t) => Some(t.text_plain.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(paper_texts, ["GARDEN PAVILION"]);
    let block = &db.tables.block_records["TITLEBLOCK"];
    let inside: Vec<&str> = block
        .entities
        .iter()
        .filter_map(|e| match e {
            uncad::Entity::Text(t) => Some(t.text_plain.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(inside, ["SHEET 1 OF 2"]);
    assert_eq!(db.tables.layouts.len(), 1);
}
