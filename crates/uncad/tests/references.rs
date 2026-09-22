//! Reference fields (layer, block, style names) are three-state values, not
//! strings: a name that resolved, a field the file has no handle for, and a
//! handle nothing answers to. These tests pin the shape from both sides -- a
//! reference that must resolve, and one that must not come back as a name.

use std::fs;
use std::path::{Path, PathBuf};

use uncad::model::{
    Confidence, EntityCommon, EntityId, LeaderAnnotation, LineEntity, Origin, Point3D, Ref,
};
use uncad::{CadDatabase, Entity};

const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
);

fn layer_of(e: &Entity) -> &Ref<String> {
    &e.common().layer
}

#[test]
fn every_layer_in_a_clean_corpus_drawing_resolves_to_a_non_empty_name() {
    let db = uncad::parse(CORPUS_DXF).expect("the corpus DXF should parse");
    assert!(!db.entities.is_empty());
    for e in &db.entities {
        match layer_of(e) {
            Ref::Resolved(name) => assert!(!name.is_empty(), "{e:?}"),
            other => panic!("a clean drawing's entity has an unresolved layer: {other:?} on {e:?}"),
        }
    }
}

#[test]
fn the_three_states_serialize_distinguishably_and_round_trip() {
    fn line(layer: Ref<String>) -> Entity {
        Entity::Line(LineEntity {
            common: EntityCommon {
                id: EntityId::new(1),
                origin: Origin::Vector,
                confidence: Confidence::High,
                source_handle: Ref::Resolved("1".to_string()),
                layer,
                color_index: 256,
                true_color: None,
            },
            start_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        })
    }
    let db = CadDatabase {
        entities: vec![
            line(Ref::Resolved("0".to_string())),
            line(Ref::Absent),
            line(Ref::Unresolved("2A".to_string())),
        ],
        tables: Default::default(),
        read_diagnostics: Default::default(),
    };
    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serialize");
    assert!(
        json.contains(r#""layer":{"type":"RESOLVED","data":"0"}"#),
        "{json}"
    );
    assert!(json.contains(r#""layer":{"type":"ABSENT"}"#), "{json}");
    assert!(
        json.contains(r#""layer":{"type":"UNRESOLVED","data":"2A"}"#),
        "{json}"
    );
    // The old fill is gone from the wire: no reference is an empty string.
    assert!(!json.contains(r#""layer":"""#), "{json}");

    let back: CadDatabase = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, db);
}

/// Removes its file on drop.
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
impl TempFile {
    fn new(name: &str) -> Self {
        TempFile(std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name)))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

/// Two INSERTs in one file: one whose block is defined in the BLOCKS
/// section, one whose block is not. The first must resolve to its name; the
/// second must not come back as a *name* -- and neither as an empty string.
/// (Measured: LibreDWG's importer stores no handle at all for the unknown
/// block, so it reads as `Absent`; a handle that points at nothing would be
/// `Unresolved`. Both are "not a name", which is what a consumer must be
/// able to tell from "the block called `""`".)
///
/// A LINE on a layer no LAYER table declares is not testable this way:
/// LibreDWG refuses such a file outright (`IOERROR`), so a dangling layer
/// cannot even be written into a DXF by hand.
#[test]
fn a_reference_to_a_missing_block_is_not_a_name() {
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
    let file = TempFile::new("dangling-block.dxf");
    fs::write(file.path(), text).expect("temp dir writable");
    let db = uncad::parse(file.path()).expect("the DXF should parse");

    let inserts: Vec<_> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Insert(i) => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(inserts.len(), 2, "{:?}", db.entities);

    // The control: a block that exists resolves to its name.
    assert_eq!(inserts[0].block_name, Ref::Resolved("REAL".to_string()));
    // The case: a block that does not exist is not a name of any kind.
    assert!(
        !inserts[1].block_name.is_resolved(),
        "a block that does not exist must not come back as a name: {:?}",
        inserts[1].block_name
    );

    for e in &db.entities {
        assert_ne!(
            layer_of(e),
            &Ref::Resolved(String::new()),
            "an empty layer name is the old fill, not a state: {e:?}"
        );
    }
}

/// Before R13 a drawing points at its LAYER table by index, not by handle.
/// Measured across the LibreDWG corpus before this was resolved: every entity
/// layer in the pre-R13 drawings came back `Unresolved("0")` while the table
/// itself was read. The R2000 drawing of the same content is the oracle: the
/// R11 file must yield the same set of layer names (the two files hold a
/// different number of entities, so they are compared as sets), not merely
/// names that exist in its table -- an off-by-one index would still produce a
/// valid name.
#[test]
fn pre_r13_layer_references_resolve_by_index_to_the_same_names_as_the_r2000_twin() {
    let old = uncad::parse(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/r11/entities-2d.dwg"
    ))
    .expect("the R11 corpus DWG should parse");
    let modern = uncad::parse(CORPUS_DXF).expect("the R2000 corpus DXF should parse");
    assert!(!old.entities.is_empty());
    assert!(
        old.tables.layers.contains_key("0"),
        "{:?}",
        old.tables.layers.keys().collect::<Vec<_>>()
    );

    fn resolved_layer_names(db: &uncad::CadDatabase) -> std::collections::BTreeSet<&str> {
        db.entities
            .iter()
            .map(|e| match layer_of(e) {
                Ref::Resolved(name) => name.as_str(),
                other => panic!("layer not resolved: {other:?} on {e:?}"),
            })
            .collect()
    }
    let old_names = resolved_layer_names(&old);
    let modern_names = resolved_layer_names(&modern);
    assert_eq!(
        old_names, modern_names,
        "R11 and R2000 twins must name the same layers"
    );
    for name in &old_names {
        assert!(old.tables.layers.contains_key(*name), "{name}");
    }
}

const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

fn drawings_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("corpus directory should be readable") {
        let path = entry.expect("corpus entry should be readable").path();
        if path.is_dir() {
            drawings_under(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg") || e.eq_ignore_ascii_case("dxf"))
        {
            out.push(path);
        }
    }
}

fn corpus_drawings() -> Vec<PathBuf> {
    let mut out = Vec::new();
    drawings_under(Path::new(CORPUS_ROOT), &mut out);
    out.sort();
    out
}

/// DXF 340 on a LEADER names another entity of the *same* drawing, so the
/// reference ID the model carries has to be one that drawing's entities
/// answer to. An ID minted by any other route points at nothing, and a
/// consumer cannot tell that from a leader that annotates nothing on
/// purpose -- the two are the same `None`/`Some` shape.
#[test]
fn a_leader_annotation_id_names_an_entity_the_same_drawing_carries() {
    let mut dangling: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut leaders = 0usize;
    for path in corpus_drawings() {
        let Ok(db) = uncad::parse(&path) else {
            continue;
        };
        let ids: std::collections::BTreeSet<EntityId> =
            db.all_entities().map(|e| e.common().id).collect();
        for entity in db.all_entities() {
            let Entity::Leader(leader) = entity else {
                continue;
            };
            leaders += 1;
            let Some(id) = leader.annotation_id else {
                continue;
            };
            checked += 1;
            if !ids.contains(&id) {
                dangling.push(format!("{}: {:#x}", path.display(), id.value()));
            }
        }
    }
    assert!(leaders > 0, "the corpus should hold at least one LEADER");
    assert!(
        checked > 0,
        "{leaders} LEADERs, none of which carried an annotation reference"
    );
    assert!(
        dangling.is_empty(),
        "{} of {checked} annotation references name no entity of their own drawing:\n{}",
        dangling.len(),
        dangling.join("\n")
    );
}

/// The file states the annotation twice: DXF 73 says what kind of thing the
/// leader annotates, and DXF 340 names the entity. A reference ID read the
/// wrong way still lands on *an* entity often enough for the previous test
/// to pass, so this one checks the two statements agree.
#[test]
fn a_leader_annotation_reference_points_at_the_kind_of_entity_it_declares() {
    let mut disagreements: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for path in corpus_drawings() {
        let Ok(db) = uncad::parse(&path) else {
            continue;
        };
        let kinds: std::collections::BTreeMap<EntityId, &'static str> = db
            .all_entities()
            .map(|e| (e.common().id, entity_kind(e)))
            .collect();
        for entity in db.all_entities() {
            let Entity::Leader(leader) = entity else {
                continue;
            };
            let Some(id) = leader.annotation_id else {
                continue;
            };
            let expected = match leader.annotation {
                LeaderAnnotation::MText => "MText",
                LeaderAnnotation::Tolerance => "Tolerance",
                LeaderAnnotation::Insert => "Insert",
                // The format's fallback: the file declares nothing, so
                // there is nothing to agree with.
                LeaderAnnotation::Nothing => continue,
            };
            checked += 1;
            let found = kinds.get(&id).copied().unwrap_or("(no such entity)");
            if found != expected {
                disagreements.push(format!(
                    "{}: leader declares {expected}, reference {:#x} is {found}",
                    path.display(),
                    id.value()
                ));
            }
        }
    }
    assert!(
        checked > 0,
        "no leader in the corpus declares an annotation"
    );
    assert!(
        disagreements.is_empty(),
        "{} of {checked} leaders name an entity of another kind:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same drawing in both formats must yield the same reference IDs: an ID
/// is minted from what the file says, and both files say the same thing.
/// This is the pair that has a leader whose annotation reference is set.
#[test]
fn a_leader_annotation_id_is_the_same_read_as_dwg_and_as_its_dxf_twin() {
    let dwg = annotation_ids_of(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/2000/Leader.dwg"
    )));
    let dxf = annotation_ids_of(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/2000/Leader.dxf"
    )));
    assert!(
        !dwg.is_empty(),
        "the twin should hold a leader with an annotation reference"
    );
    assert_eq!(dwg, dxf, "the two formats disagree on the reference IDs");
}

fn annotation_ids_of(path: &Path) -> Vec<(u64, Option<u64>)> {
    let db = uncad::parse(path).expect("the twin parses");
    let mut out: Vec<(u64, Option<u64>)> = db
        .all_entities()
        .filter_map(|e| match e {
            Entity::Leader(l) => Some((e.common().id.value(), l.annotation_id.map(|i| i.value()))),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn entity_kind(entity: &Entity) -> &'static str {
    match entity {
        Entity::MText(_) => "MText",
        Entity::Text(_) => "Text",
        Entity::Tolerance(_) => "Tolerance",
        Entity::Insert(_) => "Insert",
        Entity::Leader(_) => "Leader",
        _ => "other",
    }
}
