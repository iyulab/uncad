//! Runs the CLI calls the README and `--help` advertise.
//!
//! `main.rs` had no tests at all, so every documented invocation -- the
//! summary output, each `-o` extension, `--space`, `--scale` -- was verified
//! only by someone typing it. An option name could change, or a branch stop
//! producing a file, with `cargo test` still green.
//!
//! Fixtures come from the submodule-tracked LibreDWG corpus, the same source
//! `crates/uncad/src/png.rs` and `crates/uncad/tests/dxf_pipeline.rs` use --
//! plus, for `--no-trim`, a five-line DXF the test writes itself (see
//! `dxf_with_an_outlier`), because neither corpus file has an outlier for the
//! flag to act on.
//!
//! Assertions stay reference-free, as in `dxf_pipeline.rs`: that a file of the
//! right format appears, that an exit status is what it claims to be, and --
//! for options -- that the option *changes the result*, which is the property
//! that actually fails when a flag stops being wired up. Nothing here pins
//! bytes or counts copied out of this project's own output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXE: &str = env!("CARGO_BIN_EXE_uncad");

const CORPUS_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/circle.dwg"
);
const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
);

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl TempFile {
    fn new(name: &str) -> Self {
        TempFile(std::env::temp_dir().join(format!("uncad-cli-{}-{}", std::process::id(), name)))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn arg(&self) -> &str {
        self.0.to_str().expect("temp paths are UTF-8 here")
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(&self.0).expect("the command reported success, so the file should exist")
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .output()
        .expect("the test binary should be runnable")
}

/// Width and height from a PNG IHDR chunk.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "not a PNG: {:?}",
        &bytes[..bytes.len().min(8)]
    );
    let read = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (read(16), read(20))
}

