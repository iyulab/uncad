//! Walks a parsed `Dwg_Data`'s objects and converts entities into
//! [`RenderEntity`] values. Mirrors `converter.ts`'s `switch (fixedtype)`
//! dispatch and `entityConverter.ts`'s per-type functions for most types
//! (see `docs/CAVEATS.md` for current entity-type coverage, and which
//! types are JS-baseline ports vs. new functionality beyond it).
//!
//! **Not** a flat global object scan. `db.entities` in the JS baseline is
//! built by walking `BLOCK_HEADER` (BLOCK_RECORD) objects and only
//! collecting entities owned by the `*Model_Space`/`*Paper_Space*` blocks
//! (`converter.ts:133-160`) -- entities that live inside a *named* block
//! definition (i.e. a symbol referenced by INSERT elsewhere) are
//! deliberately excluded from the top-level list; they stay reachable only
//! via that block's own owned-entity chain. A naive global fixedtype scan
//! (this module's Phase 1/2-early approach) silently over-collects: it
//! picks up every entity inside every block definition too, which only
//! went unnoticed on `sample_2000.dwg` because that fixture has no
//! INSERT-referenced blocks with real content. Confirmed by diffing
//! against the JS baseline on `example_r14.dwg` (10 INSERTs): a POINT
//! (handle `1BC`) turned up in the naive walk with no corresponding entry
//! in `db.entities` at all -- it belongs to a named block, not model/paper
//! space.

use crate::dynapi::{
    get_array_field, get_common_field, get_field, get_utf8_field, resolve_handle_name, Point2D,
    Point3D, SplineControlPoint,
};
use crate::render_model::{
    AcadTableEntity, ArcEntity, AttdefEntity, AttribEntity, CircleEntity, DimensionEntity,
    EllipseEntity, EntityCommon, Face3DEntity, HatchBoundaryPath, HatchEdge, HatchEntity,
    HatchGradient, HatchPatternLine, InsertEntity, LeaderEntity, LightEntity, LineEntity,
    LwPolylineEntity, MLineEntity, MLineVertex, MTextEntity, MultiLeaderEntity, PointEntity,
    PolylineEntity, RayEntity, RenderEntity, Solid3DEntity, SolidEntity, SplineEntity, TextEntity,
    ToleranceEntity, ViewportEntity, WipeoutEntity,
};
use std::ffi::CStr;

/// LWPOLYLINE.flag bit checked for "closed". dwg.h's own field comment on
/// Dwg_Entity_LWPOLYLINE documents bit 512 as "closed", but the JS
/// baseline's toSVG() actually checks bit 1 (`e.flag & 1`, matching the
/// *standard DXF group-70* closed-bit convention, not LibreDWG's comment).
/// Confirmed by diffing rendered SVG output against the JS baseline on the
/// sample_2000.dwg fixture used during development (no longer bundled with
/// the repo -- see docs/CAVEATS.md): its one LWPOLYLINE had flag=512 (closed
/// per dwg.h's comment) and JS rendered it as an *open* `<polyline>`, not a
/// `<polygon>` -- matched here rather than "corrected" to 512, since there's
/// no independently-verified ground truth (a real AutoCAD screenshot) to
/// confirm which interpretation is actually right, and the whole point of
/// porting against the JS baseline is byte-for-byte behavioral parity, not
/// silently diverging on an unverified guess.
const LWPOLYLINE_CLOSED_FLAG: u16 = 1;

/// `name.to_uppercase() == "*MODEL_SPACE"` -- exact match, matching
/// `converter.ts`'s `utils.ts::isModelSpace`.
fn is_model_space(name: &str) -> bool {
    name.to_uppercase() == "*MODEL_SPACE"
}

/// `name.to_uppercase().starts_with("*PAPER_SPACE")` -- matches
/// `*Paper_Space`, `*Paper_Space0`, `*Paper_Space1`, ..., mirroring
/// `converter.ts`'s `utils.ts::isPaperSpace`.
fn is_paper_space(name: &str) -> bool {
    name.to_uppercase().starts_with("*PAPER_SPACE")
}

