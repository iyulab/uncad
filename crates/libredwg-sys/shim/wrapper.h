/* bindgen entry point: the public LibreDWG API plus this crate's own read-side
   C shims (uncad_shim.h: entity/object pointer accessors, MULTILEADER leader
   flattening, 3DSOLID SAB->SAT conversion on a copy). */
#include "dwg.h"
#include "dwg_api.h"
#include "uncad_shim.h"
