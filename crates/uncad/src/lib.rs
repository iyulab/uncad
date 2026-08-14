//! Safe DWG/DXF parsing on top of `libredwg-sys`.
//!
//! Rust port of `src/index.mjs`'s `parse()`/`toSVG()`/`dwgToDxf()`.

mod acis;
pub mod color;
mod convert;
mod dynapi;
pub mod png;
pub mod render_model;
pub mod svg;
pub mod tables;

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;

pub use png::{PngError, ToPngOptions, ToPngResult};
pub use render_model::RenderEntity;
pub use svg::{Space, ToSvgOptions, ToSvgResult};
pub use tables::Tables;

/// LibreDWG's C code has non-reentrant global state (confirmed: the
/// `loglevel` global read/written throughout `decode.c`/`bits.c`/etc, and
/// likely more) -- calling into it concurrently from multiple threads
/// reliably produced `STATUS_HEAP_CORRUPTION` under `cargo test`'s default
/// parallel test runner once the object walk did enough work per call for
/// two threads' read/convert/free cycles to overlap (Phase 0 only verified
/// *sequential* reuse across many calls was safe -- a different property
/// from *concurrent* calls, and this is where that gap showed up). Every
/// entry point that touches the FFI boundary takes this lock for its
/// entire duration, so `uncad::parse()` is safe to call from multiple
/// threads even though the underlying C library isn't -- callers don't
/// need to know libredwg-sys exists, let alone serialize around it
/// themselves.
static LIBREDWG_LOCK: Mutex<()> = Mutex::new(());

/// A parsed CAD drawing. Two layers live here, deliberately kept separate
/// (see `docs/ARCHITECTURE.md`'s "two-layer model" section):
/// - `entities`/`tables`: a lossy, rendering-oriented Rust projection (see
///   [`crate::render_model`]) -- only the fields `to_svg()` needs, nothing
///   else. This is what [`CadDatabase::to_svg`] reads.
/// - `dwg` (private): the actual `Dwg_Data` LibreDWG populated from the
///   source file, kept alive instead of freed immediately -- a genuinely
///   complete, round-trip-faithful representation, since it's LibreDWG's
///   own native structure. [`CadDatabase::write_dwg`]/[`write_dxf`] write
///   *this*, not a re-derivation from `entities`/`tables` (which couldn't
///   round-trip -- they're missing everything not needed for rendering).
pub struct CadDatabase {
    pub entities: Vec<RenderEntity>,
    pub tables: Tables,
    dwg: Box<libredwg_sys::Dwg_Data>,
}

