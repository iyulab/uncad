//! Polyline bulges and the OCS transform (docs/VLM_EXPORT_DESIGN.md, P4)
//! against real files: the corpus's `example_2000.dwg` has two revision
//! clouds -- LWPOLYLINEs whose every segment is a 110-degree arc -- and its
//! DXF twin states each bulge in group 42, which is the ground truth here.
//! The project's own `mirrored_ocs_r2000.dxf` covers the world-coordinate
//! side in `tests/fixtures.rs`; this file checks what the renderer and the
//! JSON make of the same data.

use uncad::model::LwPolylineEntity;
use uncad::Entity;

const EXAMPLE_2000_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);
const EXAMPLE_2000_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);
const MIRRORED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mirrored_ocs_r2000.dxf"
);
const MIRRORED_BULGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mirrored_bulge_r2000.dxf"
);

/// The small revision cloud in model space (the other one has 3635 vertices).
const REVCLOUD: &str = "156";

fn lwpolyline<'a>(db: &'a uncad::CadDatabase, handle: &str) -> &'a LwPolylineEntity {
    db.entities
        .iter()
        .find_map(|e| match e {
            Entity::LwPolyline(p) if p.common.handle == handle => Some(p),
            _ => None,
        })
        .unwrap_or_else(|| panic!("LWPOLYLINE {handle} is in the drawing"))
}

/// Group 42 of one LWPOLYLINE, read straight out of the DXF text.
fn bulges_from_dxf_text(handle: &str) -> Vec<f64> {
    let bytes = std::fs::read(EXAMPLE_2000_DXF).expect("corpus DXF is readable");
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i] == "0" && lines[i + 1] == "LWPOLYLINE" {
            let mut found = false;
            let mut bulges = Vec::new();
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                match lines[j] {
                    "5" if lines[j + 1] == handle => found = true,
                    "42" => bulges.push(lines[j + 1].parse::<f64>().expect("a bulge")),
                    _ => {}
                }
                j += 2;
            }
            if found {
                return bulges;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    panic!("LWPOLYLINE {handle} is in the DXF text");
}

#[test]
fn the_revision_cloud_reads_every_bulge_autocad_wrote() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD);
    let expected = bulges_from_dxf_text(REVCLOUD);
    assert_eq!(cloud.vertices.len(), 25);
    assert_eq!(cloud.bulges.len(), expected.len());
    for (got, want) in cloud.bulges.iter().zip(&expected) {
        assert!((got - want).abs() < 1e-9, "{got} vs DXF 42 {want}");
    }
    // Every arc of a revision cloud bulges the same way: 110 degrees,
    // clockwise (negative bulge) here.
    let sweep = 4.0 * cloud.bulges[0].atan();
    assert!(
        (sweep.to_degrees() + 110.0).abs() < 1e-6,
        "{}",
        sweep.to_degrees()
    );
    assert!(cloud.closed);
    assert!(cloud.extrusion.z > 0.0 && cloud.elevation == 0.0);
    assert!(cloud.widths.is_empty() && cloud.const_width == 0.0);
}

#[test]
fn the_revision_clouds_length_is_the_sum_of_its_arcs() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD);
    // Independently: chord c and bulge b give r = c (1 + b^2) / (4 |b|) and
    // the arc length r * |4 atan b|, summed over the 25 closing segments.
    let n = cloud.vertices.len();
    let mut expected = 0.0;
    for i in 0..n {
        let (a, b) = (cloud.vertices[i], cloud.vertices[(i + 1) % n]);
        let chord = (b.x - a.x).hypot(b.y - a.y);
        let bulge = cloud.bulges[i].abs();
        let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge);
        expected += radius * 4.0 * bulge.atan();
    }
    let length = cloud.length();
    assert!(
        (length - expected).abs() < 1e-6 * expected,
        "{length} vs {expected}"
    );
    // Every segment is a 110-degree arc, so the ratio of arc length to
    // chord is the same for all of them: theta / (2 sin(theta / 2)).
    let chords = uncad::geom::polyline_length(&cloud.vertices, &[], true);
    let theta = 110f64.to_radians();
    let ratio = theta / (2.0 * (theta / 2.0).sin());
    assert!(
        (length / chords - ratio).abs() < 1e-9,
        "{length} / {chords} vs {ratio}"
    );
    // A cloud bulging outward encloses more than its vertex polygon.
    let area = cloud.area().expect("closed outline");
    let polygon = uncad::geom::polyline_area(&cloud.vertices, &[], true);
    assert!(area > polygon, "{area} vs polygon {polygon}");
}

