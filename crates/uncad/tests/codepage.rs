//! How a drawing's 8-bit strings come out, end to end: single-byte code
//! pages keep every character, a byte the declared code page has no
//! character for is U+FFFD *and* a `TEXT_ENCODING` warning, the DOS-era
//! double-byte pages (Big5, GB2312, CP932) pair only the bytes that can
//! pair, and a code page LibreDWG has no table for is reported and read as
//! UTF-8. The 8-bit fixtures are written by the
//! tests themselves from group codes, so the expected strings are the ones
//! the test encoded; `text.rs` has the byte-level cases.

use std::path::PathBuf;

use uncad::{CadDatabase, Entity, Format};

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("uncad-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("temp dir should be writable");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An AC1015 DXF declaring `codepage` in `$DWGCODEPAGE`, with one TEXT per
/// entry of `texts` (already encoded in that code page).
fn dxf_with_texts(codepage: &str, texts: &[&[u8]]) -> Vec<u8> {
    let mut dxf = Vec::new();
    dxf.extend_from_slice(b"  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n");
    dxf.extend_from_slice(b"  9\n$DWGCODEPAGE\n  3\n");
    dxf.extend_from_slice(codepage.as_bytes());
    dxf.extend_from_slice(b"\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
    for (i, text) in texts.iter().enumerate() {
        dxf.extend_from_slice(b"  0\nTEXT\n  8\n0\n 10\n0.0\n 20\n");
        dxf.extend_from_slice(format!("{}.0\n", i * 5).as_bytes());
        dxf.extend_from_slice(b" 40\n2.5\n  1\n");
        dxf.extend_from_slice(text);
        dxf.extend_from_slice(b"\n");
    }
    // LibreDWG's DXF reader rejects files under 256 bytes; pad with LINEs,
    // which the assertions never look at.
    for i in 0..8 {
        dxf.extend_from_slice(
            format!("  0\nLINE\n  8\n0\n 10\n{i}.0\n 20\n0.0\n 11\n{i}.0\n 21\n1.0\n").as_bytes(),
        );
    }
    dxf.extend_from_slice(b"  0\nENDSEC\n  0\nEOF\n");
    dxf
}

fn text_values(db: &CadDatabase) -> Vec<String> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn text_encoding_warnings(db: &CadDatabase) -> Vec<&String> {
    db.read_diagnostics
        .warnings
        .iter()
        .filter(|w| w.starts_with("TEXT_ENCODING"))
        .collect()
}

#[test]
fn single_byte_code_pages_keep_every_character() {
    let dir = TempDir::new("codepage-8bit");
    // CP1251 "Стена" (five Cyrillic letters, all non-ASCII) and CP1252 "€€€"
    // (three bytes of UTF-8 each): the cases where the library's own
    // converter, with its 1.5x output buffer, loses the tail.
    let cases: [(&str, &[u8], &str); 3] = [
        ("ANSI_1251", b"\xD1\xF2\xE5\xED\xE0", "Стена"),
        ("ANSI_1252", b"\x80\x80\x80", "€€€"),
        ("ANSI_1252", b"\xB1\xB0\xF8 100", "±°ø 100"),
    ];
    for (i, (codepage, bytes, expected)) in cases.iter().enumerate() {
        let path = dir.0.join(format!("case{i}.dxf"));
        std::fs::write(&path, dxf_with_texts(codepage, &[bytes])).expect("writable");
        let (db, header) = uncad::parse_with_header(&path).expect("the DXF must parse");
        assert_eq!(header.codepage_name.as_deref(), Some(*codepage));
        assert_eq!(text_values(&db), vec![expected.to_string()], "{codepage}");
        assert!(text_encoding_warnings(&db).is_empty(), "{codepage}");
    }
}

#[test]
fn an_unmappable_byte_becomes_a_reported_replacement_character_not_a_truncation() {
    // 0x81 is unassigned in CP1252; the text after it must survive, and the
    // gap must be reported rather than papered over.
    let bytes = dxf_with_texts("ANSI_1252", &[b"A\x81B"]);
    let db = uncad::parse_bytes(&bytes, Format::Dxf).expect("the DXF must parse");
    assert_eq!(text_values(&db), vec!["A\u{FFFD}B".to_string()]);
    let warnings = text_encoding_warnings(&db);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("TEXT.text_value") && warnings[0].contains("ANSI_1252 (30)"),
        "{}",
        warnings[0]
    );
}

#[test]
fn the_dos_era_double_byte_code_pages_do_not_pair_ascii() {
    // The library says every byte of Big5 (24) and GB2312 (31) opens a pair,
    // so "*Model_Space" was consumed as "*M", "od", ... and no entity was
    // ever selected. Patch the declared code page (byte 0x13 of an R2000
    // file header) to the two real values and to CP932, which is decoded as
    // double-byte too: the ASCII names must survive and the drawing keep the
    // entities its declared ANSI_1252 gives.
    let reference = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file is readable");
    assert!(!reference.entities.is_empty());
    let bytes = std::fs::read(EXAMPLE_2000_DWG).expect("corpus file is readable");
    assert_eq!(bytes[0x13], 30, "example_2000.dwg declares ANSI_1252 (30)");
    for (cp, name) in [(24u8, "BIG5"), (31, "GB2312"), (22, "CP932")] {
        let mut patched = bytes.clone();
        patched[0x13] = cp;
        let (db, header) =
            uncad::parse_bytes_with_header(&patched, Format::Dwg).expect("still a valid DWG");
        assert_eq!(header.codepage_name.as_deref(), Some(name));
        assert_eq!(db.entities.len(), reference.entities.len(), "{name}");
        assert!(
            db.tables.layers.contains_key("Tavolo 3"),
            "{name}: {:?}",
            db.tables.layers.keys().collect::<Vec<_>>()
        );
        assert!(
            db.tables.block_records.contains_key("*Model_Space"),
            "{name}"
        );
    }
}

#[test]
fn the_dos_era_double_byte_code_pages_decode_their_cjk_pairs() {
    // Byte sequences from Python: '中国 AB'.encode('gb2312') (EUC-CN),
    // '中文 AB'.encode('big5'), '日本 AB'.encode('shift_jis') (CP932). The
    // Windows twins 936/950/932 of the same bytes are the control group.
    // The last CP932 case keeps 0x5C a backslash (LibreDWG's table says
    // yen), so the \U+ escape it opens and a \P stay what they are; an
    // escape is text as far as the model is concerned, and is not expanded.
    let cases: [(&str, &[u8], &str); 7] = [
        ("GB2312", b"\xD6\xD0\xB9\xFA AB", "中国 AB"),
        ("ANSI_936", b"\xD6\xD0\xB9\xFA AB", "中国 AB"),
        ("BIG5", b"\xA4\xA4\xA4\xE5 AB", "中文 AB"),
        ("ANSI_950", b"\xA4\xA4\xA4\xE5 AB", "中文 AB"),
        ("CP932", b"\x93\xFA\x96\x7B AB", "日本 AB"),
        ("ANSI_932", b"\x93\xFA\x96\x7B AB", "日本 AB"),
        ("CP932", b"\x93\xFA\\P\\U+00B1", "日\\P\\U+00B1"),
    ];
    for (codepage, bytes, expected) in cases {
        let (db, header) =
            uncad::parse_bytes_with_header(&dxf_with_texts(codepage, &[bytes]), Format::Dxf)
                .expect("the DXF must parse");
        assert_eq!(header.codepage_name.as_deref(), Some(codepage));
        assert_eq!(text_values(&db), vec![expected.to_string()], "{codepage}");
        assert!(text_encoding_warnings(&db).is_empty(), "{codepage}");
    }
}

#[test]
fn a_corrupt_code_page_in_the_file_header_is_reported_not_guessed() {
    // header.spec reads the code page as a raw 16-bit value at byte 0x13 of
    // an R2000 file; LibreDWG indexes its tables with it unchecked. 100 is
    // outside every table, so it must never reach them.
    let mut bytes = std::fs::read(EXAMPLE_2000_DWG).expect("corpus file is readable");
    assert_eq!(bytes[0x13], 30, "example_2000.dwg declares ANSI_1252 (30)");
    bytes[0x13] = 100;
    let (db, header) =
        uncad::parse_bytes_with_header(&bytes, Format::Dwg).expect("still a valid DWG");
    assert_eq!(
        header.codepage, 100,
        "the header reports what the file says"
    );
    assert_eq!(header.codepage_name, None, "LibreDWG has no name for it");
    // ASCII reads the same in every code page and is never looked up: the
    // names survive. The one string with CP1252 bytes in it (an MTEXT) is
    // not decoded as 1252 by guesswork -- it is read as UTF-8, which it is
    // not, and said so.
    assert!(db.tables.layers.contains_key("Tavolo 3"));
    let warnings = text_encoding_warnings(&db);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("MTEXT.text")
            && warnings[0].contains("codepage 100 cannot be decoded"),
        "{}",
        warnings[0]
    );

    // A non-ASCII string under an unknown code page is read as UTF-8 when it
    // is UTF-8, and reported when it is not -- never decoded as 1252.
    let dxf = String::from_utf8(dxf_with_texts("ANSI_1252", &[b"X"])).expect("ASCII");
    for (text, expected, warned) in [
        ("caf\u{E9}".as_bytes(), "caf\u{E9}", false),
        (b"caf\xE9".as_slice(), "caf\u{FFFD}", true),
    ] {
        let mut bytes = dxf.replace("ANSI_1252", "BOGUS_CP").into_bytes();
        let at = bytes
            .windows(3)
            .position(|w| w == b"\nX\n")
            .expect("the text value")
            + 1;
        bytes.splice(at..at + 1, text.iter().copied());
        let (db, header) =
            uncad::parse_bytes_with_header(&bytes, Format::Dxf).expect("the DXF must parse");
        assert_eq!(header.codepage_name, None, "{:?}", header.codepage);
        assert_eq!(text_values(&db), vec![expected.to_string()]);
        assert_eq!(
            !text_encoding_warnings(&db).is_empty(),
            warned,
            "{expected}"
        );
    }
}
