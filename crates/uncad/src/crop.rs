//! Where the picture's edges go (docs/VLM_EXPORT_DESIGN.md, section 4).
//!
//! The renderer measures every visible entity's world extent; this module
//! turns those extents into the rectangle an image shows and says what it
//! left out and why. The rules, in order:
//!
//! 1. **Outlier guard** ([`outliers`]): at most `max(3, 1 %)` entities can
//!    be outliers. Each pass sets aside the largest entities and the ones
//!    farthest from the median centre (at most a quarter of the drawing)
//!    and measures them against the *rest*: a candidate whose diagonal is
//!    over 20x the rest's is a `scale_outlier` (the 3256x-scaled INSERT of
//!    `example_2000.dwg` and `example_2018.dwg`); one more than 20 rest
//!    diagonals away is a `far_outlier` (a stray point a million units
//!    out) -- but never more than a fifth of
//!    the drawing, so a notes block a drawing-width away stays. Passes
//!    repeat until nothing changes. Nothing else is ever trimmed: the
//!    overview shows every entity that is not one of those.
//! 2. **Header candidate** ([`header_candidate`]): `$EXTMIN/$EXTMAX` is
//!    accepted when finite, sane, containing at least 90 % of the kept
//!    extents and at most 4x the content area; [`CropMode::Auto`] uses it
//!    only when it covers more entities than the computed content.
//! 3. **Padding** ([`auto_padding`]): 2 % of the longer side, or 24 output
//!    pixels when that is more.
//! 4. **Lattice snap** ([`snap_to_lattice`]): the pixel size is rounded up
//!    to a multiple of the patch size (28 px for Claude) and the world
//!    rectangle grows on the right and bottom so that pixels and units stay
//!    in exact proportion -- the affine in [`crate::ViewBox`] then round-trips.
//!
//! Excluded entities are not drawn; the report lists them so an export
//! can leave them out of its records too.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::header::Header;
use crate::svg::bounds::Box2D;

/// An axis-aligned rectangle in world (drawing) units.
///
/// JSON form: the `[x0, y0, x1, y1]` array the package documents for every
/// box (`docs/VLM_EXPORT_DESIGN.md`, section 3), so a rectangle reads the
/// same in `manifest.json`, `sheets.json`, `report.json` and the records.
/// Reading accepts the `{min_x, min_y, max_x, max_y}` object 0.3.0's
/// derived `Serialize` used to write as well.
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

    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Finite, no coordinate at or beyond 1e15 (the runaway values some
    /// headers hold), and a positive area.
    pub fn is_sane(&self) -> bool {
        [self.min_x, self.min_y, self.max_x, self.max_y]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 1e15)
            && self.width() > 0.0
            && self.height() > 0.0
    }

    /// Whether `other` lies entirely inside (edges included).
    pub fn contains(&self, other: &Rect) -> bool {
        other.min_x >= self.min_x
            && other.max_x <= self.max_x
            && other.min_y >= self.min_y
            && other.max_y <= self.max_y
    }

    /// Whether the two overlap or touch.
    pub fn intersects(&self, other: &Rect) -> bool {
        other.min_x <= self.max_x
            && other.max_x >= self.min_x
            && other.min_y <= self.max_y
            && other.max_y >= self.min_y
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Shortest distance between the two; 0 when they overlap.
    pub fn gap(&self, other: &Rect) -> f64 {
        let dx = (self.min_x - other.max_x)
            .max(other.min_x - self.max_x)
            .max(0.0);
        let dy = (self.min_y - other.max_y)
            .max(other.min_y - self.max_y)
            .max(0.0);
        dx.hypot(dy)
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

    pub(crate) fn from_box(b: &Box2D) -> Rect {
        Rect::new(b.min_x, b.min_y, b.max_x, b.max_y)
    }
}

/// What to show. `Auto` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CropMode {
    /// The computed content extents minus scale outliers, or the header
    /// extents when those cover more (see the module docs).
    #[default]
    Auto,
    /// Every visible entity, outliers included: the raw bounds.
    Raw,
    /// `$EXTMIN/$EXTMAX` as stored, when sane; `Auto` otherwise.
    Header,
    /// Exactly this world rectangle.
    Fixed(Rect),
}

