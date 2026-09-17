//! JSON export of the parsed model -- [`CadDatabase::to_json`].
//!
//! The output is a direct serde serialization of [`CadDatabase`] (`entities` +
//! `tables`): exactly the model `to_svg()` renders from, so a consumer sees the
//! same drawing the SVG shows. It round-trips for any database whose `f64`
//! fields are all finite (everything `parse()` has produced so far):
//! `serde_json::from_str::<CadDatabase>` gives back a database equal
//! (`PartialEq`) to the one serialized. It is also reproducible -- `tables.*`
//! are sorted maps and entity order follows the file, so the same input
//! serializes to the same bytes every run.
//!
//! `serde` 1.x and `serde_json` 1.x are public dependencies of this crate (the
//! model derives their traits and `serde_json::Error` appears in
//! [`JsonError`]); bumping either major would be a breaking change.
//!
//! Shape notes for consumers:
//! - Every entity object carries a `"type"` tag holding the DXF name
//!   [`Entity::type_name`](crate::Entity::type_name) reports (`"LINE"`,
//!   `"LWPOLYLINE"`, `"3DSOLID"`, `"POLYLINE_PFACE"`, ...), so dispatching on
//!   `type` needs no knowledge of the Rust enum. The one exception is an entity
//!   type this crate does not convert: it is tagged `"UNKNOWN"` and carries the
//!   real DXF name in its `type_name` field.
//! - Points are objects (`{"x":..,"y":..}` / `{"x":..,"y":..,"z":..}`); angles
//!   are radians, as in the model.
//! - HATCH: each item of `boundary_paths` is `{"type":"POLYLINE","data":
//!   [pt,..]}` or `{"type":"EDGES","data":[edge,..]}`, and each edge is
//!   `{"type":"LINE"|"ARC"|"ELLIPSE"|"SPLINE", ...}` with the edge's own fields
//!   beside the tag. Upper-case like the entity tags, but these are path/edge
//!   kinds, not DXF entity names.
//! - `entities` holds what the drawing shows (model + paper space), while
//!   `tables.block_records` holds *every* block including those two, so a
//!   model-space entity appears twice. That duplication is the model's own, not
//!   a JSON artifact: read `entities` for the drawing and `block_records` for
//!   what an INSERT's `block_name` refers to.
//! - Likewise every INSERT's `attribs` are also present as top-level `ATTRIB`
//!   entities in `entities` (that is how `to_svg` draws them -- it ignores
//!   `attribs`); `block_records[..].entities` does not carry that duplication,
//!   so the two lists differ even for `*Model_Space`.
//! - `f64` values that are not finite serialize as `null` (serde_json's
//!   default) and such a document does not deserialize back. Nothing observed
//!   so far produces any, but a consumer should not assume every numeric field
//!   is a number.

use crate::CadDatabase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToJsonOptions {
    /// Indented, multi-line output instead of the compact single line.
    pub pretty: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum JsonError {
    /// `serde_json` refused the model. Not expected for anything `parse()`
    /// produces (every field is a plain number, string, bool, sequence, or
    /// map with string keys); kept as an error rather than a panic so a
    /// future model change cannot take the caller's process down.
    Serialize(serde_json::Error),
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonError::Serialize(e) => write!(f, "JSON serialization failed: {e}"),
        }
    }
}

impl std::error::Error for JsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JsonError::Serialize(e) => Some(e),
        }
    }
}

