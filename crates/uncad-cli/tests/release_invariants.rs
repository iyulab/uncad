//! What has to be true of the repository before a `cargo publish`.
//!
//! **Licence files reach the tarball.** `cargo package` never includes a file
//! from outside the crate directory, so the repository root's `LICENSE` is
//! not enough: every published crate needs its own copy (GPLv3 §4), and the
//! modified vendored LibreDWG needs its modification notice inside the crate
//! too (GPLv3 §5(a)). Both were missing from 0.2.0 -- `uncad-cli-0.2.0.crate`
//! shipped no licence text at all -- and were verified only by someone
//! remembering.
//!
//! Like `documented_invocations.rs`, this reads the repository it is checked
//! into: run it from the workspace, not from an unpacked crate tarball.

use std::fs;
use std::path::{Path, PathBuf};

/// The three published crates, in workspace order.
const MEMBERS: [&str; 3] = ["libredwg-sys", "uncad", "uncad-cli"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every crate a `cargo publish` uploads carries the GPLv3 text itself.
///
/// `cargo package --list` is the real check, but it needs the crates.io index
/// and a clean VCS state, so it cannot run here. The property it would prove
/// is this one: the file is inside the crate directory, which is exactly what
/// decides whether the tarball has it.
#[test]
fn every_published_crate_ships_the_gpl_text() {
    let root = repo_root();
    let canonical = read(root.join("LICENSE"));
    assert!(
        canonical.contains("GNU GENERAL PUBLIC LICENSE")
            && canonical.contains("Version 3, 29 June 2007"),
        "the repository's LICENSE should be the GPLv3 text"
    );

    for member in MEMBERS {
        let path = root.join("crates").join(member).join("LICENSE");
        assert!(
            path.exists(),
            "crates/{member}/LICENSE is missing: `cargo package` only includes files under \
             the crate directory, so this crate would be published with no licence text"
        );
        assert_eq!(
            read(&path),
            canonical,
            "crates/{member}/LICENSE has drifted from the repository's LICENSE"
        );
    }
}

/// The bundled third-party source keeps its own licence beside it: LibreDWG's
/// `COPYING` in `libredwg-sys`.
#[test]
fn the_vendored_libredwg_ships_its_own_licence() {
    let copying = read(repo_root().join("crates/libredwg-sys/vendor/libredwg/COPYING"));
    assert!(
        copying.contains("GNU GENERAL PUBLIC LICENSE"),
        "LibreDWG's own COPYING should stay beside the vendored sources"
    );
}

/// Every vendored LibreDWG file carrying an `uncad local patch` marker, as
/// its `/`-separated path under `vendor/libredwg`, found by walking the tree
/// rather than trusting a list.
fn patched_vendor_files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, vendor: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("the vendored tree is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, vendor, out);
            } else if fs::read(&path)
                .expect("a readable vendored file")
                .windows(b"uncad local patch".len())
                .any(|w| w == b"uncad local patch")
            {
                out.push(
                    path.strip_prefix(vendor)
                        .expect("inside the vendored tree")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let vendor = root.join("crates/libredwg-sys/vendor/libredwg");
    let mut out = Vec::new();
    walk(&vendor, &vendor, &mut out);
    out.sort();
    out
}

/// The comment a source file opens with -- its `//!` lines, or its first
/// `/* ... */` block: what a reader of that file is told it is.
fn leading_comment(text: &str) -> String {
    if text.starts_with("//!") {
        return text
            .lines()
            .take_while(|line| line.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    assert!(
        text.starts_with("/*"),
        "the file should open with a comment"
    );
    let end = text.find("*/").expect("the opening comment is closed");
    text[..end].to_string()
}

/// Nothing may call the vendored LibreDWG unmodified while it carries local
/// patches -- not the crate description crates.io shows, not the crate's own
/// documentation, and not the project's documents. GPLv3 §5(a) asks for the
/// opposite, and every one of these used to say it.
#[test]
fn nothing_describes_the_patched_libredwg_as_unmodified() {
    let root = repo_root();
    let patched = patched_vendor_files(&root);
    assert!(
        !patched.is_empty(),
        "expected the vendored LibreDWG to carry `uncad local patch` markers; if the \
         patches were dropped, the claims this test guards have to be revisited too"
    );

    // What the crate says about itself: the description crates.io shows, the
    // top of the rustdoc, and the config.h header that sits beside the tree.
    let manifest = read(root.join("crates/libredwg-sys/Cargo.toml"));
    let description = manifest
        .lines()
        .find(|line| line.starts_with("description ="))
        .expect("libredwg-sys declares a description");
    let rustdoc = leading_comment(&read(root.join("crates/libredwg-sys/src/lib.rs")));
    let config_h = leading_comment(&read(
        root.join("crates/libredwg-sys/vendor-config/config.h"),
    ));
    for (what, text) in [
        ("the crates.io description", description.to_string()),
        ("src/lib.rs's crate documentation", rustdoc),
        ("vendor-config/config.h's header", config_h),
    ] {
        assert!(
            !text.contains("unmodified"),
            "{what} calls the vendored LibreDWG unmodified, but {patched:?} carry local \
             patches:\n{text}"
        );
        assert!(
            text.contains("patch"),
            "{what} should say the vendored LibreDWG carries local patches:\n{text}"
        );
    }

    // The project's documents: each once described the vendored copy with one
    // of these phrases.
    for (doc, wrong) in [
        ("README.md", "holds the unmodified"),
        ("docs/ARCHITECTURE.md", "unmodified (see"),
        ("docs/THIRD_PARTY_NOTICES.md", "unmodified subset"),
        (
            "docs/THIRD_PARTY_NOTICES.md",
            "an unmodified, byte-for-byte copy",
        ),
        (
            "docs/THIRD_PARTY_NOTICES.md",
            "**Modified**: no — built directly",
        ),
    ] {
        let text = read(root.join(doc));
        assert!(
            !text.contains(wrong),
            "{doc} says \"{wrong}\" of the vendored LibreDWG while {patched:?} carry local \
             patches"
        );
        assert!(
            text.contains("local patch"),
            "{doc} should say the vendored LibreDWG carries local patches"
        );
    }

    // The markers point readers at NOTICE.md, which is inside the tarball
    // (docs/ is not), and at docs/CAVEATS.md for the reasoning: both have to
    // name every patched file.
    let notice = read(root.join("crates/libredwg-sys/NOTICE.md"));
    let caveats = read(root.join("docs/CAVEATS.md"));
    for file in &patched {
        assert!(
            notice.contains(&format!("vendor/libredwg/{file}")),
            "crates/libredwg-sys/NOTICE.md should name the patched vendor/libredwg/{file}"
        );
        assert!(
            caveats.contains(&format!("`{file}`")),
            "docs/CAVEATS.md's \"Local patches to the vendored LibreDWG\" should name the \
             patched {file}"
        );
    }
}
