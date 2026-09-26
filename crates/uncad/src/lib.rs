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
mod dxf_records;
mod dynapi;
pub mod header;
mod table_convert;
mod text;

use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;

// The model this crate fills lives in `uncad-model`; it is re-exported whole
// so `uncad::model::...` / `uncad::tables::...` keep naming the same types.
pub use uncad_model::{json, model, tables};
pub use uncad_model::{CadDatabase, Entity, JsonError, ReadDiagnostics, Tables, ToJsonOptions};

pub use header::{Header, Units};

/// LibreDWG's C code has non-reentrant global state (the `loglevel` global
/// read and written throughout `decode.c`/`bits.c`, and likely more).
/// Calling into it concurrently reliably produced `STATUS_HEAP_CORRUPTION`
/// under `cargo test`'s parallel runner once two threads' read/convert/free
/// cycles overlapped. Sequential reuse across many calls is fine; concurrent
/// calls are what this lock closes off.
///
/// [`parse_bytes_with_header`] (which every other entry point calls) is the
/// only one that touches the FFI boundary and it holds this lock for its
/// whole duration, so callers never need to know `libredwg-sys` exists, let
/// alone serialize around it themselves.
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
    /// LibreDWG's decoder returned a critical error code (>= DWG_ERR_CRITICAL,
    /// see dwg.h) for the bytes it was given.
    Critical(i32),
    /// The file could not be read from disk ([`parse`] only; [`parse_bytes`]
    /// never produces it).
    Io(std::io::Error),
    /// A DXF saved as R2007 or later (`$ACADVER` `AC1021` and up) that this
    /// crate could not read: LibreDWG's importer placed entities in its model
    /// or paper space, and none of them reached the model -- the way every
    /// such file used to come out, as a drawing with no entities and no
    /// error. The value is the `$ACADVER` string as written in the file (or
    /// LibreDWG's name for the version, for a binary DXF). See
    /// `docs/CAVEATS.md`, "DXF saved as R2007 or later".
    UnsupportedDxfVersion(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Critical(code) => write!(f, "LibreDWG critical read error (code {code})"),
            ParseError::Io(e) => write!(f, "cannot read the file: {e}"),
            ParseError::UnsupportedDxfVersion(v) => write!(
                f,
                "DXF version {v} (R2007 or later) could not be read: none of the entities LibreDWG's DXF importer placed in model or paper space reached the drawing -- save it as R2004 DXF or as DWG"
            ),
        }
    }
}
impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::Io(e) => Some(e),
            ParseError::Critical(_) | ParseError::UnsupportedDxfVersion(_) => None,
        }
    }
}

/// The on-disk format of a drawing. [`parse`] picks it from the file
/// extension; [`parse_bytes`] takes it explicitly because bytes carry no
/// name. A binary DXF is [`Format::Dxf`] too: the reader tells ASCII from
/// binary by the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// Groups an entity cannot be understood without, where this crate filled a
/// value instead of finding one.
///
/// Most absent groups have a reading -- a flag word that is not there means
/// no bit is set, an absent colour means ByLayer -- and those are silent
/// because there is nothing to report. An attribute's tag is the other
/// kind: it is how a consumer asks for the attribute and how a block
/// definition lines up with its references, so an attribute without one is
/// not an attribute named `""` but an entity nothing can place. Reporting
/// is not refusing: the entity is kept, and the reader says what it had to
/// make up.
///
/// Read off the finished model rather than threaded through the conversion:
/// the condition is a property of the value, and checking it in one place
/// keeps the two from drifting apart. It cannot tell a group that was
/// absent from one that was present and empty -- and neither can the
/// condition itself, which is why both read the same way here.
fn missing_required_groups(entities: &[Entity]) -> Vec<String> {
    fn walk(entities: &[Entity], out: &mut Vec<String>) {
        for entity in entities {
            let name = match entity {
                Entity::Attrib(a) if a.tag.is_empty() => "ATTRIB",
                Entity::Attdef(a) if a.tag.is_empty() => "ATTDEF",
                Entity::Insert(insert) => {
                    let owned: Vec<Entity> =
                        insert.attribs.iter().cloned().map(Entity::Attrib).collect();
                    walk(&owned, out);
                    continue;
                }
                _ => continue,
            };
            out.push(format!(
                "MISSING_REQUIRED_GROUP: {name} carries no tag (group 2); the entity is kept with an empty one"
            ));
        }
    }
    let mut out = Vec::new();
    walk(entities, &mut out);
    out
}

