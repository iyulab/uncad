//! Library-level coverage for `CadDatabase::write_dwg`.
//!
//! Until now the DWG encoder was reached only by one CLI test, from a
//! DXF-sourced database, and only as far as "the output re-parses". Two
//! properties the docs promise were checked nowhere: a DWG -> DWG round trip
//! keeps the drawing, and `write_dwg` refuses to overwrite an existing file
//! instead of clobbering it (`dwg_write_file` stat()s the target first --
//! see `docs/CAVEATS.md`, "DWG/DXF 쓰기 지원").
//!
//! Fixture: `lib/libredwg/test/test-data/2000/circle.dwg` from the
//! submodule-tracked LibreDWG corpus -- R2000, inside the <= R_2004 range the
//! docs describe as reliable for writing. Assertions are reference-free, as
//! in `dxf_pipeline.rs`: the drawing is its own expectation.

use std::fs;
use std::path::{Path, PathBuf};

const CORPUS_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/circle.dwg"
);

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl TempFile {
    fn new(name: &str) -> Self {
        TempFile(std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name)))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn a_dwg_survives_being_written_back_out_and_reparsed() {
    let mut db = uncad::parse(CORPUS_DWG).expect("a corpus DWG should parse");
    let before: Vec<String> = db
        .entities
        .iter()
        .map(|e| e.type_name().to_string())
        .collect();
    assert!(
        !before.is_empty(),
        "circle.dwg should project to at least one render entity"
    );

    let out = TempFile::new("roundtrip.dwg");
    db.write_dwg(out.path())
        .expect("writing an R2000 DWG should succeed");
    let written = fs::metadata(out.path())
        .expect("the encoder should have produced a file")
        .len();
    assert!(written > 0, "the encoder wrote an empty file");

    let reparsed = uncad::parse(out.path()).expect("what this crate wrote, it should read back");
    let after: Vec<String> = reparsed
        .entities
        .iter()
        .map(|e| e.type_name().to_string())
        .collect();
    assert_eq!(
        after, before,
        "entity type sequence changed across a DWG write/reparse round trip"
    );
}

#[test]
fn write_dwg_refuses_to_overwrite_an_existing_file() {
    let mut db = uncad::parse(CORPUS_DWG).expect("a corpus DWG should parse");

    let out = TempFile::new("existing.dwg");
    let sentinel = b"do not clobber me";
    fs::write(out.path(), sentinel).expect("temp dir should be writable");

    let err = db
        .write_dwg(out.path())
        .expect_err("dwg_write_file stat()s the target first and must refuse");
    assert!(
        matches!(err, uncad::WriteError::Critical(_)),
        "documented as surfacing upstream's refusal as WriteError::Critical, got: {err}"
    );
    assert_eq!(
        fs::read(out.path()).expect("the existing file should still be there"),
        sentinel,
        "the existing file must be left untouched"
    );
}
