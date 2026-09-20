//! End-to-end coverage for the DXF half of the public API.
//!
//! `png.rs` already runs `parse()` -> `to_svg()` -> `to_png()` against a real
//! DWG from the LibreDWG submodule. Nothing did the same for DXF, and nothing
//! exercised the JSON export against a real drawing.
//!
//! Fixtures come from the same place `png.rs` takes its DWG: the
//! submodule-tracked LibreDWG corpus (a precondition of the tests only -- the
//! build compiles `libredwg-sys`'s vendored copy). That is deliberate --
//! `samples/README.md` explains why real-file tests cannot hang off
//! `samples/` (gitignored, so CI has nothing to read).
//!
//! The assertions avoid the trap that killed the previous file-based tests:
//! they pinned expected values that nobody could regenerate for a different
//! file without an independent reference. A **round trip** needs no such
//! reference -- the drawing is its own expectation. The remaining assertions
//! stay at the level of properties a drawing file has by construction, rather
//! than counts copied out of this project's own output.

use std::fs;
use std::path::{Path, PathBuf};

/// A DXF from the LibreDWG corpus (R2000, a version LibreDWG's DXF reader
/// handles well -- see `docs/CAVEATS.md`, "DXF reading"); the same fixture the
/// CLI tests use.
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
fn parses_a_dxf_and_projects_it_into_the_model() {
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
    let first_line: Option<uncad::model::Point3D> = db.entities.iter().find_map(|e| match e {
        uncad::Entity::Line(l) => Some(l.start_point),
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
fn a_parsed_dxf_survives_a_json_round_trip() {
    let db = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");

    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializing the model should succeed");
    assert!(
        json.starts_with('{'),
        "to_json should produce a JSON object: {json}"
    );

    // The drawing is its own expectation -- no external reference needed, and
    // this holds for any input file, so it does not rot when the fixture
    // changes. It spans decode -> model -> JSON -> model, and compares the
    // whole model (`PartialEq`), not a count.
    let back: uncad::CadDatabase =
        serde_json::from_str(&json).expect("what this crate wrote, it should read back");
    assert_eq!(back, db, "the model changed across a JSON round trip");
}

/// `CadDatabase` owns no C memory any more, so it should be an ordinary
/// value -- and `parse()` should have no side effect that makes a second
/// parse of the same file come out different. Both are stated in lib.rs;
/// this pins them.
#[test]
fn parse_is_deterministic_and_the_database_is_a_plain_value() {
    fn assert_plain<T: Send + Sync + Clone + PartialEq + std::fmt::Debug>() {}
    assert_plain::<uncad::CadDatabase>();

    let first = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");
    let second = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse again");
    assert_eq!(
        first, second,
        "two parses of one file should produce equal models"
    );
    assert_eq!(
        first.to_json(uncad::ToJsonOptions::default()).unwrap(),
        second.to_json(uncad::ToJsonOptions::default()).unwrap(),
        "and byte-identical JSON (sorted tables, file-ordered entities)"
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

#[test]
fn a_trace_written_by_a_cad_program_fills_as_a_simple_quadrilateral() {
    // TRACE lists its corners like SOLID does: 1-2 across the start, 3-4
    // across the end. Walking them 1-2-4-3 gives the outline; 1-2-3-4 would
    // cross itself. The renderer relies on that, so check it against a TRACE
    // a CAD program wrote rather than one written by hand for a test.
    let db = uncad::parse(CORPUS_DXF).expect("a corpus DXF should parse");
    let trace = db
        .entities
        .iter()
        .find_map(|e| match e {
            uncad::Entity::Trace(t) => Some(t),
            _ => None,
        })
        .expect("the corpus drawing contains a TRACE");

    let outline = [trace.corner1, trace.corner2, trace.corner4, trace.corner3];
    let turns: Vec<f64> = (0..4)
        .map(|i| {
            let (a, b, c) = (outline[i], outline[(i + 1) % 4], outline[(i + 2) % 4]);
            (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x)
        })
        .collect();
    assert!(
        turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0),
        "corners walked 1-2-4-3 should turn the same way at every corner: {turns:?}"
    );
}
