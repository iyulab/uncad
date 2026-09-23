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
    get_point3d_array, is_pre_r13, is_r2010_or_later, is_r2013_or_later, SplineControlPoint,
};
use crate::text::TextDecoder;
use std::ffi::CStr;
use uncad_model::model::{
    AcadTableEntity, ArcEntity, AttdefEntity, AttribEntity, CircleEntity, Confidence,
    DimensionEntity, DimensionKind, DimensionPoints, EllipseEntity, Entity, EntityCommon, EntityId,
    Face3DEntity, HatchBoundaryPath, HatchEdge, HatchEntity, HatchGradient, HatchPatternLine,
    InsertEntity, LeaderAnnotation, LeaderEntity, LeaderPath, LightEntity, LightType, LineEntity,
    LwPolylineEntity, MLineEntity, MLineVertex, MTextAttachment, MTextEntity, MultiLeaderEntity,
    Origin, PointEntity, PolylineEntity, RayEntity, Ref, Solid3DEntity, SolidEntity, SplineEntity,
    TextEntity, TextHorizontalAlignment, TextOverride, TextVerticalAlignment, ToleranceEntity,
    ViewportEntity, WipeoutEntity,
};
use uncad_model::model::{Point2D, Point3D, PolylineVertex};

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
        if let Some(entity) = unsafe { convert_entity(dwg, text, owned) } {
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
    matches!(
        fixedtype,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE_FACE
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND
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
            if let Some(entity) = unsafe { convert_entity(dwg, text, obj) } {
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
/// # Safety
/// `dwg` must be live and `entity_ptr` the type-specific struct pointer of a
/// valid INSERT object of it.
unsafe fn chained_insert_attribs(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
) -> Vec<AttribEntity> {
    let mut attribs = Vec::new();
    let owned = get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
        entity_ptr,
        "INSERT",
        "num_owned",
        "attribs",
    );
    if !owned.is_empty() {
        for reference in owned {
            let sub = unsafe { referenced_object(dwg, reference) };
            if sub.is_null() {
                continue;
            }
            if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, text, sub) } {
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
    while !sub.is_null() && steps <= max_steps {
        steps += 1;
        let fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        if fixedtype != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB {
            break;
        }
        if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, text, sub) } {
            attribs.push(attrib);
        }
        if sub == last_obj {
            break;
        }
        sub = unsafe { libredwg_sys::dwg_next_entity(sub) };
    }
    attribs
}

/// A 2D or 3D POLYLINE's VERTEX records, in order.
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
/// The second value says whether the records ran to the polyline's SEQEND.
/// Before R13 they may not: when the entity section continues elsewhere (a
/// JUMP entity), the library's object stream can end at the JUMP, before the
/// vertices that follow it in the file -- the polyline then has fewer
/// vertices than the file gives it, and the caller says so.
///
/// # Safety
/// `obj` must be a valid `POLYLINE_2D`/`POLYLINE_3D` object of `dwg`.
unsafe fn polyline_vertex_records(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
) -> (Vec<*mut libredwg_sys::Dwg_Object>, bool) {
    let mut records = Vec::new();
    if is_pre_r13(dwg) {
        let max_steps = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
        let mut sub = unsafe { libredwg_sys::dwg_next_object(obj) };
        let mut steps = 0;
        while !sub.is_null() && steps <= max_steps {
            steps += 1;
            let fixedtype = unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) }
                as libredwg_sys::DWG_OBJECT_TYPE;
            match fixedtype {
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND => return (records, true),
                libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D
                | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D => records.push(sub),
                // Anything else is not this polyline's: its vertex run ended
                // without its SEQEND.
                _ => break,
            }
            sub = unsafe { libredwg_sys::dwg_next_object(sub) };
        }
        (records, false)
    } else {
        let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
        while !sub.is_null() {
            records.push(sub);
            sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
        }
        (records, true)
    }
}

