//! Test driver (not shipped): fakes real pointer input with `XTest`, like a hand on the mouse.
//!   xtest move X Y | down | up | click X Y | key KEYCODE... | path X1 Y1 X2 Y2 STEPS `DELAY_MS`

use std::{thread::sleep, time::Duration};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| -> i32 { args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0) };
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let motion = |x: i32, y: i32| conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x as i16, y as i16, 0);
    let button = |ty: u8| conn.xtest_fake_input(ty, 1, 0, root, 0, 0, 0);
    match args.first().map(String::as_str) {
        Some("move") => motion(n(1), n(2))?.check()?,
        Some("down") => button(BUTTON_PRESS_EVENT)?.check()?,
        Some("up") => button(BUTTON_RELEASE_EVENT)?.check()?,
        Some("click") => {
            motion(n(1), n(2))?;
            conn.flush()?;
            sleep(Duration::from_millis(80));
            button(BUTTON_PRESS_EVENT)?;
            conn.flush()?;
            sleep(Duration::from_millis(60));
            button(BUTTON_RELEASE_EVENT)?;
        }
        Some("key") => {
            // Several keycodes make a shortcut: pressed in order, released in reverse.
            let codes: Vec<u8> = args[1..].iter().filter_map(|c| c.parse().ok()).collect();
            for &c in &codes {
                conn.xtest_fake_input(KEY_PRESS_EVENT, c, 0, root, 0, 0, 0)?.check()?;
            }
            for &c in codes.iter().rev() {
                conn.xtest_fake_input(KEY_RELEASE_EVENT, c, 0, root, 0, 0, 0)?.check()?;
            }
        }
        Some("path") => {
            let (x1, y1, x2, y2, steps, delay) = (n(1), n(2), n(3), n(4), n(5).max(1), n(6) as u64);
            for i in 1..=steps {
                motion(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps)?;
                conn.flush()?;
                sleep(Duration::from_millis(delay));
            }
        }
        _ => {
            return Err(
                "usage: xtest move X Y | down | up | click X Y | key KEYCODE... | path X1 Y1 X2 Y2 STEPS DELAY_MS"
                    .into(),
            );
        }
    }
    conn.flush()?;
    Ok(())
}
