//! Ported from `src/index.mjs`'s `toSVG()`/`renderEntity`/`renderBlockRef`/
//! `selectEntitiesForSpace`/`clusterEntityBoxes`/`dominantClusterBox`.
//! Every algorithm here is a deliberate line-for-line port, not a
//! redesign -- the JS version's outlier-trim/stroke-width/color logic
//! already fixed real-file regressions once (see git history), so this
//! port keeps the same behavior rather than rediscovering them.
//!
//! Entity coverage matches [`crate::render_model::RenderEntity`]'s current variants,
//! which as of LEADER/POLYLINE_2D covers every type the JS baseline's own
//! `toSVG()` ever drew a `case` for. `ACAD_PROXY_ENTITY` is the one type
//! left permanently unsupported: it's an opaque per-app serialized blob
//! with no fixed geometry to draw at all, not something worth adding a
//! no-op variant for -- `Unknown` already reports its real dxfname. See
//! `docs/CAVEATS.md` for the full picture. `Unknown` (including ATTDEF
//! outside a block) is reported via `unsupported_entity_types` rather
//! than silently dropped, matching `toSVG(db, {report:true})`.
//!
//! MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE,
//! WIPEOUT, and LIGHT are deliberate exceptions: new functionality, not a
//! JS-baseline port (the JS baseline never rendered any of them, and
//! never even parsed REGION/POLYLINE_PFACE/LIGHT) -- see each type's own
//! doc comment in model.rs and `docs/CAVEATS.md` for scope/risk notes
//! specific to each (WIPEOUT in particular carries an extra one: an
//! unverified pixel-to-world transform, not just "untested against a real
//! file"). ARC_DIMENSION is a narrower, lower-risk case: not rendered by
//! the JS baseline either, but no new rendering logic -- see
//! `dimension_dxfname` in `convert.rs`.

use crate::color::{resolve_color, DEFAULT_COLOR};
use crate::dynapi::{Point2D, Point3D};
use crate::render_model::{
    EntityCommon, HatchBoundaryPath, HatchEdge, HatchGradient, HatchPatternLine, MLineVertex,
    RenderEntity,
};
use crate::tables::Tables;
use crate::CadDatabase;
use regex::Regex;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Space {
    Model,
    Paper,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToSvgOptions {
    pub padding: f64,
    /// `None` = auto-scaled to the computed viewBox (see `to_svg`'s doc).
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
    pub unsupported_entity_types: Vec<String>,
}

// --- transform (renderBlockRef's blockCadTransform, composed across nested INSERTs) ---

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

    /// `blockCadTransform`: local (x,y) -> world (x,y) through this transform.
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (cos, sin) = (self.rotation.cos(), self.rotation.sin());
        (
            self.insertion_point.x + self.x_scale * cos * x - self.y_scale * sin * y,
            self.insertion_point.y + self.x_scale * sin * x + self.y_scale * cos * y,
        )
    }

    /// `blockSvgMatrix`: the SVG `matrix(a b c d e f)` equivalent, composed
    /// with the renderer's CAD-y-up -> SVG-y-down flip.
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

#[derive(Debug, Clone, Copy)]
struct Box2D {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

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
    /// `<pattern>` element definitions accumulated by HATCH rendering (see
    /// `render_hatch_pattern_line`), emitted once into a top-level `<defs>`
    /// by `to_svg`. Persists across `render_block_ref`'s transform
    /// save/restore (like `unsupported`/`xs`/`ys`) since a HATCH can appear
    /// inside a block reference too.
    defs: Vec<String>,
    next_pattern_id: u32,
    /// Remaining budget for `render_block_ref` calls across the whole
    /// render pass, decremented once per call and never restored --
    /// bounds total block-reference work, not just recursion depth.
    /// `render_block_ref`'s own `depth > 20` cap only bounds how deep the
    /// recursion can nest; it does nothing about *breadth*, so a crafted
    /// file with many INSERTs per block at every nested level can still
    /// fan out combinatorially (up to ~N^20 total block instantiations for
    /// N INSERTs per level) before the depth cap is ever reached. This
    /// budget is a hard ceiling on total block-reference work regardless
    /// of how depth and breadth are combined.
    block_ref_budget: u32,
}

/// See [`Ctx::block_ref_budget`]. Generous enough for any real drawing this
/// project has been spot-checked against (low thousands of entities, see
/// `docs/CAVEATS.md`) while still cutting off combinatorial block-reference
/// blowup quickly -- e.g. 10 INSERTs per level exhausts this budget by
/// nesting level 6 (10^6 == 1_000_000), long before `depth`'s own 20-level
/// cap would ever engage.
const BLOCK_REF_BUDGET: u32 = 1_000_000;

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
            next_pattern_id: 0,
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
        if self.ent_min_x.is_finite() {
            Some(Box2D {
                min_x: self.ent_min_x,
                max_x: self.ent_max_x,
                min_y: self.ent_min_y,
                max_y: self.ent_max_y,
            })
        } else {
            None
        }
    }

    /// `ctx.consider(x, y)` -- takes *local* coordinates, applies the
    /// current (possibly block-nested) transform, and records the result.
    ///
    /// Non-finite results (`NaN`/`Infinity`, e.g. from a malformed source
    /// file or a degenerate transform) are dropped rather than recorded:
    /// letting `Infinity` into `ent_min_*`/`ent_max_*` can leave a min and
    /// max both pinned to `Infinity` (the min-side update never fires
    /// because `Infinity < Infinity` is false, while the max-side update
    /// does), and `diag()` later computes `Infinity - Infinity = NaN` from
    /// that box -- which panics the `.partial_cmp(..).unwrap()` calls in
    /// `cluster_entity_boxes`/`dominant_cluster_box` instead of just
    /// rendering a degenerate point.
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
}

/// Snaps a subnormal `f64` (magnitude roughly below 2.2e-308, i.e.
/// anywhere from "a billion times smaller than double's normal epsilon"
/// down to the smallest representable value) to exactly `0.0`. Rust's
/// `f64` `Display` never switches to scientific notation the way JS's
/// `Number.prototype.toString` does, so a subnormal coordinate stringifies
/// as several hundred characters of leading zeros instead of a short one.
///
/// Originally added after finding exactly this on 2 of 9 real AutoCAD
/// samples' SPLINE control points -- which turned out to be a *symptom*,
/// not innocent data: `SplineControlPoint`'s missing `parent` field (see
/// its doc comment in dynapi.rs) meant control points were being read at
/// the wrong stride, reinterpreting stray pointer bytes as coordinates
/// (pointer bit patterns bit-cast to `f64` often land in subnormal
/// range) -- fixed at the real source now. This is kept anyway as a cheap
/// defensive backstop: at subnormal magnitude a value is geometrically
/// and visually indistinguishable from exactly `0` at any realistic
/// drawing scale regardless of *why* it ended up that small, and Rust's
/// unbounded-length Display for such values is a real latent risk for any
/// future code path that ends up formatting a raw memory-read float.
fn clean(x: f64) -> f64 {
    if x != 0.0 && x.is_subnormal() {
        0.0
    } else {
        x
    }
}

/// Negates `x` for the CAD-y-up -> SVG-y-down flip, normalizing `-0.0` to
/// `0.0`. JS's `-y` on a zero coordinate stringifies as `"0"` (JS's
/// `String(-0) === "0"`), but Rust's `f64` `Display` preserves the sign
/// (`"-0"`) -- purely cosmetic (SVG renders `-0`/`0` identically), but
/// normalized here anyway for byte-for-byte parity with the JS baseline's
/// output, confirmed by diffing rendered SVG on example_r14.dwg (an
/// INSERT at rotation 0 produced `matrix(1 -0 0 1 ...)` here vs JS's
/// `matrix(1 0 0 1 ...)` until this was applied). Also runs the value
/// through [`clean`] first -- see its doc comment.
fn neg(x: f64) -> f64 {
    let n = -clean(x);
    if n == 0.0 {
        0.0
    } else {
        n
    }
}

