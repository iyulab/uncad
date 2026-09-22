//! A second, independent DWG reader as an oracle.
//!
//! This crate reads DWG through one engine. A defect in that engine cannot
//! be seen from inside it: the reader and the expectation share the mistake.
//! These tests read the same corpus files through an unrelated Rust
//! implementation and compare what the two agree on.
//!
//! The comparison is deliberately coarse for now -- it establishes that the
//! second reader opens the same files and reports entities in the same
//! order of magnitude. What the two disagree about, field by field, is the
//! measurement this file grows into.
//!
//! The second reader is a development dependency: nothing a consumer builds
//! reaches it.

use std::path::{Path, PathBuf};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

/// The versions this crate's corpus holds a full set of drawings for, newest
/// first. The second reader states R13..R2018+ support, so a version outside
/// that range is not a disagreement -- it is out of scope, and named here so
/// the distinction stays visible rather than silently untested.
const VERSIONS: &[&str] = &["2018", "2013", "2010", "2007", "2004", "2000", "r14"];

fn drawings_for(version: &str) -> Vec<PathBuf> {
    let dir = Path::new(CORPUS).join(version);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
        })
        .collect();
    out.sort();
    out
}

fn second_reader_entity_count(path: &Path) -> Option<usize> {
    let mut reader = acadrust::DwgReader::from_file(path).ok()?;
    let document = reader.read().ok()?;
    Some(document.entities().count())
}

/// Reading the same file twice through the second reader must give the same
/// answer. This is the property that decides whether it can sit behind a
/// deterministic surface at all: its dependency graph carries a randomly
/// seeded hasher and a work-stealing thread pool, either of which can leak
/// ordering into an output.
#[test]
fn the_second_reader_gives_the_same_count_for_the_same_file() {
    let mut compared = 0usize;
    let mut unstable: Vec<String> = Vec::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Some(first) = second_reader_entity_count(&path) else {
                continue;
            };
            compared += 1;
            for _ in 0..3 {
                let again = second_reader_entity_count(&path);
                if again != Some(first) {
                    unstable.push(format!("{}: {first} then {again:?}", path.display()));
                    break;
                }
            }
        }
    }
    assert!(compared > 0, "the second reader opened no corpus drawing");
    assert!(
        unstable.is_empty(),
        "{} of {compared} drawings read differently on a repeat:\n{}",
        unstable.len(),
        unstable.join("\n")
    );
}

/// How far the two readers agree, per version, pinned as a measurement.
///
/// These are not requirements. They are what was observed, recorded so that
/// a change in either reader shows up as a diff instead of passing unseen.
/// A count of drawings the second reader *opens*, out of those this crate
/// opens -- the coarsest agreement there is, and the one every finer
/// comparison depends on.
#[test]
fn how_many_drawings_both_readers_open_is_what_it_was_when_last_measured() {
    let mut report = String::new();
    for version in VERSIONS {
        let drawings = drawings_for(version);
        if drawings.is_empty() {
            continue;
        }
        let mut ours = 0usize;
        let mut theirs = 0usize;
        for path in &drawings {
            if uncad::parse(path).is_ok() {
                ours += 1;
            }
            if second_reader_entity_count(path).is_some() {
                theirs += 1;
            }
        }
        report.push_str(&format!(
            "{version}: {ours} / {theirs} of {}\n",
            drawings.len()
        ));
    }
    // Written by the first run of this test; a diff here is the signal.
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-agreement.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "\nthe two readers' agreement moved; measured now:\n{report}"
    );
}

