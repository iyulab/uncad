use std::path::PathBuf;

// Core LibreDWG sources actually needed for DWG/DXF *reading*, mirroring the
// former WASM build's `emmake make -C src` scope (which itself excludes
// examples/programs/test/). JSON/GeoJSON in/out modules are excluded outright
// (not compiled-and-ifdef'd-out) since this project never enables them --
// matches the WASM build's `--disable-json` flag.
//
// encode.c/encode2.c/out_dxf.c/out_dxfb.c are *writer* sources, and nothing
// in this workspace writes DWG or DXF any more (the write API was removed --
// see CHANGELOG.md). They stay compiled because reading depends on them:
// dwg.c gates dxf_read_file() on USE_WRITE, in_dxf.c calls encode.c's
// in_postprocess_handles()/in_postprocess_SEQEND() (and dwg.c's own
// USE_WRITE-gated dwg_find_tablehandle_silent()), and out_dxf.c hosts
// dwg_convert_SAB_to_SAT1(), which the 3DSOLID wireframe extraction
// needs. Only the symbols in the bindgen allowlist below reach Rust, and no
// writer entry point is among them.
const LIBREDWG_SOURCES: &[&str] = &[
    "bits.c",
    "classes.c",
    "codepages.c",
    "common.c",
    "decode.c",
    "decode2.c",
    "decode_r11.c",
    "decode_r2007.c",
    "dwg.c",
    "dwg_api.c",
    "dxfclasses.c",
    "dynapi.c",
    "encode.c",
    "encode2.c",
    "free.c",
    "geom.c",
    "hash.c",
    "in_dxf.c",
    "logging.c",
    "objects.c",
    "out_dxf.c",
    "out_dxfb.c",
    "print.c",
    "reedsolomon.c",
];

