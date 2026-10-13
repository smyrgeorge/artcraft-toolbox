//! The popover: on macOS and Windows the toolbox's window drops down from its menu-bar icon (or
//! pops up above its tray icon), without a title bar, and hides when it loses the focus, as
//! menu-bar apps do. Linux keeps a normal window: Wayland lets no app place its own window.
//!
//! Placement is a pure function of the icon's position and the screen's size ([`place`]); the
//! rest turns tray clicks and focus changes into viewport commands.

use std::time::{Duration, Instant};

use egui::{Pos2, Rect, Vec2, ViewportCommand};

/// The popover's size, in points (shorter on a short screen, [`size_for`], or with little room
/// beside its icon, [`fit`]).
pub const SIZE: Vec2 = Vec2::new(440.0, 700.0);
/// The shortest it gets.
const MIN_HEIGHT: f32 = 360.0;
/// Between the icon and the popover.
const GAP: f32 = 6.0;
/// Between the popover and a screen edge.
const MARGIN: f32 = 8.0;
/// A tray click this soon after the popover hid because that click took the focus away is the
/// same click: it meant "close", so the popover stays hidden.
const REOPEN_GUARD: Duration = Duration::from_millis(500);
/// How long the first showing waits for the menu bar to place the icon (then it shows anyway, in
/// a corner).
const PLACE_WAIT: Duration = Duration::from_secs(2);

/// Where the window is made, off every screen. eframe shows a window after its first frame
/// whatever its viewport says, and until the menu bar has placed the icon there is nowhere right
/// to show it; without a position macOS would centre it near the top of the screen, and it would
/// jump when placed. It is hidden again until [`Popover::start`] can place it.
pub const PARKED: Pos2 = Pos2::new(-20000.0, -20000.0);

/// Platforms where the window is a popover (when their tray icon exists).
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// The popover's size on a screen of `screen` points: [`SIZE`], or less tall to fit.
pub fn size_for(screen: Option<Vec2>) -> Vec2 {
    match screen {
        Some(s) => Vec2::new(SIZE.x, SIZE.y.min(s.y - 80.0).max(MIN_HEIGHT)),
        None => SIZE,
    }
}

/// Does the popover open under the icon (in the top half of the screen: a menu bar) or above it
/// (a taskbar)? `top`: the platform's bar is usually at the top, which decides when the screen
/// isn't known.
fn opens_below(icon: Rect, screen: Option<Vec2>, top: bool) -> bool {
    screen.map_or(top, |s| icon.center().y < s.y / 2.0)
}

/// Is the icon on the screen we know the size of? (An icon on another monitor has coordinates
/// outside it, and the popover is left where the icon is.)
fn on_screen(icon: Rect, screen: Vec2) -> bool {
    (0.0..=screen.x).contains(&icon.center().x) && (0.0..=screen.y).contains(&icon.center().y)
}

/// `size`, no taller than the room between the icon and the screen's edge on the side it opens
/// to (down to [`MIN_HEIGHT`]). A Windows tray icon in the hidden-icons flyout sits well above
/// the taskbar, where a full-height popover would run off the top of the screen (issue #1).
pub fn fit(icon: Option<Rect>, size: Vec2, screen: Option<Vec2>, top: bool) -> Vec2 {
    let (Some(icon), Some(s)) = (icon, screen) else { return size };
    if !on_screen(icon, s) {
        return size;
    }
    let room = if opens_below(icon, screen, top) { s.y - icon.bottom() } else { icon.top() } - GAP - MARGIN;
    Vec2::new(size.x, size.y.min(room.max(MIN_HEIGHT)))
}

