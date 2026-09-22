//! Geometry the exports need and the file does not state outright: the OCS
//! (object coordinate system, DXF 210) to world transform, the arc a
//! polyline bulge describes, and the exact length, area and extents of a
//! polyline whose segments may be arcs.
//!
//! Bulge convention (AutoCAD's): the bulge on vertex `i` applies to the
//! segment from vertex `i` to `i + 1` (the closing segment included when the
//! polyline is closed) and equals `tan(theta / 4)` for the arc's included
//! angle `theta`; positive is counter-clockwise from the first vertex to the
//! second, 0 is a straight segment, 1 is a semicircle.

use crate::model::{Point2D, Point3D};

/// The world Z axis: the OCS every 2D entity is drawn in unless its
/// extrusion says otherwise.
pub const WORLD_Z: Point3D = Point3D {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// Whether an extrusion vector is (within rounding) the world Z axis, so
/// OCS coordinates already are world coordinates. A zero vector counts too:
/// LibreDWG leaves it at (0,0,0) when the file did not store one.
pub fn is_world_z(normal: Point3D) -> bool {
    let eps = 1e-9;
    (normal.x.abs() < eps && normal.y.abs() < eps && (normal.z - 1.0).abs() < eps)
        || (normal.x == 0.0 && normal.y == 0.0 && normal.z == 0.0)
}

/// The largest magnitude, in radians, this crate will read a stored angle
/// (or ellipse parameter) at. Real files store an angle in `0..2*pi`;
/// LibreDWG hands one through unchecked, so a corrupt or mis-decoded field
/// can arrive as 1e20 or 1e247. Past ~1e6 rad an `f64` step is already
/// coarser than 1e-10 rad and the value names no direction any more, so the
/// arc it would describe is not geometry but noise.
pub const MAX_ANGLE: f64 = 1.0e6;

/// Whether `a` can be read as a stored angle at all -- finite and within
/// [`MAX_ANGLE`]. The entity-level screen for angles, the way
/// [`crate::crop::Rect::is_sane`] screens rectangles.
pub fn is_sane_angle(a: f64) -> bool {
    a.is_finite() && a.abs() <= MAX_ANGLE
}

/// A stored extrusion, normalized: a zero or non-finite vector becomes the
/// world Z axis, anything else is scaled to unit length.
pub fn normalize_extrusion(normal: Point3D) -> Point3D {
    let len = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if !len.is_finite() || len < 1e-12 {
        return WORLD_Z;
    }
    Point3D {
        x: normal.x / len,
        y: normal.y / len,
        z: normal.z / len,
    }
}

/// Maps a point given in the OCS of `normal` to world coordinates with the
/// DXF reference's arbitrary axis algorithm: the OCS x axis is `Wy x N` when
/// `|Nx|` and `|Ny|` are both under 1/64, `Wz x N` otherwise; the OCS y axis
/// is `N x Ax`. For the common mirrored case `N = (0,0,-1)` this is
/// `(x, y, z) -> (-x, y, -z)`.
pub fn ocs_to_wcs(p: Point3D, normal: Point3D) -> Point3D {
    let n = normalize_extrusion(normal);
    if is_world_z(n) {
        return p;
    }
    let ax = if n.x.abs() < 1.0 / 64.0 && n.y.abs() < 1.0 / 64.0 {
        cross(
            Point3D {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross(WORLD_Z, n)
    };
    let ax = normalize_extrusion(ax);
    let ay = cross(n, ax);
    Point3D {
        x: p.x * ax.x + p.y * ay.x + p.z * n.x,
        y: p.x * ax.y + p.y * ay.y + p.z * n.y,
        z: p.x * ax.z + p.y * ay.z + p.z * n.z,
    }
}

/// [`ocs_to_wcs`] for a 2D point at `elevation` in its OCS, keeping the
/// world x and y.
pub fn ocs_to_wcs_2d(p: Point2D, elevation: f64, normal: Point3D) -> Point2D {
    let w = ocs_to_wcs(
        Point3D {
            x: p.x,
            y: p.y,
            z: elevation,
        },
        normal,
    );
    Point2D { x: w.x, y: w.y }
}

fn cross(a: Point3D, b: Point3D) -> Point3D {
    Point3D {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// The circular arc a bulge describes between two vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BulgeArc {
    pub center: Point2D,
    pub radius: f64,
    /// Radians, of the arc's first point about the centre.
    pub start_angle: f64,
    /// Radians, of the arc's second point about the centre.
    pub end_angle: f64,
    /// The signed included angle in radians: positive counter-clockwise.
    pub sweep: f64,
}

impl BulgeArc {
    pub fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
    }

    /// The area between the arc and its chord (the circular segment),
    /// always positive.
    pub fn segment_area(&self) -> f64 {
        let theta = self.sweep.abs();
        self.radius * self.radius / 2.0 * (theta - theta.sin())
    }

    /// The point at parameter `t` in `0..=1` along the arc.
    pub fn point_at(&self, t: f64) -> Point2D {
        let angle = self.start_angle + self.sweep * t;
        Point2D {
            x: self.center.x + self.radius * angle.cos(),
            y: self.center.y + self.radius * angle.sin(),
        }
    }

    /// Axis-aligned bounds of the arc itself (not the whole circle): its
    /// endpoints plus every axis crossing inside the sweep.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut take = |p: Point2D| {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        };
        take(self.point_at(0.0));
        take(self.point_at(1.0));
        let tau = std::f64::consts::TAU;
        let quarter = tau / 4.0;
        let (a0, a1) = if self.sweep >= 0.0 {
            (self.start_angle, self.start_angle + self.sweep)
        } else {
            (self.end_angle, self.end_angle - self.sweep)
        };
        // Every multiple of 90 degrees between a0 and a1 (counter-clockwise),
        // and *at most four* of them: a fifth crossing would only repeat the
        // first one's axis direction, so four already pin every extreme a
        // circle has. The bound is by construction rather than by trusting
        // a1: a corrupt file's start angle of 1e20 rad (or 1e247, seen in a
        // fuzzed DWG) made this walk run a1 / (pi/2) times, and past 2^53
        // `k += 1.0` stops advancing at all, so the loop never ended. A NaN
        // angle is tested for on its own, since every comparison against it
        // is false.
        let first = (a0 / quarter).ceil();
        for i in 0..4 {
            let angle = (first + f64::from(i)) * quarter;
            if angle.is_nan() || angle > a1 + 1e-12 {
                break;
            }
            take(Point2D {
                x: self.center.x + self.radius * angle.cos(),
                y: self.center.y + self.radius * angle.sin(),
            });
        }
        (min_x, min_y, max_x, max_y)
    }
}

/// The elliptical arc an ELLIPSE entity describes, in world coordinates.
///
/// DXF 41/42 (`start_angle`/`end_angle` in the model, and in LibreDWG) are
/// *parameters*, not angles: the point at parameter `t` is
/// `center + major cos t + minor sin t`, where `minor` is `major` turned a
/// quarter turn counter-clockwise and scaled by the axis ratio. Only on a
/// circle (ratio 1) is `t` the angle to the point; on a flattened ellipse
/// the parameter runs ahead of the polar angle everywhere but on the axes.
/// A full ellipse stores `0 .. 2*pi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EllipseArc {
    pub center: Point2D,
    /// The major-axis endpoint *relative to* `center` (DXF 11), i.e. the
    /// major radius as a vector.
    pub major: Point2D,
    /// Minor/major radius ratio (DXF 40).
    pub ratio: f64,
    /// DXF 41, radians.
    pub start_param: f64,
    /// DXF 42, radians.
    pub end_param: f64,
}

impl EllipseArc {
    /// The major radius: the length of [`major`](Self::major).
    pub fn major_radius(&self) -> f64 {
        self.major.x.hypot(self.major.y)
    }

    /// The minor-axis vector: [`major`](Self::major) turned a quarter turn
    /// counter-clockwise and scaled by [`ratio`](Self::ratio).
    pub fn minor(&self) -> Point2D {
        Point2D {
            x: -self.major.y * self.ratio,
            y: self.major.x * self.ratio,
        }
    }

    pub fn point_at_param(&self, t: f64) -> Point2D {
        let m = self.minor();
        let (sin, cos) = t.sin_cos();
        Point2D {
            x: self.center.x + self.major.x * cos + m.x * sin,
            y: self.center.y + self.major.y * cos + m.y * sin,
        }
    }

    /// The counter-clockwise sweep from `start_param` to `end_param`, in
    /// `(0, 2*pi]`. A pair that does not advance (equal parameters) means a
    /// full ellipse, which is how AutoCAD stores one, so zero maps to a
    /// full turn rather than to nothing.
    pub fn sweep(&self) -> f64 {
        let tau = std::f64::consts::TAU;
        let raw = self.end_param - self.start_param;
        if !raw.is_finite() || raw >= tau {
            return tau;
        }
        let s = if raw <= 0.0 { raw + tau } else { raw };
        if s >= tau {
            tau
        } else {
            s
        }
    }

    /// Whether this is a closed ellipse rather than an arc of one.
    pub fn is_full(&self) -> bool {
        self.sweep() >= std::f64::consts::TAU - 1e-9
    }

    /// Axis-aligned bounds of the arc itself (not of the whole ellipse): its
    /// two ends plus whichever of the four points where `dx/dt` or `dy/dt`
    /// vanishes fall inside the sweep. `x(t) = cx + Mx cos t + mx sin t` is
    /// stationary where `tan t = mx / Mx`, i.e. at `atan2(mx, Mx)` and half
    /// a turn later; `y` likewise from the other components.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut take = |p: Point2D| {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        };
        let sweep = self.sweep();
        take(self.point_at_param(self.start_param));
        take(self.point_at_param(self.start_param + sweep));
        let m = self.minor();
        for base in [m.x.atan2(self.major.x), m.y.atan2(self.major.y)] {
            for half_turn in [0.0, std::f64::consts::PI] {
                let offset =
                    (base + half_turn - self.start_param).rem_euclid(std::f64::consts::TAU);
                if offset <= sweep + 1e-12 {
                    take(self.point_at_param(self.start_param + offset));
                }
            }
        }
        (min_x, min_y, max_x, max_y)
    }
}

