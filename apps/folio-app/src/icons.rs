//! Folio's toolbar icons, drawn as simple vector strokes: no font or image
//! asset to ship, nothing taken from another product (AGENTS.md §1).

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Open,
    First,
    Previous,
    Next,
    Last,
    ZoomOut,
    ZoomIn,
    Presets,
    FitPage,
    FitWidth,
    RotateCounterclockwise,
    RotateClockwise,
}

/// Draw `icon` centred in `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) * 0.5 * 0.7;
    let c = rect.center();
    let p = |x: f32, y: f32| Pos2::new(c.x + x * s, c.y + y * s);
    let stroke = Stroke::new(1.6, color);
    let line = |pts: Vec<Pos2>| painter.add(Shape::line(pts, stroke));
    match icon {
        Icon::Open => {
            line(vec![
                p(-1.0, 0.8),
                p(-1.0, -0.7),
                p(-0.35, -0.7),
                p(-0.15, -0.45),
                p(0.85, -0.45),
                p(0.85, -0.15),
            ]);
            line(vec![
                p(-1.0, 0.8),
                p(-0.65, -0.15),
                p(1.1, -0.15),
                p(0.75, 0.8),
                p(-1.0, 0.8),
            ]);
        }
        Icon::Previous => {
            line(vec![p(0.35, -0.8), p(-0.4, 0.0), p(0.35, 0.8)]);
        }
        Icon::Next => {
            line(vec![p(-0.35, -0.8), p(0.4, 0.0), p(-0.35, 0.8)]);
        }
        Icon::First => {
            line(vec![p(-0.7, -0.8), p(-0.7, 0.8)]);
            line(vec![p(0.45, -0.8), p(-0.3, 0.0), p(0.45, 0.8)]);
        }
        Icon::Last => {
            line(vec![p(0.7, -0.8), p(0.7, 0.8)]);
            line(vec![p(-0.45, -0.8), p(0.3, 0.0), p(-0.45, 0.8)]);
        }
        Icon::ZoomOut => {
            line(vec![p(-0.8, 0.0), p(0.8, 0.0)]);
        }
        Icon::ZoomIn => {
            line(vec![p(-0.8, 0.0), p(0.8, 0.0)]);
            line(vec![p(0.0, -0.8), p(0.0, 0.8)]);
        }
        Icon::Presets => {
            painter.add(Shape::convex_polygon(
                vec![p(-0.55, -0.25), p(0.55, -0.25), p(0.0, 0.4)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::FitPage => {
            painter.rect_stroke(
                Rect::from_min_max(p(-0.6, -0.85), p(0.6, 0.85)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            line(vec![p(-0.3, -0.35), p(0.3, -0.35)]);
            line(vec![p(-0.3, 0.0), p(0.3, 0.0)]);
            line(vec![p(-0.3, 0.35), p(0.3, 0.35)]);
        }
        Icon::FitWidth => {
            line(vec![p(-1.0, -0.8), p(-1.0, 0.8)]);
            line(vec![p(1.0, -0.8), p(1.0, 0.8)]);
            line(vec![p(-0.7, 0.0), p(0.7, 0.0)]);
            line(vec![p(-0.35, -0.35), p(-0.7, 0.0), p(-0.35, 0.35)]);
            line(vec![p(0.35, -0.35), p(0.7, 0.0), p(0.35, 0.35)]);
        }
        Icon::RotateClockwise | Icon::RotateCounterclockwise => {
            let cw = icon == Icon::RotateClockwise;
            let dir = if cw { 1.0 } else { -1.0 };
            // An arc of 270° starting at the top, with an arrowhead at its end.
            let steps = 24;
            let pts: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2
                        + dir * (i as f32 / steps as f32) * 1.5 * std::f32::consts::PI;
                    p(0.75 * a.cos(), 0.75 * a.sin())
                })
                .collect();
            line(pts);
            // Arrowhead at the arc's end, along the direction of turning.
            let end = -std::f32::consts::FRAC_PI_2 + dir * 1.5 * std::f32::consts::PI;
            let tip = p(0.75 * end.cos(), 0.75 * end.sin());
            let along = Vec2::new(-end.sin(), end.cos()) * dir;
            let across = Vec2::new(-along.y, along.x);
            let back = tip - along * 0.4 * s;
            line(vec![back + across * 0.3 * s, tip, back - across * 0.3 * s]);
        }
    }
}

/// A toolbar button showing `icon`, named `name` for tooltips, screen
/// readers and tests.
pub fn button(ui: &mut egui::Ui, icon: Icon, name: &'static str, enabled: bool) -> egui::Response {
    let r = ui
        .add_enabled(
            enabled,
            egui::Button::new("").min_size(Vec2::new(26.0, 22.0)),
        )
        .on_hover_text(name);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    let visuals = ui.style().interact(&r);
    let color = if enabled {
        visuals.fg_stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    paint(ui.painter(), r.rect.shrink(3.0), icon, color);
    r
}