/// Where the popover's top-left corner goes, in points: under the icon or above it
/// ([`opens_below`]), centred on it and kept on the screen (overlapping the icon only when even
/// a [`fit`] popover has no room beside it). Without the icon's position: the screen's top-right
/// corner (`top`) or its bottom-right corner.
pub fn place(icon: Option<Rect>, size: Vec2, screen: Option<Vec2>, top: bool) -> Pos2 {
    let Some(icon) = icon else {
        let s = screen.unwrap_or(Vec2::new(1440.0, 900.0));
        let x = s.x - size.x - MARGIN;
        let y = if top { 32.0 } else { s.y - size.y - 56.0 };
        return Pos2::new(x.max(0.0), y.max(0.0));
    };
    let mut y = if opens_below(icon, screen, top) { icon.bottom() + GAP } else { icon.top() - GAP - size.y };
    let mut x = icon.center().x - size.x / 2.0;
    if let Some(s) = screen
        && on_screen(icon, s)
    {
        x = x.clamp(MARGIN, (s.x - size.x - MARGIN).max(MARGIN));
        y = y.clamp(MARGIN, (s.y - size.y - MARGIN).max(MARGIN));
    }
    Pos2::new(x, y)
}

/// Is this where the icon really is? Just after start, before the menu bar has laid the icon
/// out, macOS reports its frame at the screen's origin, which comes out as the bottom-left
/// corner; no menu-bar or tray icon sits at x = 0.
pub fn plausible(icon: &tray_icon::Rect) -> bool {
    icon.size.width > 0 && icon.size.height > 0 && icon.position.x.is_finite() && icon.position.y.is_finite() && icon.position.x > 0.0
}

/// The tray icon's rectangle (physical pixels) in points.
pub fn icon_rect(icon: &tray_icon::Rect, pixels_per_point: f32) -> Rect {
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let min = Pos2::new(icon.position.x as f32 / ppp, icon.position.y as f32 / ppp);
    Rect::from_min_size(min, Vec2::new(icon.size.width as f32 / ppp, icon.size.height as f32 / ppp))
}

/// The popover's state: shown or not, focused or not, when it last hid on losing the focus, and
/// since when it waits to be shown the first time.
#[derive(Debug, Default)]
pub struct Popover {
    shown: bool,
    focused: bool,
    hid_on_blur: Option<Instant>,
    starting: Option<Instant>,
    /// Hidden again after eframe showed it (parked) while it waits to be placed.
    rehidden: bool,
    /// Placed without knowing the screen (it was parked off every screen): the icon it was placed
    /// at, to place it again once the screen is known.
    unsure: Option<Option<tray_icon::Rect>>,
    /// Where it was placed last (position, size).
    placed: Option<(Pos2, Vec2)>,
}

/// Show the popover the first time now? Once the icon's position and the display's scale are
/// known, or after [`PLACE_WAIT`] without them.
pub fn ready_to_place(icon_known: bool, scale_known: bool, waited: Duration) -> bool {
    (icon_known && scale_known) || waited >= PLACE_WAIT
}

