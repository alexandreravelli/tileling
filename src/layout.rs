//! Layouts, zones and the geometry of the picker. Pure functions, no X11.

/// A rectangle in screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl R {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }

    pub fn intersect(&self, o: &R) -> Option<R> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w as i32).min(o.x + o.w as i32);
        let y1 = (self.y + self.h as i32).min(o.y + o.h as i32);
        (x1 > x0 && y1 > y0).then(|| R { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
    }
}

/// A zone as fractions of the work area: x, y, width, height in 0..=1.
pub type Frac = [f64; 4];

pub struct Layout {
    pub name: &'static str,
    pub zones: &'static [Frac],
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

pub fn layout(name: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.name == name)
}

/// The pixels of `zone` inside `area`. Edges are rounded on their own, so neighbouring zones
/// share their border exactly. `gap` is kept around the area's edges and between zones
/// (half on each side of a shared border).
pub fn zone_rect(area: R, zone: Frac, gap: u32) -> R {
    let edge = |start: i32, len: u32, f: f64| start + (len as f64 * f).round() as i32;
    let (x0, x1) = (edge(area.x, area.w, zone[0]), edge(area.x, area.w, zone[0] + zone[2]));
    let (y0, y1) = (edge(area.y, area.h, zone[1]), edge(area.y, area.h, zone[1] + zone[3]));
    let g = gap as i32;
    let side = |at_edge: bool| if at_edge { g } else { g / 2 };
    let left = x0 + side(zone[0] <= 0.0);
    let right = x1 - side(zone[0] + zone[2] >= 1.0 - 1e-9);
    let top = y0 + side(zone[1] <= 0.0);
    let bottom = y1 - side(zone[1] + zone[3] >= 1.0 - 1e-9);
    R { x: left, y: top, w: (right - left).max(1) as u32, h: (bottom - top).max(1) as u32 }
}

/// Size of one layout thumbnail in the picker, with the work area's aspect ratio.
pub const THUMB_W: u32 = 112;
pub const PAD: u32 = 12;
pub const SPACING: u32 = 10;

pub struct Picker {
    /// The picker window, in screen coordinates.
    pub rect: R,
    /// Each thumbnail, relative to the picker window.
    pub thumbs: Vec<R>,
}

/// Places the picker at the top centre of `area` and lays out one thumbnail per layout.
pub fn picker(area: R, count: usize) -> Picker {
    let th = ((THUMB_W as f64) * area.h as f64 / area.w.max(1) as f64).round().clamp(40.0, 90.0) as u32;
    let n = count.max(1) as u32;
    let w = n * THUMB_W + (n - 1) * SPACING + 2 * PAD;
    let h = th + 2 * PAD;
    let rect = R { x: area.x + (area.w as i32 - w as i32) / 2, y: area.y + 8, w, h };
    let thumbs =
        (0..n).map(|i| R { x: (PAD + i * (THUMB_W + SPACING)) as i32, y: PAD as i32, w: THUMB_W, h: th }).collect();
    Picker { rect, thumbs }
}

/// The (layout, zone) under the point (x, y), relative to the picker window. Zones cover their
/// whole thumbnail, so there is no dead space between them.
pub fn hit(p: &Picker, layouts: &[&Layout], x: i32, y: i32) -> Option<(usize, usize)> {
    let (li, thumb) = p.thumbs.iter().enumerate().find(|(_, t)| t.contains(x, y))?;
    let zi = layouts.get(li)?.zones.iter().position(|&z| zone_rect(*thumb, z, 0).contains(x, y))?;
    Some((li, zi))
}

/// A grid of `count` square cells of `cell` pixels, centred in `area` (assist popups).
pub fn grid(area: R, count: usize, cell: u32) -> Vec<R> {
    let cols = ((area.w.saturating_sub(16)) / cell).max(1) as usize;
    let cols = cols.min(count.max(1));
    let rows = count.div_ceil(cols);
    let (gw, gh) = (cols as u32 * cell, rows as u32 * cell);
    let x0 = area.x + (area.w as i32 - gw as i32) / 2;
    let y0 = area.y + (area.h as i32 - gh as i32) / 2;
    (0..count)
        .map(|i| R {
            x: x0 + ((i % cols) as u32 * cell) as i32,
            y: y0 + ((i / cols) as u32 * cell) as i32,
            w: cell,
            h: cell,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: R = R { x: 0, y: 26, w: 1920, h: 1000 };

    #[test]
    fn zones_tile_the_area_without_gaps() {
        for l in LAYOUTS {
            let total: u64 = l.zones.iter().map(|&z| zone_rect(AREA, z, 0)).map(|r| r.w as u64 * r.h as u64).sum();
            assert_eq!(total, AREA.w as u64 * AREA.h as u64, "{}", l.name);
        }
    }

    #[test]
    fn gaps_are_even() {
        let l = layout("halves").unwrap();
        let (a, b) = (zone_rect(AREA, l.zones[0], 8), zone_rect(AREA, l.zones[1], 8));
        assert_eq!((a.x, a.y), (8, 34));
        assert_eq!(b.x - (a.x + a.w as i32), 8, "gap between the two zones");
        assert_eq!(AREA.x + AREA.w as i32 - (b.x + b.w as i32), 8, "gap at the right edge");
        assert_eq!(AREA.y + AREA.h as i32 - (a.y + a.h as i32), 8, "gap at the bottom edge");
    }

    #[test]
    fn thirds_share_their_border() {
        let l = layout("two-thirds").unwrap();
        let (a, b) = (zone_rect(AREA, l.zones[0], 0), zone_rect(AREA, l.zones[1], 0));
        assert_eq!(a.x + a.w as i32, b.x);
        assert_eq!(b.x + b.w as i32, 1920);
    }

    #[test]
    fn picker_is_centred_and_hits_zones() {
        let layouts: Vec<&Layout> = LAYOUTS.iter().collect();
        let p = picker(AREA, layouts.len());
        assert_eq!(p.rect.x + p.rect.w as i32 / 2, 960);
        assert_eq!(p.rect.y, 34);
        let t = p.thumbs[3]; // quarters
        assert_eq!(hit(&p, &layouts, t.x + 2, t.y + 2), Some((3, 0)));
        assert_eq!(hit(&p, &layouts, t.x + t.w as i32 - 2, t.y + t.h as i32 - 2), Some((3, 3)));
        assert_eq!(hit(&p, &layouts, 1, 1), None, "padding is not a zone");
    }

    #[test]
    fn grid_is_centred() {
        let cells = grid(R { x: 0, y: 0, w: 400, h: 300 }, 3, 80);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].x, (400 - 240) / 2);
        assert_eq!(cells[0].y, (300 - 80) / 2);
    }

    #[test]
    fn intersect() {
        let a = R { x: 0, y: 0, w: 100, h: 100 };
        assert_eq!(a.intersect(&R { x: 50, y: 50, w: 100, h: 100 }), Some(R { x: 50, y: 50, w: 50, h: 50 }));
        assert_eq!(a.intersect(&R { x: 100, y: 0, w: 10, h: 10 }), None);
    }
}
