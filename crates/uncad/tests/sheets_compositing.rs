//! Which viewports a sheet composites the model through, and where the
//! sheet of paper lies on the layout (docs/VLM_EXPORT_DESIGN.md, section 4
//! step 10). The rules are: a viewport is composited when it is on, is not
//! the sheet's overall frame, looks straight down the world z axis and
//! stores a view; a viewport whose *border* is hidden (an off, frozen or
//! non-plotting layer -- the usual way to hide it) still shows its window,
//! only the dashed frame goes. `viewport_states_r2000.dxf` is this
//! project's own fixture with one viewport per state (see
//! tests/fixtures/README.md); its two layouts also carry the plot
//! rotations and the inch paper unit no other test file has.

use std::path::{Path, PathBuf};

use uncad::export::{export_package, ExportOptions, SheetReport};
use uncad::model::Point2D;
use uncad::tables::PlotSettings;

const VIEWPORT_STATES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/viewport_states_r2000.dxf"
);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("compositing_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn export(name: &str) -> (TempDir, Vec<SheetReport>) {
    let db = uncad::parse(VIEWPORT_STATES).expect("fixture must parse");
    let tmp = TempDir::new(name);
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("exports");
    (tmp, report.sheets)
}

/// The sheet image as a "is this pixel inked?" function in paper units.
struct Sheet {
    dark: Vec<bool>,
    width: usize,
    height: usize,
    min_x: f64,
    max_y: f64,
    ppu: f64,
}

impl Sheet {
    fn read(dir: &Path, sheet: &SheetReport) -> Sheet {
        let bytes = std::fs::read(dir.join(&sheet.overview.png)).expect("the sheet image");
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("a PNG");
        let mut buf = vec![0; reader.output_buffer_size().expect("a frame size")];
        let info = reader.next_frame(&mut buf).expect("a frame");
        let samples = info.color_type.samples();
        Sheet {
            dark: buf
                .chunks_exact(samples)
                .map(|p| p[0] < 128)
                .collect::<Vec<bool>>(),
            width: info.width as usize,
            height: info.height as usize,
            min_x: sheet.overview.world.min_x,
            max_y: sheet.overview.world.max_y,
            ppu: sheet.overview.ppu,
        }
    }

    fn at(&self, x: f64, y: f64) -> (usize, usize) {
        let px = ((x - self.min_x) * self.ppu).round().max(0.0) as usize;
        let py = ((self.max_y - y) * self.ppu).round().max(0.0) as usize;
        (px.min(self.width - 1), py.min(self.height - 1))
    }

    /// Whether anything is inked within `r` pixels of the paper point.
    fn inked_near(&self, x: f64, y: f64, r: usize) -> bool {
        let (px, py) = self.at(x, y);
        (px.saturating_sub(r)..=(px + r).min(self.width - 1)).any(|x| {
            (py.saturating_sub(r)..=(py + r).min(self.height - 1))
                .any(|y| self.dark[y * self.width + x])
        })
    }

    /// Inked pixels along the horizontal paper segment `y`, `x0..x1`.
    fn inked_along(&self, y: f64, x0: f64, x1: f64) -> usize {
        let (px0, py) = self.at(x0, y);
        let (px1, _) = self.at(x1, y);
        (px0..=px1)
            .filter(|x| {
                (py.saturating_sub(1)..=(py + 1).min(self.height - 1))
                    .any(|y| self.dark[y * self.width + x])
            })
            .count()
    }
}

