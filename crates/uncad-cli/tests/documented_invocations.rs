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
/// A drawing with texts, dimensions, blocks and regions, so `uncad export`
/// writes every kind of record file.
const CORPUS_EXAMPLE_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
/// This project's own fixture, read only: nine LINEs of which four are
/// hidden (`crates/uncad/tests/fixtures/README.md`).
const HIDDEN_LAYERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../uncad/tests/fixtures/hidden_layers_r2000.dxf"
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

/// `--output` is the long spelling of `-o`, in both commands (USAGE's
/// "Both commands" block, the README's CLI section). It reached neither,
/// so nothing exercised the arm that accepts it.
#[test]
fn output_is_the_long_form_of_o() {
    let short = TempFile::new("long-form-short.svg");
    let long = TempFile::new("long-form-long.svg");
    ok(&run(&[CORPUS_DXF, "-o", short.arg()]), "-o");
    ok(&run(&[CORPUS_DXF, "--output", long.arg()]), "--output");
    assert_eq!(
        short.bytes(),
        long.bytes(),
        "--output should write what -o writes"
    );

    // And under `uncad export`, where it names the package directory.
    let dir = TempDir::new("long-form-export");
    let mut args = vec!["export", CORPUS_DXF, "--output", dir.arg()];
    args.extend_from_slice(&QUICK);
    ok(&run(&args), "export --output");
    assert!(
        dir.join("manifest.json").exists(),
        "export --output should fill the package directory"
    );
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

    // Both runs pass --scale: the default sizing is a pixel fit, not a
    // unit multiplier, so "no flag" would not be the 1x baseline. The
    // lattice is off so a small drawing is not rounded up to 28 px twice.
    assert!(run(&[
        CORPUS_DXF,
        "-o",
        one.arg(),
        "--scale",
        "1",
        "--lattice",
        "0"
    ])
    .status
    .success());
    assert!(run(&[
        CORPUS_DXF,
        "-o",
        two.arg(),
        "--scale",
        "2",
        "--lattice",
        "0"
    ])
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
    // Every flag a test in this file passes, so a flag cannot be added to
    // the parser without being documented (or documented without a test).
    for flag in [
        "--output",
        // Asserted against the binary in release_invariants.rs; listed here
        // so the "every flag this file passes is documented" rule still
        // covers it.
        "--version",
        "--pretty",
        "--space",
        "--crop",
        "--padding",
        "--no-trim",
        "--include-hidden",
        "--fit",
        "--ppu",
        "--scale",
        "--bg",
        "--stroke",
        "--max-edge",
        "--lattice",
        "--fonts",
        "--profile",
        "--max-levels",
        "--max-tiles",
        "--text-px",
        "--shard-kb",
        "--frame-gap",
        "--min-frame-entities",
        "--max-frames",
        "--no-sheets",
        "--svg",
        "--full",
    ] {
        assert!(text.contains(flag), "usage should document {flag}: {text}");
    }

    // The subcommand answers it too, rather than asking for an input file.
    let out = run(&["export", "--help"]);
    assert!(out.status.success(), "export --help should exit 0");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("uncad export"),
        "export --help should print the usage"
    );
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
    let mut dxf = String::from("  0\nSECTION\n  2\nENTITIES\n");
    dxf.push_str(&square_lines());
    dxf.push_str(&dxf_line(
        1_000_000.0,
        1_000_000.0,
        1_000_001.0,
        1_000_001.0,
    ));
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

// ---------------------------------------------------------------------------
// The flags the README and `--help` document, and the parser's refusals.
//
// Before these, `uncad export` had no test at all and neither had any PNG or
// crop flag: an export-only flag before `<input>` was taken *as* the input
// ("cannot open input file '--max-levels'"), and a misspelt flag anywhere was
// dropped by a `_ => {}` arm, so the run exited 0 with the defaults applied.
// Every assertion below derives its expected number from the documented rule
// (the comment says how), not from a value copied out of this project's own
// output.
// ---------------------------------------------------------------------------

/// A directory for one `uncad export` package, removed on drop.
///
/// Distinct names per test, for the reason
/// `space_paper_renders_something_different_from_space_model` records: the
/// whole file runs in one process, so two of these sharing a name would
/// delete each other's package mid-run.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("uncad-cli-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn arg(&self) -> &str {
        self.0.to_str().expect("temp paths are UTF-8 here")
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    /// The package's `manifest.json`, parsed.
    fn manifest(&self) -> serde_json::Value {
        let bytes = fs::read(self.join("manifest.json"))
            .expect("the command reported success, so manifest.json should exist");
        serde_json::from_slice(&bytes).expect("manifest.json should be JSON")
    }
}

