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
}
