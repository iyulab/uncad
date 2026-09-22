//! Number and string formatting for the emitted SVG: the CAD-y-up to
//! SVG-y-down flip, coordinate cleanup, XML escaping and MTEXT's inline
//! formatting codes. Pure functions, no renderer state.

use crate::dynapi::{Point2D, Point3D};
use std::fmt::Write as _;

/// Below this magnitude a value is geometrically indistinguishable from `0`
/// at any realistic drawing scale, and Rust's `f64` `Display` -- which never
/// switches to scientific notation -- would spell it out with that many
/// leading zeros (1e-300 prints as 300 characters).
const NEGLIGIBLE: f64 = 1e-12;

/// Makes `x` safe to write into an SVG attribute: every non-finite value
/// becomes `0.0`, and so does anything below [`NEGLIGIBLE`] (subnormals
/// included).
///
/// `NaN` and `inf` are not in SVG's `<number>` grammar at all -- Rust's
/// `Display` writes them as the literals `NaN` and `inf`, which put the
/// attribute, and for a conforming SVG 1.1 consumer the whole element, in
/// error. They reach here from a corrupt or half-decoded entity (a fuzzed
/// DWG, a hand-written DXF). This is the document-level backstop that keeps
/// the output well-formed whatever the input; `svg::finite` screens the
/// entities themselves, so a bogus coordinate is normally left undrawn
/// rather than drawn at the origin.
///
/// The small-magnitude half is a cheap backstop for any path that ends up
/// formatting a float read straight out of memory. (One such bug, a wrong
/// element stride in [`crate::dynapi::SplineControlPoint`], was found
/// through exactly this symptom and fixed at its real source.)
pub(super) fn clean(x: f64) -> f64 {
    if !x.is_finite() || (x != 0.0 && x.abs() < NEGLIGIBLE) {
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

/// The origin the emitted coordinates are relative to: an SVG user unit is
/// the world minus this, y flipped. `(0, 0)` unless the drawing sits far
/// from the origin (see `svg::choose_origin`). Inside a block reference it
/// is the block-local point that reference's own placement sends to the
/// enclosing frame's origin (`(0, 0)` when that is `(0, 0)`, so a drawing
/// near the origin writes block interiors in their own coordinates as
/// before) -- see `svg::render_block_ref`.
///
/// Every coordinate the renderer writes goes through [`x`](Self::x),
/// [`y`](Self::y) or [`points`](Self::points): usvg and tiny-skia keep path
/// points in `f32`, so the numbers in the SVG text must stay small even
/// when the drawing's coordinates are not (a plan in millimetres at
/// projected coordinates of 2.5e8 has an `f32` step of 16 mm).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Frame {
    pub(super) ox: f64,
    pub(super) oy: f64,
}

impl Frame {
    pub(super) fn x(&self, x: f64) -> f64 {
        clean(x - self.ox)
    }

    pub(super) fn y(&self, y: f64) -> f64 {
        neg(y - self.oy)
    }

    /// A `points="..."` attribute value: every point shifted, cleaned and
    /// y-flipped.
    pub(super) fn points(&self, pts: &[Point2D]) -> String {
        let mut s = String::new();
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{},{}", self.x(p.x), self.y(p.y));
        }
        s
    }
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
    fn frame_points_applies_the_origin_clean_and_y_flip_to_each_point() {
        let pts = vec![Point2D { x: 1.0, y: 2.0 }, Point2D { x: 3.0, y: -4.0 }];
        assert_eq!(Frame::default().points(&pts), "1,-2 3,4");
        let far = Frame {
            ox: 1.0e7,
            oy: -2.0e7,
        };
        let pts = vec![Point2D {
            x: 1.0e7 + 1.0,
            y: -2.0e7 + 2.0,
        }];
        assert_eq!(far.points(&pts), "1,-2");
        assert_eq!(far.x(1.0e7), 0.0);
        assert!(far.y(-2.0e7).is_sign_positive(), "no -0");
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
