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
        let (a0, a1) = if self.sweep >= 0.0 {
            (self.start_angle, self.start_angle + self.sweep)
        } else {
            (self.end_angle, self.end_angle - self.sweep)
        };
        // Every multiple of 90 degrees between a0 and a1 (counter-clockwise).
        let first = (a0 / (tau / 4.0)).ceil();
        let mut k = first;
        while k * (tau / 4.0) <= a1 + 1e-12 {
            let angle = k * (tau / 4.0);
            take(Point2D {
                x: self.center.x + self.radius * angle.cos(),
                y: self.center.y + self.radius * angle.sin(),
            });
            k += 1.0;
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
    if segments.len() < 3 {
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
        let (_, min_y, _, max_y) = arc.bounds();
        assert!(close(max_y, arc.center.y + arc.radius) || max_y <= 0.0 + 1e-9);
        assert!(close(min_y, arc.center.y - arc.radius), "{min_y}");
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
}