/// `uncad export <input> -o <dir> [flags]`, the documented form.
fn export(input: &str, dir: &TempDir, flags: &[&str]) -> Output {
    let mut args = vec!["export", input, "-o", dir.arg()];
    args.extend_from_slice(flags);
    run(&args)
}

fn ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Levels and sheets cost seconds and most flags below do not touch them;
/// the two tests that do pass their own.
const QUICK: [&str; 3] = ["--max-levels", "0", "--no-sheets"];

/// One LINE in DXF group codes, on layer 0 at z = 0.
fn dxf_line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
    format!("  0\nLINE\n  8\n0\n 10\n{x1}\n 20\n{y1}\n 30\n0.0\n 11\n{x2}\n 21\n{y2}\n 31\n0.0\n")
}

/// The four LINEs of a 10 x 10 square with its lower-left corner at the
/// origin: a drawing whose content rectangle is exactly 10 x 10 units, so a
/// pixel count can be derived from `--ppu` and `--padding` by hand.
fn square_lines() -> String {
    let mut dxf = String::new();
    dxf.push_str(&dxf_line(0.0, 0.0, 10.0, 0.0));
    dxf.push_str(&dxf_line(10.0, 0.0, 10.0, 10.0));
    dxf.push_str(&dxf_line(10.0, 10.0, 0.0, 10.0));
    dxf.push_str(&dxf_line(0.0, 10.0, 0.0, 0.0));
    dxf
}

/// That square, written to `file`.
fn write_square(file: &TempFile) {
    let dxf = format!(
        "  0\nSECTION\n  2\nENTITIES\n{}  0\nENDSEC\n  0\nEOF\n",
        square_lines()
    );
    fs::write(file.path(), dxf).expect("temp dir should be writable");
}

/// The same square plus a HEADER whose `$EXTMIN/$EXTMAX` is 0,0 .. 50,30 --
/// five times the square's area, which is what makes `--crop header`
/// (50 x 30) and `--crop auto` (10 x 10) different pictures: `crop::choose`
/// only adopts header extents that are at most 4x the content's area, so
/// `auto` keeps the content here.
fn write_square_with_header_extents(file: &TempFile) {
    let dxf = format!(
        "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n\
           9\n$EXTMIN\n 10\n0.0\n 20\n0.0\n 30\n0.0\n\
           9\n$EXTMAX\n 10\n50.0\n 20\n30.0\n 30\n0.0\n  0\nENDSEC\n\
         0\nSECTION\n  2\nENTITIES\n{}  0\nENDSEC\n  0\nEOF\n",
        square_lines()
    );
    fs::write(file.path(), dxf).expect("temp dir should be writable");
}

/// Two clusters of 25 horizontal LINEs each: one spanning x 0..10, the other
/// x 100..110, both y 0..9.6. The gap between them is 90 units and the whole
/// content is 110 x 9.6, whose diagonal is sqrt(110^2 + 9.6^2) = 110.4, so a
/// `--frame-gap` above 90 / 110.4 = 0.82 makes them one group and anything
/// below it makes them two. 25 entities per cluster likewise brackets
/// `--min-frame-entities` (the default 20 lets a cluster become a frame; 30
/// does not).
fn write_two_clusters(file: &TempFile) {
    let mut dxf = String::from("  0\nSECTION\n  2\nENTITIES\n");
    for origin_x in [0.0, 100.0] {
        for k in 0..25 {
            let y = f64::from(k) * 0.4;
            dxf.push_str(&dxf_line(origin_x, y, origin_x + 10.0, y));
        }
    }
    dxf.push_str("  0\nENDSEC\n  0\nEOF\n");
    fs::write(file.path(), dxf).expect("temp dir should be writable");
}

/// The IHDR colour-type byte: 2 is RGB, 6 is RGBA (PNG spec, IHDR is the
/// first chunk, colour type follows the 8-byte signature, the 8-byte chunk
/// header, width, height and the bit depth).
fn png_color_type(bytes: &[u8]) -> u8 {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
    bytes[25]
}

