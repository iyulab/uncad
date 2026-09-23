//! Paper layouts: the LAYOUT objects with their plot settings, and the
//! viewports on their sheets. Sources: the corpus's `example_2000.dwg` and
//! its text twin (two Letter layouts, each with only its overall viewport,
//! and the model tab set up in inches), and this project's
//! `twisted_viewport_r2000.dxf`, `plot_origin_r2000.dxf` and
//! `viewport_states_r2000.dxf`, whose page setups `tests/fixtures/README.md`
//! lists group by group. Where a sheet lies on its paper, and what a
//! viewport maps where, are consumers' derivations and not asserted here.

use uncad::model::{Entity, Point2D, Ref};
use uncad::tables::{LayoutRecord, PlotPaperUnits, PlotRotation};

const EXAMPLE_2000: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000"
);

macro_rules! fixture {
    ($name:literal) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $name)
    };
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

fn point(x: f64, y: f64) -> Point2D {
    Point2D { x, y }
}

fn near_point(a: Point2D, b: Point2D) -> bool {
    near(a.x, b.x) && near(a.y, b.y)
}

/// `[left, bottom, right, top]`.
fn margins(l: &LayoutRecord) -> [f64; 4] {
    let p = &l.plot_settings;
    [p.margin_left, p.margin_bottom, p.margin_right, p.margin_top]
}

#[test]
fn autocad_layouts_carry_their_plot_settings_in_both_formats() {
    for extension in ["dwg", "dxf"] {
        let path = format!("{EXAMPLE_2000}.{extension}");
        let db = uncad::parse(&path).expect("corpus file must parse");
        let layouts = &db.tables.layouts;
        assert_eq!(
            layouts.keys().cloned().collect::<Vec<_>>(),
            ["Layout1", "Layout2", "Model"],
            "{path}"
        );

        let model = &layouts["Model"];
        assert_eq!(model.tab_order, 0);
        assert_eq!(model.block_name, Ref::Resolved("*Model_Space".to_string()));
        let m = &model.plot_settings;
        assert_eq!(m.paper_name, "ANSI_A_(8.50_x_11.00_Inches)");
        assert_eq!(m.paper_units, Some(PlotPaperUnits::Inches), "{path}");
        // A custom scale of 1 : 78.436...
        assert!(
            near(m.scale_numerator / m.scale_denominator, 0.012749),
            "{path}"
        );

        for (name, tab, block) in [
            ("Layout1", 1, "*Paper_Space0"),
            ("Layout2", 2, "*Paper_Space"),
        ] {
            let l = &layouts[name];
            assert_eq!(l.tab_order, tab, "{path} {name}");
            assert_eq!(l.block_name, Ref::Resolved(block.to_string()), "{path}");
            let p = &l.plot_settings;
            assert_eq!(p.paper_name, "Letter_(8.50_x_11.00_Inches)", "{path}");
            assert!(near(p.paper_width, 215.9) && near(p.paper_height, 279.4));
            assert!(margins(l).iter().all(|m| near(*m, 6.35)), "{path}");
            assert_eq!(
                (p.paper_units, p.rotation),
                (
                    Some(PlotPaperUnits::Millimeters),
                    Some(PlotRotation::Counterclockwise90)
                ),
                "{path}"
            );
            assert_eq!((p.scale_numerator, p.scale_denominator), (1.0, 1.0));
            // Landscape Letter with the printable corner at the origin: the
            // limits AutoCAD stored for the layout.
            assert!(near_point(l.limits_min, point(-6.35, -6.35)), "{path}");
            assert!(near_point(l.limits_max, point(273.05, 209.55)), "{path}");
        }

        // The two layouts' overall viewports, in plan. The DWG says both are
        // on and states no number (the binary format stores none); the DXF
        // AutoCAD wrote states 68 = 0 and 69 = 0 for the one of the layout
        // that is not current, and 68 = 1, 69 = 1 for the other -- what each
        // file states.
        let mut viewports: Vec<(String, Option<bool>, Option<i32>)> = db
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Viewport(v) => {
                    let view = v.view.expect("an R2000 viewport states its view");
                    assert_eq!(view.direction.z, 1.0, "{path}");
                    let Ref::Resolved(h) = &v.common.source_handle else {
                        panic!("{v:?}");
                    };
                    Some((h.clone(), v.on, v.viewport_id))
                }
                _ => None,
            })
            .collect();
        viewports.sort();
        let expected = if extension == "dwg" {
            [
                ("84".to_string(), Some(true), None),
                ("88".to_string(), Some(true), None),
            ]
        } else {
            [
                ("84".to_string(), Some(false), Some(0)),
                ("88".to_string(), Some(true), Some(1)),
            ]
        };
        assert_eq!(viewports, expected, "{path}");
    }
}

