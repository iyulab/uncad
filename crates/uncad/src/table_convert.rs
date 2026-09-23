//! Conversion of the non-entity OBJECT-supertype records an entity or a
//! sheet resolves against -- LAYER, BLOCK_RECORD, DIMSTYLE, MLINESTYLE, and
//! LAYOUT with its embedded plot settings -- from LibreDWG's structures into
//! the model's [`Tables`].
//!
//! All of them are collected by one pass over the objects, dispatching on
//! `dwg_object_get_fixedtype`. The named object dictionary is never walked:
//! a file with no LAYOUT object -- a DXF without an OBJECTS section, or a
//! drawing older than R2000 that no application with layouts saved -- has
//! no layouts.

use crate::convert::{owned_entities, reference};
use crate::dynapi::{
    get_array_field, get_field, get_point2d, get_sub_field, is_from_dxf, is_r2000_or_later,
    RawPoint2D,
};
use crate::text::TextDecoder;
use std::collections::BTreeMap;
use std::ffi::c_void;
use uncad_model::model::Point2D;
use uncad_model::tables::{
    AngularUnitFormat, BlockRecord, DimStyleRecord, FractionFormat, LayerRecord, LayoutRecord,
    LinearUnitFormat, PlotPaperUnits, PlotRotation, PlotSettings, Tables,
};

