//! An R2007+ DXF must resolve its entities' *handle-borne* names -- the
//! layer every entity points at and the block record every INSERT points at
//! -- exactly as the DWG of the same drawing does. An R2007+ DXF is UTF-8,
//! and a name longer than one character is the case a reader that took its
//! strings in the wrong width would get wrong.
//!
//! The corpus carries the same drawing in both formats, which makes the DWG
//! an independent reference for the DXF: two different readers, one
//! drawing. The absolute numbers below were derived a third way -- by
//! walking the DXF's own text -- and each assertion says how.

use std::collections::BTreeMap;

use uncad::model::{Entity, Ref};
use uncad::CadDatabase;

const DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dxf"
);
const DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2018.dwg"
);

/// A reference as one comparable string: the name, or its state.
fn name(r: &Ref<String>) -> String {
    match r {
        Ref::Resolved(name) => name.clone(),
        Ref::Absent => "<absent>".to_string(),
        Ref::Unresolved(handle) => format!("<unresolved {handle}>"),
    }
}

/// How many model- and paper-space entities sit on each layer.
fn layers(db: &CadDatabase) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for entity in &db.entities {
        *out.entry(name(&entity.common().layer)).or_insert(0) += 1;
    }
    out
}

/// How many INSERTs name each block record.
fn block_names(db: &CadDatabase) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for entity in &db.entities {
        if let Entity::Insert(insert) = entity {
            *out.entry(name(&insert.block_name)).or_insert(0) += 1;
        }
    }
    out
}

/// Every entity the drawing holds: model and paper space plus each *named*
/// block definition. `*Model_Space`/`*Paper_Space*` are skipped because
/// their contents are already in `db.entities`.
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

/// What a consumer decides an entity's visibility from, as far as the
/// parser states it: the entity's layer and its own invisible flag, counted
/// over every entity of the drawing. (Which of these hide an entity -- a
/// `Defpoints` layer, an invisible flag, a layer that is off or frozen -- is
/// the consumer's rule, not the reader's.)
fn layer_and_visibility(db: &CadDatabase) -> BTreeMap<(String, bool), usize> {
    let mut out = BTreeMap::new();
    for entity in all_entities(db) {
        let common = entity.common();
        *out.entry((name(&common.layer), common.invisible))
            .or_insert(0) += 1;
    }
    out
}

#[test]
fn an_r2018_dxf_resolves_the_same_layers_blocks_and_visibility_as_its_dwg_twin() {
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
        layer_and_visibility(&dxf),
        layer_and_visibility(&dwg),
        "DXF and DWG must state the same layer and invisible flag for every entity"
    );
    let resolved = |db: &CadDatabase| {
        db.entities
            .iter()
            .all(|e| matches!(e.common().layer, Ref::Resolved(_)))
    };
    assert!(resolved(&dxf), "no entity may be left without a layer name");
}

#[test]
fn the_r2018_dxf_layers_and_block_names_match_the_dxf_text() {
    let dxf = uncad::parse(DXF).expect("the corpus DXF parses");

    // Derived independently of this crate, by walking example_2018.dxf's own
    // group codes: for every record in its ENTITIES section (skipping the
    // VERTEX and SEQEND sub-records, which belong to their owning
    // POLYLINE/INSERT), the group 8 value. That walk yields
    // {"Tavolo 3": 57, "Tavolo 2": 7, "0": 6, "*ADSK_SYSTEM_LIGHTS": 1} over
    // 71 records. The seventh layer-`0` entity is the second paper-space
    // VIEWPORT: the ENTITIES section spells out one VIEWPORT record, and the
    // layout's own viewport is written in its paper-space block in BLOCKS
    // (the DWG decoder produces the same pair, which is why both formats
    // report 2).
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
    // itself one of those 34.
    let tally = layer_and_visibility(&dxf);
    let on_defpoints: usize = tally
        .iter()
        .filter(|((layer, _), _)| layer == "Defpoints")
        .map(|(_, n)| n)
        .sum();
    assert_eq!(on_defpoints, 34);
    let invisible: Vec<_> = tally
        .iter()
        .filter(|((_, invisible), _)| *invisible)
        .collect();
    assert_eq!(invisible, [(&("Defpoints".to_string(), true), &1)]);
}

#[test]
fn every_readable_r2007_plus_corpus_dxf_resolves_its_entity_layers() {
    // Every R2007+ DXF of the corpus, not just the one above. A layer that
    // is not resolved means the entity names a layer its table does not
    // declare.
    for name in [
        "2010/gh209_1.dxf",
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
            db.entities
                .iter()
                .all(|e| matches!(e.common().layer, Ref::Resolved(_))),
            "{name}: every entity must carry a layer name, got {:?}",
            layers(&db)
        );
    }
}
