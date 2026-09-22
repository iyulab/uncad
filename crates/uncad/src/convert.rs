//! Walks a parsed `Dwg_Data`'s objects and converts entities into [`Entity`]
//! values. `docs/CAVEATS.md` tracks which entity types are covered and how
//! faithful each one is.
//!
//! **Not** a flat global object scan. `CadDatabase::entities` is built by
//! walking `BLOCK_HEADER` (BLOCK_RECORD) objects and collecting only what the
//! `*Model_Space`/`*Paper_Space*` blocks own. Entities that live inside a
//! *named* block definition (a symbol an INSERT references elsewhere) are
//! deliberately excluded from the top-level list; they stay reachable through
//! that block's own owned-entity chain in [`crate::tables::Tables`]. A global
//! scan over every object silently over-collects those.

use crate::dynapi::{
    get_array_field, get_common_field, get_field, get_utf8_field, resolve_handle_name, Point2D,
    Point3D, SplineControlPoint,
};
use crate::model::{
    AcadTableEntity, ArcEntity, AttdefEntity, AttribEntity, CircleEntity, DimensionEntity,
    DimensionGeometry, DisplaySource, EllipseEntity, Entity, EntityCommon, Face3DEntity,
    HatchBoundaryPath, HatchEdge, HatchEntity, HatchGradient, HatchPatternLine, InsertEntity,
    LeaderEntity, LightEntity, LineEntity, LwPolylineEntity, MLineEntity, MLineVertex, MTextEntity,
    MultiLeaderEntity, PointEntity, PolylineEntity, RayEntity, Solid3DEntity, SolidEntity,
    SplineEntity, TextEntity, ToleranceEntity, ViewportEntity, WipeoutEntity,
};
use std::ffi::CStr;

/// LWPOLYLINE's in-memory `flag` bit for "closed" (`FLAG_LWPOLYLINE_CLOSED`
/// in dwg.h). It is *not* the DXF group-70 convention (bit 1): LibreDWG keeps
/// the DWG bit layout, where 1 means "an extrusion is stored", and its DXF
/// reader rewrites group-70 bit 1 to 512 on input, so both input paths agree
/// on 512. Verified handle by handle against `example_2000.dxf`'s group 70
/// (see `docs/CAVEATS.md`, "The polyline closed flag"); until 0.3.0 this
/// tested bit 1 and exported every closed LWPOLYLINE as open.
const LWPOLYLINE_CLOSED_FLAG: u16 = 512;

/// POLYLINE_2D/POLYLINE_3D keep the classic 1 = closed bit (dwg.h
/// `Dwg_Entity_POLYLINE_2D.flag`: "1: closed").
const POLYLINE_CLOSED_FLAG: u8 = 1;

/// POLYLINE_MESH.flag bit 1: the grid wraps in M ("closed polygon mesh in
/// the M direction", DXF group 70).
const POLYLINE_MESH_CLOSED_M_FLAG: u16 = 1;

/// POLYLINE_MESH.flag bit 32: the grid wraps in N.
const POLYLINE_MESH_CLOSED_N_FLAG: u16 = 32;

/// `MLINE_FLAGS_CLOSED` (dwg.h).
const MLINE_CLOSED_FLAG: u16 = 2;

fn is_model_space(name: &str) -> bool {
    name.to_uppercase() == "*MODEL_SPACE"
}

/// Matches `*Paper_Space`, `*Paper_Space0`, `*Paper_Space1`, ... -- every
/// layout tab.
fn is_paper_space(name: &str) -> bool {
    name.to_uppercase().starts_with("*PAPER_SPACE")
}

/// Walks every `BLOCK_HEADER` object, collects the entities owned by the
/// `*Model_Space`/`*Paper_Space*` ones (see this module's doc for why not
/// every block qualifies), and converts each. An entity of an unhandled type
/// becomes [`Entity::Unknown`], which keeps its real DXF name -- counted and
/// reportable rather than silently dropped.
///
/// # Safety
/// `dwg` must be a successfully-`dwg_read_file`'d, not-yet-`dwg_free`'d
/// `Dwg_Data`.
pub unsafe fn convert_entities(dwg: *mut libredwg_sys::Dwg_Data) -> Vec<Entity> {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut entities = Vec::new();

    for i in 0..num_objects {
        let block_obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if block_obj.is_null() {
            continue;
        }
        // `dwg_object_get_fixedtype`'s real C declaration returns plain `int`
        // rather than DWG_OBJECT_TYPE -- a signature/enum mismatch in
        // dwg_api.h itself. bindgen infers DWG_OBJECT_TYPE's own Rust
        // representation from clang's target-dependent choice of underlying
        // integer type for that C enum, which differs in practice: `i32` on
        // the MSVC target, `u32` on x86_64-unknown-linux-gnu. Every
        // `fixedtype` value is therefore cast at its FFI call site, so the
        // rest of the crate compares one canonical type on every target.
        let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(block_obj) }
            as libredwg_sys::DWG_OBJECT_TYPE;
        if fixedtype != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK_HEADER {
            continue;
        }
        let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(block_obj) };
        if object_ptr.is_null() {
            continue;
        }
        let Some(name) = get_utf8_field(object_ptr, "BLOCK_HEADER", "name") else {
            continue;
        };
        if !is_model_space(&name) && !is_paper_space(&name) {
            continue;
        }

        // Each INSERT's attribs are duplicated as top-level Entity::Attrib
        // entries because that is what rendering draws. Deliberately here and
        // not inside owned_entities(): a block record's own entity list must
        // not carry the duplication (see crate::tables::BlockRecord).
        for entity in unsafe { owned_entities(dwg, block_obj) } {
            if let Entity::Insert(insert) = &entity {
                entities.extend(insert.attribs.iter().cloned().map(Entity::Attrib));
            }
            entities.push(entity);
        }
    }

    entities
}

/// Walks every entity directly owned by `block_obj` (a live `BLOCK_HEADER`
/// `Dwg_Object`) and converts each -- no attrib duplication, see
/// `convert_entities`'s call site. Shared by the model/paper-space entity list
/// and by every block's own entry in [`crate::tables::Tables::block_records`],
/// so the two stay in sync as entity types are added.
///
/// A polyline's VERTEX_* records are skipped: they are structural
/// subentities of the POLYLINE that owns them, read from *its* chain (see
/// [`polyline_subentities`]), not drawing content of the block. That is
/// exactly the contract LibreDWG documents for `get_next_owned_entity`
/// ("Not subentities: ATTRIB, VERTEX") and what its R13-R2000 branch
/// implements -- but its R2004+ branch just indexes `BLOCK_HEADER.entities[]`,
/// which the DXF reader fills with every object between the BLOCK and the
/// ENDBLK. So before 0.3.0 an R2004+ DXF reported each polyline's vertices as
/// top-level `Entity::Unknown` values: nine "entities" for one polyface mesh,
/// counted in the CLI summary, named in `report.json`'s unsupported list and
/// present in `block_records["*Model_Space"].entities`.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` `block_obj` was obtained from;
/// `block_obj` must be a valid, non-null `BLOCK_HEADER` object.
pub(crate) unsafe fn owned_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    let mut owned = unsafe { libredwg_sys::get_first_owned_entity(block_obj) };
    while !owned.is_null() {
        let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(owned) }
            as libredwg_sys::DWG_OBJECT_TYPE;
        if !is_polyline_vertex(fixedtype) {
            if let Some(entity) = unsafe { convert_entity(dwg, owned, 0) } {
                entities.push(entity);
            }
        }
        owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
    }
    entities
}

/// True for the five VERTEX_* subentity types an old-style POLYLINE owns.
/// Matches the list LibreDWG's own `get_next_owned_entity` skips over ("Not
/// subentities: ATTRIB, VERTEX", dwg.c).
fn is_polyline_vertex(fixedtype: libredwg_sys::DWG_OBJECT_TYPE) -> bool {
    matches!(
        fixedtype,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE_FACE
    )
}

/// Every subentity an old-style POLYLINE owns, in order.
///
/// Walks the owned-subentity chain (`get_first`/`get_next_owned_subentity`,
/// dwg.c) rather than calling LibreDWG's own
/// `dwg_object_polyline_{2,3}d_get_points`, because those are short by one on
/// every R13/R14/R2000 file: for `version < R_2004` both the point and the
/// count accessor walk `first_vertex..last_vertex` as
/// `do { ... } while ((vobj = dwg_next_object (vobj)) && vobj != vlast);`
/// (dwg_api.c), whose condition ends the loop *before* the body ever sees
/// `vlast`. They return N-1 points, and the last vertex of every such
/// polyline was dropped -- from the picture, from `length`/`area`, and,
/// because the bulges were already read from this chain and so came back N
/// long, from the bulge list too (the length mismatch cleared every bulge,
/// which turned a two-vertex arc into a point). `get_next_owned_subentity`
/// stops *at* `last_vertex` and yields all N; for R2004+ it indexes the
/// `vertex[]` array by `num_owned`, the same source the accessors use there.
///
/// Pre-R13 files fill neither `first_vertex` nor `vertex[]`, so that chain is
/// empty for them and the fallback is the scan LibreDWG's own pre-R13 branch
/// uses: forward through the object list, which is where a pre-R13 polyline's
/// vertices physically are, stopping at the first object that is not a VERTEX
/// (the SEQEND, in a well-formed file).
///
/// Both walks are bounded against a chain a damaged handle turned into a ring
/// (see [`crate::limits::MAX_OWNED_SUBENTITIES`]).
///
/// # Safety
/// `obj` must be a valid, non-null `POLYLINE_2D`/`POLYLINE_3D`/
/// `POLYLINE_PFACE`/`POLYLINE_MESH` `Dwg_Object`.
unsafe fn polyline_subentities(
    obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<*mut libredwg_sys::Dwg_Object> {
    let mut subs = Vec::new();
    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    let mut walked = 0usize;
    while !sub.is_null() && walked < crate::limits::MAX_OWNED_SUBENTITIES {
        walked += 1;
        subs.push(sub);
        sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
    }
    if !subs.is_empty() {
        return subs;
    }
    let mut next = unsafe { libredwg_sys::dwg_next_object(obj) };
    while !next.is_null() && subs.len() < crate::limits::MAX_OWNED_SUBENTITIES {
        let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(next) }
            as libredwg_sys::DWG_OBJECT_TYPE;
        if !is_polyline_vertex(fixedtype) {
            break;
        }
        subs.push(next);
        next = unsafe { libredwg_sys::dwg_next_object(next) };
    }
    subs
}

