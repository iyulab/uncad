//! The rendering-only entity model, read from a parsed drawing and consumed
//! by [`crate::svg::to_svg`]. **Not a general-purpose CAD IR** and not a
//! round-trip representation -- it only keeps the fields `to_svg()` needs.
//! `CadDatabase::write_dwg`/`write_dxf` never go through this model; they
//! write the original `Dwg_Data` LibreDWG parsed, which is the crate's
//! actual complete/round-trip-faithful representation (see
//! `docs/ARCHITECTURE.md`'s "two-layer model" section for the full
//! rationale).
//!
//! Mirrors the JS/WASM-era predecessor's
//! `bindings/javascript/src/database/entities/*.ts` shape (see git history
//! for that predecessor; the fork it lived in is no longer vendored): one
//! `EntityCommon` (base `RenderEntity` interface fields) embedded
//! in each variant of the `RenderEntity` enum, rather than one giant struct
//! with 100+ `Option<T>` fields -- see the plan for why (exhaustiveness
//! checking on the eventual `toSVG`-equivalent match, mainly).
//!
//! See `docs/CAVEATS.md` for current entity-type coverage (which types
//! are supported, and which of those are JS-baseline ports vs. new
//! functionality beyond it).

use crate::dynapi::{Point2D, Point3D};

