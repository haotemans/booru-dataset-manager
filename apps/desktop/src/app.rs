use crate::{model::*, theme::*, widgets::*};
use eframe::egui::{self, vec2, Color32, RichText};
use std::collections::BTreeSet;

pub enum Dialog {
    Add,
    Action(String),
    Profile(Option<usize>),
    ImageUpload(Box<crate::workflows::ImageUploadConfirmation>),
    CharacterRelations,
    Similarity,
    Media(usize),
    Delete { return_media: Option<usize> },
    DeleteCollections(crate::collections::CollectionDeletion),
}

pub struct BdmApp {
    pub prefs: Preferences,
    pub library: Library,
    pub demo: bool,
    pub gallery: bool,
    pub context: Option<usize>,
    pub query: String,
    pub selected: BTreeSet<usize>,
    pub collection_delete_mode: bool,
    pub selected_media: BTreeSet<usize>,
    pub pending_deletion: BTreeSet<usize>,
    pub review_order: Vec<usize>,
    pub media_review: crate::media_review::ReviewState,
    pub image_editor: crate::image_editor::EditorState,
    pub similarity_review: crate::similarity_review::SimilarityReview,
    pub page_number: usize,
    pub sort: usize,
    pub review_filter: usize,
    pub source_filter: usize,
    pub minimum_only: bool,
    pub tag_tab: usize,
    pub tags: crate::tags::TagPageState,
    pub log_filter: usize,
    pub logs: Vec<LogEntry>,
    pub dialog: Option<Dialog>,
    pub input_tag: String,
    pub form_error: String,
    pub profile_draft: ApiProfile,
    pub api_profiles: crate::api_profiles::ApiProfileSettings,
    pub action_mode: usize,
    pub action_selected: bool,
    pub action_snapshot: Vec<usize>,
    pub root_draft: String,
    pub model_draft: String,
    pub minimum_short_side_draft: u32,
    pub danbooru: crate::sources::DanbooruSettings,
    pub catalog: crate::catalog::CatalogSettings,
    pub ingest: crate::ingest::IngestSettings,
    pub workflows: crate::workflows::WorkflowSettings,
    pub notice: Option<(String, f64)>,
    capture_dir: Option<std::path::PathBuf>,
    capture_step: usize,
    capture_frames: usize,
    capture_waiting: bool,
}

