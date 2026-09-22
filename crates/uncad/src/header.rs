//! The drawing's header variables (`$INSUNITS`, `$EXTMIN`, `$DIMSCALE`, ...)
//! plus the file-level facts `Dwg_Header_Variables` does not hold (version,
//! code page): what a consumer needs to give the model's numbers a meaning.
//!
//! Read once inside [`crate::parse_bytes`], through LibreDWG's
//! `dwg_dynapi_header_value` for the header variables and the file-header
//! shims in `libredwg-sys` for the rest, and stored as plain values. Every
//! variable is always present in `Dwg_Header_Variables` (zero when the file
//! did not set it), so the fields here are plain values rather than `Option`s
//! -- the one derived field, [`Header::units`], is where "unset" becomes
//! visible (`$INSUNITS` 0 is "unitless").

use serde::{Deserialize, Serialize};

use crate::dynapi::{get_header_field, get_header_utf8, resolve_handle_name, Point2D, Point3D};

/// The drawing unit declared by `$INSUNITS`, with its conversion to
/// millimetres when the code names a real length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Units {
    /// Short name: `"mm"`, `"in"`, `"ft"`, `"m"`, ... -- or `"du"` ("drawing
    /// units") when `$INSUNITS` is 0 (unitless) or a code this crate does not
    /// know.
    pub name: String,
    /// Millimetres per drawing unit; `None` when unitless.
    pub to_mm: Option<f64>,
}

impl Default for Units {
    fn default() -> Self {
        Units::from_insunits(0)
    }
}

impl Units {
    /// The DXF reference's `$INSUNITS` table. LibreDWG has no such table of
    /// its own (it only reads and writes the code), so it lives here.
    pub fn from_insunits(code: u16) -> Units {
        let (name, to_mm): (&str, Option<f64>) = match code {
            1 => ("in", Some(25.4)),
            2 => ("ft", Some(304.8)),
            3 => ("mi", Some(1_609_344.0)),
            4 => ("mm", Some(1.0)),
            5 => ("cm", Some(10.0)),
            6 => ("m", Some(1000.0)),
            7 => ("km", Some(1_000_000.0)),
            8 => ("uin", Some(2.54e-5)),
            9 => ("mil", Some(0.0254)),
            10 => ("yd", Some(914.4)),
            11 => ("angstrom", Some(1e-7)),
            12 => ("nm", Some(1e-6)),
            13 => ("um", Some(1e-3)),
            14 => ("dm", Some(100.0)),
            15 => ("dam", Some(10_000.0)),
            16 => ("hm", Some(100_000.0)),
            17 => ("Gm", Some(1e12)),
            18 => ("au", Some(1.495_978_707e14)),
            19 => ("ly", Some(9.460_730_472_580_8e18)),
            20 => ("pc", Some(3.085_677_581_491_367e19)),
            // US survey units: 1200/3937 m to the foot.
            21 => ("us-ft", Some(304.800_609_601_219_2)),
            22 => ("us-in", Some(25.400_050_800_101_6)),
            23 => ("us-yd", Some(914.401_828_803_657_7)),
            24 => ("us-mi", Some(1_609_347.218_694_437_2)),
            _ => ("du", None),
        };
        Units {
            name: name.to_string(),
            to_mm,
        }
    }
}

/// File-level facts and the header variables the exports need. Field names
/// follow the DXF `$VARIABLE` names in lower case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Header {
    /// LibreDWG's name for the file's version, lower case (`"r2004"`,
    /// `"r2018"`, ...); empty when unknown.
    pub version: String,
    /// The `Dwg_Version_Type` enum value behind [`version`](Self::version).
    pub version_code: i32,
    /// The version the file was *read* as; differs from `version_code` only
    /// for DXF input.
    pub from_version_code: i32,
    /// The file header's code page, as LibreDWG's `Dwg_Codepage` enum value
    /// (0 UTF-8, 30 ANSI_1252, 39 ANSI_936, 40 ANSI_949, ...). This, not
    /// `$DWGCODEPAGE` (empty for DWG input), is what pre-R2007 strings were
    /// decoded with.
    pub codepage: u32,
    /// The DXF name of [`codepage`](Self::codepage): `"ANSI_949"` and so on;
    /// empty when LibreDWG has no name for it.
    pub codepage_name: String,
    /// `$INSUNITS` as stored.
    pub insunits: u16,
    /// [`insunits`](Self::insunits) resolved to a unit name and a factor.
    pub units: Units,
    /// `$MEASUREMENT`: 0 English, 1 metric (the dimensioning/hatch defaults,
    /// not the drawing unit).
    pub measurement: u16,
    /// `$LUNITS`: 1 scientific, 2 decimal, 3 engineering, 4 architectural,
    /// 5 fractional.
    pub lunits: u16,
    /// `$LUPREC`: decimal places (or, for `$LUNITS` 4/5, the fraction
    /// precision index) linear units display with.
    pub luprec: u16,
    pub aunits: u16,
    pub auprec: u16,
    /// `$EXTMIN`/`$EXTMAX`: AutoCAD's stored model-space extents. Kept
    /// verbatim -- may be stale or a `+/-1e20` "unset" sentinel; validate
    /// before use.
    pub extmin: Point3D,
    pub extmax: Point3D,
    pub limmin: Point2D,
    pub limmax: Point2D,
    /// Paper-space extents and limits (`$PEXTMIN` ...), same caveats.
    pub pextmin: Point3D,
    pub pextmax: Point3D,
    pub plimmin: Point2D,
    pub plimmax: Point2D,
    pub dimscale: f64,
    /// `$DIMLFAC`: linear measurement factor of the current dimension
    /// style; a DIMENSION's own style can override it.
    pub dimlfac: f64,
    pub dimdec: u16,
    pub dimlunit: u16,
    pub dimpost: String,
    pub dimrnd: f64,
    pub dimzin: u16,
    pub dimfrac: u16,
    pub dimaunit: u16,
    pub dimadec: u16,
    pub dimtxt: f64,
    pub dimasz: f64,
    pub ltscale: f64,
    pub textsize: f64,
    /// `$CLAYER`: the current layer's name; empty if unresolvable.
    pub clayer: String,
}

