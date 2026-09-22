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

/// The largest value the *rasterizer* may be handed where the work it does
/// grows with the number.
///
/// `f64::is_finite` is not the line for such a value: 1e150 is a perfectly
/// finite number, and a bulge of 1e-160 over a hundred-unit segment is an
/// arc of radius 1e238. Handing that to tiny-skia as an SVG `A` command did
/// not finish converting to beziers in five minutes at *any* image size.
/// The same bound keeps the viewBox buildable: an entity whose measured box
/// runs past it is held out of the crop decision and listed in the crop's
/// `excluded` (see [`crate::svg`]), because a viewBox 1.45e150 units wide
/// asked tiny-skia's dasher for ~1e149 dashes.
///
/// It is *not* a screen on a coordinate as written. An entity a long way
/// from the rest of the drawing is a drawing problem, not a rasterizer
/// problem, and [`crate::crop`]'s outlier rule is what answers it -- it
/// says so in `report.json`, where dropping the entity silently said
/// nothing. Since 0.3.0.
///
/// The bound is the one [`crate::crop::Rect::is_sane`] already applies to a
/// header's `$EXTMIN`/`$EXTMAX`. No real drawing comes near it -- the
/// Earth's circumference in micrometres is 4e13.
pub const MAX_WORLD_COORDINATE: f64 = 1e15;

/// How many bytes of drawing body one *top-level* entity may emit before
/// its block expansion is cut short.
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
/// A quarter of the document's budget, because that is the thing it is a
/// share of: one part may draw a quarter of everything the renderer emits.
/// A flat 4 MiB was the cap until 0.3.0 shipped, and it was the wrong
/// shape -- a drawing that wraps its content in one block and places it
/// once (a bound XREF, an imported survey, a "whole floor" block) is a
/// single top-level INSERT, so the per-*entity* cap was really a cap on
/// the whole drawing, and past ~65 000 short lines the picture came out
/// empty. 16 MiB is over 200 000 lines from one entity.
///
/// What happens at the cap changed with it: the expansion stops at the
/// next block boundary and **what was drawn is kept**, counted in
/// [`LimitReport::truncated_parts`] and named in [`LimitReport::dropped`].
/// Stopping bounds the work either way; keeping the part is the difference
/// between a legitimate drawing rendering all but its tail and rendering
/// as a blank page.
pub const MAX_ENTITY_SVG_BYTES: usize = MAX_SVG_BODY_BYTES / 4;

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

/// Which cap left one entity out of the picture, or cut it short.
///
/// Serialized in lower snake case, so `report.json` reads
/// `"cap": "entity_points"`. Since 0.3.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Cap {
    /// [`MAX_ENTITY_POINTS`]. The entity is not drawn.
    EntityPoints,
    /// [`MAX_SVG_BODY_BYTES`] was already spent when this entity's turn
    /// came. It is not drawn.
    DocumentBytes,
    /// [`MAX_ENTITY_SVG_BYTES`]. The entity *is* drawn, but its block
    /// expansion stopped at a block boundary, so part of it is missing.
    EntityBytes,
    /// [`MAX_BLOCK_REF_DEPTH`] or [`MAX_BLOCK_REFS`]. A block reference
    /// this entity expands to was not followed.
    BlockRefs,
    /// [`MAX_HATCH_TILE_SPAN`]. The HATCH keeps its outline but not its
    /// pattern fill.
    HatchTile,
    /// A coordinate, radius or angle in the entity is not a real number
    /// (`NaN`, `inf`). Nothing sensible can be drawn from it, so the entity
    /// is not drawn.
    NotANumber,
}

impl Cap {
    /// The name this cap is serialized and printed under.
    pub fn as_str(self) -> &'static str {
        match self {
            Cap::EntityPoints => "entity_points",
            Cap::DocumentBytes => "document_bytes",
            Cap::EntityBytes => "entity_bytes",
            Cap::BlockRefs => "block_refs",
            Cap::HatchTile => "hatch_tile",
            Cap::NotANumber => "not_a_number",
        }
    }
}