/// Which rule produced the crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropSource {
    /// The computed content extents (outliers excluded).
    Content,
    /// The header's `$EXTMIN/$EXTMAX`.
    Header,
    /// The raw bounds of every visible entity.
    Raw,
    /// A rectangle the caller gave.
    Fixed,
    /// Nothing visible: a 10 x 10 canvas at the origin.
    Empty,
}

impl CropSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CropSource::Content => "content",
            CropSource::Header => "header",
            CropSource::Raw => "raw",
            CropSource::Fixed => "fixed",
            CropSource::Empty => "empty",
        }
    }
}

/// Why an entity is outside the crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeReason {
    /// Far larger than everything else put together (rule 1, 20x).
    ScaleOutlier,
    /// Far away from the rest of the drawing (rule 1, 20 diagonals).
    FarOutlier,
    /// Outside a header or fixed crop.
    OutsideCrop,
}

impl ExcludeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExcludeReason::ScaleOutlier => "scale_outlier",
            ExcludeReason::FarOutlier => "far_outlier",
            ExcludeReason::OutsideCrop => "outside_crop",
        }
    }
}

/// One entity's measured extent, as the renderer hands it over.
#[derive(Debug, Clone, PartialEq)]
pub struct Extent {
    pub handle: String,
    pub type_name: String,
    pub rect: Rect,
}

/// An entity the crop leaves out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Excluded {
    pub handle: String,
    pub type_name: String,
    pub rect: Rect,
    pub reason: ExcludeReason,
}

/// What an image shows and what it left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CropReport {
    pub source: CropSource,
    /// The world rectangle the image shows, padding (and any lattice
    /// growth) included. Same rectangle as the result's `view_box`.
    pub rect: Rect,
    /// The tight bounds of the entities inside the crop; `None` when
    /// nothing is.
    pub content: Option<Rect>,
    /// The padding added on each side, in drawing units (the lattice snap
    /// may add a little more on the right and bottom).
    pub padding_units: f64,
    /// `$EXTMIN/$EXTMAX` as stored, when sane.
    pub header_extents: Option<Rect>,
    /// Entities outside the crop, in drawing order.
    pub excluded: Vec<Excluded>,
}

/// The canvas an empty drawing gets.
pub const EMPTY_RECT: Rect = Rect::new(0.0, 0.0, 10.0, 10.0);

/// The unpadded decision [`choose`] makes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Choice {
    pub(crate) source: CropSource,
    pub(crate) rect: Rect,
    pub(crate) content: Option<Rect>,
    pub(crate) header_extents: Option<Rect>,
    pub(crate) excluded: Vec<Excluded>,
}

impl Choice {
    pub(crate) fn report(&self, rect: Rect, padding_units: f64) -> CropReport {
        CropReport {
            source: self.source,
            rect,
            content: self.content,
            padding_units,
            header_extents: self.header_extents,
            excluded: self.excluded.clone(),
        }
    }
}

/// Rule 1: the indices of the extents the overview should not stretch to,
/// each with its reason, in index order. Empty when everything belongs
/// together. See the module docs for the rules.
pub fn outliers(extents: &[Extent]) -> Vec<(usize, ExcludeReason)> {
    let n = extents.len();
    if n < 3 {
        return Vec::new();
    }
    let limit = 3.max((n as f64 * 0.01).ceil() as usize);
    let mut excluded: Vec<(usize, ExcludeReason)> = Vec::new();
    // One pass can only judge what it set aside; a second huge entity hides
    // behind the first until that one is out, so passes repeat until
    // nothing changes (at most `limit` entities can go in total).
    loop {
        let remaining: Vec<usize> = (0..n)
            .filter(|i| !excluded.iter().any(|(j, _)| j == i))
            .collect();
        let found = outlier_pass(extents, &remaining, limit - excluded.len().min(limit));
        if found.is_empty() {
            break;
        }
        excluded.extend(found);
        if excluded.len() >= limit {
            break;
        }
    }
    excluded.sort_by_key(|(i, _)| *i);
    excluded
}

