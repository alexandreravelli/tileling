//! The X11 side. Tileling sleeps until a mouse button goes down. While a window is dragged it
//! follows the pointer through XInput2 raw events (delivered even while the window manager holds
//! the pointer grab), shows the layout picker near the top of the screen and a preview of the
//! hovered zone, then tiles the window with _NET_MOVERESIZE_WINDOW and offers the other windows
//! for the zones that are left.

use crate::config::Config;
use crate::layout::{self, Frac, Layout, Picker, R};
use std::borrow::Cow;
use std::time::Duration;
use tiny_skia::{Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::image::{BitsPerPixel, Image, ScanlinePad};
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _, XIEventMask};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

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
        _NET_FRAME_EXTENTS,
        _NET_MOVERESIZE_WINDOW,
        _NET_WM_ICON,
    }
}

const ICON: u32 = 48;
const CELL: u32 = 80;
/// The picker closes when the pointer goes this far below it.
const LEAVE: i32 = 60;
/// Ignore raw motion events closer together than this (milliseconds).
const MOTION_MS: u32 = 16;

fn debug() -> bool {
    std::env::var_os("TILELING_DEBUG").is_some()
}

struct Drag {
    client: Window,
    frame: Window,
    start: (i16, i16, u16, u16),
    moved: bool,
    area: R,
    last: u32,
}

struct Shown {
    picker: Picker,
    area: R,
    hovered: Option<(usize, usize)>,
}

struct AssistZone {
    win: Window,
    rect: R,
    zone: Frac,
    hovered: Option<usize>,
}

struct Assist {
    area: R,
    zones: Vec<AssistZone>,
    candidates: Vec<(Window, Option<Pixmap>)>,
}

