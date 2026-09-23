//! The drawing's `Header` on real corpus files: version, code page, units
//! and the header variables a consumer needs to read the model's numbers.
//! The expected values are the ones the files state (cross-checked against
//! the DXF twin's `$` variables where one exists), so the drawing is its own
//! reference; nothing here is a number copied from this crate's output.
//! And what a file does not state is `None`, never the value LibreDWG's
//! struct happens to hold for it.

use uncad::model::{Point2D, Ref};
use uncad::{Format, Header, Units};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);
const EXAMPLE_R14_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_r14.dwg"
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

fn header_of(path: &str) -> Header {
    uncad::parse_with_header(path)
        .unwrap_or_else(|e| panic!("{path} must parse: {e}"))
        .1
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-6)
}

#[test]
fn example_2000_dwg_header_reads_the_dxf_variables() {
    let h = header_of(EXAMPLE_2000_DWG);

    assert_eq!(h.format, Format::Dwg);
    assert_eq!(h.acadver.as_deref(), Some("AC1015"));
    assert_eq!(h.version.as_deref(), Some("r2000"));
    assert_eq!(
        (h.codepage, h.codepage_name.as_deref()),
        (30, Some("ANSI_1252"))
    );

    // $INSUNITS 4 = millimetres, $MEASUREMENT 1 = metric.
    assert_eq!(h.insunits, Some(4));
    assert_eq!(
        h.units(),
        Some(Units {
            name: "mm".into(),
            to_mm: Some(1.0)
        })
    );
    assert_eq!(h.measurement, Some(1));
    // $LUNITS 2 decimal, $DIMDEC 2, $DIMLFAC 1.0 -- what the file declares.
    assert_eq!((h.lunits, h.dimdec), (Some(2), Some(2)));
    assert!(close(h.dimlfac, 1.0));
    // $LIMMAX is the A3 sheet in mm.
    assert_eq!(h.limmax, Some(Point2D { x: 420.0, y: 297.0 }));
    // The stored model extents include a 3256x-scaled INSERT far off the
    // sheet: kept verbatim, not "corrected".
    let (extmin, extmax) = (h.extmin.expect("stated"), h.extmax.expect("stated"));
    assert!(
        extmin.x < -2_000_000.0 && extmax.x > 800_000.0,
        "{extmin:?}..{extmax:?}"
    );
    // Paper-space extents were never set: AutoCAD's +/-1e20 sentinel, verbatim.
    let (pextmin, pextmax) = (h.pextmin.expect("stated"), h.pextmax.expect("stated"));
    assert!(
        pextmin.x > 1e19 && pextmax.x < -1e19,
        "{pextmin:?}..{pextmax:?}"
    );
    assert_eq!(h.clayer, Ref::Resolved("Tavolo 3".to_string()));
}

#[test]
fn the_dxf_twin_declares_the_same_units_and_dimension_variables() {
    let (a, b) = (header_of(EXAMPLE_2000_DWG), header_of(EXAMPLE_2000_DXF));

    assert_eq!(b.format, Format::Dxf);
    assert_eq!((&a.acadver, &a.version), (&b.acadver, &b.version));
    assert_eq!(a.insunits, b.insunits);
    assert_eq!(a.measurement, b.measurement);
    assert_eq!(
        (a.lunits, a.luprec, a.aunits, a.auprec),
        (b.lunits, b.luprec, b.aunits, b.auprec)
    );
    assert_eq!(
        (a.dimdec, a.dimlunit, a.dimzin, a.dimfrac, a.dimaunit, a.dimadec),
        (b.dimdec, b.dimlunit, b.dimzin, b.dimfrac, b.dimaunit, b.dimadec)
    );
    for (dwg, dxf, name) in [
        (a.dimlfac, b.dimlfac, "DIMLFAC"),
        (a.dimscale, b.dimscale, "DIMSCALE"),
        (a.dimtxt, b.dimtxt, "DIMTXT"),
        (a.dimasz, b.dimasz, "DIMASZ"),
        (a.ltscale, b.ltscale, "LTSCALE"),
        (a.textsize, b.textsize, "TEXTSIZE"),
    ] {
        assert!(
            close(dxf, dwg.expect("stated")),
            "{name}: {dwg:?} vs {dxf:?}"
        );
    }
    assert_eq!(a.limmax, b.limmax);
    assert_eq!(a.dimpost, b.dimpost);
    assert_eq!(a.clayer, b.clayer);
    // A DXF names its code page in $DWGCODEPAGE, which the reader records.
    assert_eq!(b.codepage_name.as_deref(), Some("ANSI_1252"));
}

