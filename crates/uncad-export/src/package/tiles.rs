//! Frames and their tile pyramids: the drawing's detached groups, each with
//! its own overview when there is more than one; zoom levels deep enough for
//! the frame's text; overlapping tiles of every level, culled to the ones
//! something visible reaches; and the sidecar beside each tile saying what
//! is on it.

use std::collections::{BTreeMap, BTreeSet};

use iron_render_cad::{Background, Fonts, Part, PngError, Scene};
use serde_json::{json, Value};
use uncad_model::model::{EntityId, Point2D};

use super::output::to_rgb;
use super::records::{PlacedText, Record, Rounder};
use super::{
    fit_overview, ExportError, ExportOptions, FrameReport, HeightClass, ImageInfo, LevelInfo,
    Profile, SCHEMA, STROKE_PX,
};
use crate::frame::{detached_groups, Rect};

/// The most entries `manifest.frames_dropped` lists (the count is always
/// exact in `frames_dropped_total`).
const MAX_DROPPED_FRAMES: usize = 100;

pub(crate) struct Tile {
    pub(crate) id: String,
    pub(crate) frame: String,
    pub(crate) z: u32,
    pub(crate) row: u32,
    pub(crate) col: u32,
    /// Pixel origin on the level canvas.
    pub(crate) origin_px: (u32, u32),
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) world: Rect,
    pub(crate) empty: bool,
}

pub(crate) struct FrameBuild {
    pub(crate) report: FrameReport,
    pub(crate) tiles: Vec<Tile>,
}

/// The frames a drawing is split into, and the groups that were not made
/// frames.
pub(crate) struct FramePlan {
    pub(crate) frames: Vec<FrameBuild>,
    /// `manifest.frames_dropped`: at most [`MAX_DROPPED_FRAMES`] entries.
    pub(crate) dropped: Vec<Value>,
    pub(crate) dropped_total: usize,
}

/// Splits the drawing into frames -- `f0` the largest connected group of
/// entities, a detached group (a detail drawn beside the plan) its own when
/// it holds `min_frame_entities` entities or a text -- and plans each
/// frame's tile pyramid within the tile budget.
///
/// `extents` are the drawn entities' boxes, in drawing order; `crop` the
/// whole crop (the one frame of a drawing that does not split); `overview`
/// the whole crop's image, which a one-frame drawing reuses.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_frames(
    extents: &[Rect],
    crop: Rect,
    texts: &[PlacedText],
    overview: &ImageInfo,
    options: &ExportOptions,
    rounder: &Rounder,
    warnings: &mut Vec<String>,
) -> FramePlan {
    let groups = detached_groups(extents, options.frame_gap * crop.diagonal());
    let group_rect =
        |g: &[usize]| Rect::bounding(g.iter().map(|i| &extents[*i])).expect("non-empty");
    let group_texts = |r: &Rect| texts.iter().filter(|t| r.intersects(&t.bbox)).count();
    let mut specs: Vec<(String, &str, Rect, usize)> = Vec::new();
    // Every group that did not become a frame, with the reason: a reader
    // seeing a record with `tiles: []` needs to know whether that part of
    // the drawing was left out on purpose. The list is capped (a drawing
    // of scattered symbols can have thousands of groups) and
    // `frames_dropped_total` says how many there were.
    let mut dropped: Vec<Value> = Vec::new();
    let mut dropped_total = 0usize;
    let (mut dropped_small, mut dropped_over_max) = (0usize, 0usize);
    if groups.len() <= 1 {
        specs.push(("f0".to_string(), "primary", crop, extents.len()));
    } else {
        for (n, g) in groups.iter().enumerate() {
            let r = group_rect(g);
            let qualifies = n == 0 || g.len() >= options.min_frame_entities || group_texts(&r) > 0;
            let reason = if !qualifies {
                dropped_small += 1;
                "below_min_entities"
            } else if specs.len() >= options.max_frames.max(1) {
                dropped_over_max += 1;
                "max_frames"
            } else {
                let id = format!("f{}", specs.len());
                specs.push((id, if n == 0 { "primary" } else { "detached" }, r, g.len()));
                continue;
            };
            dropped_total += 1;
            if dropped.len() < MAX_DROPPED_FRAMES {
                dropped.push(json!({
                    "content": rounder.rect(&r),
                    "entities": g.len(),
                    "texts": group_texts(&r),
                    "reason": reason,
                }));
            }
        }
        if dropped_over_max > 0 {
            warnings.push(format!(
                "MaxFrames: {dropped_over_max} detached groups beyond the {} frames written stay in the overview only (frames_dropped)",
                options.max_frames
            ));
        }
        if dropped_small > 0 {
            warnings.push(format!(
                "SmallGroups: {dropped_small} detached groups hold fewer than {} entities and no text; they stay in the overview only, so their records carry `tiles: []` (frames_dropped)",
                options.min_frame_entities
            ));
        }
    }
    let mut frames = Vec::new();
    let mut tile_budget = options.max_tiles;
    let single = specs.len() == 1;
    for (id, kind, content, entities) in &specs {
        let reuse = single.then(|| overview.clone());
        frames.push(build_frame(
            id,
            kind,
            *content,
            *entities,
            texts,
            extents,
            options,
            reuse,
            &mut tile_budget,
            warnings,
        ));
    }
    FramePlan {
        frames,
        dropped,
        dropped_total,
    }
}

