//! Overlay and settings UI in the lu-minal "Moon" theme. Owned by `design`.
//! The three public functions are the fixed contract from CONCEPT.md.

pub mod theme;
pub mod widgets;

use crate::model::{
    Anchor, BatteryReading, Charging, DeviceKind, DeviceStatus, Layout, Link, Settings, UiAction,
};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Painter, PointerButton, Rect, RichText, Sense,
    Stroke, StrokeKind, Vec2, pos2, vec2,
};
use std::time::Duration;
use theme::{alpha, mix};

/// Installs the Moon visuals and style. Cheap to call every frame: the style is
/// only rebuilt once per context. `settings.scale` is applied by [`overlay`]
/// itself (it must not grow the settings window), so it is not used here.
pub fn apply_theme(ctx: &egui::Context, _settings: &Settings) {
    let id = Id::unique("lu_battery_theme_applied");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(theme::style);
    ctx.data_mut(|d| d.insert_temp(id, true));
}

/// Base metrics of the overlay at scale 1.0, in logical px.
struct Metrics {
    pad: f32,
    gap: f32,
    cell_h: f32,
    cell_pad: f32,
    inner_gap: f32,
    icon: f32,
    bar: Vec2,
    font: f32,
    name_font: f32,
    radius: f32,
    cell_radius: f32,
}

impl Metrics {
    fn new(scale: f32) -> Self {
        let s = scale.clamp(0.75, 2.0);
        Self {
            pad: 4.0 * s,
            gap: 4.0 * s,
            cell_h: 30.0 * s,
            cell_pad: 9.0 * s,
            inner_gap: 7.0 * s,
            icon: 17.0 * s,
            bar: vec2(30.0 * s, 7.0 * s),
            font: 14.0 * s,
            name_font: 12.5 * s,
            radius: 12.0 * s,
            cell_radius: 8.0 * s,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Ok,
    Low,
    Critical,
}

/// Everything needed to paint one device cell.
struct Cell<'a> {
    device: &'a DeviceStatus,
    text: String,
    /// Width reserved for `text`, so the window does not jitter between values.
    text_w: f32,
    name_w: f32,
    level: Option<f32>,
    state: Level,
    charging: Option<Charging>,
    asleep: bool,
    stale: bool,
}

fn format_percent(b: &BatteryReading, precise: bool) -> String {
    if precise {
        format!("{:.1}%", b.percent()).replace('.', ",")
    } else {
        format!("{:.0}%", b.percent())
    }
}

fn text_width(p: &Painter, text: &str, font: FontId) -> f32 {
    p.layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

fn build_cell<'a>(p: &Painter, d: &'a DeviceStatus, settings: &Settings, m: &Metrics) -> Cell<'a> {
    let font = FontId::proportional(m.font);
    let asleep = d.link == Link::Asleep;
    let stale = matches!(d.link, Link::Error(_));
    let text = if asleep {
        "schläft".to_owned()
    } else {
        d.battery
            .as_ref()
            .map(|b| format_percent(b, settings.precise_percent))
            .unwrap_or_else(|| "–".to_owned())
    };
    let widest = if settings.precise_percent {
        "100,0%"
    } else {
        "100%"
    };
    let text_w = text_width(p, widest, font.clone()).max(text_width(p, &text, font));
    let name_w = text_width(p, &d.name, FontId::proportional(m.name_font));
    let pct = d.battery.as_ref().map(|b| b.percent());
    let state = match pct {
        Some(v) if v <= settings.critical_threshold as f32 => Level::Critical,
        Some(v) if v <= settings.low_threshold as f32 => Level::Low,
        _ => Level::Ok,
    };
    Cell {
        device: d,
        text,
        text_w,
        name_w,
        level: if asleep { None } else { pct.map(|v| v / 100.0) },
        state,
        charging: d.battery.as_ref().map(|b| b.charging),
        asleep,
        stale,
    }
}

impl Cell<'_> {
    fn width(&self, settings: &Settings, m: &Metrics) -> f32 {
        let mut w = m.cell_pad * 2.0 + m.icon;
        if settings.show_names {
            w += m.inner_gap + self.name_w;
        }
        if settings.show_bar {
            w += m.inner_gap + m.bar.x;
        }
        if settings.show_percent {
            w += m.inner_gap * 0.8 + self.text_w;
        }
        w
    }

    fn is_charging(&self) -> bool {
        !self.asleep && matches!(self.charging, Some(Charging::Charging))
    }

    /// Colour of bar / percent text.
    fn color(&self) -> Color32 {
        let base = if self.is_charging() {
            theme::ACCENT2
        } else {
            match self.state {
                Level::Ok => theme::SUCCESS,
                Level::Low => theme::WARNING,
                Level::Critical => theme::ERROR,
            }
        };
        if self.stale {
            mix(base, theme::SURFACE, 0.5)
        } else {
            base
        }
    }

    fn pulses(&self) -> bool {
        self.state == Level::Critical && !self.stale && !self.asleep && !self.is_charging()
    }
}