#[test]
fn an_r14_dwg_states_no_variable_its_header_does_not_carry() {
    // $INSUNITS, $DIMADEC, $DIMFRAC and $DIMLUNIT entered the DWG header in
    // R2000: an R14 file cannot state them, whatever LibreDWG's struct holds.
    let h = header_of(EXAMPLE_R14_DWG);
    assert_eq!(h.version.as_deref(), Some("r14"));
    assert_eq!(
        (h.insunits, h.dimadec, h.dimfrac, h.dimlunit),
        (None, None, None, None)
    );
    assert_eq!(h.units(), None);
    // What an R14 header does carry is stated.
    assert!(h.lunits.is_some() && h.dimdec.is_some() && h.dimscale.is_some());
    assert!(h.extmin.is_some() && h.limmax.is_some());
    assert!(matches!(h.clayer, Ref::Resolved(_)), "{:?}", h.clayer);
}

#[test]
fn a_pre_r13_dwg_states_what_its_header_count_covers() {
    // A pre-R13 header is one run of variables, as long as the file header
    // says (header.spec: AC2.10 holds 83, AC1009 204/205). R2.10 stops
    // before $DIMZIN, $DIMPOST and $DIMLFAC; R11 carries them and the
    // paper-space extents. R1.4 stops sooner still, after $LUPREC.
    let r2_10 = header_of(&format!("{TEST_DATA}r2.10/entities.dwg"));
    assert_eq!(r2_10.version.as_deref(), Some("r2.10"));
    assert!(r2_10.dimscale.is_some() && r2_10.dimtxt.is_some());
    assert_eq!((r2_10.dimzin, r2_10.dimlfac), (None, None));
    assert_eq!((r2_10.dimpost, r2_10.pextmin), (None, None));

    let r11 = header_of(&format!("{TEST_DATA}r11/entities-2d.dwg"));
    assert_eq!(r11.version.as_deref(), Some("r11"));
    assert!(close(r11.dimlfac, 1.0));
    assert_eq!(r11.dimpost.as_deref(), Some(""));
    assert!(r11.pextmin.is_some());
    // Never in a pre-R13 header, whatever its length.
    assert_eq!(
        (r11.insunits, r11.dimdec, r11.measurement),
        (None, None, None)
    );

    let r1_4 = header_of(&format!("{TEST_DATA}r1.4/entities.dwg"));
    assert_eq!(r1_4.version.as_deref(), Some("r1.4"));
    assert!(r1_4.lunits.is_some() && r1_4.limmax.is_some());
    assert_eq!(
        (r1_4.aunits, r1_4.dimscale, r1_4.ltscale),
        (None, None, None)
    );
}