/// Plans one frame: its overview (or the whole-crop one when `reuse` is
/// given), its depth from the texts inside it, and its tiles within the
/// remaining `tile_budget`.
#[allow(clippy::too_many_arguments)]
fn build_frame(
    id: &str,
    kind: &str,
    content: Rect,
    entities: usize,
    texts: &[PlacedText],
    extents: &[Rect],
    options: &ExportOptions,
    reuse: Option<ImageInfo>,
    tile_budget: &mut usize,
    warnings: &mut Vec<String>,
) -> FrameBuild {
    let profile = &options.profile;
    let overview = reuse.unwrap_or_else(|| {
        let fit = fit_overview(&content, profile, options.padding);
        ImageInfo::new(
            &format!("{id}/ov"),
            &format!("frames/{id}/overview.png"),
            fit.rect,
            fit.ppu,
            fit.width,
            fit.height,
        )
    });
    let (rect0, w0, h0, ppu_0) = (overview.world, overview.px[0], overview.px[1], overview.ppu);
    let inside: Vec<&PlacedText> = texts
        .iter()
        .filter(|t| content.intersects(&t.bbox))
        .collect();
    let heights = height_classes(&inside);
    let z_max = depth_for(&heights, ppu_0, options);
    let mut levels: Vec<LevelInfo> = Vec::new();
    let mut tiles: Vec<Tile> = Vec::new();
    let mut reached = true;
    for z in 1..=z_max {
        let ppu = ppu_0 * 2f64.powi(z as i32);
        let (cw, ch) = (w0 * 2u32.pow(z), h0 * 2u32.pow(z));
        let plan = plan_tiles(id, z, cw, ch, &rect0, ppu, profile, extents);
        let written = plan.iter().filter(|t| !t.empty).count();
        if written > *tile_budget {
            reached = false;
            warnings.push(format!(
                "MaxTiles: frame {id} level z{z} would need {written} tiles with {} left of {}; stopping at z{}",
                *tile_budget,
                options.max_tiles,
                z - 1
            ));
            break;
        }
        *tile_budget -= written;
        let (cols, rows) = grid(cw, ch, profile);
        levels.push(LevelInfo {
            z,
            ppu,
            canvas_px: [cw, ch],
            cols,
            rows,
            tile_px: profile.tile,
            overlap_px: profile.overlap,
            step_px: profile.tile - profile.overlap,
            tiles_written: written,
            tiles_empty: plan.len() - written,
        });
        tiles.extend(plan);
    }
    let z_reached = levels.last().map_or(0, |l| l.z);
    let height_classes = heights
        .iter()
        .map(|(h, n)| {
            let px = h * ppu_0 * 2f64.powi(z_reached as i32);
            HeightClass {
                height: super::records::round_to(*h, 6),
                count: *n,
                px_at_zmax: super::records::round_to(px, 3),
                legible: px >= options.target_text_px,
            }
        })
        .collect();
    FrameBuild {
        report: FrameReport {
            id: id.to_string(),
            kind: kind.to_string(),
            content,
            entities,
            texts: inside.len(),
            overview,
            levels,
            z_max: z_reached,
            reached,
            height_classes,
        },
        tiles,
    }
}

