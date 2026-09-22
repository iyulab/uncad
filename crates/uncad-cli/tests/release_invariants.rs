//! What has to be true of the repository before a `cargo publish`.
//!
//! Two kinds of claim get checked here, both of which used to be verified only
//! by someone remembering:
//!
//! 1. **Licence files reach the tarball.** `cargo package` never includes a
//!    file from outside the crate directory, so the repository root's
//!    `LICENSE` is not enough: every published crate needs its own copy
//!    (GPLv3 §4), and the modified vendored LibreDWG needs its modification
//!    notice inside the crate too (GPLv3 §5(a)). Both were missing until
//!    0.3.0 -- `uncad-cli-0.2.0.crate` shipped no licence text at all.
//! 2. **Documents that count things agree with the tree.** README,
//!    `docs/ARCHITECTURE.md` and `docs/CAVEATS.md` each state how many
//!    integration tests there are and which files they live in, and each has
//!    been wrong at least once. The numbers below are derived from the tree
//!    at test time, never copied from the documents.
//!
//! Like `documented_invocations.rs`, this reads the repository it is checked
//! into: run it from the workspace, not from an unpacked crate tarball.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_uncad");

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

// --- 1. licence files ------------------------------------------------------

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

/// The two bundled third-party works keep their own licence inside the crate
/// that bundles them: the font in `uncad`, LibreDWG's own `COPYING` in
/// `libredwg-sys`.
#[test]
fn the_bundled_works_ship_their_own_licences() {
    let root = repo_root();

    let ofl = read(root.join("crates/uncad/fonts/OFL-NotoSansKR.txt"));
    assert!(
        ofl.contains("SIL OPEN FONT LICENSE Version 1.1"),
        "the bundled font's OFL text should travel with the font"
    );

    let copying = read(root.join("crates/libredwg-sys/vendor/libredwg/COPYING"));
    assert!(
        copying.contains("GNU GENERAL PUBLIC LICENSE"),
        "LibreDWG's own COPYING should stay beside the vendored sources"
    );
}