/// The arc from `from` to `to` with the given bulge, or `None` for a
/// straight segment (bulge 0, or coincident points).
pub fn bulge_arc(from: Point2D, to: Point2D, bulge: f64) -> Option<BulgeArc> {
    if bulge == 0.0 || !bulge.is_finite() {
        return None;
    }
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let chord = dx.hypot(dy);
    if chord < 1e-12 {
        return None;
    }
    let sweep = 4.0 * bulge.atan();
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let sagitta = chord * bulge.abs() / 2.0;
    // The centre sits on the chord's perpendicular bisector, to the left of
    // travel for a counter-clockwise arc, `radius - sagitta` away (negative
    // past a semicircle, i.e. on the bulge's own side).
    let mid = Point2D {
        x: (from.x + to.x) / 2.0,
        y: (from.y + to.y) / 2.0,
    };
    let left = Point2D {
        x: -dy / chord,
        y: dx / chord,
    };
    let offset = (radius - sagitta) * bulge.signum();
    let center = Point2D {
        x: mid.x + left.x * offset,
        y: mid.y + left.y * offset,
    };
    Some(BulgeArc {
        center,
        radius,
        start_angle: (from.y - center.y).atan2(from.x - center.x),
        end_angle: (to.y - center.y).atan2(to.x - center.x),
        sweep,
    })
}

