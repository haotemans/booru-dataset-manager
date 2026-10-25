use crate::theme::*;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Sense, Shape, Stroke, Ui,
};

pub fn primary(ui: &mut Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(Color32::WHITE))
            .fill(INK)
            .rounding(6.0),
    )
}

/// Compact controls retain a readable accessible name and a hover label.
pub fn icon_button(ui: &mut Ui, kind: &str, label: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if selected || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 8.0, if selected { INK } else { NAV });
    }
    let color = if !ui.is_enabled() {
        MUTED.gamma_multiply(0.45)
    } else if selected {
        Color32::WHITE
    } else if kind == "trash" {
        DANGER
    } else {
        INK
    };
    icon(ui, rect.shrink(11.0), kind, color);
    response.on_hover_text(label)
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.09))
        .rounding(5.0)
        .inner_margin(vec2(8.0, 4.0))
        .show(ui, |ui| {
            let width = ui
                .painter()
                .layout_no_wrap(text.into(), FontId::proportional(12.0), color)
                .size()
                .x;
            ui.set_width(width);
            ui.label(RichText::new(text).size(12.0).color(color));
        });
}

pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    ui.add_sized(
        [width, 34.0],
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(vec2(11.0, 8.0)),
    )
}

pub fn section(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).font(medium(15.0)));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).size(13.0).color(MUTED));
        }
    });
}

pub fn setting_header(ui: &mut Ui, kind: &str, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(23.0, 23.0), Sense::hover());
        icon(ui, rect.shrink(3.0), kind, ACCENT);
        ui.label(RichText::new(title).font(medium(15.0)));
    });
}

pub fn icon_stat(ui: &mut Ui, kind: &str, count: u64, label: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
        icon(ui, rect, kind, color);
        ui.label(RichText::new(count.to_string()).size(12.0).color(color));
    })
    .response
    .on_hover_text(format!("{label} {count}"));
}

pub struct TaskRow<'a> {
    pub kind: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub state: &'a str,
    pub color: Color32,
    pub action: Option<&'a str>,
    pub enabled: bool,
    pub error: Option<&'a str>,
}

pub fn task_state(state: &str) -> (&'static str, Color32) {
    match state {
        "completed" => ("已完成", ACCENT),
        "running" => ("进行中", ACCENT),
        "paused" | "interrupted" | "cancelled" => ("已暂停", MUTED),
        "queued" => ("等待中", MUTED),
        "needs_model" => ("模型未就绪", AMBER),
        "awaiting_approval" => ("待确认", AMBER),
        "deferred" => ("待重试", AMBER),
        "failed" => ("失败", DANGER),
        _ => ("未完成", MUTED),
    }
}

/// Shared task hierarchy: identity, fixed status/action columns, then readable metrics.
pub fn task_row(ui: &mut Ui, row: TaskRow<'_>, details: impl FnOnce(&mut Ui)) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.spacing_mut().button_padding = vec2(10.0, 4.0);
        let mut clicked = false;
        let title_width = (ui.available_width() - 170.0).max(80.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.allocate_ui_with_layout(
                vec2(title_width, 30.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(title_width);
                    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    icon(ui, rect.shrink(2.0), row.kind, ACCENT);
                    ui.add(
                        egui::Label::new(RichText::new(row.title).font(medium(13.0))).truncate(),
                    )
                    .on_hover_text(row.title);
                },
            );
            ui.allocate_ui_with_layout(
                vec2(80.0, 30.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(80.0);
                    badge(ui, row.state, row.color);
                },
            );
            ui.allocate_ui_with_layout(
                vec2(74.0, 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.set_width(74.0);
                    if let Some(action) = row.action {
                        clicked = ui
                            .add_enabled_ui(row.enabled, |ui| {
                                ui.add_sized(
                                    [64.0, 28.0],
                                    egui::Button::new(RichText::new(action).size(12.0))
                                        .rounding(5.0),
                                )
                            })
                            .inner
                            .clicked();
                    }
                },
            );
        });
        egui::Frame::none()
            .inner_margin(egui::Margin {
                left: 28.0,
                right: 0.0,
                top: 2.0,
                bottom: 8.0,
            })
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(16.0, 6.0);
                    if !row.subtitle.is_empty() {
                        ui.label(RichText::new(row.subtitle).size(12.0).color(MUTED));
                    }
                    details(ui);
                });
                if let Some(error) = row.error {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(error).size(12.0).color(AMBER)).wrap());
                }
            });
        clicked
    })
    .inner
}

