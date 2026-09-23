//! What runs between a polyline's vertices -- its bulges, its widths, its
//! constant width -- and its elevation and normal, read from corpus drawings
//! in both of their formats: the DXF twin states each of them as a group,
//! so the two readers have to agree.

use std::collections::BTreeMap;

use uncad::model::{LwPolylineEntity, SegmentWidth};
use uncad::Entity;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/"
);

/// The corpus drawings that have both formats and polylines in them.
const TWINS: [&str; 4] = [
    "example_2000",
    "example_2004",
    "example_r13",
    "2000/PolyLine2D",
];

fn parse(twin: &str, extension: &str) -> uncad::CadDatabase {
    let path = format!("{CORPUS}{twin}.{extension}");
    uncad::parse(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every LWPOLYLINE and POLYLINE_2D the drawing's blocks own, by handle.
fn polylines(db: &uncad::CadDatabase) -> BTreeMap<String, LwPolylineEntity> {
    let mut out = BTreeMap::new();
    for record in db.tables.block_records.values() {
        for e in &record.entities {
            if let Entity::LwPolyline(p) | Entity::Polyline2D(p) = e {
                let uncad::model::Ref::Resolved(handle) = &p.common.source_handle else {
                    panic!("an R2000 entity has a handle");
                };
                out.insert(handle.clone(), p.clone());
            }
        }
    }
    out
}

#[test]
fn the_dwg_and_its_dxf_twin_agree_on_what_runs_between_the_vertices() {
    let mut compared = 0;
    let mut bulged = 0;
    for twin in TWINS {
        let dwg = polylines(&parse(twin, "dwg"));
        let dxf = polylines(&parse(twin, "dxf"));
        assert_eq!(
            dwg.keys().collect::<Vec<_>>(),
            dxf.keys().collect::<Vec<_>>(),
            "{twin}"
        );
        for (handle, a) in &dwg {
            let b = &dxf[handle];
            let what = format!("{twin}: {handle}");
            // The DXF writes each number with at most 16 significant
            // digits, so the two agree to that, not to the last bit.
            let near = |x: &[f64], y: &[f64]| {
                x.len() == y.len()
                    && x.iter()
                        .zip(y)
                        .all(|(p, q)| (p - q).abs() <= 1e-12 * p.abs().max(q.abs()).max(1.0))
            };
            let xy = |p: &LwPolylineEntity| -> Vec<f64> {
                p.vertices.iter().flat_map(|v| [v.x, v.y]).collect()
            };
            let widths = |p: &LwPolylineEntity| -> Vec<f64> {
                p.widths.iter().flat_map(|w| [w.start, w.end]).collect()
            };
            assert!(
                near(&xy(a), &xy(b)),
                "{what}: {:?} vs {:?}",
                a.vertices,
                b.vertices
            );
            assert!(
                near(&a.bulges, &b.bulges),
                "{what}: {:?} vs {:?}",
                a.bulges,
                b.bulges
            );
            assert!(
                near(&widths(a), &widths(b)),
                "{what}: {:?} vs {:?}",
                a.widths,
                b.widths
            );
            assert!(
                near(&[a.const_width, a.elevation], &[b.const_width, b.elevation]),
                "{what}"
            );
            assert_eq!(a.extrusion, b.extrusion, "{what}");
            compared += 1;
            bulged += usize::from(!a.bulges.is_empty());
        }
    }
    assert!(compared >= 30, "{compared} polylines compared");
    assert!(bulged > 0, "no bulged polyline among the {compared}");
}

/// The dimension-arrow block `_ARCHTICK` draws a 0.15-wide tick. The DWG
/// stores that width on each of the tick's two vertices; the DXF writes it
/// once, as the POLYLINE's own default width (groups 40/41), and leaves it
/// out of both VERTEX records. Read as the vertices' own widths only, the
/// DXF's tick had no width at all.
#[test]
fn a_dxf_polyline_s_default_width_is_the_width_of_the_vertices_that_state_none() {
    for extension in ["dwg", "dxf"] {
        let db = parse("2000/PolyLine2D", extension);
        let tick = db.tables.block_records["_ARCHTICK"]
            .entities
            .iter()
            .find_map(|e| match e {
                Entity::Polyline2D(p) => Some(p),
                _ => None,
            })
            .expect("the tick is a POLYLINE_2D");
        let width = SegmentWidth {
            start: 0.15,
            end: 0.15,
        };
        assert_eq!(tick.widths, [width, width], "{extension}");
        assert_eq!(tick.const_width, 0.0, "{extension}");
    }
}