/// One segment of a polyline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line {
        from: Point2D,
        to: Point2D,
    },
    Arc {
        from: Point2D,
        to: Point2D,
        bulge: f64,
        arc: BulgeArc,
    },
}

impl Segment {
    pub fn length(&self) -> f64 {
        match self {
            Segment::Line { from, to } => (to.x - from.x).hypot(to.y - from.y),
            Segment::Arc { arc, .. } => arc.length(),
        }
    }
}

/// The segments of a polyline: `bulges[i]` (0 when absent) applies to the
/// segment leaving vertex `i`; a closed polyline gets the segment back to
/// its first vertex. A closing vertex that repeats the first one is dropped
/// first so it does not produce a zero-length segment.
pub fn polyline_segments(vertices: &[Point2D], bulges: &[f64], closed: bool) -> Vec<Segment> {
    let vertices = dedup_closing_vertex(vertices, closed);
    let n = vertices.len();
    if n < 2 {
        return Vec::new();
    }
    let count = if closed { n } else { n - 1 };
    (0..count)
        .map(|i| {
            let from = vertices[i];
            let to = vertices[(i + 1) % n];
            let bulge = bulges.get(i).copied().unwrap_or(0.0);
            match bulge_arc(from, to, bulge) {
                Some(arc) => Segment::Arc {
                    from,
                    to,
                    bulge,
                    arc,
                },
                None => Segment::Line { from, to },
            }
        })
        .collect()
}

fn dedup_closing_vertex(vertices: &[Point2D], closed: bool) -> Vec<Point2D> {
    let mut out = vertices.to_vec();
    if closed && out.len() > 2 {
        let (first, last) = (out[0], out[out.len() - 1]);
        if (first.x - last.x).abs() < 1e-9 && (first.y - last.y).abs() < 1e-9 {
            out.pop();
        }
    }
    out
}

/// The polyline's length (its perimeter when closed).
pub fn polyline_length(vertices: &[Point2D], bulges: &[f64], closed: bool) -> f64 {
    polyline_segments(vertices, bulges, closed)
        .iter()
        .map(Segment::length)
        .sum()
}

