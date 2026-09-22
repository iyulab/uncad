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

/// How deep an entity's owned-*sub*entity chain may recurse on the way in.
///
/// Only one kind of nesting is real: an INSERT owns its ATTRIBs, which are
/// themselves entities, so converting an INSERT converts them too. A file
/// whose handles have been damaged can point an INSERT's subentity chain
/// back at the INSERT (or around a cycle of them), and that conversion then
/// recurses until the stack runs out -- a 512 MB stack was not enough for
/// one byte-flipped `example_2000.dwg`. One level of nesting is what the
/// format has; two is the cap.
pub const MAX_SUBENTITY_DEPTH: u32 = 2;

/// How many subentities one owned-subentity walk may hand back.
///
/// The same damaged handles can make the chain a *ring* instead of a list,
/// which is not recursion but a loop that never ends. No real entity owns
/// anything like this many children.
pub const MAX_OWNED_SUBENTITIES: usize = 100_000;

/// How many bytes of drawing body one render may emit.
///
/// The backstop behind every other cap here, and the one that bounds the
/// *allocation*: whatever a file asks the renderer to draw, the SVG string
/// it builds stops growing at this size. 64 MiB is three and a half times
/// the largest real drawing measured (18 MB) and leaves the peak -- the
/// string plus the copy a `String` makes when it grows -- around 200 MB.
pub const MAX_SVG_BODY_BYTES: usize = 64 * 1024 * 1024;

/// The largest world coordinate, radius or size the renderer will draw
/// with.
///
/// `f64::is_finite` is not the line: 1e150 is a perfectly finite number,
/// and one entity carrying it drags the measured extents -- and so the
/// viewBox, the automatic stroke width and every length the rasterizer
/// derives from them -- up with it. A fuzzed `example_2000.dwg` did exactly
/// that: a 590 KB SVG with a viewBox 1.45e150 units wide, whose dashed
/// strokes then asked tiny-skia's dasher for ~1e149 dashes. `to_png` had
/// not returned after five minutes.
///
/// The bound is the one [`crate::crop::Rect::is_sane`] already applies to a
/// header's `$EXTMIN`/`$EXTMAX`, so an entity and a header extent are now
/// held to the same standard. No real drawing comes near it -- the Earth's
/// circumference in micrometres is 4e13.
pub const MAX_WORLD_COORDINATE: f64 = 1e15;

/// How many bytes of drawing body one *top-level* entity may emit before
/// it is left out of the picture altogether.
///
/// [`MAX_SVG_BODY_BYTES`] bounds the document; this bounds any one part of
/// it, and it is the package that needs it. A tile rasterizes every part
/// whose extent touches it, so one INSERT that expanded into a
/// picture-wide 60 MB part is re-assembled and re-parsed for every tile at
/// every zoom level, on up to sixteen threads at once: a fuzzed
/// `example_2000.dwg` made `uncad export` peak at 5.7 GB and run for 132 s
/// that way. The largest real drawing has 18 MB of body but spread over
/// 40 000 small parts, so each tile keeps only a handful and the same
/// export peaks at 402 MB in 3.2 s -- it is one entity covering everything
/// that costs, not a large drawing.
///
/// Rendering stops at the cap, so building the part is bounded work; the
/// part is then dropped whole rather than shown half-drawn, and counted in
/// [`LimitReport::oversized_parts`]. Four MiB is tens of thousands of
/// elements from a single entity -- far past anything a real one draws.
pub const MAX_ENTITY_SVG_BYTES: usize = 4 * 1024 * 1024;

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
    /// Entities left out because drawing one of them would have taken more
    /// than [`MAX_ENTITY_SVG_BYTES`].
    pub oversized_parts: usize,
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
        if self.oversized_parts > 0 {
            parts.push(format!(
                "{} entities that would each have drawn more than {} MiB",
                self.oversized_parts,
                MAX_ENTITY_SVG_BYTES / (1024 * 1024)
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
        self.oversized_parts += other.oversized_parts;
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
            oversized_parts: 5,
        };
        let s = r.summary().expect("engaged");
        assert!(s.contains("1 entities over"), "{s}");
        assert!(s.contains("2 block references"), "{s}");
        assert!(s.contains("3 hatch patterns"), "{s}");
        assert!(s.contains("4 entities past"), "{s}");
        assert!(s.contains("5 entities that would each"), "{s}");
    }

    #[test]
    fn merge_adds_every_field() {
        let mut a = LimitReport {
            oversized_entities: 1,
            block_refs_dropped: 1,
            hatch_patterns_dropped: 1,
            entities_dropped: 1,
            oversized_parts: 1,
        };
        a.merge(&a.clone());
        assert_eq!(
            a,
            LimitReport {
                oversized_entities: 2,
                block_refs_dropped: 2,
                hatch_patterns_dropped: 2,
                entities_dropped: 2,
                oversized_parts: 2,
            }
        );
    }
}
