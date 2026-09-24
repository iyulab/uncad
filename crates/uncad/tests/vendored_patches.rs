//! What the local patches to the vendored LibreDWG change, pinned from the
//! outside.
//!
//! `crates/libredwg-sys/vendor/libredwg/` carries `uncad local patch`
//! blocks (listed in `crates/libredwg-sys/NOTICE.md`, reasoned in
//! `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG"). A re-vendor
//! that drops one is caught by `build.rs`, which counts the markers; these
//! tests catch a patch that is still marked but no longer does its job. Each
//! input below was checked against a build without the patches: the corrupt
//! header date ends that build's process with 0xC0000409, and the polygon
//! mesh makes it refuse the whole file with critical error 2048.
//!
//! The `dwg.c` one, which makes the DXF importer decode an R2007+ file's
//! table-record names before it compares them, is pinned in
//! `tests/r2007_dxf_handles.rs`: an R2018 DXF must resolve the same layers
//! and block records as its DWG twin, which without the patch it does not
//! (every name longer than one character missed its lookup). The
//! `common_entity_data.spec` one, which puts an entity's true colour and its
//! transparency back in their own fields, is pinned below on the entity it
//! was measured on (HATCH 29F in `test-data/2004/HatchG.dwg`). The HATCH
//! spline edge, DXF transparency and R13/R14 linetype ones are pinned in
//! `tests/hatch_edges.rs`, `tests/golden.rs` (G17), `tests/entity_style.rs`
//! and the corpus twin comparison; the `dwg.spec` one, an R2010+ ATTRIB's
//! text style, below.

use std::fs;
use std::path::{Path, PathBuf};
use uncad::model::Ref;

const HELIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/Helix.dwg"
);

/// The R2004 drawing the colour-order patch was measured on.
const HATCH_G: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2004/HatchG.dwg"
);

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

// --- src/common.c: cvt_TIMEBLL ---------------------------------------------

/// The single byte that turned this R2000 drawing into a process killer,
/// minimised from a fuzzed corpus file by bisecting over 89 mutated offsets.
/// It sits in the header-variables bit stream just before `TDUUPDATE`, so
/// flipping it gives that date a wild value, which `cvt_TIMEBLL` used to turn
/// into a `struct tm` that Microsoft's `strftime` refuses by ending the
/// process.
const HEADER_DATE_OFFSET: usize = 27644;
const HEADER_DATE_ORIGINAL: u8 = 0x25;
const HEADER_DATE_CORRUPT: u8 = 0x80;

#[test]
fn a_corrupt_header_date_does_not_end_the_process() {
    let mut bytes = fs::read(HELIX).expect("the corpus DWG is readable");
    assert_eq!(
        bytes.get(HEADER_DATE_OFFSET).copied(),
        Some(HEADER_DATE_ORIGINAL),
        "the corpus file changed: byte {HEADER_DATE_OFFSET} is no longer the one this \
         regression was minimised against"
    );

    // The file as it stands must still read, so what follows is about the
    // corruption and not about Helix.dwg.
    let clean = uncad::parse(HELIX).expect("the unmodified drawing parses");
    assert!(!clean.entities.is_empty());

    bytes[HEADER_DATE_OFFSET] = HEADER_DATE_CORRUPT;
    let corrupt = TempFile::with_contents("corrupt-header-date.dwg", &bytes);
    // Reaching the next line at all is the assertion: without the patch this
    // call ends the test binary with 0xC0000409 on Windows, printing nothing.
    // Either outcome is acceptable -- LibreDWG may still decode the rest of
    // the file or give up on it.
    match uncad::parse(corrupt.path()) {
        Ok(_) | Err(_) => {}
    }
}

// --- src/in_dxf.c and src/dynapi.c: AcDbPolygonMeshVertex ------------------

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

    // Without the patch this is Err(Critical(2048)): the VERTEX stays a
    // VERTEX_2D, fails the subclass check and the importer gives up on the
    // file.
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

// --- src/common_entity_data.spec: an entity's RGB before its transparency --

/// HATCH `29F` of `2004/HatchG.dwg` carries both an inline RGB and a
/// transparency (its colour's ENC flag is `0xa0`); it lies inside LWPOLYLINE
/// `28D`, whose flag is `0x80` alone -- one BL, which no reading order can
/// get wrong -- and the file draws the two in the same colour. Without the
/// patch the spec read the HATCH's two BLs the other way round, so its
/// colour was the transparency word `0x020000e5` (a true colour of
/// `0x0000e5`) and the RGB went to the transparency field.
#[test]
fn an_entity_with_a_transparency_keeps_its_true_colour() {
    let db = uncad::parse(HATCH_G).expect("the corpus DWG parses");
    let colour = |handle: &str| {
        db.entities
            .iter()
            .find(|e| e.common().source_handle == uncad::model::Ref::Resolved(handle.to_string()))
            .unwrap_or_else(|| panic!("HatchG.dwg has an entity {handle}"))
            .common()
            .true_color
    };
    assert_eq!(colour("28D"), Some(0x1a_e464), "the LWPOLYLINE, one BL");
    assert_eq!(
        colour("29F"),
        Some(0x1a_e464),
        "the HATCH, RGB and transparency"
    );
}

/// An R2010+ ATTRIB's text style: the vendored spec no longer reads, for an
/// ATTRIB, the version byte that only an ATTDEF stores, which ran past the
/// record and stopped the decode before the style handle. The DXF twins
/// name the styles these come back with.
#[test]
fn an_r2010_attrib_keeps_its_text_style() {
    for (file, style) in [
        ("example_2010.dwg", "Standard"),
        ("example_2018.dwg", "Standard"),
        ("2010/gh209_1.dwg", "Hebtxt"),
    ] {
        let path = format!(
            "{}/../../lib/libredwg/test/test-data/{file}",
            env!("CARGO_MANIFEST_DIR")
        );
        let db = uncad::parse(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let styles: Vec<&Ref<String>> = db
            .entities
            .iter()
            .filter_map(|e| match e {
                uncad::Entity::Attrib(a) => Some(&a.style_name),
                _ => None,
            })
            .collect();
        assert!(!styles.is_empty(), "{file}: attributes");
        for s in styles {
            assert_eq!(s, &Ref::Resolved(style.to_string()), "{file}");
        }
    }
}
