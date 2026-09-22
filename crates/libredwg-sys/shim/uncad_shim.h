#ifndef UNCAD_SHIM_H
#define UNCAD_SHIM_H

#include <stddef.h>

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
 * public accessor for these three header fields, which drive how strings
 * must be decoded (see uncad_tv_to_utf8): `version`/`from_version` are the
 * Dwg_Version_Type enum values (dwg.h) and `codepage` the Dwg_Codepage enum
 * (src/codepages.h: 0 = UTF-8, 30 = ANSI_1252, 40 = ANSI_949, ...). All
 * three return 0 for a NULL `dwg`.
 */
int uncad_dwg_version(const Dwg_Data *dwg);
int uncad_dwg_from_version(const Dwg_Data *dwg);
unsigned int uncad_dwg_codepage(const Dwg_Data *dwg);

/* LibreDWG's own IS_FROM_TU_DWG(dwg) rule (src/bits.h): 1 when the strings
 * dynapi hands out are already UTF-8 (converted from the R2007+ UTF-16
 * storage), 0 when they are the file's raw 8-bit code-page bytes -- which
 * is the case for every pre-R2007 DWG *and* for every DXF input, whatever
 * its version. 0 for a NULL `dwg`.
 */
int uncad_dwg_is_tu(const Dwg_Data *dwg);

/* The DXF name of a Dwg_Codepage value ("ANSI_1252", "ANSI_949", "UTF-8",
 * ...) from LibreDWG's own table (dwg_codepage_dxfstr in src/codepages.h,
 * which is not a public header), or NULL for a value it has no name for.
 * The string is static; do not free it.
 */
const char *uncad_codepage_name(unsigned int codepage);

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
 *     memory is UTF-16 -- in_dxf.c stores it as TU, but IS_FROM_TU_DWG
 *     excludes DXF input so dynapi never converts it -- and it goes through
 *     bit_convert_TU() like a DWG's would;
 *   - a CP_UTF8 code page only has its `\U+XXXX` / `\M+nXXXX` escapes
 *     expanded (bit_TV_to_utf8);
 *   - CP_UNDEFINED, CP_UTF16 and any value outside LibreDWG's tables fall
 *     back to ANSI_1252, LibreDWG's own default (an unchecked value would
 *     index past its tables);
 *   - otherwise the string is transcoded with LibreDWG's code-page tables,
 *     an unmappable character becoming U+FFFD, and the escapes expanded.
 *
 * The result is always a fresh heap buffer (or NULL only for a NULL `s` or
 * out of memory) -- bit_TV_to_utf8() may return its input pointer unchanged,
 * which is copied here so the caller never has to guess who owns what.
 * Free with uncad_free_string.
 */
char *uncad_tv_to_utf8(const Dwg_Data *dwg, const char *s);

/* Same as uncad_tv_to_utf8, but finds the owning Dwg_Data through the
 * entity/object struct pointer (what uncad_object_entity_ptr /
 * uncad_object_object_ptr returned -- the same pointer dynapi takes), via
 * dwg_obj_generic_to_object(). If that lookup fails the string is copied
 * without conversion.
 */
char *uncad_entity_tv_to_utf8(const void *entity, const char *s);

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

#ifdef __cplusplus
}
#endif

#endif /* UNCAD_SHIM_H */