#[test]
fn export_writes_the_package_its_readme_describes() {
    let dir = TempDir::new("export-default");
    let out = export(CORPUS_EXAMPLE_DWG, &dir, &[]);
    ok(&out, "the documented export invocation");

    let manifest = dir.manifest();
    assert_eq!(
        manifest["$schema"], "uncad-package/1",
        "the package should declare its schema: {manifest}"
    );
    // Not a list copied from an output: these are the files README.txt's
    // reading order names, and `files` is the manifest's own inventory.
    for name in [
        "manifest.json",
        "README.txt",
        "report.json",
        "overview.png",
        "strings.json",
        "tiles.json",
        "drawing.json",
    ] {
        assert!(dir.join(name).exists(), "the package should hold {name}");
    }
    for file in manifest["files"]
        .as_array()
        .expect("`files` should be an array")
    {
        let path = file["path"].as_str().expect("a file path is a string");
        assert!(
            dir.join(path).exists(),
            "manifest lists {path}, which is not on disk"
        );
    }
    assert!(
        manifest["counts"]["tiles"].as_u64().unwrap_or(0) > 0,
        "a default export writes a tile pyramid: {}",
        manifest["counts"]
    );
}

#[test]
fn export_profile_sizes_the_overview_for_the_models_patch_budget() {
    use uncad::Profile;

    let input = TempFile::new("profile-square.dxf");
    write_square(&input);

    let mut edges = Vec::new();
    for profile in [
        Profile::CLAUDE,
        Profile::CLAUDE_HIRES,
        Profile::OPENAI_PATCH,
    ] {
        let dir = TempDir::new(&format!("export-profile-{}", profile.name));
        let mut flags = vec!["--profile", profile.name];
        flags.extend_from_slice(&QUICK);
        let out = export(input.arg(), &dir, &flags);
        ok(&out, &format!("--profile {}", profile.name));

        let manifest = dir.manifest();
        assert_eq!(manifest["profile"], profile.name);
        let px = &manifest["overview"]["px"];
        let (w, h) = (
            px[0].as_u64().expect("width") as u32,
            px[1].as_u64().expect("height") as u32,
        );
        // The profile's own definition, nothing else: the overview is a
        // whole number of patches on each side, fits the patch budget, and
        // no side exceeds the edge limit.
        assert_eq!(
            (w % profile.lattice, h % profile.lattice),
            (0, 0),
            "{}: {w}x{h} should be whole {} px patches",
            profile.name,
            profile.lattice
        );
        assert!(
            (w / profile.lattice) * (h / profile.lattice) <= profile.overview_patches,
            "{}: {w}x{h} exceeds the {} patch budget",
            profile.name,
            profile.overview_patches
        );
        assert!(
            w.max(h) <= profile.overview_edge,
            "{}: {w}x{h} exceeds the {} px edge",
            profile.name,
            profile.overview_edge
        );
        edges.push(w.max(h));
    }
    // claude-hires is the high-resolution tier of the same model family.
    assert!(
        edges[1] > edges[0],
        "claude-hires should give a larger overview than claude: {edges:?}"
    );
}

#[test]
fn export_max_levels_zero_writes_no_tiles() {
    let input = TempFile::new("levels.dxf");
    write_two_clusters(&input);

    let none = TempDir::new("export-levels-0");
    ok(
        &export(input.arg(), &none, &["--max-levels", "0", "--no-sheets"]),
        "--max-levels 0",
    );
    let one = TempDir::new("export-levels-1");
    ok(
        &export(input.arg(), &one, &["--max-levels", "1", "--no-sheets"]),
        "--max-levels 1",
    );

    // "deepest zoom level" -- 0 means there is no z1 to write.
    assert_eq!(
        none.manifest()["counts"]["tiles"].as_u64(),
        Some(0),
        "--max-levels 0 should write no tile"
    );
    assert!(
        !none.join("frames/f0/tiles").exists(),
        "--max-levels 0 should not create a tiles directory"
    );
    assert!(
        one.manifest()["counts"]["tiles"].as_u64().unwrap_or(0) > 0,
        "--max-levels 1 should write z1"
    );
}

#[test]
fn export_max_tiles_drops_a_level_that_would_exceed_it() {
    let input = TempFile::new("tiles.dxf");
    write_two_clusters(&input);

    let capped = TempDir::new("export-tiles-1");
    ok(
        &export(
            input.arg(),
            &capped,
            &["--max-levels", "1", "--max-tiles", "1", "--no-sheets"],
        ),
        "--max-tiles 1",
    );
    let uncapped = TempDir::new("export-tiles-many");
    ok(
        &export(
            input.arg(),
            &uncapped,
            &["--max-levels", "1", "--max-tiles", "400", "--no-sheets"],
        ),
        "--max-tiles 400",
    );

    // z1 is a 2x2-or-larger grid on any drawing wider than one tile, so a
    // budget of one tile cannot hold it and "deeper levels are dropped
    // whole" leaves nothing.
    assert_eq!(
        capped.manifest()["counts"]["tiles"].as_u64(),
        Some(0),
        "a level that does not fit the budget is dropped whole"
    );
    assert!(
        uncapped.manifest()["counts"]["tiles"].as_u64().unwrap_or(0) > 1,
        "the same level fits the default budget"
    );
}

