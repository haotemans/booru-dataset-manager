use crate::{
    app::{BdmApp, Dialog},
    theme::*,
    widgets::*,
};
use eframe::egui::{self, vec2, RichText, Sense};

impl BdmApp {
    pub(crate) fn remove_sample_media(&mut self, ids: &std::collections::BTreeSet<usize>) {
        self.tags.invalidate();
        self.library.media.retain(|m| !ids.contains(&m.id));
        for id in ids {
            self.ingest.stored.remove(id);
            self.invalidate_review_texture(*id);
        }
        self.rebuild_collection_summaries();
        self.selected_media.retain(|id| !ids.contains(id));
        self.action_snapshot.retain(|id| !ids.contains(id));
        self.review_order.retain(|id| !ids.contains(id));
        self.pending_deletion.retain(|id| !ids.contains(id));
    }

    pub fn open_media(&mut self, id: usize) {
        self.form_error.clear();
        self.media_review.reset_canvas();
        self.review_order = if self.gallery {
            self.filtered_media().iter().map(|m| m.id).collect()
        } else {
            self.library.media.iter().rev().map(|m| m.id).collect()
        };
        if self.review_order.contains(&id) {
            self.dialog = Some(Dialog::Media(id));
        }
    }

    pub fn review_neighbor(&self, id: usize, forward: bool) -> Option<usize> {
        let ids: Vec<_> = self
            .review_order
            .iter()
            .copied()
            .filter(|id| self.library.media.iter().any(|m| m.id == *id))
            .collect();
        let position = ids.iter().position(|candidate| *candidate == id)?;
        let offset = if forward { 1 } else { ids.len() - 1 };
        Some(ids[(position + offset) % ids.len()])
    }

    pub fn reject_media(&mut self, id: usize) {
        // Rejection is only a proposed scope; never change media before confirmation.
        if self.library.media.iter().any(|m| m.id == id) {
            self.pending_deletion.insert(id);
        }
    }

    pub fn open_deletion_review(&mut self, return_media: Option<usize>) {
        if !self.pending_deletion.is_empty() {
            self.form_error.clear();
            self.dialog = Some(Dialog::Delete { return_media });
        }
    }

    pub fn cancel_deletion(&mut self, return_media: Option<usize>) {
        self.pending_deletion.clear();
        self.dialog = return_media
            .filter(|id| self.library.media.iter().any(|m| m.id == *id))
            .map(Dialog::Media);
    }

