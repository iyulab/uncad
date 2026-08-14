/* Hand-authored config.h for a native build of the vendored LibreDWG C
   sources (lib/libredwg-web/src), used in place of the autotools-generated
   src/config.h.in output. See PATCHES.md for why this project builds
   LibreDWG via a direct `cc`-crate compile instead of autotools/configure
   (Windows autotools friction documented there).

   Scope mirrors the existing WASM build's flags: write + DXF read/write
   enabled (USE_WRITE defined, DISABLE_DXF left undefined), JSON/GeoJSON,
   Python and language-binding code paths disabled (their .c files are
   simply excluded from the build, not compiled-and-ifdef'd-out; the
   DISABLE_JSON/DISABLE_BINDINGS macros below only matter for the few
   headers that branch on them outside those excluded files).

   Originally written (and empirically validated, header by header, against
   real cl.exe compile errors) for MSVC/Windows x64 only -- the #if
   defined(_WIN32) branch below is that exact original content, untouched.
   The #else (POSIX/glibc, e.g. Ubuntu CI) branch was added afterward and
   validated the same way, against real gcc compile errors in a Docker
   `rust:latest` container (see PATCHES.md). It intentionally changes as
   little as possible from the Windows branch -- most HAVE_* flags mirror
   LibreDWG's own general policy of preferring its portable internal
   fallbacks over platform-specific code paths, so this only diverges where
   a real compile error proved it had to. */

#ifndef CONFIG_H
#define CONFIG_H 1

/* --- feature flags -------------------------------------------------- */
#undef DEBUG_CLASSES
#undef DISABLE_DXF          /* must stay undefined: dxf_read_file() is gated
                                on !DISABLE_DXF && USE_WRITE */
#define DISABLE_JSON 1
#define DISABLE_BINDINGS 1
#define USE_WRITE 1
#undef USE_TRACING
#undef ENABLE_MIMALLOC
#undef ENABLE_SHARED
#define IS_RELEASE 1
#define DXF_PRECISION 16
#define GEOJSON_PRECISION 6

/* --- headers available on MSVC / Windows x64 ------------------------- */
#define HAVE_STDDEF_H 1
#define HAVE_STDINT_H 1
#define HAVE_STDIO_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRING_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_LIMITS_H 1
#define HAVE_FLOAT_H 1
#define HAVE_CTYPE_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_DIRECT_H 1
#define HAVE_MALLOC_H 1
#define HAVE_WCHAR_H 1
#define HAVE_WCTYPE_H 1
#define HAVE_WINSOCK2_H 1

/* POSIX-only headers not available on MSVC */
#undef HAVE_UNISTD_H
#undef HAVE_STRINGS_H
#undef HAVE_GETOPT_H
#undef HAVE_LIBGEN_H
#undef HAVE_DLFCN_H
#undef HAVE_SYS_TIME_H
#undef HAVE_SYS_PARAM_H
#undef HAVE_ENDIAN_H
#undef HAVE_MACHINE_ENDIAN_H
#undef HAVE_SYS_ENDIAN_H
#undef HAVE_SYS_BYTEORDER_H
#undef HAVE_BYTEORDER_H
#undef HAVE_BYTESWAP_H
#undef HAVE_ALLOCA_H
#undef HAVE_VALGRIND_VALGRIND_H
#undef HAVE_ICONV_H
#undef HAVE_LIBPS_PSLIB_H
#undef HAVE_MIMALLOC_OVERRIDE_H
#undef HAVE_PCRE2_H

/* --- functions -------------------------------------------------------- */
#undef HAVE_STRCASECMP       /* -> common.h falls back to its own my_strcasecmp */
#undef AX_STRCASECMP_HEADER
#undef HAVE_STRCASESTR
#define HAVE_STRCHR 1
#define HAVE_STRRCHR 1
#define HAVE_STRSTR 1
#define HAVE_STRTOL 1
#define HAVE_STRTOUL 1
#define HAVE_STRTOLL 1
#define HAVE_STRTOULL 1
#define HAVE_MEMCHR 1
#define HAVE_MEMMOVE 1
#undef HAVE_MEMMEM
#define HAVE_MALLOC 1
#define HAVE_REALLOC 1
#undef HAVE_ALLOCA
#undef C_ALLOCA
#undef HAVE_BASENAME
#undef HAVE_SCANDIR
#undef HAVE_SETENV
#undef HAVE_GETTIMEOFDAY
#undef HAVE_GMTIME_R
#undef HAVE_SINCOS
#define HAVE_SQRT 1
#define HAVE_FLOOR 1
#undef HAVE_GETOPT_LONG
#define HAVE_SSCANF_S 1       /* MSVC CRT extension */
#undef HAVE_ICONV
#undef HAVE_PCRE2_16

