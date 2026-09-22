//! SVG rendering of a parsed [`CadDatabase`].
//!
//! Entity coverage matches [`crate::model::Entity`]'s variants. The one type
//! left permanently unsupported is `ACAD_PROXY_ENTITY`: an opaque per-app
//! serialized blob with no geometry to draw at all. Anything this renderer
//! cannot draw is reported through [`ToSvgResult::unsupported_types`] rather
//! than dropped silently.
//!
//! Several types render as deliberate approximations -- curves as chords, 3D
//! solids as isometric wireframes, VIEWPORT and WIPEOUT as outlines only. See
//! `docs/CAVEATS.md` for the full picture.
//!
//! Layout of this module: options and results, the block transform, the
//! rendering context, per-entity rendering, then [`to_svg`] itself.
//! Submodules hold the parts that stand on their own -- [`format`] (number and
//! string formatting), [`hatch`] (HATCH fills) and [`bounds`] (viewBox and
//! outlier trim).

mod bounds;
mod format;
mod hatch;

use crate::color::{contrast_on_white, resolve_color, DEFAULT_COLOR};
use crate::dynapi::{Point2D, Point3D};
use crate::model::{Entity, EntityCommon, MLineVertex};
use crate::tables::Tables;
use crate::CadDatabase;
use bounds::{dominant_cluster_box, Box2D};
use format::{clean, escape_xml, neg, points_attr, rotate_transform_attr, xy};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write as _;

/// Which of a drawing's spaces to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Space {
    /// The drawing itself.
    Model,
    /// Sheet layouts: borders, title blocks.
    Paper,
    /// Everything, in one document.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToSvgOptions {
    pub padding: f64,
    /// `None` = auto-scaled to the computed viewBox (see [`to_svg`]).
    pub stroke_width: Option<f64>,
    pub space: Space,
    pub outlier_trim: bool,
}

impl Default for ToSvgOptions {
    fn default() -> Self {
        ToSvgOptions {
            padding: 5.0,
            stroke_width: None,
            space: Space::Model,
            outlier_trim: true,
        }
    }
}

pub struct ToSvgResult {
    pub svg: String,
    /// The `viewBox` the document was given: what the image shows, padding
    /// included, and the key to mapping its pixels back to the drawing.
    pub view_box: ViewBox,
    /// DXF names of entity types this renderer had nothing to draw for,
    /// sorted.
    pub unsupported_types: Vec<String>,
}

/// An SVG `viewBox`, in SVG coordinates: `x`/`y` are the top-left corner and
/// y runs downward, so a world point `(wx, wy)` sits at SVG `(wx, -wy)`. The
/// world bounds are therefore `x..x + width` horizontally and
/// `-(y + height)..-y` vertically. Padding is already included.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ViewBox {
    /// The world-space rectangle this viewBox shows, as
    /// `(min_x, min_y, max_x, max_y)`.
    pub fn world_bounds(&self) -> (f64, f64, f64, f64) {
        (
            self.x,
            -(self.y + self.height),
            self.x + self.width,
            -self.y,
        )
    }

    /// Where the world point `(x, y)` lands in an image rendered at
    /// `px_per_unit` (row 0 at the top), as fractional pixels.
    pub fn world_to_px(&self, x: f64, y: f64, px_per_unit: f64) -> (f64, f64) {
        ((x - self.x) * px_per_unit, (-y - self.y) * px_per_unit)
    }

    /// The inverse of [`world_to_px`](Self::world_to_px).
    pub fn px_to_world(&self, px: f64, py: f64, px_per_unit: f64) -> (f64, f64) {
        (self.x + px / px_per_unit, -(self.y + py / px_per_unit))
    }
}

// --- block transform ---------------------------------------------------

/// An INSERT-style placement, composed across nested block references.
#[derive(Debug, Clone, Copy)]
struct Transform {
    insertion_point: Point2D,
    x_scale: f64,
    y_scale: f64,
    rotation: f64,
}

impl Transform {
    fn identity() -> Self {
        Transform {
            insertion_point: Point2D { x: 0.0, y: 0.0 },
            x_scale: 1.0,
            y_scale: 1.0,
            rotation: 0.0,
        }
    }

    /// Local (x, y) -> world (x, y) through this transform.
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (cos, sin) = (self.rotation.cos(), self.rotation.sin());
        (
            self.insertion_point.x + self.x_scale * cos * x - self.y_scale * sin * y,
            self.insertion_point.y + self.x_scale * sin * x + self.y_scale * cos * y,
        )
    }

    /// The SVG `matrix(a b c d e f)` equivalent, composed with the renderer's
    /// CAD-y-up to SVG-y-down flip.
    fn svg_matrix(&self) -> [f64; 6] {
        let (cos, sin) = (self.rotation.cos(), self.rotation.sin());
        [
            self.x_scale * cos,
            neg(self.x_scale * sin),
            self.y_scale * sin,
            self.y_scale * cos,
            self.insertion_point.x,
            neg(self.insertion_point.y),
        ]
    }
}

/// Composes a parent world-transform with a child's local transform: applying
/// the result to a point equals applying `child` then `parent`.
fn compose(parent: &Transform, child: &Transform) -> Transform {
    let (px, py) = parent.apply(child.insertion_point.x, child.insertion_point.y);
    Transform {
        insertion_point: Point2D { x: px, y: py },
        x_scale: parent.x_scale * child.x_scale,
        y_scale: parent.y_scale * child.y_scale,
        rotation: parent.rotation + child.rotation,
    }
}

// --- render context ----------------------------------------------------

/// Generous enough for any real drawing this project has been checked against
/// while still cutting off combinatorial block-reference blowup quickly: 10
/// INSERTs per level exhausts it by nesting level 6, long before the 20-level
/// depth cap could engage.
const BLOCK_REF_BUDGET: u32 = 1_000_000;

/// How deep block references may nest before rendering gives up.
const MAX_BLOCK_REF_DEPTH: u32 = 20;

