//! Paper layouts (docs/VLM_EXPORT_DESIGN.md, section 4 step 10): the
//! LAYOUT objects and their plot settings, the viewport view fields, the
//! paper-to-model mapping, and the sheet images `uncad export` writes.
//! Sources: the corpus's `example_2000.dwg` (two Letter layouts, each with
//! only its overall viewport) and this project's `twisted_viewport_r2000.dxf`
//! (a 200 x 120 viewport at scale 2 twisted 30 degrees over a model line).

use std::path::{Path, PathBuf};

use serde_json::Value;
use uncad::export::{export_package, ExportOptions};
use uncad::model::Point2D;
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const TWISTED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/twisted_viewport_r2000.dxf"
);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("sheets_{name}_{}", std::process::id()));
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

fn viewport(db: &uncad::CadDatabase) -> &uncad::model::ViewportEntity {
    db.entities
        .iter()
        .find_map(|e| match e {
            Entity::Viewport(v) => Some(v),
            _ => None,
        })
        .expect("a VIEWPORT")
}

#[test]
fn autocad_layouts_carry_their_plot_settings() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let layouts = &db.tables.layouts;
    assert_eq!(
        layouts.keys().cloned().collect::<Vec<_>>(),
        ["Layout1", "Layout2", "Model"]
    );
    let model = &layouts["Model"];
    assert_eq!(
        (model.tab_order, model.block_name.as_str()),
        (0, "*Model_Space")
    );
    let l1 = &layouts["Layout1"];
    assert_eq!((l1.tab_order, l1.block_name.as_str()), (1, "*Paper_Space0"));
    assert_eq!(l1.active_viewport.as_deref(), Some("84"));
    assert_eq!(l1.extmin, None, "never computed: +-1e20 in the file");
    let p = &l1.plot;
    assert_eq!(p.paper_name, "Letter_(8.50_x_11.00_Inches)");
    assert_eq!(p.printer, "none_device");
    assert!((p.paper_width_mm - 215.9).abs() < 1e-3 && (p.paper_height_mm - 279.4).abs() < 1e-3);
    assert!(p.margins_mm.iter().all(|m| (m - 6.35).abs() < 1e-4));
    assert_eq!((p.paper_units, p.rotation, p.plot_type), (1, 1, 5));
    assert_eq!((p.std_scale_type, p.scale), (16, 1.0));
    // Landscape Letter with the printable corner at the origin.
    let sheet = p.sheet_rect().expect("a paper size");
    for (got, want) in [
        (sheet.min_x, -6.35),
        (sheet.min_y, -6.35),
        (sheet.max_x, 273.05),
        (sheet.max_y, 209.55),
    ] {
        assert!((got - want).abs() < 1e-3, "{got} vs {want}");
    }
    // The file's own limits say the same.
    assert!((l1.limmax.x - 273.05).abs() < 1e-3 && (l1.limmin.y + 6.35).abs() < 1e-3);
    // The Model tab is set up in inches with a custom scale.
    assert_eq!(model.plot.paper_units, 0);
    assert!((model.plot.scale - 0.012749).abs() < 1e-5);
    assert_eq!(db.header.format, "dwg");

    // Its viewports are the layouts' overall frames: scale 1, centred.
    let overall: Vec<&uncad::model::ViewportEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Viewport(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(overall.len(), 2);
    for v in overall {
        assert!(v.is_overall(), "{v:?}");
        assert_eq!(v.scale(), Some(1.0));
        assert!(v.on && v.is_plan());
    }
}

#[test]
fn the_twisted_viewport_maps_paper_to_model_and_back() {
    let db = uncad::parse(TWISTED).expect("fixture must parse");
    assert_eq!(db.header.format, "dxf");
    let v = viewport(&db);
    assert_eq!((v.view_center.x, v.view_center.y), (50.0, 25.0));
    assert_eq!(v.view_size, 60.0);
    assert!((v.twist - 30f64.to_radians()).abs() < 1e-12);
    assert_eq!((v.status_flag, v.id), (32864, 2));
    assert!(v.on && v.is_plan() && !v.is_overall());
    assert_eq!(v.scale(), Some(2.0));

    // The frame's corners in the world (ezdxf's convention): the view
    // centre un-rotated by the twist, the window 100 x 60 model units.
    let window = v.model_window().expect("a stored view");
    let expected = [
        (-2.5, -4.330),
        (84.103, -54.330),
        (114.103, -2.369),
        (27.5, 47.631),
    ];
    for (got, (x, y)) in window.iter().zip(expected) {
        assert!(
            (got.x - x).abs() < 1e-3 && (got.y - y).abs() < 1e-3,
            "{got:?} vs ({x}, {y})"
        );
    }
    // Round trip through both mappings, and the model line's ends on paper.
    for p in [
        Point2D { x: 0.0, y: 0.0 },
        Point2D { x: 100.0, y: 50.0 },
        Point2D { x: -37.5, y: 12.25 },
    ] {
        let on_paper = v.model_to_paper(p).unwrap();
        let back = v.paper_to_model(on_paper).unwrap();
        assert!((back.x - p.x).abs() < 1e-9 && (back.y - p.y).abs() < 1e-9);
    }
    let origin = v.model_to_paper(Point2D { x: 0.0, y: 0.0 }).unwrap();
    assert!(
        (origin.x - 50.0).abs() < 1e-9 && (origin.y - 50.0).abs() < 1e-9,
        "{origin:?}"
    );
    let end = v.model_to_paper(Point2D { x: 100.0, y: 50.0 }).unwrap();
    assert!(
        (end.x - 173.205).abs() < 1e-3 && (end.y - 236.603).abs() < 1e-3,
        "{end:?}"
    );
}

