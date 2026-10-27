use crate::{
    app::{BdmApp, Dialog},
    model::*,
    theme::*,
    widgets::*,
};
use eframe::egui::{self, vec2, Color32, RichText, Sense};

impl BdmApp {
    pub fn dashboard(&mut self, ui: &mut egui::Ui) {
        self.ingestion_status(ui);
        self.workflow_status(ui);
        let pending = self.library.media.iter().filter(|m| !m.keep).count();
        ui.columns(4, |cols| {
            let values = [
                self.library.media.len(),
                self.library.artists.len(),
                self.library.characters.len(),
                pending,
            ];
            for (i, (title, kind)) in [
                ("图片", "image"),
                ("画师", "artist"),
                ("角色", "character"),
                ("待审核", "logs"),
            ]
            .iter()
            .enumerate()
            {
                let r = count_card(&mut cols[i], values[i], title, kind);
                if cols[i]
                    .interact(r.rect, r.id.with("click"), Sense::click())
                    .on_hover_text(*title)
                    .clicked()
                {
                    match i {
                        1 => self.navigate(Page::Artist),
                        2 => self.navigate(Page::Character),
                        _ => {
                            self.navigate(Page::Artist);
                            self.gallery = true;
                            if i == 3 {
                                self.review_filter = 2;
                            }
                        }
                    }
                }
            }
        });
        ui.add_space(30.0);
        let gap = 32.0;
        let width = ui.available_width();
        let left = (width - gap) * 0.68;
        let right = width - gap - left;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(
                vec2(left, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_width(left);
                    ui.horizontal(|ui| {
                        section(ui, "最近", "");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, "forward", "全部图片", false).clicked() {
                                self.navigate(Page::Artist);
                                self.gallery = true;
                            }
                        });
                    });
                    ui.add_space(12.0);
                    if self.library.media.is_empty() {
                        let (r, _) = ui.allocate_exact_size(vec2(left, 310.0), Sense::hover());
                        ui.painter().rect_filled(r, 10.0, NAV);
                        icon(
                            ui,
                            egui::Rect::from_center_size(
                                r.center() - vec2(0.0, 28.0),
                                vec2(40.0, 40.0),
                            ),
                            "image",
                            MUTED,
                        );
                        let response = ui.interact(
                            egui::Rect::from_center_size(
                                r.center() + vec2(0.0, 30.0),
                                vec2(40.0, 40.0),
                            ),
                            ui.id().with("empty-add"),
                            Sense::click(),
                        );
                        ui.painter().rect_filled(response.rect, 8.0, PANEL);
                        icon(ui, response.rect.shrink(11.0), "plus", INK);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "添加画师")
                        });
                        if response.on_hover_text("添加画师").clicked() {
                            self.navigate(Page::Artist);
                            self.dialog = Some(Dialog::Add);
                        }
                    } else {
                        let cell = (left - 24.0) / 3.0;
                        let height = (cell * 0.86).min(190.0);
                        let ids: Vec<_> = self
                            .library
                            .media
                            .iter()
                            .rev()
                            .take(6)
                            .map(|m| m.id)
                            .collect();
                        for row in ids.chunks(3) {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                for id in row {
                                    let (rect, response) =
                                        ui.allocate_exact_size(vec2(cell, height), Sense::click());
                                    self.paint_media(ui, rect, *id);
                                    if response.on_hover_text(self.media_name(*id)).clicked() {
                                        self.open_media(*id);
                                    }
                                }
                            });
                            ui.add_space(6.0);
                        }
                    }
                },
            );
            ui.allocate_ui_with_layout(
                vec2(right, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_width(right);
                    ui.add_space(10.0);
                    section(ui, "文本", "");
                    ui.add_space(26.0);
                    let counts = self.library.coverage();
                    let total = self.library.media.len();
                    for row in [0..2, 2..4] {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            for i in row {
                                let label = ["原始标签", "优化标签", "自然语言", "齐全"][i];
                                let hint = [
                                    "原始标签",
                                    "优化标签（含待确认结果）",
                                    "自然语言描述（含待确认结果）",
                                    "三份文本齐全（含待确认结果）",
                                ][i];
                                if coverage_ring(
                                    ui,
                                    counts[i],
                                    total,
                                    label,
                                    hint,
                                    (right - 16.0) / 2.0,
                                )
                                .clicked()
                                {
                                    self.navigate(Page::Artist);
                                    self.gallery = true;
                                    self.review_filter = [0, 5, 6, 7][i];
                                }
                            }
                        });
                        ui.add_space(18.0);
                    }
                },
            );
        });
    }

    pub fn api_page(&mut self, ui: &mut egui::Ui) {
        if self.prefs.profiles.is_empty() {
            empty(ui, "暂无配置", "", "api");
        }
        let mut remove = None;
        for (index, profile) in self.prefs.profiles.clone().iter().enumerate() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    section(ui, &profile.name, &profile.model);
                    icon_button(ui, "info", "配置已保存，连接未验证", false);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("移除").clicked() {
                            remove = Some(index);
                        }
                        if ui.button("编辑").clicked() {
                            self.begin_api_profile(Some(index));
                        }
                        ui.add_enabled(false, egui::Button::new("测试连接"))
                            .on_hover_text("保存不发起请求；可在优化弹窗中选择此配置执行任务。");
                    });
                });
                ui.add_space(8.0);
                ui.label(RichText::new(&profile.endpoint).size(13.0).color(MUTED));
                ui.label(
                    RichText::new(format!(
                        "批量发送：每次 {} 张图片",
                        profile.request.batch_size
                    ))
                    .strong(),
                );
                ui.label(
                    RichText::new(format!(
                        "超时 {} 秒  ·  {}  ·  {}",
                        profile.request.timeout_seconds,
                        profile.request.json_output.description(),
                        if profile.request.send_images {
                            "图片 + Danbooru 通用元数据"
                        } else {
                            "仅文本"
                        }
                    ))
                    .size(13.0)
                    .color(MUTED),
                );
                ui.label(
                    RichText::new(format!(
                        "最大并发 {}  ·  限流组：{}",
                        profile.concurrency,
                        if profile.group.is_empty() {
                            "独立配置"
                        } else {
                            &profile.group
                        }
                    ))
                    .size(13.0)
                    .color(MUTED),
                );
            });
            ui.add_space(12.0);
        }
        if let Some(index) = remove {
            match self.remove_api_profile(index) {
                Ok(()) => self.notify(ui.ctx(), "配置及密钥已移除。"),
                Err(error) => self.notify(ui.ctx(), &error),
            }
        }
    }

    pub fn logs_page(&mut self, ui: &mut egui::Ui) {
        self.workflow_history(ui);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (i, label) in ["全部", "信息", "警告", "错误"].iter().enumerate() {
                    ui.selectable_value(&mut self.log_filter, i, *label);
                }
                search(ui, &mut self.query, "搜索日志…", 240.0);
            });
            ui.add_space(15.0);
            ui.separator();
            let entries: Vec<_> = self
                .logs
                .iter()
                .rev()
                .filter(|e| {
                    e.message.contains(&self.query)
                        && match self.log_filter {
                            1 => e.level == "INFO",
                            2 => e.level == "WARN",
                            3 => e.level == "ERROR",
                            _ => true,
                        }
                })
                .cloned()
                .collect();
            if entries.is_empty() {
                empty(ui, "没有匹配的日志", "调整筛选条件后再试。", "logs");
            }
            for e in entries {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui(vec2(110.0, 30.0), |ui| {
                        ui.label(RichText::new(e.time).color(MUTED).size(13.0));
                    });
                    badge(
                        ui,
                        e.level,
                        match e.level {
                            "ERROR" => Color32::from_rgb(185, 74, 74),
                            "WARN" => AMBER,
                            _ => ACCENT,
                        },
                    );
                    ui.add(egui::Label::new(RichText::new(e.message).size(13.0)).wrap());
                });
                ui.separator();
            }
        });
    }

    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        self.danbooru_settings_card(ui);
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "image", "图片查看");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.prefs.auto_load_review_original, "审核时自动加载原图")
                    .on_hover_text("默认使用缩略图。开启后，打开审核和切图自动加载原图；仍可手动切换。设置自动保存。");
                let (rect, response) = ui.allocate_exact_size(vec2(110.0, 32.0), egui::Sense::hover());
                icon(ui, egui::Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(17.0, 17.0)), "image", MUTED);
                icon(ui, egui::Rect::from_center_size(rect.center(), vec2(13.0, 13.0)), "forward", if self.prefs.auto_load_review_original { ACCENT } else { MUTED });
                icon(ui, egui::Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(29.0, 29.0)), "image", if self.prefs.auto_load_review_original { ACCENT } else { MUTED });
                response.on_hover_text("缩略图 → 原图。裁剪和修复选区映射到原图，保存修改原图；缩放和平移只改变视图。");
            });
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "character", "角色关系");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("自动处理").size(12.0).color(MUTED))
                    .on_hover_text("来源关系自动生效；在设置中查看和修改已生效关系。");
                if ui.button("角色关系设置").clicked() {
                    self.dialog = Some(Dialog::CharacterRelations);
                }
            });
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "tag", "文本姓名格式");
            ui.add_space(8.0);
            ui.label(
                RichText::new("画师名和角色名不会发送给 AI；在本地合并到标签和自然语言时使用以下格式。自然语言按画师、角色、描述排列。")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("画师名");
                egui::ComboBox::from_id_salt("artist_tag_format")
                    .selected_text(self.prefs.artist_tag_format.artist_label())
                    .show_ui(ui, |ui| {
                        for format in [
                            ProtectedTagFormat::At,
                            ProtectedTagFormat::Plain,
                            ProtectedTagFormat::Typed,
                        ] {
                            ui.selectable_value(
                                &mut self.prefs.artist_tag_format,
                                format,
                                format.artist_label(),
                            );
                        }
                    });
                ui.label("角色名");
                egui::ComboBox::from_id_salt("character_tag_format")
                    .selected_text(self.prefs.character_tag_format.character_label())
                    .show_ui(ui, |ui| {
                        for format in [ProtectedTagFormat::Plain, ProtectedTagFormat::Typed] {
                            ui.selectable_value(
                                &mut self.prefs.character_tag_format,
                                format,
                                format.character_label(),
                            );
                        }
                    });
            });
            ui.label(
                RichText::new("例如：画师可输出 @name、name 或 artist:name；角色可输出 name 或 character:name。")
                    .size(12.0)
                    .color(MUTED),
            );
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "download", "下载范围");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                badge(ui, "PNG", ACCENT);
                badge(ui, "JPEG", ACCENT);
                badge(ui, "WebP", ACCENT);
            })
            .response
            .on_hover_text(self.prefs.download_media.description());
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("最小短边");
                ui.add(
                    egui::DragValue::new(&mut self.minimum_short_side_draft)
                        .range(1..=bdm_domain::ingest::MAXIMUM_SHORT_SIDE)
                        .suffix(" px"),
                );
                if icon_button(ui, "reset", "恢复默认", false).clicked() {
                    self.minimum_short_side_draft =
                        bdm_domain::ingest::default_minimum_short_side();
                }
                if icon_button(ui, "check", "保存下载设置", true).clicked() {
                    match self.save_download_settings() {
                        Ok(()) => self.notify(ui.ctx(), "下载设置已保存，下次新建下载任务时生效。"),
                        Err(error) => self.notify(ui.ctx(), error),
                    }
                }
            });
            ui.add(
                egui::Label::new(
                    RichText::new(format!("已保存 ≥ {} px", self.prefs.minimum_short_side))
                        .size(12.0)
                        .color(MUTED),
                )
                .wrap(),
            )
            .on_hover_text(
                "默认 800 px。下载前筛选、下载后核验。修改用于新任务，继续旧任务沿用原阈值。",
            );
            ui.checkbox(
                &mut self.prefs.flatten_transparency,
                "下载时将透明图片自动转为白底",
            )
            .on_hover_text(
                "新建下载任务时生效；已下载文件不会被自动改写。关闭后保留原始透明通道。",
            );
            ui.label(
                RichText::new("设置只影响新建任务，继续中的任务沿用创建时的设置。")
                    .size(12.0)
                    .color(MUTED),
            );
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "layers", "图片编辑");
            ui.add_space(8.0);
            ui.checkbox(&mut self.prefs.confirm_image_edits,"局部修复后二次确认")
                .on_hover_text("局部修复开启后先预览再保存；关闭后直接替换。裁剪由图片下方的“预览裁剪”和“保存裁剪”按钮分别控制。设置自动保存。");
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "layers", "存储位置");
            ui.add_space(16.0);
            if let Some(path) = &self.catalog.path {
                ui.label(RichText::new("收藏记录位置").size(13.0).strong());
                ui.label(
                    RichText::new(path.display().to_string())
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add_space(14.0);
            }
            ui.label(RichText::new("托管根目录").size(13.0).strong());
            ui.add_sized(
                [ui.available_width().min(650.0), 36.0],
                egui::TextEdit::singleline(&mut self.root_draft)
                    .hint_text(crate::paths::library_directory().display().to_string())
                    .margin(vec2(12.0, 9.0)),
            );
            ui.add_space(14.0);
            if primary(ui, "保存").clicked() {
                let path = self.root_draft.trim();
                if !path.is_empty() && !std::path::Path::new(path).is_absolute() {
                    self.notify(ui.ctx(), "请填写绝对路径，例如 D:\\BooruLibrary。");
                } else {
                    self.prefs.root = path.into();
                    self.log("INFO", "已保存工作空间路径偏好，未移动任何文件。");
                    self.notify(ui.ctx(), "路径偏好已保存。");
                }
            }
        });
        ui.add_space(16.0);
        ui.columns(2, |cols| {
            card().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                section(ui, "视觉模型", "DINOv3");
                ui.add_space(12.0);

                ui.add_space(12.0);
                ui.label(RichText::new("模型目录").size(13.0));
                ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::TextEdit::singleline(&mut self.model_draft)
                        .hint_text("默认跟随托管根目录 models/")
                        .margin(vec2(10.0, 9.0)),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("保存").clicked() {
                        let path = self.model_draft.trim();
                        if path.is_empty() || std::path::Path::new(path).is_absolute() {
                            self.prefs.model_path = path.into();
                            self.notify(ui.ctx(), "模型目录偏好已保存。");
                        } else {
                            self.notify(ui.ctx(), "模型目录需要使用绝对路径。");
                        }
                    }
                    if ui
                        .add_enabled(
                            !self.workflow_active() && !self.demo && !self.is_capture(),
                            egui::Button::new("下载模型"),
                        )
                        .clicked()
                    {
                        if let Err(e) = self.start_model_download(ui.ctx()) {
                            self.workflows.error = Some(e);
                        }
                    }
                    ui.label(self.model_directory().display().to_string());
                });
                ui.add_space(5.0);
            });
            card().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                section(ui, "备份与恢复", "");
                ui.add_space(12.0);

                ui.add_space(12.0);

                ui.add_space(14.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled(false, egui::Button::new("备份"))
                        .on_hover_text("备份服务未接入");
                    ui.add_enabled(false, egui::Button::new("恢复"))
                        .on_hover_text("备份服务未接入");
                });
            });
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section(ui, "Booru Dataset Manager", "v0.1.0");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled(false, egui::Button::new("检查更新"))
                        .on_hover_text("官方发布源与更新检查适配器尚未接入。");
                });
            });
        });
    }

    pub fn save_download_settings(&mut self) -> Result<(), &'static str> {
        if !bdm_domain::ingest::valid_minimum_short_side(self.minimum_short_side_draft) {
            return Err("最小短边必须在 1–16384 像素之间。");
        }
        self.prefs.minimum_short_side = self.minimum_short_side_draft;
        self.log(
            "INFO",
            &format!("已保存下载最小短边：{} px。", self.prefs.minimum_short_side),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_status_cards_are_global_on_dashboard_and_absent_from_collection_pages() {
        fn texts(shape: &egui::Shape, result: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => result.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        texts(shape, result);
                    }
                }
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.ingest.jobs = vec![bdm_domain::ingest::IngestJob {
            id: 11,
            collection_id: 1,
            tag: "dashboard_download_fixture".into(),
            root: Default::default(),
            minimum_short_side: 800,
            flatten_transparency: true,
            before_id: None,
            state: "interrupted".into(),
            progress: Default::default(),
            error: None,
            failed_posts: vec![],
        }];
        app.workflows.jobs = ["dedup", "optimize", "export"]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| bdm_infrastructure::workflows::WorkflowJob {
                id: index as i64 + 1,
                kind: kind.into(),
                ids: vec![3, 8],
                state: "paused".into(),
                ..Default::default()
            })
            .collect();
        let optimize = app
            .workflows
            .jobs
            .iter_mut()
            .find(|job| job.kind == "optimize")
            .unwrap();
        optimize.mode = "caption".into();
        optimize.cursor = 2;
        optimize.deferred = vec![3, 8];
        optimize.accepted.insert(9999); // Results outside this scope are not successes.
        optimize.error = Some("API content_filter：未生成成功".into());
        let source = app.library.media.clone();
        for page in [Page::Dashboard, Page::Artist, Page::Character] {
            for gallery in [false, true] {
                if page == Page::Dashboard && gallery {
                    continue;
                }
                app.prefs.page = page;
                app.gallery = gallery;
                app.context = if gallery {
                    app.library.collections(page).first().map(|c| c.id)
                } else {
                    None
                };
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(1440.0, 960.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if page == Page::Dashboard {
                                app.dashboard(ui);
                            } else if gallery {
                                app.gallery_page(ui);
                            } else {
                                app.collections_page(ui);
                            }
                        });
                    },
                );
                let mut visible = Vec::new();
                for shape in output.shapes {
                    texts(&shape.shape, &mut visible);
                }
                for label in [
                    "图片下载",
                    "dashboard_download_fixture",
                    "去重",
                    "文本优化",
                    "导出",
                    "自然语言",
                    "成功 0 / 2",
                    "待重试 2",
                    "最近失败：API content_filter：未生成成功",
                ] {
                    assert_eq!(
                        visible.iter().any(|text| text == label),
                        page == Page::Dashboard,
                        "task label {label} on {page:?} gallery={gallery}"
                    );
                }
                assert_eq!(app.ingest.jobs.len(), 1);
                assert_eq!(app.workflows.jobs.len(), 3);
                assert!(app.workflows.jobs.iter().all(|job| job.state == "paused"
                    && job.cursor == if job.kind == "optimize" { 2 } else { 0 }));
                assert_eq!(app.library.media, source);
            }
        }
    }

    #[test]
    fn every_page_renders_at_supported_sizes() {
        for size in [[1050.0, 720.0], [1440.0, 960.0]] {
            for page in Page::ALL {
                let ctx = egui::Context::default();
                crate::theme::setup(&ctx);
                let mut prefs = Preferences::default();
                prefs.normalize();
                prefs.page = page;
                let mut app = BdmApp::from_preferences(prefs, true, None);
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(size[0], size[1]),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.shell(ctx);
                    },
                );
                assert!(!output.shapes.is_empty(), "empty page: {page:?}");
            }
        }
    }
}
