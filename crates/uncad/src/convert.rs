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
    get_array_field, get_common_field, get_field, get_point2d, get_point2d_array, get_point3d,
    get_point3d_array, is_from_dxf, is_pre_r13, is_r2000_or_later, is_r2010_or_later,
    is_r2013_or_later, RawSegmentWidth, SplineControlPoint,
};
use crate::table_convert::LINEWEIGHTS;
use crate::text::TextDecoder;
use std::ffi::CStr;
use uncad_model::model::{
    AcadTableEntity, ArcEntity, AttdefEntity, AttribEntity, CircleEntity, Confidence,
    DimensionEntity, DimensionKind, DimensionPoints, EllipseEntity, Entity, EntityCommon, EntityId,
    Face3DEntity, HatchBoundaryPath, HatchEdge, HatchEntity, HatchGradient, HatchPatternLine,
    ImageEntity, InsertEntity, LeaderAnnotation, LeaderEntity, LeaderPath, LightEntity, LightType,
    LineEntity, LwPolylineEntity, MLineEntity, MLineVertex, MTextAttachment, MTextEntity,
    MultiLeaderEntity, OrdinateAxis, Origin, PointEntity, PolylineEntity, RayEntity, Ref,
    Solid3DEntity, SolidEntity, SplineEntity, TextEntity, TextOverride, ToleranceEntity,
    ViewportEntity, ViewportView, WipeoutEntity,
};
use uncad_model::model::{
    AttributeFlags, EntityLinetype, HorizontalJustification, OverrideValue, Point2D, Point3D,
    PolylineVertex, StyleOverride, VerticalJustification,
};

/// The `flag` bit that means "closed" on POLYLINE_2D and POLYLINE_3D: bit 1,
/// as in DXF group 70 and as `dwg.h` documents for `Dwg_Entity_POLYLINE_2D`.
const POLYLINE_CLOSED_FLAG: u16 = 1;

/// The `flag` bit that means "closed" on LWPOLYLINE: **512**, not 1. The
/// library stores LWPOLYLINE's flag in its DWG layout, where bit 1 means
/// "has extrusion" and 512 means closed (`dwg.h`, `Dwg_Entity_LWPOLYLINE`);
/// its DXF importer maps group 70 bit 1 onto 512 accordingly. Reading bit 1
/// here reported every closed LWPOLYLINE as open -- across the whole corpus
/// (1,137 of them) not one came back closed. Caught by a synthetic drawing
/// whose spec said "closed" and whose outline came back as an open polyline.
const LWPOLYLINE_CLOSED_FLAG: u16 = 512;

/// The LWPOLYLINE `flag` bit that says the record stores an extrusion. The
/// library reads the extrusion only when it is set and leaves the field zero
/// otherwise -- a zero vector, which names no plane, where the format means
/// the default one. A drawing's every LWPOLYLINE came back that way until a
/// field-by-field comparison with the same drawing saved as DXF caught it.
const LWPOLYLINE_EXTRUSION_FLAG: u16 = 1;

/// POLYLINE_MESH's `flag` bit 1: the grid wraps in M ("closed polygon mesh
/// in the M direction", DXF group 70).
const POLYLINE_MESH_CLOSED_M_FLAG: u16 = 1;

/// POLYLINE_MESH's `flag` bit 32: the grid wraps in N.
const POLYLINE_MESH_CLOSED_N_FLAG: u16 = 32;

/// The default normal of an object coordinate system (DXF 210): the world's
/// z axis, which makes the OCS the world's own axes.
const Z_AXIS: Point3D = Point3D {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// VIEWPORT's status-flag bit (DXF 90) that says the viewport is off.
const VIEWPORT_OFF_FLAG: u32 = 0x20000;

/// `MLINE_FLAGS_CLOSED` (dwg.h).
const MLINE_CLOSED_FLAG: u16 = 2;

/// How deep the conversion of an entity's owned *sub*entities may recurse.
///
/// Only one kind of nesting is real: an INSERT owns its ATTRIBs, which are
/// entities themselves, so converting an INSERT converts them too (depth 1;
/// an entity a block owns is depth 0). A file whose handles are damaged can
/// point an INSERT's attribute chain back at the INSERT, or around a cycle
/// of them, and the conversion then recurses until the stack runs out -- a
/// 512 MB stack did not survive one byte-flipped `example_2000.dwg`. One
/// level is what the format has; two is the cap.
const MAX_SUBENTITY_DEPTH: u32 = 2;

/// How many subentities one owned-subentity walk may hand back.
///
/// The same damaged handles can make a chain a *ring* instead of a list,
/// which is not recursion but a loop that never ends. No real entity owns
/// anything like this many: the largest polyline in the corpus has a few
/// thousand vertices.
const MAX_OWNED_SUBENTITIES: usize = 100_000;

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
pub unsafe fn convert_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
) -> Vec<Entity> {
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
        let Some(name) = text.field(object_ptr, "BLOCK_HEADER", "name") else {
            continue;
        };
        if !is_model_space(&name) && !is_paper_space(&name) {
            continue;
        }

        // Each INSERT's attribs are duplicated as top-level Entity::Attrib
        // entries because that is what rendering draws -- after the INSERT,
        // in file order (the ATTRIBs follow their INSERT in the file).
        // Deliberately here and not inside owned_entities(): a block
        // record's own entity list must not carry the duplication (see
        // uncad_model::tables::BlockRecord).
        for entity in unsafe { owned_entities(dwg, text, block_obj) } {
            let attribs: Vec<Entity> = match &entity {
                Entity::Insert(insert) => {
                    insert.attribs.iter().cloned().map(Entity::Attrib).collect()
                }
                _ => Vec::new(),
            };
            entities.push(entity);
            entities.extend(attribs);
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
/// A polyline's VERTEX records are skipped: they belong to the POLYLINE that
/// owns them and are read from *its* chain (see [`polyline_vertex_records`]),
/// not drawing content of the block. That is the contract LibreDWG
/// documents for `get_next_owned_entity` ("Not subentities: ATTRIB,
/// VERTEX") and what the R13..R2000 walk here implements -- but the
/// library's walker for other versions hands back whatever its list holds,
/// and the DXF importer fills that list with every object between a BLOCK
/// and its ENDBLK. Each polyline's vertices came back as entities of their
/// own: in a drawing older than R13, one `VERTEX_2D` "entity" per vertex.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` `block_obj` was obtained from;
/// `block_obj` must be a valid, non-null `BLOCK_HEADER` object.
pub(crate) unsafe fn owned_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<Entity> {
    if unsafe { libredwg_sys::uncad_dwg_is_r13_to_r2000(dwg) } != 0 {
        return unsafe { chained_block_entities(dwg, text, block_obj) };
    }
    let mut entities = Vec::new();
    let mut owned = unsafe { libredwg_sys::get_first_owned_entity(block_obj) };
    while !owned.is_null() {
        let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(owned) }
            as libredwg_sys::DWG_OBJECT_TYPE;
        if is_polyline_vertex(fixedtype) {
            owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
            continue;
        }
        if let Some(entity) = unsafe { convert_entity(dwg, text, owned, 0) } {
            entities.push(entity);
        }
        owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
    }
    entities
}

/// The object a handle reference points at: the pointer the reference
/// already carries, or a lookup by handle when it carries none -- which is
/// the state the DXF importer leaves `first_attrib`/`last_attrib` in.
///
/// # Safety
/// `dwg` must be live, and `reference` either null or a valid
/// `Dwg_Object_Ref` belonging to it.
unsafe fn referenced_object(
    dwg: *mut libredwg_sys::Dwg_Data,
    reference: *mut libredwg_sys::Dwg_Object_Ref,
) -> *mut libredwg_sys::Dwg_Object {
    if reference.is_null() {
        return std::ptr::null_mut();
    }
    let reference = unsafe { &*reference };
    if !reference.obj.is_null() {
        // bindgen names the pointee differently in the two declarations
        // (`_dwg_object` here, the opaque `Dwg_Object` blob elsewhere); it is
        // the same C struct.
        return reference.obj.cast();
    }
    if reference.absolute_ref == 0 {
        return std::ptr::null_mut();
    }
    unsafe { libredwg_sys::dwg_resolve_handle(dwg, reference.absolute_ref) }.cast()
}

/// `true` for the entity kinds that belong to another entity (an INSERT's
/// attributes, a polyline's vertices, the SEQEND that closes either) rather
/// than to the block that owns that entity. They sit in the block's chain,
/// so a chain walk has to step over them. ATTDEF is *not* one of these: an
/// attribute definition is an ordinary block-owned entity.
fn is_sub_entity(fixedtype: libredwg_sys::DWG_OBJECT_TYPE) -> bool {
    fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND
        || is_polyline_vertex(fixedtype)
}

/// `true` for the five VERTEX kinds an old-style POLYLINE owns -- the list
/// LibreDWG's own `get_next_owned_entity` skips ("Not subentities: ATTRIB,
/// VERTEX", dwg.c).
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

/// The entities of a block in an R13..R2000 drawing, where the library links
/// them as a `first_entity` .. `last_entity` chain through each entity's
/// `next_entity`.
///
/// This crate walks the chain itself instead of calling the library's
/// `get_next_owned_entity`, because that walker treats ATTDEF as a
/// sub-entity and skips it -- every attribute definition in a block
/// definition but the last was lost, with no diagnostic. Measured on a
/// synthetic R2000 DXF with three ATTDEFs in one block: one came back.
///
/// # Safety
/// Same contract as [`owned_entities`].
unsafe fn chained_block_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    let header_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(block_obj) };
    let first =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(header_ptr, "BLOCK_HEADER", "first_entity")
            .unwrap_or(std::ptr::null_mut());
    let last =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(header_ptr, "BLOCK_HEADER", "last_entity")
            .unwrap_or(std::ptr::null_mut());
    let last_obj = unsafe { referenced_object(dwg, last) };
    let mut obj = unsafe { referenced_object(dwg, first) };

    // The chain is data from the file; a cycle in it must not hang the
    // parse. No chain can be longer than the object table.
    let max_steps = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut steps = 0;
    while !obj.is_null() && steps <= max_steps {
        steps += 1;
        let fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) } as libredwg_sys::DWG_OBJECT_TYPE;
        if !is_sub_entity(fixedtype) {
            if let Some(entity) = unsafe { convert_entity(dwg, text, obj, 0) } {
                entities.push(entity);
            }
        }
        if obj == last_obj {
            break;
        }
        obj = unsafe { libredwg_sys::dwg_next_entity(obj) };
    }
    entities
}

/// An INSERT's attributes in an R13..R2000 drawing, walked here rather than
/// through the library's `get_first_owned_subentity`.
///
/// Two sources, tried in order. The `attribs[]` array (`num_owned` long) is
/// what the DXF importer fills correctly; its `first_attrib`/`last_attrib`
/// links are not usable after an import (measured: `first_attrib` with a zero
/// handle and no object, `last_attrib` pointing at an unrelated object), and
/// the library's walker reads `first_attrib->obj` without resolving it, so
/// every attribute of an imported INSERT was lost. A drawing decoded from
/// DWG carries the chain and no array, so the chain is the fallback.
///
/// Either source is data from the file, so both are bounded: only ATTRIB
/// objects are converted (an INSERT owns nothing else, and converting what
/// a damaged reference points at instead -- the INSERT itself, say -- is
/// endless recursion), at `depth + 1`, and neither yields more than
/// [`MAX_OWNED_SUBENTITIES`].
///
/// # Safety
/// `dwg` must be live and `entity_ptr` the type-specific struct pointer of a
/// valid INSERT object of it, converted at `depth`.
unsafe fn chained_insert_attribs(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
    depth: u32,
) -> Vec<AttribEntity> {
    let mut attribs = Vec::new();
    let owned = get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
        entity_ptr,
        "INSERT",
        "num_owned",
        "attribs",
    );
    if !owned.is_empty() {
        for reference in owned.into_iter().take(MAX_OWNED_SUBENTITIES) {
            let sub = unsafe { referenced_object(dwg, reference) };
            if sub.is_null() || !unsafe { is_attrib(sub) } {
                continue;
            }
            if let Some(Entity::Attrib(attrib)) =
                unsafe { convert_entity(dwg, text, sub, depth + 1) }
            {
                attribs.push(attrib);
            }
        }
        return attribs;
    }
    let first =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "INSERT", "first_attrib")
            .unwrap_or(std::ptr::null_mut());
    let last = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "INSERT", "last_attrib")
        .unwrap_or(std::ptr::null_mut());
    let last_obj = unsafe { referenced_object(dwg, last) };
    let mut sub = unsafe { referenced_object(dwg, first) };
    let max_steps = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut steps = 0;
    while !sub.is_null() && steps <= max_steps && attribs.len() < MAX_OWNED_SUBENTITIES {
        steps += 1;
        if !unsafe { is_attrib(sub) } {
            break;
        }
        if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, text, sub, depth + 1) } {
            attribs.push(attrib);
        }
        if sub == last_obj {
            break;
        }
        sub = unsafe { libredwg_sys::dwg_next_entity(sub) };
    }
    attribs
}

