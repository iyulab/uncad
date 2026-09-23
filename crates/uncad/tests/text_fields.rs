//! How TEXT, ATTRIB and MTEXT are placed beyond their insertion point:
//! justification and the alignment point, width factor, oblique angle and
//! text style on TEXT; the attachment point, wrap width, extents and
//! rotation on MTEXT; the tag on ATTRIB. The single-line and multi-line
//! cases come from a DXF this test writes from group codes (so the expected
//! values are the ones it wrote); the ATTRIB case from the corpus.

use std::f64::consts::FRAC_PI_2;
use std::path::PathBuf;

use uncad::model::{HorizontalJustification, MTextAttachment, Point2D, Ref, VerticalJustification};
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("uncad-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("temp dir should be writable");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An AC1015 DXF with: a center/middle-justified TEXT anchored at (50,50)
/// with a width factor of 0.8 and a 15-degree slant; a plain left/baseline
/// TEXT; and a middle-center MTEXT rotated 90 degrees (direction vector
/// (0,1,0)) with a wrap width and a paragraph break. Padded past LibreDWG's
/// 256-byte minimum with LINEs.
fn text_dxf() -> String {
    let mut dxf = String::from(
        "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n",
    );
    dxf.push_str(
        "  0\nTEXT\n  8\n0\n 10\n0.0\n 20\n0.0\n 40\n5.0\n  1\nCENTER\n 41\n0.8\n 51\n15.0\n 72\n1\n 11\n50.0\n 21\n50.0\n 73\n2\n",
    );
    dxf.push_str("  0\nTEXT\n  8\n0\n 10\n0.0\n 20\n20.0\n 40\n5.0\n  1\n%%c50\n");
    dxf.push_str(
        "  0\nMTEXT\n  8\n0\n 10\n100.0\n 20\n100.0\n 30\n0.0\n 40\n2.5\n 41\n80.0\n 71\n5\n  1\nA\\PB\n 11\n0.0\n 21\n1.0\n 31\n0.0\n",
    );
    for i in 0..8 {
        dxf.push_str(&format!(
            "  0\nLINE\n  8\n0\n 10\n{i}.0\n 20\n0.0\n 11\n{i}.0\n 21\n1.0\n"
        ));
    }
    dxf.push_str("  0\nENDSEC\n  0\nEOF\n");
    dxf
}

/// Writes the DXF into a directory of its own (`name` keeps the tests
/// apart: they run on parallel threads of one process) and parses it.
fn parse_text_dxf(name: &str) -> uncad::CadDatabase {
    let dir = TempDir::new(&format!("text-fields-{name}"));
    let path = dir.0.join("text.dxf");
    std::fs::write(&path, text_dxf()).expect("writable");
    uncad::parse(&path).expect("the DXF must parse")
}

#[test]
fn text_justification_and_alignment_point_are_read() {
    let db = parse_text_dxf("justification");
    let texts: Vec<&uncad::model::TextEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2, "{texts:?}");

    let centered = texts.iter().find(|t| t.text == "CENTER").expect("CENTER");
    assert_eq!(
        (
            centered.horizontal_justification,
            centered.vertical_justification
        ),
        (
            HorizontalJustification::Center,
            VerticalJustification::Middle
        )
    );
    assert_eq!(
        centered.alignment_point,
        Some(Point2D { x: 50.0, y: 50.0 }),
        "DXF 11 is the anchor"
    );
    assert_eq!(centered.width_factor, 0.8);
    assert!((centered.oblique_angle - 15f64.to_radians()).abs() < 1e-9);

    // The text keeps its `%%c` code verbatim; decoding it is a consumer's.
    let plain = texts.iter().find(|t| t.text == "%%c50").expect("%%c50");
    assert_eq!(
        (plain.horizontal_justification, plain.vertical_justification),
        (
            HorizontalJustification::Left,
            VerticalJustification::Baseline
        )
    );
    assert_eq!(
        plain.alignment_point, None,
        "left/baseline states no DXF 11"
    );
    assert_eq!(plain.width_factor, 1.0, "an absent width factor reads as 1");
    assert_eq!(plain.oblique_angle, 0.0);
    // The file has no STYLE table, so there is no STANDARD entry to stand
    // in for the absent group 7.
    assert_eq!(plain.style_name, Ref::Absent);
}

#[test]
fn mtext_reads_attachment_width_and_rotation_from_its_direction_vector() {
    let db = parse_text_dxf("mtext");
    let mtext = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::MText(m) => Some(m),
            _ => None,
        })
        .expect("MTEXT");
    assert_eq!(mtext.text, "A\\PB");
    assert_eq!(mtext.attachment, Some(MTextAttachment::MiddleCenter));
    assert_eq!(mtext.reference_width, 80.0);
    assert_eq!(
        (mtext.extents_width, mtext.extents_height),
        (None, None),
        "the file states no extents"
    );
    assert!(
        (mtext.rotation - FRAC_PI_2).abs() < 1e-12,
        "atan2(1, 0): {}",
        mtext.rotation
    );
}

#[test]
fn autocad_written_attributes_carry_their_tag_and_style() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let attribs: Vec<&uncad::model::AttribEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Attrib(a) => Some(a),
            _ => None,
        })
        .collect();
    assert!(!attribs.is_empty(), "example_2000.dwg has ATTRIBs");
    for attrib in &attribs {
        assert!(!attrib.tag.is_empty(), "{attrib:?}");
        assert!(
            matches!(attrib.style_name, Ref::Resolved(_)),
            "an AutoCAD-written attribute names its style: {attrib:?}"
        );
    }
    // And the INSERT's own copy agrees.
    let inserts_attribs: Vec<&uncad::model::AttribEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Insert(i) => Some(i.attribs.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(inserts_attribs, attribs);
}
