//! Every bound the renderer puts on a number that came out of the drawing
//! file, in one place.
//!
//! A DWG/DXF is untrusted input: a count, a scale or a spacing in it is
//! whatever bytes happened to be there, and a corrupt one turns straight
//! into an allocation size or a loop bound. Left alone that is not a wrong
//! picture but a dead process. One flipped byte of `example_2000.dwg`
//! (offset 130005, `0x80` -> `0x4B`) redirects the `CIRKLO_PUNKTOJ` block
//! record's owned-entity chain so the block holds eight INSERTs *of
//! itself* beside its fifty drawable entities. The file still parsed in
//! 0.03 s; rendering it then spent three minutes growing one SVG string
//! until a 12,074,460,607-byte reallocation aborted the process. The
//! regression is `crates/uncad/tests/limits.rs`.
//!
//! So each such number is capped here, the caps are named, and every render
//! says through [`LimitReport`] what a cap took away. The rule this module
//! follows: **a malformed file may cost a missing entity and a note saying
//! so; it may never cost the process.**
//!
//! The caps are deliberately far above anything a real drawing reaches --
//! the largest sample this project renders (`AutoCADSamples5.dwg`, 18 MB of
//! SVG, ~40,000 entities) uses under a third of the output budget and none
//! of the others.
//!
//! Since 0.3.0.

use serde::{Deserialize, Serialize};

/// How deep block references may nest before rendering stops following them.
///
/// A block that references itself (directly, or around a cycle) is otherwise
/// unbounded recursion. Real drawings nest a handful of levels; twenty is
/// past every file this project has been checked against.
pub const MAX_BLOCK_REF_DEPTH: u32 = 20;

/// How many block references one render may expand in total.
///
/// [`MAX_BLOCK_REF_DEPTH`] bounds nesting but not *breadth*: a block holding
/// nine references to itself fans out to 9^20 instantiations before the
/// depth cap is ever reached. This is the budget that actually stops that,
/// decremented once per expanded reference and never restored.
pub const MAX_BLOCK_REFS: u32 = 100_000;

/// How many bytes of drawing body one render may emit.
///
/// The backstop behind every other cap here, and the one that bounds the
/// *allocation*: whatever a file asks the renderer to draw, the SVG string
/// it builds stops growing at this size. 64 MiB is three and a half times
/// the largest real drawing measured (18 MB) and leaves the peak -- the
/// string plus the copy a `String` makes when it grows -- around 200 MB.
pub const MAX_SVG_BODY_BYTES: usize = 64 * 1024 * 1024;

/// How many points one entity may contribute to the picture.
///
/// A polyline's vertex count, a spline's control points, a hatch boundary's
/// edges and a 3D solid's wireframe all come from the file. One entity past
/// this many points is left out whole rather than drawn: 100,000 points is
/// already a path no raster image at any sane size can resolve, and drawing
/// it costs both the emitted text and the rasterizer's work.
pub const MAX_ENTITY_POINTS: usize = 100_000;

/// How much larger than the shape it fills a hatch pattern's tile may be.
///
/// A HATCH's pattern is emitted as an SVG `<pattern>` whose tile is the
/// pattern's own line spacing, in drawing units. The rasterizer allocates a
/// pixmap for that tile at the *device* scale of the element being filled,
/// so a spacing of 1e12 over a ten-unit boundary asks it for a pixmap 1e11
/// pixels on a side. A tile bigger than this multiple of the boundary's
/// diagonal cannot show more than a single line anyway, so the pattern is
/// dropped and the hatch keeps its outline.
pub const MAX_HATCH_TILE_SPAN: f64 = 16.0;

/// What [`MAX_ENTITY_POINTS`], [`MAX_BLOCK_REFS`] and the rest took away
/// from one render, so the caller can say so instead of quietly drawing
/// less than the file holds.
///
/// Every field counts *events*, not entities-as-the-file-sees-them: one
/// block referenced twice and dropped twice counts twice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LimitReport {
    /// Entities left out because they held more than [`MAX_ENTITY_POINTS`]
    /// points.
    pub oversized_entities: usize,
    /// Block references not expanded because [`MAX_BLOCK_REF_DEPTH`] or
    /// [`MAX_BLOCK_REFS`] was reached.
    pub block_refs_dropped: usize,
    /// HATCH pattern fills reduced to an outline by [`MAX_HATCH_TILE_SPAN`].
    pub hatch_patterns_dropped: usize,
    /// Entities not drawn at all because [`MAX_SVG_BODY_BYTES`] was already
    /// spent when their turn came.
    pub entities_dropped: usize,
}

impl LimitReport {
    /// Whether any cap engaged at all -- i.e. whether the picture is missing
    /// something the file holds.
    pub fn engaged(&self) -> bool {
        *self != LimitReport::default()
    }

    /// A one-line, human-readable account of what was dropped, or `None`
    /// when nothing was.
    pub fn summary(&self) -> Option<String> {
        if !self.engaged() {
            return None;
        }
        let mut parts = Vec::new();
        if self.oversized_entities > 0 {
            parts.push(format!(
                "{} entities over {} points",
                self.oversized_entities, MAX_ENTITY_POINTS
            ));
        }
        if self.block_refs_dropped > 0 {
            parts.push(format!(
                "{} block references past the {} nesting / {} expansion limit",
                self.block_refs_dropped, MAX_BLOCK_REF_DEPTH, MAX_BLOCK_REFS
            ));
        }
        if self.hatch_patterns_dropped > 0 {
            parts.push(format!(
                "{} hatch patterns whose tile dwarfs the shape (outline kept)",
                self.hatch_patterns_dropped
            ));
        }
        if self.entities_dropped > 0 {
            parts.push(format!(
                "{} entities past the {} MiB the renderer emits",
                self.entities_dropped,
                MAX_SVG_BODY_BYTES / (1024 * 1024)
            ));
        }
        Some(parts.join("; "))
    }

    /// Folds `other`'s counts into this one -- a sheet composited from a
    /// model render and a paper render reports both.
    pub fn merge(&mut self, other: &LimitReport) {
        self.oversized_entities += other.oversized_entities;
        self.block_refs_dropped += other.block_refs_dropped;
        self.hatch_patterns_dropped += other.hatch_patterns_dropped;
        self.entities_dropped += other.entities_dropped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_report_is_not_engaged_and_summarizes_to_nothing() {
        let r = LimitReport::default();
        assert!(!r.engaged());
        assert_eq!(r.summary(), None);
    }

    #[test]
    fn the_summary_names_every_cap_that_engaged() {
        let r = LimitReport {
            oversized_entities: 1,
            block_refs_dropped: 2,
            hatch_patterns_dropped: 3,
            entities_dropped: 4,
        };
        let s = r.summary().expect("engaged");
        assert!(s.contains("1 entities over"), "{s}");
        assert!(s.contains("2 block references"), "{s}");
        assert!(s.contains("3 hatch patterns"), "{s}");
        assert!(s.contains("4 entities past"), "{s}");
    }

    #[test]
    fn merge_adds_every_field() {
        let mut a = LimitReport {
            oversized_entities: 1,
            block_refs_dropped: 1,
            hatch_patterns_dropped: 1,
            entities_dropped: 1,
        };
        a.merge(&a.clone());
        assert_eq!(
            a,
            LimitReport {
                oversized_entities: 2,
                block_refs_dropped: 2,
                hatch_patterns_dropped: 2,
                entities_dropped: 2,
            }
        );
    }
}
