# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/alexandreravelli/tileling/compare/v0.1.0...v0.2.0) - 2026-10-11

### Added

- `tileling --reload` makes the running Tileling read its configuration again, `--quit` stops it and `--replace` starts a new one that takes over: no more `pkill -x tileling; tileling &`.
- `tileling --check-config [FILE]` lists the problems of the configuration file as `file:line: message`, then the settings in use, and exits with 1 when there is a problem.
- The crate is packaged for crates.io (`cargo install tileling`), with its layout and configuration library documented for docs.rs.

### Changed

- Configuration messages say what was expected and suggest close names: `invalid value for "trigger": -1 (expected a whole number from 4 to 200)`, `unknown setting "triger". Did you mean "trigger"?`
- Clearer error messages, for example when there is no X server to connect to.
- Each release runs every check (tests on Xvfb, Clippy, MSRV, cargo-deny, the static build) before anything is published, and also carries `tileling_amd64.deb` under a name that does not change.
- Library: `config::parse` returns the settings and their warnings instead of taking a callback, and `config::load` returns `Result<Loaded, ConfigError>`.

## [0.1.0](https://github.com/alexandreravelli/tileling/releases/tag/v0.1.0) - 2026-10-11

### Added

- Ubuntu 26.10-style window tiling for XFCE (xfwm4) on Debian 13, written in Rust.
- Layout picker: drag a window towards the top of the screen and drop it on a zone. Halves, half and quarters, quarters, two thirds and one third; rows can be added.
- Live preview of the hovered zone, translucent with a compositor and an outline without one. Overlays follow the compositor when it is turned on or off.
- Tiling assistant: the other windows of the desktop are offered, by icon, for the zones that are left. Escape or a click elsewhere closes it; only Escape is grabbed.
- Tiles inside each monitor's work area (panels and docks are left alone), with even gaps. Maximized windows can be dragged into the picker; Escape during a drag cancels it.
- Commented TOML configuration (gap, trigger distance, layouts, accent colour, assistant). Unknown keys and bad values are reported and skipped one by one.
- One static binary (musl), about 1 MB, about 1 MB of RAM and no CPU while idle.
