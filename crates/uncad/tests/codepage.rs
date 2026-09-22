//! The string-decoding paths that review found wanting in the first cut of
//! the 0.3.0 code-page work (see `docs/ARCHITECTURE.md`, "Strings and
//! paths"): single-byte code pages must keep every character, an R2007+ DXF
//! (UTF-16 in memory) must decode at all, the strings LibreDWG's DXF reader
//! keeps 8-bit in an R2007+ DXF (MTEXT) must not be read as UTF-16, and a
//! corrupt code-page value in a file header must not crash the process. The
//! 8-bit fixtures are written by the tests themselves from group codes, as
//! `documented_invocations.rs` does, so the expected strings are the ones
//! the test encoded.

use std::path::PathBuf;

use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2018_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dwg"
);
const EXAMPLE_2007_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2007.dxf"
);
const EXAMPLE_2018_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dxf"
);
const TEXT_2007_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2007/Text.dxf"
);
const LEADER_2018_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2018/Leader.dxf"
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
    dxf_with_strings("AC1015", Some(codepage), texts, &[])
}

/// A DXF of version `acadver` (`$DWGCODEPAGE` only when given) with one
/// TEXT per entry of `texts` and one MTEXT per entry of `mtexts`, the
/// string bytes written as they are.
fn dxf_with_strings(
    acadver: &str,
    codepage: Option<&str>,
    texts: &[&[u8]],
    mtexts: &[&[u8]],
) -> Vec<u8> {
    let mut dxf = Vec::new();
    dxf.extend_from_slice(b"  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\n");
    dxf.extend_from_slice(acadver.as_bytes());
    dxf.extend_from_slice(b"\n");
    if let Some(codepage) = codepage {
        dxf.extend_from_slice(b"  9\n$DWGCODEPAGE\n  3\n");
        dxf.extend_from_slice(codepage.as_bytes());
        dxf.extend_from_slice(b"\n");
    }
    dxf.extend_from_slice(b"  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
    for (i, text) in texts.iter().enumerate() {
        dxf.extend_from_slice(b"  0\nTEXT\n  8\n0\n 10\n0.0\n 20\n");
        dxf.extend_from_slice(format!("{}.0\n", i * 5).as_bytes());
        dxf.extend_from_slice(b" 40\n2.5\n  1\n");
        dxf.extend_from_slice(text);
        dxf.extend_from_slice(b"\n");
    }
    for (i, text) in mtexts.iter().enumerate() {
        dxf.extend_from_slice(b"  0\nMTEXT\n  8\n0\n100\nAcDbMText\n 10\n50.0\n 20\n");
        dxf.extend_from_slice(format!("{}.0\n", i * 5).as_bytes());
        dxf.extend_from_slice(b" 40\n2.5\n 41\n50.0\n 71\n1\n  1\n");
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

fn text_values(db: &uncad::CadDatabase) -> Vec<String> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn mtext_values(db: &uncad::CadDatabase) -> Vec<String> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::MText(m) => Some(m.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn single_byte_code_pages_keep_every_character() {
    let dir = TempDir::new("codepage-8bit");

    // CP1251 "Стена" (five Cyrillic letters, all non-ASCII) and CP1252 "€€€"
    // (three-byte UTF-8 each): the cases where a 1.5x output buffer runs out.
    let cases: [(&str, &[u8], &str); 3] = [
        ("ANSI_1251", b"\xD1\xF2\xE5\xED\xE0", "Стена"),
        ("ANSI_1252", b"\x80\x80\x80", "€€€"),
        ("ANSI_1252", b"\xB1\xB0\xF8 100", "±°ø 100"),
    ];
    for (i, (codepage, bytes, expected)) in cases.iter().enumerate() {
        let path = dir.0.join(format!("case{i}.dxf"));
        std::fs::write(&path, dxf_with_texts(codepage, &[bytes])).expect("writable");
        let db = uncad::parse(&path).expect("the DXF must parse");
        assert_eq!(db.header.codepage_name, *codepage);
        assert_eq!(text_values(&db), vec![expected.to_string()], "{codepage}");
    }
}

#[test]
fn an_unmappable_byte_becomes_a_replacement_character_not_a_truncation() {
    let dir = TempDir::new("codepage-unmappable");
    // 0x81 is unassigned in CP1252; the text after it must survive.
    let path = dir.0.join("gap.dxf");
    std::fs::write(&path, dxf_with_texts("ANSI_1252", &[b"A\x81B"])).expect("writable");
    let db = uncad::parse(&path).expect("the DXF must parse");
    assert_eq!(text_values(&db), vec!["A\u{FFFD}B".to_string()]);
}

#[test]
fn r2007_and_later_dxf_input_decodes_its_utf16_strings() {
    for (path, version) in [(TEXT_2007_DXF, "r2007"), (LEADER_2018_DXF, "r2018")] {
        let db = uncad::parse(path).expect("corpus file must parse");
        assert_eq!(db.header.version, version, "{path}");
        // Before the fix every block name was cut at the first NUL of its
        // UTF-16 storage, "*Model_Space" read as "*", and nothing was selected.
        assert!(
            db.tables.block_records.contains_key("*Model_Space"),
            "{path}: {:?}",
            db.tables.block_records.keys().collect::<Vec<_>>()
        );
        assert!(!db.entities.is_empty(), "{path} should project to entities");
    }
}

#[test]
fn r2007_and_later_dxf_mtext_is_read_as_the_utf8_it_is_stored_as() {
    // in_dxf.c stores MTEXT's group 1/3 text 8-bit (strdup) with no R2007
    // branch, unlike every other T field, which becomes UTF-16. Reading it
    // as UTF-16 gave "\u{6554}\u{736b}..." ("Te", "ks" as one LE unit each)
    // plus whatever heap bytes followed the NUL. The reference is the same
    // drawing as a DWG, which LibreDWG converts itself.
    let dwg = uncad::parse(EXAMPLE_2018_DWG).expect("corpus file must parse");
    let mut expected = mtext_values(&dwg);
    expected.sort();
    assert_eq!(
        expected,
        vec!["Teksto granda nur por testi.\\PAlia linio.\\PAlia pli.".to_string()],
        "the reference drawing's MTEXT, as the DXF twin spells it too"
    );
    for path in [EXAMPLE_2007_DXF, EXAMPLE_2018_DXF] {
        let db = uncad::parse(path).expect("corpus file must parse");
        let mut got = mtext_values(&db);
        got.sort();
        assert_eq!(got, expected, "{path}");
    }
}

#[test]
fn an_r2018_dxf_carries_its_non_ascii_mtext_as_utf8_or_escapes() {
    // An AC1021+ DXF is UTF-8 by definition (older ones encode non-ASCII
    // as \U+XXXX escapes, which are still valid). "가나" is U+AC00 U+B098,
    // i.e. the bytes EA B0 80 EB 82 98 -- Python's `'가나 AB'.encode()`.
    let dir = TempDir::new("codepage-r2018-utf8");
    let hangul: &[u8] = b"\xEA\xB0\x80\xEB\x82\x98 AB";
    let escaped: &[u8] = b"\\U+AC00\\U+B098 AB";
    let path = dir.0.join("hangul.dxf");
    std::fs::write(
        &path,
        dxf_with_strings("AC1032", None, &[hangul, escaped], &[hangul, escaped]),
    )
    .expect("writable");
    let db = uncad::parse(&path).expect("the DXF must parse");
    assert_eq!(db.header.version, "r2018");
    // MTEXT (8-bit in memory): the UTF-8 bytes are kept, the escapes
    // expanded by the shim.
    assert_eq!(
        mtext_values(&db),
        vec!["가나 AB".to_string(), "가나 AB".to_string()]
    );
    // TEXT (UTF-16 in memory): the UTF-8 survives the round trip; the escape
    // is left for the decoder, which text_plain shows expanded.
    assert_eq!(
        text_values(&db),
        vec!["가나 AB".to_string(), "\\U+AC00\\U+B098 AB".to_string()]
    );
    let plain: Vec<String> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some(t.text_plain.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(plain, vec!["가나 AB".to_string(), "가나 AB".to_string()]);
}

#[test]
fn a_corrupt_code_page_in_the_file_header_does_not_crash() {
    // header.spec reads the code page as a raw 16-bit value at byte 0x13 of
    // an R2000 file; LibreDWG indexes its tables with it unchecked. 100 is
    // outside every table.
    let mut bytes = std::fs::read(EXAMPLE_2000_DWG).expect("corpus file is readable");
    assert_eq!(bytes[0x13], 30, "example_2000.dwg declares ANSI_1252 (30)");
    bytes[0x13] = 100;
    let db = uncad::parse_bytes(&bytes, uncad::Format::Dwg).expect("still a valid DWG");
    assert_eq!(
        db.header.codepage, 100,
        "the header reports what the file says"
    );
    assert!(
        db.header.codepage_name.is_empty(),
        "LibreDWG has no name for it"
    );
    // The strings were decoded through the ANSI_1252 fallback, not garbage.
    assert!(db.tables.layers.contains_key("Tavolo 3"));
}
