//! The DWG reader `undwg` behind the `undwg-reader` feature.
//!
//! `undwg` reads a DWG from the public file specification, in safe Rust, with
//! no C library in the way. It fills the same entity model as the LibreDWG
//! path; this module gives the drawing's [`Header`] the same way, from what
//! the file states about itself ([`undwg::DrawingFacts`]). A field `undwg`
//! does not read stays `None`, as a field a file does not state does.

use uncad_model::{CadDatabase, Point2D, Point3D, Ref};

use crate::header::Header;
use crate::{Format, ParseError};

/// Reads the DWG in `bytes` with `undwg`.
pub(crate) fn parse(bytes: &[u8]) -> Result<(CadDatabase, Header), ParseError> {
    let (database, facts) = undwg::read_database_with_facts(bytes).map_err(ParseError::Undwg)?;
    let header = header_of(bytes, &database, &facts);
    Ok((database, header))
}

/// LibreDWG's release name for a version: how [`Header::version`] spells it.
fn release_name(version: undwg::Version) -> Option<&'static str> {
    Some(match version {
        undwg::Version::R2000 => "r2000",
        undwg::Version::R2004 => "r2004",
        undwg::Version::R2007 => "r2007",
        undwg::Version::R2010 => "r2010",
        undwg::Version::R2013 => "r2013",
        undwg::Version::R2018 => "r2018",
        _ => return None,
    })
}

fn point3(p: [f64; 3]) -> Point3D {
    Point3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

fn point2(p: [f64; 2]) -> Point2D {
    Point2D { x: p[0], y: p[1] }
}

/// A header variable the file keeps as a signed short and the header as an
/// unsigned one. A negative value (which no such variable has) is left out
/// rather than wrapped.
fn unsigned(value: i16) -> Option<u16> {
    u16::try_from(value).ok()
}

fn header_of(bytes: &[u8], database: &CadDatabase, facts: &undwg::DrawingFacts) -> Header {
    let v = &facts.variables;
    let model = &v.model_space;
    let paper = &v.paper_space;
    let d = &v.dimension;
    let clayer = match (&facts.current_layer, v.current_layer) {
        (Some(name), _) => Ref::Resolved(name.clone()),
        (None, 0) => Ref::Absent,
        (None, handle) => Ref::Unresolved(format!("{handle:X}")),
    };
    Header {
        format: Format::Dwg,
        acadver: bytes
            .get(..6)
            .and_then(|magic| std::str::from_utf8(magic).ok())
            .map(str::to_owned),
        version: release_name(facts.version).map(str::to_owned),
        codepage: facts.code_page,
        codepage_name: undwg::CodePage::name_of(facts.code_page).map(str::to_owned),
        insunits: database.header.insunits,
        // `MEASUREMENT` is in a section `undwg` does not read yet.
        measurement: None,
        lunits: unsigned(v.linear_units),
        luprec: unsigned(v.linear_precision),
        aunits: unsigned(v.angular_units),
        auprec: unsigned(v.angular_precision),
        extmin: Some(point3(model.extents_min)),
        extmax: Some(point3(model.extents_max)),
        limmin: Some(point2(model.limits_min)),
        limmax: Some(point2(model.limits_max)),
        pextmin: Some(point3(paper.extents_min)),
        pextmax: Some(point3(paper.extents_max)),
        plimmin: Some(point2(paper.limits_min)),
        plimmax: Some(point2(paper.limits_max)),
        dimscale: Some(d.scale),
        dimlfac: Some(d.linear_factor),
        dimdec: unsigned(d.decimals),
        dimlunit: unsigned(d.linear_unit_format),
        dimpost: d.post.plain(),
        dimrnd: Some(d.rounding),
        dimzin: unsigned(d.zero_suppression),
        dimfrac: unsigned(d.fraction_format),
        dimaunit: unsigned(d.angular_unit_format),
        dimadec: unsigned(d.angular_decimals),
        dimtxt: Some(d.text_height),
        dimasz: Some(d.arrow_size),
        ltscale: Some(v.line_type_scale),
        textsize: Some(v.text_size),
        pdmode: database.header.pdmode,
        pdsize: database.header.pdsize,
        clayer,
        fingerprintguid: database.header.fingerprintguid.clone(),
        versionguid: database.header.versionguid.clone(),
    }
}