    pub fn confirm_sample_deletion(
        &mut self,
        return_media: Option<usize>,
    ) -> Result<usize, &'static str> {
        if !self.demo {
            if self.ingest_active() || self.workflow_active() || self.image_editor.active() {
                return Err("处理任务运行中，请稍后确认删除");
            }
            let path = self.catalog.path.as_deref().ok_or("本地数据库未打开")?;
            let mut repo = bdm_infrastructure::catalog::SqliteCatalog::open(path)
                .map_err(|_| "本地数据库未打开")?;
            let ids = self.pending_deletion.iter().map(|id| *id as i64).collect();
            repo.delete_media(&ids, None, "human-confirmed review deletion")
                .map_err(|_| "图片删除失败，请检查存储目录，重启后继续清理")?;
        }
        let removed: std::collections::BTreeSet<_> = self
            .library
            .media
            .iter()
            .filter(|m| self.pending_deletion.contains(&m.id))
            .map(|m| m.id)
            .collect();
        let next = return_media.and_then(|current| {
            let position = self.review_order.iter().position(|id| *id == current)?;
            (0..self.review_order.len())
                .map(|offset| self.review_order[(position + offset) % self.review_order.len()])
                .find(|id| !removed.contains(id) && self.library.media.iter().any(|m| m.id == *id))
        });
        let before = self.library.media.len();
        self.remove_sample_media(&removed);
        let count = before - self.library.media.len();
        self.pending_deletion.clear();
        let pages = self
            .filtered_media()
            .len()
            .div_ceil(self.prefs.page_size)
            .max(1);
        self.page_number = self.page_number.min(pages - 1);
        for id in removed {
            self.log(
                "INFO",
                &format!("人工确认删除图片 ID {} 及其专属数据。", id),
            );
        }
        self.dialog = next.map(Dialog::Media);
        Ok(count)
    }

    pub fn deletion_queue_button(
        &mut self,
        ui: &mut egui::Ui,
        return_media: Option<usize>,
    ) -> bool {
        if self.pending_deletion.is_empty() {
            return false;
        }
        let clicked = ui
            .horizontal(|ui| {
                let clicked = icon_button(ui, "trash", "核对待删除清单", false).clicked();
                if ui
                    .link(
                        RichText::new(format!("待删除 {}", self.pending_deletion.len()))
                            .color(DANGER),
                    )
                    .clicked()
                {
                    return true;
                }
                clicked
            })
            .inner;
        if clicked {
            self.open_deletion_review(return_media);
        }
        clicked
    }

    pub fn deletion_dialog(&mut self, ui: &mut egui::Ui, return_media: Option<usize>) -> bool {
        ui.label(RichText::new("确认后立即删除，无法撤销。").color(DANGER));
        if self.demo {
            ui.label(
                RichText::new("示例记录 · 仅影响本次会话")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        ui.add_space(8.0);
        let ids: Vec<_> = self.pending_deletion.iter().copied().collect();
        egui::ScrollArea::vertical()
            .id_salt("deletion_list")
            .max_height(270.0)
            .show(ui, |ui| {
                for id in ids {
                    ui.push_id(id, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(66.0, 48.0), Sense::hover());
                            self.paint_media(ui, rect, id);
                            ui.label(self.media_name(id));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icon_button(ui, "close", "从清单移除，保留图片", false)
                                        .clicked()
                                    {
                                        self.pending_deletion.remove(&id);
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                    });
                }
            });
        ui.add_space(8.0);
        ui.label(RichText::new("原图 · 标签 / 描述 · 专属分析数据").size(12.0).color(MUTED))
            .on_hover_text("永久删除原图、侧车和专属记录；保留其他图片及共享模型、索引。清理失败时保留任务以便重试。");
        if !self.form_error.is_empty() {
            ui.colored_label(DANGER, &self.form_error);
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("取消").clicked() {
                self.cancel_deletion(return_media);
                return true;
            }
            let label = format!("确认删除 {} 张", self.pending_deletion.len());
            if ui
                .add_enabled(
                    (self.demo
                        || (self.catalog.path.is_some()
                            && !self.ingest_active()
                            && !self.workflow_active()))
                        && !self.pending_deletion.is_empty(),
                    egui::Button::new(RichText::new(label).color(egui::Color32::WHITE))
                        .fill(DANGER),
                )
                .on_disabled_hover_text("请等待处理任务结束后确认删除。")
                .clicked()
            {
                match self.confirm_sample_deletion(return_media) {
                    Ok(count) => {
                        self.notify(ui.ctx(), &format!("已删除 {count} 条示例记录"));
                        return true;
                    }
                    Err(error) => self.form_error = error.into(),
                }
            }
            false
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Preferences};
    use std::collections::BTreeSet;

    fn sample() -> BdmApp {
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.navigate(Page::Artist);
        app.gallery = true;
        app.open_media(23);
        app
    }

    #[test]
    fn rejection_and_cancel_preserve_media_metadata_and_associations() {
        let mut app = sample();
        let original = app.library.media.clone();
        app.reject_media(23);
        app.reject_media(4);
        app.open_deletion_review(Some(23));
        assert_eq!(app.library.media, original);
        app.cancel_deletion(Some(23));
        assert_eq!(app.library.media, original);
        assert!(app.pending_deletion.is_empty());
        assert!(matches!(app.dialog, Some(Dialog::Media(23))));
    }

    #[test]
    fn confirmed_scope_only_deletes_listed_items_and_prunes_stale_references() {
        let mut app = sample();
        let original = app.library.media.clone();
        app.selected_media = BTreeSet::from([23, 22, 4]);
        app.action_snapshot = vec![23, 22, 4];
        app.page_number = 1;
        for id in [23, 22, 21, 20, 19] {
            app.reject_media(id);
        }
        // Removing a row from the confirmation list keeps it, even if selected.
        app.pending_deletion.remove(&22);
        assert_eq!(app.confirm_sample_deletion(Some(23)), Ok(4));
        assert_eq!(
            app.library.media,
            original
                .into_iter()
                .filter(|m| ![23, 21, 20, 19].contains(&m.id))
                .collect::<Vec<_>>()
        );
        assert_eq!(app.selected_media, BTreeSet::from([22, 4]));
        assert_eq!(app.action_snapshot, vec![22, 4]);
        assert_eq!(app.page_number, 0);
        assert!(matches!(app.dialog, Some(Dialog::Media(22))));
        assert_eq!(app.review_neighbor(22, true), Some(18));
        assert_eq!(app.review_neighbor(0, true), Some(22));
        assert_eq!(app.library.coverage()[0], 20);
        assert_eq!(app.library.artists.len(), 6);
        assert_eq!(app.library.characters.len(), 6);
        assert_eq!(
            app.logs
                .iter()
                .filter(|entry| entry.message.contains("人工确认删除"))
                .count(),
            4
        );
    }

    #[test]
    fn context_navigation_stays_in_scope_and_deleting_last_item_closes_detail() {
        let mut app = sample();
        app.context = Some(0);
        app.open_media(18);
        assert_eq!(app.review_neighbor(18, true), Some(12));
        for id in [18, 12, 6] {
            app.reject_media(id);
        }
        app.confirm_sample_deletion(Some(18)).unwrap();
        assert!(matches!(app.dialog, Some(Dialog::Media(0))));
        assert_eq!(app.review_neighbor(0, true), Some(0));
        app.reject_media(0);
        app.confirm_sample_deletion(Some(0)).unwrap();
        assert!(app.dialog.is_none());
        assert!(app.scope_media().is_empty());
        assert_eq!(app.library.media.len(), 20);
        assert_eq!(app.review_neighbor(0, true), None);
    }

    #[test]
    fn production_deletion_requires_an_adapter_and_cannot_claim_success() {
        let mut app = sample();
        app.demo = false;
        app.reject_media(23);
        assert!(app.confirm_sample_deletion(Some(23)).is_err());
        assert_eq!(app.library.media.len(), 24);
        assert!(app.pending_deletion.contains(&23));
    }

    #[test]
    fn leaving_review_opens_confirmation_and_escape_cancels_without_deleting() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = sample();
        let original = app.library.media.clone();
        app.reject_media(23);
        let frame = |app: &mut BdmApp, pressed: Option<bool>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1050.0, 720.0),
                    )),
                    events: pressed
                        .map(|pressed| egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: Some(egui::Key::Escape),
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        })
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                |ctx| app.show_dialog(ctx),
            );
        };
        frame(&mut app, None);
        frame(&mut app, Some(true));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Delete {
                return_media: Some(23)
            })
        ));
        frame(&mut app, Some(false));
        frame(&mut app, Some(true));
        assert!(matches!(app.dialog, Some(Dialog::Media(23))));
        assert!(app.pending_deletion.is_empty());
        assert_eq!(app.library.media, original);
    }
}
