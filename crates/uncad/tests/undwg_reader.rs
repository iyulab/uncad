//! The `undwg-reader` feature: the second DWG reader gives the same header.
//!
//! Every DWG of the corpus both readers read must give the same [`Header`]
//! field by field, except the fields listed in `KNOWN_DIFFERENCES`, each with
//! the reason. A drawing only LibreDWG reads must be one of a version `undwg`
//! names and refuses (R13 and R14, and older).

#![cfg(feature = "undwg-reader")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uncad::{parse_dwg_with_reader, DwgReader};

/// A header field the two readers state differently, and why.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[(
    "codepage_name",
    "undwg names only the code pages its table covers (28 to 30); LibreDWG names more",
)];

/// A header field LibreDWG leaves out and undwg reads from the file.
const MORE_STATED: &[(&str, &str)] = &[(
    "measurement",
    "the template of the versions that keep it in a section states it; LibreDWG does not read it there",
)];

fn dwg_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            dwg_files(&path, into);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
        {
            into.push(path);
        }
    }
}

#[test]
fn the_two_readers_give_the_same_header_field_by_field() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib/libredwg/test/test-data");
    let mut files = Vec::new();
    dwg_files(&corpus, &mut files);
    assert!(!files.is_empty(), "no DWG in {}", corpus.display());

    let mut compared = 0usize;
    let mut only_libredwg = 0usize;
    let mut differences: BTreeMap<String, usize> = BTreeMap::new();
    let mut more_stated: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok((_, libredwg)) = parse_dwg_with_reader(&bytes, DwgReader::LibreDwg) else {
            continue;
        };
        let undwg = match parse_dwg_with_reader(&bytes, DwgReader::Undwg) {
            Ok((_, header)) => header,
            Err(e) => {
                // Only the versions undwg names and refuses may be missing.
                assert!(
                    undwg::detect_version(&bytes).is_err()
                        || matches!(
                            undwg::detect_version(&bytes),
                            Ok(v) if v < undwg::Version::R2000
                        ),
                    "{}: undwg refused a version it reads: {e}",
                    path.display()
                );
                only_libredwg += 1;
                continue;
            }
        };
        compared += 1;
        let a = serde_json::to_value(&libredwg).expect("header as json");
        let b = serde_json::to_value(&undwg).expect("header as json");
        let (a, b) = (
            a.as_object().expect("object"),
            b.as_object().expect("object"),
        );
        for (key, left) in a {
            let right = b.get(key);
            if right != Some(left) {
                let stated_more = left.is_null()
                    && right.is_some_and(|r| !r.is_null())
                    && MORE_STATED.iter().any(|(name, _)| name == &key.as_str());
                if stated_more {
                    *more_stated
                        .entry(format!(
                            "{key} ({})",
                            libredwg.acadver.as_deref().unwrap_or("?")
                        ))
                        .or_default() += 1;
                    continue;
                }
                *differences.entry(key.clone()).or_default() += 1;
                examples.entry(key.clone()).or_insert_with(|| {
                    format!("{}: libredwg {left} / undwg {:?}", path.display(), right)
                });
            }
        }
    }
    eprintln!(
        "{compared} drawings compared · {only_libredwg} only LibreDWG reads · differing fields: {differences:?} · stated by undwg only: {more_stated:?}"
    );
    for (field, example) in &examples {
        eprintln!("  {field}: {example}");
    }
    let unexplained: Vec<&String> = differences
        .keys()
        .filter(|k| {
            !KNOWN_DIFFERENCES
                .iter()
                .any(|(name, _)| name == &k.as_str())
        })
        .collect();
    assert!(
        unexplained.is_empty(),
        "header fields the readers state differently with no reason listed: {unexplained:?}"
    );
    assert!(compared > 0);
}
