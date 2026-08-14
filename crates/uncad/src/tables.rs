//! Non-entity OBJECT-supertype table conversion (LAYER, BLOCK_RECORD --
//! mirrors `converter.ts`'s `convertLayer`/`convertBlockRecord`).

use crate::convert::owned_entities;
use crate::dynapi::{get_array_field, get_field};
use crate::render_model::RenderEntity;
use std::collections::HashMap;
use std::ffi::c_void;

#[derive(Debug, Clone)]
pub struct LayerRecord {
    pub name: String,
    /// Same raw semantics as `EntityCommon::color_index`: <0 off, else the
    /// layer's own ACI palette index -- should never be 0/256 in practice
    /// (BYLAYER/BYBLOCK are entity-level concepts, not something a layer
    /// resolves against itself), but LibreDWG's own `bit_read_CMC` can hand
    /// back a raw `256` "no palette match" sentinel for certain real files;
    /// see [`resolve_layer_color_index`]'s doc comment for the recovery
    /// this module applies before a `256` ever reaches this field.
    pub color_index: i16,
}

#[derive(Debug, Clone)]
pub struct BlockRecord {
    pub name: String,
    /// Every entity directly owned by this block (via
    /// `get_first_owned_entity`/`get_next_owned_entity`), regardless of
    /// whether the block is `*Model_Space`/`*Paper_Space*` or a named
    /// block definition referenced by INSERT elsewhere -- unlike
    /// `CadDatabase::entities`, which only includes the former. This is
    /// what an INSERT's `block_name` resolves against to find what it
    /// actually draws.
    pub entities: Vec<RenderEntity>,
}

#[derive(Debug, Clone, Default)]
pub struct Tables {
    /// Layer name -> record. Deliberately does *not* expose `Dwg_Color.rgb`
    /// for layers -- see docs/CAVEATS.md / the color module: that field is a
    /// constant placeholder (always 0xFFFFFF) on every LAYER table entry
    /// LibreDWG parses, regardless of the layer's real color, confirmed
    /// across 22 real-world files during the original JS implementation.
    /// Only `color_index` is trustworthy for BYLAYER resolution.
    pub layers: HashMap<String, LayerRecord>,
    /// Block name -> record, every `BLOCK_HEADER` in the file (including
    /// `*Model_Space`/`*Paper_Space*`, which also show up flattened into
    /// `CadDatabase::entities` -- see that field's doc comment).
    pub block_records: HashMap<String, BlockRecord>,
    /// MLINESTYLE name -> each parallel line's `offset` (distance from the
    /// MLINE centerline), in the same order LibreDWG stores them in --
    /// `svg.rs`'s MLINE rendering pairs index `i` here with the same index
    /// `i` in every MLINE vertex's per-vertex line list (implicit -- there's
    /// no separate "line identity" field, just consistent array order). See
    /// `convert::convert_mline`'s doc comment for why only `offset` is read
    /// (color/linetype per line aren't rendered).
    pub mlinestyles: HashMap<String, Vec<f64>>,
}

/// # Safety
/// `dwg` must be a successfully-`dwg_read_file`'d, not-yet-`dwg_free`'d
/// `Dwg_Data`.
pub unsafe fn convert_tables(dwg: *mut libredwg_sys::Dwg_Data) -> Tables {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut layers = HashMap::new();
    let mut block_records = HashMap::new();
    let mut mlinestyles = HashMap::new();

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
                if let Some(record) = convert_layer(object_ptr) {
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
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINESTYLE {
            let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if !object_ptr.is_null() {
                if let Some((name, offsets)) = convert_mlinestyle(object_ptr) {
                    mlinestyles.insert(name, offsets);
                }
            }
        }
    }

    Tables {
        layers,
        block_records,
        mlinestyles,
    }
}

/// `BLOCK_HEADER.name` is only the *abbreviated* name for anonymous blocks
/// (e.g. `"*D"` for every anonymous DIMENSION-geometry-cache block, `"*T"`
/// for table-cache blocks) -- the real, disambiguating name (`"*D30"`) is
/// only present on the block's own `BLOCK` entity, reached via
/// `BLOCK_HEADER.block_entity` (a direct handle field -- *not* via
/// `get_first_owned_entity`, which was confirmed to skip the BLOCK/ENDBLK
/// sentinels entirely as part of its own iteration protocol, not just
/// something this crate filters out afterward). Confirmed against
/// `example_r14.dwg`: 9 separate anonymous blocks all reported
/// `BLOCK_HEADER.name == "*D"`, silently collapsing to one entry when used
/// as a HashMap key -- matches `converter.ts`'s own `convertBlockRecord`,
/// which reads the BLOCK entity's name specifically for this reason ("we
/// want '*D30' instead of '*D'"). Falls back to the abbreviated name if
/// there's no BLOCK entity (shouldn't normally happen, but the fallback is
/// cheap insurance).
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
                // independently-generated (non-opaque) bindgen item for
                // the same `_dwg_object` C tag that Dwg_Object/dwg_object
                // are opaqued to elsewhere (see build.rs's opaque_type
                // comments -- the same duplicate-representation quirk hit
                // there for Dwg_Data/dwg_data). Identical layout, so a
                // pointer cast is sound.
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

