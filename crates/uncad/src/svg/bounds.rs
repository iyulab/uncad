//! viewBox computation: per-entity bounding boxes, and the outlier trim that
//! picks the dominant spatially-connected cluster of them.
//!
//! A straight per-axis gap test is fundamentally wrong for drawings: a
//! rectangular room's opposite walls only touch at the corners, so an axis-gap
//! approach reads the empty interior as a disconnected outlier and trims away
//! real geometry. These boxes are clustered by corner proximity instead.

#[derive(Debug, Clone, Copy)]
pub(super) struct Box2D {
    pub(super) min_x: f64,
    pub(super) max_x: f64,
    pub(super) min_y: f64,
    pub(super) max_y: f64,
}

pub(super) fn bbox_of(group: &[Box2D]) -> Box2D {
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

/// Shortest distance between two rectangles; 0 when they overlap.
pub(super) fn rect_gap(a: &Box2D, b: &Box2D) -> f64 {
    let dx = (a.min_x - b.max_x).max(b.min_x - a.max_x).max(0.0);
    let dy = (a.min_y - b.max_y).max(b.min_y - a.max_y).max(0.0);
    dx.hypot(dy)
}

pub(super) fn diag(b: &Box2D) -> f64 {
    (b.max_x - b.min_x).hypot(b.max_y - b.min_y)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 * p).floor() as usize).min(sorted.len() - 1);
    sorted[idx]
}

/// Groups boxes whose corners lie within a scale-derived epsilon of each
/// other, via union-find over a grid of corner buckets.
fn cluster_entity_boxes(boxes: &[Box2D]) -> Vec<Vec<Box2D>> {
    let n = boxes.len();
    if n == 0 {
        return Vec::new();
    }

    let mut all_x: Vec<f64> = boxes.iter().flat_map(|b| [b.min_x, b.max_x]).collect();
    let mut all_y: Vec<f64> = boxes.iter().flat_map(|b| [b.min_y, b.max_y]).collect();
    all_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    all_y.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // The interquartile span, not the full extent: one far-away outlier must
    // not inflate the epsilon that decides what counts as "touching".
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

    // Bucket corners into eps-sized cells so each one only has to be compared
    // against the 9 cells around it, not against every other box.
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

/// The bounding box of the drawing's dominant cluster, or `None` when
/// everything is one cluster (nothing to trim) or no cluster holds a majority
/// of the boxes (too ambiguous to trim safely).
///
/// Starting from the highest-scoring cluster (count times diagonal), nearby
/// clusters are absorbed until nothing else is within 30% of the growing seed's
/// diagonal. A cluster with very few boxes but a diagonal far larger than the
/// typical one is treated as degenerate and never absorbed -- that is the
/// runaway-coordinate case the trim exists for.
pub(super) fn dominant_cluster_box(boxes: &[Box2D]) -> Option<Box2D> {
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

    if rest.is_empty() {
        return None;
    }
    if (seed.len() as f64) / (boxes.len() as f64) < 0.5 {
        return None;
    }
    Some(bbox_of(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ~= {b}");
    }

    fn close_box(a: Box2D, b: Box2D) {
        close(a.min_x, b.min_x);
        close(a.max_x, b.max_x);
        close(a.min_y, b.min_y);
        close(a.max_y, b.max_y);
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
    fn percentile_indexes_by_fraction_and_clamps_at_the_last_element() {
        let sorted = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 0.25), 2.0);
        assert_eq!(percentile(&sorted, 0.75), 4.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn bbox_of_and_diag_and_rect_gap() {
        let a = bx(0.0, 0.0, 1.0, 1.0);
        let b = bx(2.0, 0.0, 3.0, 1.0);
        close(diag(&bx(0.0, 0.0, 3.0, 4.0)), 5.0);
        close(rect_gap(&a, &b), 1.0);
        close(rect_gap(&a, &bx(0.5, 0.5, 1.5, 1.5)), 0.0); // overlapping
        close_box(bbox_of(&[a, b]), bx(0.0, 0.0, 3.0, 1.0));
    }

    #[test]
    fn cluster_entity_boxes_separates_a_distant_outlier() {
        let boxes = [
            bx(0.0, 0.0, 1.0, 1.0),
            bx(1.0, 1.0, 2.0, 2.0),
            bx(1.0, 0.0, 2.0, 1.0),
            bx(1000.0, 1000.0, 1001.0, 1001.0),
        ];
        let clusters = cluster_entity_boxes(&boxes);
        assert_eq!(clusters.len(), 2);
        let mut sizes: Vec<usize> = clusters.iter().map(|c| c.len()).collect();
        sizes.sort_unstable();
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
}