/// The signed area a polyline encloses, arcs included: the shoelace area of
/// its vertices plus each arc's circular segment, added when the arc bulges
/// outward and subtracted when it bulges inward. Positive for a
/// counter-clockwise outline. An open polyline is closed by a straight
/// segment from its last vertex back to its first for this purpose: the
/// bulge stored on the last vertex (AutoCAD keeps one there, e.g. after
/// BREAK or TRIM) applies to no segment, exactly as in
/// [`polyline_segments`]. Self-intersecting outlines give a value with no
/// geometric meaning; see [`is_simple`].
pub fn polyline_signed_area(vertices: &[Point2D], bulges: &[f64], closed: bool) -> f64 {
    let mut segments = polyline_segments(vertices, bulges, closed);
    if !closed {
        if let (Some(&first), Some(&last)) = (vertices.first(), vertices.last()) {
            if vertices.len() >= 2 {
                segments.push(Segment::Line {
                    from: last,
                    to: first,
                });
            }
        }
    }
    // Three straight segments are the fewest that can enclose anything --
    // but two *arcs* can. A closed two-vertex polyline with bulges is
    // exactly what AutoCAD's DONUT command writes, and two bulges of 1
    // describe a full circle. The shoelace terms of such a shape cancel to
    // zero, so what is left is the two circular-segment terms, which are
    // its area. Fewer than three segments with no arc among them still
    // encloses nothing.
    if segments.len() < 3 && !segments.iter().any(|s| matches!(s, Segment::Arc { .. })) {
        return 0.0;
    }
    let mut area = 0.0;
    for segment in &segments {
        match *segment {
            Segment::Line { from, to } => area += (from.x * to.y - to.x * from.y) / 2.0,
            Segment::Arc {
                from,
                to,
                bulge,
                arc,
            } => {
                area += (from.x * to.y - to.x * from.y) / 2.0;
                // A counter-clockwise arc bulges to the right of travel,
                // which is outward for a counter-clockwise outline and
                // inward for a clockwise one -- so adding a signed term does
                // the right thing for both orientations.
                area += arc.segment_area() * bulge.signum();
            }
        }
    }
    area
}

/// The enclosed area, unsigned.
pub fn polyline_area(vertices: &[Point2D], bulges: &[f64], closed: bool) -> f64 {
    polyline_signed_area(vertices, bulges, closed).abs()
}

