//! pixlens: decode an image, fit it to a box of terminal cells, draw it.
//!
//! - [`cli`] parses the command line and [`commands`] runs each command:
//!   `show` (draw in any terminal), `render` (the JSON the Claude Code
//!   plugin reads), `copy` and `open` (hand an image to the desktop),
//!   `menu` and `overlay` (Pixlens Overlay, macOS) and `setup`.
//! - [`load`], [`fit`], [`blocks`], [`protocol`] and [`cache`] do the
//!   image work; [`system`] and [`overlay`] talk to the desktop.
//! - [`platform`] names the desktop and the environment, so every
//!   platform's branch can be tested on one machine.

pub mod blocks;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod fit;
pub mod load;
pub mod overlay;
pub mod platform;
pub mod protocol;
pub mod system;

pub use fit::{CellBox, fit_cells};
pub use load::load;
