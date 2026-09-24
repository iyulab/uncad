#!/bin/bash
# The local patches to the vendored LibreDWG, as files.
#
# crates/libredwg-sys/vendor/libredwg/ is upstream LibreDWG at the commit
# vendor/UPSTREAM names, plus local patches (each marked "uncad local patch";
# docs/CAVEATS.md, "Local patches to the vendored LibreDWG"). This script keeps
# those patches as one unified diff per patched file under
# crates/libredwg-sys/patches/, so a re-vendor re-applies them instead of
# someone re-deriving them by hand:
#
#   scripts/libredwg-patches.sh export   write patches/ from the vendored copy
#                                        against the UPSTREAM commit
#   scripts/libredwg-patches.sh check    exit 1 if patches/ no longer matches
#                                        the vendored copy (a patch was edited
#                                        in place and not exported)
#   scripts/libredwg-patches.sh apply    apply patches/ to a freshly synced
#                                        copy -- scripts/sync-libredwg-vendor.sh
#                                        calls this
#
# The original of each file is read from the lib/libredwg submodule's object
# store (git show <commit>:src/...), not its working tree, so the result does
# not depend on how that checkout converts line endings. Patches apply with
# no fuzz: a hunk that no longer matches upstream fails loudly instead of
# landing somewhere nearby.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUBMODULE="$REPO_ROOT/lib/libredwg"
CRATE="$REPO_ROOT/crates/libredwg-sys"
VENDOR_DIR="$CRATE/vendor/libredwg"
PATCH_DIR="$CRATE/patches"
MARKER="uncad local patch"

upstream_commit() {
    sed -n 's/^commit //p' "$CRATE/vendor/UPSTREAM"
}

# Writes one patch per vendored file that carries the marker into $1.
export_to() {
    local out="$1" commit
    commit="$(upstream_commit)"
    mkdir -p "$out"
    rm -f "$out"/*.patch
    (cd "$VENDOR_DIR" && grep -rl --include='*' "$MARKER" .) | sed 's|^\./||' | sort |
        while IFS= read -r rel; do
            local name="${rel//\//_}.patch"
            # diff exits 1 when the files differ, which is the point here.
            diff -u --label "a/$rel" --label "b/$rel" \
                <(git -C "$SUBMODULE" show "$commit:$rel") "$VENDOR_DIR/$rel" \
                >"$out/$name" || [ $? -eq 1 ]
        done
}

case "${1:-}" in
export)
    export_to "$PATCH_DIR"
    echo "Wrote $(ls "$PATCH_DIR"/*.patch | wc -l) patches to $PATCH_DIR against $(upstream_commit)"
    ;;
check)
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    export_to "$tmp"
    if diff -r "$PATCH_DIR" "$tmp" >/dev/null; then
        echo "patches/ matches the vendored copy"
    else
        echo "error: patches/ does not match the vendored copy -- run '$0 export'" >&2
        diff -r "$PATCH_DIR" "$tmp" >&2 || true
        exit 1
    fi
    ;;
apply)
    for p in "$PATCH_DIR"/*.patch; do
        patch -p1 -d "$VENDOR_DIR" --fuzz=0 --forward --batch --no-backup-if-mismatch --quiet <"$p" ||
            { echo "error: $p does not apply to the synced copy -- rebase it on the new upstream" >&2; exit 1; }
    done
    echo "Applied $(ls "$PATCH_DIR"/*.patch | wc -l) patches to $VENDOR_DIR"
    ;;
*)
    echo "usage: $0 export|check|apply" >&2
    exit 2
    ;;
esac
