//! Phase 0 exit-criterion smoke test: read a real fixture through the
//! bindgen-generated FFI (not the C smoke test used to debug the build) and
//! print its object count.
//!
//! Run: cargo run -p libredwg-sys --example smoke -- <path/to/file.dwg>

use std::ffi::CString;
use std::mem::MaybeUninit;

fn main() {
    let dwg_path = std::env::args().nth(1).expect("usage: smoke <in.dwg>");
    let c_dwg_path = CString::new(dwg_path.as_str()).unwrap();

    // SAFETY: Dwg_Data is a plain-old-data FFI struct; dwg_read_file expects
    // a zero-initialized instance (mirrors dwg_read_file_wrapper's `Dwg_Data
    // dwg = {}` in the former JS/embind binding -- an uninitialized stack
    // struct was confirmed during Phase 0's C-level smoke test to produce a
    // STATUS_STACK_BUFFER_OVERRUN, traced to `dwg.opts` feeding garbage into
    // the runtime `loglevel` global).
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
