//! Block references and what the renderer derives from them beyond the
//! picture: the world extent of a block's contents (the crop, tile
//! membership, `blocks.json` boxes) and the world placement of the texts
//! inside it (`texts.json` anchors). The SVG itself nests
//! `<g transform="matrix(...)">` groups, so it is right by construction;
//! these tests pin the composed transform against it, with every expected
//! number worked out by hand in the comments.

use std::path::{Path, PathBuf};

use uncad::model::{
    ArcEntity, CircleEntity, DimensionEntity, DimensionGeometry, DisplaySource, EntityCommon,
    InsertEntity, LineEntity, Point2D, Point3D, TextEntity,
};
use uncad::tables::BlockRecord;
use uncad::{CropMode, Entity, Rect, ToSvgOptions};

fn common(handle: &str) -> EntityCommon {
    EntityCommon {
        handle: handle.into(),
        layer: "0".into(),
        ..EntityCommon::default()
    }
}

fn p3(x: f64, y: f64) -> Point3D {
    Point3D { x, y, z: 0.0 }
}

fn line(handle: &str, x1: f64, y1: f64, x2: f64, y2: f64) -> Entity {
    Entity::Line(LineEntity {
        common: common(handle),
        start_point: p3(x1, y1),
        end_point: p3(x2, y2),
    })
}

fn circle(handle: &str, cx: f64, cy: f64, r: f64) -> Entity {
    Entity::Circle(CircleEntity {
        common: common(handle),
        center: p3(cx, cy),
        radius: r,
        extrusion: uncad::geom::WORLD_Z,
    })
}

fn arc(handle: &str, cx: f64, cy: f64, r: f64, a0_deg: f64, a1_deg: f64) -> Entity {
    Entity::Arc(ArcEntity {
        common: common(handle),
        center: p3(cx, cy),
        radius: r,
        start_angle: a0_deg.to_radians(),
        end_angle: a1_deg.to_radians(),
        extrusion: uncad::geom::WORLD_Z,
    })
}

/// An INSERT of `block` at `(x, y)` with the given per-axis scale and
/// rotation in degrees.
fn insert(handle: &str, block: &str, x: f64, y: f64, sx: f64, sy: f64, rot_deg: f64) -> Entity {
    Entity::Insert(InsertEntity {
        common: common(handle),
        block_name: block.into(),
        insertion_point: p3(x, y),
        scale: Point3D {
            x: sx,
            y: sy,
            z: 1.0,
        },
        rotation: rot_deg.to_radians(),
        extrusion: uncad::geom::WORLD_Z,
        attribs: Vec::new(),
    })
}

/// A drawing whose model space holds `model` and whose block table holds
/// the named `blocks`.
fn drawing(model: Vec<Entity>, blocks: Vec<(&str, Vec<Entity>)>) -> uncad::CadDatabase {
    let mut tables = uncad::Tables::default();
    for (name, entities) in blocks {
        tables.block_records.insert(
            name.into(),
            BlockRecord {
                name: name.into(),
                entities,
            },
        );
    }
    tables.block_records.insert(
        "*Model_Space".into(),
        BlockRecord {
            name: "*Model_Space".into(),
            entities: model.clone(),
        },
    );
    uncad::CadDatabase::new(model, tables)
}

/// The tight content rectangle the renderer measured (`--crop raw`).
fn content(db: &uncad::CadDatabase) -> Rect {
    db.to_svg(ToSvgOptions {
        crop: CropMode::Raw,
        padding: Some(0.0),
        ..Default::default()
    })
    .crop
    .content
    .expect("something was drawn")
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what}: {got} vs {want}");
}

