//! Configuration: ~/.config/tileling/config.toml, else the first tileling/config.toml in
//! $`XDG_CONFIG_DIRS` (default /etc/xdg), else the built-in defaults. Unknown keys and bad values
//! are reported and ignored one by one: one typo never resets the whole file.

use crate::layout;
use std::fmt;
use std::io;
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

/// The settings of the file, in the order of the default file.
pub const KEYS: [&str; 5] = ["gap", "trigger", "assist", "layouts", "accent"];

/// A problem in the configuration file. The setting is skipped; the others still apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The line of the problem in the file, starting at 1, when it is known.
    pub line: Option<usize>,
    /// What is wrong, and what Tileling does about it.
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// The configuration file could not be read.
#[derive(Debug, thiserror::Error)]
#[error("cannot read {}: {error}", path.display())]
pub struct ConfigError {
    /// The configuration file.
    pub path: PathBuf,
    /// Why it cannot be read.
    pub error: io::Error,
}

/// A configuration, where it comes from, and the problems found in its file.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The file read, or None when there is none and the defaults apply.
    pub path: Option<PathBuf>,
    /// The settings: the file's valid ones, the defaults for the others.
    pub config: Config,
    /// Every setting of the file that was skipped, in file order.
    pub warnings: Vec<Warning>,
}

/// The configuration file in use: `~/.config/tileling/config.toml`, else the first
/// `tileling/config.toml` of `$XDG_CONFIG_DIRS` (default `/etc/xdg`); None when there is none.
#[must_use]
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

/// Loads the configuration file in use ([`path`]), or the defaults when there is none.
///
/// # Errors
///
/// Returns a [`ConfigError`] when the file exists but cannot be read.
pub fn load() -> Result<Loaded, ConfigError> {
    match path() {
        Some(path) => load_from(&path),
        None => Ok(Loaded { path: None, config: Config::default(), warnings: Vec::new() }),
    }
}

/// Loads the configuration file at `path`.
///
/// # Errors
///
/// Returns a [`ConfigError`] when the file cannot be read.
pub fn load_from(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|error| ConfigError { path: path.to_path_buf(), error })?;
    let (config, warnings) = parse(&text);
    Ok(Loaded { path: Some(path.to_path_buf()), config, warnings })
}

/// Reads a configuration file's text. Every valid setting applies; an unknown key or a bad value
/// is reported and skipped, so one typo never resets the whole file.
///
/// ```
/// use tileling::config;
///
/// let (cfg, warnings) = config::parse("gap = 12\ngapp = 3\ntrigger = -1\n");
/// assert_eq!((cfg.gap, cfg.trigger), (12, 40), "trigger keeps its default");
/// let warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
/// assert_eq!(warnings, [
///     "line 2: unknown setting \"gapp\". Did you mean \"gap\"?",
///     "line 3: invalid value for \"trigger\": -1 (expected a whole number from 4 to 200)",
/// ]);
/// ```
#[must_use]
pub fn parse(text: &str) -> (Config, Vec<Warning>) {
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map(|span| text[..span.start].matches('\n').count() + 1);
            warnings.push(Warning { line, message: format!("{}; using the defaults", e.message().trim()) });
            return (cfg, warnings);
        }
    };
    let number = |value: &toml::Value, range: std::ops::RangeInclusive<u32>| {
        value.as_integer().and_then(|v| u32::try_from(v).ok()).filter(|v| range.contains(v))
    };
    for (key, value) in table {
        let line = key_line(text, &key);
        let mut warn = |message: String| warnings.push(Warning { line, message });
        let expected = match key.as_str() {
            "gap" => number(&value, 0..=200).map(|v| cfg.gap = v).ok_or("a whole number from 0 to 200"),
            "trigger" => number(&value, 4..=200).map(|v| cfg.trigger = v).ok_or("a whole number from 4 to 200"),
            "assist" => value.as_bool().map(|v| cfg.assist = v).ok_or("true or false"),
            "accent" => value.as_str().and_then(color).map(|v| cfg.accent = v).ok_or("a colour like \"#3584e4\""),
            "layouts" => match value.as_array() {
                Some(items) => {
                    let mut names = Vec::new();
                    for item in items {
                        match item.as_str() {
                            Some(n) if layout::layout(n).is_some() => names.push(n.to_string()),
                            _ => {
                                let names: Vec<&str> = layout::LAYOUTS.iter().map(|l| l.name).collect();
                                warn(format!("unknown layout {}{}", show(item), hint(&show(item), &names)));
                            }
                        }
                    }
                    if names.is_empty() {
                        warn("no usable layout in \"layouts\"; keeping the default ones".into());
                    } else {
                        cfg.layouts = names;
                    }
                    Ok(())
                }
                None => Err("a list of layout names"),
            },
            _ => {
                warn(format!("unknown setting \"{key}\"{}", hint(&key, &KEYS)));
                Ok(())
            }
        };
        if let Err(expected) = expected {
            warn(format!("invalid value for \"{key}\": {} (expected {expected})", show(&value)));
        }
    }
    // Tables are read in key order: report in file order.
    warnings.sort_by_key(|w| w.line.unwrap_or(usize::MAX));
    (cfg, warnings)
}