/// The vertices of `vertex_type` an old-style POLYLINE owns, in order, each
/// with its bulge (0.0 for a vertex type that has no `bulge` field).
///
/// # Safety
/// `obj` must be a valid, non-null POLYLINE `Dwg_Object` per
/// [`polyline_subentities`], and `vertex_dxfname` must be the dynapi name of
/// `vertex_type`.
unsafe fn polyline_vertices(
    obj: *mut libredwg_sys::Dwg_Object,
    vertex_type: libredwg_sys::DWG_OBJECT_TYPE,
    vertex_dxfname: &str,
) -> Vec<(Point3D, f64)> {
    let mut vertices = Vec::new();
    for sub in unsafe { polyline_subentities(obj) } {
        let sub_fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        if sub_fixedtype != vertex_type {
            continue;
        }
        let sub_entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
        if let Some(point) = get_field::<Point3D>(sub_entity_ptr, vertex_dxfname, "point") {
            // VERTEX_3D and the mesh vertex types carry no `bulge`, and the
            // dynapi lookup simply fails for them.
            let bulge = get_field::<f64>(sub_entity_ptr, vertex_dxfname, "bulge").unwrap_or(0.0);
            vertices.push((point, bulge));
        }
    }
    vertices
}

/// Resolves a POLYLINE_PFACE's mesh into wireframe edges by walking its owned
/// subentities directly -- vertex positions, in order, then `VERTEX_PFACE_FACE`
/// (up to 4 vertex indices per face, 1-based, negative meaning "invisible
/// edge" -- the sign carries no other meaning, so it is just dropped);
/// LibreDWG's own accessor for this type is documented as not implemented.
///
/// A position vertex is a `VERTEX_PFACE` *or* a `VERTEX_MESH`. Both mean the
/// same thing inside a POLYLINE_PFACE's own chain, and the DXF reader hands
/// back the second one for a polyface whose `AcDbPolyFaceMeshVertex` records
/// name the block record as their owner rather than the POLYLINE: `in_dxf.c`
/// picks between the two types by looking the VERTEX's own group 330 up and
/// asking whether it is a POLYLINE_PFACE, and falls back to VERTEX_MESH when
/// it is not. ezdxf writes exactly that shape (and `audit()` passes it), so
/// before 0.3.0 a DXF polyface mesh found no positions at all and drew
/// nothing.
///
/// Face records may be interleaved with vertex records, so indices are only
/// resolved once the whole chain has been walked. Faces referencing an
/// out-of-range or all-zero index list are skipped.
///
/// # Safety
/// `obj` must be a valid, non-null `POLYLINE_PFACE` `Dwg_Object`.
unsafe fn polyline_pface_wireframe(obj: *mut libredwg_sys::Dwg_Object) -> Vec<[Point3D; 2]> {
    let mut positions = Vec::new();
    let mut faces: Vec<[i16; 4]> = Vec::new();

    for sub in unsafe { polyline_subentities(obj) } {
        let sub_fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        let sub_entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
        if sub_entity_ptr.is_null() {
            continue;
        }
        let vertex_dxfname = match sub_fixedtype {
            libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE => Some("VERTEX_PFACE"),
            libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH => Some("VERTEX_MESH"),
            _ => None,
        };
        if let Some(dxfname) = vertex_dxfname {
            if let Some(p) = get_field::<Point3D>(sub_entity_ptr, dxfname, "point") {
                positions.push(p);
            }
        } else if sub_fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE_FACE {
            if let Some(vertind) =
                get_field::<[i16; 4]>(sub_entity_ptr, "VERTEX_PFACE_FACE", "vertind")
            {
                faces.push(vertind);
            }
        }
    }

    let mut edges = Vec::new();
    for face in &faces {
        let idxs: Vec<usize> = face
            .iter()
            // A negative index marks the edge that follows as invisible; the
            // sign is dropped (every edge is drawn). 0 means "no vertex".
            // checked_sub, not `then_some(a - 1)`: then_some evaluates its
            // argument eagerly, so index 0 overflowed in debug builds.
            .filter_map(|&i| (i.unsigned_abs() as usize).checked_sub(1))
            .collect();
        if idxs.len() < 2 {
            continue;
        }
        for w in 0..idxs.len() {
            let (a, b) = (idxs[w], idxs[(w + 1) % idxs.len()]);
            if let (Some(&pa), Some(&pb)) = (positions.get(a), positions.get(b)) {
                edges.push([pa, pb]);
            }
        }
    }
    edges
}

/// Resolves a POLYLINE_MESH ("polygon mesh") into the wireframe of its grid:
/// `m` rows by `n` columns of VERTEX_MESH subentities, stored row-major, with
/// one edge between every pair of grid neighbours. `flag` bit 1 wraps the grid
/// in M and bit 32 in N (`dwg.spec`'s POLYLINE_MESH group 70), which adds the
/// closing row/column of edges.
///
/// An open `m` by `n` grid has `n * (m - 1) + m * (n - 1)` edges. No edges are
/// produced unless exactly `m * n` vertices were found: a smooth-surface mesh
/// stores spline control points alongside the approximated ones, and guessing
/// a grid shape that the counts do not support would draw a lie.
///
/// # Safety
/// `obj` must be a valid, non-null `POLYLINE_MESH` `Dwg_Object`.
unsafe fn polyline_mesh_wireframe(
    obj: *mut libredwg_sys::Dwg_Object,
    m: usize,
    n: usize,
    flag: u16,
) -> Vec<[Point3D; 2]> {
    let positions: Vec<Point3D> = unsafe {
        polyline_vertices(
            obj,
            libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH,
            "VERTEX_MESH",
        )
    }
    .into_iter()
    .map(|(point, _no_bulge)| point)
    .collect();
    if m < 2 || n < 2 || positions.len() != m.saturating_mul(n) {
        return Vec::new();
    }
    let closed_m = flag & POLYLINE_MESH_CLOSED_M_FLAG != 0;
    let closed_n = flag & POLYLINE_MESH_CLOSED_N_FLAG != 0;
    let mut edges = Vec::new();
    // Down each column, then along each row.
    for i in 0..if closed_m { m } else { m - 1 } {
        for j in 0..n {
            edges.push([positions[i * n + j], positions[((i + 1) % m) * n + j]]);
        }
    }
    for row in positions.chunks_exact(n) {
        for j in 0..if closed_n { n } else { n - 1 } {
            edges.push([row[j], row[(j + 1) % n]]);
        }
    }
    edges
}

/// The entity's linetype name. R2000+ files store `ltype_flags` (0 BYLAYER,
/// 1 BYBLOCK, 2 CONTINUOUS, 3 "a handle follows"); R13/R14 and the DXF
/// reader store the handle alone. A handle wins when there is one.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` that owns `entity_ptr`.
unsafe fn entity_linetype(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
) -> String {
    if let Some(name) = get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "ltype")
        .filter(|h| !h.is_null())
        .and_then(|h| resolve_handle_name(dwg, h))
    {
        return name;
    }
    match get_common_field::<u8>(entity_ptr, "ltype_flags").unwrap_or(0) {
        1 => "BYBLOCK",
        2 => "CONTINUOUS",
        _ => "BYLAYER",
    }
    .to_string()
}

/// An entity's stored OCS normal (`extrusion`, DXF 210), normalized; the
/// world z axis when absent, zero or unreadable.
fn read_extrusion(entity_ptr: *mut std::ffi::c_void, dxfname: &str) -> Point3D {
    get_field::<Point3D>(entity_ptr, dxfname, "extrusion")
        .map(crate::geom::normalize_extrusion)
        .unwrap_or(crate::geom::WORLD_Z)
}

/// Keeps a polyline's bulges consistent with its vertices once those are in
/// world coordinates: the OCS-to-world map of a mirrored extrusion (z < 0)
/// is a reflection, which reverses every arc's turning direction, so each
/// bulge changes sign. A bulge is `tan(theta / 4)` with the sign of the
/// turn, so negating it is exact.
fn mirror_bulges(bulges: &mut [f64], extrusion: Point3D) {
    if extrusion.z < 0.0 {
        for b in bulges.iter_mut().filter(|b| **b != 0.0) {
            *b = -*b;
        }
    }
}

/// The justification and style fields TEXT and ATTRIB share (same dynapi
/// names on both types).
struct TextLayout {
    horizontal_alignment: u16,
    vertical_alignment: u16,
    alignment_point: Option<Point2D>,
    width_factor: f64,
    oblique_angle: f64,
    style: String,
}

/// Moves a TEXT/ATTRIB's anchor points from its OCS (at `elevation`) to
/// world coordinates.
///
/// # Safety
/// `entity_ptr` must be a `dxfname` entity's type-specific struct pointer.
unsafe fn text_to_wcs(
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
    start_point: Point2D,
    mut layout: TextLayout,
) -> (Point2D, TextLayout) {
    let extrusion = read_extrusion(entity_ptr, dxfname);
    if crate::geom::is_world_z(extrusion) {
        return (start_point, layout);
    }
    let elevation = get_field::<f64>(entity_ptr, dxfname, "elevation").unwrap_or(0.0);
    let start_point = crate::geom::ocs_to_wcs_2d(start_point, elevation, extrusion);
    layout.alignment_point = layout
        .alignment_point
        .map(|p| crate::geom::ocs_to_wcs_2d(p, elevation, extrusion));
    (start_point, layout)
}