#[test]
fn a_circle_in_a_block_rotated_45_degrees_keeps_its_extent() {
    // Block C = CIRCLE at (0,0) r 10, inserted at (100,100) rotated 45
    // degrees: the circle is still the circle, x and y 90..110. Considering
    // only the (-10,-10) and (10,10) corners of its box rotates them onto
    // the vertical line x = 100 (both land at 100 +- 0, 100 +- 14.142) and
    // the width collapsed to the 1-unit floor. The four-corner box is
    // conservative -- the rotated square's corners at 100 +- 10 sqrt 2 =
    // 85.858 .. 114.142 -- and always contains the circle.
    let db = drawing(
        vec![insert("I", "C", 100.0, 100.0, 1.0, 1.0, 45.0)],
        vec![("C", vec![circle("C1", 0.0, 0.0, 10.0)])],
    );
    let r = content(&db);
    assert!(
        r.min_x <= 90.0 && r.max_x >= 110.0 && r.min_y <= 90.0 && r.max_y >= 110.0,
        "{r:?} does not cover the circle"
    );
    close(r.min_x, 100.0 - 10.0 * 2f64.sqrt(), "min_x");
    close(r.max_x, 100.0 + 10.0 * 2f64.sqrt(), "max_x");
    close(r.min_y, 100.0 - 10.0 * 2f64.sqrt(), "min_y");
    close(r.max_y, 100.0 + 10.0 * 2f64.sqrt(), "max_y");

    // Unrotated, the box is exact.
    let db = drawing(
        vec![insert("I", "C", 100.0, 100.0, 1.0, 1.0, 0.0)],
        vec![("C", vec![circle("C1", 0.0, 0.0, 10.0)])],
    );
    let r = content(&db);
    close(r.min_x, 90.0, "min_x");
    close(r.max_x, 110.0, "max_x");
}

