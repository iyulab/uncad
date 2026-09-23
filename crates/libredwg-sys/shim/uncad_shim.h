#ifndef UNCAD_SHIM_H
#define UNCAD_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* No write shims live here any more: this workspace reads DWG/DXF and
 * exports the parsed model (JSON/SVG/PNG), and the former uncad_write_dxf /
 * uncad_write_dxf_file helpers went with the write API (see CHANGELOG.md).
 */

/* Returns the type-specific entity struct pointer (e.g. Dwg_Entity_LINE*,
 * as a void*) for `obj`, or NULL if `obj` isn't an entity or has no data.
 *
 * This exists because Dwg_Object/Dwg_Object_Entity are bound as opaque
 * blobs on the Rust side (see build.rs: their real definitions pull in a
 * ~90-type union that broke bindgen's struct codegen), so Rust cannot do
 * the C idiom `obj->tio.entity->tio.LINE` itself. The union's members are
 * all same-representation pointers (LibreDWG's own dynapi relies on this:
 * dwg_dynapi_entity_value() is handed a `void *entity` and a type-name
 * string, not a specifically-typed pointer), so reading through any single
 * fixed member name yields the correct address for every entity type --
 * this shim always reads `.tio.UNKNOWN_ENT`, which is defined for every
 * DWG version this project supports. Callers pass the result to
 * dwg_dynapi_entity_value()/dwg_dynapi_entity_field() together with the
 * entity's own dxfname (from dwg_object_get_dxfname), never dereferencing
 * it directly as a Rust struct.
 */
void *uncad_object_entity_ptr(Dwg_Object *obj);

/* Same idea for non-entity OBJECT types (LAYER, BLOCK_RECORD, ...): returns
 * the type-specific struct pointer via obj->tio.object->tio.UNKNOWN_OBJ.
 */
void *uncad_object_object_ptr(Dwg_Object *obj);

/* --- reading from memory ------------------------------------------------
 *
 * dwg_read_file()/dxf_read_file() take a `const char *filename` and open it
 * with fopen(). On Windows the MSVC C runtime interprets that byte string in
 * the process's ANSI code page, so a UTF-8 path with non-ASCII characters
 * (e.g. a Korean directory name) fails with DWG_ERR_IOERROR even though the
 * file exists. Reading the bytes in Rust (std::fs::read handles Unicode
 * paths on every platform) and decoding from memory sidesteps that, and is
 * also what a server that already holds the file in memory wants.
 *
 * Both functions mirror the body of their file-based LibreDWG counterpart
 * (src/dwg.c): `dwg` is cleared except for the log-level bits of its `opts`
 * (and, for DXF, its `header.version`), the buffer is copied into a
 * Bit_Chain LibreDWG owns for the duration of the decode, and the return
 * value has the same meaning (0 or a DWG_ERROR bit set; >= DWG_ERR_CRITICAL
 * means the decode failed). `buf` is only read, never retained.
 */
int uncad_dwg_read_bytes(const unsigned char *buf, size_t len, Dwg_Data *dwg);
int uncad_dxf_read_bytes(const unsigned char *buf, size_t len, Dwg_Data *dwg);

/* --- file header ----------------------------------------------------------
 *
 * Dwg_Data is opaque on the Rust side (see build.rs), and LibreDWG has no
 * public accessor for these two header fields, which together with the
 * codepage (uncad_dwg_codepage, below) drive how strings must be decoded
 * (see uncad_tv_to_utf8): `version` is the version the drawing is held as,
 * `from_version` the one it was read from, both Dwg_Version_Type enum values
 * (dwg.h). Both return 0 for a NULL `dwg`.
 */
int uncad_dwg_version(const Dwg_Data *dwg);
int uncad_dwg_from_version(const Dwg_Data *dwg);

/* 1 when the data came from DXF text (LibreDWG's DWG_OPTS_INDXF flag on
 * `dwg->opts`), 0 for a DWG or a NULL `dwg`. Some fields mean something
 * else on that path (a LAYER's plot flag, for one). */
int uncad_dwg_from_dxf(const Dwg_Data *dwg);

/* The DXF name of a Dwg_Codepage value -- as uncad_dwg_codepage returns it
 * -- ("ANSI_1252", "ANSI_949", "UTF-8", ...) from LibreDWG's own table
 * (dwg_codepage_dxfstr in src/codepages.h, which is not a public header), or
 * NULL for a value it has no name for. The string is static; do not free it.
 */
const char *uncad_codepage_name(uint16_t codepage);