struct Ctx<'a> {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ent_min_x: f64,
    ent_max_x: f64,
    ent_min_y: f64,
    ent_max_y: f64,
    unsupported: HashSet<String>,
    tables: &'a Tables,
    depth: u32,
    scale: f64,
    inherited_color: String,
    transform: Transform,
    /// `<defs>` entries accumulated by HATCH rendering, emitted once into a
    /// top-level `<defs>` by [`to_svg`]. Persists across `render_block_ref`'s
    /// transform save/restore, since a HATCH can appear inside a block too.
    defs: Vec<String>,
    next_def_id: u32,
    /// Remaining budget for `render_block_ref` calls across the whole render
    /// pass, decremented once per call and never restored. The depth cap alone
    /// bounds nesting but not *breadth*: a crafted file with many INSERTs per
    /// block at every level can still fan out combinatorially before the depth
    /// cap is ever reached.
    block_ref_budget: u32,
}

impl<'a> Ctx<'a> {
    fn new(tables: &'a Tables) -> Self {
        Ctx {
            xs: Vec::new(),
            ys: Vec::new(),
            ent_min_x: f64::INFINITY,
            ent_max_x: f64::NEG_INFINITY,
            ent_min_y: f64::INFINITY,
            ent_max_y: f64::NEG_INFINITY,
            unsupported: HashSet::new(),
            tables,
            depth: 0,
            scale: 1.0,
            inherited_color: DEFAULT_COLOR.to_string(),
            transform: Transform::identity(),
            defs: Vec::new(),
            next_def_id: 0,
            block_ref_budget: BLOCK_REF_BUDGET,
        }
    }

    fn reset_entity_bounds(&mut self) {
        self.ent_min_x = f64::INFINITY;
        self.ent_max_x = f64::NEG_INFINITY;
        self.ent_min_y = f64::INFINITY;
        self.ent_max_y = f64::NEG_INFINITY;
    }

    fn entity_box(&self) -> Option<Box2D> {
        self.ent_min_x.is_finite().then_some(Box2D {
            min_x: self.ent_min_x,
            max_x: self.ent_max_x,
            min_y: self.ent_min_y,
            max_y: self.ent_max_y,
        })
    }

    /// Records one *local* coordinate pair, applying the current (possibly
    /// block-nested) transform first.
    ///
    /// Non-finite results (from a malformed source file or a degenerate
    /// transform) are dropped rather than recorded: letting `Infinity` into the
    /// running bounds can pin both the min and the max to `Infinity` (the
    /// min-side update never fires because `Infinity < Infinity` is false),
    /// and the box's diagonal then computes as `NaN`, which panics the
    /// `partial_cmp(..).unwrap()` calls in [`bounds`] instead of just rendering
    /// a degenerate point.
    fn consider(&mut self, local_x: f64, local_y: f64) {
        let (x, y) = self.transform.apply(local_x, local_y);
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        self.xs.push(x);
        self.ys.push(y);
        if x < self.ent_min_x {
            self.ent_min_x = x;
        }
        if x > self.ent_max_x {
            self.ent_max_x = x;
        }
        if y < self.ent_min_y {
            self.ent_min_y = y;
        }
        if y > self.ent_max_y {
            self.ent_max_y = y;
        }
    }

    fn consider_all(&mut self, points: &[Point2D]) {
        for p in points {
            self.consider(p.x, p.y);
        }
    }

    fn consider_all_3d(&mut self, points: &[Point3D]) {
        for p in points {
            self.consider(p.x, p.y);
        }
    }

    /// A document-unique id for a `<defs>` entry, e.g. `"hp3"`.
    fn next_def_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.next_def_id);
        self.next_def_id += 1;
        id
    }
}

// --- element helpers ---------------------------------------------------

/// `stroke-width` cannot be resolved while rendering -- it is derived from the
/// final viewBox, which is only known once every entity has been walked. Block
/// references emit this placeholder carrying their cumulative scale instead,
/// and [`resolve_stroke_widths`] substitutes the real value in one pass at the
/// end.
fn stroke_width_placeholder(cumulative_scale: f64) -> String {
    format!("@@SW@@{cumulative_scale}@@")
}

/// Resolves every `@@SW@@<cumulative scale>@@` placeholder to
/// `effective_stroke_width / scale`, so a stroke keeps the same visual weight
/// however deeply nested or scaled its block reference is.
fn resolve_stroke_widths(body: &str, effective_stroke_width: f64) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        let Some(start) = rest.find("@@SW@@") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start + "@@SW@@".len()..];
        let Some(end) = after.find("@@") else {
            // Malformed placeholder (shouldn't happen) -- emit verbatim.
            out.push_str(&rest[start..]);
            break;
        };
        let scale: f64 = after[..end].parse().unwrap_or(1.0);
        let _ = write!(out, "{}", effective_stroke_width / scale);
        rest = &after[end + "@@".len()..];
    }
    out
}

/// A `<polyline>`, or a `<polygon>` when `closed` -- the shape every polyline
/// entity renders to.
fn polyline_element(pts: &[Point2D], closed: bool, color: &str) -> String {
    let tag = if closed { "polygon" } else { "polyline" };
    format!(
        "<{tag} points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
        points_attr(pts)
    )
}

/// A `<path>` for a polyline with arc segments: `A` commands for the bulges,
/// `L` for the straight runs. SVG's sweep flag 1 runs clockwise on a y-down
/// canvas, which is what a counter-clockwise (positive-bulge) world arc looks
/// like once y is flipped.
fn bulged_polyline_element(p: &crate::model::LwPolylineEntity, color: &str) -> String {
    let segments = crate::geom::polyline_segments(&p.vertices, &p.bulges, p.closed);
    let Some(first) = segments.first() else {
        return polyline_element(&p.vertices, p.closed, color);
    };
    let start = match first {
        crate::geom::Segment::Line { from, .. } | crate::geom::Segment::Arc { from, .. } => *from,
    };
    let mut d = format!("M {} {}", clean(start.x), neg(start.y));
    for segment in &segments {
        match segment {
            crate::geom::Segment::Line { to, .. } => {
                let _ = write!(d, " L {} {}", clean(to.x), neg(to.y));
            }
            crate::geom::Segment::Arc { to, bulge, arc, .. } => {
                let large = u8::from(bulge.abs() > 1.0);
                let sweep = u8::from(*bulge > 0.0);
                let _ = write!(
                    d,
                    " A {r} {r} 0 {large} {sweep} {} {}",
                    clean(to.x),
                    neg(to.y),
                    r = clean(arc.radius)
                );
            }
        }
    }
    if p.closed {
        d.push_str(" Z");
    }
    format!("<path d=\"{d}\" fill=\"none\" stroke=\"{color}\"/>")
}

