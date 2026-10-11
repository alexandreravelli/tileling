//! Configuration: ~/.config/tileling/config.toml, else the first tileling/config.toml in
//! $XDG_CONFIG_DIRS (default /etc/xdg), else the built-in defaults. Unknown keys and bad values
//! are reported and ignored one by one: one typo never resets the whole file.

use crate::layout;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Pixels kept between tiled windows and around them.
    pub gap: u32,
    /// Distance from the top of the work area, while dragging a window, that opens the picker.
    pub trigger: u32,
    /// After tiling, offer the other windows for the zones that are left.
    pub assist: bool,
    /// Layout names, in the picker's order.
    pub layouts: Vec<String>,
    /// "#RRGGBB" accent of the hovered zone and of the preview.
    pub accent: [u8; 3],
}

impl Default for Config {
    fn default() -> Self {
        Config {
            gap: 8,
            trigger: 40,
            assist: true,
            layouts: ["halves", "half-quarters", "quarters-half", "quarters", "two-thirds", "one-third"]
                .map(String::from)
                .to_vec(),
            accent: [0x35, 0x84, 0xe4],
        }
    }
}

pub fn path() -> Option<PathBuf> {
    path_in(&|k| std::env::var(k).ok(), &|p| p.exists())
}

/// The XDG Base Directory lookup, with the environment and the file system passed in.
/// Relative directories are ignored, as the specification asks.
fn path_in(env: &dyn Fn(&str) -> Option<String>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let absolute = |v: String| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    let home =
        env("XDG_CONFIG_HOME").and_then(absolute).or_else(|| env("HOME").and_then(absolute).map(|h| h.join(".config")));
    let system = env("XDG_CONFIG_DIRS").filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    home.into_iter()
        .chain(system.split(':').filter_map(|d| absolute(d.to_string())))
        .map(|d| d.join("tileling/config.toml"))
        .find(|p| exists(p))
}

pub fn load() -> Config {
    let Some(p) = path() else { return Config::default() };
    match std::fs::read_to_string(&p) {
        Ok(text) => parse(&text, &mut |msg| eprintln!("tileling: {}: {msg}", p.display())),
        Err(e) => {
            eprintln!("tileling: {}: {e}", p.display());
            Config::default()
        }
    }
}

pub fn parse(text: &str, warn: &mut dyn FnMut(String)) -> Config {
    let mut cfg = Config::default();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            warn(format!("{e}; using the defaults"));
            return cfg;
        }
    };
    for (key, value) in table {
        let ok = match key.as_str() {
            "gap" => value.as_integer().and_then(|v| u32::try_from(v).ok()).filter(|&v| v <= 200).map(|v| cfg.gap = v),
            "trigger" => value
                .as_integer()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|&v| (4..=200).contains(&v))
                .map(|v| cfg.trigger = v),
            "assist" => value.as_bool().map(|v| cfg.assist = v),
            "accent" => value.as_str().and_then(color).map(|v| cfg.accent = v),
            "layouts" => value.as_array().map(|items| {
                let names: Vec<String> = items
                    .iter()
                    .filter_map(|i| match i.as_str() {
                        Some(n) if layout::layout(n).is_some() => Some(n.to_string()),
                        _ => {
                            warn(format!("unknown layout {i:?}"));
                            None
                        }
                    })
                    .collect();
                if names.is_empty() {
                    warn("no usable layout in \"layouts\"; keeping the default ones".into());
                } else {
                    cfg.layouts = names;
                }
            }),
            _ => {
                warn(format!("unknown key {key:?}"));
                Some(())
            }
        };
        if ok.is_none() {
            warn(format!("invalid value for {key:?}: {value:?}"));
        }
    }
    cfg
}

/// "#RRGGBB" (ASCII hex digits only).
fn color(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(text: &str) -> (Config, Vec<String>) {
        let mut warnings = Vec::new();
        let cfg = parse(text, &mut |m| warnings.push(m));
        (cfg, warnings)
    }

    #[test]
    fn default_file_matches_defaults() {
        let (cfg, warnings) = quiet(include_str!("../config/default.toml"));
        assert_eq!(cfg, Config::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn one_bad_key_keeps_the_others() {
        let (cfg, warnings) = quiet("gap = 12\ngapp = 3\naccent = \"#ff8000\"\ntrigger = -1\n");
        assert_eq!(cfg.gap, 12);
        assert_eq!(cfg.accent, [0xff, 0x80, 0x00]);
        assert_eq!(cfg.trigger, Config::default().trigger);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn non_ascii_colour_is_rejected_without_panicking() {
        assert_eq!(color("#e95\u{a0}0"), None);
        assert_eq!(color("#3584e4"), Some([0x35, 0x84, 0xe4]));
    }

    #[test]
    fn empty_layouts_keep_the_defaults() {
        let (cfg, warnings) = quiet("layouts = []\n");
        assert_eq!(cfg.layouts, Config::default().layouts);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn config_lookup_follows_xdg() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        let only = |file: &'static str| move |p: &Path| p == Path::new(file);
        let home = env(&[("HOME", "/home/u")]);
        assert_eq!(
            path_in(&home, &only("/home/u/.config/tileling/config.toml")),
            Some("/home/u/.config/tileling/config.toml".into())
        );
        assert_eq!(
            path_in(&home, &only("/etc/xdg/tileling/config.toml")),
            Some("/etc/xdg/tileling/config.toml".into())
        );
        // No HOME at all: the system directories are still searched.
        assert_eq!(
            path_in(&env(&[]), &only("/etc/xdg/tileling/config.toml")),
            Some("/etc/xdg/tileling/config.toml".into())
        );
        // A relative XDG_CONFIG_HOME is ignored, HOME is used instead.
        let relative = env(&[("XDG_CONFIG_HOME", "conf"), ("HOME", "/home/u")]);
        assert_eq!(
            path_in(&relative, &only("/home/u/.config/tileling/config.toml")),
            Some("/home/u/.config/tileling/config.toml".into())
        );
        let dirs = env(&[("XDG_CONFIG_DIRS", "/etc/xdg/cloudagent:/etc/xdg")]);
        assert_eq!(
            path_in(&dirs, &|p: &Path| p.starts_with("/etc/xdg")),
            Some("/etc/xdg/cloudagent/tileling/config.toml".into())
        );
    }

    #[test]
    fn unknown_layouts_are_skipped() {
        let (cfg, warnings) = quiet("layouts = [\"quarters\", \"spiral\"]\n");
        assert_eq!(cfg.layouts, vec!["quarters".to_string()]);
        assert_eq!(warnings.len(), 1);
    }
}
