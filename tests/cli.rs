//! The `tileling` command line, for what works without an X server.

use std::process::{Command, Output};

fn tileling(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tileling"))
        .args(args)
        .env("DISPLAY", ":no-such-display")
        .env("XDG_CONFIG_HOME", "/nonexistent")
        .env("XDG_CONFIG_DIRS", "/nonexistent")
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn check_config_lists_the_problems_then_the_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "gap = 4\ntriger = 50\nlayouts = [\"halves\", \"rows\"]\n").unwrap();
    let out = tileling(&["--check-config", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let p = path.display();
    let text = stdout(&out);
    assert!(
        text.starts_with(&format!(
            "{p}:2: unknown setting \"triger\". Did you mean \"trigger\"?\n\
             1 problem: Tileling skips this setting and uses the others.\n\n# Settings in use\ngap = 4\n"
        )),
        "{text}"
    );
    assert!(text.contains("layouts = [\"halves\", \"rows\"]\n"), "{text}");

    std::fs::write(&path, include_str!("../config/default.toml")).unwrap();
    let out = tileling(&["--check-config", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with(&format!("{p}: OK\n")));

    // Without a file anywhere, the built-in defaults are in use.
    let out = tileling(&["--check-config"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("(no file: built-in defaults): OK\n"));

    let out = tileling(&["--check-config", "/no/such/file.toml"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("tileling: cannot read /no/such/file.toml"));
}

#[test]
fn version_help_and_mistakes() {
    assert_eq!(stdout(&tileling(&["--version"])), format!("tileling {}\n", env!("CARGO_PKG_VERSION")));
    assert!(stdout(&tileling(&["--help"])).starts_with("Usage: tileling"));
    let out = tileling(&["--frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    let out = tileling(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("tileling: cannot connect to the X server"));
}
