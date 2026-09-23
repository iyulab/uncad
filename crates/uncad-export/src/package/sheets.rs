//! Sheets: one image per paper layout, the model composited through its
//! viewports by the renderer's layout rendering, with what each viewport
//! shows, how a model point maps onto the sheet, and the sheet's own texts
//! -- the title and the title block -- as records on it.

use std::collections::{BTreeMap, BTreeSet};

use iron_render_cad::{limits::LimitReport, Background, Scene, SheetSource, ToSvgOptions};
use serde::Serialize;
use uncad_model::model::{Entity, EntityId, Point2D, Ref, ViewportEntity};
use uncad_model::tables::{LayoutRecord, PlotPaperUnits, PlotSettings};
use uncad_model::CadDatabase;

use super::output::to_rgb;
use super::records::{placed_texts, PlacedText, Rounder};
use super::{fit_overview, ExportError, ExportOptions, ImageInfo, STROKE_PX};
use crate::frame::{Rect, EMPTY_RECT};

/// A viewport on a sheet, as `sheets.json` lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetViewport {
    /// The VIEWPORT's reference ID in the model.
    pub id: u64,
    /// The file's handle for it, when it came from a file.
    pub handle: Option<String>,
    /// The number the file gives it (DXF 69); a DWG states none.
    pub viewport_id: Option<i32>,
    /// Whether it is on: a viewport the file does not say is off is on.
    pub on: bool,
    /// The sheet's own frame (the paper), never composited.
    pub overall: bool,
    /// Whether the model was drawn through it.
    pub composited: bool,
    /// The frame on the sheet, paper units.
    pub frame: Rect,
    /// Paper units per model unit.
    pub scale: Option<f64>,
    pub twist_deg: f64,
    /// World corners of the model window (lower-left, lower-right,
    /// upper-right, upper-left on the sheet), when composited.
    pub model_window: Option<[[f64; 2]; 4]>,
    /// Row-major `[a, b, c, d, e, f]` with `paper_x = a x + b y + c` and
    /// `paper_y = d x + e y + f` for a model point `(x, y)`, when
    /// composited: the map the renderer drew the model through.
    pub model_to_paper: Option<[f64; 6]>,
    pub frozen_layers: Vec<String>,
}

/// One paper layout: its sheet, its viewports and its image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetReport {
    pub name: String,
    pub tab_order: i32,
    pub block: String,
    /// `mm`, `in` or `px` -- the layout's paper unit.
    pub units: String,
    /// The plot settings, when the file has a LAYOUT for this sheet.
    pub plot: Option<PlotSettings>,
    /// The sheet rectangle in paper units and where it came from, in order
    /// of preference: `layout_limits` (the LAYOUT's LIMMIN/LIMMAX, AutoCAD's
    /// own placement of the paper), `paper_size` (the paper its plot
    /// settings describe), `entities` (the paper entities' extents) or
    /// `empty`.
    pub rect: Rect,
    pub rect_source: String,
    pub overview: ImageInfo,
    pub viewports: Vec<SheetViewport>,
    /// The sheet's own entities the drawing shows (viewport frames
    /// included).
    pub entities: usize,
}

/// A paper layout to export.
struct SheetSpec {
    /// The key of `tables.layouts` the renderer knows it by.
    name: String,
    tab_order: i32,
    block: String,
    units: String,
    plot: Option<PlotSettings>,
}

/// The sheets written, their paper texts (each with the index of its
/// sheet), and every bound their renders engaged.
#[derive(Default)]
pub(crate) struct Sheets {
    pub(crate) reports: Vec<SheetReport>,
    pub(crate) texts: Vec<(PlacedText, usize)>,
    pub(crate) images: Vec<(String, Vec<u8>)>,
    pub(crate) limits: Vec<LimitReport>,
}

/// The most characters of a layout name a sheet directory keeps: well under
/// every per-component limit even after the `_99` dedup suffix, and under
/// what is left of Windows' 260-character path budget once the package
/// directory and `sheets/<name>/overview.png` are counted.
const MAX_SHEET_DIR: usize = 100;