/// Parses a DWG or DXF file at `path` into a [`CadDatabase`] -- the same
/// model either way, with the format inferred from the `.dwg`/`.dxf`
/// extension (see [`Format::from_path`]). [`parse_with_header`] returns the
/// drawing's header variables beside it.
///
/// The file is read into memory here and decoded by [`parse_bytes`] rather
/// than handed to LibreDWG as a path: LibreDWG opens paths with `fopen()`,
/// which on Windows interprets the bytes in the process's ANSI code page and
/// so cannot open a path with, say, a Korean directory name. `std::fs::read`
/// has no such limit, and a file that cannot be read is [`ParseError::Io`].
///
/// How complete DXF reading is depends on the entity type: LibreDWG's own
/// DXF reader is documented as working "for most objects" rather than being
/// feature-complete the way DWG reading is, and it costs more than the
/// square of the entity count, so a DXF of ten megabytes takes over a
/// minute. A DXF saved as R2007 or later is read like any other; one whose
/// entities nonetheless reach no model or paper space is
/// [`ParseError::UnsupportedDxfVersion`] rather than an empty drawing. See
/// `docs/CAVEATS.md`.
pub fn parse(path: impl AsRef<Path>) -> Result<CadDatabase, ParseError> {
    parse_with_header(path).map(|(db, _)| db)
}

/// [`parse`], returning the drawing's [`Header`] -- version, codepage,
/// units, extents, the dimension variables -- beside the database.
///
/// The header is a type of this crate rather than part of the model, which
/// deliberately carries no header variables (see [`header`]); it is read in
/// the same pass, under the same lock, from the same decoded drawing, so
/// asking for it costs no second decode -- which a separate header read
/// would, and a DXF decode is the expensive part.
pub fn parse_with_header(path: impl AsRef<Path>) -> Result<(CadDatabase, Header), ParseError> {
    let path = path.as_ref();
    let format = Format::from_path(path);
    let bytes = std::fs::read(path).map_err(ParseError::Io)?;
    parse_bytes_with_header(&bytes, format)
}

/// Parses a drawing already held in memory. This is what [`parse`] calls
/// after reading the file; it is public for callers that receive drawings
/// over the network or from an archive and never have a path.
pub fn parse_bytes(bytes: &[u8], format: Format) -> Result<CadDatabase, ParseError> {
    parse_bytes_with_header(bytes, format).map(|(db, _)| db)
}

/// [`parse_bytes`], returning the drawing's [`Header`] beside the database
/// -- see [`parse_with_header`].
pub fn parse_bytes_with_header(
    bytes: &[u8],
    format: Format,
) -> Result<(CadDatabase, Header), ParseError> {
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
    // heap address; it lives only until the conversion walks below have
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

    // Below the critical threshold the bits still mean something (an
    // UNHANDLEDCLASS read may be missing objects); they travel with the
    // result instead of being dropped here.
    let mut read_diagnostics = read_diagnostics_from_libredwg_bits(error);

    // Every string the walks below read goes through this decoder: it knows
    // how this drawing's strings sit in LibreDWG's memory, and what could
    // not be decoded is reported, never dropped.
    let text = unsafe { text::TextDecoder::new(dwg.as_mut()) };

    // Walks over the live C structure, none of which mutates it.
    // Everything the returned value exposes is an owned Rust copy by the end.
    let entities = unsafe { convert::convert_entities(dwg.as_mut(), &text) };
    let tables = unsafe { table_convert::convert_tables(dwg.as_mut(), &text) };
    let (acadver, stated) = match format {
        Format::Dwg => (
            header::dwg_magic(bytes),
            // SAFETY: the shims read file-header fields of a live Dwg_Data.
            header::Stated::Dwg {
                version: unsafe { libredwg_sys::uncad_dwg_from_version(dwg.as_mut()) },
                numheader_vars: unsafe { libredwg_sys::uncad_dwg_numheader_vars(dwg.as_mut()) },
                template_read: unsafe { libredwg_sys::uncad_dwg_template_read(dwg.as_mut()) } != 0,
            },
        ),
        Format::Dxf => match header::scan_dxf_header(bytes) {
            Some(scan) => (scan.acadver, header::Stated::Dxf(scan.variables)),
            None => (None, header::Stated::Unknown),
        },
    };
    let header = unsafe { header::read_header(dwg.as_mut(), &text, format, acadver, &stated) };

    // The one way an R2007+ DXF used to fail was silently: the importer
    // stored its strings in a width the lookups did not expect, no block
    // record was found by name, and the result was a drawing with no
    // entities and no error. The width is handled now (text.rs, and the
    // vendored dwg.c patch for the importer's own lookups); should a file
    // still come out that way, it is an error, not an empty drawing.
    let unplaced = entities_went_missing(
        format,
        is_r2007_or_later(dwg.as_mut()),
        entities.len(),
        // SAFETY: the Dwg_Data is live until the dwg_free below.
        || unsafe { entities_placed_in_a_space(dwg.as_mut()) },
    );

    read_diagnostics.warnings.extend(text.into_warnings());
    read_diagnostics
        .warnings
        .extend(missing_required_groups(&entities));
    if format == Format::Dxf {
        read_diagnostics.warnings.extend(entities_lost(
            dxf_records::entities_section_records(bytes),
            entities.len(),
        ));
    }

    // Nothing needs LibreDWG's structure past this point: this crate has no
    // write path, and the model above is what every export reads. Freeing it
    // here, still under the lock, is what keeps CadDatabase a plain value (no
    // Drop, no C memory, Send + Sync by construction).
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };

    if unplaced {
        let version = header
            .acadver
            .clone()
            .or_else(|| header.version.clone())
            .unwrap_or_default();
        return Err(ParseError::UnsupportedDxfVersion(version));
    }

    Ok((
        CadDatabase {
            entities,
            tables,
            read_diagnostics,
        },
        header,
    ))
}

