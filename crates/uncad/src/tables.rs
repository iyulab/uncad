//! Conversion of the non-entity OBJECT-supertype records an entity or a
//! paper sheet resolves against: LAYER (with its on/frozen/locked/plot
//! state, lineweight and linetype name), BLOCK_RECORD, MLINESTYLE, DIMSTYLE,
//! and LAYOUT with its embedded plot settings.
//!
//! All of them are collected by one pass over the objects, dispatching on
//! `dwg_object_get_fixedtype`. The named object dictionary is never walked,
//! so a file whose LAYOUTs this pass does not see (pre-R2000, or a DXF with
//! no OBJECTS section) simply has none. There is no LTYPE or STYLE record
//! here: only the names layers and text entities carry survive.

use crate::convert::owned_entities;
use crate::dynapi::{get_array_field, get_field, get_sub_field, get_sub_utf8_field};
use crate::model::{Entity, Point2D, Point3D};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::c_void;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerRecord {
    pub name: String,
    /// Same raw semantics as `EntityCommon::color_index`: negative means
    /// "off", otherwise the layer's own ACI palette index. It should never be
    /// 0 or 256 in practice (BYLAYER/BYBLOCK are entity-level concepts a layer
    /// cannot resolve against itself), but LibreDWG's `bit_read_CMC` does hand
    /// back a raw `256` "no palette match" sentinel for some real files -- see
    /// [`resolve_layer_color_index`] for the recovery applied first.
    pub color_index: i16,
    /// Switched on. Off = the DWG's own bit, or a negative `color_index`
    /// (how a DXF says it). Since 0.3.0; the defaults below make 0.2.0 JSON
    /// load as "everything shown".
    #[serde(default = "yes")]
    pub on: bool,
    #[serde(default)]
    pub frozen: bool,
    /// Locked layers are still drawn; reported for completeness.
    #[serde(default)]
    pub locked: bool,
    /// The "plot this layer" flag. Read from R2000+ DWG files; a DXF's group
    /// 290 is optional and LibreDWG's reader leaves it indistinguishable
    /// from an absent one, so DXF layers (and R13/R14) always read `true`.
    /// `DEFPOINTS` is hidden by name regardless -- see
    /// [`crate::visibility::hidden_reason`].
    #[serde(default = "yes")]
    pub plot: bool,
    /// The layer's lineweight in millimetres; `None` for the default (and
    /// from R13/R14 files, which store none, or a DXF that omits group 370).
    #[serde(default)]
    pub lineweight_mm: Option<f64>,
    /// The layer's LTYPE name; empty when unresolvable.
    #[serde(default)]
    pub linetype: String,
}

fn yes() -> bool {
    true
}

