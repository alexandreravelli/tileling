//! Tileling: Ubuntu 26.10-style window tiling for XFCE and other X11 desktops.

mod app;
mod config;
mod layout;

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
            println!("{:#?}", config::load());
        }
        Some(other) => {
            eprintln!("tileling: unknown option {other}\n\n{USAGE}");
            std::process::exit(2);
        }
        None => {
            if let Err(e) = app::run(config::load()) {
                eprintln!("tileling: {e}");
                std::process::exit(1);
            }
        }
    }
}