#[test]
fn only_on_plan_viewports_composite_and_a_hidden_border_still_does() {
    let (tmp, sheets) = export("states");
    assert_eq!(sheets.len(), 2, "{sheets:?}");
    let sheet = &sheets[0];
    assert_eq!((sheet.name.as_str(), sheet.tab_order), ("Layout1", 1));

    // The fixture's four viewports, in the order the block holds them.
    let states: Vec<(&str, u16, bool, bool)> = sheet
        .viewports
        .iter()
        .map(|v| (v.handle.as_str(), v.id, v.on, v.composited))
        .collect();
    assert_eq!(
        states,
        [
            // on, plan, layer 0
            ("2A", 2, true, true),
            // on, plan, on the frozen layer VPFROZEN: still composited
            ("2D", 3, true, true),
            // DXF 68 = 0 and status bit 0x20000: off
            ("2E", 4, false, false),
            // VIEWDIR (1,1,1): not a plan view
            ("2F", 5, true, false),
        ]
    );
    for v in &sheet.viewports {
        assert!(!v.overall, "{}", v.handle);
        // 40 paper units of frame over a 20-unit model window.
        assert_eq!(v.scale, Some(2.0), "{}", v.handle);
    }
    // Three of the four borders are drawn: 2D's is on a frozen layer, so it
    // is not among the sheet's paper entities at all.
    assert_eq!(sheet.entities, 3);

    // In the picture. Each frame is 60 x 40 paper units around its centre
    // and shows the model window 30 x 20 around model (50,25) at scale 2,
    // so the model line (0,0)-(100,50) -- which passes through (50,25) --
    // crosses each composited frame's centre and leaves through the right
    // edge (at the frame's top edge y the line is already at paper x =
    // centre.x + 40, outside the 30-unit half-width).
    let image = Sheet::read(&tmp.0, sheet);
    for (handle, cx, cy, composited) in [
        ("2A", 50.0, 50.0, true),
        ("2D", 50.0, 120.0, true),
        ("2E", 50.0, 190.0, false),
        ("2F", 140.0, 50.0, false),
    ] {
        assert_eq!(
            image.inked_near(cx, cy, 2),
            composited,
            "{handle}: ink at the frame centre ({cx}, {cy})"
        );
    }
    // The borders: along each frame's top edge, clear of the composited
    // line, 2A/2E/2F are dashed rectangles and 2D is nothing at all.
    for (handle, cx, cy, border) in [
        ("2A", 50.0, 70.0, true),
        ("2D", 50.0, 140.0, false),
        ("2E", 50.0, 210.0, true),
        ("2F", 140.0, 70.0, true),
    ] {
        let inked = image.inked_along(cy, cx - 25.0, cx + 25.0);
        assert_eq!(
            inked > 0,
            border,
            "{handle}: {inked} inked pixels along its top edge at y = {cy}"
        );
    }
}

#[test]
fn the_sheet_rectangle_follows_the_rotation_and_the_paper_unit() {
    // Layout1: A4 210 x 297 mm, plot rotation 2 (upside down, so the size
    // is not turned), margins (10, 20, 5, 15) mm, no plot origin. The
    // layout origin is the printable corner, so the sheet runs from
    // (-10, -20) to (210 - 10, 297 - 20) = (200, 277) mm.
    // Layout2: ANSI B 431.8 x 279.4 mm, rotation 3 (90 degrees clockwise:
    // the landscape sheet becomes portrait, 279.4 x 431.8) in inches with
    // 6.35 mm margins all round, so the sheet runs from (-0.25, -0.25) to
    // ((279.4 - 6.35) / 25.4, (431.8 - 6.35) / 25.4) = (10.75, 16.75) in.
    let db = uncad::parse(VIEWPORT_STATES).expect("fixture must parse");
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for (name, units, rotation, want) in [
        ("Layout1", 1u16, 2u16, (-10.0, -20.0, 200.0, 277.0)),
        ("Layout2", 0, 3, (-0.25, -0.25, 10.75, 16.75)),
    ] {
        let layout = &db.tables.layouts[name];
        assert_eq!(
            (layout.plot.paper_units, layout.plot.rotation),
            (units, rotation)
        );
        let rect = layout.plot.sheet_rect().expect("a paper size");
        assert!(
            close(rect.min_x, want.0)
                && close(rect.min_y, want.1)
                && close(rect.max_x, want.2)
                && close(rect.max_y, want.3),
            "{name}: {rect:?} vs {want:?}"
        );
        // The limits AutoCAD would have stored say the same thing.
        assert!(
            close(layout.limmin.x, want.0)
                && close(layout.limmin.y, want.1)
                && close(layout.limmax.x, want.2)
                && close(layout.limmax.y, want.3),
            "{name}: {:?} {:?}",
            layout.limmin,
            layout.limmax
        );
    }

    // And the export takes the limits, reporting the unit each layout is
    // set up in.
    let (_tmp, sheets) = export("rects");
    for (sheet, unit, want) in [
        (&sheets[0], "mm", (-10.0, -20.0, 200.0, 277.0)),
        (&sheets[1], "in", (-0.25, -0.25, 10.75, 16.75)),
    ] {
        assert_eq!(sheet.units, unit);
        assert_eq!(sheet.rect_source, "layout_limits");
        assert!(
            close(sheet.rect.min_x, want.0)
                && close(sheet.rect.min_y, want.1)
                && close(sheet.rect.max_x, want.2)
                && close(sheet.rect.max_y, want.3),
            "{}: {:?}",
            sheet.name,
            sheet.rect
        );
    }
    // The empty paper block still becomes a sheet, sized by its limits.
    assert_eq!((sheets[1].entities, sheets[1].viewports.len()), (0, 0));
}