/// `uncad export --padding`: the flag used to be refused as plain-only, so
/// a package could only ever have the automatic 2 %.
#[test]
fn export_padding_sets_the_window_around_the_drawing() {
    let input = TempFile::new("export-padding.dxf");
    write_square(&input);
    let world = |dir: &TempDir| -> [f64; 4] {
        let w = dir.manifest()["overview"]["world"].clone();
        let a = w.as_array().expect("world is [x0, y0, x1, y1]");
        [
            a[0].as_f64().unwrap(),
            a[1].as_f64().unwrap(),
            a[2].as_f64().unwrap(),
            a[3].as_f64().unwrap(),
        ]
    };
    let padding_units =
        |dir: &TempDir| -> f64 { dir.manifest()["crop"]["padding_units"].as_f64().unwrap() };

    let automatic = TempDir::new("export-padding-auto");
    ok(&export(input.arg(), &automatic, &QUICK), "export");
    // 2 % of the square's 10 units, and at least 24 px at the overview's
    // scale: a fraction of a unit either way.
    let auto = padding_units(&automatic);
    assert!(auto > 0.0 && auto < 1.0, "automatic padding was {auto}");

    let none = TempDir::new("export-padding-0");
    let mut flags = vec!["--padding", "0"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &none, &flags), "export --padding 0");
    assert_eq!(padding_units(&none), 0.0, "--padding 0 is no padding");

    let wide = TempDir::new("export-padding-20");
    let mut flags = vec!["--padding", "20"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &wide, &flags), "export --padding 20");
    assert_eq!(padding_units(&wide), 20.0, "--padding 20 reaches the crop");

    // 20 units a side around a 10-unit square is a 50-unit window, against
    // the square itself with none: the images really do show more.
    let (tight, roomy) = (world(&none), world(&wide));
    let width = |w: [f64; 4]| w[2] - w[0];
    assert!(
        width(roomy) > width(tight) + 30.0,
        "--padding 20 should widen the window: {tight:?} vs {roomy:?}"
    );
}

#[test]
fn export_text_px_is_the_legibility_target() {
    let input = TempFile::new("textpx.dxf");
    write_square(&input);

    let dir = TempDir::new("export-text-px");
    let mut flags = vec!["--text-px", "3"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &dir, &flags), "--text-px 3");
    assert_eq!(
        dir.manifest()["legibility"]["target_px"].as_f64(),
        Some(3.0),
        "the target the manifest reports should be the one asked for"
    );

    let default = TempDir::new("export-text-px-default");
    ok(&export(input.arg(), &default, &QUICK), "the default");
    assert_eq!(
        default.manifest()["legibility"]["target_px"].as_f64(),
        Some(14.0),
        "--help documents 14 px as the default"
    );
}

#[test]
fn export_shard_kb_splits_the_record_files() {
    let input = TempFile::new("shard.dxf");
    write_two_clusters(&input);

    let sharded = TempDir::new("export-shard-1");
    let mut flags = vec!["--shard-kb", "1"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &sharded, &flags), "--shard-kb 1");
    let whole = TempDir::new("export-shard-default");
    ok(
        &export(input.arg(), &whole, &QUICK),
        "the default shard size",
    );

    let geometry_files = |dir: &TempDir| -> Vec<String> {
        dir.manifest()["shard_index"]
            .as_array()
            .expect("`shard_index` should be an array")
            .iter()
            .filter(|s| s["kind"] == "geometry")
            .map(|s| s["file"].as_str().expect("a file name").to_string())
            .collect()
    };
    // 50 LINEs are some tens of kilobytes of records: one file at the
    // default 96 KB, several at 1 KB.
    assert_eq!(
        geometry_files(&whole),
        vec!["geometry.json".to_string()],
        "the default should keep the records in one file"
    );
    let shards = geometry_files(&sharded);
    assert!(
        shards.len() > 1 && shards.iter().all(|f| f.starts_with("geometry.")),
        "--shard-kb 1 should split geometry.json: {shards:?}"
    );
    for shard in &shards {
        assert!(
            sharded.join(shard).exists(),
            "the manifest names {shard}, which should be on disk"
        );
    }
}

