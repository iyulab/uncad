//! An R2007+ DXF must resolve its entities' *handle-borne* names -- the
//! layer every entity points at and the block record every INSERT points at
//! -- exactly as the DWG of the same drawing does.
//!
//! LibreDWG's DXF reader stores table-record names as UTF-16 as soon as
//! `$ACADVER` is AC1021 or later (dynapi widens every string field once
//! `header.version >= R_2007`), but its name->handle lookups read those
//! names back as 8-bit C strings, which stop at the first NUL. So every
//! name longer than one character compared as just its first letter,
//! `entity->layer` and `INSERT->block_header` were left NULL, and this
//! crate saw `layer: ""` and `block_name: ""` for all but the layer-`0`
//! entities. See the `uncad local patch` block in
//! `crates/libredwg-sys/vendor/libredwg/src/dwg.c` and `docs/CAVEATS.md`.
//!
//! The corpus carries the same drawing in both formats, which makes the DWG
//! an independent reference for the DXF: two different decoders, one
//! drawing. The absolute numbers below were derived a third way -- by
//! walking the DXF's own text -- and each assertion says how.

use std::collections::BTreeMap;

use uncad::model::Entity;
use uncad::visibility::hidden_reason;
use uncad::CadDatabase;

const DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dxf"
);
const DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dwg"
);

/// How many model-space entities sit on each layer name.
fn layers(db: &CadDatabase) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for entity in &db.entities {
        *out.entry(entity.common().layer.clone()).or_insert(0) += 1;
    }
    out
}

/// How many INSERTs name each block record.
fn block_names(db: &CadDatabase) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for entity in &db.entities {
        if let Entity::Insert(insert) = entity {
            *out.entry(insert.block_name.clone()).or_insert(0) += 1;
        }
    }
    out
}

/// Every entity the drawing holds: model space plus each *named* block
/// definition. `*Model_Space`/`*Paper_Space*` are skipped because their
/// contents are already in `db.entities`.
fn all_entities(db: &CadDatabase) -> Vec<&Entity> {
    let mut out: Vec<&Entity> = db.entities.iter().collect();
    for (name, record) in &db.tables.block_records {
        if name.starts_with("*Model_Space") || name.starts_with("*Paper_Space") {
            continue;
        }
        out.extend(record.entities.iter());
    }
    out
}

/// How many of those entities each hiding rule takes out of the drawing.
/// Layer-driven rules can only fire once the layer name resolves, which is
/// what this whole file is about.
fn hidden(db: &CadDatabase) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for entity in all_entities(db) {
        if let Some(reason) = hidden_reason(entity.common(), &db.tables) {
            *out.entry(reason.as_str()).or_insert(0) += 1;
        }
    }
    out
}

#[test]
fn an_r2018_dxf_resolves_the_same_layers_blocks_and_hidden_entities_as_its_dwg_twin() {
    let dxf = uncad::parse(DXF).expect("the corpus DXF parses");
    let dwg = uncad::parse(DWG).expect("the corpus DWG parses");

    // The two formats hold the same drawing, so these must agree whatever
    // the numbers are. Before the fix the DXF side was {"": 65, "0": 7}.
    assert_eq!(
        layers(&dxf),
        layers(&dwg),
        "DXF and DWG must put the same entities on the same layers"
    );
    assert_eq!(
        block_names(&dxf),
        block_names(&dwg),
        "DXF and DWG must resolve the same block records"
    );
    assert_eq!(
        hidden(&dxf),
        hidden(&dwg),
        "DXF and DWG must hide the same entities"
    );
    assert!(
        !layers(&dxf).contains_key(""),
        "no entity may be left without a layer name"
    );
}

#[test]
fn the_r2018_dxf_layers_and_block_names_match_the_dxf_text() {
    let dxf = uncad::parse(DXF).expect("the corpus DXF parses");

    // Derived independently of this crate, by walking example_2018.dxf's own
    // group codes: for every record in its ENTITIES section (skipping the
    // VERTEX and SEQEND sub-records, which LibreDWG folds into their owning
    // POLYLINE/INSERT), the group 8 value. That walk yields
    // {"Tavolo 3": 57, "Tavolo 2": 7, "0": 6, "*ADSK_SYSTEM_LIGHTS": 1} over
    // 71 records. The seventh layer-`0` entity is the second paper-space
    // VIEWPORT: the ENTITIES section spells out one VIEWPORT record, and
    // LibreDWG materialises the layout's own viewport alongside it (the DWG
    // decoder produces the same pair, which is why both formats report 2).
    let expected: BTreeMap<String, usize> = [
        ("*ADSK_SYSTEM_LIGHTS", 1),
        ("0", 7),
        ("Tavolo 2", 7),
        ("Tavolo 3", 57),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_string(), count))
    .collect();
    assert_eq!(layers(&dxf), expected);

    // Same walk, group 2 of each INSERT record in the ENTITIES section.
    let expected: BTreeMap<String, usize> = [("CIRKLO_PUNKTOJ", 8), ("bloko", 2)]
        .into_iter()
        .map(|(name, count)| (name.to_string(), count))
        .collect();
    assert_eq!(block_names(&dxf), expected);

    // And the same walk over ENTITIES *and* BLOCKS: 34 records carry group
    // 8 "Defpoints" (all of them inside the anonymous `*D` dimension
    // blocks) and exactly one record carries group 60 = 1 -- a LINE that is
    // itself one of those 34. `hidden_reason` tests the invisible flag
    // before the layer name, so that one counts as `invisible` and 33
    // remain `defpoints`. No layer in this drawing is off, frozen or
    // non-plotting, so those three rules contribute nothing; what the
    // numbers below depend on is the layer *name* resolving at all, which
    // is the regression (before the fix the map was empty).
    let expected: BTreeMap<&'static str, usize> =
        [("defpoints", 33), ("invisible", 1)].into_iter().collect();
    assert_eq!(hidden(&dxf), expected);
}

#[test]
fn every_r2007_plus_corpus_dxf_resolves_its_entity_layers() {
    // The same failure hit every R2007+ DXF in the corpus, not just the one
    // above; sample_2007/sample_2010 came back as {"": 4, "0": 2}. An empty
    // layer name means the entity's layer handle did not resolve.
    for name in [
        "example_2007.dxf",
        "example_2010.dxf",
        "example_2013.dxf",
        "example_2018.dxf",
        "sample_2007.dxf",
        "sample_2010.dxf",
    ] {
        let path = format!(
            "{}/../../lib/libredwg/test/test-data/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let db = uncad::parse(&path).unwrap_or_else(|e| panic!("{name} parses: {e}"));
        assert!(!db.entities.is_empty(), "{name} has entities");
        assert!(
            !layers(&db).contains_key(""),
            "{name}: every entity must carry a layer name, got {:?}",
            layers(&db)
        );
    }
}
