//! The drawing model this crate exposes: built by `crate::convert` from a
//! parsed file, rendered by `crate::svg` and serialized verbatim by
//! [`CadDatabase::to_json`](crate::CadDatabase::to_json).
//!
//! **Not a general-purpose CAD IR** and not a round-trip representation --
//! it keeps the fields rendering needs (see `docs/ARCHITECTURE.md`, "Model").
//! Every type here derives `serde::Serialize`/`Deserialize`; [`Entity`] is
//! internally tagged with `"type"` using the same DXF names
//! [`Entity::type_name`] reports, so JSON consumers can dispatch on `type`
//! without knowing the Rust enum (see [`crate::json`]).
//!
//! Several entity types are rendered as deliberate approximations (curves as
//! chords, 3D solids as wireframes, ...) and some have never been exercised
//! by a real drawing -- `docs/CAVEATS.md` tracks which is which.

// Re-exported, not merely imported: these are the declared types of public
// fields below (`LineEntity::start_point` and friends), but `dynapi` is a
// private module, so without this a caller can read `line.start_point.x` and
// still have no way to *name* the type it just read.
pub use crate::dynapi::{Point2D, Point3D};

use serde::{Deserialize, Serialize};

/// A value this model reached through a handle in the source file -- a layer
/// name, a block name, a style name.
///
/// Three states rather than an `Option`, because two different things used
/// to collapse into one empty string: a field the file carries no handle for
/// at all (normal for some fields), and a handle that nothing in the drawing
/// answers to (always a defect in the file or in the read). The unresolved
/// case keeps the handle: it is the only thing that tells one missing table
/// row (many entities point at the same dead handle) from references broken
/// wholesale (every entity points somewhere different).
///
/// Serialized adjacently tagged, like [`HatchBoundaryPath`]:
/// `{"type":"RESOLVED","data":"0"}`, `{"type":"ABSENT"}`,
/// `{"type":"UNRESOLVED","data":"2A"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "UPPERCASE")]
pub enum Ref<T> {
    /// The handle resolved. `T` is what it resolved to (a name, today).
    Resolved(T),
    /// The file carries no handle for this field.
    Absent,
    /// The file carries a handle, but nothing in the drawing answers to it.
    /// The value is the handle as a hex string, the same form as
    /// [`EntityCommon::handle`].
    Unresolved(String),
}

impl<T> Ref<T> {
    /// The resolved value, if there is one.
    pub fn resolved(&self) -> Option<&T> {
        match self {
            Ref::Resolved(value) => Some(value),
            Ref::Absent | Ref::Unresolved(_) => None,
        }
    }

    /// `true` for [`Ref::Resolved`].
    pub fn is_resolved(&self) -> bool {
        matches!(self, Ref::Resolved(_))
    }
}

impl Ref<String> {
    /// The resolved name as a `&str`, or `""` when there is none. For
    /// consumers that only need a lookup key and treat "no name" and "no such
    /// name" alike; anything that must tell them apart matches on the enum.
    pub fn name(&self) -> &str {
        self.resolved().map_or("", String::as_str)
    }
}

