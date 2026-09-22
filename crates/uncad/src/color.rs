//! AutoCAD color resolution: the ACI palette, BYLAYER/BYBLOCK precedence, and
//! the white-background normalization. Pure functions, kept separate from the
//! renderer's string building.
//!
//! Two of the quirks here (a layer's color reporting as white, and
//! white-on-white invisibility) were real bugs caught only by comparing
//! rendered output against AutoCAD itself -- see `docs/CAVEATS.md`.

use crate::tables::Tables;

pub const DEFAULT_COLOR: &str = "#000000";

/// The AutoCAD Color Index palette: packed 24-bit RGB, index 0-256, in
/// hex so each entry can be read off against a published ACI chart.
///
/// Autodesk's own historical choices, *not* derivable from a formula: the
/// grey ramp at 250-254 is `333333 505050 696969 828282 BEBEBE`, which no
/// interpolation produces. Taken verbatim from `rgb_palette[256]` in
/// `crates/libredwg-sys/vendor/libredwg/src/dwg.c` -- the table this crate
/// already ships and the one LibreDWG's own `dwg_rgb_palette_index()`
/// answers from, so a DXF colour LibreDWG resolved from an index and one
/// resolved here agree. Until 0.3.0 this table was a linear ramp between
/// the pure hues and differed from AutoCAD at 222 of the 255 real indices
/// (ACI 8 `808080` for `414141`, 9 `C0C0C0` for `808080`, 254 `D6D6D6`
/// for `BEBEBE`, ...).
///
/// Index 256 is BYLAYER and index 0 BYBLOCK; both are resolved by
/// [`resolve_color`] before the table is ever indexed, and the 0/256
/// entries here are only what a direct [`aci_to_hex`] call gets.
#[rustfmt::skip]
pub const ACI_PALETTE: [u32; 257] = [
    0x000000, 0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xffffff, // 0
    0x414141, 0x808080, 0xff0000, 0xffaaaa, 0xbd0000, 0xbd7e7e, 0x810000, 0x815656, // 8
    0x680000, 0x684545, 0x4f0000, 0x4f3535, 0xff3f00, 0xffbfaa, 0xbd2e00, 0xbd8d7e, // 16
    0x811f00, 0x816056, 0x681900, 0x684e45, 0x4f1300, 0x4f3b35, 0xff7f00, 0xffd4aa, // 24
    0xbd5e00, 0xbd9d7e, 0x814000, 0x816b56, 0x683400, 0x685645, 0x4f2700, 0x4f4235, // 32
    0xffbf00, 0xffeaaa, 0xbd8d00, 0xbdad7e, 0x816000, 0x817656, 0x684e00, 0x685f45, // 40
    0x4f3b00, 0x4f4935, 0xffff00, 0xffffaa, 0xbdbd00, 0xbdbd7e, 0x818100, 0x818156, // 48
    0x686800, 0x686845, 0x4f4f00, 0x4f4f35, 0xbfff00, 0xeaffaa, 0x8dbd00, 0xadbd7e, // 56
    0x608100, 0x768156, 0x4e6800, 0x5f6845, 0x3b4f00, 0x494f35, 0x7fff00, 0xd4ffaa, // 64
    0x5ebd00, 0x9dbd7e, 0x408100, 0x6b8156, 0x346800, 0x566845, 0x274f00, 0x424f35, // 72
    0x3fff00, 0xbfffaa, 0x2ebd00, 0x8dbd7e, 0x1f8100, 0x608156, 0x196800, 0x4e6845, // 80
    0x134f00, 0x3b4f35, 0x00ff00, 0xaaffaa, 0x00bd00, 0x7ebd7e, 0x008100, 0x568156, // 88
    0x006800, 0x456845, 0x004f00, 0x354f35, 0x00ff3f, 0xaaffbf, 0x00bd2e, 0x7ebd8d, // 96
    0x00811f, 0x568160, 0x006819, 0x45684e, 0x004f13, 0x354f3b, 0x00ff7f, 0xaaffd4, // 104
    0x00bd5e, 0x7ebd9d, 0x008140, 0x56816b, 0x006834, 0x456856, 0x004f27, 0x354f42, // 112
    0x00ffbf, 0xaaffea, 0x00bd8d, 0x7ebdad, 0x008160, 0x568176, 0x00684e, 0x45685f, // 120
    0x004f3b, 0x354f49, 0x00ffff, 0xaaffff, 0x00bdbd, 0x7ebdbd, 0x008181, 0x568181, // 128
    0x006868, 0x456868, 0x004f4f, 0x354f4f, 0x00bfff, 0xaaeaff, 0x008dbd, 0x7eadbd, // 136
    0x006081, 0x567681, 0x004e68, 0x455f68, 0x003b4f, 0x35494f, 0x007fff, 0xaad4ff, // 144
    0x005ebd, 0x7e9dbd, 0x004081, 0x566b81, 0x003468, 0x455668, 0x00274f, 0x35424f, // 152
    0x003fff, 0xaabfff, 0x002ebd, 0x7e8dbd, 0x001f81, 0x566081, 0x001968, 0x454e68, // 160
    0x00134f, 0x353b4f, 0x0000ff, 0xaaaaff, 0x0000bd, 0x7e7ebd, 0x000081, 0x565681, // 168
    0x000068, 0x454568, 0x00004f, 0x35354f, 0x3f00ff, 0xbfaaff, 0x2e00bd, 0x8d7ebd, // 176
    0x1f0081, 0x605681, 0x190068, 0x4e4568, 0x13004f, 0x3b354f, 0x7f00ff, 0xd4aaff, // 184
    0x5e00bd, 0x9d7ebd, 0x400081, 0x6b5681, 0x340068, 0x564568, 0x27004f, 0x42354f, // 192
    0xbf00ff, 0xeaaaff, 0x8d00bd, 0xad7ebd, 0x600081, 0x765681, 0x4e0068, 0x5f4568, // 200
    0x3b004f, 0x49354f, 0xff00ff, 0xffaaff, 0xbd00bd, 0xbd7ebd, 0x810081, 0x815681, // 208
    0x680068, 0x684568, 0x4f004f, 0x4f354f, 0xff00bf, 0xffaaea, 0xbd008d, 0xbd7ead, // 216
    0x810060, 0x815676, 0x68004e, 0x68455f, 0x4f003b, 0x4f3549, 0xff007f, 0xffaad4, // 224
    0xbd005e, 0xbd7e9d, 0x810040, 0x81566b, 0x680034, 0x684556, 0x4f0027, 0x4f3542, // 232
    0xff003f, 0xffaabf, 0xbd002e, 0xbd7e8d, 0x81001f, 0x815660, 0x680019, 0x68454e, // 240
    0x4f0013, 0x4f353b, 0x333333, 0x505050, 0x696969, 0x828282, 0xbebebe, 0xffffff, // 248
    0x000000, // 256 (BYLAYER: resolved before the table is indexed)
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

/// Deliberately ignores a layer's own truecolor field. On an older LibreDWG it
/// was a constant 0xFFFFFF placeholder on every real LAYER entry, so trusting
/// it rendered every BYLAYER entity black; a newer LibreDWG reports something
/// different (see [`crate::tables::resolve_layer_color_index`]), but the
/// conclusion holds either way -- `color_index` is the only trustworthy field,
/// and it is corrected before it ever reaches [`crate::tables::LayerRecord`].
pub fn layer_color_hex(tables: &Tables, layer_name: &str) -> Option<String> {
    let layer = tables.layers.get(layer_name)?;
    aci_to_hex(layer.color_index.unsigned_abs())
}

/// AutoCAD's layer 0, the one name with a meaning inside a block: geometry
/// created on layer 0 in a block *definition* is placed on the layer of the
/// block *reference* when the block is inserted, and resolves its BYLAYER
/// properties against that layer. Every other layer name inside a block is
/// used as stored.
pub const LAYER_ZERO: &str = "0";

/// The layer an entity is effectively on. `reference_layer` is the layer of
/// the enclosing block reference -- `None` at the top level, where there is
/// none -- and it applies only to an entity on [`LAYER_ZERO`].
///
/// Deliberately resolved here, at render/report time, rather than baked into
/// the model by `convert.rs`: one block definition is placed by many INSERTs
/// on many layers, so "the layer of this entity" only has an answer per
/// reference. [`crate::model::EntityCommon::layer`] keeps what the file
/// stores, and this is the one place that says what that means in context.
pub fn effective_layer<'a>(layer: &'a str, reference_layer: Option<&'a str>) -> &'a str {
    match reference_layer {
        Some(reference) if layer == LAYER_ZERO => reference,
        _ => layer,
    }
}

/// Resolves an entity's rendered color following AutoCAD's own precedence:
/// explicit 24-bit truecolor overrides everything; otherwise `color_index`
/// is either BYLAYER (256, resolved through the entity's own layer),
/// BYBLOCK (0, inherited from the enclosing INSERT/DIMENSION via
/// `inherited_color` -- pass [`DEFAULT_COLOR`] at the top level, matching
/// AutoCAD's documented BYBLOCK-with-no-enclosing-block fallback), or a
/// direct ACI palette index. The sign of `color_index` (negative = "layer
/// off") is deliberately ignored -- this doesn't track visibility, only color.
///
/// For an entity being drawn *inside* a block reference, call
/// [`resolve_color_in_block`] instead: this one has no reference layer to
/// resolve a layer-0 BYLAYER child against, and so answers with layer 0's own
/// colour, which is the one thing AutoCAD never does.
pub fn resolve_color(
    color_index: i16,
    true_color: Option<u32>,
    layer: &str,
    tables: &Tables,
    inherited_color: &str,
) -> String {
    resolve_color_in_block(
        color_index,
        true_color,
        layer,
        tables,
        inherited_color,
        None,
    )
}

/// [`resolve_color`] plus the layer-0-in-a-block rule: a BYLAYER entity on
/// layer 0 inside a block reference resolves against `reference_layer` (see
/// [`effective_layer`]), which is how the window, door and fixture blocks
/// every CAD office draws on layer 0 take their discipline's colour from the
/// layer they are inserted on. `reference_layer` is `None` at the top level.
pub fn resolve_color_in_block(
    color_index: i16,
    true_color: Option<u32>,
    layer: &str,
    tables: &Tables,
    inherited_color: &str,
    reference_layer: Option<&str>,
) -> String {
    if let Some(hex) = true_color_to_hex(true_color) {
        return hex;
    }
    match color_index {
        256 => layer_color_hex(tables, effective_layer(layer, reference_layer))
            .unwrap_or_else(|| DEFAULT_COLOR.to_string()),
        0 => inherited_color.to_string(),
        idx => aci_to_hex(idx.unsigned_abs()).unwrap_or_else(|| DEFAULT_COLOR.to_string()),
    }
}

/// Darkens a colour that would be hard to see on a white page, for the
/// rendered image: a hex colour whose luminance (`0.299 R + 0.587 G +
/// 0.114 B`, on 0..1) exceeds [`MAX_LUMINANCE_ON_WHITE`] has its channels
/// scaled down together until it meets it, so hue is kept. ACI 2 yellow
/// (`#ffff00`, luminance 0.89, contrast about 1.07:1 against white) becomes
/// `#828200`; cyan `#00a4a4`; pure white was already black (see
/// [`aci_to_hex`]). Colours below the threshold are returned unchanged.
///
/// Applied by the renderer only: [`resolve_color`] and the JSON keep the
/// file's own colours.
pub fn contrast_on_white(hex: &str) -> String {
    let Ok(packed) = u32::from_str_radix(hex.trim_start_matches('#'), 16) else {
        return hex.to_string();
    };
    let (r, g, b) = (
        f64::from((packed >> 16) & 0xff),
        f64::from((packed >> 8) & 0xff),
        f64::from(packed & 0xff),
    );
    let luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
    if luminance <= MAX_LUMINANCE_ON_WHITE {
        return hex.to_string();
    }
    let factor = MAX_LUMINANCE_ON_WHITE / luminance;
    let scale = |c: f64| -> u32 { (c * factor).round().clamp(0.0, 255.0) as u32 };
    format!("#{:02x}{:02x}{:02x}", scale(r), scale(g), scale(b))
}

/// The brightest a rendered colour may be on the white background, as
/// luminance on 0..1. 0.45 keeps ACI yellow/cyan/green readable while leaving
/// every mid and dark colour untouched.
pub const MAX_LUMINANCE_ON_WHITE: f64 = 0.45;

/// Blends a hex color toward white by `tint` (0.0 = unchanged, 1.0 = white),
/// clamped to `[0, 1]`. Approximates a single-color HATCH gradient's second
/// stop -- unverified, like the rest of [`crate::model::HatchGradient`].
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
    use std::collections::BTreeMap;

    fn tables_with(name: &str, color_index: i16) -> Tables {
        let mut layers = BTreeMap::new();
        layers.insert(
            name.to_string(),
            LayerRecord {
                name: name.to_string(),
                color_index,
                ..LayerRecord::default()
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

    /// A spread of indices read straight off `rgb_palette[256]` in
    /// `crates/libredwg-sys/vendor/libredwg/src/dwg.c` -- the AutoCAD Color
    /// Index table this crate vendors, and the one LibreDWG's own
    /// `dwg_rgb_palette_index()` answers from. Each expectation below is the
    /// `{ 0xRR, 0xGG, 0xBB }` triple at that index in that file, transcribed
    /// by hand rather than computed from this table: the pure hues (1-6), the
    /// two greys AutoCAD keeps distinct at 8/9, four shades a formula gets
    /// wrong (11, 12, 22, 152), and the whole 250-254 grey ramp, which is the
    /// proof the palette is not interpolated.
    #[test]
    fn aci_palette_matches_the_vendored_autocad_table() {
        let expected: &[(u16, &str)] = &[
            (1, "#ff0000"),
            (2, "#ffff00"),
            (3, "#00ff00"),
            (4, "#00ffff"),
            (5, "#0000ff"),
            (6, "#ff00ff"),
            (8, "#414141"),
            (9, "#808080"),
            (11, "#ffaaaa"),
            (12, "#bd0000"),
            (22, "#bd2e00"),
            (152, "#005ebd"),
            (250, "#333333"),
            (251, "#505050"),
            (252, "#696969"),
            (253, "#828282"),
            (254, "#bebebe"),
        ];
        for &(index, expected_hex) in expected {
            assert_eq!(
                aci_to_hex(index).as_deref(),
                Some(expected_hex),
                "ACI {index}"
            );
        }
        // 7 and 255 are both pure white in the table; the white-background
        // rule is what turns them black (see `normalize_hex_for_white_bg`).
        assert_eq!(ACI_PALETTE[7], 0xff_ffff);
        assert_eq!(ACI_PALETTE[255], 0xff_ffff);
        // The grey ramp is not a linear interpolation. The pre-0.3.0 table
        // read 333333 5B5B5B 848484 ADADAD D6D6D6 -- an exact ramp from 0x33
        // to 0xFF -- and only its first entry happened to be right.
        for (i, step) in (250..=254usize).zip([0x33u32, 0x5b, 0x84, 0xad, 0xd6]) {
            let interpolated = (step << 16) | (step << 8) | step;
            assert_eq!(
                i == 250,
                ACI_PALETTE[i] == interpolated,
                "ACI {i} vs the old linear ramp"
            );
        }
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
    fn contrast_on_white_darkens_yellow_and_cyan_but_not_red_or_black() {
        assert_eq!(contrast_on_white("#ffff00"), "#828200");
        assert_eq!(contrast_on_white("#00ffff"), "#00a4a4");
        assert_eq!(
            contrast_on_white("#ff0000"),
            "#ff0000",
            "red is 0.30: unchanged"
        );
        assert_eq!(contrast_on_white("#000000"), "#000000");
        assert_eq!(contrast_on_white("#123456"), "#123456");
    }

    #[test]
    fn contrast_on_white_keeps_hue_and_tolerates_garbage() {
        // Light grey darkens to the threshold, staying grey.
        let grey = contrast_on_white("#e0e0e0");
        let packed = u32::from_str_radix(&grey[1..], 16).unwrap();
        let (r, g, b) = ((packed >> 16) & 0xff, (packed >> 8) & 0xff, packed & 0xff);
        assert_eq!((r, g), (g, b));
        assert!(r < 0xe0);
        assert_eq!(contrast_on_white("not-a-colour"), "not-a-colour");
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