/// Whether the straight-segment outline through `vertices` (closed) has no
/// two non-adjacent segments crossing. Quadratic; arcs are treated as their
/// chords.
pub fn is_simple(vertices: &[Point2D]) -> bool {
    let vertices = dedup_closing_vertex(vertices, true);
    let n = vertices.len();
    if n < 4 {
        return true;
    }
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (a, b) = (vertices[i], vertices[(i + 1) % n]);
            let (c, d) = (vertices[j], vertices[(j + 1) % n]);
            if segments_cross(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

fn segments_cross(a: Point2D, b: Point2D, c: Point2D, d: Point2D) -> bool {
    let orient =
        |p: Point2D, q: Point2D, r: Point2D| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    (o1 > 0.0) != (o2 > 0.0)
        && (o3 > 0.0) != (o4 > 0.0)
        && o1 != 0.0
        && o2 != 0.0
        && o3 != 0.0
        && o4 != 0.0
}

/// Axis-aligned bounds of a polyline, arcs included; `None` when it has no
/// vertices.
pub fn polyline_bounds(
    vertices: &[Point2D],
    bulges: &[f64],
    closed: bool,
) -> Option<(f64, f64, f64, f64)> {
    if vertices.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for v in vertices {
        min_x = min_x.min(v.x);
        min_y = min_y.min(v.y);
        max_x = max_x.max(v.x);
        max_y = max_y.max(v.y);
    }
    for segment in polyline_segments(vertices, bulges, closed) {
        if let Segment::Arc { arc, .. } = segment {
            let (x0, y0, x1, y1) = arc.bounds();
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
        }
    }
    Some((min_x, min_y, max_x, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2D {
        Point2D { x, y }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn mirrored_ocs_flips_x() {
        let n = Point3D {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        let w = ocs_to_wcs(
            Point3D {
                x: 10.0,
                y: 20.0,
                z: 3.0,
            },
            n,
        );
        assert!(
            close(w.x, -10.0) && close(w.y, 20.0) && close(w.z, -3.0),
            "{w:?}"
        );
        let same = ocs_to_wcs(
            Point3D {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            WORLD_Z,
        );
        assert_eq!((same.x, same.y, same.z), (1.0, 2.0, 3.0));
        // A zero vector (LibreDWG's "not stored") is the world axis.
        let zero = Point3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(is_world_z(zero));
        assert_eq!(ocs_to_wcs_2d(p(5.0, 6.0), 0.0, zero), p(5.0, 6.0));
    }

    #[test]
    fn tilted_normal_uses_the_arbitrary_axis_rule() {
        // N = (1,0,0): Ax = Wz x N = (0,1,0), Ay = N x Ax = (0,0,1).
        let n = Point3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
        let w = ocs_to_wcs(
            Point3D {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            n,
        );
        assert!(
            close(w.x, 3.0) && close(w.y, 1.0) && close(w.z, 2.0),
            "{w:?}"
        );
    }

    #[test]
    fn the_design_documents_worked_example() {
        // A 100 x 50 rectangle whose right edge is a 90-degree arc.
        let vertices = [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)];
        let bulges = [0.0, 0.41421356, 0.0, 0.0];
        let segments = polyline_segments(&vertices, &bulges, true);
        assert_eq!(segments.len(), 4);
        let Segment::Arc { arc, .. } = segments[1] else {
            panic!("second segment is the arc");
        };
        assert!(
            close(arc.center.x, 75.0) && close(arc.center.y, 25.0),
            "{arc:?}"
        );
        assert!(close(arc.radius, 35.355339), "{}", arc.radius);
        assert!(close(arc.sweep.to_degrees(), 90.0), "{}", arc.sweep);
        assert!(close(arc.length(), 55.536037), "{}", arc.length());
        assert!(
            close(arc.segment_area(), 356.747702),
            "{}",
            arc.segment_area()
        );
        assert!(close(polyline_length(&vertices, &bulges, true), 305.536037));
        assert!(close(polyline_area(&vertices, &bulges, true), 5356.747702));
        let (_, _, max_x, _) = polyline_bounds(&vertices, &bulges, true).unwrap();
        assert!(close(max_x, 110.355339), "{max_x}");
        // The same outline clockwise: same area, negative sign.
        let cw: Vec<Point2D> = vertices.iter().rev().copied().collect();
        // Reversed, the arc is the segment leaving vertex 1 ((100,50) down to
        // (100,0)) and must bulge to the left of travel, i.e. negatively.
        let cw_bulges = [0.0, -0.41421356, 0.0, 0.0];
        assert!(close(
            polyline_signed_area(&cw, &cw_bulges, true),
            -5356.747702
        ));
        assert!(is_simple(&vertices));
    }

    #[test]
    fn bulges_beyond_a_semicircle_put_the_centre_on_the_bulge_side() {
        // A 270-degree arc from (0,0) to (10,0): bulge = tan(67.5 deg).
        let bulge = (270f64 / 4.0).to_radians().tan();
        let arc = bulge_arc(p(0.0, 0.0), p(10.0, 0.0), bulge).unwrap();
        assert!(close(arc.sweep.to_degrees(), 270.0));
        // Centre below the chord (the arc bulges to the right of travel,
        // i.e. downward, and past a semicircle the centre is on that side).
        assert!(arc.center.y < 0.0, "{arc:?}");
        assert!(close(arc.center.x, 5.0));
        // The exact box of the arc itself. Centre (5,-5), r = 5 sqrt(2) =
        // 7.0710678, start 135 degrees, sweep +270, so the counter-clockwise
        // run 135..405 degrees crosses 180, 270 and 360 but never 90 (450 is
        // past the end). The crossings give min_x = centre.x - r, min_y =
        // centre.y - r and max_x = centre.x + r; the top of the circle is
        // outside the sweep, so max_y comes from the endpoints (0,0) and
        // (10,0) and is 0, not centre.y + r = 2.0710678.
        let r = 5.0 * 2f64.sqrt();
        let (min_x, min_y, max_x, max_y) = arc.bounds();
        assert!(close(arc.radius, r), "{}", arc.radius);
        assert!(close(min_x, 5.0 - r), "{min_x}");
        assert!(close(min_y, -5.0 - r), "{min_y}");
        assert!(close(max_x, 5.0 + r), "{max_x}");
        assert!(
            max_y <= 1e-9,
            "the sweep misses the top of the circle: {max_y}"
        );
        assert!(close(max_y, 0.0), "{max_y}");
    }

    #[test]
    fn an_arcs_bounds_are_its_own_not_the_whole_circles() {
        // A quarter circle of radius 10 about the origin, 0 to 90 degrees:
        // the endpoints are (10,0) and (0,10) and the only crossings in the
        // sweep are those same two points, so the box is (0,0)-(10,10), a
        // tenth of the whole circle's 20 x 20 area.
        let quarter = BulgeArc {
            center: p(0.0, 0.0),
            radius: 10.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            sweep: std::f64::consts::FRAC_PI_2,
        };
        let (min_x, min_y, max_x, max_y) = quarter.bounds();
        assert!(close(min_x, 0.0) && close(min_y, 0.0), "{min_x} {min_y}");
        assert!(close(max_x, 10.0) && close(max_y, 10.0), "{max_x} {max_y}");
        // The same quarter turned by 45 degrees (-45 to 45) crosses 0
        // degrees, so it reaches x = 10 while y stays within +-10/sqrt(2).
        let turned = BulgeArc {
            center: p(0.0, 0.0),
            radius: 10.0,
            start_angle: -std::f64::consts::FRAC_PI_4,
            end_angle: std::f64::consts::FRAC_PI_4,
            sweep: std::f64::consts::FRAC_PI_2,
        };
        let h = 10.0 / 2f64.sqrt();
        let (min_x, min_y, max_x, max_y) = turned.bounds();
        assert!(close(min_x, h) && close(max_x, 10.0), "{min_x} {max_x}");
        assert!(close(min_y, -h) && close(max_y, h), "{min_y} {max_y}");
        // A clockwise sweep bounds the same arc as the counter-clockwise one
        // between the same two angles.
        let backwards = BulgeArc {
            center: p(0.0, 0.0),
            radius: 10.0,
            start_angle: std::f64::consts::FRAC_PI_2,
            end_angle: 0.0,
            sweep: -std::f64::consts::FRAC_PI_2,
        };
        let (min_x, min_y, max_x, max_y) = backwards.bounds();
        assert!(close(min_x, 0.0) && close(min_y, 0.0), "{min_x} {min_y}");
        assert!(close(max_x, 10.0) && close(max_y, 10.0), "{max_x} {max_y}");
    }

    #[test]
    fn open_polylines_and_degenerate_input() {
        let vertices = [p(0.0, 0.0), p(3.0, 4.0)];
        assert!(close(polyline_length(&vertices, &[], false), 5.0));
        assert_eq!(polyline_segments(&vertices, &[], true).len(), 2);
        assert_eq!(polyline_segments(&[p(1.0, 1.0)], &[], true).len(), 0);
        assert_eq!(polyline_area(&vertices, &[], false), 0.0);
        assert!(bulge_arc(p(0.0, 0.0), p(0.0, 0.0), 1.0).is_none());
        // A repeated closing vertex does not add a zero-length segment.
        let repeated = [p(0.0, 0.0), p(4.0, 0.0), p(4.0, 3.0), p(0.0, 0.0)];
        assert_eq!(polyline_segments(&repeated, &[], true).len(), 3);
        assert!(close(polyline_area(&repeated, &[], true), 6.0));
        assert!(close(polyline_length(&repeated, &[], true), 12.0));
    }

    #[test]
    fn an_open_polylines_last_bulge_does_not_bend_the_closing_segment() {
        // The right triangle (0,0) -> (10,0) -> (10,10), open, with a bulge
        // of 1 (a semicircle) left on its last vertex. Closed by a straight
        // segment the area is 10 * 10 / 2 = 50, and the last bulge applies
        // to nothing. Bent, the closing chord (10,10) -> (0,0) of length
        // 10 sqrt(2) would carry a semicircle of radius 5 sqrt(2), adding
        // pi * 50 / 2 = 78.539816 for a bulge of +1 (an arc to the right of
        // travel, outside this counter-clockwise triangle) or taking it
        // away for -1.
        let vertices = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)];
        for bulge in [1.0, -1.0, 669.19] {
            let bulges = [0.0, 0.0, bulge];
            assert!(
                close(polyline_signed_area(&vertices, &bulges, false), 50.0),
                "bulge {bulge}: {}",
                polyline_signed_area(&vertices, &bulges, false)
            );
            // Length is unaffected either way: two straight sides.
            assert!(close(polyline_length(&vertices, &bulges, false), 20.0));
        }
        // Flagged closed, the same bulge is the real closing segment.
        let bulges = [0.0, 0.0, 1.0];
        assert!(close(
            polyline_signed_area(&vertices, &bulges, true),
            50.0 + std::f64::consts::PI * 50.0 / 2.0
        ));
        // An open polyline drawn back to its first vertex: the bulge on the
        // vertex before the repeat is a real segment, the repeat's own is
        // not. Here the third segment (10,10) -> (0,0) with bulge -1 is a
        // semicircle to the left of travel, i.e. inside, so 50 - 78.539816;
        // the 5.0 on the repeated vertex (a 315-degree arc, were there a
        // chord for it) must count for nothing.
        let back_home = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)];
        let bulges = [0.0, 0.0, -1.0, 5.0];
        assert!(close(
            polyline_signed_area(&back_home, &bulges, false),
            50.0 - std::f64::consts::PI * 50.0 / 2.0
        ));
    }

    #[test]
    fn self_intersection_is_detected() {
        let bowtie = [p(0.0, 0.0), p(10.0, 10.0), p(10.0, 0.0), p(0.0, 10.0)];
        assert!(!is_simple(&bowtie));
        let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
        assert!(is_simple(&square));
    }

    #[test]
    fn arc_bounds_terminate_for_any_angle_however_absurd() {
        // The quarter-turn walk used to run once per quarter turn between
        // the two angles, so a start angle of 1e20 rad meant ~6e19 steps --
        // and past 2^53 (1.4e16 rad) `k += 1.0` no longer advances at all,
        // so it never ended. 1e20 is the hand-written repro's DXF group 50;
        // 1.4052120271735542e247 is what a fuzzed `entities-2d.dwg` stored.
        // Whatever the angles, the answer must contain the two endpoints,
        // which is all that is asserted here -- the point of the test is
        // that it returns.
        for (start, end, radius) in [
            (1.0e20, 1.0, 2.0),
            (1.4052120271735542e247, 1.0, 0.0),
            (-1.0e18, 1.0e18, 5.0),
            (0.0, f64::INFINITY, 1.0),
            (f64::NAN, 1.0, 1.0),
        ] {
            let arc = BulgeArc {
                center: p(0.0, 0.0),
                radius,
                start_angle: start,
                end_angle: end,
                sweep: end - start,
            };
            let (min_x, min_y, max_x, max_y) = arc.bounds();
            assert!(min_x <= max_x || min_x.is_infinite(), "{min_x} {max_x}");
            assert!(min_y <= max_y || min_y.is_infinite(), "{min_y} {max_y}");
        }
        // And a real arc is unaffected: a 270-degree arc of radius 2 about
        // the origin from 0 to 3*pi/2 crosses +y and -x and -y, so its box
        // is x -2..2, y -2..2 (only the +x extreme is missing, and the
        // start point (2,0) supplies it).
        let arc = BulgeArc {
            center: p(0.0, 0.0),
            radius: 2.0,
            start_angle: 0.0,
            end_angle: 3.0 * std::f64::consts::FRAC_PI_2,
            sweep: 3.0 * std::f64::consts::FRAC_PI_2,
        };
        let (min_x, min_y, max_x, max_y) = arc.bounds();
        assert!(close(min_x, -2.0) && close(max_x, 2.0), "{min_x} {max_x}");
        assert!(close(min_y, -2.0) && close(max_y, 2.0), "{min_y} {max_y}");
        // A quarter arc in the first quadrant keeps its own box, not the
        // circle's: (0,2) to (2,0) via the +45 degree point.
        let quarter = BulgeArc {
            center: p(0.0, 0.0),
            radius: 2.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            sweep: std::f64::consts::FRAC_PI_2,
        };
        let (min_x, min_y, max_x, max_y) = quarter.bounds();
        assert!(close(min_x, 0.0) && close(max_x, 2.0), "{min_x} {max_x}");
        assert!(close(min_y, 0.0) && close(max_y, 2.0), "{min_y} {max_y}");
    }

    #[test]
    fn is_sane_angle_screens_what_no_file_can_mean() {
        assert!(is_sane_angle(0.0));
        assert!(is_sane_angle(-std::f64::consts::TAU));
        assert!(is_sane_angle(MAX_ANGLE));
        assert!(!is_sane_angle(MAX_ANGLE * 1.001));
        assert!(!is_sane_angle(1.0e20));
        assert!(!is_sane_angle(f64::NAN));
        assert!(!is_sane_angle(f64::INFINITY));
    }

    #[test]
    fn an_ellipse_arc_runs_the_parameter_range_not_the_whole_ellipse() {
        // Major axis (20, 0), ratio 0.5, so the minor axis vector is
        // (0, 10): the point at parameter t is (20 cos t, 10 sin t).
        let half = EllipseArc {
            center: p(0.0, 0.0),
            major: p(20.0, 0.0),
            ratio: 0.5,
            start_param: 0.0,
            end_param: std::f64::consts::PI,
        };
        assert!(close(half.sweep(), std::f64::consts::PI));
        assert!(!half.is_full());
        let start = half.point_at_param(0.0);
        let end = half.point_at_param(std::f64::consts::PI);
        assert!(close(start.x, 20.0) && close(start.y, 0.0));
        assert!(close(end.x, -20.0) && close(end.y, 0.0));
        // The upper half only: y from 0 (both ends) to 10 (t = pi/2).
        let (min_x, min_y, max_x, max_y) = half.bounds();
        assert!(close(min_x, -20.0) && close(max_x, 20.0), "{min_x} {max_x}");
        assert!(close(min_y, 0.0) && close(max_y, 10.0), "{min_y} {max_y}");

        // A quarter of the same ellipse, pi/2 .. pi: x -20..0, y 0..10.
        let quarter = EllipseArc {
            start_param: std::f64::consts::FRAC_PI_2,
            ..half
        };
        let (min_x, min_y, max_x, max_y) = quarter.bounds();
        assert!(close(min_x, -20.0) && close(max_x, 0.0), "{min_x} {max_x}");
        assert!(close(min_y, 0.0) && close(max_y, 10.0), "{min_y} {max_y}");

        // AutoCAD writes a closed ellipse as 0 .. 2*pi (and a wrapped
        // sweep, 2*pi .. 3*pi, is the half again -- both forms occur in
        // samples/AutoCADSamples3.dwg).
        let full = EllipseArc {
            start_param: 0.0,
            end_param: std::f64::consts::TAU,
            ..half
        };
        assert!(full.is_full());
        let (min_x, min_y, max_x, max_y) = full.bounds();
        assert!(close(min_x, -20.0) && close(max_x, 20.0));
        assert!(close(min_y, -10.0) && close(max_y, 10.0));
        let wrapped = EllipseArc {
            start_param: std::f64::consts::TAU,
            end_param: 3.0 * std::f64::consts::PI,
            ..half
        };
        assert!(close(wrapped.sweep(), std::f64::consts::PI));
        let (_, min_y, _, max_y) = wrapped.bounds();
        assert!(close(min_y, 0.0) && close(max_y, 10.0), "{min_y} {max_y}");

        // A major axis along +y turns the frame a quarter turn: the point
        // at t is (0,20) cos t + (-10,0) sin t, so 0 .. pi is the left
        // half, x -10..0 and y -20..20.
        let turned = EllipseArc {
            center: p(0.0, 0.0),
            major: p(0.0, 20.0),
            ratio: 0.5,
            start_param: 0.0,
            end_param: std::f64::consts::PI,
        };
        let (min_x, min_y, max_x, max_y) = turned.bounds();
        assert!(close(min_x, -10.0) && close(max_x, 0.0), "{min_x} {max_x}");
        assert!(close(min_y, -20.0) && close(max_y, 20.0), "{min_y} {max_y}");
    }

    #[test]
    fn a_closed_two_vertex_bulged_polyline_encloses_its_real_area() {
        // AutoCAD's DONUT: two vertices 100 apart with bulges of 1 each is
        // a circle of radius 50, so the area is pi * 50^2 = 7853.981634 and
        // the perimeter is 2 * pi * 50 = 314.159265. Both numbers come from
        // the circle, not from this crate. The guard used to be a flat
        // "fewer than three segments encloses nothing", which is true of
        // straight edges and false of arcs.
        let verts = [p(0.0, 0.0), p(100.0, 0.0)];
        let bulges = [1.0, 1.0];
        let area = polyline_area(&verts, &bulges, true);
        assert!(
            close(area, std::f64::consts::PI * 2500.0),
            "area came out {area}"
        );
        assert!(
            close(
                polyline_length(&verts, &bulges, true),
                std::f64::consts::TAU * 50.0
            ),
            "perimeter"
        );
        // Orientation still reads: two negative bulges trace the same
        // circle clockwise.
        assert!(polyline_signed_area(&verts, &bulges, true) > 0.0);
        assert!(polyline_signed_area(&verts, &[-1.0, -1.0], true) < 0.0);

        // Two *straight* segments still enclose nothing -- there and back
        // along the same line.
        assert!(close(polyline_area(&verts, &[0.0, 0.0], true), 0.0));
    }

    #[test]
    fn a_half_donut_is_half_the_circle() {
        // One bulge of 1, one of 0: a semicircle closed by its diameter,
        // pi * 50^2 / 2 = 3926.990817.
        let area = polyline_area(&[p(0.0, 0.0), p(100.0, 0.0)], &[1.0, 0.0], true);
        assert!(
            close(area, std::f64::consts::PI * 1250.0),
            "area came out {area}"
        );
    }
}
