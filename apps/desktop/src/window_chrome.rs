//! Windows window controls share the app's contrast, spacing and DPI scaling.
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Rect, Sense, Stroke, ViewportCommand};

const TITLE_HEIGHT: f32 = 40.0;
const BUTTON_WIDTH: f32 = 48.0;

pub fn content_rect(ctx: &egui::Context) -> Rect {
    let mut rect = ctx.screen_rect();
    if cfg!(target_os = "windows") {
        rect.min.y += TITLE_HEIGHT;
    }
    rect
}

pub fn show(ctx: &egui::Context) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    egui::TopBottomPanel::top("window_titlebar")
        .exact_height(TITLE_HEIGHT)
        .frame(egui::Frame::none().fill(NAV))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            let controls_left = rect.right() - BUTTON_WIDTH * 3.0 - 6.0;
            let drag_rect = Rect::from_min_max(
                rect.min + vec2(6.0, 4.0),
                pos2(controls_left - 8.0, rect.bottom()),
            );
            let drag = ui.interact(drag_rect, ui.id().with("drag"), Sense::click_and_drag());
            if drag.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            } else if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            let brand =
                Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(22.0, 22.0));
            ui.painter().rect_filled(brand, 5.0, ACCENT);
            crate::widgets::icon(ui, brand.shrink(4.0), "image", egui::Color32::WHITE);
            ui.painter().text(
                pos2(rect.left() + 44.0, rect.center().y),
                Align2::LEFT_CENTER,
                "Booru Dataset Manager",
                medium(13.0),
                INK,
            );

            for (index, label) in [
                "最小化",
                if maximized {
                    "还原窗口"
                } else {
                    "最大化"
                },
                "关闭程序",
            ]
            .into_iter()
            .enumerate()
            {
                let button_rect = Rect::from_min_size(
                    pos2(
                        controls_left + index as f32 * BUTTON_WIDTH,
                        rect.top() + 4.0,
                    ),
                    vec2(BUTTON_WIDTH - 2.0, 32.0),
                );
                let response = ui.interact(button_rect, ui.id().with(index), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                });
                let highlighted = response.hovered() || response.has_focus();
                let danger = index == 2 && highlighted;
                let color = if danger { egui::Color32::WHITE } else { INK };
                ui.painter().rect(
                    button_rect,
                    5.0,
                    if danger {
                        DANGER
                    } else if highlighted {
                        LINE
                    } else {
                        SOFT
                    },
                    Stroke::new(1.0_f32, if response.has_focus() { ACCENT } else { LINE }),
                );
                let center = button_rect.center();
                let line = |a, b| {
                    ui.painter()
                        .line_segment([a, b], Stroke::new(1.8_f32, color));
                };
                match index {
                    0 => line(center + vec2(-7.0, 0.0), center + vec2(7.0, 0.0)),
                    1 => {
                        let square = Rect::from_center_size(center, vec2(12.0, 12.0));
                        if maximized {
                            ui.painter().rect_stroke(
                                square.translate(vec2(2.0, -2.0)),
                                0.0,
                                Stroke::new(1.6_f32, color),
                            );
                            ui.painter().rect_filled(
                                square.translate(vec2(-1.0, 1.0)),
                                0.0,
                                if highlighted { LINE } else { SOFT },
                            );
                        }
                        ui.painter()
                            .rect_stroke(square, 0.0, Stroke::new(1.6_f32, color));
                    }
                    _ => {
                        line(center + vec2(-5.0, -5.0), center + vec2(5.0, 5.0));
                        line(center + vec2(5.0, -5.0), center + vec2(-5.0, 5.0));
                    }
                }
                if response.clicked() {
                    ctx.send_viewport_cmd(match index {
                        0 => ViewportCommand::Minimized(true),
                        1 => ViewportCommand::Maximized(!maximized),
                        _ => ViewportCommand::Close,
                    });
                }
                response.on_hover_text(label);
            }
            ui.painter().hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                Stroke::new(1.0_f32, LINE),
            );
        });
    if !maximized && !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        resize_edges(ctx);
    }
}

fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon, ResizeDirection};
    let rect = ctx.screen_rect();
    let edge = 5.0;
    let corner = 10.0;
    for (index, (area, direction, cursor)) in [
        (
            Rect::from_min_max(rect.min, rect.min + vec2(corner, corner)),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_max(
                pos2(rect.right() - corner, rect.top()),
                pos2(rect.right(), rect.top() + corner),
            ),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_max(
                pos2(rect.left(), rect.bottom() - corner),
                pos2(rect.left() + corner, rect.bottom()),
            ),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_max(rect.max - vec2(corner, corner), rect.max),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_max(
                pos2(rect.left() + corner, rect.top()),
                pos2(rect.right() - corner, rect.top() + edge),
            ),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(
                pos2(rect.left() + corner, rect.bottom() - edge),
                pos2(rect.right() - corner, rect.bottom()),
            ),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(
                pos2(rect.left(), rect.top() + corner),
                pos2(rect.left() + edge, rect.bottom() - corner),
            ),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(
                pos2(rect.right() - edge, rect.top() + corner),
                pos2(rect.right(), rect.bottom() - corner),
            ),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        egui::Area::new(egui::Id::new(("window_resize", index)))
            .order(egui::Order::Tooltip)
            .fixed_pos(area.min)
            .movable(false)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(area.size(), Sense::drag());
                let response = response.on_hover_cursor(cursor);
                if response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())
                {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
                }
            });
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
        maximized: bool,
    ) -> egui::FullOutput {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .maximized = Some(maximized);
        ctx.run(input, |ctx| {
            show(ctx);
            egui::CentralPanel::default().show(ctx, |_| {});
        })
    }

    #[test]
    fn window_buttons_accept_wide_click_targets_and_send_native_commands() {
        for size in [vec2(1050.0, 720.0), vec2(1440.0, 960.0)] {
            for (index, maximized, expected) in [
                (0, false, ViewportCommand::Minimized(true)),
                (1, false, ViewportCommand::Maximized(true)),
                (1, true, ViewportCommand::Maximized(false)),
                (2, false, ViewportCommand::Close),
            ] {
                let ctx = egui::Context::default();
                crate::theme::setup(&ctx);
                for _ in 0..3 {
                    frame(&ctx, size, vec![], maximized);
                }
                // Near the button edge, well outside the painted glyph.
                let pos = pos2(
                    size.x - BUTTON_WIDTH * 3.0 - 6.0 + index as f32 * BUTTON_WIDTH + 5.0,
                    30.0,
                );
                frame(
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    maximized,
                );
                let output = frame(
                    &ctx,
                    size,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                    maximized,
                );
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&expected),
                    "missing {expected:?}"
                );
                assert_eq!(content_rect(&ctx).top(), TITLE_HEIGHT);
            }
        }
    }

    #[test]
    fn modal_review_keeps_minimize_control_clickable() {
        use crate::{
            app::{BdmApp, Dialog},
            model::Preferences,
        };
        for review in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
            if review {
                app.open_media(3);
            } else {
                app.dialog = Some(Dialog::Add);
            }
            let size = vec2(1050.0, 720.0);
            let mut draw = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        show(ctx);
                        app.shell(ctx);
                    },
                )
            };
            for _ in 0..3 {
                draw(vec![]);
            }
            let pos = pos2(920.0, 20.0);
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            let output = draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&ViewportCommand::Minimized(true)),
                "minimize blocked by modal, review={review}"
            );
            assert!(app.dialog.is_some());
        }
    }

    #[test]
    fn border_resize_and_title_drag_send_native_commands() {
        let size = vec2(1050.0, 720.0);
        for (pos, direction) in [
            (pos2(2.0, 360.0), egui::ResizeDirection::West),
            (pos2(1048.0, 360.0), egui::ResizeDirection::East),
            (pos2(600.0, 2.0), egui::ResizeDirection::North),
            (pos2(600.0, 718.0), egui::ResizeDirection::South),
            (pos2(2.0, 2.0), egui::ResizeDirection::NorthWest),
            (pos2(1048.0, 718.0), egui::ResizeDirection::SouthEast),
        ] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            for _ in 0..3 {
                frame(&ctx, size, vec![], false);
            }
            let output = frame(
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                false,
            );
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&ViewportCommand::BeginResize(direction)),
                "missing border resize at {pos:?}"
            );
        }
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        for _ in 0..3 {
            frame(&ctx, size, vec![], false);
        }
        let pos = pos2(400.0, 20.0);
        frame(
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            false,
        );
        let output = frame(
            &ctx,
            size,
            vec![egui::Event::PointerMoved(pos + vec2(20.0, 20.0))],
            false,
        );
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&ViewportCommand::StartDrag));
    }
}