/// Serializes a parsed drawing to JSON text -- see the module doc for the
/// shape. `CadDatabase::to_json` is the public entry point; this is the
/// implementation behind it.
pub(crate) fn to_json(db: &CadDatabase, options: ToJsonOptions) -> Result<String, JsonError> {
    let text = if options.pretty {
        serde_json::to_string_pretty(db)
    } else {
        serde_json::to_string(db)
    };
    text.map_err(JsonError::Serialize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::tables::{BlockRecord, LayerRecord, Tables};
    use std::collections::BTreeMap;

    fn common(handle: &str) -> EntityCommon {
        EntityCommon {
            handle: handle.to_string(),
            layer: "0".to_string(),
            color_index: 256,
            true_color: Some(0x12_34_56),
        }
    }

    fn p3(x: f64, y: f64, z: f64) -> Point3D {
        Point3D { x, y, z }
    }

    fn p2(x: f64, y: f64) -> Point2D {
        Point2D { x, y }
    }

    /// One instance of every `Entity` variant. The `match` at the end
    /// has no wildcard arm, so adding a variant without adding it here (and
    /// to the tag table in `tags_match_type_name`) fails to compile.
    fn one_of_each() -> Vec<Entity> {
        let c = common("1A");
        let attrib = AttribEntity {
            common: c.clone(),
            start_point: p2(1.0, 2.0),
            text_height: 2.5,
            text: "TAG".to_string(),
            rotation: 0.1,
        };
        let ray = RayEntity {
            common: c.clone(),
            point: p3(0.0, 0.0, 0.0),
            vector: p3(1.0, 0.0, 0.0),
        };
        let lwpoly = LwPolylineEntity {
            common: c.clone(),
            vertices: vec![p2(0.0, 0.0), p2(1.0, 0.0), p2(1.0, 1.0)],
            closed: true,
        };
        let solid3d = Solid3DEntity {
            common: c.clone(),
            wireframe_edges: vec![[p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0)]],
        };
        let all = vec![
            Entity::Line(LineEntity {
                common: c.clone(),
                start_point: p3(0.0, 0.0, 0.0),
                end_point: p3(1.0, 1.0, 0.0),
            }),
            Entity::Circle(CircleEntity {
                common: c.clone(),
                center: p3(0.0, 0.0, 0.0),
                radius: 1.0,
            }),
            Entity::Text(TextEntity {
                common: c.clone(),
                start_point: p2(0.0, 0.0),
                text_height: 2.5,
                text: "hi".to_string(),
                rotation: 0.2,
            }),
            Entity::LwPolyline(lwpoly.clone()),
            Entity::Arc(ArcEntity {
                common: c.clone(),
                center: p3(0.0, 0.0, 0.0),
                radius: 1.0,
                start_angle: 0.0,
                end_angle: 1.0,
            }),
            Entity::Ellipse(EllipseEntity {
                common: c.clone(),
                center: p3(0.0, 0.0, 0.0),
                major_axis_endpoint: p3(2.0, 0.0, 0.0),
                axis_ratio: 0.5,
                start_angle: 0.1,
                end_angle: 1.5,
            }),
            Entity::Point(PointEntity {
                common: c.clone(),
                position: p3(0.0, 0.0, 0.0),
            }),
            Entity::Solid(SolidEntity {
                common: c.clone(),
                corner1: p2(0.0, 0.0),
                corner2: p2(1.0, 0.0),
                corner3: p2(1.0, 1.0),
                corner4: p2(0.0, 1.0),
            }),
            Entity::Ray(ray.clone()),
            Entity::XLine(ray),
            Entity::Insert(InsertEntity {
                common: c.clone(),
                block_name: "DOOR".to_string(),
                insertion_point: p3(0.0, 0.0, 0.0),
                scale: p3(1.0, 1.0, 1.0),
                rotation: 0.25,
                attribs: vec![attrib.clone()],
            }),
            Entity::Attrib(attrib),
            Entity::Attdef(AttdefEntity {
                common: c.clone(),
                start_point: p2(0.0, 0.0),
                text_height: 2.5,
                default_value: "?".to_string(),
                rotation: 0.4,
            }),
            Entity::Viewport(ViewportEntity {
                common: c.clone(),
                center: p3(0.0, 0.0, 0.0),
                width: 10.0,
                height: 5.0,
            }),
            Entity::Face3D(Face3DEntity {
                common: c.clone(),
                corner1: p3(0.0, 0.0, 0.0),
                corner2: p3(1.0, 0.0, 0.0),
                corner3: p3(1.0, 1.0, 0.0),
                corner4: p3(0.0, 1.0, 0.0),
            }),
            Entity::Spline(SplineEntity {
                common: c.clone(),
                fit_points: vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)],
                control_points: vec![p3(0.5, 0.5, 0.0)],
            }),
            Entity::MText(MTextEntity {
                common: c.clone(),
                insertion_point: p3(0.0, 0.0, 0.0),
                text: "para".to_string(),
                text_height: 2.5,
                rotation: 0.3,
                line_spacing_factor: 1.0,
            }),
            Entity::Polyline3D(PolylineEntity {
                common: c.clone(),
                vertices: vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0)],
                closed: true,
            }),
            Entity::Dimension(DimensionEntity {
                common: c.clone(),
                block_name: "*D1".to_string(),
            }),
            Entity::Hatch(HatchEntity {
                common: c.clone(),
                boundary_paths: vec![
                    HatchBoundaryPath::Polyline(vec![p2(0.0, 0.0), p2(1.0, 0.0), p2(0.0, 1.0)]),
                    HatchBoundaryPath::Edges(vec![
                        HatchEdge::Line {
                            start: p2(0.0, 0.0),
                        },
                        HatchEdge::Arc {
                            center: p2(0.0, 0.0),
                            radius: 1.0,
                            start_angle: 0.0,
                            end_angle: 1.0,
                            is_ccw: true,
                        },
                        HatchEdge::Ellipse {
                            center: p2(0.0, 0.0),
                            end: p2(1.0, 0.0),
                            minor_major_ratio: 0.5,
                            start_angle: 0.0,
                            end_angle: 1.0,
                            is_ccw: false,
                        },
                        HatchEdge::Spline {
                            control_points: vec![p2(0.0, 0.0), p2(1.0, 1.0)],
                        },
                    ]),
                ],
                solid_fill: true,
                gradient: Some(HatchGradient {
                    is_radial: true,
                    angle: 0.7,
                    color1: "#ff0000".to_string(),
                    color2: "#0000ff".to_string(),
                }),
                pattern_lines: vec![HatchPatternLine {
                    angle: 0.5,
                    base_point: p2(0.0, 0.0),
                    offset: p2(0.0, 1.0),
                    dash_pattern: vec![1.0, -0.5],
                }],
            }),
            Entity::Solid3D(solid3d.clone()),
            Entity::Leader(LeaderEntity {
                common: c.clone(),
                vertices: vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)],
                has_arrowhead: true,
            }),
            Entity::MultiLeader(MultiLeaderEntity {
                common: c.clone(),
                lines: vec![vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)]],
            }),
            Entity::MLine(MLineEntity {
                common: c.clone(),
                vertices: vec![MLineVertex {
                    point: p3(0.0, 0.0, 0.0),
                    miter_direction: p3(0.0, 1.0, 0.0),
                }],
                closed: true,
                mlinestyle_name: "STANDARD".to_string(),
            }),
            Entity::Region(solid3d.clone()),
            Entity::PolylinePFace(solid3d),
            Entity::Polyline2D(lwpoly),
            Entity::Tolerance(ToleranceEntity {
                common: c.clone(),
                insertion_point: p3(0.0, 0.0, 0.0),
                text_height: 2.5,
                text_value: "%%v0.1".to_string(),
            }),
            Entity::AcadTable(AcadTableEntity {
                common: c.clone(),
                block_name: "*T1".to_string(),
                insertion_point: p3(0.0, 0.0, 0.0),
                scale: p3(1.0, 1.0, 1.0),
                rotation: 0.1,
            }),
            Entity::Wipeout(WipeoutEntity {
                common: c.clone(),
                boundary: vec![p2(0.0, 0.0), p2(1.0, 0.0), p2(0.0, 1.0)],
            }),
            Entity::Light(LightEntity {
                common: c.clone(),
                position: p3(0.0, 0.0, 10.0),
                target: p3(0.0, 0.0, 0.0),
                has_target: true,
            }),
            Entity::Unknown {
                common: c,
                type_name: "ACAD_PROXY_ENTITY".to_string(),
            },
        ];
        for e in &all {
            // Exhaustiveness guard -- see the doc comment above.
            match e {
                Entity::Line(_)
                | Entity::Circle(_)
                | Entity::Text(_)
                | Entity::LwPolyline(_)
                | Entity::Arc(_)
                | Entity::Ellipse(_)
                | Entity::Point(_)
                | Entity::Solid(_)
                | Entity::Ray(_)
                | Entity::XLine(_)
                | Entity::Insert(_)
                | Entity::Attrib(_)
                | Entity::Attdef(_)
                | Entity::Viewport(_)
                | Entity::Face3D(_)
                | Entity::Spline(_)
                | Entity::MText(_)
                | Entity::Polyline3D(_)
                | Entity::Dimension(_)
                | Entity::Hatch(_)
                | Entity::Solid3D(_)
                | Entity::Leader(_)
                | Entity::MultiLeader(_)
                | Entity::MLine(_)
                | Entity::Region(_)
                | Entity::PolylinePFace(_)
                | Entity::Polyline2D(_)
                | Entity::Tolerance(_)
                | Entity::AcadTable(_)
                | Entity::Wipeout(_)
                | Entity::Light(_)
                | Entity::Unknown { .. } => {}
            }
        }
        all
    }

    #[test]
    fn every_variant_is_tagged_with_its_type_name() {
        for e in one_of_each() {
            let value = serde_json::to_value(&e).expect("serializable");
            let expected = match &e {
                Entity::Unknown { .. } => "UNKNOWN",
                other => other.type_name(),
            };
            assert_eq!(
                value["type"], expected,
                "the JSON `type` tag must equal type_name() for {e:?}"
            );
            if let Entity::Unknown { type_name, .. } = &e {
                assert_eq!(value["type_name"], type_name.as_str());
            }
            // Internally tagged: the entity's own fields sit next to `type`,
            // not nested under a variant-name key.
            assert!(
                value["common"].is_object(),
                "entity fields should be flattened beside the tag: {value}"
            );
        }
    }

    #[test]
    fn hatch_paths_and_edges_are_tagged_like_everything_else() {
        let hatch = one_of_each()
            .into_iter()
            .find(|e| matches!(e, Entity::Hatch(_)))
            .expect("one_of_each has a HATCH");
        let value = serde_json::to_value(&hatch).expect("serializable");
        let paths = value["boundary_paths"]
            .as_array()
            .expect("boundary_paths is an array");
        assert_eq!(paths[0]["type"], "POLYLINE");
        assert!(paths[0]["data"].is_array(), "{}", paths[0]);
        assert_eq!(paths[1]["type"], "EDGES");
        let kinds: Vec<&str> = paths[1]["data"]
            .as_array()
            .expect("edges are an array")
            .iter()
            .map(|e| {
                e["type"]
                    .as_str()
                    .expect("each edge carries a string `type`")
            })
            .collect();
        assert_eq!(kinds, ["LINE", "ARC", "ELLIPSE", "SPLINE"]);
    }

    #[test]
    fn non_finite_floats_serialize_as_null_and_do_not_round_trip() {
        let e = Entity::Circle(CircleEntity {
            common: common("2B"),
            center: p3(0.0, 0.0, 0.0),
            radius: f64::NAN,
        });
        let text =
            serde_json::to_string(&e).expect("serde_json writes null for NaN, it does not fail");
        assert!(text.contains("\"radius\":null"), "{text}");
        assert!(
            serde_json::from_str::<Entity>(&text).is_err(),
            "a null radius must be rejected on the way back, not silently defaulted"
        );
    }

    #[test]
    fn an_unknown_or_missing_type_tag_is_an_error_not_a_panic() {
        let common = r#""common":{"handle":"1","layer":"0","color_index":256,"true_color":null}"#;
        assert!(serde_json::from_str::<Entity>(&format!(r#"{{"type":"NOPE",{common}}}"#)).is_err());
        assert!(serde_json::from_str::<Entity>(&format!(r#"{{{common}}}"#)).is_err());
        assert!(
            serde_json::from_str::<Entity>(&format!(r#"{{"type":"UNKNOWN",{common}}}"#)).is_err(),
            "UNKNOWN without its type_name field is incomplete"
        );
        let ok: Entity = serde_json::from_str(&format!(
            r#"{{"type":"UNKNOWN",{common},"type_name":"ACAD_PROXY_ENTITY"}}"#
        ))
        .expect("a complete UNKNOWN entity deserializes");
        assert_eq!(ok.type_name(), "ACAD_PROXY_ENTITY");
    }

    #[test]
    fn every_variant_survives_a_round_trip() {
        for e in one_of_each() {
            let text = serde_json::to_string(&e).expect("serializable");
            let back: Entity = serde_json::from_str(&text).expect("deserializable");
            assert_eq!(back, e);
        }
    }

    #[test]
    fn a_database_round_trips_and_pretty_only_changes_whitespace() {
        let mut block_records = BTreeMap::new();
        block_records.insert(
            "*Model_Space".to_string(),
            BlockRecord {
                name: "*Model_Space".to_string(),
                entities: one_of_each(),
            },
        );
        let mut layers = BTreeMap::new();
        layers.insert(
            "0".to_string(),
            LayerRecord {
                name: "0".to_string(),
                color_index: 7,
            },
        );
        let mut mlinestyles = BTreeMap::new();
        mlinestyles.insert("STANDARD".to_string(), vec![0.5, -0.5]);
        let db = CadDatabase {
            entities: one_of_each(),
            tables: Tables {
                layers,
                block_records,
                mlinestyles,
            },
        };

        let compact = to_json(&db, ToJsonOptions::default()).expect("serializable");
        let pretty = to_json(&db, ToJsonOptions { pretty: true }).expect("serializable");
        assert!(!compact.contains('\n'), "compact output is a single line");
        assert!(
            pretty.contains('\n'),
            "pretty output is indented across lines"
        );

        let from_compact: CadDatabase = serde_json::from_str(&compact).expect("deserializable");
        let from_pretty: CadDatabase = serde_json::from_str(&pretty).expect("deserializable");
        assert_eq!(from_compact, db);
        assert_eq!(from_pretty, db);
    }
}
