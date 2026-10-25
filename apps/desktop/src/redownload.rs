use crate::{app::BdmApp, theme::*};
use eframe::egui::{self, vec2, RichText};
use std::sync::mpsc::{self, TryRecvError};

pub(crate) struct RedownloadNotice {
    pub(crate) id: usize,
    // None while downloading; retain the outcome until the user dismisses it.
    pub(crate) result: Option<Result<(), String>>,
}

impl BdmApp {
    pub(crate) fn capture_redownload_notice(&mut self) {
        if !self.is_capture() {
            return;
        }
        self.library = crate::model::Library::demo();
        self.open_media(3);
        self.image_editor.reset(3);
        self.image_editor.visible = true;
        self.image_editor.cropping = false;
        self.media_review.redownload_notice = Some(RedownloadNotice {
            id: 3,
            result: None,
        });
    }

    pub(crate) fn request_redownload(&mut self, ctx: &egui::Context, id: usize) {
        if let Some((active_id, _)) = &self.media_review.redownload {
            self.media_review.redownload_notice = Some(RedownloadNotice {
                id: *active_id,
                result: None,
            });
            return;
        }
        self.image_editor.error.clear();
        self.form_error.clear();
        let result = self.start_redownload(ctx, id);
        if let Err(error) = &result {
            self.image_editor.error = error.clone();
            self.notify(ctx, error);
        }
        self.media_review.redownload_notice = Some(RedownloadNotice {
            id,
            result: result.err().map(Err),
        });
        ctx.request_repaint();
    }