/// Fields common to every DWG entity, regardless of type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityCommon {
    /// Hex handle string (e.g. `"2A"`), used for cross-referencing
    /// (3DSOLID wireframe attachment, INSERT/DIMENSION block lookups, ...).
    pub handle: String,
    /// Owning layer's name, resolved from the entity's `layer` handle field.
    /// [`Ref::Unresolved`] when the handle points at nothing (the R2007+ DXF
    /// failure looked exactly like that before it was refused up front);
    /// never an empty string standing in for "could not read".
    pub layer: Ref<String>,
    /// Raw ACI color index straight from `Dwg_Color.index`: negative means
    /// "layer off" (this tracks color, not visibility, so the sign is
    /// ignored at render time), 0 is BYBLOCK, 256 is BYLAYER, anything else
    /// is a direct palette index.
    pub color_index: i16,
    /// Explicit 24-bit truecolor override, present only when
    /// `Dwg_Color.method == DWG_COLOR_METHOD_TRUECOLOR`. `Dwg_Color.rgb`
    /// always holds *something*, so the method is what decides whether the
    /// entity really opted into a direct RGB color.
    pub true_color: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineEntity {
    pub common: EntityCommon,
    pub start_point: Point3D,
    pub end_point: Point3D,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CircleEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub text: String,
    /// Radians (DXF 50). TEXT stores this as a plain angle, unlike MTEXT,
    /// whose rotation is a direction vector (see [`MTextEntity::rotation`]).
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LwPolylineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point2D>,
    /// LWPOLYLINE.flag bit 1 -- see `convert.rs`'s `POLYLINE_CLOSED_FLAG`
    /// for why not the 512 dwg.h's own comment documents.
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArcEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub radius: f64,
    /// Radians.
    pub start_angle: f64,
    /// Radians.
    pub end_angle: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointEntity {
    pub common: EntityCommon,
    pub position: Point3D,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidEntity {
    pub common: EntityCommon,
    /// Raw DXF corner order (1-2-3-4). AutoCAD's classic 1-2-4-3
    /// triangle-strip reordering for filled rendering is a rendering
    /// concern (see `svg.rs`), not part of the parsed data.
    pub corner1: Point2D,
    pub corner2: Point2D,
    pub corner3: Point2D,
    pub corner4: Point2D,
}

/// Shared by RAY and XLINE -- LibreDWG itself uses one C struct
/// (`Dwg_Entity_RAY`, `typedef`'d as `Dwg_Entity_XLINE`) for both; they only
/// differ in the DXF type name and in rendering (RAY extends one direction
/// from `point`, XLINE extends both).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RayEntity {
    pub common: EntityCommon,
    pub point: Point3D,
    pub vector: Point3D,
}

/// ATTRIB shares TEXT's exact field shape: it is a block-attribute value
/// attached to an INSERT, but geometrically just another piece of text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttribEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub text: String,
    /// Radians (DXF 50).
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertEntity {
    pub common: EntityCommon,
    /// Referenced block's name. Callers look it up in
    /// [`crate::tables::Tables::block_records`] to render the block's own
    /// entities (see `render_block_ref` in `svg.rs`).
    pub block_name: Ref<String>,
    pub insertion_point: Point3D,
    /// Per-axis scale factors (DXF 41/42/43); (1,1,1) if never set.
    pub scale: Point3D,
    /// Radians.
    pub rotation: f64,
    /// Attribute values attached to this INSERT (the ATTRIB records between
    /// the INSERT and its SEQEND). Also present as top-level
    /// [`Entity::Attrib`] entries in `CadDatabase::entities` -- that is what
    /// rendering actually draws; see `convert.rs`.
    pub attribs: Vec<AttribEntity>,
}

/// TOLERANCE, a GD&T feature control frame. Rendered as plain text at
/// `insertion_point`: `text_value` keeps its raw DXF feature-control-frame
/// codes (e.g. `%%v`-style symbol escapes) unstripped, so this is a
/// readable-but-unfaithful approximation, not real GD&T symbol rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToleranceEntity {
    pub common: EntityCommon,
    pub insertion_point: Point3D,
    pub text_height: f64,
    pub text_value: String,
}

/// ACAD_TABLE -- the same field shape as [`InsertEntity`] minus `attribs`
/// (a table has no attribute chain). `dwg.h`'s `Dwg_Entity_TABLE` carries
/// its own `ins_pt`/`scale`/`rotation`/`block_header`, and (per its own
/// `flag_for_table_value` field comment) "normally always" references a
/// cached-geometry block the way INSERT/DIMENSION do -- so it renders
/// through the same block-reference path rather than reconstructing cell
/// content from `num_cols`/`num_rows`/`col_widths` (which this crate does
/// not even parse).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcadTableEntity {
    pub common: EntityCommon,
    pub block_name: Ref<String>,
    pub insertion_point: Point3D,
    pub scale: Point3D,
    /// Radians.
    pub rotation: f64,
}