/// Renders a `rotate(deg x y)` SVG `transform` attribute (leading space
/// included, empty string for zero rotation) around the already-SVG-space
/// point `(x, y)` -- shared by TEXT/ATTRIB, matching the sign convention
/// MTEXT's rendering already established: `rot_rad` negated (via [`neg`])
/// because SVG's y-axis points down while DWG angles are measured
/// counter-clockwise in a y-up world, so a positive DWG rotation must
/// become a negative SVG rotation to look the same.
fn rotate_transform_attr(rot_rad: f64, x: f64, y: f64) -> String {
    if rot_rad == 0.0 {
        return String::new();
    }
    let rot_deg = neg(rot_rad * 180.0 / std::f64::consts::PI);
    format!(" transform=\"rotate({rot_deg} {x} {y})\"")
}

/// `stripMtextFormatting`: strips MTEXT's inline formatting codes (`\A`,
/// `\H`, `\W`, `\C`, `\Q`, `\T`, `\f`, `{...}` grouping, `\P` paragraph
/// breaks) down to plain, possibly multi-line text. `\S...;` (stacked/
/// fraction text, e.g. `"1#8"` -> `"1/8"`) is unwrapped rather than dropped
/// since the fraction content itself is meaningful. Best-effort plain-text
/// approximation, not a real MTEXT formatter -- no stacked fractions,
/// color, or font changes are visually reproduced. Same regex passes, same
/// order, as the JS baseline.
fn strip_mtext_formatting(text: &str) -> String {
    static STACKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\S([^;]*);").unwrap());
    static HASH_CARET_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[#^\\]").unwrap());
    static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\P").unwrap());
    static NBSP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\~").unwrap());
    static INLINE_CODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\\[A-Za-z][^;]*;").unwrap());
    static BRACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[{}]").unwrap());
    static DOUBLE_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\\\").unwrap());

    let step1 = STACKED.replace_all(text, |caps: &regex::Captures| {
        HASH_CARET_BACKSLASH.replace_all(&caps[1], "/").into_owned()
    });
    let step2 = PARAGRAPH.replace_all(&step1, "\n");
    let step3 = NBSP.replace_all(&step2, " ");
    let step4 = INLINE_CODE.replace_all(&step3, "");
    let step5 = BRACES.replace_all(&step4, "");
    DOUBLE_BACKSLASH.replace_all(&step5, "\\").into_owned()
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

/// `renderBlockRef`: looks up `block_name` in `ctx.tables.block_records`,
/// recursively renders its entities (skipping ATTDEF -- it's a template,
/// not drawn; the real values are separate top-level ATTRIB entities
/// already rendered directly, exactly like the JS baseline), under a
/// nested transform, wrapped in `<g transform="matrix(...)">` with
/// a stroke-width placeholder (see `to_svg`'s doc for why it can't be
/// resolved until the final viewBox is known).
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
    if block.entities.is_empty() || ctx.depth > 20 || ctx.block_ref_budget == 0 {
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
    // transform evaluated in the *parent's* already-established space
    // (parent's transform is left in ctx.transform and restored after).
    let parent_transform = ctx.transform;
    let parent_depth = ctx.depth;
    let parent_scale = ctx.scale;
    let parent_inherited = std::mem::replace(&mut ctx.inherited_color, color.to_string());

    ctx.transform = compose(&parent_transform, &child_transform);
    ctx.depth = parent_depth + 1;
    ctx.scale = cumulative_scale;

    let mut body_parts = Vec::new();
    for child in &block.entities {
        if matches!(child, RenderEntity::Attdef(_)) {
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

    let [a, b, c, d, e, f] = child_transform.svg_matrix();
    // The parent transform is baked into ctx.transform for *bounds*
    // purposes (world-space consider()), but the emitted SVG `matrix(...)`
    // here is only this block's own local transform -- nesting is
    // expressed by nested `<g>` elements (each INSERT's own matrix),
    // exactly like `renderBlockRef` in the JS source.
    let _ = &a; // (a..f) all used below; silence unused-if-reordered lints
    format!(
        "<g transform=\"matrix({a} {b} {c} {d} {e} {f})\" stroke-width=\"@@SW@@{cumulative_scale}@@\">\n  {}\n</g>",
        body_parts.join("\n  ")
    )
}

/// Composes a parent world-transform with a child's local transform:
/// applying the result to a point equals applying `child` then `parent`.
fn compose(parent: &Transform, child: &Transform) -> Transform {
    let (px, py) = parent.apply(child.insertion_point.x, child.insertion_point.y);
    Transform {
        insertion_point: Point2D { x: px, y: py },
        x_scale: parent.x_scale * child.x_scale,
        y_scale: parent.y_scale * child.y_scale,
        rotation: parent.rotation + child.rotation,
    }
}

/// `renderArrowhead`: a small filled triangle at `tip`, pointing away from
/// `from` -- used for LEADER's optional arrowhead at its first vertex.
fn render_arrowhead(tip: &Point2D, from: &Point2D, size: f64, color: &str) -> String {
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

/// `renderEntity`: renders one entity (or returns `None` if unsupported,
/// recording its type in `ctx.unsupported`), extending `ctx`'s bounds via
/// `consider()` as it goes. All coordinates in the emitted SVG stay in
/// *local* (untransformed) space -- the enclosing `<g transform>` from
/// `render_block_ref` does the visual repositioning; `consider()` is what
/// separately tracks *world*-space bounds through that same transform.
fn render_entity(e: &RenderEntity, ctx: &mut Ctx) -> Option<String> {
    let color = resolve_entity_color(e.common(), ctx);
    match e {
        RenderEntity::Line(l) => {
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
        RenderEntity::Circle(c) => {
            ctx.consider(c.center.x - c.radius, c.center.y - c.radius);
            ctx.consider(c.center.x + c.radius, c.center.y + c.radius);
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                c.center.x,
                neg(c.center.y),
                c.radius
            ))
        }
        RenderEntity::Arc(a) => {
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
        RenderEntity::Ellipse(el) => {
            let rx = el.major_axis_endpoint.x.hypot(el.major_axis_endpoint.y);
            ctx.consider(el.center.x - rx, el.center.y - rx);
            ctx.consider(el.center.x + rx, el.center.y + rx);
            let ry = rx * el.axis_ratio;
            let rot = el.major_axis_endpoint.y.atan2(el.major_axis_endpoint.x) * 180.0
                / std::f64::consts::PI;
            let (cx, cy) = (el.center.x, neg(el.center.y));
            Some(format!(
                "<ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{rx}\" ry=\"{ry}\" transform=\"rotate({} {cx} {cy})\" fill=\"none\" stroke=\"{color}\"/>",
                neg(rot)
            ))
        }
        RenderEntity::LwPolyline(p) | RenderEntity::Polyline2D(p) => {
            for v in &p.vertices {
                ctx.consider(v.x, v.y);
            }
            let pts = points_attr(&p.vertices);
            let tag = if p.closed { "polygon" } else { "polyline" };
            Some(format!(
                "<{tag} points=\"{pts}\" fill=\"none\" stroke=\"{color}\"/>"
            ))
        }
        RenderEntity::Text(t) => {
            ctx.consider(t.start_point.x, t.start_point.y);
            let (x, y) = (t.start_point.x, neg(t.start_point.y));
            Some(format!(
                "<text x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{color}\" stroke=\"none\"{}>{}</text>",
                t.text_height,
                rotate_transform_attr(t.rotation, x, y),
                escape_xml(&t.text)
            ))
        }
        RenderEntity::Point(p) => {
            ctx.consider(p.position.x, p.position.y);
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"{color}\" stroke=\"none\"/>",
                p.position.x,
                neg(p.position.y)
            ))
        }
        RenderEntity::Solid(s) => {
            // Classic AutoCAD SOLID vertex order is 1-2-4-3, not 1-2-3-4.
            let pts = [s.corner1, s.corner2, s.corner4, s.corner3];
            for p in &pts {
                ctx.consider(p.x, p.y);
            }
            Some(format!(
                "<polygon points=\"{}\" fill=\"{color}\" fill-opacity=\"0.6\" stroke=\"none\"/>",
                points_attr(&pts)
            ))
        }
        RenderEntity::Ray(r) | RenderEntity::XLine(r) => {
            let is_xline = matches!(e, RenderEntity::XLine(_));
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
        RenderEntity::Attrib(a) => {
            ctx.consider(a.start_point.x, a.start_point.y);
            if a.text.is_empty() {
                return Some(String::new());
            }
            let (x, y) = (a.start_point.x, neg(a.start_point.y));
            Some(format!(
                "<text x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{color}\" stroke=\"none\"{}>{}</text>",
                a.text_height,
                rotate_transform_attr(a.rotation, x, y),
                escape_xml(&a.text)
            ))
        }
        RenderEntity::Insert(i) => Some(render_block_ref(
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
        RenderEntity::Dimension(d) => {
            // AutoCAD caches a dimension's rendered geometry (lines, arrows,
            // text) as an anonymous block already in final/world
            // coordinates -- render it directly, no transform (identity).
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
        RenderEntity::Viewport(v) => {
            // Not real drawing geometry (it's a window onto model space),
            // but drawing its frame is a reasonable, honest representation.
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
            for c in &corners {
                ctx.consider(c.x, c.y);
            }
            Some(format!(
                "<polygon points=\"{}\" fill=\"none\" stroke-dasharray=\"2,2\" stroke=\"{color}\"/>",
                points_attr(&corners)
            ))
        }
        RenderEntity::Face3D(f) => {
            // Unlike SOLID, 3DFACE's 4 corners are already sequential --
            // edge-visibility flag bits are ignored, all 4 edges always draw.
            let pts = [f.corner1, f.corner2, f.corner3, f.corner4];
            for p in &pts {
                ctx.consider(p.x, p.y);
            }
            let pts2d: Vec<Point2D> = pts.iter().map(|p| Point2D { x: p.x, y: p.y }).collect();
            Some(format!(
                "<polygon points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                points_attr(&pts2d)
            ))
        }
        RenderEntity::Spline(s) => {
            // Straight-line approximation through fit points (preferred --
            // they lie exactly on the curve) or control points -- not a
            // real NURBS evaluation.
            let pts = if !s.fit_points.is_empty() {
                &s.fit_points
            } else {
                &s.control_points
            };
            if pts.len() < 2 {
                return None;
            }
            for p in pts {
                ctx.consider(p.x, p.y);
            }
            let pts2d: Vec<Point2D> = pts.iter().map(|p| Point2D { x: p.x, y: p.y }).collect();
            Some(format!(
                "<polyline points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                points_attr(&pts2d)
            ))
        }
        RenderEntity::MText(m) => {
            ctx.consider(m.insertion_point.x, m.insertion_point.y);
            let stripped = strip_mtext_formatting(&m.text);
            let lines: Vec<&str> = stripped.lines().filter(|l| !l.is_empty()).collect();
            if lines.is_empty() {
                return Some(String::new());
            }
            let rot_deg = m.rotation * 180.0 / std::f64::consts::PI;
            // JS: `(e.textHeight || 1) * (e.lineSpacing || 1)` -- 0 is
            // treated the same as "unset" at *render* time (not parse
            // time: the raw parsed value is legitimately 0 in some real
            // files, e.g. line_spacing_factor on example_r14.dwg's MTEXT).
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
                neg(rot_deg)
            ))
        }
        RenderEntity::Polyline3D(p) => {
            if p.vertices.is_empty() {
                return None;
            }
            for v in &p.vertices {
                ctx.consider(v.x, v.y);
            }
            let pts2d: Vec<Point2D> = p
                .vertices
                .iter()
                .map(|v| Point2D { x: v.x, y: v.y })
                .collect();
            let pts = points_attr(&pts2d);
            let tag = if p.closed { "polygon" } else { "polyline" };
            Some(format!(
                "<{tag} points=\"{pts}\" fill=\"none\" stroke=\"{color}\"/>"
            ))
        }
        RenderEntity::Solid3D(s) => {
            if s.wireframe_edges.is_empty() {
                ctx.unsupported.insert("3DSOLID".to_string());
                return None;
            }
            Some(render_wireframe(&s.wireframe_edges, &color, ctx))
        }
        // REGION: new functionality, not a JS-baseline port (the JS
        // baseline never handled REGION at all) -- see
        // MultiLeaderEntity's doc comment for this project's precedent on
        // documenting that distinction. Shares 3DSOLID's exact wireframe
        // rendering (same isometric projection, same "no edges ==
        // unsupported" treatment) since REGION's ACIS extraction is the
        // same mechanism (see acis.rs / RenderEntity::Region's doc comment).
        RenderEntity::Region(r) => {
            if r.wireframe_edges.is_empty() {
                ctx.unsupported.insert("REGION".to_string());
                return None;
            }
            Some(render_wireframe(&r.wireframe_edges, &color, ctx))
        }
        // POLYLINE_PFACE: new functionality, not a JS-baseline port -- see
        // RenderEntity::PolylinePFace's doc comment. Shares the same
        // isometric wireframe renderer as Solid3D/Region since it's an
        // inherently 3D mesh, not a flat 2D shape.
        RenderEntity::PolylinePFace(p) => {
            if p.wireframe_edges.is_empty() {
                ctx.unsupported.insert("POLYLINE_PFACE".to_string());
                return None;
            }
            Some(render_wireframe(&p.wireframe_edges, &color, ctx))
        }
        RenderEntity::Hatch(h) => {
            let mut subpaths = Vec::new();
            for path in &h.boundary_paths {
                let pts: Vec<Point2D> = match path {
                    HatchBoundaryPath::Polyline(vertices) => vertices.clone(),
                    HatchBoundaryPath::Edges(edges) => {
                        edges.iter().flat_map(hatch_edge_points).collect()
                    }
                };
                if pts.len() < 2 {
                    continue;
                }
                for p in &pts {
                    ctx.consider(p.x, p.y);
                }
                let mut d = String::from("M ");
                for (i, p) in pts.iter().enumerate() {
                    if i > 0 {
                        d.push_str(" L ");
                    }
                    let _ = write!(d, "{} {}", clean(p.x), neg(p.y));
                }
                d.push_str(" Z");
                subpaths.push(d);
            }
            if subpaths.is_empty() {
                return None;
            }
            let d = subpaths.join(" ");
            let outline =
                format!("<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" fill-rule=\"evenodd\"/>");
            // Gradient takes priority over solid_fill -- AutoCAD sets
            // is_solid_fill on gradient-fill HATCHes too (a gradient is a
            // "solid style" fill with a varying color), so checking
            // solid_fill first would always shadow this branch.
            if let Some(gradient) = &h.gradient {
                return Some(render_hatch_gradient(gradient, &d, ctx));
            }
            if h.solid_fill {
                return Some(format!(
                    "<path d=\"{d}\" fill=\"{color}\" fill-opacity=\"0.2\" stroke=\"{color}\" fill-rule=\"evenodd\"/>"
                ));
            }
            // Actual pattern-fill lines, tiled via an SVG <pattern> (defined
            // in ctx.defs, emitted once into a top-level <defs> by to_svg)
            // and clipped to the boundary by SVG's own fill mechanism --
            // avoids hand-rolling polygon/line clipping. See
            // render_hatch_pattern_line's doc comment for the simplifications
            // this carries (no brick-style stagger, approximate dash
            // alternation).
            let scale = ctx.scale;
            let pattern_fills: Vec<String> = h
                .pattern_lines
                .iter()
                .filter_map(|pl| render_hatch_pattern_line(pl, &color, scale, &d, ctx))
                .collect();
            if pattern_fills.is_empty() {
                // No usable pattern data (gradient fill, unreadable
                // deflines, or every defline degenerate) -- outline-only
                // fallback, same as before this feature existed.
                return Some(outline);
            }
            Some(format!("{}\n  {outline}", pattern_fills.join("\n  ")))
        }
        RenderEntity::Leader(l) => {
            if l.vertices.is_empty() {
                return None;
            }
            for v in &l.vertices {
                ctx.consider(v.x, v.y);
            }
            let pts2d: Vec<Point2D> = l
                .vertices
                .iter()
                .map(|v| Point2D { x: v.x, y: v.y })
                .collect();
            let line = format!(
                "<polyline points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                points_attr(&pts2d)
            );
            let arrow = if l.is_arrowhead_enabled && pts2d.len() >= 2 {
                render_arrowhead(&pts2d[0], &pts2d[1], 2.5, &color)
            } else {
                String::new()
            };
            Some(line + &arrow)
        }
        // MLINE: new functionality, not a JS-baseline port -- see
        // MLineEntity's doc comment. Draws one polyline per line in the
        // referenced MLINESTYLE (Tables::mlinestyles, looked up by name),
        // each vertex offset from the centerline via
        // `point + miter_direction * offset`; falls back to a single
        // centerline (the old behavior, same render shape as
        // LwPolyline/Polyline3D) when the style can't be resolved.
        RenderEntity::MLine(l) => {
            if l.vertices.is_empty() {
                return None;
            }
            for v in &l.vertices {
                ctx.consider(v.point.x, v.point.y);
            }
            let tag = if l.closed { "polygon" } else { "polyline" };
            // A single 0.0 "offset" is exactly the old centerline-only
            // fallback -- point + miter_direction * 0.0 is just point.
            let offsets = match ctx.tables.mlinestyles.get(&l.mlinestyle_name) {
                Some(offsets) if !offsets.is_empty() => offsets.clone(),
                _ => vec![0.0],
            };
            let lines: Vec<String> = offsets
                .iter()
                .map(|&offset| {
                    let pts2d = mline_offset_points(&l.vertices, offset);
                    format!(
                        "<{tag} points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                        points_attr(&pts2d)
                    )
                })
                .collect();
            Some(lines.join("\n  "))
        }
        // MULTILEADER: new functionality, not a JS-baseline port -- see
        // MultiLeaderEntity's doc comment. Leader lines only, each with an
        // arrowhead at its tip (unconditionally, unlike LEADER's
        // is_arrowhead_enabled check -- MULTILEADER's real per-line
        // arrow-visibility flag lives in LEADER_Line.flags, which isn't
        // extracted by the geometry-only shim this reads from; always
        // drawing one is the simpler, still-reasonable default for a
        // best-effort rendering).
        RenderEntity::MultiLeader(m) => {
            if m.lines.is_empty() {
                return None;
            }
            let mut parts = Vec::new();
            for line in &m.lines {
                if line.len() < 2 {
                    continue;
                }
                for v in line {
                    ctx.consider(v.x, v.y);
                }
                let pts2d: Vec<Point2D> = line.iter().map(|v| Point2D { x: v.x, y: v.y }).collect();
                parts.push(format!(
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                    points_attr(&pts2d)
                ));
                let n = pts2d.len();
                parts.push(render_arrowhead(&pts2d[n - 1], &pts2d[n - 2], 2.5, &color));
            }
            if parts.is_empty() {
                return None;
            }
            Some(parts.join("\n  "))
        }
        // ATTDEF is a block-attribute *template*, not drawn -- the JS
        // baseline's toSVG() has no switch case for it either (only special-
        // cased as "skip when iterating a block's children", handled in
        // render_block_ref above); reaching here means it wasn't inside a
        // block reference, matching JS's own "falls through to default,
        // reported unsupported" behavior for that case.
        RenderEntity::Attdef(_) => {
            ctx.unsupported.insert("ATTDEF".to_string());
            None
        }
        // TOLERANCE: new functionality, not a JS-baseline port -- see
        // ToleranceEntity's doc comment. Same rendering shape as ATTRIB.
        RenderEntity::Tolerance(t) => {
            ctx.consider(t.insertion_point.x, t.insertion_point.y);
            if t.text_value.is_empty() {
                return Some(String::new());
            }
            Some(format!(
                "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{color}\" stroke=\"none\">{}</text>",
                t.insertion_point.x,
                neg(t.insertion_point.y),
                t.text_height,
                escape_xml(&t.text_value)
            ))
        }
        // ACAD_TABLE: new functionality, not a JS-baseline port -- see
        // AcadTableEntity's doc comment. Same rendering shape as INSERT.
        RenderEntity::AcadTable(a) => Some(render_block_ref(
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
        // WIPEOUT: new functionality, not a JS-baseline port -- see
        // WipeoutEntity's doc comment for the extra risk here (an
        // unverified pixel-to-world transform). Outline only, not filled
        // -- a filled shape would visually mask whatever's drawn under it
        // (arguably WIPEOUT's actual real-world effect, but not one this
        // renderer's simple in-order painting can be trusted to reproduce
        // correctly, and an unexpectedly opaque box is a worse failure
        // mode than a merely-incomplete outline).
        RenderEntity::Wipeout(w) => {
            if w.boundary.len() < 2 {
                ctx.unsupported.insert("WIPEOUT".to_string());
                return None;
            }
            for p in &w.boundary {
                ctx.consider(p.x, p.y);
            }
            Some(format!(
                "<polygon points=\"{}\" fill=\"none\" stroke-dasharray=\"2,2\" stroke=\"{color}\"/>",
                points_attr(&w.boundary)
            ))
        }
        // LIGHT: new functionality, not a JS-baseline port -- see
        // LightEntity's doc comment.
        RenderEntity::Light(l) => {
            ctx.consider(l.position.x, l.position.y);
            let marker = format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"none\" stroke=\"{color}\"/>",
                l.position.x,
                neg(l.position.y)
            );
            if !l.target_is_meaningful {
                return Some(marker);
            }
            ctx.consider(l.target.x, l.target.y);
            let line = format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-dasharray=\"1,1\" stroke=\"{color}\"/>",
                l.position.x, neg(l.position.y), l.target.x, neg(l.target.y)
            );
            Some(format!("{marker}\n  {line}"))
        }
        RenderEntity::Unknown { type_name, .. } => {
            ctx.unsupported.insert(type_name.clone());
            None
        }
    }
}

/// `projectIsometric`: fixed engineering-isometric projection (30 degrees)
/// for the experimental 3DSOLID/REGION wireframe renderer -- see `acis.rs`.
/// Not configurable; this is a best-effort "something is better than
/// nothing" view, not a real camera/viewport.
fn project_isometric(p: &Point3D) -> (f64, f64) {
    let cos30 = (std::f64::consts::PI / 6.0).cos();
    let sin30 = (std::f64::consts::PI / 6.0).sin();
    ((p.x - p.z) * cos30, p.y + (p.x + p.z) * sin30)
}

/// Shared by `RenderEntity::Solid3D`/`RenderEntity::Region` (identical ACIS
/// wireframe-extraction mechanism, see `acis.rs`): one `<line>` per edge,
/// isometrically projected.
fn render_wireframe(edges: &[[Point3D; 2]], color: &str, ctx: &mut Ctx) -> String {
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

/// `arcSweep`: signed sweep angle from `start_angle` to `end_angle`,
/// normalized into the half-open range implied by `is_ccw` (matching
/// AutoCAD's arc-direction convention, not just the shorter/positive arc).
fn arc_sweep(start_angle: f64, end_angle: f64, is_ccw: bool) -> f64 {
    let mut sweep = end_angle - start_angle;
    if is_ccw {
        if sweep <= 0.0 {
            sweep += 2.0 * std::f64::consts::PI;
        }
    } else if sweep >= 0.0 {
        sweep -= 2.0 * std::f64::consts::PI;
    }
    sweep
}

/// Renders one [`HatchPatternLine`] as a tiled SVG `<pattern>` fill of the
/// HATCH's boundary path (`path_d`, already computed by the caller) --
/// pushes the `<pattern>` definition into `ctx.defs` (emitted once into a
/// top-level `<defs>` by `to_svg`) and returns a `<path fill="url(#...)">`
/// element referencing it, or `None` if this line family is degenerate
/// (non-finite/zero perpendicular spacing -- can't tile).
///
/// `pl`'s `angle`/`base_point`/`offset`/`dash_pattern` are used directly, as
/// already-final values in the HATCH's own local coordinate space --
/// [`crate::render_model::HatchEntity`]'s own `pattern_angle`/`pattern_scale`
/// fields (DXF 52/41) are deliberately *not* reapplied here. This was
/// tried first (standard DXF-pattern-fill documentation describes 52/41 as
/// transforming the definition-line data) and produced visibly wrong
/// output on a real file: a HATCH with `pattern_angle` of exactly 90
/// degrees had a single defline whose own `angle` was *also* exactly 90
/// degrees (not 0) -- only explainable if LibreDWG's defline data already
/// has 52/41 baked in. Confirmed further by tile size: multiplying by
/// `pattern_scale` (60, for one real HATCH) inflated a defline's ~6.5-unit
/// perpendicular spacing to ~390 units, many times larger than the ~90-unit
/// boundary shape it was supposed to fill -- rendering as no visible lines
/// at all (verified by screenshot). Removing the multiplication fixed it.
///
/// Two further deliberate simplifications, in the same "best-effort, not
/// full fidelity" spirit as this file's chord-approximated curves and
/// MLINE's centerline-only rendering:
/// - Only `offset`'s component perpendicular to the line direction (the
///   spacing between adjacent lines) is honored. A real DXF pattern's
///   `offset` can also have a component *parallel* to the line direction,
///   used for brick/masonry-style staggering between rows -- dropped here
///   rather than attempting a sheared/staggered tile.
/// - The rendered line sits at the vertical midpoint of its tile (not at
///   `base_point` itself, which lands exactly on a tile edge) to avoid
///   `<pattern>`'s default tile-edge clipping cutting a boundary-straddling
///   stroke in half. `base_point`'s exact phase is therefore only
///   approximately reproduced -- immaterial for an infinitely-repeating
///   pattern's visual appearance, but not a literal reproduction of the DXF
///   data.
///
/// `stroke_scale` is `ctx.scale` at render time (the same value
/// `render_block_ref` divides into its own placeholder stroke-width) --
/// needed because `<pattern>` content only inherits presentation properties
/// from its *own* ancestor chain (`<defs>` under the root `<svg>`), never
/// from wherever `fill="url(#...)"` is actually used, so a HATCH nested
/// inside a scaled block reference would otherwise render its pattern lines
/// at the wrong visual thickness.
fn render_hatch_pattern_line(
    pl: &HatchPatternLine,
    color: &str,
    stroke_scale: f64,
    path_d: &str,
    ctx: &mut Ctx,
) -> Option<String> {
    let line_angle = pl.angle;
    let base = pl.base_point;
    let offset = pl.offset;

    let (dir_x, dir_y) = (line_angle.cos(), line_angle.sin());
    let (perp_x, perp_y) = (-dir_y, dir_x);
    let spacing = (offset.x * perp_x + offset.y * perp_y).abs();
    if !spacing.is_finite() || spacing < 1e-6 {
        return None;
    }

    let dashes: Vec<f64> = pl.dash_pattern.iter().map(|d| d.abs()).collect();
    let cycle: f64 = dashes.iter().sum();
    let width = if cycle.is_finite() && cycle > 1e-6 {
        cycle
    } else {
        spacing
    };
    let dasharray = if dashes.is_empty() {
        String::new()
    } else {
        let joined = dashes
            .iter()
            .map(|d| clean(*d).to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(" stroke-dasharray=\"{joined}\"")
    };

    let id = format!("hp{}", ctx.next_pattern_id);
    ctx.next_pattern_id += 1;

    let (tx, ty) = (clean(base.x), neg(base.y));
    let deg = neg(line_angle * 180.0 / std::f64::consts::PI);
    let (w, h, half) = (clean(width), clean(spacing), clean(spacing / 2.0));
    ctx.defs.push(format!(
        "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{w}\" height=\"{h}\" \
         patternTransform=\"translate({tx} {ty}) rotate({deg})\">\n    \
         <line x1=\"0\" y1=\"{half}\" x2=\"{w}\" y2=\"{half}\" stroke=\"{color}\" \
         stroke-width=\"@@SW@@{stroke_scale}@@\"{dasharray}/>\n  </pattern>"
    ));

    Some(format!(
        "<path d=\"{path_d}\" fill=\"url(#{id})\" stroke=\"none\" fill-rule=\"evenodd\"/>"
    ))
}

/// Renders a [`HatchGradient`] as an SVG `linearGradient`/`radialGradient`
/// (per `is_radial`) filling the HATCH boundary path (`path_d`, already
/// computed by the caller) -- pushed into `ctx.defs` like
/// `render_hatch_pattern_line`'s `<pattern>` defs, sharing the same
/// `next_pattern_id` counter for a globally-unique id (prefixed `hg`
/// instead of `hp` to stay visually distinguishable in the emitted SVG).
///
/// Both gradient kinds use `objectBoundingBox` units (`0..1` relative to the
/// filled path's own bounding box) rather than absolute coordinates, unlike
/// `render_hatch_pattern_line`'s `userSpaceOnUse` `<pattern>` -- there's no
/// tiling here, just two color stops spanning the shape, so bbox-relative
/// coordinates need no `ctx.scale`/translate bookkeeping at all. The linear
/// case starts as a left-to-right gradient and rotates `angle` around the
/// bbox center (`gradientTransform="rotate(deg 0.5 0.5)"`); `deg` is negated
/// the same way `render_hatch_pattern_line`'s pattern rotation is, to
/// compensate for this renderer's DXF-Y-up-to-SVG-Y-down flip. The radial
/// case ignores `angle` entirely -- a center-out radial gradient has no
/// meaningful rotation.
fn render_hatch_gradient(g: &HatchGradient, path_d: &str, ctx: &mut Ctx) -> String {
    let id = format!("hg{}", ctx.next_pattern_id);
    ctx.next_pattern_id += 1;
    let def = if g.is_radial {
        format!(
            "<radialGradient id=\"{id}\" gradientUnits=\"objectBoundingBox\" cx=\"0.5\" cy=\"0.5\" r=\"0.5\">\n    \
             <stop offset=\"0%\" stop-color=\"{}\"/>\n    <stop offset=\"100%\" stop-color=\"{}\"/>\n  </radialGradient>",
            g.color1, g.color2
        )
    } else {
        let deg = neg(g.angle * 180.0 / std::f64::consts::PI);
        format!(
            "<linearGradient id=\"{id}\" gradientUnits=\"objectBoundingBox\" x1=\"0\" y1=\"0.5\" x2=\"1\" y2=\"0.5\" \
             gradientTransform=\"rotate({deg} 0.5 0.5)\">\n    \
             <stop offset=\"0%\" stop-color=\"{}\"/>\n    <stop offset=\"100%\" stop-color=\"{}\"/>\n  </linearGradient>",
            g.color1, g.color2
        )
    };
    ctx.defs.push(def);
    format!("<path d=\"{path_d}\" fill=\"url(#{id})\" stroke=\"none\" fill-rule=\"evenodd\"/>")
}

/// Projects an MLINE's centerline vertices onto the parallel line `offset`
/// away from it -- `point + miter_direction * offset` per vertex, per
/// [`MLineEntity`]'s doc comment. `offset == 0.0` reproduces the centerline
/// itself, which is why the caller uses this same function for both the
/// real-offset and no-resolvable-MLINESTYLE-fallback cases.
fn mline_offset_points(vertices: &[MLineVertex], offset: f64) -> Vec<Point2D> {
    vertices
        .iter()
        .map(|v| Point2D {
            x: v.point.x + v.miter_direction.x * offset,
            y: v.point.y + v.miter_direction.y * offset,
        })
        .collect()
}

/// `hatchEdgePoints`: chord-approximates one HATCH boundary edge, in the
/// same spirit used elsewhere in this file (SPLINE, curved 3DSOLID edges).
/// The final point of each edge is omitted since it should coincide with
/// the next edge's first point (or, for a single closed edge like a full
/// ellipse, with its own first point).
fn hatch_edge_points(edge: &HatchEdge) -> Vec<Point2D> {
    match edge {
        HatchEdge::Line { start } => vec![*start],
        HatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            is_ccw,
        } => {
            let sweep = arc_sweep(*start_angle, *end_angle, *is_ccw);
            let segments = 12;
            (0..segments)
                .map(|i| {
                    let a = start_angle + sweep * (i as f64 / segments as f64);
                    Point2D {
                        x: center.x + radius * a.cos(),
                        y: center.y + radius * a.sin(),
                    }
                })
                .collect()
        }
        HatchEdge::Ellipse {
            center,
            end,
            minor_major_ratio,
            start_angle,
            end_angle,
            is_ccw,
        } => {
            let major_len = end.x.hypot(end.y);
            let minor_len = major_len * minor_major_ratio;
            let rot = end.y.atan2(end.x);
            let (cos_r, sin_r) = (rot.cos(), rot.sin());
            let sweep = arc_sweep(*start_angle, *end_angle, *is_ccw);
            let segments = 16;
            (0..segments)
                .map(|i| {
                    let a = start_angle + sweep * (i as f64 / segments as f64);
                    let (ex, ey) = (major_len * a.cos(), minor_len * a.sin());
                    Point2D {
                        x: center.x + ex * cos_r - ey * sin_r,
                        y: center.y + ex * sin_r + ey * cos_r,
                    }
                })
                .collect()
        }
        HatchEdge::Spline { control_points } => {
            if control_points.is_empty() {
                Vec::new()
            } else {
                control_points[..control_points.len() - 1].to_vec()
            }
        }
    }
}

fn points_attr(pts: &[Point2D]) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{},{}", clean(p.x), neg(p.y));
    }
    s
}

/// `selectEntitiesForSpace`: `db.entities` already only ever contains
/// model+paper-space entities (see convert.rs's module doc), so `'all'`
/// here just returns everything; `'model'`/`'paper'` narrow further by
/// matching block name (`*Model_Space` exact, `*Paper_Space*` prefix,
/// case-insensitive -- multiple paper space layout tabs are all included).
fn select_entities_for_space(db: &CadDatabase, space: Space) -> Vec<&RenderEntity> {
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
            if let RenderEntity::Insert(insert) = e {
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

// --- outlier trim: clusterEntityBoxes / dominantClusterBox ---

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 * p).floor() as usize).min(sorted.len() - 1);
    sorted[idx]
}

fn cluster_entity_boxes(boxes: &[Box2D]) -> Vec<Vec<Box2D>> {
    let n = boxes.len();
    if n == 0 {
        return Vec::new();
    }

    let mut all_x: Vec<f64> = boxes.iter().flat_map(|b| [b.min_x, b.max_x]).collect();
    let mut all_y: Vec<f64> = boxes.iter().flat_map(|b| [b.min_y, b.max_y]).collect();
    all_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    all_y.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let core_diag = (percentile(&all_x, 0.75) - percentile(&all_x, 0.25))
        .hypot(percentile(&all_y, 0.75) - percentile(&all_y, 0.25));
    let raw_diag = (all_x[all_x.len() - 1] - all_x[0]).hypot(all_y[all_y.len() - 1] - all_y[0]);
    let eps = if core_diag > 0.0 {
        (core_diag * 0.005).max(1e-9)
    } else {
        (raw_diag * 0.001).max(1e-9)
    };

    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    }

    let corners: Vec<[(f64, f64); 4]> = boxes
        .iter()
        .map(|b| {
            [
                (b.min_x, b.min_y),
                (b.min_x, b.max_y),
                (b.max_x, b.min_y),
                (b.max_x, b.max_y),
            ]
        })
        .collect();

    let cell = eps;
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    let key = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
    for (i, c) in corners.iter().enumerate() {
        for &(x, y) in c {
            grid.entry(key(x, y)).or_default().push(i);
        }
    }

    for (i, c) in corners.iter().enumerate() {
        for &(x, y) in c {
            let (cx, cy) = key(x, y);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(bucket) = grid.get(&(cx + dx, cy + dy)) else {
                        continue;
                    };
                    for &j in bucket {
                        if j == i || find(&mut parent, i) == find(&mut parent, j) {
                            continue;
                        }
                        if corners[j]
                            .iter()
                            .any(|&(jx, jy)| (x - jx).hypot(y - jy) <= eps)
                        {
                            union(&mut parent, i, j);
                        }
                    }
                }
            }
        }
    }

    let mut groups: std::collections::HashMap<usize, Vec<Box2D>> = std::collections::HashMap::new();
    for (i, &b) in boxes.iter().enumerate() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(b);
    }
    groups.into_values().collect()
}

