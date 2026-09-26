//! The drawing's header variables (`$INSUNITS`, `$EXTMIN`, `$DIMSCALE`, ...)
//! and the file-level facts that give the model's numbers their meaning --
//! version, code page, format -- as a type of this crate, outside the model.
//!
//! [`uncad_model::CadDatabase`] deliberately carries no header variables: the
//! model is format-shaped and states what the drawing *draws*. What a
//! consumer needs to read those numbers -- the unit a coordinate is in, the
//! extents the file claims, the dimension variables a DIMENSION falls back
//! on -- is here instead, read in the same pass and returned beside the
//! database by [`crate::parse_with_header`] / [`crate::parse_bytes_with_header`].
//!
//! # Unknown is `None`
//!
//! LibreDWG's header struct holds every variable whether or not the file
//! states it: a variable the file leaves out reads as whatever the reader put
//! there -- zero, or the importer's default -- and cannot be told from a
//! stated one by its value. So a variable is `Some` only when the file is
//! known to state it:
//!
//! - **DWG:** the header section has a fixed layout per version, and every
//!   variable in that layout is stated. `$INSUNITS`, `$DIMADEC`, `$DIMFRAC`
//!   and `$DIMLUNIT` exist from R2000, `$DIMDEC` and `$DIMAUNIT` from R13,
//!   and a pre-R13 header ends after however many variables its own count
//!   says (`numheader_vars`), which cuts off `$DIMZIN`/`$DIMRND`,
//!   `$DIMPOST`, `$DIMLFAC` and the paper-space extents and limits in the
//!   older releases. `$MEASUREMENT` is not in the header section at all but
//!   in the optional Template section, and is stated only when that was read.
//! - **ASCII DXF:** exactly the variables its HEADER section names, found by
//!   a scan of the file's own text (bounded by that section).
//! - **Binary DXF:** not scanned, so nothing can be told apart from a
//!   default and every variable is `None`.
//!
//! What is stated is kept as stated: `$EXTMIN`/`$EXTMAX` may be stale or
//! AutoCAD's `1e20` "never set" sentinel, and are not corrected here.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uncad_model::{Point2D, Point3D, Ref};

use crate::dynapi::{self, RawPoint2D, RawPoint3D};
use crate::text::{codepage_name, TextDecoder};
use crate::Format;

/// The drawing unit a `$INSUNITS` code names -- the model's table.
pub use uncad_model::Units;

