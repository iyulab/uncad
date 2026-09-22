//! Safer access to LibreDWG's dynapi reflection API
//! (`dwg_dynapi_entity_value`/`dwg_dynapi_entity_field`): reading a field off
//! an entity struct by its string name.
//!
//! `Dwg_Object`/`Dwg_Object_Entity`/`Dwg_Entity_*` are all bound as opaque
//! blobs (see libredwg-sys's build.rs) precisely so this module is the *only*
//! place that touches a raw entity pointer -- callers get typed Rust values
//! back, never a struct to poke at directly.

use std::ffi::{c_void, CStr, CString};
use std::mem::MaybeUninit;

/// A DWG 3D point/vector field (BITCODE_3BD, BE, ...): plain C structs of
/// 3 `double`s with no padding.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A DWG 2D point field (BITCODE_2RD, 2BD, 2DPOINT, ...): plain C structs
/// of 2 `double`s -- 2RD (raw) and 2BD (bitcode-compressed on disk) are
/// identical once decoded into memory, so one Rust type covers both.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

/// `Dwg_SPLINE_control_point`'s exact layout: a leading `parent` pointer back
/// to the owning `Dwg_Entity_SPLINE`, then 4 doubles (x, y, z, weight).
///
/// `parent` is not there for dynapi's size check, which only validates the
/// pointer-to-the-array field (8 bytes either way) and never the element
/// stride. It matters because [`get_array_field`] computes each element's
/// address as `base_ptr + i * size_of::<T>()`. An earlier version of this
/// struct omitted it, understating the real 40-byte stride, so every control
/// point after the first was read at a progressively wrong offset --
/// reinterpreting the next point's `parent` pointer as a coordinate. The
/// symptoms were drawing-spanning zigzags on SPLINE-heavy files and a batch of
/// subnormal coordinates (a pointer bit-pattern read as `f64` is often
/// subnormal), the latter of which had been papered over at the formatting
/// layer before the real cause was found here.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplineControlPoint {
    pub parent: *mut c_void,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Default for SplineControlPoint {
    fn default() -> Self {
        SplineControlPoint {
            parent: std::ptr::null_mut(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        }
    }
}

// Pins the element stride get_array_field uses to the real
// Dwg_SPLINE_control_point size, guarding against a recurrence of the
// silent misalignment described above.
#[allow(clippy::unnecessary_operation, clippy::identity_op)]
const _: () = {
    ["Size of SplineControlPoint"][std::mem::size_of::<SplineControlPoint>() - 40];
};

impl From<SplineControlPoint> for Point3D {
    fn from(p: SplineControlPoint) -> Self {
        Point3D {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

/// Returns whether `dwg_dynapi_*_value` writing into a `MaybeUninit<T>` would
/// fit: it `memcpy`s `f.is_malloc() != 0 ? sizeof(char*) : f.size` bytes
/// (dynapi.c's own rule, mirrored here), entirely independent of whatever Rust
/// type the caller chose. Panics in debug builds, turning a wrong-`T` call site
/// into an immediate test failure, and returns `false` in release builds so the
/// caller can refuse the read rather than let dynapi overrun a short buffer.
fn field_write_size_matches<T>(
    f: &libredwg_sys::Dwg_DYNAPI_field,
    dxfname: &str,
    field: &str,
) -> bool {
    let write_size = if f.is_malloc() != 0 {
        std::mem::size_of::<*const c_void>()
    } else {
        f.size as usize
    };
    let ok = write_size == std::mem::size_of::<T>();
    debug_assert!(
        ok,
        "dynapi size mismatch for {dxfname}.{field}: dynapi reports {write_size} bytes, \
         requested Rust type is {} bytes -- wrong T for this field",
        std::mem::size_of::<T>()
    );
    ok
}

/// Reads a plain-old-data field off a DWG entity via `dwg_dynapi_entity_value`.
///
/// `entity` must be the type-specific entity struct pointer (e.g. what
/// would be `Dwg_Entity_LINE*` in C) -- obtained via
/// `libredwg_sys::uncad_object_entity_ptr`, never dereferenced by this
/// crate as anything but an opaque `*mut c_void` handed straight to dynapi.
/// `dxfname` is the entity's type name (e.g. `"LINE"`), matching
/// `dwg_object_get_dxfname()`'s output; `field` is the struct field name
/// as declared in `dwg.h` (e.g. `"start"`, `"radius"`).
///
/// Returns `None` if the field doesn't exist for this entity type, `entity`
/// is null, or (see below) the caller's `T` doesn't match the field's real
/// C size. `T` must be `Copy` -- this is only for fixed-size POD fields
/// (points, numbers, small structs like `Dwg_Color`); string/handle-array
/// fields need dedicated accessors.
///
/// The size check happens *before* the read: `dwg_dynapi_entity_value`
/// unconditionally `memcpy`s the field's own byte count into `out` regardless
/// of `T`, so the read-only `dwg_dynapi_entity_field` lookup is used first to
/// confirm the sizes agree. Checking afterwards (as an earlier version did,
/// with a post-call `debug_assert_eq!`) is too late -- in a release build the
/// C-side write has already overrun a too-small `MaybeUninit<T>`.
pub fn get_field<T: Copy>(entity: *mut c_void, dxfname: &str, field: &str) -> Option<T> {
    if entity.is_null() {
        return None;
    }
    let c_dxfname = CString::new(dxfname).expect("dxfname has no interior NUL");
    let c_field = CString::new(field).expect("field name has no interior NUL");

    // SAFETY: pure name -> descriptor lookup, no write through any pointer.
    let field_desc =
        unsafe { libredwg_sys::dwg_dynapi_entity_field(c_dxfname.as_ptr(), c_field.as_ptr()) };
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, dxfname, field) {
        return None;
    }

    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();

    // SAFETY: `entity` is a valid pointer to the entity type dynapi expects
    // for `dxfname` (upheld by the caller per this function's contract);
    // `out` is sized for T, and the size check above confirms dynapi will
    // write exactly `size_of::<T>()` bytes into it.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_entity_value(
            entity,
            c_dxfname.as_ptr(),
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }

    // SAFETY: dynapi reported success, and the size check above confirms it
    // wrote exactly size_of::<T>() bytes.
    Some(unsafe { out.assume_init() })
}

/// Same as [`get_field`], but for fields declared on the *common*
/// entity/object struct (`Dwg_Object_Entity`/`Dwg_Object_Object`, e.g.
/// `layer`, `color`) rather than the type-specific one (`Dwg_Entity_LINE`,
/// ...) -- calls `dwg_dynapi_common_value` instead of
/// `dwg_dynapi_entity_value`. Takes the same entity pointer as
/// [`get_field`]: `dwg_obj_generic_to_object` (used internally by dynapi)
/// can walk backward from either the type-specific or common struct
/// pointer to find the owning `Dwg_Object`, so callers never need to track
/// which pointer flavor they have.
pub fn get_common_field<T: Copy>(entity: *mut c_void, field: &str) -> Option<T> {
    if entity.is_null() {
        return None;
    }
    let c_field = CString::new(field).expect("field name has no interior NUL");

    // Same rationale as get_field: look the field up (no write) before
    // calling dwg_dynapi_common_value. dwg_dynapi_common_value itself picks
    // the entity- or object-common field table based on the live object's
    // supertype (dynapi.c), which isn't available here -- so try the entity
    // table first (every current call site in this crate passes an entity
    // pointer) and fall back to the object table, matching this function's
    // own doc comment that it accepts either.
    // SAFETY: pure name -> descriptor lookups, no write through any pointer.
    let field_desc = unsafe { libredwg_sys::dwg_dynapi_common_entity_field(c_field.as_ptr()) };
    let field_desc = if field_desc.is_null() {
        unsafe { libredwg_sys::dwg_dynapi_common_object_field(c_field.as_ptr()) }
    } else {
        field_desc
    };
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, "<common>", field) {
        return None;
    }

    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();

    // SAFETY: same contract as get_field -- out is sized for T, and the
    // size check above confirms dynapi will write exactly size_of::<T>()
    // bytes into it.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_common_value(
            entity,
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }

    Some(unsafe { out.assume_init() })
}