#[test]
fn sheet_rect_turns_the_paper_and_converts_the_unit() {
    // ANSI A portrait, 215.9 x 279.4 mm, margins left 6.35, bottom 12.7,
    // right 3.175, top 19.05 mm, no plot origin. Only the left and bottom
    // margins place the sheet: it runs from (-left, -bottom) to that plus
    // the physical size, turned for rotation 1 and 3 (90 degrees either
    // way) and left alone for 0 and 2.
    let page = |rotation: u16, paper_units: u16, origin: (f64, f64)| PlotSettings {
        paper_width_mm: 215.9,
        paper_height_mm: 279.4,
        margins_mm: [6.35, 12.7, 3.175, 19.05],
        plot_origin: Point2D {
            x: origin.0,
            y: origin.1,
        },
        paper_units,
        rotation,
        ..PlotSettings::default()
    };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let check = |got: uncad::Rect, want: (f64, f64, f64, f64), what: &str| {
        assert!(
            close(got.min_x, want.0)
                && close(got.min_y, want.1)
                && close(got.max_x, want.2)
                && close(got.max_y, want.3),
            "{what}: {got:?} vs {want:?}"
        );
    };
    // Portrait: 215.9 - 6.35 = 209.55, 279.4 - 12.7 = 266.7.
    let portrait = (-6.35, -12.7, 209.55, 266.7);
    // Landscape: the size swaps, the margins do not. 279.4 - 6.35 = 273.05,
    // 215.9 - 12.7 = 203.2.
    let landscape = (-6.35, -12.7, 273.05, 203.2);
    for (rotation, want) in [(0, portrait), (1, landscape), (2, portrait), (3, landscape)] {
        let rect = page(rotation, 1, (0.0, 0.0))
            .sheet_rect()
            .expect("a paper size");
        check(rect, want, &format!("rotation {rotation}"));
    }
    // The same page in inches: every length over 25.4. 6.35 / 25.4 = 0.25,
    // 12.7 / 25.4 = 0.5, 209.55 / 25.4 = 8.25, 266.7 / 25.4 = 10.5.
    let inches = page(0, 0, (0.0, 0.0)).sheet_rect().expect("a paper size");
    check(inches, (-0.25, -0.5, 8.25, 10.5), "inches");
    // Paper unit 2 (pixels) is treated as millimetres.
    check(
        page(0, 2, (0.0, 0.0)).sheet_rect().expect("a paper size"),
        portrait,
        "pixels",
    );
    // The usual page setup "plot origin = minus the margins" puts the
    // paper's own corner at the layout origin.
    check(
        page(0, 1, (-6.35, -12.7))
            .sheet_rect()
            .expect("a paper size"),
        (0.0, 0.0, 215.9, 279.4),
        "origin cancels the margins",
    );
    // No page setup at all: no rectangle.
    assert_eq!(PlotSettings::default().sheet_rect(), None);
    assert_eq!(
        PlotSettings {
            paper_width_mm: 210.0,
            ..PlotSettings::default()
        }
        .sheet_rect(),
        None,
        "a height of 0 is no paper"
    );
}

