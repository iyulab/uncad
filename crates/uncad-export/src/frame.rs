//! Where the package's pictures stop: the crop a caller asks for, as the
//! renderer's [`Crop`]; the padding around it; the snap of an image's size
//! to the model's patch lattice; and the detached groups of a drawing that
//! become frames of their own.
//!
//! The renderer decides the crop -- which entities a picture is framed on
//! and which few outliers it leaves out, and says so in its `CropReport`.
//! What is left here is what only a reader counting patches cares about:
//!
//! 1. **Crop mode** ([`CropMode`]): `Auto` is the renderer's guard, handed
//!    the header's `$EXTMIN/$EXTMAX` as the stated extent it may take
//!    instead; `Raw` frames everything; `Header` frames the header's
//!    extents when they are a sane rectangle; `Fixed` a caller's rectangle.
//! 2. **Padding** ([`auto_padding`]): 2 % of the longer side, or 24 output
//!    pixels when that is more.
//! 3. **Lattice snap** ([`snap_to_lattice`]): the pixel size is rounded up
//!    to a multiple of the patch size (28 px for Claude) and the world
//!    rectangle grows on the right and bottom so that pixels and units stay
//!    in exact proportion -- the affine the package publishes beside every
//!    image then round-trips.
//! 4. **Frames** ([`detached_groups`]): entities within about a cell of one
//!    another are one group; a detail drawn beside the plan is another.

use std::collections::BTreeMap;

use iron_render_cad::Crop;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An axis-aligned rectangle in world (drawing) units -- or, on a sheet, in
/// the layout's paper units.
///
/// JSON form: the `[x0, y0, x1, y1]` array the package documents for every
/// box, so a rectangle reads the same in `manifest.json`, `sheets.json`,
/// `report.json` and the records. Reading also accepts the `{min_x, min_y,
/// max_x, max_y}` object an earlier form of the package wrote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Serialize for Rect {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [self.min_x, self.min_y, self.max_x, self.max_y].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Rect, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Array([f64; 4]),
            Object {
                min_x: f64,
                min_y: f64,
                max_x: f64,
                max_y: f64,
            },
        }
        Ok(match Form::deserialize(deserializer)? {
            Form::Array([min_x, min_y, max_x, max_y]) => Rect::new(min_x, min_y, max_x, max_y),
            Form::Object {
                min_x,
                min_y,
                max_x,
                max_y,
            } => Rect::new(min_x, min_y, max_x, max_y),
        })
    }
}

impl From<iron_render_cad::Rect> for Rect {
    fn from(r: iron_render_cad::Rect) -> Rect {
        Rect::new(r.min_x, r.min_y, r.max_x, r.max_y)
    }
}

impl From<Rect> for iron_render_cad::Rect {
    fn from(r: Rect) -> iron_render_cad::Rect {
        iron_render_cad::Rect::new(r.min_x, r.min_y, r.max_x, r.max_y)
    }
}

impl Rect {
    pub const fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Rect {
        Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn longer_side(&self) -> f64 {
        self.width().max(self.height())
    }

    pub fn diagonal(&self) -> f64 {
        self.width().hypot(self.height())
    }

    /// Finite, no coordinate at or beyond 1e15 (the runaway values some
    /// headers hold), and a positive area -- the renderer's test for a
    /// stated extent it may take.
    pub fn is_sane(&self) -> bool {
        [self.min_x, self.min_y, self.max_x, self.max_y]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 1e15)
            && self.width() > 0.0
            && self.height() > 0.0
    }

    /// Whether the two overlap or touch.
    pub fn intersects(&self, other: &Rect) -> bool {
        other.min_x <= self.max_x
            && other.max_x >= self.min_x
            && other.min_y <= self.max_y
            && other.max_y >= self.min_y
    }

    /// Whether `other` lies entirely inside (edges included).
    pub fn contains(&self, other: &Rect) -> bool {
        other.min_x >= self.min_x
            && other.max_x <= self.max_x
            && other.min_y >= self.min_y
            && other.max_y <= self.max_y
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Grown by `pad` on every side.
    pub fn padded(&self, pad: f64) -> Rect {
        Rect {
            min_x: self.min_x - pad,
            min_y: self.min_y - pad,
            max_x: self.max_x + pad,
            max_y: self.max_y + pad,
        }
    }

    /// The bounding rectangle of several; `None` for none.
    pub fn bounding<'a>(rects: impl IntoIterator<Item = &'a Rect>) -> Option<Rect> {
        rects.into_iter().fold(None, |acc: Option<Rect>, r| {
            Some(acc.map_or(*r, |a| a.union(r)))
        })
    }

    /// A rectangle of one point.
    pub fn at(x: f64, y: f64) -> Rect {
        Rect::new(x, y, x, y)
    }
}