/// Count-weighted text height classes (heights rounded to 3 decimals),
/// most common first.
fn height_classes(texts: &[&PlacedText]) -> Vec<(f64, usize)> {
    let mut classes: BTreeMap<i64, usize> = BTreeMap::new();
    for t in texts {
        if t.height > 0.0 && t.height.is_finite() {
            *classes
                .entry((t.height * 1000.0).round() as i64)
                .or_default() += 1;
        }
    }
    let mut out: Vec<(f64, usize)> = classes
        .into_iter()
        .map(|(k, n)| (k as f64 / 1000.0, n))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.total_cmp(&b.0)));
    out
}

/// The deepest level: where the dominant text class (the count-weighted
/// median height) reaches the target pixel height; one level without text.
fn depth_for(heights: &[(f64, usize)], ppu_0: f64, options: &ExportOptions) -> u32 {
    if options.max_levels == 0 {
        return 0;
    }
    let total: usize = heights.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return 1;
    }
    let mut sorted: Vec<(f64, usize)> = heights.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut seen = 0;
    let mut median = sorted[0].0;
    for (h, n) in &sorted {
        seen += n;
        if seen * 2 >= total {
            median = *h;
            break;
        }
    }
    let px_now = median * ppu_0;
    if px_now <= 0.0 {
        return 1;
    }
    let z = (options.target_text_px / px_now).log2().ceil();
    if z.is_finite() {
        (z.max(1.0) as u32).min(options.max_levels)
    } else {
        1
    }
}

fn grid(canvas_w: u32, canvas_h: u32, profile: &Profile) -> (u32, u32) {
    let step = profile.tile - profile.overlap;
    let count = |extent: u32| -> u32 {
        if extent <= profile.tile {
            1
        } else {
            (extent - profile.tile).div_ceil(step) + 1
        }
    };
    (count(canvas_w), count(canvas_h))
}

/// The tiles of one level: SAHI-style, the last row and column shifted
/// inward so every tile is the full size (or the whole canvas when that is
/// smaller); a tile is empty when no drawn extent touches it.
#[allow(clippy::too_many_arguments)]
fn plan_tiles(
    frame: &str,
    z: u32,
    canvas_w: u32,
    canvas_h: u32,
    rect0: &Rect,
    ppu: f64,
    profile: &Profile,
    extents: &[Rect],
) -> Vec<Tile> {
    let (cols, rows) = grid(canvas_w, canvas_h, profile);
    let step = profile.tile - profile.overlap;
    let origin = |index: u32, extent: u32| -> (u32, u32) {
        if extent <= profile.tile {
            (0, extent)
        } else {
            let o = (index * step).min(extent - profile.tile);
            (o, profile.tile)
        }
    };
    let mut tiles = Vec::new();
    for row in 0..rows {
        let (oy, th) = origin(row, canvas_h);
        for col in 0..cols {
            let (ox, tw) = origin(col, canvas_w);
            let x0 = rect0.min_x + f64::from(ox) / ppu;
            let y1 = rect0.max_y - f64::from(oy) / ppu;
            let world = Rect::new(x0, y1 - f64::from(th) / ppu, x0 + f64::from(tw) / ppu, y1);
            let empty = !extents.iter().any(|e| e.intersects(&world));
            tiles.push(Tile {
                id: format!("{frame}/z{z}/r{row:02}_c{col:02}"),
                frame: frame.to_string(),
                z,
                row,
                col,
                origin_px: (ox, oy),
                width: tw,
                height: th,
                world,
                empty,
            });
        }
    }
    tiles
}

/// Which parts an image of `window` draws: every part the crop did not
/// leave out whose extent reaches the window grown by `margin` (for strokes
/// and text overhang), every construction line, and every part that
/// measured no extent -- so rasterizing a tile costs what is on it, not the
/// whole drawing.
pub(crate) fn keep_for<'a>(
    window: Rect,
    margin: f64,
    extent_of: &'a BTreeMap<EntityId, Rect>,
) -> impl Fn(&Part) -> bool + 'a {
    let grown = window.padded(margin);
    move |p: &Part| {
        p.left_out.is_none()
            && (p.unbounded
                || extent_of
                    .get(&p.id)
                    .is_none_or(|rect| rect.intersects(&grown)))
    }
}