/// An INSERT's attributes from R2004 on, where the library keeps them as an
/// owned array its own subentity walker resolves correctly -- walked from
/// the INSERT's own `Dwg_Object`, not from its type-specific struct.
///
/// Bounded three ways, because damaged handles can turn the chain into a
/// ring (an endless walk), point it back at the INSERT (endless recursion
/// through [`convert_entity`]) or run it off the end of the object list: an
/// INSERT owns ATTRIBs and nothing else, so the walk stops the moment the
/// chain says otherwise; it converts at `depth + 1`; and it hands back at
/// most [`MAX_OWNED_SUBENTITIES`].
///
/// # Safety
/// `dwg` must be live and `obj` a valid INSERT object of it, converted at
/// `depth`.
unsafe fn owned_insert_attribs(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
    depth: u32,
) -> Vec<AttribEntity> {
    let mut attribs = Vec::new();
    let mut walked = 0usize;
    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    while !sub.is_null() && walked < MAX_OWNED_SUBENTITIES {
        walked += 1;
        if !unsafe { is_attrib(sub) } {
            break;
        }
        if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, text, sub, depth + 1) } {
            attribs.push(attrib);
        }
        sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
    }
    attribs
}

/// # Safety
/// `obj` must be a valid, non-null `Dwg_Object`.
unsafe fn is_attrib(obj: *mut libredwg_sys::Dwg_Object) -> bool {
    // Cast for cross-platform bindgen enum-width consistency -- see
    // convert_entities().
    (unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) } as libredwg_sys::DWG_OBJECT_TYPE)
        == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB
}

/// A POLYLINE's VERTEX records, in order -- a 2D or 3D polyline's vertices,
/// and a polyface's or a polygon mesh's records alike (a polyface keeps its
/// face records in the same run).
///
/// Before R13 a polyline's vertices simply follow it in the object stream,
/// up to its SEQEND. From R13 on the polyline owns them, and LibreDWG's own
/// owned-subentity walk returns them -- through `first_vertex..last_vertex`,
/// last included, up to R2000, and through the `vertex` handle array after.
///
/// LibreDWG's dedicated point accessors (`dwg_object_polyline_{2,3}d_get_points`)
/// are not used: from R13 to R2000 their loop stops *before* `last_vertex`,
/// so a polyline arrived one vertex short -- a file's DXF twin writes the
/// vertex they drop.
///
/// A polyline from R13 on can also own none of the records that follow it:
/// the DXF importer attaches a VERTEX to its polyline by the owner the
/// VERTEX names (group 330), and a file whose vertices name the block record
/// instead -- the shape ezdxf writes for a polyface and a polygon mesh --
/// leaves the polyline's own list empty. Its records are then the ones that
/// follow it in the object stream, as before R13.
///
/// The records are data from the file, so both walks are bounded: the one
/// through the object stream by the number of objects, the owned one by
/// [`MAX_OWNED_SUBENTITIES`].
///
/// When the records in the object stream end before the polyline's SEQEND
/// the read reports `POLYLINE_VERTICES`, naming the polyline. Before R13 they
/// may: when the entity section continues elsewhere (a JUMP entity), the
/// library's object stream can end at the JUMP, before the vertices that
/// follow it in the file -- the polyline then has fewer vertices than the
/// file gives it.
///
/// # Safety
/// `obj` must be a valid `POLYLINE_2D`/`POLYLINE_3D`/`POLYLINE_PFACE`/
/// `POLYLINE_MESH` object of `dwg`.
unsafe fn polyline_vertex_records(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<*mut libredwg_sys::Dwg_Object> {
    let mut records = Vec::new();
    if !is_pre_r13(dwg) {
        let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
        while !sub.is_null() && records.len() < MAX_OWNED_SUBENTITIES {
            records.push(sub);
            sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
        }
        if !records.is_empty() {
            return records;
        }
    }
    let max_steps = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut sub = unsafe { libredwg_sys::dwg_next_object(obj) };
    let mut steps = 0;
    while !sub.is_null() && steps <= max_steps {
        steps += 1;
        let fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND {
            return records;
        }
        // Anything else is not this polyline's: its vertex run ended
        // without its SEQEND.
        if !is_polyline_vertex(fixedtype) {
            break;
        }
        records.push(sub);
        sub = unsafe { libredwg_sys::dwg_next_object(sub) };
    }
    let (id, _) = unsafe { entity_identity(obj) };
    text.warn(format!(
        "POLYLINE_VERTICES: the vertex records of the POLYLINE {:X} end before its SEQEND; it is read with the {} vertices that were found",
        id.value(),
        records.len()
    ));
    records
}

/// The positions (and whatever else `read` takes) of a polyline's vertex
/// records of type `vertex_type`, in order. A record of another type -- a
/// polyface's face record, say -- is not a vertex and is skipped.
///
/// # Safety
/// As [`polyline_vertex_records`].
unsafe fn polyline_vertices<T>(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
    vertex_type: libredwg_sys::DWG_OBJECT_TYPE,
    mut read: impl FnMut(*mut std::ffi::c_void) -> Option<T>,
) -> Vec<T> {
    unsafe { polyline_vertex_records(dwg, text, obj) }
        .into_iter()
        .filter_map(|sub| {
            let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) }
                as libredwg_sys::DWG_OBJECT_TYPE;
            let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
            (fixedtype == vertex_type && !entity_ptr.is_null())
                .then(|| read(entity_ptr))
                .flatten()
        })
        .collect()
}

/// Pairs a polyline's vertex positions with the bulges its record stores as a
/// separate array. The array is empty when every segment is straight, and
/// otherwise has one entry per vertex; any other length does not say which
/// bulge belongs to which vertex, so none is used and the read says so.
fn with_bulges(
    text: &TextDecoder,
    dxfname: &str,
    points: Vec<Point2D>,
    bulges: Vec<f64>,
) -> Vec<PolylineVertex> {
    if !bulges.is_empty() && bulges.len() != points.len() {
        text.warn(format!(
            "POLYLINE_BULGE: a {dxfname} stores {} bulges for {} vertices; its segments are read as straight",
            bulges.len(),
            points.len()
        ));
    }
    let matched = bulges.len() == points.len();
    points
        .into_iter()
        .enumerate()
        .map(|(i, point)| PolylineVertex {
            point,
            bulge: if matched { bulges[i] } else { 0.0 },
            ..PolylineVertex::default()
        })
        .collect()
}

/// Gives an LWPOLYLINE's vertices the widths its record stores as a separate
/// array, as [`with_bulges`] does the bulges: the array is empty when no
/// vertex has a width of its own, and otherwise has one entry per vertex;
/// any other length does not say which width belongs to which vertex, so
/// none is used and the read says so.
///
/// A file that states the polyline's constant width again on every vertex,
/// at both ends, draws the same polyline as one that states it only once
/// (see `LwPolylineEntity::const_width`), so such vertices are given no
/// width of their own either.
fn with_widths(
    text: &TextDecoder,
    dxfname: &str,
    mut vertices: Vec<PolylineVertex>,
    widths: Vec<RawSegmentWidth>,
    const_width: f64,
) -> Vec<PolylineVertex> {
    if widths.is_empty() {
        return vertices;
    }
    if widths.len() != vertices.len() {
        text.warn(format!(
            "POLYLINE_WIDTH: a {dxfname} stores {} widths for {} vertices; its vertices are read with no width of their own",
            widths.len(),
            vertices.len()
        ));
        return vertices;
    }
    if widths
        .iter()
        .all(|w| w.start == const_width && w.end == const_width)
    {
        return vertices;
    }
    for (vertex, width) in vertices.iter_mut().zip(widths) {
        vertex.start_width = width.start;
        vertex.end_width = width.end;
    }
    vertices
}

/// An entity's extrusion direction (DXF 210): the normal of its plane, and
/// for an entity stored in its own coordinate system the Z axis of that
/// system. The default is the world Z axis.
///
/// That default is also what a zero vector reads as. A zero vector is not a
/// direction, and it is what the library leaves in the field when the
/// record does not store one: an LWPOLYLINE stores its extrusion only when
/// its flag says so, and a pre-R13 entity only when its options say so --
/// both are the absent group. Not normalised: the model carries what the
/// file states, and taking the entity's coordinates to the world through it
/// is the consumer's step.
fn extrusion(entity_ptr: *mut std::ffi::c_void, dxfname: &str) -> Point3D {
    get_point3d(entity_ptr, dxfname, "extrusion")
        .filter(|e| e.x != 0.0 || e.y != 0.0 || e.z != 0.0)
        .unwrap_or(Z_AXIS)
}

