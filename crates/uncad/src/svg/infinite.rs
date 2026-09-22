//! RAY and XLINE: the two entities that have no end.
//!
//! A construction line is infinite, and the renderer used to draw it as a
//! segment 1e6 drawing units long. That is a coordinate, not a length: at a
//! high pixel scale (a drawing a few thousandths of a unit across, or a deep
//! tile level) the segment lands billions of pixels off the canvas, and
//! tiny-skia's fixed-point scan converter asserts its way out of the process
//! (`assertion failed: edges[curr_idx].last_y >= curr_y as i32`).
//!
//! The right endpoints are the ones the picture's own edge cuts, and they
//! cannot be computed while the entity is rendered: the crop, and with it
//! the viewBox, is only settled once every entity has been walked. So the
//! entity emits a *placeholder* carrying the line instead -- the same trick
//! [`super::stroke_width_placeholder`] plays for stroke widths -- and
//! [`resolve`] rewrites the assembled document, replacing each placeholder
//! with the clipped `<line>`, or with nothing when the line misses the
//! window entirely.
//!
//! The placeholder carries the line in the coordinates the element is
//! *written* in (SVG-local: y already flipped, and inside a block reference
//! the block's own space) together with [`InfiniteLine::matrix`], the
//! composition of the enclosing `<g transform>` matrices that takes those
//! coordinates to the document. Clipping happens in document space, where
//! the viewBox lives; the endpoints come back as parameters along the line,
//! which an affine map preserves, so they can be written straight back in
//! the element's own space without inverting anything.

use std::fmt::Write as _;

use super::format::clean;
use super::ViewBox;

/// Opens a placeholder. Closed by `@@`, like the stroke-width one, and
/// likewise chosen to be something no SVG this renderer emits contains.
pub(super) const MARKER: &str = "@@IL@@";

/// A 2 x 3 affine map in SVG's `matrix(a b c d e f)` order:
/// `(X, Y) = (a x + c y + e, b x + d y + f)`.
pub(super) type Matrix = [f64; 6];

pub(super) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `outer` applied after `inner`, i.e. the matrix product `outer * inner`:
/// what nesting one `<g transform>` inside another does to a point.
pub(super) fn compose(outer: Matrix, inner: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = outer;
    let [a2, b2, c2, d2, e2, f2] = inner;
    [
        a * a2 + c * b2,
        b * a2 + d * b2,
        a * c2 + c * d2,
        b * c2 + d * d2,
        a * e2 + c * f2 + e,
        b * e2 + d * f2 + f,
    ]
}

/// One RAY or XLINE, as the placeholder carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct InfiniteLine {
    /// The element's own coordinates -> the document's, i.e. the enclosing
    /// `<g transform>` matrices composed. Identity for a top-level entity.
    pub(super) matrix: Matrix,
    /// The base point, in the element's own coordinates.
    pub(super) base: (f64, f64),
    /// The direction, in the element's own coordinates, normalized so the
    /// clip parameters are lengths and stay in a sane numeric range.
    pub(super) dir: (f64, f64),
    /// An XLINE runs both ways from its base point; a RAY only forwards.
    pub(super) both_ways: bool,
}

impl InfiniteLine {
    /// The endpoints of the part of this line that `window` (a document-space
    /// rectangle `[x0, y0, x1, y1]`) shows, in the line's own coordinates.
    /// `None` when the line misses the window, is degenerate, or grazes it in
    /// a single point -- in every one of those cases there is nothing to draw.
    pub(super) fn clip_to(&self, window: [f64; 4]) -> Option<((f64, f64), (f64, f64))> {
        let [a, b, c, d, e, f] = self.matrix;
        let (bx, by) = self.base;
        let (dx, dy) = self.dir;
        // The line through the document, where the window is known.
        let base = (a * bx + c * by + e, b * bx + d * by + f);
        let dir = (a * dx + c * dy, b * dx + d * dy);
        let (t0, t1) = slab(base, dir, self.both_ways, window)?;
        // An affine map carries the parameter along: the document point at
        // t is the image of the own-space point at the same t.
        let p0 = (bx + t0 * dx, by + t0 * dy);
        let p1 = (bx + t1 * dx, by + t1 * dy);
        if !(p0.0.is_finite() && p0.1.is_finite() && p1.0.is_finite() && p1.1.is_finite()) {
            return None;
        }
        if p0 == p1 {
            return None;
        }
        Some((p0, p1))
    }
}