#[test]
fn the_export_writes_one_sheet_per_paper_layout() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let tmp = TempDir::new("letter");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(report.sheets.len(), 2);
    assert_eq!(report.counts.sheets, 2);
    let s1 = &report.sheets[0];
    assert_eq!(
        (s1.name.as_str(), s1.tab_order, s1.units.as_str()),
        ("Layout1", 1, "mm")
    );
    assert_eq!(s1.rect_source, "paper_size");
    assert!((s1.rect.max_x - 273.05).abs() < 1e-3);
    assert_eq!(s1.overview.png, "sheets/Layout1/overview.png");
    assert!(tmp.0.join(&s1.overview.png).exists());
    assert_eq!(s1.overview.px[0] % 28, 0);
    // The whole sheet, no padding: the image's world rectangle is the sheet
    // (grown only by the lattice snap).
    assert!((s1.overview.world.min_x - s1.rect.min_x).abs() < 1e-9);
    assert!((s1.overview.world.max_y - s1.rect.max_y).abs() < 1e-9);
    assert_eq!(s1.viewports.len(), 1);
    assert!(s1.viewports[0].overall && !s1.viewports[0].composited);
    assert_eq!(s1.viewports[0].scale, Some(1.0));

    let sheets = read_json(&tmp.0.join("sheets.json"));
    assert_eq!(sheets["sheets"].as_array().unwrap().len(), 2);
    assert!(sheets["twist_convention"]
        .as_str()
        .unwrap()
        .contains("counter-clockwise"));
    let manifest = read_json(&tmp.0.join("manifest.json"));
    assert_eq!(manifest["sheets"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["capabilities"]["paper_layouts"], "composited");
    assert_eq!(manifest["counts"]["sheets"], 2);

    // Opting out.
    let quiet = export_package(
        &db,
        &TempDir::new("nosheets").0,
        &ExportOptions {
            max_levels: 0,
            sheets: false,
            ..Default::default()
        },
    )
    .expect("exports");
    assert!(quiet.sheets.is_empty());
    assert!(!quiet.files.iter().any(|f| f.path.starts_with("sheets")));
}

#[test]
fn the_model_is_composited_through_a_real_viewport() {
    let db = uncad::parse(TWISTED).expect("fixture must parse");
    let tmp = TempDir::new("twisted");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    assert_eq!(report.sheets.len(), 1, "{:?}", report.sheets);
    let sheet = &report.sheets[0];
    assert_eq!(sheet.viewports.len(), 1);
    let vp = &sheet.viewports[0];
    assert!(vp.composited && !vp.overall && vp.on);
    assert_eq!(vp.scale, Some(2.0));
    assert!((vp.twist_deg - 30.0).abs() < 1e-9);
    let window = vp.model_window.expect("a stored view");
    assert!((window[0][0] + 2.5).abs() < 1e-3 && (window[3][1] - 47.631).abs() < 1e-3);
    // The frame is the 200 x 120 viewport; the sheet rectangle covers it.
    assert!(sheet.rect.min_x <= 50.0 && sheet.rect.max_x >= 250.0);
    assert!(sheet.rect.min_y <= 40.0 && sheet.rect.max_y >= 160.0);

    // The image shows the model line through the viewport: it enters at the
    // frame's lower-left corner (paper (50,50)) and leaves through the top
    // edge, so there is ink along that diagonal and none in the frame's
    // lower-right quarter.
    let png = std::fs::read(tmp.0.join(&sheet.overview.png)).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("a frame size")];
    let info = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let stride = 3;
    let dark = |x: usize, y: usize| buf[(y * w + x) * stride] < 128;
    let ov = &sheet.overview;
    let at = |wx: f64, wy: f64| -> (usize, usize) {
        let px = ((wx - ov.world.min_x) * ov.ppu).round() as usize;
        let py = ((ov.world.max_y - wy) * ov.ppu).round() as usize;
        (px.min(w - 1), py.min(h - 1))
    };
    // A point on the composited line: model (40, 20) -> paper.
    let v = viewport(&db);
    let on_line = v.model_to_paper(Point2D { x: 40.0, y: 20.0 }).unwrap();
    let (lx, ly) = at(on_line.x, on_line.y);
    let hit = (0..3).any(|dx| {
        (0..3).any(|dy| {
            dark(
                (lx + dx).saturating_sub(1).min(w - 1),
                (ly + dy).saturating_sub(1).min(h - 1),
            )
        })
    });
    assert!(hit, "no ink at the composited line ({lx}, {ly})");
    // Well inside the frame's lower-right quarter: blank.
    let (bx, by) = at(220.0, 70.0);
    assert!(!dark(bx, by), "unexpected ink at ({bx}, {by})");
}
