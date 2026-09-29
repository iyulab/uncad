//! What the local patches to the vendored LibreDWG change, pinned from the
//! outside.
//!
//! `crates/libredwg-sys/vendor/libredwg/` carries `uncad local patch`
//! blocks (listed in `crates/libredwg-sys/NOTICE.md`, reasoned in
//! `docs/CAVEATS.md`, "Local patches to the vendored LibreDWG"). A re-vendor
//! that drops one is caught by `build.rs`, which counts the markers; these
//! tests catch a patch that is still marked but no longer does its job. The
//! corrupt header date below was checked against a build without the
//! patches: it ends that build's process with 0xC0000409.
//!
//! The `common_entity_data.spec` one, which puts an entity's true colour and
//! its transparency back in their own fields, is pinned below on the entity
//! it was measured on (HATCH 29F in `test-data/2004/HatchG.dwg`); the R13/R14
//! linetype one in the corpus twin comparison; and the `dwg.spec` one, an
//! R2010+ ATTRIB's text style, below.

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