fn bbox_of(group: &[Box2D]) -> Box2D {
    let mut b = Box2D {
        min_x: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        min_y: f64::INFINITY,
        max_y: f64::NEG_INFINITY,
    };
    for g in group {
        b.min_x = b.min_x.min(g.min_x);
        b.max_x = b.max_x.max(g.max_x);
        b.min_y = b.min_y.min(g.min_y);
        b.max_y = b.max_y.max(g.max_y);
    }
    b
}

fn rect_gap(a: &Box2D, b: &Box2D) -> f64 {
    let dx = (a.min_x - b.max_x).max(b.min_x - a.max_x).max(0.0);
    let dy = (a.min_y - b.max_y).max(b.min_y - a.max_y).max(0.0);
    dx.hypot(dy)
}

fn diag(b: &Box2D) -> f64 {
    (b.max_x - b.min_x).hypot(b.max_y - b.min_y)
}

fn dominant_cluster_box(boxes: &[Box2D]) -> Option<Box2D> {
    let mut clusters = cluster_entity_boxes(boxes);
    if clusters.len() <= 1 {
        return None;
    }

    let score = |group: &[Box2D]| group.len() as f64 * diag(&bbox_of(group));
    clusters.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap());

    let mut seed = clusters[0].clone();
    let mut seed_box = bbox_of(&seed);
    let mut rest: Vec<Vec<Box2D>> = clusters[1..].to_vec();

    let mut diags: Vec<f64> = clusters.iter().map(|c| diag(&bbox_of(c))).collect();
    diags.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let typical_cluster_scale = diags.get(clusters.len() / 2).copied().unwrap_or(0.0);

    loop {
        let threshold = diag(&seed_box) * 0.3;
        let mut still_separate = Vec::new();
        let mut absorbed_any = false;
        for cluster in rest {
            let c_box = bbox_of(&cluster);
            let c_diag = diag(&c_box);
            let is_degenerate_huge = cluster.len() < 5
                && typical_cluster_scale > 0.0
                && c_diag > typical_cluster_scale * 20.0;
            if !is_degenerate_huge && rect_gap(&seed_box, &c_box) <= threshold {
                seed.extend(cluster);
                absorbed_any = true;
            } else {
                still_separate.push(cluster);
            }
        }
        rest = still_separate;
        if absorbed_any {
            seed_box = bbox_of(&seed);
        }
        if !absorbed_any || rest.is_empty() {
            break;
        }
    }

    if !rest.is_empty() {
        let total = boxes.len();
        if (seed.len() as f64) / (total as f64) < 0.5 {
            return None;
        }
        return Some(bbox_of(&seed));
    }
    None
}

