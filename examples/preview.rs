//! Design preview with fake devices: `cargo run --example preview`.
//!
//! Left: overlay variants on a fake desktop (all states at once).
//! Right: the settings panel; changes apply live to the variants.

#[allow(dead_code)]
#[path = "../src/model.rs"]
mod model;
#[allow(dead_code)]
#[path = "../src/ui/mod.rs"]
mod ui;

use eframe::egui::{self, Color32, CornerRadius, Mesh, Pos2, Rect, RichText, pos2, vec2};
use model::{BatteryReading, Charging, DeviceKind, DeviceStatus, Layout, Link, Settings, UiAction};
use std::time::Instant;

fn reading(raw: u16, raw_max: u16, charging: Charging) -> Option<BatteryReading> {
    Some(BatteryReading {
        raw,
        raw_max,
        charging,
        read_at: Instant::now(),
    })
}

fn dev(
    id: &str,
    name: &str,
    kind: DeviceKind,
    link: Link,
    b: Option<BatteryReading>,
) -> DeviceStatus {
    DeviceStatus {
        id: id.into(),
        name: name.into(),
        kind,
        link,
        battery: b,
    }
}

/// The user's two devices in normal use.
fn main_pair() -> Vec<DeviceStatus> {
    vec![
        dev(
            "1532:00ab",
            "Basilisk V3 Pro",
            DeviceKind::Mouse,
            Link::Online,
            reading(223, 255, Charging::Discharging),
        ),
        dev(
            "1532:0550",
            "Barracuda X",
            DeviceKind::Headset,
            Link::Online,
            reading(42, 100, Charging::Charging),
        ),
    ]
}

/// One device per state.
fn all_states() -> Vec<DeviceStatus> {
    vec![
        dev(
            "a",
            "Basilisk V3 Pro",
            DeviceKind::Mouse,
            Link::Online,
            reading(223, 255, Charging::Discharging),
        ),
        dev(
            "b",
            "Barracuda X",
            DeviceKind::Headset,
            Link::Online,
            reading(42, 100, Charging::Charging),
        ),
        dev(
            "c",
            "Niedrig",
            DeviceKind::Mouse,
            Link::Online,
            reading(46, 255, Charging::Discharging),
        ),
        dev(
            "d",
            "Kritisch",
            DeviceKind::Headset,
            Link::Online,
            reading(7, 100, Charging::Discharging),
        ),
        dev(
            "e",
            "Voll",
            DeviceKind::Mouse,
            Link::Online,
            reading(255, 255, Charging::Full),
        ),
        dev(
            "f",
            "Schläft",
            DeviceKind::Headset,
            Link::Asleep,
            reading(80, 100, Charging::Discharging),
        ),
        dev(
            "g",
            "Veraltet",
            DeviceKind::Mouse,
            Link::Error("Timeout".into()),
            reading(163, 255, Charging::Discharging),
        ),
        dev(
            "h",
            "Noch kein Wert",
            DeviceKind::Keyboard,
            Link::Online,
            None,
        ),
    ]
}

struct Preview {
    settings: Settings,
    devices: Vec<DeviceStatus>,
    states: Vec<DeviceStatus>,
    log: Vec<String>,
    frames: u32,
}

/// Writes an RGBA image as a 32-bit BMP (no extra dependency needed).
fn write_bmp(path: &str, img: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let data_len = (w * h * 4) as u32;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for px in &img.pixels {
        let [r, g, b, a] = px.to_srgba_unmultiplied();
        out.extend_from_slice(&[b, g, r, a]);
    }
    std::fs::write(path, out)
}

/// Soft night-sky gradient so the transparency is visible.
fn desktop(ui: &egui::Ui, rect: Rect) {
    let mut mesh = Mesh::default();
    let top = Color32::from_rgb(0x2a, 0x3d, 0x6e);
    let bottom = Color32::from_rgb(0xc9, 0x8b, 0x8f);
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), Color32::from_rgb(0x4b, 0x3a, 0x7a));
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), Color32::from_rgb(0x8a, 0x6f, 0x9e));
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    ui.painter().add(mesh);
    // A few "windows" behind the overlay.
    for (k, r) in [
        Rect::from_min_size(rect.min + vec2(260.0, 40.0), vec2(220.0, 140.0)),
        Rect::from_min_size(rect.min + vec2(30.0, 420.0), vec2(300.0, 160.0)),
    ]
    .into_iter()
    .enumerate()
    {
        let c = if k == 0 {
            Color32::from_rgb(0xee, 0xee, 0xf2)
        } else {
            Color32::from_rgb(0x20, 0x22, 0x28)
        };
        ui.painter().rect_filled(r, CornerRadius::same(6), c);
    }
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .color(Color32::from_white_alpha(200)),
    );
}

