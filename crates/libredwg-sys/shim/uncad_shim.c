#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "dwg.h"
#include "dwg_api.h"
/* Private LibreDWG headers (vendor/libredwg/src, on the include path in
   build.rs): bits.h for Bit_Chain and IS_FROM_TU_DWG; decode.h for
   dwg_decode; in_dxf.h for dwg_read_dxf/dwg_read_dxfb; logging.h for the
   `loglevel` global the file-based readers set from dwg->opts. */
#include "bits.h"
#include "decode.h"
#include "logging.h" /* must precede in_dxf.h (logging.h enforces it) */
#include "in_dxf.h"
#include "uncad_shim.h"

void *
uncad_object_entity_ptr (Dwg_Object *obj)
{
  /* dwg_object_to_entity() dereferences obj->supertype with no null check
     of its own -- every current Rust call site already null-checks obj
     before calling this, but this shim's own contract shouldn't rely on
     callers upholding that silently; fail closed instead of crashing. */
  if (!obj)
    return NULL;
  int error = 0;
  Dwg_Object_Entity *ent = dwg_object_to_entity (obj, &error);
  if (!ent || error)
    return NULL;
  return (void *)ent->tio.UNKNOWN_ENT;
}

void *
uncad_object_object_ptr (Dwg_Object *obj)
{
  /* Same reasoning as uncad_object_entity_ptr above. */
  if (!obj)
    return NULL;
  int error = 0;
  Dwg_Object_Object *o = dwg_object_to_object (obj, &error);
  if (!o || error)
    return NULL;
  return (void *)o->tio.UNKNOWN_OBJ;
}

/* --- reading from memory ------------------------------------------------ */

/* Copy of the file-static dwg_fixup_viewport_ids() in src/dwg.c, which
   dwg_read_file() runs after a successful decode and which this shim's
   memory-based reader must therefore run too: it gives every paper-space
   VIEWPORT its on_off/id the way AutoCAD numbers them, with the layout's
   own overall viewport (entmode 0) as id 0 / off. */
static void
fixup_viewport_ids (Dwg_Data *restrict dwg)
{
  BITCODE_BL i;
  BITCODE_RS last_id = 0;
  for (i = 0; i < dwg->num_objects; i++)
    {
      Dwg_Object *obj = &dwg->object[i];
      if (obj->supertype == DWG_SUPERTYPE_ENTITY
          && obj->fixedtype == DWG_TYPE_VIEWPORT)
        {
          Dwg_Entity_VIEWPORT *_obj = obj->tio.entity->tio.VIEWPORT;
          if (obj->tio.entity->entmode == 0)
            {
              _obj->on_off = 0;
              _obj->id = 0;
              last_id = 0;
            }
          else
            {
              _obj->on_off = 1;
              last_id++;
              _obj->id = last_id;
            }
        }
    }
}

int
uncad_dwg_read_bytes (const unsigned char *buf, size_t len, Dwg_Data *dwg)
{
  Bit_Chain dat;
  int error;
  unsigned int opts;

  if (!dwg)
    return DWG_ERR_INVALIDDWG;
  /* Same reset as dwg_read_file(): everything but the log-level bits. */
  opts = dwg->opts & DWG_OPTS_LOGLEVEL;
  loglevel = opts;
  memset (dwg, 0, sizeof (Dwg_Data));
  dwg->opts = opts;

  if (!buf || len < 6)
    return DWG_ERR_INVALIDDWG;

  /* dat_read_file() leaves the chain NUL-terminated (size + 1 bytes); a
     little extra zeroed slack costs nothing and protects the bit reader's
     look-ahead on a truncated file. */
  memset (&dat, 0, sizeof (Bit_Chain));
  dat.chain = (unsigned char *)calloc (1, len + 16);
  if (!dat.chain)
    return DWG_ERR_OUTOFMEM;
  memcpy (dat.chain, buf, len);
  dat.size = len;
  dat.opts = dwg->opts;

  error = dwg_decode (&dat, dwg);
  free (dat.chain);
  if (error >= DWG_ERR_CRITICAL)
    return error;

  fixup_viewport_ids (dwg);
  return error;
}