/// Renders a parsed [`CadDatabase`] to an SVG string.
///
/// `outlier_trim` (default `true`) computes the viewBox from the dominant
/// spatially-connected cluster of entities instead of the raw min/max --
/// see `dominant_cluster_box`'s doc for why (a straight per-axis gap test
/// is fundamentally wrong: a rectangular room's opposite walls only touch
/// at corners, so an axis-gap approach misreads the empty interior as a
/// disconnected outlier and trims away real geometry).
///
/// `stroke_width` defaults to ~1/6000th of the computed viewBox diagonal
/// rather than a fixed value, and nested INSERT block references get a
/// `@@SW@@<scale>@@` placeholder (resolved in one pass at the very end)
/// so their visual stroke width stays constant regardless of how deeply
/// nested or scaled the block reference is -- the true value isn't known
/// until the whole render pass (and therefore the final viewBox) completes.
pub(crate) fn to_svg(db: &CadDatabase, options: ToSvgOptions) -> ToSvgResult {
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut entity_boxes: Vec<Box2D> = Vec::new();
    let mut body: Vec<String> = Vec::new();
    let mut unsupported: HashSet<String> = HashSet::new();

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
    xs.append(&mut ctx.xs);
    ys.append(&mut ctx.ys);
    unsupported.extend(ctx.unsupported.drain());

    let raw_bounds = |xs: &[f64], ys: &[f64]| Box2D {
        min_x: xs.iter().cloned().fold(f64::INFINITY, f64::min),
        max_x: xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        min_y: ys.iter().cloned().fold(f64::INFINITY, f64::min),
        max_y: ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    };

    let bounds = if xs.is_empty() {
        Box2D {
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
        }
    } else if options.outlier_trim && entity_boxes.len() > 2 {
        dominant_cluster_box(&entity_boxes).unwrap_or_else(|| raw_bounds(&xs, &ys))
    } else {
        raw_bounds(&xs, &ys)
    };

    let x = bounds.min_x - options.padding;
    let y = -bounds.max_y - options.padding;
    let width = (bounds.max_x - bounds.min_x) + options.padding * 2.0;
    let height = (bounds.max_y - bounds.min_y) + options.padding * 2.0;
    let effective_stroke_width = options
        .stroke_width
        .unwrap_or_else(|| (width.hypot(height) / 6000.0).max(0.01));

    let joined_body = body.join("\n  ");
    let resolved_body = resolve_stroke_width_placeholders(&joined_body, effective_stroke_width);
    // HATCH pattern-fill <pattern> defs (see render_hatch_pattern_line) also
    // carry @@SW@@ placeholders -- same resolution pass, kept separate from
    // resolved_body only so an empty defs list doesn't emit an empty
    // <defs></defs> block.
    let defs_block = if ctx.defs.is_empty() {
        String::new()
    } else {
        let joined_defs = ctx.defs.join("\n  ");
        let resolved_defs = resolve_stroke_width_placeholders(&joined_defs, effective_stroke_width);
        format!("<defs>\n  {resolved_defs}\n</defs>\n  ")
    };

    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x} {y} {} {}\" stroke=\"black\" stroke-width=\"{effective_stroke_width}\">\n  {defs_block}{resolved_body}\n</svg>",
        if width != 0.0 { width } else { 1.0 },
        if height != 0.0 { height } else { 1.0 }
    );

    ToSvgResult {
        svg,
        unsupported_entity_types: unsupported.into_iter().collect(),
    }
}