/// 0..=1, slow sine for the critical state.
fn pulse(ui: &egui::Ui) -> f32 {
    let t = ui.input(|i| i.time) as f32;
    (t * std::f32::consts::TAU / 1.6).sin() * 0.5 + 0.5
}

fn paint_cell(p: &Painter, rect: Rect, cell: &Cell, settings: &Settings, m: &Metrics, pulse: f32) {
    let bg_a = settings.opacity.clamp(0.0, 1.0);
    let crit = cell.pulses();
    let fill = if crit {
        mix(theme::SURFACE, theme::ERROR, 0.10 + 0.10 * pulse)
    } else {
        theme::SURFACE
    };
    let stroke = if crit {
        Stroke::new(1.0, alpha(theme::ERROR, 0.30 + 0.55 * pulse))
    } else {
        Stroke::NONE
    };
    p.rect(
        rect,
        CornerRadius::same(m.cell_radius as u8),
        alpha(fill, bg_a),
        stroke,
        StrokeKind::Inside,
    );

    let dim = cell.asleep || cell.stale;
    let color = cell.color();
    let cy = rect.center().y;

    // Icon.
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + m.cell_pad + m.icon / 2.0, cy),
        Vec2::splat(m.icon),
    );
    let icon_color = if dim {
        theme::faint()
    } else if cell.state != Level::Ok && !cell.is_charging() {
        color
    } else {
        theme::ACCENT
    };
    widgets::device_icon(p, icon_rect, cell.device.kind, icon_color);

    // Badges on the icon: charging bolt, full check, asleep moon, stale dot.
    let ring = mix(theme::BG, fill, bg_a);
    let br = m.icon * 0.22;
    let badge_at = icon_rect.right_bottom() + vec2(-br * 0.2, -br * 0.4);
    if cell.asleep {
        let r = Rect::from_center_size(badge_at, Vec2::splat(br * 3.0));
        p.circle_filled(badge_at, br * 1.45, ring);
        widgets::moon_icon(p, r, theme::ACCENT, ring);
    } else if cell.is_charging() || cell.charging == Some(Charging::Full) {
        let c = if cell.is_charging() {
            theme::ACCENT2
        } else {
            theme::SUCCESS
        };
        p.circle_filled(badge_at, br * 1.4, ring);
        let b = Rect::from_center_size(badge_at, vec2(br * 1.5, br * 2.3));
        widgets::bolt(
            p,
            b,
            if cell.stale {
                mix(c, theme::SURFACE, 0.5)
            } else {
                c
            },
        );
    }
    if cell.stale {
        let at = icon_rect.right_top() + vec2(-br * 0.2, br * 0.4);
        widgets::badge(p, at, br * 0.8, ring, theme::WARNING);
    }

    let mut x = icon_rect.right() + m.inner_gap;
    if settings.show_names {
        let c = if dim { theme::faint() } else { theme::muted() };
        p.text(
            pos2(x, cy),
            Align2::LEFT_CENTER,
            &cell.device.name,
            FontId::proportional(m.name_font),
            c,
        );
    }

    // Bar and percent are right-aligned, so vertical stacks line up.
    let mut right = rect.right() - m.cell_pad;
    if settings.show_percent {
        let c = if dim {
            theme::faint()
        } else if cell.level.is_none() {
            theme::muted()
        } else {
            color
        };
        p.text(
            pos2(right, cy),
            Align2::RIGHT_CENTER,
            &cell.text,
            FontId::proportional(m.font),
            c,
        );
        right -= cell.text_w + m.inner_gap * 0.8;
    }
    if settings.show_bar {
        x = right - m.bar.x;
        let bar = Rect::from_min_size(pos2(x, cy - m.bar.y / 2.0), m.bar);
        let c = if crit {
            alpha(color, 0.6 + 0.4 * pulse)
        } else {
            color
        };
        widgets::battery_bar(p, bar, cell.level, c);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MenuItem {
    Settings,
    Refresh,
    Quit,
}

impl MenuItem {
    const ALL: [MenuItem; 3] = [MenuItem::Settings, MenuItem::Refresh, MenuItem::Quit];

    fn label(self) -> &'static str {
        match self {
            MenuItem::Settings => "Einstellungen",
            MenuItem::Refresh => "Aktualisieren",
            MenuItem::Quit => "Beenden",
        }
    }

    fn action(self) -> UiAction {
        match self {
            MenuItem::Settings => UiAction::OpenSettings,
            MenuItem::Refresh => UiAction::RefreshNow,
            MenuItem::Quit => UiAction::Quit,
        }
    }

    fn paint_icon(self, p: &Painter, rect: Rect, color: Color32) {
        match self {
            MenuItem::Settings => widgets::gear_icon(p, rect, color),
            MenuItem::Refresh => widgets::refresh_icon(p, rect, color),
            MenuItem::Quit => widgets::power_icon(p, rect, color),
        }
    }
}

