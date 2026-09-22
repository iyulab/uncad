//! Safe DWG/DXF parsing on top of `libredwg-sys`, plus JSON/SVG/PNG export
//! of the parsed model and the LLM/VLM package built from it.
//!
//! Reading only -- this crate does not write DWG or DXF. The shape is
//! `DWG/DXF -> CadDatabase (entities + tables + header) -> to_json() |
//! to_svg() | to_png() | export::export_package()`.

mod acis;
pub mod color;
mod convert;
pub mod crop;
pub mod dimension;
mod dynapi;
pub mod export;
pub mod geom;
pub mod header;
pub mod json;
pub mod limits;
pub mod model;
pub mod png;
pub mod svg;
pub mod tables;
pub mod text;
pub mod visibility;

use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub use crop::{CropMode, CropReport, CropSource, Rect};
pub use export::{ExportError, ExportOptions, ExportReport, Profile};
pub use header::{Header, Units};
pub use json::{JsonError, ToJsonOptions};
pub use limits::LimitReport;
pub use model::Entity;
pub use png::{
    Background, Fonts, PngError, PngSize, ToPngOptions, ToPngResult, BUNDLED_FONT_FAMILY,
};
pub use svg::{Space, ToSvgOptions, ToSvgResult, ViewBox};
pub use tables::Tables;

/// LibreDWG's C code has non-reentrant global state (the `loglevel` global
/// read and written throughout `decode.c`/`bits.c`, and likely more).
/// Calling into it concurrently reliably produced `STATUS_HEAP_CORRUPTION`
/// under `cargo test`'s parallel runner once two threads' read/convert/free
/// cycles overlapped. Sequential reuse across many calls is fine; concurrent
/// calls are what this lock closes off.
///
/// [`parse_bytes`] (which [`parse`] calls) is the only entry point that
/// touches the FFI boundary and it holds this lock for its whole duration, so
/// callers never need to know `libredwg-sys` exists, let alone serialize
/// around it themselves.
static LIBREDWG_LOCK: Mutex<()> = Mutex::new(());

/// A parsed CAD drawing: the model, and nothing else.
///
/// `entities` holds what the drawing shows (everything owned by the
/// `*Model_Space`/`*Paper_Space*` blocks, see [`crate::model`]), `tables`
/// the LAYER / BLOCK_RECORD / MLINESTYLE / DIMSTYLE / LAYOUT records it
/// resolves against, and
/// `header` the file-level facts and header variables (version, code page,
/// `$INSUNITS`, `$EXTMIN`/`$EXTMAX`, `$DIMLFAC`, ...) that give the numbers
/// their meaning. This is what [`to_json`](Self::to_json) serializes verbatim
/// and what [`to_svg`](Self::to_svg)/[`to_png`](Self::to_png) render from.
///
/// It is a plain Rust value: LibreDWG's own `Dwg_Data` is freed inside
/// [`parse_bytes`] as soon as these fields have been built from it, so a
/// `CadDatabase` owns no C memory, is `Clone`/`PartialEq`/`Send`/`Sync`
/// without ceremony, and can be constructed directly (see
/// [`new`](Self::new)) or deserialized from the JSON `to_json` produced --
/// JSON written before `header` existed still loads, with a default header.
/// It is deliberately *not* a round-trip representation of the file (no
/// linetypes, styles, dictionaries, ...): the model keeps what the exports
/// need, and this crate has no write path that would need more.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CadDatabase {
    pub entities: Vec<Entity>,
    pub tables: Tables,
    #[serde(default)]
    pub header: Header,
}

impl CadDatabase {
    /// A database with a default (unitless, versionless) header -- the
    /// 0.2.0 `CadDatabase { entities, tables }` literal, for callers that
    /// build a model by hand.
    pub fn new(entities: Vec<Entity>, tables: Tables) -> Self {
        CadDatabase {
            entities,
            tables,
            header: Header::default(),
        }
    }

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

    /// Renders this parsed drawing straight to PNG bytes -- the intermediate
    /// SVG text never touches disk. By default the image's long edge is 1568
    /// px, the background opaque white and strokes 1.25 px wide; see
    /// [`ToPngOptions`].
    pub fn to_png(&self, options: ToPngOptions) -> Result<ToPngResult, PngError> {
        png::to_png(self, options)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ParseError {
    /// LibreDWG's decoder returned a critical error code (>= DWG_ERR_CRITICAL,
    /// see dwg.h) for the bytes it was given.
    Critical(i32),
    /// The file could not be read from disk ([`parse`] only; [`parse_bytes`]
    /// never produces it).
    Io(std::io::Error),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Critical(code) => write!(f, "LibreDWG critical read error (code {code})"),
            ParseError::Io(e) => write!(f, "cannot read the file: {e}"),
        }
    }
}
impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::Critical(_) => None,
            ParseError::Io(e) => Some(e),
        }
    }
}

