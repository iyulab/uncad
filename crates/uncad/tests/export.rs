//! The package (docs/VLM_EXPORT_DESIGN.md, sections 2, 3 and 5 / P7) on
//! `example_2000.dwg`: every file the design lists, images sized to the
//! Claude profile, sidecars whose affines round-trip, records that point
//! at tiles that exist, and byte-identical output on a second run.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use uncad::export::{export_package, ExportOptions, Profile};

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
fn tiles_cover_the_levels_and_their_sidecars_round_trip() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("tiles");
    let report = export_package(&db, &tmp.0, &ExportOptions::default()).expect("exports");
    assert!(
        !report.levels.is_empty(),
        "the drawing has text, so it has zoom levels"
    );

    let tiles = read_json(&tmp.0.join("tiles.json"));
    let entries = tiles["tiles"].as_array().unwrap();
    let mut written = 0;
    for level in &report.levels {
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
    assert!(texts.iter().all(|t| t["bbox_confidence"] == "estimated"));

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
    assert_eq!(ra.levels.len(), 1);
    assert!(a.0.join("drawing.svg").exists() && a.0.join("entities.json").exists());
    // A 4 KB shard size splits the geometry records.
    assert!(a.0.join("geometry.001.json").exists(), "{:?}", ra.files);
    let manifest = read_json(&a.0.join("manifest.json"));
    assert_eq!(manifest["source"]["name"], "example_2000.dwg");
    assert!(manifest["shard_index"].as_array().unwrap().len() > 5);

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
    assert!(rc.levels.is_empty());
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
