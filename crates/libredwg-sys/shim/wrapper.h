/* bindgen entry point: the public LibreDWG API plus this crate's own read-side
   C shims (uncad_shim.h: entity/object pointer accessors, MULTILEADER leader
   flattening, 3DSOLID SAB->SAT conversion on a copy). */
#include "dwg.h"
#include "dwg_api.h"
/* Internal header, reachable because build.rs puts vendor/libredwg/src on
   the include path: the codepage tables' lookup functions, which the Rust
   side uses to decode pre-R2007 strings (see uncad_dwg_codepage). */
#include "codepages.h"
#include "uncad_shim.h"
