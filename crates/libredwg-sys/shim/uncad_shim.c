#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "dwg.h"
#include "dwg_api.h"
#include "bits.h"
#include "out_dxf.h"
#include "uncad_shim.h"

int
uncad_write_dxf_file (const char *dwg_path, const char *dxf_path)
{
  if (strcmp (dwg_path, dxf_path) == 0)
    return DWG_ERR_IOERROR;

  Dwg_Data dwg;
  Bit_Chain dat;
  memset (&dwg, 0, sizeof (dwg));
  memset (&dat, 0, sizeof (dat));

  int error = dwg_read_file (dwg_path, &dwg);
  if (error >= DWG_ERR_CRITICAL)
    {
      dwg_free (&dwg);
      return error;
    }

  dat.version = dwg.header.version;
  dat.from_version = dwg.header.from_version;
  dat.fh = fopen (dxf_path, "wb");
  if (!dat.fh)
    {
      dwg_free (&dwg);
      return DWG_ERR_IOERROR;
    }

  error = dwg_write_dxf (&dat, &dwg);
  fclose (dat.fh);
  dwg_free (&dwg);
  return error;
}

int
uncad_write_dxf (Dwg_Data *dwg, const char *dxf_path)
{
  Bit_Chain dat;
  memset (&dat, 0, sizeof (dat));

  dat.version = dwg->header.version;
  dat.from_version = dwg->header.from_version;
  dat.fh = fopen (dxf_path, "wb");
  if (!dat.fh)
    return DWG_ERR_IOERROR;

  int error = dwg_write_dxf (&dat, dwg);
  fclose (dat.fh);
  return error;
}

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

unsigned int
uncad_multileader_get_lines (void *entity, uncad_multileader_line_t **out_lines)
{
  *out_lines = NULL;
  if (!entity)
    return 0;
  Dwg_Entity_MULTILEADER *mleader = (Dwg_Entity_MULTILEADER *)entity;
  Dwg_MLEADER_AnnotContext *ctx = &mleader->ctx;

  /* First pass: count visible (type != 0) lines with at least one point,
   * across every leader node, so out_lines can be sized exactly. */
  unsigned int total_lines = 0;
  for (BITCODE_BL i = 0; i < ctx->num_leaders; i++)
    {
      Dwg_LEADER_Node *node = &ctx->leaders[i];
      for (BITCODE_BL j = 0; j < node->num_lines; j++)
        {
          Dwg_LEADER_Line *line = &node->lines[j];
          if (line->type != 0 && line->num_points > 0 && line->points)
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
          if (line->type == 0 || line->num_points == 0 || !line->points)
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