/// # Safety
/// `dwg` must be the live `Dwg_Data` that owns `entity_ptr`, a `dxfname`
/// entity's type-specific struct pointer.
unsafe fn text_layout(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
) -> TextLayout {
    let horizontal_alignment =
        get_field::<u16>(entity_ptr, dxfname, "horiz_alignment").unwrap_or(0);
    let vertical_alignment = get_field::<u16>(entity_ptr, dxfname, "vert_alignment").unwrap_or(0);
    // dwg.h: alignment_pt is "optional, when dataflags & 2, i.e. 72/73 != 0";
    // for left/baseline text the field holds whatever the decoder left.
    let alignment_point = if horizontal_alignment != 0 || vertical_alignment != 0 {
        get_field::<Point2D>(entity_ptr, dxfname, "alignment_pt")
    } else {
        None
    };
    // 0 is "unset" in a hand-written DXF; the width factor is never really 0.
    let width_factor = get_field::<f64>(entity_ptr, dxfname, "width_factor")
        .filter(|w| *w > 0.0)
        .unwrap_or(1.0);
    let oblique_angle = get_field::<f64>(entity_ptr, dxfname, "oblique_angle").unwrap_or(0.0);
    let style = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "style")
        .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
        .unwrap_or_default();
    TextLayout {
        horizontal_alignment,
        vertical_alignment,
        alignment_point,
        width_factor,
        oblique_angle,
        style,
    }
}

