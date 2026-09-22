//! `CadDatabase::header` on real corpus files: version, code page, units and
//! the header variables the exports depend on. The expected values are the
//! ones LibreDWG's own `dwg_dynapi_header_value` reports for these files
//! (cross-checked against the DXF twin's `$` variables where one exists), so
//! the drawing is its own reference; nothing here is a number copied from
//! this project's output.

use uncad::{CadDatabase, Format, Units};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);
const GH109_2013_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2013/gh109_1.dwg"
);
const TEST_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

/// Every R2007+ DXF in the corpus that LibreDWG reads (`$ACADVER` AC1021 or
/// later; `2013/gh109_1`, `2018/Constraints`, `Dynblocks`, `LiveSection1`
/// and `TS1` fail in LibreDWG itself). All of them declare an empty
/// `$DIMPOST` (`grep -A1 '^\$DIMPOST'` over the files).
const R2007_PLUS_DXFS: &[&str] = &[
    "example_2007.dxf",
    "example_2010.dxf",
    "example_2013.dxf",
    "example_2018.dxf",
    "sample_2007.dxf",
    "sample_2010.dxf",
    "sample_2013.dxf",
    "sample_2018.dxf",
    "2007/Arc.dxf",
    "2007/circle.dxf",
    "2007/Constraints.dxf",
    "2007/ConstructionLine.dxf",
    "2007/Donut.dxf",
    "2007/Ellipse.dxf",
    "2007/Helix.dxf",
    "2007/Leader.dxf",
    "2007/Line.dxf",
    "2007/Polygon.dxf",
    "2007/Polyline.dxf",
    "2007/RAY.dxf",
    "2007/Text.dxf",
    "2010/Constraints.dxf",
    "2010/gh209_1.dxf",
    "2010/Leader.dxf",
    "2013/Constraints.dxf",
    "2013/Leader.dxf",
    "2018/Leader.dxf",
];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn example_2000_dwg_header_reads_the_dxf_variables() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let h = &db.header;

    assert_eq!(h.version, "r2000");
    assert_eq!(
        h.version_code, h.from_version_code,
        "a DWG is read as its own version"
    );
    assert_eq!((h.codepage, h.codepage_name.as_str()), (30, "ANSI_1252"));

    // $INSUNITS 4 = millimetres, $MEASUREMENT 1 = metric.
    assert_eq!(h.insunits, 4);
    assert_eq!(
        h.units,
        Units {
            name: "mm".into(),
            to_mm: Some(1.0)
        }
    );
    assert_eq!(h.measurement, 1);
    // $LUNITS 2 decimal, $DIMDEC 2, $DIMLFAC 1.0 -- what the file declares.
    assert_eq!((h.lunits, h.dimdec), (2, 2));
    assert!(close(h.dimlfac, 1.0));
    // $LIMMAX is the A3 sheet in mm.
    assert!(
        close(h.limmax.x, 420.0) && close(h.limmax.y, 297.0),
        "{:?}",
        h.limmax
    );
    // The stored model extents include the 3256x-scaled INSERT (see
    // docs/VLM_INVESTIGATION.md): kept verbatim, not "corrected".
    assert!(
        h.extmin.x < -2_000_000.0 && h.extmax.x > 800_000.0,
        "{:?}..{:?}",
        h.extmin,
        h.extmax
    );
    // Paper-space extents were never set: AutoCAD's +/-1e20 sentinel, verbatim.
    assert!(
        h.pextmin.x > 1e19 && h.pextmax.x < -1e19,
        "{:?}..{:?}",
        h.pextmin,
        h.pextmax
    );
    assert_eq!(h.clayer, "Tavolo 3");
}

#[test]
fn the_dxf_twin_declares_the_same_units_and_dimension_variables() {
    let dwg = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let dxf = uncad::parse(EXAMPLE_2000_DXF).expect("corpus file must parse");
    let (a, b) = (&dwg.header, &dxf.header);

    assert_eq!(a.insunits, b.insunits);
    assert_eq!(a.units, b.units);
    assert_eq!(a.measurement, b.measurement);
    assert_eq!(
        (a.lunits, a.luprec, a.dimdec, a.dimlunit),
        (b.lunits, b.luprec, b.dimdec, b.dimlunit)
    );
    assert!(close(a.dimlfac, b.dimlfac) && close(a.dimscale, b.dimscale));
    assert!(close(a.limmax.x, b.limmax.x) && close(a.limmax.y, b.limmax.y));
    assert_eq!(a.clayer, b.clayer);
    // A DXF names its code page in $DWGCODEPAGE, which the reader records.
    assert_eq!(b.codepage_name, "ANSI_1252");
}

#[test]
fn a_chinese_r2013_file_reports_its_code_page_and_unitless_units() {
    let db = uncad::parse(GH109_2013_DWG).expect("corpus file must parse");
    let h = &db.header;
    assert_eq!(h.version, "r2013");
    assert_eq!((h.codepage, h.codepage_name.as_str()), (39, "ANSI_936"));
    assert_eq!(h.insunits, 0);
    assert_eq!(
        h.units,
        Units {
            name: "du".into(),
            to_mm: None
        }
    );
    // R2007+ strings arrive as UTF-8 from LibreDWG itself; the code page is
    // recorded but not applied. The file's layer table has a Chinese name.
    assert!(
        db.tables
            .layers
            .keys()
            .any(|name| name.chars().any(|c| c as u32 > 0x2E80)),
        "expected a CJK layer name, got {:?}",
        db.tables.layers.keys().collect::<Vec<_>>()
    );
}

