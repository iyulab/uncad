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

use uncad_model::{Point2D, Point3D};

/// A type whose Rust layout is exactly the C layout of the LibreDWG field it
/// is read from, so that `dwg_dynapi_*_value`'s `memcpy` into it is sound.
///
/// # Safety
/// Implement only for `#[repr(C)]` types (and primitives / raw pointers)
/// that mirror a `dwg.h` field byte for byte. The model's own types are
/// deliberately *not* implementors: `uncad_model::Point3D` is a plain Rust
/// struct with no layout guarantee, and a `get_field::<Point3D>` call site
/// must fail to compile rather than read C memory through it.
pub unsafe trait DwgRaw: Copy {}

// SAFETY: primitives and raw pointers have the layout C gives them.
unsafe impl DwgRaw for u8 {}
unsafe impl DwgRaw for u16 {}
unsafe impl DwgRaw for u32 {}
unsafe impl DwgRaw for i16 {}
unsafe impl DwgRaw for f64 {}
unsafe impl DwgRaw for [i16; 4] {}
unsafe impl<T> DwgRaw for *const T {}
unsafe impl<T> DwgRaw for *mut T {}
// SAFETY: bindgen generates these as `#[repr(C)]` mirrors of dwg.h.
unsafe impl DwgRaw for libredwg_sys::Dwg_Color {}
unsafe impl DwgRaw for libredwg_sys::Dwg_HATCH_Path {}
unsafe impl DwgRaw for libredwg_sys::Dwg_HATCH_DefLine {}
unsafe impl DwgRaw for libredwg_sys::Dwg_HATCH_Color {}
unsafe impl DwgRaw for libredwg_sys::Dwg_MLINE_vertex {}
unsafe impl DwgRaw for libredwg_sys::Dwg_MLINESTYLE_line {}

/// A DWG 3D point/vector field (BITCODE_3BD, BE, ...): plain C structs of
/// 3 `double`s with no padding. Converted into the model's own [`Point3D`]
/// at the boundary -- the model type carries no layout promise.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RawPoint3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

// SAFETY: `#[repr(C)]`, three `double`s, matching BITCODE_3BD.
unsafe impl DwgRaw for RawPoint3D {}