/// Collects every vendored LibreDWG source carrying an `uncad local patch`
/// marker, by walking the vendored tree rather than trusting a list.
fn patched_vendor_files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>, root: &Path) {
        for entry in fs::read_dir(dir).expect("the vendored tree is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, out, root);
            } else if let Ok(text) = fs::read_to_string(&path) {
                if text.contains("uncad local patch") {
                    out.push(
                        path.strip_prefix(root)
                            .expect("inside the vendored tree")
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
    }
    let vendor = root.join("crates/libredwg-sys/vendor/libredwg");
    let mut out = Vec::new();
    walk(&vendor, &mut out, &vendor);
    out.sort();
    out
}

/// Nothing may call the vendored LibreDWG unmodified while it carries local
/// patches -- neither the crate description crates.io shows, nor the project's
/// own notice file. GPLv3 §5(a) asks for the opposite.
#[test]
fn nothing_describes_the_patched_libredwg_as_unmodified() {
    let root = repo_root();
    let patched = patched_vendor_files(&root);
    assert!(
        !patched.is_empty(),
        "expected the vendored LibreDWG to carry `uncad local patch` markers; if the \
         patches were dropped, the claims this test guards have to be revisited too"
    );

    let manifest = read(root.join("crates/libredwg-sys/Cargo.toml"));
    let description = manifest
        .lines()
        .find(|line| line.starts_with("description ="))
        .expect("libredwg-sys declares a description");
    assert!(
        !description.contains("unmodified"),
        "libredwg-sys's crates.io description calls the vendored LibreDWG unmodified, but \
         {patched:?} carry local patches: {description}"
    );

    let notices = read(root.join("docs/THIRD_PARTY_NOTICES.md"));
    assert!(
        !notices.contains("unmodified subset"),
        "docs/THIRD_PARTY_NOTICES.md calls the vendored subset unmodified while {patched:?} \
         carry local patches"
    );

    // The markers point readers at a file, so that file has to be one the
    // tarball actually has -- docs/ is not in it.
    let notice = read(root.join("crates/libredwg-sys/NOTICE.md"));
    for file in &patched {
        let stem = file.rsplit('/').next().expect("a file name");
        assert!(
            notice.contains(stem),
            "crates/libredwg-sys/NOTICE.md should name the patched {stem}"
        );
    }
}

// --- 2. the CLI says which version it is -----------------------------------

/// The version `uncad --version` prints, read from the manifest rather than
/// from `CARGO_PKG_VERSION`, so the test does not agree with the binary just
/// by sharing its source.
fn manifest_version() -> String {
    let manifest = read(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let line = manifest
        .lines()
        .find(|line| line.starts_with("version ="))
        .expect("uncad-cli declares a version");
    line.split('"')
        .nth(1)
        .expect("version is spelled as a quoted string")
        .to_string()
}

/// `uncad --version` used to be refused as an unknown option, leaving an
/// installed binary with no way to say what it was.
#[test]
fn the_version_flag_reports_the_installed_version() {
    let expected = format!("uncad {}", manifest_version());

    for args in [
        vec!["--version"],
        vec!["-V"],
        // Both commands answer it, and neither asks for an input file first.
        vec!["export", "--version"],
    ] {
        let out = Command::new(EXE)
            .args(&args)
            .output()
            .expect("the CLI should be runnable");
        assert!(out.status.success(), "{args:?} should exit 0");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            expected,
            "{args:?} should print the crate version on stdout"
        );
    }

    // And --help says it too, so a pasted usage block identifies the build.
    let help = Command::new(EXE)
        .arg("--help")
        .output()
        .expect("the CLI should be runnable");
    assert!(
        String::from_utf8_lossy(&help.stderr).contains(&expected),
        "--help should carry the version"
    );
}

// --- 3. documents that count things ----------------------------------------

/// `(every integration file's stem, how many read the submodule corpus, how
/// many `#[test]`s they hold)`, derived from the tree.
fn integration_tests(root: &Path) -> (Vec<String>, usize, usize) {
    let dir = root.join("crates/uncad/tests");
    let mut stems = Vec::new();
    let mut corpus = 0;
    let mut tests = 0;
    for entry in fs::read_dir(&dir).expect("crates/uncad/tests is readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = read(&path);
        if text.contains("lib/libredwg") {
            corpus += 1;
        }
        // The same count `cargo test -- --list` reports: one per attribute.
        tests += text.lines().filter(|line| line.trim() == "#[test]").count();
        stems.push(
            path.file_stem()
                .expect("a .rs file has a stem")
                .to_string_lossy()
                .into_owned(),
        );
    }
    stems.sort();
    (stems, corpus, tests)
}

/// README, ARCHITECTURE and CAVEATS all state how many integration test files
/// there are; all three have been stale at some point, and ARCHITECTURE's
/// list of file names once contradicted its own total four lines later.
#[test]
fn the_documents_count_the_integration_tests_correctly() {
    let root = repo_root();
    let (stems, corpus, tests) = integration_tests(&root);
    let total = stems.len();

    let readme = read(root.join("README.md"));
    assert!(
        readme.contains(&format!("{corpus} of the {total} integration test files")),
        "README.md should say \"{corpus} of the {total} integration test files\""
    );

    let architecture = read(root.join("docs/ARCHITECTURE.md"));
    assert!(
        architecture.contains(&format!("({total} files: ")),
        "docs/ARCHITECTURE.md's crate layout should say \"({total} files: ...\""
    );
    assert!(
        architecture.contains(&format!("{corpus} of the {total} integration files")),
        "docs/ARCHITECTURE.md should say \"{corpus} of the {total} integration files\""
    );

    // The layout block enumerates the files; every one of them has to be there.
    let listing_start = architecture
        .find("files: acceptance")
        .expect("the crate-layout listing starts at acceptance");
    let listing = &architecture[listing_start..listing_start + 600];
    for stem in &stems {
        assert!(
            listing.contains(stem.as_str()),
            "docs/ARCHITECTURE.md's crate-layout listing omits tests/{stem}.rs"
        );
    }

    let caveats = read(root.join("docs/CAVEATS.md"));
    assert!(
        caveats.contains(&format!(
            "**Real-file tests**: {tests} across the {total} integration files"
        )),
        "docs/CAVEATS.md should say \"**Real-file tests**: {tests} across the {total} \
         integration files\""
    );
    assert!(
        caveats.contains(&format!("{corpus} of the {total} integration files")),
        "docs/CAVEATS.md should say \"{corpus} of the {total} integration files\""
    );
}

/// CAVEATS promises that lint and the Linux job run "on every push". They did
/// not: the workflow was restricted to `main`, so a long-lived release branch
/// with no pull request open got no coverage at all.
#[test]
fn ci_runs_on_every_push_as_the_documents_claim() {
    let root = repo_root();
    let workflow = read(root.join(".github/workflows/ci.yml"));
    let triggers = workflow
        .split("\njobs:")
        .next()
        .expect("the trigger block comes before the jobs");
    assert!(
        triggers.contains("\n  push:\n") && triggers.contains("\n  pull_request:\n"),
        "ci.yml should trigger on push and pull_request"
    );
    assert!(
        !triggers.contains("branches:"),
        "ci.yml restricts its triggers to some branches, but docs/CAVEATS.md says the lint \
         and Linux jobs run on every push"
    );

    let caveats = read(root.join("docs/CAVEATS.md"));
    assert!(
        caveats.contains("on every push and pull request"),
        "docs/CAVEATS.md's Clippy section should still describe the trigger"
    );
}
