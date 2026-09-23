//! Frames and the tile pyramid: the levels a frame's text asks for, tiles
//! that overlap, are culled to what is on them and hashed, sidecars whose
//! affines round-trip and whose every pointer leads to a file, and the
//! pixels of a tile showing what its records say is there.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use serde_json::Value;
use uncad_export::{ExportOptions, Profile};
use uncad_model::model::Entity;

fn example_2000() -> String {
    corpus("example_2000.dwg")
}

fn written_tiles(dir: &std::path::Path) -> BTreeSet<String> {
    read_json(&dir.join("tiles.json"))["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["png"].is_string())
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tiles_cover_the_levels_and_their_sidecars_round_trip() {
    let tmp = TempDir::new("tiles");
    let report = export_path(&example_2000(), &tmp.0, &ExportOptions::default()).expect("exports");
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
        let p2w = sidecar["px_to_world"].as_array().unwrap();
        let w2p = sidecar["world_to_px"].as_array().unwrap();
        let (a, c, e, f) = (
            w2p[0].as_f64().unwrap(),
            w2p[2].as_f64().unwrap(),
            w2p[4].as_f64().unwrap(),
            w2p[5].as_f64().unwrap(),
        );
        assert!((a - ppu).abs() < 1e-12 && (e + ppu).abs() < 1e-12);
        // px -> world -> px on the tile's far corner.
        let px = sidecar["px"].as_array().unwrap();
        let (pw, ph) = (px[0].as_f64().unwrap(), px[1].as_f64().unwrap());
        let wx = p2w[0].as_f64().unwrap() * pw + p2w[2].as_f64().unwrap();
        let wy = p2w[4].as_f64().unwrap() * ph + p2w[5].as_f64().unwrap();
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
        // The PNG is exactly the size the sidecar says, in 8-bit RGB.
        let bytes = std::fs::read(tmp.0.join(png)).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let info = decoder.read_info().unwrap();
        let header = info.info();
        assert_eq!(
            (f64::from(header.width), f64::from(header.height)),
            (pw, ph)
        );
        assert_eq!(header.color_type, png::ColorType::Rgb);
    }
}

#[test]
fn every_record_points_at_tiles_that_exist() {
    let tmp = TempDir::new("records_tiles");
    export_path(&example_2000(), &tmp.0, &ExportOptions::default()).expect("exports");
    let written = written_tiles(&tmp.0);
    for kind in ["texts", "dimensions", "geometry", "regions", "blocks"] {
        for record in records(&tmp.0, kind) {
            for tile in record["tiles"].as_array().unwrap() {
                assert!(written.contains(tile.as_str().unwrap()), "{record}");
                assert!(record["px"][tile.as_str().unwrap()].is_array(), "{record}");
            }
            assert!(record["px"]["ov"].is_array(), "{record}");
        }
    }
}

/// Two 100 x 100 squares of 25 lines each, 1000 units apart, the second
/// with a label: two frames.
fn two_islands() -> uncad::CadDatabase {
    let mut entities = Vec::new();
    let mut id = 0x100u64;
    for dx in [0.0, 1000.0] {
        for i in 0..25 {
            let y = f64::from(i) * 4.0;
            entities.push(line(id, dx, y, dx + 100.0, y));
            id += 1;
        }
    }
    entities.push(text(0x7000, 1010.0, 110.0, 5.0, "DETAIL A"));
    model_space(entities)
}

#[test]
fn a_detached_group_becomes_its_own_frame() {
    let db = two_islands();
    let tmp = TempDir::new("frames");
    let report = export_db(
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
    assert_eq!(f0.overview.png, "frames/f0/overview.png");
    assert!(tmp.0.join("frames/f1/overview.png").exists());
    assert!(tmp.0.join("overview.png").exists());
    assert!(f0.content.min_x >= 999.0 && f1.content.max_x <= 101.0);
    // Each frame overview shows its own island.
    for f in [f0, f1] {
        let png = std::fs::read(tmp.0.join(&f.overview.png)).unwrap();
        assert!(dark_pixels(&png) > 500, "{} shows its lines", f.id);
    }
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
    let label = by_handle(&texts, "7000");
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
    let merged = export_db(
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

#[test]
fn padding_sets_the_window_of_the_overview_and_every_frame() {
    let db = two_islands();
    let run = |name: &str, padding: Option<f64>| {
        let tmp = TempDir::new(name);
        let report = export_db(
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
    assert!(
        automatic.overview.world.width() > tight.overview.world.width()
            && automatic.overview.world.width() < roomy.overview.world.width()
    );
    assert_eq!(tight.crop.padding_units, 0.0);
    assert_eq!(roomy.crop.padding_units, 200.0);
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

/// An 11 x 11 grid of 1000-unit lines with a 3-unit label in every cell
/// (small enough to need two zoom levels), shifted by `offset` on both
/// axes.
fn labelled_grid(offset: f64) -> uncad::CadDatabase {
    let mut entities = Vec::new();
    let mut id = 0x100u64;
    for i in 0..=10 {
        let at = offset + f64::from(i) * 100.0;
        entities.push(line(id, offset, at, offset + 1000.0, at));
        entities.push(line(id + 1, at, offset, at, offset + 1000.0));
        id += 2;
    }
    for row in 0..10 {
        for col in 0..10 {
            entities.push(text(
                id,
                offset + f64::from(col) * 100.0 + 20.0,
                offset + f64::from(row) * 100.0 + 40.0,
                3.0,
                &format!("R{row}{col}"),
            ));
            id += 1;
        }
    }
    model_space(entities)
}

fn small() -> Profile {
    Profile {
        name: "claude-small",
        overview_edge: 784,
        overview_patches: 784,
        ..Profile::CLAUDE
    }
}

#[test]
fn a_drawing_far_from_the_origin_rasterizes_like_one_at_the_origin() {
    // usvg and tiny-skia keep path points in f32. The renderer writes its
    // coordinates relative to the drawing's own origin whenever that
    // exceeds 32768, and every tile is a window of that same document, so
    // the same grid gives the same picture wherever it sits: the dark
    // pixel counts of each tile level agree within a few percent.
    let mut levels: Vec<Vec<usize>> = Vec::new();
    for (name, offset) in [("0", 0.0), ("1e7", 1.0e7), ("2.5e8", 2.5e8)] {
        let db = labelled_grid(offset);
        let tmp = TempDir::new(&format!("far_{name}"));
        let report = export_db(
            &db,
            &tmp.0,
            &ExportOptions {
                max_levels: 2,
                profile: small(),
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
    let within = |a: usize, b: usize| (a as f64 - b as f64).abs() / a as f64 <= 0.03;
    assert!(levels[0][1] > 20_000, "the grid and its labels: {levels:?}");
    for i in 1..3 {
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

#[test]
fn a_sub_millimetre_drawing_keeps_its_tile_rectangles_apart() {
    // A drawing 0.004 x 0.003 units across. The world rectangles are
    // rounded to at least the decimals the deepest scale needs, so no two
    // tiles print the same rectangle and the affine beside each maps the
    // rounded corners onto the image's corners.
    let (w, h) = (0.004, 0.003);
    let mut entities = vec![
        line(0x100, 0.0, 0.0, w, 0.0),
        line(0x101, w, 0.0, w, h),
        line(0x102, w, h, 0.0, h),
        line(0x103, 0.0, h, 0.0, 0.0),
    ];
    for i in 1..12 {
        let y = h * f64::from(i) / 12.0;
        entities.push(line(0x110 + i as u64, 0.0, y, w, y));
    }
    entities.push(text(0x200, w * 0.1, h * 0.5, 1e-5, "0.004"));
    let db = model_space(entities);
    let tmp = TempDir::new("tiny");
    let report = export_db(
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
        "{:?}",
        report.overview.world
    );
    let deepest = report.frames[0].z_max;
    assert_eq!(deepest, 1);
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(sidecar_path) = entry["sidecar"].as_str() else {
            continue;
        };
        assert!(
            seen.insert(entry["world"].to_string()),
            "two tiles share the rectangle {}: {}",
            entry["world"],
            entry["id"]
        );
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
        for (got, want) in [
            (w2p[0] * world[0] + w2p[2], 0.0),
            (w2p[0] * world[2] + w2p[2], pw),
            (w2p[4] * world[3] + w2p[5], 0.0),
            (w2p[4] * world[1] + w2p[5], ph),
        ] {
            assert!(
                (got - want).abs() < 0.01,
                "{}: {got} vs {want}",
                entry["id"]
            );
        }
        checked += 1;
    }
    assert!(checked > 1, "the deepest level holds more than one tile");
}

#[test]
fn a_dense_drawing_keeps_every_sidecar_under_the_32_kb_cap() {
    // 6000 small closed squares: 6000 geometry and 6000 region records,
    // enough on one tile to push its sidecar past its 32 KB budget, which
    // the compact form on disk must still meet.
    let count = 6000usize;
    let side = (count as f64).sqrt().ceil() as usize;
    let entities: Vec<Entity> = (0..count)
        .map(|i| {
            let (x, y) = ((i % side) as f64 * 10.0, (i / side) as f64 * 10.0);
            lwpolyline(
                0x1000 + i as u64,
                &[(x, y), (x + 6.0, y), (x + 6.0, y + 6.0), (x, y + 6.0)],
                &[],
                true,
            )
        })
        .collect();
    let tmp = TempDir::new("dense");
    let report = export_db(
        &model_space(entities),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(report.counts.regions, 6000);
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let (mut checked, mut truncated) = (0, 0);
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let file = tmp.0.join(path);
        let bytes = std::fs::metadata(&file).unwrap().len();
        assert!(bytes <= 32 * 1024, "{path} is {bytes} bytes");
        let sidecar = read_json(&file);
        assert!(sidecar["records"]["regions"].is_array());
        if sidecar["records_truncated"] == true {
            truncated += 1;
        }
        checked += 1;
    }
    assert!(checked >= 2, "{checked} sidecars");
    assert!(truncated >= 1, "6000 regions must overflow a sidecar");
}

#[test]
fn a_tile_on_hundreds_of_layers_keeps_its_sidecar_under_the_cap() {
    // 900 layers with 45-character names, one line each, all over the same
    // patch of the drawing: the layer list alone cannot fit, so it is cut
    // after the rows, and said so.
    let entities: Vec<Entity> = (0..900usize)
        .map(|i| {
            let (x, y) = (20.0 + (i % 17) as f64, 20.0 + (i % 13) as f64);
            let mut e = line(0x1000 + i as u64, x, y, x + 8.0, y + 6.0);
            let layer = format!("A-WALL-FULL-DIMS-ANNO-TEXT-IDENTITY-PATT-{i:04}");
            *e.common_mut() = on_layer(e.common().clone(), &layer);
            e
        })
        .collect();
    let tmp = TempDir::new("layers_everywhere");
    export_db(
        &model_space(entities),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let (mut checked, mut trimmed) = (0, 0);
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let file = tmp.0.join(path);
        let bytes = std::fs::metadata(&file).unwrap().len();
        assert!(bytes <= 32 * 1024, "{path} is {bytes} bytes");
        let sidecar = read_json(&file);
        let present = sidecar["layers_present"].as_array().unwrap().len();
        if sidecar["layers_truncated"] == true {
            let total = sidecar["layers_total"].as_u64().unwrap() as usize;
            assert!(
                total > present && total <= 900,
                "{path}: {present} of {total}"
            );
            assert_eq!(sidecar["records_truncated"], true, "{path}");
            trimmed += 1;
        } else {
            assert!(sidecar["layers_total"].is_null());
        }
        assert!(
            sidecar["records"]["geometry"].as_array().unwrap().len()
                <= sidecar["counts"]["geometry"].as_u64().unwrap() as usize
        );
        checked += 1;
    }
    assert!(checked >= 2, "{checked} sidecars");
    assert!(trimmed >= 1, "900 layers on one tile must overflow it");
}

#[test]
fn a_sidecar_lists_the_layers_and_the_geometry_of_everything_on_its_tile() {
    let tmp = TempDir::new("layers");
    export_path(
        &example_2000(),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    // What each tile shows, derived from the records' own `tiles` lists.
    let mut expected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut geometry_on: BTreeMap<String, usize> = BTreeMap::new();
    let mut geometry_layers: BTreeSet<String> = BTreeSet::new();
    for kind in ["texts", "dimensions", "geometry", "regions", "blocks"] {
        for record in records(&tmp.0, kind) {
            let layer = record["layer"].as_str().unwrap().to_string();
            if kind == "geometry" {
                geometry_layers.insert(layer.clone());
            }
            for tile in record["tiles"].as_array().unwrap() {
                let tile = tile.as_str().unwrap().to_string();
                if kind == "geometry" {
                    *geometry_on.entry(tile.clone()).or_default() += 1;
                }
                expected.entry(tile).or_default().insert(layer.clone());
            }
        }
    }
    assert!(geometry_layers.len() > 1, "{geometry_layers:?}");
    assert!(geometry_on.values().any(|n| *n > 3));
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
            "{id}"
        );
        let want = geometry_on.get(id).copied().unwrap_or(0);
        assert_eq!(
            sidecar["counts"]["geometry"].as_u64().unwrap() as usize,
            want
        );
        let by_kind: usize = sidecar["geometry_by_kind"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_u64().unwrap() as usize)
            .sum();
        assert_eq!(by_kind, want, "{id}");
        seen.extend(listed);
        checked += 1;
    }
    assert!(checked >= 2, "{checked} sidecars");
    for layer in &geometry_layers {
        assert!(seen.contains(layer), "{layer} is on no sidecar");
    }
}

#[test]
fn the_guidance_quotes_the_profile_in_use() {
    let (db, header) = uncad::parse_with_header(example_2000()).expect("parses");
    for profile in [
        Profile::CLAUDE,
        Profile::CLAUDE_HIRES,
        Profile::OPENAI_PATCH,
    ] {
        let tmp = TempDir::new(&format!("guidance_{}", profile.name));
        let report = uncad_export::export_package(
            &db,
            Some(&header),
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
        for level in &report.frames[0].levels {
            assert_eq!(level.overlap_px, profile.overlap);
            assert_eq!(level.tile_px, profile.tile);
        }
    }
}

#[test]
fn re_exporting_clears_the_previous_package_but_nothing_else() {
    let db = labelled_grid(0.0);
    let tmp = TempDir::new("reexport");
    let first = export_db(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 2,
            shard_kb: 4,
            profile: small(),
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

    let second = export_db(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            profile: small(),
            ..Default::default()
        },
    )
    .expect("exports again");
    assert_eq!(second.frames[0].levels.len(), 1);
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
    assert!(!on_disk.iter().any(|p| p.starts_with("texts.0")));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"mine");
    assert!(tmp.0.join("frames/f0/tiles/z1").exists());
    assert!(
        tmp.0.join("frames/f0/tiles/z2").exists(),
        "keep.txt is in it"
    );
    std::fs::remove_file(&kept_tile).unwrap();
    export_db(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            profile: small(),
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
    export_db(
        &db,
        &foreign.0,
        &ExportOptions {
            max_levels: 0,
            profile: small(),
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
    // Content with no size at all -- the corpus's Point.dwg, RAY.dwg and
    // ConstructionLine.dwg, whose entities record a single base point --
    // gets a ten-unit window, and the picture shows the entity.
    for name in ["Point", "RAY", "ConstructionLine"] {
        let tmp = TempDir::new(&format!("degenerate_{name}"));
        let report = export_path(
            &corpus(&format!("2000/{name}.dwg")),
            &tmp.0,
            &ExportOptions {
                max_levels: 1,
                ..Default::default()
            },
        )
        .expect("exports");
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
        // A RAY and an XLINE cross the whole window; a POINT is drawn as a
        // cross sized in pixels, about a dozen dark pixels.
        let png = std::fs::read(tmp.0.join("overview.png")).unwrap();
        let floor = if name == "Point" { 8 } else { 100 };
        assert!(dark_pixels(&png) > floor, "{name}: {}", dark_pixels(&png));
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
            let (px, py) = (sx * w[0] + cx, sy * w[3] + cy);
            assert!(px.abs() < 1.0 && py.abs() < 1.0, "{name}: ({px}, {py})");
            checked += 1;
        }
        assert!(checked >= 1, "{name}: no tile was written");
    }
}

#[test]
fn a_tile_keeps_the_half_of_a_long_hangul_text_that_reaches_it() {
    // A 3000 x 1000 frame with a centre line and one note of 100 Hangul
    // syllables at (40, 510), height 20. Each advances about 0.92 of the
    // text height where the renderer's estimate allows 0.8186 (0.6 em over
    // the bundled font's 0.733 cap height), so the string runs some 200
    // units past the estimate -- and the tiles are culled by the measured
    // box, so the tile past the estimate still draws the text.
    let note = "\u{ac00}\u{b098}\u{b2e4}\u{b77c}".repeat(25);
    let db = model_space(vec![
        line(0x10, 0.0, 0.0, 3000.0, 0.0),
        line(0x11, 3000.0, 0.0, 3000.0, 1000.0),
        line(0x12, 3000.0, 1000.0, 0.0, 1000.0),
        line(0x13, 0.0, 1000.0, 0.0, 0.0),
        line(0x14, 0.0, 500.0, 3000.0, 500.0),
        text(0x20, 40.0, 510.0, 20.0, &note),
    ]);
    let tmp = TempDir::new("hangul_tile");
    export_db(
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
    let note = by_handle(&texts, "20");
    assert_eq!(note["bbox_confidence"], "measured");
    let b: Vec<f64> = note["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let estimate_end = 40.0 + 100.0 * (0.6 / 0.733) * 20.0;
    assert!(b[2] > estimate_end + 100.0, "{b:?}");
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let listed: BTreeSet<&str> = note["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
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
        let area: Vec<i64> = note["px"][id]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let ink = dark_pixels_in(&png, Some([area[0], area[1], area[2], area[3]]));
        assert!(ink > 200, "{id}: {ink} dark pixels in the text band");
        checked += 1;
    }
    assert!(checked >= 1, "no tile starts past the estimate's end");
}

#[test]
fn a_nested_attribute_is_on_the_tile_its_record_names() {
    let tmp = TempDir::new("nested_attrib_tile");
    export_path(
        &fixture("nested_attrib_r2000.dxf"),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    let texts = records(&tmp.0, "texts");
    let nested = texts.iter().find(|t| t["text"] == "D-101").expect("D-101");
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
        dark_pixels_in(&png, Some([area[0], area[1], area[2], area[3]])) > 20,
        "{tile} shows no attribute text"
    );
}

#[test]
fn every_pointer_in_a_sidecar_leads_somewhere_and_every_box_is_on_the_image() {
    let tmp = TempDir::new("pointers");
    export_path(&example_2000(), &tmp.0, &ExportOptions::default()).expect("exports");
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let written: BTreeSet<String> = tiles["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["empty"] == false)
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    let empty = tiles["tiles"].as_array().unwrap().len() - written.len();
    assert!(empty > 0, "the test needs a package with culled tiles");
    let mut boxes = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let sidecar = read_json(&tmp.0.join(path));
        let mut pointers: Vec<&Value> = ["n", "s", "e", "w"]
            .iter()
            .map(|d| &sidecar["neighbors"][*d])
            .collect();
        pointers.push(&sidecar["parent"]);
        pointers.extend(sidecar["children"].as_array().unwrap());
        for pointer in pointers {
            let Some(id) = pointer.as_str() else { continue };
            assert!(written.contains(id), "{path} points at {id}, never written");
        }
        let (w, h) = (
            sidecar["px"][0].as_i64().unwrap(),
            sidecar["px"][1].as_i64().unwrap(),
        );
        for group in ["texts", "dims", "blocks", "regions", "geometry"] {
            for row in sidecar["records"][group].as_array().unwrap() {
                let v: Vec<i64> = row[1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_i64().unwrap())
                    .collect();
                assert!(
                    0 <= v[0]
                        && v[0] <= v[2]
                        && v[2] <= w
                        && 0 <= v[1]
                        && v[1] <= v[3]
                        && v[3] <= h,
                    "{path}: {group} {row} on a {w}x{h} image"
                );
                boxes += 1;
            }
        }
    }
    assert!(boxes > 20, "{boxes} boxes");
    let mut spanning = 0;
    for record in records(&tmp.0, "geometry") {
        if record["tiles"].as_array().unwrap().len() < 2 {
            continue;
        }
        spanning += 1;
        for (id, value) in record["px"].as_object().unwrap() {
            if !written.contains(id.as_str()) {
                continue;
            }
            let v: Vec<i64> = value
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_i64().unwrap())
                .collect();
            assert!(
                v[0] >= 0 && v[1] >= 0 && v[2] <= 1092 && v[3] <= 1092,
                "{record} on {id}"
            );
        }
    }
    assert!(spanning > 0, "no record spans two tiles");
}

#[test]
fn the_package_explains_its_own_compact_forms() {
    let tmp = TempDir::new("legend");
    export_path(
        &example_2000(),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            ..Default::default()
        },
    )
    .expect("exports");
    let manifest = read_json(&tmp.0.join("manifest.json"));
    let legend = &manifest["legend"];
    for key in [
        "confidence",
        "text_records",
        "ids",
        "measurement_source",
        "display_source",
        "region_labels",
        "px_boxes",
        "tile_sidecar",
        "shard_lookup",
        "legibility",
    ] {
        assert!(!legend[key].is_null(), "the legend is missing {key}");
    }
    let confidence = legend["confidence"].as_object().unwrap();
    for record in records(&tmp.0, "geometry")
        .iter()
        .chain(records(&tmp.0, "regions").iter())
        .chain(records(&tmp.0, "dimensions").iter())
    {
        let value = record["confidence"].as_str().expect("a confidence");
        assert!(
            confidence.contains_key(value),
            "{value} is not in the legend"
        );
    }
    let sources = legend["display_source"].as_array().unwrap();
    for record in records(&tmp.0, "dimensions") {
        assert!(sources.contains(&record["display_source"]), "{record}");
    }
    assert!(manifest["guidance"].as_str().unwrap().contains("legend"));
    for record in records(&tmp.0, "blocks") {
        assert!(record["confidence"].is_null(), "{record}");
    }
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let mut checked = 0;
    for entry in tiles["tiles"].as_array().unwrap() {
        let Some(path) = entry["sidecar"].as_str() else {
            continue;
        };
        let sidecar = read_json(&tmp.0.join(path));
        let columns = sidecar["columns"].as_object().expect("a columns legend");
        assert_eq!(columns.len(), 5, "{path}");
        for (group, names) in columns {
            let arity = names.as_array().unwrap().len();
            assert_eq!(names[0], "id");
            assert_eq!(names[1], "px_box");
            for row in sidecar["records"][group].as_array().unwrap() {
                assert_eq!(row.as_array().unwrap().len(), arity, "{path}: {group}");
                checked += 1;
            }
        }
    }
    assert!(checked > 20, "{checked} rows");
}

#[test]
fn a_group_too_small_to_frame_is_listed_as_dropped() {
    // sample_2000.dwg has two groups below `min_frame_entities` and without
    // text: a circle and a 100 x 140 rectangle, both in the overview.
    let tmp = TempDir::new("small_groups");
    export_path(
        &corpus("sample_2000.dwg"),
        &tmp.0,
        &ExportOptions::default(),
    )
    .expect("exports");
    let manifest = read_json(&tmp.0.join("manifest.json"));
    let dropped = manifest["frames_dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    assert_eq!(manifest["frames_dropped_total"], 2);
    assert!(dropped.iter().all(|d| d["reason"] == "below_min_entities"));
    assert!(manifest["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().starts_with("SmallGroups:")));
    // Every record with no tile is inside one of the dropped rectangles.
    let mut without_tiles = 0;
    for record in records(&tmp.0, "geometry") {
        if !record["tiles"].as_array().unwrap().is_empty() {
            continue;
        }
        without_tiles += 1;
        let b: Vec<f64> = record["bbox"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!(
            dropped.iter().any(|d| {
                let r: Vec<f64> = d["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect();
                b[0] >= r[0] - 1e-6
                    && b[1] >= r[1] - 1e-6
                    && b[2] <= r[2] + 1e-6
                    && b[3] <= r[3] + 1e-6
            }),
            "{record} is in none of {dropped:?}"
        );
    }
    assert_eq!(without_tiles, 2);
}

#[test]
fn legibility_says_whether_the_target_was_met_not_whether_the_budget_held() {
    let tmp = TempDir::new("legibility");
    export_path(
        &example_2000(),
        &tmp.0,
        &ExportOptions {
            max_levels: 1,
            target_text_px: 1000.0,
            ..Default::default()
        },
    )
    .expect("exports");
    let manifest = read_json(&tmp.0.join("manifest.json"));
    let legibility = &manifest["legibility"];
    assert_eq!(legibility["target_px"], 1000.0);
    let frame = &legibility["per_frame"][0];
    assert!(frame["reached"].is_null(), "the ambiguous name is gone");
    assert_eq!(frame["pyramid_complete"], true);
    assert_eq!(frame["target_met"], false);
    let classes = frame["height_classes"].as_array().unwrap();
    assert!(!classes.is_empty());
    assert!(classes
        .iter()
        .all(|c| c["legible"] == false && c["px_at_zmax"].as_f64().unwrap() < 1000.0));
    assert_eq!(
        frame["target_met"].as_bool().unwrap(),
        classes.iter().all(|c| c["legible"] == true)
    );
}

#[test]
fn tiles_json_says_how_big_each_tile_is_and_why_an_empty_one_is_not_there() {
    let tmp = TempDir::new("tile_hashes");
    export_path(&example_2000(), &tmp.0, &ExportOptions::default()).expect("exports");
    let tiles = read_json(&tmp.0.join("tiles.json"));
    let mut hashes: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    let (mut written, mut empty) = (0, 0);
    for entry in tiles["tiles"].as_array().unwrap() {
        if entry["empty"] == true {
            empty += 1;
            assert_eq!(entry["reason"], "no_visible_entity_on_tile");
            assert!(entry["png"].is_null());
            continue;
        }
        written += 1;
        let path = tmp.0.join(entry["png"].as_str().unwrap());
        let content = std::fs::read(&path).unwrap();
        assert_eq!(entry["bytes"].as_u64().unwrap(), content.len() as u64);
        let sha = entry["sha256"].as_str().expect("a hash").to_string();
        assert_eq!(sha.len(), 64);
        assert!(sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        if let Some(other) = hashes.get(&content) {
            assert_eq!(*other, sha, "{path:?}");
        }
        assert!(
            hashes.iter().all(|(k, v)| k == &content || v != &sha),
            "two different tiles share a hash"
        );
        hashes.insert(content, sha);
    }
    assert!(written > 5 && empty > 0, "{written} written, {empty} empty");
}