/// Same field shape as ATTRIB -- the *template* stored in a block
/// definition, versus ATTRIB, the value attached to an INSERT.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttdefEntity {
    pub common: EntityCommon,
    pub start_point: Point2D,
    pub text_height: f64,
    pub default_value: String,
    /// Radians (DXF 50). Unused by rendering (a template is never drawn),
    /// kept for parity with ATTRIB and for callers reading `entities`
    /// directly.
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportEntity {
    pub common: EntityCommon,
    pub center: Point3D,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face3DEntity {
    pub common: EntityCommon,
    /// Already-sequential order (unlike SOLID, no 1-2-4-3 reorder needed).
    pub corner1: Point3D,
    pub corner2: Point3D,
    pub corner3: Point3D,
    pub corner4: Point3D,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplineEntity {
    pub common: EntityCommon,
    /// Points that lie exactly on the curve -- preferred over
    /// `control_points` when present.
    pub fit_points: Vec<Point3D>,
    pub control_points: Vec<Point3D>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MTextEntity {
    pub common: EntityCommon,
    pub insertion_point: Point3D,
    pub text: String,
    pub text_height: f64,
    /// Radians. **Currently always `0.0`**: DWG stores MTEXT's rotation as
    /// a direction vector (`x_axis_dir`) and `convert.rs` does not derive an
    /// angle from it (`docs/CAVEATS.md`, "MTEXT rotation"). Callers must not
    /// expect a rotated MTEXT to report as rotated.
    pub rotation: f64,
    pub line_spacing_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolylineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point3D>,
    pub closed: bool,
}

/// One edge of a non-polyline HATCH boundary path. Curved edges are
/// chord-approximated at render time, the same simplification used for
/// SPLINE and curved 3DSOLID edges.
// JSON: internally tagged like `Entity`, upper-case like its tags --
// `{"type":"ARC","center":...}` -- but these are edge kinds, not DXF entity
// names (see `crate::json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", rename_all = "UPPERCASE")]
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
/// bulge/arc segments are dropped) or a list of curved/straight edges.
// JSON: adjacently tagged, because the payload is a sequence rather than a
// struct -- `{"type":"POLYLINE","data":[pt,..]}` / `{"type":"EDGES","data":
// [edge,..]}` -- keeping the `type` key every other tagged object uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data", rename_all = "UPPERCASE")]
pub enum HatchBoundaryPath {
    Polyline(Vec<Point2D>),
    Edges(Vec<HatchEdge>),
}

/// One pattern-fill "definition line" (`Dwg_HATCH_DefLine`): a family of
/// parallel lines that hatch a non-solid-fill boundary rather than just
/// outlining it.
///
/// The fields are used as-is, in the HATCH's own local coordinate space --
/// they are *not* further transformed by the HATCH's `angle`/`scale_spacing`
/// (DXF 52/41) that standard DXF documentation describes as applying on top,
/// because LibreDWG's defline data already has those baked in. See
/// `render_pattern_line` in `svg/hatch.rs` for the measurements behind
/// that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HatchPatternLine {
    /// Radians.
    pub angle: f64,
    /// A point that lies on one of the family's lines.
    pub base_point: Point2D,
    /// Displacement from one line to the next. Only the component
    /// perpendicular to `angle` (the spacing) is honored at render time; a
    /// parallel component (brick/masonry-style staggering) is dropped.
    pub offset: Point2D,
    /// Dash lengths along each line: positive = drawn, negative = gap,
    /// empty = continuous.
    pub dash_pattern: Vec<f64>,
}

/// A HATCH gradient fill, reduced to the two colors and the shape SVG's own
/// gradient elements need. `gradient_name` (`SPHERICAL`/`HEMISPHERICAL`/
/// `CURVED`/`LINEAR`/`CYLINDER`) collapses to `is_radial`: the two spherical
/// names become a radial gradient, everything else a linear one.
/// `gradient_shift` (DXF 461) is not applied.
///
/// **Unverified**: no drawing checked during development contains a
/// gradient-fill HATCH, so this is built from `dwg.h`'s field comments and
/// general DXF knowledge rather than confirmed against AutoCAD's own
/// rendering (`docs/CAVEATS.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HatchGradient {
    pub is_radial: bool,
    /// Radians.
    pub angle: f64,
    pub color1: String,
    pub color2: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HatchEntity {
    pub common: EntityCommon,
    pub boundary_paths: Vec<HatchBoundaryPath>,
    pub solid_fill: bool,
    /// `Some` when the HATCH is a gradient fill. Takes priority over
    /// `pattern_lines` at render time (a gradient fill's defline data is not
    /// meaningful pattern geometry).
    pub gradient: Option<HatchGradient>,
    /// Empty for solid fills, for gradient fills, or when the file's pattern
    /// data could not be read. When non-empty, rendering tiles the actual
    /// pattern lines inside `boundary_paths` instead of drawing an outline
    /// only.
    pub pattern_lines: Vec<HatchPatternLine>,
}

/// AutoCAD caches a DIMENSION's rendered geometry (lines, arrows, text) as an
/// anonymous block already in final world coordinates, so rendering one is
/// just drawing that block with an identity transform. All 7 DWG dimension
/// subtypes (ALIGNED, ANG2LN, ANG3PT, DIAMETER, LINEAR, ORDINATE,
/// ARC_DIMENSION) share this shape and fold into [`Entity::Dimension`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionEntity {
    pub common: EntityCommon,
    pub block_name: Ref<String>,
}

/// Best-effort wireframe extracted from a 3DSOLID's ACIS B-rep data (see
/// `acis.rs`). A solid's real shape has no directly 2D-renderable geometry;
/// `wireframe_edges` are straight chords between each ACIS `edge` record's
/// two endpoint vertices, in the solid's own local 3D space (the isometric
/// projection is applied at render time).
///
/// Empty when the wireframe could not be extracted (empty solid,
/// unrecognized ACIS version, conversion failure) -- callers should treat
/// that like an unsupported entity type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solid3DEntity {
    pub common: EntityCommon,
    pub wireframe_edges: Vec<[Point3D; 2]>,
    /// Edges in the ACIS data that could not be turned into a wireframe
    /// segment (endpoint vertices not resolvable). `0` for a fully read
    /// solid; a solid with no `wireframe_edges` and a non-zero count here
    /// was not read, not empty.
    #[serde(default)]
    pub skipped_edges: usize,
}

