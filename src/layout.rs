//! Layouts, zones, work areas and drag classification. Pure functions, no X11.
//!
//! ```
//! use tileling::layout::{self, Rect};
//!
//! // A 1920x1080 screen with a 26-pixel panel at the top.
//! let area = Rect { x: 0, y: 26, w: 1920, h: 1054 };
//! let halves = layout::layout("halves").expect("a built-in layout");
//! let left = layout::zone_rect(area, halves.zones[0], 8);
//! let right = layout::zone_rect(area, halves.zones[1], 8);
//! assert_eq!(left, Rect { x: 8, y: 34, w: 948, h: 1038 });
//! // 8 pixels around the area, and 8 between the two zones (4 on each side).
//! assert_eq!(right.x - left.right(), 8);
//! assert_eq!(area.right() - right.right(), 8);
//! ```

/// A rectangle in screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// The first column right of the rectangle (`x + w`).
    #[must_use]
    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    /// The first row below the rectangle (`y + h`).
    #[must_use]
    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    /// True when the pixel (x, y) is inside.
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The part of both rectangles, None when they do not overlap.
    ///
    /// ```
    /// use tileling::layout::Rect;
    ///
    /// let a = Rect { x: 0, y: 0, w: 100, h: 100 };
    /// assert_eq!(a.intersect(&Rect { x: 50, y: 50, w: 100, h: 100 }), Some(Rect { x: 50, y: 50, w: 50, h: 50 }));
    /// assert_eq!(a.intersect(&Rect { x: 100, y: 0, w: 10, h: 10 }), None, "touching is not overlapping");
    /// ```
    #[must_use]
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        (x1 > x0 && y1 > y0).then(|| Rect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
    }
}

/// A zone as fractions of the work area: x, y, width, height in 0..=1.
pub type Zone = [f64; 4];

/// A layout of the picker: named zones that tile the work area together.
pub struct Layout {
    /// The name used in the configuration file (`"halves"`).
    pub name: &'static str,
    /// The zones, from left to right then top to bottom.
    pub zones: &'static [Zone],
}

const T: f64 = 1.0 / 3.0;

/// Ubuntu 26.10's layouts: at most two columns or two rows.
pub const LAYOUTS: &[Layout] = &[
    Layout { name: "halves", zones: &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]] },
    Layout { name: "half-quarters", zones: &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]] },
    Layout { name: "quarters-half", zones: &[[0.0, 0.0, 0.5, 0.5], [0.0, 0.5, 0.5, 0.5], [0.5, 0.0, 0.5, 1.0]] },
    Layout {
        name: "quarters",
        zones: &[[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5], [0.0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]],
    },
    Layout { name: "two-thirds", zones: &[[0.0, 0.0, 2.0 * T, 1.0], [2.0 * T, 0.0, T, 1.0]] },
    Layout { name: "one-third", zones: &[[0.0, 0.0, T, 1.0], [T, 0.0, 2.0 * T, 1.0]] },
    Layout { name: "rows", zones: &[[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]] },
];

/// The built-in layout called `name`.
///
/// ```
/// use tileling::layout;
///
/// assert_eq!(layout::layout("quarters").map(|l| l.zones.len()), Some(4));
/// assert!(layout::layout("spiral").is_none());
/// ```
#[must_use]
pub fn layout(name: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.name == name)
}

/// The pixels of `zone` inside `area`. Edges are rounded on their own, so neighbouring zones
/// share their border exactly. `gap` is kept around the area's edges and between zones
/// (half on each side of a shared border).
#[must_use]
pub fn zone_rect(area: Rect, zone: Zone, gap: u32) -> Rect {
    let edge = |start: i32, len: u32, f: f64| start + (f64::from(len) * f).round() as i32;
    let (x0, x1) = (edge(area.x, area.w, zone[0]), edge(area.x, area.w, zone[0] + zone[2]));
    let (y0, y1) = (edge(area.y, area.h, zone[1]), edge(area.y, area.h, zone[1] + zone[3]));
    let g = gap as i32;
    let side = |at_edge: bool| if at_edge { g } else { g / 2 };
    let left = x0 + side(zone[0] <= 0.0);
    let right = x1 - side(zone[0] + zone[2] >= 1.0 - 1e-9);
    let top = y0 + side(zone[1] <= 0.0);
    let bottom = y1 - side(zone[1] + zone[3] >= 1.0 - 1e-9);
    Rect { x: left, y: top, w: (right - left).max(1) as u32, h: (bottom - top).max(1) as u32 }
}

