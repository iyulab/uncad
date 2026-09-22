//! The one color computation the parser itself performs: the two hex stops
//! of a HATCH gradient, which the model carries as strings.
//!
//! Everything else about color -- BYLAYER/BYBLOCK resolution, the white
//! background flip for whole entities -- is the renderer crate's. These few
//! functions are the same as its palette-to-hex step, kept here so that this
//! crate does not depend on a renderer. That the model carries gradient stops
//! as rendered hex strings rather than as packed RGB is a known wrinkle of the
//! model's shape, not of this crate.

use uncad_model::color::aci_to_rgb;

pub const DEFAULT_COLOR: &str = "#000000";

/// ACI index 7 (`0xFFFFFF`, "white/black") is AutoCAD's own
/// auto-invert-by-background special case; for a white-background rendering
/// any resolved pure-white color is flipped to black, otherwise it is
/// invisible white-on-white. The renderer applies the same rule to every
/// other color it resolves, so a gradient stop follows it here.
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

/// Blends a hex color toward white by `tint` (0.0 = unchanged, 1.0 = white),
/// clamped to `[0, 1]`. The second stop of a single-color gradient.
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

    #[test]
    fn aci_index_2_is_yellow_and_7_normalizes_to_black() {
        assert_eq!(aci_to_hex(2).as_deref(), Some("#ffff00"));
        assert_eq!(aci_to_hex(7).as_deref(), Some("#000000"));
    }

    #[test]
    fn tint_blends_toward_white_and_clamps() {
        assert_eq!(tint_toward_white("#123456", 0.0), "#123456");
        assert_eq!(tint_toward_white("#123456", 1.0), "#ffffff");
        assert_eq!(tint_toward_white("#000000", 0.5), "#808080");
        assert_eq!(tint_toward_white("#123456", -1.0), "#123456");
        assert_eq!(tint_toward_white("#123456", 2.0), "#ffffff");
    }
}
