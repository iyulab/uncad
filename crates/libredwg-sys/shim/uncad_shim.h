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
 * dwg_read_file() takes a `const char *filename` and opens it
 * with fopen(). On Windows the MSVC C runtime interprets that byte string in
 * the process's ANSI code page, so a UTF-8 path with non-ASCII characters
 * (e.g. a Korean directory name) fails with DWG_ERR_IOERROR even though the
 * file exists. Reading the bytes in Rust (std::fs::read handles Unicode
 * paths on every platform) and decoding from memory sidesteps that, and is
 * also what a server that already holds the file in memory wants.
 *
 * The function mirrors the body of its file-based LibreDWG counterpart
 * (src/dwg.c): `dwg` is cleared except for the log-level bits of its `opts`,
 * the buffer is copied into a Bit_Chain LibreDWG owns for the duration of
 * the decode, and the return value has the same meaning (0 or a DWG_ERROR
 * bit set; >= DWG_ERR_CRITICAL means the decode failed). `buf` is only
 * read, never retained.
 */
int uncad_dwg_read_bytes(const unsigned char *buf, size_t len, Dwg_Data *dwg);

/* --- file header ----------------------------------------------------------
 *
 * Dwg_Data is opaque on the Rust side (see build.rs), and LibreDWG has no
 * public accessor for these header fields, which together with the
 * codepage (uncad_dwg_codepage, below) decide how the drawing's strings sit
 * in memory and which header variables the file states: `version` is the
 * version the drawing is held as, `from_version` the one it was read from,
 * both Dwg_Version_Type enum values (dwg.h). Both return 0 for a NULL
 * `dwg`.
 */
int uncad_dwg_version(const Dwg_Data *dwg);
int uncad_dwg_from_version(const Dwg_Data *dwg);

/* The handle of the CONTINUOUS linetype the header names
 * (`header_vars.LTYPE_CONTINUOUS`): what an entity whose linetype flags
 * say "continuous" (2) uses, since such an entity carries no handle of its
 * own. NULL when the header names none, or for a NULL `dwg`.
 */
Dwg_Object_Ref *uncad_dwg_ltype_continuous(const Dwg_Data *dwg);

/* How many header variables a pre-R13 DWG's file header says its header
 * section holds (`dwg->header.numheader_vars`: 74, 83, ... 205 across the
 * releases), which decides where LibreDWG's pre-R13 header layout stops
 * reading (header_variables_r11.spec). 0 for R13 and later, whose header
 * layout is fixed per version, and for a NULL `dwg`. */
uint16_t uncad_dwg_numheader_vars(const Dwg_Data *dwg);

/* 1 when the decoder read a DWG's Template section -- the one that holds
 * $MEASUREMENT, optional before R2007 and skipped without an error bit when
 * it is missing -- 0 otherwise, for any DXF input and for a NULL `dwg`.
 * Without this a $MEASUREMENT of 0 ("English") cannot be told from one the
 * file never stated. */
int uncad_dwg_template_read(const Dwg_Data *dwg);

/* One leader-line's vertices from a MULTILEADER, flattened into a single
 * malloc'd (x,y,z) array -- see uncad_multileader_get_lines. */
typedef struct uncad_multileader_line
{
  unsigned int root; /* index of the leader node (root) the line belongs to */
  unsigned int num_points;
  double *points; /* x0,y0,z0, x1,y1,z1, ... -- length 3*num_points */
  /* The line's own type (DXF 170: 0 invisible, 1 straight, 2 spline) and
   * override flags (DXF 93: bit 0x1 = the type is the line's own). Stored
   * from R2010; an older drawing reads 0 for both, which overrides nothing. */
  int type;
  unsigned int flags;
} uncad_multileader_line_t;

/* One leader node (root) of a MULTILEADER: where its lines end and its
 * dogleg -- see uncad_multileader_get_roots. A flag of 0 means the file
 * states no such value; the value fields are then 0. */
typedef struct uncad_multileader_root
{
  int has_last_point;     /* DXF 290 */
  double last_point[3];   /* DXF 10 */
  int has_dogleg;         /* DXF 291 */
  double dogleg_vector[3]; /* DXF 11 */
  double dogleg_length;   /* DXF 40 */
} uncad_multileader_root_t;

