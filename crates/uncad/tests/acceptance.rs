//! Acceptance questions (docs/VLM_EXPORT_DESIGN.md, P9): the package of
//! `example_2000.dwg` must let an agent answer these without looking at a
//! pixel, following only the manifest's guidance -- strings.json for the
//! lookup, the record for the value, the sidecar for the place. One export
//! serves every question, so this file is a single test.

use std::path::{Path, PathBuf};

use serde_json::Value;
use uncad::export::{export_package, ExportOptions};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The records of a kind, following the manifest's shard index the way an
/// agent would.
fn records(dir: &Path, manifest: &Value, kind: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for shard in manifest["shard_index"].as_array().unwrap() {
        if shard["kind"] == kind {
            let file = read_json(&dir.join(shard["file"].as_str().unwrap()));
            out.extend(file["records"].as_array().unwrap().clone());
        }
    }
    out
}

fn find<'a>(records: &'a [Value], id: &str) -> &'a Value {
    records
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("record {id}"))
}

#[test]
fn the_five_questions_are_answerable_from_the_package_alone() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let dir = TempDir(
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("acceptance_{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    export_package(&db, &dir.0, &ExportOptions::default()).expect("exports");
    let manifest = read_json(&dir.0.join("manifest.json"));
    assert!(manifest["guidance"]
        .as_str()
        .unwrap()
        .starts_with("Read manifest.json first."));
    let unit = manifest["units"]["name"].as_str().unwrap().to_string();
    assert_eq!(unit, "mm");

    // Q1: "What does the aligned dimension read, and is the drawing
    // consistent?" -> dimensions.json: the cached label, the stored value,
    // the value recomputed from the definition points, their difference.
    let dims = records(&dir.0, &manifest, "dimension");
    let aligned = dims
        .iter()
        .find(|d| d["kind"] == "ALIGNED")
        .expect("an ALIGNED dimension");
    assert_eq!(aligned["display"], "1504,68");
    assert_eq!(aligned["display_source"], "cached_block");
    assert_eq!(aligned["unit"], "mm");
    assert!(aligned["delta"].as_f64().unwrap().abs() < 0.001);
    assert_eq!(aligned["measurement_source"], "act_measurement");
    // Every dimension carries a label and a stored value on this R2000 file.
    assert_eq!(dims.len(), 10);
    assert!(dims
        .iter()
        .all(|d| !d["display"].as_str().unwrap().is_empty()));
    assert!(dims.iter().all(|d| d["measurement"].is_number()));

    // Q2: "How many CIRKLO_PUNKTOJ blocks are placed, and where?" ->
    // blocks.json definitions and instances.
    let blocks = read_json(&dir.0.join("blocks.json"));
    let definition = blocks["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "CIRKLO_PUNKTOJ")
        .expect("the block definition");
    assert_eq!(definition["instances"], 8);
    let placed: Vec<&Value> = blocks["instances"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["block"] == "CIRKLO_PUNKTOJ")
        .collect();
    assert_eq!(placed.len(), 8);
    assert!(placed
        .iter()
        .all(|i| i["at"].as_array().unwrap().len() == 2));
    assert!(placed
        .iter()
        .all(|i| !i["tiles"].as_array().unwrap().is_empty()));
    // The attribute of the other block reads as text.
    let bloko = blocks["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["block"] == "bloko")
        .expect("the bloko instance inside the crop");
    assert_eq!(bloko["attribs"]["ETIKEDO"], "valoro de la teksto en bloko");

    // Q3: "Where is the text 'teksto simpla'?" -> strings.json gives the
    // id, texts.json the record, its tiles list an image, and that image's
    // sidecar places it in pixels inside the tile.
    let strings = read_json(&dir.0.join("strings.json"));
    let ids = strings["strings"]["teksto simpla"]
        .as_array()
        .expect("indexed");
    let id = ids[0].as_str().unwrap();
    let texts = records(&dir.0, &manifest, "text");
    let text = find(&texts, id);
    assert_eq!(text["kind"], "TEXT");
    assert_eq!(text["height"], 100.0);
    let tile_id = text["tiles"][0].as_str().expect("on a tile");
    let tiles = read_json(&dir.0.join("tiles.json"));
    let tile = tiles["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tile_id)
        .expect("the tile is listed");
    let sidecar = read_json(&dir.0.join(tile["sidecar"].as_str().unwrap()));
    let row = sidecar["records"]["texts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[0] == id)
        .expect("the sidecar lists the text");
    let px = row[1].as_array().unwrap();
    let (w, h) = (
        sidecar["px"][0].as_i64().unwrap(),
        sidecar["px"][1].as_i64().unwrap(),
    );
    assert!(px[2].as_i64().unwrap() > 0 && px[0].as_i64().unwrap() < w);
    assert!(px[3].as_i64().unwrap() > 0 && px[1].as_i64().unwrap() < h);
    assert_eq!(row[2], "teksto simpla");
    // And the record's own pixel box on that tile is the same box.
    assert_eq!(text["px"][tile_id], row[1]);

    // Q4: "What is the largest closed area, and what is written inside it?"
    // -> regions.json, agreeing with geometry.json for the same entity.
    let regions = records(&dir.0, &manifest, "region");
    let largest = regions
        .iter()
        .max_by(|a, b| {
            a["area"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["area"].as_f64().unwrap())
        })
        .expect("a region");
    assert_eq!(largest["area_unit"], "mm2");
    assert!(largest["area_si"].as_f64().unwrap() > 0.0, "square metres");
    let geometry = records(&dir.0, &manifest, "geometry");
    let same = find(&geometry, largest["id"].as_str().unwrap());
    assert_eq!(same["area"], largest["area"]);
    assert_eq!(same["perimeter"], largest["perimeter"]);
    assert!(largest["labels"].is_array());
    assert_eq!(largest["confidence"], "exact");

    // Q5: "What is not in the picture?" -> report.json and the manifest
    // agree on the two entities the crop left out and why.
    let report = read_json(&dir.0.join("report.json"));
    let excluded = report["excluded"].as_array().unwrap();
    assert_eq!(
        excluded.len(),
        manifest["counts"]["excluded"].as_u64().unwrap() as usize
    );
    let insert = excluded
        .iter()
        .find(|e| e["type_name"] == "INSERT")
        .expect("the 3256x INSERT");
    assert_eq!(insert["reason"], "scale_outlier");
    assert_eq!(insert["handle"], "756");
    assert!(geometry.iter().all(|g| g["id"] != "756"));
    assert!(blocks["instances"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["id"] != "756"));
    assert_eq!(report["hidden"]["by_reason"]["layer_frozen"], 1);
}
