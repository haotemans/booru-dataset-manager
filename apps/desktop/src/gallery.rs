use crate::{app::BdmApp, model::*, theme::*, widgets::*};
use eframe::egui::{self, vec2, RichText, Sense, Stroke};
use std::collections::BTreeSet;

impl BdmApp {
    pub fn scope_media(&self) -> Vec<&Media> {
        if let Some(id) = self.context {
            self.associated_media(self.prefs.page, &BTreeSet::from([id]))
        } else {
            self.library.media.iter().collect()
        }
    }

    pub fn collections_page(&mut self, ui: &mut egui::Ui) {
        self.ingestion_status(ui);
        if self.prefs.page == Page::Character {
            self.character_relationships(ui);
        }
        if let Some(error) = self.catalog.error {
            ui.colored_label(DANGER, error.to_string());
            if ui.button("重新打开本地收藏").clicked() {
                if let Some(path) = self.catalog.path.clone() {
                    self.open_catalog(&path);
                }
            }
            ui.add_space(12.0);
        }
        if self.collection_delete_mode
            && self.dialog.is_none()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.cancel_collection_selection();
        }
        ui.horizontal_wrapped(|ui| {
            if search(ui, &mut self.query, "Tag", 240.0).changed() {
                self.page_number = 0;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.action_buttons(ui);
            });
        });
        ui.add_space(12.0);
        let collections: Vec<_> = self
            .library
            .collections(self.prefs.page)
            .iter()
            .rev()
            .filter(|c| {
                c.tag
                    .to_lowercase()
                    .contains(&self.query.trim().to_lowercase())
            })
            .cloned()
            .collect();
        let total = collections.len();
        let pages = total.div_ceil(self.prefs.page_size).max(1);
        self.page_number = self.page_number.min(pages - 1);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{total}")).size(12.0).color(MUTED));
            if !self.selected.is_empty() {
                badge(ui, &format!("{}", self.selected.len()), ACCENT);
                if !self.collection_delete_mode
                    && icon_button(ui, "close", "取消选择", false).clicked()
                {
                    self.selected.clear();
                }
            }
        });
        ui.add_space(12.0);
        if total == 0 {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                empty(
                    ui,
                    if self.query.is_empty() {
                        "还没有收藏"
                    } else {
                        "没有匹配的 Tag"
                    },
                    if self.query.is_empty() {
                        "添加 Danbooru Tag"
                    } else {
                        "试试其他关键词"
                    },
                    self.prefs.page.icon(),
                );
                ui.vertical_centered(|ui| {
                    if icon_button(
                        ui,
                        if self.query.is_empty() {
                            "plus"
                        } else {
                            "reset"
                        },
                        if self.query.is_empty() {
                            "添加"
                        } else {
                            "清空搜索"
                        },
                        true,
                    )
                    .clicked()
                    {
                        if self.query.is_empty() {
                            self.begin_add_collection();
                        } else {
                            self.query.clear();
                        }
                    }
                    ui.add_space(28.0);
                });
            });
        } else {
            let gap = 16.0;
            let cols = ((ui.available_width() + gap) / 205.0).floor().max(1.0) as usize;
            let width = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
            let visible: Vec<_> = collections
                .into_iter()
                .skip(self.page_number * self.prefs.page_size)
                .take(self.prefs.page_size)
                .collect();
            for row in visible.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for c in row {
                        self.collection_card(ui, c, width);
                    }
                });
                ui.add_space(4.0);
            }
        }
        ui.add_space(18.0);
        self.pagination(ui, total, "collections");
    }

    fn collection_card(&mut self, ui: &mut egui::Ui, c: &Collection, width: f32) {
        ui.allocate_ui_with_layout(
            vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.push_id(c.id, |ui| {
                    egui::Frame::none()
                        .fill(PANEL)
                        .rounding(10.0)
                        .stroke(Stroke::new(
                            if self.selected.contains(&c.id) {
                                2.0_f32
                            } else {
                                1.0_f32
                            },
                            if self.selected.contains(&c.id) {
                                ACCENT
                            } else {
                                LINE
                            },
                        ))
                        .inner_margin(7.0)
                        .show(ui, |ui| {
                            ui.set_width(width - 14.0);
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(width - 14.0, (width - 14.0) * 1.12),
                                Sense::click(),
                            );
                            let (image_count, latest) = if self.demo || self.is_capture() {
                                let images = self.associated_media(self.prefs.page, &BTreeSet::from([c.id]));
                                (images.len(), images.iter().map(|m| m.id).max())
                            } else {
                                self.ingest.collection_summaries.get(&c.id).copied().unwrap_or_default()
                            };
                            if let Some(id) = latest {
                                self.paint_media(ui, rect, id);
                            } else {
                                ui.painter().rect_filled(rect, 7.0, BG);
                                icon(
                                    ui,
                                    egui::Rect::from_center_size(rect.center(), vec2(30.0, 30.0)),
                                    "image",
                                    MUTED,
                                );
                            }
                            if response.clicked() {
                                if self.collection_delete_mode {
                                    if !self.selected.remove(&c.id) {
                                        self.selected.insert(c.id);
                                    }
                                } else {
                                    self.gallery = true;
                                    self.context = Some(c.id);
                                    self.query.clear();
                                    self.selected_media.clear();
                                    self.review_filter = 0;
                                    self.page_number = 0;
                                }
                            }
                            response.on_hover_text(&c.tag);
                            let check_rect = egui::Rect::from_min_size(
                                rect.left_top() + vec2(7.0, 7.0),
                                vec2(28.0, 28.0),
                            );
                            let mut checked = self.selected.contains(&c.id);
                            let mut check_ui =
                                ui.new_child(egui::UiBuilder::new().max_rect(check_rect));
                            if check_ui.checkbox(&mut checked, "").changed() {
                                if checked {
                                    self.selected.insert(c.id);
                                } else {
                                    self.selected.remove(&c.id);
                                }
                            }
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{image_count}")).strong().size(16.0),
                                );
                                let (icon_rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                icon(ui, icon_rect, "image", MUTED);
                                response.on_hover_text("本地图片");
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(c.api_count.map_or_else(|| "本地发现".into(), |count| format!("API {count}")))
                                                .color(MUTED)
                                                .size(13.0),
                                        )
                                        .on_hover_text(
                                            if c.api_count.is_none() {
                                                "来自已下载图片的画师标签；远程总数尚未查询。可直接进入图库或下载。"
                                            } else if self.demo {
                                                "示例 API 总数，未发起远程请求"
                                            } else {
                                                "添加时从 Danbooru 获取的总数；本地下载数单独显示"
                                            },
                                        );
                                    },
                                );
                            });
                        });
                });
            },
        );
    }

    fn action_buttons(&mut self, ui: &mut egui::Ui) {
        self.download_controls(ui);
        if !self.gallery {
            if self.collection_delete_mode {
                if icon_button(ui, "close", "取消删除选择", false).clicked() {
                    self.cancel_collection_selection();
                }
                if ui
                    .add_enabled(
                        !self.selected.is_empty(),
                        egui::Button::new(
                            RichText::new(format!("删除 {}", self.selected.len()))
                                .color(egui::Color32::WHITE),
                        )
                        .fill(DANGER)
                        .min_size(vec2(76.0, 40.0)),
                    )
                    .on_hover_text("删除所选记录及关联图片")
                    .clicked()
                {
                    self.open_collection_deletion(self.prefs.page, &self.selected.clone());
                }
                return;
            }
            if icon_button(ui, "trash", "选择要删除的记录", false).clicked() {
                self.selected.clear();
                self.collection_delete_mode = true;
            }
        }
        for (kind, label) in [("download", "导出"), ("spark", "优化"), ("layers", "去重")] {
            if icon_button(ui, kind, label, false).clicked() {
                self.open_action(label);
            }
        }
    }

    fn cancel_collection_selection(&mut self) {
        self.collection_delete_mode = false;
        self.selected.clear();
    }

    pub fn pagination(&mut self, ui: &mut egui::Ui, total: usize, id: &str) {
        let pages = total.div_ceil(self.prefs.page_size).max(1);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(id)
                .width(60.0)
                .selected_text(self.prefs.page_size.to_string())
                .show_ui(ui, |ui| {
                    for size in [20, 50, 100] {
                        if ui
                            .selectable_value(&mut self.prefs.page_size, size, size.to_string())
                            .changed()
                        {
                            self.page_number = 0;
                        }
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(self.page_number + 1 < pages, |ui| {
                        icon_button(ui, "forward", "下一页", false)
                    })
                    .inner
                    .clicked()
                {
                    self.page_number += 1;
                }
                let mut page = self.page_number + 1;
                ui.add(
                    egui::DragValue::new(&mut page)
                        .range(1..=pages)
                        .suffix(format!(" / {pages}")),
                );
                self.page_number = page - 1;
                if ui
                    .add_enabled_ui(self.page_number > 0, |ui| {
                        icon_button(ui, "back", "上一页", false)
                    })
                    .inner
                    .clicked()
                {
                    self.page_number -= 1;
                }
            });
        });
    }

    pub fn filtered_media(&self) -> Vec<Media> {
        let tokens: Vec<_> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut media: Vec<_> = self
            .scope_media()
            .into_iter()
            .filter(|m| {
                tokens
                    .iter()
                    .all(|q| m.tags.iter().any(|tag| tag.contains(q)))
                    && match self.review_filter {
                        1 => m.keep,
                        2 => !m.keep,
                        3 => !m.optimized,
                        4 => !m.caption,
                        5 => m.optimized,
                        6 => m.caption,
                        7 => m.optimized && m.caption,
                        _ => true,
                    }
                    && (self.source_filter == 0
                        || if self.demo {
                            (m.id % 2 + 1) == self.source_filter
                        } else {
                            self.source_filter == 1
                        })
                    && (!self.minimum_only || (m.width >= 1600 && m.height >= 1600))
            })
            .cloned()
            .collect();
        media.sort_by(|a, b| match self.sort {
            1 => a.id.cmp(&b.id),
            2 => (b.width * b.height)
                .cmp(&(a.width * a.height))
                .then(b.id.cmp(&a.id)),
            3 => b.rating.cmp(&a.rating).then(b.id.cmp(&a.id)),
            _ => b.id.cmp(&a.id),
        });
        media
    }

    pub fn gallery_page(&mut self, ui: &mut egui::Ui) {
        self.ingestion_status(ui);
        self.workflow_status(ui);
        ui.horizontal_wrapped(|ui| {
            if icon_button(ui, "back", "返回列表", false).clicked() {
                self.gallery = false;
                self.context = None;
                self.query.clear();
                self.selected_media.clear();
                self.page_number = 0;
            }
            let tag = self
                .context
                .and_then(|id| {
                    self.library
                        .collections(self.prefs.page)
                        .iter()
                        .find(|c| c.id == id)
                })
                .map(|c| c.tag.clone())
                .unwrap_or_else(|| "全部图片".into());
            ui.label(RichText::new(tag).color(MUTED).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.action_buttons(ui);
            });
        });
        ui.add_space(10.0);
        card().inner_margin(14.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if search(ui, &mut self.query, "Tag", 260.0).changed() {
                    self.page_number = 0;
                }
                egui::ComboBox::from_id_salt("review_filter")
                    .width(110.0)
                    .selected_text(
                        [
                            "状态",
                            "已保留",
                            "待审核",
                            "缺少优化标签",
                            "缺少描述",
                            "已有优化标签",
                            "已有描述",
                            "三份文本齐全",
                        ][self.review_filter],
                    )
                    .show_ui(ui, |ui| {
                        for (i, text) in [
                            "状态",
                            "已保留",
                            "待审核",
                            "缺少优化标签",
                            "缺少描述",
                            "已有优化标签",
                            "已有描述",
                            "三份文本齐全",
                        ]
                        .iter()
                        .enumerate()
                        {
                            if ui
                                .selectable_value(&mut self.review_filter, i, *text)
                                .changed()
                            {
                                self.page_number = 0;
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("source_filter")
                    .width(90.0)
                    .selected_text(["来源", "Danbooru", "Gelbooru"][self.source_filter])
                    .show_ui(ui, |ui| {
                        for (i, s) in ["来源", "Danbooru", "Gelbooru"].iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.source_filter, i, *s)
                                .changed()
                            {
                                self.page_number = 0;
                            }
                        }
                    });
                if ui
                    .checkbox(&mut self.minimum_only, "≥ 1600 × 1600")
                    .changed()
                {
                    self.page_number = 0;
                }
                if icon_button(ui, "reset", "重置筛选", false).clicked() {
                    self.query.clear();
                    self.review_filter = 0;
                    self.source_filter = 0;
                    self.minimum_only = false;
                    self.page_number = 0;
                }
            });
        });
        ui.add_space(12.0);
        let media = self.filtered_media();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{} / {}", media.len(), self.scope_media().len()))
                    .color(MUTED)
                    .size(12.0),
            );
            if icon_button(ui, "check", "选择筛选结果", false).clicked() {
                self.selected_media.extend(media.iter().map(|m| m.id));
            }
            if !self.selected_media.is_empty() {
                badge(ui, &format!("已选 {}", self.selected_media.len()), ACCENT);
                if icon_button(ui, "close", "清空选择", false).clicked() {
                    self.selected_media.clear();
                }
                if icon_button(ui, "trash", "删除所选图片：先核对清单", false).clicked()
                {
                    for id in self.selected_media.clone() {
                        self.reject_media(id);
                    }
                    self.open_deletion_review(None);
                }
            }
            self.deletion_queue_button(ui, None);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(
                    ui,
                    "masonry",
                    "瀑布流",
                    self.prefs.layout == GalleryLayout::Masonry,
                )
                .clicked()
                {
                    self.prefs.layout = GalleryLayout::Masonry;
                }
                if icon_button(ui, "grid", "网格", self.prefs.layout == GalleryLayout::Grid)
                    .clicked()
                {
                    self.prefs.layout = GalleryLayout::Grid;
                }
                egui::ComboBox::from_id_salt("media_sort")
                    .width(100.0)
                    .selected_text(["最新导入", "文件名", "图片尺寸", "评分"][self.sort])
                    .show_ui(ui, |ui| {
                        for (i, s) in ["最新导入", "文件名", "图片尺寸", "评分"]
                            .iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut self.sort, i, *s);
                        }
                    });
            });
        });
        ui.add_space(10.0);
        if media.is_empty() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                empty(ui, "没有符合条件的图片", "调整筛选条件后再试", "image");
            });
            return;
        }
        let pages = media.len().div_ceil(self.prefs.page_size).max(1);
        self.page_number = self.page_number.min(pages - 1);
        let visible: Vec<_> = media
            .iter()
            .skip(self.page_number * self.prefs.page_size)
            .take(self.prefs.page_size)
            .collect();
        let columns = ((ui.available_width() + 14.0) / 205.0).floor().max(1.0) as usize;
        let width = (ui.available_width() - 14.0 * (columns - 1) as f32) / columns as f32;
        if self.prefs.layout == GalleryLayout::Grid {
            for row in visible.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    for m in row {
                        self.media_card(ui, m, width, width * 0.95);
                    }
                });
                ui.add_space(4.0);
            }
        } else {
            let mut heights = vec![0.0_f32; columns];
            let mut lanes = vec![Vec::new(); columns];
            for m in visible {
                let lane = heights
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.total_cmp(b.1))
                    .map(|x| x.0)
                    .unwrap_or(0);
                let h = (width * m.height as f32 / m.width as f32).clamp(125.0, 340.0);
                heights[lane] += h + 42.0;
                lanes[lane].push((m, h));
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for lane in lanes {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        for (m, h) in lane {
                            self.media_card(ui, m, width, h);
                            ui.add_space(4.0);
                        }
                    });
                }
            });
        }
        ui.add_space(14.0);
        self.pagination(ui, media.len(), "media_pagination");
    }

    fn media_card(&mut self, ui: &mut egui::Ui, m: &Media, width: f32, height: f32) {
        ui.allocate_ui_with_layout(
            vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.push_id(("media", m.id), |ui| {
                    egui::Frame::none()
                        .fill(PANEL)
                        .rounding(9.0)
                        .stroke(Stroke::new(
                            if self.selected_media.contains(&m.id) {
                                2.0_f32
                            } else {
                                1.0_f32
                            },
                            if self.selected_media.contains(&m.id) {
                                ACCENT
                            } else {
                                LINE
                            },
                        ))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.set_width(width - 12.0);
                            let (rect, response) =
                                ui.allocate_exact_size(vec2(width - 12.0, height), Sense::click());
                            self.paint_media(ui, rect, m.id);
                            if response
                                .on_hover_text(format!(
                                    "{} · {}",
                                    self.media_name(m.id),
                                    if !self.demo || m.id.is_multiple_of(2) {
                                        "Danbooru"
                                    } else {
                                        "Gelbooru"
                                    }
                                ))
                                .clicked()
                            {
                                self.open_media(m.id);
                            }
                            let mut selected = self.selected_media.contains(&m.id);
                            let mut check_ui = ui.new_child(egui::UiBuilder::new().max_rect(
                                egui::Rect::from_min_size(
                                    rect.left_top() + vec2(6.0, 6.0),
                                    vec2(28.0, 28.0),
                                ),
                            ));
                            if check_ui.checkbox(&mut selected, "").changed() {
                                if selected {
                                    self.selected_media.insert(m.id);
                                } else {
                                    self.selected_media.remove(&m.id);
                                }
                            }
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{} × {}", m.width, m.height))
                                        .size(12.0)
                                        .color(MUTED),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let (r, response) = ui
                                            .allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                                        icon(
                                            ui,
                                            r,
                                            if self.pending_deletion.contains(&m.id) {
                                                "trash"
                                            } else if m.keep {
                                                "check"
                                            } else {
                                                "clock"
                                            },
                                            if self.pending_deletion.contains(&m.id) {
                                                DANGER
                                            } else if m.keep {
                                                ACCENT
                                            } else {
                                                AMBER
                                            },
                                        );
                                        response.on_hover_text(
                                            if self.pending_deletion.contains(&m.id) {
                                                "待删除 · 尚未确认"
                                            } else if m.keep {
                                                "已保留"
                                            } else {
                                                "待审核"
                                            },
                                        );
                                    },
                                );
                            });
                        });
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Dialog;

    fn click_collection_ui(ctx: &egui::Context, app: &mut BdmApp, position: egui::Pos2) {
        let record = app.library.collections(app.prefs.page)[0].clone();
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(position)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(420.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.horizontal(|ui| app.action_buttons(ui));
                        app.collection_card(ui, &record, 230.0);
                    });
                },
            );
        }
    }

    #[test]
    fn toolbar_delete_selects_cards_before_opening_frozen_confirmation() {
        for page in [Page::Artist, Page::Character] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
            app.navigate(page);
            // The former card-trash area is once again a normal cover click.
            click_collection_ui(&ctx, &mut app, egui::pos2(205.0, 85.0));
            assert!(app.gallery);
            assert_eq!(app.context, Some(0));
            assert!(app.dialog.is_none());
            app.navigate(page);

            click_collection_ui(&ctx, &mut app, egui::pos2(28.0, 28.0));
            assert!(app.collection_delete_mode);
            assert!(app.dialog.is_none());
            // An empty selection cannot open confirmation.
            click_collection_ui(&ctx, &mut app, egui::pos2(90.0, 28.0));
            assert!(app.dialog.is_none());
            let cover = egui::pos2(205.0, 85.0);
            click_collection_ui(&ctx, &mut app, cover);
            assert_eq!(app.selected, BTreeSet::from([0]));
            assert!(!app.gallery);
            assert!(app.dialog.is_none());
            click_collection_ui(&ctx, &mut app, cover);
            assert!(app.selected.is_empty());
            // The checkbox also selects once, without opening the gallery.
            click_collection_ui(&ctx, &mut app, egui::pos2(29.0, 73.0));
            assert_eq!(app.selected, BTreeSet::from([0]));
            assert!(!app.gallery);
            click_collection_ui(&ctx, &mut app, egui::pos2(90.0, 28.0));
            assert!(
                matches!(&app.dialog, Some(Dialog::DeleteCollections(scope)) if scope.page == page && scope.ids == BTreeSet::from([0]) && scope.media_ids.len() == 4)
            );
            assert_eq!(app.library.collections(page).len(), 6);
            assert_eq!(app.library.media.len(), 24);
            let Some(Dialog::DeleteCollections(scope)) = app.dialog.take() else {
                panic!("missing confirmation");
            };
            app.confirm_sample_collection_deletion(&scope).unwrap();
            assert!(!app.collection_delete_mode);
            assert!(app.selected.is_empty());
            assert_eq!(app.library.media.len(), 20);
        }
    }

    #[test]
    fn cancelling_or_navigating_exits_delete_selection_without_removing_records() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.navigate(Page::Artist);
        let original = app.library.artists.clone();
        let media = app.library.media.clone();
        click_collection_ui(&ctx, &mut app, egui::pos2(28.0, 28.0));
        click_collection_ui(&ctx, &mut app, egui::pos2(205.0, 85.0));
        click_collection_ui(&ctx, &mut app, egui::pos2(28.0, 28.0));
        assert!(!app.collection_delete_mode);
        assert!(app.selected.is_empty());
        click_collection_ui(&ctx, &mut app, egui::pos2(205.0, 85.0));
        assert!(app.gallery);
        app.navigate(Page::Artist);
        app.collection_delete_mode = true;
        app.selected.insert(0);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.collections_page(ui));
            },
        );
        assert!(!app.collection_delete_mode);
        assert!(app.selected.is_empty());
        app.collection_delete_mode = true;
        app.selected.insert(0);
        app.navigate(Page::Character);
        assert!(!app.collection_delete_mode);
        assert!(app.selected.is_empty());
        assert_eq!(app.library.artists, original);
        assert_eq!(app.library.media, media);
    }
    #[test]
    fn selecting_thumbnail_does_not_open_detail() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut prefs = Preferences::default();
        prefs.normalize();
        let mut app = BdmApp::from_preferences(prefs, true, None);
        let media = app.library.media[0].clone();
        for pressed in [None, Some(true), Some(false)] {
            let position = egui::pos2(29.0, 29.0);
            let mut events = vec![egui::Event::PointerMoved(position)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(420.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.media_card(ui, &media, 230.0, 200.0));
                },
            );
        }
        assert!(app.selected_media.contains(&media.id));
        assert!(app.dialog.is_none());
    }
    #[test]
    fn coverage_filters_match_dashboard_numerators() {
        let mut prefs = Preferences::default();
        prefs.normalize();
        let mut app = BdmApp::from_preferences(prefs, true, None);
        for (filter, count) in [0, 5, 6, 7].into_iter().zip(app.library.coverage()) {
            app.review_filter = filter;
            assert_eq!(app.filtered_media().len(), count);
        }
    }
    #[test]
    fn live_filters_compose_and_do_not_change_action_scope() {
        let mut prefs = Preferences::default();
        prefs.normalize();
        prefs.page = Page::Artist;
        let mut app = BdmApp::from_preferences(prefs, true, None);
        app.gallery = true;
        app.context = Some(0);
        app.query = "mountain sunset".into();
        app.review_filter = 1;
        assert_eq!(app.filtered_media().len(), 2);
        app.open_action("导出");
        assert_eq!(app.action_snapshot.len(), 4);
    }
    #[test]
    fn latest_import_and_rating_sorts_are_deterministic() {
        let mut prefs = Preferences::default();
        prefs.normalize();
        let mut app = BdmApp::from_preferences(prefs, true, None);
        assert_eq!(app.filtered_media()[0].id, 23);
        app.sort = 3;
        let media = app.filtered_media();
        assert_eq!(media[0].rating, 5);
        assert!(media.windows(2).all(|p| p[0].rating >= p[1].rating));
    }
}
