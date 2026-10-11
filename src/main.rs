//! Tileling: Ubuntu 26.10-style window tiling for XFCE and other X11 desktops.

mod app;

use tileling::config;

const USAGE: &str = "\
Usage: tileling [--check-config | --version | --help]

Drag a window towards the top of the screen: a layout picker appears. Drop the window on a zone
to tile it there, then pick the windows that fill the other zones.

  --check-config   print the configuration in use and any problem in it, then exit
  --version        print the version
  --help           print this help

Configuration: ~/.config/tileling/config.toml (else /etc/xdg/tileling/config.toml).
Set TILELING_DEBUG=1 to log what Tileling does.";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--help" | "-h") => println!("{USAGE}"),
        Some("--version" | "-V") => println!("tileling {}", env!("CARGO_PKG_VERSION")),
        Some("--check-config") => {
            let path = config::path();
            println!("file: {}", path.as_ref().map_or("(none, built-in defaults)".into(), |p| p.display().to_string()));
            println!("{:#?}", load());
        }
        Some(other) => {
            eprintln!("tileling: unknown option {other}\n\n{USAGE}");
            std::process::exit(2);
        }
        None => {
            if let Err(e) = app::run(load()) {
                eprintln!("tileling: {e}");
                std::process::exit(1);
            }
        }
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