/// A dashed outline, used for the shapes this renderer draws as an indication
/// rather than as real geometry (VIEWPORT frames, WIPEOUT boundaries).
fn dashed_outline(pts: &[Point2D], color: &str, dash: &str) -> String {
    format!(
        "<polygon points=\"{}\" fill=\"none\" stroke-dasharray=\"{dash}\" stroke=\"{color}\"/>",
        points_attr(pts)
    )
}

/// Where a single-line text is anchored and how: the SVG `text-anchor`, the
/// world-space anchor point, and how far (in text heights) the baseline sits
/// below the anchor in SVG's y-down space.
struct TextAnchor {
    at: Point2D,
    anchor: &'static str,
    baseline_drop: f64,
}

/// AutoCAD's justification rules, approximated with SVG's `text-anchor` and a
/// baseline offset: horizontal 1/4 = middle, 2 = end, 3 (aligned) and 5 (fit)
/// are drawn centered between the two points; vertical 1 bottom raises the
/// baseline by a descender, 2 middle and 4 (middle-center) drop it by roughly
/// half a cap height, 3 top by a cap height. Glyph widths come from the
/// renderer's font, not AutoCAD's, so the extent is an approximation; the
/// anchor point itself is exact.
fn text_anchor(
    start: Point2D,
    alignment: Option<Point2D>,
    horizontal: u16,
    vertical: u16,
) -> TextAnchor {
    let Some(align) = alignment else {
        return TextAnchor {
            at: start,
            anchor: "start",
            baseline_drop: 0.0,
        };
    };
    let (at, anchor) = match horizontal {
        1 | 4 => (align, "middle"),
        2 => (align, "end"),
        3 | 5 => (
            Point2D {
                x: (start.x + align.x) / 2.0,
                y: (start.y + align.y) / 2.0,
            },
            "middle",
        ),
        _ => (align, "start"),
    };
    let baseline_drop = if horizontal == 4 {
        0.36
    } else {
        match vertical {
            1 => -0.2,
            2 => 0.36,
            3 => 0.72,
            _ => 0.0,
        }
    };
    TextAnchor {
        at,
        anchor,
        baseline_drop,
    }
}

/// A single-line `<text>` -- TEXT, ATTRIB and TOLERANCE all render to this.
/// `anchor` positions it (world space); `rotation` is about the anchor.
fn text_element(
    anchor: &TextAnchor,
    height: f64,
    rotation: f64,
    color: &str,
    text: &str,
) -> String {
    let (x, y) = (
        anchor.at.x,
        neg(anchor.at.y) + anchor.baseline_drop * height,
    );
    let anchor_attr = if anchor.anchor == "start" {
        String::new()
    } else {
        format!(" text-anchor=\"{}\"", anchor.anchor)
    };
    format!(
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{height}\" fill=\"{color}\" stroke=\"none\"{anchor_attr}{}>{}</text>",
        rotate_transform_attr(rotation, x, y),
        escape_xml(text)
    )
}

/// A small filled triangle at `tip`, pointing away from `from` -- LEADER and
/// MULTILEADER arrowheads.
fn arrowhead_element(tip: &Point2D, from: &Point2D, size: f64, color: &str) -> String {
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let len = dx.hypot(dy);
    let len = if len == 0.0 { 1.0 } else { len };
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (-uy, ux);
    let (back_x, back_y) = (tip.x - ux * size, tip.y - uy * size);
    let (p2x, p2y) = (back_x + px * size * 0.35, back_y + py * size * 0.35);
    let (p3x, p3y) = (back_x - px * size * 0.35, back_y - py * size * 0.35);
    format!(
        "<polygon points=\"{},{} {p2x},{} {p3x},{}\" fill=\"{color}\" stroke=\"none\"/>",
        tip.x,
        neg(tip.y),
        neg(p2y),
        neg(p3y)
    )
}

/// The arrowhead size LEADER and MULTILEADER draw at. Not read from the file:
/// neither the geometry-only MULTILEADER shim nor LEADER's model carries an
/// arrow size, so one fixed value keeps the two consistent.
const ARROWHEAD_SIZE: f64 = 2.5;

/// Fixed engineering-isometric projection (30 degrees) for the wireframe
/// renderer. Not configurable -- a best-effort view, not a real camera.
fn project_isometric(p: &Point3D) -> (f64, f64) {
    let cos30 = (std::f64::consts::PI / 6.0).cos();
    let sin30 = (std::f64::consts::PI / 6.0).sin();
    ((p.x - p.z) * cos30, p.y + (p.x + p.z) * sin30)
}