/// File-level facts and the header variables a consumer needs to give the
/// model's numbers a meaning. Variable fields are named after the DXF
/// `$VARIABLE` in lower case, and are `None` when the file does not state
/// them (see the module doc).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Header {
    /// What the bytes were.
    pub format: Format,
    /// The version code as the file states it: the magic of a DWG
    /// (`"AC1015"`), `$ACADVER` of a DXF. `None` for a DXF without
    /// `$ACADVER` (pre-R10) or a binary one.
    pub acadver: Option<String>,
    /// LibreDWG's release name for that version (`"r2000"`, `"r2018"`, ...);
    /// `None` exactly when [`acadver`](Self::acadver) is, or when LibreDWG
    /// has no name for it.
    pub version: Option<String>,
    /// The codepage this read decoded the drawing's 8-bit strings with, as
    /// LibreDWG's `Dwg_Codepage` number (30 `ANSI_1252`, 40 `ANSI_949`, ...):
    /// the DWG file header's (R13 and later), a DXF's `$DWGCODEPAGE`, and
    /// LibreDWG's own default where a file names none -- which is why it is
    /// a fact of the read, not a variable. An R2007+ drawing's strings are
    /// Unicode and do not depend on it.
    pub codepage: u16,
    /// LibreDWG's name for [`codepage`](Self::codepage) (`"ANSI_1252"`);
    /// `None` for a value it has no table for -- such a codepage is reported
    /// in `read_diagnostics` and its strings read as UTF-8.
    pub codepage_name: Option<String>,
    /// `$INSUNITS`: the drawing unit, as its code; see [`Header::units`].
    pub insunits: Option<u16>,
    /// `$MEASUREMENT`: 0 English, 1 metric -- the default for dimensioning
    /// and hatch patterns, not the drawing unit.
    pub measurement: Option<u16>,
    /// `$LUNITS`: 1 scientific, 2 decimal, 3 engineering, 4 architectural,
    /// 5 fractional.
    pub lunits: Option<u16>,
    /// `$LUPREC`: decimal places (or fraction precision) of linear units.
    pub luprec: Option<u16>,
    /// `$AUNITS`: 0 degrees, 1 deg/min/sec, 2 grads, 3 radians, 4 surveyor.
    pub aunits: Option<u16>,
    /// `$AUPREC`: decimal places of angular units.
    pub auprec: Option<u16>,
    /// `$EXTMIN`/`$EXTMAX`: the model-space extents the file stores. May be
    /// stale, or AutoCAD's `+/-1e20` sentinel for "never set".
    pub extmin: Option<Point3D>,
    pub extmax: Option<Point3D>,
    /// `$LIMMIN`/`$LIMMAX`: the model-space drawing limits.
    pub limmin: Option<Point2D>,
    pub limmax: Option<Point2D>,
    /// `$PEXTMIN`/`$PEXTMAX`/`$PLIMMIN`/`$PLIMMAX`: the same for paper space.
    pub pextmin: Option<Point3D>,
    pub pextmax: Option<Point3D>,
    pub plimmin: Option<Point2D>,
    pub plimmax: Option<Point2D>,
    /// `$DIMSCALE`: overall dimension scale of the current dimension style.
    pub dimscale: Option<f64>,
    /// `$DIMLFAC`: linear measurement factor of the current dimension style.
    pub dimlfac: Option<f64>,
    /// `$DIMDEC`: decimal places of a linear dimension.
    pub dimdec: Option<u16>,
    /// `$DIMLUNIT`: linear unit format of a dimension (as `$LUNITS`, plus 6
    /// Windows desktop).
    pub dimlunit: Option<u16>,
    /// `$DIMPOST`: prefix/suffix of a dimension's text (`"<> mm"`, where
    /// `<>` stands for the measurement); `Some("")` when stated empty.
    pub dimpost: Option<String>,
    /// `$DIMRND`: rounding of a linear dimension.
    pub dimrnd: Option<f64>,
    /// `$DIMZIN`: zero suppression of a linear dimension.
    pub dimzin: Option<u16>,
    /// `$DIMFRAC`: fraction format of a dimension.
    pub dimfrac: Option<u16>,
    /// `$DIMAUNIT`: angle format of an angular dimension.
    pub dimaunit: Option<u16>,
    /// `$DIMADEC`: decimal places of an angular dimension.
    pub dimadec: Option<u16>,
    /// `$DIMTXT`: dimension text height.
    pub dimtxt: Option<f64>,
    /// `$DIMASZ`: dimension arrow size.
    pub dimasz: Option<f64>,
    /// `$LTSCALE`: global linetype scale.
    pub ltscale: Option<f64>,
    /// `$TEXTSIZE`: default text height.
    pub textsize: Option<f64>,
    /// `$CLAYER`: the current layer. `Absent` when not stated; `Unresolved`
    /// (the handle, hex) when it names a layer the table does not have.
    pub clayer: Ref<String>,
}

impl Header {
    /// `$INSUNITS` as a unit name and a millimetre factor; `None` when the
    /// file does not state `$INSUNITS`.
    pub fn units(&self) -> Option<Units> {
        self.insunits.map(Units::from_insunits)
    }
}

/// What the scan of an ASCII DXF's HEADER section found.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DxfHeaderScan {
    /// `$ACADVER`'s value, trimmed.
    pub acadver: Option<String>,
    /// Every variable the section names, without the `$`.
    pub variables: BTreeSet<String>,
}

/// Reads the variable names out of an ASCII DXF's HEADER section, as
/// (group code, value) line pairs, up to the section's `ENDSEC`. A file
/// whose first section is not HEADER states no variables. `None` for a
/// binary DXF, which is not scanned.
pub(crate) fn scan_dxf_header(bytes: &[u8]) -> Option<DxfHeaderScan> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return None;
    }
    fn trim(line: &[u8]) -> &[u8] {
        line.trim_ascii()
    }
    let mut scan = DxfHeaderScan::default();
    let mut lines = bytes.split(|&b| b == b'\n').map(trim);
    let mut in_header = false;
    let mut current: Option<Vec<u8>> = None;
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        match (code, value) {
            (b"0", b"SECTION") => continue,
            (b"0", b"ENDSEC") | (b"0", b"EOF") => break,
            (b"2", b"HEADER") if !in_header => in_header = true,
            // The first section is some other one: no HEADER to read.
            (b"2", _) if !in_header => break,
            (b"9", name) if in_header => {
                let name = name.strip_prefix(b"$").unwrap_or(name);
                scan.variables
                    .insert(String::from_utf8_lossy(name).into_owned());
                current = Some(name.to_vec());
            }
            (b"1", value) if in_header && current.as_deref() == Some(b"ACADVER") => {
                scan.acadver = Some(String::from_utf8_lossy(value).into_owned());
            }
            _ => {}
        }
    }
    Some(scan)
}

