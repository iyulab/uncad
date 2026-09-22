//! AutoCAD color resolution for rendering: BYLAYER/BYBLOCK precedence, the
//! true-color override, and the white-background normalization, on top of the
//! ACI palette the model carries. Pure functions, kept separate from the
//! renderer's string building.
//!
//! Two of the quirks here (a layer's color reporting as white, and
//! white-on-white invisibility) were real bugs caught only by comparing
//! rendered output against AutoCAD itself -- see `docs/CAVEATS.md`.

use uncad_model::color::aci_to_rgb;
use uncad_model::tables::Tables;

pub const DEFAULT_COLOR: &str = "#000000";

/// ACI index 7 (0xFFFFFF, "white/black") is AutoCAD's own
/// auto-invert-by-background special case; since this renderer always
/// targets a plain white background, any resolved pure-white color is
/// flipped to black -- otherwise it's silently invisible white-on-white.
/// Applied at every color-producing path (not just literal index 7),
/// because LAYER.color reports 0xFFFFFF unconditionally too (see
/// [`layer_color_hex`]).
fn normalize_hex_for_white_bg(hex: String) -> String {
    if hex == "#ffffff" {
        "#000000".to_string()
    } else {
        hex
    }
}

fn hex(packed: u32) -> String {
    normalize_hex_for_white_bg(format!("#{:06x}", packed & 0xff_ffff))
}

pub fn aci_to_hex(index: u16) -> Option<String> {
    aci_to_rgb(index).map(hex)
}

pub fn true_color_to_hex(color: Option<u32>) -> Option<String> {
    color.map(hex)
}

/// Deliberately ignores a layer's own truecolor field. On an older LibreDWG it
/// was a constant 0xFFFFFF placeholder on every real LAYER entry, so trusting
/// it rendered every BYLAYER entity black; a newer LibreDWG reports something
/// different (see `resolve_layer_color_index` in `table_convert.rs`), but the
/// conclusion holds either way -- `color_index` is the only trustworthy field,
/// and it is corrected before it ever reaches the model's `LayerRecord`.
pub fn layer_color_hex(tables: &Tables, layer_name: &str) -> Option<String> {
    let layer = tables.layers.get(layer_name)?;
    aci_to_hex(layer.color_index.unsigned_abs())
}

/// Resolves an entity's rendered color following AutoCAD's own precedence:
/// explicit 24-bit truecolor overrides everything; otherwise `color_index`
/// is either BYLAYER (256, resolved through the entity's own layer),
/// BYBLOCK (0, inherited from the enclosing INSERT/DIMENSION via
/// `inherited_color` -- pass [`DEFAULT_COLOR`] at the top level, matching
/// AutoCAD's documented BYBLOCK-with-no-enclosing-block fallback), or a
/// direct ACI palette index. The sign of `color_index` (negative = "layer
/// off") is deliberately ignored -- this doesn't track visibility, only color.
pub fn resolve_color(
    color_index: i16,
    true_color: Option<u32>,
    layer: &str,
    tables: &Tables,
    inherited_color: &str,
) -> String {
    if let Some(hex) = true_color_to_hex(true_color) {
        return hex;
    }
    match color_index {
        256 => layer_color_hex(tables, layer).unwrap_or_else(|| DEFAULT_COLOR.to_string()),
        0 => inherited_color.to_string(),
        idx => aci_to_hex(idx.unsigned_abs()).unwrap_or_else(|| DEFAULT_COLOR.to_string()),
    }
}

/// Blends a hex color toward white by `tint` (0.0 = unchanged, 1.0 = white),
/// clamped to `[0, 1]`. Approximates a single-color HATCH gradient's second
/// stop -- unverified, like the rest of `HatchGradient`.
pub fn tint_toward_white(hex: &str, tint: f64) -> String {
    let t = tint.clamp(0.0, 1.0);
    let packed = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let blend = |shift: u32| -> u32 {
        let c = ((packed >> shift) & 0xff) as f64;
        (c + (255.0 - c) * t).round() as u32
    };
    format!("#{:02x}{:02x}{:02x}", blend(16), blend(8), blend(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use uncad_model::tables::LayerRecord;

    fn tables_with(name: &str, color_index: i16) -> Tables {
        let mut layers = BTreeMap::new();
        layers.insert(
            name.to_string(),
            LayerRecord {
                name: name.to_string(),
                color_index,
            },
        );
        Tables {
            layers,
            ..Default::default()
        }
    }

    #[test]
    fn aci_index_2_is_yellow() {
        assert_eq!(aci_to_hex(2).as_deref(), Some("#ffff00"));
    }

    #[test]
    fn aci_index_7_white_normalizes_to_black() {
        assert_eq!(aci_to_hex(7).as_deref(), Some("#000000"));
    }

    #[test]
    fn truecolor_overrides_colorindex() {
        let tables = tables_with("L", 2);
        let resolved = resolve_color(256, Some(0x00ff00), "L", &tables, DEFAULT_COLOR);
        assert_eq!(resolved, "#00ff00");
    }

    #[test]
    fn bylayer_resolves_through_layer_colorindex_not_layer_rgb() {
        // Regression test for the bug in docs/CAVEATS.md: a layer with color
        // index 2 (yellow) must resolve to yellow, never black, whatever
        // Dwg_Color.rgb the LAYER entry reports (LayerRecord does not even
        // carry it, by design).
        let tables = tables_with("Tavolo 1", 2);
        let resolved = resolve_color(256, None, "Tavolo 1", &tables, DEFAULT_COLOR);
        assert_eq!(resolved, "#ffff00");
    }

    #[test]
    fn bylayer_with_unknown_layer_falls_back_to_default() {
        let tables = Tables::default();
        let resolved = resolve_color(256, None, "nonexistent", &tables, DEFAULT_COLOR);
        assert_eq!(resolved, DEFAULT_COLOR);
    }

    #[test]
    fn byblock_inherits_from_context() {
        let tables = Tables::default();
        let resolved = resolve_color(0, None, "0", &tables, "#123456");
        assert_eq!(resolved, "#123456");
    }

    #[test]
    fn direct_aci_index_ignores_sign() {
        let tables = Tables::default();
        let positive = resolve_color(2, None, "0", &tables, DEFAULT_COLOR);
        let negative = resolve_color(-2, None, "0", &tables, DEFAULT_COLOR);
        assert_eq!(positive, "#ffff00");
        assert_eq!(
            negative, "#ffff00",
            "sign marks 'layer off', not a different color"
        );
    }

    #[test]
    fn tint_zero_leaves_color_unchanged() {
        assert_eq!(tint_toward_white("#123456", 0.0), "#123456");
    }

    #[test]
    fn tint_one_is_pure_white() {
        assert_eq!(tint_toward_white("#123456", 1.0), "#ffffff");
    }

    #[test]
    fn tint_half_blends_black_toward_mid_gray() {
        assert_eq!(tint_toward_white("#000000", 0.5), "#808080");
    }

    #[test]
    fn tint_out_of_range_is_clamped() {
        assert_eq!(tint_toward_white("#123456", -1.0), "#123456");
        assert_eq!(tint_toward_white("#123456", 2.0), "#ffffff");
    }
}