/// One `<line>` per edge, isometrically projected -- shared by 3DSOLID, REGION
/// and POLYLINE_PFACE, which all reduce to a set of 3D edges.
fn wireframe_element(edges: &[[Point3D; 2]], color: &str, ctx: &mut Ctx) -> String {
    edges
        .iter()
        .map(|[a, b]| {
            let (x1, y1) = project_isometric(a);
            let (x2, y2) = project_isometric(b);
            ctx.consider(x1, y1);
            ctx.consider(x2, y2);
            format!(
                "<line x1=\"{x1}\" y1=\"{}\" x2=\"{x2}\" y2=\"{}\" stroke=\"{color}\"/>",
                neg(y1),
                neg(y2)
            )
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// Projects an MLINE's centerline vertices onto the parallel line `offset`
/// away from it. `offset == 0.0` reproduces the centerline itself, which is
/// why the caller uses this for both the real-offset and the
/// no-resolvable-MLINESTYLE fallback case.
fn mline_offset_points(vertices: &[MLineVertex], offset: f64) -> Vec<Point2D> {
    vertices
        .iter()
        .map(|v| Point2D {
            x: v.point.x + v.miter_direction.x * offset,
            y: v.point.y + v.miter_direction.y * offset,
        })
        .collect()
}

/// The entity's colour as drawn: AutoCAD's precedence rules, then darkened
/// if it would not read on the white page (`contrast_on_white`).
fn resolve_entity_color(common: &EntityCommon, ctx: &Ctx) -> String {
    contrast_on_white(&resolve_color(
        common.color_index,
        common.true_color,
        &common.layer,
        ctx.tables,
        &ctx.inherited_color,
    ))
}

// --- entity rendering --------------------------------------------------

/// Looks `block_name` up in `ctx.tables.block_records` and recursively renders
/// its entities under a nested transform, wrapped in a
/// `<g transform="matrix(...)">`.
///
/// ATTDEF children are skipped: an attribute *template* is not drawn, and the
/// real values are separate top-level ATTRIB entities already rendered.
fn render_block_ref(
    block_name: &str,
    insertion_point: Point2D,
    x_scale: f64,
    y_scale: f64,
    rotation: f64,
    color: &str,
    ctx: &mut Ctx,
) -> String {
    let Some(block) = ctx.tables.block_records.get(block_name) else {
        return String::new();
    };
    if block.entities.is_empty() || ctx.depth > MAX_BLOCK_REF_DEPTH || ctx.block_ref_budget == 0 {
        return String::new();
    }
    ctx.block_ref_budget -= 1;

    let raw_scale = ctx.scale * (x_scale * y_scale).abs().sqrt();
    let cumulative_scale = if raw_scale.is_finite() && raw_scale > 0.0 {
        raw_scale
    } else {
        ctx.scale
    };

    let child_transform = Transform {
        insertion_point,
        x_scale,
        y_scale,
        rotation,
    };
    // Compose: local (within the block) -> world, via this block's own
    // transform evaluated in the parent's already-established space. The
    // parent's own state is restored afterwards.
    let parent_transform = ctx.transform;
    let parent_depth = ctx.depth;
    let parent_scale = ctx.scale;
    let parent_inherited = std::mem::replace(&mut ctx.inherited_color, color.to_string());

    ctx.transform = compose(&parent_transform, &child_transform);
    ctx.depth = parent_depth + 1;
    ctx.scale = cumulative_scale;

    let mut body_parts = Vec::new();
    for child in &block.entities {
        if matches!(child, Entity::Attdef(_)) {
            continue;
        }
        if let Some(svg) = render_entity(child, ctx) {
            body_parts.push(svg);
        }
    }

    ctx.transform = parent_transform;
    ctx.depth = parent_depth;
    ctx.scale = parent_scale;
    ctx.inherited_color = parent_inherited;

    if body_parts.is_empty() {
        return String::new();
    }

    // The parent transform is baked into ctx.transform for *bounds* purposes
    // (world-space consider()), but the emitted matrix is only this block's own
    // local transform -- nesting is expressed by nested <g> elements.
    let [a, b, c, d, e, f] = child_transform.svg_matrix();
    format!(
        "<g transform=\"matrix({a} {b} {c} {d} {e} {f})\" stroke-width=\"{}\">\n  {}\n</g>",
        stroke_width_placeholder(cumulative_scale),
        body_parts.join("\n  ")
    )
}

/// Renders one entity, or returns `None` if there is nothing to draw (with the
/// type recorded in `ctx.unsupported` when that is because the renderer has no
/// support for it). Bounds are extended through `ctx.consider*` as it goes.
///
/// All coordinates in the emitted SVG stay in *local* space -- the enclosing
/// `<g transform>` from [`render_block_ref`] does the visual repositioning,
/// while `consider` separately tracks world-space bounds through that same
/// transform.
fn render_entity(e: &Entity, ctx: &mut Ctx) -> Option<String> {
    let color = resolve_entity_color(e.common(), ctx);
    match e {
        Entity::Line(l) => {
            ctx.consider(l.start_point.x, l.start_point.y);
            ctx.consider(l.end_point.x, l.end_point.y);
            Some(format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\"/>",
                l.start_point.x,
                neg(l.start_point.y),
                l.end_point.x,
                neg(l.end_point.y)
            ))
        }
        Entity::Circle(c) => {
            ctx.consider(c.center.x - c.radius, c.center.y - c.radius);
            ctx.consider(c.center.x + c.radius, c.center.y + c.radius);
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                c.center.x,
                neg(c.center.y),
                c.radius
            ))
        }
        Entity::Arc(a) => {
            ctx.consider(a.center.x - a.radius, a.center.y - a.radius);
            ctx.consider(a.center.x + a.radius, a.center.y + a.radius);
            let (x, y, r) = (a.center.x, a.center.y, a.radius);
            let (x1, y1) = (x + r * a.start_angle.cos(), y + r * a.start_angle.sin());
            let (x2, y2) = (x + r * a.end_angle.cos(), y + r * a.end_angle.sin());
            let mut sweep = a.end_angle - a.start_angle;
            if sweep < 0.0 {
                sweep += 2.0 * std::f64::consts::PI;
            }
            let large = if sweep > std::f64::consts::PI { 1 } else { 0 };
            Some(format!(
                "<path d=\"M {x1} {} A {r} {r} 0 {large} 0 {x2} {}\" fill=\"none\" stroke=\"{color}\"/>",
                neg(y1), neg(y2)
            ))
        }
        Entity::Ellipse(el) => {
            let rx = el.major_axis_endpoint.x.hypot(el.major_axis_endpoint.y);
            ctx.consider(el.center.x - rx, el.center.y - rx);
            ctx.consider(el.center.x + rx, el.center.y + rx);
            let ry = rx * el.axis_ratio;
            let rot = el
                .major_axis_endpoint
                .y
                .atan2(el.major_axis_endpoint.x)
                .to_degrees();
            let (cx, cy) = (el.center.x, neg(el.center.y));
            Some(format!(
                "<ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{rx}\" ry=\"{ry}\" transform=\"rotate({} {cx} {cy})\" fill=\"none\" stroke=\"{color}\"/>",
                neg(rot)
            ))
        }
        Entity::LwPolyline(p) | Entity::Polyline2D(p) => {
            if p.bulges.is_empty() {
                ctx.consider_all(&p.vertices);
                Some(polyline_element(&p.vertices, p.closed, &color))
            } else {
                if let Some((min_x, min_y, max_x, max_y)) =
                    crate::geom::polyline_bounds(&p.vertices, &p.bulges, p.closed)
                {
                    ctx.consider(min_x, min_y);
                    ctx.consider(max_x, max_y);
                }
                Some(bulged_polyline_element(p, &color))
            }
        }
        Entity::Polyline3D(p) => {
            if p.vertices.is_empty() {
                return None;
            }
            ctx.consider_all_3d(&p.vertices);
            Some(polyline_element(&xy(&p.vertices), p.closed, &color))
        }
        Entity::Text(t) => {
            let anchor = text_anchor(
                t.start_point,
                t.alignment_point,
                t.horizontal_alignment,
                t.vertical_alignment,
            );
            ctx.consider(anchor.at.x, anchor.at.y);
            Some(text_element(
                &anchor,
                t.text_height,
                t.rotation,
                &color,
                &t.text_plain,
            ))
        }
        Entity::Attrib(a) => {
            let anchor = text_anchor(
                a.start_point,
                a.alignment_point,
                a.horizontal_alignment,
                a.vertical_alignment,
            );
            ctx.consider(anchor.at.x, anchor.at.y);
            if a.text.is_empty() || a.invisible {
                return Some(String::new());
            }
            Some(text_element(
                &anchor,
                a.text_height,
                a.rotation,
                &color,
                &a.text_plain,
            ))
        }
        Entity::Tolerance(t) => {
            ctx.consider(t.insertion_point.x, t.insertion_point.y);
            if t.text_value.is_empty() {
                return Some(String::new());
            }
            let anchor = TextAnchor {
                at: Point2D {
                    x: t.insertion_point.x,
                    y: t.insertion_point.y,
                },
                anchor: "start",
                baseline_drop: 0.0,
            };
            Some(text_element(
                &anchor,
                t.text_height,
                0.0,
                &color,
                &t.text_plain,
            ))
        }
        Entity::MText(m) => {
            ctx.consider(m.insertion_point.x, m.insertion_point.y);
            let lines: Vec<&str> = m.text_plain.lines().filter(|l| !l.is_empty()).collect();
            if lines.is_empty() {
                return Some(String::new());
            }
            // A stored 0 means "unset" at render time (the parsed value is
            // legitimately 0 in real files), not at parse time.
            let text_height = if m.text_height == 0.0 {
                1.0
            } else {
                m.text_height
            };
            let line_spacing_factor = if m.line_spacing_factor == 0.0 {
                1.0
            } else {
                m.line_spacing_factor
            };
            let line_height = text_height * line_spacing_factor * 1.2;
            // The attachment point is a corner or edge of the text block
            // (DXF 71, 1 = top-left ... 9 = bottom-right): columns pick the
            // SVG anchor, rows where the first baseline sits relative to the
            // insertion point (a cap height is ~0.72 em, a descender ~0.2).
            let column = (m.attachment.clamp(1, 9) - 1) % 3;
            let row = (m.attachment.clamp(1, 9) - 1) / 3;
            let anchor_attr = match column {
                1 => " text-anchor=\"middle\"",
                2 => " text-anchor=\"end\"",
                _ => "",
            };
            let block_height = line_height * (lines.len() as f64 - 1.0) + text_height;
            let first_baseline_drop = match row {
                0 => 0.72 * text_height,
                1 => 0.72 * text_height - block_height / 2.0,
                _ => 0.72 * text_height - block_height + 0.2 * text_height,
            };
            let (x, y) = (m.insertion_point.x, neg(m.insertion_point.y));
            let mut tspans = String::new();
            for (i, line) in lines.iter().enumerate() {
                let dy = if i == 0 {
                    first_baseline_drop
                } else {
                    line_height
                };
                let _ = write!(
                    tspans,
                    "<tspan x=\"{x}\" dy=\"{dy}\">{}</tspan>",
                    escape_xml(line)
                );
            }
            Some(format!(
                "<text x=\"{x}\" y=\"{y}\" font-size=\"{text_height}\" fill=\"{color}\" stroke=\"none\"{anchor_attr}{}>{tspans}</text>",
                rotate_transform_attr(m.rotation, x, y)
            ))
        }
        Entity::Point(p) => {
            ctx.consider(p.position.x, p.position.y);
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"{color}\" stroke=\"none\"/>",
                p.position.x,
                neg(p.position.y)
            ))
        }
        Entity::Solid(s) => {
            // Classic AutoCAD SOLID vertex order is 1-2-4-3, not 1-2-3-4.
            let pts = [s.corner1, s.corner2, s.corner4, s.corner3];
            ctx.consider_all(&pts);
            Some(format!(
                "<polygon points=\"{}\" fill=\"{color}\" fill-opacity=\"0.6\" stroke=\"none\"/>",
                points_attr(&pts)
            ))
        }
        Entity::Face3D(f) => {
            // Unlike SOLID, 3DFACE's 4 corners are already sequential.
            // Edge-visibility flag bits are ignored; all 4 edges always draw.
            let pts = xy(&[f.corner1, f.corner2, f.corner3, f.corner4]);
            ctx.consider_all(&pts);
            Some(format!(
                "<polygon points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                points_attr(&pts)
            ))
        }
        Entity::Ray(r) | Entity::XLine(r) => {
            let is_xline = matches!(e, Entity::XLine(_));
            ctx.consider(r.point.x, r.point.y);
            let len = 1e6;
            let (dx, dy) = (r.vector.x * len, r.vector.y * len);
            let (x1, y1) = if is_xline {
                (r.point.x - dx, r.point.y - dy)
            } else {
                (r.point.x, r.point.y)
            };
            let (x2, y2) = (r.point.x + dx, r.point.y + dy);
            Some(format!(
                "<line x1=\"{x1}\" y1=\"{}\" x2=\"{x2}\" y2=\"{}\" stroke-dasharray=\"4,2\" stroke=\"{color}\"/>",
                neg(y1), neg(y2)
            ))
        }
        Entity::Insert(i) => {
            // A block reference in a mirrored OCS (normal (0,0,-1)) is the
            // block drawn at its world insertion point with the x scale and
            // the rotation negated: mirror . rotate(a) . scale(sx, sy) =
            // rotate(-a) . scale(-sx, sy).
            let (x_scale, rotation) = if i.extrusion.z < 0.0 {
                (-i.scale.x, -i.rotation)
            } else {
                (i.scale.x, i.rotation)
            };
            Some(render_block_ref(
                &i.block_name,
                Point2D {
                    x: i.insertion_point.x,
                    y: i.insertion_point.y,
                },
                x_scale,
                i.scale.y,
                rotation,
                &color,
                ctx,
            ))
        }
        Entity::AcadTable(a) => Some(render_block_ref(
            &a.block_name,
            Point2D {
                x: a.insertion_point.x,
                y: a.insertion_point.y,
            },
            a.scale.x,
            a.scale.y,
            a.rotation,
            &color,
            ctx,
        )),
        Entity::Dimension(d) => {
            // The cached geometry block is already in final world coordinates,
            // so it is drawn with an identity transform.
            let svg = render_block_ref(
                &d.block_name,
                Point2D { x: 0.0, y: 0.0 },
                1.0,
                1.0,
                0.0,
                &color,
                ctx,
            );
            if svg.is_empty() {
                ctx.unsupported.insert("DIMENSION".to_string());
                return None;
            }
            Some(svg)
        }
        Entity::Viewport(v) => {
            // Not real drawing geometry (it is a window onto model space), but
            // drawing its frame is a reasonable, honest representation.
            let (cx, cy) = (v.center.x, v.center.y);
            let (hw, hh) = (v.width / 2.0, v.height / 2.0);
            let corners = [
                Point2D {
                    x: cx - hw,
                    y: cy - hh,
                },
                Point2D {
                    x: cx + hw,
                    y: cy - hh,
                },
                Point2D {
                    x: cx + hw,
                    y: cy + hh,
                },
                Point2D {
                    x: cx - hw,
                    y: cy + hh,
                },
            ];
            ctx.consider_all(&corners);
            Some(dashed_outline(&corners, &color, "2,2"))
        }
        Entity::Wipeout(w) => {
            // Outline only, not filled: a filled shape would mask whatever is
            // drawn under it. That is arguably WIPEOUT's real effect, but this
            // renderer's simple in-order painting cannot be trusted to
            // reproduce it, and an unexpectedly opaque box is a worse failure
            // than a merely incomplete outline.
            if w.boundary.len() < 2 {
                ctx.unsupported.insert("WIPEOUT".to_string());
                return None;
            }
            ctx.consider_all(&w.boundary);
            Some(dashed_outline(&w.boundary, &color, "2,2"))
        }
        Entity::Spline(s) => {
            // Straight-line approximation through fit points (preferred, since
            // they lie exactly on the curve) or control points -- not a real
            // NURBS evaluation.
            let pts = if !s.fit_points.is_empty() {
                &s.fit_points
            } else {
                &s.control_points
            };
            if pts.len() < 2 {
                return None;
            }
            ctx.consider_all_3d(pts);
            Some(polyline_element(&xy(pts), false, &color))
        }
        Entity::Solid3D(s) => render_wireframe_entity(&s.wireframe_edges, "3DSOLID", &color, ctx),
        Entity::Region(r) => render_wireframe_entity(&r.wireframe_edges, "REGION", &color, ctx),
        Entity::PolylinePFace(p) => {
            render_wireframe_entity(&p.wireframe_edges, "POLYLINE_PFACE", &color, ctx)
        }
        Entity::Hatch(h) => hatch::render_hatch(h, &color, ctx),
        Entity::Leader(l) => {
            if l.vertices.is_empty() {
                return None;
            }
            ctx.consider_all_3d(&l.vertices);
            let pts = xy(&l.vertices);
            let line = polyline_element(&pts, false, &color);
            let arrow = if l.has_arrowhead && pts.len() >= 2 {
                arrowhead_element(&pts[0], &pts[1], ARROWHEAD_SIZE, &color)
            } else {
                String::new()
            };
            Some(line + &arrow)
        }
        Entity::MultiLeader(m) => {
            // An arrowhead is drawn on every line, unconditionally: the real
            // per-line visibility flag lives in LEADER_Line.flags, which the
            // geometry-only shim this reads from does not extract.
            let mut parts = Vec::new();
            for line in &m.lines {
                if line.len() < 2 {
                    continue;
                }
                ctx.consider_all_3d(line);
                let pts = xy(line);
                parts.push(polyline_element(&pts, false, &color));
                let n = pts.len();
                parts.push(arrowhead_element(
                    &pts[n - 1],
                    &pts[n - 2],
                    ARROWHEAD_SIZE,
                    &color,
                ));
            }
            (!parts.is_empty()).then(|| parts.join("\n  "))
        }
        Entity::MLine(l) => {
            if l.vertices.is_empty() {
                return None;
            }
            for v in &l.vertices {
                ctx.consider(v.point.x, v.point.y);
            }
            // A single 0.0 offset is exactly the centerline-only fallback:
            // point + miter_direction * 0.0 is just point.
            let offsets = match ctx.tables.mlinestyles.get(&l.mlinestyle_name) {
                Some(offsets) if !offsets.is_empty() => offsets.clone(),
                _ => vec![0.0],
            };
            let lines: Vec<String> = offsets
                .iter()
                .map(|&offset| {
                    polyline_element(&mline_offset_points(&l.vertices, offset), l.closed, &color)
                })
                .collect();
            Some(lines.join("\n  "))
        }
        Entity::Light(l) => {
            ctx.consider(l.position.x, l.position.y);
            let marker = format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"none\" stroke=\"{color}\"/>",
                l.position.x,
                neg(l.position.y)
            );
            if !l.has_target {
                return Some(marker);
            }
            ctx.consider(l.target.x, l.target.y);
            let line = format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-dasharray=\"1,1\" stroke=\"{color}\"/>",
                l.position.x, neg(l.position.y), l.target.x, neg(l.target.y)
            );
            Some(format!("{marker}\n  {line}"))
        }
        // Reaching here means an ATTDEF outside a block reference -- a
        // template with no instance, so there is nothing to draw.
        Entity::Attdef(_) => {
            ctx.unsupported.insert("ATTDEF".to_string());
            None
        }
        Entity::Unknown { type_name, .. } => {
            ctx.unsupported.insert(type_name.clone());
            None
        }
    }
}

