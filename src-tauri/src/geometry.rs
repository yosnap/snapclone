//! Pure rectangle maths for placing windows on the desktop, in whatever unit the caller
//! uses consistently (physical pixels for the editor, points for the scroll overlays).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    fn contains(&self, other: &Rect) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }

    fn intersection_area(&self, other: &Rect) -> i64 {
        let width = self.right().min(other.right()) - i64::from(self.x.max(other.x));
        let height = self.bottom().min(other.bottom()) - i64::from(self.y.max(other.y));
        width.max(0) * height.max(0)
    }

    fn area(&self) -> i64 {
        i64::from(self.width) * i64::from(self.height)
    }
}

/// Smallest gap kept between a centred window and the edges of the work area.
const SCREEN_MARGIN: u32 = 24;

/// Centres a window of `width` x `height` in `area`, shrinking it first if it does not fit.
pub fn centered(width: u32, height: u32, area: Rect) -> Rect {
    let fit = |size: u32, available: u32| size.min(available.saturating_sub(SCREEN_MARGIN * 2).max(1));
    let (width, height) = (fit(width, area.width), fit(height, area.height));
    let offset = |available: u32, size: u32| i32::try_from((available - size) / 2).unwrap_or(0);
    Rect::new(area.x + offset(area.width, width), area.y + offset(area.height, height), width, height)
}

/// Whether a window can be used where it is: its top edge (where the title bar is) lies on
/// some monitor that also shows at least a quarter of it.
pub fn is_reachable(window: Rect, monitors: &[Rect]) -> bool {
    let quarter = window.area() / 4;
    monitors.iter().any(|monitor| {
        let top_on_monitor = window.y >= monitor.y && i64::from(window.y) < monitor.bottom();
        top_on_monitor && monitor.intersection_area(&window) >= quarter.max(1)
    })
}

/// Places a toolbar of `width` x `height` next to `zone` without covering it: below, then
/// above, on the zone's monitor; then centred low on another monitor. Only when nothing
/// else fits does it overlap the zone, at the bottom of its monitor.
pub fn toolbar_position(zone: Rect, monitor: Rect, others: &[Rect], width: u32, height: u32, gap: u32) -> Rect {
    let clamp_x = |area: Rect| {
        let centred = i64::from(zone.x) + (i64::from(zone.width) - i64::from(width)) / 2;
        let max = (area.right() - i64::from(width)).max(i64::from(area.x));
        centred.clamp(i64::from(area.x), max) as i32
    };
    let gap = i64::from(gap);
    let below = Rect::new(clamp_x(monitor), (zone.bottom() + gap) as i32, width, height);
    let above = Rect::new(clamp_x(monitor), (i64::from(zone.y) - gap - i64::from(height)) as i32, width, height);
    if let Some(spot) = [below, above].into_iter().find(|spot| monitor.contains(spot)) {
        return spot;
    }
    let low_on = |area: Rect| {
        let x = i64::from(area.x) + (i64::from(area.width) - i64::from(width)) / 2;
        let y = area.bottom() - i64::from(height) - gap * 4;
        Rect::new(x.max(i64::from(area.x)) as i32, y.max(i64::from(area.y)) as i32, width, height)
    };
    others
        .iter()
        .map(|other| low_on(*other))
        .find(|spot| spot.intersection_area(&zone) == 0)
        .unwrap_or_else(|| low_on(monitor))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect::new(0, 0, 2560, 1440);
    const LEFT: Rect = Rect::new(-2560, 360, 2560, 1080);

    #[test]
    fn centres_inside_the_work_area() {
        assert_eq!(centered(1000, 700, Rect::new(0, 25, 2560, 1415)), Rect::new(780, 382, 1000, 700));
        assert_eq!(centered(1000, 700, LEFT), Rect::new(-1780, 550, 1000, 700));
    }

    #[test]
    fn shrinks_windows_larger_than_the_screen() {
        assert_eq!(centered(3000, 2000, Rect::new(0, 0, 1920, 1080)), Rect::new(24, 24, 1872, 1032));
    }

    #[test]
    fn windows_off_every_monitor_are_not_reachable() {
        let monitors = [MAIN, LEFT];
        assert!(is_reachable(Rect::new(100, 100, 1000, 700), &monitors));
        assert!(is_reachable(Rect::new(-1500, 500, 1000, 700), &monitors));
        // A monitor that was unplugged, or the title bar above every screen.
        assert!(!is_reachable(Rect::new(6000, 100, 1000, 700), &monitors));
        assert!(!is_reachable(Rect::new(100, -300, 1000, 700), &monitors));
        // Only a sliver left on screen.
        assert!(!is_reachable(Rect::new(2500, 100, 1000, 700), &monitors));
    }

    #[test]
    fn toolbar_goes_below_then_above_the_zone() {
        let zone = Rect::new(100, 100, 800, 600);
        assert_eq!(toolbar_position(zone, MAIN, &[], 400, 44, 8), Rect::new(300, 708, 400, 44));
        let low = Rect::new(100, 800, 800, 620);
        assert_eq!(toolbar_position(low, MAIN, &[], 400, 44, 8), Rect::new(300, 748, 400, 44));
    }

    #[test]
    fn toolbar_stays_on_the_monitor_near_its_edges() {
        let zone = Rect::new(0, 100, 200, 600);
        assert_eq!(toolbar_position(zone, MAIN, &[], 400, 44, 8).x, 0);
    }

    #[test]
    fn full_screen_zone_sends_the_toolbar_to_another_monitor() {
        let spot = toolbar_position(MAIN, MAIN, &[LEFT], 400, 44, 8);
        assert!(LEFT.contains(&spot));
        assert_eq!(spot.intersection_area(&MAIN), 0);
    }

    #[test]
    fn single_full_screen_zone_falls_back_to_overlapping() {
        let spot = toolbar_position(MAIN, MAIN, &[], 400, 44, 8);
        assert!(MAIN.contains(&spot));
    }
}