/// Reads a plain-old-data header variable (`INSUNITS`, `EXTMIN`, `DIMSCALE`,
/// ...) via `dwg_dynapi_header_value`, with the same size check as
/// [`get_field`] (through `dwg_dynapi_header_field`). `dwg` must be a live,
/// successfully read `Dwg_Data`. Returns `None` for an unknown variable name
/// or a `T` of the wrong size.
pub fn get_header_field<T: Copy>(dwg: *const libredwg_sys::Dwg_Data, field: &str) -> Option<T> {
    if dwg.is_null() {
        return None;
    }
    let c_field = CString::new(field).expect("field name has no interior NUL");

    // SAFETY: pure name -> descriptor lookup, no write through any pointer.
    let field_desc = unsafe { libredwg_sys::dwg_dynapi_header_field(c_field.as_ptr()) };
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, "<header>", field) {
        return None;
    }

    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();
    // SAFETY: dwg is live (caller contract); out is sized for T and the size
    // check above confirms dynapi writes exactly size_of::<T>() bytes.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_header_value(
            dwg,
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }
    // SAFETY: dynapi reported success and wrote size_of::<T>() bytes.
    Some(unsafe { out.assume_init() })
}

/// Reads a text header variable (`DIMPOST`, `DWGCODEPAGE`, ...) as UTF-8 via
/// `dwg_dynapi_header_utf8text`, with the same code-page handling as
/// [`get_utf8_field`]. Returns `None` for an unknown name or a null string.
pub fn get_header_utf8(dwg: *const libredwg_sys::Dwg_Data, field: &str) -> Option<String> {
    if dwg.is_null() {
        return None;
    }
    let c_field = CString::new(field).expect("field name has no interior NUL");
    let mut text_ptr: *mut std::os::raw::c_char = std::ptr::null_mut();
    let mut is_new: std::os::raw::c_int = 0;

    // SAFETY: dwg is live (caller contract); text_ptr/is_new are valid
    // out-params for the duration of the call.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_header_utf8text(
            dwg,
            c_field.as_ptr(),
            &mut text_ptr,
            &mut is_new,
            std::ptr::null_mut(),
        )
    };
    if !ok || text_ptr.is_null() {
        return None;
    }
    let owned = if is_new != 0 {
        // SAFETY: as in get_utf8_field -- a malloc'd UTF-8 copy that is ours
        // to free.
        let owned = unsafe { CStr::from_ptr(text_ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { libc::free(text_ptr.cast()) };
        owned
    } else {
        // SAFETY: dwg is live and text_ptr is a NUL-terminated string it owns.
        let converted = unsafe { libredwg_sys::uncad_tv_to_utf8(dwg, text_ptr) };
        owned_utf8(converted, text_ptr)
    };
    Some(owned)
}

/// Resolves a `BITCODE_H` handle reference (e.g. an entity's `layer`
/// field) to the name of the object it points at, via
/// `dwg_dynapi_handle_name`. Returns `None` for a null handle or an object
/// with no name field (not every handle target has one).
pub fn resolve_handle_name(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<String> {
    if handle.is_null() {
        return None;
    }
    let mut alloced: std::os::raw::c_int = 0;
    // SAFETY: dwg is a live Dwg_Data (caller contract, same as the rest of
    // this crate's conversion pass); handle is checked non-null above.
    let name_ptr = unsafe { libredwg_sys::dwg_dynapi_handle_name(dwg, handle, &mut alloced) };
    if name_ptr.is_null() {
        return None;
    }
    let owned = if alloced != 0 {
        // SAFETY: alloced != 0 means dwg_dynapi_handle_name malloc'd this
        // buffer itself (documented in dwg_api.h) -- already UTF-8, converted
        // from the R2007+ UTF-16 storage -- and it is ours to free.
        let owned = unsafe { CStr::from_ptr(name_ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { libc::free(name_ptr.cast()) };
        owned
    } else {
        // A raw pointer into the Dwg_Data, in the file's own code page (every
        // pre-R2007 DWG, and every DXF input) -- see codepage_to_utf8.
        // SAFETY: dwg is live (caller contract) and name_ptr is a valid
        // NUL-terminated string owned by it.
        let converted = unsafe { libredwg_sys::uncad_tv_to_utf8(dwg, name_ptr) };
        owned_utf8(converted, name_ptr)
    };
    Some(owned)
}

/// Turns the buffer `uncad_tv_to_utf8`/`uncad_entity_tv_to_utf8` returned
/// into an owned `String` and frees it. `raw` is the unconverted string the
/// shim was given, used only as a last-resort lossy fallback when the shim
/// reports out of memory (a null result), so text is degraded rather than
/// dropped.
fn owned_utf8(converted: *mut std::os::raw::c_char, raw: *const std::os::raw::c_char) -> String {
    if converted.is_null() {
        // SAFETY: raw is a valid NUL-terminated C string (caller contract).
        return unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
    }
    // SAFETY: the shim returns a fresh, NUL-terminated heap buffer that is
    // ours to free (see uncad_shim.h); nothing else holds a reference to it.
    let owned = unsafe { CStr::from_ptr(converted) }
        .to_string_lossy()
        .into_owned();
    unsafe { libredwg_sys::uncad_free_string(converted) };
    owned
}

/// Converts a dynapi string that came back with `isnew == 0` -- a raw pointer
/// into the parsed `Dwg_Data`, holding the file's own code-page bytes -- to
/// UTF-8 through the `uncad_entity_tv_to_utf8` shim, which finds the owning
/// `Dwg_Data` (and so the code page and the R2007+ rule) from the entity
/// pointer.
///
/// This is what makes Korean text in an R2000/R2004 drawing, or a degree
/// sign in an R2004 dimension, come out as the right characters: LibreDWG's
/// dynapi only transcodes the R2007+ UTF-16 storage itself, and the
/// `to_string_lossy` this crate used to apply to the raw bytes replaced every
/// non-ASCII byte with U+FFFD.
fn codepage_to_utf8(entity: *const c_void, raw: *const std::os::raw::c_char) -> String {
    // SAFETY: entity is a live entity/object pointer (caller contract, same as
    // every dynapi read here) and raw a valid NUL-terminated string owned by
    // the same Dwg_Data; the shim reads both and allocates its result.
    let converted = unsafe { libredwg_sys::uncad_entity_tv_to_utf8(entity, raw) };
    owned_utf8(converted, raw)
}

/// Reads a text field (BITCODE_T/TV/TU) as a UTF-8 `String`, via
/// `dwg_dynapi_entity_utf8text`. Returns `None` if the field doesn't exist
/// or is a null string.
///
/// The C function returns either a freshly `malloc`'d UTF-8 buffer (the
/// r2007+ path: it converts the UTF-16 storage itself) or a pointer straight
/// into the parsed `Dwg_Data` holding the file's own 8-bit code-page bytes
/// (every older DWG, and every DXF input) -- `isnew` tells us which. The
/// second case goes through [`codepage_to_utf8`]; both end up as an owned
/// Rust `String`, with the C-side buffer freed here so nothing leaks per
/// field read.
pub fn get_utf8_field(entity: *mut c_void, dxfname: &str, field: &str) -> Option<String> {
    if entity.is_null() {
        return None;
    }
    let c_dxfname = CString::new(dxfname).expect("dxfname has no interior NUL");
    let c_field = CString::new(field).expect("field name has no interior NUL");
    let mut text_ptr: *mut std::os::raw::c_char = std::ptr::null_mut();
    let mut is_new: std::os::raw::c_int = 0;

    // SAFETY: entity is valid per this function's contract (same as
    // get_field); text_ptr/is_new are valid out-params for the duration of
    // the call.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_entity_utf8text(
            entity,
            c_dxfname.as_ptr(),
            c_field.as_ptr(),
            &mut text_ptr,
            &mut is_new,
            std::ptr::null_mut(),
        )
    };
    if !ok || text_ptr.is_null() {
        return None;
    }

    let owned = if is_new != 0 {
        // SAFETY: is_new != 0 means dwg_dynapi_entity_utf8text malloc'd
        // this buffer itself (documented in dwg_api.h) -- the UTF-8 it
        // converted from the R2007+ UTF-16 storage -- and it's ours to free;
        // nothing else holds a reference to it.
        let owned = unsafe { CStr::from_ptr(text_ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { libc::free(text_ptr.cast()) };
        owned
    } else {
        // A raw pointer into the Dwg_Data in the file's own code page
        // (pre-R2007 DWG, or any DXF input): transcode it.
        codepage_to_utf8(entity, text_ptr)
    };

    Some(owned)
}

/// Reads a `(count_field, array_field)` pair -- e.g. LWPOLYLINE's
/// `num_points`/`points` -- as an owned `Vec<T>`. The array field is a raw
/// pointer into memory LibreDWG itself owns (freed by `dwg_free`, not by
/// this call), so this copies every element out rather than borrowing.
///
/// `C` is the count field's own C integer type -- `BITCODE_BL` (u32) for most
/// `num_X` fields, but e.g. SPLINE's `num_fit_pts` is `BITCODE_BS` (u16, *not*
/// i16), so one count width cannot be hardcoded for every caller.
pub fn get_array_field<C: Copy + TryInto<usize>, T: Copy>(
    entity: *mut c_void,
    dxfname: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<T> {
    let Some(count) = get_field::<C>(entity, dxfname, count_field) else {
        return Vec::new();
    };
    let Ok(count) = count.try_into() else {
        return Vec::new(); // negative/bogus count -- treat as empty, not a panic.
    };
    if count == 0 {
        return Vec::new();
    }
    let Some(ptr) = get_field::<*const T>(entity, dxfname, array_field) else {
        return Vec::new();
    };
    if ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: `count` and `ptr` come from the same dynapi-reported struct
    // fields (num_X/X is LibreDWG's own array-length convention throughout
    // dwg.h), and the memory stays valid until dwg_free -- we copy out
    // before returning, so no lifetime is smuggled past this call.
    unsafe { std::slice::from_raw_parts(ptr, count) }.to_vec()
}
