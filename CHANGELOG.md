# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/alexandreravelli/tileling/compare/v0.1.0...v0.2.0) - 2026-10-11

### Added

- *(cli)* add --reload, --quit and --replace
- *(cli)* list configuration problems with --check-config [FILE]

### Changed

- report errors with anyhow and context
- *(config)* [**breaking**] return problems with their line instead of printing them
- move the layouts and the configuration into a library

### Documentation

- *(readme)* commands, --check-config, crates.io and how to contribute
- use the Keep a Changelog format
- document the library API with tested examples

### Other

- Replace the README screenshot with docs/layout-picker.png

## [0.1.0](https://github.com/alexandreravelli/tileling/releases/tag/v0.1.0) - 2026-10-11

### Added

- Ubuntu 26.10-style window tiling for XFCE (xfwm4) on Debian 13, written in Rust.
- Layout picker: drag a window towards the top of the screen and drop it on a zone. Halves, half and quarters, quarters, two thirds and one third; rows can be added.
- Live preview of the hovered zone, translucent with a compositor and an outline without one. Overlays follow the compositor when it is turned on or off.
- Tiling assistant: the other windows of the desktop are offered, by icon, for the zones that are left. Escape or a click elsewhere closes it; only Escape is grabbed.
- Tiles inside each monitor's work area (panels and docks are left alone), with even gaps. Maximized windows can be dragged into the picker; Escape during a drag cancels it.
- Commented TOML configuration (gap, trigger distance, layouts, accent colour, assistant). Unknown keys and bad values are reported and skipped one by one.
- One static binary (musl), about 1 MB, about 1 MB of RAM and no CPU while idle.