impl std::fmt::Debug for CadDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dwg` deliberately omitted -- it's an opaque 676-`u64` blob with
        // no useful `Debug` output of its own.
        f.debug_struct("CadDatabase")
            .field("entities", &self.entities)
            .field("tables", &self.tables)
            .finish_non_exhaustive()
    }
}

impl Drop for CadDatabase {
    fn drop(&mut self) {
        // See LIBREDWG_LOCK's doc comment.
        let _guard = LIBREDWG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe { libredwg_sys::dwg_free(self.dwg.as_mut()) };
    }
}

impl CadDatabase {
    /// Renders this parsed drawing to SVG. A method rather than a free
    /// function taking `&CadDatabase` -- this genuinely operates on `self`
    /// (unlike [`dwg_to_dxf`], see its doc comment), so it belongs on the
    /// type (`&self`, not consuming, since the same parsed database can be
    /// rendered multiple times with different options -- e.g. once per
    /// `Space`).
    pub fn to_svg(&self, options: ToSvgOptions) -> ToSvgResult {
        svg::to_svg(self, options)
    }

    /// Renders this parsed drawing straight to PNG bytes, via [`to_svg`]
    /// internally (see [`png`] module doc comment for the SVG -> PNG raster
    /// step) -- the intermediate SVG text never touches disk.
    ///
    /// [`to_svg`]: Self::to_svg
    pub fn to_png(&self, options: ToPngOptions) -> Result<ToPngResult, PngError> {
        png::to_png(self, options)
    }

    /// Writes this drawing back out as a DWG file at `path`, using
    /// LibreDWG's own encoder (`dwg_write_file`) on the live `Dwg_Data`
    /// `self` has kept alive since `parse()` -- not a re-derivation from
    /// `entities`/`tables`, which are a deliberately lossy rendering
    /// projection (see [`CadDatabase`]'s own doc comment).
    ///
    /// **Reliable only for DWG version <= R_2004.** LibreDWG's own `README`:
    /// "Write support only works for earlier versions until r2004...
    /// Rewriting most DWG's <= r2004 usually works fine." Newer source
    /// files may fail to encode correctly or at all -- this is an upstream
    /// LibreDWG boundary, not something this project can widen. (Verified
    /// against this project's own 9 real-file spot-check set: all 9 are
    /// `AC1018`/R_2004, squarely inside the supported range -- see
    /// `docs/CAVEATS.md`.)
    ///
    /// **Refuses to overwrite an existing file.** `dwg_write_file` itself
    /// `stat()`s `path` first and returns a critical error
    /// (`DWG_ERR_IOERROR`, surfaced as [`WriteError::Critical`]) if
    /// anything is already there, rather than silently clobbering it --
    /// this wrapper doesn't work around that; delete the target yourself
    /// first if you want to replace it.
    ///
    /// Takes `&mut self`, not `&self`, even though `dwg_write_file`'s own C
    /// signature claims `const Dwg_Data*` -- reading LibreDWG's `dwg.c`
    /// shows its implementation casts that const away and mutates
    /// internally (`dwg_encode((Dwg_Data*)dwg, &dat)`), so trusting the
    /// public signature's claim here would be unsound.
    pub fn write_dwg(&mut self, path: impl AsRef<Path>) -> Result<(), WriteError> {
        let path_str = path.as_ref().to_str().ok_or(WriteError::InvalidPath)?;
        let c_path = CString::new(path_str).map_err(|_| WriteError::InvalidPath)?;

        // See LIBREDWG_LOCK's doc comment.
        let _guard = LIBREDWG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let error = unsafe { libredwg_sys::dwg_write_file(c_path.as_ptr(), self.dwg.as_ref()) };
        // See the cast comment on the same comparison in parse() below.
        #[allow(clippy::unnecessary_cast)]
        if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
            return Err(WriteError::Critical(error));
        }
        Ok(())
    }

    /// Writes this drawing back out as DXF text at `path`, via
    /// `libredwg-sys`'s `uncad_write_dxf` shim operating on the same live
    /// `Dwg_Data` [`write_dwg`](Self::write_dwg) uses -- **not** the same
    /// code path as the free function [`dwg_to_dxf`], which re-reads its
    /// own source file from scratch and never touches a `CadDatabase` at
    /// all. Prefer this method when you've already `parse()`d the file (no
    /// point re-reading it from disk); prefer the free function when you
    /// only want a pure file-to-file conversion and don't need a
    /// `CadDatabase` for anything else (skips building `entities`/`tables`
    /// entirely).
    pub fn write_dxf(&mut self, path: impl AsRef<Path>) -> Result<(), WriteError> {
        let path_str = path.as_ref().to_str().ok_or(WriteError::InvalidPath)?;
        let c_path = CString::new(path_str).map_err(|_| WriteError::InvalidPath)?;

        // See LIBREDWG_LOCK's doc comment.
        let _guard = LIBREDWG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let error = unsafe { libredwg_sys::uncad_write_dxf(self.dwg.as_mut(), c_path.as_ptr()) };
        // See the cast comment on the same comparison in parse() below.
        #[allow(clippy::unnecessary_cast)]
        if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
            return Err(WriteError::Critical(error));
        }
        Ok(())
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ParseError {
    /// `dwg_read_file`/`dxf_read_file` returned a critical LibreDWG error
    /// code (>= DWG_ERR_CRITICAL, see dwg.h).
    Critical(i32),
    InvalidPath,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Critical(code) => write!(f, "LibreDWG critical read error (code {code})"),
            ParseError::InvalidPath => write!(f, "path is not valid UTF-8 / contains a NUL byte"),
        }
    }
}
impl std::error::Error for ParseError {}

/// Parses a DWG or DXF file at `path` into a [`CadDatabase`] (same shape
/// regardless of source format) -- format is inferred from the `.dwg`/
/// `.dxf` extension, matching the JS baseline's `detectDwgFileType()`.
///
/// DXF reading is entity-type-dependent -- `dxf_read_file()` is LibreDWG's
/// own function, not something this port controls, and its own
/// documentation describes DXF reading as working "for most objects"
/// rather than being feature-complete like DWG reading (confirmed in the
/// JS-era testing: LWPOLYLINE was silently dropped from a real `.dxf` file
/// whose `ENTITIES` section clearly contained it, even though ARC/ELLIPSE
/// parsed fine). Not something to patch around here; see docs/CAVEATS.md.
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError> {
    let path_str = path.as_ref().to_str().ok_or(ParseError::InvalidPath)?;
    let c_path = CString::new(path_str).map_err(|_| ParseError::InvalidPath)?;
    let is_dxf = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf"));

    // See LIBREDWG_LOCK's doc comment: the whole read/convert/free cycle
    // must run without another thread's LibreDWG call interleaved.
    // Recovering from a poisoned lock (rather than propagating the
    // poison) is deliberate: a panic here would be a bug in this crate's
    // Rust-side conversion code, not evidence the C library's global state
    // itself is corrupted, so permanently bricking every future parse()
    // call over one panic would be worse than the small risk of
    // continuing.
    let _guard = LIBREDWG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // SAFETY: Dwg_Data is bound as an opaque, correctly-sized byte blob
    // (see libredwg-sys build.rs); dwg_read_file/dxf_read_file expect a
    // zero-initialized instance -- an uninitialized one was confirmed
    // during Phase 0 to produce a STATUS_STACK_BUFFER_OVERRUN (garbage in
    // dwg.opts feeding the runtime loglevel global). Boxed (rather than a
    // stack local moved into the box afterward) so dwg_read_file/
    // dxf_read_file fill it in place at its final, stable heap address --
    // this is the same `Dwg_Data` CadDatabase keeps alive afterward for
    // write_dwg/write_dxf, not freed here the way earlier versions of this
    // function did.
    let mut dwg: Box<libredwg_sys::Dwg_Data> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });

    let error = if is_dxf {
        unsafe { libredwg_sys::dxf_read_file(c_path.as_ptr(), dwg.as_mut()) }
    } else {
        unsafe { libredwg_sys::dwg_read_file(c_path.as_ptr(), dwg.as_mut()) }
    };
    // `error`'s width (from dwg_read_file/dxf_read_file's plain-`int` C
    // return type) is stable across platforms, unlike the DWG_ERROR enum
    // constants' bindgen-inferred width (`u32` on Linux vs `i32` on the
    // MSVC target -- see convert.rs's DWG_OBJECT_TYPE comment for the same
    // underlying bindgen behavior) -- cast the constant, not `error`, so
    // `ParseError::Critical`'s public `i32` stays stable too. clippy only
    // flags this as a no-op cast on the MSVC target this was last built on
    // (where the constant is already `i32`); on Linux it's load-bearing --
    // removing it breaks the build there with an i32/u32 comparison
    // mismatch, so this is a real cross-platform necessity, not a stray
    // cast to silence.
    #[allow(clippy::unnecessary_cast)]
    if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
        return Err(ParseError::Critical(error));
    }

    let entities = unsafe { convert::convert_entities(dwg.as_mut()) };
    let tables = unsafe { tables::convert_tables(dwg.as_mut()) };

    Ok(CadDatabase {
        entities,
        tables,
        dwg,
    })
}

/// Shared by [`dwg_to_dxf`] and [`CadDatabase::write_dwg`]/
/// [`CadDatabase::write_dxf`] -- the shape (a critical LibreDWG error code,
/// an unusable path, or a plain I/O error) is entirely format-agnostic, so
/// one type covers writing DWG or DXF from either entry point.
#[derive(Debug)]
#[non_exhaustive]
pub enum WriteError {
    /// The underlying LibreDWG write call returned a critical error code
    /// (`dwg_write_file`/`uncad_write_dxf`/`uncad_write_dxf_file`).
    Critical(i32),
    InvalidPath,
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Critical(code) => {
                write!(f, "LibreDWG critical write error (code {code})")
            }
            WriteError::InvalidPath => write!(f, "path is not valid UTF-8 / contains a NUL byte"),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for WriteError {}

/// Converts a DWG file at `dwg_path` to DXF text, written to `dxf_path`,
/// re-reading `dwg_path` from scratch rather than going through an already-
/// `parse()`d [`CadDatabase`] -- unlike [`CadDatabase::write_dxf`], this
/// never builds `entities`/`tables` at all, so prefer this free function
/// for a pure file-to-file conversion when you don't otherwise need a
/// `CadDatabase`; prefer the method when you've already parsed the file.
///
/// Uses `libredwg-sys`'s `uncad_write_dxf_file` shim (see its doc comment)
/// rather than an in-memory buffer -- `dwg_write_dxf` itself is
/// file-stream-based (it takes a `Bit_Chain` wrapping a `FILE*`), matching
/// how the JS `dwgToDxf()` -> `dwg_write_dxf` WASM binding already worked.
pub fn dwg_to_dxf(
    dwg_path: impl AsRef<Path>,
    dxf_path: impl AsRef<Path>,
) -> Result<(), WriteError> {
    let dwg_str = dwg_path.as_ref().to_str().ok_or(WriteError::InvalidPath)?;
    let dxf_str = dxf_path.as_ref().to_str().ok_or(WriteError::InvalidPath)?;
    let c_dwg = CString::new(dwg_str).map_err(|_| WriteError::InvalidPath)?;
    let c_dxf = CString::new(dxf_str).map_err(|_| WriteError::InvalidPath)?;

    // See LIBREDWG_LOCK's doc comment.
    let _guard = LIBREDWG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let error = unsafe { libredwg_sys::uncad_write_dxf_file(c_dwg.as_ptr(), c_dxf.as_ptr()) };
    // See the cast comment on the same comparison in parse() above.
    #[allow(clippy::unnecessary_cast)]
    if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        return Err(WriteError::Critical(error));
    }
    Ok(())
}
