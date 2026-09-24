//! A polyface mesh read through this crate's DXF path: the wireframe of its
//! faces, and the edges it cannot draw counted rather than dropped.

use uncad::model::Entity;
use uncad::Format;

/// A polyface mesh: positions (70 = 192) and faces (70 = 128) in one VERTEX
/// chain. Each face's corners become edges in order and back to the first;
/// a negative index (an invisible edge) is the same corner, a 0 is an unused
/// one, and an edge to an index past the positions is counted as one that
/// could not be drawn.
#[test]
fn a_polyface_mesh_reads_as_the_wireframe_of_its_faces() {
    // The subclass markers and handles the DXF importer needs to tell the
    // two kinds of VERTEX apart.
    let handle = std::cell::Cell::new(0x30);
    let next = || {
        handle.set(handle.get() + 1);
        format!("{:X}", handle.get())
    };
    let position = |x: u8, y: u8| {
        format!(
            "  0
VERTEX
  5
{}
100
AcDbEntity
  8
0
100
AcDbVertex
100
AcDbPolyFaceMeshVertex
 10
{x}
 20
{y}
 30
0
 70
192
",
            next()
        )
    };
    let face = |a: i32, b: i32, c: i32, d: i32| {
        format!(
            "  0
VERTEX
  5
{}
100
AcDbEntity
  8
0
100
AcDbFaceRecord
 10
0
 20
0
 30
0
 70
128
 71
{a}
 72
{b}
 73
{c}
 74
{d}
",
            next()
        )
    };
    let text = [
        "  0
SECTION
  2
HEADER
  9
$ACADVER
  1
AC1015
  0
ENDSEC
  0
SECTION
  2
ENTITIES
  0
POLYLINE
  5
2A
100
AcDbEntity
  8
0
100
AcDbPolyFaceMesh
 66
1
 10
0
 20
0
 30
0
 70
64
 71
4
 72
3
"
        .to_string(),
        position(0, 0),
        position(1, 0),
        position(1, 1),
        position(0, 1),
        // A quad whose last edge is invisible, then a triangle with an
        // unused fourth corner, then a face naming a fifth position.
        face(1, 2, 3, -4),
        face(1, 3, 4, 0),
        face(1, 5, 0, 0),
        format!(
            "  0
SEQEND
  5
{}
100
AcDbEntity
  8
0
  0
ENDSEC
  0
EOF
",
            next()
        ),
    ]
    .concat();
    let db = uncad::parse_bytes(text.as_bytes(), Format::Dxf).unwrap();
    let Entity::PolylinePFace(mesh) = &db.entities[0] else {
        panic!("a polyface mesh, got {:?}", db.entities[0]);
    };
    let xy = |e: &[uncad::model::Point3D; 2]| ((e[0].x, e[0].y), (e[1].x, e[1].y));
    let edges: Vec<_> = mesh.wireframe_edges.iter().map(xy).collect();
    assert_eq!(
        edges,
        [
            ((0.0, 0.0), (1.0, 0.0)),
            ((1.0, 0.0), (1.0, 1.0)),
            ((1.0, 1.0), (0.0, 1.0)),
            ((0.0, 1.0), (0.0, 0.0)),
            ((0.0, 0.0), (1.0, 1.0)),
            ((1.0, 1.0), (0.0, 1.0)),
            ((0.0, 1.0), (0.0, 0.0)),
        ]
    );
    // The third face's two edges both touch the fifth position.
    assert_eq!(mesh.skipped_edges, 2);
}
