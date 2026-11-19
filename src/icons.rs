//! Small painter-drawn icons.
//!
//! Everything is drawn from lines and arcs instead of font glyphs, so the interface looks the
//! same on machines whose fonts lack emoji or symbol characters.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Desktop,
    Router,
    Internet,
    Wifi,
    Ethernet,
    Refresh,
    Theme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Error,
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    let center = rect.center();
    let stroke = Stroke::new((size * 0.055).clamp(1.2, 2.4), color);

    match icon {
        Icon::Desktop => desktop(painter, center, size, stroke),
        Icon::Router | Icon::Wifi => wifi(painter, rect, size, stroke, color),
        Icon::Internet => internet(painter, center, size, stroke),
        Icon::Ethernet => ethernet(painter, center, size, stroke),
        Icon::Refresh => refresh(painter, center, size, stroke, color),
        Icon::Theme => theme(painter, center, size, stroke, color),
    }
}

fn desktop(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let screen = Rect::from_center_size(
        center + vec2(0.0, -size * 0.10),
        vec2(size * 0.78, size * 0.52),
    );
    painter.rect_stroke(screen, CornerRadius::same(2), stroke, StrokeKind::Inside);
    painter.line_segment(
        [
            screen.center_bottom(),
            pos2(center.x, center.y + size * 0.30),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(center.x - size * 0.22, center.y + size * 0.30),
            pos2(center.x + size * 0.22, center.y + size * 0.30),
        ],
        stroke,
    );
}

fn wifi(painter: &Painter, rect: Rect, size: f32, stroke: Stroke, color: Color32) {
    let base = pos2(rect.center().x, rect.bottom() - size * 0.24);
    painter.circle_filled(base, size * 0.055, color);
    for radius in [0.20, 0.33, 0.46] {
        painter.add(Shape::line(arc(base, size * radius, -2.44, -0.70), stroke));
    }
}

fn internet(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let radius = size * 0.42;
    painter.circle_stroke(center, radius, stroke);

    let meridian: Vec<Pos2> = (0..=40)
        .map(|step| {
            let angle = step as f32 / 40.0 * TAU;
            pos2(
                center.x + radius * 0.42 * angle.sin(),
                center.y + radius * angle.cos(),
            )
        })
        .collect();
    painter.add(Shape::line(meridian, stroke));

    painter.line_segment(
        [
            pos2(center.x - radius, center.y),
            pos2(center.x + radius, center.y),
        ],
        stroke,
    );
}

fn ethernet(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let body = Rect::from_center_size(
        center + vec2(0.0, size * 0.06),
        vec2(size * 0.60, size * 0.42),
    );
    painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);

    let tab = Rect::from_center_size(pos2(center.x, body.top()), vec2(size * 0.24, size * 0.12));
    painter.rect_stroke(tab, CornerRadius::ZERO, stroke, StrokeKind::Inside);

    for offset in [-0.16, -0.05, 0.06] {
        painter.line_segment(
            [
                pos2(center.x + size * offset, body.top() + size * 0.14),
                pos2(center.x + size * offset, body.bottom() - size * 0.10),
            ],
            stroke,
        );
    }
}

fn refresh(painter: &Painter, center: Pos2, size: f32, stroke: Stroke, color: Color32) {
    let radius = size * 0.34;
    painter.add(Shape::line(arc(center, radius, -1.85, 4.15), stroke));

    let angle = 4.15_f32;
    let end = center + vec2(angle.cos(), angle.sin()) * radius;
    let direction = vec2(-angle.sin(), angle.cos()).normalized();
    let side = vec2(-direction.y, direction.x);
    let tip = end + direction * size * 0.11;
    painter.add(Shape::convex_polygon(
        vec![
            tip,
            end - direction * size * 0.05 + side * size * 0.09,
            end - direction * size * 0.05 - side * size * 0.09,
        ],
        color,
        Stroke::NONE,
    ));
}

fn theme(painter: &Painter, center: Pos2, size: f32, stroke: Stroke, color: Color32) {
    let radius = size * 0.36;
    painter.circle_stroke(center, radius, stroke);
    let right_half: Vec<Pos2> = (0..=28)
        .map(|step| {
            let angle = -FRAC_PI_2 + PI * step as f32 / 28.0;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(Shape::convex_polygon(right_half, color, Stroke::NONE));
}

pub fn paint_result(painter: &Painter, rect: Rect, ok: bool, color: Color32) {
    let size = rect.width().min(rect.height());
    let center = rect.center();
    let stroke = Stroke::new((size * 0.12).clamp(1.4, 2.4), color);
    if ok {
        painter.add(Shape::line(
            vec![
                pos2(center.x - size * 0.30, center.y + size * 0.02),
                pos2(center.x - size * 0.08, center.y + size * 0.24),
                pos2(center.x + size * 0.32, center.y - size * 0.24),
            ],
            stroke,
        ));
    } else {
        painter.line_segment(
            [
                pos2(center.x - size * 0.26, center.y - size * 0.26),
                pos2(center.x + size * 0.26, center.y + size * 0.26),
            ],
            stroke,
        );
        painter.line_segment(
            [
                pos2(center.x + size * 0.26, center.y - size * 0.26),
                pos2(center.x - size * 0.26, center.y + size * 0.26),
            ],
            stroke,
        );
    }
}

pub fn paint_tone(painter: &Painter, rect: Rect, tone: Tone, color: Color32) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.46;
    painter.circle_stroke(center, radius, Stroke::new(1.7, color));
    let inner = Rect::from_center_size(center, vec2(radius * 1.15, radius * 1.15));
    match tone {
        Tone::Ok => paint_result(painter, inner, true, color),
        Tone::Error => paint_result(painter, inner, false, color),
        Tone::Warn => {
            painter.line_segment(
                [
                    center - vec2(0.0, radius * 0.46),
                    center + vec2(0.0, radius * 0.14),
                ],
                Stroke::new(2.0, color),
            );
            painter.circle_filled(center + vec2(0.0, radius * 0.42), radius * 0.11, color);
        }
    }
}

fn arc(center: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    let steps = 32;
    (0..=steps)
        .map(|step| {
            let angle = start + (end - start) * step as f32 / steps as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}
