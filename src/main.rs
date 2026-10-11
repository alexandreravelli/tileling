//! Tileling: Ubuntu 26.10-style window tiling for XFCE and other X11 desktops.

mod app;

use std::path::PathBuf;
use std::process::ExitCode;
use tileling::config;

const USAGE: &str = "\
Usage: tileling [--check-config [FILE] | --version | --help]

Drag a window towards the top of the screen: a layout picker appears. Drop the window on a zone
to tile it there, then pick the windows that fill the other zones.

  --check-config [FILE]  list the problems of the configuration file, then the settings in use
  --version              print the version
  --help                 print this help

Configuration: ~/.config/tileling/config.toml (else /etc/xdg/tileling/config.toml).
Set TILELING_DEBUG=1 to log what Tileling does.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!("tileling {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--check-config") => check_config(args.get(1).map(PathBuf::from)),
        Some(other) => {
            eprintln!("tileling: unknown option {other}\n\n{USAGE}");
            ExitCode::from(2)
        }
        None => match app::run(load()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tileling: {e:#}");
                ExitCode::FAILURE
            }
        },
    }
}

/// The configuration in use; its problems go to stderr.
fn load() -> config::Config {
    match config::load() {
        Ok(loaded) => {
            let file = loaded.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            for w in &loaded.warnings {
                eprintln!("tileling: {file}: {w}");
            }
            loaded.config
        }
        Err(e) => {
            eprintln!("tileling: {e}; using the defaults");
            config::Config::default()
        }
    }
}

/// `--check-config [FILE]`: the problems of the file, one per line (`file:line: message`, as
/// compilers write them), then the settings in use. Fails when there is a problem.
fn check_config(file: Option<PathBuf>) -> ExitCode {
    let loaded = match file {
        Some(path) => config::load_from(&path),
        None => config::load(),
    };
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tileling: {e}");
            return ExitCode::FAILURE;
        }
    };
    let name = loaded.path.as_ref().map_or_else(|| "(no file: built-in defaults)".into(), |p| p.display().to_string());
    for w in &loaded.warnings {
        match w.line {
            Some(line) => println!("{name}:{line}: {}", w.message),
            None => println!("{name}: {}", w.message),
        }
    }
    match loaded.warnings.len() {
        0 => println!("{name}: OK"),
        1 => println!("1 problem: Tileling skips this setting and uses the others."),
        n => println!("{n} problems: Tileling skips these settings and uses the others."),
    }
    println!("\n# Settings in use\n{}", loaded.config);
    if loaded.warnings.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
