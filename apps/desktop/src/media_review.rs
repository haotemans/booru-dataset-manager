use crate::{
    app::{BdmApp, Dialog},
    model::Media,
    theme::*,
    widgets::*,
};
use bdm_domain::ingest::StoredMedia;
use eframe::egui::{self, vec2, Rect, RichText, Sense};
use std::sync::mpsc::Receiver;

pub struct ReviewState {
    pub tab: usize,
    pub auto_advance: bool,
    canvas_id: Option<usize>,
    pub(crate) view: CanvasView,
    pub(crate) redownload: Option<(usize, Receiver<Result<StoredMedia, String>>)>,
    pub(crate) redownload_notice: Option<crate::redownload::RedownloadNotice>,
    #[cfg(test)]
    window: Option<Rect>,
    #[cfg(test)]
    footer: Option<Rect>,
    #[cfg(test)]
    keep_button: Option<Rect>,
    #[cfg(test)]
    canvas: Option<Rect>,
    #[cfg(test)]
    pub(crate) load_original_button: Option<Rect>,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            tab: 0,
            auto_advance: true,
            canvas_id: None,
            view: CanvasView::default(),
            redownload: None,
            redownload_notice: None,
            #[cfg(test)]
            window: None,
            #[cfg(test)]
            footer: None,
            #[cfg(test)]
            keep_button: None,
            #[cfg(test)]
            canvas: None,
            #[cfg(test)]
            load_original_button: None,
        }
    }
}

