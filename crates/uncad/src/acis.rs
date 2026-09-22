//! Minimal ACIS SAT (v1, ASCII) reader scoped to wireframe extraction only --
//! NOT a general ACIS/B-rep parser. Written from scratch against publicly
//! available format documentation (Spatial's own "SAT Save File Format"
//! chapter), used only as a reference for record and field *semantics*; no code
//! or text from that document is reproduced here. See `docs/ARCHITECTURE.md`,
//! "3DSOLID / REGION ACIS wireframe", for the full rationale and scope.
//!
//! For each `edge` record reachable from a solid's body, the two endpoint
//! vertices are resolved through `point` records and emitted as one straight
//! segment. Curved edges become chords, not true arcs. Faces and surfaces are
//! not interpreted at all: the result is always a wireframe, never a filled or
//! shaded shape.

use crate::dynapi::get_field;
use std::ffi::{c_void, CStr};
use uncad_model::model::Point3D;

struct SatRecord {
    type_name: String,
    tokens: Vec<String>,
}

/// Splits raw ACIS SAT v1 text (from LibreDWG's SAB-to-SAT conversion, or
/// already-SAT `acis_data`) into records, indexed exactly as the file's own
/// `$N` pointers refer to them: 0-based, in file order, with the header lines
/// and the `End-of-ACIS-data` marker excluded.
// The fixed 3-line header skip is not verified against every ACIS SAT variant.
// On an old R14-era body it did parse every record including 18 real `edge`
// ones, yet none of their vertices resolved -- whether that is this assumption
// being wrong for that ACIS version or some other cause was never established.
// See docs/CAVEATS.md.
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

/// `true` when every `$N` pointer in the records the wireframe walk follows
/// (`edge` and `vertex`) addresses an existing record. Other record types are
/// not checked: `eye_refinement` carries `$`-prefixed values that are not
/// pointers at all.
fn pointers_are_in_range(records: &[SatRecord]) -> bool {
    records
        .iter()
        .filter(|r| r.type_name == "edge" || r.type_name == "vertex")
        .flat_map(|r| r.tokens.iter())
        .filter_map(|t| t.strip_prefix('$')?.parse::<usize>().ok())
        .all(|idx| idx < records.len())
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

/// Walks every `edge` record and returns one chord segment per edge, plus
/// the number of edges that were skipped because their two endpoint
/// vertices could not both be resolved (unexpected record shape, e.g. a
/// future/older ACIS version this wasn't written against). The count is
/// what lets a caller tell "this solid has no edges" from "this solid's
/// edges could not be read".
fn extract_wireframe_segments(records: &[SatRecord]) -> (Vec<[Point3D; 2]>, usize) {
    let mut segments = Vec::new();
    let mut skipped = 0usize;

    // Records are addressed by position, so the text is only readable when
    // every pointer the walk would follow lands inside it. Measured on a real
    // R2007 drawing: 62 of 116 solids came back from the SAB-to-SAT conversion
    // with pointers up to 166 records past the end -- the converter dropped
    // records it did not handle without renumbering the rest. Following those
    // pointers would attach edges to whatever record happens to sit at that
    // index, so such a solid is reported as entirely unread (every edge
    // counted as skipped) rather than partly, and wrongly, drawn.
    if !pointers_are_in_range(records) {
        let edges = records.iter().filter(|r| r.type_name == "edge").count();
        return (Vec::new(), edges);
    }
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
        } else {
            skipped += 1;
        }
    }
    (segments, skipped)
}

/// Reads the SAT (v1, ASCII) text for a 3DSOLID/REGION/BODY entity, converting
/// from SAB (v2, binary) first when that is how this entity stored its ACIS
/// data -- always on a copy, through `libredwg-sys`'s
/// `uncad_3dsolid_sab_to_sat_text` shim, never in place. `None` if the solid is
/// empty or its data cannot be read or converted.
///
/// `dxfname` must be the entity's own real name (`"3DSOLID"`, `"REGION"`, ...).
/// dynapi checks it against the object's actual `obj->name` and refuses to read
/// any field on a mismatch, even though REGION and 3DSOLID share one C struct
/// and one dynapi field table -- hardcoding `"3DSOLID"` would silently return
/// `None` for every field of a real REGION.
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

    // SAB (v2, binary), converted to SAT text on a *copy* of the entity.
    // Calling LibreDWG's dwg_convert_SAB_to_SAT1 on the live entity, which is
    // what this used to do, converts in place (version 2 -> 1, plaintext SAT
    // into encr_sat_data, acis_data left as SAB bytes). parse() reads every
    // solid twice -- convert_entities for model space, then convert_tables for
    // the owning block record -- so the second read took the `version != 2`
    // branch above, parsed binary SAB as text, and produced no wireframe. See
    // shim/uncad_shim.h and docs/CAVEATS.md, "3DSOLID SAB conversion".
    let mut len: usize = 0;
    // SAFETY: entity_ptr is a valid Dwg_Entity__3DSOLID* per this function's
    // contract; the shim only reads through it (and through its `parent`
    // back-pointer, to reach the drawing's header version) and writes to
    // its own stack copy.
    let text_ptr = unsafe { libredwg_sys::uncad_3dsolid_sab_to_sat_text(entity_ptr, &mut len) };
    if text_ptr.is_null() {
        return None;
    }
    // SAFETY: the shim returned a malloc'd buffer valid for `len` bytes (plus
    // a trailing NUL), owned by this function until uncad_free_sat_text below.
    let bytes = unsafe { std::slice::from_raw_parts(text_ptr.cast::<u8>(), len) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    // SAFETY: text_ptr came from uncad_3dsolid_sab_to_sat_text and is freed
    // exactly once, here, after the last read of it above.
    unsafe { libredwg_sys::uncad_free_sat_text(text_ptr) };
    if text.is_empty() {
        return None;
    }
    Some(text)
}