/// The right-click menu, drawn inside the overlay (a popup would be clipped
/// by the small overlay window). Returns false when it should close.
fn inline_menu(
    ui: &mut egui::Ui,
    rect: Rect,
    m: &Metrics,
    settings: &Settings,
    actions: &mut Vec<UiAction>,
) -> bool {
    let p = ui.painter_at(rect);
    let inner = rect.shrink(m.pad);
    p.rect(
        inner,
        CornerRadius::same(m.cell_radius as u8),
        alpha(theme::POPOVER, settings.opacity.max(0.94)),
        Stroke::new(1.0, theme::OUTLINE),
        StrokeKind::Inside,
    );
    let font = FontId::proportional(m.name_font);
    let icon = m.icon * 0.95;
    let row_h = m.cell_h * 0.9;
    let stacked = inner.height() >= row_h * 3.0;
    let n = MenuItem::ALL.len() as f32;
    let mut open = true;
    for (i, item) in MenuItem::ALL.into_iter().enumerate() {
        let slot = if stacked {
            let h = inner.height() / n;
            Rect::from_min_size(inner.min + vec2(0.0, h * i as f32), vec2(inner.width(), h))
        } else {
            let w = inner.width() / n;
            Rect::from_min_size(inner.min + vec2(w * i as f32, 0.0), vec2(w, inner.height()))
        }
        .shrink(m.pad * 0.6);
        let resp = ui.interact(
            slot,
            ui.scope_id().with(("lu_menu_item", i)),
            Sense::click(),
        );
        let hot = resp.hovered();
        if hot {
            p.rect_filled(
                slot,
                CornerRadius::same((m.cell_radius * 0.75) as u8),
                theme::SELECTION,
            );
        }
        let color = match (item, hot) {
            (MenuItem::Quit, true) => theme::ERROR,
            (_, true) => theme::TEXT,
            _ => theme::ACCENT,
        };
        let label_w = text_width(&p, item.label(), font.clone());
        let with_label = icon + m.inner_gap * 0.7 + label_w <= slot.width() - m.cell_pad;
        let content_w = if with_label {
            icon + m.inner_gap * 0.7 + label_w
        } else {
            icon
        };
        let left = if stacked {
            slot.left() + m.cell_pad * 0.8
        } else {
            slot.center().x - content_w / 2.0
        };
        let icon_rect =
            Rect::from_min_size(pos2(left, slot.center().y - icon / 2.0), Vec2::splat(icon));
        item.paint_icon(&p, icon_rect, color);
        if with_label {
            p.text(
                pos2(icon_rect.right() + m.inner_gap * 0.7, slot.center().y),
                Align2::LEFT_CENTER,
                item.label(),
                font.clone(),
                if hot { theme::TEXT } else { theme::muted() },
            );
        }
        if resp.clicked() {
            actions.push(item.action());
            open = false;
        }
    }
    let leave = !ui.rect_contains_pointer(rect) || ui.input(|i| i.key_pressed(egui::Key::Escape));
    open && !leave
}

