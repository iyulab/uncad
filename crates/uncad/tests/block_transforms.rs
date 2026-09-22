//! Block references and what the renderer derives from them beyond the
//! picture: the world extent of a block's contents (the crop, tile
//! membership, `blocks.json` boxes) and the world placement of the texts
//! inside it (`texts.json` anchors). The SVG itself nests
//! `<g transform="matrix(...)">` groups, so it is right by construction;
//! these tests pin the composed transform against it, with every expected
//! number worked out by hand in the comments.

use std::path::{Path, PathBuf};

use uncad::model::{ArcEntity, CircleEntity, EntityCommon, InsertEntity, LineEntity, Point3D};
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
    let instance = blocks["instances"]
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