/// Wireframe entities share one shape: draw the edges, or report the type as
/// unsupported when extraction produced none.
fn render_wireframe_entity(
    edges: &[[Point3D; 2]],
    type_name: &str,
    color: &str,
    ctx: &mut Ctx,
) -> Option<String> {
    if edges.is_empty() {
        ctx.unsupported.insert(type_name.to_string());
        return None;
    }
    Some(wireframe_element(edges, color, ctx))
}

/// `db.entities` only ever contains model and paper space (see `convert.rs`),
/// so [`Space::All`] returns everything; the other two narrow by block name.
fn select_entities_for_space(db: &CadDatabase, space: Space) -> Vec<&Entity> {
    if space == Space::All {
        return db.entities.iter().collect();
    }
    let mut handles: HashSet<&str> = HashSet::new();
    for (name, record) in &db.tables.block_records {
        let upper = name.to_uppercase();
        let matches = match space {
            Space::Model => upper == "*MODEL_SPACE",
            Space::Paper => upper.starts_with("*PAPER_SPACE"),
            Space::All => unreachable!(),
        };
        if !matches {
            continue;
        }
        for e in &record.entities {
            handles.insert(&e.common().handle);
            if let Entity::Insert(insert) = e {
                for a in &insert.attribs {
                    handles.insert(&a.common.handle);
                }
            }
        }
    }
    db.entities
        .iter()
        .filter(|e| handles.contains(e.common().handle.as_str()))
        .collect()
}

