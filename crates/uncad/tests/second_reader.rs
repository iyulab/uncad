//! A second, independent DWG reader as an oracle.
//!
//! This crate reads DWG through one engine. A defect in that engine cannot
//! be seen from inside it: the reader and the expectation share the mistake.
//! These tests read the same corpus files through an unrelated Rust
//! implementation and compare what the two agree on.
//!
//! The comparison runs from coarse to fine: which drawings both open, which
//! entity types each finds, and -- for leaders, lines, circles, arcs, texts,
//! multi-line texts, block references and dimensions -- every field both
//! models carry, entity by entity.
//!
//! The second reader is a development dependency: nothing a consumer builds
//! reaches it.

use std::path::{Path, PathBuf};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

/// The versions this crate's corpus holds a full set of drawings for, newest
/// first. The second reader states R13..R2018+ support, so a version outside
/// that range is not a disagreement -- it is out of scope, and named here so
/// the distinction stays visible rather than silently untested.
const VERSIONS: &[&str] = &["2018", "2013", "2010", "2007", "2004", "2000", "r14"];

fn drawings_for(version: &str) -> Vec<PathBuf> {
    let dir = Path::new(CORPUS).join(version);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
        })
        .collect();
    out.sort();
    out
}

fn second_reader_entity_count(path: &Path) -> Option<usize> {
    let mut reader = acadrust::DwgReader::from_file(path).ok()?;
    let document = reader.read().ok()?;
    Some(document.entities().count())
}

/// Reading the same file twice through the second reader must give the same
/// answer. This is the property that decides whether it can sit behind a
/// deterministic surface at all: its dependency graph carries a randomly
/// seeded hasher and a work-stealing thread pool, either of which can leak
/// ordering into an output.
#[test]
fn the_second_reader_gives_the_same_count_for_the_same_file() {
    let mut compared = 0usize;
    let mut unstable: Vec<String> = Vec::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Some(first) = second_reader_entity_count(&path) else {
                continue;
            };
            compared += 1;
            for _ in 0..3 {
                let again = second_reader_entity_count(&path);
                if again != Some(first) {
                    unstable.push(format!("{}: {first} then {again:?}", path.display()));
                    break;
                }
            }
        }
    }
    assert!(compared > 0, "the second reader opened no corpus drawing");
    assert!(
        unstable.is_empty(),
        "{} of {compared} drawings read differently on a repeat:\n{}",
        unstable.len(),
        unstable.join("\n")
    );
}

/// How far the two readers agree, per version, pinned as a measurement.
///
/// These are not requirements. They are what was observed, recorded so that
/// a change in either reader shows up as a diff instead of passing unseen.
/// A count of drawings the second reader *opens*, out of those this crate
/// opens -- the coarsest agreement there is, and the one every finer
/// comparison depends on.
#[test]
fn how_many_drawings_both_readers_open_is_what_it_was_when_last_measured() {
    let mut report = String::new();
    for version in VERSIONS {
        let drawings = drawings_for(version);
        if drawings.is_empty() {
            continue;
        }
        let mut ours = 0usize;
        let mut theirs = 0usize;
        for path in &drawings {
            if uncad::parse(path).is_ok() {
                ours += 1;
            }
            if second_reader_entity_count(path).is_some() {
                theirs += 1;
            }
        }
        report.push_str(&format!(
            "{version}: {ours} / {theirs} of {}\n",
            drawings.len()
        ));
    }
    // Written by the first run of this test; a diff here is the signal.
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-agreement.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "\nthe two readers' agreement moved; measured now:\n{report}"
    );
}