impl Default for LayerRecord {
    /// An unnamed layer with colour 7, on, thawed, unlocked and plotting.
    fn default() -> Self {
        LayerRecord {
            name: String::new(),
            color_index: 7,
            on: true,
            frozen: false,
            locked: false,
            plot: true,
            lineweight_mm: None,
            linetype: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockRecord {
    pub name: String,
    /// Every entity directly owned by this block (via
    /// `get_first_owned_entity`/`get_next_owned_entity`), regardless of
    /// whether the block is `*Model_Space`/`*Paper_Space*` or a named
    /// block definition referenced by INSERT elsewhere -- unlike
    /// `CadDatabase::entities`, which only includes the former. This is
    /// what an INSERT's `block_name` resolves against to find what it
    /// actually draws.
    pub entities: Vec<Entity>,
}

/// A DIMSTYLE table entry: the variables that turn a measurement into the
/// label AutoCAD shows (`docs/VLM_EXPORT_DESIGN.md`, "Numeric exactness").
/// Every numeric field is as stored; 0 means the file never set it and the
/// header's value applies (see `crate::dimension::EffectiveStyle`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DimStyleRecord {
    pub name: String,
    /// Linear measurement factor: the label shows `measurement * dimlfac`.
    pub dimlfac: f64,
    /// Decimal places, or the `1/2^n` fraction precision for architectural
    /// and fractional units.
    pub dimdec: u16,
    /// 1 scientific, 2 decimal, 3 engineering, 4 architectural, 5 fractional,
    /// 6 Windows desktop.
    pub dimlunit: u16,
    /// Zero suppression bits (8 = drop trailing zeros; 0-3 select feet/inch
    /// zero handling for architectural units).
    pub dimzin: u16,
    /// Prefix/suffix, `<>` standing for the value.
    pub dimpost: String,
    pub dimrnd: f64,
    pub dimscale: f64,
    pub dimtxt: f64,
    pub dimasz: f64,
    /// Angular format: 0 decimal degrees, 1 deg/min/sec, 2 gradians,
    /// 3 radians, 4 surveyor.
    pub dimaunit: u16,
    pub dimadec: u16,
    /// Fraction style: 0 horizontal, 1 diagonal, 2 not stacked.
    pub dimfrac: u16,
}

/// A layout's plot settings (DXF `AcDbPlotSettings`): the sheet of paper a
/// layout is set up to print on. Widths are millimetres whatever
/// `paper_units` says (that is how the file stores them); `rotation` is
/// the DXF code (0 none, 1 = 90 degrees counter-clockwise, 2 = upside
/// down, 3 = 90 degrees clockwise), so the sheet is landscape for 1 and 3
/// of a portrait size. Since 0.3.0.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlotSettings {
    /// DXF 1, the page setup name (usually empty).
    pub page_setup: String,
    /// DXF 2, the printer or plot configuration (`none_device`, a `.pc3`).
    pub printer: String,
    /// DXF 4, the paper size name (`ISO_A4_(210.00_x_297.00_MM)`).
    pub paper_name: String,
    /// DXF 44/45, the physical (portrait) size in millimetres; 0 when no
    /// page setup exists.
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    /// DXF 40-43: left, bottom, right, top unprintable margins, millimetres.
    pub margins_mm: [f64; 4],
    /// DXF 46/47, millimetres.
    pub plot_origin: Point2D,
    /// DXF 72: 0 inches, 1 millimetres, 2 pixels -- the layout's paper unit.
    pub paper_units: u16,
    /// DXF 73, see above.
    pub rotation: u16,
    /// DXF 74: 0 last screen display, 1 extents, 2 limits, 3 view, 4
    /// window, 5 the layout.
    pub plot_type: u16,
    /// DXF 142 / 143: paper units per drawing unit of the custom scale.
    pub scale: f64,
    /// DXF 75 / 147: the standard scale code and its factor.
    pub std_scale_type: u16,
    pub std_scale_factor: f64,
    /// DXF 70.
    pub flags: u16,
    /// DXF 7, the plot style table (`.ctb`).
    pub style_sheet: String,
}

impl PlotSettings {
    /// One millimetre in the layout's paper units (25.4 mm to the inch;
    /// pixels are treated as millimetres).
    pub fn mm_to_paper(&self) -> f64 {
        if self.paper_units == 0 {
            1.0 / 25.4
        } else {
            1.0
        }
    }