#[test]
fn a_door_swing_in_a_rotated_block_reaches_its_far_edge() {
    // Block DOOR = LINE (0,0)-(0,900) + ARC centre (0,0) r 900 from 0 to 90
    // degrees, inserted at (1000,1500) rotated 45 degrees. World points:
    // the line end (0,900) -> (1000 - 636.396, 1500 + 636.396) =
    // (363.604, 2136.396); the arc's ends (900,0) -> (1636.396, 2136.396)
    // and (0,900) -> (363.604, 2136.396); its world-45-degree point
    // (636.396, 636.396) -> (1000, 2400). True extent: x 363.604..1636.396,
    // y 1500..2400. The arc's local box (0,0)-(900,900) considered through
    // four corners adds (900,900) -> (1000, 2772.792) at the top, so the
    // measured box is x 363.604..1636.396, y 1500..2772.792 -- it contains
    // the swing. The two-corner box lost the right half of the swing (max
    // x = 1000) and listed the block on the wrong tiles.
    let db = drawing(
        vec![
            insert("D", "DOOR", 1000.0, 1500.0, 1.0, 1.0, 45.0),
            // A border so the drawing has a size of its own.
            line("B1", 0.0, 0.0, 3000.0, 0.0),
            line("B2", 0.0, 0.0, 0.0, 3000.0),
        ],
        vec![(
            "DOOR",
            vec![
                line("DL", 0.0, 0.0, 0.0, 900.0),
                arc("DA", 0.0, 0.0, 900.0, 0.0, 90.0),
            ],
        )],
    );
    let svg = db.to_svg(ToSvgOptions {
        crop: CropMode::Raw,
        padding: Some(0.0),
        ..Default::default()
    });
    // The whole drawing's content is the border plus the door; the door
    // alone is measured by rendering it without the border.
    let door_only = drawing(
        vec![insert("D", "DOOR", 1000.0, 1500.0, 1.0, 1.0, 45.0)],
        vec![(
            "DOOR",
            vec![
                line("DL", 0.0, 0.0, 0.0, 900.0),
                arc("DA", 0.0, 0.0, 900.0, 0.0, 90.0),
            ],
        )],
    );
    let r = content(&door_only);
    let half = 900.0 * std::f64::consts::FRAC_1_SQRT_2;
    close(r.min_x, 1000.0 - half, "min_x");
    close(r.max_x, 1000.0 + half, "max_x");
    close(r.min_y, 1500.0, "min_y");
    close(r.max_y, 1500.0 + 2.0 * half, "max_y");
    assert!(r.max_y >= 2400.0 - 1e-6, "{r:?} loses the top of the swing");
    // And the full drawing's crop covers the swing's far edge too.
    let full = svg.crop.content.unwrap();
    assert!(full.max_x >= 3000.0 - 1e-6 && full.max_y >= 3000.0 - 1e-6);

    // The export lists the instance on every tile its swing touches: with
    // the swing reaching x 1636 the tile column holding x > 1178 (the
    // verifier's r00_c01) is not blank.
    let tmp = TempDir::new("door");
    uncad::export::export_package(
        &db,
        &tmp.0,
        &uncad::export::ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    let blocks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.0.join("blocks.json")).unwrap()).unwrap();
    // The INSERT instances are records (`blocks.json`, or `blocks.NNN.json`
    // when they shard); this two-entity drawing writes one file.
    let instance = blocks["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "D")
        .expect("the door instance");
    let bbox = instance["bbox"].as_array().unwrap();
    assert!(
        bbox[2].as_f64().unwrap() >= 1636.0 && bbox[3].as_f64().unwrap() >= 2400.0,
        "{bbox:?}"
    );
    let tiles: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.0.join("tiles.json")).unwrap()).unwrap();
    // Every written tile whose world rectangle meets the true swing box
    // lists the instance.
    let swing = Rect::new(1000.0 - half, 1500.0, 1000.0 + half, 2400.0);
    let listed: Vec<&str> = instance["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    let mut checked = 0;
    for tile in tiles["tiles"].as_array().unwrap() {
        let w = tile["world"].as_array().unwrap();
        let world = Rect::new(
            w[0].as_f64().unwrap(),
            w[1].as_f64().unwrap(),
            w[2].as_f64().unwrap(),
            w[3].as_f64().unwrap(),
        );
        // Only the tiles the swing crosses by a margin (not a grazing
        // touch that the 16 px culling margin decides).
        let inner = Rect::new(
            swing.min_x + 50.0,
            swing.min_y + 50.0,
            swing.max_x - 50.0,
            swing.max_y - 50.0,
        );
        if !world.intersects(&inner) {
            continue;
        }
        checked += 1;
        let id = tile["id"].as_str().unwrap();
        assert!(listed.contains(&id), "{id} ({world:?}) misses the door");
    }
    assert!(checked >= 2, "the swing spans several tiles: {checked}");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("blocks_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(handle: &str, x: f64, y: f64, height: f64, value: &str) -> Entity {
    Entity::Text(TextEntity {
        common: common(handle),
        start_point: Point2D { x, y },
        text_height: height,
        text: value.into(),
        text_plain: value.into(),
        rotation: 0.0,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    })
}

/// Block B = LINE (0,0)-(10,0) + LINE (10,0)-(10,2) + TEXT "T" at (10,0);
/// block A = INSERT of B rotated 90 degrees; the model = INSERT of A at
/// (100,100) with the given scale and extrusion.
fn nested(sx: f64, sy: f64, extrusion_z: f64) -> uncad::CadDatabase {
    let mut top = insert("A1", "A", 100.0, 100.0, sx, sy, 0.0);
    if let Entity::Insert(i) = &mut top {
        i.extrusion = Point3D {
            x: 0.0,
            y: 0.0,
            z: extrusion_z,
        };
    }
    drawing(
        vec![top],
        vec![
            (
                "B",
                vec![
                    line("BL1", 0.0, 0.0, 10.0, 0.0),
                    line("BL2", 10.0, 0.0, 10.0, 2.0),
                    text("BT", 10.0, 0.0, 0.5, "T"),
                ],
            ),
            ("A", vec![insert("AB", "B", 0.0, 0.0, 1.0, 1.0, 90.0)]),
        ],
    )
}

#[test]
fn a_rotated_block_inside_a_mirrored_one_is_measured_where_it_is_drawn() {
    // A's 90-degree turn sends B's (10,0) to (0,10) and (10,2) to (-2,10);
    // the outer x scale -1 then sends those to (100,110) and (102,110):
    // the picture (matrix(-1 0 0 1 100 -100) around matrix(0 -1 1 0 0 0))
    // has the lines at x 100..102, y 100..110 and the text's anchor at
    // (100,110). The old rotation-sum composition put everything at y
    // 90..100 and the anchor at (100,90), outside the drawn geometry, so
    // `--crop raw` showed a blank image.
    for (name, db) in [
        ("x scale -1", nested(-1.0, 1.0, 1.0)),
        ("extrusion (0,0,-1)", nested(1.0, 1.0, -1.0)),
    ] {
        let r = content(&db);
        assert!(
            r.min_x >= 100.0 - 1e-9 && r.min_y >= 100.0 - 1e-9,
            "{name}: {r:?}"
        );
        // The lines end at (102,110); the text's estimate (0.5 high, one
        // 0.41-unit character running up the page from (100,110)) adds a
        // little on top.
        assert!(
            r.max_y >= 110.0 - 1e-9 && r.max_y <= 111.0 && r.max_x >= 102.0 - 1e-9,
            "{name}: {r:?}"
        );
        assert!(
            r.max_x < 104.0,
            "{name}: only the 2-unit stub and a 0.5 text: {r:?}"
        );

        let tmp = TempDir::new("mirrot");
        uncad::export::export_package(
            &db,
            &tmp.0,
            &uncad::export::ExportOptions {
                max_levels: 1,
                ..Default::default()
            },
        )
        .expect("exports");
        let texts: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.0.join("texts.json")).unwrap())
                .unwrap();
        let t = texts["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "A1/AB/BT")
            .expect("the nested text");
        let anchor = t["anchor"].as_array().unwrap();
        assert!(
            (anchor[0].as_f64().unwrap() - 100.0).abs() < 1e-6
                && (anchor[1].as_f64().unwrap() - 110.0).abs() < 1e-6,
            "{name}: {anchor:?}"
        );
        // The measured glyph box sits at the anchor, not 20 units away.
        let b = t["bbox"].as_array().unwrap();
        assert_eq!(t["bbox_confidence"], "measured");
        assert!(
            (b[1].as_f64().unwrap() - 110.0).abs() < 0.5 && b[0].as_f64().unwrap() >= 99.5,
            "{name}: {b:?}"
        );
        assert!(
            !t["tiles"].as_array().unwrap().is_empty(),
            "{name}: on a tile"
        );
        // The instance's box is the drawn one.
        let blocks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.0.join("blocks.json")).unwrap())
                .unwrap();
        let bbox = blocks["records"][0]["bbox"].as_array().unwrap();
        assert!(
            bbox[1].as_f64().unwrap() >= 100.0 - 1e-6 && bbox[3].as_f64().unwrap() >= 110.0 - 1e-6,
            "{name}: {bbox:?}"
        );
    }

    // A non-uniform outer scale (2, 1): (0,10) -> (100,110), (-2,10) ->
    // (96,110): content x 96..100, y 100..110 (plus the text). No
    // origin + rotation + scale composition can express this frame.
    let r = content(&nested(2.0, 1.0, 1.0));
    assert!(
        r.min_x <= 96.0 + 1e-9 && r.min_x >= 95.0 && r.max_y >= 110.0 - 1e-9 && r.max_y <= 111.0,
        "{r:?}"
    );
    assert!(r.min_y >= 100.0 - 1e-9 && r.max_x <= 101.0, "{r:?}");
}