/// Whether the second reader can say "this reference points at nothing".
///
/// This crate's model tells three states apart -- a name that resolved, a
/// reference the file makes that nothing answers to, and no reference at
/// all -- because a consumer that cannot tell them apart will read a made-up
/// name as a real one. The question here is whether a second reader's own
/// model carries enough for those three states to be recovered from it.
///
/// The probe is a file naming a block it does not define, which is the same
/// case this crate's own reference tests use.
#[test]
fn what_the_second_reader_says_about_a_reference_to_a_missing_block() {
    let pairs: &[(u16, &str)] = &[
        (0, "SECTION"),
        (2, "BLOCKS"),
        (0, "BLOCK"),
        (8, "0"),
        (2, "REAL"),
        (70, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (3, "REAL"),
        (0, "LINE"),
        (8, "0"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (11, "1.0"),
        (21, "0.0"),
        (31, "0.0"),
        (0, "ENDBLK"),
        (8, "0"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "INSERT"),
        (8, "0"),
        (2, "REAL"),
        (10, "0.0"),
        (20, "0.0"),
        (30, "0.0"),
        (0, "INSERT"),
        (8, "0"),
        (2, "NOBLOCK"),
        (10, "5.0"),
        (20, "0.0"),
        (30, "0.0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    let text: String = pairs
        .iter()
        .map(|(code, value)| format!("{code:>3}\n{value}\n"))
        .collect();
    let path = std::env::temp_dir().join(format!(
        "uncad-{}-second-reader-dangling-block.dxf",
        std::process::id()
    ));
    std::fs::write(&path, text).expect("temp dir writable");

    let reader =
        acadrust::DxfReader::from_file(&path).expect("the second reader should open the probe");
    let document = reader.read().expect("the second reader should read it");
    let names: Vec<String> = document
        .entities()
        .filter_map(|entity| match entity {
            acadrust::EntityType::Insert(insert) => Some(insert.block_name.clone()),
            _ => None,
        })
        .collect();
    let _ = std::fs::remove_file(&path);

    // Measured, not required: the second reader carries block references as
    // plain names, so the block it cannot find comes back as the name the
    // file wrote. That is the right answer for a *name*, and it is also the
    // reason a consumer cannot tell it from a block that exists -- nothing
    // in the value says which. Recovering the third state from this model
    // means asking the document whether a block of that name is defined.
    assert_eq!(names, vec!["REAL".to_string(), "NOBLOCK".to_string()]);
    assert!(
        document.block_records.contains("REAL"),
        "the defined block should be in the block table"
    );
    assert!(
        !document.block_records.contains("NOBLOCK"),
        "the undefined block must not be in the block table -- that absence \
         is what a consumer would have to consult to recover the third state"
    );
}

/// Where the two readers stand on layer references, per version.
///
/// This crate carries a layer reference as three states. The second reader
/// carries it as a name, and its DWG path fills a name it could not resolve
/// with the literal `"0"` -- a layer name every drawing really has. So a
/// failed resolution and a genuine layer 0 are the same value there, and the
/// handle that would tell them apart is not kept on the entity (its linetype
/// handle is, which is what makes the omission visible rather than a matter
/// of taste).
///
/// The corpus is clean, so the collapse is latent here, not active: this
/// pins that both readers agree on every drawing in it. A disagreement
/// appearing later is either a real defect or the latent case arriving.
#[test]
fn the_two_readers_agree_on_every_layer_name_in_the_corpus() {
    let mut disagreements: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let (Ok(ours), Some(theirs)) = (uncad::parse(&path), second_reader_layers(&path))
            else {
                continue;
            };
            let mut our_names: Vec<String> = ours
                .all_entities()
                .filter_map(|e| e.common().layer.resolved().cloned())
                .collect();
            our_names.sort_unstable();
            our_names.dedup();
            let mut their_names = theirs;
            their_names.sort_unstable();
            their_names.dedup();
            compared += 1;
            if our_names != their_names {
                disagreements.push(format!(
                    "{}: ours {our_names:?} vs theirs {their_names:?}",
                    path.display()
                ));
            }
        }
    }
    assert!(compared > 0, "no drawing was read by both");
    assert!(
        disagreements.is_empty(),
        "{} of {compared} drawings disagree on the set of layer names:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

fn second_reader_layers(path: &Path) -> Option<Vec<String>> {
    let mut reader = acadrust::DwgReader::from_file(path).ok()?;
    let document = reader.read().ok()?;
    Some(
        document
            .entities()
            .map(|entity| entity.common().layer.clone())
            .collect(),
    )
}

/// What each reader finds, entity type by entity type.
///
/// Two readers that open the same files and report the same *number* of
/// entities can still disagree about what those entities are: cycle after
/// cycle, the defects worth finding have been a type read as its neighbour,
/// not a type missed. So the comparison is pinned at type granularity.
///
/// Names are normalized into one vocabulary first, because the two models
/// split the same format differently. Families the two genuinely carve up
/// differently are named in `NOT_COMPARED` and left out rather than forced
/// into a mapping that would invent an agreement or a disagreement.
const NOT_COMPARED: &[&str] = &[
    // Polyline is one DXF entity that both models split, but not along the
    // same seams (2D/3D/lightweight/mesh/pface). Comparing the pieces would
    // measure the split, not the reading.
    "polyline",
    // Block begin/end markers: one model carries them as entities, the other
    // as the shape of its block table.
    "block", // Owned sub-entities, reached differently by each model.
    "attrib", "seqend",
    // Whatever each reader could not place. Not comparable by construction:
    // the same unsupported entity lands in a differently-named bucket.
    "unknown",
];

fn ours_kind(entity: &uncad::Entity) -> &'static str {
    use uncad::Entity as E;
    match entity {
        E::Line(_) => "line",
        E::Circle(_) => "circle",
        E::Arc(_) => "arc",
        E::Point(_) => "point",
        E::Ellipse(_) => "ellipse",
        E::Text(_) => "text",
        E::MText(_) => "mtext",
        E::Spline(_) => "spline",
        E::Dimension(_) => "dimension",
        E::Hatch(_) => "hatch",
        E::Solid(_) => "solid",
        E::Trace(_) => "trace",
        E::Face3D(_) => "face3d",
        E::Insert(_) => "insert",
        E::Ray(_) => "ray",
        E::XLine(_) => "xline",
        E::Viewport(_) => "viewport",
        E::Leader(_) => "leader",
        E::MultiLeader(_) => "multileader",
        E::MLine(_) => "mline",
        E::Solid3D(_) => "solid3d",
        E::Region(_) => "region",
        E::Tolerance(_) => "tolerance",
        E::AcadTable(_) => "table",
        E::Wipeout(_) => "wipeout",
        E::Light(_) => "light",
        E::Attrib(_) | E::Attdef(_) => "attrib",
        E::LwPolyline(_)
        | E::Polyline2D(_)
        | E::Polyline3D(_)
        | E::PolylinePFace(_)
        | E::PolylineMesh(_) => "polyline",
        E::Unknown { .. } => "unknown",
    }
}

fn theirs_kind(entity: &acadrust::EntityType) -> &'static str {
    use acadrust::EntityType as E;
    match entity {
        E::Line(_) => "line",
        E::Circle(_) => "circle",
        E::Arc(_) => "arc",
        E::Point(_) => "point",
        E::Ellipse(_) => "ellipse",
        E::Text(_) => "text",
        E::MText(_) => "mtext",
        E::Spline(_) => "spline",
        E::Helix(_) => "helix",
        E::Dimension(_) => "dimension",
        E::Hatch(_) => "hatch",
        // TRACE and SOLID share a geometry and a struct here, told apart by
        // a flag rather than by a variant -- so the flag is what to read.
        E::Solid(solid) => {
            if solid.is_trace {
                "trace"
            } else {
                "solid"
            }
        }
        E::Face3D(_) => "face3d",
        E::Insert(_) => "insert",
        E::Ray(_) => "ray",
        E::XLine(_) => "xline",
        E::Viewport(_) => "viewport",
        E::Leader(_) => "leader",
        E::MultiLeader(_) => "multileader",
        E::MLine(_) => "mline",
        E::Solid3D(_) => "solid3d",
        E::Region(_) => "region",
        E::Tolerance(_) => "tolerance",
        E::Table(_) => "table",
        E::Wipeout(_) => "wipeout",
        E::Light(_) => "light",
        E::AttributeEntity(_) | E::AttributeDefinition(_) => "attrib",
        E::Block(_) | E::BlockEnd(_) => "block",
        E::Seqend(_) => "seqend",
        E::Polyline(_)
        | E::Polyline2D(_)
        | E::Polyline3D(_)
        | E::LwPolyline(_)
        | E::PolygonMesh(_)
        | E::PolyfaceMesh(_) => "polyline",
        _ => "unknown",
    }
}

#[test]
fn what_each_reader_finds_per_entity_type_is_what_it_was_when_last_measured() {
    use std::collections::{BTreeMap, BTreeSet};
    let mut report = String::new();
    for version in VERSIONS {
        let mut ours_total: BTreeMap<&str, usize> = BTreeMap::new();
        let mut theirs_total: BTreeMap<&str, usize> = BTreeMap::new();
        for path in drawings_for(version) {
            if let Ok(db) = uncad::parse(&path) {
                let mut seen = BTreeSet::new();
                for entity in db.all_entities() {
                    if seen.insert(entity.common().id) {
                        *ours_total.entry(ours_kind(entity)).or_default() += 1;
                    }
                }
            }
            if let Ok(mut reader) = acadrust::DwgReader::from_file(&path) {
                if let Ok(document) = reader.read() {
                    for entity in document.entities() {
                        *theirs_total.entry(theirs_kind(entity)).or_default() += 1;
                    }
                }
            }
        }
        let kinds: BTreeSet<&str> = ours_total
            .keys()
            .chain(theirs_total.keys())
            .copied()
            .filter(|kind| !NOT_COMPARED.contains(kind))
            .collect();
        for kind in kinds {
            let ours = ours_total.get(kind).copied().unwrap_or(0);
            let theirs = theirs_total.get(kind).copied().unwrap_or(0);
            if ours != theirs {
                report.push_str(&format!("{version} {kind}: ours {ours}, theirs {theirs}\n"));
            }
        }
    }
    // What the pinned file holds, and why each line is there:
    //
    // - `helix` in every version: the second reader models HELIX; this
    //   crate's model has no such entity, so it reports one as unknown,
    //   carrying the type name. Nothing is claimed falsely -- the entity is
    //   simply not covered, and the file records where coverage stops.
    // - `table` in one drawing: this crate reports that entity as unknown
    //   under the name its engine gives an unrecognized class, while the
    //   second reader reads it as a table. Which reading is right has not
    //   been measured; the disagreement is recorded, not resolved.
    //
    // Both are of one shape: a place where one reader says "unknown" and the
    // other gives a name. That is the shape worth watching, because the
    // opposite -- both naming it, differently -- is a defect in one of them.
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-types.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "\nthe two readers' per-type disagreement moved; measured now:\n{report}"
    );
}

/// The same leaders, field by field.
///
/// Leaders are matched by handle -- the one identity both readers take
/// straight from the file -- and every field both models carry is compared:
/// the vertices, whether an arrowhead is drawn, the path type, what the
/// leader annotates, and the entity it names. The annotation reference is
/// compared in all three of this crate's states: resolved, it must name the
/// entity whose handle the second reader holds; unresolved, it must carry
/// that handle; absent, the second reader's handle must be null.
#[test]
fn the_two_readers_agree_on_every_leader_field_in_the_corpus() {
    use acadrust::entities::{LeaderCreationType, LeaderPathType};
    use std::collections::BTreeMap;
    use uncad::model::{LeaderAnnotation, LeaderPath, Ref};

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared = 0usize;
    let mut unmatched = 0usize;
    let mut per_version: BTreeMap<&str, usize> = BTreeMap::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::entities::Leader> = document
                .entities()
                .filter_map(|e| match e {
                    acadrust::EntityType::Leader(l) => Some((l.common.handle.value(), l)),
                    _ => None,
                })
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = std::collections::BTreeSet::new();
            for entity in ours.all_entities() {
                let uncad::Entity::Leader(l) = entity else {
                    continue;
                };
                if !seen.insert(entity.common().id) {
                    continue;
                }
                let Some(theirs) = theirs.get(&entity.common().id.value()) else {
                    unmatched += 1;
                    continue;
                };
                compared += 1;
                *per_version.entry(*version).or_default() += 1;
                let mut say = |field: &str, o: String, t: String| {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {field}: ours {o}, theirs {t}",
                            entity.common().id.value()
                        ));
                    }
                };
                let their_vertices: Vec<(f64, f64, f64)> =
                    theirs.vertices.iter().map(|v| (v.x, v.y, v.z)).collect();
                let our_vertices: Vec<(f64, f64, f64)> =
                    l.vertices.iter().map(|v| (v.x, v.y, v.z)).collect();
                say(
                    "vertices",
                    format!("{our_vertices:?}"),
                    format!("{their_vertices:?}"),
                );
                say(
                    "arrowhead",
                    format!("{:?}", l.has_arrowhead),
                    format!("{:?}", Some(theirs.arrow_enabled)),
                );
                let their_path = match theirs.path_type {
                    LeaderPathType::StraightLine => LeaderPath::Straight,
                    LeaderPathType::Spline => LeaderPath::Spline,
                };
                say(
                    "path type",
                    format!("{:?}", l.path_type),
                    format!("{:?}", Some(their_path)),
                );
                let their_annotation = match theirs.creation_type {
                    LeaderCreationType::WithText => LeaderAnnotation::MText,
                    LeaderCreationType::WithTolerance => LeaderAnnotation::Tolerance,
                    LeaderCreationType::WithBlock => LeaderAnnotation::Insert,
                    LeaderCreationType::NoAnnotation => LeaderAnnotation::Nothing,
                };
                say(
                    "annotation",
                    format!("{:?}", l.annotation),
                    format!("{their_annotation:?}"),
                );
                let their_handle = theirs.annotation_handle.value();
                let ours_as_handle = match &l.annotation_id {
                    Ref::Resolved(id) => format!("resolved {:X}", id.value()),
                    Ref::Unresolved(h) => format!("unresolved {h}"),
                    Ref::Absent => "absent".to_string(),
                };
                let agrees = match &l.annotation_id {
                    Ref::Resolved(id) => id.value() == their_handle,
                    Ref::Unresolved(h) => u64::from_str_radix(h, 16).ok() == Some(their_handle),
                    Ref::Absent => their_handle == 0,
                };
                if !agrees {
                    disagreements.push(format!(
                        "{version}/{name} annotation reference: ours {ours_as_handle}, theirs {their_handle:X}"
                    ));
                }
            }
        }
    }
    assert!(compared > 0, "no leader was read by both");
    assert_eq!(unmatched, 0, "a leader one reader has and the other lacks");
    // What the pinned file holds, and why: every other field agrees on every
    // leader, and every line left is the arrowhead flag of an R2010-or-later
    // drawing, which this crate reports as unknown.
    //
    // That is this crate's engine, not a disagreement about the file: from
    // R2010 on it reads the LEADER record one field short (it skips the
    // annotation offset those files still carry), so its flag there is a bit
    // of the offset's encoding -- always off when the offset's z is zero.
    // Its own text-box fields in those files hold exactly the offset the
    // earlier versions read, which is what settled it. This comparison is
    // what first showed the difference; before the flag was reported as
    // unknown, seven of these lines read "no arrowhead" against the second
    // reader's "arrowhead", and the others agreed only by that accident.
    let report = format!(
        "leaders compared {compared} {per_version:?}\n{}",
        disagreements.join("\n")
    );
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-leaders.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "\nthe two readers' leader agreement moved; measured now:\n{report}"
    );
}