/// MULTILEADER's leader-line geometry only: the lines connecting the content
/// to its landing point, as straight-line polylines (spline leaders are
/// chord-approximated). The text/block content itself (`ctx.content`) is
/// neither extracted nor drawn -- a deliberately narrow scope, matching the
/// same "best-effort approximation" precedent as 3DSOLID's wireframe-only
/// and VIEWPORT's frame-only rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiLeaderEntity {
    pub common: EntityCommon,
    pub lines: Vec<Vec<Point3D>>,
}

/// One MLINE vertex: the centerline `point` plus `miter_direction`, a vector
/// that already accounts for the miter angle at this vertex, such that
/// `point + miter_direction * offset` is that vertex's position on the
/// parallel line `offset` away from the centerline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MLineVertex {
    pub point: Point3D,
    pub miter_direction: Point3D,
}

/// An MLINE is really a set of parallel offset lines (wall-style multi-line).
/// The per-line offsets live in the referenced MLINESTYLE object, resolved
/// against [`crate::tables::Tables::mlinestyles`] at render time: when the
/// lookup succeeds one polyline is drawn per style line, and when it fails
/// rendering falls back to a single centerline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MLineEntity {
    pub common: EntityCommon,
    pub vertices: Vec<MLineVertex>,
    pub closed: bool,
    pub mlinestyle_name: Ref<String>,
}

/// WIPEOUT's clip boundary, resolved to 2D points in the entity's own local
/// space (block nesting composes on top at render time).
///
/// **Unverified, twice over**: no drawing checked during development
/// contains a WIPEOUT, *and* the `pt0 + u*uvec + v*vvec` pixel-space-to-world
/// transform used to compute `boundary` comes from standard DXF image-entity
/// knowledge rather than from LibreDWG's own source (LibreDWG reads and
/// writes the raw fields but never renders images, so there is no reference
/// implementation to check against). See `wipeout_boundary` in `convert.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WipeoutEntity {
    pub common: EntityCommon,
    pub boundary: Vec<Point2D>,
}

/// A light source, which has no natural drawable shape. Rendered as a small
/// marker at `position` plus a dashed line to `target` when the light
/// actually aims somewhere (`has_target`) -- an honest placeholder in the
/// same spirit as VIEWPORT's frame, not a claim that this is what a LIGHT
/// looks like in AutoCAD (it is invisible in a normal 2D plan view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightEntity {
    pub common: EntityCommon,
    pub position: Point3D,
    pub target: Point3D,
    /// Distant and spot lights aim at `target`; point lights do not.
    pub has_target: bool,
}

