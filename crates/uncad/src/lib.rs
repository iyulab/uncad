//! Safe DWG/DXF parsing on top of `libredwg-sys`, into the shared
//! [`uncad_model`] entity model.
//!
//! Reading only -- this crate does not write DWG or DXF, and it does not
//! render: the shape is `DWG/DXF -> CadDatabase (entities + tables)`, and
//! from there `CadDatabase::to_json()` is the model's own, while SVG/PNG
//! rendering is the `iron-render-cad` crate's. This crate is one backend
//! that fills the model.

mod acis;
mod convert;
mod dynapi;
mod hatch_color;
mod table_convert;

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;

// The model this crate fills lives in `uncad-model`; it is re-exported whole
// so `uncad::model::...` / `uncad::tables::...` keep naming the same types.
pub use uncad_model::{json, model, tables};
pub use uncad_model::{CadDatabase, Entity, JsonError, ReadDiagnostics, Tables, ToJsonOptions};

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

/// Names for the non-critical `DWG_ERROR` bits, in bit order (dwg.h).
const NON_CRITICAL_BIT_NAMES: [&str; 7] = [
    "WRONGCRC",
    "NOTYETSUPPORTED",
    "UNHANDLEDCLASS",
    "INVALIDTYPE",
    "INVALIDHANDLE",
    "INVALIDEED",
    "VALUEOUTOFBOUNDS",
];

/// Turns a LibreDWG read result that was below the critical threshold into
/// the model's [`ReadDiagnostics`]: one warning per set bit, by its dwg.h
/// name (`WRONGCRC`, `NOTYETSUPPORTED`, `UNHANDLEDCLASS`, `INVALIDTYPE`,
/// `INVALIDHANDLE`, `INVALIDEED`, `VALUEOUTOFBOUNDS`), in ascending bit
/// order so the same read always lists them the same way. A bit this crate
/// does not know the name of is listed as `BIT<n>`, never dropped.
///
/// The bits used to be discarded, so a file LibreDWG read while skipping
/// objects it could not decode was indistinguishable from a clean read.
pub fn read_diagnostics_from_libredwg_bits(bits: i32) -> ReadDiagnostics {
    let mut warnings = Vec::new();
    for bit in 0..31 {
        if bits & (1 << bit) != 0 {
            warnings.push(match NON_CRITICAL_BIT_NAMES.get(bit) {
                Some(name) => (*name).to_string(),
                None => format!("BIT{bit}"),
            });
        }
    }
    ReadDiagnostics { warnings }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ParseError {
    /// `dwg_read_file`/`dxf_read_file` returned a critical LibreDWG error
    /// code (>= DWG_ERR_CRITICAL, see dwg.h).
    Critical(i32),
    InvalidPath,
    /// The DXF declares `$ACADVER` R2007 or later (`AC1021` and up). LibreDWG's
    /// DXF importer reads these files without reporting an error but loses
    /// every layer and block name on the way, so this crate refuses them
    /// instead of returning a drawing that is silently incomplete. The value
    /// is the `$ACADVER` string as written in the file. See `docs/CAVEATS.md`,
    /// "DXF reading".
    UnsupportedDxfVersion(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Critical(code) => write!(f, "LibreDWG critical read error (code {code})"),
            ParseError::InvalidPath => write!(f, "path is not valid UTF-8 / contains a NUL byte"),
            ParseError::UnsupportedDxfVersion(v) => write!(
                f,
                "DXF version {v} (R2007 or later) is not supported: LibreDWG's DXF importer                  would return a silently incomplete drawing -- save it as R2004 DXF or as DWG"
            ),
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
/// reading is. A DXF whose `$ACADVER` is R2007 or later is refused up front
/// with [`ParseError::UnsupportedDxfVersion`] rather than handed to LibreDWG,
/// because the importer would return it as a drawing with no entities and no
/// error. See `docs/CAVEATS.md`.
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError> {
    let path_str = path.as_ref().to_str().ok_or(ParseError::InvalidPath)?;
    let c_path = CString::new(path_str).map_err(|_| ParseError::InvalidPath)?;
    let is_dxf = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf"));

    // Decided from the file's own header, before LibreDWG sees it: the R2007+
    // failure is silent on the C side (error code 0, zero entities), so the
    // only place it can be turned into an error is here. No lock needed --
    // this is plain file I/O.
    if is_dxf {
        if let Some(acadver) = dxf_acadver(path.as_ref()) {
            if dxf_version_number(&acadver).is_some_and(|n| n >= DXF_VERSION_R2007) {
                return Err(ParseError::UnsupportedDxfVersion(acadver));
            }
        }
    }

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

    // Below the critical threshold the bits still mean something (an
    // UNHANDLEDCLASS read may be missing objects); they travel with the
    // result instead of being dropped here.
    let read_diagnostics = read_diagnostics_from_libredwg_bits(error);

    // Two walks over the live C structure, neither of which mutates it.
    // Everything the returned value exposes is an owned Rust copy by the end.
    let entities = unsafe { convert::convert_entities(dwg.as_mut()) };
    let tables = unsafe { table_convert::convert_tables(dwg.as_mut()) };

    // Nothing needs LibreDWG's structure past this point: this crate has no
    // write path, and the model above is what every export reads. Freeing it
    // here, still under the lock, is what keeps CadDatabase a plain value (no
    // Drop, no C memory, Send + Sync by construction).
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };

    Ok(CadDatabase {
        entities,
        tables,
        read_diagnostics,
    })
}

/// `$ACADVER` value of the first DXF release LibreDWG's importer reads back
/// incompletely: `AC1021` = R2007, the release that switched DXF strings to
/// UTF-16 in the importer's storage. Every later code (`AC1024`, `AC1027`,
/// `AC1032`, ...) is numerically above it.
const DXF_VERSION_R2007: u32 = 1021;

/// The numeric part of an `$ACADVER` code (`AC1021` -> `1021`), or `None` for
/// anything that is not shaped like one. An unrecognised value never rejects a
/// file: the decision falls back to LibreDWG, as it did before this check.
fn dxf_version_number(acadver: &str) -> Option<u32> {
    acadver.strip_prefix("AC")?.parse().ok()
}

/// Reads `$ACADVER` out of an ASCII DXF's HEADER section, or `None` if the
/// file has no such variable (pre-R10 files), is not readable, or is not laid
/// out as (group code, value) line pairs. Binary DXF is not handled here and
/// falls through to LibreDWG untouched.
///
/// The scan is bounded: `$ACADVER` is by convention the first header variable,
/// and the HEADER section ends at the first `ENDSEC`, so a file that reaches
/// either bound without it is treated as not declaring a version.
fn dxf_acadver(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};

    const MAX_PAIRS_SCANNED: usize = 4096;

    let file = std::fs::File::open(path).ok()?;
    let mut lines = BufReader::new(file).lines().map_while(Result::ok);
    for _ in 0..MAX_PAIRS_SCANNED {
        let (code, value) = (lines.next()?, lines.next()?);
        match (code.trim(), value.trim()) {
            ("9", "$ACADVER") => {
                let (code, value) = (lines.next()?, lines.next()?);
                return (code.trim() == "1").then(|| value.trim().to_string());
            }
            // End of the HEADER section (or of a headerless file's first
            // section): $ACADVER cannot appear after this point.
            ("0", "ENDSEC") | ("0", "EOF") => return None,
            _ => {}
        }
    }
    None
}