/* --- strings ---------------------------------------------------------------
 *
 * Converts one string as returned by dwg_dynapi_*_utf8text() /
 * dwg_dynapi_handle_name() with `isnew == 0` (i.e. a raw pointer into the
 * parsed Dwg_Data, in the file's own code page) to UTF-8.
 *
 * This exists because LibreDWG's dynapi only transcodes the R2007+ UTF-16
 * path; for older files it returns the code-page bytes unchanged, and the
 * Rust side used to run those through a lossy UTF-8 decode, turning every
 * non-ASCII character (Korean text in R2000/R2004 drawings, the degree and
 * plus-minus signs in dimension text) into U+FFFD. LibreDWG does ship the
 * converter -- bit_TV_to_utf8() in src/bits.c, with built-in CP949/CP936/
 * CP1252/... tables, used only by its own DXF writer -- so this wraps it:
 *
 *   - when IS_FROM_TU_DWG(dwg) the input is already UTF-8 and is copied;
 *   - for an R2007+ DXF (DWG_OPTS_IN set, version >= R_2007) the string in
 *     memory is UTF-16 -- in_dxf.c stores its T fields as TU, but
 *     IS_FROM_TU_DWG excludes DXF input so dynapi never converts them --
 *     and it goes through bit_convert_TU() like a DWG's would. This is the
 *     one rule that is not true of every field: see uncad_bytes_to_utf8
 *     for the strings in_dxf.c keeps 8-bit whatever the version;
 *   - an R2007+ DXF's 8-bit strings are UTF-8 (the DXF's own encoding from
 *     AC1021 on, and what in_dxf.c's TU conversion assumes) and only have
 *     their `\U+XXXX` / `\M+nXXXX` escapes expanded (bit_TV_to_utf8), as
 *     does a CP_UTF8 code page;
 *   - CP_UNDEFINED, CP_UTF16 and any value outside LibreDWG's tables fall
 *     back to ANSI_1252, LibreDWG's own default (an unchecked value would
 *     index past its tables);
 *   - otherwise the string is transcoded with LibreDWG's code-page tables,
 *     an unmappable character becoming U+FFFD, and the escapes expanded.
 *     The double-byte pages follow the file's bytes rather than every
 *     quirk of LibreDWG's tables: the DOS-era BIG5 (24) and GB2312 (31)
 *     pair only bytes >= 0x80 (dwg_codepage_is_twobyte pairs ASCII too,
 *     which garbled every table name and emptied the drawing), CP932 (22)
 *     is decoded as the double-byte Shift-JIS it is, and GB2312's EUC-CN
 *     bytes are masked to the 7-bit form its table is indexed by.
 *
 * The result is always a fresh heap buffer (or NULL only for a NULL `s` or
 * out of memory) -- bit_TV_to_utf8() may return its input pointer unchanged,
 * which is copied here so the caller never has to guess who owns what.
 * Free with uncad_free_string.
 */
char *uncad_tv_to_utf8(const Dwg_Data *dwg, const char *s);

/* uncad_tv_to_utf8 for a string that is 8-bit in memory whatever the file's
 * version: the bytes are decoded as the file's own text encoding (UTF-8 for
 * an R2007+ DXF, the header code page otherwise) with the escapes expanded,
 * and bit_convert_TU() is never run over them.
 *
 * This exists because LibreDWG's DXF reader does not store every string the
 * way the "R2007+ means UTF-16" rule says. dxf_header_read() runs while
 * `header.version` is still R_INVALID (dxf_fixup_header sets it afterwards,
 * and the $ACADVER branch only sets it for R13..R2000), so every HEADER text
 * variable ($DIMPOST, ...) is a plain malloc'd copy of the file's bytes;
 * and MTEXT's group 1/3 text chunks are strdup'd/realloc'd together with no
 * version branch at all (in_dxf.c, "MTEXT text > 250 chars"). Handing
 * either to bit_convert_TU() walks 16-bit units past the end of the
 * allocation until a zero pair happens to occur, so the header's DIMPOST
 * and every MTEXT of an R2007+ DXF came back as CJK-looking garbage with
 * neighbouring heap bytes in it. Same result type and ownership as
 * uncad_tv_to_utf8; a `dwg` that IS_FROM_TU_DWG (only a TF field can get
 * here from one) is copied unchanged.
 */
char *uncad_bytes_to_utf8(const Dwg_Data *dwg, const char *s);

/* Same as uncad_tv_to_utf8, but finds the owning Dwg_Data through the
 * entity/object struct pointer (what uncad_object_entity_ptr /
 * uncad_object_object_ptr returned -- the same pointer dynapi takes), via
 * dwg_obj_generic_to_object(). If that lookup fails the string is copied
 * without conversion.
 */