/// The twisted-viewport fixture, whose one viewport shows the model at
/// scale 2 twisted 30 degrees -- the frame an infinite line in model space
/// has to be clipped in.
const TWISTED_VIEWPORT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/twisted_viewport_r2000.dxf"
);

/// A RAY or an XLINE in model space is clipped through the viewport's own
/// matrix, in the *sheet's* frame -- the one the model fragment is finally
/// written in. Clip it in the model's frame instead and the segment is cut
/// at the wrong place: too short (nothing where the viewport shows the
/// line) or too long (the coordinate that overflows the rasterizer is back).
/// This sheet's scale is low enough that the pre-0.3.0 1e6-unit segment
/// survived it; what the test pins is the frame, and that the clip keeps
/// the line inside the viewport it belongs to.
///
/// The fixture carries no construction line, so one is added to the parsed
/// model: to `entities` and to the `*Model_Space` block record, which is
/// what the renderer selects model space by.
#[test]
fn an_infinite_line_in_model_space_is_drawn_inside_the_viewport_and_nowhere_else() {
    use uncad::model::{Entity, EntityCommon, Point3D, RayEntity};

    let ink = |db: &uncad::CadDatabase, name: &str| -> (usize, usize) {
        let tmp = TempDir::new(name);
        let report = export_package(
            db,
            &tmp.0,
            &ExportOptions {
                max_levels: 0,
                ..Default::default()
            },
        )
        .expect("exports");
        let sheet = Sheet::read(&tmp.0, report.sheets.first().expect("one sheet"));
        // The viewport's frame is 200 x 120 paper units about (150, 100).
        // Its own dashed border, and the model line that starts exactly on
        // it, live in a two-unit band that counts as neither side.
        let (inside, outside) = (0..sheet.height)
            .flat_map(|y| (0..sheet.width).map(move |x| (x, y)))
            .filter(|&(x, y)| sheet.dark[y * sheet.width + x])
            .fold((0, 0), |(inside, outside), (x, y)| {
                let wx = sheet.min_x + x as f64 / sheet.ppu;
                let wy = sheet.max_y - y as f64 / sheet.ppu;
                if (52.0..=248.0).contains(&wx) && (42.0..=158.0).contains(&wy) {
                    (inside + 1, outside)
                } else if (48.0..=252.0).contains(&wx) && (38.0..=162.0).contains(&wy) {
                    (inside, outside)
                } else {
                    (inside, outside + 1)
                }
            });
        (inside, outside)
    };

    let plain = uncad::parse(TWISTED_VIEWPORT).expect("fixture must parse");
    let mut with_lines = uncad::parse(TWISTED_VIEWPORT).expect("fixture must parse");
    // Through the middle of the model window (view centre (50, 25)).
    for (handle, vector) in [
        (
            "2F",
            Point3D {
                x: 1.0,
                y: 0.5,
                z: 0.0,
            },
        ),
        (
            "3F",
            Point3D {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ),
    ] {
        let line = RayEntity {
            common: EntityCommon {
                handle: handle.to_string(),
                layer: "0".to_string(),
                color_index: 256,
                ..EntityCommon::default()
            },
            point: Point3D {
                x: 50.0,
                y: 25.0,
                z: 0.0,
            },
            vector,
        };
        let entity = if handle == "2F" {
            Entity::XLine(line)
        } else {
            Entity::Ray(line)
        };
        with_lines
            .tables
            .block_records
            .get_mut("*Model_Space")
            .expect("the model block")
            .entities
            .push(entity.clone());
        with_lines.entities.push(entity);
    }

    let (inside_plain, outside_plain) = ink(&plain, "no_construction_lines");
    let (inside_lines, outside_lines) = ink(&with_lines, "construction_lines");
    assert!(
        inside_lines > inside_plain + 100,
        "the construction lines drew almost nothing inside the viewport: \
         {inside_plain} -> {inside_lines} pixels"
    );
    assert_eq!(
        outside_plain, outside_lines,
        "an infinite line escaped the viewport frame"
    );
}
