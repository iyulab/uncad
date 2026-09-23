//! `parse()` reads the file itself and decodes from memory (`parse_bytes`),
//! so a path with non-ASCII characters works on Windows too -- LibreDWG's
//! own `dwg_read_file` opens paths with `fopen()`, which the MSVC runtime
//! interprets in the ANSI code page, and a Korean directory name failed with
//! `DWG_ERR_IOERROR` (critical error 4096). The fixtures are the LibreDWG
//! corpus's one-entity `circle.dwg` and the R2000 DXF the other tests read;
//! the assertions stay reference-free (a count, and parse-vs-parse_bytes
//! equality).

use std::path::PathBuf;

const CIRCLE_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/circle.dwg"
);
const ENTITIES_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
);

/// A per-test directory under the OS temp dir, removed on drop.
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

#[test]
fn parse_opens_a_dwg_under_a_korean_path() {
    let dir = TempDir::new("한글경로");
    let target = dir.0.join("도면.dwg");
    std::fs::copy(CIRCLE_DWG, &target).expect("copying the fixture should succeed");

    let db = uncad::parse(&target).expect("a path with Hangul in it must parse");
    assert_eq!(db.entities.len(), 1, "circle.dwg holds exactly one entity");
}

#[test]
fn parse_opens_a_dxf_under_a_korean_path() {
    let dir = TempDir::new("한글경로-dxf");
    let target = dir.0.join("도면.dxf");
    std::fs::copy(ENTITIES_DXF, &target).expect("copying the fixture should succeed");

    let from_korean_path = uncad::parse(&target).expect("a path with Hangul in it must parse");
    let from_ascii_path = uncad::parse(ENTITIES_DXF).expect("the corpus path must parse");
    assert_eq!(from_korean_path, from_ascii_path);
}

#[test]
fn parse_bytes_gives_the_same_model_as_parse() {
    for (path, format) in [
        (CIRCLE_DWG, uncad::Format::Dwg),
        (ENTITIES_DXF, uncad::Format::Dxf),
    ] {
        let from_path = uncad::parse(path).expect("the corpus file must parse");
        let bytes = std::fs::read(path).expect("the corpus file must be readable");
        let from_bytes = uncad::parse_bytes(&bytes, format).expect("the same bytes must parse");
        assert_eq!(from_path, from_bytes, "{path}");
    }
}

#[test]
fn format_from_path_only_treats_dxf_as_dxf() {
    assert_eq!(uncad::Format::from_path("a/b.DXF"), uncad::Format::Dxf);
    assert_eq!(uncad::Format::from_path("a/b.dwg"), uncad::Format::Dwg);
    assert_eq!(uncad::Format::from_path("no-extension"), uncad::Format::Dwg);
}

#[test]
fn a_missing_file_is_an_io_error() {
    let err = uncad::parse("this-file-does-not-exist.dwg").expect_err("must fail");
    assert!(matches!(err, uncad::ParseError::Io(_)), "{err:?}");
    // The cause travels with it, and the message says what failed.
    assert!(std::error::Error::source(&err).is_some());
    assert!(err.to_string().starts_with("cannot read the file"), "{err}");
}

#[test]
fn garbage_bytes_are_a_critical_decode_error() {
    for format in [uncad::Format::Dwg, uncad::Format::Dxf] {
        let err = uncad::parse_bytes(b"this is not a drawing, in any format", format)
            .expect_err("must fail");
        assert!(
            matches!(err, uncad::ParseError::Critical(_)),
            "{format:?}: {err:?}"
        );
    }
}