/// The check against a DXF read that lost entities without saying so: the
/// importer can stop partway through a file (a malformed value it logs and
/// moves past) and still return success, leaving the model with a fraction
/// of what the file holds. `stated` is the number of top-level entity
/// records the file's ENTITIES section holds (see [`dxf_records`]), `read`
/// the number of top-level entities in the model. A model can hold more than
/// the section states (paper-space content kept in BLOCKS), never fewer
/// unless some were lost -- measured on the corpus, every ASCII DXF that
/// reads holds at least as many.
fn entities_lost(stated: Option<usize>, read: usize) -> Option<String> {
    let stated = stated?;
    (read < stated).then(|| {
        format!(
            "ENTITIES_MISSING: the file's ENTITIES section holds {stated} entity records \
             (a polyline's vertices and an insert's attributes counted with their owner); \
             {read} reached the drawing"
        )
    })
}

/// The guard against the silent R2007+ DXF failure: a DXF saved as R2007 or
/// later whose model holds no entity although LibreDWG placed some in model
/// or paper space. Scoped to R2007+ DXF because that is where the failure
/// lived: measured on the corpus, the same test applied to every input would
/// also refuse two pre-R13 DWGs (`r11/ACEB10.dwg`, `r2.10/block.dwg`) that
/// read today with an empty model space -- a different question, and not an
/// error this change is entitled to introduce.
fn entities_went_missing(
    format: Format,
    r2007_or_later: bool,
    converted: usize,
    placed_in_a_space: impl FnOnce() -> usize,
) -> bool {
    format == Format::Dxf && r2007_or_later && converted == 0 && placed_in_a_space() > 0
}

/// Whether the drawing was read from an R2007-or-later source.
fn is_r2007_or_later(dwg: *mut libredwg_sys::Dwg_Data) -> bool {
    // SAFETY: the shim reads one header field of a live Dwg_Data.
    let version = unsafe { libredwg_sys::uncad_dwg_from_version(dwg) };
    #[allow(clippy::unnecessary_cast)]
    let r2007 = libredwg_sys::DWG_VERSION_TYPE_R_2007 as i32;
    version >= r2007
}

/// How many entities LibreDWG itself placed in model or paper space
/// (`entmode` 2 or 1) -- found without any name lookup, so a drawing whose
/// block records failed to resolve by name still counts what it holds.
///
/// # Safety
/// `dwg` must be a live, successfully read `Dwg_Data`.
unsafe fn entities_placed_in_a_space(dwg: *mut libredwg_sys::Dwg_Data) -> usize {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    (0..num_objects)
        .filter(|&i| {
            // SAFETY: i is below the object count of the live Dwg_Data; the
            // shim answers NULL for an object that is not an entity.
            let obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
            if obj.is_null() {
                return false;
            }
            let entity = unsafe { libredwg_sys::uncad_object_entity_ptr(obj) };
            matches!(
                dynapi::get_common_field::<u8>(entity, "entmode"),
                Some(1 | 2)
            )
        })
        .count()
}

