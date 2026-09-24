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

use std::collections::BTreeMap;
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
    let result = iron_render_cad::to_svg(&db, iron_render_cad::ToSvgOptions::default());

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

// --- R2007+ DXF reads, and reads like its DWG twin ------------------------
//
// LibreDWG's DXF importer stores an R2007+ file's strings in two widths
// (`docs/CAVEATS.md`, "DXF saved as R2007 or later"), and until both were
// read in the right one such a file came back as a drawing with no entities
// and no error -- which is why it used to be refused. Two tests pin the
// reading from both sides the refusal was pinned from: every corpus DXF of
// that age, against the DWG of the same drawing where the corpus has one;
// and one drawing under several `$ACADVER` stamps, against itself.

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

/// A reference as one comparable string: the name, or its state.
fn ref_name(r: &uncad::model::Ref<String>) -> String {
    match r {
        uncad::model::Ref::Resolved(name) => name.clone(),
        uncad::model::Ref::Absent => "<absent>".to_string(),
        uncad::model::Ref::Unresolved(handle) => format!("<unresolved {handle}>"),
    }
}

/// What the parser states about a drawing that two decoders of the same
/// drawing must agree on: how many entities, on which layers, and which
/// block record each INSERT names.
fn names(db: &uncad::CadDatabase) -> (usize, BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut layers = BTreeMap::new();
    let mut blocks = BTreeMap::new();
    for entity in &db.entities {
        *layers.entry(ref_name(&entity.common().layer)).or_insert(0) += 1;
        if let uncad::Entity::Insert(insert) = entity {
            *blocks.entry(ref_name(&insert.block_name)).or_insert(0) += 1;
        }
    }
    (db.entities.len(), layers, blocks)
}

#[test]
fn every_r2007_plus_corpus_dxf_reads_like_its_dwg_twin_or_fails_in_libredwg() {
    let mut files = Vec::new();
    dxf_files_under(Path::new(CORPUS_ROOT), &mut files);
    files.sort();

    // Known deviations, each pinned with a tripwire: if the twins ever
    // agree, the entry is stale and must go.
    // - 2010/gh209_1: LibreDWG's importer leaves every entity without a
    //   layer handle (the DWG puts them on five layers); the model says so
    //   with `Absent`, which is what the parser owes.
    // - 2018/Leader: the DXF itself puts one LEADER on a layer "0 @ 1" that
    //   the DWG does not have (groups 8 at lines 1836 and 2606 of the file).
    let deviations = ["2010/gh209_1.dxf", "2018/Leader.dxf"];

    let (mut read, mut failed, mut twins) = (Vec::new(), Vec::new(), 0usize);
    for path in &files {
        let is_r2007_plus = acadver_by_search(path)
            .as_deref()
            .and_then(|v| v.strip_prefix("AC")?.parse::<u32>().ok())
            .is_some_and(|n| n >= 1021);
        if !is_r2007_plus {
            continue;
        }
        let rel = path
            .strip_prefix(CORPUS_ROOT)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let db = match uncad::parse(path) {
            Ok(db) => db,
            // LibreDWG's own reader gives up on these, as it does on the
            // older DXFs that fail: an error, never an empty drawing.
            Err(uncad::ParseError::Critical(_)) => {
                failed.push(rel);
                continue;
            }
            Err(e) => panic!("{rel}: {e}"),
        };
        assert!(!db.entities.is_empty(), "{rel} read as an empty drawing");
        read.push(rel.clone());

        let twin = path.with_extension("dwg");
        if !twin.exists() {
            continue;
        }
        twins += 1;
        let dwg = uncad::parse(&twin).expect("the DWG twin parses");
        let agree = names(&db) == names(&dwg);
        let deviates = deviations.contains(&rel.as_str());
        assert!(
            agree != deviates,
            "{rel}: {}\n  dxf {:?}\n  dwg {:?}",
            if deviates {
                "now agrees with its DWG twin -- drop it from `deviations`"
            } else {
                "must state what its DWG twin states"
            },
            names(&db),
            names(&dwg)
        );
    }
    eprintln!(
        "R2007+ corpus DXF: {} read ({twins} against a DWG twin), {} failed in LibreDWG: {failed:?}",
        read.len(),
        failed.len()
    );
    assert_eq!((read.len(), twins), (27, 24));
    assert_eq!(
        failed,
        [
            "2013/gh109_1.dxf",
            "2018/Constraints.dxf",
            "2018/Dynblocks.dxf",
            "2018/LiveSection1.dxf",
            "2018/TS1.dxf"
        ]
    );
}

/// The corpus R2000 drawing with its `$ACADVER` value line rewritten -- the
/// same file otherwise, so every reading of it differs in nothing but the
/// version stamp. (A synthetic HEADER holding only `$ACADVER` is not
/// enough for LibreDWG's importer, which is why the drawing is not built
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
fn the_same_drawing_reads_the_same_whatever_release_it_is_stamped() {
    // The drawing is its own reference: stamped R2007 and later, the
    // importer stores every string it sets as UTF-16 instead of 8-bit, and
    // nothing of what comes out may change -- not a name, not a text, not a
    // reference. Restamping with the file's own version is the null control.
    let reference = uncad::parse(CORPUS_DXF).expect("the untouched corpus DXF should parse");
    assert!(!reference.entities.is_empty());
    for acadver in ["AC1015", "AC1018", "AC1021", "AC1024", "AC1027", "AC1032"] {
        let stamped = TempFile::new(&format!("acadver-{acadver}.dxf"));
        fs::write(stamped.path(), corpus_dxf_stamped(acadver)).expect("temp dir writable");
        let (db, header) = uncad::parse_with_header(stamped.path())
            .unwrap_or_else(|e| panic!("the {acadver} stamp must be read: {e}"));
        assert_eq!(header.acadver.as_deref(), Some(acadver));
        // The one thing the stamp itself decides: from R2004 a release has
        // transparency, so an entity that states none is BYLAYER (0) there
        // and unstated before it.
        let mut expected = reference.clone();
        if acadver >= "AC1018" {
            fn bylayer(e: &mut uncad::Entity) {
                e.common_mut().transparency = Some(0);
                if let uncad::Entity::Insert(insert) = e {
                    for a in &mut insert.attribs {
                        a.common.transparency = Some(0);
                    }
                }
            }
            expected.entities.iter_mut().for_each(bylayer);
            for block in expected.tables.block_records.values_mut() {
                block.entities.iter_mut().for_each(bylayer);
            }
        }
        assert_eq!(db, expected, "the {acadver} stamp read differently");
    }
}
