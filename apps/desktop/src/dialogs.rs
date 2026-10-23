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
                if self.prefs.page == Page::Artist {
                    "添加画师"
                } else {
                    "添加角色"
                }
            }
            Dialog::Action(kind) => kind,
            Dialog::Profile(_) => "API 配置",
            Dialog::ImageUpload(_) => "确认发送图片",
            Dialog::CharacterRelations => "角色关系设置",
            Dialog::Similarity => "相似图审核",
            Dialog::Media(_) => "图片审核",
            Dialog::Delete { .. } => "确认删除",
            Dialog::DeleteCollections(scope) => {
                if scope.page == Page::Artist {
                    "删除画师"
                } else {
                    "删除角色"
                }
            }
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
                .frame(card().inner_margin(22.0))
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
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
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

    fn add_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        ui.set_min_width(460.0);
        ui.set_max_width(460.0);
        ui.label("Danbooru Tag");
        ui.add(
            egui::Label::new(
                RichText::new(if self.catalog.anonymous {
                    "公开查询 · 不使用账号"
                } else if self.danbooru.saved_username.is_some() {
                    "使用已保存的 Danbooru 账号"
                } else {
                    "公开查询 · 未配置账号"
                })
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
                "{} · API 总数 {} · 本地图片 0",
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
            if matches!(
                self.catalog.lookup_error,
                Some(
                    bdm_ports::CatalogError::Authentication | bdm_ports::CatalogError::Credentials
                )
            ) {
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
            if matches!(
                self.catalog.lookup_error,
                Some(
                    bdm_ports::CatalogError::WebsiteVerification
                        | bdm_ports::CatalogError::AccessDenied
                )
            ) {
                ui.add_space(8.0);
                ui.hyperlink_to("在浏览器中检查 Danbooru", "https://danbooru.donmai.us/");
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
        ui.horizontal(|ui| {
            ui.label("最大并发");
            ui.add(egui::DragValue::new(&mut self.profile_draft.concurrency).range(1..=16));
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("每批数量");
            ui.add(egui::DragValue::new(&mut self.profile_draft.request.batch_size).range(1..=8));
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
            ui.label("仅连接失败、超时或服务端暂时故障");
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
        ui.label(
            RichText::new("所有模式都在本地严格解析 JSON，并按图片 ID 校验结果。")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        ui.checkbox(
            &mut self.profile_draft.request.send_images,
            "生成自然语言描述时附带图片",
        );
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
        ui.label(
            RichText::new(
                "发送前在内存内等比例缩小，小图不放大；最长边不超过 1024 px，单图 JPEG 自动压到 1 MiB 内。每次发送前确认范围，标签优化只发送文本。",
            )
                .size(12.0)
                .color(MUTED),
        );
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
        let selected_count = self
            .action_snapshot
            .iter()
            .filter(|id| self.selected_media.contains(id))
            .count();
        if kind != "去重" {
            section(ui, "范围", "");
            ui.add_space(12.0);
        }
        ui.horizontal_wrapped(|ui| {
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
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);
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
                            .range(0.85..=0.995)
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
            ui.label(if kind == "导出" {
                "文本格式"
            } else {
                "优化目标"
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.action_mode, 0, "自然语言");
                ui.selectable_value(&mut self.action_mode, 1, "优化标签");
            });
            if kind == "优化" {
                let selected = self
                    .prefs
                    .profiles
                    .get(self.workflows.profile)
                    .map(|p| p.name.as_str())
                    .unwrap_or("选择 API 配置");
                egui::ComboBox::from_id_salt("optimization_profile")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, profile) in self.prefs.profiles.iter().enumerate() {
                            ui.selectable_value(&mut self.workflows.profile, i, &profile.name);
                        }
                    });
                ui.label(
                    RichText::new(
                        "文本请求仅使用普通标签；画师、角色和受保护词本地保留。原始标签不会改写。",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            }
            if kind == "导出" {
                ui.add(
                    egui::TextEdit::singleline(&mut self.workflows.export_root)
                        .hint_text("导出目录，例如 D:\\DatasetExport")
                        .desired_width(410.0),
                );
                ui.label(RichText::new("导出已保留且所选文本完整的图片。已存在文件须通过哈希和文本校验；不一致时停止并提示。").size(12.0).color(MUTED));
            }
        }
        ui.add_space(14.0);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.demo
                        && !self.is_capture()
                        && !self.workflow_active()
                        && (!self.ingest_active() || kind == "去重")
                        && !self.action_snapshot.is_empty()
                        && matches!(kind, "优化" | "去重" | "导出"),
                    egui::Button::new(if kind == "去重" {
                        "开始筛查"
                    } else {
                        kind
                    }),
                )
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
                if let Err(e) = self.start_workflow(ui.ctx(), operation, ids, None) {
                    self.workflows.error = Some(e);
                }
            }
            ui.button("返回").clicked()
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