int
uncad_dxf_read_bytes (const unsigned char *buf, size_t len, Dwg_Data *dwg)
{
  Bit_Chain dat;
  int error;
  unsigned int opts;
  Dwg_Version_Type version;

  if (!dwg)
    return DWG_ERR_INVALIDDWG;
  /* Same reset as dxf_read_file(): keep the log level and a caller-preset
     target version. */
  opts = dwg->opts & DWG_OPTS_LOGLEVEL;
  loglevel = opts;
  version = dwg->header.version;
  memset (dwg, 0, sizeof (Dwg_Data));
  dwg->opts = opts | DWG_OPTS_INDXF;
  dwg->header.version = version;

  /* "0\nSECTION\n2\nENTITIES\n0\nENDSEC\n" is the smallest DXF the reader
     accepts; dxf_read_file() rejects anything shorter the same way. */
  if (!buf || len < 31)
    return DWG_ERR_IOERROR;

  memset (&dat, 0, sizeof (Bit_Chain));
  dat.chain = (unsigned char *)calloc (1, len + 2);
  if (!dat.chain)
    return DWG_ERR_OUTOFMEM;
  memcpy (dat.chain, buf, len);
  dat.size = len;
  dat.from_version = dwg->header.from_version;
  dat.version = dwg->header.version;
  dat.opts = dwg->opts;

  /* Terminate the buffer for the strtol()/sscanf() readers -- reproduced
     verbatim from dxf_read_file(), including its quirk of writing the NUL
     over the newline it just appended (the calloc'd slack keeps the result
     NUL-terminated either way). */
  if (dat.chain[len - 1] != '\n')
    {
      dat.chain[len] = '\n';
      dat.size++;
    }
  dat.chain[len] = '\0';

  /* Fail on DWG */
  if (!memcmp (dat.chain, "AC10", 4) || !memcmp (dat.chain, "AC1.", 4)
      || !memcmp (dat.chain, "AC2.10", 4) || !memcmp (dat.chain, "MC0.0", 4))
    {
      free (dat.chain);
      return DWG_ERR_INVALIDDWG;
    }
  /* See if binary or ascii */
  if (!memcmp (dat.chain, "AutoCAD Binary DXF",
               sizeof ("AutoCAD Binary DXF") - 1))
    {
      dat.byte = 22;
      error = dwg_read_dxfb (&dat, dwg);
    }
  else
    error = dwg_read_dxf (&dat, dwg);

  dwg->opts |= (DWG_OPTS_INDXF | opts);
  free (dat.chain);
  if (error >= DWG_ERR_CRITICAL)
    return error;
  return 0;
}

/* --- file header ---------------------------------------------------------- */

int
uncad_dwg_version (const Dwg_Data *dwg)
{
  return dwg ? (int)dwg->header.version : 0;
}

int
uncad_dwg_from_version (const Dwg_Data *dwg)
{
  return dwg ? (int)dwg->header.from_version : 0;
}

int
uncad_dwg_from_dxf (const Dwg_Data *dwg)
{
  return (dwg && (dwg->opts & DWG_OPTS_INDXF)) ? 1 : 0;
}

Dwg_Object_Ref *
uncad_dwg_ltype_continuous (const Dwg_Data *dwg)
{
  return dwg ? dwg->header_vars.LTYPE_CONTINUOUS : NULL;
}

uint16_t
uncad_dwg_numheader_vars (const Dwg_Data *dwg)
{
  return dwg ? (uint16_t)dwg->header.numheader_vars : 0;
}

int
uncad_dwg_template_read (const Dwg_Data *dwg)
{
  /* template.spec reads `description` first, and bit_read_T16/TU16 allocate
     it for every length, zero included: non-NULL exactly when the decoder
     found and read the section. The DXF importer copies $MEASUREMENT into
     the Template without it. */
  return (dwg && dwg->Template.description) ? 1 : 0;
}

/* --- MULTILEADER ------------------------------------------------------------ */

unsigned int
uncad_multileader_get_lines (void *entity, uncad_multileader_line_t **out_lines)
{
  *out_lines = NULL;
  if (!entity)
    return 0;
  Dwg_Entity_MULTILEADER *mleader = (Dwg_Entity_MULTILEADER *)entity;
  Dwg_MLEADER_AnnotContext *ctx = &mleader->ctx;

  /* First pass: count the lines with at least one point, across every
   * leader node, so out_lines can be sized exactly. A line's `type` is not
   * a filter: it is stored from R2010 only (earlier it reads 0, which would
   * drop every line of an older drawing), and "invisible" describes how the
   * line is drawn, not whether the file states it. */
  unsigned int total_lines = 0;
  for (BITCODE_BL i = 0; i < ctx->num_leaders; i++)
    {
      Dwg_LEADER_Node *node = &ctx->leaders[i];
      for (BITCODE_BL j = 0; j < node->num_lines; j++)
        {
          Dwg_LEADER_Line *line = &node->lines[j];
          if (line->num_points > 0 && line->points)
            total_lines++;
        }
    }
  if (total_lines == 0)
    return 0;

  uncad_multileader_line_t *lines
      = calloc (total_lines, sizeof (uncad_multileader_line_t));
  if (!lines)
    return 0;

  unsigned int k = 0;
  for (BITCODE_BL i = 0; i < ctx->num_leaders; i++)
    {
      Dwg_LEADER_Node *node = &ctx->leaders[i];
      for (BITCODE_BL j = 0; j < node->num_lines; j++)
        {
          Dwg_LEADER_Line *line = &node->lines[j];
          if (line->num_points == 0 || !line->points)
            continue;
          double *pts = malloc (sizeof (double) * 3 * line->num_points);
          if (!pts)
            continue;
          for (BITCODE_BL p = 0; p < line->num_points; p++)
            {
              pts[p * 3 + 0] = line->points[p].x;
              pts[p * 3 + 1] = line->points[p].y;
              pts[p * 3 + 2] = line->points[p].z;
            }
          lines[k].num_points = line->num_points;
          lines[k].points = pts;
          k++;
        }
    }

  *out_lines = lines;
  return k;
}

