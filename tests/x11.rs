//! Tileling on a real X server: Xvfb, started for the test. Skipped where Xvfb is not installed
//! (`apt install xvfb`).

use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// An Xvfb server, stopped when dropped.
struct Server {
    display: String,
    child: Child,
}

impl Server {
    fn start() -> Option<Server> {
        // A display number of its own, so tests running at the same time do not meet.
        let number = 100 + std::process::id() % 800;
        let display = format!(":{number}");
        let child = Command::new("Xvfb")
            .args([display.as_str(), "-screen", "0", "1280x800x24", "-nolisten", "tcp"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let socket = format!("/tmp/.X11-unix/X{number}");
        let ready = wait(|| Path::new(&socket).exists());
        let server = Server { display, child };
        ready.then_some(server)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Waits up to five seconds for `condition`.
fn wait(mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if condition() {
            return true;
        }
        sleep(Duration::from_millis(50));
    }
    false
}

fn tileling(server: &Server, config: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tileling"));
    cmd.env("DISPLAY", &server.display).env("XDG_CONFIG_HOME", config).env("XDG_CONFIG_DIRS", config);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd
}

fn run(server: &Server, config: &Path, arg: &str) -> Output {
    tileling(server, config).arg(arg).output().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn tileling_starts_reloads_is_replaced_and_quits() {
    let Some(server) = Server::start() else {
        eprintln!("skipped: Xvfb is not installed");
        return;
    };
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("tileling")).unwrap();
    std::fs::write(config.path().join("tileling/config.toml"), "gap = 4\n").unwrap();

    let mut first = tileling(&server, config.path()).spawn().unwrap();
    assert!(wait(|| run(&server, config.path(), "--reload").status.success()), "Tileling did not start");

    let second = tileling(&server, config.path()).output().unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(stderr(&second).contains("already running (use --replace"), "{}", stderr(&second));

    // A reload reads the file again and reports its new problems on stderr.
    std::fs::write(config.path().join("tileling/config.toml"), "gap = 4\ngapp = 2\n").unwrap();
    assert!(run(&server, config.path(), "--reload").status.success());

    // --replace takes over: the first one stops cleanly.
    let mut replacement = tileling(&server, config.path()).arg("--replace").spawn().unwrap();
    assert!(wait(|| first.try_wait().is_ok_and(|s| s.is_some())), "the first Tileling did not stop");
    let first = first.wait_with_output().unwrap();
    assert!(first.status.success());
    assert!(stderr(&first).contains("unknown setting \"gapp\""), "{}", stderr(&first));

    assert!(wait(|| run(&server, config.path(), "--reload").status.success()));
    assert!(run(&server, config.path(), "--quit").status.success());
    assert!(wait(|| replacement.try_wait().is_ok_and(|s| s.is_some())), "--quit did not stop it");
    assert!(replacement.wait().unwrap().success());
    let after = run(&server, config.path(), "--quit");
    assert!(stderr(&after).contains("Tileling is not running"), "{}", stderr(&after));
}