/// One entity a cap acted on, so a report can *name* what is missing
/// instead of only counting it.
///
/// Since 0.3.0.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Dropped {
    /// The entity's handle, as `report.json`'s other lists spell it.
    pub handle: String,
    /// Its DXF type name.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Which cap acted, and so whether the entity is missing entirely or
    /// only incomplete.
    pub cap: Cap,
}

/// How many distinct entities [`LimitReport::dropped`] names before it
/// stops collecting.
///
/// The counts stay exact past this; only the naming stops. A corrupt file
/// can drop hundreds of thousands of block references from one handle, and
/// `report.json` is a file a reader is meant to be able to read.
pub const MAX_REPORTED_HANDLES: usize = 100;

/// What [`MAX_ENTITY_POINTS`], [`MAX_BLOCK_REFS`] and the rest took away
/// from one render, so the caller can say so instead of quietly drawing
/// less than the file holds.
///
/// Every count is of *events*, not entities-as-the-file-sees-them: one
/// block referenced twice and dropped twice counts twice. [`dropped`] is
/// the other half -- which entities, at most [`MAX_REPORTED_HANDLES`] of
/// them, once each per cap.
///
/// [`dropped`]: LimitReport::dropped
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
    /// Entities drawn only as far as [`MAX_ENTITY_SVG_BYTES`] reached: the
    /// part is in the picture, but its block expansion stopped early.
    /// Named `oversized_parts` while it meant "dropped whole".
    pub truncated_parts: usize,
    /// Entities not drawn because a coordinate, radius or angle in them is
    /// not a real number. Since 0.3.0.
    pub unreadable_entities: usize,
    /// Which entities the counts above are about, in the order they were
    /// met, one entry per (entity, cap) pair and at most
    /// [`MAX_REPORTED_HANDLES`] of them. Since 0.3.0.
    pub dropped: Vec<Dropped>,
}

impl LimitReport {
    /// Whether any cap engaged at all -- i.e. whether the picture is missing
    /// something the file holds.
    pub fn engaged(&self) -> bool {
        *self != LimitReport::default()
    }

    /// Names one entity a cap acted on, unless it is already named for that
    /// cap or the list is full.
    pub fn note(&mut self, cap: Cap, handle: &str, type_name: &str) {
        if self.dropped.len() >= MAX_REPORTED_HANDLES
            || self
                .dropped
                .iter()
                .any(|d| d.cap == cap && d.handle == handle)
        {
            return;
        }
        self.dropped.push(Dropped {
            handle: handle.to_string(),
            type_name: type_name.to_string(),
            cap,
        });
    }

    /// Up to `n` of the handles named in [`dropped`](Self::dropped), for a
    /// one-line message.
    fn first_handles(&self, n: usize) -> String {
        let named: Vec<&str> = self
            .dropped
            .iter()
            .take(n)
            .map(|d| d.handle.as_str())
            .collect();
        if named.is_empty() {
            return String::new();
        }
        format!(
            " (handles {}{})",
            named.join(", "),
            if self.dropped.len() > n { ", ..." } else { "" }
        )
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
        if self.truncated_parts > 0 {
            parts.push(format!(
                "{} entities drawn only as far as the {} MiB one entity may emit",
                self.truncated_parts,
                MAX_ENTITY_SVG_BYTES / (1024 * 1024)
            ));
        }
        if self.unreadable_entities > 0 {
            parts.push(format!(
                "{} entities whose coordinates are not numbers",
                self.unreadable_entities
            ));
        }
        Some(format!("{}{}", parts.join("; "), self.first_handles(5)))
    }

