//! Minimal, independent ACIS SAT (v1, ASCII) reader scoped to wireframe
//! extraction only -- NOT a general ACIS/B-rep parser. Written from scratch
//! against publicly available format documentation (Spatial's own "SAT Save
//! File Format" chapter, long-mirrored at e.g. paulbourke.net/dataformats/sat)
//! used only as a reference for record/field *semantics*; no code or text
//! from that document is reproduced here. See docs/ARCHITECTURE.md's
//! "3DSOLID / ACIS 와이어프레임" section for the full rationale, scope, and
//! legal considerations. Ported line-for-line from the project's own former
//! JS implementation (`src/acisWireframe.mjs`, see git history) rather than
//! redesigned.
//!
//! Scope: for each `edge` record reachable from a 3DSOLID's body, resolve
//! its two endpoint vertices (via `point` records) and emit a straight line
//! segment between them. Curved edges (arcs on ellipse-curve/intcurve-curve)
//! are rendered as chords, not true arcs -- a deliberate simplification.
//! Faces/surfaces (plane-surface, cone-surface, sphere-surface,
//! spline-surface, ...) are not interpreted at all; this only produces a
//! wireframe, never a filled/shaded shape.

use crate::dynapi::{get_field, Point3D};
use std::ffi::{c_void, CStr};

struct SatRecord {
    type_name: String,
    tokens: Vec<String>,
}

/// Splits raw ACIS SAT v1 text (as produced by `dwg_convert_SAB_to_SAT1`, or
/// already-SAT `acis_data`) into records, indexed exactly as `$N` pointers
/// within the file refer to them (0-based, in file order, header lines and
/// the `End-of-ACIS-data` marker excluded).
// Unverified against every ACIS SAT header variant -- ported as-is from
// the JS baseline's own equally-unverified assumption. On the
// `example_r14.dwg` fixture used during development (an old R14-era
// ACIS v106/SAT-v1 body, no longer bundled with the repo), this 3-line
// skip did parse 131 records including 18 real
// `edge` records, but every one failed vertex/point resolution and
// `extract_wireframe_segments` returned empty -- matching the JS
// baseline's own real (WASM-executed) output on that exact file, which
// reported 3DSOLID as unsupported too. Not independently confirmed
// whether that's this header-skip assumption being wrong for this
// particular (old) ACIS version, or some other cause -- but since it
// reproduced JS's own observed behavior byte-for-byte on the one real
// file available to verify against at the time, "fixing" it without
// independent ground truth (a real AutoCAD render) would risk diverging
// from the verified reference on an unverified guess, the same discipline
// applied elsewhere in this codebase (see convert.rs's LWPOLYLINE
// closed-bit comment).
fn parse_sat_records(sat_text: &str) -> Vec<SatRecord> {
    // The first 3 lines are the ACIS header (version, product/version/date
    // string, tolerances); entity records start on line 3 (0-based).
    let lines: Vec<&str> = sat_text.split('\n').collect();
    let body = if lines.len() > 3 {
        lines[3..].join("\n")
    } else {
        String::new()
    };
    let records_text = match body.find("End-of-ACIS-data") {
        Some(idx) => &body[..idx],
        None => body.as_str(),
    };

    let mut records = Vec::new();
    for chunk in records_text.split('#') {
        let trimmed = chunk.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(type_name) = parts.next() else {
            continue;
        };
        records.push(SatRecord {
            type_name: type_name.to_string(),
            tokens: parts.map(|s| s.to_string()).collect(),
        });
    }
    records
}

fn resolve_pointer<'a>(records: &'a [SatRecord], token: &str) -> Option<(usize, &'a SatRecord)> {
    let rest = token.strip_prefix('$')?;
    let idx: usize = rest.parse().ok()?;
    records.get(idx).map(|r| (idx, r))
}

fn point_xyz(point_record: &SatRecord) -> Option<[f64; 3]> {
    let nums: Vec<f64> = point_record
        .tokens
        .iter()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    if nums.len() < 3 {
        return None;
    }
    Some([nums[0], nums[1], nums[2]])
}