#[test]
fn the_twisted_viewport_fixture_s_layout_is_an_a4_sheet_turned_landscape() {
    let db = uncad::parse(fixture!("twisted_viewport_r2000.dxf")).expect("fixture parses");
    assert_eq!(db.tables.layouts.len(), 1);
    let l = &db.tables.layouts["Layout1"];
    assert_eq!(l.tab_order, 1);
    assert_eq!(l.block_name, Ref::Resolved("*Paper_Space".to_string()));
    let p = &l.plot_settings;
    assert_eq!(p.paper_name, "ISO_A4_(210.00_x_297.00_MM)");
    assert_eq!((p.paper_width, p.paper_height), (210.0, 297.0));
    assert_eq!(margins(l), [6.35; 4]);
    assert_eq!(p.plot_origin, point(0.0, 0.0));
    assert_eq!(
        (p.paper_units, p.rotation),
        (
            Some(PlotPaperUnits::Millimeters),
            Some(PlotRotation::Counterclockwise90)
        )
    );
    assert_eq!(
        (l.limits_min, l.limits_max),
        (point(-6.35, -6.35), point(290.65, 203.65))
    );
}

/// An inch layout with asymmetric margins and a plot origin (DXF 46/47):
/// every value as the file states it, millimetres whatever the paper unit.
#[test]
fn the_plot_origin_fixture_s_page_setup_reads_as_stated() {
    let db = uncad::parse(fixture!("plot_origin_r2000.dxf")).expect("fixture parses");
    let l = &db.tables.layouts["Layout1"];
    let p = &l.plot_settings;
    assert_eq!(p.paper_name, "ANSI_B_(17.00_x_11.00_Inches)");
    assert_eq!((p.paper_width, p.paper_height), (431.8, 279.4));
    assert_eq!(margins(l), [6.35, 19.05, 6.35, 19.05]);
    assert_eq!(p.plot_origin, point(-6.35, -12.7));
    assert_eq!(
        (p.paper_units, p.rotation),
        (Some(PlotPaperUnits::Inches), Some(PlotRotation::Unrotated))
    );
    // AutoCAD's own placement of the sheet, which the file stores.
    assert_eq!(
        (l.limits_min, l.limits_max),
        (point(0.0, -0.25), point(17.0, 10.75))
    );
}

/// Two page setups no other fixture has: upside down in millimetres with
/// asymmetric margins, and a quarter turn clockwise in inches.
#[test]
fn the_viewport_states_fixture_s_two_layouts_read_as_stated() {
    let db = uncad::parse(fixture!("viewport_states_r2000.dxf")).expect("fixture parses");
    let layouts = &db.tables.layouts;
    assert_eq!(layouts.len(), 2);

    let first = &layouts["Layout1"];
    assert_eq!(first.block_name, Ref::Resolved("*Paper_Space".to_string()));
    assert_eq!(margins(first), [10.0, 20.0, 5.0, 15.0]);
    assert_eq!(
        (
            first.plot_settings.paper_units,
            first.plot_settings.rotation
        ),
        (
            Some(PlotPaperUnits::Millimeters),
            Some(PlotRotation::UpsideDown)
        )
    );
    assert_eq!(
        (first.limits_min, first.limits_max),
        (point(-10.0, -20.0), point(200.0, 277.0))
    );

    let second = &layouts["Layout2"];
    assert_eq!(second.tab_order, 2);
    assert_eq!(
        second.block_name,
        Ref::Resolved("*Paper_Space0".to_string())
    );
    assert_eq!(
        (
            second.plot_settings.paper_width,
            second.plot_settings.paper_height
        ),
        (431.8, 279.4)
    );
    assert_eq!(
        (
            second.plot_settings.paper_units,
            second.plot_settings.rotation
        ),
        (
            Some(PlotPaperUnits::Inches),
            Some(PlotRotation::Clockwise90)
        )
    );
    assert_eq!(
        (second.limits_min, second.limits_max),
        (point(-0.25, -0.25), point(10.75, 16.75))
    );
}

/// R13 and R14 have no layouts, but a drawing an application with layouts
/// saved in that format keeps its LAYOUT objects, and they are read like any
/// other: the corpus's `example_r14.dwg` has the same three tabs as its
/// R2000 sibling.
#[test]
fn an_r14_drawing_saved_by_a_later_application_keeps_its_layouts() {
    let db = uncad::parse(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/example_r14.dwg"
    ))
    .expect("corpus file must parse");
    let tabs: Vec<(&str, i32, Ref<String>)> = db
        .tables
        .layouts
        .values()
        .map(|l| (l.name.as_str(), l.tab_order, l.block_name.clone()))
        .collect();
    assert_eq!(
        tabs,
        [
            ("Layout1", 1, Ref::Resolved("*PAPER_SPACE0".to_string())),
            ("Layout2", 2, Ref::Resolved("*PAPER_SPACE".to_string())),
            ("Model", 0, Ref::Resolved("*MODEL_SPACE".to_string())),
        ]
    );
}
