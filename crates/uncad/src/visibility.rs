//! What the drawing shows. A DWG carries entities nobody sees on a plot:
//! layers switched off, frozen or marked non-plotting, AutoCAD's own
//! `DEFPOINTS` layer, and entities with their own invisible flag.
//! [`hidden_reason`] is the one rule the renderer and the exports share,
//! and [`lineweight_mm`] decodes the lineweight codes both layers and
//! entities store.

use serde::{Deserialize, Serialize};

use crate::model::EntityCommon;
use crate::tables::Tables;

/// Why an entity is not shown. Checked in this order, so an invisible
/// entity on a frozen layer reports `Invisible`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hidden {
    /// The entity's own invisible flag (DXF 60).
    Invisible,
    /// On the `DEFPOINTS` layer, which AutoCAD never plots whatever its
    /// flags say (dimension definition points live there).
    Defpoints,
    /// The layer is switched off.
    LayerOff,
    /// The layer is frozen.
    LayerFrozen,
    /// The layer is marked "do not plot". Only R2000+ DWG files state this
    /// reliably; see [`crate::tables::LayerRecord::plot`].
    LayerNoPlot,
}

impl Hidden {
    /// The `snake_case` name the JSON uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Hidden::Invisible => "invisible",
            Hidden::Defpoints => "defpoints",
            Hidden::LayerOff => "layer_off",
            Hidden::LayerFrozen => "layer_frozen",
            Hidden::LayerNoPlot => "layer_no_plot",
        }
    }
}

/// Why `common`'s entity is hidden, or `None` when the drawing shows it. A
/// layer the tables do not know is treated as visible.
///
/// Block contents follow the same rule through their own layers; an entity
/// on layer `0` inside a block is shown when layer `0` is, which matches
/// AutoCAD as long as the INSERT itself is visible (the renderer never
/// reaches the contents of a hidden INSERT).
pub fn hidden_reason(common: &EntityCommon, tables: &Tables) -> Option<Hidden> {
    if common.invisible {
        return Some(Hidden::Invisible);
    }
    if common.layer.eq_ignore_ascii_case("DEFPOINTS") {
        return Some(Hidden::Defpoints);
    }
    let layer = tables.layers.get(&common.layer)?;
    if !layer.on {
        Some(Hidden::LayerOff)
    } else if layer.frozen {
        Some(Hidden::LayerFrozen)
    } else if !layer.plot {
        Some(Hidden::LayerNoPlot)
    } else {
        None
    }
}

/// AutoCAD's lineweight enum in hundredths of a millimetre, indexed by the
/// code LibreDWG stores in `linewt` (`lweights[]` in its `dwg.c`).
const LINEWEIGHTS_100TH_MM: [u16; 24] = [
    0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200,
    211,
];

/// The lineweight in millimetres for LibreDWG's `linewt` code: codes 0
/// to 23 are the standard weights 0.00 to 2.11 mm; 29 is BYLAYER, 30
/// BYBLOCK, 31 the default, and 24 to 28 are unused. `None` for everything
/// that is not a number.
pub fn lineweight_mm(code: u8) -> Option<f64> {
    LINEWEIGHTS_100TH_MM
        .get(usize::from(code))
        .map(|w| f64::from(*w) / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::LayerRecord;
    use std::collections::BTreeMap;

    fn tables() -> Tables {
        let mut layers = BTreeMap::new();
        for (name, on, frozen, plot) in [
            ("VISIBLE", true, false, true),
            ("OFF", false, false, true),
            ("FROZEN", true, true, true),
            ("NOPLOT", true, false, false),
            ("Defpoints", true, false, false),
            ("OFF-AND-FROZEN", false, true, true),
        ] {
            layers.insert(
                name.to_string(),
                LayerRecord {
                    name: name.to_string(),
                    on,
                    frozen,
                    plot,
                    ..LayerRecord::default()
                },
            );
        }
        Tables {
            layers,
            ..Tables::default()
        }
    }

    fn on(layer: &str) -> EntityCommon {
        EntityCommon {
            layer: layer.to_string(),
            ..EntityCommon::default()
        }
    }

    #[test]
    fn each_reason_in_priority_order() {
        let t = tables();
        assert_eq!(hidden_reason(&on("VISIBLE"), &t), None);
        assert_eq!(hidden_reason(&on("OFF"), &t), Some(Hidden::LayerOff));
        assert_eq!(hidden_reason(&on("FROZEN"), &t), Some(Hidden::LayerFrozen));
        assert_eq!(hidden_reason(&on("NOPLOT"), &t), Some(Hidden::LayerNoPlot));
        assert_eq!(
            hidden_reason(&on("OFF-AND-FROZEN"), &t),
            Some(Hidden::LayerOff)
        );
        // DEFPOINTS by name, whatever the record says, in any case.
        assert_eq!(hidden_reason(&on("Defpoints"), &t), Some(Hidden::Defpoints));
        assert_eq!(hidden_reason(&on("DEFPOINTS"), &t), Some(Hidden::Defpoints));
        // The entity's own flag wins over its layer.
        let mut invisible = on("FROZEN");
        invisible.invisible = true;
        assert_eq!(hidden_reason(&invisible, &t), Some(Hidden::Invisible));
        // An unknown layer (no TABLES section, say) is shown.
        assert_eq!(hidden_reason(&on("NOT-A-LAYER"), &t), None);
        assert_eq!(hidden_reason(&on(""), &t), None);
    }

    #[test]
    fn lineweight_codes() {
        assert_eq!(lineweight_mm(0), Some(0.0));
        assert_eq!(lineweight_mm(11), Some(0.5));
        assert_eq!(lineweight_mm(23), Some(2.11));
        for code in [24, 28, 29, 30, 31, 255] {
            assert_eq!(lineweight_mm(code), None, "code {code}");
        }
    }

    #[test]
    fn reason_names_match_the_json() {
        let json = serde_json::to_string(&Hidden::LayerNoPlot).unwrap();
        assert_eq!(json, "\"layer_no_plot\"");
        for reason in [
            Hidden::Invisible,
            Hidden::Defpoints,
            Hidden::LayerOff,
            Hidden::LayerFrozen,
            Hidden::LayerNoPlot,
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, format!("\"{}\"", reason.as_str()));
        }
    }
}