pub fn task_metric(ui: &mut Ui, label: &str, value: impl std::fmt::Display, color: Color32) {
    ui.label(
        RichText::new(format!("{label} {value}"))
            .size(12.0)
            .color(color),
    );
}

pub fn empty(ui: &mut Ui, title: &str, detail: &str, kind: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        let (rect, _) = ui.allocate_exact_size(vec2(62.0, 62.0), Sense::hover());
        ui.painter().rect_filled(rect, 12.0, NAV);
        icon(ui, rect.shrink(18.0), kind, MUTED);
        ui.add_space(12.0);
        ui.label(RichText::new(title).font(medium(16.0)))
            .on_hover_text(detail);
        ui.add_space(22.0);
    });
}

pub fn icon(ui: &Ui, rect: Rect, kind: &str, color: Color32) {
    let p = ui.painter();
    let at = |x: f32, y: f32| {
        pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([at(a.0, a.1), at(b.0, b.1)], Stroke::new(1.6_f32, color));
    };
    match kind {
        "pause" => {
            line((0.32, 0.15), (0.32, 0.85));
            line((0.68, 0.15), (0.68, 0.85));
        }
        "trash" => {
            line((0.12, 0.23), (0.88, 0.23));
            line((0.35, 0.23), (0.35, 0.08));
            line((0.35, 0.08), (0.65, 0.08));
            line((0.65, 0.08), (0.65, 0.23));
            line((0.23, 0.23), (0.29, 0.92));
            line((0.29, 0.92), (0.71, 0.92));
            line((0.71, 0.92), (0.77, 0.23));
            line((0.42, 0.4), (0.44, 0.75));
            line((0.58, 0.4), (0.56, 0.75));
        }
        "plus" => {
            line((0.5, 0.12), (0.5, 0.88));
            line((0.12, 0.5), (0.88, 0.5));
        }
        "download" => {
            line((0.5, 0.08), (0.5, 0.65));
            line((0.26, 0.43), (0.5, 0.67));
            line((0.5, 0.67), (0.74, 0.43));
            line((0.12, 0.7), (0.12, 0.91));
            line((0.12, 0.91), (0.88, 0.91));
            line((0.88, 0.91), (0.88, 0.7));
        }
        "spark" => {
            p.add(Shape::closed_line(
                vec![
                    at(0.5, 0.06),
                    at(0.64, 0.37),
                    at(0.94, 0.5),
                    at(0.64, 0.64),
                    at(0.5, 0.94),
                    at(0.36, 0.64),
                    at(0.06, 0.5),
                    at(0.36, 0.37),
                ],
                Stroke::new(1.6_f32, color),
            ));
        }
        "layers" | "masonry" => {
            for (x, y, w, h) in if kind == "layers" {
                [(0.1, 0.1, 0.55, 0.55), (0.35, 0.35, 0.55, 0.55)]
            } else {
                [(0.08, 0.08, 0.32, 0.54), (0.59, 0.08, 0.32, 0.3)]
            } {
                p.rect_stroke(
                    Rect::from_min_size(at(x, y), vec2(rect.width() * w, rect.height() * h)),
                    1.5,
                    Stroke::new(1.6_f32, color),
                );
            }
            if kind == "masonry" {
                p.rect_stroke(
                    Rect::from_min_max(at(0.08, 0.76), at(0.4, 0.94)),
                    1.5,
                    Stroke::new(1.6_f32, color),
                );
                p.rect_stroke(
                    Rect::from_min_max(at(0.59, 0.53), at(0.91, 0.94)),
                    1.5,
                    Stroke::new(1.6_f32, color),
                );
            }
        }
        "back" | "forward" => {
            let x = if kind == "back" { 0.35 } else { 0.65 };
            let other = 1.0 - x;
            line((other, 0.18), (x, 0.5));
            line((x, 0.5), (other, 0.82));
        }
        "close" => {
            line((0.23, 0.23), (0.77, 0.77));
            line((0.77, 0.23), (0.23, 0.77));
        }
        "check" => {
            line((0.12, 0.5), (0.4, 0.77));
            line((0.4, 0.77), (0.89, 0.23));
        }
        "reset" => {
            let points: Vec<_> = (0..=20)
                .map(|i| {
                    let a = -2.7 + i as f32 / 20.0 * 5.1;
                    at(0.5 + a.cos() * 0.35, 0.5 + a.sin() * 0.35)
                })
                .collect();
            p.add(Shape::line(points, Stroke::new(1.6_f32, color)));
            line((0.13, 0.12), (0.13, 0.42));
            line((0.13, 0.42), (0.42, 0.42));
        }
        "search" => {
            p.circle_stroke(
                at(0.42, 0.42),
                rect.width() * 0.3,
                Stroke::new(1.6_f32, color),
            );
            line((0.65, 0.65), (0.93, 0.93));
        }
        "lock" => {
            p.rect_stroke(
                Rect::from_min_max(at(0.18, 0.42), at(0.82, 0.92)),
                2.0,
                Stroke::new(1.6_f32, color),
            );
            p.add(Shape::line(
                vec![
                    at(0.3, 0.42),
                    at(0.3, 0.22),
                    at(0.4, 0.1),
                    at(0.6, 0.1),
                    at(0.7, 0.22),
                    at(0.7, 0.42),
                ],
                Stroke::new(1.6_f32, color),
            ));
        }
        "clock" => {
            p.circle_stroke(
                rect.center(),
                rect.width() * 0.4,
                Stroke::new(1.6_f32, color),
            );
            line((0.5, 0.25), (0.5, 0.5));
            line((0.5, 0.5), (0.7, 0.6));
        }
        "dashboard" | "grid" => {
            for y in [0.08, 0.57] {
                for x in [0.08, 0.57] {
                    p.rect_stroke(
                        Rect::from_min_max(at(x, y), at(x + 0.32, y + 0.32)),
                        2.0,
                        Stroke::new(1.6_f32, color),
                    );
                }
            }
        }
        "artist" => {
            p.circle_stroke(
                at(0.5, 0.5),
                rect.width() * 0.4,
                Stroke::new(1.6_f32, color),
            );
            for (x, y) in [(0.32, 0.28), (0.6, 0.25), (0.72, 0.48), (0.3, 0.57)] {
                p.circle_filled(at(x, y), 1.6, color);
            }
            line((0.49, 0.88), (0.65, 0.72));
        }
        "character" => {
            p.circle_stroke(
                at(0.5, 0.28),
                rect.width() * 0.18,
                Stroke::new(1.6_f32, color),
            );
            p.add(Shape::line(
                vec![
                    at(0.12, 0.91),
                    at(0.2, 0.68),
                    at(0.38, 0.57),
                    at(0.62, 0.57),
                    at(0.8, 0.68),
                    at(0.88, 0.91),
                ],
                Stroke::new(1.6_f32, color),
            ));
        }
        "tag" => {
            p.add(Shape::closed_line(
                vec![
                    at(0.08, 0.12),
                    at(0.53, 0.12),
                    at(0.94, 0.54),
                    at(0.54, 0.94),
                    at(0.08, 0.48),
                ],
                Stroke::new(1.6_f32, color),
            ));
            p.circle_stroke(at(0.3, 0.32), 2.0, Stroke::new(1.4_f32, color));
        }
        "api" => {
            line((0.28, 0.25), (0.05, 0.5));
            line((0.05, 0.5), (0.28, 0.75));
            line((0.72, 0.25), (0.95, 0.5));
            line((0.95, 0.5), (0.72, 0.75));
            line((0.6, 0.12), (0.4, 0.88));
        }
        "settings" => {
            for y in [0.2, 0.5, 0.8] {
                line((0.06, y), (0.94, y));
                p.circle_filled(at(if y == 0.5 { 0.7 } else { 0.3 }, y), 3.0, color);
            }
        }
        "logs" => {
            p.rect_stroke(rect.shrink(2.0), 2.0, Stroke::new(1.6_f32, color));
            for y in [0.3, 0.5, 0.7] {
                line((0.25, y), (0.75, y));
            }
        }
        "image" => {
            p.rect_stroke(rect.shrink(1.0), 3.0, Stroke::new(1.5_f32, color));
            p.circle_filled(at(0.7, 0.3), 2.5, color);
            p.add(Shape::line(
                vec![
                    at(0.12, 0.78),
                    at(0.38, 0.48),
                    at(0.6, 0.7),
                    at(0.72, 0.57),
                    at(0.9, 0.8),
                ],
                Stroke::new(1.5_f32, color),
            ));
        }
        _ => {
            p.circle_stroke(
                rect.center(),
                rect.width() * 0.35,
                Stroke::new(1.6_f32, color),
            );
            line((0.5, 0.29), (0.5, 0.55));
            p.circle_filled(at(0.5, 0.7), 1.0, color);
        }
    }
}

