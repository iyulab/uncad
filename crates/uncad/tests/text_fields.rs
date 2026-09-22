//! The text fields 0.3.0 adds (docs/VLM_EXPORT_DESIGN.md, P2): justification
//! and the alignment point on TEXT, the attachment point, wrap width and
//! baseline direction on MTEXT, the decoded `text_plain`, and the ATTRIB
//! `tag`. The single-line and multi-line cases come from a DXF this test
//! writes from group codes (so the expected values are the ones it wrote);
//! the ATTRIB case from the corpus.

use std::f64::consts::FRAC_PI_2;
use std::path::PathBuf;

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
/// whose derived start point is (0,0); a plain left/baseline TEXT with a
/// `%%c` code; and a middle-center MTEXT rotated 90 degrees (direction
/// vector (0,1,0)) with a wrap width and a paragraph break. Padded past
/// LibreDWG's 256-byte minimum with LINEs.
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

fn parse_text_dxf() -> uncad::CadDatabase {
    let dir = TempDir::new("text-fields");
    let path = dir.0.join("text.dxf");
    std::fs::write(&path, text_dxf()).expect("writable");
    uncad::parse(&path).expect("the DXF must parse")
}

#[test]
fn text_justification_and_alignment_point_are_read() {
    let db = parse_text_dxf();
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
        (centered.horizontal_alignment, centered.vertical_alignment),
        (1, 2)
    );
    let ap = centered.alignment_point.expect("DXF 11 is the anchor");
    assert_eq!((ap.x, ap.y), (50.0, 50.0));
    assert_eq!(centered.width_factor, 0.8);
    assert!((centered.oblique_angle - 15f64.to_radians()).abs() < 1e-9);
    assert_eq!(centered.text_plain, "CENTER");

    let plain = texts.iter().find(|t| t.text == "%%c50").expect("%%c50");
    assert_eq!(
        (plain.horizontal_alignment, plain.vertical_alignment),
        (0, 0)
    );
    assert_eq!(plain.alignment_point, None);
    assert_eq!(plain.width_factor, 1.0, "unset width factor reads as 1");
    assert_eq!(plain.text_plain, "\u{2205}50");
}

#[test]
fn mtext_reads_attachment_width_and_rotation_from_its_direction_vector() {
    let db = parse_text_dxf();
    let mtext = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::MText(m) => Some(m),
            _ => None,
        })
        .expect("MTEXT");
    assert_eq!(mtext.text, "A\\PB");
    assert_eq!(mtext.text_plain, "A\nB");
    assert_eq!(mtext.attachment, 5);
    assert_eq!(mtext.rect_width, 80.0);
    assert_eq!(
        (mtext.x_axis_dir.x, mtext.x_axis_dir.y, mtext.x_axis_dir.z),
        (0.0, 1.0, 0.0)
    );
    assert!(
        (mtext.rotation - FRAC_PI_2).abs() < 1e-12,
        "0.2.0 always reported 0; now atan2(1, 0): {}",
        mtext.rotation
    );
}

#[test]
fn the_renderer_anchors_justified_text_at_the_alignment_point() {
    let db = parse_text_dxf();
    let svg = db.to_svg(Default::default()).svg;
    // Center/middle text: anchored at (50,50) with text-anchor middle, drawn
    // from its decoded string; the old renderer put it at (0,0).
    assert!(
        svg.contains("text-anchor=\"middle\""),
        "no middle anchor in {svg}"
    );
    let centered = svg
        .split("<text ")
        .find(|t| t.contains(">CENTER<"))
        .expect("CENTER is drawn");
    assert!(centered.contains("x=\"50\""), "{centered}");
    assert!(!centered.contains("x=\"0\""), "{centered}");
    // The %%c became the diameter sign in the image too.
    assert!(svg.contains(">\u{2205}50<"), "{svg}");
    // The MTEXT is rotated (SVG rotates clockwise, so -90) and split into
    // two lines.
    assert!(svg.contains("rotate(-90 100 -100)"), "{svg}");
    assert!(
        svg.contains(">A</tspan>") && svg.contains(">B</tspan>"),
        "{svg}"
    );
}

#[test]
fn attribs_carry_their_tag() {
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
        assert!(
            !attrib.tag.is_empty(),
            "ATTRIB {} has no tag",
            attrib.common.handle
        );
        assert_eq!(
            attrib.text_plain,
            uncad::text::decode_text(&attrib.text).plain
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
    assert!(inserts_attribs.iter().all(|a| !a.tag.is_empty()));
}

#[test]
fn nested_texts_carry_their_block_reference_in_the_id() {
    // The dimlfac fixture's dimension draws its *D1 block, whose TEXT "120"
    // has handle 42: the SVG id is "<dimension>/42".
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dimlfac12_r2000.dxf"
    );
    let db = uncad::parse(fixture).expect("fixture must parse");
    let svg = db.to_svg(Default::default()).svg;
    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) => Some(d.common.handle.clone()),
            _ => None,
        })
        .expect("a dimension");
    assert!(svg.contains(&format!("id=\"{dim}/42\"")), "{svg}");
}
