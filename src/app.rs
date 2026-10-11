//! The X11 side. Tileling sleeps until the left mouse button goes down. While a window is dragged
//! it follows the pointer through XInput 2 raw events (delivered even while the window manager
//! holds the pointer grab), shows the layout picker near the top of the screen and a preview of
//! the hovered zone, then tiles the window with _NET_MOVERESIZE_WINDOW and offers the other
//! windows for the zones that are left.

use std::borrow::Cow;
use std::error::Error;
use std::time::Duration;
use tileling::config::Config;
use tileling::layout::{self, Change, Layout, Picker, Rect, Strut, Zone};
use tiny_skia::{Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::{ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::image::{BitsPerPixel, Image, ScanlinePad};
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _, XIEventMask};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

pub type Res<T> = Result<T, Box<dyn Error>>;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_CLIENT_LIST,
        _NET_CLIENT_LIST_STACKING,
        _NET_ACTIVE_WINDOW,
        _NET_WORKAREA,
        _NET_CURRENT_DESKTOP,
        _NET_WM_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_MAXIMIZED_VERT,
        _NET_WM_STATE_MAXIMIZED_HORZ,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
        _NET_FRAME_EXTENTS,
        _NET_MOVERESIZE_WINDOW,
        _NET_WM_ICON,
    }
}

/// Icon size and cell size in the assist popups.
const ICON: u32 = 48;
const CELL: u32 = 80;
/// At most this many windows are offered in each assist popup.
const MAX_CANDIDATES: usize = 12;
/// Ignore raw motion events closer together than this (milliseconds).
const MOTION_MS: u32 = 16;
const XK_ESCAPE: u32 = 0xff1b;

/// Escape alone, with Caps Lock (Lock), Num Lock (Mod2) or both.
fn escape_modifiers() -> [ModMask; 4] {
    [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2]
}

fn debug() -> bool {
    std::env::var_os("TILELING_DEBUG").is_some()
}

/// Losing the X connection is the only error that stops the daemon; anything else (a window
/// that went away, a failed drawing) is logged and the next event is handled as usual.
fn fatal(e: &(dyn Error + 'static)) -> bool {
    e.is::<ConnectionError>()
        || matches!(e.downcast_ref::<ReplyError>(), Some(ReplyError::ConnectionError(_)))
        || matches!(e.downcast_ref::<ReplyOrIdError>(), Some(ReplyOrIdError::ConnectionError(_)))
}

/// The visual the overlays are drawn with: 32-bit ARGB while a compositor runs, else the root's.
#[derive(Clone, Copy, PartialEq)]
struct Visual {
    depth: u8,
    id: Visualid,
    colormap: Colormap,
    argb: bool,
}

/// The layout picker on screen, with the work area it belongs to.
struct Shown {
    picker: Picker,
    area: Rect,
    hovered: Option<(usize, usize)>,
}

/// A left-button press on a window, which becomes a drag once the window moves.
struct Drag {
    client: Window,
    frame: Window,
    origin: Rect,
    was_maximized: bool,
    moved: bool,
    area: Option<Rect>,
    last: u32,
    shown: Option<Shown>,
}

struct AssistZone {
    win: Window,
    /// The window's background pixmap: hover changes repaint single cells into it.
    background: u32,
    rect: Rect,
    zone: Zone,
    cells: Vec<Rect>,
    hovered: Option<usize>,
}

struct Assist {
    area: Rect,
    zones: Vec<AssistZone>,
    candidates: Vec<(Window, Option<Pixmap>)>,
}

enum Mode {
    Idle,
    Drag(Drag),
    Assist(Assist),
}

pub struct App {
    conn: RustConnection,
    root: Window,
    selection: Atom,
    compositor: Atom,
    atoms: Atoms,
    root_visual: Visual,
    argb_visual: Option<Visual>,
    visual: Visual,
    shape: bool,
    gc: Gcontext,
    escape: Option<Keycode>,
    cfg: Config,
    layouts: Vec<&'static Layout>,
    picker_win: Window,
    ghost_win: Window,
    ghost: Option<Rect>,
    mode: Mode,
}

fn intern(conn: &RustConnection, name: &str) -> Res<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn argb_visual(screen: &Screen) -> Option<Visualid> {
    screen
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
}

fn keycode_of(conn: &RustConnection, keysym: u32) -> Option<Keycode> {
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
    let per = (map.keysyms_per_keycode as usize).max(1);
    map.keysyms.chunks(per).position(|syms| syms.contains(&keysym)).map(|i| min + i as u8)
}

pub fn run(cfg: Config) -> Res<()> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num].clone();
    let root = screen.root;

    // One instance per screen: whoever owns this selection.
    let selection = intern(&conn, &format!("_TILELING_S{screen_num}"))?;
    if conn.get_selection_owner(selection)?.reply()?.owner != NONE {
        return Err("Tileling is already running".into());
    }
    let owner = conn.generate_id()?;
    conn.create_window(0, owner, root, -1, -1, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?;
    conn.set_selection_owner(owner, selection, CURRENT_TIME)?;
    if conn.get_selection_owner(selection)?.reply()?.owner != owner {
        return Err("Tileling is already running".into());
    }

    if conn.extension_information(xinput::X11_EXTENSION_NAME)?.is_none() {
        return Err("the X server has no XInput extension".into());
    }
    // Raw events reach every client regardless of grabs from XInput 2.1 on.
    let v = conn.xinput_xi_query_version(2, 2)?.reply()?;
    if (v.major_version, v.minor_version) < (2, 1) {
        return Err(format!("XInput 2.1 or newer is needed (found {}.{})", v.major_version, v.minor_version).into());
    }
    let atoms = Atoms::new(&conn)?.reply()?;
    let shape = conn.extension_information(shape::X11_EXTENSION_NAME)?.is_some();
    let compositor = intern(&conn, &format!("_NET_WM_CM_S{screen_num}"))?;
    let root_visual =
        Visual { depth: screen.root_depth, id: screen.root_visual, colormap: screen.default_colormap, argb: false };
    let argb_visual = match argb_visual(&screen) {
        Some(id) => {
            let colormap = conn.generate_id()?;
            conn.create_colormap(ColormapAlloc::NONE, colormap, root, id)?;
            Some(Visual { depth: 32, id, colormap, argb: true })
        }
        None => None,
    };
    let escape = keycode_of(&conn, XK_ESCAPE);

    let mut app = App {
        conn,
        root,
        selection,
        compositor,
        atoms,
        root_visual,
        argb_visual,
        visual: root_visual,
        shape,
        gc: NONE,
        escape,
        layouts: cfg.layouts.iter().filter_map(|n| layout::layout(n)).collect(),
        cfg,
        picker_win: NONE,
        ghost_win: NONE,
        ghost: None,
        mode: Mode::Idle,
    };
    app.ensure_overlays()?;
    app.select_raw(false)?;
    app.conn.flush()?;
    if debug() {
        eprintln!("tileling: running (argb: {}, shape: {shape}, layouts: {:?})", app.visual.argb, app.cfg.layouts);
    }

    loop {
        let event = app.conn.wait_for_event()?;
        if let Err(e) = app.handle(event) {
            if fatal(e.as_ref()) {
                return Err(e);
            }
            eprintln!("tileling: {e}");
        }
        app.conn.flush()?;
    }
}