/// Best-effort wireframe extraction for one 3DSOLID/REGION/BODY entity: reads
/// its ACIS data, converting from SAB to SAT if needed, parses the records, and
/// returns one chord segment per ACIS `edge`. Empty when the solid is empty,
/// its data cannot be read, or it has no edges -- the caller then treats the
/// entity as unsupported.
///
/// `dxfname` is the entity's own real name; see `read_sat_text_from_entity` for
/// why it cannot be hardcoded.
///
/// # Safety
/// `entity_ptr` must be a valid, non-null `Dwg_Entity__3DSOLID*`.
pub unsafe fn extract_wireframe(
    entity_ptr: *mut c_void,
    dxfname: &str,
) -> (Vec<[Point3D; 2]>, usize) {
    let Some(sat_text) = (unsafe { read_sat_text_from_entity(entity_ptr, dxfname) }) else {
        return (Vec::new(), 0);
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
        let (segments, skipped) = extract_wireframe_segments(&records);
        assert_eq!(skipped, 0);
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
        let (segments, skipped) = extract_wireframe_segments(&records);
        assert_eq!(segments.len(), 0);
        assert_eq!(
            skipped, 1,
            "the unresolvable edge must be counted, not just dropped"
        );
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

    /// A text whose edge points past the last record: nothing is drawn and
    /// every edge is counted, even the one that would have resolved.
    #[test]
    fn out_of_range_pointers_make_the_whole_solid_unread_not_partly_drawn() {
        let sat = "700 0 1 0
            9 SomeProduct 9 SomeVersion 24 Mon Jan 01 00:00:00 2024
            1e-06 1e-10
            point 0.0 0.0 0.0 #
            point 1.0 2.0 3.0 #
            vertex $0 #
            vertex $1 #
            edge $2 $3 #
            edge $2 $99 #
            End-of-ACIS-data
";
        let records = parse_sat_records(sat);
        assert!(!pointers_are_in_range(&records));
        let (segments, skipped) = extract_wireframe_segments(&records);
        assert!(
            segments.is_empty(),
            "no chord may be drawn from an inconsistent text"
        );
        assert_eq!(
            skipped, 2,
            "both edges count as unread, including the resolvable one"
        );
    }

    /// The guard covers the vertex -> point hop too: a vertex whose point
    /// pointer runs past the records makes the whole solid unread, rather
    /// than each edge failing to find a point one by one.
    #[test]
    fn a_vertex_pointing_past_the_records_makes_the_whole_solid_unread() {
        let sat = "700 0 1 0
            9 SomeProduct 9 SomeVersion 24 Mon Jan 01 00:00:00 2024
            1e-06 1e-10
            point 0.0 0.0 0.0 #
            vertex $0 #
            vertex $77 #
            edge $1 $2 #
            End-of-ACIS-data
";
        let records = parse_sat_records(sat);
        assert!(!pointers_are_in_range(&records));
        let (segments, skipped) = extract_wireframe_segments(&records);
        assert!(segments.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn a_dollar_value_inside_eye_refinement_is_not_a_pointer() {
        let sat = "700 0 1 0
            9 SomeProduct 9 SomeVersion 24 Mon Jan 01 00:00:00 2024
            1e-06 1e-10
            eye_refinement $-1 $-1 5 mgrid $3000 #
            point 0.0 0.0 0.0 #
            point 1.0 2.0 3.0 #
            vertex $1 #
            vertex $2 #
            edge $3 $4 #
            End-of-ACIS-data
";
        let records = parse_sat_records(sat);
        assert!(pointers_are_in_range(&records));
        let (segments, skipped) = extract_wireframe_segments(&records);
        assert_eq!((segments.len(), skipped), (1, 0));
    }
}