/// Where the data came from, for the few fields whose meaning depends on
/// it: R13/R14 files store no lineweights, and LibreDWG's DXF reader cannot
/// tell an absent LAYER plot flag (group 290) from a cleared one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source {
    pub from_dxf: bool,
    pub r2000_plus: bool,
}

/// # Safety
/// `dwg` must point at a `Dwg_Data` a successful read filled in.
pub(crate) unsafe fn source(dwg: *mut libredwg_sys::Dwg_Data) -> Source {
    // SAFETY: the shims only read two header fields of a live Dwg_Data.
    let version = unsafe { libredwg_sys::uncad_dwg_version(dwg) };
    let from_dxf = unsafe { libredwg_sys::uncad_dwg_from_dxf(dwg) } != 0;
    #[allow(clippy::unnecessary_cast)]
    let r2000_plus = version >= libredwg_sys::DWG_VERSION_TYPE_R_2000 as i32;
    Source {
        from_dxf,
        r2000_plus,
    }
}

/// Reads the header out of a live `Dwg_Data`.
///
/// # Safety
/// `dwg` must point at a `Dwg_Data` a successful read filled in and that
/// has not been `dwg_free`d.
pub(crate) unsafe fn convert_header(dwg: *mut libredwg_sys::Dwg_Data) -> Header {
    // SAFETY: dwg is live per this function's contract; the shims only read
    // the file header.
    let version_code = unsafe { libredwg_sys::uncad_dwg_version(dwg) };
    let from_version_code = unsafe { libredwg_sys::uncad_dwg_from_version(dwg) };
    let codepage = unsafe { libredwg_sys::uncad_dwg_codepage(dwg) };
    let version = static_c_str(unsafe {
        libredwg_sys::dwg_version_type(version_code as libredwg_sys::Dwg_Version_Type)
    });
    let codepage_name = static_c_str(unsafe { libredwg_sys::uncad_codepage_name(codepage) });

    let u16_var = |name: &str| get_header_field::<u16>(dwg, name).unwrap_or(0);
    let f64_var = |name: &str| get_header_field::<f64>(dwg, name).unwrap_or(0.0);
    let p3_var = |name: &str| get_header_field::<Point3D>(dwg, name).unwrap_or_default();
    let p2_var = |name: &str| get_header_field::<Point2D>(dwg, name).unwrap_or_default();

    let insunits = u16_var("INSUNITS");
    let clayer = get_header_field::<*mut libredwg_sys::Dwg_Object_Ref>(dwg, "CLAYER")
        .and_then(|handle| resolve_handle_name(dwg, handle))
        .unwrap_or_default();

    Header {
        version,
        version_code,
        from_version_code,
        codepage,
        codepage_name,
        insunits,
        units: Units::from_insunits(insunits),
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
        dimpost: get_header_utf8(dwg, "DIMPOST").unwrap_or_default(),
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

/// Copies a C string LibreDWG owns for the life of the process (a name-table
/// entry), or gives an empty string for a null pointer.
fn static_c_str(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: a non-null pointer from these helpers is a NUL-terminated
    // string literal inside LibreDWG's own tables.
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
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
        assert_eq!(Units::default(), Units::from_insunits(0));
    }

    #[test]
    fn header_default_is_unitless_and_empty() {
        let header = Header::default();
        assert_eq!(header.units.name, "du");
        assert!(header.version.is_empty());
        assert_eq!(header.dimlfac, 0.0);
    }
}
