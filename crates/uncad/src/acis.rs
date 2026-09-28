//! Reads a 3DSOLID/REGION/BODY's ACIS body out of LibreDWG as SAT text, and
//! hands it to `uncad_model::acis::wireframe` -- the SAT reading itself is the
//! model's, shared with every other reader. What stays here is the part that
//! needs LibreDWG: reading the entity's ACIS fields through dynapi, and
//! converting a binary (SAB) body to SAT on a copy. See `docs/ARCHITECTURE.md`,
//! "3DSOLID / REGION ACIS wireframe".

use crate::dynapi::get_field;
use std::ffi::{c_void, CStr};
use uncad_model::model::Point3D;

/// Reads the SAT (v1, ASCII) text for a 3DSOLID/REGION/BODY entity, converting
/// from SAB (v2, binary) first when that is how this entity stored its ACIS
/// data -- always on a copy, through `libredwg-sys`'s
/// `uncad_3dsolid_sab_to_sat_text` shim, never in place. `None` if the solid is
/// empty or its data cannot be read or converted.
///
/// `dxfname` must be the entity's own real name (`"3DSOLID"`, `"REGION"`, ...).
/// dynapi checks it against the object's actual `obj->name` and refuses to read
/// any field on a mismatch, even though REGION and 3DSOLID share one C struct
/// and one dynapi field table -- hardcoding `"3DSOLID"` would silently return
/// `None` for every field of a real REGION.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity__3DSOLID*` (as
/// returned by `uncad_object_entity_ptr` for a 3DSOLID/REGION/BODY object),
/// valid for the duration of this call.
unsafe fn read_sat_text_from_entity(entity_ptr: *mut c_void, dxfname: &str) -> Option<String> {
    let version = get_field::<u16>(entity_ptr, dxfname, "version")?;
    let acis_empty = get_field::<u8>(entity_ptr, dxfname, "acis_empty").unwrap_or(0);
    if acis_empty != 0 {
        return None;
    }

    if version != 2 {
        // Already SAT v1 (older AutoCAD releases write ACIS as text directly).
        let size = get_field::<u32>(entity_ptr, dxfname, "sab_size").unwrap_or(0);
        let ptr = get_field::<*const u8>(entity_ptr, dxfname, "acis_data")?;
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid pointer to at least `size` bytes (or, if
        // size is unset/0, a NUL-terminated buffer) per LibreDWG's own
        // acis_data contract; valid until dwg_free, which outlives this
        // whole conversion pass.
        let bytes: Vec<u8> = unsafe {
            if size > 0 {
                std::slice::from_raw_parts(ptr, size as usize).to_vec()
            } else {
                CStr::from_ptr(ptr.cast()).to_bytes().to_vec()
            }
        };
        return Some(String::from_utf8_lossy(&bytes).into_owned());
    }

    // SAB (v2, binary), converted to SAT text on a *copy* of the entity.
    // Calling LibreDWG's dwg_convert_SAB_to_SAT1 on the live entity, which is
    // what this used to do, converts in place (version 2 -> 1, plaintext SAT
    // into encr_sat_data, acis_data left as SAB bytes). parse() reads every
    // solid twice -- convert_entities for model space, then convert_tables for
    // the owning block record -- so the second read took the `version != 2`
    // branch above, parsed binary SAB as text, and produced no wireframe. See
    // shim/uncad_shim.h and docs/CAVEATS.md, "3DSOLID SAB conversion".
    let mut len: usize = 0;
    // SAFETY: entity_ptr is a valid Dwg_Entity__3DSOLID* per this function's
    // contract; the shim only reads through it (and through its `parent`
    // back-pointer, to reach the drawing's header version) and writes to
    // its own stack copy.
    let text_ptr = unsafe { libredwg_sys::uncad_3dsolid_sab_to_sat_text(entity_ptr, &mut len) };
    if text_ptr.is_null() {
        return None;
    }
    // SAFETY: the shim returned a malloc'd buffer valid for `len` bytes (plus
    // a trailing NUL), owned by this function until uncad_free_sat_text below.
    let bytes = unsafe { std::slice::from_raw_parts(text_ptr.cast::<u8>(), len) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    // SAFETY: text_ptr came from uncad_3dsolid_sab_to_sat_text and is freed
    // exactly once, here, after the last read of it above.
    unsafe { libredwg_sys::uncad_free_sat_text(text_ptr) };
    if text.is_empty() {
        return None;
    }
    Some(text)
}

/// Best-effort wireframe extraction for one 3DSOLID/REGION/BODY entity: reads
/// its ACIS data, converting from SAB to SAT if needed, parses the records, and
/// returns one chord segment per ACIS `edge`. Empty when the solid is empty,
/// its data cannot be read, or it has no edges -- the caller then treats the
/// entity as unsupported.
///
/// `dxfname` is the entity's own real name; see `read_sat_text_from_entity` for
/// why it cannot be hardcoded.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity__3DSOLID*`.
pub unsafe fn extract_wireframe(
    entity_ptr: *mut c_void,
    dxfname: &str,
) -> (Vec<[Point3D; 2]>, usize) {
    let Some(sat_text) = (unsafe { read_sat_text_from_entity(entity_ptr, dxfname) }) else {
        return (Vec::new(), 0);
    };
    uncad_model::acis::wireframe(&sat_text)
}