/// Resolves a WIPEOUT's clip boundary to local 2D points -- see
/// [`crate::model::WipeoutEntity`] for the risk this carries.
///
/// `clip_boundary_type` 1 ("rect") stores exactly 2 `clip_verts`, two opposite
/// corners of a pixel-space-axis-aligned rectangle; anything else (2,
/// "polygon", or unset) is used as an explicit vertex list. With no usable
/// `clip_verts` at all this falls back to the full image rectangle implied by
/// `image_size`: clipping is optional in the format, but every WIPEOUT still
/// has its full image extent.
///
/// Each pixel-space `(u, v)` maps to `pt0 + u*uvec + v*vvec` -- `uvec`/`vvec`
/// are already one-pixel-length vectors in the entity's local space, not
/// normalized directions.
fn wipeout_boundary(entity_ptr: *mut std::ffi::c_void) -> Vec<Point2D> {
    let Some(pt0) = get_field::<Point3D>(entity_ptr, "WIPEOUT", "pt0") else {
        return Vec::new();
    };
    let uvec = get_field::<Point3D>(entity_ptr, "WIPEOUT", "uvec").unwrap_or(Point3D {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    let vvec = get_field::<Point3D>(entity_ptr, "WIPEOUT", "vvec").unwrap_or(Point3D {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    });
    let clip_verts: Vec<Point2D> =
        get_array_field::<u32, _>(entity_ptr, "WIPEOUT", "num_clip_verts", "clip_verts");
    // BITCODE_BS ("1 rect, 2 polygon"). An unreadable or unset value is
    // treated like "polygon", not assumed to be "rect".
    let clip_boundary_type =
        get_field::<u16>(entity_ptr, "WIPEOUT", "clip_boundary_type").unwrap_or(0);

    let pixel_points: Vec<Point2D> = if clip_boundary_type == 1 && clip_verts.len() == 2 {
        let (a, b) = (clip_verts[0], clip_verts[1]);
        vec![
            Point2D { x: a.x, y: a.y },
            Point2D { x: b.x, y: a.y },
            Point2D { x: b.x, y: b.y },
            Point2D { x: a.x, y: b.y },
        ]
    } else if !clip_verts.is_empty() {
        clip_verts
    } else {
        let size = get_field::<Point2D>(entity_ptr, "WIPEOUT", "image_size")
            .unwrap_or(Point2D { x: 0.0, y: 0.0 });
        vec![
            Point2D { x: 0.0, y: 0.0 },
            Point2D { x: size.x, y: 0.0 },
            Point2D {
                x: size.x,
                y: size.y,
            },
            Point2D { x: 0.0, y: size.y },
        ]
    };

    pixel_points
        .into_iter()
        .map(|p| Point2D {
            x: pt0.x + p.x * uvec.x + p.y * vvec.x,
            y: pt0.y + p.x * uvec.y + p.y * vvec.y,
        })
        .collect()
}

/// `depth` is how many owned-*sub*entity steps were taken to reach `obj`
/// (0 for an entity owned by a block record). An INSERT converts its own
/// ATTRIBs, which is the one real nesting the format has; a file whose
/// handles have been damaged can point that chain back at the INSERT and
/// make the recursion endless, so it stops at
/// [`MAX_SUBENTITY_DEPTH`](crate::limits::MAX_SUBENTITY_DEPTH).
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` `obj` was obtained from; `obj` must be a
/// valid pointer from `dwg_get_object` on that same `Dwg_Data`.
unsafe fn convert_entity(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    depth: u32,
) -> Option<Entity> {
    if depth > crate::limits::MAX_SUBENTITY_DEPTH {
        return None;
    }
    // Cast for cross-platform bindgen enum-width consistency -- see the
    // comment on the same call in convert_entities() above.
    let fixedtype =
        unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) } as libredwg_sys::DWG_OBJECT_TYPE;

    // Only entities have geometry to convert here. Non-entity OBJECT-supertype
    // objects (LAYER, BLOCK_RECORD, DICTIONARY, ...) return null from
    // uncad_object_entity_ptr and are skipped rather than mis-reported as
    // unknown entities; the ones this crate needs are converted separately by
    // tables::convert_tables.
    let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(obj) };
    if entity_ptr.is_null() {
        return None;
    }

    // BLOCK/ENDBLK are structural block-boundary sentinels (every BLOCK_RECORD
    // owns exactly one of each) and SEQEND closes an INSERT's attrib chain or
    // an old-style POLYLINE's vertex chain. None of them is user-visible
    // drawing content.
    if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ENDBLK
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND
    {
        return None;
    }

    // SAFETY: obj is valid per this function's own `# Safety` doc contract.
    let handle = unsafe { entity_handle(obj) };
    let layer = get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "layer")
        .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
        .unwrap_or_default();
    let (color_index, true_color) = entity_color(entity_ptr);
    let source = unsafe { crate::header::source(dwg) };
    let invisible = get_common_field::<u16>(entity_ptr, "invisible").unwrap_or(0) != 0;
    // R13/R14 entities store no lineweight; LibreDWG leaves the code at 0,
    // which would read as 0.00 mm.
    let lineweight_mm = get_common_field::<u8>(entity_ptr, "linewt")
        .filter(|_| source.r2000_plus)
        .and_then(crate::visibility::lineweight_mm);
    let ltype_scale = get_common_field::<f64>(entity_ptr, "ltype_scale")
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(1.0);
    let linetype = unsafe { entity_linetype(dwg, entity_ptr) };
    let common = EntityCommon {
        handle,
        layer,
        color_index,
        true_color,
        invisible,
        lineweight_mm,
        linetype,
        ltype_scale,
    };

    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LINE => {
            let start_point = get_field::<Point3D>(entity_ptr, "LINE", "start")?;
            let end_point = get_field::<Point3D>(entity_ptr, "LINE", "end")?;
            Entity::Line(LineEntity {
                common,
                start_point,
                end_point,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_CIRCLE => {
            let center = get_field::<Point3D>(entity_ptr, "CIRCLE", "center")?;
            let radius = get_field::<f64>(entity_ptr, "CIRCLE", "radius")?;
            let extrusion = read_extrusion(entity_ptr, "CIRCLE");
            Entity::Circle(CircleEntity {
                common,
                center: crate::geom::ocs_to_wcs(center, extrusion),
                radius,
                extrusion,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TEXT => {
            let start_point = get_field::<Point2D>(entity_ptr, "TEXT", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "TEXT", "height")?;
            let text = get_utf8_field(entity_ptr, "TEXT", "text_value").unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0);
            let layout = unsafe { text_layout(dwg, entity_ptr, "TEXT") };
            let (start_point, layout) =
                unsafe { text_to_wcs(entity_ptr, "TEXT", start_point, layout) };
            Entity::Text(TextEntity {
                common,
                start_point,
                text_height,
                text_plain: crate::text::decode_text(&text).plain,
                text,
                rotation,
                horizontal_alignment: layout.horizontal_alignment,
                vertical_alignment: layout.vertical_alignment,
                alignment_point: layout.alignment_point,
                width_factor: layout.width_factor,
                oblique_angle: layout.oblique_angle,
                style: layout.style,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LWPOLYLINE => {
            let stored: Vec<Point2D> =
                get_array_field::<u32, _>(entity_ptr, "LWPOLYLINE", "num_points", "points");
            let flag = get_field::<u16>(entity_ptr, "LWPOLYLINE", "flag").unwrap_or(0);
            // LibreDWG decodes the extrusion only when flag bit 1 says one is
            // stored; otherwise the field stays (0,0,0), which means world z.
            let extrusion = if flag & 1 != 0 {
                read_extrusion(entity_ptr, "LWPOLYLINE")
            } else {
                crate::geom::WORLD_Z
            };
            let elevation = get_field::<f64>(entity_ptr, "LWPOLYLINE", "elevation").unwrap_or(0.0);
            let mut bulges: Vec<f64> =
                get_array_field::<u32, _>(entity_ptr, "LWPOLYLINE", "num_bulges", "bulges");
            if bulges.iter().all(|b| *b == 0.0) {
                bulges.clear();
            }
            mirror_bulges(&mut bulges, extrusion);
            let widths: Vec<[f64; 2]> =
                get_array_field::<u32, _>(entity_ptr, "LWPOLYLINE", "num_widths", "widths");
            let const_width =
                get_field::<f64>(entity_ptr, "LWPOLYLINE", "const_width").unwrap_or(0.0);
            let vertices = stored
                .iter()
                .map(|v| crate::geom::ocs_to_wcs_2d(*v, elevation, extrusion))
                .collect();
            Entity::LwPolyline(LwPolylineEntity {
                common,
                vertices,
                closed: flag & LWPOLYLINE_CLOSED_FLAG != 0,
                bulges,
                widths,
                const_width,
                elevation,
                extrusion,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC => {
            let center = get_field::<Point3D>(entity_ptr, "ARC", "center")?;
            let radius = get_field::<f64>(entity_ptr, "ARC", "radius")?;
            let start_angle = get_field::<f64>(entity_ptr, "ARC", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ARC", "end_angle")?;
            let extrusion = read_extrusion(entity_ptr, "ARC");
            // Mirrored about the y axis (normal (0,0,-1)): an angle a becomes
            // pi - a, and a counter-clockwise sweep becomes clockwise, so the
            // ends swap to keep the arc counter-clockwise in world terms.
            let (start_angle, end_angle) = if extrusion.z < 0.0 {
                (
                    std::f64::consts::PI - end_angle,
                    std::f64::consts::PI - start_angle,
                )
            } else {
                (start_angle, end_angle)
            };
            Entity::Arc(ArcEntity {
                common,
                center: crate::geom::ocs_to_wcs(center, extrusion),
                radius,
                start_angle,
                end_angle,
                extrusion,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ELLIPSE => {
            let center = get_field::<Point3D>(entity_ptr, "ELLIPSE", "center")?;
            let major_axis_endpoint = get_field::<Point3D>(entity_ptr, "ELLIPSE", "sm_axis")?;
            let axis_ratio = get_field::<f64>(entity_ptr, "ELLIPSE", "axis_ratio")?;
            let start_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "end_angle")?;
            Entity::Ellipse(EllipseEntity {
                common,
                center,
                major_axis_endpoint,
                axis_ratio,
                start_angle,
                end_angle,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POINT => {
            // POINT stores x/y/z as three separate BD fields, not one 3BD
            // struct field, unlike every other point-shaped entity here.
            let x = get_field::<f64>(entity_ptr, "POINT", "x")?;
            let y = get_field::<f64>(entity_ptr, "POINT", "y")?;
            let z = get_field::<f64>(entity_ptr, "POINT", "z")?;
            Entity::Point(PointEntity {
                common,
                position: Point3D { x, y, z },
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SOLID => {
            let corner1 = get_field::<Point2D>(entity_ptr, "SOLID", "corner1")?;
            let corner2 = get_field::<Point2D>(entity_ptr, "SOLID", "corner2")?;
            let corner3 = get_field::<Point2D>(entity_ptr, "SOLID", "corner3")?;
            let corner4 = get_field::<Point2D>(entity_ptr, "SOLID", "corner4")?;
            let extrusion = read_extrusion(entity_ptr, "SOLID");
            let elevation = get_field::<f64>(entity_ptr, "SOLID", "elevation").unwrap_or(0.0);
            let to_wcs = |p: Point2D| crate::geom::ocs_to_wcs_2d(p, elevation, extrusion);
            Entity::Solid(SolidEntity {
                common,
                corner1: to_wcs(corner1),
                corner2: to_wcs(corner2),
                corner3: to_wcs(corner3),
                corner4: to_wcs(corner4),
                extrusion,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_RAY => {
            let point = get_field::<Point3D>(entity_ptr, "RAY", "point")?;
            let vector = get_field::<Point3D>(entity_ptr, "RAY", "vector")?;
            Entity::Ray(RayEntity {
                common,
                point,
                vector,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_XLINE => {
            // Same underlying C struct as RAY (Dwg_Entity_XLINE is a typedef
            // of Dwg_Entity_RAY), but dynapi is keyed by dxfname, so "XLINE"
            // is required here.
            let point = get_field::<Point3D>(entity_ptr, "XLINE", "point")?;
            let vector = get_field::<Point3D>(entity_ptr, "XLINE", "vector")?;
            Entity::XLine(RayEntity {
                common,
                point,
                vector,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB => {
            let start_point = get_field::<Point2D>(entity_ptr, "ATTRIB", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "ATTRIB", "height")?;
            let text = get_utf8_field(entity_ptr, "ATTRIB", "text_value").unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "ATTRIB", "rotation").unwrap_or(0.0);
            let tag = get_utf8_field(entity_ptr, "ATTRIB", "tag").unwrap_or_default();
            // DXF 70: 1 invisible, 2 constant, 4 verification required, 8 preset.
            let flags = get_field::<u8>(entity_ptr, "ATTRIB", "flags").unwrap_or(0);
            let layout = unsafe { text_layout(dwg, entity_ptr, "ATTRIB") };
            let (start_point, layout) =
                unsafe { text_to_wcs(entity_ptr, "ATTRIB", start_point, layout) };
            Entity::Attrib(AttribEntity {
                common,
                start_point,
                text_height,
                text_plain: crate::text::decode_text(&text).plain,
                text,
                rotation,
                tag,
                invisible: flags & 1 != 0,
                horizontal_alignment: layout.horizontal_alignment,
                vertical_alignment: layout.vertical_alignment,
                alignment_point: layout.alignment_point,
                width_factor: layout.width_factor,
                oblique_angle: layout.oblique_angle,
                style: layout.style,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_INSERT => {
            let block_name = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                entity_ptr,
                "INSERT",
                "block_header",
            )
            .and_then(crate::tables::resolve_block_name)
            .unwrap_or_default();
            let insertion_point = get_field::<Point3D>(entity_ptr, "INSERT", "ins_pt")?;
            let extrusion = read_extrusion(entity_ptr, "INSERT");
            let insertion_point = crate::geom::ocs_to_wcs(insertion_point, extrusion);
            let scale = get_field::<Point3D>(entity_ptr, "INSERT", "scale").unwrap_or(Point3D {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
            let rotation = get_field::<f64>(entity_ptr, "INSERT", "rotation").unwrap_or(0.0);

            // ATTRIBs are owned by the INSERT itself -- a separate ownership
            // relationship from BLOCK_HEADER -> entity, walked from the
            // INSERT's own Dwg_Object rather than from entity_ptr (which is
            // the type-specific struct dynapi needs, a different pointer).
            let mut attribs = Vec::new();
            let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
            // Bounded three ways over, because a damaged handle can turn
            // this chain into a ring (an endless walk), point it back at
            // the INSERT (endless recursion through `convert_entity`, which
            // a 512 MB stack did not survive), or run it off the end of the
            // object list (where LibreDWG's own walker dereferences a null
            // -- see `docs/CAVEATS.md`). An INSERT owns ATTRIBs and nothing
            // else, so the walk stops the moment the chain says otherwise;
            // the other two bounds are in [`crate::limits`].
            let mut walked = 0usize;
            while !sub.is_null() && walked < crate::limits::MAX_OWNED_SUBENTITIES {
                walked += 1;
                let sub_type = unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) }
                    as libredwg_sys::DWG_OBJECT_TYPE;
                if sub_type != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB {
                    break;
                }
                if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, sub, depth + 1) }
                {
                    attribs.push(attrib);
                }
                sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
            }

            Entity::Insert(InsertEntity {
                common,
                block_name,
                insertion_point,
                scale,
                rotation,
                extrusion,
                attribs,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF => {
            let start_point = get_field::<Point2D>(entity_ptr, "ATTDEF", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "ATTDEF", "height")?;
            let default_value =
                get_utf8_field(entity_ptr, "ATTDEF", "default_value").unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "ATTDEF", "rotation").unwrap_or(0.0);
            let tag = get_utf8_field(entity_ptr, "ATTDEF", "tag").unwrap_or_default();
            Entity::Attdef(AttdefEntity {
                common,
                start_point,
                text_height,
                default_value,
                rotation,
                tag,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VIEWPORT => {
            let center = get_field::<Point3D>(entity_ptr, "VIEWPORT", "center")?;
            let width = get_field::<f64>(entity_ptr, "VIEWPORT", "width")?;
            let height = get_field::<f64>(entity_ptr, "VIEWPORT", "height")?;
            let status_flag = get_field::<u32>(entity_ptr, "VIEWPORT", "status_flag").unwrap_or(0);
            let on_off = get_field::<u16>(entity_ptr, "VIEWPORT", "on_off").unwrap_or(1);
            // A DWG stores no on/off or id: LibreDWG synthesises them in
            // block order, and the status flag's 0x20000 bit is the real
            // "off". A DXF states both (68: 0 = off; 69: 1 = overall).
            let on = if source.from_dxf {
                on_off != 0
            } else {
                status_flag & 0x20000 == 0
            };
            let frozen_layers: Vec<String> =
                get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
                    entity_ptr,
                    "VIEWPORT",
                    "num_frozen_layers",
                    "frozen_layers",
                )
                .into_iter()
                .filter(|h| !h.is_null())
                .filter_map(|h| resolve_handle_name(dwg, h))
                .collect();
            Entity::Viewport(ViewportEntity {
                common,
                center,
                width,
                height,
                view_center: get_field::<Point2D>(entity_ptr, "VIEWPORT", "VIEWCTR")
                    .unwrap_or_default(),
                view_size: get_field::<f64>(entity_ptr, "VIEWPORT", "VIEWSIZE").unwrap_or(0.0),
                view_target: get_field::<Point3D>(entity_ptr, "VIEWPORT", "view_target")
                    .unwrap_or_default(),
                view_direction: get_field::<Point3D>(entity_ptr, "VIEWPORT", "VIEWDIR")
                    .filter(|d| d.x != 0.0 || d.y != 0.0 || d.z != 0.0)
                    .unwrap_or(crate::geom::WORLD_Z),
                twist: get_field::<f64>(entity_ptr, "VIEWPORT", "VIEWTWIST").unwrap_or(0.0),
                lens_length: get_field::<f64>(entity_ptr, "VIEWPORT", "LENSLENGTH").unwrap_or(0.0),
                status_flag,
                on,
                id: get_field::<u16>(entity_ptr, "VIEWPORT", "id").unwrap_or(0),
                frozen_layers,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DFACE => {
            let corner1 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner1")?;
            let corner2 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner2")?;
            let corner3 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner3")?;
            let corner4 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner4")?;
            Entity::Face3D(Face3DEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SPLINE => {
            // num_fit_pts is BITCODE_BS (u16), unlike most other num_X fields
            // (BITCODE_BL/u32) -- see get_array_field's doc comment.
            let fit_points: Vec<Point3D> =
                get_array_field::<u16, _>(entity_ptr, "SPLINE", "num_fit_pts", "fit_pts");
            let control_points: Vec<Point3D> = get_array_field::<u32, SplineControlPoint>(
                entity_ptr,
                "SPLINE",
                "num_ctrl_pts",
                "ctrl_pts",
            )
            .into_iter()
            .map(Into::into)
            .collect();
            Entity::Spline(SplineEntity {
                common,
                fit_points,
                control_points,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MTEXT => {
            let insertion_point = get_field::<Point3D>(entity_ptr, "MTEXT", "ins_pt")?;
            let text = get_utf8_field(entity_ptr, "MTEXT", "text").unwrap_or_default();
            let text_height = get_field::<f64>(entity_ptr, "MTEXT", "text_height").unwrap_or(1.0);
            // The baseline direction (DXF 11): its angle is the rotation. A
            // zero vector (never written by AutoCAD, but seen in hand-made
            // files) means "no rotation".
            let x_axis_dir = get_field::<Point3D>(entity_ptr, "MTEXT", "x_axis_dir")
                .filter(|d| d.x != 0.0 || d.y != 0.0)
                .unwrap_or(Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                });
            let rotation = x_axis_dir.y.atan2(x_axis_dir.x);
            let line_spacing_factor =
                get_field::<f64>(entity_ptr, "MTEXT", "linespace_factor").unwrap_or(1.0);
            let attachment = get_field::<u16>(entity_ptr, "MTEXT", "attachment")
                .filter(|a| (1..=9).contains(a))
                .unwrap_or(1);
            let rect_width = get_field::<f64>(entity_ptr, "MTEXT", "rect_width").unwrap_or(0.0);
            let extents_width =
                get_field::<f64>(entity_ptr, "MTEXT", "extents_width").unwrap_or(0.0);
            let extents_height =
                get_field::<f64>(entity_ptr, "MTEXT", "extents_height").unwrap_or(0.0);
            let style =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "MTEXT", "style")
                    .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
                    .unwrap_or_default();
            Entity::MText(MTextEntity {
                common,
                insertion_point,
                text_plain: crate::text::decode_mtext(&text).plain,
                text,
                text_height,
                rotation,
                line_spacing_factor,
                attachment,
                rect_width,
                extents_width,
                extents_height,
                x_axis_dir,
                style,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_3D => {
            // SAFETY: obj is a POLYLINE_3D per fixedtype, and the subentities
            // it owns are VERTEX_3D.
            let vertices: Vec<Point3D> = unsafe {
                polyline_vertices(
                    obj,
                    libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D,
                    "VERTEX_3D",
                )
            }
            .into_iter()
            .map(|(point, _no_bulge)| point)
            .collect();
            // POLYLINE_3D.flag is BITCODE_RC (1 byte), unlike LWPOLYLINE's
            // BITCODE_BS (2 bytes) -- same closed-bit convention, different
            // underlying C width.
            let flag = get_field::<u8>(entity_ptr, "POLYLINE_3D", "flag").unwrap_or(0);
            Entity::Polyline3D(PolylineEntity {
                common,
                vertices,
                closed: flag & POLYLINE_CLOSED_FLAG != 0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_2D => {
            // SAFETY: obj is a POLYLINE_2D per fixedtype, and the subentities
            // it owns are VERTEX_2D, each carrying its own point and bulge.
            let owned = unsafe {
                polyline_vertices(
                    obj,
                    libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D,
                    "VERTEX_2D",
                )
            };
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_2D", "flag").unwrap_or(0);
            let extrusion = read_extrusion(entity_ptr, "POLYLINE_2D");
            let elevation = get_field::<f64>(entity_ptr, "POLYLINE_2D", "elevation").unwrap_or(0.0);
            // A VERTEX_2D's stored point is 3D, but its z is the polyline's
            // own elevation repeated; the OCS transform below takes that from
            // the POLYLINE_2D field, as the DXF does.
            let mut bulges: Vec<f64> = owned.iter().map(|(_, bulge)| *bulge).collect();
            if bulges.iter().all(|b| *b == 0.0) {
                bulges.clear();
            }
            mirror_bulges(&mut bulges, extrusion);
            let vertices = owned
                .iter()
                .map(|(point, _)| {
                    crate::geom::ocs_to_wcs_2d(
                        Point2D {
                            x: point.x,
                            y: point.y,
                        },
                        elevation,
                        extrusion,
                    )
                })
                .collect();
            Entity::Polyline2D(LwPolylineEntity {
                common,
                vertices,
                closed: flag & u16::from(POLYLINE_CLOSED_FLAG) != 0,
                bulges,
                widths: Vec::new(),
                const_width: get_field::<f64>(entity_ptr, "POLYLINE_2D", "start_width")
                    .unwrap_or(0.0),
                elevation,
                extrusion,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => {
            let dxfname = dimension_dxfname(fixedtype);
            let block_name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "block")
                    .and_then(crate::tables::resolve_block_name)
                    .unwrap_or_default();
            let p3 =
                |field: &str| get_field::<Point3D>(entity_ptr, dxfname, field).unwrap_or_default();
            let f64_field =
                |field: &str| get_field::<f64>(entity_ptr, dxfname, field).unwrap_or(0.0);
            // For a 2-line angular dimension the two readers fill `def_pt`
            // and `xline2end_pt` the other way round: the DWG decoder
            // follows the stream order (`dwg.spec`: the leading 2RD is the
            // arc point, DXF 16, and `xline2end_pt` gets the last point,
            // DXF 10 -- the second line's end), while the DXF reader maps
            // by group code (dynapi: `def_pt` = 10, `xline2end_pt` = 16).
            // Both paths must end up with the arc point as the definition
            // point (the sector probe) and the DXF 10 point as `line2_end`.
            let ang2ln_swapped = source.from_dxf
                && fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN;
            let definition_point = if ang2ln_swapped {
                p3("xline2end_pt")
            } else {
                p3("def_pt")
            };
            let geometry = match fixedtype {
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR => {
                    DimensionGeometry::Linear {
                        xline1: p3("xline1_pt"),
                        xline2: p3("xline2_pt"),
                        rotation: f64_field("dim_rotation"),
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => {
                    DimensionGeometry::Aligned {
                        xline1: p3("xline1_pt"),
                        xline2: p3("xline2_pt"),
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT => {
                    DimensionGeometry::Angular3Point {
                        center: p3("center_pt"),
                        xline1: p3("xline1_pt"),
                        xline2: p3("xline2_pt"),
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => {
                    DimensionGeometry::Angular2Line {
                        line1_start: p3("xline1start_pt"),
                        line1_end: p3("xline1end_pt"),
                        line2_start: p3("xline2start_pt"),
                        line2_end: if ang2ln_swapped {
                            p3("def_pt")
                        } else {
                            p3("xline2end_pt")
                        },
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS => {
                    DimensionGeometry::Radius {
                        center: definition_point,
                        chord_point: p3("first_arc_pt"),
                        leader_length: f64_field("leader_len"),
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => {
                    DimensionGeometry::Diameter {
                        chord_start: definition_point,
                        chord_end: p3("first_arc_pt"),
                        leader_length: f64_field("leader_len"),
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => {
                    // Which coordinate is dimensioned. A DWG stores it as
                    // bit 1 of the stream-only `flag2` byte; a DXF carries
                    // it as bit 0x40 of group 70, which the DXF reader
                    // stores in `flag` and never copies into `flag2`.
                    let x_datum = if source.from_dxf {
                        get_field::<u8>(entity_ptr, dxfname, "flag").unwrap_or(0) & 0x40 != 0
                    } else {
                        get_field::<u8>(entity_ptr, dxfname, "flag2").unwrap_or(0) & 1 != 0
                    };
                    DimensionGeometry::Ordinate {
                        feature: p3("feature_location_pt"),
                        leader_end: p3("leader_endpt"),
                        x_datum,
                    }
                }
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => DimensionGeometry::Arc {
                    center: p3("center_pt"),
                    xline1: p3("xline1_pt"),
                    xline2: p3("xline2_pt"),
                },
                _ => DimensionGeometry::Unknown,
            };
            // act_measurement is written for R2000+ files; an older one (and
            // a DXF without group 42) leaves it at -1.0 or at 0.0.
            // `usable_stored_measurement` says which values are a
            // measurement of this dimension and converts angular radians to
            // degrees; the definition points stand in for the rest.
            let stored = get_field::<f64>(entity_ptr, dxfname, "act_measurement");
            let measurement_from_points =
                crate::dimension::measurement_from_points(&geometry, definition_point);
            let measurement = crate::dimension::usable_stored_measurement(
                stored,
                &geometry,
                measurement_from_points,
            );
            let user_text = get_utf8_field(entity_ptr, dxfname, "user_text").unwrap_or_default();
            let text_midpoint =
                get_field::<Point2D>(entity_ptr, dxfname, "text_midpt").unwrap_or_default();
            let dimstyle =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "dimstyle")
                    .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
                    .unwrap_or_default();
            // display_* and dimlfac are filled by dimension::attach_display_text
            // once the tables (cached labels, DIMSTYLEs) exist.
            Entity::Dimension(DimensionEntity {
                common,
                block_name,
                geometry,
                measurement,
                measurement_from_points,
                user_text,
                display_text: String::new(),
                display_text_raw: String::new(),
                display_source: DisplaySource::None,
                definition_point,
                text_midpoint,
                dimstyle,
                dimlfac: 1.0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TABLE => {
            // dynapi's field-table key for this type is "TABLE" (dwg.h's
            // internal name), not "ACAD_TABLE" (the DXF name
            // dwg_object_get_dxfname reports) -- passing the latter would fail
            // dwg_dynapi_entity_value's strict obj->name check, the same
            // pitfall as REGION/3DSOLID (see acis.rs).
            let block_name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "TABLE", "block_header")
                    .and_then(crate::tables::resolve_block_name)
                    .unwrap_or_default();
            let insertion_point = get_field::<Point3D>(entity_ptr, "TABLE", "ins_pt")?;
            let scale = get_field::<Point3D>(entity_ptr, "TABLE", "scale").unwrap_or(Point3D {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
            let rotation = get_field::<f64>(entity_ptr, "TABLE", "rotation").unwrap_or(0.0);
            Entity::AcadTable(AcadTableEntity {
                common,
                block_name,
                insertion_point,
                scale,
                rotation,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_HATCH => {
            let solid_fill =
                get_field::<u8>(entity_ptr, "HATCH", "is_solid_fill").unwrap_or(0) != 0;
            let paths: Vec<libredwg_sys::Dwg_HATCH_Path> =
                get_array_field::<u32, _>(entity_ptr, "HATCH", "num_paths", "paths");
            let boundary_paths = paths.iter().map(convert_hatch_path).collect();
            // num_deflines is BITCODE_BS (u16), unlike num_paths' BITCODE_BL.
            let deflines: Vec<libredwg_sys::Dwg_HATCH_DefLine> =
                get_array_field::<u16, _>(entity_ptr, "HATCH", "num_deflines", "deflines");
            let pattern_lines = deflines.iter().map(convert_hatch_defline).collect();
            // is_gradient_fill/single_color_gradient are BITCODE_BL (u32),
            // unlike is_solid_fill (BITCODE_B, u8) above.
            let is_gradient_fill =
                get_field::<u32>(entity_ptr, "HATCH", "is_gradient_fill").unwrap_or(0) != 0;
            let gradient = is_gradient_fill
                .then(|| {
                    let gradient_angle =
                        get_field::<f64>(entity_ptr, "HATCH", "gradient_angle").unwrap_or(0.0);
                    let single_color_gradient =
                        get_field::<u32>(entity_ptr, "HATCH", "single_color_gradient").unwrap_or(0)
                            != 0;
                    let gradient_tint =
                        get_field::<f64>(entity_ptr, "HATCH", "gradient_tint").unwrap_or(0.0);
                    let gradient_name =
                        get_utf8_field(entity_ptr, "HATCH", "gradient_name").unwrap_or_default();
                    let colors: Vec<libredwg_sys::Dwg_HATCH_Color> =
                        get_array_field::<u32, _>(entity_ptr, "HATCH", "num_colors", "colors");
                    convert_hatch_gradient(
                        gradient_angle,
                        single_color_gradient,
                        gradient_tint,
                        &gradient_name,
                        &colors,
                    )
                })
                .flatten();
            Entity::Hatch(HatchEntity {
                common,
                boundary_paths,
                solid_fill,
                gradient,
                pattern_lines,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DSOLID => {
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity__3DSOLID*
            // (checked above), matching fixedtype.
            let wireframe_edges = unsafe { crate::acis::extract_wireframe(entity_ptr, "3DSOLID") };
            Entity::Solid3D(Solid3DEntity {
                common,
                wireframe_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_REGION => {
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_REGION*
            // (checked above, matching fixedtype), which dwg.h typedefs from
            // Dwg_Entity__3DSOLID -- layout-identical, so the cast
            // extract_wireframe does internally is sound. Its real dxfname has
            // to be passed through: dynapi refuses a name mismatch (acis.rs).
            let wireframe_edges = unsafe { crate::acis::extract_wireframe(entity_ptr, "REGION") };
            Entity::Region(Solid3DEntity {
                common,
                wireframe_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_PFACE => {
            // SAFETY: obj is a valid, non-null POLYLINE_PFACE Dwg_Object*
            // (matching fixedtype); the helper only walks its owned-subentity
            // chain.
            let wireframe_edges = unsafe { polyline_pface_wireframe(obj) };
            Entity::PolylinePFace(Solid3DEntity {
                common,
                wireframe_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_MESH => {
            let m = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "num_m_verts").unwrap_or(0);
            let n = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "num_n_verts").unwrap_or(0);
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "flag").unwrap_or(0);
            // SAFETY: obj is a valid, non-null POLYLINE_MESH Dwg_Object*
            // (matching fixedtype); the helper only walks its owned-subentity
            // chain.
            let wireframe_edges =
                unsafe { polyline_mesh_wireframe(obj, usize::from(m), usize::from(n), flag) };
            Entity::PolylineMesh(Solid3DEntity {
                common,
                wireframe_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TOLERANCE => {
            let insertion_point = get_field::<Point3D>(entity_ptr, "TOLERANCE", "ins_pt")?;
            // `height` is only decoded for R13/R14 (`dwg.spec`); every later
            // file leaves it at 0.0 and takes the height from the DIMSTYLE
            // -- dimension::attach_display_text fills that in once the
            // tables exist.
            let text_height = get_field::<f64>(entity_ptr, "TOLERANCE", "height")
                .filter(|h| h.is_finite() && *h > 0.0)
                .unwrap_or(0.0);
            let text_value =
                get_utf8_field(entity_ptr, "TOLERANCE", "text_value").unwrap_or_default();
            let dimstyle =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "TOLERANCE", "dimstyle")
                    .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
                    .unwrap_or_default();
            Entity::Tolerance(ToleranceEntity {
                common,
                insertion_point,
                text_height,
                text_plain: crate::text::decode_text(&text_value).plain,
                text_value,
                dimstyle,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_WIPEOUT => {
            let boundary = wipeout_boundary(entity_ptr);
            Entity::Wipeout(WipeoutEntity { common, boundary })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LIGHT => {
            let position = get_field::<Point3D>(entity_ptr, "LIGHT", "position")?;
            let target = get_field::<Point3D>(entity_ptr, "LIGHT", "target").unwrap_or(position);
            // type: distant=1, point=2, spot=3 (dwg.h, BITCODE_BL) -- only
            // distant/spot actually aim at `target`.
            let light_type = get_field::<u32>(entity_ptr, "LIGHT", "type").unwrap_or(2);
            Entity::Light(LightEntity {
                common,
                position,
                target,
                has_target: light_type != 2 && target != position,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINE => {
            Entity::MLine(convert_mline(dwg, entity_ptr, common))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MULTILEADER => Entity::MultiLeader({
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_MULTILEADER*
            // (checked above), matching fixedtype.
            let lines = unsafe { multileader_lines(entity_ptr) };
            MultiLeaderEntity { common, lines }
        }),
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LEADER => {
            let vertices: Vec<Point3D> =
                get_array_field::<u32, _>(entity_ptr, "LEADER", "num_points", "points");
            // arrowhead_type is BITCODE_BS (u16, not a bit flag): any nonzero
            // value means an arrowhead is drawn.
            let has_arrowhead =
                get_field::<u16>(entity_ptr, "LEADER", "arrowhead_type").unwrap_or(0) > 0;
            Entity::Leader(LeaderEntity {
                common,
                vertices,
                has_arrowhead,
            })
        }
        // SAFETY: obj is valid per this function's own `# Safety` doc contract.
        _ => Entity::Unknown {
            common,
            type_name: unsafe { dxfname(obj) },
        },
    })
}

/// Reads a MULTILEADER's leader lines through the `uncad_multileader_get_lines`
/// shim, which flattens the three struct levels dynapi cannot reach
/// (`ctx.leaders[].lines[].points[]`) into plain `(x, y, z)` arrays.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity_MULTILEADER*`.
unsafe fn multileader_lines(entity_ptr: *mut std::ffi::c_void) -> Vec<Vec<Point3D>> {
    let mut lines_ptr: *mut libredwg_sys::uncad_multileader_line_t = std::ptr::null_mut();
    // SAFETY: entity_ptr is valid per this function's contract; on success the
    // shim mallocs *lines_ptr (num_lines entries, each owning its own points
    // buffer), freed below before returning.
    let num_lines =
        unsafe { libredwg_sys::uncad_multileader_get_lines(entity_ptr, &mut lines_ptr) };
    if lines_ptr.is_null() {
        return Vec::new();
    }

    let raw_lines = unsafe { std::slice::from_raw_parts(lines_ptr, num_lines as usize) };
    let mut lines = Vec::with_capacity(num_lines as usize);
    for line in raw_lines {
        if line.points.is_null() || line.num_points == 0 {
            continue;
        }
        // SAFETY: points is a flat (x,y,z) triple array of num_points*3
        // doubles, per uncad_multileader_get_lines' contract.
        let flat = unsafe { std::slice::from_raw_parts(line.points, line.num_points as usize * 3) };
        lines.push(
            flat.as_chunks::<3>()
                .0
                .iter()
                .map(|&[x, y, z]| Point3D { x, y, z })
                .collect(),
        );
    }
    unsafe { libredwg_sys::uncad_multileader_free_lines(lines_ptr, num_lines) };
    lines
}

/// Reads a raw `(ptr, count)` pair from a *nested* struct field (not reachable
/// through dynapi, which only exposes top-level entity fields by name) as an
/// owned `Vec<T>`.
///
/// # Safety
/// `ptr` must be valid for reads of `count` consecutive `T`s (or null, treated
/// as empty), matching LibreDWG's own num_X/X array convention.
unsafe fn read_raw_array<T: Copy>(ptr: *const T, count: u32) -> Vec<T> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, count as usize) }.to_vec()
}

/// `HATCH.paths[i].flag` bit for "this path is a polyline" (vs. a list of
/// curved/straight edges).
const HATCH_PATH_IS_POLYLINE_FLAG: u32 = 0x02;

/// HATCH boundary edge curve types (`Dwg_HATCH_PathSeg::curve_type`).
const HATCH_EDGE_LINE: u8 = 1;
const HATCH_EDGE_ARC: u8 = 2;
const HATCH_EDGE_ELLIPSE: u8 = 3;
const HATCH_EDGE_SPLINE: u8 = 4;

fn convert_hatch_path(path: &libredwg_sys::Dwg_HATCH_Path) -> HatchBoundaryPath {
    if path.flag & HATCH_PATH_IS_POLYLINE_FLAG != 0 {
        // SAFETY: polyline_paths/num_segs_or_paths are LibreDWG's own matched
        // array-length convention (see Dwg_HATCH_Path in dwg.h); valid until
        // dwg_free, which outlives this whole conversion pass.
        let vertices: Vec<Point2D> =
            unsafe { read_raw_array(path.polyline_paths, path.num_segs_or_paths) }
                .into_iter()
                .map(|v| Point2D {
                    x: v.point.x,
                    y: v.point.y,
                })
                .collect();
        HatchBoundaryPath::Polyline(vertices)
    } else {
        // SAFETY: same as above, for segs.
        let segs = unsafe { read_raw_array(path.segs, path.num_segs_or_paths) };
        let edges = segs.iter().filter_map(convert_hatch_edge).collect();
        HatchBoundaryPath::Edges(edges)
    }
}

fn convert_hatch_edge(seg: &libredwg_sys::Dwg_HATCH_PathSeg) -> Option<HatchEdge> {
    let p2 = |p: libredwg_sys::BITCODE_2RD| Point2D { x: p.x, y: p.y };
    Some(match seg.curve_type {
        HATCH_EDGE_LINE => HatchEdge::Line {
            start: p2(seg.first_endpoint),
        },
        HATCH_EDGE_ARC => HatchEdge::Arc {
            center: p2(seg.center),
            radius: seg.radius,
            start_angle: seg.start_angle,
            end_angle: seg.end_angle,
            is_ccw: seg.is_ccw != 0,
        },
        HATCH_EDGE_ELLIPSE => HatchEdge::Ellipse {
            center: p2(seg.center),
            end: p2(seg.endpoint),
            minor_major_ratio: seg.minor_major_ratio,
            start_angle: seg.start_angle,
            end_angle: seg.end_angle,
            is_ccw: seg.is_ccw != 0,
        },
        HATCH_EDGE_SPLINE => {
            // SAFETY: num_control_points/control_points is the same matched
            // array-length convention as everywhere else in dwg.h.
            let control_points =
                unsafe { read_raw_array(seg.control_points, seg.num_control_points) }
                    .into_iter()
                    .map(|cp| p2(cp.point))
                    .collect();
            HatchEdge::Spline { control_points }
        }
        _ => return None,
    })
}

fn convert_hatch_defline(defline: &libredwg_sys::Dwg_HATCH_DefLine) -> HatchPatternLine {
    // SAFETY: num_dashes/dashes is the same matched array-length convention as
    // everywhere else in dwg.h. num_dashes is BITCODE_BS (unsigned), so the
    // widening cast has no negative-count wraparound to guard against.
    let dash_pattern = unsafe { read_raw_array(defline.dashes, defline.num_dashes as u32) };
    HatchPatternLine {
        angle: defline.angle,
        base_point: Point2D {
            x: defline.pt0.x,
            y: defline.pt0.y,
        },
        offset: Point2D {
            x: defline.offset.x,
            y: defline.offset.y,
        },
        dash_pattern,
    }
}

/// Resolves one gradient stop's `Dwg_Color` to a hex string. Same
/// truecolor-overrides-ACI precedence as [`entity_color`], but a gradient stop
/// is never BYLAYER/BYBLOCK, so it needs no layer or inherited-color context.
fn hatch_stop_color(c: &libredwg_sys::Dwg_HATCH_Color) -> String {
    let true_color = (c.color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR)
        .then_some(c.color.rgb & 0xff_ffff);
    crate::color::true_color_to_hex(true_color)
        .or_else(|| crate::color::aci_to_hex(c.color.index.unsigned_abs()))
        .unwrap_or_else(|| crate::color::DEFAULT_COLOR.to_string())
}

/// Builds a [`HatchGradient`] from the raw `Dwg_Entity_HATCH` gradient fields.
/// `None` when there is no usable color data, in which case the caller falls
/// back to pattern or outline-only rendering.
fn convert_hatch_gradient(
    angle: f64,
    single_color_gradient: bool,
    gradient_tint: f64,
    gradient_name: &str,
    colors: &[libredwg_sys::Dwg_HATCH_Color],
) -> Option<HatchGradient> {
    let (color1, color2) = if single_color_gradient {
        let color1 = hatch_stop_color(colors.first()?);
        let color2 = crate::color::tint_toward_white(&color1, gradient_tint);
        (color1, color2)
    } else if colors.len() >= 2 {
        // shift_value (0.0-1.0) orders the stops; `colors` is not guaranteed
        // to already be sorted by it.
        let mut sorted: Vec<&libredwg_sys::Dwg_HATCH_Color> = colors.iter().collect();
        sorted.sort_by(|a, b| {
            a.shift_value
                .partial_cmp(&b.shift_value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        (
            hatch_stop_color(sorted[0]),
            hatch_stop_color(sorted[sorted.len() - 1]),
        )
    } else {
        let color1 = hatch_stop_color(colors.first()?);
        (color1.clone(), color1)
    };
    let is_radial = matches!(
        gradient_name.to_ascii_uppercase().as_str(),
        "SPHERICAL" | "HEMISPHERICAL"
    );
    Some(HatchGradient {
        is_radial,
        angle,
        color1,
        color2,
    })
}

/// Reads an MLINE's vertices and its MLINESTYLE reference. `mlinestyle_name` is
/// resolved here but only looked up against
/// [`crate::tables::Tables::mlinestyles`] at render time -- `Tables` is not
/// built yet while entities are converted, the same reason BYLAYER color
/// resolution is deferred.
fn convert_mline(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
    common: EntityCommon,
) -> MLineEntity {
    // num_verts is BITCODE_BS (u16), not BITCODE_BL.
    let verts: Vec<libredwg_sys::Dwg_MLINE_vertex> =
        get_array_field::<u16, _>(entity_ptr, "MLINE", "num_verts", "verts");
    let vertices: Vec<MLineVertex> = verts
        .iter()
        .map(|v| MLineVertex {
            point: Point3D {
                x: v.vertex.x,
                y: v.vertex.y,
                z: v.vertex.z,
            },
            miter_direction: Point3D {
                x: v.miter_direction.x,
                y: v.miter_direction.y,
                z: v.miter_direction.z,
            },
        })
        .collect();
    let flags = get_field::<u16>(entity_ptr, "MLINE", "flags").unwrap_or(0);
    let mlinestyle_name =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "MLINE", "mlinestyle")
            .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
            .unwrap_or_default();
    MLineEntity {
        common,
        vertices,
        closed: flags & MLINE_CLOSED_FLAG != 0,
        mlinestyle_name,
    }
}

/// The 7 DIMENSION subtypes share `DIMENSION_COMMON`'s fields (including
/// `block`, the cached-geometry handle), but dynapi is keyed by each subtype's
/// own dxfname -- there is no generic `"DIMENSION"` to pass.
fn dimension_dxfname(fixedtype: libredwg_sys::DWG_OBJECT_TYPE) -> &'static str {
    match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => "DIMENSION_ORDINATE",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR => "DIMENSION_LINEAR",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => "DIMENSION_ALIGNED",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT => "DIMENSION_ANG3PT",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => "DIMENSION_ANG2LN",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS => "DIMENSION_RADIUS",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => "DIMENSION_DIAMETER",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => "ARC_DIMENSION",
        _ => unreachable!("dimension_dxfname called with a non-DIMENSION fixedtype"),
    }
}

/// `Dwg_Color.flag` bit: an inline 24-bit RGB follows in `rgb`. The *only*
/// statement an R2004+ DWG entity makes about a true colour -- `bit_read_ENC`
/// (bits.c), which is what `common_entity_data.spec` uses from R2004a on,
/// reads `rgb` under this bit and never assigns `method` at all.
const COLOR_FLAG_INLINE_RGB: u16 = 0x80;

/// `Dwg_Color.flag` bit: a DBCOLOR object handle follows *instead of* an
/// inline RGB. That object is not converted, so there is no colour to report
/// and whatever `rgb` holds is not it.
const COLOR_FLAG_COLOR_HANDLE: u16 = 0x40;

/// Reads the common `color` (`Dwg_Color`) field and splits it into
/// `(color_index, true_color)` per [`EntityCommon`].
fn entity_color(entity_ptr: *mut std::ffi::c_void) -> (i16, Option<u32>) {
    let Some(color) = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color") else {
        return (256, None); // no color field at all -- BYLAYER default
    };
    split_entity_color(color.index, color.flag, color.method, color.rgb)
}

/// Decides whether a `Dwg_Color` read off an *entity* really states a direct
/// RGB, from the three fields the two readers fill differently. Until 0.3.0
/// this tested `method == DWG_COLOR_METHOD_TRUECOLOR` alone, which was wrong
/// in both directions:
///
/// * **DWG, R2004+** -- `bit_read_ENC` puts the 420 value in `rgb` under
///   `flag & 0x80` and leaves `method` at 0, so a real true colour was
///   dropped and every entity in every corpus DWG reported `None`.
/// * **DXF** -- `dxf_set_CMC_index` (in_dxf.c) answers a plain group 62 with
///   `method = 0xc3` and an `rgb` *synthesised* from LibreDWG's own copy of
///   the ACI palette, so an entity that states only an index was reported as
///   carrying an RGB the file never wrote. A real group 420 instead takes the
///   `color.method = value >> 24` path, which is 0 for a plain 24-bit value.
///
/// Two cases stay indistinguishable from the fields available and are
/// reported as "no true colour", which renders identically either way: a
/// group 420 of pure black (`rgb` 0 is also what an untouched field holds),
/// and a group 420 that repeats the entity's own ACI colour exactly.
fn split_entity_color(
    index: i16,
    flag: u16,
    method: libredwg_sys::Dwg_Color_Method,
    rgb: u32,
) -> (i16, Option<u32>) {
    let rgb24 = rgb & 0xff_ffff;
    if flag & COLOR_FLAG_COLOR_HANDLE != 0 {
        return (index, None);
    }
    if flag & COLOR_FLAG_INLINE_RGB != 0 {
        return (index, Some(rgb24));
    }
    // The DXF reader's method byte. VOID (0) is a plain 24-bit group 420;
    // ACI (0xc2) and TRUECOLOR (0xc3) are a pre-tagged one -- except that
    // 0xc2 with no RGB is how it spells BYLAYER and 0xc3 with the palette's
    // own entry is how it spells a plain group 62.
    let tagged_rgb = matches!(
        method,
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_VOID
            | libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI
            | libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR
    );
    let from_palette = usize::try_from(index)
        .ok()
        .and_then(|i| crate::color::ACI_PALETTE.get(i))
        .is_some_and(|&packed| packed == rgb24);
    (
        index,
        (tagged_rgb && rgb24 != 0 && !from_palette).then_some(rgb24),
    )
}

/// # Safety
/// `obj` must be a valid, non-null pointer from `dwg_get_object`.
unsafe fn entity_handle(obj: *mut libredwg_sys::Dwg_Object) -> String {
    let mut error = 0i32;
    let handle_ptr = unsafe { libredwg_sys::dwg_object_get_handle(obj, &mut error) };
    if handle_ptr.is_null() || error != 0 {
        return String::new();
    }
    let value = unsafe { (*handle_ptr).value };
    format!("{value:X}")
}

/// # Safety
/// `obj` must be a valid, non-null pointer from `dwg_get_object`.
unsafe fn dxfname(obj: *mut libredwg_sys::Dwg_Object) -> String {
    let ptr = unsafe { libredwg_sys::dwg_object_get_dxfname(obj) };
    if ptr.is_null() {
        return "UNKNOWN".to_string();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOID: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_VOID;
    const BYLAYER: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_BYLAYER;
    const BYBLOCK: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_BYBLOCK;
    const ACI: libredwg_sys::Dwg_Color_Method = libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI;
    const TRUECOLOR: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR;

    /// Each case below is the `(index, flag, method, rgb)` LibreDWG leaves in
    /// `Dwg_Color` for one way of writing a colour, read off the reader that
    /// writes it: `bit_read_ENC` / `common_entity_data.spec` for the DWG
    /// rows and `dxf_set_CMC_index` / the group-420 arm of the common-entity
    /// loop in `in_dxf.c` for the DXF ones.
    #[test]
    fn split_entity_color_follows_what_each_reader_actually_stores() {
        // --- R2004+ DWG (bit_read_ENC): flag 0x80 says an RGB follows, and
        // nothing ever sets `method` on this path. The pre-0.3.0 test
        // (method == TRUECOLOR) made this case report None, which is why no
        // entity in any corpus DWG carried a true colour.
        assert_eq!(
            split_entity_color(256, 0x80, VOID, 0x00_ff7f),
            (256, Some(0x00_ff7f))
        );
        // The stored BL may carry a method byte of its own; only the low 24
        // bits are the colour (which is what the DXF writer emits for 420).
        assert_eq!(
            split_entity_color(256, 0x80, VOID, 0xc200_ff7f),
            (256, Some(0x00_ff7f))
        );
        // flag 0x40 is a DBCOLOR handle *instead of* an inline RGB (the spec
        // reads one or the other), and that object is not converted.
        assert_eq!(split_entity_color(256, 0xc0, VOID, 0x00_ff7f), (256, None));
        // Plain BYLAYER: no flag bits, rgb zeroed by the decoder.
        assert_eq!(split_entity_color(256, 0, VOID, 0), (256, None));

        // --- DXF group 62 only (dxf_set_CMC_index): method 0xc3 with `rgb`
        // synthesised from LibreDWG's ACI palette. ACI 1 is 0xff0000.
        assert_eq!(
            split_entity_color(1, 0, TRUECOLOR, 0xc300_0000 | 0xff_0000),
            (1, None),
            "an index-only entity states no RGB"
        );
        // ... and the same for BYLAYER / BYBLOCK / none, which that function
        // spells with an empty rgb.
        assert_eq!(
            split_entity_color(256, 0, ACI, 0xc200_0000),
            (256, None),
            "0xc2 with no RGB is how the DXF reader spells BYLAYER"
        );
        assert_eq!(split_entity_color(0, 0, BYBLOCK, 0xc100_0000), (0, None));
        assert_eq!(
            split_entity_color(256, 0, BYLAYER, 0xc000_0000),
            (256, None)
        );

        // --- DXF group 420. The common-entity arm stores the value verbatim
        // and takes the method from its top byte, so a plain 24-bit RGB
        // arrives with method 0 and a pre-tagged one with 0xc2 or 0xc3.
        // 65407 == 0x00ff7f is the reviewer's (0, 255, 127).
        assert_eq!(
            split_entity_color(256, 0, VOID, 65407),
            (256, Some(0x00_ff7f))
        );
        assert_eq!(
            split_entity_color(3, 0, ACI, 0xc200_ff7f),
            (3, Some(0x00_ff7f)),
            "a 420 override wins over the entity's own group 62"
        );
        assert_eq!(
            split_entity_color(3, 0, TRUECOLOR, 0xc300_ff7f),
            (3, Some(0x00_ff7f))
        );
        // Group 420 of 257 is the reader's "none": method 0xc8, rgb 0.
        assert_eq!(
            split_entity_color(
                256,
                0,
                libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_NONE,
                0xc800_0000
            ),
            (256, None)
        );
    }

    #[test]
    fn mirror_bulges_negates_every_bulge_under_a_mirrored_ocs_only() {
        // Shared by the LWPOLYLINE and POLYLINE_2D arms: the OCS-to-world
        // map for (0,0,-1) is the reflection x -> -x, which turns a
        // counter-clockwise arc clockwise, so tan(theta / 4) flips sign; a
        // zero stays a plain 0 (not -0), and an upright OCS changes nothing.
        let mut bulges = vec![0.0, 0.41421356, -1.0, 0.0];
        mirror_bulges(
            &mut bulges,
            Point3D {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
        );
        assert_eq!(bulges, [0.0, -0.41421356, 1.0, 0.0]);
        assert!(bulges.iter().all(|b| *b != 0.0 || !b.is_sign_negative()));
        let mut upright = vec![0.0, 0.41421356];
        mirror_bulges(&mut upright, crate::geom::WORLD_Z);
        assert_eq!(upright, [0.0, 0.41421356]);
    }

    fn aci_stop(shift_value: f64, index: i16) -> libredwg_sys::Dwg_HATCH_Color {
        libredwg_sys::Dwg_HATCH_Color {
            parent: std::ptr::null_mut(),
            shift_value,
            color: libredwg_sys::Dwg_Color {
                index,
                method: libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI,
                ..Default::default()
            },
        }
    }

    #[test]
    fn hatch_stop_color_prefers_truecolor_over_aci_index() {
        let stop = libredwg_sys::Dwg_HATCH_Color {
            parent: std::ptr::null_mut(),
            shift_value: 0.0,
            color: libredwg_sys::Dwg_Color {
                index: 2, // yellow, should be ignored
                method: libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR,
                rgb: 0x00ff00,
                ..Default::default()
            },
        };
        assert_eq!(hatch_stop_color(&stop), "#00ff00");
    }

    #[test]
    fn hatch_stop_color_falls_back_to_aci_index() {
        assert_eq!(hatch_stop_color(&aci_stop(0.0, 1)), "#ff0000"); // ACI 1 = red
    }

    #[test]
    fn gradient_two_color_orders_stops_by_shift_value_not_array_order() {
        // colors[0] is the *second* stop (shift_value 1.0); the array is not
        // guaranteed to already be sorted.
        let colors = [aci_stop(1.0, 5), aci_stop(0.0, 1)];
        let g = convert_hatch_gradient(0.0, false, 0.0, "LINEAR", &colors).unwrap();
        assert_eq!(g.color1, "#ff0000"); // ACI 1, shift 0.0
        assert_eq!(g.color2, "#0000ff"); // ACI 5, shift 1.0
        assert!(!g.is_radial);
    }

    #[test]
    fn gradient_single_color_tints_toward_white_for_second_stop() {
        let colors = [aci_stop(0.0, 7)]; // ACI 7 -> normalized to black
        let g = convert_hatch_gradient(0.0, true, 0.5, "LINEAR", &colors).unwrap();
        assert_eq!(g.color1, "#000000");
        assert_eq!(g.color2, "#808080");
    }

    #[test]
    fn gradient_name_spherical_and_hemispherical_are_radial_case_insensitive() {
        let colors = [aci_stop(0.0, 1), aci_stop(1.0, 5)];
        assert!(
            convert_hatch_gradient(0.0, false, 0.0, "spherical", &colors)
                .unwrap()
                .is_radial
        );
        assert!(
            convert_hatch_gradient(0.0, false, 0.0, "HEMISPHERICAL", &colors)
                .unwrap()
                .is_radial
        );
        assert!(
            !convert_hatch_gradient(0.0, false, 0.0, "CYLINDER", &colors)
                .unwrap()
                .is_radial
        );
        assert!(
            !convert_hatch_gradient(0.0, false, 0.0, "", &colors)
                .unwrap()
                .is_radial
        );
    }

    #[test]
    fn gradient_with_no_colors_returns_none() {
        assert!(convert_hatch_gradient(0.0, false, 0.0, "LINEAR", &[]).is_none());
        assert!(convert_hatch_gradient(0.0, true, 0.0, "LINEAR", &[]).is_none());
    }
}
