//! Hand-drawn glyphs (device icons, bolt, menu icons), the battery bar and the
//! corner picker. Everything is painted, no fonts or image assets.

use super::theme::{self, alpha, mix};
use crate::model::{Anchor, DeviceKind};
use eframe::egui::{
    self, Color32, CornerRadius, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};
use std::f32::consts::{PI, TAU};

/// Points on a circle arc from `a0` to `a1` (radians, 0 = right, clockwise in screen space).
fn arc(center: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = ((a1 - a0).abs() * r / 2.0).clamp(8.0, 48.0) as usize;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Maps a point in the unit square onto `rect`.
fn at(rect: Rect, x: f32, y: f32) -> Pos2 {
    rect.min + vec2(x * rect.width(), y * rect.height())
}

/// Device glyph inside a square `rect`.
pub fn device_icon(p: &Painter, rect: Rect, kind: DeviceKind, color: Color32) {
    let h = rect.height();
    let stroke = Stroke::new((h / 11.0).max(1.0), color);
    let c = rect.center();
    match kind {
        DeviceKind::Mouse => {
            let body = Rect::from_center_size(c, vec2(h * 0.60, h * 0.94));
            p.rect_stroke(
                body,
                CornerRadius::same((body.width() * 0.5) as u8),
                stroke,
                StrokeKind::Middle,
            );
            let split = body.top() + body.height() * 0.40;
            p.line_segment(
                [pos2(body.left(), split), pos2(body.right(), split)],
                stroke,
            );
            p.line_segment([pos2(c.x, body.top()), pos2(c.x, split)], stroke);
            let wheel = Rect::from_center_size(
                pos2(c.x, body.top() + body.height() * 0.22),
                vec2(h * 0.14, h * 0.20),
            );
            p.rect_filled(wheel, CornerRadius::same(4), color);
        }
        DeviceKind::Headset => {
            let r = h * 0.38;
            let hub = pos2(c.x, c.y + h * 0.04);
            p.add(Shape::line(arc(hub, r, PI, TAU), stroke));
            let cup = vec2(h * 0.24, h * 0.40);
            for x in [hub.x - r, hub.x + r] {
                let cup_rect = Rect::from_center_size(pos2(x, hub.y + h * 0.16), cup);
                p.rect_filled(cup_rect, CornerRadius::same((h * 0.09) as u8), color);
            }
        }
        DeviceKind::Keyboard => {
            let body = Rect::from_center_size(c, vec2(h * 0.96, h * 0.60));
            p.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Middle);
            let dot = h * 0.06;
            for row in 0..2 {
                for col in 0..4 {
                    let x = body.left() + body.width() * (0.2 + 0.2 * col as f32);
                    let y = body.top() + body.height() * (0.28 + 0.22 * row as f32);
                    p.circle_filled(pos2(x, y), dot, color);
                }
            }
            let y = body.top() + body.height() * 0.76;
            p.line_segment(
                [
                    pos2(body.left() + body.width() * 0.3, y),
                    pos2(body.left() + body.width() * 0.7, y),
                ],
                stroke,
            );
        }
        DeviceKind::Other => {
            p.circle_stroke(c, h * 0.36, stroke);
            p.circle_filled(c, h * 0.12, color);
        }
    }
}

/// Lightning bolt filling `rect` (taller than wide works best).
pub fn bolt(p: &Painter, rect: Rect, color: Color32) {
    let top = vec![
        at(rect, 0.64, 0.0),
        at(rect, 0.10, 0.60),
        at(rect, 0.54, 0.60),
    ];
    let bottom = vec![
        at(rect, 0.46, 0.40),
        at(rect, 0.90, 0.40),
        at(rect, 0.36, 1.0),
    ];
    p.add(Shape::convex_polygon(top, color, Stroke::NONE));
    p.add(Shape::convex_polygon(bottom, color, Stroke::NONE));
}

/// Small round badge (e.g. charging bolt, stale dot) cut out of the background.
pub fn badge(p: &Painter, center: Pos2, r: f32, ring: Color32, fill: Color32) {
    p.circle_filled(center, r + r * 0.35, ring);
    p.circle_filled(center, r, fill);
}

/// Rounded battery bar. `level` is 0..=1, `None` draws only the track.
pub fn battery_bar(p: &Painter, rect: Rect, level: Option<f32>, color: Color32) {
    let r = CornerRadius::same((rect.height() * 0.5) as u8);
    p.rect_filled(rect, r, alpha(theme::TEXT, 0.10));
    if let Some(level) = level {
        let level = level.clamp(0.0, 1.0);
        if level > 0.0 {
            let w = (rect.width() * level).max(rect.height());
            let fill = Rect::from_min_size(rect.min, vec2(w, rect.height()));
            p.rect_filled(fill, r, color);
        }
    }
}

pub fn gear_icon(p: &Painter, rect: Rect, color: Color32) {
    let h = rect.height();
    let c = rect.center();
    let stroke = Stroke::new((h / 9.0).max(1.0), color);
    p.circle_stroke(c, h * 0.26, stroke);
    p.circle_filled(c, h * 0.09, color);
    for i in 0..8 {
        let a = i as f32 * TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * h * 0.30, c + d * h * 0.44],
            Stroke::new(h / 7.0, color),
        );
    }
}