/// A DIMENSION and the cached `*D1` geometry block AutoCAD writes for it.
/// That block's contents are in *world* coordinates, which is why the
/// renderer places it through an identity transform.
fn dimension(handle: &str, block: &str, x: f64, y: f64) -> Entity {
    Entity::Dimension(DimensionEntity {
        common: common(handle),
        block_name: block.into(),
        geometry: DimensionGeometry::Unknown,
        measurement: Some(100.0),
        measurement_from_points: None,
        user_text: String::new(),
        display_text: "100".into(),
        display_text_raw: "100".into(),
        display_source: DisplaySource::None,
        definition_point: p3(x, y),
        text_midpoint: Point2D { x, y },
        dimstyle: String::new(),
        dimlfac: 1.0,
    })
}

/// Two plain LINEs and one 100-unit DIMENSION, the whole thing offset by
/// `(ox, oy)`. Every model entity's reference point is at `(ox, oy)` except
/// the dimension's definition point, so the median origin the renderer
/// chooses is exactly `(ox, oy)`.
fn dimensioned(ox: f64, oy: f64) -> uncad::CadDatabase {
    let cached = vec![
        line("50", ox, oy + 20.0, ox + 100.0, oy + 20.0),
        line("51", ox, oy, ox, oy + 20.0),
        line("52", ox + 100.0, oy, ox + 100.0, oy + 20.0),
        text("53", ox + 40.0, oy + 22.0, 5.0, "100"),
    ];
    drawing(
        vec![
            line("A", ox, oy, ox + 100.0, oy),
            line("B", ox, oy, ox, oy + 40.0),
            dimension("60", "*D1", ox + 100.0, oy),
        ],
        vec![("*D1", cached)],
    )
}

