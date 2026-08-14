//! AutoCAD color resolution, ported from `src/index.mjs`'s `resolveColor`/
//! `layerColorHex`/`aciToHex`/`trueColorToHex`/`normalizeHexForWhiteBg`.
//! Kept as pure functions here, separate from `svg.rs`'s string-building
//! code, matching how the JS source already separates this logic too.
//!
//! Every quirk below is preserved verbatim from the JS version, not
//! rediscovered -- see docs/CAVEATS.md for the two real bugs (LAYER.color
//! always reporting white, white-on-white invisibility) that were only
//! caught by comparing rendered output against real AutoCAD screenshots.

use crate::tables::Tables;

pub const DEFAULT_COLOR: &str = "#000000";

// Standard AutoCAD Color Index (ACI) palette -- packed 24-bit RGB integers,
// index 0-256. Ported verbatim from this project's own JS/WASM-era
// predecessor (`src/index.mjs`, itself ported from the mlightcad/
// libredwg-web fork's `bindings/javascript/src/svg/color.ts`, GPLv3+ --
// both retired since; see git history), rather than re-transcribed a second
// time -- these are Autodesk's own historical palette choices, not
// derivable from a formula, so every re-transcription is a chance to
// introduce a typo.
#[rustfmt::skip]
pub const ACI_PALETTE: [u32; 257] = [
    0, 16711680, 16776960, 65280, 65535, 255, 16711935, 16777215, 8421504,
    12632256, 16711680, 16744319, 13369344, 13395558, 10027008, 10046540, 8323072,
    8339263, 4980736, 4990502, 16727808, 16752511, 13382400, 13401958, 10036736,
    10051404, 8331008, 8343359, 4985600, 4992806, 16744192, 16760703, 13395456,
    13408614, 10046464, 10056268, 8339200, 8347455, 4990464, 4995366, 16760576,
    16768895, 13408512, 13415014, 10056192, 10061132, 8347392, 8351551, 4995328,
    4997670, 16776960, 16777087, 13421568, 13421670, 10000384, 10000460, 8355584,
    8355647, 5000192, 5000230, 12582656, 14679935, 10079232, 11717734, 7510016,
    8755276, 6258432, 7307071, 3755008, 4344870, 8388352, 12582783, 6736896,
    10079334, 5019648, 7510092, 4161280, 6258495, 2509824, 3755046, 4194048,
    10485631, 3394560, 8375398, 2529280, 6264908, 2064128, 5209919, 1264640,
    3099686, 65280, 8388479, 52224, 6736998, 38912, 5019724, 32512, 4161343,
    19456, 2509862, 65343, 8388511, 52275, 6737023, 38950, 5019743, 32543,
    4161359, 19475, 2509871, 65407, 8388543, 52326, 6737049, 38988, 5019762,
    32575, 4161375, 19494, 2509881, 65471, 8388575, 52377, 6737074, 39026,
    5019781, 32607, 4161391, 19513, 2509890, 65535, 8388607, 52428, 6737100,
    39064, 5019800, 32639, 4161407, 19532, 2509900, 49151, 8380415, 39372,
    6730444, 29336, 5014936, 24447, 4157311, 14668, 2507340, 32767, 8372223,
    26316, 6724044, 19608, 5010072, 16255, 4153215, 9804, 2505036, 16383, 8364031,
    13260, 6717388, 9880, 5005208, 8063, 4149119, 4940, 2502476, 255, 8355839,
    204, 6710988, 152, 5000344, 127, 4145023, 76, 2500172, 4129023, 10452991,
    3342540, 8349388, 2490520, 6245528, 2031743, 5193599, 1245260, 3089996,
    8323327, 12550143, 6684876, 10053324, 4980888, 7490712, 4128895, 6242175,
    2490444, 3745356, 12517631, 14647295, 10027212, 11691724, 7471256, 8735896,
    6226047, 7290751, 3735628, 4335180, 16711935, 16744447, 13369548, 13395660,
    9961624, 9981080, 8323199, 8339327, 4980812, 4990540, 16711871, 16744415,
    13369497, 13395634, 9961586, 9981061, 8323167, 8339311, 4980793, 4990530,
    16711807, 16744383, 13369446, 13395609, 9961548, 9981042, 8323135, 8339295,
    4980774, 4990521, 16711743, 16744351, 13369395, 13395583, 9961510, 9981023,
    8323103, 8339279, 4980755, 4990511, 3355443, 5987163, 8684676, 11382189,
    14079702, 16777215, 0,
];

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
    ACI_PALETTE.get(index as usize).map(|&packed| hex(packed))
}

pub fn true_color_to_hex(color: Option<u32>) -> Option<String> {
    color.map(hex)
}

/// Deliberately ignores the layer's own truecolor field -- see
/// [`crate::tables::Tables`]'s doc comment: on an older `lib/libredwg` it
/// was a constant 0xFFFFFF placeholder on every real-world LAYER table
/// entry, independent of the layer's actual color; trusting it made every
/// BYLAYER entity in every real professional DWG render black -- caught by
/// comparing against real AutoCAD screenshots, not synthetic test files. A
/// newer `lib/libredwg` reports something different (see
/// [`crate::tables::resolve_layer_color_index`]), but the conclusion is
/// the same either way: `color_index` is the only trustworthy field, now
/// corrected before it reaches `LayerRecord` rather than read here.
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

/// Blends a hex color toward white by `tint` (0.0 = unchanged, 1.0 = pure
/// white), clamped to `[0, 1]`. Approximates a single-color HATCH
/// gradient's second stop from `gradient_tint` -- see `render_model::HatchGradient`'s
/// doc comment for why this is an unverified approximation (no real file in
/// this project's `samples/` exercises a gradient-fill HATCH at all), not a
/// value read directly from any file the way most colors here are.
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
    use crate::tables::LayerRecord;
    use std::collections::HashMap;

    fn tables_with(name: &str, color_index: i16) -> Tables {
        let mut layers = HashMap::new();
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
        // Regression test for the exact bug documented in docs/CAVEATS.md: a layer with
        // colorIndex 2 (yellow) must resolve to yellow via layer_color_hex,
        // never black, regardless of whatever Dwg_Color.rgb the LAYER
        // table entry itself reports (which Tables/LayerRecord doesn't
        // even carry, by design -- see Tables's doc comment).
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
