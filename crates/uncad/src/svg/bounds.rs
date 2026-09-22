//! The per-entity bounding box the renderer measures.
//!
//! `svg.rs` fills one [`Box2D`] per drawn entity (`entity_box`) and the crop
//! rule turns them into [`crate::crop::Rect`]s: the outlier guard in
//! `crate::crop` compares sizes and distances to the median centre, and the
//! frame split the export does (`crop::detached_groups`) groups them with a
//! square-grid union-find.

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
