//! Reads a 3DSOLID/REGION/BODY's ACIS body out of LibreDWG and hands it to
//! `uncad_model::acis` -- as SAT text (`wireframe`) or as SAB bytes
//! (`wireframe_sab`); the reading of either is the model's, shared with every
//! other reader. What stays here is the part that needs LibreDWG: reading the
//! entity's ACIS fields through dynapi, and telling an empty body from one
//! that is stored where this read did not reach. See `docs/ARCHITECTURE.md`,
//! "3DSOLID / REGION ACIS wireframe".

use crate::dynapi::{get_common_field, get_field};
use std::ffi::{c_void, CStr};
use uncad_model::model::Point3D;

/// One chord segment per ACIS `edge` of a 3DSOLID/REGION/BODY entity, and
/// the number of edges that could not be made one -- or `None` when its body
/// could not be read, which the caller reports as an entity it does not
/// read rather than as an empty solid.
///
/// - No body in the object, and the entity says its body is in the file's
///   data storage (R2013 on): `None` -- no data-storage record named this
///   entity with a body LibreDWG could attach (the vendored copy attaches
///   each by the handle its record names; see `docs/CAVEATS.md`, "Local
///   patches to the vendored LibreDWG").
/// - No body in the object and no data-storage record: an empty solid,
///   `(vec![], 0)`.
/// - SAT text (`version` 1): the model's SAT reading.
/// - SAB bytes (`version` 2, and every body attached from the data
///   storage): the model's SAB reading -- `None` when they do not decode.
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
pub unsafe fn extract_wireframe(
    entity_ptr: *mut c_void,
    dxfname: &str,
) -> Option<(Vec<[Point3D; 2]>, usize)> {
    let acis_empty = get_field::<u8>(entity_ptr, dxfname, "acis_empty").unwrap_or(0);
    if acis_empty != 0 {
        let has_ds_data = get_common_field::<u8>(entity_ptr, "has_ds_data").unwrap_or(0);
        return (has_ds_data == 0).then(|| (Vec::new(), 0));
    }
    let version = get_field::<u16>(entity_ptr, dxfname, "version")?;
    let ptr = get_field::<*const u8>(entity_ptr, dxfname, "acis_data")?;
    if ptr.is_null() {
        return None;
    }
    let size = get_field::<u32>(entity_ptr, dxfname, "sab_size").unwrap_or(0);
    // SAFETY: ptr is a valid pointer to at least `size` bytes (or, if size is
    // unset/0, a NUL-terminated buffer) per LibreDWG's own acis_data
    // contract; valid until dwg_free, which outlives this whole conversion
    // pass. Only read, never written: parse() reads every solid twice
    // (model space, then the owning block record), and both reads must see
    // the same body.
    let bytes: &[u8] = unsafe {
        if size > 0 {
            std::slice::from_raw_parts(ptr, size as usize)
        } else {
            CStr::from_ptr(ptr.cast()).to_bytes()
        }
    };
    if version == 2 {
        uncad_model::acis::wireframe_sab(bytes)
    } else {
        // SAT (v1, ASCII): older AutoCAD releases write ACIS as text.
        Some(uncad_model::acis::wireframe(&String::from_utf8_lossy(
            bytes,
        )))
    }
}