#[test]
fn export_frame_gap_and_min_frame_entities_decide_the_second_frame() {
    let input = TempFile::new("frames.dxf");
    write_two_clusters(&input);

    let split = TempDir::new("export-frames-split");
    ok(&export(input.arg(), &split, &QUICK), "the default grouping");
    let merged = TempDir::new("export-frames-merged");
    let mut flags = vec!["--frame-gap", "0.9"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &merged, &flags), "--frame-gap 0.9");
    let too_small = TempDir::new("export-frames-too-small");
    let mut flags = vec!["--min-frame-entities", "30"];
    flags.extend_from_slice(&QUICK);
    ok(
        &export(input.arg(), &too_small, &flags),
        "--min-frame-entities 30",
    );

    let frames = |dir: &TempDir| dir.manifest()["frames"].as_array().map(Vec::len);
    // The clusters are 90 units apart in a 110.4-unit diagonal: the default
    // 0.05 leaves them detached (two frames), 0.9 joins them (0.9 * 110.4 =
    // 99 > 90, one frame).
    assert_eq!(
        frames(&split),
        Some(2),
        "two detached groups should become two frames"
    );
    assert_eq!(
        frames(&merged),
        Some(1),
        "--frame-gap 0.9 spans the 90-unit gap, so the clusters are one group"
    );
    // Each cluster holds 25 LINEs and no text, so a threshold of 30 keeps
    // the detached group out of the frames.
    assert_eq!(
        frames(&too_small),
        Some(1),
        "--min-frame-entities 30 is above the cluster's 25 entities"
    );
}

#[test]
fn export_max_frames_caps_the_frames_and_lists_what_it_dropped() {
    let input = TempFile::new("maxframes.dxf");
    write_two_clusters(&input);

    let dir = TempDir::new("export-max-frames-1");
    let mut flags = vec!["--max-frames", "1"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &dir, &flags), "--max-frames 1");

    let manifest = dir.manifest();
    assert_eq!(
        manifest["frames"].as_array().map(Vec::len),
        Some(1),
        "--max-frames 1 should write one frame"
    );
    // The other cluster is the one documented as "listed as dropped".
    assert_eq!(
        manifest["frames_dropped"].as_array().map(Vec::len),
        Some(1),
        "the group that did not fit should be listed: {manifest}"
    );
}

#[test]
fn export_no_sheets_skips_the_paper_layouts() {
    let with_sheets = TempDir::new("export-sheets");
    ok(
        &export(CORPUS_DWG, &with_sheets, &["--max-levels", "0"]),
        "the default",
    );
    let without = TempDir::new("export-no-sheets");
    ok(
        &export(CORPUS_DWG, &without, &["--max-levels", "0", "--no-sheets"]),
        "--no-sheets",
    );

    // circle.dwg carries the two layouts every AutoCAD drawing starts with.
    assert!(
        with_sheets.manifest()["sheets"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "a default export should describe the paper layouts"
    );
    assert!(
        with_sheets.join("sheets.json").exists(),
        "a default export should write sheets.json"
    );
    assert_eq!(
        without.manifest()["sheets"].as_array().map(Vec::len),
        Some(0),
        "--no-sheets should leave the layouts out of the manifest"
    );
    assert!(
        !without.join("sheets.json").exists() && !without.join("sheets").exists(),
        "--no-sheets should write neither sheets.json nor the images"
    );
}

#[test]
fn export_svg_and_full_write_the_extra_inputs() {
    let input = TempFile::new("extras.dxf");
    write_square(&input);

    let plain = TempDir::new("export-extras-default");
    ok(&export(input.arg(), &plain, &QUICK), "the default");
    let extras = TempDir::new("export-extras-both");
    let mut flags = vec!["--svg", "--full"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &extras, &flags), "--svg --full");

    assert!(
        !plain.join("drawing.svg").exists() && !plain.join("entities.json").exists(),
        "neither extra is written by default"
    );
    let svg = fs::read_to_string(extras.join("drawing.svg")).expect("--svg writes drawing.svg");
    assert!(svg.contains("<svg"), "drawing.svg should be an SVG: {svg}");
    let model: serde_json::Value =
        serde_json::from_slice(&fs::read(extras.join("entities.json")).expect("--full writes it"))
            .expect("entities.json should be JSON");
    assert!(
        model["entities"]
            .as_array()
            .is_some_and(|e| e.len() == square_lines().matches("\nLINE\n").count()),
        "entities.json should hold the whole model: {model}"
    );
}