/// Draws the overlay into the (frameless, transparent) central area of the
/// main window. Returns the size the window needs for it, in logical px.
pub fn overlay(
    ui: &mut egui::Ui,
    devices: &[DeviceStatus],
    settings: &Settings,
    actions: &mut Vec<UiAction>,
) -> Vec2 {
    let m = Metrics::new(settings.scale);
    let measure = ui.painter().clone();
    let cells: Vec<Cell> = devices
        .iter()
        .filter(|d| !settings.hidden_devices.contains(&d.id))
        .map(|d| build_cell(&measure, d, settings, &m))
        .collect();

    // Sizes.
    let widths: Vec<f32> = cells.iter().map(|c| c.width(settings, &m)).collect();
    let empty_text = if devices.is_empty() {
        "Keine Razer-Geräte gefunden"
    } else {
        "Alle Geräte ausgeblendet"
    };
    let content = if cells.is_empty() {
        let w = text_width(&measure, empty_text, FontId::proportional(m.name_font));
        vec2(m.cell_pad * 2.0 + m.icon + m.inner_gap + w, m.cell_h)
    } else {
        let n = cells.len() as f32;
        match settings.layout {
            Layout::Horizontal => vec2(widths.iter().sum::<f32>() + m.gap * (n - 1.0), m.cell_h),
            Layout::Vertical => vec2(
                widths.iter().cloned().fold(0.0, f32::max),
                m.cell_h * n + m.gap * (n - 1.0),
            ),
        }
    };
    let size = (content + Vec2::splat(m.pad * 2.0)).ceil();

    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let p = ui.painter_at(rect);
    let bg_a = settings.opacity.clamp(0.0, 1.0);
    p.rect(
        rect,
        CornerRadius::same(m.radius as u8),
        alpha(theme::BG, bg_a),
        Stroke::new(1.0, alpha(theme::border(), bg_a)),
        StrokeKind::Inside,
    );

    let menu_id = ui.scope_id().with("lu_overlay_menu");
    let mut menu_open = ui.data(|d| d.get_temp::<bool>(menu_id)).unwrap_or(false);
    if resp.secondary_clicked() {
        menu_open = !menu_open;
    }

    if cells.is_empty() {
        let icon = Rect::from_center_size(
            pos2(
                rect.left() + m.pad + m.cell_pad + m.icon / 2.0,
                rect.center().y,
            ),
            Vec2::splat(m.icon),
        );
        widgets::moon_icon(&p, icon, theme::faint(), alpha(theme::BG, bg_a));
        p.text(
            pos2(icon.right() + m.inner_gap, rect.center().y),
            Align2::LEFT_CENTER,
            empty_text,
            FontId::proportional(m.name_font),
            theme::muted(),
        );
    } else if !menu_open {
        let pulse_v = pulse(ui);
        let mut cursor = rect.min + Vec2::splat(m.pad);
        let col_w = content.x;
        for (cell, w) in cells.iter().zip(&widths) {
            let cell_rect = match settings.layout {
                Layout::Horizontal => Rect::from_min_size(cursor, vec2(*w, m.cell_h)),
                Layout::Vertical => Rect::from_min_size(cursor, vec2(col_w, m.cell_h)),
            };
            paint_cell(&p, cell_rect, cell, settings, &m, pulse_v);
            match settings.layout {
                Layout::Horizontal => cursor.x += w + m.gap,
                Layout::Vertical => cursor.y += m.cell_h + m.gap,
            }
        }
        if cells.iter().any(Cell::pulses) {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    // Interaction.
    if menu_open {
        // A click on the background (between the items) closes it, too.
        menu_open = inline_menu(ui, rect, &m, settings, actions) && !resp.clicked();
        if !menu_open {
            ui.ctx().request_repaint();
        }
    } else if resp.double_clicked_by(PointerButton::Primary) {
        actions.push(UiAction::OpenSettings);
    } else if resp.drag_started_by(PointerButton::Primary) {
        actions.push(UiAction::StartDrag);
    }
    ui.data_mut(|d| d.insert_temp(menu_id, menu_open));

    size
}

// ---------------------------------------------------------------------------
// Settings window

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(11.5)
            .color(theme::ACCENT)
            .strong(),
    );
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::faint()));
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(label).color(theme::muted()));
    add(ui);
    ui.end_row();
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(150.0)
        .spacing(vec2(16.0, 10.0))
        .show(ui, add);
}

fn charging_text(c: Charging) -> &'static str {
    match c {
        Charging::Charging => "lädt",
        Charging::Discharging => "Akku",
        Charging::Full => "voll geladen",
        Charging::Unknown => "unbekannt",
    }
}

fn kind_text(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => "Maus",
        DeviceKind::Headset => "Headset",
        DeviceKind::Keyboard => "Tastatur",
        DeviceKind::Other => "Gerät",
    }
}