// --- top level ---------------------------------------------------------

/// Everything [`to_svg`] computes before the stroke width is known: the
/// rendered elements with their stroke placeholders still in place, the
/// `<defs>` entries HATCH patterns need, the viewBox, and the types nothing
/// was drawn for. [`assemble`] turns it into a document; `png.rs` uses the
/// split to pick a stroke width in output pixels once it knows the scale.
pub(crate) struct Rendered {
    body: String,
    defs: Vec<String>,
    pub(crate) view_box: ViewBox,
    unsupported: HashSet<String>,
}

impl Rendered {
    /// The unsupported type names, sorted so the result is the same on
    /// every run (they come out of a `HashSet`).
    pub(crate) fn unsupported_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self.unsupported.iter().cloned().collect();
        types.sort();
        types
    }
}

/// Renders every selected entity and computes the viewBox, leaving the
/// stroke width unresolved.
///
/// `outlier_trim` (default `true`) computes the viewBox from the dominant
/// spatially-connected cluster of entities instead of the raw min/max -- see
/// [`bounds`] for why.
pub(crate) fn render(db: &CadDatabase, options: ToSvgOptions) -> Rendered {
    let mut entity_boxes: Vec<Box2D> = Vec::new();
    let mut body: Vec<String> = Vec::new();

    let mut ctx = Ctx::new(&db.tables);
    for e in select_entities_for_space(db, options.space) {
        ctx.reset_entity_bounds();
        if let Some(svg) = render_entity(e, &mut ctx) {
            if !svg.is_empty() {
                body.push(svg);
            }
        }
        if let Some(b) = ctx.entity_box() {
            entity_boxes.push(b);
        }
    }

    let raw_bounds = || Box2D {
        min_x: ctx.xs.iter().cloned().fold(f64::INFINITY, f64::min),
        max_x: ctx.xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        min_y: ctx.ys.iter().cloned().fold(f64::INFINITY, f64::min),
        max_y: ctx.ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    };
    let bounds = if ctx.xs.is_empty() {
        Box2D {
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
        }
    } else if options.outlier_trim && entity_boxes.len() > 2 {
        dominant_cluster_box(&entity_boxes).unwrap_or_else(raw_bounds)
    } else {
        raw_bounds()
    };

    let width = (bounds.max_x - bounds.min_x) + options.padding * 2.0;
    let height = (bounds.max_y - bounds.min_y) + options.padding * 2.0;
    let view_box = ViewBox {
        x: bounds.min_x - options.padding,
        y: -bounds.max_y - options.padding,
        // A degenerate (single-point) drawing still gets a 1x1 canvas.
        width: if width != 0.0 { width } else { 1.0 },
        height: if height != 0.0 { height } else { 1.0 },
    };

    Rendered {
        body: body.join("\n  "),
        defs: ctx.defs,
        view_box,
        unsupported: ctx.unsupported,
    }
}