#[test]
fn a_chinese_r2013_file_reports_its_code_page_and_unitless_units() {
    let (db, h) = uncad::parse_with_header(GH109_2013_DWG).expect("corpus file must parse");
    assert_eq!(h.version.as_deref(), Some("r2013"));
    assert_eq!(
        (h.codepage, h.codepage_name.as_deref()),
        (39, Some("ANSI_936"))
    );
    // $INSUNITS 0 is stated -- "unitless" -- which is not the same as absent.
    assert_eq!(h.insunits, Some(0));
    assert_eq!(
        h.units(),
        Some(Units {
            name: "du".into(),
            to_mm: None
        })
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
    // in an R2007+ DXF. Read as UTF-16, the 1-byte allocation of an empty
    // $DIMPOST was overrun into "DIMSE..."-style heap garbage on every one of
    // these files.
    for name in R2007_PLUS_DXFS {
        let (db, h) = uncad::parse_with_header(format!("{TEST_DATA}{name}"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            ["r2007", "r2010", "r2013", "r2018"].contains(&h.version.as_deref().unwrap_or("")),
            "{name}: {:?}",
            h.version
        );
        assert_eq!(h.dimpost.as_deref(), Some(""), "{name}");
        assert!(
            !db.read_diagnostics
                .warnings
                .iter()
                .any(|w| w.starts_with("TEXT_ENCODING")),
            "{name}: {:?}",
            db.read_diagnostics.warnings
        );
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
    // in the file (AC1021+ DXF), the R2000 one CP1252 (B5 = U+00B5; with no
    // $DWGCODEPAGE, LibreDWG assumes ANSI_1252).
    let cases: [(&str, &[u8], &str); 4] = [
        ("AC1032", b"<> mm", "<> mm"),
        ("AC1015", b"<> mm", "<> mm"),
        ("AC1032", b"<> \xC2\xB5m", "<> \u{B5}m"),
        ("AC1015", b"<> \xB5m", "<> \u{B5}m"),
    ];
    for (acadver, raw, expected) in cases {
        let (_, h) = uncad::parse_bytes_with_header(&dxf_with_dimpost(acadver, raw), Format::Dxf)
            .expect("the DXF must parse");
        assert_eq!(h.dimpost.as_deref(), Some(expected), "{acadver} {raw:?}");
    }
}

#[test]
fn a_dxf_states_only_the_variables_its_header_section_names() {
    // LibreDWG fills the rest of its header struct with zeros or its own
    // defaults; none of those may come out as if the file had said them.
    let (_, h) = uncad::parse_bytes_with_header(&dxf_with_dimpost("AC1015", b"<>"), Format::Dxf)
        .expect("the DXF must parse");
    assert_eq!(h.acadver.as_deref(), Some("AC1015"));
    assert_eq!(h.dimpost.as_deref(), Some("<>"));
    assert_eq!(
        (h.insunits, h.measurement, h.lunits, h.dimdec, h.dimzin),
        (None, None, None, None, None)
    );
    assert_eq!((h.extmin, h.limmax), (None, None));
    assert_eq!((h.dimscale, h.dimlfac, h.ltscale), (None, None, None));
    assert_eq!(h.clayer, Ref::Absent);

    // A DXF without $ACADVER (pre-R10) states no version either: LibreDWG
    // assumes R11 for it, which is a default, not what the file says.
    let text = String::from_utf8(dxf_with_dimpost("AC1015", b"<>")).expect("ASCII");
    let headerless = text.replace("  9\n$ACADVER\n  1\nAC1015\n", "");
    let (_, h) = uncad::parse_bytes_with_header(headerless.as_bytes(), Format::Dxf)
        .expect("the DXF must parse");
    assert_eq!((h.acadver, h.version), (None, None));
}

#[test]
fn the_header_serializes_with_unstated_variables_as_null() {
    let h = header_of(EXAMPLE_R14_DWG);
    let json = serde_json::to_value(&h).expect("serializes");
    assert_eq!(json["format"], "dwg");
    assert_eq!(json["acadver"], "AC1014");
    assert!(json["insunits"].is_null());
    assert_eq!(json["clayer"]["type"], "RESOLVED");
    let back: Header = serde_json::from_value(json).expect("deserializes");
    assert_eq!(back, h);
}

#[test]
fn parse_bytes_reads_the_same_header() {
    let (_, from_path) = uncad::parse_with_header(EXAMPLE_2000_DWG).expect("parses");
    let bytes = std::fs::read(EXAMPLE_2000_DWG).expect("readable");
    let (_, from_bytes) = uncad::parse_bytes_with_header(&bytes, Format::Dwg).expect("parses");
    assert_eq!(from_path, from_bytes);
    // And the database beside it is the one parse() returns.
    let (db, _) = uncad::parse_with_header(EXAMPLE_2000_DWG).expect("parses");
    assert_eq!(db, uncad::parse(EXAMPLE_2000_DWG).expect("parses"));
}
