use crate::{app::BdmApp, model::*, theme::*, widgets::*};
use eframe::egui::{self, vec2, RichText, Sense};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};

#[derive(Clone)]
struct TagRow {
    tag: String,
    count: usize,
    kind: &'static str,
    search: String,
}

#[derive(Default)]
struct TagSummary {
    rows: Vec<TagRow>,
    warnings: Vec<(usize, String)>,
}

impl TagSummary {
    fn build(app: &BdmApp) -> Self {
        let mut counts = BTreeMap::<String, usize>::new();
        let mut associations = BTreeMap::<usize, usize>::new();
        for media in &app.library.media {
            let unique: BTreeSet<_> = media.tags.iter().collect();
            for tag in unique {
                *counts.entry(tag.clone()).or_default() += 1;
            }
            if let Some(stored) = app.ingest.stored.get(&media.id) {
                for id in &stored.collections {
                    *associations.entry(*id as usize).or_default() += 1;
                }
            }
        }
        let mut rows: BTreeMap<String, TagRow> = counts
            .into_iter()
            .map(|(tag, count)| {
                let kind = match app.workflows.rules.categories.get(&tag) {
                    Some(1) => "画师",
                    Some(4) => "角色",
                    Some(3) => "作品",
                    Some(5) => "元数据",
                    Some(0) => "通用",
                    _ if app.demo => "通用",
                    _ => "未解析",
                };
                (
                    tag.clone(),
                    TagRow {
                        search: tag.to_lowercase(),
                        tag,
                        count,
                        kind,
                    },
                )
            })
            .collect();
        for (collections, kind) in [
            (&app.library.artists, "画师"),
            (&app.library.characters, "角色"),
        ] {
            for collection in collections {
                rows.entry(collection.tag.clone()).or_insert_with(|| TagRow {
                    tag: collection.tag.clone(), search: collection.tag.to_lowercase(), kind,
                    count: if app.demo {
                        app.library.media.iter().filter(|m| if kind == "画师" {m.artist} else {m.character} == Some(collection.id)).count()
                    } else { associations.get(&collection.id).copied().unwrap_or(0) },
                });
            }
        }
        let warnings = app
            .ingest
            .stored
            .values()
            .flat_map(|media| {
                bdm_domain::tags::normalize(
                    &media.post.tag_string,
                    &media.post.tag_string_artist,
                    &media.post.tag_string_character,
                    &app.workflows.rules,
                )
                .warnings
                .into_iter()
                .map(move |warning| (media.id as usize, warning))
            })
            .collect();
        Self {
            rows: rows.into_values().collect(),
            warnings,
        }
    }
}

#[derive(Default)]
pub struct TagPageState {
    summary: Option<Arc<TagSummary>>,
    filtered_summary: Option<Arc<TagSummary>>,
    filtered: Vec<usize>,
    query: String,
    tab: usize,
    page: usize,
    warning_page: usize,
    reset_scroll: bool,
    #[cfg(test)]
    rendered_rows: usize,
}

impl TagPageState {
    pub fn invalidate(&mut self) {
        self.summary = None;
    }

    fn filter(&mut self, summary: &Arc<TagSummary>, query: &str, tab: usize) -> bool {
        let query = query.trim().to_lowercase();
        let new_summary = self
            .filtered_summary
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, summary));
        let changed = self.query != query || self.tab != tab;
        if !new_summary && !changed {
            return false;
        }
        if changed {
            self.page = 0;
            self.warning_page = 0;
            self.reset_scroll = true;
        }
        self.filtered = summary
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.search.contains(&query)
                    && match tab {
                        1 => row.kind == "通用",
                        2 => row.kind != "通用",
                        3 => row.kind == "未解析",
                        _ => true,
                    }
            })
            .map(|(index, _)| index)
            .collect();
        self.query = query;
        self.tab = tab;
        self.filtered_summary = Some(summary.clone());
        true
    }
}

fn page_range(total: usize, size: usize, page: &mut usize) -> Range<usize> {
    let size = size.max(1);
    *page = (*page).min(total.div_ceil(size).max(1) - 1);
    let start = *page * size;
    start..(start + size).min(total)
}