#[test]
fn export_takes_the_crop_and_font_flags_of_the_render() {
    let input = TempFile::new("export-crop.dxf");
    write_two_clusters(&input);

    let dir = TempDir::new("export-crop-fixed");
    let mut flags = vec!["--crop", "0,0,10,10", "--fonts", "bundled+system"];
    flags.extend_from_slice(&QUICK);
    ok(&export(input.arg(), &dir, &flags), "--crop with --fonts");

    let manifest = dir.manifest();
    assert_eq!(
        manifest["crop"]["source"], "fixed",
        "an explicit rectangle is the crop's source: {manifest}"
    );
    // The rectangle covers the left cluster only, so the right one's 25
    // LINEs are outside (50 LINEs in, 25 kept as geometry records).
    assert_eq!(
        manifest["counts"]["excluded"].as_u64(),
        Some(25),
        "the far cluster should be outside the crop: {}",
        manifest["counts"]
    );
    assert_eq!(manifest["counts"]["geometry"].as_u64(), Some(25));
    assert_eq!(
        manifest["capabilities"]["fonts"], "bundled+system",
        "the font source belongs in the package's capabilities"
    );
}

#[test]
fn an_export_flag_before_the_input_is_not_taken_as_the_input() {
    let dir = TempDir::new("export-flag-first");
    // The flag-first ordering the plain command has always accepted. It
    // used to fail with "cannot open input file '--max-levels'".
    let out = run(&[
        "export",
        "--max-levels",
        "0",
        "--no-sheets",
        CORPUS_DWG,
        "-o",
        dir.arg(),
    ]);
    ok(&out, "an export flag before the input");
    assert!(
        dir.join("manifest.json").exists(),
        "the package should still be written"
    );
}

#[test]
fn export_rejects_a_flag_it_does_not_know() {
    let dir = TempDir::new("export-unknown-flag");
    let out = export(CORPUS_DWG, &dir, &["--max-level", "1"]);
    assert!(
        !out.status.success(),
        "a misspelt flag should not exit 0 with the defaults applied"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--max-level"),
        "the refusal should name the flag: {stderr}"
    );
    assert!(
        !dir.path().exists(),
        "a rejected run should write no package"
    );
}

#[test]
fn each_command_rejects_the_other_ones_flags() {
    // --fit sizes a PNG; the package sizes its images from the profile.
    let dir = TempDir::new("export-plain-flag");
    let out = export(CORPUS_DWG, &dir, &["--fit", "100"]);
    assert!(!out.status.success(), "--fit is not an export option");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--fit") && stderr.contains("-o <output>"),
        "the refusal should point at the plain command: {stderr}"
    );

    // ... and the other way round: a package option on a PNG render.
    let png = TempFile::new("export-only-flag.png");
    let out = run(&[CORPUS_DWG, "-o", png.arg(), "--max-levels", "1"]);
    assert!(!out.status.success(), "--max-levels is an export option");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--max-levels") && stderr.contains("export"),
        "the refusal should name the subcommand: {stderr}"
    );
    assert!(!png.path().exists(), "a rejected run should write nothing");
}

#[test]
fn an_unknown_flag_is_refused_before_and_after_the_input() {
    for args in [
        vec!["--no-such-flag", CORPUS_DWG],
        vec![CORPUS_DWG, "--no-such-flag"],
    ] {
        let out = run(&args);
        assert!(
            !out.status.success(),
            "an unknown flag should fail, not be silently dropped: {args:?}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("--no-such-flag"),
            "the refusal should name the flag: {stderr}"
        );
    }
}

#[test]
fn a_second_input_path_is_refused() {
    let svg = TempFile::new("second-positional.svg");
    let out = run(&[CORPUS_DWG, CORPUS_DXF, "-o", svg.arg()]);
    assert!(
        !out.status.success(),
        "two input paths should fail rather than quietly using the first"
    );
    assert!(!svg.path().exists(), "a rejected run should write nothing");
}

#[test]
fn an_option_without_its_value_is_refused() {
    let out = run(&[CORPUS_DWG, "-o"]);
    assert!(!out.status.success(), "-o with no path should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("-o"),
        "the refusal should name the option: {stderr}"
    );
}

