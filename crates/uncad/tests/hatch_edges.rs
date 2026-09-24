//! A HATCH boundary's edges carry what the file states about them: both
//! ends of a straight edge, and a spline edge's degree, knots, control
//! points, weights and fit data.

use uncad::Format;
use uncad_model::model::{Entity, HatchBoundaryPath, HatchEdge, Point2D};

/// An R2010 DXF with one HATCH: one edge path of a line and a fitted,
/// rational spline.
fn drawing() -> Vec<u8> {
    let groups: &[(i32, &str)] = &[
        (0, "SECTION"),
        (2, "HEADER"),
        (9, "$ACADVER"),
        (1, "AC1024"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "HATCH"),
        (5, "2D"),
        (100, "AcDbEntity"),
        (8, "0"),
        (100, "AcDbHatch"),
        (10, "0"),
        (20, "0"),
        (30, "0"),
        (210, "0"),
        (220, "0"),
        (230, "1"),
        (2, "SOLID"),
        (70, "1"),
        (71, "0"),
        (91, "1"),
        (92, "1"),
        (93, "2"),
        (72, "1"),
        (10, "4"),
        (20, "0"),
        (11, "0"),
        (21, "0"),
        (72, "4"),
        (94, "3"),
        (73, "1"),
        (74, "0"),
        (95, "8"),
        (96, "4"),
        (40, "0"),
        (40, "0"),
        (40, "0"),
        (40, "0"),
        (40, "1"),
        (40, "1"),
        (40, "1"),
        (40, "1"),
        (10, "0"),
        (20, "0"),
        (10, "1"),
        (20, "2"),
        (10, "3"),
        (20, "2"),
        (10, "4"),
        (20, "0"),
        (42, "1"),
        (42, "0.5"),
        (42, "0.5"),
        (42, "1"),
        (97, "2"),
        (11, "0"),
        (21, "0"),
        (11, "4"),
        (21, "0"),
        (12, "1"),
        (22, "1"),
        (13, "1"),
        (23, "-1"),
        (97, "0"),
        (75, "0"),
        (76, "1"),
        (98, "0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    let mut text = String::new();
    for (code, value) in groups {
        text.push_str(&format!("{code:>3}\n{value}\n"));
    }
    text.into_bytes()
}

fn p(x: f64, y: f64) -> Point2D {
    Point2D { x, y }
}

#[test]
fn a_hatch_edge_path_carries_what_the_file_states_of_each_edge() {
    let db = uncad::parse_bytes(&drawing(), Format::Dxf).expect("the DXF reads");
    let hatch = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Hatch(h) => Some(h),
            _ => None,
        })
        .expect("a HATCH");
    let [HatchBoundaryPath::Edges(edges)] = hatch.boundary_paths.as_slice() else {
        panic!("{:?}", hatch.boundary_paths);
    };
    assert_eq!(
        edges[0],
        HatchEdge::Line {
            start: p(4.0, 0.0),
            end: p(0.0, 0.0)
        }
    );
    let HatchEdge::Spline {
        degree,
        rational,
        periodic,
        knots,
        control_points,
        weights,
        fit_points,
        start_tangent,
        end_tangent,
    } = &edges[1]
    else {
        panic!("{:?}", edges[1]);
    };
    assert_eq!((*degree, *rational, *periodic), (3, true, false));
    assert_eq!(knots, &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(
        control_points,
        &[p(0.0, 0.0), p(1.0, 2.0), p(3.0, 2.0), p(4.0, 0.0)]
    );
    // Group 42, one per control point. Upstream's DXF importer read none of
    // them; a local patch to the vendored copy does (docs/CAVEATS.md).
    assert_eq!(weights, &[1.0, 0.5, 0.5, 1.0]);
    assert_eq!(fit_points, &[p(0.0, 0.0), p(4.0, 0.0)]);
    assert_eq!(*start_tangent, Some(p(1.0, 1.0)));
    assert_eq!(*end_tangent, Some(p(1.0, -1.0)));
}