    /// The sheet as it lies on the layout, in paper units, computed from
    /// the page setup: the layout origin is the printable area's lower-left
    /// corner moved by the plot origin (DXF 46/47), so the sheet runs from
    /// `(-(left + origin_x), -(bottom + origin_y))` to that plus the
    /// physical size turned by `rotation` -- ezdxf's `reset_paper_limits`
    /// rule, which is what AutoCAD writes into a paper layout's
    /// `LIMMIN`/`LIMMAX`. The common page setup "origin = minus the
    /// margins" therefore puts the paper's own corner at (0,0). `None`
    /// without a paper size.
    ///
    /// The export prefers the layout's stored limits
    /// ([`LayoutRecord::limmin`] / [`LayoutRecord::limmax`]) whenever they
    /// span a rectangle and uses this only as the fallback: the limits are
    /// AutoCAD's own placement and stay right where the offset's
    /// interaction with a rotated sheet is not modelled here (a rotation of
    /// 1 or 3 turns the size but keeps the left/bottom margins and the
    /// offset on the layout's own axes).
    pub fn sheet_rect(&self) -> Option<crate::crop::Rect> {
        if !(self.paper_width_mm > 0.0 && self.paper_height_mm > 0.0) {
            return None;
        }
        let k = self.mm_to_paper();
        let (w, h) = if self.rotation == 1 || self.rotation == 3 {
            (self.paper_height_mm, self.paper_width_mm)
        } else {
            (self.paper_width_mm, self.paper_height_mm)
        };
        let [left, bottom, _, _] = self.margins_mm;
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        let shift_x = left + finite(self.plot_origin.x);
        let shift_y = bottom + finite(self.plot_origin.y);
        Some(crate::crop::Rect::new(
            -shift_x * k,
            -shift_y * k,
            (w - shift_x) * k,
            (h - shift_y) * k,
        ))
    }
}

/// A LAYOUT: a model or paper tab, the block it draws and its plot
/// settings. Since 0.3.0.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LayoutRecord {
    pub name: String,
    /// DXF 71: 0 is the Model tab.
    pub tab_order: u16,
    /// DXF 70: 1 PSLTSCALE, 2 LIMCHECK.
    pub flags: u16,
    /// The `block_records` key this layout draws (`*Model_Space`,
    /// `*Paper_Space`, `*Paper_Space0`, ...); empty when unresolvable.
    pub block_name: String,
    /// DXF 10 / 11, the drawing limits in the layout's units.
    pub limmin: Point2D,
    pub limmax: Point2D,
    /// DXF 14 / 15; `None` while AutoCAD has never computed them (the file
    /// holds +-1e20).
    pub extmin: Option<Point3D>,
    pub extmax: Option<Point3D>,
    /// DXF 331, the handle of the viewport last active in this layout.
    pub active_viewport: Option<String>,
    pub plot: PlotSettings,
}

/// The maps are `BTreeMap`s, not `HashMap`s, so iteration -- and
/// therefore `to_json()`'s key order -- is deterministic: the same input
/// file serializes to the same bytes on every run and every machine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Tables {
    /// Layer name -> record. Deliberately does *not* expose a layer's
    /// `Dwg_Color.rgb`: only `color_index` is trustworthy for BYLAYER
    /// resolution (see `docs/CAVEATS.md` and [`crate::color`]).
    pub layers: BTreeMap<String, LayerRecord>,
    /// Block name -> record, every `BLOCK_HEADER` in the file (including
    /// `*Model_Space`/`*Paper_Space*`, which also show up flattened into
    /// `CadDatabase::entities` -- see that field's doc comment).
    pub block_records: BTreeMap<String, BlockRecord>,
    /// MLINESTYLE name -> each parallel line's `offset` (distance from the
    /// MLINE centerline), in LibreDWG's own storage order. There is no
    /// separate line-identity field, so array order is the only correspondence
    /// between a style's lines and an MLINE's vertices. Per-line color and
    /// linetype are not read: nothing renders them.
    pub mlinestyles: BTreeMap<String, Vec<f64>>,
    /// DIMSTYLE name -> record. Since 0.3.0 (serde default: an older JSON
    /// document loads with an empty map).
    #[serde(default)]
    pub dimstyles: BTreeMap<String, DimStyleRecord>,
    /// Layout name -> record, every LAYOUT object (the Model tab included).
    /// Files older than R2000 and DXF files without an OBJECTS section have
    /// none; the paper-space blocks are still in `block_records`. Since
    /// 0.3.0.
    #[serde(default)]
    pub layouts: BTreeMap<String, LayoutRecord>,
}