/// The same lines, circles, arcs and texts, field by field.
///
/// The same shape as the leader comparison: entities are matched by handle
/// and every field both models carry is compared exactly. Both readers take
/// these values straight from the same bits, so an inexact match is not
/// rounding -- it is one of them reading something else.
#[test]
fn the_two_readers_agree_on_every_line_circle_arc_and_text_field() {
    use std::collections::{BTreeMap, BTreeSet};

    fn p3(x: f64, y: f64, z: f64) -> String {
        format!("({x:?}, {y:?}, {z:?})")
    }

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unmatched: BTreeMap<&str, usize> = BTreeMap::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let kind = match entity {
                    uncad::Entity::Line(_) => "line",
                    uncad::Entity::Circle(_) => "circle",
                    uncad::Entity::Arc(_) => "arc",
                    uncad::Entity::Text(_) => "text",
                    _ => continue,
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(theirs) = theirs.get(&id.value()) else {
                    *unmatched.entry(kind).or_default() += 1;
                    continue;
                };
                *compared.entry(kind).or_default() += 1;
                let mut fields: Vec<(&str, String, String)> = Vec::new();
                use acadrust::EntityType as E;
                match (entity, theirs) {
                    (uncad::Entity::Line(o), E::Line(t)) => {
                        let (s, e) = (o.start_point, o.end_point);
                        fields.push((
                            "start",
                            p3(s.x, s.y, s.z),
                            p3(t.start.x, t.start.y, t.start.z),
                        ));
                        fields.push(("end", p3(e.x, e.y, e.z), p3(t.end.x, t.end.y, t.end.z)));
                    }
                    (uncad::Entity::Circle(o), E::Circle(t)) => {
                        let c = o.center;
                        fields.push((
                            "center",
                            p3(c.x, c.y, c.z),
                            p3(t.center.x, t.center.y, t.center.z),
                        ));
                        fields.push((
                            "radius",
                            format!("{:?}", o.radius),
                            format!("{:?}", t.radius),
                        ));
                    }
                    (uncad::Entity::Arc(o), E::Arc(t)) => {
                        let c = o.center;
                        fields.push((
                            "center",
                            p3(c.x, c.y, c.z),
                            p3(t.center.x, t.center.y, t.center.z),
                        ));
                        fields.push((
                            "radius",
                            format!("{:?}", o.radius),
                            format!("{:?}", t.radius),
                        ));
                        fields.push((
                            "start angle",
                            format!("{:?}", o.start_angle),
                            format!("{:?}", t.start_angle),
                        ));
                        fields.push((
                            "end angle",
                            format!("{:?}", o.end_angle),
                            format!("{:?}", t.end_angle),
                        ));
                    }
                    (uncad::Entity::Text(o), E::Text(t)) => {
                        let s = o.start_point;
                        let ti = t.insertion_point;
                        fields.push((
                            "start",
                            format!("({:?}, {:?})", s.x, s.y),
                            format!("({:?}, {:?})", ti.x, ti.y),
                        ));
                        fields.push((
                            "height",
                            format!("{:?}", o.text_height),
                            format!("{:?}", t.height),
                        ));
                        fields.push((
                            "rotation",
                            format!("{:?}", o.rotation),
                            format!("{:?}", t.rotation),
                        ));
                        fields.push(("text", format!("{:?}", o.text), format!("{:?}", t.value)));
                        fields.push((
                            "alignment",
                            format!(
                                "{:?} {:?}",
                                o.horizontal_justification, o.vertical_justification
                            ),
                            format!("{:?} {:?}", t.horizontal_alignment, t.vertical_alignment),
                        ));
                        // The alignment point means something only for an
                        // aligned text; that is the only case either model
                        // is compared on.
                        let aligned =
                            format!("{:?} {:?}", t.horizontal_alignment, t.vertical_alignment)
                                != "Left Baseline";
                        fields.push((
                            "alignment point",
                            format!("{:?}", o.alignment_point.map(|a| (a.x, a.y))),
                            format!(
                                "{:?}",
                                t.alignment_point.filter(|_| aligned).map(|a| (a.x, a.y))
                            ),
                        ));
                        fields.push((
                            "width factor",
                            format!("{:?}", o.width_factor),
                            format!("{:?}", t.width_factor),
                        ));
                    }
                    _ => {
                        disagreements.push(format!(
                            "{version}/{name} {:X} kind: ours {kind}, theirs {}",
                            id.value(),
                            theirs_kind(theirs)
                        ));
                        continue;
                    }
                }
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {kind} {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    // A requirement, not a measurement: on the corpus the two readers agree
    // on every one of these fields, exactly, for every entity both hold.
    assert!(!compared.is_empty(), "nothing was read by both");
    assert!(
        unmatched.is_empty() && disagreements.is_empty(),
        "compared {compared:?}, unmatched {unmatched:?}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same texts, block references and dimensions, field by field.
///
/// These are the records whose layout changes most across versions, which
/// is where a reader that follows the wrong version's layout goes wrong --
/// the leader comparison found exactly that. Matched by handle, compared
/// exactly, names compared as the names each reader resolved.
#[test]
fn the_two_readers_agree_on_every_mtext_insert_and_dimension_field() {
    use std::collections::{BTreeMap, BTreeSet};
    use uncad::model::Ref;

    fn p3(x: f64, y: f64, z: f64) -> String {
        format!("({x:?}, {y:?}, {z:?})")
    }
    fn name(r: &Ref<String>) -> String {
        match r {
            Ref::Resolved(n) => n.clone(),
            Ref::Unresolved(n) => format!("unresolved {n}"),
            Ref::Absent => "absent".to_string(),
        }
    }

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unmatched: BTreeMap<&str, usize> = BTreeMap::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let kind = match entity {
                    uncad::Entity::MText(_) => "mtext",
                    uncad::Entity::Insert(_) => "insert",
                    uncad::Entity::Dimension(_) => "dimension",
                    _ => continue,
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(theirs) = theirs.get(&id.value()) else {
                    *unmatched.entry(kind).or_default() += 1;
                    continue;
                };
                *compared.entry(kind).or_default() += 1;
                let mut fields: Vec<(&str, String, String)> = Vec::new();
                use acadrust::EntityType as E;
                match (entity, theirs) {
                    (uncad::Entity::MText(o), E::MText(t)) => {
                        let (i, ti) = (o.insertion_point, t.insertion_point);
                        fields.push(("insertion", p3(i.x, i.y, i.z), p3(ti.x, ti.y, ti.z)));
                        fields.push((
                            "height",
                            format!("{:?}", o.text_height),
                            format!("{:?}", t.height),
                        ));
                        fields.push((
                            "rotation",
                            format!("{:?}", o.rotation),
                            format!("{:?}", t.rotation),
                        ));
                        fields.push((
                            "line spacing",
                            format!("{:?}", o.line_spacing_factor),
                            format!("{:?}", t.line_spacing_factor),
                        ));
                        fields.push(("text", format!("{:?}", o.text), format!("{:?}", t.value)));
                        fields.push((
                            "attachment",
                            o.attachment
                                .map_or("unstated".to_string(), |a| format!("{a:?}")),
                            format!("{:?}", t.attachment_point),
                        ));
                        fields.push((
                            "reference width",
                            format!("{:?}", o.reference_width),
                            format!("{:?}", t.rectangle_width),
                        ));
                    }
                    (uncad::Entity::Insert(o), E::Insert(t)) => {
                        let (i, ti) = (o.insertion_point, t.insert_point);
                        fields.push(("insertion", p3(i.x, i.y, i.z), p3(ti.x, ti.y, ti.z)));
                        let s = o.scale;
                        fields.push((
                            "scale",
                            p3(s.x, s.y, s.z),
                            p3(t.x_scale(), t.y_scale(), t.z_scale()),
                        ));
                        fields.push((
                            "rotation",
                            format!("{:?}", o.rotation),
                            format!("{:?}", t.rotation),
                        ));
                        fields.push(("block", name(&o.block_name), t.block_name.clone()));
                    }
                    (uncad::Entity::Dimension(o), E::Dimension(t)) => {
                        let b = t.base();
                        fields.push((
                            "measurement",
                            format!("{:?}", o.measurement),
                            format!("{:?}", Some(b.actual_measurement)),
                        ));
                        let d = b.definition_point;
                        fields.push((
                            "definition point",
                            format!("{:?}", o.definition_point.map(|p| p3(p.x, p.y, p.z))),
                            format!("{:?}", Some(p3(d.x, d.y, d.z))),
                        ));
                        let (m, tm) = (o.text_midpoint, b.text_middle_point);
                        fields.push((
                            "text midpoint",
                            format!("({:?}, {:?})", m.x, m.y),
                            format!("({:?}, {:?})", tm.x, tm.y),
                        ));
                        fields.push((
                            "text rotation",
                            format!("{:?}", o.text_rotation),
                            format!("{:?}", b.text_rotation),
                        ));
                        fields.push(("block", name(&o.block_name), b.block_name.clone()));
                        fields.push(("style", name(&o.style_name), b.style_name.clone()));
                    }
                    _ => {
                        disagreements.push(format!(
                            "{version}/{file} {:X} kind: ours {kind}, theirs {}",
                            id.value(),
                            theirs_kind(theirs)
                        ));
                        continue;
                    }
                }
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{file} {:X} {kind} {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    assert!(!compared.is_empty(), "nothing was read by both");
    // What the pinned file holds, and why each kind of line is there. Every
    // one was checked against the drawing's text twin where it has one, and
    // in each such case this crate states what the twin states:
    //
    // - block names of anonymous blocks (`*D…`, `*U…`): the second reader
    //   renames them with its own counters (`*D`, `*D0`, `*U`, …), and for a
    //   dimension without a block it gives `*U0` where the twin writes no
    //   block at all. For `*U…` references this crate's names are the
    //   twin's. For `*D…` the twin itself numbers them differently from the
    //   binary drawing -- anonymous names are not stable across a save --
    //   so those lines are recorded, not judged.
    // - an MTEXT string holding a `\U+2205`-style escape: the file (and its
    //   twin) holds the escape; the second reader decodes it.
    // - one dimension's definition point, where the twin agrees with this
    //   crate.
    //
    // Fields that are not in the file: none. Rotations, spacing, insertion
    // points, scales, heights, measurements and text midpoints agree
    // exactly on every entity.
    let report = format!(
        "compared {compared:?} unmatched {unmatched:?}\n{}",
        disagreements.join("\n")
    );
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-mtext-insert-dimension.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "\nthe two readers' agreement on these types moved; measured now:\n{report}"
    );
}

/// The same lightweight polylines and ellipses, field by field.
///
/// Matched by handle and compared exactly, like the other field comparisons,
/// each polyline vertex with its bulge (an arc segment's). The second reader
/// also carries vertex widths, which the model does not; how many vertices
/// carry a non-zero one is printed alongside. The comparison is required to
/// have met bulged vertices, so that it cannot pass by comparing only
/// straight segments.
#[test]
fn the_two_readers_agree_on_every_lwpolyline_and_ellipse_field() {
    use std::collections::{BTreeMap, BTreeSet};

    fn p3(x: f64, y: f64, z: f64) -> String {
        format!("({x:?}, {y:?}, {z:?})")
    }

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unmatched: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut vertices, mut bulged, mut widened) = (0usize, 0usize, 0usize);
    let mut bulged_polylines = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let kind = match entity {
                    uncad::Entity::LwPolyline(_) => "lwpolyline",
                    uncad::Entity::Ellipse(_) => "ellipse",
                    _ => continue,
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(theirs) = theirs.get(&id.value()) else {
                    *unmatched.entry(kind).or_default() += 1;
                    continue;
                };
                *compared.entry(kind).or_default() += 1;
                let mut fields: Vec<(&str, String, String)> = Vec::new();
                use acadrust::EntityType as E;
                match (entity, theirs) {
                    (uncad::Entity::LwPolyline(o), E::LwPolyline(t)) => {
                        let ov: Vec<String> = o
                            .vertices
                            .iter()
                            .map(|v| format!("({:?}, {:?}) b{:?}", v.point.x, v.point.y, v.bulge))
                            .collect();
                        let tv: Vec<String> = t
                            .vertices
                            .iter()
                            .map(|v| {
                                format!("({:?}, {:?}) b{:?}", v.location.x, v.location.y, v.bulge)
                            })
                            .collect();
                        fields.push(("vertex count", ov.len().to_string(), tv.len().to_string()));
                        for (i, (a, b)) in ov.iter().zip(&tv).enumerate() {
                            if a != b {
                                fields.push(("vertex", format!("[{i}] {a}"), format!("[{i}] {b}")));
                            }
                        }
                        fields.push(("closed", o.closed.to_string(), t.is_closed.to_string()));
                        fields.push((
                            "elevation",
                            format!("{:?}", o.elevation),
                            format!("{:?}", t.elevation),
                        ));
                        fields.push((
                            "extrusion",
                            p3(o.extrusion.x, o.extrusion.y, o.extrusion.z),
                            p3(t.normal.x, t.normal.y, t.normal.z),
                        ));
                        vertices += t.vertices.len();
                        let b = t.vertices.iter().filter(|v| v.bulge != 0.0).count();
                        bulged += b;
                        bulged_polylines += usize::from(b > 0);
                        widened += t
                            .vertices
                            .iter()
                            .filter(|v| v.start_width != 0.0 || v.end_width != 0.0)
                            .count();
                    }
                    (uncad::Entity::Ellipse(o), E::Ellipse(t)) => {
                        let (c, m) = (o.center, o.major_axis_endpoint);
                        fields.push((
                            "center",
                            p3(c.x, c.y, c.z),
                            p3(t.center.x, t.center.y, t.center.z),
                        ));
                        fields.push((
                            "major axis",
                            p3(m.x, m.y, m.z),
                            p3(t.major_axis.x, t.major_axis.y, t.major_axis.z),
                        ));
                        fields.push((
                            "axis ratio",
                            format!("{:?}", o.axis_ratio),
                            format!("{:?}", t.minor_axis_ratio),
                        ));
                        fields.push((
                            "start",
                            format!("{:?}", o.start_angle),
                            format!("{:?}", t.start_parameter),
                        ));
                        fields.push((
                            "end",
                            format!("{:?}", o.end_angle),
                            format!("{:?}", t.end_parameter),
                        ));
                        let e = o.extrusion;
                        fields.push((
                            "extrusion",
                            p3(e.x, e.y, e.z),
                            p3(t.normal.x, t.normal.y, t.normal.z),
                        ));
                    }
                    _ => {
                        disagreements.push(format!(
                            "{version}/{name} {:X} kind: ours {kind}, theirs {}",
                            id.value(),
                            theirs_kind(theirs)
                        ));
                        continue;
                    }
                }
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {kind} {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    println!(
        "compared {compared:?}; lwpolyline vertices {vertices}: bulge != 0 on {bulged} \
         (in {bulged_polylines} polylines), width != 0 on {widened}"
    );
    assert!(!compared.is_empty(), "nothing was read by both");
    assert!(bulged > 0, "no bulged vertex was compared");
    assert!(
        unmatched.is_empty() && disagreements.is_empty(),
        "compared {compared:?}, unmatched {unmatched:?}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same splines, field by field: degree, the closed and periodic bits,
/// knots, weights, control points and fit points.
///
/// A spline record comes in two forms -- by control points or by fit
/// points -- and which one a record is decides which fields exist at all,
/// so the form is compared first, as whichever of the two point lists each
/// reader filled.
#[test]
fn the_two_readers_agree_on_every_spline_field() {
    use std::collections::{BTreeMap, BTreeSet};

    fn pts(points: impl Iterator<Item = (f64, f64, f64)>) -> String {
        points
            .map(|(x, y, z)| format!("({x:?}, {y:?}, {z:?})"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared = 0usize;
    let mut unmatched = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let uncad::Entity::Spline(o) = entity else {
                    continue;
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(acadrust::EntityType::Spline(t)) = theirs.get(&id.value()) else {
                    unmatched += 1;
                    continue;
                };
                compared += 1;
                let flag = |b: Option<bool>| b.map_or("unstated".to_string(), |b| b.to_string());
                let fields: Vec<(&str, String, String)> = vec![
                    ("degree", o.degree.to_string(), t.degree.to_string()),
                    (
                        "control points",
                        pts(o.control_points.iter().map(|p| (p.x, p.y, p.z))),
                        pts(t.control_points.iter().map(|p| (p.x, p.y, p.z))),
                    ),
                    (
                        "fit points",
                        pts(o.fit_points.iter().map(|p| (p.x, p.y, p.z))),
                        pts(t.fit_points.iter().map(|p| (p.x, p.y, p.z))),
                    ),
                    ("knots", format!("{:?}", o.knots), format!("{:?}", t.knots)),
                    (
                        "weights",
                        format!("{:?}", o.weights),
                        format!("{:?}", t.weights),
                    ),
                    ("closed", flag(o.closed), t.flags.closed.to_string()),
                    ("periodic", flag(o.periodic), t.flags.periodic.to_string()),
                ];
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} spline {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    assert!(compared > 0, "nothing was read by both");
    // Every remaining line is a fit-point spline whose record states no
    // periodic bit in any version, and no closed bit before R2013: this
    // reader reports those as unstated, the other reader as false. Degree,
    // knots, weights, control points and fit points agree exactly on every
    // spline both read.
    let report = format!(
        "compared {compared} unmatched {unmatched}
{}",
        disagreements.join(
            "
"
        )
    );
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/second-reader-splines.txt"
    ))
    .unwrap_or_default();
    assert_eq!(
        report.trim(),
        pinned.trim(),
        "
the two readers' agreement on splines moved; measured now:
{report}"
    );
}

/// Whether an entity is marked invisible (DXF 60), for every entity both
/// readers hold. A dynamic block keeps its hidden visibility states as
/// invisible entities, so the flag is common -- and an entity drawn that
/// the file hides is a wrong picture, not a missing detail.
#[test]
fn the_two_readers_agree_on_which_entities_are_invisible() {
    use std::collections::{BTreeMap, BTreeSet};

    let (mut compared, mut invisible) = (0usize, 0usize);
    let mut disagreements: Vec<String> = Vec::new();
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, bool> = document
                .entities()
                .map(|e| (e.common().handle.value(), e.common().invisible))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let common = entity.common();
                if !seen.insert(common.id) {
                    continue;
                }
                let Some(&t) = theirs.get(&common.id.value()) else {
                    continue;
                };
                compared += 1;
                invisible += usize::from(common.invisible);
                if common.invisible != t {
                    disagreements.push(format!(
                        "{version}/{name} {:X}: ours {}, theirs {t}",
                        common.id.value(),
                        common.invisible
                    ));
                }
            }
        }
    }
    assert!(invisible > 0, "the corpus has invisible entities");
    assert!(
        disagreements.is_empty(),
        "compared {compared}, invisible {invisible}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
    println!("compared {compared}, invisible {invisible}");
}

/// The same 2D and 3D POLYLINEs, vertex by vertex: every position, each 2D
/// vertex with its bulge, and the closed bit.
///
/// The vertex list is the field most at risk here: a POLYLINE stores its
/// vertices as separate records chained to it, and walking that chain is
/// version-dependent -- a walk that stops one record early drops the last
/// vertex without an error. So the count is compared too, and the
/// comparison is required to have met polylines from before R2004 (where the
/// chain runs `first_vertex..last_vertex`) as well as after.
#[test]
fn the_two_readers_agree_on_every_2d_and_3d_polyline_vertex() {
    use std::collections::{BTreeMap, BTreeSet};

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unmatched = 0usize;
    let mut chained = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let id = entity.common().id;
                let (kind, ov, oc): (&str, Vec<String>, bool) = match entity {
                    uncad::Entity::Polyline2D(o) => (
                        "polyline 2d",
                        o.vertices
                            .iter()
                            .map(|v| format!("({:?}, {:?}) b{:?}", v.point.x, v.point.y, v.bulge))
                            .collect(),
                        o.closed,
                    ),
                    uncad::Entity::Polyline3D(o) => (
                        "polyline 3d",
                        o.vertices
                            .iter()
                            .map(|v| format!("({:?}, {:?}, {:?})", v.x, v.y, v.z))
                            .collect(),
                        o.closed,
                    ),
                    _ => continue,
                };
                if !seen.insert(id) {
                    continue;
                }
                use acadrust::EntityType as E;
                let (tv, tc): (Vec<String>, bool) = match (kind, theirs.get(&id.value())) {
                    ("polyline 2d", Some(E::Polyline2D(t))) => (
                        t.vertices
                            .iter()
                            .map(|v| {
                                format!("({:?}, {:?}) b{:?}", v.location.x, v.location.y, v.bulge)
                            })
                            .collect(),
                        t.flags.is_closed(),
                    ),
                    ("polyline 3d", Some(E::Polyline3D(t))) => (
                        t.vertices
                            .iter()
                            .map(|v| {
                                format!(
                                    "({:?}, {:?}, {:?})",
                                    v.position.x, v.position.y, v.position.z
                                )
                            })
                            .collect(),
                        t.flags.closed,
                    ),
                    _ => {
                        unmatched += 1;
                        continue;
                    }
                };
                *compared.entry(kind).or_default() += 1;
                if matches!(*version, "2000" | "r14") {
                    chained += 1;
                }
                let mut fields = vec![
                    ("vertex count", ov.len().to_string(), tv.len().to_string()),
                    ("closed", oc.to_string(), tc.to_string()),
                ];
                for (i, (a, b)) in ov.iter().zip(&tv).enumerate() {
                    if a != b {
                        fields.push(("vertex", format!("[{i}] {a}"), format!("[{i}] {b}")));
                    }
                }
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {kind} {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    println!("compared {compared:?}, from before R2004 {chained}, unmatched {unmatched}");
    assert!(!compared.is_empty(), "nothing was read by both");
    assert!(chained > 0, "no polyline from before R2004 was compared");
    assert!(
        unmatched == 0 && disagreements.is_empty(),
        "compared {compared:?}, unmatched {unmatched}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same 3DFACEs, field by field: the four corners and which edges are
/// invisible. A mesh of faces hides the edges its faces share, so the flags
/// decide what outline is drawn; the comparison is required to have met
/// faces with hidden edges.
#[test]
fn the_two_readers_agree_on_every_3dface_corner_and_hidden_edge() {
    use std::collections::{BTreeMap, BTreeSet};

    let mut disagreements: Vec<String> = Vec::new();
    let (mut compared, mut unmatched, mut with_hidden) = (0usize, 0usize, 0usize);
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let uncad::Entity::Face3D(o) = entity else {
                    continue;
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(acadrust::EntityType::Face3D(t)) = theirs.get(&id.value()) else {
                    unmatched += 1;
                    continue;
                };
                compared += 1;
                with_hidden += usize::from(o.invisible_edges.iter().any(|h| *h));
                let p = |x: f64, y: f64, z: f64| format!("({x:?}, {y:?}, {z:?})");
                let ours_corners = [o.corner1, o.corner2, o.corner3, o.corner4]
                    .iter()
                    .map(|c| p(c.x, c.y, c.z))
                    .collect::<Vec<_>>()
                    .join(" ");
                let their_corners = [
                    t.first_corner,
                    t.second_corner,
                    t.third_corner,
                    t.fourth_corner,
                ]
                .iter()
                .map(|c| p(c.x, c.y, c.z))
                .collect::<Vec<_>>()
                .join(" ");
                let flags = &t.invisible_edges;
                let their_hidden = [
                    flags.is_first_invisible(),
                    flags.is_second_invisible(),
                    flags.is_third_invisible(),
                    flags.is_fourth_invisible(),
                ];
                for (field, a, b) in [
                    ("corners", ours_corners, their_corners),
                    (
                        "invisible edges",
                        format!("{:?}", o.invisible_edges),
                        format!("{their_hidden:?}"),
                    ),
                ] {
                    if a != b {
                        disagreements.push(format!(
                            "{version}/{name} {:X} 3dface {field}: ours {a}, theirs {b}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    println!("compared {compared}, with hidden edges {with_hidden}, unmatched {unmatched}");
    assert!(compared > 0, "nothing was read by both");
    assert!(with_hidden > 0, "no face with a hidden edge was compared");
    assert!(
        unmatched == 0 && disagreements.is_empty(),
        "compared {compared}, unmatched {unmatched}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same SOLIDs and TRACEs, field by field: the four corners, the
/// elevation they share and the extrusion. The other reader keeps the
/// elevation as each corner's z and the extrusion as the normal.
#[test]
fn the_two_readers_agree_on_every_solid_and_trace_field() {
    use std::collections::{BTreeMap, BTreeSet};

    let mut disagreements: Vec<String> = Vec::new();
    let (mut compared, mut unmatched, mut raised) = (0usize, 0usize, 0usize);
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            for entity in ours.all_entities() {
                let (kind, o) = match entity {
                    uncad::Entity::Solid(o) => ("solid", o),
                    uncad::Entity::Trace(o) => ("trace", o),
                    _ => continue,
                };
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                let Some(acadrust::EntityType::Solid(t)) = theirs.get(&id.value()) else {
                    unmatched += 1;
                    continue;
                };
                compared += 1;
                raised += usize::from(o.elevation != 0.0);
                let xy = |x: f64, y: f64| format!("({x:?}, {y:?})");
                let fields = [
                    (
                        "corners",
                        [o.corner1, o.corner2, o.corner3, o.corner4]
                            .iter()
                            .map(|c| xy(c.x, c.y))
                            .collect::<Vec<_>>()
                            .join(" "),
                        [
                            t.first_corner,
                            t.second_corner,
                            t.third_corner,
                            t.fourth_corner,
                        ]
                        .iter()
                        .map(|c| xy(c.x, c.y))
                        .collect::<Vec<_>>()
                        .join(" "),
                    ),
                    (
                        "elevation",
                        format!("{:?}", o.elevation),
                        format!("{:?}", t.first_corner.z),
                    ),
                    (
                        "extrusion",
                        format!(
                            "({:?}, {:?}, {:?})",
                            o.extrusion.x, o.extrusion.y, o.extrusion.z
                        ),
                        format!("({:?}, {:?}, {:?})", t.normal.x, t.normal.y, t.normal.z),
                    ),
                ];
                for (field, a, b) in fields {
                    if a != b {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {kind} {field}: ours {a}, theirs {b}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    println!("compared {compared}, with an elevation {raised}, unmatched {unmatched}");
    assert!(compared > 0, "nothing was read by both");
    assert!(
        unmatched == 0 && disagreements.is_empty(),
        "compared {compared}, unmatched {unmatched}, {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same attribute values and definitions, field by field: where each is,
/// what it says, and how it is aligned.
///
/// An attribute's vertical alignment is the one code whose DXF group differs
/// from a TEXT's (74, not 73), which is where a reader that treats an
/// attribute as a text goes wrong. Matched by handle, compared exactly.
#[test]
fn the_two_readers_agree_on_every_attribute_field() {
    use std::collections::{BTreeMap, BTreeSet};

    let mut disagreements: Vec<String> = Vec::new();
    let mut compared: BTreeMap<&str, usize> = BTreeMap::new();
    let mut aligned = 0usize;
    for version in VERSIONS {
        for path in drawings_for(version) {
            let Ok(ours) = uncad::parse(&path) else {
                continue;
            };
            let Ok(mut reader) = acadrust::DwgReader::from_file(&path) else {
                continue;
            };
            let Ok(document) = reader.read() else {
                continue;
            };
            let theirs: BTreeMap<u64, &acadrust::EntityType> = document
                .entities()
                .map(|e| (e.common().handle.value(), e))
                .collect();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut seen = BTreeSet::new();
            let attribs = ours.all_entities().flat_map(|e| match e {
                uncad::Entity::Insert(i) => i
                    .attribs
                    .iter()
                    .cloned()
                    .map(uncad::Entity::Attrib)
                    .collect(),
                other => vec![other.clone()],
            });
            for entity in attribs {
                let id = entity.common().id;
                if !seen.insert(id) {
                    continue;
                }
                use acadrust::EntityType as E;
                let (kind, fields): (&str, Vec<(&str, String, String)>) =
                    match (&entity, theirs.get(&id.value())) {
                        (uncad::Entity::Attrib(o), Some(E::AttributeEntity(t))) => {
                            let t_aligned =
                                format!("{:?} {:?}", t.horizontal_alignment, t.vertical_alignment)
                                    != "Left Baseline";
                            (
                                "attrib",
                                vec![
                                    (
                                        "start",
                                        format!("{:?}", (o.start_point.x, o.start_point.y)),
                                        format!("{:?}", (t.insertion_point.x, t.insertion_point.y)),
                                    ),
                                    ("tag", o.tag.clone(), t.tag.clone()),
                                    ("value", o.text.clone(), t.value.clone()),
                                    (
                                        "alignment",
                                        format!(
                                            "{:?} {:?}",
                                            o.horizontal_justification, o.vertical_justification
                                        ),
                                        format!(
                                            "{:?} {:?}",
                                            t.horizontal_alignment, t.vertical_alignment
                                        ),
                                    ),
                                    (
                                        "alignment point",
                                        format!("{:?}", o.alignment_point.map(|a| (a.x, a.y))),
                                        format!(
                                            "{:?}",
                                            t_aligned.then_some((
                                                t.alignment_point.x,
                                                t.alignment_point.y
                                            ))
                                        ),
                                    ),
                                    (
                                        "width factor",
                                        format!("{:?}", o.width_factor),
                                        format!("{:?}", t.width_factor),
                                    ),
                                ],
                            )
                        }
                        (uncad::Entity::Attdef(o), Some(E::AttributeDefinition(t))) => {
                            let t_aligned =
                                format!("{:?} {:?}", t.horizontal_alignment, t.vertical_alignment)
                                    != "Left Baseline";
                            (
                                "attdef",
                                vec![
                                    (
                                        "start",
                                        format!("{:?}", (o.start_point.x, o.start_point.y)),
                                        format!("{:?}", (t.insertion_point.x, t.insertion_point.y)),
                                    ),
                                    ("tag", o.tag.clone(), t.tag.clone()),
                                    (
                                        "alignment",
                                        format!(
                                            "{:?} {:?}",
                                            o.horizontal_justification, o.vertical_justification
                                        ),
                                        format!(
                                            "{:?} {:?}",
                                            t.horizontal_alignment, t.vertical_alignment
                                        ),
                                    ),
                                    (
                                        "alignment point",
                                        format!("{:?}", o.alignment_point.map(|a| (a.x, a.y))),
                                        format!(
                                            "{:?}",
                                            t_aligned.then_some((
                                                t.alignment_point.x,
                                                t.alignment_point.y
                                            ))
                                        ),
                                    ),
                                    (
                                        "width factor",
                                        format!("{:?}", o.width_factor),
                                        format!("{:?}", t.width_factor),
                                    ),
                                ],
                            )
                        }
                        _ => continue,
                    };
                *compared.entry(kind).or_default() += 1;
                if fields
                    .iter()
                    .any(|(f, o, _)| *f == "alignment" && o != "Left Baseline")
                {
                    aligned += 1;
                }
                for (field, o, t) in fields {
                    if o != t {
                        disagreements.push(format!(
                            "{version}/{name} {:X} {kind} {field}: ours {o}, theirs {t}",
                            id.value()
                        ));
                    }
                }
            }
        }
    }
    // A requirement, not a measurement: every attribute both readers hold
    // agrees on these fields, exactly -- aligned ones included.
    assert!(!compared.is_empty(), "no attribute was read by both");
    assert!(
        aligned > 0,
        "the corpus holds no aligned attribute to compare"
    );
    assert!(
        disagreements.is_empty(),
        "compared {compared:?} ({aligned} aligned), {} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}
