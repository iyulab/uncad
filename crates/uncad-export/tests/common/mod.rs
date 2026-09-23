//! What the package tests share: a scratch directory, JSON reading, record
//! collection across shards, and small drawings built on the model's own
//! types -- model space is what the renderer draws, so an entity has to be
//! both at the top level and in the `*Model_Space` block record.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use uncad_export::{export_package, ExportError, ExportOptions, ExportReport};
use uncad_model::model::{
    Confidence, Entity, EntityCommon, EntityId, LineEntity, LwPolylineEntity, MTextAttachment,
    MTextEntity, Origin, Point2D, Point3D, PolylineVertex, Ref, TextEntity,
};
use uncad_model::tables::BlockRecord;
use uncad_model::{CadDatabase, Tables};

pub const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);
pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../uncad/tests/fixtures");

/// A corpus file under `lib/libredwg/test/test-data`.
pub fn corpus(name: &str) -> String {
    format!("{TEST_DATA}/{name}")
}

/// One of `crates/uncad/tests/fixtures`.
pub fn fixture(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

/// A fresh directory under the target dir, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("export_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every record of `name` (`texts`, `geometry`, ...), whether written whole
/// or in shards.
pub fn records(dir: &Path, name: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let single = dir.join(format!("{name}.json"));
    if single.exists() {
        out.extend(read_json(&single)["records"].as_array().unwrap().clone());
        return out;
    }
    let mut shards: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{name}.")) && n.ends_with(".json"))
        })
        .collect();
    shards.sort();
    assert!(!shards.is_empty(), "{name}.json or its shards");
    for shard in shards {
        out.extend(read_json(&shard)["records"].as_array().unwrap().clone());
    }
    out
}

/// The record of `records` whose `handle` is `handle`.
pub fn by_handle<'a>(records: &'a [Value], handle: &str) -> &'a Value {
    records
        .iter()
        .find(|r| r["handle"] == handle)
        .unwrap_or_else(|| panic!("a record with handle {handle}"))
}

/// Parses a file with its header and exports it.
pub fn export_path(
    path: &str,
    dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let (db, header) = uncad::parse_with_header(path).expect("the drawing must parse");
    export_package(&db, Some(&header), dir, options)
}

/// Exports a drawing built here, which has no header.
pub fn export_db(
    db: &CadDatabase,
    dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    export_package(db, None, dir, options)
}

/// Dark (< 128) pixels of an 8-bit RGB PNG, inside `[x0, y0, x1, y1]` when
/// given.
pub fn dark_pixels_in(png: &[u8], area: Option<[i64; 4]>) -> usize {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("a frame size")];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(
        info.color_type,
        png::ColorType::Rgb,
        "the package writes RGB"
    );
    let (w, h) = (i64::from(info.width), i64::from(info.height));
    let [x0, y0, x1, y1] = area.unwrap_or([0, 0, w, h]);
    let mut count = 0;
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w) {
            if buf[((y * w + x) * 3) as usize] < 128 {
                count += 1;
            }
        }
    }
    count
}

pub fn dark_pixels(png: &[u8]) -> usize {
    dark_pixels_in(png, None)
}

// ------------------------------------------------------------ drawings

/// An entity's common fields: its ID `id`, its handle the ID in hex, on
/// layer 0, by layer, as a reader of a file would state them.
pub fn common(id: u64) -> EntityCommon {
    EntityCommon {
        id: EntityId::new(id),
        origin: Origin::Vector,
        confidence: Confidence::High,
        source_handle: Ref::Resolved(format!("{id:X}")),
        layer: Ref::Resolved("0".into()),
        color_index: 256,
        true_color: None,
        invisible: false,
    }
}

pub fn on_layer(mut common: EntityCommon, layer: &str) -> EntityCommon {
    common.layer = Ref::Resolved(layer.into());
    common
}

pub fn p3(x: f64, y: f64) -> Point3D {
    Point3D { x, y, z: 0.0 }
}

pub fn z_axis() -> Point3D {
    Point3D {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

pub fn line(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
    Entity::Line(LineEntity {
        common: common(id),
        start_point: p3(x0, y0),
        end_point: p3(x1, y1),
    })
}

pub fn text(id: u64, x: f64, y: f64, height: f64, s: &str) -> Entity {
    Entity::Text(TextEntity {
        common: common(id),
        start_point: Point2D { x, y },
        text_height: height,
        text: s.into(),
        rotation: 0.0,
        horizontal_justification: Default::default(),
        vertical_justification: Default::default(),
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style_name: Ref::Absent,
        elevation: 0.0,
        extrusion: z_axis(),
    })
}

pub fn mtext(id: u64, x: f64, y: f64, height: f64, s: &str) -> Entity {
    Entity::MText(MTextEntity {
        common: common(id),
        insertion_point: p3(x, y),
        text: s.into(),
        text_height: height,
        rotation: 0.0,
        line_spacing_factor: 1.0,
        attachment: Some(MTextAttachment::TopLeft),
        rect_width: 0.0,
        extents_width: None,
        extents_height: None,
        style_name: Ref::Absent,
    })
}

pub fn lwpolyline(id: u64, points: &[(f64, f64)], bulges: &[f64], closed: bool) -> Entity {
    Entity::LwPolyline(LwPolylineEntity {
        common: common(id),
        vertices: points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| PolylineVertex {
                point: Point2D { x: *x, y: *y },
                bulge: bulges.get(i).copied().unwrap_or(0.0),
                start_width: 0.0,
                end_width: 0.0,
            })
            .collect(),
        closed,
        const_width: 0.0,
        elevation: 0.0,
        extrusion: z_axis(),
    })
}

/// A drawing whose model space is `entities`, wired up the way a parsed
/// file has it.
pub fn model_space(entities: Vec<Entity>) -> CadDatabase {
    with_blocks(entities, Vec::new())
}

/// [`model_space`] with block definitions beside it.
pub fn with_blocks(entities: Vec<Entity>, blocks: Vec<(&str, Vec<Entity>)>) -> CadDatabase {
    let mut tables = Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    for (name, block) in blocks {
        tables.block_records.insert(
            name.into(),
            BlockRecord {
                name: name.into(),
                entities: block,
            },
        );
    }
    CadDatabase {
        entities,
        tables,
        read_diagnostics: Default::default(),
    }
}