fn main() {
    // vendor-config/config.h hardcodes SIZEOF_SIZE_T to 8 with no platform
    // branch (unlike its neighboring SIZEOF_WCHAR_T, which does branch on
    // _WIN32) -- that value feeds LibreDWG's MAX_MEM_ALLOC allocation-size
    // sanity gate (bits.h) and multiple word-aligned fast-path reads in
    // bits.c. On a 32-bit target this crate has never been built or
    // validated on, that mismatch would silently misalign those reads and
    // make the allocation-size gate against attacker-controlled DWG size
    // fields far too permissive, rather than failing to compile. Fail
    // loudly here instead -- see docs/CAVEATS.md.
    let pointer_width =
        std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").expect("cargo sets this for build scripts");
    assert_eq!(
        pointer_width, "64",
        "libredwg-sys only supports 64-bit targets: vendor-config/config.h hardcodes \
         SIZEOF_SIZE_T=8 (and other 64-bit assumptions) with no 32-bit branch, unvalidated \
         and unsafe to silently use on a {pointer_width}-bit target. See docs/CAVEATS.md."
    );

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Build from the vendored copy inside the crate (vendor/libredwg), not
    // the workspace's lib/libredwg git submodule -- a `cargo package`/publish
    // tarball only contains files under the crate root, and a downstream
    // consumer building from crates.io has no submodule (no .git at all) to
    // fall back to. vendor/libredwg is a plain, git-tracked copy of exactly
    // the submodule files this crate's build actually reaches (traced via
    // the real #include graph, not just the top-level LIBREDWG_SOURCES
    // list) -- see scripts/sync-libredwg-vendor.sh and docs/ARCHITECTURE.md's
    // "Build" section for how to refresh it after a submodule update.
    let libredwg_src = manifest_dir.join("vendor/libredwg/src");
    let libredwg_include = manifest_dir.join("vendor/libredwg/include");
    let vendor_config = manifest_dir.join("vendor-config");
    let shim_dir = manifest_dir.join("shim");

    // Drift detector: fail loudly if vendor/libredwg/src's .c file count
    // doesn't match what this file expects to compile, rather than silently
    // compiling a stale/incomplete vendored copy. See docs/ARCHITECTURE.md's
    // "Build" section for the submodule-update / re-vendor procedure this is
    // meant to catch early.
    let actual_c_files: Vec<String> = std::fs::read_dir(&libredwg_src)
        .expect("crates/libredwg-sys/vendor/libredwg/src should exist (checked into git -- see scripts/sync-libredwg-vendor.sh)")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".c"))
        .collect();
    if actual_c_files.len() != LIBREDWG_SOURCES.len() {
        panic!(
            "vendor/libredwg/src/*.c file count changed ({} found, {} expected). The vendored \
             copy was likely edited or re-synced with a different file set -- re-check the \
             LIBREDWG_SOURCES list in build.rs against vendor/libredwg/src before proceeding \
             (see docs/ARCHITECTURE.md).",
            actual_c_files.len(),
            LIBREDWG_SOURCES.len()
        );
    }

    // Dev-only cross-check: when the lib/libredwg submodule is also checked
    // out (local dev / CI clone with submodules, not a published-crate
    // build), warn if its .c file set has drifted from the vendored copy --
    // i.e. someone ran `git submodule update --remote` without re-running
    // scripts/sync-libredwg-vendor.sh.
    if let Some(repo_root) = manifest_dir.parent().and_then(|p| p.parent()) {
        let submodule_src = repo_root.join("lib/libredwg/src");
        if let Ok(entries) = std::fs::read_dir(&submodule_src) {
            const EXCLUDED_JSON_SOURCES: &[&str] = &["in_json.c", "out_json.c", "out_geojson.c"];
            let submodule_c_count = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".c"))
                .count();
            let expected_submodule_total = LIBREDWG_SOURCES.len() + EXCLUDED_JSON_SOURCES.len();
            if submodule_c_count != expected_submodule_total {
                println!(
                    "cargo:warning=lib/libredwg/src/*.c ({submodule_c_count} files) has drifted \
                     from vendor/libredwg/src ({expected_submodule_total} expected: {} compiled + \
                     {} excluded JSON). Upstream LibreDWG was likely updated -- re-run \
                     scripts/sync-libredwg-vendor.sh and update LIBREDWG_SOURCES in build.rs.",
                    LIBREDWG_SOURCES.len(),
                    EXCLUDED_JSON_SOURCES.len()
                );
            }
        }
    }

    let mut build = cc::Build::new();
    build
        .include(&vendor_config)
        .include(&libredwg_src)
        .include(&libredwg_include)
        .warnings(false); // upstream C source, not ours to keep warning-clean

    if build.get_compiler().is_like_msvc() {
        // Vendored LibreDWG source has a UTF-8 comment (decode.c) that MSVC's
        // default code-page-based source interpretation misreads badly enough
        // to swallow the following source line into the comment. /utf-8 fixes
        // it and matches how every other toolchain already reads the file.
        build.flag("/utf-8").flag("/std:c11");
    }

    // --- bindgen -----------------------------------------------------------
    // Runs before the C compile on purpose: a missing libclang or an
    // unusable header search path then fails within seconds instead of after
    // the several-minute LibreDWG compile.
    //
    // Explicit --target so libclang follows the same ABI cl.exe used to
    // actually compile the library (matters for anyone building on a
    // different host/target combination than this was validated on).
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    let bindings = bindgen::Builder::default()
        .header(shim_dir.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("--target={target}"))
        .clang_arg("-xc")
        .clang_args(msvc_system_include_args(&build))
        .clang_arg("-std=c11")
        .clang_arg(format!("-I{}", vendor_config.display()))
        .clang_arg(format!("-I{}", libredwg_src.display()))
        .clang_arg(format!("-I{}", libredwg_include.display()))
        .clang_arg(format!("-I{}", shim_dir.display()))
        .allowlist_function("dwg_read_file")
        .allowlist_function("dxf_read_file")
        // Deliberately NOT allowlisted: dwg_write_file. It exists in the
        // compiled library (config.h defines USE_WRITE, which dxf_read_file
        // also needs -- see the LIBREDWG_SOURCES comment), but this
        // workspace has no write path; keeping the binding out makes that a
        // property of the crate rather than of its callers.
        .allowlist_function("dwg_get_num_objects")
        .allowlist_function("dwg_get_object")
        .allowlist_function("dwg_object_get_fixedtype")
        .allowlist_function("dwg_object_get_dxfname")
        .allowlist_function("dwg_object_get_handle")
        .allowlist_function("get_first_owned_entity")
        .allowlist_function("get_next_owned_entity")
        .allowlist_function("get_first_owned_subentity")
        .allowlist_function("get_next_owned_subentity")
        .allowlist_function("dwg_object_polyline_2d_get_numpoints")
        .allowlist_function("dwg_object_polyline_2d_get_points")
        .allowlist_function("dwg_object_polyline_3d_get_numpoints")
        .allowlist_function("dwg_object_polyline_3d_get_points")
        // Named explicitly because src/lib.rs's hand-written
        // Dwg_MLINE_vertex refers to it, not only because the polyline
        // accessors above return it.
        .allowlist_type("dwg_point_3d")
        .allowlist_function("uncad_object_entity_ptr")
        .allowlist_function("uncad_object_object_ptr")
        .allowlist_function("uncad_multileader_get_lines")
        .allowlist_function("uncad_multileader_free_lines")
        .allowlist_type("uncad_multileader_line_t")
        .allowlist_function("dwg_free")
        .allowlist_function("uncad_3dsolid_sab_to_sat_text")
        .allowlist_function("uncad_free_sat_text")
        .allowlist_function("dwg_dynapi_.*")
        .allowlist_type("Dwg_Data")
        .allowlist_type("Dwg_Object")
        .allowlist_type("Dwg_Object_Type")
        // Dwg_Object's `tio` union alone pulls in ~90 Dwg_Entity_*/
        // Dwg_Object_* struct types (one pointer variant per DWG entity/object
        // type), one of which bindgen cannot lay out cleanly. Its failure mode
        // is silent and self-contradictory: it emits a 1-byte placeholder
        // struct body but keeps the correct clang-computed size in the
        // layout_tests() assertion, so that assertion always fails even though
        // clang itself parses everything fine (a standalone clang compile with
        // identical flags reports the right sizeof() for every one of these
        // types).
        //
        // No field-level Rust access into these types is needed: entity data
        // always goes through dwg_dynapi_entity_value()/
        // dwg_dynapi_entity_field(), and the Dwg_Object fields this workspace
        // reads (fixedtype, handle.value) come from C accessor functions. So
        // the root structs are opaqued outright -- bindgen then represents
        // them as correctly-sized `[u8; N]` blobs, safe to zero, stack- or
        // heap-allocate and pass by pointer, without ever laying out the
        // problematic union.
        .opaque_type("_dwg_struct")
        .opaque_type("Dwg_Data")
        .opaque_type("dwg_data") // dwg_api.h's own separate `typedef struct _dwg_struct dwg_data;`
        .opaque_type("_dwg_object")
        .opaque_type("Dwg_Object")
        .opaque_type("dwg_object") // dwg_api.h's own separate `typedef struct _dwg_object dwg_object;`
        // Same reasoning: these are parameter or return types of
        // otherwise-allowlisted functions even with Dwg_Object itself opaque.
        // Dwg_Entity__3DSOLID stays listed because bindgen still reaches it
        // transitively and emits it as an opaque blob, even though nothing
        // allowlisted names it directly any more (the SAB conversion goes
        // through the uncad_3dsolid_sab_to_sat_text shim, which takes a
        // void*).
        .opaque_type("_dwg_object_entity")
        .opaque_type("Dwg_Object_Entity")
        .opaque_type("_dwg_object_object")
        .opaque_type("Dwg_Object_Object")
        .opaque_type("_dwg_entity_3DSOLID")
        .opaque_type("Dwg_Entity__3DSOLID")
        .allowlist_type("DWG_ERROR")
        .allowlist_type("Dwg_DYNAPI_field")
        // Referenced by src/lib.rs's hand-written Dwg_HATCH_Path.
        .allowlist_type("Dwg_HATCH_PolylinePath")
        // The HATCH and MLINE sub-structs below hit the same bindgen
        // struct-codegen failure as Dwg_Object, but unlike Dwg_Object this
        // crate needs real field access into them: HATCH boundary-path
        // geometry is not reachable through dynapi at all, which only exposes
        // top-level entity fields by name. Opaquing them removes field access
        // entirely, and opaquing the types they reference does not help
        // either. So they are blocklisted here and hand-written in src/lib.rs
        // against dwg.h's field layout, each with a compile-time assertion on
        // clang's own sizeof() (the value bindgen's own layout_tests()
        // reported before blocklisting).
        //
        // Several of them cascade for one specific reason: a `parent` field
        // typed `struct _dwg_entity_HATCH *` or `struct _dwg_object_MLINESTYLE
        // *` forces bindgen to materialize that parent as a real, non-opaque
        // type, and the parent is what fails. The hand-written versions leave
        // `parent` as an untyped `*mut c_void` to break the chain.
        .blocklist_type("_dwg_HATCH_Path")
        .blocklist_type("Dwg_HATCH_Path")
        .blocklist_type("_dwg_HATCH_PathSeg")
        .blocklist_type("Dwg_HATCH_PathSeg")
        .blocklist_type("_dwg_HATCH_ControlPoint")
        .blocklist_type("Dwg_HATCH_ControlPoint")
        .blocklist_type("_dwg_HATCH_DefLine")
        .blocklist_type("Dwg_HATCH_DefLine")
        .blocklist_type("_dwg_HATCH_Color")
        .blocklist_type("Dwg_HATCH_Color")
        .blocklist_type("_dwg_MLINE_vertex")
        .blocklist_type("Dwg_MLINE_vertex")
        // Dwg_MLINE_line is referenced by Dwg_MLINE_vertex.lines and by
        // nothing this crate allowlists, yet a Linux/gcc build still
        // generates it -- with a dangling reference to the blocklisted
        // _dwg_MLINE_vertex, which does not compile. Windows/MSVC does not.
        .blocklist_type("_dwg_MLINE_line")
        .blocklist_type("Dwg_MLINE_line")
        .blocklist_type("_dwg_MLINESTYLE_line")
        .blocklist_type("Dwg_MLINESTYLE_line")
        .allowlist_type("Dwg_Handle")
        .allowlist_type("_dwg_handle")
        .allowlist_type("Dwg_Object_Ref")
        .allowlist_type("_dwg_object_ref")
        .allowlist_type("Dwg_Color")
        .allowlist_type("_dwg_color")
        .allowlist_type("DWG_COLOR_METHOD")
        .allowlist_type("Dwg_Color_Method")
        .allowlist_var("DWG_ERR_.*")
        .derive_default(true)
        .generate_comments(true)
        .layout_tests(true)
        .generate()
        .expect("bindgen should generate libredwg bindings");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write bindings.rs");

    // --- C compile ---------------------------------------------------------
    for src in LIBREDWG_SOURCES {
        build.file(libredwg_src.join(src));
    }
    build.file(shim_dir.join("uncad_shim.c"));

    build.compile("libredwg");

    println!("cargo:rerun-if-changed={}", shim_dir.display());
    println!("cargo:rerun-if-changed={}", vendor_config.display());
    // The vendored C sources are the actual compile input, and neither cc
    // (for .file() sources) nor bindgen (without CargoCallbacks) registers
    // them with Cargo. Once any rerun-if-changed is printed Cargo watches
    // *only* the named paths, so without this line a refreshed vendor/ (from
    // scripts/sync-libredwg-vendor.sh) silently reused stale objects and a
    // stale bindings.rs until `cargo clean -p libredwg-sys`. A directory
    // path makes Cargo scan the whole tree (112 files -- negligible).
    println!(
        "cargo:rerun-if-changed={}",
        libredwg_src
            .parent()
            .expect("vendor/libredwg/src always has a parent directory")
            .display()
    );
}

/// `-isystem` arguments for the MSVC and Windows SDK header directories the
/// `cc` crate located, so libclang can resolve the C standard headers the
/// same way `cl.exe` does.
///
/// `cc` finds an installed MSVC toolchain on its own (registry / vswhere),
/// so the C compile works from any shell. libclang has no such discovery: it
/// only sees those directories when the `INCLUDE` environment variable is
/// already set, i.e. inside a Visual Studio developer prompt. Forwarding what
/// `cc` found makes both halves of this build script agree. Empty on
/// non-MSVC targets, and harmless inside a developer prompt (same paths).
fn msvc_system_include_args(build: &cc::Build) -> Vec<String> {
    let compiler = build.get_compiler();
    if !compiler.is_like_msvc() {
        return Vec::new();
    }
    compiler
        .env()
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("INCLUDE"))
        .flat_map(|(_, value)| std::env::split_paths(value).collect::<Vec<_>>())
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| format!("-isystem{}", dir.display()))
        .collect()
}
