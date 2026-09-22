//! HATCH rendering: boundary-path outlines, gradient fills, and pattern fills
//! tiled through SVG's own `<pattern>` element.

use super::format::{clean, neg};
use super::Ctx;
use crate::dynapi::Point2D;
use crate::model::{HatchBoundaryPath, HatchEdge, HatchEntity, HatchGradient, HatchPatternLine};
use std::fmt::Write as _;

/// Renders one HATCH: always an outline of its boundary paths, plus -- in
/// priority order -- a gradient fill, a translucent solid fill, or tiled
/// pattern lines. `None` when no boundary path has enough points to draw.
///
/// Gradient is checked before `solid_fill` because AutoCAD sets `solid_fill`
/// on gradient hatches too (a gradient is a "solid style" fill with a varying
/// color), so the other order would always shadow the gradient branch.
pub(super) fn render_hatch(h: &HatchEntity, color: &str, ctx: &mut Ctx) -> Option<String> {
    let mut subpaths = Vec::new();
    for path in &h.boundary_paths {
        let pts: Vec<Point2D> = match path {
            HatchBoundaryPath::Polyline(vertices) => vertices.clone(),
            HatchBoundaryPath::Edges(edges) => edges.iter().flat_map(edge_points).collect(),
        };
        if pts.len() < 2 {
            continue;
        }
        ctx.consider_all(&pts);
        let frame = ctx.frame;
        let mut d = String::from("M ");
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                d.push_str(" L ");
            }
            let _ = write!(d, "{} {}", frame.x(p.x), frame.y(p.y));
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

    if let Some(gradient) = &h.gradient {
        return Some(render_gradient(gradient, &d, ctx));
    }
    if h.solid_fill {
        return Some(format!(
            "<path d=\"{d}\" fill=\"{color}\" fill-opacity=\"0.2\" stroke=\"{color}\" fill-rule=\"evenodd\"/>"
        ));
    }

    let scale = ctx.scale;
    let pattern_fills: Vec<String> = h
        .pattern_lines
        .iter()
        .filter_map(|pl| render_pattern_line(pl, color, scale, &d, ctx))
        .collect();
    if pattern_fills.is_empty() {
        // No usable pattern data (unreadable deflines, or every defline
        // degenerate) -- outline only.
        return Some(outline);
    }
    Some(format!("{}\n  {outline}", pattern_fills.join("\n  ")))
}

/// Signed sweep angle from `start_angle` to `end_angle`, normalized into the
/// half-open range implied by `is_ccw` -- AutoCAD's arc-direction convention,
/// not just the shorter or positive arc.
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