/// Renders `images` on as many threads as the machine offers (at most one
/// per image), returning the 8-bit RGB PNG bytes in their order. A thread
/// that panics ends the export with [`PngError::RenderPanic`], not with the
/// process.
pub(crate) fn render_images(
    scene: &Scene,
    fonts: &Fonts,
    extent_of: &BTreeMap<EntityId, Rect>,
    images: &[&ImageInfo],
) -> Result<Vec<Vec<u8>>, ExportError> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 16)
        .min(images.len());
    let chunk = images.len().div_ceil(threads);
    let render_one = |image: &ImageInfo| -> Result<Vec<u8>, ExportError> {
        let keep = keep_for(image.world, 16.0 / image.ppu, extent_of);
        let rgba = scene.png(&image.view(), STROKE_PX, fonts, Background::White, keep)?;
        to_rgb(&rgba)
    };
    let results: Vec<Result<Vec<Vec<u8>>, ExportError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = images
            .chunks(chunk)
            .map(|group| {
                let render_one = &render_one;
                scope.spawn(move || {
                    group
                        .iter()
                        .map(|image| render_one(image))
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    Err(ExportError::Render(PngError::RenderPanic(
                        "a tile rendering thread panicked".to_string(),
                    )))
                })
            })
            .collect()
    });
    let mut out = Vec::with_capacity(images.len());
    for group in results {
        out.extend(group?);
    }
    Ok(out)
}

/// A sidecar's cap: its compact JSON never exceeds this.
const SIDECAR_LIMIT: usize = 32 * 1024;

/// The most rows of one kind a sidecar builds before the shrink loop even
/// looks at it. The shortest row a group can write is about 25 bytes, so
/// nothing beyond this could ever fit in [`SIDECAR_LIMIT`]; cutting here is
/// the same cut the loop would make, reported the same way.
const MAX_SIDECAR_ROWS: usize = SIDECAR_LIMIT / 20;

/// The records a sidecar looks at, by kind.
pub(crate) struct OnTiles<'a> {
    pub(crate) texts: &'a [Record],
    pub(crate) dims: &'a [Record],
    pub(crate) blocks: &'a [Record],
    pub(crate) regions: &'a [Record],
    pub(crate) geometry: &'a [Record],
}

