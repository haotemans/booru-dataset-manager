use crate::{
    app::{BdmApp, Dialog},
    model::*,
    theme::*,
    widgets::*,
};
use eframe::egui::{self, vec2, RichText, Sense};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionDeletion {
    pub page: Page,
    pub ids: BTreeSet<usize>,
    pub media_ids: BTreeSet<usize>,
}

impl BdmApp {
    pub fn collection_deletion_scope(
        &self,
        page: Page,
        ids: &BTreeSet<usize>,
    ) -> Option<CollectionDeletion> {
        if !matches!(page, Page::Artist | Page::Character) {
            return None;
        }
        let ids: BTreeSet<_> = self
            .library
            .collections(page)
            .iter()
            .filter(|record| ids.contains(&record.id))
            .map(|record| record.id)
            .collect();
        if ids.is_empty() {
            return None;
        }
        let media_ids = self
            .associated_media(page, &ids)
            .into_iter()
            .map(|media| media.id)
            .collect();
        Some(CollectionDeletion {
            page,
            ids,
            media_ids,
        })
    }

    pub fn open_collection_deletion(&mut self, page: Page, ids: &BTreeSet<usize>) {
        if let Some(scope) = self.collection_deletion_scope(page, ids) {
            self.form_error.clear();
            self.dialog = Some(Dialog::DeleteCollections(scope));
        }
    }