/// Resolves a `BITCODE_H` handle to a `BLOCK_HEADER` (e.g. INSERT's
/// `block_header` field, or a DIMENSION's `block` field) to that block's
/// *disambiguated* name -- the same resolution [`convert_tables`] uses to
/// key `block_records`, so callers can reliably look up
/// `Tables::block_records` afterward. **Not** the same as
/// `crate::dynapi::resolve_handle_name` (which returns `BLOCK_HEADER.name`
/// directly, i.e. the *abbreviated* name for anonymous blocks) -- using
/// that one here was a real bug: a DIMENSION's `block_name` resolved to
/// `"*D"` for every anonymous dimension-cache block, none of which could
/// ever be found in `block_records` (keyed by the real `"*D30"`-style
/// name), silently breaking every DIMENSION's rendering.
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

/// Reads an MLINESTYLE OBJECT's name and each of its parallel lines'
/// `offset` (distance from the MLINE centerline), in array order -- see
/// `Tables::mlinestyles`'s doc comment for why order (not any explicit
/// identity) is what `svg.rs` pairs against each MLINE vertex's own line
/// list.
fn convert_mlinestyle(object_ptr: *mut c_void) -> Option<(String, Vec<f64>)> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "MLINESTYLE", "name")?;
    // num_lines is BITCODE_RC (a single unsigned byte), unlike num_paths'
    // BITCODE_BL -- see get_array_field's doc comment on why the count
    // field's width can't be hardcoded for every caller.
    let lines: Vec<libredwg_sys::Dwg_MLINESTYLE_line> =
        get_array_field::<u8, _>(object_ptr, "MLINESTYLE", "num_lines", "lines");
    let offsets = lines.iter().map(|l| l.offset).collect();
    Some((name, offsets))
}

fn convert_layer(object_ptr: *mut c_void) -> Option<LayerRecord> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "LAYER", "name")?;
    let color = get_field::<libredwg_sys::Dwg_Color>(object_ptr, "LAYER", "color")?;
    let color_index = resolve_layer_color_index(color.index, color.method, color.rgb);
    Some(LayerRecord { name, color_index })
}

/// Recovers a usable ACI index from a LAYER's raw `Dwg_Color` when LibreDWG
/// itself couldn't. `bit_read_CMC` (`lib/libredwg/src/bits.c`) sets `index`
/// via `dwg_find_color_index(rgb)`, which returns `256` ("no exact palette
/// match", *not* a real BYLAYER-on-a-layer sentinel -- a LAYER can't be
/// BYLAYER against itself) whenever a TRUECOLOR-tagged layer's `rgb` isn't
/// bit-identical to one of the 256 ACI palette entries' own packed RGB.
///
/// Every real file in `samples/` (9 independent AutoCAD Architecture
/// drawings, spot-checked 2026-08-14 by rendering all of them and comparing
/// against AutoCAD's own default/AIA layer-color conventions) hits exactly
/// this case, and stores what's obviously the *intended* ACI index itself
/// in `rgb`'s low byte rather than a genuine 24-bit color -- e.g. layer
/// `"0"` is always `rgb=0x..000007` across every file, matching AutoCAD's
/// real default of ACI 7 for that layer. `bit_downconvert_CMC`
/// (`bits.c:4069-4071`) already has the identical `if index==256 { index =
/// rgb & 0xff }` fallback for this exact situation on its own (differently
/// gated) code path; this mirrors it for `bit_read_CMC`'s plain-read path,
/// which lacks it.
///
/// **Known limitation** (inherited from `bit_downconvert_CMC`'s own
/// version of this fallback, not introduced here): a layer with a genuine
/// arbitrary truecolor RGB that simply doesn't exactly match any ACI
/// palette entry would also report `index=256`, and this reinterprets its
/// blue channel as an ACI index instead -- indistinguishable, from the
/// data available here, from the degenerate case above. Not verified
/// against real AutoCAD (no AutoCAD available); verified only by rendering
/// and eyeballing plausibility across the 9 `samples/` files.
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
}