fn tag_pagination(ui: &mut egui::Ui, total: usize, size: usize, page: &mut usize, id: &str) {
    let range = page_range(total, size, page);
    let pages = total.div_ceil(size.max(1)).max(1);
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{}–{} / {total}",
                if total == 0 { 0 } else { range.start + 1 },
                range.end
            ));
            if ui
                .add_enabled(*page > 0, egui::Button::new("上一页"))
                .clicked()
            {
                *page -= 1;
            }
            let mut number = *page + 1;
            ui.add(
                egui::DragValue::new(&mut number)
                    .range(1..=pages)
                    .suffix(format!(" / {pages}")),
            );
            *page = number - 1;
            if ui
                .add_enabled(*page + 1 < pages, egui::Button::new("下一页"))
                .clicked()
            {
                *page += 1;
            }
        })
    });
}

impl BdmApp {
    pub fn tags_page(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        {
            self.tags.rendered_rows = 0;
        }
        if !self.demo && !self.is_capture() {
            if ui
                .add_enabled(
                    !self.workflow_active() && !self.ingest_active(),
                    egui::Button::new("解析标签与 Danbooru 别名"),
                )
                .clicked()
            {
                if let Err(e) = self.resolve_tags(ui.ctx()) {
                    self.workflows.error = Some(e);
                }
            }
            if let Some(error) = &self.workflows.error {
                ui.colored_label(AMBER, error);
            }
        }
        if self.workflows.metadata_active {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在解析标签并发现角色关系…");
            });
        }
        if self.tags.summary.is_none() {
            self.tags.summary = Some(Arc::new(TagSummary::build(self)));
        }
        let summary = self.tags.summary.as_ref().unwrap().clone();
        ui.columns(3, |cols| {
            count_card(&mut cols[0], summary.rows.len(), "标签", "tag");
            count_card(
                &mut cols[1],
                summary.warnings.len(),
                "待解析 / 冲突",
                "logs",
            );
            count_card(
                &mut cols[2],
                self.library.artists.len() + self.library.characters.len(),
                "受保护名称",
                "character",
            );
        });
        ui.add_space(18.0);
        if !summary.warnings.is_empty() {
            egui::CollapsingHeader::new("待解析与角色冲突")
                .default_open(self.tag_tab == 3)
                .show(ui, |ui| {
                    let warning_total = summary.warnings.len();
                    tag_pagination(
                        ui,
                        warning_total,
                        self.prefs.page_size,
                        &mut self.tags.warning_page,
                        "tag_warnings",
                    );
                    let range = page_range(
                        warning_total,
                        self.prefs.page_size,
                        &mut self.tags.warning_page,
                    );
                    for (id, warning) in &summary.warnings[range] {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(AMBER, warning);
                            if ui.link(self.media_name(*id)).clicked() {
                                self.open_media(*id);
                            }
                        });
                    }
                });
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (i, label) in ["全部", "通用", "名称", "冲突"].iter().enumerate() {
                    ui.selectable_value(&mut self.tag_tab, i, *label);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    search(ui, &mut self.query, "搜索标签…", 220.0);
                });
            });
            ui.add_space(14.0);
            ui.separator();
            let filter_changed = self.tags.filter(&summary, &self.query, self.tag_tab);
            let total = self.tags.filtered.len();
            let old_page = self.tags.page;
            let old_size = self.prefs.page_size;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("共 {total} 个标签 · 每页"));
                egui::ComboBox::from_id_salt("tag_page_size")
                    .selected_text(self.prefs.page_size.to_string())
                    .show_ui(ui, |ui| {
                        for size in [20, 50, 100] {
                            if ui
                                .selectable_value(&mut self.prefs.page_size, size, size.to_string())
                                .changed()
                            {
                                self.tags.page = 0;
                            }
                        }
                    });
            });
            tag_pagination(
                ui,
                total,
                self.prefs.page_size,
                &mut self.tags.page,
                "tags_top",
            );
            let range = page_range(total, self.prefs.page_size, &mut self.tags.page);
            let rows: Vec<_> = self.tags.filtered[range]
                .iter()
                .map(|&index| summary.rows[index].clone())
                .collect();
            let pending_reset = std::mem::take(&mut self.tags.reset_scroll);
            let reset_scroll = filter_changed
                || old_page != self.tags.page
                || old_size != self.prefs.page_size
                || pending_reset;
            ui.horizontal(|ui| {
                let width = ui.available_width();
                table_cell(ui, "标签", width * 0.40, MUTED);
                table_cell(ui, "来源分类", width * 0.18, MUTED);
                table_cell(ui, "关联图片", width * 0.16, MUTED);
                ui.label(RichText::new("状态").color(MUTED).size(13.0));
            });
            ui.separator();
            if rows.is_empty() {
                empty(ui, "暂无标签", "", "tag");
            }
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("tag_rows")
                .max_height(420.0)
                .auto_shrink([false, false]);
            if reset_scroll {
                scroll = scroll.vertical_scroll_offset(0.0);
            }
            scroll.show_rows(ui, 42.0, rows.len(), |ui, visible| {
                #[cfg(test)]
                {
                    self.tags.rendered_rows = visible.len();
                }
                for index in visible {
                    let TagRow {
                        tag, count, kind, ..
                    } = rows[index].clone();
                    let row = ui.push_id(&tag, |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 42.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let width = ui.available_width();
                                ui.allocate_ui_with_layout(
                                    vec2(width * 0.40, 34.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(width * 0.40);
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(&tag).color(ACCENT).size(13.0),
                                                )
                                                .truncate()
                                                .frame(false),
                                            )
                                            .on_hover_text(&tag)
                                            .clicked()
                                        {
                                            self.navigate(Page::Artist);
                                            self.gallery = true;
                                            self.query = if kind == "通用" {
                                                tag.clone()
                                            } else {
                                                String::new()
                                            };
                                            if kind != "通用" {
                                                let page = if kind == "画师" {
                                                    Page::Artist
                                                } else {
                                                    Page::Character
                                                };
                                                self.prefs.page = page;
                                                self.context = self
                                                    .library
                                                    .collections(page)
                                                    .iter()
                                                    .find(|c| c.tag == tag)
                                                    .map(|c| c.id);
                                            }
                                        }
                                    },
                                );
                                ui.allocate_ui(vec2(width * 0.18, 34.0), |ui| {
                                    ui.set_min_width(width * 0.18);
                                    ui.label(RichText::new(kind).size(13.0));
                                });
                                ui.allocate_ui(vec2(width * 0.16, 34.0), |ui| {
                                    ui.set_min_width(width * 0.16);
                                    ui.label(RichText::new(count.to_string()).size(13.0));
                                });
                                if kind == "通用" && !self.demo && !self.is_capture() {
                                    let mut protected =
                                        self.workflows.rules.protected.contains(&tag);
                                    if ui
                                        .add_enabled(
                                            !self.workflow_active() && !self.ingest_active(),
                                            egui::Checkbox::new(&mut protected, "保护"),
                                        )
                                        .on_hover_text("保留精确术语，不发送给 AI 改写")
                                        .changed()
                                    {
                                        if protected {
                                            self.workflows.rules.protected.insert(tag.clone());
                                        } else {
                                            self.workflows.rules.protected.remove(&tag);
                                        }
                                        self.save_relationships(ui.ctx());
                                    }
                                } else {
                                    let (r, response) =
                                        ui.allocate_exact_size(vec2(20.0, 34.0), Sense::hover());
                                    let icon_rect =
                                        egui::Rect::from_center_size(r.center(), vec2(20.0, 20.0));
                                    icon(
                                        ui,
                                        icon_rect,
                                        if kind == "通用" { "tag" } else { "lock" },
                                        MUTED,
                                    );
                                    response.on_hover_text(if kind == "通用" {
                                        "通用标签"
                                    } else {
                                        "名称与术语本地保留"
                                    });
                                }
                            },
                        )
                    });
                    let rect = row.inner.response.rect;
                    ui.painter().hline(
                        rect.x_range(),
                        rect.bottom(),
                        egui::Stroke::new(1.0_f32, LINE),
                    );
                }
            });
            let old_page = self.tags.page;
            tag_pagination(
                ui,
                total,
                self.prefs.page_size,
                &mut self.tags.page,
                "tags_bottom",
            );
            self.tags.reset_scroll |= old_page != self.tags.page;
        });
    }
}
fn table_cell(ui: &mut egui::Ui, text: &str, width: f32, color: egui::Color32) {
    ui.allocate_ui(vec2(width, 30.0), |ui| {
        ui.set_min_width(width);
        ui.label(RichText::new(text).color(color).size(13.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large_app() -> BdmApp {
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.library.artists.clear();
        app.library.characters.clear();
        app.library.media.truncate(1);
        app.library.media[0].tags = (0..793).map(|i| format!("tag_{i:04}")).collect();
        app.navigate(Page::Tags);
        app
    }

    fn render(ctx: &egui::Context, app: &mut BdmApp, size: [f32; 2]) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(size[0], size[1]),
                )),
                ..Default::default()
            },
            |ctx| app.shell(ctx),
        );
    }

    #[test]
    fn every_tag_is_reachable_once_and_shrinking_results_clamps_page() {
        let app = large_app();
        let summary = Arc::new(TagSummary::build(&app));
        let mut state = TagPageState::default();
        state.filter(&summary, "", 0);
        for size in [20, 50, 100] {
            let mut all = Vec::new();
            for mut page in 0..state.filtered.len().div_ceil(size) {
                all.extend_from_slice(
                    &state.filtered[page_range(state.filtered.len(), size, &mut page)],
                );
            }
            assert_eq!(all, (0..793).collect::<Vec<_>>());
        }
        state.page = 15;
        assert!(state.filter(&summary, " TAG_0792 ", 0));
        assert_eq!(state.page, 0);
        assert_eq!(state.filtered.len(), 1);
        assert!(!state.filter(&summary, "tag_0792", 0));
        state.page = 100;
        assert_eq!(page_range(1, 50, &mut state.page), 0..1);
        assert_eq!(state.page, 0);
        assert_eq!(page_range(0, 50, &mut state.page), 0..0);
    }

    #[test]
    fn counts_are_unique_images_and_category_filter_uses_full_list() {
        let mut app = large_app();
        app.library.media[0].tags.push("tag_0000".into());
        app.library.media[0].tags.push("mixed_CASE".into());
        app.workflows.rules.categories.insert("tag_0700".into(), 4);
        let summary = Arc::new(TagSummary::build(&app));
        assert_eq!(
            summary
                .rows
                .iter()
                .find(|r| r.tag == "tag_0000")
                .unwrap()
                .count,
            1
        );
        let mut state = TagPageState::default();
        state.filter(&summary, "", 2);
        assert_eq!(state.filtered.len(), 1);
        assert_eq!(summary.rows[state.filtered[0]].tag, "tag_0700");
        state.filter(&summary, "MIXED_case", 1);
        assert_eq!(state.filtered.len(), 1);
    }

    #[test]
    fn idle_frames_reuse_summary_and_draw_only_visible_rows_for_793_tags() {
        for size in [[1050.0, 720.0], [1440.0, 960.0]] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = large_app();
            render(&ctx, &mut app, size);
            let summary = app.tags.summary.as_ref().unwrap().clone();
            let started = std::time::Instant::now();
            for _ in 0..100 {
                render(&ctx, &mut app, size);
                assert!(Arc::ptr_eq(app.tags.summary.as_ref().unwrap(), &summary));
                assert!(app.tags.rendered_rows > 0);
                assert!(
                    app.tags.rendered_rows < 20,
                    "drew {} rows at {size:?}",
                    app.tags.rendered_rows
                );
            }
            eprintln!(
                "793-tag cached page at {size:?}: {:.3} ms/frame, {} visible rows",
                started.elapsed().as_secs_f64() * 10.0,
                app.tags.rendered_rows
            );
            app.tags.page = 15;
            render(&ctx, &mut app, size);
            assert_eq!(page_range(793, 50, &mut app.tags.page), 750..793);
            app.query = "tag_0792".into();
            render(&ctx, &mut app, size);
            assert_eq!(app.tags.page, 0);
            assert_eq!(app.tags.filtered.len(), 1);
            app.query = "no-match".into();
            render(&ctx, &mut app, size);
            assert_eq!(app.tags.rendered_rows, 0);
        }
    }

    #[test]
    fn media_deletion_invalidates_cached_counts_and_rules_refresh_changes_categories() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = large_app();
        render(&ctx, &mut app, [1050.0, 720.0]);
        app.workflows.rules.categories.insert("tag_0000".into(), 4);
        app.tags.invalidate();
        app.tag_tab = 2;
        render(&ctx, &mut app, [1050.0, 720.0]);
        assert_eq!(app.tags.filtered.len(), 1);
        app.remove_sample_media(&BTreeSet::from([0]));
        render(&ctx, &mut app, [1050.0, 720.0]);
        assert_eq!(app.tags.summary.as_ref().unwrap().rows.len(), 0);
    }
}