#[test]
fn an_open_polylines_area_ignores_the_bulge_on_its_last_vertex() {
    // AutoCAD keeps a bulge on the last vertex of an open polyline (after
    // BREAK or TRIM, say); it belongs to no segment. The right triangle
    // (0,0) -> (10,0) -> (10,10) closed by a straight segment for the area
    // is 10 * 10 / 2 = 50 whatever that bulge says; a semicircle (bulge 1)
    // on the closing chord would have added pi * (5 sqrt 2)^2 / 2 =
    // 78.54. Open, the length is the two drawn sides: 20.
    let mut p: LwPolylineEntity = {
        let db = uncad::parse(MIRRORED).expect("fixture must parse");
        lwpolyline(&db, "21").clone()
    };
    p.vertices = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
        .into_iter()
        .map(|(x, y)| uncad::model::Point2D { x, y })
        .collect();
    p.closed = false;
    for bulge in [1.0, -1.0, 669.19] {
        p.bulges = vec![0.0, 0.0, bulge];
        let area = p.area().expect("three vertices");
        assert!((area - 50.0).abs() < 1e-9, "bulge {bulge}: {area}");
        assert!(p.signed_area() > 0.0, "counter-clockwise");
        assert!((p.length() - 20.0).abs() < 1e-9);
    }
    // Flagged closed, the same bulge is the third, real segment.
    p.closed = true;
    p.bulges = vec![0.0, 0.0, 1.0];
    let area = p.area().unwrap();
    assert!(
        (area - (50.0 + std::f64::consts::PI * 50.0 / 2.0)).abs() < 1e-9,
        "{area}"
    );
}

#[test]
fn bulges_render_as_svg_arcs_and_survive_json() {
    let db = uncad::parse(MIRRORED).expect("fixture must parse");
    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    // Handle 21's 90-degree arc from (100,0) to (100,50): radius
    // 50 sqrt(2) / 2, small arc, positive bulge = counter-clockwise, which
    // stays counter-clockwise on screen since the canvas is the world with
    // y negated -- SVG sweep flag 0, as the ARC branch emits. (Sweep flag 1
    // would put the centre at (125,25) and bend the arc into the
    // rectangle.)
    assert!(svg.contains(" A 35.355"), "{svg}");
    assert!(svg.contains("0 0 0 100 -50"), "{svg}");
    // The mirrored rectangle (handle 20) is drawn at negative x.
    assert!(svg.contains("-100,"), "{svg}");

    let json = db
        .to_json(uncad::ToJsonOptions::default())
        .expect("serializes");
    assert!(
        json.contains("\"bulges\":[0.0,0.41421356,0.0,0.0]"),
        "{json}"
    );
    let back: uncad::CadDatabase = serde_json::from_str(&json).expect("deserializes");
    let (a, b) = (lwpolyline(&db, "21"), lwpolyline(&back, "21"));
    assert_eq!(a, b);
    assert_eq!(lwpolyline(&db, "20"), lwpolyline(&back, "20"));

    // 0.2.0 JSON, without the new fields, still loads.
    let mut value = serde_json::to_value(Entity::LwPolyline(a.clone())).expect("serializes");
    let object = value.as_object_mut().expect("an object");
    for key in ["bulges", "widths", "const_width", "elevation", "extrusion"] {
        assert!(object.remove(key).is_some(), "{key} is a top-level field");
    }
    let e: Entity = serde_json::from_value(value).expect("0.2.0 shape loads");
    let Entity::LwPolyline(p) = e else {
        panic!("an LWPOLYLINE");
    };
    assert!(p.bulges.is_empty());
    assert_eq!(p.extrusion, uncad::geom::WORLD_Z);
}