impl ReviewState {
    pub(crate) fn reset_canvas(&mut self) {
        self.canvas_id = None;
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CanvasView {
    pub(crate) zoom: f32,
    pan: egui::Vec2,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

impl CanvasView {
    fn image_rect(self, stage: Rect, dimensions: egui::Vec2) -> Rect {
        Rect::from_center_size(
            stage.center() + self.pan,
            dimensions * self.fit(stage, dimensions) * self.zoom,
        )
    }

    fn fit(self, stage: Rect, dimensions: egui::Vec2) -> f32 {
        (stage.width() / dimensions.x).min(stage.height() / dimensions.y)
    }

    fn zoom_at(&mut self, stage: Rect, pointer: egui::Pos2, factor: f32) {
        let zoom = (self.zoom * factor).clamp(0.1, 32.0);
        let offset = pointer - stage.center();
        self.pan = offset - (offset - self.pan) * (zoom / self.zoom);
        self.zoom = zoom;
    }

    fn constrain(&mut self, stage: Rect, dimensions: egui::Vec2) {
        let excess = ((self.image_rect(stage, dimensions).size() - stage.size()) * 0.5)
            .max(egui::Vec2::ZERO);
        self.pan = self.pan.clamp(-excess, excess);
    }
}

struct ReviewLayout {
    header: Rect,
    image: Rect,
    info: Rect,
    footer: Rect,
}

impl ReviewLayout {
    fn new(rect: Rect) -> Self {
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 48.0));
        let footer = Rect::from_min_size(
            egui::pos2(rect.left(), rect.bottom() - 80.0),
            vec2(rect.width(), 80.0),
        );
        let body = Rect::from_min_max(
            egui::pos2(rect.left(), header.bottom() + 8.0),
            egui::pos2(rect.right(), footer.top() - 8.0),
        );
        let info_width = (body.width() * 0.34).clamp(320.0, 400.0);
        let image_width = body.width() - info_width - 12.0;
        Self {
            header,
            image: Rect::from_min_size(body.min, vec2(image_width, body.height())),
            info: Rect::from_min_max(
                egui::pos2(body.left() + image_width + 12.0, body.top()),
                body.max,
            ),
            footer,
        }
    }
}

impl BdmApp {
    pub(crate) fn capture_review_content(&mut self) {
        if !self.is_capture() {
            return;
        }
        let raw = "1girl bell blonde_hair blush bound commentary_request hair_between_eyes hair_ornament hairclip hands_up highres jingle_bell leotard long_hair looking_at_viewer low-tied_long_hair open_mouth phoenix_reffaella red_ribbon ribbon sh1zuki simple_background solo white_background white_leotard yellow_eyes zenless_zone_zero";
        if let Some(media) = self.library.media.iter_mut().find(|m| m.id == 3) {
            media.tags = raw.split_whitespace().map(str::to_owned).collect();
            media.width = 1930;
            media.height = 1849;
        }
        for tag in raw.split_whitespace() {
            self.workflows.rules.categories.insert(tag.into(), 0);
        }
        self.workflows.rules.categories.extend([
            ("sh1zuki".into(), 1),
            ("phoenix_reffaella".into(), 4),
            ("zenless_zone_zero".into(), 3),
        ]);
        self.workflows.artifacts.insert(
            (3, "tags".into()),
            bdm_infrastructure::workflows::TextArtifact {
                media_id: 3,
                kind: "tags".into(),
                content: raw
                    .split_whitespace()
                    .map(|tag| {
                        if tag == "sh1zuki" {
                            format!("@artist {tag}")
                        } else {
                            tag.into()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                state: "complete".into(),
                reviewed: false,
                provenance: "synthetic-capture:tags:bdm-text-v1".into(),
            },
        );
    }

    pub(crate) fn show_media_review(&mut self, ctx: &egui::Context, id: usize) -> bool {
        let Some(media) = self.library.media.iter().find(|m| m.id == id).cloned() else {
            return true;
        };
        if self.media_review.canvas_id != Some(id) {
            self.clear_review_original();
            self.media_review.canvas_id = Some(id);
            self.media_review.view = CanvasView::default();
            self.form_error.clear();
            if self.prefs.auto_load_review_original {
                self.load_review_original(ctx, id);
            }
        }
        let screen = ctx.screen_rect();
        self.image_editor.reset(id);
        self.editor_poll(ctx);
        let size = screen.size() - vec2(24.0, 32.0);
        let mut close = false;
        let response = egui::Window::new("图片审核")
            .id(egui::Id::new("media_review_dialog"))
            .order(egui::Order::Foreground)
            .enabled(self.media_review.redownload_notice.is_none())
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .fixed_pos(screen.min)
            .fixed_size(size)
            .frame(egui::Frame::none().fill(NAV).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let layout = ReviewLayout::new(rect);
                #[cfg(test)]
                {
                    self.media_review.footer = Some(layout.footer);
                }
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(layout.header), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        ui.label(RichText::new("图片审核").font(medium(17.0)));
                        badge(
                            ui,
                            if self.pending_deletion.contains(&id) {
                                "待删除"
                            } else if media.keep {
                                "已保留"
                            } else {
                                "待审核"
                            },
                            if self.pending_deletion.contains(&id) {
                                DANGER
                            } else if media.keep {
                                ACCENT
                            } else {
                                AMBER
                            },
                        );
                        let name_width = (ui.available_width() - 290.0).clamp(80.0, 340.0);
                        ui.add_sized(
                            [name_width, 36.0],
                            egui::Label::new(RichText::new(self.media_name(id)).size(13.0))
                                .truncate(),
                        )
                        .on_hover_text(self.media_name(id));
                        ui.label(
                            RichText::new(format!("{} × {}", media.width, media.height))
                                .size(12.0)
                                .color(MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close |= icon_button(ui, "close", "结束审核 · Esc", false).clicked();
                            if !self.pending_deletion.is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!(
                                                "待删除 {}",
                                                self.pending_deletion.len()
                                            ))
                                            .color(DANGER),
                                        )
                                        .min_size(vec2(94.0, 36.0)),
                                    )
                                    .clicked()
                            {
                                self.open_deletion_review(Some(id));
                                close = true;
                            }
                        });
                    });
                });
                ui.painter().line_segment(
                    [layout.header.left_bottom(), layout.header.right_bottom()],
                    egui::Stroke::new(1.0_f32, LINE),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(layout.image), |ui| {
                    ui.set_clip_rect(layout.image);
                    self.review_canvas(ui, &media, layout.image);
                });
                ui.painter()
                    .rect(layout.info, 10.0, PANEL, egui::Stroke::new(1.0_f32, LINE));
                let info = layout.info.shrink(14.0);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(info), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.set_clip_rect(info);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let width = (ui.available_width() - 8.0) / 3.0;
                        if crate::review_ui::choice(
                            ui,
                            "标签与描述",
                            !self.image_editor.visible,
                            width,
                        )
                        .clicked()
                        {
                            self.image_editor.visible = false;
                        }
                        let busy =
                            self.image_editor.active() || self.image_editor.preview.is_some();
                        let button = ui
                            .add_enabled_ui(!busy || !self.image_editor.cropping, |ui| {
                                crate::review_ui::choice(
                                    ui,
                                    "局部修复",
                                    self.image_editor.visible && !self.image_editor.cropping,
                                    width,
                                )
                            })
                            .inner
                            .on_hover_text("Inpainting · 局部修复");
                        if button.clicked() {
                            self.image_editor.visible = true;
                            self.image_editor.cropping = false;
                        }
                        #[cfg(test)]
                        {
                            self.image_editor.editor_button = Some(button.rect);
                        }
                        #[cfg(not(test))]
                        let _ = button;
                        if ui
                            .add_enabled_ui(!busy || self.image_editor.cropping, |ui| {
                                crate::review_ui::choice(
                                    ui,
                                    "裁剪",
                                    self.image_editor.visible && self.image_editor.cropping,
                                    width,
                                )
                            })
                            .inner
                            .clicked()
                        {
                            self.image_editor.visible = true;
                            self.image_editor.cropping = true;
                        }
                    });
                    ui.add_space(12.0);
                    let height = (info.bottom() - ui.cursor().top()).max(1.0);
                    if self.image_editor.visible {
                        self.editor_panel(ui, id, height);
                    } else {
                        self.review_info(ui, &media, height);
                    }
                });
                ui.painter().line_segment(
                    [layout.footer.left_top(), layout.footer.right_top()],
                    egui::Stroke::new(1.0_f32, LINE),
                );
                let mut decision = None;
                let mut navigation = None;
                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(layout.footer.shrink2(vec2(0.0, 8.0))),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.horizontal(|ui| {
                            for (value, label) in [(1, "待删除"), (2, "待审核"), (3, "保留")]
                            {
                                let selected = match value {
                                    1 => self.pending_deletion.contains(&id),
                                    3 => media.keep && !self.pending_deletion.contains(&id),
                                    _ => !media.keep && !self.pending_deletion.contains(&id),
                                };
                                let button = crate::review_ui::decision(
                                    ui,
                                    label,
                                    &value.to_string(),
                                    selected,
                                    value == 3,
                                    value == 1,
                                );
                                #[cfg(test)]
                                if value == 3 {
                                    self.media_review.keep_button = Some(button.rect);
                                }
                                if button.clicked() {
                                    decision = Some(value);
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icon_button(ui, "forward", "下一张 · →", false).clicked()
                                    {
                                        navigation = Some(true);
                                    }
                                    let position = self
                                        .review_order
                                        .iter()
                                        .position(|candidate| *candidate == id);
                                    ui.add_sized(
                                        [84.0, 40.0],
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "{} / {}",
                                                position.map_or(0, |p| p + 1),
                                                self.review_order.len()
                                            ))
                                            .size(13.0),
                                        ),
                                    );
                                    if icon_button(ui, "back", "上一张 · ←", false).clicked()
                                    {
                                        navigation = Some(false);
                                    }
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            if self
                                .ingest
                                .stored
                                .get(&id)
                                .is_some_and(|stored| stored.edit.is_some())
                                && ui
                                    .button("恢复编辑前版本")
                                    .on_hover_text(
                                        "撤销最近一次裁剪或去水印，从编辑前文件重新开始。",
                                    )
                                    .clicked()
                            {
                                if let Err(error) = self.restore_last_edit(ui.ctx(), id) {
                                    self.form_error = error;
                                }
                            }
                            ui.checkbox(&mut self.media_review.auto_advance, "判断后自动下一张");
                            ui.label(
                                RichText::new("← → 切图    滚轮缩放    双击适应")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                    },
                );
                if self.media_review.redownload_notice.is_none()
                    && !ctx.wants_keyboard_input()
                    && !ctx.input(|i| i.modifiers.any())
                {
                    ctx.input_mut(|i| {
                        for (key, value) in [
                            (egui::Key::Num1, 1),
                            (egui::Key::Delete, 1),
                            (egui::Key::Num2, 2),
                            (egui::Key::Num3, 3),
                        ] {
                            if i.consume_key(egui::Modifiers::NONE, key) {
                                decision = Some(value);
                            }
                        }
                        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                            navigation = Some(false);
                        }
                        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                            navigation = Some(true);
                        }
                        if i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
                            && !self.selected_media.remove(&id)
                        {
                            self.selected_media.insert(id);
                        }
                    });
                }
                if self.media_review.redownload_notice.is_none()
                    && !ctx.wants_keyboard_input()
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A))
                {
                    self.selected_media
                        .extend(self.review_order.iter().copied());
                }
                if let Some(value) = decision {
                    let success = if value == 1 {
                        self.reject_media(id);
                        true
                    } else {
                        self.set_review_keep(id, value == 3)
                    };
                    if success && self.media_review.auto_advance {
                        // Decisions stop at the last item instead of wrapping and silently restarting a batch.
                        if self.review_order.last() == Some(&id) {
                            close = true;
                        } else {
                            navigation = Some(true);
                        }
                    }
                }
                if let Some(forward) = navigation {
                    if let Some(neighbor) = self.review_neighbor(id, forward) {
                        self.dialog = Some(Dialog::Media(neighbor));
                        close = true;
                    }
                }
            });
        #[cfg(test)]
        {
            self.media_review.window = response.map(|r| r.response.rect);
        }
        #[cfg(not(test))]
        let _ = response;
        close
    }

    fn review_canvas(&mut self, ui: &mut egui::Ui, media: &Media, rect: Rect) {
        let stage = Rect::from_min_max(rect.min + vec2(0.0, 72.0), rect.max);
        let texture = self.review_texture(ui.ctx(), media.id);
        let preview = if self.image_editor.visible && self.image_editor.show_preview {
            self.image_editor.texture.clone()
        } else {
            None
        };
        let dimensions = preview.as_ref().or(texture.as_ref()).map_or(
            vec2(media.width.max(1) as f32, media.height.max(1) as f32),
            |t| t.size_vec2(),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().button_padding = vec2(10.0, 5.0);
            ui.spacing_mut().interact_size.y = 28.0;
            let open = ui.add_enabled(
                !self.demo
                    && !self.is_capture()
                    && self
                        .ingest
                        .stored
                        .get(&media.id)
                        .is_some_and(|m| m.path.is_file()),
                egui::Button::new(RichText::new("打开原图").size(12.0)).fill(PANEL),
            );
            #[cfg(test)]
            {
                self.image_editor.open_button = Some(open.rect);
            }
            if open.clicked() {
                if let Err(error) = self.open_original(media.id) {
                    if self.image_editor.visible {
                        self.image_editor.error = error;
                    } else {
                        self.form_error = error;
                    }
                }
            }
            if ui.button("适应窗口").clicked() {
                self.media_review.view = CanvasView::default();
            }
            if ui
                .button("100%")
                .on_hover_text("按屏幕物理像素查看原图")
                .clicked()
            {
                self.media_review.view.zoom = 1.0
                    / (self.media_review.view.fit(stage, dimensions) * ui.ctx().pixels_per_point());
                self.media_review.view.pan = egui::Vec2::ZERO;
            }
            for (label, factor) in [("−", 1.0 / 1.25), ("+", 1.25)] {
                if ui.button(label).clicked() {
                    self.media_review
                        .view
                        .zoom_at(stage, stage.center(), factor);
                }
            }
            let scale = self.media_review.view.fit(stage, dimensions)
                * self.media_review.view.zoom
                * ui.ctx().pixels_per_point();
            ui.label(
                RichText::new(format!("{:.0}%", scale * 100.0))
                    .size(12.0)
                    .color(MUTED),
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 24.0;
            let requested = self.ingest.detail_requested == Some(media.id);
            let load = ui
                .add_enabled(
                    !self.demo
                        && !self.is_capture()
                        && self
                            .ingest
                            .stored
                            .get(&media.id)
                            .is_some_and(|m| m.path.is_file()),
                    egui::Button::new(if requested {
                        "使用缩略图"
                    } else {
                        "加载原图"
                    }),
                )
                .on_hover_text("默认显示缩略图；加载原图后可检查完整细节。");
            #[cfg(test)]
            {
                self.media_review.load_original_button = Some(load.rect);
            }
            if load.clicked() {
                if requested {
                    self.clear_review_original();
                } else {
                    self.load_review_original(ui.ctx(), media.id);
                }
            }
            if requested && texture.is_none() {
                if let Some((_, error)) = self
                    .ingest
                    .detail_error
                    .as_ref()
                    .filter(|(failed, _)| *failed == media.id)
                {
                    ui.label(RichText::new("原图加载失败").color(DANGER))
                        .on_hover_text(error);
                    if ui.button("重试").clicked() {
                        self.load_review_original(ui.ctx(), media.id);
                    }
                } else {
                    ui.spinner();
                    ui.label(RichText::new("正在加载原图").size(12.0).color(MUTED));
                }
            } else {
                ui.label(
                    RichText::new(if requested {
                        "原图预览"
                    } else {
                        "缩略图预览"
                    })
                    .size(12.0)
                    .color(MUTED),
                );
            }
        });
        ui.painter().rect(
            stage,
            8.0,
            egui::Color32::from_rgb(237, 240, 244),
            egui::Stroke::new(1.0_f32, LINE),
        );
        let response = ui.interact(
            stage,
            ui.id().with(("review-canvas", media.id)),
            Sense::click_and_drag(),
        );
        #[cfg(test)]
        {
            self.media_review.canvas = Some(stage);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let pointer = response.hover_pos().unwrap_or(stage.center());
                self.media_review
                    .view
                    .zoom_at(stage, pointer, (scroll * 0.005).exp());
            }
            let pinch = ui.input(|i| i.zoom_delta());
            if (pinch - 1.0).abs() > f32::EPSILON {
                self.media_review.view.zoom_at(
                    stage,
                    response.hover_pos().unwrap_or(stage.center()),
                    pinch,
                );
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let selecting = self.image_editor.visible
            && (self.image_editor.cropping
                || self.image_editor.tool != crate::image_editor::Tool::Pan)
            && self.image_editor.preview.is_none();
        if response.dragged_by(egui::PointerButton::Secondary)
            || (!selecting && response.dragged_by(egui::PointerButton::Primary))
        {
            self.media_review.view.pan += ui.input(|i| i.pointer.delta());
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.double_clicked() && !selecting {
            self.media_review.view = CanvasView::default();
        }
        self.media_review.view.constrain(stage, dimensions);
        let image_rect = self.media_review.view.image_rect(stage, dimensions);
        let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(stage));
        canvas_ui.set_clip_rect(stage);
        if let Some(texture) = preview.or(texture) {
            canvas_ui.painter().image(
                texture.id(),
                image_rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            self.paint_media(&canvas_ui, image_rect, media.id);
        }
        self.editor_canvas(&canvas_ui, &response, image_rect, dimensions);
    }
    fn review_info(&mut self, ui: &mut egui::Ui, media: &Media, height: f32) {
        let id = media.id;
        egui::Frame::none()
            .fill(NAV)
            .rounding(6.0)
            .inner_margin(3.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let width = (ui.available_width() - 6.0) / 3.0;
                ui.horizontal(|ui| {
                    for (index, label) in ["原始标签", "优化标签", "自然语言"].iter().enumerate()
                    {
                        let selected = self.media_review.tab == index;
                        if ui
                            .add_sized(
                                [width, 30.0],
                                egui::Button::new(
                                    RichText::new(*label).size(13.0).color(if selected {
                                        INK
                                    } else {
                                        MUTED
                                    }),
                                )
                                .fill(if selected {
                                    PANEL
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::NONE)
                                .rounding(4.0),
                            )
                            .clicked()
                        {
                            self.media_review.tab = index;
                        }
                    }
                });
            });
        ui.add_space(8.0);
        let tab = self.media_review.tab;
        let kind = if tab == 2 { "caption" } else { "tags" };
        let artifact = self.review_output(id, tab);
        let content = self.review_text(media, tab);
        let (empty_title, empty_hint) = self.review_empty_message(id, tab);
        let groups = if tab == 0 {
            raw_tag_groups(
                &content,
                self.ingest.stored.get(&id).map(|s| &s.post),
                &self.workflows.rules,
            )
        } else {
            grouped_tags(&content, &self.workflows.rules)
        };
        let needs_confirmation = tab != 0
            && !self.demo
            && !self.is_capture()
            && artifact.as_ref().is_some_and(|a| !a.reviewed);
        let error_height = if self.form_error.is_empty() {
            0.0
        } else {
            64.0
        };
        let bottom = ui.max_rect().top() + height;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if tab == 0 {
                    "来源标签"
                } else if tab == 2 {
                    "自然语言"
                } else {
                    "优化标签"
                })
                .size(12.0)
                .color(MUTED),
            );
            if artifact.as_ref().is_some_and(|a| a.reviewed) && tab != 0 {
                badge(ui, "已确认", ACCENT);
            } else if artifact.is_some() && tab != 0 {
                badge(ui, "待确认", AMBER);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !content.is_empty(),
                        egui::Button::new(RichText::new("复制").size(12.0))
                            .min_size(vec2(48.0, 28.0)),
                    )
                    .clicked()
                {
                    ui.output_mut(|o| o.copied_text = content.clone());
                }
                if tab != 2 && !content.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} 个标签",
                            groups.iter().map(|(_, tags)| tags.len()).sum::<usize>()
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                }
            });
        });
        ui.add_space(8.0);
        let body_height = (bottom
            - ui.cursor().top()
            - if needs_confirmation { 48.0 } else { 0.0 }
            - error_height)
            .max(1.0);
        egui::ScrollArea::vertical()
            .id_salt(("review-text", id, tab))
            .max_height(body_height)
            .min_scrolled_height(body_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if content.is_empty() {
                    ui.add_space((body_height * 0.23).max(12.0));
                    ui.vertical_centered(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        icon(ui, rect.shrink(6.0), "tag", MUTED);
                        ui.add_space(10.0);
                        ui.label(RichText::new(empty_title).font(medium(15.0)));
                        ui.label(RichText::new(empty_hint).size(12.0).color(MUTED));
                    });
                } else if tab == 0 {
                    crate::review_ui::section().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for index in [1, 2] {
                            let (name, tags) = &groups[index];
                            ui.horizontal_top(|ui| {
                                ui.add_sized(
                                    [52.0, 26.0],
                                    egui::Label::new(RichText::new(*name).size(12.0).color(MUTED)),
                                );
                                ui.vertical(|ui| {
                                    if tags.is_empty() {
                                        ui.label(RichText::new("无").size(12.0).color(MUTED));
                                    } else {
                                        raw_review_tags(ui, tags, Some(index - 1));
                                    }
                                });
                            });
                        }
                    });
                    ui.add_space(14.0);
                    for index in [0, 3, 4, 5] {
                        let (name, tags) = &groups[index];
                        if tags.is_empty() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*name).font(medium(13.0)));
                            ui.label(
                                RichText::new(tags.len().to_string())
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(6.0);
                        if tags.is_empty() {
                            ui.label(RichText::new("无").size(12.0).color(MUTED));
                        } else {
                            raw_review_tags(
                                ui,
                                tags,
                                match index {
                                    1 => Some(0),
                                    2 => Some(1),
                                    _ => None,
                                },
                            );
                        }
                        ui.add_space(12.0);
                    }
                } else if tab == 2 {
                    egui::Frame::none()
                        .fill(NAV)
                        .rounding(6.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(RichText::new(&content).size(13.0))
                                    .wrap()
                                    .selectable(true),
                            );
                        });
                } else {
                    if groups.iter().take(3).any(|(_, tags)| !tags.is_empty()) {
                        egui::Frame::none()
                            .fill(NAV)
                            .rounding(6.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 10.0;
                                for (index, (name, tags)) in groups.iter().take(3).enumerate() {
                                    if tags.is_empty() {
                                        continue;
                                    }
                                    ui.horizontal_top(|ui| {
                                        ui.add_sized(
                                            [36.0, 22.0],
                                            egui::Label::new(
                                                RichText::new(*name).size(12.0).color(MUTED),
                                            ),
                                        );
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                                            review_tags(ui, tags, Some(index));
                                        });
                                    });
                                }
                            });
                        ui.add_space(16.0);
                    }
                    for (name, tags) in groups.iter().skip(3) {
                        if tags.is_empty() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*name).font(medium(13.0)));
                            ui.label(
                                RichText::new(tags.len().to_string())
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(6.0);
                        review_tags(ui, tags, None);
                        ui.add_space(16.0);
                    }
                }
                if let Some(stored) = self.ingest.stored.get(&id).filter(|_| tab != 2) {
                    let warnings = bdm_domain::tags::normalize(
                        &stored.post.tag_string,
                        &stored.post.tag_string_artist,
                        &stored.post.tag_string_character,
                        &self.workflows.rules,
                    )
                    .warnings;
                    if !warnings.is_empty() {
                        ui.separator();
                        egui::CollapsingHeader::new(format!("标签提示 · {}", warnings.len())).show(
                            ui,
                            |ui| {
                                for warning in warnings {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(warning).size(12.0).color(AMBER),
                                        )
                                        .wrap(),
                                    );
                                }
                            },
                        );
                    }
                }
            });
        if !self.form_error.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("review-error")
                .max_height(56.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&self.form_error).size(12.0).color(DANGER))
                            .wrap(),
                    );
                });
        }
        if needs_confirmation {
            ui.add_space(10.0);
        }
        if needs_confirmation
            && ui
                .button(if tab == 2 {
                    "确认自然语言内容"
                } else {
                    "确认优化标签"
                })
                .clicked()
        {
            let result = self
                .catalog
                .path
                .as_deref()
                .ok_or_else(|| "本地数据库未打开。".to_owned())
                .and_then(|p| {
                    bdm_infrastructure::catalog::SqliteCatalog::open(p).map_err(|e| e.to_string())
                })
                .and_then(|mut repo| {
                    repo.review_artifact(id as i64, kind)
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = result {
                self.form_error = error;
            } else {
                let _ = self.reload_workflow_data();
            }
        }
    }

    pub(crate) fn review_output(
        &self,
        id: usize,
        tab: usize,
    ) -> Option<bdm_infrastructure::workflows::TextArtifact> {
        if tab == 0 {
            return None;
        }
        self.workflows
            .artifacts
            .get(&(id, if tab == 2 { "caption" } else { "tags" }.into()))
            .filter(|a| {
                a.provenance != "local-normalization-v1"
                    && !a.provenance.is_empty()
                    && matches!(a.state.as_str(), "complete" | "needs_review")
                    && !a.content.trim().is_empty()
            })
            .cloned()
    }

    pub(crate) fn review_text(&self, media: &Media, tab: usize) -> String {
        if tab == 0 {
            return self
                .ingest
                .stored
                .get(&media.id)
                .map(|m| m.post.tag_string.clone())
                .unwrap_or_else(|| {
                    if self.demo || self.is_capture() {
                        media.tags.join(" ")
                    } else {
                        String::new()
                    }
                });
        }
        if let Some(artifact) = self.review_output(media.id, tab) {
            return artifact.content;
        }
        if self.demo && !self.is_capture() {
            if tab == 1 && media.optimized {
                return media.tags.join(", ");
            }
            if tab == 2 && media.caption {
                return "A landscape with layered mountains and a warm sun.".into();
            }
        }
        String::new()
    }

    fn review_empty_message(&self, id: usize, tab: usize) -> (&'static str, &'static str) {
        if tab == 0 {
            return ("暂无原始标签", "没有可用的来源标签。");
        }
        let mode = if tab == 2 { "caption" } else { "tags" };
        let job = self
            .workflows
            .jobs
            .iter()
            .filter(|j| j.kind == "optimize" && j.mode == mode && j.ids.contains(&(id as i64)))
            .max_by_key(|j| j.id);
        if let Some(job) = job {
            return match job.state.as_str() {
                "running" | "queued" => (
                    "优化任务进行中",
                    "当前图片尚无可用结果，可在任务区查看进度。",
                ),
                "paused" | "interrupted" => {
                    ("优化任务已暂停", "当前图片尚无可用结果，可在任务区继续。")
                }
                "failed" | "deferred" => ("优化未产生可用结果", "请在任务区查看错误并重试。"),
                _ => (
                    "暂无优化结果",
                    "该目标已有任务记录，但当前图片没有可用结果。",
                ),
            };
        }
        (
            if tab == 2 {
                "尚未优化自然语言"
            } else {
                "尚未优化标签"
            },
            "在图库点击“优化”，选择对应目标后启动。",
        )
    }

    fn set_review_keep(&mut self, id: usize, keep: bool) -> bool {
        if !self.demo && !self.is_capture() {
            let result = self
                .catalog
                .path
                .as_deref()
                .ok_or_else(|| "本地数据库未打开。".to_owned())
                .and_then(|p| {
                    bdm_infrastructure::catalog::SqliteCatalog::open(p).map_err(|e| e.to_string())
                })
                .and_then(|mut repo| {
                    repo.set_media_keep(id as i64, keep)
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = result {
                self.form_error = error;
                return false;
            }
        }
        self.pending_deletion.remove(&id);
        if let Some(media) = self.library.media.iter_mut().find(|m| m.id == id) {
            media.keep = keep;
        }
        self.log("INFO", "已修改图片审核状态。");
        true
    }
}

fn review_tags(ui: &mut egui::Ui, tags: &[String], category: Option<usize>) {
    tag_chips(ui, tags, category, false);
}
fn raw_review_tags(ui: &mut egui::Ui, tags: &[String], category: Option<usize>) {
    tag_chips(ui, tags, category, true);
}
fn tag_chips(ui: &mut egui::Ui, tags: &[String], category: Option<usize>, raw: bool) {
    let color = match category {
        Some(0) => egui::Color32::from_rgb(69, 92, 127),
        Some(1) => egui::Color32::from_rgb(114, 82, 131),
        Some(2) => egui::Color32::from_rgb(137, 104, 55),
        _ => INK,
    };
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.spacing_mut().interact_size.y = 26.0;
        let max_width = (ui.available_width() - 14.0).max(1.0);
        ui.horizontal_wrapped(|ui| {
            for tag in tags {
                // Decoration is display-only; copy/export keep the original artifact.
                let label = if raw {
                    tag.clone()
                } else {
                    tag.strip_prefix("@artist ")
                        .unwrap_or(tag)
                        .replace('_', " ")
                };
                let galley =
                    ui.painter()
                        .layout(label, egui::FontId::proportional(13.0), color, max_width);
                let (rect, _) =
                    ui.allocate_exact_size(galley.size() + vec2(14.0, 8.0), Sense::hover());
                ui.painter().rect(
                    rect,
                    4.0,
                    if category.is_some() { PANEL } else { NAV },
                    egui::Stroke::new(1.0_f32, LINE),
                );
                let mut label_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(vec2(7.0, 4.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                label_ui
                    .add(egui::Label::new(galley).selectable(true))
                    .on_hover_text(tag);
            }
        });
    });
}

fn grouped_tags(
    content: &str,
    rules: &bdm_domain::tags::TagRules,
) -> Vec<(&'static str, Vec<String>)> {
    let mut groups: Vec<_> = ["画师", "角色", "作品", "普通标签", "其他标签"]
        .into_iter()
        .map(|name| (name, Vec::new()))
        .collect();
    for tag in content
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        let name = tag.strip_prefix("@artist ").unwrap_or(tag);
        let category = rules
            .categories
            .get(name)
            .or_else(|| rules.categories.get(&name.replace(' ', "_")));
        let group = if tag.starts_with("@artist ") || category == Some(&1) {
            0
        } else if category == Some(&4) {
            1
        } else if category == Some(&3) {
            2
        } else if category == Some(&0) {
            3
        } else {
            4
        };
        groups[group].1.push(tag.to_owned());
    }
    groups
}

fn raw_tag_groups(
    content: &str,
    post: Option<&bdm_domain::ingest::SourcePost>,
    rules: &bdm_domain::tags::TagRules,
) -> Vec<(&'static str, Vec<String>)> {
    let mut groups: Vec<_> = [
        "通用标签",
        "画师标签",
        "角色标签",
        "作品标签",
        "元标签",
        "其他标签",
    ]
    .into_iter()
    .map(|name| (name, Vec::new()))
    .collect();
    let sets: Vec<std::collections::BTreeSet<&str>> = post
        .map(|p| {
            [
                &p.tag_string_general,
                &p.tag_string_artist,
                &p.tag_string_character,
                &p.tag_string_copyright,
                &p.tag_string_meta,
            ]
            .into_iter()
            .map(|s| s.split_whitespace().collect())
            .collect()
        })
        .unwrap_or_default();
    for tag in content.split_whitespace() {
        let group = if post.is_some() {
            // Source categories take priority; preserve uncategorized tokens verbatim.
            [1, 2, 3, 4, 0]
                .into_iter()
                .find(|i| sets[*i].contains(tag))
                .unwrap_or(5)
        } else {
            match rules.categories.get(tag) {
                Some(1) => 1,
                Some(4) => 2,
                Some(3) => 3,
                Some(5) => 4,
                _ => 0,
            }
        };
        groups[group].1.push(tag.to_owned());
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Preferences;
    use bdm_infrastructure::workflows::TextArtifact;

    fn sample() -> BdmApp {
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.open_media(3);
        app.media_review.auto_advance = false;
        app.library
            .media
            .iter_mut()
            .find(|m| m.id == 3)
            .unwrap()
            .tags = (0..240).map(|i| format!("tag_{i:03}")).collect();
        for (kind, content) in [
            (
                "tags",
                (0..240)
                    .map(|i| format!("tag_{i:03}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                "caption",
                "A scene with several characters and detailed costumes. ".repeat(120),
            ),
        ] {
            app.workflows.artifacts.insert(
                (3, kind.into()),
                TextArtifact {
                    media_id: 3,
                    kind: kind.into(),
                    content,
                    state: "complete".into(),
                    reviewed: false,
                    provenance: "synthetic-test:model:job-1:bdm-text-v1".into(),
                },
            );
        }
        app
    }

    fn frame(ctx: &egui::Context, app: &mut BdmApp, size: egui::Vec2, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.show_dialog(ctx),
        );
    }

    fn key(ctx: &egui::Context, app: &mut BdmApp, key: egui::Key) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec2(1050.0, 720.0),
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    }

    #[test]
    fn empty_optimization_tabs_follow_target_job_history_and_ignore_stale_flags() {
        let mut app = sample();
        app.demo = false;
        app.workflows.artifacts.clear();
        let mut media = app
            .library
            .media
            .iter()
            .find(|m| m.id == 3)
            .unwrap()
            .clone();
        media.optimized = true;
        media.caption = true;
        assert_eq!(app.review_text(&media, 0), ""); // No source provenance available.
        assert_eq!(app.review_text(&media, 1), "");
        assert_eq!(app.review_text(&media, 2), "");
        assert_eq!(app.review_empty_message(3, 1).0, "尚未优化标签");
        app.workflows
            .jobs
            .push(bdm_infrastructure::workflows::WorkflowJob {
                id: 2,
                kind: "optimize".into(),
                ids: vec![3],
                mode: "tags".into(),
                state: "running".into(),
                ..Default::default()
            });
        assert_eq!(app.review_empty_message(3, 1).0, "优化任务进行中");
        assert_eq!(app.review_empty_message(3, 2).0, "尚未优化自然语言");
        for (state, title) in [
            ("paused", "优化任务已暂停"),
            ("failed", "优化未产生可用结果"),
            ("completed", "暂无优化结果"),
        ] {
            app.workflows.jobs[0].state = state.into();
            assert_eq!(app.review_empty_message(3, 1).0, title);
        }
        for (content, state, provenance) in [
            ("local tags", "complete", "local-normalization-v1"),
            (" ", "complete", "synthetic-test"),
            ("partial text", "failed", "synthetic-test"),
            ("unknown text", "complete", ""),
        ] {
            app.workflows.artifacts.insert(
                (3, "tags".into()),
                TextArtifact {
                    media_id: 3,
                    kind: "tags".into(),
                    content: content.into(),
                    state: state.into(),
                    reviewed: false,
                    provenance: provenance.into(),
                },
            );
            assert_eq!(app.review_text(&media, 1), "");
        }
    }

    #[test]
    fn keyboard_decisions_advance_in_snapshot_and_last_rejection_requires_confirmation() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        app.review_order = vec![3, 8];
        app.media_review.auto_advance = true;
        frame(&ctx, &mut app, vec2(1050.0, 720.0), Vec::new());
        key(&ctx, &mut app, egui::Key::Num3);
        assert!(app.library.media.iter().find(|m| m.id == 3).unwrap().keep);
        assert!(matches!(app.dialog, Some(Dialog::Media(8))));
        let media = app.library.media.clone();
        key(&ctx, &mut app, egui::Key::Num1);
        assert!(matches!(
            app.dialog,
            Some(Dialog::Delete {
                return_media: Some(8)
            })
        ));
        assert_eq!(app.library.media, media);
        assert_eq!(app.pending_deletion, std::collections::BTreeSet::from([8]));
        key(&ctx, &mut app, egui::Key::Escape);
        assert_eq!(app.library.media, media);
        assert!(app.pending_deletion.is_empty());
        key(&ctx, &mut app, egui::Key::Num3);
        assert!(
            app.dialog.is_none(),
            "last kept image completes the session"
        );
    }

    #[test]
    fn failed_review_save_does_not_advance_or_clear_deletion_mark() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        app.demo = false;
        app.review_order = vec![3, 8];
        app.media_review.auto_advance = true;
        app.reject_media(3);
        frame(&ctx, &mut app, vec2(1050.0, 720.0), Vec::new());
        key(&ctx, &mut app, egui::Key::Num3);
        assert!(matches!(app.dialog, Some(Dialog::Media(3))));
        assert!(app.pending_deletion.contains(&3));
        assert!(!app.form_error.is_empty());
    }

    #[test]
    fn zoom_keeps_pointer_pixel_fixed_and_pan_cannot_lose_image() {
        let stage = Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 400.0));
        let dimensions = vec2(1200.0, 800.0);
        let pointer = egui::pos2(220.0, 170.0);
        let mut view = CanvasView::default();
        let before = (pointer - view.image_rect(stage, dimensions).min)
            / view.image_rect(stage, dimensions).size();
        view.zoom_at(stage, pointer, 4.0);
        let after = (pointer - view.image_rect(stage, dimensions).min)
            / view.image_rect(stage, dimensions).size();
        assert!((before - after).length() < 0.0001);
        view.pan = vec2(10000.0, -10000.0);
        view.constrain(stage, dimensions);
        assert!(view.image_rect(stage, dimensions).contains_rect(stage));
        view.zoom = 0.5;
        view.constrain(stage, dimensions);
        assert_eq!(view.pan, egui::Vec2::ZERO);
    }

    #[test]
    fn canvas_scroll_drag_and_image_switch_change_only_viewport() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        let size = vec2(1050.0, 720.0);
        for _ in 0..3 {
            frame(&ctx, &mut app, size, Vec::new());
        }
        let source = app.library.media.clone();
        let pointer = app.media_review.canvas.unwrap().center();
        frame(
            &ctx,
            &mut app,
            size,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 200.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(app.media_review.view.zoom > 1.0);
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::PointerMoved(pointer + vec2(30.0, 20.0))],
        );
        assert!(app.media_review.view.pan.length() > 0.0);
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::PointerButton {
                pos: pointer + vec2(30.0, 20.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        key(&ctx, &mut app, egui::Key::ArrowRight);
        assert!(matches!(app.dialog, Some(Dialog::Media(2))));
        assert_eq!(app.media_review.view.zoom, 1.0);
        assert_eq!(app.media_review.view.pan, egui::Vec2::ZERO);
        assert_eq!(app.library.media, source);
    }

    #[test]
    fn long_text_tabs_keep_review_window_and_action_inside_small_and_large_viewports() {
        for size in [vec2(981.0, 613.0), vec2(1050.0, 720.0), vec2(1440.0, 960.0)] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = sample();
            let source = app.library.media.clone();
            for tab in 0..3 {
                assert!(!app
                    .review_text(
                        &app.library
                            .media
                            .iter()
                            .find(|m| m.id == 3)
                            .unwrap()
                            .clone(),
                        tab
                    )
                    .is_empty());
                app.media_review.tab = tab;
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, Vec::new());
                }
                let window = app.media_review.window.unwrap();
                let screen = Rect::from_min_size(egui::Pos2::ZERO, size);
                assert!(
                    screen.contains_rect(window),
                    "tab={tab} window={window:?} screen={screen:?}"
                );
                assert!(window.contains_rect(app.media_review.footer.unwrap()));
                assert!(window.contains_rect(app.media_review.canvas.unwrap()));
                assert!(window.contains_rect(app.media_review.load_original_button.unwrap()));
                assert!(app.media_review.canvas.unwrap().width() > size.x * 0.60);
                assert!(app
                    .media_review
                    .footer
                    .unwrap()
                    .contains_rect(app.media_review.keep_button.unwrap()));
                assert_eq!(app.library.media, source);
                assert!(matches!(app.dialog, Some(Dialog::Media(3))));
            }
        }
    }

    #[test]
    fn inpainting_entry_and_manual_selection_keep_review_context_at_supported_sizes() {
        for size in [vec2(981.0, 613.0), vec2(1050.0, 720.0), vec2(1440.0, 960.0)] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = sample();
            for _ in 0..3 {
                frame(&ctx, &mut app, size, Vec::new());
            }
            let entry = app.image_editor.editor_button.unwrap().center();
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut app,
                    size,
                    vec![
                        egui::Event::PointerMoved(entry),
                        egui::Event::PointerButton {
                            pos: entry,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(app.image_editor.visible);
            assert!(matches!(app.dialog, Some(Dialog::Media(3))));
            assert!(app
                .media_review
                .window
                .unwrap()
                .contains_rect(app.image_editor.open_button.unwrap()));
            app.image_editor.tool = crate::image_editor::Tool::Rectangle;
            let canvas = app.media_review.canvas.unwrap();
            let start = canvas.center() - vec2(35.0, 25.0);
            let end = canvas.center() + vec2(35.0, 25.0);
            frame(
                &ctx,
                &mut app,
                size,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            frame(&ctx, &mut app, size, vec![egui::Event::PointerMoved(end)]);
            frame(
                &ctx,
                &mut app,
                size,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(app.image_editor.regions.len(), 1);
            assert_eq!(app.media_review.view.pan, egui::Vec2::ZERO);
            key(&ctx, &mut app, egui::Key::ArrowRight);
            frame(&ctx, &mut app, size, Vec::new());
            assert!(app.image_editor.regions.is_empty());
            assert!(app.open_original(2).is_err()); // Synthetic capture must never launch the OS viewer.
        }
    }

    #[test]
    fn keep_action_is_clickable_below_long_text_and_cancels_only_current_deletion_mark() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        app.reject_media(3);
        app.reject_media(8);
        let size = vec2(1050.0, 720.0);
        for _ in 0..3 {
            frame(&ctx, &mut app, size, Vec::new());
        }
        let pos = app.media_review.keep_button.unwrap().center();
        frame(
            &ctx,
            &mut app,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.library.media.iter().find(|m| m.id == 3).unwrap().keep);
        assert!(!app.pending_deletion.contains(&3));
        assert!(app.pending_deletion.contains(&8));
    }

    #[test]
    fn tag_chips_share_a_baseline_and_wrap_long_names_inside_the_panel() {
        for width in [220.0, 380.0] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let tags = vec![
                "solo".into(),
                "blue_hair".into(),
                "long_hair".into(),
                "x".repeat(300),
            ];
            let mut bounds = Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 600.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        bounds = ui.available_rect_before_wrap();
                        review_tags(ui, &tags, None);
                    });
                },
            );
            let chips: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == NAV => Some(rect.rect),
                    _ => None,
                })
                .collect();
            assert_eq!(chips.len(), tags.len());
            assert!(
                (chips[0].top() - chips[1].top()).abs() < 0.5,
                "chips drift between rows: {chips:?}"
            );
            assert!(
                chips.iter().all(|rect| bounds.contains_rect(*rect)),
                "chips overflow {bounds:?}: {chips:?}"
            );
            assert!(chips[3].top() > chips[0].bottom());
        }
    }

    #[test]
    fn original_tag_groups_use_source_categories_without_losing_tokens_or_copy_text() {
        let post = bdm_domain::ingest::SourcePost {
            id: 3,
            md5: String::new(),
            file_url: None,
            file_ext: "png".into(),
            file_size: 1,
            image_width: 800,
            image_height: 800,
            tag_string: "solo  blue_hair artist character work highres unknown".into(),
            tag_string_general: "solo blue_hair".into(),
            tag_string_artist: "artist".into(),
            tag_string_character: "character".into(),
            tag_string_copyright: "work".into(),
            tag_string_meta: "highres".into(),
            rating: "g".into(),
            is_deleted: false,
        };
        let mut app = sample();
        app.ingest.stored.insert(
            3,
            bdm_domain::ingest::StoredMedia {
                id: 3,
                post: post.clone(),
                path: Default::default(),
                thumbnail: Default::default(),
                sha256: String::new(),
                source_sha256: String::new(),
                collections: Default::default(),
                keep: false,
                minimum_short_side: 800,
                flatten_transparency: true,
                edit: None,
            },
        );
        let media = app.library.media.iter().find(|m| m.id == 3).unwrap();
        assert_eq!(app.review_text(media, 0), post.tag_string);
        let groups = raw_tag_groups(&post.tag_string, Some(&post), &app.workflows.rules);
        assert_eq!(groups[0].1, vec!["solo", "blue_hair"]);
        for (index, tag) in [
            (1, "artist"),
            (2, "character"),
            (3, "work"),
            (4, "highres"),
            (5, "unknown"),
        ] {
            assert_eq!(groups[index].1, vec![tag]);
        }
        assert_eq!(groups.iter().map(|(_, v)| v.len()).sum::<usize>(), 7);
    }
    #[test]
    fn crop_drag_moves_and_resizes_selection_without_panning_review_canvas() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        let size = vec2(1050.0, 720.0);
        frame(&ctx, &mut app, size, Vec::new());
        app.image_editor.visible = true;
        app.image_editor.cropping = true;
        frame(&ctx, &mut app, size, Vec::new());
        let center = app.media_review.canvas.unwrap().center();
        let mut drag = |start: egui::Pos2, end: egui::Pos2| {
            frame(
                &ctx,
                &mut app,
                size,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            frame(&ctx, &mut app, size, vec![egui::Event::PointerMoved(end)]);
            frame(
                &ctx,
                &mut app,
                size,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            app.image_editor.crop.selection.unwrap()
        };
        let selection = drag(center - vec2(60.0, 50.0), center + vec2(60.0, 50.0));
        let moved = drag(center, center + vec2(15.0, 10.0));
        assert!((selection.size() - moved.size()).length() < 0.001);
        assert!(moved.min.x > selection.min.x);
        let resized = drag(center + vec2(75.0, 60.0), center + vec2(100.0, 80.0));
        assert!(resized.width() > moved.width());
        assert_eq!(app.media_review.view.pan, egui::Vec2::ZERO);
        app.open_media(2);
        frame(&ctx, &mut app, size, Vec::new());
        assert!(app.image_editor.crop.selection.is_none());
    }
    #[test]
    fn grouped_display_preserves_all_terms_and_recognizes_spaced_ordinary_labels() {
        let mut rules = bdm_domain::tags::TagRules::default();
        rules.categories.extend([
            ("artist".into(), 1),
            ("character".into(), 4),
            ("series".into(), 3),
            ("blue_hair".into(), 0),
        ]);
        let groups = grouped_tags(
            "@artist artist, character, series, blue hair, special_term",
            &rules,
        );
        assert_eq!(
            groups
                .iter()
                .map(|(_, tags)| tags.len())
                .collect::<Vec<_>>(),
            vec![1, 1, 1, 1, 1]
        );
        assert_eq!(groups[3].1, vec!["blue hair"]);
        assert_eq!(groups[4].1, vec!["special_term"]);
    }
}