impl eframe::App for Preview {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::apply_theme(root.ctx(), &self.settings);
        // `LU_PREVIEW_SHOT=out.bmp` saves one rendered frame and exits.
        self.frames += 1;
        if let Ok(path) = std::env::var("LU_PREVIEW_SHOT") {
            if self.frames == 5 {
                root.ctx().request_screenshot(move |img| {
                    let res = write_bmp(&path, &img);
                    eprintln!("screenshot {path}: {res:?}");
                    std::process::exit(0);
                });
            }
            root.ctx().request_repaint();
        }
        let full = root.max_rect();
        let left = Rect::from_min_max(full.min, pos2(full.min.x + 560.0, full.max.y));
        let right = Rect::from_min_max(pos2(left.max.x, full.min.y), full.max);
        root.painter()
            .rect_filled(full, CornerRadius::ZERO, ui::theme::BG);

        let mut actions: Vec<UiAction> = Vec::new();
        root.scope_builder(egui::UiBuilder::new().max_rect(left.shrink(0.0)), |ui| {
            desktop(ui, left);
            ui.scope_builder(egui::UiBuilder::new().max_rect(left.shrink(18.0)), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let s = &self.settings;
                caption(ui, "Deine Geräte (aktuelle Einstellungen)");
                ui.push_id("v1", |ui| ui::overlay(ui, &self.devices, s, &mut actions));
                ui.add_space(8.0);

                caption(ui, "Mit Namen, präzise");
                let named = Settings {
                    show_names: true,
                    precise_percent: true,
                    ..s.clone()
                };
                ui.push_id("v2", |ui| {
                    ui::overlay(ui, &self.devices, &named, &mut actions)
                });
                ui.add_space(8.0);

                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        caption(ui, "Alle Zustände, untereinander");
                        let vert = Settings {
                            layout: Layout::Vertical,
                            show_names: true,
                            ..s.clone()
                        };
                        ui.push_id("v3", |ui| {
                            ui::overlay(ui, &self.states, &vert, &mut actions)
                        });
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        caption(ui, "Kompakt (nur Prozent)");
                        let compact = Settings {
                            show_bar: false,
                            ..s.clone()
                        };
                        ui.push_id("v4", |ui| {
                            ui::overlay(ui, &self.devices, &compact, &mut actions)
                        });
                        ui.add_space(8.0);
                        caption(ui, "Nur Balken, 30 % Deckkraft");
                        let bar = Settings {
                            show_percent: false,
                            opacity: 0.3,
                            ..s.clone()
                        };
                        ui.push_id("v5", |ui| {
                            ui::overlay(ui, &self.devices, &bar, &mut actions)
                        });
                        ui.add_space(8.0);
                        caption(ui, "Keine Geräte");
                        ui.push_id("v7", |ui| ui::overlay(ui, &[], s, &mut actions));
                        ui.add_space(8.0);
                        caption(ui, "Rechtsklick-Menü");
                        ui.push_id("v8", |ui| {
                            // Keep the inline menu open for the preview.
                            let id = ui.scope_id().with("lu_overlay_menu");
                            ui.data_mut(|d| d.insert_temp(id, true));
                            ui::overlay(ui, &self.devices, s, &mut actions)
                        });
                    });
                });
                ui.add_space(12.0);
                caption(ui, "Größe 150 %");
                let big = Settings {
                    scale: 1.5,
                    ..s.clone()
                };
                ui.push_id("v6", |ui| {
                    ui::overlay(ui, &self.devices, &big, &mut actions)
                });
            });
        });

        // `LU_PREVIEW_SETTINGS_OFFSET=700` shifts the settings up (for screenshots of the lower part).
        let offset: f32 = std::env::var("LU_PREVIEW_SETTINGS_OFFSET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let right = if offset > 0.0 {
            Rect::from_min_max(right.min - vec2(0.0, offset), right.max)
        } else {
            right
        };
        root.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            ui::settings_panel(ui, &mut self.settings, &self.devices, &mut actions);
        });

        for a in actions {
            if a != UiAction::SettingsChanged || self.log.last() != Some(&format!("{a:?}")) {
                self.log.push(format!("{a:?}"));
            }
        }
        if !self.log.is_empty() {
            let text = self.log[self.log.len().saturating_sub(4)..].join("  ·  ");
            root.painter().text(
                Pos2::new(left.left() + 18.0, left.bottom() - 18.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::monospace(11.0),
                Color32::from_white_alpha(220),
            );
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lu-battery – Design-Vorschau")
            .with_inner_size([1160.0, 860.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lu-battery preview",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Preview {
                settings: Settings::default(),
                devices: main_pair(),
                states: all_states(),
                log: Vec::new(),
                frames: 0,
            }))
        }),
    )
}
