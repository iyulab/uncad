#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "dwg.h"
#include "dwg_api.h"
/* Private LibreDWG headers (vendor/libredwg/src, on the include path in
   build.rs): bits.h for Bit_Chain, IS_FROM_TU_DWG and bit_TV_to_utf8;
   decode.h for dwg_decode; in_dxf.h for dwg_read_dxf/dwg_read_dxfb;
   codepages.h for the Dwg_Codepage enum; logging.h for the `loglevel`
   global the file-based readers set from dwg->opts. */
#include "bits.h"
#include "codepages.h"
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

unsigned int
uncad_dwg_codepage (const Dwg_Data *dwg)
{
  return dwg ? (unsigned int)dwg->header.codepage : 0;
}

int
uncad_dwg_from_dxf (const Dwg_Data *dwg)
{
  return (dwg && (dwg->opts & DWG_OPTS_INDXF)) ? 1 : 0;
}

char *
uncad_dwg_string_to_utf8 (const Dwg_Data *dwg, const char *s)
{
  if (!s)
    return NULL;
  if (dwg && IS_FROM_TU_DWG (dwg))
    return bit_convert_TU ((BITCODE_TU)s);
  return uncad_tv_to_utf8 (dwg, s);
}

int
uncad_dwg_is_tu (const Dwg_Data *dwg)
{
  return (dwg && IS_FROM_TU_DWG (dwg)) ? 1 : 0;
}

const char *
uncad_codepage_name (unsigned int codepage)
{
  return dwg_codepage_dxfstr ((Dwg_Codepage)codepage);
}

/* --- strings -------------------------------------------------------------- */

static char *
dup_string (const char *s)
{
  size_t n = strlen (s);
  char *d = (char *)malloc (n + 1);
  if (d)
    memcpy (d, s, n + 1);
  return d;
}

/* Appends the UTF-8 encoding of one BMP code point (at most 3 bytes) and
   returns how many bytes it wrote. Lone surrogates and anything outside the
   BMP -- which no 8-bit code-page table produces -- become U+FFFD so the
   3-bytes-per-input-byte bound below always holds. */
static size_t
put_utf8 (char *out, uint32_t wc)
{
  if (wc < 0x80)
    {
      out[0] = (char)wc;
      return 1;
    }
  if (wc < 0x800)
    {
      out[0] = (char)(0xC0 | (wc >> 6));
      out[1] = (char)(0x80 | (wc & 0x3F));
      return 2;
    }
  if ((wc >= 0xD800 && wc <= 0xDFFF) || wc > 0xFFFF)
    wc = 0xFFFD;
  out[0] = (char)(0xE0 | (wc >> 12));
  out[1] = (char)(0x80 | ((wc >> 6) & 0x3F));
  out[2] = (char)(0x80 | (wc & 0x3F));
  return 3;
}

/* Transcodes an 8-bit code-page string to UTF-8 with LibreDWG's own
   code-page tables (dwg_codepage_uc / dwg_codepage_uwc / is_twobyte) and
   then expands \U+XXXX / \M+nXXXX escapes the way bit_TV_to_utf8 does.

   This is a re-implementation of bit_TV_to_utf8_codepage (src/bits.c)
   rather than a call to it, for two reasons found in review: that function
   sizes its output at 1.5x the input for single-byte code pages and stops
   *reading* once the output is full, so a CP1251/CP1252 string that is
   mostly non-ASCII loses its tail ("Стена" -> "Стен"); and it writes a NUL
   for a character the table cannot map, truncating the rest of the string.
   Here the buffer is 3 bytes per input byte (the true bound) and an
   unmappable character becomes U+FFFD. `cp` must be a value
   dwg_codepage_dxfstr knows (the caller checks). Returns NULL only when
   out of memory. */
static char *
convert_codepage (const char *src, Dwg_Codepage cp)
{
  const bool is_asian = dwg_codepage_isasian (cp);
  const size_t srclen = strlen (src);
  const unsigned char *p = (const unsigned char *)src;
  const unsigned char *end = p + srclen;
  char *out = (char *)malloc (srclen * 3 + 1);
  char *expanded;
  size_t o = 0;

  if (!out)
    return NULL;
  while (p < end)
    {
      uint32_t wc;
      unsigned int c = *p++;
      if (is_asian)
        {
          /* Two-byte code pages have exceptions below 0x80 too, so every
             byte goes through the table, as in bits.c. */
          uint16_t cc = (uint16_t)c;
          if (dwg_codepage_is_twobyte (cp, (unsigned char)c) && p < end)
            cc = (uint16_t)((cc << 8) | *p++);
          wc = (uint32_t)dwg_codepage_uwc (cp, cc);
          if (wc == 0)
            wc = cc < 0x80 ? cc : 0xFFFD;
        }
      else if (c < 0x80)
        wc = c;
      else
        {
          wc = (uint32_t)dwg_codepage_uc (cp, (unsigned char)c);
          if (wc == 0)
            wc = 0xFFFD;
        }
      o += put_utf8 (out + o, wc);
    }
  out[o] = '\0';

  /* The escape expansion: with CP_UTF8, bit_TV_to_utf8 only rewrites the
     \U+XXXX / \M+nXXXX sequences (in place, or in a fresh copy) and leaves
     every other byte alone. */
  expanded = bit_TV_to_utf8 (out, (BITCODE_RS)CP_UTF8);
  if (expanded && expanded != out)
    {
      free (out);
      return expanded;
    }
  return out;
}