/// Chord-approximates one boundary edge, in the same spirit as SPLINE and
/// curved 3DSOLID edges elsewhere. Each edge's final point is omitted, since it
/// coincides with the next edge's first point (or, for a single closed edge
/// like a full ellipse, with its own first point).
fn edge_points(edge: &HatchEdge) -> Vec<Point2D> {
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

/// Renders one [`HatchPatternLine`] as a tiled `<pattern>` fill of the
/// boundary path `path_d`: the definition goes into `ctx.defs` (emitted once
/// into a top-level `<defs>`) and the returned element references it. `None`
/// if the line family is degenerate (non-finite or zero perpendicular spacing,
/// so nothing to tile).
///
/// Clipping is left to SVG's own fill mechanism rather than hand-rolled
/// polygon clipping.
///
/// `pl`'s fields are used directly, as already-final values -- the HATCH's own
/// `pattern_angle`/`pattern_scale` (DXF 52/41) are deliberately not reapplied.
/// Applying them, as standard DXF documentation describes, was tried and
/// produced visibly wrong output: a HATCH with a 90-degree pattern angle had a
/// single defline whose own angle was *also* exactly 90 degrees, only
/// explainable if LibreDWG's defline data already has 52/41 baked in.
/// Multiplying by a `pattern_scale` of 60 likewise inflated a ~6.5-unit
/// spacing to ~390 units, far larger than the ~90-unit shape it was filling,
/// so no line landed inside the boundary at all.
///
/// Two further simplifications, in the same best-effort spirit as this
/// renderer's chord-approximated curves:
/// - only `offset`'s component perpendicular to the line direction (the
///   spacing) is honored; a parallel component, which real DXF patterns use
///   for brick/masonry-style staggering, is dropped rather than sheared into
///   the tile.
/// - the line sits at the vertical midpoint of its tile rather than at
///   `base_point`, which would land exactly on a tile edge and let
///   `<pattern>`'s default clipping cut the stroke in half. The phase is
///   therefore only approximate -- immaterial for an infinitely repeating
///   pattern.
///
/// `stroke_scale` is `ctx.scale` at render time: `<pattern>` content inherits
/// presentation properties from its own ancestors under `<defs>`, never from
/// wherever `fill="url(#...)"` is used, so a HATCH nested inside a scaled block
/// reference would otherwise draw its pattern lines at the wrong thickness.
fn render_pattern_line(
    pl: &HatchPatternLine,
    color: &str,
    stroke_scale: f64,
    path_d: &str,
    ctx: &mut Ctx,
) -> Option<String> {
    let (dir_x, dir_y) = (pl.angle.cos(), pl.angle.sin());
    let (perp_x, perp_y) = (-dir_y, dir_x);
    let spacing = (pl.offset.x * perp_x + pl.offset.y * perp_y).abs();
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

    let id = ctx.next_def_id("hp");
    // The pattern tiles the referencing element's user space, so its base
    // point is written in the same frame as the boundary.
    let (tx, ty) = (ctx.frame.x(pl.base_point.x), ctx.frame.y(pl.base_point.y));
    let deg = neg(pl.angle.to_degrees());
    let (w, h, half) = (clean(width), clean(spacing), clean(spacing / 2.0));
    ctx.defs.push(format!(
        "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{w}\" height=\"{h}\" \
         patternTransform=\"translate({tx} {ty}) rotate({deg})\">\n    \
         <line x1=\"0\" y1=\"{half}\" x2=\"{w}\" y2=\"{half}\" stroke=\"{color}\" \
         stroke-width=\"{}\"{dasharray}/>\n  </pattern>",
        super::stroke_width_placeholder(stroke_scale)
    ));

    Some(format!(
        "<path d=\"{path_d}\" fill=\"url(#{id})\" stroke=\"none\" fill-rule=\"evenodd\"/>"
    ))
}

/// Renders a [`HatchGradient`] as an SVG `linearGradient`/`radialGradient`
/// filling `path_d`, pushed into `ctx.defs` like the pattern definitions above.
///
/// Both kinds use `objectBoundingBox` units (0..1 relative to the filled
/// path's own box) rather than absolute coordinates: there is no tiling here,
/// just two stops spanning the shape, so no `ctx.scale` bookkeeping is needed.
/// The linear case starts left-to-right and rotates `angle` around the box
/// center, negated for the y-flip the same way pattern rotation is. The radial
/// case ignores `angle` -- a center-out gradient has no meaningful rotation.
fn render_gradient(g: &HatchGradient, path_d: &str, ctx: &mut Ctx) -> String {
    let id = ctx.next_def_id("hg");
    let def = if g.is_radial {
        format!(
            "<radialGradient id=\"{id}\" gradientUnits=\"objectBoundingBox\" cx=\"0.5\" cy=\"0.5\" r=\"0.5\">\n    \
             <stop offset=\"0%\" stop-color=\"{}\"/>\n    <stop offset=\"100%\" stop-color=\"{}\"/>\n  </radialGradient>",
            g.color1, g.color2
        )
    } else {
        let deg = neg(g.angle.to_degrees());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::Tables;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ~= {b}");
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
    fn edge_points_line_is_a_single_point() {
        let pts = edge_points(&HatchEdge::Line {
            start: Point2D { x: 1.0, y: 2.0 },
        });
        assert_eq!(pts, vec![Point2D { x: 1.0, y: 2.0 }]);
    }

    #[test]
    fn edge_points_arc_chord_approximates_on_the_circle() {
        let pi = std::f64::consts::PI;
        let pts = edge_points(&HatchEdge::Arc {
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
    fn edge_points_spline_drops_the_final_control_point() {
        let cps = vec![
            Point2D { x: 0.0, y: 0.0 },
            Point2D { x: 1.0, y: 1.0 },
            Point2D { x: 2.0, y: 2.0 },
        ];
        let pts = edge_points(&HatchEdge::Spline {
            control_points: cps.clone(),
        });
        assert_eq!(pts, &cps[..2]);
        assert_eq!(
            edge_points(&HatchEdge::Spline {
                control_points: vec![]
            }),
            Vec::new()
        );
    }

    #[test]
    fn render_pattern_line_skips_zero_perpendicular_spacing() {
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
        assert!(render_pattern_line(&pl, "#000000", 1.0, "M 0 0 Z", &mut ctx).is_none());
        assert!(ctx.defs.is_empty());
    }

    #[test]
    fn render_pattern_line_emits_one_pattern_def_per_call() {
        let tables = Tables::default();
        let mut ctx = Ctx::new(&tables);
        let pl = HatchPatternLine {
            angle: 0.0,
            base_point: Point2D { x: 0.0, y: 0.0 },
            offset: Point2D { x: 0.0, y: 2.0 },
            dash_pattern: vec![],
        };
        let path_d = "M 0 0 L 1 1 Z";
        let first = render_pattern_line(&pl, "#000000", 1.0, path_d, &mut ctx)
            .expect("valid spacing should produce a fill");
        assert_eq!(ctx.defs.len(), 1);
        assert!(ctx.defs[0].contains("<pattern"));
        assert!(first.contains("fill=\"url(#hp0)\""));

        let second = render_pattern_line(&pl, "#000000", 1.0, path_d, &mut ctx)
            .expect("second call should also succeed");
        assert_eq!(
            ctx.defs.len(),
            2,
            "each call must get its own unique pattern id"
        );
        assert!(second.contains("fill=\"url(#hp1)\""));
    }
}