impl App {
    fn handle(&mut self, event: Event) -> Res<()> {
        match event {
            Event::XinputRawButtonPress(e) if e.detail == 1 => self.on_press(),
            Event::XinputRawButtonRelease(e) if e.detail == 1 => self.on_release(),
            Event::XinputRawMotion(e) => self.on_motion(e.time),
            Event::ButtonPress(e) if e.detail == 1 => self.assist_click(e.event, e.event_x, e.event_y, e.time),
            Event::MotionNotify(e) => self.assist_hover(e.event, Some((e.event_x, e.event_y))),
            Event::LeaveNotify(e) => self.assist_hover(e.event, None),
            // Only Escape is grabbed, and only while the assist popups are open.
            Event::KeyPress(_) => self.close_assist(),
            Event::SelectionClear(e) if e.selection == self.selection => std::process::exit(0),
            // Requests on windows that went away in the meantime: nothing to do.
            Event::Error(e) if debug() => {
                eprintln!("tileling: X error {:?}", e.error_kind);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // ---- X helpers -----------------------------------------------------------------------------

    fn select_raw(&self, motion: bool) -> Res<()> {
        let mut mask = XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_BUTTON_RELEASE;
        if motion {
            mask |= XIEventMask::RAW_MOTION;
        }
        // Device 1: all master devices.
        self.conn.xinput_xi_select_events(self.root, &[xinput::EventMask { deviceid: 1, mask: vec![mask] }])?;
        Ok(())
    }

    fn prop32(&self, w: Window, prop: impl Into<Atom>, ty: impl Into<Atom>) -> Vec<u32> {
        self.conn
            .get_property(false, w, prop, ty, 0, u32::MAX / 4)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    /// The same property of many windows, in one round trip.
    fn prop32_all(&self, windows: &[Window], prop: Atom, ty: impl Into<Atom> + Copy) -> Vec<Vec<u32>> {
        let cookies: Vec<_> =
            windows.iter().map(|&w| self.conn.get_property(false, w, prop, ty, 0, u32::MAX / 4)).collect();
        cookies
            .into_iter()
            .map(|c| {
                c.ok().and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
            })
            .collect()
    }

    fn pointer(&self) -> Option<QueryPointerReply> {
        self.conn.query_pointer(self.root).ok()?.reply().ok()
    }

    fn geometry(&self, w: Window) -> Option<Rect> {
        let g = self.conn.get_geometry(w).ok()?.reply().ok()?;
        Some(Rect { x: g.x as i32, y: g.y as i32, w: g.width as u32, h: g.height as u32 })
    }

    fn clients(&self) -> Vec<Window> {
        self.prop32(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW)
    }

    /// The child of the root window that contains `w` (its frame, for a reparenting WM).
    fn top_level(&self, mut w: Window) -> Option<Window> {
        for _ in 0..16 {
            let tree = self.conn.query_tree(w).ok()?.reply().ok()?;
            if tree.parent == self.root || tree.parent == NONE {
                return Some(w);
            }
            w = tree.parent;
        }
        None
    }

    /// The client window inside a top-level `frame`: the frame itself, one of its children (the
    /// usual case with xfwm4, one round trip), or, failing that, a deeper descendant.
    fn client_of(&self, frame: Window) -> Option<Window> {
        let clients = self.clients();
        if clients.contains(&frame) {
            return Some(frame);
        }
        let children = self.conn.query_tree(frame).ok()?.reply().ok()?.children;
        if let Some(&c) = children.iter().find(|c| clients.contains(c)) {
            return Some(c);
        }
        clients.into_iter().find(|&c| self.top_level(c) == Some(frame))
    }

    /// Normal, resizable windows only: not docks, panels, dialogs, the desktop, fullscreen
    /// windows or windows whose minimum and maximum sizes are the same.
    fn tileable(&self, w: Window) -> bool {
        let types = self.prop32(w, self.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM);
        let state = self.prop32(w, self.atoms._NET_WM_STATE, AtomEnum::ATOM);
        let hints = self.prop32(w, AtomEnum::WM_NORMAL_HINTS, AtomEnum::WM_SIZE_HINTS);
        // WM_SIZE_HINTS: flags, 4 obsolete fields, min width/height, max width/height (PMinSize = 16, PMaxSize = 32).
        let fixed = hints.len() >= 9 && hints[0] & 48 == 48 && hints[5] == hints[7] && hints[6] == hints[8];
        (types.is_empty() || types.contains(&self.atoms._NET_WM_WINDOW_TYPE_NORMAL))
            && !state.contains(&self.atoms._NET_WM_STATE_FULLSCREEN)
            && !state.contains(&self.atoms._NET_WM_STATE_SKIP_TASKBAR)
            && !fixed
    }

    fn current_desktop(&self) -> u32 {
        self.prop32(self.root, self.atoms._NET_CURRENT_DESKTOP, AtomEnum::CARDINAL).first().copied().unwrap_or(0)
    }

    /// The work area of the monitor under (x, y): the monitor minus the docks and panels that
    /// reserve space on it. Falls back to _NET_WORKAREA when no window declares struts.
    fn work_area(&self, x: i32, y: i32) -> Rect {
        let screen = self.geometry(self.root).unwrap_or(Rect { x: 0, y: 0, w: 1024, h: 768 });
        let monitors: Vec<Rect> = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.monitors
                    .iter()
                    .map(|m| Rect { x: m.x as i32, y: m.y as i32, w: m.width as u32, h: m.height as u32 })
                    .collect()
            })
            .unwrap_or_default();
        let monitor = monitors.into_iter().find(|m| m.contains(x, y)).unwrap_or(screen);

        let clients = self.clients();
        let partial = self.prop32_all(&clients, self.atoms._NET_WM_STRUT_PARTIAL, AtomEnum::CARDINAL);
        let plain = self.prop32_all(&clients, self.atoms._NET_WM_STRUT, AtomEnum::CARDINAL);
        let struts: Vec<Strut> = partial
            .iter()
            .zip(&plain)
            .filter_map(|(p, s)| layout::strut(p, screen).or_else(|| layout::strut(s, screen)))
            .collect();
        if !struts.is_empty() {
            return layout::work_area(monitor, screen, &struts);
        }
        let desktop = self.current_desktop() as usize;
        let workarea = self.prop32(self.root, self.atoms._NET_WORKAREA, AtomEnum::CARDINAL);
        workarea
            .as_chunks::<4>()
            .0
            .get(desktop)
            .map(|c| Rect { x: c[0] as i32, y: c[1] as i32, w: c[2], h: c[3] })
            .and_then(|wa| monitor.intersect(&wa))
            .unwrap_or(monitor)
    }

    fn send_root_message(&self, w: Window, ty: Atom, data: [u32; 5]) -> Res<()> {
        let event = ClientMessageEvent::new(32, w, ty, data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        self.conn.send_event(false, self.root, mask, event)?;
        Ok(())
    }

    /// (Re)creates the picker and preview windows when the compositor appeared or went away
    /// since they were made: translucent with one, solid without.
    fn ensure_overlays(&mut self) -> Res<()> {
        let compositing = self.conn.get_selection_owner(self.compositor)?.reply()?.owner != NONE;
        let wanted = self.argb_visual.filter(|_| compositing).unwrap_or(self.root_visual);
        if wanted == self.visual && self.picker_win != NONE {
            return Ok(());
        }
        for w in [self.picker_win, self.ghost_win] {
            if w != NONE {
                self.conn.destroy_window(w)?;
            }
        }
        if self.gc != NONE {
            self.conn.free_gc(self.gc)?;
        }
        self.visual = wanted;
        self.ghost = None;
        self.picker_win = self.overlay(EventMask::NO_EVENT)?;
        self.ghost_win = self.overlay(EventMask::NO_EVENT)?;
        self.gc = self.conn.generate_id()?;
        self.conn.create_gc(self.gc, self.picker_win, &CreateGCAux::new())?;
        // Neither takes input: they show up during a drag the window manager is running.
        if self.shape {
            for w in [self.picker_win, self.ghost_win] {
                self.conn.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, w, 0, 0, &[])?;
            }
        }
        if debug() {
            eprintln!("tileling: overlays drawn with {} visual", if wanted.argb { "an ARGB" } else { "the root" });
        }
        Ok(())
    }

    /// An override-redirect window drawn with our visual (hidden until mapped).
    fn overlay(&self, events: EventMask) -> Res<Window> {
        let w = self.conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(self.visual.colormap)
            .override_redirect(1)
            .event_mask(events);
        let v = self.visual;
        self.conn.create_window(v.depth, w, self.root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, v.id, &aux)?;
        Ok(w)
    }

    fn place(&self, w: Window, r: Rect) -> Res<()> {
        let aux = ConfigureWindowAux::new().x(r.x).y(r.y).width(r.w).height(r.h).stack_mode(StackMode::ABOVE);
        self.conn.configure_window(w, &aux)?;
        Ok(())
    }

    /// Draws a tiny-skia pixmap into an X drawable at (x, y).
    fn put(&self, drawable: Drawable, pm: &Pixmap, x: i16, y: i16) -> Res<()> {
        let mut data = pm.data().to_vec();
        for px in data.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // RGBA -> BGRA (little-endian ARGB32)
        }
        let (w, h) = (pm.width() as u16, pm.height() as u16);
        let order = x11rb::image::ImageOrder::LsbFirst;
        let image =
            Image::new(w, h, ScanlinePad::Pad32, self.visual.depth, BitsPerPixel::B32, order, Cow::Owned(data))?;
        image.put(&self.conn, drawable, self.gc, x, y)?;
        Ok(())
    }

    /// A server pixmap holding `pm`, for use as a window background.
    fn server_pixmap(&self, win: Window, pm: &Pixmap) -> Res<u32> {
        let xpm = self.conn.generate_id()?;
        self.conn.create_pixmap(self.visual.depth, xpm, win, pm.width() as u16, pm.height() as u16)?;
        self.put(xpm, pm, 0, 0)?;
        Ok(xpm)
    }

    /// Shows `pm` as the background of `win`.
    fn upload(&self, win: Window, pm: &Pixmap) -> Res<()> {
        let xpm = self.server_pixmap(win, pm)?;
        self.conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().background_pixmap(xpm))?;
        self.conn.clear_area(false, win, 0, 0, 0, 0)?;
        self.conn.free_pixmap(xpm)?;
        Ok(())
    }

