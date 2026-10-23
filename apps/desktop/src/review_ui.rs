use crate::theme::*;
use eframe::egui::{self, vec2, RichText};

pub fn choice(ui: &mut egui::Ui, label: &str, selected: bool, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 32.0],
        egui::Button::new(RichText::new(label).size(13.0).color(if selected {
            egui::Color32::WHITE
        } else {
            MUTED
        }))
        .fill(if selected {
            ACCENT
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .rounding(6.0),
    )
}

pub fn section() -> egui::Frame {
    egui::Frame::none()
        .fill(NAV)
        .rounding(8.0)
        .inner_margin(12.0)
}

pub fn action(ui: &mut egui::Ui, label: &str, enabled: bool, primary: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label)
                .size(13.0)
                .color(if primary && enabled {
                    egui::Color32::WHITE
                } else {
                    INK
                }),
        )
        .min_size(vec2(ui.available_width(), 36.0))
        .fill(if primary && enabled { ACCENT } else { PANEL })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if primary && enabled { ACCENT } else { LINE },
        ))
        .rounding(6.0),
    )
}

pub fn decision(
    ui: &mut egui::Ui,
    label: &str,
    key: &str,
    selected: bool,
    primary: bool,
    danger: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(116.0, 38.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let fill = if primary {
        ACCENT
    } else if selected || response.hovered() {
        SOFT
    } else {
        PANEL
    };
    let color = if primary {
        egui::Color32::WHITE
    } else if danger {
        DANGER
    } else {
        INK
    };
    ui.painter().rect(
        rect,
        7.0,
        fill,
        egui::Stroke::new(1.0_f32, if primary { ACCENT } else { LINE }),
    );
    ui.painter().text(
        rect.left_center() + vec2(14.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        medium(13.0),
        color,
    );
    let key_rect =
        egui::Rect::from_center_size(rect.right_center() - vec2(19.0, 0.0), vec2(22.0, 22.0));
    ui.painter().rect_filled(
        key_rect,
        4.0,
        if primary {
            egui::Color32::from_white_alpha(24)
        } else {
            NAV
        },
    );
    ui.painter().text(
        key_rect.center(),
        egui::Align2::CENTER_CENTER,
        key,
        egui::FontId::proportional(11.0),
        color,
    );
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect.expand(2.0), 8.0, egui::Stroke::new(1.0_f32, ACCENT));
    }
    response.on_hover_text(format!("{label} · {key}"))
}
