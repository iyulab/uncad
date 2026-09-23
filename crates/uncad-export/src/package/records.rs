//! The package's records: one per text, dimension, geometric entity,
//! closed region and block instance, each with its exact numbers, its world
//! box and -- filled in by the caller -- the images it is on.
//!
//! A record's id is the model's reference ID ([`EntityId`]) as a decimal
//! string, or the path of IDs `<insert>/<child>` for a text drawn inside a
//! block reference: the same path the renderer names that text's `<text>`
//! element with (`t<id>.<id>`). The file's own handle, where the entity
//! came from a file, is the record's `handle`.

use std::collections::{BTreeMap, BTreeSet};

use iron_render_cad::TextBox;
use serde_json::{json, Map, Value};
use uncad_model::model::{
    AttribEntity, Confidence, Entity, EntityCommon, EntityId, HorizontalJustification, Point2D,
    Point3D, Ref, VerticalJustification,
};
use uncad_model::tables::BlockRecord;
use uncad_model::{Affine2, Tables};

use crate::dimension::tolerance_text_height;
use crate::frame::Rect;
use crate::geom::plane_to_world;
use crate::text::{decode_mtext, decode_text};

/// One entity's record before its images are known.
#[derive(Clone)]
pub(crate) struct Record {
    pub(crate) id: String,
    pub(crate) bbox: Rect,
    pub(crate) value: Map<String, Value>,
}

/// A record id: the path's IDs, outermost first, joined by `/`.
pub(crate) fn record_id(path: &[EntityId]) -> String {
    let mut id = String::new();
    for (i, e) in path.iter().enumerate() {
        if i > 0 {
            id.push('/');
        }
        id.push_str(&e.value().to_string());
    }
    id
}

/// The order records are written in, and what a shard's `first_key` and
/// `last_key` publish: the id's IDs as numbers. A malformed segment sorts
/// last.
pub(crate) fn id_key(id: &str) -> Vec<u64> {
    id.split('/')
        .map(|s| s.parse::<u64>().unwrap_or(u64::MAX))
        .collect()
}

/// The file's handle for an entity, when it came from a file.
pub(crate) fn handle_of(common: &EntityCommon) -> Value {
    match &common.source_handle {
        Ref::Resolved(h) => json!(h),
        Ref::Absent | Ref::Unresolved(_) => Value::Null,
    }
}

/// A layer reference as a record names it: the name it resolved to; the
/// name or handle the file wrote when nothing answers to it; `""` when the
/// file names none.
pub(crate) fn layer_name(layer: &Ref<String>) -> &str {
    match layer {
        Ref::Resolved(name) | Ref::Unresolved(name) => name,
        Ref::Absent => "",
    }
}

// ------------------------------------------------------------- rounding