    // ---- dragging ------------------------------------------------------------------------------

    fn on_press(&mut self) -> Res<()> {
        let Some(ptr) = self.pointer() else { return Ok(()) };
        match &self.mode {
            Mode::Drag(_) => return Ok(()),
            // Clicks on the assist popups arrive as ButtonPress; anywhere else closes them.
            Mode::Assist(a) if a.zones.iter().any(|z| z.win == ptr.child) => return Ok(()),
            Mode::Assist(_) => self.close_assist()?,
            Mode::Idle => {}
        }
        if ptr.child == NONE {
            return Ok(());
        }
        let Some(client) = self.client_of(ptr.child).filter(|&c| self.tileable(c)) else { return Ok(()) };
        let Some(origin) = self.geometry(ptr.child) else { return Ok(()) };
        let state = self.prop32(client, self.atoms._NET_WM_STATE, AtomEnum::ATOM);
        let a = &self.atoms;
        let was_maximized =
            state.iter().any(|&s| s == a._NET_WM_STATE_MAXIMIZED_VERT || s == a._NET_WM_STATE_MAXIMIZED_HORZ);
        self.ensure_overlays()?;
        self.mode = Mode::Drag(Drag {
            client,
            frame: ptr.child,
            origin,
            was_maximized,
            moved: false,
            area: None,
            last: 0,
            shown: None,
        });
        // Motion is only followed while the button is down, so Tileling sleeps otherwise.
        self.select_raw(true)
    }