    /// Folds `other`'s counts into this one -- a sheet composited from a
    /// model render and a paper render reports both.
    pub fn merge(&mut self, other: &LimitReport) {
        self.oversized_entities += other.oversized_entities;
        self.block_refs_dropped += other.block_refs_dropped;
        self.hatch_patterns_dropped += other.hatch_patterns_dropped;
        self.entities_dropped += other.entities_dropped;
        self.truncated_parts += other.truncated_parts;
        self.unreadable_entities += other.unreadable_entities;
        for d in &other.dropped {
            self.note(d.cap, &d.handle, &d.type_name);
        }
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
            truncated_parts: 5,
            unreadable_entities: 6,
            dropped: Vec::new(),
        };
        let s = r.summary().expect("engaged");
        assert!(s.contains("1 entities over"), "{s}");
        assert!(s.contains("2 block references"), "{s}");
        assert!(s.contains("3 hatch patterns"), "{s}");
        assert!(s.contains("4 entities past"), "{s}");
        assert!(s.contains("5 entities drawn only as far"), "{s}");
        assert!(s.contains("6 entities whose coordinates"), "{s}");
    }

    #[test]
    fn the_summary_names_the_handles_it_has() {
        // The point of the list: a reader of the warning can go and look
        // the entity up. Six are noted, five are named and the sixth turns
        // into the ellipsis.
        let mut r = LimitReport {
            oversized_entities: 6,
            ..LimitReport::default()
        };
        for h in ["A1", "A2", "A3", "A4", "A5", "A6"] {
            r.note(Cap::EntityPoints, h, "LWPOLYLINE");
        }
        let s = r.summary().expect("engaged");
        assert!(s.contains("(handles A1, A2, A3, A4, A5, ...)"), "{s}");
    }

    #[test]
    fn a_handle_is_noted_once_per_cap_and_the_list_has_a_ceiling() {
        let mut r = LimitReport::default();
        // The same entity met twice under one cap is one entry; under two
        // caps it is two.
        r.note(Cap::BlockRefs, "7F", "INSERT");
        r.note(Cap::BlockRefs, "7F", "INSERT");
        r.note(Cap::EntityBytes, "7F", "INSERT");
        assert_eq!(r.dropped.len(), 2, "{:?}", r.dropped);

        for i in 0..MAX_REPORTED_HANDLES * 2 {
            r.note(Cap::DocumentBytes, &format!("{i:X}"), "LINE");
        }
        assert_eq!(r.dropped.len(), MAX_REPORTED_HANDLES);
    }

    #[test]
    fn merge_adds_every_field() {
        let mut a = LimitReport {
            oversized_entities: 1,
            block_refs_dropped: 1,
            hatch_patterns_dropped: 1,
            entities_dropped: 1,
            truncated_parts: 1,
            unreadable_entities: 1,
            dropped: vec![Dropped {
                handle: "A".into(),
                type_name: "LINE".into(),
                cap: Cap::NotANumber,
            }],
        };
        a.merge(&a.clone());
        assert_eq!(
            a,
            LimitReport {
                oversized_entities: 2,
                block_refs_dropped: 2,
                hatch_patterns_dropped: 2,
                entities_dropped: 2,
                truncated_parts: 2,
                unreadable_entities: 2,
                // The same entity under the same cap is still one entry:
                // a merged report names entities, not events.
                dropped: vec![Dropped {
                    handle: "A".into(),
                    type_name: "LINE".into(),
                    cap: Cap::NotANumber,
                }],
            }
        );
    }

    #[test]
    fn one_entity_may_draw_a_quarter_of_what_the_document_may() {
        // The regression W0 left: a flat 4 MiB per entity meant a drawing
        // that is one INSERT of one big block rendered empty past about
        // 65 000 lines. The per-entity budget is a share of the document's
        // now, and a `<line>` is around 64 bytes, so this is over 200 000
        // of them from a single entity.
        assert_eq!(MAX_ENTITY_SVG_BYTES * 4, MAX_SVG_BODY_BYTES);
        assert_eq!(MAX_ENTITY_SVG_BYTES / 64, 262_144);
    }
}