/// Walks every `edge` record and returns one chord segment per edge.
/// Edges whose two endpoint vertices can't both be resolved (unexpected
/// record shape, e.g. a future/older ACIS version this wasn't written
/// against) are silently skipped.
fn extract_wireframe_segments(records: &[SatRecord]) -> Vec<[Point3D; 2]> {
    let mut segments = Vec::new();
    for record in records {
        if record.type_name != "edge" {
            continue;
        }
        let mut vertex_points = Vec::new();
        for token in &record.tokens {
            let Some((_, resolved)) = resolve_pointer(records, token) else {
                continue;
            };
            if resolved.type_name != "vertex" {
                continue;
            }
            let point = resolved
                .tokens
                .iter()
                .filter_map(|t| resolve_pointer(records, t))
                .find(|(_, r)| r.type_name == "point");
            if let Some((_, point_record)) = point {
                if let Some([x, y, z]) = point_xyz(point_record) {
                    vertex_points.push(Point3D { x, y, z });
                }
            }
        }
        if vertex_points.len() == 2 {
            segments.push([vertex_points[0], vertex_points[1]]);
        }
    }
    segments
}

/// Reads the SAT (v1, ASCII) text for a 3DSOLID/REGION/BODY entity,
/// converting from SAB (v2, binary) first via `dwg_convert_SAB_to_SAT1` if
/// that's how this particular entity stored its ACIS data. Returns `None`
/// if the solid is empty or its ACIS data can't be read/converted.
///
/// `dxfname` must be the entity's own real dxfname (`"3DSOLID"`,
/// `"REGION"`, ...) -- dynapi enforces this exactly (`dwg_dynapi_entity_value`
/// checks the passed name against the object's actual `obj->name` and
/// refuses to read any field at all on a mismatch), even though REGION and
/// 3DSOLID share the identical underlying C struct
/// (`typedef Dwg_Entity__3DSOLID Dwg_Entity_REGION` in dwg.h, and
/// `dynapi.c`'s `"REGION"` entry literally points at `_dwg_3DSOLID_fields`,
/// the same field table `"3DSOLID"` uses) -- hardcoding `"3DSOLID"` here
/// would silently return `None` for every field on a real REGION entity.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity__3DSOLID*` (as
/// returned by `uncad_object_entity_ptr` for a 3DSOLID/REGION/BODY object),
/// valid for the duration of this call.
unsafe fn read_sat_text_from_entity(entity_ptr: *mut c_void, dxfname: &str) -> Option<String> {
    let version = get_field::<u16>(entity_ptr, dxfname, "version")?;
    let acis_empty = get_field::<u8>(entity_ptr, dxfname, "acis_empty").unwrap_or(0);
    if acis_empty != 0 {
        return None;
    }

    if version != 2 {
        // Already SAT v1 (older AutoCAD releases write ACIS as text directly).
        let size = get_field::<u32>(entity_ptr, dxfname, "sab_size").unwrap_or(0);
        let ptr = get_field::<*const u8>(entity_ptr, dxfname, "acis_data")?;
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid pointer to at least `size` bytes (or, if
        // size is unset/0, a NUL-terminated buffer) per LibreDWG's own
        // acis_data contract; valid until dwg_free, which outlives this
        // whole conversion pass.
        let bytes: Vec<u8> = unsafe {
            if size > 0 {
                std::slice::from_raw_parts(ptr, size as usize).to_vec()
            } else {
                CStr::from_ptr(ptr.cast()).to_bytes().to_vec()
            }
        };
        return Some(String::from_utf8_lossy(&bytes).into_owned());
    }

    // SAFETY: entity_ptr is a valid Dwg_Entity__3DSOLID* per this function's
    // contract; dwg_convert_SAB_to_SAT1 mutates the entity in place to
    // populate num_blocks/block_size/encr_sat_data from the raw SAB stream.
    let ret = unsafe {
        libredwg_sys::dwg_convert_SAB_to_SAT1(
            entity_ptr.cast::<libredwg_sys::Dwg_Entity__3DSOLID>(),
        )
    };
    if ret != 0 {
        return None;
    }
    let num_blocks = get_field::<u32>(entity_ptr, dxfname, "num_blocks").unwrap_or(0);
    if num_blocks == 0 {
        return None;
    }
    let block_size_ptr = get_field::<*const u32>(entity_ptr, dxfname, "block_size")?;
    let block_ptrs_ptr =
        get_field::<*const *const std::os::raw::c_char>(entity_ptr, dxfname, "encr_sat_data")?;
    if block_size_ptr.is_null() || block_ptrs_ptr.is_null() {
        return None;
    }

    let mut text = String::new();
    for i in 0..num_blocks as usize {
        // SAFETY: block_size_ptr/block_ptrs_ptr are both num_blocks-long
        // arrays per LibreDWG's own convention (populated together by the
        // dwg_convert_SAB_to_SAT1 call above); valid until dwg_free.
        let (block_size, str_ptr) = unsafe { (*block_size_ptr.add(i), *block_ptrs_ptr.add(i)) };
        if str_ptr.is_null() || block_size == 0 {
            continue;
        }
        // SAFETY: str_ptr is valid for block_size bytes, matching the same
        // encr_sat_data/block_size convention.
        let bytes =
            unsafe { std::slice::from_raw_parts(str_ptr.cast::<u8>(), block_size as usize) };
        text.push_str(&String::from_utf8_lossy(bytes));
    }
    Some(text)
}

/// Best-effort wireframe extraction for one 3DSOLID/REGION/BODY entity:
/// reads its ACIS data, converting from SAB to SAT if needed, parses the
/// SAT records, and returns one chord segment per ACIS `edge`. Returns an
/// empty `Vec` if the solid is empty, its ACIS data can't be read, or it
/// has no edges (in which case the caller should treat this entity as
/// unsupported, matching the JS baseline's `wireframeEdges?.length > 0`
/// check at render time -- for REGION this isn't actually a JS behavior to
/// match, since the JS baseline never handled REGION at all; see
/// docs/CAVEATS.md).
///
/// `dxfname` is the entity's own real dxfname -- see
/// `read_sat_text_from_entity`'s doc comment for why this can't be
/// hardcoded even though REGION/3DSOLID/BODY are structurally identical.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity__3DSOLID*`.
pub unsafe fn extract_wireframe(entity_ptr: *mut c_void, dxfname: &str) -> Vec<[Point3D; 2]> {
    let Some(sat_text) = (unsafe { read_sat_text_from_entity(entity_ptr, dxfname) }) else {
        return Vec::new();
    };
    let records = parse_sat_records(&sat_text);
    extract_wireframe_segments(&records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic SAT text: 3 header lines (skipped), 2 `point` records, 2
    /// `vertex` records each pointing at one of them, 1 `edge` record
    /// pointing at both vertices, then the `End-of-ACIS-data` marker (with
    /// trailing content after it that must be ignored). Record indices are
    /// 0-based in file order: 0=point, 1=point, 2=vertex, 3=vertex, 4=edge.
    const SAT_TEXT: &str = "700 0 1 0\n\
        9 SomeProduct 9 SomeVersion 24 Mon Jan 01 00:00:00 2024\n\
        1e-06 1e-10\n\
        point 0.0 0.0 0.0 #\n\
        point 1.0 2.0 3.0 #\n\
        vertex $0 #\n\
        vertex $1 #\n\
        edge $2 $3 #\n\
        End-of-ACIS-data\n\
        garbage that should never be parsed as a record #";

    #[test]
    fn parses_expected_record_count_and_types() {
        let records = parse_sat_records(SAT_TEXT);
        assert_eq!(records.len(), 5);
        assert_eq!(records[0].type_name, "point");
        assert_eq!(records[2].type_name, "vertex");
        assert_eq!(records[4].type_name, "edge");
    }

    #[test]
    fn stops_at_end_of_acis_data_marker() {
        let records = parse_sat_records(SAT_TEXT);
        assert!(records.iter().all(|r| r.type_name != "garbage"));
    }

    #[test]
    fn resolves_edge_to_its_two_endpoint_coordinates() {
        let records = parse_sat_records(SAT_TEXT);
        let segments = extract_wireframe_segments(&records);
        assert_eq!(segments.len(), 1);
        assert_eq!(
            segments[0][0],
            Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0
            }
        );
        assert_eq!(
            segments[0][1],
            Point3D {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
    }

    #[test]
    fn edge_with_unresolvable_vertex_is_skipped_not_panicking() {
        // References a vertex index ($9) that doesn't exist.
        let records = parse_sat_records(
            "h\nh\nh\npoint 0.0 0.0 0.0 #\nvertex $0 #\nedge $1 $9 #\nEnd-of-ACIS-data",
        );
        assert_eq!(extract_wireframe_segments(&records).len(), 0);
    }

    #[test]
    fn resolve_pointer_rejects_non_dollar_and_out_of_range_tokens() {
        let records = parse_sat_records(SAT_TEXT);
        assert!(resolve_pointer(&records, "not-a-pointer").is_none());
        assert!(resolve_pointer(&records, "$999").is_none());
        assert!(resolve_pointer(&records, "$0").is_some());
    }

    #[test]
    fn point_xyz_requires_at_least_three_numeric_tokens() {
        let records = parse_sat_records(SAT_TEXT);
        assert_eq!(point_xyz(&records[0]), Some([0.0, 0.0, 0.0]));
        let too_few = SatRecord {
            type_name: "point".to_string(),
            tokens: vec!["1.0".to_string(), "2.0".to_string()],
        };
        assert_eq!(point_xyz(&too_few), None);
    }
}