    fn on_motion(&mut self, time: u32) -> Res<()> {
        let Mode::Drag(d) = &mut self.mode else { return Ok(()) };
        if time.wrapping_sub(d.last) < MOTION_MS {
            return Ok(());
        }
        d.last = time;
        let (frame, origin, was_maximized, moved) = (d.frame, d.origin, d.was_maximized, d.moved);

        let Some(now) = self.geometry(frame) else { return self.end_drag() }; // closed mid-drag
        if !moved {
            match layout::change(origin, now, was_maximized) {
                Change::None => return Ok(()), // a click, or a drag inside the window
                Change::Resize => return self.end_drag(),
                Change::Move => {}
            }
        } else if now == origin {
            // Back where it started while the button is still down: the window manager
            // cancelled the move (Escape).
            return self.end_drag();
        }

        let Some(ptr) = self.pointer() else { return Ok(()) };
        let (px, py) = (ptr.root_x as i32, ptr.root_y as i32);
        let Mode::Drag(d) = &mut self.mode else { return Ok(()) };
        d.moved = true;
        let area = match d.area {
            Some(a) if a.contains(px, py) || d.shown.is_some() => a,
            _ => {
                let a = self.work_area(px, py);
                if let Mode::Drag(d) = &mut self.mode {
                    d.area = Some(a);
                }
                a
            }
        };
        let Mode::Drag(d) = &self.mode else { return Ok(()) };
        let inside_x = px >= area.x && px < area.right();
        match &d.shown {
            None if inside_x && py < area.y + self.cfg.trigger as i32 => {
                self.show_picker(area)?;
                self.update_hover(px, py)
            }
            None => Ok(()),
            Some(s) if py > layout::picker_close_y(&s.picker, s.area, self.cfg.trigger) || !inside_x => {
                self.hide_picker()
            }
            Some(_) => self.update_hover(px, py),
        }
    }

    fn end_drag(&mut self) -> Res<()> {
        if matches!(self.mode, Mode::Drag(_)) {
            self.mode = Mode::Idle;
            self.select_raw(false)?;
        }
        self.unmap_overlays()
    }

