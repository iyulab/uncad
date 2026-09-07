//! Runs the CLI calls the README and `--help` advertise.
//!
//! `main.rs` had no tests at all, so every documented invocation -- the
//! summary output, each `-o` extension, `--space`, `--scale` -- was verified
//! only by someone typing it. An option name could change, or a branch stop
//! producing a file, with `cargo test` still green.
//!
//! Fixtures come from the submodule-tracked LibreDWG corpus, the same source
//! `crates/uncad/src/png.rs` and `crates/uncad/tests/dxf_pipeline.rs` use.
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
fn converts_dwg_to_dxf() {
    let dxf = TempFile::new("converted.dxf");
    let out = run(&[CORPUS_DWG, "-o", dxf.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Readable by the same library that wrote it.
    assert!(uncad::parse(dxf.path()).is_ok());
}

#[test]
fn converts_dxf_to_dwg() {
    let dwg = TempFile::new("converted.dwg");
    let out = run(&[CORPUS_DXF, "-o", dwg.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(uncad::parse(dwg.path()).is_ok());
}

#[test]
fn writes_dxf_from_a_dxf_input_too() {
    // Regression guard: the "dxf" arm used to re-read `input` through
    // `dwg_to_dxf()`, which rejects a DXF input (LibreDWG code 2048) even
    // though the `write_dxf()` behind it accepts one. The library could do
    // this; the CLI could not.
    let dxf = TempFile::new("rewritten.dxf");
    let out = run(&[CORPUS_DXF, "-o", dxf.arg()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(uncad::parse(dxf.path()).is_ok());
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
    let other = TempFile::new("out.pdf");
    let out = run(&[CORPUS_DXF, "-o", other.arg()]);
    assert!(!out.status.success());
    assert!(!other.path().exists());
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
    for flag in ["--space", "--scale", "--no-trim"] {
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
