//! Hidden entities (docs/VLM_EXPORT_DESIGN.md, P5): layer state, the
//! entity's own flags, and what the renderer does with them. Sources: the
//! project's `hidden_layers_r2000.dxf` (one LINE per layer state, ground
//! truth in tests/fixtures/README.md) and the corpus's `example_2000.dwg`,
//! whose `Defpoints` layer is non-plotting and whose `ADSK_SYSTEM_LIGHTS`
//! layer is frozen and locked (DXF group 70 = 5 in the text twin).

use uncad::visibility::{hidden_reason, Hidden};
use uncad::Entity;

const HIDDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hidden_layers_r2000.dxf"
);
const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);

#[test]
fn the_fixture_layers_carry_their_state() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let l = &db.tables.layers;
    assert_eq!(l.len(), 7, "{:?}", l.keys().collect::<Vec<_>>());
    let visible = &l["VISIBLE"];
    assert!(visible.on && !visible.frozen && !visible.locked && visible.plot);
    assert_eq!(visible.linetype, "Continuous");
    assert_eq!(visible.lineweight_mm, None, "no 370 in a DXF: unknown");
    // Off = negative colour in a DXF; the raw index keeps its sign.
    assert!(!l["OFF"].on);
    assert_eq!(l["OFF"].color_index, -3);
    assert!(l["FROZEN"].frozen && l["FROZEN"].on);
    assert!(l["LOCKED"].locked && l["LOCKED"].on && !l["LOCKED"].frozen);
    // DXF 290 cannot be told apart from an absent one through LibreDWG, so
    // a DXF layer always reads as plotting (docs/CAVEATS.md); DEFPOINTS is
    // hidden by name instead.
    assert!(l["NOPLOT"].plot);
    assert!(l["Defpoints"].plot);
}

#[test]
fn the_fixture_entities_report_why_they_are_hidden() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let reasons: Vec<(String, Option<Hidden>)> = db
        .entities
        .iter()
        .map(|e| {
            (
                e.common().layer.clone(),
                hidden_reason(e.common(), &db.tables),
            )
        })
        .collect();
    assert_eq!(
        reasons,
        [
            ("0".to_string(), None),
            ("VISIBLE".to_string(), None),
            ("OFF".to_string(), Some(Hidden::LayerOff)),
            ("FROZEN".to_string(), Some(Hidden::LayerFrozen)),
            ("NOPLOT".to_string(), None),
            ("Defpoints".to_string(), Some(Hidden::Defpoints)),
            ("LOCKED".to_string(), None),
            ("VISIBLE".to_string(), Some(Hidden::Invisible)),
            ("VISIBLE".to_string(), None),
        ]
    );

    // The entity's own fields: DXF 60, 370, 6 and 48.
    let invisible = db.entities[7].common();
    assert!(invisible.invisible);
    assert_eq!(invisible.lineweight_mm, None);
    assert_eq!(invisible.linetype, "BYLAYER");
    assert_eq!(invisible.ltype_scale, 1.0);
    let dashed = db.entities[8].common();
    assert!(!dashed.invisible);
    assert_eq!(dashed.lineweight_mm, Some(0.5));
    assert_eq!(dashed.linetype, "DASHED");
    assert_eq!(dashed.ltype_scale, 2.0);
}

