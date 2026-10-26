//! Tray icon with menu and battery tooltip. Owned by `code`.

use crate::model::{Charging, DeviceStatus, Link, ReadSource};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// What the user picked in the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    OpenSettings,
    RefreshNow,
    /// "Overlay anzeigen" was clicked; see [`Tray::overlay_checked`].
    ToggleOverlay,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    show_item: CheckMenuItem,
    commands: Receiver<TrayCommand>,
    tooltip: String,
}

impl Tray {
    /// Creates the tray icon. Must run on the thread with the event loop.
    pub fn new(ctx: &egui::Context) -> anyhow::Result<Self> {
        let settings = MenuItem::new("Einstellungen", true, None);
        let refresh = MenuItem::new("Jetzt aktualisieren", true, None);
        let show = CheckMenuItem::new("Overlay anzeigen", true, true, None);
        let quit = MenuItem::new("Beenden", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &settings,
            &refresh,
            &show,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let (tx, rx) = mpsc::channel();
        {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            let ids = (
                settings.id().clone(),
                refresh.id().clone(),
                show.id().clone(),
                quit.id().clone(),
            );
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let cmd = if e.id == ids.0 {
                    TrayCommand::OpenSettings
                } else if e.id == ids.1 {
                    TrayCommand::RefreshNow
                } else if e.id == ids.2 {
                    TrayCommand::ToggleOverlay
                } else if e.id == ids.3 {
                    TrayCommand::Quit
                } else {
                    return;
                };
                let _ = tx.send(cmd);
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = e
                {
                    let _ = tx.send(TrayCommand::OpenSettings);
                    ctx.request_repaint();
                }
            }));
        }

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("lu-battery")
            .with_icon(tray_icon()?)
            .build()?;
        Ok(Self {
            icon,
            show_item: show,
            commands: rx,
            tooltip: String::new(),
        })
    }

    pub fn try_recv(&self) -> Option<TrayCommand> {
        self.commands.try_recv().ok()
    }

    /// State of the "Overlay anzeigen" check mark (toggled by the menu itself).
    pub fn overlay_checked(&self) -> bool {
        self.show_item.is_checked()
    }

    /// Keeps the "Overlay anzeigen" check mark in sync.
    pub fn set_overlay_visible(&self, visible: bool) {
        self.show_item.set_checked(visible);
    }

    pub fn update_tooltip(&mut self, devices: &[DeviceStatus], precise: bool) {
        let text = tooltip_text(devices, precise);
        if text != self.tooltip {
            if let Err(e) = self.icon.set_tooltip(Some(&text)) {
                log::warn!("tray tooltip: {e}");
            }
            self.tooltip = text;
        }
    }
}

/// One line per device, e.g. "Basilisk V3 Pro: 69 % (lädt)".
pub fn tooltip_text(devices: &[DeviceStatus], precise: bool) -> String {
    let mut lines = vec!["lu-battery".to_string()];
    for d in devices {
        let value = match (&d.link, d.battery) {
            (Link::Asleep, _) => "schläft".to_string(),
            (link, Some(b)) => {
                let pct = if precise {
                    format!("{:.1} %", b.percent())
                } else {
                    format!("{:.0} %", b.percent())
                };
                let state = match b.charging {
                    Charging::Charging => " (lädt)",
                    Charging::Full => " (voll)",
                    _ => "",
                };
                let stale = if matches!(link, Link::Error(_)) {
                    " (veraltet)"
                } else {
                    ""
                };
                let source = match b.source {
                    ReadSource::Synapse => " (Synapse)",
                    ReadSource::Hid => "",
                };
                format!("{pct}{state}{source}{stale}")
            }
            (Link::Error(msg), None) => msg.clone(),
            (Link::Online, None) => "–".to_string(),
        };
        lines.push(format!("{}: {value}", d.name));
    }
    if devices.is_empty() {
        lines.push("keine Geräte gefunden".into());
    }
    // Windows cuts tooltips at 127 characters.
    let mut text = lines.join("\n");
    if text.chars().count() > 127 {
        text = text.chars().take(126).collect::<String>() + "…";
    }
    text
}

// ---------------------------------------------------------------------------
// Icons

/// Resource id of the icon embedded by build.rs (from assets/icon.ico).
#[cfg(lu_icon_ico)]
const ICON_RESOURCE_ID: u16 = 1;

fn tray_icon() -> anyhow::Result<Icon> {
    #[cfg(all(windows, lu_icon_ico))]
    if let Ok(icon) = Icon::from_resource(ICON_RESOURCE_ID, None) {
        return Ok(icon);
    }
    let (rgba, w, h) = app_icon_rgba();
    Ok(Icon::from_rgba(rgba, w, h)?)
}

/// The app icon as RGBA: assets/icon.png when it exists at build time,
/// otherwise a small drawn battery.
pub fn app_icon_rgba() -> (Vec<u8>, u32, u32) {
    #[cfg(lu_icon_png)]
    {
        if let Some(icon) = decode_png(include_bytes!("../assets/icon.png")) {
            return icon;
        }
    }
    fallback_icon(32)
}

#[cfg(lu_icon_png)]
fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, info.width, info.height))
}

/// A battery in the Moon palette: lavender outline, mint fill.
fn fallback_icon(size: u32) -> (Vec<u8>, u32, u32) {
    const OUTLINE: [u8; 4] = [0xd6, 0xcf, 0xfd, 0xff];
    const FILL: [u8; 4] = [0x7f, 0xe3, 0xc6, 0xff];
    let s = size as f32 / 32.0;
    let px = |v: f32| (v * s).round() as i32;
    let (l, r, t, b) = (px(3.0), px(26.0), px(9.0), px(23.0));
    let (nub_r, nub_t, nub_b) = (px(29.0), px(13.0), px(19.0));
    let stroke = px(2.0).max(1);
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let body = x >= l && x < r && y >= t && y < b;
            let inner = x >= l + stroke && x < r - stroke && y >= t + stroke && y < b - stroke;
            let fill =
                x >= l + 2 * stroke && x < px(19.0) && y >= t + 2 * stroke && y < b - 2 * stroke;
            let nub = x >= r && x < nub_r && y >= nub_t && y < nub_b;
            let color = if fill {
                Some(FILL)
            } else if (body && !inner) || nub {
                Some(OUTLINE)
            } else {
                None
            };
            if let Some(c) = color {
                let i = ((y as u32 * size + x as u32) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&c);
            }
        }
    }
    (rgba, size, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BatteryReading, DeviceKind};
    use std::time::Instant;

    fn dev(name: &str, link: Link, raw: Option<u16>, charging: Charging) -> DeviceStatus {
        DeviceStatus {
            id: name.into(),
            name: name.into(),
            kind: DeviceKind::Mouse,
            link,
            battery: raw.map(|raw| BatteryReading {
                source: ReadSource::Hid,
                raw,
                raw_max: 255,
                charging,
                read_at: Instant::now(),
            }),
        }
    }

    #[test]
    fn tooltip_lines() {
        let t = tooltip_text(
            &[
                dev("Maus", Link::Online, Some(176), Charging::Charging),
                dev("Headset", Link::Asleep, Some(10), Charging::Unknown),
                dev(
                    "X",
                    Link::Error("kaputt".into()),
                    Some(255),
                    Charging::Discharging,
                ),
            ],
            true,
        );
        assert_eq!(
            t,
            "lu-battery\nMaus: 69.0 % (lädt)\nHeadset: schläft\nX: 100.0 % (veraltet)"
        );
    }

    #[test]
    fn fallback_icon_has_pixels() {
        let (rgba, w, h) = fallback_icon(32);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
    }
}
