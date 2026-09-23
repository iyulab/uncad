//! A layer's state: off, frozen, locked, plotted, its lineweight and the
//! linetype it names. Two sources: the project's `hidden_layers_r2000.dxf`
//! (one layer per state, the group codes in `tests/fixtures/README.md`) and
//! the corpus's `example_2000.dwg` with its text twin, whose `Defpoints`
//! layer is non-plotting and whose `ADSK_SYSTEM_LIGHTS` layer is frozen and
//! locked (DXF group 70 = 5 in the twin).

use uncad::model::Ref;
use uncad::tables::LayerRecord;

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

/// `(off, frozen, locked)`.
fn state(layer: &LayerRecord) -> (bool, bool, bool) {
    (layer.off, layer.frozen, layer.locked)
}

#[test]
fn the_fixture_layers_carry_the_state_their_groups_state() {
    let db = uncad::parse(HIDDEN).expect("fixture must parse");
    let l = &db.tables.layers;
    assert_eq!(l.len(), 7, "{:?}", l.keys().collect::<Vec<_>>());
    for name in ["0", "VISIBLE", "NOPLOT", "Defpoints"] {
        assert_eq!(state(&l[name]), (false, false, false), "{name}");
    }
    // Off is a negative colour in a DXF; the index keeps its sign.
    assert_eq!(state(&l["OFF"]), (true, false, false));
    assert_eq!(l["OFF"].color_index, -3);
    // Group 70 bit 1; bit 4. The importer read both into its own bit
    // layout, where they would mean "off" and "frozen in new viewports".
    assert_eq!(state(&l["FROZEN"]), (false, true, false));
    assert_eq!(state(&l["LOCKED"]), (false, false, true));
    // The importer leaves an absent 290 at 0, so a stated 0 cannot be told
    // from silence: not plotting is not something this DXF can be read to
    // say, and every plot flag is unknown rather than a guess.
    for layer in l.values() {
        assert_eq!(layer.plot, None, "{}", layer.name);
        assert_eq!(layer.lineweight, None, "{}: no 370", layer.name);
        assert_eq!(
            layer.linetype,
            Ref::Resolved("Continuous".to_string()),
            "{}",
            layer.name
        );
    }
}

#[test]
fn autocad_written_layer_state_reads_the_same_from_dwg_and_dxf() {
    for (path, from_dwg) in [(EXAMPLE_2000_DWG, true), (EXAMPLE_2000_DXF, false)] {
        let db = uncad::parse(path).expect("corpus file must parse");
        let l = &db.tables.layers;
        assert_eq!(state(&l["0"]), (false, false, false), "{path}");
        assert_eq!(
            state(&l["ADSK_SYSTEM_LIGHTS"]),
            (false, true, true),
            "{path}"
        );
        for layer in l.values() {
            assert_eq!(
                layer.linetype,
                Ref::Resolved("Continuous".to_string()),
                "{path}: {}",
                layer.name
            );
            // 370 = -3 on every layer of the twin: the default weight.
            assert_eq!(layer.lineweight, Some(-3), "{path}: {}", layer.name);
        }
        // The R2000 DWG states the plot flag of every layer; the DXF states
        // only Defpoints' 290 = 0, which its importer cannot tell from an
        // absent group.
        let plot: Vec<(&str, Option<bool>)> =
            l.values().map(|l| (l.name.as_str(), l.plot)).collect();
        let expected: Vec<(&str, Option<bool>)> = if from_dwg {
            vec![
                ("0", Some(true)),
                ("ADSK_SYSTEM_LIGHTS", Some(true)),
                ("Defpoints", Some(false)),
                ("Tavolo 2", Some(true)),
                ("Tavolo 3", Some(true)),
            ]
        } else {
            l.keys().map(|name| (name.as_str(), None)).collect()
        };
        assert_eq!(plot, expected, "{path}");
    }
}

#[test]
fn a_drawing_older_than_r2000_states_no_plot_flag_and_no_lineweight() {
    let db = uncad::parse(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/example_r14.dwg"
    ))
    .expect("corpus file must parse");
    assert!(!db.tables.layers.is_empty());
    for layer in db.tables.layers.values() {
        assert_eq!(
            (layer.plot, layer.lineweight),
            (None, None),
            "{}",
            layer.name
        );
    }
}