/* The 8-bit half of uncad_tv_to_utf8: `s` holds bytes in the file's own
   text encoding -- UTF-8 for an R2007+ DXF (what in_dxf.c's own TU
   conversion, bit_utf8_to_TU, assumes of the file too), the header code
   page for everything else -- and is never a UTF-16 buffer. */
static char *
bytes_to_utf8 (const Dwg_Data *dwg, const char *s)
{
  unsigned int cp = (unsigned int)dwg->header.codepage;
  char *converted;

  if ((dwg->opts & DWG_OPTS_IN) && dwg->header.from_version >= R_2007)
    cp = CP_UTF8;
  if (cp == CP_UTF8)
    {
      /* Only the \U+XXXX / \M+nXXXX escapes are rewritten; bit_TV_to_utf8
         returns NULL on error, its *input* when there was nothing to
         convert, or a fresh buffer. Normalise all three to "a buffer the
         caller owns". */
      converted = bit_TV_to_utf8 (s, (BITCODE_RS)CP_UTF8);
      if (!converted || converted == s)
        return dup_string (s);
      return converted;
    }
  /* The code page is a raw RS from the file header (header.spec) and
     LibreDWG indexes its tables with it unchecked -- a corrupt or unknown
     value would read past cp_fntbl[]. CP_UNDEFINED ("mostly R11") and
     CP_UTF16 (never valid for an 8-bit string) fall back to ANSI_1252,
     LibreDWG's own default for an undeclared code page. */
  if (cp > CP_ANSI_1258 || cp == CP_UTF16 || !dwg_codepage_dxfstr ((Dwg_Codepage)cp))
    cp = CP_ANSI_1252;

  converted = convert_codepage (s, (Dwg_Codepage)cp);
  return converted ? converted : dup_string (s);
}

char *
uncad_tv_to_utf8 (const Dwg_Data *dwg, const char *s)
{
  if (!s)
    return NULL;
  /* R2007+ DWG: dynapi already handed out UTF-8 (bit_convert_TU). */
  if (!dwg || IS_FROM_TU_DWG (dwg))
    return dup_string (s);

  /* R2007+ DXF: in_dxf stores its T fields as UTF-16 (TU) as well, but
     IS_FROM_TU_DWG is false for DXF input, so dynapi hands the UTF-16
     buffer out as if it were an 8-bit string (truncated at its first NUL,
     which is why "*Model_Space" used to come back as "*"). Convert it the
     way dynapi does for a DWG. The fields in_dxf.c stores 8-bit anyway
     (see uncad_bytes_to_utf8) must not come through here: bit_convert_TU
     scans for a 16-bit NUL and would read past their allocation. */
  if ((dwg->opts & DWG_OPTS_IN) && dwg->header.version >= R_2007)
    {
      char *converted = bit_convert_TU ((BITCODE_TU)(uintptr_t)s);
      return converted ? converted : dup_string ("");
    }

  return bytes_to_utf8 (dwg, s);
}

char *
uncad_bytes_to_utf8 (const Dwg_Data *dwg, const char *s)
{
  if (!s)
    return NULL;
  if (!dwg || IS_FROM_TU_DWG (dwg))
    return dup_string (s);
  return bytes_to_utf8 (dwg, s);
}

/* The Dwg_Data owning an entity/object struct pointer (what
   uncad_object_entity_ptr / uncad_object_object_ptr returned), or NULL
   when dwg_obj_generic_to_object cannot walk back to the Dwg_Object. */
static const Dwg_Data *
owning_dwg (const void *entity)
{
  int error = 0;
  const Dwg_Object *obj;
  if (!entity)
    return NULL;
  obj = dwg_obj_generic_to_object (entity, &error);
  return (obj && !error) ? obj->parent : NULL;
}

char *
uncad_entity_tv_to_utf8 (const void *entity, const char *s)
{
  return uncad_tv_to_utf8 (owning_dwg (entity), s);
}

char *
uncad_entity_bytes_to_utf8 (const void *entity, const char *s)
{
  return uncad_bytes_to_utf8 (owning_dwg (entity), s);
}

void
uncad_free_string (char *s)
{
  free (s);
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
