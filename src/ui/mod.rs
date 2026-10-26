//! Overlay and settings UI in the lu-minal "Moon" theme. Owned by `design`.
//! Stub: the signatures are the fixed contract from CONCEPT.md.

use crate::model::{DeviceStatus, Settings, UiAction};
use eframe::egui;

pub fn apply_theme(_ctx: &egui::Context, _settings: &Settings) {}

pub fn overlay(
    ui: &mut egui::Ui,
    devices: &[DeviceStatus],
    _settings: &Settings,
    _actions: &mut Vec<UiAction>,
) -> egui::Vec2 {
    for d in devices {
        let pct = d.battery.map(|b| format!("{:.0}%", b.percent())).unwrap_or("–".into());
        ui.label(format!("{} {}", d.name, pct));
    }
    egui::vec2(220.0, 60.0)
}

pub fn settings_panel(
    ui: &mut egui::Ui,
    _settings: &mut Settings,
    _devices: &[DeviceStatus],
    _actions: &mut Vec<UiAction>,
) {
    ui.label("Settings");
}
