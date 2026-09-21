//! Reference fields (layer, block, style names) are three-state values, not
//! strings: a name that resolved, a field the file has no handle for, and a
//! handle nothing answers to. These tests pin the shape from both sides -- a
//! reference that must resolve, and one that must not come back as a name.

use std::fs;
use std::path::{Path, PathBuf};

use uncad::model::{EntityCommon, LineEntity, Point3D, Ref};
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
                handle: "1".to_string(),
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

/// Every corpus file that parses, tallied by reference state. Measured: no
/// `Unresolved` carries a bare `0` any more (a null handle is `Absent` from
/// R13 on -- the 18 DIMENSIONs inside one R2018 file's dynamic-block
/// definitions -- and a pre-R13 index is looked up in the table); the only
/// unresolved references left in the corpus are the 22 layers of the R1.4
/// drawing, whose LAYER table LibreDWG does not read at all, kept as `idx:1`.
/// No block reference is unresolved. The counts are printed so a change in
/// the corpus or in the resolver shows up as a number, not as a feeling.
#[test]
fn corpus_references_never_carry_a_bare_zero_handle() {
    use std::collections::BTreeMap;
    let corpus = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data"
    ));
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(corpus).expect("corpus dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            for sub in std::fs::read_dir(&path).expect("subdir") {
                files.push(sub.expect("entry").path());
            }
        } else {
            files.push(path);
        }
    }
    files.retain(|p| {
        matches!(
            p.extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("dwg" | "dxf")
        )
    });
    files.sort();

    // (directory, field, state) -> count
    let mut tally: BTreeMap<(String, &str, &str), usize> = BTreeMap::new();
    let mut bare_zero: Vec<String> = Vec::new();
    // (file, payload, size of the file's layer table) per unresolved reference
    let mut unresolved: Vec<(String, String, usize)> = Vec::new();
    let mut parsed = 0usize;
    for path in &files {
        let Ok(db) = uncad::parse(path) else { continue };
        parsed += 1;
        let dir = path
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|d| d.to_str())
            .unwrap_or("")
            .to_string();
        let mut note = |field: &'static str, r: &Ref<String>| {
            let state = match r {
                Ref::Resolved(_) => "resolved",
                Ref::Absent => "absent",
                Ref::Unresolved(h) => {
                    if h == "0" {
                        bare_zero.push(format!("{}:{field}", path.display()));
                    }
                    unresolved.push((
                        path.display().to_string(),
                        h.clone(),
                        db.tables.layers.len(),
                    ));
                    "unresolved"
                }
            };
            *tally.entry((dir.clone(), field, state)).or_default() += 1;
        };
        let in_blocks = db
            .tables
            .block_records
            .values()
            .flat_map(|b| b.entities.iter());
        for e in db.entities.iter().chain(in_blocks) {
            note("layer", layer_of(e));
            match e {
                uncad::Entity::Insert(i) => note("block", &i.block_name),
                uncad::Entity::Dimension(d) => note("block", &d.block_name),
                uncad::Entity::AcadTable(t) => note("block", &t.block_name),
                uncad::Entity::MLine(m) => note("mlinestyle", &m.mlinestyle_name),
                _ => {}
            }
        }
    }
    println!("parsed {parsed} of {} corpus files", files.len());
    for ((dir, field, state), n) in &tally {
        println!("{dir:>14} {field:<10} {state:<10} {n}");
    }
    assert!(bare_zero.is_empty(), "bare-zero handles: {bare_zero:?}");
    assert_eq!(unresolved.len(), 22, "{unresolved:?}");
    for (file, payload, layer_table_len) in &unresolved {
        assert!(file.contains("r1.4"), "{file}");
        assert_eq!(payload, "idx:1", "{file}");
        assert_eq!(
            *layer_table_len, 0,
            "{file}: an index can only go unresolved when the table is missing"
        );
    }
    assert!(
        !tally.contains_key(&("2018".to_string(), "block", "unresolved")),
        "R13+ null block handles are absent, not unresolved"
    );
    assert_eq!(tally[&("2018".to_string(), "block", "absent")], 18);
}