impl fmt::Display for Config {
    /// The settings as a configuration file would write them.
    ///
    /// ```
    /// use tileling::config::{self, Config};
    ///
    /// let text = Config::default().to_string();
    /// assert!(text.starts_with("gap = 8\ntrigger = 40\n"));
    /// assert_eq!(config::parse(&text), (Config::default(), Vec::new()));
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.accent;
        writeln!(f, "gap = {}", self.gap)?;
        writeln!(f, "trigger = {}", self.trigger)?;
        writeln!(f, "assist = {}", self.assist)?;
        let names: Vec<String> = self.layouts.iter().map(|n| format!("\"{n}\"")).collect();
        writeln!(f, "layouts = [{}]", names.join(", "))?;
        writeln!(f, "accent = \"#{r:02x}{g:02x}{b:02x}\"")
    }
}

/// A value as the file writes it, for messages.
fn show(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("{s:?}"),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(items) => format!("[{}]", items.iter().map(show).collect::<Vec<_>>().join(", ")),
        toml::Value::Table(_) => "a table".into(),
    }
}

/// The line (from 1) where `key` is set.
fn key_line(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|l| l.trim().strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('=')))
        .map(|i| i + 1)
}

/// ". Did you mean \"x\"?" when one of `names` is one or two typos away from `word`.
fn hint(word: &str, names: &[&str]) -> String {
    let word = word.trim_matches('"');
    names
        .iter()
        .map(|n| (distance(word, n), *n))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map_or_else(String::new, |(_, n)| format!(". Did you mean \"{n}\"?"))
}

/// Edits (insertion, deletion, substitution, swap of two neighbours) from `a` to `b`.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut rows = vec![(0..=b.len()).collect::<Vec<usize>>()];
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (rows[i - 1][j] + 1).min(row[j - 1] + 1).min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(rows[i - 2][j - 2] + 1);
            }
        }
        rows.push(row);
    }
    rows[a.len()][b.len()]
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
        let (cfg, warnings) = parse(text);
        (cfg, warnings.iter().map(ToString::to_string).collect())
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
        let (cfg, warnings) = quiet("layouts = [\"quarters\", \"spiral\", \"halfs\"]\n");
        assert_eq!(cfg.layouts, vec!["quarters".to_string()]);
        assert_eq!(
            warnings,
            ["line 1: unknown layout \"spiral\"", "line 1: unknown layout \"halfs\". Did you mean \"halves\"?"]
        );
    }

    #[test]
    fn messages_say_what_is_expected() {
        let text = "assist = \"yes\"\naccent = \"blue\"\nlayouts = \"halves\"\n[extra]\nx = 1\n";
        let (cfg, warnings) = quiet(text);
        assert_eq!(cfg, Config::default());
        assert_eq!(
            warnings,
            [
                "line 1: invalid value for \"assist\": \"yes\" (expected true or false)",
                "line 2: invalid value for \"accent\": \"blue\" (expected a colour like \"#3584e4\")",
                "line 3: invalid value for \"layouts\": \"halves\" (expected a list of layout names)",
                "unknown setting \"extra\"",
            ]
        );
    }

    #[test]
    fn a_file_that_is_not_toml_gives_the_defaults() {
        let (cfg, warnings) = quiet("gap = 4\naccent = \"#fff\n");
        assert_eq!(cfg, Config::default());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("line 2: ") && warnings[0].ends_with("; using the defaults"), "{warnings:?}");
    }

    #[test]
    fn settings_written_out_read_back() {
        let cfg =
            Config { gap: 3, assist: false, layouts: vec!["rows".into()], accent: [1, 2, 3], ..Config::default() };
        assert_eq!(parse(&cfg.to_string()), (cfg, Vec::new()));
    }

    #[test]
    fn files_are_loaded_or_reported() {
        let dir = std::env::temp_dir().join(format!("tileling-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, "gap = 2\nnope = 1\n").unwrap();
        let loaded = load_from(&file).unwrap();
        assert_eq!((loaded.config.gap, loaded.warnings.len(), loaded.path), (2, 1, Some(file.clone())));
        let err = load_from(&dir.join("missing.toml")).unwrap_err();
        assert!(err.to_string().starts_with("cannot read "), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