#[cfg(test)]
mod missing_required_groups_tests {
    use super::missing_required_groups;
    use uncad_model::model::{
        AttribEntity, Confidence, Entity, EntityCommon, EntityId, Origin, Point2D, Ref,
    };

    /// No drawing is known to make this fire through the importer, so the
    /// wording is pinned on the function itself: it must be the other
    /// reader's, word for word.
    #[test]
    fn an_untagged_attribute_is_reported_in_the_other_readers_words() {
        let untagged = Entity::Attrib(AttribEntity {
            common: EntityCommon {
                id: EntityId::new(0x41),
                origin: Origin::Vector,
                confidence: Confidence::High,
                source_handle: Ref::Resolved("41".to_string()),
                layer: Ref::Resolved("0".to_string()),
                color_index: 256,
                true_color: None,
                invisible: false,
                linetype: uncad_model::model::EntityLinetype::ByLayer,
                linetype_scale: 1.0,
                lineweight: Some(-1),
                transparency: Some(0),
            },
            start_point: Point2D::default(),
            text_height: 1.0,
            tag: String::new(),
            text: "X".to_string(),
            rotation: 0.0,
            flags: Default::default(),
            horizontal_justification: Default::default(),
            vertical_justification: Default::default(),
            alignment_point: None,
            width_factor: 1.0,
            oblique_angle: 0.0,
            style_name: uncad_model::Ref::Absent,
            elevation: 0.0,
            extrusion: uncad_model::Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        });
        assert_eq!(
            missing_required_groups(&[untagged]),
            ["MISSING_REQUIRED_GROUP: ATTRIB carries no tag (group 2); the entity is kept with an empty one"]
        );
    }

    /// A message split across source lines must not carry the indentation
    /// of the second line into what the user reads.
    #[test]
    fn the_unsupported_version_message_reads_as_one_sentence() {
        let message = super::ParseError::UnsupportedDxfVersion("AC1024".to_string()).to_string();
        assert!(!message.contains("  "), "{message}");
    }
}

#[cfg(test)]
mod unplaced_entities_tests {
    use super::*;

    #[test]
    fn only_an_r2007_plus_dxf_whose_placed_entities_all_went_missing_is_refused() {
        assert!(entities_went_missing(Format::Dxf, true, 0, || 72));
        // Something reached the model, or nothing was there to reach it.
        assert!(!entities_went_missing(Format::Dxf, true, 1, || 72));
        assert!(!entities_went_missing(Format::Dxf, true, 0, || 0));
        // Not the case the guard is for; the count is not even taken.
        assert!(!entities_went_missing(
            Format::Dxf,
            false,
            0,
            || unreachable!()
        ));
        assert!(!entities_went_missing(
            Format::Dwg,
            true,
            0,
            || unreachable!()
        ));
    }

    /// The count the guard compares against comes from LibreDWG's own
    /// placement of each entity, not from any name lookup: it is there for
    /// an R2007+ DXF whatever its strings did.
    #[test]
    fn libredwg_places_an_r2018_dxfs_entities_without_a_name_lookup() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../lib/libredwg/test/test-data/example_2018.dxf"
        );
        let bytes = std::fs::read(path).expect("the corpus DXF is readable");
        let _guard = LIBREDWG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut dwg: Box<libredwg_sys::Dwg_Data> =
            Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        let error = unsafe {
            libredwg_sys::uncad_dxf_read_bytes(bytes.as_ptr(), bytes.len(), dwg.as_mut())
        };
        assert_eq!(error, 0);
        let placed = unsafe { entities_placed_in_a_space(dwg.as_mut()) };
        let r2007 = is_r2007_or_later(dwg.as_mut());
        unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
        assert!(r2007);
        // 69 on this file when measured; any count at all is the point.
        assert!(placed > 0, "{placed}");
    }
}

#[cfg(test)]
mod entities_lost_tests {
    use super::entities_lost;

    #[test]
    fn fewer_than_the_file_states_is_reported_with_both_counts() {
        let warning = entities_lost(Some(68), 1).unwrap();
        assert!(warning.starts_with("ENTITIES_MISSING: "), "{warning}");
        assert!(warning.contains("holds 68 entity records"), "{warning}");
        assert!(warning.ends_with("1 reached the drawing"), "{warning}");
    }

    #[test]
    fn as_many_or_more_than_the_file_states_is_silent() {
        assert_eq!(entities_lost(Some(68), 68), None);
        assert_eq!(entities_lost(Some(69), 72), None);
    }

    #[test]
    fn an_unscanned_file_is_silent() {
        assert_eq!(entities_lost(None, 0), None);
    }
}