/// Fields common to every DWG entity, regardless of type.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCommon {
    /// Hex handle string, matching the JS CadDatabase's `.handle` shape
    /// (e.g. `"2A"`), used for cross-referencing (3DSOLID wireframe
    /// attachment, INSERT/DIMENSION block lookups, ...).
    pub handle: String,
    /// Owning layer's name, resolved from the entity's `layer` handle field
    /// via `dwg_dynapi_handle_name`. Empty string if unresolvable.
    pub layer: String,
    /// Raw ACI color index straight from `Dwg_Color.index`: negative means
    /// "layer off" (color, not visibility, is tracked here -- matches
    /// `toSVG()`'s existing `Math.abs(idx)` treatment), 0 is BYBLOCK, 256
    /// is BYLAYER, anything else is a direct palette index.
    pub color_index: i16,
    /// Explicit 24-bit truecolor override, present only when
    /// `Dwg_Color.method == DWG_COLOR_METHOD_TRUECOLOR` -- mirrors the JS
    /// model's `e.color` being `undefined` unless the entity genuinely
    /// opted into a direct RGB color (as opposed to `Dwg_Color.rgb` simply
    /// holding a stale/default value, which it always does regardless of
    /// method).
    pub true_color: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineEntity {
    pub common: EntityCommon,
    pub start_point: Point3D,
    pub end_point: Point3D,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CircleEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub text: String,
    /// Radians (DXF 50) -- unlike MTEXT's `x_axis_dir`-derived rotation
    /// (see [`MTextEntity::rotation`]), TEXT stores this as a plain,
    /// unambiguous angle.
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LwPolylineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point2D>,
    /// LWPOLYLINE.flag bit 1: closed polyline (see convert.rs's
    /// `LWPOLYLINE_CLOSED_FLAG` for why not the 512 dwg.h's own comment
    /// documents).
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArcEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub radius: f64,
    /// Radians.
    pub start_angle: f64,
    /// Radians.
    pub end_angle: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EllipseEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    /// Major-axis endpoint, relative to `center` (DXF 11).
    pub major_axis_endpoint: Point3D,
    /// Minor/major radius ratio (DXF 40).
    pub axis_ratio: f64,
    pub start_angle: f64,
    pub end_angle: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointEntity {
    pub common: EntityCommon,
    pub position: Point3D,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolidEntity {
    pub common: EntityCommon,
    /// Raw DXF corner order (1-2-3-4) -- AutoCAD's classic 1-2-4-3
    /// triangle-strip reordering for filled rendering is a `to_svg()`
    /// rendering concern (see svg.rs), not part of the parsed data model.
    pub corner1: Point2D,
    pub corner2: Point2D,
    pub corner3: Point2D,
    pub corner4: Point2D,
}

/// Shared by RAY and XLINE -- LibreDWG itself uses one C struct
/// (`Dwg_Entity_RAY`, `typedef`'d as `Dwg_Entity_XLINE`) for both; they only
/// differ in the DXF/dynapi type name and in `toSVG()`'s eventual rendering
/// (RAY extends one direction from `point`, XLINE extends both).
#[derive(Debug, Clone, PartialEq)]
pub struct RayEntity {
    pub common: EntityCommon,
    pub point: Point3D,
    pub vector: Point3D,
}

/// ATTRIB shares TEXT's exact field shape (position/height/text value) --
/// it's a block-attribute value attached to an INSERT, but geometrically
/// it's just another piece of text.
#[derive(Debug, Clone, PartialEq)]
pub struct AttribEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub text: String,
    /// Radians (DXF 50), same field shape as [`TextEntity::rotation`].
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertEntity {
    pub common: EntityCommon,
    /// Referenced block's name, resolved from the `block_header` handle
    /// field -- callers look this up in
    /// [`crate::tables::Tables::block_records`] to actually render (see
    /// `render_block_ref` in svg.rs), not stored here.
    pub block_name: String,
    pub insertion_point: Point3D,
    /// Per-axis scale factors (DXF 41/42/43); (1,1,1) if never set.
    pub scale: Point3D,
    /// Radians.
    pub rotation: f64,
    /// Attribute values attached to this INSERT (DXF 0 ATTRIB records
    /// between the INSERT and its SEQEND). Also duplicated as top-level
    /// [`RenderEntity::Attrib`] entries in `CadDatabase::entities`, matching
    /// the JS baseline's `db.entities.push(attrib)` -- see convert.rs.
    pub attribs: Vec<AttribEntity>,
}

/// TOLERANCE (a GD&T feature control frame) -- **new functionality, not a
/// JS-baseline port** (the JS baseline parsed but never rendered it; see
/// `docs/CAVEATS.md`). Rendered as plain text at `insertion_point`, same
/// as ATTRIB/TEXT -- `text_value` keeps its raw DXF feature-control-frame
/// formatting codes (e.g. `%%v`-style symbol escapes) unstripped, so this
/// is a readable-but-unfaithful approximation, not real GD&T symbol
/// rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct ToleranceEntity {
    pub common: EntityCommon,
    pub insertion_point: Point3D,
    pub text_height: f64,
    pub text_value: String,
}

/// ACAD_TABLE -- **new functionality, not a JS-baseline port** (see
/// `docs/CAVEATS.md`). Same field shape as [`InsertEntity`] minus
/// `attribs` (ACAD_TABLE has no attribute chain): `dwg.h`'s
/// `Dwg_Entity_TABLE` has its own `ins_pt`/`scale`/`rotation`/
/// `block_header` fields, identical in spirit to INSERT's, and (per its
/// own `flag_for_table_value` field comment) "normally always" carries a
/// cached-geometry block reference the same way INSERT/DIMENSION do --
/// rendered through the same `render_block_ref` machinery, not table-cell
/// content reconstructed from `num_cols`/`num_rows`/`col_widths`/etc
/// (which this project doesn't even parse).
#[derive(Debug, Clone, PartialEq)]
pub struct AcadTableEntity {
    pub common: EntityCommon,
    pub block_name: String,
    pub insertion_point: Point3D,
    pub scale: Point3D,
    /// Radians.
    pub rotation: f64,
}

/// Same field shape as ATTRIB -- the *template* stored in a block
/// definition (vs. ATTRIB, the value instance attached to an INSERT).
#[derive(Debug, Clone, PartialEq)]
pub struct AttdefEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub default_value: String,
    /// Radians (DXF 50), same field shape as [`AttribEntity::rotation`].
    /// Unused by `to_svg()` (ATTDEF is a template, never drawn directly --
    /// see its `RenderEntity::Attdef` match arm in svg.rs), kept for parity
    /// with ATTRIB and for callers using `entities` outside `to_svg()`.
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewportEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face3DEntity {
    pub common: EntityCommon,
    /// Already-sequential order (unlike SOLID, no 1-2-4-3 reorder needed).
    pub corner1: Point3D,
    pub corner2: Point3D,
    pub corner3: Point3D,
    pub corner4: Point3D,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplineEntity {
    pub common: EntityCommon,
    /// Points that lie exactly on the curve -- preferred over
    /// `control_points` when present (matches `toSVG()`'s straight-line
    /// approximation preference, see svg.rs).
    pub fit_points: Vec<Point3D>,
    pub control_points: Vec<Point3D>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MTextEntity {
    pub common: EntityCommon,
    pub insertion_point: Point3D,
    pub text: String,
    pub text_height: f64,
    /// Radians, derived from `x_axis_dir` (DWG stores MTEXT's rotation as a
    /// direction vector, not an angle).
    pub rotation: f64,
    pub line_spacing_factor: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolylineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point3D>,
    pub closed: bool,
}

/// One edge of a non-polyline HATCH boundary path -- mirrors
/// `hatchEdgePoints`'s 4 cases in the JS baseline exactly (chord/segment
/// approximation, not real curve evaluation, matching the same
/// simplification used for SPLINE/curved 3DSOLID edges elsewhere).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HatchEdge {
    Line {
        start: Point2D,
    },
    Arc {
        center: Point2D,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Ellipse {
        center: Point2D,
        /// Major-axis endpoint, relative to `center`.
        end: Point2D,
        /// Minor/major axis length ratio.
        minor_major_ratio: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Spline {
        control_points: Vec<Point2D>,
    },
}

/// One HATCH boundary path -- either an explicit polyline (vertices only;
/// bulge/arc segments are dropped, matching the JS baseline's own
/// simplification -- `toSVG()` never reconstructs the arcs a bulge would
/// imply) or a list of curved/straight edges.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HatchBoundaryPath {
    Polyline(Vec<Point2D>),
    Edges(Vec<HatchEdge>),
}

/// One pattern-fill "definition line" (`Dwg_HATCH_DefLine`): a family of
/// parallel lines used to actually hatch a non-solid-fill boundary, rather
/// than just outlining it. `angle`/`base_point`/`offset`/`dash_pattern` are
/// used as-is, in the HATCH's own local coordinate space -- *not* further
/// transformed by the HATCH's own `angle`/`scale_spacing` fields (DXF
/// 52/41), even though standard DXF pattern-fill documentation describes
/// those as applying on top of the definition-line data. That was tried
/// first and confirmed wrong on a real file: LibreDWG's defline data
/// already has 52/41 baked in (a HATCH with a 90-degree pattern angle had a
/// defline whose own `angle` was independently also exactly 90 degrees; a
/// pattern_scale of 60 applied on top inflated a ~6.5-unit spacing to
/// ~390 units, many times larger than the shape it was supposed to fill,
/// rendering no visible lines at all) -- see `svg.rs`'s
/// `render_hatch_pattern_line` doc comment for the full account.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    /// Radians.
    pub angle: f64,
    /// A point that lies on one of the family's lines.
    pub base_point: Point2D,
    /// Displacement from one line to the next. Only the component
    /// perpendicular to `angle` (the spacing) is honored at render time --
    /// a nonzero component parallel to `angle` (used by real DXF patterns
    /// for brick/masonry-style staggering) is a deliberate simplification,
    /// not reproduced; see `svg.rs`.
    pub offset: Point2D,
    /// Dash lengths along each line: positive = drawn, negative = gap,
    /// empty = continuous (no dashing).
    pub dash_pattern: Vec<f64>,
}

/// A HATCH gradient fill (`is_gradient_fill`), reduced to the two colors and
/// shape SVG's own gradient elements need. `gradient_name` (`SPHERICAL`/
/// `HEMISPHERICAL`/`CURVED`/`LINEAR`/`CYLINDER`, per `dwg.h`'s comment on
/// `Dwg_Entity_HATCH.gradient_name`) collapses to a binary choice at render
/// time: `SPHERICAL`/`HEMISPHERICAL` become an SVG `radialGradient`
/// (matches their AutoCAD appearance -- a center-out glow), everything else
/// (`LINEAR`/`CYLINDER`/`CURVED`, and any unrecognized name) becomes a
/// `linearGradient` -- `CURVED`/`CYLINDER` are directional-but-not-radial in
/// AutoCAD too, so a straight linear approximation is closer than a radial
/// one even though it drops the curvature. `gradient_shift` (DXF 461,
/// AutoCAD's "centered" option when nonzero) is deliberately not applied --
/// same category of simplification as `HatchPatternLine.offset`'s
/// parallel-to-`angle` component, see its doc comment.
///
/// **Unverified**: none of this project's 9 real-file spot checks contain a
/// gradient-fill HATCH (`is_gradient_fill` was `0` on every one of the
/// ~2500 HATCH entities across all 9 files -- see `docs/CAVEATS.md`), so
/// this is built from `dwg.h`'s field comments and general DXF-gradient
/// knowledge, not confirmed against a real AutoCAD rendering the way most
/// of this project's other logic is.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchGradient {
    pub is_radial: bool,
    /// Radians.
    pub angle: f64,
    pub color1: String,
    pub color2: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchEntity {
    pub common: EntityCommon,
    pub boundary_paths: Vec<HatchBoundaryPath>,
    pub solid_fill: bool,
    /// `Some` when `is_gradient_fill` is set -- takes priority over
    /// `pattern_lines` at render time (a gradient-fill HATCH's `deflines`
    /// data isn't meaningful pattern geometry).
    pub gradient: Option<HatchGradient>,
    /// Empty for solid fills (irrelevant -- solid fill ignores pattern
    /// data), for gradient fills (`gradient` is used instead), or when the
    /// file's pattern data couldn't be read. When non-empty, `svg.rs`
    /// renders the actual hatch pattern (tiled parallel lines clipped to
    /// `boundary_paths` via SVG's own fill mechanism) instead of the
    /// outline-only fallback.
    pub pattern_lines: Vec<HatchPatternLine>,
}

/// AutoCAD caches a DIMENSION's rendered geometry (lines, arrows, text) as
/// an anonymous block already in final/world coordinates -- rendering it is
/// just resolving `block_name` and drawing that block's entities with an
/// identity transform (see `render_block_ref` in svg.rs), same mechanism as
/// INSERT. All 7 DWG dimension subtypes (ALIGNED, ANG2LN, ANG3PT, DIAMETER,
/// LINEAR, ORDINATE, ARC_DIMENSION) share this one shape and fold into a
/// single `RenderEntity::Dimension` -- the first 6 matching the JS baseline's
/// own `"DIMENSION"` type; ARC_DIMENSION is the one subtype the JS
/// baseline never recognized (unlike MULTILEADER/MLINE this isn't new
/// functionality though, just extending an already-ported mechanism to a
/// 7th case using its own `block` field the same way).
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionEntity {
    pub common: EntityCommon,
    pub block_name: String,
}

/// Best-effort wireframe extraction for a 3DSOLID's ACIS B-rep data -- see
/// `acis.rs`. A 3DSOLID's real shape is a B-rep blob with no directly
/// 2D-renderable geometry; `wireframe_edges` are straight chords between
/// each ACIS `edge` record's two resolved endpoint vertices, in the
/// solid's own local 3D coordinate space (no projection applied here --
/// that's a render-time concern, see svg.rs's fixed isometric projection).
/// Empty when the wireframe couldn't be extracted (empty solid,
/// unrecognized ACIS version, conversion failure) -- callers should treat
/// that the same as an unsupported entity type.
#[derive(Debug, Clone, PartialEq)]
pub struct Solid3DEntity {
    pub common: EntityCommon,
    pub wireframe_edges: Vec<[Point3D; 2]>,
}

/// MULTILEADER's leader-line geometry only -- **new functionality, not a
/// JS-baseline port** (the JS predecessor's `toSVG()` never rendered
/// MULTILEADER at all, see `docs/CAVEATS.md`). Deliberately narrow scope,
/// matching this project's existing "best-effort approximation, not full
/// fidelity" precedent for other structurally complex types (3DSOLID's
/// wireframe-only rendering, VIEWPORT's frame-only rendering): just the
/// leader lines/splines connecting the content to its landing point, as
/// straight-line polylines (spline leaders are chord-approximated, same
/// simplification used for SPLINE/HATCH curved edges elsewhere). MLEADER
/// text/block content itself (`ctx.content`) is not extracted or drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderEntity {
    pub common: EntityCommon,
    pub lines: Vec<Vec<Point3D>>,
}

/// One MLINE vertex: the centerline `point`, plus `miter_direction` -- a
/// vector already accounting for the miter angle at this vertex, such that
/// `point + miter_direction * offset` is that vertex's position on the
/// parallel line `offset` away from the centerline (see
/// `convert::convert_mline`'s doc comment).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MLineVertex {
    pub point: Point3D,
    pub miter_direction: Point3D,
}