/// Procedural illustration for explicitly labelled sample data; never a media thumbnail.
pub fn sample_art(ui: &Ui, rect: Rect, seed: usize) {
    let palettes = [
        ([220, 226, 227], [159, 180, 181], [99, 131, 137]),
        ([233, 225, 216], [192, 171, 154], [139, 123, 114]),
        ([223, 229, 237], [168, 184, 206], [105, 128, 156]),
        ([226, 229, 218], [178, 188, 160], [121, 141, 118]),
        ([233, 223, 226], [194, 170, 181], [144, 120, 140]),
        ([220, 229, 234], [156, 185, 196], [97, 139, 158]),
    ];
    let (sky, far, near) = palettes[seed % palettes.len()];
    let color = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 8.0, color(sky));
    let at = |x: f32, y: f32| {
        pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    p.circle_filled(
        at(0.72, 0.28),
        rect.width() * 0.105,
        Color32::from_rgb(249, 239, 216),
    );
    p.add(Shape::convex_polygon(
        vec![at(-0.1, 0.85), at(0.3, 0.3), at(0.77, 0.98)],
        color(far),
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![at(0.12, 1.05), at(0.7, 0.48), at(1.1, 1.05)],
        color(near),
        Stroke::NONE,
    ));
    p.rect_filled(
        Rect::from_min_max(at(0.0, 0.84), at(1.0, 1.0)),
        0.0,
        color(near).gamma_multiply(0.92),
    );
    for n in 0..4 {
        let y = 0.88 + n as f32 * 0.027;
        p.line_segment(
            [at(0.04, y), at(0.4 + n as f32 * 0.1, y)],
            Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.22)),
        );
    }
}