/// The directory one sheet's image goes in, under `sheets/`: the layout's
/// name with every character outside `[A-Za-z0-9_-]` replaced by `_` (so
/// the path is portable), cut to [`MAX_SHEET_DIR`] characters, `sheet` when
/// nothing is left of it, and `_2`, `_3`, ... in tab order when an earlier
/// layout already took the name -- every Hangul syllable sanitises to `_`,
/// so two three-syllable Korean names both become `___`.
fn sheet_dir(name: &str, used: &mut BTreeSet<String>) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        // Sanitising leaves pure ASCII, so this cuts characters and bytes
        // alike and can never split one.
        .take(MAX_SHEET_DIR)
        .collect();
    let base = if safe.is_empty() {
        "sheet".to_string()
    } else {
        safe
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

/// The unit a layout's paper is set up in.
fn paper_units(plot: &PlotSettings) -> &'static str {
    match plot.paper_units {
        Some(PlotPaperUnits::Inches) => "in",
        Some(PlotPaperUnits::Pixels) => "px",
        _ => "mm",
    }
}

/// The paper layouts of `db`, in tab order: the LAYOUT objects whose block
/// the model holds -- or, for a file without them (R13/R14, a DXF without
/// an OBJECTS section), every paper-space block with entities, which the
/// renderer can only draw as a layout: `db` is then returned with a
/// layout stated for each, one that states no sheet, so it is framed on its
/// paper entities.
fn sheet_specs(db: &CadDatabase, unit: &str) -> (Vec<SheetSpec>, Option<CadDatabase>) {
    let mut specs: Vec<SheetSpec> = db
        .tables
        .layouts
        .iter()
        .filter(|(_, l)| {
            l.tab_order > 0
                && l.block_name
                    .resolved()
                    .is_some_and(|b| db.tables.block_records.contains_key(b))
        })
        .map(|(key, l)| SheetSpec {
            name: key.clone(),
            tab_order: l.tab_order,
            block: l.block_name.name().to_string(),
            units: paper_units(&l.plot_settings).to_string(),
            plot: Some(l.plot_settings.clone()),
        })
        .collect();
    let mut synthetic = None;
    if specs.is_empty() {
        let mut tab = 1;
        for (name, block) in &db.tables.block_records {
            if name.to_uppercase().starts_with("*PAPER_SPACE") && !block.entities.is_empty() {
                specs.push(SheetSpec {
                    name: name.trim_start_matches('*').to_string(),
                    tab_order: tab,
                    block: name.clone(),
                    units: unit.to_string(),
                    plot: None,
                });
                tab += 1;
            }
        }
        if !specs.is_empty() {
            let mut stated = db.clone();
            for spec in &specs {
                stated.tables.layouts.insert(
                    spec.name.clone(),
                    LayoutRecord {
                        name: spec.name.clone(),
                        tab_order: spec.tab_order,
                        block_name: Ref::Resolved(spec.block.clone()),
                        limits_min: Point2D::default(),
                        limits_max: Point2D::default(),
                        plot_settings: PlotSettings {
                            paper_name: String::new(),
                            paper_width: 0.0,
                            paper_height: 0.0,
                            margin_left: 0.0,
                            margin_bottom: 0.0,
                            margin_right: 0.0,
                            margin_top: 0.0,
                            plot_origin: Point2D::default(),
                            paper_units: None,
                            rotation: None,
                            scale_numerator: 0.0,
                            scale_denominator: 0.0,
                        },
                    },
                );
            }
            synthetic = Some(stated);
        }
    }
    specs.sort_by(|a, b| a.tab_order.cmp(&b.tab_order).then(a.name.cmp(&b.name)));
    (specs, synthetic)
}