/// An MLINE is really `num_lines` parallel offset lines (wall-style
/// multi-line) -- **new functionality, not a JS-baseline port**, same as
/// [`MultiLeaderEntity`] (see `docs/CAVEATS.md`). The real per-line offset
/// distances live in the MLINE's referenced MLINESTYLE object
/// (`mlinestyle_name`, resolved against `Tables::mlinestyles` at render
/// time -- same deferred-to-render-time pattern as BYLAYER color
/// resolution, see `EntityCommon::layer`'s doc comment) -- when that lookup
/// succeeds, `svg.rs` draws one polyline per style line using each vertex's
/// `miter_direction`; when it fails (style not found, e.g. an `.mlinestyle`
/// handle LibreDWG couldn't resolve), it falls back to a single centerline
/// through each vertex's `point`, same as this type's previous
/// centerline-only behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct MLineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<MLineVertex>,
    pub closed: bool,
    pub mlinestyle_name: String,
}

/// WIPEOUT's clip boundary, already resolved to world-relative (well,
/// local-to-whatever-block-owns-it, like every other entity's stored
/// geometry -- see svg.rs's `ctx.transform` for how block nesting composes
/// on top) 2D points -- **new functionality, not a JS-baseline port** (see
/// `docs/CAVEATS.md`). Unlike MULTILEADER/MLINE/REGION/POLYLINE_PFACE,
/// this one carries an *extra* layer of risk beyond "unverified against a
/// real file": the `pt0 + u*uvec + v*vvec` pixel-space-to-world transform
/// used to compute `boundary` is standard DXF-format knowledge (image
/// entities' insertion point + U/V pixel vectors + clip vertices in that
/// pixel space), not something confirmed by reading LibreDWG's own source
/// (LibreDWG only reads/writes the raw fields; it doesn't render images,
/// so there's no reference implementation in this codebase to check
/// against either) -- see `convert.rs`'s `wipeout_boundary` for the exact
/// computation.
#[derive(Debug, Clone, PartialEq)]
pub struct WipeoutEntity {
    pub common: EntityCommon,
    pub boundary: Vec<Point2D>,
}