#[test]
fn shard_kb_is_measured_in_kilobytes() {
    let dir = TempDir::new("export-shard-zero");
    let out = export(CORPUS_DWG, &dir, &["--shard-kb", "0"]);
    assert!(!out.status.success(), "--shard-kb 0 should be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // It used to borrow --fit's parser, so it complained about pixels.
    assert!(
        stderr.contains("kilobytes") && !stderr.contains("pixels"),
        "--shard-kb is a size in kilobytes: {stderr}"
    );
}

#[test]
fn fit_sets_the_long_edge_and_lattice_rounds_it_down() {
    let input = TempFile::new("fit.dxf");
    write_square(&input);

    let exact = TempFile::new("fit-200.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            exact.arg(),
            "--fit",
            "200",
            "--lattice",
            "0",
        ]),
        "--fit 200 --lattice 0",
    );
    let (w, h) = png_size(&exact.bytes());
    // "longest side of the image in pixels", with the patch rounding off.
    assert_eq!(
        w.max(h),
        200,
        "--fit 200 should give a 200 px long edge, not {w}x{h}"
    );

    let snapped = TempFile::new("fit-210-lattice-50.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            snapped.arg(),
            "--fit",
            "210",
            "--lattice",
            "50",
        ]),
        "--fit 210 --lattice 50",
    );
    let (w, h) = png_size(&snapped.bytes());
    // 210 rounded down to a multiple of the 50 px patch is 200.
    assert_eq!(
        (w.max(h), w % 50, h % 50),
        (200, 0, 0),
        "--lattice 50 should give whole 50 px patches: {w}x{h}"
    );
}

#[test]
fn ppu_and_padding_give_an_exact_pixel_count() {
    let input = TempFile::new("ppu.dxf");
    write_square(&input);

    let bare = TempFile::new("ppu-2-pad-0.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            bare.arg(),
            "--ppu",
            "2",
            "--padding",
            "0",
            "--lattice",
            "0",
        ]),
        "--ppu 2 --padding 0",
    );
    // The square is 10 x 10 drawing units: 2 px per unit and no padding is
    // 20 x 20 px.
    assert_eq!(png_size(&bare.bytes()), (20, 20));

    let padded = TempFile::new("ppu-2-pad-5.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            padded.arg(),
            "--ppu",
            "2",
            "--padding",
            "5",
            "--lattice",
            "0",
        ]),
        "--ppu 2 --padding 5",
    );
    // 5 units of padding on each side: (10 + 10) units * 2 px = 40 px.
    assert_eq!(png_size(&padded.bytes()), (40, 40));
}

#[test]
fn bg_transparent_writes_an_alpha_channel() {
    let input = TempFile::new("bg.dxf");
    write_square(&input);

    let white = TempFile::new("bg-white.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            white.arg(),
            "--fit",
            "200",
            "--bg",
            "white",
        ]),
        "--bg white",
    );
    let clear = TempFile::new("bg-transparent.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            clear.arg(),
            "--fit",
            "200",
            "--bg",
            "transparent",
        ]),
        "--bg transparent",
    );

    // PNG colour types: 2 = RGB (the documented "written as RGB"), 6 = RGBA.
    assert_eq!(
        png_color_type(&white.bytes()),
        2,
        "--bg white is opaque RGB"
    );
    assert_eq!(
        png_color_type(&clear.bytes()),
        6,
        "--bg transparent needs an alpha channel"
    );

    let out = run(&[input.arg(), "-o", clear.arg(), "--bg", "puce"]);
    assert!(!out.status.success(), "an undocumented --bg should fail");
}

#[test]
fn stroke_changes_the_ink_but_not_the_image_size() {
    let input = TempFile::new("stroke.dxf");
    write_square(&input);

    let thin = TempFile::new("stroke-1.png");
    let thick = TempFile::new("stroke-9.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            thin.arg(),
            "--fit",
            "200",
            "--stroke",
            "1",
        ]),
        "--stroke 1",
    );
    ok(
        &run(&[
            input.arg(),
            "-o",
            thick.arg(),
            "--fit",
            "200",
            "--stroke",
            "9",
        ]),
        "--stroke 9",
    );

    // Same canvas, different lines: the stroke is a pixel width, so it must
    // change the raster without changing its size.
    assert_eq!(
        png_size(&thin.bytes()),
        png_size(&thick.bytes()),
        "--stroke should not resize the image"
    );
    assert_ne!(
        thin.bytes(),
        thick.bytes(),
        "a 9 px stroke should not draw the same pixels as a 1 px one"
    );
}

#[test]
fn max_edge_refuses_an_image_over_the_limit() {
    let input = TempFile::new("maxedge.dxf");
    write_square(&input);

    let over = TempFile::new("max-edge-over.png");
    let out = run(&[
        input.arg(),
        "-o",
        over.arg(),
        "--fit",
        "200",
        "--max-edge",
        "100",
    ]);
    assert!(
        !out.status.success(),
        "a 200 px image should be refused by a 100 px limit"
    );
    assert!(!over.path().exists(), "a refused render writes nothing");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("100"),
        "the refusal should name the limit: {stderr}"
    );

    let under = TempFile::new("max-edge-under.png");
    ok(
        &run(&[
            input.arg(),
            "-o",
            under.arg(),
            "--fit",
            "200",
            "--max-edge",
            "200",
        ]),
        "a limit the image fits",
    );
}