void
uncad_multileader_free_lines (uncad_multileader_line_t *lines, unsigned int num_lines)
{
  if (!lines)
    return;
  for (unsigned int i = 0; i < num_lines; i++)
    free (lines[i].points);
  free (lines);
}

/* --- 3DSOLID ---------------------------------------------------------------- */

char *
uncad_3dsolid_sab_to_sat_text (const void *entity, size_t *out_len)
{
  if (out_len)
    *out_len = 0;
  if (!entity)
    return NULL;

  /* Shallow copy. dwg_convert_SAB_to_SAT1 only *reads* acis_data/sab_size
     (and header.version through ->parent) from the struct it is handed; it
     *writes* version/num_blocks/block_size/encr_sat_data/sab_size/
     acis_empty/_dxf_sab_converted -- all of which land on this copy and are
     thrown away below. The three output pointers are cleared first so the
     conversion callocs fresh arrays instead of realloc()ing (and thereby
     possibly freeing) anything the original owns. */
  Dwg_Entity_3DSOLID copy = *(const Dwg_Entity_3DSOLID *)entity;
  copy.num_blocks = 0;
  copy.block_size = NULL;
  copy.encr_sat_data = NULL;

  char *text = NULL;
  int error = dwg_convert_SAB_to_SAT1 (&copy);
  if (error == 0 && copy.num_blocks > 0 && copy.block_size && copy.encr_sat_data)
    {
      size_t total = 0;
      for (BITCODE_BL i = 0; i < copy.num_blocks; i++)
        if (copy.encr_sat_data[i])
          total += copy.block_size[i];
      text = malloc (total + 1);
      if (text)
        {
          size_t off = 0;
          for (BITCODE_BL i = 0; i < copy.num_blocks; i++)
            {
              if (!copy.encr_sat_data[i] || copy.block_size[i] == 0)
                continue;
              memcpy (text + off, copy.encr_sat_data[i], copy.block_size[i]);
              off += copy.block_size[i];
            }
          text[off] = '\0';
          if (out_len)
            *out_len = off;
        }
    }

  /* Release what the conversion allocated on the copy. On the error path
     it may already have calloc'd block_size/encr_sat_data (before the
     "ACIS BinaryFile" header check) with num_blocks reset to 0, so the
     per-block loop is a no-op there and only the arrays themselves go. */
  if (copy.encr_sat_data)
    {
      for (BITCODE_BL i = 0; i < copy.num_blocks; i++)
        free (copy.encr_sat_data[i]);
      free (copy.encr_sat_data);
    }
  free (copy.block_size);
  return text;
}

void
uncad_free_sat_text (char *text)
{
  free (text);
}

/* --- version bands, codepage and string width ---------------------------- */

int
uncad_dwg_is_pre_r13 (const Dwg_Data *dwg)
{
  if (!dwg)
    return 0;
  Dwg_Version_Type v = dwg->header.from_version;
  if (v == R_INVALID)
    v = dwg->header.version;
  return v < R_13;
}

int
uncad_dwg_is_r13_to_r2000 (const Dwg_Data *dwg)
{
  if (!dwg)
    return 0;
  return R_13b1 <= dwg->header.version && dwg->header.version <= R_2000;
}

int
uncad_dwg_is_r2010_or_later (const Dwg_Data *dwg)
{
  if (!dwg)
    return 0;
  Dwg_Version_Type v = dwg->header.from_version;
  if (v == R_INVALID)
    v = dwg->header.version;
  return v >= R_2010;
}

int
uncad_dwg_is_r2013_or_later (const Dwg_Data *dwg)
{
  if (!dwg)
    return 0;
  Dwg_Version_Type v = dwg->header.from_version;
  if (v == R_INVALID)
    v = dwg->header.version;
  return v >= R_2013b;
}

uint16_t
uncad_dwg_codepage (const Dwg_Data *dwg)
{
  if (!dwg)
    return 0;
  return dwg->header.codepage;
}

int
uncad_dwg_is_wide_string (const Dwg_Data *dwg)
{
  /* The library's own IS_FROM_TU_DWG (bits.h, an internal header on this
   * file's include path): read from an R2007+ DWG, and not through an
   * importer (DWG_OPTS_IN -- DXF/JSON input). */
  return (dwg && IS_FROM_TU_DWG (dwg)) ? 1 : 0;
}
