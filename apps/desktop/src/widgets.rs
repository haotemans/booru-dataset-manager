use crate::theme::*;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Sense, Shape, Stroke, Ui,
};

pub fn primary(ui: &mut Ui, label: &str) -> Response {
    let enabled = ui.is_enabled();
    ui.add(
        egui::Button::new(RichText::new(label).color(if enabled { Color32::WHITE } else { MUTED }))
            .fill(if enabled { INK } else { PANEL })
            .stroke(Stroke::new(1.0_f32, if enabled { INK } else { LINE }))
            .rounding(7.0),
    )
}

/// A shared, compact vector control for image tools. Keep the accessible name
/// even when the toolbar only displays an icon.
pub fn tool_button(ui: &mut Ui, kind: &str, label: &str, selected: bool) -> Response {
    tool_control(ui, kind, label, label, selected)
}

pub fn tool_icon(ui: &mut Ui, kind: &str, label: &str, selected: bool) -> Response {
    tool_control(ui, kind, "", label, selected)
}

fn tool_control(ui: &mut Ui, kind: &str, text: &str, name: &str, selected: bool) -> Response {
    let selected = selected && ui.is_enabled();
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), FontId::proportional(12.0), INK);
    let width = if text.is_empty() {
        30.0
    } else {
        galley.size().x + 38.0
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    let color = if ui.is_enabled() {
        if selected {
            Color32::WHITE
        } else {
            INK
        }
    } else {
        MUTED
    };
    ui.painter().rect(
        rect,
        6.0,
        if selected { INK } else { PANEL },
        Stroke::new(
            1.0_f32,
            if selected {
                INK
            } else if response.hovered() || response.has_focus() {
                LINE
            } else {
                Color32::TRANSPARENT
            },
        ),
    );
    let center = rect.left_center() + vec2(15.0, 0.0);
    icon(
        ui,
        Rect::from_center_size(center, vec2(15.0, 15.0)),
        kind,
        color,
    );
    if !text.is_empty() {
        ui.painter().text(
            rect.left_center() + vec2(28.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            color,
        );
    }
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect.expand(2.0), 7.0, Stroke::new(1.0_f32, ACCENT));
    }
    response.on_hover_text(name)
}

pub fn hint(ui: &mut Ui, detail: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    icon(ui, rect.shrink(3.0), "info", MUTED);
    response.on_hover_text(detail);
}

pub fn icon_label(ui: &mut Ui, kind: &str, label: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
        icon(ui, rect, kind, MUTED);
        ui.label(RichText::new(label).size(12.0).color(MUTED));
    })
    .response
    .on_hover_text(detail);
}

pub fn choice_card(
    ui: &mut Ui,
    kind: &str,
    title: &str,
    detail: &str,
    selected: bool,
    width: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 60.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            title,
        )
    });
    ui.painter().rect(
        rect,
        8.0,
        if selected {
            SOFT
        } else if response.hovered() {
            NAV
        } else {
            PANEL
        },
        Stroke::new(
            1.0_f32,
            if selected || response.has_focus() {
                ACCENT
            } else {
                LINE
            },
        ),
    );
    icon(
        ui,
        Rect::from_center_size(rect.left_center() + vec2(24.0, 0.0), vec2(21.0, 21.0)),
        kind,
        if selected { ACCENT } else { MUTED },
    );
    ui.painter().text(
        rect.min + vec2(46.0, 12.0),
        Align2::LEFT_TOP,
        title,
        medium(13.0),
        INK,
    );
    ui.painter().text(
        rect.min + vec2(46.0, 34.0),
        Align2::LEFT_TOP,
        detail,
        FontId::proportional(11.0),
        MUTED,
    );
    response
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
        if !selected {
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(1.0_f32, if response.has_focus() { INK } else { LINE }),
            );
        }
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