    pub fn confirm_sample_collection_deletion(
        &mut self,
        scope: &CollectionDeletion,
    ) -> Result<(usize, usize), &'static str> {
        if !self.demo {
            return Err("记录删除服务尚未接入");
        }
        if self
            .collection_deletion_scope(scope.page, &scope.ids)
            .as_ref()
            != Some(scope)
        {
            return Err("删除范围已变化，请关闭后重新确认");
        }
        let page = scope.page;
        let records: Vec<_> = self
            .library
            .collections(page)
            .iter()
            .filter(|record| scope.ids.contains(&record.id))
            .cloned()
            .collect();
        let removed: BTreeSet<_> = records.iter().map(|record| record.id).collect();
        let collections = if page == Page::Artist {
            &mut self.library.artists
        } else {
            &mut self.library.characters
        };
        collections.retain(|record| !removed.contains(&record.id));
        // Media is unique across views: deleting it removes every collection's reference.
        self.remove_sample_media(&scope.media_ids);
        self.action_snapshot.clear();
        self.collection_delete_mode = false;
        if self.prefs.page == page {
            self.selected.retain(|id| !removed.contains(id));
            if self.context.is_some_and(|id| removed.contains(&id)) {
                self.gallery = false;
                self.context = None;
                self.query.clear();
                self.selected_media.clear();
                self.review_order.clear();
            }
            let total = self
                .library
                .collections(page)
                .iter()
                .filter(|record| {
                    record
                        .tag
                        .to_lowercase()
                        .contains(&self.query.trim().to_lowercase())
                })
                .count();
            self.page_number = self
                .page_number
                .min(total.div_ceil(self.prefs.page_size).max(1) - 1);
        }
        for record in &records {
            self.log(
                "INFO",
                &format!(
                    "已删除示例{}记录 {} 及其关联图片（仅内存）。",
                    page.title(),
                    record.tag
                ),
            );
        }
        for id in &scope.media_ids {
            self.log(
                "INFO",
                &format!(
                    "随{}删除示例 sample_{:03}.png 及其专属数据和全部引用（仅内存）。",
                    page.title(),
                    id + 1
                ),
            );
        }
        self.dialog = None;
        Ok((records.len(), scope.media_ids.len()))
    }

    pub fn can_delete_empty_collection_scope(&self, scope: &CollectionDeletion) -> bool {
        !self.demo
            && !self.is_capture()
            && self.catalog.repository.is_some()
            && !self.ingest_active()
            && !self.workflow_active()
            && self
                .collection_deletion_scope(scope.page, &scope.ids)
                .as_ref()
                == Some(scope)
    }

    pub fn confirm_collection_deletion(
        &mut self,
        scope: &CollectionDeletion,
    ) -> Result<(usize, usize), String> {
        if self.demo {
            return self
                .confirm_sample_collection_deletion(scope)
                .map_err(str::to_owned);
        }
        if !self.can_delete_empty_collection_scope(scope) {
            return Err("删除范围已变化或任务正在处理图片，请稍后重新确认。".into());
        }
        let ids: Vec<i64> = scope
            .ids
            .iter()
            .map(|id| i64::try_from(*id))
            .collect::<Result<_, _>>()
            .map_err(|_| "记录 ID 无效。".to_owned())?;
        let kind = crate::catalog::collection_kind(scope.page)
            .ok_or_else(|| "删除范围无效。".to_owned())?;
        if !scope.media_ids.is_empty() {
            let mut media_repo = bdm_infrastructure::catalog::SqliteCatalog::open(
                self.catalog.path.as_deref().ok_or("本地收藏未打开。")?,
            )
            .map_err(|e| e.to_string())?;
            media_repo
                .delete_media(
                    &scope.media_ids.iter().map(|id| *id as i64).collect(),
                    None,
                    "human-confirmed collection deletion",
                )
                .map_err(|e| e.to_string())?;
        }
        let repo = self.catalog.repository.as_mut().ok_or("本地收藏未打开。")?;
        bdm_application::catalog::delete_empty_collections(repo.as_mut(), kind, &ids)
            .map_err(|e| e.to_string())?;
        self.remove_sample_media(&scope.media_ids);
        self.reload_media()?;
        let items = if scope.page == Page::Artist {
            &mut self.library.artists
        } else {
            &mut self.library.characters
        };
        items.retain(|item| !scope.ids.contains(&item.id));
        self.navigate(scope.page);
        self.action_snapshot.clear();
        self.dialog = None;
        self.log(
            "INFO",
            &format!(
                "已删除 {} 个无图片的本地{}记录。",
                scope.ids.len(),
                scope.page.title()
            ),
        );
        Ok((scope.ids.len(), scope.media_ids.len()))
    }

    pub fn collection_deletion_dialog(
        &mut self,
        ui: &mut egui::Ui,
        scope: &CollectionDeletion,
    ) -> bool {
        let page = scope.page;
        let records: Vec<_> = self
            .library
            .collections(page)
            .iter()
            .filter(|record| scope.ids.contains(&record.id))
            .cloned()
            .collect();
        let image_count = scope.media_ids.len();
        ui.horizontal(|ui| {
            badge(ui, &format!("{} 个{}", records.len(), page.title()), DANGER);
            badge(ui, &format!("删除 {image_count} 张图片"), DANGER);
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(if !self.demo && image_count == 0 {
                "所选记录暂无本地图片，确认后删除收藏记录，无法撤销。"
            } else {
                "图片及其数据、其他页面的同图引用一并删除，无法撤销。"
            })
            .size(13.0)
            .color(MUTED),
        );
        if self.demo {
            ui.label(
                RichText::new("示例记录 · 仅影响本次会话")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("collection_deletion_list")
            .max_height(260.0)
            .show(ui, |ui| {
                for record in &records {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        icon(ui, rect, page.icon(), MUTED);
                        ui.label(&record.tag);
                    });
                    ui.add_space(8.0);
                }
                for id in &scope.media_ids {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(66.0, 48.0), Sense::hover());
                        self.paint_media(ui, rect, *id);
                        ui.label(self.media_name(*id));
                    });
                    ui.add_space(8.0);
                }
            });
        if !self.form_error.is_empty() {
            ui.colored_label(DANGER, &self.form_error);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("取消").clicked() {
                return true;
            }
            if ui
                .add_enabled(
                    (self.demo || self.can_delete_empty_collection_scope(scope))
                        && !records.is_empty(),
                    egui::Button::new(
                        RichText::new(format!(
                            "删除 {} 个{}及 {image_count} 张图片",
                            records.len(),
                            page.title()
                        ))
                        .color(egui::Color32::WHITE),
                    )
                    .fill(DANGER),
                )
                .on_disabled_hover_text("处理任务运行时请稍后删除；范围变化后须重新确认。")
                .clicked()
            {
                match self.confirm_collection_deletion(scope) {
                    Ok((count, images)) => {
                        self.notify(
                            ui.ctx(),
                            &format!("已删除 {count} 个{}及 {images} 张图片", page.title()),
                        );
                        return true;
                    }
                    Err(error) => self.form_error = error,
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

    fn sample(page: Page) -> BdmApp {
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.navigate(page);
        app
    }

    #[test]
    fn deleting_artists_removes_unique_media_from_every_view_and_prunes_state() {
        let mut app = sample(Page::Artist);
        let expected: Vec<_> = app
            .library
            .media
            .iter()
            .filter(|m| !matches!(m.artist, Some(1 | 4)))
            .cloned()
            .collect();
        let characters = app.library.characters.clone();
        let scope = app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([1, 4]))
            .unwrap();
        assert_eq!(scope.media_ids.len(), 8);
        app.selected = BTreeSet::from([1, 4, 5]);
        app.selected_media = BTreeSet::from([1, 2, 4]);
        app.reject_media(1);
        app.reject_media(2);
        app.review_order = vec![1, 2, 4, 5];
        app.action_snapshot = vec![1, 4, 5];
        app.page_number = 9;
        assert_eq!(app.confirm_sample_collection_deletion(&scope), Ok((2, 8)));
        assert_eq!(app.library.media, expected);
        assert_eq!(
            app.library.artists.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![0, 2, 3, 5]
        );
        assert_eq!(app.library.characters, characters);
        assert_eq!(app.selected, BTreeSet::from([5]));
        assert_eq!(app.selected_media, BTreeSet::from([2]));
        assert_eq!(app.pending_deletion, BTreeSet::from([2]));
        assert_eq!(app.review_order, vec![2, 5]);
        assert!(app.action_snapshot.is_empty());
        assert_eq!(app.page_number, 0);
        assert_eq!(app.library.coverage(), [16, 8, 8, 4]);
        for character in &app.library.characters {
            assert!(app
                .library
                .associated(Page::Character, &BTreeSet::from([character.id]))
                .iter()
                .all(|m| !scope.media_ids.contains(&m.id)));
        }
    }

    #[test]
    fn deleting_character_removes_shared_images_but_keeps_unselected_artist_records() {
        let mut app = sample(Page::Character);
        let expected: Vec<_> = app
            .library
            .media
            .iter()
            .filter(|m| m.character != Some(0))
            .cloned()
            .collect();
        let artists = app.library.artists.clone();
        let scope = app
            .collection_deletion_scope(Page::Character, &BTreeSet::from([0]))
            .unwrap();
        app.gallery = true;
        app.context = Some(0);
        app.selected_media.insert(0);
        app.open_media(0);
        assert_eq!(app.confirm_sample_collection_deletion(&scope), Ok((1, 4)));
        assert_eq!(app.library.media, expected);
        assert_eq!(app.library.artists, artists);
        assert_eq!(app.library.characters.len(), 5);
        assert!(app.context.is_none());
        assert!(!app.gallery);
        assert!(app.selected_media.is_empty());
        assert!(app.review_order.is_empty());
        assert!(app
            .library
            .associated(Page::Character, &BTreeSet::from([0]))
            .is_empty());
        assert_eq!(
            app.library
                .associated(Page::Artist, &BTreeSet::from([0]))
                .len(),
            2
        );
        assert_eq!(
            app.library
                .associated(Page::Artist, &BTreeSet::from([1]))
                .len(),
            2
        );
    }

    #[test]
    fn collection_confirmation_freezes_selection_and_escape_preserves_records() {
        let mut app = sample(Page::Artist);
        let original = app.library.artists.clone();
        let media = app.library.media.clone();
        app.selected = BTreeSet::from([0, 2]);
        app.open_collection_deletion(Page::Artist, &app.selected.clone());
        app.selected = BTreeSet::from([3]);
        app.query = "sample_artist_f".into();
        let Some(Dialog::DeleteCollections(scope)) = &app.dialog else {
            panic!("missing confirmation")
        };
        let scope = scope.clone();
        assert_eq!(scope.page, Page::Artist);
        assert_eq!(scope.ids, BTreeSet::from([0, 2]));
        assert_eq!(
            scope.media_ids,
            BTreeSet::from([0, 2, 6, 8, 12, 14, 18, 20])
        );
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1050.0, 720.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.show_dialog(ctx),
        );
        assert!(app.dialog.is_none());
        assert_eq!(app.library.artists, original);
        assert_eq!(app.library.media, media);
        // A later confirmation still only removes its captured scope.
        assert_eq!(app.confirm_sample_collection_deletion(&scope), Ok((2, 8)));
        assert!(app.library.artists.iter().any(|record| record.id == 3));
        assert!(app
            .library
            .artists
            .iter()
            .all(|record| ![0, 2].contains(&record.id)));
        assert!(app
            .library
            .media
            .iter()
            .all(|m| !scope.media_ids.contains(&m.id)));
        assert!(app.library.media.iter().any(|m| m.artist == Some(3)));
    }

    #[test]
    fn production_record_deletion_and_invalid_scopes_cannot_report_success() {
        let mut app = sample(Page::Artist);
        let original = app.library.artists.clone();
        let media = app.library.media.clone();
        let scope = app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([0]))
            .unwrap();
        app.demo = false;
        assert!(app.confirm_sample_collection_deletion(&scope).is_err());
        assert_eq!(app.library.artists, original);
        assert_eq!(app.library.media, media);
        app.demo = true;
        app.open_collection_deletion(Page::Settings, &BTreeSet::from([0]));
        assert!(app.dialog.is_none());
        assert!(app
            .confirm_sample_collection_deletion(&CollectionDeletion {
                page: Page::Settings,
                ..scope
            })
            .is_err());
        assert!(app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([99]))
            .is_none());
        assert_eq!(app.library.media, media);
    }

    #[test]
    fn changed_media_scope_requires_reconfirmation_before_any_mutation() {
        let mut app = sample(Page::Artist);
        let scope = app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([0]))
            .unwrap();
        // A newly associated image was not in the reviewed confirmation list.
        app.library.media[1].artist = Some(0);
        let records = app.library.artists.clone();
        let media = app.library.media.clone();
        assert!(app.confirm_sample_collection_deletion(&scope).is_err());
        assert_eq!(app.library.artists, records);
        assert_eq!(app.library.media, media);
        // A supplied scope must never include an unrelated image.
        let mut forged = app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([0]))
            .unwrap();
        forged.media_ids.insert(5);
        assert!(app.confirm_sample_collection_deletion(&forged).is_err());
        assert_eq!(app.library.media, media);
    }

    #[test]
    fn empty_collection_and_last_collection_do_not_leave_orphaned_media() {
        let mut app = sample(Page::Artist);
        // An empty collection can still be removed.
        app.library.media.retain(|m| m.artist != Some(0));
        let scope = app
            .collection_deletion_scope(Page::Artist, &BTreeSet::from([0]))
            .unwrap();
        assert_eq!(app.confirm_sample_collection_deletion(&scope), Ok((1, 0)));
        let all = app.library.artists.iter().map(|r| r.id).collect();
        let scope = app.collection_deletion_scope(Page::Artist, &all).unwrap();
        assert_eq!(app.confirm_sample_collection_deletion(&scope), Ok((5, 20)));
        assert!(app.library.artists.is_empty());
        assert!(app.library.media.is_empty());
        assert_eq!(app.library.characters.len(), 6);
        assert_eq!(app.library.coverage(), [0, 0, 0, 0]);
    }
}