/// Walks every `BLOCK_HEADER` object, collects entities owned by the
/// `*Model_Space`/`*Paper_Space*` ones (see this module's doc comment for
/// why not every block qualifies), and converts each. Entities of an
/// unhandled type become [`RenderEntity::Unknown`] (counted, not dropped) --
/// a deliberate deviation from the JS baseline, which silently drops
/// entities its `entityConverter.ts` has no case for (`convertEntities()`
/// only pushes when `converter.convert(next)` returns truthy) rather than
/// tracking them. `Unknown` is more useful during porting -- it makes
/// coverage gaps visible instead of silently invisible -- and it's a
/// superset of the JS behavior, not a mismatch: every type the JS baseline
/// *does* implement still round-trips with an identical count (verified
/// against both test fixtures), the only difference is what happens to the
/// ones neither implementation really supports yet.
///
/// # Safety
/// `dwg` must be a successfully-`dwg_read_file`'d, not-yet-`dwg_free`'d
/// `Dwg_Data`.
pub unsafe fn convert_entities(dwg: *mut libredwg_sys::Dwg_Data) -> Vec<RenderEntity> {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut entities = Vec::new();

    for i in 0..num_objects {
        let block_obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if block_obj.is_null() {
            continue;
        }
        // `dwg_object_get_fixedtype`'s real C declaration returns plain
        // `int`, not `DWG_OBJECT_TYPE` -- a pre-existing signature/enum-type
        // mismatch in dwg_api.h itself. bindgen infers DWG_OBJECT_TYPE's own
        // Rust representation from clang's target-dependent choice of
        // underlying integer type for the (otherwise unconstrained) C enum,
        // which isn't guaranteed to match plain `int` on every platform --
        // confirmed to differ in practice: `c_int` (i32) on the MSVC target,
        // `c_uint` (u32) on x86_64-unknown-linux-gnu (found by an actual gcc
        // compile of this crate in a Docker `rust:latest` container, see
        // docs/ARCHITECTURE.md). Every `fixedtype`-typed value in this crate is cast
        // to `DWG_OBJECT_TYPE` immediately at its one FFI call site (here,
        // and again in `convert_one` below and in tables.rs) so downstream
        // code always compares one canonical, self-consistent type
        // regardless of what bindgen inferred on a given target.
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

        // Duplicate each INSERT's attribs as top-level RenderEntity::Attrib
        // entries, matching the JS baseline's db.entities.push(attrib) for
        // every attribute value on an INSERT (converter.ts:151-157) --
        // deliberately *only* here, not inside owned_entities() itself:
        // JS's own per-block entities array (btr.entities, what
        // BLOCK_RECORD.entries[].entities mirrors) does NOT get this
        // duplication, only the flattened top-level db.entities does.
        // Confirmed by diffing block-record entity counts against the JS
        // baseline on example_r14.dwg: *MODEL_SPACE read 68 instead of 61
        // until the duplication was moved out of the shared helper.
        for entity in unsafe { owned_entities(dwg, block_obj) } {
            if let RenderEntity::Insert(insert) = &entity {
                entities.extend(insert.attribs.iter().cloned().map(RenderEntity::Attrib));
            }
            entities.push(entity);
        }
    }

    entities
}

/// Walks every entity directly owned by `block_obj` (a live `BLOCK_HEADER`
/// `Dwg_Object`) and converts each -- no attrib duplication (see
/// `convert_entities`'s call site for why that's applied separately, only
/// for the top-level list). Used both for the model/paper-space entity
/// list and for every block's own entry in
/// [`crate::tables::Tables::block_records`] (all blocks, unfiltered) -- one
/// shared implementation so both stay in sync as entity types get ported.
///
/// # Safety
/// `dwg` must be the live `Dwg_Data` `block_obj` was obtained from;
/// `block_obj` must be a valid, non-null `BLOCK_HEADER` object.
pub(crate) unsafe fn owned_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<RenderEntity> {
    let mut entities = Vec::new();
    let mut owned = unsafe { libredwg_sys::get_first_owned_entity(block_obj) };
    while !owned.is_null() {
        if let Some(entity) = unsafe { convert_one(dwg, owned) } {
            entities.push(entity);
        }
        owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
    }
    entities
}

