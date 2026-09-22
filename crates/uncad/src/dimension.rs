//! Dimension values: the measurement a DIMENSION stores, the same quantity
//! recomputed from its definition points, and the text the drawing shows for
//! it.
//!
//! A DIMENSION carries three things a reader of the drawing needs and the
//! 0.2.0 model dropped: `act_measurement` (DXF 42, the value AutoCAD last
//! measured, in drawing units before `DIMLFAC`; radians for angular kinds),
//! the definition points the value was measured between, and the label --
//! which is either `user_text` (DXF 1, an override, `<>` standing for the
//! measurement), or the TEXT/MTEXT inside the anonymous `*D` block AutoCAD
//! caches the drawn dimension in. This module derives
//! [`DimensionEntity::display_text`] from those, in that order of trust:
//! a whitespace-only override means "suppressed", an override wins over the
//! cache, the cache over a value this crate formats itself.
//!
//! The formatter here is deliberately basic (decimal, architectural and
//! fractional units at the style's precision, `DIMZIN` trailing-zero
//! suppression, degrees for angles); the cached label is the value of record
//! whenever the file has one.

use std::collections::BTreeMap;

use crate::header::Header;
use crate::model::{DimensionEntity, DimensionGeometry, DisplaySource, Entity, Point3D};
use crate::tables::DimStyleRecord;
use crate::CadDatabase;

/// The dimension-style variables the label depends on, resolved for one
/// dimension: its own DIMSTYLE when the file has it, the header's current
/// values otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectiveStyle {
    pub dimlfac: f64,
    pub dimdec: u16,
    pub dimlunit: u16,
    pub dimzin: u16,
    pub dimadec: u16,
}

impl EffectiveStyle {
    /// A style field of 0 means "unset" in a file that never wrote it; the
    /// header's value stands in.
    pub fn resolve(style: Option<&DimStyleRecord>, header: &Header) -> EffectiveStyle {
        let pick_f64 = |s: Option<f64>, h: f64| s.filter(|v| *v > 0.0).unwrap_or(h);
        let pick_u16 = |s: Option<u16>, h: u16| s.filter(|v| *v > 0).unwrap_or(h);
        let dimlfac = pick_f64(style.map(|s| s.dimlfac), header.dimlfac);
        EffectiveStyle {
            dimlfac: if dimlfac > 0.0 { dimlfac } else { 1.0 },
            dimdec: pick_u16(style.map(|s| s.dimdec), header.dimdec),
            dimlunit: pick_u16(style.map(|s| s.dimlunit), header.dimlunit),
            dimzin: style.map(|s| s.dimzin).unwrap_or(header.dimzin),
            dimadec: pick_u16(style.map(|s| s.dimadec), header.dimadec),
        }
    }
}

