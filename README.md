# Tileling

Ubuntu 26.10's window tiling for XFCE and other X11 desktops, in one ~1 MB static binary.

- **Layout picker**: drag a window towards the top of the screen and a row of layouts slides
  in: halves, half and quarters, quarters, two thirds and one third.
- **Preview**: hover a zone and a translucent preview shows where the window will go.
- **Tiling assistant**: drop the window and Tileling offers your other windows, by icon, for
  the zones that are left. Click one to tile it there; Escape or a click elsewhere closes it.
- **Gaps** between windows, an accent colour, and the layouts you want, in one TOML file.

xfwm4 already tiles halves and quarters when you drag a window to a screen edge; Tileling adds
the picker, the preview and the assistant on top of it.

## How it works

Tileling sleeps until a mouse button goes down. It follows a drag with XInput 2 raw events
(they keep arriving while the window manager holds the pointer), so it needs no window manager
plugin. It tiles with the standard `_NET_MOVERESIZE_WINDOW` request, which takes the window
frame and panels into account. About 1 MB of memory, no CPU while idle, one thread, no GTK.

It needs an X11 session with XInput 2.1 or newer. With a compositor (xfwm4's is on by default)
the picker and the preview are translucent with rounded corners; without one they are solid.

## Install

Build a static binary that runs on any distribution (Debian 13 included):

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
sudo install -m 755 target/x86_64-unknown-linux-musl/release/tileling /usr/local/bin/
sudo install -m 644 data/tileling.desktop /etc/xdg/autostart/
```

Log out and back in, or run `tileling &`. A Debian package: `cargo deb` (needs `cargo-deb`).

## Configuration

`~/.config/tileling/config.toml`, else `/etc/xdg/tileling/config.toml` for every user; see
[config/default.toml](config/default.toml). Unknown keys and bad values are reported and skipped
one by one, so a typo never resets the rest of the file. `tileling --check-config` shows the
configuration in use.

```toml
gap = 8           # pixels between windows and around them
trigger = 40      # distance from the top that opens the picker
assist = true     # offer the other windows after tiling
layouts = ["halves", "half-quarters", "quarters-half", "quarters", "two-thirds", "one-third"]
accent = "#3584e4"
```

To turn Tileling off, untick it in *Settings > Session and Startup > Application Autostart*.

## Development

`cargo test` covers the zone geometry, the picker, the configuration and icon decoding.
`examples/xtest.rs` drives the real pointer through XTest (move, press, drag, release) to test
the whole flow on a live session: `TILELING_DEBUG=1 tileling` logs each tiling.
