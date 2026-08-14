use std::path::PathBuf;

// Core LibreDWG sources actually needed for DWG/DXF read + DXF write, mirroring
// the existing WASM build's `emmake make -C src` scope (which itself excludes
// examples/programs/test/). JSON/GeoJSON in/out modules are excluded outright
// (not compiled-and-ifdef'd-out) since this project never enables them --
// matches the WASM build's `--disable-json` flag.
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
    // repo_root = crates/libredwg-sys/../..
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/libredwg-sys should be two levels under the repo root");

    let libredwg_src = repo_root.join("lib/libredwg/src");
    let libredwg_include = repo_root.join("lib/libredwg/include");
    let vendor_config = manifest_dir.join("vendor-config");
    let shim_dir = manifest_dir.join("shim");

    // Drift detector: fail loudly if upstream lib/libredwg/src gains or
    // loses .c files on a future submodule update, rather than silently
    // compiling a stale file list. See docs/ARCHITECTURE.md's "빌드" section
    // for the submodule-update procedure this is meant to catch early.
    let actual_c_files: Vec<String> = std::fs::read_dir(&libredwg_src)
        .expect("lib/libredwg/src should exist (did you run `git submodule update --init`?)")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".c"))
        .collect();
    const EXCLUDED_JSON_SOURCES: &[&str] = &["in_json.c", "out_json.c", "out_geojson.c"];
    let expected_total = LIBREDWG_SOURCES.len() + EXCLUDED_JSON_SOURCES.len();
    if actual_c_files.len() != expected_total {
        panic!(
            "lib/libredwg/src/*.c file count changed ({} found, {} expected: {} compiled + {} excluded JSON). \
             Upstream LibreDWG source was likely updated -- re-check the LIBREDWG_SOURCES list in build.rs \
             against the new file list before proceeding (see docs/ARCHITECTURE.md).",
            actual_c_files.len(), expected_total, LIBREDWG_SOURCES.len(), EXCLUDED_JSON_SOURCES.len()
        );
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

    for src in LIBREDWG_SOURCES {
        build.file(libredwg_src.join(src));
    }
    build.file(shim_dir.join("uncad_shim.c"));

    build.compile("libredwg");

    // --- bindgen -----------------------------------------------------------
    // Explicit --target so libclang follows the same ABI cl.exe used to
    // actually compile the library (matters for anyone building on a
    // different host/target combination than this was validated on).
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    let bindings = bindgen::Builder::default()
        .header(shim_dir.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("--target={target}"))
        .clang_arg("-xc")
        .clang_arg("-std=c11")
        .clang_arg(format!("-I{}", vendor_config.display()))
        .clang_arg(format!("-I{}", libredwg_src.display()))
        .clang_arg(format!("-I{}", libredwg_include.display()))
        .clang_arg(format!("-I{}", shim_dir.display()))
        .allowlist_function("dwg_read_file")
        .allowlist_function("dxf_read_file")
        // Public, already-compiled (this crate's config.h defines USE_WRITE,
        // required for dwg_write_file to even exist in dwg.c -- see
        // docs/CAVEATS.md's DWG/DXF-writing section). Reliable for <= R_2004
        // per LibreDWG's own README ("Write support only works for earlier
        // versions until r2004"); despite the public `const Dwg_Data*`
        // signature, dwg_write_file's own implementation casts away that
        // const and mutates internally (`dwg_encode((Dwg_Data*)dwg, &dat)`,
        // confirmed by reading dwg.c directly) -- CadDatabase::write_dwg
        // takes `&mut self` to reflect that honestly rather than trust the
        // C signature's claim.
        .allowlist_function("dwg_write_file")
        .allowlist_function("dwg_get_num_objects")
        .allowlist_function("dwg_get_object")
        .allowlist_function("dwg_object_get_fixedtype")
        .allowlist_function("dwg_object_get_supertype")
        .allowlist_function("dwg_object_get_dxfname")
        .allowlist_function("dwg_object_get_name")
        .allowlist_function("dwg_object_get_handle")
        .allowlist_function("dwg_object_to_entity")
        .allowlist_function("dwg_object_to_object")
        .allowlist_function("get_first_owned_entity")
        .allowlist_function("get_next_owned_entity")
        .allowlist_function("get_first_owned_subentity")
        .allowlist_function("get_next_owned_subentity")
        .allowlist_function("dwg_object_polyline_2d_get_numpoints")
        .allowlist_function("dwg_object_polyline_2d_get_points")
        .allowlist_function("dwg_object_polyline_3d_get_numpoints")
        .allowlist_function("dwg_object_polyline_3d_get_points")
        .allowlist_type("dwg_point_2d")
        .allowlist_type("dwg_point_3d")
        .allowlist_function("uncad_object_entity_ptr")
        .allowlist_function("uncad_object_object_ptr")
        .allowlist_function("uncad_multileader_get_lines")
        .allowlist_function("uncad_multileader_free_lines")
        .allowlist_type("uncad_multileader_line_t")
        .allowlist_function("dwg_free")
        .allowlist_function("dwg_free_object")
        .allowlist_function("dwg_abandon")
        .allowlist_function("dwg_convert_SAB_to_SAT1")
        .allowlist_function("dwg_dynapi_.*")
        .allowlist_function("dwg_obj_.*")
        .allowlist_function("dwg_ref_.*")
        .allowlist_function("dwg_resolve_handleref")
        .allowlist_function("uncad_write_dxf_file")
        .allowlist_function("uncad_write_dxf")
        .allowlist_type("Dwg_Data")
        .allowlist_type("Dwg_Object")
        .allowlist_type("Dwg_Object_Type")
        // Dwg_Object's `tio` union alone pulls in ~90 Dwg_Entity_*/Dwg_Object_*
        // struct types (one pointer variant per DWG entity/object type), one
        // of which bindgen cannot lay out cleanly (root cause not isolated --
        // candidates include the #pragma pack(1) bitfields + flexible-array-
        // in-union EED record type reachable transitively). bindgen's failure
        // mode for that is silent and self-contradictory: it emits a
        // 1-byte placeholder struct body but keeps the *correct*
        // clang-computed size in the layout_tests() assertion, so the
        // assertion always fails even though clang itself parses everything
        // fine (confirmed: a standalone clang.exe compile with identical
        // flags reports the right sizeof() for every one of these types).
        //
        // This project's design never needed field-level Rust access into
        // these types anyway -- entity data always goes through
        // dwg_dynapi_entity_value()/dwg_dynapi_entity_field() (see the
        // Dwg_Object.tio union-safety note in the plan), and Dwg_Object's own
        // fields we need (fixedtype, handle.value) are read via the C
        // accessor functions dwg_object_get_fixedtype()/dwg_obj_get_handle_value()
        // rather than direct struct-field access, exactly like the existing
        // JS binding already does. So: opaque the two root structs outright
        // (bindgen then represents them as correctly-sized `[u8; N]` blobs,
        // safe to stack-allocate/zero and pass by pointer to the C API,
        // without ever trying to lay out the problematic union).
        .opaque_type("_dwg_struct")
        .opaque_type("Dwg_Data")
        .opaque_type("dwg_data") // dwg_api.h's own separate `typedef struct _dwg_struct dwg_data;`
        .opaque_type("_dwg_object")
        .opaque_type("Dwg_Object")
        .opaque_type("dwg_object") // dwg_api.h's own separate `typedef struct _dwg_object dwg_object;`
        // Same reasoning, needed because these are parameter/return types of
        // otherwise-allowlisted functions (dwg_convert_SAB_to_SAT1,
        // dwg_dynapi_*, dwg_obj_*) even with Dwg_Object itself opaque.
        .opaque_type("_dwg_object_entity")
        .opaque_type("Dwg_Object_Entity")
        .opaque_type("_dwg_object_object")
        .opaque_type("Dwg_Object_Object")
        .opaque_type("_dwg_entity_3DSOLID")
        .opaque_type("Dwg_Entity__3DSOLID")
        .allowlist_type("DWG_ERROR")
        .allowlist_type("Dwg_Error")
        .allowlist_type("Dwg_DYNAPI_field")
        .allowlist_type("dwg_field_name_type_offset")
        .allowlist_type("Dwg_HATCH_PolylinePath")
        // Dwg_HATCH_Path/Dwg_HATCH_PathSeg hit the same bindgen struct-
        // codegen limitation as Dwg_Object (see the big opaque_type block
        // above) -- but unlike Dwg_Object, this project actually needs
        // real field access into these two (HATCH boundary-path geometry
        // isn't reachable through dynapi at all, since dynapi only exposes
        // top-level entity fields by name, not nested struct internals).
        // Opaquing them (tried first) doesn't help since that removes
        // field access entirely, and neither does opaquing the types they
        // reference (Dwg_HATCH_ControlPoint, Dwg_Entity_HATCH) -- the
        // cascade's real trigger was never isolated. Blocklisted here and
        // hand-defined instead, in src/lib.rs, matching dwg.h's field
        // layout exactly (verified against clang's own authoritative
        // sizeof(): 56 and 200 bytes respectively, both confirmed by the
        // layout_tests() assertions bindgen itself generated before
        // blocklisting -- i.e. clang parses these types fine, only
        // bindgen's Rust codegen for them specifically fails).
        .blocklist_type("_dwg_HATCH_Path")
        .blocklist_type("Dwg_HATCH_Path")
        .blocklist_type("_dwg_HATCH_PathSeg")
        .blocklist_type("Dwg_HATCH_PathSeg")
        .blocklist_type("_dwg_HATCH_ControlPoint")
        .blocklist_type("Dwg_HATCH_ControlPoint")
        // Dwg_HATCH_DefLine (the pattern-fill line-family definitions --
        // angle/pt0/offset/dashes -- used to render actual hatch patterns
        // instead of outline-only) is a top-level Dwg_Entity_HATCH field
        // (num_deflines/deflines), reachable through dynapi unlike the
        // Path/PathSeg/ControlPoint types above -- but allowlisting it
        // directly was tried first and made things *worse*: its `parent`
        // field is typed `struct _dwg_entity_HATCH *`, which forces bindgen
        // to also generate a real (non-opaque) _dwg_entity_HATCH just to
        // have a pointee type, and _dwg_entity_HATCH itself then hits the
        // same struct-codegen failure (confirmed: allowlisting DefLine
        // alone produced a `1usize - 200usize` layout_tests() assert on
        // _dwg_entity_HATCH, which nothing else in this crate ever
        // allowlists directly -- HATCH's own fields are all read through
        // dynapi's untyped void*). Blocklisted and hand-defined instead,
        // same as the three types above, with `parent` left as an untyped
        // `*mut c_void` (matching Dwg_HATCH_ControlPoint/PathSeg/Path's own
        // `parent` fields) specifically to avoid pulling _dwg_entity_HATCH
        // in at all.
        .blocklist_type("_dwg_HATCH_DefLine")
        .blocklist_type("Dwg_HATCH_DefLine")
        // Dwg_HATCH_Color (one gradient-fill color stop: shift_value +
        // Dwg_Color) has the exact same `parent: struct _dwg_entity_HATCH *`
        // cascade as Dwg_HATCH_DefLine right above -- allowlisting it forces
        // bindgen to materialize a real _dwg_entity_HATCH, which fails the
        // same way. Blocklisted and hand-defined instead, same recipe.
        .blocklist_type("_dwg_HATCH_Color")
        .blocklist_type("Dwg_HATCH_Color")
        // Dwg_MLINE_vertex hits the exact same bindgen struct-codegen
        // failure (confirmed: allowlisting it alone produces a
        // layout_tests() assert computing `1usize - 96usize`, i.e. bindgen
        // silently generated a 1-byte placeholder while clang's own
        // sizeof() -- what the assert checks against -- is the correct 96).
        // Blocklisted and hand-defined instead, same as the HATCH types
        // above. Dwg_MLINE_line (referenced by Dwg_MLINE_vertex.lines, and
        // by nothing this crate allowlists directly) also has to be
        // blocklisted here -- confirmed via Docker Linux/gcc build that,
        // unlike on the Windows/MSVC target, bindgen there still generates
        // it (a dangling reference to the now-blocklisted
        // _dwg_MLINE_vertex, a compile error) even though nothing
        // allowlisted uses it; same class of platform-conditional bindgen
        // divergence as docs/CAVEATS.md's other Windows/Linux notes.
        .blocklist_type("_dwg_MLINE_vertex")
        .blocklist_type("Dwg_MLINE_vertex")
        .blocklist_type("_dwg_MLINE_line")
        .blocklist_type("Dwg_MLINE_line")
        // Dwg_MLINESTYLE_line (one parallel-line definition in an
        // MLINESTYLE OBJECT -- offset/color/linetype -- read to give MLINE
        // its real per-line offsets instead of centerline-only rendering)
        // has the same `parent: struct _dwg_object_MLINESTYLE *` cascade as
        // the HATCH sub-structs above (confirmed: allowlisting it alone
        // produces a `1usize - 112usize` layout_tests() assert on
        // _dwg_object_MLINESTYLE). Blocklisted and hand-defined instead,
        // same recipe -- verified against clang's real sizeof() (80 bytes)
        // the same way.
        .blocklist_type("_dwg_MLINESTYLE_line")
        .blocklist_type("Dwg_MLINESTYLE_line")
        .allowlist_type("Dwg_Handle")
        .allowlist_type("_dwg_handle")
        .allowlist_type("Dwg_Object_Supertype")
        .allowlist_type("Dwg_Object_Ref")
        .allowlist_type("_dwg_object_ref")
        .allowlist_type("Dwg_Color")
        .allowlist_type("_dwg_color")
        .allowlist_type("DWG_COLOR_METHOD")
        .allowlist_type("Dwg_Color_Method")
        .allowlist_var("DWG_NOERR")
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

    println!("cargo:rerun-if-changed={}", shim_dir.display());
    println!("cargo:rerun-if-changed={}", vendor_config.display());
}
