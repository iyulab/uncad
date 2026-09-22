//! The corpus sweep (docs/VLM_EXPORT_DESIGN.md, P9): every DWG and DXF under
//! LibreDWG's `test/test-data` is parsed and rendered to the default PNG, and
//! nothing may panic. Parse failures are allowed -- the corpus holds pre-R13
//! and deliberately odd files LibreDWG itself only partly reads -- but they
//! are counted and printed, so a regression in the number shows up in the
//! log. Ignored by default because it takes minutes: run it with
//!
//! ```text
//! cargo test -p uncad --test corpus_sweep -- --ignored --nocapture
//! ```
//!
//! and copy the summary into `docs/EVAL.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg") || e.eq_ignore_ascii_case("dxf"))
        {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct Tally {
    files: usize,
    parsed: usize,
    parse_failed: Vec<String>,
    rendered: usize,
    render_failed: Vec<String>,
    panicked: Vec<String>,
    entities: usize,
    excluded: usize,
    hidden: usize,
    ms: u128,
}

#[test]
#[ignore = "parses and renders the whole LibreDWG corpus; minutes"]
fn every_corpus_file_parses_or_fails_cleanly_and_never_panics() {
    let mut files = Vec::new();
    collect(Path::new(CORPUS), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "the corpus is checked out under lib/libredwg"
    );

    let mut by_ext: BTreeMap<String, Tally> = BTreeMap::new();
    for path in &files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let tally = by_ext.entry(ext).or_default();
        tally.files += 1;
        let rel = path
            .strip_prefix(CORPUS)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let started = Instant::now();
        let outcome = std::panic::catch_unwind(|| {
            let db = uncad::parse(path)?;
            let entities = db.entities.len();
            let png = db.to_png(uncad::ToPngOptions::default());
            Ok::<_, uncad::ParseError>((entities, png))
        });
        tally.ms += started.elapsed().as_millis();
        match outcome {
            Err(_) => tally.panicked.push(rel),
            Ok(Err(e)) => tally.parse_failed.push(format!("{rel} ({e})")),
            Ok(Ok((entities, png))) => {
                tally.parsed += 1;
                tally.entities += entities;
                match png {
                    Ok(png) => {
                        tally.rendered += 1;
                        tally.excluded += png.crop.excluded.len();
                        tally.hidden += png.hidden;
                    }
                    Err(e) => tally.render_failed.push(format!("{rel} ({e})")),
                }
            }
        }
    }

    eprintln!("\ncorpus sweep: {}", CORPUS);
    eprintln!(
        "| ext | files | parsed | rendered | entities | excluded by crop | hidden | seconds |"
    );
    eprintln!("|---|---|---|---|---|---|---|---|");
    let mut panics = Vec::new();
    for (ext, t) in &by_ext {
        eprintln!(
            "| {ext} | {} | {} | {} | {} | {} | {} | {:.1} |",
            t.files,
            t.parsed,
            t.rendered,
            t.entities,
            t.excluded,
            t.hidden,
            t.ms as f64 / 1000.0
        );
        for f in &t.parse_failed {
            eprintln!("  parse failed: {f}");
        }
        for f in &t.render_failed {
            eprintln!("  render failed: {f}");
        }
        panics.extend(t.panicked.iter().cloned());
    }
    assert!(panics.is_empty(), "panicked on: {panics:?}");
}
