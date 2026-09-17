//! Safe DWG/DXF parsing on top of `libredwg-sys`, plus JSON/SVG/PNG export
//! of the parsed model.
//!
//! Reading only -- this crate does not write DWG or DXF. The shape is
//! `DWG/DXF -> CadDatabase (entities + tables) -> to_json() | to_svg() |
//! to_png()`.

mod acis;
pub mod color;
mod convert;
mod dynapi;
pub mod json;
pub mod model;
pub mod png;
pub mod svg;
pub mod tables;

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub use json::{JsonError, ToJsonOptions};
pub use model::Entity;
pub use png::{PngError, ToPngOptions, ToPngResult};
pub use svg::{Space, ToSvgOptions, ToSvgResult};
pub use tables::Tables;

/// LibreDWG's C code has non-reentrant global state (the `loglevel` global
/// read and written throughout `decode.c`/`bits.c`, and likely more).
/// Calling into it concurrently reliably produced `STATUS_HEAP_CORRUPTION`
/// under `cargo test`'s parallel runner once two threads' read/convert/free
/// cycles overlapped. Sequential reuse across many calls is fine; concurrent
/// calls are what this lock closes off.
///
/// [`parse`] is the only entry point that touches the FFI boundary and it
/// holds this lock for its whole duration, so callers never need to know
/// `libredwg-sys` exists, let alone serialize around it themselves.
static LIBREDWG_LOCK: Mutex<()> = Mutex::new(());

/// A parsed CAD drawing: the model, and nothing else.
///
/// `entities` holds what the drawing shows (everything owned by the
/// `*Model_Space`/`*Paper_Space*` blocks, see [`crate::model`]) and `tables`
/// the LAYER / BLOCK_RECORD / MLINESTYLE tables it resolves against. This is
/// what [`to_json`](Self::to_json) serializes verbatim and what
/// [`to_svg`](Self::to_svg)/[`to_png`](Self::to_png) render from.
///
/// It is a plain Rust value: LibreDWG's own `Dwg_Data` is freed inside
/// [`parse`] as soon as these two fields have been built from it, so a
/// `CadDatabase` owns no C memory, is `Clone`/`PartialEq`/`Send`/`Sync`
/// without ceremony, and can be constructed directly or deserialized from
/// the JSON `to_json` produced. It is deliberately *not* a round-trip
/// representation of the file (no linetypes, lineweights, styles,
/// dictionaries, header variables...): the model keeps the fields rendering
/// needs, and this crate has no write path that would need more.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CadDatabase {
    pub entities: Vec<Entity>,
    pub tables: Tables,
}

impl CadDatabase {
    /// Serializes this parsed drawing (`entities` + `tables`) to JSON text --
    /// see the [`json`] module doc for the exact shape.
    pub fn to_json(&self, options: ToJsonOptions) -> Result<String, JsonError> {
        json::to_json(self, options)
    }

    /// Renders this parsed drawing to SVG. `&self`, so the same database can
    /// be rendered repeatedly with different options (e.g. once per
    /// [`Space`]).
    pub fn to_svg(&self, options: ToSvgOptions) -> ToSvgResult {
        svg::to_svg(self, options)
    }

    /// Renders this parsed drawing straight to PNG bytes, via
    /// [`to_svg`](Self::to_svg) internally -- the intermediate SVG text never
    /// touches disk.
    pub fn to_png(&self, options: ToPngOptions) -> Result<ToPngResult, PngError> {
        png::to_png(self, options)
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

/// Parses a DWG or DXF file at `path` into a [`CadDatabase`] -- the same
/// model either way, with the format inferred from the `.dwg`/`.dxf`
/// extension.
///
/// How complete DXF reading is depends on the entity type: `dxf_read_file()`
/// is LibreDWG's own function and its documentation describes DXF reading as
/// working "for most objects" rather than being feature-complete the way DWG
/// reading is. See `docs/CAVEATS.md`.
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError> {
    let path_str = path.as_ref().to_str().ok_or(ParseError::InvalidPath)?;
    let c_path = CString::new(path_str).map_err(|_| ParseError::InvalidPath)?;
    let is_dxf = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf"));

    // See LIBREDWG_LOCK: the whole read/convert/free cycle must run without
    // another thread's LibreDWG call interleaved. Recovering from a poisoned
    // lock rather than propagating the poison is deliberate -- a panic here
    // would be a bug in this crate's own conversion code, not evidence that
    // the C library's global state is corrupted, so bricking every future
    // parse() over one panic would be the worse trade.
    let _guard = LIBREDWG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // SAFETY: Dwg_Data is bound as an opaque, correctly-sized byte blob (see
    // libredwg-sys's build.rs); dwg_read_file/dxf_read_file expect a
    // zero-initialized instance -- an uninitialized one aborts with
    // STATUS_STACK_BUFFER_OVERRUN (garbage in dwg.opts feeding the runtime
    // loglevel global). Boxed so the C side fills it in place at a stable
    // heap address; it lives only until the two conversion walks below have
    // copied out everything this crate exposes.
    let mut dwg: Box<libredwg_sys::Dwg_Data> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });

    let error = if is_dxf {
        unsafe { libredwg_sys::dxf_read_file(c_path.as_ptr(), dwg.as_mut()) }
    } else {
        unsafe { libredwg_sys::dwg_read_file(c_path.as_ptr(), dwg.as_mut()) }
    };
    // Cast the constant, not `error`: the DWG_ERROR enum constants' width is
    // whatever bindgen inferred for the target (u32 on Linux, i32 on MSVC --
    // see convert.rs's DWG_OBJECT_TYPE comment), while `error` itself comes
    // from a plain `int` C return type and is stable. Casting this way keeps
    // ParseError::Critical's public i32 stable too. clippy sees a no-op cast
    // on MSVC; on Linux removing it fails the build.
    #[allow(clippy::unnecessary_cast)]
    if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
        return Err(ParseError::Critical(error));
    }

    // Two walks over the live C structure, neither of which mutates it.
    // Everything the returned value exposes is an owned Rust copy by the end.
    let entities = unsafe { convert::convert_entities(dwg.as_mut()) };
    let tables = unsafe { tables::convert_tables(dwg.as_mut()) };

    // Nothing needs LibreDWG's structure past this point: this crate has no
    // write path, and the model above is what every export reads. Freeing it
    // here, still under the lock, is what keeps CadDatabase a plain value (no
    // Drop, no C memory, Send + Sync by construction).
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };

    Ok(CadDatabase { entities, tables })
}