/// Which header variables the file states -- see the module doc.
pub(crate) enum Stated {
    /// A DWG, by its version's header layout.
    Dwg {
        version: i32,
        numheader_vars: u16,
        template_read: bool,
    },
    /// An ASCII DXF, by the variables its HEADER section names.
    Dxf(BTreeSet<String>),
    /// A binary DXF: nothing known.
    Unknown,
}

impl Stated {
    fn has(&self, name: &str) -> bool {
        match self {
            Stated::Dxf(variables) => variables.contains(name),
            Stated::Unknown => false,
            Stated::Dwg {
                version,
                numheader_vars,
                template_read,
            } => dwg_layout_has(name, *version, *numheader_vars, *template_read),
        }
    }
}

/// Whether a DWG of `version` carries `name` in its header, per LibreDWG's
/// own layout (`header_variables.spec`, `header_variables_r11.spec`) for the
/// variables this module reads.
#[allow(clippy::unnecessary_cast)] // the enum's width differs by target
fn dwg_layout_has(name: &str, version: i32, numheader_vars: u16, template_read: bool) -> bool {
    use libredwg_sys as sys;
    let at_least = |v: sys::Dwg_Version_Type| version >= v as i32;
    if name == "MEASUREMENT" {
        // In the Template section, which is optional before R2007 and which
        // the decoder skips silently when it is missing.
        return template_read;
    }
    if at_least(sys::DWG_VERSION_TYPE_R_13b1) {
        return match name {
            "INSUNITS" | "DIMADEC" | "DIMFRAC" | "DIMLUNIT" => {
                at_least(sys::DWG_VERSION_TYPE_R_2000b)
            }
            _ => true,
        };
    }
    // Before R13 the header is one run of variables whose length the file
    // header states; the early releases stop sooner still.
    let r1_3 = version == sys::DWG_VERSION_TYPE_R_1_3 as i32;
    let r2_0 = at_least(sys::DWG_VERSION_TYPE_R_2_0);
    match name {
        "EXTMIN" | "EXTMAX" | "LIMMIN" | "LIMMAX" | "TEXTSIZE" | "CLAYER" => true,
        "LUNITS" | "LUPREC" => r1_3 || at_least(sys::DWG_VERSION_TYPE_R_1_4),
        "AUNITS" | "AUPREC" | "DIMSCALE" | "DIMASZ" | "DIMTXT" => {
            r1_3 || at_least(sys::DWG_VERSION_TYPE_R_2_0b)
        }
        "LTSCALE" => r2_0,
        "DIMZIN" | "DIMRND" => r2_0 && numheader_vars > 83,
        "DIMPOST" => r2_0 && numheader_vars > 114,
        "DIMLFAC" => r2_0 && numheader_vars > 120,
        "PEXTMIN" | "PEXTMAX" | "PLIMMIN" | "PLIMMAX" => r2_0 && numheader_vars > 160,
        // INSUNITS, DIMDEC, DIMAUNIT, DIMADEC, DIMFRAC, DIMLUNIT: R13 and later.
        _ => false,
    }
}

/// The version code a DWG's first six bytes spell (`AC1015`).
pub(crate) fn dwg_magic(bytes: &[u8]) -> Option<String> {
    let magic = bytes.get(..6)?;
    magic
        .iter()
        .all(|b| b.is_ascii_graphic())
        .then(|| String::from_utf8_lossy(magic).into_owned())
}

