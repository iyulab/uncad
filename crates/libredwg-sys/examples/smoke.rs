//! Phase 0 exit-criterion smoke test: read a real fixture through the
//! bindgen-generated FFI (not the C smoke test used to debug the build),
//! print its object count, and round-trip it through the DXF-write shim.
//!
//! Run: cargo run -p libredwg-sys --example smoke -- <path/to/file.dwg> <path/to/out.dxf>

use std::ffi::CString;
use std::mem::MaybeUninit;

fn main() {
    let mut args = std::env::args().skip(1);
    let dwg_path = args.next().expect("usage: smoke <in.dwg> <out.dxf>");
    let dxf_path = args.next().expect("usage: smoke <in.dwg> <out.dxf>");

    let c_dwg_path = CString::new(dwg_path.as_str()).unwrap();

    // SAFETY: Dwg_Data is a plain-old-data FFI struct; dwg_read_file expects
    // a zero-initialized instance (mirrors dwg_read_file_wrapper's `Dwg_Data
    // dwg = {}` in the existing JS/embind binding -- an uninitialized stack
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

    let c_dxf_path = CString::new(dxf_path.as_str()).unwrap();
    let write_err =
        unsafe { libredwg_sys::uncad_write_dxf_file(c_dwg_path.as_ptr(), c_dxf_path.as_ptr()) };
    println!("uncad_write_dxf_file -> error={write_err}");
    assert!(write_err < critical, "critical write error: {write_err}");

    let written = std::fs::read(&dxf_path).expect("dxf output should exist");
    println!("wrote {} bytes to {dxf_path}", written.len());

    // sanity: DXF text starts with the "999" comment-group-code convention
    // and ends with EOF, matching test/core.test.mjs's dwgToDxf() assertion.
    let text = String::from_utf8_lossy(&written);
    assert!(
        text.starts_with("999"),
        "DXF should start with group code 999"
    );
    assert!(text.trim_end().ends_with("EOF"), "DXF should end with EOF");
    println!("DXF header/footer sanity check passed");
}