    fn start_redownload(&mut self, ctx: &egui::Context, id: usize) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("当前处于示例预览模式，不能下载真实原图。".into());
        }
        if self.image_editor.active() {
            return Err("当前有图片处理任务正在运行，请等待完成。".into());
        }
        if self.ingest_active() {
            return Err("当前正在导入图片，请等待导入完成。".into());
        }
        if self.workflow_active() {
            return Err("当前有工作流正在运行，请等待完成。".into());
        }
        if self.image_editor.preview.is_some() {
            return Err("当前存在未保存的编辑预览，请先确认或放弃预览。".into());
        }
        if !self.ingest.stored.get(&id).is_some_and(|media| {
            media
                .post
                .file_url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty())
        }) {
            return Err("当前记录没有来源原图地址，无法自动下载。".into());
        }
        let path = self.catalog.path.clone().ok_or("本地数据库未打开。")?;
        let credentials = self
            .saved_danbooru_credentials()
            .map_err(|error| error.to_string())?;
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("redownload-original".into())
            .spawn(move || {
                let result = (|| {
                    let mut repo = bdm_infrastructure::catalog::SqliteCatalog::open(&path)
                        .map_err(|error| error.to_string())?;
                    let provider = bdm_infrastructure::danbooru::DanbooruProvider::new(credentials)
                        .map_err(|error| error.to_string())?;
                    repo.redownload_original(
                        id as i64,
                        &provider,
                        &std::sync::atomic::AtomicBool::new(false),
                    )
                    .map_err(|error| error.to_string())
                })();
                let _ = sender.send(result);
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动重新下载任务。".to_owned())?;
        self.media_review.redownload = Some((id, receiver));
        Ok(())
    }

    pub(crate) fn poll_redownload(&mut self, ctx: &egui::Context) {
        let Some((id, receiver)) = &self.media_review.redownload else {
            return;
        };
        let id = *id;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("重新下载任务意外结束，请重试。".into()),
        };
        self.media_review.redownload = None;
        let outcome = match result {
            Ok(media) => {
                self.editor_apply_saved(ctx, media);
                self.form_error.clear();
                self.notify(ctx, "原图下载完成，已替换当前图片；旧文件已保留备份。");
                Ok(())
            }
            Err(error) => {
                if matches!(self.dialog, Some(crate::app::Dialog::Media(current)) if current == id)
                {
                    self.image_editor.error = error.clone();
                    self.form_error = error.clone();
                }
                self.notify(ctx, &format!("原图下载失败：{error}"));
                Err(error)
            }
        };
        self.media_review.redownload_notice = Some(RedownloadNotice {
            id,
            result: Some(outcome),
        });
    }

    pub(crate) fn show_redownload_notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = &self.media_review.redownload_notice else {
            return;
        };
        let running = notice.result.is_none();
        let mut close = false;
        egui::Window::new("重新下载原图")
            .id(egui::Id::new("redownload_notice"))
            .order(egui::Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .fixed_size(vec2(420.0, 180.0))
            .frame(card().inner_margin(22.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("重新下载原图").font(medium(18.0)));
                ui.label(
                    RichText::new(format!("图片 #{}", notice.id))
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add_space(12.0);
                match &notice.result {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("正在下载原图并校验…");
                        });
                        ui.label("校验通过后替换当前图片，并保留当前文件备份。");
                    }
                    Some(Ok(())) => {
                        ui.label(RichText::new("原图下载完成").color(ACCENT));
                        ui.label("已替换当前图片并刷新预览，旧文件已保留备份。");
                    }
                    Some(Err(error)) => {
                        ui.label(RichText::new("未能完成原图下载").color(DANGER));
                        ui.label(error);
                    }
                }
                ui.add_space(16.0);
                close = ui
                    .add_enabled(!running, egui::Button::new("关闭"))
                    .clicked();
            });
        // Escape belongs to this popup, never to the underlying review window.
        let escape =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if !running && (close || escape) {
            self.media_review.redownload_notice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::Dialog, model::Preferences};

    fn frame(ctx: &egui::Context, app: &mut BdmApp, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1050.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.shell(ctx),
        )
    }

    fn text_in(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.text().contains(expected),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| text_in(shape, expected)),
            _ => false,
        }
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn redownload_click_shows_popup_and_toast_over_review_and_escape_only_closes_popup() {
        for cropping in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
            app.open_media(3);
            app.image_editor.visible = true;
            app.image_editor.cropping = cropping;
            app.prefs.confirm_image_edits = false;
            for _ in 0..3 {
                frame(&ctx, &mut app, vec![]);
            }
            {
                let menu = app.image_editor.more_button.unwrap().center();
                for pressed in [true, false] {
                    frame(
                        &ctx,
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(menu),
                            egui::Event::PointerButton {
                                pos: menu,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                }
                frame(&ctx, &mut app, vec![]);
            }
            let pos = app.image_editor.redownload_button.unwrap().center();
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut app,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    }],
                );
            }
            assert!(matches!(
                app.media_review.redownload_notice.as_ref().unwrap().result,
                Some(Err(_))
            ));
            assert!(app.media_review.redownload.is_none());
            let output = frame(&ctx, &mut app, vec![]);
            let popup =
                egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("redownload_notice"));
            let rect = ctx.memory(|memory| memory.area_rect(popup.id)).unwrap();
            assert_eq!(ctx.layer_id_at(rect.center()), Some(popup));
            assert!(output
                .shapes
                .iter()
                .any(|shape| text_in(&shape.shape, "未能完成原图下载")));
            assert!(
                ctx.memory(|memory| memory.area_rect(egui::Id::new("toast")))
                    .is_some(),
                "review must render global notifications"
            );
            frame(&ctx, &mut app, vec![key(egui::Key::Escape)]);
            assert!(app.media_review.redownload_notice.is_none());
            assert!(matches!(app.dialog, Some(Dialog::Media(3))));
        }
    }

    #[test]
    fn redownload_progress_blocks_review_shortcuts_and_keeps_async_errors_visible() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.open_media(3);
        frame(&ctx, &mut app, vec![]);
        let (sender, receiver) = mpsc::channel();
        app.media_review.redownload = Some((3, receiver));
        app.request_redownload(&ctx, 3);
        let media = app.library.media.clone();
        for key_code in [egui::Key::ArrowRight, egui::Key::Num3, egui::Key::Escape] {
            frame(&ctx, &mut app, vec![key(key_code)]);
            assert!(matches!(app.dialog, Some(Dialog::Media(3))));
            assert!(app
                .media_review
                .redownload_notice
                .as_ref()
                .unwrap()
                .result
                .is_none());
            assert_eq!(app.library.media, media);
        }
        sender.send(Err("测试：来源暂时不可用".into())).unwrap();
        frame(&ctx, &mut app, vec![]);
        let output = frame(&ctx, &mut app, vec![]);
        assert!(app.media_review.redownload.is_none());
        assert!(output
            .shapes
            .iter()
            .any(|shape| text_in(&shape.shape, "测试：来源暂时不可用")));
        assert_eq!(app.image_editor.error, "测试：来源暂时不可用");
        assert_eq!(app.library.media, media);
    }

    #[test]
    fn redownload_disconnect_is_reported_even_after_review_closes() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        let (sender, receiver) = mpsc::channel();
        app.media_review.redownload = Some((3, receiver));
        drop(sender);
        frame(&ctx, &mut app, vec![]);
        let notice = app.media_review.redownload_notice.as_ref().unwrap();
        assert_eq!(notice.id, 3);
        assert!(notice
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .contains("意外结束"));
        assert!(app.media_review.redownload.is_none());
    }
}
