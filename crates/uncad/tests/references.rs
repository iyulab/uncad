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
