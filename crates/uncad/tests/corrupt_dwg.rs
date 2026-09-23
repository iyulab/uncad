//! A corrupt DWG must come back as a value or a `ParseError`, never as a
//! dead process -- here for the one place this crate's own conversion
//! recurses. (The corruptions that kill the process inside LibreDWG itself
//! are pinned with the vendored patches that fix them, in
//! `tests/vendored_patches.rs`.)

use std::fs;
use std::path::PathBuf;

const EXAMPLE_2000: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Three bytes of `example_2000.dwg`, bisected out of a fuzzed file's 1 155
/// mutated offsets, that damage an INSERT's attribute chain.
///
/// An INSERT owns its ATTRIBs, so converting one converts them too -- the
/// single place the conversion recurses. With these bytes the chain leads
/// back to the INSERT, and a walk that converted whatever the chain pointed
/// at recursed until the stack ran out: not deep recursion but endless, since
/// a 512 MB stack did not survive it either (STATUS_STACK_OVERFLOW,
/// 0xC00000FD). The decoder itself is untouched by this: reading the same
/// bytes without converting them returns normally.
///
/// The walk converts only ATTRIBs, stops at the first object that is not
/// one, converts them one level down with a cap on the depth, and hands back
/// at most `MAX_OWNED_SUBENTITIES` (100,000) of them.
const ATTRIB_CHAIN_FLIPS: [(usize, u8, u8); 3] = [
    (581_356, 0x32, 0xC4),
    (581_784, 0xC6, 0x6A),
    (582_336, 0x34, 0x30),
];

#[test]
fn an_inserts_attribute_chain_leading_back_to_itself_does_not_kill_the_process() {
    let mut bytes = fs::read(EXAMPLE_2000).expect("the corpus DWG is readable");
    for (offset, original, corrupt) in ATTRIB_CHAIN_FLIPS {
        assert_eq!(
            bytes.get(offset).copied(),
            Some(original),
            "the corpus file changed: byte {offset} is no longer the one this \
             regression was minimised against"
        );
        bytes[offset] = corrupt;
    }
    let path = std::env::temp_dir().join(format!("uncad-{}-attrib-chain.dwg", std::process::id()));
    fs::write(&path, &bytes).expect("the temp dir should be writable");
    let file = TempFile(path);

    // Reaching the next line at all is the assertion.
    let drawing = uncad::parse(&file.0).expect("the corrupted file still parses");

    // The damage is reported, not hidden: LibreDWG met handles it could not
    // resolve.
    assert!(
        !drawing.read_diagnostics.is_clean(),
        "a damaged drawing read with nothing to say"
    );
    // And the walk terminated with a sane answer rather than by running out
    // of some other resource: a ring would have yielded the cap.
    let attribs: usize = drawing
        .tables
        .block_records
        .values()
        .flat_map(|b| b.entities.iter())
        .filter_map(|e| match e {
            uncad::Entity::Insert(i) => Some(i.attribs.len()),
            _ => None,
        })
        .sum();
    assert!(attribs < 100_000, "{attribs} attributes were collected");
}