/// The quantity a dimension's definition points measure, with the same
/// meaning as [`DimensionEntity::measurement`]: drawing units for linear
/// kinds (before `DIMLFAC`), degrees for angular kinds, the arc length for
/// an arc-length dimension, a signed offset for an ordinate. `None` for an
/// unknown geometry.
pub fn measurement_from_points(
    geometry: &DimensionGeometry,
    definition_point: Point3D,
) -> Option<f64> {
    match geometry {
        DimensionGeometry::Linear {
            xline1,
            xline2,
            rotation,
        } => {
            // The distance between the extension-line origins projected on
            // the dimension line's direction.
            let (dx, dy) = (xline2.x - xline1.x, xline2.y - xline1.y);
            Some((dx * rotation.cos() + dy * rotation.sin()).abs())
        }
        DimensionGeometry::Aligned { xline1, xline2 } => Some(distance(xline1, xline2)),
        DimensionGeometry::Angular3Point {
            center,
            xline1,
            xline2,
        } => Some(sector_degrees(
            center,
            &[angle_of(center, xline1), angle_of(center, xline2)],
            &definition_point,
        )),
        DimensionGeometry::Angular2Line {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
        } => {
            let vertex = intersection(line1_start, line1_end, line2_start, line2_end)?;
            let a1 = (line1_end.y - line1_start.y).atan2(line1_end.x - line1_start.x);
            let a2 = (line2_end.y - line2_start.y).atan2(line2_end.x - line2_start.x);
            // Each line contributes two rays from the vertex; the arc point
            // says which of the four sectors was dimensioned.
            let pi = std::f64::consts::PI;
            Some(sector_degrees(
                &vertex,
                &[a1, a1 + pi, a2, a2 + pi],
                &definition_point,
            ))
        }
        DimensionGeometry::Radius {
            center,
            chord_point,
            ..
        } => Some(distance(center, chord_point)),
        DimensionGeometry::Diameter {
            chord_start,
            chord_end,
            ..
        } => Some(distance(chord_start, chord_end)),
        DimensionGeometry::Ordinate {
            feature, x_datum, ..
        } => Some(if *x_datum {
            feature.x - definition_point.x
        } else {
            feature.y - definition_point.y
        }),
        DimensionGeometry::Arc {
            center,
            xline1,
            xline2,
        } => {
            let radius = distance(center, xline1);
            let sweep = sector_degrees(
                center,
                &[angle_of(center, xline1), angle_of(center, xline2)],
                &definition_point,
            );
            Some(radius * sweep.to_radians())
        }
        DimensionGeometry::Unknown => None,
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
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
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

/// Formats a measurement the way a basic dimension style would show it:
/// linear values times `DIMLFAC` in decimal (`DIMLUNIT` 2 and every code
/// this does not handle), architectural feet-inches (4) or fractional
/// inches (5) at `DIMDEC` precision, trailing zeros dropped when `DIMZIN`
/// has bit 8 set; angles as decimal degrees at `DIMADEC` precision with a
/// degree sign.
pub fn format_measurement(value: f64, angular: bool, style: &EffectiveStyle) -> String {
    if angular {
        return format!("{:.*}\u{00B0}", usize::from(style.dimadec), value);
    }
    let scaled = value * style.dimlfac;
    match style.dimlunit {
        4 => format_architectural(scaled, style.dimdec),
        5 => format_fractional(scaled, style.dimdec),
        _ => {
            let text = format!("{:.*}", usize::from(style.dimdec), scaled);
            if style.dimzin & 8 != 0 && text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                text
            }
        }
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
    let denominator: u64 = 1 << dimdec.min(8);
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

/// Fills `display_text`, `display_text_raw`, `display_source` and `dimlfac`
/// on every DIMENSION in `db` (top level and inside block definitions) from
/// the override text, the cached `*D` block label, or a formatted value.
pub(crate) fn attach_display_text(db: &mut CadDatabase) {
    let labels = cached_labels(db);
    let header = db.header.clone();
    let dimstyles = db.tables.dimstyles.clone();
    let resolve = |dim: &mut DimensionEntity| {
        let style = EffectiveStyle::resolve(dimstyles.get(&dim.dimstyle), &header);
        resolve_display(dim, &style, labels.get(&dim.block_name));
    };
    for entity in &mut db.entities {
        if let Entity::Dimension(dim) = entity {
            resolve(dim);
        }
    }
    for record in db.tables.block_records.values_mut() {
        for entity in &mut record.entities {
            if let Entity::Dimension(dim) = entity {
                resolve(dim);
            }
        }
    }
}

/// The text inside each block a DIMENSION could reference: `(raw, plain)`,
/// the TEXT/MTEXT entities of the block in order, joined by a space.
fn cached_labels(db: &CadDatabase) -> BTreeMap<String, (String, String)> {
    let mut labels = BTreeMap::new();
    for (name, record) in &db.tables.block_records {
        let mut raw = Vec::new();
        let mut plain = Vec::new();
        for entity in &record.entities {
            match entity {
                Entity::Text(t) => {
                    raw.push(t.text.clone());
                    plain.push(t.text_plain.clone());
                }
                Entity::MText(m) => {
                    raw.push(m.text.clone());
                    plain.push(m.text_plain.clone());
                }
                _ => {}
            }
        }
        if !plain.is_empty() {
            labels.insert(name.clone(), (raw.join(" "), plain.join(" ")));
        }
    }
    labels
}

/// The precedence described in the module doc.
pub(crate) fn resolve_display(
    dim: &mut DimensionEntity,
    style: &EffectiveStyle,
    cached: Option<&(String, String)>,
) {
    dim.dimlfac = style.dimlfac;
    let angular = dim.geometry.is_angular();
    let value = dim.measurement.or(dim.measurement_from_points);
    let formatted = value.map(|v| format_measurement(v, angular, style));

    if !dim.user_text.is_empty() && dim.user_text.trim().is_empty() {
        dim.display_text = String::new();
        dim.display_text_raw = dim.user_text.clone();
        dim.display_source = DisplaySource::Suppressed;
    } else if !dim.user_text.is_empty() {
        let substituted = dim
            .user_text
            .replace("<>", formatted.as_deref().unwrap_or(""));
        dim.display_text = crate::text::decode_mtext(&substituted).plain;
        dim.display_text_raw = dim.user_text.clone();
        dim.display_source = DisplaySource::UserText;
    } else if let Some((raw, plain)) = cached.filter(|(_, plain)| !plain.trim().is_empty()) {
        dim.display_text = plain.clone();
        dim.display_text_raw = raw.clone();
        dim.display_source = DisplaySource::CachedBlock;
    } else if let Some(formatted) = formatted {
        dim.display_text = formatted;
        dim.display_text_raw = String::new();
        dim.display_source = DisplaySource::Formatted;
    } else {
        dim.display_text = String::new();
        dim.display_text_raw = String::new();
        dim.display_source = DisplaySource::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    #[test]
    fn linear_projects_on_the_dimension_direction_and_aligned_does_not() {
        let geometry = DimensionGeometry::Linear {
            xline1: p(0.0, 0.0),
            xline2: p(3.0, 4.0),
            rotation: 0.0,
        };
        assert_eq!(measurement_from_points(&geometry, p(0.0, 0.0)), Some(3.0));
        let geometry = DimensionGeometry::Linear {
            xline1: p(0.0, 0.0),
            xline2: p(3.0, 4.0),
            rotation: std::f64::consts::FRAC_PI_2,
        };
        assert!((measurement_from_points(&geometry, p(0.0, 0.0)).unwrap() - 4.0).abs() < 1e-12);
        let geometry = DimensionGeometry::Aligned {
            xline1: p(0.0, 0.0),
            xline2: p(3.0, 4.0),
        };
        assert_eq!(measurement_from_points(&geometry, p(0.0, 0.0)), Some(5.0));
    }

    #[test]
    fn radius_and_diameter_are_the_chord_distances() {
        let radius = DimensionGeometry::Radius {
            center: p(0.0, 0.0),
            chord_point: p(0.0, 7.0),
            leader_length: 0.0,
        };
        assert_eq!(measurement_from_points(&radius, p(0.0, 0.0)), Some(7.0));
        let diameter = DimensionGeometry::Diameter {
            chord_start: p(-5.0, 0.0),
            chord_end: p(5.0, 0.0),
            leader_length: 0.0,
        };
        assert_eq!(measurement_from_points(&diameter, p(-5.0, 0.0)), Some(10.0));
    }

    #[test]
    fn angular_sectors_follow_the_arc_point() {
        let three_point = DimensionGeometry::Angular3Point {
            center: p(0.0, 0.0),
            xline1: p(10.0, 0.0),
            xline2: p(0.0, 10.0),
        };
        // The arc point inside the 90-degree sector, then in the reflex one.
        let inside = measurement_from_points(&three_point, p(5.0, 5.0)).unwrap();
        assert!((inside - 90.0).abs() < 1e-9, "{inside}");
        let reflex = measurement_from_points(&three_point, p(-5.0, -5.0)).unwrap();
        assert!((reflex - 270.0).abs() < 1e-9, "{reflex}");

        // Two lines crossing at 108 degrees (a 72-degree acute pair).
        let a = 108f64.to_radians();
        let two_line = DimensionGeometry::Angular2Line {
            line1_start: p(0.0, 0.0),
            line1_end: p(10.0, 0.0),
            line2_start: p(0.0, 0.0),
            line2_end: p(10.0 * a.cos(), 10.0 * a.sin()),
        };
        let probe = p((a / 2.0).cos() * 5.0, (a / 2.0).sin() * 5.0);
        let obtuse = measurement_from_points(&two_line, probe).unwrap();
        assert!((obtuse - 108.0).abs() < 1e-9, "{obtuse}");
        let probe = p(
            (a / 2.0 + std::f64::consts::PI).cos() * 5.0,
            (a / 2.0 + std::f64::consts::PI).sin() * 5.0,
        );
        let opposite = measurement_from_points(&two_line, probe).unwrap();
        assert!((opposite - 108.0).abs() < 1e-9, "{opposite}");
        let probe = p(-1.0, 3.0);
        let acute = measurement_from_points(&two_line, probe).unwrap();
        assert!((acute - 72.0).abs() < 1e-9, "{acute}");
    }

    #[test]
    fn ordinate_and_arc_length() {
        let ordinate = DimensionGeometry::Ordinate {
            feature: p(25.0, 40.0),
            leader_end: p(25.0, 60.0),
            x_datum: true,
        };
        assert_eq!(
            measurement_from_points(&ordinate, p(10.0, 10.0)),
            Some(15.0)
        );
        let arc = DimensionGeometry::Arc {
            center: p(0.0, 0.0),
            xline1: p(2.0, 0.0),
            xline2: p(0.0, 2.0),
        };
        let length = measurement_from_points(&arc, p(1.0, 1.0)).unwrap();
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
    fn display_precedence_suppressed_override_cache_formatted() {
        let base = DimensionEntity {
            common: crate::model::EntityCommon {
                handle: "1".into(),
                layer: "0".into(),
                color_index: 256,
                true_color: None,
            },
            block_name: "*D1".into(),
            geometry: DimensionGeometry::Aligned {
                xline1: p(0.0, 0.0),
                xline2: p(10.0, 0.0),
            },
            measurement: Some(10.0),
            measurement_from_points: Some(10.0),
            user_text: String::new(),
            display_text: String::new(),
            display_text_raw: String::new(),
            display_source: DisplaySource::None,
            definition_point: p(0.0, 0.0),
            text_midpoint: crate::model::Point2D { x: 5.0, y: 2.0 },
            dimstyle: String::new(),
            dimlfac: 1.0,
        };
        let cached = ("\\A1;120".to_string(), "120".to_string());

        let mut d = base.clone();
        d.user_text = " ".into();
        resolve_display(&mut d, &style(), Some(&cached));
        assert_eq!(
            (d.display_text.as_str(), d.display_source),
            ("", DisplaySource::Suppressed)
        );

        let mut d = base.clone();
        d.user_text = "<> TYP.".into();
        resolve_display(&mut d, &style(), Some(&cached));
        assert_eq!(d.display_text, "10.00 TYP.");
        assert_eq!(d.display_source, DisplaySource::UserText);

        let mut d = base.clone();
        resolve_display(&mut d, &style(), Some(&cached));
        assert_eq!(
            (d.display_text.as_str(), d.display_source),
            ("120", DisplaySource::CachedBlock)
        );
        assert_eq!(d.display_text_raw, "\\A1;120");

        let mut d = base.clone();
        resolve_display(&mut d, &style(), None);
        assert_eq!(
            (d.display_text.as_str(), d.display_source),
            ("10.00", DisplaySource::Formatted)
        );

        let mut d = base;
        d.measurement = None;
        d.measurement_from_points = None;
        resolve_display(&mut d, &style(), None);
        assert_eq!(d.display_source, DisplaySource::None);
    }
}