impl Popover {
    /// A popover to show as soon as it can be placed ([`Popover::start`]).
    pub fn starting() -> Popover {
        Popover { starting: Some(Instant::now()), ..Popover::default() }
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// The first showing, at start: under the icon once the menu bar has placed it (call every
    /// frame; `icon` is where the tray says the icon is now).
    pub fn start(&mut self, ctx: &egui::Context, icon: Option<tray_icon::Rect>) {
        let Some(since) = self.starting else { return };
        let icon = icon.filter(plausible);
        let scale_known = ctx.input(|i| i.viewport().native_pixels_per_point.is_some());
        if ready_to_place(icon.is_some(), scale_known, since.elapsed()) {
            self.show(ctx, icon.as_ref());
        } else {
            if !std::mem::replace(&mut self.rehidden, true) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Show it under (or above) `icon`, in front, with the focus.
    pub fn show(&mut self, ctx: &egui::Context, icon: Option<&tray_icon::Rect>) {
        let icon = icon.copied().filter(plausible);
        let screen_known = self.place(ctx, icon);
        self.unsure = (!screen_known).then_some(icon);
        self.starting = None;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.shown = true;
        self.hid_on_blur = None;
    }

    /// Size and move the window for `icon`. Returns whether the screen's size was known.
    fn place(&mut self, ctx: &egui::Context, icon: Option<tray_icon::Rect>) -> bool {
        let (ppp, screen) = ctx.input(|i| (i.viewport().native_pixels_per_point, i.viewport().monitor_size));
        let top = cfg!(target_os = "macos");
        let icon_points = icon.map(|r| icon_rect(&r, ppp.unwrap_or(1.0)));
        let size = fit(icon_points, size_for(screen), screen, top);
        let pos = place(icon_points, size, screen, top);
        if self.placed != Some((pos, size)) {
            log::info!("popover at {pos:?}, {size:?} (icon {icon_points:?} in points, scale {ppp:?}, screen {screen:?})");
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
            self.placed = Some((pos, size));
        }
        screen.is_some()
    }

    /// Once the screen is known after a placement without it: place it again, now kept on the
    /// screen and sized for it (usually where it already is). Call every frame.
    pub fn refine(&mut self, ctx: &egui::Context) {
        let Some(icon) = self.unsure else { return };
        if !self.shown {
            self.unsure = None;
        } else if let Some(screen) = ctx.input(|i| i.viewport().monitor_size) {
            log::debug!("popover: the screen is {screen:?} now; placing it again");
            self.unsure = None;
            self.place(ctx, icon);
        }
    }

    pub fn hide(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.shown = false;
        self.focused = false;
    }

    /// The window was hidden another way (closed to the tray).
    pub fn hidden(&mut self) {
        self.shown = false;
        self.focused = false;
    }

    /// A click on the tray icon: show it, or hide it if it is showing (or just hid because this
    /// very click took the focus away).
    pub fn toggle(&mut self, ctx: &egui::Context, icon: Option<&tray_icon::Rect>) {
        if self.shown {
            self.hide(ctx);
        } else if self.hid_on_blur.is_some_and(|at| at.elapsed() < REOPEN_GUARD) {
            self.hid_on_blur = None;
        } else {
            self.show(ctx, icon);
        }
    }

    /// Hide on losing the focus (a click elsewhere, another app in front).
    pub fn follow_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if self.shown && self.focused && !focused {
            self.hide(ctx);
            self.hid_on_blur = Some(Instant::now());
        }
        self.focused = focused && self.shown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1512.0, 982.0);

    fn icon(x: f32, y: f32) -> Option<Rect> {
        Some(Rect::from_min_size(Pos2::new(x, y), Vec2::new(24.0, 24.0)))
    }

    #[test]
    fn under_a_menu_bar_icon_centred_on_it() {
        let p = place(icon(700.0, 0.0), SIZE, Some(SCREEN), true);
        assert_eq!(p, Pos2::new(712.0 - SIZE.x / 2.0, 24.0 + GAP));
    }

    #[test]
    fn above_a_taskbar_icon() {
        let p = place(icon(1300.0, 950.0), SIZE, Some(Vec2::new(1920.0, 1040.0)), false);
        assert_eq!(p.y, 950.0 - GAP - SIZE.y);
    }

    #[test]
    fn kept_on_the_screen_near_its_edges() {
        assert_eq!(place(icon(1480.0, 0.0), SIZE, Some(SCREEN), true).x, SCREEN.x - SIZE.x - MARGIN);
        assert_eq!(place(icon(4.0, 0.0), SIZE, Some(SCREEN), true).x, MARGIN);
        // On another monitor (outside the known screen): left where the icon is.
        assert_eq!(place(icon(2000.0, 0.0), SIZE, Some(SCREEN), true).x, 2012.0 - SIZE.x / 2.0);
    }

    /// Issue #1: Windows at 125% on a 1920×1080 screen (1536×864 points), the tray icon in the
    /// hidden-icons flyout, well above the taskbar: the full-height popover ran off the top.
    #[test]
    fn a_tray_icon_in_the_hidden_icons_flyout_keeps_the_popover_on_the_screen() {
        let screen = Some(Vec2::new(1536.0, 864.0));
        let flyout = icon(1200.0, 684.0);
        assert_eq!(place(flyout, size_for(screen), screen, false).y, MARGIN, "never off the top, even at full height");
        // Shorter instead, to stay clear of the icon.
        let size = fit(flyout, size_for(screen), screen, false);
        assert_eq!(size, Vec2::new(SIZE.x, 684.0 - GAP - MARGIN));
        let p = place(flyout, size, screen, false);
        assert_eq!((p.y, p.y + size.y), (MARGIN, 684.0 - GAP));
        // The same icon on the taskbar has room: full height, right above it.
        let taskbar = icon(1200.0, 826.0);
        assert_eq!(fit(taskbar, SIZE, screen, false), SIZE);
        assert_eq!(place(taskbar, SIZE, screen, false).y, 826.0 - GAP - SIZE.y);
    }

    #[test]
    fn without_room_beside_the_icon_it_stays_on_the_screen_over_it() {
        let screen = Some(Vec2::new(1536.0, 600.0));
        let size = fit(icon(1200.0, 320.0), size_for(screen), screen, false);
        assert_eq!(size.y, MIN_HEIGHT);
        let p = place(icon(1200.0, 320.0), size, screen, false);
        assert_eq!(p.y, MARGIN);
        assert!(p.y + size.y <= 600.0 - MARGIN);
    }

    #[test]
    fn a_menu_bar_icon_has_room_and_another_monitor_is_left_alone() {
        assert_eq!(fit(icon(700.0, 0.0), SIZE, Some(SCREEN), true), SIZE);
        // A monitor below the known screen: nothing is clamped to the wrong one.
        assert_eq!(fit(icon(300.0, 1500.0), SIZE, Some(SCREEN), false), SIZE);
        assert_eq!(place(icon(300.0, 1500.0), SIZE, Some(SCREEN), false).y, 1500.0 - GAP - SIZE.y);
        assert_eq!(fit(icon(300.0, 900.0), SIZE, None, false), SIZE, "unknown screen: as is");
    }

    #[test]
    fn without_the_screen_the_platform_decides_above_or_below() {
        assert_eq!(place(icon(700.0, 0.0), SIZE, None, true).y, 24.0 + GAP);
        assert_eq!(place(icon(1300.0, 1050.0), SIZE, None, false).y, 1050.0 - GAP - SIZE.y);
    }

    #[test]
    fn without_an_icon_position_a_corner() {
        assert_eq!(place(None, SIZE, Some(SCREEN), true), Pos2::new(SCREEN.x - SIZE.x - MARGIN, 32.0));
        assert_eq!(place(None, SIZE, Some(SCREEN), false), Pos2::new(SCREEN.x - SIZE.x - MARGIN, SCREEN.y - SIZE.y - 56.0));
        let tiny = place(None, SIZE, Some(Vec2::new(100.0, 100.0)), false);
        assert!(tiny.x >= 0.0 && tiny.y >= 0.0);
    }

    #[test]
    fn short_screens_get_a_shorter_popover() {
        assert_eq!(size_for(Some(SCREEN)), SIZE);
        assert_eq!(size_for(Some(Vec2::new(1280.0, 600.0))).y, 520.0);
        assert_eq!(size_for(Some(Vec2::new(800.0, 100.0))).y, 360.0);
        assert_eq!(size_for(None), SIZE);
    }

    #[test]
    fn an_icon_the_menu_bar_has_not_placed_yet_is_ignored() {
        let at =
            |x: f64, y: f64, w: u32| tray_icon::Rect { size: tray_icon::dpi::PhysicalSize::new(w, 44), position: tray_icon::dpi::PhysicalPosition::new(x, y) };
        // macOS before layout: the screen's origin, flipped to the bottom-left corner.
        assert!(!plausible(&at(0.0, 1920.0, 44)));
        assert!(!plausible(&at(1400.0, 0.0, 0)));
        assert!(!plausible(&at(f64::NAN, 0.0, 44)));
        assert!(plausible(&at(2700.0, 0.0, 44)));
    }

    #[test]
    fn the_first_showing_waits_for_a_place_but_not_forever() {
        assert!(!ready_to_place(false, true, Duration::from_millis(100)));
        assert!(!ready_to_place(true, false, Duration::from_millis(100)));
        assert!(ready_to_place(true, true, Duration::ZERO));
        assert!(ready_to_place(false, false, PLACE_WAIT));
    }

    #[test]
    fn physical_icon_rects_become_points() {
        let r = tray_icon::Rect { size: tray_icon::dpi::PhysicalSize::new(48, 48), position: tray_icon::dpi::PhysicalPosition::new(1400.0, 0.0) };
        assert_eq!(icon_rect(&r, 2.0), Rect::from_min_size(Pos2::new(700.0, 0.0), Vec2::new(24.0, 24.0)));
        assert_eq!(icon_rect(&r, 0.0).width(), 48.0, "a nonsense scale counts as 1");
    }
}