/* Windows sprintf/strdup-family already provided by MSVC's CRT under their
   normal (non-underscore) names via <string.h>/<stdio.h> here, no rpl_*
   replacement needed. */
#undef malloc
#undef realloc

/* endian.h-style helpers: none available, and unused since WORDS_BIGENDIAN
   is left undefined below (x64 is little-endian; LibreDWG's own portable
   byte-swap routines handle the rest without these). */
#undef HAVE_BE64TOH
#undef HAVE_HTOBE16
#undef HAVE_HTOBE32
#undef HAVE_HTOBE64
#undef HAVE_HTOLE32
#undef HAVE_HTOLE64
#undef HAVE_LE16TOH
#undef HAVE_LE32TOH
#undef HAVE_LE64TOH

#undef HAVE_WCSCMP
#undef HAVE_WCSCPY
#undef HAVE_WCSLEN
#undef HAVE_WCSNLEN
#undef HAVE_WCSSTR

/* --- compiler / platform characteristics ------------------------------ */
#define HAVE_C99 1
#define HAVE_C11 1
#define HAVE__BOOL 1
#define STDC_HEADERS 1
#undef WORDS_BIGENDIAN            /* x86_64 is little-endian */
#undef HAVE_ALIGNED_ACCESS_REQUIRED
#define SIZEOF_SIZE_T 8            /* 64-bit target */
/* dwg.h gates BITCODE_TU's real type on this: wchar_t is UTF-16 (2 bytes)
   on Windows, so BITCODE_TU becomes native wchar_t* there (HAVE_NATIVE_WCHAR2).
   On POSIX/glibc wchar_t is UTF-32 (4 bytes) -- if this were left at 2 there
   too, dwg.h would still take the HAVE_NATIVE_WCHAR2 branch and BITCODE_TU
   would become a 4-byte wchar_t*, but the actual UCS-2 data (and functions
   like bit_convert_TU) all assume a 2-byte-element buffer (BITCODE_RS*,
   i.e. uint16_t*) -- a real, silent buffer-layout mismatch, not just a
   style issue. Confirmed as a hard compile error under gcc (stricter
   pointer-type checking than MSVC caught the mismatch in bits.c's
   bit_u_expand); getting this right on POSIX makes BITCODE_TU fall through
   to dwg.h's other branch (BITCODE_RS*), matching what the code actually
   expects everywhere. */
#if defined(_WIN32)
#  define SIZEOF_WCHAR_T 2
#else
#  define SIZEOF_WCHAR_T 4
#endif
#undef AC_APPLE_UNIVERSAL_BUILD
#undef HAVE_ASAN
#undef HAVE_WERROR
#undef HAVE_WFORMAT_Y2K

/* GCC/Clang function attributes: MSVC's cl.exe doesn't support
   __attribute__((...)) at all, so all of these stay undefined. */
#undef HAVE_ATTRIBUTE_VISIBILITY_DEFAULT
#undef HAVE_FUNC_ATTRIBUTE_ALIGNED
#undef HAVE_FUNC_ATTRIBUTE_COUNTED_BY
#undef HAVE_FUNC_ATTRIBUTE_FORMAT
#undef HAVE_FUNC_ATTRIBUTE_GNU_FORMAT
#undef HAVE_FUNC_ATTRIBUTE_MALLOC
#undef HAVE_FUNC_ATTRIBUTE_MS_FORMAT
#undef HAVE_FUNC_ATTRIBUTE_NORETURN
#undef HAVE_FUNC_ATTRIBUTE_RETURNS_NONNULL

#undef HAVE_STAT_EMPTY_STRING_BUG
#undef LSTAT_FOLLOWS_SLASHED_SYMLINK
#undef GPERF_VERSION
#undef HAVE_GPERF_SIZE_T
#undef HAVE_PYTHON
#undef ICONV_CONST
#undef STACK_DIRECTION
#undef LT_OBJDIR
#define LIBREDWG_SO_VERSION "0.13"
#undef _UINT32_T
#undef _UINT64_T
#undef __XSI_VISIBLE

/* --- package metadata (only used for --version-style strings) --------- */
#define PACKAGE_NAME "LibreDWG"
#define PACKAGE_TARNAME "libredwg"
#define PACKAGE_VERSION "0.13.3"
#define PACKAGE_STRING "LibreDWG 0.13.3"
#define PACKAGE_BUGREPORT "https://github.com/iyulab-rnd/uncad/issues"
#define PACKAGE_URL "https://www.gnu.org/software/libredwg/"

#ifndef __cplusplus
/* inline is natively supported by MSVC's C compiler; nothing to redefine */
#endif

#endif /* CONFIG_H */