/// The parameter range of `base + t dir` that stays inside `window`
/// (`[x0, y0, x1, y1]`), clamped to `t >= 0` unless the line runs both ways:
/// the parametric slab test (Liang-Barsky). `None` when the range is empty,
/// when the direction is zero (so no finite range bounds it), or when
/// anything involved is not finite.
fn slab(
    base: (f64, f64),
    dir: (f64, f64),
    both_ways: bool,
    window: [f64; 4],
) -> Option<(f64, f64)> {
    let [x0, y0, x1, y1] = window;
    if !(base.0.is_finite()
        && base.1.is_finite()
        && dir.0.is_finite()
        && dir.1.is_finite()
        && x0.is_finite()
        && y0.is_finite()
        && x1.is_finite()
        && y1.is_finite())
    {
        return None;
    }
    let mut lo = if both_ways { f64::NEG_INFINITY } else { 0.0 };
    let mut hi = f64::INFINITY;
    for (p, q) in [
        (-dir.0, base.0 - x0),
        (dir.0, x1 - base.0),
        (-dir.1, base.1 - y0),
        (dir.1, y1 - base.1),
    ] {
        if p == 0.0 {
            // Parallel to this edge: inside forever, or outside forever.
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            // Entering this half-plane.
            if r > lo {
                lo = r;
            }
        } else if r < hi {
            hi = r;
        }
        if lo > hi {
            return None;
        }
    }
    // A zero direction leaves both ends unbounded; there is no segment.
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    Some((lo, hi))
}

/// The document-space window an infinite line is clipped to: `vb` (already
/// expressed relative to the render's origin) grown by a margin, so the
/// stroke's cap, a dash phase and a tile's overlap all fall outside the
/// pixels the image keeps rather than ending at their edge.
///
/// A percent of the diagonal, which is far more than the stroke at any
/// normal width; a few stroke widths when the stroke is the bigger of the
/// two (`auto_stroke_width`'s 0.01 floor dwarfs a drawing a thousandth of
/// a unit across); and never more than one diagonal, so the coordinates
/// stay within a small multiple of the canvas whatever the caller asked
/// for.
pub(super) fn window(vb: &ViewBox, stroke_width: f64) -> [f64; 4] {
    let diagonal = vb.width.hypot(vb.height);
    let margin = (diagonal * 0.01)
        .max(stroke_width.max(0.0) * 4.0)
        .min(diagonal);
    [
        vb.x - margin,
        vb.y - margin,
        vb.x + vb.width + margin,
        vb.y + vb.height + margin,
    ]
}

/// The placeholder a RAY or an XLINE emits in place of its element.
pub(super) fn placeholder(line: &InfiniteLine, color: &str) -> String {
    let [a, b, c, d, e, f] = line.matrix;
    // Eleven space-separated numbers and then the colour, which may hold
    // spaces but never `@@`, so it is taken as the whole rest of the
    // payload.
    format!(
        "{MARKER}{a} {b} {c} {d} {e} {f} {} {} {} {} {} {color}@@",
        line.base.0,
        line.base.1,
        line.dir.0,
        line.dir.1,
        u8::from(line.both_ways),
    )
}

/// Reads back what [`placeholder`] wrote: the line and its colour.
fn parse(payload: &str) -> Option<(InfiniteLine, &str)> {
    let mut fields = payload.splitn(12, ' ');
    let mut number = || -> Option<f64> { fields.next()?.parse::<f64>().ok() };
    let matrix = [
        number()?,
        number()?,
        number()?,
        number()?,
        number()?,
        number()?,
    ];
    let base = (number()?, number()?);
    let dir = (number()?, number()?);
    let both_ways = number()? != 0.0;
    let color = fields.next()?;
    Some((
        InfiniteLine {
            matrix,
            base,
            dir,
            both_ways,
        },
        color,
    ))
}

/// Replaces every placeholder in `body` with the `<line>` the `window`
/// leaves of it, or with nothing when it shows none of it. Costs nothing
/// for a drawing with no construction lines, which is almost all of them.
pub(super) fn resolve(body: String, window: [f64; 4]) -> String {
    if !body.contains(MARKER) {
        return body;
    }
    rewrite(&body, |payload| {
        let Some((line, color)) = parse(payload) else {
            // Never emitted; keeping it would put `@@` in the document.
            return String::new();
        };
        let Some((p0, p1)) = line.clip_to(window) else {
            return String::new();
        };
        format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-dasharray=\"4,2\" stroke=\"{color}\"/>",
            clean(p0.0),
            clean(p0.1),
            clean(p1.0),
            clean(p1.1)
        )
    })
}

/// Composes `outer` onto every placeholder's matrix in `fragment`: what a
/// fragment placed inside one more `<g transform>` (a sheet's viewport)
/// needs, so the clip still happens in the document's frame.
pub(super) fn transform(fragment: String, outer: Matrix) -> String {
    if !fragment.contains(MARKER) {
        return fragment;
    }
    rewrite(&fragment, |payload| match parse(payload) {
        Some((line, color)) => placeholder(
            &InfiniteLine {
                matrix: compose(outer, line.matrix),
                ..line
            },
            color,
        ),
        None => String::new(),
    })
}

