//! Dimension values: the measurement a DIMENSION stores, the same quantity
//! recomputed from its definition points, whether the stored one can be
//! believed, and the text the drawing shows for it.
//!
//! The model carries what the file states: `measurement` (DXF 42 as
//! written, radians for an angular kind; `None` for the 0 and -1 writers
//! leave when they did not measure), the definition points by DXF group,
//! the text override, the style's name, and the anonymous `*D` block the
//! drawn dimension is cached in. Everything a reader wants on top of that is
//! derived here, in this order of trust for the label: a text override that
//! is only whitespace means "no text"; an override wins over the cache
//! (`<>` in it standing for the measurement); the cache over a value this
//! crate formats itself.
//!
//! The formatter is deliberately basic -- decimal, architectural and
//! fractional units at the style's precision, `DIMRND` rounding, `DIMZIN`'s
//! leading- and trailing-zero suppression, `DIMPOST`'s prefix and suffix,
//! and decimal degrees for angles; the cached label is the value of record
//! whenever the file has one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uncad_model::model::{
    DimensionEntity, DimensionKind, Entity, OrdinateAxis, Point3D, TextOverride, ToleranceEntity,
};
use uncad_model::tables::{BlockRecord, DimStyleRecord, LinearUnitFormat};
use uncad_model::Tables;

use crate::text::decode_mtext;

/// The header's dimension variables, as far as the file states them: what a
/// dimension falls back on when its own DIMSTYLE says nothing (see
/// [`EffectiveStyle::resolve`]). `None` is "not stated".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DimDefaults {
    pub dimlfac: Option<f64>,
    pub dimdec: Option<u16>,
    pub dimlunit: Option<u16>,
    pub dimzin: Option<u16>,
    pub dimadec: Option<u16>,
    pub dimrnd: Option<f64>,
    pub dimpost: Option<String>,
}

impl DimDefaults {
    /// The variables `header` states.
    pub fn from_header(header: &uncad::Header) -> DimDefaults {
        DimDefaults {
            dimlfac: header.dimlfac,
            dimdec: header.dimdec,
            dimlunit: header.dimlunit,
            dimzin: header.dimzin,
            dimadec: header.dimadec,
            dimrnd: header.dimrnd,
            dimpost: header.dimpost.clone(),
        }
    }
}

/// The dimension-style variables the label depends on, resolved for one
/// dimension: its own DIMSTYLE's when the file wrote one, the header's where
/// it did not, and the format's documented default where neither says
/// anything (`DIMDEC` 4, `DIMLUNIT` 2 decimal, `DIMZIN` 0, `DIMADEC` 0,
/// `DIMLFAC` 1, no rounding, no `DIMPOST`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveStyle {
    pub dimlfac: f64,
    /// Decimal places, at most 8 (see [`EffectiveStyle::resolve`]).
    pub dimdec: u16,
    /// 1 scientific, 2 decimal, 3 engineering, 4 architectural, 5
    /// fractional, 6 Windows desktop.
    pub dimlunit: u16,
    pub dimzin: u16,
    /// Decimal places of an angle, at most 8.
    pub dimadec: u16,
    /// Round every linear value to a multiple of this; 0 is no rounding.
    pub dimrnd: f64,
    /// The prefix and suffix around a linear value: `<>` stands for the
    /// value; without one, the whole string is a suffix.
    pub dimpost: String,
}

/// The most decimal places a label is written with. Both `DIMDEC` and
/// `DIMADEC` hold 0..=8 per the DXF reference, but the fields are integers
/// a corrupt record can fill with anything: `format!` would honour 65535
/// literally, once per dimension in the drawing.
const MAX_DECIMALS: u16 = 8;

/// Whether a DIMSTYLE record carries the style's own settings, or is the
/// husk a file leaves behind when it names a style without writing its
/// body.
///
/// The distinction is what tells "the file said nothing" from "the file
/// said zero", and nothing in the record's numbers says it directly:
/// LibreDWG's DXF reader creates the record, presets the few fields it has
/// defaults for (`DIMSCALE = DIMLFAC = DIMTFAC = 1`, `DIMLUNIT = 2`) and
/// leaves every other field at zero, exactly as if the file had written 0.
/// The tell is the sizing pair: no usable style has both a text height and
/// an arrow size of zero, since a zero-height dimension text draws nothing.
/// `sample_2000.dxf` is the proof -- it names the same `Standard` style
/// that `sample_2000.dwg` writes in full (DIMDEC 4, DIMTXT 0.18,
/// DIMASZ 0.18) and leaves all three at 0 -- and every one of the 40-odd
/// real styles in the corpus has DIMTXT and DIMASZ above zero.
fn carries_a_body(style: &DimStyleRecord) -> bool {
    style.text_height.is_some_and(|h| h > 0.0) || style.arrow_size.is_some_and(|a| a > 0.0)
}

/// A decimal-place count from a file's integer field: negative is not a
/// count (DIMADEC's -1 is "use DIMDEC"; the caller handles that), and more
/// than [`MAX_DECIMALS`] is capped.
fn decimals(v: i64) -> Option<u16> {
    (v >= 0).then(|| v.min(i64::from(MAX_DECIMALS)) as u16)
}

