//! A DWG/DXF drawing as a package an LLM or a vision model can read.
//!
//! [`uncad`] parses a drawing into the neutral [`uncad_model`] entity model
//! and draws nothing; `iron-render-cad` draws that model and knows nothing
//! of who looks at the picture. This crate is the consumer between them that
//! writes for one kind of reader: images sized to a model's patch budget,
//! and the exact numbers a picture cannot give.
//!
//! What is here so far: [`fonts`], the face the package draws and measures
//! text with; [`geom`], the arithmetic its records carry -- a polyline's
//! length and area with its bulge arcs, whether an outline crosses itself,
//! the map from an entity's own plane to the world; and [`text`], the
//! string a reader sees for a text the file stores with its codes.
//!
//! # Licence
//!
//! GPL-3.0-or-later, like the rest of the workspace, and OFL-1.1 for the
//! bundled font (`fonts/OFL-NotoSansKR.txt`).

pub mod fonts;
pub mod geom;
pub mod text;