/// Runs `each` over every `@@IL@@...@@` payload in `body`, splicing what it
/// returns in place of the whole placeholder.
fn rewrite(body: &str, mut each: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        let Some(start) = rest.find(MARKER) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start + MARKER.len()..];
        let Some(end) = after.find("@@") else {
            // Malformed (shouldn't happen) -- emit verbatim.
            out.push_str(&rest[start..]);
            break;
        };
        let _ = write!(out, "{}", each(&after[..end]));
        rest = &after[end + "@@".len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line in the document's own frame: no block matrix to compose.
    fn top_level(base: (f64, f64), dir: (f64, f64), both_ways: bool) -> InfiniteLine {
        let len = dir.0.hypot(dir.1);
        InfiniteLine {
            matrix: IDENTITY,
            base,
            dir: (dir.0 / len, dir.1 / len),
            both_ways,
        }
    }

    fn close(a: (f64, f64), b: (f64, f64)) {
        assert!(
            (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
            "expected {a:?} ~= {b:?}"
        );
    }

    // The window every case below clips to: the unit square (10, 10) to
    // (20, 20) in document coordinates (y down).
    const W: [f64; 4] = [10.0, 10.0, 20.0, 20.0];

    #[test]
    fn an_xline_through_the_window_is_cut_at_both_edges() {
        // Horizontal through the middle: the two vertical edges cut it.
        let (p0, p1) = top_level((15.0, 15.0), (1.0, 0.0), true)
            .clip_to(W)
            .expect("crosses the window");
        close(p0, (10.0, 15.0));
        close(p1, (20.0, 15.0));
    }

    #[test]
    fn an_xline_along_the_diagonal_is_cut_at_the_corners() {
        // 45 degrees through the centre: corner to corner, length 10 sqrt 2.
        let (p0, p1) = top_level((15.0, 15.0), (1.0, 1.0), true)
            .clip_to(W)
            .expect("crosses the window");
        close(p0, (10.0, 10.0));
        close(p1, (20.0, 20.0));
    }

    #[test]
    fn a_ray_starts_at_its_base_point_and_runs_only_forwards() {
        let (p0, p1) = top_level((15.0, 15.0), (1.0, 0.0), false)
            .clip_to(W)
            .expect("crosses the window");
        close(p0, (15.0, 15.0));
        close(p1, (20.0, 15.0));
    }

    #[test]
    fn a_ray_based_outside_the_window_still_crosses_it() {
        // From (0, 15) straight right: it enters at x = 10 and leaves at 20.
        let (p0, p1) = top_level((0.0, 15.0), (1.0, 0.0), false)
            .clip_to(W)
            .expect("crosses the window");
        close(p0, (10.0, 15.0));
        close(p1, (20.0, 15.0));
    }

    #[test]
    fn a_ray_pointing_away_from_the_window_draws_nothing() {
        assert_eq!(top_level((0.0, 15.0), (-1.0, 0.0), false).clip_to(W), None);
        // The same line as an XLINE does cross it.
        assert!(top_level((0.0, 15.0), (-1.0, 0.0), true)
            .clip_to(W)
            .is_some());
    }

    #[test]
    fn a_line_that_misses_the_window_draws_nothing() {
        // Parallel to the x edges, five units above the window.
        assert_eq!(top_level((15.0, 5.0), (1.0, 0.0), true).clip_to(W), None);
        // Diagonal that passes the corner on the outside.
        assert_eq!(top_level((0.0, 30.0), (1.0, 1.0), true).clip_to(W), None);
    }

    #[test]
    fn a_line_that_only_grazes_a_corner_draws_nothing() {
        // Through (10, 10) alone -- one step either way leaves the window,
        // so the clip is a single point and there is no segment to draw.
        assert_eq!(top_level((0.0, 20.0), (1.0, -1.0), true).clip_to(W), None);
        // One unit further in, the same direction does cross it.
        assert!(top_level((0.0, 21.0), (1.0, -1.0), true)
            .clip_to(W)
            .is_some());
    }

    #[test]
    fn a_zero_direction_draws_nothing() {
        let line = InfiniteLine {
            matrix: IDENTITY,
            base: (15.0, 15.0),
            dir: (0.0, 0.0),
            both_ways: true,
        };
        assert_eq!(line.clip_to(W), None);
    }

    #[test]
    fn the_endpoints_come_back_in_the_lines_own_coordinates() {
        // The element sits inside a group that scales by 2 and moves by
        // (10, 10): own-space (0, 0) is document (10, 10). A horizontal
        // xline through own-space (2.5, 2.5) = document (15, 15) is cut by
        // the window's vertical edges at document x = 10 and 20, i.e. own
        // x = 0 and 5.
        let line = InfiniteLine {
            matrix: [2.0, 0.0, 0.0, 2.0, 10.0, 10.0],
            base: (2.5, 2.5),
            dir: (1.0, 0.0),
            both_ways: true,
        };
        let (p0, p1) = line.clip_to(W).expect("crosses the window");
        close(p0, (0.0, 2.5));
        close(p1, (5.0, 2.5));
    }

    #[test]
    fn composing_a_matrix_moves_the_line_into_the_outer_frame() {
        // The group above, expressed as an outer matrix composed onto an
        // identity one: the same answer.
        let line = InfiniteLine {
            matrix: IDENTITY,
            base: (2.5, 2.5),
            dir: (1.0, 0.0),
            both_ways: true,
        };
        let moved = InfiniteLine {
            matrix: compose([2.0, 0.0, 0.0, 2.0, 10.0, 10.0], line.matrix),
            ..line
        };
        let (p0, p1) = moved.clip_to(W).expect("crosses the window");
        close(p0, (0.0, 2.5));
        close(p1, (5.0, 2.5));
    }

    #[test]
    fn a_placeholder_round_trips_through_the_document() {
        let line = InfiniteLine {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.5, -0.25],
            base: (15.0, 15.0),
            dir: (0.0, 1.0),
            both_ways: false,
        };
        let text = placeholder(&line, "#ff0000");
        let (back, color) = parse(&text[MARKER.len()..text.len() - 2]).expect("parses");
        assert_eq!(back, line);
        assert_eq!(color, "#ff0000");
    }

    #[test]
    fn resolve_replaces_the_placeholder_in_place_and_leaves_the_rest_alone() {
        let body = format!(
            "<line x1=\"0\"/>\n  {}\n  <circle/>",
            placeholder(&top_level((15.0, 15.0), (1.0, 0.0), true), "black")
        );
        let out = resolve(body, W);
        assert!(!out.contains(MARKER), "{out}");
        assert!(
            out.contains("<line x1=\"0\"/>") && out.contains("<circle/>"),
            "{out}"
        );
        assert!(
            out.contains("<line x1=\"10\" y1=\"15\" x2=\"20\" y2=\"15\" stroke-dasharray=\"4,2\" stroke=\"black\"/>"),
            "{out}"
        );
    }

    #[test]
    fn resolve_drops_a_line_the_window_misses_entirely() {
        let body = format!(
            "before{}after",
            placeholder(&top_level((15.0, 5.0), (1.0, 0.0), true), "black")
        );
        assert_eq!(resolve(body, W), "beforeafter");
    }

    #[test]
    fn transform_composes_onto_the_placeholder() {
        let body = placeholder(&top_level((2.5, 2.5), (1.0, 0.0), true), "black");
        let moved = transform(body, [2.0, 0.0, 0.0, 2.0, 10.0, 10.0]);
        let out = resolve(moved, W);
        assert!(
            out.contains("<line x1=\"0\" y1=\"2.5\" x2=\"5\" y2=\"2.5\""),
            "{out}"
        );
    }

    #[test]
    fn a_body_with_no_placeholder_is_returned_untouched() {
        let body = "<line x1=\"0\"/>".to_string();
        assert_eq!(resolve(body.clone(), W), body);
        assert_eq!(transform(body.clone(), IDENTITY), body);
    }

    #[test]
    fn the_window_grows_the_view_box_by_a_margin_on_every_side() {
        let vb = ViewBox {
            x: 0.0,
            y: -100.0,
            width: 100.0,
            height: 100.0,
        };
        let [x0, y0, x1, y1] = window(&vb, 0.0);
        let margin = 100.0_f64.hypot(100.0) * 0.01;
        assert!((x0 - -margin).abs() < 1e-12, "{x0}");
        assert!((y0 - (-100.0 - margin)).abs() < 1e-12, "{y0}");
        assert!((x1 - (100.0 + margin)).abs() < 1e-12, "{x1}");
        assert!((y1 - margin).abs() < 1e-12, "{y1}");
        // A stroke wider than a percent of the diagonal sets the margin.
        let [x0, _, _, _] = window(&vb, 10.0);
        assert!((x0 - -40.0).abs() < 1e-12, "{x0}");
        // But the margin never exceeds one diagonal, however wide the
        // caller's stroke: a 0.01 stroke on a drawing a thousandth of a
        // unit across would otherwise clip forty canvases out.
        let [x0, _, _, _] = window(&vb, 1e6);
        assert!((x0 - -100.0_f64.hypot(100.0)).abs() < 1e-12, "{x0}");
    }
}