fn unit_code(format: LinearUnitFormat) -> u16 {
    match format {
        LinearUnitFormat::Scientific => 1,
        LinearUnitFormat::Decimal => 2,
        LinearUnitFormat::Engineering => 3,
        LinearUnitFormat::Architectural => 4,
        LinearUnitFormat::Fractional => 5,
        LinearUnitFormat::WindowsDesktop => 6,
    }
}

impl EffectiveStyle {
    /// The values that govern a dimension's label: its own DIMSTYLE's where
    /// the file wrote one, the header's otherwise.
    ///
    /// A style the file wrote is taken at its word, 0 included: DIMDEC 0
    /// (whole millimetres) and DIMADEC 0 (whole degrees) are the ordinary
    /// metric settings, and treating them as "unset" gave a style that asks
    /// for "50" the header's precision instead -- "50.0000" against a
    /// picture, a cached label and a `strings.json` key all reading "50".
    /// DIMLUNIT and DIMLFAC keep a `> 0` guard because 0 is not a value
    /// either can hold: the unit codes run 1..=6, and a linear factor of 0
    /// would zero every label. A style that is only the husk a file leaves
    /// when it names a style without writing it (see `carries_a_body`)
    /// says nothing, and the header stands in. DIMADEC -1 is AutoCAD's "as
    /// DIMDEC".
    pub fn resolve(style: Option<&DimStyleRecord>, header: &DimDefaults) -> EffectiveStyle {
        let style = style.filter(|s| carries_a_body(s));
        let dimlfac = style
            .and_then(|s| s.length_factor)
            .filter(|v| *v > 0.0 && v.is_finite())
            .or(header.dimlfac.filter(|v| *v > 0.0 && v.is_finite()))
            .unwrap_or(1.0);
        let dimdec = match style.and_then(|s| s.decimal_places) {
            Some(v) => decimals(i64::from(v)),
            None => header.dimdec.map(|v| v.min(MAX_DECIMALS)),
        }
        .unwrap_or(4);
        let dimlunit = style
            .and_then(|s| s.linear_unit_format)
            .map(unit_code)
            .or(header.dimlunit.filter(|v| *v > 0))
            .unwrap_or(2);
        let dimzin = match style.and_then(|s| s.zero_suppression) {
            Some(v) => u16::try_from(v).unwrap_or(0),
            None => header.dimzin.unwrap_or(0),
        };
        let dimadec = match style.and_then(|s| s.angular_decimal_places) {
            Some(v) if v < 0 => Some(dimdec),
            Some(v) => decimals(i64::from(v)),
            None => header.dimadec.map(|v| v.min(MAX_DECIMALS)),
        }
        .unwrap_or(0);
        let dimrnd = style
            .and_then(|s| s.rounding)
            .or(header.dimrnd)
            .filter(|v| *v > 0.0 && v.is_finite())
            .unwrap_or(0.0);
        let dimpost = style
            .and_then(|s| s.post.clone())
            .or_else(|| header.dimpost.clone())
            .unwrap_or_default();
        EffectiveStyle {
            dimlfac,
            dimdec,
            dimlunit,
            dimzin,
            dimadec,
            dimrnd,
            dimpost,
        }
    }
}

/// Whether a dimension of this kind measures an angle.
pub fn is_angular(kind: Option<DimensionKind>) -> bool {
    matches!(
        kind,
        Some(DimensionKind::Angular2Line | DimensionKind::Angular3Point)
    )
}

/// How far a stored measurement may sit from the one the definition points
/// give before it is treated as not a measurement of this dimension at all.
///
/// Derived from the corpus, not chosen: across all 26 `example_*` /
/// `sample_*` files and the nine `AutoCADSamples*.dwg`, every DIMENSION
/// whose file wrote a real `act_measurement` agrees with
/// [`measurement_from_points`] to better than 2.3e-7 relative (one
/// dimension in `example_20xx` differs at all; the rest are exact). The
/// values this rejects are off by 100 %. 1 % leaves four orders of
/// magnitude of headroom over the worst honest disagreement.
const MEASUREMENT_DISAGREEMENT: f64 = 0.01;

/// The measurement a DIMENSION stores when it can be believed, in the unit
/// the package reports: degrees for an angular kind, drawing units (before
/// `DIMLFAC`) otherwise. `None` means "ask the definition points instead".
///
/// The model already reads the writers' "not measured" values -- 0, and
/// -1 -- as not stated. What is left is the file's value, which is refused
/// here when
/// - it is not finite, or is exactly -1 (a model built another way may
///   still carry it);
/// - it is negative or zero and the kind's measurement cannot be (every
///   kind but ORDINATE, which is a signed offset from a datum);
/// - it disagrees with the definition points by more than
///   [`MEASUREMENT_DISAGREEMENT`]. Magnitudes are compared, so a sign
///   convention reconstructed differently (ORDINATE's datum axis) does not
///   throw a good value away, and a computed value of 0 is no evidence
///   against a stored one.
pub fn usable_stored_measurement(
    stored: Option<f64>,
    kind: Option<DimensionKind>,
    from_points: Option<f64>,
) -> Option<f64> {
    let stored = stored.filter(|v| v.is_finite())?;
    if stored == -1.0 {
        return None;
    }
    let signed = kind == Some(DimensionKind::Ordinate);
    let value = if is_angular(kind) {
        stored.to_degrees()
    } else {
        stored
    };
    if !signed && value <= 0.0 {
        return None;
    }
    if let Some(computed) = from_points {
        let (a, b) = (value.abs(), computed.abs());
        if b != 0.0 && (a - b).abs() > MEASUREMENT_DISAGREEMENT * a.max(b) {
            return None;
        }
    }
    Some(value)
}

