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
    let result = uncad::to_svg(&db, uncad::ToSvgOptions::default());

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

#[test]
fn no_corpus_example_makes_the_parser_panic() {
    // Whether each of these parses is LibreDWG's business (see
    // `docs/CAVEATS.md`, "DXF reading"); that none of them brings the process
    // down is this crate's. A polyface mesh with an unused face slot used to
    // underflow an index here, in debug builds only.
    let dir = Path::new(CORPUS_DXF)
        .parent()
        .and_then(Path::parent)
        .expect("the corpus fixture sits two levels below test-data");
    let mut seen = 0;
    for entry in fs::read_dir(dir).expect("the corpus checkout should be readable") {
        let path = entry.expect("a readable directory entry").path();
        let is_example = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("example_") && n.ends_with(".dxf"));
        if is_example {
            let _ = uncad::parse(&path);
            seen += 1;
        }
    }
    assert!(seen >= 5, "expected the corpus example DXFs, found {seen}");
}

// --- R2007+ DXF is refused, and only R2007+ -------------------------------
//
// LibreDWG reads a DXF saved as R2007 or later without any error and hands
// back a drawing with no entities (`docs/CAVEATS.md`, "DXF reading"). The
// crate now decides from `$ACADVER` before LibreDWG sees the file. Two tests
// pin that decision from both sides: an input that must be refused, and one
// that must keep reading -- a check that only looked at the refusal could not
// tell "R2007+ is rejected" from "every DXF is rejected".

/// Root of the LibreDWG corpus (submodule); every `.dxf` under it is walked.
const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

/// An independent reading of `$ACADVER`, deliberately naive so that it shares
/// no code with the crate: the whole file as text, the value line after the
/// variable's group code 1. `None` when the file does not declare one.
fn acadver_by_search(path: &Path) -> Option<String> {
    let text = String::from_utf8_lossy(&fs::read(path).ok()?).into_owned();
    let at = text.find("$ACADVER")?;
    let mut rest = text[at..].lines().skip(1).map(str::trim);
    if rest.next()? != "1" {
        return None;
    }
    rest.next().map(str::to_string)
}

fn dxf_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("corpus directory should be readable") {
        let path = entry.expect("corpus entry should be readable").path();
        if path.is_dir() {
            dxf_files_under(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
        {
            out.push(path);
        }
    }
}

#[test]
fn refuses_exactly_the_corpus_dxfs_saved_as_r2007_or_later() {
    let mut files = Vec::new();
    dxf_files_under(Path::new(CORPUS_ROOT), &mut files);
    files.sort();
    assert!(!files.is_empty(), "the corpus should contain DXF files");

    let (mut refused, mut accepted) = (0usize, 0usize);
    for path in &files {
        let declared = acadver_by_search(path);
        let is_r2007_plus = declared
            .as_deref()
            .and_then(|v| v.strip_prefix("AC")?.parse::<u32>().ok())
            .is_some_and(|n| n >= 1021);

        match uncad::parse(path) {
            Err(uncad::ParseError::UnsupportedDxfVersion(v)) => {
                assert!(
                    is_r2007_plus,
                    "{} was refused as {v} but declares {declared:?}",
                    path.display()
                );
                assert_eq!(Some(v.as_str()), declared.as_deref(), "{}", path.display());
                refused += 1;
            }
            other => {
                assert!(
                    !is_r2007_plus,
                    "{} declares {declared:?} (R2007+) but was not refused: {:?}",
                    path.display(),
                    other.map(|db| db.entities.len())
                );
                accepted += 1;
            }
        }
    }
    // Both sides must be populated for the test to have discriminated at all.
    assert!(
        refused > 0,
        "no corpus DXF was refused -- the version check is not running"
    );
    assert!(
        accepted > 0,
        "every corpus DXF was refused -- the version check is too broad"
    );
    eprintln!("corpus DXF: {refused} refused (R2007+), {accepted} handed to LibreDWG");
}

/// The corpus R2000 drawing with its `$ACADVER` value line rewritten -- the
/// same file otherwise, so the two sides of the control differ in nothing but
/// the version stamp. (A synthetic HEADER holding only `$ACADVER` is not
/// enough for LibreDWG's importer, which is why the control is not built
/// from scratch.)
fn corpus_dxf_stamped(acadver: &str) -> String {
    let text = fs::read_to_string(CORPUS_DXF).expect("the corpus DXF should be readable text");
    let at = text
        .find("$ACADVER")
        .expect("the corpus DXF declares $ACADVER");
    let (head, tail) = text.split_at(at);
    // tail = "$ACADVER\n  1\nAC1015\n..." -- replace the third line.
    let mut lines = tail.splitn(4, '\n');
    let (var, code, _old, rest) = (
        lines.next().unwrap(),
        lines.next().unwrap(),
        lines.next().unwrap(),
        lines.next().unwrap_or(""),
    );
    assert_eq!(
        code.trim(),
        "1",
        "the group code after $ACADVER should be 1"
    );
    format!("{head}{var}\n{code}\n{acadver}\n{rest}")
}

#[test]
fn the_same_drawing_is_refused_at_r2007_and_read_at_r2004() {
    let r2007 = TempFile::new("acadver-r2007.dxf");
    fs::write(r2007.path(), corpus_dxf_stamped("AC1021")).expect("temp dir writable");
    match uncad::parse(r2007.path()) {
        Err(uncad::ParseError::UnsupportedDxfVersion(v)) => assert_eq!(v, "AC1021"),
        other => panic!(
            "an R2007 DXF must be refused, got {:?}",
            other.map(|db| db.entities.len())
        ),
    }

    // Restamping with the file's own version is the null control: it proves
    // the rewrite itself changes nothing before the other stamps are judged.
    let reference = uncad::parse(CORPUS_DXF).expect("the untouched corpus DXF should parse");
    let same = TempFile::new("acadver-r2000.dxf");
    fs::write(same.path(), corpus_dxf_stamped("AC1015")).expect("temp dir writable");
    let db = uncad::parse(same.path()).expect("the R2000 restamp must still be read");
    assert_eq!(
        db.entities.len(),
        reference.entities.len(),
        "restamping with the same version must not change what is read"
    );

    // One step below the threshold: handed to LibreDWG and read. Only the
    // *decision* is asserted here -- how many entities LibreDWG's importer
    // produces for an R2004 stamp of R2000 content is its business (it does
    // differ: 14 against 12 for this file), and pinning it would test the
    // importer, not this crate's version check.
    let r2004 = TempFile::new("acadver-r2004.dxf");
    fs::write(r2004.path(), corpus_dxf_stamped("AC1018")).expect("temp dir writable");
    let db = uncad::parse(r2004.path()).expect("an R2004 DXF must still be read");
    assert!(
        !db.entities.is_empty(),
        "the R2004 control should read entities"
    );
}
