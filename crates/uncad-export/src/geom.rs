//! The arithmetic the package's records carry and the model does not: a
//! polyline's length and area with its bulge arcs, whether an outline
//! crosses itself, a polygon's centroid, and the map from an entity's own
//! plane to the world.
//!
//! The model states a drawing and computes nothing but a block reference's
//! placement ([`uncad_model::Affine2`]); lengths and areas are this
//! consumer's derivations, and every one of them is exact arithmetic on
//! what the file states, not an estimate.

use uncad_model::model::{
    Confidence, EntityCommon, EntityId, InsertEntity, Origin, Point2D, Point3D, PolylineVertex, Ref,
};
use uncad_model::Affine2;

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
}

/// The arc from `from` to `to` with the given bulge -- the tangent of a
/// quarter of its included angle, positive counter-clockwise -- or `None`
/// for a straight segment (bulge 0 or not a number, or coincident points).
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

/// The segments of a polyline: a vertex's bulge applies to the segment
/// leaving it, and a closed polyline gets the segment from its last vertex
/// back to its first. A closing vertex that repeats the first one is
/// dropped first, so it does not add a zero-length segment.
pub fn polyline_segments(vertices: &[PolylineVertex], closed: bool) -> Vec<Segment> {
    let vertices = without_repeated_close(vertices, closed);
    let n = vertices.len();
    if n < 2 {
        return Vec::new();
    }
    let count = if closed { n } else { n - 1 };
    (0..count)
        .map(|i| {
            let (from, to) = (vertices[i].point, vertices[(i + 1) % n].point);
            let bulge = vertices[i].bulge;
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

fn without_repeated_close(vertices: &[PolylineVertex], closed: bool) -> &[PolylineVertex] {
    if closed && vertices.len() > 2 {
        let (first, last) = (vertices[0].point, vertices[vertices.len() - 1].point);
        if (first.x - last.x).abs() < 1e-9 && (first.y - last.y).abs() < 1e-9 {
            return &vertices[..vertices.len() - 1];
        }
    }
    vertices
}

/// The polyline's length (its perimeter when closed), arcs included.
pub fn polyline_length(vertices: &[PolylineVertex], closed: bool) -> f64 {
    polyline_segments(vertices, closed)
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
pub fn polyline_signed_area(vertices: &[PolylineVertex], closed: bool) -> f64 {
    let mut segments = polyline_segments(vertices, closed);
    if !closed && vertices.len() >= 2 {
        segments.push(Segment::Line {
            from: vertices[vertices.len() - 1].point,
            to: vertices[0].point,
        });
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

/// The enclosed area, unsigned, when the polyline can enclose one at all:
/// three or more vertices, or a closed pair (two arcs, AutoCAD's DONUT).
/// `None` otherwise.
pub fn polyline_area(vertices: &[PolylineVertex], closed: bool) -> Option<f64> {
    let enclosing = vertices.len() >= 3 || (closed && vertices.len() == 2);
    enclosing.then(|| polyline_signed_area(vertices, closed).abs())
}

/// Whether the straight-segment outline through `vertices` (closed) has no
/// two non-adjacent segments crossing. Quadratic in the vertex count --
/// callers bound it (the package tests at most 2000 vertices); arcs are
/// treated as their chords.
pub fn is_simple(vertices: &[Point2D]) -> bool {
    let vertices = if vertices.len() > 2 && vertices[0] == vertices[vertices.len() - 1] {
        &vertices[..vertices.len() - 1]
    } else {
        vertices
    };
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

/// Area-weighted centroid of a polygon's vertices (straight segments); the
/// vertices' mean when they enclose no area.
pub fn polygon_centroid(vertices: &[Point2D]) -> Point2D {
    let n = vertices.len();
    if n == 0 {
        return Point2D::default();
    }
    let (mut cx, mut cy, mut area) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (vertices[i], vertices[(i + 1) % n]);
        let cross = a.x * b.y - b.x * a.y;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
        area += cross;
    }
    if area.abs() < 1e-12 {
        let sx: f64 = vertices.iter().map(|p| p.x).sum();
        let sy: f64 = vertices.iter().map(|p| p.y).sum();
        return Point2D {
            x: sx / n as f64,
            y: sy / n as f64,
        };
    }
    Point2D {
        x: cx / (3.0 * area),
        y: cy / (3.0 * area),
    }
}

/// Whether `p` is inside the polygon `vertices` (even-odd rule).
pub fn point_in_polygon(p: Point2D, vertices: &[Point2D]) -> bool {
    let n = vertices.len();
    if n == 0 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (vertices[i], vertices[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// The map from an entity's own plane -- the object coordinate system whose
/// Z axis is `extrusion`, at height `elevation` in it -- to the world's (x,
/// y): the DXF reference's arbitrary axis algorithm, seen from above. The
/// identity for the usual normal (0, 0, 1), a mirror across the y axis for
/// (0, 0, -1).
///
/// Computed by the model's own [`Affine2::from_insert`] -- which applies
/// exactly this map to an INSERT's placement -- for an unscaled, unrotated
/// reference at the plane's origin, so the package and every other
/// consumer of the model agree on the algorithm to the bit. A normal that
/// is not a direction places the entity as (0, 0, 1) would, as it does
/// there.
pub fn plane_to_world(extrusion: Point3D, elevation: f64) -> Affine2 {
    let unit = InsertEntity {
        common: EntityCommon {
            id: EntityId::new(0),
            origin: Origin::Derived,
            confidence: Confidence::High,
            source_handle: Ref::Absent,
            layer: Ref::Absent,
            color_index: 256,
            true_color: None,
            invisible: false,
        },
        block_name: Ref::Absent,
        insertion_point: Point3D {
            x: 0.0,
            y: 0.0,
            z: elevation,
        },
        scale: Point3D {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
        rotation: 0.0,
        attribs: Vec::new(),
        extrusion,
    };
    Affine2::from_insert(&unit)
}

/// Whether `m` keeps shapes: a rotation, a uniform scale and possibly one
/// mirror. A circle stays a circle through it, a bulge stays a bulge (its
/// sign turned by a mirror), and lengths scale by `sqrt(|det|)`.
pub fn is_similarity(m: &Affine2) -> bool {
    let la = m.a.hypot(m.b);
    let lb = m.c.hypot(m.d);
    let dot = m.a * m.c + m.b * m.d;
    la > 0.0 && lb > 0.0 && dot.abs() <= 1e-9 * la * lb && (la - lb).abs() <= 1e-9 * la.max(lb)
}

/// `vertices` taken through `m`, when `m` is a similarity: the points
/// moved, a bulge's sign turned by a mirror (an arc that ran
/// counter-clockwise runs clockwise in a mirrored plane), the widths scaled.
/// `None` for a map that does not keep an arc an arc.
pub fn map_polyline(vertices: &[PolylineVertex], m: &Affine2) -> Option<Vec<PolylineVertex>> {
    if !is_similarity(m) {
        return None;
    }
    let flip = if m.determinant() < 0.0 { -1.0 } else { 1.0 };
    let scale = m.determinant().abs().sqrt();
    Some(
        vertices
            .iter()
            .map(|v| PolylineVertex {
                point: m.apply(v.point),
                bulge: v.bulge * flip,
                start_width: v.start_width * scale,
                end_width: v.end_width * scale,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2D {
        Point2D { x, y }
    }

    fn v(x: f64, y: f64, bulge: f64) -> PolylineVertex {
        PolylineVertex {
            point: p(x, y),
            bulge,
            start_width: 0.0,
            end_width: 0.0,
        }
    }

    fn vs(points: &[(f64, f64)], bulges: &[f64]) -> Vec<PolylineVertex> {
        points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| v(*x, *y, bulges.get(i).copied().unwrap_or(0.0)))
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_design_documents_worked_example() {
        // A 100 x 50 rectangle whose right edge is a 90-degree arc.
        let vertices = vs(
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)],
            &[0.0, 0.41421356, 0.0, 0.0],
        );
        let segments = polyline_segments(&vertices, true);
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
        assert!(close(polyline_length(&vertices, true), 305.536037));
        assert!(close(polyline_area(&vertices, true).unwrap(), 5356.747702));
        // The same outline clockwise: same area, negative sign. Reversed,
        // the arc is the segment leaving vertex 1 ((100,50) down to
        // (100,0)) and must bulge to the left of travel, i.e. negatively.
        let cw = vs(
            &[(0.0, 50.0), (100.0, 50.0), (100.0, 0.0), (0.0, 0.0)],
            &[0.0, -0.41421356, 0.0, 0.0],
        );
        assert!(close(polyline_signed_area(&cw, true), -5356.747702));
        let points: Vec<Point2D> = vertices.iter().map(|v| v.point).collect();
        assert!(is_simple(&points));
    }

    #[test]
    fn bulges_beyond_a_semicircle_put_the_centre_on_the_bulge_side() {
        // A 270-degree arc from (0,0) to (10,0): bulge = tan(67.5 deg).
        let bulge = (270f64 / 4.0).to_radians().tan();
        let arc = bulge_arc(p(0.0, 0.0), p(10.0, 0.0), bulge).unwrap();
        assert!(close(arc.sweep.to_degrees(), 270.0));
        // Centre below the chord (the arc bulges to the right of travel,
        // i.e. downward, and past a semicircle the centre is on that side):
        // (5, -5), radius 5 sqrt(2).
        assert!(
            close(arc.center.x, 5.0) && close(arc.center.y, -5.0),
            "{arc:?}"
        );
        assert!(close(arc.radius, 5.0 * 2f64.sqrt()), "{}", arc.radius);
    }

    #[test]
    fn open_polylines_and_degenerate_input() {
        let vertices = vs(&[(0.0, 0.0), (3.0, 4.0)], &[]);
        assert!(close(polyline_length(&vertices, false), 5.0));
        assert_eq!(polyline_segments(&vertices, true).len(), 2);
        assert_eq!(polyline_segments(&vs(&[(1.0, 1.0)], &[]), true).len(), 0);
        assert_eq!(polyline_area(&vertices, false), None, "two open vertices");
        assert!(bulge_arc(p(0.0, 0.0), p(0.0, 0.0), 1.0).is_none());
        assert!(bulge_arc(p(0.0, 0.0), p(1.0, 0.0), f64::NAN).is_none());
        // A repeated closing vertex does not add a zero-length segment.
        let repeated = vs(&[(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 0.0)], &[]);
        assert_eq!(polyline_segments(&repeated, true).len(), 3);
        assert!(close(polyline_area(&repeated, true).unwrap(), 6.0));
        assert!(close(polyline_length(&repeated, true), 12.0));
    }

    #[test]
    fn an_open_polylines_last_bulge_does_not_bend_the_closing_segment() {
        // The right triangle (0,0) -> (10,0) -> (10,10), open, with a bulge
        // left on its last vertex. Closed by a straight segment the area is
        // 10 * 10 / 2 = 50, and the last bulge applies to nothing. Bent, the
        // closing chord (10,10) -> (0,0) of length 10 sqrt(2) would carry a
        // semicircle of radius 5 sqrt(2), adding or taking pi * 50 / 2 =
        // 78.539816.
        for bulge in [1.0, -1.0, 669.19] {
            let vertices = vs(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], &[0.0, 0.0, bulge]);
            assert!(
                close(polyline_signed_area(&vertices, false), 50.0),
                "bulge {bulge}: {}",
                polyline_signed_area(&vertices, false)
            );
            // Length is unaffected either way: two straight sides.
            assert!(close(polyline_length(&vertices, false), 20.0));
        }
        // Flagged closed, the same bulge is the real closing segment.
        let vertices = vs(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], &[0.0, 0.0, 1.0]);
        assert!(close(
            polyline_signed_area(&vertices, true),
            50.0 + std::f64::consts::PI * 50.0 / 2.0
        ));
        // An open polyline drawn back to its first vertex: the bulge on the
        // vertex before the repeat is a real segment, the repeat's own is
        // not. The third segment (10,10) -> (0,0) with bulge -1 is a
        // semicircle inside the triangle, so 50 - 78.539816; the 5.0 on the
        // repeated vertex must count for nothing.
        let back_home = vs(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)],
            &[0.0, 0.0, -1.0, 5.0],
        );
        assert!(close(
            polyline_signed_area(&back_home, false),
            50.0 - std::f64::consts::PI * 50.0 / 2.0
        ));
    }

    #[test]
    fn self_intersection_is_detected() {
        let bowtie = [p(0.0, 0.0), p(10.0, 10.0), p(10.0, 0.0), p(0.0, 10.0)];
        assert!(!is_simple(&bowtie));
        let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
        assert!(is_simple(&square));
        // The same square with its closing vertex repeated.
        let repeated = [
            p(0.0, 0.0),
            p(10.0, 0.0),
            p(10.0, 10.0),
            p(0.0, 10.0),
            p(0.0, 0.0),
        ];
        assert!(is_simple(&repeated));
    }

    #[test]
    fn a_closed_two_vertex_bulged_polyline_encloses_its_real_area() {
        // AutoCAD's DONUT: two vertices 100 apart with bulges of 1 each is
        // a circle of radius 50, so the area is pi * 50^2 = 7853.981634 and
        // the perimeter is 2 * pi * 50 = 314.159265. Both numbers come from
        // the circle, not from this crate.
        let donut = vs(&[(0.0, 0.0), (100.0, 0.0)], &[1.0, 1.0]);
        let area = polyline_area(&donut, true).unwrap();
        assert!(close(area, std::f64::consts::PI * 2500.0), "{area}");
        assert!(close(
            polyline_length(&donut, true),
            std::f64::consts::TAU * 50.0
        ));
        // Orientation still reads: two negative bulges trace the same
        // circle clockwise.
        assert!(polyline_signed_area(&donut, true) > 0.0);
        let clockwise = vs(&[(0.0, 0.0), (100.0, 0.0)], &[-1.0, -1.0]);
        assert!(polyline_signed_area(&clockwise, true) < 0.0);
        // Two *straight* segments still enclose nothing -- there and back
        // along the same line.
        let straight = vs(&[(0.0, 0.0), (100.0, 0.0)], &[0.0, 0.0]);
        assert!(close(polyline_area(&straight, true).unwrap(), 0.0));
        // One bulge of 1, one of 0: a semicircle closed by its diameter,
        // pi * 50^2 / 2.
        let half = vs(&[(0.0, 0.0), (100.0, 0.0)], &[1.0, 0.0]);
        assert!(close(
            polyline_area(&half, true).unwrap(),
            std::f64::consts::PI * 1250.0
        ));
    }

    #[test]
    fn centroids_and_containment() {
        let square = [p(0.0, 0.0), p(4.0, 0.0), p(4.0, 2.0), p(0.0, 2.0)];
        let c = polygon_centroid(&square);
        assert!(close(c.x, 2.0) && close(c.y, 1.0), "{c:?}");
        assert!(point_in_polygon(p(1.0, 1.0), &square));
        assert!(!point_in_polygon(p(5.0, 1.0), &square));
        // No area: the mean of the vertices.
        let line = [p(0.0, 0.0), p(10.0, 0.0)];
        let c = polygon_centroid(&line);
        assert!(close(c.x, 5.0) && close(c.y, 0.0), "{c:?}");
    }

    #[test]
    fn a_mirrored_plane_turns_x_and_the_bulges_sign() {
        // (0, 0, -1): the arbitrary axis algorithm takes the OCS x axis to
        // the world's -x (Wy x N) and leaves y alone.
        let mirror = plane_to_world(
            Point3D {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            0.0,
        );
        let q = mirror.apply(p(3.0, 4.0));
        assert!(close(q.x, -3.0) && close(q.y, 4.0), "{q:?}");
        assert!(is_similarity(&mirror) && mirror.determinant() < 0.0);
        // A counter-clockwise half circle in the mirrored plane runs
        // clockwise in the world: same length and area, orientation turned.
        let ocs = vs(&[(0.0, 0.0), (10.0, 0.0)], &[1.0, 1.0]);
        let world = map_polyline(&ocs, &mirror).unwrap();
        assert!(world.iter().all(|v| v.bulge == -1.0));
        assert!(close(
            polyline_length(&world, true),
            polyline_length(&ocs, true)
        ));
        assert!(close(
            polyline_signed_area(&world, true),
            -polyline_signed_area(&ocs, true)
        ));
        // The usual normal is the identity, whatever the elevation.
        let flat = plane_to_world(
            Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            7.0,
        );
        assert_eq!(flat, Affine2::IDENTITY);
        // A tilted plane seen from above is no similarity: an arc in it is
        // an ellipse in the plan.
        let tilted = plane_to_world(
            Point3D {
                x: 1.0,
                y: 0.0,
                z: 1.0,
            },
            0.0,
        );
        assert!(!is_similarity(&tilted));
        assert!(map_polyline(&ocs, &tilted).is_none());
    }
}