/// The quantity a dimension's definition points measure, with the meaning
/// [`usable_stored_measurement`] gives a stored one: drawing units for
/// linear kinds (before `DIMLFAC`), degrees for angular kinds, the arc
/// length for an arc-length dimension, a signed offset for an ordinate.
/// `None` for an unknown kind, or when a point the kind needs is missing.
///
/// The points are the model's, by DXF group: 13 and 14 (`extension1`,
/// `extension2`), 15 (`radial`), 16 (`arc`) and 10 (`definition_point`).
/// Per kind: a rotated dimension measures 13-14 along its rotation (DXF
/// 50); an aligned one the distance 13-14; a three-point angle the sector
/// at 15 between 13 and 14 that holds 10; a two-line angle the sector
/// between the line 13-14 and the line 15-10 that holds 16; a radius the
/// distance 10-15 (the centre to the curve); a diameter the chord 10-15; an
/// ordinate the feature 13's x or y offset from the datum 10; an arc length
/// the arc about 15 from 13 to 14 through 10.
pub fn measurement_from_points(d: &DimensionEntity) -> Option<f64> {
    let p = &d.points;
    let def = d.definition_point;
    match d.kind? {
        DimensionKind::Rotated => {
            let (a, b) = (p.extension1?, p.extension2?);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            Some((dx * d.rotation.cos() + dy * d.rotation.sin()).abs())
        }
        DimensionKind::Aligned => Some(distance(&p.extension1?, &p.extension2?)),
        DimensionKind::Angular3Point => {
            let (center, a, b) = (p.radial?, p.extension1?, p.extension2?);
            Some(sector_degrees(
                &center,
                &[angle_of(&center, &a), angle_of(&center, &b)],
                &def?,
            ))
        }
        DimensionKind::Angular2Line => {
            let (l1s, l1e, l2s, l2e) = (p.extension1?, p.extension2?, p.radial?, def?);
            let vertex = intersection(&l1s, &l1e, &l2s, &l2e)?;
            let a1 = (l1e.y - l1s.y).atan2(l1e.x - l1s.x);
            let a2 = (l2e.y - l2s.y).atan2(l2e.x - l2s.x);
            // Each line contributes two rays from the vertex; the arc point
            // says which of the four sectors was dimensioned.
            let pi = std::f64::consts::PI;
            Some(sector_degrees(
                &vertex,
                &[a1, a1 + pi, a2, a2 + pi],
                &p.arc?,
            ))
        }
        DimensionKind::Radius => Some(distance(&def?, &p.radial?)),
        DimensionKind::Diameter => Some(distance(&def?, &p.radial?)),
        DimensionKind::Ordinate => {
            let (feature, datum) = (p.extension1?, def?);
            Some(match d.ordinate_axis? {
                OrdinateAxis::X => feature.x - datum.x,
                OrdinateAxis::Y => feature.y - datum.y,
            })
        }
        DimensionKind::ArcLength => {
            let (center, a, b) = (p.radial?, p.extension1?, p.extension2?);
            let radius = distance(&center, &a);
            let sweep = sector_degrees(
                &center,
                &[angle_of(&center, &a), angle_of(&center, &b)],
                &def?,
            );
            Some(radius * sweep.to_radians())
        }
    }
}

fn distance(a: &Point3D, b: &Point3D) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}

fn angle_of(from: &Point3D, to: &Point3D) -> f64 {
    (to.y - from.y).atan2(to.x - from.x)
}

/// Where two lines (each given by two points) cross, in the XY plane.
fn intersection(p1: &Point3D, p2: &Point3D, p3: &Point3D, p4: &Point3D) -> Option<Point3D> {
    let (d1x, d1y) = (p2.x - p1.x, p2.y - p1.y);
    let (d2x, d2y) = (p4.x - p3.x, p4.y - p3.y);
    let cross = d1x * d2y - d1y * d2x;
    if cross.abs() < 1e-12 {
        return None;
    }
    let t = ((p3.x - p1.x) * d2y - (p3.y - p1.y) * d2x) / cross;
    Some(Point3D {
        x: p1.x + t * d1x,
        y: p1.y + t * d1y,
        z: p1.z,
    })
}

/// The angular size, in degrees, of the sector between two consecutive rays
/// (angles in radians, from `vertex`) that contains the direction of
/// `probe` -- the arc point a dimension was drawn through, which is how
/// AutoCAD records which of the possible sectors was measured.
fn sector_degrees(vertex: &Point3D, rays: &[f64], probe: &Point3D) -> f64 {
    let tau = std::f64::consts::TAU;
    let norm = |a: f64| a.rem_euclid(tau);
    let mut sorted: Vec<f64> = rays.iter().map(|&a| norm(a)).collect();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    if sorted.len() < 2 {
        return 0.0;
    }
    let target = norm(angle_of(vertex, probe));
    for i in 0..sorted.len() {
        let start = sorted[i];
        let end = if i + 1 < sorted.len() {
            sorted[i + 1]
        } else {
            sorted[0] + tau
        };
        let t = if target < start { target + tau } else { target };
        if t >= start && t <= end {
            return (end - start).to_degrees();
        }
    }
    // Unreachable in exact arithmetic; the last sector wraps around.
    (sorted[0] + tau - sorted[sorted.len() - 1]).to_degrees()
}

