//! Conversion of the non-entity OBJECT-supertype tables an entity resolves
//! against: LAYER, BLOCK_RECORD and MLINESTYLE.

use crate::convert::owned_entities;
use crate::dynapi::{get_array_field, get_field};
use crate::model::Entity;
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

/// The three maps are `BTreeMap`s, not `HashMap`s, so iteration -- and
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
    let mut layers = BTreeMap::new();
    let mut block_records = BTreeMap::new();
    let mut mlinestyles = BTreeMap::new();

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

fn convert_layer(object_ptr: *mut c_void) -> Option<LayerRecord> {
    let name = crate::dynapi::get_utf8_field(object_ptr, "LAYER", "name")?;
    let color = get_field::<libredwg_sys::Dwg_Color>(object_ptr, "LAYER", "color")?;
    let color_index = resolve_layer_color_index(color.index, color.method, color.rgb);
    Some(LayerRecord { name, color_index })
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
}