/// The canvas a package of an empty drawing gets.
pub const EMPTY_RECT: Rect = Rect::new(0.0, 0.0, 10.0, 10.0);

/// What the package's pictures show. `Auto` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CropMode {
    /// Every entity but the few outliers the renderer's guard sets aside
    /// (a block reference scaled thousands of times, a stray point a
    /// million units out), or the header's `$EXTMIN/$EXTMAX` when those
    /// pass the renderer's test for a stated extent and cover more.
    #[default]
    Auto,
    /// Every entity, outliers included: the raw bounds.
    Raw,
    /// `$EXTMIN/$EXTMAX` as stored, when they are a sane rectangle; `Auto`
    /// otherwise.
    Header,
    /// Exactly this world rectangle.
    Fixed(Rect),
}

/// Which rule produced the crop, as the manifest names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropSource {
    /// The entities' extents, outliers set aside.
    Content,
    /// The header's `$EXTMIN/$EXTMAX`.
    Header,
    /// Every entity's extent.
    Raw,
    /// A rectangle the caller gave.
    Fixed,
    /// Nothing measured: a 10 x 10 canvas at the origin ([`EMPTY_RECT`]).
    Empty,
}

/// The header's `$EXTMIN/$EXTMAX` as a rectangle, when both are stated and
/// they make a sane one (see [`Rect::is_sane`]); AutoCAD writes `1e20`
/// when it never computed them.
pub fn header_extents(header: &uncad::Header) -> Option<Rect> {
    let (min, max) = (header.extmin?, header.extmax?);
    let rect = Rect::new(min.x, min.y, max.x, max.y);
    rect.is_sane().then_some(rect)
}

/// A crop mode as the renderer's [`Crop`], with the header's extents (see
/// [`header_extents`]) as the stated extent `Auto` may take.
pub fn renderer_crop(mode: CropMode, header: Option<Rect>) -> Crop {
    let stated = header.map(iron_render_cad::Rect::from);
    match mode {
        CropMode::Auto => Crop::Guarded { stated },
        CropMode::Raw => Crop::Everything,
        CropMode::Header => match stated {
            Some(rect) => Crop::Window(rect),
            None => Crop::Guarded { stated: None },
        },
        CropMode::Fixed(rect) => Crop::Window(rect.into()),
    }
}

/// Which rule a render under `mode` framed its picture by: `stated_taken`
/// and `content` are the renderer's report of it.
pub fn crop_source(
    mode: CropMode,
    header: Option<Rect>,
    stated_taken: bool,
    content: Option<&Rect>,
) -> CropSource {
    if content.is_none() {
        return CropSource::Empty;
    }
    match mode {
        CropMode::Raw => CropSource::Raw,
        CropMode::Fixed(_) => CropSource::Fixed,
        CropMode::Header if header.is_some() => CropSource::Header,
        CropMode::Header | CropMode::Auto if stated_taken => CropSource::Header,
        CropMode::Header | CropMode::Auto => CropSource::Content,
    }
}

/// Rule 2: the padding for `rect`, in drawing units: 2 % of its longer
/// side, or 24 pixels at `px_per_unit` when that is more. A degenerate
/// (zero-size) rectangle gets half a unit, or those 24 pixels when they are
/// more -- never less, however large the scale: a scale seeded from a
/// zero-size rectangle is itself enormous, and 24 pixels of it came to
/// 1e-11 units, a blank window at 1e13 px/unit.
pub fn auto_padding(rect: &Rect, px_per_unit: Option<f64>) -> f64 {
    let l = rect.longer_side();
    let by_pixels = px_per_unit
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| 24.0 / s)
        .unwrap_or(0.0);
    if l.is_nan() || l <= 0.0 {
        return by_pixels.max(0.5);
    }
    (0.02 * l).max(by_pixels)
}

/// Rule 3: the pixel size of `padded` at `px_per_unit`, rounded up to a
/// multiple of `lattice` (0 = plain rounding), and the world rectangle
/// grown on the right and bottom so that `width / px_per_unit` and
/// `height / px_per_unit` equal its size exactly. Returns `(rect, width_px,
/// height_px)`; a size beyond `u32::MAX` is clamped (the renderer refuses
/// anything past its own edge limit anyway).
pub fn snap_to_lattice(padded: &Rect, px_per_unit: f64, lattice: u32) -> (Rect, u32, u32) {
    let to_px = |units: f64| -> f64 {
        let raw = units * px_per_unit;
        let px = if lattice > 0 {
            let l = f64::from(lattice);
            (raw / l - 1e-9).ceil() * l
        } else {
            raw.round()
        };
        if px.is_finite() {
            px.max(1.0).min(f64::from(u32::MAX))
        } else {
            f64::from(u32::MAX)
        }
    };
    let w = to_px(padded.width());
    let h = to_px(padded.height());
    let rect = Rect {
        min_x: padded.min_x,
        min_y: padded.max_y - h / px_per_unit,
        max_x: padded.min_x + w / px_per_unit,
        max_y: padded.max_y,
    };
    (rect, w as u32, h as u32)
}

