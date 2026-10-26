//! The eframe app: overlay window, settings viewport, tray and device updates.
//! Owned by `code`.

use crate::model::{Anchor, DeviceStatus, Settings, UiAction};
use crate::razer::{self, Command};
use crate::settings;
use crate::tray::{Tray, TrayCommand};
use crate::ui;
use eframe::egui::{self, ViewportBuilder, ViewportCommand, ViewportId, WindowLevel, vec2};
use std::time::{Duration, Instant};

/// Wait this long after the last change before writing settings.json.
const SAVE_DELAY: Duration = Duration::from_millis(600);
const SETTINGS_SIZE: [f32; 2] = [600.0, 760.0];

pub struct App {
    settings: Settings,
    /// What is on disk, to know when to save.
    saved: Settings,
    changed_at: Option<Instant>,
    devices: Vec<DeviceStatus>,
    razer: razer::Handle,
    tray: Option<Tray>,
    settings_open: bool,
    /// Bring the settings window to the front on the next frame.
    focus_settings: bool,
    overlay_visible: bool,
    /// A window drag is running: when it started and where the window was.
    drag: Option<(Instant, Option<(i32, i32)>)>,
    /// Window top-left in physical px as of this frame.
    window_pos: Option<(i32, i32)>,
    /// Last values sent to the window, so commands only go out on change.
    sent_size: Option<egui::Vec2>,
    sent_passthrough: Option<bool>,
    sent_on_top: Option<bool>,
    icon: std::sync::Arc<egui::IconData>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings) -> Self {
        let ctx = &cc.egui_ctx;
        ui::apply_theme(ctx, &settings);
        sync_autostart(&settings);
        let razer = razer::spawn(ctx.clone(), settings.poll_secs);
        let tray = Tray::new(ctx)
            .map_err(|e| log::error!("tray icon: {e}"))
            .ok();
        let (rgba, width, height) = crate::tray::app_icon_rgba();
        Self {
            saved: settings.clone(),
            settings,
            changed_at: None,
            devices: Vec::new(),
            razer,
            tray,
            settings_open: false,
            focus_settings: false,
            overlay_visible: true,
            drag: None,
            window_pos: None,
            sent_size: None,
            sent_passthrough: None,
            sent_on_top: None,
            icon: std::sync::Arc::new(egui::IconData {
                rgba,
                width,
                height,
            }),
        }
    }

    fn handle_action(&mut self, ctx: &egui::Context, action: UiAction) {
        match action {
            UiAction::StartDrag => {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                self.drag = Some((Instant::now(), self.window_pos));
            }
            UiAction::OpenSettings => {
                self.settings_open = true;
                self.focus_settings = true;
            }
            UiAction::CloseSettings => self.settings_open = false,
            UiAction::RefreshNow => {
                let _ = self.razer.commands.send(Command::RefreshNow);
            }
            UiAction::SettingsChanged => {}
            UiAction::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        while let Some(cmd) = self.tray.as_ref().and_then(Tray::try_recv) {
            match cmd {
                TrayCommand::OpenSettings => self.handle_action(ctx, UiAction::OpenSettings),
                TrayCommand::RefreshNow => self.handle_action(ctx, UiAction::RefreshNow),
                TrayCommand::ToggleOverlay => {
                    if let Some(tray) = &self.tray {
                        self.overlay_visible = tray.overlay_checked();
                    }
                }
                TrayCommand::Quit => self.handle_action(ctx, UiAction::Quit),
            }
        }
    }

    /// Applies side effects of a settings change and schedules the save.
    fn settings_changed(&mut self, ctx: &egui::Context, old: &Settings) {
        self.settings = settings::sanitize(self.settings.clone());
        if self.settings.poll_secs != old.poll_secs {
            let _ = self
                .razer
                .commands
                .send(Command::SetPollSecs(self.settings.poll_secs));
        }
        if self.settings.autostart != old.autostart
            && let Err(e) = settings::set_autostart(self.settings.autostart)
        {
            log::error!("autostart: {e}");
            self.settings.autostart = old.autostart;
        }
        ui::apply_theme(ctx, &self.settings);
        self.changed_at = Some(Instant::now());
    }

    fn save_if_due(&mut self, ctx: &egui::Context) {
        let Some(t) = self.changed_at else { return };
        let left = SAVE_DELAY.saturating_sub(t.elapsed());
        if !left.is_zero() {
            ctx.request_repaint_after(left);
            return;
        }
        self.changed_at = None;
        self.save_now();
    }

    fn save_now(&mut self) {
        if self.settings == self.saved {
            return;
        }
        match settings::save(&self.settings) {
            Ok(()) => self.saved = self.settings.clone(),
            Err(e) => log::error!("saving settings: {e}"),
        }
    }

    /// Size, position, click-through and z-order of the overlay window.
    fn place_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame, size: egui::Vec2) {
        if self.sent_size != Some(size) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            self.sent_size = Some(size);
        }

        let passthrough = self.settings.click_through || !self.overlay_visible;
        if self.sent_passthrough != Some(passthrough) {
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(passthrough));
            self.sent_passthrough = Some(passthrough);
        }

        let on_top = self.settings.always_on_top;
        if self.sent_on_top != Some(on_top) {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            }));
            self.sent_on_top = Some(on_top);
        }

        if self.drag.is_some() {
            return;
        }
        let monitors = monitors::all();
        let Some((x, y)) = target_position(&monitors, &self.settings, size) else {
            return;
        };
        let Some(window) = frame.winit_window() else {
            return;
        };
        let current = window.outer_position().ok().map(|p| (p.x, p.y));
        if current.is_some_and(|(cx, cy)| (cx - x).abs() <= 1 && (cy - y).abs() <= 1) {
            return;
        }
        // egui-winit multiplies by this window's pixels_per_point.
        let ppp = ctx.pixels_per_point();
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
            x as f32 / ppp,
            y as f32 / ppp,
        )));
    }

    /// While a window drag runs, wait for the mouse button to come up, then
    /// keep the new place as a free position.
    fn finish_drag(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some((started, start_pos)) = self.drag else {
            return;
        };
        if monitors::primary_button_down() || started.elapsed() < Duration::from_millis(150) {
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        self.drag = None;
        let Some(pos) = frame.winit_window().and_then(|w| w.outer_position().ok()) else {
            return;
        };
        // A click without moving keeps the anchor.
        if start_pos == Some((pos.x, pos.y)) {
            return;
        }
        let size = self.sent_size.unwrap_or(vec2(1.0, 1.0));
        let monitors = monitors::all();
        if let Some((index, free_pos)) = free_position(&monitors, (pos.x, pos.y), size) {
            let old = self.settings.clone();
            self.settings.anchor = Anchor::Free;
            self.settings.monitor = index;
            self.settings.free_pos = free_pos;
            if self.settings != old {
                self.settings_changed(ctx, &old);
            }
        }
    }

    fn show_settings(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        let id = ViewportId::from_hash_of("lu-battery-settings");
        if !self.settings_open {
            return;
        }
        if self.focus_settings {
            self.focus_settings = false;
            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
        }
        let builder = ViewportBuilder::default()
            .with_title("lu-battery – Einstellungen")
            .with_inner_size(SETTINGS_SIZE)
            .with_min_inner_size([420.0, 400.0])
            .with_icon(self.icon.clone());
        let settings = &mut self.settings;
        let devices = &self.devices;
        let close = ctx.show_viewport_immediate(id, builder, |ui, _class| {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
                .show(ui, |ui| ui::settings_panel(ui, settings, devices, actions));
            ui.ctx().input(|i| i.viewport().close_requested())
        });
        if close {
            self.settings_open = false;
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut fresh = None;
        while let Ok(list) = self.razer.updates.try_recv() {
            fresh = Some(list);
        }
        if let Some(list) = fresh {
            self.devices = list;
            if let Some(tray) = &mut self.tray {
                tray.update_tooltip(&self.devices, self.settings.precise_percent);
            }
        }
        self.handle_tray(ctx);
        if let Some(tray) = &self.tray {
            tray.set_overlay_visible(self.overlay_visible);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.window_pos = frame
            .winit_window()
            .and_then(|w| w.outer_position().ok())
            .map(|p| (p.x, p.y));
        let before = self.settings.clone();
        let mut actions = Vec::new();

        let size = if self.overlay_visible {
            ui::overlay(ui, &self.devices, &self.settings, &mut actions)
        } else {
            vec2(1.0, 1.0)
        };

        self.show_settings(&ctx, &mut actions);
        for a in actions {
            self.handle_action(&ctx, a);
        }
        if self.settings != before {
            self.settings_changed(&ctx, &before);
        }
        self.finish_drag(&ctx, frame);
        self.place_window(&ctx, frame, size);
        self.save_if_due(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.razer.commands.send(Command::Quit);
        self.save_now();
    }
}

fn sync_autostart(s: &Settings) {
    if s.autostart != settings::autostart_enabled()
        && let Err(e) = settings::set_autostart(s.autostart)
    {
        log::error!("autostart: {e}");
    }
}

// ---------------------------------------------------------------------------
// Placement math (physical px, Windows virtual desktop coordinates)

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    /// Work area (without taskbar): left, top, right, bottom.
    pub work: [i32; 4],
    /// DPI scale, 1.0 = 96 dpi.
    pub scale: f32,
}

fn monitor_for(monitors: &[Monitor], index: usize) -> Option<&Monitor> {
    monitors.get(index).or_else(|| monitors.first())
}

/// Top-left of the overlay in physical px for a content of `size` logical px.
pub fn target_position(monitors: &[Monitor], s: &Settings, size: egui::Vec2) -> Option<(i32, i32)> {
    let m = monitor_for(monitors, s.monitor)?;
    let [l, t, r, b] = m.work;
    let w = (size.x * m.scale).round() as i32;
    let h = (size.y * m.scale).round() as i32;
    let margin = (s.margin * m.scale).round() as i32;
    let (x, y) = match s.anchor {
        Anchor::TopLeft => (l + margin, t + margin),
        Anchor::TopRight => (r - margin - w, t + margin),
        Anchor::BottomLeft => (l + margin, b - margin - h),
        Anchor::BottomRight => (r - margin - w, b - margin - h),
        Anchor::Free => (
            l + (s.free_pos[0] * m.scale).round() as i32,
            t + (s.free_pos[1] * m.scale).round() as i32,
        ),
    };
    // Keep the overlay on its monitor.
    Some((x.clamp(l, (r - w).max(l)), y.clamp(t, (b - h).max(t))))
}

/// Monitor index and free position (logical px relative to the work area)
/// for a window whose top-left is at `pos` (physical px).
pub fn free_position(
    monitors: &[Monitor],
    pos: (i32, i32),
    size: egui::Vec2,
) -> Option<(usize, [f32; 2])> {
    let first = monitors.first()?;
    let (cx, cy) = (
        pos.0 + (size.x * first.scale / 2.0) as i32,
        pos.1 + (size.y * first.scale / 2.0) as i32,
    );
    let distance = |m: &Monitor| {
        let [l, t, r, b] = m.work;
        let dx = (l - cx).max(0).max(cx - r);
        let dy = (t - cy).max(0).max(cy - b);
        i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy)
    };
    let (index, m) = monitors
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| distance(m))?;
    let fx = ((pos.0 - m.work[0]) as f32 / m.scale).max(0.0).round();
    let fy = ((pos.1 - m.work[1]) as f32 / m.scale).max(0.0).round();
    Some((index, [fx, fy]))
}