/// Resolves a POLYLINE_PFACE's mesh into wireframe edges by walking its
/// owned `VERTEX_PFACE` (vertex positions, in order) and `VERTEX_PFACE_FACE`
/// (up to 4 vertex indices per face, 1-based, negative meaning "invisible
/// edge" -- sign carries no other meaning here so it's just abs'd away)
/// subentities directly -- see [`RenderEntity::PolylinePFace`]'s doc comment
/// for why (LibreDWG's own dedicated accessor for this is documented as
/// not implemented). Face records can be interleaved with vertex records
/// in the chain (DWG's own convention lists every vertex first, then every
/// face, but this doesn't assume that) -- indices are only resolved after
/// the whole chain has been walked and every position collected. Faces
/// referencing an out-of-range or all-zero index list are skipped rather
/// than panicking.
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
                if let Some(p) = get_field::<Point3D>(sub_entity_ptr, "VERTEX_PFACE", "point") {
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
            .filter_map(|&i| {
                let a = i.unsigned_abs() as usize;
                (a != 0).then_some(a - 1)
            })
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

/// Resolves a WIPEOUT's clip boundary to local (2D) points -- see
/// [`crate::render_model::WipeoutEntity`]'s doc comment for the risk this carries
/// (an unverified pixel-space-to-world transform).
///
/// `clip_boundary_type` 1 ("rect") stores exactly 2 `clip_verts`, the two
/// opposite corners of an axis-aligned (in pixel space) rectangle;
/// anything else (2, "polygon", or unset) is used as an explicit vertex
/// list directly. If there's no usable `clip_verts` at all, falls back to
/// the full image rectangle implied by `image_size` (0,0 to width,height)
/// -- clipping is optional in the format, but every WIPEOUT still has
/// *some* boundary, namely its full image extent.
///
/// Each pixel-space `(u, v)` point maps to world space as
/// `pt0 + u*uvec + v*vvec` -- `uvec`/`vvec` are already one-pixel-length
/// vectors in the entity's own local space (not normalized directions),
/// per the standard DXF image-entity convention (group 11/12 vs. group 13
/// pixel `image_size`, group 14 clip vertices in that same pixel space).
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
    // BITCODE_BS ("1 rect, 2 polygon" per dwg.h's own comment on the
    // field). Defaults to 0 (neither) if unreadable -- per this function's
    // doc comment, an unset/unknown type is treated the same as "polygon"
    // (explicit vertex list), not assumed to be "rect".
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

/// # Safety
/// `dwg` must be the live `Dwg_Data` `obj` was obtained from; `obj` must be
/// a valid pointer from `dwg_get_object` on that same `Dwg_Data`.
unsafe fn convert_one(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
) -> Option<RenderEntity> {
    // Cast needed for cross-platform bindgen enum-width consistency -- see
    // the comment on the same call in convert_entities() above.
    let fixedtype =
        unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) } as libredwg_sys::DWG_OBJECT_TYPE;

    // Only entities have a meaningful geometry conversion here -- non-entity
    // OBJECT-supertype objects (LAYER, BLOCK_RECORD, DICTIONARY, ...) return
    // null from uncad_object_entity_ptr and are skipped rather than
    // mis-reported as "unknown entities". LAYER/BLOCK_RECORD are still
    // converted, just separately, by tables::convert_tables (its own
    // top-level object walk, not through this function).
    let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(obj) };
    if entity_ptr.is_null() {
        return None;
    }

    // BLOCK/ENDBLK are structural block-boundary sentinels, not user-visible
    // drawing content -- every BLOCK_RECORD (including the implicit
    // *Model_Space/*Paper_Space ones every file has) owns exactly one BLOCK
    // + one ENDBLK. Confirmed during development against the sample_2000.dwg
    // fixture (no longer bundled): the raw object walk
    // found 3 BLOCK + 3 ENDBLK (one pair per block record) alongside the 6
    // real entities the JS `parse()` baseline reported, so the JS converter
    // must exclude these too -- excluded here to match.
    if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ENDBLK
        // SEQEND closes an INSERT's attrib chain (or an old-style
        // POLYLINE's vertex chain) -- structural, like BLOCK/ENDBLK, not
        // user-visible content. Encountered via get_first_owned_subentity
        // when walking an INSERT's attribs below.
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
    let common = EntityCommon {
        handle,
        layer,
        color_index,
        true_color,
    };

    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LINE => {
            let start_point = get_field::<Point3D>(entity_ptr, "LINE", "start")?;
            let end_point = get_field::<Point3D>(entity_ptr, "LINE", "end")?;
            RenderEntity::Line(LineEntity {
                common,
                start_point,
                end_point,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_CIRCLE => {
            let center = get_field::<Point3D>(entity_ptr, "CIRCLE", "center")?;
            let radius = get_field::<f64>(entity_ptr, "CIRCLE", "radius")?;
            RenderEntity::Circle(CircleEntity {
                common,
                center,
                radius,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TEXT => {
            let start_point = get_field::<Point2D>(entity_ptr, "TEXT", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "TEXT", "height")?;
            let text = get_utf8_field(entity_ptr, "TEXT", "text_value").unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0);
            RenderEntity::Text(TextEntity {
                common,
                start_point,
                text_height,
                text,
                rotation,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LWPOLYLINE => {
            let vertices: Vec<Point2D> =
                get_array_field::<u32, _>(entity_ptr, "LWPOLYLINE", "num_points", "points");
            let flag = get_field::<u16>(entity_ptr, "LWPOLYLINE", "flag").unwrap_or(0);
            RenderEntity::LwPolyline(LwPolylineEntity {
                common,
                vertices,
                closed: flag & LWPOLYLINE_CLOSED_FLAG != 0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC => {
            let center = get_field::<Point3D>(entity_ptr, "ARC", "center")?;
            let radius = get_field::<f64>(entity_ptr, "ARC", "radius")?;
            let start_angle = get_field::<f64>(entity_ptr, "ARC", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ARC", "end_angle")?;
            RenderEntity::Arc(ArcEntity {
                common,
                center,
                radius,
                start_angle,
                end_angle,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ELLIPSE => {
            let center = get_field::<Point3D>(entity_ptr, "ELLIPSE", "center")?;
            let major_axis_endpoint = get_field::<Point3D>(entity_ptr, "ELLIPSE", "sm_axis")?;
            let axis_ratio = get_field::<f64>(entity_ptr, "ELLIPSE", "axis_ratio")?;
            let start_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "start_angle")?;
            let end_angle = get_field::<f64>(entity_ptr, "ELLIPSE", "end_angle")?;
            RenderEntity::Ellipse(EllipseEntity {
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
            RenderEntity::Point(PointEntity {
                common,
                position: Point3D { x, y, z },
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SOLID => {
            let corner1 = get_field::<Point2D>(entity_ptr, "SOLID", "corner1")?;
            let corner2 = get_field::<Point2D>(entity_ptr, "SOLID", "corner2")?;
            let corner3 = get_field::<Point2D>(entity_ptr, "SOLID", "corner3")?;
            let corner4 = get_field::<Point2D>(entity_ptr, "SOLID", "corner4")?;
            RenderEntity::Solid(SolidEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_RAY => {
            let point = get_field::<Point3D>(entity_ptr, "RAY", "point")?;
            let vector = get_field::<Point3D>(entity_ptr, "RAY", "vector")?;
            RenderEntity::Ray(RayEntity {
                common,
                point,
                vector,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_XLINE => {
            // Same underlying C struct as RAY (Dwg_Entity_XLINE is a
            // typedef of Dwg_Entity_RAY), but dynapi is keyed by dxfname,
            // so "XLINE" (not "RAY") is required here.
            let point = get_field::<Point3D>(entity_ptr, "XLINE", "point")?;
            let vector = get_field::<Point3D>(entity_ptr, "XLINE", "vector")?;
            RenderEntity::XLine(RayEntity {
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
            RenderEntity::Attrib(AttribEntity {
                common,
                start_point,
                text_height,
                text,
                rotation,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_INSERT => {
            let block_name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "INSERT", "block_header")
                    .and_then(crate::tables::resolve_block_name)
                    .unwrap_or_default();
            let insertion_point = get_field::<Point3D>(entity_ptr, "INSERT", "ins_pt")?;
            let scale =
                get_field::<Point3D>(entity_ptr, "INSERT", "scale").unwrap_or(Point3D {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                });
            let rotation = get_field::<f64>(entity_ptr, "INSERT", "rotation").unwrap_or(0.0);

            // ATTRIBs are owned by the INSERT itself (a separate ownership
            // relationship from BLOCK_HEADER->entity), walked via
            // get_first_owned_subentity on the INSERT's own Dwg_Object --
            // not entity_ptr, which is the type-specific Dwg_Entity_INSERT
            // struct dynapi needs, a different pointer than the owning
            // Dwg_Object subentity iteration expects.
            let mut attribs = Vec::new();
            let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
            while !sub.is_null() {
                if let Some(RenderEntity::Attrib(attrib)) = unsafe { convert_one(dwg, sub) } {
                    attribs.push(attrib);
                }
                sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
            }

            RenderEntity::Insert(InsertEntity {
                common,
                block_name,
                insertion_point,
                scale,
                rotation,
                attribs,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF => {
            let start_point = get_field::<Point2D>(entity_ptr, "ATTDEF", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "ATTDEF", "height")?;
            let default_value =
                get_utf8_field(entity_ptr, "ATTDEF", "default_value").unwrap_or_default();
            let rotation = get_field::<f64>(entity_ptr, "ATTDEF", "rotation").unwrap_or(0.0);
            RenderEntity::Attdef(AttdefEntity {
                common,
                start_point,
                text_height,
                default_value,
                rotation,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VIEWPORT => {
            let center = get_field::<Point3D>(entity_ptr, "VIEWPORT", "center")?;
            let width = get_field::<f64>(entity_ptr, "VIEWPORT", "width")?;
            let height = get_field::<f64>(entity_ptr, "VIEWPORT", "height")?;
            RenderEntity::Viewport(ViewportEntity {
                common,
                center,
                width,
                height,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DFACE => {
            let corner1 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner1")?;
            let corner2 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner2")?;
            let corner3 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner3")?;
            let corner4 = get_field::<Point3D>(entity_ptr, "3DFACE", "corner4")?;
            RenderEntity::Face3D(Face3DEntity {
                common,
                corner1,
                corner2,
                corner3,
                corner4,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SPLINE => {
            // num_fit_pts is BITCODE_BS (u16, unsigned -- libredwg-sys binds
            // BITCODE_BS as u16, not i16; confirmed against the generated
            // bindings while working on HATCH's num_deflines, itself also a
            // BITCODE_BS), unlike most other num_X fields (BITCODE_BL/u32)
            // -- see get_array_field's doc comment.
            let fit_points: Vec<Point3D> =
                get_array_field::<u16, _>(entity_ptr, "SPLINE", "num_fit_pts", "fit_pts");
            let control_points: Vec<Point3D> =
                get_array_field::<u32, SplineControlPoint>(
                    entity_ptr,
                    "SPLINE",
                    "num_ctrl_pts",
                    "ctrl_pts",
                )
                .into_iter()
                .map(Into::into)
                .collect();
            RenderEntity::Spline(SplineEntity {
                common,
                fit_points,
                control_points,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MTEXT => {
            let insertion_point = get_field::<Point3D>(entity_ptr, "MTEXT", "ins_pt")?;
            let text = get_utf8_field(entity_ptr, "MTEXT", "text").unwrap_or_default();
            let text_height = get_field::<f64>(entity_ptr, "MTEXT", "text_height").unwrap_or(1.0);
            // dwg.h's own comment on x_axis_dir says "defines the rotation",
            // and atan2(x_axis_dir.y, x_axis_dir.x) does look like the right
            // derivation -- but the JS baseline's convertMText() never
            // actually implements it (`rotation: 0, // TODO: Didn't find
            // the corresponding field in libredwg`, entityConverter.ts).
            // Confirmed by diffing rendered SVG on example_r14.dwg: an
            // MTEXT with a real x_axis_dir renders `rotate(0 ...)` in the
            // JS output but a nonzero angle here until matched. Same
            // discipline as LWPOLYLINE's closed-bit: match the JS
            // baseline's actual (if incomplete) behavior rather than
            // "improve" on an unverified guess.
            let rotation = 0.0;
            let line_spacing_factor =
                get_field::<f64>(entity_ptr, "MTEXT", "linespace_factor").unwrap_or(1.0);
            RenderEntity::MText(MTextEntity {
                common,
                insertion_point,
                text,
                text_height,
                rotation,
                line_spacing_factor,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_3D => {
            // NOT a generic get_first_owned_subentity walk (tried first --
            // wrong: it over-collected one extra vertex on a real fixture,
            // confirmed by diffing against the JS baseline's rendered SVG).
            // The JS baseline itself calls the real, dedicated LibreDWG C
            // function dwg_object_polyline_3d_get_points() (via its embind
            // wrapper), which -- for pre-R2004 files -- walks
            // first_vertex..last_vertex through the raw object list
            // (dwg_next_object), a different traversal than the generic
            // owned-subentity chain. Calling the exact same C function
            // directly guarantees identical results rather than
            // reimplementing (and risking subtly re-diverging from) its
            // version-dependent logic.
            let mut error = 0i32;
            let points_ptr =
                unsafe { libredwg_sys::dwg_object_polyline_3d_get_points(obj, &mut error) };
            let num_points =
                unsafe { libredwg_sys::dwg_object_polyline_3d_get_numpoints(obj, &mut error) };
            let vertices: Vec<Point3D> = if points_ptr.is_null() || num_points == 0 {
                Vec::new()
            } else {
                // SAFETY: dwg_object_polyline_3d_get_points calloc's exactly
                // num_points dwg_point_3d entries (same layout as Point3D)
                // on success; we copy out before freeing.
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        points_ptr.cast::<Point3D>(),
                        num_points as usize,
                    )
                };
                let v = slice.to_vec();
                unsafe { libc::free(points_ptr.cast()) };
                v
            };
            // POLYLINE_3D.flag is BITCODE_RC (1 byte), unlike LWPOLYLINE's
            // BITCODE_BS (2 bytes) -- same closed-bit convention (bit 1),
            // different underlying C width.
            let flag = get_field::<u8>(entity_ptr, "POLYLINE_3D", "flag").unwrap_or(0);
            RenderEntity::Polyline3D(PolylineEntity {
                common,
                vertices,
                closed: flag & (LWPOLYLINE_CLOSED_FLAG as u8) != 0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER
        // ARC_DIMENSION (dwg.h's 7th DIMENSION subtype) shares the same
        // DIMENSION_COMMON layout -- including the same `block` handle to
        // its cached-geometry anonymous block -- as the other 6, so it
        // needs no new rendering path, just recognizing it here. The JS
        // baseline never recognized it (unlike the other 6 -- see
        // MultiLeaderEntity/MLineEntity for genuinely new functionality;
        // this one only extends an existing DIMENSION path to a type JS
        // happened not to switch on, using the exact mechanism JS itself
        // established for its 6 siblings).
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => {
            let dxfname = dimension_dxfname(fixedtype);
            let block_name = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                entity_ptr,
                dxfname,
                "block",
            )
            .and_then(crate::tables::resolve_block_name)
            .unwrap_or_default();
            RenderEntity::Dimension(DimensionEntity { common, block_name })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TABLE => {
            // ACAD_TABLE: new functionality, not a JS-baseline port -- see
            // AcadTableEntity's doc comment. dynapi's field-table key for
            // this type is "TABLE" (dwg.h/dynapi.c's internal name), not
            // "ACAD_TABLE" (the real DXF/type_name() name, only returned
            // by dwg_object_get_dxfname) -- passing "ACAD_TABLE" here
            // would fail dwg_dynapi_entity_value's strict obj->name check,
            // same class of pitfall as REGION/3DSOLID (see acis.rs).
            let block_name = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                entity_ptr,
                "TABLE",
                "block_header",
            )
            .and_then(crate::tables::resolve_block_name)
            .unwrap_or_default();
            let insertion_point = get_field::<Point3D>(entity_ptr, "TABLE", "ins_pt")?;
            let scale = get_field::<Point3D>(entity_ptr, "TABLE", "scale").unwrap_or(Point3D {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
            let rotation = get_field::<f64>(entity_ptr, "TABLE", "rotation").unwrap_or(0.0);
            RenderEntity::AcadTable(AcadTableEntity {
                common,
                block_name,
                insertion_point,
                scale,
                rotation,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_HATCH => {
            let solid_fill = get_field::<u8>(entity_ptr, "HATCH", "is_solid_fill").unwrap_or(0) != 0;
            let paths: Vec<libredwg_sys::Dwg_HATCH_Path> =
                get_array_field::<u32, _>(entity_ptr, "HATCH", "num_paths", "paths");
            let boundary_paths = paths.iter().map(convert_hatch_path).collect();
            // num_deflines is BITCODE_BS (u16), unlike num_paths' BITCODE_BL
            // (u32) -- see get_array_field's doc comment.
            let deflines: Vec<libredwg_sys::Dwg_HATCH_DefLine> =
                get_array_field::<u16, _>(entity_ptr, "HATCH", "num_deflines", "deflines");
            let pattern_lines = deflines.iter().map(convert_hatch_defline).collect();
            // is_gradient_fill/single_color_gradient are BITCODE_BL (u32),
            // unlike is_solid_fill (BITCODE_B, u8) above.
            let is_gradient_fill =
                get_field::<u32>(entity_ptr, "HATCH", "is_gradient_fill").unwrap_or(0) != 0;
            let gradient = is_gradient_fill.then(|| {
                let gradient_angle =
                    get_field::<f64>(entity_ptr, "HATCH", "gradient_angle").unwrap_or(0.0);
                let single_color_gradient =
                    get_field::<u32>(entity_ptr, "HATCH", "single_color_gradient").unwrap_or(0) != 0;
                let gradient_tint =
                    get_field::<f64>(entity_ptr, "HATCH", "gradient_tint").unwrap_or(0.0);
                let gradient_name =
                    get_utf8_field(entity_ptr, "HATCH", "gradient_name").unwrap_or_default();
                // num_colors is BITCODE_BL (u32), like num_paths.
                let colors: Vec<libredwg_sys::Dwg_HATCH_Color> =
                    get_array_field::<u32, _>(entity_ptr, "HATCH", "num_colors", "colors");
                convert_hatch_gradient(
                    gradient_angle,
                    single_color_gradient,
                    gradient_tint,
                    &gradient_name,
                    &colors,
                )
            }).flatten();
            RenderEntity::Hatch(HatchEntity {
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
            RenderEntity::Solid3D(Solid3DEntity { common, wireframe_edges })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_REGION => {
            // REGION reuses Dwg_Entity__3DSOLID's exact struct layout
            // (dwg.h: `typedef Dwg_Entity__3DSOLID Dwg_Entity_REGION`) and
            // ACIS wireframe extraction, but needs its own real dxfname
            // passed through -- see extract_wireframe's doc comment for
            // why "3DSOLID" can't be hardcoded here. New functionality
            // beyond the JS baseline, which never handled REGION at all;
            // see docs/CAVEATS.md.
            //
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_REGION*
            // (checked above, matching fixedtype), which is
            // layout-identical to Dwg_Entity__3DSOLID* per the typedef
            // above -- the cast extract_wireframe does internally is sound.
            let wireframe_edges = unsafe { crate::acis::extract_wireframe(entity_ptr, "REGION") };
            RenderEntity::Region(Solid3DEntity { common, wireframe_edges })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_PFACE => {
            // SAFETY: obj is a valid, non-null Dwg_Object* for a
            // POLYLINE_PFACE (matching fixedtype); polyline_pface_wireframe
            // only walks its owned-subentity chain (get_first_owned_subentity/
            // get_next_owned_subentity), the same mechanism already used for
            // INSERT's attribs above.
            let wireframe_edges = unsafe { polyline_pface_wireframe(obj) };
            RenderEntity::PolylinePFace(Solid3DEntity { common, wireframe_edges })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TOLERANCE => {
            // New functionality, not a JS-baseline port -- see
            // ToleranceEntity's doc comment.
            let insertion_point = get_field::<Point3D>(entity_ptr, "TOLERANCE", "ins_pt")?;
            let text_height = get_field::<f64>(entity_ptr, "TOLERANCE", "height").unwrap_or(1.0);
            let text_value =
                get_utf8_field(entity_ptr, "TOLERANCE", "text_value").unwrap_or_default();
            RenderEntity::Tolerance(ToleranceEntity {
                common,
                insertion_point,
                text_height,
                text_value,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_WIPEOUT => {
            // New functionality, not a JS-baseline port -- see
            // WipeoutEntity's doc comment for the risk this one carries
            // beyond "unverified against a real file".
            let boundary = wipeout_boundary(entity_ptr);
            RenderEntity::Wipeout(WipeoutEntity { common, boundary })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LIGHT => {
            // New functionality, not a JS-baseline port -- see
            // LightEntity's doc comment.
            let position = get_field::<Point3D>(entity_ptr, "LIGHT", "position")?;
            let target = get_field::<Point3D>(entity_ptr, "LIGHT", "target").unwrap_or(position);
            // type: distant=1, point=2, spot=3 (dwg.h, BITCODE_BL) -- only
            // distant/spot actually aim at `target`.
            let light_type = get_field::<u32>(entity_ptr, "LIGHT", "type").unwrap_or(2);
            let target_is_meaningful = light_type != 2 && target != position;
            RenderEntity::Light(LightEntity {
                common,
                position,
                target,
                target_is_meaningful,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_2D => {
            // Same dedicated-C-function discipline as POLYLINE_3D just
            // above: the JS baseline calls libredwg's own
            // dwg_object_polyline_2d_get_points() (not a generic
            // owned-subentity walk), so this does too rather than risking
            // re-diverging from its version-dependent vertex traversal.
            let mut error = 0i32;
            let points_ptr =
                unsafe { libredwg_sys::dwg_object_polyline_2d_get_points(obj, &mut error) };
            let num_points =
                unsafe { libredwg_sys::dwg_object_polyline_2d_get_numpoints(obj, &mut error) };
            let vertices: Vec<Point2D> = if points_ptr.is_null() || num_points == 0 {
                Vec::new()
            } else {
                // SAFETY: dwg_object_polyline_2d_get_points calloc's exactly
                // num_points dwg_point_2d entries (same layout as Point2D)
                // on success; we copy out before freeing.
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        points_ptr.cast::<Point2D>(),
                        num_points as usize,
                    )
                };
                let v = slice.to_vec();
                unsafe { libc::free(points_ptr.cast()) };
                v
            };
            // POLYLINE_2D.flag is BITCODE_BS, same width and closed-bit
            // convention (bit 1) as LWPOLYLINE's.
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_2D", "flag").unwrap_or(0);
            RenderEntity::Polyline2D(LwPolylineEntity {
                common,
                vertices,
                closed: flag & LWPOLYLINE_CLOSED_FLAG != 0,
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINE => {
            RenderEntity::MLine(convert_mline(dwg, entity_ptr, common))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MULTILEADER => {
            let mut lines_ptr: *mut libredwg_sys::uncad_multileader_line_t = std::ptr::null_mut();
            // SAFETY: entity_ptr is a valid, non-null Dwg_Entity_MULTILEADER*
            // (checked above), matching fixedtype; uncad_multileader_get_lines
            // mallocs *lines_ptr (num_lines entries, each owning its own
            // points buffer) on success, freed below via
            // uncad_multileader_free_lines before returning.
            let num_lines =
                unsafe { libredwg_sys::uncad_multileader_get_lines(entity_ptr, &mut lines_ptr) };
            let mut lines = Vec::with_capacity(num_lines as usize);
            if !lines_ptr.is_null() {
                let raw_lines =
                    unsafe { std::slice::from_raw_parts(lines_ptr, num_lines as usize) };
                for l in raw_lines {
                    if l.points.is_null() || l.num_points == 0 {
                        continue;
                    }
                    // SAFETY: points is a flat (x,y,z) triple array of
                    // num_points*3 doubles, per uncad_multileader_get_lines'
                    // contract.
                    let flat = unsafe {
                        std::slice::from_raw_parts(l.points, l.num_points as usize * 3)
                    };
                    let pts = flat
                        .chunks_exact(3)
                        .map(|c| Point3D { x: c[0], y: c[1], z: c[2] })
                        .collect();
                    lines.push(pts);
                }
                unsafe { libredwg_sys::uncad_multileader_free_lines(lines_ptr, num_lines) };
            }
            RenderEntity::MultiLeader(MultiLeaderEntity { common, lines })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LEADER => {
            let vertices: Vec<Point3D> =
                get_array_field::<u32, _>(entity_ptr, "LEADER", "num_points", "points");
            // arrowhead_type is BITCODE_BS (u16, unsigned; not a bit flag)
            // -- JS baseline treats any nonzero value as "enabled",
            // matching its own `isArrowheadEnabled > 0` check.
            let is_arrowhead_enabled =
                get_field::<u16>(entity_ptr, "LEADER", "arrowhead_type").unwrap_or(0) > 0;
            RenderEntity::Leader(LeaderEntity {
                common,
                vertices,
                is_arrowhead_enabled,
            })
        }
        // SAFETY: obj is valid per this function's own `# Safety` doc contract.
        _ => RenderEntity::Unknown {
            common,
            type_name: unsafe { dxfname(obj) },
        },
    })
}

/// Reads a raw `(ptr, count)` pair from a *nested* struct field (not
/// reachable through dynapi, which only exposes top-level entity fields by
/// name) as an owned `Vec<T>`.
///
/// # Safety
/// `ptr` must be valid for reads of `count` consecutive `T`s (or null,
/// treated as empty), matching LibreDWG's own num_X/X array convention.
unsafe fn read_raw_array<T: Copy>(ptr: *const T, count: u32) -> Vec<T> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, count as usize) }.to_vec()
}

/// HATCH.paths[i].flag bit for "this path is a polyline" (vs. a list of
/// curved/straight edges) -- see dwg.h's field comment on
/// Dwg_HATCH_Path::flag.
const HATCH_PATH_IS_POLYLINE_FLAG: u32 = 0x02;

fn convert_hatch_path(path: &libredwg_sys::Dwg_HATCH_Path) -> HatchBoundaryPath {
    if path.flag & HATCH_PATH_IS_POLYLINE_FLAG != 0 {
        // SAFETY: polyline_paths/num_segs_or_paths are LibreDWG's own
        // matched array-length convention (see Dwg_HATCH_Path in dwg.h);
        // valid until dwg_free, which outlives this whole conversion pass.
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

/// HATCH boundary edge curve types (Dwg_HATCH_PathSeg::curve_type in dwg.h).
const HATCH_EDGE_LINE: u8 = 1;
const HATCH_EDGE_ARC: u8 = 2;
const HATCH_EDGE_ELLIPSE: u8 = 3;
const HATCH_EDGE_SPLINE: u8 = 4;

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
            // SAFETY: num_control_points/control_points is the same
            // matched array-length convention as everywhere else in
            // dwg.h; valid until dwg_free.
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

/// `point + miter_direction * offset` gives a vertex's position on the
/// parallel line `offset` away from an MLINE's centerline -- `miter_direction`
/// (per LibreDWG's own field, not derived here) already accounts for the
/// miter angle at that vertex, so this is a plain scalar-multiply-and-add,
/// no trigonometry needed. `mlinestyle_name` is resolved here (at convert
/// time, like `EntityCommon::layer`) but not looked up against
/// `Tables::mlinestyles` until render time in `svg.rs` -- `Tables` isn't
/// built yet when entities are converted (see `lib.rs::parse`'s ordering),
/// the same reason BYLAYER color resolution is deferred.
fn convert_mline(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut std::ffi::c_void,
    common: EntityCommon,
) -> MLineEntity {
    // num_verts is BITCODE_BS (u16, unsigned) -- see the SPLINE
    // num_fit_pts comment above for how this class of mistake was
    // found (libredwg-sys binds BITCODE_BS as u16, not i16).
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
    // MLINE_FLAGS_CLOSED (dwg.h) = 2.
    let flags = get_field::<u16>(entity_ptr, "MLINE", "flags").unwrap_or(0);
    let mlinestyle_name =
        get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "MLINE", "mlinestyle")
            .and_then(|handle_ptr| resolve_handle_name(dwg, handle_ptr))
            .unwrap_or_default();
    MLineEntity {
        common,
        vertices,
        closed: flags & 2 != 0,
        mlinestyle_name,
    }
}

fn convert_hatch_defline(defline: &libredwg_sys::Dwg_HATCH_DefLine) -> HatchPatternLine {
    // SAFETY: num_dashes/dashes is the same matched array-length convention
    // as everywhere else in dwg.h; valid until dwg_free. num_dashes is
    // BITCODE_BS, which libredwg-sys binds as u16 (unsigned) -- a plain
    // widening cast is enough, no negative-count wraparound to guard
    // against.
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

/// Resolves one `Dwg_HATCH_Color` gradient stop's `Dwg_Color` to a hex
/// string, using the same truecolor-overrides-ACI precedence as
/// [`entity_color`]/[`crate::color::resolve_color`] -- but unlike an
/// entity's own color, a gradient stop is never BYLAYER/BYBLOCK (256/0
/// don't apply here), so this skips straight to `aci_to_hex` for the
/// non-truecolor case instead of needing layer/inherited-color context.
fn hatch_stop_color(c: &libredwg_sys::Dwg_HATCH_Color) -> String {
    let true_color = (c.color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR)
        .then_some(c.color.rgb & 0xff_ffff);
    crate::color::true_color_to_hex(true_color)
        .or_else(|| crate::color::aci_to_hex(c.color.index.unsigned_abs()))
        .unwrap_or_else(|| crate::color::DEFAULT_COLOR.to_string())
}

/// Builds a [`HatchGradient`] from the raw `Dwg_Entity_HATCH` gradient
/// fields. Returns `None` when there's no usable color data (falls back to
/// `pattern_lines`/outline-only rendering at the call site) -- see
/// [`HatchGradient`]'s doc comment for the unverified `gradient_name`
/// radial/linear split and the `single_color_gradient` tint approximation.
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
        // shift_value (0.0-1.0) orders the stops; colors isn't guaranteed
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

/// The 7 DIMENSION subtypes share `DIMENSION_COMMON`'s fields (including
/// `block`, the cached-geometry handle), but dynapi is keyed by each
/// subtype's own dxfname -- there's no generic `"DIMENSION"` to pass.
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
/// `(color_index, true_color)` per [`EntityCommon`]'s doc comment.
fn entity_color(entity_ptr: *mut std::ffi::c_void) -> (i16, Option<u32>) {
    let Some(color) = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color") else {
        return (256, None); // no color field at all -- BYLAYER default, matches toSVG's `?? 256`.
    };
    let true_color = (color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR)
        .then_some(color.rgb & 0xff_ffff);
    (color.index, true_color)
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
        // colors[0] is the *second* stop (shift_value 1.0); the array isn't
        // guaranteed to already be sorted.
        let colors = [aci_stop(1.0, 5), aci_stop(0.0, 1)];
        let g = convert_hatch_gradient(0.0, false, 0.0, "LINEAR", &colors).unwrap();
        assert_eq!(g.color1, "#ff0000"); // ACI 1, shift 0.0
        assert_eq!(g.color2, "#0000ff"); // ACI 5, shift 1.0
        assert!(!g.is_radial);
    }

    #[test]
    fn gradient_single_color_tints_toward_white_for_second_stop() {
        let colors = [aci_stop(0.0, 7)]; // ACI 7 -> normalized to black (#000000)
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