/// The sidecar of one written tile: where it is, how its pixels map to the
/// world, its written neighbours, parent and children, and the records on
/// it as positional rows (their columns named beside them), cut to fit
/// [`SIDECAR_LIMIT`] with `counts` and `geometry_by_kind` staying exact.
pub(crate) fn sidecar(
    img: &ImageInfo,
    tile: &Tile,
    tiles: &[Tile],
    profile: &Profile,
    records: &OnTiles<'_>,
    rounder: &Rounder,
) -> Value {
    // Only tiles that were *written*: an empty tile gets no .png and no
    // .json, so a reader panning by `neighbors` must not be sent to one.
    // tiles.json still lists the empty ones, with their reason.
    let find = |z: u32, row: i64, col: i64| -> Option<String> {
        if row < 0 || col < 0 {
            return None;
        }
        tiles
            .iter()
            .find(|t| t.z == z && i64::from(t.row) == row && i64::from(t.col) == col && !t.empty)
            .map(|t| t.id.clone())
    };
    let (r, c) = (i64::from(tile.row), i64::from(tile.col));
    let neighbors = json!({
        "n": find(tile.z, r - 1, c),
        "s": find(tile.z, r + 1, c),
        "w": find(tile.z, r, c - 1),
        "e": find(tile.z, r, c + 1),
    });
    let centre = Point2D {
        x: (tile.world.min_x + tile.world.max_x) / 2.0,
        y: (tile.world.min_y + tile.world.max_y) / 2.0,
    };
    let parent = tiles
        .iter()
        .find(|t| {
            t.z + 1 == tile.z
                && !t.empty
                && t.world.min_x <= centre.x
                && centre.x <= t.world.max_x
                && t.world.min_y <= centre.y
                && centre.y <= t.world.max_y
        })
        .map(|t| t.id.clone());
    let children: Vec<String> = tiles
        .iter()
        .filter(|t| t.z == tile.z + 1 && t.world.intersects(&tile.world) && !t.empty)
        .map(|t| t.id.clone())
        .collect();
    fn on_tile<'a>(records: &'a [Record], world: &Rect) -> Vec<&'a Record> {
        records
            .iter()
            .filter(|rec| rec.bbox.intersects(world))
            .collect()
    }
    let truncate = |s: &str| -> String {
        if s.chars().count() > 24 {
            let cut: String = s.chars().take(24).collect();
            format!("{cut}...")
        } else {
            s.to_string()
        }
    };
    let (on_texts, on_dims, on_blocks, on_regions, on_geometry) = (
        on_tile(records.texts, &tile.world),
        on_tile(records.dims, &tile.world),
        on_tile(records.blocks, &tile.world),
        on_tile(records.regions, &tile.world),
        on_tile(records.geometry, &tile.world),
    );
    let rows = |on: &[&Record], third: &dyn Fn(&Record) -> Vec<Value>| -> Vec<Value> {
        on.iter()
            .take(MAX_SIDECAR_ROWS)
            .map(|rec| {
                let mut row = vec![json!(rec.id), json!(img.px_box(&rec.bbox))];
                row.extend(third(rec));
                Value::Array(row)
            })
            .collect()
    };
    let field = |rec: &Record, key: &str| rec.value.get(key).cloned().unwrap_or(Value::Null);
    let mut text_rows = rows(&on_texts, &|rec| {
        vec![json!(truncate(
            rec.value.get("text").and_then(Value::as_str).unwrap_or("")
        ))]
    });
    let mut dim_rows = rows(&on_dims, &|rec| {
        vec![
            json!(truncate(
                rec.value
                    .get("display")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )),
            field(rec, "measurement"),
        ]
    });
    let mut block_rows = rows(&on_blocks, &|rec| vec![field(rec, "block")]);
    let mut region_rows = rows(&on_regions, &|rec| vec![field(rec, "area")]);
    // The geometry -- the bulk of every tile. It is cut first when the file
    // has to shrink (a text or a dimension is what a reader came for), so
    // `counts`, which is never truncated, and `geometry_by_kind` are what
    // say how much is really there.
    let mut geometry_rows = rows(&on_geometry, &|rec| vec![field(rec, "type")]);
    let mut geometry_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for rec in &on_geometry {
        if let Some(Value::String(t)) = rec.value.get("type") {
            *geometry_by_kind.entry(t.clone()).or_default() += 1;
        }
    }
    let counts = json!({
        "texts": on_texts.len(),
        "dims": on_dims.len(),
        "blocks": on_blocks.len(),
        "regions": on_regions.len(),
        "geometry": on_geometry.len(),
    });
    // Every record on the tile, geometry included, computed before the
    // shrink loop so the layer set stays complete even when rows are cut.
    let mut layer_set: BTreeSet<String> = BTreeSet::new();
    for rec in on_texts
        .iter()
        .chain(on_dims.iter())
        .chain(on_blocks.iter())
        .chain(on_regions.iter())
        .chain(on_geometry.iter())
    {
        if let Some(Value::String(l)) = rec.value.get("layer") {
            layer_set.insert(l.clone());
        }
    }
    let layers_total = layer_set.len();
    let mut layers: Vec<String> = layer_set.into_iter().collect();
    let build = |text_rows: &[Value],
                 dim_rows: &[Value],
                 block_rows: &[Value],
                 region_rows: &[Value],
                 geometry_rows: &[Value],
                 layers: &[String],
                 truncated: bool| {
        let mut value = json!({
            "$schema": SCHEMA,
            "id": img.id,
            "png": img.png,
            "z": tile.z,
            "row": tile.row,
            "col": tile.col,
            "px": img.px,
            "canvas_origin_px": [tile.origin_px.0, tile.origin_px.1],
            "world": rounder.rect(&img.world),
            "ppu": img.ppu,
            "world_to_px": img.world_to_px,
            "px_to_world": img.px_to_world,
            "overlap_px": profile.overlap,
            "neighbors": neighbors,
            "parent": parent,
            "children": children,
            "empty": tile.empty,
            "layers_present": layers,
            "layers_truncated": layers.len() < layers_total,
            // What each positional row holds, beside the rows themselves.
            "columns": {
                "texts": ["id", "px_box", "text"],
                "dims": ["id", "px_box", "display", "measurement"],
                "blocks": ["id", "px_box", "block"],
                "regions": ["id", "px_box", "area"],
                "geometry": ["id", "px_box", "type"],
            },
            "counts": counts,
            "geometry_by_kind": geometry_by_kind,
            "records": { "texts": text_rows, "dims": dim_rows, "blocks": block_rows, "regions": region_rows, "geometry": geometry_rows },
            "records_truncated": truncated,
        });
        if layers.len() < layers_total {
            value["layers_total"] = json!(layers_total);
        }
        value
    };
    let mut truncated = [
        (text_rows.len(), on_texts.len()),
        (dim_rows.len(), on_dims.len()),
        (block_rows.len(), on_blocks.len()),
        (region_rows.len(), on_regions.len()),
        (geometry_rows.len(), on_geometry.len()),
    ]
    .iter()
    .any(|(rows, all)| rows < all);
    let mut value = build(
        &text_rows,
        &dim_rows,
        &block_rows,
        &region_rows,
        &geometry_rows,
        &layers,
        truncated,
    );
    // Geometry rows are cut first (halved), then the other rows (by a
    // quarter), then the layer list: each list has its own flag, so the
    // file always says which one the reader is missing.
    while serde_json::to_string(&value).map_or(0, |s| s.len()) > SIDECAR_LIMIT {
        let rows_left = [
            text_rows.len(),
            dim_rows.len(),
            block_rows.len(),
            region_rows.len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if !geometry_rows.is_empty() {
            truncated = true;
            let keep = geometry_rows.len() / 2;
            geometry_rows.truncate(keep);
        } else if rows_left > 0 {
            truncated = true;
            for rows in [
                &mut text_rows,
                &mut dim_rows,
                &mut block_rows,
                &mut region_rows,
            ] {
                let keep = rows.len() * 3 / 4;
                rows.truncate(keep);
            }
        } else if !layers.is_empty() {
            let keep = layers.len() * 3 / 4;
            layers.truncate(keep);
        } else {
            break;
        }
        value = build(
            &text_rows,
            &dim_rows,
            &block_rows,
            &region_rows,
            &geometry_rows,
            &layers,
            truncated,
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_and_tile_plan_match_the_design() {
        let profile = Profile::CLAUDE;
        assert_eq!(grid(1000, 500, &profile), (1, 1));
        // 2688 px: (2688 - 1092) / 868 = 1.84 -> 2 + 1 = 3 columns.
        assert_eq!(grid(2688, 1792, &profile), (3, 2));
        let rect0 = Rect::new(0.0, 0.0, 2688.0, 1792.0);
        let extents = [Rect::new(2600.0, 100.0, 2650.0, 150.0)];
        let tiles = plan_tiles("f0", 1, 2688, 1792, &rect0, 1.0, &profile, &extents);
        assert_eq!(tiles.len(), 6);
        assert!(tiles.iter().all(|t| t.width == 1092 && t.height == 1092));
        // The last column is shifted inward to end at the canvas edge.
        let last = tiles.iter().find(|t| t.row == 0 && t.col == 2).unwrap();
        assert_eq!(last.origin_px, (2688 - 1092, 0));
        assert_eq!(last.id, "f0/z1/r00_c02");
        // Only the tiles touching the line at the top right are non-empty:
        // rows are y-down, so row 0 holds y in [700, 1792] and the line at
        // y 100..150 lies in row 1.
        let non_empty: Vec<&str> = tiles
            .iter()
            .filter(|t| !t.empty)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(non_empty, ["f0/z1/r01_c02"]);
    }

    #[test]
    fn depth_reaches_the_target_text_height() {
        let options = ExportOptions::default();
        // 2.5-unit text at 0.5 px/unit is 1.25 px: 14 / 1.25 = 11.2 -> 2^4.
        assert_eq!(depth_for(&[(2.5, 10)], 0.5, &options), 4);
        // Already legible: still one level.
        assert_eq!(depth_for(&[(50.0, 10)], 1.0, &options), 1);
        // No text: one level; max_levels caps.
        assert_eq!(depth_for(&[], 1.0, &options), 1);
        let capped = ExportOptions {
            max_levels: 2,
            ..Default::default()
        };
        assert_eq!(depth_for(&[(0.01, 1)], 0.1, &capped), 2);
    }
}
