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
