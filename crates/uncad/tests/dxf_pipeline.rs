//! End-to-end coverage for the DXF half of the public API.
//!
//! `png.rs` already runs `parse()` -> `to_svg()` -> `to_png()` against a real
//! DWG from the LibreDWG submodule. Nothing did the same for DXF, and nothing
//! at all exercised `write_dxf()` -- the encoder is a separate LibreDWG code
//! path from the decoder, so a green suite said nothing about whether writing
//! worked.
//!
//! Fixtures come from the same place `png.rs` takes its DWG: the
//! submodule-tracked LibreDWG corpus, which `libredwg-sys` already requires to
//! build. That is deliberate -- `samples/README.md` explains why real-file
//! tests cannot hang off `samples/` (gitignored, so CI has nothing to read).
//!
//! The assertions avoid the trap that killed the previous file-based tests:
//! they pinned expected values that nobody could regenerate for a different
//! file without an independent reference. A **round trip** needs no such
//! reference -- the drawing is its own expectation. The remaining assertions
//! stay at the level of properties a drawing file has by construction, rather
//! than counts copied out of this project's own output.

use std::fs;
use std::path::{Path, PathBuf};

/// A DXF from the LibreDWG corpus. R2000, inside the version range
/// `docs/CAVEATS.md` documents as reliable for writing.
const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
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
fn parses_a_dxf_and_projects_it_into_the_render_model() {
    let db = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");

    // A drawing file that produced no entities would mean the decode path
    // silently returned an empty database -- which is exactly what an
    // `entities.len() >= 0` style assertion would wave through.
    assert!(
        !db.entities.is_empty(),
        "a drawing with contents should project to at least one render entity"
    );

    // Naming the type, not just reading a field through it: a caller that
    // wants to keep a point has to be able to write it down. This stops
    // compiling if `Point3D` ever becomes unnameable from outside the crate
    // again.
    let first_line: Option<uncad::render_model::Point3D> =
        db.entities.iter().find_map(|e| match e {
            uncad::RenderEntity::Line(l) => Some(l.start_point),
            _ => None,
        });
    assert!(
        first_line.is_some(),
        "entities-2d.dxf contains LINE entities; got {:?}",
        db.entities
    );
}

#[test]
fn renders_a_parsed_dxf_to_svg() {
    let db = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");
    let result = db.to_svg(uncad::ToSvgOptions::default());

    // Deliberately not asserted: which entity types come back unsupported.
    // Pinning that set would turn every future *widening* of renderer support
    // into a test failure.
    assert!(
        result.svg.contains("<line") || result.svg.contains("<path"),
        "the rendered document should carry geometry, not just a wrapper: {}",
        result.svg
    );
    assert!(
        result.svg.starts_with("<svg") || result.svg.contains("<svg"),
        "output should be an SVG document: {}",
        result.svg
    );
}

#[test]
fn a_dxf_survives_being_written_back_out_and_reparsed() {
    let mut db = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");
    let before = db.entities.len();

    let out = TempFile::new("roundtrip.dxf");
    db.write_dxf(out.path())
        .expect("writing R2000 DXF should succeed");

    let written = fs::metadata(out.path())
        .expect("the encoder should have produced a file")
        .len();
    assert!(written > 0, "the encoder wrote an empty file");

    let reparsed = uncad::parse(out.path()).expect("what this crate wrote, it should read back");

    // The drawing is its own expectation -- no external reference needed, and
    // this holds for any input file, so it does not rot when the fixture
    // changes. It spans decode -> render projection -> encode -> decode.
    assert_eq!(
        reparsed.entities.len(),
        before,
        "entity count changed across a write/reparse round trip"
    );
}

#[test]
fn reports_an_error_instead_of_panicking_on_a_file_that_is_not_a_drawing() {
    let garbage = TempFile::new("garbage.dxf");
    fs::write(garbage.path(), b"this is not a DXF file").expect("temp dir should be writable");

    // The FFI boundary is the one place a malformed input could take the
    // process down rather than return. This asserts it returns.
    assert!(uncad::parse(garbage.path()).is_err());
}
