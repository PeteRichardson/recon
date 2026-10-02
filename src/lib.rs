//! `recon`: a terminal log viewer.

// The public surface stays honest as more `_range` variants land. `pub`
// suppresses the dead-code warning outright in a lib crate, which is how five
// `Document` methods, `ActiveFilters::set_enabled` and two `FileView` methods
// reached #76 without ever surfacing on their own.
//
// The lint has a blind spot worth knowing (#166): a `pub fn` on a type that is
// itself nominally `pub` counts as reachable even when the type sits in a
// private module and nothing exports it, so two dozen `FileView` methods went
// unaudited for as long as `FileView` was `pub`. A type in a private module
// should be `pub(crate)`; the lint then sees every method on it.
#![warn(unreachable_pub)]

pub mod batch;
pub mod clipboard;
pub mod config;
pub mod document;
pub mod editor;
pub mod emit;
pub mod filter;
pub mod filtersets;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod generate;
pub mod help;
mod hex;
pub mod keymap;
pub mod panes;
mod path;
pub mod scan;
pub mod startup;
pub mod syntax;
mod toml_fmt;
mod widgets;

mod app;

pub use app::App;
pub use config::Config;