impl From<RawPoint3D> for Point3D {
    fn from(p: RawPoint3D) -> Self {
        Point3D {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

/// A DWG 2D point field (BITCODE_2RD, 2BD, 2DPOINT, ...): plain C structs
/// of 2 `double`s -- 2RD (raw) and 2BD (bitcode-compressed on disk) are
/// identical once decoded into memory, so one Rust type covers both.
/// Converted into the model's own [`Point2D`] at the boundary.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RawPoint2D {
    pub x: f64,
    pub y: f64,
}

// SAFETY: `#[repr(C)]`, two `double`s, matching BITCODE_2RD.
unsafe impl DwgRaw for RawPoint2D {}

impl From<RawPoint2D> for Point2D {
    fn from(p: RawPoint2D) -> Self {
        Point2D { x: p.x, y: p.y }
    }
}

/// [`get_field`] for a 3D point field, handed back as the model's type.
pub fn get_point3d(entity: *mut c_void, dxfname: &str, field: &str) -> Option<Point3D> {
    get_field::<RawPoint3D>(entity, dxfname, field).map(Point3D::from)
}

/// [`get_field`] for a 2D point field, handed back as the model's type.
pub fn get_point2d(entity: *mut c_void, dxfname: &str, field: &str) -> Option<Point2D> {
    get_field::<RawPoint2D>(entity, dxfname, field).map(Point2D::from)
}

/// [`get_array_field`] for an array of 3D points, as the model's type.
pub fn get_point3d_array<C: DwgRaw + TryInto<usize>>(
    entity: *mut c_void,
    dxfname: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<Point3D> {
    get_array_field::<C, RawPoint3D>(entity, dxfname, count_field, array_field)
        .into_iter()
        .map(Point3D::from)
        .collect()
}

/// [`get_array_field`] for an array of 2D points, as the model's type.
pub fn get_point2d_array<C: DwgRaw + TryInto<usize>>(
    entity: *mut c_void,
    dxfname: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<Point2D> {
    get_array_field::<C, RawPoint2D>(entity, dxfname, count_field, array_field)
        .into_iter()
        .map(Point2D::from)
        .collect()
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

// SAFETY: `#[repr(C)]`, mirrors Dwg_SPLINE_control_point (size pinned above).
unsafe impl DwgRaw for SplineControlPoint {}

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
pub fn get_field<T: DwgRaw>(entity: *mut c_void, dxfname: &str, field: &str) -> Option<T> {
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
pub fn get_common_field<T: DwgRaw>(entity: *mut c_void, field: &str) -> Option<T> {
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

/// Resolves a `BITCODE_H` handle reference (e.g. an entity's `layer`
/// field) to the name bytes of the object it points at (undecoded -- see
/// [`get_text_bytes`]), via
/// `dwg_dynapi_handle_name`. Returns `None` for a null handle or an object
/// with no name field (not every handle target has one).
pub fn handle_name_bytes(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<Vec<u8>> {
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
    // SAFETY: name_ptr is a valid NUL-terminated C string per dynapi's contract.
    let owned = unsafe { CStr::from_ptr(name_ptr) }.to_bytes().to_vec();
    if alloced != 0 {
        // SAFETY: alloced != 0 means dwg_dynapi_handle_name malloc'd this
        // buffer itself (documented in dwg_api.h); ours to free.
        unsafe { libc::free(name_ptr.cast()) };
    }
    Some(owned)
}

/// `true` when the drawing was read from a pre-R13 source (DWG R1.4 .. R12,
/// or a DXF stamped so). Such a drawing points at its tables by index rather
/// than by handle -- see [`resolve_table_entry_name`].
pub fn is_pre_r13(dwg: *mut libredwg_sys::Dwg_Data) -> bool {
    if dwg.is_null() {
        return false;
    }
    // SAFETY: dwg is a live Dwg_Data (caller contract, same as the rest of
    // this crate's conversion pass); the shim null-checks it again itself.
    unsafe { libredwg_sys::uncad_dwg_is_pre_r13(dwg) != 0 }
}

/// `true` when the drawing was read from an R2010-or-later DWG. The
/// library's LEADER record layout loses its place in such a file after the
/// annotation offset, so the fields it reads past that point -- the
/// arrowhead flag among them -- are not what the file says.
pub fn is_r2010_or_later(dwg: *mut libredwg_sys::Dwg_Data) -> bool {
    if dwg.is_null() {
        return false;
    }
    // SAFETY: dwg is a live Dwg_Data (caller contract, same as the rest of
    // this crate's conversion pass); the shim null-checks it again itself.
    unsafe { libredwg_sys::uncad_dwg_is_r2010_or_later(dwg) != 0 }
}

/// `true` when the drawing was read from an R2013-or-later DWG. Only from
/// that version does a SPLINE record carry `splineflags` itself; before it
/// the library fills the field in from the record's form.
pub fn is_r2013_or_later(dwg: *mut libredwg_sys::Dwg_Data) -> bool {
    if dwg.is_null() {
        return false;
    }
    // SAFETY: as `is_r2010_or_later`.
    unsafe { libredwg_sys::uncad_dwg_is_r2013_or_later(dwg) != 0 }
}

/// Resolves a table reference to the name of the entry it points at in
/// `table` (`LAYER`, `BLOCK`, `LTYPE`, ...) via `dwg_handle_name`. For a
/// pre-R13 drawing the library matches the reference's `r11_idx` against the
/// table's entry order, since such references carry no handle; from R13 on it
/// matches the handle. Returns `None` when there is no such table or entry.
/// The library always hands back a copy, freed here once it has been read.
pub fn table_entry_name_bytes(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle: *mut libredwg_sys::Dwg_Object_Ref,
    table: &CStr,
) -> Option<Vec<u8>> {
    if dwg.is_null() || handle.is_null() {
        return None;
    }
    // SAFETY: dwg is a live Dwg_Data and handle a non-null Dwg_Object_Ref it
    // owns (caller contract); table is a NUL-terminated C string.
    let name_ptr = unsafe { libredwg_sys::dwg_handle_name(dwg, table.as_ptr(), handle) };
    if name_ptr.is_null() {
        return None;
    }
    // SAFETY: name_ptr is a NUL-terminated string dwg_handle_name allocated
    // for its caller (every non-NULL return is a strdup or a fresh utf8text
    // conversion); ours to free once copied.
    let owned = unsafe { CStr::from_ptr(name_ptr) }.to_bytes().to_vec();
    unsafe { libc::free(name_ptr.cast()) };
    Some(owned)
}

/// Reads a text field (`BITCODE_T`/`TV`/`TU`) as the bytes LibreDWG holds
/// for it, via `dwg_dynapi_entity_utf8text`. For an R2007+ drawing those
/// bytes are UTF-8 (the library converts its UTF-16 strings); for an older
/// one they are the file's own 8-bit bytes in the drawing's codepage, which
/// the function does not decode despite its name. [`crate::text::TextDecoder`]
/// is what turns either into a `String`; nothing else reads text fields.
/// Returns `None` if the field doesn't exist or is a null string.
///
/// The C function may return a freshly `malloc`'d buffer (r2007+ conversion
/// path) or a pointer straight into the parsed `Dwg_Data` (older formats) --
/// `isnew` tells us which. We always copy before returning, and `free()` the
/// malloc'd buffer ourselves in the `isnew` case so this doesn't leak one
/// string per TEXT/MTEXT/... field read for the lifetime of the process.
pub fn get_text_bytes(entity: *mut c_void, dxfname: &str, field: &str) -> Option<Vec<u8>> {
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

    // SAFETY: text_ptr is a valid, NUL-terminated C string per dynapi's
    // contract (checked non-null above).
    let owned = unsafe { CStr::from_ptr(text_ptr) }.to_bytes().to_vec();

    if is_new != 0 {
        // SAFETY: is_new != 0 means dwg_dynapi_entity_utf8text malloc'd
        // this buffer itself (documented in dwg_api.h); it's ours to free
        // and nothing else holds a reference to it.
        unsafe { libc::free(text_ptr.cast()) };
    }

    Some(owned)
}

/// Reads a plain-old-data header variable (`INSUNITS`, `EXTMIN`,
/// `DIMSCALE`, ...) via `dwg_dynapi_header_value`, with the same up-front
/// size check as [`get_field`] (through `dwg_dynapi_header_field`). Returns
/// `None` for an unknown variable name or a `T` of the wrong size -- never
/// for a variable the file did not state, which reads as whatever the
/// reader left there (see `crate::header`).
pub fn get_header_field<T: DwgRaw>(dwg: *const libredwg_sys::Dwg_Data, name: &str) -> Option<T> {
    if dwg.is_null() {
        return None;
    }
    let c_name = CString::new(name).expect("variable name has no interior NUL");
    // SAFETY: pure name -> descriptor lookup, no write through any pointer.
    let field_desc = unsafe { libredwg_sys::dwg_dynapi_header_field(c_name.as_ptr()) };
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, "<header>", name) {
        return None;
    }
    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();
    // SAFETY: dwg is live (caller contract); out is sized for T and the size
    // check above confirms dynapi writes exactly size_of::<T>() bytes.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_header_value(
            dwg,
            c_name.as_ptr(),
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

/// Reads a text header variable (`DIMPOST`, ...) as the bytes LibreDWG holds
/// for it, via `dwg_dynapi_header_utf8text` -- the same contract as
/// [`get_text_bytes`]. Returns `None` for an unknown name or a null string.
pub fn get_header_text_bytes(dwg: *const libredwg_sys::Dwg_Data, name: &str) -> Option<Vec<u8>> {
    if dwg.is_null() {
        return None;
    }
    let c_name = CString::new(name).expect("variable name has no interior NUL");
    let mut text_ptr: *mut std::os::raw::c_char = std::ptr::null_mut();
    let mut is_new: std::os::raw::c_int = 0;
    // SAFETY: dwg is live (caller contract); text_ptr/is_new are valid
    // out-params for the duration of the call.
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_header_utf8text(
            dwg,
            c_name.as_ptr(),
            &mut text_ptr,
            &mut is_new,
            std::ptr::null_mut(),
        )
    };
    if !ok || text_ptr.is_null() {
        return None;
    }
    // SAFETY: text_ptr is a valid, NUL-terminated C string per dynapi's
    // contract (checked non-null above).
    let owned = unsafe { CStr::from_ptr(text_ptr) }.to_bytes().to_vec();
    if is_new != 0 {
        // SAFETY: as in get_text_bytes -- a malloc'd copy that is ours.
        unsafe { libc::free(text_ptr.cast()) };
    }
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
pub fn get_array_field<C: DwgRaw + TryInto<usize>, T: DwgRaw>(
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
