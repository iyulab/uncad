//! Safe(r) access to LibreDWG's dynapi reflection API
//! (`dwg_dynapi_entity_value`/`dwg_dynapi_entity_field`): read a named
//! field off an entity struct by string name, the same pattern the
//! existing JS converter already relies on for every entity type.
//!
//! `Dwg_Object`/`Dwg_Object_Entity`/`Dwg_Entity_*` are all bound as opaque
//! blobs (see libredwg-sys's build.rs) precisely so that this module is the
//! *only* place that ever touches a raw entity pointer -- callers get typed
//! Rust values back, never a struct to poke at directly.

use std::ffi::{c_void, CStr, CString};
use std::mem::MaybeUninit;

/// A DWG 3D point/vector field (BITCODE_3BD, BE, ...): plain C structs of
/// 3 `double`s with no padding.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A DWG 2D point field (BITCODE_2RD, 2BD, 2DPOINT, ...): plain C structs
/// of 2 `double`s -- 2RD (raw) and 2BD (bitcode-compressed on disk) are
/// identical once decoded into memory, so one Rust type covers both.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

/// `Dwg_SPLINE_control_point`'s exact layout: a leading `parent` pointer
/// (back to the owning `Dwg_Entity_SPLINE`, per dwg.h -- `struct
/// _dwg_SPLINE_control_point { struct _dwg_entity_SPLINE *parent; double
/// x, y, z, w; }`) followed by 4 doubles (x, y, z, weight).
///
/// The `parent` field isn't a `get_array_field`/dynapi size-check concern
/// (that only validates the *pointer-to-the-array* field's own size, 8
/// bytes either way, never the element stride) -- it matters because
/// `get_array_field` computes each element's address as `base_ptr +
/// i * size_of::<T>()`. Omitting `parent` here (an earlier version of this
/// struct only had x/y/z/w, 32 bytes) understates the real 40-byte
/// element stride, so every control point after the first was read at a
/// progressively larger offset than its real location -- silently
/// reinterpreting 8 bytes of the *next* real control point's `parent`
/// pointer as this point's leading coordinate field, cascading further
/// out of alignment with each subsequent element. Confirmed on a real
/// file (`samples/AutoCADSamples1.dwg`, a SPLINE-heavy architectural
/// drawing): rendered SPLINE entities with only control points (no fit
/// points) produced wild, drawing-spanning zigzag lines, and separately
/// explains a batch of subnormal-magnitude coordinates (a `parent`
/// pointer value bit-reinterpreted as `f64` is often subnormal) that had
/// been worked around at the *formatting* layer in svg.rs before this was
/// traced back to its real cause here.
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

// Pins the element stride get_array_field::<_, SplineControlPoint> uses to
// the real Dwg_SPLINE_control_point size (pointer + 4 doubles, 64-bit) --
// see the struct's doc comment for the silent-misalignment bug this
// guards against recurring.
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

/// Returns whether `dwg_dynapi_*_value` writing into a `MaybeUninit<T>`
/// would fit -- it `memcpy`s `f.is_malloc() != 0 ? sizeof(char*) : f.size`
/// bytes (`dynapi.c`'s own rule, mirrored here), entirely independent of
/// whatever Rust type the caller chose. Panics in debug builds (turning a
/// wrong-`T` call site into an immediate test failure, same as before) and
/// returns `false` in release builds so the caller can refuse the read
/// instead of letting dynapi overrun a too-small buffer.
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
/// The JS binding's equivalent (`dwg_dynapi_entity_data<T>(obj, field)`)
/// does an unchecked `as T` cast with no runtime check that `T` actually
/// matches the field's real C type. Rust has no safe analogue of that, so
/// this first calls the read-only `dwg_dynapi_entity_field` lookup (name ->
/// field descriptor, no write) to check the field's real size *before*
/// calling `dwg_dynapi_entity_value`, which unconditionally `memcpy`s that
/// many bytes into `out` regardless of `T` -- checking only *after* that
/// call (an earlier version of this function used `debug_assert_eq!` post-
/// call) still lets a release build's missing debug assertions overrun a
/// too-small `MaybeUninit<T>`, since the C-side write already happened by
/// the time Rust gets to check anything.
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
    // SAFETY: name_ptr is a valid NUL-terminated C string per dynapi's contract.
    let owned = unsafe { CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .into_owned();
    if alloced != 0 {
        // SAFETY: alloced != 0 means dwg_dynapi_handle_name malloc'd this
        // buffer itself (documented in dwg_api.h); ours to free.
        unsafe { libc::free(name_ptr.cast()) };
    }
    Some(owned)
}

/// Reads a text field (BITCODE_T/TV/TU) as a UTF-8 `String`, via
/// `dwg_dynapi_entity_utf8text` -- which itself handles the r2007+
/// UTF-16-wide-string-to-UTF-8 conversion (older DWGs store text fields as
/// plain 8-bit strings already). Returns `None` if the field doesn't exist
/// or is a null string.
///
/// The C function may return a freshly `malloc`'d buffer (r2007+ conversion
/// path) or a pointer straight into the parsed `Dwg_Data` (older formats) --
/// `isnew` tells us which. We always copy into an owned Rust `String`
/// before returning, and `free()` the malloc'd buffer ourselves in the
/// `isnew` case so this doesn't leak one string per TEXT/MTEXT/... field
/// read for the lifetime of the process.
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

    // SAFETY: text_ptr is a valid, NUL-terminated C string per dynapi's
    // contract (checked non-null above).
    let owned = unsafe { CStr::from_ptr(text_ptr) }
        .to_string_lossy()
        .into_owned();

    if is_new != 0 {
        // SAFETY: is_new != 0 means dwg_dynapi_entity_utf8text malloc'd
        // this buffer itself (documented in dwg_api.h); it's ours to free
        // and nothing else holds a reference to it.
        unsafe { libc::free(text_ptr.cast()) };
    }

    Some(owned)
}

/// Reads a `(count_field, array_field)` pair -- e.g. LWPOLYLINE's
/// `num_points`/`points` -- as an owned `Vec<T>`. The array field is a raw
/// pointer into memory LibreDWG itself owns (freed by `dwg_free`, not by
/// this call), so this copies every element out rather than borrowing.
///
/// `C` is the count field's own C integer type -- `BITCODE_BL` (u32) for
/// most `num_X` fields, but e.g. SPLINE's `num_fit_pts` is `BITCODE_BS`
/// (u16, *not* i16 -- see `docs/CAVEATS.md`'s `BITCODE_BS` note), so this
/// can't hardcode one count width for every caller.
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