pub fn refresh_icon(p: &Painter, rect: Rect, color: Color32) {
    let h = rect.height();
    let c = rect.center();
    let r = h * 0.34;
    let stroke = Stroke::new((h / 9.0).max(1.0), color);
    let a0 = -PI * 0.35;
    let a1 = a0 + TAU * 0.80;
    p.add(Shape::line(arc(c, r, a0, a1), stroke));
    // Arrow head at the start of the arc, pointing against the sweep.
    let tip = c + vec2(a0.cos(), a0.sin()) * r;
    let tangent = vec2(-a0.sin(), a0.cos());
    let normal = vec2(a0.cos(), a0.sin());
    let k = h * 0.20;
    p.add(Shape::convex_polygon(
        vec![
            tip - tangent * k * 0.9,
            tip + normal * k * 0.7 + tangent * k * 0.3,
            tip - normal * k * 0.7 + tangent * k * 0.3,
        ],
        color,
        Stroke::NONE,
    ));
}

pub fn power_icon(p: &Painter, rect: Rect, color: Color32) {
    let h = rect.height();
    let c = rect.center() + vec2(0.0, h * 0.04);
    let stroke = Stroke::new((h / 9.0).max(1.0), color);
    let gap = 0.30 * PI;
    p.add(Shape::line(
        arc(c, h * 0.34, -PI / 2.0 + gap, 1.5 * PI - gap),
        stroke,
    ));
    p.line_segment([c - vec2(0.0, h * 0.46), c - vec2(0.0, h * 0.08)], stroke);
}

/// Crescent moon (used for "asleep" and the empty state).
pub fn moon_icon(p: &Painter, rect: Rect, color: Color32, cut: Color32) {
    let h = rect.height();
    let c = rect.center();
    p.circle_filled(c, h * 0.38, color);
    p.circle_filled(c + vec2(h * 0.18, -h * 0.14), h * 0.32, cut);
}

/// Mini screen with four clickable corners. Returns true if `anchor` changed.
pub fn corner_picker(ui: &mut egui::Ui, anchor: &mut Anchor) -> bool {
    let size = vec2(132.0, 92.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let screen = Rect::from_min_size(rect.min, vec2(size.x, 76.0));

    // Stand.
    let neck = Rect::from_center_size(
        pos2(screen.center().x, screen.bottom() + 5.0),
        vec2(14.0, 10.0),
    );
    p.rect_filled(neck, CornerRadius::ZERO, theme::border());
    let foot = Rect::from_center_size(
        pos2(screen.center().x, screen.bottom() + 12.0),
        vec2(48.0, 4.0),
    );
    p.rect_filled(foot, CornerRadius::same(2), theme::border());

    p.rect(
        screen,
        CornerRadius::same(7),
        theme::BG,
        Stroke::new(1.5, theme::border()),
        StrokeKind::Inside,
    );
    // Faint "taskbar".
    let bar = Rect::from_min_max(
        pos2(screen.left() + 6.0, screen.bottom() - 9.0),
        pos2(screen.right() - 6.0, screen.bottom() - 6.0),
    );
    p.rect_filled(bar, CornerRadius::same(2), alpha(theme::TEXT, 0.06));

    let pill = vec2(34.0, 14.0);
    let inset = 8.0;
    let corners = [
        (Anchor::TopLeft, Align::Min, Align::Min, "Oben links"),
        (Anchor::TopRight, Align::Max, Align::Min, "Oben rechts"),
        (Anchor::BottomLeft, Align::Min, Align::Max, "Unten links"),
        (Anchor::BottomRight, Align::Max, Align::Max, "Unten rechts"),
    ];
    let mut changed = false;
    for (value, hx, vy, label) in corners {
        let x = match hx {
            Align::Min => screen.left() + inset,
            Align::Max => screen.right() - inset - pill.x,
        };
        let y = match vy {
            Align::Min => screen.top() + inset,
            Align::Max => screen.bottom() - inset - pill.y - 6.0,
        };
        let r = Rect::from_min_size(pos2(x, y), pill);
        let hit = r.expand(4.0);
        let resp = ui
            .interact(hit, ui.scope_id().with(("corner", label)), Sense::click())
            .on_hover_text(label);
        if resp.clicked() && *anchor != value {
            *anchor = value;
            changed = true;
        }
        let selected = *anchor == value;
        let (fill, stroke) = if selected {
            (theme::ACCENT, Stroke::NONE)
        } else if resp.hovered() {
            (theme::SELECTION, Stroke::new(1.0, theme::OUTLINE))
        } else {
            (theme::SURFACE_FOCUS, Stroke::new(1.0, theme::border()))
        };
        p.rect(r, CornerRadius::same(5), fill, stroke, StrokeKind::Inside);
        // Tiny level stripe so the pill reads as the overlay.
        let stripe_color = if selected {
            mix(theme::ACCENT, theme::BG, 0.55)
        } else {
            theme::faint()
        };
        let stripe = Rect::from_min_size(r.min + vec2(6.0, 5.5), vec2(pill.x - 12.0, 3.0));
        p.rect_filled(stripe, CornerRadius::same(2), stripe_color);
    }
    changed
}

#[derive(Clone, Copy)]
enum Align {
    Min,
    Max,
}