#[test]
fn include_hidden_draws_the_entities_left_out_by_default() {
    // The project's own fixture: nine LINEs, four of them hidden (off,
    // frozen, DEFPOINTS, invisible -- tests/fixtures/README.md's table).
    let visible = TempFile::new("hidden-default.svg");
    ok(&run(&[HIDDEN_LAYERS, "-o", visible.arg()]), "the default");
    let all = TempFile::new("hidden-included.svg");
    ok(
        &run(&[HIDDEN_LAYERS, "-o", all.arg(), "--include-hidden"]),
        "--include-hidden",
    );

    let count = |svg: &[u8]| String::from_utf8_lossy(svg).matches("<line").count();
    assert_eq!(
        count(&visible.bytes()),
        5,
        "five of the nine LINEs are shown"
    );
    assert_eq!(
        count(&all.bytes()),
        9,
        "--include-hidden should draw all nine"
    );
    // Documented as "at 50 %": the four extra lines carry the opacity.
    assert_eq!(
        String::from_utf8_lossy(&all.bytes())
            .matches("opacity")
            .count(),
        4,
        "the hidden four should be drawn faded"
    );
}

#[test]
fn both_documented_font_sources_render() {
    let input = TempFile::new("fonts.dxf");
    write_square(&input);

    for which in ["bundled", "bundled+system"] {
        let png = TempFile::new(&format!("fonts-{}.png", which.replace('+', "-")));
        ok(
            &run(&[
                input.arg(),
                "-o",
                png.arg(),
                "--fonts",
                which,
                "--fit",
                "200",
            ]),
            &format!("--fonts {which}"),
        );
        let (w, h) = png_size(&png.bytes());
        assert!(w > 0 && h > 0, "--fonts {which} should still render");
    }

    let png = TempFile::new("fonts-bad.png");
    let out = run(&[input.arg(), "-o", png.arg(), "--fonts", "comic"]);
    assert!(!out.status.success(), "an unknown --fonts should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bundled"),
        "the refusal should list what is supported: {stderr}"
    );
}

#[test]
fn every_documented_crop_mode_sets_its_own_viewbox() {
    let input = TempFile::new("crop.dxf");
    write_square_with_header_extents(&input);

    let svg_for = |name: &str, mode: &str| -> String {
        let svg = TempFile::new(name);
        ok(
            &run(&[
                input.arg(),
                "-o",
                svg.arg(),
                "--crop",
                mode,
                "--padding",
                "0",
            ]),
            &format!("--crop {mode}"),
        );
        view_box(&svg.bytes())
    };

    // The SVG's y axis points down, so a world rectangle x0..x1, y0..y1
    // becomes "x0 -y1 width height".
    assert_eq!(
        svg_for("crop-auto.svg", "auto"),
        "0 -10 10 10",
        "auto keeps the content: the header extents are 15x its area"
    );
    assert_eq!(
        svg_for("crop-header.svg", "header"),
        "0 -30 50 30",
        "header is $EXTMIN/$EXTMAX as written into the file"
    );
    assert_eq!(
        svg_for("crop-fixed.svg", "20,10,40,25"),
        "20 -25 20 15",
        "x0,y0,x1,y1 is that world rectangle"
    );

    let bad = TempFile::new("crop-bad.svg");
    let out = run(&[input.arg(), "-o", bad.arg(), "--crop", "sideways"]);
    assert!(!out.status.success(), "an unknown --crop should fail");
    assert!(!bad.path().exists(), "a rejected run should write nothing");
}

#[test]
fn crop_raw_is_the_documented_name_for_no_trim() {
    let input = TempFile::new("raw.dxf");
    fs::write(input.path(), dxf_with_an_outlier()).expect("temp dir should be writable");

    let raw = TempFile::new("crop-raw.svg");
    ok(
        &run(&[input.arg(), "-o", raw.arg(), "--crop", "raw"]),
        "--crop raw",
    );
    let no_trim = TempFile::new("crop-no-trim.svg");
    ok(
        &run(&[input.arg(), "-o", no_trim.arg(), "--no-trim"]),
        "--no-trim",
    );

    // "--no-trim  same as --crop raw (0.2.0's name)".
    assert_eq!(view_box(&raw.bytes()), view_box(&no_trim.bytes()));
    let width: f64 = view_box(&raw.bytes())
        .split_whitespace()
        .nth(2)
        .and_then(|w| w.parse().ok())
        .expect("viewBox has four numbers");
    assert!(
        width > 1_000_000.0,
        "--crop raw should keep the outlier a million units out: {width}"
    );
}