/* A MULTILEADER's leader nodes, ctx.leaders[], in order: each node's last
 * leader line point and dogleg, with the flags that say whether the file
 * states them. Returns the number of nodes and mallocs *out_roots to that
 * length; 0 with *out_roots NULL for a NULL entity or one with no node.
 * Free with uncad_multileader_free_roots. A line's `root` from
 * uncad_multileader_get_lines indexes this array. */
unsigned int uncad_multileader_get_roots(void *entity,
                                          uncad_multileader_root_t **out_roots);

void uncad_multileader_free_roots(uncad_multileader_root_t *roots);

/* What a MULTILEADER points out: its context data's content, a union the
 * library fills by has_content_txt (DXF 290) or has_content_blk (296).
 * Pointers point into the entity and live as long as it; nothing to free. */
typedef struct uncad_multileader_content
{
  int kind;              /* 0 none, 1 a text (290), 2 a block (296) */
  double scale;          /* ctx.scale_factor, DXF 40 */
  double text_height;    /* ctx.text_height, DXF 41 */
  char *text;            /* DXF 304, as the library stores it */
  void *style;           /* the text style's Dwg_Object_Ref*, DXF 340 */
  void *block;           /* the block's Dwg_Object_Ref*, DXF 341 */
  double normal[3];      /* DXF 11 (text) or 14 (block) */
  double location[3];    /* DXF 12 (text) or 15 (block) */
  double direction[3];   /* DXF 13 (text) */
  double rotation;       /* DXF 42 (text) or 46 (block) */
  double width;          /* DXF 43 (text) */
  int alignment;         /* DXF 171 (text): the attachment point, 1 to 9 */
  double block_scale[3]; /* DXF 16 (block) */
} uncad_multileader_content_t;

/* Fills *out with a MULTILEADER's content; kind 0 for a NULL entity or one
 * that states none. */
void uncad_multileader_get_content(void *entity,
                                   uncad_multileader_content_t *out);

/* Flattens a MULTILEADER entity's ctx.leaders[].lines[] (every leader node
 * can own several lines/splines) into one array of polylines -- every line
 * that has a point, whatever its `type` (stored from R2010 only; how a line
 * is drawn is not whether the file states it). `entity` is the MULTILEADER's
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
 * unset/NULL if entity is NULL or has no leader line with a point (not an
 * error -- MULTILEADER's leader-line data is legitimately absent for some
 * block/text-only content variants). Free with
 * uncad_multileader_free_lines.
 */
unsigned int uncad_multileader_get_lines(void *entity,
                                          uncad_multileader_line_t **out_lines);

void uncad_multileader_free_lines(uncad_multileader_line_t *lines,
                                   unsigned int num_lines);

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
 * -- the Rust side (crates/uncad/src/text.rs) says which strings are which
 * and reads each in its width.
 */
int uncad_dwg_is_wide_string(const Dwg_Data *dwg);

/* One dimension-style variable an entity sets for itself -- a pair of the
 * `DSTYLE` list its extended data carries under the `ACAD` application. */
typedef struct uncad_style_override
{
  uint16_t variable; /* the variable's DIMSTYLE group code (41 DIMASZ, ...) */
  uint8_t kind;      /* 0 real, 1 integer, 2 text, 3 handle */
  uint8_t text_is_wide; /* text is UTF-16 (text_len units), else 8-bit bytes */
  double real;
  int32_t integer;
  uint64_t handle;
  const char *text;  /* points into the drawing; valid while it lives */
  uint32_t text_len; /* bytes, or UTF-16 units when text_is_wide */
} uncad_style_override_t;

/* The `DSTYLE` list in `obj`'s extended data under the `ACAD` application:
 * the string "DSTYLE", "{", then (70 variable, value) pairs up to "}". The
 * value's group says its kind: 40 a real, 70 or 71 an integer, 0 a string,
 * 5 a handle; a pair of any other kind ends the list. The application is
 * matched by its APPID record's name, in either string width.
 *
 * Returns the number of pairs and mallocs *out to that length; 0 with *out
 * NULL when the entity carries no such list. Free with
 * uncad_free_style_overrides. */
unsigned int uncad_entity_style_overrides(const Dwg_Data *dwg,
                                          const Dwg_Object *obj,
                                          uncad_style_override_t **out);

void uncad_free_style_overrides(uncad_style_override_t *list);

#ifdef __cplusplus
}
#endif

#endif /* UNCAD_SHIM_H */
