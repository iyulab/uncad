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

/// The origin the emitted coordinates are relative to: an SVG user unit is
/// the world minus this, y flipped. `(0, 0)` unless the drawing sits far
/// from the origin (see `svg::choose_origin`), and always `(0, 0)` inside a
/// block reference, whose own `<g transform>` carries the shift.
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