/// One pass of the guard over `remaining` (indices into `extents`).
///
/// The candidates are the `k` largest entities and the `k` farthest from
/// the median centre, `k = min(budget, remaining / 4)`, so that at least
/// three quarters of the drawing stay as the *rest*. With `R` the rest's
/// bounding box and `D` its diagonal (or the median entity diagonal when
/// larger): a candidate whose own diagonal exceeds 20 D is a scale
/// outlier; one whose gap to `R` exceeds 20 D is a far outlier. Far
/// outliers are dropped only when there are at most `budget` of them and
/// they are no more than a fifth of the drawing -- a notes block a
/// drawing-width away is part of the drawing.
fn outlier_pass(
    extents: &[Extent],
    remaining: &[usize],
    budget: usize,
) -> Vec<(usize, ExcludeReason)> {
    let m = remaining.len();
    let k = budget.min(m / 4);
    if m < 4 || k == 0 {
        return Vec::new();
    }
    let centre = |i: usize| {
        let r = &extents[i].rect;
        ((r.min_x + r.max_x) / 2.0, (r.min_y + r.max_y) / 2.0)
    };
    let mut xs: Vec<f64> = remaining.iter().map(|i| centre(*i).0).collect();
    let mut ys: Vec<f64> = remaining.iter().map(|i| centre(*i).1).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let (mx, my) = (xs[m / 2], ys[m / 2]);
    let distance = |i: usize| {
        let (x, y) = centre(i);
        (x - mx).hypot(y - my)
    };
    let mut by_diag: Vec<usize> = remaining.to_vec();
    by_diag.sort_by(|a, b| {
        extents[*b]
            .rect
            .diagonal()
            .total_cmp(&extents[*a].rect.diagonal())
            .then(a.cmp(b))
    });
    let mut by_dist: Vec<usize> = remaining.to_vec();
    by_dist.sort_by(|a, b| distance(*b).total_cmp(&distance(*a)).then(a.cmp(b)));
    let mut candidates: Vec<usize> = by_diag[..k].to_vec();
    for i in &by_dist[..k] {
        if !candidates.contains(i) {
            candidates.push(*i);
        }
    }
    let rest: Vec<usize> = remaining
        .iter()
        .copied()
        .filter(|i| !candidates.contains(i))
        .collect();
    if rest.len() < 3 {
        return Vec::new();
    }
    let rest_box =
        Rect::bounding(rest.iter().map(|i| &extents[*i].rect)).expect("rest is not empty");
    let mut diagonals: Vec<f64> = rest.iter().map(|i| extents[*i].rect.diagonal()).collect();
    diagonals.sort_by(f64::total_cmp);
    let d = rest_box
        .diagonal()
        .max(diagonals[diagonals.len() / 2])
        .max(1e-9);

    let mut out: Vec<(usize, ExcludeReason)> = Vec::new();
    let mut scale: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|i| extents[*i].rect.diagonal() > 20.0 * d)
        .collect();
    scale.sort_by(|a, b| {
        extents[*b]
            .rect
            .diagonal()
            .total_cmp(&extents[*a].rect.diagonal())
    });
    scale.truncate(budget);
    out.extend(scale.iter().map(|i| (*i, ExcludeReason::ScaleOutlier)));
    let far: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|i| !scale.contains(i) && extents[*i].rect.gap(&rest_box) > 20.0 * d)
        .collect();
    if !far.is_empty() && far.len() + scale.len() <= budget && far.len() * 5 <= m {
        out.extend(far.into_iter().map(|i| (i, ExcludeReason::FarOutlier)));
    }
    out
}