/// What a window's frame did since the button went down.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// The frame did not change.
    None,
    /// The window is being moved.
    Move,
    /// The window is being resized.
    Resize,
}

/// Tells a move from a resize. A resize keeps the opposite edge where it was. A move keeps the
/// size, except when the window manager restores a maximized (or edge-tiled) window as the
/// drag starts: then no edge stays put, or the window was maximized to begin with.
///
/// ```
/// use tileling::layout::{Change, Rect, change};
///
/// let start = Rect { x: 100, y: 100, w: 800, h: 600 };
/// assert_eq!(change(start, Rect { x: 150, y: 90, ..start }, false), Change::Move);
/// assert_eq!(change(start, Rect { w: 900, h: 650, ..start }, false), Change::Resize, "top-left corner kept");
/// ```
#[must_use]
pub fn change(start: Rect, now: Rect, was_maximized: bool) -> Change {
    if now == start {
        return Change::None;
    }
    if (now.w, now.h) == (start.w, start.h) || was_maximized {
        return Change::Move;
    }
    let edge_kept =
        now.x == start.x || now.y == start.y || now.right() == start.right() || now.bottom() == start.bottom();
    if edge_kept { Change::Resize } else { Change::Move }
}

/// Width of one layout thumbnail in the picker; its height follows the work area's aspect ratio.
pub const THUMB_W: u32 = 112;
/// Space between the picker's border and its thumbnails.
pub const PAD: u32 = 12;
/// Space between two thumbnails.
pub const SPACING: u32 = 10;
/// The picker closes when the pointer goes this far below it (or below the trigger line).
pub const LEAVE: i32 = 60;

/// Where the picker and its thumbnails are.
pub struct Picker {
    /// The picker window, in screen coordinates.
    pub rect: Rect,
    /// Each thumbnail, relative to the picker window.
    pub thumbs: Vec<Rect>,
}

/// Places the picker at the top centre of `area` and lays out one thumbnail per layout.
///
/// ```
/// use tileling::layout::{self, Rect};
///
/// let area = Rect { x: 0, y: 26, w: 1920, h: 1054 };
/// let p = layout::picker(area, 6);
/// assert_eq!(p.thumbs.len(), 6);
/// assert_eq!(p.rect.x * 2 + p.rect.w as i32, 1920, "centred");
/// assert_eq!(p.rect.y, area.y + 8);
/// ```
#[must_use]
pub fn picker(area: Rect, count: usize) -> Picker {
    let th = (f64::from(THUMB_W) * f64::from(area.h) / f64::from(area.w.max(1))).round().clamp(40.0, 90.0) as u32;
    let n = count.max(1) as u32;
    let w = n * THUMB_W + (n - 1) * SPACING + 2 * PAD;
    let h = th + 2 * PAD;
    let rect = Rect { x: area.x + (area.w as i32 - w as i32) / 2, y: area.y + 8, w, h };
    let thumbs =
        (0..n).map(|i| Rect { x: (PAD + i * (THUMB_W + SPACING)) as i32, y: PAD as i32, w: THUMB_W, h: th }).collect();
    Picker { rect, thumbs }
}

/// Below this line the picker closes. Never above the line that opens it, so it cannot flicker.
#[must_use]
pub fn picker_close_y(p: &Picker, area: Rect, trigger: u32) -> i32 {
    p.rect.bottom().max(area.y + trigger as i32) + LEAVE
}

/// The (layout, zone) under the point (x, y), relative to the picker window. Zones cover their
/// whole thumbnail, so there is no dead space between them.
///
/// ```
/// use tileling::layout::{self, Rect};
///
/// let layouts = [layout::layout("halves").unwrap(), layout::layout("quarters").unwrap()];
/// let p = layout::picker(Rect { x: 0, y: 0, w: 1920, h: 1080 }, layouts.len());
/// let quarters = p.thumbs[1];
/// // The bottom-right corner of the second thumbnail is the fourth quarter.
/// assert_eq!(layout::hit(&p, &layouts, quarters.right() - 2, quarters.bottom() - 2), Some((1, 3)));
/// assert_eq!(layout::hit(&p, &layouts, 1, 1), None, "the padding is no zone");
/// ```
#[must_use]
pub fn hit(p: &Picker, layouts: &[&Layout], x: i32, y: i32) -> Option<(usize, usize)> {
    let (li, thumb) = p.thumbs.iter().enumerate().find(|(_, t)| t.contains(x, y))?;
    let zi = layouts.get(li)?.zones.iter().position(|&z| zone_rect(*thumb, z, 0).contains(x, y))?;
    Some((li, zi))
}