/// Draws every paper layout of `db` as its sheet and collects its texts.
pub(crate) fn export_sheets(
    db: &CadDatabase,
    unit: &str,
    options: &ExportOptions,
    cap_height: f64,
    rounder: &Rounder,
    warnings: &mut Vec<String>,
) -> Result<Sheets, ExportError> {
    let (specs, synthetic) = sheet_specs(db, unit);
    let db = synthetic.as_ref().unwrap_or(db);
    let fonts = &options.fonts;
    let mut out = Sheets {
        reports: Vec::new(),
        texts: Vec::new(),
        images: Vec::new(),
        limits: Vec::new(),
    };
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    // Two LAYOUTs of a malformed file can name one paper block; a record
    // file must not hold an id twice.
    let mut text_ids: BTreeSet<String> = BTreeSet::new();
    for spec in specs {
        let sheet = match Scene::layout(
            db,
            &spec.name,
            ToSvgOptions {
                crop: iron_render_cad::Crop::Everything,
                padding: 0.0,
                include_hidden: options.include_hidden,
                cap_height,
                ..ToSvgOptions::default()
            },
        ) {
            Ok(sheet) => sheet,
            Err(e) => {
                warnings.push(format!(
                    "UnusableSheet: layout {} ({}) cannot be drawn: {e}",
                    spec.name, spec.block
                ));
                continue;
            }
        };
        let (rect, rect_source) = match (sheet.sheet, sheet.crop.content) {
            (Some(SheetSource::Limits), _) => (Rect::from(sheet.view_box), "layout_limits"),
            (Some(SheetSource::PlotSettings), _) => (Rect::from(sheet.view_box), "paper_size"),
            (Some(_), _) => (Rect::from(sheet.view_box), "stated"),
            (None, Some(content)) => (Rect::from(content), "entities"),
            (None, None) => (EMPTY_RECT, "empty"),
        };
        // No paper size, no limits and nothing but point-like content (a
        // lone POINT, a zero-length LINE): there is no rectangle to fit,
        // and one unusable sheet is worth a warning, not the whole export.
        let usable = [rect.min_x, rect.min_y, rect.max_x, rect.max_y]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 1e15)
            && (rect.width() > 0.0 || rect.height() > 0.0);
        if !usable {
            warnings.push(format!(
                "UnusableSheet: layout {} ({}) has no paper size, no limits and no usable content ({rect_source}); its sheet is skipped",
                spec.name, spec.block
            ));
            continue;
        }
        let fit = fit_overview(&rect, &options.profile, Some(0.0));
        let png_path = format!("sheets/{}/overview.png", sheet_dir(&spec.name, &mut dirs));
        let overview = ImageInfo::new(
            &format!("sheet:{}", spec.name),
            &png_path,
            fit.rect,
            fit.ppu,
            fit.width,
            fit.height,
        );
        let bytes = to_rgb(&sheet.png(
            &overview.view(),
            STROKE_PX,
            fonts,
            Background::White,
            |_| true,
        )?)?;
        out.images.push((png_path, bytes));

        let block_entities: &[Entity] = db
            .tables
            .block_records
            .get(&spec.block)
            .map_or(&[], |b| b.entities.as_slice());
        let viewport_entities: BTreeMap<EntityId, &ViewportEntity> = block_entities
            .iter()
            .filter_map(|e| match e {
                Entity::Viewport(v) => Some((v.common.id, v)),
                _ => None,
            })
            .collect();
        let viewports: Vec<SheetViewport> = sheet
            .viewports
            .iter()
            .map(|r| {
                let vp = viewport_entities.get(&r.id);
                let view = vp.and_then(|v| v.view);
                let m = r.model_to_paper;
                SheetViewport {
                    id: r.id.value(),
                    handle: vp.and_then(|v| v.common.source_handle.resolved().cloned()),
                    viewport_id: vp.and_then(|v| v.viewport_id),
                    on: vp.is_none_or(|v| v.on != Some(false)),
                    overall: r.overall,
                    composited: m.is_some(),
                    frame: Rect::from(r.frame),
                    scale: match (m, vp, view) {
                        (Some(m), _, _) => Some(m.determinant().abs().sqrt()),
                        (None, Some(v), Some(view)) if view.height > 0.0 && v.height > 0.0 => {
                            Some(v.height / view.height)
                        }
                        _ => None,
                    },
                    twist_deg: match (m, view) {
                        (Some(m), _) => m.b.atan2(m.a).to_degrees(),
                        (None, Some(view)) => view.twist.to_degrees(),
                        (None, None) => 0.0,
                    },
                    model_window: r
                        .model_window
                        .map(|w| w.map(|p| [rounder.coord(p.x), rounder.coord(p.y)])),
                    model_to_paper: m.map(|m| [m.a, m.c, m.e, m.b, m.d, m.f]),
                    frozen_layers: vp.map_or(Vec::new(), |v| {
                        v.frozen_layers
                            .iter()
                            .map(|l| l.name().to_string())
                            .collect()
                    }),
                }
            })
            .collect();
        let entities = sheet
            .parts()
            .iter()
            .filter(|p| !p.through_viewport && p.hidden.is_none())
            .count();

        // The sheet's own texts: the ones whose path does not start at a
        // viewport (the model drawn through one starts there), placed on
        // the paper, measured through the same fonts the sheet image got.
        let viewport_ids: BTreeSet<EntityId> = sheet.viewports.iter().map(|v| v.id).collect();
        let boxes = sheet.text_boxes(fonts)?;
        let mut top: BTreeMap<EntityId, &Entity> =
            db.entities.iter().map(|e| (e.common().id, e)).collect();
        for e in block_entities {
            top.insert(e.common().id, e);
        }
        let sheet_index = out.reports.len();
        let texts = placed_texts(&db.tables, &top, &boxes, |id| !viewport_ids.contains(&id));
        out.texts.extend(
            texts
                .into_iter()
                .filter(|t| text_ids.insert(t.id.clone()))
                .map(|t| (t, sheet_index)),
        );
        out.limits.push(sheet.limits.clone());
        out.reports.push(SheetReport {
            name: spec.name,
            tab_order: spec.tab_order,
            block: spec.block,
            units: spec.units,
            plot: spec.plot,
            rect,
            rect_source: rect_source.to_string(),
            overview,
            viewports,
            entities,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_directories_are_unique() {
        let mut used = BTreeSet::new();
        assert_eq!(sheet_dir("\u{d3c9}\u{ba74}\u{b3c4}", &mut used), "___");
        assert_eq!(sheet_dir("\u{c785}\u{ba74}\u{b3c4}", &mut used), "____2");
        assert_eq!(sheet_dir("Layout 1", &mut used), "Layout_1");
        assert_eq!(sheet_dir("Layout_1", &mut used), "Layout_1_2");
        assert_eq!(sheet_dir("Layout-1", &mut used), "Layout-1");
        assert_eq!(sheet_dir("", &mut used), "sheet");
        assert_eq!(sheet_dir("", &mut used), "sheet_2");
        assert_eq!(used.len(), 7, "every name got its own directory");
    }

    #[test]
    fn a_sheet_directory_is_short_enough_for_the_filesystem() {
        // A layout name longer than the filesystem's 255-byte component
        // limit made the first sheet write fail, and the export with it.
        let mut used = BTreeSet::new();
        let long = "L".repeat(300);
        let first = sheet_dir(&long, &mut used);
        assert_eq!(first, "L".repeat(MAX_SHEET_DIR));
        let second = sheet_dir(&format!("{long}-other"), &mut used);
        assert_eq!(second, format!("{}_2", "L".repeat(MAX_SHEET_DIR)));
        for dir in [&first, &second] {
            assert!(dir.len() < 255, "{} bytes", dir.len());
            assert!(dir.is_ascii());
        }
        let exact = "N".repeat(MAX_SHEET_DIR);
        assert_eq!(sheet_dir(&exact, &mut used), exact);
    }
}