/// The positions (and whatever else `read` takes) of a polyline's vertex
/// records of type `vertex_type`, in order. A record of another type -- a
/// polyface's face record, say -- is not a vertex and is skipped. When the
/// records end before the polyline's SEQEND the read reports
/// `POLYLINE_VERTICES`, since the polyline is then missing vertices the file
/// gives it.
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
    let (records, complete) = unsafe { polyline_vertex_records(dwg, obj) };
    if !complete {
        let (id, _) = unsafe { entity_identity(obj) };
        text.warn(format!(
            "POLYLINE_VERTICES: the vertex records of the POLYLINE {:X} end before its SEQEND; it is read with the {} vertices that were found",
            id.value(),
            records.len()
        ));
    }
    records
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

/// Where a TEXT, ATTRIB or ATTDEF record (`dxfname`) is aligned: its two
/// alignment codes (DXF 72, and 73 -- 74 in an attribute's DXF form) and
/// its alignment point, which the record stores only for an alignment other
/// than left and baseline. A code outside the format's range is reported and
/// read as the default.
fn placement(
    text: &TextDecoder,
    entity_ptr: *mut std::ffi::c_void,
    dxfname: &str,
) -> (
    TextHorizontalAlignment,
    TextVerticalAlignment,
    Option<Point2D>,
) {
    let horizontal = get_field::<u16>(entity_ptr, dxfname, "horiz_alignment").unwrap_or(0);
    let vertical = get_field::<u16>(entity_ptr, dxfname, "vert_alignment").unwrap_or(0);
    let vertical_group = if dxfname == "TEXT" { 73 } else { 74 };
    let h = match horizontal {
        0 => TextHorizontalAlignment::Left,
        1 => TextHorizontalAlignment::Center,
        2 => TextHorizontalAlignment::Right,
        3 => TextHorizontalAlignment::Aligned,
        4 => TextHorizontalAlignment::Middle,
        5 => TextHorizontalAlignment::Fit,
        other => {
            text.warn(format!(
                "TEXT_ALIGNMENT: a {dxfname} states horizontal alignment {other} (group 72), outside 0 to 5; it is read as left"
            ));
            TextHorizontalAlignment::Left
        }
    };
    let v = match vertical {
        0 => TextVerticalAlignment::Baseline,
        1 => TextVerticalAlignment::Bottom,
        2 => TextVerticalAlignment::Middle,
        3 => TextVerticalAlignment::Top,
        other => {
            text.warn(format!(
                "TEXT_ALIGNMENT: a {dxfname} states vertical alignment {other} (group {vertical_group}), outside 0 to 3; it is read as baseline"
            ));
            TextVerticalAlignment::Baseline
        }
    };
    let point = ((h, v)
        != (
            TextHorizontalAlignment::Left,
            TextVerticalAlignment::Baseline,
        ))
        .then(|| get_point2d(entity_ptr, dxfname, "alignment_pt"))
        .flatten();
    (h, v, point)
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
        })
        .collect()
}

/// An entity's extrusion direction (DXF 210): the normal of its plane, and
/// for an entity stored in its own coordinate system the Z axis of that
/// system. The default is the world Z axis.
fn extrusion(entity_ptr: *mut std::ffi::c_void, dxfname: &str) -> Point3D {
    get_point3d(entity_ptr, dxfname, "extrusion").unwrap_or(Point3D {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    })
}