/// Rule 4: groups the extents that lie within about `cell` of one another
/// (rectangles are painted onto a grid of `cell`-sized squares; entities
/// sharing a square or neighbouring squares are one group). Groups come
/// back largest first (then by position), each with its indices in order.
/// The package turns detached groups into frames.
pub fn detached_groups(extents: &[Rect], cell: f64) -> Vec<Vec<usize>> {
    let n = extents.len();
    if n == 0 {
        return Vec::new();
    }
    let cell = if cell.is_finite() && cell > 0.0 {
        cell
    } else {
        1.0
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
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    // The first entity to touch a square owns it; later ones join it.
    let mut owner: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    let key = |v: f64| (v / cell).floor() as i64;
    for (i, r) in extents.iter().enumerate() {
        if !(r.min_x.is_finite()
            && r.max_x.is_finite()
            && r.min_y.is_finite()
            && r.max_y.is_finite())
        {
            continue;
        }
        let (x0, x1, y0, y1) = (key(r.min_x), key(r.max_x), key(r.min_y), key(r.max_y));
        // A rectangle spanning an absurd number of squares (a runaway
        // extent that slipped through) only paints its four corners.
        let too_many = (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > 1 << 20;
        let mut paint = |x: i64, y: i64| match owner.get(&(x, y)) {
            Some(&o) => union(&mut parent, i, o),
            None => {
                owner.insert((x, y), i);
            }
        };
        if too_many {
            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                paint(x, y);
            }
        } else {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    paint(x, y);
                }
            }
        }
    }
    // Neighbouring squares join too (8-connectivity), so the effective gap
    // is between one and two cells.
    for (&(x, y), &i) in &owner {
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                if let Some(&j) = owner.get(&(x + dx, y + dy)) {
                    union(&mut parent, i, j);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    let rect_of = |g: &[usize]| Rect::bounding(g.iter().map(|i| &extents[*i])).expect("non-empty");
    out.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| rect_of(a).min_x.total_cmp(&rect_of(b).min_x))
            .then_with(|| rect_of(a).min_y.total_cmp(&rect_of(b).min_y))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_groups_split_a_plan_from_its_details() {
        // A 100 x 50 "plan" of four lines, a detail of three lines 30 units
        // away, and a lone dot far off. With 5-unit cells the plan and the
        // detail are separate; with 40-unit cells they merge.
        let mut extents = vec![
            Rect::new(0.0, 0.0, 100.0, 0.0),
            Rect::new(100.0, 0.0, 100.0, 50.0),
            Rect::new(0.0, 50.0, 100.0, 50.0),
            Rect::new(0.0, 0.0, 0.0, 50.0),
            Rect::new(130.0, 0.0, 150.0, 0.0),
            Rect::new(150.0, 0.0, 150.0, 20.0),
            Rect::new(130.0, 20.0, 150.0, 20.0),
            Rect::new(500.0, 500.0, 500.0, 500.0),
        ];
        let groups = detached_groups(&extents, 5.0);
        assert_eq!(groups, vec![vec![0, 1, 2, 3], vec![4, 5, 6], vec![7]]);
        let merged = detached_groups(&extents, 40.0);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].len(), 7);
        // The empty interior of the plan is no gap: opposite walls meet at
        // the corners.
        extents.truncate(4);
        assert_eq!(detached_groups(&extents, 1.0).len(), 1);
        assert!(detached_groups(&[], 1.0).is_empty());
        // A runaway extent paints only its corners, and still groups.
        let runaway = [
            Rect::new(0.0, 0.0, 1e12, 1e12),
            Rect::new(0.0, 0.0, 1.0, 1.0),
        ];
        assert_eq!(detached_groups(&runaway, 1.0).len(), 1);
    }

    #[test]
    fn padding_and_lattice() {
        let rect = Rect::new(0.0, 0.0, 1000.0, 500.0);
        assert_eq!(auto_padding(&rect, None), 20.0);
        // 24 px at 0.5 px/unit is 48 units, more than 2 %.
        assert_eq!(auto_padding(&rect, Some(0.5)), 48.0);
        assert_eq!(auto_padding(&rect, Some(10.0)), 20.0);
        let point = Rect::at(3.0, 3.0);
        assert_eq!(auto_padding(&point, None), 0.5);
        assert_eq!(auto_padding(&point, Some(2.0)), 12.0);
        // A scale seeded from the same zero-size rectangle is ~1e12, which
        // would make 24 px a 2e-11-unit window: half a unit is the floor.
        assert_eq!(auto_padding(&point, Some(1.05e12)), 0.5);
        assert_eq!(auto_padding(&point, Some(f64::INFINITY)), 0.5);
        assert_eq!(auto_padding(&point, Some(0.0)), 0.5, "no scale at all");

        // 1040 x 540 units at 1.5 px/unit = 1560 x 810 px, snapped up to
        // 1568 x 812; the world rectangle grows to 1045.333.. x 541.333..
        let padded = rect.padded(20.0);
        let (snapped, w, h) = snap_to_lattice(&padded, 1.5, 28);
        assert_eq!((w, h), (1568, 812));
        assert!((snapped.width() * 1.5 - 1568.0).abs() < 1e-9);
        assert!((snapped.height() * 1.5 - 812.0).abs() < 1e-9);
        assert_eq!((snapped.min_x, snapped.max_y), (-20.0, 520.0));
        assert!(snapped.max_x > padded.max_x && snapped.min_y < padded.min_y);
        // Without a lattice: plain rounding, still exact.
        let (plain, w, h) = snap_to_lattice(&padded, 1.5, 0);
        assert_eq!((w, h), (1560, 810));
        assert_eq!(plain, padded);
        // An exact multiple is not bumped up.
        let (_, w, _) = snap_to_lattice(&Rect::new(0.0, 0.0, 28.0, 28.0), 1.0, 28);
        assert_eq!(w, 28);
    }

    #[test]
    fn every_crop_mode_is_a_renderer_crop_and_names_its_source() {
        let header = Some(Rect::new(-1.0, -1.0, 11.0, 11.0));
        let content = Rect::new(0.0, 0.0, 10.0, 10.0);
        let stated = iron_render_cad::Rect::new(-1.0, -1.0, 11.0, 11.0);
        assert_eq!(
            renderer_crop(CropMode::Auto, header),
            Crop::Guarded {
                stated: Some(stated)
            }
        );
        assert_eq!(
            renderer_crop(CropMode::Auto, None),
            Crop::Guarded { stated: None }
        );
        assert_eq!(renderer_crop(CropMode::Raw, header), Crop::Everything);
        assert_eq!(
            renderer_crop(CropMode::Header, header),
            Crop::Window(stated)
        );
        // No usable header: `Header` is `Auto`.
        assert_eq!(
            renderer_crop(CropMode::Header, None),
            Crop::Guarded { stated: None }
        );
        let fixed = Rect::new(0.0, 0.0, 5.0, 5.0);
        assert_eq!(
            renderer_crop(CropMode::Fixed(fixed), header),
            Crop::Window(fixed.into())
        );

        let source = |mode, header, taken, content| crop_source(mode, header, taken, content);
        assert_eq!(
            source(CropMode::Auto, header, false, Some(&content)),
            CropSource::Content
        );
        assert_eq!(
            source(CropMode::Auto, header, true, Some(&content)),
            CropSource::Header
        );
        assert_eq!(
            source(CropMode::Header, header, false, Some(&content)),
            CropSource::Header
        );
        assert_eq!(
            source(CropMode::Header, None, false, Some(&content)),
            CropSource::Content
        );
        assert_eq!(
            source(CropMode::Raw, header, false, Some(&content)),
            CropSource::Raw
        );
        assert_eq!(
            source(CropMode::Fixed(fixed), header, false, Some(&content)),
            CropSource::Fixed
        );
        assert_eq!(
            source(CropMode::Auto, header, false, None),
            CropSource::Empty
        );
    }

    #[test]
    fn a_rect_is_the_documented_array_and_reads_both_forms() {
        let rect = Rect::new(-6.35, -6.35, 273.05, 209.55);
        assert_eq!(
            serde_json::to_string(&rect).unwrap(),
            "[-6.35,-6.35,273.05,209.55]"
        );
        assert_eq!(
            serde_json::from_str::<Rect>("[-6.35,-6.35,273.05,209.55]").unwrap(),
            rect
        );
        let object = r#"{"min_x":-6.35,"min_y":-6.35,"max_x":273.05,"max_y":209.55}"#;
        assert_eq!(serde_json::from_str::<Rect>(object).unwrap(), rect);
        // And the renderer's rectangle converts both ways unchanged.
        let theirs: iron_render_cad::Rect = rect.into();
        assert_eq!(Rect::from(theirs), rect);
        // Sanity is the renderer's test for a stated extent.
        assert!(rect.is_sane());
        assert!(!Rect::new(1e20, 0.0, 1e21, 1.0).is_sane());
        assert!(!Rect::at(5.0, 5.0).is_sane());
    }
}