#[cfg(windows)]
pub mod monitors {
    //! Win32 monitor enumeration. Index 0 is the primary monitor, the rest
    //! follow from left to right.

    use super::Monitor;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    use windows::core::BOOL;

    const MONITORINFOF_PRIMARY: u32 = 1;

    pub fn all() -> Vec<Monitor> {
        let mut found: Vec<(bool, Monitor)> = Vec::new();
        unsafe extern "system" fn callback(
            hmon: HMONITOR,
            _hdc: HDC,
            _rect: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            // SAFETY: `data` is the `&mut Vec` passed to EnumDisplayMonitors below,
            // which outlives the enumeration.
            let found = unsafe { &mut *(data.0 as *mut Vec<(bool, Monitor)>) };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: plain Win32 calls with valid out pointers.
            if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
                let (mut dx, mut dy) = (96u32, 96u32);
                let scale =
                    match unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } {
                        Ok(()) => dx as f32 / 96.0,
                        Err(_) => 1.0,
                    };
                let w = info.rcWork;
                found.push((
                    info.dwFlags & MONITORINFOF_PRIMARY != 0,
                    Monitor {
                        work: [w.left, w.top, w.right, w.bottom],
                        scale,
                    },
                ));
            }
            BOOL(1)
        }
        // SAFETY: the callback only touches `found` during this call.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(callback),
                LPARAM(&mut found as *mut _ as isize),
            );
        }
        found.sort_by_key(|(primary, m)| (!primary, m.work[0], m.work[1]));
        found.into_iter().map(|(_, m)| m).collect()
    }

    /// Whether the primary mouse button is held (respects swapped buttons).
    pub fn primary_button_down() -> bool {
        // SAFETY: plain Win32 queries without pointers.
        unsafe {
            let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
                VK_RBUTTON
            } else {
                VK_LBUTTON
            };
            GetAsyncKeyState(i32::from(key.0)) as u16 & 0x8000 != 0
        }
    }
}

