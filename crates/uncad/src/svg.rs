//! SVG rendering of a parsed [`CadDatabase`].
//!
//! Entity coverage matches [`crate::model::Entity`]'s variants; every other
//! type arrives as `Entity::Unknown`, whether or not it has geometry. Anything
//! this renderer cannot draw is reported through
//! [`ToSvgResult::unsupported_types`] rather than dropped silently. The one
//! type that will stay there for good is `ACAD_PROXY_ENTITY`: an opaque
//! per-app serialized blob with no geometry to draw at all.
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

use crate::color::{resolve_color, DEFAULT_COLOR};
use crate::dynapi::{Point2D, Point3D};
use crate::model::{Entity, EntityCommon, MLineVertex};
use crate::tables::Tables;
use crate::CadDatabase;
use bounds::{dominant_cluster_box, Box2D};
use format::{escape_xml, neg, points_attr, rotate_transform_attr, strip_mtext_formatting, xy};
use std::collections::BTreeSet;
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
    /// DXF names of entity types this renderer had nothing to draw for,
    /// sorted by name so the same input always reports them in the same order.
    pub unsupported_types: Vec<String>,
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
    // Copied straight into the public result, in this set's (sorted) order.
    unsupported: BTreeSet<String>,
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
            unsupported: BTreeSet::new(),
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

/// A dashed outline, used for the shapes this renderer draws as an indication
/// rather than as real geometry (VIEWPORT frames, WIPEOUT boundaries).
fn dashed_outline(pts: &[Point2D], color: &str, dash: &str) -> String {
    format!(
        "<polygon points=\"{}\" fill=\"none\" stroke-dasharray=\"{dash}\" stroke=\"{color}\"/>",
        points_attr(pts)
    )
}

/// A single-line `<text>` at an already-y-flipped position -- TEXT, ATTRIB and
/// TOLERANCE all render to this.
fn text_element(at: Point2D, height: f64, rotation: f64, color: &str, text: &str) -> String {
    let (x, y) = (at.x, neg(at.y));
    format!(
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{height}\" fill=\"{color}\" stroke=\"none\"{}>{}</text>",
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

fn resolve_entity_color(common: &EntityCommon, ctx: &Ctx) -> String {
    resolve_color(
        common.color_index,
        common.true_color,
        &common.layer,
        ctx.tables,
        &ctx.inherited_color,
    )
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
            ctx.consider_all(&p.vertices);
            Some(polyline_element(&p.vertices, p.closed, &color))
        }
        Entity::Polyline3D(p) => {
            if p.vertices.is_empty() {
                return None;
            }
            ctx.consider_all_3d(&p.vertices);
            Some(polyline_element(&xy(&p.vertices), p.closed, &color))
        }
        Entity::Text(t) => {
            ctx.consider(t.start_point.x, t.start_point.y);
            Some(text_element(
                t.start_point,
                t.text_height,
                t.rotation,
                &color,
                &t.text,
            ))
        }
        Entity::Attrib(a) => {
            ctx.consider(a.start_point.x, a.start_point.y);
            if a.text.is_empty() {
                return Some(String::new());
            }
            Some(text_element(
                a.start_point,
                a.text_height,
                a.rotation,
                &color,
                &a.text,
            ))
        }
        Entity::Tolerance(t) => {
            ctx.consider(t.insertion_point.x, t.insertion_point.y);
            if t.text_value.is_empty() {
                return Some(String::new());
            }
            Some(text_element(
                Point2D {
                    x: t.insertion_point.x,
                    y: t.insertion_point.y,
                },
                t.text_height,
                0.0,
                &color,
                &t.text_value,
            ))
        }
        Entity::MText(m) => {
            ctx.consider(m.insertion_point.x, m.insertion_point.y);
            let stripped = strip_mtext_formatting(&m.text);
            let lines: Vec<&str> = stripped.lines().filter(|l| !l.is_empty()).collect();
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
            let (x, y) = (m.insertion_point.x, neg(m.insertion_point.y));
            let mut tspans = String::new();
            for (i, line) in lines.iter().enumerate() {
                let dy = if i == 0 { 0.0 } else { line_height };
                let _ = write!(
                    tspans,
                    "<tspan x=\"{x}\" dy=\"{dy}\">{}</tspan>",
                    escape_xml(line)
                );
            }
            Some(format!(
                "<text x=\"{x}\" y=\"{y}\" font-size=\"{text_height}\" fill=\"{color}\" stroke=\"none\" transform=\"rotate({} {x} {y})\">{tspans}</text>",
                neg(m.rotation.to_degrees())
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
        Entity::Solid(s) | Entity::Trace(s) => {
            // Classic AutoCAD SOLID/TRACE vertex order is 1-2-4-3, not 1-2-3-4.
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
        Entity::Insert(i) => Some(render_block_ref(
            &i.block_name,
            Point2D {
                x: i.insertion_point.x,
                y: i.insertion_point.y,
            },
            i.scale.x,
            i.scale.y,
            i.rotation,
            &color,
            ctx,
        )),
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
    let mut handles: BTreeSet<&str> = BTreeSet::new();
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

/// Renders a parsed [`CadDatabase`] to an SVG string.
///
/// `outlier_trim` (default `true`) computes the viewBox from the dominant
/// spatially-connected cluster of entities instead of the raw min/max -- see
/// [`bounds`] for why.
///
/// `stroke_width` defaults to ~1/6000th of the computed viewBox diagonal
/// rather than a fixed value; see [`stroke_width_placeholder`] for how nested
/// block references keep a constant visual weight.
pub(crate) fn to_svg(db: &CadDatabase, options: ToSvgOptions) -> ToSvgResult {
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

    let x = bounds.min_x - options.padding;
    let y = -bounds.max_y - options.padding;
    let width = (bounds.max_x - bounds.min_x) + options.padding * 2.0;
    let height = (bounds.max_y - bounds.min_y) + options.padding * 2.0;
    let effective_stroke_width = options
        .stroke_width
        .unwrap_or_else(|| (width.hypot(height) / 6000.0).max(0.01));

    let resolved_body = resolve_stroke_widths(&body.join("\n  "), effective_stroke_width);
    // HATCH pattern defs carry stroke-width placeholders too. Kept separate
    // from the body only so an empty defs list emits no <defs> block at all.
    let defs_block = if ctx.defs.is_empty() {
        String::new()
    } else {
        let resolved_defs = resolve_stroke_widths(&ctx.defs.join("\n  "), effective_stroke_width);
        format!("<defs>\n  {resolved_defs}\n</defs>\n  ")
    };

    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x} {y} {} {}\" stroke=\"black\" stroke-width=\"{effective_stroke_width}\">\n  {defs_block}{resolved_body}\n</svg>",
        if width != 0.0 { width } else { 1.0 },
        if height != 0.0 { height } else { 1.0 }
    );

    ToSvgResult {
        svg,
        unsupported_types: ctx.unsupported.into_iter().collect(),
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
