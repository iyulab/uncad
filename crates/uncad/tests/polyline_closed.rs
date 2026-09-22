//! LWPOLYLINE's `closed` flag against the one file in the corpus that has a
//! text twin: `example_2000.dxf` states each polyline's closed bit in group
//! code 70, and `example_2000.dwg` is the same drawing. The DXF is the
//! reference, so the numbers here (11 polylines, 10 closed) come from AutoCAD's
//! own writer, not from this project's output.

use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);

/// `(handle, closed)` for every LWPOLYLINE in the drawing, top level and
/// block definitions alike, so the DWG and the DXF can be compared entity by
/// entity.
fn lwpolylines(db: &uncad::CadDatabase) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = db
        .tables
        .block_records
        .values()
        .flat_map(|record| record.entities.iter())
        .filter_map(|e| match e {
            Entity::LwPolyline(p) => Some((p.common.handle.clone(), p.closed)),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Reads group code 70 of every LWPOLYLINE straight out of the DXF text, the
/// way the DXF reference defines it (bit 1 = closed): the ground truth.
fn closed_bits_from_dxf_text() -> Vec<(String, bool)> {
    // Bytes, not read_to_string: the file's text is CP1252 (it carries a
    // degree sign), and only the ASCII group codes matter here.
    let bytes = std::fs::read(EXAMPLE_2000_DXF).expect("corpus DXF is readable");
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i] == "0" && lines[i + 1] == "LWPOLYLINE" {
            let (mut handle, mut flags) = (None, None);
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                match lines[j] {
                    "5" => handle = Some(lines[j + 1].to_string()),
                    "70" => flags = lines[j + 1].parse::<u32>().ok(),
                    _ => {}
                }
                j += 2;
            }
            if let (Some(h), Some(f)) = (handle, flags) {
                out.push((h, f & 1 != 0));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out.sort();
    out
}

#[test]
fn closed_bits_read_from_the_dwg_match_the_dxf_group_70() {
    let truth = closed_bits_from_dxf_text();
    assert_eq!(
        truth.len(),
        11,
        "example_2000.dxf holds 11 LWPOLYLINEs: {truth:?}"
    );
    assert_eq!(truth.iter().filter(|(_, closed)| *closed).count(), 10);

    let dwg = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    assert_eq!(
        lwpolylines(&dwg),
        truth,
        "DWG closed flags vs the DXF's group 70"
    );
}

#[test]
fn the_dxf_input_path_agrees_with_the_dwg() {
    let dwg = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let dxf = uncad::parse(EXAMPLE_2000_DXF).expect("corpus file must parse");
    assert_eq!(lwpolylines(&dxf), lwpolylines(&dwg));
}

#[test]
fn a_closed_rectangle_renders_as_a_polygon() {
    // The SVG side of the same fact: a closed LWPOLYLINE becomes <polygon>,
    // an open one <polyline>. example_2000.dwg's ten closed polylines used to
    // all come out as <polyline> (see docs/CAVEATS.md).
    let dwg = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let svg = dwg
        .to_svg(uncad::ToSvgOptions {
            space: uncad::Space::All,
            ..Default::default()
        })
        .svg;
    assert!(
        svg.contains("<polygon points="),
        "no closed polyline was drawn as a polygon"
    );
}