char *uncad_entity_tv_to_utf8(const void *entity, const char *s);

/* uncad_bytes_to_utf8 with the same entity-pointer lookup as
 * uncad_entity_tv_to_utf8: for the fields in_dxf.c stores 8-bit whatever
 * the version (MTEXT.text). */
char *uncad_entity_bytes_to_utf8(const void *entity, const char *s);

/* Any string pointer read straight out of `dwg`'s structures (a T/TV field
 * of an embedded struct dynapi cannot reach through utf8text, say): the
 * UTF-16 a R2007+ DWG stores is converted with bit_convert_TU, anything
 * else goes through uncad_tv_to_utf8. NULL in, NULL out. Free with
 * uncad_free_string. */
char *uncad_dwg_string_to_utf8(const Dwg_Data *dwg, const char *s);

void uncad_free_string(char *s);

/* One leader-line's vertices from a MULTILEADER, flattened into a single
 * malloc'd (x,y,z) array -- see uncad_multileader_get_lines. */
typedef struct uncad_multileader_line
{
  unsigned int num_points;
  double *points; /* x0,y0,z0, x1,y1,z1, ... -- length 3*num_points */
} uncad_multileader_line_t;

/* Flattens a MULTILEADER entity's ctx.leaders[].lines[] (every leader node
 * can own several lines/splines) into one array of polylines, skipping
 * type==0 ("invisible leader") lines. `entity` is the MULTILEADER's
 * type-specific struct pointer, i.e. what uncad_object_entity_ptr returned
 * for this object (Dwg_Entity_MULTILEADER*, passed as void* for the same
 * reason uncad_object_entity_ptr itself returns void*).
 *
 * This exists because MULTILEADER's leader geometry lives 3 struct levels
 * deep (entity.ctx.leaders[i].lines[j].points[k]) through a chain of nested
 * structs/arrays that dynapi's flat `dwg_dynapi_entity_value` can't reach
 * (it only exposes top-level entity fields by name) and that bindgen can't
 * safely bind directly on the Rust side (Dwg_MLEADER_AnnotContext and its
 * nested LEADER_Node/LEADER_Line types are exactly the kind of
 * pointer-heavy nested struct that already broke bindgen's codegen for
 * Dwg_HATCH_Path -- see build.rs). Doing the walk here, where dwg.h's real
 * struct layouts are visible, sidesteps both problems at once: Rust only
 * ever sees this one flat, stable-shape output type.
 *
 * Returns the number of lines and mallocs *out_lines to that length (each
 * line's own ->points also malloc'd separately); 0 with *out_lines
 * unset/NULL if entity is NULL or has no visible leader lines (not an
 * error -- MULTILEADER's leader-line data is legitimately absent for some
 * block/text-only content variants). Free with
 * uncad_multileader_free_lines.
 */
unsigned int uncad_multileader_get_lines(void *entity,
                                          uncad_multileader_line_t **out_lines);

void uncad_multileader_free_lines(uncad_multileader_line_t *lines,
                                   unsigned int num_lines);

/* Converts a 3DSOLID/REGION/BODY entity's ACIS payload from SAB ("ACIS
 * BinaryFile", `version == 2`) to SAT v1 text and returns it as one
 * malloc'd, NUL-terminated buffer (its byte length in *out_len), or NULL
 * if `entity` is NULL, the payload is not SAB, or the conversion fails.
 * Free with uncad_free_sat_text. `entity` is the type-specific struct
 * pointer (Dwg_Entity_3DSOLID*, passed as const void* for the same reason
 * uncad_object_entity_ptr returns void*).
 *
 * This exists because LibreDWG's own dwg_convert_SAB_to_SAT1 converts *in
 * place*: it rewrites version/num_blocks/block_size/encr_sat_data (and
 * sab_size/acis_empty/_dxf_sab_converted) on the entity it is handed, while
 * leaving acis_data as the original SAB bytes. uncad reads every solid
 * twice during parse() (once for the top-level entity list, once for the
 * owning block record), so a first in-place conversion left the second
 * read looking at a `version == 1` entity whose acis_data is still binary
 * SAB -- which it then parsed as SAT text and got nothing out of. (Before
 * the write API was removed, the same mutation also corrupted every later
 * DXF/DWG write of the drawing.) This shim runs the conversion on a
 * shallow stack copy of the entity (with the three output pointers cleared
 * so nothing aliases the original), copies the SAT text out, frees what
 * the conversion allocated on the copy, and leaves the entity
 * byte-for-byte untouched, so parse() has no side effect on the Dwg_Data.
 * The copy's `parent` still points at the real Dwg_Object_Entity, which is
 * all the conversion reads through it (the drawing's header.version, for
 * the target ACIS version).
 */