/// Formats a measurement the way a basic dimension style shows it: a linear
/// value times `DIMLFAC`, rounded to `DIMRND`, in decimal (`DIMLUNIT` 2 and
/// every code this does not handle), architectural feet-inches (4) or
/// fractional inches (5) at `DIMDEC` precision, `DIMZIN`'s leading (bit 4)
/// and trailing (bit 8) zeros dropped from a decimal, and `DIMPOST`
/// around it; an angle as decimal degrees at `DIMADEC` precision with a
/// degree sign.
pub fn format_measurement(value: f64, angular: bool, style: &EffectiveStyle) -> String {
    if angular {
        return format!("{:.*}\u{00B0}", usize::from(style.dimadec), value);
    }
    let mut scaled = value * style.dimlfac;
    if style.dimrnd > 0.0 {
        scaled = (scaled / style.dimrnd).round() * style.dimrnd;
    }
    let text = match style.dimlunit {
        4 => format_architectural(scaled, style.dimdec),
        5 => format_fractional(scaled, style.dimdec),
        _ => {
            let mut text = format!("{:.*}", usize::from(style.dimdec), scaled);
            if style.dimzin & 8 != 0 && text.contains('.') {
                text = text.trim_end_matches('0').trim_end_matches('.').to_string();
            }
            if style.dimzin & 4 != 0 {
                if let Some(rest) = text.strip_prefix("0.") {
                    text = format!(".{rest}");
                } else if let Some(rest) = text.strip_prefix("-0.") {
                    text = format!("-.{rest}");
                }
            }
            text
        }
    };
    with_post(&text, &style.dimpost)
}

/// `DIMPOST` around a formatted value: `<>` in it stands for the value;
/// without one, the whole string follows the value.
fn with_post(value: &str, post: &str) -> String {
    if post.is_empty() {
        value.to_string()
    } else if post.contains("<>") {
        post.replacen("<>", value, 1)
    } else {
        format!("{value}{post}")
    }
}

/// `7'-4 1/2"`: whole feet, whole inches, and a fraction to the nearest
/// `1/2^DIMDEC` inch (for architectural units `DIMDEC` is that exponent).
fn format_architectural(inches: f64, dimdec: u16) -> String {
    let sign = if inches < 0.0 { "-" } else { "" };
    let (whole, fraction) = split_fraction(inches.abs(), dimdec);
    let feet = whole / 12;
    let rest = whole % 12;
    let mut text = format!("{sign}{feet}'-{rest}");
    if let Some(fraction) = fraction {
        text.push(' ');
        text.push_str(&fraction);
    }
    text.push('"');
    text
}

fn format_fractional(value: f64, dimdec: u16) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let (whole, fraction) = split_fraction(value.abs(), dimdec);
    match fraction {
        Some(fraction) if whole == 0 => format!("{sign}{fraction}"),
        Some(fraction) => format!("{sign}{whole} {fraction}"),
        None => format!("{sign}{whole}"),
    }
}

/// Rounds `value` to the nearest `1/2^dimdec` and splits it into a whole
/// part and a reduced fraction (`None` when there is none).
fn split_fraction(value: f64, dimdec: u16) -> (u64, Option<String>) {
    let denominator: u64 = 1 << dimdec.min(MAX_DECIMALS);
    let total = (value * denominator as f64).round() as u64;
    let whole = total / denominator;
    let mut numerator = total % denominator;
    if numerator == 0 {
        return (whole, None);
    }
    let mut den = denominator;
    while numerator.is_multiple_of(2) {
        numerator /= 2;
        den /= 2;
    }
    (whole, Some(format!("{numerator}/{den}")))
}

/// Where a dimension's label came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplaySource {
    /// Nothing to show: no override, no cached label, no measurement.
    None,
    /// The text override (DXF 1), with `<>` replaced by the formatted
    /// measurement.
    UserText,
    /// The TEXT/MTEXT inside the cached `*D` block: what the drawing shows.
    CachedBlock,
    /// This crate formatted the measurement itself (basic rules only).
    Formatted,
    /// The override is whitespace: AutoCAD's "no text" convention.
    Suppressed,
}

/// A dimension's label, as a reader sees it and as the file wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayText {
    /// With the codes read (see [`crate::text`]).
    pub text: String,
    /// As the file wrote it: the override, or the cached block's strings;
    /// empty for a label this crate formatted.
    pub raw: String,
    pub source: DisplaySource,
}

/// The text inside a dimension's cached block: `(raw, plain)`, the block's
/// TEXT and MTEXT entities in order, joined by a space; `None` when it has
/// none, or only whitespace.
pub fn cached_label(block: &BlockRecord) -> Option<(String, String)> {
    let mut raw = Vec::new();
    let mut plain = Vec::new();
    for entity in &block.entities {
        match entity {
            Entity::Text(t) => {
                raw.push(t.text.clone());
                plain.push(crate::text::decode_text(&t.text).plain);
            }
            Entity::MText(m) => {
                raw.push(m.text.clone());
                plain.push(decode_mtext(&m.text).plain);
            }
            _ => {}
        }
    }
    let plain = plain.join(" ");
    (!plain.trim().is_empty()).then(|| (raw.join(" "), plain))
}