pub(crate) fn round_to(v: f64, decimals: u16) -> f64 {
    let f = 10f64.powi(i32::from(decimals));
    let r = (v * f).round() / f;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

pub(crate) struct Rounder {
    pub(crate) coords: u16,
    pub(crate) derived: u16,
}

impl Rounder {
    pub(crate) fn coord(&self, v: f64) -> f64 {
        round_to(v, self.coords)
    }

    pub(crate) fn derived(&self, v: f64) -> f64 {
        round_to(v, self.derived)
    }

    pub(crate) fn pt2(&self, p: Point2D) -> Value {
        json!([self.coord(p.x), self.coord(p.y)])
    }

    pub(crate) fn pt3(&self, p: Point3D) -> Value {
        json!([self.coord(p.x), self.coord(p.y)])
    }

    pub(crate) fn rect(&self, r: &Rect) -> Value {
        json!([
            self.coord(r.min_x),
            self.coord(r.min_y),
            self.coord(r.max_x),
            self.coord(r.max_y)
        ])
    }
}

/// The decimals a world coordinate is written with: `$LUPREC` (clamped to
/// 3..=12), but never fewer than the package's own scale needs.
///
/// `$LUPREC` says how precisely the drawing's units are *displayed*; it
/// says nothing about how far the package zooms into them. A drawing a few
/// thousandths of a unit across is drawn at hundreds of thousands of pixels
/// per unit, where LUPREC's three or four decimals put every tile rectangle
/// of a frame on the same numbers: the `world` a consumer reads is then a
/// different rectangle from the one the tile shows, and the affine beside
/// it maps to the wrong pixels.
///
/// The floor is the scale's. `deepest_ppu` is the finest scale the package
/// may reach -- the overview's, doubled once per zoom level -- and the
/// coordinate carries three decimals beyond one pixel there, so one unit in
/// the last place is a thousandth of a pixel.
pub(crate) fn coord_decimals(luprec: u16, deepest_ppu: f64) -> u16 {
    let from_scale = if deepest_ppu.is_finite() && deepest_ppu > 0.0 {
        (deepest_ppu.log10().ceil() + 3.0).clamp(3.0, 12.0) as u16
    } else {
        3
    };
    luprec.clamp(3, 12).max(from_scale)
}

// ----------------------------------------------------------- visibility

/// The layer an entity is drawn on: its own, except that an entity on
/// layer 0 inside a block reference takes the layer of the reference that
/// places it -- the rule the renderer draws by.
pub(crate) fn effective_layer<'a>(layer: &'a str, reference_layer: Option<&'a str>) -> &'a str {
    match reference_layer {
        Some(reference) if layer == "0" => reference,
        _ => layer,
    }
}

/// Whether the drawing hides an entity with `common` drawn under
/// `reference_layer` -- the renderer's rule: its own invisible flag (or an
/// attribute's), AutoCAD's `DEFPOINTS` layer, or a layer that is off,
/// frozen or stated not to plot. A layer the tables do not hold is shown.
pub(crate) fn is_hidden(
    common: &EntityCommon,
    invisible_attribute: bool,
    tables: &Tables,
    reference_layer: Option<&str>,
) -> bool {
    if common.invisible || invisible_attribute {
        return true;
    }
    let layer = effective_layer(common.layer.name(), reference_layer);
    if layer.eq_ignore_ascii_case("DEFPOINTS") {
        return true;
    }
    tables
        .layers
        .get(layer)
        .is_some_and(|l| l.off || l.frozen || l.plot == Some(false))
}

fn entity_hidden(e: &Entity, tables: &Tables, reference_layer: Option<&str>) -> bool {
    let invisible_attribute = matches!(e, Entity::Attrib(a) if a.flags.invisible);
    is_hidden(e.common(), invisible_attribute, tables, reference_layer)
}

// ---------------------------------------------------------------- texts

/// A text placed in the world (or on a sheet's paper).
pub(crate) struct PlacedText {
    pub(crate) id: String,
    /// The top-level entity whose part draws the text: its path's first ID.
    pub(crate) part: EntityId,
    pub(crate) handle: Value,
    pub(crate) kind: &'static str,
    pub(crate) layer: String,
    pub(crate) text: String,
    pub(crate) raw: String,
    pub(crate) height: f64,
    pub(crate) rotation: f64,
    pub(crate) anchor: Point2D,
    /// The glyph outlines' box when the fonts laid the text out, the
    /// renderer's estimate otherwise.
    pub(crate) bbox: Rect,
    pub(crate) measured: bool,
    /// Glyphs the fonts could not shape (drawn as `.notdef` boxes).
    pub(crate) unshaped: usize,
    pub(crate) style: String,
    pub(crate) tag: Option<String>,
}

/// An entity a text path steps through: a drawing's entity, or an
/// attribute that hangs off a nested block reference and nothing else.
#[derive(Clone, Copy)]
enum Located<'a> {
    Entity(&'a Entity),
    NestedAttribute(&'a AttribEntity),
}

/// Finds the entities along a text's path. The first ID is a top-level
/// entity (`top`); each later one is an entity of the block the previous
/// one references, or an attribute of a block reference among them -- the
/// renderer draws a nested reference's attributes beside it, in the
/// enclosing block's frame. Blocks are indexed once, on first use.
struct PathIndex<'a> {
    tables: &'a Tables,
    top: &'a BTreeMap<EntityId, &'a Entity>,
    blocks: BTreeMap<&'a str, BTreeMap<EntityId, Located<'a>>>,
}

impl<'a> PathIndex<'a> {
    fn block(&mut self, name: &'a str) -> Option<&BTreeMap<EntityId, Located<'a>>> {
        if !self.blocks.contains_key(name) {
            let block: &'a BlockRecord = self.tables.block_records.get(name)?;
            let mut index = BTreeMap::new();
            for e in &block.entities {
                if let Entity::Insert(i) = e {
                    for a in &i.attribs {
                        index
                            .entry(a.common.id)
                            .or_insert(Located::NestedAttribute(a));
                    }
                }
            }
            for e in &block.entities {
                index.insert(e.common().id, Located::Entity(e));
            }
            self.blocks.insert(name, index);
        }
        self.blocks.get(name)
    }
}

/// The world rotation of a text drawn with `rotation` in the frame `m`: the
/// angle of its transformed *up* axis less 90 degrees, i.e. how the glyphs
/// are oriented on the page. Through the up axis rather than the baseline
/// so that a frame mirrored about the vertical axis (the usual MIRROR of a
/// block) reports an upright, mirror-written text as 0 rather than 180.
fn text_rotation(m: &Affine2, rotation: f64) -> f64 {
    let (ux, uy) = (-rotation.sin(), rotation.cos());
    let (wx, wy) = (m.a * ux + m.c * uy, m.b * ux + m.d * uy);
    wy.atan2(wx) - std::f64::consts::FRAC_PI_2
}

/// The length scale of a frame, `sqrt(|det|)`: exact for a uniform scale,
/// the geometric mean of the axis scales otherwise.
fn length_scale(m: &Affine2) -> f64 {
    m.determinant().abs().sqrt()
}

/// The point a TEXT or ATTRIB is placed by: its alignment point when it is
/// justified at all (the file states one then), its start point otherwise.
fn text_base(
    start: Point2D,
    alignment: Option<Point2D>,
    h: HorizontalJustification,
    v: VerticalJustification,
) -> Point2D {
    if h != HorizontalJustification::Left || v != VerticalJustification::Baseline {
        alignment.unwrap_or(start)
    } else {
        start
    }
}

/// Every text the renderer drew that a reader should find, as the package
/// places it: `boxes` are the scene's texts in drawing order, `top` its
/// top-level entities by ID, and `keep` says which top-level entities'
/// texts count (the shown ones -- not hidden, not left out of the picture,
/// not a dimension, whose label is its own record). A text the drawing
/// hides (drawn only faded, when hidden entities are included) or that
/// shows nothing but spaces is not a record.
pub(crate) fn placed_texts(
    tables: &Tables,
    top: &BTreeMap<EntityId, &Entity>,
    boxes: &[TextBox],
    keep: impl Fn(EntityId) -> bool,
) -> Vec<PlacedText> {
    let mut index = PathIndex {
        tables,
        top,
        blocks: BTreeMap::new(),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for b in boxes {
        let Some(&first) = b.path.first() else {
            continue;
        };
        if !keep(first) {
            continue;
        }
        let Some(placed) = place(&mut index, b) else {
            continue;
        };
        if seen.insert(placed.id.clone()) {
            out.push(placed);
        }
    }
    out
}

fn place(index: &mut PathIndex<'_>, b: &TextBox) -> Option<PlacedText> {
    let tables = index.tables;
    let mut current = Located::Entity(*index.top.get(b.path.first()?)?);
    let mut affine = Affine2::IDENTITY;
    let mut reference_layer: Option<String> = None;
    for id in &b.path[1..] {
        let Located::Entity(owner) = current else {
            return None;
        };
        if entity_hidden(owner, tables, reference_layer.as_deref()) {
            return None;
        }
        let (block, placement) = match owner {
            Entity::Insert(i) => (&i.block_name, Affine2::from_insert(i)),
            Entity::AcadTable(t) => (
                &t.block_name,
                Affine2::placement(
                    Point2D {
                        x: t.insertion_point.x,
                        y: t.insertion_point.y,
                    },
                    t.scale.x,
                    t.scale.y,
                    t.rotation,
                ),
            ),
            Entity::Dimension(d) => (&d.block_name, Affine2::IDENTITY),
            _ => return None,
        };
        affine = placement.then(&affine);
        reference_layer = Some(
            effective_layer(owner.common().layer.name(), reference_layer.as_deref()).to_string(),
        );
        let children = index.block(block.resolved()?)?;
        current = *children.get(id)?;
    }
    let reference = reference_layer.as_deref();
    let (common, hidden) = match current {
        Located::Entity(e) => (e.common(), entity_hidden(e, tables, reference)),
        Located::NestedAttribute(a) => (
            &a.common,
            is_hidden(&a.common, a.flags.invisible, tables, reference),
        ),
    };
    if hidden {
        return None;
    }
    struct Leaf<'b> {
        kind: &'static str,
        raw: &'b str,
        text: String,
        base: Point2D,
        plane: Affine2,
        height: f64,
        rotation: f64,
        style: &'b Ref<String>,
        tag: Option<&'b str>,
    }
    let attribute = |a: &'_ AttribEntity| -> (Point2D, Affine2) {
        (
            text_base(
                a.start_point,
                a.alignment_point,
                a.horizontal_justification,
                a.vertical_justification,
            ),
            plane_to_world(a.extrusion, a.elevation),
        )
    };
    let no_style = Ref::Absent;
    let leaf = match current {
        Located::Entity(Entity::Text(t)) => Leaf {
            kind: "TEXT",
            raw: &t.text,
            text: decode_text(&t.text).plain,
            base: text_base(
                t.start_point,
                t.alignment_point,
                t.horizontal_justification,
                t.vertical_justification,
            ),
            plane: plane_to_world(t.extrusion, t.elevation),
            height: t.text_height,
            rotation: t.rotation,
            style: &t.style_name,
            tag: None,
        },
        Located::Entity(Entity::Attrib(a)) | Located::NestedAttribute(a) => {
            let (base, plane) = attribute(a);
            Leaf {
                kind: "ATTRIB",
                raw: &a.text,
                text: decode_text(&a.text).plain,
                base,
                plane,
                height: a.text_height,
                rotation: a.rotation,
                style: &a.style_name,
                tag: Some(&a.tag),
            }
        }
        Located::Entity(Entity::MText(m)) => Leaf {
            kind: "MTEXT",
            raw: &m.text,
            text: decode_mtext(&m.text).plain,
            base: Point2D {
                x: m.insertion_point.x,
                y: m.insertion_point.y,
            },
            plane: Affine2::IDENTITY,
            height: m.text_height,
            rotation: m.rotation,
            style: &m.style_name,
            tag: None,
        },
        Located::Entity(Entity::Tolerance(t)) => Leaf {
            kind: "TOLERANCE",
            raw: &t.text_value,
            text: decode_text(&t.text_value).plain,
            base: Point2D {
                x: t.insertion_point.x,
                y: t.insertion_point.y,
            },
            plane: Affine2::IDENTITY,
            height: tolerance_text_height(t, tables),
            rotation: 0.0,
            style: &no_style,
            tag: None,
        },
        _ => return None,
    };
    if leaf.text.trim().is_empty() {
        return None;
    }
    let frame = leaf.plane.then(&affine);
    let anchor = frame.apply(leaf.base);
    let bbox = b
        .measured
        .or(b.estimate)
        .map(Rect::from)
        .unwrap_or(Rect::at(anchor.x, anchor.y));
    Some(PlacedText {
        id: record_id(&b.path),
        part: b.path[0],
        handle: handle_of(common),
        kind: leaf.kind,
        layer: effective_layer(common.layer.name(), reference).to_string(),
        raw: leaf.raw.to_string(),
        text: leaf.text,
        height: leaf.height * length_scale(&frame),
        rotation: text_rotation(&frame, leaf.rotation),
        anchor,
        bbox,
        measured: b.measured.is_some(),
        unshaped: b.missing_glyphs,
        style: leaf.style.name().to_string(),
        tag: leaf.tag.map(str::to_string),
    })
}

/// One text's record, for `texts.json`.
///
/// `space` is `model` or `paper`; a paper-space text also names the
/// `sheet` it is on and carries that sheet image's pixel box instead of
/// tiles, since the model tile pyramid does not cover the paper. Its
/// `bbox` and `anchor` are in the layout's paper units, not world units --
/// the only records in the package that are.
pub(crate) fn text_record(
    t: &PlacedText,
    rounder: &Rounder,
    space: &str,
    sheet: Option<&str>,
    measured_why: &str,
) -> Record {
    let mut v = Map::new();
    v.insert("id".into(), json!(t.id));
    v.insert("handle".into(), t.handle.clone());
    v.insert("kind".into(), json!(t.kind));
    v.insert("text".into(), json!(t.text));
    if t.raw != t.text {
        v.insert("raw".into(), json!(t.raw));
    }
    if let Some(tag) = &t.tag {
        v.insert("tag".into(), json!(tag));
    }
    v.insert("space".into(), json!(space));
    if let Some(name) = sheet {
        v.insert("sheet".into(), json!(name));
    }
    v.insert("layer".into(), json!(t.layer));
    v.insert("height".into(), json!(rounder.derived(t.height)));
    v.insert(
        "rotation_deg".into(),
        json!(rounder.derived(t.rotation.to_degrees())),
    );
    v.insert("anchor".into(), rounder.pt2(t.anchor));
    if !t.style.is_empty() {
        v.insert("style".into(), json!(t.style));
    }
    v.insert("bbox".into(), rounder.rect(&t.bbox));
    v.insert(
        "bbox_confidence".into(),
        json!(if t.measured { "measured" } else { "estimated" }),
    );
    v.insert(
        "why".into(),
        json!(if t.measured {
            measured_why
        } else {
            "the renderer's estimate: 0.6 em per character from the anchor"
        }),
    );
    v.insert("font_ok".into(), json!(t.unshaped == 0));
    if t.unshaped > 0 {
        v.insert("unshaped_glyphs".into(), json!(t.unshaped));
    }
    Record {
        id: t.id.clone(),
        bbox: t.bbox,
        value: v,
    }
}

// ------------------------------------------------------------ confidence

/// A record's `confidence`, never above the model's for the entity it is
/// made from (the model's invariant: a value that entered low never leaves
/// high). The model's `HIGH` changes nothing; below it, a value the package
/// would call `exact` or `stored` is `estimated`, with the reason.
pub(crate) fn capped_confidence(
    confidence: &'static str,
    model: Confidence,
) -> (&'static str, Option<&'static str>) {
    match (model, confidence) {
        (Confidence::High, _) => (confidence, None),
        (_, "exact" | "stored") => (
            "estimated",
            Some("the reader that produced the model trusts this entity's values less than fully"),
        ),
        _ => (confidence, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_their_numbers() {
        let path = [EntityId::new(0x125), EntityId::new(270)];
        assert_eq!(record_id(&path), "293/270");
        assert_eq!(record_id(&path[..1]), "293");
        assert_eq!(id_key("293/270"), vec![293, 270]);
        assert!(id_key("31") < id_key("293"), "numbers, not strings");
        assert!(id_key("293") < id_key("293/1"));
        assert_eq!(id_key("x"), vec![u64::MAX]);
    }

    #[test]
    fn rounding() {
        assert_eq!(round_to(1.23456789, 3), 1.235);
        assert_eq!(round_to(-0.0001, 3), 0.0);
    }

    #[test]
    fn coordinate_decimals_never_fall_below_the_scale() {
        // An ordinary drawing: a 10 000-unit plan on a 1568 px overview is
        // about 0.15 px/unit, 4.8 at the deepest of five levels, so four
        // decimals put one ulp at 5e-4 px.
        assert_eq!(coord_decimals(0, 4.8), 4);
        assert_eq!(coord_decimals(4, 4.8), 4);
        assert_eq!(coord_decimals(8, 4.8), 8, "a precise drawing keeps its 8");
        // A drawing 0.004 units across: 8.7e6 px/unit at z5. Ten decimals
        // keep one ulp at a thousandth of a pixel.
        assert_eq!(coord_decimals(4, 8.7e6), 10);
        assert!(10f64.powi(-10) * 8.7e6 < 1e-3);
        // Clamped at both ends; a scale that is not a number falls back to
        // the LUPREC floor.
        assert_eq!(coord_decimals(99, 4.8), 12);
        assert_eq!(coord_decimals(4, 1e30), 12);
        assert_eq!(coord_decimals(4, f64::NAN), 4);
        assert_eq!(coord_decimals(4, 0.0), 4);
    }

    #[test]
    fn a_frame_reports_the_text_orientation_a_reader_sees() {
        let quarter = std::f64::consts::FRAC_PI_2;
        // A 90-degree child under a parent mirrored about the vertical
        // axis: a text along the child's x axis runs up the page with its
        // glyph tops to +x, which reads as -90 degrees.
        let child = Affine2::placement(Point2D { x: 0.0, y: 0.0 }, 1.0, 1.0, quarter);
        let mirror = Affine2::placement(Point2D { x: 100.0, y: 100.0 }, -1.0, 1.0, 0.0);
        let both = child.then(&mirror);
        assert!((text_rotation(&both, 0.0) + quarter).abs() < 1e-12);
        // The top-level mirror alone keeps a horizontal text at 0 (the
        // glyphs are mirrored, not turned).
        assert!(text_rotation(&mirror, 0.0).abs() < 1e-12);
        assert!((length_scale(&both) - 1.0).abs() < 1e-12);
        let stretch = Affine2::placement(Point2D { x: 0.0, y: 0.0 }, 2.0, 1.0, 0.0);
        assert!((length_scale(&stretch) - 2f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn a_record_never_claims_more_than_the_model() {
        assert_eq!(
            capped_confidence("exact", Confidence::High),
            ("exact", None)
        );
        assert_eq!(capped_confidence("exact", Confidence::Low).0, "estimated");
        assert_eq!(
            capped_confidence("stored", Confidence::Unknown).0,
            "estimated"
        );
        assert_eq!(
            capped_confidence("unavailable", Confidence::Low),
            ("unavailable", None)
        );
    }
}