    fn on_release(&mut self) -> Res<()> {
        let Mode::Drag(drag) = std::mem::replace(&mut self.mode, Mode::Idle) else { return Ok(()) };
        self.select_raw(false)?;
        self.unmap_overlays()?;
        let (Some(shown), true) = (drag.shown, drag.moved) else { return Ok(()) };
        // Motion events are throttled: hit-test the pointer where it really is.
        let Some(ptr) = self.pointer() else { return Ok(()) };
        let (x, y) = (ptr.root_x as i32 - shown.picker.rect.x, ptr.root_y as i32 - shown.picker.rect.y);
        let Some((li, zi)) = layout::hit(&shown.picker, &self.layouts, x, y) else { return Ok(()) };

        self.wait_for_window_manager()?;
        // A move cancelled with Escape puts the window back where it was: nothing to tile.
        if self.geometry(drag.frame) == Some(drag.origin) {
            return Ok(());
        }
        let layout = self.layouts[li];
        self.tile(drag.client, shown.area, layout.zones[zi])?;
        if self.cfg.assist {
            let free = layout.zones.iter().enumerate().filter(|&(i, _)| i != zi).map(|(_, &z)| z).collect();
            self.open_assist(drag.client, shown.area, free)?;
        }
        Ok(())
    }

    /// Waits (up to half a second) until the window manager has released the pointer, that is,
    /// has finished the move: a geometry request sent during the move would be overridden.
    fn wait_for_window_manager(&self) -> Res<()> {
        for _ in 0..50 {
            let none = EventMask::NO_EVENT;
            let reply = self
                .conn
                .grab_pointer(false, self.root, none, GrabMode::ASYNC, GrabMode::ASYNC, NONE, NONE, CURRENT_TIME)?
                .reply()?;
            if reply.status == GrabStatus::SUCCESS {
                self.conn.ungrab_pointer(CURRENT_TIME)?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Moves and resizes `client` so that its frame fills `zone` of `area`, then checks the
    /// result and asks once more if the window manager placed it elsewhere.
    fn tile(&self, client: Window, area: Rect, zone: Zone) -> Res<()> {
        let target = layout::zone_rect(area, zone, self.cfg.gap);
        let a = &self.atoms;
        // Maximized (or edge-tiled) windows ignore geometry requests.
        self.send_root_message(
            client,
            a._NET_WM_STATE,
            [0, a._NET_WM_STATE_MAXIMIZED_VERT, a._NET_WM_STATE_MAXIMIZED_HORZ, 2, 0],
        )?;
        let ext = self.prop32(client, a._NET_FRAME_EXTENTS, AtomEnum::CARDINAL);
        let extents = ext.get(..4).and_then(|e| e.try_into().ok()).unwrap_or([0; 4]);
        let data = layout::moveresize_data(target, extents);
        self.send_root_message(client, a._NET_MOVERESIZE_WINDOW, data)?;
        self.conn.flush()?;
        std::thread::sleep(Duration::from_millis(60));
        let placed = self.top_level(client).and_then(|f| self.geometry(f));
        if placed.is_some_and(|p| (p.x, p.y) != (target.x, target.y)) {
            self.send_root_message(client, a._NET_MOVERESIZE_WINDOW, data)?;
        }
        if debug() {
            eprintln!("tileling: tile 0x{client:x} into {target:?} (frame {extents:?}, placed at {placed:?})");
        }
        Ok(())
    }

    // ---- picker and preview --------------------------------------------------------------------

    fn show_picker(&mut self, area: Rect) -> Res<()> {
        let picker = layout::picker(area, self.layouts.len());
        self.place(self.picker_win, picker.rect)?;
        if let Mode::Drag(d) = &mut self.mode {
            d.shown = Some(Shown { picker, area, hovered: None });
        }
        self.draw_picker()?;
        self.conn.map_window(self.picker_win)?;
        Ok(())
    }

    fn hide_picker(&mut self) -> Res<()> {
        if let Mode::Drag(d) = &mut self.mode {
            d.shown = None;
        }
        self.unmap_overlays()
    }

    fn unmap_overlays(&mut self) -> Res<()> {
        self.conn.unmap_window(self.picker_win)?;
        self.show_ghost(None)
    }

    fn shown(&self) -> Option<&Shown> {
        match &self.mode {
            Mode::Drag(d) => d.shown.as_ref(),
            _ => None,
        }
    }

    fn update_hover(&mut self, px: i32, py: i32) -> Res<()> {
        let Some(s) = self.shown() else { return Ok(()) };
        let hovered = layout::hit(&s.picker, &self.layouts, px - s.picker.rect.x, py - s.picker.rect.y);
        if hovered == s.hovered {
            return Ok(());
        }
        let ghost = hovered.map(|(li, zi)| layout::zone_rect(s.area, self.layouts[li].zones[zi], self.cfg.gap));
        if let Mode::Drag(Drag { shown: Some(s), .. }) = &mut self.mode {
            s.hovered = hovered;
        }
        self.show_ghost(ghost)?;
        self.draw_picker()?;
        // Keep the picker above the preview.
        self.conn.configure_window(self.picker_win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        Ok(())
    }

    fn draw_picker(&self) -> Res<()> {
        let Some(s) = self.shown() else { return Ok(()) };
        let r = s.picker.rect;
        let mut pm = Pixmap::new(r.w, r.h).ok_or("picker size")?;
        let radius = if self.visual.argb { 14.0 } else { 0.0 };
        fill_rrect(&mut pm, Rect { x: 0, y: 0, w: r.w, h: r.h }, radius, rgba(0x1d, 0x1d, 0x1d, 0xeb));
        let [ar, ag, ab] = self.cfg.accent;
        for (li, thumb) in s.picker.thumbs.iter().enumerate() {
            for (zi, &zone) in self.layouts[li].zones.iter().enumerate() {
                let color =
                    if s.hovered == Some((li, zi)) { rgba(ar, ag, ab, 0xff) } else { rgba(0xff, 0xff, 0xff, 0x30) };
                fill_rrect(&mut pm, layout::zone_rect(*thumb, zone, 4), 5.0, color);
            }
        }
        self.upload(self.picker_win, &pm)
    }

    fn show_ghost(&mut self, r: Option<Rect>) -> Res<()> {
        if r == self.ghost {
            return Ok(());
        }
        self.ghost = r;
        let Some(r) = r else {
            self.conn.unmap_window(self.ghost_win)?;
            return Ok(());
        };
        self.place(self.ghost_win, r)?;
        let [ar, ag, ab] = self.cfg.accent;
        if self.visual.argb {
            let mut pm = Pixmap::new(r.w, r.h).ok_or("preview size")?;
            let full = Rect { x: 0, y: 0, w: r.w, h: r.h };
            fill_rrect(&mut pm, full, 12.0, rgba(ar, ag, ab, 0x48));
            stroke_rrect(&mut pm, full, 12.0, 3.0, rgba(ar, ag, ab, 0xd8));
            self.upload(self.ghost_win, &pm)?;
        } else {
            // Without a compositor: a solid 4-pixel outline cut out with the Shape extension.
            let pixel = (ar as u32) << 16 | (ag as u32) << 8 | ab as u32;
            self.conn
                .change_window_attributes(self.ghost_win, &ChangeWindowAttributesAux::new().background_pixel(pixel))?;
            if self.shape {
                let (w, h) = (r.w as u16, r.h as u16);
                let edges = [
                    Rectangle { x: 0, y: 0, width: w, height: 4 },
                    Rectangle { x: 0, y: h as i16 - 4, width: w, height: 4 },
                    Rectangle { x: 0, y: 0, width: 4, height: h },
                    Rectangle { x: w as i16 - 4, y: 0, width: 4, height: h },
                ];
                self.conn.shape_rectangles(
                    shape::SO::SET,
                    shape::SK::BOUNDING,
                    ClipOrdering::UNSORTED,
                    self.ghost_win,
                    0,
                    0,
                    &edges,
                )?;
            }
            self.conn.clear_area(false, self.ghost_win, 0, 0, 0, 0)?;
        }
        self.conn.map_window(self.ghost_win)?;
        Ok(())
    }

    // ---- assist --------------------------------------------------------------------------------

    /// Other windows of the current desktop, most recently used first.
    fn candidates(&self, tiled: Window) -> Vec<Window> {
        let desktop = self.current_desktop();
        let stacking = self.prop32(self.root, self.atoms._NET_CLIENT_LIST_STACKING, AtomEnum::WINDOW);
        stacking
            .into_iter()
            .rev()
            .filter(|&w| w != tiled && self.tileable(w))
            .filter(|&w| {
                let d = self.prop32(w, self.atoms._NET_WM_DESKTOP, AtomEnum::CARDINAL);
                d.first().is_none_or(|&d| d == desktop || d == u32::MAX)
            })
            .take(MAX_CANDIDATES)
            .collect()
    }

    fn open_assist(&mut self, tiled: Window, area: Rect, free: Vec<Zone>) -> Res<()> {
        let windows = self.candidates(tiled);
        if free.is_empty() || windows.is_empty() {
            return Ok(());
        }
        let icons = self.prop32_all(&windows, self.atoms._NET_WM_ICON, AtomEnum::CARDINAL);
        let candidates: Vec<_> =
            windows.into_iter().zip(icons).map(|(w, data)| (w, from_net_wm_icon(&data, ICON))).collect();
        let mut zones = Vec::new();
        for zone in free {
            let rect = layout::zone_rect(area, zone, self.cfg.gap);
            let events = EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW;
            let win = self.overlay(events)?;
            self.place(win, rect)?;
            let cells = layout::grid(Rect { x: 0, y: 0, w: rect.w, h: rect.h }, candidates.len(), CELL);
            zones.push(AssistZone { win, background: NONE, rect, zone, cells, hovered: None });
        }
        self.mode = Mode::Assist(Assist { area, zones, candidates });
        self.redraw_assist()?;
        if let Mode::Assist(a) = &self.mode {
            for z in &a.zones {
                self.conn.map_window(z.win)?;
            }
        }
        // Escape closes the popups. Only that key is grabbed (shortcuts and screen lockers keep
        // working), with Caps Lock and Num Lock on or off: other modifiers belong to shortcuts
        // such as Ctrl+Escape, and grabbing those would fail.
        if let Some(code) = self.escape {
            let mut grabbed = false;
            for mods in escape_modifiers() {
                let grab = self.conn.grab_key(false, self.root, mods, code, GrabMode::ASYNC, GrabMode::ASYNC)?;
                grabbed |= grab.check().is_ok();
            }
            if !grabbed {
                eprintln!("tileling: cannot grab Escape; click elsewhere to close the assistant");
            }
        }
        Ok(())
    }

    fn close_assist(&mut self) -> Res<()> {
        let Mode::Assist(a) = std::mem::replace(&mut self.mode, Mode::Idle) else { return Ok(()) };
        for z in a.zones {
            self.free_zone(&z)?;
        }
        if let Some(code) = self.escape {
            for mods in escape_modifiers() {
                self.conn.ungrab_key(code, self.root, mods)?;
            }
        }
        Ok(())
    }

    fn free_zone(&self, z: &AssistZone) -> Res<()> {
        self.conn.destroy_window(z.win)?;
        if z.background != NONE {
            self.conn.free_pixmap(z.background)?;
        }
        Ok(())
    }

    /// The colour inside an assist popup, also painted under each redrawn cell.
    fn assist_fill(&self) -> Color {
        if self.visual.argb { rgba(0x10, 0x10, 0x10, 0xb0) } else { rgba(0x24, 0x24, 0x24, 0xff) }
    }

    /// One cell: background, hover highlight, then the window's icon (or a plain tile).
    fn draw_cell(&self, pm: &mut Pixmap, cell: Rect, icon: Option<&Pixmap>, hovered: bool) {
        if hovered {
            let inner = Rect { x: cell.x + 4, y: cell.y + 4, w: cell.w - 8, h: cell.h - 8 };
            fill_rrect(pm, inner, 10.0, rgba(0xff, 0xff, 0xff, 0x2c));
        }
        let (ix, iy) = (cell.x + (cell.w - ICON) as i32 / 2, cell.y + (cell.h - ICON) as i32 / 2);
        match icon {
            Some(icon) => pm.draw_pixmap(ix, iy, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None),
            None => {
                let [ar, ag, ab] = self.cfg.accent;
                fill_rrect(pm, Rect { x: ix, y: iy, w: ICON, h: ICON }, 10.0, rgba(ar, ag, ab, 0xff));
            }
        }
    }

    /// Repaints every assist popup in full (on opening, and after a window was placed).
    fn redraw_assist(&mut self) -> Res<()> {
        let Mode::Assist(a) = &self.mode else { return Ok(()) };
        let mut backgrounds = Vec::new();
        for z in &a.zones {
            let mut pm = Pixmap::new(z.rect.w, z.rect.h).ok_or("assist size")?;
            let full = Rect { x: 0, y: 0, w: z.rect.w, h: z.rect.h };
            if self.visual.argb {
                let [ar, ag, ab] = self.cfg.accent;
                fill_rrect(&mut pm, full, 12.0, self.assist_fill());
                stroke_rrect(&mut pm, full, 12.0, 2.0, rgba(ar, ag, ab, 0x90));
            } else {
                fill_rrect(&mut pm, full, 0.0, self.assist_fill());
            }
            for (ci, &cell) in z.cells.iter().enumerate() {
                self.draw_cell(&mut pm, cell, a.candidates[ci].1.as_ref(), z.hovered == Some(ci));
            }
            let xpm = self.server_pixmap(z.win, &pm)?;
            self.conn.change_window_attributes(z.win, &ChangeWindowAttributesAux::new().background_pixmap(xpm))?;
            self.conn.clear_area(false, z.win, 0, 0, 0, 0)?;
            if z.background != NONE {
                self.conn.free_pixmap(z.background)?;
            }
            backgrounds.push(xpm);
        }
        if let Mode::Assist(a) = &mut self.mode {
            for (z, xpm) in a.zones.iter_mut().zip(backgrounds) {
                z.background = xpm;
            }
        }
        Ok(())
    }

    /// Repaints a single cell into a popup's background pixmap.
    fn redraw_cell(&self, z: &AssistZone, ci: usize, icon: Option<&Pixmap>) -> Res<()> {
        let cell = z.cells[ci];
        let mut pm = Pixmap::new(cell.w, cell.h).ok_or("cell size")?;
        pm.fill(self.assist_fill());
        self.draw_cell(&mut pm, Rect { x: 0, y: 0, ..cell }, icon, z.hovered == Some(ci));
        self.put(z.background, &pm, cell.x as i16, cell.y as i16)?;
        self.conn.clear_area(false, z.win, cell.x as i16, cell.y as i16, cell.w as u16, cell.h as u16)?;
        Ok(())
    }

    fn assist_hover(&mut self, win: Window, at: Option<(i16, i16)>) -> Res<()> {
        let Mode::Assist(a) = &mut self.mode else { return Ok(()) };
        let Some(z) = a.zones.iter_mut().find(|z| z.win == win) else { return Ok(()) };
        let hovered = at.and_then(|(x, y)| z.cells.iter().position(|c| c.contains(x as i32, y as i32)));
        let before = std::mem::replace(&mut z.hovered, hovered);
        if before == hovered {
            return Ok(());
        }
        let Mode::Assist(a) = &self.mode else { return Ok(()) };
        let Some(z) = a.zones.iter().find(|z| z.win == win) else { return Ok(()) };
        for ci in [before, hovered].into_iter().flatten() {
            self.redraw_cell(z, ci, a.candidates[ci].1.as_ref())?;
        }
        Ok(())
    }

    fn assist_click(&mut self, win: Window, x: i16, y: i16, time: Timestamp) -> Res<()> {
        let Mode::Assist(a) = &mut self.mode else { return Ok(()) };
        let Some(i) = a.zones.iter().position(|z| z.win == win) else { return Ok(()) };
        let Some(ci) = a.zones[i].cells.iter().position(|c| c.contains(x as i32, y as i32)) else { return Ok(()) };
        let (client, _) = a.candidates.remove(ci);
        let zone = a.zones.remove(i);
        let area = a.area;
        let count = a.candidates.len();
        for z in &mut a.zones {
            z.cells = layout::grid(Rect { x: 0, y: 0, w: z.rect.w, h: z.rect.h }, count, CELL);
            z.hovered = None;
        }
        let done = a.zones.is_empty() || count == 0;
        self.free_zone(&zone)?;
        self.tile(client, area, zone.zone)?;
        self.send_root_message(client, self.atoms._NET_ACTIVE_WINDOW, [2, time, 0, 0, 0])?;
        if done { self.close_assist() } else { self.redraw_assist() }
    }
}

// ---- drawing ---------------------------------------------------------------------------------

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn paint(color: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color);
    p.anti_alias = true;
    p
}

fn rrect_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let rad = radius.min(w / 2.0).min(h / 2.0);
    if rad < 0.5 {
        return tiny_skia::Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    let k = rad * 0.447_715; // (1 - 0.552285) * rad: cubic approximation of a quarter circle
    let mut pb = PathBuilder::new();
    pb.move_to(x + rad, y);
    pb.line_to(x + w - rad, y);
    pb.cubic_to(x + w - k, y, x + w, y + k, x + w, y + rad);
    pb.line_to(x + w, y + h - rad);
    pb.cubic_to(x + w, y + h - k, x + w - k, y + h, x + w - rad, y + h);
    pb.line_to(x + rad, y + h);
    pb.cubic_to(x + k, y + h, x, y + h - k, x, y + h - rad);
    pb.line_to(x, y + rad);
    pb.cubic_to(x, y + k, x + k, y, x + rad, y);
    pb.close();
    pb.finish()
}

fn fill_rrect(pm: &mut Pixmap, r: Rect, radius: f32, color: Color) {
    if let Some(path) = rrect_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
        pm.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

/// An outline drawn inside `r` (the stroke is centred on a path inset by half its width).
fn stroke_rrect(pm: &mut Pixmap, r: Rect, radius: f32, width: f32, color: Color) {
    let half = width / 2.0;
    let (w, h) = (r.w as f32 - width, r.h as f32 - width);
    if let Some(path) = rrect_path(r.x as f32 + half, r.y as f32 + half, w, h, (radius - half).max(0.0)) {
        let stroke = Stroke { width, ..Stroke::default() };
        pm.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }
}

/// The best fitting image of _NET_WM_ICON (several ARGB images), scaled to `size`.
pub fn from_net_wm_icon(data: &[u32], size: u32) -> Option<Pixmap> {
    let mut best: Option<(u32, u32, &[u32])> = None;
    let mut rest = data;
    while rest.len() >= 2 {
        let (w, h) = (rest[0], rest[1]);
        let len = (w as usize).checked_mul(h as usize)?;
        if w == 0 || h == 0 || rest.len() < 2 + len {
            break;
        }
        let pixels = &rest[2..2 + len];
        let better = match best {
            None => true,
            // The smallest image that is at least `size`, else the largest.
            Some((bw, _, _)) => (w >= size && (bw < size || w < bw)) || (bw < size && w > bw),
        };
        if better {
            best = Some((w, h, pixels));
        }
        rest = &rest[2 + len..];
    }
    let (w, h, pixels) = best?;
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for &argb in pixels {
        let a = (argb >> 24) & 0xff;
        let premul = |c: u32| ((c & 0xff) * a / 255) as u8;
        rgba.extend_from_slice(&[premul(argb >> 16), premul(argb >> 8), premul(argb), a as u8]);
    }
    let image = Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(w, h)?)?;
    let scale = size as f32 / w.max(h) as f32;
    let mut out = Pixmap::new(size, size)?;
    let quality = if scale < 1.0 { FilterQuality::Bicubic } else { FilterQuality::Bilinear };
    let (dx, dy) = ((size as f32 - w as f32 * scale) / 2.0, (size as f32 - h as f32 * scale) / 2.0);
    let paint = PixmapPaint { quality, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, image.as_ref(), &paint, Transform::from_scale(scale, scale).post_translate(dx, dy), None);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, argb: u32) -> Vec<u32> {
        let mut v = vec![w, h];
        v.extend(std::iter::repeat_n(argb, (w * h) as usize));
        v
    }

    #[test]
    fn picks_the_closest_icon_and_scales_it() {
        let mut data = image(2, 2, 0xffff0000);
        data.extend(image(4, 4, 0xff0000ff));
        let icon = from_net_wm_icon(&data, 4).unwrap();
        assert_eq!((icon.width(), icon.height()), (4, 4));
        let px = icon.pixel(1, 1).unwrap();
        assert_eq!((px.red(), px.blue(), px.alpha()), (0, 255, 255));
    }

    #[test]
    fn non_square_icons_are_centred() {
        let icon = from_net_wm_icon(&image(8, 4, 0xff00ff00), 8).unwrap();
        assert_eq!(icon.pixel(4, 0).unwrap().alpha(), 0, "letterboxed above");
        assert_eq!(icon.pixel(4, 4).unwrap().green(), 255, "image in the middle");
    }

    #[test]
    fn truncated_icon_data_is_rejected() {
        assert!(from_net_wm_icon(&[16, 16, 0xffffffff], 16).is_none());
        assert!(from_net_wm_icon(&[], 16).is_none());
    }

    #[test]
    fn only_connection_errors_are_fatal() {
        let conn: Box<dyn Error> = Box::new(ConnectionError::UnknownError);
        assert!(fatal(conn.as_ref()));
        let reply: Box<dyn Error> = Box::new(ReplyError::ConnectionError(ConnectionError::UnknownError));
        assert!(fatal(reply.as_ref()));
        let other: Box<dyn Error> = "picker size".into();
        assert!(!fatal(other.as_ref()));
    }
}
