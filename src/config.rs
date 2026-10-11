//! Configuration: ~/.config/tileling/config.toml, else the first tileling/config.toml in
//! $XDG_CONFIG_DIRS (default /etc/xdg), else the built-in defaults. Unknown keys and bad values
//! are reported and ignored one by one: one typo never resets the whole file.

use crate::layout;
use std::path::PathBuf;

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
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let user = home.join("tileling/config.toml");
    if user.exists() {
        return Some(user);
    }
    let dirs = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    dirs.split(':').map(|d| PathBuf::from(d).join("tileling/config.toml")).find(|p| p.exists())
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
                .filter(|&v| (4..=400).contains(&v))
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
                if !names.is_empty() {
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
    fn unknown_layouts_are_skipped() {
        let (cfg, warnings) = quiet("layouts = [\"quarters\", \"spiral\"]\n");
        assert_eq!(cfg.layouts, vec!["quarters".to_string()]);
        assert_eq!(warnings.len(), 1);
    }
}