/// Resolves every `@@SW@@<cumulativeScale>@@` placeholder (see
/// `render_block_ref`) to `effective_stroke_width / cumulativeScale`, in
/// one pass over the fully-assembled body.
fn resolve_stroke_width_placeholders(body: &str, effective_stroke_width: f64) -> String {
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
        let scale_str = &after[..end];
        let scale: f64 = scale_str.parse().unwrap_or(1.0);
        let _ = write!(out, "{}", effective_stroke_width / scale);
        rest = &after[end + "@@".len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_model::HatchPatternLine;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ~= {b}");
    }

    fn close_box(a: Box2D, b: Box2D) {
        close(a.min_x, b.min_x);
        close(a.max_x, b.max_x);
        close(a.min_y, b.min_y);
        close(a.max_y, b.max_y);
    }

    #[test]
    fn render_block_ref_budget_caps_combinatorial_blowup_from_self_referencing_blocks() {
        use crate::render_model::InsertEntity;
        use crate::tables::BlockRecord;
        use std::collections::HashMap;

        // Block "R" contains 5 INSERTs of itself. `depth`'s own cap (>20)
        // bounds recursion *depth*, but nothing bounds *breadth* on its
        // own -- a full 5-ary tree 20 levels deep is 5^20 (~9.5e13) block
        // instantiations, which would never finish. block_ref_budget is
        // what actually stops this in reasonable time.
        let common = EntityCommon {
            handle: String::new(),
            layer: String::new(),
            color_index: 0,
            true_color: None,
        };
        let children: Vec<RenderEntity> = (0..5)
            .map(|i| {
                RenderEntity::Insert(InsertEntity {
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
        let mut block_records = HashMap::new();
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
        // Only the first, fully-finite point should have been recorded --
        // an entity whose bounds were corrupted by a non-finite coordinate
        // used to leave `ent_min_y == ent_max_y == Infinity`, which made
        // `diag()` compute `Infinity - Infinity == NaN` and panic the
        // `.partial_cmp(..).unwrap()` calls in `cluster_entity_boxes`/
        // `dominant_cluster_box` downstream.
        let b = ctx.entity_box().expect("one finite point was recorded");
        close_box(
            b,
            Box2D {
                min_x: 1.0,
                max_x: 1.0,
                min_y: 2.0,
                max_y: 2.0,
            },
        );
        assert!(diag(&b).is_finite());
    }

    #[test]
    fn clean_snaps_subnormals_to_zero_but_leaves_normal_values_alone() {
        assert_eq!(clean(0.0), 0.0);
        assert_eq!(clean(1.5), 1.5);
        assert_eq!(clean(-1.5), -1.5);
        // A pointer bit-pattern reinterpreted as f64 typically lands here --
        // see dynapi.rs's SplineControlPoint doc comment for why this
        // backstop exists.
        assert_eq!(clean(f64::MIN_POSITIVE / 2.0), 0.0);
    }

    #[test]
    fn neg_flips_sign_and_canonicalizes_zero() {
        assert_eq!(neg(5.0), -5.0);
        assert_eq!(neg(-5.0), 5.0);
        assert!(
            neg(0.0).is_sign_positive(),
            "neg(0.0) must not print as \"-0\""
        );
        assert!(neg(-0.0).is_sign_positive());
    }

    #[test]
    fn strip_mtext_formatting_unwraps_stacked_fractions_and_drops_inline_codes() {
        let stripped = strip_mtext_formatting(r"\A1;Line1\PLine2 {\C1;colored} \S1#8;");
        assert_eq!(stripped, "Line1\nLine2 colored 1/8");
    }

    #[test]
    fn strip_mtext_formatting_collapses_double_backslash_and_expands_nbsp() {
        assert_eq!(strip_mtext_formatting(r"a\\b"), r"a\b");
        assert_eq!(strip_mtext_formatting(r"a\~b"), "a b");
    }

    #[test]
    fn escape_xml_escapes_only_the_three_xml_text_metacharacters() {
        assert_eq!(escape_xml("<a & b>"), "&lt;a &amp; b&gt;");
        assert_eq!(escape_xml("no special chars"), "no special chars");
        // Quotes are deliberately NOT escaped -- text content, not an
        // attribute value.
        assert_eq!(escape_xml("\"quoted\""), "\"quoted\"");
    }

    #[test]
    fn rotate_transform_attr_is_empty_for_zero_rotation() {
        assert_eq!(rotate_transform_attr(0.0, 5.0, 7.0), "");
    }

    #[test]
    fn rotate_transform_attr_negates_degrees_for_svgs_flipped_y_axis() {
        let quarter_turn = std::f64::consts::FRAC_PI_2;
        assert_eq!(
            rotate_transform_attr(quarter_turn, 5.0, 7.0),
            " transform=\"rotate(-90 5 7)\""
        );
    }

    #[test]
    fn arc_sweep_matches_autocads_direction_convention() {
        let pi = std::f64::consts::PI;
        close(arc_sweep(0.0, pi / 2.0, true), pi / 2.0);
        close(arc_sweep(pi / 2.0, 0.0, true), 1.5 * pi);
        close(arc_sweep(0.0, pi / 2.0, false), -1.5 * pi);
        close(arc_sweep(pi / 2.0, 0.0, false), -pi / 2.0);
    }

    #[test]
    fn hatch_edge_points_line_is_a_single_point() {
        let pts = hatch_edge_points(&HatchEdge::Line {
            start: Point2D { x: 1.0, y: 2.0 },
        });
        assert_eq!(pts, vec![Point2D { x: 1.0, y: 2.0 }]);
    }

    #[test]
    fn hatch_edge_points_arc_chord_approximates_on_the_circle() {
        let pi = std::f64::consts::PI;
        let pts = hatch_edge_points(&HatchEdge::Arc {
            center: Point2D { x: 0.0, y: 0.0 },
            radius: 2.0,
            start_angle: 0.0,
            end_angle: pi / 2.0,
            is_ccw: true,
        });
        assert_eq!(pts.len(), 12);
        for p in &pts {
            close(p.x.hypot(p.y), 2.0);
        }
        close(pts[0].x, 2.0);
        close(pts[0].y, 0.0);
    }

    #[test]
    fn hatch_edge_points_spline_drops_the_final_control_point() {
        let cps = vec![
            Point2D { x: 0.0, y: 0.0 },
            Point2D { x: 1.0, y: 1.0 },
            Point2D { x: 2.0, y: 2.0 },
        ];
        let pts = hatch_edge_points(&HatchEdge::Spline {
            control_points: cps.clone(),
        });
        assert_eq!(pts, &cps[..2]);
        assert_eq!(
            hatch_edge_points(&HatchEdge::Spline {
                control_points: vec![]
            }),
            Vec::new()
        );
    }

    #[test]
    fn points_attr_applies_clean_and_y_flip_to_each_point() {
        let pts = vec![Point2D { x: 1.0, y: 2.0 }, Point2D { x: 3.0, y: -4.0 }];
        assert_eq!(points_attr(&pts), "1,-2 3,4");
    }

    #[test]
    fn percentile_indexes_by_fraction_and_clamps_at_the_last_element() {
        let sorted = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 0.25), 2.0);
        assert_eq!(percentile(&sorted, 0.75), 4.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    fn bx(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Box2D {
        Box2D {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    #[test]
    fn bbox_of_and_diag_and_rect_gap() {
        let a = bx(0.0, 0.0, 1.0, 1.0);
        let b = bx(2.0, 0.0, 3.0, 1.0);
        close(diag(&bx(0.0, 0.0, 3.0, 4.0)), 5.0);
        close(rect_gap(&a, &b), 1.0);
        close(rect_gap(&a, &bx(0.5, 0.5, 1.5, 1.5)), 0.0); // overlapping
        let union = bbox_of(&[a, b]);
        close_box(union, bx(0.0, 0.0, 3.0, 1.0));
    }

    #[test]
    fn cluster_entity_boxes_separates_a_distant_outlier() {
        let touching = [
            bx(0.0, 0.0, 1.0, 1.0),
            bx(1.0, 1.0, 2.0, 2.0),
            bx(1.0, 0.0, 2.0, 1.0),
        ];
        let outlier = bx(1000.0, 1000.0, 1001.0, 1001.0);
        let boxes = [touching[0], touching[1], touching[2], outlier];
        let clusters = cluster_entity_boxes(&boxes);
        assert_eq!(clusters.len(), 2);
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = clusters.iter().map(|c| c.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, vec![1, 3]);
    }

    #[test]
    fn dominant_cluster_box_picks_the_larger_group_and_ignores_the_outlier() {
        let boxes = vec![
            bx(0.0, 0.0, 1.0, 1.0),
            bx(1.0, 1.0, 2.0, 2.0),
            bx(1.0, 0.0, 2.0, 1.0),
            bx(1000.0, 1000.0, 1001.0, 1001.0),
        ];
        let dominant = dominant_cluster_box(&boxes).expect("should find a dominant cluster");
        close_box(dominant, bx(0.0, 0.0, 2.0, 2.0));
    }

    #[test]
    fn dominant_cluster_box_returns_none_when_everything_is_one_cluster() {
        let boxes = vec![bx(0.0, 0.0, 1.0, 1.0), bx(1.0, 0.0, 2.0, 1.0)];
        assert!(dominant_cluster_box(&boxes).is_none());
    }

    #[test]
    fn resolve_stroke_width_placeholders_divides_by_the_embedded_scale() {
        let out =
            resolve_stroke_width_placeholders("prefix @@SW@@2@@ middle @@SW@@0.5@@ suffix", 10.0);
        assert_eq!(out, "prefix 5 middle 20 suffix");
    }

    #[test]
    fn resolve_stroke_width_placeholders_emits_malformed_placeholder_verbatim() {
        let out = resolve_stroke_width_placeholders("before @@SW@@2 no closing marker", 10.0);
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
    fn render_hatch_pattern_line_skips_zero_perpendicular_spacing() {
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        // offset is parallel to the line direction (angle 0) -- zero
        // perpendicular component, nothing to tile.
        let pl = HatchPatternLine {
            angle: 0.0,
            base_point: Point2D { x: 0.0, y: 0.0 },
            offset: Point2D { x: 1.0, y: 0.0 },
            dash_pattern: vec![],
        };
        let result = render_hatch_pattern_line(&pl, "#000000", 1.0, "M 0 0 Z", &mut ctx);
        assert!(result.is_none());
        assert!(ctx.defs.is_empty());
    }

    #[test]
    fn render_hatch_pattern_line_emits_one_pattern_def_per_call() {
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        let pl = HatchPatternLine {
            angle: 0.0,
            base_point: Point2D { x: 0.0, y: 0.0 },
            offset: Point2D { x: 0.0, y: 2.0 },
            dash_pattern: vec![],
        };
        let path_d = "M 0 0 L 1 1 Z";
        let first = render_hatch_pattern_line(&pl, "#000000", 1.0, path_d, &mut ctx)
            .expect("valid spacing should produce a fill");
        assert_eq!(ctx.defs.len(), 1);
        assert!(ctx.defs[0].contains("<pattern"));
        assert!(first.contains("fill=\"url(#hp0)\""));

        let second = render_hatch_pattern_line(&pl, "#000000", 1.0, path_d, &mut ctx)
            .expect("second call should also succeed");
        assert_eq!(
            ctx.defs.len(),
            2,
            "each call must get its own unique pattern id"
        );
        assert!(second.contains("fill=\"url(#hp1)\""));
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