/// The label of `d`, by the precedence in the module docs. `value` is the
/// measurement to format (see [`usable_stored_measurement`] and
/// [`measurement_from_points`]), `cached` its block's label.
pub fn display_text(
    d: &DimensionEntity,
    style: &EffectiveStyle,
    value: Option<f64>,
    cached: Option<&(String, String)>,
) -> DisplayText {
    let formatted = value.map(|v| format_measurement(v, is_angular(d.kind), style));
    match &d.text_override {
        TextOverride::Suppressed => DisplayText {
            text: String::new(),
            raw: " ".to_string(),
            source: DisplaySource::Suppressed,
        },
        TextOverride::Literal(s) if s.trim().is_empty() => DisplayText {
            text: String::new(),
            raw: s.clone(),
            source: DisplaySource::Suppressed,
        },
        TextOverride::Literal(s) => DisplayText {
            text: decode_mtext(&s.replace("<>", formatted.as_deref().unwrap_or(""))).plain,
            raw: s.clone(),
            source: DisplaySource::UserText,
        },
        TextOverride::Measured => match (cached, formatted) {
            (Some((raw, plain)), _) => DisplayText {
                text: plain.clone(),
                raw: raw.clone(),
                source: DisplaySource::CachedBlock,
            },
            (None, Some(text)) => DisplayText {
                text,
                raw: String::new(),
                source: DisplaySource::Formatted,
            },
            (None, None) => DisplayText {
                text: String::new(),
                raw: String::new(),
                source: DisplaySource::None,
            },
        },
    }
}

/// The cached label of every block that has one, by block name -- what
/// [`display_text`] looks a dimension's `block_name` up in.
pub fn cached_labels(tables: &Tables) -> BTreeMap<String, (String, String)> {
    tables
        .block_records
        .iter()
        .filter_map(|(name, block)| cached_label(block).map(|label| (name.clone(), label)))
        .collect()
}

