#!/bin/bash
# Refreshes crates/libredwg-sys/vendor/libredwg/ from the lib/libredwg git
# submodule, after `git submodule update --remote` (or any other submodule
# pointer move).
#
# Why this exists: crates.io only packages files inside a crate's own
# directory, and a downstream consumer building from crates.io has no
# lib/libredwg submodule (no .git checkout) to fall back to. So build.rs
# compiles from this vendored copy, not the submodule directly -- see
# docs/ARCHITECTURE.md's "Build" section.
#
# This script re-derives the exact file set by tracing the real #include
# graph from the .c files build.rs compiles (LIBREDWG_SOURCES in build.rs),
# rather than copying all of lib/libredwg/src (which also has the three
# JSON/GeoJSON .c files this crate deliberately never builds -- see
# EXCLUDED_JSON_SOURCES in build.rs -- plus test/example content). If
# upstream adds a new #include this list doesn't yet know about, the next
# `cargo build` fails loudly with a file-not-found from the C compiler --
# add the missing file's search dir below and re-run. build.rs registers
# the whole vendor/libredwg directory with cargo:rerun-if-changed, so that
# next build really does recompile from the refreshed copy (no `cargo
# clean` needed).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUBMODULE_SRC="$REPO_ROOT/lib/libredwg/src"
SUBMODULE_INCLUDE="$REPO_ROOT/lib/libredwg/include"
SUBMODULE_PROGRAMS="$REPO_ROOT/lib/libredwg/programs"
VENDOR_DIR="$REPO_ROOT/crates/libredwg-sys/vendor/libredwg"

if [ ! -d "$SUBMODULE_SRC" ]; then
    echo "error: $SUBMODULE_SRC not found -- run 'git submodule update --init' first" >&2
    exit 1
fi

# Same 24 .c files as LIBREDWG_SOURCES in crates/libredwg-sys/build.rs.
COMPILED_SOURCES=(
    bits.c classes.c codepages.c common.c decode.c decode2.c decode_r11.c
    decode_r2007.c dwg.c dwg_api.c dxfclasses.c dynapi.c encode.c encode2.c
    free.c geom.c hash.c in_dxf.c logging.c objects.c out_dxf.c out_dxfb.c
    print.c reedsolomon.c
)

SEARCHDIRS=("$SUBMODULE_SRC" "$SUBMODULE_INCLUDE" "$SUBMODULE_SRC/codepages" "$SUBMODULE_PROGRAMS")

declare -A VISITED
QUEUE=()

resolve() {
    local inc="$1"
    for d in "${SEARCHDIRS[@]}"; do
        if [ -f "$d/$inc" ]; then
            echo "$d/$inc"
            return 0
        fi
    done
    return 1
}

for s in "${COMPILED_SOURCES[@]}"; do
    QUEUE+=("$SUBMODULE_SRC/$s")
done

while [ ${#QUEUE[@]} -gt 0 ]; do
    cur="${QUEUE[0]}"
    QUEUE=("${QUEUE[@]:1}")
    [ -n "${VISITED[$cur]:-}" ] && continue
    if [ ! -f "$cur" ]; then
        echo "warning: $cur referenced but missing" >&2
        continue
    fi
    VISITED[$cur]=1
    incs=$(grep -oE '#include[[:space:]]*"[^"]+"' "$cur" | sed -E 's/#include[[:space:]]*"([^"]+)"/\1/' || true)
    while IFS= read -r inc; do
        [ -z "$inc" ] && continue
        resolved=$(resolve "$inc" || true)
        if [ -z "$resolved" ]; then
            # e.g. signature.spec: only reachable from `#if 0` dead code in
            # decode.c/decode_r2007.c/encode.c and doesn't exist upstream at
            # all -- expected to be unresolved, not a bug in this script.
            continue
        fi
        [ -z "${VISITED[$resolved]:-}" ] && QUEUE+=("$resolved")
    done <<< "$incs"
done

rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"
for f in "${!VISITED[@]}"; do
    rel="${f#"$REPO_ROOT"/lib/libredwg/}"
    dest="$VENDOR_DIR/$rel"
    mkdir -p "$(dirname "$dest")"
    cp "$f" "$dest"
done
cp "$REPO_ROOT/lib/libredwg/COPYING" "$VENDOR_DIR/COPYING"

echo "Synced ${#VISITED[@]} files into $VENDOR_DIR"
echo "Next: cargo build -p libredwg-sys, and if LIBREDWG_SOURCES in build.rs needs updating, update it now."
