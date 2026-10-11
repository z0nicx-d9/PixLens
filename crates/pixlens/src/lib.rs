//! Decode an image, fit it to a box of terminal cells and draw it.
//!
//! The binary uses this for `pixlens show` (draw in any terminal),
//! `pixlens render` (the JSON the Claude Code mod reads), and `copy` and
//! `open` (hand an image to the clipboard or the default viewer), and
//! `overlay` (real pictures over block previews in Apple's Terminal).

pub mod blocks;
pub mod cache;
pub mod fit;
pub mod load;
pub mod overlay;
pub mod protocol;
pub mod system;

pub use fit::{CellBox, fit_cells};
pub use load::load;