fn device_row(ui: &mut egui::Ui, d: &DeviceStatus, settings: &mut Settings) {
    let mut visible = !settings.hidden_devices.contains(&d.id);
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut visible, "")
            .on_hover_text("Im Overlay anzeigen")
            .changed()
        {
            if visible {
                settings.hidden_devices.retain(|id| id != &d.id);
            } else {
                settings.hidden_devices.push(d.id.clone());
            }
        }
        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        widgets::device_icon(
            ui.painter(),
            r,
            d.kind,
            if visible {
                theme::ACCENT
            } else {
                theme::faint()
            },
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(&d.name).color(if visible {
                theme::TEXT
            } else {
                theme::muted()
            }));
            hint(ui, &format!("{} · {}", kind_text(d.kind), d.id));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (link, link_color) = match &d.link {
                Link::Online => ("verbunden".to_owned(), theme::SUCCESS),
                Link::Asleep => ("schläft".to_owned(), theme::muted()),
                Link::Error(e) => (format!("Fehler: {e}"), theme::ERROR),
            };
            let pill = egui::Frame::new()
                .fill(alpha(link_color, 0.14))
                .corner_radius(CornerRadius::same(9))
                .inner_margin(egui::Margin::symmetric(8, 2));
            pill.show(ui, |ui| {
                ui.label(RichText::new(&link).size(12.0).color(link_color));
            })
            .response
            .on_hover_text(&link);
            match &d.battery {
                Some(b) => {
                    let mut t = format!("{}  ·  {}/{}", format_percent(b, true), b.raw, b.raw_max);
                    if b.charging != Charging::Discharging {
                        t = format!("{t}  ·  {}", charging_text(b.charging));
                    }
                    ui.label(RichText::new(t).monospace().color(theme::muted()))
                        .on_hover_text(format!(
                            "Rohwert vom Gerät: {} von {}\nZuletzt gelesen vor {} s",
                            b.raw,
                            b.raw_max,
                            b.read_at.elapsed().as_secs()
                        ));
                }
                None => {
                    ui.label(RichText::new("kein Wert").color(theme::faint()));
                }
            }
        });
    });
}

/// Content of the settings window (the app opens the viewport).
pub fn settings_panel(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    devices: &[DeviceStatus],
    actions: &mut Vec<UiAction>,
) {
    let before = settings.clone();

    let full = ui.available_rect_before_wrap();
    let footer_h = 52.0;
    let body = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - footer_h));
    let footer = Rect::from_min_max(pos2(full.min.x, full.max.y - footer_h), full.max);

    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| settings_body(ui, settings, devices, actions));
            });
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
        ui.painter().line_segment(
            [footer.left_top(), footer.right_top()],
            Stroke::new(1.0, theme::border()),
        );
        ui.painter()
            .rect_filled(footer, CornerRadius::ZERO, theme::BG);
        let inner = footer.shrink2(vec2(16.0, 10.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                let close = egui::Button::new(RichText::new("Schließen").color(theme::BG))
                    .fill(theme::ACCENT)
                    .min_size(vec2(96.0, 28.0));
                if ui.add(close).clicked() {
                    actions.push(UiAction::CloseSettings);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    hint(ui, "Änderungen werden sofort übernommen und gespeichert.");
                });
            },
        );
    });

    if *settings != before {
        actions.push(UiAction::SettingsChanged);
    }
}