/// The stroke width [`to_svg`] uses when none is given: ~1/6000th of the
/// viewBox diagonal in drawing units, floored at 0.01. A hairline at most
/// output sizes -- `png.rs` overrides it with a width in pixels.
pub(crate) fn auto_stroke_width(view_box: &ViewBox) -> f64 {
    (view_box.width.hypot(view_box.height) / 6000.0).max(0.01)
}

/// Resolves the stroke placeholders at `effective_stroke_width` (drawing
/// units; see [`stroke_width_placeholder`] for how nested block references
/// keep a constant visual weight) and wraps everything in the `<svg>`
/// element.
pub(crate) fn assemble(rendered: &Rendered, effective_stroke_width: f64) -> String {
    let resolved_body = resolve_stroke_widths(&rendered.body, effective_stroke_width);
    // HATCH pattern defs carry stroke-width placeholders too. Kept separate
    // from the body only so an empty defs list emits no <defs> block at all.
    let defs_block = if rendered.defs.is_empty() {
        String::new()
    } else {
        let resolved_defs =
            resolve_stroke_widths(&rendered.defs.join("\n  "), effective_stroke_width);
        format!("<defs>\n  {resolved_defs}\n</defs>\n  ")
    };
    let vb = rendered.view_box;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" stroke=\"black\" stroke-width=\"{effective_stroke_width}\">\n  {defs_block}{resolved_body}\n</svg>",
        vb.x, vb.y, vb.width, vb.height
    )
}