pub struct App {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    depth: u8,
    visual: Visualid,
    colormap: Colormap,
    argb: bool,
    shape: bool,
    gc: Gcontext,
    cfg: Config,
    layouts: Vec<&'static Layout>,
    picker_win: Window,
    ghost_win: Window,
    drag: Option<Drag>,
    shown: Option<Shown>,
    ghost: Option<R>,
    assist: Option<Assist>,
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

pub fn run(cfg: Config) -> Res<()> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num].clone();
    let root = screen.root;

    let selection = intern(&conn, &format!("_TILELING_S{screen_num}"))?;
    if conn.get_selection_owner(selection)?.reply()?.owner != NONE {
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

    // A 32-bit visual gives real transparency when a compositor is running.
    let cm = intern(&conn, &format!("_NET_WM_CM_S{screen_num}"))?;
    let compositing = conn.get_selection_owner(cm)?.reply()?.owner != NONE;
    let (depth, visual, colormap, argb) = match argb_visual(&screen).filter(|_| compositing) {
        Some(v) => {
            let cmap = conn.generate_id()?;
            conn.create_colormap(ColormapAlloc::NONE, cmap, root, v)?;
            (32, v, cmap, true)
        }
        None => (screen.root_depth, screen.root_visual, screen.default_colormap, false),
    };

    let owner = conn.generate_id()?;
    let aux = CreateWindowAux::new();
    conn.create_window(0, owner, root, -1, -1, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux)?;
    conn.set_selection_owner(owner, selection, CURRENT_TIME)?;

    let mut app = App {
        conn,
        root,
        atoms,
        depth,
        visual,
        colormap,
        argb,
        shape,
        gc: 0,
        cfg,
        layouts: Vec::new(),
        picker_win: 0,
        ghost_win: 0,
        drag: None,
        shown: None,
        ghost: None,
        assist: None,
    };
    app.layouts = app.cfg.layouts.iter().filter_map(|n| layout::layout(n)).collect();
    app.picker_win = app.overlay(EventMask::NO_EVENT)?;
    app.ghost_win = app.overlay(EventMask::NO_EVENT)?;
    app.gc = app.conn.generate_id()?;
    app.conn.create_gc(app.gc, app.picker_win, &CreateGCAux::new())?;
    // Neither window takes input: they show up during a drag the window manager is running.
    if app.shape {
        for w in [app.picker_win, app.ghost_win] {
            app.conn.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, w, 0, 0, &[])?;
        }
    }
    app.select_raw(false)?;
    app.conn.flush()?;
    if debug() {
        eprintln!("tileling: running (argb: {argb}, shape: {shape}, layouts: {:?})", app.cfg.layouts);
    }

    loop {
        let event = app.conn.wait_for_event()?;
        app.handle(event)?;
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
            Event::KeyPress(_) => self.close_assist(),
            // Requests on windows that went away in the meantime: nothing to do.
            Event::Error(e) if debug() => {
                eprintln!("tileling: X error {:?}", e.error_kind);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // ---- X helpers -------------------------------------------------------------------------

    fn select_raw(&self, motion: bool) -> Res<()> {
        let mut mask = XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_BUTTON_RELEASE;
        if motion {
            mask |= XIEventMask::RAW_MOTION;
        }
        // Device 1: all master devices.
        self.conn.xinput_xi_select_events(self.root, &[xinput::EventMask { deviceid: 1, mask: vec![mask] }])?;
        Ok(())
    }

    fn prop32(&self, w: Window, prop: Atom, ty: impl Into<Atom>) -> Vec<u32> {
        self.conn
            .get_property(false, w, prop, ty, 0, u32::MAX / 4)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    fn pointer(&self) -> Option<QueryPointerReply> {
        self.conn.query_pointer(self.root).ok()?.reply().ok()
    }

    fn geometry(&self, w: Window) -> Option<GetGeometryReply> {
        self.conn.get_geometry(w).ok()?.reply().ok()
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

    fn client_of(&self, frame: Window) -> Option<Window> {
        let clients = self.prop32(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW);
        if clients.contains(&frame) {
            return Some(frame);
        }
        clients.into_iter().find(|&c| self.top_level(c) == Some(frame))
    }

    /// Normal windows only: not docks, panels, dialogs, the desktop or fullscreen windows.
    fn tileable(&self, w: Window) -> bool {
        let types = self.prop32(w, self.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM);
        let state = self.prop32(w, self.atoms._NET_WM_STATE, AtomEnum::ATOM);
        (types.is_empty() || types.contains(&self.atoms._NET_WM_WINDOW_TYPE_NORMAL))
            && !state.contains(&self.atoms._NET_WM_STATE_FULLSCREEN)
            && !state.contains(&self.atoms._NET_WM_STATE_SKIP_TASKBAR)
    }

    fn current_desktop(&self) -> u32 {
        self.prop32(self.root, self.atoms._NET_CURRENT_DESKTOP, AtomEnum::CARDINAL).first().copied().unwrap_or(0)
    }

    /// The work area (screen minus panels and docks) of the monitor under (x, y).
    fn work_area(&self, x: i32, y: i32) -> R {
        let screen = self
            .geometry(self.root)
            .map(|g| R { x: 0, y: 0, w: g.width as u32, h: g.height as u32 })
            .unwrap_or(R { x: 0, y: 0, w: 1024, h: 768 });
        let monitors: Vec<R> = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.monitors
                    .iter()
                    .map(|m| R { x: m.x as i32, y: m.y as i32, w: m.width as u32, h: m.height as u32 })
                    .collect()
            })
            .unwrap_or_default();
        let monitor = monitors.into_iter().find(|m| m.contains(x, y)).unwrap_or(screen);
        let desktop = self.current_desktop() as usize;
        let workarea = self.prop32(self.root, self.atoms._NET_WORKAREA, AtomEnum::CARDINAL);
        workarea
            .as_chunks::<4>()
            .0
            .get(desktop)
            .map(|c| R { x: c[0] as i32, y: c[1] as i32, w: c[2], h: c[3] })
            .and_then(|wa| monitor.intersect(&wa))
            .unwrap_or(monitor)
    }

    fn send_root_message(&self, w: Window, ty: Atom, data: [u32; 5]) -> Res<()> {
        let event = ClientMessageEvent::new(32, w, ty, data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        self.conn.send_event(false, self.root, mask, event)?;
        Ok(())
    }

    /// An override-redirect window drawn with our visual (hidden until mapped).
    fn overlay(&self, events: EventMask) -> Res<Window> {
        let w = self.conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(self.colormap)
            .override_redirect(1)
            .event_mask(events);
        self.conn.create_window(
            self.depth,
            w,
            self.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            self.visual,
            &aux,
        )?;
        Ok(w)
    }

    fn place(&self, w: Window, r: R) -> Res<()> {
        let aux = ConfigureWindowAux::new().x(r.x).y(r.y).width(r.w).height(r.h).stack_mode(StackMode::ABOVE);
        self.conn.configure_window(w, &aux)?;
        Ok(())
    }

    /// Uploads a tiny-skia pixmap as the background of `win`.
    fn upload(&self, win: Window, pm: &Pixmap) -> Res<()> {
        let (w, h) = (pm.width() as u16, pm.height() as u16);
        let mut data = pm.data().to_vec();
        for px in data.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // RGBA -> BGRA (little-endian ARGB32)
        }
        let image = Image::new(
            w,
            h,
            ScanlinePad::Pad32,
            self.depth,
            BitsPerPixel::B32,
            x11rb::image::ImageOrder::LsbFirst,
            Cow::Owned(data),
        )?;
        let xpm = self.conn.generate_id()?;
        self.conn.create_pixmap(self.depth, xpm, win, w, h)?;
        image.put(&self.conn, xpm, self.gc, 0, 0)?;
        self.conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().background_pixmap(xpm))?;
        self.conn.clear_area(false, win, 0, 0, 0, 0)?;
        self.conn.free_pixmap(xpm)?;
        Ok(())
    }

    // ---- dragging --------------------------------------------------------------------------

    fn on_press(&mut self) -> Res<()> {
        let Some(ptr) = self.pointer() else { return Ok(()) };
        if let Some(assist) = &self.assist {
            // Clicks on the assist popups arrive as ButtonPress; anywhere else closes them.
            if assist.zones.iter().any(|z| z.win == ptr.child) {
                return Ok(());
            }
            self.close_assist()?;
        }
        if ptr.child == NONE {
            return Ok(());
        }
        let Some(client) = self.client_of(ptr.child).filter(|&c| self.tileable(c)) else { return Ok(()) };
        let Some(g) = self.geometry(ptr.child) else { return Ok(()) };
        self.drag = Some(Drag {
            client,
            frame: ptr.child,
            start: (g.x, g.y, g.width, g.height),
            moved: false,
            area: self.work_area(ptr.root_x as i32, ptr.root_y as i32),
            last: 0,
        });
        // Motion is only followed while the button is down, so Tileling sleeps otherwise.
        self.select_raw(true)
    }

    fn on_motion(&mut self, time: u32) -> Res<()> {
        let Some(d) = &self.drag else { return Ok(()) };
        if time.wrapping_sub(d.last) < MOTION_MS {
            return Ok(());
        }
        let (frame, start, moved) = (d.frame, d.start, d.moved);
        let Some(ptr) = self.pointer() else { return Ok(()) };
        let (px, py) = (ptr.root_x as i32, ptr.root_y as i32);
        if !moved {
            let Some(g) = self.geometry(frame) else { return self.cancel_drag() };
            if (g.width, g.height) != (start.2, start.3) {
                return self.cancel_drag(); // a resize, not a move
            }
            if (g.x, g.y) == (start.0, start.1) {
                return Ok(()); // a click, or a drag inside the window
            }
        }
        let mut area = self.drag.as_ref().map(|d| d.area).unwrap_or_default();
        if !area.contains(px, py) && self.shown.is_none() {
            area = self.work_area(px, py);
        }
        if let Some(d) = &mut self.drag {
            d.last = time;
            d.moved = true;
            d.area = area;
        }

        match &self.shown {
            None if py < area.y + self.cfg.trigger as i32 && area.contains(px, py.max(area.y)) => {
                self.show_picker(area)?;
                self.update_hover(px, py)
            }
            None => Ok(()),
            Some(s) if py > s.picker.rect.y + s.picker.rect.h as i32 + LEAVE => self.hide_picker(),
            Some(_) => self.update_hover(px, py),
        }
    }

    fn cancel_drag(&mut self) -> Res<()> {
        self.drag = None;
        self.hide_picker()?;
        self.select_raw(false)
    }

    fn on_release(&mut self) -> Res<()> {
        let Some(drag) = self.drag.take() else { return Ok(()) };
        self.select_raw(false)?;
        let target = self.shown.as_ref().and_then(|s| s.hovered.map(|h| (s.area, h)));
        self.hide_picker()?;
        let Some((area, (li, zi))) = target.filter(|_| drag.moved) else { return Ok(()) };
        // Let the window manager finish the move before resizing the window.
        self.conn.flush()?;
        std::thread::sleep(Duration::from_millis(40));
        let zone = self.layouts[li].zones[zi];
        self.tile(drag.client, area, zone)?;
        if self.cfg.assist {
            let free: Vec<Frac> =
                self.layouts[li].zones.iter().enumerate().filter(|&(i, _)| i != zi).map(|(_, &z)| z).collect();
            self.open_assist(drag.client, area, free)?;
        }
        Ok(())
    }

    /// Moves and resizes `client` so that its frame fills `zone` of `area`.
    fn tile(&self, client: Window, area: R, zone: Frac) -> Res<()> {
        let r = layout::zone_rect(area, zone, self.cfg.gap);
        let ext = self.prop32(client, self.atoms._NET_FRAME_EXTENTS, AtomEnum::CARDINAL);
        let (l, rt, t, b) = match ext[..] {
            [l, r, t, b, ..] => (l, r, t, b),
            _ => (0, 0, 0, 0),
        };
        let a = &self.atoms;
        // Maximized (or tiled by xfwm4 at a screen edge) windows ignore geometry requests.
        self.send_root_message(
            client,
            a._NET_WM_STATE,
            [0, a._NET_WM_STATE_MAXIMIZED_VERT, a._NET_WM_STATE_MAXIMIZED_HORZ, 2, 0],
        )?;
        let w = r.w.saturating_sub(l + rt).max(1);
        let h = r.h.saturating_sub(t + b).max(1);
        // NorthWest gravity (1), x/y/width/height given (0xf00), sent by a pager-like tool (2 << 12):
        // x and y are those of the frame's outer corner.
        self.send_root_message(
            client,
            a._NET_MOVERESIZE_WINDOW,
            [1 | 0xf00 | (2 << 12), r.x as u32, r.y as u32, w, h],
        )?;
        if debug() {
            eprintln!("tileling: tile 0x{client:x} into {r:?} (frame {l} {rt} {t} {b})");
        }
        Ok(())
    }

    // ---- picker and preview ----------------------------------------------------------------

    fn show_picker(&mut self, area: R) -> Res<()> {
        let picker = layout::picker(area, self.layouts.len());
        self.place(self.picker_win, picker.rect)?;
        self.shown = Some(Shown { picker, area, hovered: None });
        self.draw_picker()?;
        self.conn.map_window(self.picker_win)?;
        Ok(())
    }

    fn hide_picker(&mut self) -> Res<()> {
        if self.shown.take().is_some() {
            self.conn.unmap_window(self.picker_win)?;
        }
        self.show_ghost(None)
    }

    fn update_hover(&mut self, px: i32, py: i32) -> Res<()> {
        let Some(s) = &self.shown else { return Ok(()) };
        let hovered = layout::hit(&s.picker, &self.layouts, px - s.picker.rect.x, py - s.picker.rect.y);
        if hovered == s.hovered {
            return Ok(());
        }
        let ghost = hovered.map(|(li, zi)| layout::zone_rect(s.area, self.layouts[li].zones[zi], self.cfg.gap));
        if let Some(s) = &mut self.shown {
            s.hovered = hovered;
        }
        self.show_ghost(ghost)?;
        self.draw_picker()?;
        // Keep the picker above the preview.
        self.conn.configure_window(self.picker_win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        Ok(())
    }

    fn draw_picker(&self) -> Res<()> {
        let Some(s) = &self.shown else { return Ok(()) };
        let r = s.picker.rect;
        let mut pm = Pixmap::new(r.w, r.h).ok_or("picker size")?;
        let radius = if self.argb { 14.0 } else { 0.0 };
        fill_rrect(&mut pm, R { x: 0, y: 0, w: r.w, h: r.h }, radius, rgba(0x1d, 0x1d, 0x1d, 0xeb));
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

    fn show_ghost(&mut self, r: Option<R>) -> Res<()> {
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
        if self.argb {
            let mut pm = Pixmap::new(r.w, r.h).ok_or("preview size")?;
            let full = R { x: 0, y: 0, w: r.w, h: r.h };
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

    // ---- assist ----------------------------------------------------------------------------

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
            .take(12)
            .collect()
    }

    fn open_assist(&mut self, tiled: Window, area: R, free: Vec<Frac>) -> Res<()> {
        let windows = self.candidates(tiled);
        if free.is_empty() || windows.is_empty() {
            return Ok(());
        }
        let candidates = windows
            .into_iter()
            .map(|w| (w, from_net_wm_icon(&self.prop32(w, self.atoms._NET_WM_ICON, AtomEnum::CARDINAL), ICON)))
            .collect();
        let mut zones = Vec::new();
        for zone in free {
            let rect = layout::zone_rect(area, zone, self.cfg.gap);
            let events = EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW;
            let win = self.overlay(events)?;
            self.place(win, rect)?;
            zones.push(AssistZone { win, rect, zone, hovered: None });
        }
        self.assist = Some(Assist { area, zones, candidates });
        for i in 0..self.assist.as_ref().map_or(0, |a| a.zones.len()) {
            self.draw_assist(i)?;
            let win = self.assist.as_ref().map(|a| a.zones[i].win).unwrap_or(NONE);
            self.conn.map_window(win)?;
        }
        // Any key closes the popups (Escape, mostly).
        self.conn.grab_keyboard(false, self.root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?;
        Ok(())
    }

    fn close_assist(&mut self) -> Res<()> {
        if let Some(a) = self.assist.take() {
            for z in a.zones {
                self.conn.destroy_window(z.win)?;
            }
            self.conn.ungrab_keyboard(CURRENT_TIME)?;
        }
        Ok(())
    }

    fn cells(&self, z: &AssistZone) -> Vec<R> {
        let n = self.assist.as_ref().map_or(0, |a| a.candidates.len());
        layout::grid(R { x: 0, y: 0, w: z.rect.w, h: z.rect.h }, n, CELL)
    }

    fn draw_assist(&self, i: usize) -> Res<()> {
        let Some(a) = &self.assist else { return Ok(()) };
        let z = &a.zones[i];
        let mut pm = Pixmap::new(z.rect.w, z.rect.h).ok_or("assist size")?;
        let full = R { x: 0, y: 0, w: z.rect.w, h: z.rect.h };
        let [ar, ag, ab] = self.cfg.accent;
        if self.argb {
            fill_rrect(&mut pm, full, 12.0, rgba(0x10, 0x10, 0x10, 0xb0));
            stroke_rrect(&mut pm, full, 12.0, 2.0, rgba(ar, ag, ab, 0x90));
        } else {
            fill_rrect(&mut pm, full, 0.0, rgba(0x24, 0x24, 0x24, 0xff));
        }
        for (ci, cell) in self.cells(z).into_iter().enumerate() {
            if z.hovered == Some(ci) {
                let inner = R { x: cell.x + 4, y: cell.y + 4, w: cell.w - 8, h: cell.h - 8 };
                fill_rrect(&mut pm, inner, 10.0, rgba(0xff, 0xff, 0xff, 0x2c));
            }
            let (ix, iy) = (cell.x + (cell.w - ICON) as i32 / 2, cell.y + (cell.h - ICON) as i32 / 2);
            match &a.candidates[ci].1 {
                Some(icon) => {
                    pm.draw_pixmap(ix, iy, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None)
                }
                None => fill_rrect(&mut pm, R { x: ix, y: iy, w: ICON, h: ICON }, 10.0, rgba(ar, ag, ab, 0xff)),
            }
        }
        self.upload(z.win, &pm)
    }

    fn assist_hover(&mut self, win: Window, at: Option<(i16, i16)>) -> Res<()> {
        let Some(i) = self.assist.as_ref().and_then(|a| a.zones.iter().position(|z| z.win == win)) else {
            return Ok(());
        };
        let cells = self.cells(&self.assist.as_ref().unwrap().zones[i]);
        let hovered = at.and_then(|(x, y)| cells.iter().position(|c| c.contains(x as i32, y as i32)));
        let z = &mut self.assist.as_mut().unwrap().zones[i];
        if z.hovered != hovered {
            z.hovered = hovered;
            self.draw_assist(i)?;
        }
        Ok(())
    }

    fn assist_click(&mut self, win: Window, x: i16, y: i16, time: Timestamp) -> Res<()> {
        let Some(i) = self.assist.as_ref().and_then(|a| a.zones.iter().position(|z| z.win == win)) else {
            return Ok(());
        };
        let cells = self.cells(&self.assist.as_ref().unwrap().zones[i]);
        let Some(ci) = cells.iter().position(|c| c.contains(x as i32, y as i32)) else { return Ok(()) };
        let a = self.assist.as_mut().unwrap();
        let (client, _) = a.candidates.remove(ci);
        let z = a.zones.remove(i);
        let area = a.area;
        self.conn.destroy_window(z.win)?;
        self.tile(client, area, z.zone)?;
        self.send_root_message(client, self.atoms._NET_ACTIVE_WINDOW, [2, time, 0, 0, 0])?;
        let a = self.assist.as_mut().unwrap();
        if a.zones.is_empty() || a.candidates.is_empty() {
            return self.close_assist();
        }
        for z in &mut a.zones {
            z.hovered = None;
        }
        for i in 0..self.assist.as_ref().unwrap().zones.len() {
            self.draw_assist(i)?;
        }
        Ok(())
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
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
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

fn fill_rrect(pm: &mut Pixmap, r: R, radius: f32, color: Color) {
    if let Some(path) = rrect_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
        pm.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

/// An outline drawn inside `r` (the stroke is centred on a path inset by half its width).
fn stroke_rrect(pm: &mut Pixmap, r: R, radius: f32, width: f32, color: Color) {
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

    #[test]
    fn picks_the_closest_icon_and_scales_it() {
        // A 2x2 red image and a 4x4 blue one.
        let mut data = vec![2, 2];
        data.extend([0xffff0000; 4]);
        data.extend([4, 4]);
        data.extend([0xff0000ff; 16]);
        let icon = from_net_wm_icon(&data, 4).unwrap();
        assert_eq!((icon.width(), icon.height()), (4, 4));
        let px = icon.pixel(1, 1).unwrap();
        assert_eq!((px.red(), px.blue(), px.alpha()), (0, 255, 255));
    }

    #[test]
    fn truncated_icon_data_is_rejected() {
        assert!(from_net_wm_icon(&[16, 16, 0xffffffff], 16).is_none());
        assert!(from_net_wm_icon(&[], 16).is_none());
    }
}