char *uncad_3dsolid_sab_to_sat_text(const void *entity, size_t *out_len);

void uncad_free_sat_text(char *text);

/* 1 when the drawing was read from a pre-R13 source (DWG R1.4 .. R12, or a
 * DXF stamped so), 0 otherwise or when `dwg` is NULL.
 *
 * This exists because Dwg_Data is bound as an opaque blob on the Rust side
 * (see build.rs), so `dwg->header.from_version` cannot be read there. It
 * reads `from_version` -- the field the library's own table lookup
 * (dwg_handle_name) gates on, set for DWG and DXF reads alike -- and falls
 * back to `version` only when that is unset. Pre-R13 drawings address their
 * tables by index rather than by handle, which is what the Rust side needs
 * to know to resolve a reference at all.
 */
int uncad_dwg_is_pre_r13(const Dwg_Data *dwg);

/* 1 when `dwg->header.version` is in R13 .. R2000 inclusive, 0 otherwise or
 * when `dwg` is NULL.
 *
 * That is the version band in which the library links a block's entities
 * (and an INSERT's attributes) as a prev/next chain rather than an owned
 * array, and in which its own chain walkers have two gaps the Rust side
 * has to step around: `get_next_owned_entity` skips ATTDEF as if it were a
 * sub-entity, and `get_first_owned_subentity` reads `first_attrib->obj`
 * without resolving it, which is NULL after a DXF import. Reads `version`
 * (the target version), the same field those walkers branch on.
 */
int uncad_dwg_is_r13_to_r2000(const Dwg_Data *dwg);

/* 1 when the drawing was read from an R2010-or-later source, 0 otherwise or
 * when `dwg` is NULL. Reads `from_version`, falling back to `version`, like
 * uncad_dwg_is_pre_r13.
 *
 * The library's record layout for LEADER stops reading the annotation
 * offset (`endptproj`) after R2007 while files from R2010 on still carry
 * it, so every field it reads after that point in such a file comes from
 * the wrong bits -- among them the arrowhead flag. The Rust side needs to
 * know when that is the case, to say it cannot read the flag rather than
 * report the misread value.
 */
int uncad_dwg_is_r2010_or_later(const Dwg_Data *dwg);

/*
 * Nonzero when the drawing was read from an R2013-or-later DWG (the file's
 * own version, as `uncad_dwg_is_r2010_or_later`). From that version on a
 * SPLINE record stores `splineflags`, whose bit 4 says a fit-point spline
 * is closed; before it the library fills `splineflags` in itself from the
 * record's form, so the bit is not the file's.
 */
int uncad_dwg_is_r2013_or_later(const Dwg_Data *dwg);

/* `dwg->header.codepage`: the codepage the drawing's 8-bit strings (every
 * string before R2007) are in, as the Dwg_Codepage number -- read from the
 * DWG header, or from `$DWGCODEPAGE` by the DXF importer (which defaults to
 * ANSI_1252 when the variable is absent). 0 when `dwg` is NULL.
 *
 * This exists because Dwg_Data is opaque on the Rust side (see build.rs),
 * and the library's own text accessors do not apply the codepage to such
 * strings: they return the bytes as stored. The Rust side decodes them
 * itself through the library's codepage tables (codepages.h).
 */
uint16_t uncad_dwg_codepage(const Dwg_Data *dwg);

/* 1 when the drawing's strings are wide (UTF-16, R2007 and later), which the
 * library converts to UTF-8 in its text accessors, 0 otherwise or when `dwg`
 * is NULL. This is the library's own IS_FROM_TU_DWG (src/bits.h, an internal
 * macro): `from_version` (the source's version) is R2007 or later and the
 * drawing did not come through an importer (DXF/JSON input). For a DWG, 0
 * means the strings are the file's raw 8-bit code-page bytes. For a DXF it
 * is 0 whatever the version, although an R2007+ DXF holds most of its T
 * fields as UTF-16 in memory, which the accessors then hand out unconverted
 * -- uncad_tv_to_utf8 and uncad_bytes_to_utf8 above say which strings are
 * which.
 */
int uncad_dwg_is_wide_string(const Dwg_Data *dwg);

#ifdef __cplusplus
}
#endif

#endif /* UNCAD_SHIM_H */