/// The largest magnitude among the numbers written into `svg`: every
/// whitespace- or comma-separated token of every quoted attribute value
/// that parses as one.
fn max_magnitude(svg: &str) -> f64 {
    let mut max: f64 = 0.0;
    let mut rest = svg;
    while let Some(at) = rest.find('"') {
        let after = &rest[at + 1..];
        let Some(end) = after.find('"') else { break };
        for token in after[..end].split([' ', ',']) {
            if let Ok(v) = token.trim_start_matches(['M', 'A', 'L']).parse::<f64>() {
                max = max.max(v.abs());
            }
        }
        rest = &after[end + 1..];
    }
    max
}

#[test]
fn a_dimension_far_from_the_origin_is_drawn_where_the_records_say() {
    // A site plan in millimetres at projected coordinates. Above 32768
    // units the renderer writes its SVG relative to the drawing's own
    // origin, because usvg and tiny-skia keep path points and transforms in
    // f32 -- at 2.5e8 the f32 step is 16 units, so anything written at full
    // world magnitude is quantised away. A DIMENSION's cached block used to
    // be exempt: its interior was written unshifted inside a group that
    // carried the whole 2.5e8 translation, so every dimension line and
    // label vanished while the plain LINEs of the same drawing drew
    // perfectly.
    let far = 2.5e8;
    let options = ToSvgOptions {
        crop: CropMode::Raw,
        padding: Some(0.0),
        ..Default::default()
    };
    let here = dimensioned(0.0, 0.0).to_svg(options);
    let there = dimensioned(far, far).to_svg(options);
    assert_eq!(here.origin, [0.0, 0.0]);
    assert_eq!(there.origin, [far, far]);

    // Every coordinate written is its world value minus the origin, so the
    // far drawing's document is the near one's, character for character.
    assert_eq!(there.svg, here.svg);

    // The cached block's group carries no shift of its own at all: its
    // children are already written in the render's frame.
    assert!(there.svg.contains("matrix(1 0 0 1 0 0)"), "{}", there.svg);

    // And they land where the block record says. The dimension line runs
    // (ox, oy+20) to (ox+100, oy+20), which minus the origin (ox, oy) and
    // with y flipped for SVG is (0,-20) to (100,-20); the extension lines
    // are (0,0)-(0,-20) and (100,0)-(100,-20); the label's anchor is
    // (ox+40, oy+22) -> (40,-22).
    for expected in [
        "<line x1=\"0\" y1=\"-20\" x2=\"100\" y2=\"-20\"",
        "<line x1=\"0\" y1=\"0\" x2=\"0\" y2=\"-20\"",
        "<line x1=\"100\" y1=\"0\" x2=\"100\" y2=\"-20\"",
        "<text id=\"60/53\" x=\"40\" y=\"-22\"",
    ] {
        assert!(there.svg.contains(expected), "{expected} in {}", there.svg);
    }
    assert!(there.svg.contains(">100</text>"), "{}", there.svg);

    // Nothing anywhere in the document is at world magnitude: the largest
    // number is the 104-unit viewBox width, and an f32 resolves that to
    // better than a millionth of a unit.
    assert!(max_magnitude(&there.svg) < 1.0e4, "{}", there.svg);

    // The same holds for an ordinary block reference whose contents sit far
    // from its own base point: block W is inserted at the world origin but
    // drawn at (far, far), which is the other shape of the same bug.
    let world_block = drawing(
        vec![
            insert("I", "W", 0.0, 0.0, 1.0, 1.0, 0.0),
            line("C", far, far, far + 1.0, far),
        ],
        vec![("W", vec![line("WL", far, far, far + 50.0, far + 50.0)])],
    );
    let svg = world_block.to_svg(options).svg;
    assert!(max_magnitude(&svg) < 1.0e4, "{svg}");
    assert!(svg.contains("x2=\"50\" y2=\"-50\""), "{svg}");
}