/// Resolves a POLYLINE_PFACE's mesh into wireframe edges by walking its owned
/// `VERTEX_PFACE` (vertex positions, in order) and `VERTEX_PFACE_FACE` (up to
/// 4 vertex indices per face, 1-based, negative meaning "invisible edge" --
/// the sign carries no other meaning, so it is just dropped) subentities
/// directly; LibreDWG's own accessor for this type is documented as not
/// implemented.
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

    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    while !sub.is_null() {
        let sub_fixedtype =
            unsafe { libredwg_sys::dwg_object_get_fixedtype(sub) } as libredwg_sys::DWG_OBJECT_TYPE;
        let sub_entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
        if !sub_entity_ptr.is_null() {
            if sub_fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE {
                if let Some(p) = get_point3d(sub_entity_ptr, "VERTEX_PFACE", "point") {
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
        sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
    }

    let mut edges = Vec::new();
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
            if let (Some(&pa), Some(&pb)) = (positions.get(a), positions.get(b)) {
                edges.push([pa, pb]);
            }
        }
    }
    edges
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
    let clip_verts: Vec<Point2D> =
        get_point2d_array::<u32>(entity_ptr, "WIPEOUT", "num_clip_verts", "clip_verts");
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
        let size =
            get_point2d(entity_ptr, "WIPEOUT", "image_size").unwrap_or(Point2D { x: 0.0, y: 0.0 });
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

/// # Safety
/// `dwg` must be the live `Dwg_Data` `obj` was obtained from; `obj` must be a
/// valid pointer from `dwg_get_object` on that same `Dwg_Data`.
unsafe fn convert_entity(
    dwg: *mut libredwg_sys::Dwg_Data,
    text: &TextDecoder,
    obj: *mut libredwg_sys::Dwg_Object,
) -> Option<Entity> {
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
            let text_value = text
                .field(entity_ptr, "TEXT", "text_value")
                .unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0);
            let (horizontal_alignment, vertical_alignment, alignment_point) =
                placement(text, entity_ptr, "TEXT");
            Entity::Text(TextEntity {
                common,
                start_point,
                text_height,
                text: text_value,
                rotation,
                horizontal_alignment,
                vertical_alignment,
                alignment_point,
                width_factor: get_field::<f64>(entity_ptr, "TEXT", "width_factor").unwrap_or(1.0),
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
            Entity::LwPolyline(LwPolylineEntity {
                common,
                vertices,
                closed: flag & LWPOLYLINE_CLOSED_FLAG != 0,
                elevation: get_field::<f64>(entity_ptr, "LWPOLYLINE", "elevation").unwrap_or(0.0),
                extrusion: extrusion(entity_ptr, "LWPOLYLINE"),
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
            let (horizontal_alignment, vertical_alignment, alignment_point) =
                placement(text, entity_ptr, "ATTRIB");
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
                horizontal_alignment,
                vertical_alignment,
                alignment_point,
                width_factor: get_field::<f64>(entity_ptr, "ATTRIB", "width_factor").unwrap_or(1.0),
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
                unsafe { chained_insert_attribs(dwg, text, entity_ptr) }
            } else {
                let mut attribs = Vec::new();
                let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
                while !sub.is_null() {
                    if let Some(Entity::Attrib(attrib)) = unsafe { convert_entity(dwg, text, sub) }
                    {
                        attribs.push(attrib);
                    }
                    sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
                }
                attribs
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
            let (horizontal_alignment, vertical_alignment, alignment_point) =
                placement(text, entity_ptr, "ATTDEF");
            Entity::Attdef(AttdefEntity {
                common,
                start_point,
                text_height,
                tag,
                default_value,
                rotation,
                horizontal_alignment,
                vertical_alignment,
                alignment_point,
                width_factor: get_field::<f64>(entity_ptr, "ATTDEF", "width_factor").unwrap_or(1.0),
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
            let knots = if by_control_points {
                get_array_field::<u32, f64>(entity_ptr, "SPLINE", "num_knots", "knots")
            } else {
                Vec::new()
            };
            // Weights are stored only when the `weighted` bit is set; the
            // library leaves `w` at 0 otherwise, which is not a weight. No
            // weights means every weight is 1.
            let weights = if bit("weighted") == Some(true) {
                control.iter().map(|p| p.w).collect()
            } else {
                Vec::new()
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
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MTEXT => {
            let insertion_point = get_point3d(entity_ptr, "MTEXT", "ins_pt")?;
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
                reference_width: get_field::<f64>(entity_ptr, "MTEXT", "rect_width").unwrap_or(0.0),
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
            // Each VERTEX_2D carries its own bulge (the segment to the next
            // vertex); an unreadable one is a straight segment.
            // SAFETY: obj is a POLYLINE_2D of dwg per fixedtype.
            let vertices: Vec<PolylineVertex> = unsafe {
                polyline_vertices(
                    dwg,
                    text,
                    obj,
                    libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D,
                    |v| {
                        let p = get_point3d(v, "VERTEX_2D", "point")?;
                        Some(PolylineVertex {
                            point: Point2D { x: p.x, y: p.y },
                            bulge: get_field::<f64>(v, "VERTEX_2D", "bulge").unwrap_or(0.0),
                        })
                    },
                )
            };
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_2D", "flag").unwrap_or(0);
            Entity::Polyline2D(LwPolylineEntity {
                common,
                vertices,
                closed: flag & POLYLINE_CLOSED_FLAG != 0,
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
            let (p13, p14, p15, p16) = dimension_point_fields(fixedtype);
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
                measurement: get_field::<f64>(entity_ptr, dxfname, "act_measurement")
                    .filter(|m| *m != 0.0),
                text_override: dimension_text_override(
                    text.field(entity_ptr, dxfname, "user_text").as_deref(),
                ),
                // A two-line angular dimension keeps group 10 in the record's
                // last point, which this library names `xline2end_pt` (its
                // `def_pt` holds group 16 -- see `dimension_point_fields`).
                definition_point: if kind == Some(DimensionKind::Angular2Line) {
                    get_point3d(entity_ptr, dxfname, "xline2end_pt")
                } else {
                    get_point3d(entity_ptr, dxfname, "def_pt")
                },
                text_midpoint: get_point2d(entity_ptr, dxfname, "text_midpt").unwrap_or_default(),
                points: DimensionPoints {
                    extension1: point(p13),
                    extension2: point(p14),
                    radial: point(p15),
                    arc: point(p16).or_else(|| {
                        // An arc-length dimension's group 16 is its first
                        // leader point, and it has one only when it says so.
                        let has_leader = kind == Some(DimensionKind::ArcLength)
                            && get_field::<u8>(entity_ptr, dxfname, "has_leader")
                                .is_some_and(|v| v != 0);
                        has_leader
                            .then(|| get_point3d(entity_ptr, dxfname, "leader1_pt"))
                            .flatten()
                    }),
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
            // SAFETY: obj is a valid, non-null POLYLINE_PFACE Dwg_Object*
            // (matching fixedtype); the helper only walks its owned-subentity
            // chain.
            let wireframe_edges = unsafe { polyline_pface_wireframe(obj) };
            Entity::PolylinePFace(Solid3DEntity {
                common,
                wireframe_edges,
                // A polyface mesh has no ACIS data to skip edges from.
                skipped_edges: 0,
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
        let vertices: Vec<PolylineVertex> =
            unsafe { read_raw_array(path.polyline_paths, path.num_segs_or_paths) }
                .into_iter()
                .map(|v| PolylineVertex {
                    point: Point2D {
                        x: v.point.x,
                        y: v.point.y,
                    },
                    bulge: v.bulge,
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
        sorted.sort_by(|a, b| {
            a.shift_value
                .partial_cmp(&b.shift_value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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

/// Reads the common `color` (`Dwg_Color`) field and splits it into
/// `(color_index, true_color)` per [`EntityCommon`].
fn entity_color(entity_ptr: *mut std::ffi::c_void) -> (i16, Option<u32>) {
    let Some(color) = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color") else {
        return (256, None); // no color field at all -- BYLAYER default
    };
    let true_color = (color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR)
        .then_some(color.rgb & 0xff_ffff);
    (color.index, true_color)
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
unsafe fn entity_identity(obj: *mut libredwg_sys::Dwg_Object) -> (EntityId, Ref<String>) {
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
fn entity_reference(
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
fn reference(
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

/// Which of this backend's point fields carries DXF group 13, 14, 15 and 16,
/// per dimension subtype. `None` is a group the subtype does not write.
///
/// The library's own names are not a mapping: `xline1_pt` is group 13 for a
/// linear dimension, while a two-line angular dimension calls its group 13
/// `xline1start_pt` and its group 16 `xline2end_pt`.
fn dimension_point_fields(
    fixedtype: libredwg_sys::Dwg_Object_Type,
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
        // Measured against the same drawing in both formats: this
        // library's `def_pt` holds group 16 here, and `xline2end_pt` holds
        // group 10 (the dimension's definition point) rather than 16. The
        // mapping follows the measurement, not the field names. (An earlier
        // measurement read `xline2end_pt` as group 13's point; that drawing
        // has groups 10 and 13 at the same place, so it could not tell.)
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => (
            Some("xline1start_pt"),
            Some("xline1end_pt"),
            Some("xline2start_pt"),
            Some("def_pt"),
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
        // Group 16 here is the first leader point, which the format writes
        // only when the dimension has a leader at all (group 71) -- the
        // field is there either way, holding zeros when it does not.
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => (
            Some("xline1_pt"),
            Some("xline2_pt"),
            Some("center_pt"),
            None,
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