/// # Safety
/// `dwg` must be a successfully-`dwg_read_file`'d, not-yet-`dwg_free`'d
/// `Dwg_Data`, and the caller must hold `LIBREDWG_LOCK` (see lib.rs) for
/// the whole call -- this walks LibreDWG's non-reentrant C API directly.
///
/// `pub(crate)`, not `pub`: `parse()` is the only intended caller. The module
/// itself has to be public so `LayerRecord`/`BlockRecord` are nameable, but
/// exporting a `*mut Dwg_Data` entry point would bypass the lock and leak
/// `libredwg_sys` types into the public surface.
pub(crate) unsafe fn convert_tables(dwg: *mut libredwg_sys::Dwg_Data) -> Tables {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let source = unsafe { crate::header::source(dwg) };
    let mut layers = BTreeMap::new();
    let mut block_records = BTreeMap::new();
    let mut mlinestyles = BTreeMap::new();
    let mut dimstyles = BTreeMap::new();
    let mut layouts = BTreeMap::new();

    for i in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if obj.is_null() {
            continue;
        }
        // Cast needed for cross-platform bindgen enum-width consistency --
        // see convert.rs's comment on the same call.
        let fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) } as libredwg_sys::DWG_OBJECT_TYPE;

        if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LAYER {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(record) = convert_layer(dwg, object_ptr, source) {
                    layers.insert(record.name.clone(), record);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK_HEADER {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(name) = block_record_name(object_ptr) {
                    let entities = unsafe { owned_entities(dwg, obj) };
                    block_records.insert(name.clone(), BlockRecord { name, entities });
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LAYOUT {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(record) = unsafe { convert_layout(dwg, object_ptr) } {
                    layouts.insert(record.name.clone(), record);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINESTYLE {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some((name, offsets)) = convert_mlinestyle(object_ptr) {
                    mlinestyles.insert(name, offsets);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMSTYLE {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(record) = convert_dimstyle(object_ptr) {
                    dimstyles.insert(record.name.clone(), record);
                }
            }
        }
    }

    Tables {
        layers,
        block_records,
        mlinestyles,
        dimstyles,
        layouts,
    }
}

fn convert_dimstyle(object_ptr: *mut c_void) -> Option<DimStyleRecord> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "DIMSTYLE", "name")?;
    let f64_var = |field: &str| get_field::<f64>(object_ptr, "DIMSTYLE", field).unwrap_or(0.0);
    let u16_var = |field: &str| get_field::<u16>(object_ptr, "DIMSTYLE", field).unwrap_or(0);
    Some(DimStyleRecord {
        name,
        dimlfac: f64_var("DIMLFAC"),
        dimdec: u16_var("DIMDEC"),
        dimlunit: u16_var("DIMLUNIT"),
        dimzin: u16_var("DIMZIN"),
        dimpost: crate::dynapi::get_utf8_field(object_ptr, "DIMSTYLE", "DIMPOST")
            .unwrap_or_default(),
        dimrnd: f64_var("DIMRND"),
        dimscale: f64_var("DIMSCALE"),
        dimtxt: f64_var("DIMTXT"),
        dimasz: f64_var("DIMASZ"),
        dimaunit: u16_var("DIMAUNIT"),
        dimadec: u16_var("DIMADEC"),
        dimfrac: u16_var("DIMFRAC"),
    })
}

/// Resolves a block's real name.
///
/// `BLOCK_HEADER.name` is only the *abbreviated* name for anonymous blocks
/// (`"*D"` for every anonymous DIMENSION-geometry cache, `"*T"` for table
/// caches) -- the disambiguating name (`"*D30"`) lives on the block's own
/// `BLOCK` entity, reached through the `block_entity` handle field rather than
/// `get_first_owned_entity`, whose iteration protocol skips the BLOCK/ENDBLK
/// sentinels entirely. Without this, every anonymous block in a drawing
/// collapses onto one `"*D"` map key. Falls back to the abbreviated name if
/// there is no BLOCK entity.
fn block_record_name(block_header_object_ptr: *mut c_void) -> Option<String> {
    let abbreviated =
        crate::dynapi::get_utf8_field(block_header_object_ptr, "BLOCK_HEADER", "name");

    if let Some(block_ref) = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
        block_header_object_ptr,
        "BLOCK_HEADER",
        "block_entity",
    ) {
        if !block_ref.is_null() {
            // SAFETY: Dwg_Object_Ref is a plain, non-opaque FFI struct
            // (see libredwg-sys build.rs); block_ref is non-null per the
            // check above and was populated by dwg_read_file.
            let block_obj = unsafe { (*block_ref).obj };
            if !block_obj.is_null() {
                // Dwg_Object_Ref.obj's declared type is a second,
                // independently-generated bindgen item for the same
                // `_dwg_object` C tag that Dwg_Object is opaqued to elsewhere.
                // Identical layout, so the pointer cast is sound.
                let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(block_obj.cast()) };
                if let Some(full_name) = crate::dynapi::get_utf8_field(entity_ptr, "BLOCK", "name")
                {
                    if !full_name.is_empty() {
                        return Some(full_name);
                    }
                }
            }
        }
    }

    abbreviated
}

/// Resolves a `BITCODE_H` handle to a `BLOCK_HEADER` (an INSERT's
/// `block_header`, a DIMENSION's `block`) to that block's disambiguated name --
/// the same resolution `convert_tables` uses to key `block_records`, so a
/// caller can look the result up there.
///
/// **Not** interchangeable with [`crate::dynapi::resolve_handle_name`], which
/// returns `BLOCK_HEADER.name` directly. Using that here was a real bug: every
/// anonymous dimension cache resolved to `"*D"`, which is never a key in
/// `block_records`, so no DIMENSION rendered at all.
pub(crate) fn resolve_block_name(
    block_header_ref: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<String> {
    if block_header_ref.is_null() {
        return None;
    }
    // SAFETY: same contract as block_record_name's use of Dwg_Object_Ref.
    let block_header_obj = unsafe { (*block_header_ref).obj };
    if block_header_obj.is_null() {
        return None;
    }
    let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(block_header_obj.cast()) };
    if object_ptr.is_null() {
        return None;
    }
    block_record_name(object_ptr)
}

/// Reads an MLINESTYLE object's name and each of its parallel lines' `offset`,
/// in array order -- see [`Tables::mlinestyles`].
fn convert_mlinestyle(object_ptr: *mut c_void) -> Option<(String, Vec<f64>)> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "MLINESTYLE", "name")?;
    // num_lines is BITCODE_RC (one unsigned byte), unlike num_paths'
    // BITCODE_BL -- see get_array_field on why the count width cannot be
    // hardcoded for every caller.
    let lines: Vec<libredwg_sys::Dwg_MLINESTYLE_line> =
        get_array_field::<u8, _>(object_ptr, "MLINESTYLE", "num_lines", "lines");
    let offsets = lines.iter().map(|l| l.offset).collect();
    Some((name, offsets))
}