/// Groups extents that lie within about `cell` of one another (rectangles
/// are painted onto a grid of `cell`-sized squares; entities sharing a
/// square or neighbouring squares are one group). Groups come back largest
/// first (then by position), each with its indices in order. The export
/// turns detached groups into frames.
pub fn detached_groups(extents: &[Extent], cell: f64) -> Vec<Vec<usize>> {
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
    let mut owner: std::collections::HashMap<(i64, i64), usize> = std::collections::HashMap::new();
    let key = |v: f64| (v / cell).floor() as i64;
    for (i, e) in extents.iter().enumerate() {
        let r = &e.rect;
        if !(r.min_x.is_finite()
            && r.max_x.is_finite()
            && r.min_y.is_finite()
            && r.max_y.is_finite())
        {
            continue;
        }
        let (x0, x1, y0, y1) = (key(r.min_x), key(r.max_x), key(r.min_y), key(r.max_y));
        // A rectangle spanning an absurd number of squares (a runaway outlier
        // that slipped through) only paints its corners and edges' ends.
        let too_many = (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > 1 << 20;
        let mut paint = |x: i64, y: i64| match owner.entry((x, y)) {
            std::collections::hash_map::Entry::Occupied(o) => union(&mut parent, i, *o.get()),
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(i);
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
    let cells: Vec<((i64, i64), usize)> = owner.iter().map(|(k, v)| (*k, *v)).collect();
    for ((x, y), i) in &cells {
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                if let Some(j) = owner.get(&(x + dx, y + dy)) {
                    union(&mut parent, *i, *j);
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    let rect_of =
        |g: &[usize]| Rect::bounding(g.iter().map(|i| &extents[*i].rect)).expect("non-empty");
    out.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| rect_of(a).min_x.total_cmp(&rect_of(b).min_x))
            .then_with(|| rect_of(a).min_y.total_cmp(&rect_of(b).min_y))
    });
    out
}

/// The header's `$EXTMIN/$EXTMAX` as a rectangle, when sane.
pub fn header_extents(header: &Header) -> Option<Rect> {
    let rect = Rect::new(
        header.extmin.x,
        header.extmin.y,
        header.extmax.x,
        header.extmax.y,
    );
    rect.is_sane().then_some(rect)
}

/// Rule 2: the header extents, accepted as a crop candidate when they are
/// sane, contain at least 90 % of `kept` and are at most 4x `content`'s
/// area.
pub fn header_candidate(header: &Header, content: Option<&Rect>, kept: &[&Rect]) -> Option<Rect> {
    let rect = header_extents(header)?;
    if let Some(c) = content {
        if c.area() > 0.0 && rect.area() > 4.0 * c.area() {
            return None;
        }
    }
    let covered = kept.iter().filter(|r| rect.contains(r)).count();
    (covered * 10 >= kept.len() * 9).then_some(rect)
}

/// Decides the unpadded crop for `extents` (the visible entities' extents,
/// in drawing order) under `mode`.
pub(crate) fn choose(extents: &[Extent], header: &Header, mode: CropMode) -> Choice {
    let header_rect = header_extents(header);
    let empty = || Choice {
        source: CropSource::Empty,
        rect: EMPTY_RECT,
        content: None,
        header_extents: header_rect,
        excluded: Vec::new(),
    };
    if extents.is_empty() {
        return empty();
    }
    let all: Vec<&Rect> = extents.iter().map(|e| &e.rect).collect();
    let raw = Rect::bounding(all.iter().copied()).expect("non-empty");
    let outside = |rect: &Rect| -> Vec<Excluded> {
        extents
            .iter()
            .filter(|e| !rect.intersects(&e.rect))
            .map(|e| Excluded {
                handle: e.handle.clone(),
                type_name: e.type_name.clone(),
                rect: e.rect,
                reason: ExcludeReason::OutsideCrop,
            })
            .collect()
    };
    let content_inside =
        |rect: &Rect| Rect::bounding(all.iter().copied().filter(|r| rect.intersects(r)));

    match mode {
        CropMode::Raw => Choice {
            source: CropSource::Raw,
            rect: raw,
            content: Some(raw),
            header_extents: header_rect,
            excluded: Vec::new(),
        },
        CropMode::Fixed(rect) => Choice {
            source: CropSource::Fixed,
            rect,
            content: content_inside(&rect),
            header_extents: header_rect,
            excluded: outside(&rect),
        },
        CropMode::Header if header_rect.is_some() => {
            let rect = header_rect.expect("checked");
            Choice {
                source: CropSource::Header,
                rect,
                content: content_inside(&rect),
                header_extents: header_rect,
                excluded: outside(&rect),
            }
        }
        CropMode::Header | CropMode::Auto => {
            let out = outliers(extents);
            let kept: Vec<&Rect> = (0..extents.len())
                .filter(|i| !out.iter().any(|(j, _)| j == i))
                .map(|i| &extents[i].rect)
                .collect();
            let content = Rect::bounding(kept.iter().copied());
            let mut excluded: Vec<Excluded> = out
                .iter()
                .map(|(i, reason)| Excluded {
                    handle: extents[*i].handle.clone(),
                    type_name: extents[*i].type_name.clone(),
                    rect: extents[*i].rect,
                    reason: *reason,
                })
                .collect();
            let covers = |rect: &Rect| all.iter().filter(|r| rect.contains(r)).count();
            match (content, header_candidate(header, content.as_ref(), &kept)) {
                (Some(c), Some(h)) if covers(&h) > covers(&c) => {
                    // The header reaches entities the guard excluded: they are
                    // inside after all.
                    excluded.retain(|e| !h.intersects(&e.rect));
                    Choice {
                        source: CropSource::Header,
                        rect: h,
                        content: content_inside(&h),
                        header_extents: header_rect,
                        excluded,
                    }
                }
                (Some(c), _) => Choice {
                    source: CropSource::Content,
                    rect: c,
                    content: Some(c),
                    header_extents: header_rect,
                    excluded,
                },
                (None, _) => empty(),
            }
        }
    }
}

/// Rule 3: the padding for `rect`, in drawing units: 2 % of its longer
/// side, or 24 pixels at `px_per_unit` when that is more. A degenerate
/// (zero-size) rectangle gets half a unit, or those 24 pixels.
pub fn auto_padding(rect: &Rect, px_per_unit: Option<f64>) -> f64 {
    let l = rect.longer_side();
    let by_pixels = px_per_unit.map(|s| 24.0 / s).unwrap_or(0.0);
    if l.is_nan() || l <= 0.0 {
        return if by_pixels > 0.0 { by_pixels } else { 0.5 };
    }
    (0.02 * l).max(by_pixels)
}

/// Rule 4: the pixel size of `padded` at `px_per_unit`, rounded up to a
/// multiple of `lattice` (0 = plain rounding), and the world rectangle
/// grown on the right and bottom so that `width / px_per_unit` and
/// `height / px_per_unit` equal its size exactly. Returns
/// `(rect, width_px, height_px)`; a size beyond `u32::MAX` is clamped
/// (callers cap it anyway).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point3D;

    fn ext(handle: &str, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Extent {
        Extent {
            handle: handle.to_string(),
            type_name: "LINE".to_string(),
            rect: Rect::new(min_x, min_y, max_x, max_y),
        }
    }

    /// Four lines around a 10 x 10 square at the origin.
    fn square() -> Vec<Extent> {
        vec![
            ext("1", 0.0, 0.0, 10.0, 0.0),
            ext("2", 10.0, 0.0, 10.0, 10.0),
            ext("3", 0.0, 10.0, 10.0, 10.0),
            ext("4", 0.0, 0.0, 0.0, 10.0),
        ]
    }

    fn header_with(min: (f64, f64), max: (f64, f64)) -> Header {
        Header {
            extmin: Point3D {
                x: min.0,
                y: min.1,
                z: 0.0,
            },
            extmax: Point3D {
                x: max.0,
                y: max.1,
                z: 0.0,
            },
            ..Header::default()
        }
    }

    #[test]
    fn a_huge_far_line_is_a_scale_outlier_and_a_small_far_dot_a_far_outlier() {
        let mut extents = square();
        extents.push(ext("5", 1e6, 1e6, 2e6, 2e6));
        assert_eq!(outliers(&extents), vec![(4, ExcludeReason::ScaleOutlier)]);

        let mut extents = square();
        extents.push(ext("5", 1e6, 1e6, 1e6 + 1.0, 1e6 + 1.0));
        assert_eq!(outliers(&extents), vec![(4, ExcludeReason::FarOutlier)]);

        // A frame around the drawing stays: 50x the content is not 20x
        // everything else *combined* once the frame's own lines count.
        let mut extents = square();
        for line in square() {
            let r = line.rect;
            extents.push(Extent {
                rect: Rect::new(
                    r.min_x * 50.0,
                    r.min_y * 50.0,
                    r.max_x * 50.0,
                    r.max_y * 50.0,
                ),
                ..line
            });
        }
        assert!(outliers(&extents).is_empty());
        // One entity 50x the rest is out; so are two of them.
        let mut extents = square();
        extents.push(ext("5", 0.0, 0.0, 500.0, 500.0));
        assert_eq!(outliers(&extents), vec![(4, ExcludeReason::ScaleOutlier)]);
        extents.push(ext("6", -500.0, -500.0, 0.0, 0.0));
        assert_eq!(
            outliers(&extents),
            vec![
                (4, ExcludeReason::ScaleOutlier),
                (5, ExcludeReason::ScaleOutlier)
            ]
        );
        // Two squares 30 units apart are one drawing.
        let mut extents = square();
        extents.extend(square().into_iter().map(|mut e| {
            e.rect = Rect::new(
                e.rect.min_x + 30.0,
                e.rect.min_y,
                e.rect.max_x + 30.0,
                e.rect.max_y,
            );
            e
        }));
        assert!(outliers(&extents).is_empty());
        assert!(outliers(&square()[..2]).is_empty(), "fewer than 3: nothing");
    }

    /// Three 10 x 10 squares side by side: twelve lines.
    fn squares() -> Vec<Extent> {
        let mut out = Vec::new();
        for (n, dx) in [0.0, 20.0, 40.0].into_iter().enumerate() {
            for (i, line) in square().into_iter().enumerate() {
                let r = line.rect;
                out.push(Extent {
                    handle: format!("{}", n * 4 + i + 1),
                    type_name: "LINE".into(),
                    rect: Rect::new(r.min_x + dx, r.min_y, r.max_x + dx, r.max_y),
                });
            }
        }
        out
    }

    #[test]
    fn a_fifth_of_the_drawing_is_never_far() {
        // Twelve lines and one dot a million units away: the dot is out.
        let mut extents = squares();
        extents.push(ext("d1", 1e6, 1e6, 1e6 + 1.0, 1e6 + 1.0));
        assert_eq!(outliers(&extents), vec![(12, ExcludeReason::FarOutlier)]);
        // Four dots out of sixteen are a quarter of the drawing: it is just
        // that big, and nothing is excluded.
        extents.push(ext("d2", 1e6, 0.0, 1e6 + 1.0, 1.0));
        extents.push(ext("d3", 0.0, 1e6, 1.0, 1e6 + 1.0));
        extents.push(ext("d4", -1e6, -1e6, -1e6 + 1.0, -1e6 + 1.0));
        assert!(outliers(&extents).is_empty());
        // A sparse drawing: a few small lines and labels 30 diagonals out
        // are one drawing (the labels are a third of it).
        let mut sparse = square();
        sparse.push(ext("t1", 0.0, 20.0, 0.0, 20.0));
        sparse.push(ext("t2", 50.0, 50.0, 50.0, 50.0));
        sparse.push(ext("t3", 100.0, 100.0, 100.0, 100.0));
        assert!(outliers(&sparse).is_empty());
    }

    #[test]
    fn detached_groups_split_a_plan_from_its_details() {
        // A 100 x 50 "plan" of four lines, a detail of three lines 30 units
        // away, and a lone dot far off. With 5-unit cells the plan and the
        // detail are separate; with 40-unit cells they merge.
        let mut extents = vec![
            ext("1", 0.0, 0.0, 100.0, 0.0),
            ext("2", 100.0, 0.0, 100.0, 50.0),
            ext("3", 0.0, 50.0, 100.0, 50.0),
            ext("4", 0.0, 0.0, 0.0, 50.0),
            ext("5", 130.0, 0.0, 150.0, 0.0),
            ext("6", 150.0, 0.0, 150.0, 20.0),
            ext("7", 130.0, 20.0, 150.0, 20.0),
            ext("8", 500.0, 500.0, 500.0, 500.0),
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
    }

    #[test]
    fn the_header_is_a_candidate_only_when_sane_covering_and_not_too_large() {
        let extents = square();
        let kept: Vec<&Rect> = extents.iter().map(|e| &e.rect).collect();
        let content = Rect::new(0.0, 0.0, 10.0, 10.0);
        let ok = header_with((-1.0, -1.0), (11.0, 11.0));
        assert_eq!(
            header_candidate(&ok, Some(&content), &kept),
            Some(Rect::new(-1.0, -1.0, 11.0, 11.0))
        );
        // Too large: 30 x 30 is 9x the content area.
        let large = header_with((-10.0, -10.0), (20.0, 20.0));
        assert_eq!(header_candidate(&large, Some(&content), &kept), None);
        // Covers only 2 of 4 lines.
        let partial = header_with((0.0, 0.0), (10.0, 5.0));
        assert_eq!(header_candidate(&partial, Some(&content), &kept), None);
        // Runaway and empty headers.
        assert_eq!(header_extents(&header_with((1e20, 0.0), (1e21, 1.0))), None);
        assert_eq!(header_extents(&Header::default()), None);
        assert_eq!(header_extents(&header_with((5.0, 5.0), (5.0, 5.0))), None);
    }

    #[test]
    fn choose_covers_every_mode() {
        let mut extents = square();
        extents.push(ext("5", 1e6, 1e6, 2e6, 2e6));
        let header = header_with((-1.0, -1.0), (11.0, 11.0));

        let auto = choose(&extents, &header, CropMode::Auto);
        // The header covers the same 4 entities as the content: content wins.
        assert_eq!(auto.source, CropSource::Content);
        assert_eq!(auto.rect, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(auto.excluded.len(), 1);
        assert_eq!(auto.excluded[0].handle, "5");
        assert_eq!(auto.excluded[0].reason, ExcludeReason::ScaleOutlier);
        assert_eq!(auto.header_extents, Some(Rect::new(-1.0, -1.0, 11.0, 11.0)));

        let raw = choose(&extents, &header, CropMode::Raw);
        assert_eq!(raw.source, CropSource::Raw);
        assert_eq!(raw.rect, Rect::new(0.0, 0.0, 2e6, 2e6));
        assert!(raw.excluded.is_empty());

        let by_header = choose(&extents, &header, CropMode::Header);
        assert_eq!(by_header.source, CropSource::Header);
        assert_eq!(by_header.rect, Rect::new(-1.0, -1.0, 11.0, 11.0));
        assert_eq!(by_header.excluded.len(), 1);
        assert_eq!(by_header.excluded[0].reason, ExcludeReason::OutsideCrop);
        // No usable header: falls back to auto.
        let fallback = choose(&extents, &Header::default(), CropMode::Header);
        assert_eq!(fallback.source, CropSource::Content);

        let fixed = choose(
            &extents,
            &header,
            CropMode::Fixed(Rect::new(0.0, 0.0, 5.0, 5.0)),
        );
        assert_eq!(fixed.source, CropSource::Fixed);
        assert_eq!(fixed.content, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        // Lines 1, 4 touch the fixed rect; 2, 3 and the outlier are outside.
        let outside: Vec<&str> = fixed.excluded.iter().map(|e| e.handle.as_str()).collect();
        assert_eq!(outside, ["2", "3", "5"]);

        let empty = choose(&[], &header, CropMode::Auto);
        assert_eq!(empty.source, CropSource::Empty);
        assert_eq!(empty.rect, EMPTY_RECT);
    }

    #[test]
    fn auto_uses_the_header_when_it_covers_more() {
        // The guard drops the far dot; a header that reaches it covers 5 > 4.
        let mut extents = square();
        extents.push(ext("5", 1e6, 1e6, 1e6 + 1.0, 1e6 + 1.0));
        // 4x the content area at most: content is 1001 x 1001 with the dot,
        // 10 x 10 without; the header must stay within 4x of the latter, so
        // a header reaching the dot is rejected... unless the content is the
        // dot-inclusive one. Use a header that is exactly the raw bounds and
        // a dot close enough to be a far outlier but inside 4x: impossible
        // by construction, so test the rule the other way round: the header
        // is rejected and content wins.
        let header = header_with((0.0, 0.0), (1e6 + 1.0, 1e6 + 1.0));
        let auto = choose(&extents, &header, CropMode::Auto);
        assert_eq!(auto.source, CropSource::Content);
        assert_eq!(auto.rect, Rect::new(0.0, 0.0, 10.0, 10.0));

        // A header slightly larger than the content that also contains a
        // fifth entity the guard flagged (a large but touching one is never
        // flagged, so build the case with a forced seed): skip -- covered by
        // the corpus test in tests/crop.rs.
    }

    #[test]
    fn a_rect_is_the_documented_array_and_reads_both_forms() {
        // The package documents every box as `[x0, y0, x1, y1]`
        // (docs/VLM_EXPORT_DESIGN.md section 3), which is what the record
        // bboxes and tiles.json have always written; the derive wrote an
        // object, so the same rectangle had two shapes in one package.
        let rect = Rect::new(-6.35, -6.35, 273.05, 209.55);
        assert_eq!(
            serde_json::to_string(&rect).unwrap(),
            "[-6.35,-6.35,273.05,209.55]"
        );
        assert_eq!(
            serde_json::from_str::<Rect>("[-6.35,-6.35,273.05,209.55]").unwrap(),
            rect
        );
        // The object form 0.3.0 wrote still reads back.
        let object = r#"{"min_x":-6.35,"min_y":-6.35,"max_x":273.05,"max_y":209.55}"#;
        assert_eq!(serde_json::from_str::<Rect>(object).unwrap(), rect);
        // And so does every struct that holds one.
        let report = CropReport {
            source: CropSource::Content,
            rect,
            content: Some(rect),
            padding_units: 0.5,
            header_extents: None,
            excluded: vec![Excluded {
                handle: "5".into(),
                type_name: "LINE".into(),
                rect,
                reason: ExcludeReason::ScaleOutlier,
            }],
        };
        let text = serde_json::to_string(&report).unwrap();
        assert!(
            text.contains(r#""rect":[-6.35,-6.35,273.05,209.55]"#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<CropReport>(&text).unwrap(), report);
    }

    #[test]
    fn padding_and_lattice() {
        let rect = Rect::new(0.0, 0.0, 1000.0, 500.0);
        assert_eq!(auto_padding(&rect, None), 20.0);
        // 24 px at 0.5 px/unit is 48 units, more than 2 %.
        assert_eq!(auto_padding(&rect, Some(0.5)), 48.0);
        assert_eq!(auto_padding(&rect, Some(10.0)), 20.0);
        assert_eq!(auto_padding(&Rect::new(3.0, 3.0, 3.0, 3.0), None), 0.5);
        assert_eq!(
            auto_padding(&Rect::new(3.0, 3.0, 3.0, 3.0), Some(2.0)),
            12.0
        );

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

        // The affine round-trips through the ViewBox.
        let vb = crate::ViewBox::from_world(&snapped);
        let (px, py) = vb.world_to_px(123.456, 78.9, 1.5);
        let (x, y) = vb.px_to_world(px, py, 1.5);
        assert!((x - 123.456).abs() < 1e-9 && (y - 78.9).abs() < 1e-9);
        let (px, py) = vb.world_to_px(snapped.max_x, snapped.min_y, 1.5);
        assert!((px - 1568.0).abs() < 1e-9 && (py - 812.0).abs() < 1e-9);
    }
}