#[test]
fn the_revision_cloud_rasterizes() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD).clone();
    let mut only = uncad::CadDatabase::new(vec![Entity::LwPolyline(cloud)], db.tables.clone());
    only.header = db.header.clone();
    let svg = only.to_svg(uncad::ToSvgOptions::default()).svg;
    assert_eq!(svg.matches(" A ").count(), 25, "one arc per segment");
    let png = only
        .to_png(uncad::ToPngOptions::default())
        .expect("rasterizes");
    assert!(png.width > 0 && png.height > 0 && !png.png.is_empty());
}

/// An 8-bit RGB PNG decoded to a darkness test per world point.
struct Raster {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    view_box: uncad::ViewBox,
    ppu: f64,
}

impl Raster {
    /// `db` rendered over exactly `rect` at `ppu` pixels per unit, no
    /// lattice, no padding.
    fn of(db: &uncad::CadDatabase, rect: uncad::Rect, ppu: f64) -> Raster {
        let result = db
            .to_png(uncad::ToPngOptions {
                svg: uncad::ToSvgOptions {
                    crop: uncad::CropMode::Fixed(rect),
                    padding: Some(0.0),
                    ..Default::default()
                },
                size: uncad::PngSize::PxPerUnit(ppu),
                lattice: 0,
                ..Default::default()
            })
            .expect("rasterizes");
        let decoder = png::Decoder::new(std::io::Cursor::new(result.png));
        let mut reader = decoder.read_info().expect("valid PNG");
        let mut rgb = vec![0; reader.output_buffer_size().expect("a frame size")];
        let info = reader.next_frame(&mut rgb).expect("decodable");
        rgb.truncate(info.buffer_size());
        Raster {
            width: info.width as usize,
            height: info.height as usize,
            rgb,
            view_box: result.view_box,
            ppu: result.px_per_unit,
        }
    }

    /// Anything the 1.25 px stroke touched: an anti-aliased hairline
    /// leaves grey (130-190) rather than black pixels at a few px/unit.
    fn dark_px(&self, x: usize, y: usize) -> bool {
        self.rgb[(y * self.width + x) * 3] < 200
    }

    /// Whether any pixel within `radius_px` of the world point is dark.
    fn ink_near(&self, wx: f64, wy: f64, radius_px: i64) -> bool {
        let (px, py) = self.view_box.world_to_px(wx, wy, self.ppu);
        let (px, py) = (px.round() as i64, py.round() as i64);
        for dy in -radius_px..=radius_px {
            for dx in -radius_px..=radius_px {
                let (x, y) = (px + dx, py + dy);
                if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
                    continue;
                }
                if self.dark_px(x as usize, y as usize) {
                    return true;
                }
            }
        }
        false
    }

    /// Dark pixels whose world y is above / below `chord_y` (a band of
    /// `band_px` around the chord itself is left out).
    fn ink_above_and_below(&self, chord_y: f64, band_px: usize) -> (usize, usize) {
        let (_, chord_py) = self.view_box.world_to_px(0.0, chord_y, self.ppu);
        let chord_py = chord_py.round() as usize;
        let (mut above, mut below) = (0, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                if !self.dark_px(x, y) {
                    continue;
                }
                if y + band_px < chord_py {
                    above += 1;
                } else if y > chord_py + band_px {
                    below += 1;
                }
            }
        }
        (above, below)
    }
}

/// One open LWPOLYLINE in model space.
fn polyline_db(vertices: &[(f64, f64)], bulges: &[f64]) -> uncad::CadDatabase {
    use uncad::model::{EntityCommon, Point2D};
    let entities = vec![Entity::LwPolyline(LwPolylineEntity {
        common: EntityCommon {
            handle: "P".into(),
            layer: "0".into(),
            ..EntityCommon::default()
        },
        vertices: vertices.iter().map(|&(x, y)| Point2D { x, y }).collect(),
        closed: false,
        bulges: bulges.to_vec(),
        widths: Vec::new(),
        const_width: 0.0,
        elevation: 0.0,
        extrusion: uncad::geom::WORLD_Z,
    })];
    let mut tables = uncad::Tables::default();
    tables.block_records.insert(
        "*Model_Space".into(),
        uncad::tables::BlockRecord {
            name: "*Model_Space".into(),
            entities: entities.clone(),
        },
    );
    uncad::CadDatabase::new(entities, tables)
}