pub fn nav_button(ui: &mut Ui, kind: &str, label: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    if selected {
        ui.painter().line_segment(
            [
                rect.left_center() - vec2(0.0, 8.0),
                rect.left_center() + vec2(0.0, 8.0),
            ],
            Stroke::new(2.5_f32, INK),
        );
    }
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            6.0,
            Stroke::new(1.0_f32, if response.has_focus() { INK } else { LINE }),
        );
    }
    let color = if selected || response.hovered() {
        INK
    } else {
        MUTED
    };
    icon(
        ui,
        Rect::from_center_size(rect.left_center() + vec2(20.0, 0.0), vec2(17.0, 17.0)),
        kind,
        color,
    );
    ui.painter().text(
        rect.left_center() + vec2(40.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        if selected {
            medium(13.0)
        } else {
            FontId::proportional(13.0)
        },
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn segment(ui: &mut Ui, label: &str, selected: bool, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    if selected && ui.is_enabled() {
        ui.painter().rect_filled(rect, 5.0, INK);
    }
    if response.has_focus() || (response.hovered() && !selected) {
        ui.painter()
            .rect_stroke(rect, 5.0, Stroke::new(1.0_f32, INK));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        if selected && ui.is_enabled() {
            Color32::WHITE
        } else if ui.is_enabled() {
            MUTED
        } else {
            MUTED.gamma_multiply(0.5)
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    let width = ui
        .painter()
        .layout_no_wrap(text.into(), FontId::proportional(12.0), color)
        .size()
        .x;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 13.0, 24.0), Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    ui.painter()
        .circle_filled(rect.left_center() + vec2(3.0, 0.0), 2.5, color);
    ui.painter().text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        color,
    );
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
        "info" => {
            p.circle_stroke(
                at(0.5, 0.5),
                rect.width() * 0.4,
                Stroke::new(1.4_f32, color),
            );
            p.circle_filled(at(0.5, 0.3), rect.width() * 0.045, color);
            line((0.5, 0.46), (0.5, 0.74));
        }
        "crop" => {
            line((0.23, 0.04), (0.23, 0.77));
            line((0.23, 0.77), (0.96, 0.77));
            line((0.04, 0.23), (0.77, 0.23));
            line((0.77, 0.23), (0.77, 0.96));
        }
        "brush" => {
            p.add(Shape::closed_line(
                vec![
                    at(0.35, 0.58),
                    at(0.72, 0.12),
                    at(0.88, 0.26),
                    at(0.48, 0.7),
                ],
                Stroke::new(1.5_f32, color),
            ));
            line((0.35, 0.58), (0.16, 0.85));
            line((0.16, 0.85), (0.48, 0.7));
            line((0.13, 0.94), (0.61, 0.94));
        }
        "text" => {
            line((0.15, 0.18), (0.85, 0.18));
            line((0.5, 0.18), (0.5, 0.85));
            line((0.32, 0.85), (0.68, 0.85));
        }
        "fit" => {
            for (x, y, dx, dy) in [
                (0.12, 0.12, 0.24, 0.24),
                (0.88, 0.12, -0.24, 0.24),
                (0.12, 0.88, 0.24, -0.24),
                (0.88, 0.88, -0.24, -0.24),
            ] {
                line((x, y + dy), (x, y));
                line((x, y), (x + dx, y));
            }
        }
        "external" => {
            line((0.43, 0.15), (0.15, 0.15));
            line((0.15, 0.15), (0.15, 0.85));
            line((0.15, 0.85), (0.85, 0.85));
            line((0.85, 0.85), (0.85, 0.57));
            line((0.49, 0.51), (0.88, 0.12));
            line((0.58, 0.12), (0.88, 0.12));
            line((0.88, 0.12), (0.88, 0.42));
        }
        "minus" => line((0.16, 0.5), (0.84, 0.5)),
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
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 80.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    icon(
        ui,
        Rect::from_min_size(rect.min + vec2(0.0, 3.0), vec2(15.0, 15.0)),
        kind,
        MUTED,
    );
    ui.painter().text(
        rect.min + vec2(23.0, 10.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.0),
        MUTED,
    );
    ui.painter().text(
        rect.min + vec2(0.0, 45.0),
        Align2::LEFT_CENTER,
        count.to_string(),
        medium(30.0),
        INK,
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(
            1.0_f32,
            if response.hovered() || response.has_focus() {
                INK
            } else {
                LINE
            },
        ),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn coverage_bar(
    ui: &mut Ui,
    count: usize,
    total: usize,
    label: &str,
    hint: &str,
    width: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), hint));
    let fraction = if total == 0 {
        0.0
    } else {
        (count as f32 / total as f32).clamp(0.0, 1.0)
    };
    let track = Rect::from_min_size(rect.min + vec2(0.0, 27.0), vec2(width, 3.0));
    ui.painter()
        .rect(track, 1.5, PANEL, Stroke::new(1.0_f32, LINE));
    if fraction > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(track.min, vec2(width * fraction, 3.0)),
            1.5,
            INK,
        );
    }
    ui.painter().text(
        rect.left_top(),
        Align2::LEFT_TOP,
        label,
        FontId::proportional(12.0),
        if response.hovered() { INK } else { MUTED },
    );
    ui.painter().text(
        rect.right_top(),
        Align2::RIGHT_TOP,
        if total == 0 {
            "—".into()
        } else {
            format!("{count} / {total}")
        },
        FontId::proportional(12.0),
        INK,
    );
    response.on_hover_text(format!("{hint} · {count} / {total}"))
}
