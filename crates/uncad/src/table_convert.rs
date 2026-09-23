//! Conversion of the non-entity OBJECT-supertype tables an entity resolves
//! against: LAYER, BLOCK_RECORD and MLINESTYLE -- from LibreDWG's structures
//! into the model's [`Tables`].

use crate::convert::{owned_entities, reference};
use crate::dynapi::{get_array_field, get_field, is_from_dxf, is_r2000_or_later};
use crate::text::TextDecoder;
use std::collections::BTreeMap;
use std::ffi::c_void;
use uncad_model::tables::{BlockRecord, DimStyleRecord, LayerRecord, Tables};

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
        layouts: Default::default(),
    }
}

/// Reads a DIMSTYLE table entry -- the settings a dimension names rather than
/// carries (see [`DimStyleRecord`]).
///
/// Every value comes back `Some`. This library holds a style as a struct with
/// no "the file did not write this group", so which of these the file stated
/// and which are the values it starts from cannot be told apart here; the
/// other reader of this format, which sees the groups themselves, can.
fn convert_dim_style(text: &TextDecoder, object_ptr: *mut c_void) -> Option<DimStyleRecord> {
    let name = text.field(object_ptr, "DIMSTYLE", "name")?;
    let number = |field: &str| get_field::<f64>(object_ptr, "DIMSTYLE", field);
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
        arrow_size: None,
        linear_unit_format: None,
        zero_suppression: None,
        rounding: None,
        angular_unit_format: None,
        angular_decimal_places: None,
        fraction_format: None,
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
    let color_index = resolve_layer_color_index(color.index, color.method, color.rgb);
    let bit = |field: &str| get_field::<u8>(object_ptr, "LAYER", field).is_some_and(|b| b != 0);
    let from_dxf = is_from_dxf(dwg);
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
}