/// Square cells of `cell` pixels centred in `area`, at most `count`, as many as fit with an
/// 8-pixel margin (assist popups).
#[must_use]
pub fn grid(area: Rect, count: usize, cell: u32) -> Vec<Rect> {
    let fit_cols = (area.w.saturating_sub(16) / cell).max(1) as usize;
    let fit_rows = (area.h.saturating_sub(16) / cell).max(1) as usize;
    let count = count.min(fit_cols * fit_rows);
    if count == 0 {
        return Vec::new();
    }
    let cols = fit_cols.min(count);
    let rows = count.div_ceil(cols);
    let (gw, gh) = (cols as u32 * cell, rows as u32 * cell);
    let x0 = area.x + (area.w as i32 - gw as i32) / 2;
    let y0 = area.y + (area.h as i32 - gh as i32) / 2;
    (0..count)
        .map(|i| Rect {
            x: x0 + ((i % cols) as u32 * cell) as i32,
            y: y0 + ((i / cols) as u32 * cell) as i32,
            w: cell,
            h: cell,
        })
        .collect()
}

/// A dock or panel's `_NET_WM_STRUT_PARTIAL`: left, right, top, bottom, then the start and end of
/// each (`left_start_y`, `left_end_y`, `right_start_y`, `right_end_y`, `top_start_x`, `top_end_x`,
/// `bottom_start_x`, `bottom_end_x`). Plain `_NET_WM_STRUT` has only the first four values.
pub type Strut = [u32; 12];

/// The strut of a window from its `_NET_WM_STRUT_PARTIAL` (12 values) or `_NET_WM_STRUT` (4 values,
/// spanning the whole `screen`); None for anything else.
///
/// ```
/// use tileling::layout::{self, Rect};
///
/// let screen = Rect { x: 0, y: 0, w: 1920, h: 1080 };
/// let top_panel = layout::strut(&[0, 0, 30, 0], screen).unwrap();
/// assert_eq!(&top_panel[8..10], &[0, 1919], "over the whole width");
/// assert_eq!(layout::strut(&[1, 2, 3], screen), None);
/// ```
#[must_use]
pub fn strut(values: &[u32], screen: Rect) -> Option<Strut> {
    match values.len() {
        n if n >= 12 => values[..12].try_into().ok(),
        4 => {
            let (w, h) = (screen.w.saturating_sub(1), screen.h.saturating_sub(1));
            Some([values[0], values[1], values[2], values[3], 0, h, 0, h, 0, w, 0, w])
        }
        _ => None,
    }
}

/// The part of `monitor` left by the struts of docks and panels. Unlike `_NET_WORKAREA`, which is
/// one rectangle for the whole screen, a panel on one monitor does not shrink the others.
///
/// ```
/// use tileling::layout::{self, Rect};
///
/// let screen = Rect { x: 0, y: 0, w: 3840, h: 1080 };
/// let (left, right) = (Rect { w: 1920, ..screen }, Rect { x: 1920, w: 1920, ..screen });
/// // A 30-pixel panel at the top of the left monitor only.
/// let panel = [0, 0, 30, 0, 0, 0, 0, 0, 0, 1919, 0, 0];
/// assert_eq!(layout::work_area(left, screen, &[panel]).y, 30);
/// assert_eq!(layout::work_area(right, screen, &[panel]), right);
/// ```
#[must_use]
pub fn work_area(monitor: Rect, screen: Rect, struts: &[Strut]) -> Rect {
    let (mut left, mut top, mut right, mut bottom) = (monitor.x, monitor.y, monitor.right(), monitor.bottom());
    let overlaps = |a0: i32, a1: i32, b0: u32, b1: u32| (b0 as i32) < a1 && (b1 as i32) >= a0;
    for s in struts {
        let [l, r, t, b, ly0, ly1, ry0, ry1, tx0, tx1, bx0, bx1] = *s;
        if l > 0 && overlaps(monitor.y, monitor.bottom(), ly0, ly1) && (l as i32) > monitor.x {
            left = left.max(screen.x + l as i32);
        }
        if r > 0 && overlaps(monitor.y, monitor.bottom(), ry0, ry1) && screen.right() - (r as i32) < monitor.right() {
            right = right.min(screen.right() - r as i32);
        }
        if t > 0 && overlaps(monitor.x, monitor.right(), tx0, tx1) && (t as i32) > monitor.y {
            top = top.max(screen.y + t as i32);
        }
        if b > 0 && overlaps(monitor.x, monitor.right(), bx0, bx1) && screen.bottom() - (b as i32) < monitor.bottom() {
            bottom = bottom.min(screen.bottom() - b as i32);
        }
    }
    if right - left < 100 || bottom - top < 100 {
        return monitor; // nonsense struts: ignore them rather than tile into a sliver
    }
    Rect { x: left, y: top, w: (right - left) as u32, h: (bottom - top) as u32 }
}

