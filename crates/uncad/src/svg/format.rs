//! Number and string formatting for the emitted SVG: the CAD-y-up to
//! SVG-y-down flip, coordinate cleanup, XML escaping and MTEXT's inline
//! formatting codes. Pure functions, no renderer state.

use crate::dynapi::{Point2D, Point3D};
use std::fmt::Write as _;

/// Snaps a subnormal `f64` (magnitude roughly below 2.2e-308) to exactly
/// `0.0`. Rust's `f64` `Display` never switches to scientific notation, so a
/// subnormal coordinate stringifies as several hundred characters of leading
/// zeros.
///
/// At subnormal magnitude a value is geometrically indistinguishable from `0`
/// at any realistic drawing scale, whatever made it that small -- this is a
/// cheap backstop for any path that ends up formatting a float read straight
/// out of memory. (One such bug, a wrong element stride in
/// [`crate::dynapi::SplineControlPoint`], was found through exactly this
/// symptom and fixed at its real source.)
pub(super) fn clean(x: f64) -> f64 {
    if x != 0.0 && x.is_subnormal() {
        0.0
    } else {
        x
    }
}

/// Negates `x` for the CAD-y-up to SVG-y-down flip, normalizing `-0.0` to
/// `0.0` so a zero coordinate prints as `"0"` rather than `"-0"` (purely
/// cosmetic -- SVG renders them identically). Runs the value through [`clean`]
/// first.
pub(super) fn neg(x: f64) -> f64 {
    let n = -clean(x);
    if n == 0.0 {
        0.0
    } else {
        n
    }
}

/// Drops the z coordinate: 3D entity geometry is rendered in plan view.
pub(super) fn xy(points: &[Point3D]) -> Vec<Point2D> {
    points.iter().map(|p| Point2D { x: p.x, y: p.y }).collect()
}

/// A `points="..."` attribute value: every point cleaned and y-flipped.
pub(super) fn points_attr(pts: &[Point2D]) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{},{}", clean(p.x), neg(p.y));
    }
    s
}

/// Escapes `s` for XML text content (`&`, `<`, `>`; quotes are left alone)
/// and drops every character XML 1.0 forbids outright
/// ([`crate::text::is_xml_illegal`]): a control byte in a label would
/// otherwise make roxmltree reject the whole document, and `to_png` and
/// the export package with it. The decoder already replaces such characters
/// in `text_plain`; this is the last line of defence for any string that
/// reaches the SVG some other way.
pub(super) fn escape_xml(s: &str) -> String {
    let s: std::borrow::Cow<str> = if s.chars().any(crate::text::is_xml_illegal) {
        s.chars()
            .filter(|&c| !crate::text::is_xml_illegal(c))
            .collect::<String>()
            .into()
    } else {
        s.into()
    };
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders a `rotate(deg x y)` `transform` attribute (leading space included,
/// empty string for zero rotation) around the already-SVG-space point
/// `(x, y)`. `rot_rad` is negated because SVG's y-axis points down while DWG
/// angles are counter-clockwise in a y-up world.
pub(super) fn rotate_transform_attr(rot_rad: f64, x: f64, y: f64) -> String {
    if rot_rad == 0.0 {
        return String::new();
    }
    let rot_deg = neg(rot_rad.to_degrees());
    format!(" transform=\"rotate({rot_deg} {x} {y})\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_snaps_subnormals_to_zero_but_leaves_normal_values_alone() {
        assert_eq!(clean(0.0), 0.0);
        assert_eq!(clean(1.5), 1.5);
        assert_eq!(clean(-1.5), -1.5);
        // A pointer bit-pattern reinterpreted as f64 typically lands here.
        assert_eq!(clean(f64::MIN_POSITIVE / 2.0), 0.0);
    }

    #[test]
    fn neg_flips_sign_and_canonicalizes_zero() {
        assert_eq!(neg(5.0), -5.0);
        assert_eq!(neg(-5.0), 5.0);
        assert!(
            neg(0.0).is_sign_positive(),
            "neg(0.0) must not print as \"-0\""
        );
        assert!(neg(-0.0).is_sign_positive());
    }

    #[test]
    fn points_attr_applies_clean_and_y_flip_to_each_point() {
        let pts = vec![Point2D { x: 1.0, y: 2.0 }, Point2D { x: 3.0, y: -4.0 }];
        assert_eq!(points_attr(&pts), "1,-2 3,4");
    }

    #[test]
    fn escape_xml_escapes_only_the_three_xml_text_metacharacters() {
        assert_eq!(escape_xml("<a & b>"), "&lt;a &amp; b&gt;");
        assert_eq!(escape_xml("no special chars"), "no special chars");
        // Quotes are deliberately NOT escaped -- text content, not an
        // attribute value.
        assert_eq!(escape_xml("\"quoted\""), "\"quoted\"");
    }

    #[test]
    fn escape_xml_drops_the_characters_xml_forbids() {
        // XML 1.0 `Char`: #x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD]
        // | [#x10000-#x10FFFF] -- so U+0001, U+000B, U+001F and U+FFFE are
        // out, while tab, newline, DEL and U+FFFD are in.
        assert_eq!(escape_xml("ZE\u{1}\u{B}RO\u{1F}\u{FFFE}"), "ZERO");
        assert_eq!(
            escape_xml("a\tb\nc\r\u{7F}\u{FFFD}"),
            "a\tb\nc\r\u{7F}\u{FFFD}"
        );
        // Stripping happens before escaping, so an entity is still emitted.
        assert_eq!(escape_xml("\u{0}<"), "&lt;");
    }

    #[test]
    fn rotate_transform_attr_is_empty_for_zero_rotation() {
        assert_eq!(rotate_transform_attr(0.0, 5.0, 7.0), "");
    }

    #[test]
    fn rotate_transform_attr_negates_degrees_for_svgs_flipped_y_axis() {
        let quarter_turn = std::f64::consts::FRAC_PI_2;
        assert_eq!(
            rotate_transform_attr(quarter_turn, 5.0, 7.0),
            " transform=\"rotate(-90 5 7)\""
        );
    }
}