/// The on-disk format of a drawing. [`parse`] picks it from the file
/// extension; [`parse_bytes`] takes it explicitly because bytes carry no
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Dwg,
    Dxf,
}

impl Format {
    /// `.dxf` (any case) is [`Format::Dxf`]; anything else is read as DWG,
    /// which is what [`parse`] has always done.
    pub fn from_path(path: impl AsRef<Path>) -> Format {
        let is_dxf = path
            .as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dxf"));
        if is_dxf {
            Format::Dxf
        } else {
            Format::Dwg
        }
    }
}

/// Parses a DWG or DXF file at `path` into a [`CadDatabase`] -- the same
/// model either way, with the format inferred from the `.dwg`/`.dxf`
/// extension (see [`Format::from_path`]).
///
/// The file is read into memory here and decoded by [`parse_bytes`] rather
/// than handed to LibreDWG as a path: LibreDWG opens paths with `fopen()`,
/// which on Windows interprets the bytes in the process's ANSI code page and
/// so cannot open a path with, say, a Korean directory name. `std::fs::read`
/// has no such limit.
///
/// How complete DXF reading is depends on the entity type: LibreDWG's own
/// DXF reader is documented as working "for most objects" rather than being
/// feature-complete the way DWG reading is. Reading a DXF also costs roughly
/// the square of its entity count inside that reader, so a 34 MB one takes
/// minutes. See `docs/CAVEATS.md`.
///
/// # A corrupt drawing can abort the process
///
/// Decoding happens in LibreDWG's C code, and a malformed DWG can terminate
/// the process there instead of returning [`ParseError`] -- an `abort()` or
/// a C runtime fail-fast unwinds nothing, so `catch_unwind` around this call
/// cannot help and none is attempted. **Parse untrusted drawings in a
/// separate process** you can afford to lose. See `docs/CAVEATS.md`, "A
/// corrupt DWG can abort the process below the FFI boundary", for what has
/// been fuzzed and what that does and does not prove.
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError> {
    let path = path.as_ref();
    let format = Format::from_path(path);
    let bytes = std::fs::read(path).map_err(ParseError::Io)?;
    parse_bytes(&bytes, format)
}

/// Parses a drawing already held in memory. This is what [`parse`] calls
/// after reading the file; it is public for callers that receive drawings
/// over the network or from an archive and never have a path.
///
/// Same caveat as [`parse`]: a corrupt DWG can abort the process inside
/// LibreDWG's decoder, below the FFI boundary, where no Rust guard reaches
/// it. Untrusted bytes -- exactly what a caller reading from the network
/// has -- belong in a separate process. See `docs/CAVEATS.md`.
pub fn parse_bytes(bytes: &[u8], format: Format) -> Result<CadDatabase, ParseError> {
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
    // libredwg-sys's build.rs); the readers expect a zero-initialized
    // instance -- an uninitialized one aborts with
    // STATUS_STACK_BUFFER_OVERRUN (garbage in dwg.opts feeding the runtime
    // loglevel global). Boxed so the C side fills it in place at a stable
    // heap address; it lives only until the two conversion walks below have
    // copied out everything this crate exposes.
    let mut dwg: Box<libredwg_sys::Dwg_Data> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });

    // SAFETY: `bytes` is a live slice for the duration of the call and the
    // shims only read it (they copy it into their own Bit_Chain); `dwg` is a
    // valid zeroed Dwg_Data they fill in place.
    let error = match format {
        Format::Dwg => unsafe {
            libredwg_sys::uncad_dwg_read_bytes(bytes.as_ptr(), bytes.len(), dwg.as_mut())
        },
        Format::Dxf => unsafe {
            libredwg_sys::uncad_dxf_read_bytes(bytes.as_ptr(), bytes.len(), dwg.as_mut())
        },
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

    // Three walks over the live C structure, none of which mutates it.
    // Everything the returned value exposes is an owned Rust copy by the end.
    let header = unsafe { header::convert_header(dwg.as_mut()) };
    let entities = unsafe { convert::convert_entities(dwg.as_mut()) };
    let tables = unsafe { tables::convert_tables(dwg.as_mut()) };

    // Nothing needs LibreDWG's structure past this point: this crate has no
    // write path, and the model above is what every export reads. Freeing it
    // here, still under the lock, is what keeps CadDatabase a plain value (no
    // Drop, no C memory, Send + Sync by construction).
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };

    let mut db = CadDatabase {
        entities,
        tables,
        header,
    };
    // Needs the tables (cached labels, DIMSTYLEs) and the header together.
    dimension::attach_display_text(&mut db);
    Ok(db)
}
