//! The parts of [Tileling](https://github.com/alexandreravelli/tileling) that need no X server:
//! the layouts and their geometry, and the configuration file.
//!
//! - [`layout`] places zones in a work area, lays out the picker and tells moves from resizes.
//! - [`config`] finds and reads `~/.config/tileling/config.toml`.
//!
//! The `tileling` program adds the X11 side: following drags, the picker, the preview and the
//! tiling assistant.

pub mod config;
pub mod layout;
