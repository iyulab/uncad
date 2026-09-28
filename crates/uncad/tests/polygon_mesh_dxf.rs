//! A polygon mesh in a DXF: a POLYLINE (flag 16) whose VERTEX records carry
//! the subclass marker AutoCAD writes on a polygon mesh's vertices,
//! `AcDbPolygonMeshVertex`. The file reads, keeps everything it held, and
//! gains the mesh as one entity.

use std::fs;
use std::path::{Path, PathBuf};

/// The same corpus R2000 DXF the other DXF tests read.
const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
);

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl TempFile {
    fn with_contents(name: &str, contents: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name));
        fs::write(&path, contents).expect("the temp dir should be writable");
        TempFile(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

/// `CORPUS_DXF` with one polygon mesh added to its ENTITIES section: a 2 x 2
/// POLYLINE (flag 16) whose VERTEX records carry the subclass marker AutoCAD
/// writes on a polygon mesh's vertices, `AcDbPolygonMeshVertex`. The handles
/// are above everything the file already uses. The file's own line ending is
/// kept, whatever the checkout turned it into.
fn corpus_dxf_with_a_polygon_mesh() -> String {
    let text = fs::read_to_string(CORPUS_DXF).expect("the corpus DXF should be readable text");
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let pair = |code: u16, value: &str| format!("{code:>3}{nl}{value}{nl}");

    let mut mesh = String::new();
    for (code, value) in [
        (0, "POLYLINE"),
        (5, "F0"),
        (330, "1F"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbPolygonMesh"),
        (66, "1"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (70, "16"),
        (71, "2"),
        (72, "2"),
    ] {
        mesh.push_str(&pair(code, value));
    }
    for (handle, (x, y)) in [
        ("F1", (0, 0)),
        ("F2", (0, 5)),
        ("F3", (10, 0)),
        ("F4", (10, 5)),
    ] {
        for (code, value) in [
            (0, "VERTEX".to_string()),
            (5, handle.to_string()),
            (330, "F0".to_string()),
            (100, "AcDbEntity".to_string()),
            (8, "0".to_string()),
            (100, "AcDbVertex".to_string()),
            (100, "AcDbPolygonMeshVertex".to_string()),
            (10, format!("{x}.0")),
            (20, format!("{y}.0")),
            (30, "0.0".to_string()),
            (70, "64".to_string()),
        ] {
            mesh.push_str(&pair(code, &value));
        }
    }
    for (code, value) in [
        (0, "SEQEND"),
        (5, "F5"),
        (330, "F0"),
        (100, "AcDbEntity"),
        (8, "0"),
    ] {
        mesh.push_str(&pair(code, value));
    }

    let entities = text
        .find(&format!("{nl}ENTITIES{nl}"))
        .expect("the corpus DXF has an ENTITIES section");
    let end = entities
        + text[entities..]
            .find(&format!("  0{nl}ENDSEC{nl}"))
            .expect("the ENTITIES section ends");
    format!("{}{mesh}{}", &text[..end], &text[end..])
}

/// How many entities of each type name a drawing holds.
fn type_counts(db: &uncad::CadDatabase) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for entity in &db.entities {
        *counts.entry(entity.type_name().to_string()).or_insert(0) += 1;
    }
    counts
}

#[test]
fn a_polygon_mesh_does_not_cost_the_whole_dxf() {
    let reference = uncad::parse(CORPUS_DXF).expect("the untouched corpus DXF should parse");
    let with_mesh = TempFile::with_contents(
        "polygon-mesh.dxf",
        corpus_dxf_with_a_polygon_mesh().as_bytes(),
    );

    let db = uncad::parse(with_mesh.path())
        .unwrap_or_else(|e| panic!("a DXF holding a polygon mesh should parse: {e}"));

    // Everything the file held before is still there, and the mesh is one
    // entity more -- whatever type this crate gives it, and with its
    // vertices not counted as entities of their own.
    let before = type_counts(&reference);
    let after = type_counts(&db);
    for (name, count) in &before {
        assert_eq!(
            after.get(name),
            Some(count),
            "{name}: the entities of the untouched file should all survive the mesh"
        );
    }
    assert_eq!(
        db.entities.len(),
        reference.entities.len() + 1,
        "the polygon mesh should add exactly one entity: {after:?}"
    );
}