#[test]
fn a_semicircle_bulge_is_drawn_on_the_side_its_bulge_says() {
    // (0,0) -> (10,0) with bulge 1: a counter-clockwise semicircle, centre
    // (5,0), which sweeps from angle pi through 3pi/2 -- its apex is at
    // (5,-5), below the chord. That is where geom::bulge_arc puts it and
    // where the crop reserves space; the picture must agree.
    let arc = uncad::geom::bulge_arc(
        uncad::model::Point2D { x: 0.0, y: 0.0 },
        uncad::model::Point2D { x: 10.0, y: 0.0 },
        1.0,
    )
    .expect("an arc");
    let apex = arc.point_at(0.5);
    assert!(
        (apex.x - 5.0).abs() < 1e-9 && (apex.y + 5.0).abs() < 1e-9,
        "{apex:?}"
    );

    let ccw = polyline_db(&[(0.0, 0.0), (10.0, 0.0)], &[1.0, 0.0]);
    let raster = Raster::of(&ccw, uncad::Rect::new(0.0, -6.0, 10.0, 6.0), 20.0);
    let (above, below) = raster.ink_above_and_below(0.0, 2);
    assert!(
        above == 0 && below > 100,
        "ccw semicircle: {above} px above the chord, {below} below"
    );
    assert!(raster.ink_near(5.0, -5.0, 2));
    assert!(!raster.ink_near(5.0, 5.0, 2));

    // The mirror image: a negative bulge is a clockwise arc, apex (5,5).
    let cw = polyline_db(&[(0.0, 0.0), (10.0, 0.0)], &[-1.0, 0.0]);
    let raster = Raster::of(&cw, uncad::Rect::new(0.0, -6.0, 10.0, 6.0), 20.0);
    let (above, below) = raster.ink_above_and_below(0.0, 2);
    assert!(
        above > 100 && below == 0,
        "cw semicircle: {above} px above the chord, {below} below"
    );
}