#[test]
fn prints_a_summary_when_no_output_is_requested() {
    let out = run(&[CORPUS_DXF]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    // The summary is a count plus a per-type breakdown; a run that parsed
    // nothing would still print the header, so look for a type line too.
    assert!(
        stdout.contains("LINE"),
        "summary should list entity types: {stdout}"
    );
}

#[test]
fn writes_an_svg() {
    let svg = TempFile::new("out.svg");
    let out = run(&[CORPUS_DXF, "-o", svg.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let written = String::from_utf8(svg.bytes()).expect("SVG is text");
    assert!(written.contains("<svg"), "not an SVG document: {written}");
}

#[test]
fn writes_a_png() {
    let png = TempFile::new("out.png");
    let out = run(&[CORPUS_DXF, "-o", png.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (w, h) = png_size(&png.bytes());
    assert!(w > 0 && h > 0, "degenerate PNG: {w}x{h}");
}

#[test]
fn scale_actually_scales_the_raster() {
    let one = TempFile::new("scale1.png");
    let two = TempFile::new("scale2.png");

    assert!(run(&[CORPUS_DXF, "-o", one.arg()]).status.success());
    assert!(run(&[CORPUS_DXF, "-o", two.arg(), "--scale", "2"])
        .status
        .success());

    let (w1, h1) = png_size(&one.bytes());
    let (w2, h2) = png_size(&two.bytes());

    // The point of the flag. Comparing the two runs rather than asserting a
    // pixel count keeps this true for any fixture, and it is what fails if
    // `--scale` ever stops reaching `ToPngOptions`.
    assert!(
        w2 > w1 && h2 > h1,
        "--scale 2 should enlarge the raster: {w1}x{h1} -> {w2}x{h2}"
    );
}

#[test]
fn writes_json() {
    let json = TempFile::new("out.json");
    let out = run(&[CORPUS_DXF, "-o", json.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let value: serde_json::Value =
        serde_json::from_slice(&json.bytes()).expect("the output should be valid JSON");
    let entities = value["entities"]
        .as_array()
        .expect("`entities` should be an array");
    assert!(
        !entities.is_empty(),
        "a drawing with contents should export at least one entity"
    );
    assert!(value["tables"].is_object(), "`tables` should be an object");
    // The DXF type name is the tag a consumer dispatches on.
    assert!(
        entities.iter().all(|e| e["type"].is_string()),
        "every entity should carry a string `type`: {value}"
    );
    assert!(
        entities.iter().any(|e| e["type"] == "LINE"),
        "entities-2d.dxf contains LINE entities: {value}"
    );
}

#[test]
fn writes_json_from_a_dwg_too() {
    let json = TempFile::new("from-dwg.json");
    let out = run(&[CORPUS_DWG, "-o", json.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let value: serde_json::Value =
        serde_json::from_slice(&json.bytes()).expect("the output should be valid JSON");
    assert!(
        value["entities"]
            .as_array()
            .is_some_and(|entities| !entities.is_empty()),
        "circle.dwg should export at least one entity: {value}"
    );
    // The tables come along, and model space is one of the block records.
    let block_records = value["tables"]["block_records"]
        .as_object()
        .expect("`tables.block_records` should be an object");
    let model_space = block_records
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("*MODEL_SPACE"))
        .map(|(_, record)| record)
        .expect("a model-space block record should be exported");
    assert!(
        model_space["entities"]
            .as_array()
            .is_some_and(|entities| !entities.is_empty()),
        "model space should carry its entities: {model_space}"
    );
}

#[test]
fn pretty_changes_the_layout_but_not_the_content() {
    let compact = TempFile::new("compact.json");
    let pretty = TempFile::new("pretty.json");

    assert!(run(&[CORPUS_DXF, "-o", compact.arg()]).status.success());
    assert!(run(&[CORPUS_DXF, "-o", pretty.arg(), "--pretty"])
        .status
        .success());

    let compact_bytes = compact.bytes();
    let pretty_bytes = pretty.bytes();
    assert!(
        !compact_bytes.contains(&b'\n'),
        "the default should be a single line"
    );
    assert!(
        pretty_bytes.contains(&b'\n'),
        "--pretty should indent across lines"
    );

    let a: serde_json::Value = serde_json::from_slice(&compact_bytes).expect("valid JSON");
    let b: serde_json::Value = serde_json::from_slice(&pretty_bytes).expect("valid JSON");
    assert_eq!(a, b, "--pretty should change only whitespace");
}

#[test]
fn every_documented_space_is_accepted() {
    for space in ["model", "paper", "all"] {
        let svg = TempFile::new(&format!("space-{space}.svg"));
        let out = run(&[CORPUS_DXF, "-o", svg.arg(), "--space", space]);
        assert!(
            out.status.success(),
            "--space {space} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&svg.bytes()).contains("<svg"));
    }
}

#[test]
fn rejects_a_space_that_is_not_documented() {
    let svg = TempFile::new("bad-space.svg");
    let out = run(&[CORPUS_DXF, "-o", svg.arg(), "--space", "sideways"]);
    assert!(!out.status.success(), "an unknown --space should fail");
    assert!(!svg.path().exists(), "a rejected run should write nothing");
}

#[test]
fn rejects_a_scale_that_is_not_a_positive_number() {
    for bad in ["0", "-1", "abc"] {
        let png = TempFile::new(&format!("bad-scale-{bad}.png"));
        let out = run(&[CORPUS_DXF, "-o", png.arg(), "--scale", bad]);
        assert!(!out.status.success(), "--scale {bad} should be rejected");
        assert!(!png.path().exists(), "a rejected run should write nothing");
    }
}

#[test]
fn rejects_an_output_extension_it_cannot_write() {
    // `.dxf`/`.dwg` are the headline removal: the CLI used to write them.
    for name in ["out.pdf", "out.dxf", "out.dwg"] {
        let other = TempFile::new(name);
        let out = run(&[CORPUS_DXF, "-o", other.arg()]);
        assert!(!out.status.success(), "-o {name} should be refused");
        assert!(
            !other.path().exists(),
            "a refused run should write nothing for {name}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(".json, .svg, .png"),
            "the refusal should list what is supported: {stderr}"
        );
    }
}

#[test]
fn reports_a_missing_input_without_a_raw_library_code() {
    let missing = std::env::temp_dir().join("uncad-cli-definitely-not-here.dwg");
    let out = run(&[missing.to_str().unwrap()]);
    assert!(!out.status.success());

    // main.rs checks the path itself precisely so this case does not surface
    // as a bare LibreDWG number the user cannot act on.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("4096"),
        "a missing file should not report a raw library code: {stderr}"
    );
}

#[test]
fn help_exits_successfully_and_lists_the_options() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "--help should exit 0");

    let text = String::from_utf8_lossy(&out.stderr);
    for flag in ["--space", "--scale", "--no-trim", "--pretty"] {
        assert!(text.contains(flag), "usage should document {flag}: {text}");
    }
}

#[test]
fn no_arguments_is_a_usage_error() {
    let out = run(&[]);
    assert!(
        !out.status.success(),
        "invoking with no input should fail, not exit 0"
    );
}

#[test]
fn space_paper_renders_something_different_from_space_model() {
    // Distinct names from `every_documented_space_is_accepted`'s
    // `space-<name>.svg`: tests share one process (one pid), so two
    // TempFiles with the same name would delete each other's output.
    let model = TempFile::new("differential-model.svg");
    let paper = TempFile::new("differential-paper.svg");

    assert!(run(&[CORPUS_DXF, "-o", model.arg(), "--space", "model"])
        .status
        .success());
    assert!(run(&[CORPUS_DXF, "-o", paper.arg(), "--space", "paper"])
        .status
        .success());

    let model_svg = String::from_utf8(model.bytes()).expect("SVG is text");
    let paper_svg = String::from_utf8(paper.bytes()).expect("SVG is text");
    assert!(
        model_svg.contains("<line") || model_svg.contains("<path"),
        "model space of entities-2d.dxf should carry geometry: {model_svg}"
    );
    // `every_documented_space_is_accepted` only proves each value exits 0.
    // This is the property that fails when `--space` stops reaching
    // `ToSvgOptions`: the two renders would then be the same document.
    assert_ne!(
        model_svg, paper_svg,
        "--space paper should not render the same document as --space model"
    );
}

/// A DXF written from group codes by the test itself: four LINEs forming a
/// 10x10 square at the origin, plus one LINE a million units away. Authoring
/// the fixture is what makes a *differential* `--no-trim` test possible --
/// neither corpus file has an outlier, so on them the flag is (correctly) a
/// no-op and a test could not tell a wired-up flag from an ignored one.
fn dxf_with_an_outlier() -> String {
    fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
        format!(
            "  0\nLINE\n  8\n0\n 10\n{x1}\n 20\n{y1}\n 30\n0.0\n 11\n{x2}\n 21\n{y2}\n 31\n0.0\n"
        )
    }
    let mut dxf = String::from("  0\nSECTION\n  2\nENTITIES\n");
    dxf.push_str(&line(0.0, 0.0, 10.0, 0.0));
    dxf.push_str(&line(10.0, 0.0, 10.0, 10.0));
    dxf.push_str(&line(10.0, 10.0, 0.0, 10.0));
    dxf.push_str(&line(0.0, 10.0, 0.0, 0.0));
    dxf.push_str(&line(1_000_000.0, 1_000_000.0, 1_000_001.0, 1_000_001.0));
    dxf.push_str("  0\nENDSEC\n  0\nEOF\n");
    dxf
}

/// The `viewBox` attribute's value from an SVG document.
fn view_box(svg: &[u8]) -> String {
    let text = String::from_utf8_lossy(svg);
    let start = text
        .find("viewBox=\"")
        .expect("the SVG root should carry a viewBox")
        + "viewBox=\"".len();
    let end = text[start..]
        .find('"')
        .expect("the viewBox attribute should be closed")
        + start;
    text[start..end].to_string()
}

#[test]
fn no_trim_keeps_the_outlier_inside_the_viewbox() {
    let input = TempFile::new("outlier.dxf");
    fs::write(input.path(), dxf_with_an_outlier()).expect("temp dir should be writable");

    let trimmed = TempFile::new("trimmed.svg");
    let untrimmed = TempFile::new("untrimmed.svg");
    let out = run(&[input.arg(), "-o", trimmed.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run(&[input.arg(), "-o", untrimmed.arg(), "--no-trim"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let width = |vb: &str| -> f64 {
        vb.split_whitespace()
            .nth(2)
            .and_then(|w| w.parse().ok())
            .expect("viewBox has four numbers")
    };
    let vb_trimmed = view_box(&trimmed.bytes());
    let vb_untrimmed = view_box(&untrimmed.bytes());

    // Default: the far-away line is trimmed, so the box is about the square.
    // --no-trim: the box has to reach the outlier a million units out.
    assert!(
        width(&vb_trimmed) < 1_000.0,
        "the default should trim the outlier: viewBox {vb_trimmed}"
    );
    assert!(
        width(&vb_untrimmed) > 1_000_000.0,
        "--no-trim should keep the outlier inside the viewBox: {vb_untrimmed}"
    );
}

/// `uncad export <input> -o <dir>` writes a finished package (its
/// manifest is the last file written) and refuses an option it does not
/// know rather than ignoring it.
#[test]
fn export_writes_a_package_and_refuses_unknown_options() {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/example_2000.dwg"
    );
    let dir = std::env::temp_dir().join(format!("uncad-cli-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = std::process::Command::new(EXE)
        .args(["export", input, "-o"])
        .arg(&dir)
        .args(["--max-levels", "0"])
        .output()
        .expect("the binary runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("manifest.json").is_file());
    assert!(dir.join("overview.png").is_file());
    let _ = std::fs::remove_dir_all(&dir);

    let bad = std::process::Command::new(EXE)
        .args(["export", input, "--bogus"])
        .output()
        .expect("the binary runs");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown option '--bogus'"));
}

/// `--version` answers on both commands; an option the plain command does
/// not know, a missing value and a second input are errors, not ignored.
#[test]
fn version_and_the_parsers_refusals() {
    let run = |args: &[&str]| {
        std::process::Command::new(EXE)
            .args(args)
            .output()
            .expect("the binary runs")
    };
    let v = run(&["--version"]);
    assert!(v.status.success());
    assert_eq!(
        String::from_utf8_lossy(&v.stdout).trim(),
        format!("uncad {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(run(&["export", "--version"]).status.success());
    for (args, needle) in [
        (&["x.dwg", "--bogus"][..], "unknown option '--bogus'"),
        (&["--bogus", "x.dwg"][..], "unknown option '--bogus'"),
        (&["x.dwg", "y.dwg"][..], "unexpected argument 'y.dwg'"),
        (&["x.dwg", "-o"][..], "-o needs a value"),
    ] {
        let out = run(args);
        assert!(!out.status.success(), "{args:?} should fail");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(needle), "{args:?}: {err}");
    }
}

/// `--include-hidden` draws what the layer rules leave out, so the SVG of a
/// drawing with hidden entities grows.
#[test]
fn include_hidden_draws_more() {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../uncad/tests/fixtures/hidden_layers_r2000.dxf"
    );
    let dir = std::env::temp_dir().join(format!("uncad-cli-hidden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let render = |name: &str, extra: &[&str]| {
        let path = dir.join(name);
        let out = std::process::Command::new(EXE)
            .arg(input)
            .arg("-o")
            .arg(&path)
            .args(extra)
            .output()
            .expect("the binary runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::read_to_string(&path).unwrap()
    };
    let plain = render("plain.svg", &[]);
    let all = render("all.svg", &["--include-hidden"]);
    assert!(all.len() > plain.len(), "{} vs {}", all.len(), plain.len());
    let _ = std::fs::remove_dir_all(&dir);
}
