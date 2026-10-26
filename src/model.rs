//! Shared data contract between the device layer (`razer`), the app shell
//! (`app`) and the UI (`ui`). Owned by the lead: changes go through luna.

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// What kind of gear a device is; picks the icon in the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    Mouse,
    Headset,
    Keyboard,
    Other,
}

/// Charging state as reported by the device itself.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charging {
    Charging,
    Discharging,
    /// Fully charged while on the cable/dock.
    Full,
    Unknown,
}

/// One battery reading, exactly as the device reported it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryReading {
    /// Raw value from the device (Razer mice: 0..=255, headsets: device-specific).
    pub raw: u16,
    /// The largest raw value, so `percent` is exact and never guessed.
    pub raw_max: u16,
    pub charging: Charging,
    pub read_at: Instant,
}

impl BatteryReading {
    /// Percent with one decimal of precision, e.g. 87.5.
    pub fn percent(&self) -> f32 {
        if self.raw_max == 0 {
            return 0.0;
        }
        (self.raw as f32 * 100.0 / self.raw_max as f32).clamp(0.0, 100.0)
    }
}

/// Connection state of a device.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// Device answered; `battery` is fresh.
    Online,
    /// Dongle is plugged in but the device is off / asleep / out of range.
    Asleep,
    /// Last read failed; `battery` holds the last good value (if any).
    Error(String),
}

/// Everything the UI needs to draw one device row.
#[derive(Debug, Clone)]
pub struct DeviceStatus {
    /// Stable id, e.g. "1532:00ab" — used as key in `Settings::hidden_devices`.
    pub id: String,
    /// Human name, e.g. "Basilisk V3 Pro".
    pub name: String,
    pub kind: DeviceKind,
    pub link: Link,
    /// Last good reading. `None` until the first successful read.
    pub battery: Option<BatteryReading>,
}

/// Where the overlay sits on its monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Free position, dragged by the user (`Settings::free_pos`).
    Free,
}

/// Overlay layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    /// Devices side by side.
    Horizontal,
    /// Devices stacked.
    Vertical,
}

/// User settings, persisted as JSON in %APPDATA%\lu-battery\settings.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub anchor: Anchor,
    /// Margin from the screen edge in logical px (for corner anchors).
    pub margin: f32,
    /// Top-left position in logical px when `anchor == Free`.
    pub free_pos: [f32; 2],
    /// Index of the monitor to show on (0 = primary).
    pub monitor: usize,
    /// UI scale, 0.75..=2.0.
    pub scale: f32,
    /// Background opacity, 0.0..=1.0 (text stays opaque).
    pub opacity: f32,
    pub layout: Layout,
    pub show_percent: bool,
    pub show_bar: bool,
    pub show_names: bool,
    /// Show percent with one decimal (exact raw value) instead of rounded.
    pub precise_percent: bool,
    /// Mouse clicks pass through the overlay (settings via tray then).
    pub click_through: bool,
    pub always_on_top: bool,
    /// Seconds between battery reads, 5..=300.
    pub poll_secs: u32,
    /// Below this percent the device is shown as low (warning colour).
    pub low_threshold: u8,
    /// Below this percent it is critical (danger colour, pulse).
    pub critical_threshold: u8,
    /// Device ids the user hid from the overlay.
    pub hidden_devices: Vec<String>,
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            anchor: Anchor::TopRight,
            margin: 16.0,
            free_pos: [100.0, 100.0],
            monitor: 0,
            scale: 1.0,
            opacity: 0.85,
            layout: Layout::Horizontal,
            show_percent: true,
            show_bar: true,
            show_names: false,
            precise_percent: false,
            click_through: false,
            always_on_top: true,
            poll_secs: 30,
            low_threshold: 20,
            critical_threshold: 10,
            hidden_devices: Vec::new(),
            autostart: false,
        }
    }
}

/// Things the UI asks the app shell to do.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// User pressed on the overlay background: start a window drag
    /// (the app switches `anchor` to `Free` when the drag ends).
    StartDrag,
    OpenSettings,
    CloseSettings,
    /// Read all devices now.
    RefreshNow,
    /// Settings were changed in the settings window; the app persists and applies them.
    SettingsChanged,
    Quit,
}
