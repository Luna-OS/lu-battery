//! The lu-minal "Moon" palette and the egui visuals built from it.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Visuals,
};

/// Builds a colour from `0xrrggbb`.
const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Linear mix of two opaque colours, `t = 0` gives `a`, `t = 1` gives `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Same colour with an alpha of `a` (0..=1), unmultiplied.
pub fn alpha(c: Color32, a: f32) -> Color32 {
    let a = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub const BG: Color32 = hex(0x0b0920);
pub const SURFACE: Color32 = hex(0x141030);
pub const SURFACE_FOCUS: Color32 = hex(0x1d1742);
pub const POPOVER: Color32 = hex(0x181338);
pub const TEXT: Color32 = hex(0xfbf7f0);
pub const ACCENT: Color32 = hex(0xd6cffd);
pub const ACCENT2: Color32 = hex(0x9ad7f5);
pub const SUCCESS: Color32 = hex(0x7fe3c6);
pub const WARNING: Color32 = hex(0xf3c766);
pub const ERROR: Color32 = hex(0xe5626b);
pub const SELECTION: Color32 = hex(0x3b2e6b);
pub const LAVENDER400: Color32 = hex(0xb9aefb);
pub const LAVENDER600: Color32 = hex(0x8f82e0);
pub const OUTLINE: Color32 = hex(0x5b4f9e);

/// surface + 18 % lavender400.
pub fn border() -> Color32 {
    mix(SURFACE, LAVENDER400, 0.18)
}

/// surface + 62 % text.
pub fn muted() -> Color32 {
    mix(SURFACE, TEXT, 0.62)
}

/// surface + 40 % text.
pub fn faint() -> Color32 {
    mix(SURFACE, TEXT, 0.40)
}

pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let radius = CornerRadius::same(6);

    v.override_text_color = None;
    v.weak_text_color = Some(faint());
    v.hyperlink_color = ACCENT2;
    v.faint_bg_color = SURFACE;
    v.extreme_bg_color = BG;
    v.text_edit_bg_color = Some(BG);
    v.code_bg_color = SURFACE_FOCUS;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.panel_fill = BG;
    v.window_fill = POPOVER;
    v.window_stroke = Stroke::new(1.0, border());
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: alpha(Color32::BLACK, 0.45),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: alpha(Color32::BLACK, 0.40),
    };
    v.selection.bg_fill = SELECTION;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, border());
    w.noninteractive.fg_stroke = Stroke::new(1.0, muted());
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = SURFACE_FOCUS;
    w.inactive.weak_bg_fill = SURFACE_FOCUS;
    w.inactive.bg_stroke = Stroke::new(1.0, border());
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = mix(SURFACE_FOCUS, SELECTION, 0.6);
    w.hovered.weak_bg_fill = mix(SURFACE_FOCUS, SELECTION, 0.6);
    w.hovered.bg_stroke = Stroke::new(1.0, OUTLINE);
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.corner_radius = radius;

    w.active.bg_fill = SELECTION;
    w.active.weak_bg_fill = SELECTION;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(2.0, TEXT);
    w.active.corner_radius = radius;

    w.open.bg_fill = SURFACE_FOCUS;
    w.open.weak_bg_fill = SURFACE_FOCUS;
    w.open.bg_stroke = Stroke::new(1.0, OUTLINE);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;

    v
}

pub fn style(style: &mut egui::Style) {
    style.visuals = visuals();
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 7.0);
    s.button_padding = egui::vec2(10.0, 4.0);
    s.interact_size.y = 24.0;
    s.slider_width = 180.0;
    s.window_margin = egui::Margin::same(12);
    s.menu_margin = egui::Margin::same(6);
}