/// `_NET_MOVERESIZE_WINDOW` data that makes a frame with these extents (left, right, top, bottom)
/// fill `r`: `NorthWest` gravity (1), x/y/width/height given (0xf00), from a pager-like tool (2 << 12).
/// With `NorthWest` gravity, x and y are those of the frame's outer corner; width and height are
/// the client's.
///
/// ```
/// use tileling::layout::{self, Rect};
///
/// // Fill a zone with a window whose frame has a 29-pixel title bar and 5-pixel borders.
/// let zone = Rect { x: 8, y: 34, w: 948, h: 978 };
/// assert_eq!(layout::moveresize_data(zone, [5, 5, 29, 5]), [0x2f01, 8, 34, 938, 944]);
/// ```
#[must_use]
pub fn moveresize_data(r: Rect, extents: [u32; 4]) -> [u32; 5] {
    let [l, rt, t, b] = extents;
    let w = r.w.saturating_sub(l + rt).max(1);
    let h = r.h.saturating_sub(t + b).max(1);
    [1 | 0xf00 | (2 << 12), r.x as u32, r.y as u32, w, h]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 26, w: 1920, h: 1000 };
    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    #[test]
    fn zones_tile_the_area_without_gaps() {
        for l in LAYOUTS {
            let total: u64 =
                l.zones.iter().map(|&z| zone_rect(AREA, z, 0)).map(|r| u64::from(r.w) * u64::from(r.h)).sum();
            assert_eq!(total, u64::from(AREA.w) * u64::from(AREA.h), "{}", l.name);
        }
    }

    #[test]
    fn gaps_are_even() {
        let l = layout("halves").unwrap();
        let (a, b) = (zone_rect(AREA, l.zones[0], 8), zone_rect(AREA, l.zones[1], 8));
        assert_eq!((a.x, a.y), (8, 34));
        assert_eq!(b.x - a.right(), 8, "gap between the two zones");
        assert_eq!(AREA.right() - b.right(), 8, "gap at the right edge");
        assert_eq!(AREA.bottom() - a.bottom(), 8, "gap at the bottom edge");
    }

    #[test]
    fn thirds_share_their_border() {
        let l = layout("two-thirds").unwrap();
        let (a, b) = (zone_rect(AREA, l.zones[0], 0), zone_rect(AREA, l.zones[1], 0));
        assert_eq!(a.right(), b.x);
        assert_eq!(b.right(), 1920);
    }

    #[test]
    fn moves_and_resizes_are_told_apart() {
        let start = Rect { x: 100, y: 100, w: 800, h: 600 };
        assert_eq!(change(start, start, false), Change::None);
        assert_eq!(change(start, Rect { x: 150, y: 90, ..start }, false), Change::Move);
        // Resizes keep the opposite edge: from the bottom-right corner, from the top-left corner.
        assert_eq!(change(start, Rect { w: 900, h: 650, ..start }, false), Change::Resize);
        assert_eq!(change(start, Rect { x: 50, y: 80, w: 850, h: 620 }, false), Change::Resize);
        // A maximized window restored by the window manager as the drag starts.
        let max = Rect { x: 0, y: 26, w: 1920, h: 1000 };
        assert_eq!(change(max, Rect { x: 400, y: 200, w: 800, h: 600 }, false), Change::Move);
        assert_eq!(change(max, Rect { x: 400, y: 26, w: 800, h: 600 }, true), Change::Move, "top edge kept by chance");
    }

    #[test]
    fn picker_is_centred_and_hits_zones() {
        let layouts: Vec<&Layout> = LAYOUTS.iter().collect();
        let p = picker(AREA, layouts.len());
        assert_eq!(p.rect.x + p.rect.w as i32 / 2, 960);
        assert_eq!(p.rect.y, 34);
        let t = p.thumbs[3]; // quarters
        assert_eq!(hit(&p, &layouts, t.x + 2, t.y + 2), Some((3, 0)));
        assert_eq!(hit(&p, &layouts, t.right() - 2, t.bottom() - 2), Some((3, 3)));
        assert_eq!(hit(&p, &layouts, 1, 1), None, "padding is not a zone");
    }

    #[test]
    fn picker_never_closes_above_the_line_that_opens_it() {
        let p = picker(AREA, 6);
        for trigger in [4, 40, 150, 200] {
            assert!(picker_close_y(&p, AREA, trigger) > AREA.y + trigger as i32, "trigger {trigger}");
        }
    }

    #[test]
    fn grid_is_centred_and_never_overflows() {
        let cells = grid(Rect { x: 0, y: 0, w: 400, h: 300 }, 3, 80);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].x, (400 - 240) / 2);
        assert_eq!(cells[0].y, (300 - 80) / 2);
        let small = Rect { x: 0, y: 0, w: 200, h: 120 };
        let cells = grid(small, 12, 80);
        assert_eq!(cells.len(), 2, "only what fits");
        assert!(cells.iter().all(|c| small.intersect(c) == Some(*c)));
        assert_eq!(grid(small, 0, 80), Vec::<Rect>::new());
    }

    #[test]
    fn work_area_follows_panels_and_docks() {
        // Three top panels side by side (26 px) and a centred dock at the bottom (60 px).
        let struts = [
            [0, 0, 26, 0, 0, 0, 0, 0, 0, 863, 0, 0],
            [0, 0, 26, 0, 0, 0, 0, 0, 864, 1055, 0, 0],
            [0, 0, 26, 0, 0, 0, 0, 0, 1056, 1919, 0, 0],
            [0, 0, 0, 60, 0, 0, 0, 0, 0, 0, 760, 1160],
        ];
        assert_eq!(work_area(SCREEN, SCREEN, &struts), Rect { x: 0, y: 26, w: 1920, h: 994 });
    }

    #[test]
    fn a_panel_on_one_monitor_leaves_the_other_alone() {
        let screen = Rect { x: 0, y: 0, w: 3840, h: 1080 };
        let (left, right) = (Rect { x: 0, y: 0, w: 1920, h: 1080 }, Rect { x: 1920, y: 0, w: 1920, h: 1080 });
        let struts = [[0, 0, 30, 0, 0, 0, 0, 0, 0, 1919, 0, 0]];
        assert_eq!(work_area(left, screen, &struts).y, 30);
        assert_eq!(work_area(right, screen, &struts), right);
        // _NET_WM_STRUT (no ranges) spans the whole screen.
        let plain = strut(&[0, 0, 30, 0], screen).unwrap();
        assert_eq!(work_area(right, screen, &[plain]).y, 30);
    }

    #[test]
    fn moveresize_payload() {
        let r = Rect { x: 8, y: 34, w: 948, h: 978 };
        assert_eq!(moveresize_data(r, [5, 5, 29, 5]), [0x2f01, 8, 34, 938, 944]);
        let left_monitor = Rect { x: -1920, ..r };
        assert_eq!(moveresize_data(left_monitor, [0; 4])[1] as i32, -1920);
    }

    #[test]
    fn intersect() {
        let a = Rect { x: 0, y: 0, w: 100, h: 100 };
        assert_eq!(a.intersect(&Rect { x: 50, y: 50, w: 100, h: 100 }), Some(Rect { x: 50, y: 50, w: 50, h: 50 }));
        assert_eq!(a.intersect(&Rect { x: 100, y: 0, w: 10, h: 10 }), None);
    }
}