#[cfg(not(windows))]
pub mod monitors {
    use super::Monitor;

    pub fn all() -> Vec<Monitor> {
        vec![Monitor {
            work: [0, 0, 1920, 1080],
            scale: 1.0,
        }]
    }

    pub fn primary_button_down() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONS: [Monitor; 2] = [
        Monitor {
            work: [0, 0, 1920, 1040],
            scale: 1.0,
        },
        Monitor {
            work: [1920, 0, 1920 + 2560, 1400],
            scale: 1.5,
        },
    ];

    fn s(anchor: Anchor, monitor: usize) -> Settings {
        Settings {
            anchor,
            monitor,
            margin: 16.0,
            ..Settings::default()
        }
    }

    #[test]
    fn corners() {
        let size = vec2(200.0, 50.0);
        assert_eq!(
            target_position(&MONS, &s(Anchor::TopLeft, 0), size),
            Some((16, 16))
        );
        assert_eq!(
            target_position(&MONS, &s(Anchor::TopRight, 0), size),
            Some((1920 - 16 - 200, 16))
        );
        assert_eq!(
            target_position(&MONS, &s(Anchor::BottomRight, 0), size),
            Some((1920 - 16 - 200, 1040 - 16 - 50))
        );
        // Second monitor at 150 %: margin 24, size 300×75.
        assert_eq!(
            target_position(&MONS, &s(Anchor::BottomLeft, 1), size),
            Some((1920 + 24, 1400 - 24 - 75))
        );
    }

    #[test]
    fn missing_monitor_falls_back_to_primary() {
        let size = vec2(200.0, 50.0);
        assert_eq!(
            target_position(&MONS, &s(Anchor::TopLeft, 7), size),
            Some((16, 16))
        );
        assert_eq!(target_position(&[], &s(Anchor::TopLeft, 0), size), None);
    }

    #[test]
    fn free_position_roundtrip() {
        let size = vec2(200.0, 50.0);
        let (index, free) = free_position(&MONS, (1920 + 300, 150), size).unwrap();
        assert_eq!(index, 1);
        assert_eq!(free, [200.0, 100.0]);
        let mut st = s(Anchor::Free, index);
        st.free_pos = free;
        assert_eq!(target_position(&MONS, &st, size), Some((1920 + 300, 150)));
    }

    #[test]
    fn free_position_is_kept_on_screen() {
        let mut st = s(Anchor::Free, 0);
        st.free_pos = [5000.0, -40.0];
        assert_eq!(
            target_position(&MONS, &st, vec2(200.0, 50.0)),
            Some((1920 - 200, 0))
        );
    }
}
