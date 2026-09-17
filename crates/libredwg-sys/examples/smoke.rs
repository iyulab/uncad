//! Manual smoke check for the raw FFI layer: reads a DWG through the
//! bindgen-generated bindings and prints its object count.
//!
//! Like every `examples/` target in this workspace it asserts nothing --
//! `cargo test` and `cargo clippy --workspace --all-targets` compile it, so a
//! public FFI signature that stops matching shows up as a CI build error. See
//! `docs/ARCHITECTURE.md`'s "Test layout" section for how the three test
//! layers divide up.
//!
//! Run: cargo run -p libredwg-sys --example smoke -- <path/to/file.dwg>

use std::ffi::CString;
use std::mem::MaybeUninit;

fn main() {
    let dwg_path = std::env::args().nth(1).expect("usage: smoke <in.dwg>");
    let c_dwg_path = CString::new(dwg_path.as_str()).unwrap();

    // SAFETY: Dwg_Data is a plain-old-data FFI struct, and dwg_read_file
    // requires a zero-initialized instance: an uninitialized stack struct
    // aborts with STATUS_STACK_BUFFER_OVERRUN, because `dwg.opts` then feeds
    // garbage into LibreDWG's runtime `loglevel` global.
    let mut dwg: libredwg_sys::Dwg_Data = unsafe { MaybeUninit::zeroed().assume_init() };

    let error = unsafe { libredwg_sys::dwg_read_file(c_dwg_path.as_ptr(), &mut dwg) };
    println!("dwg_read_file({dwg_path}) -> error={error}");
    // Constant cast, not `error`: DWG_ERROR's bindgen-inferred width isn't
    // stable across targets (u32 on Linux, i32 on MSVC -- see uncad's
    // lib.rs for the fuller explanation), but `error` itself (from
    // dwg_read_file's plain-`int` C return type) always is.
    #[allow(clippy::unnecessary_cast)]
    let critical = libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32;
    assert!(error < critical, "critical read error: {error}");

    let n = unsafe { libredwg_sys::dwg_get_num_objects(&dwg) };
    println!("num_objects={n}");

    unsafe { libredwg_sys::dwg_free(&mut dwg) };
}