/// LIGHT -- **new functionality, not a JS-baseline port** (the JS baseline
/// never parsed or rendered it at all, unlike TOLERANCE/ACAD_TABLE/WIPEOUT
/// which it at least parsed; see `docs/CAVEATS.md`). A light source has no
/// natural drawable shape, so this is rendered as a small marker at
/// `position` plus a dashed line to `target` when the light actually has a
/// directional target (`target_is_meaningful` -- distant/spot lights,
/// `type` 1/3; point lights, `type` 2, don't aim anywhere) -- an honest
/// "something is better than nothing" placeholder, same spirit as
/// VIEWPORT's frame-only rendering, not a claim that this is what LIGHT
/// looks like in AutoCAD (it isn't -- lights aren't visible at all in a
/// normal 2D plan view).
#[derive(Debug, Clone, PartialEq)]
pub struct LightEntity {
    pub common: EntityCommon,
    pub position: Point3D,
    pub target: Point3D,
    pub target_is_meaningful: bool,
}

/// Simple (non-MULTILEADER) LEADER: a polyline of `vertices` plus an
/// optional arrowhead at the first vertex -- only the fields the JS
/// baseline's `renderEntity` actually draws (styleName/isSpline/text box
/// size/etc are parsed by the JS model but never reach the SVG, so they're
/// left out here rather than ported unused).
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point3D>,
    pub is_arrowhead_enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RenderEntity {
    Line(LineEntity),
    Circle(CircleEntity),
    Text(TextEntity),
    LwPolyline(LwPolylineEntity),
    Arc(ArcEntity),
    Ellipse(EllipseEntity),
    Point(PointEntity),
    Solid(SolidEntity),
    Ray(RayEntity),
    XLine(RayEntity),
    Insert(InsertEntity),
    Attrib(AttribEntity),
    Attdef(AttdefEntity),
    Viewport(ViewportEntity),
    Face3D(Face3DEntity),
    Spline(SplineEntity),
    MText(MTextEntity),
    Polyline3D(PolylineEntity),
    Dimension(DimensionEntity),
    Hatch(HatchEntity),
    Solid3D(Solid3DEntity),
    Leader(LeaderEntity),
    MultiLeader(MultiLeaderEntity),
    MLine(MLineEntity),
    /// REGION -- **new functionality, not a JS-baseline port** (the JS
    /// baseline never handled REGION at all; see `docs/CAVEATS.md`).
    /// Reuses [`Solid3DEntity`]'s exact shape (same ACIS
    /// wireframe-extraction mechanism -- dwg.h's `Dwg_Entity_REGION` is a
    /// typedef of `Dwg_Entity__3DSOLID`, byte-for-byte the same struct) as
    /// a distinct variant, matching how [`RenderEntity::XLine`] reuses
    /// [`RayEntity`] and [`RenderEntity::Polyline2D`] reuses
    /// [`LwPolylineEntity`] -- kept separate so `type_name()` still
    /// reports the real "REGION" dxfname.
    Region(Solid3DEntity),
    /// POLYLINE_PFACE ("polyface mesh") -- **new functionality, not a
    /// JS-baseline port** (the JS baseline never handled it; see
    /// `docs/CAVEATS.md`). LibreDWG's own dedicated accessor for this type
    /// (`dwg_ent_polyline_pface_get_points`) is documented `/* not
    /// implemented. use the dynapi instead */` in `dwg_api.h`, so unlike
    /// POLYLINE_3D this reads the raw `VERTEX_PFACE`/`VERTEX_PFACE_FACE`
    /// owned-subentity chain directly (see `convert.rs`) and resolves each
    /// face's up-to-4 vertex indices into wireframe edges -- reusing
    /// [`Solid3DEntity`]'s shape and `svg.rs`'s isometric wireframe
    /// renderer, same as [`RenderEntity::Region`], since a polyface mesh is
    /// just as inherently 3D as an ACIS solid's wireframe.
    PolylinePFace(Solid3DEntity),
    /// Same shape as [`RenderEntity::LwPolyline`] (2D vertices + closed flag,
    /// same flag-bit convention) -- reuses [`LwPolylineEntity`] rather than
    /// a near-duplicate struct, matching how [`RenderEntity::XLine`] already
    /// reuses [`RayEntity`]. Kept as a distinct variant (not folded into
    /// `LwPolyline`) so `type_name()`/CLI summaries still report the real
    /// DXF type, matching the JS baseline's `entityConverter.ts` (which
    /// also keeps `POLYLINE2D` a separate `type:` from `LWPOLYLINE`, even
    /// though `toSVG()` renders both through one shared `case`).
    Polyline2D(LwPolylineEntity),
    Tolerance(ToleranceEntity),
    AcadTable(AcadTableEntity),
    Wipeout(WipeoutEntity),
    Light(LightEntity),
    /// Any entity type not yet ported. Carries the real DXF type name (from
    /// `dwg_object_get_dxfname`) so callers can still count/report by type
    /// -- see convert.rs's module doc for how this differs from (and is a
    /// superset of) the JS baseline's silent-drop behavior.
    Unknown {
        common: EntityCommon,
        type_name: String,
    },
}