/// Simple (non-MULTILEADER) LEADER: a polyline of `vertices` plus an optional
/// arrowhead at the first vertex. Style name, spline flag and text-box size
/// are parsed by neither this crate nor its renderer, so they are left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderEntity {
    pub common: EntityCommon,
    pub vertices: Vec<Point3D>,
    pub has_arrowhead: bool,
}

/// One entity of a parsed drawing.
///
/// A few variants share a payload type where the underlying DWG types are
/// structurally identical ([`Entity::XLine`] reuses [`RayEntity`],
/// [`Entity::Trace`] reuses [`SolidEntity`],
/// [`Entity::Region`]/[`Entity::PolylinePFace`] reuse [`Solid3DEntity`],
/// [`Entity::Polyline2D`] reuses [`LwPolylineEntity`]). They stay distinct
/// variants so [`type_name`](Self::type_name) still reports the real DXF
/// name.
// Internally tagged for JSON: `{"type":"LINE","common":{...},"start_point":...}`.
// Each variant's tag is spelled exactly as `type_name()` reports it -- the
// unit tests in json.rs check every variant for that agreement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type")]
pub enum Entity {
    #[serde(rename = "LINE")]
    Line(LineEntity),
    #[serde(rename = "CIRCLE")]
    Circle(CircleEntity),
    #[serde(rename = "TEXT")]
    Text(TextEntity),
    #[serde(rename = "LWPOLYLINE")]
    LwPolyline(LwPolylineEntity),
    #[serde(rename = "ARC")]
    Arc(ArcEntity),
    #[serde(rename = "ELLIPSE")]
    Ellipse(EllipseEntity),
    #[serde(rename = "POINT")]
    Point(PointEntity),
    #[serde(rename = "SOLID")]
    Solid(SolidEntity),
    /// Same four corners as SOLID, in the same order.
    #[serde(rename = "TRACE")]
    Trace(SolidEntity),
    #[serde(rename = "RAY")]
    Ray(RayEntity),
    #[serde(rename = "XLINE")]
    XLine(RayEntity),
    #[serde(rename = "INSERT")]
    Insert(InsertEntity),
    #[serde(rename = "ATTRIB")]
    Attrib(AttribEntity),
    #[serde(rename = "ATTDEF")]
    Attdef(AttdefEntity),
    #[serde(rename = "VIEWPORT")]
    Viewport(ViewportEntity),
    #[serde(rename = "3DFACE")]
    Face3D(Face3DEntity),
    #[serde(rename = "SPLINE")]
    Spline(SplineEntity),
    #[serde(rename = "MTEXT")]
    MText(MTextEntity),
    #[serde(rename = "POLYLINE_3D")]
    Polyline3D(PolylineEntity),
    #[serde(rename = "DIMENSION")]
    Dimension(DimensionEntity),
    #[serde(rename = "HATCH")]
    Hatch(HatchEntity),
    #[serde(rename = "3DSOLID")]
    Solid3D(Solid3DEntity),
    #[serde(rename = "LEADER")]
    Leader(LeaderEntity),
    #[serde(rename = "MULTILEADER")]
    MultiLeader(MultiLeaderEntity),
    #[serde(rename = "MLINE")]
    MLine(MLineEntity),
    /// REGION. `dwg.h` typedefs `Dwg_Entity_REGION` to `Dwg_Entity__3DSOLID`,
    /// so this reuses the identical ACIS wireframe extraction.
    #[serde(rename = "REGION")]
    Region(Solid3DEntity),
    /// POLYLINE_PFACE ("polyface mesh"). LibreDWG's own accessor for this
    /// type is documented `/* not implemented. use the dynapi instead */`,
    /// so `convert.rs` walks the `VERTEX_PFACE`/`VERTEX_PFACE_FACE` chain
    /// itself and resolves each face's vertex indices into wireframe edges --
    /// rendered like [`Entity::Region`], a polyface mesh being just as
    /// inherently 3D as an ACIS solid's wireframe.
    #[serde(rename = "POLYLINE_PFACE")]
    PolylinePFace(Solid3DEntity),
    #[serde(rename = "POLYLINE_2D")]
    Polyline2D(LwPolylineEntity),
    #[serde(rename = "TOLERANCE")]
    Tolerance(ToleranceEntity),
    #[serde(rename = "ACAD_TABLE")]
    AcadTable(AcadTableEntity),
    #[serde(rename = "WIPEOUT")]
    Wipeout(WipeoutEntity),
    #[serde(rename = "LIGHT")]
    Light(LightEntity),
    /// An entity type this crate does not convert. Carries the real DXF type
    /// name (from `dwg_object_get_dxfname`) so callers can still count and
    /// report by type. In JSON this is the one variant whose `"type"` tag
    /// (`"UNKNOWN"`) is not the DXF name; the real name is in `type_name`.
    #[serde(rename = "UNKNOWN")]
    Unknown {
        common: EntityCommon,
        type_name: String,
    },
}

