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
 * is NULL. The same test as the library's own IS_FROM_TU_DWG (an internal
 * macro): `from_version` (the source's version) is R2007 or later and the
 * drawing did not come through an importer (DXF/JSON input keeps strings
 * as given).
 */
int uncad_dwg_is_wide_string(const Dwg_Data *dwg);

#ifdef __cplusplus
}
#endif

#endif /* UNCAD_SHIM_H */
