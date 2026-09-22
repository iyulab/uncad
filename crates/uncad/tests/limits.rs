//! A malformed drawing may cost a missing entity and a note saying so; it
//! may never cost the process.
//!
//! Every cap these tests exercise is named and documented in
//! `uncad::limits`. Each one stands between a number read straight out of
//! the file and either an allocation size or a loop bound, and each one is
//! here because an adversarial file made that number absurd.

use std::collections::BTreeMap;
use std::time::Instant;

use uncad::limits::{
    MAX_BLOCK_REFS, MAX_BLOCK_REF_DEPTH, MAX_ENTITY_POINTS, MAX_SVG_BODY_BYTES,
    MAX_WORLD_COORDINATE,
};
use uncad::model::{
    Entity, EntityCommon, HatchBoundaryPath, HatchEntity, HatchPatternLine, InsertEntity,
    LineEntity, LwPolylineEntity, Point2D, Point3D,
};
use uncad::tables::{BlockRecord, Tables};
use uncad::{parse_bytes, CadDatabase, Format, ToSvgOptions};

const EXAMPLE_2000: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dwg"
);

/// How far past [`MAX_SVG_BODY_BYTES`] a finished document may still be:
/// the cap is checked before each entity, so the last one drawn, the
/// `<g>` wrappers closing above it and the `<svg>` element itself all land
/// on top of it. Half as much again is generous for all three.
const SLACK: usize = MAX_SVG_BODY_BYTES / 2;

fn xy(x: f64, y: f64) -> Point2D {
    Point2D { x, y }
}

fn xyz(x: f64, y: f64) -> Point3D {
    Point3D { x, y, z: 0.0 }
}

fn common(handle: &str) -> EntityCommon {
    EntityCommon {
        handle: handle.to_string(),
        ..EntityCommon::default()
    }
}

fn line(handle: &str, x: f64, y: f64) -> Entity {
    Entity::Line(LineEntity {
        common: common(handle),
        start_point: xyz(x, y),
        end_point: xyz(x + 1.0, y + 1.0),
    })
}