#[test]
fn r2007_and_later_dxf_header_text_variables_are_not_read_as_utf16() {
    // LibreDWG parses the HEADER section before it sets header.version (only
    // R13..R2000 get it from $ACADVER; dxf_fixup_header runs afterwards), so
    // every $ text variable is a plain 8-bit copy of the file's bytes even
    // in an R2007+ DXF. Reading it as UTF-16 scanned past the 1-byte
    // allocation of an empty $DIMPOST and gave "DIMSE..."-style heap
    // garbage on every one of these files.
    for name in R2007_PLUS_DXFS {
        let path = format!("{TEST_DATA}{name}");
        let db = uncad::parse(&path).expect("corpus file must parse");
        assert!(
            ["r2007", "r2010", "r2013", "r2018"].contains(&db.header.version.as_str()),
            "{name}: {}",
            db.header.version
        );
        assert_eq!(db.header.dimpost, "", "{name}");
    }
}

/// A DXF of version `acadver` whose only header variables are `$ACADVER`
/// and `$DIMPOST` (`dimpost` written as the raw bytes), padded with LINEs
/// past LibreDWG's 256-byte minimum.
fn dxf_with_dimpost(acadver: &str, dimpost: &[u8]) -> Vec<u8> {
    let mut dxf = Vec::new();
    dxf.extend_from_slice(b"  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\n");
    dxf.extend_from_slice(acadver.as_bytes());
    dxf.extend_from_slice(b"\n  9\n$DIMPOST\n  1\n");
    dxf.extend_from_slice(dimpost);
    dxf.extend_from_slice(b"\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
    for i in 0..8 {
        dxf.extend_from_slice(
            format!("  0\nLINE\n  8\n0\n 10\n{i}.0\n 20\n0.0\n 11\n{i}.0\n 21\n1.0\n").as_bytes(),
        );
    }
    dxf.extend_from_slice(b"  0\nENDSEC\n  0\nEOF\n");
    dxf
}

#[test]
fn a_dimpost_suffix_round_trips_through_every_dxf_version() {
    // "<> mm" is the DXF reference's own example of $DIMPOST (the "<>" is
    // where the measurement goes); a 6-byte allocation that used to come
    // back as 13 UTF-16 units of heap. The R2018 non-ASCII variant is UTF-8
    // in the file (AC1021+ DXF), the R2000 one CP1252 (B5 = U+00B5, so the
    // default code page applies: no $DWGCODEPAGE, LibreDWG assumes ANSI_1252).
    let cases: [(&str, &[u8], &str); 4] = [
        ("AC1032", b"<> mm", "<> mm"),
        ("AC1015", b"<> mm", "<> mm"),
        ("AC1032", b"<> \xC2\xB5m", "<> \u{B5}m"),
        ("AC1015", b"<> \xB5m", "<> \u{B5}m"),
    ];
    for (acadver, raw, expected) in cases {
        let db = uncad::parse_bytes(&dxf_with_dimpost(acadver, raw), Format::Dxf)
            .expect("the DXF must parse");
        assert_eq!(db.header.dimpost, expected, "{acadver} {raw:?}");
    }
}

#[test]
fn header_survives_the_json_round_trip_and_defaults_when_absent() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializes");
    let back: CadDatabase = serde_json::from_str(&json).expect("deserializes");
    // serde_json's default float parser can land 1 ULP off (its
    // `float_roundtrip` feature makes it exact), so the comparison zeroes the
    // floats out and checks them separately with a tolerance -- see
    // `uncad::json`'s module doc.
    assert_eq!(strip_floats(&back.header), strip_floats(&db.header));
    for (a, b) in [
        (back.header.extmin.z, db.header.extmin.z),
        (back.header.pextmin.x, db.header.pextmin.x),
        (back.header.dimlfac, db.header.dimlfac),
        (back.header.limmax.x, db.header.limmax.x),
    ] {
        assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{a} vs {b}");
    }

    // A 0.2.0 document has no `header` key at all.
    let legacy = r#"{"entities":[],"tables":{"layers":{},"block_records":{},"mlinestyles":{}}}"#;
    let legacy: CadDatabase = serde_json::from_str(legacy).expect("0.2.0 JSON still loads");
    assert_eq!(legacy.header, uncad::Header::default());
    assert_eq!(legacy.header.units.name, "du");
}

#[test]
fn parse_bytes_reads_the_same_header() {
    let from_path = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let bytes = std::fs::read(EXAMPLE_2000_DWG).expect("readable");
    let from_bytes = uncad::parse_bytes(&bytes, Format::Dwg).expect("parses");
    assert_eq!(from_path.header, from_bytes.header);
}

/// A copy of `h` with every float field zeroed, so the integer and string
/// fields can be compared exactly.
fn strip_floats(h: &uncad::Header) -> uncad::Header {
    let zero3 = uncad::model::Point3D::default();
    let zero2 = uncad::model::Point2D::default();
    uncad::Header {
        units: Units {
            name: h.units.name.clone(),
            to_mm: h.units.to_mm.map(|_| 0.0),
        },
        extmin: zero3,
        extmax: zero3,
        pextmin: zero3,
        pextmax: zero3,
        limmin: zero2,
        limmax: zero2,
        plimmin: zero2,
        plimmax: zero2,
        dimscale: 0.0,
        dimlfac: 0.0,
        dimrnd: 0.0,
        dimtxt: 0.0,
        dimasz: 0.0,
        ltscale: 0.0,
        textsize: 0.0,
        ..h.clone()
    }
}