fn convert_layer(
    dwg: *mut libredwg_sys::Dwg_Data,
    object_ptr: *mut c_void,
    source: crate::header::Source,
) -> Option<LayerRecord> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "LAYER", "name")?;
    let color = get_field::<libredwg_sys::Dwg_Color>(object_ptr, "LAYER", "color")?;
    let color_index = resolve_layer_color_index(color.index, color.method, color.rgb);
    let bit = |field: &str| get_field::<u8>(object_ptr, "LAYER", field).unwrap_or(0) != 0;
    // R2000+ DWG: the decoder splits `flag0` into these bits, plot flag
    // included. R13/R14 store the first four as bits of their own and no
    // plot flag. LibreDWG's DXF reader applies the DWG bit layout to group
    // 70 (bit 2 -> off, bit 4 -> frozen_in_new, bit 8 -> locked), so on
    // that path the DXF layout is read from the raw flag instead: 1 frozen,
    // 2 frozen in new viewports, 4 locked; "off" is a negative group 62,
    // and an absent 290 cannot be told from a 0.
    let (on, frozen, locked) = if source.from_dxf {
        let flag = get_field::<u8>(object_ptr, "LAYER", "flag").unwrap_or(0);
        (color.index >= 0, flag & 1 != 0, flag & 4 != 0)
    } else {
        (
            !bit("off") && color.index >= 0,
            bit("frozen"),
            bit("locked"),
        )
    };
    let plot = if source.r2000_plus && !source.from_dxf {
        bit("plotflag")
    } else {
        true
    };
    // The lineweight code is only meaningful from R2000 on; a DXF layer
    // without group 370 reads as code 0 (0.00 mm), which is left unknown.
    let lineweight_mm = get_field::<u8>(object_ptr, "LAYER", "linewt")
        .filter(|code| source.r2000_plus && !(source.from_dxf && *code == 0))
        .and_then(crate::visibility::lineweight_mm);
    let linetype = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYER", "ltype")
        .filter(|h| !h.is_null())
        .and_then(|h| crate::dynapi::resolve_handle_name(dwg, h))
        .unwrap_or_default();
    Some(LayerRecord {
        name,
        color_index,
        on,
        frozen,
        locked,
        plot,
        lineweight_mm,
        linetype,
    })
}