fn insert(handle: &str, block: &str, x: f64) -> Entity {
    Entity::Insert(InsertEntity {
        common: common(handle),
        block_name: block.to_string(),
        insertion_point: xyz(x, 0.0),
        scale: Point3D {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
        rotation: 0.0,
        extrusion: Point3D {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        attribs: Vec::new(),
    })
}

fn block(name: &str, entities: Vec<Entity>) -> (String, BlockRecord) {
    (
        name.to_string(),
        BlockRecord {
            name: name.to_string(),
            entities,
        },
    )
}

/// A database whose model space is `entities` and whose block table is
/// `blocks`, with `*Model_Space` wired up the way a parsed file has it.
fn db(entities: Vec<Entity>, blocks: Vec<(String, BlockRecord)>) -> CadDatabase {
    let mut block_records: BTreeMap<String, BlockRecord> = blocks.into_iter().collect();
    block_records.insert(
        "*Model_Space".to_string(),
        BlockRecord {
            name: "*Model_Space".to_string(),
            entities: entities.clone(),
        },
    );
    CadDatabase::new(
        entities,
        Tables {
            block_records,
            ..Default::default()
        },
    )
}

// --- the reported failure ----------------------------------------------

/// One byte of `example_2000.dwg`, found by bisecting a fuzzed file's 290
/// mutated offsets down to the single one that matters.
///
/// It redirects the `CIRKLO_PUNKTOJ` block record's owned-entity chain so
/// that the block ends up holding eight INSERTs *of itself* beside its 50
/// drawable entities. Parsing still takes 0.03 s and reports nothing odd --
/// the damage only shows when something walks the block table.
///
/// Before the caps in `uncad::limits`, rendering that walk spent 3 minutes
/// growing one SVG string and then aborted the process on a
/// 12,074,460,607-byte reallocation (measured on the release build; the
/// fuzzed 290-flip original aborted at 12,068,794,615 bytes after 130 s).
const SELF_REF_OFFSET: usize = 130_005;
const SELF_REF_ORIGINAL: u8 = 0x80;
const SELF_REF_CORRUPT: u8 = 0x4B;

#[test]
fn a_block_record_corrupted_into_referencing_itself_renders_bounded_and_says_so() {
    let mut bytes = std::fs::read(EXAMPLE_2000).expect("the corpus DWG is readable");
    assert_eq!(
        bytes.get(SELF_REF_OFFSET).copied(),
        Some(SELF_REF_ORIGINAL),
        "the corpus file changed: byte {SELF_REF_OFFSET} is no longer the one this \
         regression was minimised against"
    );
    bytes[SELF_REF_OFFSET] = SELF_REF_CORRUPT;

    let drawing = parse_bytes(&bytes, Format::Dwg).expect("the corrupted file still parses");
    let self_referencing = drawing
        .tables
        .block_records
        .get("CIRKLO_PUNKTOJ")
        .expect("the damaged block record is there")
        .entities
        .iter()
        .filter(|e| matches!(e, Entity::Insert(i) if i.block_name == "CIRKLO_PUNKTOJ"))
        .count();
    assert!(
        self_referencing > 0,
        "this test is only meaningful while the flipped byte still makes the \
         block reference itself"
    );

    let started = Instant::now();
    let result = drawing.to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    assert!(
        result.svg.len() < MAX_SVG_BODY_BYTES + SLACK,
        "the document grew to {} bytes, past the {MAX_SVG_BODY_BYTES}-byte budget",
        result.svg.len()
    );
    assert!(
        result.limits.engaged(),
        "the render must say what it left out, not silently draw less"
    );
    assert!(
        result.limits.entities_dropped > 0 || result.limits.block_refs_dropped > 0,
        "a cap on the block walk is what should have stopped this: {:?}",
        result.limits
    );
    assert!(
        elapsed.as_secs() < 120,
        "bounded, but it took {elapsed:?} -- the budget is not doing its job"
    );
}

// --- the caps, one at a time -------------------------------------------

#[test]
fn a_block_that_references_itself_stops_at_the_output_budget() {
    // The shape the corrupted file above happens to produce, built by hand
    // so the cap is tested without depending on any one file: a block that
    // both draws something and references itself several times, so every
    // level of the walk adds to the document.
    let mut children: Vec<Entity> = (0..40).map(|i| line("C", i as f64, 0.0)).collect();
    children.extend((0..8).map(|i| insert("I", "R", i as f64)));
    let drawing = db(vec![insert("T", "R", 0.0)], vec![block("R", children)]);

    let started = Instant::now();
    let result = drawing.to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    assert!(
        result.svg.len() < MAX_SVG_BODY_BYTES + SLACK,
        "the document grew to {} bytes",
        result.svg.len()
    );
    assert!(
        result.limits.entities_dropped > 0,
        "the output budget is what should have stopped this: {:?}",
        result.limits
    );
    assert!(elapsed.as_secs() < 120, "took {elapsed:?}");
}

#[test]
fn a_chain_of_blocks_deeper_than_the_cap_stops_at_the_cap() {
    // B0 references B1 references B2 ... Each level draws one line, so the
    // count of lines in the document is exactly how many levels were
    // followed. Nothing here fans out, so the expansion budget cannot be
    // what stops it -- only the depth cap can.
    let depth = MAX_BLOCK_REF_DEPTH as usize * 3;
    let blocks: Vec<(String, BlockRecord)> = (0..depth)
        .map(|i| {
            let mut entities = vec![line("L", i as f64, 0.0)];
            if i + 1 < depth {
                entities.push(insert("I", &format!("B{}", i + 1), 0.0));
            }
            block(&format!("B{i}"), entities)
        })
        .collect();
    let drawing = db(vec![insert("T", "B0", 0.0)], blocks);

    let started = Instant::now();
    let result = drawing.to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    let drawn = result.svg.matches("<line ").count();
    assert!(
        drawn <= MAX_BLOCK_REF_DEPTH as usize,
        "{drawn} levels were drawn, past the {MAX_BLOCK_REF_DEPTH}-level cap"
    );
    assert!(
        drawn > 0,
        "the levels within the cap should still be drawn: {:?}",
        result.limits
    );
    assert_eq!(
        result.limits.block_refs_dropped, 1,
        "exactly the one reference past the cap should have been dropped"
    );
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
}

#[test]
fn a_block_fanning_out_below_the_depth_cap_stops_at_the_expansion_budget() {
    // Twelve self-references per level reaches 12^20 instantiations without
    // ever exceeding the depth cap, so this is the breadth the depth cap
    // cannot see. The block draws nothing, so the output budget cannot be
    // what stops it either.
    let children: Vec<Entity> = (0..12).map(|i| insert("I", "F", i as f64)).collect();
    let drawing = db(vec![insert("T", "F", 0.0)], vec![block("F", children)]);

    let started = Instant::now();
    let result = drawing.to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    assert!(
        result.limits.block_refs_dropped > 0,
        "the expansion budget is what should have stopped this: {:?}",
        result.limits
    );
    assert!(
        result.limits.entities_dropped == 0,
        "nothing was drawable, so the output budget cannot be what engaged: {:?}",
        result.limits
    );
    assert!(
        elapsed.as_secs() < 60,
        "{MAX_BLOCK_REFS} expansions took {elapsed:?}"
    );
}

#[test]
fn a_polyline_with_more_vertices_than_the_cap_is_left_out_whole() {
    let vertices: Vec<Point2D> = (0..=MAX_ENTITY_POINTS)
        .map(|i| xy(i as f64 * 0.001, (i % 7) as f64))
        .collect();
    assert!(vertices.len() > MAX_ENTITY_POINTS);
    let huge = Entity::LwPolyline(LwPolylineEntity {
        common: common("BIG"),
        vertices,
        closed: false,
        bulges: Vec::new(),
        widths: Vec::new(),
        const_width: 0.0,
        elevation: 0.0,
        extrusion: Point3D {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    });
    let drawing = db(vec![huge, line("OK", 0.0, 0.0)], Vec::new());

    let started = Instant::now();
    let result = drawing.to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    assert_eq!(
        result.limits.oversized_entities, 1,
        "the oversized polyline should have been screened: {:?}",
        result.limits
    );
    assert!(
        !result.svg.contains("<polyline"),
        "the screened polyline must not be in the document"
    );
    assert!(
        result.svg.contains("<line "),
        "the rest of the drawing must still be drawn"
    );
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
}

#[test]
fn a_hatch_whose_pattern_tile_dwarfs_the_shape_keeps_only_its_outline() {
    // A ten-unit square with a pattern spacing of 1e12. The spacing is the
    // SVG `<pattern>` tile's size in user units, and the rasterizer sizes
    // its pixmap for that tile at the filled element's device scale -- so
    // taken at face value this asks for a pixmap around 1e11 pixels on a
    // side, which is an allocation failure, not a picture.
    let square = vec![xy(0.0, 0.0), xy(10.0, 0.0), xy(10.0, 10.0), xy(0.0, 10.0)];
    let hatch = |offset: Point2D| {
        Entity::Hatch(HatchEntity {
            common: common("H"),
            boundary_paths: vec![HatchBoundaryPath::Polyline(square.clone())],
            solid_fill: false,
            gradient: None,
            pattern_lines: vec![HatchPatternLine {
                angle: 0.0,
                base_point: xy(0.0, 0.0),
                offset,
                dash_pattern: Vec::new(),
            }],
        })
    };

    // A sane spacing still tiles, so this test is about the absurd one.
    let sane = db(vec![hatch(xy(0.0, 0.5))], Vec::new()).to_svg(ToSvgOptions::default());
    assert!(sane.svg.contains("<pattern"), "a normal hatch still tiles");
    assert!(!sane.limits.engaged(), "{:?}", sane.limits);

    let started = Instant::now();
    let result = db(vec![hatch(xy(0.0, 1e12))], Vec::new()).to_svg(ToSvgOptions::default());
    let elapsed = started.elapsed();

    assert_eq!(
        result.limits.hatch_patterns_dropped, 1,
        "the absurd pattern should have been dropped: {:?}",
        result.limits
    );
    assert!(
        !result.svg.contains("<pattern"),
        "no tile that size may reach the rasterizer"
    );
    assert!(
        result.svg.contains("<path "),
        "the boundary outline must still be drawn"
    );
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
}

#[test]
fn a_hatch_the_cap_dropped_still_rasterizes_quickly() {
    // The same drawing all the way through `to_png`: dropping the pattern
    // is only worth anything if what reaches resvg is now cheap.
    let square = vec![xy(0.0, 0.0), xy(10.0, 0.0), xy(10.0, 10.0), xy(0.0, 10.0)];
    let drawing = db(
        vec![Entity::Hatch(HatchEntity {
            common: common("H"),
            boundary_paths: vec![HatchBoundaryPath::Polyline(square)],
            solid_fill: false,
            gradient: None,
            pattern_lines: vec![HatchPatternLine {
                angle: 0.0,
                base_point: xy(0.0, 0.0),
                offset: xy(0.0, 1e12),
                dash_pattern: Vec::new(),
            }],
        })],
        Vec::new(),
    );

    let started = Instant::now();
    let png = drawing
        .to_png(uncad::ToPngOptions::default())
        .expect("the hatch rasterizes");
    let elapsed = started.elapsed();

    assert_eq!(png.limits.hatch_patterns_dropped, 1);
    assert!(!png.png.is_empty());
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
}

#[test]
fn an_entity_at_an_absurd_coordinate_does_not_drag_the_viewbox_with_it() {
    // 1e150 is finite, so nothing about the number itself says no. What it
    // does say no to is everything derived from the extents: a fuzzed
    // `example_2000.dwg` with one such vertex produced a viewBox 1.45e150
    // units wide and a stroke width of 5e149 to match.
    let drawing = db(
        vec![
            line("NEAR", 0.0, 0.0),
            Entity::Line(LineEntity {
                common: common("FAR"),
                start_point: xyz(0.0, 0.0),
                end_point: xyz(1e150, 1e150),
            }),
        ],
        Vec::new(),
    );
    let result = drawing.to_svg(ToSvgOptions::default());

    assert!(
        result.view_box.width < MAX_WORLD_COORDINATE,
        "the viewBox is {} units wide",
        result.view_box.width
    );
    assert!(
        result.view_box.width < 100.0,
        "the sane line is 1 unit long; the viewBox came out {} wide",
        result.view_box.width
    );
    assert_eq!(
        result.svg.matches("<line ").count(),
        1,
        "the entity with the absurd endpoint should not be drawn"
    );
}

#[test]
fn a_bulge_that_makes_an_arc_of_absurd_radius_is_drawn_as_a_line() {
    // A bulge is tan(theta/4), so a bulge of 1e-160 over a hundred-unit
    // segment is an arc of radius ~1e238. Handing that to the rasterizer as
    // an SVG `A` command is what made one fuzzed drawing take longer than
    // five minutes to rasterize at *any* image size, 200 px included.
    let drawing = db(
        vec![Entity::LwPolyline(LwPolylineEntity {
            common: common("P"),
            vertices: vec![xy(0.0, 0.0), xy(100.0, 0.0), xy(100.0, 50.0)],
            closed: false,
            bulges: vec![1e-160, 0.0, 0.0],
            widths: Vec::new(),
            const_width: 0.0,
            elevation: 0.0,
            extrusion: Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        })],
        Vec::new(),
    );

    let svg = drawing.to_svg(ToSvgOptions::default()).svg;
    assert!(
        !svg.contains(" A "),
        "an arc of that radius is a straight line: {svg}"
    );

    let started = Instant::now();
    let png = drawing
        .to_png(uncad::ToPngOptions::default())
        .expect("it rasterizes");
    let elapsed = started.elapsed();
    assert!(!png.png.is_empty());
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
}
