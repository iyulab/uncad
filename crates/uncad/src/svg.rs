//! SVG rendering of a parsed [`CadDatabase`].
//!
//! Entity coverage matches [`crate::model::Entity`]'s variants. The one type
//! left *permanently* unsupported is `ACAD_PROXY_ENTITY`: an opaque per-app
//! serialized blob with no geometry to draw at all. A handful of types that
//! do carry geometry are not converted yet either (MINSERT, TRACE,
//! POLYLINE_MESH, SHAPE, BODY, OLE2FRAME) -- see `docs/CAVEATS.md`. Anything
//! this renderer cannot draw is reported through
//! [`ToSvgResult::unsupported_types`] rather than dropped silently.
//!
//! Several types render as deliberate approximations -- curves as chords, 3D
//! solids as isometric wireframes, VIEWPORT and WIPEOUT as outlines only. See
//! `docs/CAVEATS.md` for the full picture.
//!
//! Layout of this module: options and results, the block transform, the
//! rendering context, per-entity rendering, then [`to_svg`] itself.
//! Submodules hold the parts that stand on their own -- [`format`] (number and
//! string formatting), [`hatch`] (HATCH fills), [`infinite`] (RAY and XLINE,
//! which are clipped to the picture once the viewBox is known) and [`bounds`]
//! (per-entity boxes and proximity clustering). The viewBox is
//! [`crate::crop`]'s decision, taken from the extents this module measures
//! while it draws.

pub(crate) mod bounds;
mod format;
mod hatch;
mod infinite;

use crate::color::{contrast_on_white, resolve_color, DEFAULT_COLOR};
use crate::dynapi::{Point2D, Point3D};
use crate::limits::{
    LimitReport, MAX_BLOCK_REFS, MAX_BLOCK_REF_DEPTH, MAX_ENTITY_POINTS, MAX_ENTITY_SVG_BYTES,
    MAX_SVG_BODY_BYTES, MAX_WORLD_COORDINATE,
};
use crate::model::{Entity, EntityCommon, MLineVertex};
use crate::tables::Tables;
use crate::CadDatabase;
use bounds::Box2D;

use crate::crop::{self, CropMode, CropReport, Extent, Rect};
use format::{clean, escape_xml, neg, rotate_transform_attr, xy, Frame};
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
    /// Padding around the crop, in drawing units. `None` (the default) is
    /// automatic: 2 % of the crop's longer side here, and at least 24 px in
    /// a PNG (see [`crate::crop::auto_padding`]). Since 0.3.0 an `Option`.
    pub padding: Option<f64>,
    /// `None` = auto-scaled to the computed viewBox (see [`to_svg`]).
    pub stroke_width: Option<f64>,
    pub space: Space,
    /// What the viewBox shows -- see [`crate::crop`]. Default
    /// [`CropMode::Auto`]: the visible entities' extents minus scale
    /// outliers. Replaces 0.2.0's `outlier_trim`, whose cluster trim could
    /// drop real geometry; `CropMode::Raw` is the old `outlier_trim: false`.
    pub crop: CropMode,
    /// Draw the entities the drawing hides (layers off, frozen or
    /// non-plotting, DEFPOINTS, invisible entities -- see
    /// [`crate::visibility`]) at 50 % opacity instead of leaving them out.
    /// Default `false`. Since 0.3.0.
    pub include_hidden: bool,
}

impl Default for ToSvgOptions {
    fn default() -> Self {
        ToSvgOptions {
            padding: None,
            stroke_width: None,
            space: Space::Model,
            crop: CropMode::Auto,
            include_hidden: false,
        }
    }
}

pub struct ToSvgResult {
    pub svg: String,
    /// The `viewBox` the document was given: what the image shows, padding
    /// included, and the key to mapping its pixels back to the drawing. In
    /// world units; the document's own `viewBox` attribute is this minus
    /// [`origin`](Self::origin).
    pub view_box: ViewBox,
    /// The world point the SVG's coordinates are relative to: SVG user
    /// units = world minus origin (before the y flip). `[0, 0]` for a
    /// drawing near the origin, so its SVG reads in world units; a drawing
    /// whose coordinates are large (over 32768 units) is shifted by the
    /// rounded median of its entities' reference points, because the
    /// rasterizer keeps path points in `f32`. Since 0.3.0.
    pub origin: [f64; 2],
    /// DXF names of entity types this renderer had nothing to draw for,
    /// sorted.
    pub unsupported_types: Vec<String>,
    /// How many entities were hidden (drawn faded with
    /// [`ToSvgOptions::include_hidden`], left out otherwise), block contents
    /// included. Since 0.3.0.
    pub hidden: usize,
    /// How the viewBox was chosen and what it leaves out. Since 0.3.0.
    pub crop: CropReport,
    /// What the robustness caps in [`crate::limits`] took away -- empty for
    /// every well-formed drawing. Since 0.3.0.
    pub limits: LimitReport,
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
    /// The viewBox showing the world rectangle `rect`. A degenerate
    /// (zero-size) rectangle still gets a 1 x 1 canvas.
    pub fn from_world(rect: &Rect) -> ViewBox {
        let (width, height) = (rect.width(), rect.height());
        ViewBox {
            x: rect.min_x,
            y: -rect.max_y,
            width: if width > 0.0 { width } else { 1.0 },
            height: if height > 0.0 { height } else { 1.0 },
        }
    }

    /// This viewBox expressed relative to `origin` (world minus origin, y
    /// flipped): what a document whose coordinates were shifted by
    /// `origin` writes in its `viewBox` attribute.
    pub(crate) fn shifted(&self, origin: [f64; 2]) -> ViewBox {
        ViewBox {
            x: self.x - origin[0],
            y: self.y + origin[1],
            width: self.width,
            height: self.height,
        }
    }

    /// The world rectangle this viewBox shows.
    pub fn world_rect(&self) -> Rect {
        let (min_x, min_y, max_x, max_y) = self.world_bounds();
        Rect::new(min_x, min_y, max_x, max_y)
    }

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

/// An INSERT-style placement, composed across nested block references, as
/// the general 2 x 3 matrix `world = (a x + c y + e, b x + d y + f)`.
///
/// A full matrix rather than origin + rotation + per-axis scale: that form
/// cannot represent the composition of a mirrored or non-uniformly scaled
/// parent with a rotated child (`diag(-1, 1) R(t) = R(-t) diag(-1, 1)`, so
/// adding rotations and multiplying scales reflected the child about the
/// parent's insertion point), while matrices compose by multiplication for
/// any combination. The picture is emitted as nested `<g transform>`
/// groups and was always right; this is what the bounds go through.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Transform {
    fn identity() -> Self {
        Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// One INSERT's placement: `insertion_point + R(rotation) diag(x_scale,
    /// y_scale) p`.
    fn placement(insertion_point: Point2D, x_scale: f64, y_scale: f64, rotation: f64) -> Self {
        let (cos, sin) = (rotation.cos(), rotation.sin());
        Transform {
            a: x_scale * cos,
            b: x_scale * sin,
            c: -y_scale * sin,
            d: y_scale * cos,
            e: insertion_point.x,
            f: insertion_point.y,
        }
    }

    /// Local (x, y) -> world (x, y) through this transform.
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The local point this transform sends to `(x, y)`: [`apply`](Self::apply)
    /// run backwards. `None` when the placement is singular (a zero scale
    /// collapses the block) or the answer is not a usable number.
    fn invert_point(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let (dx, dy) = (x - self.e, y - self.f);
        let local = (
            (self.d * dx - self.c * dy) / det,
            (self.a * dy - self.b * dx) / det,
        );
        (local.0.is_finite() && local.1.is_finite()).then_some(local)
    }

    /// The SVG `matrix(a b c d e f)` equivalent, composed with the renderer's
    /// CAD-y-up to SVG-y-down flip on both sides: a point written in the
    /// child's frame, `(u, v) = (x - child.ox, -(y - child.oy))`, maps to the
    /// same point written in the parent's, `(X - parent.ox, -(Y - parent.oy))`.
    ///
    /// The linear part is the placement's with y negated on both sides; the
    /// translation is wherever this placement puts the child frame's own
    /// origin, expressed in the parent's frame. It is therefore zero exactly
    /// when the child frame is the pullback of the parent's -- which is what
    /// [`render_block_ref`] chooses whenever a render origin is engaged.
    fn svg_matrix(&self, parent: Frame, child: Frame) -> [f64; 6] {
        let (ex, ey) = self.apply(child.ox, child.oy);
        [
            clean(self.a),
            neg(self.b),
            neg(self.c),
            clean(self.d),
            parent.x(ex),
            parent.y(ey),
        ]
    }
}

/// Composes a parent world-transform with a child's local transform: applying
/// the result to a point equals applying `child` then `parent` (the matrix
/// product `parent * child`).
fn compose(parent: &Transform, child: &Transform) -> Transform {
    Transform {
        a: parent.a * child.a + parent.c * child.b,
        b: parent.b * child.a + parent.d * child.b,
        c: parent.a * child.c + parent.c * child.d,
        d: parent.b * child.c + parent.d * child.d,
        e: parent.a * child.e + parent.c * child.f + parent.e,
        f: parent.b * child.e + parent.d * child.f + parent.f,
    }
}

// --- render context ----------------------------------------------------

// The block-reference depth and expansion caps this module used to declare
// itself now live in `crate::limits` with the rest of them (imported at the
// top of the file), so every bound a malformed file runs into is named and
// explained in one place.

struct Ctx<'a> {
    ent_min_x: f64,
    ent_max_x: f64,
    ent_min_y: f64,
    ent_max_y: f64,
    unsupported: HashSet<String>,
    tables: &'a Tables,
    depth: u32,
    scale: f64,
    inherited_color: String,
    /// `"<insert>/"` chain of the block references being rendered, so a
    /// text inside a block gets the id the export's records use.
    id_prefix: String,
    transform: Transform,
    /// The origin the coordinates written now are relative to: the
    /// render's origin at the top level, `(0, 0)` inside a block reference
    /// (see [`Frame`]). `transform` and the bounds stay in world units.
    frame: Frame,
    /// The enclosing `<g transform>` matrices composed: what takes a
    /// coordinate written now to the document's own frame. Identity at the
    /// top level. Only an entity whose element cannot be finished until the
    /// viewBox is known needs it -- see [`infinite`].
    svg_matrix: infinite::Matrix,
    /// `<defs>` entries accumulated by HATCH rendering, emitted once into a
    /// top-level `<defs>` by [`to_svg`]. Persists across `render_block_ref`'s
    /// transform save/restore, since a HATCH can appear inside a block too.
    defs: Vec<String>,
    next_def_id: u32,
    /// Namespace for the def ids this render mints (`""` for a model
    /// render, `"p"` for a paper one), so two renders composited into one
    /// document ([`assemble_sheet`]) do not both define `hp0`.
    def_prefix: String,
    /// Remaining budget for `render_block_ref` calls across the whole render
    /// pass, decremented once per call and never restored. The depth cap alone
    /// bounds nesting but not *breadth*: a crafted file with many INSERTs per
    /// block at every level can still fan out combinatorially before the depth
    /// cap is ever reached.
    block_ref_budget: u32,
    /// Bytes of drawing body emitted so far, kept equal to the length of
    /// the strings [`render_entity`] has handed back. The backstop behind
    /// every other cap: once it reaches [`MAX_SVG_BODY_BYTES`] nothing
    /// further is drawn, so no file can make this render grow a string
    /// without bound. See [`crate::limits`].
    emitted: usize,
    /// [`emitted`](Self::emitted) when the current *top-level* entity
    /// started, so one part can be bounded by [`MAX_ENTITY_SVG_BYTES`] as
    /// well as the document by [`MAX_SVG_BODY_BYTES`]. The package needs
    /// the per-part bound: a tile re-assembles and re-parses every part
    /// that touches it.
    entity_start: usize,
    /// What the caps in [`crate::limits`] took away from this render.
    limits: LimitReport,
    /// [`ToSvgOptions::include_hidden`].
    include_hidden: bool,
    /// Entities [`render_entity`] found hidden.
    hidden: usize,
}

