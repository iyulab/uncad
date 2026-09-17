//! Number and string formatting for the emitted SVG: the CAD-y-up to
//! SVG-y-down flip, coordinate cleanup, XML escaping and MTEXT's inline
//! formatting codes. Pure functions, no renderer state.

use crate::dynapi::{Point2D, Point3D};
use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

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

pub(super) fn escape_xml(s: &str) -> String {
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

/// Strips MTEXT's inline formatting codes (`\A`, `\H`, `\W`, `\C`, `\Q`, `\T`,
/// `\f`, `{...}` grouping, `\P` paragraph breaks) down to plain, possibly
/// multi-line text. `\S...;` (stacked/fraction text, e.g. `"1#8"` -> `"1/8"`)
/// is unwrapped rather than dropped, since the fraction content is meaningful.
///
/// A best-effort plain-text approximation, not a real MTEXT formatter: no
/// stacked fractions, color or font changes are visually reproduced.
pub(super) fn strip_mtext_formatting(text: &str) -> String {
    static STACKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\S([^;]*);").unwrap());
    static HASH_CARET_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[#^\\]").unwrap());
    static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\P").unwrap());
    static NBSP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\~").unwrap());
    static INLINE_CODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\\[A-Za-z][^;]*;").unwrap());
    static BRACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[{}]").unwrap());
    static DOUBLE_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\\\").unwrap());

    let unstacked = STACKED.replace_all(text, |caps: &regex::Captures| {
        HASH_CARET_BACKSLASH.replace_all(&caps[1], "/").into_owned()
    });
    let paragraphs = PARAGRAPH.replace_all(&unstacked, "\n");
    let spaces = NBSP.replace_all(&paragraphs, " ");
    let uncoded = INLINE_CODE.replace_all(&spaces, "");
    let ungrouped = BRACES.replace_all(&uncoded, "");
    DOUBLE_BACKSLASH.replace_all(&ungrouped, "\\").into_owned()
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

    #[test]
    fn strip_mtext_formatting_unwraps_stacked_fractions_and_drops_inline_codes() {
        let stripped = strip_mtext_formatting(r"\A1;Line1\PLine2 {\C1;colored} \S1#8;");
        assert_eq!(stripped, "Line1\nLine2 colored 1/8");
    }

    #[test]
    fn strip_mtext_formatting_collapses_double_backslash_and_expands_nbsp() {
        assert_eq!(strip_mtext_formatting(r"a\\b"), r"a\b");
        assert_eq!(strip_mtext_formatting(r"a\~b"), "a b");
    }
}