impl RenderEntity {
    pub fn common(&self) -> &EntityCommon {
        match self {
            RenderEntity::Line(e) => &e.common,
            RenderEntity::Circle(e) => &e.common,
            RenderEntity::Text(e) => &e.common,
            RenderEntity::LwPolyline(e) => &e.common,
            RenderEntity::Arc(e) => &e.common,
            RenderEntity::Ellipse(e) => &e.common,
            RenderEntity::Point(e) => &e.common,
            RenderEntity::Solid(e) => &e.common,
            RenderEntity::Ray(e) => &e.common,
            RenderEntity::XLine(e) => &e.common,
            RenderEntity::Insert(e) => &e.common,
            RenderEntity::Attrib(e) => &e.common,
            RenderEntity::Attdef(e) => &e.common,
            RenderEntity::Viewport(e) => &e.common,
            RenderEntity::Face3D(e) => &e.common,
            RenderEntity::Spline(e) => &e.common,
            RenderEntity::MText(e) => &e.common,
            RenderEntity::Polyline3D(e) => &e.common,
            RenderEntity::Dimension(e) => &e.common,
            RenderEntity::Hatch(e) => &e.common,
            RenderEntity::Solid3D(e) => &e.common,
            RenderEntity::Leader(e) => &e.common,
            RenderEntity::MultiLeader(e) => &e.common,
            RenderEntity::MLine(e) => &e.common,
            RenderEntity::Region(e) => &e.common,
            RenderEntity::PolylinePFace(e) => &e.common,
            RenderEntity::Polyline2D(e) => &e.common,
            RenderEntity::Tolerance(e) => &e.common,
            RenderEntity::AcadTable(e) => &e.common,
            RenderEntity::Wipeout(e) => &e.common,
            RenderEntity::Light(e) => &e.common,
            RenderEntity::Unknown { common, .. } => common,
        }
    }