/// LibreDWG's release name of a `Dwg_Version_Type` (`r2000`), or `None` for
/// `R_INVALID`.
pub(crate) fn release_name(version: i32) -> Option<String> {
    #[allow(clippy::unnecessary_cast)]
    if version <= libredwg_sys::DWG_VERSION_TYPE_R_INVALID as i32
        || version >= libredwg_sys::DWG_VERSION_TYPE_R_AFTER as i32
    {
        return None;
    }
    // SAFETY: an in-range enum value; the function returns a static string
    // from LibreDWG's version table.
    let ptr = unsafe { libredwg_sys::dwg_version_type(version as libredwg_sys::Dwg_Version_Type) };
    if ptr.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy();
    Some(name.into_owned())
}

/// Reads the header out of a live `Dwg_Data`. `acadver` is the version code
/// the file states (see [`Header::acadver`]), `stated` which variables it
/// states.
///
/// # Safety
/// `dwg` must point at a `Dwg_Data` a successful read filled in and that
/// has not been freed; `text` must be the decoder made for it.
pub(crate) unsafe fn read_header(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    format: Format,
    acadver: Option<String>,
    stated: &Stated,
) -> Header {
    // SAFETY: the shim reads one header field of a live Dwg_Data.
    let from_version = unsafe { libredwg_sys::uncad_dwg_from_version(dwg) };
    let version = acadver.as_ref().and_then(|_| release_name(from_version));
    let codepage = text.codepage();

    let u16_var = |name: &str| {
        stated
            .has(name)
            .then(|| dynapi::get_header_field::<u16>(dwg, name))
            .flatten()
    };
    let f64_var = |name: &str| {
        stated
            .has(name)
            .then(|| dynapi::get_header_field::<f64>(dwg, name))
            .flatten()
    };
    let p3_var = |name: &str| {
        stated
            .has(name)
            .then(|| dynapi::get_header_field::<RawPoint3D>(dwg, name).map(Point3D::from))
            .flatten()
    };
    let p2_var = |name: &str| {
        stated
            .has(name)
            .then(|| dynapi::get_header_field::<RawPoint2D>(dwg, name).map(Point2D::from))
            .flatten()
    };
    let dimpost = stated
        .has("DIMPOST")
        .then(|| text.header_text(dwg, "DIMPOST").unwrap_or_default());
    let clayer = if stated.has("CLAYER") {
        let handle = dynapi::get_header_field::<*mut libredwg_sys::Dwg_Object_Ref>(dwg, "CLAYER");
        layer_reference(dwg, text, handle.unwrap_or(std::ptr::null_mut()))
    } else {
        Ref::Absent
    };

    Header {
        format,
        acadver,
        version,
        codepage,
        codepage_name: codepage_name(codepage),
        insunits: u16_var("INSUNITS"),
        measurement: u16_var("MEASUREMENT"),
        lunits: u16_var("LUNITS"),
        luprec: u16_var("LUPREC"),
        aunits: u16_var("AUNITS"),
        auprec: u16_var("AUPREC"),
        extmin: p3_var("EXTMIN"),
        extmax: p3_var("EXTMAX"),
        limmin: p2_var("LIMMIN"),
        limmax: p2_var("LIMMAX"),
        pextmin: p3_var("PEXTMIN"),
        pextmax: p3_var("PEXTMAX"),
        plimmin: p2_var("PLIMMIN"),
        plimmax: p2_var("PLIMMAX"),
        dimscale: f64_var("DIMSCALE"),
        dimlfac: f64_var("DIMLFAC"),
        dimdec: u16_var("DIMDEC"),
        dimlunit: u16_var("DIMLUNIT"),
        dimpost,
        dimrnd: f64_var("DIMRND"),
        dimzin: u16_var("DIMZIN"),
        dimfrac: u16_var("DIMFRAC"),
        dimaunit: u16_var("DIMAUNIT"),
        dimadec: u16_var("DIMADEC"),
        dimtxt: f64_var("DIMTXT"),
        dimasz: f64_var("DIMASZ"),
        ltscale: f64_var("LTSCALE"),
        textsize: f64_var("TEXTSIZE"),
        clayer,
    }
}