impl BdmApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let args: Vec<String> = std::env::args().collect();
        let capture_dir = args
            .iter()
            .position(|a| a == "--capture")
            .and_then(|i| args.get(i + 1))
            .map(std::path::PathBuf::from);
        let mut prefs: Preferences = if capture_dir.is_none() {
            cc.storage
                .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
                .unwrap_or_default()
        } else {
            Preferences::default()
        };
        prefs.normalize();
        let preview = args.iter().any(|a| a == "--preview");
        let mut app = Self::from_preferences(prefs, preview, capture_dir);
        if app.capture_dir.is_none() && !app.demo {
            app.load_danbooru_settings();
            if !app.demo {
                app.open_catalog(&crate::paths::data_directory().join("catalog.sqlite3"));
            }
        }
        app
    }

    pub fn from_preferences(
        mut prefs: Preferences,
        demo: bool,
        capture_dir: Option<std::path::PathBuf>,
    ) -> Self {
        prefs.normalize();
        if prefs.root.is_empty() {
            prefs.root = crate::paths::library_directory().display().to_string();
        }
        let root_draft = prefs.root.clone();
        let model_draft = prefs.model_path.clone();
        let minimum_short_side_draft = prefs.minimum_short_side;
        let danbooru = crate::sources::DanbooruSettings::new(prefs.danbooru_username.clone());
        Self {
            prefs,
            library: if demo {
                Library::demo()
            } else {
                Library::default()
            },
            demo,
            gallery: false,
            context: None,
            query: String::new(),
            selected: BTreeSet::new(),
            collection_delete_mode: false,
            selected_media: BTreeSet::new(),
            pending_deletion: BTreeSet::new(),
            review_order: Vec::new(),
            media_review: crate::media_review::ReviewState::default(),
            image_editor: crate::image_editor::EditorState::default(),
            similarity_review: crate::similarity_review::SimilarityReview::default(),
            page_number: 0,
            sort: 0,
            review_filter: 0,
            source_filter: 0,
            minimum_only: false,
            tag_tab: 0,
            tags: crate::tags::TagPageState::default(),
            log_filter: 0,
            logs: vec![LogEntry {
                time: "本次启动".into(),
                level: "INFO",
                message: "工作台已启动。".into(),
            }],
            dialog: None,
            input_tag: String::new(),
            form_error: String::new(),
            profile_draft: ApiProfile::default(),
            api_profiles: crate::api_profiles::ApiProfileSettings::default(),
            action_mode: 0,
            action_selected: false,
            action_snapshot: Vec::new(),
            root_draft,
            model_draft,
            minimum_short_side_draft,
            danbooru,
            catalog: crate::catalog::CatalogSettings::default(),
            ingest: crate::ingest::IngestSettings::default(),
            workflows: crate::workflows::WorkflowSettings::default(),
            notice: None,
            capture_dir,
            capture_step: 0,
            capture_frames: 0,
            capture_waiting: false,
        }
    }

    pub fn navigate(&mut self, page: Page) {
        self.cancel_tag_lookup();
        self.catalog.kind = None;
        if self.catalog.edit_credentials {
            self.danbooru.api_key.clear();
            self.danbooru.error = None;
            self.catalog.edit_credentials = false;
        }
        if matches!(self.dialog, Some(Dialog::Add)) {
            self.dialog = None;
        }
        self.prefs.page = page;
        self.gallery = false;
        self.context = None;
        self.query.clear();
        self.selected.clear();
        self.collection_delete_mode = false;
        self.selected_media.clear();
        self.page_number = 0;
        self.review_filter = 0;
        self.source_filter = 0;
        self.minimum_only = false;
    }

    pub fn toggle_demo(&mut self) {
        self.tags.invalidate();
        self.pending_deletion.clear();
        self.review_order.clear();
        self.demo = !self.demo;
        self.library = if self.demo {
            Library::demo()
        } else {
            Library::default()
        };
        self.navigate(Page::Dashboard);
        self.log(
            "INFO",
            if self.demo {
                "已进入示例预览，展示数据仅保存在内存中。"
            } else {
                "已退出示例预览，示例内容已清空。"
            },
        );
    }

    pub fn log(&mut self, level: &'static str, text: &str) {
        self.logs.push(LogEntry {
            time: format!("会话 #{:03}", self.logs.len() + 1),
            level,
            message: text.to_owned(),
        });
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }
    pub fn is_capture(&self) -> bool {
        self.capture_dir.is_some()
    }
    pub fn notify(&mut self, ctx: &egui::Context, text: &str) {
        self.notice = Some((text.to_owned(), ctx.input(|i| i.time) + 4.0));
    }

    pub fn open_action(&mut self, kind: &str) {
        self.action_mode = 0;
        self.action_selected = false;
        self.action_snapshot = if self.gallery {
            self.scope_media().iter().map(|m| m.id).collect()
        } else if !self.selected.is_empty() {
            self.associated_media(self.prefs.page, &self.selected)
                .iter()
                .map(|m| m.id)
                .collect()
        } else {
            self.associated_media(
                self.prefs.page,
                &self
                    .library
                    .collections(self.prefs.page)
                    .iter()
                    .map(|c| c.id)
                    .collect(),
            )
            .iter()
            .map(|m| m.id)
            .collect()
        };
        self.dialog = Some(Dialog::Action(kind.to_owned()));
    }

    pub(crate) fn shell(&mut self, ctx: &egui::Context) {
        self.poll_redownload(ctx);
        self.shell_content(ctx);
        self.show_notice(ctx);
        self.show_redownload_notice(ctx);
    }

    fn shell_content(&mut self, ctx: &egui::Context) {
        if matches!(self.dialog, Some(Dialog::Media(_) | Dialog::Similarity)) {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(PANEL))
                .show(ctx, |_| {});
            self.show_dialog(ctx);
            return;
        }
        egui::SidePanel::left("navigation")
            .exact_width(72.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::hover());
                ui.painter().rect_filled(r.shrink(3.0), 9.0, INK);
                icon(ui, r.shrink(11.0), "image", Color32::WHITE);
                ui.add_space(34.0);
                for page in Page::ALL.into_iter().take(4) {
                    if icon_button(ui, page.icon(), page.title(), self.prefs.page == page).clicked()
                    {
                        self.navigate(page);
                    }
                    ui.add_space(6.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    for page in Page::ALL.into_iter().skip(4).rev() {
                        if icon_button(ui, page.icon(), page.title(), self.prefs.page == page)
                            .clicked()
                        {
                            self.navigate(page);
                        }
                        ui.add_space(6.0);
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(32.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("page", self.prefs.page as u8, self.gallery))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(if self.gallery {
                                "图片"
                            } else {
                                self.prefs.page.title()
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !self.gallery
                                        && matches!(
                                            self.prefs.page,
                                            Page::Dashboard
                                                | Page::Artist
                                                | Page::Character
                                                | Page::Api
                                        )
                                    {
                                        let label = match self.prefs.page {
                                            Page::Character => "添加角色",
                                            Page::Api => "添加 API 配置",
                                            _ => "添加画师",
                                        };
                                        if icon_button(ui, "plus", label, true).clicked() {
                                            if self.prefs.page == Page::Api {
                                                self.begin_api_profile(None);
                                            } else {
                                                if self.prefs.page == Page::Dashboard {
                                                    self.navigate(Page::Artist);
                                                }
                                                self.begin_add_collection();
                                            }
                                            self.form_error.clear();
                                        }
                                    }
                                },
                            );
                        });
                        ui.add_space(24.0);
                        match self.prefs.page {
                            Page::Dashboard => self.dashboard(ui),
                            Page::Artist | Page::Character => {
                                if self.gallery {
                                    self.gallery_page(ui);
                                } else {
                                    self.collections_page(ui);
                                }
                            }
                            Page::Tags => self.tags_page(ui),
                            Page::Api => self.api_page(ui),
                            Page::Logs => self.logs_page(ui),
                            Page::Settings => self.settings_page(ui),
                        }
                    });
            });
        self.show_dialog(ctx);
    }

    fn show_notice(&self, ctx: &egui::Context) {
        if let Some((message, deadline)) = &self.notice {
            if ctx.input(|i| i.time) < *deadline {
                egui::Area::new("toast".into())
                    .anchor(egui::Align2::CENTER_BOTTOM, vec2(36.0, -24.0))
                    .order(egui::Order::Tooltip)
                    .show(ctx, |ui| {
                        egui::Frame::none()
                            .fill(INK)
                            .rounding(7.0)
                            .inner_margin(vec2(18.0, 12.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new(message).color(Color32::WHITE));
                            });
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    fn capture(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.capture_dir.clone() else {
            return;
        };
        if self.capture_step == 0
            && self.capture_frames == 0
            && std::env::args().any(|arg| arg == "--capture-redownload")
        {
            self.capture_step = 54;
            self.capture_redownload_notice();
        }
        let review_layout = std::env::args().any(|arg| arg == "--capture-review-layout");
        if review_layout && self.capture_step == 0 && self.capture_frames == 0 {
            self.capture_step = 37;
            self.library = crate::model::Library::demo();
            self.open_media(3);
            self.capture_review_content();
            self.image_editor.visible = false;
            self.media_review.sidebar = false;
            let args: Vec<_> = std::env::args().collect();
            if let Some(path) = args
                .iter()
                .position(|arg| arg == "--capture-review-image")
                .and_then(|i| args.get(i + 1))
            {
                if let Ok(image) = image::open(path) {
                    if let Some(media) = self.library.media.iter_mut().find(|m| m.id == 3) {
                        media.width = image.width() as usize;
                        media.height = image.height() as usize;
                    }
                    let pixels = image.thumbnail(1800, 1800).to_rgba8();
                    self.ingest.textures.insert(
                        3,
                        ctx.load_texture(
                            "capture-review-reference",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [pixels.width() as usize, pixels.height() as usize],
                                pixels.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                }
            }
        }
        if self.capture_step >= if review_layout { 44 } else { 57 } {
            let _ = std::fs::write(
                dir.join("capture.complete"),
                format!(
                    "Native capture completed at {} x {} logical pixels",
                    ctx.screen_rect().width(),
                    ctx.screen_rect().height()
                ),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if !self.capture_waiting {
                    continue;
                }
                let names = [
                    "01-overview-empty",
                    "02-overview-demo",
                    "03-artists",
                    "04-characters",
                    "05-gallery",
                    "06-tags",
                    "07-api",
                    "08-logs",
                    "09-settings",
                    "10-export",
                    "11-masonry",
                    "12-detail",
                    "13-review-rejected",
                    "14-delete-confirmation",
                    "15-review-after-delete",
                    "16-profile",
                    "17-select-delete-artists",
                    "18-delete-artists",
                    "19-artists-after-delete",
                    "20-delete-character",
                    "21-add-tag",
                    "22-tag-query-pending",
                    "23-tag-query-failed",
                    "24-tag-query-result",
                    "25-tag-website-verification",
                    "26-tag-authentication",
                    "27-tag-credentials-recovery",
                    "28-character-hierarchy",
                    "29-optimization",
                    "30-dedup-candidates",
                    "31-workflow-progress",
                    "32-review-caption",
                    "33-review-zoom",
                    "34-review-tags-unstarted",
                    "35-review-caption-unstarted",
                    "36-review-inpainting-selection",
                    "37-review-inpainting-preview",
                    "38-review-raw-categories",
                    "39-review-crop-free",
                    "40-review-crop-ratio",
                    "41-review-crop-preview",
                    "42-character-pending-relations",
                    "43-crop-direct-save",
                    "44-inpainting-direct-save",
                    "45-similarity-summary",
                    "46-similarity-candidates",
                    "47-character-relations-settings-entry",
                    "48-character-relations-settings",
                    "49-image-view-settings-enabled",
                    "50-discovered-artists",
                    "51-dedup-completed-clean",
                    "52-similarity-review-large",
                    "53-similarity-review-zoom",
                    "54-similarity-review-finished",
                    "55-redownload-progress",
                    "56-redownload-failed",
                    "57-redownload-complete",
                ];
                if let Err(error) = std::fs::create_dir_all(&dir).and_then(|_| {
                    image::save_buffer(
                        dir.join(format!("{}.png", names[self.capture_step])),
                        &image
                            .pixels
                            .iter()
                            .flat_map(|p| p.to_array())
                            .collect::<Vec<_>>(),
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .map_err(std::io::Error::other)
                }) {
                    eprintln!("Screenshot failed: {error}");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                self.capture_step += 1;
                self.capture_frames = 0;
                self.capture_waiting = false;
                match self.capture_step {
                    1 => self.toggle_demo(),
                    2 => self.navigate(Page::Artist),
                    3 => self.navigate(Page::Character),
                    4 => {
                        self.navigate(Page::Artist);
                        self.gallery = true;
                        self.context = None;
                    }
                    5 => {
                        self.navigate(Page::Tags);
                        if std::env::args().any(|arg| arg == "--capture-large-tags") {
                            self.library.media[0]
                                .tags
                                .extend((0..793).map(|i| format!("tag_{i:04}")));
                            self.tags.invalidate();
                        }
                    }
                    6 => self.navigate(Page::Api),
                    7 => self.navigate(Page::Logs),
                    8 => self.navigate(Page::Settings),
                    9 => {
                        self.navigate(Page::Artist);
                        self.gallery = true;
                        self.context = Some(0);
                        self.open_action("导出");
                    }
                    10 => {
                        self.dialog = None;
                        self.navigate(Page::Artist);
                        self.gallery = true;
                        self.prefs.layout = GalleryLayout::Masonry;
                    }
                    11 => {
                        if std::env::args().any(|arg| arg == "--capture-review-tags") {
                            self.capture_review_content();
                        }
                        self.open_media(3);
                    }
                    12 => {
                        if std::env::args().any(|arg| arg == "--capture-review-tags") {
                            self.media_review.tab = 1;
                        }
                        self.reject_media(3);
                        self.reject_media(8);
                    }
                    13 => self.open_deletion_review(Some(3)),
                    14 => {
                        if let Err(error) = self.confirm_sample_deletion(Some(3)) {
                            eprintln!("Capture deletion failed: {error}");
                        }
                    }
                    15 => {
                        self.navigate(Page::Api);
                        self.begin_api_profile(None);
                    }
                    16 => {
                        self.dialog = None;
                        self.navigate(Page::Artist);
                        self.collection_delete_mode = true;
                        self.selected = BTreeSet::from([4, 5]);
                    }
                    17 => {
                        self.open_collection_deletion(Page::Artist, &self.selected.clone());
                    }
                    18 => {
                        if let Some(Dialog::DeleteCollections(scope)) = self.dialog.take() {
                            if let Err(error) = self.confirm_sample_collection_deletion(&scope) {
                                eprintln!("Collection capture deletion failed: {error}");
                            }
                        }
                    }
                    19 => {
                        self.navigate(Page::Character);
                        self.open_collection_deletion(Page::Character, &BTreeSet::from([0]));
                    }
                    20 => {
                        self.dialog = None;
                        self.navigate(Page::Character);
                        self.begin_add_collection();
                        self.input_tag = "synthetic_character_(series)".into();
                    }
                    21 => self.capture_tag_state(0),
                    22 => self.capture_tag_state(1),
                    23 => self.capture_tag_state(2),
                    24 => self.capture_tag_state(3),
                    25 => self.capture_tag_state(4),
                    26 => self.catalog.edit_credentials = true,
                    27 => {
                        self.dialog = None;
                        self.navigate(Page::Character);
                        self.workflows.rules.characters = vec![
                            bdm_domain::tags::CharacterRelation {
                                base: "ganyu_(genshin_impact)".into(),
                                variant: "ganyu_(twilight_blossom)_(genshin_impact)".into(),
                                verified: true,
                                exclusive_group: "costume".into(),
                                evidence: "synthetic human-confirmed capture".into(),
                            },
                            bdm_domain::tags::CharacterRelation {
                                base: "aria_(zenless_zone_zero)".into(),
                                variant: "aria_(human)_(zenless_zone_zero)".into(),
                                verified: true,
                                exclusive_group: String::new(),
                                evidence: bdm_domain::tags::DANBOORU_CHARACTER_IMPLICATION.into(),
                            },
                            bdm_domain::tags::CharacterRelation {
                                base: "aria_(zenless_zone_zero)".into(),
                                variant: "aria_(robot)_(zenless_zone_zero)".into(),
                                verified: true,
                                exclusive_group: String::new(),
                                evidence: bdm_domain::tags::DANBOORU_CHARACTER_IMPLICATION.into(),
                            },
                        ];
                    }
                    28 => {
                        self.gallery = true;
                        self.context = Some(1);
                        self.open_action("优化");
                    }
                    29 => {
                        self.open_action("去重");
                        self.context = None;
                        self.action_snapshot = self.library.media.iter().map(|m| m.id).collect();
                        self.workflows.candidates = vec![bdm_domain::dedup::DuplicateCandidate {
                            first: 1,
                            second: 2,
                            phash_distance: Some(1),
                            cosine: None,
                            model_version: None,
                            state: "pending".into(),
                        }];
                    }
                    30 => {
                        self.dialog = None;
                        self.navigate(Page::Dashboard);
                        self.workflows.jobs = vec![bdm_infrastructure::workflows::WorkflowJob {
                            id: 1,
                            kind: "dedup".into(),
                            ids: (0..24).collect(),
                            cursor: 12,
                            state: "paused".into(),
                            ..Default::default()
                        }];
                    }
                    31 => {
                        self.workflows.artifacts.insert(
                            (2, "caption".into()),
                            bdm_infrastructure::workflows::TextArtifact {
                                media_id: 2,
                                kind: "caption".into(),
                                content: "A landscape with layered mountains and a warm sun."
                                    .into(),
                                state: "needs_review".into(),
                                reviewed: false,
                                provenance: "synthetic-capture:caption:bdm-text-v1".into(),
                            },
                        );
                        self.open_media(2);
                        self.media_review.tab = 2;
                    }
                    32 => {
                        self.media_review.tab = 0;
                        self.media_review.view.zoom = 3.0;
                    }
                    33 => {
                        self.workflows.artifacts.remove(&(2, "caption".into()));
                        self.workflows.artifacts.insert(
                            (2, "tags".into()),
                            bdm_infrastructure::workflows::TextArtifact {
                                media_id: 2,
                                kind: "tags".into(),
                                content: "landscape, mountains, sun".into(),
                                state: "complete".into(),
                                reviewed: false,
                                provenance: "local-normalization-v1".into(),
                            },
                        );
                        self.media_review.view = crate::media_review::CanvasView::default();
                        self.media_review.tab = 1;
                    }
                    34 => self.media_review.tab = 2,
                    35 => {
                        self.image_editor.visible = true;
                        self.image_editor.tool = crate::image_editor::Tool::Rectangle;
                        self.image_editor.regions =
                            vec![bdm_domain::editing::MaskRegion::Rectangle {
                                start: [650.0, 280.0],
                                end: [1050.0, 680.0],
                            }];
                    }
                    36 => {
                        if let Ok(directory) = tempfile::tempdir() {
                            let image = directory.path().join("preview.png");
                            let mask = directory.path().join("mask.png");
                            let pixels = image::RgbaImage::from_fn(480, 480, |x, y| {
                                image::Rgba([170 + (x / 10) as u8, 190, 200 + (y / 16) as u8, 255])
                            });
                            let _ = pixels.save(&image);
                            let _ = image::GrayImage::from_pixel(480, 480, image::Luma([255]))
                                .save(&mask);
                            self.image_editor.texture = Some(ctx.load_texture(
                                "capture-inpainting-preview",
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [480, 480],
                                    pixels.as_raw(),
                                ),
                                egui::TextureOptions::LINEAR,
                            ));
                            self.image_editor.preview =
                                Some(bdm_infrastructure::editing::RepairPreview {
                                    directory,
                                    image,
                                    mask,
                                    parent_sha256: "synthetic".into(),
                                    model: "synthetic".into(),
                                    regions: self.image_editor.regions.clone(),
                                    crop: None,
                                });
                        }
                    }
                    37 => {
                        self.image_editor.discard();
                        self.library = crate::model::Library::demo();
                        self.open_media(3);
                        self.capture_review_content();
                        self.image_editor.visible = false;
                        self.media_review.tab = 0;
                        self.media_review.view = crate::media_review::CanvasView::default();
                    }
                    38 => {
                        self.image_editor.visible = true;
                        self.image_editor.cropping = true;
                        self.image_editor.crop.ratio = 0;
                        self.image_editor.crop.selection = Some(egui::Rect::from_min_max(
                            egui::pos2(170.0, 90.0),
                            egui::pos2(650.0, 700.0),
                        ));
                    }
                    39 => {
                        self.image_editor.crop.ratio = if review_layout { 8 } else { 7 };
                        let media = self.library.media.iter().find(|m| m.id == 3).unwrap();
                        self.image_editor
                            .crop
                            .center(egui::vec2(media.width as f32, media.height as f32));
                    }
                    40 => {
                        let directory = tempfile::tempdir().unwrap();
                        let image = directory.path().join("preview.png");
                        let pixels = image::RgbaImage::from_fn(270, 480, |x, y| {
                            image::Rgba([170 + (x / 10) as u8, 190, 200 + (y / 16) as u8, 255])
                        });
                        pixels.save(&image).unwrap();
                        self.image_editor.texture = Some(ctx.load_texture(
                            "capture-crop-preview",
                            egui::ColorImage::from_rgba_unmultiplied([270, 480], pixels.as_raw()),
                            egui::TextureOptions::LINEAR,
                        ));
                        self.image_editor.preview =
                            Some(bdm_infrastructure::editing::RepairPreview {
                                directory,
                                image,
                                mask: std::path::PathBuf::new(),
                                parent_sha256: "synthetic".into(),
                                model: "local-crop-v1".into(),
                                regions: Vec::new(),
                                crop: Some(bdm_domain::editing::CropRect {
                                    x: 0,
                                    y: 0,
                                    width: 270,
                                    height: 480,
                                }),
                            });
                        self.image_editor.show_preview = true;
                        self.media_review.view = crate::media_review::CanvasView::default();
                    }
                    41 if review_layout => {
                        self.image_editor.discard();
                        self.image_editor.visible = false;
                        self.media_review.sidebar = true;
                    }
                    41 => {
                        self.image_editor.discard();
                        self.dialog = None;
                        self.navigate(Page::Character);
                        self.workflows
                            .rules
                            .characters
                            .push(bdm_domain::tags::CharacterRelation {
                                base: "synthetic_pending_base".into(),
                                variant: "synthetic_pending_variant".into(),
                                verified: false,
                                exclusive_group: String::new(),
                                evidence: "synthetic ambiguous capture".into(),
                            });
                    }
                    42 => {
                        self.media_review.sidebar = false;
                        self.dialog = None;
                        self.open_media(3);
                        self.image_editor.reset(3);
                        self.image_editor.visible = true;
                        self.image_editor.cropping = true;
                        self.prefs.confirm_image_edits = false;
                        self.image_editor.crop.ratio = 1;
                        if let Some(media) = self.library.media.iter().find(|m| m.id == 3) {
                            self.image_editor
                                .crop
                                .center(egui::vec2(media.width as f32, media.height as f32));
                        }
                    }
                    43 => {
                        self.image_editor.cropping = false;
                        self.image_editor.tool = crate::image_editor::Tool::Rectangle;
                        self.image_editor.regions =
                            vec![bdm_domain::editing::MaskRegion::Rectangle {
                                start: [650.0, 280.0],
                                end: [1050.0, 680.0],
                            }];
                    }
                    44 => {
                        self.image_editor.discard();
                        self.dialog = None;
                        self.navigate(Page::Dashboard);
                        self.workflows.jobs = (1..=5)
                            .map(|id| bdm_infrastructure::workflows::WorkflowJob {
                                id,
                                kind: "dedup-auto".into(),
                                ids: (0..24).collect(),
                                cursor: 24,
                                state: "completed".into(),
                                review_similarity: Some(0.95),
                                dedup_removed: 2,
                                dedup_stats_recorded: true,
                                dedup_review_pairs: std::collections::BTreeSet::from([
                                    (1, 2),
                                    (2, 3),
                                ]),
                                ..Default::default()
                            })
                            .collect();
                        self.workflows.candidates = vec![bdm_domain::dedup::DuplicateCandidate {
                            first: 1,
                            second: 2,
                            phash_distance: Some(12),
                            cosine: Some(0.95957),
                            model_version: Some("synthetic-capture".into()),
                            state: "pending".into(),
                        }];
                    }
                    45 => {
                        self.open_action("去重");
                        self.action_snapshot = self.library.media.iter().map(|m| m.id).collect();
                    }
                    46 => {
                        self.dialog = None;
                        self.navigate(Page::Settings);
                        self.workflows
                            .rules
                            .characters
                            .retain(|relation| relation.verified);
                        self.workflows.relationship_summary = Some("关系与本地标签已更新。".into());
                    }
                    47 => self.dialog = Some(Dialog::CharacterRelations),
                    48 => {
                        self.dialog = None;
                        self.navigate(Page::Settings);
                        self.prefs.auto_load_review_original = true;
                    }
                    49 => {
                        self.navigate(Page::Artist);
                        if let Some(artist) = self.library.artists.last_mut() {
                            artist.api_count = None;
                        }
                    }
                    50 => {
                        self.dialog = None;
                        self.navigate(Page::Dashboard);
                        self.workflows.candidates.clear();
                    }
                    51 => {
                        let ids: Vec<_> = self
                            .library
                            .media
                            .iter()
                            .map(|media| media.id)
                            .take(2)
                            .collect();
                        let pair = bdm_domain::dedup::DuplicateCandidate {
                            first: ids[0] as i64,
                            second: ids[1] as i64,
                            phash_distance: Some(12),
                            cosine: Some(0.96),
                            model_version: Some("synthetic-capture".into()),
                            state: "pending".into(),
                        };
                        self.workflows.candidates = vec![pair.clone()];
                        self.open_pair_review(&pair);
                    }
                    52 => self.similarity_review.capture_zoom(),
                    53 => {
                        self.workflows.candidates.clear();
                    }
                    54 => self.capture_redownload_notice(),
                    55 => {
                        if let Some(notice) = &mut self.media_review.redownload_notice {
                            notice.result = Some(Err("来源暂时不可用，请稍后重试。".into()));
                        }
                    }
                    56 => {
                        if let Some(notice) = &mut self.media_review.redownload_notice {
                            notice.result = Some(Ok(()));
                        }
                    }
                    _ => {
                        let _ = std::fs::write(
                            dir.join("capture.complete"),
                            format!(
                                "Native capture completed at {} x {} logical pixels",
                                ctx.screen_rect().width(),
                                ctx.screen_rect().height()
                            ),
                        );
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        return;
                    }
                }
            }
        }
        self.capture_frames += 1;
        if self.capture_frames >= 5 && !self.capture_waiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            self.capture_waiting = true;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }
}

impl eframe::App for BdmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tag_lookup(ctx);
        self.poll_danbooru_verification(ctx);
        self.poll_ingest(ctx);
        self.ingest.thumbnails.poll(ctx, &mut self.ingest.textures);
        self.poll_workflows(ctx);
        self.poll_similarity_review(ctx);
        self.editor_poll(ctx);
        self.capture(ctx);
        if self.capture_dir.is_some() {
            // Native snapshots represent idle pages, without accidental cursor hover/tooltips.
            ctx.input_mut(|input| input.pointer = Default::default());
        }
        self.shell(ctx);
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.capture_dir.is_none() {
            eframe::set_value(storage, eframe::APP_KEY, &self.prefs);
        }
    }
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
    fn persist_egui_memory(&self) -> bool {
        self.capture_dir.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::{App, Storage};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct MemoryStorage(BTreeMap<String, String>);
    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    #[test]
    fn preferences_persist_without_sample_library_or_review_data() {
        let mut prefs = Preferences {
            root: "D:\\资料库".into(),
            layout: GalleryLayout::Masonry,
            ..Default::default()
        };
        prefs.normalize();
        let mut app = BdmApp::from_preferences(prefs, true, None);
        let mut storage = MemoryStorage::default();
        app.save(&mut storage);
        let saved: Preferences = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        assert_eq!(saved.root, "D:\\资料库");
        assert_eq!(saved.layout, GalleryLayout::Masonry);
        assert_eq!(saved.minimum_short_side, 800);
        assert!(saved.confirm_image_edits);
        assert!(!saved.auto_load_review_original);
        assert!(saved.similar_image_review);
        assert_eq!(saved.review_similarity, 0.95);
        app.prefs.review_similarity = 0.92;
        app.prefs.similar_image_review = false;
        app.prefs.confirm_image_edits = false;
        app.prefs.auto_load_review_original = true;
        app.save(&mut storage);
        let saved: Preferences = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        let restarted = BdmApp::from_preferences(saved, false, None);
        assert!(!restarted.prefs.confirm_image_edits);
        assert!(restarted.prefs.auto_load_review_original);
        assert_eq!(restarted.prefs.review_similarity, 0.92);
        assert!(!restarted.prefs.similar_image_review);
        assert_eq!(
            restarted.prefs.download_media,
            bdm_config::DownloadMediaPolicy::StaticImagesOnly
        );
        assert!(!storage
            .0
            .values()
            .any(|s| s.contains("sample_artist") || s.contains("sample_character")));
        storage.set_string(
            eframe::APP_KEY,
            "(root:\"D:\\\\legacy\",page_size:20)".into(),
        );
        let legacy: Preferences = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        assert_eq!(legacy.root, "D:\\legacy");
        assert_eq!(legacy.minimum_short_side, 800);
        assert!(legacy.confirm_image_edits);
        assert!(!legacy.auto_load_review_original);
        assert!(legacy.similar_image_review);
        assert_eq!(legacy.review_similarity, 0.95);
        assert_eq!(
            legacy.download_media,
            bdm_config::AppConfig::default().download_media
        );
    }
    #[test]
    fn edited_download_minimum_survives_save_and_restart() {
        let mut app = BdmApp::from_preferences(Preferences::default(), false, None);
        app.minimum_short_side_draft = 1200;
        assert_eq!(app.prefs.minimum_short_side, 800);
        app.save_download_settings().unwrap();
        let mut storage = MemoryStorage::default();
        app.save(&mut storage);
        let saved: Preferences = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        let mut restarted = BdmApp::from_preferences(saved, false, None);
        assert_eq!(restarted.prefs.minimum_short_side, 1200);
        assert_eq!(restarted.minimum_short_side_draft, 1200);
        for invalid in [0, 16385] {
            restarted.minimum_short_side_draft = invalid;
            assert!(restarted.save_download_settings().is_err());
            assert_eq!(restarted.prefs.minimum_short_side, 1200);
        }
        restarted.minimum_short_side_draft = 800;
        restarted.save_download_settings().unwrap();
        assert_eq!(restarted.prefs.minimum_short_side, 800);
    }

    #[test]
    fn screenshot_run_does_not_save_user_preferences() {
        let mut app =
            BdmApp::from_preferences(Preferences::default(), false, Some("target/capture".into()));
        let mut storage = MemoryStorage::default();
        app.save(&mut storage);
        assert!(storage.0.is_empty());
    }
}
