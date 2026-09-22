//! A control character in a label must never make the SVG unparseable.
//! XML 1.0 forbids U+0000..U+0008, U+000B, U+000C, U+000E..U+001F, U+FFFE
//! and U+FFFF outright, and the renderer's SVG is parsed by roxmltree
//! (through usvg) for every PNG, every tile and the package's text metrics;
//! one stray byte in one label used to fail `to_png` with `InvalidSvg` and
//! leave the export directory empty. The DXF is written by the test itself
//! from group codes, so the bytes in it are exactly the ones asserted on.

use std::path::{Path, PathBuf};

use uncad::export::{export_package, ExportOptions};
use uncad::{Entity, Format, ToPngOptions, ToSvgOptions};

/// A fresh directory under the target dir, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("control_chars_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An AC1015 DXF with one TEXT per entry of `texts` (raw bytes) at 5-unit
/// steps, plus a LINE and enough padding LINEs to pass LibreDWG's 256-byte
/// minimum.
fn dxf_with_texts(texts: &[&[u8]]) -> Vec<u8> {
    let mut dxf = Vec::new();
    dxf.extend_from_slice(
        b"  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n",
    );
    for (i, text) in texts.iter().enumerate() {
        dxf.extend_from_slice(b"  0\nTEXT\n  5\n");
        dxf.extend_from_slice(format!("{:X}\n", 0x80 + i).as_bytes());
        dxf.extend_from_slice(b"  8\n0\n 10\n0.0\n 20\n");
        dxf.extend_from_slice(format!("{}.0\n", i * 5).as_bytes());
        dxf.extend_from_slice(b" 40\n2.5\n  1\n");
        dxf.extend_from_slice(text);
        dxf.extend_from_slice(b"\n");
    }
    for i in 0..8 {
        dxf.extend_from_slice(
            format!("  0\nLINE\n  8\n0\n 10\n{i}.0\n 20\n0.0\n 11\n{i}.0\n 21\n20.0\n").as_bytes(),
        );
    }
    dxf.extend_from_slice(b"  0\nENDSEC\n  0\nEOF\n");
    dxf
}

fn is_xml_illegal(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}'
    )
}

fn plain_texts(db: &uncad::CadDatabase) -> Vec<(String, String)> {
    db.entities
        .iter()
        .filter_map(|e| match e {
            Entity::Text(t) => Some((t.text.clone(), t.text_plain.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_control_character_in_a_text_reaches_neither_the_svg_nor_the_png_nor_the_package() {
    // TEXT 80 holds the raw bytes 01 and 0B (SOH, VT) between "ZE" and
    // "RO"; TEXT 81 spells the same with the %%nnn code AutoCAD's decoder
    // would have turned into U+0001.
    let db = uncad::parse_bytes(
        &dxf_with_texts(&[b"ZE\x01\x0BRO", b"ZE%%001RO"]),
        Format::Dxf,
    )
    .expect("the DXF must parse");
    let texts = plain_texts(&db);
    // The raw string is kept as the file had it; the decoded one marks each
    // forbidden character with U+FFFD, and refuses the %%001 code.
    assert_eq!(
        texts,
        vec![
            (
                "ZE\u{1}\u{B}RO".to_string(),
                "ZE\u{FFFD}\u{FFFD}RO".to_string()
            ),
            ("ZE%%001RO".to_string(), "ZE%%001RO".to_string()),
        ]
    );

    let svg = db.to_svg(ToSvgOptions::default()).svg;
    assert!(
        !svg.chars().any(is_xml_illegal),
        "the SVG must hold no character XML forbids"
    );
    assert!(svg.contains("ZE\u{FFFD}\u{FFFD}RO"), "{svg}");

    // The PNG: roxmltree used to refuse the document ("a non-XML character
    // '\u{1}' found").
    let png = db
        .to_png(ToPngOptions::default())
        .expect("to_png must not fail on a control character");
    assert!(png.png.starts_with(b"\x89PNG"));

    // The package: nothing at all was written before.
    let tmp = TempDir::new("package");
    let report = export_package(
        &db,
        &tmp.0,
        &ExportOptions {
            max_levels: 0,
            ..Default::default()
        },
    )
    .expect("export_package must not fail on a control character");
    assert!(tmp.0.join("overview.png").exists());
    assert!(tmp.0.join("manifest.json").exists());
    let texts_json = std::fs::read_to_string(tmp.0.join("texts.json")).expect("texts.json");
    let records: serde_json::Value = serde_json::from_str(&texts_json).expect("valid JSON");
    let recorded: Vec<&str> = records["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|r| r["text"].as_str().expect("text"))
        .collect();
    assert_eq!(recorded.len(), 2, "{recorded:?}");
    assert!(recorded.contains(&"ZE\u{FFFD}\u{FFFD}RO"), "{recorded:?}");
    assert!(recorded.contains(&"ZE%%001RO"), "{recorded:?}");
    assert!(
        !texts_json.chars().any(is_xml_illegal),
        "no forbidden character in the records"
    );
    let _ = report;
}

#[test]
fn the_svg_writer_strips_a_control_character_that_bypassed_the_decoder() {
    // A CadDatabase built by hand (or loaded from JSON) can carry any string
    // in text_plain; escape_xml is the last line of defence.
    use uncad::model::{EntityCommon, LineEntity, Point2D, Point3D, TextEntity};
    let entities = vec![
        Entity::Line(LineEntity {
            common: EntityCommon {
                handle: "L".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        }),
        Entity::Text(TextEntity {
            common: EntityCommon {
                handle: "T".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point2D { x: 10.0, y: 10.0 },
            text_height: 5.0,
            text: "A\u{1}B".into(),
            text_plain: "A\u{1}B\u{FFFE}".into(),
            rotation: 0.0,
            horizontal_alignment: 0,
            vertical_alignment: 0,
            alignment_point: None,
            width_factor: 1.0,
            oblique_angle: 0.0,
            style: String::new(),
        }),
    ];
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    let db = uncad::CadDatabase::new(entities, tables);
    let svg = db.to_svg(ToSvgOptions::default()).svg;
    assert!(svg.contains(">AB</text>"), "{svg}");
    assert!(!svg.chars().any(is_xml_illegal));
    db.to_png(ToPngOptions::default())
        .expect("to_png must not fail on a control character in text_plain");
}
