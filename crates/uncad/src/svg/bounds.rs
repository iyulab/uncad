//! Per-entity bounding boxes and the proximity clustering the crop rule
//! (`crate::crop`) builds its scale-outlier guard on.
//!
//! A straight per-axis gap test is fundamentally wrong for drawings: a
//! rectangular room's opposite walls only touch at the corners, so an axis-gap
//! approach reads the empty interior as a disconnected outlier and trims away
//! real geometry. These boxes are clustered by corner proximity instead.

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Box2D {
    pub(crate) min_x: f64,
    pub(crate) max_x: f64,
    pub(crate) min_y: f64,
    pub(crate) max_y: f64,
}

#[cfg(test)]
pub(crate) fn diag(b: &Box2D) -> f64 {
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
/// other, via union-find over a grid of corner buckets. Returns the index
/// groups, each sorted, in a deterministic order (by their smallest index).
/// Not used by the crop rule any more (0.2.0's trim was built on it); kept
/// for the frame split the export (design section 4, step 5) will need.
#[allow(dead_code)]
pub(crate) fn cluster_indices(boxes: &[Box2D]) -> Vec<Vec<usize>> {
    let n = boxes.len();
    if n == 0 {
        return Vec::new();
    }

    let mut all_x: Vec<f64> = boxes.iter().flat_map(|b| [b.min_x, b.max_x]).collect();
    let mut all_y: Vec<f64> = boxes.iter().flat_map(|b| [b.min_y, b.max_y]).collect();
    all_x.sort_by(|a, b| a.total_cmp(b));
    all_y.sort_by(|a, b| a.total_cmp(b));

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

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cluster_indices_separates_a_distant_outlier() {
        let boxes = [
            bx(0.0, 0.0, 1.0, 1.0),
            bx(1.0, 1.0, 2.0, 2.0),
            bx(1.0, 0.0, 2.0, 1.0),
            bx(1000.0, 1000.0, 1001.0, 1001.0),
        ];
        let clusters = cluster_indices(&boxes);
        assert_eq!(clusters, vec![vec![0, 1, 2], vec![3]]);
        assert!(cluster_indices(&[]).is_empty());
        // The empty interior of a room is not a gap: opposite walls meet
        // the side walls at the corners.
        let room = [
            bx(0.0, 0.0, 100.0, 0.0),
            bx(100.0, 0.0, 100.0, 60.0),
            bx(0.0, 60.0, 100.0, 60.0),
            bx(0.0, 0.0, 0.0, 60.0),
        ];
        assert_eq!(cluster_indices(&room).len(), 1);
    }
}