impl<'a> Ctx<'a> {
    fn new(tables: &'a Tables) -> Self {
        Ctx {
            ent_min_x: f64::INFINITY,
            ent_max_x: f64::NEG_INFINITY,
            ent_min_y: f64::INFINITY,
            ent_max_y: f64::NEG_INFINITY,
            unsupported: HashSet::new(),
            tables,
            depth: 0,
            scale: 1.0,
            inherited_color: DEFAULT_COLOR.to_string(),
            id_prefix: String::new(),
            transform: Transform::identity(),
            frame: Frame::default(),
            svg_matrix: infinite::IDENTITY,
            defs: Vec::new(),
            next_def_id: 0,
            def_prefix: String::new(),
            block_ref_budget: MAX_BLOCK_REFS,
            emitted: 0,
            entity_start: 0,
            limits: LimitReport::default(),
            include_hidden: false,
            hidden: 0,
        }
    }

    /// The package id of an entity drawn now: its handle behind the block
    /// references it is nested in.
    fn text_id(&self, handle: &str) -> String {
        format!("{}{handle}", self.id_prefix)
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
    /// A result that is not finite, or larger than
    /// [`MAX_WORLD_COORDINATE`] (from a malformed source file or a
    /// degenerate transform), is dropped rather than recorded. Letting
    /// `Infinity` into the running bounds can pin both the min and the max
    /// to `Infinity` (the min-side update never fires because
    /// `Infinity < Infinity` is false), and the box's diagonal then computes
    /// as `NaN`, which panics the `partial_cmp(..).unwrap()` calls in
    /// [`bounds`] instead of just rendering a degenerate point; letting
    /// 1e150 in takes the viewBox -- and every length derived from it -- up
    /// with it.
    fn consider(&mut self, local_x: f64, local_y: f64) {
        let (x, y) = self.transform.apply(local_x, local_y);
        // The same screen `finite` applies to a coordinate as written, but
        // on the value *after* the block transform: a sane coordinate under
        // a corrupt block scale lands just as far out, and the bounds are
        // what the viewBox (and so every length derived from it) is built
        // from. See [`MAX_WORLD_COORDINATE`].
        if !finite([x, y]) {
            return;
        }
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

    /// Records a local-axis-aligned rectangle through all four corners (in
    /// local coordinates, like [`consider`](Self::consider)), so the world
    /// box still contains it under a rotated or mirrored block transform.
    /// Two diagonal corners are enough only for rotations by multiples of
    /// 90 degrees; the four-corner box is conservative (up to sqrt 2 larger
    /// at 45 degrees) but never loses geometry.
    fn consider_rect(&mut self, rect: &Rect) {
        for (x, y) in [
            (rect.min_x, rect.min_y),
            (rect.max_x, rect.min_y),
            (rect.max_x, rect.max_y),
            (rect.min_x, rect.max_y),
        ] {
            self.consider(x, y);
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

    /// A document-unique id for a `<defs>` entry, e.g. `"hp3"` (`"php3"`
    /// in a paper render).
    fn next_def_id(&mut self, prefix: &str) -> String {
        let id = format!("{}{prefix}{}", self.def_prefix, self.next_def_id);
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

/// Whether every one of these values is a real number the renderer can draw
/// with -- finite, and below [`MAX_WORLD_COORDINATE`] in magnitude.
///
/// The entity-level screen for coordinates, radii and sizes: an arm that
/// returns `None` here leaves the entity undrawn, which is the honest
/// reading of a coordinate that is not a number. A corrupt or half-decoded
/// entity used to reach the document as `x1="NaN"` or `r="inf"`, neither of
/// which is in SVG's `<number>` grammar. [`format::clean`] is the backstop
/// underneath for anything not screened here; this is what keeps a bogus
/// entity from being *drawn* at the fallback value.
///
/// The magnitude half of the test matters just as much and is easier to
/// miss: 1e150 is finite, and one entity carrying it takes the measured
/// extents, the viewBox and every length derived from them with it -- see
/// [`MAX_WORLD_COORDINATE`], which is the bound [`crate::crop::Rect::is_sane`]
/// already held a header's extents to.
fn finite<const N: usize>(vals: [f64; N]) -> bool {
    vals.iter()
        .all(|v| v.is_finite() && v.abs() < MAX_WORLD_COORDINATE)
}

/// The points of `pts` that can be drawn at all. One corrupt vertex does not
/// justify dropping a whole polyline, so the rest is still drawn; fewer than
/// two survivors leave nothing to draw.
fn drawable_points(pts: &[Point2D]) -> Vec<Point2D> {
    pts.iter().filter(|p| finite([p.x, p.y])).copied().collect()
}

/// A `<polyline>`, or a `<polygon>` when `closed` -- the shape every polyline
/// entity renders to.
fn polyline_element(pts: &[Point2D], closed: bool, color: &str, frame: Frame) -> String {
    let pts = drawable_points(pts);
    if pts.len() < 2 {
        return String::new();
    }
    let tag = if closed { "polygon" } else { "polyline" };
    format!(
        "<{tag} points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
        frame.points(&pts)
    )
}

/// A `<path>` for a polyline with arc segments: `A` commands for the bulges,
/// `L` for the straight runs. The canvas is the world with y negated, so the
/// picture on screen *is* the world picture and a counter-clockwise
/// (positive-bulge) arc still turns counter-clockwise on screen; in SVG's
/// own y-down frame that is the negative-angle direction, sweep flag 0 --
/// the same flag the ARC branch uses for its always-counter-clockwise arcs.
/// A negative (clockwise) bulge gets sweep flag 1.
fn bulged_polyline_element(
    p: &crate::model::LwPolylineEntity,
    color: &str,
    frame: Frame,
) -> String {
    let segments = crate::geom::polyline_segments(&p.vertices, &p.bulges, p.closed);
    let Some(first) = segments.first() else {
        return polyline_element(&p.vertices, p.closed, color, frame);
    };
    let start = match first {
        crate::geom::Segment::Line { from, .. } | crate::geom::Segment::Arc { from, .. } => *from,
    };
    let mut d = format!("M {} {}", frame.x(start.x), frame.y(start.y));
    for segment in &segments {
        match segment {
            crate::geom::Segment::Line { to, .. } => {
                let _ = write!(d, " L {} {}", frame.x(to.x), frame.y(to.y));
            }
            crate::geom::Segment::Arc { to, bulge, arc, .. } => {
                // A bulge of 1e-160 over a hundred-unit segment is an arc of
                // radius 1e238. An `A` command carrying that hands the
                // rasterizer an arc-to-bezier conversion whose scale has
                // nothing to do with the segment's -- a fuzzed
                // `example_2000.dwg` with one such vertex had not finished
                // rasterizing after five minutes, at any image size. A
                // radius that large *is* a straight line, so it is drawn as
                // one. See [`MAX_WORLD_COORDINATE`].
                if !finite([arc.radius]) {
                    let _ = write!(d, " L {} {}", frame.x(to.x), frame.y(to.y));
                    continue;
                }
                let large = u8::from(bulge.abs() > 1.0);
                let sweep = u8::from(*bulge < 0.0);
                let _ = write!(
                    d,
                    " A {r} {r} 0 {large} {sweep} {} {}",
                    frame.x(to.x),
                    frame.y(to.y),
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
fn dashed_outline(pts: &[Point2D], color: &str, dash: &str, frame: Frame) -> String {
    let pts = drawable_points(pts);
    if pts.len() < 2 {
        return String::new();
    }
    format!(
        "<polygon points=\"{}\" fill=\"none\" stroke-dasharray=\"{dash}\" stroke=\"{color}\"/>",
        frame.points(&pts)
    )
}

/// The `font-size` a CAD text height is drawn at: the height is the cap
/// height, so the em is larger by the bundled face's cap-height ratio.
fn font_size(height: f64) -> f64 {
    height / crate::png::BUNDLED_CAP_HEIGHT
}

/// How far the baseline sits above a text's bottom, in text heights (cap
/// heights). Shared with [`crate::text`], so the estimated box of a
/// bottom-anchored text and the drawn one are placed by the same offset.
const DESCENDER_DROP: f64 = crate::text::DESCENDER_DROP;

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
/// baseline by a descender, 2 middle and 4 (middle-center) drop it by half
/// a cap height, 3 top by a cap height. The text height *is* the cap height
/// (the em is scaled so, see [`font_size`]), so those drops are 0.5 and 1
/// text heights. Glyph widths come from the renderer's font, not AutoCAD's,
/// so the extent is an approximation; the anchor point itself is exact.
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
        0.5
    } else {
        match vertical {
            1 => -DESCENDER_DROP,
            2 => 0.5,
            3 => 1.0,
            _ => 0.0,
        }
    };
    TextAnchor {
        at,
        anchor,
        baseline_drop,
    }
}

/// The height a text is drawn at: the stored one, or 1 when the file
/// stores 0 (a legitimately stored "use the style's height" that no
/// renderer-side default fills in) or nonsense. `font-size="0"` would make
/// usvg drop the text without a trace.
fn effective_text_height(stored: f64) -> f64 {
    if stored.is_finite() && stored > 0.0 {
        stored
    } else {
        1.0
    }
}

/// A single-line `<text>` -- TEXT, ATTRIB and TOLERANCE all render to this.
/// `anchor` positions it (world space); `rotation` is about the anchor.
/// `id` is the entity's package id (`handle`, or `insert/handle` inside a
/// block reference): the export's metrics pre-pass finds the shaped text
/// by it, and every `<text>` names the bundled family so an SVG consumer
/// with the font gets the same glyphs. `height` is the stored text height
/// (the cap height; the em follows from [`font_size`]); a stored 0 is
/// drawn at [`effective_text_height`]'s fallback.
fn text_element(
    id: &str,
    anchor: &TextAnchor,
    height: f64,
    rotation: f64,
    color: &str,
    text: &str,
    frame: Frame,
) -> String {
    let height = effective_text_height(height);
    let (x, y) = (
        frame.x(anchor.at.x),
        frame.y(anchor.at.y) + anchor.baseline_drop * height,
    );
    let anchor_attr = if anchor.anchor == "start" {
        String::new()
    } else {
        format!(" text-anchor=\"{}\"", anchor.anchor)
    };
    format!(
        "<text id=\"{}\" x=\"{x}\" y=\"{y}\" font-size=\"{}\" font-family=\"{}\" fill=\"{color}\" stroke=\"none\"{anchor_attr}{}>{}</text>",
        escape_xml(id),
        font_size(height),
        crate::png::BUNDLED_FONT_FAMILY,
        rotate_transform_attr(rotation, x, y),
        escape_xml(text)
    )
}

/// A small filled triangle at `tip`, pointing away from `from` -- LEADER and
/// MULTILEADER arrowheads.
fn arrowhead_element(
    tip: &Point2D,
    from: &Point2D,
    size: f64,
    color: &str,
    frame: Frame,
) -> String {
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let len = dx.hypot(dy);
    let len = if len == 0.0 { 1.0 } else { len };
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (-uy, ux);
    let (back_x, back_y) = (tip.x - ux * size, tip.y - uy * size);
    let (p2x, p2y) = (back_x + px * size * 0.35, back_y + py * size * 0.35);
    let (p3x, p3y) = (back_x - px * size * 0.35, back_y - py * size * 0.35);
    format!(
        "<polygon points=\"{},{} {},{} {},{}\" fill=\"{color}\" stroke=\"none\"/>",
        frame.x(tip.x),
        frame.y(tip.y),
        frame.x(p2x),
        frame.y(p2y),
        frame.x(p3x),
        frame.y(p3y)
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
            let frame = ctx.frame;
            format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\"/>",
                frame.x(x1),
                frame.y(y1),
                frame.x(x2),
                frame.y(y2)
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
/// ATTDEF children are skipped: an attribute *template* is not drawn. A
/// top-level INSERT's real values are separate ATTRIB entities already
/// rendered, but a *nested* INSERT's are drawn here, from its own
/// `attribs` (the DWG shape) or as ATTRIB children of this block (the DXF
/// shape), never twice.
#[allow(clippy::too_many_arguments)]
fn render_block_ref(
    owner_handle: &str,
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
    if block.entities.is_empty() {
        return String::new();
    }
    // Two file-supplied numbers meet here: how deeply a block reference
    // nests, and how many references there are. A block that references
    // itself makes both unbounded, so both are capped and the drop is
    // counted -- see [`crate::limits`].
    if ctx.depth >= MAX_BLOCK_REF_DEPTH || ctx.block_ref_budget == 0 {
        ctx.limits.block_refs_dropped += 1;
        return String::new();
    }
    ctx.block_ref_budget -= 1;

    let raw_scale = ctx.scale * (x_scale * y_scale).abs().sqrt();
    let cumulative_scale = if raw_scale.is_finite() && raw_scale > 0.0 {
        raw_scale
    } else {
        ctx.scale
    };

    let child_transform = Transform::placement(insertion_point, x_scale, y_scale, rotation);
    // A placement built from a non-finite insertion point, scale or rotation
    // has no SVG matrix at all (`matrix(NaN NaN NaN NaN ...)`), and nothing
    // drawn under it would land anywhere: the whole reference is left out.
    if !finite([
        child_transform.a,
        child_transform.b,
        child_transform.c,
        child_transform.d,
        child_transform.e,
        child_transform.f,
    ]) {
        return String::new();
    }
    // Compose: local (within the block) -> world, via this block's own
    // transform evaluated in the parent's already-established space. The
    // parent's own state is restored afterwards.
    let parent_transform = ctx.transform;
    let parent_depth = ctx.depth;
    let parent_scale = ctx.scale;
    let parent_inherited = std::mem::replace(&mut ctx.inherited_color, color.to_string());
    let child_prefix = format!("{}{owner_handle}/", ctx.id_prefix);
    let parent_prefix = std::mem::replace(&mut ctx.id_prefix, child_prefix);
    // Which point of the block's own space the interior is written about.
    //
    // For a drawing near the origin (`parent_frame` still the default, the
    // overwhelming majority) it is `(0, 0)`: the block's interior is drawn
    // in its own coordinates, which are small, and the placement goes into
    // this group's matrix translation. That is what every block reference
    // did unconditionally -- and it is wrong as soon as a render origin is
    // engaged (a drawing past `ORIGIN_SHIFT_THRESHOLD`, see
    // `choose_origin`), because the interior is then written at whatever
    // magnitude the *block* uses while only the top level is shifted. A
    // DIMENSION's cached geometry block made that visible: it is placed
    // through an identity transform precisely because its children already
    // hold world coordinates, so with the frame reset every line and the
    // label came out at full world magnitude inside the group and the
    // rasterizer's `f32` (a 16-unit step at 2.5e8) quantised them away --
    // silently, and only for dimensions.
    //
    // So when an origin is engaged the interior is written about the point
    // that this placement sends *to* that origin. Then a child's emitted
    // coordinate is its world offset from the render origin (divided by the
    // block's scale, which the group multiplies straight back), the group's
    // own translation is exactly zero, and no number anywhere is far from
    // zero. It covers the identity placement of a dimension and equally a
    // block definition whose geometry sits far from its own base point.
    let parent_frame = ctx.frame;
    let child_frame = if parent_frame == Frame::default() {
        Frame::default()
    } else {
        match child_transform.invert_point(parent_frame.ox, parent_frame.oy) {
            Some((ox, oy)) => Frame { ox, oy },
            // A singular placement (a zero scale) collapses the block to a
            // point whatever the frame, so there is nothing to pull back:
            // the interior keeps its own coordinates.
            None => Frame::default(),
        }
    };
    ctx.frame = child_frame;
    // The `<g transform>` this call emits, needed before the children are
    // rendered: one of them may be an infinite line, which is clipped in
    // the document's frame and so must know what gets it there.
    let group_matrix = child_transform.svg_matrix(parent_frame, child_frame);
    let parent_svg_matrix = ctx.svg_matrix;

    ctx.transform = compose(&parent_transform, &child_transform);
    ctx.svg_matrix = infinite::compose(parent_svg_matrix, group_matrix);
    ctx.depth = parent_depth + 1;
    ctx.scale = cumulative_scale;

    // An ATTRIB that is a child of this block (a DXF whose ATTRIB is owned
    // by the block record) is drawn by the loop below; the same handle may
    // also be linked into its INSERT's attribute chain, and must not be
    // drawn twice -- two <text> elements with one id would also leave the
    // export's metrics pass measuring whichever came last.
    let attrib_children: HashSet<&str> = block
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::Attrib(a) => Some(a.common.handle.as_str()),
            _ => None,
        })
        .collect();
    let mut body_parts = Vec::new();
    for child in &block.entities {
        if matches!(child, Entity::Attdef(_)) {
            continue;
        }
        if let Some(svg) = render_entity(child, ctx) {
            body_parts.push(svg);
        }
        // A nested INSERT's attribute values: they live on the INSERT (the
        // DWG shape, and R2004+ DXF), where nothing else picks them up --
        // only a top-level INSERT's attribs reach the entity list. Their
        // coordinates are this block's, like the INSERT's insertion point,
        // so they are drawn here and not inside the reference.
        if let Entity::Insert(i) = child {
            for a in &i.attribs {
                if attrib_children.contains(a.common.handle.as_str()) {
                    continue;
                }
                if let Some(svg) = render_entity(&Entity::Attrib(a.clone()), ctx) {
                    body_parts.push(svg);
                }
            }
        }
    }

    ctx.transform = parent_transform;
    ctx.svg_matrix = parent_svg_matrix;
    ctx.depth = parent_depth;
    ctx.scale = parent_scale;
    ctx.inherited_color = parent_inherited;
    ctx.id_prefix = parent_prefix;
    ctx.frame = parent_frame;

    if body_parts.is_empty() {
        return String::new();
    }

    // The parent transform is baked into ctx.transform for *bounds* purposes
    // (world-space consider()), but the emitted matrix is only this block's own
    // local transform -- nesting is expressed by nested <g> elements.
    let [a, b, c, d, e, f] = group_matrix;
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
    // The two caps that stand between a malformed file and an unbounded
    // allocation (see [`crate::limits`]). Both are checked before any work
    // is done for this entity, so exhausting the budget unwinds the whole
    // walk -- however deep inside nested block references it happens.
    if ctx.emitted >= MAX_SVG_BODY_BYTES {
        ctx.limits.entities_dropped += 1;
        return None;
    }
    if ctx.emitted - ctx.entity_start >= MAX_ENTITY_SVG_BYTES {
        // Inside a top-level entity that has already drawn more than one
        // entity may. Building stops here, which bounds the work; the part
        // is then dropped whole by `render_selected`, which is also where
        // it is counted.
        return None;
    }
    if drawn_point_count(e) > MAX_ENTITY_POINTS {
        ctx.limits.oversized_entities += 1;
        return None;
    }
    let before = ctx.emitted;
    let svg = if crate::visibility::hidden_reason(e.common(), ctx.tables).is_some() {
        ctx.hidden += 1;
        if !ctx.include_hidden {
            return None;
        }
        render_shown_entity(e, ctx).map(|svg| format!("<g opacity=\"0.5\">{svg}</g>"))
    } else {
        render_shown_entity(e, ctx)
    };
    // The string handed back *contains* everything the children below this
    // call already charged, so the running total is set to its length
    // rather than incremented by it -- nothing is counted twice, and the
    // total stays exactly the size of the body built so far.
    if let Some(svg) = &svg {
        ctx.emitted = before + svg.len();
    }
    svg
}

/// How many points from the file this entity would put into the picture --
/// the count [`MAX_ENTITY_POINTS`] bounds.
///
/// Only the arrays a malformed file can make arbitrarily long are counted;
/// a fixed-shape entity (a LINE, a CIRCLE, a TEXT) is always 0 here, and an
/// INSERT is 0 because what it draws is bounded by the block-reference caps
/// instead. A bulged polyline draws one arc per segment, which is a
/// constant factor on the vertex count, so the vertex count is the measure.
fn drawn_point_count(e: &Entity) -> usize {
    use crate::model::HatchBoundaryPath;
    match e {
        Entity::LwPolyline(p) | Entity::Polyline2D(p) => p.vertices.len(),
        Entity::Polyline3D(p) => p.vertices.len(),
        Entity::Spline(s) => s.fit_points.len() + s.control_points.len(),
        Entity::Leader(l) => l.vertices.len(),
        Entity::MultiLeader(m) => m.lines.iter().map(Vec::len).sum(),
        Entity::MLine(l) => l.vertices.len(),
        Entity::Wipeout(w) => w.boundary.len(),
        Entity::Solid3D(s) => s.wireframe_edges.len(),
        Entity::Region(r) => r.wireframe_edges.len(),
        Entity::PolylinePFace(p) => p.wireframe_edges.len(),
        Entity::Hatch(h) => h
            .boundary_paths
            .iter()
            .map(|path| match path {
                HatchBoundaryPath::Polyline(v) => v.len(),
                HatchBoundaryPath::Edges(edges) => edges.len(),
            })
            .sum(),
        _ => 0,
    }
}

/// [`render_entity`] once visibility is settled.
fn render_shown_entity(e: &Entity, ctx: &mut Ctx) -> Option<String> {
    let color = resolve_entity_color(e.common(), ctx);
    let frame = ctx.frame;
    match e {
        Entity::Line(l) => {
            if !finite([
                l.start_point.x,
                l.start_point.y,
                l.end_point.x,
                l.end_point.y,
            ]) {
                return None;
            }
            ctx.consider(l.start_point.x, l.start_point.y);
            ctx.consider(l.end_point.x, l.end_point.y);
            Some(format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\"/>",
                frame.x(l.start_point.x),
                frame.y(l.start_point.y),
                frame.x(l.end_point.x),
                frame.y(l.end_point.y)
            ))
        }
        Entity::Circle(c) => {
            if !finite([c.center.x, c.center.y, c.radius]) {
                return None;
            }
            // All four corners of the local box: `consider` transforms each
            // point, and two diagonal corners of a box do not bound it
            // once a block rotation is applied (at 45 degrees they land on
            // a vertical line).
            ctx.consider_rect(&Rect::new(
                c.center.x - c.radius,
                c.center.y - c.radius,
                c.center.x + c.radius,
                c.center.y + c.radius,
            ));
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                frame.x(c.center.x),
                frame.y(c.center.y),
                c.radius
            ))
        }
        Entity::Arc(a) => {
            // A stored angle of 1e20 (or the 1.4e247 a fuzzed DWG produced)
            // names no direction: the sweep it would describe is noise, and
            // it used to hang `BulgeArc::bounds` outright. Nothing sensible
            // can be drawn from it, so nothing is.
            if !finite([a.center.x, a.center.y, a.radius])
                || !crate::geom::is_sane_angle(a.start_angle)
                || !crate::geom::is_sane_angle(a.end_angle)
            {
                return None;
            }
            let (x, y, r) = (a.center.x, a.center.y, a.radius);
            let (x1, y1) = (x + r * a.start_angle.cos(), y + r * a.start_angle.sin());
            let (x2, y2) = (x + r * a.end_angle.cos(), y + r * a.end_angle.sin());
            let mut sweep = a.end_angle - a.start_angle;
            if sweep < 0.0 {
                sweep += 2.0 * std::f64::consts::PI;
            }
            // The arc's own extent, not the whole circle's: a large-radius
            // fillet must not stretch the crop (or a frame) to its centre.
            let arc = crate::geom::BulgeArc {
                center: Point2D { x, y },
                radius: r,
                start_angle: a.start_angle,
                end_angle: a.start_angle + sweep,
                sweep,
            };
            let (min_x, min_y, max_x, max_y) = arc.bounds();
            ctx.consider_rect(&Rect::new(min_x, min_y, max_x, max_y));
            let large = if sweep > std::f64::consts::PI { 1 } else { 0 };
            Some(format!(
                "<path d=\"M {} {} A {r} {r} 0 {large} 0 {} {}\" fill=\"none\" stroke=\"{color}\"/>",
                frame.x(x1),
                frame.y(y1),
                frame.x(x2),
                frame.y(y2)
            ))
        }
        Entity::Ellipse(el) => {
            if !finite([
                el.center.x,
                el.center.y,
                el.major_axis_endpoint.x,
                el.major_axis_endpoint.y,
                el.axis_ratio,
            ]) || !crate::geom::is_sane_angle(el.start_angle)
                || !crate::geom::is_sane_angle(el.end_angle)
            {
                return None;
            }
            // DXF 41/42 are parameters on the major axis, not angles -- see
            // [`crate::geom::EllipseArc`]. An ellipse with no stored sweep
            // is the closed one AutoCAD writes as 0 .. 2*pi.
            let arc = crate::geom::EllipseArc {
                center: Point2D {
                    x: el.center.x,
                    y: el.center.y,
                },
                major: Point2D {
                    x: el.major_axis_endpoint.x,
                    y: el.major_axis_endpoint.y,
                },
                ratio: el.axis_ratio,
                start_param: el.start_angle,
                end_param: el.end_angle,
            };
            let (min_x, min_y, max_x, max_y) = arc.bounds();
            if !finite([min_x, min_y, max_x, max_y]) {
                return None;
            }
            ctx.consider_rect(&Rect::new(min_x, min_y, max_x, max_y));
            let rx = arc.major_radius();
            let ry = rx * el.axis_ratio;
            let rot = el
                .major_axis_endpoint
                .y
                .atan2(el.major_axis_endpoint.x)
                .to_degrees();
            let (cx, cy) = (frame.x(el.center.x), frame.y(el.center.y));
            if arc.is_full() {
                // A closed ellipse: an SVG elliptical arc whose two ends
                // coincide is defined to draw nothing, so the closed form
                // stays its own element.
                return Some(format!(
                    "<ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{}\" ry=\"{}\" transform=\"rotate({} {cx} {cy})\" fill=\"none\" stroke=\"{color}\"/>",
                    clean(rx),
                    clean(ry.abs()),
                    neg(rot)
                ));
            }
            let sweep = arc.sweep();
            let start = arc.point_at_param(arc.start_param);
            let end = arc.point_at_param(arc.start_param + sweep);
            let large = u8::from(sweep > std::f64::consts::PI);
            // The canvas is the world with y negated, so a sweep that turns
            // counter-clockwise in the drawing turns clockwise here: sweep
            // flag 0, like the ARC branch. A negative axis ratio mirrors the
            // parameter frame and so reverses that.
            let sweep_flag = u8::from(el.axis_ratio < 0.0);
            Some(format!(
                "<path d=\"M {} {} A {} {} {} {large} {sweep_flag} {} {}\" fill=\"none\" stroke=\"{color}\"/>",
                frame.x(start.x),
                frame.y(start.y),
                clean(rx),
                clean(ry.abs()),
                neg(rot),
                frame.x(end.x),
                frame.y(end.y),
            ))
        }
        Entity::LwPolyline(p) | Entity::Polyline2D(p) => {
            if p.bulges.is_empty() {
                ctx.consider_all(&p.vertices);
                Some(polyline_element(&p.vertices, p.closed, &color, frame))
            } else {
                if let Some((min_x, min_y, max_x, max_y)) =
                    crate::geom::polyline_bounds(&p.vertices, &p.bulges, p.closed)
                {
                    ctx.consider_rect(&Rect::new(min_x, min_y, max_x, max_y));
                }
                Some(bulged_polyline_element(p, &color, frame))
            }
        }
        Entity::Polyline3D(p) => {
            if p.vertices.is_empty() {
                return None;
            }
            ctx.consider_all_3d(&p.vertices);
            Some(polyline_element(&xy(&p.vertices), p.closed, &color, frame))
        }
        Entity::Text(t) => {
            let anchor = text_anchor(
                t.start_point,
                t.alignment_point,
                t.horizontal_alignment,
                t.vertical_alignment,
            );
            if !finite([anchor.at.x, anchor.at.y]) {
                return None;
            }
            ctx.consider(anchor.at.x, anchor.at.y);
            ctx.consider_rect(&crate::text::estimate_text_box(
                anchor.at,
                effective_text_height(t.text_height),
                t.rotation,
                &t.text_plain,
                t.width_factor,
                t.horizontal_alignment,
                t.vertical_alignment,
            ));
            Some(text_element(
                &ctx.text_id(&t.common.handle),
                &anchor,
                t.text_height,
                t.rotation,
                &color,
                &t.text_plain,
                frame,
            ))
        }
        Entity::Attrib(a) => {
            let anchor = text_anchor(
                a.start_point,
                a.alignment_point,
                a.horizontal_alignment,
                a.vertical_alignment,
            );
            if !finite([anchor.at.x, anchor.at.y]) {
                return None;
            }
            ctx.consider(anchor.at.x, anchor.at.y);
            if a.text.is_empty() || a.invisible {
                return Some(String::new());
            }
            ctx.consider_rect(&crate::text::estimate_text_box(
                anchor.at,
                effective_text_height(a.text_height),
                a.rotation,
                &a.text_plain,
                a.width_factor,
                a.horizontal_alignment,
                a.vertical_alignment,
            ));
            Some(text_element(
                &ctx.text_id(&a.common.handle),
                &anchor,
                a.text_height,
                a.rotation,
                &color,
                &a.text_plain,
                frame,
            ))
        }
        Entity::Tolerance(t) => {
            if !finite([t.insertion_point.x, t.insertion_point.y]) {
                return None;
            }
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
                &ctx.text_id(&t.common.handle),
                &anchor,
                t.text_height,
                0.0,
                &color,
                &t.text_plain,
                frame,
            ))
        }
        Entity::MText(m) => {
            if !finite([m.insertion_point.x, m.insertion_point.y]) {
                return None;
            }
            ctx.consider(m.insertion_point.x, m.insertion_point.y);
            ctx.consider_rect(&crate::text::estimate_mtext_box(
                Point2D {
                    x: m.insertion_point.x,
                    y: m.insertion_point.y,
                },
                effective_text_height(m.text_height),
                m.rotation,
                &m.text_plain,
                m.attachment,
                m.extents_width,
                m.extents_height,
            ));
            // Split on '\n' rather than `lines()`: an empty line is a real
            // line (`\P\P` is how a note spaces its paragraphs) and must
            // take up its line height, and a trailing `\P` leaves a
            // trailing empty line -- exactly the lines
            // `estimate_mtext_box` counts, so the drawn block and the
            // estimated one are the same height.
            let lines: Vec<&str> = m
                .text_plain
                .split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l))
                .collect();
            if lines.iter().all(|l| l.is_empty()) {
                return Some(String::new());
            }
            // A stored 0 means "unset" at render time (the parsed value is
            // legitimately 0 in real files), not at parse time.
            let text_height = effective_text_height(m.text_height);
            let line_spacing_factor = if m.line_spacing_factor == 0.0 {
                1.0
            } else {
                m.line_spacing_factor
            };
            // AutoCAD's single ("3-on-5") spacing: 5/3 of the text height
            // between baselines, the same rule `estimate_mtext_box` uses.
            let line_height = text_height * line_spacing_factor * crate::text::LINE_SPACING;
            // The attachment point is a corner or edge of the text block
            // (DXF 71, 1 = top-left ... 9 = bottom-right): columns pick the
            // SVG anchor, rows where the first baseline sits relative to the
            // insertion point.
            //
            // The three rows, derived once (SVG y grows downward, and the
            // text height *is* the cap height -- see `font_size`):
            //   top     the anchor is the cap top of the first line, so the
            //           first baseline is one cap height below it;
            //   middle  the cap band (cap top of the first line down to the
            //           last baseline, `block_height` tall) is centred on
            //           the anchor;
            //   bottom  the anchor is the bottom of the text, i.e. the
            //           descender line of the last line, so the last
            //           baseline sits a descender *above* it -- the same
            //           rule `text_anchor` applies to a single-line TEXT
            //           with vertical alignment 1 (`-DESCENDER_DROP`), and
            //           the sign this had wrong, which drew the block a
            //           third of a line low.
            // `estimate_mtext_box` places the same cap band from the same
            // anchor.
            let column = (m.attachment.clamp(1, 9) - 1) % 3;
            let row = (m.attachment.clamp(1, 9) - 1) / 3;
            let anchor_attr = match column {
                1 => " text-anchor=\"middle\"",
                2 => " text-anchor=\"end\"",
                _ => "",
            };
            let block_height = line_height * (lines.len() as f64 - 1.0) + text_height;
            let last_baseline_drop = match row {
                0 => block_height,
                1 => block_height / 2.0,
                _ => -DESCENDER_DROP * text_height,
            };
            let first_baseline_drop = last_baseline_drop - line_height * (lines.len() as f64 - 1.0);
            let (x, y) = (frame.x(m.insertion_point.x), frame.y(m.insertion_point.y));
            let mut tspans = String::new();
            // An empty line carries no glyphs, so no `<tspan>` of its own:
            // SVG applies a `dy` to the characters that follow it, and an
            // empty element has none, so the shift would simply be lost.
            // Its line height is added to the next drawn line's `dy`
            // instead, which puts every following line exactly where a
            // blank paragraph leaves it.
            let mut pending = 0.0;
            for (i, line) in lines.iter().enumerate() {
                pending += if i == 0 {
                    first_baseline_drop
                } else {
                    line_height
                };
                if line.is_empty() {
                    continue;
                }
                let _ = write!(
                    tspans,
                    "<tspan x=\"{x}\" dy=\"{}\">{}</tspan>",
                    clean(pending),
                    escape_xml(line)
                );
                pending = 0.0;
            }
            Some(format!(
                "<text id=\"{}\" x=\"{x}\" y=\"{y}\" font-size=\"{}\" font-family=\"{}\" fill=\"{color}\" stroke=\"none\"{anchor_attr}{}>{tspans}</text>",
                escape_xml(&ctx.text_id(&m.common.handle)),
                font_size(text_height),
                crate::png::BUNDLED_FONT_FAMILY,
                rotate_transform_attr(m.rotation, x, y)
            ))
        }
        Entity::Point(p) => {
            if !finite([p.position.x, p.position.y]) {
                return None;
            }
            ctx.consider(p.position.x, p.position.y);
            Some(format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"{color}\" stroke=\"none\"/>",
                frame.x(p.position.x),
                frame.y(p.position.y)
            ))
        }
        Entity::Solid(s) => {
            // Classic AutoCAD SOLID vertex order is 1-2-4-3, not 1-2-3-4.
            let pts = [s.corner1, s.corner2, s.corner4, s.corner3];
            // A filled quadrilateral is all four corners or none.
            if drawable_points(&pts).len() < pts.len() {
                return None;
            }
            ctx.consider_all(&pts);
            Some(format!(
                "<polygon points=\"{}\" fill=\"{color}\" fill-opacity=\"0.6\" stroke=\"none\"/>",
                frame.points(&pts)
            ))
        }
        Entity::Face3D(f) => {
            // Unlike SOLID, 3DFACE's 4 corners are already sequential.
            // Edge-visibility flag bits are ignored; all 4 edges always draw.
            let pts = xy(&[f.corner1, f.corner2, f.corner3, f.corner4]);
            ctx.consider_all(&pts);
            Some(format!(
                "<polygon points=\"{}\" fill=\"none\" stroke=\"{color}\"/>",
                frame.points(&pts)
            ))
        }
        Entity::Ray(r) | Entity::XLine(r) => {
            // A construction line has no end, so only its base point counts
            // towards the bounds: a crop that had to contain the line itself
            // would show nothing else. Where the line *stops* is the edge of
            // the picture, which is not known until every entity has been
            // walked, so the element is emitted as a placeholder and cut to
            // the viewBox at assembly time (see [`infinite`]).
            ctx.consider(r.point.x, r.point.y);
            // The element's own frame: y already flipped, like every
            // coordinate written here.
            let (dx, dy) = (r.vector.x, -r.vector.y);
            let len = dx.hypot(dy);
            if !(len.is_finite() && len > 0.0) {
                // No direction: nothing to draw, and nothing to report as
                // unsupported either -- the type is handled.
                return None;
            }
            Some(infinite::placeholder(
                &infinite::InfiniteLine {
                    matrix: ctx.svg_matrix,
                    base: (frame.x(r.point.x), frame.y(r.point.y)),
                    dir: (dx / len, dy / len),
                    both_ways: matches!(e, Entity::XLine(_)),
                },
                &color,
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
                &i.common.handle,
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
            &a.common.handle,
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
            // The cached geometry block is already in final world
            // coordinates, so it is drawn with an identity transform --
            // which is also what tells `render_block_ref` that its interior
            // belongs in the render's own frame, not in a block-local one.
            let svg = render_block_ref(
                &d.common.handle,
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
            Some(dashed_outline(&corners, &color, "2,2", frame))
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
            Some(dashed_outline(&w.boundary, &color, "2,2", frame))
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
            Some(polyline_element(&xy(pts), false, &color, frame))
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
            let line = polyline_element(&pts, false, &color, frame);
            let arrow = if l.has_arrowhead && pts.len() >= 2 {
                arrowhead_element(&pts[0], &pts[1], ARROWHEAD_SIZE, &color, frame)
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
                parts.push(polyline_element(&pts, false, &color, frame));
                let n = pts.len();
                parts.push(arrowhead_element(
                    &pts[n - 1],
                    &pts[n - 2],
                    ARROWHEAD_SIZE,
                    &color,
                    frame,
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
                    polyline_element(
                        &mline_offset_points(&l.vertices, offset),
                        l.closed,
                        &color,
                        frame,
                    )
                })
                .collect();
            Some(lines.join("\n  "))
        }
        Entity::Light(l) => {
            ctx.consider(l.position.x, l.position.y);
            let marker = format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"none\" stroke=\"{color}\"/>",
                frame.x(l.position.x),
                frame.y(l.position.y)
            );
            if !l.has_target {
                return Some(marker);
            }
            ctx.consider(l.target.x, l.target.y);
            let line = format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-dasharray=\"1,1\" stroke=\"{color}\"/>",
                frame.x(l.position.x),
                frame.y(l.position.y),
                frame.x(l.target.x),
                frame.y(l.target.y)
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
pub(crate) fn select_entities_for_space(db: &CadDatabase, space: Space) -> Vec<&Entity> {
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
    /// One SVG fragment per drawn top-level entity, `(handle, svg)`, in
    /// drawing order; [`assemble`] joins them and the export picks the ones
    /// a tile needs.
    pub(crate) parts: Vec<(String, String)>,
    defs: Vec<String>,
    /// The viewBox for an SVG document: the crop padded by the options'
    /// padding or the automatic 2 %. `png.rs` computes its own.
    pub(crate) view_box: ViewBox,
    unsupported: HashSet<String>,
    pub(crate) hidden: usize,
    /// The unpadded crop decision.
    pub(crate) choice: crop::Choice,
    /// The padding `view_box` carries, in drawing units.
    pub(crate) padding_units: f64,
    /// `view_box`'s world rectangle, exact (the viewBox round trip loses a
    /// bit).
    pub(crate) padded_rect: Rect,
    /// Every visible top-level entity's measured extent, in drawing order.
    pub(crate) extents: Vec<Extent>,
    /// The world point the parts' coordinates are relative to (see
    /// [`ToSvgResult::origin`]); `[0, 0]` for a drawing near the origin.
    /// `view_box`, `extents` and the crop stay in world units.
    pub(crate) origin: [f64; 2],
    /// The handles whose part draws an infinite line (RAY, XLINE). Their
    /// extent is the base point alone, but what they draw reaches every
    /// corner of the picture, so a window that keeps parts by their extent
    /// (a tile, a viewport) must keep these whatever their extent says --
    /// [`infinite::resolve`] cuts them to that window anyway.
    pub(crate) unbounded: HashSet<String>,
    /// What the caps in [`crate::limits`] took away from this render.
    pub(crate) limits: LimitReport,
    /// The handles of the entities [`MAX_ENTITY_SVG_BYTES`] left out. The
    /// package excludes them from its records too: the records cover what
    /// the picture shows, and this is not in it.
    pub(crate) oversized: HashSet<String>,
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

/// Renders every selected entity, measures each visible one's extent and
/// decides the crop ([`crate::crop`]), leaving the stroke width unresolved.
pub(crate) fn render(db: &CadDatabase, options: ToSvgOptions) -> Rendered {
    render_selected(
        db,
        select_entities_for_space(db, options.space),
        options,
        "",
    )
}

/// Coordinates this large (in drawing units) get the render shifted to a
/// local origin: below it an `f32` still resolves better than 1/250 of a
/// unit, far finer than any stroke, so every drawing near the origin keeps
/// its world-unit SVG byte for byte.
const ORIGIN_SHIFT_THRESHOLD: f64 = 32768.0;

/// One cheap reference point per entity (an end, a centre, an insertion
/// point, a text anchor), for [`choose_origin`]'s median.
fn reference_point(e: &Entity) -> Option<Point2D> {
    let p3 = |p: &Point3D| Point2D { x: p.x, y: p.y };
    Some(match e {
        Entity::Line(l) => p3(&l.start_point),
        Entity::Circle(c) => p3(&c.center),
        Entity::Arc(a) => p3(&a.center),
        Entity::Ellipse(el) => p3(&el.center),
        Entity::LwPolyline(p) | Entity::Polyline2D(p) => *p.vertices.first()?,
        Entity::Polyline3D(p) => p3(p.vertices.first()?),
        Entity::Text(t) => t.start_point,
        Entity::Attrib(a) => a.start_point,
        Entity::Attdef(a) => a.start_point,
        Entity::Tolerance(t) => p3(&t.insertion_point),
        Entity::MText(m) => p3(&m.insertion_point),
        Entity::Point(p) => p3(&p.position),
        Entity::Solid(s) => s.corner1,
        Entity::Face3D(f) => p3(&f.corner1),
        Entity::Ray(r) | Entity::XLine(r) => p3(&r.point),
        Entity::Insert(i) => p3(&i.insertion_point),
        Entity::AcadTable(a) => p3(&a.insertion_point),
        Entity::Dimension(d) => p3(&d.definition_point),
        Entity::Viewport(v) => p3(&v.center),
        Entity::Wipeout(w) => *w.boundary.first()?,
        Entity::Spline(s) => p3(s.fit_points.first().or(s.control_points.first())?),
        Entity::Solid3D(s) => p3(&s.wireframe_edges.first()?[0]),
        Entity::Region(r) => p3(&r.wireframe_edges.first()?[0]),
        Entity::PolylinePFace(p) => p3(&p.wireframe_edges.first()?[0]),
        Entity::Hatch(h) => match h.boundary_paths.first()? {
            crate::model::HatchBoundaryPath::Polyline(v) => *v.first()?,
            crate::model::HatchBoundaryPath::Edges(edges) => match edges.first()? {
                crate::model::HatchEdge::Line { start } => *start,
                crate::model::HatchEdge::Arc { center, .. }
                | crate::model::HatchEdge::Ellipse { center, .. } => *center,
                crate::model::HatchEdge::Spline { control_points } => *control_points.first()?,
            },
        },
        Entity::Leader(l) => p3(l.vertices.first()?),
        Entity::MultiLeader(m) => p3(m.lines.first()?.first()?),
        Entity::MLine(l) => p3(&l.vertices.first()?.point),
        Entity::Light(l) => p3(&l.position),
        Entity::Unknown { .. } => return None,
    })
}

/// The origin a render of `selected` is written relative to: the per-axis
/// median of the entities' reference points, rounded to whole units, when
/// its magnitude exceeds [`ORIGIN_SHIFT_THRESHOLD`] on either axis; `[0, 0]`
/// otherwise. The median (not the crop's corner) so the shift is settled
/// before anything is rendered and a far-away outlier does not move it.
fn choose_origin(selected: &[&Entity]) -> [f64; 2] {
    let mut xs: Vec<f64> = Vec::with_capacity(selected.len());
    let mut ys: Vec<f64> = Vec::with_capacity(selected.len());
    for p in selected.iter().filter_map(|e| reference_point(e)) {
        if p.x.is_finite() && p.y.is_finite() {
            xs.push(p.x);
            ys.push(p.y);
        }
    }
    if xs.is_empty() {
        return [0.0, 0.0];
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2].round()
    };
    let (mx, my) = (median(&mut xs), median(&mut ys));
    if mx.abs().max(my.abs()) > ORIGIN_SHIFT_THRESHOLD {
        [mx, my]
    } else {
        [0.0, 0.0]
    }
}

/// [`render`] over an explicit entity list (a layout's paper-space block,
/// say) instead of the options' space. `def_prefix` namespaces the
/// `<defs>` ids (see [`Ctx::def_prefix`]): `""` for a render that stands
/// alone or supplies the model of a sheet, `"p"` for the sheet's own
/// entities.
pub(crate) fn render_selected(
    db: &CadDatabase,
    selected: Vec<&Entity>,
    options: ToSvgOptions,
    def_prefix: &str,
) -> Rendered {
    let mut extents: Vec<Extent> = Vec::new();
    let mut body: Vec<(String, String)> = Vec::new();
    let mut unbounded: HashSet<String> = HashSet::new();
    let mut oversized: HashSet<String> = HashSet::new();

    let mut ctx = Ctx::new(&db.tables);
    ctx.include_hidden = options.include_hidden;
    ctx.def_prefix = def_prefix.to_string();
    let origin = choose_origin(&selected);
    ctx.frame = Frame {
        ox: origin[0],
        oy: origin[1],
    };
    for e in selected {
        // A hidden entity never affects the crop, drawn faded or not.
        let hidden = crate::visibility::hidden_reason(e.common(), &db.tables).is_some();
        ctx.reset_entity_bounds();
        ctx.entity_start = ctx.emitted;
        if let Some(svg) = render_entity(e, &mut ctx) {
            if svg.len() >= MAX_ENTITY_SVG_BYTES {
                // One entity that drew more than any entity may. It is left
                // out whole rather than shown half-drawn: a part this size
                // is a block reference that expanded over the entire
                // picture, and a package re-assembles and re-parses every
                // part each of its tiles touches. See [`crate::limits`].
                ctx.limits.oversized_parts += 1;
                ctx.emitted = ctx.entity_start;
                oversized.insert(e.common().handle.clone());
            } else if !svg.is_empty() {
                if svg.contains(infinite::MARKER) {
                    unbounded.insert(e.common().handle.clone());
                }
                body.push((e.common().handle.clone(), svg));
            }
        }
        if hidden {
            continue;
        }
        if let Some(b) = ctx.entity_box() {
            extents.push(Extent {
                handle: e.common().handle.clone(),
                type_name: e.type_name().to_string(),
                rect: Rect::from_box(&b),
            });
        }
    }

    let choice = crop::choose(&extents, &db.header, options.crop);
    // What the crop leaves out is not drawn either: a 3256x INSERT clipped
    // by the viewBox would still cross the whole picture.
    let excluded: HashSet<&str> = choice.excluded.iter().map(|x| x.handle.as_str()).collect();
    let body: Vec<(String, String)> = body
        .into_iter()
        .filter(|(handle, _)| !excluded.contains(handle.as_str()))
        .collect();
    let padding_units = options
        .padding
        .unwrap_or_else(|| crop::auto_padding(&choice.rect, None));
    let padded_rect = choice.rect.padded(padding_units);
    let view_box = ViewBox::from_world(&padded_rect);

    Rendered {
        parts: body,
        defs: ctx.defs,
        view_box,
        unsupported: ctx.unsupported,
        hidden: ctx.hidden,
        choice,
        padding_units,
        padded_rect,
        extents,
        origin,
        unbounded,
        oversized,
        limits: ctx.limits,
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
pub(crate) fn assemble(
    rendered: &Rendered,
    view_box: &ViewBox,
    effective_stroke_width: f64,
) -> String {
    assemble_subset(rendered, view_box, effective_stroke_width, |_| true)
}

/// [`assemble`] with only the parts `keep` accepts (by entity handle): what
/// a tile needs, so rasterizing it does not pay for the whole drawing.
/// `view_box` is in world units; the document gets it shifted by
/// `rendered.origin`, like the parts.
///
/// This is also where an infinite line learns where to stop
/// ([`infinite::resolve`]): `view_box` is the first thing that says how far
/// the picture reaches.
pub(crate) fn assemble_subset(
    rendered: &Rendered,
    view_box: &ViewBox,
    effective_stroke_width: f64,
    keep: impl Fn(&str) -> bool,
) -> String {
    let body: Vec<&str> = rendered
        .parts
        .iter()
        .filter(|(handle, _)| keep(handle))
        .map(|(_, svg)| svg.as_str())
        .collect();
    let vb = view_box.shifted(rendered.origin);
    let resolved_body = infinite::resolve(
        resolve_stroke_widths(&body.join("\n  "), effective_stroke_width),
        infinite::window(&vb, effective_stroke_width),
    );
    // HATCH pattern defs carry stroke-width placeholders too. Kept separate
    // from the body only so an empty defs list emits no <defs> block at all.
    let defs_block = if rendered.defs.is_empty() {
        String::new()
    } else {
        let resolved_defs =
            resolve_stroke_widths(&rendered.defs.join("\n  "), effective_stroke_width);
        format!("<defs>\n  {resolved_defs}\n</defs>\n  ")
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" stroke=\"black\" stroke-width=\"{effective_stroke_width}\">\n  {defs_block}{resolved_body}\n</svg>",
        vb.x, vb.y, vb.width, vb.height
    )
}

/// Multiplies every `@@SW@@<scale>@@` stroke placeholder in `fragment` by
/// `factor`: a model fragment placed inside a viewport scaled by `s` keeps
/// its pixel-constant stroke when its placeholders carry `s` too.
fn scale_stroke_placeholders(fragment: &str, factor: f64) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(start) = rest.find("@@SW@@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "@@SW@@".len()..];
        match after.find("@@") {
            Some(end) => {
                let scale: f64 = after[..end].parse().unwrap_or(1.0);
                out.push_str(&stroke_width_placeholder(scale * factor));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Appends `-{suffix}` to every `url(#id)` reference in `fragment`.
fn suffix_url_refs(fragment: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(start) = rest.find("url(#") {
        let after = &rest[start + "url(#".len()..];
        match after.find(')') {
            Some(end) => {
                out.push_str(&rest[..start]);
                let _ = write!(out, "url(#{}-{suffix})", &after[..end]);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Appends `-{suffix}` to the first `id="..."` in `def` (the element's
/// own id).
fn suffix_def_id(def: &str, suffix: &str) -> String {
    let Some(start) = def.find("id=\"") else {
        return def.to_string();
    };
    let after = &def[start + "id=\"".len()..];
    let Some(end) = after.find('"') else {
        return def.to_string();
    };
    format!(
        "{}id=\"{}-{suffix}\"{}",
        &def[..start],
        &after[..end],
        &after[end + 1..]
    )
}

/// A paper layout as one SVG document: the sheet's own entities (`paper`),
/// and for every viewport in `viewports` the model (`model`'s fragments,
/// those whose extent touches the viewport's model window and whose layer
/// is not frozen in it) clipped to the viewport's frame and transformed
/// with [`ViewportEntity::model_to_paper`]. `view_box` is the sheet.
///
/// `paper` must have been rendered with a def prefix (`"p"`) so its
/// `<defs>` ids cannot collide with the model's, and every viewport gets
/// its own copy of the model's defs (ids suffixed with the viewport's
/// handle, references in its fragments rewritten to match) with the
/// pattern strokes scaled by that viewport's scale: `<pattern>` content is
/// drawn in the referencing element's user space, i.e. inside the
/// viewport's matrix, so one shared def could not serve two viewports at
/// different scales. usvg resolves a duplicated id to the last definition,
/// which used to hand the paper's hatches the model's patterns.
///
/// Each render carries its own origin: the paper parts and the sheet's
/// `view_box` are written relative to `paper.origin`, and the viewport
/// matrix takes the model parts from their `model.origin`-relative
/// coordinates to the paper's.
///
/// An infinite line (RAY, XLINE) inside a viewport carries that same
/// matrix into its placeholder, so the one clip at the end cuts the
/// sheet's own construction lines and the model's alike, in the sheet's
/// frame -- see [`infinite`].
pub(crate) fn assemble_sheet(
    paper: &Rendered,
    model: &Rendered,
    viewports: &[&crate::model::ViewportEntity],
    model_extents: &std::collections::HashMap<&str, Rect>,
    model_layers: &std::collections::HashMap<&str, &str>,
    view_box: &ViewBox,
    effective_stroke_width: f64,
) -> String {
    let mut body = String::new();
    for (_, svg) in &paper.parts {
        body.push_str(svg);
        body.push_str("\n  ");
    }
    let paper_frame = Frame {
        ox: paper.origin[0],
        oy: paper.origin[1],
    };
    let [mx, my] = model.origin;
    let mut defs: Vec<String> = paper.defs.clone();
    for vp in viewports {
        let Some(scale) = vp.scale() else { continue };
        let Some(window) = vp.model_window() else {
            continue;
        };
        let window_rect = window.iter().fold(
            Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |r, p| {
                Rect::new(
                    r.min_x.min(p.x),
                    r.min_y.min(p.y),
                    r.max_x.max(p.x),
                    r.max_y.max(p.y),
                )
            },
        );
        // model -> paper, in SVG (y-down) coordinates: X = a x + c y + e,
        // Y = b x + d y + f with the paper's y flipped like the model's.
        let (co, si) = (vp.twist.cos(), vp.twist.sin());
        let (tx, ty) = (vp.view_target.x, vp.view_target.y);
        let (vx, vy) = (vp.view_center.x, vp.view_center.y);
        let (cx, cy) = (vp.center.x, vp.center.y);
        let a = scale * co;
        let b = -scale * si;
        let c = scale * si;
        let d = scale * co;
        let e = cx - scale * (co * tx - si * ty) - scale * vx;
        let f = -cy + scale * (si * tx + co * ty) + scale * vy;
        // The model parts are written as (x - mx, -(y - my)) and the sheet
        // as (X - px, -(Y - py)): fold both origins into the translation.
        let e = e + a * mx - c * my - paper_frame.ox;
        let f = f + b * mx - d * my + paper_frame.oy;
        let handle = escape_xml(&vp.common.handle);
        defs.extend(
            model
                .defs
                .iter()
                .map(|def| scale_stroke_placeholders(&suffix_def_id(def, &handle), scale)),
        );
        let _ = write!(
            body,
            "<clipPath id=\"vp-{handle}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>\n  <g clip-path=\"url(#vp-{handle})\"><g transform=\"matrix({} {} {} {} {} {})\" stroke-width=\"{}\">\n  ",
            paper_frame.x(cx - vp.width / 2.0),
            paper_frame.y(cy + vp.height / 2.0),
            clean(vp.width),
            clean(vp.height),
            clean(a),
            clean(b),
            clean(c),
            clean(d),
            clean(e),
            clean(f),
            stroke_width_placeholder(scale)
        );
        for (h, svg) in &model.parts {
            if let Some(layer) = model_layers.get(h.as_str()) {
                if vp.frozen_layers.iter().any(|f| f == layer) {
                    continue;
                }
            }
            // An infinite line's extent is its base point, which says
            // nothing about where it is seen; it stays in and is clipped
            // to the sheet below.
            if !model.unbounded.contains(h.as_str())
                && model_extents
                    .get(h.as_str())
                    .is_some_and(|r| !r.intersects(&window_rect))
            {
                continue;
            }
            // This viewport's matrix is one more frame between the model
            // fragment and the document, so an infinite line inside it
            // carries it too: the clip below happens in the sheet's frame.
            body.push_str(&infinite::transform(
                suffix_url_refs(&scale_stroke_placeholders(svg, scale), &handle),
                [a, b, c, d, e, f],
            ));
            body.push_str("\n  ");
        }
        body.push_str("</g></g>\n  ");
    }
    let vb = view_box.shifted(paper.origin);
    // Both the sheet's own fragments and the ones inside a viewport are now
    // written in the sheet's frame, so one clip to the sheet serves both.
    let resolved_body = infinite::resolve(
        resolve_stroke_widths(&body, effective_stroke_width),
        infinite::window(&vb, effective_stroke_width),
    );
    let defs_block = if defs.is_empty() {
        String::new()
    } else {
        let resolved_defs = resolve_stroke_widths(&defs.join("\n  "), effective_stroke_width);
        format!("<defs>\n  {resolved_defs}\n</defs>\n  ")
    };
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
        svg: assemble(&rendered, &rendered.view_box, stroke_width),
        view_box: rendered.view_box,
        origin: rendered.origin,
        unsupported_types: rendered.unsupported_types(),
        hidden: rendered.hidden,
        crop: rendered
            .choice
            .report(rendered.padded_rect, rendered.padding_units),
        limits: rendered.limits,
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
            ..EntityCommon::default()
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
            "T",
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
        // A 5-ary tree 20 levels deep is 5^20 (~9.5e13) instantiations, so
        // the depth cap cannot be what ended this walk: one of the budgets
        // did, and the part it produced is bounded either way.
        assert!(
            ctx.block_ref_budget < MAX_BLOCK_REFS,
            "the walk should have spent some of its expansion budget"
        );
        assert!(
            svg.len() < MAX_ENTITY_SVG_BYTES * 2,
            "the emitted part grew to {} bytes",
            svg.len()
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
    fn a_stored_text_height_of_zero_is_never_emitted_as_font_size_zero() {
        // usvg drops a `font-size="0"` text without a trace, which is how a
        // TEXT whose file stores height 0 (meaning "the style's height")
        // vanished from every image. The fallback is the MTEXT branch's 1.
        let anchor = TextAnchor {
            at: Point2D { x: 3.0, y: 4.0 },
            anchor: "start",
            baseline_drop: 0.0,
        };
        for stored in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let svg = text_element(
                "T",
                &anchor,
                stored,
                0.0,
                "#000000",
                "ZERO",
                Frame::default(),
            );
            assert!(!svg.contains("font-size=\"0\""), "{svg}");
            assert!(
                svg.contains(&format!("font-size=\"{}\"", font_size(1.0))),
                "{svg}"
            );
        }
        assert_eq!(effective_text_height(2.5), 2.5);
        assert_eq!(effective_text_height(0.0), 1.0);
    }

    #[test]
    fn text_is_drawn_with_capitals_the_size_of_the_cad_height() {
        // A height-2 TEXT: the em is 2 / 0.733 = 2.7285 so the capitals
        // (0.733 em in the bundled face) come out exactly 2 units tall. A
        // top-aligned one (vertical 3) has its baseline one cap height, i.e.
        // one text height, below the anchor; middle half of that; bottom a
        // descender (0.2 em = 0.2729 heights) above it.
        let at = Point2D { x: 0.0, y: 10.0 };
        let plain = text_element(
            "T",
            &text_anchor(at, None, 0, 0),
            2.0,
            0.0,
            "#000",
            "A",
            Frame::default(),
        );
        assert!(
            plain.contains(&format!("font-size=\"{}\"", 2.0 / 0.733)),
            "{plain}"
        );
        assert!(plain.contains(" y=\"-10\""), "{plain}");
        let top = text_element(
            "T",
            &text_anchor(at, Some(at), 0, 3),
            2.0,
            0.0,
            "#000",
            "A",
            Frame::default(),
        );
        assert!(top.contains(" y=\"-8\""), "{top}");
        let middle = text_element(
            "T",
            &text_anchor(at, Some(at), 0, 2),
            2.0,
            0.0,
            "#000",
            "A",
            Frame::default(),
        );
        assert!(middle.contains(" y=\"-9\""), "{middle}");
        let bottom = text_element(
            "T",
            &text_anchor(at, Some(at), 0, 1),
            2.0,
            0.0,
            "#000",
            "A",
            Frame::default(),
        );
        let expected_y = -10.0 - 2.0 * 0.2 / 0.733;
        assert!(
            bottom.contains(&format!(" y=\"{expected_y}\"")),
            "{bottom} vs {expected_y}"
        );
    }

    /// The value of every `id="..."` attribute in `svg`, in order.
    fn ids(svg: &str) -> Vec<&str> {
        svg.match_indices("id=\"")
            .map(|(at, _)| {
                let after = &svg[at + 4..];
                &after[..after.find('"').unwrap()]
            })
            .collect()
    }

    /// The `stroke-width` of the `<line>` inside the `<pattern id="{id}">`.
    fn pattern_stroke_width(svg: &str, id: &str) -> f64 {
        let start = svg
            .find(&format!("<pattern id=\"{id}\""))
            .unwrap_or_else(|| panic!("pattern {id} in {svg}"));
        let def = &svg[start..start + svg[start..].find("</pattern>").unwrap()];
        let at = def.find("stroke-width=\"").unwrap() + "stroke-width=\"".len();
        def[at..at + def[at..].find('"').unwrap()].parse().unwrap()
    }

    #[test]
    fn a_sheet_keeps_paper_and_model_defs_apart_and_scales_each_viewports_patterns() {
        // hatched_viewport_r2000.dxf: a pattern hatch in model space
        // (vertical lines, handle 30) and one in paper space (horizontal
        // lines, handle 31), each the first pattern of its render, plus a
        // viewport (handle 2A) at scale 2 twisted 30 degrees.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/hatched_viewport_r2000.dxf"
        );
        let db = crate::parse(path).expect("fixture parses");
        let options = ToSvgOptions {
            crop: CropMode::Raw,
            padding: Some(0.0),
            ..Default::default()
        };
        let model = render(&db, options);
        let paper_block = &db.tables.block_records["*Paper_Space"];
        let paper = render_selected(&db, paper_block.entities.iter().collect(), options, "p");
        let viewports: Vec<&crate::model::ViewportEntity> = paper_block
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Viewport(v) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(viewports.len(), 1);
        assert_eq!(viewports[0].scale(), Some(2.0));
        let extents: std::collections::HashMap<&str, Rect> = model
            .extents
            .iter()
            .map(|e| (e.handle.as_str(), e.rect))
            .collect();
        let layers = std::collections::HashMap::new();
        let view_box = ViewBox::from_world(&Rect::new(0.0, 0.0, 297.0, 210.0));
        let svg = assemble_sheet(
            &paper, &model, &viewports, &extents, &layers, &view_box, 0.5,
        );

        // Every id is defined once: the paper's pattern under its own
        // prefix, the model's as a copy for the viewport, none bare.
        let mut all = ids(&svg);
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count, "duplicate ids in {svg}");
        assert!(svg.contains("<pattern id=\"php0\""), "{svg}");
        assert!(svg.contains("<pattern id=\"hp0-2A\""), "{svg}");
        assert!(!svg.contains("<pattern id=\"hp0\""), "{svg}");
        // Each hatch fills with its own pattern: the paper one horizontal
        // (rotate(0)), the model one vertical (rotate(-90) on the y-down
        // canvas), the viewport's fragment rewritten to the copy.
        assert!(svg.contains("fill=\"url(#php0)\""), "{svg}");
        assert!(svg.contains("fill=\"url(#hp0-2A)\""), "{svg}");
        assert!(!svg.contains("url(#hp0)"), "{svg}");
        let paper_def = &svg[svg.find("<pattern id=\"php0\"").unwrap()..];
        assert!(paper_def[..paper_def.find('>').unwrap()].contains("rotate(0)"));
        let model_def = &svg[svg.find("<pattern id=\"hp0-2A\"").unwrap()..];
        assert!(model_def[..model_def.find('>').unwrap()].contains("rotate(-90)"));
        // Pattern content is drawn in the referencing element's user
        // space. The paper hatch is in sheet units: its line is the sheet
        // stroke, 0.5. The model copy is used inside the viewport's
        // matrix(2 ...), so its line must be 0.5 / 2 to come out 0.5 on
        // the sheet -- the same rule the viewport group's own
        // stroke-width follows.
        assert!((pattern_stroke_width(&svg, "php0") - 0.5).abs() < 1e-12);
        assert!((pattern_stroke_width(&svg, "hp0-2A") - 0.25).abs() < 1e-12);
        assert!(
            svg.contains("stroke-width=\"0.25\">"),
            "the viewport group: {svg}"
        );
    }

    #[test]
    fn a_far_away_model_is_composited_through_the_viewport_from_its_own_origin() {
        use crate::model::{LineEntity, ViewportEntity};
        use crate::tables::BlockRecord;
        // Model: one LINE at 1e7, so the model render is written about
        // its rounded median origin (the line's start, (10000010,
        // 10000010)) as x1=0 y1=0 x2=80 y2=-40. Paper: a viewport of 200 x
        // 120 at (150,100) showing 60 units of model height (scale 2) with
        // no twist, centred on model (1e7+50, 1e7+25). By
        // ViewportEntity::model_to_paper the line's ends land on paper at
        // (70,70) and (230,150), i.e. SVG (70,-70) and (230,-150); the
        // emitted matrix must take the shifted model coordinates there.
        let far = 1.0e7;
        let common = |handle: &str| EntityCommon {
            handle: handle.into(),
            layer: "0".into(),
            ..EntityCommon::default()
        };
        let line = Entity::Line(LineEntity {
            common: common("L"),
            start_point: Point3D {
                x: far + 10.0,
                y: far + 10.0,
                z: 0.0,
            },
            end_point: Point3D {
                x: far + 90.0,
                y: far + 50.0,
                z: 0.0,
            },
        });
        let viewport = ViewportEntity {
            common: common("V"),
            center: Point3D {
                x: 150.0,
                y: 100.0,
                z: 0.0,
            },
            width: 200.0,
            height: 120.0,
            view_center: Point2D {
                x: far + 50.0,
                y: far + 25.0,
            },
            view_size: 60.0,
            view_target: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            view_direction: crate::geom::WORLD_Z,
            twist: 0.0,
            lens_length: 50.0,
            status_flag: 0,
            on: true,
            id: 2,
            frozen_layers: Vec::new(),
        };
        let mut tables = Tables::default();
        tables.block_records.insert(
            "*Model_Space".into(),
            BlockRecord {
                name: "*Model_Space".into(),
                entities: vec![line.clone()],
            },
        );
        let db = CadDatabase::new(vec![line], tables);
        let options = ToSvgOptions {
            crop: CropMode::Raw,
            padding: Some(0.0),
            ..Default::default()
        };
        let model = render(&db, options);
        assert_eq!(model.origin, [far + 10.0, far + 10.0]);
        assert!(model.parts[0]
            .1
            .contains("x1=\"0\" y1=\"0\" x2=\"80\" y2=\"-40\""));
        let paper = render_selected(&db, Vec::new(), options, "p");
        assert_eq!(paper.origin, [0.0, 0.0]);
        let extents = std::collections::HashMap::new();
        let layers = std::collections::HashMap::new();
        let view_box = ViewBox::from_world(&Rect::new(0.0, 0.0, 297.0, 210.0));
        let svg = assemble_sheet(
            &paper,
            &model,
            &[&viewport],
            &extents,
            &layers,
            &view_box,
            0.5,
        );
        let at = svg.find("matrix(").expect("the viewport group") + "matrix(".len();
        let m: Vec<f64> = svg[at..at + svg[at..].find(')').unwrap()]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        let apply = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        let expected = |x: f64, y: f64| {
            let p = viewport.model_to_paper(Point2D { x, y }).unwrap();
            (p.x, -p.y)
        };
        for ((x, y), (wx, wy)) in [
            ((0.0, 0.0), (far + 10.0, far + 10.0)),
            ((80.0, -40.0), (far + 90.0, far + 50.0)),
        ] {
            let (px, py) = apply(x, y);
            let (ex, ey) = expected(wx, wy);
            assert!(
                (px - ex).abs() < 1e-6 && (py - ey).abs() < 1e-6,
                "({px}, {py}) vs ({ex}, {ey})"
            );
        }
        assert!((apply(0.0, 0.0).0 - 70.0).abs() < 1e-6 && (apply(0.0, 0.0).1 + 70.0).abs() < 1e-6);
        // The matrix itself carries only sheet-sized numbers.
        assert!(m.iter().all(|v| v.abs() < 1000.0), "{m:?}");
    }

    #[test]
    fn suffixing_rewrites_every_reference_and_only_the_defs_own_id() {
        assert_eq!(
            suffix_url_refs(
                "<path fill=\"url(#hp0)\"/><path fill=\"url(#hg12)\"/>",
                "2A"
            ),
            "<path fill=\"url(#hp0-2A)\"/><path fill=\"url(#hg12-2A)\"/>"
        );
        assert_eq!(suffix_url_refs("no refs", "2A"), "no refs");
        assert_eq!(
            suffix_def_id(
                "<pattern id=\"hp0\" width=\"1\"><line id=\"x\"/></pattern>",
                "2A"
            ),
            "<pattern id=\"hp0-2A\" width=\"1\"><line id=\"x\"/></pattern>"
        );
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
        let t = Transform::placement(Point2D { x: 10.0, y: 20.0 }, 2.0, 2.0, 0.0);
        let (x, y) = t.apply(1.0, 1.0);
        close(x, 12.0);
        close(y, 22.0);
        // Rotation first, then the translation: (1,0) scaled by 3 and
        // turned 90 degrees is (0,3), landing at (10,23).
        let t = Transform::placement(
            Point2D { x: 10.0, y: 20.0 },
            3.0,
            3.0,
            std::f64::consts::FRAC_PI_2,
        );
        let (x, y) = t.apply(1.0, 0.0);
        close(x, 10.0);
        close(y, 23.0);
    }

    #[test]
    fn compose_applies_child_transform_within_parents_space() {
        let parent = Transform::placement(Point2D { x: 10.0, y: 0.0 }, 1.0, 1.0, 0.0);
        let child = Transform::placement(Point2D { x: 1.0, y: 1.0 }, 2.0, 2.0, 0.0);
        let composed = compose(&parent, &child);
        let (x, y) = composed.apply(0.0, 0.0);
        close(x, 11.0);
        close(y, 1.0);
        let (x, y) = composed.apply(1.0, 1.0);
        close(x, 13.0);
        close(y, 3.0);
    }

    #[test]
    fn compose_matches_the_nested_picture_under_mirrored_and_non_uniform_parents() {
        let quarter = std::f64::consts::FRAC_PI_2;
        // A child rotated 90 degrees inside a parent mirrored about the
        // vertical axis at (100,100): the child sends (10,0) to (0,10), the
        // parent then sends (0,10) to (100 - 0, 100 + 10) = (100,110). The
        // old rotation-sum form gave the parent's rotation 0 + 90 with x
        // scale -1: (100 - 0, 100 - 10) = (100,90), the reflection of the
        // drawn point about the insertion point.
        let parent = Transform::placement(Point2D { x: 100.0, y: 100.0 }, -1.0, 1.0, 0.0);
        let child = Transform::placement(Point2D { x: 0.0, y: 0.0 }, 1.0, 1.0, quarter);
        let composed = compose(&parent, &child);
        let (x, y) = composed.apply(10.0, 0.0);
        close(x, 100.0);
        close(y, 110.0);
        let (x, y) = composed.apply(10.0, 2.0);
        close(x, 100.0 + 2.0);
        close(y, 110.0);

        // A non-uniform parent (2, 1) over the same child: (10,0) -> child
        // (0,10) -> parent (100 + 0, 100 + 10); (0,2) -> child (-2,0) ->
        // parent (100 - 4, 100). No origin + rotation + scale form can
        // express this frame (its axes are not perpendicular after
        // scaling), only the matrix.
        let parent = Transform::placement(Point2D { x: 100.0, y: 100.0 }, 2.0, 1.0, 0.0);
        let composed = compose(&parent, &child);
        let (x, y) = composed.apply(10.0, 0.0);
        close(x, 100.0);
        close(y, 110.0);
        let (x, y) = composed.apply(0.0, 2.0);
        close(x, 96.0);
        close(y, 100.0);

        // In general: compose(p, c).apply(q) == p.apply(c.apply(q)), which
        // is exactly what the nested <g transform> groups draw.
        let frames = [
            Transform::placement(Point2D { x: 3.0, y: -7.0 }, -2.0, 0.5, 0.3),
            Transform::placement(Point2D { x: -1.0, y: 4.0 }, 1.5, -1.5, -1.1),
            Transform::placement(Point2D { x: 0.0, y: 0.0 }, 1.0, 3.0, 2.0),
        ];
        for p in &frames {
            for c in &frames {
                let composed = compose(p, c);
                for (qx, qy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-3.5, 2.25)] {
                    let (cx, cy) = c.apply(qx, qy);
                    let (wx, wy) = p.apply(cx, cy);
                    let (x, y) = composed.apply(qx, qy);
                    close(x, wx);
                    close(y, wy);
                }
            }
        }
        // And the emitted matrix is the placement with y negated on both
        // sides: matrix(a -b -c d e -f).
        let t = Transform::placement(Point2D { x: 5.0, y: 6.0 }, 2.0, 3.0, quarter);
        let [a, b, c, d, e, f] = t.svg_matrix(Frame::default(), Frame::default());
        close(a, 0.0);
        close(b, -2.0);
        close(c, 3.0);
        close(d, 0.0);
        close(e, 5.0);
        close(f, -6.0);
    }

    /// One entity rendered on its own, with no block nesting and no origin
    /// shift: the fragment the document would carry.
    fn render_one(e: &Entity) -> String {
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        render_entity(e, &mut ctx).unwrap_or_default()
    }

    fn mtext(text_plain: &str, attachment: u16) -> Entity {
        Entity::MText(crate::model::MTextEntity {
            common: EntityCommon {
                handle: "M".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            insertion_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            text: text_plain.into(),
            text_plain: text_plain.into(),
            text_height: 10.0,
            rotation: 0.0,
            line_spacing_factor: 1.0,
            attachment,
            rect_width: 0.0,
            extents_width: 0.0,
            extents_height: 0.0,
            x_axis_dir: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            style: String::new(),
        })
    }

    /// Every `dy="..."` in `svg`, in order.
    fn dys(svg: &str) -> Vec<f64> {
        svg.match_indices("dy=\"")
            .map(|(at, _)| {
                let after = &svg[at + 4..];
                after[..after.find('"').unwrap()].parse().unwrap()
            })
            .collect()
    }

    #[test]
    fn an_mtext_attachment_row_places_the_cap_band_the_estimate_predicts() {
        // Height 10, single spacing: baselines 10 * 5/3 = 16.6667 apart, and
        // the cap band (first line's cap top down to the last baseline) is
        // 10 tall for one line, 2 * 16.6667 + 10 = 43.3333 for three. SVG y
        // grows downward, so a positive dy is below the anchor.
        //
        // row 0 (top):    the anchor is the cap top, so the first baseline
        //                 is one cap height below it: +10.
        // row 1 (middle): the band is centred, so its top is half a band
        //                 above the anchor and the first baseline is
        //                 10 - 43.3333/2 = -11.6667 (one line: 10 - 5 = 5).
        // row 2 (bottom): the anchor is the bottom of the text, so the LAST
        //                 baseline is one descender above it,
        //                 -0.2/0.733 * 10 = -2.72851, and the first is
        //                 2 * 16.6667 above that: -36.0618 (one line:
        //                 -2.72851 itself).
        let descender = 0.2 / 0.733 * 10.0;
        let line_height = 10.0 * 5.0 / 3.0;
        for (attachment, one_line, first_of_three) in [
            (1, 10.0, 10.0),
            (4, 5.0, 10.0 - (2.0 * line_height + 10.0) / 2.0),
            (7, -descender, -descender - 2.0 * line_height),
        ] {
            let svg = render_one(&mtext("Hxg", attachment));
            close(dys(&svg)[0], one_line);
            let svg = render_one(&mtext("Hxg\nsecond\nthird", attachment));
            close(dys(&svg)[0], first_of_three);
        }

        // ... and `estimate_mtext_box`, which is what the crop and the
        // entity's extent are built from, places the same band from the
        // same anchor: bottom attachment puts its lower edge one descender
        // above the anchor, where the last baseline now is.
        let anchor = Point2D { x: 0.0, y: 0.0 };
        let total = 10.0 + 2.0 * line_height;
        let bottom =
            crate::text::estimate_mtext_box(anchor, 10.0, 0.0, "Hxg\nsecond\nthird", 7, 0.0, 0.0);
        close(bottom.min_y, descender);
        close(bottom.max_y, descender + total);
        let top =
            crate::text::estimate_mtext_box(anchor, 10.0, 0.0, "Hxg\nsecond\nthird", 1, 0.0, 0.0);
        close(top.max_y, 0.0);
        close(top.min_y, -total);
    }

    #[test]
    fn an_mtext_blank_line_keeps_its_line_height() {
        // `ALPHA\P\PBRAVO` is three lines, the middle one empty, so BRAVO's
        // baseline is two line heights (2 * 16.6667 = 33.3333) below
        // ALPHA's -- not one, which is where dropping the blank line put
        // it. An empty line carries no glyphs, so it gets no `<tspan>` of
        // its own (SVG applies a `dy` to the characters that follow it, and
        // an empty element has none): its height is added to the next
        // drawn line instead.
        let svg = render_one(&mtext("ALPHA\n\nBRAVO", 1));
        assert_eq!(svg.matches("<tspan").count(), 2, "{svg}");
        let dy = dys(&svg);
        close(dy[0], 10.0);
        close(dy[1], 2.0 * 10.0 * 5.0 / 3.0);
        // The control: the same text with a line of Xs where the blank was.
        let control = render_one(&mtext("ALPHA\nXXXXX\nBRAVO", 1));
        assert_eq!(control.matches("<tspan").count(), 3, "{control}");
        let control_dy = dys(&control);
        close(control_dy[1] + control_dy[2], dy[1]);
        // A blank line is a line for the block height too, so the bottom
        // attachment lands both texts' last baseline in the same place.
        let blank = dys(&render_one(&mtext("ALPHA\n\nBRAVO", 7)));
        let filled = dys(&render_one(&mtext("ALPHA\nXXXXX\nBRAVO", 7)));
        close(blank[0], filled[0]);
        // Text that is nothing but blank lines draws nothing at all.
        assert_eq!(render_one(&mtext("\n\n", 1)), "");
    }

    fn ellipse(major: (f64, f64), ratio: f64, start: f64, end: f64) -> Entity {
        Entity::Ellipse(crate::model::EllipseEntity {
            common: EntityCommon {
                handle: "E".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            center: Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            major_axis_endpoint: Point3D {
                x: major.0,
                y: major.1,
                z: 0.0,
            },
            axis_ratio: ratio,
            start_angle: start,
            end_angle: end,
        })
    }

    #[test]
    fn an_ellipse_draws_only_its_stored_parameter_range() {
        let pi = std::f64::consts::PI;
        // Major axis (20, 0), ratio 0.5: the point at parameter t is
        // (20 cos t, 10 sin t). Parameters 0 .. pi is the upper half, from
        // (20, 0) to (-20, 0) -- SVG y down, so (20, 0) to (-20, 0) with
        // the arc bulging to negative y, which is sweep flag 0.
        let svg = render_one(&ellipse((20.0, 0.0), 0.5, 0.0, pi));
        assert_eq!(
            svg,
            "<path d=\"M 20 0 A 20 10 0 0 0 -20 0\" fill=\"none\" stroke=\"#000000\"/>"
        );
        // pi .. 2*pi is the lower half: (-20, 0) back to (20, 0).
        let svg = render_one(&ellipse((20.0, 0.0), 0.5, pi, 2.0 * pi));
        assert!(svg.contains("M -20 0 A 20 10 0 0 0 20 0"), "{svg}");
        // Over half a turn sets the large-arc flag: 0 .. 3*pi/2 ends at
        // (0, -10), i.e. SVG (0, 10).
        let svg = render_one(&ellipse((20.0, 0.0), 0.5, 0.0, 3.0 * pi / 2.0));
        assert!(svg.contains("M 20 0 A 20 10 0 1 0 0 10"), "{svg}");
        // 0 .. 2*pi is AutoCAD's closed ellipse, and an SVG arc whose ends
        // coincide draws nothing, so that one keeps the <ellipse> element.
        let svg = render_one(&ellipse((20.0, 0.0), 0.5, 0.0, 2.0 * pi));
        assert!(
            svg.starts_with("<ellipse cx=\"0\" cy=\"0\" rx=\"20\" ry=\"10\""),
            "{svg}"
        );
        // A major axis along +y turns the whole frame: the x-axis rotation
        // is 90 degrees counter-clockwise in the drawing, so -90 on the
        // y-down canvas, and 0 .. pi runs from (0, 20) to (0, -20).
        let svg = render_one(&ellipse((0.0, 20.0), 0.5, 0.0, pi));
        assert!(svg.contains("M 0 -20 A 20 10 -90 0 0 0 20"), "{svg}");
    }

    #[test]
    fn an_ellipse_arcs_extent_is_the_arcs_own() {
        // The upper half of the same ellipse reaches y 0..10, not -10..10,
        // and the crop must not be stretched to the half that is not drawn.
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        render_entity(
            &ellipse((20.0, 0.0), 0.5, 0.0, std::f64::consts::PI),
            &mut ctx,
        );
        let b = ctx.entity_box().expect("the arc was measured");
        close(b.min_x, -20.0);
        close(b.max_x, 20.0);
        close(b.min_y, 0.0);
        close(b.max_y, 10.0);
    }

    fn arc(start_angle: f64, end_angle: f64, radius: f64) -> Entity {
        Entity::Arc(crate::model::ArcEntity {
            common: EntityCommon {
                handle: "A".into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            center: Point3D {
                x: 5.0,
                y: 5.0,
                z: 0.0,
            },
            radius,
            start_angle,
            end_angle,
            extrusion: crate::geom::WORLD_Z,
        })
    }

    fn line_entity(handle: &str, from: (f64, f64), to: (f64, f64)) -> Entity {
        Entity::Line(crate::model::LineEntity {
            common: EntityCommon {
                handle: handle.into(),
                layer: "0".into(),
                ..EntityCommon::default()
            },
            start_point: Point3D {
                x: from.0,
                y: from.1,
                z: 0.0,
            },
            end_point: Point3D {
                x: to.0,
                y: to.1,
                z: 0.0,
            },
        })
    }

    fn drawing(entities: Vec<Entity>) -> CadDatabase {
        let mut tables = Tables::default();
        tables.block_records.insert(
            "*Model_Space".into(),
            crate::tables::BlockRecord {
                name: "*Model_Space".into(),
                entities: entities.clone(),
            },
        );
        CadDatabase::new(entities, tables)
    }

    #[test]
    fn an_arc_with_a_garbage_angle_is_left_undrawn_and_the_render_finishes() {
        // 1e20 is the hand-written repro's DXF group 50; 1.4052120271735542e247
        // with radius 0 is what a fuzzed `entities-2d.dwg` stored. Both used
        // to spin `BulgeArc::bounds` forever, so `to_svg` (and `to_png`, and
        // `export_package` through it) never returned. This test hangs
        // rather than fails without the fix.
        let db = drawing(vec![
            line_entity("L", (0.0, 0.0), (10.0, 10.0)),
            arc(1.0e20, 1.0, 2.0),
            arc(1.4052120271735542e247, 1.0, 0.0),
            arc(f64::NAN, f64::INFINITY, 2.0),
        ]);
        let result = to_svg(&db, ToSvgOptions::default());
        // The LINE is still drawn; the three unreadable arcs are not.
        assert!(result.svg.contains("<line "), "{}", result.svg);
        assert!(!result.svg.contains("<path"), "{}", result.svg);
        assert!(
            crate::png::to_png(&db, crate::png::ToPngOptions::default()).is_ok(),
            "to_png must finish too"
        );
        // A real arc of the same shape is unaffected: 0 .. pi/2 about
        // (5,5) r 2 runs from (7,5) to (5,7), i.e. SVG (7,-5) to (5,-7).
        let db = drawing(vec![arc(0.0, std::f64::consts::FRAC_PI_2, 2.0)]);
        let svg = to_svg(&db, ToSvgOptions::default()).svg;
        assert!(svg.contains("M 7 -5 A 2 2 0 0 0 5 -7"), "{svg}");
    }

    #[test]
    fn a_non_finite_coordinate_never_reaches_an_svg_attribute() {
        // Rust's Display writes these as `NaN` and `inf`, neither of which
        // is in SVG's <number> grammar, so the attribute -- and for a
        // conforming consumer the element -- is in error.
        let nan = f64::NAN;
        let inf = f64::INFINITY;
        let mut tables = Tables::default();
        tables.block_records.insert(
            "B".into(),
            crate::tables::BlockRecord {
                name: "B".into(),
                entities: vec![line_entity("BL", (0.0, 0.0), (10.0, 10.0))],
            },
        );
        let entities = vec![
            line_entity("L0", (0.0, 0.0), (10.0, 10.0)),
            line_entity("L1", (nan, nan), (10.0, 10.0)),
            line_entity("L2", (0.0, 0.0), (inf, -inf)),
            Entity::Circle(crate::model::CircleEntity {
                common: EntityCommon {
                    handle: "C".into(),
                    layer: "0".into(),
                    ..EntityCommon::default()
                },
                center: Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                radius: inf,
                extrusion: crate::geom::WORLD_Z,
            }),
            ellipse((nan, 0.0), 0.5, 0.0, 1.0),
            Entity::Insert(crate::model::InsertEntity {
                common: EntityCommon {
                    handle: "I".into(),
                    layer: "0".into(),
                    ..EntityCommon::default()
                },
                block_name: "B".into(),
                insertion_point: Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Point3D {
                    x: nan,
                    y: inf,
                    z: 1.0,
                },
                rotation: 0.0,
                extrusion: crate::geom::WORLD_Z,
                attribs: Vec::new(),
            }),
        ];
        tables.block_records.insert(
            "*Model_Space".into(),
            crate::tables::BlockRecord {
                name: "*Model_Space".into(),
                entities: entities.clone(),
            },
        );
        let db = CadDatabase::new(entities, tables);
        let svg = to_svg(
            &db,
            ToSvgOptions {
                crop: CropMode::Raw,
                ..Default::default()
            },
        )
        .svg;
        assert!(!svg.contains("NaN"), "{svg}");
        assert!(!svg.contains("inf"), "{svg}");
        // Only the one sound LINE survives.
        assert_eq!(svg.matches("<line ").count(), 1, "{svg}");
        assert!(!svg.contains("<circle"), "{svg}");
        assert!(!svg.contains("<ellipse"), "{svg}");
        assert!(!svg.contains("<g "), "{svg}");
        // A scale of 1e-300 is finite but prints as 300 characters of
        // leading zeros, which is what `clean` exists to prevent.
        assert!(svg.len() < 600, "{svg}");
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