pub fn count_card(ui: &mut Ui, count: usize, title: &str, kind: &str) -> Response {
    egui::Frame::none()
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 20.0;
            ui.set_min_height(64.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                icon(ui, r.shrink(5.0), kind, MUTED);
                ui.vertical(|ui| {
                    ui.label(RichText::new(count.to_string()).font(medium(30.0)));
                    ui.label(RichText::new(title).size(13.0).color(MUTED));
                });
            });
        })
        .response
}

pub fn coverage_ring(
    ui: &mut Ui,
    count: usize,
    total: usize,
    label: &str,
    hint: &str,
    width: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 116.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), hint));
    let center = rect.center_top() + vec2(0.0, 38.0);
    let fraction = if total == 0 {
        0.0
    } else {
        count as f32 / total as f32
    };
    let radius = 31.0;
    ui.painter()
        .circle_stroke(center, radius, Stroke::new(5.0_f32, SOFT));
    if fraction > 0.0 {
        let points = (0..=64)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2
                    + std::f32::consts::TAU * fraction * i as f32 / 64.0;
                center + vec2(a.cos(), a.sin()) * radius
            })
            .collect();
        ui.painter().add(Shape::line(
            points,
            Stroke::new(5.0_f32, if response.hovered() { INK } else { ACCENT }),
        ));
    }
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        if total == 0 {
            "—".into()
        } else {
            format!("{:.0}%", fraction * 100.0)
        },
        medium(16.0),
        INK,
    );
    ui.painter().text(
        center + vec2(0.0, 55.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        MUTED,
    );
    response.on_hover_text(format!("{hint} · {count} / {total}"))
}