/// # Safety
/// `dwg` must be a successfully-`dwg_read_file`'d, not-yet-`dwg_free`'d
/// `Dwg_Data`, and the caller must hold `LIBREDWG_LOCK` (see lib.rs) for
/// the whole call -- this walks LibreDWG's non-reentrant C API directly.
///
/// `pub(crate)`, not `pub`: `parse()` is the only intended caller. Exporting
/// a `*mut Dwg_Data` entry point would bypass the lock and leak
/// `libredwg_sys` types into the public surface.
pub(crate) unsafe fn convert_tables(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
) -> Tables {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut layers = BTreeMap::new();
    let mut block_records = BTreeMap::new();
    let mut mlinestyles = BTreeMap::new();
    let mut dim_styles = BTreeMap::new();
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
                if let Some(record) = convert_layer(dwg, text, object_ptr) {
                    layers.insert(record.name.clone(), record);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK_HEADER {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(name) = block_record_name(text, object_ptr) {
                    let entities = unsafe { owned_entities(dwg, text, obj) };
                    block_records.insert(name.clone(), BlockRecord { name, entities });
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMSTYLE {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(record) = convert_dim_style(text, object_ptr) {
                    dim_styles.insert(record.name.clone(), record);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LAYOUT {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some(record) = convert_layout(dwg, text, object_ptr) {
                    layouts.insert(record.name.clone(), record);
                }
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINESTYLE {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some((name, offsets)) = convert_mlinestyle(text, object_ptr) {
                    mlinestyles.insert(name, offsets);
                }
            }
        }
    }

    Tables {
        layers,
        dim_styles,
        block_records,
        mlinestyles,
        layouts,
    }
}

/// Reads a LAYOUT object -- a tab of the drawing, the block it shows -- and
/// the plot settings embedded in it (`plotsettings`, a
/// `Dwg_Object_PLOTSETTINGS` read through its own dynapi table; see
/// [`get_sub_field`]).
///
/// Every value is what the object holds. Two of them a DXF may leave out
/// and the importer fills in, so they cannot be told from a stated value:
/// the zero points, and the paper-units side of the custom scale (DXF 142),
/// which the importer sets to 1 on every LAYOUT before it reads the groups.
/// A plot unit or rotation outside the format's range is `None`.
fn convert_layout(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    object_ptr: *mut c_void,
) -> Option<LayoutRecord> {
    let name = text.field(object_ptr, "LAYOUT", "layout_name")?;
    let block_name = reference(
        dwg,
        text,
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYOUT", "block_header"),
        c"BLOCK",
        |handle_ptr| resolve_block_name(text, handle_ptr),
    );
    let point = |field: &str| get_point2d(object_ptr, "LAYOUT", field).unwrap_or_default();
    let plot = |field: &str| {
        get_sub_field::<f64>(object_ptr, "LAYOUT", "plotsettings", "PLOTSETTINGS", field)
            .unwrap_or(0.0)
    };
    let plot_code = |field: &str| {
        get_sub_field::<u16>(object_ptr, "LAYOUT", "plotsettings", "PLOTSETTINGS", field)
    };
    let plot_settings = PlotSettings {
        paper_name: text
            .sub_field(
                object_ptr,
                "LAYOUT",
                "plotsettings",
                "PLOTSETTINGS",
                "canonical_media_name",
            )
            .unwrap_or_default(),
        paper_width: plot("paper_width"),
        paper_height: plot("paper_height"),
        margin_left: plot("left_margin"),
        margin_bottom: plot("bottom_margin"),
        margin_right: plot("right_margin"),
        margin_top: plot("top_margin"),
        plot_origin: get_sub_field::<RawPoint2D>(
            object_ptr,
            "LAYOUT",
            "plotsettings",
            "PLOTSETTINGS",
            "plot_origin",
        )
        .map(Point2D::from)
        .unwrap_or_default(),
        paper_units: plot_code("plot_paper_unit").and_then(|v| match v {
            0 => Some(PlotPaperUnits::Inches),
            1 => Some(PlotPaperUnits::Millimeters),
            2 => Some(PlotPaperUnits::Pixels),
            _ => None,
        }),
        rotation: plot_code("plot_rotation_mode").and_then(|v| match v {
            0 => Some(PlotRotation::Unrotated),
            1 => Some(PlotRotation::Counterclockwise90),
            2 => Some(PlotRotation::UpsideDown),
            3 => Some(PlotRotation::Clockwise90),
            _ => None,
        }),
        scale_numerator: plot("paper_units"),
        scale_denominator: plot("drawing_units"),
    };
    Some(LayoutRecord {
        name,
        tab_order: get_field::<u16>(object_ptr, "LAYOUT", "tab_order").map_or(0, i32::from),
        block_name,
        limits_min: point("LIMMIN"),
        limits_max: point("LIMMAX"),
        plot_settings,
    })
}

/// Reads a DIMSTYLE table entry -- the settings a dimension names rather than
/// carries (see [`DimStyleRecord`]).
///
/// Every number comes back `Some` (an enumeration only when its value is one
/// the format defines). This library holds a style as a struct with no "the
/// file did not write this group", so which of these the file stated and
/// which are the values it starts from cannot be told apart here; the other
/// reader of this format, which sees the groups themselves, can. What to do
/// with a husk of a style -- one that states next to nothing -- is a
/// consumer's decision, like the displayed text itself.
fn convert_dim_style(text: &TextDecoder, object_ptr: *mut c_void) -> Option<DimStyleRecord> {
    let name = text.field(object_ptr, "DIMSTYLE", "name")?;
    let number = |field: &str| get_field::<f64>(object_ptr, "DIMSTYLE", field);
    // The 16-bit variables (BITCODE_BS). Read signed: DIMADEC's -1 ("as
    // DIMDEC") is a value the format uses.
    let small = |field: &str| get_field::<i16>(object_ptr, "DIMSTYLE", field).map(i32::from);
    Some(DimStyleRecord {
        name,
        post: text.field(object_ptr, "DIMSTYLE", "DIMPOST"),
        scale: number("DIMSCALE"),
        length_factor: number("DIMLFAC"),
        tolerances: get_field::<u8>(object_ptr, "DIMSTYLE", "DIMTOL").map(|v| v != 0),
        limits: get_field::<u8>(object_ptr, "DIMSTYLE", "DIMLIM").map(|v| v != 0),
        tolerance_upper: number("DIMTP"),
        tolerance_lower: number("DIMTM"),
        decimal_places: get_field::<i16>(object_ptr, "DIMSTYLE", "DIMDEC").map(i32::from),
        tolerance_decimal_places: get_field::<i16>(object_ptr, "DIMSTYLE", "DIMTDEC")
            .map(i32::from),
        text_height: number("DIMTXT"),
        arrow_size: number("DIMASZ"),
        linear_unit_format: small("DIMLUNIT").and_then(|v| match v {
            1 => Some(LinearUnitFormat::Scientific),
            2 => Some(LinearUnitFormat::Decimal),
            3 => Some(LinearUnitFormat::Engineering),
            4 => Some(LinearUnitFormat::Architectural),
            5 => Some(LinearUnitFormat::Fractional),
            6 => Some(LinearUnitFormat::WindowsDesktop),
            _ => None,
        }),
        zero_suppression: small("DIMZIN"),
        rounding: number("DIMRND"),
        angular_unit_format: small("DIMAUNIT").and_then(|v| match v {
            0 => Some(AngularUnitFormat::DecimalDegrees),
            1 => Some(AngularUnitFormat::DegreesMinutesSeconds),
            2 => Some(AngularUnitFormat::Gradians),
            3 => Some(AngularUnitFormat::Radians),
            4 => Some(AngularUnitFormat::SurveyorsUnits),
            _ => None,
        }),
        angular_decimal_places: small("DIMADEC"),
        fraction_format: small("DIMFRAC").and_then(|v| match v {
            0 => Some(FractionFormat::Horizontal),
            1 => Some(FractionFormat::Diagonal),
            2 => Some(FractionFormat::NotStacked),
            _ => None,
        }),
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
fn block_record_name(text: &TextDecoder, block_header_object_ptr: *mut c_void) -> Option<String> {
    let abbreviated = text.field(block_header_object_ptr, "BLOCK_HEADER", "name");

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
                if let Some(full_name) = text.field(entity_ptr, "BLOCK", "name") {
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
/// **Not** interchangeable with `TextDecoder::handle_name`, which
/// returns `BLOCK_HEADER.name` directly. Using that here was a real bug: every
/// anonymous dimension cache resolved to `"*D"`, which is never a key in
/// `block_records`, so no DIMENSION rendered at all.
pub(crate) fn resolve_block_name(
    text: &TextDecoder,
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
    block_record_name(text, object_ptr)
}

/// Reads an MLINESTYLE object's name and each of its parallel lines' `offset`,
/// in array order -- see [`Tables::mlinestyles`].
fn convert_mlinestyle(text: &TextDecoder, object_ptr: *mut c_void) -> Option<(String, Vec<f64>)> {
    let name = text.field(object_ptr, "MLINESTYLE", "name")?;
    // num_lines is BITCODE_RC (one unsigned byte), unlike num_paths'
    // BITCODE_BL -- see get_array_field on why the count width cannot be
    // hardcoded for every caller.
    let lines: Vec<libredwg_sys::Dwg_MLINESTYLE_line> =
        get_array_field::<u8, _>(object_ptr, "MLINESTYLE", "num_lines", "lines");
    let offsets = lines.iter().map(|l| l.offset).collect();
    Some((name, offsets))
}

/// Reads a LAYER table entry: its colour, its state and the linetype it
/// names.
///
/// Which fields state the layer's state depends on who read the file. The
/// binary format's decoder fills the `off`, `frozen` and `locked` bits (for
/// a drawing older than R13 it derives them from group 70 and the colour's
/// sign itself). LibreDWG's DXF importer instead applies the binary layout
/// to group 70 -- bit 2 becomes "off", 4 "frozen in new viewports", 8
/// "locked" -- where a DXF means 1 frozen, 2 frozen in new viewports, 4
/// locked, and says "off" with a negative colour; so for a DXF the raw
/// group 70 and the colour's sign are read instead. A negative colour
/// means "off" whoever read it, as [`LayerRecord::color_index`] documents.
///
/// The plot flag (DXF 290) and the lineweight (DXF 370) exist from R2000
/// on. A DWG states both. The DXF importer leaves a flag the file left out
/// at 0, so a DXF's stated "do not plot" (290 = 0) and its silence read the
/// same -- `None`, not `Some(false)` -- and likewise a lineweight code of 0
/// (0.00 mm, or absent).
fn convert_layer(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    object_ptr: *mut c_void,
) -> Option<LayerRecord> {
    let name = text.field(object_ptr, "LAYER", "name")?;
    let color = get_field::<libredwg_sys::Dwg_Color>(object_ptr, "LAYER", "color")?;
    let from_dxf = is_from_dxf(dwg);
    let color_index = resolve_layer_color_index(color.index, color.method, color.rgb, from_dxf);
    let bit = |field: &str| get_field::<u8>(object_ptr, "LAYER", field).is_some_and(|b| b != 0);
    let (off, frozen, locked) = if from_dxf {
        let flag = get_field::<u8>(object_ptr, "LAYER", "flag").unwrap_or(0);
        (color.index < 0, flag & 1 != 0, flag & 4 != 0)
    } else {
        (bit("off") || color.index < 0, bit("frozen"), bit("locked"))
    };
    let r2000 = is_r2000_or_later(dwg);
    let plot = if !r2000 {
        None
    } else if from_dxf {
        bit("plotflag").then_some(true)
    } else {
        Some(bit("plotflag"))
    };
    let lineweight = get_field::<u8>(object_ptr, "LAYER", "linewt")
        .filter(|&code| r2000 && !(from_dxf && code == 0))
        .and_then(layer_lineweight);
    let linetype = reference(
        dwg,
        text,
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYER", "ltype"),
        c"LTYPE",
        |handle_ptr| text.handle_name(dwg, handle_ptr),
    );
    Some(LayerRecord {
        name,
        color_index,
        off,
        frozen,
        locked,
        plot,
        lineweight,
        linetype,
    })
}

/// The standard lineweights in hundredths of a millimetre, indexed by the
/// code the library stores in `linewt` (`lweights[]` in its `dwg.c` and
/// `in_dxf.c`).
const LINEWEIGHTS: [i16; 24] = [
    0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200,
    211,
];

/// A layer's lineweight for the library's `linewt` code: codes 0 to 23 are
/// the standard weights, 31 is "the default" (DXF -3). 29 and 30 are BYLAYER
/// and BYBLOCK, which a layer cannot be, and 24 to 28 are unused: none of
/// those is a lineweight a layer states.
fn layer_lineweight(code: u8) -> Option<i16> {
    match code {
        31 => Some(-3),
        code => LINEWEIGHTS.get(usize::from(code)).copied(),
    }
}

/// A LAYER's ACI index from its raw `Dwg_Color`.
///
/// From R2004 a color is stored as a 32-bit value whose high byte says how
/// to read the rest. Method `0xC3` -- which `dwg.h` names `TRUECOLOR` -- is a
/// color *index*: the low byte is the ACI number. Layer `"0"` stores
/// `0xC3000007`, AutoCAD's default of ACI 7, and a second, independent DWG
/// reader and the same drawing saved as DXF agree on the index in the low
/// byte wherever it was compared.
///
/// `bit_read_CMC` instead sets `index` from a palette lookup of the value as
/// if it were an RGB color. That lookup fails (the "no match" `256`) for most
/// small values, and -- worse -- succeeds for some: `0xC3000068` (ACI 104) is
/// the RGB color (0, 0, 104), which is palette entry 176, so the layer came
/// back as 176. For method `0xC3` read from a DWG the low byte is therefore
/// taken whatever the lookup said; every other method keeps the library's
/// index.
///
/// Read from DXF it is the other way round: the importer takes the index
/// from group 62 and stores the palette's RGB color under the same method
/// (white is `0xC3FFFFFF`), so there the library's index is the one the file
/// stated, and only its "no match" `256` falls back to the low byte.
fn resolve_layer_color_index(
    index: i16,
    method: libredwg_sys::Dwg_Color_Method,
    rgb: u32,
    from_dxf: bool,
) -> i16 {
    let index_color = method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR;
    if index_color && (!from_dxf || index == 256) {
        (rgb & 0xff) as i16
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_lineweight_is_the_standard_weight_or_the_default() {
        assert_eq!(layer_lineweight(0), Some(0));
        assert_eq!(layer_lineweight(11), Some(50), "0.50 mm");
        assert_eq!(layer_lineweight(23), Some(211));
        assert_eq!(layer_lineweight(31), Some(-3), "the default");
        for not_a_layer_weight in [24, 28, 29, 30, 32, 255] {
            assert_eq!(layer_lineweight(not_a_layer_weight), None);
        }
    }

    #[test]
    fn an_index_colors_low_byte_is_its_aci_number_when_no_palette_entry_matches() {
        let resolved = resolve_layer_color_index(
            256,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
            0xc3000007,
            false,
        );
        assert_eq!(resolved, 7);
    }

    #[test]
    fn an_index_colors_low_byte_wins_over_a_palette_match_of_the_same_value() {
        // ACI 104, which the library's palette lookup of (0, 0, 104) turns
        // into 176.
        let resolved = resolve_layer_color_index(
            176,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
            0xc3000068,
            false,
        );
        assert_eq!(resolved, 104);
    }

    #[test]
    fn another_method_keeps_the_librarys_index() {
        let resolved = resolve_layer_color_index(
            256,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI,
            0,
            false,
        );
        assert_eq!(resolved, 256);
    }

    #[test]
    fn read_from_dxf_the_importers_index_is_the_files() {
        // White, from group 62 = 7: the importer stores the palette's RGB.
        let resolved = resolve_layer_color_index(
            7,
            libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
            0xc3ffffff,
            true,
        );
        assert_eq!(resolved, 7);
    }
}