impl Entity {
    pub fn common(&self) -> &EntityCommon {
        match self {
            Entity::Line(e) => &e.common,
            Entity::Circle(e) => &e.common,
            Entity::Text(e) => &e.common,
            Entity::LwPolyline(e) => &e.common,
            Entity::Arc(e) => &e.common,
            Entity::Ellipse(e) => &e.common,
            Entity::Point(e) => &e.common,
            Entity::Solid(e) | Entity::Trace(e) => &e.common,
            Entity::Ray(e) => &e.common,
            Entity::XLine(e) => &e.common,
            Entity::Insert(e) => &e.common,
            Entity::Attrib(e) => &e.common,
            Entity::Attdef(e) => &e.common,
            Entity::Viewport(e) => &e.common,
            Entity::Face3D(e) => &e.common,
            Entity::Spline(e) => &e.common,
            Entity::MText(e) => &e.common,
            Entity::Polyline3D(e) => &e.common,
            Entity::Dimension(e) => &e.common,
            Entity::Hatch(e) => &e.common,
            Entity::Solid3D(e) => &e.common,
            Entity::Leader(e) => &e.common,
            Entity::MultiLeader(e) => &e.common,
            Entity::MLine(e) => &e.common,
            Entity::Region(e) => &e.common,
            Entity::PolylinePFace(e) => &e.common,
            Entity::Polyline2D(e) => &e.common,
            Entity::Tolerance(e) => &e.common,
            Entity::AcadTable(e) => &e.common,
            Entity::Wipeout(e) => &e.common,
            Entity::Light(e) => &e.common,
            Entity::Unknown { common, .. } => common,
        }
    }

    /// The DXF/entity type name, e.g. `"LINE"`.
    pub fn type_name(&self) -> &str {
        match self {
            Entity::Line(_) => "LINE",
            Entity::Circle(_) => "CIRCLE",
            Entity::Text(_) => "TEXT",
            Entity::LwPolyline(_) => "LWPOLYLINE",
            Entity::Arc(_) => "ARC",
            Entity::Ellipse(_) => "ELLIPSE",
            Entity::Point(_) => "POINT",
            Entity::Solid(_) => "SOLID",
            Entity::Trace(_) => "TRACE",
            Entity::Ray(_) => "RAY",
            Entity::XLine(_) => "XLINE",
            Entity::Insert(_) => "INSERT",
            Entity::Attrib(_) => "ATTRIB",
            Entity::Attdef(_) => "ATTDEF",
            Entity::Viewport(_) => "VIEWPORT",
            Entity::Face3D(_) => "3DFACE",
            Entity::Spline(_) => "SPLINE",
            Entity::MText(_) => "MTEXT",
            Entity::Polyline3D(_) => "POLYLINE_3D",
            Entity::Dimension(_) => "DIMENSION",
            Entity::Hatch(_) => "HATCH",
            Entity::Solid3D(_) => "3DSOLID",
            Entity::Leader(_) => "LEADER",
            Entity::MultiLeader(_) => "MULTILEADER",
            Entity::MLine(_) => "MLINE",
            Entity::Region(_) => "REGION",
            Entity::PolylinePFace(_) => "POLYLINE_PFACE",
            Entity::Polyline2D(_) => "POLYLINE_2D",
            Entity::Tolerance(_) => "TOLERANCE",
            Entity::AcadTable(_) => "ACAD_TABLE",
            Entity::Wipeout(_) => "WIPEOUT",
            Entity::Light(_) => "LIGHT",
            Entity::Unknown { type_name, .. } => type_name,
        }
    }
}