#[test]
fn the_renderer_skips_hidden_entities_unless_asked_to_fade_them_in() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let shown = db.to_svg(uncad::ToSvgOptions {
        crop: uncad::CropMode::Raw,
        padding: Some(5.0),
        ..Default::default()
    });
    assert_eq!(shown.svg.matches("<line ").count(), 5, "{}", shown.svg);
    assert_eq!(shown.hidden, 4);
    assert!(!shown.svg.contains("opacity"));
    // Hidden entities do not stretch the viewBox: the OFF line at y = 20
    // and the invisible one at y = 100 are inside anyway, but the world
    // bounds are those of the shown lines (y 0..110, padding 5).
    assert!(
        (shown.view_box.y + 115.0).abs() < 1e-9,
        "{:?}",
        shown.view_box
    );

    let faded = db.to_svg(uncad::ToSvgOptions {
        include_hidden: true,
        crop: uncad::CropMode::Raw,
        padding: Some(5.0),
        ..Default::default()
    });
    assert_eq!(faded.svg.matches("<line ").count(), 9, "{}", faded.svg);
    assert_eq!(faded.svg.matches("<g opacity=\"0.5\">").count(), 4);
    assert_eq!(faded.hidden, 4);

    let png = db
        .to_png(uncad::ToPngOptions::default())
        .expect("rasterizes");
    assert_eq!(png.hidden, 4);
}

#[test]
fn autocad_written_layer_state_reads_the_same_from_dwg_and_dxf() {
    for (path, from_dwg) in [(EXAMPLE_2000_DWG, true), (EXAMPLE_2000_DXF, false)] {
        let db = uncad::parse(path).expect("corpus file must parse");
        let l = &db.tables.layers;
        assert!(l["0"].on && !l["0"].frozen && l["0"].plot, "{path}");
        assert_eq!(l["0"].linetype, "Continuous", "{path}");
        assert_eq!(l["0"].lineweight_mm, None, "370 = -3 is the default");
        let lights = &l["ADSK_SYSTEM_LIGHTS"];
        assert!(
            lights.frozen && lights.locked && lights.on,
            "{path}: {lights:?}"
        );
        // The R2000 DWG states the plot flag; the DXF cannot.
        assert_eq!(l["Defpoints"].plot, !from_dwg, "{path}");

        let on_lights: Vec<&Entity> = db
            .entities
            .iter()
            .filter(|e| e.common().layer == "ADSK_SYSTEM_LIGHTS")
            .collect();
        assert_eq!(on_lights.len(), 1, "{path}");
        assert_eq!(
            hidden_reason(on_lights[0].common(), &db.tables),
            Some(Hidden::LayerFrozen),
            "{path}"
        );
        let hidden = db
            .entities
            .iter()
            .filter(|e| hidden_reason(e.common(), &db.tables).is_some())
            .count();
        assert_eq!(hidden, 1, "{path}: only the light on the frozen layer");
    }
}

#[test]
fn layer_state_and_entity_flags_survive_the_json_round_trip() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializes");
    assert!(json.contains("\"frozen\":true"), "{json}");
    assert!(json.contains("\"lineweight_mm\":0.5"), "{json}");
    assert!(json.contains("\"linetype\":\"DASHED\""), "{json}");
    let back: uncad::CadDatabase = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.tables.layers, db.tables.layers);
    assert_eq!(back.entities[8].common(), db.entities[8].common());

    // 0.2.0 JSON, without any of the new fields, still loads with every
    // layer on and plotting and every entity visible.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for (_, layer) in value["tables"]["layers"].as_object_mut().unwrap() {
        let layer = layer.as_object_mut().unwrap();
        for key in [
            "on",
            "frozen",
            "locked",
            "plot",
            "lineweight_mm",
            "linetype",
        ] {
            assert!(layer.remove(key).is_some(), "{key}");
        }
    }
    for entity in value["entities"].as_array_mut().unwrap() {
        let common = entity["common"].as_object_mut().unwrap();
        for key in ["invisible", "lineweight_mm", "linetype", "ltype_scale"] {
            assert!(common.remove(key).is_some(), "{key}");
        }
    }
    let old: uncad::CadDatabase = serde_json::from_value(value).expect("0.2.0 shape loads");
    assert!(old
        .tables
        .layers
        .values()
        .all(|l| l.on && !l.frozen && l.plot));
    assert!(old
        .entities
        .iter()
        .all(|e| !e.common().invisible && e.common().ltype_scale == 1.0));
}
