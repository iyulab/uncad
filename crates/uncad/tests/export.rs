//! The package (docs/VLM_EXPORT_DESIGN.md, sections 2, 3 and 5 / P7) on
//! `example_2000.dwg`: every file the design lists, images sized to the
//! Claude profile, sidecars whose affines round-trip, records that point
//! at tiles that exist, and byte-identical output on a second run.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use uncad::export::{export_package, ExportError, ExportOptions, Profile};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const HIDDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hidden_layers_r2000.dxf"
);

/// A fresh directory under the target dir, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
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

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn records(dir: &Path, name: &str) -> Vec<Value> {
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

#[test]
fn the_package_has_every_file_and_a_profile_sized_overview() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("files");
    let report = export_package(&db, &tmp.0, &ExportOptions::default()).expect("exports");

    for name in [
        "README.txt",
        "manifest.json",
        "drawing.json",
        "overview.png",
        "tiles.json",
        "blocks.json",
        "strings.json",
        "report.json",
    ] {
        assert!(tmp.0.join(name).exists(), "{name} is written");
    }
    // Record files may be sharded (`name.001.json`, ...).
    for name in ["texts", "dimensions", "geometry", "regions"] {
        assert!(
            tmp.0.join(format!("{name}.json")).exists()
                || tmp.0.join(format!("{name}.001.json")).exists(),
            "{name} is written"
        );
    }
    assert!(!tmp.0.join("drawing.svg").exists());
    assert!(!tmp.0.join("entities.json").exists());

    // The overview fits Claude's budget: edge <= 1568, patches <= 1568.
    let [w, h] = report.overview.px;
    assert!(w <= 1568 && h <= 1568, "{w}x{h}");
    assert!((w / 28) * (h / 28) <= 1568, "{w}x{h}");
    assert_eq!(w % 28, 0);
    assert_eq!(h % 28, 0);
    // Either the edge or the patch budget is the binding limit.
    assert!(
        w.max(h) >= 1568 - 28 || (w / 28) * (h / 28) >= 1568 - 56,
        "the budget is used: {w}x{h}"
    );
    let png = std::fs::read(tmp.0.join("overview.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    let manifest = read_json(&tmp.0.join("manifest.json"));
    assert_eq!(manifest["$schema"], "uncad-package/1");
    assert_eq!(manifest["profile"], "claude");
    assert_eq!(manifest["source"]["name"], Value::Null);
    assert_eq!(manifest["svg_origin"], Value::Null, "no drawing.svg");
    assert_eq!(manifest["units"]["name"], db.header.units.name);
    assert_eq!(manifest["crop"]["source"], "content");
    assert_eq!(manifest["crop"]["excluded"].as_array().unwrap().len(), 2);
    assert!(manifest["guidance"]
        .as_str()
        .unwrap()
        .contains("strings.json"));
    assert!(manifest["files"].as_array().unwrap().len() == report.files.len());
    assert_eq!(manifest["counts"]["dimensions"], 10);
    assert_eq!(manifest["capabilities"]["dimension_values"], "exact");

    // Every listed file exists with the listed size (manifest, README and
    // report are listed without one: they are written last).
    for file in manifest["files"].as_array().unwrap() {
        let path = tmp.0.join(file["path"].as_str().unwrap());
        let meta = std::fs::metadata(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Some(bytes) = file["bytes"].as_u64() {
            assert_eq!(meta.len(), bytes, "{}", path.display());
        } else {
            assert!(
                ["manifest.json", "README.txt", "report.json"]
                    .contains(&file["path"].as_str().unwrap()),
                "{file}"
            );
        }
    }
}

#[test]
fn every_world_box_in_the_package_is_the_documented_array() {
    // docs/VLM_EXPORT_DESIGN.md section 3: "boxes are [x0, y0, x1, y1] in
    // world units". The record bboxes, tiles.json and manifest.sheets[].rect
    // always were; the manifest's overview/frames/crop and sheets.json went
    // through serde's derive on `Rect` and came out as
    // {"min_x": .., "min_y": .., "max_x": .., "max_y": ..}, so the same
    // rectangle had two shapes in one package (a reader indexing
    // manifest.overview.world[2] hit a KeyError).
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("boxes");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    let manifest = read_json(&tmp.0.join("manifest.json"));
    let sheets = read_json(&tmp.0.join("sheets.json"));
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let report_json = read_json(&tmp.0.join("report.json"));

    // Every box, wherever it comes from, is four numbers.
    let box_of = |value: &Value, what: &str| -> [f64; 4] {
        let array = value
            .as_array()
            .unwrap_or_else(|| panic!("{what} is an array, got {value}"));
        assert_eq!(array.len(), 4, "{what}: {value}");
        let mut out = [0.0; 4];
        for (slot, v) in out.iter_mut().zip(array) {
            *slot = v.as_f64().unwrap_or_else(|| panic!("{what}: {value}"));
        }
        out
    };
    let overview = box_of(&manifest["overview"]["world"], "manifest.overview.world");
    assert!(overview[2] > overview[0] && overview[3] > overview[1]);
    // The array holds the same numbers the report's struct does.
    let world = report.overview.world;
    for (got, want) in overview
        .iter()
        .zip([world.min_x, world.min_y, world.max_x, world.max_y])
    {
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    }
    box_of(&manifest["crop"]["rect"], "manifest.crop.rect");
    box_of(&manifest["crop"]["content"], "manifest.crop.content");
    box_of(
        &manifest["crop"]["header_extents"],
        "manifest.crop.header_extents",
    );
    for excluded in manifest["crop"]["excluded"].as_array().unwrap() {
        box_of(&excluded["rect"], "manifest.crop.excluded[].rect");
    }
    for excluded in report_json["excluded"].as_array().unwrap() {
        box_of(&excluded["rect"], "report.excluded[].rect");
    }
    for frame in manifest["frames"].as_array().unwrap() {
        box_of(&frame["content"], "manifest.frames[].content");
        box_of(
            &frame["overview"]["world"],
            "manifest.frames[].overview.world",
        );
    }
    // The same frame content in tiles.json has always been an array: the two
    // now agree to the rounding tiles.json applies.
    let content = box_of(&manifest["frames"][0]["content"], "frames[0].content");
    let rounded = box_of(&tiles["frames"][0]["content"], "tiles.frames[0].content");
    for (a, b) in content.iter().zip(rounded) {
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
    for sheet in sheets["sheets"].as_array().unwrap() {
        box_of(&sheet["rect"], "sheets.json rect");
        box_of(&sheet["overview"]["world"], "sheets.json overview.world");
        for viewport in sheet["viewports"].as_array().unwrap() {
            box_of(&viewport["frame"], "sheets.json viewports[].frame");
        }
    }
    // manifest.sheets[].rect (rounded, always an array) and sheets.json's
    // rect are the same rectangle.
    let from_manifest = box_of(&manifest["sheets"][0]["rect"], "manifest.sheets[0].rect");
    let from_sheets = box_of(&sheets["sheets"][0]["rect"], "sheets.json sheets[0].rect");
    for (a, b) in from_manifest.iter().zip(from_sheets) {
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
}

#[test]
fn tiles_cover_the_levels_and_their_sidecars_round_trip() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("tiles");
    let report = export_package(&db, &tmp.0, &ExportOptions::default()).expect("exports");
    assert_eq!(report.frames.len(), 1, "one connected drawing, one frame");
    let frame = &report.frames[0];
    assert_eq!(frame.id, "f0");
    assert_eq!(
        frame.overview.id, "ov",
        "a single frame reuses the overview"
    );
    assert!(
        !frame.levels.is_empty(),
        "the drawing has text, so it has zoom levels"
    );

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let entries = tiles["tiles"].as_array().unwrap();
    let mut written = 0;
    for level in &frame.levels {
        let step = level.tile_px - level.overlap_px;
        let expect = |extent: u32| -> u32 {
            if extent <= level.tile_px {
                1
            } else {
                (extent - level.tile_px).div_ceil(step) + 1
            }
        };
        assert_eq!(level.cols, expect(level.canvas_px[0]), "z{}", level.z);
        assert_eq!(level.rows, expect(level.canvas_px[1]), "z{}", level.z);
        let on_level: Vec<&Value> = entries.iter().filter(|t| t["z"] == level.z).collect();
        assert_eq!(on_level.len() as u32, level.cols * level.rows);
        written += level.tiles_written;
    }
    assert_eq!(written, report.counts.tiles);
    assert!(written >= 1);

    for entry in entries {
        let Some(png) = entry["png"].as_str() else {
            assert_eq!(entry["empty"], true);
            continue;
        };
        assert!(tmp.0.join(png).exists(), "{png}");
        let sidecar = read_json(&tmp.0.join(entry["sidecar"].as_str().unwrap()));
        assert_eq!(sidecar["id"], entry["id"]);
        let ppu = sidecar["ppu"].as_f64().unwrap();
        // Row-major [a, b, c, d, e, f]: px = a x + b y + c, py = d x + e y + f.
        let w2p = sidecar["px_to_world"].as_array().unwrap();
        let p2w = sidecar["world_to_px"].as_array().unwrap();
        let (a, c, e, f) = (
            p2w[0].as_f64().unwrap(),
            p2w[2].as_f64().unwrap(),
            p2w[4].as_f64().unwrap(),
            p2w[5].as_f64().unwrap(),
        );
        assert!((a - ppu).abs() < 1e-12 && (e + ppu).abs() < 1e-12);
        // px -> world -> px on the tile's far corner.
        let px = sidecar["px"].as_array().unwrap();
        let (pw, ph) = (px[0].as_f64().unwrap(), px[1].as_f64().unwrap());
        let wx = w2p[0].as_f64().unwrap() * pw + w2p[2].as_f64().unwrap();
        let wy = w2p[4].as_f64().unwrap() * ph + w2p[5].as_f64().unwrap();
        let bx = a * wx + c;
        let by = e * wy + f;
        assert!(
            (bx - pw).abs() < 1e-6 && (by - ph).abs() < 1e-6,
            "{bx} {by}"
        );
        assert_eq!(sidecar["overlap_px"], 224);
        assert!(sidecar["records"]["texts"].is_array());
        assert!(
            std::fs::metadata(tmp.0.join(entry["sidecar"].as_str().unwrap()))
                .unwrap()
                .len()
                <= 32 * 1024
        );
    }
}

#[test]
fn records_carry_exact_numbers_and_point_at_existing_tiles() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("records");
    export_package(&db, &tmp.0, &ExportOptions::default()).expect("exports");
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let written: BTreeSet<String> = tiles["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["png"].is_string())
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    let dims = records(&tmp.0, "dimensions");
    assert_eq!(dims.len(), 10);
    let aligned = dims
        .iter()
        .find(|d| d["kind"] == "ALIGNED")
        .expect("an ALIGNED dimension");
    assert_eq!(aligned["display"], "1504,68");
    assert_eq!(aligned["measurement_source"], "act_measurement");
    assert!((aligned["measurement"].as_f64().unwrap() - 1504.6795).abs() < 1e-3);
    assert_eq!(aligned["confidence"], "stored");
    let angular = dims.iter().find(|d| d["kind"] == "ANGULAR_2LINE").unwrap();
    assert_eq!(angular["unit"], "deg");

    let geometry = records(&tmp.0, "geometry");
    let cloud = geometry
        .iter()
        .find(|g| g["id"] == "156")
        .expect("the revision cloud");
    assert_eq!(cloud["type"], "LWPOLYLINE");
    assert_eq!(cloud["vfmt"], "[x,y,bulge]");
    assert_eq!(cloud["closed"], true);
    assert!(cloud["perimeter"].as_f64().unwrap() > 1400.0);
    assert!(cloud["area"].as_f64().unwrap() > 0.0);
    assert_eq!(cloud["confidence"], "exact");
    // The 3256x INSERT is not a record: it is outside the crop.
    let blocks = read_json(&tmp.0.join("blocks.json"));
    let instances = blocks["instances"].as_array().unwrap();
    assert!(instances.iter().all(|i| i["id"] != "756"), "{instances:?}");
    assert!(instances.iter().any(|i| i["block"] == "CIRKLO_PUNKTOJ"));

    let regions = records(&tmp.0, "regions");
    assert!(regions.iter().any(|r| r["id"] == "156"));
    let texts = records(&tmp.0, "texts");
    assert!(!texts.is_empty());
    assert!(texts
        .iter()
        .all(|t| t["bbox_confidence"] == "measured" && t["font_ok"] == true));

    // Every record's tiles exist and were written; every record has an
    // overview pixel box.
    for record in dims.iter().chain(&geometry).chain(&regions).chain(&texts) {
        for tile in record["tiles"].as_array().unwrap() {
            assert!(written.contains(tile.as_str().unwrap()), "{record}");
        }
        assert!(record["px"]["ov"].is_array(), "{record}");
    }

    // strings.json finds the dimension label.
    let strings = read_json(&tmp.0.join("strings.json"));
    let ids = strings["strings"]["1504,68"]
        .as_array()
        .expect("the label is indexed");
    assert!(ids.iter().any(|i| i == &aligned["id"]));

    let report = read_json(&tmp.0.join("report.json"));
    assert_eq!(report["excluded"].as_array().unwrap().len(), 2);
    // One top-level entity on the frozen layer; the dimension blocks hold
    // the definition points on Defpoints.
    assert_eq!(report["hidden"]["count"], 1);
    assert_eq!(report["hidden"]["by_reason"]["layer_frozen"], 1);
    assert!(report["hidden"]["inside_blocks"].as_u64().unwrap() > 1);
}

#[test]
fn the_export_is_deterministic_and_options_are_honoured() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let options = ExportOptions {
        max_levels: 1,
        svg: true,
        full: true,
        shard_kb: 4,
        source_name: Some("example_2000.dwg".into()),
        ..Default::default()
    };
    let a = TempDir::new("det_a");
    let b = TempDir::new("det_b");
    let ra = export_package(&db, &a.0, &options).expect("exports");
    let rb = export_package(&db, &b.0, &options).expect("exports");
    assert_eq!(ra.frames[0].levels.len(), 1);
    assert!(a.0.join("drawing.svg").exists() && a.0.join("entities.json").exists());
    // A 4 KB shard size splits the geometry records.
    assert!(a.0.join("geometry.001.json").exists(), "{:?}", ra.files);
    let manifest = read_json(&a.0.join("manifest.json"));
    assert_eq!(manifest["source"]["name"], "example_2000.dwg");
    assert!(manifest["shard_index"].as_array().unwrap().len() > 5);
    // drawing.svg reads in world units for a drawing near the origin.
    assert_eq!(manifest["svg_origin"], serde_json::json!([0.0, 0.0]));

    for file in &ra.files {
        if file.path == "report.json" {
            continue; // timings
        }
        let fa = std::fs::read(a.0.join(&file.path)).unwrap();
        let fb = std::fs::read(b.0.join(&file.path)).unwrap();
        assert!(fa == fb, "{} differs between runs", file.path);
    }
    assert_eq!(ra.counts, rb.counts);

    // Another profile changes the lattice.
    let c = TempDir::new("openai");
    let rc = export_package(
        &db,
        &c.0,
        &ExportOptions {
            profile: Profile::OPENAI_PATCH,
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(rc.overview.px[0] % 32, 0);
    assert!(rc.frames[0].levels.is_empty());
}

/// `ExportOptions::padding`: the package used to take the automatic 2 %
/// and nothing else, so a caller who wanted the drawing flush to the edge,
/// or room around it, had no way to ask.
#[test]
fn padding_sets_the_window_of_the_overview_and_every_frame() {
    // Two islands: one frame per group, so the option has to reach the
    // frames' own windows and not only the overview's.
    let db = two_islands();
    let run = |name: &str, padding: Option<f64>| {
        let tmp = TempDir::new(name);
        let report = export_package(
            &db,
            &tmp.0,
            &ExportOptions {
                max_levels: 1,
                frame_gap: 0.05,
                sheets: false,
                padding,
                ..Default::default()
            },
        )
        .expect("exports");
        (tmp, report)
    };

    let (_auto, automatic) = run("pad_auto", None);
    let (_tight, tight) = run("pad_0", Some(0.0));
    let (_roomy, roomy) = run("pad_200", Some(200.0));
    assert!(automatic.frames.len() > 1, "two islands, two frames");

    // With no padding the overview is the content, give or take the
    // lattice snap; 200 units a side add 400 to a 1100-unit drawing.
    let content = tight.crop.content.expect("the drawing has content");
    assert!(
        tight.overview.world.width() < content.width() * 1.05,
        "no padding: {:?} vs {:?}",
        tight.overview.world,
        content
    );
    assert!(
        roomy.overview.world.width() > tight.overview.world.width() + 390.0,
        "200 units a side: {:?} vs {:?}",
        roomy.overview.world,
        tight.overview.world
    );
    // The automatic padding sits between the two.
    assert!(
        automatic.overview.world.width() > tight.overview.world.width()
            && automatic.overview.world.width() < roomy.overview.world.width()
    );
    assert_eq!(tight.crop.padding_units, 0.0);
    assert_eq!(roomy.crop.padding_units, 200.0);

    // Each frame's own window, not just the whole crop's.
    for (a, b) in tight.frames.iter().zip(&roomy.frames) {
        assert_eq!(a.content, b.content, "the same group either way");
        assert!(
            b.overview.world.width() > a.overview.world.width() + 390.0,
            "frame {}: {:?} vs {:?}",
            a.id,
            b.overview.world,
            a.overview.world
        );
    }
}

#[test]
fn hidden_entities_stay_out_of_the_records() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let tmp = TempDir::new("hidden");
    let report = export_package(&db, &tmp.0, &ExportOptions::default()).expect("exports");
    assert_eq!(report.counts.hidden, 4);
    let geometry = records(&tmp.0, "geometry");
    assert_eq!(geometry.len(), 5, "{geometry:?}");
    let lines: BTreeSet<&str> = geometry
        .iter()
        .map(|g| g["layer"].as_str().unwrap())
        .collect();
    assert!(!lines.contains("OFF") && !lines.contains("FROZEN") && !lines.contains("Defpoints"));
    let dashed = geometry.iter().find(|g| g["layer"] == "VISIBLE").unwrap();
    assert_eq!(dashed["length"], 100.0);
    assert_eq!(dashed["unit"], "mm");
    assert!(records(&tmp.0, "texts").is_empty());
    let report_json = read_json(&tmp.0.join("report.json"));
    assert_eq!(report_json["hidden"]["by_reason"]["layer_off"], 1);
    assert_eq!(report_json["hidden"]["by_reason"]["invisible"], 1);
}

/// Two 100 x 100 squares of 25 lines each, 1000 units apart, the second
/// with a label: two frames.
fn two_islands() -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let mut entities = Vec::new();
    let mut handle = 0x100u32;
    for dx in [0.0, 1000.0] {
        for i in 0..25 {
            let y = f64::from(i) * 4.0;
            entities.push(uncad::Entity::Line(LineEntity {
                common: EntityCommon {
                    handle: format!("{handle:X}"),
                    layer: "0".into(),
                    ..EntityCommon::default()
                },
                start_point: Point3D { x: dx, y, z: 0.0 },
                end_point: Point3D {
                    x: dx + 100.0,
                    y,
                    z: 0.0,
                },
            }));
            handle += 1;
        }
    }
    entities.push(uncad::Entity::Text(TextEntity {
        common: EntityCommon {
            handle: "TXT".into(),
            layer: "0".into(),
            ..EntityCommon::default()
        },
        start_point: Point2D {
            x: 1010.0,
            y: 110.0,
        },
        text_height: 5.0,
        text: "DETAIL A".into(),
        text_plain: "DETAIL A".into(),
        rotation: 0.0,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    }));
    // Model-space membership is what the renderer selects by.
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

#[test]
fn a_detached_group_becomes_its_own_frame() {
    let db = two_islands();
    let tmp = TempDir::new("frames");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(report.frames.len(), 2, "{:?}", report.frames);
    let (f0, f1) = (&report.frames[0], &report.frames[1]);
    assert_eq!((f0.id.as_str(), f0.kind.as_str()), ("f0", "primary"));
    assert_eq!((f1.id.as_str(), f1.kind.as_str()), ("f1", "detached"));
    // The primary frame is the group with the most entities: the labelled
    // island (25 lines and the text).
    assert_eq!((f0.entities, f1.entities), (26, 25));
    assert_eq!((f0.texts, f1.texts), (1, 0));
    // Each frame has its own overview file and tiles under its directory.
    assert_eq!(f0.overview.png, "frames/f0/overview.png");
    assert!(tmp.0.join("frames/f1/overview.png").exists());
    assert!(tmp.0.join("overview.png").exists());
    assert!(f0.content.min_x >= 999.0 && f1.content.max_x <= 101.0);
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let ids: Vec<&str> = tiles["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.iter().any(|i| i.starts_with("f0/z1/")) && ids.iter().any(|i| i.starts_with("f1/z1/")),
        "{ids:?}"
    );
    // The label's record shows where it is on the whole overview, on its
    // frame's overview and on its tiles -- not on the other frame's images.
    let texts = records(&tmp.0, "texts");
    let label = texts.iter().find(|t| t["id"] == "TXT").expect("the label");
    let px = label["px"].as_object().unwrap();
    assert!(px.contains_key("ov") && px.contains_key("f0/ov"), "{px:?}");
    assert!(!px.contains_key("f1/ov"));
    assert!(label["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t.as_str().unwrap().starts_with("f0/")));
    let manifest = read_json(&tmp.0.join("manifest.json"));
    assert_eq!(manifest["frames"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["capabilities"]["frames"], 2);
    // A gap of 100 % of the diagonal merges everything into one frame.
    let merged = export_package(
        &db,
        &TempDir::new("frames_merged").0,
        &ExportOptions {
            max_levels: 0,
            frame_gap: 1.0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(merged.frames.len(), 1);
    assert_eq!(merged.frames[0].overview.id, "ov");
}

/// One line and one TEXT saying `text`, in model space.
fn drawing_with_text(text: &str) -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let entities = vec![
        uncad::Entity::Line(LineEntity {
            common: EntityCommon {
                handle: "L".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        }),
        uncad::Entity::Text(TextEntity {
            common: EntityCommon {
                handle: "T".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point2D { x: 10.0, y: 10.0 },
            text_height: 5.0,
            text: text.into(),
            text_plain: text.into(),
            rotation: 0.0,
            horizontal_alignment: 0,
            vertical_alignment: 0,
            alignment_point: None,
            width_factor: 1.0,
            oblique_angle: 0.0,
            style: String::new(),
        }),
    ];
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

#[test]
fn text_boxes_are_measured_with_the_bundled_font_and_gaps_are_reported() {
    // Hangul, CP949-decoded, shaped by the bundled Noto Sans KR subset.
    let db = uncad::parse(HIDDEN.replace("hidden_layers", "cp949")).expect("fixture must parse");
    let tmp = TempDir::new("hangul");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let texts = records(&tmp.0, "texts");
    assert_eq!(texts.len(), 5);
    assert!(texts
        .iter()
        .all(|t| t["bbox_confidence"] == "measured" && t["font_ok"] == true));
    // "도면" at height 2.5 is drawn at font-size 2.5 / 0.733 = 3.41 (the
    // CAD height is the cap height): two syllables of roughly 0.85 em
    // each, so about 5.8 units wide, and a Hangul glyph is about 0.88 em
    // tall, so about 3 units -- taller than the 2.5 of a capital.
    let domyeon = texts.iter().find(|t| t["id"] == "23").expect("TEXT 23");
    assert_eq!(domyeon["text"], "\u{b3c4}\u{ba74}");
    let b = domyeon["bbox"].as_array().unwrap();
    let (w, h) = (
        b[2].as_f64().unwrap() - b[0].as_f64().unwrap(),
        b[3].as_f64().unwrap() - b[1].as_f64().unwrap(),
    );
    assert!(
        (4.8..7.5).contains(&w) && (2.5..3.8).contains(&h),
        "{w} x {h}"
    );
    let manifest = read_json(&tmp.0.join("manifest.json"));
    assert_eq!(manifest["capabilities"]["text_boxes"], "measured");
    assert_eq!(manifest["capabilities"]["fonts"], "bundled");

    // Hanja is outside the subset: the box is measured (as .notdef boxes),
    // the record says the font failed, and the manifest warns.
    let db = drawing_with_text("\u{6f22}\u{5b57} A");
    let tmp = TempDir::new("hanja");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("UnshapedGlyphs")),
        "{:?}",
        report.warnings
    );
    let texts = records(&tmp.0, "texts");
    let t = texts.iter().find(|t| t["id"] == "T").expect("the text");
    assert_eq!(t["font_ok"], false);
    assert_eq!(t["unshaped_glyphs"], 2);
    assert_eq!(t["bbox_confidence"], "measured");
}

/// Two 1000-unit lines 2 units apart with a label between them: a 500:1
/// drawing.
fn very_wide() -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let mut entities = Vec::new();
    for (handle, y) in [("L0", 0.0), ("L1", 2.0)] {
        entities.push(uncad::Entity::Line(LineEntity {
            common: EntityCommon {
                handle: handle.into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D { x: 0.0, y, z: 0.0 },
            end_point: Point3D {
                x: 1000.0,
                y,
                z: 0.0,
            },
        }));
    }
    entities.push(uncad::Entity::Text(TextEntity {
        common: EntityCommon {
            handle: "T".into(),
            layer: "0".into(),
            ..EntityCommon::default()
        },
        start_point: Point2D { x: 10.0, y: 0.5 },
        text_height: 1.0,
        text: "WIDE".into(),
        text_plain: "WIDE".into(),
        rotation: 0.0,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    }));
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

#[test]
fn a_very_wide_drawing_raises_the_tiny_overview_warning() {
    let tmp = TempDir::new("wide");
    let report = export_package(
        &very_wide(),
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    // fit_overview for a 1000 x 2 content: pw = min(56, floor(sqrt(1568 *
    // 500))) = 56 patches wide, ph = min(floor(1568 / 56), ceil(56 / 500))
    // = 1 patch tall, so the image is one patch (28 px) tall whatever the
    // padding does to the width (the CLI gives 700 x 28 px): far under 200
    // on the short edge while the long edge is not. The old guard tested
    // the long edge and could never fire.
    let [w, h] = report.overview.px;
    assert_eq!(h, 28, "{w}x{h}");
    assert!(w >= 200, "{w}x{h}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("TinyOverview")),
        "{:?}",
        report.warnings
    );
}

/// A capital-only TEXT of height 2 and a three-line capital-only MTEXT of
/// height 2.5 attached at its top-left corner, over a line.
fn capitals() -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, MTextEntity, Point2D, Point3D, TextEntity};
    let entities = vec![
        uncad::Entity::Line(LineEntity {
            common: EntityCommon {
                handle: "L".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        }),
        uncad::Entity::Text(TextEntity {
            common: EntityCommon {
                handle: "T".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point2D { x: 10.0, y: 10.0 },
            text_height: 2.0,
            text: "HIH".into(),
            text_plain: "HIH".into(),
            rotation: 0.0,
            horizontal_alignment: 0,
            vertical_alignment: 0,
            alignment_point: None,
            width_factor: 1.0,
            oblique_angle: 0.0,
            style: String::new(),
        }),
        uncad::Entity::MText(MTextEntity {
            common: EntityCommon {
                handle: "M".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            insertion_point: Point3D {
                x: 10.0,
                y: 40.0,
                z: 0.0,
            },
            text: r"H\PH\PH".into(),
            text_plain: "H\nH\nH".into(),
            text_height: 2.5,
            rotation: 0.0,
            line_spacing_factor: 1.0,
            attachment: 1,
            rect_width: 0.0,
            extents_width: 0.0,
            extents_height: 0.0,
            x_axis_dir: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            style: String::new(),
        }),
    ];
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

#[test]
fn capitals_are_drawn_as_tall_as_the_cad_text_height() {
    let tmp = TempDir::new("capitals");
    export_package(
        &capitals(),
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    let texts = records(&tmp.0, "texts");
    let bbox = |id: &str| -> [f64; 4] {
        let t = texts.iter().find(|t| t["id"] == id).expect(id);
        assert_eq!(t["bbox_confidence"], "measured");
        let b = t["bbox"].as_array().unwrap();
        [
            b[0].as_f64().unwrap(),
            b[1].as_f64().unwrap(),
            b[2].as_f64().unwrap(),
            b[3].as_f64().unwrap(),
        ]
    };
    // The CAD height is the height of the capitals: "HIH" at height 2
    // measures 2 units from baseline to cap top (glyph outlines, so exact
    // to the font's rounding), sitting on its baseline at y = 10.
    let t = bbox("T");
    assert!((t[3] - t[1] - 2.0).abs() < 0.05, "{t:?}");
    assert!((t[1] - 10.0).abs() < 0.05, "{t:?}");
    // The record keeps the drawing's height.
    let record = texts.iter().find(|t| t["id"] == "T").unwrap();
    assert_eq!(record["height"], 2.0);
    // Three lines of height 2.5 at AutoCAD's 5/3 spacing: the first cap
    // top at the insertion point (top attachment), baselines 4.1667 apart,
    // so the block runs from y = 40 down to the last baseline at
    // 40 - 2.5 - 2 x 4.1667 = 29.1667, i.e. 10.833 tall -- the same
    // 2.5 x (1 + 2 x 5/3) that estimate_mtext_box gives.
    let m = bbox("M");
    assert!((m[3] - 40.0).abs() < 0.05, "{m:?}");
    assert!(
        (m[3] - m[1] - 2.5 * (1.0 + 2.0 * 5.0 / 3.0)).abs() < 0.05,
        "{m:?}"
    );
}

/// An 11 x 11 grid of 1000-unit lines with a 3-unit label in every cell
/// (small enough to need two zoom levels), shifted by `offset` on both
/// axes.
fn labelled_grid(offset: f64) -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let mut entities = Vec::new();
    let mut handle = 0x100u32;
    for i in 0..=10 {
        let at = offset + f64::from(i) * 100.0;
        for (start, end) in [
            (
                Point3D {
                    x: offset,
                    y: at,
                    z: 0.0,
                },
                Point3D {
                    x: offset + 1000.0,
                    y: at,
                    z: 0.0,
                },
            ),
            (
                Point3D {
                    x: at,
                    y: offset,
                    z: 0.0,
                },
                Point3D {
                    x: at,
                    y: offset + 1000.0,
                    z: 0.0,
                },
            ),
        ] {
            entities.push(uncad::Entity::Line(LineEntity {
                common: EntityCommon {
                    handle: format!("{handle:X}"),
                    layer: "0".into(),
                    ..EntityCommon::default()
                },
                start_point: start,
                end_point: end,
            }));
            handle += 1;
        }
    }
    for row in 0..10 {
        for col in 0..10 {
            let text = format!("R{row}{col}");
            entities.push(uncad::Entity::Text(TextEntity {
                common: EntityCommon {
                    handle: format!("{handle:X}"),
                    layer: "0".into(),
                    ..EntityCommon::default()
                },
                start_point: Point2D {
                    x: offset + f64::from(col) * 100.0 + 20.0,
                    y: offset + f64::from(row) * 100.0 + 40.0,
                },
                text_height: 3.0,
                text: text.clone(),
                text_plain: text,
                rotation: 0.0,
                horizontal_alignment: 0,
                vertical_alignment: 0,
                alignment_point: None,
                width_factor: 1.0,
                oblique_angle: 0.0,
                style: String::new(),
            }));
            handle += 1;
        }
    }
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

/// Dark (< 128) pixels of an 8-bit RGB PNG.
fn dark_pixels(png: &[u8]) -> usize {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("a frame size")];
    let info = reader.next_frame(&mut buf).unwrap();
    buf[..info.buffer_size()]
        .chunks_exact(3)
        .filter(|p| p[0] < 128)
        .count()
}

#[test]
fn a_drawing_far_from_the_origin_rasterizes_like_one_at_the_origin() {
    // usvg and tiny-skia keep path points in f32. At 1e7 the f32 step is
    // 1 unit, at 2.5e8 it is 16: a 1.25 px stroke at ~1.5 px/unit
    // collapses and every LINE vanished, the text became wedges. The
    // renderer now writes its coordinates relative to the drawing's own
    // (median, whole-unit) origin whenever that exceeds 32768, so the same
    // grid gives the same picture wherever it sits: the dark pixel counts
    // of the plain PNG and of each tile level agree within a few percent
    // (the shift is a whole number of units, so the only difference is
    // sub-pixel anti-aliasing of the same geometry).
    let mut plain: Vec<usize> = Vec::new();
    let mut levels: Vec<Vec<usize>> = Vec::new();
    let mut origins: Vec<[f64; 2]> = Vec::new();
    for (name, offset) in [("0", 0.0), ("1e7", 1.0e7), ("2.5e8", 2.5e8)] {
        let db = labelled_grid(offset);
        let png = db.to_png(uncad::ToPngOptions::default()).expect("renders");
        origins.push(png.origin);
        plain.push(dark_pixels(&png.png));

        let tmp = TempDir::new(&format!("far_{name}"));
        // A quarter-size overview (784 px) keeps the z2 canvas at ~3100 px
        // and the tile count small; the tiles themselves are full size.
        let report = export_package(
            &db,
            &tmp.0,
            &ExportOptions {
                max_levels: 2,
                profile: Profile {
                    name: "claude-small",
                    overview_edge: 784,
                    overview_patches: 784,
                    ..Profile::CLAUDE
                },
                ..Default::default()
            },
        )
        .expect("exports");
        assert_eq!(report.frames[0].levels.len(), 2, "{name}: two levels");
        let tiles = read_json(&tmp.0.join("tiles.json"));
        let mut per_level = vec![0usize; 3];
        for tile in tiles["tiles"].as_array().unwrap() {
            let Some(path) = tile["png"].as_str() else {
                continue;
            };
            let z = tile["z"].as_u64().unwrap() as usize;
            per_level[z] += dark_pixels(&std::fs::read(tmp.0.join(path)).unwrap());
        }
        levels.push(per_level);
        assert!(report.warnings.is_empty(), "{name}: {:?}", report.warnings);
    }
    // The origin is chosen only when needed: [0, 0] at the origin, the
    // rounded median of the entities' reference points otherwise (the
    // grid's median line start / text anchor lies inside the grid).
    assert_eq!(origins[0], [0.0, 0.0]);
    for (o, offset) in origins[1..].iter().zip([1.0e7, 2.5e8]) {
        assert!(
            o[0] >= offset && o[0] <= offset + 1000.0 && o[1] >= offset && o[1] <= offset + 1000.0,
            "{o:?} is inside the grid at {offset}"
        );
        assert_eq!(o[0].fract(), 0.0, "whole units");
    }
    let within = |a: usize, b: usize| (a as f64 - b as f64).abs() / a as f64 <= 0.03;
    assert!(plain[0] > 20_000, "the grid and its labels: {plain:?}");
    for i in 1..3 {
        assert!(within(plain[0], plain[i]), "plain PNG: {plain:?}");
        for z in 1..=2 {
            assert!(
                within(levels[0][z], levels[i][z]),
                "z{z}: {:?} vs {:?}",
                levels[0],
                levels[i]
            );
        }
    }
}

/// `count` small closed squares on a grid: `count` geometry records and
/// `count` region records, enough of them on one tile to push its sidecar
/// past the 32 KB budget.
fn many_regions(count: usize) -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LwPolylineEntity, Point2D, Point3D};
    let side = (count as f64).sqrt().ceil() as usize;
    let mut entities = Vec::new();
    for i in 0..count {
        let (x, y) = ((i % side) as f64 * 10.0, (i / side) as f64 * 10.0);
        entities.push(uncad::Entity::LwPolyline(LwPolylineEntity {
            common: EntityCommon {
                handle: format!("{:X}", 0x1000 + i),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            vertices: vec![
                Point2D { x, y },
                Point2D { x: x + 6.0, y },
                Point2D {
                    x: x + 6.0,
                    y: y + 6.0,
                },
                Point2D { x, y: y + 6.0 },
            ],
            closed: true,
            bulges: Vec::new(),
            widths: Vec::new(),
            const_width: 0.0,
            elevation: 0.0,
            extrusion: Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        }));
    }
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

/// A drawing 0.004 x 0.003 units across -- a jewellery detail, a PCB pad,
/// anything drawn in inches and dimensioned in thousandths.
fn sub_millimetre_drawing() -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let (w, h) = (0.004, 0.003);
    let mut entities = Vec::new();
    let mut handle = 0x100u32;
    let line = |x1: f64, y1: f64, x2: f64, y2: f64, handle: &mut u32| {
        let e = uncad::Entity::Line(LineEntity {
            common: EntityCommon {
                handle: format!("{handle:X}"),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: x1,
                y: y1,
                z: 0.0,
            },
            end_point: Point3D {
                x: x2,
                y: y2,
                z: 0.0,
            },
        });
        *handle += 1;
        e
    };
    entities.push(line(0.0, 0.0, w, 0.0, &mut handle));
    entities.push(line(w, 0.0, w, h, &mut handle));
    entities.push(line(w, h, 0.0, h, &mut handle));
    entities.push(line(0.0, h, 0.0, 0.0, &mut handle));
    for i in 1..12 {
        let y = h * f64::from(i) / 12.0;
        entities.push(line(0.0, y, w, y, &mut handle));
    }
    entities.push(uncad::Entity::Text(TextEntity {
        common: EntityCommon {
            handle: "TXT".into(),
            layer: "0".into(),
            ..EntityCommon::default()
        },
        start_point: Point2D {
            x: w * 0.1,
            y: h * 0.5,
        },
        text_height: 1e-5,
        text: "0.004".into(),
        text_plain: "0.004".into(),
        rotation: 0.0,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    }));
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

/// The world rectangles used to be rounded to `$LUPREC` decimals alone
/// (3 at the least). On a drawing four thousandths of a unit wide that is
/// coarser than the whole drawing: every tile rectangle came out as the
/// same `[0.0, 0.0, 0.004, 0.003]`, so a consumer could not tell two tiles
/// apart and the `world` printed beside `world_to_px` described a
/// different rectangle from the one the affine maps.
#[test]
fn a_sub_millimetre_drawing_keeps_its_tile_rectangles_apart() {
    let db = sub_millimetre_drawing();
    let tmp = TempDir::new("tiny");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            sheets: false,
            ..Default::default()
        },
    )
    .expect("exports");
    assert!(
        report.overview.world.width() < 0.01,
        "the drawing is a few thousandths of a unit across: {:?}",
        report.overview.world
    );
    // One level is a canvas twice the overview's edge, which is several
    // tiles wide: enough rectangles to tell apart.
    let deepest = report.frames[0].z_max;
    assert_eq!(deepest, 1);

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let entries = tiles["tiles"].as_array().unwrap();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0;
    for entry in entries {
        let Some(sidecar_path) = entry["sidecar"].as_str() else {
            continue;
        };
        if entry["z"].as_u64() != Some(u64::from(deepest)) {
            continue;
        }
        // No two tiles of a level may print the same rectangle.
        assert!(
            seen.insert(entry["world"].to_string()),
            "two tiles share the rectangle {}: {}",
            entry["world"],
            entry["id"]
        );
        // The rectangle and the affine beside it have to agree: the
        // rounded corners must land on the image's own corners, to well
        // under a pixel.
        let sidecar = read_json(&tmp.0.join(sidecar_path));
        assert_eq!(sidecar["world"], entry["world"], "{}", entry["id"]);
        let world: Vec<f64> = sidecar["world"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let w2p: Vec<f64> = sidecar["world_to_px"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let px = sidecar["px"].as_array().unwrap();
        let (pw, ph) = (px[0].as_f64().unwrap(), px[1].as_f64().unwrap());
        // px = a x + c, py = e y + f (b and d are zero: no rotation).
        let corners = [
            (w2p[0] * world[0] + w2p[2], 0.0),
            (w2p[0] * world[2] + w2p[2], pw),
            (w2p[4] * world[3] + w2p[5], 0.0),
            (w2p[4] * world[1] + w2p[5], ph),
        ];
        for (got, want) in corners {
            assert!(
                (got - want).abs() < 0.01,
                "{}: the rounded world maps to {got} px, not {want} (world {world:?}, affine {w2p:?})",
                entry["id"]
            );
        }
        checked += 1;
    }
    assert!(
        checked > 1,
        "the deepest level should hold more than one tile to compare"
    );
}

#[test]
fn a_dense_drawing_keeps_every_sidecar_under_the_32_kb_cap() {
    // The trim loop measures the compact serialization against 32 KB, but
    // the file used to be written with `to_string_pretty`, which puts every
    // number of every [id, [x0,y0,x1,y1], value] row on its own line: the
    // file on disk was about 3.3x the measured size, so rows were dropped
    // (`records_truncated: true`) to satisfy a limit the file then broke
    // threefold anyway.
    let db = many_regions(6000);
    let tmp = TempDir::new("dense");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(report.counts.regions, 6000);

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let mut checked = 0;
    let mut truncated = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let file = tmp.0.join(path);
        let bytes = std::fs::metadata(&file).unwrap().len();
        assert!(bytes <= 32 * 1024, "{path} is {bytes} bytes");
        let sidecar = read_json(&file);
        // The rows that survived are still valid JSON, and the file says so
        // when it had to cut any.
        assert!(sidecar["records"]["regions"].is_array());
        if sidecar["records_truncated"] == true {
            truncated += 1;
        }
        checked += 1;
    }
    assert!(checked >= 2, "{checked} sidecars");
    assert!(
        truncated >= 1,
        "6000 regions on one level must overflow at least one sidecar"
    );
}

#[test]
fn a_sidecar_lists_the_layers_of_everything_on_its_tile() {
    // `layers_present` chained only texts, dimensions and block instances,
    // so a tile drawn from geometry alone reported none -- and geometry is
    // the bulk of every tile. The design calls sidecars authoritative for
    // "what is on this image", so the field has to cover the ink.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("layers");
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");

    // What each tile shows, derived from the records' own `tiles` lists
    // (the same membership test the sidecar uses, computed independently).
    let mut expected: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    let mut geometry_layers: BTreeSet<String> = BTreeSet::new();
    for kind in ["texts", "dimensions", "geometry", "regions", "blocks"] {
        let rows = if kind == "blocks" {
            read_json(&tmp.0.join("blocks.json"))["instances"]
                .as_array()
                .unwrap()
                .clone()
        } else {
            records(&tmp.0, kind)
        };
        for record in rows {
            let layer = record["layer"].as_str().unwrap().to_string();
            if kind == "geometry" {
                geometry_layers.insert(layer.clone());
            }
            for tile in record["tiles"].as_array().unwrap() {
                expected
                    .entry(tile.as_str().unwrap().to_string())
                    .or_default()
                    .insert(layer.clone());
            }
        }
    }
    assert!(geometry_layers.len() > 1, "{geometry_layers:?}");

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let id = entry["id"].as_str().unwrap();
        let sidecar = read_json(&tmp.0.join(path));
        let listed: BTreeSet<String> = sidecar["layers_present"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            listed,
            *expected.get(id).unwrap_or(&BTreeSet::new()),
            "{id}: {listed:?}"
        );
        seen.extend(listed);
        checked += 1;
    }
    assert!(checked >= 2, "{checked} sidecars");
    // Every layer that carries geometry reaches some sidecar; before the
    // fix the geometry-only layers reached none.
    for layer in &geometry_layers {
        assert!(seen.contains(layer), "{layer} is on no sidecar: {seen:?}");
    }
}

#[test]
fn the_guidance_quotes_the_profile_in_use() {
    // manifest.guidance told the reader "224 px overlap" whatever the
    // profile was, while frames[].levels[].overlap_px said 392 for
    // claude-hires: the prose an agent is told to read first contradicted
    // the structured data it is meant to trust.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    for profile in [
        Profile::CLAUDE,
        Profile::CLAUDE_HIRES,
        Profile::OPENAI_PATCH,
    ] {
        let tmp = TempDir::new(&format!("guidance_{}", profile.name));
        // One level for the default profile (so the prose can be held
        // against real `levels[]` numbers), none for the others: a
        // 1932 px tile pyramid costs minutes and says nothing more here.
        let report = export_package(
            &db,
            &tmp.0,
            &ExportOptions {
                profile,
                max_levels: u32::from(profile == Profile::CLAUDE),
                ..Default::default()
            },
        )
        .expect("exports");
        let manifest = read_json(&tmp.0.join("manifest.json"));
        let guidance = manifest["guidance"].as_str().unwrap();
        let sentence = format!("{} px with {} px overlap", profile.tile, profile.overlap);
        assert!(guidance.contains(&sentence), "{}: {guidance}", profile.name);
        // And it says what the levels say.
        for level in &report.frames[0].levels {
            assert_eq!(level.overlap_px, profile.overlap);
            assert_eq!(level.tile_px, profile.tile);
        }
        // No other profile's numbers are quoted.
        for other in [
            Profile::CLAUDE,
            Profile::CLAUDE_HIRES,
            Profile::OPENAI_PATCH,
        ] {
            if other.overlap != profile.overlap {
                assert!(
                    !guidance.contains(&format!("{} px overlap", other.overlap)),
                    "{}: {guidance}",
                    profile.name
                );
            }
        }
    }
}

#[test]
fn re_exporting_clears_the_previous_package_but_nothing_else() {
    // A second export with other options used to leave the first run's
    // files beside the new ones: deeper tile levels with sidecars whose
    // `parent`/`children` describe a pyramid the new manifest does not
    // have, and record shards (`texts.003.json`) holding ids the new
    // `texts.json` also holds. Both look valid, so a consumer walking the
    // tree -- which README.txt invites -- mixes two exports.
    let db = labelled_grid(0.0);
    let small = Profile {
        name: "claude-small",
        overview_edge: 784,
        overview_patches: 784,
        ..Profile::CLAUDE
    };
    let tmp = TempDir::new("reexport");
    let first = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 2,
            shard_kb: 4,
            profile: small,
            ..Default::default()
        },
    )
    .expect("exports");
    assert!(first.frames[0].levels.len() == 2, "{:?}", first.frames);
    assert!(tmp.0.join("frames/f0/tiles/z2").exists());
    assert!(tmp.0.join("texts.001.json").exists(), "sharded");

    // Something the export did not write: not ours to remove.
    let sentinel = tmp.0.join("notes.txt");
    std::fs::write(&sentinel, b"mine").unwrap();
    let kept_tile = tmp.0.join("frames/f0/tiles/z2/keep.txt");
    std::fs::write(&kept_tile, b"mine too").unwrap();

    let second = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            profile: small,
            ..Default::default()
        },
    )
    .expect("exports again");
    assert_eq!(second.frames[0].levels.len(), 1);

    // Everything on disk is either the new package or the files placed by
    // hand: no z2 tile, no stale shard.
    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![tmp.0.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                on_disk.insert(
                    path.strip_prefix(&tmp.0)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let listed: BTreeSet<String> = second.files.iter().map(|f| f.path.clone()).collect();
    let extra: Vec<&String> = on_disk.difference(&listed).collect();
    assert_eq!(
        extra,
        [
            &"frames/f0/tiles/z2/keep.txt".to_string(),
            &"notes.txt".to_string()
        ],
        "stale files: {extra:?}"
    );
    assert!(listed.iter().all(|p| on_disk.contains(p)), "{listed:?}");
    assert!(
        !on_disk.iter().any(|p| p.starts_with("texts.0")),
        "{on_disk:?}"
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"mine");

    // A directory that only held the old package's files is gone; one that
    // still holds something stays.
    assert!(tmp.0.join("frames/f0/tiles/z1").exists());
    assert!(
        tmp.0.join("frames/f0/tiles/z2").exists(),
        "keep.txt is in it"
    );
    std::fs::remove_file(&kept_tile).unwrap();
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            profile: small,
            ..Default::default()
        },
    )
    .expect("exports a third time");
    assert!(!tmp.0.join("frames/f0/tiles/z2").exists(), "now empty");

    // A directory that is not an uncad package is never touched.
    let foreign = TempDir::new("foreign");
    std::fs::create_dir_all(&foreign.0).unwrap();
    std::fs::write(foreign.0.join("manifest.json"), br#"{"schema":"other"}"#).unwrap();
    std::fs::write(foreign.0.join("important.bin"), b"keep").unwrap();
    export_package(
        &db,
        &foreign.0,
        &ExportOptions {
            max_levels: 0,
            profile: small,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(
        std::fs::read(foreign.0.join("important.bin")).unwrap(),
        b"keep"
    );
}

#[test]
fn a_drawing_that_is_one_point_gets_a_window_it_can_be_seen_in() {
    // The overview solves the padding and the scale against each other: a
    // seed scale from the content size, `auto_padding` from that, then the
    // scale that fits. For content with no size at all -- the corpus's
    // Point.dwg, RAY.dwg and ConstructionLine.dwg, whose entities record a
    // single base point -- the seed was ~1e12 px/unit, so "24 px of
    // padding" came to 2e-11 units and the drawing was rendered into a
    // 5e-11-unit window at 2.4e13 px/unit: every image blank, and every
    // `world` box in tiles.json and the sidecars collapsing to zero size
    // once it was rounded, so the affines no longer agreed with it.
    for name in ["Point", "RAY", "ConstructionLine"] {
        let path = format!(
            "{}/../../lib/libredwg/test/test-data/2000/{name}.dwg",
            env!("CARGO_MANIFEST_DIR")
        );
        let db = uncad::parse(&path).expect("corpus file must parse");
        let tmp = TempDir::new(&format!("degenerate_{name}"));
        let report = export_package(
            &db,
            &tmp.0,
            &ExportOptions {
                max_levels: 1,
                ..Default::default()
            },
        )
        .expect("exports");

        // The window is the ten-unit minimum around the content, plus the
        // 2 % padding: about 10.5 units, so a 1092 px overview is ~104
        // px/unit rather than the 2.4e13 the runaway padding produced.
        let world = report.overview.world;
        assert!(
            (10.0..=12.0).contains(&world.width()) && (10.0..=12.0).contains(&world.height()),
            "{name}: {world:?}"
        );
        assert!(
            report.overview.ppu.is_finite() && (50.0..=200.0).contains(&report.overview.ppu),
            "{name}: {} px/unit",
            report.overview.ppu
        );
        // The picture shows the entity: a POINT is a filled dot, a RAY and
        // an XLINE cross the window.
        let png = std::fs::read(tmp.0.join("overview.png")).unwrap();
        assert!(dark_pixels(&png) > 100, "{name}: a blank overview");

        // The rounded world box is a real rectangle, and the sidecar's
        // affine still maps that box's corner to the image's corner.
        let tiles = read_json(&tmp.0.join("tiles.json"));
        let mut checked = 0;
        for entry in tiles["tiles"].as_array().unwrap() {
            let Some(sidecar_path) = entry["sidecar"].as_str() else {
                continue;
            };
            let sidecar = read_json(&tmp.0.join(sidecar_path));
            let w: Vec<f64> = sidecar["world"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            assert!(w[2] > w[0] && w[3] > w[1], "{name}: {w:?}");
            let a = sidecar["world_to_px"].as_array().unwrap();
            let (sx, cx) = (a[0].as_f64().unwrap(), a[2].as_f64().unwrap());
            let (sy, cy) = (a[4].as_f64().unwrap(), a[5].as_f64().unwrap());
            // The upper-left corner of the world box is pixel (0, 0), up to
            // the rounding of the box itself (luprec decimals at this
            // scale: well under a pixel).
            let (px, py) = (sx * w[0] + cx, sy * w[3] + cy);
            assert!(px.abs() < 1.0 && py.abs() < 1.0, "{name}: ({px}, {py})");
            checked += 1;
        }
        assert!(checked >= 1, "{name}: no tile was written");
    }
}

/// A 3000 x 1000 frame with a centre line and one long Hangul note at
/// (40, 510), height 20: 100 syllables, each advancing about 0.92 of the
/// text height where the renderer's estimate allows 0.8186 (`CHAR_ADVANCE`
/// = 0.6 em over the bundled font's 0.733 cap height), so the string runs
/// some 200 units past the box the crop and the tile cull use.
fn long_hangul_note() -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let mut entities = Vec::new();
    let corners = [
        (0.0, 0.0, 3000.0, 0.0),
        (3000.0, 0.0, 3000.0, 1000.0),
        (3000.0, 1000.0, 0.0, 1000.0),
        (0.0, 1000.0, 0.0, 0.0),
        (0.0, 500.0, 3000.0, 500.0),
    ];
    for (n, (x0, y0, x1, y1)) in corners.into_iter().enumerate() {
        entities.push(uncad::Entity::Line(LineEntity {
            common: EntityCommon {
                handle: format!("L{n}"),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: x0,
                y: y0,
                z: 0.0,
            },
            end_point: Point3D {
                x: x1,
                y: y1,
                z: 0.0,
            },
        }));
    }
    let note = "\u{ac00}\u{b098}\u{b2e4}\u{b77c}".repeat(25);
    entities.push(uncad::Entity::Text(TextEntity {
        common: EntityCommon {
            handle: "T".into(),
            layer: "0".into(),
            ..EntityCommon::default()
        },
        start_point: Point2D { x: 40.0, y: 510.0 },
        text_height: 20.0,
        text: note.clone(),
        text_plain: note,
        rotation: 0.0,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    }));
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

/// Dark (< 128) pixels of an 8-bit RGB PNG inside `[x0, y0, x1, y1]`.
fn dark_pixels_in(png: &[u8], area: [i64; 4]) -> usize {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("a frame size")];
    let info = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as i64, info.height as i64);
    let mut count = 0;
    for y in area[1].max(0)..area[3].min(h) {
        for x in area[0].max(0)..area[2].min(w) {
            if buf[((y * w + x) * 3) as usize] < 128 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn a_tile_keeps_the_half_of_a_long_hangul_text_that_reaches_it() {
    // Tile culling kept only the parts whose *extent* touched the tile, and
    // the extents carry the renderer's 0.6-em-per-character estimate, while
    // `texts.json` lists a text's tiles from its measured glyph box. A
    // 40-syllable Korean note is ~50 % wider than the estimate, so the
    // tile the record pointed at was drawn without it: the record said the
    // text is there, the picture showed only the line.
    let db = long_hangul_note();
    let tmp = TempDir::new("hangul_tile");
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            frame_gap: 1.0,
            ..Default::default()
        },
    )
    .expect("exports");

    let texts = records(&tmp.0, "texts");
    let note = texts.iter().find(|t| t["id"] == "T").expect("the note");
    assert_eq!(note["bbox_confidence"], "measured");
    let b: Vec<f64> = note["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    // The renderer's estimate ends at 40 + 100 x (0.6 / 0.733) x 20 =
    // 1677.2 (uncad::text::CHAR_ADVANCE heights per character); the shaped
    // Hangul runs past it.
    let estimate_end = 40.0 + 100.0 * (0.6 / 0.733) * 20.0;
    assert!(
        b[2] > estimate_end + 100.0,
        "the measured box should be wider than {estimate_end}: {b:?}"
    );

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let listed: BTreeSet<&str> = note["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    // A tile that starts past where the estimate ended but still inside the
    // measured box: that is the one the cull used to empty.
    let mut checked = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let world: Vec<f64> = entry["world"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let beyond_the_estimate = world[0] > estimate_end + 20.0;
        let inside_the_text = world[0] < b[2] && world[1] < b[3] && world[3] > b[1];
        if !(beyond_the_estimate && inside_the_text) {
            continue;
        }
        let id = entry["id"].as_str().unwrap();
        assert!(listed.contains(id), "texts.json lists {id}: {listed:?}");
        let png_path = entry["png"]
            .as_str()
            .unwrap_or_else(|| panic!("{id} empty"));
        let png = std::fs::read(tmp.0.join(png_path)).unwrap();
        // The text's band on this tile, from the record's own pixel box.
        let area: Vec<i64> = note["px"][id]
            .as_array()
            .unwrap_or_else(|| panic!("{id} is not in the record's px map: {note}"))
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let ink = dark_pixels_in(&png, [area[0], area[1], area[2], area[3]]);
        assert!(ink > 200, "{id}: {ink} dark pixels in the text band");
        checked += 1;
    }
    assert!(
        checked >= 1,
        "no tile starts past the estimate's end: the case is not exercised"
    );
}

const NESTED_ATTRIB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/nested_attrib_r2000.dxf"
);

#[test]
fn a_nested_block_references_attribute_is_drawn_and_indexed() {
    // The tag-inside-assembly pattern: block DOOR holds an INSERT of block
    // TAG whose ATTRIB says NUM = D-101. `collect_texts` skipped every
    // ATTRIB child of a block ("the top-level walk already sees them",
    // which holds only for a top-level INSERT's own attribs) and never
    // looked at a nested INSERT's `attribs` at all, so the value was in no
    // record -- and in the DWG shape it was not even drawn. An agent asked
    // "which door is D-101" could not find it.
    let db = uncad::parse(NESTED_ATTRIB).expect("fixture must parse");
    let tmp = TempDir::new("nested_attrib");
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            svg: true,
            ..Default::default()
        },
    )
    .expect("exports");

    let texts = records(&tmp.0, "texts");
    // DOOR is inserted at (100, 100) unrotated and unscaled and the ATTRIB
    // sits at (21, 21) in DOOR's own frame, so the value is drawn at
    // (121, 121) at height 2.5; its id is the INSERT's handle and the
    // ATTRIB's, the same id the renderer gives the <text>.
    let nested = texts
        .iter()
        .find(|t| t["id"] == "60/56")
        .unwrap_or_else(|| panic!("the nested attribute: {texts:?}"));
    assert_eq!(nested["text"], "D-101");
    assert_eq!(nested["kind"], "ATTRIB");
    assert_eq!(nested["tag"], "NUM");
    let b: Vec<f64> = nested["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(
        (120.5..122.5).contains(&b[0]) && (120.5..122.5).contains(&b[1]),
        "{b:?}"
    );
    assert!((b[3] - b[1] - 2.5).abs() < 0.3, "cap height 2.5: {b:?}");
    // The top-level attribute, which always worked, is still there once,
    // and the nested one is not listed twice (the fixture's ATTRIB is a
    // child of DOOR, and from R2004 on the same record is linked to the
    // nested INSERT as well).
    assert_eq!(texts.iter().filter(|t| t["text"] == "D-TOP").count(), 1);
    assert_eq!(texts.iter().filter(|t| t["text"] == "D-101").count(), 1);

    // strings.json finds it, and so does the picture: exactly one <text>.
    let strings = read_json(&tmp.0.join("strings.json"));
    assert_eq!(
        strings["strings"]["d-101"],
        serde_json::json!(["60/56"]),
        "{}",
        strings["strings"]
    );
    let svg = std::fs::read_to_string(tmp.0.join("drawing.svg")).unwrap();
    assert_eq!(svg.matches("D-101").count(), 1, "drawn once");
    assert!(svg.contains("id=\"60/56\""), "with the record's id");

    // And the tile the record names really shows it.
    let tile = nested["tiles"].as_array().unwrap()[0].as_str().unwrap();
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let entry = tiles["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tile)
        .expect("the tile");
    let png = std::fs::read(tmp.0.join(entry["png"].as_str().unwrap())).unwrap();
    let area: Vec<i64> = nested["px"][tile]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert!(
        dark_pixels_in(&png, [area[0], area[1], area[2], area[3]]) > 20,
        "{tile} shows no attribute text"
    );
}

/// The same block nesting in the shape a DWG (and an R2004+ DXF) gives:
/// the nested INSERT carries its ATTRIB in `attribs` and the block has no
/// ATTRIB child at all. No R2000 DXF produces that, so it is built here.
fn nested_attrib_dwg_shape() -> uncad::CadDatabase {
    use uncad::model::{AttribEntity, EntityCommon, InsertEntity, LineEntity, Point2D, Point3D};
    let common = |handle: &str| EntityCommon {
        handle: handle.into(),
        layer: "0".into(),
        ..EntityCommon::default()
    };
    let line = |handle: &str, x1: f64, y1: f64| {
        uncad::Entity::Line(LineEntity {
            common: common(handle),
            start_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: x1,
                y: y1,
                z: 0.0,
            },
        })
    };
    let insert = |handle: &str, block: &str, x: f64, y: f64, attribs: Vec<AttribEntity>| {
        uncad::Entity::Insert(InsertEntity {
            common: common(handle),
            block_name: block.into(),
            insertion_point: Point3D { x, y, z: 0.0 },
            scale: Point3D {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            rotation: 0.0,
            extrusion: Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            attribs,
        })
    };
    let value = AttribEntity {
        common: common("56"),
        start_point: Point2D { x: 21.0, y: 21.0 },
        text_height: 2.5,
        text: "D-101".into(),
        text_plain: "D-101".into(),
        rotation: 0.0,
        tag: "NUM".into(),
        invisible: false,
        horizontal_alignment: 0,
        vertical_alignment: 0,
        alignment_point: None,
        width_factor: 1.0,
        oblique_angle: 0.0,
        style: String::new(),
    };
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "TAG".into(),
        uncad::tables::BlockRecord {
            name: "TAG".into(),
            entities: vec![line("42", 10.0, 0.0)],
        },
    );
    tables.block_records.insert(
        "DOOR".into(),
        uncad::tables::BlockRecord {
            name: "DOOR".into(),
            entities: vec![
                line("52", 40.0, 0.0),
                line("53", 0.0, 40.0),
                insert("55", "TAG", 20.0, 20.0, vec![value]),
            ],
        },
    );
    let model = vec![insert("60", "DOOR", 100.0, 100.0, Vec::new())];
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: model.clone(),
        },
    );
    uncad::CadDatabase::new(model, tables)
}

#[test]
fn a_nested_attribute_stored_on_the_insert_is_drawn_and_indexed_too() {
    let db = nested_attrib_dwg_shape();
    let tmp = TempDir::new("nested_attrib_dwg");
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            svg: true,
            ..Default::default()
        },
    )
    .expect("exports");
    let texts = records(&tmp.0, "texts");
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert_eq!(texts[0]["id"], "60/56");
    assert_eq!(texts[0]["text"], "D-101");
    // Same placement as the DXF shape: DOOR at (100, 100), the value at
    // (21, 21) inside it.
    let b: Vec<f64> = texts[0]["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(
        (120.5..122.5).contains(&b[0]) && (120.5..122.5).contains(&b[1]),
        "{b:?}"
    );
    let strings = read_json(&tmp.0.join("strings.json"));
    assert_eq!(strings["strings"]["d-101"], serde_json::json!(["60/56"]));
    let svg = std::fs::read_to_string(tmp.0.join("drawing.svg")).unwrap();
    assert_eq!(svg.matches("D-101").count(), 1, "drawn once: {svg}");
}

#[test]
fn exporting_onto_an_existing_file_reports_the_path_it_could_not_write() {
    // `export_package` creates its directory; when the path is a file, the
    // very first `create_dir_all` fails and the error names that path.
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("io");
    std::fs::create_dir_all(&tmp.0).expect("a writable directory");
    let file = tmp.0.join("not-a-directory");
    std::fs::write(&file, b"in the way").expect("writable");
    let err = export_package(&db, &file, &ExportOptions::default())
        .expect_err("a file is not a directory");
    match &err {
        ExportError::Io { path, source } => {
            assert_eq!(path, &file);
            assert!(
                matches!(
                    source.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory
                ),
                "{source:?}"
            );
        }
        other => panic!("expected an Io error, got {other:?}"),
    }
    let message = err.to_string();
    assert!(
        message.starts_with(&format!("cannot write {}", file.display())),
        "{message}"
    );
    // Nothing was written next to the file that blocked it.
    assert!(file.is_file());
}

/// The same drawing as `EXAMPLE_2000_DWG`, saved in a format that predates
/// `act_measurement`.
const EXAMPLE_R14_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_r14.dwg"
);

#[test]
fn a_package_says_when_its_dimension_values_were_recomputed() {
    // An R14 file stores no measurement, and the records used to present
    // the 0.0 it leaves behind as one: `measurement: 0`, `confidence:
    // "stored"`, `capabilities.dimension_values: "exact"`, beside a
    // `display` of "1504,68". The values now come from the definition
    // points, and every place that says where a number came from says so.
    let db = uncad::parse(EXAMPLE_R14_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("r14");
    export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");

    let dims = records(&tmp.0, "dimensions");
    assert_eq!(dims.len(), 10);
    for d in &dims {
        assert_eq!(d["measurement_source"], "from_points", "{}", d["id"]);
        assert_eq!(d["confidence"], "exact", "{}", d["id"]);
        // The value is the one the points give, and it is not 0.
        assert_eq!(
            d["measurement"], d["measurement_from_points"],
            "{}",
            d["id"]
        );
        assert!(
            d["measurement"].as_f64().is_some_and(|m| m.abs() > 1.0),
            "{} measures {}",
            d["id"],
            d["measurement"]
        );
        // Nothing stored means nothing to compare against.
        assert!(d["delta"].is_null(), "{}", d["id"]);
    }
    let manifest = read_json(&tmp.0.join("manifest.json"));
    assert_eq!(manifest["capabilities"]["dimension_values"], "computed");
}