/// Resolves a POLYLINE_PFACE's mesh into wireframe edges by walking its owned
/// subentities -- vertex positions, in order, then `VERTEX_PFACE_FACE` (up to
/// 4 vertex indices per face, 1-based, negative meaning "invisible edge" --
/// the sign carries no other meaning, so it is just dropped) -- directly;
/// LibreDWG's own accessor for this type is documented as not implemented.
///
/// A position vertex is a `VERTEX_PFACE` *or* a `VERTEX_MESH`. Both mean the
/// same thing inside a POLYLINE_PFACE's own chain, and the DXF importer hands
/// back the second one for a polyface whose `AcDbPolyFaceMeshVertex` records
/// name the block record as their owner rather than the POLYLINE: `in_dxf.c`
/// picks between the two types by looking the VERTEX's own group 330 up and
/// asking whether it is a POLYLINE_PFACE, and falls back to VERTEX_MESH when
/// it is not. ezdxf writes exactly that shape (and its `audit()` passes it),
/// and such a polyface found no positions at all.
///
/// Face records may be interleaved with vertex records, so indices are only
/// resolved once the whole chain has been walked. A face with fewer than two
/// corners has no edge; an edge to an index past the positions cannot be
/// drawn and is counted in the second value, as the model's
/// `skipped_edges` says.
///
/// # Safety
/// `obj` must be a valid, non-null `POLYLINE_PFACE` object of `dwg`.
unsafe fn polyline_pface_wireframe(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
) -> (Vec<[Point3D; 2]>, usize) {
    let mut positions = Vec::new();
    let mut faces: Vec<[i16; 4]> = Vec::new();

    for sub in unsafe { polyline_vertex_records(dwg, text, obj) } {
        let sub_fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        let sub_entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
        if !sub_entity_ptr.is_null() {
            let position = match sub_fixedtype {
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE => Some("VERTEX_PFACE"),
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH => Some("VERTEX_MESH"),
                _ => None,
            };
            if let Some(dxfname) = position {
                if let Some(p) = get_point3d(sub_entity_ptr, dxfname, "point") {
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
    }

    let mut edges = Vec::new();
    let mut skipped = 0;
    for face in &faces {
        let idxs: Vec<usize> = face
            .iter()
            // 1-based; 0 marks an unused slot. `checked_sub` rather than
            // `(a != 0).then_some(a - 1)`: `then_some` evaluates its argument
            // even when the condition is false, so the unused slot underflowed.
            .filter_map(|&i| usize::from(i.unsigned_abs()).checked_sub(1))
            .collect();
        if idxs.len() < 2 {
            continue;
        }
        for w in 0..idxs.len() {
            let (a, b) = (idxs[w], idxs[(w + 1) % idxs.len()]);
            match (positions.get(a), positions.get(b)) {
                (Some(&pa), Some(&pb)) => edges.push([pa, pb]),
                _ => skipped += 1,
            }
        }
    }
    (edges, skipped)
}

/// Resolves a POLYLINE_MESH ("polygon mesh") into the wireframe of its grid,
/// in the order the model states for [`Entity::PolylineMesh`]: `m` rows of
/// `n` VERTEX_MESH subentities, stored row by row (vertex `i * n + j` is row
/// `i`, column `j`); first the edges `(i, j)-(i + 1, j)` row by row, then
/// `(i, j)-(i, j + 1)` row by row. `flag` bit 1 wraps the grid in M and bit
/// 32 in N (`dwg.spec`'s POLYLINE_MESH, DXF group 70), which adds the
/// closing edges.
///
/// Returns the edges and how many the grid's definition promises but could
/// not be drawn: unless exactly `m * n` vertices were found, no grid shape
/// is guessed -- a smooth-surface mesh stores spline control points beside
/// the approximated ones -- and every edge the definition implies is
/// reported as skipped, so the mesh reads as "not read" rather than as an
/// empty one.
///
/// # Safety
/// `obj` must be a valid, non-null `POLYLINE_MESH` object of `dwg`.
unsafe fn polyline_mesh_wireframe(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
    m: usize,
    n: usize,
    flag: u16,
) -> (Vec<[Point3D; 2]>, usize) {
    let closed_m = flag & POLYLINE_MESH_CLOSED_M_FLAG != 0;
    let closed_n = flag & POLYLINE_MESH_CLOSED_N_FLAG != 0;
    let rows_joined = if closed_m { m } else { m.saturating_sub(1) };
    let columns_joined = if closed_n { n } else { n.saturating_sub(1) };
    let edge_count = rows_joined
        .saturating_mul(n)
        .saturating_add(m.saturating_mul(columns_joined));
    let positions: Vec<Point3D> = unsafe {
        polyline_vertices(
            dwg,
            text,
            obj,
            libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH,
            |v| get_point3d(v, "VERTEX_MESH", "point"),
        )
    };
    if positions.is_empty() || positions.len() != m.saturating_mul(n) {
        return (Vec::new(), edge_count);
    }
    let at = |i: usize, j: usize| positions[i * n + j];
    let mut edges = Vec::with_capacity(edge_count);
    for i in 0..rows_joined {
        for j in 0..n {
            edges.push([at(i, j), at((i + 1) % m, j)]);
        }
    }
    for i in 0..m {
        for j in 0..columns_joined {
            edges.push([at(i, j), at(i, (j + 1) % n)]);
        }
    }
    (edges, 0)
}

/// How a TEXT, ATTRIB or ATTDEF is placed beyond its start point: the
/// fields the three share, under the same dynapi names.
struct TextPlacement {
    horizontal: HorizontalJustification,
    vertical: VerticalJustification,
    alignment_point: Option<Point2D>,
    width_factor: f64,
    oblique_angle: f64,
    style_name: Ref<String>,
}

/// Reads a TEXT's, ATTRIB's or ATTDEF's [`TextPlacement`] off
/// `entity_ptr`, the type-specific struct pointer of a live `dxfname`
/// entity of `dwg`.
///
/// - The justifications are DXF 72 and 73 (74 on ATTRIB/ATTDEF), 0 to 5 and
///   0 to 3; a value outside the format's range is reported and reads as
///   the format's default, left and baseline, which is also what an absent
///   group means.
/// - The alignment point (DXF 11) exists only for a justified text:
///   dwg.h's `alignment_pt` is "optional, when dataflags & 2, i.e. 72/73 !=
///   0", and for left/baseline text the field holds whatever the decoder
///   left in it, which is not a point the file stated.
/// - The width factor (DXF 41) is a ratio whose default is 1; a 0 is no
///   width at all, and it is what a hand-written DXF that leaves the group
///   out reads as, so it is 1 too.
/// - The oblique angle (DXF 51) arrives in radians from both readers.
fn text_placement(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
) -> TextPlacement {
    let vertical_group = if dxfname == "TEXT" { 73 } else { 74 };
    let horizontal = match get_field::<u16>(entity_ptr, dxfname, "horiz_alignment") {
        None | Some(0) => HorizontalJustification::Left,
        Some(1) => HorizontalJustification::Center,
        Some(2) => HorizontalJustification::Right,
        Some(3) => HorizontalJustification::Aligned,
        Some(4) => HorizontalJustification::Middle,
        Some(5) => HorizontalJustification::Fit,
        Some(other) => {
            text.warn(format!(
                "TEXT_ALIGNMENT: a {dxfname} states horizontal alignment {other} (group 72), outside 0 to 5; it is read as left"
            ));
            HorizontalJustification::Left
        }
    };
    let vertical = match get_field::<u16>(entity_ptr, dxfname, "vert_alignment") {
        None | Some(0) => VerticalJustification::Baseline,
        Some(1) => VerticalJustification::Bottom,
        Some(2) => VerticalJustification::Middle,
        Some(3) => VerticalJustification::Top,
        Some(other) => {
            text.warn(format!(
                "TEXT_ALIGNMENT: a {dxfname} states vertical alignment {other} (group {vertical_group}), outside 0 to 3; it is read as baseline"
            ));
            VerticalJustification::Baseline
        }
    };
    let justified =
        horizontal != HorizontalJustification::Left || vertical != VerticalJustification::Baseline;
    TextPlacement {
        horizontal,
        vertical,
        alignment_point: justified
            .then(|| get_point2d(entity_ptr, dxfname, "alignment_pt"))
            .flatten(),
        width_factor: get_field::<f64>(entity_ptr, dxfname, "width_factor")
            .filter(|w| *w != 0.0)
            .unwrap_or(1.0),
        oblique_angle: get_field::<f64>(entity_ptr, dxfname, "oblique_angle").unwrap_or(0.0),
        style_name: text_style(dwg, text, entity_ptr, dxfname),
    }
}

/// What a paper-space viewport shows of the model (DXF 12, 45, 17, 16, 51,
/// 42). The record carries the view from R2000 on; an older viewport keeps
/// it in extended data, which is not read, so there is none to report. A
/// zero view direction is not a direction and reads as a plan view,
/// (0, 0, 1); the twist arrives in radians from both readers.
fn viewport_view(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
) -> Option<ViewportView> {
    if !is_r2000_or_later(dwg) {
        return None;
    }
    let number = |field: &str| get_field::<f64>(entity_ptr, "VIEWPORT", field);
    Some(ViewportView {
        center: get_point2d(entity_ptr, "VIEWPORT", "VIEWCTR")?,
        height: number("VIEWSIZE")?,
        target: get_point3d(entity_ptr, "VIEWPORT", "view_target")?,
        direction: get_point3d(entity_ptr, "VIEWPORT", "VIEWDIR")
            .filter(|d| d.x != 0.0 || d.y != 0.0 || d.z != 0.0)
            .unwrap_or(Z_AXIS),
        twist: number("VIEWTWIST")?,
        lens_length: number("LENSLENGTH")?,
    })
}

/// Whether a viewport is on. From R2000 on, both formats state it as bit
/// 0x20000 of the status flags (DXF 90), set when it is off. A DXF also
/// writes group 68, the viewport's place in the stack of active viewports,
/// where 0 is also what a viewport of a layout that is not the current one
/// is written with, on or not -- so 68 is read only for a DXF older than
/// R2000, which has no status flags; the binary format does not store it
/// (LibreDWG makes one up for a DWG, in block order).
fn viewport_on(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
) -> Option<bool> {
    if is_r2000_or_later(dwg) {
        get_field::<u32>(entity_ptr, "VIEWPORT", "status_flag").map(|f| f & VIEWPORT_OFF_FLAG == 0)
    } else if is_from_dxf(dwg) {
        get_field::<u16>(entity_ptr, "VIEWPORT", "on_off").map(|on| on != 0)
    } else {
        None
    }
}

/// An ATTRIB's or ATTDEF's flags (DXF 70), one per bit as the reference
/// names them: 1 invisible, 2 constant, 4 verify, 8 preset. An absent group
/// is no flag set.
fn attribute_flags(entity_ptr: *mut std::ffi::c_void, dxfname: &str) -> AttributeFlags {
    let flags = get_field::<u8>(entity_ptr, dxfname, "flags").unwrap_or(0);
    AttributeFlags {
        invisible: flags & 1 != 0,
        constant: flags & 2 != 0,
        verify: flags & 4 != 0,
        preset: flags & 8 != 0,
    }
}

/// The text style (DXF 7) a TEXT, ATTRIB, ATTDEF or MTEXT names, as a
/// reference like a layer. The DXF reference's default for an absent group
/// is the style named STANDARD, and LibreDWG's DXF importer already points
/// such an entity at that entry when the drawing declares it; a drawing
/// that declares none leaves the reference null, which is `Absent`.
fn text_style(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
) -> Ref<String> {
    reference(
        dwg,
        text,
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "style"),
        c"STYLE",
        |handle_ptr| text.handle_name(dwg, handle_ptr),
    )
}

/// Resolves a WIPEOUT's clip boundary to local 2D points -- see
/// [`crate::model::WipeoutEntity`] for the risk this carries.
///
/// `clip_boundary_type` 1 ("rect") stores exactly 2 `clip_verts`, two opposite
/// corners of a pixel-space-axis-aligned rectangle; anything else (2,
/// "polygon", or unset) is used as an explicit vertex list. With no usable
/// `clip_verts` at all this falls back to the full image, whose pixel extent
/// runs from -0.5 to `image_size - 0.5` on each axis.
///
/// The clip vertices are in the image's pixel space, which runs from the
/// image's *upper* left corner with each pixel's center on a whole number:
/// a vertex `(x, y)` lies at `pt0 + (x + 0.5)*uvec + (h - 0.5 - y)*vvec`,
/// `h` being `image_size.y`. `uvec`/`vvec` are one pixel long in the
/// entity's local space. A WIPEOUT's image is one pixel square, so its
/// vertices run from -0.5 to 0.5 and span `pt0` to `pt0 + uvec + vvec`.
/// Two independent readers of the format (ezdxf's image boundary and
/// acadrust's wipeout) agree on this.
fn wipeout_boundary(entity_ptr: *mut std::ffi::c_void) -> Vec<Point2D> {
    let Some(pt0) = get_point3d(entity_ptr, "WIPEOUT", "pt0") else {
        return Vec::new();
    };
    let uvec = get_point3d(entity_ptr, "WIPEOUT", "uvec").unwrap_or(Point3D {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    let vvec = get_point3d(entity_ptr, "WIPEOUT", "vvec").unwrap_or(Point3D {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    });
    let size =
        get_point2d(entity_ptr, "WIPEOUT", "image_size").unwrap_or(Point2D { x: 1.0, y: 1.0 });
    clip_boundary(entity_ptr, "WIPEOUT", pt0, uvec, vvec, size)
}

/// The clip boundary of a raster entity -- an IMAGE, or the WIPEOUT that
/// shares its layout (`dxfname` says which struct to read) -- put through
/// the entity's frame, as [`wipeout_boundary`] describes.
fn clip_boundary(
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
    pt0: Point3D,
    uvec: Point3D,
    vvec: Point3D,
    size: Point2D,
) -> Vec<Point2D> {
    let mut clip_verts: Vec<Point2D> =
        get_point2d_array::<u32>(entity_ptr, dxfname, "num_clip_verts", "clip_verts");
    // A polygon stored closed repeats its first vertex at the end; the loop
    // is closed either way, and the repeat is not a vertex.
    if clip_verts.len() > 2 && clip_verts.first() == clip_verts.last() {
        clip_verts.pop();
    }
    // BITCODE_BS ("1 rect, 2 polygon"). An unreadable or unset value is
    // treated like "polygon", not assumed to be "rect".
    let clip_boundary_type =
        get_field::<u16>(entity_ptr, dxfname, "clip_boundary_type").unwrap_or(0);

    let rect = |a: Point2D, b: Point2D| {
        vec![
            Point2D { x: a.x, y: a.y },
            Point2D { x: b.x, y: a.y },
            Point2D { x: b.x, y: b.y },
            Point2D { x: a.x, y: b.y },
        ]
    };
    let pixel_points: Vec<Point2D> = if clip_boundary_type == 1 && clip_verts.len() == 2 {
        rect(clip_verts[0], clip_verts[1])
    } else if !clip_verts.is_empty() {
        clip_verts
    } else {
        rect(
            Point2D { x: -0.5, y: -0.5 },
            Point2D {
                x: size.x - 0.5,
                y: size.y - 0.5,
            },
        )
    };

    pixel_points
        .into_iter()
        .map(|p| {
            let (u, v) = (p.x + 0.5, size.y - 0.5 - p.y);
            Point2D {
                x: pt0.x + u * uvec.x + v * vvec.x,
                y: pt0.y + u * uvec.y + v * vvec.y,
            }
        })
        .collect()
}

/// `depth` is how many owned-*sub*entity steps were taken to reach `obj` (0
/// for an entity a block owns); past [`MAX_SUBENTITY_DEPTH`] nothing is
/// converted.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` `obj` was obtained from; `obj` must be a
/// valid pointer from `dwg_get_object` on that same `Dwg_Data`.
unsafe fn convert_entity(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
    depth: u32,
) -> Option<Entity> {
    if depth > MAX_SUBENTITY_DEPTH {
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
    let (id, source_handle) = unsafe { entity_identity(obj) };
    let layer = reference(
        dwg,
        text,
        get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "layer"),
        c"LAYER",
        |handle_ptr| text.handle_name(dwg, handle_ptr),
    );
    let (color_index, true_color) = entity_color(entity_ptr);
    // This backend reads vector files: everything it produces is a vector
    // entity whose values are what the file states. A raster recognizer or
    // an editor states different markers; nothing here has a default.
    let common = EntityCommon {
        id,
        origin: Origin::Vector,
        confidence: Confidence::High,
        source_handle,
        layer,
        color_index,
        true_color,
        // Bit 1 of the common `invisible` word (DXF 60).
        invisible: get_common_field::<u16>(entity_ptr, "invisible").is_some_and(|v| v & 1 != 0),
        linetype: entity_linetype(dwg, text, entity_ptr),
        linetype_scale: get_common_field::<f64>(entity_ptr, "ltype_scale").unwrap_or(1.0),
        lineweight: entity_lineweight(dwg, entity_ptr),
        transparency: entity_transparency(dwg, entity_ptr),
    };

    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LINE => {
            let start_point = get_point3d(entity_ptr, "LINE", "start")?;
            let end_point = get_point3d(entity_ptr, "LINE", "end")?;
            Entity::Line(LineEntity {
                common,
                start_point,
                end_point,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_CIRCLE => {
            let center = get_point3d(entity_ptr, "CIRCLE", "center")?;
            let radius = get_field::<f64>(entity_ptr, "CIRCLE", "radius")?;
            Entity::Circle(CircleEntity {
                common,
                center,
                radius,
                extrusion: extrusion(entity_ptr, "CIRCLE"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TEXT => {
            let start_point = get_point2d(entity_ptr, "TEXT", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "TEXT", "height")?;
            let placement = text_placement(dwg, text, entity_ptr, "TEXT");
            let text = text
                .field(entity_ptr, "TEXT", "text_value")
                .unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0);
            Entity::Text(TextEntity {
                common,
                start_point,
                text_height,
                text,
                rotation,
                horizontal_justification: placement.horizontal,
                vertical_justification: placement.vertical,
                alignment_point: placement.alignment_point,
                width_factor: placement.width_factor,
                oblique_angle: placement.oblique_angle,
                style_name: placement.style_name,
                elevation: get_field::<f64>(entity_ptr, "TEXT", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "TEXT"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LWPOLYLINE => {
            let points: Vec<Point2D> =
                get_point2d_array::<u32>(entity_ptr, "LWPOLYLINE", "num_points", "points");
            // The record stores the bulges as a separate array that is either
            // empty (every segment straight) or one per vertex.
            let bulges: Vec<f64> =
                get_array_field::<u32, f64>(entity_ptr, "LWPOLYLINE", "num_bulges", "bulges");
            let vertices = with_bulges(text, "LWPOLYLINE", points, bulges);
            let flag = get_field::<u16>(entity_ptr, "LWPOLYLINE", "flag").unwrap_or(0);
            let const_width =
                get_field::<f64>(entity_ptr, "LWPOLYLINE", "const_width").unwrap_or(0.0);
            // The widths are a separate array too, on the same terms.
            let widths = get_array_field::<u32, RawSegmentWidth>(
                entity_ptr,
                "LWPOLYLINE",
                "num_widths",
                "widths",
            );
            let vertices = with_widths(text, "LWPOLYLINE", vertices, widths, const_width);
            Entity::LwPolyline(LwPolylineEntity {
                common,
                vertices,
                closed: flag & LWPOLYLINE_CLOSED_FLAG != 0,
                const_width,
                elevation: get_field::<f64>(entity_ptr, "LWPOLYLINE", "elevation").unwrap_or(0.0),
                extrusion: if flag & LWPOLYLINE_EXTRUSION_FLAG != 0 {
                    extrusion(entity_ptr, "LWPOLYLINE")
                } else {
                    Point3D {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    }
                },
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC => {
            let center = get_point3d(entity_ptr, "ARC", "center")?;
            let radius = get_field::<f64>(entity_ptr, "ARC", "radius")?;
            let start_angle = get_field::<f64>(entity_ptr, "ARC", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ARC", "end_angle")?;
            Entity::Arc(ArcEntity {
                common,
                center,
                radius,
                start_angle,
                end_angle,
                extrusion: extrusion(entity_ptr, "ARC"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ELLIPSE => {
            let center = get_point3d(entity_ptr, "ELLIPSE", "center")?;
            let major_axis_endpoint = get_point3d(entity_ptr, "ELLIPSE", "sm_axis")?;
            let axis_ratio = get_field::<f64>(entity_ptr, "ELLIPSE", "axis_ratio")?;
            let start_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "end_angle")?;
            let extrusion = extrusion(entity_ptr, "ELLIPSE");
            Entity::Ellipse(EllipseEntity {
                common,
                center,
                major_axis_endpoint,
                axis_ratio,
                start_angle,
                end_angle,
                extrusion,
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
            let corner1 = get_point2d(entity_ptr, "SOLID", "corner1")?;
            let corner2 = get_point2d(entity_ptr, "SOLID", "corner2")?;
            let corner3 = get_point2d(entity_ptr, "SOLID", "corner3")?;
            let corner4 = get_point2d(entity_ptr, "SOLID", "corner4")?;
            Entity::Solid(SolidEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
                elevation: get_field::<f64>(entity_ptr, "SOLID", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "SOLID"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TRACE => {
            // Dwg_Entity_TRACE has exactly SOLID's fields (dwg.h), and DXF
            // gives both the same group codes.
            let corner1 = get_point2d(entity_ptr, "TRACE", "corner1")?;
            let corner2 = get_point2d(entity_ptr, "TRACE", "corner2")?;
            let corner3 = get_point2d(entity_ptr, "TRACE", "corner3")?;
            let corner4 = get_point2d(entity_ptr, "TRACE", "corner4")?;
            Entity::Trace(SolidEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
                elevation: get_field::<f64>(entity_ptr, "TRACE", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "TRACE"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_RAY => {
            let point = get_point3d(entity_ptr, "RAY", "point")?;
            let vector = get_point3d(entity_ptr, "RAY", "vector")?;
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
            let point = get_point3d(entity_ptr, "XLINE", "point")?;
            let vector = get_point3d(entity_ptr, "XLINE", "vector")?;
            Entity::XLine(RayEntity {
                common,
                point,
                vector,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB => {
            let start_point = get_point2d(entity_ptr, "ATTRIB", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "ATTRIB", "height")?;
            // Read before `text` is shadowed by the value below.
            let tag = text.field(entity_ptr, "ATTRIB", "tag").unwrap_or_default();
            let placement = text_placement(dwg, text, entity_ptr, "ATTRIB");
            let flags = attribute_flags(entity_ptr, "ATTRIB");
            let text = text
                .field(entity_ptr, "ATTRIB", "text_value")
                .unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "ATTRIB", "rotation").unwrap_or(0.0);
            Entity::Attrib(AttribEntity {
                common,
                start_point,
                text_height,
                tag,
                text,
                rotation,
                flags,
                horizontal_justification: placement.horizontal,
                vertical_justification: placement.vertical,
                alignment_point: placement.alignment_point,
                width_factor: placement.width_factor,
                oblique_angle: placement.oblique_angle,
                style_name: placement.style_name,
                elevation: get_field::<f64>(entity_ptr, "ATTRIB", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "ATTRIB"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_INSERT => {
            let block_name = reference(
                dwg,
                text,
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                    entity_ptr,
                    "INSERT",
                    "block_header",
                ),
                c"BLOCK",
                |handle_ptr| crate::table_convert::resolve_block_name(text, handle_ptr),
            );
            let insertion_point = get_point3d(entity_ptr, "INSERT", "ins_pt")?;
            let scale = get_point3d(entity_ptr, "INSERT", "scale").unwrap_or(Point3D {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
            let rotation = get_field::<f64>(entity_ptr, "INSERT", "rotation").unwrap_or(0.0);

            // ATTRIBs are owned by the INSERT itself -- a separate ownership
            // relationship from BLOCK_HEADER -> entity. R13..R2000 chains
            // them and the library's own walker cannot follow a chain the
            // DXF importer built (see chained_insert_attribs); from R2004 on
            // they are an owned array the library resolves correctly, walked
            // from the INSERT's own Dwg_Object rather than from entity_ptr
            // (the type-specific struct dynapi needs, a different pointer).
            let attribs = if unsafe { libredwg_sys::uncad_dwg_is_r13_to_r2000(dwg) } != 0 {
                unsafe { chained_insert_attribs(dwg, text, entity_ptr, depth) }
            } else {
                unsafe { owned_insert_attribs(dwg, text, obj, depth) }
            };

            Entity::Insert(InsertEntity {
                common,
                block_name,
                insertion_point,
                scale,
                rotation,
                attribs,
                extrusion: extrusion(entity_ptr, "INSERT"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF => {
            let start_point = get_point2d(entity_ptr, "ATTDEF", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "ATTDEF", "height")?;
            let default_value = text
                .field(entity_ptr, "ATTDEF", "default_value")
                .unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "ATTDEF", "rotation").unwrap_or(0.0);
            let tag = text.field(entity_ptr, "ATTDEF", "tag").unwrap_or_default();
            let placement = text_placement(dwg, text, entity_ptr, "ATTDEF");
            let flags = attribute_flags(entity_ptr, "ATTDEF");
            Entity::Attdef(AttdefEntity {
                common,
                start_point,
                text_height,
                tag,
                default_value,
                rotation,
                flags,
                horizontal_justification: placement.horizontal,
                vertical_justification: placement.vertical,
                alignment_point: placement.alignment_point,
                width_factor: placement.width_factor,
                oblique_angle: placement.oblique_angle,
                style_name: placement.style_name,
                elevation: get_field::<f64>(entity_ptr, "ATTDEF", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "ATTDEF"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VIEWPORT => {
            let center = get_point3d(entity_ptr, "VIEWPORT", "center")?;
            let width = get_field::<f64>(entity_ptr, "VIEWPORT", "width")?;
            let height = get_field::<f64>(entity_ptr, "VIEWPORT", "height")?;
            Entity::Viewport(ViewportEntity {
                common,
                center,
                width,
                height,
                view: viewport_view(dwg, entity_ptr),
                on: viewport_on(dwg, entity_ptr),
                // DXF 69. The binary format stores no such number: LibreDWG
                // makes one up for a DWG's viewports, in block order, so
                // only a DXF's is the file's.
                viewport_id: is_from_dxf(dwg)
                    .then(|| get_field::<u16>(entity_ptr, "VIEWPORT", "id"))
                    .flatten()
                    .map(i32::from),
                frozen_layers: get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
                    entity_ptr,
                    "VIEWPORT",
                    "num_frozen_layers",
                    "frozen_layers",
                )
                .into_iter()
                .filter(|h| !h.is_null())
                .map(|h| {
                    reference(dwg, text, Some(h), c"LAYER", |handle_ptr| {
                        text.handle_name(dwg, handle_ptr)
                    })
                })
                .collect(),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DFACE => {
            let corner1 = get_point3d(entity_ptr, "3DFACE", "corner1")?;
            let corner2 = get_point3d(entity_ptr, "3DFACE", "corner2")?;
            let corner3 = get_point3d(entity_ptr, "3DFACE", "corner3")?;
            let corner4 = get_point3d(entity_ptr, "3DFACE", "corner4")?;
            let invis_flags = get_field::<u16>(entity_ptr, "3DFACE", "invis_flags").unwrap_or(0);
            Entity::Face3D(Face3DEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
                invisible_edges: Face3DEntity::invisible_edges_from_bits(u32::from(invis_flags)),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SPLINE => {
            // num_fit_pts is BITCODE_BS (u16), unlike most other num_X fields
            // (BITCODE_BL/u32) -- see get_array_field's doc comment.
            let fit_points: Vec<Point3D> =
                get_point3d_array::<u16>(entity_ptr, "SPLINE", "num_fit_pts", "fit_pts");
            let control: Vec<SplineControlPoint> = get_array_field::<u32, SplineControlPoint>(
                entity_ptr,
                "SPLINE",
                "num_ctrl_pts",
                "ctrl_pts",
            );
            let degree = u32::from(get_field::<u16>(entity_ptr, "SPLINE", "degree")?);
            // The record has two forms (`scenario`): 1 stores the curve by
            // control points, knots and weights, with the closed / periodic /
            // weighted bits beside them; 2 stores fit points and end tangents
            // and none of those bits, so there they are not stated -- not
            // false. From R2013 on the record also carries `splineflags`,
            // whose bit 4 states "closed" for either form; before R2013 the
            // library fills that field in itself.
            let by_control_points = get_field::<u16>(entity_ptr, "SPLINE", "scenario") == Some(1);
            let bit = |name: &str| {
                by_control_points
                    .then(|| get_field::<u8>(entity_ptr, "SPLINE", name).map(|b| b != 0))
                    .flatten()
            };
            let closed = bit("closed_b").or_else(|| {
                is_r2013_or_later(dwg)
                    .then(|| get_field::<u32>(entity_ptr, "SPLINE", "splineflags"))
                    .flatten()
                    .map(|f| f & 4 != 0)
            });
            let periodic = bit("periodic");
            // The curve's definition -- its knots, and its weights when it
            // has them -- goes with its control points, whichever form the
            // record is flagged as. A DWG record of the fit-point form stores
            // none of them; a DXF of that form writes the control points and
            // knots its program computed, and they define the curve all the
            // same.
            let knots = if control.is_empty() {
                Vec::new()
            } else {
                get_array_field::<u32, f64>(entity_ptr, "SPLINE", "num_knots", "knots")
            };
            // Weights are stated when the record says it is weighted, or --
            // read from a DXF -- when a group 41 gave one: the importer fills
            // `w` from 41 but sets the weighted bit from another bit of group
            // 70. The library leaves `w` at 0 where no weight was given,
            // which is not a weight. No weights means every weight is 1.
            let weighted = get_field::<u8>(entity_ptr, "SPLINE", "weighted") == Some(1);
            let weights = if !control.is_empty() && (weighted || control.iter().any(|p| p.w != 0.0))
            {
                control.iter().map(|p| p.w).collect()
            } else {
                Vec::new()
            };
            // The fit-point form stores both end tangents, the control-point
            // form none. A tangent the curve was not given is stored as the
            // zero vector -- which is no direction at all -- and the DXF of
            // the same drawing leaves the group out: not stated.
            let tangent = |field: &str| {
                (!by_control_points)
                    .then(|| get_point3d(entity_ptr, "SPLINE", field))
                    .flatten()
                    .filter(|v| (v.x, v.y, v.z) != (0.0, 0.0, 0.0))
            };
            Entity::Spline(SplineEntity {
                common,
                degree,
                closed,
                periodic,
                knots,
                weights,
                fit_points,
                control_points: control.into_iter().map(Into::into).collect(),
                start_tangent: tangent("beg_tan_vec"),
                end_tangent: tangent("end_tan_vec"),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MTEXT => {
            let insertion_point = get_point3d(entity_ptr, "MTEXT", "ins_pt")?;
            let style_name = text_style(dwg, text, entity_ptr, "MTEXT");
            // DXF 41: 0 is a text that is not wrapped, which is what an absent
            // group means too.
            let rect_width = get_field::<f64>(entity_ptr, "MTEXT", "rect_width").unwrap_or(0.0);
            // DXF 42/43, a measurement of the laid-out text: zero measures no
            // text, so it is "not stated" rather than a size.
            let extent =
                |field: &str| get_field::<f64>(entity_ptr, "MTEXT", field).filter(|v| *v != 0.0);
            let text = text.field(entity_ptr, "MTEXT", "text").unwrap_or_default();
            let text_height = get_field::<f64>(entity_ptr, "MTEXT", "text_height").unwrap_or(1.0);
            // The file states the rotation as the text's X-axis direction
            // (DXF 11); the DXF reference defines a rotation angle given as
            // input (DXF 50) as the same thing expressed as that vector, so
            // the angle is the vector's direction. A drawing's own twin read
            // by a second reader gives the same values.
            let rotation =
                get_point3d(entity_ptr, "MTEXT", "x_axis_dir").map_or(0.0, |d| d.y.atan2(d.x));
            // The reference's range for this factor is 0.25 to 4.00, so a
            // zero is not a value the file stated: a drawing older than
            // R2000 has no such field, and the record comes back zero-filled.
            // Unstated reads as 1 (a fraction of the default spacing).
            let line_spacing_factor = get_field::<f64>(entity_ptr, "MTEXT", "linespace_factor")
                .filter(|f| *f != 0.0)
                .unwrap_or(1.0);
            // DXF 71, 1 to 9: which point of the text block the insertion
            // point is. Anything else is not a value this reader can state.
            let attachment = match get_field::<u16>(entity_ptr, "MTEXT", "attachment") {
                Some(1) => Some(MTextAttachment::TopLeft),
                Some(2) => Some(MTextAttachment::TopCenter),
                Some(3) => Some(MTextAttachment::TopRight),
                Some(4) => Some(MTextAttachment::MiddleLeft),
                Some(5) => Some(MTextAttachment::MiddleCenter),
                Some(6) => Some(MTextAttachment::MiddleRight),
                Some(7) => Some(MTextAttachment::BottomLeft),
                Some(8) => Some(MTextAttachment::BottomCenter),
                Some(9) => Some(MTextAttachment::BottomRight),
                _ => None,
            };
            Entity::MText(MTextEntity {
                common,
                insertion_point,
                text,
                text_height,
                rotation,
                line_spacing_factor,
                attachment,
                reference_width: rect_width,
                extents_width: extent("extents_width"),
                extents_height: extent("extents_height"),
                style_name,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_3D => {
            // SAFETY: obj is a POLYLINE_3D of dwg per fixedtype.
            let vertices: Vec<Point3D> = unsafe {
                polyline_vertices(
                    dwg,
                    text,
                    obj,
                    libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D,
                    |v| get_point3d(v, "VERTEX_3D", "point"),
                )
            };
            // POLYLINE_3D.flag is BITCODE_RC (1 byte), unlike LWPOLYLINE's
            // BITCODE_BS (2 bytes) -- same closed-bit convention, different
            // underlying C width.
            let flag = get_field::<u8>(entity_ptr, "POLYLINE_3D", "flag").unwrap_or(0);
            Entity::Polyline3D(PolylineEntity {
                common,
                vertices,
                closed: flag & (POLYLINE_CLOSED_FLAG as u8) != 0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_2D => {
            // A DXF states the polyline's default widths once (its own
            // groups 40/41) and leaves them out of every VERTEX that has
            // them, and the importer reads an absent vertex width as 0: so
            // there a vertex that states no width has the default. A DWG
            // stores every vertex's widths on the vertex (the DWG twin of
            // 2000/PolyLine2D.dxf has 0.15 on each vertex where the DXF has
            // it once, on the POLYLINE).
            let default_width = if is_from_dxf(dwg) {
                (
                    get_field::<f64>(entity_ptr, "POLYLINE_2D", "start_width").unwrap_or(0.0),
                    get_field::<f64>(entity_ptr, "POLYLINE_2D", "end_width").unwrap_or(0.0),
                )
            } else {
                (0.0, 0.0)
            };
            // Each VERTEX_2D carries its own bulge (the segment to the next
            // vertex) and widths; an unreadable bulge is a straight segment.
            // A VERTEX_2D's stored point is 3D, but its z is the polyline's
            // own elevation repeated.
            // SAFETY: obj is a POLYLINE_2D of dwg per fixedtype.
            let vertices: Vec<PolylineVertex> = unsafe {
                polyline_vertices(
                    dwg,
                    text,
                    obj,
                    libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D,
                    |v| {
                        let p = get_point3d(v, "VERTEX_2D", "point")?;
                        let width =
                            |field: &str| get_field::<f64>(v, "VERTEX_2D", field).unwrap_or(0.0);
                        let stated = (width("start_width"), width("end_width"));
                        let (start_width, end_width) = if stated == (0.0, 0.0) {
                            default_width
                        } else {
                            stated
                        };
                        Some(PolylineVertex {
                            point: Point2D { x: p.x, y: p.y },
                            bulge: get_field::<f64>(v, "VERTEX_2D", "bulge").unwrap_or(0.0),
                            start_width,
                            end_width,
                        })
                    },
                )
            };
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_2D", "flag").unwrap_or(0);
            Entity::Polyline2D(LwPolylineEntity {
                common,
                vertices,
                closed: flag & POLYLINE_CLOSED_FLAG != 0,
                // A POLYLINE_2D states no constant width: its widths are its
                // vertices' own (see LwPolylineEntity::const_width).
                const_width: 0.0,
                elevation: get_field::<f64>(entity_ptr, "POLYLINE_2D", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "POLYLINE_2D"),
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
            let block_name = reference(
                dwg,
                text,
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "block"),
                c"BLOCK",
                |handle_ptr| crate::table_convert::resolve_block_name(text, handle_ptr),
            );
            // Which of this backend's points is which DXF group depends on
            // the subtype: group 13 is the first extension line for a linear
            // dimension and the feature location for an ordinate one, and a
            // two-line angular dimension keeps its group 10 under a different
            // field name than every other subtype does. The mapping is written out
            // per subtype rather than passing this backend's field names
            // through, so one model field never holds two different points.
            let (p13, p14, p15, p16) = dimension_point_fields(fixedtype, is_from_dxf(dwg));
            let point = |field: Option<&'static str>| {
                field.and_then(|f| get_point3d(entity_ptr, dxfname, f))
            };
            let kind = dimension_kind(fixedtype);
            Entity::Dimension(DimensionEntity {
                common,
                block_name,
                kind,
                // This backend has no "the file did not carry this group":
                // an absent DXF 42 and a stated 0.0 arrive the same way. A
                // dimension that measures nothing is not a measurement, so
                // zero is reported as "not stated" -- erring toward not
                // knowing rather than toward a value the file never gave.
                // Drawings older than R2000 routinely omit the group, and
                // reporting 0.0 for them would put a false difference
                // between a drawing and its own twin in the other format.
                // -1 is the other "not measured" the format's writers leave
                // behind (no length or angle is negative); any other value
                // is the file's, whether or not it agrees with the points --
                // judging that is a consumer's step.
                measurement: get_field::<f64>(entity_ptr, dxfname, "act_measurement")
                    .filter(|m| *m != 0.0 && *m != -1.0),
                text_override: dimension_text_override(
                    text.field(entity_ptr, dxfname, "user_text").as_deref(),
                ),
                // A two-line angular dimension decoded from a DWG keeps group
                // 10 in the record's last point, which this library names
                // `xline2end_pt` (its `def_pt` holds group 16); the DXF
                // importer fills the two by group code instead -- see
                // `dimension_point_fields`.
                definition_point: if kind == Some(DimensionKind::Angular2Line) && !is_from_dxf(dwg)
                {
                    get_point3d(entity_ptr, dxfname, "xline2end_pt")
                } else {
                    get_point3d(entity_ptr, dxfname, "def_pt")
                },
                text_midpoint: get_point2d(entity_ptr, dxfname, "text_midpt").unwrap_or_default(),
                points: DimensionPoints {
                    extension1: point(p13),
                    extension2: point(p14),
                    radial: point(p15),
                    arc: if kind == Some(DimensionKind::ArcLength) && is_from_dxf(dwg) {
                        // Read from DXF, a zeroed first leader point is also
                        // what the importer leaves when the file omits group
                        // 16, which it does when the dimension has no leader
                        // (group 71): only a stated leader makes it a fact.
                        get_field::<u8>(entity_ptr, dxfname, "has_leader")
                            .is_some_and(|v| v != 0)
                            .then(|| point(p16))
                            .flatten()
                    } else {
                        point(p16)
                    },
                },
                // Group 50 is the measured angle only for a rotated linear
                // dimension; the other subtypes do not write it, and the
                // format's default is 0.
                rotation: if kind == Some(DimensionKind::Rotated) {
                    get_field::<f64>(entity_ptr, dxfname, "dim_rotation").unwrap_or(0.0)
                } else {
                    0.0
                },
                text_rotation: get_field::<f64>(entity_ptr, dxfname, "text_rotation")
                    .unwrap_or(0.0),
                style_name: reference(
                    dwg,
                    text,
                    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "dimstyle"),
                    c"DIMSTYLE",
                    |handle_ptr| text.handle_name(dwg, handle_ptr),
                ),
                ordinate_axis: (kind == Some(DimensionKind::Ordinate))
                    .then(|| ordinate_axis(dwg, entity_ptr, dxfname)),
                // SAFETY: obj is a live entity object of dwg.
                style_overrides: Some(unsafe { style_overrides(dwg, obj, text) }),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TABLE => {
            // dynapi's field-table key for this type is "TABLE" (dwg.h's
            // internal name), not "ACAD_TABLE" (the DXF name
            // dwg_object_get_dxfname reports) -- passing the latter would fail
            // dwg_dynapi_entity_value's strict obj->name check, the same
            // pitfall as REGION/3DSOLID (see acis.rs).
            let block_name = reference(
                dwg,
                text,
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "TABLE", "block_header"),
                c"BLOCK",
                |handle_ptr| crate::table_convert::resolve_block_name(text, handle_ptr),
            );
            let insertion_point = get_point3d(entity_ptr, "TABLE", "ins_pt")?;
            let scale = get_point3d(entity_ptr, "TABLE", "scale").unwrap_or(Point3D {
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
                    let gradient_name = text
                        .field(entity_ptr, "HATCH", "gradient_name")
                        .unwrap_or_default();
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
                elevation: get_field::<f64>(entity_ptr, "HATCH", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "HATCH"),
                style: get_field::<u16>(entity_ptr, "HATCH", "style")
                    .and_then(|s| crate::model::HatchStyle::from_code(i64::from(s))),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DSOLID => {
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity__3DSOLID*
            // (checked above), matching fixedtype.
            let (wireframe_edges, skipped_edges) =
                unsafe { crate::acis::extract_wireframe(entity_ptr, "3DSOLID") };
            Entity::Solid3D(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_REGION => {
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_REGION*
            // (checked above, matching fixedtype), which dwg.h typedefs from
            // Dwg_Entity__3DSOLID -- layout-identical, so the cast
            // extract_wireframe does internally is sound. Its real dxfname has
            // to be passed through: dynapi refuses a name mismatch (acis.rs).
            let (wireframe_edges, skipped_edges) =
                unsafe { crate::acis::extract_wireframe(entity_ptr, "REGION") };
            Entity::Region(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_PFACE => {
            // SAFETY: obj is a valid, non-null POLYLINE_PFACE object of dwg
            // (matching fixedtype); the helper only walks its vertex records.
            let (wireframe_edges, skipped_edges) =
                unsafe { polyline_pface_wireframe(dwg, text, obj) };
            Entity::PolylinePFace(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_MESH => {
            let m = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "num_m_verts").unwrap_or(0);
            let n = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "num_n_verts").unwrap_or(0);
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_MESH", "flag").unwrap_or(0);
            // SAFETY: obj is a valid, non-null POLYLINE_MESH object of dwg
            // (matching fixedtype); the helper only walks its vertex records.
            let (wireframe_edges, skipped_edges) = unsafe {
                polyline_mesh_wireframe(dwg, text, obj, usize::from(m), usize::from(n), flag)
            };
            Entity::PolylineMesh(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TOLERANCE => {
            let insertion_point = get_point3d(entity_ptr, "TOLERANCE", "ins_pt")?;
            // Same as the dimension's measurement: this library has no "the
            // file did not write this group" for a number, and a frame of
            // zero height is not a height.
            let text_height =
                get_field::<f64>(entity_ptr, "TOLERANCE", "height").filter(|h| *h != 0.0);
            let text_value = text
                .field(entity_ptr, "TOLERANCE", "text_value")
                .unwrap_or_default();
            Entity::Tolerance(ToleranceEntity {
                common,
                insertion_point,
                text_height,
                text_value,
                // A zero vector is not a direction: this library cannot tell
                // an absent DXF 11 from a zeroed one, so the degenerate
                // value is reported as nothing rather than as a direction.
                direction: get_point3d(entity_ptr, "TOLERANCE", "x_direction")
                    .filter(|d| d.x != 0.0 || d.y != 0.0 || d.z != 0.0),
                style_name: reference(
                    dwg,
                    text,
                    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                        entity_ptr,
                        "TOLERANCE",
                        "dimstyle",
                    ),
                    c"DIMSTYLE",
                    |handle_ptr| text.handle_name(dwg, handle_ptr),
                ),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_WIPEOUT => {
            let boundary = wipeout_boundary(entity_ptr);
            Entity::Wipeout(WipeoutEntity { common, boundary })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_IMAGE => {
            let point = |field: &str| get_point3d(entity_ptr, "IMAGE", field).unwrap_or_default();
            let (insertion_point, u_vector, v_vector) =
                (point("pt0"), point("uvec"), point("vvec"));
            let size_pixels = get_point2d(entity_ptr, "IMAGE", "image_size").unwrap_or_default();
            let byte = |field: &str| get_field::<u8>(entity_ptr, "IMAGE", field);
            Entity::Image(ImageEntity {
                common,
                insertion_point,
                u_vector,
                v_vector,
                size_pixels,
                definition: reference(
                    dwg,
                    text,
                    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "IMAGE", "imagedef"),
                    c"IMAGEDEF",
                    // SAFETY: the handle belongs to the live Dwg_Data this
                    // pass walks, the contract `reference` calls under.
                    |handle_ptr| unsafe { image_definition_handle(dwg, handle_ptr) },
                ),
                display_flags: get_field::<u16>(entity_ptr, "IMAGE", "display_props"),
                clipping: byte("clipping").map(|v| v != 0),
                brightness: byte("brightness"),
                contrast: byte("contrast"),
                fade: byte("fade"),
                // The flag exists from R2010 on; before it, the struct holds
                // a zero the file never stated.
                clip_outside: since(dwg, libredwg_sys::DWG_VERSION_TYPE_R_2010b)
                    .then(|| byte("clip_mode"))
                    .flatten()
                    .map(|v| v != 0),
                boundary: clip_boundary(
                    entity_ptr,
                    "IMAGE",
                    insertion_point,
                    u_vector,
                    v_vector,
                    size_pixels,
                ),
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LIGHT => {
            let position = get_point3d(entity_ptr, "LIGHT", "position")?;
            let target = get_point3d(entity_ptr, "LIGHT", "target").unwrap_or(position);
            // type: distant=1, point=2, spot=3 (dwg.h, BITCODE_BL). Whether
            // the light aims at `target` follows from this; carrying the
            // type rather than that conclusion leaves the step to whoever
            // needs it. An unreadable field or a value outside the three is
            // nothing, not a point light.
            let light_type = match get_field::<u32>(entity_ptr, "LIGHT", "type") {
                Some(1) => Some(LightType::Distant),
                Some(2) => Some(LightType::Point),
                Some(3) => Some(LightType::Spot),
                _ => None,
            };
            Entity::Light(LightEntity {
                common,
                position,
                target,
                light_type,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINE => {
            Entity::MLine(convert_mline(dwg, text, entity_ptr, common))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MULTILEADER => Entity::MultiLeader({
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_MULTILEADER*
            // (checked above), matching fixedtype.
            let lines = unsafe { multileader_lines(entity_ptr) };
            MultiLeaderEntity { common, lines }
        }),
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LEADER => {
            let vertices: Vec<Point3D> =
                get_point3d_array::<u32>(entity_ptr, "LEADER", "num_points", "points");
            // Whether an arrowhead is drawn is `arrowhead_on`, the bit the
            // format writes as group 71. `arrowhead_type` is a different
            // field -- *which* arrowhead, not whether -- and the format does
            // not write it to DXF at all, so reading it here reported an
            // arrowhead for a leader whose file says it has none.
            //
            // From R2010 on the library reads this record one field short:
            // it skips the annotation offset, which those files still carry,
            // and every field after it comes from the wrong bits. Its
            // "arrowhead" there is part of the offset's encoding (always
            // off when the offset's z is zero), so it is not reported.
            let has_arrowhead = if is_r2010_or_later(dwg) {
                None
            } else {
                get_field::<u8>(entity_ptr, "LEADER", "arrowhead_on").map(|on| on != 0)
            };
            // 0 straight, 1 spline (dwg.spec). The format does not state
            // what an absent group means, so an unreadable field is nothing.
            let path_type = match get_field::<u16>(entity_ptr, "LEADER", "path_type") {
                Some(0) => Some(LeaderPath::Straight),
                Some(1) => Some(LeaderPath::Spline),
                _ => None,
            };
            // 0 text, 1 tolerance, 2 insert, 3 none -- and 3 is the value the
            // format falls back to, so anything else reads as "nothing".
            let annotation = match get_field::<u16>(entity_ptr, "LEADER", "annot_type") {
                Some(0) => LeaderAnnotation::MText,
                Some(1) => LeaderAnnotation::Tolerance,
                Some(2) => LeaderAnnotation::Insert,
                _ => LeaderAnnotation::Nothing,
            };
            let annotation_id = entity_reference(
                dwg,
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                    entity_ptr,
                    "LEADER",
                    "associated_annotation",
                ),
            );
            Entity::Leader(LeaderEntity {
                common,
                vertices,
                has_arrowhead,
                path_type,
                annotation,
                annotation_id,
                style_name: reference(
                    dwg,
                    text,
                    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                        entity_ptr, "LEADER", "dimstyle",
                    ),
                    c"DIMSTYLE",
                    |handle_ptr| text.handle_name(dwg, handle_ptr),
                ),
                // SAFETY: obj is a live entity object of dwg.
                style_overrides: Some(unsafe { style_overrides(dwg, obj, text) }),
            })
        }
        // SAFETY: obj is valid per this function's own `# Safety` doc contract.
        _ => Entity::Unknown {
            common,
            type_name: unsafe { dxfname(obj) },
        },
    })
}

/// The dimension-style variables an entity sets for itself -- the `DSTYLE`
/// list in its extended data under `ACAD` -- through the
/// `uncad_entity_style_overrides` shim, which walks the extended-data items
/// dynapi does not expose. Empty when the entity carries no such list.
///
/// # Safety
/// `dwg` must be live and `obj` one of its entity objects.
unsafe fn style_overrides(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    text: &TextDecoder,
) -> Vec<StyleOverride> {
    let mut list: *mut libredwg_sys::uncad_style_override_t = std::ptr::null_mut();
    // SAFETY: per this function's contract; on success the shim mallocs
    // `list` (`count` entries), freed below. Text pointers in it point into
    // the drawing, which outlives this call.
    let count = unsafe { libredwg_sys::uncad_entity_style_overrides(dwg, obj, &mut list) };
    if list.is_null() {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(list, count as usize) };
    let overrides = raw
        .iter()
        .filter_map(|o| {
            let value = match o.kind {
                0 => OverrideValue::Real(o.real),
                1 => OverrideValue::Integer(o.integer),
                2 if o.text.is_null() => OverrideValue::Text(String::new()),
                2 if o.text_is_wide != 0 => {
                    // SAFETY: text_len UTF-16 units at text, per the shim.
                    let units = unsafe {
                        std::slice::from_raw_parts(o.text.cast::<u16>(), o.text_len as usize)
                    };
                    OverrideValue::Text(String::from_utf16_lossy(units))
                }
                2 => {
                    // SAFETY: text_len bytes at text, per the shim.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(o.text.cast::<u8>(), o.text_len as usize)
                    };
                    OverrideValue::Text(text.decode(bytes, || {
                        format!("style override {} of an entity", o.variable)
                    }))
                }
                3 => OverrideValue::Handle(format!("{:X}", o.handle)),
                _ => return None,
            };
            Some(StyleOverride {
                variable: o.variable,
                value,
            })
        })
        .collect();
    unsafe { libredwg_sys::uncad_free_style_overrides(list) };
    overrides
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
        let vertices: Vec<PolylineVertex> =
            unsafe { read_raw_array(path.polyline_paths, path.num_segs_or_paths) }
                .into_iter()
                .map(|v| PolylineVertex {
                    point: Point2D {
                        x: v.point.x,
                        y: v.point.y,
                    },
                    bulge: v.bulge,
                    // A HATCH boundary has no widths.
                    ..PolylineVertex::default()
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
            end: p2(seg.second_endpoint),
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
            let points = unsafe { read_raw_array(seg.control_points, seg.num_control_points) };
            let rational = seg.is_rational != 0;
            // SAFETY: num_knots/knots and num_fitpts/fitpts, the same
            // convention.
            let knots = unsafe { read_raw_array(seg.knots, seg.num_knots) };
            let fit_points: Vec<Point2D> = unsafe { read_raw_array(seg.fitpts, seg.num_fitpts) }
                .into_iter()
                .map(p2)
                .collect();
            // The tangents are stated with the fit points and only then.
            let fitted = !fit_points.is_empty();
            HatchEdge::Spline {
                degree: seg.degree,
                rational,
                periodic: seg.is_periodic != 0,
                knots,
                control_points: points.iter().map(|cp| p2(cp.point)).collect(),
                weights: if rational {
                    points.iter().map(|cp| cp.weight).collect()
                } else {
                    Vec::new()
                },
                fit_points,
                start_tangent: fitted.then(|| p2(seg.start_tangent)),
                end_tangent: fitted.then(|| p2(seg.end_tangent)),
            }
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

/// Resolves one gradient stop's `Dwg_Color` to packed 24-bit RGB. Same
/// truecolor-overrides-ACI precedence as [`entity_color`], but a gradient stop
/// is never BYLAYER/BYBLOCK, so it needs no layer or inherited-color context.
/// An ACI index outside the palette (a malformed stop) reads as black, as it
/// always has; what the fill then looks like is the renderer's call.
fn hatch_stop_color(c: &libredwg_sys::Dwg_HATCH_Color) -> u32 {
    if c.color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR {
        return c.color.rgb & 0xff_ffff;
    }
    uncad_model::color::aci_to_rgb(c.color.index.unsigned_abs()).unwrap_or(0)
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
    // The stops as the file states them; how a single-color gradient fades
    // toward white by `tint` is a renderer's derivation, not a color the
    // file carries.
    let (color1, color2, tint) = if single_color_gradient {
        (hatch_stop_color(colors.first()?), None, gradient_tint)
    } else if colors.len() >= 2 {
        // shift_value (0.0-1.0) orders the stops; `colors` is not guaranteed
        // to already be sorted by it.
        let mut sorted: Vec<&libredwg_sys::Dwg_HATCH_Color> = colors.iter().collect();
        sorted.sort_by(|a, b| a.shift_value.total_cmp(&b.shift_value));
        (
            hatch_stop_color(sorted[0]),
            Some(hatch_stop_color(sorted[sorted.len() - 1])),
            0.0,
        )
    } else {
        let color1 = hatch_stop_color(colors.first()?);
        (color1, Some(color1), 0.0)
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
        tint,
    })
}

/// Reads an MLINE's vertices and its MLINESTYLE reference. `mlinestyle_name` is
/// resolved here but only looked up against
/// [`crate::tables::Tables::mlinestyles`] at render time -- `Tables` is not
/// built yet while entities are converted, the same reason BYLAYER color
/// resolution is deferred.
fn convert_mline(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
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
    let mlinestyle_name = reference(
        dwg,
        text,
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "MLINE", "mlinestyle"),
        c"MLINESTYLE",
        |handle_ptr| text.handle_name(dwg, handle_ptr),
    );
    MLineEntity {
        common,
        vertices,
        closed: flags & MLINE_CLOSED_FLAG != 0,
        mlinestyle_name,
        // DXF 40, which the format requires: the factor the style's
        // offsets are drawn at. Carried as stated -- a 0 collapses every
        // element onto the centreline, which is what the file says.
        scale: get_field::<f64>(entity_ptr, "MLINE", "scale"),
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
/// (bits.c), which `common_entity_data.spec` uses from R2004 on, reads `rgb`
/// under this bit and never assigns `method` at all.
const COLOR_FLAG_INLINE_RGB: u16 = 0x80;

/// `Dwg_Color.flag` bit: a DBCOLOR object handle follows *instead of* an
/// inline RGB. That object is not converted, so there is no colour to report
/// and whatever `rgb` holds is not it.
const COLOR_FLAG_COLOR_HANDLE: u16 = 0x40;

/// Reads the common `color` (`Dwg_Color`) field and splits it into
/// `(color_index, true_color)` per [`EntityCommon`].
/// An entity's linetype from its common `ltype_flags` (R2000+; the decoder
/// derives them for R13/R14 from the by-layer bit, 0 or 3): 0 BYLAYER, 1 BYBLOCK,
/// 2 the CONTINUOUS linetype -- which the entity does not reference itself,
/// so its name is the one the header's continuous handle resolves to -- and
/// 3 the linetype the entity's own handle names.
fn entity_linetype(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
) -> EntityLinetype {
    let handle = match get_common_field::<u8>(entity_ptr, "ltype_flags") {
        Some(0) | None => return EntityLinetype::ByLayer,
        Some(1) => return EntityLinetype::ByBlock,
        // SAFETY: the shim reads one header field of the live Dwg_Data.
        Some(2) => Some(unsafe { libredwg_sys::uncad_dwg_ltype_continuous(dwg) }),
        Some(_) => get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "ltype"),
    };
    // The LTYPE table has records named BYLAYER and BYBLOCK, which R13 and
    // R14 point at by handle where later releases use the flags.
    match reference(dwg, text, handle, c"LTYPE", |handle_ptr| {
        text.handle_name(dwg, handle_ptr)
    }) {
        Ref::Resolved(name) if name.eq_ignore_ascii_case("BYLAYER") => EntityLinetype::ByLayer,
        Ref::Resolved(name) if name.eq_ignore_ascii_case("BYBLOCK") => EntityLinetype::ByBlock,
        other => EntityLinetype::Named(other),
    }
}

/// An entity's lineweight (DXF 370) from the library's `linewt` code: 0 to
/// 23 the standard weights, 29 BYLAYER, 30 BYBLOCK, 31 the default. `None`
/// for a drawing older than R2000, which has no lineweights, and for the
/// codes the format leaves unused.
fn entity_lineweight(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
) -> Option<i16> {
    if !since(dwg, libredwg_sys::DWG_VERSION_TYPE_R_2000b) {
        return None;
    }
    match get_common_field::<u8>(entity_ptr, "linewt")? {
        29 => Some(-1),
        30 => Some(-2),
        31 => Some(-3),
        code => LINEWEIGHTS.get(usize::from(code)).copied(),
    }
}

/// An entity's transparency (DXF 440) as stored -- the same 32-bit value in
/// a DWG and a DXF -- or `None` for a drawing older than R2004, which has
/// none.
fn entity_transparency(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
) -> Option<u32> {
    if !since(dwg, libredwg_sys::DWG_VERSION_TYPE_R_2004a) {
        return None;
    }
    let color = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color")?;
    Some(color.alpha_raw)
}

/// Whether the drawing was read from `version` or later.
#[allow(clippy::unnecessary_cast)] // the enum's width differs by target
fn since(dwg: *mut libredwg_sys::Dwg_Data, version: libredwg_sys::DWG_VERSION_TYPE) -> bool {
    // SAFETY: the shim reads one header field of a live Dwg_Data.
    (unsafe { libredwg_sys::uncad_dwg_from_version(dwg) }) >= version as i32
}

fn entity_color(entity_ptr: *mut std::ffi::c_void) -> (i16, Option<u32>) {
    let Some(color) = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color") else {
        return (256, None); // no color field at all -- BYLAYER default
    };
    split_entity_color(color.index, color.flag, color.method, color.rgb, |index| {
        // SAFETY: a pure lookup in the library's static palette; any index
        // is accepted (256 and above answer 0).
        unsafe { libredwg_sys::dwg_rgb_palette_index(index) }
    })
}

/// Decides whether a `Dwg_Color` read off an *entity* really states a direct
/// RGB, from the three fields the two readers fill differently. Testing
/// `method == TRUECOLOR` alone is wrong in both directions:
///
/// * **DWG, R2004+** -- `bit_read_ENC` puts the 420 value in `rgb` under
///   `flag & 0x80` and leaves `method` at 0, so a real true colour was
///   dropped: no entity of any corpus DWG reported one.
/// * **DXF** -- `dxf_set_CMC_index` (in_dxf.c) answers a plain group 62 with
///   `method = 0xc3` and an `rgb` *synthesised* from LibreDWG's own ACI
///   palette, so an entity that states only an index was reported as
///   carrying an RGB the file never wrote. A real group 420 instead takes the
///   `color.method = value >> 24` path, which is 0 for a plain 24-bit value.
///
/// `palette` is the RGB the library synthesises for an ACI index -- its own
/// table (`dwg_rgb_palette_index`), which is not the display palette the
/// model publishes; the two disagree on most indices, and only the
/// library's tells a synthesised RGB from a stated one.
///
/// Two cases stay indistinguishable from the fields available and are
/// reported as "no true colour", which renders identically either way: a
/// group 420 of pure black (`rgb` 0 is also what an untouched field holds),
/// and a group 420 that repeats the library's RGB for the entity's own ACI
/// index exactly.
fn split_entity_color(
    index: i16,
    flag: u16,
    method: libredwg_sys::Dwg_Color_Method,
    rgb: u32,
    palette: impl Fn(u16) -> u32,
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
    let from_palette = u16::try_from(index).is_ok_and(|i| palette(i) & 0xff_ffff == rgb24);
    (
        index,
        (tagged_rgb && rgb24 != 0 && !from_palette).then_some(rgb24),
    )
}

/// # Safety
/// `obj` must be a valid, non-null pointer from `dwg_get_object`.
/// The reference ID this backend mints for an entity, and the file handle
/// it records as provenance.
///
/// The ID is the handle's value: a file's handles are unique within it and
/// stable, so the same entity gets the same ID on every read -- and the same
/// ID whether the drawing is read as DWG or as its DXF twin. An entity with
/// no handle (pre-R13 files may carry none) still needs an ID that is unique
/// and reproducible: its position in the file's object table, in a range no
/// handle reaches (the top bit set), with the provenance field left
/// [`Ref::Absent`]. The test sweep over the corpus checks that the scheme
/// yields no duplicate within any file.
///
/// # Safety
/// `obj` must be a valid, non-null `Dwg_Object`.
pub(crate) unsafe fn entity_identity(
    obj: *mut libredwg_sys::Dwg_Object,
) -> (EntityId, Ref<String>) {
    let mut error = 0i32;
    let handle_ptr = unsafe { libredwg_sys::dwg_object_get_handle(obj, &mut error) };
    let value = if handle_ptr.is_null() || error != 0 {
        0
    } else {
        unsafe { (*handle_ptr).value }
    };
    if value != 0 {
        return (EntityId::new(value), Ref::Resolved(format!("{value:X}")));
    }
    let index = unsafe { libredwg_sys::dwg_object_get_index(obj as *const _, &mut error) };
    (
        EntityId::new(HANDLELESS_ID_BASE | u64::from(index)),
        Ref::Absent,
    )
}

/// The handle (upper-case hex) of the IMAGEDEF `handle_ptr` points at --
/// the key [`uncad_model::Tables::image_definitions`] holds it under -- or
/// `None` when it points at nothing, or at an object of another type.
///
/// # Safety
/// `handle_ptr` must be a non-null `Dwg_Object_Ref` of the live `dwg`.
unsafe fn image_definition_handle(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle_ptr: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<String> {
    let object = unsafe { referenced_object(dwg, handle_ptr) };
    if object.is_null() {
        return None;
    }
    // Cast needed for cross-platform bindgen enum-width consistency.
    let fixedtype =
        unsafe { libredwg_sys::dwg_object_get_fixedtype(object) } as libredwg_sys::DWG_OBJECT_TYPE;
    if fixedtype != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_IMAGEDEF {
        return None;
    }
    match unsafe { entity_identity(object) }.1 {
        Ref::Resolved(handle) => Some(handle),
        _ => None,
    }
}

/// Where the IDs of handle-less entities live: above every possible handle
/// value, so they can never collide with a handle-derived ID.
const HANDLELESS_ID_BASE: u64 = 1 << 63;

/// The reference ID of the entity a handle field points at.
///
/// Mints it through [`entity_identity`], the one scheme this backend has for
/// naming an entity, so the ID a *reference* carries is the same one the
/// entity it names carries. Deriving it from the handle value instead is a
/// second scheme that agrees with the first only where a file has handles:
/// a drawing that carries none (before R13) names its entities by their
/// position in the object table, and a handle copied out of the reference
/// would match nothing in such a file.
///
/// A reference the object table does not answer to is [`Ref::Unresolved`],
/// carrying whatever value it has (`absolute_ref`, else the relative handle
/// the library never resolved -- the two rungs [`reference`] uses), in hex.
/// That is a different fact from "the file names nothing", which is
/// [`Ref::Absent`].
pub(crate) fn entity_reference(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle_ptr: Option<*mut libredwg_sys::Dwg_Object_Ref>,
) -> Ref<EntityId> {
    let Some(handle_ptr) = handle_ptr.filter(|p| !p.is_null()) else {
        return Ref::Absent;
    };
    // SAFETY: a non-null Dwg_Object_Ref owned by the live Dwg_Data this
    // conversion pass walks, the same contract reference() reads under.
    let object = unsafe { referenced_object(dwg, handle_ptr) };
    if !object.is_null() {
        return Ref::Resolved(unsafe { entity_identity(object) }.0);
    }
    // Nothing in the drawing answers to the handle. The handle is kept, the
    // same form an entity's own handle takes: a file that points at an
    // entity it does not carry has said something different from a file
    // that points at nothing.
    let (absolute_ref, handle_value) =
        unsafe { ((*handle_ptr).absolute_ref, (*handle_ptr).handleref.value) };
    match (absolute_ref, handle_value) {
        (0, 0) => Ref::Absent,
        (0, value) | (value, _) => Ref::Unresolved(format!("{value:X}")),
    }
}

/// Turns a handle field into the model's three-state reference: no field or
/// a null handle is [`Ref::Absent`]; a handle the resolver turns into a name
/// is [`Ref::Resolved`]; a handle it cannot is [`Ref::Unresolved`] carrying
/// the handle itself (`absolute_ref`, hex, the same form as
/// [`EntityCommon::source_handle`]). This is the one place the empty-string fill
/// used to happen, for every reference field the model has.
///
/// Two cases carry no handle to resolve by, and are told apart by the
/// drawing's version:
/// - Before R13 a drawing points at its tables by *index* (`r11_idx`), not by
///   handle, so the entry is looked up by index in `table` -- the name of a
///   `*_CONTROL` table (`LAYER`, `BLOCK`, ...; a dictionary-held object type
///   such as `MLINESTYLE` has no such table and simply does not resolve this
///   way, which is moot before R13). An index the table does not answer to is
///   `Unresolved("idx:<n>")` -- the index is kept the way a handle would be.
/// - From R13 on, a handle whose value is zero is a reference the file does
///   not carry (a DIMENSION without a block, for instance): `Absent`.
pub(crate) fn reference(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    handle_ptr: Option<*mut libredwg_sys::Dwg_Object_Ref>,
    table: &CStr,
    resolve: impl FnOnce(*mut libredwg_sys::Dwg_Object_Ref) -> Option<String>,
) -> Ref<String> {
    let Some(handle_ptr) = handle_ptr else {
        return Ref::Absent;
    };
    if handle_ptr.is_null() {
        return Ref::Absent;
    }
    if let Some(name) = resolve(handle_ptr) {
        return Ref::Resolved(name);
    }
    // SAFETY: handle_ptr is a non-null Dwg_Object_Ref owned by the live
    // Dwg_Data this conversion pass walks (same contract as the resolvers
    // that just read it).
    let (absolute_ref, handle_value, r11_idx) = unsafe {
        (
            (*handle_ptr).absolute_ref,
            (*handle_ptr).handleref.value,
            (*handle_ptr).r11_idx,
        )
    };
    if is_pre_r13(dwg) {
        return match text.table_entry_name(dwg, handle_ptr, table) {
            Some(name) => Ref::Resolved(name),
            None => Ref::Unresolved(format!("idx:{r11_idx}")),
        };
    }
    if absolute_ref != 0 {
        return Ref::Unresolved(format!("{absolute_ref:X}"));
    }
    if handle_value != 0 {
        // A relative (offset) handle the library never resolved against its
        // owner: still a reference the file makes, so not `Absent`.
        return Ref::Unresolved(format!("{handle_value:X}"));
    }
    Ref::Absent
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

/// Which coordinate an ordinate dimension measures (DXF 70, bit 64: X when
/// set). A DXF, and a drawing older than R13, state it in `flag`, the group
/// 70 the file wrote. From R13 on a DWG states it as bit 1 of the
/// stream-only `flag2` byte, and the decoder rebuilds `flag` from it wrongly
/// -- it sets bit 128 and clears bit 64 (`dwg.spec`'s DIMENSION_ORDINATE) --
/// so there `flag2` is read. The DXF importer, for its part, never fills
/// `flag2`.
fn ordinate_axis(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
) -> OrdinateAxis {
    let x = if is_from_dxf(dwg) || is_pre_r13(dwg) {
        get_field::<u8>(entity_ptr, dxfname, "flag").is_some_and(|f| f & 0x40 != 0)
    } else {
        get_field::<u8>(entity_ptr, dxfname, "flag2").is_some_and(|f| f & 1 != 0)
    };
    if x {
        OrdinateAxis::X
    } else {
        OrdinateAxis::Y
    }
}

/// Which of this backend's point fields carries DXF group 13, 14, 15 and 16,
/// per dimension subtype. `None` is a group the subtype does not write.
///
/// The library's own names are not a mapping: `xline1_pt` is group 13 for a
/// linear dimension, while a two-line angular dimension calls its group 13
/// `xline1start_pt` and its group 16 `xline2end_pt` -- when the DXF importer
/// filled the record, that is. The two readers disagree on a two-line
/// angular dimension's last two points: the DWG decoder fills the record in
/// stream order (the leading 2RD, `def_pt`, is the arc point, group 16, and
/// `xline2end_pt` the last point, group 10), the DXF importer by group code
/// (`def_pt` 10, `xline2end_pt` 16). `from_dxf` says which reader filled it.
fn dimension_point_fields(
    fixedtype: libredwg_sys::Dwg_Object_Type,
    from_dxf: bool,
) -> (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
) {
    match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => {
            (Some("xline1_pt"), Some("xline2_pt"), None, None)
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT => (
            Some("xline1_pt"),
            Some("xline2_pt"),
            Some("center_pt"),
            None,
        ),
        // Measured against the same drawing in both formats: decoded from
        // the DWG, this library's `def_pt` holds group 16 here, and
        // `xline2end_pt` holds group 10 (the dimension's definition point)
        // rather than 16; read from the DXF, each holds the group its name
        // says. The mapping follows the measurement, not the field names.
        // (An earlier measurement read `xline2end_pt` as group 13's point;
        // that drawing has groups 10 and 13 at the same place, so it could
        // not tell.)
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => (
            Some("xline1start_pt"),
            Some("xline1end_pt"),
            Some("xline2start_pt"),
            Some(if from_dxf { "xline2end_pt" } else { "def_pt" }),
        ),
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => {
            (None, None, Some("first_arc_pt"), None)
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => (
            Some("feature_location_pt"),
            Some("leader_endpt"),
            None,
            None,
        ),
        // Group 16 here is the first leader point. A DWG record stores it
        // whether or not the dimension has a leader (group 71), so it is
        // carried either way -- what the file states; from DXF, see the
        // caller.
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => (
            Some("xline1_pt"),
            Some("xline2_pt"),
            Some("center_pt"),
            Some("leader1_pt"),
        ),
        _ => (None, None, None, None),
    }
}

/// The subtype, from the type the library resolved the entity to rather than
/// from the flag it computes for DXF output.
fn dimension_kind(fixedtype: libredwg_sys::Dwg_Object_Type) -> Option<DimensionKind> {
    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR => DimensionKind::Rotated,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => DimensionKind::Aligned,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => DimensionKind::Angular2Line,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => DimensionKind::Diameter,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS => DimensionKind::Radius,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT => DimensionKind::Angular3Point,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => DimensionKind::Ordinate,
        // Its group 70 says 5, a three-point angular dimension; the entity
        // it is decides it instead.
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => DimensionKind::ArcLength,
        _ => return None,
    })
}

/// DXF 1 folded to one value per meaning: nothing, an empty string and `<>`
/// all mean "show the measurement"; a single space means "show nothing".
fn dimension_text_override(user_text: Option<&str>) -> TextOverride {
    match user_text {
        None | Some("") | Some("<>") => TextOverride::Measured,
        Some(" ") => TextOverride::Suppressed,
        Some(other) => TextOverride::Literal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(x: f64) -> PolylineVertex {
        PolylineVertex::straight(Point2D { x, y: 0.0 })
    }

    fn width(start: f64, end: f64) -> RawSegmentWidth {
        RawSegmentWidth { start, end }
    }

    fn widths_of(vertices: &[PolylineVertex]) -> Vec<(f64, f64)> {
        vertices
            .iter()
            .map(|v| (v.start_width, v.end_width))
            .collect()
    }

    /// An LWPOLYLINE's width array pairs with its points one to one, the
    /// way its bulge array does; the widths go on the vertices.
    #[test]
    fn an_lwpolyline_s_widths_go_on_its_vertices_one_to_one() {
        let text = TextDecoder::for_tests();
        let vertices = with_widths(
            &text,
            "LWPOLYLINE",
            vec![straight(0.0), straight(1.0), straight(2.0)],
            vec![width(0.0, 2.0), width(2.0, 2.0), width(1.0, 0.0)],
            0.0,
        );
        assert_eq!(widths_of(&vertices), [(0.0, 2.0), (2.0, 2.0), (1.0, 0.0)]);
        assert!(text.into_warnings().is_empty());
    }

    /// A file that states the constant width again on every vertex, at both
    /// ends, is the same polyline as one that states it once: its vertices
    /// have no width of their own.
    #[test]
    fn widths_that_all_repeat_the_constant_width_are_no_widths_of_the_vertices() {
        let text = TextDecoder::for_tests();
        let vertices = with_widths(
            &text,
            "LWPOLYLINE",
            vec![straight(0.0), straight(1.0)],
            vec![width(0.5, 0.5), width(0.5, 0.5)],
            0.5,
        );
        assert_eq!(widths_of(&vertices), [(0.0, 0.0), (0.0, 0.0)]);
        // One vertex that differs keeps them all.
        let vertices = with_widths(
            &text,
            "LWPOLYLINE",
            vec![straight(0.0), straight(1.0)],
            vec![width(0.5, 0.5), width(0.5, 0.0)],
            0.5,
        );
        assert_eq!(widths_of(&vertices), [(0.5, 0.5), (0.5, 0.0)]);
        assert!(text.into_warnings().is_empty());
    }

    /// A width array of another length than the points' does not say which
    /// width is whose: none is used, and the read says so -- the rule the
    /// bulge array has (`POLYLINE_BULGE`).
    #[test]
    fn a_width_array_that_does_not_match_the_points_is_reported_and_not_used() {
        let text = TextDecoder::for_tests();
        let vertices = with_widths(
            &text,
            "LWPOLYLINE",
            vec![straight(0.0), straight(1.0), straight(2.0)],
            vec![width(1.0, 1.0), width(1.0, 1.0)],
            0.0,
        );
        assert_eq!(widths_of(&vertices), [(0.0, 0.0); 3]);
        assert_eq!(
            text.into_warnings(),
            ["POLYLINE_WIDTH: a LWPOLYLINE stores 2 widths for 3 vertices; its vertices are read with no width of their own"]
        );
    }

    const VOID: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_VOID;
    const BYLAYER: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_BYLAYER;
    const BYBLOCK: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_BYBLOCK;
    const ACI: libredwg_sys::Dwg_Color_Method = libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI;
    const TRUECOLOR: libredwg_sys::Dwg_Color_Method =
        libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR;

    /// The library's own palette, through the same lookup the conversion
    /// uses.
    fn split(
        index: i16,
        flag: u16,
        method: libredwg_sys::Dwg_Color_Method,
        rgb: u32,
    ) -> (i16, Option<u32>) {
        split_entity_color(index, flag, method, rgb, |i| unsafe {
            libredwg_sys::dwg_rgb_palette_index(i)
        })
    }

    /// Each case below is the `(index, flag, method, rgb)` LibreDWG leaves in
    /// `Dwg_Color` for one way of writing a colour, read off the reader that
    /// writes it: `bit_read_ENC` / `common_entity_data.spec` for the DWG
    /// rows and `dxf_set_CMC_index` / the group-420 arm of the common-entity
    /// loop in `in_dxf.c` for the DXF ones.
    #[test]
    fn split_entity_color_follows_what_each_reader_actually_stores() {
        // --- R2004+ DWG (bit_read_ENC): flag 0x80 says an RGB follows, and
        // nothing ever sets `method` on this path.
        assert_eq!(split(256, 0x80, VOID, 0x00_ff7f), (256, Some(0x00_ff7f)));
        // The stored BL may carry a method byte of its own; only the low 24
        // bits are the colour (which is what the DXF writer emits for 420).
        assert_eq!(split(256, 0x80, VOID, 0xc200_ff7f), (256, Some(0x00_ff7f)));
        // flag 0x40 is a DBCOLOR handle *instead of* an inline RGB (the spec
        // reads one or the other), and that object is not converted.
        assert_eq!(split(256, 0xc0, VOID, 0x00_ff7f), (256, None));
        // Plain BYLAYER: no flag bits, rgb zeroed by the decoder.
        assert_eq!(split(256, 0, VOID, 0), (256, None));

        // --- DXF group 62 only (dxf_set_CMC_index): method 0xc3 with `rgb`
        // synthesised from LibreDWG's ACI palette. ACI 1 is 0xff0000 there,
        // and ACI 8 -- where the library's table and the model's display
        // palette disagree -- is 0x414141.
        assert_eq!(
            split(1, 0, TRUECOLOR, 0xc300_0000 | 0xff_0000),
            (1, None),
            "an index-only entity states no RGB"
        );
        assert_eq!(split(8, 0, TRUECOLOR, 0xc341_4141), (8, None));
        // ... and the same for BYLAYER / BYBLOCK / none, which that function
        // spells with an empty rgb.
        assert_eq!(
            split(256, 0, ACI, 0xc200_0000),
            (256, None),
            "0xc2 with no RGB is how the DXF reader spells BYLAYER"
        );
        assert_eq!(split(0, 0, BYBLOCK, 0xc100_0000), (0, None));
        assert_eq!(split(256, 0, BYLAYER, 0xc000_0000), (256, None));

        // --- DXF group 420. The common-entity arm stores the value verbatim
        // and takes the method from its top byte, so a plain 24-bit RGB
        // arrives with method 0 and a pre-tagged one with 0xc2 or 0xc3.
        assert_eq!(split(256, 0, VOID, 65407), (256, Some(0x00_ff7f)));
        assert_eq!(
            split(3, 0, ACI, 0xc200_ff7f),
            (3, Some(0x00_ff7f)),
            "a 420 override wins over the entity's own group 62"
        );
        assert_eq!(split(3, 0, TRUECOLOR, 0xc300_ff7f), (3, Some(0x00_ff7f)));
        // Group 420 of 257 is the reader's "none": method 0xc8, rgb 0.
        assert_eq!(
            split(
                256,
                0,
                libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_NONE,
                0xc800_0000
            ),
            (256, None)
        );
    }

    #[test]
    fn the_librarys_palette_is_its_own_not_the_models() {
        // The reason the lookup goes through the library: its ACI 8 is
        // 0x414141, the model's display palette says 0x808080.
        assert_eq!(unsafe { libredwg_sys::dwg_rgb_palette_index(8) }, 0x41_4141);
        assert_eq!(uncad_model::color::aci_to_rgb(8), Some(0x80_8080));
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
        assert_eq!(hatch_stop_color(&stop), 0x00ff00);
    }

    #[test]
    fn hatch_stop_color_falls_back_to_aci_index() {
        assert_eq!(hatch_stop_color(&aci_stop(0.0, 1)), 0xff0000); // ACI 1 = red
    }

    #[test]
    fn gradient_two_color_orders_stops_by_shift_value_not_array_order() {
        // colors[0] is the *second* stop (shift_value 1.0); the array is not
        // guaranteed to already be sorted.
        let colors = [aci_stop(1.0, 5), aci_stop(0.0, 1)];
        let g = convert_hatch_gradient(0.0, false, 0.0, "LINEAR", &colors).unwrap();
        assert_eq!(g.color1, 0xff0000); // ACI 1, shift 0.0
        assert_eq!(g.color2, Some(0x0000ff)); // ACI 5, shift 1.0
        assert_eq!(g.tint, 0.0);
        assert!(!g.is_radial);
    }

    #[test]
    fn gradient_single_color_carries_the_tint_and_no_second_stop() {
        // ACI 7 is white in the palette; whether a renderer flips it for a
        // white background is the renderer's decision, so the stop is white
        // here and the second stop is left to the renderer to derive.
        let colors = [aci_stop(0.0, 7)];
        let g = convert_hatch_gradient(0.0, true, 0.5, "LINEAR", &colors).unwrap();
        assert_eq!(g.color1, 0xffffff);
        assert_eq!(g.color2, None);
        assert_eq!(g.tint, 0.5);
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