fn settings_body(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    devices: &[DeviceStatus],
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
        widgets::moon_icon(ui.painter(), r, theme::ACCENT, theme::BG);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                RichText::new("lu-battery")
                    .size(18.0)
                    .strong()
                    .color(theme::TEXT),
            );
            hint(ui, "Akkustand deiner Razer-Geräte");
        });
    });

    section(ui, "Vorschau", |ui| {
        let mut ignored = Vec::new();
        ui.push_id("lu_preview", |ui| {
            ui.horizontal(|ui| {
                overlay(ui, devices, settings, &mut ignored);
            });
        });
    });

    section(ui, "Position", |ui| {
        ui.horizontal(|ui| {
            widgets::corner_picker(ui, &mut settings.anchor);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let free = settings.anchor == Anchor::Free;
                let btn = egui::Button::selectable(free, "Frei")
                    .frame_when_inactive(true)
                    .min_size(vec2(72.0, 26.0));
                if ui
                    .add(btn)
                    .on_hover_text("Overlay mit der Maus an eine beliebige Stelle ziehen")
                    .clicked()
                {
                    settings.anchor = Anchor::Free;
                }
                hint(
                    ui,
                    if free {
                        "Zum Verschieben das Overlay ziehen."
                    } else {
                        "Ecke wählen oder Overlay ziehen."
                    },
                );
            });
        });
        ui.add_space(8.0);
        grid(ui, "lu_pos", |ui| {
            row(ui, "Abstand zum Rand", |ui| {
                ui.add_enabled(
                    settings.anchor != Anchor::Free,
                    egui::Slider::new(&mut settings.margin, 0.0..=96.0)
                        .step_by(1.0)
                        .fixed_decimals(0)
                        .suffix(" px"),
                );
            });
            row(ui, "Monitor", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut settings.monitor).range(0..=15));
                    hint(ui, "0 = Hauptbildschirm");
                });
            });
        });
    });

    section(ui, "Aussehen", |ui| {
        grid(ui, "lu_look", |ui| {
            row(ui, "Größe", |ui| {
                ui.add(
                    egui::Slider::new(&mut settings.scale, 0.75..=2.0)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
                );
            });
            row(ui, "Hintergrund-Deckkraft", |ui| {
                ui.add(
                    egui::Slider::new(&mut settings.opacity, 0.0..=1.0)
                        .step_by(0.01)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
                );
            });
            row(ui, "Anordnung", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (value, label) in [
                        (Layout::Horizontal, "Nebeneinander"),
                        (Layout::Vertical, "Untereinander"),
                    ] {
                        let btn = egui::Button::selectable(settings.layout == value, label)
                            .frame_when_inactive(true);
                        if ui.add(btn).clicked() {
                            settings.layout = value;
                        }
                    }
                });
            });
            row(ui, "Anzeigen", |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut settings.show_percent, "Prozent");
                    ui.checkbox(&mut settings.show_bar, "Balken");
                    ui.checkbox(&mut settings.show_names, "Namen");
                });
            });
            row(ui, "Genauigkeit", |ui| {
                ui.checkbox(
                    &mut settings.precise_percent,
                    "Eine Nachkommastelle (87,5 %)",
                );
            });
        });
    });

    section(ui, "Verhalten", |ui| {
        grid(ui, "lu_behave", |ui| {
            row(ui, "Fenster", |ui| {
                ui.checkbox(&mut settings.always_on_top, "Immer im Vordergrund");
            });
            row(ui, "", |ui| {
                ui.vertical(|ui| {
                    ui.checkbox(&mut settings.click_through, "Klick-durchlässig");
                    if settings.click_through {
                        ui.label(
                            RichText::new(
                                "Das Overlay reagiert dann nicht mehr auf die Maus. \
                                 Einstellungen öffnest du über das Tray-Symbol.",
                            )
                            .size(12.0)
                            .color(theme::WARNING),
                        );
                    } else {
                        hint(ui, "Mausklicks gehen durch das Overlay hindurch.");
                    }
                });
            });
            row(ui, "Abfrage alle", |ui| {
                ui.add(
                    egui::Slider::new(&mut settings.poll_secs, 5..=300)
                        .logarithmic(true)
                        .suffix(" s"),
                );
            });
            row(ui, "Warnung ab", |ui| {
                if ui
                    .add(egui::Slider::new(&mut settings.low_threshold, 1..=100).suffix(" %"))
                    .changed()
                    && settings.critical_threshold > settings.low_threshold
                {
                    settings.critical_threshold = settings.low_threshold;
                }
            });
            row(ui, "Kritisch ab", |ui| {
                if ui
                    .add(egui::Slider::new(&mut settings.critical_threshold, 0..=100).suffix(" %"))
                    .changed()
                    && settings.critical_threshold > settings.low_threshold
                {
                    settings.low_threshold = settings.critical_threshold;
                }
            });
            row(ui, "Autostart", |ui| {
                ui.checkbox(&mut settings.autostart, "Mit Windows starten");
            });
        });
    });

    section(ui, "Geräte", |ui| {
        if devices.is_empty() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                widgets::moon_icon(ui.painter(), r, theme::faint(), theme::SURFACE);
                ui.label(RichText::new("Keine Razer-Geräte gefunden.").color(theme::muted()));
            });
            hint(ui, "Dongle eingesteckt? Gerät eingeschaltet?");
        }
        for (i, d) in devices.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            device_row(ui, d, settings);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let btn = egui::Button::new("Jetzt aktualisieren").min_size(vec2(0.0, 28.0));
            if ui.add(btn).clicked() {
                actions.push(UiAction::RefreshNow);
            }
            hint(ui, &format!("Automatisch alle {} s", settings.poll_secs));
        });
    });
    ui.add_space(8.0);
}