/// `$CLAYER` as a reference: the layer's name when the table resolves it
/// (by handle, or by index before R13), the handle when it does not, and
/// `Absent` for a null one -- the same three states the model gives an
/// entity's layer.
fn layer_reference(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    handle: *mut libredwg_sys::Dwg_Object_Ref,
) -> Ref<String> {
    if handle.is_null() {
        return Ref::Absent;
    }
    if let Some(name) = text.table_entry_name(dwg, handle, c"LAYER") {
        return Ref::Resolved(name);
    }
    // SAFETY: a non-null Dwg_Object_Ref the live Dwg_Data owns.
    let (absolute_ref, r11_idx) = unsafe { ((*handle).absolute_ref, (*handle).r11_idx) };
    if dynapi::is_pre_r13(dwg) {
        return Ref::Unresolved(format!("idx:{r11_idx}"));
    }
    if absolute_ref != 0 {
        return Ref::Unresolved(format!("{absolute_ref:X}"));
    }
    Ref::Absent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insunits_table_names_the_common_units() {
        assert_eq!(
            Units::from_insunits(4),
            Units {
                name: "mm".into(),
                to_mm: Some(1.0)
            }
        );
        assert_eq!(
            Units::from_insunits(1),
            Units {
                name: "in".into(),
                to_mm: Some(25.4)
            }
        );
        assert_eq!(Units::from_insunits(6).to_mm, Some(1000.0));
        assert_eq!(Units::from_insunits(2).to_mm, Some(304.8));
    }

    #[test]
    fn insunits_zero_and_unknown_codes_are_unitless() {
        for code in [0, 25, 99, u16::MAX] {
            let units = Units::from_insunits(code);
            assert_eq!(units.name, "du", "code {code}");
            assert_eq!(units.to_mm, None, "code {code}");
        }
    }

    #[test]
    fn the_scan_reads_the_header_section_only() {
        let dxf = b"  0\r\nSECTION\r\n  2\r\nHEADER\r\n  9\r\n$ACADVER\r\n  1\r\nAC1015 \r\n  9\r\n$INSUNITS\r\n 70\r\n     4\r\n  0\r\nENDSEC\r\n  0\r\nSECTION\r\n  2\r\nENTITIES\r\n  9\r\n$DIMPOST\r\n  0\r\nENDSEC\r\n";
        let scan = scan_dxf_header(dxf).expect("ASCII DXF");
        assert_eq!(scan.acadver.as_deref(), Some("AC1015"));
        assert_eq!(
            scan.variables
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["ACADVER", "INSUNITS"]
        );
        // No HEADER section: nothing stated.
        let scan = scan_dxf_header(b"  0\nSECTION\n  2\nENTITIES\n  9\n$ACADVER\n  1\nAC1015\n")
            .expect("ASCII DXF");
        assert_eq!(scan, DxfHeaderScan::default());
        assert_eq!(scan_dxf_header(b"AutoCAD Binary DXF\r\n\x1a\0"), None);
    }

    #[test]
    #[allow(clippy::unnecessary_cast)] // the enum's width differs by target
    fn a_dwg_states_what_its_versions_header_layout_carries() {
        use libredwg_sys as sys;
        let (r14, r2000) = (
            sys::DWG_VERSION_TYPE_R_14 as i32,
            sys::DWG_VERSION_TYPE_R_2000 as i32,
        );
        for name in ["INSUNITS", "DIMADEC", "DIMFRAC", "DIMLUNIT"] {
            assert!(!dwg_layout_has(name, r14, 0, true), "{name}");
            assert!(dwg_layout_has(name, r2000, 0, true), "{name}");
        }
        assert!(dwg_layout_has("DIMDEC", r14, 0, false));
        assert!(!dwg_layout_has("MEASUREMENT", r2000, 0, false));
        assert!(dwg_layout_has("MEASUREMENT", r2000, 0, true));
        // R11 (numheader_vars 204/205) carries DIMLFAC and the paper-space
        // extents; an R2.10 header (83 variables) stops before DIMZIN.
        let (r11, r2_10) = (
            sys::DWG_VERSION_TYPE_R_11 as i32,
            sys::DWG_VERSION_TYPE_R_2_10 as i32,
        );
        assert!(dwg_layout_has("DIMLFAC", r11, 205, false));
        assert!(dwg_layout_has("PEXTMIN", r11, 205, false));
        assert!(!dwg_layout_has("INSUNITS", r11, 205, false));
        assert!(dwg_layout_has("EXTMIN", r2_10, 83, false));
        assert!(!dwg_layout_has("DIMZIN", r2_10, 83, false));
    }
}