#[test]
fn the_fixture_arcs_bulge_outward_and_a_mirrored_ocs_flips_the_bulge_sign() {
    // mirrored_ocs_r2000.dxf handle 21 (extrusion +Z): the 90-degree arc
    // from (100,0) to (100,50) has centre (75,25) and apex (110.355,25), so
    // the polyline reaches x = 100 + 50 sqrt(2) / 2 - 25 = 110.355 (README).
    let db = uncad::parse(MIRRORED).expect("fixture must parse");
    let bulged = lwpolyline(&db, "21").clone();
    let (_, _, max_x, _) =
        uncad::geom::polyline_bounds(&bulged.vertices, &bulged.bulges, false).unwrap();
    assert!((max_x - 110.35533906).abs() < 1e-6, "{max_x}");
    let mut only = uncad::CadDatabase::new(vec![Entity::LwPolyline(bulged)], db.tables.clone());
    only.header = db.header.clone();
    let raster = Raster::of(&only, uncad::Rect::new(80.0, 0.0, 120.0, 50.0), 10.0);
    assert!(raster.ink_near(110.355, 25.0, 2), "no ink at the apex");
    assert!(
        !raster.ink_near(89.645, 25.0, 2),
        "ink at the mirror image of the apex: the arc bends into the rectangle"
    );

    // mirrored_bulge_r2000.dxf: the same outline in an OCS with extrusion
    // (0,0,-1), whose OCS-to-world map is x -> -x. A reflection reverses
    // the turning direction, so the stored +0.41421356 becomes -0.41421356
    // for the world vertices (0,0) (-100,0) (-100,50) (0,50): a clockwise
    // arc from (-100,0) to (-100,50), centre (-75,25), apex (-110.355,25).
    // The ARC next to it traces the same arc through the ARC branch.
    let db = uncad::parse(MIRRORED_BULGE).expect("fixture must parse");
    let p = lwpolyline(&db, "20");
    assert_eq!(p.extrusion.z, -1.0);
    assert_eq!(p.bulges, [0.0, -0.41421356, 0.0, 0.0]);
    assert!(
        p.bulges.iter().all(|b| *b != 0.0 || !b.is_sign_negative()),
        "no -0.0: {:?}",
        p.bulges
    );
    assert_eq!(
        p.vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>(),
        [(0.0, 0.0), (-100.0, 0.0), (-100.0, 50.0), (0.0, 50.0)]
    );
    let (min_x, _, _, _) = uncad::geom::polyline_bounds(&p.vertices, &p.bulges, false).unwrap();
    assert!((min_x + 110.35533906).abs() < 1e-6, "{min_x}");
    let segments = uncad::geom::polyline_segments(&p.vertices, &p.bulges, false);
    let uncad::geom::Segment::Arc { arc, .. } = segments[1] else {
        panic!("the second segment is the arc");
    };
    assert!((arc.center.x + 75.0).abs() < 1e-6 && (arc.center.y - 25.0).abs() < 1e-6);
    let apex = arc.point_at(0.5);
    assert!((apex.x + 110.35533906).abs() < 1e-6 && (apex.y - 25.0).abs() < 1e-6);
    // The picture: the polyline alone, then the ARC alone, both with ink
    // at the apex and none at its mirror image inside the rectangle.
    for handle in ["20", "21"] {
        let entity = db
            .entities
            .iter()
            .find(|e| e.common().handle == handle)
            .unwrap()
            .clone();
        let mut only = uncad::CadDatabase::new(vec![entity], db.tables.clone());
        only.header = db.header.clone();
        let raster = Raster::of(&only, uncad::Rect::new(-120.0, 0.0, -80.0, 50.0), 10.0);
        assert!(
            raster.ink_near(-110.355, 25.0, 2),
            "{handle}: no ink at the apex"
        );
        assert!(
            !raster.ink_near(-89.645, 25.0, 2),
            "{handle}: ink inside the rectangle"
        );
    }
    let svg = db.to_svg(uncad::ToSvgOptions::default()).svg;
    assert!(
        svg.contains("0 0 1 -100 -50"),
        "negative bulge -> sweep flag 1: {svg}"
    );
}

#[test]
fn the_revision_cloud_scallops_point_outward() {
    let db = uncad::parse(EXAMPLE_2000_DWG).expect("corpus file must parse");
    let cloud = lwpolyline(&db, REVCLOUD).clone();
    let segments = uncad::geom::polyline_segments(&cloud.vertices, &cloud.bulges, true);
    assert_eq!(segments.len(), 25);
    let (min_x, min_y, max_x, max_y) =
        uncad::geom::polyline_bounds(&cloud.vertices, &cloud.bulges, true).unwrap();
    let mut only = uncad::CadDatabase::new(vec![Entity::LwPolyline(cloud)], db.tables.clone());
    only.header = db.header.clone();
    let rect = uncad::Rect::new(min_x - 5.0, min_y - 5.0, max_x + 5.0, max_y + 5.0);
    let raster = Raster::of(&only, rect, 4.0);
    // Every arc's midpoint (from the bulge, so on the outward side: the
    // cloud's area exceeds its vertex polygon's) carries ink, and its
    // reflection across the chord -- inside the polygon, where a cloud
    // drawn with inverted arcs would put the scallop -- does not.
    for segment in &segments {
        let uncad::geom::Segment::Arc { from, to, arc, .. } = segment else {
            panic!("every segment of the cloud is an arc");
        };
        let apex = arc.point_at(0.5);
        let mid = ((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        let inward = (2.0 * mid.0 - apex.x, 2.0 * mid.1 - apex.y);
        assert!(
            raster.ink_near(apex.x, apex.y, 2),
            "no ink at the apex {apex:?}"
        );
        assert!(
            !raster.ink_near(inward.0, inward.1, 2),
            "ink at {inward:?}, the mirror image of the apex {apex:?}"
        );
    }
}
