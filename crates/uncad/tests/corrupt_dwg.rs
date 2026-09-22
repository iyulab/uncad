//! A corrupt DWG must come back as a value or a `ParseError`, never as a
//! dead process.
//!
//! LibreDWG's header decoder hands every TIMEBLL it reads to `cvt_TIMEBLL`
//! and the result straight to `strftime` (`header_variables.spec`'s DECODER
//! block, which runs at any log level). A TIMEBLL is two raw 32-bit words
//! off the bit stream, so a corrupt file reaches `strftime` with a struct
//! tm whose year/month/hour are far out of range -- and Microsoft's UCRT
//! `strftime` *validates* its struct tm and fail-fasts the process with
//! 0xC0000409 on an out-of-range field. That abort happens inside the C
//! library, below the FFI boundary, so no `catch_unwind` can see it: a test
//! that trips it does not fail, it takes the whole test binary down. See
//! the `uncad local patch` block in
//! `crates/libredwg-sys/vendor/libredwg/src/common.c` and `docs/CAVEATS.md`.

use uncad::{parse_bytes, Format};

const HELIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/Helix.dwg"
);

/// The single byte that turned this R2000 drawing into a process killer,
/// minimised from a fuzzed corpus file by bisecting over 89 mutated
/// offsets. It sits in the header-variables bit stream just before
/// `TDUUPDATE`, so flipping it is enough to give that TIMEBLL a wild value.
const OFFSET: usize = 27644;
const ORIGINAL: u8 = 0x25;
const CORRUPT: u8 = 0x80;

#[test]
fn a_one_byte_corruption_of_a_header_date_does_not_kill_the_process() {
    let mut bytes = std::fs::read(HELIX).expect("the corpus DWG is readable");
    assert_eq!(
        bytes.get(OFFSET).copied(),
        Some(ORIGINAL),
        "the corpus file changed: byte {OFFSET} is no longer the one this \
         regression was minimised against"
    );

    // The file as it stands must still read, so the assertion below is about
    // the corruption and not about Helix.dwg.
    let clean = parse_bytes(&bytes, Format::Dwg).expect("the unmodified drawing parses");
    assert!(!clean.entities.is_empty());

    bytes[OFFSET] = CORRUPT;
    // Reaching the next line at all is the assertion: before the fix this
    // call terminated the test binary with STATUS_STACK_BUFFER_OVERRUN
    // (0xC0000409), printing nothing. Either outcome is acceptable now --
    // LibreDWG may still decode the rest of the file or give up on it.
    match parse_bytes(&bytes, Format::Dwg) {
        Ok(_) | Err(_) => {}
    }
}

#[test]
fn a_truncated_dwg_does_not_kill_the_process() {
    // Same rule for the other shape of corruption the sweep exercised: a
    // file cut short mid-header, so the TIMEBLLs are read off the end of
    // the (zero-padded) chain rather than off wrong-but-present bits.
    let bytes = std::fs::read(HELIX).expect("the corpus DWG is readable");
    for fraction in [2usize, 6, 17, 50, 83] {
        let cut = bytes.len() * fraction / 100;
        match parse_bytes(&bytes[..cut], Format::Dwg) {
            Ok(_) | Err(_) => {}
        }
    }
}