/// Renders a parsed [`CadDatabase`] to an SVG string: [`render`], then
/// [`assemble`] with the explicit `stroke_width` or [`auto_stroke_width`].
pub(crate) fn to_svg(db: &CadDatabase, options: ToSvgOptions) -> ToSvgResult {
    let rendered = render(db, options);
    let stroke_width = options
        .stroke_width
        .unwrap_or_else(|| auto_stroke_width(&rendered.view_box));
    ToSvgResult {
        svg: assemble(&rendered, stroke_width),
        view_box: rendered.view_box,
        unsupported_types: rendered.unsupported_types(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bounds::diag;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ~= {b}");
    }

    #[test]
    fn render_block_ref_budget_caps_combinatorial_blowup_from_self_referencing_blocks() {
        use crate::model::InsertEntity;
        use crate::tables::BlockRecord;
        use std::collections::BTreeMap;

        // Block "R" contains 5 INSERTs of itself. The depth cap alone bounds
        // recursion depth, but a full 5-ary tree 20 levels deep is 5^20
        // (~9.5e13) block instantiations, which would never finish.
        let common = EntityCommon {
            handle: String::new(),
            layer: String::new(),
            color_index: 0,
            true_color: None,
        };
        let children: Vec<Entity> = (0..5)
            .map(|i| {
                Entity::Insert(InsertEntity {
                    common: common.clone(),
                    block_name: "R".to_string(),
                    extrusion: crate::geom::WORLD_Z,
                    insertion_point: Point3D {
                        x: i as f64,
                        y: 0.0,
                        z: 0.0,
                    },
                    scale: Point3D {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                    rotation: 0.0,
                    attribs: Vec::new(),
                })
            })
            .collect();
        let mut block_records = BTreeMap::new();
        block_records.insert(
            "R".to_string(),
            BlockRecord {
                name: "R".to_string(),
                entities: children,
            },
        );
        let tables = Tables {
            block_records,
            ..Default::default()
        };

        let mut ctx = Ctx::new(&tables);
        let svg = render_block_ref(
            "R",
            Point2D { x: 0.0, y: 0.0 },
            1.0,
            1.0,
            0.0,
            DEFAULT_COLOR,
            &mut ctx,
        );
        assert!(
            !svg.is_empty(),
            "the shallow levels within budget should still render something"
        );
        assert_eq!(
            ctx.block_ref_budget, 0,
            "the budget, not the depth cap, should be what stopped this combinatorial blowup"
        );
    }

    #[test]
    fn consider_drops_non_finite_coordinates_instead_of_recording_them() {
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        ctx.consider(1.0, 2.0);
        ctx.consider(3.0, f64::INFINITY);
        ctx.consider(f64::NAN, 4.0);
        // Only the fully-finite point should have been recorded: a box
        // corrupted by a non-finite coordinate used to leave min == max ==
        // Infinity, whose diagonal is NaN, which panics the comparisons in
        // `bounds`.
        let b = ctx.entity_box().expect("one finite point was recorded");
        close(b.min_x, 1.0);
        close(b.max_x, 1.0);
        close(b.min_y, 2.0);
        close(b.max_y, 2.0);
        assert!(diag(&b).is_finite());
    }

    #[test]
    fn resolve_stroke_widths_divides_by_the_embedded_scale() {
        let out = resolve_stroke_widths("prefix @@SW@@2@@ middle @@SW@@0.5@@ suffix", 10.0);
        assert_eq!(out, "prefix 5 middle 20 suffix");
    }

    #[test]
    fn resolve_stroke_widths_emits_a_malformed_placeholder_verbatim() {
        let out = resolve_stroke_widths("before @@SW@@2 no closing marker", 10.0);
        assert_eq!(out, "before @@SW@@2 no closing marker");
    }

    #[test]
    fn transform_identity_apply_is_a_no_op() {
        let (x, y) = Transform::identity().apply(3.0, 4.0);
        close(x, 3.0);
        close(y, 4.0);
    }

    #[test]
    fn transform_apply_scales_rotates_then_translates() {
        let t = Transform {
            insertion_point: Point2D { x: 10.0, y: 20.0 },
            x_scale: 2.0,
            y_scale: 2.0,
            rotation: 0.0,
        };
        let (x, y) = t.apply(1.0, 1.0);
        close(x, 12.0);
        close(y, 22.0);
    }

    #[test]
    fn compose_applies_child_transform_within_parents_space() {
        let parent = Transform {
            insertion_point: Point2D { x: 10.0, y: 0.0 },
            x_scale: 1.0,
            y_scale: 1.0,
            rotation: 0.0,
        };
        let child = Transform {
            insertion_point: Point2D { x: 1.0, y: 1.0 },
            x_scale: 2.0,
            y_scale: 2.0,
            rotation: 0.0,
        };
        let composed = compose(&parent, &child);
        close(composed.insertion_point.x, 11.0);
        close(composed.insertion_point.y, 1.0);
        close(composed.x_scale, 2.0);
        close(composed.y_scale, 2.0);
    }

    #[test]
    fn mline_offset_points_zero_offset_is_the_centerline() {
        let verts = vec![
            MLineVertex {
                point: Point3D {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                },
                miter_direction: Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            MLineVertex {
                point: Point3D {
                    x: 3.0,
                    y: 4.0,
                    z: 0.0,
                },
                miter_direction: Point3D {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
        ];
        let pts = mline_offset_points(&verts, 0.0);
        close(pts[0].x, 1.0);
        close(pts[0].y, 2.0);
        close(pts[1].x, 3.0);
        close(pts[1].y, 4.0);
    }

    #[test]
    fn mline_offset_points_displaces_along_miter_direction() {
        let verts = vec![MLineVertex {
            point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            miter_direction: Point3D {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        }];
        let pts = mline_offset_points(&verts, 5.0);
        close(pts[0].x, 0.0);
        close(pts[0].y, 5.0);

        let pts_negative = mline_offset_points(&verts, -5.0);
        close(pts_negative[0].y, -5.0);
    }
}