/// Reads a LAYOUT object and its embedded plot settings.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` owning `object_ptr`, a LAYOUT's
/// type-specific struct pointer.
unsafe fn convert_layout(
    dwg: *mut libredwg_sys::Dwg_Data,
    object_ptr: *mut c_void,
) -> Option<LayoutRecord> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "LAYOUT", "layout_name")?;
    let block_name =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYOUT", "block_header")
            .and_then(resolve_block_name)
            .unwrap_or_default();
    let active_viewport =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYOUT", "active_viewport")
            .filter(|h| !h.is_null())
            .map(|h| {
                // SAFETY: a live Dwg_Object_Ref of this Dwg_Data.
                let absolute = unsafe { (*h).absolute_ref };
                format!("{absolute:X}")
            })
            .filter(|h| h != "0");
    let ext = |field: &str| {
        get_field::<Point3D>(object_ptr, "LAYOUT", field).filter(|p| {
            [p.x, p.y, p.z]
                .iter()
                .all(|v| v.is_finite() && v.abs() < 1e20)
        })
    };
    let sub = |field: &str| {
        get_sub_field::<f64>(object_ptr, "LAYOUT", "plotsettings", "PLOTSETTINGS", field)
            .unwrap_or(0.0)
    };
    let sub_u16 = |field: &str| {
        get_sub_field::<u16>(object_ptr, "LAYOUT", "plotsettings", "PLOTSETTINGS", field)
            .unwrap_or(0)
    };
    let sub_text = |field: &str| {
        // SAFETY: dwg owns object_ptr (this function's contract).
        unsafe {
            get_sub_utf8_field(
                dwg,
                object_ptr,
                "LAYOUT",
                "plotsettings",
                "PLOTSETTINGS",
                field,
            )
        }
        .unwrap_or_default()
    };
    let (paper_units_num, drawing_units) = (sub("paper_units"), sub("drawing_units"));
    let plot = PlotSettings {
        page_setup: sub_text("printer_cfg_file"),
        printer: sub_text("paper_size"),
        paper_name: sub_text("canonical_media_name"),
        paper_width_mm: sub("paper_width"),
        paper_height_mm: sub("paper_height"),
        margins_mm: [
            sub("left_margin"),
            sub("bottom_margin"),
            sub("right_margin"),
            sub("top_margin"),
        ],
        plot_origin: get_sub_field::<Point2D>(
            object_ptr,
            "LAYOUT",
            "plotsettings",
            "PLOTSETTINGS",
            "plot_origin",
        )
        .unwrap_or_default(),
        paper_units: sub_u16("plot_paper_unit"),
        rotation: sub_u16("plot_rotation_mode"),
        plot_type: sub_u16("plot_type"),
        scale: if drawing_units > 0.0 && paper_units_num > 0.0 {
            paper_units_num / drawing_units
        } else {
            1.0
        },
        std_scale_type: sub_u16("std_scale_type"),
        std_scale_factor: sub("std_scale_factor"),
        flags: sub_u16("plot_flags"),
        style_sheet: sub_text("stylesheet"),
    };
    Some(LayoutRecord {
        name,
        tab_order: get_field::<u16>(object_ptr, "LAYOUT", "tab_order").unwrap_or(0),
        flags: get_field::<u16>(object_ptr, "LAYOUT", "layout_flags").unwrap_or(0),
        block_name,
        limmin: get_field::<Point2D>(object_ptr, "LAYOUT", "LIMMIN").unwrap_or_default(),
        limmax: get_field::<Point2D>(object_ptr, "LAYOUT", "LIMMAX").unwrap_or_default(),
        extmin: ext("EXTMIN"),
        extmax: ext("EXTMAX"),
        active_viewport,
        plot,
    })
}

