use crate::{
    app::{BdmApp, Dialog},
    model::*,
    theme::*,
    widgets::*,
};
use eframe::egui::{self, vec2, Color32, RichText, Sense};

impl BdmApp {
    pub fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let title = match &dialog {
            Dialog::Add => {
                if self.catalog.kind == Some(bdm_domain::catalog::CollectionKind::Query) {
                    "添加标签合集"
                } else {
                    match self.prefs.page {
                        Page::Downloads => "添加标签合集",
                        Page::Artist => "添加画师",
                        _ => "添加角色",
                    }
                }
            }
            Dialog::Action(kind) => kind,
            Dialog::Profile(_) => "API 配置",
            Dialog::ImageUpload(_) => "确认发送图片",
            Dialog::CharacterRelations => "角色关系设置",
            Dialog::Similarity => "相似图审核",
            Dialog::Media(_) => "图片审核",
            Dialog::Delete { .. } => "确认删除",
            Dialog::DeleteCollections(scope) => match scope.page {
                Page::Downloads => "删除下载查询",
                Page::Artist => "删除画师",
                _ => "删除角色",
            },
        };
        egui::Area::new("dialog_backdrop".into())
            .order(egui::Order::Middle)
            .fixed_pos(ctx.screen_rect().min)
            .show(ctx, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(ctx.screen_rect().size(), Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::BLACK.gamma_multiply(0.15));
            });
        let mut open = true;
        let mut close = false;
        if matches!(dialog, Dialog::Similarity) {
            close = self.show_similarity_review(ctx);
        } else if let Dialog::Media(id) = &dialog {
            close = self.show_media_review(ctx, *id);
        } else {
            egui::Window::new(title)
                .id(egui::Id::new(match &dialog {
                    Dialog::Add => "add_dialog",
                    Dialog::Action(_) => "action_dialog",
                    Dialog::Profile(_) => "profile_dialog",
                    Dialog::ImageUpload(_) => "image_upload_dialog",
                    Dialog::CharacterRelations => "character_relations_dialog",
                    Dialog::Similarity => "similarity_review_dialog",
                    Dialog::Media(_) => "media_dialog",
                    Dialog::Delete { .. } => "delete_dialog",
                    Dialog::DeleteCollections(_) => "delete_collections_dialog",
                }))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .frame(card().inner_margin(24.0))
                .default_width(if matches!(dialog, Dialog::Media(_)) {
                    690.0
                } else if matches!(dialog, Dialog::CharacterRelations) {
                    600.0
                } else {
                    460.0
                })
                .anchor(egui::Align2::CENTER_CENTER, vec2(36.0, 0.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).font(medium(18.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, "close", "关闭", false).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    close |= match &dialog {
                        Dialog::Add => self.add_dialog(ui),
                        Dialog::Profile(index) => {
                            egui::ScrollArea::vertical()
                                .max_height((ctx.screen_rect().height() - 150.0).max(200.0))
                                .show(ui, |ui| self.profile_dialog(ui, *index))
                                .inner
                        }
                        Dialog::ImageUpload(scope) => self.image_upload_dialog(ui, scope),
                        Dialog::CharacterRelations => self.character_relations_dialog(ui),
                        Dialog::Action(kind) => self.action_dialog(ui, kind),
                        Dialog::Media(_) => unreachable!(),
                        Dialog::Similarity => unreachable!(),
                        Dialog::Delete { return_media } => self.deletion_dialog(ui, *return_media),
                        Dialog::DeleteCollections(scope) => {
                            self.collection_deletion_dialog(ui, scope)
                        }
                    };
                    ui.add_space(8.0);
                });
        }
        if self.media_review.redownload_notice.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            close = true;
        }
        if open && !close && self.dialog.is_none() {
            self.dialog = Some(dialog);
        } else {
            if matches!(&dialog, Dialog::Add) {
                self.cancel_tag_lookup();
                if self.catalog.edit_credentials {
                    self.danbooru.api_key.clear();
                    self.danbooru.error = None;
                    self.catalog.edit_credentials = false;
                }
            }
            if matches!(&dialog, Dialog::Profile(_)) {
                self.api_profiles.key_draft.clear();
                self.api_profiles.load_error = None;
            }
            if let Dialog::Media(id) = dialog {
                self.clear_review_original();
                self.media_review.reset_canvas();
                self.image_editor.discard();
                if self.dialog.is_none() && !self.pending_deletion.is_empty() {
                    self.open_deletion_review(Some(id));
                }
            }
            if let Dialog::Delete { return_media } = dialog {
                // X and Escape have exactly the same non-destructive behavior as Cancel.
                if self.dialog.is_none() {
                    self.cancel_deletion(return_media);
                }
            }
            if matches!(dialog, Dialog::Similarity) {
                self.similarity_review.release();
            }
            self.form_error.clear();
        }
    }

    fn download_query_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        ui.set_min_width(460.0);
        ui.set_max_width(460.0);
        ui.label("来源标签组合");
        ui.add_space(8.0);
        ui.add_sized(
            [460.0, 38.0],
            egui::TextEdit::singleline(&mut self.input_tag)
                .hint_text("official_art zenless_zone_zero")
                .margin(vec2(12.0, 10.0)),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            icon_label(
                ui,
                "tag",
                "空格分隔",
                "多个完整标签须同时满足；支持标签内的冒号，不支持搜索操作符、通配符或排除词。",
            );
            icon_label(
                ui,
                "artist",
                "画师页合集",
                "整批图片统一审核、裁剪、优化和导出；每张图片保留真实画师信息。",
            );
            hint(
                ui,
                "保存不联网；下载沿用来源账号与权限，已有图片直接复用，任务进度显示在总览。",
            );
        });
        ui.add_space(20.0);
        let mut choice = None;
        ui.horizontal(|ui| {
            let enabled = !self.demo && !self.is_capture() && self.catalog.repository.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("保存合集"))
                .clicked()
            {
                choice = Some(false);
            }
            if ui
                .add_enabled_ui(enabled && !self.ingest_active(), |ui| {
                    primary(ui, "保存并下载")
                })
                .inner
                .clicked()
            {
                choice = Some(true);
            }
        });
        if let Some(download) = choice {
            match self.save_download_query() {
                Ok(_) => {
                    if download {
                        if let Err(error) = self.start_ingest(ui.ctx(), None) {
                            self.notify(
                                ui.ctx(),
                                &format!("合集已加入画师页，下载未启动：{error}"),
                            );
                            return true;
                        }
                        self.notify(ui.ctx(), "合集已加入画师页，下载进度在总览查看");
                    } else {
                        self.notify(ui.ctx(), "合集已加入画师页，点击卡片进入图库审核");
                    }
                    return true;
                }
                Err(error) => self.form_error = error,
            }
        }
        if !self.form_error.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(&self.form_error).color(AMBER).size(13.0)).wrap(),
            );
        }
        false
    }

    fn add_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        if self.prefs.page == Page::Artist {
            use bdm_domain::catalog::CollectionKind;
            ui.horizontal(|ui| {
                for (kind, label) in [
                    (CollectionKind::Artist, "画师"),
                    (CollectionKind::Query, "标签合集"),
                ] {
                    if ui
                        .selectable_label(self.catalog.kind == Some(kind), label)
                        .clicked()
                        && self.catalog.kind != Some(kind)
                    {
                        self.cancel_tag_lookup();
                        self.catalog.kind = Some(kind);
                        self.input_tag.clear();
                        self.form_error.clear();
                    }
                }
            });
            ui.add_space(12.0);
        }
        if self.prefs.page == Page::Downloads
            || self.catalog.kind == Some(bdm_domain::catalog::CollectionKind::Query)
        {
            return self.download_query_dialog(ui);
        }
        ui.set_min_width(460.0);
        ui.set_max_width(460.0);
        ui.label(format!("来源 Tag · {}", self.prefs.download_source.label()));
        ui.add(
            egui::Label::new(
                RichText::new(self.tag_lookup_account_label())
                    .color(MUTED)
                    .size(12.0),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        let hint = if self.prefs.page == Page::Artist {
            "artist_tag"
        } else {
            "character_(copyright)"
        };
        let changed = ui
            .add_enabled_ui(!self.tag_lookup_pending(), |ui| {
                ui.add_sized(
                    [460.0, 38.0],
                    egui::TextEdit::singleline(&mut self.input_tag)
                        .hint_text(hint)
                        .margin(vec2(12.0, 10.0)),
                )
            })
            .inner
            .changed();
        if changed {
            self.catalog.suggestions.clear();
            self.catalog.found = None;
            self.catalog.lookup_error = None;
            self.form_error.clear();
        }
        ui.add_space(14.0);
        if self.demo {
            ui.label(
                RichText::new("示例预览不查询远程 Tag，也不写入本地收藏。")
                    .color(MUTED)
                    .size(13.0),
            );
        }
        if ui
            .add_enabled_ui(
                !self.tag_lookup_pending() && !self.demo && !self.is_capture(),
                |ui| primary(ui, "查询 Tag"),
            )
            .inner
            .clicked()
        {
            if let Err(error) = self.start_tag_lookup(ui.ctx()) {
                self.form_error = error;
            }
        }
        if self.tag_lookup_pending() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在查询 Danbooru…");
            });
        }
        if let Some(tag) = &self.catalog.found {
            ui.add_space(12.0);
            ui.label(&tag.name);
            ui.label(format!(
                "{} · {} · API 总数 {} · 本地图片 0",
                tag.provider.label(),
                if tag.category == 1 {
                    "画师"
                } else {
                    "角色"
                },
                tag.post_count
            ));
            ui.label(
                RichText::new("确认后保存收藏；选择卡片或进入图库即可下载图片。")
                    .color(MUTED)
                    .size(13.0),
            );
            if ui
                .add_enabled_ui(!self.demo && !self.is_capture(), |ui| {
                    primary(ui, "确认添加")
                })
                .inner
                .clicked()
            {
                match self.save_found_collection() {
                    Ok(created) => {
                        self.notify(
                            ui.ctx(),
                            if created {
                                "已添加到本地收藏"
                            } else {
                                "已在本地收藏中"
                            },
                        );
                        return true;
                    }
                    Err(error) => self.form_error = error,
                }
            }
        }
        if !self.form_error.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(&self.form_error).color(AMBER).size(13.0)).wrap(),
            );
            let from_gelbooru = self.catalog.lookup_provider
                == Some(bdm_domain::sources::SourceProvider::Gelbooru)
                || self.catalog.lookup_error == Some(bdm_ports::CatalogError::GelbooruCredentials);
            if from_gelbooru {
                if matches!(
                    self.catalog.lookup_error,
                    Some(
                        bdm_ports::CatalogError::GelbooruCredentials
                            | bdm_ports::CatalogError::Authentication
                            | bdm_ports::CatalogError::Credentials
                    )
                ) && ui.button("打开 Gelbooru 来源设置").clicked()
                {
                    self.navigate(Page::Settings);
                    return true;
                }
                ui.hyperlink_to("在浏览器中检查 Gelbooru", "https://gelbooru.com/");
            }
            if !from_gelbooru
                && matches!(
                    self.catalog.lookup_error,
                    Some(
                        bdm_ports::CatalogError::Authentication
                            | bdm_ports::CatalogError::Credentials
                    )
                )
            {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("修改凭据").clicked() {
                        self.catalog.edit_credentials = !self.catalog.edit_credentials;
                        self.danbooru.api_key.clear();
                    }
                    if ui
                        .add_enabled(
                            !self.tag_lookup_pending() && !self.demo && !self.is_capture(),
                            egui::Button::new("不使用账号重试"),
                        )
                        .clicked()
                    {
                        self.catalog.anonymous = true;
                        if let Err(error) = self.start_tag_lookup(ui.ctx()) {
                            self.form_error = error;
                        }
                    }
                });
            }
            if !from_gelbooru
                && matches!(
                    self.catalog.lookup_error,
                    Some(
                        bdm_ports::CatalogError::WebsiteVerification
                            | bdm_ports::CatalogError::AccessDenied
                    )
                )
            {
                ui.add_space(8.0);
                ui.hyperlink_to("在浏览器中检查 Danbooru", "https://danbooru.donmai.us/");
            }
        }
        if !self.catalog.suggestions.is_empty() {
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    RichText::new("相近标签 · 请确认是同一画师／角色后选择")
                        .size(13.0)
                        .color(MUTED),
                )
                .wrap(),
            );
            for candidate in self.catalog.suggestions.clone() {
                if ui
                    .button(format!(
                        "{} · {} · {} 张",
                        candidate.provider.label(),
                        candidate.name,
                        candidate.post_count
                    ))
                    .clicked()
                {
                    if let Err(error) = self.select_tag_suggestion(&candidate) {
                        self.form_error = error;
                    }
                }
            }
        }
        if self.catalog.edit_credentials {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            self.danbooru_recovery_form(ui);
        }
        false
    }

    fn profile_dialog(&mut self, ui: &mut egui::Ui, index: Option<usize>) -> bool {
        form_field(
            ui,
            "配置名称",
            &mut self.profile_draft.name,
            "例如：本地文本服务",
        );
        form_field(
            ui,
            "服务基础地址",
            &mut self.profile_draft.endpoint,
            "https://api.example.com/v1",
        );
        ui.label(RichText::new("API Key").size(13.0));
        ui.horizontal(|ui| {
            let saved = self.api_profiles.key_saved;
            ui.add_sized(
                [if saved { 412.0 } else { 460.0 }, 36.0],
                egui::TextEdit::singleline(&mut self.api_profiles.key_draft)
                    .password(true)
                    .hint_text(if saved {
                        "已保存，留空保留"
                    } else {
                        "粘贴 API Key；无需认证可留空"
                    })
                    .margin(vec2(10.0, 8.0)),
            )
            .on_hover_text("密钥保存在 Windows 凭据管理器，不写入普通配置文件。");
            if saved && icon_button(ui, "trash", "移除当前配置的密钥", false).clicked() {
                match self.clear_api_profile_key() {
                    Ok(()) => self.form_error.clear(),
                    Err(error) => self.form_error = error,
                }
            }
        });
        ui.add_space(4.0);
        form_field(
            ui,
            "模型名称",
            &mut self.profile_draft.model,
            "填写服务支持的模型 ID",
        );
        form_field(
            ui,
            "限流组（可选）",
            &mut self.profile_draft.group,
            "留空时独立限流",
        );
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("批量发送").strong());
                egui::ComboBox::from_id_salt("api_batch_size")
                    .selected_text(format!(
                        "{} 张 / 请求",
                        self.profile_draft.request.batch_size
                    ))
                    .show_ui(ui, |ui| {
                        for size in 1..=8 {
                            ui.selectable_value(
                                &mut self.profile_draft.request.batch_size,
                                size,
                                format!("{} 张图片", size),
                            );
                        }
                    });
            });
            hint(ui, "每次请求发送的图片数量，不改变最大并发。");
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("最大并发");
            ui.add(egui::DragValue::new(&mut self.profile_draft.concurrency).range(1..=16));
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("超时");
            ui.add(
                egui::DragValue::new(&mut self.profile_draft.request.timeout_seconds)
                    .range(10..=600)
                    .suffix(" 秒"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("网络重试");
            ui.add(
                egui::DragValue::new(&mut self.profile_draft.request.transport_retries)
                    .range(0..=3)
                    .suffix(" 次"),
            );
            hint(
                ui,
                "仅重试连接失败、超时或服务端暂时故障，不重复提交被拒绝的内容。",
            );
        });
        ui.horizontal(|ui| {
            ui.label("JSON 输出");
            egui::ComboBox::from_id_salt("json_output_mode")
                .selected_text(self.profile_draft.request.json_output.description())
                .show_ui(ui, |ui| {
                    for mode in [
                        bdm_config::JsonOutputMode::JsonObject,
                        bdm_config::JsonOutputMode::JsonSchema,
                        bdm_config::JsonOutputMode::PromptOnly,
                    ] {
                        ui.selectable_value(
                            &mut self.profile_draft.request.json_output,
                            mode,
                            mode.description(),
                        );
                    }
                });
        });
        ui.add_space(8.0);
        ui.checkbox(
            &mut self.profile_draft.request.send_images,
            "发送图片 + 标签",
        )
        .on_hover_text("发送图片及 Danbooru 通用视觉标签用于优化。每次发送前仍需确认图片范围。");
        ui.add_enabled_ui(self.profile_draft.request.send_images, |ui| {
            ui.horizontal(|ui| {
                ui.label("最长边");
                ui.add(
                    egui::DragValue::new(&mut self.profile_draft.request.image_max_edge)
                        .range(64..=bdm_config::AI_IMAGE_MAX_EDGE)
                        .suffix(" px"),
                );
                ui.label("JPEG 质量");
                ui.add(
                    egui::DragValue::new(&mut self.profile_draft.request.jpeg_quality)
                        .range(40..=95),
                );
            });
        });
        icon_label(ui, "image", "等比预处理", "仅在内存中等比例缩小，不放大小图；最长边不超过 1024 px，单图 JPEG 不超过 1 MiB。原文件不修改。");
        ui.checkbox(&mut self.profile_draft.request.use_cache, "复用历史结果")
            .on_hover_text("相同图片、元数据和提示词复用历史优化结果；关闭后重新调用 API。");
        ui.add_space(10.0);
        ui.collapsing("V4 联合视觉标注 · 系统提示词", |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.profile_draft.request.caption_system_prompt)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            if ui.button("恢复 V4 默认提示词").clicked() {
                self.profile_draft.request.caption_system_prompt =
                    bdm_config::DEFAULT_CAPTION_SYSTEM_PROMPT.into();
            }
        });
        ui.add_space(12.0);
        if primary(ui, "保存配置").clicked() {
            match self.save_api_profile(index) {
                Ok(()) => {
                    self.notify(ui.ctx(), "配置已保存，尚未验证连接。");
                    return true;
                }
                Err(error) => self.form_error = error,
            }
        }
        if let Some(error) = &self.api_profiles.load_error {
            ui.label(RichText::new(error).color(AMBER).size(13.0));
        }
        if !self.form_error.is_empty() {
            ui.label(RichText::new(&self.form_error).color(AMBER).size(13.0));
        }
        false
    }

    fn action_dialog(&mut self, ui: &mut egui::Ui, kind: &str) -> bool {
        ui.set_min_width(460.0);
        ui.set_max_width(460.0);
        egui::ScrollArea::vertical()
            .id_salt(("action-options", kind))
            .max_height((ui.ctx().screen_rect().height() - 210.0).max(200.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
        let selected_count = self
            .action_snapshot
            .iter()
            .filter(|id| self.selected_media.contains(id))
            .count();
        ui.horizontal_wrapped(|ui| {
            if kind != "去重" {
                ui.label(RichText::new("范围").font(medium(13.0)));
                ui.add_space(12.0);
            }
            ui.radio_value(
                &mut self.action_selected,
                false,
                format!("全部 · {}", self.action_snapshot.len()),
            )
            .on_hover_text("当前范围全部图片，不受搜索筛选影响");
            ui.add_enabled_ui(selected_count > 0, |ui| {
                ui.radio_value(
                    &mut self.action_selected,
                    true,
                    format!("已选 · {selected_count}"),
                );
            });
        });
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);
        if kind == "去重" {
            ui.checkbox(
                &mut self.prefs.similar_image_review,
                "相似图审核",
            ).on_hover_text("背景或内容变化列为候选，供看图审核；自动删重仍要求 pHash ≤ 2 且 DINOv3 ≥ 0.997。");
            ui.add_enabled_ui(self.prefs.similar_image_review, |ui| {
                ui.horizontal(|ui| {
                    ui.label("相似度门槛");
                    ui.add(
                        egui::DragValue::new(&mut self.prefs.review_similarity)
                            .range(0.85..=1.0)
                            .speed(0.001)
                            .fixed_decimals(3),
                    );
                    for value in [0.95, 0.92, 0.90] {
                        if ui.small_button(format!("{value:.2}")).clicked() {
                            self.prefs.review_similarity = value;
                        }
                    }
                });
            });
            self.candidate_review(ui);
        } else {
            if kind == "导出" {
                section(ui, "文本", "");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let width = (ui.available_width() - 12.0) / 2.0;
                    if choice_card(
                        ui,
                        "text",
                        "自然语言",
                        "画面描述",
                        self.action_mode == 0,
                        width,
                    )
                    .clicked()
                    {
                        self.action_mode = 0;
                    }
                    if choice_card(
                        ui,
                        "tag",
                        "优化标签",
                        "Danbooru",
                        self.action_mode == 1,
                        width,
                    )
                    .clicked()
                    {
                        self.action_mode = 1;
                    }
                });
                ui.add_space(16.0);
            }
            if kind == "优化" {
                self.action_mode = 2;
                section(ui, "V4 单次推理双输出", "");
                ui.add_space(6.0);
                icon_label(
                    ui,
                    "spark",
                    "自然语言 + Danbooru 标签 + 文字 BBox",
                    "每张图片只调用一次多模态模型，同步产出单行英文描述与无下划线 Danbooru 视觉标签（涵盖人物、动作、服饰、场景、构图、光影、画风、细节及文字框），画师与角色名在本地强保护拼接。",
                );
                ui.add_space(10.0);
                ui.checkbox(
                    &mut self.workflows.overwrite_optimized,
                    "覆盖已有结果",
                ).on_hover_text("默认只补全尚未同时具备自然语言与优化标签的图片。勾选或填写补充提示词后会重新处理已有结果。");
                ui.add_space(6.0);
                let selected = self
                    .prefs
                    .profiles
                    .get(self.workflows.profile)
                    .map(|p| p.name.as_str())
                    .unwrap_or("选择 API 配置");
                egui::ComboBox::from_id_salt("optimization_profile")
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, profile) in self.prefs.profiles.iter().enumerate() {
                            ui.selectable_value(&mut self.workflows.profile, i, &profile.name);
                        }
                    });
                ui.add_space(10.0);
                ui.label(RichText::new("补充提示词 · 精准重审（可选）").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.workflows.optimize_hint)
                        .hint_text("例如：异色瞳左蓝右红，手持折扇，右上角气泡文字需标注…")
                        .desired_width(f32::INFINITY)
                        .margin(vec2(10.0, 8.0)),
                );
                ui.add_space(8.0);
                icon_label(ui, "image", "图片 + 通用标签先验", "发送图片和空格分隔的 Danbooru 通用视觉标签先验；画师、角色及作品名在本地合成，不发送给 AI。");
            }
            if kind == "导出" {
                section(ui, "图片", "");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let width = (ui.available_width() - 12.0) / 2.0;
                    let detail = if self.workflows.export_folders { "逐图文件夹" } else { "扁平 · 图片 + TXT" };
                    if choice_card(ui, "image", "原尺寸", detail, !self.workflows.export_multires, width)
                        .on_hover_text("默认：保留当前分辨率，所有档位混合直出同一目录，不缩小、不放大。附 1024 训练参考配置，缩放由训练脚本完成。")
                        .clicked() { self.workflows.export_multires = false; self.workflows.export_folders = false; }
                    if choice_card(ui, "layers", "多分辨率", "768 · 1024 · 1536 · 2048", self.workflows.export_multires, width)
                        .on_hover_text("生成四个独立目录，各自放置图片和同名 TXT；不放大小图，尺寸不足的档位跳过。原文件不修改。")
                        .clicked() { self.workflows.export_multires = true; self.workflows.export_folders = false; }
                });
                ui.add_space(16.0);
                section(ui, "保存位置", "");
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.workflows.export_root)
                        .hint_text("D:\\DatasetExport")
                        .desired_width(f32::INFINITY)
                        .margin(vec2(12.0, 10.0)),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    icon_label(ui, "lock", "保护手动修改", "再次导出只安全更新有导出记录的 TXT；图片哈希异常会停止，手动修改不会被静默覆盖。");
                    hint(ui, "所有档位符合条件的图片均可导出；默认扁平结构，每张图片配一个同名 TXT。");
                });
            }
        }
            });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(
                        !self.demo
                            && !self.is_capture()
                            && !self.workflow_active()
                            && (!self.ingest_active() || kind == "去重")
                            && !self.action_snapshot.is_empty()
                            && matches!(kind, "优化" | "去重" | "导出"),
                        |ui| {
                            primary(
                                ui,
                                if kind == "去重" {
                                    "开始筛查"
                                } else {
                                    kind
                                },
                            )
                        },
                    )
                    .inner
                    .clicked()
                {
                    let ids = self.action_ids();
                    let operation = if kind == "去重" {
                        "dedup"
                    } else if kind == "导出" {
                        "export"
                    } else {
                        "optimize"
                    };
                    match self.start_workflow(ui.ctx(), operation, ids, None) {
                        Ok(()) if self.workflow_active() => {
                            self.notify(ui.ctx(), "任务已启动，可在总览查看进度。")
                        }
                        Ok(()) => {}
                        Err(error) => self.workflows.error = Some(error),
                    }
                }
                let close = ui.button("返回").clicked();
                if kind == "导出" {
                    ui.menu_button("更多", |ui| {
                        ui.add_enabled_ui(!self.workflows.export_multires, |ui| {
                            ui.checkbox(&mut self.workflows.export_folders, "每张图片独立文件夹");
                        });
                    });
                }
                close
            })
            .inner
        })
        .inner
    }
}

fn form_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.label(RichText::new(label).size(13.0));
    ui.add_sized(
        [460.0, 36.0],
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);
}