/// Whether the second reader can say "this reference points at nothing".
///
/// This crate's model tells three states apart -- a name that resolved, a
/// reference the file makes that nothing answers to, and no reference at
/// all -- because a consumer that cannot tell them apart will read a made-up
/// name as a real one. The question here is whether a second reader's own
/// model carries enough for those three states to be recovered from it.
///
/// The probe is a file naming a block it does not define, which is the same
/// case this crate's own reference tests use.
#[test]
fn what_the_second_reader_says_about_a_reference_to_a_missing_block() {
    let pairs: &[(u16, &str)] = &[
        (0, "SECTION"),
        (2, "BLOCKS"),
        (0, "BLOCK"),
        (8, "0"),
        (2, "REAL"),
        (70, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (3, "REAL"),
        (0, "LINE"),
        (8, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (11, "1.0"),
        (21, "0.0"),
        (31, "0.0"),
        (0, "ENDBLK"),
        (8, "0"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "INSERT"),
        (8, "0"),
        (2, "REAL"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (0, "INSERT"),
        (8, "0"),
        (2, "NOBLOCK"),
        (10, "5.0"),
        (20, "0.0"),
        (30, "0.0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    let text: String = pairs
        .iter()
        .map(|(code, value)| format!("{code:>3}\n{value}\n"))
        .collect();
    let path = std::env::temp_dir().join(format!(
        "uncad-{}-second-reader-dangling-block.dxf",
        std::process::id()
    ));
    std::fs::write(&path, text).expect("temp dir writable");

    let reader =
        acadrust::DxfReader::from_file(&path).expect("the second reader should open the probe");
    let document = reader.read().expect("the second reader should read it");
    let names: Vec<String> = document
        .entities()
        .filter_map(|entity| match entity {
            acadrust::EntityType::Insert(insert) => Some(insert.block_name.clone()),
            _ => None,
        })
        .collect();
    let _ = std::fs::remove_file(&path);

    // Measured, not required: the second reader carries block references as
    // plain names, so the block it cannot find comes back as the name the
    // file wrote. That is the right answer for a *name*, and it is also the
    // reason a consumer cannot tell it from a block that exists -- nothing
    // in the value says which. Recovering the third state from this model
    // means asking the document whether a block of that name is defined.
    assert_eq!(names, vec!["REAL".to_string(), "NOBLOCK".to_string()]);
    assert!(
        document.block_records.contains("REAL"),
        "the defined block should be in the block table"
    );
    assert!(
        !document.block_records.contains("NOBLOCK"),
        "the undefined block must not be in the block table -- that absence \
         is what a consumer would have to consult to recover the third state"
    );
}

/// Where the two readers stand on layer references, per version.
///
/// This crate carries a layer reference as three states. The second reader
/// carries it as a name, and its DWG path fills a name it could not resolve
/// with the literal `"0"` -- a layer name every drawing really has. So a
/// failed resolution and a genuine layer 0 are the same value there, and the
/// handle that would tell them apart is not kept on the entity (its linetype
/// handle is, which is what makes the omission visible rather than a matter
/// of taste).
///
/// The corpus is clean, so the collapse is latent here, not active: this
/// pins that both readers agree on every drawing in it. A disagreement
/// appearing later is either a real defect or the latent case arriving.
#[test]
fn the_two_readers_agree_on_every_layer_name_in_the_corpus() {
    let mut disagreements: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let (Ok(ours), Some(theirs)) = (uncad::parse(&path), second_reader_layers(&path))
            else {
                continue;
            };
            let mut our_names: Vec<String> = ours
                .all_entities()
                .filter_map(|e| e.common().layer.resolved().cloned())
                .collect();
            our_names.sort_unstable();
            our_names.dedup();
            let mut their_names = theirs;
            their_names.sort_unstable();
            their_names.dedup();
            compared += 1;
            if our_names != their_names {
                disagreements.push(format!(
                    "{}: ours {our_names:?} vs theirs {their_names:?}",
                    path.display()
                ));
            }
        }
    }
    assert!(compared > 0, "no drawing was read by both");
    assert!(
        disagreements.is_empty(),
        "{} of {compared} drawings disagree on the set of layer names:
{}",
        disagreements.len(),
        disagreements.join(
            "
"
        )
    );
}

fn second_reader_layers(path: &Path) -> Option<Vec<String>> {
    let mut reader = acadrust::DwgReader::from_file(path).ok()?;
    let document = reader.read().ok()?;
    Some(
        document
            .entities()
            .map(|entity| entity.common().layer.clone())
            .collect(),
    )
}