/// Recovers a usable ACI index from a LAYER's raw `Dwg_Color` when LibreDWG
/// could not.
///
/// `bit_read_CMC` sets `index` via `dwg_find_color_index(rgb)`, which returns
/// `256` ("no exact palette match" -- not a real BYLAYER sentinel, since a
/// layer cannot be BYLAYER against itself) whenever a TRUECOLOR-tagged layer's
/// `rgb` is not bit-identical to one of the 256 palette entries. Real drawings
/// hit exactly that case while storing the *intended* ACI index in `rgb`'s low
/// byte rather than a genuine 24-bit color: layer `"0"` reads as
/// `rgb = 0x..000007`, matching AutoCAD's real default of ACI 7. LibreDWG's own
/// `bit_downconvert_CMC` already carries the identical `if index == 256 { index
/// = rgb & 0xff }` fallback on its (differently gated) path; this mirrors it
/// for the plain-read path, which lacks it.
///
/// **Known limitation**, inherited from that same fallback rather than
/// introduced here: a layer with a genuine arbitrary truecolor that happens not
/// to match any palette entry also reports `index = 256`, and its blue channel
/// is reinterpreted as an ACI index. From the data available here the two cases
/// are indistinguishable. See `docs/CAVEATS.md`.
fn resolve_layer_color_index(index: i16, method: libredwg_sys::Dwg_Color_Method, rgb: u32) -> i16 {
    if index == 256 && method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR {
        (rgb & 0xff) as i16
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_with_no_palette_match_falls_back_to_rgbs_low_byte() {
        let resolved = resolve_layer_color_index(
            256,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
            0xc3000007,
        );
        assert_eq!(resolved, 7);
    }

    #[test]
    fn non_256_index_passes_through_unchanged_regardless_of_method() {
        let resolved = resolve_layer_color_index(
            3,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
            0xc3000007,
        );
        assert_eq!(resolved, 3);
    }

    #[test]
    fn index_256_with_non_truecolor_method_is_left_alone() {
        // 256 only means "no ACI palette match" when it came from a
        // TRUECOLOR-tagged rgb lookup -- for any other method it's just
        // whatever LibreDWG actually read, not this fallback's business.
        let resolved =
            resolve_layer_color_index(256, libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI, 0);
        assert_eq!(resolved, 256);
    }

    #[test]
    fn the_sheet_rect_moves_the_paper_by_the_plot_origin() {
        // AutoCAD's usual "origin at the paper corner" page setup: an ARCH D
        // sheet (36 x 24 in = 914.4 x 609.6 mm) in inches, margins
        // (0.25, 0.75, 0.25, 0.75) in and a plot origin of exactly minus
        // the margins, so the layout origin is the paper's corner and the
        // sheet is (0,0)..(36,24) -- the LIMMIN/LIMMAX AutoCAD stores for
        // that setup (samples/AutoCADSamples2.dwg, Layout1).
        let mut p = PlotSettings {
            paper_width_mm: 914.4,
            paper_height_mm: 609.6,
            margins_mm: [6.35, 19.05, 6.35, 19.05],
            plot_origin: Point2D {
                x: -6.35,
                y: -19.05,
            },
            paper_units: 0,
            ..Default::default()
        };
        let r = p.sheet_rect().expect("a paper size");
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(
            close(r.min_x, 0.0)
                && close(r.min_y, 0.0)
                && close(r.max_x, 36.0)
                && close(r.max_y, 24.0),
            "{r:?}"
        );
        // A different offset: shift = (left + ox, bottom + oy) in inches =
        // (0.25 - 0.25, 0.75 - 0.5) = (0, 0.25), so the sheet is
        // (0, -0.25)..(36, 23.75).
        p.plot_origin = Point2D { x: -6.35, y: -12.7 };
        let r = p.sheet_rect().unwrap();
        assert!(
            close(r.min_x, 0.0)
                && close(r.min_y, -0.25)
                && close(r.max_x, 36.0)
                && close(r.max_y, 23.75),
            "{r:?}"
        );
        // No offset, millimetres, turned 90 degrees: the size swaps and the
        // margins alone place it -- (-6.35, -19.05)..(603.25, 895.35).
        p.plot_origin = Point2D { x: 0.0, y: 0.0 };
        p.paper_units = 1;
        p.rotation = 1;
        let r = p.sheet_rect().unwrap();
        assert!(
            close(r.min_x, -6.35)
                && close(r.min_y, -19.05)
                && close(r.max_x, 609.6 - 6.35)
                && close(r.max_y, 914.4 - 19.05),
            "{r:?}"
        );
        p.paper_width_mm = 0.0;
        assert!(p.sheet_rect().is_none());
    }
}