/// The height a TOLERANCE's text is drawn at: the height the entity states
/// when positive (LibreDWG decodes one for R13/R14 files only), else its
/// DIMSTYLE's text height, else 1 -- the renderer's rule, so the record and
/// the picture agree.
pub fn tolerance_text_height(t: &ToleranceEntity, tables: &Tables) -> f64 {
    let positive = |h: f64| (h.is_finite() && h > 0.0).then_some(h);
    t.text_height
        .and_then(positive)
        .or_else(|| {
            t.style_name
                .resolved()
                .and_then(|name| tables.dim_styles.get(name))
                .and_then(|style| style.text_height)
                .and_then(positive)
        })
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uncad_model::model::{Confidence, DimensionPoints, EntityCommon, EntityId, Origin, Ref};
    use uncad_model::Point2D;

    fn p(x: f64, y: f64) -> Point3D {
        Point3D { x, y, z: 0.0 }
    }

    fn style() -> EffectiveStyle {
        EffectiveStyle {
            dimlfac: 1.0,
            dimdec: 2,
            dimlunit: 2,
            dimzin: 0,
            dimadec: 0,
            dimrnd: 0.0,
            dimpost: String::new(),
        }
    }

    fn dim(kind: DimensionKind, def: Option<Point3D>, points: DimensionPoints) -> DimensionEntity {
        DimensionEntity {
            common: EntityCommon {
                id: EntityId::new(1),
                origin: Origin::Vector,
                confidence: Confidence::High,
                source_handle: Ref::Resolved("1".into()),
                layer: Ref::Resolved("0".into()),
                color_index: 256,
                true_color: None,
                invisible: false,
            },
            block_name: Ref::Resolved("*D1".into()),
            kind: Some(kind),
            measurement: None,
            text_override: TextOverride::Measured,
            definition_point: def,
            text_midpoint: Point2D { x: 5.0, y: 2.0 },
            points,
            rotation: 0.0,
            text_rotation: 0.0,
            style_name: Ref::Resolved("STANDARD".into()),
            ordinate_axis: None,
        }
    }

    fn pts(
        e1: Option<Point3D>,
        e2: Option<Point3D>,
        radial: Option<Point3D>,
        arc: Option<Point3D>,
    ) -> DimensionPoints {
        DimensionPoints {
            extension1: e1,
            extension2: e2,
            radial,
            arc,
        }
    }

    #[test]
    fn a_rotated_dimension_projects_on_its_direction_and_an_aligned_one_does_not() {
        let mut d = dim(
            DimensionKind::Rotated,
            Some(p(0.0, 0.0)),
            pts(Some(p(0.0, 0.0)), Some(p(3.0, 4.0)), None, None),
        );
        assert_eq!(measurement_from_points(&d), Some(3.0));
        d.rotation = std::f64::consts::FRAC_PI_2;
        assert!((measurement_from_points(&d).unwrap() - 4.0).abs() < 1e-12);
        d.kind = Some(DimensionKind::Aligned);
        assert_eq!(measurement_from_points(&d), Some(5.0));
        // A point the kind needs is missing: nothing to compute.
        d.points.extension2 = None;
        assert_eq!(measurement_from_points(&d), None);
        d.kind = None;
        assert_eq!(measurement_from_points(&d), None);
    }

    #[test]
    fn radius_and_diameter_are_the_chord_distances() {
        // RADIUS: group 10 is the centre, 15 the point on the curve.
        let radius = dim(
            DimensionKind::Radius,
            Some(p(0.0, 0.0)),
            pts(None, None, Some(p(0.0, 7.0)), None),
        );
        assert_eq!(measurement_from_points(&radius), Some(7.0));
        // DIAMETER: 10 and 15 are the two ends of the chord.
        let diameter = dim(
            DimensionKind::Diameter,
            Some(p(-5.0, 0.0)),
            pts(None, None, Some(p(5.0, 0.0)), None),
        );
        assert_eq!(measurement_from_points(&diameter), Some(10.0));
    }

    #[test]
    fn angular_sectors_follow_the_arc_point() {
        // Three-point: vertex 15, ends 13 and 14, the arc through 10.
        let mut three_point = dim(
            DimensionKind::Angular3Point,
            Some(p(5.0, 5.0)),
            pts(
                Some(p(10.0, 0.0)),
                Some(p(0.0, 10.0)),
                Some(p(0.0, 0.0)),
                None,
            ),
        );
        let inside = measurement_from_points(&three_point).unwrap();
        assert!((inside - 90.0).abs() < 1e-9, "{inside}");
        three_point.definition_point = Some(p(-5.0, -5.0));
        let reflex = measurement_from_points(&three_point).unwrap();
        assert!((reflex - 270.0).abs() < 1e-9, "{reflex}");

        // Two lines crossing at 108 degrees (a 72-degree acute pair): the
        // first line 13-14, the second 15-10, the arc point 16.
        let a = 108f64.to_radians();
        let two_line = |probe: Point3D| {
            dim(
                DimensionKind::Angular2Line,
                Some(p(10.0 * a.cos(), 10.0 * a.sin())),
                pts(
                    Some(p(0.0, 0.0)),
                    Some(p(10.0, 0.0)),
                    Some(p(0.0, 0.0)),
                    Some(probe),
                ),
            )
        };
        let obtuse =
            measurement_from_points(&two_line(p((a / 2.0).cos() * 5.0, (a / 2.0).sin() * 5.0)))
                .unwrap();
        assert!((obtuse - 108.0).abs() < 1e-9, "{obtuse}");
        let acute = measurement_from_points(&two_line(p(-1.0, 3.0))).unwrap();
        assert!((acute - 72.0).abs() < 1e-9, "{acute}");
    }

    #[test]
    fn ordinate_and_arc_length() {
        let mut ordinate = dim(
            DimensionKind::Ordinate,
            Some(p(10.0, 10.0)),
            pts(Some(p(25.0, 40.0)), Some(p(25.0, 60.0)), None, None),
        );
        ordinate.ordinate_axis = Some(OrdinateAxis::X);
        assert_eq!(measurement_from_points(&ordinate), Some(15.0));
        ordinate.ordinate_axis = Some(OrdinateAxis::Y);
        assert_eq!(measurement_from_points(&ordinate), Some(30.0));
        ordinate.ordinate_axis = None;
        assert_eq!(measurement_from_points(&ordinate), None);
        let arc = dim(
            DimensionKind::ArcLength,
            Some(p(1.0, 1.0)),
            pts(
                Some(p(2.0, 0.0)),
                Some(p(0.0, 2.0)),
                Some(p(0.0, 0.0)),
                None,
            ),
        );
        let length = measurement_from_points(&arc).unwrap();
        assert!((length - std::f64::consts::PI).abs() < 1e-9, "{length}");
    }

    #[test]
    fn formatting_follows_the_style() {
        let mut s = style();
        assert_eq!(format_measurement(10.0, false, &s), "10.00");
        s.dimlfac = 12.0;
        assert_eq!(format_measurement(10.0, false, &s), "120.00");
        s.dimzin = 8;
        assert_eq!(format_measurement(10.0, false, &s), "120");
        assert_eq!(format_measurement(10.55, false, &s), "126.6");
        s.dimlfac = 1.0;
        s.dimlunit = 4;
        s.dimdec = 4;
        assert_eq!(format_measurement(88.0, false, &s), "7'-4\"");
        assert_eq!(format_measurement(92.375, false, &s), "7'-8 3/8\"");
        assert_eq!(format_measurement(2.5, false, &s), "0'-2 1/2\"");
        assert_eq!(format_measurement(12.0, false, &s), "1'-0\"");
        s.dimlunit = 5;
        assert_eq!(format_measurement(2.5, false, &s), "2 1/2");
        assert_eq!(format_measurement(0.75, false, &s), "3/4");
        assert_eq!(format_measurement(3.0, false, &s), "3");
        s.dimadec = 1;
        assert_eq!(format_measurement(108.0, true, &s), "108.0\u{00B0}");
    }

    #[test]
    fn rounding_leading_zeros_and_the_post_string() {
        let mut s = style();
        // DIMRND 0.25: 10.1 is 10.00, 10.2 is 10.25.
        s.dimrnd = 0.25;
        assert_eq!(format_measurement(10.1, false, &s), "10.00");
        assert_eq!(format_measurement(10.2, false, &s), "10.25");
        s.dimrnd = 0.0;
        // DIMZIN bit 4 drops a leading zero.
        s.dimzin = 4;
        assert_eq!(format_measurement(0.5, false, &s), ".50");
        assert_eq!(format_measurement(-0.5, false, &s), "-.50");
        assert_eq!(format_measurement(1.5, false, &s), "1.50");
        s.dimzin = 12;
        assert_eq!(format_measurement(0.5, false, &s), ".5");
        s.dimzin = 0;
        // DIMPOST: a suffix, or a template around `<>`. Not for angles.
        s.dimpost = " mm".into();
        assert_eq!(format_measurement(2.0, false, &s), "2.00 mm");
        s.dimpost = "\u{2205}<> TYP".into();
        assert_eq!(format_measurement(2.0, false, &s), "\u{2205}2.00 TYP");
        assert_eq!(format_measurement(90.0, true, &s), "90\u{00B0}");
    }

    #[test]
    fn display_precedence_suppressed_override_cache_formatted() {
        let base = dim(
            DimensionKind::Aligned,
            Some(p(0.0, 0.0)),
            pts(Some(p(0.0, 0.0)), Some(p(10.0, 0.0)), None, None),
        );
        let cached = ("\\A1;120".to_string(), "120".to_string());

        let mut d = base.clone();
        d.text_override = TextOverride::Suppressed;
        let shown = display_text(&d, &style(), Some(10.0), Some(&cached));
        assert_eq!(
            (shown.text.as_str(), shown.source),
            ("", DisplaySource::Suppressed)
        );
        d.text_override = TextOverride::Literal("  ".into());
        let shown = display_text(&d, &style(), Some(10.0), Some(&cached));
        assert_eq!(shown.source, DisplaySource::Suppressed);

        d.text_override = TextOverride::Literal("<> TYP.".into());
        let shown = display_text(&d, &style(), Some(10.0), Some(&cached));
        assert_eq!(shown.text, "10.00 TYP.");
        assert_eq!(shown.raw, "<> TYP.");
        assert_eq!(shown.source, DisplaySource::UserText);

        let shown = display_text(&base, &style(), Some(10.0), Some(&cached));
        assert_eq!(
            (shown.text.as_str(), shown.source),
            ("120", DisplaySource::CachedBlock)
        );
        assert_eq!(shown.raw, "\\A1;120");

        let shown = display_text(&base, &style(), Some(10.0), None);
        assert_eq!(
            (shown.text.as_str(), shown.source),
            ("10.00", DisplaySource::Formatted)
        );
        let shown = display_text(&base, &style(), None, None);
        assert_eq!(shown.source, DisplaySource::None);
    }

    #[test]
    fn a_stored_measurement_that_cannot_be_this_dimensions_is_refused() {
        // Expected values come from the geometry, not from the function:
        // the ALIGNED pair (0,0)-(3,4) is 5 units apart (3-4-5), and the
        // ORDINATE offset is 4 - 1.
        let aligned = Some(DimensionKind::Aligned);
        let ordinate = Some(DimensionKind::Ordinate);
        let take = usable_stored_measurement;

        assert_eq!(take(Some(0.0), aligned, Some(5.0)), None);
        assert_eq!(take(Some(0.0), aligned, None), None);
        assert_eq!(take(Some(-1.0), aligned, Some(5.0)), None);
        assert_eq!(take(Some(-5.0), aligned, Some(5.0)), None);
        assert_eq!(take(Some(f64::NAN), aligned, Some(5.0)), None);
        assert_eq!(take(None, aligned, Some(5.0)), None);
        // A value that measures something else entirely.
        assert_eq!(take(Some(500.0), aligned, Some(5.0)), None);
        // A believable one survives, disagreement inside the tolerance
        // included (5.02 is 0.4 % off 5.0).
        assert_eq!(take(Some(5.0), aligned, Some(5.0)), Some(5.0));
        assert_eq!(take(Some(5.02), aligned, Some(5.0)), Some(5.02));
        assert_eq!(take(Some(5.0), aligned, None), Some(5.0));

        // An ORDINATE is a signed offset from a datum, so 0 and a negative
        // value are both measurements it can have ...
        assert_eq!(take(Some(0.0), ordinate, Some(0.0)), Some(0.0));
        assert_eq!(take(Some(-3.0), ordinate, Some(3.0)), Some(-3.0));
        assert_eq!(take(Some(3.0), ordinate, Some(-3.0)), Some(3.0));
        // ... but not one that disagrees with the points by 100 %.
        assert_eq!(take(Some(0.0), ordinate, Some(3.0)), None);

        // Angular kinds store radians and are reported in degrees.
        let angular = Some(DimensionKind::Angular3Point);
        let quarter = std::f64::consts::FRAC_PI_2;
        let got = take(Some(quarter), angular, Some(90.0)).expect("a right angle");
        assert!((got - 90.0).abs() < 1e-9, "{got}");
        assert_eq!(take(Some(0.0), angular, Some(90.0)), None);
    }

    fn header(dimdec: u16, dimadec: u16, dimzin: u16) -> DimDefaults {
        DimDefaults {
            dimdec: Some(dimdec),
            dimadec: Some(dimadec),
            dimzin: Some(dimzin),
            dimlunit: Some(2),
            dimlfac: Some(1.0),
            ..Default::default()
        }
    }

    fn metric() -> DimStyleRecord {
        DimStyleRecord {
            name: "ISO-0".into(),
            decimal_places: Some(0),
            angular_decimal_places: Some(0),
            zero_suppression: Some(0),
            linear_unit_format: Some(LinearUnitFormat::Decimal),
            length_factor: Some(1.0),
            // What says the file wrote this style's body at all.
            text_height: Some(2.5),
            arrow_size: Some(2.5),
            ..Default::default()
        }
    }

    #[test]
    fn a_style_the_file_wrote_is_taken_at_its_word_zero_included() {
        // DIMDEC 0 ("50", whole millimetres) and DIMADEC 0 are ordinary
        // metric settings, and were read as "unset": the header's DIMDEC 4
        // stood in and every label came out as "50.0000".
        let header = header(4, 3, 8);
        let resolved = EffectiveStyle::resolve(Some(&metric()), &header);
        assert_eq!(
            (resolved.dimdec, resolved.dimadec, resolved.dimzin),
            (0, 0, 0)
        );
        assert_eq!(format_measurement(50.0, false, &resolved), "50");
        assert_eq!(format_measurement(108.0, true, &resolved), "108\u{00B0}");

        // A style the file wrote in full is still honoured when it asks for
        // decimals.
        let three = DimStyleRecord {
            decimal_places: Some(3),
            ..metric()
        };
        assert_eq!(
            format_measurement(50.0, false, &EffectiveStyle::resolve(Some(&three), &header)),
            "50.000"
        );

        // The husk a file leaves when it names a style without writing its
        // body reads back as zeros (LibreDWG's DXF reader presets only
        // DIMSCALE/DIMLFAC/DIMTFAC = 1 and DIMLUNIT = 2), and says nothing:
        // the header's values stand in.
        let husk = DimStyleRecord {
            name: "STANDARD".into(),
            length_factor: Some(1.0),
            linear_unit_format: Some(LinearUnitFormat::Decimal),
            decimal_places: Some(0),
            angular_decimal_places: Some(0),
            zero_suppression: Some(0),
            text_height: Some(0.0),
            arrow_size: Some(0.0),
            ..Default::default()
        };
        let resolved = EffectiveStyle::resolve(Some(&husk), &header);
        assert_eq!(
            (resolved.dimdec, resolved.dimadec, resolved.dimzin),
            (4, 3, 8)
        );
        assert_eq!(
            EffectiveStyle::resolve(None, &header),
            EffectiveStyle::resolve(Some(&husk), &header)
        );

        // DIMLUNIT and DIMLFAC keep their guard: 0 is not a value either
        // can hold, so a style carrying one takes the header's.
        let zeroed = DimStyleRecord {
            linear_unit_format: None,
            length_factor: Some(0.0),
            ..metric()
        };
        let header = DimDefaults {
            dimlunit: Some(4),
            dimlfac: Some(12.0),
            ..header
        };
        let resolved = EffectiveStyle::resolve(Some(&zeroed), &header);
        assert_eq!((resolved.dimlunit, resolved.dimlfac), (4, 12.0));

        // Nothing stated anywhere: the format's defaults.
        let resolved = EffectiveStyle::resolve(None, &DimDefaults::default());
        assert_eq!(
            (
                resolved.dimdec,
                resolved.dimlunit,
                resolved.dimzin,
                resolved.dimadec,
                resolved.dimlfac
            ),
            (4, 2, 0, 0, 1.0)
        );
        // DIMADEC -1 is "as DIMDEC".
        let as_dimdec = DimStyleRecord {
            decimal_places: Some(2),
            angular_decimal_places: Some(-1),
            ..metric()
        };
        assert_eq!(
            EffectiveStyle::resolve(Some(&as_dimdec), &DimDefaults::default()).dimadec,
            2
        );
        // The post string comes from the style, else the header.
        let posted = DimStyleRecord {
            post: Some("<> mm".into()),
            ..metric()
        };
        let header = DimDefaults {
            dimpost: Some(" in".into()),
            ..DimDefaults::default()
        };
        assert_eq!(
            EffectiveStyle::resolve(Some(&posted), &header).dimpost,
            "<> mm"
        );
        assert_eq!(EffectiveStyle::resolve(None, &header).dimpost, " in");
    }

    #[test]
    fn a_precision_outside_the_dxf_range_does_not_build_a_65_kb_label() {
        // A 16-bit DIMDEC of 65535 (a corrupt record) would have `format!`
        // build a 65 KB label for every dimension in the drawing.
        let wild = DimStyleRecord {
            decimal_places: Some(65_535),
            angular_decimal_places: Some(i32::MAX),
            ..metric()
        };
        let resolved = EffectiveStyle::resolve(Some(&wild), &DimDefaults::default());
        // 8 decimals is the DXF reference's maximum for both fields.
        assert_eq!(format_measurement(1.5, false, &resolved), "1.50000000");
        assert_eq!(
            format_measurement(90.0, true, &resolved),
            "90.00000000\u{00B0}"
        );
        let header = DimDefaults {
            dimdec: Some(u16::MAX),
            ..DimDefaults::default()
        };
        assert_eq!(EffectiveStyle::resolve(None, &header).dimdec, 8);
    }
}
