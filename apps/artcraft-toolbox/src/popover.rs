//! The popover: on macOS and Windows the toolbox's window drops down from its menu-bar icon (or
//! pops up above its tray icon), without a title bar, and hides when it loses the focus, as
//! menu-bar apps do. Linux keeps a normal window: Wayland lets no app place its own window.
//!
//! Placement is a pure function of the icon's position and the screen's size ([`place`]); the
//! rest turns tray clicks and focus changes into viewport commands.

use std::time::{Duration, Instant};

use egui::{Pos2, Rect, Vec2, ViewportCommand};

/// The popover's size, in points (shorter on a short screen, [`size_for`]).
pub const SIZE: Vec2 = Vec2::new(440.0, 700.0);
/// Between the icon and the popover.
const GAP: f32 = 6.0;
/// Between the popover and a screen edge.
const MARGIN: f32 = 8.0;
/// A tray click this soon after the popover hid because that click took the focus away is the
/// same click: it meant "close", so the popover stays hidden.
const REOPEN_GUARD: Duration = Duration::from_millis(500);

/// Platforms where the window is a popover (when their tray icon exists).
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// The popover's size on a screen of `screen` points: [`SIZE`], or less tall to fit.
pub fn size_for(screen: Option<Vec2>) -> Vec2 {
    match screen {
        Some(s) => Vec2::new(SIZE.x, SIZE.y.min(s.y - 80.0).max(360.0)),
        None => SIZE,
    }
}

/// Where the popover's top-left corner goes, in points: under the icon when it is in the top
/// half of the screen (a menu bar), above it otherwise (a taskbar), centred on it and kept on the
/// screen. Without the icon's position: the screen's top-right corner (`top`: a menu bar) or its
/// bottom-right corner (a taskbar).
pub fn place(icon: Option<Rect>, size: Vec2, screen: Option<Vec2>, top: bool) -> Pos2 {
    let Some(icon) = icon else {
        let s = screen.unwrap_or(Vec2::new(1440.0, 900.0));
        let x = s.x - size.x - MARGIN;
        let y = if top { 32.0 } else { s.y - size.y - 56.0 };
        return Pos2::new(x.max(0.0), y.max(0.0));
    };
    let below = screen.is_none_or(|s| icon.center().y < s.y / 2.0);
    let y = if below { icon.bottom() + GAP } else { icon.top() - GAP - size.y };
    let mut x = icon.center().x - size.x / 2.0;
    // Kept on the screen we know the size of, when the icon is on it (an icon on another monitor
    // has coordinates outside it, and is left alone).
    if let Some(s) = screen
        && (0.0..=s.x).contains(&icon.center().x)
    {
        x = x.clamp(MARGIN, (s.x - size.x - MARGIN).max(MARGIN));
    }
    Pos2::new(x, y)
}

/// The tray icon's rectangle (physical pixels) in points.
pub fn icon_rect(icon: &tray_icon::Rect, pixels_per_point: f32) -> Rect {
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let min = Pos2::new(icon.position.x as f32 / ppp, icon.position.y as f32 / ppp);
    Rect::from_min_size(min, Vec2::new(icon.size.width as f32 / ppp, icon.size.height as f32 / ppp))
}

/// The popover's state: shown or not, focused or not, and when it last hid on losing the focus.
#[derive(Debug, Default)]
pub struct Popover {
    shown: bool,
    focused: bool,
    hid_on_blur: Option<Instant>,
}

impl Popover {
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// Show it under (or above) `icon`, in front, with the focus.
    pub fn show(&mut self, ctx: &egui::Context, icon: Option<&tray_icon::Rect>) {
        let (ppp, screen) = ctx.input(|i| (i.viewport().native_pixels_per_point.unwrap_or(1.0), i.viewport().monitor_size));
        let size = size_for(screen);
        let pos = place(icon.map(|r| icon_rect(r, ppp)), size, screen, cfg!(target_os = "macos"));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.shown = true;
        self.hid_on_blur = None;
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
    fn physical_icon_rects_become_points() {
        let r = tray_icon::Rect { size: tray_icon::dpi::PhysicalSize::new(48, 48), position: tray_icon::dpi::PhysicalPosition::new(1400.0, 0.0) };
        assert_eq!(icon_rect(&r, 2.0), Rect::from_min_size(Pos2::new(700.0, 0.0), Vec2::new(24.0, 24.0)));
        assert_eq!(icon_rect(&r, 0.0).width(), 48.0, "a nonsense scale counts as 1");
    }
}