    /// The DXF/entity type name, e.g. `"LINE"`.
    pub fn type_name(&self) -> &str {
        match self {
            RenderEntity::Line(_) => "LINE",
            RenderEntity::Circle(_) => "CIRCLE",
            RenderEntity::Text(_) => "TEXT",
            RenderEntity::LwPolyline(_) => "LWPOLYLINE",
            RenderEntity::Arc(_) => "ARC",
            RenderEntity::Ellipse(_) => "ELLIPSE",
            RenderEntity::Point(_) => "POINT",
            RenderEntity::Solid(_) => "SOLID",
            RenderEntity::Ray(_) => "RAY",
            RenderEntity::XLine(_) => "XLINE",
            RenderEntity::Insert(_) => "INSERT",
            RenderEntity::Attrib(_) => "ATTRIB",
            RenderEntity::Attdef(_) => "ATTDEF",
            RenderEntity::Viewport(_) => "VIEWPORT",
            RenderEntity::Face3D(_) => "3DFACE",
            RenderEntity::Spline(_) => "SPLINE",
            RenderEntity::MText(_) => "MTEXT",
            RenderEntity::Polyline3D(_) => "POLYLINE3D",
            RenderEntity::Dimension(_) => "DIMENSION",
            RenderEntity::Hatch(_) => "HATCH",
            RenderEntity::Solid3D(_) => "3DSOLID",
            RenderEntity::Leader(_) => "LEADER",
            RenderEntity::MultiLeader(_) => "MULTILEADER",
            RenderEntity::MLine(_) => "MLINE",
            RenderEntity::Region(_) => "REGION",
            RenderEntity::PolylinePFace(_) => "POLYLINE_PFACE",
            RenderEntity::Polyline2D(_) => "POLYLINE_2D",
            RenderEntity::Tolerance(_) => "TOLERANCE",
            RenderEntity::AcadTable(_) => "ACAD_TABLE",
            RenderEntity::Wipeout(_) => "WIPEOUT",
            RenderEntity::Light(_) => "LIGHT",
            RenderEntity::Unknown { type_name, .. } => type_name,
        }
    }
}
