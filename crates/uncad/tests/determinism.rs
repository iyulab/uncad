//! The same input must produce the same output, byte for byte, every time.
//!
//! Nothing else in the suite would notice a violation: a value that is right
//! but arrives in a different order on the next run passes every ordinary
//! assertion. The usual culprit is a hash-based collection whose iteration
//! order leaks into a result -- `std`'s hasher is seeded per instance, so the
//! order differs between two calls in one process, which is what makes the
//! repetition below able to catch it.
//!
//! The last test is not about determinism; it lives here because it reads the
//! same fixture. The fixture is written by the test itself, so this file does not depend on
//! the LibreDWG corpus checkout the other integration tests need.

use std::fs;
use std::path::PathBuf;

/// How often each output is regenerated. With four or more entries in a
/// leaked hash set, two consecutive identical orders are already unlikely;
/// this many leave no realistic chance of a false pass.
const RUNS: usize = 24;

/// Entity types the renderer reports as left out, in the order they must be
/// reported: sorted by name, whatever order the drawing lists them in.
const EXPECTED_UNSUPPORTED: &[&str] = &["ATTDEF", "DIMENSION"];

/// A minimal ASCII DXF: three drawable entities (LINE, TRACE, CIRCLE) and two
/// the renderer leaves out (a DIMENSION that carries no cached block, and an
/// ATTDEF). DIMENSION deliberately comes first so that file order and sorted
/// order disagree.
fn minimal_dxf() -> String {
    let pairs: &[(u16, &str)] = &[
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "LINE"),
        (8, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (11, "100.0"),
        (21, "0.0"),
        (31, "0.0"),
        (0, "TRACE"),
        (8, "0"),
        (10, "0.0"),
        (20, "10.0"),
        (30, "0.0"),
        (11, "10.0"),
        (21, "10.0"),
        (31, "0.0"),
        (12, "0.0"),
        (22, "12.0"),
        (32, "0.0"),
        (13, "10.0"),
        (23, "12.0"),
        (33, "0.0"),
        (0, "CIRCLE"),
        (8, "0"),
        (10, "50.0"),
        (20, "50.0"),
        (30, "0.0"),
        (40, "25.0"),
        (0, "DIMENSION"),
        (8, "0"),
        (10, "100.0"),
        (20, "-20.0"),
        (30, "0.0"),
        (11, "50.0"),
        (21, "-20.0"),
        (31, "0.0"),
        (70, "0"),
        (13, "0.0"),
        (23, "0.0"),
        (33, "0.0"),
        (14, "100.0"),
        (24, "0.0"),
        (34, "0.0"),
        (0, "ATTDEF"),
        (8, "0"),
        (10, "5.0"),
        (20, "5.0"),
        (30, "0.0"),
        (40, "2.5"),
        (1, "default"),
        (3, "prompt"),
        (2, "TAG"),
        (70, "0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    pairs
        .iter()
        .map(|(code, value)| format!("{code:>3}\n{value}\n"))
        .collect()
}

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct Fixture(PathBuf);

impl Fixture {
    fn write(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name));
        fs::write(&path, minimal_dxf()).expect("the temp dir should be writable");
        Fixture(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn unsupported_types_are_reported_in_sorted_order_on_every_run() {
    let fixture = Fixture::write("determinism-unsupported.dxf");
    let db = uncad::parse(&fixture.0).expect("the minimal DXF should parse");

    for run in 0..RUNS {
        let result = iron_render_cad::to_svg(&db, iron_render_cad::ToSvgOptions::default());
        assert_eq!(
            result.unsupported_types, EXPECTED_UNSUPPORTED,
            "run {run}: unsupported_types must not depend on hash iteration order"
        );
    }
}

#[test]
fn repeated_parse_and_export_are_byte_identical() {
    let fixture = Fixture::write("determinism-export.dxf");

    let export = || {
        let db = uncad::parse(&fixture.0).expect("the minimal DXF should parse");
        let json = db
            .to_json(uncad::ToJsonOptions::default())
            .expect("the model should serialize");
        let svg = iron_render_cad::to_svg(&db, iron_render_cad::ToSvgOptions::default());
        (json, svg.svg, svg.unsupported_types)
    };

    let first = export();
    for run in 1..RUNS {
        let again = export();
        assert_eq!(first.0, again.0, "run {run}: JSON output changed");
        assert_eq!(first.1, again.1, "run {run}: SVG output changed");
        assert_eq!(first.2, again.2, "run {run}: unsupported_types changed");
    }
}

#[test]
fn a_trace_is_read_and_drawn_like_a_solid() {
    let fixture = Fixture::write("determinism-trace.dxf");
    let db = uncad::parse(&fixture.0).expect("the minimal DXF should parse");

    let trace = db
        .entities
        .iter()
        .find_map(|e| match e {
            uncad::Entity::Trace(t) => Some(t),
            _ => None,
        })
        .expect("the TRACE should arrive as Entity::Trace, not Unknown");
    // The corners as the file lists them (group codes 10/20 .. 13/23).
    let corners = [trace.corner1, trace.corner2, trace.corner3, trace.corner4];
    let expected = [(0.0, 10.0), (10.0, 10.0), (0.0, 12.0), (10.0, 12.0)];
    for (got, want) in corners.iter().zip(expected) {
        assert_eq!((got.x, got.y), want);
    }

    let result = iron_render_cad::to_svg(&db, iron_render_cad::ToSvgOptions::default());
    assert!(
        !result.unsupported_types.iter().any(|t| t == "TRACE"),
        "a TRACE must not be reported as left out: {:?}",
        result.unsupported_types
    );
    // Filled in 1-2-4-3 order, the same bow-tie-avoiding order SOLID uses
    // (SVG y is flipped).
    assert!(
        result.svg.contains("points=\"0,-10 10,-10 10,-12 0,-12\""),
        "the TRACE should be drawn as a quadrilateral: {}",
        result.svg
    );
}
